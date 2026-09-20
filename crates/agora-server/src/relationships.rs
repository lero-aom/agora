use std::{net::SocketAddr, sync::atomic::Ordering};

use agora_common::{
    ApiError, BlockListResponse, BlockUserRequest, BlockUserResponse, CreateReportRequest,
    CreateReportResponse, FriendListResponse, FriendRequest, FriendshipResponse, FriendshipStatus,
    FriendshipSummary, MessageKind, RemoveFriendResponse, UnblockUserResponse, UserSearchResponse,
    UserSummary,
};
use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use sqlx::{postgres::PgRow, Postgres, Row, Transaction};
use tracing::warn;
use uuid::Uuid;

use crate::{auth, chat, visibility, AppState};

const USER_SEARCH_LIMIT: i64 = 20;
const MAX_REASON_LEN: usize = 500;
const MAX_DETAILS_LEN: usize = 2_000;

type RelationshipResult<T> = Result<T, RelationshipError>;

#[derive(Deserialize)]
struct UserSearchQuery {
    q: Option<String>,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/users/search", get(search_users))
        .route("/friends", get(list_friends).post(send_friend_request))
        .route(
            "/friends/{friendship_id}/accept",
            post(accept_friend_request),
        )
        .route(
            "/friends/{friendship_id}/decline",
            post(decline_friend_request),
        )
        .route("/friends/{user_id}", delete(remove_friendship))
        .route("/blocks", get(list_blocks).post(block_user))
        .route("/blocks/{user_id}", delete(unblock_user))
        .route("/reports", post(create_report))
}

async fn search_users(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<UserSearchQuery>,
) -> RelationshipResult<Json<UserSearchResponse>> {
    check_relationship_read_limit(&state, peer_addr, &headers).await?;
    let user = current_user(&state, &headers).await?;
    let query = query.q.unwrap_or_default().trim().to_string();
    if query.is_empty() {
        return Ok(Json(UserSearchResponse { users: Vec::new() }));
    }

    let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
    let sql = format!(
        "select id, display_name, avatar_url
         from users
         where id <> $1
           and banned_at is null
           and (suspended_until is null or suspended_until <= now())
           and {}
           and display_name ilike $2 escape '\\'
         order by display_name asc
         limit $3",
        visibility::not_blocked_between_sql("$1", "users.id")
    );
    let rows = sqlx::query(&sql)
        .bind(user.id)
        .bind(pattern)
        .bind(USER_SEARCH_LIMIT)
        .fetch_all(&state.db)
        .await?;

    Ok(Json(UserSearchResponse {
        users: rows
            .iter()
            .map(row_to_user)
            .collect::<Result<Vec<_>, _>>()?,
    }))
}

async fn list_friends(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> RelationshipResult<Json<FriendListResponse>> {
    check_relationship_read_limit(&state, peer_addr, &headers).await?;
    let user = current_user(&state, &headers).await?;
    let sql = friendship_select_sql(
        "where f.requester_id = $1 or f.addressee_id = $1
         order by f.updated_at desc",
    );
    let rows = sqlx::query(&sql).bind(user.id).fetch_all(&state.db).await?;

    Ok(Json(FriendListResponse {
        friendships: rows
            .iter()
            .map(row_to_friendship)
            .collect::<RelationshipResult<Vec<_>>>()?,
    }))
}

async fn send_friend_request(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<FriendRequest>,
) -> RelationshipResult<Json<FriendshipResponse>> {
    check_relationship_write_limit(&state, peer_addr, &headers).await?;
    let user = current_user(&state, &headers).await?;
    let target = user_by_id(&state, request.user_id).await?;
    if user.id == target.id {
        return Err(RelationshipError::bad_request(
            "cannot send a friend request to yourself",
        ));
    }
    let mut tx = state.db.begin().await?;
    lock_relationship_pair(&mut tx, user.id, target.id).await?;
    if visibility::blocked_between_tx(&mut tx, user.id, target.id).await? {
        tx.rollback().await?;
        return Err(RelationshipError::conflict(
            "friend request is blocked by an existing block",
        ));
    }

    let friendship_id = match existing_friendship_tx(&mut tx, user.id, target.id).await? {
        Some(existing)
            if existing.status == FriendshipStatus::Pending && existing.addressee.id == user.id =>
        {
            update_friendship_status(&mut tx, existing.id, FriendshipStatus::Accepted).await?
        }
        Some(existing)
            if matches!(
                existing.status,
                FriendshipStatus::Pending | FriendshipStatus::Accepted
            ) =>
        {
            existing.id
        }
        Some(existing) => {
            sqlx::query(
                "update friendships
                 set requester_id = $2,
                     addressee_id = $3,
                     status = 'pending',
                     updated_at = now()
                 where id = $1",
            )
            .bind(existing.id)
            .bind(user.id)
            .bind(target.id)
            .execute(&mut *tx)
            .await?;
            existing.id
        }
        None => {
            let row = sqlx::query(
                "insert into friendships (requester_id, addressee_id, status)
                 values ($1, $2, 'pending')
                 returning id",
            )
            .bind(user.id)
            .bind(target.id)
            .fetch_one(&mut *tx)
            .await?;
            row.try_get("id")?
        }
    };
    tx.commit().await?;
    send_relationship_changed(&state, user.id, target.id);

    Ok(Json(FriendshipResponse {
        friendship: friendship_by_id(&state, friendship_id).await?,
    }))
}

async fn accept_friend_request(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(friendship_id): Path<Uuid>,
) -> RelationshipResult<Json<FriendshipResponse>> {
    check_relationship_write_limit(&state, peer_addr, &headers).await?;
    let user = current_user(&state, &headers).await?;
    let (requester_id, addressee_id) = update_incoming_pending_friendship(
        &state,
        friendship_id,
        user.id,
        FriendshipStatus::Accepted,
    )
    .await?;
    send_relationship_changed(&state, requester_id, addressee_id);

    Ok(Json(FriendshipResponse {
        friendship: friendship_by_id(&state, friendship_id).await?,
    }))
}

async fn decline_friend_request(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(friendship_id): Path<Uuid>,
) -> RelationshipResult<Json<FriendshipResponse>> {
    check_relationship_write_limit(&state, peer_addr, &headers).await?;
    let user = current_user(&state, &headers).await?;
    let (requester_id, addressee_id) = update_incoming_pending_friendship(
        &state,
        friendship_id,
        user.id,
        FriendshipStatus::Declined,
    )
    .await?;
    send_relationship_changed(&state, requester_id, addressee_id);

    Ok(Json(FriendshipResponse {
        friendship: friendship_by_id(&state, friendship_id).await?,
    }))
}

async fn remove_friendship(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
) -> RelationshipResult<Json<RemoveFriendResponse>> {
    check_relationship_write_limit(&state, peer_addr, &headers).await?;
    let user = current_user(&state, &headers).await?;
    let mut tx = state.db.begin().await?;
    lock_relationship_pair(&mut tx, user.id, user_id).await?;
    let result = sqlx::query(
        "update friendships
         set status = 'removed', updated_at = now()
         where status <> 'removed'
           and ((requester_id = $1 and addressee_id = $2)
             or (requester_id = $2 and addressee_id = $1))",
    )
    .bind(user.id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if result.rows_affected() > 0 {
        send_relationship_changed(&state, user.id, user_id);
    }

    Ok(Json(RemoveFriendResponse {
        removed: result.rows_affected() > 0,
    }))
}

async fn list_blocks(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> RelationshipResult<Json<BlockListResponse>> {
    check_relationship_read_limit(&state, peer_addr, &headers).await?;
    let user = current_user(&state, &headers).await?;
    let rows = sqlx::query(
        "select u.id, u.display_name, u.avatar_url
         from blocks b
         join users u on u.id = b.blocked_id
         where b.blocker_id = $1
         order by b.created_at desc",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(BlockListResponse {
        blocked_users: rows
            .iter()
            .map(row_to_user)
            .collect::<Result<Vec<_>, _>>()?,
    }))
}

async fn block_user(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<BlockUserRequest>,
) -> RelationshipResult<Json<BlockUserResponse>> {
    check_relationship_write_limit(&state, peer_addr, &headers).await?;
    let user = current_user(&state, &headers).await?;
    let target = user_by_id_any_status(&state, request.user_id).await?;
    if user.id == target.id {
        return Err(RelationshipError::bad_request("cannot block yourself"));
    }

    let _direct_message_delivery = state
        .direct_message_delivery_locks
        .lock(user.id, target.id)
        .await;
    let _realtime_state = state.realtime_state_lock.lock().await;
    let mut tx = state.db.begin().await?;
    lock_relationship_pair(&mut tx, user.id, target.id).await?;
    let block_result = sqlx::query(
        "insert into blocks (blocker_id, blocked_id)
         values ($1, $2)
         on conflict (blocker_id, blocked_id) do nothing",
    )
    .bind(user.id)
    .bind(target.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update friendships
         set status = 'removed', updated_at = now()
         where status <> 'removed'
           and ((requester_id = $1 and addressee_id = $2)
             or (requester_id = $2 and addressee_id = $1))",
    )
    .bind(user.id)
    .bind(target.id)
    .execute(&mut *tx)
    .await?;
    if block_result.rows_affected() > 0 {
        // Invalidate cached delivery decisions before this transaction becomes visible.
        state.visibility_epoch.fetch_add(1, Ordering::AcqRel);
    }
    tx.commit().await?;
    if block_result.rows_affected() > 0 {
        send_relationship_changed(&state, user.id, target.id);
        chat::send_user_event(
            &state.chat_tx,
            user.id,
            agora_common::ServerEvent::UserMessagesHidden { user_id: target.id },
        );
        refresh_user_global_snapshot_locked(&state, target.id).await;
    }

    Ok(Json(BlockUserResponse { blocked: true }))
}

async fn unblock_user(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
) -> RelationshipResult<Json<UnblockUserResponse>> {
    check_relationship_write_limit(&state, peer_addr, &headers).await?;
    let user = current_user(&state, &headers).await?;
    let _direct_message_delivery = state
        .direct_message_delivery_locks
        .lock(user.id, user_id)
        .await;
    let _realtime_state = state.realtime_state_lock.lock().await;
    let mut tx = state.db.begin().await?;
    lock_relationship_pair(&mut tx, user.id, user_id).await?;
    let result = sqlx::query("delete from blocks where blocker_id = $1 and blocked_id = $2")
        .bind(user.id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() > 0 {
        // Invalidate cached delivery decisions before this transaction becomes visible.
        state.visibility_epoch.fetch_add(1, Ordering::AcqRel);
    }
    tx.commit().await?;
    if result.rows_affected() > 0 {
        send_relationship_changed(&state, user.id, user_id);
        refresh_user_global_snapshot_locked(&state, user.id).await;
        refresh_user_global_snapshot_locked(&state, user_id).await;
    }

    Ok(Json(UnblockUserResponse {
        unblocked: result.rows_affected() > 0,
    }))
}

async fn create_report(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CreateReportRequest>,
) -> RelationshipResult<Json<CreateReportResponse>> {
    check_relationship_write_limit(&state, peer_addr, &headers).await?;
    let user = current_user(&state, &headers).await?;
    if user.id == request.reported_user_id {
        return Err(RelationshipError::bad_request("cannot report yourself"));
    }

    let reason = normalize_required_text(&request.reason, "reason", MAX_REASON_LEN)?;
    let details = normalize_optional_text(request.details.as_deref(), MAX_DETAILS_LEN)?;
    validate_report_target(&state, user.id, &request).await?;
    let message_kind = request.message_kind.map(message_kind_as_str);

    let row = sqlx::query(
        "insert into reports (
            reporter_id,
            reported_user_id,
            message_id,
            message_kind,
            reason,
            details
         ) values ($1, $2, $3, $4, $5, $6)
         on conflict (
             reporter_id,
             reported_user_id,
             coalesce(message_id, '00000000-0000-0000-0000-000000000000'::uuid),
             coalesce(message_kind, '')
         ) where status = 'open'
         do update set id = reports.id
         returning id",
    )
    .bind(user.id)
    .bind(request.reported_user_id)
    .bind(request.message_id)
    .bind(message_kind)
    .bind(reason)
    .bind(details)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(CreateReportResponse {
        id: row.try_get("id")?,
    }))
}

async fn current_user(state: &AppState, headers: &HeaderMap) -> RelationshipResult<UserSummary> {
    let access_token = auth::bearer_token(headers)
        .ok_or_else(|| RelationshipError::unauthorized("missing bearer token"))?;
    auth::user_for_access_token(state, access_token)
        .await?
        .map(|session| session.user)
        .ok_or_else(|| RelationshipError::unauthorized("session is invalid or expired"))
}

fn send_relationship_changed(state: &AppState, first: Uuid, second: Uuid) {
    chat::send_user_event(
        &state.chat_tx,
        first,
        agora_common::ServerEvent::RelationshipStateChanged,
    );
    chat::send_user_event(
        &state.chat_tx,
        second,
        agora_common::ServerEvent::RelationshipStateChanged,
    );
}

async fn refresh_user_global_snapshot_locked(state: &AppState, user_id: Uuid) {
    if let Err(error) = chat::send_user_global_snapshot_locked(state, user_id).await {
        warn!(%error, "failed to refresh global chat snapshot after a relationship change");
        chat::send_user_event(
            &state.chat_tx,
            user_id,
            agora_common::ServerEvent::GlobalMessageSnapshot {
                messages: Vec::new(),
            },
        );
    }
}

async fn check_relationship_read_limit(
    state: &AppState,
    peer_addr: SocketAddr,
    headers: &HeaderMap,
) -> RelationshipResult<()> {
    state
        .rate_limits
        .check_relationship_read(peer_addr, headers, &state.config.trusted_proxy_cidrs)
        .await
        .map_err(|error| RelationshipError::too_many_requests(error.message()))
}

async fn check_relationship_write_limit(
    state: &AppState,
    peer_addr: SocketAddr,
    headers: &HeaderMap,
) -> RelationshipResult<()> {
    state
        .rate_limits
        .check_relationship_write(peer_addr, headers, &state.config.trusted_proxy_cidrs)
        .await
        .map_err(|error| RelationshipError::too_many_requests(error.message()))
}

async fn user_by_id(state: &AppState, user_id: Uuid) -> RelationshipResult<UserSummary> {
    let Some(row) = sqlx::query(
        "select id, display_name, avatar_url
         from users
         where id = $1
           and banned_at is null
           and (suspended_until is null or suspended_until <= now())",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    else {
        return Err(RelationshipError::not_found("user was not found"));
    };

    Ok(row_to_user(&row)?)
}

async fn user_by_id_any_status(state: &AppState, user_id: Uuid) -> RelationshipResult<UserSummary> {
    let Some(row) = sqlx::query(
        "select id, display_name, avatar_url
         from users
         where id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    else {
        return Err(RelationshipError::not_found("user was not found"));
    };

    Ok(row_to_user(&row)?)
}

async fn lock_relationship_pair(
    tx: &mut Transaction<'_, Postgres>,
    first: Uuid,
    second: Uuid,
) -> RelationshipResult<()> {
    sqlx::query("select lock_relationship_pair($1, $2)")
        .bind(first)
        .bind(second)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn existing_friendship_tx(
    tx: &mut Transaction<'_, Postgres>,
    first: Uuid,
    second: Uuid,
) -> RelationshipResult<Option<FriendshipSummary>> {
    let sql = friendship_select_sql(
        "where (f.requester_id = $1 and f.addressee_id = $2)
            or (f.requester_id = $2 and f.addressee_id = $1)",
    );
    let row = sqlx::query(&sql)
        .bind(first)
        .bind(second)
        .fetch_optional(&mut **tx)
        .await?;

    row.as_ref().map(row_to_friendship).transpose()
}

async fn update_incoming_pending_friendship(
    state: &AppState,
    friendship_id: Uuid,
    user_id: Uuid,
    status: FriendshipStatus,
) -> RelationshipResult<(Uuid, Uuid)> {
    let status = friendship_status_as_str(status);
    let mut tx = state.db.begin().await?;
    let Some(row) = sqlx::query(
        "select requester_id, addressee_id
         from friendships
         where id = $1 and addressee_id = $2",
    )
    .bind(friendship_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        tx.rollback().await?;
        return Err(RelationshipError::forbidden(
            "friend request is not pending for this user",
        ));
    };
    let requester_id = row.try_get("requester_id")?;
    let addressee_id = row.try_get("addressee_id")?;
    lock_relationship_pair(&mut tx, requester_id, addressee_id).await?;

    if status == "accepted"
        && visibility::blocked_between_tx(&mut tx, requester_id, addressee_id).await?
    {
        tx.rollback().await?;
        return Err(RelationshipError::conflict(
            "friend request is blocked by an existing block",
        ));
    }

    let updated = sqlx::query(
        "update friendships
         set status = $3, updated_at = now()
         where id = $1 and addressee_id = $2 and status = 'pending'
         returning id",
    )
    .bind(friendship_id)
    .bind(user_id)
    .bind(status)
    .fetch_optional(&mut *tx)
    .await?;

    if updated.is_some() {
        tx.commit().await?;
        Ok((requester_id, addressee_id))
    } else {
        tx.rollback().await?;
        Err(RelationshipError::forbidden(
            "friend request is not pending for this user",
        ))
    }
}

async fn update_friendship_status(
    tx: &mut Transaction<'_, Postgres>,
    friendship_id: Uuid,
    status: FriendshipStatus,
) -> RelationshipResult<Uuid> {
    sqlx::query(
        "update friendships
         set status = $2, updated_at = now()
         where id = $1",
    )
    .bind(friendship_id)
    .bind(friendship_status_as_str(status))
    .execute(&mut **tx)
    .await?;
    Ok(friendship_id)
}

async fn friendship_by_id(
    state: &AppState,
    friendship_id: Uuid,
) -> RelationshipResult<FriendshipSummary> {
    let sql = friendship_select_sql("where f.id = $1");
    let Some(row) = sqlx::query(&sql)
        .bind(friendship_id)
        .fetch_optional(&state.db)
        .await?
    else {
        return Err(RelationshipError::not_found("friendship was not found"));
    };

    row_to_friendship(&row)
}

async fn validate_report_target(
    state: &AppState,
    reporter_id: Uuid,
    request: &CreateReportRequest,
) -> RelationshipResult<()> {
    let _ = user_by_id_any_status(state, request.reported_user_id).await?;

    match (request.message_id, request.message_kind) {
        (None, None) => Ok(()),
        (Some(message_id), Some(MessageKind::Global)) => {
            let exists = sqlx::query_scalar::<_, bool>(
                "select exists(
                    select 1
                    from global_messages
                    where id = $1 and user_id = $2 and deleted_at is null
                )",
            )
            .bind(message_id)
            .bind(request.reported_user_id)
            .fetch_one(&state.db)
            .await?;
            if exists {
                Ok(())
            } else {
                Err(RelationshipError::bad_request(
                    "reported global message was not found for that user",
                ))
            }
        }
        (Some(message_id), Some(MessageKind::Dm)) => {
            let exists = sqlx::query_scalar::<_, bool>(
                "select exists(
                    select 1
                    from dm_messages m
                    join dm_members member on member.thread_id = m.thread_id
                    where m.id = $1
                      and m.user_id = $2
                      and member.user_id = $3
                      and m.deleted_at is null
                )",
            )
            .bind(message_id)
            .bind(request.reported_user_id)
            .bind(reporter_id)
            .fetch_one(&state.db)
            .await?;
            if exists {
                Ok(())
            } else {
                Err(RelationshipError::bad_request(
                    "reported DM message was not found for that user",
                ))
            }
        }
        _ => Err(RelationshipError::bad_request(
            "message_id and message_kind must be provided together",
        )),
    }
}

fn friendship_select_sql(tail: &str) -> String {
    format!(
        "select
            f.id,
            f.status,
            f.created_at::text as created_at,
            f.updated_at::text as updated_at,
            requester.id as requester_id,
            requester.display_name as requester_display_name,
            requester.avatar_url as requester_avatar_url,
            addressee.id as addressee_id,
            addressee.display_name as addressee_display_name,
            addressee.avatar_url as addressee_avatar_url
         from friendships f
         join users requester on requester.id = f.requester_id
         join users addressee on addressee.id = f.addressee_id
         {tail}"
    )
}

fn row_to_friendship(row: &PgRow) -> RelationshipResult<FriendshipSummary> {
    Ok(FriendshipSummary {
        id: row.try_get("id")?,
        requester: UserSummary {
            id: row.try_get("requester_id")?,
            display_name: row.try_get("requester_display_name")?,
            avatar_url: row.try_get("requester_avatar_url")?,
        },
        addressee: UserSummary {
            id: row.try_get("addressee_id")?,
            display_name: row.try_get("addressee_display_name")?,
            avatar_url: row.try_get("addressee_avatar_url")?,
        },
        status: friendship_status_from_str(row.try_get::<String, _>("status")?.as_str())?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn row_to_user(row: &PgRow) -> Result<UserSummary, sqlx::Error> {
    Ok(UserSummary {
        id: row.try_get("id")?,
        display_name: row.try_get("display_name")?,
        avatar_url: row.try_get("avatar_url")?,
    })
}

fn friendship_status_from_str(value: &str) -> RelationshipResult<FriendshipStatus> {
    match value {
        "pending" => Ok(FriendshipStatus::Pending),
        "accepted" => Ok(FriendshipStatus::Accepted),
        "declined" => Ok(FriendshipStatus::Declined),
        "removed" => Ok(FriendshipStatus::Removed),
        _ => Err(RelationshipError::internal("unknown friendship status")),
    }
}

fn friendship_status_as_str(status: FriendshipStatus) -> &'static str {
    match status {
        FriendshipStatus::Pending => "pending",
        FriendshipStatus::Accepted => "accepted",
        FriendshipStatus::Declined => "declined",
        FriendshipStatus::Removed => "removed",
    }
}

fn message_kind_as_str(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Global => "global",
        MessageKind::Dm => "dm",
    }
}

fn normalize_required_text(
    value: &str,
    field_name: &'static str,
    max_len: usize,
) -> RelationshipResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(RelationshipError::bad_request(format!(
            "{field_name} cannot be empty"
        )));
    }
    if value.chars().count() > max_len {
        return Err(RelationshipError::bad_request(format!(
            "{field_name} cannot exceed {max_len} characters"
        )));
    }
    Ok(value.to_string())
}

fn normalize_optional_text(
    value: Option<&str>,
    max_len: usize,
) -> RelationshipResult<Option<String>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => normalize_required_text(value, "details", max_len).map(Some),
        None => Ok(None),
    }
}

#[derive(Debug)]
struct RelationshipError {
    status: StatusCode,
    message: String,
}

impl RelationshipError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn too_many_requests(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for RelationshipError {
    fn into_response(self) -> Response {
        let status = self.status;
        let message = self.message;
        (status, Json(ApiError { message })).into_response()
    }
}

impl From<sqlx::Error> for RelationshipError {
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(database_error) = &error {
            if database_error.code().as_deref() == Some("23514")
                && database_error.message().contains("active friendship")
            {
                return Self::conflict("friend request is blocked by an existing block");
            }
        }

        warn!(%error, "database error in relationship route");
        Self::internal("database operation failed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;

    #[test]
    fn parses_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer abc".parse().unwrap());

        assert_eq!(auth::bearer_token(&headers), Some("abc"));
    }

    #[test]
    fn normalizes_required_text() {
        assert_eq!(
            normalize_required_text(" reason ", "reason", 20).unwrap(),
            "reason"
        );
        assert!(normalize_required_text(" ", "reason", 20).is_err());
        assert!(normalize_required_text("too long", "reason", 3).is_err());
    }

    #[test]
    fn serializes_friendship_statuses_to_db_values() {
        assert_eq!(
            friendship_status_as_str(FriendshipStatus::Pending),
            "pending"
        );
        assert_eq!(
            friendship_status_as_str(FriendshipStatus::Accepted),
            "accepted"
        );
        assert_eq!(
            friendship_status_as_str(FriendshipStatus::Declined),
            "declined"
        );
        assert_eq!(
            friendship_status_as_str(FriendshipStatus::Removed),
            "removed"
        );
    }
}

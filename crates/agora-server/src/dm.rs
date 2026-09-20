use agora_common::{
    ApiError, CreateDmThreadRequest, CreateDmThreadResponse, DmMessage, DmMessageHistoryResponse,
    DmThreadListResponse, DmThreadSummary, SendDmMessageRequest, SendDmMessageResponse,
    UserSummary, MAX_MESSAGE_LEN,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use sqlx::{postgres::PgRow, PgPool, Postgres, Row, Transaction};
use tracing::warn;
use uuid::Uuid;

use crate::{auth, chat, visibility, AppState};

const DEFAULT_HISTORY_LIMIT: u16 = 50;
const MAX_HISTORY_LIMIT: u16 = 100;

type DmResult<T> = Result<T, DmError>;

#[derive(Deserialize)]
struct DmHistoryQuery {
    before: Option<String>,
    limit: Option<String>,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/dm/threads", get(list_threads).post(create_thread))
        .route(
            "/dm/threads/{thread_id}/messages",
            get(list_messages).post(send_message),
        )
}

async fn list_threads(
    State(state): State<AppState>,
    principal: auth::Principal,
) -> DmResult<Json<DmThreadListResponse>> {
    check_dm_read_limit(&state, principal.user.id).await?;

    let rows = sqlx::query(&direct_thread_select_sql(""))
        .bind(principal.user.id)
        .fetch_all(&state.db)
        .await?;
    let threads = rows
        .iter()
        .map(row_to_thread_summary)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(DmThreadListResponse { threads }))
}

async fn create_thread(
    State(state): State<AppState>,
    principal: auth::Principal,
    Json(request): Json<CreateDmThreadRequest>,
) -> DmResult<Json<CreateDmThreadResponse>> {
    check_dm_write_limit(&state, principal.user.id).await?;

    let sender_id = principal.user.id;
    let recipient_id = request.recipient_id;
    if sender_id == recipient_id {
        return Err(DmError::bad_request(
            "cannot create a direct message thread with yourself",
        ));
    }

    // Keep a block and the resulting realtime publication in the same pair ordering domain.
    let _direct_message_delivery = state
        .direct_message_delivery_locks
        .lock(sender_id, recipient_id)
        .await;
    let mut tx = state.db.begin().await?;
    lock_direct_message_pair(&mut tx, sender_id, recipient_id).await?;
    if !active_user_exists_tx(&mut tx, sender_id).await? {
        return Err(DmError::forbidden(
            "account is no longer allowed to create threads",
        ));
    }
    if !active_user_exists_tx(&mut tx, recipient_id).await? {
        return Err(DmError::not_found("recipient was not found"));
    }
    ensure_not_blocked_tx(&mut tx, sender_id, recipient_id).await?;

    let (first_user_id, second_user_id) = canonical_pair(sender_id, recipient_id);
    let (thread_id, created) = match sqlx::query_scalar::<_, Uuid>(
        "select thread_id
         from dm_direct_threads
         where first_user_id = $1 and second_user_id = $2",
    )
    .bind(first_user_id)
    .bind(second_user_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        Some(thread_id) => (thread_id, false),
        None => (
            create_direct_thread_tx(&mut tx, first_user_id, second_user_id).await?,
            true,
        ),
    };

    let thread = direct_thread_summary_by_id_tx(&mut tx, sender_id, thread_id)
        .await?
        .ok_or_else(|| DmError::internal("failed to load the direct message thread"))?;
    tx.commit().await?;

    if created {
        chat::send_direct_message_thread_updated(
            &state.chat_tx,
            thread_id,
            sender_id,
            recipient_id,
        );
    }

    Ok(Json(CreateDmThreadResponse { thread }))
}

async fn list_messages(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<DmHistoryQuery>,
    principal: auth::Principal,
) -> DmResult<Json<DmMessageHistoryResponse>> {
    check_dm_read_limit(&state, principal.user.id).await?;
    let before = parse_history_cursor(query.before.as_deref())?;
    let page_limit = parse_history_limit(query.limit.as_deref())?;

    let mut tx = state.db.begin().await?;
    let other_user_id =
        direct_thread_other_member_tx(&mut tx, thread_id, principal.user.id).await?;
    lock_direct_message_pair(&mut tx, principal.user.id, other_user_id).await?;
    if !active_user_exists_tx(&mut tx, principal.user.id).await? {
        return Err(DmError::forbidden(
            "account is no longer allowed to view direct messages",
        ));
    }
    if !active_user_exists_tx(&mut tx, other_user_id).await? {
        return Err(DmError::not_found("direct message thread was not found"));
    }
    ensure_not_blocked_tx(&mut tx, principal.user.id, other_user_id).await?;
    validate_history_cursor_tx(&mut tx, thread_id, before).await?;

    let rows = sqlx::query(
        "select
             m.id,
             m.thread_id,
             m.body,
             m.created_at::text as created_at,
             u.id as author_id,
             u.display_name as author_display_name,
             u.avatar_url as author_avatar_url
         from dm_messages m
         join users u on u.id = m.user_id
         where m.thread_id = $1
           and m.deleted_at is null
           and (
               $2::uuid is null
               or (m.created_at, m.id) < (
                   select cursor.created_at, cursor.id
                   from dm_messages cursor
                   where cursor.id = $2 and cursor.thread_id = $1
               )
           )
         order by m.created_at desc, m.id desc
         limit $3",
    )
    .bind(thread_id)
    .bind(before)
    .bind(page_limit + 1)
    .fetch_all(&mut *tx)
    .await?;

    let mut messages = rows
        .iter()
        .map(row_to_dm_message)
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = messages.len() > page_limit as usize;
    if has_more {
        messages.truncate(page_limit as usize);
    }
    let next_before_message_id = has_more
        .then(|| messages.last().map(|message| message.id))
        .flatten();
    messages.reverse();
    tx.commit().await?;

    Ok(Json(DmMessageHistoryResponse {
        messages,
        next_before_message_id,
    }))
}

async fn send_message(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    principal: auth::Principal,
    Json(request): Json<SendDmMessageRequest>,
) -> DmResult<Json<SendDmMessageResponse>> {
    state
        .rate_limits
        .check_chat_message(principal.user.id)
        .await
        .map_err(|error| DmError::too_many_requests(error.message()))?;
    let body = normalize_message_body(&request.body)?;

    let lock_user_id = direct_thread_other_member(&state.db, thread_id, principal.user.id).await?;
    // A block takes this pair lock before its relationship transaction, so a committed message
    // is published before the block or is rechecked and suppressed at delivery.
    let _direct_message_delivery = state
        .direct_message_delivery_locks
        .lock(principal.user.id, lock_user_id)
        .await;
    let mut tx = state.db.begin().await?;
    let other_user_id =
        direct_thread_other_member_tx(&mut tx, thread_id, principal.user.id).await?;
    lock_direct_message_pair(&mut tx, principal.user.id, other_user_id).await?;
    if !active_user_exists_tx(&mut tx, principal.user.id).await? {
        return Err(DmError::forbidden(
            "account is no longer allowed to send direct messages",
        ));
    }
    if !active_user_exists_tx(&mut tx, other_user_id).await? {
        return Err(DmError::forbidden("direct message thread is unavailable"));
    }
    ensure_not_blocked_tx(&mut tx, principal.user.id, other_user_id).await?;

    let Some(row) = sqlx::query(
        "insert into dm_messages (thread_id, user_id, body)
         select $1, $2, $3
         where exists (
             select 1
             from users
             where id = $2
               and banned_at is null
               and (suspended_until is null or suspended_until <= now())
         )
         returning id, thread_id, body, created_at::text as created_at",
    )
    .bind(thread_id)
    .bind(principal.user.id)
    .bind(body)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Err(DmError::forbidden(
            "account is no longer allowed to send direct messages",
        ));
    };

    let message = DmMessage {
        id: row.try_get("id")?,
        thread_id: row.try_get("thread_id")?,
        author: principal.user.clone(),
        body: row.try_get("body")?,
        created_at: row.try_get("created_at")?,
    };
    tx.commit().await?;

    chat::send_direct_message_thread_updated(
        &state.chat_tx,
        thread_id,
        principal.user.id,
        other_user_id,
    );
    chat::send_direct_message_created(
        &state.chat_tx,
        message.clone(),
        principal.user.id,
        other_user_id,
    );

    Ok(Json(SendDmMessageResponse { message }))
}

async fn create_direct_thread_tx(
    tx: &mut Transaction<'_, Postgres>,
    first_user_id: Uuid,
    second_user_id: Uuid,
) -> DmResult<Uuid> {
    let row = sqlx::query("insert into dm_threads default values returning id")
        .fetch_one(&mut **tx)
        .await?;
    let candidate_thread_id: Uuid = row.try_get("id")?;
    sqlx::query(
        "insert into dm_members (thread_id, user_id)
         values ($1, $2), ($1, $3)",
    )
    .bind(candidate_thread_id)
    .bind(first_user_id)
    .bind(second_user_id)
    .execute(&mut **tx)
    .await?;

    let inserted_thread_id = sqlx::query_scalar::<_, Uuid>(
        "insert into dm_direct_threads (thread_id, first_user_id, second_user_id)
         values ($1, $2, $3)
         on conflict (first_user_id, second_user_id) do nothing
         returning thread_id",
    )
    .bind(candidate_thread_id)
    .bind(first_user_id)
    .bind(second_user_id)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(thread_id) = inserted_thread_id {
        return Ok(thread_id);
    }

    sqlx::query("delete from dm_threads where id = $1")
        .bind(candidate_thread_id)
        .execute(&mut **tx)
        .await?;
    Ok(sqlx::query_scalar::<_, Uuid>(
        "select thread_id
         from dm_direct_threads
         where first_user_id = $1 and second_user_id = $2",
    )
    .bind(first_user_id)
    .bind(second_user_id)
    .fetch_one(&mut **tx)
    .await?)
}

async fn direct_thread_other_member_tx(
    tx: &mut Transaction<'_, Postgres>,
    thread_id: Uuid,
    user_id: Uuid,
) -> DmResult<Uuid> {
    let other_user_id = sqlx::query_scalar::<_, Uuid>(
        "select case
             when first_user_id = $2 then second_user_id
             else first_user_id
         end
         from dm_direct_threads
         where thread_id = $1
           and (first_user_id = $2 or second_user_id = $2)",
    )
    .bind(thread_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;

    other_user_id.ok_or_else(|| DmError::not_found("direct message thread was not found"))
}

async fn direct_thread_other_member(db: &PgPool, thread_id: Uuid, user_id: Uuid) -> DmResult<Uuid> {
    let other_user_id = sqlx::query_scalar::<_, Uuid>(
        "select case
             when first_user_id = $2 then second_user_id
             else first_user_id
         end
         from dm_direct_threads
         where thread_id = $1
           and (first_user_id = $2 or second_user_id = $2)",
    )
    .bind(thread_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;

    other_user_id.ok_or_else(|| DmError::not_found("direct message thread was not found"))
}

async fn direct_thread_summary_by_id_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    thread_id: Uuid,
) -> DmResult<Option<DmThreadSummary>> {
    let row = sqlx::query(&direct_thread_select_sql("and d.thread_id = $2"))
        .bind(user_id)
        .bind(thread_id)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(row.as_ref().map(row_to_thread_summary).transpose()?)
}

pub(crate) async fn direct_thread_summary_for_realtime(
    db: &PgPool,
    user_id: Uuid,
    thread_id: Uuid,
) -> Result<Option<DmThreadSummary>, sqlx::Error> {
    let row = sqlx::query(&direct_thread_select_sql("and d.thread_id = $2"))
        .bind(user_id)
        .bind(thread_id)
        .fetch_optional(db)
        .await?;
    row.as_ref().map(row_to_thread_summary).transpose()
}

async fn active_user_exists_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> DmResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "select exists(
             select 1
             from users
             where id = $1
               and banned_at is null
               and (suspended_until is null or suspended_until <= now())
         )",
    )
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?)
}

async fn ensure_not_blocked_tx(
    tx: &mut Transaction<'_, Postgres>,
    first_user_id: Uuid,
    second_user_id: Uuid,
) -> DmResult<()> {
    if visibility::blocked_between_tx(tx, first_user_id, second_user_id).await? {
        Err(DmError::forbidden(
            "direct messages are unavailable because one user blocked the other",
        ))
    } else {
        Ok(())
    }
}

async fn validate_history_cursor_tx(
    tx: &mut Transaction<'_, Postgres>,
    thread_id: Uuid,
    before: Option<Uuid>,
) -> DmResult<()> {
    let Some(before) = before else {
        return Ok(());
    };
    let belongs_to_thread = sqlx::query_scalar::<_, bool>(
        "select exists(
             select 1
             from dm_messages
             where id = $1 and thread_id = $2
         )",
    )
    .bind(before)
    .bind(thread_id)
    .fetch_one(&mut **tx)
    .await?;
    if belongs_to_thread {
        Ok(())
    } else {
        Err(DmError::bad_request(
            "pagination cursor does not belong to this thread",
        ))
    }
}

async fn lock_direct_message_pair(
    tx: &mut Transaction<'_, Postgres>,
    first_user_id: Uuid,
    second_user_id: Uuid,
) -> DmResult<()> {
    sqlx::query("select lock_relationship_pair($1, $2)")
        .bind(first_user_id)
        .bind(second_user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn check_dm_read_limit(state: &AppState, user_id: Uuid) -> DmResult<()> {
    state
        .rate_limits
        .check_chat_event(user_id)
        .await
        .map_err(|error| DmError::too_many_requests(error.message()))
}

async fn check_dm_write_limit(state: &AppState, user_id: Uuid) -> DmResult<()> {
    state
        .rate_limits
        .check_chat_event(user_id)
        .await
        .map_err(|error| DmError::too_many_requests(error.message()))
}

fn direct_thread_select_sql(additional_conditions: &str) -> String {
    let not_blocked = visibility::not_blocked_between_sql("$1", "other.id");
    format!(
        "select
             d.thread_id as id,
             t.created_at::text as created_at,
             other.id as other_user_id,
             other.display_name as other_user_display_name,
             other.avatar_url as other_user_avatar_url,
             latest.created_at::text as last_message_at
         from dm_direct_threads d
          join dm_threads t on t.id = d.thread_id
          join dm_members viewer_member
            on viewer_member.thread_id = d.thread_id and viewer_member.user_id = $1
          join users other on other.id = case
             when d.first_user_id = $1 then d.second_user_id
             else d.first_user_id
         end
         left join lateral (
             select m.created_at
             from dm_messages m
             where m.thread_id = d.thread_id and m.deleted_at is null
             order by m.created_at desc, m.id desc
             limit 1
         ) latest on true
          where (d.first_user_id = $1 or d.second_user_id = $1)
            and exists (
                select 1
                from users viewer
                where viewer.id = $1
                  and viewer.banned_at is null
                  and (viewer.suspended_until is null or viewer.suspended_until <= now())
            )
            and other.banned_at is null
           and (other.suspended_until is null or other.suspended_until <= now())
           and {not_blocked}
           {additional_conditions}
         order by coalesce(latest.created_at, t.created_at) desc, d.thread_id desc"
    )
}

fn row_to_thread_summary(row: &PgRow) -> Result<DmThreadSummary, sqlx::Error> {
    Ok(DmThreadSummary {
        id: row.try_get("id")?,
        other_user: UserSummary {
            id: row.try_get("other_user_id")?,
            display_name: row.try_get("other_user_display_name")?,
            avatar_url: row.try_get("other_user_avatar_url")?,
        },
        created_at: row.try_get("created_at")?,
        last_message_at: row.try_get("last_message_at")?,
    })
}

fn row_to_dm_message(row: &PgRow) -> Result<DmMessage, sqlx::Error> {
    Ok(DmMessage {
        id: row.try_get("id")?,
        thread_id: row.try_get("thread_id")?,
        author: UserSummary {
            id: row.try_get("author_id")?,
            display_name: row.try_get("author_display_name")?,
            avatar_url: row.try_get("author_avatar_url")?,
        },
        body: row.try_get("body")?,
        created_at: row.try_get("created_at")?,
    })
}

fn canonical_pair(first_user_id: Uuid, second_user_id: Uuid) -> (Uuid, Uuid) {
    if first_user_id < second_user_id {
        (first_user_id, second_user_id)
    } else {
        (second_user_id, first_user_id)
    }
}

fn normalize_message_body(body: &str) -> DmResult<String> {
    let body = body.trim();
    if body.is_empty() {
        return Err(DmError::bad_request("message cannot be empty"));
    }
    if body.chars().count() > MAX_MESSAGE_LEN {
        return Err(DmError::bad_request(format!(
            "message cannot exceed {MAX_MESSAGE_LEN} characters"
        )));
    }
    Ok(body.to_string())
}

fn parse_history_cursor(value: Option<&str>) -> DmResult<Option<Uuid>> {
    let Some(value) = value else {
        return Ok(None);
    };
    Uuid::parse_str(value.trim())
        .map(Some)
        .map_err(|_| DmError::bad_request("before must be a UUID"))
}

fn parse_history_limit(value: Option<&str>) -> DmResult<i64> {
    let Some(value) = value else {
        return Ok(i64::from(DEFAULT_HISTORY_LIMIT));
    };
    let limit = value
        .trim()
        .parse::<u16>()
        .map_err(|_| DmError::bad_request("limit must be a positive integer"))?;
    if limit == 0 || limit > MAX_HISTORY_LIMIT {
        return Err(DmError::bad_request(format!(
            "limit must be between 1 and {MAX_HISTORY_LIMIT}"
        )));
    }
    Ok(i64::from(limit))
}

#[derive(Debug)]
struct DmError {
    status: StatusCode,
    message: String,
}

impl DmError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
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

impl IntoResponse for DmError {
    fn into_response(self) -> Response {
        let status = self.status;
        let message = self.message;
        (status, Json(ApiError { message })).into_response()
    }
}

impl From<sqlx::Error> for DmError {
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(database_error) = &error {
            if database_error.code().as_deref() == Some("23514")
                && database_error.message().contains(
                    "direct message cannot be sent while either user has blocked the other",
                )
            {
                return Self::forbidden(
                    "direct messages are unavailable because one user blocked the other",
                );
            }
        }

        warn!(%error, "database error in direct message route");
        Self::internal("database operation failed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_direct_message_bodies() {
        assert_eq!(normalize_message_body(" hello ").unwrap(), "hello");
        assert!(normalize_message_body("  ").is_err());
        assert!(normalize_message_body(&"x".repeat(MAX_MESSAGE_LEN + 1)).is_err());
    }

    #[test]
    fn validates_history_pagination_values() {
        assert_eq!(
            parse_history_limit(None).unwrap(),
            i64::from(DEFAULT_HISTORY_LIMIT)
        );
        assert_eq!(parse_history_limit(Some("25")).unwrap(), 25);
        assert!(parse_history_limit(Some("0")).is_err());
        assert!(parse_history_limit(Some("101")).is_err());
        assert!(parse_history_limit(Some("invalid")).is_err());
    }

    #[test]
    fn validates_history_cursor() {
        let id = Uuid::new_v4();

        assert_eq!(parse_history_cursor(None).unwrap(), None);
        assert_eq!(
            parse_history_cursor(Some(&id.to_string())).unwrap(),
            Some(id)
        );
        assert!(parse_history_cursor(Some("invalid")).is_err());
    }

    #[test]
    fn orders_direct_message_pairs_canonically() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);

        assert_eq!(canonical_pair(first, second), (first, second));
        assert_eq!(canonical_pair(second, first), (first, second));
    }

    #[test]
    fn direct_message_summary_sql_rechecks_membership_and_block_visibility() {
        let sql = direct_thread_select_sql("and d.thread_id = $2");

        assert!(sql.contains("join dm_members viewer_member"));
        assert!(sql.contains("viewer_member.user_id = $1"));
        assert!(sql.contains("from blocks b"));
        assert!(sql.contains("and d.thread_id = $2"));
    }
}

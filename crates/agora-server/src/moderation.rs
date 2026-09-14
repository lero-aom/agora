use std::net::SocketAddr;

use agora_common::{
    ApiError, BanUserRequest, DeleteGlobalMessageRequest, MessageKind, ModerationActionKind,
    ModerationActionResponse, ModerationActionSummary, ModerationReasonRequest, ReportDetail,
    ReportDetailResponse, ReportListResponse, ReportStatus, ReportSummary, ReportedMessage,
    SuspendUserRequest, UserRole, UserSummary,
};
use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use sqlx::{postgres::PgRow, PgPool, Postgres, Row, Transaction};
use tracing::warn;
use uuid::Uuid;

use crate::{auth, chat, AppState};

const MAX_REASON_LEN: usize = 500;
const MAX_SUSPEND_SECONDS: u64 = 30 * 24 * 60 * 60;

type ModerationResult<T> = Result<T, ModerationError>;

#[derive(Deserialize)]
struct ReportListQuery {
    status: Option<ReportStatus>,
}

struct ModeratedUser {
    role: UserRole,
}

struct ReportActionTarget {
    reported_user_id: Uuid,
    message_id: Option<Uuid>,
    message_kind: Option<MessageKind>,
    status: ReportStatus,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/staff", get(staff_console))
        .route("/staff/reports", get(list_reports))
        .route("/staff/reports/{report_id}", get(get_report))
        .route("/staff/reports/{report_id}/resolve", post(resolve_report))
        .route("/staff/reports/{report_id}/dismiss", post(dismiss_report))
        .route(
            "/staff/global-messages/{message_id}/delete",
            post(delete_global_message),
        )
        .route("/staff/users/{user_id}/suspend", post(suspend_user))
        .route("/staff/users/{user_id}/ban", post(ban_user))
        .route("/staff/users/{user_id}/unban", post(unban_user))
}

async fn staff_console() -> Html<&'static str> {
    Html(STAFF_CONSOLE_HTML)
}

async fn list_reports(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    principal: auth::Principal,
    Query(query): Query<ReportListQuery>,
) -> ModerationResult<Json<ReportListResponse>> {
    check_staff_read_limit(&state, peer_addr, &headers).await?;
    require_moderator(&principal)?;

    let status = query.status.map(report_status_as_str);
    let sql = report_select_sql(
        "where ($1::text is null or r.status = $1)
         order by r.created_at desc
         limit 100",
    );
    let rows = sqlx::query(&sql).bind(status).fetch_all(&state.db).await?;

    Ok(Json(ReportListResponse {
        reports: rows
            .iter()
            .map(row_to_report_summary)
            .collect::<ModerationResult<Vec<_>>>()?,
    }))
}

async fn get_report(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    principal: auth::Principal,
    Path(report_id): Path<Uuid>,
) -> ModerationResult<Json<ReportDetailResponse>> {
    check_staff_read_limit(&state, peer_addr, &headers).await?;
    require_moderator(&principal)?;

    Ok(Json(ReportDetailResponse {
        report: report_detail_by_id(&state.db, report_id).await?,
    }))
}

async fn resolve_report(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    principal: auth::Principal,
    Path(report_id): Path<Uuid>,
    Json(request): Json<ModerationReasonRequest>,
) -> ModerationResult<Json<ReportDetailResponse>> {
    check_staff_write_limit(&state, peer_addr, &headers).await?;
    let report = close_report(
        &state,
        &principal,
        report_id,
        ReportStatus::Resolved,
        ModerationActionKind::ResolveReport,
        &request.reason,
    )
    .await?;

    Ok(Json(ReportDetailResponse { report }))
}

async fn dismiss_report(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    principal: auth::Principal,
    Path(report_id): Path<Uuid>,
    Json(request): Json<ModerationReasonRequest>,
) -> ModerationResult<Json<ReportDetailResponse>> {
    check_staff_write_limit(&state, peer_addr, &headers).await?;
    let report = close_report(
        &state,
        &principal,
        report_id,
        ReportStatus::Dismissed,
        ModerationActionKind::DismissReport,
        &request.reason,
    )
    .await?;

    Ok(Json(ReportDetailResponse { report }))
}

async fn delete_global_message(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    principal: auth::Principal,
    Path(message_id): Path<Uuid>,
    Json(request): Json<DeleteGlobalMessageRequest>,
) -> ModerationResult<Json<ModerationActionResponse>> {
    check_staff_write_limit(&state, peer_addr, &headers).await?;
    require_moderator(&principal)?;
    let reason = normalize_reason(&request.reason)?;

    let mut tx = state.db.begin().await?;
    let Some(row) = sqlx::query(
        "select
            m.user_id,
            m.deleted_at is not null as already_deleted,
            u.role as author_role
         from global_messages m
         join users u on u.id = m.user_id
         where m.id = $1
         for update of m",
    )
    .bind(message_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        tx.rollback().await?;
        return Err(ModerationError::not_found("global message was not found"));
    };
    let target_user_id = row.try_get("user_id")?;
    let target_role = user_role_from_db(row.try_get::<String, _>("author_role")?.as_str());
    require_can_moderate_target(&principal, target_role)?;

    claim_report_for_action(
        &mut tx,
        request.report_id,
        target_user_id,
        Some(message_id),
        Some(MessageKind::Global),
        &principal,
    )
    .await?;

    if !row.try_get::<bool, _>("already_deleted")? {
        sqlx::query(
            "update global_messages
             set deleted_at = now(), deleted_by = $2
             where id = $1 and deleted_at is null",
        )
        .bind(message_id)
        .bind(principal.user.id)
        .execute(&mut *tx)
        .await?;
    }

    let action = insert_moderation_action(
        &mut tx,
        &principal,
        target_user_id,
        ModerationActionKind::DeleteGlobalMessage,
        &reason,
        request.report_id,
        Some(message_id),
        Some(MessageKind::Global),
        false,
    )
    .await?;
    tx.commit().await?;
    chat::send_global_message_deleted(&state.chat_tx, message_id);

    Ok(Json(ModerationActionResponse {
        action,
        revoked_sessions: 0,
    }))
}

async fn suspend_user(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    principal: auth::Principal,
    Path(user_id): Path<Uuid>,
    Json(request): Json<SuspendUserRequest>,
) -> ModerationResult<Json<ModerationActionResponse>> {
    check_staff_write_limit(&state, peer_addr, &headers).await?;
    require_moderator(&principal)?;
    require_not_self(&principal, user_id)?;
    let reason = normalize_reason(&request.reason)?;
    validate_suspend_duration(request.duration_seconds)?;

    let mut tx = state.db.begin().await?;
    let target = user_for_moderation_tx(&mut tx, user_id).await?;
    require_can_moderate_target(&principal, target.role)?;
    claim_report_for_action(&mut tx, request.report_id, user_id, None, None, &principal).await?;
    sqlx::query(
        "update users
         set suspended_until = now() + ($2::bigint * interval '1 second')
         where id = $1",
    )
    .bind(user_id)
    .bind(request.duration_seconds as i64)
    .execute(&mut *tx)
    .await?;
    let revoked_sessions = revoke_user_sessions_tx(&mut tx, user_id).await?;
    let action = insert_moderation_action(
        &mut tx,
        &principal,
        user_id,
        ModerationActionKind::Suspend,
        &reason,
        request.report_id,
        None,
        None,
        true,
    )
    .await?;
    tx.commit().await?;
    send_session_revocations(&state, &revoked_sessions);

    Ok(Json(ModerationActionResponse {
        action,
        revoked_sessions: revoked_sessions.len() as u64,
    }))
}

async fn ban_user(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    principal: auth::Principal,
    Path(user_id): Path<Uuid>,
    Json(request): Json<BanUserRequest>,
) -> ModerationResult<Json<ModerationActionResponse>> {
    check_staff_write_limit(&state, peer_addr, &headers).await?;
    require_admin(&principal)?;
    require_not_self(&principal, user_id)?;
    let reason = normalize_reason(&request.reason)?;

    let mut tx = state.db.begin().await?;
    let _ = user_for_moderation_tx(&mut tx, user_id).await?;
    claim_report_for_action(&mut tx, request.report_id, user_id, None, None, &principal).await?;
    sqlx::query(
        "update users
         set banned_at = coalesce(banned_at, now()), suspended_until = null
         where id = $1",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    let revoked_sessions = revoke_user_sessions_tx(&mut tx, user_id).await?;
    let action = insert_moderation_action(
        &mut tx,
        &principal,
        user_id,
        ModerationActionKind::Ban,
        &reason,
        request.report_id,
        None,
        None,
        false,
    )
    .await?;
    tx.commit().await?;
    send_session_revocations(&state, &revoked_sessions);

    Ok(Json(ModerationActionResponse {
        action,
        revoked_sessions: revoked_sessions.len() as u64,
    }))
}

async fn unban_user(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    principal: auth::Principal,
    Path(user_id): Path<Uuid>,
    Json(request): Json<ModerationReasonRequest>,
) -> ModerationResult<Json<ModerationActionResponse>> {
    check_staff_write_limit(&state, peer_addr, &headers).await?;
    require_admin(&principal)?;
    require_not_self(&principal, user_id)?;
    let reason = normalize_reason(&request.reason)?;

    let mut tx = state.db.begin().await?;
    let _ = user_for_moderation_tx(&mut tx, user_id).await?;
    sqlx::query("update users set banned_at = null where id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let action = insert_moderation_action(
        &mut tx,
        &principal,
        user_id,
        ModerationActionKind::Unban,
        &reason,
        None,
        None,
        None,
        false,
    )
    .await?;
    tx.commit().await?;

    Ok(Json(ModerationActionResponse {
        action,
        revoked_sessions: 0,
    }))
}

async fn close_report(
    state: &AppState,
    principal: &auth::Principal,
    report_id: Uuid,
    status: ReportStatus,
    action: ModerationActionKind,
    reason: &str,
) -> ModerationResult<ReportDetail> {
    require_moderator(principal)?;
    let reason = normalize_reason(reason)?;
    let mut tx = state.db.begin().await?;
    let report = report_for_update_tx(&mut tx, report_id).await?;
    if report.status != ReportStatus::Open {
        tx.rollback().await?;
        return Err(ModerationError::conflict("report is not open"));
    }

    sqlx::query(
        "update reports
         set status = $2, resolved_at = now(), resolved_by = $3
         where id = $1",
    )
    .bind(report_id)
    .bind(report_status_as_str(status))
    .bind(principal.user.id)
    .execute(&mut *tx)
    .await?;
    insert_moderation_action(
        &mut tx,
        principal,
        report.reported_user_id,
        action,
        &reason,
        Some(report_id),
        report.message_id,
        report.message_kind,
        false,
    )
    .await?;
    tx.commit().await?;

    report_detail_by_id(&state.db, report_id).await
}

async fn claim_report_for_action(
    tx: &mut Transaction<'_, Postgres>,
    report_id: Option<Uuid>,
    target_user_id: Uuid,
    message_id: Option<Uuid>,
    message_kind: Option<MessageKind>,
    principal: &auth::Principal,
) -> ModerationResult<()> {
    let Some(report_id) = report_id else {
        return Ok(());
    };
    let report = report_for_update_tx(tx, report_id).await?;
    if report.status != ReportStatus::Open {
        return Err(ModerationError::conflict("report is not open"));
    }
    if report.reported_user_id != target_user_id {
        return Err(ModerationError::bad_request(
            "report target does not match moderation target",
        ));
    }
    if message_id.is_some()
        && (report.message_id != message_id || report.message_kind != message_kind)
    {
        return Err(ModerationError::bad_request(
            "report message does not match moderation target",
        ));
    }

    sqlx::query(
        "update reports
         set status = 'resolved', resolved_at = now(), resolved_by = $2
         where id = $1",
    )
    .bind(report_id)
    .bind(principal.user.id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn report_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    report_id: Uuid,
) -> ModerationResult<ReportActionTarget> {
    let Some(row) = sqlx::query(
        "select reported_user_id, message_id, message_kind, status
         from reports
         where id = $1
         for update",
    )
    .bind(report_id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Err(ModerationError::not_found("report was not found"));
    };

    Ok(ReportActionTarget {
        reported_user_id: row.try_get("reported_user_id")?,
        message_id: row.try_get("message_id")?,
        message_kind: optional_message_kind(row.try_get::<Option<String>, _>("message_kind")?)?,
        status: report_status_from_str(row.try_get::<String, _>("status")?.as_str())?,
    })
}

async fn report_detail_by_id(db: &PgPool, report_id: Uuid) -> ModerationResult<ReportDetail> {
    let sql = report_select_sql("where r.id = $1");
    let Some(row) = sqlx::query(&sql).bind(report_id).fetch_optional(db).await? else {
        return Err(ModerationError::not_found("report was not found"));
    };
    let summary = row_to_report_summary(&row)?;
    let message = match (summary.message_id, summary.message_kind) {
        (Some(message_id), Some(kind)) => reported_message_by_id(db, message_id, kind).await?,
        _ => None,
    };
    let actions = moderation_actions_for_report(db, report_id).await?;

    Ok(ReportDetail {
        summary,
        details: row.try_get("details")?,
        message,
        actions,
    })
}

async fn reported_message_by_id(
    db: &PgPool,
    message_id: Uuid,
    kind: MessageKind,
) -> ModerationResult<Option<ReportedMessage>> {
    let row = match kind {
        MessageKind::Global => {
            sqlx::query(
                "select
                m.id,
                m.body,
                m.created_at::text as created_at,
                m.deleted_at::text as deleted_at,
                u.id as author_id,
                u.display_name as author_display_name,
                u.avatar_url as author_avatar_url
             from global_messages m
             join users u on u.id = m.user_id
             where m.id = $1",
            )
            .bind(message_id)
            .fetch_optional(db)
            .await?
        }
        // DM bodies stay private until a staff-access policy is defined for DMs.
        MessageKind::Dm => {
            sqlx::query(
                "select
                m.id,
                null::text as body,
                m.created_at::text as created_at,
                m.deleted_at::text as deleted_at,
                u.id as author_id,
                u.display_name as author_display_name,
                u.avatar_url as author_avatar_url
             from dm_messages m
             join users u on u.id = m.user_id
             where m.id = $1",
            )
            .bind(message_id)
            .fetch_optional(db)
            .await?
        }
    };

    row.as_ref()
        .map(|row| {
            Ok(ReportedMessage {
                id: row.try_get("id")?,
                kind,
                author: row_to_prefixed_user(row, "author")?,
                body: row.try_get("body")?,
                created_at: display_db_timestamp(&row.try_get::<String, _>("created_at")?),
                deleted_at: row
                    .try_get::<Option<String>, _>("deleted_at")?
                    .map(|value| display_db_timestamp(&value)),
            })
        })
        .transpose()
}

async fn moderation_actions_for_report(
    db: &PgPool,
    report_id: Uuid,
) -> ModerationResult<Vec<ModerationActionSummary>> {
    let sql = moderation_action_select_sql(
        "where a.report_id = $1
         order by a.created_at desc",
    );
    let rows = sqlx::query(&sql).bind(report_id).fetch_all(db).await?;
    rows.iter()
        .map(row_to_moderation_action)
        .collect::<ModerationResult<Vec<_>>>()
}

async fn insert_moderation_action(
    tx: &mut Transaction<'_, Postgres>,
    principal: &auth::Principal,
    target_user_id: Uuid,
    action: ModerationActionKind,
    reason: &str,
    report_id: Option<Uuid>,
    message_id: Option<Uuid>,
    message_kind: Option<MessageKind>,
    expires_from_target: bool,
) -> ModerationResult<ModerationActionSummary> {
    let expires_at_sql = if expires_from_target {
        "(select suspended_until from users where id = $2)"
    } else {
        "null"
    };
    let sql = format!(
        "insert into moderation_actions (
            moderator_id,
            target_user_id,
            action,
            reason,
            report_id,
            message_id,
            message_kind,
            expires_at
         ) values ($1, $2, $3, $4, $5, $6, $7, {expires_at_sql})
         returning id"
    );
    let row = sqlx::query(&sql)
        .bind(principal.user.id)
        .bind(target_user_id)
        .bind(moderation_action_as_str(action))
        .bind(reason)
        .bind(report_id)
        .bind(message_id)
        .bind(message_kind.map(message_kind_as_str))
        .fetch_one(&mut **tx)
        .await?;
    moderation_action_by_id_tx(tx, row.try_get("id")?).await
}

async fn moderation_action_by_id_tx(
    tx: &mut Transaction<'_, Postgres>,
    action_id: Uuid,
) -> ModerationResult<ModerationActionSummary> {
    let sql = moderation_action_select_sql("where a.id = $1");
    let row = sqlx::query(&sql)
        .bind(action_id)
        .fetch_one(&mut **tx)
        .await?;
    row_to_moderation_action(&row)
}

async fn user_for_moderation_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> ModerationResult<ModeratedUser> {
    let Some(row) = sqlx::query(
        "select id, display_name, avatar_url, role
         from users
         where id = $1
         for update",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Err(ModerationError::not_found("user was not found"));
    };

    Ok(ModeratedUser {
        role: user_role_from_db(row.try_get::<String, _>("role")?.as_str()),
    })
}

async fn revoke_user_sessions_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> ModerationResult<Vec<Uuid>> {
    let rows = sqlx::query(
        "update sessions
         set revoked_at = now()
         where user_id = $1 and revoked_at is null
         returning id",
    )
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await?;

    rows.iter()
        .map(|row| row.try_get("id").map_err(ModerationError::from))
        .collect()
}

fn send_session_revocations(state: &AppState, session_ids: &[Uuid]) {
    for session_id in session_ids {
        chat::send_session_revoked(&state.chat_tx, *session_id);
    }
}

async fn check_staff_read_limit(
    state: &AppState,
    peer_addr: SocketAddr,
    headers: &HeaderMap,
) -> ModerationResult<()> {
    state
        .rate_limits
        .check_relationship_read(peer_addr, headers, state.config.trust_proxy_headers)
        .await
        .map_err(|error| ModerationError::too_many_requests(error.message()))
}

async fn check_staff_write_limit(
    state: &AppState,
    peer_addr: SocketAddr,
    headers: &HeaderMap,
) -> ModerationResult<()> {
    state
        .rate_limits
        .check_relationship_write(peer_addr, headers, state.config.trust_proxy_headers)
        .await
        .map_err(|error| ModerationError::too_many_requests(error.message()))
}

fn require_moderator(principal: &auth::Principal) -> ModerationResult<()> {
    if principal.is_moderator() {
        Ok(())
    } else {
        Err(ModerationError::forbidden("moderator access is required"))
    }
}

fn require_admin(principal: &auth::Principal) -> ModerationResult<()> {
    if principal.is_admin() {
        Ok(())
    } else {
        Err(ModerationError::forbidden("admin access is required"))
    }
}

fn require_not_self(principal: &auth::Principal, target_user_id: Uuid) -> ModerationResult<()> {
    if principal.user.id == target_user_id {
        Err(ModerationError::bad_request("cannot moderate yourself"))
    } else {
        Ok(())
    }
}

fn require_can_moderate_target(
    principal: &auth::Principal,
    target_role: UserRole,
) -> ModerationResult<()> {
    if principal.is_admin()
        || (principal.role == UserRole::Moderator && target_role == UserRole::User)
    {
        Ok(())
    } else {
        Err(ModerationError::forbidden(
            "moderators can only act on regular users",
        ))
    }
}

fn validate_suspend_duration(duration_seconds: u64) -> ModerationResult<()> {
    if duration_seconds == 0 {
        return Err(ModerationError::bad_request(
            "suspension duration must be positive",
        ));
    }
    if duration_seconds > MAX_SUSPEND_SECONDS {
        return Err(ModerationError::bad_request(format!(
            "suspension duration cannot exceed {MAX_SUSPEND_SECONDS} seconds"
        )));
    }
    Ok(())
}

fn normalize_reason(reason: &str) -> ModerationResult<String> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(ModerationError::bad_request("reason is required"));
    }
    if reason.chars().count() > MAX_REASON_LEN {
        return Err(ModerationError::bad_request(format!(
            "reason cannot exceed {MAX_REASON_LEN} characters"
        )));
    }
    Ok(reason.to_string())
}

fn report_select_sql(tail: &str) -> String {
    format!(
        "select
            r.id,
            r.message_id,
            r.message_kind,
            r.reason,
            r.details,
            r.status,
            r.created_at::text as created_at,
            r.resolved_at::text as resolved_at,
            reporter.id as reporter_id,
            reporter.display_name as reporter_display_name,
            reporter.avatar_url as reporter_avatar_url,
            reported.id as reported_user_id,
            reported.display_name as reported_user_display_name,
            reported.avatar_url as reported_user_avatar_url,
            resolver.id as resolved_by_id,
            resolver.display_name as resolved_by_display_name,
            resolver.avatar_url as resolved_by_avatar_url
         from reports r
         join users reporter on reporter.id = r.reporter_id
         join users reported on reported.id = r.reported_user_id
         left join users resolver on resolver.id = r.resolved_by
         {tail}"
    )
}

fn moderation_action_select_sql(tail: &str) -> String {
    format!(
        "select
            a.id,
            a.report_id,
            a.message_id,
            a.message_kind,
            a.action,
            a.reason,
            a.created_at::text as created_at,
            a.expires_at::text as expires_at,
            moderator.id as moderator_id,
            moderator.display_name as moderator_display_name,
            moderator.avatar_url as moderator_avatar_url,
            target.id as target_user_id,
            target.display_name as target_user_display_name,
            target.avatar_url as target_user_avatar_url
         from moderation_actions a
         join users moderator on moderator.id = a.moderator_id
         join users target on target.id = a.target_user_id
         {tail}"
    )
}

fn row_to_report_summary(row: &PgRow) -> ModerationResult<ReportSummary> {
    Ok(ReportSummary {
        id: row.try_get("id")?,
        reporter: row_to_prefixed_user(row, "reporter")?,
        reported_user: row_to_prefixed_user(row, "reported_user")?,
        message_id: row.try_get("message_id")?,
        message_kind: optional_message_kind(row.try_get::<Option<String>, _>("message_kind")?)?,
        reason: row.try_get("reason")?,
        status: report_status_from_str(row.try_get::<String, _>("status")?.as_str())?,
        created_at: display_db_timestamp(&row.try_get::<String, _>("created_at")?),
        resolved_at: row
            .try_get::<Option<String>, _>("resolved_at")?
            .map(|value| display_db_timestamp(&value)),
        resolved_by: row_to_optional_prefixed_user(row, "resolved_by")?,
    })
}

fn row_to_moderation_action(row: &PgRow) -> ModerationResult<ModerationActionSummary> {
    Ok(ModerationActionSummary {
        id: row.try_get("id")?,
        moderator: row_to_prefixed_user(row, "moderator")?,
        target_user: row_to_prefixed_user(row, "target_user")?,
        report_id: row.try_get("report_id")?,
        message_id: row.try_get("message_id")?,
        message_kind: optional_message_kind(row.try_get::<Option<String>, _>("message_kind")?)?,
        action: moderation_action_from_str(row.try_get::<String, _>("action")?.as_str())?,
        reason: row.try_get("reason")?,
        created_at: display_db_timestamp(&row.try_get::<String, _>("created_at")?),
        expires_at: row
            .try_get::<Option<String>, _>("expires_at")?
            .map(|value| display_db_timestamp(&value)),
    })
}

fn row_to_prefixed_user(row: &PgRow, prefix: &str) -> Result<UserSummary, sqlx::Error> {
    Ok(UserSummary {
        id: row.try_get(format!("{prefix}_id").as_str())?,
        display_name: row.try_get(format!("{prefix}_display_name").as_str())?,
        avatar_url: row.try_get(format!("{prefix}_avatar_url").as_str())?,
    })
}

fn row_to_optional_prefixed_user(
    row: &PgRow,
    prefix: &str,
) -> Result<Option<UserSummary>, sqlx::Error> {
    let id_column = format!("{prefix}_id");
    let Some(id) = row.try_get::<Option<Uuid>, _>(id_column.as_str())? else {
        return Ok(None);
    };
    Ok(Some(UserSummary {
        id,
        display_name: row.try_get(format!("{prefix}_display_name").as_str())?,
        avatar_url: row.try_get(format!("{prefix}_avatar_url").as_str())?,
    }))
}

fn user_role_from_db(value: &str) -> UserRole {
    match value {
        "moderator" => UserRole::Moderator,
        "admin" => UserRole::Admin,
        _ => UserRole::User,
    }
}

fn report_status_as_str(status: ReportStatus) -> &'static str {
    match status {
        ReportStatus::Open => "open",
        ReportStatus::Resolved => "resolved",
        ReportStatus::Dismissed => "dismissed",
    }
}

fn report_status_from_str(value: &str) -> ModerationResult<ReportStatus> {
    match value {
        "open" => Ok(ReportStatus::Open),
        "resolved" => Ok(ReportStatus::Resolved),
        "dismissed" => Ok(ReportStatus::Dismissed),
        _ => Err(ModerationError::internal("unknown report status")),
    }
}

fn message_kind_as_str(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Global => "global",
        MessageKind::Dm => "dm",
    }
}

fn optional_message_kind(value: Option<String>) -> ModerationResult<Option<MessageKind>> {
    value.as_deref().map(message_kind_from_str).transpose()
}

fn message_kind_from_str(value: &str) -> ModerationResult<MessageKind> {
    match value {
        "global" => Ok(MessageKind::Global),
        "dm" => Ok(MessageKind::Dm),
        _ => Err(ModerationError::internal("unknown message kind")),
    }
}

fn moderation_action_as_str(action: ModerationActionKind) -> &'static str {
    match action {
        ModerationActionKind::DeleteGlobalMessage => "delete_global_message",
        ModerationActionKind::Suspend => "suspend",
        ModerationActionKind::Ban => "ban",
        ModerationActionKind::Unban => "unban",
        ModerationActionKind::ResolveReport => "resolve_report",
        ModerationActionKind::DismissReport => "dismiss_report",
    }
}

fn moderation_action_from_str(value: &str) -> ModerationResult<ModerationActionKind> {
    match value {
        "delete_global_message" => Ok(ModerationActionKind::DeleteGlobalMessage),
        "suspend" => Ok(ModerationActionKind::Suspend),
        "ban" => Ok(ModerationActionKind::Ban),
        "unban" => Ok(ModerationActionKind::Unban),
        "resolve_report" => Ok(ModerationActionKind::ResolveReport),
        "dismiss_report" => Ok(ModerationActionKind::DismissReport),
        _ => Err(ModerationError::internal("unknown moderation action")),
    }
}

fn display_db_timestamp(value: &str) -> String {
    let value = value.trim();
    if has_timestamp_prefix(value) {
        value[..19].replace('T', " ")
    } else {
        value.to_string()
    }
}

fn has_timestamp_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 19
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .into_iter()
            .all(|index| bytes[index].is_ascii_digit())
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && matches!(bytes[10], b' ' | b'T')
        && bytes[13] == b':'
        && bytes[16] == b':'
}

const STAFF_CONSOLE_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Agora Staff</title>
<style>
:root {
  color-scheme: dark;
  --bg: #091018;
  --panel: #101a27;
  --panel-soft: #142235;
  --line: #28405f;
  --text: #edf5ff;
  --muted: #99aec9;
  --accent: #66d9ef;
  --danger: #ff7676;
}
* { box-sizing: border-box; }
body {
  margin: 0;
  background: radial-gradient(circle at top left, #17314a, var(--bg) 36rem);
  color: var(--text);
  font-family: Inter, Segoe UI, system-ui, sans-serif;
}
main { width: min(1180px, calc(100vw - 32px)); margin: 32px auto; }
header { display: flex; align-items: end; justify-content: space-between; gap: 16px; margin-bottom: 20px; }
h1, h2, h3, p { margin: 0; }
h1 { font-size: clamp(28px, 5vw, 52px); letter-spacing: -0.05em; }
.muted { color: var(--muted); }
.panel {
  background: color-mix(in srgb, var(--panel), transparent 6%);
  border: 1px solid var(--line);
  border-radius: 18px;
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.3);
}
.auth { display: grid; grid-template-columns: 1fr auto auto; gap: 10px; padding: 14px; margin-bottom: 18px; }
input, select, button {
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--panel-soft);
  color: var(--text);
  padding: 10px 12px;
  font: inherit;
}
button { cursor: pointer; background: #173657; }
button:hover { border-color: var(--accent); }
button.danger { background: #4a1d24; color: #ffd7dc; }
button.secondary { background: #172335; }
.grid { display: grid; grid-template-columns: minmax(280px, 420px) 1fr; gap: 18px; align-items: start; }
.toolbar { display: flex; gap: 10px; padding: 14px; border-bottom: 1px solid var(--line); }
.report-list { display: grid; gap: 8px; padding: 12px; max-height: 68vh; overflow: auto; }
.report-card {
  display: grid;
  gap: 5px;
  text-align: left;
  background: #0f1c2b;
}
.report-card.active { outline: 2px solid var(--accent); }
.detail { padding: 18px; display: grid; gap: 16px; min-height: 360px; }
.detail-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
.box { background: #0d1825; border: 1px solid var(--line); border-radius: 14px; padding: 14px; display: grid; gap: 6px; }
.actions { display: flex; flex-wrap: wrap; gap: 10px; }
.audit { display: grid; gap: 8px; }
.audit-row { border: 1px solid var(--line); border-radius: 12px; padding: 10px; background: #0c1622; }
.status { min-height: 24px; color: var(--muted); margin: 12px 0; }
code { color: var(--accent); word-break: break-all; }
@media (max-width: 820px) {
  main { width: min(100vw - 20px, 680px); margin: 16px auto; }
  header, .auth, .grid, .detail-grid { grid-template-columns: 1fr; display: grid; }
  .toolbar, .actions { flex-direction: column; }
}
</style>
</head>
<body>
<main>
  <header>
    <div>
      <p class="muted">Agora moderation</p>
      <h1>Staff Console</h1>
    </div>
    <p class="muted">REST-backed triage, sanctions, and audit review.</p>
  </header>

  <section class="panel auth">
    <input id="token" type="password" placeholder="Paste staff access token">
    <select id="statusFilter">
      <option value="open">Open reports</option>
      <option value="resolved">Resolved reports</option>
      <option value="dismissed">Dismissed reports</option>
      <option value="">All reports</option>
    </select>
    <button id="saveToken">Save and Load</button>
  </section>

  <p id="status" class="status">Enter a moderator or admin bearer token to load reports.</p>

  <section class="grid">
    <div class="panel">
      <div class="toolbar">
        <button id="reload">Reload</button>
      </div>
      <div id="reports" class="report-list"></div>
    </div>
    <div id="detail" class="panel detail">
      <p class="muted">Select a report to inspect details and take action.</p>
    </div>
  </section>
</main>

<script>
const state = { token: localStorage.getItem("agora_staff_token") || "", reports: [], selected: null };
const tokenInput = document.querySelector("#token");
const statusFilter = document.querySelector("#statusFilter");
const statusEl = document.querySelector("#status");
const reportsEl = document.querySelector("#reports");
const detailEl = document.querySelector("#detail");
tokenInput.value = state.token;

function setStatus(message) { statusEl.textContent = message; }
function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"]/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;" }[char]));
}
function authHeaders() {
  return { "Authorization": `Bearer ${state.token}`, "Content-Type": "application/json" };
}
async function api(path, options = {}) {
  if (!state.token) throw new Error("Staff token is required");
  const response = await fetch(path, { ...options, headers: { ...authHeaders(), ...(options.headers || {}) } });
  if (!response.ok) {
    let message = `HTTP ${response.status}`;
    try { message = (await response.json()).message || message; } catch (_) {}
    throw new Error(message);
  }
  return response.json();
}
async function loadReports() {
  try {
    setStatus("Loading reports...");
    const status = statusFilter.value;
    const query = status ? `?status=${encodeURIComponent(status)}` : "";
    const data = await api(`/staff/reports${query}`);
    state.reports = data.reports;
    renderReports();
    setStatus(`${data.reports.length} report(s) loaded.`);
  } catch (error) {
    setStatus(`Could not load reports: ${error.message}`);
  }
}
function renderReports() {
  reportsEl.innerHTML = state.reports.map(report => `
    <button class="report-card ${state.selected === report.id ? "active" : ""}" data-id="${report.id}">
      <strong>${escapeHtml(report.reason)}</strong>
      <span>${escapeHtml(report.status)} - ${escapeHtml(report.created_at)}</span>
      <span>${escapeHtml(report.reported_user.display_name)} reported by ${escapeHtml(report.reporter.display_name)}</span>
      <code>${report.id}</code>
    </button>
  `).join("") || `<p class="muted">No reports match this filter.</p>`;
  reportsEl.querySelectorAll("button[data-id]").forEach(button => {
    button.addEventListener("click", () => loadReportDetail(button.dataset.id));
  });
}
async function loadReportDetail(reportId) {
  try {
    state.selected = reportId;
    renderReports();
    setStatus("Loading report detail...");
    const data = await api(`/staff/reports/${reportId}`);
    renderDetail(data.report);
    setStatus("Report detail loaded.");
  } catch (error) {
    setStatus(`Could not load report detail: ${error.message}`);
  }
}
function renderDetail(report) {
  const summary = report.summary;
  const message = report.message;
  const isOpen = summary.status === "open";
  const globalMessageId = message && message.kind === "global" ? message.id : "";
  detailEl.innerHTML = `
    <div>
      <h2>${escapeHtml(summary.reason)}</h2>
      <p class="muted">${escapeHtml(summary.status)} - ${escapeHtml(summary.created_at)}</p>
    </div>
    <div class="detail-grid">
      <div class="box"><span class="muted">Reporter</span><strong>${escapeHtml(summary.reporter.display_name)}</strong><code>${summary.reporter.id}</code></div>
      <div class="box"><span class="muted">Reported User</span><strong>${escapeHtml(summary.reported_user.display_name)}</strong><code>${summary.reported_user.id}</code></div>
    </div>
    <div class="box"><span class="muted">Details</span><p>${escapeHtml(report.details || "No additional details.")}</p></div>
    <div class="box">
      <span class="muted">Message</span>
      ${message ? `<code>${message.kind} ${message.id}</code><p>${escapeHtml(message.body || "Body unavailable by policy.")}</p><span class="muted">Created ${escapeHtml(message.created_at)}${message.deleted_at ? `, deleted ${escapeHtml(message.deleted_at)}` : ""}</span>` : `<p>No message attached.</p>`}
    </div>
    <div class="actions">
      <button data-action="resolve" ${isOpen ? "" : "disabled"}>Resolve</button>
      <button data-action="dismiss" ${isOpen ? "" : "disabled"}>Dismiss</button>
      <button data-action="deleteMessage" class="danger" ${globalMessageId ? "" : "disabled"}>Delete Global Message</button>
      <button data-action="suspend" class="danger">Suspend User</button>
      <button data-action="ban" class="danger">Ban User</button>
      <button data-action="unban" class="secondary">Unban User</button>
    </div>
    <div class="audit">
      <h3>Audit</h3>
      ${report.actions.map(action => `<div class="audit-row"><strong>${escapeHtml(action.action)}</strong><p>${escapeHtml(action.reason)}</p><span class="muted">${escapeHtml(action.created_at)} by ${escapeHtml(action.moderator.display_name)}</span></div>`).join("") || `<p class="muted">No audit actions for this report.</p>`}
    </div>
  `;
  detailEl.querySelector("[data-action=resolve]").addEventListener("click", () => closeReport(summary.id, "resolve"));
  detailEl.querySelector("[data-action=dismiss]").addEventListener("click", () => closeReport(summary.id, "dismiss"));
  detailEl.querySelector("[data-action=deleteMessage]").addEventListener("click", () => deleteMessage(globalMessageId, summary.id));
  detailEl.querySelector("[data-action=suspend]").addEventListener("click", () => suspendUser(summary.reported_user.id, summary.id));
  detailEl.querySelector("[data-action=ban]").addEventListener("click", () => banUser(summary.reported_user.id, summary.id));
  detailEl.querySelector("[data-action=unban]").addEventListener("click", () => unbanUser(summary.reported_user.id));
}
function requiredReason(label) {
  const reason = prompt(label);
  return reason && reason.trim() ? reason.trim() : null;
}
async function closeReport(reportId, action) {
  const reason = requiredReason(`Reason to ${action} this report`);
  if (!reason) return;
  await postAction(`/staff/reports/${reportId}/${action}`, { reason }, reportId);
}
async function deleteMessage(messageId, reportId) {
  const reason = requiredReason("Reason to delete this global message");
  if (!reason) return;
  await postAction(`/staff/global-messages/${messageId}/delete`, { reason, report_id: reportId }, reportId);
}
async function suspendUser(userId, reportId) {
  const seconds = Number(prompt("Suspension length in seconds", "3600"));
  if (!Number.isFinite(seconds) || seconds <= 0) return;
  const reason = requiredReason("Reason to suspend this user");
  if (!reason) return;
  await postAction(`/staff/users/${userId}/suspend`, { reason, duration_seconds: seconds, report_id: reportId }, reportId);
}
async function banUser(userId, reportId) {
  const reason = requiredReason("Reason to ban this user");
  if (!reason) return;
  await postAction(`/staff/users/${userId}/ban`, { reason, report_id: reportId }, reportId);
}
async function unbanUser(userId) {
  const reason = requiredReason("Reason to unban this user");
  if (!reason) return;
  await postAction(`/staff/users/${userId}/unban`, { reason }, state.selected);
}
async function postAction(path, body, reportId) {
  try {
    setStatus("Submitting action...");
    await api(path, { method: "POST", body: JSON.stringify(body) });
    if (reportId) await loadReportDetail(reportId);
    await loadReports();
    setStatus("Action completed.");
  } catch (error) {
    setStatus(`Action failed: ${error.message}`);
  }
}
document.querySelector("#saveToken").addEventListener("click", () => {
  state.token = tokenInput.value.trim();
  localStorage.setItem("agora_staff_token", state.token);
  loadReports();
});
document.querySelector("#reload").addEventListener("click", loadReports);
statusFilter.addEventListener("change", loadReports);
if (state.token) loadReports();
</script>
</body>
</html>"##;

#[derive(Debug)]
struct ModerationError {
    status: StatusCode,
    message: String,
}

impl ModerationError {
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

impl IntoResponse for ModerationError {
    fn into_response(self) -> Response {
        let status = self.status;
        let message = self.message;
        (status, Json(ApiError { message })).into_response()
    }
}

impl From<sqlx::Error> for ModerationError {
    fn from(error: sqlx::Error) -> Self {
        warn!(%error, "database error in moderation route");
        Self::internal("database operation failed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(role: UserRole) -> auth::Principal {
        auth::Principal {
            session_id: Uuid::nil(),
            user: UserSummary {
                id: Uuid::from_u128(1),
                display_name: "Staff".to_string(),
                avatar_url: None,
            },
            role,
        }
    }

    #[test]
    fn requires_moderator_for_staff_access() {
        assert!(require_moderator(&principal(UserRole::Moderator)).is_ok());
        assert!(require_moderator(&principal(UserRole::Admin)).is_ok());
        assert!(require_moderator(&principal(UserRole::User)).is_err());
    }

    #[test]
    fn reserves_admin_actions_for_admins() {
        assert!(require_admin(&principal(UserRole::Admin)).is_ok());
        assert!(require_admin(&principal(UserRole::Moderator)).is_err());
    }

    #[test]
    fn moderators_cannot_act_on_staff_targets() {
        let moderator = principal(UserRole::Moderator);

        assert!(require_can_moderate_target(&moderator, UserRole::User).is_ok());
        assert!(require_can_moderate_target(&moderator, UserRole::Moderator).is_err());
        assert!(require_can_moderate_target(&moderator, UserRole::Admin).is_err());
    }

    #[test]
    fn normalizes_required_moderation_reason() {
        assert_eq!(normalize_reason(" spam ").unwrap(), "spam");
        assert!(normalize_reason("   ").is_err());
    }

    #[test]
    fn validates_suspend_duration() {
        assert!(validate_suspend_duration(1).is_ok());
        assert!(validate_suspend_duration(0).is_err());
        assert!(validate_suspend_duration(MAX_SUSPEND_SECONDS + 1).is_err());
    }

    #[test]
    fn serializes_new_audit_actions_to_db_values() {
        assert_eq!(
            moderation_action_as_str(ModerationActionKind::DeleteGlobalMessage),
            "delete_global_message"
        );
        assert_eq!(
            moderation_action_as_str(ModerationActionKind::Suspend),
            "suspend"
        );
        assert_eq!(moderation_action_as_str(ModerationActionKind::Ban), "ban");
        assert_eq!(
            moderation_action_as_str(ModerationActionKind::Unban),
            "unban"
        );
    }
}

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use agora_common::{
    ApiError, AuthSession, DevLoginRequest, DevLoginResponse, LogoutRequest, LogoutResponse,
    RefreshRequest, RefreshResponse, SteamLoginPollRequest, SteamLoginPollResponse,
    SteamLoginStartResponse, SteamLoginStatus, UserRole, UserSummary,
};
use axum::{
    extract::{ConnectInfo, FromRequestParts, Query, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use serde::Deserialize;
use sha2::Sha256;
use sqlx::{Postgres, Row, Transaction};
use tracing::warn;
use url::Url;
use uuid::Uuid;

use crate::{chat, normalize_dev_account_id, AppState, DevLoginAccount};

const STEAM_OPENID_ENDPOINT: &str = "https://steamcommunity.com/openid/login";
const STEAM_IDENTIFIER_SELECT: &str = "http://specs.openid.net/auth/2.0/identifier_select";
const LOGIN_CHALLENGE_EXPIRES_SECONDS: u64 = 10 * 60;
const ACCESS_TOKEN_EXPIRES_SECONDS: u64 = 15 * 60;
const REFRESH_TOKEN_EXPIRES_SECONDS: u64 = 30 * 24 * 60 * 60;
const ABSOLUTE_SESSION_EXPIRES_SECONDS: u64 = 90 * 24 * 60 * 60;
const AUTH_RECORD_RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;
const LOGIN_CHALLENGE_RETENTION_SECONDS: u64 = 24 * 60 * 60;
const AUTH_CLEANUP_INTERVAL_SECONDS: u64 = 5 * 60;
const MIN_REFRESH_ROTATION_INTERVAL_SECONDS: u64 = 5 * 60;
const MAX_SESSION_FAMILY_TOKENS: i64 =
    (ABSOLUTE_SESSION_EXPIRES_SECONDS / MIN_REFRESH_ROTATION_INTERVAL_SECONDS) as i64 + 1;
static LAST_AUTH_CLEANUP_UNIX_SECONDS: AtomicU64 = AtomicU64::new(0);

type HmacSha256 = Hmac<Sha256>;
type AuthResult<T> = Result<T, AuthError>;

#[derive(Clone, Copy)]
enum SessionSource {
    Steam,
    LocalTest,
}

impl SessionSource {
    fn as_db(self) -> &'static str {
        match self {
            Self::Steam => "steam",
            Self::LocalTest => "local_test",
        }
    }
}

#[derive(Clone)]
pub(crate) struct AuthenticatedSession {
    pub(crate) session_id: Uuid,
    pub(crate) user: UserSummary,
    pub(crate) role: UserRole,
    pub(crate) access_token_ttl_seconds: u64,
}

#[derive(Clone)]
pub(crate) struct Principal {
    #[allow(dead_code)]
    pub(crate) session_id: Uuid,
    pub(crate) user: UserSummary,
    pub(crate) role: UserRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionActivity {
    Active,
    AccessTokenExpired,
    Revoked,
    UserRestricted,
    Ended,
}

impl Principal {
    pub(crate) fn is_moderator(&self) -> bool {
        matches!(
            self.role,
            UserRole::Moderator | UserRole::Admin | UserRole::Owner
        )
    }

    pub(crate) fn is_admin(&self) -> bool {
        matches!(self.role, UserRole::Admin | UserRole::Owner)
    }
}

impl FromRequestParts<AppState> for Principal {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer_addr = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|value| value.0)
            .ok_or_else(|| AuthError::internal("missing client connection details"))?;
        state
            .rate_limits
            .check_auth(peer_addr, &parts.headers, &state.config.trusted_proxy_cidrs)
            .await
            .map_err(|error| AuthError::too_many_requests(error.message()))?;
        principal_for_headers(state, &parts.headers).await
    }
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/steam/device/start", post(steam_login_start))
        .route("/auth/steam/callback", get(steam_login_callback))
        .route("/auth/steam/device/poll", post(steam_login_poll))
        .route("/auth/refresh", post(refresh))
        .route("/auth/logout", post(logout))
        .route("/auth/dev/login", post(dev_login))
}

pub(crate) async fn revoke_unconfigured_local_test_sessions(
    db: &sqlx::PgPool,
    config: &crate::Config,
) -> Result<(), sqlx::Error> {
    let provider_user_ids = configured_local_test_provider_user_ids(config);
    sqlx::query(
        "update sessions s
         set revoked_at = now()
         where s.auth_source = 'local_test'
           and s.revoked_at is null
           and not exists (
               select 1
               from identities i
               where i.user_id = s.user_id
                 and i.provider = 'dev'
                 and i.provider_user_id = any($1)
           )",
    )
    .bind(provider_user_ids)
    .execute(db)
    .await?;
    Ok(())
}

pub(crate) async fn reconcile_local_test_accounts(
    db: &sqlx::PgPool,
    config: &crate::Config,
) -> Result<(), sqlx::Error> {
    if !config.enable_dev_login {
        return Ok(());
    }

    let provider_user_ids = configured_local_test_provider_user_ids(config);
    let mut tx = db.begin().await?;
    for account in &config.dev_login_accounts {
        find_or_create_dev_user(&mut tx, account).await?;
    }
    sqlx::query(
        "update users u
          set role = 'user'
          where exists (
                select 1
                from identities dev_identity
                where dev_identity.user_id = u.id
                  and dev_identity.provider = 'dev'
            )
            and not exists (
                select 1
                from identities configured_dev_identity
                where configured_dev_identity.user_id = u.id
                  and configured_dev_identity.provider = 'dev'
                  and configured_dev_identity.provider_user_id = any($1)
            )
            and not exists (
                select 1
                from identities steam_identity
                where steam_identity.user_id = u.id
                  and steam_identity.provider = 'steam'
            )",
    )
    .bind(provider_user_ids)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

fn configured_local_test_provider_user_ids(config: &crate::Config) -> Vec<String> {
    if !config.enable_dev_login {
        return Vec::new();
    }
    config
        .dev_login_accounts
        .iter()
        .map(|account| dev_provider_user_id(&account.account_id))
        .collect()
}

pub(crate) async fn user_for_access_token(
    state: &AppState,
    access_token: &str,
) -> Result<Option<AuthenticatedSession>, sqlx::Error> {
    let access_token = access_token.trim();
    if access_token.is_empty() {
        return Ok(None);
    }
    let Ok(access_token_hash) = token_hash(access_token, &state.config.session_secret) else {
        return Ok(None);
    };

    let Some(row) = sqlx::query(
        "update sessions s
         set last_used_at = now()
         from users u
         where u.id = s.user_id
           and s.access_token_hash = $1
            and s.access_token_expires_at > now()
            and s.revoked_at is null
            and s.expires_at > now()
            and s.absolute_expires_at > now()
            and u.banned_at is null
           and (u.suspended_until is null or u.suspended_until <= now())
         returning
            s.id as session_id,
            u.id as user_id,
             u.display_name,
             u.avatar_url,
             u.role,
             s.auth_source,
             (
                 select i.provider_user_id
                 from identities i
                 where i.user_id = u.id and i.provider = 'dev'
                 limit 1
             ) as dev_provider_user_id,
             greatest(1, extract(epoch from (s.access_token_expires_at - now()))::bigint) as access_token_ttl_seconds",
    )
    .bind(access_token_hash)
    .fetch_optional(&state.db)
    .await?
    else {
        return Ok(None);
    };

    let stored_role = user_role_from_db(row.try_get::<String, _>("role")?.as_str());
    let source: String = row.try_get("auth_source")?;
    let role = match source.as_str() {
        "steam" => stored_role,
        "local_test" => {
            let provider_user_id: Option<String> = row.try_get("dev_provider_user_id")?;
            let Some(role) = local_test_role(
                state.config.enable_dev_login,
                &state.config.dev_login_accounts,
                provider_user_id.as_deref(),
            ) else {
                return Ok(None);
            };
            role
        }
        _ => return Ok(None),
    };
    let ttl: i64 = row.try_get("access_token_ttl_seconds")?;
    Ok(Some(AuthenticatedSession {
        session_id: row.try_get("session_id")?,
        user: UserSummary {
            id: row.try_get("user_id")?,
            display_name: row.try_get("display_name")?,
            avatar_url: row.try_get("avatar_url")?,
        },
        role,
        access_token_ttl_seconds: ttl.max(1) as u64,
    }))
}

pub(crate) async fn principal_for_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> AuthResult<Principal> {
    let access_token =
        bearer_token(headers).ok_or_else(|| AuthError::unauthorized("missing bearer token"))?;
    user_for_access_token(state, access_token)
        .await?
        .map(|session| Principal {
            session_id: session.session_id,
            user: session.user,
            role: session.role,
        })
        .ok_or_else(|| AuthError::unauthorized("session is invalid or expired"))
}

fn user_role_from_db(value: &str) -> UserRole {
    match value {
        "moderator" => UserRole::Moderator,
        "admin" => UserRole::Admin,
        "owner" => UserRole::Owner,
        _ => UserRole::User,
    }
}

fn user_role_as_db(role: UserRole) -> &'static str {
    match role {
        UserRole::User => "user",
        UserRole::Moderator => "moderator",
        UserRole::Admin => "admin",
        UserRole::Owner => "owner",
    }
}

fn local_test_role(
    dev_login_enabled: bool,
    accounts: &[DevLoginAccount],
    provider_user_id: Option<&str>,
) -> Option<UserRole> {
    if !dev_login_enabled {
        return None;
    }
    let account_id = provider_user_id?.strip_prefix("local:")?;
    accounts
        .iter()
        .find(|account| account.account_id == account_id)
        .map(|account| account.role)
}

pub(crate) async fn session_activity(
    state: &AppState,
    session_id: Uuid,
) -> Result<SessionActivity, sqlx::Error> {
    let Some((
        access_token_active,
        session_revoked,
        session_lifetime_active,
        user_restricted,
        source,
        provider_user_id,
    )) = sqlx::query_as::<_, (bool, bool, bool, bool, String, Option<String>)>(
        "select
             s.access_token_expires_at > now() as access_token_active,
             s.revoked_at is not null as session_revoked,
             (s.expires_at > now()
               and s.absolute_expires_at > now()) as session_lifetime_active,
             (u.banned_at is not null
               or (u.suspended_until is not null and u.suspended_until > now())) as user_restricted,
             s.auth_source,
            (
                select i.provider_user_id
                from identities i
                where i.user_id = u.id and i.provider = 'dev'
                limit 1
            ) as dev_provider_user_id
         from sessions s
         join users u on u.id = s.user_id
         where s.id = $1",
    )
    .bind(session_id)
    .fetch_optional(&state.db)
    .await?
    else {
        return Ok(SessionActivity::Ended);
    };

    let source_is_allowed = match source.as_str() {
        "steam" => true,
        "local_test" => local_test_role(
            state.config.enable_dev_login,
            &state.config.dev_login_accounts,
            provider_user_id.as_deref(),
        )
        .is_some(),
        _ => false,
    };
    Ok(if user_restricted {
        SessionActivity::UserRestricted
    } else if session_revoked {
        SessionActivity::Revoked
    } else if !session_lifetime_active || !source_is_allowed {
        SessionActivity::Ended
    } else if !access_token_active {
        SessionActivity::AccessTokenExpired
    } else {
        SessionActivity::Active
    })
}

pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let mut parts = value.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if parts.next().is_some() || !scheme.eq_ignore_ascii_case("Bearer") || token.is_empty() {
        return None;
    }
    Some(token)
}

async fn dev_login(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    request: Option<Json<DevLoginRequest>>,
) -> AuthResult<Json<DevLoginResponse>> {
    if !state.config.enable_dev_login
        || !is_local_dev_login_request(
            peer_addr,
            &headers,
            state.config.dev_login_proxy_token.as_deref(),
        )
    {
        return Err(AuthError::not_found("dev login is not enabled"));
    }
    state
        .rate_limits
        .check_auth(peer_addr, &headers, &state.config.trusted_proxy_cidrs)
        .await
        .map_err(|error| AuthError::too_many_requests(error.message()))?;
    opportunistic_auth_cleanup(&state).await;

    let account_id = request
        .map(|Json(request)| request.account_id)
        .ok_or_else(|| AuthError::bad_request("local test account ID is required"))?;
    let account_id = normalize_dev_account_id(&account_id)
        .map_err(|error| AuthError::bad_request(error.to_string()))?;
    let account = state
        .config
        .dev_login_accounts
        .iter()
        .find(|account| account.account_id == account_id)
        .ok_or_else(|| AuthError::bad_request("unknown local test account"))?;

    let mut tx = state.db.begin().await?;
    let user = find_or_create_dev_user(&mut tx, account).await?;
    let session = create_session(&state, &mut tx, user, SessionSource::LocalTest).await?;
    tx.commit().await?;

    Ok(Json(DevLoginResponse { session }))
}

async fn steam_login_start(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> AuthResult<Json<SteamLoginStartResponse>> {
    state
        .rate_limits
        .check_auth(peer_addr, &headers, &state.config.trusted_proxy_cidrs)
        .await
        .map_err(|error| AuthError::too_many_requests(error.message()))?;
    expire_old_login_challenges(&state).await?;

    let challenge_id = Uuid::new_v4();
    let poll_token = random_token();
    let poll_token_hash = token_hash(&poll_token, &state.config.session_secret)?;
    let browser_url = steam_login_url(&state.config.public_url, challenge_id)?;

    sqlx::query(
        "insert into steam_login_challenges (id, poll_token_hash, expires_at)
         values ($1, $2, now() + make_interval(secs => $3))",
    )
    .bind(challenge_id)
    .bind(poll_token_hash)
    .bind(LOGIN_CHALLENGE_EXPIRES_SECONDS as i32)
    .execute(&state.db)
    .await?;

    Ok(Json(SteamLoginStartResponse {
        browser_url,
        poll_token,
        expires_in_seconds: LOGIN_CHALLENGE_EXPIRES_SECONDS,
    }))
}

async fn steam_login_callback(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    match complete_steam_login(&state, &query).await {
        Ok(user) => Html(steam_login_success_page(&user.display_name)).into_response(),
        Err(error) => (error.status, Html(steam_login_error_page(&error.message))).into_response(),
    }
}

async fn steam_login_poll(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<SteamLoginPollRequest>,
) -> AuthResult<Json<SteamLoginPollResponse>> {
    state
        .rate_limits
        .check_auth(peer_addr, &headers, &state.config.trusted_proxy_cidrs)
        .await
        .map_err(|error| AuthError::too_many_requests(error.message()))?;
    expire_old_login_challenges(&state).await?;
    let poll_token = request.poll_token.trim();
    if poll_token.is_empty() {
        return Err(AuthError::bad_request("poll token is required"));
    }

    let poll_token_hash = token_hash(poll_token, &state.config.session_secret)?;
    let Some(row) = sqlx::query(
        "select
            c.id,
            c.status,
             c.user_id,
             c.error,
             c.expires_at <= now() as expired,
             c.consumed_at is not null as consumed
          from steam_login_challenges c
          where c.poll_token_hash = $1",
    )
    .bind(poll_token_hash)
    .fetch_optional(&state.db)
    .await?
    else {
        return Ok(Json(SteamLoginPollResponse {
            status: SteamLoginStatus::Denied {
                message: "login challenge was not found".to_string(),
            },
        }));
    };

    let challenge_id: Uuid = row.try_get("id")?;
    let status: String = row.try_get("status")?;
    let expired: bool = row.try_get("expired")?;
    let consumed: bool = row.try_get("consumed")?;

    if expired && status == "pending" {
        sqlx::query("update steam_login_challenges set status = 'expired' where id = $1")
            .bind(challenge_id)
            .execute(&state.db)
            .await?;
        return Ok(Json(SteamLoginPollResponse {
            status: SteamLoginStatus::Expired,
        }));
    }

    match status.as_str() {
        "pending" => Ok(Json(SteamLoginPollResponse {
            status: SteamLoginStatus::Pending,
        })),
        "expired" => Ok(Json(SteamLoginPollResponse {
            status: SteamLoginStatus::Expired,
        })),
        "denied" => Ok(Json(SteamLoginPollResponse {
            status: SteamLoginStatus::Denied {
                message: row
                    .try_get::<Option<String>, _>("error")?
                    .unwrap_or_else(|| "Steam login was denied".to_string()),
            },
        })),
        "complete" => {
            if consumed {
                return Ok(Json(SteamLoginPollResponse {
                    status: SteamLoginStatus::Denied {
                        message: "login challenge was already used".to_string(),
                    },
                }));
            }

            let user_id = row
                .try_get::<Option<Uuid>, _>("user_id")?
                .ok_or_else(|| AuthError::internal("completed login is missing a user"))?;
            let mut tx = state.db.begin().await?;
            let Some(user) = active_user_for_session_tx(&mut tx, user_id).await? else {
                let denied = sqlx::query(
                    "update steam_login_challenges
                     set status = 'denied', error = 'account is unavailable'
                     where id = $1 and status = 'complete' and consumed_at is null",
                )
                .bind(challenge_id)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                let message = if denied.rows_affected() == 1 {
                    "account is unavailable"
                } else {
                    "login challenge was already used"
                };
                return Ok(Json(SteamLoginPollResponse {
                    status: SteamLoginStatus::Denied {
                        message: message.to_string(),
                    },
                }));
            };
            let claimed = sqlx::query(
                "update steam_login_challenges
                 set consumed_at = now()
                 where id = $1 and status = 'complete' and consumed_at is null
                 returning id",
            )
            .bind(challenge_id)
            .fetch_optional(&mut *tx)
            .await?;
            if claimed.is_none() {
                tx.rollback().await?;
                return Ok(Json(SteamLoginPollResponse {
                    status: SteamLoginStatus::Denied {
                        message: "login challenge was already used".to_string(),
                    },
                }));
            }

            let session = create_session(&state, &mut tx, user, SessionSource::Steam).await?;
            tx.commit().await?;

            Ok(Json(SteamLoginPollResponse {
                status: SteamLoginStatus::Complete { session },
            }))
        }
        _ => Err(AuthError::internal("unknown login challenge status")),
    }
}

async fn refresh(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<RefreshRequest>,
) -> AuthResult<Json<RefreshResponse>> {
    state
        .rate_limits
        .check_auth(peer_addr, &headers, &state.config.trusted_proxy_cidrs)
        .await
        .map_err(|error| AuthError::too_many_requests(error.message()))?;
    opportunistic_auth_cleanup(&state).await;
    let refresh_token = request.refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(AuthError::bad_request("refresh token is required"));
    }

    let refresh_token_hash = token_hash(refresh_token, &state.config.session_secret)?;
    let mut tx = state.db.begin().await?;
    let Some(token_user_id) =
        sqlx::query_scalar::<_, Uuid>("select user_id from sessions where refresh_token_hash = $1")
            .bind(&refresh_token_hash)
            .fetch_optional(&mut *tx)
            .await?
    else {
        return Err(AuthError::unauthorized("session is invalid or expired"));
    };
    let Some((user, user_active)) = user_for_session_update_tx(&mut tx, token_user_id).await?
    else {
        return Err(AuthError::unauthorized("session is invalid or expired"));
    };
    let Some(row) = sqlx::query(
        "select
            s.id as session_id,
            s.session_family_id,
             s.refresh_token_used_at is not null as refresh_token_used,
             s.revoked_at is not null as session_revoked,
             s.expires_at > now() as refresh_token_active,
             s.absolute_expires_at > now() as absolute_session_active,
             s.created_at
                 <= now() - make_interval(secs => $2) as refresh_rotation_due,
             s.auth_source,
            (
                select i.provider_user_id
                from identities i
                where i.user_id = s.user_id and i.provider = 'dev'
                limit 1
            ) as dev_provider_user_id
          from sessions s
          where s.refresh_token_hash = $1 and s.user_id = $3
          for update of s",
    )
    .bind(&refresh_token_hash)
    .bind(MIN_REFRESH_ROTATION_INTERVAL_SECONDS as i32)
    .bind(token_user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Err(AuthError::unauthorized("session is invalid or expired"));
    };

    let session_id: Uuid = row.try_get("session_id")?;
    let session_family_id: Uuid = row.try_get("session_family_id")?;
    let refresh_token_used: bool = row.try_get("refresh_token_used")?;
    let session_revoked: bool = row.try_get("session_revoked")?;
    let refresh_token_active: bool = row.try_get("refresh_token_active")?;
    let absolute_session_active: bool = row.try_get("absolute_session_active")?;
    let refresh_rotation_due: bool = row.try_get("refresh_rotation_due")?;
    if refresh_token_used {
        let revoked_session_ids = if session_revoked {
            Vec::new()
        } else {
            revoke_session_family_tx(&mut tx, session_family_id).await?
        };
        tx.commit().await?;
        if !revoked_session_ids.is_empty() {
            warn!(%session_family_id, "detected refresh token reuse; revoked session family");
            for revoked_session_id in revoked_session_ids {
                chat::send_session_revoked(&state, revoked_session_id);
            }
        }
        return Err(AuthError::unauthorized("session is invalid or expired"));
    }

    let source: String = row.try_get("auth_source")?;
    let allowed = match source.as_str() {
        "steam" => true,
        "local_test" => {
            let provider_user_id: Option<String> = row.try_get("dev_provider_user_id")?;
            local_test_role(
                state.config.enable_dev_login,
                &state.config.dev_login_accounts,
                provider_user_id.as_deref(),
            )
            .is_some()
        }
        _ => false,
    };
    if session_revoked
        || !refresh_token_active
        || !absolute_session_active
        || !user_active
        || !allowed
    {
        tx.rollback().await?;
        return Err(AuthError::unauthorized("session is invalid or expired"));
    }
    if !refresh_rotation_due {
        tx.rollback().await?;
        return Err(AuthError::too_many_requests(format!(
            "session refresh can rotate once every {} seconds",
            MIN_REFRESH_ROTATION_INTERVAL_SECONDS
        )));
    }
    let family_token_count = session_family_token_count_tx(&mut tx, session_family_id).await?;
    if session_family_at_capacity(family_token_count) {
        let revoked_session_ids = revoke_session_family_tx(&mut tx, session_family_id).await?;
        tx.commit().await?;
        warn!(%session_family_id, "session refresh family reached its token limit; revoked family");
        for revoked_session_id in revoked_session_ids {
            chat::send_session_revoked(&state, revoked_session_id);
        }
        return Err(AuthError::unauthorized(
            "session refresh limit reached; sign in again",
        ));
    }
    let session = rotate_session(&state, &mut tx, session_id, user).await?;
    tx.commit().await?;

    Ok(Json(RefreshResponse { session }))
}

async fn logout(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<LogoutRequest>,
) -> AuthResult<Json<LogoutResponse>> {
    state
        .rate_limits
        .check_auth(peer_addr, &headers, &state.config.trusted_proxy_cidrs)
        .await
        .map_err(|error| AuthError::too_many_requests(error.message()))?;
    opportunistic_auth_cleanup(&state).await;
    let refresh_token = request.refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(AuthError::bad_request("refresh token is required"));
    }

    let refresh_token_hash = token_hash(refresh_token, &state.config.session_secret)?;
    let mut tx = state.db.begin().await?;
    let Some(user_id) =
        sqlx::query_scalar::<_, Uuid>("select user_id from sessions where refresh_token_hash = $1")
            .bind(&refresh_token_hash)
            .fetch_optional(&mut *tx)
            .await?
    else {
        return Ok(Json(LogoutResponse { revoked: false }));
    };
    if user_for_session_update_tx(&mut tx, user_id)
        .await?
        .is_none()
    {
        return Ok(Json(LogoutResponse { revoked: false }));
    }
    let rows = sqlx::query(
        "with target_family as (
             select session_family_id
             from sessions
             where refresh_token_hash = $1
         )
         update sessions s
         set revoked_at = now()
         from target_family f
         where s.session_family_id = f.session_family_id
           and s.revoked_at is null
         returning s.id",
    )
    .bind(&refresh_token_hash)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    for row in &rows {
        chat::send_session_revoked(&state, row.try_get("id")?);
    }

    Ok(Json(LogoutResponse {
        revoked: !rows.is_empty(),
    }))
}

async fn complete_steam_login(
    state: &AppState,
    query: &HashMap<String, String>,
) -> AuthResult<UserSummary> {
    let challenge_id = parse_challenge_id(query)?;
    validate_openid_return_to(&state.config.public_url, challenge_id, query)?;
    ensure_challenge_pending(state, challenge_id).await?;

    let result = async {
        validate_steam_openid(state, query).await?;
        let steam_id = steam_id_from_openid_claim(query)?;
        let profile = steam_profile(state, &steam_id).await;
        let mut tx = state.db.begin().await?;
        let user = find_or_create_steam_user(&mut tx, &steam_id, profile.as_ref()).await?;
        let result = sqlx::query(
            "update steam_login_challenges
             set status = 'complete', user_id = $2, completed_at = now(), error = null
             where id = $1 and status = 'pending' and expires_at > now()",
        )
        .bind(challenge_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(AuthError::bad_request(
                "login challenge expired or is no longer pending",
            ));
        }
        tx.commit().await?;
        Ok(user)
    }
    .await;

    match result {
        Ok(user) => Ok(user),
        Err(error) => {
            let _ = sqlx::query(
                "update steam_login_challenges
                 set status = 'denied', error = $2
                 where id = $1 and status = 'pending'",
            )
            .bind(challenge_id)
            .bind(&error.message)
            .execute(&state.db)
            .await;
            Err(error)
        }
    }
}

fn parse_challenge_id(query: &HashMap<String, String>) -> AuthResult<Uuid> {
    let challenge_id = query
        .get("challenge_id")
        .ok_or_else(|| AuthError::bad_request("missing login challenge"))?;
    Uuid::parse_str(challenge_id).map_err(|_| AuthError::bad_request("invalid login challenge"))
}

async fn ensure_challenge_pending(state: &AppState, challenge_id: Uuid) -> AuthResult<()> {
    expire_old_login_challenges(state).await?;
    let active = sqlx::query_scalar::<_, bool>(
        "select exists(
            select 1
            from steam_login_challenges
            where id = $1 and status = 'pending' and expires_at > now()
        )",
    )
    .bind(challenge_id)
    .fetch_one(&state.db)
    .await?;

    if active {
        Ok(())
    } else {
        Err(AuthError::bad_request(
            "login challenge expired or is no longer pending",
        ))
    }
}

async fn validate_steam_openid(
    state: &AppState,
    query: &HashMap<String, String>,
) -> AuthResult<()> {
    if query.get("openid.mode").map(String::as_str) != Some("id_res") {
        return Err(AuthError::bad_request(
            "Steam did not return a login assertion",
        ));
    }

    let mut form = query
        .iter()
        .filter(|(key, _)| key.starts_with("openid."))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Vec<_>>();
    for (key, value) in &mut form {
        if key == "openid.mode" {
            *value = "check_authentication".to_string();
        }
    }

    let response = state
        .http
        .post(STEAM_OPENID_ENDPOINT)
        .form(&form)
        .send()
        .await
        .map_err(|error| AuthError::bad_gateway(format!("Steam validation failed: {error}")))?;
    if !response.status().is_success() {
        return Err(AuthError::bad_gateway(format!(
            "Steam validation returned HTTP {}",
            response.status()
        )));
    }

    let body = response
        .text()
        .await
        .map_err(|error| AuthError::bad_gateway(format!("Steam validation failed: {error}")))?;
    if body.lines().any(|line| line.trim() == "is_valid:true") {
        Ok(())
    } else {
        Err(AuthError::unauthorized("Steam login assertion was invalid"))
    }
}

fn steam_id_from_openid_claim(query: &HashMap<String, String>) -> AuthResult<String> {
    let claim = query
        .get("openid.claimed_id")
        .or_else(|| query.get("openid.identity"))
        .ok_or_else(|| AuthError::bad_request("missing Steam identity claim"))?;
    let steam_id = claim
        .strip_prefix("https://steamcommunity.com/openid/id/")
        .or_else(|| claim.strip_prefix("http://steamcommunity.com/openid/id/"))
        .filter(|value| value.len() >= 16 && value.chars().all(|c| c.is_ascii_digit()))
        .ok_or_else(|| AuthError::bad_request("invalid Steam identity claim"))?;
    Ok(steam_id.to_string())
}

async fn find_or_create_steam_user(
    tx: &mut Transaction<'_, Postgres>,
    steam_id: &str,
    profile: Option<&SteamProfile>,
) -> AuthResult<UserSummary> {
    if let Some((user, active)) = steam_identity_user_for_update(tx, steam_id).await? {
        if !active {
            return Err(AuthError::unauthorized("account is unavailable"));
        }
        return refresh_steam_user(tx, user, steam_id, profile).await;
    }

    let new_profile = profile
        .cloned()
        .unwrap_or_else(|| fallback_steam_profile(steam_id));
    let row = sqlx::query(
        "insert into users (display_name, avatar_url)
         values ($1, $2)
         returning id, display_name, avatar_url",
    )
    .bind(&new_profile.display_name)
    .bind(&new_profile.avatar_url)
    .fetch_one(&mut **tx)
    .await?;
    let user = UserSummary {
        id: row.try_get("id")?,
        display_name: row.try_get("display_name")?,
        avatar_url: row.try_get("avatar_url")?,
    };

    let inserted_identity = sqlx::query(
        "insert into identities (
            user_id,
            provider,
            provider_user_id,
            provider_display_name,
            provider_avatar_url
         ) values ($1, 'steam', $2, $3, $4)
         on conflict (provider, provider_user_id) do nothing
         returning user_id",
    )
    .bind(user.id)
    .bind(steam_id)
    .bind(&new_profile.display_name)
    .bind(&new_profile.avatar_url)
    .fetch_optional(&mut **tx)
    .await?;
    if inserted_identity.is_some() {
        return Ok(user);
    }

    // A concurrent callback created the identity first. Remove the unreferenced contender.
    sqlx::query("delete from users where id = $1")
        .bind(user.id)
        .execute(&mut **tx)
        .await?;
    let Some((user, active)) = steam_identity_user_for_update(tx, steam_id).await? else {
        return Err(AuthError::internal("Steam identity conflict was not found"));
    };
    if !active {
        return Err(AuthError::unauthorized("account is unavailable"));
    }
    refresh_steam_user(tx, user, steam_id, profile).await
}

async fn steam_identity_user_for_update(
    tx: &mut Transaction<'_, Postgres>,
    steam_id: &str,
) -> Result<Option<(UserSummary, bool)>, sqlx::Error> {
    let Some(row) = sqlx::query(
        "select
            u.id,
            u.display_name,
            u.avatar_url,
            u.banned_at is null
              and (u.suspended_until is null or u.suspended_until <= now()) as active
         from identities i
         join users u on u.id = i.user_id
         where i.provider = 'steam' and i.provider_user_id = $1
         for update of i, u",
    )
    .bind(steam_id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };
    Ok(Some((
        UserSummary {
            id: row.try_get("id")?,
            display_name: row.try_get("display_name")?,
            avatar_url: row.try_get("avatar_url")?,
        },
        row.try_get("active")?,
    )))
}

async fn refresh_steam_user(
    tx: &mut Transaction<'_, Postgres>,
    user: UserSummary,
    steam_id: &str,
    profile: Option<&SteamProfile>,
) -> AuthResult<UserSummary> {
    let Some(profile) = profile else {
        sqlx::query("update users set last_seen_at = now() where id = $1")
            .bind(user.id)
            .execute(&mut **tx)
            .await?;
        return Ok(user);
    };
    update_steam_profile(tx, user.id, steam_id, profile).await
}

async fn find_or_create_dev_user(
    tx: &mut Transaction<'_, Postgres>,
    account: &DevLoginAccount,
) -> Result<UserSummary, sqlx::Error> {
    let provider_user_id = dev_provider_user_id(&account.account_id);
    if let Some(row) = sqlx::query(
        "select
             u.id,
             u.display_name,
             u.avatar_url,
             exists (
                 select 1
                 from identities steam_identity
                 where steam_identity.user_id = u.id
                   and steam_identity.provider = 'steam'
             ) as has_steam_identity
         from identities i
          join users u on u.id = i.user_id
         where i.provider = 'dev' and i.provider_user_id = $1",
    )
    .bind(&provider_user_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        let id = row.try_get("id")?;
        if row.try_get("has_steam_identity")? {
            return Ok(UserSummary {
                id,
                display_name: row.try_get("display_name")?,
                avatar_url: row.try_get("avatar_url")?,
            });
        }
        sqlx::query(
            "update users
              set display_name = $2, role = $3, last_seen_at = now()
              where id = $1",
        )
        .bind(id)
        .bind(&account.display_name)
        .bind(user_role_as_db(account.role))
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "update identities
             set provider_display_name = $2, updated_at = now()
             where provider = 'dev' and provider_user_id = $1",
        )
        .bind(&provider_user_id)
        .bind(&account.display_name)
        .execute(&mut **tx)
        .await?;

        return Ok(UserSummary {
            id,
            display_name: account.display_name.clone(),
            avatar_url: row.try_get("avatar_url")?,
        });
    }

    let row = sqlx::query(
        "insert into users (display_name, role)
         values ($1, $2)
         returning id, display_name, avatar_url",
    )
    .bind(&account.display_name)
    .bind(user_role_as_db(account.role))
    .fetch_one(&mut **tx)
    .await?;
    let user = UserSummary {
        id: row.try_get("id")?,
        display_name: row.try_get("display_name")?,
        avatar_url: row.try_get("avatar_url")?,
    };

    sqlx::query(
        "insert into identities (user_id, provider, provider_user_id, provider_display_name)
         values ($1, 'dev', $2, $3)",
    )
    .bind(user.id)
    .bind(&provider_user_id)
    .bind(&account.display_name)
    .execute(&mut **tx)
    .await?;

    Ok(user)
}

async fn update_steam_profile(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    steam_id: &str,
    profile: &SteamProfile,
) -> AuthResult<UserSummary> {
    sqlx::query(
        "update identities
         set provider_display_name = $3,
             provider_avatar_url = coalesce($4, provider_avatar_url),
             updated_at = now()
         where provider = 'steam' and provider_user_id = $2 and user_id = $1",
    )
    .bind(user_id)
    .bind(steam_id)
    .bind(&profile.display_name)
    .bind(&profile.avatar_url)
    .execute(&mut **tx)
    .await?;

    let row = sqlx::query(
        "update users
         set display_name = $2,
             avatar_url = coalesce($3, avatar_url),
             last_seen_at = now()
         where id = $1
         returning id, display_name, avatar_url",
    )
    .bind(user_id)
    .bind(&profile.display_name)
    .bind(&profile.avatar_url)
    .fetch_one(&mut **tx)
    .await?;

    Ok(UserSummary {
        id: row.try_get("id")?,
        display_name: row.try_get("display_name")?,
        avatar_url: row.try_get("avatar_url")?,
    })
}

async fn steam_profile(state: &AppState, steam_id: &str) -> Option<SteamProfile> {
    if let Some(api_key) = state.config.steam_web_api_key.as_ref() {
        if let Some(profile) = steam_web_api_profile(state, steam_id, api_key).await {
            return Some(profile);
        }
    }

    steam_community_profile(state, steam_id).await
}

fn fallback_steam_profile(steam_id: &str) -> SteamProfile {
    SteamProfile {
        display_name: format!("Steam {steam_id}"),
        avatar_url: None,
    }
}

async fn steam_web_api_profile(
    state: &AppState,
    steam_id: &str,
    api_key: &str,
) -> Option<SteamProfile> {
    let response = state
        .http
        .get("https://api.steampowered.com/ISteamUser/GetPlayerSummaries/v0002/")
        .query(&[("key", api_key), ("steamids", steam_id)])
        .send()
        .await;

    match response {
        Ok(response) => match response.json::<SteamPlayerSummariesResponse>().await {
            Ok(summary) => summary
                .response
                .players
                .into_iter()
                .next()
                .and_then(|player| {
                    steam_profile_from_parts(
                        player.personaname,
                        player.avatarfull.or(player.avatarmedium).or(player.avatar),
                    )
                }),
            Err(error) => {
                warn!(%error, "failed to parse Steam profile response");
                None
            }
        },
        Err(error) => {
            warn!(%error, "failed to fetch Steam profile");
            None
        }
    }
}

async fn steam_community_profile(state: &AppState, steam_id: &str) -> Option<SteamProfile> {
    let response = state
        .http
        .get(format!("https://steamcommunity.com/profiles/{steam_id}"))
        .query(&[("xml", "1")])
        .send()
        .await;

    match response {
        Ok(response) if response.status().is_success() => match response.text().await {
            Ok(body) => parse_steam_community_profile(&body),
            Err(error) => {
                warn!(%error, "failed to read Steam community profile response");
                None
            }
        },
        Ok(response) => {
            warn!(status = %response.status(), "Steam community profile returned non-success status");
            None
        }
        Err(error) => {
            warn!(%error, "failed to fetch Steam community profile");
            None
        }
    }
}

fn steam_profile_from_parts(
    display_name: Option<String>,
    avatar_url: Option<String>,
) -> Option<SteamProfile> {
    let display_name = display_name?.trim().to_string();
    (!display_name.is_empty()).then_some(SteamProfile {
        display_name,
        avatar_url,
    })
}

fn parse_steam_community_profile(xml: &str) -> Option<SteamProfile> {
    steam_profile_from_parts(
        xml_tag_text(xml, "steamID"),
        xml_tag_text(xml, "avatarFull")
            .or_else(|| xml_tag_text(xml, "avatarMedium"))
            .or_else(|| xml_tag_text(xml, "avatarIcon")),
    )
}

fn xml_tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let value = xml[start..end].trim();
    let value = value
        .strip_prefix("<![CDATA[")
        .and_then(|value| value.strip_suffix("]]>"))
        .unwrap_or(value)
        .trim();
    let value = xml_unescape(value).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

async fn create_session(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    user: UserSummary,
    source: SessionSource,
) -> AuthResult<AuthSession> {
    let Some(user) = active_user_for_session_tx(tx, user.id).await? else {
        return Err(AuthError::unauthorized("account is unavailable"));
    };
    let tokens = session_tokens(state)?;
    let session_family_id = Uuid::new_v4();
    let row = sqlx::query(
        "insert into sessions (
            user_id,
            refresh_token_hash,
            access_token_hash,
            access_token_expires_at,
            expires_at,
            absolute_expires_at,
            last_used_at,
            auth_source,
            session_family_id
         ) values (
            $1,
            $2,
            $3,
            least(
                now() + make_interval(secs => $4),
                now() + make_interval(secs => $6)
            ),
            least(
                now() + make_interval(secs => $5),
                now() + make_interval(secs => $6)
            ),
            now() + make_interval(secs => $6),
            now(),
            $7,
            $8
         )
         returning greatest(
             1,
             extract(epoch from (access_token_expires_at - now()))::bigint
         ) as access_token_ttl_seconds",
    )
    .bind(user.id)
    .bind(&tokens.refresh_hash)
    .bind(&tokens.access_hash)
    .bind(ACCESS_TOKEN_EXPIRES_SECONDS as i32)
    .bind(REFRESH_TOKEN_EXPIRES_SECONDS as i32)
    .bind(ABSOLUTE_SESSION_EXPIRES_SECONDS as i32)
    .bind(source.as_db())
    .bind(session_family_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(tokens.into_session(user, row.try_get("access_token_ttl_seconds")?))
}

async fn rotate_session(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    user: UserSummary,
) -> AuthResult<AuthSession> {
    let tokens = session_tokens(state)?;
    let rotated = sqlx::query(
        "update sessions
         set refresh_token_used_at = now(), last_used_at = now()
         where id = $1
           and refresh_token_used_at is null
           and revoked_at is null
         returning id",
    )
    .bind(session_id)
    .fetch_optional(&mut **tx)
    .await?;
    if rotated.is_none() {
        return Err(AuthError::unauthorized("session is invalid or expired"));
    }

    let Some(row) = sqlx::query(
        "insert into sessions (
            user_id,
            refresh_token_hash,
            access_token_hash,
            access_token_expires_at,
            expires_at,
            absolute_expires_at,
            last_used_at,
            auth_source,
            session_family_id
         )
         select
            s.user_id,
            $2,
            $3,
            least(
                now() + make_interval(secs => $4),
                s.absolute_expires_at
            ),
            least(
                now() + make_interval(secs => $5),
                s.absolute_expires_at
            ),
            s.absolute_expires_at,
            now(),
            s.auth_source,
            s.session_family_id
         from sessions s
         where s.id = $1 and s.absolute_expires_at > now()
         returning greatest(
             1,
             extract(epoch from (access_token_expires_at - now()))::bigint
         ) as access_token_ttl_seconds",
    )
    .bind(session_id)
    .bind(&tokens.refresh_hash)
    .bind(&tokens.access_hash)
    .bind(ACCESS_TOKEN_EXPIRES_SECONDS as i32)
    .bind(REFRESH_TOKEN_EXPIRES_SECONDS as i32)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Err(AuthError::unauthorized("session is invalid or expired"));
    };

    Ok(tokens.into_session(user, row.try_get("access_token_ttl_seconds")?))
}

async fn active_user_for_session_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<Option<UserSummary>, sqlx::Error> {
    Ok(user_for_session_update_tx(tx, user_id)
        .await?
        .and_then(|(user, active)| active.then_some(user)))
}

async fn user_for_session_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<Option<(UserSummary, bool)>, sqlx::Error> {
    let Some(row) = sqlx::query(
        "select
            id,
            display_name,
            avatar_url,
            banned_at is null
              and (suspended_until is null or suspended_until <= now()) as active
         from users
         where id = $1
         for update",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };
    let active: bool = row.try_get("active")?;
    let user = UserSummary {
        id: row.try_get("id")?,
        display_name: row.try_get("display_name")?,
        avatar_url: row.try_get("avatar_url")?,
    };
    Ok(Some((user, active)))
}

async fn revoke_session_family_tx(
    tx: &mut Transaction<'_, Postgres>,
    session_family_id: Uuid,
) -> AuthResult<Vec<Uuid>> {
    let rows = sqlx::query(
        "update sessions
         set revoked_at = now()
         where session_family_id = $1 and revoked_at is null
         returning id",
    )
    .bind(session_family_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.iter()
        .map(|row| row.try_get("id").map_err(AuthError::from))
        .collect()
}

async fn session_family_token_count_tx(
    tx: &mut Transaction<'_, Postgres>,
    session_family_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("select count(*) from sessions where session_family_id = $1")
        .bind(session_family_id)
        .fetch_one(&mut **tx)
        .await
}

fn session_family_at_capacity(token_count: i64) -> bool {
    token_count >= MAX_SESSION_FAMILY_TOKENS
}

fn session_tokens(state: &AppState) -> AuthResult<SessionTokens> {
    let access_token = random_token();
    let refresh_token = random_token();
    Ok(SessionTokens {
        access_hash: token_hash(&access_token, &state.config.session_secret)?,
        refresh_hash: token_hash(&refresh_token, &state.config.session_secret)?,
        access_token,
        refresh_token,
    })
}

struct SessionTokens {
    access_hash: String,
    refresh_hash: String,
    access_token: String,
    refresh_token: String,
}

impl SessionTokens {
    fn into_session(self, user: UserSummary, access_token_ttl_seconds: i64) -> AuthSession {
        AuthSession {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_in_seconds: access_token_ttl_seconds.max(1) as u64,
            user,
        }
    }
}

async fn expire_old_login_challenges(state: &AppState) -> AuthResult<()> {
    sqlx::query(
        "update steam_login_challenges
         set status = 'expired'
         where status = 'pending' and expires_at <= now()",
    )
    .execute(&state.db)
    .await?;
    opportunistic_auth_cleanup(state).await;
    Ok(())
}

async fn opportunistic_auth_cleanup(state: &AppState) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let last_cleanup = LAST_AUTH_CLEANUP_UNIX_SECONDS.load(Ordering::Relaxed);
    if now.saturating_sub(last_cleanup) < AUTH_CLEANUP_INTERVAL_SECONDS
        || LAST_AUTH_CLEANUP_UNIX_SECONDS
            .compare_exchange(last_cleanup, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    if let Err(error) = delete_expired_auth_records(&state.db).await {
        LAST_AUTH_CLEANUP_UNIX_SECONDS.store(0, Ordering::Relaxed);
        warn!(%error, "failed to clean up expired auth records");
    }
}

async fn delete_expired_auth_records(db: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "delete from steam_login_challenges
         where expires_at <= now() - make_interval(secs => $1)",
    )
    .bind(LOGIN_CHALLENGE_RETENTION_SECONDS as i32)
    .execute(db)
    .await?;
    sqlx::query(
        "delete from sessions
         where absolute_expires_at <= now()
             or (
                 revoked_at is not null
                and revoked_at <= now() - make_interval(secs => $1)
            )",
    )
    .bind(AUTH_RECORD_RETENTION_SECONDS as i32)
    .execute(db)
    .await?;
    Ok(())
}

fn steam_login_url(public_url: &str, challenge_id: Uuid) -> AuthResult<String> {
    let return_to = steam_return_to(public_url, challenge_id);
    let mut url = Url::parse(STEAM_OPENID_ENDPOINT)
        .map_err(|error| AuthError::internal(format!("invalid Steam OpenID URL: {error}")))?;
    url.query_pairs_mut()
        .append_pair("openid.ns", "http://specs.openid.net/auth/2.0")
        .append_pair("openid.mode", "checkid_setup")
        .append_pair("openid.return_to", &return_to)
        .append_pair("openid.identity", STEAM_IDENTIFIER_SELECT)
        .append_pair("openid.claimed_id", STEAM_IDENTIFIER_SELECT);
    if should_send_openid_realm(public_url) {
        url.query_pairs_mut()
            .append_pair("openid.realm", public_url);
    }
    Ok(url.to_string())
}

fn validate_openid_return_to(
    public_url: &str,
    challenge_id: Uuid,
    query: &HashMap<String, String>,
) -> AuthResult<()> {
    let actual = query
        .get("openid.return_to")
        .ok_or_else(|| AuthError::bad_request("missing Steam return_to"))?;
    let expected = steam_return_to(public_url, challenge_id);
    if actual == &expected {
        Ok(())
    } else {
        Err(AuthError::unauthorized(
            "Steam return_to did not match the login challenge",
        ))
    }
}

fn steam_return_to(public_url: &str, challenge_id: Uuid) -> String {
    let public_url = public_url.trim_end_matches('/');
    format!("{public_url}/auth/steam/callback?challenge_id={challenge_id}")
}

fn steam_login_success_page(display_name: &str) -> String {
    steam_login_page(
        "Steam login complete",
        "Steam login complete",
        &format!(
            "Signed in as <strong>{}</strong>. You can return to AOM.",
            html_escape(display_name)
        ),
        false,
    )
}

fn steam_login_error_page(message: &str) -> String {
    steam_login_page(
        "Steam login failed",
        "Steam login failed",
        &html_escape(message),
        true,
    )
}

fn steam_login_page(title: &str, heading: &str, message_html: &str, error: bool) -> String {
    let status_html = if error {
        r#"<span class="status error">Try again</span>"#
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{}</title>
<style>
:root {{
    --aom-bg: #070909;
    --aom-panel-top: #171815;
    --aom-panel-bottom: #070909;
    --aom-frame: #a57932;
    --aom-frame-mid: #c49845;
    --aom-frame-bright: #ecd48b;
    --aom-frame-dark: #251806;
    --aom-text: #f5efe0;
    --aom-muted: #b2a78f;
}}
* {{ box-sizing: border-box; }}
body {{
    display: grid;
    min-height: 100vh;
    margin: 0;
    place-items: center;
    color: var(--aom-text);
    background: radial-gradient(circle at top, #1d1e19 0, var(--aom-bg) 56%);
    font-family: Segoe UI, system-ui, sans-serif;
}}
.card {{
    width: min(92vw, 520px);
    padding: 28px;
    border: 3px solid var(--aom-frame-mid);
    border-radius: 6px;
    background: linear-gradient(180deg, var(--aom-panel-top), var(--aom-panel-bottom));
    box-shadow: inset 0 0 0 1px var(--aom-frame-bright), 0 0 0 1px var(--aom-frame-dark), 0 18px 52px rgba(0, 0, 0, 0.5);
}}
.eyebrow {{
    margin-bottom: 10px;
    color: var(--aom-frame-bright);
    font-size: 13px;
    font-weight: 800;
    letter-spacing: .12em;
    text-transform: uppercase;
}}
h1 {{
    margin: 0 0 12px;
    font-size: clamp(28px, 6vw, 42px);
    line-height: 1;
}}
p {{
    margin: 0;
    color: var(--aom-muted);
    font-size: 17px;
    line-height: 1.5;
}}
strong {{ color: var(--aom-text); }}
.status {{
    display: inline-block;
    margin-top: 22px;
    padding: 8px 11px;
    border: 1px solid var(--aom-frame-bright);
    border-radius: 999px;
    color: #1b1207;
    background: linear-gradient(180deg, #d6b45b, #7c5e23);
    font-size: 12px;
    font-weight: 800;
}}
.status.error {{
    color: #fecaca;
    border-color: #7f1d1d;
    background: rgba(127, 29, 29, .35);
}}
</style>
</head>
<body>
<main class="card">
    <div class="eyebrow">Agora</div>
    <h1>{}</h1>
    <p>{}</p>
    {}
</main>
</body>
</html>"#,
        html_escape(title),
        html_escape(heading),
        message_html,
        status_html
    )
}

fn should_send_openid_realm(public_url: &str) -> bool {
    let Ok(url) = Url::parse(public_url) else {
        return false;
    };
    match url.host_str() {
        Some(host)
            if host.eq_ignore_ascii_case("localhost")
                || matches!(host, "127.0.0.1" | "::1" | "[::1]") =>
        {
            false
        }
        Some(_) => true,
        None => false,
    }
}

fn is_local_dev_login_request(
    peer_addr: SocketAddr,
    headers: &HeaderMap,
    proxy_token: Option<&str>,
) -> bool {
    peer_addr.ip().is_loopback()
        || proxy_token.is_some_and(|token| {
            headers
                .get("x-agora-dev-login-proxy")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == token)
        })
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn token_hash(token: &str, secret: &str) -> AuthResult<String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| AuthError::internal("failed to initialize token hasher"))?;
    mac.update(token.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn dev_provider_user_id(account_id: &str) -> String {
    format!("local:{account_id}")
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[derive(Clone)]
struct SteamProfile {
    display_name: String,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct SteamPlayerSummariesResponse {
    response: SteamPlayersResponse,
}

#[derive(Deserialize)]
struct SteamPlayersResponse {
    players: Vec<SteamPlayer>,
}

#[derive(Deserialize)]
struct SteamPlayer {
    personaname: Option<String>,
    avatar: Option<String>,
    avatarmedium: Option<String>,
    avatarfull: Option<String>,
}

#[derive(Debug)]
pub(crate) struct AuthError {
    status: StatusCode,
    message: String,
}

impl AuthError {
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

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
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

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let status = self.status;
        let message = self.message;
        (status, Json(ApiError { message })).into_response()
    }
}

impl From<sqlx::Error> for AuthError {
    fn from(error: sqlx::Error) -> Self {
        warn!(%error, "database error in auth route");
        Self::internal("database operation failed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "postgres-tests")]
    use std::sync::Arc;

    #[cfg(feature = "postgres-tests")]
    use sqlx::PgPool;

    #[cfg(feature = "postgres-tests")]
    fn auth_test_config(
        enable_dev_login: bool,
        dev_login_accounts: Vec<DevLoginAccount>,
    ) -> crate::Config {
        crate::Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            public_url: "http://localhost:8080".to_string(),
            database_url: "postgres://unused".to_string(),
            database_max_connections: 5,
            minimum_client_version: "0.5.0".to_string(),
            run_migrations: false,
            session_secret: "test-session-secret-with-enough-length".to_string(),
            steam_web_api_key: None,
            enable_dev_login,
            dev_login_accounts,
            dev_login_proxy_token: None,
            trusted_proxy_cidrs: Vec::new(),
            websocket_max_connections: 8,
        }
    }

    #[cfg(feature = "postgres-tests")]
    fn auth_test_state(db: PgPool) -> AppState {
        let (_shutdown_tx, shutdown) = tokio::sync::watch::channel(false);
        AppState {
            config: Arc::new(auth_test_config(false, Vec::new())),
            db,
            http: reqwest::Client::new(),
            chat_tx: chat::broadcast_channel(),
            realtime_access: Arc::new(chat::RealtimeAccess::new()),
            realtime_state_lock: Arc::new(tokio::sync::Mutex::new(())),
            direct_message_delivery_locks: Arc::new(chat::DirectMessageDeliveryLocks::new()),
            visibility_epoch: Arc::new(AtomicU64::new(0)),
            presence: Arc::new(crate::presence::PresenceTracker::new()),
            rate_limits: Arc::new(crate::rate_limit::RateLimiters::new()),
            websocket_connections: Arc::new(tokio::sync::Semaphore::new(8)),
            snapshot_delivery: Arc::new(chat::SnapshotDelivery::new()),
            shutdown,
        }
    }

    #[cfg(feature = "postgres-tests")]
    async fn insert_auth_test_user(pool: &PgPool, display_name: &str) -> UserSummary {
        let row = sqlx::query(
            "insert into users (display_name, avatar_url)
             values ($1, 'https://avatars.example/original.png')
             returning id, display_name, avatar_url",
        )
        .bind(display_name)
        .fetch_one(pool)
        .await
        .unwrap();
        UserSummary {
            id: row.try_get("id").unwrap(),
            display_name: row.try_get("display_name").unwrap(),
            avatar_url: row.try_get("avatar_url").unwrap(),
        }
    }

    #[test]
    fn extracts_steam_id_from_claimed_id() {
        let mut query = HashMap::new();
        query.insert(
            "openid.claimed_id".to_string(),
            "https://steamcommunity.com/openid/id/76561198000000000".to_string(),
        );

        assert_eq!(
            steam_id_from_openid_claim(&query).unwrap(),
            "76561198000000000"
        );
    }

    #[test]
    fn rejects_invalid_steam_claim() {
        let mut query = HashMap::new();
        query.insert(
            "openid.claimed_id".to_string(),
            "https://example.com/not-steam".to_string(),
        );

        assert!(steam_id_from_openid_claim(&query).is_err());
    }

    #[test]
    fn rejects_non_steam_numeric_claim() {
        let mut query = HashMap::new();
        query.insert(
            "openid.claimed_id".to_string(),
            "https://example.com/openid/id/76561198000000000".to_string(),
        );

        assert!(steam_id_from_openid_claim(&query).is_err());
    }

    #[test]
    fn validates_openid_return_to_against_challenge() {
        let challenge_id = Uuid::new_v4();
        let mut query = HashMap::new();
        query.insert(
            "openid.return_to".to_string(),
            steam_return_to("https://agora.example", challenge_id),
        );

        assert!(validate_openid_return_to("https://agora.example", challenge_id, &query).is_ok());
        assert!(
            validate_openid_return_to("https://agora.example", Uuid::new_v4(), &query).is_err()
        );
    }

    #[test]
    fn omits_openid_realm_for_localhost() {
        let url = steam_login_url("http://localhost", Uuid::nil()).unwrap();

        assert!(!url.contains("openid.realm"));
        assert!(url.contains("openid.return_to"));
        assert!(!should_send_openid_realm("http://[::1]:8080"));
    }

    #[test]
    fn includes_openid_realm_for_public_urls() {
        let url = steam_login_url("https://agora.example", Uuid::nil()).unwrap();

        assert!(url.contains("openid.realm=https%3A%2F%2Fagora.example"));
    }

    #[test]
    fn success_page_tells_users_to_return_to_aom() {
        let page = steam_login_success_page("Arkantos");

        assert!(page.contains("Signed in as <strong>Arkantos</strong>."));
        assert!(page.contains("You can return to AOM."));
        assert!(!page.contains("Ready"));
        assert!(!page.contains("return to Agora"));
    }

    #[test]
    fn parses_steam_community_profile_name_and_avatar() {
        let profile = parse_steam_community_profile(
            r#"
            <profile>
                <steamID><![CDATA[Ark & Zeus]]></steamID>
                <avatarFull><![CDATA[https://avatars.steamstatic.com/full.jpg]]></avatarFull>
            </profile>
            "#,
        )
        .unwrap();

        assert_eq!(profile.display_name, "Ark & Zeus");
        assert_eq!(
            profile.avatar_url,
            Some("https://avatars.steamstatic.com/full.jpg".to_string())
        );
    }

    #[test]
    fn incomplete_steam_profile_does_not_replace_an_existing_profile() {
        assert!(parse_steam_community_profile(
            r#"<profile><avatarMedium>https://avatars.steamstatic.com/medium.jpg</avatarMedium></profile>"#,
        )
        .is_none());
        assert_eq!(
            fallback_steam_profile("76561198000000000").display_name,
            "Steam 76561198000000000"
        );
    }

    #[test]
    fn xml_tag_text_decodes_entities() {
        assert_eq!(
            xml_tag_text("<steamID>Ark &amp; Zeus</steamID>", "steamID").as_deref(),
            Some("Ark & Zeus")
        );
    }

    #[test]
    fn owner_has_admin_capabilities() {
        let principal = Principal {
            session_id: Uuid::nil(),
            user: UserSummary {
                id: Uuid::nil(),
                display_name: "Owner".to_string(),
                avatar_url: None,
            },
            role: UserRole::Owner,
        };

        assert!(principal.is_moderator());
        assert!(principal.is_admin());
    }

    #[test]
    fn allows_dev_login_only_from_loopback_or_authenticated_proxy() {
        let headers = HeaderMap::new();
        assert!(is_local_dev_login_request(
            "127.0.0.1:8080".parse().unwrap(),
            &headers,
            None
        ));
        assert!(!is_local_dev_login_request(
            "10.0.0.2:8080".parse().unwrap(),
            &headers,
            Some("secret")
        ));

        let mut proxy_headers = HeaderMap::new();
        proxy_headers.insert("x-agora-dev-login-proxy", "secret".parse().unwrap());
        assert!(is_local_dev_login_request(
            "10.0.0.2:8080".parse().unwrap(),
            &proxy_headers,
            Some("secret")
        ));
    }

    #[test]
    fn local_dev_provider_ids_use_configured_account_ids() {
        assert_eq!(dev_provider_user_id("alice"), "local:alice");
        assert_eq!(dev_provider_user_id("staff_admin"), "local:staff_admin");
    }

    #[test]
    fn serializes_configured_local_dev_roles() {
        assert_eq!(user_role_as_db(UserRole::User), "user");
        assert_eq!(user_role_as_db(UserRole::Moderator), "moderator");
        assert_eq!(user_role_as_db(UserRole::Admin), "admin");
        assert_eq!(user_role_as_db(UserRole::Owner), "owner");
    }

    #[test]
    fn local_test_sessions_require_an_enabled_configured_fixture() {
        let accounts = vec![DevLoginAccount {
            account_id: "admin".to_string(),
            display_name: "Admin".to_string(),
            role: UserRole::Admin,
        }];

        assert_eq!(
            local_test_role(true, &accounts, Some("local:admin")),
            Some(UserRole::Admin)
        );
        assert_eq!(
            local_test_role(true, &accounts, Some("local:removed")),
            None
        );
        assert_eq!(local_test_role(false, &accounts, Some("local:admin")), None);
    }

    #[test]
    fn session_family_token_capacity_is_bounded() {
        assert_eq!(
            MAX_SESSION_FAMILY_TOKENS,
            (ABSOLUTE_SESSION_EXPIRES_SECONDS / MIN_REFRESH_ROTATION_INTERVAL_SECONDS) as i64 + 1
        );
        assert!(!session_family_at_capacity(MAX_SESSION_FAMILY_TOKENS - 1));
        assert!(session_family_at_capacity(MAX_SESSION_FAMILY_TOKENS));
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn disabled_local_dev_reconciliation_does_not_change_roles(pool: PgPool) {
        let user = insert_auth_test_user(&pool, "disabled-dev-reconciliation").await;
        sqlx::query("update users set role = 'admin' where id = $1")
            .bind(user.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "insert into identities (user_id, provider, provider_user_id)
             values ($1, 'dev', 'local:retired')",
        )
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();

        reconcile_local_test_accounts(&pool, &auth_test_config(false, Vec::new()))
            .await
            .unwrap();

        let role = sqlx::query_scalar::<_, String>("select role from users where id = $1")
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(role, "admin");
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn local_dev_reconciliation_preserves_steam_user_roles(pool: PgPool) {
        let configured = insert_auth_test_user(&pool, "Steam Configured").await;
        let retired = insert_auth_test_user(&pool, "Steam Retired").await;
        for (user, dev_id, steam_id) in [
            (configured.id, "local:alice", "76561198000000011"),
            (retired.id, "local:retired", "76561198000000012"),
        ] {
            sqlx::query("update users set role = 'admin' where id = $1")
                .bind(user)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "insert into identities (user_id, provider, provider_user_id)
                 values ($1, 'dev', $2), ($1, 'steam', $3)",
            )
            .bind(user)
            .bind(dev_id)
            .bind(steam_id)
            .execute(&pool)
            .await
            .unwrap();
        }

        reconcile_local_test_accounts(
            &pool,
            &auth_test_config(
                true,
                vec![DevLoginAccount {
                    account_id: "alice".to_string(),
                    display_name: "Alice".to_string(),
                    role: UserRole::User,
                }],
            ),
        )
        .await
        .unwrap();

        let users = sqlx::query_as::<_, (String, String)>(
            "select display_name, role from users where id = any($1) order by display_name",
        )
        .bind(vec![configured.id, retired.id])
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            users,
            vec![
                ("Steam Configured".to_string(), "admin".to_string()),
                ("Steam Retired".to_string(), "admin".to_string()),
            ]
        );
    }

    #[test]
    fn parses_bearer_tokens_case_insensitively() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "bearer token".parse().unwrap());

        assert_eq!(bearer_token(&headers), Some("token"));
    }

    #[test]
    fn rejects_malformed_bearer_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer token extra".parse().unwrap());
        assert_eq!(bearer_token(&headers), None);

        headers.insert(header::AUTHORIZATION, "Basic token".parse().unwrap());
        assert_eq!(bearer_token(&headers), None);
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_token_reuse_revokes_the_entire_family_and_notifies(pool: PgPool) {
        let state = auth_test_state(pool.clone());
        let user = insert_auth_test_user(&pool, "refresh-user").await;
        let initial = {
            let mut tx = pool.begin().await.unwrap();
            let session = create_session(&state, &mut tx, user.clone(), SessionSource::Steam)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            session
        };
        sqlx::query(
            "update sessions
             set created_at = now() - interval '6 minutes', last_used_at = now()
             where refresh_token_hash = $1",
        )
        .bind(token_hash(&initial.refresh_token, &state.config.session_secret).unwrap())
        .execute(&pool)
        .await
        .unwrap();

        let peer_addr = "127.0.0.1:4000".parse().unwrap();
        let axum::Json(first_rotation) = refresh(
            axum::extract::State(state.clone()),
            axum::extract::ConnectInfo(peer_addr),
            HeaderMap::new(),
            axum::Json(RefreshRequest {
                refresh_token: initial.refresh_token.clone(),
            }),
        )
        .await
        .unwrap();
        assert_ne!(first_rotation.session.refresh_token, initial.refresh_token);

        let mut notifications = state.chat_tx.subscribe();
        let replay = refresh(
            axum::extract::State(state.clone()),
            axum::extract::ConnectInfo(peer_addr),
            HeaderMap::new(),
            axum::Json(RefreshRequest {
                refresh_token: initial.refresh_token,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(replay.status, StatusCode::UNAUTHORIZED);

        let (members, revoked, used, shared_absolute_expiry) =
            sqlx::query_as::<_, (i64, i64, i64, bool)>(
                "select
                 count(*),
                 count(*) filter (where revoked_at is not null),
                 count(*) filter (where refresh_token_used_at is not null),
                 min(absolute_expires_at) = max(absolute_expires_at)
             from sessions
             where user_id = $1",
            )
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!((members, revoked, used), (2, 2, 1));
        assert!(shared_absolute_expiry);
        assert!(notifications.try_recv().is_ok());
        assert!(notifications.try_recv().is_ok());
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_rotation_requires_a_minimum_interval(pool: PgPool) {
        let state = auth_test_state(pool.clone());
        let user = insert_auth_test_user(&pool, "refresh-interval-user").await;
        let initial = {
            let mut tx = pool.begin().await.unwrap();
            let session = create_session(&state, &mut tx, user, SessionSource::Steam)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            session
        };

        let error = refresh(
            axum::extract::State(state.clone()),
            axum::extract::ConnectInfo("127.0.0.1:4003".parse().unwrap()),
            HeaderMap::new(),
            axum::Json(RefreshRequest {
                refresh_token: initial.refresh_token.clone(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
        let used = sqlx::query_scalar::<_, bool>(
            "select refresh_token_used_at is not null
             from sessions
             where refresh_token_hash = $1",
        )
        .bind(token_hash(&initial.refresh_token, &state.config.session_secret).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!used);
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_tombstone_replay_revokes_an_active_family(pool: PgPool) {
        let state = auth_test_state(pool.clone());
        let user = insert_auth_test_user(&pool, "refresh-tombstone-user").await;
        let initial = {
            let mut tx = pool.begin().await.unwrap();
            let session = create_session(&state, &mut tx, user.clone(), SessionSource::Steam)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            session
        };
        let initial_hash =
            token_hash(&initial.refresh_token, &state.config.session_secret).unwrap();
        sqlx::query(
            "update sessions
             set created_at = now() - interval '6 minutes'
             where refresh_token_hash = $1",
        )
        .bind(&initial_hash)
        .execute(&pool)
        .await
        .unwrap();
        let axum::Json(_) = refresh(
            axum::extract::State(state.clone()),
            axum::extract::ConnectInfo("127.0.0.1:4004".parse().unwrap()),
            HeaderMap::new(),
            axum::Json(RefreshRequest {
                refresh_token: initial.refresh_token.clone(),
            }),
        )
        .await
        .unwrap();
        sqlx::query(
            "update sessions
             set refresh_token_used_at = now() - interval '31 days'
             where refresh_token_hash = $1",
        )
        .bind(&initial_hash)
        .execute(&pool)
        .await
        .unwrap();

        delete_expired_auth_records(&pool).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "select count(*) from sessions where refresh_token_hash = $1",
            )
            .bind(&initial_hash)
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        let replay = refresh(
            axum::extract::State(state),
            axum::extract::ConnectInfo("127.0.0.1:4004".parse().unwrap()),
            HeaderMap::new(),
            axum::Json(RefreshRequest {
                refresh_token: initial.refresh_token,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "select count(*) from sessions where user_id = $1 and revoked_at is not null",
            )
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            2
        );
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_cannot_extend_an_absolute_session_expiry(pool: PgPool) {
        let state = auth_test_state(pool.clone());
        let user = insert_auth_test_user(&pool, "absolute-expiry-user").await;
        let initial = {
            let mut tx = pool.begin().await.unwrap();
            let session = create_session(&state, &mut tx, user.clone(), SessionSource::Steam)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            session
        };
        let refresh_hash =
            token_hash(&initial.refresh_token, &state.config.session_secret).unwrap();
        sqlx::query(
            "update sessions
             set absolute_expires_at = now() - interval '1 second'
             where refresh_token_hash = $1",
        )
        .bind(refresh_hash)
        .execute(&pool)
        .await
        .unwrap();

        let denied = refresh(
            axum::extract::State(state),
            axum::extract::ConnectInfo("127.0.0.1:4001".parse().unwrap()),
            HeaderMap::new(),
            axum::Json(RefreshRequest {
                refresh_token: initial.refresh_token,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
        let sessions =
            sqlx::query_scalar::<_, i64>("select count(*) from sessions where user_id = $1")
                .bind(user.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(sessions, 0);
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn failed_steam_profile_lookup_preserves_existing_user_profile(pool: PgPool) {
        let user = insert_auth_test_user(&pool, "Stored Steam Name").await;
        let steam_id = "76561198000000001";
        sqlx::query(
            "insert into identities (
                user_id,
                provider,
                provider_user_id,
                provider_display_name,
                provider_avatar_url
             ) values ($1, 'steam', $2, 'Stored Steam Name', 'https://avatars.example/original.png')",
        )
        .bind(user.id)
        .bind(steam_id)
        .execute(&pool)
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let returned = find_or_create_steam_user(&mut tx, steam_id, None)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        assert_eq!(returned, user);
        let stored = sqlx::query_as::<_, (String, Option<String>)>(
            "select display_name, avatar_url from users where id = $1",
        )
        .bind(user.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored.0, "Stored Steam Name");
        assert_eq!(
            stored.1.as_deref(),
            Some("https://avatars.example/original.png")
        );
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn suspended_steam_identities_are_rejected_before_completion(pool: PgPool) {
        let user = insert_auth_test_user(&pool, "suspended-steam-user").await;
        let steam_id = "76561198000000003";
        sqlx::query(
            "insert into identities (user_id, provider, provider_user_id)
             values ($1, 'steam', $2)",
        )
        .bind(user.id)
        .bind(steam_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("update users set suspended_until = now() + interval '1 hour' where id = $1")
            .bind(user.id)
            .execute(&pool)
            .await
            .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let error = find_or_create_steam_user(&mut tx, steam_id, None)
            .await
            .unwrap_err();
        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn steam_identity_creation_is_idempotent_during_a_race(pool: PgPool) {
        let steam_id = "76561198000000002".to_string();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let first_pool = pool.clone();
        let first_barrier = barrier.clone();
        let first_steam_id = steam_id.clone();
        let first = tokio::spawn(async move {
            let mut tx = first_pool.begin().await.unwrap();
            first_barrier.wait().await;
            let user = find_or_create_steam_user(
                &mut tx,
                &first_steam_id,
                Some(&SteamProfile {
                    display_name: "Concurrent Steam User".to_string(),
                    avatar_url: None,
                }),
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            user.id
        });
        let second_pool = pool.clone();
        let second_barrier = barrier.clone();
        let second_steam_id = steam_id.clone();
        let second = tokio::spawn(async move {
            let mut tx = second_pool.begin().await.unwrap();
            second_barrier.wait().await;
            let user = find_or_create_steam_user(
                &mut tx,
                &second_steam_id,
                Some(&SteamProfile {
                    display_name: "Concurrent Steam User".to_string(),
                    avatar_url: None,
                }),
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            user.id
        });
        let first_user = first.await.unwrap();
        let second_user = second.await.unwrap();

        assert_eq!(first_user, second_user);
        let identity_count = sqlx::query_scalar::<_, i64>(
            "select count(*)
             from identities
             where provider = 'steam' and provider_user_id = $1",
        )
        .bind(steam_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let user_count = sqlx::query_scalar::<_, i64>("select count(*) from users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(identity_count, 1);
        assert_eq!(user_count, 1);
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn restricted_users_cannot_consume_completed_steam_challenges(pool: PgPool) {
        let state = auth_test_state(pool.clone());
        let user = insert_auth_test_user(&pool, "restricted-user").await;
        sqlx::query("update users set banned_at = now() where id = $1")
            .bind(user.id)
            .execute(&pool)
            .await
            .unwrap();
        let poll_token = "completed-challenge-token";
        sqlx::query(
            "insert into steam_login_challenges (
                id,
                poll_token_hash,
                status,
                user_id,
                expires_at,
                completed_at
             ) values ($1, $2, 'complete', $3, now() + interval '1 minute', now())",
        )
        .bind(Uuid::new_v4())
        .bind(token_hash(poll_token, &state.config.session_secret).unwrap())
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();

        let axum::Json(response) = steam_login_poll(
            axum::extract::State(state),
            axum::extract::ConnectInfo("127.0.0.1:4002".parse().unwrap()),
            HeaderMap::new(),
            axum::Json(SteamLoginPollRequest {
                poll_token: poll_token.to_string(),
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            response.status,
            SteamLoginStatus::Denied { ref message } if message == "account is unavailable"
        ));
        let sessions =
            sqlx::query_scalar::<_, i64>("select count(*) from sessions where user_id = $1")
                .bind(user.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(sessions, 0);
    }

    #[cfg(feature = "postgres-tests")]
    #[sqlx::test(migrations = "./migrations")]
    async fn expired_auth_records_retain_active_refresh_tombstones(pool: PgPool) {
        let user = insert_auth_test_user(&pool, "retention-user").await;
        sqlx::query(
            "insert into sessions (
                user_id,
                refresh_token_hash,
                expires_at,
                absolute_expires_at,
                session_family_id,
                auth_source
             ) values (
                $1,
                'expired-refresh-record',
                now() - interval '31 days',
                now() - interval '31 days',
                $2,
                'steam'
             )",
        )
        .bind(user.id)
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into steam_login_challenges (id, poll_token_hash, expires_at)
             values ($1, 'expired-challenge-record', now() - interval '2 days')",
        )
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into sessions (
                user_id,
                refresh_token_hash,
                expires_at,
                absolute_expires_at,
                refresh_token_used_at,
                session_family_id,
                auth_source
             ) values (
                $1,
                'used-refresh-record',
                now() + interval '1 day',
                now() + interval '1 day',
                now() - interval '31 days',
                $2,
                'steam'
             )",
        )
        .bind(user.id)
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .unwrap();

        delete_expired_auth_records(&pool).await.unwrap();

        assert_eq!(
            sqlx::query_scalar::<_, i64>("select count(*) from sessions where user_id = $1",)
                .bind(user.id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
        sqlx::query(
            "update sessions
             set absolute_expires_at = now() - interval '1 second'
             where refresh_token_hash = 'used-refresh-record'",
        )
        .execute(&pool)
        .await
        .unwrap();
        delete_expired_auth_records(&pool).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("select count(*) from sessions where user_id = $1",)
                .bind(user.id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("select count(*) from steam_login_challenges")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
}

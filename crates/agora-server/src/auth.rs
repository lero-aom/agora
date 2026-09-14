use std::{collections::HashMap, net::SocketAddr};

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

use crate::{chat, AppState};

const STEAM_OPENID_ENDPOINT: &str = "https://steamcommunity.com/openid/login";
const STEAM_IDENTIFIER_SELECT: &str = "http://specs.openid.net/auth/2.0/identifier_select";
const LOGIN_CHALLENGE_EXPIRES_SECONDS: u64 = 10 * 60;
const ACCESS_TOKEN_EXPIRES_SECONDS: u64 = 15 * 60;
const REFRESH_TOKEN_EXPIRES_SECONDS: u64 = 30 * 24 * 60 * 60;

type HmacSha256 = Hmac<Sha256>;
type AuthResult<T> = Result<T, AuthError>;

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
            .check_auth(peer_addr, &parts.headers, state.config.trust_proxy_headers)
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
           and u.banned_at is null
           and (u.suspended_until is null or u.suspended_until <= now())
         returning
            s.id as session_id,
            u.id as user_id,
            u.display_name,
            u.avatar_url,
            u.role,
            greatest(1, extract(epoch from (s.access_token_expires_at - now()))::bigint) as access_token_ttl_seconds",
    )
    .bind(access_token_hash)
    .fetch_optional(&state.db)
    .await?
    else {
        return Ok(None);
    };

    let ttl: i64 = row.try_get("access_token_ttl_seconds")?;
    Ok(Some(AuthenticatedSession {
        session_id: row.try_get("session_id")?,
        user: UserSummary {
            id: row.try_get("user_id")?,
            display_name: row.try_get("display_name")?,
            avatar_url: row.try_get("avatar_url")?,
        },
        role: user_role_from_db(row.try_get::<String, _>("role")?.as_str()),
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

pub(crate) async fn session_activity(
    state: &AppState,
    session_id: Uuid,
) -> Result<SessionActivity, sqlx::Error> {
    let Some((access_token_active, session_active)) = sqlx::query_as::<_, (bool, bool)>(
        "select
            s.access_token_expires_at > now() as access_token_active,
            s.revoked_at is null
              and s.expires_at > now()
              and u.banned_at is null
              and (u.suspended_until is null or u.suspended_until <= now()) as session_active
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

    Ok(if !session_active {
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
        .check_auth(peer_addr, &headers, state.config.trust_proxy_headers)
        .await
        .map_err(|error| AuthError::too_many_requests(error.message()))?;

    let display_name = normalize_dev_display_name(
        request
            .and_then(|Json(request)| request.display_name)
            .as_deref(),
    )?;
    let provider_user_id = dev_provider_user_id(&display_name);

    let mut tx = state.db.begin().await?;
    let user = find_or_create_dev_user(&mut tx, &provider_user_id, &display_name).await?;
    let session = create_session(&state, &mut tx, user).await?;
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
        .check_auth(peer_addr, &headers, state.config.trust_proxy_headers)
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
        .check_auth(peer_addr, &headers, state.config.trust_proxy_headers)
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
            c.consumed_at is not null as consumed,
            u.display_name,
            u.avatar_url
         from steam_login_challenges c
         left join users u on u.id = c.user_id
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
            let user = UserSummary {
                id: user_id,
                display_name: row.try_get("display_name")?,
                avatar_url: row.try_get("avatar_url")?,
            };
            let mut tx = state.db.begin().await?;
            let claimed = sqlx::query(
                "update steam_login_challenges
                 set consumed_at = now()
                 where id = $1 and consumed_at is null
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

            let session = create_session(&state, &mut tx, user).await?;
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
        .check_auth(peer_addr, &headers, state.config.trust_proxy_headers)
        .await
        .map_err(|error| AuthError::too_many_requests(error.message()))?;
    let refresh_token = request.refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(AuthError::bad_request("refresh token is required"));
    }

    let refresh_token_hash = token_hash(refresh_token, &state.config.session_secret)?;
    let mut tx = state.db.begin().await?;
    let Some(row) = sqlx::query(
        "select
            s.id as session_id,
            u.id as user_id,
            u.display_name,
            u.avatar_url
         from sessions s
         join users u on u.id = s.user_id
          where s.refresh_token_hash = $1
            and s.revoked_at is null
            and s.expires_at > now()
            and u.banned_at is null
            and (u.suspended_until is null or u.suspended_until <= now())
          for update of s",
    )
    .bind(refresh_token_hash)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Err(AuthError::unauthorized("session is invalid or expired"));
    };

    let session_id: Uuid = row.try_get("session_id")?;
    let user = UserSummary {
        id: row.try_get("user_id")?,
        display_name: row.try_get("display_name")?,
        avatar_url: row.try_get("avatar_url")?,
    };
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
        .check_auth(peer_addr, &headers, state.config.trust_proxy_headers)
        .await
        .map_err(|error| AuthError::too_many_requests(error.message()))?;
    let refresh_token = request.refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(AuthError::bad_request("refresh token is required"));
    }

    let refresh_token_hash = token_hash(refresh_token, &state.config.session_secret)?;
    let rows = sqlx::query(
        "update sessions
         set revoked_at = now()
         where refresh_token_hash = $1 and revoked_at is null
         returning id",
    )
    .bind(refresh_token_hash)
    .fetch_all(&state.db)
    .await?;
    for row in &rows {
        chat::send_session_revoked(&state.chat_tx, row.try_get("id")?);
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
        find_or_create_steam_user(state, &steam_id).await
    }
    .await;

    match result {
        Ok(user) => {
            let result = sqlx::query(
                "update steam_login_challenges
                 set status = 'complete', user_id = $2, completed_at = now(), error = null
                 where id = $1 and status = 'pending' and expires_at > now()",
            )
            .bind(challenge_id)
            .bind(user.id)
            .execute(&state.db)
            .await?;
            if result.rows_affected() == 1 {
                Ok(user)
            } else {
                Err(AuthError::bad_request(
                    "login challenge expired or is no longer pending",
                ))
            }
        }
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

async fn find_or_create_steam_user(state: &AppState, steam_id: &str) -> AuthResult<UserSummary> {
    let profile = steam_profile(state, steam_id).await;
    let mut tx = state.db.begin().await?;
    if let Some(row) = sqlx::query(
        "select u.id, u.display_name, u.avatar_url
         from identities i
         join users u on u.id = i.user_id
         where i.provider = 'steam' and i.provider_user_id = $1",
    )
    .bind(steam_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let user = UserSummary {
            id: row.try_get("id")?,
            display_name: row.try_get("display_name")?,
            avatar_url: row.try_get("avatar_url")?,
        };
        update_steam_profile(&mut tx, user.id, steam_id, &profile).await?;
        tx.commit().await?;
        return Ok(UserSummary {
            display_name: profile.display_name,
            avatar_url: profile.avatar_url,
            ..user
        });
    }

    let row = sqlx::query(
        "insert into users (display_name, avatar_url)
         values ($1, $2)
         returning id, display_name, avatar_url",
    )
    .bind(&profile.display_name)
    .bind(&profile.avatar_url)
    .fetch_one(&mut *tx)
    .await?;
    let user = UserSummary {
        id: row.try_get("id")?,
        display_name: row.try_get("display_name")?,
        avatar_url: row.try_get("avatar_url")?,
    };

    sqlx::query(
        "insert into identities (
            user_id,
            provider,
            provider_user_id,
            provider_display_name,
            provider_avatar_url
         ) values ($1, 'steam', $2, $3, $4)",
    )
    .bind(user.id)
    .bind(steam_id)
    .bind(&profile.display_name)
    .bind(&profile.avatar_url)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(user)
}

async fn find_or_create_dev_user(
    tx: &mut Transaction<'_, Postgres>,
    provider_user_id: &str,
    display_name: &str,
) -> AuthResult<UserSummary> {
    if let Some(row) = sqlx::query(
        "select u.id, u.display_name, u.avatar_url
         from identities i
         join users u on u.id = i.user_id
         where i.provider = 'dev' and i.provider_user_id = $1",
    )
    .bind(provider_user_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        let id = row.try_get("id")?;
        sqlx::query(
            "update users
             set display_name = $2, last_seen_at = now()
             where id = $1",
        )
        .bind(id)
        .bind(display_name)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "update identities
             set provider_display_name = $2, updated_at = now()
             where provider = 'dev' and provider_user_id = $1",
        )
        .bind(provider_user_id)
        .bind(display_name)
        .execute(&mut **tx)
        .await?;

        return Ok(UserSummary {
            id,
            display_name: display_name.to_string(),
            avatar_url: row.try_get("avatar_url")?,
        });
    }

    let row = sqlx::query(
        "insert into users (display_name)
         values ($1)
         returning id, display_name, avatar_url",
    )
    .bind(display_name)
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
    .bind(provider_user_id)
    .bind(display_name)
    .execute(&mut **tx)
    .await?;

    Ok(user)
}

async fn update_steam_profile(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    steam_id: &str,
    profile: &SteamProfile,
) -> AuthResult<()> {
    sqlx::query(
        "update identities
         set provider_display_name = $3, provider_avatar_url = $4, updated_at = now()
         where provider = 'steam' and provider_user_id = $2 and user_id = $1",
    )
    .bind(user_id)
    .bind(steam_id)
    .bind(&profile.display_name)
    .bind(&profile.avatar_url)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "update users
         set display_name = $2, avatar_url = $3, last_seen_at = now()
         where id = $1",
    )
    .bind(user_id)
    .bind(&profile.display_name)
    .bind(&profile.avatar_url)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

async fn steam_profile(state: &AppState, steam_id: &str) -> SteamProfile {
    let fallback = || SteamProfile {
        display_name: format!("Steam {steam_id}"),
        avatar_url: None,
    };

    if let Some(api_key) = state.config.steam_web_api_key.as_ref() {
        if let Some(profile) = steam_web_api_profile(state, steam_id, api_key).await {
            return profile;
        }
    }

    steam_community_profile(state, steam_id)
        .await
        .unwrap_or_else(fallback)
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
                .map(|player| SteamProfile {
                    display_name: non_empty_or(player.personaname, format!("Steam {steam_id}")),
                    avatar_url: player.avatarfull.or(player.avatarmedium).or(player.avatar),
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
            Ok(body) => Some(parse_steam_community_profile(steam_id, &body)),
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

fn parse_steam_community_profile(steam_id: &str, xml: &str) -> SteamProfile {
    SteamProfile {
        display_name: non_empty_or(xml_tag_text(xml, "steamID"), format!("Steam {steam_id}")),
        avatar_url: xml_tag_text(xml, "avatarFull")
            .or_else(|| xml_tag_text(xml, "avatarMedium"))
            .or_else(|| xml_tag_text(xml, "avatarIcon")),
    }
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
) -> AuthResult<AuthSession> {
    let tokens = session_tokens(state)?;
    sqlx::query(
        "insert into sessions (
            user_id,
            refresh_token_hash,
            access_token_hash,
            access_token_expires_at,
            expires_at,
            last_used_at
         ) values (
            $1,
            $2,
            $3,
            now() + make_interval(secs => $4),
            now() + make_interval(secs => $5),
            now()
         )",
    )
    .bind(user.id)
    .bind(&tokens.refresh_hash)
    .bind(&tokens.access_hash)
    .bind(ACCESS_TOKEN_EXPIRES_SECONDS as i32)
    .bind(REFRESH_TOKEN_EXPIRES_SECONDS as i32)
    .execute(&mut **tx)
    .await?;

    Ok(tokens.into_session(user))
}

async fn rotate_session(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    user: UserSummary,
) -> AuthResult<AuthSession> {
    let tokens = session_tokens(state)?;
    sqlx::query(
        "update sessions
         set refresh_token_hash = $2,
             access_token_hash = $3,
             access_token_expires_at = now() + make_interval(secs => $4),
             expires_at = now() + make_interval(secs => $5),
             last_used_at = now()
         where id = $1",
    )
    .bind(session_id)
    .bind(&tokens.refresh_hash)
    .bind(&tokens.access_hash)
    .bind(ACCESS_TOKEN_EXPIRES_SECONDS as i32)
    .bind(REFRESH_TOKEN_EXPIRES_SECONDS as i32)
    .execute(&mut **tx)
    .await?;

    Ok(tokens.into_session(user))
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
    fn into_session(self, user: UserSummary) -> AuthSession {
        AuthSession {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_in_seconds: ACCESS_TOKEN_EXPIRES_SECONDS,
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
        Some("localhost") | Some("127.0.0.1") | Some("::1") => false,
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

fn non_empty_or(value: Option<String>, fallback: String) -> String {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
}

fn normalize_dev_display_name(value: Option<&str>) -> AuthResult<String> {
    let value = value.unwrap_or("Local Developer").trim();
    if value.is_empty() {
        return Err(AuthError::bad_request("display name cannot be empty"));
    }
    if value.chars().count() > 32 {
        return Err(AuthError::bad_request(
            "display name cannot exceed 32 characters",
        ));
    }
    Ok(value.to_string())
}

fn dev_provider_user_id(display_name: &str) -> String {
    format!("local:{}", display_name.trim().to_ascii_lowercase())
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
            "76561198000000000",
            r#"
            <profile>
                <steamID><![CDATA[Ark & Zeus]]></steamID>
                <avatarFull><![CDATA[https://avatars.steamstatic.com/full.jpg]]></avatarFull>
            </profile>
            "#,
        );

        assert_eq!(profile.display_name, "Ark & Zeus");
        assert_eq!(
            profile.avatar_url,
            Some("https://avatars.steamstatic.com/full.jpg".to_string())
        );
    }

    #[test]
    fn steam_community_profile_falls_back_to_steam_id_when_name_is_missing() {
        let profile = parse_steam_community_profile(
            "76561198000000000",
            r#"<profile><avatarMedium>https://avatars.steamstatic.com/medium.jpg</avatarMedium></profile>"#,
        );

        assert_eq!(profile.display_name, "Steam 76561198000000000");
        assert_eq!(
            profile.avatar_url,
            Some("https://avatars.steamstatic.com/medium.jpg".to_string())
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
    fn normalizes_dev_display_name() {
        assert_eq!(
            normalize_dev_display_name(Some(" Alice ")).unwrap(),
            "Alice"
        );
        assert_eq!(normalize_dev_display_name(None).unwrap(), "Local Developer");
        assert!(normalize_dev_display_name(Some("   ")).is_err());
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
}

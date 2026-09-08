use std::{collections::HashMap, net::SocketAddr};

use agora_common::{
    ApiError, AuthSession, DevLoginRequest, DevLoginResponse, LogoutRequest, LogoutResponse,
    RefreshRequest, RefreshResponse, SteamLoginPollRequest, SteamLoginPollResponse,
    SteamLoginStartResponse, SteamLoginStatus, UserSummary,
};
use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, StatusCode},
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

use crate::AppState;

const STEAM_OPENID_ENDPOINT: &str = "https://steamcommunity.com/openid/login";
const STEAM_IDENTIFIER_SELECT: &str = "http://specs.openid.net/auth/2.0/identifier_select";
const LOGIN_CHALLENGE_EXPIRES_SECONDS: u64 = 10 * 60;
const ACCESS_TOKEN_EXPIRES_SECONDS: u64 = 15 * 60;
const REFRESH_TOKEN_EXPIRES_SECONDS: u64 = 30 * 24 * 60 * 60;

type HmacSha256 = Hmac<Sha256>;
type AuthResult<T> = Result<T, AuthError>;

#[derive(Clone)]
pub(crate) struct AuthenticatedSession {
    pub(crate) user: UserSummary,
    pub(crate) access_token_ttl_seconds: u64,
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
            u.id as user_id,
            u.display_name,
            u.avatar_url,
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
        user: UserSummary {
            id: row.try_get("user_id")?,
            display_name: row.try_get("display_name")?,
            avatar_url: row.try_get("avatar_url")?,
        },
        access_token_ttl_seconds: ttl.max(1) as u64,
    }))
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
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !state.config.enable_dev_login || !is_loopback_host(host) {
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
        Ok(user) => Html(format!(
            r#"<!doctype html><title>Agora Steam Login</title><body style="font-family: sans-serif; background: #100b18; color: #f7efe2;"><h1>Steam login complete</h1><p>Signed in as <strong>{}</strong>. You can return to Agora.</p></body>"#,
            html_escape(&user.display_name)
        ))
        .into_response(),
        Err(error) => (
            error.status,
            Html(format!(
                r#"<!doctype html><title>Agora Steam Login</title><body style="font-family: sans-serif; background: #100b18; color: #f7efe2;"><h1>Steam login failed</h1><p>{}</p></body>"#,
                html_escape(&error.message)
            )),
        )
            .into_response(),
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
    let result = sqlx::query(
        "update sessions
         set revoked_at = now()
         where refresh_token_hash = $1 and revoked_at is null",
    )
    .bind(refresh_token_hash)
    .execute(&state.db)
    .await?;

    Ok(Json(LogoutResponse {
        revoked: result.rows_affected() > 0,
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
    let Some(api_key) = state.config.steam_web_api_key.as_ref() else {
        return fallback();
    };

    let response = state
        .http
        .get("https://api.steampowered.com/ISteamUser/GetPlayerSummaries/v0002/")
        .query(&[("key", api_key.as_str()), ("steamids", steam_id)])
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
                })
                .unwrap_or_else(fallback),
            Err(error) => {
                warn!(%error, "failed to parse Steam profile response");
                fallback()
            }
        },
        Err(error) => {
            warn!(%error, "failed to fetch Steam profile");
            fallback()
        }
    }
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

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    let host = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if host == "::1" {
        host
    } else {
        host.split(':').next().unwrap_or(host)
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
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
struct AuthError {
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
    fn recognizes_loopback_hosts() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("localhost:8080"));
        assert!(is_loopback_host("127.0.0.1:80"));
        assert!(is_loopback_host("[::1]:8080"));
        assert!(!is_loopback_host("agora.example"));
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

use std::{env, net::SocketAddr, sync::Arc, time::Duration};

use agora_common::{HealthResponse, VersionResponse, PROTOCOL_VERSION};
use anyhow::{bail, Context, Result};
use axum::{
    extract::State,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use url::Url;

mod auth;
mod chat;
mod db_events;
mod moderation;
mod presence;
mod rate_limit;
mod relationships;
mod visibility;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) config: Arc<Config>,
    pub(crate) db: PgPool,
    pub(crate) http: reqwest::Client,
    pub(crate) chat_tx: broadcast::Sender<chat::RealtimeEvent>,
    pub(crate) presence: Arc<presence::PresenceTracker>,
    pub(crate) rate_limits: Arc<rate_limit::RateLimiters>,
}

pub(crate) struct Config {
    pub(crate) bind_addr: SocketAddr,
    pub(crate) public_url: String,
    pub(crate) database_url: String,
    pub(crate) database_max_connections: u32,
    pub(crate) minimum_client_version: String,
    pub(crate) run_migrations: bool,
    pub(crate) session_secret: String,
    pub(crate) steam_web_api_key: Option<String>,
    pub(crate) enable_dev_login: bool,
    pub(crate) trust_proxy_headers: bool,
}

impl Config {
    fn from_env() -> Result<Self> {
        let bind_addr = env_or("AGORA_BIND_ADDR", "127.0.0.1:8080")
            .parse()
            .context("AGORA_BIND_ADDR must be a socket address")?;
        let database_max_connections = env_or("AGORA_DB_MAX_CONNECTIONS", "5")
            .parse()
            .context("AGORA_DB_MAX_CONNECTIONS must be an integer")?;

        let public_url = env_or("AGORA_PUBLIC_URL", "http://localhost:8080");
        let public_url_is_loopback = is_loopback_url(&public_url);
        validate_public_url(&public_url, public_url_is_loopback)?;
        let enable_dev_login_requested = env_bool("AGORA_ENABLE_DEV_LOGIN", false)?;
        if enable_dev_login_requested && !public_url_is_loopback {
            bail!("AGORA_ENABLE_DEV_LOGIN can only be true when AGORA_PUBLIC_URL is loopback");
        }
        let session_secret = env_or("AGORA_SESSION_SECRET", "dev-insecure-change-me");
        validate_session_secret(&session_secret, public_url_is_loopback)?;

        Ok(Self {
            bind_addr,
            enable_dev_login: enable_dev_login_requested,
            public_url,
            database_url: env_or(
                "DATABASE_URL",
                "postgres://agora:change-me@localhost:5432/agora",
            ),
            database_max_connections,
            minimum_client_version: env_or("AGORA_MIN_CLIENT_VERSION", env!("CARGO_PKG_VERSION")),
            run_migrations: env_bool("AGORA_RUN_MIGRATIONS", true)?,
            session_secret,
            steam_web_api_key: env_optional("STEAM_WEB_API_KEY"),
            trust_proxy_headers: env_bool("AGORA_TRUST_PROXY_HEADERS", false)?,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::from_env()?;
    let pool = PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .connect(&config.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    if config.run_migrations {
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("failed to run database migrations")?;
    }

    let bind_addr = config.bind_addr;
    let public_url = config.public_url.clone();
    if is_insecure_session_secret(&config.session_secret) {
        warn!("using development AGORA_SESSION_SECRET; set a long random value before deployment");
    }
    let state = AppState {
        config: Arc::new(config),
        db: pool,
        http: reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .context("failed to build HTTP client")?,
        chat_tx: chat::broadcast_channel(),
        presence: Arc::new(presence::PresenceTracker::new()),
        rate_limits: Arc::new(rate_limit::RateLimiters::new()),
    };
    db_events::spawn_database_event_listener(state.clone());
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;

    info!(%bind_addr, %public_url, "agora server listening");
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("server failed")
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .merge(auth::router())
        .merge(chat::router())
        .merge(moderation::router())
        .merge(relationships::router())
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                tracing::info_span!(
                    "request",
                    method = %request.method(),
                    path = %request.uri().path(),
                    version = ?request.version(),
                )
            }),
        )
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query_scalar::<_, i32>("select 1")
        .fetch_one(&state.db)
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(HealthResponse {
                status: "ok".to_string(),
                database: "ok".to_string(),
            }),
        ),
        Err(error) => {
            warn!(%error, "database health check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthResponse {
                    status: "degraded".to_string(),
                    database: "error".to_string(),
                }),
            )
        }
    }
}

async fn version(State(state): State<AppState>) -> Json<VersionResponse> {
    Json(VersionResponse {
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
        minimum_client_version: state.config.minimum_client_version.clone(),
    })
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        warn!(%error, "failed to install Ctrl+C handler");
    }
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

fn env_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_bool(name: &str, default: bool) -> Result<bool> {
    parse_env_bool(env::var(name).ok().as_deref(), default)
}

fn parse_env_bool(value: Option<&str>, default: bool) -> Result<bool> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value)
            if matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ) =>
        {
            Ok(true)
        }
        Some(value)
            if matches!(
                value.to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            ) =>
        {
            Ok(false)
        }
        Some(_) => bail!("boolean environment values must be true/false, yes/no, on/off, or 1/0"),
        None => Ok(default),
    }
}

fn env_optional(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn validate_public_url(value: &str, is_loopback: bool) -> Result<()> {
    let url = Url::parse(value).context("AGORA_PUBLIC_URL must be an absolute URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("AGORA_PUBLIC_URL must use http or https");
    }
    if url.host_str().is_none() {
        bail!("AGORA_PUBLIC_URL must include a host");
    }
    if !is_loopback && url.scheme() != "https" {
        bail!("AGORA_PUBLIC_URL must use https for non-local deployments");
    }
    Ok(())
}

fn validate_session_secret(secret: &str, allow_insecure: bool) -> Result<()> {
    if !allow_insecure && is_insecure_session_secret(secret) {
        bail!("AGORA_SESSION_SECRET must be at least 32 random characters and not a placeholder");
    }
    Ok(())
}

fn is_insecure_session_secret(secret: &str) -> bool {
    let normalized = secret.trim().to_ascii_lowercase();
    normalized.len() < 32
        || normalized.contains("change-me")
        || normalized.contains("replace-with")
        || normalized.contains("dev-insecure")
}

fn is_loopback_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(is_loopback_host))
        .unwrap_or(false)
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_http_only_for_loopback_public_urls() {
        assert!(validate_public_url("http://localhost:8080", true).is_ok());
        assert!(validate_public_url("http://agora.example", false).is_err());
        assert!(validate_public_url("https://agora.example", false).is_ok());
    }

    #[test]
    fn rejects_insecure_session_secrets_for_public_urls() {
        assert!(validate_session_secret("replace-with-at-least-32-random-bytes", false).is_err());
        assert!(validate_session_secret("short", false).is_err());
        assert!(validate_session_secret("0123456789abcdef0123456789abcdef", false).is_ok());
        assert!(validate_session_secret("short", true).is_ok());
    }

    #[test]
    fn empty_bool_env_values_use_defaults() {
        assert!(parse_env_bool(None, true).unwrap());
        assert!(!parse_env_bool(None, false).unwrap());
        assert!(parse_env_bool(Some(""), true).unwrap());
        assert!(!parse_env_bool(Some(""), false).unwrap());
        assert!(parse_env_bool(Some("true"), false).unwrap());
        assert!(!parse_env_bool(Some("false"), true).unwrap());
        assert!(parse_env_bool(Some("maybe"), true).is_err());
    }
}

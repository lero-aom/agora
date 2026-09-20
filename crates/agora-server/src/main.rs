use std::{
    collections::HashSet,
    env,
    net::SocketAddr,
    sync::{atomic::AtomicU64, Arc},
    time::Duration,
};

use agora_common::{HealthResponse, UserRole, VersionResponse, PROTOCOL_VERSION};
use anyhow::{bail, Context, Result};
use axum::{
    extract::State,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use sqlx::{postgres::PgPoolOptions, Connection, PgConnection, PgPool};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, watch, Semaphore};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use url::Url;

mod auth;
mod chat;
mod db_events;
mod dm;
mod moderation;
mod presence;
mod rate_limit;
mod relationships;
mod visibility;

const REALTIME_SINGLETON_LOCK_KEY: i64 = 0x4147_4F52_415F_5254;
const REALTIME_SINGLETON_HEARTBEAT_SECONDS: u64 = 5;
const DEFAULT_DEV_LOGIN_ACCOUNTS: &str =
    "alice:user,bob:user,reporter:user,target:user,moderator:moderator,admin:admin,owner:owner";
const MAX_DEV_LOGIN_ACCOUNT_ID_LEN: usize = 24;
const DEFAULT_MAX_WEBSOCKET_CONNECTIONS: &str = "256";
const MAX_WEBSOCKET_CONNECTIONS: usize = 10_000;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) config: Arc<Config>,
    pub(crate) db: PgPool,
    pub(crate) http: reqwest::Client,
    pub(crate) chat_tx: broadcast::Sender<chat::RealtimeEvent>,
    pub(crate) realtime_access: Arc<chat::RealtimeAccess>,
    pub(crate) realtime_state_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) direct_message_delivery_locks: Arc<chat::DirectMessageDeliveryLocks>,
    pub(crate) visibility_epoch: Arc<AtomicU64>,
    pub(crate) presence: Arc<presence::PresenceTracker>,
    pub(crate) rate_limits: Arc<rate_limit::RateLimiters>,
    pub(crate) websocket_connections: Arc<Semaphore>,
    pub(crate) snapshot_delivery: Arc<chat::SnapshotDelivery>,
    pub(crate) shutdown: watch::Receiver<bool>,
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
    pub(crate) microsoft_login: Option<MicrosoftLoginConfig>,
    pub(crate) enable_dev_login: bool,
    pub(crate) dev_login_accounts: Vec<DevLoginAccount>,
    pub(crate) dev_login_proxy_token: Option<String>,
    pub(crate) trusted_proxy_cidrs: Vec<rate_limit::TrustedProxy>,
    pub(crate) websocket_max_connections: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevLoginAccount {
    pub(crate) account_id: String,
    pub(crate) display_name: String,
    pub(crate) role: UserRole,
}

#[derive(Clone)]
pub(crate) struct MicrosoftLoginConfig {
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
}

struct RealtimeSingletonLock {
    release_tx: watch::Sender<bool>,
    heartbeat: tokio::task::JoinHandle<()>,
}

impl RealtimeSingletonLock {
    async fn release(self) {
        let _ = self.release_tx.send(true);
        if let Err(error) = self.heartbeat.await {
            error!(%error, "realtime singleton lock task ended unexpectedly");
        }
    }
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
        let dev_login_accounts = if enable_dev_login_requested {
            parse_dev_login_accounts(&env_or(
                "AGORA_DEV_LOGIN_ACCOUNTS",
                DEFAULT_DEV_LOGIN_ACCOUNTS,
            ))?
        } else {
            Vec::new()
        };
        validate_realtime_mode(&env_or("AGORA_REALTIME_MODE", "single-replica"))?;
        let session_secret = env_or("AGORA_SESSION_SECRET", "dev-insecure-change-me");
        validate_session_secret(&session_secret, public_url_is_loopback)?;
        let microsoft_login = microsoft_login_config_from_env()?;
        let minimum_client_version = env_or("AGORA_MIN_CLIENT_VERSION", env!("CARGO_PKG_VERSION"))
            .trim()
            .to_string();
        validate_minimum_client_version(&minimum_client_version)?;
        let trusted_proxy_cidrs = rate_limit::parse_trusted_proxy_cidrs(
            env_optional("AGORA_TRUSTED_PROXY_CIDRS").as_deref(),
        )
        .map_err(anyhow::Error::msg)?;
        let websocket_max_connections = parse_bounded_usize(
            &env_or(
                "AGORA_MAX_WEBSOCKET_CONNECTIONS",
                DEFAULT_MAX_WEBSOCKET_CONNECTIONS,
            ),
            "AGORA_MAX_WEBSOCKET_CONNECTIONS",
            MAX_WEBSOCKET_CONNECTIONS,
        )?;
        if env_bool("AGORA_TRUST_PROXY_HEADERS", false)? {
            warn!(
                "AGORA_TRUST_PROXY_HEADERS is ignored; set AGORA_TRUSTED_PROXY_CIDRS to trust a proxy"
            );
        }

        Ok(Self {
            bind_addr,
            enable_dev_login: enable_dev_login_requested,
            dev_login_accounts,
            public_url,
            database_url: env_or(
                "DATABASE_URL",
                "postgres://agora:change-me@localhost:5432/agora",
            ),
            database_max_connections,
            minimum_client_version,
            run_migrations: env_bool("AGORA_RUN_MIGRATIONS", true)?,
            session_secret,
            steam_web_api_key: env_optional("STEAM_WEB_API_KEY"),
            microsoft_login,
            dev_login_proxy_token: env_optional("AGORA_DEV_LOGIN_PROXY_TOKEN"),
            trusted_proxy_cidrs,
            websocket_max_connections,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::from_env()?;
    let (drain_shutdown_tx, drain_shutdown_rx) = watch::channel(false);
    let pool = PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .connect(&config.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;
    let realtime_lock = acquire_realtime_singleton_lock(&config.database_url).await?;

    if config.run_migrations {
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("failed to run database migrations")?;
    }
    if config.enable_dev_login {
        auth::reconcile_local_test_accounts(&pool, &config)
            .await
            .context("failed to reconcile local test accounts")?;
    }
    auth::revoke_unconfigured_local_test_sessions(&pool, &config)
        .await
        .context("failed to revoke disabled local test sessions")?;

    let bind_addr = config.bind_addr;
    let public_url = config.public_url.clone();
    let websocket_connections = Arc::new(Semaphore::new(config.websocket_max_connections));
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
        realtime_access: Arc::new(chat::RealtimeAccess::new()),
        realtime_state_lock: Arc::new(tokio::sync::Mutex::new(())),
        direct_message_delivery_locks: Arc::new(chat::DirectMessageDeliveryLocks::new()),
        visibility_epoch: Arc::new(AtomicU64::new(0)),
        presence: Arc::new(presence::PresenceTracker::new()),
        rate_limits: Arc::new(rate_limit::RateLimiters::new()),
        websocket_connections,
        snapshot_delivery: Arc::new(chat::SnapshotDelivery::new()),
        shutdown: drain_shutdown_rx,
    };
    db_events::spawn_database_event_listener(state.clone());
    chat::spawn_presence_reaper(state.clone());
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;

    info!(%bind_addr, %public_url, "agora server listening");
    let serve_result = axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        info!("shutdown signal received; draining active connections");
        let _ = drain_shutdown_tx.send(true);
    })
    .await;

    info!("active connections drained; releasing realtime singleton lock");
    realtime_lock.release().await;
    serve_result.context("server failed")
}

async fn acquire_realtime_singleton_lock(database_url: &str) -> Result<RealtimeSingletonLock> {
    let mut connection = PgConnection::connect(database_url)
        .await
        .context("failed to connect for realtime singleton lock")?;
    let acquired = sqlx::query_scalar::<_, bool>("select pg_try_advisory_lock($1)")
        .bind(REALTIME_SINGLETON_LOCK_KEY)
        .fetch_one(&mut connection)
        .await
        .context("failed to acquire realtime singleton lock")?;
    if !acquired {
        bail!("another Agora server already owns the single-replica realtime lock")
    }

    let (release_tx, mut release) = watch::channel(false);
    let heartbeat = tokio::spawn(async move {
        loop {
            tokio::select! {
                release_result = release.changed() => {
                    if release_result.is_err() || *release.borrow() {
                        return;
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(REALTIME_SINGLETON_HEARTBEAT_SECONDS)) => {
                    if let Err(error) = sqlx::query_scalar::<_, i32>("select 1")
                        .fetch_one(&mut connection)
                        .await
                    {
                        if *release.borrow() {
                            return;
                        }
                        error!(%error, "lost the realtime singleton lock connection; exiting to avoid split brain");
                        std::process::exit(1);
                    }
                }
            }
        }
    });
    Ok(RealtimeSingletonLock {
        release_tx,
        heartbeat,
    })
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .merge(auth::router())
        .merge(chat::router())
        .merge(dm::router())
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
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut interrupt = match signal(SignalKind::interrupt()) {
            Ok(signal) => signal,
            Err(error) => {
                warn!(%error, "failed to install SIGINT handler");
                wait_for_ctrl_c().await;
                return;
            }
        };
        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                warn!(%error, "failed to install SIGTERM handler");
                wait_for_ctrl_c().await;
                return;
            }
        };

        tokio::select! {
            _ = interrupt.recv() => info!("received SIGINT"),
            _ = terminate.recv() => info!("received SIGTERM"),
        }
    }

    #[cfg(not(unix))]
    wait_for_ctrl_c().await;
}

async fn wait_for_ctrl_c() {
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

fn microsoft_login_config_from_env() -> Result<Option<MicrosoftLoginConfig>> {
    microsoft_login_config_from_values(
        env_optional("AGORA_MICROSOFT_CLIENT_ID"),
        env_optional("AGORA_MICROSOFT_CLIENT_SECRET"),
    )
}

fn microsoft_login_config_from_values(
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Result<Option<MicrosoftLoginConfig>> {
    match (client_id, client_secret) {
        (None, None) => Ok(None),
        (Some(client_id), Some(client_secret)) => {
            if client_id.len() > 256 {
                bail!("AGORA_MICROSOFT_CLIENT_ID must be at most 256 characters")
            }
            if client_secret.len() > 4_096 {
                bail!("AGORA_MICROSOFT_CLIENT_SECRET must be at most 4096 characters")
            }
            Ok(Some(MicrosoftLoginConfig {
                client_id,
                client_secret,
            }))
        }
        _ => bail!(
            "AGORA_MICROSOFT_CLIENT_ID and AGORA_MICROSOFT_CLIENT_SECRET must be set together"
        ),
    }
}

fn parse_bounded_usize(value: &str, name: &str, max: usize) -> Result<usize> {
    let parsed = value
        .trim()
        .parse::<usize>()
        .with_context(|| format!("{name} must be an integer"))?;
    if parsed == 0 || parsed > max {
        bail!("{name} must be between 1 and {max}");
    }
    Ok(parsed)
}

fn validate_minimum_client_version(value: &str) -> Result<()> {
    if chat::is_valid_minimum_client_version(value) {
        Ok(())
    } else {
        bail!("AGORA_MIN_CLIENT_VERSION must be a stable semantic version such as 0.5.0")
    }
}

fn parse_dev_login_accounts(value: &str) -> Result<Vec<DevLoginAccount>> {
    let mut accounts = Vec::new();
    let mut account_ids = HashSet::new();

    for entry in value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let Some((account_id, role)) = entry.split_once(':') else {
            bail!("AGORA_DEV_LOGIN_ACCOUNTS entries must use account_id:role")
        };
        if role.contains(':') {
            bail!("AGORA_DEV_LOGIN_ACCOUNTS entries must use account_id:role")
        }
        let account_id = normalize_dev_account_id(account_id)?;
        if !account_ids.insert(account_id.clone()) {
            bail!("AGORA_DEV_LOGIN_ACCOUNTS cannot contain duplicate account IDs")
        }
        let role = match role.trim().to_ascii_lowercase().as_str() {
            "user" => UserRole::User,
            "moderator" => UserRole::Moderator,
            "admin" => UserRole::Admin,
            "owner" => UserRole::Owner,
            _ => bail!("AGORA_DEV_LOGIN_ACCOUNTS roles must be user, moderator, admin, or owner"),
        };
        accounts.push(DevLoginAccount {
            display_name: dev_account_display_name(&account_id),
            account_id,
            role,
        });
    }

    if accounts.is_empty() {
        bail!("AGORA_DEV_LOGIN_ACCOUNTS must define at least one account when dev login is enabled")
    }
    Ok(accounts)
}

pub(crate) fn normalize_dev_account_id(value: &str) -> Result<String> {
    let account_id = value.trim().to_ascii_lowercase();
    let bytes = account_id.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_DEV_LOGIN_ACCOUNT_ID_LEN {
        bail!("local test account IDs must be 1 to {MAX_DEV_LOGIN_ACCOUNT_ID_LEN} characters")
    }
    if !bytes[0].is_ascii_alphanumeric()
        || !bytes[bytes.len() - 1].is_ascii_alphanumeric()
        || !bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(*byte, b'-' | b'_')
        })
    {
        bail!("local test account IDs may contain only lowercase letters, digits, hyphens, and underscores")
    }
    Ok(account_id)
}

fn dev_account_display_name(account_id: &str) -> String {
    account_id
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            let Some(first) = characters.next() else {
                return String::new();
            };
            format!("{}{}", first.to_ascii_uppercase(), characters.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ")
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

fn validate_realtime_mode(value: &str) -> Result<()> {
    if value.trim() == "single-replica" {
        Ok(())
    } else {
        bail!("AGORA_REALTIME_MODE must be single-replica until shared realtime delivery ships")
    }
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
    host.eq_ignore_ascii_case("localhost") || matches!(host, "127.0.0.1" | "::1" | "[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_http_only_for_loopback_public_urls() {
        assert!(validate_public_url("http://localhost:8080", true).is_ok());
        assert!(is_loopback_url("http://[::1]:8080"));
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

    #[test]
    fn only_single_replica_realtime_mode_is_supported() {
        assert!(validate_realtime_mode("single-replica").is_ok());
        assert!(validate_realtime_mode("multi-instance").is_err());
    }

    #[test]
    fn validates_minimum_client_versions_at_startup() {
        assert!(validate_minimum_client_version("0.5.0").is_ok());
        assert!(validate_minimum_client_version("1.0.0+build.4").is_ok());
        assert!(validate_minimum_client_version("0.5").is_err());
        assert!(validate_minimum_client_version("0.5.0-beta").is_err());
    }

    #[test]
    fn bounds_websocket_connection_configuration() {
        assert_eq!(parse_bounded_usize("1", "LIMIT", 10).unwrap(), 1);
        assert_eq!(parse_bounded_usize("10", "LIMIT", 10).unwrap(), 10);
        assert!(parse_bounded_usize("0", "LIMIT", 10).is_err());
        assert!(parse_bounded_usize("11", "LIMIT", 10).is_err());
        assert!(parse_bounded_usize("nope", "LIMIT", 10).is_err());
    }

    #[test]
    fn microsoft_login_credentials_must_be_paired() {
        assert!(microsoft_login_config_from_values(None, None)
            .unwrap()
            .is_none());
        assert!(microsoft_login_config_from_values(Some("client-id".to_string()), None).is_err());
        assert!(
            microsoft_login_config_from_values(None, Some("client-secret".to_string())).is_err()
        );

        let config = microsoft_login_config_from_values(
            Some("client-id".to_string()),
            Some("client-secret".to_string()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(config.client_id, "client-id");
        assert_eq!(config.client_secret, "client-secret");
    }

    #[test]
    fn parses_local_dev_accounts_with_server_owned_roles() {
        let accounts = parse_dev_login_accounts("alice:user,staff_admin:admin").unwrap();

        assert_eq!(
            accounts,
            vec![
                DevLoginAccount {
                    account_id: "alice".to_string(),
                    display_name: "Alice".to_string(),
                    role: UserRole::User,
                },
                DevLoginAccount {
                    account_id: "staff_admin".to_string(),
                    display_name: "Staff Admin".to_string(),
                    role: UserRole::Admin,
                },
            ]
        );
    }

    #[test]
    fn rejects_invalid_local_dev_accounts() {
        assert!(parse_dev_login_accounts("").is_err());
        assert!(parse_dev_login_accounts("alice").is_err());
        assert!(parse_dev_login_accounts("alice:root").is_err());
        assert!(parse_dev_login_accounts("alice:user,alice:admin").is_err());
        assert!(parse_dev_login_accounts("not valid:user").is_err());
    }

    #[tokio::test]
    async fn draining_does_not_release_the_realtime_lock() {
        let (drain_tx, mut drain) = watch::channel(false);
        let (release_tx, mut release) = watch::channel(false);
        let heartbeat = tokio::spawn(async move {
            let _ = release.changed().await;
        });
        let mut release_probe = release_tx.subscribe();
        let realtime_lock = RealtimeSingletonLock {
            release_tx,
            heartbeat,
        };

        drain_tx.send(true).unwrap();
        drain.changed().await.unwrap();
        assert!(!*release_probe.borrow());

        realtime_lock.release().await;
        release_probe.changed().await.unwrap();
        assert!(*release_probe.borrow());
    }
}

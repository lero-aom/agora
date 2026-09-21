use super::{
    api::{api_error, http_client},
    server::{server_origin, server_url, RemoteLoginProvider, ServerOrigin},
};
use agora_common::{
    AuthSession, DevLoginRequest, DevLoginResponse, LogoutRequest, LogoutResponse,
    MicrosoftLoginPollRequest, MicrosoftLoginPollResponse, MicrosoftLoginStartResponse,
    MicrosoftLoginStatus, RefreshRequest, RefreshResponse, SteamLoginPollRequest,
    SteamLoginPollResponse, SteamLoginStartResponse, SteamLoginStatus,
};
use keyring::v1::{Entry, Error};

const KEYRING_SERVICE: &str = "agora";
const KEYRING_REFRESH_TOKEN_USER: &str = "refresh-token";
const KEYRING_REFRESH_TOKEN_QUARANTINE_USER: &str = "refresh-token-quarantine";
const REFRESH_TOKEN_QUARANTINE_MARKER: &str = "refresh-outcome-unknown";

pub(super) enum SavedRefreshToken {
    Scoped(String),
    Ambiguous,
    LegacyCredential,
    None,
}

pub(super) enum RefreshSessionError {
    Invalid(String),
    Retryable(String),
    Ambiguous(String),
}

pub(super) async fn complete_login(
    local_dev_account_id: &str,
    remote_login_provider: RemoteLoginProvider,
) -> Result<AuthSession, String> {
    if server_origin()?.is_loopback {
        complete_dev_login(local_dev_account_id).await
    } else {
        match remote_login_provider {
            RemoteLoginProvider::Steam => complete_steam_login().await,
            RemoteLoginProvider::Microsoft => complete_microsoft_login().await,
        }
    }
}

async fn complete_dev_login(account_id: &str) -> Result<AuthSession, String> {
    let server_url = server_url()?;
    let response = http_client()?
        .post(format!("{server_url}/auth/dev/login"))
        .json(&DevLoginRequest {
            account_id: account_id.trim().to_string(),
        })
        .send()
        .await
        .map_err(|error| format!("Could not reach local Agora server: {error}"))?;

    if !response.status().is_success() {
        return Err(api_error(response, "Local dev login failed").await);
    }

    response
        .json::<DevLoginResponse>()
        .await
        .map(|response| response.session)
        .map_err(|error| format!("Could not read local dev login response: {error}"))
}

async fn complete_steam_login() -> Result<AuthSession, String> {
    let start = request_steam_login().await?;
    webbrowser::open(&start.browser_url)
        .map_err(|error| format!("Could not open browser: {error}"))?;

    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(start.expires_in_seconds.saturating_add(5));
    loop {
        if std::time::Instant::now() >= deadline {
            return Err("Steam login expired".to_string());
        }

        match poll_steam_login(&start.poll_token).await? {
            SteamLoginStatus::Pending => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            SteamLoginStatus::Complete { session } => return Ok(session),
            SteamLoginStatus::Expired => return Err("Steam login expired".to_string()),
            SteamLoginStatus::Denied { message } => return Err(message),
        }
    }
}

async fn request_steam_login() -> Result<SteamLoginStartResponse, String> {
    let server_url = server_url()?;
    let response = http_client()?
        .post(format!("{server_url}/auth/steam/device/start"))
        .send()
        .await
        .map_err(|error| format!("Could not reach Agora server: {error}"))?;

    if !response.status().is_success() {
        return Err(api_error(response, "Steam login start failed").await);
    }

    response
        .json::<SteamLoginStartResponse>()
        .await
        .map_err(|error| format!("Could not read Steam login response: {error}"))
}

async fn poll_steam_login(poll_token: &str) -> Result<SteamLoginStatus, String> {
    let server_url = server_url()?;
    let response = http_client()?
        .post(format!("{server_url}/auth/steam/device/poll"))
        .json(&SteamLoginPollRequest {
            poll_token: poll_token.to_string(),
        })
        .send()
        .await
        .map_err(|error| format!("Could not poll Steam login: {error}"))?;

    if !response.status().is_success() {
        return Err(api_error(response, "Steam login polling failed").await);
    }

    response
        .json::<SteamLoginPollResponse>()
        .await
        .map(|response| response.status)
        .map_err(|error| format!("Could not read Steam login polling response: {error}"))
}

async fn complete_microsoft_login() -> Result<AuthSession, String> {
    let start = request_microsoft_login().await?;
    webbrowser::open(&start.browser_url)
        .map_err(|error| format!("Could not open browser: {error}"))?;

    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(start.expires_in_seconds.saturating_add(5));
    loop {
        if std::time::Instant::now() >= deadline {
            return Err("Microsoft login expired".to_string());
        }

        match poll_microsoft_login(&start.poll_token).await? {
            MicrosoftLoginStatus::Pending => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            MicrosoftLoginStatus::Complete { session } => return Ok(session),
            MicrosoftLoginStatus::Expired => return Err("Microsoft login expired".to_string()),
            MicrosoftLoginStatus::Denied { message } => return Err(message),
        }
    }
}

async fn request_microsoft_login() -> Result<MicrosoftLoginStartResponse, String> {
    let server_url = server_url()?;
    let response = http_client()?
        .post(format!("{server_url}/auth/microsoft/device/start"))
        .send()
        .await
        .map_err(|error| format!("Could not reach Agora server: {error}"))?;

    if !response.status().is_success() {
        return Err(api_error(response, "Microsoft login start failed").await);
    }

    response
        .json::<MicrosoftLoginStartResponse>()
        .await
        .map_err(|error| format!("Could not read Microsoft login response: {error}"))
}

async fn poll_microsoft_login(poll_token: &str) -> Result<MicrosoftLoginStatus, String> {
    let server_url = server_url()?;
    let response = http_client()?
        .post(format!("{server_url}/auth/microsoft/device/poll"))
        .json(&MicrosoftLoginPollRequest {
            poll_token: poll_token.to_string(),
        })
        .send()
        .await
        .map_err(|error| format!("Could not poll Microsoft login: {error}"))?;

    if !response.status().is_success() {
        return Err(api_error(response, "Microsoft login polling failed").await);
    }

    response
        .json::<MicrosoftLoginPollResponse>()
        .await
        .map(|response| response.status)
        .map_err(|error| format!("Could not read Microsoft login polling response: {error}"))
}

pub(super) async fn refresh_session(
    refresh_token: String,
) -> Result<AuthSession, RefreshSessionError> {
    let server_url = server_url().map_err(RefreshSessionError::Retryable)?;
    let client = http_client().map_err(RefreshSessionError::Retryable)?;
    let response = client
        .post(format!("{server_url}/auth/refresh"))
        .json(&RefreshRequest { refresh_token })
        .send()
        .await
        .map_err(|error| {
            // Once reqwest has started a request, a transport error cannot prove that the
            // single-use refresh token was not consumed by the server.
            RefreshSessionError::Ambiguous(format!("Could not refresh saved session: {error}"))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let error = api_error(response, "Saved session refresh failed").await;
        return Err(if refresh_session_is_invalid(status) {
            RefreshSessionError::Invalid(error)
        } else {
            RefreshSessionError::Retryable(error)
        });
    }

    response
        .json::<RefreshResponse>()
        .await
        .map(|response| response.session)
        .map_err(|error| {
            // A successful response status can still have consumed the token even when its
            // JSON body is truncated or malformed locally.
            RefreshSessionError::Ambiguous(format!(
                "Could not read session refresh response: {error}"
            ))
        })
}

pub(super) fn refresh_session_is_invalid(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED
}

pub(super) async fn logout_session(refresh_token: String) -> Result<bool, String> {
    let server_url = server_url()?;
    let response = http_client()?
        .post(format!("{server_url}/auth/logout"))
        .json(&LogoutRequest { refresh_token })
        .send()
        .await
        .map_err(|error| format!("Could not contact Agora server: {error}"))?;

    if response.status().is_success() {
        response
            .json::<LogoutResponse>()
            .await
            .map(|response| response.revoked)
            .map_err(|error| format!("Could not read server logout response: {error}"))
    } else {
        Err(api_error(response, "Server logout failed").await)
    }
}

pub(super) fn store_refresh_token(refresh_token: &str) -> Result<(), String> {
    let origin = server_origin()?;
    if origin.is_loopback {
        return Ok(());
    }
    if let Err(write_error) = refresh_token_entry(&origin)?.set_password(refresh_token) {
        return match quarantine_refresh_token_for_origin(&origin) {
            Ok(()) => Err(format!(
                "Windows Credential Manager write failed: {write_error}. The previous saved refresh token will not be retried."
            )),
            Err(quarantine_error) => Err(format!(
                "Windows Credential Manager write failed: {write_error}. The previous saved refresh token could not be marked unsafe: {quarantine_error}"
            )),
        };
    }
    clear_refresh_token_quarantine(&origin)
}

pub(super) fn load_refresh_token() -> Result<SavedRefreshToken, String> {
    let origin = server_origin()?;
    if origin.is_loopback {
        return Ok(SavedRefreshToken::None);
    }
    match refresh_token_quarantine_entry(&origin)?.get_password() {
        // A marker is deliberately enough. The original token must never be sent again after
        // an unknown refresh outcome, even if the old credential remains in the keyring.
        Ok(_) => return Ok(SavedRefreshToken::Ambiguous),
        Err(Error::NoEntry) => {}
        Err(error) => {
            return Err(format!("Windows Credential Manager read failed: {error}"));
        }
    }
    match refresh_token_entry(&origin)?.get_password() {
        Ok(token) if token.trim().is_empty() => {
            Err("Windows Credential Manager contains an empty saved refresh token".to_string())
        }
        Ok(token) => Ok(SavedRefreshToken::Scoped(token)),
        Err(Error::NoEntry) => match legacy_refresh_token_entry()?.get_password() {
            // Legacy credentials have no origin binding, so never send them to a configured URL.
            Ok(_) => Ok(SavedRefreshToken::LegacyCredential),
            Err(Error::NoEntry) => Ok(SavedRefreshToken::None),
            Err(error) => Err(format!("Windows Credential Manager read failed: {error}")),
        },
        Err(error) => Err(format!("Windows Credential Manager read failed: {error}")),
    }
}

pub(super) fn clear_refresh_token() -> Result<(), String> {
    let origin = server_origin()?;
    if origin.is_loopback {
        return Ok(());
    }
    delete_refresh_token_value(&origin)?;
    clear_refresh_token_quarantine(&origin)
}

pub(super) fn quarantine_refresh_token() -> Result<(), String> {
    let origin = server_origin()?;
    if origin.is_loopback {
        return Ok(());
    }
    quarantine_refresh_token_for_origin(&origin)
}

fn quarantine_refresh_token_for_origin(origin: &ServerOrigin) -> Result<(), String> {
    match refresh_token_quarantine_entry(origin)?.set_password(REFRESH_TOKEN_QUARANTINE_MARKER) {
        Ok(()) => Ok(()),
        Err(marker_error) => match delete_refresh_token_value(origin) {
            // Removing the old token is equally safe when the marker cannot be persisted.
            Ok(()) => Ok(()),
            Err(token_error) => Err(format!(
                "Windows Credential Manager could not mark the refresh token unsafe ({marker_error}) or remove it ({token_error})"
            )),
        },
    }
}

fn delete_refresh_token_value(origin: &ServerOrigin) -> Result<(), String> {
    match refresh_token_entry(origin)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("Windows Credential Manager delete failed: {error}")),
    }
}

fn clear_refresh_token_quarantine(origin: &ServerOrigin) -> Result<(), String> {
    match refresh_token_quarantine_entry(origin)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(format!(
            "Windows Credential Manager safety marker cleanup failed: {error}"
        )),
    }
}

fn refresh_token_entry(origin: &ServerOrigin) -> Result<Entry, String> {
    let user = refresh_token_user(origin);
    Entry::new(KEYRING_SERVICE, &user)
        .map_err(|error| format!("Windows Credential Manager is unavailable: {error}"))
}

fn refresh_token_quarantine_entry(origin: &ServerOrigin) -> Result<Entry, String> {
    let user = format!(
        "{KEYRING_REFRESH_TOKEN_QUARANTINE_USER}:{}",
        origin.base_url
    );
    Entry::new(KEYRING_SERVICE, &user)
        .map_err(|error| format!("Windows Credential Manager is unavailable: {error}"))
}

fn legacy_refresh_token_entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, KEYRING_REFRESH_TOKEN_USER)
        .map_err(|error| format!("Windows Credential Manager is unavailable: {error}"))
}

pub(super) fn refresh_token_user(origin: &ServerOrigin) -> String {
    format!("{KEYRING_REFRESH_TOKEN_USER}:{}", origin.base_url)
}

pub(super) fn login_start_message(
    local_dev_account_id: &str,
    remote_login_provider: RemoteLoginProvider,
) -> String {
    match server_origin() {
        Ok(origin) if origin.is_loopback => {
            format!(
                "Starting local session as {}...",
                local_dev_account_id.trim()
            )
        }
        Ok(_) => format!("Requesting {} login...", remote_login_provider.label()),
        Err(error) => format!("Cannot sign in: {error}"),
    }
}

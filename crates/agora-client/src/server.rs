use url::Url;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RemoteLoginProvider {
    Steam,
    Microsoft,
}

impl RemoteLoginProvider {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Steam => "Steam",
            Self::Microsoft => "Microsoft",
        }
    }

    pub(super) fn selection_value(self) -> &'static str {
        match self {
            Self::Steam => "steam",
            Self::Microsoft => "microsoft",
        }
    }

    pub(super) fn from_selection_value(value: &str) -> Option<Self> {
        match value {
            "steam" => Some(Self::Steam),
            "microsoft" => Some(Self::Microsoft),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ServerOrigin {
    pub(super) base_url: String,
    pub(super) websocket_url: String,
    pub(super) is_loopback: bool,
}

fn configured_server_url() -> String {
    std::env::var("AGORA_SERVER_URL")
        .unwrap_or_else(|_| {
            option_env!("AGORA_DEFAULT_SERVER_URL")
                .unwrap_or("http://localhost")
                .to_string()
        })
        .trim()
        .to_string()
}

pub(super) fn server_origin() -> Result<ServerOrigin, String> {
    parse_server_origin(&configured_server_url())
}

pub(super) fn parse_server_origin(value: &str) -> Result<ServerOrigin, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("AGORA_SERVER_URL must not be empty".to_string());
    }

    let url = Url::parse(value)
        .map_err(|_| "AGORA_SERVER_URL must be an absolute HTTP(S) origin".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err("AGORA_SERVER_URL must be an absolute HTTP(S) origin".to_string());
    }
    let authority = value
        .split_once("://")
        .map(|(_, value)| value)
        .unwrap_or_default()
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if authority.contains('@') || !url.username().is_empty() || url.password().is_some() {
        return Err("AGORA_SERVER_URL must not include credentials".to_string());
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(
            "AGORA_SERVER_URL must be an origin without a path, query, or fragment".to_string(),
        );
    }

    let host = url
        .host_str()
        .ok_or_else(|| "AGORA_SERVER_URL must include a host".to_string())?;
    let ip_host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    let is_loopback = host.eq_ignore_ascii_case("localhost")
        || ip_host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if url.scheme() == "http" && !is_loopback {
        return Err(
            "AGORA_SERVER_URL must use HTTPS unless it targets a loopback host".to_string(),
        );
    }

    let base_url = url.origin().ascii_serialization();
    let authority = base_url
        .strip_prefix(&format!("{}://", url.scheme()))
        .ok_or_else(|| "AGORA_SERVER_URL could not be normalized".to_string())?;
    let websocket_scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    let websocket_url = format!("{websocket_scheme}://{authority}/ws");
    Ok(ServerOrigin {
        base_url,
        websocket_url,
        is_loopback,
    })
}

pub(super) fn server_url() -> Result<String, String> {
    server_origin().map(|origin| origin.base_url)
}

pub(super) fn websocket_url() -> Result<String, String> {
    server_origin().map(|origin| origin.websocket_url)
}

pub(super) fn configured_local_dev_account_id() -> String {
    local_dev_account_id_or_default(std::env::var("AGORA_DEV_ACCOUNT_ID").ok().as_deref())
}

pub(super) fn local_dev_account_id_or_default(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("alice")
        .to_string()
}

pub(super) fn is_local_server_url() -> bool {
    server_origin().is_ok_and(|origin| origin.is_loopback)
}

pub(super) fn is_loopback_server_url(server_url: &str) -> bool {
    parse_server_origin(server_url).is_ok_and(|origin| origin.is_loopback)
}

pub(super) fn standalone_local_dev_window_enabled() -> bool {
    standalone_local_dev_window_enabled_for(
        std::env::var("AGORA_LOCAL_DEV_WINDOW").ok().as_deref(),
        &configured_server_url(),
    )
}

pub(super) fn standalone_local_dev_window_enabled_for(
    value: Option<&str>,
    server_url: &str,
) -> bool {
    is_loopback_server_url(server_url)
        && value.is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

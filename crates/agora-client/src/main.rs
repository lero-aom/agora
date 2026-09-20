#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("Agora currently supports Windows desktop.");
}

#[cfg(windows)]
mod game;
#[cfg(windows)]
mod update;

#[cfg(windows)]
fn main() {
    if update::run_helper_if_requested() {
        return;
    }
    if update::recover_interrupted_update() {
        return;
    }
    game::become_dpi_aware();
    let standalone_local_dev = standalone_local_dev_window_enabled();
    let (window, close_behaviour, background_color) = if standalone_local_dev {
        (
            dioxus::desktop::WindowBuilder::new()
                .with_title("Agora Local Dev")
                .with_visible(true)
                .with_focused(true)
                .with_decorations(true)
                .with_always_on_top(false)
                .with_resizable(true)
                .with_transparent(false)
                .with_inner_size(dioxus::desktop::LogicalSize::new(900.0, 560.0)),
            dioxus::desktop::WindowCloseBehaviour::LastWindowExitsApp,
            (10, 7, 18, 255),
        )
    } else {
        (
            dioxus::desktop::WindowBuilder::new()
                .with_title("Agora")
                .with_visible(false)
                .with_focused(false)
                .with_decorations(false)
                .with_always_on_top(true)
                .with_resizable(false)
                .with_transparent(true),
            dioxus::desktop::WindowCloseBehaviour::LastWindowHides,
            (10, 7, 18, 0),
        )
    };
    let mut config = dioxus::desktop::Config::new()
        .with_window(window)
        .with_close_behaviour(close_behaviour)
        .with_background_color(background_color)
        .with_menu(None);
    if let Some(icon) = app_window_icon() {
        config = config.with_icon(icon);
    }

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}

#[cfg(windows)]
fn app_window_icon() -> Option<dioxus::desktop::tao::window::Icon> {
    let (rgba, width, height) = load_logo_rgba()?;
    dioxus::desktop::tao::window::Icon::from_rgba(rgba, width, height).ok()
}

#[cfg(windows)]
fn app_tray_icon() -> Option<dioxus::desktop::trayicon::Icon> {
    let (rgba, width, height) = load_logo_rgba()?;
    dioxus::desktop::trayicon::Icon::from_rgba(rgba, width, height).ok()
}

#[cfg(windows)]
fn load_logo_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let icon = image::load_from_memory(include_bytes!("../../../assets/logo/logo.png"))
        .ok()?
        .resize(256, 256, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    let (width, height) = icon.dimensions();
    Some((icon.into_raw(), width, height))
}

#[cfg(windows)]
use agora_common::{
    ApiError, AuthSession, BlockListResponse, BlockUserRequest, BlockUserResponse, ChatMessage,
    ClientEvent, CreateDmThreadRequest, CreateDmThreadResponse, CreateReportRequest,
    CreateReportResponse, DevLoginRequest, DevLoginResponse, DmMessage, DmMessageHistoryResponse,
    DmRealtimeEvent, DmThreadListResponse, DmThreadSummary, FriendListResponse, FriendRequest,
    FriendshipResponse, FriendshipStatus, FriendshipSummary, LogoutRequest, LogoutResponse,
    MessageKind, MicrosoftLoginPollRequest, MicrosoftLoginPollResponse,
    MicrosoftLoginStartResponse, MicrosoftLoginStatus, PresenceCounts, PresenceState,
    RefreshRequest, RefreshResponse, RemoveFriendResponse, SendDmMessageRequest,
    SendDmMessageResponse, ServerEvent, SteamLoginPollRequest, SteamLoginPollResponse,
    SteamLoginStartResponse, SteamLoginStatus, UnblockUserResponse, UserSearchResponse,
    UserSummary, MAX_MESSAGE_LEN, PROTOCOL_VERSION,
};
#[cfg(windows)]
use dioxus::desktop::tao::platform::windows::WindowExtWindows;
#[cfg(windows)]
use dioxus::prelude::*;
#[cfg(windows)]
use futures_util::{SinkExt, StreamExt};
#[cfg(windows)]
use global_hotkey::hotkey::{Code as HotKeyCode, HotKey, Modifiers as HotKeyModifiers};
#[cfg(windows)]
use serde::{de::DeserializeOwned, Serialize};
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};
#[cfg(windows)]
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    oneshot,
};
#[cfg(windows)]
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::{header::AUTHORIZATION, HeaderValue},
        Message,
    },
};
#[cfg(windows)]
use url::Url;
#[cfg(windows)]
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
};
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_RETURN};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowLongPtrW, IsChild, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

#[cfg(windows)]
const KEYRING_SERVICE: &str = "agora";
#[cfg(windows)]
const KEYRING_REFRESH_TOKEN_USER: &str = "refresh-token";
#[cfg(windows)]
const KEYRING_REFRESH_TOKEN_QUARANTINE_USER: &str = "refresh-token-quarantine";
#[cfg(windows)]
const REFRESH_TOKEN_QUARANTINE_MARKER: &str = "refresh-outcome-unknown";
#[cfg(windows)]
const SESSION_REFRESH_SAFETY_SECONDS: u64 = 60;
#[cfg(windows)]
const SESSION_REFRESH_RETRY_INITIAL_SECONDS: u64 = 2;
#[cfg(windows)]
const SESSION_REFRESH_RETRY_MAX_SECONDS: u64 = 30;
#[cfg(windows)]
const CHAT_RECONNECT_SAFETY_SECONDS: u64 = 30;
#[cfg(windows)]
const GAME_WATCH_INTERVAL_MS: u64 = 75;
#[cfg(windows)]
const OVERLAY_PASSIVE_WIDTH: i32 = 420;
#[cfg(windows)]
const OVERLAY_PASSIVE_HEIGHT: i32 = 88;
#[cfg(windows)]
const OVERLAY_INTERACTIVE_WIDTH: i32 = 900;
#[cfg(windows)]
const OVERLAY_INTERACTIVE_HEIGHT: i32 = 560;
#[cfg(windows)]
const OVERLAY_MARGIN: i32 = 24;
#[cfg(windows)]
const OVERLAY_PASSIVE_TOP_OFFSET: i32 = 22;
#[cfg(windows)]
const TASKBAR_ANCHOR_POSITION: i32 = -32_000;
#[cfg(windows)]
const TASKBAR_ANCHOR_SIZE: i32 = 1;
#[cfg(windows)]
const DELETED_MESSAGE_CACHE_LIMIT: usize = 500;
#[cfg(windows)]
const USER_AUTOCOMPLETE_MIN_QUERY_CHARS: usize = 2;
#[cfg(windows)]
const USER_AUTOCOMPLETE_DEBOUNCE_MS: u64 = 300;

#[cfg(windows)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum AppTab {
    Global,
    DirectMessages,
    Friends,
    BlockReport,
}

#[cfg(windows)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum RemoteLoginProvider {
    Steam,
    Microsoft,
}

#[cfg(windows)]
impl RemoteLoginProvider {
    fn label(self) -> &'static str {
        match self {
            Self::Steam => "Steam",
            Self::Microsoft => "Microsoft",
        }
    }

    fn selection_value(self) -> &'static str {
        match self {
            Self::Steam => "steam",
            Self::Microsoft => "microsoft",
        }
    }

    fn from_selection_value(value: &str) -> Option<Self> {
        match value {
            "steam" => Some(Self::Steam),
            "microsoft" => Some(Self::Microsoft),
            _ => None,
        }
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellMode {
    Tray,
    OverlayPassive,
    OverlayInteractive,
}

#[cfg(windows)]
fn initial_shell_mode(standalone_local_dev: bool) -> ShellMode {
    if standalone_local_dev {
        ShellMode::OverlayInteractive
    } else {
        ShellMode::Tray
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverlayWindowCommand {
    None,
    ApplyPassive(game::GameWindow),
    ApplyInteractive(game::GameWindow),
    HideToTray,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GameWatchStep {
    detected: Option<game::GameWindow>,
    shell_mode: ShellMode,
    overlay_interactive: bool,
    command: OverlayWindowCommand,
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReportDraft {
    target: UserSummary,
    message_id: Option<uuid::Uuid>,
    message_kind: Option<MessageKind>,
    message_preview: Option<String>,
}

#[cfg(windows)]
#[derive(Default)]
struct FriendSections {
    accepted: Vec<FriendshipSummary>,
    incoming: Vec<FriendshipSummary>,
    outgoing: Vec<FriendshipSummary>,
    inactive: Vec<FriendshipSummary>,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct RelationshipSignals {
    friendships: Signal<Vec<FriendshipSummary>>,
    friendships_load_generation: Signal<u64>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    blocks_load_generation: Signal<u64>,
    block_status: Signal<String>,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct UserAutocompleteSignals {
    query: Signal<String>,
    results: Signal<Vec<UserSummary>>,
    selected: Signal<Option<UserSummary>>,
    request_generation: Signal<u64>,
    pending: Signal<bool>,
    expanded: Signal<bool>,
    active_index: Signal<Option<usize>>,
    status: Signal<String>,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct DirectMessageSignals {
    threads: Signal<Vec<DmThreadSummary>>,
    threads_load_generation: Signal<u64>,
    selected_thread_id: Signal<Option<uuid::Uuid>>,
    messages: Signal<Vec<DmMessage>>,
    next_before_message_id: Signal<Option<uuid::Uuid>>,
    history_load_generation: Signal<u64>,
    status: Signal<String>,
    composer_body: Signal<String>,
    search: UserAutocompleteSignals,
    threads_pending: Signal<bool>,
    history_pending: Signal<bool>,
    create_pending: Signal<bool>,
    send_pending: Signal<bool>,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct PresenceSelectionSignals {
    selected: Signal<PresenceState>,
    confirmed: Signal<PresenceState>,
    unconfirmed: Signal<Option<PresenceState>>,
    status: Signal<String>,
    pending: Signal<bool>,
    connected: Signal<bool>,
    request_generation: Signal<u64>,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct ChatSessionSignals {
    auth_session: Signal<Option<AuthSession>>,
    reauth_required: Signal<Option<String>>,
    login_status: Signal<String>,
    session_generation: Signal<u64>,
    chat_messages: Signal<Vec<ChatMessage>>,
    deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    presence_selection: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    chat_send_pending: Signal<bool>,
    updates: UpdateSignals,
    relationships: RelationshipSignals,
    direct_messages: DirectMessageSignals,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
enum UpdateUiStatus {
    Bootstrap,
    Checking {
        required: Option<String>,
    },
    UpToDate,
    Available {
        version: String,
        required: Option<String>,
    },
    Downloading {
        version: String,
        required: Option<String>,
    },
    Installing {
        version: String,
        required: Option<String>,
    },
    Failed {
        message: String,
        required: Option<String>,
    },
    SkippedLoopback,
    Recovery(String),
}

#[cfg(windows)]
impl UpdateUiStatus {
    fn message(&self) -> String {
        match self {
            Self::Bootstrap => {
                "This bootstrap build cannot check for automatic updates. Install a current release manually."
                    .to_string()
            }
            Self::Checking { required: Some(required) } => {
                format!("{required} Checking the signed release...")
            }
            Self::Checking { required: None } => "Checking the signed release...".to_string(),
            Self::UpToDate => "Agora is up to date.".to_string(),
            Self::Available { version, required: Some(required) } => {
                format!("{required} Agora {version} is ready to install.")
            }
            Self::Available { version, required: None } => {
                format!("Agora {version} is ready to install.")
            }
            Self::Downloading { version, required: Some(required) } => {
                format!("{required} Downloading and verifying Agora {version}...")
            }
            Self::Downloading { version, required: None } => {
                format!("Downloading and verifying Agora {version}...")
            }
            Self::Installing { version, required: Some(required) } => {
                format!("{required} Restarting to install Agora {version}...")
            }
            Self::Installing { version, required: None } => {
                format!("Restarting to install Agora {version}...")
            }
            Self::Failed { message, required: Some(required) } => {
                format!("{required} {message}")
            }
            Self::Failed { message, required: None } => message.clone(),
            Self::SkippedLoopback => "Automatic updates are disabled for a loopback server.".to_string(),
            Self::Recovery(message) => message.clone(),
        }
    }

    fn required_reason(&self) -> Option<String> {
        match self {
            Self::Checking { required }
            | Self::Available { required, .. }
            | Self::Downloading { required, .. }
            | Self::Installing { required, .. }
            | Self::Failed { required, .. } => required.clone(),
            _ => None,
        }
    }
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct UpdateSignals {
    status: Signal<UpdateUiStatus>,
    available: Signal<Option<update::AvailableUpdate>>,
    pending: Signal<bool>,
    generation: Signal<u64>,
}

#[cfg(windows)]
struct OutgoingChatEvent {
    event: ClientEvent,
    transport_result: oneshot::Sender<Result<(), String>>,
}

#[cfg(windows)]
enum SavedRefreshToken {
    Scoped(String),
    Ambiguous,
    LegacyCredential,
    None,
}

#[cfg(windows)]
enum RefreshSessionError {
    Invalid(String),
    Retryable(String),
    Ambiguous(String),
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
enum BearerAccess {
    Open,
    AwaitingCompatibility,
    Blocked(String),
}

#[cfg(windows)]
static BEARER_ACCESS: OnceLock<Mutex<BearerAccess>> = OnceLock::new();

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct ServerOrigin {
    base_url: String,
    websocket_url: String,
    is_loopback: bool,
}

#[cfg(windows)]
#[allow(non_snake_case)]
fn App() -> Element {
    let desktop = dioxus::desktop::use_window();
    let standalone_local_dev = standalone_local_dev_window_enabled();
    let _tray_icon = use_hook(|| {
        (!standalone_local_dev).then(|| {
            dioxus::desktop::trayicon::init_tray_icon(
                dioxus::desktop::trayicon::default_tray_icon(),
                app_tray_icon(),
            )
        })
    });
    let active_tab = use_signal(|| AppTab::Global);
    let shell_mode = use_signal(|| initial_shell_mode(standalone_local_dev));
    let game_window = use_signal(|| None::<game::GameWindow>);
    let overlay_interactive = use_signal(|| standalone_local_dev);
    let ctrl_enter_down = use_signal(|| false);
    let mut login_status = use_signal(|| "Checking saved session...".to_string());
    let mut auth_action_pending = use_signal(|| true);
    let mut auth_session = use_signal(|| None::<AuthSession>);
    let mut reauth_required = use_signal(|| None::<String>);
    let session_generation = use_signal(|| 0u64);
    let local_dev_account_id = use_signal(configured_local_dev_account_id);
    let remote_login_provider = use_signal(|| RemoteLoginProvider::Steam);
    let chat_messages = use_signal(Vec::<ChatMessage>::new);
    let deleted_message_ids = use_signal(Vec::<uuid::Uuid>::new);
    let chat_status = use_signal(|| "Sign in to connect to global chat".to_string());
    let presence_counts = use_signal(PresenceCounts::default);
    let presence_selection = PresenceSelectionSignals {
        selected: use_signal(|| PresenceState::Online),
        confirmed: use_signal(|| PresenceState::Online),
        unconfirmed: use_signal(|| None::<PresenceState>),
        status: use_signal(|| "Sign in to set availability".to_string()),
        pending: use_signal(|| false),
        connected: use_signal(|| false),
        request_generation: use_signal(|| 0u64),
    };
    let composer_body = use_signal(String::new);
    let chat_outbox = use_signal(|| None::<UnboundedSender<OutgoingChatEvent>>);
    let chat_send_pending = use_signal(|| false);
    let updates = UpdateSignals {
        status: use_signal(initial_update_status),
        available: use_signal(|| None::<update::AvailableUpdate>),
        pending: use_signal(|| false),
        generation: use_signal(|| 0u64),
    };
    let direct_messages = DirectMessageSignals {
        threads: use_signal(Vec::<DmThreadSummary>::new),
        threads_load_generation: use_signal(|| 0u64),
        selected_thread_id: use_signal(|| None::<uuid::Uuid>),
        messages: use_signal(Vec::<DmMessage>::new),
        next_before_message_id: use_signal(|| None::<uuid::Uuid>),
        history_load_generation: use_signal(|| 0u64),
        status: use_signal(|| "Sign in to load direct messages".to_string()),
        composer_body: use_signal(String::new),
        search: UserAutocompleteSignals {
            query: use_signal(String::new),
            results: use_signal(Vec::<UserSummary>::new),
            selected: use_signal(|| None::<UserSummary>),
            request_generation: use_signal(|| 0u64),
            pending: use_signal(|| false),
            expanded: use_signal(|| false),
            active_index: use_signal(|| None::<usize>),
            status: use_signal(String::new),
        },
        threads_pending: use_signal(|| false),
        history_pending: use_signal(|| false),
        create_pending: use_signal(|| false),
        send_pending: use_signal(|| false),
    };
    let friendships = use_signal(Vec::<FriendshipSummary>::new);
    let friendships_load_generation = use_signal(|| 0u64);
    let friend_search = UserAutocompleteSignals {
        query: use_signal(String::new),
        results: use_signal(Vec::<UserSummary>::new),
        selected: use_signal(|| None::<UserSummary>),
        request_generation: use_signal(|| 0u64),
        pending: use_signal(|| false),
        expanded: use_signal(|| false),
        active_index: use_signal(|| None::<usize>),
        status: use_signal(String::new),
    };
    let friends_status = use_signal(|| "Sign in to load friends".to_string());
    let blocked_users = use_signal(Vec::<UserSummary>::new);
    let blocks_load_generation = use_signal(|| 0u64);
    let block_search = UserAutocompleteSignals {
        query: use_signal(String::new),
        results: use_signal(Vec::<UserSummary>::new),
        selected: use_signal(|| None::<UserSummary>),
        request_generation: use_signal(|| 0u64),
        pending: use_signal(|| false),
        expanded: use_signal(|| false),
        active_index: use_signal(|| None::<usize>),
        status: use_signal(String::new),
    };
    let block_status = use_signal(|| "Sign in to manage blocks".to_string());
    let report_reason = use_signal(String::new);
    let report_details = use_signal(String::new);
    let report_draft = use_signal(|| None::<ReportDraft>);
    let report_status = use_signal(|| "Search for a user to report".to_string());

    use_hook(move || {
        check_for_updates(updates, None);
    });

    // This runs after the initial UI render, which is the point at which a helper-launched
    // replacement may safely discard its retained rollback executable.
    use_effect(move || {
        let _ = update::acknowledge_update_after_ui_startup();
    });

    use_hook(move || {
        spawn(async move {
            match load_refresh_token() {
                Ok(SavedRefreshToken::Scoped(refresh_token)) => {
                    let restore_generation = next_session_generation(session_generation);
                    let mut retry_attempt = 0u32;
                    loop {
                        if !session_generation_current(session_generation, restore_generation) {
                            return;
                        }

                        if retry_attempt == 0 {
                            login_status.set("Restoring saved session...".to_string());
                        }
                        match refresh_session(refresh_token.clone()).await {
                            Ok(session) => {
                                if !session_generation_current(
                                    session_generation,
                                    restore_generation,
                                ) {
                                    return;
                                }

                                next_session_generation(session_generation);
                                let display_name = session.user.display_name.clone();
                                if let Err(error) = store_refresh_token(&session.refresh_token) {
                                    login_status.set(format!(
                                        "Signed in as {display_name}, but token storage failed: {error}"
                                    ));
                                } else {
                                    login_status.set(format!("Signed in as {display_name}"));
                                }
                                auth_session.set(Some(session.clone()));
                                reauth_required.set(None);
                                let relationship_signals = RelationshipSignals {
                                    friendships,
                                    friendships_load_generation,
                                    friends_status,
                                    blocked_users,
                                    blocks_load_generation,
                                    block_status,
                                };
                                start_direct_message_session(
                                    &session,
                                    session_generation,
                                    direct_messages,
                                );
                                start_chat_session(
                                    &session,
                                    ChatSessionSignals {
                                        auth_session,
                                        reauth_required,
                                        login_status,
                                        session_generation,
                                        chat_messages,
                                        deleted_message_ids,
                                        chat_status,
                                        presence_counts,
                                        presence_selection,
                                        chat_outbox,
                                        chat_send_pending,
                                        updates,
                                        relationships: relationship_signals,
                                        direct_messages,
                                    },
                                );
                                start_session_refresh_loop(
                                    session,
                                    session_generation,
                                    auth_session,
                                    reauth_required,
                                    login_status,
                                    chat_messages,
                                    deleted_message_ids,
                                    chat_status,
                                    presence_counts,
                                    presence_selection,
                                    chat_outbox,
                                    chat_send_pending,
                                    updates,
                                    friendships,
                                    friendships_load_generation,
                                    friend_search,
                                    friends_status,
                                    blocked_users,
                                    blocks_load_generation,
                                    block_search,
                                    block_status,
                                    report_draft,
                                    direct_messages,
                                );
                                auth_action_pending.set(false);
                                return;
                            }
                            Err(RefreshSessionError::Invalid(error)) => {
                                if !session_generation_current(
                                    session_generation,
                                    restore_generation,
                                ) {
                                    return;
                                }

                                next_session_generation(session_generation);
                                let clear_error = clear_refresh_token().err();
                                stop_chat_session(
                                    chat_messages,
                                    deleted_message_ids,
                                    chat_status,
                                    presence_counts,
                                    presence_selection,
                                    chat_outbox,
                                    chat_send_pending,
                                );
                                reset_relationship_state(
                                    friendships,
                                    friend_search,
                                    friends_status,
                                    blocked_users,
                                    block_search,
                                    block_status,
                                    report_reason,
                                    report_details,
                                    report_draft,
                                    report_status,
                                );
                                reset_direct_message_state(direct_messages);
                                let status = match clear_error {
                                    Some(clear_error) => format!(
                                        "Saved session is no longer valid: {error}. Local credential cleanup failed: {clear_error}"
                                    ),
                                    None => format!("Saved session is no longer valid: {error}"),
                                };
                                login_status.set(status);
                                auth_action_pending.set(false);
                                return;
                            }
                            Err(RefreshSessionError::Retryable(error)) => {
                                retry_attempt = retry_attempt.saturating_add(1);
                                let delay = session_refresh_retry_delay(retry_attempt);
                                login_status.set(format!(
                                    "Saved session is temporarily unavailable: {error}. Retrying in {delay}s."
                                ));
                                // Let the user choose a fresh sign-in while the saved session retries.
                                auth_action_pending.set(false);
                                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                            }
                            Err(RefreshSessionError::Ambiguous(error)) => {
                                let quarantine_error = quarantine_refresh_token().err();
                                let status = match quarantine_error {
                                    Some(quarantine_error) => format!(
                                        "Saved session refresh may have succeeded, but its response was lost: {error}. The saved token was not retried, but could not be marked unsafe locally: {quarantine_error}. Sign in again before restarting Agora."
                                    ),
                                    None => format!(
                                        "Saved session refresh may have succeeded, but its response was lost: {error}. The saved token was not retried to protect your account. Sign in again."
                                    ),
                                };
                                login_status.set(status);
                                auth_action_pending.set(false);
                                return;
                            }
                        }
                    }
                }
                Ok(SavedRefreshToken::Ambiguous) => {
                    login_status.set(
                        "A previous session refresh had an unknown outcome, so its saved token was not retried. Sign in again."
                            .to_string(),
                    );
                    auth_action_pending.set(false);
                }
                Ok(SavedRefreshToken::LegacyCredential) => {
                    login_status.set(
                        "A legacy saved session was not sent because it is not scoped to this server. Sign in again."
                            .to_string(),
                    );
                    auth_action_pending.set(false);
                }
                Ok(SavedRefreshToken::None) => {
                    login_status.set("Not signed in".to_string());
                    auth_action_pending.set(false);
                }
                Err(error) => {
                    login_status.set(format!("Authentication unavailable: {error}"));
                    auth_action_pending.set(false);
                }
            }
        });
    });

    use_hook({
        let desktop = desktop.clone();
        move || {
            if standalone_local_dev {
                return;
            }
            let (foreground_tx, foreground_rx) = mpsc::unbounded_channel();
            game::start_foreground_event_watcher(foreground_tx);
            spawn(async move {
                watch_game_window(
                    desktop,
                    game_window,
                    shell_mode,
                    overlay_interactive,
                    foreground_rx,
                )
                .await;
            });
        }
    });

    let _shortcuts = use_hook({
        let desktop = desktop.clone();
        move || {
            register_global_shortcuts(
                desktop,
                active_tab,
                shell_mode,
                game_window,
                overlay_interactive,
                ctrl_enter_down,
                standalone_local_dev,
            )
        }
    });

    use_hook({
        let desktop = desktop.clone();
        move || {
            if standalone_local_dev {
                return;
            }
            hide_to_tray(&desktop);
            spawn(async move {
                watch_tray_events(desktop).await;
            });
        }
    });

    let active = *active_tab.read();
    let mode = *shell_mode.read();
    let counts = *presence_counts.read();
    let current_session = auth_session.read().clone();
    let signed_in = current_session.is_some();

    rsx! {
        style { {STYLE} }
        if mode == ShellMode::OverlayPassive {
            {passive_overlay_view(chat_messages, chat_status, counts, signed_in)}
        } else if mode == ShellMode::OverlayInteractive {
            {chat_mode_overlay_view(
                desktop.clone(),
                standalone_local_dev,
                active,
                active_tab,
                current_session.clone(),
                session_generation,
                reauth_required,
                local_dev_account_id,
                remote_login_provider,
                auth_action_pending,
                auth_session,
                login_status,
                chat_messages,
                deleted_message_ids,
                composer_body,
                chat_status,
                presence_counts,
                presence_selection,
                chat_outbox,
                chat_send_pending,
                updates,
                direct_messages,
                shell_mode,
                game_window,
                overlay_interactive,
                friendships,
                friendships_load_generation,
                friend_search,
                friends_status,
                blocked_users,
                blocks_load_generation,
                block_search,
                block_status,
                report_reason,
                report_details,
                report_draft,
                report_status,
            )}
        } else {
            div { class: "tray-idle" }
        }
    }
}

#[cfg(windows)]
async fn watch_game_window(
    desktop: dioxus::desktop::DesktopContext,
    mut game_window: Signal<Option<game::GameWindow>>,
    mut shell_mode: Signal<ShellMode>,
    mut overlay_interactive: Signal<bool>,
    mut foreground_events: UnboundedReceiver<()>,
) {
    loop {
        let raw_detected = game::detect_aom_window();
        let previous = *game_window.read();
        let desktop_foreground = desktop_is_foreground(&desktop);
        let step = game_watch_step(
            *shell_mode.read(),
            *overlay_interactive.read(),
            previous,
            raw_detected,
            desktop_foreground,
        );

        game_window.set(step.detected);
        if *overlay_interactive.read() != step.overlay_interactive {
            overlay_interactive.set(step.overlay_interactive);
        }
        if *shell_mode.read() != step.shell_mode {
            shell_mode.set(step.shell_mode);
        }
        apply_overlay_window_command(&desktop, step.command);

        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(GAME_WATCH_INTERVAL_MS)) => {}
            event = foreground_events.recv() => {
                if event.is_none() {
                    tokio::time::sleep(std::time::Duration::from_millis(GAME_WATCH_INTERVAL_MS)).await;
                }
            }
        }
    }
}

#[cfg(windows)]
fn game_watch_step(
    shell_mode: ShellMode,
    overlay_interactive: bool,
    previous: Option<game::GameWindow>,
    raw_detected: Option<game::GameWindow>,
    desktop_foreground: bool,
) -> GameWatchStep {
    let detected = visible_detected_game_window(raw_detected);
    match detected {
        Some(bounds) if overlay_interactive && (desktop_foreground || bounds.foreground) => {
            GameWatchStep {
                detected,
                shell_mode: ShellMode::OverlayInteractive,
                overlay_interactive: true,
                command: if shell_mode != ShellMode::OverlayInteractive
                    || overlay_bounds_changed(previous, bounds)
                {
                    OverlayWindowCommand::ApplyInteractive(bounds)
                } else {
                    OverlayWindowCommand::None
                },
            }
        }
        Some(_) if overlay_interactive => GameWatchStep {
            detected,
            shell_mode: ShellMode::Tray,
            overlay_interactive: true,
            command: hide_to_tray_command(shell_mode),
        },
        Some(bounds) if bounds.foreground => GameWatchStep {
            detected,
            shell_mode: ShellMode::OverlayPassive,
            overlay_interactive: false,
            command: if shell_mode != ShellMode::OverlayPassive
                || overlay_bounds_changed(previous, bounds)
            {
                OverlayWindowCommand::ApplyPassive(bounds)
            } else {
                OverlayWindowCommand::None
            },
        },
        Some(_) => GameWatchStep {
            detected,
            shell_mode: ShellMode::Tray,
            overlay_interactive: false,
            command: hide_to_tray_command(shell_mode),
        },
        None => GameWatchStep {
            detected: None,
            shell_mode: ShellMode::Tray,
            overlay_interactive: false,
            command: hide_to_tray_command(shell_mode),
        },
    }
}

#[cfg(windows)]
fn hide_to_tray_command(shell_mode: ShellMode) -> OverlayWindowCommand {
    if shell_mode == ShellMode::Tray {
        OverlayWindowCommand::None
    } else {
        OverlayWindowCommand::HideToTray
    }
}

#[cfg(windows)]
fn visible_detected_game_window(
    raw_detected: Option<game::GameWindow>,
) -> Option<game::GameWindow> {
    raw_detected.filter(|window| !window.minimized && window.width > 0 && window.height > 0)
}

#[cfg(windows)]
fn focused_visible_game_window(raw_detected: Option<game::GameWindow>) -> Option<game::GameWindow> {
    visible_detected_game_window(raw_detected).filter(|window| window.foreground)
}

#[cfg(windows)]
fn apply_overlay_window_command(
    desktop: &dioxus::desktop::DesktopContext,
    command: OverlayWindowCommand,
) {
    match command {
        OverlayWindowCommand::None => {}
        OverlayWindowCommand::ApplyPassive(bounds) => apply_passive_overlay_window(desktop, bounds),
        OverlayWindowCommand::ApplyInteractive(bounds) => {
            apply_interactive_overlay_window(desktop, bounds);
        }
        OverlayWindowCommand::HideToTray => hide_to_tray(desktop),
    }
}

#[cfg(windows)]
async fn watch_tray_events(desktop: dioxus::desktop::DesktopContext) {
    loop {
        while dioxus::desktop::trayicon::TrayIconEvent::receiver()
            .try_recv()
            .is_ok()
        {
            // Tray clicks are intentionally ignored; Agora is overlay-only.
            hide_to_tray(&desktop);
        }

        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }
}

#[cfg(windows)]
fn register_global_shortcuts(
    desktop: dioxus::desktop::DesktopContext,
    active_tab: Signal<AppTab>,
    shell_mode: Signal<ShellMode>,
    game_window: Signal<Option<game::GameWindow>>,
    overlay_interactive: Signal<bool>,
    mut ctrl_enter_down: Signal<bool>,
    standalone_local_dev: bool,
) -> Option<dioxus::desktop::ShortcutHandle> {
    if standalone_local_dev {
        return None;
    }
    let shortcut_desktop = desktop.clone();
    desktop
        .create_shortcut(
            HotKey::new(Some(HotKeyModifiers::CONTROL), HotKeyCode::Enter),
            move || {
                let keys_down = ctrl_enter_keys_down();
                if !keys_down {
                    ctrl_enter_down.set(false);
                    return;
                }
                if *ctrl_enter_down.read() {
                    return;
                }
                ctrl_enter_down.set(true);

                if *overlay_interactive.read() {
                    let aom_foreground =
                        visible_aom_window().is_some_and(|bounds| bounds.foreground);
                    if desktop_is_foreground(&shortcut_desktop) || aom_foreground {
                        close_interaction_or_hide(
                            &shortcut_desktop,
                            shell_mode,
                            game_window,
                            overlay_interactive,
                        );
                    }
                } else {
                    open_typing_mode_if_game_focused(
                        &shortcut_desktop,
                        active_tab,
                        shell_mode,
                        game_window,
                        overlay_interactive,
                    );
                }
            },
        )
        .ok()
}

#[cfg(windows)]
fn ctrl_enter_keys_down() -> bool {
    unsafe { GetAsyncKeyState(VK_CONTROL as i32) < 0 && GetAsyncKeyState(VK_RETURN as i32) < 0 }
}

#[cfg(windows)]
fn desktop_is_foreground(desktop: &dioxus::desktop::DesktopContext) -> bool {
    let hwnd = desktop.window.hwnd() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let foreground = GetForegroundWindow();
        !foreground.is_null() && (foreground == hwnd || IsChild(hwnd, foreground) != 0)
    }
}

#[cfg(windows)]
fn open_typing_mode_if_game_focused(
    desktop: &dioxus::desktop::DesktopContext,
    mut active_tab: Signal<AppTab>,
    mut shell_mode: Signal<ShellMode>,
    mut game_window: Signal<Option<game::GameWindow>>,
    mut overlay_interactive: Signal<bool>,
) {
    let raw_detected = game::detect_aom_window();
    let detected = visible_detected_game_window(raw_detected);
    game_window.set(detected);

    if let Some(bounds) = focused_visible_game_window(raw_detected) {
        overlay_interactive.set(true);
        active_tab.set(AppTab::Global);
        shell_mode.set(ShellMode::OverlayInteractive);
        apply_interactive_overlay_window(desktop, bounds);
    }
}

#[cfg(windows)]
fn visible_aom_window() -> Option<game::GameWindow> {
    visible_detected_game_window(game::detect_aom_window())
}

#[cfg(windows)]
fn focus_or_get_foreground_aom_window(
    detected: Option<game::GameWindow>,
) -> Option<game::GameWindow> {
    let bounds = detected?;
    if bounds.foreground {
        return Some(bounds);
    }

    if !game::focus_aom_window() {
        return None;
    }

    visible_aom_window().filter(|bounds| bounds.foreground)
}

#[cfg(windows)]
fn close_interaction_or_hide(
    desktop: &dioxus::desktop::DesktopContext,
    mut shell_mode: Signal<ShellMode>,
    mut game_window: Signal<Option<game::GameWindow>>,
    mut overlay_interactive: Signal<bool>,
) {
    overlay_interactive.set(false);
    let detected = visible_aom_window();
    game_window.set(detected);
    if let Some(bounds) = focus_or_get_foreground_aom_window(detected) {
        shell_mode.set(ShellMode::OverlayPassive);
        apply_passive_overlay_window(desktop, bounds);
    } else {
        shell_mode.set(ShellMode::Tray);
        hide_to_tray(desktop);
    }
}

#[cfg(windows)]
fn overlay_bounds_changed(previous: Option<game::GameWindow>, current: game::GameWindow) -> bool {
    !matches!(
        previous,
        Some(previous)
            if previous.pid == current.pid
                && previous.x == current.x
                && previous.y == current.y
                && previous.width == current.width
                && previous.height == current.height
                && previous.minimized == current.minimized
    )
}

#[cfg(windows)]
fn apply_passive_overlay_window(
    desktop: &dioxus::desktop::DesktopContext,
    bounds: game::GameWindow,
) {
    desktop.window.set_enable(true);
    let _ = desktop.set_ignore_cursor_events(false);
    desktop.set_decorations(false);
    desktop.set_resizable(true);
    desktop.set_always_on_top(true);
    set_overlay_background(desktop);
    set_window_rect(
        desktop,
        bounds,
        OVERLAY_PASSIVE_WIDTH,
        OVERLAY_PASSIVE_HEIGHT,
    );
    desktop.set_resizable(false);
    apply_passive_native_window_style(desktop);
    set_dwm_border_color(desktop, DWMWA_COLOR_NONE);
    let _ = desktop.set_ignore_cursor_events(true);
    desktop.set_visible(true);
}

#[cfg(windows)]
fn apply_interactive_overlay_window(
    desktop: &dioxus::desktop::DesktopContext,
    bounds: game::GameWindow,
) {
    apply_typing_native_window_style(desktop);
    set_dwm_border_color(desktop, DWMWA_COLOR_NONE);
    let _ = desktop.set_ignore_cursor_events(false);
    desktop.set_decorations(false);
    desktop.set_resizable(false);
    desktop.set_always_on_top(true);
    set_overlay_background(desktop);
    set_window_rect(
        desktop,
        bounds,
        OVERLAY_INTERACTIVE_WIDTH,
        OVERLAY_INTERACTIVE_HEIGHT,
    );
    desktop.set_visible(true);
    if !desktop_is_foreground(desktop) {
        desktop.set_focus();
    }
    set_dwm_border_color(desktop, DWMWA_COLOR_NONE);
}

#[cfg(windows)]
fn hide_to_tray(desktop: &dioxus::desktop::DesktopContext) {
    desktop.set_visible(false);
    let _ = desktop.set_ignore_cursor_events(true);
    desktop.set_decorations(false);
    desktop.set_resizable(false);
    desktop.set_always_on_top(false);
    set_taskbar_anchor_background(desktop);
    set_taskbar_anchor_rect(desktop);
    apply_tray_native_window_style(desktop);
    desktop.set_visible(true);
}

#[cfg(windows)]
fn set_overlay_background(desktop: &dioxus::desktop::DesktopContext) {
    desktop.window.set_background_color(Some((7, 9, 9, 255)));
}

#[cfg(windows)]
fn set_taskbar_anchor_background(desktop: &dioxus::desktop::DesktopContext) {
    desktop.window.set_background_color(Some((7, 9, 9, 0)));
}

#[cfg(windows)]
fn apply_passive_native_window_style(desktop: &dioxus::desktop::DesktopContext) {
    desktop.window.set_enable(false);
    set_overlay_ex_style(
        desktop,
        WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_APPWINDOW,
        WS_EX_TOOLWINDOW,
    );
    let _ = desktop.window.set_skip_taskbar(false);
}

#[cfg(windows)]
fn apply_typing_native_window_style(desktop: &dioxus::desktop::DesktopContext) {
    desktop.window.set_enable(true);
    set_overlay_ex_style(
        desktop,
        WS_EX_APPWINDOW,
        WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
    );
    let _ = desktop.window.set_skip_taskbar(false);
}

#[cfg(windows)]
fn apply_tray_native_window_style(desktop: &dioxus::desktop::DesktopContext) {
    desktop.window.set_enable(true);
    set_overlay_ex_style(
        desktop,
        WS_EX_NOACTIVATE | WS_EX_APPWINDOW,
        WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
    );
    let _ = desktop.window.set_skip_taskbar(false);
}

#[cfg(windows)]
fn set_overlay_ex_style(desktop: &dioxus::desktop::DesktopContext, add: u32, remove: u32) {
    let hwnd = desktop.window.hwnd() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let updated = (style | add) & !remove;
        if updated != style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, updated as isize);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            );
        }
    }
}

#[cfg(windows)]
fn set_dwm_border_color(desktop: &dioxus::desktop::DesktopContext, color: u32) {
    let hwnd = desktop.window.hwnd() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            &color as *const u32 as *const std::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

#[cfg(windows)]
fn set_window_rect(
    desktop: &dioxus::desktop::DesktopContext,
    bounds: game::GameWindow,
    width: i32,
    height: i32,
) {
    let (x, y) = overlay_position(bounds, width, height);
    desktop.set_inner_size(dioxus::desktop::tao::dpi::PhysicalSize::new(
        width.max(1) as u32,
        height.max(1) as u32,
    ));
    desktop.set_outer_position(dioxus::desktop::tao::dpi::PhysicalPosition::new(x, y));
    let hwnd = desktop.window.hwnd() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            width.max(1),
            height.max(1),
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

#[cfg(windows)]
fn set_taskbar_anchor_rect(desktop: &dioxus::desktop::DesktopContext) {
    desktop.set_inner_size(dioxus::desktop::tao::dpi::PhysicalSize::new(
        TASKBAR_ANCHOR_SIZE as u32,
        TASKBAR_ANCHOR_SIZE as u32,
    ));
    desktop.set_outer_position(dioxus::desktop::tao::dpi::PhysicalPosition::new(
        TASKBAR_ANCHOR_POSITION,
        TASKBAR_ANCHOR_POSITION,
    ));

    let hwnd = desktop.window.hwnd() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            TASKBAR_ANCHOR_POSITION,
            TASKBAR_ANCHOR_POSITION,
            TASKBAR_ANCHOR_SIZE,
            TASKBAR_ANCHOR_SIZE,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

#[cfg(windows)]
fn overlay_position(bounds: game::GameWindow, width: i32, height: i32) -> (i32, i32) {
    let centered_x = bounds.x + ((bounds.width - width) / 2);
    let min_x = bounds.x + OVERLAY_MARGIN;
    let max_x = bounds.x + bounds.width - width - OVERLAY_MARGIN;
    let target_x = if width == OVERLAY_PASSIVE_WIDTH && height == OVERLAY_PASSIVE_HEIGHT {
        centered_x + ((max_x - centered_x) / 2)
    } else {
        centered_x
    };
    let x = if min_x <= max_x {
        target_x.clamp(min_x, max_x)
    } else {
        target_x
    };
    let y = if width == OVERLAY_PASSIVE_WIDTH && height == OVERLAY_PASSIVE_HEIGHT {
        bounds.y + OVERLAY_PASSIVE_TOP_OFFSET
    } else {
        bounds.y + OVERLAY_MARGIN
    };
    (x, y)
}

#[cfg(all(test, windows))]
mod overlay_state_tests {
    use super::*;

    fn game_window(foreground: bool) -> game::GameWindow {
        game::GameWindow {
            pid: 42,
            x: -2560,
            y: 0,
            width: 2560,
            height: 1440,
            foreground,
            minimized: false,
        }
    }

    fn hidden_step(detected: Option<game::GameWindow>, overlay_interactive: bool) -> GameWatchStep {
        GameWatchStep {
            detected,
            shell_mode: ShellMode::Tray,
            overlay_interactive,
            command: OverlayWindowCommand::HideToTray,
        }
    }

    #[test]
    fn game_absence_hides_to_tray_and_clears_interaction_mode() {
        let step = game_watch_step(
            ShellMode::OverlayInteractive,
            true,
            Some(game_window(true)),
            None,
            true,
        );

        assert_eq!(step, hidden_step(None, false));
    }

    #[test]
    fn minimized_or_invalid_game_bounds_are_treated_as_absent() {
        let visible = game_window(true);
        let minimized = game::GameWindow {
            minimized: true,
            ..visible
        };
        let zero_width = game::GameWindow {
            width: 0,
            ..visible
        };
        let zero_height = game::GameWindow {
            height: 0,
            ..visible
        };

        for raw_detected in [Some(minimized), Some(zero_width), Some(zero_height)] {
            let step = game_watch_step(
                ShellMode::OverlayPassive,
                false,
                Some(visible),
                raw_detected,
                false,
            );

            assert_eq!(step, hidden_step(None, false));
        }
    }

    #[test]
    fn visible_but_unfocused_game_hides_to_tray() {
        let unfocused = game_window(false);

        let step = game_watch_step(
            ShellMode::OverlayPassive,
            false,
            Some(unfocused),
            Some(unfocused),
            false,
        );

        assert_eq!(step, hidden_step(Some(unfocused), false));
    }

    #[test]
    fn repeated_unfocused_game_does_not_reapply_tray_hide() {
        let unfocused = game_window(false);

        let step = game_watch_step(
            ShellMode::Tray,
            false,
            Some(unfocused),
            Some(unfocused),
            false,
        );

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(unfocused),
                shell_mode: ShellMode::Tray,
                overlay_interactive: false,
                command: OverlayWindowCommand::None,
            }
        );
    }

    #[test]
    fn repeated_game_absence_does_not_reapply_tray_hide() {
        let step = game_watch_step(ShellMode::Tray, false, None, None, false);

        assert_eq!(
            step,
            GameWatchStep {
                detected: None,
                shell_mode: ShellMode::Tray,
                overlay_interactive: false,
                command: OverlayWindowCommand::None,
            }
        );
    }

    #[test]
    fn focused_game_opens_passive_overlay_when_not_typing() {
        let focused = game_window(true);

        let step = game_watch_step(ShellMode::Tray, false, None, Some(focused), false);

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(focused),
                shell_mode: ShellMode::OverlayPassive,
                overlay_interactive: false,
                command: OverlayWindowCommand::ApplyPassive(focused),
            }
        );
    }

    #[test]
    fn unchanged_focused_game_keeps_existing_passive_overlay() {
        let focused = game_window(true);

        let step = game_watch_step(
            ShellMode::OverlayPassive,
            false,
            Some(focused),
            Some(focused),
            false,
        );

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(focused),
                shell_mode: ShellMode::OverlayPassive,
                overlay_interactive: false,
                command: OverlayWindowCommand::None,
            }
        );
    }

    #[test]
    fn focused_game_bounds_change_reapplies_passive_overlay() {
        let previous = game_window(true);
        let resized = game::GameWindow {
            width: 1920,
            height: 1080,
            ..previous
        };

        let step = game_watch_step(
            ShellMode::OverlayPassive,
            false,
            Some(previous),
            Some(resized),
            false,
        );

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(resized),
                shell_mode: ShellMode::OverlayPassive,
                overlay_interactive: false,
                command: OverlayWindowCommand::ApplyPassive(resized),
            }
        );
    }

    #[test]
    fn typing_mode_opens_interactive_overlay_when_game_is_focused() {
        let focused = game_window(true);

        let step = game_watch_step(ShellMode::Tray, true, None, Some(focused), false);

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(focused),
                shell_mode: ShellMode::OverlayInteractive,
                overlay_interactive: true,
                command: OverlayWindowCommand::ApplyInteractive(focused),
            }
        );
    }

    #[test]
    fn typing_mode_stays_visible_when_overlay_itself_has_focus() {
        let unfocused_game = game_window(false);

        let step = game_watch_step(ShellMode::Tray, true, None, Some(unfocused_game), true);

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(unfocused_game),
                shell_mode: ShellMode::OverlayInteractive,
                overlay_interactive: true,
                command: OverlayWindowCommand::ApplyInteractive(unfocused_game),
            }
        );
    }

    #[test]
    fn typing_mode_hides_when_neither_game_nor_overlay_has_focus() {
        let unfocused_game = game_window(false);

        let step = game_watch_step(
            ShellMode::OverlayInteractive,
            true,
            Some(unfocused_game),
            Some(unfocused_game),
            false,
        );

        assert_eq!(step, hidden_step(Some(unfocused_game), true));
    }

    #[test]
    fn repeated_hidden_typing_mode_does_not_reapply_tray_hide() {
        let unfocused_game = game_window(false);

        let step = game_watch_step(
            ShellMode::Tray,
            true,
            Some(unfocused_game),
            Some(unfocused_game),
            false,
        );

        assert_eq!(
            step,
            GameWatchStep {
                detected: Some(unfocused_game),
                shell_mode: ShellMode::Tray,
                overlay_interactive: true,
                command: OverlayWindowCommand::None,
            }
        );
    }

    #[test]
    fn hotkey_open_requires_a_focused_visible_game_window() {
        let focused = game_window(true);
        let unfocused = game_window(false);
        let minimized = game::GameWindow {
            minimized: true,
            ..focused
        };
        let zero_width = game::GameWindow {
            width: 0,
            ..focused
        };
        let zero_height = game::GameWindow {
            height: 0,
            ..focused
        };

        assert_eq!(focused_visible_game_window(Some(focused)), Some(focused));
        for raw_detected in [
            None,
            Some(unfocused),
            Some(minimized),
            Some(zero_width),
            Some(zero_height),
        ] {
            assert_eq!(focused_visible_game_window(raw_detected), None);
        }
    }
}

#[cfg(windows)]
fn refresh_relationship_state(
    session: &AuthSession,
    session_generation: Signal<u64>,
    signals: RelationshipSignals,
) {
    load_friends(
        session.clone(),
        session_generation,
        signals.friendships_load_generation,
        signals.friendships,
        signals.friends_status,
    );
    load_blocks(
        session.clone(),
        session_generation,
        signals.blocks_load_generation,
        signals.blocked_users,
        signals.block_status,
    );
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn start_session_refresh_loop(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut auth_session: Signal<Option<AuthSession>>,
    mut reauth_required: Signal<Option<String>>,
    mut login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    presence_selection: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    chat_send_pending: Signal<bool>,
    updates: UpdateSignals,
    mut friendships: Signal<Vec<FriendshipSummary>>,
    friendships_load_generation: Signal<u64>,
    friend_search: UserAutocompleteSignals,
    mut friends_status: Signal<String>,
    mut blocked_users: Signal<Vec<UserSummary>>,
    blocks_load_generation: Signal<u64>,
    block_search: UserAutocompleteSignals,
    mut block_status: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    direct_messages: DirectMessageSignals,
) {
    spawn(async move {
        let mut session = session;
        let mut generation = *session_generation.read();
        let mut retry_attempt = 0u32;
        let mut next_delay = session_refresh_delay(session.expires_in_seconds);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(next_delay)).await;

            if !current_session_matches(
                auth_session,
                session_generation,
                generation,
                &session.refresh_token,
            ) {
                return;
            }

            match refresh_session(session.refresh_token.clone()).await {
                Ok(refreshed) => {
                    if !current_session_matches(
                        auth_session,
                        session_generation,
                        generation,
                        &session.refresh_token,
                    ) {
                        return;
                    }

                    generation = next_session_generation(session_generation);
                    clear_direct_message_pending(direct_messages);
                    invalidate_user_autocomplete(friend_search);
                    invalidate_user_autocomplete(block_search);
                    retry_attempt = 0;
                    let display_name = refreshed.user.display_name.clone();
                    match store_refresh_token(&refreshed.refresh_token) {
                        Ok(()) => login_status.set(format!("Signed in as {display_name}")),
                        Err(error) => login_status.set(format!(
                            "Session refreshed for {display_name}, but token storage failed: {error}"
                        )),
                    }
                    auth_session.set(Some(refreshed.clone()));
                    reauth_required.set(None);
                    let relationship_signals = RelationshipSignals {
                        friendships,
                        friendships_load_generation,
                        friends_status,
                        blocked_users,
                        blocks_load_generation,
                        block_status,
                    };
                    start_chat_session(
                        &refreshed,
                        ChatSessionSignals {
                            auth_session,
                            reauth_required,
                            login_status,
                            session_generation,
                            chat_messages,
                            deleted_message_ids,
                            chat_status,
                            presence_counts,
                            presence_selection,
                            chat_outbox,
                            chat_send_pending,
                            updates,
                            relationships: relationship_signals,
                            direct_messages,
                        },
                    );
                    session = refreshed;
                    next_delay = session_refresh_delay(session.expires_in_seconds);
                }
                Err(RefreshSessionError::Invalid(error)) => {
                    if !current_session_matches(
                        auth_session,
                        session_generation,
                        generation,
                        &session.refresh_token,
                    ) {
                        return;
                    }

                    next_session_generation(session_generation);
                    let clear_error = clear_refresh_token().err();
                    auth_session.set(None);
                    stop_chat_session(
                        chat_messages,
                        deleted_message_ids,
                        chat_status,
                        presence_counts,
                        presence_selection,
                        chat_outbox,
                        chat_send_pending,
                    );
                    friendships.set(Vec::new());
                    reset_user_autocomplete(friend_search);
                    friends_status.set("Sign in to load friends".to_string());
                    blocked_users.set(Vec::new());
                    reset_user_autocomplete(block_search);
                    block_status.set("Sign in to manage blocks".to_string());
                    report_draft.set(None);
                    reset_direct_message_state(direct_messages);
                    let status = match clear_error {
                        Some(clear_error) => format!(
                            "Session is no longer valid: {error}. Local credential cleanup failed: {clear_error}"
                        ),
                        None => format!("Session is no longer valid: {error}"),
                    };
                    login_status.set(status);
                    return;
                }
                Err(RefreshSessionError::Retryable(error)) => {
                    if !current_session_matches(
                        auth_session,
                        session_generation,
                        generation,
                        &session.refresh_token,
                    ) {
                        return;
                    }

                    retry_attempt = retry_attempt.saturating_add(1);
                    next_delay = session_refresh_retry_delay(retry_attempt);
                    login_status.set(format!(
                        "Session refresh is temporarily unavailable: {error}. Retrying in {next_delay}s."
                    ));
                }
                Err(RefreshSessionError::Ambiguous(error)) => {
                    if !current_session_matches(
                        auth_session,
                        session_generation,
                        generation,
                        &session.refresh_token,
                    ) {
                        return;
                    }

                    let quarantine_error = quarantine_refresh_token().err();
                    let status = match quarantine_error {
                        Some(quarantine_error) => format!(
                            "Session refresh may have succeeded, but its response was lost: {error}. The saved token was not retried, but could not be marked unsafe locally: {quarantine_error}. Sign in again before restarting Agora."
                        ),
                        None => format!(
                            "Session refresh may have succeeded, but its response was lost: {error}. The current session can continue until it expires; sign in again before restarting Agora."
                        ),
                    };
                    reauth_required.set(Some(status.clone()));
                    login_status.set(status);
                    return;
                }
            }
        }
    });
}

#[cfg(windows)]
fn session_refresh_delay(expires_in_seconds: u64) -> u64 {
    expires_in_seconds
        .saturating_sub(SESSION_REFRESH_SAFETY_SECONDS)
        .max(1)
}

#[cfg(windows)]
fn session_refresh_retry_delay(attempt: u32) -> u64 {
    let exponent = attempt.saturating_sub(1).min(4);
    SESSION_REFRESH_RETRY_INITIAL_SECONDS
        .saturating_mul(1u64 << exponent)
        .min(SESSION_REFRESH_RETRY_MAX_SECONDS)
}

#[cfg(windows)]
fn next_session_generation(mut session_generation: Signal<u64>) -> u64 {
    let current = *session_generation.read();
    let next = current.wrapping_add(1);
    session_generation.set(next);
    next
}

#[cfg(windows)]
fn next_relationship_load_generation(mut load_generation: Signal<u64>) -> u64 {
    let current = *load_generation.read();
    let next = current.wrapping_add(1);
    load_generation.set(next);
    next
}

#[cfg(windows)]
fn session_generation_current(session_generation: Signal<u64>, generation: u64) -> bool {
    *session_generation.read() == generation
}

#[cfg(windows)]
fn relationship_load_generation_current(load_generation: Signal<u64>, generation: u64) -> bool {
    *load_generation.read() == generation
}

#[cfg(windows)]
fn next_direct_message_load_generation(mut load_generation: Signal<u64>) -> u64 {
    let current = *load_generation.read();
    let next = current.wrapping_add(1);
    load_generation.set(next);
    next
}

#[cfg(windows)]
fn direct_message_load_generation_current(load_generation: Signal<u64>, generation: u64) -> bool {
    *load_generation.read() == generation
}

#[cfg(windows)]
fn current_session_matches(
    auth_session: Signal<Option<AuthSession>>,
    session_generation: Signal<u64>,
    generation: u64,
    refresh_token: &str,
) -> bool {
    session_generation_current(session_generation, generation)
        && matches!(
            auth_session.read().as_ref(),
            Some(session) if session.refresh_token == refresh_token
        )
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn sign_in_session(
    mut auth_action_pending: Signal<bool>,
    mut auth_session: Signal<Option<AuthSession>>,
    session_generation: Signal<u64>,
    mut reauth_required: Signal<Option<String>>,
    local_dev_account_id: String,
    remote_login_provider: RemoteLoginProvider,
    mut login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    mut chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    presence_selection: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    chat_send_pending: Signal<bool>,
    updates: UpdateSignals,
    friendships: Signal<Vec<FriendshipSummary>>,
    friendships_load_generation: Signal<u64>,
    friend_search: UserAutocompleteSignals,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    blocks_load_generation: Signal<u64>,
    block_search: UserAutocompleteSignals,
    block_status: Signal<String>,
    report_draft: Signal<Option<ReportDraft>>,
    direct_messages: DirectMessageSignals,
) {
    if *auth_action_pending.read() {
        return;
    }
    auth_action_pending.set(true);
    next_session_generation(session_generation);
    clear_direct_message_pending(direct_messages);
    reset_user_autocomplete(friend_search);
    reset_user_autocomplete(block_search);

    spawn(async move {
        let start_message = login_start_message(&local_dev_account_id, remote_login_provider);
        login_status.set(start_message.clone());
        chat_status.set(start_message);
        match complete_login(&local_dev_account_id, remote_login_provider).await {
            Ok(session) => {
                next_session_generation(session_generation);
                let display_name = session.user.display_name.clone();
                match store_refresh_token(&session.refresh_token) {
                    Ok(()) => login_status.set(format!("Signed in as {display_name}")),
                    Err(error) => login_status.set(format!(
                        "Signed in as {display_name}, but token storage failed: {error}"
                    )),
                }
                auth_session.set(Some(session.clone()));
                reauth_required.set(None);
                let relationship_signals = RelationshipSignals {
                    friendships,
                    friendships_load_generation,
                    friends_status,
                    blocked_users,
                    blocks_load_generation,
                    block_status,
                };
                start_direct_message_session(&session, session_generation, direct_messages);
                start_chat_session(
                    &session,
                    ChatSessionSignals {
                        auth_session,
                        reauth_required,
                        login_status,
                        session_generation,
                        chat_messages,
                        deleted_message_ids,
                        chat_status,
                        presence_counts,
                        presence_selection,
                        chat_outbox,
                        chat_send_pending,
                        updates,
                        relationships: relationship_signals,
                        direct_messages,
                    },
                );
                start_session_refresh_loop(
                    session,
                    session_generation,
                    auth_session,
                    reauth_required,
                    login_status,
                    chat_messages,
                    deleted_message_ids,
                    chat_status,
                    presence_counts,
                    presence_selection,
                    chat_outbox,
                    chat_send_pending,
                    updates,
                    friendships,
                    friendships_load_generation,
                    friend_search,
                    friends_status,
                    blocked_users,
                    blocks_load_generation,
                    block_search,
                    block_status,
                    report_draft,
                    direct_messages,
                );
            }
            Err(error) => {
                login_status.set(error.clone());
                chat_status.set(error);
            }
        }
        auth_action_pending.set(false);
    });
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn sign_out_session(
    session: AuthSession,
    mut auth_action_pending: Signal<bool>,
    mut auth_session: Signal<Option<AuthSession>>,
    session_generation: Signal<u64>,
    mut reauth_required: Signal<Option<String>>,
    mut login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    presence_selection: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    chat_send_pending: Signal<bool>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friend_search: UserAutocompleteSignals,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_search: UserAutocompleteSignals,
    block_status: Signal<String>,
    report_reason: Signal<String>,
    report_details: Signal<String>,
    report_draft: Signal<Option<ReportDraft>>,
    report_status: Signal<String>,
    direct_messages: DirectMessageSignals,
) {
    if *auth_action_pending.read() {
        return;
    }
    auth_action_pending.set(true);
    next_session_generation(session_generation);
    clear_direct_message_pending(direct_messages);
    reauth_required.set(None);

    let refresh_token = session.refresh_token;
    spawn(async move {
        login_status.set("Signing out...".to_string());
        auth_session.set(None);
        stop_chat_session(
            chat_messages,
            deleted_message_ids,
            chat_status,
            presence_counts,
            presence_selection,
            chat_outbox,
            chat_send_pending,
        );
        reset_relationship_state(
            friendships,
            friend_search,
            friends_status,
            blocked_users,
            block_search,
            block_status,
            report_reason,
            report_details,
            report_draft,
            report_status,
        );
        reset_direct_message_state(direct_messages);

        let clear_result = clear_refresh_token();
        let logout_result = logout_session(refresh_token).await;
        match (logout_result, clear_result) {
            (Ok(true), Ok(())) => {
                login_status.set("Signed out. Server session revoked.".to_string())
            }
            (Ok(false), Ok(())) => login_status
                .set("Signed out locally. Server session was already inactive.".to_string()),
            (Err(error), Ok(())) => {
                login_status.set(format!("Signed out locally. Server logout failed: {error}"))
            }
            (Ok(true), Err(error)) => login_status.set(format!(
                "Server session revoked. Local credential cleanup failed: {error}"
            )),
            (Ok(false), Err(error)) => login_status.set(format!(
                "Server session was already inactive. Local credential cleanup failed: {error}"
            )),
            (Err(logout_error), Err(clear_error)) => login_status.set(format!(
                "Sign-out had issues. Server: {logout_error}. Local: {clear_error}"
            )),
        }
        auth_action_pending.set(false);
    });
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn reset_relationship_state(
    mut friendships: Signal<Vec<FriendshipSummary>>,
    friend_search: UserAutocompleteSignals,
    mut friends_status: Signal<String>,
    mut blocked_users: Signal<Vec<UserSummary>>,
    block_search: UserAutocompleteSignals,
    mut block_status: Signal<String>,
    mut report_reason: Signal<String>,
    mut report_details: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_status: Signal<String>,
) {
    friendships.set(Vec::new());
    reset_user_autocomplete(friend_search);
    friends_status.set("Sign in to load friends".to_string());
    blocked_users.set(Vec::new());
    reset_user_autocomplete(block_search);
    block_status.set("Sign in to manage blocks".to_string());
    report_reason.set(String::new());
    report_details.set(String::new());
    report_draft.set(None);
    report_status.set("Search for a user to report".to_string());
}

#[cfg(windows)]
fn reset_direct_message_state(mut signals: DirectMessageSignals) {
    signals.threads.set(Vec::new());
    signals.selected_thread_id.set(None);
    signals.messages.set(Vec::new());
    signals.next_before_message_id.set(None);
    signals
        .status
        .set("Sign in to load direct messages".to_string());
    signals.composer_body.set(String::new());
    reset_user_autocomplete(signals.search);
    clear_direct_message_pending(signals);
}

#[cfg(windows)]
fn clear_direct_message_pending(mut signals: DirectMessageSignals) {
    // A token refresh invalidates all result guards. Clear these flags before starting new
    // requests so an old task that exits on its generation check cannot strand the UI.
    next_direct_message_load_generation(signals.threads_load_generation);
    next_direct_message_load_generation(signals.history_load_generation);
    invalidate_user_autocomplete(signals.search);
    signals.threads_pending.set(false);
    signals.history_pending.set(false);
    signals.create_pending.set(false);
    signals.send_pending.set(false);
}

#[cfg(windows)]
fn start_direct_message_session(
    _session: &AuthSession,
    _session_generation: Signal<u64>,
    signals: DirectMessageSignals,
) {
    reset_direct_message_state(signals);
}

#[cfg(windows)]
fn refresh_direct_message_state(
    session: &AuthSession,
    session_generation: Signal<u64>,
    signals: DirectMessageSignals,
) {
    load_direct_message_threads(session.clone(), session_generation, signals);
    if let Some(thread_id) = *signals.selected_thread_id.read() {
        load_direct_message_history_page(
            session.clone(),
            session_generation,
            signals,
            thread_id,
            None,
            false,
        );
    }
}

#[cfg(windows)]
fn load_direct_message_threads(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
) {
    let generation = *session_generation.read();
    let load_generation = next_direct_message_load_generation(signals.threads_load_generation);
    signals.threads_pending.set(true);
    signals.status.set("Loading direct messages...".to_string());
    spawn(async move {
        match list_direct_message_threads_api(&session).await {
            Ok(threads) => {
                if !session_generation_current(session_generation, generation)
                    || !direct_message_load_generation_current(
                        signals.threads_load_generation,
                        load_generation,
                    )
                {
                    return;
                }

                let selected_thread_id = *signals.selected_thread_id.read();
                let selected_is_visible = match selected_thread_id {
                    Some(thread_id) => threads.iter().any(|thread| thread.id == thread_id),
                    None => true,
                };
                let count = threads.len();
                signals.threads.set(threads);
                signals.threads_pending.set(false);
                if selected_is_visible {
                    signals
                        .status
                        .set(format!("Loaded {count} direct message threads"));
                } else {
                    signals.selected_thread_id.set(None);
                    signals.messages.set(Vec::new());
                    signals.next_before_message_id.set(None);
                    signals.composer_body.set(String::new());
                    signals
                        .status
                        .set("That direct message is no longer available".to_string());
                }
            }
            Err(error) => {
                if session_generation_current(session_generation, generation)
                    && direct_message_load_generation_current(
                        signals.threads_load_generation,
                        load_generation,
                    )
                {
                    signals.threads_pending.set(false);
                    signals.status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn open_direct_message_thread(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
    thread_id: uuid::Uuid,
) {
    signals.selected_thread_id.set(Some(thread_id));
    signals.messages.set(Vec::new());
    signals.next_before_message_id.set(None);
    signals.composer_body.set(String::new());
    load_direct_message_history_page(session, session_generation, signals, thread_id, None, true);
}

#[cfg(windows)]
fn load_direct_message_history_page(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
    thread_id: uuid::Uuid,
    before: Option<uuid::Uuid>,
    replace: bool,
) {
    if before.is_some() && *signals.history_pending.read() {
        return;
    }

    let generation = *session_generation.read();
    let load_generation = next_direct_message_load_generation(signals.history_load_generation);
    signals.history_pending.set(true);
    if replace {
        signals
            .status
            .set("Loading direct message history...".to_string());
    }
    spawn(async move {
        match list_direct_message_history_api(&session, thread_id, before).await {
            Ok(history) => {
                if !session_generation_current(session_generation, generation)
                    || !direct_message_load_generation_current(
                        signals.history_load_generation,
                        load_generation,
                    )
                    || *signals.selected_thread_id.read() != Some(thread_id)
                {
                    return;
                }

                merge_direct_messages(&mut signals.messages.write(), history.messages);
                signals
                    .next_before_message_id
                    .set(history.next_before_message_id);
                signals.history_pending.set(false);
                signals
                    .status
                    .set("Direct message history loaded".to_string());
            }
            Err(error) => {
                if session_generation_current(session_generation, generation)
                    && direct_message_load_generation_current(
                        signals.history_load_generation,
                        load_generation,
                    )
                    && *signals.selected_thread_id.read() == Some(thread_id)
                {
                    signals.history_pending.set(false);
                    signals.status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn user_autocomplete_query_ready(query: &str) -> bool {
    query.trim().chars().count() >= USER_AUTOCOMPLETE_MIN_QUERY_CHARS
}

#[cfg(windows)]
fn next_user_autocomplete_generation(mut signals: UserAutocompleteSignals) -> u64 {
    let current = *signals.request_generation.read();
    let next = current.wrapping_add(1);
    signals.request_generation.set(next);
    next
}

#[cfg(windows)]
fn user_autocomplete_request_current(
    session_generation: Signal<u64>,
    session_value: u64,
    signals: UserAutocompleteSignals,
    request_generation: u64,
    query: &str,
) -> bool {
    session_generation_current(session_generation, session_value)
        && *signals.request_generation.read() == request_generation
        && signals.query.read().trim() == query
}

#[cfg(windows)]
fn update_user_autocomplete_query(
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    mut signals: UserAutocompleteSignals,
    value: String,
) {
    let query = value.trim().to_string();
    signals.query.set(value);
    signals.selected.set(None);
    signals.results.set(Vec::new());
    signals.active_index.set(None);
    let request_generation = next_user_autocomplete_generation(signals);

    if !user_autocomplete_query_ready(&query) {
        signals.pending.set(false);
        signals.expanded.set(false);
        signals.status.set(if query.is_empty() {
            String::new()
        } else {
            format!("Type at least {USER_AUTOCOMPLETE_MIN_QUERY_CHARS} characters to search")
        });
        return;
    }

    let Some(session) = session else {
        signals.pending.set(false);
        signals.expanded.set(false);
        signals
            .status
            .set("Sign in to search for users".to_string());
        return;
    };

    let session_value = *session_generation.read();
    signals.pending.set(true);
    signals.expanded.set(true);
    signals.status.set(format!("Searching for {query}..."));
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(
            USER_AUTOCOMPLETE_DEBOUNCE_MS,
        ))
        .await;
        if !user_autocomplete_request_current(
            session_generation,
            session_value,
            signals,
            request_generation,
            &query,
        ) {
            return;
        }

        match search_users_api(&session, &query).await {
            Ok(users) => {
                if !user_autocomplete_request_current(
                    session_generation,
                    session_value,
                    signals,
                    request_generation,
                    &query,
                ) {
                    return;
                }

                let count = users.len();
                signals.results.set(users);
                signals.pending.set(false);
                signals.active_index.set((count > 0).then_some(0));
                signals.status.set(if count == 0 {
                    "No matching users".to_string()
                } else {
                    format!("Found {count} users")
                });
            }
            Err(error) => {
                if user_autocomplete_request_current(
                    session_generation,
                    session_value,
                    signals,
                    request_generation,
                    &query,
                ) {
                    signals.results.set(Vec::new());
                    signals.pending.set(false);
                    signals.active_index.set(None);
                    signals.status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn select_user_autocomplete_option(mut signals: UserAutocompleteSignals, index: usize) {
    let Some(user) = signals.results.read().get(index).cloned() else {
        return;
    };

    next_user_autocomplete_generation(signals);
    let display_name = user.display_name.clone();
    signals.query.set(display_name.clone());
    signals.results.set(Vec::new());
    signals.selected.set(Some(user));
    signals.pending.set(false);
    signals.expanded.set(false);
    signals.active_index.set(None);
    signals.status.set(format!("Selected {display_name}"));
}

#[cfg(windows)]
fn next_user_autocomplete_active_index(
    count: usize,
    active: Option<usize>,
    move_forward: bool,
) -> Option<usize> {
    match (count, active, move_forward) {
        (0, _, _) => None,
        (count, Some(index), true) if index < count => Some((index + 1) % count),
        (count, Some(index), false) if index < count => Some((index + count - 1) % count),
        (_, _, true) => Some(0),
        (count, _, false) => Some(count - 1),
    }
}

#[cfg(windows)]
fn move_user_autocomplete_active_option(mut signals: UserAutocompleteSignals, move_forward: bool) {
    let next = next_user_autocomplete_active_index(
        signals.results.read().len(),
        *signals.active_index.read(),
        move_forward,
    );
    signals.active_index.set(next);
    if next.is_some() {
        signals.expanded.set(true);
    }
}

#[cfg(windows)]
fn dismiss_user_autocomplete(mut signals: UserAutocompleteSignals) {
    signals.expanded.set(false);
    signals.active_index.set(None);
}

#[cfg(windows)]
fn show_user_autocomplete(mut signals: UserAutocompleteSignals) {
    if signals.selected.read().is_none() && user_autocomplete_query_ready(&signals.query.read()) {
        signals.expanded.set(true);
    }
}

#[cfg(windows)]
fn invalidate_user_autocomplete(mut signals: UserAutocompleteSignals) {
    next_user_autocomplete_generation(signals);
    signals.pending.set(false);
    dismiss_user_autocomplete(signals);
}

#[cfg(windows)]
fn reset_user_autocomplete(mut signals: UserAutocompleteSignals) {
    invalidate_user_autocomplete(signals);
    signals.query.set(String::new());
    signals.results.set(Vec::new());
    signals.selected.set(None);
    signals.status.set(String::new());
}

#[cfg(windows)]
fn create_direct_message_thread(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
    mut active_tab: Signal<AppTab>,
    target: UserSummary,
) {
    if *signals.create_pending.read() {
        return;
    }

    let generation = *session_generation.read();
    signals.create_pending.set(true);
    signals.status.set(format!(
        "Opening a conversation with {}...",
        target.display_name
    ));
    spawn(async move {
        match create_direct_message_thread_api(&session, target.id).await {
            Ok(thread) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }

                upsert_direct_message_thread(&mut signals.threads.write(), thread.clone());
                signals.selected_thread_id.set(Some(thread.id));
                signals.messages.set(Vec::new());
                signals.next_before_message_id.set(None);
                signals.composer_body.set(String::new());
                signals.create_pending.set(false);
                active_tab.set(AppTab::DirectMessages);
                load_direct_message_history_page(
                    session,
                    session_generation,
                    signals,
                    thread.id,
                    None,
                    true,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    signals.create_pending.set(false);
                    signals.status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn send_direct_message(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
    thread_id: uuid::Uuid,
) {
    if *signals.send_pending.read() {
        return;
    }

    let body = signals.composer_body.read().trim().to_string();
    if body.is_empty() {
        return;
    }
    if body.chars().count() > MAX_MESSAGE_LEN {
        signals.status.set(format!(
            "Message is too long. Limit: {MAX_MESSAGE_LEN} characters"
        ));
        return;
    }

    let generation = *session_generation.read();
    signals.send_pending.set(true);
    signals.status.set("Sending direct message...".to_string());
    spawn(async move {
        match send_direct_message_api(&session, thread_id, body).await {
            Ok(message) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }

                signals.send_pending.set(false);
                if *signals.selected_thread_id.read() == Some(thread_id) {
                    merge_direct_messages(&mut signals.messages.write(), vec![message]);
                    signals.composer_body.set(String::new());
                }
                signals.status.set("Direct message sent".to_string());
                load_direct_message_threads(session, session_generation, signals);
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    signals.send_pending.set(false);
                    signals.status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn remove_direct_messages_for_user(mut signals: DirectMessageSignals, user_id: uuid::Uuid) {
    let selected_thread_id = *signals.selected_thread_id.read();
    let selected_is_hidden = selected_thread_id.is_some_and(|thread_id| {
        signals
            .threads
            .read()
            .iter()
            .any(|thread| thread.id == thread_id && thread.other_user.id == user_id)
    });
    signals
        .threads
        .write()
        .retain(|thread| thread.other_user.id != user_id);
    if selected_is_hidden {
        signals.selected_thread_id.set(None);
        signals.messages.set(Vec::new());
        signals.next_before_message_id.set(None);
        signals.composer_body.set(String::new());
        signals
            .status
            .set("Direct message hidden because this user is blocked".to_string());
    }
}

#[cfg(windows)]
fn upsert_direct_message_thread(threads: &mut Vec<DmThreadSummary>, thread: DmThreadSummary) {
    match threads.iter_mut().find(|existing| existing.id == thread.id) {
        Some(existing) => *existing = thread,
        None => threads.push(thread),
    }
    threads.sort_by(|left, right| {
        let left_time = left.last_message_at.as_deref().unwrap_or(&left.created_at);
        let right_time = right
            .last_message_at
            .as_deref()
            .unwrap_or(&right.created_at);
        right_time
            .cmp(left_time)
            .then_with(|| right.id.cmp(&left.id))
    });
}

#[cfg(windows)]
fn merge_direct_messages(messages: &mut Vec<DmMessage>, incoming: Vec<DmMessage>) {
    for message in incoming {
        if !messages.iter().any(|existing| existing.id == message.id) {
            messages.push(message);
        }
    }
    messages.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[cfg(windows)]
fn load_friends(
    session: AuthSession,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    mut friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    let load_generation = next_relationship_load_generation(friendships_load_generation);
    spawn(async move {
        if !session_generation_current(session_generation, generation)
            || !relationship_load_generation_current(friendships_load_generation, load_generation)
        {
            return;
        }
        friends_status.set("Loading friends...".to_string());
        match list_friends_api(&session).await {
            Ok(items) => {
                if !session_generation_current(session_generation, generation)
                    || !relationship_load_generation_current(
                        friendships_load_generation,
                        load_generation,
                    )
                {
                    return;
                }
                let count = items.len();
                friendships.set(items);
                friends_status.set(format!("Loaded {count} friendship records"));
            }
            Err(error) => {
                if session_generation_current(session_generation, generation)
                    && relationship_load_generation_current(
                        friendships_load_generation,
                        load_generation,
                    )
                {
                    friends_status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn load_blocks(
    session: AuthSession,
    session_generation: Signal<u64>,
    blocks_load_generation: Signal<u64>,
    mut blocked_users: Signal<Vec<UserSummary>>,
    mut block_status: Signal<String>,
) {
    let generation = *session_generation.read();
    let load_generation = next_relationship_load_generation(blocks_load_generation);
    spawn(async move {
        if !session_generation_current(session_generation, generation)
            || !relationship_load_generation_current(blocks_load_generation, load_generation)
        {
            return;
        }
        block_status.set("Loading blocked users...".to_string());
        match list_blocks_api(&session).await {
            Ok(users) => {
                if !session_generation_current(session_generation, generation)
                    || !relationship_load_generation_current(
                        blocks_load_generation,
                        load_generation,
                    )
                {
                    return;
                }
                let count = users.len();
                blocked_users.set(users);
                block_status.set(format!("Loaded {count} blocked users"));
            }
            Err(error) => {
                if session_generation_current(session_generation, generation)
                    && relationship_load_generation_current(blocks_load_generation, load_generation)
                {
                    block_status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn send_friend_invite(
    session: AuthSession,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    target: UserSummary,
    friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        friends_status.set(format!(
            "Sending friend request to {}...",
            target.display_name
        ));
        match send_friend_request_api(&session, &target.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                friends_status.set(format!("Friend request sent to {}", target.display_name));
                load_friends(
                    session,
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    friends_status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn accept_friend_invite(
    session: AuthSession,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    friendship: FriendshipSummary,
    friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        friends_status.set("Accepting friend request...".to_string());
        match accept_friend_request_api(&session, &friendship.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                friends_status.set("Friend request accepted".to_string());
                load_friends(
                    session,
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    friends_status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn decline_friend_invite(
    session: AuthSession,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    friendship: FriendshipSummary,
    friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        friends_status.set("Declining friend request...".to_string());
        match decline_friend_request_api(&session, &friendship.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                friends_status.set("Friend request declined".to_string());
                load_friends(
                    session,
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    friends_status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn remove_friend(
    session: AuthSession,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    target: UserSummary,
    friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        friends_status.set(format!("Removing {}...", target.display_name));
        match remove_friend_api(&session, &target.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                friends_status.set(format!("Removed {}", target.display_name));
                load_friends(
                    session,
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    friends_status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn block_user_action(
    session: AuthSession,
    session_generation: Signal<u64>,
    target: UserSummary,
    mut chat_messages: Signal<Vec<ChatMessage>>,
    mut block_status: Signal<String>,
    signals: RelationshipSignals,
    direct_messages: DirectMessageSignals,
) {
    let generation = *session_generation.read();
    let blocked_user_id = target.id;
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        block_status.set(format!("Blocking {}...", target.display_name));
        match block_user_api(&session, &target.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                {
                    let mut messages = chat_messages.write();
                    remove_blocked_user_messages(&mut messages, blocked_user_id);
                }
                remove_direct_messages_for_user(direct_messages, blocked_user_id);
                block_status.set(format!("Blocked {}", target.display_name));
                refresh_direct_message_state(&session, session_generation, direct_messages);
                load_blocks(
                    session.clone(),
                    session_generation,
                    signals.blocks_load_generation,
                    signals.blocked_users,
                    block_status,
                );
                load_friends(
                    session,
                    session_generation,
                    signals.friendships_load_generation,
                    signals.friendships,
                    signals.friends_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    block_status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn remove_blocked_user_messages(messages: &mut Vec<ChatMessage>, blocked_user_id: uuid::Uuid) {
    messages.retain(|message| message.author.id != blocked_user_id);
}

#[cfg(windows)]
fn unblock_user_action(
    session: AuthSession,
    session_generation: Signal<u64>,
    blocks_load_generation: Signal<u64>,
    target: UserSummary,
    blocked_users: Signal<Vec<UserSummary>>,
    mut block_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        block_status.set(format!("Unblocking {}...", target.display_name));
        match unblock_user_api(&session, &target.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                block_status.set(format!("Unblocked {}", target.display_name));
                load_blocks(
                    session,
                    session_generation,
                    blocks_load_generation,
                    blocked_users,
                    block_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    block_status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn user_report_draft(user: UserSummary) -> ReportDraft {
    ReportDraft {
        target: user,
        message_id: None,
        message_kind: None,
        message_preview: None,
    }
}

#[cfg(windows)]
fn global_message_report_draft(message: &ChatMessage) -> ReportDraft {
    ReportDraft {
        target: message.author.clone(),
        message_id: Some(message.id),
        message_kind: Some(MessageKind::Global),
        message_preview: Some(message.body.clone()),
    }
}

#[cfg(windows)]
fn direct_message_report_draft(message: &DmMessage) -> ReportDraft {
    ReportDraft {
        target: message.author.clone(),
        message_id: Some(message.id),
        message_kind: Some(MessageKind::Dm),
        message_preview: Some(message.body.clone()),
    }
}

#[cfg(windows)]
fn report_draft_status(draft: &ReportDraft) -> String {
    if draft.message_id.is_some() {
        format!(
            "Selected message from {}. Enter a reason, then submit.",
            draft.target.display_name
        )
    } else {
        format!(
            "Selected {}. Enter a reason, then submit.",
            draft.target.display_name
        )
    }
}

#[cfg(windows)]
fn report_draft_kind_label(draft: &ReportDraft) -> &'static str {
    match draft.message_kind {
        Some(MessageKind::Global) => "Global message",
        Some(MessageKind::Dm) => "Direct message",
        None => "User",
    }
}

#[cfg(windows)]
fn report_preview_text(value: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 140;
    let mut preview = value.chars().take(MAX_PREVIEW_CHARS).collect::<String>();
    if value.chars().count() > MAX_PREVIEW_CHARS {
        preview.push_str("...");
    }
    preview
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn submit_report_action(
    session: AuthSession,
    session_generation: Signal<u64>,
    draft: ReportDraft,
    reason: String,
    details: String,
    mut report_status: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_reason: Signal<String>,
    mut report_details: Signal<String>,
) {
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        report_status.set("Report reason is required".to_string());
        return;
    }
    let details = details.trim().to_string();
    let details = if details.is_empty() {
        None
    } else {
        Some(details)
    };

    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        let target_name = draft.target.display_name.clone();
        report_status.set(format!("Reporting {target_name}..."));
        match create_report_api(
            &session,
            draft.target.id,
            draft.message_id,
            draft.message_kind,
            reason,
            details,
        )
        .await
        {
            Ok(report_id) => {
                if session_generation_current(session_generation, generation) {
                    report_draft.set(None);
                    report_reason.set(String::new());
                    report_details.set(String::new());
                    report_status.set(format!("Report submitted for {target_name}: {report_id}"));
                }
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    report_status.set(error);
                }
            }
        }
    });
}

#[cfg(windows)]
fn start_chat_session(session: &AuthSession, signals: ChatSessionSignals) {
    let session_generation = signals.session_generation;
    let mut chat_messages = signals.chat_messages;
    let mut deleted_message_ids = signals.deleted_message_ids;
    let mut chat_status = signals.chat_status;
    let mut presence_counts = signals.presence_counts;
    let mut chat_outbox = signals.chat_outbox;
    let mut chat_send_pending = signals.chat_send_pending;
    let _ = chat_outbox.write().take();
    // REST data is held until this session's hello confirms protocol compatibility.
    begin_bearer_compatibility_check();
    reset_presence_selection(signals.presence_selection, "Connecting to set availability");
    chat_send_pending.set(false);
    chat_messages.set(Vec::new());
    deleted_message_ids.set(Vec::new());
    presence_counts.set(PresenceCounts::default());
    chat_status.set("Connecting to global chat...".to_string());

    let session = session.clone();
    let generation = *session_generation.read();
    let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();
    chat_outbox.set(Some(outgoing_tx));

    spawn(async move {
        if let Err(error) = run_chat_socket(
            session,
            session_generation,
            generation,
            outgoing_rx,
            signals,
        )
        .await
        {
            if session_generation_current(session_generation, generation) {
                chat_outbox.set(None);
                chat_send_pending.set(false);
                reset_presence_selection(
                    signals.presence_selection,
                    "Chat disconnected; availability reset to Online",
                );
                chat_status.set(format!("Chat disconnected: {error}"));
            }
        }
    });
}

#[cfg(windows)]
fn stop_chat_session(
    mut chat_messages: Signal<Vec<ChatMessage>>,
    mut deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    mut chat_status: Signal<String>,
    mut presence_counts: Signal<PresenceCounts>,
    presence_selection: PresenceSelectionSignals,
    mut chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    mut chat_send_pending: Signal<bool>,
) {
    let _ = chat_outbox.write().take();
    chat_send_pending.set(false);
    chat_messages.set(Vec::new());
    deleted_message_ids.set(Vec::new());
    presence_counts.set(PresenceCounts::default());
    reset_presence_selection(presence_selection, "Sign in to set availability");
    chat_status.set("Sign in to connect to global chat".to_string());
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
async fn run_chat_socket(
    session: AuthSession,
    session_generation: Signal<u64>,
    generation: u64,
    mut outgoing_rx: UnboundedReceiver<OutgoingChatEvent>,
    signals: ChatSessionSignals,
) -> Result<(), String> {
    let mut chat_status = signals.chat_status;
    let mut reconnect_attempt = 0u64;
    let reconnect_until = std::time::Instant::now()
        + std::time::Duration::from_secs(chat_reconnect_window(session.expires_in_seconds));

    while !outgoing_rx.is_closed() {
        if !session_generation_current(session_generation, generation) {
            return Ok(());
        }

        if std::time::Instant::now() >= reconnect_until {
            return Err("chat token is waiting for session refresh".to_string());
        }

        match run_chat_socket_once(
            &session,
            session_generation,
            generation,
            &mut outgoing_rx,
            signals,
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(error) => {
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                let delay = reconnect_attempt.min(5);
                if std::time::Instant::now() + std::time::Duration::from_secs(delay)
                    >= reconnect_until
                {
                    return Err("chat token is waiting for session refresh".to_string());
                }
                if session_generation_current(session_generation, generation) {
                    reset_presence_selection(
                        signals.presence_selection,
                        "Chat disconnected; availability reset to Online",
                    );
                    chat_status.set(format!(
                        "Chat disconnected: {error}. Reconnecting in {delay}s..."
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }
        }
    }

    Ok(())
}

#[cfg(windows)]
fn chat_reconnect_window(expires_in_seconds: u64) -> u64 {
    expires_in_seconds
        .saturating_sub(CHAT_RECONNECT_SAFETY_SECONDS)
        .max(1)
}

#[cfg(windows)]
async fn run_chat_socket_once(
    session: &AuthSession,
    session_generation: Signal<u64>,
    generation: u64,
    outgoing_rx: &mut UnboundedReceiver<OutgoingChatEvent>,
    mut signals: ChatSessionSignals,
) -> Result<(), String> {
    let mut chat_status = signals.chat_status;
    signals.chat_messages.set(Vec::new());
    signals.deleted_message_ids.set(Vec::new());
    if !session_generation_current(session_generation, generation) {
        return Ok(());
    }
    // Revalidate before every reconnect too: a server can raise its minimum version while a
    // client is already running.
    begin_bearer_compatibility_check();
    reset_presence_selection(signals.presence_selection, "Connecting to set availability");

    let mut request = websocket_url()?
        .into_client_request()
        .map_err(|error| format!("Could not build chat request: {error}"))?;
    let bearer = format!("Bearer {}", session.access_token);
    let header = HeaderValue::from_str(&bearer)
        .map_err(|error| format!("Could not build chat auth header: {error}"))?;
    request.headers_mut().insert(AUTHORIZATION, header);

    let (socket, _) = connect_async(request)
        .await
        .map_err(|error| format!("Could not connect to chat: {error}"))?;
    let (mut socket_tx, mut socket_rx) = socket.split();

    let hello = ClientEvent::Hello {
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
    };
    let text = serde_json::to_string(&hello)
        .map_err(|error| format!("Could not serialize chat hello: {error}"))?;
    socket_tx
        .send(Message::Text(text.into()))
        .await
        .map_err(|error| format!("Could not send chat hello: {error}"))?;
    if session_generation_current(session_generation, generation) {
        chat_status.set("Global chat connected".to_string());
    }

    loop {
        tokio::select! {
            outgoing = outgoing_rx.recv() => {
                let Some(outgoing) = outgoing else {
                    let _ = socket_tx.send(Message::Close(None)).await;
                    return Ok(());
                };
                let text = match serde_json::to_string(&outgoing.event) {
                    Ok(text) => text,
                    Err(error) => {
                        let message = format!("Could not serialize chat event: {error}");
                        let _ = outgoing.transport_result.send(Err(message.clone()));
                        return Err(message);
                    }
                };
                if let Err(error) = socket_tx.send(Message::Text(text.into())).await {
                    let message = format!("Could not send chat event: {error}");
                    let _ = outgoing.transport_result.send(Err(message.clone()));
                    return Err(message);
                }
                let _ = outgoing.transport_result.send(Ok(()));
            }
            incoming = socket_rx.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if !handle_chat_event(
                            text.as_str(),
                            session,
                            session_generation,
                            generation,
                            signals,
                        ) {
                            let _ = socket_tx.send(Message::Close(None)).await;
                            return Ok(());
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => socket_tx
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| format!("Could not respond to chat ping: {error}"))?,
                    Some(Ok(Message::Close(_))) | None => return Err("server closed the chat connection".to_string()),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(format!("Chat WebSocket failed: {error}")),
                }
            }
        }
    }
}

#[cfg(windows)]
fn handle_chat_event(
    text: &str,
    session: &AuthSession,
    session_generation: Signal<u64>,
    generation: u64,
    signals: ChatSessionSignals,
) -> bool {
    if !session_generation_current(session_generation, generation) {
        return false;
    }
    let mut chat_messages = signals.chat_messages;
    let mut deleted_message_ids = signals.deleted_message_ids;
    let mut chat_status = signals.chat_status;
    let mut presence_counts = signals.presence_counts;
    let mut chat_outbox = signals.chat_outbox;
    let mut chat_send_pending = signals.chat_send_pending;
    let mut reauth_required = signals.reauth_required;

    match serde_json::from_str::<ServerEvent>(text) {
        Ok(ServerEvent::HelloOk {
            protocol_version,
            presence,
        }) => {
            if protocol_version != PROTOCOL_VERSION {
                let message = "Client protocol mismatch. Update Agora.".to_string();
                block_bearer_access(message.clone());
                reauth_required.set(Some(message.clone()));
                request_required_update(signals.updates, message.clone());
                terminate_chat_session_locally(signals, message, false, false);
                return false;
            }
            allow_bearer_access();
            presence_counts.set(presence);
            mark_presence_selection_connected(signals.presence_selection);
            refresh_relationship_state(session, session_generation, signals.relationships);
            refresh_direct_message_state(session, session_generation, signals.direct_messages);
            chat_status.set("Global chat connected".to_string());
        }
        Ok(ServerEvent::GlobalMessageSnapshot { messages }) => {
            let deleted = deleted_message_ids.read().clone();
            chat_messages.set(
                messages
                    .into_iter()
                    .filter(|message| !deleted.contains(&message.id))
                    .collect(),
            );
            // Snapshots are sent after reconnect and broadcast lag recovery, so they are a
            // reliable boundary for refreshing REST-backed DM state as well.
            refresh_direct_message_state(session, session_generation, signals.direct_messages);
        }
        Ok(ServerEvent::GlobalMessageCreated(message)) => {
            if deleted_message_ids.read().contains(&message.id) {
                return true;
            }
            let mut messages = chat_messages.write();
            if messages.iter().any(|existing| existing.id == message.id) {
                return true;
            }
            messages.push(message);
            let overflow = messages.len().saturating_sub(200);
            if overflow > 0 {
                messages.drain(0..overflow);
            }
        }
        Ok(ServerEvent::GlobalMessageDeleted { message_id }) => {
            {
                let mut deleted = deleted_message_ids.write();
                remember_deleted_message(&mut deleted, message_id);
            }
            let mut messages = chat_messages.write();
            remove_message_by_id(&mut messages, message_id);
        }
        Ok(ServerEvent::UserMessagesHidden { user_id }) => {
            let mut messages = chat_messages.write();
            remove_blocked_user_messages(&mut messages, user_id);
            remove_direct_messages_for_user(signals.direct_messages, user_id);
        }
        Ok(ServerEvent::RelationshipStateChanged) => {
            refresh_relationship_state(session, session_generation, signals.relationships);
            refresh_direct_message_state(session, session_generation, signals.direct_messages);
        }
        Ok(ServerEvent::AccessTokenExpired) => {
            if reauth_required.read().is_some() {
                terminate_chat_session_locally(
                    signals,
                    "Session expired before its refresh outcome could be recovered. Sign in again."
                        .to_string(),
                    false,
                    true,
                );
                return false;
            }
            let _ = chat_outbox.write().take();
            chat_send_pending.set(false);
            reset_presence_selection(
                signals.presence_selection,
                "Refreshing chat session; availability reset to Online",
            );
            chat_status.set("Refreshing chat session...".to_string());
            return false;
        }
        Ok(ServerEvent::Error { message }) if terminal_chat_error(&message) => {
            terminate_chat_session_locally(
                signals,
                format!("Chat disconnected: {message}"),
                true,
                true,
            );
            return false;
        }
        Ok(ServerEvent::Error { message }) => {
            reject_unconfirmed_presence_selection(signals.presence_selection, &message);
            chat_status.set(format!("Chat error: {message}"));
        }
        Ok(ServerEvent::MinimumVersionRequired {
            minimum_client_version,
        }) => {
            let message =
                format!("Client update required. Minimum version: {minimum_client_version}");
            block_bearer_access(message.clone());
            reauth_required.set(Some(message.clone()));
            request_required_update(signals.updates, message.clone());
            terminate_chat_session_locally(signals, message, false, false);
            return false;
        }
        Ok(ServerEvent::ProtocolIncompatible {
            required_protocol_version,
        }) => {
            let message = format!(
                "Client protocol update required. Required protocol: {required_protocol_version}"
            );
            block_bearer_access(message.clone());
            reauth_required.set(Some(message.clone()));
            request_required_update(signals.updates, message.clone());
            terminate_chat_session_locally(signals, message, false, false);
            return false;
        }
        Ok(ServerEvent::PresenceCounts(counts)) => presence_counts.set(counts),
        Err(_) => match serde_json::from_str::<DmRealtimeEvent>(text) {
            Ok(event) => handle_direct_message_event(
                event,
                session,
                session_generation,
                signals.direct_messages,
            ),
            Err(error) => chat_status.set(format!("Could not read chat event: {error}")),
        },
    }
    true
}

#[cfg(windows)]
fn handle_direct_message_event(
    event: DmRealtimeEvent,
    session: &AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
) {
    match event {
        DmRealtimeEvent::DmThreadUpdated(thread) => {
            upsert_direct_message_thread(&mut signals.threads.write(), thread);
        }
        DmRealtimeEvent::DmMessageCreated(message) => {
            let thread_is_known = signals
                .threads
                .read()
                .iter()
                .any(|thread| thread.id == message.thread_id);
            if *signals.selected_thread_id.read() == Some(message.thread_id) {
                merge_direct_messages(&mut signals.messages.write(), vec![message.clone()]);
            }
            if thread_is_known {
                signals.status.set(format!(
                    "New direct message from {}",
                    message.author.display_name
                ));
            } else {
                // A reconnect can resume between the paired thread and message events.
                load_direct_message_threads(session.clone(), session_generation, signals);
            }
        }
    }
}

#[cfg(windows)]
fn terminate_chat_session_locally(
    mut signals: ChatSessionSignals,
    message: String,
    clear_credentials: bool,
    clear_auth_session: bool,
) {
    if clear_credentials {
        let _ = clear_refresh_token();
    }
    if clear_auth_session {
        signals.auth_session.set(None);
    }
    next_session_generation(signals.session_generation);
    stop_chat_session(
        signals.chat_messages,
        signals.deleted_message_ids,
        signals.chat_status,
        signals.presence_counts,
        signals.presence_selection,
        signals.chat_outbox,
        signals.chat_send_pending,
    );
    signals.relationships.friendships.set(Vec::new());
    signals
        .relationships
        .friends_status
        .set("Sign in to load friends".to_string());
    signals.relationships.blocked_users.set(Vec::new());
    signals
        .relationships
        .block_status
        .set("Sign in to manage blocks".to_string());
    reset_direct_message_state(signals.direct_messages);
    signals.login_status.set(message.clone());
    signals.chat_status.set(message);
}

#[cfg(windows)]
fn terminal_chat_error(message: &str) -> bool {
    matches!(
        message,
        "Chat session ended; sign in again" | "Account status changed; sign in again"
    )
}

#[cfg(windows)]
fn remove_message_by_id(messages: &mut Vec<ChatMessage>, message_id: uuid::Uuid) {
    messages.retain(|message| message.id != message_id);
}

#[cfg(windows)]
fn remember_deleted_message(deleted_message_ids: &mut Vec<uuid::Uuid>, message_id: uuid::Uuid) {
    if deleted_message_ids.contains(&message_id) {
        return;
    }
    deleted_message_ids.push(message_id);
    let overflow = deleted_message_ids
        .len()
        .saturating_sub(DELETED_MESSAGE_CACHE_LIMIT);
    if overflow > 0 {
        deleted_message_ids.drain(0..overflow);
    }
}

#[cfg(windows)]
fn send_pending_global_message(
    session_generation: Signal<u64>,
    mut composer_body: Signal<String>,
    mut chat_status: Signal<String>,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    mut chat_send_pending: Signal<bool>,
) {
    if *chat_send_pending.read() {
        return;
    }

    let body = composer_body.read().trim().to_string();
    if body.is_empty() {
        return;
    }
    if body.chars().count() > MAX_MESSAGE_LEN {
        chat_status.set(format!(
            "Message is too long. Limit: {MAX_MESSAGE_LEN} characters"
        ));
        return;
    }

    let sender = chat_outbox.read().clone();
    match sender {
        Some(sender) => {
            let generation = *session_generation.read();
            let (transport_result_tx, transport_result_rx) = oneshot::channel();
            match sender.send(OutgoingChatEvent {
                event: ClientEvent::GlobalMessageSend { body: body.clone() },
                transport_result: transport_result_tx,
            }) {
                Ok(()) => {
                    composer_body.set(String::new());
                    chat_send_pending.set(true);
                    chat_status.set("Sending message...".to_string());
                    spawn(async move {
                        let result = match transport_result_rx.await {
                            Ok(result) => result,
                            Err(_) => Err(
                                "chat connection closed before the message could be delivered"
                                    .to_string(),
                            ),
                        };
                        if !session_generation_current(session_generation, generation) {
                            return;
                        }

                        chat_send_pending.set(false);
                        match result {
                            Ok(()) => {
                                chat_status.set("Message handed to the chat connection".to_string())
                            }
                            Err(error) => {
                                let restored = restore_composer_after_transport_failure(
                                    &mut composer_body.write(),
                                    &body,
                                );
                                let status = if restored {
                                    format!(
                                    "Message was not delivered to the chat connection: {error}. Draft restored."
                                )
                                } else {
                                    format!(
                                    "Message was not delivered to the chat connection: {error}. Your newer draft was kept."
                                )
                                };
                                chat_status.set(status);
                            }
                        }
                    });
                }
                Err(_) => chat_status.set("Chat connection is not available".to_string()),
            }
        }
        None => chat_status.set("Chat connection is not available".to_string()),
    }
}

#[cfg(windows)]
fn presence_selection_value(state: PresenceState) -> &'static str {
    match state {
        PresenceState::Online | PresenceState::Offline => "online",
        PresenceState::LookingForGame => "looking_for_game",
        PresenceState::InGame => "in_game",
    }
}

#[cfg(windows)]
fn presence_state_from_selection_value(value: &str) -> Option<PresenceState> {
    match value {
        "online" => Some(PresenceState::Online),
        "looking_for_game" => Some(PresenceState::LookingForGame),
        "in_game" => Some(PresenceState::InGame),
        _ => None,
    }
}

#[cfg(windows)]
fn presence_selection_label(state: PresenceState) -> &'static str {
    match state {
        PresenceState::Online | PresenceState::Offline => "Online",
        PresenceState::LookingForGame => "Looking for game",
        PresenceState::InGame => "In game",
    }
}

#[cfg(windows)]
fn presence_selection_status(state: PresenceState) -> String {
    format!("Availability: {}", presence_selection_label(state))
}

#[cfg(windows)]
fn can_update_presence_selection(signed_in: bool, connected: bool, pending: bool) -> bool {
    signed_in && connected && !pending
}

#[cfg(windows)]
fn next_presence_selection_request_generation(mut signals: PresenceSelectionSignals) -> u64 {
    let next = (*signals.request_generation.read()).wrapping_add(1);
    signals.request_generation.set(next);
    next
}

#[cfg(windows)]
fn presence_selection_request_is_current(
    signals: PresenceSelectionSignals,
    generation: u64,
) -> bool {
    *signals.request_generation.read() == generation
}

#[cfg(windows)]
fn reset_presence_selection(mut signals: PresenceSelectionSignals, status: &str) {
    next_presence_selection_request_generation(signals);
    signals.selected.set(PresenceState::Online);
    signals.confirmed.set(PresenceState::Online);
    signals.unconfirmed.set(None);
    signals.pending.set(false);
    signals.connected.set(false);
    signals.status.set(status.to_string());
}

#[cfg(windows)]
fn mark_presence_selection_connected(mut signals: PresenceSelectionSignals) {
    signals.connected.set(true);
    signals.pending.set(false);
    // The server establishes a newly connected presence as Online. Manual updates have no
    // protocol-level acknowledgement, so only this connection state is confirmed.
    signals.confirmed.set(PresenceState::Online);
    signals.unconfirmed.set(None);
    signals
        .status
        .set(presence_selection_status(PresenceState::Online));
}

#[cfg(windows)]
fn unconfirmed_presence_selection_status(state: PresenceState) -> String {
    format!(
        "Availability requested as {}. The server has not confirmed it.",
        presence_selection_label(state)
    )
}

#[cfg(windows)]
fn reject_unconfirmed_presence_selection(
    mut signals: PresenceSelectionSignals,
    error: &str,
) -> bool {
    let Some(requested) = *signals.unconfirmed.read() else {
        return false;
    };
    let confirmed = *signals.confirmed.read();
    next_presence_selection_request_generation(signals);
    signals.selected.set(confirmed);
    signals.unconfirmed.set(None);
    signals.pending.set(false);
    signals.status.set(format!(
        "Server rejected availability request for {}: {error}. Availability reverted to {}.",
        presence_selection_label(requested),
        presence_selection_label(confirmed)
    ));
    true
}

#[cfg(windows)]
fn update_presence_selection(
    state: PresenceState,
    signed_in: bool,
    session_generation: Signal<u64>,
    mut signals: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
) {
    if !can_update_presence_selection(
        signed_in,
        *signals.connected.read(),
        *signals.pending.read(),
    ) {
        let status = if !signed_in {
            "Sign in to set availability"
        } else if !*signals.connected.read() {
            "Chat connection is not available to set availability"
        } else {
            "Availability update is already in progress"
        };
        signals.status.set(status.to_string());
        return;
    }
    if state == *signals.selected.read() {
        return;
    }

    let Some(sender) = chat_outbox.read().clone() else {
        reset_presence_selection(
            signals,
            "Chat connection is not available; availability reset to Online",
        );
        return;
    };
    let session = *session_generation.read();
    let request = next_presence_selection_request_generation(signals);
    let (transport_result_tx, transport_result_rx) = oneshot::channel();
    signals.selected.set(state);
    signals.unconfirmed.set(Some(state));
    signals.pending.set(true);
    signals.status.set(format!(
        "Updating availability to {}...",
        presence_selection_label(state)
    ));

    match sender.send(OutgoingChatEvent {
        event: ClientEvent::PresenceUpdate { state },
        transport_result: transport_result_tx,
    }) {
        Ok(()) => {
            spawn(async move {
                let result = match transport_result_rx.await {
                    Ok(result) => result,
                    Err(_) => Err(
                        "chat connection closed before the availability update could be delivered"
                            .to_string(),
                    ),
                };
                if !session_generation_current(session_generation, session)
                    || !presence_selection_request_is_current(signals, request)
                {
                    return;
                }

                signals.pending.set(false);
                match result {
                    // The socket write only proves local transport delivery. The current wire
                    // protocol has no per-request presence acknowledgement.
                    Ok(()) => signals
                        .status
                        .set(unconfirmed_presence_selection_status(state)),
                    Err(error) => reset_presence_selection(
                        signals,
                        &format!(
                            "Could not update availability: {error}. Availability reset to Online"
                        ),
                    ),
                }
            });
        }
        Err(_) => reset_presence_selection(
            signals,
            "Chat connection is not available; availability reset to Online",
        ),
    }
}

#[cfg(all(test, windows))]
mod presence_selection_tests {
    use super::*;

    #[test]
    fn manual_availability_values_map_to_supported_presence_states() {
        assert_eq!(
            presence_state_from_selection_value("online"),
            Some(PresenceState::Online)
        );
        assert_eq!(
            presence_state_from_selection_value("looking_for_game"),
            Some(PresenceState::LookingForGame)
        );
        assert_eq!(
            presence_state_from_selection_value("in_game"),
            Some(PresenceState::InGame)
        );
        assert_eq!(presence_state_from_selection_value("offline"), None);
    }

    #[test]
    fn availability_updates_require_a_signed_in_connected_chat() {
        assert!(can_update_presence_selection(true, true, false));
        assert!(!can_update_presence_selection(false, true, false));
        assert!(!can_update_presence_selection(true, false, false));
        assert!(!can_update_presence_selection(true, true, true));
    }

    #[test]
    fn manual_availability_transport_delivery_is_not_presented_as_server_confirmation() {
        assert_eq!(
            unconfirmed_presence_selection_status(PresenceState::LookingForGame),
            "Availability requested as Looking for game. The server has not confirmed it."
        );
    }

    #[test]
    fn looking_for_game_uses_the_presence_update_wire_event() {
        let event = ClientEvent::PresenceUpdate {
            state: PresenceState::LookingForGame,
        };

        let json = serde_json::to_value(event).unwrap();

        assert_eq!(json["type"], "presence_update");
        assert_eq!(json["payload"]["state"], "looking_for_game");
    }
}

#[cfg(windows)]
fn restore_composer_after_transport_failure(composer: &mut String, body: &str) -> bool {
    if composer.is_empty() {
        composer.push_str(body);
        true
    } else {
        false
    }
}

#[cfg(windows)]
async fn complete_login(
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

#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
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

#[cfg(windows)]
async fn refresh_session(refresh_token: String) -> Result<AuthSession, RefreshSessionError> {
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

#[cfg(windows)]
fn refresh_session_is_invalid(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED
}

#[cfg(windows)]
fn bearer_access() -> &'static Mutex<BearerAccess> {
    BEARER_ACCESS.get_or_init(|| Mutex::new(BearerAccess::Open))
}

#[cfg(windows)]
fn begin_bearer_compatibility_check() {
    if let Ok(mut access) = bearer_access().lock() {
        *access = BearerAccess::AwaitingCompatibility;
    }
}

#[cfg(windows)]
fn allow_bearer_access() {
    if let Ok(mut access) = bearer_access().lock() {
        *access = BearerAccess::Open;
    }
}

#[cfg(windows)]
fn block_bearer_access(reason: String) {
    if let Ok(mut access) = bearer_access().lock() {
        *access = BearerAccess::Blocked(reason);
    }
}

#[cfg(windows)]
fn bearer_access_error(access: &BearerAccess) -> Option<String> {
    match access {
        BearerAccess::Open => None,
        BearerAccess::AwaitingCompatibility => Some(
            "Waiting for server version compatibility confirmation before accessing Agora data"
                .to_string(),
        ),
        BearerAccess::Blocked(reason) => Some(format!(
            "{reason} Update Agora or sign in again before accessing Agora data"
        )),
    }
}

#[cfg(windows)]
fn ensure_bearer_access() -> Result<(), String> {
    let access = bearer_access()
        .lock()
        .map_err(|_| "Agora version compatibility state is unavailable".to_string())?;
    bearer_access_error(&access).map_or(Ok(()), Err)
}

#[cfg(windows)]
async fn logout_session(refresh_token: String) -> Result<bool, String> {
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

#[cfg(windows)]
async fn api_error(response: reqwest::Response, context: &str) -> String {
    let status = response.status();
    match response.json::<ApiError>().await {
        Ok(error) => format!("{context}: {}", error.message),
        Err(_) => format!("{context}: HTTP {status}"),
    }
}

#[cfg(windows)]
async fn search_users_api(session: &AuthSession, query: &str) -> Result<Vec<UserSummary>, String> {
    ensure_bearer_access()?;
    let server_url = server_url()?;
    let response = http_client()?
        .get(format!("{server_url}/users/search"))
        .bearer_auth(&session.access_token)
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|error| format!("Could not search users: {error}"))?;

    json_response::<UserSearchResponse>(response, "User search failed")
        .await
        .map(|response| response.users)
}

#[cfg(windows)]
async fn list_direct_message_threads_api(
    session: &AuthSession,
) -> Result<Vec<DmThreadSummary>, String> {
    api_get::<DmThreadListResponse>(session, "/dm/threads", "Direct message list failed")
        .await
        .map(|response| response.threads)
}

#[cfg(windows)]
async fn create_direct_message_thread_api(
    session: &AuthSession,
    recipient_id: uuid::Uuid,
) -> Result<DmThreadSummary, String> {
    api_post_json::<CreateDmThreadResponse, _>(
        session,
        "/dm/threads",
        &CreateDmThreadRequest { recipient_id },
        "Create direct message failed",
    )
    .await
    .map(|response| response.thread)
}

#[cfg(windows)]
async fn list_direct_message_history_api(
    session: &AuthSession,
    thread_id: uuid::Uuid,
    before: Option<uuid::Uuid>,
) -> Result<DmMessageHistoryResponse, String> {
    ensure_bearer_access()?;
    let server_url = server_url()?;
    let request = http_client()?
        .get(format!("{server_url}/dm/threads/{thread_id}/messages"))
        .bearer_auth(&session.access_token);
    let request = match before {
        Some(before) => {
            request.query(&[("before", before.to_string()), ("limit", "50".to_string())])
        }
        None => request.query(&[("limit", "50")]),
    };
    let response = request
        .send()
        .await
        .map_err(|error| format!("Could not load direct message history: {error}"))?;

    json_response(response, "Direct message history failed").await
}

#[cfg(windows)]
async fn send_direct_message_api(
    session: &AuthSession,
    thread_id: uuid::Uuid,
    body: String,
) -> Result<DmMessage, String> {
    api_post_json::<SendDmMessageResponse, _>(
        session,
        &format!("/dm/threads/{thread_id}/messages"),
        &SendDmMessageRequest { body },
        "Send direct message failed",
    )
    .await
    .map(|response| response.message)
}

#[cfg(windows)]
async fn list_friends_api(session: &AuthSession) -> Result<Vec<FriendshipSummary>, String> {
    api_get::<FriendListResponse>(session, "/friends", "Friend list failed")
        .await
        .map(|response| response.friendships)
}

#[cfg(windows)]
async fn send_friend_request_api(
    session: &AuthSession,
    user_id: &str,
) -> Result<FriendshipSummary, String> {
    api_post_json::<FriendshipResponse, _>(
        session,
        "/friends",
        &FriendRequest {
            user_id: user_id
                .parse()
                .map_err(|error| format!("Invalid user id: {error}"))?,
        },
        "Friend request failed",
    )
    .await
    .map(|response| response.friendship)
}

#[cfg(windows)]
async fn accept_friend_request_api(
    session: &AuthSession,
    friendship_id: &str,
) -> Result<FriendshipSummary, String> {
    api_post_json::<FriendshipResponse, _>(
        session,
        &format!("/friends/{friendship_id}/accept"),
        &(),
        "Accept friend request failed",
    )
    .await
    .map(|response| response.friendship)
}

#[cfg(windows)]
async fn decline_friend_request_api(
    session: &AuthSession,
    friendship_id: &str,
) -> Result<FriendshipSummary, String> {
    api_post_json::<FriendshipResponse, _>(
        session,
        &format!("/friends/{friendship_id}/decline"),
        &(),
        "Decline friend request failed",
    )
    .await
    .map(|response| response.friendship)
}

#[cfg(windows)]
async fn remove_friend_api(session: &AuthSession, user_id: &str) -> Result<bool, String> {
    api_delete::<RemoveFriendResponse>(
        session,
        &format!("/friends/{user_id}"),
        "Remove friend failed",
    )
    .await
    .map(|response| response.removed)
}

#[cfg(windows)]
async fn list_blocks_api(session: &AuthSession) -> Result<Vec<UserSummary>, String> {
    api_get::<BlockListResponse>(session, "/blocks", "Block list failed")
        .await
        .map(|response| response.blocked_users)
}

#[cfg(windows)]
async fn block_user_api(session: &AuthSession, user_id: &str) -> Result<bool, String> {
    api_post_json::<BlockUserResponse, _>(
        session,
        "/blocks",
        &BlockUserRequest {
            user_id: user_id
                .parse()
                .map_err(|error| format!("Invalid user id: {error}"))?,
        },
        "Block user failed",
    )
    .await
    .map(|response| response.blocked)
}

#[cfg(windows)]
async fn unblock_user_api(session: &AuthSession, user_id: &str) -> Result<bool, String> {
    api_delete::<UnblockUserResponse>(
        session,
        &format!("/blocks/{user_id}"),
        "Unblock user failed",
    )
    .await
    .map(|response| response.unblocked)
}

#[cfg(windows)]
async fn create_report_api(
    session: &AuthSession,
    reported_user_id: uuid::Uuid,
    message_id: Option<uuid::Uuid>,
    message_kind: Option<MessageKind>,
    reason: String,
    details: Option<String>,
) -> Result<String, String> {
    api_post_json::<CreateReportResponse, _>(
        session,
        "/reports",
        &CreateReportRequest {
            reported_user_id,
            message_id,
            message_kind,
            reason,
            details,
        },
        "Create report failed",
    )
    .await
    .map(|response| response.id.to_string())
}

#[cfg(windows)]
async fn api_get<T: DeserializeOwned>(
    session: &AuthSession,
    path: &str,
    context: &str,
) -> Result<T, String> {
    ensure_bearer_access()?;
    let server_url = server_url()?;
    let response = http_client()?
        .get(format!("{server_url}{path}"))
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|error| format!("Could not contact Agora server: {error}"))?;
    json_response(response, context).await
}

#[cfg(windows)]
async fn api_post_json<T: DeserializeOwned, B: Serialize + ?Sized>(
    session: &AuthSession,
    path: &str,
    body: &B,
    context: &str,
) -> Result<T, String> {
    ensure_bearer_access()?;
    let server_url = server_url()?;
    let response = http_client()?
        .post(format!("{server_url}{path}"))
        .bearer_auth(&session.access_token)
        .json(body)
        .send()
        .await
        .map_err(|error| format!("Could not contact Agora server: {error}"))?;
    json_response(response, context).await
}

#[cfg(windows)]
async fn api_delete<T: DeserializeOwned>(
    session: &AuthSession,
    path: &str,
    context: &str,
) -> Result<T, String> {
    ensure_bearer_access()?;
    let server_url = server_url()?;
    let response = http_client()?
        .delete(format!("{server_url}{path}"))
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|error| format!("Could not contact Agora server: {error}"))?;
    json_response(response, context).await
}

#[cfg(windows)]
async fn json_response<T: DeserializeOwned>(
    response: reqwest::Response,
    context: &str,
) -> Result<T, String> {
    if response.status().is_success() {
        response
            .json::<T>()
            .await
            .map_err(|error| format!("Could not read Agora response: {error}"))
    } else {
        Err(api_error(response, context).await)
    }
}

#[cfg(windows)]
fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        // Redirects can otherwise replay bearer or refresh credentials to another origin.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Could not configure Agora transport: {error}"))
}

#[cfg(windows)]
fn store_refresh_token(refresh_token: &str) -> Result<(), String> {
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

#[cfg(windows)]
fn load_refresh_token() -> Result<SavedRefreshToken, String> {
    let origin = server_origin()?;
    if origin.is_loopback {
        return Ok(SavedRefreshToken::None);
    }
    match refresh_token_quarantine_entry(&origin)?.get_password() {
        // A marker is deliberately enough. The original token must never be sent again after
        // an unknown refresh outcome, even if the old credential remains in the keyring.
        Ok(_) => return Ok(SavedRefreshToken::Ambiguous),
        Err(keyring::v1::Error::NoEntry) => {}
        Err(error) => {
            return Err(format!("Windows Credential Manager read failed: {error}"));
        }
    }
    match refresh_token_entry(&origin)?.get_password() {
        Ok(token) if token.trim().is_empty() => {
            Err("Windows Credential Manager contains an empty saved refresh token".to_string())
        }
        Ok(token) => Ok(SavedRefreshToken::Scoped(token)),
        Err(keyring::v1::Error::NoEntry) => match legacy_refresh_token_entry()?.get_password() {
            // Legacy credentials have no origin binding, so never send them to a configured URL.
            Ok(_) => Ok(SavedRefreshToken::LegacyCredential),
            Err(keyring::v1::Error::NoEntry) => Ok(SavedRefreshToken::None),
            Err(error) => Err(format!("Windows Credential Manager read failed: {error}")),
        },
        Err(error) => Err(format!("Windows Credential Manager read failed: {error}")),
    }
}

#[cfg(windows)]
fn clear_refresh_token() -> Result<(), String> {
    let origin = server_origin()?;
    if origin.is_loopback {
        return Ok(());
    }
    delete_refresh_token_value(&origin)?;
    clear_refresh_token_quarantine(&origin)
}

#[cfg(windows)]
fn quarantine_refresh_token() -> Result<(), String> {
    let origin = server_origin()?;
    if origin.is_loopback {
        return Ok(());
    }
    quarantine_refresh_token_for_origin(&origin)
}

#[cfg(windows)]
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

#[cfg(windows)]
fn delete_refresh_token_value(origin: &ServerOrigin) -> Result<(), String> {
    match refresh_token_entry(origin)?.delete_credential() {
        Ok(()) | Err(keyring::v1::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("Windows Credential Manager delete failed: {error}")),
    }
}

#[cfg(windows)]
fn clear_refresh_token_quarantine(origin: &ServerOrigin) -> Result<(), String> {
    match refresh_token_quarantine_entry(origin)?.delete_credential() {
        Ok(()) | Err(keyring::v1::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!(
            "Windows Credential Manager safety marker cleanup failed: {error}"
        )),
    }
}

#[cfg(windows)]
fn refresh_token_entry(origin: &ServerOrigin) -> Result<keyring::v1::Entry, String> {
    let user = refresh_token_user(origin);
    keyring::v1::Entry::new(KEYRING_SERVICE, &user)
        .map_err(|error| format!("Windows Credential Manager is unavailable: {error}"))
}

#[cfg(windows)]
fn refresh_token_quarantine_entry(origin: &ServerOrigin) -> Result<keyring::v1::Entry, String> {
    let user = format!(
        "{KEYRING_REFRESH_TOKEN_QUARANTINE_USER}:{}",
        origin.base_url
    );
    keyring::v1::Entry::new(KEYRING_SERVICE, &user)
        .map_err(|error| format!("Windows Credential Manager is unavailable: {error}"))
}

#[cfg(windows)]
fn legacy_refresh_token_entry() -> Result<keyring::v1::Entry, String> {
    keyring::v1::Entry::new(KEYRING_SERVICE, KEYRING_REFRESH_TOKEN_USER)
        .map_err(|error| format!("Windows Credential Manager is unavailable: {error}"))
}

#[cfg(windows)]
fn refresh_token_user(origin: &ServerOrigin) -> String {
    format!("{KEYRING_REFRESH_TOKEN_USER}:{}", origin.base_url)
}

#[cfg(windows)]
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

#[cfg(windows)]
fn server_origin() -> Result<ServerOrigin, String> {
    parse_server_origin(&configured_server_url())
}

#[cfg(windows)]
fn parse_server_origin(value: &str) -> Result<ServerOrigin, String> {
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

#[cfg(windows)]
fn server_url() -> Result<String, String> {
    server_origin().map(|origin| origin.base_url)
}

#[cfg(windows)]
fn websocket_url() -> Result<String, String> {
    server_origin().map(|origin| origin.websocket_url)
}

#[cfg(windows)]
fn configured_local_dev_account_id() -> String {
    local_dev_account_id_or_default(std::env::var("AGORA_DEV_ACCOUNT_ID").ok().as_deref())
}

#[cfg(windows)]
fn local_dev_account_id_or_default(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("alice")
        .to_string()
}

#[cfg(windows)]
fn is_local_server_url() -> bool {
    server_origin().is_ok_and(|origin| origin.is_loopback)
}

#[cfg(windows)]
fn is_loopback_server_url(server_url: &str) -> bool {
    parse_server_origin(server_url).is_ok_and(|origin| origin.is_loopback)
}

#[cfg(windows)]
fn standalone_local_dev_window_enabled() -> bool {
    standalone_local_dev_window_enabled_for(
        std::env::var("AGORA_LOCAL_DEV_WINDOW").ok().as_deref(),
        &configured_server_url(),
    )
}

#[cfg(windows)]
fn standalone_local_dev_window_enabled_for(value: Option<&str>, server_url: &str) -> bool {
    is_loopback_server_url(server_url)
        && value.is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

#[cfg(windows)]
fn login_start_message(
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

#[cfg(windows)]
fn initial_update_status() -> UpdateUiStatus {
    match update::startup_notice() {
        Some(notice) => UpdateUiStatus::Recovery(notice),
        None if update::is_configured() => UpdateUiStatus::Checking { required: None },
        None => UpdateUiStatus::Bootstrap,
    }
}

#[cfg(windows)]
fn check_for_updates(signals: UpdateSignals, required: Option<String>) {
    let mut status = signals.status;
    let mut available = signals.available;
    let mut pending = signals.pending;
    let generation = signals.generation;
    let required = required.or_else(|| status.read().required_reason());

    // AGORA_SERVER_URL is consulted only to disable update traffic for local development.
    // All metadata and package URLs come from the compiled updater configuration.
    if is_local_server_url() {
        available.set(None);
        pending.set(false);
        status.set(UpdateUiStatus::SkippedLoopback);
        return;
    }

    let current_generation = next_update_generation(generation);
    available.set(None);
    pending.set(true);
    status.set(UpdateUiStatus::Checking {
        required: required.clone(),
    });
    spawn(async move {
        let result = update::check_for_update().await;
        if *generation.read() != current_generation {
            return;
        }

        pending.set(false);
        match result {
            Ok(update::CheckResult::Disabled) => status.set(UpdateUiStatus::Bootstrap),
            Ok(update::CheckResult::UpToDate) if required.is_some() => {
                status.set(UpdateUiStatus::Failed {
                    message: "No newer signed update is published for this requirement. Use the manual release link."
                        .to_string(),
                    required,
                });
            }
            Ok(update::CheckResult::UpToDate) => status.set(UpdateUiStatus::UpToDate),
            Ok(update::CheckResult::Available(update)) => {
                let version = update.version().to_string();
                available.set(Some(update));
                status.set(UpdateUiStatus::Available { version, required });
            }
            Err(error) => status.set(UpdateUiStatus::Failed {
                message: format!(
                    "Secure update check failed: {error}. Retry or use the manual release link."
                ),
                required,
            }),
        }
    });
}

#[cfg(windows)]
fn next_update_generation(mut generation: Signal<u64>) -> u64 {
    let next = generation.read().wrapping_add(1);
    generation.set(next);
    next
}

#[cfg(windows)]
fn request_required_update(signals: UpdateSignals, reason: String) {
    check_for_updates(signals, Some(reason));
}

#[cfg(windows)]
fn install_available_update(signals: UpdateSignals) {
    let available_update = signals.available.read().clone();
    if *signals.pending.read() || available_update.is_none() {
        return;
    }

    let update = available_update.expect("checked above");
    let version = update.version().to_string();
    let required = signals.status.read().required_reason();
    let mut status = signals.status;
    let mut pending = signals.pending;
    pending.set(true);
    status.set(UpdateUiStatus::Downloading {
        version: version.clone(),
        required: required.clone(),
    });
    spawn(async move {
        match update::download_update(&update).await {
            Ok(staged) => {
                let install_required = required.clone();
                status.set(UpdateUiStatus::Installing {
                    version,
                    required: install_required.clone(),
                });
                match update::schedule_replace_and_restart(staged) {
                    Ok(()) => std::process::exit(0),
                    Err(error) => {
                        pending.set(false);
                        status.set(UpdateUiStatus::Failed {
                            message: format!(
                                "Update failed: {error}. Use the manual release link if retrying does not help."
                            ),
                            required: install_required,
                        });
                    }
                }
            }
            Err(error) => {
                pending.set(false);
                status.set(UpdateUiStatus::Failed {
                    message: format!(
                        "Update download failed: {error}. Use the manual release link if retrying does not help."
                    ),
                    required,
                });
            }
        }
    });
}

#[cfg(windows)]
fn update_banner(signals: UpdateSignals) -> Element {
    let status = signals.status.read().clone();
    let message = status.message();
    let required = status.required_reason();
    let pending = *signals.pending.read();
    let update_available = signals.available.read().is_some();
    let banner_class = if required.is_some() {
        "update-banner required"
    } else {
        "update-banner"
    };
    let manual_release_url = update::manual_release_url();

    rsx! {
        section { class: "{banner_class}", aria_label: "Agora update status",
            h2 { "Update" }
            p {
                class: "update-status",
                role: "status",
                aria_live: "polite",
                aria_atomic: "true",
                "{message}"
            }
            div { class: "update-actions",
                if update_available {
                    button {
                        class: "secondary-button compact",
                        r#type: "button",
                        disabled: pending,
                        aria_label: "Download verified update and restart Agora",
                        onclick: move |_| install_available_update(signals),
                        "Update and restart"
                    }
                }
                button {
                    class: "secondary-button compact",
                    r#type: "button",
                    disabled: pending,
                    aria_label: "Retry signed Agora update check",
                    onclick: move |_| check_for_updates(signals, required.clone()),
                    "Retry check"
                }
                if let Some(manual_release_url) = manual_release_url {
                    button {
                        class: "secondary-button compact",
                        r#type: "button",
                        aria_label: "Open the Agora manual release page",
                        onclick: move |_| {
                            let _ = webbrowser::open(&manual_release_url);
                        },
                        "Open release page"
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn overlay_tab_class(active: AppTab, tab: AppTab) -> &'static str {
    if active == tab {
        "overlay-tab active"
    } else {
        "overlay-tab"
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn chat_mode_overlay_view(
    desktop: dioxus::desktop::DesktopContext,
    standalone_local_dev: bool,
    active: AppTab,
    mut active_tab: Signal<AppTab>,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    reauth_required: Signal<Option<String>>,
    mut local_dev_account_id: Signal<String>,
    mut remote_login_provider: Signal<RemoteLoginProvider>,
    auth_action_pending: Signal<bool>,
    auth_session: Signal<Option<AuthSession>>,
    login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    composer_body: Signal<String>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    presence_selection: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    chat_send_pending: Signal<bool>,
    updates: UpdateSignals,
    direct_messages: DirectMessageSignals,
    shell_mode: Signal<ShellMode>,
    game_window: Signal<Option<game::GameWindow>>,
    overlay_interactive: Signal<bool>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friendships_load_generation: Signal<u64>,
    friend_search: UserAutocompleteSignals,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    blocks_load_generation: Signal<u64>,
    block_search: UserAutocompleteSignals,
    block_status: Signal<String>,
    report_reason: Signal<String>,
    report_details: Signal<String>,
    report_draft: Signal<Option<ReportDraft>>,
    report_status: Signal<String>,
) -> Element {
    let overlay_tab = if matches!(
        active,
        AppTab::DirectMessages | AppTab::Friends | AppTab::BlockReport
    ) {
        active
    } else {
        AppTab::Global
    };
    let friend_badge = session
        .as_ref()
        .map(|session| incoming_friend_request_count(&friendships.read(), session.user.id))
        .unwrap_or(0);
    let auth_status_text = login_status.read().clone();
    let toolbar_session = session.clone();
    let toolbar_signed_in = toolbar_session.is_some();
    let toolbar_requires_reauth = reauth_required.read().is_some();
    let toolbar_auth_action_pending = *auth_action_pending.read();
    let local_dev_mode = is_local_server_url();
    let local_dev_account_id_value = local_dev_account_id.read().clone();
    let sign_in_account_id = local_dev_account_id_value.clone();
    let remote_login_provider_value = *remote_login_provider.read();
    let remote_login_provider_selection = remote_login_provider_value.selection_value();
    let sign_in_provider = remote_login_provider_value;
    let toolbar_auth_button_disabled = toolbar_auth_action_pending
        || (!toolbar_signed_in && local_dev_mode && local_dev_account_id_value.trim().is_empty());
    let toolbar_auth_button_label = if toolbar_signed_in && !toolbar_requires_reauth {
        "Sign off"
    } else if toolbar_requires_reauth {
        "Sign in again"
    } else {
        "Sign in"
    };
    let local_staff_url = server_url()
        .ok()
        .map(|server_url| format!("{server_url}/staff"));
    let local_access_token = toolbar_session
        .as_ref()
        .map(|session| session.access_token.clone());
    let content = match overlay_tab {
        AppTab::DirectMessages => rsx! {
            {direct_messages_panel(
                session,
                session_generation,
                direct_messages,
                active_tab,
                report_draft,
                report_status,
            )}
        },
        AppTab::Friends => rsx! {
            {friends_panel(
                session,
                session_generation,
                friendships,
                friendships_load_generation,
                friend_search,
                friends_status,
                direct_messages,
                active_tab,
            )}
        },
        AppTab::BlockReport => rsx! {
            {block_report_panel(
                session,
                session_generation,
                chat_messages,
                friendships,
                friendships_load_generation,
                friends_status,
                blocked_users,
                blocks_load_generation,
                block_search,
                block_status,
                report_reason,
                report_details,
                report_draft,
                report_status,
                direct_messages,
            )}
        },
        AppTab::Global => rsx! {
            {overlay_global_chat_panel(
                session,
                active_tab,
                session_generation,
                chat_messages,
                composer_body,
                chat_status,
                presence_selection,
                chat_outbox,
                chat_send_pending,
                report_draft,
                report_status,
            )}
        },
    };
    let escape_desktop = desktop.clone();

    rsx! {
        main { class: "chat-mode-overlay",
            onkeydown: move |event| {
                if !standalone_local_dev && event.key() == Key::Escape {
                    event.prevent_default();
                    close_interaction_or_hide(
                        &escape_desktop,
                        shell_mode,
                        game_window,
                        overlay_interactive,
                    );
                }
            },
            nav { class: "overlay-tabs",
                button {
                    class: overlay_tab_class(overlay_tab, AppTab::Global),
                    onclick: move |_| active_tab.set(AppTab::Global),
                    "Global Chat"
                }
                button {
                    class: overlay_tab_class(overlay_tab, AppTab::DirectMessages),
                    onclick: move |_| active_tab.set(AppTab::DirectMessages),
                    "Messages"
                }
                button {
                    class: overlay_tab_class(overlay_tab, AppTab::Friends),
                    onclick: move |_| active_tab.set(AppTab::Friends),
                    "Friends"
                    if friend_badge > 0 {
                        span { class: "tab-badge", "{friend_badge}" }
                    }
                }
                button {
                    class: overlay_tab_class(overlay_tab, AppTab::BlockReport),
                    onclick: move |_| active_tab.set(AppTab::BlockReport),
                    "Block / Report"
                }
            }
            section { class: "overlay-content",
                section { class: "overlay-toolbar", aria_label: "Account toolbar",
                    div { class: "overlay-toolbar-account",
                        span { class: "toolbar-auth-status", role: "status", aria_live: "polite", aria_atomic: "true",
                            "Authentication: {auth_status_text}"
                        }
                        if !local_dev_mode {
                            label { class: "remote-login-provider",
                                span { "Provider" }
                                select {
                                    value: "{remote_login_provider_selection}",
                                    disabled: toolbar_auth_action_pending || (toolbar_signed_in && !toolbar_requires_reauth),
                                    aria_label: "Sign-in provider",
                                    onchange: move |event| {
                                        if let Some(provider) = RemoteLoginProvider::from_selection_value(&event.value()) {
                                            remote_login_provider.set(provider);
                                        }
                                    },
                                    option { value: "steam", "Steam" }
                                    option { value: "microsoft", "Microsoft" }
                                }
                            }
                        }
                        button {
                            class: "toolbar-auth-button",
                            r#type: "button",
                            disabled: toolbar_auth_button_disabled,
                            onclick: move |_| {
                                if let Some(session) = toolbar_session.clone().filter(|_| !toolbar_requires_reauth) {
                                    sign_out_session(
                                        session,
                                        auth_action_pending,
                                        auth_session,
                                        session_generation,
                                        reauth_required,
                                        login_status,
                                        chat_messages,
                                        deleted_message_ids,
                                        chat_status,
                                        presence_counts,
                                        presence_selection,
                                        chat_outbox,
                                        chat_send_pending,
                                        friendships,
                                        friend_search,
                                        friends_status,
                                        blocked_users,
                                        block_search,
                                        block_status,
                                        report_reason,
                                        report_details,
                                        report_draft,
                                        report_status,
                                        direct_messages,
                                    );
                                } else {
                                    sign_in_session(
                                        auth_action_pending,
                                        auth_session,
                                        session_generation,
                                        reauth_required,
                                        sign_in_account_id.clone(),
                                        sign_in_provider,
                                        login_status,
                                        chat_messages,
                                        deleted_message_ids,
                                        chat_status,
                                        presence_counts,
                                        presence_selection,
                                        chat_outbox,
                                        chat_send_pending,
                                        updates,
                                        friendships,
                                        friendships_load_generation,
                                        friend_search,
                                        friends_status,
                                        blocked_users,
                                        blocks_load_generation,
                                        block_search,
                                        block_status,
                                        report_draft,
                                        direct_messages,
                                    );
                                }
                            },
                            "{toolbar_auth_button_label}"
                        }
                    }
                    if local_dev_mode {
                        div { class: "local-dev-controls",
                            label { class: "local-dev-account",
                                span { "Local fixture" }
                                input {
                                    value: "{local_dev_account_id_value}",
                                    disabled: toolbar_signed_in || toolbar_auth_action_pending,
                                    oninput: move |event| local_dev_account_id.set(event.value())
                                }
                            }
                            if toolbar_signed_in {
                                span { class: "local-dev-note", "Sign off before switching fixtures." }
                            } else {
                                span { class: "local-dev-note", "Roles come from AGORA_DEV_LOGIN_ACCOUNTS." }
                            }
                            if let (Some(access_token), Some(local_staff_url)) =
                                (local_access_token, local_staff_url)
                            {
                                div { class: "local-dev-staff",
                                    input {
                                        class: "local-dev-token",
                                        value: "{access_token}",
                                        readonly: true
                                    }
                                    button {
                                        class: "secondary-button compact",
                                        r#type: "button",
                                        onclick: move |_| {
                                            let _ = webbrowser::open(&local_staff_url);
                                        },
                                        "Open Staff Console"
                                    }
                                }
                                p { class: "local-dev-note", "Sign in as a configured staff fixture, then paste this token into the console." }
                            }
                        }
                    }
                }
                {update_banner(updates)}
                div { class: "overlay-content-body",
                    {content}
                }
            }
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn overlay_global_chat_panel(
    session: Option<AuthSession>,
    active_tab: Signal<AppTab>,
    session_generation: Signal<u64>,
    chat_messages: Signal<Vec<ChatMessage>>,
    mut composer_body: Signal<String>,
    chat_status: Signal<String>,
    presence_selection: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    chat_send_pending: Signal<bool>,
    report_draft: Signal<Option<ReportDraft>>,
    report_status: Signal<String>,
) -> Element {
    let mut composer_focus_generation = use_signal(|| 0_u64);
    let chat_status_text = chat_status.read().clone();
    let composer_text = composer_body.read().clone();
    let connected = chat_outbox.read().is_some();
    let signed_in = session.is_some();
    let current_user_id = session.as_ref().map(|session| session.user.id);
    let chat_send_pending_value = *chat_send_pending.read();
    let composer_focus_generation_value = *composer_focus_generation.read();
    let presence_state = *presence_selection.selected.read();
    let presence_value = presence_selection_value(presence_state);
    let presence_status = presence_selection.status.read().clone();
    let presence_pending = *presence_selection.pending.read();
    let can_update_availability = can_update_presence_selection(
        signed_in,
        *presence_selection.connected.read(),
        presence_pending,
    );
    let can_send =
        signed_in && connected && !chat_send_pending_value && !composer_text.trim().is_empty();
    let mut recent_messages = chat_messages
        .read()
        .iter()
        .rev()
        .take(10)
        .cloned()
        .collect::<Vec<_>>();
    recent_messages.reverse();
    let latest_message_id = recent_messages.last().map(|message| message.id);

    rsx! {
        div { class: "overlay-global-panel",
            header { class: "overlay-panel-heading",
                div {
                    h2 { "Global" }
                    p { "Last 10 messages" }
                }
                div { class: "overlay-panel-actions",
                    span { class: "chat-status-inline", "{chat_status_text}" }
                }
                div { class: "availability-control",
                    label { class: "availability-label",
                        span { "Availability" }
                        select {
                            value: "{presence_value}",
                            disabled: !can_update_availability,
                            aria_label: "Availability",
                            onchange: move |event| {
                                if let Some(state) = presence_state_from_selection_value(&event.value()) {
                                    update_presence_selection(
                                        state,
                                        signed_in,
                                        session_generation,
                                        presence_selection,
                                        chat_outbox,
                                    );
                                }
                            },
                            option { value: "online", "Online" }
                            option { value: "looking_for_game", "Looking for game" }
                            option { value: "in_game", "In game" }
                        }
                    }
                    span {
                        class: "availability-status",
                        role: "status",
                        aria_live: "polite",
                        aria_atomic: "true",
                        "{presence_status}"
                    }
                }
            }
            div { class: "overlay-message-list",
                if recent_messages.is_empty() {
                    div { class: "overlay-empty",
                        h2 { "Global Chat" }
                        p { "{chat_status_text}" }
                        if !signed_in {
                            p { "Use Sign in to join chat." }
                        }
                    }
                } else {
                    for message in recent_messages {
                        {global_message_row(message, current_user_id, active_tab, report_draft, report_status)}
                    }
                    if let Some(latest_message_id) = latest_message_id {
                        {message_scroll_anchor(format!("global-{latest_message_id}"))}
                    }
                }
            }
            form {
                class: "overlay-composer",
                onsubmit: move |event| {
                    event.prevent_default();
                    if can_send {
                        composer_focus_generation.set(
                            composer_focus_generation_value.wrapping_add(1),
                        );
                        send_pending_global_message(
                            session_generation,
                            composer_body,
                            chat_status,
                            chat_outbox,
                            chat_send_pending,
                        );
                    }
                },
                input {
                    key: "global-composer-{composer_focus_generation_value}",
                    autofocus: true,
                    onmounted: move |event| async move {
                        let _ = event.set_focus(true).await;
                    },
                    placeholder: if signed_in { "Type a global message" } else { "Sign in to chat" },
                    value: "{composer_text}",
                    disabled: !signed_in || !connected,
                    oninput: move |event| composer_body.set(event.value())
                }
                button {
                    r#type: "submit",
                    disabled: !can_send,
                    if chat_send_pending_value { "Sending..." } else { "Send" }
                }
            }
        }
    }
}

#[cfg(windows)]
fn message_scroll_anchor(key: String) -> Element {
    rsx! {
        div {
            key: "{key}",
            class: "message-scroll-anchor",
            aria_hidden: "true",
            onmounted: move |event| async move {
                let _ = event.scroll_to(ScrollBehavior::Instant).await;
            }
        }
    }
}

#[cfg(windows)]
fn global_message_row(
    message: ChatMessage,
    current_user_id: Option<uuid::Uuid>,
    mut active_tab: Signal<AppTab>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_status: Signal<String>,
) -> Element {
    let message_id = message.id;
    let can_report = current_user_id.is_some_and(|user_id| user_id != message.author.id);
    let report_message = message.clone();

    rsx! {
        article { key: "{message_id}", class: "overlay-message",
            div { class: "message-meta",
                strong { "{message.author.display_name}" }
                time { "{message.created_at}" }
                div { class: "message-actions",
                    button {
                        class: "message-action",
                        r#type: "button",
                        disabled: !can_report,
                        onclick: move |_| {
                            let draft = global_message_report_draft(&report_message);
                            report_status.set(report_draft_status(&draft));
                            report_draft.set(Some(draft));
                            active_tab.set(AppTab::BlockReport);
                        },
                        "Report"
                    }
                }
            }
            p { "{message.body}" }
        }
    }
}

#[cfg(windows)]
fn friend_sections(
    friendships: &[FriendshipSummary],
    current_user_id: uuid::Uuid,
) -> FriendSections {
    let mut sections = FriendSections::default();
    for friendship in friendships {
        match friendship.status {
            FriendshipStatus::Accepted => sections.accepted.push(friendship.clone()),
            FriendshipStatus::Pending if friendship.addressee.id == current_user_id => {
                sections.incoming.push(friendship.clone());
            }
            FriendshipStatus::Pending => sections.outgoing.push(friendship.clone()),
            FriendshipStatus::Declined | FriendshipStatus::Removed => {
                sections.inactive.push(friendship.clone());
            }
        }
    }
    sections
}

#[cfg(windows)]
fn incoming_friend_request_count(
    friendships: &[FriendshipSummary],
    current_user_id: uuid::Uuid,
) -> usize {
    friendships
        .iter()
        .filter(|friendship| {
            friendship.status == FriendshipStatus::Pending
                && friendship.addressee.id == current_user_id
        })
        .count()
}

#[cfg(all(test, windows))]
mod relationship_ui_tests {
    use super::*;

    fn user(id: u128, display_name: &str) -> UserSummary {
        UserSummary {
            id: uuid::Uuid::from_u128(id),
            display_name: display_name.to_string(),
            avatar_url: None,
        }
    }

    fn friendship(
        status: FriendshipStatus,
        requester: UserSummary,
        addressee: UserSummary,
    ) -> FriendshipSummary {
        FriendshipSummary {
            id: uuid::Uuid::new_v4(),
            requester,
            addressee,
            status,
            created_at: "2026-09-08 18:15:09".to_string(),
            updated_at: "2026-09-08 18:15:09".to_string(),
        }
    }

    #[test]
    fn splits_friendships_into_dioxus_sections() {
        let me = user(1, "Me");
        let incoming = friendship(FriendshipStatus::Pending, user(2, "Alice"), me.clone());
        let outgoing = friendship(FriendshipStatus::Pending, me.clone(), user(3, "Bob"));
        let accepted = friendship(FriendshipStatus::Accepted, me.clone(), user(4, "Cora"));
        let removed = friendship(FriendshipStatus::Removed, me.clone(), user(5, "Dion"));
        let friendships = vec![
            incoming.clone(),
            outgoing.clone(),
            accepted.clone(),
            removed.clone(),
        ];

        let sections = friend_sections(&friendships, me.id);

        assert_eq!(sections.incoming, vec![incoming]);
        assert_eq!(sections.outgoing, vec![outgoing]);
        assert_eq!(sections.accepted, vec![accepted]);
        assert_eq!(sections.inactive, vec![removed]);
        assert_eq!(incoming_friend_request_count(&friendships, me.id), 1);
    }

    #[test]
    fn autocomplete_waits_for_two_non_whitespace_characters() {
        assert!(!user_autocomplete_query_ready(""));
        assert!(!user_autocomplete_query_ready("   "));
        assert!(!user_autocomplete_query_ready(" a "));
        assert!(user_autocomplete_query_ready(" al "));
        assert!(user_autocomplete_query_ready("\u{00c5}\u{00df}"));
    }

    #[test]
    fn autocomplete_keyboard_navigation_wraps_and_recovers_from_stale_indices() {
        assert_eq!(next_user_autocomplete_active_index(0, None, true), None);
        assert_eq!(next_user_autocomplete_active_index(3, None, true), Some(0));
        assert_eq!(next_user_autocomplete_active_index(3, None, false), Some(2));
        assert_eq!(
            next_user_autocomplete_active_index(3, Some(2), true),
            Some(0)
        );
        assert_eq!(
            next_user_autocomplete_active_index(3, Some(0), false),
            Some(2)
        );
        assert_eq!(
            next_user_autocomplete_active_index(3, Some(9), true),
            Some(0)
        );
        assert_eq!(
            next_user_autocomplete_active_index(3, Some(9), false),
            Some(2)
        );
    }

    #[test]
    fn global_message_report_drafts_keep_message_context() {
        let message = ChatMessage {
            id: uuid::Uuid::new_v4(),
            author: user(2, "Alice"),
            body: "spam".to_string(),
            created_at: "2026-09-08 18:15:09".to_string(),
        };

        let draft = global_message_report_draft(&message);

        assert_eq!(draft.target, message.author);
        assert_eq!(draft.message_id, Some(message.id));
        assert_eq!(draft.message_kind, Some(MessageKind::Global));
        assert_eq!(draft.message_preview.as_deref(), Some("spam"));
    }

    #[test]
    fn removes_blocked_user_messages_from_cache() {
        let blocked = user(2, "Alice");
        let other = user(3, "Bob");
        let retained_message = ChatMessage {
            id: uuid::Uuid::new_v4(),
            author: other,
            body: "hello".to_string(),
            created_at: "2026-09-08 18:15:09".to_string(),
        };
        let mut messages = vec![
            ChatMessage {
                id: uuid::Uuid::new_v4(),
                author: blocked.clone(),
                body: "blocked".to_string(),
                created_at: "2026-09-08 18:15:09".to_string(),
            },
            retained_message.clone(),
        ];

        remove_blocked_user_messages(&mut messages, blocked.id);

        assert_eq!(messages, vec![retained_message]);
    }

    #[test]
    fn removes_deleted_global_message_from_cache() {
        let deleted_id = uuid::Uuid::new_v4();
        let retained_message = ChatMessage {
            id: uuid::Uuid::new_v4(),
            author: user(3, "Bob"),
            body: "hello".to_string(),
            created_at: "2026-09-08 18:15:09".to_string(),
        };
        let mut messages = vec![
            ChatMessage {
                id: deleted_id,
                author: user(2, "Alice"),
                body: "deleted".to_string(),
                created_at: "2026-09-08 18:15:09".to_string(),
            },
            retained_message.clone(),
        ];

        remove_message_by_id(&mut messages, deleted_id);

        assert_eq!(messages, vec![retained_message]);
    }

    #[test]
    fn remembers_deleted_message_ids_once() {
        let deleted_id = uuid::Uuid::new_v4();
        let mut deleted = Vec::new();

        remember_deleted_message(&mut deleted, deleted_id);
        remember_deleted_message(&mut deleted, deleted_id);

        assert_eq!(deleted, vec![deleted_id]);
    }

    #[test]
    fn caps_deleted_message_tombstone_cache() {
        let mut deleted = Vec::new();
        for index in 0..(DELETED_MESSAGE_CACHE_LIMIT + 1) {
            remember_deleted_message(&mut deleted, uuid::Uuid::from_u128(index as u128));
        }

        assert_eq!(deleted.len(), DELETED_MESSAGE_CACHE_LIMIT);
        assert!(!deleted.contains(&uuid::Uuid::from_u128(0)));
    }

    #[test]
    fn identifies_terminal_chat_errors() {
        assert!(terminal_chat_error("Chat session ended; sign in again"));
        assert!(terminal_chat_error("Account status changed; sign in again"));
        assert!(!terminal_chat_error("Could not load recent chat history"));
    }

    #[test]
    fn defaults_empty_local_fixture_account_ids_to_alice() {
        assert_eq!(local_dev_account_id_or_default(None), "alice");
        assert_eq!(local_dev_account_id_or_default(Some("  ")), "alice");
        assert_eq!(
            local_dev_account_id_or_default(Some("  moderator  ")),
            "moderator"
        );
    }

    #[test]
    fn only_loopback_urls_enable_local_fixture_mode() {
        assert!(is_loopback_server_url("http://localhost"));
        assert!(is_loopback_server_url("http://LOCALHOST:8080"));
        assert!(is_loopback_server_url("http://[::1]:8080"));
        assert!(!is_loopback_server_url(
            "http://localhost:password@remote.example"
        ));
        assert!(!is_loopback_server_url("https://agora.example"));
    }

    #[test]
    fn standalone_local_dev_window_requires_explicit_loopback_opt_in() {
        assert!(standalone_local_dev_window_enabled_for(
            Some("true"),
            "http://localhost"
        ));
        assert!(standalone_local_dev_window_enabled_for(
            Some("1"),
            "http://[::1]:8080"
        ));
        assert!(!standalone_local_dev_window_enabled_for(
            Some("true"),
            "https://agora.example"
        ));
        assert!(!standalone_local_dev_window_enabled_for(
            Some("false"),
            "http://localhost"
        ));
        assert_eq!(initial_shell_mode(true), ShellMode::OverlayInteractive);
        assert_eq!(initial_shell_mode(false), ShellMode::Tray);
    }

    #[test]
    fn server_origins_require_https_away_from_loopback_and_normalize() {
        let remote = parse_server_origin("https://AGORA.example:443/").unwrap();
        assert_eq!(remote.base_url, "https://agora.example");
        assert_eq!(remote.websocket_url, "wss://agora.example/ws");
        assert!(!remote.is_loopback);

        let local = parse_server_origin("http://127.0.0.2:8080/").unwrap();
        assert_eq!(local.base_url, "http://127.0.0.2:8080");
        assert_eq!(local.websocket_url, "ws://127.0.0.2:8080/ws");
        assert!(local.is_loopback);

        for invalid in [
            "http://agora.example",
            "https://user:password@agora.example",
            "https://@agora.example",
            "https://agora.example/api",
            "https://agora.example?next=https://elsewhere.example",
            "https://agora.example/#fragment",
            "wss://agora.example",
            "not a URL",
        ] {
            assert!(parse_server_origin(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn refresh_credentials_are_scoped_to_the_normalized_origin() {
        let canonical = parse_server_origin("https://agora.example").unwrap();
        let equivalent = parse_server_origin("https://AGORA.example:443/").unwrap();
        let other = parse_server_origin("https://staging.agora.example").unwrap();

        assert_eq!(
            refresh_token_user(&canonical),
            "refresh-token:https://agora.example"
        );
        assert_eq!(
            refresh_token_user(&canonical),
            refresh_token_user(&equivalent)
        );
        assert_ne!(refresh_token_user(&canonical), refresh_token_user(&other));
    }

    #[test]
    fn only_unauthorized_refresh_responses_invalidate_a_saved_session() {
        assert!(refresh_session_is_invalid(
            reqwest::StatusCode::UNAUTHORIZED
        ));
        assert!(!refresh_session_is_invalid(
            reqwest::StatusCode::BAD_REQUEST
        ));
        assert!(!refresh_session_is_invalid(
            reqwest::StatusCode::TOO_MANY_REQUESTS
        ));
        assert!(!refresh_session_is_invalid(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
    }

    #[test]
    fn bearer_requests_wait_for_compatibility_and_stop_after_an_update_requirement() {
        assert_eq!(bearer_access_error(&BearerAccess::Open), None);
        assert_eq!(
            bearer_access_error(&BearerAccess::AwaitingCompatibility),
            Some(
                "Waiting for server version compatibility confirmation before accessing Agora data"
                    .to_string()
            )
        );
        assert_eq!(
            bearer_access_error(&BearerAccess::Blocked("Update required.".to_string())),
            Some(
                "Update required. Update Agora or sign in again before accessing Agora data"
                    .to_string()
            )
        );
    }

    #[test]
    fn refresh_retry_backoff_is_capped() {
        assert_eq!(session_refresh_retry_delay(1), 2);
        assert_eq!(session_refresh_retry_delay(2), 4);
        assert_eq!(session_refresh_retry_delay(3), 8);
        assert_eq!(session_refresh_retry_delay(4), 16);
        assert_eq!(session_refresh_retry_delay(5), 30);
        assert_eq!(session_refresh_retry_delay(u32::MAX), 30);
    }

    #[test]
    fn failed_transport_restores_only_an_untouched_global_draft() {
        let mut empty_composer = String::new();
        assert!(restore_composer_after_transport_failure(
            &mut empty_composer,
            "hello"
        ));
        assert_eq!(empty_composer, "hello");

        let mut newer_composer = "newer draft".to_string();
        assert!(!restore_composer_after_transport_failure(
            &mut newer_composer,
            "hello"
        ));
        assert_eq!(newer_composer, "newer draft");
    }

    #[test]
    fn direct_message_report_drafts_keep_message_context() {
        let message = DmMessage {
            id: uuid::Uuid::from_u128(12),
            thread_id: uuid::Uuid::from_u128(10),
            author: user(2, "Alice"),
            body: "harassment".to_string(),
            created_at: "2026-09-19 12:00:00+00".to_string(),
        };

        let draft = direct_message_report_draft(&message);

        assert_eq!(draft.target, message.author);
        assert_eq!(draft.message_id, Some(message.id));
        assert_eq!(draft.message_kind, Some(MessageKind::Dm));
        assert_eq!(draft.message_preview.as_deref(), Some("harassment"));
    }

    #[test]
    fn direct_message_history_merges_realtime_and_paginated_messages_once() {
        let thread_id = uuid::Uuid::from_u128(10);
        let older = DmMessage {
            id: uuid::Uuid::from_u128(11),
            thread_id,
            author: user(2, "Alice"),
            body: "older".to_string(),
            created_at: "2026-09-19 12:00:00+00".to_string(),
        };
        let newer = DmMessage {
            id: uuid::Uuid::from_u128(12),
            thread_id,
            author: user(1, "Me"),
            body: "newer".to_string(),
            created_at: "2026-09-19 12:01:00+00".to_string(),
        };
        let mut messages = vec![newer.clone()];

        merge_direct_messages(&mut messages, vec![older.clone(), newer.clone()]);

        assert_eq!(messages, vec![older, newer]);
    }

    #[test]
    fn direct_thread_updates_replace_existing_threads_and_keep_recent_first() {
        let first_id = uuid::Uuid::from_u128(10);
        let second_id = uuid::Uuid::from_u128(11);
        let mut threads = vec![DmThreadSummary {
            id: first_id,
            other_user: user(2, "Alice"),
            created_at: "2026-09-19 12:00:00+00".to_string(),
            last_message_at: None,
        }];

        upsert_direct_message_thread(
            &mut threads,
            DmThreadSummary {
                id: second_id,
                other_user: user(3, "Bob"),
                created_at: "2026-09-19 12:01:00+00".to_string(),
                last_message_at: None,
            },
        );
        upsert_direct_message_thread(
            &mut threads,
            DmThreadSummary {
                id: first_id,
                other_user: user(2, "Alice"),
                created_at: "2026-09-19 12:00:00+00".to_string(),
                last_message_at: Some("2026-09-19 12:02:00+00".to_string()),
            },
        );

        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, first_id);
        assert_eq!(
            threads[0].last_message_at.as_deref(),
            Some("2026-09-19 12:02:00+00")
        );
    }
}

#[cfg(windows)]
fn user_autocomplete_option_id(listbox_id: &str, user: &UserSummary) -> String {
    format!("{listbox_id}-{}", user.id)
}

#[cfg(windows)]
fn user_autocomplete_option(
    listbox_id: &'static str,
    index: usize,
    user: UserSummary,
    active: bool,
    signals: UserAutocompleteSignals,
) -> Element {
    let option_id = user_autocomplete_option_id(listbox_id, &user);
    let user_id = user.id.to_string();
    let display_name = user.display_name.clone();

    rsx! {
        div {
            key: "{user_id}",
            id: "{option_id}",
            class: if active { "user-autocomplete-option active" } else { "user-autocomplete-option" },
            role: "option",
            aria_selected: active,
            onmousedown: move |event| {
                event.prevent_default();
                select_user_autocomplete_option(signals, index);
            },
            strong { "{display_name}" }
            span { "{user_id}" }
        }
    }
}

#[cfg(windows)]
fn user_autocomplete_combobox(
    listbox_id: &'static str,
    label: &'static str,
    placeholder: &'static str,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    signals: UserAutocompleteSignals,
) -> Element {
    let query = signals.query.read().clone();
    let results = signals.results.read().clone();
    let selected = signals.selected.read().is_some();
    let pending = *signals.pending.read();
    let expanded = *signals.expanded.read();
    let status = signals.status.read().clone();
    let active_index = (*signals.active_index.read()).filter(|index| *index < results.len());
    let popup_open = expanded && user_autocomplete_query_ready(&query) && !selected;
    let active_option_id = active_index
        .and_then(|index| results.get(index))
        .map(|user| user_autocomplete_option_id(listbox_id, user))
        .unwrap_or_default();
    let input_session = session.clone();
    let can_search = session.is_some();

    rsx! {
        div { class: "user-autocomplete",
            label { class: "sr-only", "{label}" }
            input {
                class: "user-autocomplete-input",
                placeholder: "{placeholder}",
                value: "{query}",
                disabled: !can_search,
                role: "combobox",
                aria_label: "{label}",
                aria_autocomplete: "list",
                aria_controls: "{listbox_id}",
                aria_expanded: popup_open,
                aria_activedescendant: "{active_option_id}",
                aria_busy: pending,
                autocomplete: "off",
                onfocus: move |_| show_user_autocomplete(signals),
                onblur: move |_| dismiss_user_autocomplete(signals),
                oninput: move |event| {
                    update_user_autocomplete_query(
                        input_session.clone(),
                        session_generation,
                        signals,
                        event.value(),
                    );
                },
                onkeydown: move |event| match event.key() {
                    Key::ArrowDown => {
                        event.prevent_default();
                        move_user_autocomplete_active_option(signals, true);
                    }
                    Key::ArrowUp => {
                        event.prevent_default();
                        move_user_autocomplete_active_option(signals, false);
                    }
                    Key::Enter if popup_open => {
                        event.prevent_default();
                        if let Some(index) = active_index {
                            select_user_autocomplete_option(signals, index);
                        }
                    }
                    Key::Escape if popup_open => {
                        event.prevent_default();
                        event.stop_propagation();
                        dismiss_user_autocomplete(signals);
                    }
                    _ => {}
                }
            }
            if popup_open {
                div {
                    id: "{listbox_id}",
                    class: "user-autocomplete-menu",
                    role: "listbox",
                    if pending {
                        div { class: "user-autocomplete-empty", "Searching..." }
                    } else if results.is_empty() {
                        div { class: "user-autocomplete-empty", "{status}" }
                    } else {
                        for (index, user) in results.into_iter().enumerate() {
                            {user_autocomplete_option(
                                listbox_id,
                                index,
                                user,
                                active_index == Some(index),
                                signals,
                            )}
                        }
                    }
                }
            } else if !selected && !status.is_empty() {
                span { class: "user-autocomplete-hint", "{status}" }
            }
            span {
                class: "sr-only",
                role: "status",
                aria_live: "polite",
                "{status}"
            }
        }
    }
}

#[cfg(windows)]
fn direct_messages_panel(
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
    active_tab: Signal<AppTab>,
    report_draft: Signal<Option<ReportDraft>>,
    report_status: Signal<String>,
) -> Element {
    let mut composer_focus_generation = use_signal(|| 0_u64);
    let status = signals.status.read().clone();
    let threads = signals.threads.read().clone();
    let selected_thread_id = *signals.selected_thread_id.read();
    let selected_thread = selected_thread_id.and_then(|thread_id| {
        threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .cloned()
    });
    let messages = signals.messages.read().clone();
    let latest_message_id = messages.last().map(|message| message.id);
    let next_before_message_id = *signals.next_before_message_id.read();
    let search_selected_user = signals.search.selected.read().clone();
    let composer_body = signals.composer_body.read().clone();
    let composer_focus_generation_value = *composer_focus_generation.read();
    let threads_pending = *signals.threads_pending.read();
    let history_pending = *signals.history_pending.read();
    let send_pending = *signals.send_pending.read();
    let can_use = session.is_some();
    let can_send = can_use
        && selected_thread_id.is_some()
        && !composer_body.trim().is_empty()
        && !send_pending;
    let current_user_id = session.as_ref().map(|session| session.user.id);
    let refresh_session = session.clone();
    let load_older_session = session.clone();
    let refresh_selected_session = session.clone();
    let send_session = session.clone();
    let selected_thread_name = selected_thread
        .as_ref()
        .map(|thread| thread.other_user.display_name.clone())
        .unwrap_or_else(|| "Select a conversation".to_string());

    rsx! {
        div { class: "direct-message-layout",
            section { class: "tool-card dm-start-card",
                div { class: "section-heading",
                    div {
                        h2 { "Direct Messages" }
                        p { "Private 1:1 conversations with Agora users." }
                    }
                    button {
                        class: "secondary-button compact",
                        r#type: "button",
                        disabled: !can_use || threads_pending,
                        aria_label: "Refresh direct message threads",
                        onclick: move |_| {
                            if let Some(session) = refresh_session.clone() {
                                load_direct_message_threads(session, session_generation, signals);
                            }
                        },
                        if threads_pending { "Refreshing..." } else { "Refresh" }
                    }
                }
                {user_autocomplete_combobox(
                    "dm-user-suggestions",
                    "Search users to start a direct message",
                    "Search users to message",
                    session.clone(),
                    session_generation,
                    signals.search,
                )}
                span {
                    class: "panel-status",
                    role: "status",
                    aria_live: "polite",
                    "{status}"
                }
                if let Some(user) = search_selected_user {
                    {direct_message_selected_user(
                        user,
                        session.clone(),
                        session_generation,
                        signals,
                        active_tab,
                    )}
                }
            }

            div { class: "dm-workspace",
                aside { class: "dm-thread-pane", aria_label: "Direct message threads",
                    h3 { "Conversations" }
                    if threads.is_empty() {
                        p { class: "muted-copy", "No direct message threads yet." }
                    } else {
                        div { class: "dm-thread-list",
                            for thread in threads {
                                {direct_message_thread_row(
                                    thread,
                                    selected_thread_id,
                                    session.clone(),
                                    session_generation,
                                    signals,
                                )}
                            }
                        }
                    }
                }

                section { class: "dm-conversation-pane",
                    header { class: "section-heading",
                        div {
                            h3 { "{selected_thread_name}" }
                            if selected_thread.is_some() {
                                p { "Direct message history" }
                            } else {
                                p { "Choose a thread or start a new conversation." }
                            }
                        }
                        if let (Some(session), Some(thread_id)) =
                            (refresh_selected_session.clone(), selected_thread_id)
                        {
                            button {
                                class: "secondary-button compact",
                                r#type: "button",
                                disabled: history_pending,
                                aria_label: "Refresh direct message history",
                                onclick: move |_| {
                                    open_direct_message_thread(
                                        session.clone(),
                                        session_generation,
                                        signals,
                                        thread_id,
                                    );
                                },
                                "Refresh"
                            }
                        }
                    }

                    if let (Some(session), Some(thread_id), Some(before)) =
                        (load_older_session.clone(), selected_thread_id, next_before_message_id)
                    {
                        button {
                            class: "secondary-button compact dm-load-older",
                            r#type: "button",
                            disabled: history_pending,
                            onclick: move |_| {
                                load_direct_message_history_page(
                                    session.clone(),
                                    session_generation,
                                    signals,
                                    thread_id,
                                    Some(before),
                                    false,
                                );
                            },
                            if history_pending { "Loading history..." } else { "Load older messages" }
                        }
                    }

                    div {
                        class: "dm-message-list",
                        aria_label: "Direct message history",
                        aria_live: "polite",
                        if selected_thread.is_some() && messages.is_empty() && !history_pending {
                            p { class: "muted-copy", "No messages in this conversation yet." }
                        } else if selected_thread.is_none() {
                            p { class: "muted-copy", "Your selected conversation will appear here." }
                        } else {
                            for message in messages {
                                {direct_message_row(
                                    message,
                                    current_user_id,
                                    active_tab,
                                    report_draft,
                                    report_status,
                                )}
                            }
                            if let Some(latest_message_id) = latest_message_id {
                                {message_scroll_anchor(format!("dm-{latest_message_id}"))}
                            }
                        }
                    }

                    form {
                        class: "overlay-composer dm-composer",
                        onsubmit: move |event| {
                            event.prevent_default();
                            if let (Some(session), Some(thread_id)) =
                                (send_session.clone(), selected_thread_id)
                            {
                                if can_send {
                                    composer_focus_generation.set(
                                        composer_focus_generation_value.wrapping_add(1),
                                    );
                                    send_direct_message(
                                        session,
                                        session_generation,
                                        signals,
                                        thread_id,
                                    );
                                }
                            }
                        },
                        label { class: "sr-only", "Direct message" }
                        input {
                            key: "dm-composer-{composer_focus_generation_value}",
                            placeholder: if selected_thread.is_some() {
                                "Type a direct message"
                            } else {
                                "Select a conversation first"
                            },
                            value: "{composer_body}",
                            disabled: !can_use || selected_thread.is_none(),
                            aria_label: "Direct message",
                            onmounted: move |event| async move {
                                if composer_focus_generation_value != 0 {
                                    let _ = event.set_focus(true).await;
                                }
                            },
                            oninput: move |event| signals.composer_body.set(event.value())
                        }
                        button {
                            r#type: "submit",
                            disabled: !can_send,
                            if send_pending { "Sending..." } else { "Send" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn direct_message_selected_user(
    user: UserSummary,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    signals: DirectMessageSignals,
    active_tab: Signal<AppTab>,
) -> Element {
    let display_name = user.display_name.clone();
    let creating = *signals.create_pending.read();

    rsx! {
        div { class: "user-autocomplete-selected",
            div { class: "user-autocomplete-selected-user",
                span { "Selected user" }
                strong { "{display_name}" }
            }
            div { class: "user-autocomplete-actions",
                button {
                    class: "secondary-button compact",
                    r#type: "button",
                    disabled: session.is_none() || creating,
                    aria_label: "Start direct message with {display_name}",
                    onclick: move |_| {
                        if let Some(session) = session.clone() {
                            create_direct_message_thread(
                                session,
                                session_generation,
                                signals,
                                active_tab,
                                user.clone(),
                            );
                        }
                    },
                    if creating { "Opening..." } else { "Message" }
                }
            }
        }
    }
}

#[cfg(windows)]
fn direct_message_thread_row(
    thread: DmThreadSummary,
    selected_thread_id: Option<uuid::Uuid>,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    signals: DirectMessageSignals,
) -> Element {
    let thread_id = thread.id;
    let active = selected_thread_id == Some(thread_id);
    let display_name = thread.other_user.display_name.clone();
    let timestamp = thread
        .last_message_at
        .clone()
        .unwrap_or_else(|| thread.created_at.clone());

    rsx! {
        button {
            class: if active { "dm-thread active" } else { "dm-thread" },
            r#type: "button",
            disabled: session.is_none(),
            aria_label: "Open direct message with {display_name}",
            onclick: move |_| {
                if let Some(session) = session.clone() {
                    open_direct_message_thread(session, session_generation, signals, thread_id);
                }
            },
            strong { "{display_name}" }
            span { "{timestamp}" }
        }
    }
}

#[cfg(windows)]
fn direct_message_row(
    message: DmMessage,
    current_user_id: Option<uuid::Uuid>,
    mut active_tab: Signal<AppTab>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_status: Signal<String>,
) -> Element {
    let message_id = message.id;
    let can_report = current_user_id.is_some_and(|user_id| user_id != message.author.id);
    let report_message = message.clone();

    rsx! {
        article { key: "dm-{message_id}", class: "overlay-message dm-message",
            div { class: "message-meta",
                strong { "{message.author.display_name}" }
                time { "{message.created_at}" }
                div { class: "message-actions",
                    button {
                        class: "message-action",
                        r#type: "button",
                        disabled: !can_report,
                        aria_label: "Report direct message from {message.author.display_name}",
                        onclick: move |_| {
                            let draft = direct_message_report_draft(&report_message);
                            report_status.set(report_draft_status(&draft));
                            report_draft.set(Some(draft));
                            active_tab.set(AppTab::BlockReport);
                        },
                        "Report"
                    }
                }
            }
            p { "{message.body}" }
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn friends_panel(
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friendships_load_generation: Signal<u64>,
    friend_search: UserAutocompleteSignals,
    friends_status: Signal<String>,
    direct_messages: DirectMessageSignals,
    active_tab: Signal<AppTab>,
) -> Element {
    let status = friends_status.read().clone();
    let search_selected_user = friend_search.selected.read().clone();
    let friendship_items = friendships.read().clone();
    let can_use = session.is_some();
    let current_user = session.as_ref().map(|session| session.user.clone());
    let (accepted, incoming, outgoing, inactive) = current_user
        .as_ref()
        .map(|current_user| friend_sections(&friendship_items, current_user.id))
        .map(|sections| {
            (
                sections.accepted,
                sections.incoming,
                sections.outgoing,
                sections.inactive,
            )
        })
        .unwrap_or_default();
    let accepted_count = accepted.len();
    let incoming_count = incoming.len();
    let outgoing_count = outgoing.len();
    let refresh_session = session.clone();

    rsx! {
        div { class: "relationship-layout",
            section { class: "tool-card",
                div { class: "section-heading",
                    h2 { "Friends" }
                    button {
                        class: "secondary-button compact",
                        disabled: !can_use,
                        onclick: move |_| {
                            if let Some(session) = refresh_session.clone() {
                                load_friends(
                                    session,
                                    session_generation,
                                    friendships_load_generation,
                                    friendships,
                                    friends_status,
                                );
                            }
                        },
                        "Refresh"
                    }
                }
                p { "Search for another Agora user, send requests, and manage pending invites." }
                div { class: "friend-summary-grid",
                    span { class: "friend-stat", strong { "{accepted_count}" } " Friends" }
                    span { class: "friend-stat", strong { "{incoming_count}" } " Incoming" }
                    span { class: "friend-stat", strong { "{outgoing_count}" } " Sent" }
                }
                {user_autocomplete_combobox(
                    "friend-user-suggestions",
                    "Search users to add as friends",
                    "Search users",
                    session.clone(),
                    session_generation,
                    friend_search,
                )}
                span { class: "panel-status", "{status}" }
                if let Some(user) = search_selected_user {
                    {friend_search_selected_user(
                        user,
                        session.clone(),
                        session_generation,
                        friendships_load_generation,
                        friendships,
                        friends_status,
                        direct_messages,
                        active_tab,
                    )}
                }
            }

            if let Some(current_user) = current_user {
                {friendship_section(
                    "Incoming Requests",
                    incoming,
                    "No incoming friend requests.",
                    current_user.clone(),
                    session.clone(),
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                    direct_messages,
                    active_tab,
                )}
                {friendship_section(
                    "Friends",
                    accepted,
                    "No accepted friends yet.",
                    current_user.clone(),
                    session.clone(),
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                    direct_messages,
                    active_tab,
                )}
                {friendship_section(
                    "Sent Requests",
                    outgoing,
                    "No outgoing friend requests.",
                    current_user.clone(),
                    session.clone(),
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                    direct_messages,
                    active_tab,
                )}
                if !inactive.is_empty() {
                    {friendship_section(
                        "Inactive Records",
                        inactive,
                        "No inactive friend records.",
                        current_user,
                        session.clone(),
                        session_generation,
                        friendships_load_generation,
                        friendships,
                        friends_status,
                        direct_messages,
                        active_tab,
                    )}
                }
            } else {
                section { class: "relationship-section",
                    h3 { "Friends" }
                    p { class: "muted-copy", "Sign in to manage friends." }
                }
            }
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn friendship_section(
    title: &'static str,
    items: Vec<FriendshipSummary>,
    empty: &'static str,
    current_user: UserSummary,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
    direct_messages: DirectMessageSignals,
    active_tab: Signal<AppTab>,
) -> Element {
    let count = items.len();

    rsx! {
        section { class: "relationship-section",
            h3 { "{title} ({count})" }
            if items.is_empty() {
                p { class: "muted-copy", "{empty}" }
            } else {
                div { class: "relationship-list",
                    for friendship in items {
                        {friendship_row(friendship, current_user.clone(), session.clone(), session_generation, friendships_load_generation, friendships, friends_status, direct_messages, active_tab)}
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn friend_search_selected_user(
    user: UserSummary,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
    direct_messages: DirectMessageSignals,
    active_tab: Signal<AppTab>,
) -> Element {
    let display_name = user.display_name.clone();
    let message_session = session.clone();
    let message_target = user.clone();
    let message_disabled = session.is_none() || *direct_messages.create_pending.read();

    rsx! {
        div { class: "user-autocomplete-selected",
            div { class: "user-autocomplete-selected-user",
                span { "Selected user" }
                strong { "{display_name}" }
            }
            div { class: "user-autocomplete-actions",
                button {
                    class: "secondary-button compact",
                    disabled: session.is_none(),
                    onclick: move |_| {
                        if let Some(session) = session.clone() {
                            send_friend_invite(
                                session,
                                session_generation,
                                friendships_load_generation,
                                user.clone(),
                                friendships,
                                friends_status,
                            );
                        }
                    },
                    "Add Friend"
                }
                button {
                    class: "secondary-button compact",
                    disabled: message_disabled,
                    aria_label: "Start direct message with {display_name}",
                    onclick: move |_| {
                        if let Some(session) = message_session.clone() {
                            create_direct_message_thread(
                                session,
                                session_generation,
                                direct_messages,
                                active_tab,
                                message_target.clone(),
                            );
                        }
                    },
                    "Message"
                }
            }
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn friendship_row(
    friendship: FriendshipSummary,
    current_user: UserSummary,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
    direct_messages: DirectMessageSignals,
    active_tab: Signal<AppTab>,
) -> Element {
    let other = friendship_other_user(&friendship, &current_user);
    let other_id = other.id.to_string();
    let status = friendship.status;
    let status_label = friendship_status_label(status);
    let incoming_pending =
        status == FriendshipStatus::Pending && friendship.addressee.id == current_user.id;
    let outgoing_pending =
        status == FriendshipStatus::Pending && friendship.requester.id == current_user.id;
    let accepted = status == FriendshipStatus::Accepted;
    let actions_disabled = session.is_none();
    let accept_session = session.clone();
    let decline_session = session.clone();
    let remove_session = session.clone();
    let accept_friendship = friendship.clone();
    let decline_friendship = friendship.clone();
    let remove_target = other.clone();
    let message_session = session.clone();
    let message_target = other.clone();
    let message_disabled = actions_disabled || *direct_messages.create_pending.read();

    rsx! {
        article { key: "{friendship.id}", class: "relationship-row",
            div { class: "relationship-main",
                strong { "{other.display_name}" }
                span { "{status_label} | {other_id}" }
            }
            div { class: "relationship-actions",
                if incoming_pending {
                    button {
                        class: "secondary-button compact",
                        disabled: actions_disabled,
                        onclick: move |_| {
                            if let Some(session) = accept_session.clone() {
                                accept_friend_invite(
                                    session,
                                    session_generation,
                                    friendships_load_generation,
                                    accept_friendship.clone(),
                                    friendships,
                                    friends_status,
                                );
                            }
                        },
                        "Accept"
                    }
                    button {
                        class: "secondary-button compact danger",
                        disabled: actions_disabled,
                        onclick: move |_| {
                            if let Some(session) = decline_session.clone() {
                                decline_friend_invite(
                                    session,
                                    session_generation,
                                    friendships_load_generation,
                                    decline_friendship.clone(),
                                    friendships,
                                    friends_status,
                                );
                            }
                        },
                        "Decline"
                    }
                } else if accepted {
                    button {
                        class: "secondary-button compact",
                        disabled: message_disabled,
                        aria_label: "Start direct message with {other.display_name}",
                        onclick: move |_| {
                            if let Some(session) = message_session.clone() {
                                create_direct_message_thread(
                                    session,
                                    session_generation,
                                    direct_messages,
                                    active_tab,
                                    message_target.clone(),
                                );
                            }
                        },
                        "Message"
                    }
                    button {
                        class: "secondary-button compact danger",
                        disabled: actions_disabled,
                        onclick: move |_| {
                            if let Some(session) = remove_session.clone() {
                                remove_friend(
                                    session,
                                    session_generation,
                                    friendships_load_generation,
                                    remove_target.clone(),
                                    friendships,
                                    friends_status,
                                );
                            }
                        },
                        "Remove"
                    }
                } else if outgoing_pending {
                    span { class: "action-note", "Awaiting response" }
                } else {
                    span { class: "action-note", "Inactive" }
                }
            }
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn block_report_panel(
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    chat_messages: Signal<Vec<ChatMessage>>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friendships_load_generation: Signal<u64>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    blocks_load_generation: Signal<u64>,
    block_search: UserAutocompleteSignals,
    block_status: Signal<String>,
    mut report_reason: Signal<String>,
    mut report_details: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_status: Signal<String>,
    direct_messages: DirectMessageSignals,
) -> Element {
    let search_selected_user = block_search.selected.read().clone();
    let blocked = blocked_users.read().clone();
    let block_status_text = block_status.read().clone();
    let reason_text = report_reason.read().clone();
    let details_text = report_details.read().clone();
    let selected_report = report_draft.read().clone();
    let report_status_text = report_status.read().clone();
    let can_use = session.is_some();
    let can_submit_report = can_use && selected_report.is_some() && !reason_text.trim().is_empty();
    let report_kind = selected_report
        .as_ref()
        .map(report_draft_kind_label)
        .unwrap_or("No target");
    let report_target_name = selected_report
        .as_ref()
        .map(|draft| draft.target.display_name.clone())
        .unwrap_or_else(|| "Select a user or chat message".to_string());
    let report_target_id = selected_report
        .as_ref()
        .map(|draft| draft.target.id.to_string())
        .unwrap_or_default();
    let report_preview = selected_report
        .as_ref()
        .and_then(|draft| draft.message_preview.as_ref())
        .map(|body| report_preview_text(body));
    let refresh_session = session.clone();
    let submit_session = session.clone();
    let submit_report = selected_report.clone();
    let block_selected_session = session.clone();
    let block_selected_target = selected_report.as_ref().map(|draft| draft.target.clone());

    rsx! {
        div { class: "relationship-layout",
            section { class: "tool-card",
                div { class: "section-heading",
                    h2 { "Block / Report" }
                    button {
                        class: "secondary-button compact",
                        disabled: !can_use,
                        onclick: move |_| {
                            if let Some(session) = refresh_session.clone() {
                                load_blocks(
                                    session,
                                    session_generation,
                                    blocks_load_generation,
                                    blocked_users,
                                    block_status,
                                );
                            }
                        },
                        "Refresh Blocks"
                    }
                }
                p { "A block is one-way, but it hides global chat communication in both directions." }
                {user_autocomplete_combobox(
                    "block-user-suggestions",
                    "Search users to block or report",
                    "Search users to block or report",
                    session.clone(),
                    session_generation,
                    block_search,
                )}
                span { class: "panel-status", "{block_status_text}" }
                if let Some(user) = search_selected_user {
                    {block_report_selected_user(
                        user,
                        session.clone(),
                        session_generation,
                        chat_messages,
                        friendships,
                        friendships_load_generation,
                        friends_status,
                        blocked_users,
                        blocks_load_generation,
                        block_status,
                        report_draft,
                        report_status,
                        direct_messages,
                    )}
                }
            }

            if selected_report.is_some() {
                section { class: "relationship-section",
                    h3 { "Report Details" }
                    div { class: "report-target-card",
                        span { class: "report-target-kind", "{report_kind}" }
                        strong { "{report_target_name}" }
                        if !report_target_id.is_empty() {
                            span { "{report_target_id}" }
                        }
                        if let Some(preview) = report_preview {
                            p { class: "report-preview", "\"{preview}\"" }
                        }
                    }
                    div { class: "report-fields",
                        input {
                            placeholder: "Reason, e.g. harassment or spam",
                            value: "{reason_text}",
                            disabled: !can_use,
                            oninput: move |event| report_reason.set(event.value())
                        }
                        textarea {
                            placeholder: "Optional details",
                            value: "{details_text}",
                            disabled: !can_use,
                            oninput: move |event| report_details.set(event.value())
                        }
                    }
                    div { class: "report-submit-row",
                        if let Some(target) = block_selected_target {
                            button {
                                class: "secondary-button compact danger",
                                disabled: !can_use,
                                onclick: move |_| {
                                    if let Some(session) = block_selected_session.clone() {
                                        block_user_action(
                                            session,
                                            session_generation,
                                            target.clone(),
                                            chat_messages,
                                            block_status,
                                            RelationshipSignals {
                                                friendships,
                                                friendships_load_generation,
                                                friends_status,
                                                blocked_users,
                                                blocks_load_generation,
                                                block_status,
                                            },
                                            direct_messages,
                                        );
                                    }
                                },
                                "Block User"
                            }
                        }
                        button {
                            class: "secondary-button compact",
                            disabled: !can_submit_report,
                            onclick: move |_| {
                                if let (Some(session), Some(draft)) = (submit_session.clone(), submit_report.clone()) {
                                    submit_report_action(
                                        session,
                                        session_generation,
                                        draft,
                                        report_reason.read().clone(),
                                        report_details.read().clone(),
                                        report_status,
                                        report_draft,
                                        report_reason,
                                        report_details,
                                    );
                                }
                            },
                            "Submit Report"
                        }
                        button {
                            class: "secondary-button compact",
                            disabled: selected_report.is_none(),
                            onclick: move |_| {
                                report_draft.set(None);
                                report_status.set("Select a user or message to report".to_string());
                            },
                            "Clear"
                        }
                    }
                }
            }
            span {
                class: "panel-status",
                role: "status",
                aria_live: "polite",
                "{report_status_text}"
            }

            section { class: "relationship-section",
                h3 { "Blocked Users" }
                if blocked.is_empty() {
                    p { class: "muted-copy", "No blocked users." }
                } else {
                    div { class: "relationship-list",
                        for user in blocked {
                            {blocked_user_row(user, session.clone(), session_generation, blocks_load_generation, blocked_users, block_status)}
                        }
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn block_report_selected_user(
    user: UserSummary,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    chat_messages: Signal<Vec<ChatMessage>>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friendships_load_generation: Signal<u64>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    blocks_load_generation: Signal<u64>,
    block_status: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_status: Signal<String>,
    direct_messages: DirectMessageSignals,
) -> Element {
    let display_name = user.display_name.clone();
    let actions_disabled = session.is_none();
    let block_session = session.clone();
    let block_target = user.clone();
    let report_target = user.clone();

    rsx! {
        div { class: "user-autocomplete-selected",
            div { class: "user-autocomplete-selected-user",
                span { "Selected user" }
                strong { "{display_name}" }
            }
            div { class: "user-autocomplete-actions",
                button {
                    class: "secondary-button compact danger",
                    disabled: actions_disabled,
                    onclick: move |_| {
                        if let Some(session) = block_session.clone() {
                            block_user_action(
                                session,
                                session_generation,
                                block_target.clone(),
                                chat_messages,
                                block_status,
                                RelationshipSignals {
                                    friendships,
                                    friendships_load_generation,
                                    friends_status,
                                    blocked_users,
                                    blocks_load_generation,
                                    block_status,
                                },
                                direct_messages,
                            );
                        }
                    },
                    "Block"
                }
                button {
                    class: "secondary-button compact",
                    disabled: actions_disabled,
                    onclick: move |_| {
                        let draft = user_report_draft(report_target.clone());
                        report_status.set(report_draft_status(&draft));
                        report_draft.set(Some(draft));
                    },
                    "Report"
                }
            }
        }
    }
}

#[cfg(windows)]
fn blocked_user_row(
    user: UserSummary,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    blocks_load_generation: Signal<u64>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
) -> Element {
    let user_id = user.id.to_string();
    let display_name = user.display_name.clone();

    rsx! {
        article { key: "{user_id}", class: "relationship-row",
            div { class: "relationship-main",
                strong { "{display_name}" }
                span { "{user_id}" }
            }
            div { class: "relationship-actions",
                button {
                    class: "secondary-button compact",
                    disabled: session.is_none(),
                    onclick: move |_| {
                        if let Some(session) = session.clone() {
                            unblock_user_action(
                                session,
                                session_generation,
                                blocks_load_generation,
                                user.clone(),
                                blocked_users,
                                block_status,
                            );
                        }
                    },
                    "Unblock"
                }
            }
        }
    }
}

#[cfg(windows)]
fn friendship_other_user(
    friendship: &FriendshipSummary,
    current_user: &UserSummary,
) -> UserSummary {
    if friendship.requester.id == current_user.id {
        friendship.addressee.clone()
    } else {
        friendship.requester.clone()
    }
}

#[cfg(windows)]
fn friendship_status_label(status: FriendshipStatus) -> &'static str {
    match status {
        FriendshipStatus::Pending => "pending",
        FriendshipStatus::Accepted => "accepted",
        FriendshipStatus::Declined => "declined",
        FriendshipStatus::Removed => "removed",
    }
}

#[cfg(windows)]
fn passive_overlay_view(
    chat_messages: Signal<Vec<ChatMessage>>,
    chat_status: Signal<String>,
    counts: PresenceCounts,
    signed_in: bool,
) -> Element {
    let status = chat_status.read().clone();
    let mut recent_messages = chat_messages
        .read()
        .iter()
        .rev()
        .take(3)
        .cloned()
        .collect::<Vec<_>>();
    recent_messages.reverse();

    rsx! {
        section { class: "passive-overlay-card",
            header { class: "passive-overlay-header",
                div { class: "passive-overlay-brand",
                    strong { "Agora" }
                    span { if signed_in { "Global" } else { "Sign in" } }
                }
                div { class: "passive-overlay-status",
                    div { class: "passive-presence",
                        span { "{counts.online} online" }
                    }
                    span { class: "passive-hotkey", "Ctrl+Enter" }
                }
            }
            div { class: "passive-overlay-messages",
                if recent_messages.is_empty() {
                    p { class: "passive-empty", "{status}" }
                } else {
                    for message in recent_messages {
                        article { key: "{message.id}", class: "passive-message",
                            strong { "{message.author.display_name}" }
                            span { class: "passive-message-body", "{message.body}" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
const STYLE: &str = r#"
* {
    box-sizing: border-box;
}

:root {
    --aom-bg: #070909;
    --aom-panel: #111412;
    --aom-panel-top: #171815;
    --aom-panel-bottom: #070909;
    --aom-panel-soft: #131614;
    --aom-row-top: #1a1d1b;
    --aom-row-bottom: #0b0d0c;
    --aom-button-top: #3b2c18;
    --aom-button-bottom: #17100a;
    --aom-frame: #a57932;
    --aom-frame-mid: #c49845;
    --aom-frame-bright: #ecd48b;
    --aom-frame-dark: #251806;
    --aom-text: #f5efe0;
    --aom-muted: #b2a78f;
}

* {
    scrollbar-width: thin;
    scrollbar-color: var(--aom-frame-mid) #080a0a;
}

*::-webkit-scrollbar {
    width: 12px;
    height: 12px;
}

*::-webkit-scrollbar-track {
    border: 1px solid var(--aom-frame-dark);
    background: linear-gradient(
        90deg,
        #5b421b 0,
        #5b421b 1px,
        #080a0a 1px,
        #080a0a calc(100% - 1px),
        #5b421b calc(100% - 1px),
        #5b421b 100%
    );
    box-shadow: inset 0 0 0 1px #030404;
}

*::-webkit-scrollbar-thumb {
    min-height: 32px;
    border: 2px solid #080a0a;
    border-radius: 2px;
    background: linear-gradient(90deg, #704d1b, #c49845 24%, #f2dea1 50%, #c49845 76%, #704d1b);
    background-clip: padding-box;
    box-shadow: inset 0 0 0 1px #f7e5ad;
}

*::-webkit-scrollbar-thumb:hover {
    background: linear-gradient(90deg, #8a6328, #d8b763 24%, #fff0bf 50%, #d8b763 76%, #8a6328);
    background-clip: padding-box;
}

*::-webkit-scrollbar-button {
    width: 0;
    height: 0;
}

*::-webkit-scrollbar-corner {
    background: #080a0a;
}

html,
body {
    margin: 0;
    width: 100%;
    min-height: 100vh;
    overflow: hidden;
}

body {
    color: var(--aom-text);
    background: var(--aom-bg);
    font-family: Segoe UI, system-ui, sans-serif;
}

button,
input,
select {
    font: inherit;
}

h2,
p {
    margin: 0;
}

h2 {
    margin-bottom: 8px;
    font-size: 22px;
}

p {
    color: #cfc5b8;
}

.secondary-button {
    padding: 11px 15px;
    border: 0;
    border-radius: 13px;
    font-weight: 700;
    cursor: pointer;
}

.secondary-button {
    color: #f7efe2;
    background: rgba(255, 255, 255, 0.12);
}

.secondary-button.compact {
    padding: 8px 11px;
    font-size: 13px;
}

.secondary-button.danger {
    color: #fecaca;
    background: rgba(239, 68, 68, 0.16);
}

.secondary-button:hover {
    filter: brightness(1.1);
}

.chat-status-inline {
    position: sticky;
    top: 0;
    align-self: flex-start;
    padding: 5px 9px;
    border: 1px solid var(--aom-frame);
    border-radius: 999px;
    color: var(--aom-muted);
    background: #080a0a;
    font-size: 11px;
}

.message-meta {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-bottom: 4px;
}

.message-actions {
    display: flex;
    margin-left: auto;
    gap: 6px;
}

.message-action {
    padding: 2px 7px;
    border: 1px solid rgba(236, 212, 139, 0.35);
    border-radius: 999px;
    color: var(--aom-frame-bright);
    background: rgba(255, 255, 255, 0.06);
    font-size: 10px;
    cursor: pointer;
}

.message-meta time {
    color: #9f9487;
    font-size: 12px;
}

.chat-mode-overlay {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: start;
    gap: 12px;
    height: 100vh;
    padding: 13px;
    overflow: hidden;
    border: 3px solid var(--aom-frame-mid);
    border-radius: 5px;
    background: linear-gradient(180deg, var(--aom-panel-top), var(--aom-panel-bottom));
    box-shadow: inset 0 0 0 1px var(--aom-frame-bright), 0 0 0 1px var(--aom-frame-dark), 0 18px 52px rgba(0, 0, 0, 0.5);
}

.overlay-tabs {
    display: grid;
    align-self: start;
    gap: 8px;
    width: 132px;
}

.overlay-tab {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    padding: 10px 12px;
    border: 1px solid var(--aom-frame);
    border-radius: 4px;
    color: var(--aom-text);
    background: linear-gradient(180deg, var(--aom-panel-soft), #0b0d0c);
    text-align: left;
    font-weight: 700;
    cursor: pointer;
}

.overlay-tab:hover,
.overlay-tab.active {
    color: #fff8ed;
    border-color: var(--aom-frame-bright);
    background: linear-gradient(180deg, var(--aom-button-top), var(--aom-button-bottom));
}

.tab-badge {
    min-width: 18px;
    padding: 2px 5px;
    border-radius: 999px;
    color: #1b1207;
    background: var(--aom-frame-bright);
    font-size: 10px;
    text-align: center;
}

.overlay-content {
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr);
    gap: 10px;
    min-width: 0;
    height: calc(100vh - 28px);
    min-height: 0;
    overflow: hidden;
    padding: 14px;
    border: 1px solid var(--aom-frame);
    border-radius: 4px;
    background: var(--aom-bg);
}

.overlay-content-body {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
}

.overlay-toolbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid rgba(236, 212, 139, 0.35);
    border-radius: 4px;
    background: rgba(8, 10, 10, 0.82);
}

.overlay-toolbar-account {
    display: flex;
    flex: 1 1 260px;
    align-items: center;
    justify-content: space-between;
    min-width: 0;
    gap: 10px;
}

.toolbar-auth-status {
    min-width: 0;
    color: var(--aom-muted);
    font-size: 11px;
    line-height: 1.35;
    overflow-wrap: anywhere;
}

.remote-login-provider {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    gap: 6px;
    color: var(--aom-frame-bright);
    font-size: 11px;
    font-weight: 800;
}

.remote-login-provider select {
    padding: 4px 6px;
    border: 1px solid var(--aom-frame);
    border-radius: 3px;
    color: var(--aom-text);
    background: #080a0a;
}

.remote-login-provider {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    gap: 6px;
    color: var(--aom-frame-bright);
    font-size: 11px;
    font-weight: 800;
}

.remote-login-provider select {
    padding: 4px 6px;
    border: 1px solid var(--aom-frame);
    border-radius: 3px;
    color: var(--aom-text);
    background: #080a0a;
}

.update-banner {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 9px;
    padding: 8px 9px;
    border: 1px solid rgba(236, 212, 139, 0.35);
    border-radius: 4px;
    background: rgba(8, 10, 10, 0.82);
}

.update-banner.required {
    border-color: #d6b45b;
    background: rgba(77, 59, 25, 0.34);
}

.update-banner h2 {
    margin: 0;
    color: var(--aom-frame-bright);
    font-size: 12px;
    text-transform: uppercase;
}

.update-status {
    min-width: 0;
    overflow-wrap: anywhere;
    color: var(--aom-muted);
    font-size: 11px;
    line-height: 1.35;
}

.update-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 6px;
}

.update-actions .secondary-button.compact {
    padding: 6px 8px;
    border: 1px solid rgba(236, 212, 139, 0.26);
    border-radius: 4px;
    font-size: 11px;
}

.overlay-content .relationship-layout {
    max-height: 100%;
    overflow-y: auto;
}

.overlay-global-panel {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    gap: 10px;
    height: 100%;
    min-height: 0;
}

.overlay-panel-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 12px;
}

.overlay-panel-heading h2 {
    margin: 0;
}

.overlay-panel-heading p {
    color: var(--aom-muted);
    font-size: 12px;
}

.overlay-panel-actions {
    display: flex;
    align-items: center;
    gap: 8px;
}

.availability-control {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 6px 8px;
    border: 1px solid rgba(236, 212, 139, 0.35);
    border-radius: 4px;
    background: rgba(8, 10, 10, 0.78);
}

.availability-label {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--aom-frame-bright);
    font-size: 11px;
    font-weight: 800;
}

.availability-label select {
    padding: 4px 6px;
    border: 1px solid var(--aom-frame);
    border-radius: 3px;
    color: var(--aom-text);
    background: #080a0a;
}

.availability-status {
    color: var(--aom-muted);
    font-size: 10px;
}

.toolbar-auth-button {
    padding: 5px 9px;
    border: 1px solid var(--aom-frame-bright);
    border-radius: 999px;
    color: #1b1207;
    background: linear-gradient(180deg, #d6b45b, #7c5e23);
    font-size: 11px;
    font-weight: 800;
    cursor: pointer;
}

.local-dev-controls {
    display: flex;
    flex-basis: 100%;
    flex-wrap: wrap;
    align-items: center;
    gap: 7px;
    padding: 7px 9px;
    border: 1px solid #4d3b19;
    border-radius: 4px;
    background: rgba(77, 59, 25, .22);
}

.local-dev-account {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--aom-frame-bright);
    font-size: 11px;
    font-weight: 800;
}

.local-dev-account input,
.local-dev-token {
    min-width: 0;
    border: 1px solid var(--aom-frame);
    border-radius: 3px;
    color: var(--aom-text);
    background: #080a0a;
    font: inherit;
}

.local-dev-account input {
    width: 110px;
    padding: 4px 6px;
}

.local-dev-note {
    margin: 0;
    color: var(--aom-muted);
    font-size: 10px;
}

.local-dev-staff {
    display: flex;
    flex-basis: 100%;
    gap: 7px;
}

.local-dev-token {
    flex: 1;
    padding: 4px 6px;
    font-family: Consolas, monospace;
    font-size: 10px;
}

.overlay-message-list {
    display: flex;
    min-height: 0;
    overflow-y: auto;
    flex-direction: column;
    gap: 6px;
}

.message-scroll-anchor {
    min-height: 1px;
}

.overlay-message {
    padding: 8px 10px;
    border: 1px solid #3a2a11;
    border-radius: 3px;
    background: linear-gradient(180deg, var(--aom-row-top), var(--aom-row-bottom));
}

.overlay-message strong {
    color: #ffffff;
    font-weight: 800;
}

.overlay-message .message-meta {
    gap: 8px;
    margin-bottom: 3px;
}

.overlay-message .message-meta time {
    color: #a99d89;
    font-size: 10px;
}

.overlay-message p {
    overflow-wrap: anywhere;
    color: var(--aom-text);
    font-size: 14px;
    line-height: 1.35;
}

.overlay-empty {
    display: grid;
    place-content: center;
    height: 100%;
    border: 1px dashed var(--aom-frame);
    border-radius: 4px;
    background: #080a0a;
    text-align: center;
}

.overlay-composer {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 8px;
}

.overlay-composer input {
    min-width: 0;
    padding: 11px 12px;
    border: 1px solid var(--aom-frame);
    border-radius: 4px;
    color: var(--aom-text);
    background: #080a0a;
}

.overlay-composer input:focus {
    outline: none;
    border-color: var(--aom-frame-bright);
    box-shadow: inset 0 0 0 1px var(--aom-frame-bright);
}

.overlay-composer button {
    padding: 11px 15px;
    border: 1px solid var(--aom-frame-bright);
    border-radius: 4px;
    color: #1b1207;
    background: linear-gradient(180deg, #d6b45b, #7c5e23);
    font-weight: 800;
}

.passive-overlay-card {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    padding: 4px 7px 5px;
    overflow: hidden;
    border: 3px solid var(--aom-frame-mid);
    border-radius: 5px;
    background: linear-gradient(180deg, var(--aom-panel-top), var(--aom-panel-bottom));
}

.passive-overlay-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-height: 15px;
}

.passive-overlay-brand,
.passive-overlay-status {
    display: flex;
    align-items: center;
    min-width: 0;
    gap: 5px;
}

.passive-overlay-brand strong {
    color: var(--aom-text);
    font-size: 11px;
    line-height: 1;
    letter-spacing: 0.02em;
}

.passive-overlay-brand span,
.passive-empty {
    color: var(--aom-frame-bright);
    font-size: 9px;
    line-height: 1;
}

.passive-presence {
    display: flex;
    flex-wrap: nowrap;
    justify-content: flex-end;
    gap: 4px;
}

.passive-presence span,
.passive-hotkey {
    display: block;
    padding: 2px 4px;
    border-radius: 999px;
    font-size: 9px;
    line-height: 1;
    white-space: nowrap;
}

.passive-presence span {
    color: var(--aom-muted);
    background: #080a0a;
}

.passive-hotkey {
    color: #1b1207;
    background: linear-gradient(180deg, #d6b45b, #7c5e23);
    font-weight: 800;
}

.passive-overlay-messages {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 3px;
    margin-top: 4px;
    overflow: hidden;
}

.passive-message {
    display: flex;
    align-items: center;
    min-width: 0;
    gap: 5px;
    min-height: 16px;
    padding: 2px 5px;
    border: 1px solid #3a2a11;
    border-radius: 3px;
    background: linear-gradient(180deg, var(--aom-row-top), var(--aom-row-bottom));
}

.passive-message strong {
    flex: 0 1 auto;
    max-width: 104px;
    overflow: hidden;
    color: var(--aom-frame-bright);
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 9px;
    line-height: 1.1;
}

.passive-message-body {
    display: block;
    min-width: 0;
    overflow: hidden;
    color: var(--aom-text);
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 9px;
    line-height: 1.1;
}

.passive-empty {
    grid-column: 1 / -1;
    overflow: hidden;
    padding: 3px 6px;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.relationship-layout {
    display: grid;
    gap: 16px;
}

.friend-summary-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
    margin-top: 12px;
}

.friend-stat {
    padding: 9px 10px;
    border: 1px solid rgba(236, 212, 139, 0.22);
    border-radius: 4px;
    color: var(--aom-muted);
    background: #080a0a;
    font-size: 12px;
}

.friend-stat strong {
    color: var(--aom-frame-bright);
}

.tool-card,
.relationship-section {
    padding: 18px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 20px;
    background: rgba(255, 255, 255, 0.04);
}

.section-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 8px;
}

.relationship-section h3 {
    margin: 0 0 10px;
    color: #fff8ed;
    font-size: 16px;
}

.search-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 10px;
    margin-top: 14px;
}

.search-row input,
.report-fields input,
.report-fields textarea {
    min-width: 0;
    padding: 12px 13px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 14px;
    color: #f7efe2;
    background: rgba(255, 255, 255, 0.07);
}

.search-row button {
    padding: 12px 16px;
    border: 0;
    border-radius: 14px;
    color: #20160a;
    background: var(--aom-frame);
    font-weight: 700;
}

.user-autocomplete {
    position: relative;
    margin-top: 14px;
}

.user-autocomplete-input {
    width: 100%;
    min-width: 0;
    padding: 12px 13px;
    border: 1px solid rgba(236, 212, 139, 0.35);
    border-radius: 6px;
    color: var(--aom-text);
    background: #080a0a;
}

.user-autocomplete-input:focus {
    outline: none;
    border-color: var(--aom-frame-bright);
    box-shadow: inset 0 0 0 1px var(--aom-frame-bright);
}

.user-autocomplete-hint {
    display: block;
    margin-top: 6px;
    color: var(--aom-muted);
    font-size: 11px;
}

.user-autocomplete-menu {
    position: absolute;
    z-index: 4;
    top: calc(100% + 4px);
    right: 0;
    left: 0;
    max-height: 220px;
    overflow-y: auto;
    border: 1px solid var(--aom-frame);
    border-radius: 5px;
    background: linear-gradient(180deg, #1a1d1b, #080a0a);
    box-shadow: inset 0 0 0 1px var(--aom-frame-dark), 0 10px 24px rgba(0, 0, 0, 0.42);
}

.user-autocomplete-option,
.user-autocomplete-empty {
    min-width: 0;
    padding: 9px 11px;
}

.user-autocomplete-option {
    display: grid;
    gap: 2px;
    border-bottom: 1px solid rgba(236, 212, 139, 0.13);
    cursor: pointer;
}

.user-autocomplete-option:last-child {
    border-bottom: 0;
}

.user-autocomplete-option:hover,
.user-autocomplete-option.active {
    background: rgba(165, 121, 50, 0.25);
    box-shadow: inset 3px 0 0 var(--aom-frame-bright);
}

.user-autocomplete-option strong {
    overflow: hidden;
    color: #fff8ed;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.user-autocomplete-option span,
.user-autocomplete-empty {
    overflow: hidden;
    color: var(--aom-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
}

.user-autocomplete-selected {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 10px;
    padding: 10px 11px;
    border: 1px solid rgba(236, 212, 139, 0.28);
    border-radius: 6px;
    background: #080a0a;
}

.user-autocomplete-selected-user {
    display: grid;
    min-width: 0;
    gap: 2px;
}

.user-autocomplete-selected-user span {
    color: var(--aom-muted);
    font-size: 10px;
    text-transform: uppercase;
}

.user-autocomplete-selected-user strong {
    overflow: hidden;
    color: var(--aom-frame-bright);
    text-overflow: ellipsis;
    white-space: nowrap;
}

.user-autocomplete-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
}

.panel-status,
.muted-copy,
.action-note {
    color: #b8ad9f;
    font-size: 13px;
}

.panel-status {
    display: block;
    margin-top: 10px;
}

.relationship-list {
    display: grid;
    gap: 10px;
}

.relationship-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 12px;
    padding: 12px;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 16px;
    background: rgba(255, 255, 255, 0.05);
}

.relationship-main {
    min-width: 0;
}

.relationship-main strong,
.relationship-main span {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.relationship-main span {
    color: #9f9487;
    font-size: 12px;
}

.relationship-actions {
    display: flex;
    align-items: center;
    gap: 8px;
}

.report-fields {
    display: grid;
    gap: 10px;
}

.report-target-card {
    display: grid;
    gap: 4px;
    margin-bottom: 10px;
    padding: 10px;
    border: 1px solid rgba(236, 212, 139, 0.22);
    border-radius: 4px;
    background: #080a0a;
}

.report-target-card strong,
.report-target-card span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.report-target-card span,
.report-preview {
    color: var(--aom-muted);
    font-size: 12px;
}

.report-target-kind {
    color: var(--aom-frame-bright) !important;
    font-weight: 800;
    text-transform: uppercase;
}

.report-preview {
    overflow-wrap: anywhere;
}

.report-submit-row {
    display: flex;
    gap: 8px;
    margin-top: 10px;
}

.report-fields textarea {
    min-height: 90px;
    resize: vertical;
}

.sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
}

.overlay-content .direct-message-layout {
    max-height: 100%;
    overflow-y: auto;
}

.direct-message-layout {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
    height: 100%;
}

.dm-start-card p {
    color: var(--aom-muted);
    font-size: 12px;
}

.dm-workspace {
    display: grid;
    grid-template-columns: minmax(180px, 0.7fr) minmax(0, 1.5fr);
    flex: 1 1 280px;
    min-height: 280px;
    overflow: hidden;
    border: 1px solid rgba(236, 212, 139, 0.22);
    border-radius: 6px;
    background: #080a0a;
}

.dm-thread-pane,
.dm-conversation-pane {
    min-width: 0;
    min-height: 0;
    padding: 12px;
}

.dm-thread-pane {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    gap: 8px;
    border-right: 1px solid rgba(236, 212, 139, 0.18);
}

.dm-thread-pane h3,
.dm-conversation-pane h3 {
    margin: 0;
    color: var(--aom-frame-bright);
    font-size: 14px;
}

.dm-thread-list {
    display: grid;
    align-content: start;
    gap: 6px;
    overflow-y: auto;
}

.dm-thread {
    display: grid;
    gap: 3px;
    width: 100%;
    padding: 9px;
    border: 1px solid transparent;
    border-radius: 4px;
    color: var(--aom-text);
    background: transparent;
    text-align: left;
    cursor: pointer;
}

.dm-thread:hover,
.dm-thread.active {
    border-color: var(--aom-frame);
    background: rgba(165, 121, 50, 0.2);
}

.dm-thread strong,
.dm-thread span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.dm-thread span {
    color: var(--aom-muted);
    font-size: 10px;
}

.dm-conversation-pane {
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr) auto;
    gap: 8px;
}

.dm-conversation-pane .section-heading {
    margin: 0;
}

.dm-conversation-pane .section-heading p {
    color: var(--aom-muted);
    font-size: 11px;
}

.dm-load-older {
    justify-self: start;
}

.dm-message-list {
    display: grid;
    align-content: start;
    min-height: 0;
    overflow-y: auto;
    gap: 6px;
}

.dm-message {
    background: linear-gradient(180deg, #18201d, #0b0d0c);
}

.dm-composer {
    margin-top: 2px;
}

button:disabled,
input:disabled,
select:disabled {
    cursor: not-allowed;
    opacity: 0.55;
}

@media (max-width: 760px) {
    .relationship-row,
    .search-row,
    .dm-workspace,
    .update-banner {
        grid-template-columns: 1fr;
    }

    .update-actions {
        justify-content: flex-start;
    }

    .overlay-toolbar-account {
        align-items: flex-start;
        flex-direction: column;
    }

    .relationship-actions {
        flex-wrap: wrap;
    }

    .user-autocomplete-selected {
        align-items: flex-start;
        flex-direction: column;
    }

    .user-autocomplete-actions {
        justify-content: flex-start;
    }

    .direct-message-layout {
        height: auto;
        grid-template-rows: auto;
    }

    .dm-workspace {
        overflow: visible;
    }

    .dm-thread-pane {
        max-height: 180px;
        border-right: 0;
        border-bottom: 1px solid rgba(236, 212, 139, 0.18);
    }

    .dm-conversation-pane {
        min-height: 320px;
    }
}
"#;

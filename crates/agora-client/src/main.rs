#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("Agora currently supports Windows desktop.");
}

#[cfg(windows)]
mod game;

#[cfg(windows)]
fn main() {
    game::become_dpi_aware();
    let mut config = dioxus::desktop::Config::new()
        .with_window(
            dioxus::desktop::WindowBuilder::new()
                .with_title("Agora")
                .with_visible(false)
                .with_focused(false)
                .with_decorations(false)
                .with_always_on_top(true)
                .with_resizable(false)
                .with_transparent(true),
        )
        .with_close_behaviour(dioxus::desktop::WindowCloseBehaviour::LastWindowHides)
        .with_background_color((10, 7, 18, 0))
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
    ClientEvent, CreateReportRequest, CreateReportResponse, DevLoginRequest, DevLoginResponse,
    FriendListResponse, FriendRequest, FriendshipResponse, FriendshipStatus, FriendshipSummary,
    LogoutRequest, MessageKind, PresenceCounts, RefreshRequest, RefreshResponse,
    RemoveFriendResponse, ServerEvent, SteamLoginPollRequest, SteamLoginPollResponse,
    SteamLoginStartResponse, SteamLoginStatus, UnblockUserResponse, UserSearchResponse,
    UserSummary, MAX_MESSAGE_LEN,
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
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
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
const SESSION_REFRESH_SAFETY_SECONDS: u64 = 60;
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum AppTab {
    Global,
    Friends,
    BlockReport,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellMode {
    Tray,
    OverlayPassive,
    OverlayInteractive,
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
#[allow(non_snake_case)]
fn App() -> Element {
    let desktop = dioxus::desktop::use_window();
    let _tray_icon = use_hook(|| {
        dioxus::desktop::trayicon::init_tray_icon(
            dioxus::desktop::trayicon::default_tray_icon(),
            app_tray_icon(),
        )
    });
    let active_tab = use_signal(|| AppTab::Global);
    let shell_mode = use_signal(|| ShellMode::Tray);
    let game_window = use_signal(|| None::<game::GameWindow>);
    let overlay_interactive = use_signal(|| false);
    let ctrl_enter_down = use_signal(|| false);
    let mut login_status = use_signal(|| "Checking saved session...".to_string());
    let mut auth_action_pending = use_signal(|| true);
    let mut auth_session = use_signal(|| None::<AuthSession>);
    let session_generation = use_signal(|| 0u64);
    let chat_messages = use_signal(Vec::<ChatMessage>::new);
    let chat_status = use_signal(|| "Sign in to connect to global chat".to_string());
    let presence_counts = use_signal(PresenceCounts::default);
    let composer_body = use_signal(String::new);
    let chat_outbox = use_signal(|| None::<UnboundedSender<ClientEvent>>);
    let friendships = use_signal(Vec::<FriendshipSummary>::new);
    let friend_search_query = use_signal(String::new);
    let friend_search_results = use_signal(Vec::<UserSummary>::new);
    let friends_status = use_signal(|| "Sign in to load friends".to_string());
    let blocked_users = use_signal(Vec::<UserSummary>::new);
    let block_search_query = use_signal(String::new);
    let block_search_results = use_signal(Vec::<UserSummary>::new);
    let block_status = use_signal(|| "Sign in to manage blocks".to_string());
    let report_reason = use_signal(String::new);
    let report_details = use_signal(String::new);
    let report_draft = use_signal(|| None::<ReportDraft>);
    let report_status = use_signal(|| "Search for a user to report".to_string());

    use_hook(move || {
        spawn(async move {
            match load_refresh_token() {
                Ok(Some(refresh_token)) => {
                    login_status.set("Restoring Steam session...".to_string());
                    match refresh_session(refresh_token).await {
                        Ok(session) => {
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
                            start_chat_session(
                                &session,
                                session_generation,
                                chat_messages,
                                chat_status,
                                presence_counts,
                                chat_outbox,
                                friendships,
                                friends_status,
                                blocked_users,
                                block_status,
                            );
                            refresh_relationship_state(
                                &session,
                                session_generation,
                                friendships,
                                friends_status,
                                blocked_users,
                                block_status,
                            );
                            start_session_refresh_loop(
                                session,
                                session_generation,
                                auth_session,
                                login_status,
                                chat_messages,
                                chat_status,
                                presence_counts,
                                chat_outbox,
                                friendships,
                                friends_status,
                                blocked_users,
                                block_status,
                                report_draft,
                            );
                            auth_action_pending.set(false);
                        }
                        Err(error) => {
                            next_session_generation(session_generation);
                            let _ = clear_refresh_token();
                            stop_chat_session(
                                chat_messages,
                                chat_status,
                                presence_counts,
                                chat_outbox,
                            );
                            reset_relationship_state(
                                friendships,
                                friend_search_query,
                                friend_search_results,
                                friends_status,
                                blocked_users,
                                block_search_query,
                                block_search_results,
                                block_status,
                                report_reason,
                                report_details,
                                report_draft,
                                report_status,
                            );
                            login_status.set(format!("Saved session expired: {error}"));
                            auth_action_pending.set(false);
                        }
                    }
                }
                Ok(None) => {
                    login_status.set("Not signed in".to_string());
                    auth_action_pending.set(false);
                }
                Err(error) => {
                    login_status.set(format!("Credential store unavailable: {error}"));
                    auth_action_pending.set(false);
                }
            }
        });
    });

    use_hook({
        let desktop = desktop.clone();
        move || {
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
            )
        }
    });

    use_hook({
        let desktop = desktop.clone();
        move || {
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
                active,
                active_tab,
                current_session.clone(),
                session_generation,
                auth_action_pending,
                auth_session,
                login_status,
                chat_messages,
                composer_body,
                chat_status,
                presence_counts,
                chat_outbox,
                shell_mode,
                game_window,
                overlay_interactive,
                friendships,
                friend_search_query,
                friend_search_results,
                friends_status,
                blocked_users,
                block_search_query,
                block_search_results,
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
) -> Option<dioxus::desktop::ShortcutHandle> {
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
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
) {
    load_friends(
        session.clone(),
        session_generation,
        friendships,
        friends_status,
    );
    load_blocks(
        session.clone(),
        session_generation,
        blocked_users,
        block_status,
    );
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn start_session_refresh_loop(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut auth_session: Signal<Option<AuthSession>>,
    mut login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    chat_outbox: Signal<Option<UnboundedSender<ClientEvent>>>,
    mut friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
    mut blocked_users: Signal<Vec<UserSummary>>,
    mut block_status: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
) {
    spawn(async move {
        let mut session = session;
        let mut generation = *session_generation.read();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(session_refresh_delay(
                session.expires_in_seconds,
            )))
            .await;

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
                    let display_name = refreshed.user.display_name.clone();
                    match store_refresh_token(&refreshed.refresh_token) {
                        Ok(()) => login_status.set(format!("Signed in as {display_name}")),
                        Err(error) => login_status.set(format!(
                            "Session refreshed for {display_name}, but token storage failed: {error}"
                        )),
                    }
                    auth_session.set(Some(refreshed.clone()));
                    start_chat_session(
                        &refreshed,
                        session_generation,
                        chat_messages,
                        chat_status,
                        presence_counts,
                        chat_outbox,
                        friendships,
                        friends_status,
                        blocked_users,
                        block_status,
                    );
                    refresh_relationship_state(
                        &refreshed,
                        session_generation,
                        friendships,
                        friends_status,
                        blocked_users,
                        block_status,
                    );
                    session = refreshed;
                }
                Err(error) => {
                    if !current_session_matches(
                        auth_session,
                        session_generation,
                        generation,
                        &session.refresh_token,
                    ) {
                        return;
                    }

                    next_session_generation(session_generation);
                    let _ = clear_refresh_token();
                    auth_session.set(None);
                    stop_chat_session(chat_messages, chat_status, presence_counts, chat_outbox);
                    friendships.set(Vec::new());
                    friends_status.set("Sign in to load friends".to_string());
                    blocked_users.set(Vec::new());
                    block_status.set("Sign in to manage blocks".to_string());
                    report_draft.set(None);
                    login_status.set(format!("Session expired: {error}"));
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
fn next_session_generation(mut session_generation: Signal<u64>) -> u64 {
    let current = *session_generation.read();
    let next = current.wrapping_add(1);
    session_generation.set(next);
    next
}

#[cfg(windows)]
fn session_generation_current(session_generation: Signal<u64>, generation: u64) -> bool {
    *session_generation.read() == generation
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
    mut login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    mut chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    chat_outbox: Signal<Option<UnboundedSender<ClientEvent>>>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
    report_draft: Signal<Option<ReportDraft>>,
) {
    if *auth_action_pending.read() {
        return;
    }
    auth_action_pending.set(true);
    next_session_generation(session_generation);

    spawn(async move {
        let start_message = login_start_message();
        login_status.set(start_message.clone());
        chat_status.set(start_message);
        match complete_login().await {
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
                start_chat_session(
                    &session,
                    session_generation,
                    chat_messages,
                    chat_status,
                    presence_counts,
                    chat_outbox,
                    friendships,
                    friends_status,
                    blocked_users,
                    block_status,
                );
                refresh_relationship_state(
                    &session,
                    session_generation,
                    friendships,
                    friends_status,
                    blocked_users,
                    block_status,
                );
                start_session_refresh_loop(
                    session,
                    session_generation,
                    auth_session,
                    login_status,
                    chat_messages,
                    chat_status,
                    presence_counts,
                    chat_outbox,
                    friendships,
                    friends_status,
                    blocked_users,
                    block_status,
                    report_draft,
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
    mut login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    chat_outbox: Signal<Option<UnboundedSender<ClientEvent>>>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friend_search_query: Signal<String>,
    friend_search_results: Signal<Vec<UserSummary>>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_search_query: Signal<String>,
    block_search_results: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
    report_reason: Signal<String>,
    report_details: Signal<String>,
    report_draft: Signal<Option<ReportDraft>>,
    report_status: Signal<String>,
) {
    if *auth_action_pending.read() {
        return;
    }
    auth_action_pending.set(true);
    next_session_generation(session_generation);

    let refresh_token = session.refresh_token;
    spawn(async move {
        login_status.set("Signing out...".to_string());
        auth_session.set(None);
        stop_chat_session(chat_messages, chat_status, presence_counts, chat_outbox);
        reset_relationship_state(
            friendships,
            friend_search_query,
            friend_search_results,
            friends_status,
            blocked_users,
            block_search_query,
            block_search_results,
            block_status,
            report_reason,
            report_details,
            report_draft,
            report_status,
        );

        let clear_result = clear_refresh_token();
        let logout_result = logout_session(refresh_token).await;
        match (logout_result, clear_result) {
            (Ok(()), Ok(())) => login_status.set("Signed out".to_string()),
            (Err(error), Ok(())) => {
                login_status.set(format!("Signed out locally. Server logout failed: {error}"))
            }
            (Ok(()), Err(error)) => login_status.set(format!(
                "Server session revoked. Local credential cleanup failed: {error}"
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
    mut friend_search_query: Signal<String>,
    mut friend_search_results: Signal<Vec<UserSummary>>,
    mut friends_status: Signal<String>,
    mut blocked_users: Signal<Vec<UserSummary>>,
    mut block_search_query: Signal<String>,
    mut block_search_results: Signal<Vec<UserSummary>>,
    mut block_status: Signal<String>,
    mut report_reason: Signal<String>,
    mut report_details: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_status: Signal<String>,
) {
    friendships.set(Vec::new());
    friend_search_query.set(String::new());
    friend_search_results.set(Vec::new());
    friends_status.set("Sign in to load friends".to_string());
    blocked_users.set(Vec::new());
    block_search_query.set(String::new());
    block_search_results.set(Vec::new());
    block_status.set("Sign in to manage blocks".to_string());
    report_reason.set(String::new());
    report_details.set(String::new());
    report_draft.set(None);
    report_status.set("Search for a user to report".to_string());
}

#[cfg(windows)]
fn load_friends(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        friends_status.set("Loading friends...".to_string());
        match list_friends_api(&session).await {
            Ok(items) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                let count = items.len();
                friendships.set(items);
                friends_status.set(format!("Loaded {count} friendship records"));
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
fn load_blocks(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut blocked_users: Signal<Vec<UserSummary>>,
    mut block_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        block_status.set("Loading blocked users...".to_string());
        match list_blocks_api(&session).await {
            Ok(users) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                let count = users.len();
                blocked_users.set(users);
                block_status.set(format!("Loaded {count} blocked users"));
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
fn search_friend_users(
    session: AuthSession,
    session_generation: Signal<u64>,
    query: String,
    mut friend_search_results: Signal<Vec<UserSummary>>,
    mut friends_status: Signal<String>,
) {
    let query = query.trim().to_string();
    if query.is_empty() {
        friend_search_results.set(Vec::new());
        friends_status.set("Enter a name to search".to_string());
        return;
    }

    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        friends_status.set(format!("Searching for {query}..."));
        match search_users_api(&session, &query).await {
            Ok(users) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                let count = users.len();
                friend_search_results.set(users);
                friends_status.set(format!("Found {count} users"));
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
fn search_block_report_users(
    session: AuthSession,
    session_generation: Signal<u64>,
    query: String,
    mut block_search_results: Signal<Vec<UserSummary>>,
    mut block_status: Signal<String>,
) {
    let query = query.trim().to_string();
    if query.is_empty() {
        block_search_results.set(Vec::new());
        block_status.set("Enter a name to search".to_string());
        return;
    }

    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        block_status.set(format!("Searching for {query}..."));
        match search_users_api(&session, &query).await {
            Ok(users) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                let count = users.len();
                block_search_results.set(users);
                block_status.set(format!("Found {count} users"));
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
fn send_friend_invite(
    session: AuthSession,
    session_generation: Signal<u64>,
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
                load_friends(session, session_generation, friendships, friends_status);
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
                load_friends(session, session_generation, friendships, friends_status);
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
                load_friends(session, session_generation, friendships, friends_status);
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
                load_friends(session, session_generation, friendships, friends_status);
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
    blocked_users: Signal<Vec<UserSummary>>,
    mut block_status: Signal<String>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
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
                block_status.set(format!("Blocked {}", target.display_name));
                load_blocks(
                    session.clone(),
                    session_generation,
                    blocked_users,
                    block_status,
                );
                load_friends(session, session_generation, friendships, friends_status);
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
                load_blocks(session, session_generation, blocked_users, block_status);
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
fn start_chat_session(
    session: &AuthSession,
    session_generation: Signal<u64>,
    mut chat_messages: Signal<Vec<ChatMessage>>,
    mut chat_status: Signal<String>,
    mut presence_counts: Signal<PresenceCounts>,
    mut chat_outbox: Signal<Option<UnboundedSender<ClientEvent>>>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
) {
    let _ = chat_outbox.write().take();
    chat_messages.set(Vec::new());
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
            chat_messages,
            chat_status,
            presence_counts,
            friendships,
            friends_status,
            blocked_users,
            block_status,
        )
        .await
        {
            if session_generation_current(session_generation, generation) {
                chat_outbox.set(None);
                chat_status.set(format!("Chat disconnected: {error}"));
            }
        }
    });
}

#[cfg(windows)]
fn stop_chat_session(
    mut chat_messages: Signal<Vec<ChatMessage>>,
    mut chat_status: Signal<String>,
    mut presence_counts: Signal<PresenceCounts>,
    mut chat_outbox: Signal<Option<UnboundedSender<ClientEvent>>>,
) {
    let _ = chat_outbox.write().take();
    chat_messages.set(Vec::new());
    presence_counts.set(PresenceCounts::default());
    chat_status.set("Sign in to connect to global chat".to_string());
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
async fn run_chat_socket(
    session: AuthSession,
    session_generation: Signal<u64>,
    generation: u64,
    mut outgoing_rx: UnboundedReceiver<ClientEvent>,
    chat_messages: Signal<Vec<ChatMessage>>,
    mut chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
) -> Result<(), String> {
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
            chat_messages,
            chat_status,
            presence_counts,
            friendships,
            friends_status,
            blocked_users,
            block_status,
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
    outgoing_rx: &mut UnboundedReceiver<ClientEvent>,
    chat_messages: Signal<Vec<ChatMessage>>,
    mut chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
) -> Result<(), String> {
    if !session_generation_current(session_generation, generation) {
        return Ok(());
    }

    let mut request = websocket_url()
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
                let Some(event) = outgoing else {
                    let _ = socket_tx.send(Message::Close(None)).await;
                    return Ok(());
                };
                let text = serde_json::to_string(&event)
                    .map_err(|error| format!("Could not serialize chat event: {error}"))?;
                socket_tx
                    .send(Message::Text(text.into()))
                    .await
                    .map_err(|error| format!("Could not send chat event: {error}"))?;
            }
            incoming = socket_rx.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        handle_chat_event(
                            text.as_str(),
                            session,
                            session_generation,
                            generation,
                            chat_messages,
                            chat_status,
                            presence_counts,
                            friendships,
                            friends_status,
                            blocked_users,
                            block_status,
                        );
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
    mut chat_messages: Signal<Vec<ChatMessage>>,
    mut chat_status: Signal<String>,
    mut presence_counts: Signal<PresenceCounts>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
) {
    if !session_generation_current(session_generation, generation) {
        return;
    }

    match serde_json::from_str::<ServerEvent>(text) {
        Ok(ServerEvent::HelloOk { presence, .. }) => {
            presence_counts.set(presence);
            chat_status.set("Global chat connected".to_string());
        }
        Ok(ServerEvent::GlobalMessageCreated(message)) => {
            let mut messages = chat_messages.write();
            if messages.iter().any(|existing| existing.id == message.id) {
                return;
            }
            messages.push(message);
            let overflow = messages.len().saturating_sub(200);
            if overflow > 0 {
                messages.drain(0..overflow);
            }
        }
        Ok(ServerEvent::GlobalMessageDeleted { message_id }) => {
            let mut messages = chat_messages.write();
            remove_message_by_id(&mut messages, message_id);
        }
        Ok(ServerEvent::UserMessagesHidden { user_id }) => {
            let mut messages = chat_messages.write();
            remove_blocked_user_messages(&mut messages, user_id);
        }
        Ok(ServerEvent::RelationshipStateChanged) => {
            refresh_relationship_state(
                session,
                session_generation,
                friendships,
                friends_status,
                blocked_users,
                block_status,
            );
        }
        Ok(ServerEvent::Error { message }) => chat_status.set(format!("Chat error: {message}")),
        Ok(ServerEvent::MinimumVersionRequired {
            minimum_client_version,
        }) => chat_status.set(format!(
            "Client update required. Minimum version: {minimum_client_version}"
        )),
        Ok(ServerEvent::PresenceCounts(counts)) => presence_counts.set(counts),
        Err(error) => chat_status.set(format!("Could not read chat event: {error}")),
    }
}

#[cfg(windows)]
fn remove_message_by_id(messages: &mut Vec<ChatMessage>, message_id: uuid::Uuid) {
    messages.retain(|message| message.id != message_id);
}

#[cfg(windows)]
fn send_pending_global_message(
    mut composer_body: Signal<String>,
    mut chat_status: Signal<String>,
    chat_outbox: Signal<Option<UnboundedSender<ClientEvent>>>,
) {
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
        Some(sender) => match sender.send(ClientEvent::GlobalMessageSend { body }) {
            Ok(()) => {
                composer_body.set(String::new());
                chat_status.set("Message sent".to_string());
            }
            Err(_) => chat_status.set("Chat connection is not available".to_string()),
        },
        None => chat_status.set("Chat connection is not available".to_string()),
    }
}

#[cfg(windows)]
async fn complete_login() -> Result<AuthSession, String> {
    if is_local_server_url() {
        complete_dev_login().await
    } else {
        complete_steam_login().await
    }
}

#[cfg(windows)]
async fn complete_dev_login() -> Result<AuthSession, String> {
    let client = reqwest::Client::new();
    let mut request = client.post(format!("{}/auth/dev/login", server_url()));
    if let Some(display_name) = dev_display_name() {
        request = request.json(&DevLoginRequest {
            display_name: Some(display_name),
        });
    }

    let response = request
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
    let response = reqwest::Client::new()
        .post(format!("{}/auth/steam/device/start", server_url()))
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
    let response = reqwest::Client::new()
        .post(format!("{}/auth/steam/device/poll", server_url()))
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
async fn refresh_session(refresh_token: String) -> Result<AuthSession, String> {
    let response = reqwest::Client::new()
        .post(format!("{}/auth/refresh", server_url()))
        .json(&RefreshRequest { refresh_token })
        .send()
        .await
        .map_err(|error| format!("Could not refresh saved session: {error}"))?;

    if !response.status().is_success() {
        return Err(api_error(response, "Saved session refresh failed").await);
    }

    response
        .json::<RefreshResponse>()
        .await
        .map(|response| response.session)
        .map_err(|error| format!("Could not read session refresh response: {error}"))
}

#[cfg(windows)]
async fn logout_session(refresh_token: String) -> Result<(), String> {
    let response = reqwest::Client::new()
        .post(format!("{}/auth/logout", server_url()))
        .json(&LogoutRequest { refresh_token })
        .send()
        .await
        .map_err(|error| format!("Could not contact Agora server: {error}"))?;

    if response.status().is_success() {
        Ok(())
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
    let response = reqwest::Client::new()
        .get(format!("{}/users/search", server_url()))
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
    let response = reqwest::Client::new()
        .get(format!("{}{}", server_url(), path))
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
    let response = reqwest::Client::new()
        .post(format!("{}{}", server_url(), path))
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
    let response = reqwest::Client::new()
        .delete(format!("{}{}", server_url(), path))
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
fn store_refresh_token(refresh_token: &str) -> Result<(), String> {
    refresh_token_entry()?
        .set_password(refresh_token)
        .map_err(|error| format!("Windows Credential Manager write failed: {error}"))
}

#[cfg(windows)]
fn load_refresh_token() -> Result<Option<String>, String> {
    match refresh_token_entry()?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::v1::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("Windows Credential Manager read failed: {error}")),
    }
}

#[cfg(windows)]
fn clear_refresh_token() -> Result<(), String> {
    match refresh_token_entry()?.delete_credential() {
        Ok(()) | Err(keyring::v1::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("Windows Credential Manager delete failed: {error}")),
    }
}

#[cfg(windows)]
fn refresh_token_entry() -> Result<keyring::v1::Entry, String> {
    let user = refresh_token_user();
    keyring::v1::Entry::new(KEYRING_SERVICE, &user)
        .map_err(|error| format!("Windows Credential Manager is unavailable: {error}"))
}

#[cfg(windows)]
fn refresh_token_user() -> String {
    if is_local_server_url() {
        if let Some(display_name) = dev_display_name() {
            return format!(
                "{}:{}",
                KEYRING_REFRESH_TOKEN_USER,
                credential_safe_name(&display_name)
            );
        }
    }
    KEYRING_REFRESH_TOKEN_USER.to_string()
}

#[cfg(windows)]
fn credential_safe_name(value: &str) -> String {
    let normalized = value
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    if normalized.is_empty() {
        "local".to_string()
    } else {
        normalized
    }
}

#[cfg(windows)]
fn server_url() -> String {
    std::env::var("AGORA_SERVER_URL")
        .unwrap_or_else(|_| {
            option_env!("AGORA_DEFAULT_SERVER_URL")
                .unwrap_or("http://localhost")
                .to_string()
        })
        .trim_end_matches('/')
        .to_string()
}

#[cfg(windows)]
fn websocket_url() -> String {
    let server_url = server_url();
    let websocket_base = if let Some(rest) = server_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = server_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        server_url
    };

    format!("{websocket_base}/ws")
}

#[cfg(windows)]
fn dev_display_name() -> Option<String> {
    std::env::var("AGORA_DEV_DISPLAY_NAME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(windows)]
fn is_local_server_url() -> bool {
    let server_url = server_url();
    matches!(server_host(&server_url), "localhost" | "127.0.0.1" | "::1")
}

#[cfg(windows)]
fn server_host(server_url: &str) -> &str {
    let authority = server_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(server_url)
        .split('/')
        .next()
        .unwrap_or_default();
    if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if authority == "::1" {
        authority
    } else {
        authority.split(':').next().unwrap_or(authority)
    }
}

#[cfg(windows)]
fn login_start_message() -> String {
    if is_local_server_url() {
        "Starting local dev session...".to_string()
    } else {
        "Requesting Steam login...".to_string()
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
    active: AppTab,
    mut active_tab: Signal<AppTab>,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    auth_action_pending: Signal<bool>,
    auth_session: Signal<Option<AuthSession>>,
    login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    composer_body: Signal<String>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    chat_outbox: Signal<Option<UnboundedSender<ClientEvent>>>,
    shell_mode: Signal<ShellMode>,
    game_window: Signal<Option<game::GameWindow>>,
    overlay_interactive: Signal<bool>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friend_search_query: Signal<String>,
    friend_search_results: Signal<Vec<UserSummary>>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_search_query: Signal<String>,
    block_search_results: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
    report_reason: Signal<String>,
    report_details: Signal<String>,
    report_draft: Signal<Option<ReportDraft>>,
    report_status: Signal<String>,
) -> Element {
    let overlay_tab = if matches!(active, AppTab::Friends | AppTab::BlockReport) {
        active
    } else {
        AppTab::Global
    };
    let friend_badge = session
        .as_ref()
        .map(|session| incoming_friend_request_count(&friendships.read(), session.user.id))
        .unwrap_or(0);
    let content = match overlay_tab {
        AppTab::Friends => rsx! {
            {friends_panel(
                session,
                session_generation,
                friendships,
                friend_search_query,
                friend_search_results,
                friends_status,
            )}
        },
        AppTab::BlockReport => rsx! {
            {block_report_panel(
                session,
                session_generation,
                chat_messages,
                friendships,
                friends_status,
                blocked_users,
                block_search_query,
                block_search_results,
                block_status,
                report_reason,
                report_details,
                report_draft,
                report_status,
            )}
        },
        AppTab::Global => rsx! {
            {overlay_global_chat_panel(
                session,
                active_tab,
                session_generation,
                auth_action_pending,
                auth_session,
                login_status,
                chat_messages,
                composer_body,
                chat_status,
                presence_counts,
                chat_outbox,
                friendships,
                friend_search_query,
                friend_search_results,
                friends_status,
                blocked_users,
                block_search_query,
                block_search_results,
                block_status,
                report_reason,
                report_details,
                report_draft,
                report_status,
            )}
        },
    };
    let escape_desktop = desktop.clone();

    rsx! {
        main { class: "chat-mode-overlay",
            onkeydown: move |event| {
                if event.key() == Key::Escape {
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
                    "Global"
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
                {content}
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
    auth_action_pending: Signal<bool>,
    auth_session: Signal<Option<AuthSession>>,
    login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    mut composer_body: Signal<String>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    chat_outbox: Signal<Option<UnboundedSender<ClientEvent>>>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friend_search_query: Signal<String>,
    friend_search_results: Signal<Vec<UserSummary>>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_search_query: Signal<String>,
    block_search_results: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
    report_reason: Signal<String>,
    report_details: Signal<String>,
    report_draft: Signal<Option<ReportDraft>>,
    report_status: Signal<String>,
) -> Element {
    let chat_status_text = chat_status.read().clone();
    let composer_text = composer_body.read().clone();
    let connected = chat_outbox.read().is_some();
    let signed_in = session.is_some();
    let current_user_id = session.as_ref().map(|session| session.user.id);
    let can_send = signed_in && connected && !composer_text.trim().is_empty();
    let auth_action_pending_value = *auth_action_pending.read();
    let auth_button_label = if signed_in { "Sign off" } else { "Sign in" };
    let mut recent_messages = chat_messages
        .read()
        .iter()
        .rev()
        .take(10)
        .cloned()
        .collect::<Vec<_>>();
    recent_messages.reverse();

    rsx! {
        div { class: "overlay-global-panel",
            header { class: "overlay-panel-heading",
                div {
                    h2 { "Global" }
                    p { "Last 10 messages" }
                }
                div { class: "overlay-panel-actions",
                    span { class: "chat-status-inline", "{chat_status_text}" }
                    button {
                        class: "overlay-auth-button",
                        r#type: "button",
                        disabled: auth_action_pending_value,
                        onclick: move |_| {
                            if let Some(session) = session.clone() {
                                sign_out_session(
                                    session,
                                    auth_action_pending,
                                    auth_session,
                                    session_generation,
                                    login_status,
                                    chat_messages,
                                    chat_status,
                                    presence_counts,
                                    chat_outbox,
                                    friendships,
                                    friend_search_query,
                                    friend_search_results,
                                    friends_status,
                                    blocked_users,
                                    block_search_query,
                                    block_search_results,
                                    block_status,
                                    report_reason,
                                    report_details,
                                    report_draft,
                                    report_status,
                                );
                            } else {
                                sign_in_session(
                                    auth_action_pending,
                                    auth_session,
                                    session_generation,
                                    login_status,
                                    chat_messages,
                                    chat_status,
                                    presence_counts,
                                    chat_outbox,
                                    friendships,
                                    friends_status,
                                    blocked_users,
                                    block_status,
                                    report_draft,
                                );
                            }
                        },
                        "{auth_button_label}"
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
                }
            }
            form {
                class: "overlay-composer",
                onsubmit: move |event| {
                    event.prevent_default();
                    if can_send {
                        send_pending_global_message(composer_body, chat_status, chat_outbox);
                    }
                },
                input {
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
                    "Send"
                }
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
}

#[cfg(windows)]
fn friends_panel(
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    friendships: Signal<Vec<FriendshipSummary>>,
    mut friend_search_query: Signal<String>,
    friend_search_results: Signal<Vec<UserSummary>>,
    friends_status: Signal<String>,
) -> Element {
    let status = friends_status.read().clone();
    let query = friend_search_query.read().clone();
    let results = friend_search_results.read().clone();
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
    let search_session = session.clone();

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
                                load_friends(session, session_generation, friendships, friends_status);
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
                div { class: "search-row",
                    input {
                        placeholder: "Search users",
                        value: "{query}",
                        disabled: !can_use,
                        oninput: move |event| friend_search_query.set(event.value())
                    }
                    button {
                        disabled: !can_use,
                        onclick: move |_| {
                            if let Some(session) = search_session.clone() {
                                search_friend_users(
                                    session,
                                    session_generation,
                                    friend_search_query.read().clone(),
                                    friend_search_results,
                                    friends_status,
                                );
                            }
                        },
                        "Search"
                    }
                }
                span { class: "panel-status", "{status}" }
            }

            section { class: "relationship-section",
                h3 { "Search Results" }
                if results.is_empty() {
                    p { class: "muted-copy", "No user search results yet." }
                } else {
                    div { class: "relationship-list",
                        for user in results {
                            {friend_search_result_row(user, session.clone(), session_generation, friendships, friends_status)}
                        }
                    }
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
                    friendships,
                    friends_status,
                )}
                {friendship_section(
                    "Friends",
                    accepted,
                    "No accepted friends yet.",
                    current_user.clone(),
                    session.clone(),
                    session_generation,
                    friendships,
                    friends_status,
                )}
                {friendship_section(
                    "Sent Requests",
                    outgoing,
                    "No outgoing friend requests.",
                    current_user.clone(),
                    session.clone(),
                    session_generation,
                    friendships,
                    friends_status,
                )}
                if !inactive.is_empty() {
                    {friendship_section(
                        "Inactive Records",
                        inactive,
                        "No inactive friend records.",
                        current_user,
                        session.clone(),
                        session_generation,
                        friendships,
                        friends_status,
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
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
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
                        {friendship_row(friendship, current_user.clone(), session.clone(), session_generation, friendships, friends_status)}
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn friend_search_result_row(
    user: UserSummary,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
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
                            send_friend_invite(
                                session,
                                session_generation,
                                user.clone(),
                                friendships,
                                friends_status,
                            );
                        }
                    },
                    "Add Friend"
                }
            }
        }
    }
}

#[cfg(windows)]
fn friendship_row(
    friendship: FriendshipSummary,
    current_user: UserSummary,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
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
                        class: "secondary-button compact danger",
                        disabled: actions_disabled,
                        onclick: move |_| {
                            if let Some(session) = remove_session.clone() {
                                remove_friend(
                                    session,
                                    session_generation,
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
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    mut block_search_query: Signal<String>,
    block_search_results: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
    mut report_reason: Signal<String>,
    mut report_details: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_status: Signal<String>,
) -> Element {
    let query = block_search_query.read().clone();
    let results = block_search_results.read().clone();
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
    let search_session = session.clone();
    let submit_session = session.clone();
    let submit_report = selected_report.clone();

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
                                load_blocks(session, session_generation, blocked_users, block_status);
                            }
                        },
                        "Refresh Blocks"
                    }
                }
                p { "A block is one-way, but it hides global chat communication in both directions." }
                div { class: "search-row",
                    input {
                        placeholder: "Search users to block or report",
                        value: "{query}",
                        disabled: !can_use,
                        oninput: move |event| block_search_query.set(event.value())
                    }
                    button {
                        disabled: !can_use,
                        onclick: move |_| {
                            if let Some(session) = search_session.clone() {
                                search_block_report_users(
                                    session,
                                    session_generation,
                                    block_search_query.read().clone(),
                                    block_search_results,
                                    block_status,
                                );
                            }
                        },
                        "Search"
                    }
                }
                span { class: "panel-status", "{block_status_text}" }
            }

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
                span { class: "panel-status", "{report_status_text}" }
            }

            section { class: "relationship-section",
                h3 { "Search Results" }
                if results.is_empty() {
                    p { class: "muted-copy", "No user search results yet." }
                } else {
                    div { class: "relationship-list",
                        for user in results {
                            {block_report_user_row(
                                user,
                                session.clone(),
                                session_generation,
                                chat_messages,
                                friendships,
                                friends_status,
                                blocked_users,
                                block_status,
                                report_draft,
                                report_status,
                            )}
                        }
                    }
                }
            }

            section { class: "relationship-section",
                h3 { "Blocked Users" }
                if blocked.is_empty() {
                    p { class: "muted-copy", "No blocked users." }
                } else {
                    div { class: "relationship-list",
                        for user in blocked {
                            {blocked_user_row(user, session.clone(), session_generation, blocked_users, block_status)}
                        }
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn block_report_user_row(
    user: UserSummary,
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    chat_messages: Signal<Vec<ChatMessage>>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_status: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_status: Signal<String>,
) -> Element {
    let user_id = user.id.to_string();
    let display_name = user.display_name.clone();
    let actions_disabled = session.is_none();
    let block_session = session.clone();
    let block_target = user.clone();
    let report_target = user.clone();

    rsx! {
        article { key: "{user_id}", class: "relationship-row",
            div { class: "relationship-main",
                strong { "{display_name}" }
                span { "{user_id}" }
            }
            div { class: "relationship-actions",
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
                                blocked_users,
                                block_status,
                                friendships,
                                friends_status,
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
input {
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
    min-width: 0;
    height: calc(100vh - 28px);
    min-height: 0;
    overflow: hidden;
    padding: 14px;
    border: 1px solid var(--aom-frame);
    border-radius: 4px;
    background: var(--aom-bg);
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

.overlay-auth-button {
    padding: 5px 9px;
    border: 1px solid var(--aom-frame-bright);
    border-radius: 999px;
    color: #1b1207;
    background: linear-gradient(180deg, #d6b45b, #7c5e23);
    font-size: 11px;
    font-weight: 800;
    cursor: pointer;
}

.overlay-message-list {
    display: flex;
    min-height: 0;
    overflow-y: auto;
    flex-direction: column;
    gap: 6px;
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

button:disabled,
input:disabled {
    cursor: not-allowed;
    opacity: 0.55;
}

@media (max-width: 760px) {
    .relationship-row,
    .search-row {
        grid-template-columns: 1fr;
    }

    .relationship-actions {
        flex-wrap: wrap;
    }
}
"#;

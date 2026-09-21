#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("Agora currently supports Windows desktop.");
}

#[cfg(windows)]
mod api;
#[cfg(windows)]
mod auth;
#[cfg(windows)]
mod autocomplete;
#[cfg(windows)]
mod chat;
#[cfg(windows)]
mod direct_messages;
#[cfg(windows)]
mod game;
#[cfg(windows)]
mod relationships;
#[cfg(windows)]
mod server;
#[cfg(windows)]
mod session;
#[cfg(windows)]
mod shell;
#[cfg(windows)]
mod state;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod update;
#[cfg(windows)]
mod updates;

#[cfg(windows)]
use server::{
    configured_local_dev_account_id, standalone_local_dev_window_enabled, RemoteLoginProvider,
};
#[cfg(windows)]
use session::start_saved_session_restore;

#[cfg(windows)]
use shell::{
    hide_to_tray, initial_shell_mode, register_global_shortcuts, watch_game_window,
    watch_tray_events, ShellMode,
};

#[cfg(windows)]
use state::{
    AppTab, DirectMessageSignals, OutgoingChatEvent, PresenceSelectionSignals, ReportDraft,
    UpdateSignals, UserAutocompleteSignals,
};

#[cfg(windows)]
use ui::{chat_mode_overlay_view, passive_overlay_view};

#[cfg(windows)]
use updates::{check_for_updates, initial_update_status};

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
    AuthSession, ChatMessage, DmMessage, DmThreadSummary, FriendshipSummary, PresenceCounts,
    PresenceState, UserSummary,
};
#[cfg(windows)]
use dioxus::prelude::*;
#[cfg(windows)]
use tokio::sync::mpsc::{self, UnboundedSender};

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
    let login_status = use_signal(|| "Checking saved session...".to_string());
    let auth_action_pending = use_signal(|| true);
    let auth_session = use_signal(|| None::<AuthSession>);
    let reauth_required = use_signal(|| None::<String>);
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
        start_saved_session_restore(
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
            updates,
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
            direct_messages,
        );
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
const STYLE: &str = include_str!("style.css");

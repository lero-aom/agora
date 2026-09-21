#[cfg(windows)]
use super::{
    autocomplete::{
        dismiss_user_autocomplete, move_user_autocomplete_active_option,
        select_user_autocomplete_option, show_user_autocomplete, update_user_autocomplete_query,
        user_autocomplete_query_ready,
    },
    chat::{
        can_update_presence_selection, presence_selection_value,
        presence_state_from_selection_value, send_pending_global_message,
        update_presence_selection,
    },
    direct_messages::{
        create_direct_message_thread, load_direct_message_history_page,
        load_direct_message_threads, open_direct_message_thread, send_direct_message,
    },
    game,
    relationships::{
        accept_friend_invite, block_user_action, decline_friend_invite,
        direct_message_report_draft, global_message_report_draft, load_blocks, load_friends,
        remove_friend, report_draft_kind_label, report_draft_status, report_preview_text,
        send_friend_invite, submit_report_action, unblock_user_action, user_report_draft,
    },
    server::{is_local_server_url, server_url, RemoteLoginProvider},
    session::{sign_in_session, sign_out_session},
    shell::{close_interaction_or_hide, ShellMode},
    state::{
        AppTab, DirectMessageSignals, FriendSections, OutgoingChatEvent, PresenceSelectionSignals,
        RelationshipSignals, ReportDraft, UpdateSignals, UserAutocompleteSignals,
    },
    update,
    updates::{check_for_updates, install_available_update},
};

#[cfg(all(test, windows))]
use super::{
    api::{bearer_access_error, BearerAccess},
    auth::{refresh_session_is_invalid, refresh_token_user},
    autocomplete::next_user_autocomplete_active_index,
    direct_messages::{merge_direct_messages, upsert_direct_message_thread},
    relationships::remove_blocked_user_messages,
    server::{
        is_loopback_server_url, local_dev_account_id_or_default, parse_server_origin,
        standalone_local_dev_window_enabled_for,
    },
    session::session_refresh_retry_delay,
    shell::initial_shell_mode,
};

#[cfg(all(test, windows))]
use agora_common::MessageKind;
#[cfg(windows)]
use agora_common::{
    AuthSession, ChatMessage, DmMessage, DmThreadSummary, FriendshipStatus, FriendshipSummary,
    PresenceCounts, UserSummary,
};
#[cfg(windows)]
use dioxus::prelude::*;
#[cfg(windows)]
use tokio::sync::mpsc::UnboundedSender;

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
pub(super) fn chat_mode_overlay_view(
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
    use super::{
        bearer_access_error, direct_message_report_draft, friend_sections,
        global_message_report_draft, incoming_friend_request_count, initial_shell_mode,
        is_loopback_server_url, local_dev_account_id_or_default, merge_direct_messages,
        next_user_autocomplete_active_index, parse_server_origin, refresh_session_is_invalid,
        refresh_token_user, remove_blocked_user_messages, session_refresh_retry_delay,
        standalone_local_dev_window_enabled_for, upsert_direct_message_thread,
        user_autocomplete_query_ready, BearerAccess, ChatMessage, DmMessage, DmThreadSummary,
        FriendshipStatus, FriendshipSummary, MessageKind, ShellMode, UserSummary,
    };

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
pub(super) fn passive_overlay_view(
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

use super::{
    api::{allow_bearer_access, begin_bearer_compatibility_check, block_bearer_access},
    auth::clear_refresh_token,
    direct_messages::{
        load_direct_message_threads, merge_direct_messages, refresh_direct_message_state,
        remove_direct_messages_for_user, reset_direct_message_state, upsert_direct_message_thread,
    },
    relationships::{refresh_relationship_state, remove_blocked_user_messages},
    server::websocket_url,
    state::{
        next_session_generation, session_generation_current, ChatSessionSignals,
        DirectMessageSignals, OutgoingChatEvent, PresenceSelectionSignals,
    },
    updates::request_required_update,
};
use agora_common::{
    AuthSession, ChatMessage, ClientEvent, DmRealtimeEvent, PresenceCounts, PresenceState,
    ServerEvent, MAX_MESSAGE_LEN, PROTOCOL_VERSION,
};
use dioxus::prelude::{spawn, Readable, Signal, Writable};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    oneshot,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::{header::AUTHORIZATION, HeaderValue},
        Message,
    },
};

const CHAT_RECONNECT_SAFETY_SECONDS: u64 = 30;
const DELETED_MESSAGE_CACHE_LIMIT: usize = 500;

pub(super) fn start_chat_session(session: &AuthSession, signals: ChatSessionSignals) {
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

pub(super) fn stop_chat_session(
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

fn chat_reconnect_window(expires_in_seconds: u64) -> u64 {
    expires_in_seconds
        .saturating_sub(CHAT_RECONNECT_SAFETY_SECONDS)
        .max(1)
}

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

fn terminal_chat_error(message: &str) -> bool {
    matches!(
        message,
        "Chat session ended; sign in again" | "Account status changed; sign in again"
    )
}

fn remove_message_by_id(messages: &mut Vec<ChatMessage>, message_id: uuid::Uuid) {
    messages.retain(|message| message.id != message_id);
}

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

pub(super) fn send_pending_global_message(
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

pub(super) fn presence_selection_value(state: PresenceState) -> &'static str {
    match state {
        PresenceState::Online | PresenceState::Offline => "online",
        PresenceState::LookingForGame => "looking_for_game",
        PresenceState::InGame => "in_game",
    }
}

pub(super) fn presence_state_from_selection_value(value: &str) -> Option<PresenceState> {
    match value {
        "online" => Some(PresenceState::Online),
        "looking_for_game" => Some(PresenceState::LookingForGame),
        "in_game" => Some(PresenceState::InGame),
        _ => None,
    }
}

fn presence_selection_label(state: PresenceState) -> &'static str {
    match state {
        PresenceState::Online | PresenceState::Offline => "Online",
        PresenceState::LookingForGame => "Looking for game",
        PresenceState::InGame => "In game",
    }
}

fn presence_selection_status(state: PresenceState) -> String {
    format!("Availability: {}", presence_selection_label(state))
}

pub(super) fn can_update_presence_selection(
    signed_in: bool,
    connected: bool,
    pending: bool,
) -> bool {
    signed_in && connected && !pending
}

fn next_presence_selection_request_generation(mut signals: PresenceSelectionSignals) -> u64 {
    let next = (*signals.request_generation.read()).wrapping_add(1);
    signals.request_generation.set(next);
    next
}

fn presence_selection_request_is_current(
    signals: PresenceSelectionSignals,
    generation: u64,
) -> bool {
    *signals.request_generation.read() == generation
}

fn reset_presence_selection(mut signals: PresenceSelectionSignals, status: &str) {
    next_presence_selection_request_generation(signals);
    signals.selected.set(PresenceState::Online);
    signals.confirmed.set(PresenceState::Online);
    signals.unconfirmed.set(None);
    signals.pending.set(false);
    signals.connected.set(false);
    signals.status.set(status.to_string());
}

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

fn unconfirmed_presence_selection_status(state: PresenceState) -> String {
    format!(
        "Availability requested as {}. The server has not confirmed it.",
        presence_selection_label(state)
    )
}

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

pub(super) fn update_presence_selection(
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

#[cfg(all(test, windows))]
mod chat_tests {
    use super::*;
    use agora_common::UserSummary;

    fn user(id: u128, display_name: &str) -> UserSummary {
        UserSummary {
            id: uuid::Uuid::from_u128(id),
            display_name: display_name.to_string(),
            avatar_url: None,
        }
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
}

fn restore_composer_after_transport_failure(composer: &mut String, body: &str) -> bool {
    if composer.is_empty() {
        composer.push_str(body);
        true
    } else {
        false
    }
}

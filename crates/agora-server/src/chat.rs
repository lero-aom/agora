use std::{
    cmp::Ordering,
    collections::HashSet,
    net::SocketAddr,
    sync::{atomic::Ordering as AtomicOrdering, Arc},
    time::Duration,
};

use agora_common::{
    ApiError, ChatMessage, ClientEvent, PresenceState, ServerEvent, UserSummary, MAX_MESSAGE_LEN,
    PROTOCOL_VERSION,
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use serde::Deserialize;
use sqlx::{postgres::PgRow, Row};
use tokio::sync::broadcast::{self, error::RecvError};
use tracing::warn;
use uuid::Uuid;

use crate::{auth, visibility, AppState};

const RECENT_MESSAGE_LIMIT: i64 = 50;
const HELLO_TIMEOUT_SECONDS: u64 = 10;
const SESSION_REVALIDATE_SECONDS: u64 = 30;
const MAX_CACHED_HIDDEN_VIEWERS: usize = 1_024;

type WsSender = SplitSink<WebSocket, Message>;
type WsReceiver = SplitStream<WebSocket>;

#[derive(Debug, Clone)]
pub(crate) struct RealtimeEvent {
    audience: RealtimeAudience,
    payload: RealtimePayload,
}

#[derive(Debug, Clone)]
enum RealtimeAudience {
    Public,
    User(Uuid),
    Session(Uuid),
}

#[derive(Debug, Clone)]
enum RealtimePayload {
    Event(ServerEvent),
    GlobalMessageCreated {
        message: ChatMessage,
        hidden_viewer_ids: Option<Arc<HashSet<Uuid>>>,
        visibility_epoch: u64,
    },
    Disconnect {
        message: String,
    },
}

enum Delivery {
    Send(ServerEvent),
    Disconnect(String),
    Skip,
}

#[derive(Deserialize)]
struct ClientHelloEnvelope {
    #[serde(rename = "type")]
    event_type: String,
    payload: Option<ClientHelloPayload>,
}

#[derive(Deserialize)]
struct ClientHelloPayload {
    client_version: Option<String>,
    protocol_version: Option<u16>,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/ws", get(websocket))
}

pub(crate) fn broadcast_channel() -> broadcast::Sender<RealtimeEvent> {
    let (sender, _) = broadcast::channel(256);
    sender
}

pub(crate) fn send_session_revoked(tx: &broadcast::Sender<RealtimeEvent>, session_id: Uuid) {
    let _ = tx.send(RealtimeEvent {
        audience: RealtimeAudience::Session(session_id),
        payload: RealtimePayload::Disconnect {
            message: "Chat session ended; sign in again".to_string(),
        },
    });
}

fn send_public_event(tx: &broadcast::Sender<RealtimeEvent>, event: ServerEvent) {
    let _ = tx.send(RealtimeEvent {
        audience: RealtimeAudience::Public,
        payload: RealtimePayload::Event(event),
    });
}

pub(crate) fn send_user_event(
    tx: &broadcast::Sender<RealtimeEvent>,
    user_id: Uuid,
    event: ServerEvent,
) {
    let _ = tx.send(RealtimeEvent {
        audience: RealtimeAudience::User(user_id),
        payload: RealtimePayload::Event(event),
    });
}

pub(crate) fn send_user_disconnect(
    tx: &broadcast::Sender<RealtimeEvent>,
    user_id: Uuid,
    message: impl Into<String>,
) {
    let _ = tx.send(RealtimeEvent {
        audience: RealtimeAudience::User(user_id),
        payload: RealtimePayload::Disconnect {
            message: message.into(),
        },
    });
}

pub(crate) fn send_global_message_deleted(tx: &broadcast::Sender<RealtimeEvent>, message_id: Uuid) {
    send_public_event(tx, ServerEvent::GlobalMessageDeleted { message_id });
}

pub(crate) async fn send_user_global_snapshot_locked(
    state: &AppState,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    let messages = recent_global_messages(state, user_id).await?;
    send_user_event(
        &state.chat_tx,
        user_id,
        ServerEvent::GlobalMessageSnapshot { messages },
    );
    Ok(())
}

async fn send_global_message_created_locked(state: &AppState, message: ChatMessage) {
    let visibility_epoch = state.visibility_epoch.load(AtomicOrdering::Acquire);
    let hidden_viewer_ids = match hidden_viewer_ids_for_author(&state.db, message.author.id).await {
        Ok(hidden_viewer_ids) if hidden_viewer_ids.len() <= MAX_CACHED_HIDDEN_VIEWERS => {
            Some(Arc::new(hidden_viewer_ids))
        }
        Ok(_) => None,
        Err(error) => {
            warn!(%error, "failed to cache global message visibility; using per-recipient checks");
            None
        }
    };
    let _ = state.chat_tx.send(RealtimeEvent {
        audience: RealtimeAudience::Public,
        payload: RealtimePayload::GlobalMessageCreated {
            message,
            hidden_viewer_ids,
            visibility_epoch,
        },
    });
}

async fn websocket(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(error) = state
        .rate_limits
        .check_auth(peer_addr, &headers, state.config.trust_proxy_headers)
        .await
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiError {
                message: error.message(),
            }),
        )
            .into_response();
    }

    let Some(access_token) = auth::bearer_token(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                message: "missing bearer token".to_string(),
            }),
        )
            .into_response();
    };

    match auth::user_for_access_token(&state, access_token).await {
        Ok(Some(session)) => ws
            .on_upgrade(move |socket| websocket_session(state, socket, session))
            .into_response(),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                message: "chat session is invalid or expired".to_string(),
            }),
        )
            .into_response(),
        Err(error) => {
            warn!(%error, "failed to validate WebSocket access token");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    message: "database operation failed".to_string(),
                }),
            )
                .into_response()
        }
    }
}

async fn websocket_session(
    state: AppState,
    socket: WebSocket,
    session: auth::AuthenticatedSession,
) {
    let user_id = session.user.id;

    if !websocket_session_inner(state.clone(), socket, session).await {
        return;
    }

    let presence = state.presence.disconnect(user_id).await;
    send_public_event(&state.chat_tx, ServerEvent::PresenceCounts(presence));
}

async fn websocket_session_inner(
    state: AppState,
    socket: WebSocket,
    session: auth::AuthenticatedSession,
) -> bool {
    let (mut sender, mut receiver) = socket.split();

    if !receive_hello(&state, &mut sender, &mut receiver).await {
        return false;
    }
    if !ensure_session_active(&state, &mut sender, &session).await {
        return false;
    }

    let initial_presence = state.presence.connect(session.user.id).await;
    send_public_event(
        &state.chat_tx,
        ServerEvent::PresenceCounts(initial_presence),
    );
    if send_event(
        &mut sender,
        &ServerEvent::HelloOk {
            protocol_version: PROTOCOL_VERSION,
            presence: initial_presence,
        },
    )
    .await
    .is_err()
    {
        return true;
    }

    let mut broadcast_rx = match subscribe_with_global_snapshot(&state, &session).await {
        Ok(receiver) => receiver,
        Err(error) => {
            warn!(%error, "failed to load recent global messages");
            let _ = send_error(&mut sender, "Could not load recent chat history").await;
            return true;
        }
    };

    let access_token_expires =
        tokio::time::sleep(Duration::from_secs(session.access_token_ttl_seconds));
    tokio::pin!(access_token_expires);
    let mut session_revalidate =
        tokio::time::interval(Duration::from_secs(SESSION_REVALIDATE_SECONDS));

    loop {
        tokio::select! {
            _ = &mut access_token_expires => {
                let _ = send_event(&mut sender, &ServerEvent::AccessTokenExpired).await;
                break;
            }
            _ = session_revalidate.tick() => {
                if !ensure_session_active(&state, &mut sender, &session).await {
                    break;
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(error) = state.rate_limits.check_chat_event(session.user.id).await {
                            let _ = send_error(&mut sender, &error.message()).await;
                            break;
                        }
                        if !ensure_session_active(&state, &mut sender, &session).await {
                            break;
                        }
                        handle_text_message(&state, &mut sender, &session.user, text.as_str()).await;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if let Err(error) = state.rate_limits.check_chat_event(session.user.id).await {
                            let _ = send_error(&mut sender, &error.message()).await;
                            break;
                        }
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        warn!(%error, "WebSocket receive failed");
                        break;
                    }
                }
            }
            broadcast = broadcast_rx.recv() => {
                match broadcast {
                    Ok(event) => {
                        match event_delivery(&state, &session, &event).await {
                            Delivery::Send(event) => {
                                if !ensure_session_active(&state, &mut sender, &session).await {
                                    break;
                                }
                                if send_event(&mut sender, &event).await.is_err() {
                                    break;
                                }
                            }
                            Delivery::Disconnect(message) => {
                                let _ = send_error(&mut sender, &message).await;
                                break;
                            }
                            Delivery::Skip => {}
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        warn!(skipped, "chat client lagged behind broadcast stream");
                        if !ensure_session_active(&state, &mut sender, &session).await {
                            break;
                        }
                        broadcast_rx = match subscribe_with_global_snapshot(&state, &session).await {
                            Ok(receiver) => receiver,
                            Err(error) => {
                                warn!(%error, "failed to reload recent global messages after broadcast lag");
                                let _ = send_error(&mut sender, "Could not load recent chat history").await;
                                break;
                            }
                        };
                        if send_event(&mut sender, &ServerEvent::RelationshipStateChanged).await.is_err()
                            || send_event(
                                &mut sender,
                                &ServerEvent::PresenceCounts(state.presence.counts().await),
                            )
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }

    true
}

async fn receive_hello(state: &AppState, sender: &mut WsSender, receiver: &mut WsReceiver) -> bool {
    let timeout = tokio::time::sleep(Duration::from_secs(HELLO_TIMEOUT_SECONDS));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            _ = &mut timeout => {
                let _ = send_error(sender, "Chat hello timed out").await;
                return false;
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let hello = match parse_client_hello(text.as_str()) {
                            Ok(hello) => hello,
                            Err(error) => {
                                warn!(%error, "failed to parse client chat hello");
                                let _ = send_error(sender, "Invalid chat hello").await;
                                return false;
                            }
                        };

                        if hello.protocol_version != Some(PROTOCOL_VERSION) {
                            let _ = send_event(
                                sender,
                                &ServerEvent::MinimumVersionRequired {
                                    minimum_client_version: state.config.minimum_client_version.clone(),
                                },
                            )
                            .await;
                            let _ = send_event(
                                sender,
                                &ServerEvent::ProtocolIncompatible {
                                    required_protocol_version: PROTOCOL_VERSION,
                                },
                            )
                            .await;
                            return false;
                        }

                        if client_version_meets_minimum(
                            hello.client_version.as_deref().unwrap_or_default(),
                            &state.config.minimum_client_version,
                        ) {
                            return true;
                        }

                        let _ = send_event(
                            sender,
                            &ServerEvent::MinimumVersionRequired {
                                minimum_client_version: state.config.minimum_client_version.clone(),
                            },
                        )
                        .await;
                        return false;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            return false;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return false,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        warn!(%error, "WebSocket hello receive failed");
                        return false;
                    }
                }
            }
        }
    }
}

async fn ensure_session_active(
    state: &AppState,
    sender: &mut WsSender,
    session: &auth::AuthenticatedSession,
) -> bool {
    match auth::session_activity(state, session.session_id).await {
        Ok(auth::SessionActivity::Active) => true,
        Ok(auth::SessionActivity::AccessTokenExpired) => {
            let _ = send_event(sender, &ServerEvent::AccessTokenExpired).await;
            false
        }
        Ok(auth::SessionActivity::Ended) => {
            let _ = send_error(sender, "Chat session ended; sign in again").await;
            false
        }
        Err(error) => {
            warn!(%error, "failed to revalidate chat session");
            let _ = send_error(sender, "Could not validate chat session").await;
            false
        }
    }
}

async fn handle_text_message(
    state: &AppState,
    sender: &mut WsSender,
    user: &UserSummary,
    text: &str,
) {
    let event = match serde_json::from_str::<ClientEvent>(text) {
        Ok(event) => event,
        Err(error) => {
            warn!(%error, "failed to parse client chat event");
            let _ = send_error(sender, "Invalid chat event").await;
            return;
        }
    };

    match event {
        ClientEvent::GlobalMessageSend { body } => {
            if let Err(error) = state.rate_limits.check_chat_message(user.id).await {
                let _ = send_error(sender, &error.message()).await;
                return;
            }

            match normalize_message_body(&body) {
                Ok(body) => {
                    let error_message = {
                        let _realtime_state = state.realtime_state_lock.lock().await;
                        match insert_global_message(state, user, &body).await {
                            Ok(message) => {
                                send_global_message_created_locked(state, message).await;
                                None
                            }
                            Err(error) => {
                                warn!(%error, "failed to persist global message");
                                Some("Could not save chat message")
                            }
                        }
                    };
                    if let Some(error_message) = error_message {
                        let _ = send_error(sender, error_message).await;
                    }
                }
                Err(message) => {
                    let _ = send_error(sender, message).await;
                }
            }
        }
        ClientEvent::Hello { .. } | ClientEvent::Heartbeat => {}
        ClientEvent::PresenceUpdate { state: presence } => {
            if let Err(error) = state.rate_limits.check_presence_update(user.id).await {
                let _ = send_error(sender, &error.message()).await;
                return;
            }
            broadcast_presence(state, user.id, presence).await;
        }
    }
}

async fn broadcast_presence(state: &AppState, user_id: Uuid, presence: PresenceState) {
    let counts = state.presence.update(user_id, presence).await;
    send_public_event(&state.chat_tx, ServerEvent::PresenceCounts(counts));
}

async fn recent_global_messages(
    state: &AppState,
    viewer_id: Uuid,
) -> Result<Vec<ChatMessage>, sqlx::Error> {
    let sql = format!(
        "select
            m.id,
            m.body,
            m.created_at::text as created_at,
            u.id as author_id,
            u.display_name as author_display_name,
            u.avatar_url as author_avatar_url
         from global_messages m
         join users u on u.id = m.user_id
         where m.deleted_at is null
           and {}
         order by m.created_at desc
         limit $1",
        visibility::not_blocked_between_sql("$2", "m.user_id")
    );
    let rows = sqlx::query(&sql)
        .bind(RECENT_MESSAGE_LIMIT)
        .bind(viewer_id)
        .fetch_all(&state.db)
        .await?;

    let mut messages = rows
        .iter()
        .map(row_to_chat_message)
        .collect::<Result<Vec<_>, _>>()?;
    messages.reverse();
    Ok(messages)
}

async fn subscribe_with_global_snapshot(
    state: &AppState,
    session: &auth::AuthenticatedSession,
) -> Result<broadcast::Receiver<RealtimeEvent>, sqlx::Error> {
    let _realtime_state = state.realtime_state_lock.lock().await;
    let messages = recent_global_messages(state, session.user.id).await?;
    let receiver = state.chat_tx.subscribe();
    let _ = state.chat_tx.send(RealtimeEvent {
        audience: RealtimeAudience::Session(session.session_id),
        payload: RealtimePayload::Event(ServerEvent::GlobalMessageSnapshot { messages }),
    });
    Ok(receiver)
}

async fn hidden_viewer_ids_for_author(
    db: &sqlx::PgPool,
    author_id: Uuid,
) -> Result<HashSet<Uuid>, sqlx::Error> {
    let viewer_ids = sqlx::query_scalar::<_, Uuid>(
        "select distinct case
             when blocker_id = $1 then blocked_id
             else blocker_id
         end
         from blocks
         where blocker_id = $1 or blocked_id = $1
         limit $2",
    )
    .bind(author_id)
    .bind((MAX_CACHED_HIDDEN_VIEWERS + 1) as i64)
    .fetch_all(db)
    .await?;
    Ok(viewer_ids.into_iter().collect())
}

async fn event_delivery(
    state: &AppState,
    session: &auth::AuthenticatedSession,
    event: &RealtimeEvent,
) -> Delivery {
    match &event.audience {
        RealtimeAudience::Public => {
            public_event_delivery(state, &session.user, &event.payload).await
        }
        RealtimeAudience::User(user_id) if *user_id == session.user.id => {
            payload_delivery(&event.payload)
        }
        RealtimeAudience::User(_) => Delivery::Skip,
        RealtimeAudience::Session(session_id) if *session_id == session.session_id => {
            payload_delivery(&event.payload)
        }
        RealtimeAudience::Session(_) => Delivery::Skip,
    }
}

async fn public_event_delivery(
    state: &AppState,
    viewer: &UserSummary,
    payload: &RealtimePayload,
) -> Delivery {
    match payload {
        RealtimePayload::GlobalMessageCreated {
            message,
            hidden_viewer_ids,
            visibility_epoch,
        } => {
            if *visibility_epoch == state.visibility_epoch.load(AtomicOrdering::Acquire) {
                if let Some(hidden_viewer_ids) = hidden_viewer_ids {
                    return if hidden_viewer_ids.contains(&viewer.id) {
                        Delivery::Skip
                    } else {
                        Delivery::Send(ServerEvent::GlobalMessageCreated(message.clone()))
                    };
                }
            }
            match visibility::can_deliver_between(&state.db, viewer.id, message.author.id).await {
                Ok(true) => Delivery::Send(ServerEvent::GlobalMessageCreated(message.clone())),
                Ok(false) => Delivery::Skip,
                Err(error) => {
                    warn!(%error, "failed to revalidate chat block relationship");
                    Delivery::Skip
                }
            }
        }
        RealtimePayload::Event(
            ServerEvent::PresenceCounts(_) | ServerEvent::GlobalMessageDeleted { .. },
        ) => payload_delivery(payload),
        RealtimePayload::Event(
            ServerEvent::HelloOk { .. }
            | ServerEvent::MinimumVersionRequired { .. }
            | ServerEvent::ProtocolIncompatible { .. }
            | ServerEvent::AccessTokenExpired
            | ServerEvent::GlobalMessageSnapshot { .. }
            | ServerEvent::GlobalMessageCreated(_)
            | ServerEvent::UserMessagesHidden { .. }
            | ServerEvent::RelationshipStateChanged
            | ServerEvent::Error { .. },
        )
        | RealtimePayload::Disconnect { .. } => Delivery::Skip,
    }
}

fn payload_delivery(payload: &RealtimePayload) -> Delivery {
    match payload {
        RealtimePayload::Event(event) => Delivery::Send(event.clone()),
        RealtimePayload::GlobalMessageCreated { message, .. } => {
            Delivery::Send(ServerEvent::GlobalMessageCreated(message.clone()))
        }
        RealtimePayload::Disconnect { message } => Delivery::Disconnect(message.clone()),
    }
}

async fn insert_global_message(
    state: &AppState,
    user: &UserSummary,
    body: &str,
) -> Result<ChatMessage, sqlx::Error> {
    let Some(row) = sqlx::query(
        "insert into global_messages (user_id, body)
         select id, $2
         from users
         where id = $1
           and banned_at is null
           and (suspended_until is null or suspended_until <= now())
         returning id, body, created_at::text as created_at",
    )
    .bind(user.id)
    .bind(body)
    .fetch_optional(&state.db)
    .await?
    else {
        return Err(sqlx::Error::RowNotFound);
    };

    let created_at = row.try_get::<String, _>("created_at")?;
    Ok(ChatMessage {
        id: row.try_get("id")?,
        author: user.clone(),
        body: row.try_get("body")?,
        created_at: display_chat_timestamp(&created_at),
    })
}

fn row_to_chat_message(row: &PgRow) -> Result<ChatMessage, sqlx::Error> {
    let created_at = row.try_get::<String, _>("created_at")?;
    Ok(ChatMessage {
        id: row.try_get("id")?,
        author: UserSummary {
            id: row.try_get("author_id")?,
            display_name: row.try_get("author_display_name")?,
            avatar_url: row.try_get("author_avatar_url")?,
        },
        body: row.try_get("body")?,
        created_at: display_chat_timestamp(&created_at),
    })
}

fn display_chat_timestamp(value: &str) -> String {
    let value = value.trim();
    if has_timestamp_prefix(value) {
        value[..19].replace('T', " ")
    } else {
        value.to_string()
    }
}

fn has_timestamp_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 19
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .into_iter()
            .all(|index| bytes[index].is_ascii_digit())
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && matches!(bytes[10], b' ' | b'T')
        && bytes[13] == b':'
        && bytes[16] == b':'
}

fn normalize_message_body(body: &str) -> Result<String, &'static str> {
    let body = body.trim();
    if body.is_empty() {
        return Err("Message cannot be empty");
    }
    if body.chars().count() > MAX_MESSAGE_LEN {
        return Err("Message is too long");
    }
    Ok(body.to_string())
}

fn parse_client_hello(text: &str) -> Result<ClientHelloPayload, &'static str> {
    let envelope = serde_json::from_str::<ClientHelloEnvelope>(text).map_err(|_| "invalid json")?;
    if envelope.event_type != "hello" {
        return Err("not hello");
    }
    envelope.payload.ok_or("missing payload")
}

fn client_version_meets_minimum(client_version: &str, minimum_client_version: &str) -> bool {
    compare_client_versions(client_version, minimum_client_version)
        .is_some_and(|ordering| ordering != Ordering::Less)
}

fn compare_client_versions(client_version: &str, minimum_client_version: &str) -> Option<Ordering> {
    let client = parse_client_version(client_version)?;
    let minimum = parse_client_version(minimum_client_version)?;
    Some(client.cmp(&minimum))
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ParsedClientVersion {
    major: u64,
    minor: u64,
    patch: u64,
    release_rank: u8,
}

fn parse_client_version(value: &str) -> Option<ParsedClientVersion> {
    let value = value.trim();
    let without_build = value
        .split_once('+')
        .map(|(version, _)| version)
        .unwrap_or(value);
    let (core, release_rank) = match without_build.split_once('-') {
        Some((core, prerelease)) if !prerelease.is_empty() => (core, 0),
        Some(_) => return None,
        None => (without_build, 1),
    };
    let mut segments = core.split('.');
    let major = parse_version_segment(segments.next()?)?;
    let minor = parse_version_segment(segments.next()?)?;
    let patch = parse_version_segment(segments.next()?)?;
    if segments.next().is_some() {
        return None;
    }

    Some(ParsedClientVersion {
        major,
        minor,
        patch,
        release_rank,
    })
}

fn parse_version_segment(segment: &str) -> Option<u64> {
    if segment.is_empty() || !segment.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    segment.parse().ok()
}

async fn send_error(sender: &mut WsSender, message: &str) -> Result<(), String> {
    send_event(
        sender,
        &ServerEvent::Error {
            message: message.to_string(),
        },
    )
    .await
}

async fn send_event(sender: &mut WsSender, event: &ServerEvent) -> Result<(), String> {
    let text = serde_json::to_string(event).map_err(|error| error.to_string())?;
    sender
        .send(Message::Text(text.into()))
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_message_body() {
        assert_eq!(normalize_message_body("  "), Err("Message cannot be empty"));
    }

    #[test]
    fn trims_message_body() {
        assert_eq!(normalize_message_body(" hello ").unwrap(), "hello");
    }

    #[test]
    fn rejects_too_long_message_body() {
        let body = "x".repeat(MAX_MESSAGE_LEN + 1);

        assert_eq!(normalize_message_body(&body), Err("Message is too long"));
    }

    #[test]
    fn compares_client_versions_against_minimum() {
        assert!(client_version_meets_minimum("0.1.0", "0.1.0"));
        assert!(client_version_meets_minimum("0.1.1", "0.1.0"));
        assert!(client_version_meets_minimum("1.0.0", "0.9.9"));
        assert!(!client_version_meets_minimum("0.0.9", "0.1.0"));
        assert!(!client_version_meets_minimum("not-a-version", "0.1.0"));
    }

    #[test]
    fn treats_prerelease_client_versions_as_older_than_release() {
        assert!(!client_version_meets_minimum("0.1.0-alpha", "0.1.0"));
        assert!(client_version_meets_minimum("0.1.0+build.1", "0.1.0"));
    }

    #[test]
    fn parses_protocol_version_from_client_hello() {
        let hello = parse_client_hello(
            r#"{"type":"hello","payload":{"client_version":"0.4.0","protocol_version":4}}"#,
        )
        .unwrap();

        assert_eq!(hello.client_version.as_deref(), Some("0.4.0"));
        assert_eq!(hello.protocol_version, Some(PROTOCOL_VERSION));
    }

    #[test]
    fn accepts_legacy_hello_for_terminal_version_response() {
        let hello =
            parse_client_hello(r#"{"type":"hello","payload":{"client_version":"0.1.0"}}"#).unwrap();

        assert_eq!(hello.client_version.as_deref(), Some("0.1.0"));
        assert_eq!(hello.protocol_version, None);
    }

    #[test]
    fn formats_display_chat_timestamps_without_fraction_or_timezone() {
        assert_eq!(
            display_chat_timestamp("2026-09-08 18:15:09.505579+00"),
            "2026-09-08 18:15:09"
        );
        assert_eq!(
            display_chat_timestamp("2026-09-08T18:15:09.505579Z"),
            "2026-09-08 18:15:09"
        );
    }

    #[test]
    fn leaves_unexpected_display_chat_timestamps_unchanged() {
        assert_eq!(display_chat_timestamp("not a timestamp"), "not a timestamp");
    }

    #[test]
    fn parses_websocket_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer token".parse().unwrap(),
        );

        assert_eq!(auth::bearer_token(&headers), Some("token"));
    }
}

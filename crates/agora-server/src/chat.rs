use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        Arc, Mutex as StdMutex, Weak,
    },
    time::{Duration, Instant},
};

use agora_common::{
    ApiError, ChatMessage, ClientEvent, DmMessage, DmRealtimeEvent, PresenceState, ServerEvent,
    UserSummary, MAX_MESSAGE_LEN, PROTOCOL_VERSION,
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
use tokio::sync::{
    broadcast::{self, error::RecvError},
    watch, Mutex, OwnedMutexGuard,
};
use tracing::warn;
use uuid::Uuid;

use crate::{auth, visibility, AppState};

const RECENT_MESSAGE_LIMIT: i64 = 50;
const HELLO_TIMEOUT_SECONDS: u64 = 10;
const SESSION_REVALIDATE_SECONDS: u64 = 30;
const MAX_CACHED_HIDDEN_VIEWERS: usize = 1_024;
const MAX_WEBSOCKET_FRAME_BYTES: usize = 16 * 1024;
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 16 * 1024;
const WEBSOCKET_WRITE_BUFFER_BYTES: usize = 8 * 1024;
const MAX_WEBSOCKET_WRITE_BUFFER_BYTES: usize = 128 * 1024;
const WEBSOCKET_HEARTBEAT_SECONDS: u64 = 15;
const WEBSOCKET_READ_IDLE_SECONDS: u64 = 45;
const WEBSOCKET_WRITE_TIMEOUT_SECONDS: u64 = 5;
const PRESENCE_STALE_SECONDS: u64 = 60;
const PRESENCE_REAP_SECONDS: u64 = 15;
const REALTIME_ACCESS_CACHE_SECONDS: u64 = 20 * 60;
const MAX_WEBSOCKET_CONNECTIONS_PER_USER: usize = 4;
const MAX_DIRECT_MESSAGE_PAIR_LOCKS: usize = 1_024;

type WsSender = SplitSink<WebSocket, Message>;
type WsReceiver = SplitStream<WebSocket>;
type DirectMessagePair = (Uuid, Uuid);
type DirectMessagePairLock = Mutex<()>;
type DirectMessagePairLocksMap = HashMap<DirectMessagePair, Weak<DirectMessagePairLock>>;

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
    DirectMessage {
        thread_id: Uuid,
        recipient_ids: [Uuid; 2],
    },
}

#[derive(Debug, Clone)]
enum RealtimePayload {
    Event(ServerEvent),
    GlobalMessageCreated {
        message: ChatMessage,
        hidden_viewer_ids: Option<Arc<HashSet<Uuid>>>,
        visibility_epoch: u64,
    },
    DirectMessageThreadUpdated {
        thread_id: Uuid,
    },
    DirectMessageCreated {
        thread_id: Uuid,
        message: DmMessage,
    },
    Disconnect {
        message: String,
    },
}

enum Delivery {
    Send(WireEvent),
    Disconnect(String),
    Skip,
}

enum ConnectionAdmission {
    Allowed(RealtimeSessionAccess),
    Unauthorized,
    AtCapacity,
}

#[derive(Debug, Clone)]
enum WireEvent {
    Server(ServerEvent),
    DirectMessage(DmRealtimeEvent),
}

pub(crate) struct RealtimeAccess {
    state: StdMutex<RealtimeAccessState>,
}

// Pair-scoped locks retain block/DM publication ordering without serializing unrelated users.
pub(crate) struct DirectMessageDeliveryLocks {
    locks: StdMutex<DirectMessagePairLocksMap>,
}

impl DirectMessageDeliveryLocks {
    pub(crate) fn new() -> Self {
        Self {
            locks: StdMutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn lock(
        &self,
        first_user_id: Uuid,
        second_user_id: Uuid,
    ) -> OwnedMutexGuard<()> {
        let pair = if first_user_id <= second_user_id {
            (first_user_id, second_user_id)
        } else {
            (second_user_id, first_user_id)
        };
        let lock = {
            let mut locks = self
                .locks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if locks.len() >= MAX_DIRECT_MESSAGE_PAIR_LOCKS {
                locks.retain(|_, lock| lock.strong_count() > 0);
            }
            if let Some(lock) = locks.get(&pair).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(DirectMessagePairLock::new(()));
                locks.insert(pair, Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }
}

struct RealtimeAccessState {
    validation_generation: u64,
    active_sessions: HashMap<Uuid, ActiveRealtimeSession>,
    revoked_sessions: HashMap<Uuid, Instant>,
    restricted_users: HashMap<Uuid, UserRestriction>,
}

struct ActiveRealtimeSession {
    user_id: Uuid,
    connections: usize,
    access: RealtimeSessionAccess,
}

struct UserRestriction {
    generation: u64,
    expires_at: Instant,
}

#[derive(Clone)]
pub(crate) struct RealtimeSessionAccess {
    allowed: Arc<AtomicBool>,
}

impl RealtimeSessionAccess {
    fn new() -> Self {
        Self {
            allowed: Arc::new(AtomicBool::new(true)),
        }
    }

    fn can_receive(&self) -> bool {
        self.allowed.load(AtomicOrdering::Acquire)
    }

    fn revoke(&self) -> bool {
        self.allowed.swap(false, AtomicOrdering::AcqRel)
    }
}

impl RealtimeAccess {
    pub(crate) fn new() -> Self {
        Self {
            state: StdMutex::new(RealtimeAccessState {
                validation_generation: 0,
                active_sessions: HashMap::new(),
                revoked_sessions: HashMap::new(),
                restricted_users: HashMap::new(),
            }),
        }
    }

    pub(crate) fn validation_generation(&self) -> u64 {
        self.lock_state().validation_generation
    }

    fn begin_connection(
        &self,
        session_id: Uuid,
        user_id: Uuid,
        validated_at_generation: u64,
    ) -> ConnectionAdmission {
        let now = Instant::now();
        let mut state = self.lock_state();
        prune_realtime_access_cache(&mut state, now);
        if state.revoked_sessions.contains_key(&session_id) {
            return ConnectionAdmission::Unauthorized;
        }
        if let Some(restriction) = state.restricted_users.get(&user_id) {
            if restriction.generation > validated_at_generation {
                return ConnectionAdmission::Unauthorized;
            }
            // The database authentication check completed after this cached restriction.
            // It can only be active again after an administrative reversal or a new login.
            state.restricted_users.remove(&user_id);
        }

        if state
            .active_sessions
            .get(&session_id)
            .is_some_and(|session| session.user_id != user_id || !session.access.can_receive())
        {
            return ConnectionAdmission::Unauthorized;
        }
        let user_connections = state
            .active_sessions
            .values()
            .filter(|session| session.user_id == user_id)
            .map(|session| session.connections)
            .sum::<usize>();
        if user_connections >= MAX_WEBSOCKET_CONNECTIONS_PER_USER {
            return ConnectionAdmission::AtCapacity;
        }
        let session =
            state
                .active_sessions
                .entry(session_id)
                .or_insert_with(|| ActiveRealtimeSession {
                    user_id,
                    connections: 0,
                    access: RealtimeSessionAccess::new(),
                });
        session.connections += 1;
        ConnectionAdmission::Allowed(session.access.clone())
    }

    pub(crate) fn end_connection(&self, session_id: Uuid) {
        let mut state = self.lock_state();
        let remove_session = state
            .active_sessions
            .get_mut(&session_id)
            .is_some_and(|session| {
                session.connections = session.connections.saturating_sub(1);
                session.connections == 0
            });
        if remove_session {
            state.active_sessions.remove(&session_id);
        }
    }

    pub(crate) fn revoke_session(&self, session_id: Uuid) -> bool {
        let now = Instant::now();
        let mut state = self.lock_state();
        prune_realtime_access_cache(&mut state, now);
        let was_known = state.revoked_sessions.contains_key(&session_id)
            || state
                .active_sessions
                .get(&session_id)
                .is_some_and(|session| !session.access.can_receive());
        state
            .revoked_sessions
            .insert(session_id, realtime_access_cache_expiry(now));
        if let Some(session) = state.active_sessions.get(&session_id) {
            session.access.revoke();
        }
        !was_known
    }

    pub(crate) fn restrict_user(&self, user_id: Uuid) -> bool {
        let now = Instant::now();
        let mut state = self.lock_state();
        prune_realtime_access_cache(&mut state, now);
        state.validation_generation = state.validation_generation.wrapping_add(1);
        let generation = state.validation_generation;
        let newly_restricted = state
            .restricted_users
            .insert(
                user_id,
                UserRestriction {
                    generation,
                    expires_at: realtime_access_cache_expiry(now),
                },
            )
            .is_none();
        let session_ids = state
            .active_sessions
            .iter()
            .filter_map(|(session_id, session)| (session.user_id == user_id).then_some(*session_id))
            .collect::<Vec<_>>();
        let mut revoked_active_session = false;
        for session_id in session_ids {
            if let Some(session) = state.active_sessions.get(&session_id) {
                revoked_active_session |= session.access.revoke();
            }
            state
                .revoked_sessions
                .insert(session_id, realtime_access_cache_expiry(now));
        }
        newly_restricted || revoked_active_session
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, RealtimeAccessState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn realtime_access_cache_expiry(now: Instant) -> Instant {
    now + Duration::from_secs(REALTIME_ACCESS_CACHE_SECONDS)
}

fn prune_realtime_access_cache(state: &mut RealtimeAccessState, now: Instant) {
    state
        .revoked_sessions
        .retain(|_, expires_at| *expires_at > now);
    state
        .restricted_users
        .retain(|_, restriction| restriction.expires_at > now);
}

// A slow connection only needs the newest complete snapshot.
#[derive(Default)]
pub(crate) struct SnapshotDelivery {
    #[allow(clippy::type_complexity)]
    senders: Mutex<HashMap<Uuid, HashMap<Uuid, watch::Sender<Option<ServerEvent>>>>>,
}

impl SnapshotDelivery {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    async fn register(
        &self,
        user_id: Uuid,
        connection_id: Uuid,
    ) -> watch::Receiver<Option<ServerEvent>> {
        let (sender, receiver) = watch::channel(None);
        self.senders
            .lock()
            .await
            .entry(user_id)
            .or_default()
            .insert(connection_id, sender);
        receiver
    }

    async fn unregister(&self, user_id: Uuid, connection_id: Uuid) {
        let mut senders = self.senders.lock().await;
        let remove_user = senders.get_mut(&user_id).is_some_and(|connections| {
            connections.remove(&connection_id);
            connections.is_empty()
        });
        if remove_user {
            senders.remove(&user_id);
        }
    }

    async fn send_snapshot(&self, user_id: Uuid, messages: Vec<ChatMessage>) {
        let snapshot = ServerEvent::GlobalMessageSnapshot { messages };
        let mut senders = self.senders.lock().await;
        let remove_user = senders.get_mut(&user_id).is_some_and(|connections| {
            connections.retain(|_, sender| sender.send(Some(snapshot.clone())).is_ok());
            connections.is_empty()
        });
        if remove_user {
            senders.remove(&user_id);
        }
    }
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

pub(crate) fn spawn_presence_reaper(state: AppState) {
    tokio::spawn(async move {
        let mut shutdown = state.shutdown.clone();
        let mut reaper = tokio::time::interval(Duration::from_secs(PRESENCE_REAP_SECONDS));
        reaper.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        reaper.tick().await;

        loop {
            tokio::select! {
                _ = shutdown.changed() => return,
                _ = reaper.tick() => {
                    if let Some(counts) = state
                        .presence
                        .remove_stale(Duration::from_secs(PRESENCE_STALE_SECONDS))
                        .await
                    {
                        warn!("removed stale realtime presence connections");
                        send_public_event(&state.chat_tx, ServerEvent::PresenceCounts(counts));
                    }
                }
            }
        }
    });
}

pub(crate) fn send_session_revoked(state: &AppState, session_id: Uuid) {
    if !state.realtime_access.revoke_session(session_id) {
        return;
    }
    let _ = state.chat_tx.send(RealtimeEvent {
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

pub(crate) fn send_direct_message_thread_updated(
    tx: &broadcast::Sender<RealtimeEvent>,
    thread_id: Uuid,
    first_user_id: Uuid,
    second_user_id: Uuid,
) {
    let _ = tx.send(RealtimeEvent {
        audience: direct_message_audience(thread_id, first_user_id, second_user_id),
        payload: RealtimePayload::DirectMessageThreadUpdated { thread_id },
    });
}

pub(crate) fn send_direct_message_created(
    tx: &broadcast::Sender<RealtimeEvent>,
    message: DmMessage,
    first_user_id: Uuid,
    second_user_id: Uuid,
) {
    let _ = tx.send(RealtimeEvent {
        audience: direct_message_audience(message.thread_id, first_user_id, second_user_id),
        payload: RealtimePayload::DirectMessageCreated {
            thread_id: message.thread_id,
            message,
        },
    });
}

fn direct_message_audience(
    thread_id: Uuid,
    first_user_id: Uuid,
    second_user_id: Uuid,
) -> RealtimeAudience {
    debug_assert_ne!(first_user_id, second_user_id);
    RealtimeAudience::DirectMessage {
        thread_id,
        recipient_ids: [first_user_id, second_user_id],
    }
}

pub(crate) fn send_user_disconnect(state: &AppState, user_id: Uuid, message: impl Into<String>) {
    if !state.realtime_access.restrict_user(user_id) {
        return;
    }
    let _ = state.chat_tx.send(RealtimeEvent {
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
    state
        .snapshot_delivery
        .send_snapshot(user_id, messages)
        .await;
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
        .check_auth(peer_addr, &headers, &state.config.trusted_proxy_cidrs)
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

    let validation_generation = state.realtime_access.validation_generation();
    match auth::user_for_access_token(&state, access_token).await {
        Ok(Some(session)) => {
            let connection_permit = match state.websocket_connections.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(ApiError {
                            message: "Chat is at capacity; try again shortly".to_string(),
                        }),
                    )
                        .into_response();
                }
            };
            let connection_id = Uuid::new_v4();
            let session_access = match state.realtime_access.begin_connection(
                session.session_id,
                session.user.id,
                validation_generation,
            ) {
                ConnectionAdmission::Allowed(session_access) => session_access,
                ConnectionAdmission::Unauthorized => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(ApiError {
                            message: "chat session is invalid or expired".to_string(),
                        }),
                    )
                        .into_response();
                }
                ConnectionAdmission::AtCapacity => {
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(ApiError {
                            message: "Too many chat connections for this account".to_string(),
                        }),
                    )
                        .into_response();
                }
            };
            let failed_upgrade_access = state.realtime_access.clone();
            let session_id = session.session_id;
            ws.max_frame_size(MAX_WEBSOCKET_FRAME_BYTES)
                .max_message_size(MAX_WEBSOCKET_MESSAGE_BYTES)
                .write_buffer_size(WEBSOCKET_WRITE_BUFFER_BYTES)
                .max_write_buffer_size(MAX_WEBSOCKET_WRITE_BUFFER_BYTES)
                .on_failed_upgrade(move |error| {
                    failed_upgrade_access.end_connection(session_id);
                    warn!(%error, "WebSocket upgrade failed");
                })
                .on_upgrade(move |socket| {
                    websocket_session(
                        state,
                        socket,
                        session,
                        session_access,
                        connection_id,
                        connection_permit,
                    )
                })
                .into_response()
        }
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
    session_access: RealtimeSessionAccess,
    connection_id: Uuid,
    _connection_permit: tokio::sync::OwnedSemaphorePermit,
) {
    let user_id = session.user.id;
    let session_id = session.session_id;
    let joined_presence = websocket_session_inner(
        state.clone(),
        socket,
        session,
        session_access,
        connection_id,
    )
    .await;

    if joined_presence {
        state
            .snapshot_delivery
            .unregister(user_id, connection_id)
            .await;
        if let Some(presence) = state.presence.disconnect(user_id, connection_id).await {
            send_public_event(&state.chat_tx, ServerEvent::PresenceCounts(presence));
        }
    }
    state.realtime_access.end_connection(session_id);
}

async fn websocket_session_inner(
    state: AppState,
    socket: WebSocket,
    session: auth::AuthenticatedSession,
    session_access: RealtimeSessionAccess,
    connection_id: Uuid,
) -> bool {
    let (mut sender, mut receiver) = socket.split();
    let mut shutdown = state.shutdown.clone();

    if *shutdown.borrow() || !session_access.can_receive() {
        return false;
    }

    if !receive_hello(
        &state,
        &mut sender,
        &mut receiver,
        &session,
        &session_access,
        &mut shutdown,
    )
    .await
    {
        return false;
    }
    if !ensure_session_active(&state, &mut sender, &session, &session_access).await {
        return false;
    }

    if !session_access.can_receive() {
        return false;
    }
    let initial_presence = state.presence.connect(session.user.id, connection_id).await;
    send_public_event(
        &state.chat_tx,
        ServerEvent::PresenceCounts(initial_presence),
    );
    if !session_access.can_receive()
        || send_event(
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

    let mut snapshot_rx = state
        .snapshot_delivery
        .register(session.user.id, connection_id)
        .await;
    let (mut broadcast_rx, snapshot) = match subscribe_with_global_snapshot(&state, &session).await
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            warn!(%error, "failed to load recent global messages");
            let _ = send_error(&mut sender, "Could not load recent chat history").await;
            return true;
        }
    };
    if !session_access.can_receive()
        || send_event(
            &mut sender,
            &ServerEvent::GlobalMessageSnapshot { messages: snapshot },
        )
        .await
        .is_err()
    {
        return true;
    }

    let access_token_expires =
        tokio::time::sleep(Duration::from_secs(session.access_token_ttl_seconds));
    tokio::pin!(access_token_expires);
    let mut session_revalidate =
        tokio::time::interval(Duration::from_secs(SESSION_REVALIDATE_SECONDS));
    session_revalidate.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    session_revalidate.tick().await;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(WEBSOCKET_HEARTBEAT_SECONDS));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;
    let mut last_read = Instant::now();

    loop {
        tokio::select! {
            shutdown_result = shutdown.changed() => {
                if shutdown_result.is_ok() && *shutdown.borrow() {
                    let _ = send_error(&mut sender, "Server is shutting down; reconnect shortly").await;
                    let _ = close_websocket(&mut sender).await;
                }
                break;
            }
            _ = &mut access_token_expires => {
                if session_access.can_receive() {
                    let _ = send_event(&mut sender, &ServerEvent::AccessTokenExpired).await;
                }
                break;
            }
            _ = session_revalidate.tick() => {
                if !ensure_session_active(&state, &mut sender, &session, &session_access).await {
                    break;
                }
            }
            _ = heartbeat.tick() => {
                if !session_access.can_receive() {
                    break;
                }
                if last_read.elapsed() >= Duration::from_secs(WEBSOCKET_READ_IDLE_SECONDS) {
                    warn!(user_id = %session.user.id, "closing read-idle WebSocket connection");
                    let _ = close_websocket(&mut sender).await;
                    break;
                }
                if send_websocket_message(&mut sender, Message::Ping(Vec::new().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(message)) => {
                        if !session_access.can_receive() {
                            break;
                        }
                        if let Err(error) = state.rate_limits.check_chat_frame(session.user.id).await {
                            let _ = send_error(&mut sender, &error.message()).await;
                            break;
                        }
                        last_read = Instant::now();
                        if !state.presence.touch(session.user.id, connection_id).await {
                            break;
                        }

                        match message {
                            Message::Text(text) => {
                                if let Err(error) = state.rate_limits.check_chat_event(session.user.id).await {
                                    let _ = send_error(&mut sender, &error.message()).await;
                                    break;
                                }
                                handle_text_message(
                                    &state,
                                    &mut sender,
                                    &session.user,
                                    connection_id,
                                    text.as_str(),
                                )
                                .await;
                            }
                            Message::Ping(_) => {
                                if flush_websocket_sender(&mut sender).await.is_err() {
                                    break;
                                }
                            }
                            Message::Pong(_) => {}
                            Message::Binary(_) => {
                                let _ = send_error(&mut sender, "Binary WebSocket frames are not supported").await;
                                break;
                            }
                            Message::Close(_) => break,
                        }
                    }
                    None => break,
                    Some(Err(error)) => {
                        warn!(%error, "WebSocket receive failed");
                        break;
                    }
                }
            }
            snapshot_changed = snapshot_rx.changed() => {
                let Ok(()) = snapshot_changed else {
                    break;
                };
                let Some(snapshot) = snapshot_rx.borrow_and_update().clone() else {
                    continue;
                };
                if !session_access.can_receive() || send_event(&mut sender, &snapshot).await.is_err() {
                    break;
                }
            }
            broadcast = broadcast_rx.recv() => {
                match broadcast {
                    Ok(event) => {
                        if !deliver_realtime_event(
                            &state,
                            &mut sender,
                            &session,
                            &session_access,
                            &event,
                        )
                        .await
                        {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        warn!(skipped, "chat client lagged behind broadcast stream");
                        if !ensure_session_active(&state, &mut sender, &session, &session_access).await {
                            break;
                        }
                        let (receiver, snapshot) = match subscribe_with_global_snapshot(&state, &session).await {
                            Ok(snapshot) => snapshot,
                            Err(error) => {
                                warn!(%error, "failed to reload recent global messages after broadcast lag");
                                let _ = send_error(&mut sender, "Could not load recent chat history").await;
                                break;
                            }
                        };
                        broadcast_rx = receiver;
                        if !session_access.can_receive()
                            || send_event(
                            &mut sender,
                            &ServerEvent::GlobalMessageSnapshot { messages: snapshot },
                        )
                        .await
                        .is_err()
                            || send_event(&mut sender, &ServerEvent::RelationshipStateChanged).await.is_err()
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

async fn receive_hello(
    state: &AppState,
    sender: &mut WsSender,
    receiver: &mut WsReceiver,
    session: &auth::AuthenticatedSession,
    session_access: &RealtimeSessionAccess,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    let timeout = tokio::time::sleep(Duration::from_secs(HELLO_TIMEOUT_SECONDS));
    tokio::pin!(timeout);

    loop {
        if !session_access.can_receive() {
            return false;
        }
        tokio::select! {
            shutdown_result = shutdown.changed() => {
                if shutdown_result.is_ok() && *shutdown.borrow() {
                    let _ = close_websocket(sender).await;
                }
                return false;
            }
            _ = &mut timeout => {
                if session_access.can_receive() {
                    let _ = send_error(sender, "Chat hello timed out").await;
                }
                return false;
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(message)) => {
                        if !session_access.can_receive() {
                            return false;
                        }
                        if let Err(error) = state.rate_limits.check_chat_frame(session.user.id).await {
                            let _ = send_error(sender, &error.message()).await;
                            return false;
                        }

                        match message {
                            Message::Text(text) => {
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
                            Message::Ping(_) => {
                                if flush_websocket_sender(sender).await.is_err() {
                                    return false;
                                }
                            }
                            Message::Pong(_) => {}
                            Message::Binary(_) => {
                                let _ = send_error(sender, "Binary WebSocket frames are not supported").await;
                                return false;
                            }
                            Message::Close(_) => return false,
                        }
                    }
                    None => return false,
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
    session_access: &RealtimeSessionAccess,
) -> bool {
    if !session_access.can_receive() {
        return false;
    }
    match auth::session_activity(state, session.session_id).await {
        Ok(auth::SessionActivity::Active) => session_access.can_receive(),
        Ok(auth::SessionActivity::AccessTokenExpired) => {
            if session_access.can_receive() {
                let _ = send_event(sender, &ServerEvent::AccessTokenExpired).await;
            }
            false
        }
        Ok(auth::SessionActivity::Revoked) => {
            if session_access.can_receive() {
                let _ = send_error(sender, "Chat session ended; sign in again").await;
            }
            send_session_revoked(state, session.session_id);
            false
        }
        Ok(auth::SessionActivity::UserRestricted) => {
            if session_access.can_receive() {
                let _ = send_error(sender, "Account status changed; sign in again").await;
            }
            send_user_disconnect(
                state,
                session.user.id,
                "Account status changed; sign in again",
            );
            false
        }
        Ok(auth::SessionActivity::Ended) => {
            if session_access.can_receive() {
                let _ = send_error(sender, "Chat session ended; sign in again").await;
            }
            false
        }
        Err(error) => {
            warn!(%error, "failed to revalidate chat session");
            if session_access.can_receive() {
                let _ = send_error(sender, "Could not validate chat session").await;
            }
            false
        }
    }
}

async fn handle_text_message(
    state: &AppState,
    sender: &mut WsSender,
    user: &UserSummary,
    connection_id: Uuid,
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
            broadcast_presence(state, user.id, connection_id, presence).await;
        }
    }
}

async fn broadcast_presence(
    state: &AppState,
    user_id: Uuid,
    connection_id: Uuid,
    presence: PresenceState,
) {
    if let Some(counts) = state
        .presence
        .update(user_id, connection_id, presence)
        .await
    {
        send_public_event(&state.chat_tx, ServerEvent::PresenceCounts(counts));
    }
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
) -> Result<(broadcast::Receiver<RealtimeEvent>, Vec<ChatMessage>), sqlx::Error> {
    let _realtime_state = state.realtime_state_lock.lock().await;
    let messages = recent_global_messages(state, session.user.id).await?;
    let receiver = state.chat_tx.subscribe();
    Ok((receiver, messages))
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
        RealtimeAudience::DirectMessage {
            thread_id,
            recipient_ids,
        } if recipient_ids.contains(&session.user.id) => {
            direct_message_event_delivery(state, &session.user, *thread_id, &event.payload).await
        }
        RealtimeAudience::DirectMessage { .. } => Delivery::Skip,
    }
}

fn direct_message_targets_user(event: &RealtimeEvent, user_id: Uuid) -> bool {
    matches!(
        &event.audience,
        RealtimeAudience::DirectMessage { recipient_ids, .. } if recipient_ids.contains(&user_id)
    )
}

async fn deliver_realtime_event(
    state: &AppState,
    sender: &mut WsSender,
    session: &auth::AuthenticatedSession,
    session_access: &RealtimeSessionAccess,
    event: &RealtimeEvent,
) -> bool {
    let is_direct_message = matches!(&event.audience, RealtimeAudience::DirectMessage { .. });
    if is_direct_message && !direct_message_targets_user(event, session.user.id) {
        return true;
    }

    let is_disconnect = matches!(&event.payload, RealtimePayload::Disconnect { .. });
    if !is_disconnect && !session_access.can_receive() {
        return false;
    }

    if let RealtimeAudience::DirectMessage { recipient_ids, .. } = &event.audience {
        // Blocks take the same pair lock around their transaction. Keep revalidation and the
        // write ordered with that transaction, without stalling unrelated realtime work.
        let _direct_message_delivery = state
            .direct_message_delivery_locks
            .lock(recipient_ids[0], recipient_ids[1])
            .await;
        return apply_realtime_event(state, sender, session, session_access, event).await;
    }

    apply_realtime_event(state, sender, session, session_access, event).await
}

async fn apply_realtime_event(
    state: &AppState,
    sender: &mut WsSender,
    session: &auth::AuthenticatedSession,
    session_access: &RealtimeSessionAccess,
    event: &RealtimeEvent,
) -> bool {
    match event_delivery(state, session, event).await {
        Delivery::Send(event) => {
            session_access.can_receive() && send_wire_event(sender, &event).await.is_ok()
        }
        Delivery::Disconnect(message) => {
            let _ = send_error(sender, &message).await;
            let _ = close_websocket(sender).await;
            false
        }
        Delivery::Skip => true,
    }
}

async fn direct_message_event_delivery(
    state: &AppState,
    viewer: &UserSummary,
    thread_id: Uuid,
    payload: &RealtimePayload,
) -> Delivery {
    let summary = match crate::dm::direct_thread_summary_for_realtime(
        &state.db, viewer.id, thread_id,
    )
    .await
    {
        Ok(Some(summary)) => summary,
        Ok(None) => return Delivery::Skip,
        Err(error) => {
            warn!(%error, user_id = %viewer.id, %thread_id, "failed to revalidate direct message delivery");
            return Delivery::Skip;
        }
    };

    match payload {
        RealtimePayload::DirectMessageThreadUpdated {
            thread_id: payload_thread_id,
        } if *payload_thread_id == thread_id => Delivery::Send(WireEvent::DirectMessage(
            DmRealtimeEvent::DmThreadUpdated(summary),
        )),
        RealtimePayload::DirectMessageCreated {
            thread_id: payload_thread_id,
            message,
        } if *payload_thread_id == thread_id && message.thread_id == thread_id => Delivery::Send(
            WireEvent::DirectMessage(DmRealtimeEvent::DmMessageCreated(message.clone())),
        ),
        _ => Delivery::Skip,
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
                        Delivery::Send(WireEvent::Server(ServerEvent::GlobalMessageCreated(
                            message.clone(),
                        )))
                    };
                }
            }
            match visibility::can_deliver_between(&state.db, viewer.id, message.author.id).await {
                Ok(true) => Delivery::Send(WireEvent::Server(ServerEvent::GlobalMessageCreated(
                    message.clone(),
                ))),
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
        | RealtimePayload::DirectMessageThreadUpdated { .. }
        | RealtimePayload::DirectMessageCreated { .. }
        | RealtimePayload::Disconnect { .. } => Delivery::Skip,
    }
}

fn payload_delivery(payload: &RealtimePayload) -> Delivery {
    match payload {
        RealtimePayload::Event(event) => Delivery::Send(WireEvent::Server(event.clone())),
        RealtimePayload::GlobalMessageCreated { message, .. } => Delivery::Send(WireEvent::Server(
            ServerEvent::GlobalMessageCreated(message.clone()),
        )),
        RealtimePayload::DirectMessageThreadUpdated { .. }
        | RealtimePayload::DirectMessageCreated { .. } => Delivery::Skip,
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

pub(crate) fn is_valid_minimum_client_version(value: &str) -> bool {
    parse_client_version(value).is_some_and(|version| version.release_rank == 1)
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
    let (without_build, build) = match value.split_once('+') {
        Some((version, build)) => (version, Some(build)),
        None => (value, None),
    };
    if build.is_some_and(|build| !version_identifiers_are_valid(build, false)) {
        return None;
    }
    let (core, release_rank) = match without_build.split_once('-') {
        Some((core, prerelease)) if version_identifiers_are_valid(prerelease, true) => (core, 0),
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
    if segment.is_empty()
        || (segment.len() > 1 && segment.starts_with('0'))
        || !segment.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    segment.parse().ok()
}

fn version_identifiers_are_valid(value: &str, reject_numeric_leading_zero: bool) -> bool {
    value.split('.').all(|identifier| {
        let bytes = identifier.as_bytes();
        !bytes.is_empty()
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
            && (!reject_numeric_leading_zero
                || !(bytes.len() > 1
                    && bytes[0] == b'0'
                    && bytes.iter().all(|byte| byte.is_ascii_digit())))
    })
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
    send_websocket_message(sender, Message::Text(text.into())).await
}

async fn send_wire_event(sender: &mut WsSender, event: &WireEvent) -> Result<(), String> {
    let text = match event {
        WireEvent::Server(event) => serde_json::to_string(event),
        WireEvent::DirectMessage(event) => serde_json::to_string(event),
    }
    .map_err(|error| error.to_string())?;
    send_websocket_message(sender, Message::Text(text.into())).await
}

async fn send_websocket_message(sender: &mut WsSender, message: Message) -> Result<(), String> {
    match tokio::time::timeout(
        Duration::from_secs(WEBSOCKET_WRITE_TIMEOUT_SECONDS),
        sender.send(message),
    )
    .await
    {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => Err("WebSocket write timed out".to_string()),
    }
}

async fn flush_websocket_sender(sender: &mut WsSender) -> Result<(), String> {
    match tokio::time::timeout(
        Duration::from_secs(WEBSOCKET_WRITE_TIMEOUT_SECONDS),
        sender.flush(),
    )
    .await
    {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => Err("WebSocket write timed out".to_string()),
    }
}

async fn close_websocket(sender: &mut WsSender) -> Result<(), String> {
    send_websocket_message(sender, Message::Close(None)).await
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
    fn accepts_only_stable_semantic_minimum_versions() {
        assert!(is_valid_minimum_client_version("0.5.0"));
        assert!(is_valid_minimum_client_version("1.0.0+build.4"));
        assert!(!is_valid_minimum_client_version("0.5.0-beta"));
        assert!(!is_valid_minimum_client_version("0.05.0"));
        assert!(!is_valid_minimum_client_version("0.5.0+build+extra"));
    }

    #[tokio::test]
    async fn targets_and_coalesces_user_snapshots() {
        let delivery = SnapshotDelivery::new();
        let user = Uuid::new_v4();
        let other_user = Uuid::new_v4();
        let mut receiver = delivery.register(user, Uuid::new_v4()).await;
        let other_receiver = delivery.register(other_user, Uuid::new_v4()).await;

        delivery.send_snapshot(user, Vec::new()).await;
        delivery
            .send_snapshot(
                user,
                vec![ChatMessage {
                    id: Uuid::new_v4(),
                    author: UserSummary {
                        id: user,
                        display_name: "User".to_string(),
                        avatar_url: None,
                    },
                    body: "latest".to_string(),
                    created_at: "2026-09-19 12:00:00".to_string(),
                }],
            )
            .await;

        assert!(receiver.changed().await.is_ok());
        assert!(matches!(
            receiver.borrow_and_update().clone(),
            Some(ServerEvent::GlobalMessageSnapshot { messages }) if messages.len() == 1
        ));
        assert!(!other_receiver.has_changed().unwrap());
    }

    #[test]
    fn parses_protocol_version_from_client_hello() {
        let hello = parse_client_hello(
            r#"{"type":"hello","payload":{"client_version":"0.5.0","protocol_version":6}}"#,
        )
        .unwrap();

        assert_eq!(hello.client_version.as_deref(), Some("0.5.0"));
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

    #[test]
    fn direct_message_events_use_their_existing_top_level_wire_shape() {
        let event = DmRealtimeEvent::DmMessageCreated(DmMessage {
            id: Uuid::from_u128(3),
            thread_id: Uuid::from_u128(2),
            author: UserSummary {
                id: Uuid::from_u128(1),
                display_name: "Alice".to_string(),
                avatar_url: None,
            },
            body: "hello".to_string(),
            created_at: "2026-09-19 12:00:00+00".to_string(),
        });

        let json = serde_json::to_value(event).unwrap();

        assert_eq!(json["type"], "dm_message_created");
        assert_eq!(json["payload"]["thread_id"], Uuid::from_u128(2).to_string());
    }

    #[test]
    fn direct_message_payloads_cannot_use_the_generic_user_delivery_path() {
        let payload = RealtimePayload::DirectMessageThreadUpdated {
            thread_id: Uuid::new_v4(),
        };

        assert!(matches!(payload_delivery(&payload), Delivery::Skip));
    }

    #[test]
    fn direct_message_broadcasts_target_only_its_two_members() {
        let sender = broadcast_channel();
        let mut receiver = sender.subscribe();
        let thread_id = Uuid::new_v4();
        let first_user_id = Uuid::new_v4();
        let second_user_id = Uuid::new_v4();
        let outsider_id = Uuid::new_v4();

        send_direct_message_thread_updated(&sender, thread_id, first_user_id, second_user_id);

        let event = receiver.try_recv().unwrap();
        assert!(direct_message_targets_user(&event, first_user_id));
        assert!(direct_message_targets_user(&event, second_user_id));
        assert!(!direct_message_targets_user(&event, outsider_id));
        assert!(matches!(
            event,
            RealtimeEvent {
                audience: RealtimeAudience::DirectMessage {
                    thread_id: audience_thread_id,
                    recipient_ids,
                },
                payload: RealtimePayload::DirectMessageThreadUpdated { thread_id: payload_thread_id },
            } if audience_thread_id == thread_id
                && payload_thread_id == thread_id
                && recipient_ids == [first_user_id, second_user_id]
        ));
    }

    #[test]
    fn realtime_access_denies_events_after_a_session_revocation() {
        let access = RealtimeAccess::new();
        let session_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let session =
            match access.begin_connection(session_id, user_id, access.validation_generation()) {
                ConnectionAdmission::Allowed(session) => session,
                _ => panic!("expected realtime connection admission"),
            };

        assert!(session.can_receive());
        assert!(access.revoke_session(session_id));
        assert!(!session.can_receive());
        assert!(matches!(
            access.begin_connection(session_id, user_id, access.validation_generation()),
            ConnectionAdmission::Unauthorized
        ));
    }

    #[test]
    fn realtime_access_closes_connections_when_a_user_is_restricted() {
        let access = RealtimeAccess::new();
        let session_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let session =
            match access.begin_connection(session_id, user_id, access.validation_generation()) {
                ConnectionAdmission::Allowed(session) => session,
                _ => panic!("expected realtime connection admission"),
            };

        assert!(access.restrict_user(user_id));
        assert!(!session.can_receive());
    }

    #[test]
    fn cached_restrictions_after_a_database_auth_check_do_not_block_new_sessions() {
        let access = RealtimeAccess::new();
        let user_id = Uuid::new_v4();
        let before_restriction = access.validation_generation();
        assert!(access.restrict_user(user_id));

        assert!(matches!(
            access.begin_connection(Uuid::new_v4(), user_id, before_restriction),
            ConnectionAdmission::Unauthorized
        ));
        assert!(matches!(
            access.begin_connection(Uuid::new_v4(), user_id, access.validation_generation(),),
            ConnectionAdmission::Allowed(_)
        ));
    }

    #[test]
    fn realtime_access_limits_connections_per_user() {
        let access = RealtimeAccess::new();
        let user_id = Uuid::new_v4();

        for _ in 0..MAX_WEBSOCKET_CONNECTIONS_PER_USER {
            assert!(matches!(
                access.begin_connection(Uuid::new_v4(), user_id, access.validation_generation(),),
                ConnectionAdmission::Allowed(_)
            ));
        }
        assert!(matches!(
            access.begin_connection(Uuid::new_v4(), user_id, access.validation_generation()),
            ConnectionAdmission::AtCapacity
        ));
        assert!(matches!(
            access.begin_connection(
                Uuid::new_v4(),
                Uuid::new_v4(),
                access.validation_generation(),
            ),
            ConnectionAdmission::Allowed(_)
        ));
    }

    #[tokio::test]
    async fn direct_message_pair_locks_do_not_block_other_pairs() {
        let locks = DirectMessageDeliveryLocks::new();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let held = locks.lock(first, second).await;

        assert!(tokio::time::timeout(
            Duration::from_millis(50),
            locks.lock(Uuid::new_v4(), Uuid::new_v4()),
        )
        .await
        .is_ok());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), locks.lock(first, second))
                .await
                .is_err()
        );
        drop(held);
    }
}

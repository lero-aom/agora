use std::{net::SocketAddr, time::Duration};

use agora_common::{
    ApiError, ChatMessage, ClientEvent, PresenceCounts, PresenceState, ServerEvent, UserSummary,
    MAX_MESSAGE_LEN, PROTOCOL_VERSION,
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
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use sqlx::{postgres::PgRow, Row};
use tokio::sync::broadcast::{self, error::RecvError};
use tracing::warn;
use uuid::Uuid;

use crate::{auth, AppState};

const RECENT_MESSAGE_LIMIT: i64 = 50;

type WsSender = SplitSink<WebSocket, Message>;

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/ws", get(websocket))
}

pub(crate) fn broadcast_channel() -> broadcast::Sender<ServerEvent> {
    let (sender, _) = broadcast::channel(256);
    sender
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
    let user = session.user;
    let initial_presence = state.presence.connect(user.id).await;
    let _ = state
        .chat_tx
        .send(ServerEvent::PresenceCounts(initial_presence));

    websocket_session_inner(
        state.clone(),
        socket,
        user.clone(),
        initial_presence,
        session.access_token_ttl_seconds,
    )
    .await;

    let presence = state.presence.disconnect(user.id).await;
    let _ = state.chat_tx.send(ServerEvent::PresenceCounts(presence));
}

async fn websocket_session_inner(
    state: AppState,
    socket: WebSocket,
    user: UserSummary,
    initial_presence: PresenceCounts,
    access_token_ttl_seconds: u64,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut broadcast_rx = state.chat_tx.subscribe();

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
        return;
    }

    match recent_global_messages(&state, &user).await {
        Ok(messages) => {
            for message in messages {
                if send_event(&mut sender, &ServerEvent::GlobalMessageCreated(message))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
        Err(error) => {
            warn!(%error, "failed to load recent global messages");
            let _ = send_error(&mut sender, "Could not load recent chat history").await;
        }
    }

    let access_token_expires = tokio::time::sleep(Duration::from_secs(access_token_ttl_seconds));
    tokio::pin!(access_token_expires);

    loop {
        tokio::select! {
            _ = &mut access_token_expires => {
                let _ = send_error(&mut sender, "Chat session expired; sign in again").await;
                break;
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => handle_text_message(&state, &mut sender, &user, text.as_str()).await,
                    Some(Ok(Message::Ping(payload))) => {
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
                        if !should_deliver_event(&state, &user, &event).await {
                            continue;
                        }
                        if send_event(&mut sender, &event).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        warn!(skipped, "chat client lagged behind broadcast stream");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
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
                Ok(body) => match insert_global_message(state, user, &body).await {
                    Ok(message) => {
                        let _ = state
                            .chat_tx
                            .send(ServerEvent::GlobalMessageCreated(message));
                    }
                    Err(error) => {
                        warn!(%error, "failed to persist global message");
                        let _ = send_error(sender, "Could not save chat message").await;
                    }
                },
                Err(message) => {
                    let _ = send_error(sender, message).await;
                }
            }
        }
        ClientEvent::Hello { .. } | ClientEvent::Heartbeat => {}
        ClientEvent::PresenceUpdate { state: presence } => {
            broadcast_presence(state, user.id, presence).await;
        }
        ClientEvent::DmMessageSend { .. }
        | ClientEvent::ReportUser { .. }
        | ClientEvent::ReportMessage { .. } => {
            let _ = send_error(sender, "That chat action is not implemented yet").await;
        }
    }
}

async fn broadcast_presence(state: &AppState, user_id: Uuid, presence: PresenceState) {
    let counts = state.presence.update(user_id, presence).await;
    let _ = state.chat_tx.send(ServerEvent::PresenceCounts(counts));
}

async fn recent_global_messages(
    state: &AppState,
    viewer: &UserSummary,
) -> Result<Vec<ChatMessage>, sqlx::Error> {
    let rows = sqlx::query(
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
           and not exists (
                select 1
                from blocks b
                where (b.blocker_id = $2 and b.blocked_id = m.user_id)
                   or (b.blocker_id = m.user_id and b.blocked_id = $2)
           )
         order by m.created_at desc
         limit $1",
    )
    .bind(RECENT_MESSAGE_LIMIT)
    .bind(viewer.id)
    .fetch_all(&state.db)
    .await?;

    let mut messages = rows
        .iter()
        .map(row_to_chat_message)
        .collect::<Result<Vec<_>, _>>()?;
    messages.reverse();
    Ok(messages)
}

async fn should_deliver_event(state: &AppState, viewer: &UserSummary, event: &ServerEvent) -> bool {
    let ServerEvent::GlobalMessageCreated(message) = event else {
        return true;
    };
    match can_see_author(state, viewer.id, message.author.id).await {
        Ok(can_see) => can_see,
        Err(error) => {
            warn!(%error, "failed to check chat block relationship");
            false
        }
    }
}

async fn can_see_author(
    state: &AppState,
    viewer_id: Uuid,
    author_id: Uuid,
) -> Result<bool, sqlx::Error> {
    if viewer_id == author_id {
        return Ok(true);
    }

    let blocked = sqlx::query_scalar::<_, bool>(
        "select exists(
            select 1
            from blocks
            where (blocker_id = $1 and blocked_id = $2)
               or (blocker_id = $2 and blocked_id = $1)
        )",
    )
    .bind(viewer_id)
    .bind(author_id)
    .fetch_one(&state.db)
    .await?;

    Ok(!blocked)
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

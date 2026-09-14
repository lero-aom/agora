use std::time::Duration;

use sqlx::postgres::PgListener;
use tracing::warn;
use uuid::Uuid;

use crate::{chat, AppState};

const SESSION_REVOKED_CHANNEL: &str = "agora_session_revoked";
const USER_RESTRICTED_CHANNEL: &str = "agora_user_restricted";
const RECONNECT_DELAY_SECONDS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabaseNotification {
    SessionRevoked(Uuid),
    UserRestricted(Uuid),
}

pub(crate) fn spawn_database_event_listener(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = run_database_event_listener(&state).await {
                warn!(%error, "database event listener failed");
            }
            tokio::time::sleep(Duration::from_secs(RECONNECT_DELAY_SECONDS)).await;
        }
    });
}

async fn run_database_event_listener(state: &AppState) -> Result<(), sqlx::Error> {
    let mut listener = PgListener::connect(&state.config.database_url).await?;
    listener.listen(SESSION_REVOKED_CHANNEL).await?;
    listener.listen(USER_RESTRICTED_CHANNEL).await?;

    loop {
        let notification = listener.recv().await?;
        handle_database_notification(state, notification.channel(), notification.payload());
    }
}

fn handle_database_notification(state: &AppState, channel: &str, payload: &str) {
    match database_notification(channel, payload) {
        Some(DatabaseNotification::SessionRevoked(session_id)) => {
            chat::send_session_revoked(&state.chat_tx, session_id);
        }
        Some(DatabaseNotification::UserRestricted(user_id)) => {
            chat::send_user_disconnect(
                &state.chat_tx,
                user_id,
                "Account status changed; sign in again",
            );
        }
        None => warn!(channel, payload, "ignored database notification"),
    }
}

fn database_notification(channel: &str, payload: &str) -> Option<DatabaseNotification> {
    let id = Uuid::parse_str(payload).ok()?;
    match channel {
        SESSION_REVOKED_CHANNEL => Some(DatabaseNotification::SessionRevoked(id)),
        USER_RESTRICTED_CHANNEL => Some(DatabaseNotification::UserRestricted(id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_revocation_notifications() {
        let session_id = Uuid::new_v4();

        assert_eq!(
            database_notification(SESSION_REVOKED_CHANNEL, &session_id.to_string()),
            Some(DatabaseNotification::SessionRevoked(session_id))
        );
    }

    #[test]
    fn parses_user_restriction_notifications() {
        let user_id = Uuid::new_v4();

        assert_eq!(
            database_notification(USER_RESTRICTED_CHANNEL, &user_id.to_string()),
            Some(DatabaseNotification::UserRestricted(user_id))
        );
    }

    #[test]
    fn ignores_unknown_or_invalid_notifications() {
        assert_eq!(
            database_notification("other", &Uuid::new_v4().to_string()),
            None
        );
        assert_eq!(
            database_notification(SESSION_REVOKED_CHANNEL, "not-a-uuid"),
            None
        );
    }
}

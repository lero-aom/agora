use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use agora_common::{PresenceCounts, PresenceState};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct PresenceTracker {
    users: Mutex<HashMap<Uuid, HashMap<Uuid, ConnectionPresence>>>,
}

struct ConnectionPresence {
    state: PresenceState,
    last_seen: Instant,
}

impl PresenceTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn connect(&self, user_id: Uuid, connection_id: Uuid) -> PresenceCounts {
        let mut users = self.users.lock().await;
        users.entry(user_id).or_default().insert(
            connection_id,
            ConnectionPresence {
                state: PresenceState::Online,
                last_seen: Instant::now(),
            },
        );
        counts_from(&users)
    }

    pub(crate) async fn disconnect(
        &self,
        user_id: Uuid,
        connection_id: Uuid,
    ) -> Option<PresenceCounts> {
        let mut users = self.users.lock().await;
        let removed = users
            .get_mut(&user_id)
            .and_then(|connections| connections.remove(&connection_id));
        if users
            .get(&user_id)
            .is_some_and(|connections| connections.is_empty())
        {
            users.remove(&user_id);
        }
        removed.map(|_| counts_from(&users))
    }

    pub(crate) async fn update(
        &self,
        user_id: Uuid,
        connection_id: Uuid,
        state: PresenceState,
    ) -> Option<PresenceCounts> {
        let mut users = self.users.lock().await;
        let connection = users
            .get_mut(&user_id)
            .and_then(|connections| connections.get_mut(&connection_id))?;
        connection.state = match state {
            PresenceState::Offline => PresenceState::Online,
            state => state,
        };
        connection.last_seen = Instant::now();
        Some(counts_from(&users))
    }

    pub(crate) async fn touch(&self, user_id: Uuid, connection_id: Uuid) -> bool {
        let mut users = self.users.lock().await;
        let Some(connection) = users
            .get_mut(&user_id)
            .and_then(|connections| connections.get_mut(&connection_id))
        else {
            return false;
        };
        connection.last_seen = Instant::now();
        true
    }

    pub(crate) async fn remove_stale(&self, max_idle: Duration) -> Option<PresenceCounts> {
        let now = Instant::now();
        let mut users = self.users.lock().await;
        let mut removed_any = false;
        users.retain(|_, connections| {
            connections.retain(|_, connection| {
                let stale = now.duration_since(connection.last_seen) >= max_idle;
                removed_any |= stale;
                !stale
            });
            !connections.is_empty()
        });
        removed_any.then(|| counts_from(&users))
    }

    pub(crate) async fn counts(&self) -> PresenceCounts {
        let users = self.users.lock().await;
        counts_from(&users)
    }
}

fn counts_from(users: &HashMap<Uuid, HashMap<Uuid, ConnectionPresence>>) -> PresenceCounts {
    let mut counts = PresenceCounts {
        online: users.len() as u32,
        ..PresenceCounts::default()
    };
    for connections in users.values() {
        match effective_state(connections) {
            PresenceState::InGame => counts.in_game += 1,
            PresenceState::LookingForGame => counts.looking_for_game += 1,
            PresenceState::Offline | PresenceState::Online => {}
        }
    }
    counts
}

fn effective_state(connections: &HashMap<Uuid, ConnectionPresence>) -> PresenceState {
    // Multiple tabs collapse to one user-facing state without double-counting a user.
    if connections
        .values()
        .any(|presence| presence.state == PresenceState::InGame)
    {
        PresenceState::InGame
    } else if connections
        .values()
        .any(|presence| presence.state == PresenceState::LookingForGame)
    {
        PresenceState::LookingForGame
    } else {
        PresenceState::Online
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn counts_unique_online_users() {
        let tracker = PresenceTracker::new();
        let user = Uuid::new_v4();
        let first_connection = Uuid::new_v4();
        let second_connection = Uuid::new_v4();

        assert_eq!(tracker.connect(user, first_connection).await.online, 1);
        assert_eq!(tracker.connect(user, second_connection).await.online, 1);
        assert_eq!(
            tracker
                .disconnect(user, first_connection)
                .await
                .unwrap()
                .online,
            1
        );
        assert_eq!(
            tracker
                .disconnect(user, second_connection)
                .await
                .unwrap()
                .online,
            0
        );
    }

    #[tokio::test]
    async fn keeps_presence_state_per_connection() {
        let tracker = PresenceTracker::new();
        let user = Uuid::new_v4();
        let game_connection = Uuid::new_v4();
        let lfg_connection = Uuid::new_v4();

        tracker.connect(user, game_connection).await;
        tracker.connect(user, lfg_connection).await;
        let counts = tracker
            .update(user, game_connection, PresenceState::InGame)
            .await
            .unwrap();
        assert_eq!(counts.in_game, 1);
        assert_eq!(counts.looking_for_game, 0);

        let counts = tracker
            .update(user, lfg_connection, PresenceState::LookingForGame)
            .await
            .unwrap();
        assert_eq!(counts.in_game, 1);
        assert_eq!(counts.looking_for_game, 0);

        let counts = tracker
            .update(user, game_connection, PresenceState::Online)
            .await
            .unwrap();
        assert_eq!(counts.in_game, 0);
        assert_eq!(counts.looking_for_game, 1);
    }

    #[tokio::test]
    async fn removes_stale_connections() {
        let tracker = PresenceTracker::new();
        let user = Uuid::new_v4();
        tracker.connect(user, Uuid::new_v4()).await;

        assert_eq!(
            tracker
                .remove_stale(Duration::from_secs(0))
                .await
                .unwrap()
                .online,
            0
        );
        assert_eq!(tracker.counts().await.online, 0);
    }
}

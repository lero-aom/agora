use std::collections::HashMap;

use agora_common::{PresenceCounts, PresenceState};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct PresenceTracker {
    users: Mutex<HashMap<Uuid, UserPresence>>,
}

struct UserPresence {
    connections: usize,
    state: PresenceState,
}

impl PresenceTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn connect(&self, user_id: Uuid) -> PresenceCounts {
        let mut users = self.users.lock().await;
        let presence = users.entry(user_id).or_insert(UserPresence {
            connections: 0,
            state: PresenceState::Online,
        });
        presence.connections += 1;
        counts_from(&users)
    }

    pub(crate) async fn disconnect(&self, user_id: Uuid) -> PresenceCounts {
        let mut users = self.users.lock().await;
        if let Some(presence) = users.get_mut(&user_id) {
            presence.connections = presence.connections.saturating_sub(1);
            if presence.connections == 0 {
                users.remove(&user_id);
            }
        }
        counts_from(&users)
    }

    pub(crate) async fn update(&self, user_id: Uuid, state: PresenceState) -> PresenceCounts {
        let mut users = self.users.lock().await;
        if let Some(presence) = users.get_mut(&user_id) {
            presence.state = match state {
                PresenceState::Offline => PresenceState::Online,
                state => state,
            };
        }
        counts_from(&users)
    }
}

fn counts_from(users: &HashMap<Uuid, UserPresence>) -> PresenceCounts {
    PresenceCounts {
        online: users.len() as u32,
        in_game: users
            .values()
            .filter(|presence| presence.state == PresenceState::InGame)
            .count() as u32,
        looking_for_game: users
            .values()
            .filter(|presence| presence.state == PresenceState::LookingForGame)
            .count() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn counts_unique_online_users() {
        let tracker = PresenceTracker::new();
        let user = Uuid::new_v4();

        assert_eq!(tracker.connect(user).await.online, 1);
        assert_eq!(tracker.connect(user).await.online, 1);
        assert_eq!(tracker.disconnect(user).await.online, 1);
        assert_eq!(tracker.disconnect(user).await.online, 0);
    }

    #[tokio::test]
    async fn counts_presence_states() {
        let tracker = PresenceTracker::new();
        let lfg = Uuid::new_v4();
        let in_game = Uuid::new_v4();

        tracker.connect(lfg).await;
        tracker.connect(in_game).await;
        tracker.update(lfg, PresenceState::LookingForGame).await;
        let counts = tracker.update(in_game, PresenceState::InGame).await;

        assert_eq!(counts.online, 2);
        assert_eq!(counts.looking_for_game, 1);
        assert_eq!(counts.in_game, 1);
    }
}

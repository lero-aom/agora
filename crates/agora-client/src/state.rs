use super::update;
use agora_common::{
    AuthSession, ChatMessage, ClientEvent, DmMessage, DmThreadSummary, FriendshipSummary,
    MessageKind, PresenceCounts, PresenceState, UserSummary,
};
use dioxus::prelude::{Readable, Signal, Writable};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AppTab {
    Global,
    DirectMessages,
    Friends,
    BlockReport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReportDraft {
    pub(super) target: UserSummary,
    pub(super) message_id: Option<uuid::Uuid>,
    pub(super) message_kind: Option<MessageKind>,
    pub(super) message_preview: Option<String>,
}

#[derive(Default)]
pub(super) struct FriendSections {
    pub(super) accepted: Vec<FriendshipSummary>,
    pub(super) incoming: Vec<FriendshipSummary>,
    pub(super) outgoing: Vec<FriendshipSummary>,
    pub(super) inactive: Vec<FriendshipSummary>,
}

#[derive(Clone, Copy)]
pub(super) struct RelationshipSignals {
    pub(super) friendships: Signal<Vec<FriendshipSummary>>,
    pub(super) friendships_load_generation: Signal<u64>,
    pub(super) friends_status: Signal<String>,
    pub(super) blocked_users: Signal<Vec<UserSummary>>,
    pub(super) blocks_load_generation: Signal<u64>,
    pub(super) block_status: Signal<String>,
}

#[derive(Clone, Copy)]
pub(super) struct UserAutocompleteSignals {
    pub(super) query: Signal<String>,
    pub(super) results: Signal<Vec<UserSummary>>,
    pub(super) selected: Signal<Option<UserSummary>>,
    pub(super) request_generation: Signal<u64>,
    pub(super) pending: Signal<bool>,
    pub(super) expanded: Signal<bool>,
    pub(super) active_index: Signal<Option<usize>>,
    pub(super) status: Signal<String>,
}

#[derive(Clone, Copy)]
pub(super) struct DirectMessageSignals {
    pub(super) threads: Signal<Vec<DmThreadSummary>>,
    pub(super) threads_load_generation: Signal<u64>,
    pub(super) selected_thread_id: Signal<Option<uuid::Uuid>>,
    pub(super) messages: Signal<Vec<DmMessage>>,
    pub(super) next_before_message_id: Signal<Option<uuid::Uuid>>,
    pub(super) history_load_generation: Signal<u64>,
    pub(super) status: Signal<String>,
    pub(super) composer_body: Signal<String>,
    pub(super) search: UserAutocompleteSignals,
    pub(super) threads_pending: Signal<bool>,
    pub(super) history_pending: Signal<bool>,
    pub(super) create_pending: Signal<bool>,
    pub(super) send_pending: Signal<bool>,
}

#[derive(Clone, Copy)]
pub(super) struct PresenceSelectionSignals {
    pub(super) selected: Signal<PresenceState>,
    pub(super) confirmed: Signal<PresenceState>,
    pub(super) unconfirmed: Signal<Option<PresenceState>>,
    pub(super) status: Signal<String>,
    pub(super) pending: Signal<bool>,
    pub(super) connected: Signal<bool>,
    pub(super) request_generation: Signal<u64>,
}

#[derive(Clone, Copy)]
pub(super) struct ChatSessionSignals {
    pub(super) auth_session: Signal<Option<AuthSession>>,
    pub(super) reauth_required: Signal<Option<String>>,
    pub(super) login_status: Signal<String>,
    pub(super) session_generation: Signal<u64>,
    pub(super) chat_messages: Signal<Vec<ChatMessage>>,
    pub(super) deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    pub(super) chat_status: Signal<String>,
    pub(super) presence_counts: Signal<PresenceCounts>,
    pub(super) presence_selection: PresenceSelectionSignals,
    pub(super) chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    pub(super) chat_send_pending: Signal<bool>,
    pub(super) updates: UpdateSignals,
    pub(super) relationships: RelationshipSignals,
    pub(super) direct_messages: DirectMessageSignals,
}

#[derive(Clone, Debug)]
pub(super) enum UpdateUiStatus {
    Bootstrap,
    Checking {
        required: Option<String>,
    },
    UpToDate,
    Available {
        version: String,
        required: Option<String>,
    },
    Downloading {
        version: String,
        required: Option<String>,
    },
    Installing {
        version: String,
        required: Option<String>,
    },
    Failed {
        message: String,
        required: Option<String>,
    },
    SkippedLoopback,
    Recovery(String),
}

impl UpdateUiStatus {
    pub(super) fn message(&self) -> String {
        match self {
            Self::Bootstrap => {
                "This bootstrap build cannot check for automatic updates. Install a current release manually."
                    .to_string()
            }
            Self::Checking {
                required: Some(required),
            } => {
                format!("{required} Checking the signed release...")
            }
            Self::Checking { required: None } => "Checking the signed release...".to_string(),
            Self::UpToDate => "Agora is up to date.".to_string(),
            Self::Available {
                version,
                required: Some(required),
            } => {
                format!("{required} Agora {version} is ready to install.")
            }
            Self::Available {
                version,
                required: None,
            } => {
                format!("Agora {version} is ready to install.")
            }
            Self::Downloading {
                version,
                required: Some(required),
            } => {
                format!("{required} Downloading and verifying Agora {version}...")
            }
            Self::Downloading {
                version,
                required: None,
            } => {
                format!("Downloading and verifying Agora {version}...")
            }
            Self::Installing {
                version,
                required: Some(required),
            } => {
                format!("{required} Restarting to install Agora {version}...")
            }
            Self::Installing {
                version,
                required: None,
            } => {
                format!("Restarting to install Agora {version}...")
            }
            Self::Failed {
                message,
                required: Some(required),
            } => {
                format!("{required} {message}")
            }
            Self::Failed {
                message,
                required: None,
            } => message.clone(),
            Self::SkippedLoopback => {
                "Automatic updates are disabled for a loopback server.".to_string()
            }
            Self::Recovery(message) => message.clone(),
        }
    }

    pub(super) fn required_reason(&self) -> Option<String> {
        match self {
            Self::Checking { required }
            | Self::Available { required, .. }
            | Self::Downloading { required, .. }
            | Self::Installing { required, .. }
            | Self::Failed { required, .. } => required.clone(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct UpdateSignals {
    pub(super) status: Signal<UpdateUiStatus>,
    pub(super) available: Signal<Option<update::AvailableUpdate>>,
    pub(super) pending: Signal<bool>,
    pub(super) generation: Signal<u64>,
}

pub(super) struct OutgoingChatEvent {
    pub(super) event: ClientEvent,
    pub(super) transport_result: oneshot::Sender<Result<(), String>>,
}

pub(super) fn next_session_generation(mut session_generation: Signal<u64>) -> u64 {
    let current = *session_generation.read();
    let next = current.wrapping_add(1);
    session_generation.set(next);
    next
}

pub(super) fn next_relationship_load_generation(mut load_generation: Signal<u64>) -> u64 {
    let current = *load_generation.read();
    let next = current.wrapping_add(1);
    load_generation.set(next);
    next
}

pub(super) fn session_generation_current(session_generation: Signal<u64>, generation: u64) -> bool {
    *session_generation.read() == generation
}

pub(super) fn relationship_load_generation_current(
    load_generation: Signal<u64>,
    generation: u64,
) -> bool {
    *load_generation.read() == generation
}

pub(super) fn next_direct_message_load_generation(mut load_generation: Signal<u64>) -> u64 {
    let current = *load_generation.read();
    let next = current.wrapping_add(1);
    load_generation.set(next);
    next
}

pub(super) fn direct_message_load_generation_current(
    load_generation: Signal<u64>,
    generation: u64,
) -> bool {
    *load_generation.read() == generation
}

pub(super) fn current_session_matches(
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

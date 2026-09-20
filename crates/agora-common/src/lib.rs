use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 6;
pub const MAX_MESSAGE_LEN: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Offline,
    Online,
    LookingForGame,
    InGame,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceCounts {
    pub online: u32,
    pub in_game: u32,
    pub looking_for_game: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub database: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionResponse {
    pub server_version: String,
    pub protocol_version: u16,
    pub minimum_client_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    pub id: Uuid,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamLoginStartResponse {
    pub browser_url: String,
    pub poll_token: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamLoginPollRequest {
    pub poll_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamLoginPollResponse {
    pub status: SteamLoginStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SteamLoginStatus {
    Pending,
    Complete { session: AuthSession },
    Expired,
    Denied { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in_seconds: u64,
    pub user: UserSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshResponse {
    pub session: AuthSession,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutResponse {
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevLoginRequest {
    pub account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevLoginResponse {
    pub session: AuthSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendshipStatus {
    Pending,
    Accepted,
    Declined,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendshipSummary {
    pub id: Uuid,
    pub requester: UserSummary,
    pub addressee: UserSummary,
    pub status: FriendshipStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendListResponse {
    pub friendships: Vec<FriendshipSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRequest {
    pub user_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendshipResponse {
    pub friendship: FriendshipSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveFriendResponse {
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockUserRequest {
    pub user_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockListResponse {
    pub blocked_users: Vec<UserSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockUserResponse {
    pub blocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnblockUserResponse {
    pub unblocked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Global,
    Dm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    User,
    Moderator,
    Admin,
    Owner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Open,
    Resolved,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateReportRequest {
    pub reported_user_id: Uuid,
    pub message_id: Option<Uuid>,
    pub message_kind: Option<MessageKind>,
    pub reason: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateReportResponse {
    pub id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportListResponse {
    pub reports: Vec<ReportSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportSummary {
    pub id: Uuid,
    pub reporter: UserSummary,
    pub reported_user: UserSummary,
    pub message_id: Option<Uuid>,
    pub message_kind: Option<MessageKind>,
    pub reason: String,
    pub status: ReportStatus,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub resolved_by: Option<UserSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportDetailResponse {
    pub report: ReportDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportDetail {
    pub summary: ReportSummary,
    pub details: Option<String>,
    pub message: Option<ReportedMessage>,
    pub actions: Vec<ModerationActionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedMessage {
    pub id: Uuid,
    pub kind: MessageKind,
    pub author: UserSummary,
    pub body: Option<String>,
    pub created_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationActionKind {
    DeleteGlobalMessage,
    Suspend,
    Ban,
    Unban,
    ResolveReport,
    DismissReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModerationActionSummary {
    pub id: Uuid,
    pub moderator: UserSummary,
    pub target_user: UserSummary,
    pub report_id: Option<Uuid>,
    pub message_id: Option<Uuid>,
    pub message_kind: Option<MessageKind>,
    pub action: ModerationActionKind,
    pub reason: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModerationReasonRequest {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteGlobalMessageRequest {
    pub reason: String,
    pub report_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuspendUserRequest {
    pub reason: String,
    pub duration_seconds: u64,
    pub report_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BanUserRequest {
    pub reason: String,
    pub report_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModerationActionResponse {
    pub action: ModerationActionSummary,
    pub revoked_sessions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSearchResponse {
    pub users: Vec<UserSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmThreadSummary {
    pub id: Uuid,
    pub other_user: UserSummary,
    pub created_at: String,
    pub last_message_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmThreadListResponse {
    pub threads: Vec<DmThreadSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateDmThreadRequest {
    pub recipient_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateDmThreadResponse {
    pub thread: DmThreadSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmMessage {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub author: UserSummary,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendDmMessageRequest {
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendDmMessageResponse {
    pub message: DmMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmMessageHistoryResponse {
    pub messages: Vec<DmMessage>,
    pub next_before_message_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DmRealtimeEvent {
    DmThreadUpdated(DmThreadSummary),
    DmMessageCreated(DmMessage),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: Uuid,
    pub author: UserSummary,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ClientEvent {
    Hello {
        client_version: String,
        protocol_version: u16,
    },
    Heartbeat,
    PresenceUpdate {
        state: PresenceState,
    },
    GlobalMessageSend {
        body: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServerEvent {
    HelloOk {
        protocol_version: u16,
        presence: PresenceCounts,
    },
    MinimumVersionRequired {
        minimum_client_version: String,
    },
    ProtocolIncompatible {
        required_protocol_version: u16,
    },
    AccessTokenExpired,
    PresenceCounts(PresenceCounts),
    GlobalMessageSnapshot {
        messages: Vec<ChatMessage>,
    },
    GlobalMessageCreated(ChatMessage),
    GlobalMessageDeleted {
        message_id: Uuid,
    },
    UserMessagesHidden {
        user_id: Uuid,
    },
    RelationshipStateChanged,
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: u128, display_name: &str) -> UserSummary {
        UserSummary {
            id: Uuid::from_u128(id),
            display_name: display_name.to_string(),
            avatar_url: None,
        }
    }

    #[test]
    fn direct_message_realtime_events_preserve_thread_and_message_payloads() {
        let thread_id = Uuid::from_u128(10);
        let thread = DmThreadSummary {
            id: thread_id,
            other_user: user(2, "Bob"),
            created_at: "2026-09-19 12:00:00+00".to_string(),
            last_message_at: None,
        };
        let message = DmMessage {
            id: Uuid::from_u128(11),
            thread_id,
            author: user(1, "Alice"),
            body: "hello".to_string(),
            created_at: "2026-09-19 12:01:00+00".to_string(),
        };

        assert!(matches!(
            DmRealtimeEvent::DmThreadUpdated(thread.clone()),
            DmRealtimeEvent::DmThreadUpdated(payload) if payload == thread
        ));
        assert!(matches!(
            DmRealtimeEvent::DmMessageCreated(message.clone()),
            DmRealtimeEvent::DmMessageCreated(payload) if payload == message
        ));
    }
}

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_MESSAGE_LEN: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Offline,
    Online,
    LookingForGame,
    InGame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AomState {
    NotRunning,
    RunningUnknown,
    MainMenu,
    MultiplayerMenu,
    Lobby,
    InMatch,
    PostGame,
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
    pub display_name: Option<String>,
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
pub struct UserSearchResponse {
    pub users: Vec<UserSummary>,
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
    Hello { client_version: String },
    Heartbeat,
    PresenceUpdate { state: PresenceState },
    GlobalMessageSend { body: String },
    DmMessageSend { thread_id: Uuid, body: String },
    ReportUser { user_id: Uuid, reason: String },
    ReportMessage { message_id: Uuid, reason: String },
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
    PresenceCounts(PresenceCounts),
    GlobalMessageCreated(ChatMessage),
    DmMessageCreated(ChatMessage),
    Error {
        message: String,
    },
}

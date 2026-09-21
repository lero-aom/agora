use super::server::server_url;
use agora_common::{
    ApiError, AuthSession, BlockListResponse, BlockUserRequest, BlockUserResponse,
    CreateDmThreadRequest, CreateDmThreadResponse, CreateReportRequest, CreateReportResponse,
    DmMessage, DmMessageHistoryResponse, DmThreadListResponse, DmThreadSummary, FriendListResponse,
    FriendRequest, FriendshipResponse, FriendshipSummary, MessageKind, RemoveFriendResponse,
    SendDmMessageRequest, SendDmMessageResponse, UnblockUserResponse, UserSearchResponse,
    UserSummary,
};
use serde::{de::DeserializeOwned, Serialize};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum BearerAccess {
    Open,
    AwaitingCompatibility,
    Blocked(String),
}

static BEARER_ACCESS: OnceLock<Mutex<BearerAccess>> = OnceLock::new();

fn bearer_access() -> &'static Mutex<BearerAccess> {
    BEARER_ACCESS.get_or_init(|| Mutex::new(BearerAccess::Open))
}

pub(super) fn begin_bearer_compatibility_check() {
    if let Ok(mut access) = bearer_access().lock() {
        *access = BearerAccess::AwaitingCompatibility;
    }
}

pub(super) fn allow_bearer_access() {
    if let Ok(mut access) = bearer_access().lock() {
        *access = BearerAccess::Open;
    }
}

pub(super) fn block_bearer_access(reason: String) {
    if let Ok(mut access) = bearer_access().lock() {
        *access = BearerAccess::Blocked(reason);
    }
}

pub(super) fn bearer_access_error(access: &BearerAccess) -> Option<String> {
    match access {
        BearerAccess::Open => None,
        BearerAccess::AwaitingCompatibility => Some(
            "Waiting for server version compatibility confirmation before accessing Agora data"
                .to_string(),
        ),
        BearerAccess::Blocked(reason) => Some(format!(
            "{reason} Update Agora or sign in again before accessing Agora data"
        )),
    }
}

fn ensure_bearer_access() -> Result<(), String> {
    let access = bearer_access()
        .lock()
        .map_err(|_| "Agora version compatibility state is unavailable".to_string())?;
    bearer_access_error(&access).map_or(Ok(()), Err)
}

pub(super) async fn api_error(response: reqwest::Response, context: &str) -> String {
    let status = response.status();
    match response.json::<ApiError>().await {
        Ok(error) => format!("{context}: {}", error.message),
        Err(_) => format!("{context}: HTTP {status}"),
    }
}

pub(super) async fn search_users_api(
    session: &AuthSession,
    query: &str,
) -> Result<Vec<UserSummary>, String> {
    ensure_bearer_access()?;
    let server_url = server_url()?;
    let response = http_client()?
        .get(format!("{server_url}/users/search"))
        .bearer_auth(&session.access_token)
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|error| format!("Could not search users: {error}"))?;

    json_response::<UserSearchResponse>(response, "User search failed")
        .await
        .map(|response| response.users)
}

pub(super) async fn list_direct_message_threads_api(
    session: &AuthSession,
) -> Result<Vec<DmThreadSummary>, String> {
    api_get::<DmThreadListResponse>(session, "/dm/threads", "Direct message list failed")
        .await
        .map(|response| response.threads)
}

pub(super) async fn create_direct_message_thread_api(
    session: &AuthSession,
    recipient_id: uuid::Uuid,
) -> Result<DmThreadSummary, String> {
    api_post_json::<CreateDmThreadResponse, _>(
        session,
        "/dm/threads",
        &CreateDmThreadRequest { recipient_id },
        "Create direct message failed",
    )
    .await
    .map(|response| response.thread)
}

pub(super) async fn list_direct_message_history_api(
    session: &AuthSession,
    thread_id: uuid::Uuid,
    before: Option<uuid::Uuid>,
) -> Result<DmMessageHistoryResponse, String> {
    ensure_bearer_access()?;
    let server_url = server_url()?;
    let request = http_client()?
        .get(format!("{server_url}/dm/threads/{thread_id}/messages"))
        .bearer_auth(&session.access_token);
    let request = match before {
        Some(before) => {
            request.query(&[("before", before.to_string()), ("limit", "50".to_string())])
        }
        None => request.query(&[("limit", "50")]),
    };
    let response = request
        .send()
        .await
        .map_err(|error| format!("Could not load direct message history: {error}"))?;

    json_response(response, "Direct message history failed").await
}

pub(super) async fn send_direct_message_api(
    session: &AuthSession,
    thread_id: uuid::Uuid,
    body: String,
) -> Result<DmMessage, String> {
    api_post_json::<SendDmMessageResponse, _>(
        session,
        &format!("/dm/threads/{thread_id}/messages"),
        &SendDmMessageRequest { body },
        "Send direct message failed",
    )
    .await
    .map(|response| response.message)
}

pub(super) async fn list_friends_api(
    session: &AuthSession,
) -> Result<Vec<FriendshipSummary>, String> {
    api_get::<FriendListResponse>(session, "/friends", "Friend list failed")
        .await
        .map(|response| response.friendships)
}

pub(super) async fn send_friend_request_api(
    session: &AuthSession,
    user_id: &str,
) -> Result<FriendshipSummary, String> {
    api_post_json::<FriendshipResponse, _>(
        session,
        "/friends",
        &FriendRequest {
            user_id: user_id
                .parse()
                .map_err(|error| format!("Invalid user id: {error}"))?,
        },
        "Friend request failed",
    )
    .await
    .map(|response| response.friendship)
}

pub(super) async fn accept_friend_request_api(
    session: &AuthSession,
    friendship_id: &str,
) -> Result<FriendshipSummary, String> {
    api_post_json::<FriendshipResponse, _>(
        session,
        &format!("/friends/{friendship_id}/accept"),
        &(),
        "Accept friend request failed",
    )
    .await
    .map(|response| response.friendship)
}

pub(super) async fn decline_friend_request_api(
    session: &AuthSession,
    friendship_id: &str,
) -> Result<FriendshipSummary, String> {
    api_post_json::<FriendshipResponse, _>(
        session,
        &format!("/friends/{friendship_id}/decline"),
        &(),
        "Decline friend request failed",
    )
    .await
    .map(|response| response.friendship)
}

pub(super) async fn remove_friend_api(
    session: &AuthSession,
    user_id: &str,
) -> Result<bool, String> {
    api_delete::<RemoveFriendResponse>(
        session,
        &format!("/friends/{user_id}"),
        "Remove friend failed",
    )
    .await
    .map(|response| response.removed)
}

pub(super) async fn list_blocks_api(session: &AuthSession) -> Result<Vec<UserSummary>, String> {
    api_get::<BlockListResponse>(session, "/blocks", "Block list failed")
        .await
        .map(|response| response.blocked_users)
}

pub(super) async fn block_user_api(session: &AuthSession, user_id: &str) -> Result<bool, String> {
    api_post_json::<BlockUserResponse, _>(
        session,
        "/blocks",
        &BlockUserRequest {
            user_id: user_id
                .parse()
                .map_err(|error| format!("Invalid user id: {error}"))?,
        },
        "Block user failed",
    )
    .await
    .map(|response| response.blocked)
}

pub(super) async fn unblock_user_api(session: &AuthSession, user_id: &str) -> Result<bool, String> {
    api_delete::<UnblockUserResponse>(
        session,
        &format!("/blocks/{user_id}"),
        "Unblock user failed",
    )
    .await
    .map(|response| response.unblocked)
}

pub(super) async fn create_report_api(
    session: &AuthSession,
    reported_user_id: uuid::Uuid,
    message_id: Option<uuid::Uuid>,
    message_kind: Option<MessageKind>,
    reason: String,
    details: Option<String>,
) -> Result<String, String> {
    api_post_json::<CreateReportResponse, _>(
        session,
        "/reports",
        &CreateReportRequest {
            reported_user_id,
            message_id,
            message_kind,
            reason,
            details,
        },
        "Create report failed",
    )
    .await
    .map(|response| response.id.to_string())
}

async fn api_get<T: DeserializeOwned>(
    session: &AuthSession,
    path: &str,
    context: &str,
) -> Result<T, String> {
    ensure_bearer_access()?;
    let server_url = server_url()?;
    let response = http_client()?
        .get(format!("{server_url}{path}"))
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|error| format!("Could not contact Agora server: {error}"))?;
    json_response(response, context).await
}

async fn api_post_json<T: DeserializeOwned, B: Serialize + ?Sized>(
    session: &AuthSession,
    path: &str,
    body: &B,
    context: &str,
) -> Result<T, String> {
    ensure_bearer_access()?;
    let server_url = server_url()?;
    let response = http_client()?
        .post(format!("{server_url}{path}"))
        .bearer_auth(&session.access_token)
        .json(body)
        .send()
        .await
        .map_err(|error| format!("Could not contact Agora server: {error}"))?;
    json_response(response, context).await
}

async fn api_delete<T: DeserializeOwned>(
    session: &AuthSession,
    path: &str,
    context: &str,
) -> Result<T, String> {
    ensure_bearer_access()?;
    let server_url = server_url()?;
    let response = http_client()?
        .delete(format!("{server_url}{path}"))
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|error| format!("Could not contact Agora server: {error}"))?;
    json_response(response, context).await
}

async fn json_response<T: DeserializeOwned>(
    response: reqwest::Response,
    context: &str,
) -> Result<T, String> {
    if response.status().is_success() {
        response
            .json::<T>()
            .await
            .map_err(|error| format!("Could not read Agora response: {error}"))
    } else {
        Err(api_error(response, context).await)
    }
}

pub(super) fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        // Redirects can otherwise replay bearer or refresh credentials to another origin.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Could not configure Agora transport: {error}"))
}

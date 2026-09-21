use super::{
    auth::{
        clear_refresh_token, complete_login, load_refresh_token, login_start_message,
        logout_session, quarantine_refresh_token, refresh_session, store_refresh_token,
        RefreshSessionError, SavedRefreshToken,
    },
    autocomplete::{invalidate_user_autocomplete, reset_user_autocomplete},
    chat::{start_chat_session, stop_chat_session},
    direct_messages::{
        clear_direct_message_pending, reset_direct_message_state, start_direct_message_session,
    },
    relationships::reset_relationship_state,
    server::RemoteLoginProvider,
    state::{
        current_session_matches, next_session_generation, session_generation_current,
        ChatSessionSignals, DirectMessageSignals, OutgoingChatEvent, PresenceSelectionSignals,
        RelationshipSignals, ReportDraft, UpdateSignals, UserAutocompleteSignals,
    },
};
use agora_common::{AuthSession, ChatMessage, FriendshipSummary, PresenceCounts, UserSummary};
use dioxus::prelude::{spawn, Readable, Signal, Writable};
use tokio::sync::mpsc::UnboundedSender;

const SESSION_REFRESH_SAFETY_SECONDS: u64 = 60;
const SESSION_REFRESH_RETRY_INITIAL_SECONDS: u64 = 2;
const SESSION_REFRESH_RETRY_MAX_SECONDS: u64 = 30;

#[allow(clippy::too_many_arguments)]
pub(super) fn start_saved_session_restore(
    mut auth_action_pending: Signal<bool>,
    mut auth_session: Signal<Option<AuthSession>>,
    session_generation: Signal<u64>,
    mut reauth_required: Signal<Option<String>>,
    mut login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    presence_selection: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    chat_send_pending: Signal<bool>,
    updates: UpdateSignals,
    friendships: Signal<Vec<FriendshipSummary>>,
    friendships_load_generation: Signal<u64>,
    friend_search: UserAutocompleteSignals,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    blocks_load_generation: Signal<u64>,
    block_search: UserAutocompleteSignals,
    block_status: Signal<String>,
    report_reason: Signal<String>,
    report_details: Signal<String>,
    report_draft: Signal<Option<ReportDraft>>,
    report_status: Signal<String>,
    direct_messages: DirectMessageSignals,
) {
    spawn(async move {
        match load_refresh_token() {
            Ok(SavedRefreshToken::Scoped(refresh_token)) => {
                let restore_generation = next_session_generation(session_generation);
                let mut retry_attempt = 0u32;
                loop {
                    if !session_generation_current(session_generation, restore_generation) {
                        return;
                    }

                    if retry_attempt == 0 {
                        login_status.set("Restoring saved session...".to_string());
                    }
                    match refresh_session(refresh_token.clone()).await {
                        Ok(session) => {
                            if !session_generation_current(session_generation, restore_generation) {
                                return;
                            }

                            next_session_generation(session_generation);
                            let display_name = session.user.display_name.clone();
                            if let Err(error) = store_refresh_token(&session.refresh_token) {
                                login_status.set(format!(
                                    "Signed in as {display_name}, but token storage failed: {error}"
                                ));
                            } else {
                                login_status.set(format!("Signed in as {display_name}"));
                            }
                            auth_session.set(Some(session.clone()));
                            reauth_required.set(None);
                            let relationship_signals = RelationshipSignals {
                                friendships,
                                friendships_load_generation,
                                friends_status,
                                blocked_users,
                                blocks_load_generation,
                                block_status,
                            };
                            start_direct_message_session(
                                &session,
                                session_generation,
                                direct_messages,
                            );
                            start_chat_session(
                                &session,
                                ChatSessionSignals {
                                    auth_session,
                                    reauth_required,
                                    login_status,
                                    session_generation,
                                    chat_messages,
                                    deleted_message_ids,
                                    chat_status,
                                    presence_counts,
                                    presence_selection,
                                    chat_outbox,
                                    chat_send_pending,
                                    updates,
                                    relationships: relationship_signals,
                                    direct_messages,
                                },
                            );
                            start_session_refresh_loop(
                                session,
                                session_generation,
                                auth_session,
                                reauth_required,
                                login_status,
                                chat_messages,
                                deleted_message_ids,
                                chat_status,
                                presence_counts,
                                presence_selection,
                                chat_outbox,
                                chat_send_pending,
                                updates,
                                friendships,
                                friendships_load_generation,
                                friend_search,
                                friends_status,
                                blocked_users,
                                blocks_load_generation,
                                block_search,
                                block_status,
                                report_draft,
                                direct_messages,
                            );
                            auth_action_pending.set(false);
                            return;
                        }
                        Err(RefreshSessionError::Invalid(error)) => {
                            if !session_generation_current(session_generation, restore_generation) {
                                return;
                            }

                            next_session_generation(session_generation);
                            let clear_error = clear_refresh_token().err();
                            stop_chat_session(
                                chat_messages,
                                deleted_message_ids,
                                chat_status,
                                presence_counts,
                                presence_selection,
                                chat_outbox,
                                chat_send_pending,
                            );
                            reset_relationship_state(
                                friendships,
                                friend_search,
                                friends_status,
                                blocked_users,
                                block_search,
                                block_status,
                                report_reason,
                                report_details,
                                report_draft,
                                report_status,
                            );
                            reset_direct_message_state(direct_messages);
                            let status = match clear_error {
                                Some(clear_error) => format!(
                                    "Saved session is no longer valid: {error}. Local credential cleanup failed: {clear_error}"
                                ),
                                None => format!("Saved session is no longer valid: {error}"),
                            };
                            login_status.set(status);
                            auth_action_pending.set(false);
                            return;
                        }
                        Err(RefreshSessionError::Retryable(error)) => {
                            retry_attempt = retry_attempt.saturating_add(1);
                            let delay = session_refresh_retry_delay(retry_attempt);
                            login_status.set(format!(
                                "Saved session is temporarily unavailable: {error}. Retrying in {delay}s."
                            ));
                            // Let the user choose a fresh sign-in while the saved session retries.
                            auth_action_pending.set(false);
                            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                        }
                        Err(RefreshSessionError::Ambiguous(error)) => {
                            let quarantine_error = quarantine_refresh_token().err();
                            let status = match quarantine_error {
                                Some(quarantine_error) => format!(
                                    "Saved session refresh may have succeeded, but its response was lost: {error}. The saved token was not retried, but could not be marked unsafe locally: {quarantine_error}. Sign in again before restarting Agora."
                                ),
                                None => format!(
                                    "Saved session refresh may have succeeded, but its response was lost: {error}. The saved token was not retried to protect your account. Sign in again."
                                ),
                            };
                            login_status.set(status);
                            auth_action_pending.set(false);
                            return;
                        }
                    }
                }
            }
            Ok(SavedRefreshToken::Ambiguous) => {
                login_status.set(
                    "A previous session refresh had an unknown outcome, so its saved token was not retried. Sign in again."
                        .to_string(),
                );
                auth_action_pending.set(false);
            }
            Ok(SavedRefreshToken::LegacyCredential) => {
                login_status.set(
                    "A legacy saved session was not sent because it is not scoped to this server. Sign in again."
                        .to_string(),
                );
                auth_action_pending.set(false);
            }
            Ok(SavedRefreshToken::None) => {
                login_status.set("Not signed in".to_string());
                auth_action_pending.set(false);
            }
            Err(error) => {
                login_status.set(format!("Authentication unavailable: {error}"));
                auth_action_pending.set(false);
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_session_refresh_loop(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut auth_session: Signal<Option<AuthSession>>,
    mut reauth_required: Signal<Option<String>>,
    mut login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    presence_selection: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    chat_send_pending: Signal<bool>,
    updates: UpdateSignals,
    mut friendships: Signal<Vec<FriendshipSummary>>,
    friendships_load_generation: Signal<u64>,
    friend_search: UserAutocompleteSignals,
    mut friends_status: Signal<String>,
    mut blocked_users: Signal<Vec<UserSummary>>,
    blocks_load_generation: Signal<u64>,
    block_search: UserAutocompleteSignals,
    mut block_status: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    direct_messages: DirectMessageSignals,
) {
    spawn(async move {
        let mut session = session;
        let mut generation = *session_generation.read();
        let mut retry_attempt = 0u32;
        let mut next_delay = session_refresh_delay(session.expires_in_seconds);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(next_delay)).await;

            if !current_session_matches(
                auth_session,
                session_generation,
                generation,
                &session.refresh_token,
            ) {
                return;
            }

            match refresh_session(session.refresh_token.clone()).await {
                Ok(refreshed) => {
                    if !current_session_matches(
                        auth_session,
                        session_generation,
                        generation,
                        &session.refresh_token,
                    ) {
                        return;
                    }

                    generation = next_session_generation(session_generation);
                    clear_direct_message_pending(direct_messages);
                    invalidate_user_autocomplete(friend_search);
                    invalidate_user_autocomplete(block_search);
                    retry_attempt = 0;
                    let display_name = refreshed.user.display_name.clone();
                    match store_refresh_token(&refreshed.refresh_token) {
                        Ok(()) => login_status.set(format!("Signed in as {display_name}")),
                        Err(error) => login_status.set(format!(
                            "Session refreshed for {display_name}, but token storage failed: {error}"
                        )),
                    }
                    auth_session.set(Some(refreshed.clone()));
                    reauth_required.set(None);
                    let relationship_signals = RelationshipSignals {
                        friendships,
                        friendships_load_generation,
                        friends_status,
                        blocked_users,
                        blocks_load_generation,
                        block_status,
                    };
                    start_chat_session(
                        &refreshed,
                        ChatSessionSignals {
                            auth_session,
                            reauth_required,
                            login_status,
                            session_generation,
                            chat_messages,
                            deleted_message_ids,
                            chat_status,
                            presence_counts,
                            presence_selection,
                            chat_outbox,
                            chat_send_pending,
                            updates,
                            relationships: relationship_signals,
                            direct_messages,
                        },
                    );
                    session = refreshed;
                    next_delay = session_refresh_delay(session.expires_in_seconds);
                }
                Err(RefreshSessionError::Invalid(error)) => {
                    if !current_session_matches(
                        auth_session,
                        session_generation,
                        generation,
                        &session.refresh_token,
                    ) {
                        return;
                    }

                    next_session_generation(session_generation);
                    let clear_error = clear_refresh_token().err();
                    auth_session.set(None);
                    stop_chat_session(
                        chat_messages,
                        deleted_message_ids,
                        chat_status,
                        presence_counts,
                        presence_selection,
                        chat_outbox,
                        chat_send_pending,
                    );
                    friendships.set(Vec::new());
                    reset_user_autocomplete(friend_search);
                    friends_status.set("Sign in to load friends".to_string());
                    blocked_users.set(Vec::new());
                    reset_user_autocomplete(block_search);
                    block_status.set("Sign in to manage blocks".to_string());
                    report_draft.set(None);
                    reset_direct_message_state(direct_messages);
                    let status = match clear_error {
                        Some(clear_error) => format!(
                            "Session is no longer valid: {error}. Local credential cleanup failed: {clear_error}"
                        ),
                        None => format!("Session is no longer valid: {error}"),
                    };
                    login_status.set(status);
                    return;
                }
                Err(RefreshSessionError::Retryable(error)) => {
                    if !current_session_matches(
                        auth_session,
                        session_generation,
                        generation,
                        &session.refresh_token,
                    ) {
                        return;
                    }

                    retry_attempt = retry_attempt.saturating_add(1);
                    next_delay = session_refresh_retry_delay(retry_attempt);
                    login_status.set(format!(
                        "Session refresh is temporarily unavailable: {error}. Retrying in {next_delay}s."
                    ));
                }
                Err(RefreshSessionError::Ambiguous(error)) => {
                    if !current_session_matches(
                        auth_session,
                        session_generation,
                        generation,
                        &session.refresh_token,
                    ) {
                        return;
                    }

                    let quarantine_error = quarantine_refresh_token().err();
                    let status = match quarantine_error {
                        Some(quarantine_error) => format!(
                            "Session refresh may have succeeded, but its response was lost: {error}. The saved token was not retried, but could not be marked unsafe locally: {quarantine_error}. Sign in again before restarting Agora."
                        ),
                        None => format!(
                            "Session refresh may have succeeded, but its response was lost: {error}. The current session can continue until it expires; sign in again before restarting Agora."
                        ),
                    };
                    reauth_required.set(Some(status.clone()));
                    login_status.set(status);
                    return;
                }
            }
        }
    });
}

fn session_refresh_delay(expires_in_seconds: u64) -> u64 {
    expires_in_seconds
        .saturating_sub(SESSION_REFRESH_SAFETY_SECONDS)
        .max(1)
}

pub(super) fn session_refresh_retry_delay(attempt: u32) -> u64 {
    let exponent = attempt.saturating_sub(1).min(4);
    SESSION_REFRESH_RETRY_INITIAL_SECONDS
        .saturating_mul(1u64 << exponent)
        .min(SESSION_REFRESH_RETRY_MAX_SECONDS)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sign_in_session(
    mut auth_action_pending: Signal<bool>,
    mut auth_session: Signal<Option<AuthSession>>,
    session_generation: Signal<u64>,
    mut reauth_required: Signal<Option<String>>,
    local_dev_account_id: String,
    remote_login_provider: RemoteLoginProvider,
    mut login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    mut chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    presence_selection: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    chat_send_pending: Signal<bool>,
    updates: UpdateSignals,
    friendships: Signal<Vec<FriendshipSummary>>,
    friendships_load_generation: Signal<u64>,
    friend_search: UserAutocompleteSignals,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    blocks_load_generation: Signal<u64>,
    block_search: UserAutocompleteSignals,
    block_status: Signal<String>,
    report_draft: Signal<Option<ReportDraft>>,
    direct_messages: DirectMessageSignals,
) {
    if *auth_action_pending.read() {
        return;
    }
    auth_action_pending.set(true);
    next_session_generation(session_generation);
    clear_direct_message_pending(direct_messages);
    reset_user_autocomplete(friend_search);
    reset_user_autocomplete(block_search);

    spawn(async move {
        let start_message = login_start_message(&local_dev_account_id, remote_login_provider);
        login_status.set(start_message.clone());
        chat_status.set(start_message);
        match complete_login(&local_dev_account_id, remote_login_provider).await {
            Ok(session) => {
                next_session_generation(session_generation);
                let display_name = session.user.display_name.clone();
                match store_refresh_token(&session.refresh_token) {
                    Ok(()) => login_status.set(format!("Signed in as {display_name}")),
                    Err(error) => login_status.set(format!(
                        "Signed in as {display_name}, but token storage failed: {error}"
                    )),
                }
                auth_session.set(Some(session.clone()));
                reauth_required.set(None);
                let relationship_signals = RelationshipSignals {
                    friendships,
                    friendships_load_generation,
                    friends_status,
                    blocked_users,
                    blocks_load_generation,
                    block_status,
                };
                start_direct_message_session(&session, session_generation, direct_messages);
                start_chat_session(
                    &session,
                    ChatSessionSignals {
                        auth_session,
                        reauth_required,
                        login_status,
                        session_generation,
                        chat_messages,
                        deleted_message_ids,
                        chat_status,
                        presence_counts,
                        presence_selection,
                        chat_outbox,
                        chat_send_pending,
                        updates,
                        relationships: relationship_signals,
                        direct_messages,
                    },
                );
                start_session_refresh_loop(
                    session,
                    session_generation,
                    auth_session,
                    reauth_required,
                    login_status,
                    chat_messages,
                    deleted_message_ids,
                    chat_status,
                    presence_counts,
                    presence_selection,
                    chat_outbox,
                    chat_send_pending,
                    updates,
                    friendships,
                    friendships_load_generation,
                    friend_search,
                    friends_status,
                    blocked_users,
                    blocks_load_generation,
                    block_search,
                    block_status,
                    report_draft,
                    direct_messages,
                );
            }
            Err(error) => {
                login_status.set(error.clone());
                chat_status.set(error);
            }
        }
        auth_action_pending.set(false);
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sign_out_session(
    session: AuthSession,
    mut auth_action_pending: Signal<bool>,
    mut auth_session: Signal<Option<AuthSession>>,
    session_generation: Signal<u64>,
    mut reauth_required: Signal<Option<String>>,
    mut login_status: Signal<String>,
    chat_messages: Signal<Vec<ChatMessage>>,
    deleted_message_ids: Signal<Vec<uuid::Uuid>>,
    chat_status: Signal<String>,
    presence_counts: Signal<PresenceCounts>,
    presence_selection: PresenceSelectionSignals,
    chat_outbox: Signal<Option<UnboundedSender<OutgoingChatEvent>>>,
    chat_send_pending: Signal<bool>,
    friendships: Signal<Vec<FriendshipSummary>>,
    friend_search: UserAutocompleteSignals,
    friends_status: Signal<String>,
    blocked_users: Signal<Vec<UserSummary>>,
    block_search: UserAutocompleteSignals,
    block_status: Signal<String>,
    report_reason: Signal<String>,
    report_details: Signal<String>,
    report_draft: Signal<Option<ReportDraft>>,
    report_status: Signal<String>,
    direct_messages: DirectMessageSignals,
) {
    if *auth_action_pending.read() {
        return;
    }
    auth_action_pending.set(true);
    next_session_generation(session_generation);
    clear_direct_message_pending(direct_messages);
    reauth_required.set(None);

    let refresh_token = session.refresh_token;
    spawn(async move {
        login_status.set("Signing out...".to_string());
        auth_session.set(None);
        stop_chat_session(
            chat_messages,
            deleted_message_ids,
            chat_status,
            presence_counts,
            presence_selection,
            chat_outbox,
            chat_send_pending,
        );
        reset_relationship_state(
            friendships,
            friend_search,
            friends_status,
            blocked_users,
            block_search,
            block_status,
            report_reason,
            report_details,
            report_draft,
            report_status,
        );
        reset_direct_message_state(direct_messages);

        let clear_result = clear_refresh_token();
        let logout_result = logout_session(refresh_token).await;
        match (logout_result, clear_result) {
            (Ok(true), Ok(())) => {
                login_status.set("Signed out. Server session revoked.".to_string())
            }
            (Ok(false), Ok(())) => login_status
                .set("Signed out locally. Server session was already inactive.".to_string()),
            (Err(error), Ok(())) => {
                login_status.set(format!("Signed out locally. Server logout failed: {error}"))
            }
            (Ok(true), Err(error)) => login_status.set(format!(
                "Server session revoked. Local credential cleanup failed: {error}"
            )),
            (Ok(false), Err(error)) => login_status.set(format!(
                "Server session was already inactive. Local credential cleanup failed: {error}"
            )),
            (Err(logout_error), Err(clear_error)) => login_status.set(format!(
                "Sign-out had issues. Server: {logout_error}. Local: {clear_error}"
            )),
        }
        auth_action_pending.set(false);
    });
}

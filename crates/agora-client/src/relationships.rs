use super::{
    api::{
        accept_friend_request_api, block_user_api, create_report_api, decline_friend_request_api,
        list_blocks_api, list_friends_api, remove_friend_api, send_friend_request_api,
        unblock_user_api,
    },
    autocomplete::reset_user_autocomplete,
    direct_messages::{refresh_direct_message_state, remove_direct_messages_for_user},
    state::{
        next_relationship_load_generation, relationship_load_generation_current,
        session_generation_current, DirectMessageSignals, RelationshipSignals, ReportDraft,
        UserAutocompleteSignals,
    },
};
use agora_common::{
    AuthSession, ChatMessage, DmMessage, FriendshipSummary, MessageKind, UserSummary,
};
use dioxus::prelude::{spawn, Readable, Signal, Writable};

pub(super) fn refresh_relationship_state(
    session: &AuthSession,
    session_generation: Signal<u64>,
    signals: RelationshipSignals,
) {
    load_friends(
        session.clone(),
        session_generation,
        signals.friendships_load_generation,
        signals.friendships,
        signals.friends_status,
    );
    load_blocks(
        session.clone(),
        session_generation,
        signals.blocks_load_generation,
        signals.blocked_users,
        signals.block_status,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reset_relationship_state(
    mut friendships: Signal<Vec<FriendshipSummary>>,
    friend_search: UserAutocompleteSignals,
    mut friends_status: Signal<String>,
    mut blocked_users: Signal<Vec<UserSummary>>,
    block_search: UserAutocompleteSignals,
    mut block_status: Signal<String>,
    mut report_reason: Signal<String>,
    mut report_details: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_status: Signal<String>,
) {
    friendships.set(Vec::new());
    reset_user_autocomplete(friend_search);
    friends_status.set("Sign in to load friends".to_string());
    blocked_users.set(Vec::new());
    reset_user_autocomplete(block_search);
    block_status.set("Sign in to manage blocks".to_string());
    report_reason.set(String::new());
    report_details.set(String::new());
    report_draft.set(None);
    report_status.set("Search for a user to report".to_string());
}

pub(super) fn load_friends(
    session: AuthSession,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    mut friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    let load_generation = next_relationship_load_generation(friendships_load_generation);
    spawn(async move {
        if !session_generation_current(session_generation, generation)
            || !relationship_load_generation_current(friendships_load_generation, load_generation)
        {
            return;
        }
        friends_status.set("Loading friends...".to_string());
        match list_friends_api(&session).await {
            Ok(items) => {
                if !session_generation_current(session_generation, generation)
                    || !relationship_load_generation_current(
                        friendships_load_generation,
                        load_generation,
                    )
                {
                    return;
                }
                let count = items.len();
                friendships.set(items);
                friends_status.set(format!("Loaded {count} friendship records"));
            }
            Err(error) => {
                if session_generation_current(session_generation, generation)
                    && relationship_load_generation_current(
                        friendships_load_generation,
                        load_generation,
                    )
                {
                    friends_status.set(error);
                }
            }
        }
    });
}

pub(super) fn load_blocks(
    session: AuthSession,
    session_generation: Signal<u64>,
    blocks_load_generation: Signal<u64>,
    mut blocked_users: Signal<Vec<UserSummary>>,
    mut block_status: Signal<String>,
) {
    let generation = *session_generation.read();
    let load_generation = next_relationship_load_generation(blocks_load_generation);
    spawn(async move {
        if !session_generation_current(session_generation, generation)
            || !relationship_load_generation_current(blocks_load_generation, load_generation)
        {
            return;
        }
        block_status.set("Loading blocked users...".to_string());
        match list_blocks_api(&session).await {
            Ok(users) => {
                if !session_generation_current(session_generation, generation)
                    || !relationship_load_generation_current(
                        blocks_load_generation,
                        load_generation,
                    )
                {
                    return;
                }
                let count = users.len();
                blocked_users.set(users);
                block_status.set(format!("Loaded {count} blocked users"));
            }
            Err(error) => {
                if session_generation_current(session_generation, generation)
                    && relationship_load_generation_current(blocks_load_generation, load_generation)
                {
                    block_status.set(error);
                }
            }
        }
    });
}

pub(super) fn send_friend_invite(
    session: AuthSession,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    target: UserSummary,
    friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        friends_status.set(format!(
            "Sending friend request to {}...",
            target.display_name
        ));
        match send_friend_request_api(&session, &target.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                friends_status.set(format!("Friend request sent to {}", target.display_name));
                load_friends(
                    session,
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    friends_status.set(error);
                }
            }
        }
    });
}

pub(super) fn accept_friend_invite(
    session: AuthSession,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    friendship: FriendshipSummary,
    friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        friends_status.set("Accepting friend request...".to_string());
        match accept_friend_request_api(&session, &friendship.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                friends_status.set("Friend request accepted".to_string());
                load_friends(
                    session,
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    friends_status.set(error);
                }
            }
        }
    });
}

pub(super) fn decline_friend_invite(
    session: AuthSession,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    friendship: FriendshipSummary,
    friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        friends_status.set("Declining friend request...".to_string());
        match decline_friend_request_api(&session, &friendship.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                friends_status.set("Friend request declined".to_string());
                load_friends(
                    session,
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    friends_status.set(error);
                }
            }
        }
    });
}

pub(super) fn remove_friend(
    session: AuthSession,
    session_generation: Signal<u64>,
    friendships_load_generation: Signal<u64>,
    target: UserSummary,
    friendships: Signal<Vec<FriendshipSummary>>,
    mut friends_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        friends_status.set(format!("Removing {}...", target.display_name));
        match remove_friend_api(&session, &target.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                friends_status.set(format!("Removed {}", target.display_name));
                load_friends(
                    session,
                    session_generation,
                    friendships_load_generation,
                    friendships,
                    friends_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    friends_status.set(error);
                }
            }
        }
    });
}

pub(super) fn block_user_action(
    session: AuthSession,
    session_generation: Signal<u64>,
    target: UserSummary,
    mut chat_messages: Signal<Vec<ChatMessage>>,
    mut block_status: Signal<String>,
    signals: RelationshipSignals,
    direct_messages: DirectMessageSignals,
) {
    let generation = *session_generation.read();
    let blocked_user_id = target.id;
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        block_status.set(format!("Blocking {}...", target.display_name));
        match block_user_api(&session, &target.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                {
                    let mut messages = chat_messages.write();
                    remove_blocked_user_messages(&mut messages, blocked_user_id);
                }
                remove_direct_messages_for_user(direct_messages, blocked_user_id);
                block_status.set(format!("Blocked {}", target.display_name));
                refresh_direct_message_state(&session, session_generation, direct_messages);
                load_blocks(
                    session.clone(),
                    session_generation,
                    signals.blocks_load_generation,
                    signals.blocked_users,
                    block_status,
                );
                load_friends(
                    session,
                    session_generation,
                    signals.friendships_load_generation,
                    signals.friendships,
                    signals.friends_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    block_status.set(error);
                }
            }
        }
    });
}

pub(super) fn remove_blocked_user_messages(
    messages: &mut Vec<ChatMessage>,
    blocked_user_id: uuid::Uuid,
) {
    messages.retain(|message| message.author.id != blocked_user_id);
}

pub(super) fn unblock_user_action(
    session: AuthSession,
    session_generation: Signal<u64>,
    blocks_load_generation: Signal<u64>,
    target: UserSummary,
    blocked_users: Signal<Vec<UserSummary>>,
    mut block_status: Signal<String>,
) {
    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        block_status.set(format!("Unblocking {}...", target.display_name));
        match unblock_user_api(&session, &target.id.to_string()).await {
            Ok(_) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }
                block_status.set(format!("Unblocked {}", target.display_name));
                load_blocks(
                    session,
                    session_generation,
                    blocks_load_generation,
                    blocked_users,
                    block_status,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    block_status.set(error);
                }
            }
        }
    });
}

pub(super) fn user_report_draft(user: UserSummary) -> ReportDraft {
    ReportDraft {
        target: user,
        message_id: None,
        message_kind: None,
        message_preview: None,
    }
}

pub(super) fn global_message_report_draft(message: &ChatMessage) -> ReportDraft {
    ReportDraft {
        target: message.author.clone(),
        message_id: Some(message.id),
        message_kind: Some(MessageKind::Global),
        message_preview: Some(message.body.clone()),
    }
}

pub(super) fn direct_message_report_draft(message: &DmMessage) -> ReportDraft {
    ReportDraft {
        target: message.author.clone(),
        message_id: Some(message.id),
        message_kind: Some(MessageKind::Dm),
        message_preview: Some(message.body.clone()),
    }
}

pub(super) fn report_draft_status(draft: &ReportDraft) -> String {
    if draft.message_id.is_some() {
        format!(
            "Selected message from {}. Enter a reason, then submit.",
            draft.target.display_name
        )
    } else {
        format!(
            "Selected {}. Enter a reason, then submit.",
            draft.target.display_name
        )
    }
}

pub(super) fn report_draft_kind_label(draft: &ReportDraft) -> &'static str {
    match draft.message_kind {
        Some(MessageKind::Global) => "Global message",
        Some(MessageKind::Dm) => "Direct message",
        None => "User",
    }
}

pub(super) fn report_preview_text(value: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 140;
    let mut preview = value.chars().take(MAX_PREVIEW_CHARS).collect::<String>();
    if value.chars().count() > MAX_PREVIEW_CHARS {
        preview.push_str("...");
    }
    preview
}

#[allow(clippy::too_many_arguments)]
pub(super) fn submit_report_action(
    session: AuthSession,
    session_generation: Signal<u64>,
    draft: ReportDraft,
    reason: String,
    details: String,
    mut report_status: Signal<String>,
    mut report_draft: Signal<Option<ReportDraft>>,
    mut report_reason: Signal<String>,
    mut report_details: Signal<String>,
) {
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        report_status.set("Report reason is required".to_string());
        return;
    }
    let details = details.trim().to_string();
    let details = if details.is_empty() {
        None
    } else {
        Some(details)
    };

    let generation = *session_generation.read();
    spawn(async move {
        if !session_generation_current(session_generation, generation) {
            return;
        }
        let target_name = draft.target.display_name.clone();
        report_status.set(format!("Reporting {target_name}..."));
        match create_report_api(
            &session,
            draft.target.id,
            draft.message_id,
            draft.message_kind,
            reason,
            details,
        )
        .await
        {
            Ok(report_id) => {
                if session_generation_current(session_generation, generation) {
                    report_draft.set(None);
                    report_reason.set(String::new());
                    report_details.set(String::new());
                    report_status.set(format!("Report submitted for {target_name}: {report_id}"));
                }
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    report_status.set(error);
                }
            }
        }
    });
}

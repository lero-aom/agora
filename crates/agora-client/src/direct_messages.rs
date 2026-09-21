use super::{
    api::{
        create_direct_message_thread_api, list_direct_message_history_api,
        list_direct_message_threads_api, send_direct_message_api,
    },
    autocomplete::{invalidate_user_autocomplete, reset_user_autocomplete},
    state::{
        direct_message_load_generation_current, next_direct_message_load_generation,
        session_generation_current, AppTab, DirectMessageSignals,
    },
};
use agora_common::{AuthSession, DmMessage, DmThreadSummary, UserSummary, MAX_MESSAGE_LEN};
use dioxus::prelude::{spawn, Readable, Signal, Writable};

pub(super) fn reset_direct_message_state(mut signals: DirectMessageSignals) {
    signals.threads.set(Vec::new());
    signals.selected_thread_id.set(None);
    signals.messages.set(Vec::new());
    signals.next_before_message_id.set(None);
    signals
        .status
        .set("Sign in to load direct messages".to_string());
    signals.composer_body.set(String::new());
    reset_user_autocomplete(signals.search);
    clear_direct_message_pending(signals);
}

pub(super) fn clear_direct_message_pending(mut signals: DirectMessageSignals) {
    // A token refresh invalidates all result guards. Clear these flags before starting new
    // requests so an old task that exits on its generation check cannot strand the UI.
    next_direct_message_load_generation(signals.threads_load_generation);
    next_direct_message_load_generation(signals.history_load_generation);
    invalidate_user_autocomplete(signals.search);
    signals.threads_pending.set(false);
    signals.history_pending.set(false);
    signals.create_pending.set(false);
    signals.send_pending.set(false);
}

pub(super) fn start_direct_message_session(
    _session: &AuthSession,
    _session_generation: Signal<u64>,
    signals: DirectMessageSignals,
) {
    reset_direct_message_state(signals);
}

pub(super) fn refresh_direct_message_state(
    session: &AuthSession,
    session_generation: Signal<u64>,
    signals: DirectMessageSignals,
) {
    load_direct_message_threads(session.clone(), session_generation, signals);
    if let Some(thread_id) = *signals.selected_thread_id.read() {
        load_direct_message_history_page(
            session.clone(),
            session_generation,
            signals,
            thread_id,
            None,
            false,
        );
    }
}

pub(super) fn load_direct_message_threads(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
) {
    let generation = *session_generation.read();
    let load_generation = next_direct_message_load_generation(signals.threads_load_generation);
    signals.threads_pending.set(true);
    signals.status.set("Loading direct messages...".to_string());
    spawn(async move {
        match list_direct_message_threads_api(&session).await {
            Ok(threads) => {
                if !session_generation_current(session_generation, generation)
                    || !direct_message_load_generation_current(
                        signals.threads_load_generation,
                        load_generation,
                    )
                {
                    return;
                }

                let selected_thread_id = *signals.selected_thread_id.read();
                let selected_is_visible = match selected_thread_id {
                    Some(thread_id) => threads.iter().any(|thread| thread.id == thread_id),
                    None => true,
                };
                let count = threads.len();
                signals.threads.set(threads);
                signals.threads_pending.set(false);
                if selected_is_visible {
                    signals
                        .status
                        .set(format!("Loaded {count} direct message threads"));
                } else {
                    signals.selected_thread_id.set(None);
                    signals.messages.set(Vec::new());
                    signals.next_before_message_id.set(None);
                    signals.composer_body.set(String::new());
                    signals
                        .status
                        .set("That direct message is no longer available".to_string());
                }
            }
            Err(error) => {
                if session_generation_current(session_generation, generation)
                    && direct_message_load_generation_current(
                        signals.threads_load_generation,
                        load_generation,
                    )
                {
                    signals.threads_pending.set(false);
                    signals.status.set(error);
                }
            }
        }
    });
}

pub(super) fn open_direct_message_thread(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
    thread_id: uuid::Uuid,
) {
    signals.selected_thread_id.set(Some(thread_id));
    signals.messages.set(Vec::new());
    signals.next_before_message_id.set(None);
    signals.composer_body.set(String::new());
    load_direct_message_history_page(session, session_generation, signals, thread_id, None, true);
}

pub(super) fn load_direct_message_history_page(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
    thread_id: uuid::Uuid,
    before: Option<uuid::Uuid>,
    replace: bool,
) {
    if before.is_some() && *signals.history_pending.read() {
        return;
    }

    let generation = *session_generation.read();
    let load_generation = next_direct_message_load_generation(signals.history_load_generation);
    signals.history_pending.set(true);
    if replace {
        signals
            .status
            .set("Loading direct message history...".to_string());
    }
    spawn(async move {
        match list_direct_message_history_api(&session, thread_id, before).await {
            Ok(history) => {
                if !session_generation_current(session_generation, generation)
                    || !direct_message_load_generation_current(
                        signals.history_load_generation,
                        load_generation,
                    )
                    || *signals.selected_thread_id.read() != Some(thread_id)
                {
                    return;
                }

                merge_direct_messages(&mut signals.messages.write(), history.messages);
                signals
                    .next_before_message_id
                    .set(history.next_before_message_id);
                signals.history_pending.set(false);
                signals
                    .status
                    .set("Direct message history loaded".to_string());
            }
            Err(error) => {
                if session_generation_current(session_generation, generation)
                    && direct_message_load_generation_current(
                        signals.history_load_generation,
                        load_generation,
                    )
                    && *signals.selected_thread_id.read() == Some(thread_id)
                {
                    signals.history_pending.set(false);
                    signals.status.set(error);
                }
            }
        }
    });
}

pub(super) fn create_direct_message_thread(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
    mut active_tab: Signal<AppTab>,
    target: UserSummary,
) {
    if *signals.create_pending.read() {
        return;
    }

    let generation = *session_generation.read();
    signals.create_pending.set(true);
    signals.status.set(format!(
        "Opening a conversation with {}...",
        target.display_name
    ));
    spawn(async move {
        match create_direct_message_thread_api(&session, target.id).await {
            Ok(thread) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }

                upsert_direct_message_thread(&mut signals.threads.write(), thread.clone());
                signals.selected_thread_id.set(Some(thread.id));
                signals.messages.set(Vec::new());
                signals.next_before_message_id.set(None);
                signals.composer_body.set(String::new());
                signals.create_pending.set(false);
                active_tab.set(AppTab::DirectMessages);
                load_direct_message_history_page(
                    session,
                    session_generation,
                    signals,
                    thread.id,
                    None,
                    true,
                );
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    signals.create_pending.set(false);
                    signals.status.set(error);
                }
            }
        }
    });
}

pub(super) fn send_direct_message(
    session: AuthSession,
    session_generation: Signal<u64>,
    mut signals: DirectMessageSignals,
    thread_id: uuid::Uuid,
) {
    if *signals.send_pending.read() {
        return;
    }

    let body = signals.composer_body.read().trim().to_string();
    if body.is_empty() {
        return;
    }
    if body.chars().count() > MAX_MESSAGE_LEN {
        signals.status.set(format!(
            "Message is too long. Limit: {MAX_MESSAGE_LEN} characters"
        ));
        return;
    }

    let generation = *session_generation.read();
    signals.send_pending.set(true);
    signals.status.set("Sending direct message...".to_string());
    spawn(async move {
        match send_direct_message_api(&session, thread_id, body).await {
            Ok(message) => {
                if !session_generation_current(session_generation, generation) {
                    return;
                }

                signals.send_pending.set(false);
                if *signals.selected_thread_id.read() == Some(thread_id) {
                    merge_direct_messages(&mut signals.messages.write(), vec![message]);
                    signals.composer_body.set(String::new());
                }
                signals.status.set("Direct message sent".to_string());
                load_direct_message_threads(session, session_generation, signals);
            }
            Err(error) => {
                if session_generation_current(session_generation, generation) {
                    signals.send_pending.set(false);
                    signals.status.set(error);
                }
            }
        }
    });
}

pub(super) fn remove_direct_messages_for_user(
    mut signals: DirectMessageSignals,
    user_id: uuid::Uuid,
) {
    let selected_thread_id = *signals.selected_thread_id.read();
    let selected_is_hidden = selected_thread_id.is_some_and(|thread_id| {
        signals
            .threads
            .read()
            .iter()
            .any(|thread| thread.id == thread_id && thread.other_user.id == user_id)
    });
    signals
        .threads
        .write()
        .retain(|thread| thread.other_user.id != user_id);
    if selected_is_hidden {
        signals.selected_thread_id.set(None);
        signals.messages.set(Vec::new());
        signals.next_before_message_id.set(None);
        signals.composer_body.set(String::new());
        signals
            .status
            .set("Direct message hidden because this user is blocked".to_string());
    }
}

pub(super) fn upsert_direct_message_thread(
    threads: &mut Vec<DmThreadSummary>,
    thread: DmThreadSummary,
) {
    match threads.iter_mut().find(|existing| existing.id == thread.id) {
        Some(existing) => *existing = thread,
        None => threads.push(thread),
    }
    threads.sort_by(|left, right| {
        let left_time = left.last_message_at.as_deref().unwrap_or(&left.created_at);
        let right_time = right
            .last_message_at
            .as_deref()
            .unwrap_or(&right.created_at);
        right_time
            .cmp(left_time)
            .then_with(|| right.id.cmp(&left.id))
    });
}

pub(super) fn merge_direct_messages(messages: &mut Vec<DmMessage>, incoming: Vec<DmMessage>) {
    for message in incoming {
        if !messages.iter().any(|existing| existing.id == message.id) {
            messages.push(message);
        }
    }
    messages.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
}

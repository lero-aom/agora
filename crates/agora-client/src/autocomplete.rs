use super::{
    api::search_users_api,
    state::{session_generation_current, UserAutocompleteSignals},
};
use agora_common::AuthSession;
use dioxus::prelude::{spawn, Readable, Signal, Writable};

const USER_AUTOCOMPLETE_MIN_QUERY_CHARS: usize = 2;
const USER_AUTOCOMPLETE_DEBOUNCE_MS: u64 = 300;

pub(super) fn user_autocomplete_query_ready(query: &str) -> bool {
    query.trim().chars().count() >= USER_AUTOCOMPLETE_MIN_QUERY_CHARS
}

fn next_user_autocomplete_generation(mut signals: UserAutocompleteSignals) -> u64 {
    let current = *signals.request_generation.read();
    let next = current.wrapping_add(1);
    signals.request_generation.set(next);
    next
}

fn user_autocomplete_request_current(
    session_generation: Signal<u64>,
    session_value: u64,
    signals: UserAutocompleteSignals,
    request_generation: u64,
    query: &str,
) -> bool {
    session_generation_current(session_generation, session_value)
        && *signals.request_generation.read() == request_generation
        && signals.query.read().trim() == query
}

pub(super) fn update_user_autocomplete_query(
    session: Option<AuthSession>,
    session_generation: Signal<u64>,
    mut signals: UserAutocompleteSignals,
    value: String,
) {
    let query = value.trim().to_string();
    signals.query.set(value);
    signals.selected.set(None);
    signals.results.set(Vec::new());
    signals.active_index.set(None);
    let request_generation = next_user_autocomplete_generation(signals);

    if !user_autocomplete_query_ready(&query) {
        signals.pending.set(false);
        signals.expanded.set(false);
        signals.status.set(if query.is_empty() {
            String::new()
        } else {
            format!("Type at least {USER_AUTOCOMPLETE_MIN_QUERY_CHARS} characters to search")
        });
        return;
    }

    let Some(session) = session else {
        signals.pending.set(false);
        signals.expanded.set(false);
        signals
            .status
            .set("Sign in to search for users".to_string());
        return;
    };

    let session_value = *session_generation.read();
    signals.pending.set(true);
    signals.expanded.set(true);
    signals.status.set(format!("Searching for {query}..."));
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(
            USER_AUTOCOMPLETE_DEBOUNCE_MS,
        ))
        .await;
        if !user_autocomplete_request_current(
            session_generation,
            session_value,
            signals,
            request_generation,
            &query,
        ) {
            return;
        }

        match search_users_api(&session, &query).await {
            Ok(users) => {
                if !user_autocomplete_request_current(
                    session_generation,
                    session_value,
                    signals,
                    request_generation,
                    &query,
                ) {
                    return;
                }

                let count = users.len();
                signals.results.set(users);
                signals.pending.set(false);
                signals.active_index.set((count > 0).then_some(0));
                signals.status.set(if count == 0 {
                    "No matching users".to_string()
                } else {
                    format!("Found {count} users")
                });
            }
            Err(error) => {
                if user_autocomplete_request_current(
                    session_generation,
                    session_value,
                    signals,
                    request_generation,
                    &query,
                ) {
                    signals.results.set(Vec::new());
                    signals.pending.set(false);
                    signals.active_index.set(None);
                    signals.status.set(error);
                }
            }
        }
    });
}

pub(super) fn select_user_autocomplete_option(mut signals: UserAutocompleteSignals, index: usize) {
    let Some(user) = signals.results.read().get(index).cloned() else {
        return;
    };

    next_user_autocomplete_generation(signals);
    let display_name = user.display_name.clone();
    signals.query.set(display_name.clone());
    signals.results.set(Vec::new());
    signals.selected.set(Some(user));
    signals.pending.set(false);
    signals.expanded.set(false);
    signals.active_index.set(None);
    signals.status.set(format!("Selected {display_name}"));
}

pub(super) fn next_user_autocomplete_active_index(
    count: usize,
    active: Option<usize>,
    move_forward: bool,
) -> Option<usize> {
    match (count, active, move_forward) {
        (0, _, _) => None,
        (count, Some(index), true) if index < count => Some((index + 1) % count),
        (count, Some(index), false) if index < count => Some((index + count - 1) % count),
        (_, _, true) => Some(0),
        (count, _, false) => Some(count - 1),
    }
}

pub(super) fn move_user_autocomplete_active_option(
    mut signals: UserAutocompleteSignals,
    move_forward: bool,
) {
    let next = next_user_autocomplete_active_index(
        signals.results.read().len(),
        *signals.active_index.read(),
        move_forward,
    );
    signals.active_index.set(next);
    if next.is_some() {
        signals.expanded.set(true);
    }
}

pub(super) fn dismiss_user_autocomplete(mut signals: UserAutocompleteSignals) {
    signals.expanded.set(false);
    signals.active_index.set(None);
}

pub(super) fn show_user_autocomplete(mut signals: UserAutocompleteSignals) {
    if signals.selected.read().is_none() && user_autocomplete_query_ready(&signals.query.read()) {
        signals.expanded.set(true);
    }
}

pub(super) fn invalidate_user_autocomplete(mut signals: UserAutocompleteSignals) {
    next_user_autocomplete_generation(signals);
    signals.pending.set(false);
    dismiss_user_autocomplete(signals);
}

pub(super) fn reset_user_autocomplete(mut signals: UserAutocompleteSignals) {
    invalidate_user_autocomplete(signals);
    signals.query.set(String::new());
    signals.results.set(Vec::new());
    signals.selected.set(None);
    signals.status.set(String::new());
}

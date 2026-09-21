use super::{
    server::is_local_server_url,
    state::{UpdateSignals, UpdateUiStatus},
    update,
};
use dioxus::prelude::{spawn, Readable, Signal, Writable};

pub(super) fn initial_update_status() -> UpdateUiStatus {
    match update::startup_notice() {
        Some(notice) => UpdateUiStatus::Recovery(notice),
        None if update::is_configured() => UpdateUiStatus::Checking { required: None },
        None => UpdateUiStatus::Bootstrap,
    }
}

pub(super) fn check_for_updates(signals: UpdateSignals, required: Option<String>) {
    let mut status = signals.status;
    let mut available = signals.available;
    let mut pending = signals.pending;
    let generation = signals.generation;
    let required = required.or_else(|| status.read().required_reason());

    // AGORA_SERVER_URL is consulted only to disable update traffic for local development.
    // All metadata and package URLs come from the compiled updater configuration.
    if is_local_server_url() {
        available.set(None);
        pending.set(false);
        status.set(UpdateUiStatus::SkippedLoopback);
        return;
    }

    let current_generation = next_update_generation(generation);
    available.set(None);
    pending.set(true);
    status.set(UpdateUiStatus::Checking {
        required: required.clone(),
    });
    spawn(async move {
        let result = update::check_for_update().await;
        if *generation.read() != current_generation {
            return;
        }

        pending.set(false);
        match result {
            Ok(update::CheckResult::Disabled) => status.set(UpdateUiStatus::Bootstrap),
            Ok(update::CheckResult::UpToDate) if required.is_some() => {
                status.set(UpdateUiStatus::Failed {
                    message: "No newer signed update is published for this requirement. Use the manual release link."
                        .to_string(),
                    required,
                });
            }
            Ok(update::CheckResult::UpToDate) => status.set(UpdateUiStatus::UpToDate),
            Ok(update::CheckResult::Available(update)) => {
                let version = update.version().to_string();
                available.set(Some(update));
                status.set(UpdateUiStatus::Available { version, required });
            }
            Err(error) => status.set(UpdateUiStatus::Failed {
                message: format!(
                    "Secure update check failed: {error}. Retry or use the manual release link."
                ),
                required,
            }),
        }
    });
}

fn next_update_generation(mut generation: Signal<u64>) -> u64 {
    let next = generation.read().wrapping_add(1);
    generation.set(next);
    next
}

pub(super) fn request_required_update(signals: UpdateSignals, reason: String) {
    check_for_updates(signals, Some(reason));
}

pub(super) fn install_available_update(signals: UpdateSignals) {
    let available_update = signals.available.read().clone();
    if *signals.pending.read() || available_update.is_none() {
        return;
    }

    let update = available_update.expect("checked above");
    let version = update.version().to_string();
    let required = signals.status.read().required_reason();
    let mut status = signals.status;
    let mut pending = signals.pending;
    pending.set(true);
    status.set(UpdateUiStatus::Downloading {
        version: version.clone(),
        required: required.clone(),
    });
    spawn(async move {
        match update::download_update(&update).await {
            Ok(staged) => {
                let install_required = required.clone();
                status.set(UpdateUiStatus::Installing {
                    version,
                    required: install_required.clone(),
                });
                match update::schedule_replace_and_restart(staged) {
                    Ok(()) => std::process::exit(0),
                    Err(error) => {
                        pending.set(false);
                        status.set(UpdateUiStatus::Failed {
                            message: format!(
                                "Update failed: {error}. Use the manual release link if retrying does not help."
                            ),
                            required: install_required,
                        });
                    }
                }
            }
            Err(error) => {
                pending.set(false);
                status.set(UpdateUiStatus::Failed {
                    message: format!(
                        "Update download failed: {error}. Use the manual release link if retrying does not help."
                    ),
                    required,
                });
            }
        }
    });
}

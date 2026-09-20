use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use url::Url;

#[cfg(windows)]
use serde::Serialize;
#[cfg(windows)]
use sha2::{Digest, Sha256};
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::{
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering as AtomicOrdering},
        OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[cfg(windows)]
use tokio::io::AsyncWriteExt;

mod compiled {
    include!(concat!(env!("OUT_DIR"), "/agora_update_config.rs"));
}

const MANIFEST_FILE_NAME: &str = "agora-update-manifest.json";
const MANIFEST_SIGNATURE_FILE_NAME: &str = "agora-update-manifest.json.sig";
const MANIFEST_SCHEMA_VERSION: u8 = 1;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_SIGNATURE_BYTES: usize = 256;
const MAX_UPDATE_BYTES: u64 = 512 * 1024 * 1024;

#[cfg(windows)]
const UPDATE_STATE_FILE_NAME: &str = ".agora-update-state.json";
#[cfg(windows)]
const UPDATE_STATE_SCHEMA_VERSION: u8 = 1;
#[cfg(windows)]
const HELPER_WAIT_TIMEOUT_MS: u32 = 120_000;
#[cfg(windows)]
const STARTUP_TICKET_PREFIX: &str = "startup-ticket:";
#[cfg(windows)]
const MAX_UPDATE_STATE_ERROR_LEN: usize = 300;
#[cfg(windows)]
const MAX_WINDOWS_FILE_NAME_LEN: usize = 255;

#[derive(Clone, Debug)]
pub(crate) enum CheckResult {
    Disabled,
    UpToDate,
    Available(AvailableUpdate),
}

#[derive(Clone, Debug)]
pub(crate) struct AvailableUpdate {
    version: String,
    filename: String,
    size: u64,
    digest: [u8; 32],
}

impl AvailableUpdate {
    pub(crate) fn version(&self) -> &str {
        &self.version
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    schema_version: u8,
    version: String,
    target: String,
    filename: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

pub(crate) fn is_configured() -> bool {
    compiled::UPDATE_ENABLED
}

pub(crate) fn manual_release_url() -> Option<String> {
    if !is_configured() {
        return None;
    }
    let mut url = Url::parse(compiled::UPDATE_BASE_URL).ok()?;
    if let Some(release_path) = url.path().trim_end_matches('/').strip_suffix("/download") {
        url.set_path(&format!("{release_path}/"));
    }
    Some(url.into())
}

pub(crate) async fn check_for_update() -> Result<CheckResult, String> {
    if !is_configured() {
        return Ok(CheckResult::Disabled);
    }

    let verifying_key = compiled_verifying_key()?;
    let client = update_http_client()?;
    let raw_manifest = fetch_limited(
        &client,
        &pinned_asset_url(MANIFEST_FILE_NAME)?,
        MAX_MANIFEST_BYTES,
    )
    .await?;
    let raw_signature = fetch_limited(
        &client,
        &pinned_asset_url(MANIFEST_SIGNATURE_FILE_NAME)?,
        MAX_SIGNATURE_BYTES,
    )
    .await?;
    let update = verify_and_parse_manifest(
        &raw_manifest,
        &raw_signature,
        &verifying_key,
        compiled::UPDATE_TARGET,
    )?;
    let current = parse_release_version(env!("CARGO_PKG_VERSION"))
        .map_err(|_| "This client has an invalid compiled version".to_string())?;
    let available = parse_release_version(&update.version)?;

    if available <= current {
        Ok(CheckResult::UpToDate)
    } else {
        Ok(CheckResult::Available(update))
    }
}

fn compiled_verifying_key() -> Result<VerifyingKey, String> {
    if !is_configured() {
        return Err("Automatic updates are not configured in this build".to_string());
    }
    let encoded = compiled::UPDATE_PUBLIC_KEY_B64;
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| "The compiled update public key is invalid".to_string())?;
    if decoded.len() != 32 || STANDARD.encode(&decoded) != encoded {
        return Err("The compiled update public key is invalid".to_string());
    }
    let bytes: [u8; 32] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| "The compiled update public key is invalid".to_string())?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| "The compiled update public key is invalid".to_string())
}

fn verify_and_parse_manifest(
    raw_manifest: &[u8],
    raw_signature: &[u8],
    verifying_key: &VerifyingKey,
    expected_target: &str,
) -> Result<AvailableUpdate, String> {
    let signature = decode_signature(raw_signature)?;
    // Authenticate the exact response bytes before JSON parsing can act on any manifest field.
    verifying_key
        .verify_strict(raw_manifest, &signature)
        .map_err(|_| "The update manifest signature is invalid".to_string())?;

    let manifest: RawManifest = serde_json::from_slice(raw_manifest)
        .map_err(|_| "The signed update manifest is not valid JSON".to_string())?;
    validate_manifest(manifest, expected_target)
}

fn decode_signature(raw_signature: &[u8]) -> Result<Signature, String> {
    let encoded = std::str::from_utf8(raw_signature)
        .map_err(|_| "The update manifest signature is not valid Base64".to_string())?;
    if encoded.len() != 88 {
        return Err("The update manifest signature has an invalid length".to_string());
    }
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| "The update manifest signature is not valid Base64".to_string())?;
    if decoded.len() != 64 || STANDARD.encode(&decoded) != encoded {
        return Err("The update manifest signature is not canonical Base64".to_string());
    }
    let bytes: [u8; 64] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| "The update manifest signature has an invalid length".to_string())?;
    Ok(Signature::from_bytes(&bytes))
}

fn validate_manifest(
    manifest: RawManifest,
    expected_target: &str,
) -> Result<AvailableUpdate, String> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err("The update manifest uses an unsupported schema version".to_string());
    }
    let _version = parse_release_version(&manifest.version)?;
    if !is_safe_target(&manifest.target) || manifest.target != expected_target {
        return Err("The update manifest target does not match this client".to_string());
    }
    if !is_safe_asset_name(&manifest.filename)
        || manifest.filename != expected_update_filename(&manifest.version)
    {
        return Err("The update manifest filename is invalid".to_string());
    }
    if manifest.size == 0 || manifest.size > MAX_UPDATE_BYTES {
        return Err("The update manifest size is outside the allowed range".to_string());
    }

    Ok(AvailableUpdate {
        version: manifest.version,
        filename: manifest.filename,
        size: manifest.size,
        digest: decode_sha256(&manifest.sha256)?,
    })
}

fn parse_release_version(value: &str) -> Result<ReleaseVersion, String> {
    if value.len() > 64 {
        return Err("The update manifest version is too long".to_string());
    }
    let mut parts = value.split('.');
    let major = parse_version_component(parts.next(), "major")?;
    let minor = parse_version_component(parts.next(), "minor")?;
    let patch = parse_version_component(parts.next(), "patch")?;
    if parts.next().is_some() {
        return Err("The update manifest version must be stable semantic versioning".to_string());
    }
    Ok(ReleaseVersion {
        major,
        minor,
        patch,
    })
}

fn parse_version_component(value: Option<&str>, name: &str) -> Result<u64, String> {
    let value = value.ok_or_else(|| {
        "The update manifest version must be stable semantic versioning".to_string()
    })?;
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!(
            "The update manifest {name} version component is invalid"
        ));
    }
    value
        .parse()
        .map_err(|_| format!("The update manifest {name} version component is invalid"))
}

fn expected_update_filename(version: &str) -> String {
    format!("agora-client-{version}-windows-x64.exe")
}

fn is_safe_target(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn is_safe_asset_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn decode_sha256(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("The update manifest SHA-256 is invalid".to_string());
    }

    let mut digest = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (hex_value(pair[0])? << 4) | hex_value(pair[1])?;
    }
    Ok(digest)
}

fn hex_value(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err("The update manifest SHA-256 is invalid".to_string()),
    }
}

fn pinned_asset_url(asset_name: &str) -> Result<Url, String> {
    if !is_safe_asset_name(asset_name) {
        return Err("The update asset name is invalid".to_string());
    }
    let base = Url::parse(compiled::UPDATE_BASE_URL)
        .map_err(|_| "The compiled update base URL is invalid".to_string())?;
    if base.scheme() != "https"
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || !base.path().ends_with('/')
    {
        return Err("The compiled update base URL is invalid".to_string());
    }

    let asset = base
        .join(asset_name)
        .map_err(|_| "The compiled update base URL is invalid".to_string())?;
    if asset.scheme() != "https"
        || asset.origin() != base.origin()
        || !asset.path().starts_with(base.path())
    {
        return Err("The compiled update base URL is invalid".to_string());
    }
    Ok(asset)
}

fn update_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        // Release CDNs such as GitHub Releases redirect to their HTTPS object host. The signed
        // manifest and signed digest still authenticate the payload, and the redirect count is
        // bounded to avoid treating the compiled base as an open-ended transport.
        .redirect(reqwest::redirect::Policy::limited(3))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| "Could not configure the update transport".to_string())
}

async fn fetch_limited(
    client: &reqwest::Client,
    url: &Url,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .send()
        .await
        .map_err(|_| "Could not download update metadata".to_string())?;
    validate_update_response(&response, max_bytes as u64, "metadata")?;

    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read update metadata".to_string())?
    {
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "Update metadata is too large".to_string())?;
        if next_len > max_bytes {
            return Err("Update metadata is too large".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn validate_update_response(
    response: &reqwest::Response,
    max_bytes: u64,
    resource: &str,
) -> Result<(), String> {
    if response.url().scheme() != "https" {
        return Err(format!("The update {resource} left HTTPS"));
    }
    if !response.status().is_success() {
        return Err(format!("The update {resource} download was rejected"));
    }
    if response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .is_some_and(|value| {
            value
                .to_str()
                .ok()
                .is_none_or(|value| !value.eq_ignore_ascii_case("identity"))
        })
    {
        return Err(format!(
            "The update {resource} used an unsupported content encoding"
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(format!("The update {resource} is too large"));
    }
    Ok(())
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub(crate) struct StagedUpdate {
    path: PathBuf,
    size: u64,
    digest: [u8; 32],
    version: String,
}

#[cfg(windows)]
pub(crate) async fn download_update(update: &AvailableUpdate) -> Result<StagedUpdate, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "Could not locate the running Agora executable".to_string())?;
    let directory = executable
        .parent()
        .ok_or_else(|| "The running Agora executable has no parent directory".to_string())?;
    let client = update_http_client()?;
    let url = pinned_asset_url(&update.filename)?;
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .send()
        .await
        .map_err(|_| "Could not download the update".to_string())?;
    validate_update_response(&response, update.size, "package")?;
    if response
        .content_length()
        .is_some_and(|length| length != update.size)
    {
        return Err("The update package size does not match its signed manifest".to_string());
    }

    let (path, mut file) = create_staging_file(directory).await?;
    let result = stream_update_to_file(&mut response, &mut file, update.size, &update.digest).await;
    drop(file);
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(error);
    }

    Ok(StagedUpdate {
        path,
        size: update.size,
        digest: update.digest,
        version: update.version.clone(),
    })
}

#[cfg(windows)]
async fn create_staging_file(directory: &Path) -> Result<(PathBuf, tokio::fs::File), String> {
    for _ in 0..16 {
        let path = directory.join(format!("agora-update-{}.pending.exe", unique_suffix()));
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("Could not create an update file beside Agora".to_string()),
        }
    }
    Err("Could not reserve an update file beside Agora".to_string())
}

#[cfg(windows)]
async fn stream_update_to_file(
    response: &mut reqwest::Response,
    file: &mut tokio::fs::File,
    expected_size: u64,
    expected_digest: &[u8; 32],
) -> Result<(), String> {
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the update package".to_string())?
    {
        let chunk_size = u64::try_from(chunk.len())
            .map_err(|_| "The update package is too large".to_string())?;
        received = received
            .checked_add(chunk_size)
            .ok_or_else(|| "The update package is too large".to_string())?;
        if received > expected_size {
            return Err("The update package exceeds its signed size".to_string());
        }
        file.write_all(&chunk)
            .await
            .map_err(|_| "Could not write the update package".to_string())?;
        hasher.update(&chunk);
    }
    file.flush()
        .await
        .map_err(|_| "Could not flush the update package".to_string())?;
    file.sync_all()
        .await
        .map_err(|_| "Could not persist the update package".to_string())?;

    if received != expected_size {
        return Err("The update package size does not match its signed manifest".to_string());
    }
    let digest: [u8; 32] = hasher.finalize().into();
    if &digest != expected_digest {
        return Err("The update package SHA-256 does not match its signed manifest".to_string());
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn schedule_replace_and_restart(staged: StagedUpdate) -> Result<(), String> {
    let (target, directory, target_name) = current_executable_details()?;
    if !same_directory(&staged.path, &directory)
        || !staged
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_staging_file_name)
    {
        return Err("The staged update is not in Agora's installation directory".to_string());
    }
    verify_file(&staged.path, staged.size, &staged.digest)?;

    let suffix = unique_suffix();
    let helper_name = update_work_file_name(&target_name, "helper", &suffix);
    let backup_name = update_work_file_name(&target_name, "backup", &suffix);
    let helper = directory.join(&helper_name);
    let state_path = directory.join(UPDATE_STATE_FILE_NAME);
    if state_path.exists() {
        return Err("A previous update needs recovery before another update can start".to_string());
    }

    copy_current_executable(&target, &helper)?;
    let state = UpdateState {
        schema_version: UPDATE_STATE_SCHEMA_VERSION,
        phase: UpdatePhase::Staged,
        target: target_name,
        staged: staged
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "The staged update filename is invalid".to_string())?
            .to_string(),
        backup: backup_name.clone(),
        helper: helper_name,
        version: staged.version,
        // Keep the ticket in an existing optional field so older helpers can still parse the
        // recovery record during a rolling client upgrade.
        error: Some(format!("{STARTUP_TICKET_PREFIX}{backup_name}")),
    };
    if let Err(error) = write_new_state(&state_path, &state) {
        let _ = fs::remove_file(&helper);
        return Err(error);
    }

    if Command::new(&helper)
        .arg("--agora-update-helper")
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .current_dir(&directory)
        .spawn()
        .is_err()
    {
        // The original client is still running, so remove this unstarted attempt and let the
        // user retry instead of leaving a recovery record that blocks every later install.
        let state_cleanup = remove_file_if_exists(&state_path);
        let _ = remove_file_if_exists(&helper);
        let _ = remove_file_if_exists(&directory.join(&state.staged));
        if state_cleanup.is_err() {
            return Err(
                "Could not start the update helper or clear its recovery record; restart Agora before retrying"
                    .to_string(),
            );
        }
        return Err(
            "Could not start the update helper; retry the update or use the manual release link"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(windows)]
fn verify_file(path: &Path, expected_size: u64, expected_digest: &[u8; 32]) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|_| "The staged update is no longer available".to_string())?;
    if !metadata.is_file() || metadata.len() != expected_size {
        return Err("The staged update size changed before installation".to_string());
    }
    let mut file =
        fs::File::open(path).map_err(|_| "Could not verify the staged update".to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "Could not verify the staged update".to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest: [u8; 32] = hasher.finalize().into();
    if &digest != expected_digest {
        return Err("The staged update SHA-256 changed before installation".to_string());
    }
    Ok(())
}

#[cfg(windows)]
static UPDATE_NONCE: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
fn unique_suffix() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = UPDATE_NONCE.fetch_add(1, AtomicOrdering::Relaxed);
    format!("{:x}-{:x}-{:x}", std::process::id(), time, counter)
}

#[cfg(windows)]
fn update_work_file_name(target_name: &str, kind: &str, suffix: &str) -> String {
    let legacy = format!("{target_name}.agora-{kind}-{suffix}.exe");
    if is_safe_local_file_name(&legacy) {
        legacy
    } else {
        format!("agora-{kind}-{suffix}.exe")
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum UpdatePhase {
    Staged,
    Replacing,
    Replaced,
    Failed,
    RestartFailed,
}

#[cfg(windows)]
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UpdateState {
    schema_version: u8,
    phase: UpdatePhase,
    target: String,
    staged: String,
    backup: String,
    helper: String,
    version: String,
    error: Option<String>,
}

#[cfg(windows)]
struct StatePaths {
    target: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    helper: PathBuf,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryExecutable {
    Target,
    Backup,
    Helper,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryAction {
    CleanUp,
    AwaitStartupAcknowledgment,
    RestoreWithCurrentHelper,
    HandoffToHelper,
    HandoffToTarget,
}

#[cfg(windows)]
enum RecoveryOutcome {
    Continue(Option<String>),
    Handoff,
}

#[cfg(windows)]
static STARTUP_UPDATE_NOTICE: OnceLock<Option<String>> = OnceLock::new();

#[cfg(windows)]
pub(crate) fn run_helper_if_requested() -> bool {
    let arguments = std::env::args_os().collect::<Vec<OsString>>();
    let parent_pid = match arguments.as_slice() {
        [_, _, parent_flag, parent_pid] if parent_flag == "--parent-pid" => parent_pid
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value != 0),
        _ => None,
    };
    match arguments.get(1).and_then(|value| value.to_str()) {
        Some("--agora-update-helper") => {
            if let Some(parent_pid) = parent_pid {
                let _ = run_update_helper(parent_pid);
            }
            true
        }
        Some("--agora-update-recovery-helper") => {
            if let Some(parent_pid) = parent_pid {
                let _ = run_recovery_helper(parent_pid);
            }
            true
        }
        _ => false,
    }
}

#[cfg(windows)]
pub(crate) fn recover_interrupted_update() -> bool {
    let (notice, handoff) = match recover_interrupted_update_inner() {
        Ok(RecoveryOutcome::Continue(notice)) => (notice, false),
        Ok(RecoveryOutcome::Handoff) => (None, true),
        Err(_) => (
            Some(
                "An update recovery record could not be processed. Use the manual release link."
                    .to_string(),
            ),
            false,
        ),
    };
    let _ = STARTUP_UPDATE_NOTICE.set(notice);
    handoff
}

#[cfg(windows)]
pub(crate) fn startup_notice() -> Option<String> {
    STARTUP_UPDATE_NOTICE.get().cloned().flatten()
}

#[cfg(windows)]
pub(crate) fn acknowledge_update_after_ui_startup() -> Result<(), String> {
    let Some(startup_ticket) = update_startup_ticket() else {
        return Ok(());
    };
    let (current, directory, current_name) = current_executable_details()?;
    let state_path = directory.join(UPDATE_STATE_FILE_NAME);
    if !state_path.exists() {
        return Ok(());
    }
    let state = read_state(&state_path)?;
    let paths = state_paths(&directory, &state)?;
    if !matches!(state.phase, UpdatePhase::Replaced)
        || state_startup_ticket(&state) != Some(startup_ticket.as_str())
        || recovery_executable(&state, &current_name) != Some(RecoveryExecutable::Target)
        || current != paths.target
        || !paths.backup.exists()
    {
        return Ok(());
    }

    // Removing the backup is the durable commit point. Any later cleanup failure is safe to
    // retry because recovery sees the replacement without a rollback candidate.
    remove_file_if_exists(&paths.backup)?;
    clean_up_recovery_files(&paths, &state_path, RecoveryExecutable::Target)
}

#[cfg(windows)]
fn recover_interrupted_update_inner() -> Result<RecoveryOutcome, String> {
    let (current, directory, current_name) = current_executable_details()?;
    let state_path = directory.join(UPDATE_STATE_FILE_NAME);
    if !state_path.exists() {
        return Ok(RecoveryOutcome::Continue(None));
    }
    let state = read_state(&state_path)?;
    let executable = recovery_executable(&state, &current_name)
        .ok_or_else(|| "The update recovery record does not match this executable".to_string())?;
    let paths = state_paths(&directory, &state)?;
    let startup_ticket_matches = state_startup_ticket(&state)
        .map(|ticket| update_startup_ticket().as_deref() == Some(ticket));
    match recovery_action(
        state.phase,
        executable,
        paths.target.exists(),
        paths.backup.exists(),
        startup_ticket_matches,
    )? {
        RecoveryAction::CleanUp => {
            clean_up_recovery_files(&paths, &state_path, executable)?;
            let notice = match state.phase {
                UpdatePhase::Staged => Some(
                    "A pending update was not installed. You can retry or use the manual release link."
                        .to_string(),
                ),
                UpdatePhase::Failed => Some(
                    "A previous update could not be installed. The existing version was restarted."
                        .to_string(),
                ),
                UpdatePhase::RestartFailed => Some(
                    "Agora restored and restarted the previous executable after the update could not start."
                        .to_string(),
                ),
                UpdatePhase::Replacing => {
                    Some("Agora completed recovery from a previous update.".to_string())
                }
                UpdatePhase::Replaced => None,
            };
            Ok(RecoveryOutcome::Continue(notice))
        }
        RecoveryAction::AwaitStartupAcknowledgment => Ok(RecoveryOutcome::Continue(Some(
            "A signed update is starting; keeping the previous version until Agora is ready."
                .to_string(),
        ))),
        RecoveryAction::RestoreWithCurrentHelper => {
            let mut state = state;
            restore_backup_and_restart(&state_path, &mut state, &paths, &directory)?;
            Ok(RecoveryOutcome::Handoff)
        }
        RecoveryAction::HandoffToHelper => {
            launch_recovery_helper(&current, &paths.helper, &directory)?;
            Ok(RecoveryOutcome::Handoff)
        }
        RecoveryAction::HandoffToTarget => {
            launch_target(&paths.target, &directory)?;
            Ok(RecoveryOutcome::Handoff)
        }
    }
}

#[cfg(windows)]
fn recovery_executable(state: &UpdateState, name: &str) -> Option<RecoveryExecutable> {
    if name == state.target {
        Some(RecoveryExecutable::Target)
    } else if name == state.backup {
        Some(RecoveryExecutable::Backup)
    } else if name == state.helper {
        Some(RecoveryExecutable::Helper)
    } else {
        None
    }
}

#[cfg(windows)]
fn recovery_action(
    phase: UpdatePhase,
    executable: RecoveryExecutable,
    target_exists: bool,
    backup_exists: bool,
    startup_ticket_matches: Option<bool>,
) -> Result<RecoveryAction, String> {
    match phase {
        UpdatePhase::Staged | UpdatePhase::Failed => {
            if backup_exists {
                return Err("The update recovery record has an unexpected backup".to_string());
            }
            if !target_exists {
                return Err("The update left no executable to restart".to_string());
            }
            Ok(if executable == RecoveryExecutable::Target {
                RecoveryAction::CleanUp
            } else {
                RecoveryAction::HandoffToTarget
            })
        }
        UpdatePhase::Replaced => {
            if backup_exists {
                return Ok(if executable == RecoveryExecutable::Helper {
                    RecoveryAction::RestoreWithCurrentHelper
                } else if executable == RecoveryExecutable::Target {
                    match startup_ticket_matches {
                        Some(true) => RecoveryAction::AwaitStartupAcknowledgment,
                        Some(false) => RecoveryAction::HandoffToHelper,
                        // Updates launched by an older helper have no ticket marker and
                        // retain their historical cleanup behavior for a seamless transition.
                        None => RecoveryAction::CleanUp,
                    }
                } else {
                    RecoveryAction::HandoffToHelper
                });
            }
            if !target_exists {
                return Err("The update left no executable or backup".to_string());
            }
            Ok(if executable == RecoveryExecutable::Target {
                RecoveryAction::CleanUp
            } else {
                RecoveryAction::HandoffToTarget
            })
        }
        UpdatePhase::Replacing | UpdatePhase::RestartFailed => {
            // A failed restart is never a successful update merely because the replacement
            // file exists. Prefer the known-good backup while it is still available.
            if backup_exists
                && (matches!(phase, UpdatePhase::Replacing | UpdatePhase::RestartFailed)
                    || !target_exists)
            {
                return Ok(if executable == RecoveryExecutable::Helper {
                    RecoveryAction::RestoreWithCurrentHelper
                } else {
                    RecoveryAction::HandoffToHelper
                });
            }
            if !target_exists {
                return Err("The update left no executable or backup".to_string());
            }
            Ok(if executable == RecoveryExecutable::Target {
                RecoveryAction::CleanUp
            } else {
                RecoveryAction::HandoffToTarget
            })
        }
    }
}

#[cfg(windows)]
fn clean_up_recovery_files(
    paths: &StatePaths,
    state_path: &Path,
    executable: RecoveryExecutable,
) -> Result<(), String> {
    remove_file_if_exists(&paths.staged)?;
    remove_file_if_exists(&paths.backup)?;
    if executable != RecoveryExecutable::Helper {
        remove_file_with_retry(&paths.helper)?;
    }
    remove_file_if_exists(state_path)
}

#[cfg(windows)]
fn run_update_helper(parent_pid: u32) -> Result<(), String> {
    let (_helper, directory, helper_name) = current_executable_details()?;
    let state_path = directory.join(UPDATE_STATE_FILE_NAME);
    let mut state = read_state(&state_path)?;
    let paths = state_paths(&directory, &state)?;
    if state.helper != helper_name || !matches!(state.phase, UpdatePhase::Staged) {
        return Err("The update helper state is invalid".to_string());
    }

    if let Err(error) = wait_for_parent(parent_pid) {
        mark_state_failed(&state_path, &mut state, "parent wait failed");
        return Err(error);
    }

    state.phase = UpdatePhase::Replacing;
    if let Err(error) = write_state(&state_path, &state) {
        mark_state_failed(&state_path, &mut state, "state write failed");
        let _ = launch_target(&paths.target, &directory);
        return Err(error);
    }
    if let Err(error) = replace_file(&paths.target, &paths.staged, &paths.backup) {
        mark_state_failed(&state_path, &mut state, "replacement failed");
        let _ = launch_target(&paths.target, &directory);
        return Err(error);
    }

    state.phase = UpdatePhase::Replaced;
    if let Err(error) = write_state(&state_path, &state) {
        // Without a durable `replaced` record, recovery must favor the known-good executable.
        state.phase = UpdatePhase::RestartFailed;
        state.error = Some("state write failed after replacement".to_string());
        let _ = write_state(&state_path, &state);
        if let Err(recovery_error) =
            restore_backup_and_restart(&state_path, &mut state, &paths, &directory)
        {
            let fallback = if paths.backup.exists() {
                &paths.backup
            } else {
                &paths.target
            };
            let _ = launch_target(fallback, &directory);
            return Err(format!(
                "{error}; could not restore the previous version: {recovery_error}"
            ));
        }
        return Err(error);
    }
    if let Err(error) = launch_updated_target(&paths.target, &directory, &state.backup) {
        state.phase = UpdatePhase::RestartFailed;
        state.error = Some("restart failed".to_string());
        let _ = write_state(&state_path, &state);
        if let Err(recovery_error) =
            restore_backup_and_restart(&state_path, &mut state, &paths, &directory)
        {
            // If replacement cannot be completed immediately, start the backup by its own
            // name. Its startup recovery path will hand work back to the helper safely.
            let fallback = if paths.backup.exists() {
                &paths.backup
            } else {
                &paths.target
            };
            let _ = launch_target(fallback, &directory);
            return Err(format!(
                "{error}; could not restore the previous version: {recovery_error}"
            ));
        }
        return Ok(());
    }
    Ok(())
}

#[cfg(windows)]
fn run_recovery_helper(parent_pid: u32) -> Result<(), String> {
    let (_helper, directory, helper_name) = current_executable_details()?;
    let state_path = directory.join(UPDATE_STATE_FILE_NAME);
    let mut state = read_state(&state_path)?;
    let paths = state_paths(&directory, &state)?;
    if state.helper != helper_name
        || !matches!(
            state.phase,
            UpdatePhase::Replacing | UpdatePhase::Replaced | UpdatePhase::RestartFailed
        )
    {
        return Err("The update recovery helper state is invalid".to_string());
    }
    wait_for_parent(parent_pid)?;
    restore_backup_and_restart(&state_path, &mut state, &paths, &directory)
}

#[cfg(windows)]
fn restore_backup_and_restart(
    state_path: &Path,
    state: &mut UpdateState,
    paths: &StatePaths,
    directory: &Path,
) -> Result<(), String> {
    if !paths.backup.exists() {
        return Err("The update left no backup to restore".to_string());
    }
    state.phase = UpdatePhase::RestartFailed;
    state.error = Some("restart failed; restoring backup".to_string());
    // The old state remains recoverable if this write fails, so favor restoring the executable.
    let _ = write_state(state_path, state);
    move_file(&paths.backup, &paths.target, true)?;
    launch_target(&paths.target, directory)
}

#[cfg(windows)]
fn launch_recovery_helper(current: &Path, helper: &Path, directory: &Path) -> Result<(), String> {
    if !helper.exists() {
        copy_current_executable(current, helper)?;
    }
    Command::new(helper)
        .arg("--agora-update-recovery-helper")
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .current_dir(directory)
        .spawn()
        .map(|_| ())
        .map_err(|_| "Could not start the update recovery helper".to_string())
}

#[cfg(windows)]
fn mark_state_failed(state_path: &Path, state: &mut UpdateState, error: &str) {
    state.phase = UpdatePhase::Failed;
    state.error = Some(error.to_string());
    let _ = write_state(state_path, state);
}

#[cfg(windows)]
fn launch_target(target: &Path, directory: &Path) -> Result<(), String> {
    Command::new(target)
        .current_dir(directory)
        .spawn()
        .map(|_| ())
        .map_err(|_| "Could not restart Agora after updating".to_string())
}

#[cfg(windows)]
fn launch_updated_target(
    target: &Path,
    directory: &Path,
    startup_ticket: &str,
) -> Result<(), String> {
    Command::new(target)
        .arg("--agora-update-startup")
        .arg(startup_ticket)
        .current_dir(directory)
        .spawn()
        .map(|_| ())
        .map_err(|_| "Could not restart Agora after updating".to_string())
}

#[cfg(windows)]
fn update_startup_ticket() -> Option<String> {
    let arguments = std::env::args_os().collect::<Vec<OsString>>();
    match arguments.as_slice() {
        [_, flag, ticket] if flag == "--agora-update-startup" => ticket
            .to_str()
            .filter(|ticket| is_safe_local_file_name(ticket))
            .map(ToString::to_string),
        _ => None,
    }
}

#[cfg(windows)]
fn state_startup_ticket(state: &UpdateState) -> Option<&str> {
    let ticket = state
        .error
        .as_deref()?
        .strip_prefix(STARTUP_TICKET_PREFIX)?;
    (ticket == state.backup.as_str() && is_safe_legacy_work_file_name(ticket)).then_some(ticket)
}

#[cfg(windows)]
fn wait_for_parent(parent_pid: u32) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, parent_pid) };
    if process.is_null() {
        // ERROR_INVALID_PARAMETER means the parent has already exited.
        return if unsafe { GetLastError() } == 87 {
            Ok(())
        } else {
            Err("Could not wait for Agora to exit".to_string())
        };
    }
    let result = unsafe { WaitForSingleObject(process, HELPER_WAIT_TIMEOUT_MS) };
    unsafe {
        CloseHandle(process);
    }
    match result {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err("Timed out waiting for Agora to exit".to_string()),
        _ => Err("Could not wait for Agora to exit".to_string()),
    }
}

#[cfg(windows)]
fn current_executable_details() -> Result<(PathBuf, PathBuf, String), String> {
    let executable = std::env::current_exe()
        .map_err(|_| "Could not locate the running Agora executable".to_string())?;
    let directory = executable
        .parent()
        .ok_or_else(|| "The running Agora executable has no parent directory".to_string())?
        .to_path_buf();
    let name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| is_safe_local_file_name(name) && is_executable_file_name(name))
        .ok_or_else(|| "The running Agora executable filename is invalid".to_string())?
        .to_string();
    Ok((executable, directory, name))
}

#[cfg(windows)]
fn state_paths(directory: &Path, state: &UpdateState) -> Result<StatePaths, String> {
    if state.schema_version != UPDATE_STATE_SCHEMA_VERSION
        || parse_release_version(&state.version).is_err()
        || state
            .error
            .as_ref()
            .is_some_and(|value| value.len() > MAX_UPDATE_STATE_ERROR_LEN)
        || !is_safe_local_file_name(&state.target)
        || !is_executable_file_name(&state.target)
        || !is_staging_file_name(&state.staged)
        || !is_update_work_file_name(&state.backup, &state.target, "backup")
        || !is_update_work_file_name(&state.helper, &state.target, "helper")
    {
        return Err("The update recovery record is invalid".to_string());
    }
    Ok(StatePaths {
        target: directory.join(&state.target),
        staged: directory.join(&state.staged),
        backup: directory.join(&state.backup),
        helper: directory.join(&state.helper),
    })
}

#[cfg(windows)]
fn is_staging_file_name(value: &str) -> bool {
    value.starts_with("agora-update-")
        && value.ends_with(".pending.exe")
        && is_safe_local_file_name(value)
}

#[cfg(windows)]
fn is_update_work_file_name(value: &str, target_name: &str, kind: &str) -> bool {
    let legacy_prefix = format!("{target_name}.agora-{kind}-");
    let short_prefix = format!("agora-{kind}-");
    if let Some(suffix) = value.strip_prefix(&legacy_prefix) {
        return suffix.len() > 4
            && is_executable_file_name(suffix)
            && is_safe_legacy_work_file_name(value);
    }
    value.strip_prefix(&short_prefix).is_some_and(|suffix| {
        suffix.len() > 4 && is_executable_file_name(suffix) && is_safe_local_file_name(value)
    })
}

#[cfg(windows)]
fn is_executable_file_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".exe")
}

#[cfg(windows)]
fn is_safe_local_file_name(value: &str) -> bool {
    is_safe_file_name(value, 240)
}

#[cfg(windows)]
fn is_safe_legacy_work_file_name(value: &str) -> bool {
    is_safe_file_name(value, MAX_WINDOWS_FILE_NAME_LEN)
}

#[cfg(windows)]
fn is_safe_file_name(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(windows)]
fn same_directory(path: &Path, directory: &Path) -> bool {
    match (path.parent(), fs::canonicalize(directory)) {
        (Some(parent), Ok(directory)) => {
            fs::canonicalize(parent).is_ok_and(|parent| parent == directory)
        }
        _ => false,
    }
}

#[cfg(windows)]
fn copy_current_executable(source: &Path, destination: &Path) -> Result<(), String> {
    let mut source = fs::File::open(source)
        .map_err(|_| "Could not read the running Agora executable".to_string())?;
    let mut destination = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| "Could not create the update helper".to_string())?;
    std::io::copy(&mut source, &mut destination)
        .map_err(|_| "Could not copy the update helper".to_string())?;
    destination
        .sync_all()
        .map_err(|_| "Could not persist the update helper".to_string())
}

#[cfg(windows)]
fn read_state(path: &Path) -> Result<UpdateState, String> {
    let bytes =
        fs::read(path).map_err(|_| "Could not read the update recovery record".to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| "The update recovery record is invalid".to_string())
}

#[cfg(windows)]
fn write_new_state(path: &Path, state: &UpdateState) -> Result<(), String> {
    write_state_inner(path, state, false)
}

#[cfg(windows)]
fn write_state(path: &Path, state: &UpdateState) -> Result<(), String> {
    write_state_inner(path, state, true)
}

#[cfg(windows)]
fn write_state_inner(path: &Path, state: &UpdateState, replace: bool) -> Result<(), String> {
    let bytes = serde_json::to_vec(state)
        .map_err(|_| "Could not serialize the update recovery record".to_string())?;
    let directory = path
        .parent()
        .ok_or_else(|| "The update recovery path is invalid".to_string())?;
    let temporary = directory.join(format!(".agora-update-state-{}.tmp", unique_suffix()));
    let result = (|| -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "Could not create the update recovery record".to_string())?;
        file.write_all(&bytes)
            .map_err(|_| "Could not write the update recovery record".to_string())?;
        file.sync_all()
            .map_err(|_| "Could not persist the update recovery record".to_string())?;
        drop(file);
        move_file(&temporary, path, replace)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn replace_file(target: &Path, staged: &Path, backup: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Storage::FileSystem::{ReplaceFileW, REPLACEFILE_WRITE_THROUGH},
    };

    if backup.exists() {
        return Err("An update backup already exists".to_string());
    }
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let staged = staged
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let backup = backup
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe {
        ReplaceFileW(
            target.as_ptr(),
            staged.as_ptr(),
            backup.as_ptr(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    } == 0
    {
        return Err(format!(
            "Could not atomically replace Agora (Windows error {})",
            unsafe { GetLastError() }
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn move_file(source: &Path, destination: &Path, replace: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH},
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) } == 0 {
        return Err(format!(
            "Could not update the recovery record (Windows error {})",
            unsafe { GetLastError() }
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not clean up an update recovery file".to_string()),
    }
}

#[cfg(windows)]
fn remove_file_with_retry(path: &Path) -> Result<(), String> {
    for _ in 0..20 {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    Err("Could not clean up the update helper yet".to_string())
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    const TARGET: &str = "x86_64-pc-windows-msvc";

    fn signing_key() -> SigningKey {
        let mut seed = [0u8; 32];
        seed[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        seed[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        SigningKey::from_bytes(&seed)
    }

    fn signed_manifest(manifest: &str) -> (Vec<u8>, Vec<u8>, VerifyingKey) {
        let signing_key = signing_key();
        let raw_manifest = manifest.as_bytes().to_vec();
        let signature = STANDARD
            .encode(signing_key.sign(&raw_manifest).to_bytes())
            .into_bytes();
        (raw_manifest, signature, signing_key.verifying_key())
    }

    fn manifest(version: &str) -> String {
        format!(
            r#"{{"schema_version":1,"version":"{version}","target":"{TARGET}","filename":"agora-client-{version}-windows-x64.exe","size":42,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#
        )
    }

    #[test]
    fn verifies_raw_manifest_before_parsing() {
        let (raw_manifest, signature, key) = signed_manifest("this is not json");
        let error = verify_and_parse_manifest(&raw_manifest, &signature, &key, TARGET).unwrap_err();
        assert_eq!(error, "The signed update manifest is not valid JSON");

        let error = verify_and_parse_manifest(b"not json", &signature, &key, TARGET).unwrap_err();
        assert_eq!(error, "The update manifest signature is invalid");
    }

    #[test]
    fn accepts_only_a_matching_strict_manifest() {
        let (raw_manifest, signature, key) = signed_manifest(&manifest("1.2.3"));
        let update = verify_and_parse_manifest(&raw_manifest, &signature, &key, TARGET).unwrap();

        assert_eq!(update.version(), "1.2.3");
        assert_eq!(update.filename, "agora-client-1.2.3-windows-x64.exe");
        assert_eq!(update.size, 42);
    }

    #[test]
    fn rejects_unknown_or_unsafe_manifest_fields() {
        let sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let cases = [
            manifest("1.2.3").replace("\"schema_version\":1", "\"schema_version\":2"),
            manifest("01.2.3"),
            manifest("1.2.3-beta"),
            manifest("1.2.3").replace(TARGET, "x86_64-pc-windows-gnu"),
            manifest("1.2.3").replace(
                "agora-client-1.2.3-windows-x64.exe",
                "../../agora-client.exe",
            ),
            manifest("1.2.3").replace("\"size\":42", "\"size\":0"),
            manifest("1.2.3").replace("\"size\":42", "\"size\":536870913"),
            manifest("1.2.3").replace(sha256, &sha256.to_ascii_uppercase()),
            manifest("1.2.3").replace('}', ",\"extra\":true}"),
        ];

        for case in cases {
            let (raw_manifest, signature, key) = signed_manifest(&case);
            assert!(verify_and_parse_manifest(&raw_manifest, &signature, &key, TARGET).is_err());
        }
    }

    #[test]
    fn requires_canonical_detached_signatures() {
        let (raw_manifest, mut signature, key) = signed_manifest(&manifest("1.2.3"));
        signature.push(b'\n');

        assert!(verify_and_parse_manifest(&raw_manifest, &signature, &key, TARGET).is_err());
    }

    #[test]
    fn compares_stable_versions_numerically() {
        assert_eq!(
            parse_release_version("1.10.0")
                .unwrap()
                .cmp(&parse_release_version("1.9.9").unwrap()),
            Ordering::Greater
        );
        assert!(parse_release_version("1.0").is_err());
        assert!(parse_release_version("1.0.0+build").is_err());
    }

    #[cfg(windows)]
    fn update_state(phase: UpdatePhase) -> UpdateState {
        UpdateState {
            schema_version: UPDATE_STATE_SCHEMA_VERSION,
            phase,
            target: "agora-client.exe".to_string(),
            staged: "agora-update-test.pending.exe".to_string(),
            backup: "agora-client.exe.agora-backup-test.exe".to_string(),
            helper: "agora-client.exe.agora-helper-test.exe".to_string(),
            version: "1.2.3".to_string(),
            error: None,
        }
    }

    #[cfg(windows)]
    #[test]
    fn restart_failure_restores_the_backup_instead_of_cleaning_it_up() {
        assert_eq!(
            recovery_action(
                UpdatePhase::RestartFailed,
                RecoveryExecutable::Helper,
                true,
                true,
                None,
            ),
            Ok(RecoveryAction::RestoreWithCurrentHelper)
        );
        assert_eq!(
            recovery_action(
                UpdatePhase::RestartFailed,
                RecoveryExecutable::Target,
                true,
                true,
                None,
            ),
            Ok(RecoveryAction::HandoffToHelper)
        );
    }

    #[cfg(windows)]
    #[test]
    fn recovery_recognizes_target_backup_and_helper_executable_names() {
        let state = update_state(UpdatePhase::RestartFailed);

        assert_eq!(
            recovery_executable(&state, &state.target),
            Some(RecoveryExecutable::Target)
        );
        assert_eq!(
            recovery_executable(&state, &state.backup),
            Some(RecoveryExecutable::Backup)
        );
        assert_eq!(
            recovery_executable(&state, &state.helper),
            Some(RecoveryExecutable::Helper)
        );
        assert_eq!(recovery_executable(&state, "other.exe"), None);
    }

    #[cfg(windows)]
    #[test]
    fn long_or_uppercase_executable_names_use_valid_recovery_paths() {
        let target = format!("{}.EXE", "a".repeat(236));
        let helper = update_work_file_name(&target, "helper", "test");
        let backup = update_work_file_name(&target, "backup", "test");
        let mut state = update_state(UpdatePhase::Replaced);
        state.target = target.clone();
        state.helper = helper.clone();
        state.backup = backup.clone();
        state.error = Some(format!("{STARTUP_TICKET_PREFIX}{backup}"));

        assert!(is_executable_file_name(&target));
        assert!(helper.starts_with("agora-helper-"));
        assert!(is_update_work_file_name(&helper, &target, "helper"));
        assert!(is_update_work_file_name(&state.backup, &target, "backup"));
        assert_eq!(state_startup_ticket(&state), Some(state.backup.as_str()));
        assert!(state_paths(std::path::Path::new("C:\\updates"), &state).is_ok());

        let legacy_target = format!("{}.exe", "b".repeat(180));
        let legacy_suffix = "x".repeat(50);
        let mut legacy_state = update_state(UpdatePhase::RestartFailed);
        legacy_state.target = legacy_target.clone();
        legacy_state.helper = format!("{legacy_target}.agora-helper-{legacy_suffix}.exe");
        legacy_state.backup = format!("{legacy_target}.agora-backup-{legacy_suffix}.exe");
        assert!(legacy_state.helper.len() > 240);
        assert!(legacy_state.helper.len() <= MAX_WINDOWS_FILE_NAME_LEN);
        assert!(state_paths(std::path::Path::new("C:\\updates"), &legacy_state).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn interrupted_replacement_hands_backup_launches_to_the_helper() {
        assert_eq!(
            recovery_action(
                UpdatePhase::Replacing,
                RecoveryExecutable::Backup,
                false,
                true,
                None,
            ),
            Ok(RecoveryAction::HandoffToHelper)
        );
        assert_eq!(
            recovery_action(
                UpdatePhase::Replacing,
                RecoveryExecutable::Target,
                true,
                true,
                None,
            ),
            Ok(RecoveryAction::HandoffToHelper)
        );
        assert_eq!(
            recovery_action(
                UpdatePhase::RestartFailed,
                RecoveryExecutable::Target,
                true,
                false,
                None,
            ),
            Ok(RecoveryAction::CleanUp)
        );
    }

    #[cfg(windows)]
    #[test]
    fn replaced_update_requires_the_helper_startup_ticket() {
        assert_eq!(
            recovery_action(
                UpdatePhase::Replaced,
                RecoveryExecutable::Target,
                true,
                true,
                Some(true),
            ),
            Ok(RecoveryAction::AwaitStartupAcknowledgment)
        );
        assert_eq!(
            recovery_action(
                UpdatePhase::Replaced,
                RecoveryExecutable::Target,
                true,
                true,
                Some(false),
            ),
            Ok(RecoveryAction::HandoffToHelper)
        );
        assert_eq!(
            recovery_action(
                UpdatePhase::Replaced,
                RecoveryExecutable::Helper,
                true,
                true,
                Some(true),
            ),
            Ok(RecoveryAction::RestoreWithCurrentHelper)
        );
        assert_eq!(
            recovery_action(
                UpdatePhase::Replaced,
                RecoveryExecutable::Target,
                true,
                true,
                None,
            ),
            Ok(RecoveryAction::CleanUp)
        );
    }
}

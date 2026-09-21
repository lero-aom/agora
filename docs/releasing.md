# Releasing Agora

Release and image-publish scripts only operate from a clean Git worktree, including no untracked files. This ties the build, source archive, image labels, and release notes to one reviewed commit.

## Signed Windows Packages

Run the complete release check from a Windows environment with a PostgreSQL test database configured in `DATABASE_URL`. Load `AGORA_UPDATE_SIGNING_KEY_B64` into that process from a secret store before running the script. It is a Base64-encoded 32-byte Ed25519 seed, is never accepted as a command-line argument, and must not be echoed or logged.

### First Signing Seed

Before the first updater-capable release, create one 32-byte seed in a private session and immediately save it in a password manager, hardware-backed vault, or CI secret store. Never put it in Git, `.env`, the VPS, GitHub release assets, screenshots, chat logs, or shell history.

```powershell
$seedBytes = [byte[]]::new(32)
[System.Security.Cryptography.RandomNumberGenerator]::Fill($seedBytes)
$updateSigningSeed = [Convert]::ToBase64String($seedBytes)
[Array]::Clear($seedBytes, 0, $seedBytes.Length)
$updateSigningSeed
```

Do not create a replacement after publishing an updater-capable release. Earlier clients trust the public key derived from this seed and reject manifests signed by a replacement.

### Package a Release

Replace `https://your-reviewed-public-origin.example` below with the exact public HTTPS origin to embed in the client before running the command.

```powershell
$env:AGORA_UPDATE_SIGNING_KEY_B64 = Get-Secret -Name 'Agora update signing seed' -AsPlainText
pwsh -NoProfile -File scripts/release.ps1 -ServerUrl https://your-reviewed-public-origin.example
Remove-Item Env:\AGORA_UPDATE_SIGNING_KEY_B64 -ErrorAction SilentlyContinue
```

Use the same signing seed for every updater-capable public release. Replacing it after clients are distributed makes those clients reject future update manifests.

The script fixes `SOURCE_DATE_EPOCH` to the commit timestamp, uses `Cargo.lock`, checks formatting, Clippy, unit tests, PostgreSQL feature and invariant tests, and release builds. It derives the update public key, embeds it with the HTTPS update base into the Windows client, and creates these artifacts under `dist`:

- `agora-<version>-windows-x64.zip` and its SHA-256 sidecar.
- `agora-<version>-windows-x64-source.zip` and its SHA-256 sidecar.
- `agora-client-<version>-windows-x64.exe` and its SHA-256 sidecar.
- `agora-update-manifest.json` and `agora-update-manifest.json.sig`.

`ServerUrl` must be a client-valid HTTP(S) origin without credentials, a path, query, or fragment. Non-loopback hosts must use HTTPS. `UpdateBaseUrl` defaults to the public GitHub Releases download location and must be an HTTPS URL ending in `/`.

The updater supports `x86_64-pc-windows-msvc` only. It verifies the raw manifest signature before parsing it, then verifies the streamed executable size and SHA-256 before installation.

## Server Image

Publish the server image from the same clean commit after the release check passes:

```powershell
pwsh -NoProfile -File scripts/publish-server-image.ps1 -Version 0.6.0
```

The script builds and pushes `linux/amd64`, records the source version and Git revision as OCI metadata, requests an SBOM and provenance attestation when supported, and prints the immutable `repository@sha256:...` deployment reference. `-PublishLatest` is convenience tagging only; deployment configuration must use the immutable digest.

## Publish the Release

Create a published GitHub Release for the same version and upload every artifact produced by `scripts/release.ps1`. Keep the direct executable, its checksum, the manifest, and its detached signature under their exact generated names. Include the binary archive, source archive, and both checksums.

The first updater-capable release is a bootstrap release and must be installed manually. Later clients check only their compiled HTTPS update base and use the signed manifest to decide whether to install an update.

CI uses `-SkipPostgresTests` only on the Windows packaging runner because the complete PostgreSQL suite runs on Linux CI. Do not use that switch for a human release approval without separately running the PostgreSQL tests.

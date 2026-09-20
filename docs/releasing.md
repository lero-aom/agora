# Releasing Agora

Release and image-publish scripts only operate from a clean Git worktree, including no untracked files. This ties the build, source archive, image labels, and release notes to one reviewed commit.

## Client archive

Run the complete release check from a Windows environment with a PostgreSQL test database configured in `DATABASE_URL`. Load `AGORA_UPDATE_SIGNING_KEY_B64` into that process from the release secret store before running the script. It is a Base64-encoded 32-byte Ed25519 seed, is never accepted as a command-line argument, and must not be echoed or logged. The script removes it from the process environment except while invoking the signer:

```powershell
pwsh -NoProfile -File scripts/release.ps1 -ServerUrl https://chat.example.com
```

The script fixes `SOURCE_DATE_EPOCH` to the commit timestamp, uses `Cargo.lock`, checks formatting, Clippy, unit tests, all PostgreSQL feature and invariant tests, and release builds. It derives the public key with the release-only `agora-update-sign` binary, embeds that key and `UpdateBaseUrl` into the Windows client, then creates these artifacts under `dist`:

- `agora-<version>-windows-x64.zip` and its SHA-256 sidecar.
- `agora-<version>-windows-x64-source.zip` and its SHA-256 sidecar.
- `agora-client-<version>-windows-x64.exe` and its SHA-256 sidecar.
- `agora-update-manifest.json` and `agora-update-manifest.json.sig`.

`ServerUrl` must be a client-valid HTTP(S) origin: it cannot contain credentials, a path, query, or fragment; non-loopback hosts must use HTTPS. The release script normalizes the accepted origin before compiling it into the client, matching the client's own server-origin rules. `UpdateBaseUrl` defaults to `https://github.com/lero-aom/agora/releases/latest/download/`; it must be an HTTPS URL ending in `/`, and it is the only update metadata base compiled into the client. The client derives the manual release page by removing a trailing `/download/` when present. HTTPS CDN redirects are limited to three hops; the raw manifest signature and signed executable digest remain the acceptance checks. Use `-UpdateBaseUrl` only for a reviewed HTTPS release location and publish all three update files there under their exact names. Do not point it at the chat server unless that exact static release location is intentionally the pinned update origin.

The updater publishes one target only: `x86_64-pc-windows-msvc`. Its manifest and direct executable path are intentionally fixed to that target, so `scripts/release.ps1` has no target override that could produce a second incompatible build under the same updater asset names.

The manifest is compact raw JSON with this fixed schema, and its detached signature covers the exact bytes written to disk:

```json
{"schema_version":1,"version":"<stable semver>","target":"x86_64-pc-windows-msvc","filename":"agora-client-<version>-windows-x64.exe","size":123,"sha256":"<64 lowercase hex characters>"}
```

Publish the direct executable, its SHA-256 file, manifest, and signature together. Publish the source archive, source checksum, binary archive, `LICENSE`, and `SOURCE.md` with the same release. The binary archive also contains the manifest, signature, and an `agora-client.exe.sha256` sidecar for its bundled executable. Before completion, the script extracts the archive, recomputes every archive and artifact checksum, and verifies the packaged raw manifest signature. `SOURCE.md` records the version, commit, repository URL, and AGPL source-access notice. Verify the published checksums and retain both source and binary artifacts together.

The first updater-capable release is a bootstrap release: distribute its direct executable or archive manually, because older builds cannot yet check this manifest. Once installed, later versions automatically check the compiled update base on startup. Users choose **Update and restart** to download and apply a verified executable; a server minimum-version or protocol event triggers an immediate signed check and exposes the same manual fallback.

CI uses `-SkipPostgresTests` only on the Windows packaging runner because the complete PostgreSQL feature and invariant suite is already run by the Linux CI job. Do not use that switch for a human release approval without separately running that suite.

## Server image

Publish from the same clean commit after the release checks pass:

```powershell
pwsh -NoProfile -File scripts/publish-server-image.ps1 -Version 0.6.0
```

The script builds and pushes `linux/amd64`, passes the source version and Git revision as OCI labels, pulls pinned base inputs, requests an SBOM and provenance attestation, and prints the immutable `repository@sha256:...` deployment reference. Production Compose explicitly deploys `linux/amd64`; ARM64 hosts are unsupported. Put that digest in the production `.env` file. `-PublishLatest` is optional convenience tagging only; deployments must use the immutable digest. When supplied, `-Version` must exactly match the `agora-server` Cargo package version; omitting it uses that package version.

For a reviewed base-image refresh, pass exact immutable build arguments such as `-RustImage 'rust:1-bookworm@sha256:...'` and `-RuntimeImage 'debian:bookworm-slim@sha256:...'`. Update the Dockerfile defaults and deployment documentation in the same reviewed change when adopting a new baseline.

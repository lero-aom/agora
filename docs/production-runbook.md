# Agora Production Runbook

This is the end-to-end operator guide for the public Agora service at `https://chat.aomagora.com`.

It covers four separate roles:

| Role | Machine | Responsibility |
| --- | --- | --- |
| Release workstation | Windows development machine or CI | Builds and signs Windows clients, runs release tests, and publishes the server image. |
| GitHub | `https://github.com/lero-aom/agora` and its Releases page | Stores public client downloads and signed update files. |
| Production server | Debian VPS, deployment directory `/opt/agora` | Runs Docker Compose, Caddy, Agora server, and PostgreSQL. |
| Player machine | A normal Windows PC with AoM Retold | Runs the published `agora-client.exe`; it does not need Docker, server secrets, or a signing key. |

## Fixed Service Values

Use these values consistently for the public service:

```text
Public URL:        https://chat.aomagora.com
Public hostname:   chat.aomagora.com
Source repository: https://github.com/lero-aom/agora.git
Server repository: ghcr.io/lero-aom/agora-server
Update host:       https://github.com/lero-aom/agora/releases/latest/download/
VPS deployment:    /opt/agora
```

The server image repository is known, but every deployment needs a new immutable digest such as `sha256:...`. Do not deploy `ghcr.io/lero-aom/agora-server:latest`: a tag can change after review, whereas a digest identifies the exact reviewed image.

## The Signing System

The client update signing seed is the source of most release confusion. Its role is limited and important:

1. `AGORA_UPDATE_SIGNING_KEY_B64` is a private Base64-encoded 32-byte Ed25519 seed.
2. The release workstation uses the seed to sign `agora-update-manifest.json`.
3. `scripts/release.ps1` derives the matching public key and embeds it in the Windows client it builds.
4. Installed clients accept an update only when its manifest signature matches that embedded public key and its executable checksum matches the manifest.

The VPS, Docker Compose, PostgreSQL, Caddy, and normal players never need the private signing seed.

### Create the first signing seed

Because no prior updater key exists, create one exactly once on the release workstation. Do this in a private session and immediately store the resulting value in a password manager, hardware-backed vault, or CI secret store under a clear name such as `Agora update signing seed`.

```powershell
$seedBytes = [byte[]]::new(32)
[System.Security.Cryptography.RandomNumberGenerator]::Fill($seedBytes)
$updateSigningSeed = [Convert]::ToBase64String($seedBytes)
[Array]::Clear($seedBytes, 0, $seedBytes.Length)

# Display this only when ready to save it into private secret storage.
$updateSigningSeed
```

The displayed value is the one and only private seed. Never put it in Git, `.env`, the VPS, GitHub Release assets, screenshots, chat logs, or command-line arguments. Do not generate a replacement after publishing the first updater-capable client: older clients would reject manifests signed by the replacement key.

### Use the seed for a release

Load the stored value into the release PowerShell process without placing it in a file. For example, when using PowerShell SecretManagement:

```powershell
$env:AGORA_UPDATE_SIGNING_KEY_B64 = Get-Secret -Name 'Agora update signing seed' -AsPlainText
```

Run the release command in that same PowerShell window, then clear the variable when finished:

```powershell
Remove-Item Env:\AGORA_UPDATE_SIGNING_KEY_B64 -ErrorAction SilentlyContinue
```

If a different password manager or CI secret store is used, use its equivalent process to inject the value into the process environment. Do not paste the seed into a PowerShell command saved in shell history.

## One-Time VPS Setup

This section is for the Debian VPS only. Run it from an SSH session after DNS for `chat.aomagora.com` points to the VPS and ports 80 and 443 are open in both the provider firewall and any host firewall.

The host must be amd64, with Docker Engine and the Docker Compose plugin installed. PostgreSQL and the Agora server must not be exposed directly to the Internet.

If `/opt/agora` is not already a checkout:

```bash
sudo git clone https://github.com/lero-aom/agora.git /opt/agora
sudo chown -R "$USER":"$USER" /opt/agora
cd /opt/agora
```

If it already exists, use the reviewed release commit or tag rather than an unreviewed branch tip:

```bash
cd /opt/agora
git fetch --tags origin
git checkout <reviewed-release-tag-or-commit>
```

### Production environment file

The existing `/opt/agora/.env` already contains `POSTGRES_PASSWORD` and `AGORA_SESSION_SECRET`. Keep those values private and add the remaining production values:

```dotenv
POSTGRES_PASSWORD=<existing-private-password>
AGORA_SESSION_SECRET=<existing-private-secret>
AGORA_PUBLIC_URL=https://chat.aomagora.com
AGORA_SITE_ADDRESS=chat.aomagora.com
AGORA_MIN_CLIENT_VERSION=0.6.0
AGORA_SERVER_IMAGE_REPOSITORY=ghcr.io/lero-aom/agora-server
AGORA_SERVER_IMAGE_DIGEST=sha256:<digest-printed-by-publish-server-image>
STEAM_WEB_API_KEY=
AGORA_MICROSOFT_CLIENT_ID=
AGORA_MICROSOFT_CLIENT_SECRET=
```

Protect the file:

```bash
cd /opt/agora
chmod 600 .env
```

`AGORA_MIN_CLIENT_VERSION` must be a stable version and should match the minimum client you intend to support. Do not raise it until the corresponding signed client files are publicly available on GitHub Releases.

### Optional Microsoft sign-in

Microsoft personal-account sign-in stays disabled when both Microsoft variables are empty. To enable it:

1. Register a Microsoft Entra application for personal Microsoft accounts only.
2. Register this exact Web redirect URI: `https://chat.aomagora.com/auth/microsoft/callback`.
3. Set both `AGORA_MICROSOFT_CLIENT_ID` and `AGORA_MICROSOFT_CLIENT_SECRET` in `/opt/agora/.env`.
4. Redeploy the server.

Do not set only one value. Do not send either secret to anyone or store it in Git. Microsoft identities remain separate from Steam identities.

### First stack start

After the `.env` file includes a real server image digest:

```bash
cd /opt/agora
docker compose --env-file .env -f docker-compose.prod.yml config --quiet
docker compose --env-file .env -f docker-compose.prod.yml pull
docker compose --env-file .env -f docker-compose.prod.yml up -d --wait
docker compose --env-file .env -f docker-compose.prod.yml ps
curl --fail --retry 12 https://chat.aomagora.com/health
```

The expected response is:

```json
{"status":"ok","database":"ok"}
```

Caddy obtains and renews TLS certificates automatically. If startup fails before Caddy is healthy, verify DNS, public port 80/443 reachability, and `docker compose ... logs --tail 200 proxy` before changing security settings.

If `docker compose ... pull` reports that GHCR access is denied, authenticate the VPS Docker client with a GitHub token that has only the package-read permission required for the image. Do not add that token to `/opt/agora/.env` or Git.

## Standard Release Procedure

Run this section from the release workstation, not from the VPS.

### 1. Prepare the reviewed source

Both release scripts refuse a dirty worktree, including untracked files. Start from the exact commit that will be deployed:

```powershell
git status --short
git rev-parse HEAD
```

The first command must produce no output. Ensure the Windows machine has the Rust MSVC toolchain, PowerShell 7, Docker Buildx, GitHub CLI if using it to publish releases, and registry credentials that can push `ghcr.io/lero-aom/agora-server`.

The release script also needs a PostgreSQL test database through `DATABASE_URL`. A disposable local database is appropriate; do not point release tests at production.

### Disposable PostgreSQL database for release checks

If no local test database already exists, start an isolated disposable PostgreSQL container on the release workstation:

```powershell
docker run --detach --rm --name agora-release-postgres `
  --env POSTGRES_DB=agora `
  --env POSTGRES_USER=agora `
  --env POSTGRES_PASSWORD=change-me `
  --publish 127.0.0.1:5433:5432 `
  postgres:17.5-bookworm@sha256:fbcea1bd13b6a882cd6caa6b58db3ae5c102efe50ec625b3e2a5cbc50db5bfe4
docker exec agora-release-postgres pg_isready -U agora -d agora
$env:DATABASE_URL = 'postgres://agora:change-me@127.0.0.1:5433/agora'
```

The container is only for automated release tests. Stop it and clear the test connection string after the release command finishes:

```powershell
docker stop agora-release-postgres
Remove-Item Env:\DATABASE_URL -ErrorAction SilentlyContinue
```

### 2. Run the signed client release check

Load the signing seed into the current PowerShell process, set `DATABASE_URL` to the disposable test database, and run:

```powershell
pwsh -NoProfile -File scripts/release.ps1 `
  -ServerUrl https://chat.aomagora.com `
  -UpdateBaseUrl https://github.com/lero-aom/agora/releases/latest/download/
```

The script runs formatting, Clippy, workspace tests, PostgreSQL tests, release builds, artifact checksum checks, and manifest-signature verification. It creates these files under `dist`:

```text
agora-<version>-windows-x64.zip
agora-<version>-windows-x64.zip.sha256
agora-<version>-windows-x64-source.zip
agora-<version>-windows-x64-source.zip.sha256
agora-client-<version>-windows-x64.exe
agora-client-<version>-windows-x64.exe.sha256
agora-update-manifest.json
agora-update-manifest.json.sig
```

Clear the seed from the parent terminal when the command exits:

```powershell
Remove-Item Env:\AGORA_UPDATE_SIGNING_KEY_B64 -ErrorAction SilentlyContinue
```

### 3. Publish the server image

From the same clean commit and after the release check passes:

```powershell
docker login ghcr.io
pwsh -NoProfile -File scripts/publish-server-image.ps1 -Version 0.6.0
```

Replace `0.6.0` with the version in `Cargo.toml` for later releases, or omit `-Version` to use that package version automatically. The command prints a line like:

```text
ghcr.io/lero-aom/agora-server@sha256:<immutable-image-digest>
```

Copy only the repository and digest into the VPS `.env` file:

```dotenv
AGORA_SERVER_IMAGE_REPOSITORY=ghcr.io/lero-aom/agora-server
AGORA_SERVER_IMAGE_DIGEST=sha256:<immutable-image-digest>
```

Never substitute `:latest` for the digest.

### 4. Publish the GitHub Release

Create a published GitHub Release for the same version, then upload every file listed in the previous artifact list. The four updater files must keep their exact names:

```text
agora-client-<version>-windows-x64.exe
agora-client-<version>-windows-x64.exe.sha256
agora-update-manifest.json
agora-update-manifest.json.sig
```

Also upload the Windows archive, archive checksum, source archive, and source checksum. A draft or prerelease is not the public `latest` release, so it will not satisfy clients compiled with the `releases/latest/download/` update URL.

When using GitHub CLI, the command has this general form:

```powershell
gh release create v0.6.0 `
  dist\agora-0.6.0-windows-x64.zip `
  dist\agora-0.6.0-windows-x64.zip.sha256 `
  dist\agora-0.6.0-windows-x64-source.zip `
  dist\agora-0.6.0-windows-x64-source.zip.sha256 `
  dist\agora-client-0.6.0-windows-x64.exe `
  dist\agora-client-0.6.0-windows-x64.exe.sha256 `
  dist\agora-update-manifest.json `
  dist\agora-update-manifest.json.sig `
  --title "Agora 0.6.0" --generate-notes
```

Use the actual release version in every file name and tag.

## Deploy a New Server Release

On the Debian VPS, first make sure the provider backup service has a recent successful backup or snapshot. Provider backups are useful, but confirm that they cover Docker named volumes and periodically test a restoration process. Do not use `docker compose down -v`: that removes persistent data.

Then deploy the reviewed source revision and image digest:

```bash
cd /opt/agora
git fetch --tags origin
git checkout <reviewed-release-tag-or-commit>
# Edit .env: update AGORA_SERVER_IMAGE_DIGEST and, only when intended, AGORA_MIN_CLIENT_VERSION.
docker compose --env-file .env -f docker-compose.prod.yml config --quiet
docker compose --env-file .env -f docker-compose.prod.yml pull server
docker compose --env-file .env -f docker-compose.prod.yml up -d --wait
docker compose --env-file .env -f docker-compose.prod.yml ps
docker compose --env-file .env -f docker-compose.prod.yml logs --tail 200 server
curl --fail --retry 12 https://chat.aomagora.com/health
```

Use `pull` without `server` when Caddy or PostgreSQL images also changed. The explicit pull is recommended even though Compose can fetch a missing image during `up`: it confirms that the configured digest is available before the service restarts.

Smoke-test from a normal Windows player machine with the new signed release before declaring deployment complete:

1. Start AoM Retold and the packaged client.
2. Sign in with Steam.
3. If enabled, sign in with Microsoft and confirm it is a distinct Agora identity.
4. Send and receive a global message, direct message, and friend request.
5. Sign out, restart the client, and confirm session restoration.
6. Confirm `/health`, server logs, and normal WebSocket/chat behavior remain healthy.

## Normal Player Installation and Update Test

Players download `agora-<version>-windows-x64.zip` from the published GitHub Release. They should extract it to a writable user-owned folder, such as `C:\Users\<user>\Apps\Agora`, rather than running it from inside the ZIP or from a protected system folder.

The normal player runs the extracted `agora-client.exe`. No Docker, VPS access, signing key, `.env`, or `AGORA_SERVER_URL` override is needed. The release already embeds `https://chat.aomagora.com`.

The first updater-capable release is installed manually. To test later automatic updates:

1. Install an older signed release in a writable test folder.
2. Publish a newer signed GitHub Release using the same signing seed.
3. Confirm the newer release is the public GitHub `latest` release and contains the four updater files with exact names.
4. Start the older client while it can reach `https://chat.aomagora.com` and GitHub.
5. Choose **Update and restart** when the update banner appears.
6. Confirm the restarted executable has the new version and still connects normally.

Do not test automatic updates against a loopback server: local development intentionally disables update checks.

## Routine Maintenance

Run these checks from `/opt/agora` during routine review and after any deployment:

```bash
cd /opt/agora
docker compose --env-file .env -f docker-compose.prod.yml ps
docker compose --env-file .env -f docker-compose.prod.yml logs --tail 200 server
docker compose --env-file .env -f docker-compose.prod.yml logs --tail 200 proxy
curl --fail https://chat.aomagora.com/health
```

Check that the VPS backup service completed successfully and that its retention period meets your recovery needs. At least quarterly, and after important schema migrations, test the provider's restore procedure on a separate VPS or isolated environment. A snapshot that has never been restored is not a verified backup.

The repository also contains optional PostgreSQL dump and restore scripts. They require PowerShell 7 on Debian and are useful when you need an application-specific logical backup:

```bash
cd /opt/agora
pwsh -NoProfile -File scripts/backup-postgres.ps1 -EnvFile .env -OutputDirectory /srv/agora-backups
```

The dump and its `.sha256` file must be retained together. Do not restore directly into production without first validating the backup and stopping the server.

## Rollback and Recovery

### Server rollback

Record the prior `AGORA_SERVER_IMAGE_DIGEST` before every deployment. If the new server binary is faulty and database migrations are compatible, restore the old digest in `/opt/agora/.env` and run:

```bash
cd /opt/agora
docker compose --env-file .env -f docker-compose.prod.yml pull server
docker compose --env-file .env -f docker-compose.prod.yml up -d --wait
curl --fail https://chat.aomagora.com/health
```

Do not assume an older server can safely read a database after a newer release has migrated it. If a migration is incompatible, restore the tested database backup or provider snapshot as part of the approved incident procedure.

### Database recovery

For a provider-managed recovery, follow the VPS provider's documented restore process. Keep the Agora server stopped until PostgreSQL is restored and verified, then start the stack and check `/health`, login, and WebSocket behavior.

For the repository's logical-restore script, see `docs/operations.md`. It requires an explicit `-Force` acknowledgement and refuses to run while the server is still running.

### Client update recovery

Do not replace published update files in place. If a client release is defective, publish a newer corrected version using the same signing seed. Users with an installed earlier release can then verify and install the correction normally.

## Secret and Configuration Inventory

| Value | Where it belongs | Never place it |
| --- | --- | --- |
| `AGORA_UPDATE_SIGNING_KEY_B64` | Release workstation secret manager or CI secret store | VPS, `.env`, GitHub assets, Git |
| `POSTGRES_PASSWORD` | `/opt/agora/.env` and protected backup of deployment secrets | Git, GitHub Releases |
| `AGORA_SESSION_SECRET` | `/opt/agora/.env` and protected backup of deployment secrets | Git, GitHub Releases |
| `STEAM_WEB_API_KEY` | `/opt/agora/.env`, only if used | Client, Git |
| Microsoft client ID and secret | `/opt/agora/.env` | Client, Git |
| Server image digest | `/opt/agora/.env`, deployment records | Mutable `:latest` tag |

Avoid rotating `AGORA_SESSION_SECRET` casually: it invalidates stored session-token hashes and signs users out. Do not rotate the update signing seed after public client releases exist without an explicit client trust-migration design.

## Deployment Record

For each release, record the following outside the repository:

```text
Release version:
Git commit:
GitHub Release URL:
Server image digest:
Deployment timestamp (UTC):
VPS backup/snapshot identifier:
Health-check result:
Smoke-test result:
Rollback image digest:
```

This record makes it possible to identify what was deployed and revert safely without exposing player content, tokens, passwords, or private keys.

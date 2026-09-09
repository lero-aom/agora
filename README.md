# Agora

Agora is a Rust/Dioxus companion app for Age of Mythology: Retold global chat.

Current implementation status: workspace skeleton, shared protocol types, server health/version endpoints, Steam OpenID login, local dev login, Agora session issuing/refresh/logout, authenticated global WebSocket chat, live presence counts, friend/block/report relationship APIs and client UI, basic in-memory rate limits, client session restore through Windows Credential Manager, AoM window detection, tray/passive-overlay/interactive-overlay shell behavior, database migrations, and VPS Docker Compose scaffold.

## Local Server

Start PostgreSQL through Docker Compose:

```powershell
Copy-Item .env.example .env
docker compose up postgres
```

In another terminal:

```powershell
$env:DATABASE_URL="postgres://agora:change-me@localhost:5432/agora"
$env:AGORA_PUBLIC_URL="http://localhost:8080"
$env:AGORA_ENABLE_DEV_LOGIN="true"
$env:AGORA_TRUST_PROXY_HEADERS="false"
$env:AGORA_SESSION_SECRET="dev-only-change-this"
cargo run -p agora-server
```

Steam login starts at `POST /auth/steam/device/start`. The server uses Steam OpenID, then issues Agora-owned access and refresh tokens.

Global chat uses `GET /ws` over WebSocket with `Authorization: Bearer <access_token>`. The access token comes from login or refresh, messages are stored in PostgreSQL, new messages are broadcast to connected clients, and presence counts update while clients connect, disconnect, or change state.

Relationship endpoints use `Authorization: Bearer <access_token>`:

```text
GET /users/search?q=alice
GET /friends
POST /friends
POST /friends/{friendship_id}/accept
POST /friends/{friendship_id}/decline
DELETE /friends/{user_id}
GET /blocks
POST /blocks
DELETE /blocks/{user_id}
POST /reports
```

Current server rate limits are in-memory and per server process:

```text
Auth endpoints: 60 requests/minute per client IP
Relationship reads: 120 requests/minute per client IP
Relationship writes: 40 requests/minute per client IP
Global chat sends: 20 messages/10 seconds per user
```

For one VPS this is sufficient as a first guardrail. If Agora runs behind multiple server replicas later, move these limits to Redis or another shared store.

## Production Deployment

Prerequisites:

```text
VPS with Docker and Docker Compose
Domain DNS A/AAAA record pointing to the VPS
Inbound ports 80 and 443 open
Outbound HTTPS from Docker containers working
Published server image, default: ghcr.io/lero-aom/agora-server:latest
```

Build and publish the Linux server image from a local machine:

```powershell
docker login ghcr.io -u <github-username>
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\publish-server-image.ps1
```

The publish script builds `linux/amd64` and pushes both `0.1.0` and `latest` tags to `ghcr.io/lero-aom/agora-server` by default. The VPS should pull this image; it should not build Rust release artifacts itself.

Prepare the VPS deployment directory:

```bash
sudo mkdir -p /opt/agora
sudo chown "$USER:$USER" /opt/agora
cd /opt/agora
```

Copy these files to `/opt/agora` on the VPS:

```text
docker-compose.prod.yml as docker-compose.yml
Caddyfile
.env
```

Create production env:

```powershell
Copy-Item .env.production.example .env
```

Edit `.env` for production:

```text
POSTGRES_PASSWORD=<long random password>
AGORA_SESSION_SECRET=<long random secret>
AGORA_PUBLIC_URL=https://chat.aomagora.com
AGORA_ENABLE_DEV_LOGIN=false
AGORA_TRUST_PROXY_HEADERS=true
AGORA_SITE_ADDRESS=chat.aomagora.com
AGORA_MIN_CLIENT_VERSION=0.1.0
AGORA_SERVER_IMAGE=ghcr.io/lero-aom/agora-server:latest
STEAM_WEB_API_KEY=
```

Use URL-safe random values for `POSTGRES_PASSWORD` and `AGORA_SESSION_SECRET`, for example `openssl rand -hex 32`. Leave `STEAM_WEB_API_KEY` empty for the first deployment; Steam OpenID login does not require it.

Start the stack:

```bash
cd /opt/agora
sudo docker compose pull
sudo docker compose up -d
```

Verify deployment:

```bash
sudo docker compose ps
curl https://chat.aomagora.com/health
curl https://chat.aomagora.com/version
```

Production checks before inviting testers:

```text
AGORA_ENABLE_DEV_LOGIN=false
AGORA_PUBLIC_URL uses https:// and the public domain
AGORA_SITE_ADDRESS is the public domain, not :80
AGORA_TRUST_PROXY_HEADERS=true when the server is only reachable through Caddy
AGORA_SESSION_SECRET is at least 32 random characters and not a placeholder
Caddy has issued a valid TLS certificate
Steam OpenID login completes on the public domain
PostgreSQL volume is backed up or snapshot by the VPS provider
```

If the VPS uses nftables or another firewall, keep public inbound access limited to ports 22, 80, and 443, but allow Docker bridge forwarding so containers can reach Let's Encrypt and Steam over outbound HTTPS.

## Backup And Restore

Create a compressed PostgreSQL backup from the running Compose stack:

```powershell
New-Item -ItemType Directory -Force backups
docker compose exec postgres pg_dump -U agora -d agora -Fc -f /tmp/agora.dump
docker compose cp postgres:/tmp/agora.dump ./backups/agora.dump
```

Test the backup by restoring into a separate database:

```powershell
docker compose cp ./backups/agora.dump postgres:/tmp/agora.dump
docker compose exec postgres createdb -U agora agora_restore
docker compose exec postgres pg_restore -U agora -d agora_restore --clean --if-exists /tmp/agora.dump
```

For a live restore, stop the server first, recreate the `agora` database, restore the dump, then start the stack again.

## Local Client

```powershell
cargo run -p agora-client
```

Local debug builds default to `http://localhost`, which matches the local Docker Compose proxy. Release builds can bake in a different default with `AGORA_DEFAULT_SERVER_URL`; the release script defaults to `https://chat.aomagora.com`. At runtime, `AGORA_SERVER_URL` overrides either default. If you run `agora-server` manually on its default `127.0.0.1:8080` without Caddy, set `$env:AGORA_SERVER_URL="http://localhost:8080"` before launching the client.

When pointed at `localhost`, the client uses a local-only dev login because Steam rejects `localhost` as an OpenID realm during final confirmation. Against a real public domain, the same client uses Steam OpenID, then stores only the Agora refresh token in Windows Credential Manager and refreshes the session on startup and before access-token expiry. After sign-in, the Global chat tab connects automatically, reconnects after transient drops, supports Enter-to-send, and the Friends and Block / Report tabs can search users and call the relationship APIs.

Overlay/tray behavior on Windows:

- When AoM is not detected, Agora starts hidden in the system tray.
- Clicking the tray icon does nothing; Agora has no standalone desktop mode.
- When a visible `Age of Mythology: Retold` window is detected, Agora switches to a small passive topmost overlay at the top-center of the AoM client area.
- Passive overlay is disabled, no-activate, click-through, and shows the last 3 global chat messages plus presence counts while AoM keeps focus.
- Passive overlay hides automatically when AoM loses foreground focus, such as after `Alt+Tab`.
- Foreground focus changes are handled through a Windows foreground-event hook so showing/hiding the overlay does not wait for the fallback polling interval.
- Tap `Ctrl+Enter` once to enter chat mode only while AoM is the foreground window; the keys do not need to be held.
- Chat mode temporarily enables and focuses the overlay so normal Dioxus/WebView input works.
- Chat mode shows the last 10 global chat messages, focuses the message input, and sends with `Enter` without leaving chat mode.
- Tap `Escape` or `Ctrl+Enter` in chat mode to return to passive mode and focus AoM.
- If chat mode loses focus to another app, Agora hides without leaving chat mode, then restores chat mode when AoM is foreground again.
- Chat mode has a compact left tab column for Global, Friends, and Block / Report.
- While AoM is detected, Agora never switches to the desktop window; sign-in and sign-off happen from the overlay.

Current overlay detection is process/window-title based. Menu/lobby/post-game detection and automatic hiding during matches are still future work.

Use distinct local dev users for multi-window testing:

```powershell
$env:AGORA_SERVER_URL="http://localhost"
$env:AGORA_DEV_DISPLAY_NAME="Alice"
cargo run -p agora-client
```

In another terminal:

```powershell
$env:AGORA_SERVER_URL="http://localhost"
$env:AGORA_DEV_DISPLAY_NAME="Bob"
cargo run -p agora-client
```

## Checks

```powershell
cargo fmt --all
cargo check --workspace
cargo clippy --workspace --all-targets
cargo test --workspace
```

## Release Build

Build checked release binaries and package them into `dist\agora-<version>-windows-x64.zip`. By default, the packaged Windows client points at `https://chat.aomagora.com`:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\release.ps1
```

To build a package for another server URL:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\release.ps1 -ServerUrl https://your-domain.example
```

If GNU Make is available, the same workflow is exposed as:

```powershell
make release
```

The release script runs formatting, clippy, tests, `cargo build --locked --release -p agora-server`, and `cargo build --locked --release -p agora-client`. Use `-SkipChecks` only when checks have already run in the same tree. The same release executable can still be pointed elsewhere with `AGORA_SERVER_URL` for local debugging.

## Docker Troubleshooting

If `docker compose up postgres` fails with `open //./pipe/dockerDesktopLinuxEngine: The system cannot find the file specified`, Docker Desktop is installed but its Linux engine is not running.

Fix:

```powershell
Start-Process "C:\Program Files\Docker\Docker\Docker Desktop.exe"
```

Wait until Docker Desktop says it is running, then retry:

```powershell
docker compose up -d postgres
```

If Docker Desktop asks for permissions, approve them or start Docker Desktop from the Start menu as your Windows user.

# Agora Plan

## Goal

Agora is an external Windows companion app for Age of Mythology: Retold that restores a usable global chat experience without modifying the game.

It has one user-facing interface:

- In-game menu overlay when AoM Retold is focused and the player is in multiplayer menus, lobby, or post-game.

The overlay must hide automatically during matches and reappear after the match ends.

## Non-Negotiable Constraints

- Rust codebase.
- Dioxus desktop UI.
- Windows x64 first.
- Steam login first.
- Microsoft login later, not in the first milestone.
- VPS deployment with Docker Compose.
- PostgreSQL for relational data.
- WebSocket-based realtime chat and presence.
- Open source client and server.
- Store as little account data as possible.
- No game memory writes.
- No DLL injection.
- No anti-cheat bypass.
- No debug privilege enabling.
- AoM memory probing is allowed only for game-state detection and must be documented clearly.

## MVP Scope

Required MVP functionality:

- Steam sign-in.
- Global chat.
- Private messages.
- Friends.
- Block/report.
- Presence counts: online, in game, looking for game.
- Tray-only idle mode when AoM is not running or not focused.
- Overlay mode when AoM is running and in eligible multiplayer/menu states.
- Automatic overlay hide during matches.
- Automatic overlay re-show in post-game.
- Automatic updates.
- Server-side minimum client version gate.
- Basic moderation tooling for reports and bans.

Explicitly not MVP:

- Microsoft login.
- Voice chat.
- Rich media messages.
- Multiple public channels.
- Clans/groups.
- Matchmaking replacement.
- Reading player resources, army, map, or match stats.

## Recommended Architecture

Use a Rust workspace:

```text
agora/
  Cargo.toml
  crates/
    agora-client/
    agora-server/
    agora-common/
```

### `agora-client`

Responsibilities:

- Dioxus desktop app.
- Shared chat UI components.
- Standalone desktop shell.
- Borderless/topmost overlay shell.
- Steam login browser flow.
- Local session storage.
- WebSocket connection and reconnect logic.
- AoM process/window detection.
- Read-only AoM game-state probe.
- Auto-update check and restart.

### `agora-server`

Responsibilities:

- Steam OpenID login flow.
- Agora session issuing and validation.
- WebSocket chat gateway.
- Global chat storage/broadcast.
- DM storage/delivery.
- Friend/block/report APIs.
- Presence tracking.
- Rate limiting.
- Moderation endpoints.
- Minimum client version enforcement.

### `agora-common`

Responsibilities:

- Shared DTOs.
- WebSocket event types.
- API request/response structs.
- Validation constants.
- Client/server protocol version.

## Server Stack

Use:

- `axum` for HTTP and WebSocket routes.
- `tokio` runtime.
- `sqlx` for PostgreSQL.
- `serde` / `serde_json` for payloads.
- `tower-http` for tracing, CORS where needed, and request limits.
- `tracing` for logs.
- `argon2` or keyed hashing for stored session token hashes.

Avoid Redis for the first version. Keep presence in server memory and persist only durable relational data. Add Redis later only if multiple server instances become necessary.

## Client Stack

Use:

- `dioxus` with desktop feature.
- `windows-sys` for process/window detection and overlay positioning.
- `reqwest` for HTTP.
- `tokio-tungstenite` or equivalent WebSocket client.
- `keyring` for storing refresh/session tokens in Windows Credential Manager.
- `self_update` or a small manifest-based updater for automatic updates.

The client should not contain server secrets, Steam API keys, or Microsoft secrets.

## Steam Login First

Use Steam OpenID for the first implementation because it is practical for a third-party desktop companion app.

Flow:

1. Client requests a login challenge from `POST /auth/steam/device/start`.
2. Server returns a browser URL and a polling token.
3. Client opens the browser URL in the user's default browser.
4. User signs in through Steam.
5. Steam redirects back to the Agora server.
6. Server validates the OpenID response with Steam.
7. Server extracts the user's SteamID64.
8. Server optionally fetches persona/avatar using a server-side Steam Web API key.
9. Client polls `POST /auth/steam/device/poll` until login succeeds or expires.
10. Server returns an Agora session.
11. Client stores only the refresh/session secret in Windows Credential Manager.

Stored Steam data:

- SteamID64.
- Last known Steam persona name.
- Last known Steam avatar URL.
- Last refresh timestamp.

Do not store Steam credentials, Steam cookies, access tokens from Steam, or unnecessary profile data.

## Session Model

Use Agora-owned sessions after Steam identity is verified.

Recommended approach:

- Short-lived access token for HTTP/WebSocket auth.
- Long-lived refresh token stored only as a hash in PostgreSQL.
- Refresh token stored locally via Windows Credential Manager.
- Server can revoke sessions.
- Sign-out deletes local token and revokes server session.

## Core Database Tables

### Users

```text
users
  id uuid primary key
  display_name text not null
  avatar_url text null
  role text not null default 'user'
  created_at timestamptz not null
  last_seen_at timestamptz null
  suspended_until timestamptz null
  banned_at timestamptz null
```

### Linked Identities

```text
identities
  id uuid primary key
  user_id uuid references users(id)
  provider text not null
  provider_user_id text not null
  provider_display_name text null
  provider_avatar_url text null
  created_at timestamptz not null
  updated_at timestamptz not null
  unique(provider, provider_user_id)
```

### Sessions

```text
sessions
  id uuid primary key
  user_id uuid references users(id)
  refresh_token_hash text not null
  created_at timestamptz not null
  expires_at timestamptz not null
  revoked_at timestamptz null
  user_agent text null
  last_used_at timestamptz null
```

### Global Messages

```text
global_messages
  id uuid primary key
  user_id uuid references users(id)
  body text not null
  created_at timestamptz not null
  deleted_at timestamptz null
  deleted_by uuid null references users(id)
```

### DMs

```text
dm_threads
  id uuid primary key
  created_at timestamptz not null

dm_members
  thread_id uuid references dm_threads(id)
  user_id uuid references users(id)
  last_read_message_id uuid null
  primary key(thread_id, user_id)

dm_messages
  id uuid primary key
  thread_id uuid references dm_threads(id)
  user_id uuid references users(id)
  body text not null
  created_at timestamptz not null
  deleted_at timestamptz null
```

### Friends

```text
friendships
  id uuid primary key
  requester_id uuid references users(id)
  addressee_id uuid references users(id)
  status text not null
  created_at timestamptz not null
  updated_at timestamptz not null
  unique(requester_id, addressee_id)
```

Statuses:

- `pending`
- `accepted`
- `declined`
- `removed`

### Blocks

```text
blocks
  blocker_id uuid references users(id)
  blocked_id uuid references users(id)
  created_at timestamptz not null
  primary key(blocker_id, blocked_id)
```

### Reports

```text
reports
  id uuid primary key
  reporter_id uuid references users(id)
  reported_user_id uuid references users(id)
  message_id uuid null
  message_kind text null
  reason text not null
  details text null
  status text not null default 'open'
  created_at timestamptz not null
  resolved_at timestamptz null
  resolved_by uuid null references users(id)
```

### Moderation Actions

```text
moderation_actions
  id uuid primary key
  moderator_id uuid references users(id)
  target_user_id uuid references users(id)
  action text not null
  reason text not null
  created_at timestamptz not null
  expires_at timestamptz null
```

Actions:

- `delete_message`
- `timeout`
- `suspend`
- `ban`

## WebSocket Protocol

Client connects with current access token:

```text
GET /ws
Authorization: Bearer <access_token>
```

Client-to-server events:

```text
hello
heartbeat
presence_update
global_message_send
dm_message_send
friend_request_send
friend_request_accept
friend_remove
block_user
unblock_user
report_user
report_message
```

Server-to-client events:

```text
hello_ok
minimum_version_required
presence_counts
presence_user_changed
global_message_created
global_message_deleted
dm_thread_updated
dm_message_created
friendship_updated
block_updated
report_created
moderation_action_applied
error
```

Keep message bodies plain text only.

Initial limits:

- 1,000 characters per message.
- Server-side rate limit per user.
- Server-side duplicate/spam throttling.
- Reject messages from suspended/banned users.
- Do not deliver messages from blocked users.
- Do not allow DMs to users who blocked the sender.

## HTTP API

Minimum routes:

```text
GET  /health
GET  /version

POST /auth/steam/device/start
GET  /auth/steam/callback
POST /auth/steam/device/poll
POST /auth/refresh
POST /auth/logout

GET  /messages/global/history
GET  /dm/threads
GET  /dm/threads/:id/messages

GET  /friends
POST /friends/:user_id/request
POST /friends/:user_id/accept
DELETE /friends/:user_id

GET  /blocks
POST /blocks/:user_id
DELETE /blocks/:user_id

POST /reports

GET  /moderation/reports
POST /moderation/reports/:id/resolve
POST /moderation/messages/:id/delete
POST /moderation/users/:id/timeout
POST /moderation/users/:id/ban
```

Moderation routes require `role = 'moderator'` or `role = 'admin'`.

## Presence Model

Presence is computed from connected WebSocket sessions and heartbeats.

States:

```text
offline
online
looking_for_game
in_game
```

Rules:

- `online`: connected to WebSocket.
- `looking_for_game`: connected and user toggled LFG on.
- `in_game`: client reports AoM match state as `InMatch`.
- Stale sessions expire if no heartbeat is received within a short timeout.
- Counts are broadcast periodically and after meaningful state changes.

Presence display:

```text
247 online
83 in game
41 looking for game
```

## AoM Detection And Overlay State

Use a small detector in the client.

Detector states:

```text
NotRunning
RunningUnknown
MainMenu
MultiplayerMenu
Lobby
InMatch
PostGame
```

Overlay visibility rules:

- `NotRunning`: stay hidden in the system tray.
- `MultiplayerMenu`: show overlay.
- `Lobby`: show overlay.
- `PostGame`: show overlay.
- `InMatch`: hide overlay.
- `RunningUnknown`: hide overlay by default.
- `MainMenu`: hide overlay by default unless later testing proves this is useful.

Detection implementation:

1. Detect `AoMRT_s.exe` using Windows process enumeration.
2. Find the main visible AoM window for the process.
3. Track window bounds and foreground/minimized state.
4. Add read-only game-state probing only for state classification.
5. Reuse the safe patterns from `mythicwharf` where appropriate.

Current implementation status:

- Implemented: visible AoM window detection by process/window, foreground-event hook wakeups, client-rect tracking, foreground-focus gating, minimized-window filtering, top-center passive disabled/no-activate/click-through overlay, tap-once `Ctrl+Enter` typing-mode entry/exit using pressed-only global hotkey events, `Escape` typing-mode exit, compact chat-mode tabs, tray-only idle mode, and global chat composer autofocus.
- Not yet implemented: menu/lobby/post-game classification and automatic match hide/re-show.

Allowed AoM process access:

```text
OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)
ReadProcessMemory
CreateToolhelp32Snapshot
VirtualQueryEx if signature scanning is needed
```

Forbidden AoM process access:

```text
WriteProcessMemory
CreateRemoteThread
DLL injection
debug privilege escalation
process hiding/evasion
memory patching
input automation
```

The memory probe must read only the minimum addresses needed to classify the game state. It must not read player resources, map data, units, stats, chat, or match data.

If game-state signatures break after an AoM update:

- Hide the overlay by default.
- Keep the app hidden in tray until detection is updated.
- Show a clear detector status.
- Update detection config through an Agora release.

## Overlay Implementation

The overlay should be an interactive borderless/topmost Dioxus desktop window aligned to the AoM client area.

Unlike `mythicwharf`'s click-through overlay, Agora's overlay needs input for chat and tabs, so it must accept mouse and keyboard focus when visible.

Overlay requirements:

- Align to the AoM client rect.
- Move/resize when AoM moves/resizes.
- Hide when AoM is minimized.
- Hide when AoM loses foreground focus unless the overlay is currently focused for typing.
- React to foreground changes through `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` instead of relying only on polling.
- Hide during `InMatch`.
- Re-show in `PostGame`.
- Provide manual tap-once `Ctrl+Enter` typing-mode entry only while AoM is foreground.
- Return to passive mode with `Escape` or tap-once `Ctrl+Enter` and explicitly focus AoM.
- Suspend and hide typing mode if it loses focus to another app; restore it when AoM is foreground again.
- Prefer borderless/windowed AoM. Exclusive fullscreen may prevent reliable external overlays.

## UI Plan

Shared UI sections:

- Presence header.
- Global chat tab.
- Private messages tab.
- Friends tab.
- Block/report tab.
- User profile/sign-out area.
- Connection/update/status line.

Overlay mode:

- Top-center passive chat window by default.
- Passive mode is disabled, no-activate, click-through, and leaves AoM focused.
- Passive mode is only visible while AoM is the foreground window.
- Passive mode shows the last 3 global chat messages.
- Tapping `Ctrl+Enter` enters typing mode without holding the keys, but only when AoM is focused.
- Chat mode shows the last 10 global chat messages and focuses the message input.
- Chat mode sends global messages with `Enter` and stays in typing mode.
- Tapping `Escape` or `Ctrl+Enter` exits chat mode, returns to passive mode, and focuses AoM.
- Focusing another app hides Agora; passive mode remains passive, while chat mode is suspended and restored when AoM is foreground again.
- Chat mode has a compact left tab column for Global, Friends, and Block/report.
- No large settings or admin screens in overlay.

## Moderation Plan

MVP moderation must be functional, not just data collection.

Required:

- Users can block another user.
- Users can report a user or message.
- Moderators can list open reports.
- Moderators can delete global/DM messages.
- Moderators can timeout/suspend users.
- Admins can ban users.
- Banned/suspended users cannot send messages.

Keep moderation simple and server-side. Client-side hiding is convenience only; enforcement belongs on the server.

## Privacy And Trust Documentation

Add a `SECURITY.md` or `docs/trust.md` before public beta.

Must document:

- What the client does to the AoM process.
- What the client never does to the AoM process.
- Exact account data stored.
- Exact local data stored.
- How Steam login works.
- How reports/moderation work.
- How automatic updates work.
- How to build from source.

Required trust statement:

```text
Agora uses read-only Windows APIs to detect Age of Mythology: Retold process and game-state information. It does not write game memory, inject code, automate input, bypass anti-cheat, or read gameplay data beyond the minimum state needed to decide whether the chat overlay should be visible.
```

## Automatic Updates

Use signed/checksummed release artifacts.

Recommended first implementation:

- Publish Windows builds through GitHub Releases.
- Client checks for updates on startup.
- Client checks at most once per configured interval.
- Client verifies checksum/signature before replacing itself.
- Client restarts into the new version after update.
- Server exposes minimum supported client version at `GET /version`.
- WebSocket handshake rejects clients below minimum version with `minimum_version_required`.

This prevents incompatible old clients from staying connected.

## VPS Deployment With Docker Compose

Services:

```text
caddy or nginx
agora-server
postgres
```

Recommended `docker-compose.yml` shape:

```text
services:
  proxy:
    image: caddy:latest
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy_data:/data
      - caddy_config:/config
    depends_on:
      - server

  server:
    image: ghcr.io/<owner>/agora-server:<version>
    environment:
      PGPASSWORD: ${POSTGRES_PASSWORD}
      DATABASE_URL: postgres://agora@postgres:5432/agora
      AGORA_PUBLIC_URL: https://<domain>
      AGORA_SESSION_SECRET: ${AGORA_SESSION_SECRET}
      STEAM_WEB_API_KEY: ${STEAM_WEB_API_KEY}
      AGORA_MIN_CLIENT_VERSION: ${AGORA_MIN_CLIENT_VERSION}
    depends_on:
      - postgres
    restart: unless-stopped

  postgres:
    image: postgres:17
    environment:
      POSTGRES_DB: agora
      POSTGRES_USER: agora
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes:
      - postgres_data:/var/lib/postgresql/data
    restart: unless-stopped

volumes:
  postgres_data:
  caddy_data:
  caddy_config:
```

Deployment requirements:

- TLS through Caddy or nginx + Let's Encrypt.
- `.env` file on VPS for secrets.
- Database migrations run during deployment or server startup.
- Daily PostgreSQL backups.
- Log rotation.
- Firewall open only for SSH, HTTP, HTTPS.
- SSH key auth only.

## Development Milestones

### Milestone 1: Workspace And Skeleton

Deliverables:

- Rust workspace.
- `agora-client`, `agora-server`, `agora-common` crates.
- Basic Dioxus desktop window.
- Basic Axum server with `/health` and `/version`.
- PostgreSQL connection and migrations.
- CI for `cargo fmt`, `cargo clippy`, and tests.

Acceptance criteria:

- Client launches.
- Server starts locally.
- Server can connect to local Postgres.
- CI passes.

### Milestone 2: Steam Login

Deliverables:

- Steam OpenID device/browser login flow.
- User creation/linking by SteamID64.
- Session issuing, refresh, logout.
- Local token storage through Windows Credential Manager.

Acceptance criteria:

- User can sign in with Steam.
- User persists across app restart.
- User can sign out.
- Server stores only minimal Steam profile data.

### Milestone 3: WebSocket, Global Chat, Presence

Deliverables:

- Authenticated WebSocket connection.
- Global message send/broadcast.
- Global history loading.
- Heartbeats.
- Online/LFG/in-game presence counts.

Acceptance criteria:

- Two clients can chat in global chat.
- Presence counts update correctly.
- Rate limits prevent obvious spam.
- Old/invalid sessions are rejected.

### Milestone 4: DMs, Friends, Blocks

Deliverables:

- DM threads.
- DM message history.
- Friend request/accept/remove.
- Block/unblock.
- Block enforcement for global display and DMs.

Acceptance criteria:

- Users can DM each other.
- Users can manage friends.
- Blocking hides messages and prevents unwanted DMs.

### Milestone 5: AoM Detection And Overlay

Deliverables:

- Process detection for `AoMRT_s.exe`.
- Main AoM window detection and bounds tracking.
- Read-only game-state probe limited to overlay visibility.
- Detector status UI.
- Interactive overlay shell.
- Automatic hide during `InMatch`.
- Automatic show during `MultiplayerMenu`, `Lobby`, and `PostGame`.

Acceptance criteria:

- Standalone app appears when AoM is not running.
- Overlay appears in eligible AoM multiplayer/menu states.
- Overlay hides during matches.
- Overlay reappears after match completion.
- If detection fails, overlay hides safely and standalone chat remains usable.

### Milestone 6: Reports And Moderation

Deliverables:

- Report user/message flow.
- Moderator report list.
- Delete message action.
- Timeout/suspend/ban actions.
- Audit log through `moderation_actions`.

Acceptance criteria:

- Reports are visible to moderators.
- Moderators can take actions.
- Suspended/banned users cannot send messages.
- Actions are stored with moderator, target, reason, and timestamp.

### Milestone 7: Auto-Update And Compatibility Gate

Deliverables:

- Windows release build pipeline.
- Signed/checksummed release artifact.
- Client startup update check.
- Server minimum client version.
- WebSocket rejection for old clients.

Acceptance criteria:

- Client updates from a release.
- Client restarts after update.
- Server blocks incompatible clients with a clear message.

### Milestone 8: VPS Beta Deployment

Deliverables:

- Dockerfile for server.
- Docker Compose deployment.
- Caddy/nginx reverse proxy.
- PostgreSQL volume.
- Backups.
- Production `.env` template.
- Basic runbook.

Acceptance criteria:

- Server runs on VPS behind HTTPS.
- Client can log in and connect to production.
- WebSocket works through the proxy.
- Database survives container restarts.
- Backups can be restored in a test environment.

## First Implementation Order

1. Initialize workspace and server/client skeleton.
2. Add database migrations.
3. Implement Steam login.
4. Implement sessions.
5. Implement authenticated WebSocket.
6. Implement global chat and presence.
7. Implement DMs/friends/blocks.
8. Implement AoM detector.
9. Implement overlay mode.
10. Implement reports/moderation.
11. Implement auto-updates.
12. Deploy beta on VPS.

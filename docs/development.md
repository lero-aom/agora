# Development

Agora has a local fixture environment for development and integration testing. It uses disposable values, binds public development ports to loopback, and cannot be enabled for a non-loopback public URL.

## Prerequisites

- Rust stable with the Windows MSVC toolchain to run the desktop client.
- Docker Engine or Docker Desktop with Docker Compose v2.
- PowerShell 7 for the release and maintenance scripts.

## Start the Local Stack

```powershell
docker compose --env-file .env.example up --build
```

The stack is available only on the local machine. The server uses fake fixture identities instead of Steam or Microsoft sign-in.

Run the Windows client in a second terminal:

```powershell
cargo run -p agora-client
```

The client defaults to `http://localhost`. Set `AGORA_LOCAL_DEV_WINDOW=true` before starting it to use a normal 900x560 desktop window without the tray, game watcher, or overlay hotkey behavior.

## Fixture Accounts

`.env.example` defines these local-only accounts through `AGORA_DEV_LOGIN_ACCOUNTS`:

| Account | Role |
| --- | --- |
| `alice`, `bob`, `reporter`, `target` | Regular user |
| `moderator` | Review reports, delete global messages, suspend regular users |
| `admin` | Moderator permissions plus ban and unban |
| `owner` | May act on admins and lower roles |

The client shows a Local fixture field only for loopback server URLs. Sign in with an account ID, perform an action, then sign out before using another account. Client credentials for loopback origins are not persisted, so restarting the client requires signing in again. Set `AGORA_DEV_ACCOUNT_ID` to choose an initial account other than `alice`.

For moderation testing, use the local staff console at `http://localhost/staff`. The fixture UI exposes the current local access token for that purpose only. Never copy fixture configuration, tokens, or secrets to a public deployment.

## Validation

Before opening a pull request, run:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace
docker compose --env-file .env.example config --quiet
```

See [Contributing](../CONTRIBUTING.md) for review expectations and [Reference Deployment](deployment.md) for production configuration boundaries.

# Agora

## Local Fixture Mode

Local Docker development enables fake identities instead of remote Steam or Microsoft login. The server only permits this mode when `AGORA_PUBLIC_URL` is loopback, and the development Compose ports bind to `127.0.0.1`.

Start the local stack with:

```powershell
docker compose --env-file .env.example up --build
```

The example file is deliberately local-only and uses disposable development secrets. An `.env` file with equivalent local values can replace the `--env-file` option for a long-lived local instance.

`.env.example` configures these fixture accounts through `AGORA_DEV_LOGIN_ACCOUNTS`:

- `alice`, `bob`, `reporter`, and `target` are regular users.
- `moderator` can review reports, delete messages, and suspend regular users.
- `admin` can also ban and unban users.
- `owner` can act on admins as well as lower roles.

The Windows client shows a **Local fixture** field whenever its server URL is `localhost`, `127.0.0.1`, or `::1`. Enter an account ID, sign in, perform an action, sign off, and sign in as the next fixture. Local sessions stay in memory, so a restart always begins unsigned in. Set `AGORA_DEV_ACCOUNT_ID` before starting the client to choose its initial fixture instead of `alice`.

Set `AGORA_LOCAL_DEV_WINDOW=true` before starting `agora-client.exe` to open the same UI as a normal standalone 900x560 desktop window. This mode only activates for loopback server URLs, does not need AoM to be running, and disables tray, game-watcher, and `Ctrl+Enter` overlay behavior.

For moderation tests, sign in as `moderator`, `admin`, or `owner`. The Local fixture section exposes the current access token and opens the existing staff console at `http://localhost/staff`; paste the token into that console.

Example workflow:

1. Sign in as `alice` and send a friend request or global message.
2. Sign in as `bob` to accept, block, or report it.
3. Sign in as a staff fixture to review the report and exercise the permitted moderation actions.

Do not enable local fixture mode or copy its fixture configuration to a public deployment.

## Remote Sign-In

Public servers offer Steam and, when configured, Microsoft personal-account sign-in from the client toolbar. Microsoft identities are separate Agora accounts from Steam identities, even when the same person controls both. Agora does not link those accounts or verify Xbox, Microsoft Store, or Game Pass ownership.

Microsoft app registration, redirect URI, and credential configuration are documented in [docs/deployment.md](docs/deployment.md).

## Deployment and Operations

Production deployment, image-digest pinning, TLS, and update procedures are documented in [docs/deployment.md](docs/deployment.md). Database backup, restore, and incident procedures are in [docs/operations.md](docs/operations.md). Release packaging and server-image publishing are covered by [docs/releasing.md](docs/releasing.md).

## Windows Updates

Official Windows builds embed an HTTPS update base URL and an Ed25519 public key at compile time. They never use `AGORA_SERVER_URL` to discover update metadata. On startup, non-local clients verify the detached signature over the raw manifest before parsing it, then verify the streamed executable SHA-256 and size before replacing themselves. Loopback development servers skip update checks.

The first updater-capable build must be installed manually from a release. Later builds show an accessible update banner with retry and manual-release actions; selecting **Update and restart** stages the verified executable beside Agora and uses a copied helper to atomically replace it after exit. If staging or replacement cannot complete, the current executable and recovery backup remain available and the release page is the safe fallback.

## License and Source

Agora is licensed under [AGPL-3.0-only](LICENSE). Every Windows release archive includes the license, a source notice with the exact commit, and a complete matching source archive. Operators running modified network versions must make the corresponding source available to their users as required by AGPLv3 section 13.

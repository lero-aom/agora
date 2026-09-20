# Production Deployment

This runbook deploys the Compose stack in `docker-compose.prod.yml`: Caddy is the only public service, the Agora server is reachable only through Caddy, and PostgreSQL has no host port. The separate development Compose file binds PostgreSQL only to `127.0.0.1` for host-side integration tests.

## Prerequisites

- An x86_64 (amd64) Linux host with Docker Engine, Docker Compose v2, and PowerShell 7 for the maintenance scripts. Production Compose explicitly requests `linux/amd64` for every service; ARM64 deployment is unsupported.
- A DNS A/AAAA record for the production hostname pointing to the host.
- Firewall rules allowing only SSH administration and TCP 80/443. Do not publish TCP 5432 or the server's port 8080.
- A protected `.env` file based on `.env.production.example`, for example with mode `600` on Linux.

Set `AGORA_SITE_ADDRESS` to a public DNS hostname and `AGORA_PUBLIC_URL` to its matching `https://` URL. Caddy obtains and renews certificates automatically; port 80 must remain reachable for normal ACME validation and redirects.

## Microsoft personal accounts

Microsoft sign-in is optional and accepts personal Microsoft accounts only. It uses Microsoft's `consumers` authority, creates a separate Agora identity from any Steam identity, and does not verify Xbox, Microsoft Store, or Game Pass ownership.

1. In Microsoft Entra admin center, register an application for **Personal Microsoft accounts only**.
2. Add a **Web** platform redirect URI matching `AGORA_PUBLIC_URL` exactly with `/auth/microsoft/callback` appended, such as `https://chat.example.com/auth/microsoft/callback`.
3. Create a client secret and retain its **Value**. The secret value is shown only once.
4. Set both values in the protected production `.env` file:

```dotenv
AGORA_MICROSOFT_CLIENT_ID=<Application-client-ID>
AGORA_MICROSOFT_CLIENT_SECRET=<client-secret-value>
```

Set neither variable to leave Microsoft sign-in disabled; setting only one makes the server fail closed at startup. Do not grant Microsoft Graph, Xbox, or other API permissions. Agora requests only `openid profile`, validates the returned ID token, and stores the OIDC subject and display name without requesting email access.

## Image pinning

The Compose defaults use reviewed tag-and-digest references for Caddy and PostgreSQL. The server is already required as a repository plus immutable digest. Keep all three immutable in production:

```dotenv
AGORA_SERVER_IMAGE_REPOSITORY=ghcr.io/lero-aom/agora-server
AGORA_SERVER_IMAGE_DIGEST=sha256:<published-server-digest>
AGORA_CADDY_IMAGE=caddy:2.10.2-alpine@sha256:<reviewed-caddy-index-digest>
AGORA_POSTGRES_IMAGE=postgres:17.5-bookworm@sha256:<reviewed-postgres-index-digest>
```

Do not deploy `latest`. Before changing an image, review its release notes, inspect its `linux/amd64` manifest, update the value in `.env`, and test the update in a non-production environment:

```powershell
docker buildx imagetools inspect caddy:2.10.2-alpine
docker buildx imagetools inspect postgres:17.5-bookworm
```

The server Dockerfile pins its Rust builder and Debian runtime inputs and accepts `RUST_IMAGE` and `RUNTIME_IMAGE` build arguments for deliberate digest refreshes. `scripts/publish-server-image.ps1` publishes only `linux/amd64`, records the source revision as OCI metadata, and emits an SBOM and provenance attestation when the registry supports Buildx attestations.

## First deployment

1. Clone the reviewed release revision on the host and create `.env` with unique, high-entropy `POSTGRES_PASSWORD` and `AGORA_SESSION_SECRET` values.
2. Set the server image digest produced by the publish script. Set reviewed Caddy and PostgreSQL digest references as described above.
3. Do not set `AGORA_PROXY_SUBNET`, `AGORA_PROXY_IP`, or `AGORA_TRUSTED_PROXY_CIDRS` in `.env`. Compose owns one anchored Caddy address, passes that exact address to the server as its only trusted proxy, and allocates dynamic proxy endpoints from a non-overlapping range. Caddy can therefore start after the server without an address collision. Do not set the retired `AGORA_TRUST_PROXY_HEADERS` flag.
4. Validate interpolation without starting containers:

```powershell
docker compose --env-file .env -f docker-compose.prod.yml config --quiet
```

5. Pull and start the stack. `--wait` waits for PostgreSQL, the server's database-aware `/health` endpoint, and Caddy's own proxy-listener health check:

```powershell
docker compose --env-file .env -f docker-compose.prod.yml pull
docker compose --env-file .env -f docker-compose.prod.yml up -d --wait
docker compose --env-file .env -f docker-compose.prod.yml ps
```

6. Verify the public endpoint from outside the host:

```powershell
curl.exe --fail --retry 12 https://chat.example.com/health
```

The server runs as UID/GID `10001`, has a read-only root filesystem and only a small writable `/tmp`, and drops Linux capabilities. Caddy-to-server and server-to-PostgreSQL traffic use separate internal networks, so Caddy cannot reach PostgreSQL directly. The server also has a separate egress network because Steam OpenID, the Steam Web API, and Microsoft OpenID Connect require outbound access. Caddy retains only the bind capability needed for ports 80/443, and its writable certificate/config paths are named volumes.

## Proxy network

The internal proxy network reserves `172.30.0.2` for Caddy and excludes it from dynamic allocation. The server receives that same bare address in `AGORA_TRUSTED_PROXY_CIDRS`; the server interprets a bare IPv4 address as an exact `/32` trust entry. This is intentionally not an operator environment knob, preventing a broad or mismatched trust CIDR from being introduced by an `.env` edit.

If that private range conflicts with an existing Docker or routed network, make one reviewed change to both Compose files that keeps the anchored Caddy address inside the subnet and outside the dynamic range. Do not reintroduce independent `.env` overrides for the address or trusted proxy value.

If a reviewed Compose update changes the proxy network definition on an existing deployment, recreate the Compose-managed networks before starting it. This does not remove named volumes when `-v` is omitted:

```powershell
docker compose --env-file .env -f docker-compose.prod.yml down
docker compose --env-file .env -f docker-compose.prod.yml up -d --wait
```

## Updating and rollback

Create and verify a database backup before every server or PostgreSQL update. Then change only the reviewed digest values in `.env`, validate Compose, and start the updated stack:

```powershell
docker compose --env-file .env -f docker-compose.prod.yml config --quiet
docker compose --env-file .env -f docker-compose.prod.yml pull
docker compose --env-file .env -f docker-compose.prod.yml up -d --wait
```

Confirm `/health`, login, and WebSocket behavior before considering the update complete. To roll back a server image, restore the prior server digest and run the same command. Do not assume a binary rollback can read a database after forward migrations; restore the tested pre-update database backup if required.

## TLS and source access

`Caddyfile.production` disables the Caddy admin API and adds HSTS, CSP, framing, MIME-sniffing, referrer, permissions, and cross-origin protections in addition to the application controls. Keep the production Caddyfile mounted read-only and do not enable development-login proxy headers in production.

Agora is licensed under AGPL-3.0-only. Operators of a modified network service must offer users the complete corresponding source for the version they run, as required by AGPLv3 section 13. Publish the exact release source archive or a stable URL to the exact source revision alongside the service's user-facing documentation. Each client release archive already contains `LICENSE`, `SOURCE.md`, and its matching complete source archive.

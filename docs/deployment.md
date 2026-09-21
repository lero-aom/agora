# Reference Deployment

`docker-compose.prod.yml`, `Caddyfile.production`, and `.env.production.example` are the reviewed reference configuration for a single Linux amd64 Agora service. They are kept with the source so infrastructure changes are visible, reviewable, and validated in CI.

This is not a hosted-service access guide or a promise of general self-hosting support. It documents the security properties required by the checked-in reference configuration. Real host inventory, secrets, backup locations, and deployment records belong outside Git.

## Required Boundaries

- Caddy is the only publicly exposed service. PostgreSQL and the Agora server have no host ports.
- Caddy terminates TLS, has its admin API disabled, and mounts its configuration read-only.
- The server accepts forwarded client information only from Caddy's fixed internal address.
- Caddy-to-server and server-to-PostgreSQL traffic use separate internal networks. Only the server has an egress network for identity providers.
- The server runs as UID/GID `10001`, has a read-only root filesystem, a small writable `/tmp`, no Linux capabilities, and bounded CPU, memory, and PID limits.
- Production images use immutable digests. Do not deploy mutable `latest` tags.

## Configuration

Create a protected `.env` from `.env.production.example`. It must contain unique, high-entropy values for `POSTGRES_PASSWORD` and `AGORA_SESSION_SECRET`, the reviewed server image digest, and the public HTTPS origin. Protect it with host-appropriate permissions, such as mode `600` on Linux, and never commit it.

`AGORA_SITE_ADDRESS` must be the public DNS hostname and `AGORA_PUBLIC_URL` its matching `https://` origin. Keep TCP 80 and 443 available to Caddy for normal certificate issuance and renewal. Do not publish TCP 5432 or the server's port 8080.

Microsoft personal-account sign-in is optional. Set both `AGORA_MICROSOFT_CLIENT_ID` and `AGORA_MICROSOFT_CLIENT_SECRET` to enable it, or leave both empty. The server fails closed when only one is set. Use a Personal Microsoft accounts-only application, register the exact Web redirect URI `${AGORA_PUBLIC_URL}/auth/microsoft/callback`, and grant no Microsoft Graph, Xbox, Store, or Game Pass permissions.

## Deployment Checks

Run configuration validation before starting containers:

```powershell
docker compose --env-file .env -f docker-compose.prod.yml config --quiet
```

Use a reviewed source tag or commit and the immutable server digest produced by `scripts/publish-server-image.ps1`. Before changing the server or PostgreSQL image, create and verify a backup, validate configuration, deploy, and check the public `/health` endpoint, login, and WebSocket behavior.

```powershell
docker compose --env-file .env -f docker-compose.prod.yml pull
docker compose --env-file .env -f docker-compose.prod.yml up -d --wait
docker compose --env-file .env -f docker-compose.prod.yml ps
```

Never use `docker compose down -v` for a normal update. It removes named volumes.

## Backup and Recovery

Use `scripts/backup-postgres.ps1` from the reviewed deployment revision to create a PostgreSQL custom-format dump and SHA-256 sidecar without exposing PostgreSQL to the host network. Store both files in encrypted, access-controlled, off-host storage.

```powershell
pwsh -NoProfile -File scripts/backup-postgres.ps1 -EnvFile .env -OutputDirectory /srv/agora-backups
```

Test the chosen backup on an isolated environment first. In an incident, stop the server, restore with an explicit acknowledgement, then start the stack and verify health, login, and a representative WebSocket connection:

```powershell
docker compose --env-file .env -f docker-compose.prod.yml stop server
pwsh -NoProfile -File scripts/restore-postgres.ps1 -EnvFile .env -Backup /srv/agora-backups/agora-postgres-YYYYMMDDTHHMMSSZ.dump -Force
docker compose --env-file .env -f docker-compose.prod.yml up -d --wait server proxy
```

`scripts/restore-postgres.ps1` validates a checksum sidecar by default, refuses to run while the server is active, and supports `-WhatIf` for local preflight. Test restores after schema changes and on a regular schedule.

## Source Availability

Agora is licensed under AGPL-3.0-only. Operators of a modified network service must make the corresponding source available to users as required by AGPLv3 section 13. Official client archives include `LICENSE`, `SOURCE.md`, and a matching source archive.

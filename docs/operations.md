# Operations Runbook

## Routine checks

Use the production Compose file and protected `.env` file for every command:

```powershell
docker compose --env-file .env -f docker-compose.prod.yml ps
docker compose --env-file .env -f docker-compose.prod.yml logs --tail 200 server
docker compose --env-file .env -f docker-compose.prod.yml logs --tail 200 proxy
```

The PostgreSQL health check confirms database readiness. The server health check calls its database-aware `/health` endpoint. Caddy waits for that server health check, actively checks its backend, and probes its own unexposed proxy listener. The external `/health` check below remains required to validate real ingress. An unhealthy stack should be investigated rather than bypassed with manual restarts.

Resource defaults are intentionally conservative and can be tuned in `.env` with `AGORA_CADDY_*`, `AGORA_SERVER_*`, and `AGORA_POSTGRES_*` CPU, memory, and PID-limit variables. Monitor actual steady-state use before increasing limits. Do not remove the server's read-only filesystem, capability drop, or split internal proxy/database networks merely to diagnose an issue; inspect logs first.

## Backups

Run backups from the checked-out deployment revision. The script creates a PostgreSQL custom-format dump and an ASCII SHA-256 sidecar without exposing the database on the host network:

```powershell
pwsh -NoProfile -File scripts/backup-postgres.ps1 -EnvFile .env -OutputDirectory /srv/agora-backups
```

The default output location is `~/AgoraBackups`; use an encrypted directory outside the repository in production. Transfer both the `.dump` and `.dump.sha256` files to encrypted, access-controlled off-host storage. Retain backups according to the service's recovery objectives and test a restore at least quarterly and after schema changes.

Database dumps do not contain deployment secrets. Keep an independently protected record of `.env` secrets and, where recovery-time objectives require it, back up the persistent Caddy data/config volumes as part of the host's encrypted volume backup process. Never put secrets or dumps in Git, release archives, or issue trackers.

## Restore

Restoring replaces database objects. Test the selected backup on a separate host first. During a production incident:

1. Confirm the backup checksum and the intended recovery point.
2. Stop the server to prevent writes; leave PostgreSQL running:

```powershell
docker compose --env-file .env -f docker-compose.prod.yml stop server
```

3. Restore with an explicit destructive acknowledgement. The script requires and verifies the sidecar checksum, refuses to run while the server is running, validates the custom archive, restores in one transaction, and runs `SELECT 1` afterward:

```powershell
pwsh -NoProfile -File scripts/restore-postgres.ps1 -EnvFile .env -Backup /srv/agora-backups/agora-postgres-YYYYMMDDTHHMMSSZ.dump -Force
```

4. Start the application and wait for health checks:

```powershell
docker compose --env-file .env -f docker-compose.prod.yml up -d --wait server proxy
curl.exe --fail --retry 12 https://chat.example.com/health
```

5. Check logs, login, and a representative WebSocket connection before reopening normal operations.

The restore command has high confirmation impact. `-Force` is intentionally required before it can alter the database; do not suppress the PowerShell confirmation unless an approved, tested incident procedure requires automation. `-WhatIf -Force` stops after local file and checksum preflight, before any Docker command, container copy, or container mutation.

`-AllowUncheckedBackup` is only for an independently verified emergency or legacy backup that has no `.sha256` sidecar. It does not bypass a malformed or mismatching sidecar. Record the reason and independent verification in the incident log before using it.

## Incident notes

Record the deployed server digest, Compose revision, backup checksum, timestamps, and observed health/log output for every deployment or restore incident. Preserve logs and evidence while avoiding collection of access tokens, passwords, Steam credentials, or unredacted user content.

# Security Policy

## Supported versions

Security fixes are applied to the current release line and `main`. Deployments should use the latest reviewed release artifact and immutable server image digest.

## Reporting a vulnerability

Report vulnerabilities privately through [GitHub Security Advisories](https://github.com/lero-aom/agora/security/advisories/new). Include affected version or image digest, reproduction steps, impact, and any suggested mitigation. Do not include secrets, access tokens, database dumps, or personal data in the report.

Maintainers will acknowledge a report, assess impact, and coordinate a fix before public disclosure where possible. If private advisories are unavailable, contact the repository owner through GitHub and request a private reporting channel.

## Deployment expectations

Production deployments must keep PostgreSQL and the Agora server off public container ports, terminate public traffic at Caddy, use TLS, protect `.env` secrets, and retain tested backups. See [docs/deployment.md](docs/deployment.md) and [docs/operations.md](docs/operations.md).

## Scope notes

Agora's client uses read-only Windows APIs only to identify the Age of Mythology: Retold process, its visible window, focus, bounds, and minimized state. It does not read game memory or infer menu, lobby, match, or post-game state. It must not write game memory, inject code, bypass anti-cheat, automate input, or read gameplay data. Security reports affecting that boundary are in scope.

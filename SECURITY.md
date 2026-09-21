# Security Policy

## Supported Versions

Security fixes are applied to the current release line and `main`. Deployments should use the latest reviewed release artifact and immutable server image digest.

## Reporting a Vulnerability

Report vulnerabilities privately through [GitHub Security Advisories](https://github.com/lero-aom/agora/security/advisories/new). Include affected version or image digest, reproduction steps, impact, and any suggested mitigation. Do not include secrets, access tokens, database dumps, or personal data in the report.

Maintainers will acknowledge a report, assess impact, and coordinate a fix before public disclosure where possible. If private advisories are unavailable, contact the repository owner through GitHub and request a private reporting channel.

## Deployment Expectations

Production deployments must keep PostgreSQL and the Agora server off public container ports, terminate public traffic at Caddy, use TLS, protect `.env` secrets, and retain tested backups. See the [reference deployment](docs/deployment.md) for the source-coupled configuration and its security invariants.

## Scope Notes

Agora's client enumerates visible top-level windows and uses read-only Windows process and window APIs to identify Age of Mythology: Retold. After identifying the game, it tracks the matched window's visible state, focus, bounds, and minimized state. It does not read game memory or infer menu, lobby, match, or post-game state. It must not write game memory, inject code, bypass anti-cheat, automate input, or read gameplay data.

The client does register a user-triggered global hotkey and can return focus to the game after a user closes the overlay. It does not synthesize keyboard or mouse input. Reports affecting this boundary, authentication, session handling, moderation privacy, or signed updates are in scope. See [Trust and Privacy](docs/trust-and-privacy.md) for code-linked details.

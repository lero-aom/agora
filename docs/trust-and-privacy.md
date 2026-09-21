# Trust and Privacy

This document describes the security and privacy properties enforced by Agora's public source. It is a technical overview, not a legal privacy policy or a retention schedule.

## Game Integration Boundary

Agora is an external Windows companion application. Its game observer in [game.rs](../crates/agora-client/src/game.rs) enumerates visible top-level windows, reads candidate window titles and process image paths, then identifies the Age of Mythology: Retold window using limited Windows process and window queries. The overlay shell in [shell.rs](../crates/agora-client/src/shell.rs) uses the matched game window's identity, bounds, focus, and minimized state.

Agora does not read or write game memory, inject code, create remote threads, enable debug privileges, bypass anti-cheat, inspect menus or matches, or automate input. The client does register a user-triggered hotkey and can return focus to the game after a user closes the overlay; it does not synthesize keyboard or mouse input.

## Local Fixture Guardrails

Development login is restricted to loopback public URLs by [server configuration](../crates/agora-server/src/main.rs) and [authentication handling](../crates/agora-server/src/auth.rs). The local Compose stack binds ports to `127.0.0.1`, and Caddy supplies a development-only proxy token.

The desktop client enables fixture UI and standalone local-window behavior only for loopback origins. It does not persist loopback refresh credentials in Windows Credential Manager. See [Development](development.md) for the local environment.

## Accounts and Sessions

The client accepts only credential-free HTTP(S) origins and requires HTTPS for non-loopback servers. Its API client disables redirects before bearer or refresh credentials can be replayed to another origin.

The server stores opaque access and refresh token hashes rather than raw tokens. Refresh tokens rotate once, are bounded to a session family, and reuse revokes the family and active realtime access. Steam and Microsoft authentication are handled server-side; Microsoft OIDC validation checks issuer, audience, expiry, state, and nonce. The relevant implementation is in [auth.rs](../crates/agora-server/src/auth.rs), [server.rs](../crates/agora-client/src/server.rs), and [session.rs](../crates/agora-client/src/session.rs).

## Moderation and Privacy

Staff actions are authorized by server-side roles and target-rank rules in [moderation.rs](../crates/agora-server/src/moderation.rs). Suspensions and bans revoke sessions and disconnect active realtime access. Moderation audit records are database-enforced as append-only.

Blocks are enforced server-side for global chat, realtime delivery, and direct messages. Direct-message delivery rechecks canonical membership and bilateral block state. Reported direct-message bodies are intentionally not returned to staff; staff receive report metadata until a separate staff-DM policy is defined. See [visibility.rs](../crates/agora-server/src/visibility.rs), [chat.rs](../crates/agora-server/src/chat.rs), and [dm.rs](../crates/agora-server/src/dm.rs).

## Updates and Releases

Release builds compile an HTTPS update base URL and Ed25519 public key into the client. The updater in [update.rs](../crates/agora-client/src/update.rs) verifies the detached signature over the raw manifest before parsing it, validates the manifest fields, then streams and verifies the executable size and SHA-256 before staging it for replacement.

The signing tool and [release script](../scripts/release.ps1) verify the manifest, executable binding, archive checksums, and matching source package. See [Releasing](releasing.md) for the maintainer workflow.

## Stored Data Categories

Agora stores the minimum application data needed to operate accounts and chat features, including provider identity identifiers and display names, account/session records and token hashes, global and direct messages, relationship and block records, reports, and moderation audit records. The service configuration may also contain optional Steam and Microsoft credentials, but those values are not stored in source control or sent to the desktop client.

This document intentionally does not claim retention periods because no formal retention schedule has been adopted. It describes the categories handled by the implementation, not a promise about how long an operator retains them.

## Reporting a Concern

Report suspected vulnerabilities privately under the [Security Policy](../SECURITY.md). Include a clear reproduction, affected version, and impact, but never include access tokens, credentials, database dumps, or personal data.

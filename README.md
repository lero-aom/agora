<p align="center">
  <img src="assets/logo/logo.png" alt="Agora logo" width="112">
</p>

<h1 align="center">Agora</h1>

<p align="center">Unofficial Windows companion chat for Age of Mythology: Retold.</p>

<p align="center">
  <a href="https://github.com/lero-aom/agora/releases/latest">Download</a>
  | <a href="docs/trust-and-privacy.md">Trust and privacy</a>
  | <a href="CONTRIBUTING.md">Contribute</a>
  | <a href="SECURITY.md">Security</a>
</p>

[![CI](https://github.com/lero-aom/agora/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/lero-aom/agora/actions/workflows/ci.yml)

Agora provides global chat, direct messages, friends, blocking, reporting, moderation tools, and a Windows overlay designed for Age of Mythology: Retold players. The client can also run as a normal desktop window for local development.

Agora is an independent community project. It is not affiliated with or endorsed by the Age of Mythology rights holders.

## Download

Download the latest Windows release from [GitHub Releases](https://github.com/lero-aom/agora/releases/latest). Release archives include the executable, checksums, license, source notice, and matching source archive.

The first updater-capable version must be installed manually. Later releases verify a signed update manifest and the downloaded executable before replacing the installed client. See [releasing](docs/releasing.md) for the maintainer-side release process.

## Safety Boundary

Agora is an external companion application. To locate the game, it enumerates visible top-level windows and checks candidate window titles and process image paths. Once it identifies Age of Mythology: Retold, it uses the matched window's bounds, focus, and minimized state for overlay placement. It does not read or write game memory, inject code, bypass anti-cheat, automate input, or inspect game state.

The complete source-backed explanation covers game integration, local fixtures, account sessions, moderation privacy, updates, and stored data categories in [Trust and Privacy](docs/trust-and-privacy.md).

## Project Documentation

- [Development](docs/development.md) explains the local Docker fixture environment and validation commands.
- [Reference deployment](docs/deployment.md) explains the reviewed production configuration and its security invariants.
- [Releasing](docs/releasing.md) explains signed Windows packages and server image publication.
- [Security policy](SECURITY.md) explains private vulnerability reporting.

## License

Agora is licensed under [AGPL-3.0-only](LICENSE). Network operators running modified versions must make the corresponding source available to their users as required by AGPLv3 section 13.

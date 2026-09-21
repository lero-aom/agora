# Contributing to Agora

## Development Checks

Keep changes focused and do not commit secrets, generated release artifacts, local databases, or production environment files. Before opening a pull request, run:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace
docker compose --env-file .env.example config --quiet
```

Changes to Docker, Compose, Caddy, release scripts, or deployment documentation must preserve the corresponding CI validation. Test local fixture mode only with `.env.example`; it deliberately uses disposable secrets and must never be copied to a public deployment.

## Changes and Reviews

Describe user-visible behavior, security implications, and validation performed in the pull request. Keep migrations, server behavior, and client protocol changes separately reviewable when practical.

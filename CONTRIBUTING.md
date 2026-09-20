# Contributing to Agora

## Development checks

Keep changes focused and do not commit secrets, generated release artifacts, local databases, or production environment files. Before opening a pull request, run:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace
docker compose --env-file .env.example config --quiet
```

Changes to Docker, Compose, Caddy, release scripts, or operational documentation also need the deployment checks in CI to pass. Test local fixture mode only with `.env.example`; it deliberately uses disposable secrets and must never be copied to a public deployment.

## Changes and reviews

Describe user-visible behavior, security implications, and validation performed in the pull request. Keep migrations, server behavior, and client protocol changes separately reviewable when practical. Update the relevant deployment or operational documentation when a configuration variable, backup procedure, image, or release artifact changes.

## Releases

`scripts/release.ps1` and `scripts/publish-server-image.ps1` refuse a dirty Git worktree, including untracked files. Commit the exact reviewed revision first. See [docs/releasing.md](docs/releasing.md) for the complete release process.

## Security reports

Do not file suspected vulnerabilities as public issues. Follow [SECURITY.md](SECURITY.md) instead.

# Development Guide

## Stack

- Tauri v2
- React
- TypeScript
- Vite
- Bun
- Rust

## Commands

```bash
bun install
bun run typecheck
bun run build
cargo test --manifest-path src-tauri/Cargo.toml
bun run tauri:build
```

## Release

Pushing a semantic version tag builds and publishes a GitHub Release:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The release workflow syncs the tag version into `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml` before building. For example, `v0.1.0` builds app version `0.1.0`.

Release assets are uploaded with versioned names:

- Windows: NSIS `.exe` and MSI `.msi`
- macOS: universal Apple Silicon + Intel `.dmg`

Release notes are generated from Conventional Commit subjects since the previous tag:

- `feat:` -> Features
- `fix:` -> Fixes
- `build:` / `ci:` -> Build
- `docs:` -> Documentation
- `chore:` / `refactor:` / `style:` -> Maintenance

Use `!` or a `BREAKING CHANGE:` footer for Breaking Changes, for example `feat!: change config format`.

macOS uses `APPLE_SIGNING_IDENTITY` from Repository secrets when it is configured. If the secret is empty, CI falls back to ad-hoc signing with `APPLE_SIGNING_IDENTITY=-`.

## Documentation Layout

- `.agents/`: Codex and agent-facing project guidance.
- `.codex/docs/`: Codex planning notes and implementation plans.
- `docs/`: project usage and development documentation.
- `README.md`: short project introduction and navigation.

## Frontend Organization

Third-party or service-provider logic belongs in `src/integrations/<provider>/`.

The Tako integration lives in `src/integrations/tako/`:

- `api.ts`: Tauri invoke wrapper.
- `auth.ts`: browser authorization and deep-link login flow.
- `providerConfig.ts`: provider metadata loading.
- `types.ts`: integration-facing types.
- `config/account-providers.json`: provider configuration.

## Backend Organization

Integration-specific Rust code belongs in focused modules. Tako backend commands and parsing live in `src-tauri/src/tako.rs`, while `src-tauri/src/lib.rs` keeps the Tauri app setup and invoke handler registration.

Keep the existing preview/apply/backup/restore command surface stable unless a larger migration is intentionally planned.

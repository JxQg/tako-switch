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

Provider defaults live in `src-tauri/config/providers.json`. Packaged builds load editable runtime overrides from `<install directory>/config/providers.json` and fall back to the built-in defaults when that file is missing or invalid.

## Backend Organization

`src-tauri/src/lib.rs` should stay as the Tauri app entrypoint: module declarations, plugin setup, deep-link wiring, and invoke handler registration only.

Backend code is split by responsibility:

- `commands.rs`: Tauri command handlers and deep-link parsing.
- `models.rs`: shared command DTOs returned to the frontend.
- `config_paths.rs`: user config paths, install directory, and provider config path.
- `backups.rs`: backup file creation, restore, and latest apply result persistence.
- `tools.rs`: local Codex / Claude Code command detection.
- `redaction.rs`: secret masking and preview redaction.
- `providers/`: service-provider config and remote provider APIs.
  - `types.rs`: runtime provider catalog schema and unified import input.
  - `loader.rs`: load `<install directory>/config/providers.json` with built-in fallback.
  - `validation.rs`: provider schema validation and unified input normalization.
  - `tako.rs`: Tako-specific login, identity, usage, and model-list APIs.
- `platforms/`: local client writers.
  - `codex.rs`: Codex TOML and user environment writer.
  - `claude.rs`: Claude Code `settings.json` writer.

Keep the existing preview/apply/backup/restore command surface stable unless a larger migration is intentionally planned.

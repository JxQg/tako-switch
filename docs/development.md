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

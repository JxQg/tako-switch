# User Guide

## What Tako Switch Does

Tako Switch helps configure Codex and Claude Code with a Tako gateway and ApiKey through a guided desktop flow.

## Basic Flow

1. Open Tako Switch.
2. Confirm Codex / Claude Code detection status on the Home tab.
3. Click `登录 Tako` to authorize in the system browser, or paste an ApiKey manually in the Import tab.
4. Review the generated config preview.
5. Click `应用配置` only after confirming the preview.
6. Use the result panel to restore the latest backup if needed.

## Tako Login

The login button opens `https://tako.shiroha.tech/app/authorize` in the system browser. After authorization, the browser returns to the desktop app through a `takoswitch://` deep link.

Tako Switch validates the returned key and fills the import form. It does not read browser cookies, scrape web pages, or silently write config files without preview confirmation.

## Config Targets

- Codex: `CODEX_HOME/config.toml` or `~/.codex/config.toml`
- Claude Code: `~/.claude/settings.json`

Codex uses the `TAKO_CODEX_API_KEY` user environment variable for the secret. Claude Code receives the token in its settings `env`.

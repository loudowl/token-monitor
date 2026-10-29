# Token Monitor

Floating always-on-top widget for AI token and plan usage across Cursor, Codex, Claude Code, ChatGPT, Claude.ai, GitHub Copilot, and Grok.

## Stack

- **Tauri 2** + **React** + **Vite** + **Tailwind**
- Local SQLite store; API keys in the macOS keychain
- Reads Codex and Claude Code session logs; polls Cursor and GitHub where credentials exist

## Development

```bash
npm install
npm run tauri dev
```

## Windows

| Window | Route | Purpose |
|--------|-------|---------|
| Widget | `/` | Always-on-top usage rings |
| Details | `/details` | Runs by cost, model, time |
| Settings | `/settings` | Plans, keys, provider toggles |

Double-click the widget or use the tray menu to open details. Gear icon opens settings.

## Data sources

- **Codex** — `~/.codex/sessions/**/rollout-*.jsonl`
- **Claude Code** — `~/.claude/projects/**/*.jsonl`
- **Cursor** — `state.vscdb` access token + Cursor dashboard APIs (unofficial)
- **Copilot** — GitHub billing API + PAT in settings
- **ChatGPT / Claude.ai / Grok** — manual plan % and optional vendor admin/API keys

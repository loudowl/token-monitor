# Token Monitor

Floating, always-on-top desktop widget that aggregates **AI token and plan usage** across the tools you actually use: **Cursor**, **Codex**, **Claude Code**, **ChatGPT**, **Claude.ai**, **GitHub Copilot**, and **Grok**.

Click the widget (or use the menu bar tray) to open a **details** view: runs sorted by estimated cost, with timestamps and models. Everything stays **local**—SQLite on disk, API keys in the **macOS Keychain**—no custom backend.

---

## Features

- **Always-on-top widget** — compact usage rings per provider (monthly % where available; Codex 5h / weekly windows where applicable)
- **Usage details** — filter by provider; sort by cost or recency; projected end-of-month spend
- **Settings** — enable/disable providers, billing-cycle anchor day, subscription plan catalog, manual usage % for consumer apps, optional API keys
- **Background sync** — watches Codex / Claude Code log files; polls remote APIs every ~2 minutes
- **Menu bar tray** — show/hide widget, open details or settings, sync now, quit
- **Launch at login** — optional autostart (Tauri autostart plugin)

---

## Supported providers

| Provider | How data is collected | Widget shows |
|----------|------------------------|--------------|
| **Codex** | Reads `~/.codex/sessions/**/rollout-*.jsonl` | 5h + weekly `used_percent` from logs; monthly token/cost estimates |
| **Claude Code** | Reads `~/.claude/projects/**/*.jsonl` | Token totals + estimated $ from local sessions |
| **Cursor** | Reads access token from Cursor `state.vscdb`; calls Cursor dashboard APIs | Monthly usage % when API works; otherwise estimates from synced events |
| **GitHub Copilot** | GitHub REST billing API + PAT in Settings | Premium request usage when PAT + username are set |
| **ChatGPT** | Optional OpenAI **admin** key (org costs API); else **manual** plan % | Plan-based monthly %; API key adds spend from org usage |
| **Claude.ai** | Optional Anthropic admin usage API; else manual plan % | Same pattern as ChatGPT |
| **Grok** | Optional xAI usage API; else manual plan % | Same pattern as ChatGPT |

Consumer web apps (ChatGPT, Claude.ai, Grok) do **not** expose a stable public “how much of my plan have I used?” API for desktop clients. Token Monitor supports **manual used %** (from each app’s account UI) plus optional **developer/admin API keys** where you have them.

---

## Requirements

### All platforms (from source)

- [Node.js](https://nodejs.org/) 20+ (LTS recommended)
- [Rust](https://www.rust-lang.org/tools/install) stable (`rustup` recommended)
- Platform tools from [Tauri prerequisites](https://tauri.app/start/prerequisites/)

### macOS (recommended / primary target)

- macOS 11+ (Apple Silicon or Intel)
- Xcode Command Line Tools: `xcode-select --install`
- **Cursor token path** and Keychain integration are implemented for macOS paths under `~/Library/Application Support/Cursor/...`

Linux and Windows builds are possible via Tauri, but Cursor DB paths and some Keychain behavior are macOS-oriented today. PRs to broaden platform support are welcome.

---

## Installation

### From source (development)

```bash
git clone https://github.com/loudowl/token-monitor.git
cd token-monitor
npm install
npm run tauri dev
```

The first `tauri dev` run compiles Rust dependencies and may take several minutes.

### Production build (local `.app` / installer)

```bash
npm run tauri build
```

Artifacts appear under `src-tauri/target/release/bundle/` (`.app` on macOS, `.msi` / `.exe` on Windows, etc.).

---

## First-run setup

1. **Launch** the app — the transparent widget appears on screen; a tray icon is added to the menu bar.
2. **Drag** the widget by its background to reposition it (position is saved).
3. Open **Settings** (gear on the widget or tray → Settings).

### Every provider

- Toggle providers under **Providers** so the widget only shows what you use.
- Set **Billing anchor day** (1–28) if your subscriptions renew on a specific day of the month (used for monthly rollups and projections).

### Codex & Claude Code

No keys required. Use Codex / Claude Code normally; Token Monitor ingests new session logs automatically (file watcher + periodic sync).

### Cursor

- Stay signed in to Cursor on the same machine.
- Token is read from Cursor’s local `state.vscdb` (read-only).
- Dashboard endpoints are **undocumented** and may change; if sync fails, you still get estimates from any events that were synced successfully.

### GitHub Copilot

In Settings:

1. Set **GitHub username** (e.g. `loudowl`).
2. Add a **GitHub PAT** (`github_pat`) with permission to read billing / premium request usage (fine-grained token scoped to your account as required by GitHub’s billing API).

### ChatGPT, Claude.ai, Grok (consumer)

1. Pick a **plan** from the catalog (prices/limits are planning aids, not live billing).
2. Optionally enter **Manual used %** from each service’s account or usage page.
3. Optionally add API keys for automated spend (see Settings labels):
   - `openai_admin_key` — OpenAI organization costs API
   - `anthropic_admin_key` — Anthropic organization usage
   - `xai_api_key` — xAI usage endpoint

Keys are stored in the **Keychain**, not in SQLite or the repo.

4. Click **Save & sync**.

---

## Using the app

| Window | Route | How to open |
|--------|-------|-------------|
| Widget | `/` | Default on launch; tray → Show widget |
| Details | `/details` | Click widget panel; tray → Usage details |
| Settings | `/settings` | Gear icon; tray → Settings |

**Tray menu:** Show widget · Usage details · Settings · Sync now · Quit

**Details view:** Tabs per provider (or All); sort by **Most expensive** or **Most recent**; summary cards show monthly/yearly % when configured.

---

## Architecture (brief)

```text
Local logs (Codex, Claude Code) ──► file watchers ──► SQLite (runs, quotas)
Cursor / GitHub / vendor APIs ────► HTTP pollers ──► SQLite
Settings & secrets ───────────────► Keychain + SQLite
SQLite ──► Tauri commands ──► React UI (widget / details / settings)
```

- Database: `~/Library/Application Support/token-monitor/token-monitor.db` (via `dirs::data_local_dir()`).
- Cost for log-only providers uses a **static model price table** in `src-tauri/src/pricing.rs` (estimates, not invoices).

---

## Privacy & security

- Session **content** is not uploaded; parsers read token counts, models, and metadata from local JSONL where possible.
- **API keys** live in the OS Keychain (`token-monitor` service name).
- Network calls go only to providers you configure (Cursor, GitHub, OpenAI, Anthropic, x.ai).
- Do not commit `.env` files or keys; this repo’s `.gitignore` excludes `node_modules`, `dist`, and `src-tauri/target`.

---

## Troubleshooting

### `npm run tauri dev` fails: Rust not found

Install Rust and reload your shell:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustc --version
```

### Tauri build error: `macos-private-api` feature

The project enables transparent always-on-top windows on macOS. If you see a feature mismatch error, ensure `src-tauri/Cargo.toml` includes:

```toml
tauri = { version = "2", features = ["tray-icon", "image-png", "macos-private-api"] }
```

And `tauri.conf.json` has `"macOSPrivateApi": true`.

### Widget is blank or all rings show `—`

1. Tray → **Sync now** (or wait for the next poll).
2. Confirm the provider is **enabled** in Settings.
3. For **Codex / Claude Code**, confirm log paths exist:
   - `~/.codex/sessions/`
   - `~/.claude/projects/`
4. For **Cursor**, confirm Cursor is installed and you are logged in; quit and reopen Token Monitor after signing in to Cursor.

### Cursor usage stays at `0%` or “API unavailable”

Cursor’s usage API is unofficial. The app falls back to:

- Summing **usage events** when the dashboard endpoint responds, and/or
- **Estimated** monthly spend vs. a default plan limit in Settings.

Check Cursor is running recent versions; API breakage may require a future app update.

### Copilot shows “needs PAT”

- Set **GitHub username** in Settings.
- Create a PAT with billing read access; paste in Settings → **GitHub PAT** and Save & sync.
- Verify with GitHub’s docs that `GET /users/{username}/settings/billing/premium_request/usage` is allowed for your token.

### ChatGPT / Claude.ai / Grok rings empty

Expected until you either:

- Enter **Manual used %** in Settings, or
- Add the optional **admin/API key** for that vendor.

Consumer apps do not expose plan quotas to third-party desktop tools without your manual input or org-level API access.

### Keychain / “access denied” when saving keys

Grant the app Keychain access when macOS prompts you. If you previously denied access, open **Keychain Access**, search for `token-monitor`, and adjust access for the app.

### High CPU or fan spin in dev

`tauri dev` runs Vite HMR and debug Rust builds. Use `npm run tauri build` and run the release bundle for day-to-day monitoring.

### Logs and resetting local data

- Quit the app.
- To reset all local usage and settings, delete:
  - `~/Library/Application Support/token-monitor/token-monitor.db`
- Keychain entries remain until removed in Keychain Access (service `token-monitor`).

### Port 1420 already in use

Vite is pinned to port **1420** for Tauri dev. Stop other processes using that port or set `TAURI_DEV_HOST` / adjust `vite.config.ts` and `tauri.conf.json` `devUrl` consistently.

---

## Development commands

| Command | Purpose |
|---------|---------|
| `npm run dev` | Vite only (browser UI without Tauri APIs) |
| `npm run tauri dev` | Full desktop app with hot reload |
| `npm run build` | Frontend production build |
| `npm run tauri build` | Bundled desktop app |

Frontend: React 19, Vite, Tailwind. Backend: Rust, `rusqlite`, `reqwest`, `notify`, `keyring`.

---

## Related projects

This app complements ideas from the [App Idea Orchestrator](https://github.com/loudowl) ecosystem:

- **[budget-burn](https://github.com/loudowl/budget-burn)** — iOS-focused API spend widgets (conceptual overlap; different platform).
- Plan names in Settings are inspired by subscription-tracking ideas (e.g. SubscriptionSentry-style catalogs).

Token Monitor is unique in combining **desktop overlay**, **local agent logs** (Codex / Claude Code), and **multi-vendor** quotas in one widget.

---

## License

No license file is included yet. All rights reserved unless otherwise noted by the repository owner.

---

## Contributing

Issues and PRs welcome at [github.com/loudowl/token-monitor](https://github.com/loudowl/token-monitor). Please do not commit secrets or personal usage databases.

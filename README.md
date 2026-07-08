# Token Meter

A tiny always-on-top desktop pet for watching Codex or Claude Code token activity.

## Download

Download Token Meter from the project's GitHub Releases page.

- macOS: download the `.dmg` asset.
- Windows: download the `.exe` installer. The `.msi` asset is provided as an alternate installer.

Current release builds are unsigned. macOS Gatekeeper or Windows SmartScreen may warn on first launch.

## What It Shows

- Motion speed: live visual token activity.
- 5H indicator: remaining short-window quota.
- Weekly indicator: remaining long-window quota.

The exact `tokens/min` value is calculated over a 60 second window. The bee animation uses a separate visual rate so it can wake immediately when a session starts writing activity before the next token event arrives.

## Interaction

Right-click the pet:

- `Reload`: fetch a fresh usage snapshot immediately.

## Run

```sh
npm install
npm run launch
```

The default provider mode is `auto`: the app picks the provider with the freshest local activity. You can force a provider:

```sh
USAGE_METER_PROVIDER=codex npm run launch
USAGE_METER_PROVIDER=claude npm run launch
```

## Claude Code Setup

Claude Code support uses Claude's [`statusLine`](https://docs.anthropic.com/en/docs/claude-code/statusline) JSON input as the bridge. Install it once:

```sh
npm run install:claude
```

Then restart Claude Code and run the app:

```sh
npm run launch
```

The installer updates:

```text
~/.claude/settings.json
```

If you already have a Claude Code status line, it is backed up and chained from:

```text
~/.token-meter/claude-statusline-backup.json
```

The bridge writes local meter state to:

```text
~/.token-meter/claude-status.json
```

No Claude API key is required. The desktop app only reads the local bridge state.

For an Agent installing this from GitHub, the whole setup is:

```sh
git clone <repo-url>
cd token-meter
npm install
npm run install:claude
npm run launch
```

For development:

```sh
npm run tauri:dev
```

Development workflow and skin-production notes are in:

```text
docs/DEVELOPMENT.md
```

That guide includes the living-skin production playbook: ImageGen alpha layers for character likeness, SVG/CSS for dynamic quota UI, and bucketed transform rigs for animation.

To expose a terminal command from this source checkout:

```sh
npm link
token-meter
```

The legacy `codex-usage-meter` command is kept as a compatibility alias during the rename.

## Build Desktop Apps

```sh
npm run tauri:build:mac
npm run tauri:build:windows
```

Build on the matching operating system. macOS bundles are produced on macOS. Windows installers are produced on Windows.

Local bundle outputs are created under:

```text
src-tauri/target/release/bundle/
```

## GitHub Releases

This repository includes a GitHub Actions workflow at:

```text
.github/workflows/release.yml
```

Push a version tag to build and upload macOS and Windows assets:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The workflow builds:

- macOS: `.dmg` and `.app`
- Windows: `.exe` NSIS installer and `.msi` installer

You can also run the workflow manually from GitHub Actions and provide a tag such as `v0.1.0`.

## Data Sources

### Codex

Quota data comes from the same account endpoint Codex Desktop uses:

```text
https://chatgpt.com/backend-api/wham/usage
```

The app reads the local Codex login token from:

```text
~/.codex/auth.json
```

Live token speed still comes from local Codex session JSONL files:

```text
~/.codex/sessions
```

On Windows, the app looks for `.codex` under the user's home directory using `HOME`, `USERPROFILE`, or `HOMEDRIVE` + `HOMEPATH`. If Codex is running inside WSL2 and stores data in the Linux home directory, launch Token Meter with `CODEX_HOME` pointing to the directory that contains `auth.json` and `sessions`.

If the account request fails, the app keeps the last successful account quota when available. It does not use local `token_count` rate-limit snapshots as authoritative quota data, because old sessions can report stale `0%` usage and make the meter flicker to full.

### Claude Code

Claude quota and usage data comes from the local `statusLine` payload that Claude Code passes to `scripts/claude-statusline.mjs`.

- 5H quota uses `rate_limits.five_hour.used_percentage`.
- Weekly quota uses `rate_limits.seven_day.used_percentage`.
- Live token events use `context_window.current_usage`.

The bridge de-duplicates repeated status line refreshes, so the same Claude response is not counted repeatedly.

# AgentPet for Windows

A desktop pet that floats on your screen and reacts in real time to your AI
coding agents (Claude Code, Codex, Gemini CLI, Cursor, opencode, Windsurf,
Antigravity, GitHub Copilot, Kiro CLI, Factory Droid, Pi, Grok Build, jcode).
Windows port of the macOS app, built with
[Tauri](https://tauri.app) so it stays small (~10 MB) and reuses the same pet
catalog + hook model.

> Status: feature-complete, builds on CI (`.msi` + NSIS `.exe`). Not yet
> validated across mixed-DPI monitors, and not code-signed (see
> [SmartScreen](#smartscreen-no-code-signing-cert) below).

## Install

WebView2 is preinstalled on Windows 10/11, so there are no other prerequisites.

### Scoop (recommended , no SmartScreen prompt)

```powershell
scoop bucket add agentpet https://github.com/ntd4996/agentpet
scoop install agentpet
```

Scoop downloads the portable build straight from the GitHub release, so the
browser/SmartScreen download warning never appears.

### winget

```powershell
winget install ntd4996.AgentPet
```

### Manual installer

Download `AgentPet_<version>_x64-setup.exe` from the
[releases](https://github.com/ntd4996/agentpet/releases) page. It installs
per-user (no admin/UAC prompt). See [SmartScreen](#smartscreen-no-code-signing-cert)
for the one-time "Run anyway" step.

## SmartScreen (no code-signing cert)

The installer is **not code-signed** (a cert costs money), so Windows SmartScreen
may show *"Windows protected your PC"* on first run of the downloaded `.exe`.
It is safe to bypass:

1. Click **More info**.
2. Click **Run anyway**.

To avoid the prompt entirely, install via **Scoop** or **winget** (above) , those
paths don't trigger SmartScreen.

## How it works

```
agent hook  ──(stdin JSON)──►  agentpet.exe hook --agent <kind>
                                      │  POST /event
                                      ▼
                          localhost:47628 (Rust listener in the running app)
                                      │  emit "agent-event"
                                      ▼
                          pet overlay window (Tauri webview, canvas sprite)
```

- The same binary doubles as the hook CLI: `agentpet hook --agent claude` reads
  the agent's hook payload on stdin and POSTs it to the running app. It always
  exits 0 so it never blocks an agent (Copilot PreToolUse is fail-closed).
- Standard hooks use JSON on stdin. jcode is the deliberate exception: its
  detached observer supplies `JCODE_HOOK_*` environment variables (stdin is
  `/dev/null`), and a missing event/session exits without blocking the agent.
- Hook configs are written to Windows paths (`%USERPROFILE%\.claude\settings.json`,
  `\.codex\hooks.json`, ...) , identical formats to the macOS app.
- Pets come from the public CDN (`pets.thenightwatcher.online/manifest.json`),
  rendered from the 8x9 spritesheet (8 frames per state row).
- The transparent overlay is **click-through**: only the pet's opaque rect
  captures the mouse, so the empty area lets clicks reach the apps below. Drag
  the pet to move it; its position is remembered across restarts.

## Features (parity with macOS)

- 13 agents, matching the Rust hook catalog.
- Pet picker (search / random) + "use your own spritesheet".
- Bubble customization: theme (dark/light/system), opacity, font size/family,
  themed phrases, per-agent custom messages, idle chatter toggle.
- Multi-agent bubble (shows every active session at once) with a live elapsed
  clock and per-tool live activity text (file being edited, command description).
- A jcode `turn_end` whose last assistant text is a question is surfaced as
  `waiting`; error turns are not treated as questions.
- Live preview in Settings, desktop notifications + chimes, autostart,
  auto-update (Tauri updater, minisign-signed).
- i18n: English / Tiếng Việt / 简体中文 with a runtime language switcher.

## Develop

```bash
cd windows
npm install
npm run tauri dev      # runs on macOS too (dev); click-through is Windows-only
```

## Build (on Windows)

```bash
npm install
npm run tauri build    # NSIS installer + MSI in src-tauri/target/release/bundle
```

## Build via CI (no Windows machine needed)

`.github/workflows/windows-build.yml` builds the installers on `windows-latest`:

- Run it manually from the **Actions** tab (workflow_dispatch) , the `.msi` and
  `.exe` are uploaded as artifacts.
- Push a tag like `win-v0.1.0` to also attach the installers, the portable
  Scoop zip, and the signed updater manifest to a GitHub release.

## Agents

| Agent          | Config file                                  | Notes |
|----------------|----------------------------------------------|-------|
| Claude Code    | `~/.claude/settings.json`                    | works once installed |
| Codex          | `~/.codex/hooks.json` + `config.toml`        | run `/hooks` → `t` once to trust |
| Gemini CLI     | `~/.gemini/settings.json`                    | |
| Cursor         | `~/.cursor/hooks.json`                        | |
| opencode       | `~/.config/opencode/plugin/agentpet.js`      | JS plugin |
| Windsurf       | `~/.codeium/windsurf/hooks.json`             | no "needs input" alerts |
| Antigravity    | `~/.gemini/config/hooks.json`                | no "needs input" alerts |
| GitHub Copilot | `~/.copilot/hooks/agentpet.json`             | Copilot CLI |
| Kiro CLI       | `~/.kiro/agents/default.json`                | hooks the default agent |
| Factory Droid  | `~/.factory/hooks.json`                      | Claude-compatible hooks |
| Pi             | `~/.pi/agent/extensions/agentpet.ts`         | extension; no needs-input hook |
| Grok Build     | `~/.grok/hooks/agentpet.json`                | Claude-compatible hooks |
| jcode          | `~/.jcode/config.toml`                       | observer lifecycle: `session_start`, `turn_start`, `post_tool`, `turn_end`, `session_end`; no blocking `pre_tool` |

The Settings window exposes the per-agent hook install toggle, including jcode.
jcode installation preserves unrelated TOML; a conflicting existing scalar hook
or array/table hooks representation fails closed without overwriting the file.

## Upstream parity scope

The `v1.17.0` / `v1.17.1` labels below refer to upstream macOS release scope,
not the Windows package version. This fork's Windows package is `0.1.13`.

- Bubble clipping/screen bounds, filtered-empty handling, and carousel support
  are implemented and tested natively on one 2560×1600 display at 150% scale,
  including genuine grow/shrink, four screen edges, dragging and per-window
  click-through in split mode. Real mixed-DPI/multi-monitor QA remains pending.
- Native occlusion/sleep behavior is not ported.
- The macOS quota-token fallback and macOS Settings fix are not Windows support
  promises.

### Native regression check (Windows, Node 22+)

```powershell
cd windows
cargo test --manifest-path src-tauri/Cargo.toml --lib
node --experimental-strip-types scripts/check-geometry.ts
node node_modules/@tauri-apps/cli/tauri.js build --debug --no-bundle --config src-tauri/qa.windows.json
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/native-qa.ps1 -MonitorBoundary
```

The native check briefly stops/restarts the installed AgentPet to own its hook
port, uses a separate debug-only profile and verifies real configuration hashes
remain unchanged. It moves the mouse and opens a temporary click-counter window;
do not interact with the desktop while it runs. CDP/profile overrides are compiled
out of release builds. Reports/screenshots are written to the printed output path.

## Publishing the package manifests

After a `win-v*` release is published:

```bash
node scripts/fill-package-hashes.mjs win-v0.1.0
```

This fills the version + SHA256 into `packaging/scoop/agentpet.json` and
`packaging/winget/*`. Then:

- **Scoop**: this repo doubles as the bucket , the manifest at
  `windows/packaging/scoop/agentpet.json`. (Point the bucket subdir or copy it to
  a `bucket/` folder as Scoop expects.)
- **winget**: submit `packaging/winget/*` as a PR to
  [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs).

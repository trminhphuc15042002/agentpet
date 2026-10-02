# Windows 0.1.12 validation

## 0.1.14: lightweight companion + working-text regression (2026-10-02)

- Native suite **passed** on the same single 2560×1600 display at 150% DPI:
  continuous working-text canvas-center drift **0.0078 physical px**, six
  working/done/idle cycles at **0.5 px** maximum native-center drift, and all
  grow/shrink, screen-edge, approval, display-mode, drag and click-through checks.
- Focus controls, pinning in list/carousel/compact, auto-unpin, waiting badge,
  diagnostic request/ack, Vietnamese voice preview and petting/expiry passed in
  real WebView2 windows. Diagnostic/petting actions left care data unchanged.
- 74 Rust library tests, TypeScript/Vite build, geometry checks and companion
  policy checks passed. Rust tests must run **inside `windows/src-tauri`** to
  discover this checkout's local linker config; invoking cargo from `windows`
  selected an incompatible global MinGW linker on this machine.
- Animated text now reserves its full message width; intermediate text no longer
  drives native resizes. Plain bubbles are cleared when returning to rows.
- QA additionally uses an isolated debug-only hook port (`47728`), so real coding
  hooks cannot contaminate fixture sessions, and a deterministic sprite for hit
  tests. Release builds still bind `47628`. Window/DOM samples reject in-flight
  coordinate changes; the new check covers the canvas center, not just HWND.
- Added the missing `core:window:allow-hide` capability for existing app windows:
  native control tests exposed rejected hide requests from the popover/settings.
- Real user configuration hashes were unchanged and the installed app restored.

Final local evidence: `%LOCALAPPDATA%/Temp/opencode/native-qa-20261002-113122/report.json`.
NSIS installer built at `windows/src-tauri/target/release/bundle/nsis/AgentPet_0.1.14_x64-setup.exe`
(5,806,012 bytes). It has not been installed over the user's running app or
published. No commit/tag/push was performed.
Mixed-monitor/DPI QA and comparative idle CPU/RSS measurements remain unverified.
Behavior/scope: [lightweight companion spec](specs/2026-10-02-lightweight-companion.md).

### Installed-release verification (2026-10-02)

Installed NSIS 0.1.14 into `%LOCALAPPDATA%/AgentPet`, then checked both executable
file version and Tauri runtime version: **0.1.14**. First silent install returned
zero but left the old executable; stopping the process, waiting for exit and
rerunning the installer replaced it successfully. Installer status alone is not
sufficient evidence of an upgrade.

Temporarily enabled localhost WebView2 CDP on port 9224 for the installed release,
without changing the app's persisted profile. Verified against a **real working
OpenCode session**, without injecting synthetic agent events:

- Focus start/end and visible Focus badge: pass.
- Pin/unpin the real session via popover controls: pass.
- Display diagnostics request/ack: pass; real OpenCode last-event time displayed.
- Waiting reminder setting on/off persisted correctly; restored original setting.
  Actual two-minute delivery was not triggered in the user's live session.
- All three Vietnamese voice previews: pass; original voice/language restored.
- Petting text displayed, menu hid, effect expired and real working state resumed.
- Canvas-center sampling: 169 samples, about **0.5 physical px** range during
  petting → real working transition on the installed release.

Closed the diagnostic instance and restarted normally. Installed process owns
127.0.0.1:47628; no listener remains on debug port 9224. The CDP helper now selects
only `/` or `/index.html` for `main`, excluding settings/popover pages.

## 0.1.13 follow-up: transient task-completion motion

Installed 0.1.12 showed a transient 127px horizontal jump during resize despite
settled-frame checks passing. Position and size must use a **single Win32
SetWindowPos** operation, not two Tauri setters, even on the UI thread. The
operation preserves z-order/focus and updates state only after success.

Native QA now samples intermediate frames during 24 deliberate resizes and six
working→done→idle cycles, including the final celebration expiry. On the attached
150% display the observed maximum center drift was 0.5px horizontally and 0px
vertically. DOM/HWND snapshots must agree on dimensions before computing the
canvas anchor; non-target pets are parked inside the work area for click tests.
Diagnostic debug.log appends during installed-app restart are excluded from the
configuration hash assertion; persisted configuration remains checked.

Validated on 2026-10-01 with the Windows GNU toolchain, one 2560×1600 display,
144 DPI (150% scale), and the packaged frontend in a real Tauri/WebView2 window.

## Passed

- Full native Rust library test executable: 74 tests, no loader failure.
- TypeScript build and geometry/carousel/filter regression checks.
- Five-row bubble genuinely grows and shrinks; pet anchor drift is at most
  two physical pixels. Visible content stays inside the viewport.
- Four work-area edges, including taskbar boundaries, without DOM clipping.
- List/carousel/compact, filtered-empty fallback, approval buttons.
- Native sprite dragging, transparent click-through, opaque sprite capture.
- Two split project windows: independent dragging and per-window hit testing.
- Hide/show, and unchanged hashes of the real AgentPet configuration.

## Fixes discovered during native validation

- Embed Tauri's existing Common Controls v6 resource in library test executables.
  Without it, importing `TaskDialogIndirect` failed at process startup.
  The additional native link is `cfg(test, windows)` only: the MSVC release
  linker rejects the duplicate VERSION resource a catch-all link would create.
- Build the desktop library as `rlib`; an unused GNU `cdylib` exceeded PE's
  export-count limit in debug builds.
- Apply native frame/anchor changes together on the UI thread. Separately queued
  position and size changes could look like user dragging; UI-thread mutex waits
  also stalled later commands.
- Check the actual viewport before suppressing a repeated resize request.
- Retain the unrounded pet anchor across programmatic resizes and refresh it
  after real moves; account for physical pixel rounding at 150% scale.
- Include device pixel ratio in hit-rectangle invalidation.

## Limits and reproduction

Real multiple-monitor/mixed-DPI transitions are **not verified**: only one
display was attached. Pure Rust tests cover alternate monitor origins/scales,
legacy position migration and removed-monitor fallback. Native occlusion/sleep
handling is not implemented. Windows installers are not Authenticode-signed.

See `windows/README.md` for the native check commands. It uses a debug-only,
isolated WebView/profile and temporarily stops/restores the installed app so
synthetic events cannot reach the wrong listener. The script checks actual
underlying window ownership before clicking, not merely unchanged counters.
QA reports and desktop screenshots remain local; they are not committed.

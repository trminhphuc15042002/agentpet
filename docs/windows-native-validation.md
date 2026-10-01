# Windows 0.1.12 validation

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

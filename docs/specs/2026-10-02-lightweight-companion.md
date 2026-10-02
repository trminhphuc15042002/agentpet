# Lightweight Windows companion updates

Scope: Windows/Tauri only. Implemented directly, without worker/subagent delegation.

## Priority and behavior

0. **Working-state motion regression:** animated text reserves its full final
   width. The typed substring is positioned inside that reservation, so erasing,
   typing and ellipsis frames do not repeatedly resize the native window. A plain
   line is cleared before returning to structured rows.
1. **Focus:** popover → Focus for 30 minutes / End focus. Timestamp-based override;
   leaves saved sound/notification settings untouched. Suppresses chatter,
   celebrations and done alerts, not care/history/usage recording. Waiting and
   approval information remain visible; a small badge indicates Focus.
2. **Pin:** ◇ next to a live session in the popover. One in-memory pin per owning
   pet window. Separates that session from its agent-kind group, avoiding duplicate
   counts. Carousel rotates remaining rows beneath it. Filters still apply.
   Pin clears on done, dismissal or pruning. Waiting count stays visible in the
   badge even if another session is pinned.
3. **Diagnostics:** Settings → Agent integrations. Refresh reads hook configuration;
   last real-event timestamp is separate from configured status. Timestamps persist
   at most once per 10 seconds per agent, bounded to 32 agents. Test display uses
   a dedicated request/ack event, not an agent event, and grants no XP. An ack
   proves the rendering pipeline is connected, not that a real agent hook works.
4. **Waiting reminder:** Settings → Notifications, opt-in. One grouped reminder
   after two continuous minutes waiting, using a single one-shot timer in the
   main window, also when the UI is hidden. No periodic poll added. Suppressed
   during Focus, with no replay. Sleep-delayed reminders more than a minute late
   and stale sessions are consumed silently. Existing notification and sound
   toggles remain independent.
5. **Vietnamese voices:** localized pools for Cozy/Tsundere/Chaotic, no redundant
   personality IDs. Preview voice in Settings. Custom messages retain precedence;
   live tool activity remains factual. Existing cooldowns/thresholds are unchanged.
6. **Petting:** popover → Pet your companion. Main pet celebrates for two seconds
   with a personality-specific line, then resolves the current real mood. No XP,
   no queued effects; waiting/approval interrupts it, Focus suppresses it. Left
   mouse dragging stays unchanged.

## Deliberate limits

- No new windows/webviews, art, dependencies, LLM calls or network integrations.
- Pins reset after restarting the app. Petting targets the main companion because
  the existing shared popover has no per-project invocation context.
- Diagnostics does not classify quiet agents as disconnected, repair hooks
  automatically, or claim a synthetic display test verifies real hooks.
- The existing five-minute stale-session policy is unchanged.
- Native testing requires a real Windows desktop; mixed-monitor/DPI behavior
  cannot be certified from a single-monitor run.

## Verification

```powershell
cd windows
npm run build
npm run check:companion
node --experimental-strip-types scripts/check-geometry.ts
Push-Location src-tauri
cargo test --lib
Pop-Location
node node_modules/@tauri-apps/cli/tauri.js build --debug --no-bundle --config src-tauri/qa.windows.json
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/native-qa.ps1 -MonitorBoundary -TimeoutSec 160
```

Native QA adds continuous DOM/native canvas-anchor sampling while a session is
working (including short → long → short animated messages), plus pinning across
all display modes, automatic unpin, Focus, waiting badge, diagnostic ack,
Vietnamese petting, effect expiry and unchanged care data for synthetic actions.
It uses an isolated profile and restores the installed app afterward.
The debug fixture listener uses port 47728 rather than consuming live hooks on
47628. Release builds ignore that override. Hit tests use a fixed sprite after
the initial real-catalog rendering check.

Baseline on 2026-10-02: old settled/native-center checks passed, but the new
working-text canvas-center check measured **29.5 physical pixels** of drift.
The initial text-reservation candidate measured **0 pixels** on the same check.
Final verification results belong in `docs/windows-native-validation.md`.

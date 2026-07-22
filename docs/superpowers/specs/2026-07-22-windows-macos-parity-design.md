# macOS → Windows Parity & OpenCode/OpenChamber Hook Overhaul Design

## 1. Overview & Objective

AgentPet for Windows (`windows/`) was initially created as a lightweight Tauri + Rust desktop pet overlay. While basic event pipelines and bubble layouts were ported, several core macOS features (`Sources/App` & `AgentPetCore`) were missing or incomplete. Additionally, the OpenCode & OpenChamber integration on Windows experienced session instability, frozen states, and session ID collisions.

This design specification establishes full feature parity between macOS and Windows, while re-engineering the OpenCode & OpenChamber hook integration to guarantee 100% session reliability.

---

## 2. OpenCode & OpenChamber Hook Reliability Overhaul

### Problem Analysis
1. **Incomplete Lifecycle Hooking**: The generated `agentpet.js` plugin currently hooks only `session.created` and `session.idle`. Critical intermediate events such as `tool.call`, `permission.ask` (waiting input), `step.start`, and `message.created` are ignored. As a result, AgentPet fails to display `waiting` states or activity text, and often freezes on `working`.
2. **Session ID Collisions**: The current `opencode_plugin` generates session IDs using `opencode:` + `directory`. When a user opens multiple tabs or sessions in OpenChamber for the same workspace directory, all sessions share the same ID. When one session finishes, it sends `session.idle` and terminates the pet state for all active sessions.
3. **Execution Runtime Fallback**: The plugin relies exclusively on `Bun.spawn` without fallback to Node.js `child_process`. On Windows environments where OpenCode/OpenChamber runs via Node.js or Electron without global `Bun`, hook execution throws uncaught exceptions or fails silently.

### Solution Design
1. **Multi-event Hooking**:
   Re-architect `opencode_plugin` in `src-tauri/src/hooks.rs` to intercept all OpenCode & OpenChamber events:
   - `session.created` / `session.start` → `working`
   - `tool.call` / `step.start` → `working` + update `activity` text (e.g., `Editing src/main.rs`, `Running tests`)
   - `permission.ask` / `question.asked` / `waiting_input` → `waiting` (pet enters waiting mode with orange flashing bubble and notification chime)
   - `session.idle` / `session.complete` → `done` (triggers 3s celebrate burst)
   - `session.destroyed` / `session.delete` → session pruned from store.
2. **Robust Session Identification**:
   - Construct unique session IDs: `opencode:<session_id>` if provided by the event, or fallback to `opencode:<dir_hash>_<session_counter>`.
   - Prevent multi-tab session wiping in OpenChamber.
3. **Dual Runtime Dispatcher**:
   - Generate JS code that attempts `node:child_process` `spawn` with `unref()` first, falling back to `Bun.spawn` if `node:child_process` is unavailable.
   - Properly escape Windows file paths (`C:\\...`) in generated scripts.

---

## 3. macOS Feature Port: Break Reminder & Continuous Active Clock

### Rust Backend (`src-tauri/src/break_clock.rs`)
- **Active Timer**: Monitors continuous agent activity. Ticks every second when any session is in `working` state.
- **Break Rules**:
  - `enabled`: boolean (default: `true`)
  - `work_interval_mins`: integer (default: `45` mins)
  - `break_duration_mins`: integer (default: `5` mins)
- **Break Notification & State**:
  - When `work_interval_mins` is reached:
    - Emits Tauri event `break-reminder-triggered`.
    - Shows Windows OS Toast Notification ("Time for a break! You've been coding for 45 minutes.").
    - Triggers pet break speech ("Time to stretch and grab a cup of coffee/water!").
    - Plays break chime sound.

### Frontend Integration (`windows/src/care.ts` & `settings.ts`)
- UI controls in Settings under General/Care tab for configuring break interval, break duration, and enabling/disabling notifications.

---

## 4. macOS Feature Port: Pet Care / Tamagotchi HUD

### Rust & Local Storage (`care.rs`)
- **Care Attributes**:
  - `health` (0–100, default 100)
  - `hunger` (0–100, default 80)
  - `happiness` (0–100, default 80)
  - `energy` (0–100, default 90)
  - `exp` / `level`
- **Dynamic Decay & Accrual Rules**:
  - Agent completing tasks (`done`) → +10 Happiness, +15 EXP.
  - Active coding without breaks → Hunger drops by 1 per 5 mins, Energy drops by 1 per 5 mins.
  - Feeding (`feed` command) → +25 Hunger, +5 Health.
  - Playing (`play` command) → +20 Happiness, -5 Energy.
  - Resting (`rest` command) → +30 Energy.
  - Cleaning (`clean` command) → +10 Health.
- **Persistence**: Saved to `%APPDATA%\AgentPet\care.json`.

### UI Integration (`windows/src/care.ts`, `settings.html`, `styles.css`)
- Full Tamagotchi HUD inside the Care tab:
  - Visual status progress bars for Health, Hunger, Happiness, Energy.
  - Interactive action buttons: Feed, Play, Rest, Clean.
  - Pet mood expression updates based on care levels.

---

## 5. macOS Feature Port: Token Usage & Analytics Dashboard

### Rust Backend (`src-tauri/src/usage.rs`)
- **Transcript Reader**:
  - Parses agent transcripts in `~/.claude/transcripts/`, `~/.codex/`, `~/.config/opencode/`, etc.
  - Extracts input tokens, output tokens, cache creation/read tokens.
- **Usage Store**:
  - Aggregates token usage per project path and per agent kind over 90 days.
  - Persists aggregated daily usage in `%APPDATA%\AgentPet\usage_history.json`.

### UI Integration (`windows/src/usage.ts` & Settings Usage Tab)
- Interactive table showing:
  - Project name & path.
  - Agent kind.
  - Total Token count (formatted, e.g. `12.5k`).
  - Total Sessions count.
- Filter by Agent and Project search field.

---

## 6. macOS Feature Port: Project-Specific Pets

### Resolver & Storage (`src-tauri/src/project_pets.rs`)
- Maps normalized absolute project paths (`E:/workspace/project-a`) to specific Pet IDs (`cat`, `dog`, `dragon`, etc.).
- Exposes IPC commands: `get_project_pets`, `set_project_pet(project, pet_id)`, `remove_project_pet(project)`.

### Event Integration
- When an `AgentEvent` arrives with `--project <path>`, the Rust state engine looks up the assigned pet for `<path>`.
- If an explicit project pet is set, AgentPet dynamically switches the sprite sheet and bindings for that session/overlay window.

### UI Integration (`windows/src/settings.ts` - Pet Tab)
- "Project Pets" configuration card displaying all active or historical projects with dropdown selectors to pick custom pets per project.

---

## 7. Testing & Verification Strategy

1. **Rust Unit Tests**:
   - `break_clock` interval & threshold logic tests.
   - `care` state decay, action bounds, and serialization tests.
   - `usage` token calculation & aggregation tests.
   - `hooks` string generation & path escaping tests.
2. **OpenCode & OpenChamber Integration Tests**:
   - Simulated JS plugin event triggers for single and multi-tab OpenChamber sessions.
   - Verify `working` -> `waiting` -> `done` -> `pruned` state flow without session freezes.
3. **Frontend Integration & Build Verification**:
   - Verify UI rendering in `settings.html` across all 7 tabs (General, Pet, Care, Usage, Bubble, History, About).
   - Run `npm run tauri build` to verify clean compilation of the Windows installer.

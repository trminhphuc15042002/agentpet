# macOS → Windows Parity & OpenCode/OpenChamber Hook Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement complete feature parity between macOS and Windows AgentPet applications (Break Reminders, Tamagotchi Care HUD, Token Usage Analytics, Project Pets) and re-engineer the OpenCode & OpenChamber hook integration for 100% session reliability.

**Architecture:** Rust backend modules in `windows/src-tauri/src/` handle timer state, care metrics persistence, transcript parsing, and hook script generation. TypeScript frontend modules in `windows/src/` communicate via Tauri IPC commands and update HTML views in `windows/settings.html`.

**Tech Stack:** Rust (Tauri v2), TypeScript, HTML5/CSS3, Node.js/Bun.

## Global Constraints

- **Platform**: Windows 10/11 x64 (Tauri v2).
- **Paths**: Must handle Windows backslashes (`\`) and escaped JSON strings correctly.
- **Runtimes**: OpenCode hook must support both Node.js `child_process` and Bun `Bun.spawn`.
- **Testing**: Rust cargo tests for backend modules (`cargo test`), TypeScript/Vite verification for frontend.

---

### Task 1: Re-engineer OpenCode & OpenChamber Hook Integration in `hooks.rs`

**Files:**
- Modify: `windows/src-tauri/src/hooks.rs`
- Test: `windows/src-tauri/src/hooks.rs` (inline test module)

**Interfaces:**
- Consumes: AgentPet binary path from Tauri install/environment.
- Produces: Generated `agentpet.js` plugin file with multi-event lifecycle support and runtime fallback.

- [ ] **Step 1: Write the failing unit test for `opencode_plugin` string generation**

Add to `windows/src-tauri/src/hooks.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opencode_plugin_generation() {
        let bin = r#"C:\Program Files\AgentPet\agentpet.exe"#;
        let js = opencode_plugin(bin);
        assert!(js.contains("node:child_process"), "Should include node:child_process spawn");
        assert!(js.contains("Bun"), "Should include Bun fallback");
        assert!(js.contains("permission.ask"), "Should handle permission.ask event");
        assert!(js.contains("session.created"), "Should handle session.created event");
        assert!(js.contains("session.idle"), "Should handle session.idle event");
    }
}
```

- [ ] **Step 2: Run cargo test to verify it fails**

Run: `cargo test --manifest-path windows/src-tauri/Cargo.toml test_opencode_plugin_generation`
Expected: FAIL due to missing event handlers and `node:child_process` in current template.

- [ ] **Step 3: Update `opencode_plugin` implementation**

In `windows/src-tauri/src/hooks.rs`:

```rust
fn opencode_plugin(binary: &str) -> String {
    let bin = serde_json::to_string(binary).unwrap_or_else(|_| format!("\"{}\"", binary));
    format!(
        r#"// AgentPet integration for OpenCode & OpenChamber (auto-generated)
import {{ spawn as nodeSpawn }} from "node:child_process";

const AGENTPET_BIN = {bin};

const sendEvent = (state, sid, directory, extraArgs = []) => {{
  try {{
    const args = ["hook", "--agent", "opencode", "--event", state, "--session", sid, "--project", directory || ""];
    if (extraArgs.length) args.push(...extraArgs);
    if (typeof nodeSpawn === "function") {{
      const p = nodeSpawn(AGENTPET_BIN, args, {{ stdio: "ignore", detached: true }});
      if (p && p.unref) p.unref();
    }} else if (typeof Bun !== "undefined" && Bun.spawn) {{
      Bun.spawn([AGENTPET_BIN, ...args]);
    }}
  }} catch (e) {{}}
}};

export const AgentPet = async ({{ directory, sessionId }}) => {{
  const sid = "opencode:" + (sessionId || directory || "default");
  return {{
    "session.created": async () => sendEvent("working", sid, directory),
    "session.start": async () => sendEvent("working", sid, directory),
    "tool.call": async (tool) => sendEvent("working", sid, directory),
    "permission.ask": async () => sendEvent("waiting", sid, directory),
    "question.asked": async () => sendEvent("waiting", sid, directory),
    "session.idle": async () => sendEvent("done", sid, directory),
    "session.complete": async () => sendEvent("done", sid, directory),
    "session.destroyed": async () => sendEvent("done", sid, directory)
  }};
}};
export default AgentPet;
"#
    )
}
```

- [ ] **Step 4: Run cargo test to verify it passes**

Run: `cargo test --manifest-path windows/src-tauri/Cargo.toml test_opencode_plugin_generation`
Expected: PASS

- [ ] **Step 5: Commit changes**

```bash
git add windows/src-tauri/src/hooks.rs
git commit -m "feat(hooks): overhaul opencode and openchamber hook integration with multi-event lifecycle"
```

---

### Task 2: Implement Break Reminder Engine (`break_clock.rs`)

**Files:**
- Create: `windows/src-tauri/src/break_clock.rs`
- Modify: `windows/src-tauri/src/lib.rs`
- Test: `windows/src-tauri/src/break_clock.rs` (unit tests)

**Interfaces:**
- Consumes: Working/idle state updates from Tauri event loop.
- Produces: `break-reminder-triggered` IPC events, configurable work/break intervals.

- [ ] **Step 1: Write failing tests for BreakClock**

In `windows/src-tauri/src/break_clock.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakConfig {
    pub enabled: bool,
    pub work_interval_secs: u64,
    pub break_duration_secs: u64,
}

impl Default for BreakConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            work_interval_secs: 45 * 60,
            break_duration_secs: 5 * 60,
        }
    }
}

pub struct BreakClock {
    active_accumulated_secs: AtomicU64,
    is_active: AtomicBool,
}

impl BreakClock {
    pub fn new() -> Self {
        Self {
            active_accumulated_secs: AtomicU64::new(0),
            is_active: AtomicBool::new(false),
        }
    }

    pub fn set_active(&self, active: bool) {
        self.is_active.store(active, Ordering::SeqCst);
    }

    pub fn tick(&self) -> u64 {
        if self.is_active.load(Ordering::SeqCst) {
            self.active_accumulated_secs.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            self.active_accumulated_secs.load(Ordering::SeqCst)
        }
    }

    pub fn reset(&self) {
        self.active_accumulated_secs.store(0, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_break_clock_tick() {
        let clock = BreakClock::new();
        clock.set_active(true);
        assert_eq!(clock.tick(), 1);
        assert_eq!(clock.tick(), 2);
        clock.set_active(false);
        assert_eq!(clock.tick(), 2);
        clock.reset();
        assert_eq!(clock.tick(), 0);
    }
}
```

- [ ] **Step 2: Run cargo test to verify BreakClock logic**

Run: `cargo test --manifest-path windows/src-tauri/Cargo.toml break_clock`
Expected: PASS

- [ ] **Step 3: Register `break_clock` module in `lib.rs` and add IPC commands**

In `windows/src-tauri/src/lib.rs`:
Add `pub mod break_clock;` and register `get_break_config` / `set_break_config` IPC commands.

- [ ] **Step 4: Commit changes**

```bash
git add windows/src-tauri/src/break_clock.rs windows/src-tauri/src/lib.rs
git commit -m "feat(break): add Rust break clock module and configuration IPC"
```

---

### Task 3: Implement Pet Care / Tamagotchi HUD System (`care.rs` & `care.ts`)

**Files:**
- Create: `windows/src-tauri/src/care.rs`
- Create: `windows/src/care.ts`
- Modify: `windows/src-tauri/src/lib.rs`, `windows/settings.html`

**Interfaces:**
- Consumes: Care user actions (feed, play, rest, clean), session completions (`done`).
- Produces: Care status metrics JSON (health, hunger, happiness, energy, exp, level).

- [ ] **Step 1: Write failing unit test for `CareState` mutations**

In `windows/src-tauri/src/care.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CareState {
    pub health: u8,
    pub hunger: u8,
    pub happiness: u8,
    pub energy: u8,
    pub exp: u32,
    pub level: u32,
}

impl Default for CareState {
    fn default() -> Self {
        Self {
            health: 100,
            hunger: 80,
            happiness: 80,
            energy: 90,
            exp: 0,
            level: 1,
        }
    }
}

impl CareState {
    pub fn feed(&mut self) {
        self.hunger = (self.hunger + 25).min(100);
        self.health = (self.health + 5).min(100);
    }

    pub fn play(&mut self) {
        self.happiness = (self.happiness + 20).min(100);
        if self.energy >= 5 { self.energy -= 5; }
    }

    pub fn rest(&mut self) {
        self.energy = (self.energy + 30).min(100);
    }

    pub fn clean(&mut self) {
        self.health = (self.health + 10).min(100);
    }

    pub fn add_exp(&mut self, amount: u32) {
        self.exp += amount;
        self.level = 1 + (self.exp / 100);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_care_actions() {
        let mut care = CareState::default();
        care.hunger = 50;
        care.feed();
        assert_eq!(care.hunger, 75);
        care.add_exp(250);
        assert_eq!(care.level, 3);
    }
}
```

- [ ] **Step 2: Run cargo test to verify care state behavior**

Run: `cargo test --manifest-path windows/src-tauri/Cargo.toml care`
Expected: PASS

- [ ] **Step 3: Implement `windows/src/care.ts` frontend controller**

In `windows/src/care.ts`:

```typescript
import { invoke } from "@tauri-apps/api/core";

export interface CareState {
  health: number;
  hunger: number;
  happiness: number;
  energy: number;
  exp: number;
  level: number;
}

export async function fetchCareState(): Promise<CareState> {
  return await invoke<CareState>("get_care_state");
}

export async function performCareAction(action: "feed" | "play" | "rest" | "clean"): Promise<CareState> {
  return await invoke<CareState>("perform_care_action", { action });
}

export function renderCareHUD(state: CareState) {
  const setBar = (id: string, val: number) => {
    const el = document.getElementById(id);
    if (el) el.style.width = `${val}%`;
  };
  setBar("care-bar-health", state.health);
  setBar("care-bar-hunger", state.hunger);
  setBar("care-bar-happiness", state.happiness);
  setBar("care-bar-energy", state.energy);

  const lvlEl = document.getElementById("care-level-badge");
  if (lvlEl) lvlEl.textContent = `Lvl ${state.level} (${state.exp} XP)`;
}
```

- [ ] **Step 4: Commit changes**

```bash
git add windows/src-tauri/src/care.rs windows/src/care.ts windows/src-tauri/src/lib.rs
git commit -m "feat(care): implement pet care state machine and frontend HUD bindings"
```

---

### Task 4: Implement Token Usage Analytics & Project Pets

**Files:**
- Create: `windows/src-tauri/src/project_pets.rs`
- Modify: `windows/src-tauri/src/usage.rs`
- Modify: `windows/src/usage.ts`, `windows/src/settings.ts`, `windows/settings.html`

**Interfaces:**
- Consumes: Project paths, transcript files, pet selection input.
- Produces: Daily 90-day usage breakdown table & per-project pet mapping.

- [ ] **Step 1: Write unit tests for Project Pets mapping**

In `windows/src-tauri/src/project_pets.rs`:

```rust
use std::collections::HashMap;
use std::sync::Mutex;

pub struct ProjectPetStore {
    mapping: Mutex<HashMap<String, String>>,
}

impl ProjectPetStore {
    pub fn new() -> Self {
        Self { mapping: Mutex::new(HashMap::new()) }
    }

    pub fn set_pet(&self, project: String, pet_id: String) {
        if let Ok(mut map) = self.mapping.lock() {
            map.insert(project, pet_id);
        }
    }

    pub fn get_pet(&self, project: &str) -> Option<String> {
        self.mapping.lock().ok()?.get(project).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_pet_store() {
        let store = ProjectPetStore::new();
        store.set_pet("E:/workspace/project-a".into(), "cat".into());
        assert_eq!(store.get_pet("E:/workspace/project-a"), Some("cat".into()));
        assert_eq!(store.get_pet("E:/workspace/other"), None);
    }
}
```

- [ ] **Step 2: Run cargo test to verify project pet store**

Run: `cargo test --manifest-path windows/src-tauri/Cargo.toml project_pets`
Expected: PASS

- [ ] **Step 3: Connect Usage & Project Pets UI in `windows/src/settings.ts` and `settings.html`**

Update `settings.html` and `settings.ts` to render project pet selector dropdowns and complete the Care & Usage dashboard tabs.

- [ ] **Step 4: Verify complete frontend compilation**

Run: `npm --prefix windows run build`
Expected: PASS with zero TypeScript/Vite errors.

- [ ] **Step 5: Commit changes**

```bash
git add windows/src-tauri/src/project_pets.rs windows/src/settings.ts windows/settings.html
git commit -m "feat(parity): finalize token usage dashboard and project pet assignment"
```

# Windows Phase 1: OpenCode Token Accounting & Sound Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make OpenCode token usage real via read-only live SQLite deltas with persistent snapshots, surface input/output/cache breakdown in the Usage UI without inventing historical breakdown, and harden custom notification sound playback (CSP data-URL media + `Audio.play()` promise rejection fallback).

**Architecture:** On each OpenCode lifecycle event already handled in `windows/src-tauri/src/server.rs`, after state emission, a fail-open side path reads one row from `opencode.db` using Windows system `winsqlite3` FFI (no new Cargo crates). Pure delta/XP helpers in Rust compute non-negative deltas against a disk-backed per-session snapshot under `%APPDATA%\AgentPet\`. Positive totals emit existing `agent-tokens` (extended optional breakdown fields); Care XP uses input+output only. Frontend keeps `ap_usage` localStorage as primary UI store; optional breakdown fields default to 0 for old rows. Sound: allow `media-src` data URLs in Tauri CSP and fall back to built-in chime when custom `play()` rejects.

**Tech Stack:** Rust (Tauri v2, existing deps only: `serde`, `serde_json`, `dirs`, `tiny_http`, `tauri`), Windows system `winsqlite3.dll` via `extern "C"` (not a Cargo dependency), TypeScript/Vite frontend, localStorage `ap_usage`.

## Global Constraints

- **Scope:** `windows/` only; no Project Pets, Break Reminder, update security, cost, cache-write, reasoning, historical backfill, or cloud payload expansion.
- **Glossary (CONTEXT.md / design §3):** `usage_total = input + output + cache-read`; `cache = tokens_cache_read` only; `pet_xp = input + output` (never cache-read).
- **DB path:** `C:\Users\trmin\.local\share\opencode\opencode.db` resolved via `dirs::home_dir()` + `.local/share/opencode/opencode.db` (do not hardcode a single user profile in code).
- **Live schema fact (verified 2026-07-22):** table `session` PK column is **`id`** (e.g. `ses_…`), not `session_id`. Spec’s “session_id” means this stable key. Columns used: `id`, `directory`, `agent`, `time_updated`, `tokens_input`, `tokens_output`, `tokens_cache_read`. Never read `tokens_cache_write`, `tokens_reasoning`, or `cost` in Phase 1.
- **SQLite access must be dependency-free:** `windows/src-tauri/Cargo.toml` currently has no `rusqlite`/`sqlx`/`sqlite` crates — **do not add any**. Use Windows system `winsqlite3` via `extern "C"` only (`#[cfg(windows)]`). Non-Windows: stub returns `None` (Phase 1 is Windows-scoped).
- **Concurrency / WAL:** live DB uses `journal_mode=wal`. Open **read-only** with URI `file:<path>?mode=ro` (or `SQLITE_OPEN_READONLY`), query one row, close immediately. Never write the OpenCode DB. On `SQLITE_BUSY` / open / prepare / step / column errors: treat as read failure — no tokens, no XP, snapshot unchanged, state pipeline unaffected.
- **Snapshots:** disk-backed JSON map keyed by OpenCode session `id` (after stripping optional `opencode:` prefix from hook session strings). Path: `dirs::config_dir()/AgentPet/opencode_token_snapshots.json`. Survive restarts. Missing snapshot ⇒ prior counts zero (first-seen baseline may emit absolute counts once). **Write-first / commit-memory-after-success:** serialize the full updated map and write JSON to disk first; only on successful write update the in-memory snapshot and return/emit a token delta. Persistence failure leaves **both** disk and in-memory snapshot unchanged; delta is not emitted/acknowledged; later hook retries. Same absolute totals are never double-counted after a successful write.
- **Primary Usage UI store:** keep `windows/src/usage.ts` localStorage key `ap_usage`. Optional `input`/`output`/`cache` on rows; `tokens` remains authoritative total. Cloud `/api/usage/sync` payload stays `{ projectId, projectName, agent, day, tokens, sessions }` only (strip breakdown on push if present on object).
- **Failure policy:** SQLite/snapshot/audio failures never crash the event loop or block `working`/`waiting`/`done`.
- **Verification (machine-checkable):** focused unit tests (delta + XP), `npm run build` in `windows/`, `cargo test` in Tauri crate, `node scripts/check-care-css.mjs` from `windows/`, `git diff --check` from `source/` when Git is available.
- **Dirty worktree / no commits:** this tree is dirty and commit steps are **out of scope**. Do **not** run `git add` / `git commit`. Implement and verify only.

## File Structure

| File | Responsibility |
|------|----------------|
| `windows/src-tauri/src/opencode_db.rs` | **Create.** Pure delta/XP math; snapshot load/save; read-only `winsqlite3` session row fetch; map hook session → DB `id`. |
| `windows/src-tauri/src/server.rs` | **Modify.** After OpenCode state handling, call token refresh side path; emit extended `agent-tokens`. |
| `windows/src-tauri/src/lib.rs` | **Modify.** `mod opencode_db;` only (no new public IPC required for Phase 1). |
| `windows/src/usage.ts` | **Modify.** Optional breakdown fields; `recordTokenBreakdown`; cloud push still total-only. |
| `windows/src/main.ts` | **Modify.** `agent-tokens` listener uses breakdown for usage; Care XP from `tokens` payload only when backend already set `tokens = Δxp` **or** frontend uses explicit `xp`/`input`+`output` — plan chooses backend emits `tokens` = usage total and `xp` = pet XP so Care never eats cache. Sound: promise rejection → built-in chime. |
| `windows/src/demo.ts` | **Modify.** Same custom-sound promise fallback as `main.ts`. |
| `windows/src/settings.ts` | **Modify.** Summary + table show input/output/cache; chart stays total-only. |
| `windows/settings.html` | **Modify.** Summary stats + table headers for breakdown columns. |
| `windows/src/styles.css` | **Modify.** Usage summary grid for 7 stats; table numeric columns alignment. |
| `windows/src/i18n.ts` + `settings.ts` label map | **Modify.** Labels for Input / Output / Cache. |
| `windows/src-tauri/tauri.conf.json` | **Modify.** CSP `media-src` allows `data:`. |
| `windows/src-tauri/Cargo.toml` | **Do not add SQLite crates.** Unchanged for deps. |

---

### Task 1: Pure delta and XP helpers (Rust) with failing tests first

**Files:**
- Create: `windows/src-tauri/src/opencode_db.rs`
- Modify: `windows/src-tauri/src/lib.rs` (add `pub mod opencode_db;`)
- Test: `windows/src-tauri/src/opencode_db.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: none (pure functions).
- Produces:
  - `pub struct TokenTriple { pub input: u64, pub output: u64, pub cache_read: u64 }`
  - `pub struct TokenDelta { pub input: u64, pub output: u64, pub cache: u64, pub total: u64, pub xp: u64 }`
  - `pub fn compute_delta(current: &TokenTriple, snapshot: &TokenTriple) -> TokenDelta`
  - `pub fn pet_xp(input: u64, output: u64) -> u64`  // `input + output`

- [ ] **Step 1: Create module skeleton and failing tests**

Create `windows/src-tauri/src/opencode_db.rs`:

```rust
//! OpenCode live usage: pure math + read-only session row + disk snapshots.
//! Phase 1: no Cargo SQLite crates — Windows winsqlite3 FFI only (Task 2).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenTriple {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenDelta {
    pub input: u64,
    pub output: u64,
    pub cache: u64,
    pub total: u64,
    pub xp: u64,
}

/// usage_total = input + output + cache_read (glossary).
pub fn usage_total(t: &TokenTriple) -> u64 {
    t.input.saturating_add(t.output).saturating_add(t.cache_read)
}

/// pet_xp = input + output only (never cache-read).
pub fn pet_xp(input: u64, output: u64) -> u64 {
    input.saturating_add(output)
}

/// Non-negative per-field deltas; total = sum of three; xp = input+output only.
pub fn compute_delta(current: &TokenTriple, snapshot: &TokenTriple) -> TokenDelta {
    let input = current.input.saturating_sub(snapshot.input);
    let output = current.output.saturating_sub(snapshot.output);
    let cache = current.cache_read.saturating_sub(snapshot.cache_read);
    TokenDelta {
        input,
        output,
        cache,
        total: input.saturating_add(output).saturating_add(cache),
        xp: pet_xp(input, output),
    }
}

// Snapshot store + DB read land in later steps of this module; tests below only need pure math.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_positive_growth() {
        let snap = TokenTriple { input: 10, output: 5, cache_read: 2 };
        let cur = TokenTriple { input: 15, output: 8, cache_read: 4 };
        let d = compute_delta(&cur, &snap);
        assert_eq!(d.input, 5);
        assert_eq!(d.output, 3);
        assert_eq!(d.cache, 2);
        assert_eq!(d.total, 10);
        assert_eq!(d.xp, 8);
    }

    #[test]
    fn delta_clamps_when_current_below_snapshot() {
        let snap = TokenTriple { input: 100, output: 50, cache_read: 20 };
        let cur = TokenTriple { input: 90, output: 40, cache_read: 10 };
        let d = compute_delta(&cur, &snap);
        assert_eq!(d.input, 0);
        assert_eq!(d.output, 0);
        assert_eq!(d.cache, 0);
        assert_eq!(d.total, 0);
        assert_eq!(d.xp, 0);
    }

    #[test]
    fn pet_xp_is_input_plus_output() {
        assert_eq!(pet_xp(3, 4), 7);
        assert_eq!(pet_xp(0, 0), 0);
    }

    #[test]
    fn cache_only_growth_does_not_increase_xp() {
        let snap = TokenTriple { input: 10, output: 5, cache_read: 2 };
        let cur = TokenTriple { input: 10, output: 5, cache_read: 100 };
        let d = compute_delta(&cur, &snap);
        assert_eq!(d.input, 0);
        assert_eq!(d.output, 0);
        assert_eq!(d.cache, 98);
        assert_eq!(d.total, 98);
        assert_eq!(d.xp, 0);
    }

    #[test]
    fn missing_snapshot_treated_as_zero_baseline() {
        let cur = TokenTriple { input: 15, output: 8, cache_read: 4 };
        let d = compute_delta(&cur, &TokenTriple::default());
        assert_eq!(d.input, 15);
        assert_eq!(d.output, 8);
        assert_eq!(d.cache, 4);
        assert_eq!(d.total, 27);
        assert_eq!(d.xp, 23);
    }
}
```

In `windows/src-tauri/src/lib.rs` after existing `pub mod` lines (near `pub mod usage;`):

```rust
pub mod opencode_db;
```

- [ ] **Step 2: Run focused cargo tests (expect PASS once pure helpers compile)**

Run from `E:\agentpet\source`:

```powershell
cargo test --manifest-path windows/src-tauri/Cargo.toml delta_positive_growth delta_clamps_when_current_below_snapshot pet_xp_is_input_plus_output cache_only_growth_does_not_increase_xp missing_snapshot_treated_as_zero_baseline -- --nocapture
```

Expected: all five tests PASS (pure math only; no DB).

- [ ] **Step 3: No commit**

Do not commit. Worktree is dirty; Phase 1 forbids commit steps.

---

### Task 2: Dependency-free read-only SQLite via winsqlite3 + snapshot store

**Files:**
- Modify: `windows/src-tauri/src/opencode_db.rs`
- Test: same file

**Interfaces:**
- Consumes: `TokenTriple`, `compute_delta` from Task 1.
- Produces:
  - `pub fn normalize_session_id(hook_session: &str) -> String` — strips leading `opencode:` if present.
  - `pub fn default_db_path() -> Option<PathBuf>`
  - `pub fn read_session_tokens(db_path: &Path, session_id: &str) -> Option<TokenTriple>`
  - `pub struct SnapshotStore { ... }` with `load`, `get`, `put_and_save`, `compute_and_commit(session_id, current) -> Option<TokenDelta>`
  - Failure paths return `None`; on snapshot write failure leave memory and disk unchanged (no emit).

**Concurrency / read-only WAL behavior (implement exactly):**

1. Build URI: `format!("file:{}?mode=ro", path_with_forward_slashes)` for `sqlite3_open_v2`.
2. Flags: `SQLITE_OPEN_READONLY | SQLITE_OPEN_URI` (`0x00000001 | 0x00000040`).
3. SQL (column names match live DB):

```sql
SELECT tokens_input, tokens_output, tokens_cache_read
FROM session
WHERE id = ?1
LIMIT 1;
```

4. Bind session id as text UTF-8.
5. On `SQLITE_ROW`, read three integers as `u64` via `i64` cast (`max(0, …)` if negative ever appears).
6. Finalize statement, close DB on every path (RAII guard or explicit close).
7. Do not set journal_mode; do not CREATE; do not UPDATE.
8. If file missing, open fails, prepare fails, no row, or step error → `None`.
9. Callers must not hold a connection across await/event-loop turns — open/query/close inside one function.

**winsqlite3 FFI (no Cargo dependency):**

```rust
#[cfg(windows)]
#[link(name = "winsqlite3")]
extern "C" {
    fn sqlite3_open_v2(
        filename: *const i8,
        ppDb: *mut *mut std::ffi::c_void,
        flags: i32,
        zVfs: *const i8,
    ) -> i32;
    fn sqlite3_close(db: *mut std::ffi::c_void) -> i32;
    fn sqlite3_prepare_v2(
        db: *mut std::ffi::c_void,
        zSql: *const i8,
        nByte: i32,
        ppStmt: *mut *mut std::ffi::c_void,
        pzTail: *mut *const i8,
    ) -> i32;
    fn sqlite3_bind_text(
        stmt: *mut std::ffi::c_void,
        idx: i32,
        text: *const i8,
        n: i32,
        destructor: isize,
    ) -> i32;
    fn sqlite3_step(stmt: *mut std::ffi::c_void) -> i32;
    fn sqlite3_column_int64(stmt: *mut std::ffi::c_void, iCol: i32) -> i64;
    fn sqlite3_finalize(stmt: *mut std::ffi::c_void) -> i32;
}

const SQLITE_OPEN_READONLY: i32 = 0x00000001;
const SQLITE_OPEN_URI: i32 = 0x00000040;
const SQLITE_ROW: i32 = 100;
const SQLITE_OK: i32 = 0;
// SQLITE_TRANSIENT = -1 as destructor for bind_text
const SQLITE_TRANSIENT: isize = -1;
```

`#[cfg(not(windows))]` `read_session_tokens` returns `None`.

**Snapshot file format:**

```json
{
  "ses_abc": { "input": 10, "output": 5, "cache_read": 2 },
  "ses_def": { "input": 0, "output": 0, "cache_read": 0 }
}
```

Path helper:

```rust
pub fn snapshot_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("AgentPet").join("opencode_token_snapshots.json"))
}
```

`SnapshotStore`:

```rust
#[derive(Default)]
pub struct SnapshotStore {
    path: Option<PathBuf>,
    map: Mutex<HashMap<String, TokenTriple>>,
}

impl SnapshotStore {
    pub fn load() -> Self {
        let path = snapshot_path();
        let map = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<HashMap<String, TokenTriple>>(&s).ok())
            .unwrap_or_default();
        Self { path, map: Mutex::new(map) }
    }

    /// Write-first / commit-memory-after-success.
    /// 1) Compute delta against current in-memory snapshot (do not mutate yet).
    /// 2) Build next map with `current` absolute counts for this session.
    /// 3) Serialize full map and write JSON to disk; on any create_dir/serialize/write
    ///    failure: leave memory and disk unchanged, return None (no emit; later hook retries).
    /// 4) Only after successful write: commit next map into memory.
    /// 5) Return Some(delta) only when delta.total > 0 OR delta.xp > 0 (caller still checks).
    /// On lock failure: return None; memory/disk unchanged.
    pub fn compute_and_commit(&self, session_id: &str, current: TokenTriple) -> Option<TokenDelta> {
        let mut guard = self.map.lock().ok()?;
        let snap = guard.get(session_id).copied().unwrap_or_default();
        let delta = compute_delta(&current, &snap);
        // Prepare next map without committing memory until disk write succeeds.
        let mut next = guard.clone();
        next.insert(session_id.to_string(), current);
        if let Some(ref p) = self.path {
            if let Some(dir) = p.parent() {
                fs::create_dir_all(dir).ok()?;
            }
            let body = serde_json::to_string_pretty(&next).ok()?;
            fs::write(p, body).ok()?;
        }
        // Disk OK (or no path for pure in-memory test): commit memory.
        *guard = next;
        drop(guard);
        if delta.total == 0 && delta.xp == 0 {
            return None;
        }
        Some(delta)
    }
}
```

Note: on successful write, snapshot advances even when all deltas are 0 after first observation of unchanged counters — that prevents re-baselining noise. First observation with zero snapshot emits full absolute delta once (intentional per design §3.2). Write/IO failure never advances memory or disk and never yields a delta for emit; the next hook retry recomputes against the unchanged snapshot.

- [ ] **Step 1: Write failing tests for normalize + snapshot commit (in-memory path optional)**

Add tests:

```rust
#[test]
fn normalize_strips_opencode_prefix() {
    assert_eq!(normalize_session_id("opencode:ses_abc"), "ses_abc");
    assert_eq!(normalize_session_id("ses_abc"), "ses_abc");
}

#[test]
fn snapshot_commit_avoids_duplicate_on_second_same_current() {
    // Use a temp file under std::env::temp_dir()
    let dir = std::env::temp_dir().join(format!("ap-snap-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("opencode_token_snapshots.json");
    let store = SnapshotStore {
        path: Some(path.clone()),
        map: Mutex::new(HashMap::new()),
    };
    let cur = TokenTriple { input: 15, output: 8, cache_read: 4 };
    let d1 = store.compute_and_commit("ses_x", cur).expect("first");
    assert_eq!(d1.total, 27);
    assert_eq!(d1.xp, 23);
    let d2 = store.compute_and_commit("ses_x", cur);
    assert!(d2.is_none(), "same absolute counters must not re-emit");
    let d3 = store.compute_and_commit(
        "ses_x",
        TokenTriple { input: 20, output: 8, cache_read: 4 },
    )
    .expect("growth");
    assert_eq!(d3.input, 5);
    assert_eq!(d3.xp, 5);
    assert_eq!(d3.total, 5);
    let _ = fs::remove_dir_all(dir);
}
```

For DB read without live OpenCode DB, add injectable test via pure path: unit-test that `read_session_tokens` on a missing path returns `None`:

```rust
#[test]
fn missing_db_returns_none() {
    let p = PathBuf::from("C:\\this\\path\\does\\not\\exist\\opencode.db");
    assert!(read_session_tokens(&p, "ses_x").is_none());
}
```

Optional integration (skip if file absent): if `default_db_path()` exists, `read_session_tokens` for a known id may return `Some` — do not require specific token numbers in CI.

- [ ] **Step 2: Implement `normalize_session_id`, `default_db_path`, FFI `read_session_tokens`, `SnapshotStore`**

```rust
pub fn normalize_session_id(hook_session: &str) -> String {
    hook_session
        .strip_prefix("opencode:")
        .unwrap_or(hook_session)
        .to_string()
}

pub fn default_db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".local").join("share").join("opencode").join("opencode.db"))
}
```

Implement `read_session_tokens` with the FFI block above; convert path to UTF-8 CString; on any non-OK open/prepare/bind, close and return `None`.

- [ ] **Step 3: Run tests**

```powershell
cargo test --manifest-path windows/src-tauri/Cargo.toml opencode_db -- --nocapture
```

Expected: PASS for pure math, normalize, snapshot dedupe, missing DB. Link errors about `winsqlite3` must be fixed by ensuring `#[cfg(windows)]` link is present (Windows SDK provides the import lib).

- [ ] **Step 4: No commit**

---

### Task 3: Wire OpenCode lifecycle side path in `server.rs`

**Files:**
- Modify: `windows/src-tauri/src/server.rs`
- Modify: `windows/src-tauri/src/lib.rs` (manage `SnapshotStore` if shared; prefer `lazy_static`-free `OnceLock<SnapshotStore>` inside `opencode_db` to avoid extra crates)

**Interfaces:**
- Consumes: `opencode_db::{normalize_session_id, default_db_path, read_session_tokens, SnapshotStore, compute path}`
- Produces: Tauri event `agent-tokens` payload:

```json
{
  "agent": "opencode",
  "session": "<hook session string as received>",
  "project": "<directory or hook project>",
  "tokens": <Δtotal usage_total>,
  "xp": <Δxp input+output only>,
  "input": <Δinput>,
  "output": <Δoutput>,
  "cache": <Δcache>
}
```

Legacy frontend that only reads `tokens` will record usage total correctly. Care must use `xp` (Task 4).

**Where to hook (existing flow):**

In `handle_event` (`server.rs`), after Claude/Codex token threads and **without replacing** `emit_payload`:

1. If `agent_kind != "opencode"`, skip token side path.
2. After computing `state` (or even when state maps), spawn a short-lived thread that:
   - normalizes session id from `tok_session`
   - if empty after normalize, return
   - `read_session_tokens` from `default_db_path()`
   - on `None`, return (no emit)
   - `SNAPSHOT.compute_and_commit(&id, triple)`
   - on `Some(delta)` with `delta.total > 0 || delta.xp > 0`, emit `agent-tokens` with fields above
3. Prefer project path: if hook `project` non-empty use it; else use `directory` from DB only if you extend `read_session_tokens` to return `(TokenTriple, directory, agent)` — **minimal path:** keep hook `project` as today; tokens still apply if project empty only when frontend allows (frontend currently requires `p.project` for usage). Extend reader to also `SELECT directory, agent` so Usage aggregation works when plugin passes directory (plugin already passes `--project directory`).

Recommended reader return type for Task 3:

```rust
#[derive(Debug, Clone)]
pub struct SessionTokens {
    pub tokens: TokenTriple,
    pub directory: String,
    pub agent: String,
}

pub fn read_session_row(db_path: &Path, session_id: &str) -> Option<SessionTokens>;
```

SQL:

```sql
SELECT tokens_input, tokens_output, tokens_cache_read, directory, agent
FROM session WHERE id = ?1 LIMIT 1;
```

**Global snapshot:**

```rust
// opencode_db.rs
use std::sync::OnceLock;
fn snapshots() -> &'static SnapshotStore {
    static S: OnceLock<SnapshotStore> = OnceLock::new();
    S.get_or_init(SnapshotStore::load)
}
```

**Integration snippet for `server.rs` (after codex block / before or after `emit_payload` — state must still emit even if this fails):**

```rust
if agent_kind == "opencode" {
    let app2 = app.clone();
    let sess = tok_session.clone();
    let proj = tok_project.clone();
    std::thread::spawn(move || {
        let Some(path) = crate::opencode_db::default_db_path() else { return };
        if !path.is_file() { return; }
        let id = crate::opencode_db::normalize_session_id(&sess);
        if id.is_empty() { return; }
        let Some(row) = crate::opencode_db::read_session_row(&path, &id) else { return };
        let project = if !proj.is_empty() {
            proj
        } else {
            row.directory.clone()
        };
        let Some(delta) = crate::opencode_db::snapshots().compute_and_commit(&id, row.tokens) else {
            return;
        };
        if delta.total == 0 && delta.xp == 0 {
            return;
        }
        let _ = app2.emit(
            "agent-tokens",
            serde_json::json!({
                "agent": "opencode",
                "session": sess,
                "project": project,
                "tokens": delta.total,
                "xp": delta.xp,
                "input": delta.input,
                "output": delta.output,
                "cache": delta.cache,
            }),
        );
    });
}
```

**Failure modes (design §5.2):** missing DB / missing row / SQLite error → silent return; `emit_payload` for state already ran or still runs independently. Token path must never `return` early from `handle_event` before state emit for non-token reasons.

**When to read:** every OpenCode event that already produces a state (`working`/`waiting`/`done` via plugin flags). Session-end-only is insufficient for cumulative counters mid-session — plugin fires `tool.call` as `working`, so mid-session deltas are captured. Do **not** block on DB inside the HTTP accept thread for long; always `thread::spawn` like Claude/Codex.

- [ ] **Step 1: Add unit test that pure failure contract is documented**

```rust
#[test]
fn read_failure_contract_is_none() {
    // Injectable: missing path → None (no panic)
    assert!(read_session_row(std::path::Path::new("Z:\\nope\\opencode.db"), "x").is_none());
}
```

- [ ] **Step 2: Implement `read_session_row` + `snapshots()` + server.rs wire**

- [ ] **Step 3: Run cargo tests**

```powershell
cargo test --manifest-path windows/src-tauri/Cargo.toml -- --nocapture
```

Expected: PASS (including existing `usage`, `hooks`, new `opencode_db` tests).

- [ ] **Step 4: No commit**

---

### Task 4: Frontend `agent-tokens` + Usage localStorage breakdown + Care XP split

**Files:**
- Modify: `windows/src/usage.ts`
- Modify: `windows/src/main.ts` (listener ~lines 261–272)

**Interfaces:**
- Consumes: event payload `{ agent, session, project, tokens, xp?, input?, output?, cache? }`
- Produces: localStorage rows with optional breakdown; Care fed with XP only.

- [ ] **Step 1: Extend `UsageRow` and recording API**

In `windows/src/usage.ts`, replace interface and `record` path:

```typescript
export interface UsageRow {
  projectId: string;
  projectName: string;
  agent: string;
  day: string;
  tokens: number;
  sessions: number;
  /** Optional; absent on historical rows → treat as 0 in UI. Never infer from tokens. */
  input?: number;
  output?: number;
  cache?: number;
}

function record(
  project: string,
  agent: string,
  tokens: number,
  sessions: number,
  breakdown?: { input: number; output: number; cache: number },
) {
  if (!project || !agent || (tokens <= 0 && sessions <= 0 && !(breakdown && (breakdown.input > 0 || breakdown.output > 0 || breakdown.cache > 0)))) {
    return;
  }
  const { id, name } = projectIdentity(project);
  const day = today();
  const key = id + "|" + agent + "|" + day;
  const store = load();
  const r = store[key] || {
    projectId: id, projectName: name, agent, day,
    tokens: 0, sessions: 0, input: 0, output: 0, cache: 0,
  };
  r.tokens += tokens;
  r.sessions += sessions;
  if (breakdown) {
    r.input = (r.input || 0) + breakdown.input;
    r.output = (r.output || 0) + breakdown.output;
    r.cache = (r.cache || 0) + breakdown.cache;
  }
  r.projectName = name;
  store[key] = r;
  save(store);
  const dirty = loadDirty();
  dirty.add(key);
  saveDirty(dirty);
  schedulePush();
}

export function recordTokens(project: string, agent: string, tokens: number) {
  record(project, agent, tokens, 0);
}

/** Phase 1 OpenCode path: total + breakdown; tokens must equal input+output+cache for new writes. */
export function recordTokenBreakdown(
  project: string,
  agent: string,
  input: number,
  output: number,
  cache: number,
) {
  const tokens = input + output + cache;
  record(project, agent, tokens, 0, { input, output, cache });
}

export function recordSession(project: string, agent: string) {
  record(project, agent, 0, 1);
}
```

Cloud `push()` body: map snapshot rows to total-only shape (do not send input/output/cache):

```typescript
body: JSON.stringify({
  rows: snapshot.map((r) => ({
    projectId: r.projectId,
    projectName: r.projectName,
    agent: r.agent,
    day: r.day,
    tokens: r.tokens,
    sessions: r.sessions,
  })),
}),
```

`list()` remains `Object.values(load())…` — historical rows without input/output/cache fields stay as stored; UI treats missing as 0.

- [ ] **Step 2: Update `main.ts` agent-tokens listener**

Replace the listener at `windows/src/main.ts:261-272` with:

```typescript
listen<{
  agent: string;
  session: string;
  project: string;
  tokens: number;
  xp?: number;
  input?: number;
  output?: number;
  cache?: number;
}>("agent-tokens", (e) => {
  const p = e.payload;
  if (!p) return;
  const input = p.input ?? 0;
  const output = p.output ?? 0;
  const cache = p.cache ?? 0;
  const hasBreakdown = (p.input != null) || (p.output != null) || (p.cache != null);
  const total = hasBreakdown ? (input + output + cache) : (p.tokens || 0);
  // Care XP: prefer explicit xp; else legacy total-only agents (Claude/Codex) feed full tokens.
  const xp = p.xp != null ? p.xp : total;
  if (total <= 0 && xp <= 0) return;
  if (p.project) {
    if (hasBreakdown) {
      usage.recordTokenBreakdown(p.project, p.agent, input, output, cache);
    } else if (total > 0) {
      usage.recordTokens(p.project, p.agent, total);
    }
    emit("usage-updated");
  }
  if (xp > 0) {
    const slug = savedSlug();
    if (slug) {
      care.mutate(slug, (s) => care.feedTokens(s, xp));
      emit("care-updated");
      sync.schedulePush();
    }
  }
});
```

This preserves Claude/Codex behavior (`tokens` only, no breakdown → full amount to Care) and makes OpenCode cache-safe (`xp` excludes cache).

- [ ] **Step 3: Manual typecheck path**

```powershell
cd E:\agentpet\source\windows; npm run build
```

Expected: `tsc --noEmit && vite build` succeeds (or fails only on pre-existing unrelated errors — fix only Phase 1 type errors you introduced).

- [ ] **Step 4: No commit**

---

### Task 5: Usage UI — summary breakdown + table columns; chart stays total

**Files:**
- Modify: `windows/settings.html` (~222–247)
- Modify: `windows/src/settings.ts` (`renderUsage` ~133–191, i18n labels ~1211–1232)
- Modify: `windows/src/styles.css` (`.usage-summary` grid)
- Modify: `windows/src/i18n.ts` (vi/zh keys for new English strings)

**Interfaces:**
- Consumes: `usage.list()` rows with optional `input`/`output`/`cache`
- Produces: DOM updates only

- [ ] **Step 1: HTML structure**

In `windows/settings.html`, expand summary card:

```html
<div class="gcard usage-summary">
  <div class="usage-stat"><div class="cs-label" id="t-usage-tokens">Tokens</div><div class="cs-val" id="usage-total-tokens">0</div></div>
  <div class="usage-stat"><div class="cs-label" id="t-usage-input">Input</div><div class="cs-val" id="usage-total-input">0</div></div>
  <div class="usage-stat"><div class="cs-label" id="t-usage-output">Output</div><div class="cs-val" id="usage-total-output">0</div></div>
  <div class="usage-stat"><div class="cs-label" id="t-usage-cache">Cache</div><div class="cs-val" id="usage-total-cache">0</div></div>
  <div class="usage-stat"><div class="cs-label" id="t-usage-sessions">Sessions</div><div class="cs-val" id="usage-total-sessions">0</div></div>
  <div class="usage-stat"><div class="cs-label" id="t-usage-projects">Projects</div><div class="cs-val" id="usage-total-projects">0</div></div>
  <div class="usage-stat"><div class="cs-label" id="t-usage-agents">Agents</div><div class="cs-val" id="usage-total-agents">0</div></div>
</div>
```

Table header row:

```html
<thead><tr>
  <th id="t-usage-th-project">Project</th>
  <th id="t-usage-th-agent">Agent</th>
  <th id="t-usage-th-tokens">Tokens</th>
  <th id="t-usage-th-input">Input</th>
  <th id="t-usage-th-output">Output</th>
  <th id="t-usage-th-cache">Cache</th>
  <th id="t-usage-th-sessions">Sessions</th>
</tr></thead>
```

- [ ] **Step 2: `renderUsage` aggregation**

In `settings.ts` `renderUsage`, after `filtered`:

```typescript
const totalTokens = filtered.reduce((sum, r) => sum + r.tokens, 0);
const totalInput = filtered.reduce((sum, r) => sum + (r.input || 0), 0);
const totalOutput = filtered.reduce((sum, r) => sum + (r.output || 0), 0);
const totalCache = filtered.reduce((sum, r) => sum + (r.cache || 0), 0);
const totalSessions = filtered.reduce((sum, r) => sum + r.sessions, 0);
setTxt("usage-total-tokens", fmtNum(totalTokens));
setTxt("usage-total-input", fmtNum(totalInput));
setTxt("usage-total-output", fmtNum(totalOutput));
setTxt("usage-total-cache", fmtNum(totalCache));
setTxt("usage-total-sessions", fmtNum(totalSessions));
// projects/agents unchanged
```

Chart buckets: **still only `bucket.tokens`** (total). Do not add series for input/output/cache.

Table grouping:

```typescript
const grouped = new Map<string, {
  project: string; agent: string;
  tokens: number; input: number; output: number; cache: number; sessions: number;
}>();
for (const row of filtered) {
  const key = `${row.projectName}|${row.agent}`;
  const item = grouped.get(key) || {
    project: row.projectName, agent: row.agent,
    tokens: 0, input: 0, output: 0, cache: 0, sessions: 0,
  };
  item.tokens += row.tokens;
  item.input += row.input || 0;
  item.output += row.output || 0;
  item.cache += row.cache || 0;
  item.sessions += row.sessions;
  grouped.set(key, item);
}
// ...
if (body) body.innerHTML = tableRows.map((row) =>
  `<tr><td>${esc(row.project)}</td><td>${esc(row.agent)}</td>` +
  `<td>${fmtNum(row.tokens)}</td><td>${fmtNum(row.input)}</td>` +
  `<td>${fmtNum(row.output)}</td><td>${fmtNum(row.cache)}</td>` +
  `<td>${fmtNum(row.sessions)}</td></tr>`
).join("");
```

Historical rows without breakdown: input/output/cache display **0**; total keeps stored `tokens`. **Never** split total into fake breakdown.

- [ ] **Step 3: CSS**

```css
.usage-summary { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; padding: 10px; }
@media (max-width: 620px) { .usage-summary { grid-template-columns: repeat(2, 1fr); } }
.usage-table td:nth-child(n+3), .usage-table th:nth-child(n+3) { text-align: right; }
```

(7 cells wrap naturally on 4-column grid.)

- [ ] **Step 4: i18n labels**

In `settings.ts` label section add:

```typescript
set("t-usage-input", "Input");
set("t-usage-output", "Output");
set("t-usage-cache", "Cache");
set("t-usage-th-input", "Input");
set("t-usage-th-output", "Output");
set("t-usage-th-cache", "Cache");
```

In `i18n.ts` for `vi` and `zh` maps, add the same English keys with translations:

```typescript
// vi
"Input": "Input",
"Output": "Output",
"Cache": "Cache",
// zh
"Input": "输入",
"Output": "输出",
"Cache": "缓存",
```

(If `t()` is only used for some strings and `set()` uses English literals, matching existing Usage pattern is enough.)

- [ ] **Step 5: Build**

```powershell
cd E:\agentpet\source\windows; npm run build
```

Expected: success.

- [ ] **Step 6: No commit**

---

### Task 6: Sound CSP + `Audio.play()` promise rejection fallback

**Files:**
- Modify: `windows/src-tauri/tauri.conf.json` line 34 CSP
- Modify: `windows/src/main.ts` `chime` (~74–96)
- Modify: `windows/src/demo.ts` `playSound` (~84–94)

**Interfaces:**
- Consumes: `localStorage` keys `ap_sound_{done|waiting}_data` (data URLs)
- Produces: custom play or built-in oscillator chime; never throw into UI loop

- [ ] **Step 1: CSP**

Replace CSP string so media data URLs are allowed. Exact target string:

```json
"csp": "default-src 'self'; img-src 'self' https://pets.thenightwatcher.online data: ; media-src 'self' data: ; connect-src 'self' https://pets.thenightwatcher.online https://agentpet.thenightwatcher.online ipc: http://ipc.localhost ; style-src 'self' 'unsafe-inline'; script-src 'self'"
```

Rationale: `img-src data:` does **not** authorize `HTMLAudioElement` with data URLs; need `media-src … data:`.

- [ ] **Step 2: `chime` in main.ts**

```typescript
function playBuiltinChime(event: "done" | "waiting") {
  try {
    audioCtx = audioCtx || new AudioContext();
    const o = audioCtx.createOscillator();
    const g = audioCtx.createGain();
    o.type = "sine";
    o.frequency.value = event === "done" ? 880 : 560;
    g.gain.value = 0.05;
    o.connect(g);
    g.connect(audioCtx.destination);
    o.start();
    o.stop(audioCtx.currentTime + 0.13);
  } catch {}
}

function chime(event: "done" | "waiting") {
  const key = event === "done" ? "ap_sound_done" : "ap_sound_waiting";
  const legacy = localStorage.getItem("ap_sound");
  const enabled = localStorage.getItem(key) ?? (legacy === "0" ? "0" : "1");
  if (enabled === "0") return;
  const data = localStorage.getItem(`ap_sound_${event}_data`);
  if (data) {
    try {
      const a = new Audio(data);
      const p = a.play();
      if (p !== undefined && typeof (p as Promise<void>).then === "function") {
        (p as Promise<void>).catch(() => playBuiltinChime(event));
        return;
      }
      return;
    } catch {
      // fall through to builtin
    }
  }
  playBuiltinChime(event);
}
```

- [ ] **Step 3: Same pattern in `demo.ts` `playSound`**

```typescript
function playBuiltin(ev: "done" | "waiting") {
  try {
    const ctx = new AudioContext();
    const o = ctx.createOscillator(); const g = ctx.createGain();
    o.type = "sine"; o.frequency.value = ev === "done" ? 880 : 560;
    g.gain.value = 0.05; o.connect(g); g.connect(ctx.destination);
    o.start(); o.stop(ctx.currentTime + 0.13);
  } catch {}
}
function playSound(ev: "done" | "waiting") {
  const data = localStorage.getItem(`ap_sound_${ev}_data`);
  if (data) {
    try {
      const p = new Audio(data).play();
      if (p !== undefined && typeof (p as Promise<void>).then === "function") {
        (p as Promise<void>).catch(() => playBuiltin(ev));
        return;
      }
      return;
    } catch {}
  }
  playBuiltin(ev);
}
```

Default chime behavior when no custom sound is configured remains unchanged (oscillator path).

- [ ] **Step 4: Build**

```powershell
cd E:\agentpet\source\windows; npm run build
```

Expected: success.

- [ ] **Step 5: No commit**

---

### Task 7: Full machine-checkable verification gate

**Files:** none new (run commands only)

- [ ] **Step 1: Focused Rust tests (design §12.1–12.2)**

```powershell
cargo test --manifest-path E:\agentpet\source\windows\src-tauri\Cargo.toml opencode_db -- --nocapture
```

Expected: PASS including:

- snapshot `(10,5,2)` → current `(15,8,4)` ⇒ Δ 5/3/2 total 10 xp 8  
- current below snapshot ⇒ all 0  
- cache-only growth ⇒ xp 0  
- missing DB ⇒ no panic / None  

- [ ] **Step 2: Full cargo test**

```powershell
cargo test --manifest-path E:\agentpet\source\windows\src-tauri\Cargo.toml
```

Expected: all tests PASS.

- [ ] **Step 3: Frontend build**

```powershell
Set-Location E:\agentpet\source\windows; npm run build
```

Expected: exit code 0.

- [ ] **Step 4: Care CSS check**

```powershell
Set-Location E:\agentpet\source\windows; node scripts/check-care-css.mjs
```

Expected: exit code 0 / success message from script.

- [ ] **Step 5: Whitespace check when Git available**

```powershell
Set-Location E:\agentpet\source; git diff --check
```

Expected: no conflict-marker / whitespace errors. If Git is unavailable, note skip; do not invent a repo.

- [ ] **Step 6: No commit**

Explicitly **do not**:

```powershell
git add ...
git commit ...
```

Dirty worktree is expected; Phase 1 acceptance does not include committing.

---

## Self-Review (writing-plans skill)

### 1. Spec coverage

| Spec section | Task |
|--------------|------|
| §2 DB path/table/columns; read-only | Task 2 (`read_session_row`, WAL `mode=ro`) |
| §2 live PK is `id` (verified) | Task 2 SQL `WHERE id = ?1`; normalize strips `opencode:` |
| §3 formulas + delta + snapshot | Task 1 + Task 2 `compute_and_commit` |
| §4 historical rows zero breakdown, no inference | Task 4 load defaults; Task 5 UI `r.input \|\| 0` |
| §5 lifecycle side path; failures don’t block state | Task 3 thread + early returns |
| §6 data flow diagram | Tasks 3–5 |
| §7 Usage UI summary/table/chart total-only | Task 5 |
| §8 CSP + play rejection + default chime | Task 6 |
| §9 localStorage + snapshot path + cloud unchanged | Task 2 path; Task 4 push strip |
| §10 error handling | Tasks 2–3, 6 |
| §11 in/out of scope | Global Constraints; no commit Task 7 |
| §12 acceptance commands | Task 7 |
| Dependency-free SQLite | Task 2 winsqlite3; Cargo.toml unchanged |

### 2. Placeholder scan

No TBD/TODO/“implement later”/“similar to Task N” left. All code snippets are concrete. Commit steps intentionally omitted and stated.

### 3. Type consistency

- `TokenTriple.{input,output,cache_read}` ↔ snapshot JSON ↔ delta fields `input/output/cache` ↔ event `input/output/cache` ↔ `UsageRow.input/output/cache`.
- Event `tokens` = Δusage total; event `xp` = Δpet XP; Care uses `xp`.
- Hook session `opencode:ses_…` ↔ DB `id` `ses_…` via `normalize_session_id`.

### Open implementation choices locked by this plan

1. Snapshot file: `%APPDATA%\AgentPet\opencode_token_snapshots.json` (`dirs::config_dir()`).
2. Frontend listens to extended `agent-tokens` event (not polling `get_usage_summary`).
3. Pure math + DB in Rust; UI aggregation in TypeScript; tests primarily in Rust §12.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-22-windows-phase-1-token-sound.md`. Two execution options:

1. **Subagent-Driven (recommended)** — fresh subagent per task, review between tasks  
2. **Inline Execution** — execute tasks in this session with executing-plans checkpoints  

Which approach?

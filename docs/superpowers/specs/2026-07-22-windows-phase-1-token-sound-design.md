# Windows Phase 1: OpenCode Token Accounting & Sound Design

**Status:** Approved design specification (not implementation)  
**Date:** 2026-07-22  
**Scope:** Windows AgentPet (`windows/`) only

---

## 1. Overview

Phase 1 makes OpenCode token usage real and trustworthy, and hardens custom notification sound playback. It does not add Project Pets, Break Reminder, update security work, cost calculation, or historical backfill.

Canonical glossary (also in `CONTEXT.md`):

| Term | Definition |
|------|------------|
| **Usage total** | `input + output + cache-read` |
| **Cache** | OpenCode `tokens_cache_read` only |
| **Pet XP** | `input + output` only; **excludes** cache-read |

`tokens_cache_write` and `tokens_reasoning` are out of Phase 1 entirely (no read, no store, no UI, no XP).

---

## 2. OpenCode real usage source

### 2.1 Database

| Item | Value |
|------|--------|
| Path | `C:\Users\trmin\.local\share\opencode\opencode.db` |
| Table | `session` |
| Access | **Read-only** SQLite open |

### 2.2 Columns used in Phase 1

| Column | Role |
|--------|------|
| `session_id` | Stable session key for snapshots and deltas |
| `directory` | Project path for Usage aggregation |
| `agent` | Agent kind label for Usage aggregation |
| `time_updated` | Ordering / staleness aid when reading rows |
| `tokens_input` | Input tokens |
| `tokens_output` | Output tokens |
| `tokens_cache_read` | Cache-read tokens (Usage total only; not Care XP) |

### 2.3 Columns explicitly not used

- `tokens_cache_write` — excluded from Phase 1
- `tokens_reasoning` — excluded from Phase 1
- Any cost / dollar columns — excluded from Phase 1

---

## 3. Canonical calculation semantics

### 3.1 Formulas

```
usage_total = tokens_input + tokens_output + tokens_cache_read
cache       = tokens_cache_read
pet_xp      = tokens_input + tokens_output
```

Rules:

1. **Usage total** always includes cache-read.
2. **Cache** display and breakdown fields mean cache-read only.
3. **Pet / Care XP** uses input + output only. Cache-read must never feed Care XP.
4. Cache-write and reasoning never enter any Phase 1 formula.

### 3.2 Delta accounting

On each OpenCode lifecycle event that warrants a usage refresh, the backend:

1. Opens `opencode.db` read-only.
2. Reads the current session row for the event’s `session_id` (or resolves via directory + known session mapping if the event carries directory only — prefer `session_id` when present).
3. Loads the **persistent per-session snapshot** for that `session_id` (last observed input, output, cache-read).
4. Computes non-negative deltas:

```
Δinput  = max(0, current.tokens_input      - snapshot.tokens_input)
Δoutput = max(0, current.tokens_output     - snapshot.tokens_output)
Δcache  = max(0, current.tokens_cache_read - snapshot.tokens_cache_read)
Δtotal  = Δinput + Δoutput + Δcache
Δxp     = Δinput + Δoutput
```

5. If any delta is positive, emits a usage update (total + breakdown) and, separately, Care XP for `Δxp` only when `Δxp > 0`.
6. Writes the new snapshot so the same absolute counts are never double-counted.

Snapshots must survive process restarts (disk-backed map keyed by `session_id`). Missing snapshot for a session means treat prior counts as zero for that first observation only (first-seen baseline may still emit a delta equal to current absolute values — that is intentional for live sessions observed after AgentPet start).

---

## 4. Historical Usage rows (compatibility)

Existing local Usage rows store only:

- `tokens` (aggregate total)
- `sessions`

They do **not** store input / output / cache breakdown.

Rules:

1. **Retain** the stored `tokens` total as-is for summary and charts.
2. Detailed **input / output / cache** values for those rows display as **zero**.
3. Breakdown fields must **never be inferred** by splitting the stored total (no heuristics, no ratios, no reverse-engineering).
4. New writes after Phase 1 may store breakdown fields when available; old rows remain total-only forever unless the user clears data.

---

## 5. Lifecycle integration & failure behavior

### 5.1 When to read

On OpenCode lifecycle events already handled by the Windows hook path (e.g. session progress / idle / done — exact hook set is existing infrastructure), the backend attempts a read-only SQLite refresh for the relevant session and emits per-session deltas as in §3.2.

Token accounting is additive side work on the same event path. It must not replace or block state transitions (`working` / `waiting` / `done`).

### 5.2 Failure modes

| Condition | Token result | State events |
|-----------|--------------|--------------|
| DB file missing | No tokens | Unchanged; state pipeline continues |
| Session row missing | No tokens | Unchanged |
| SQLite open/read error | No tokens | Unchanged |
| Corrupt / unexpected schema | No tokens (treat as read failure) | Unchanged |

In all failure cases: emit **no** token delta, **no** XP delta, leave snapshots unchanged for that attempt, and **do not** disrupt pet state events.

---

## 6. Data flow

```
OpenCode lifecycle event
        │
        ▼
 Windows hook / backend event handler
        │
        ├─► existing state machine (working / waiting / done)  ──► UI / pet
        │
        └─► read-only SQLite (opencode.db / session)
                │
                ├─ success + deltas ──► usage store (total + input/output/cache + sessions)
                │                   ──► Care XP (input+output only)
                │                   ──► update per-session snapshot
                │
                └─ failure / missing ──► no tokens, no XP, snapshot unchanged
```

Frontend Usage UI reads the Usage store (local) and renders summary + breakdown. Optional cloud sync continues to push **existing** row shape unless and until server fields are confirmed accepted (see §9).

---

## 7. Usage UI

| Surface | Behavior |
|---------|----------|
| Summary total | Keep existing total display; value = sum of stored totals (historical) plus new rows’ `usage_total` |
| Input / Output / Cache | Add breakdown next to or under the summary; zeros for historical rows without breakdown |
| Chart | Continues to plot **total** only (not separate series for input/output/cache in Phase 1) |
| Breakdown table | Columns: **total**, **input**, **output**, **cache**, **sessions** (plus existing project/agent/day identity columns as today) |

No cost column. No cache-write or reasoning columns.

---

## 8. Sound scope

1. **CSP:** Content Security Policy must allow custom media via **data URL** sources so user-selected notification sounds can play.
2. **Playback:** Prefer `Audio.play()` for custom sound. If the returned promise **rejects**, fall back to the built-in synthesized chime.
3. **Default chime:** Built-in default chime behavior and sound remain **unchanged** when no custom sound is configured or when custom play fails.

Out of scope for Phase 1: new sound asset packs, per-event multi-sound libraries beyond the existing done/waiting pattern, and OS-level toast audio policy beyond current desktop behavior.

---

## 9. Storage, migration, cloud

### 9.1 Local Usage store

**Decision:** Keep the proposed existing **localStorage** Usage store (`windows/src/usage.ts`, key `ap_usage`) as the Phase 1 persistence path for UI-facing aggregates.

- Extend row shape **optionally** with `input`, `output`, `cache` (or equivalent names) defaulting to `0` when absent.
- Always keep `tokens` as the authoritative **total** for that row (`input + output + cache` for new writes; historical rows keep whatever total was stored).
- Rust `UsageStore` (`usage_history.json`) may remain for backend summary IPC if already wired; Phase 1 does **not** require migrating primary UI storage to Rust. Switch to Rust-only storage only if implementation proves localStorage insufficient (quota, multi-window races, or testability). Document that proof in the implementation PR if it happens.

### 9.2 Per-session snapshots

Disk-backed (backend), keyed by OpenCode `session_id`, storing last seen `tokens_input`, `tokens_output`, `tokens_cache_read`. Not user-facing.

### 9.3 Cloud payload

Do **not** change the `/api/usage/sync` payload contract in Phase 1 unless the server is confirmed to accept additional fields.

- Default: continue pushing `{ projectId, projectName, agent, day, tokens, sessions }` (or current equivalent).
- If breakdown fields are later confirmed accepted, add them in a follow-up; until then, cloud receives total only.

### 9.4 Migration

- No destructive migration.
- Old rows without breakdown fields load with input/output/cache = 0 and retain stored total.
- Snapshots start empty on first Phase 1 run.

---

## 10. Error handling (summary)

| Layer | Behavior |
|-------|----------|
| SQLite | Read-only; any error → zero token impact; log at debug/warn only |
| Snapshot I/O | Fail-open for tokens (skip delta that cycle); never crash event loop |
| Usage localStorage | Existing try/catch load pattern; corrupt store → empty map |
| Care XP | Only apply when `Δxp > 0` from a successful read; never from cache-read alone |
| Custom audio | `play()` rejection → built-in chime; never throw into UI loop |

---

## 11. Implementation boundaries

**In scope (Phase 1):**

- Read-only OpenCode SQLite session token columns listed in §2.2
- Delta + snapshot accounting
- Usage total / input / output / cache semantics
- Care XP from input + output only
- Usage UI summary + breakdown + table columns; chart stays on total
- CSP data-URL media + Audio.play fallback to built-in chime
- Focused tests and verification commands in §12
- Optional localStorage field extension for breakdown without cloud contract change

**Out of scope (explicit):**

- Project Pets
- Break Reminder
- Update security work
- Cache-write UI or accounting
- Reasoning token UI or accounting
- Historical backfill / inferring breakdown from old totals
- Cost / dollar calculation
- Cloud payload expansion without confirmed server acceptance
- macOS changes (unless a shared pure formula is extracted for tests only)
- Commit of this work tree as a Git operation when the working tree is not a Git repo (see §12)

---

## 12. Acceptance criteria (machine-checkable)

All of the following must pass for Phase 1 to be considered complete:

1. **Focused unit tests — delta calculation**  
   - Given snapshot `(in=10, out=5, cache=2)` and current `(in=15, out=8, cache=4)` → `Δinput=5`, `Δoutput=3`, `Δcache=2`, `Δtotal=10`, `Δxp=8`.  
   - Current below snapshot → deltas clamp to `0` (no negative accounting).  
   - Missing DB / missing session / read error path → no delta applied (test via injectable failure or pure function contract).

2. **Focused unit tests — XP calculation**  
   - `pet_xp = input + output` for any non-negative inputs.  
   - Cache-read alone never increases XP (`Δinput=0`, `Δoutput=0`, `Δcache>0` → `Δxp=0`).

3. **`npm run build`** succeeds in `windows/`.

4. **`cargo test`** succeeds in the Windows Tauri crate path.

5. **Care CSS script** (existing project Care CSS check/build script) succeeds.

6. **`git diff --check`** succeeds when a Git working tree is available. **No commit** — design and implementation of this phase do not commit; `source` is not assumed to be a Git repo for commit purposes.

---

## 13. Self-review checklist

| Check | Result |
|-------|--------|
| No TBD / TODO / placeholder text | Clean |
| No contradiction with `CONTEXT.md` glossary | Aligned: total = in+out+cache-read; XP = in+out; cache = cache-read |
| Historical rows never invent breakdown | Stated in §4 |
| Failures do not break state events | Stated in §5.2 |
| Cloud contract unchanged by default | Stated in §9.3 |
| Out-of-scope list explicit | §11 |
| Sound: CSP data URL + play rejection fallback + default chime unchanged | §8 |
| Acceptance criteria machine-checkable | §12 |

---

## 14. Open implementation choices (not blockers)

These are allowed flexibility during implementation; they do not reopen product scope:

1. Snapshot file path under `%APPDATA%\AgentPet\` (exact filename).
2. Whether frontend listens to a new Tauri event vs polling an extended `get_usage_summary` for breakdown fields.
3. Whether pure delta/XP helpers live in Rust, TypeScript, or both with mirrored tests — as long as §12 tests exist and pass.

No product ambiguity remains on formulas, source DB, historical display, failure behavior, UI columns, sound fallback, or out-of-scope features.
`)
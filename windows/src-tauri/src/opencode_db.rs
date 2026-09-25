//! OpenCode live usage: pure math + read-only session row + disk snapshots.
//! Phase 1: no Cargo SQLite crates — Windows winsqlite3 FFI only (Task 2).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

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
    t.input
        .saturating_add(t.output)
        .saturating_add(t.cache_read)
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

pub fn normalize_session_id(hook_session: &str) -> String {
    hook_session
        .strip_prefix("opencode:")
        .unwrap_or(hook_session)
        .to_string()
}

/// True when `id` is an OpenCode DB session PK (`ses_…`), not a directory fallback.
pub fn is_opencode_session_id(id: &str) -> bool {
    let id = id.strip_prefix("opencode:").unwrap_or(id);
    id.starts_with("ses_") && id.len() > 4
}

pub fn default_db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".local").join("share").join("opencode").join("opencode.db"))
}

pub fn snapshot_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("AgentPet").join("opencode_token_snapshots.json"))
}

/// Process-wide snapshot store (write-first JSON under config dir).
pub fn snapshots() -> &'static SnapshotStore {
    static S: OnceLock<SnapshotStore> = OnceLock::new();
    S.get_or_init(SnapshotStore::load)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTokens {
    pub id: String,
    pub tokens: TokenTriple,
    pub directory: String,
    pub agent: String,
}

#[cfg(windows)]
#[link(name = "winsqlite3", kind = "raw-dylib")]
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
    fn sqlite3_column_text(stmt: *mut std::ffi::c_void, iCol: i32) -> *const u8;
    fn sqlite3_column_bytes(stmt: *mut std::ffi::c_void, iCol: i32) -> i32;
    fn sqlite3_finalize(stmt: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
const SQLITE_OPEN_READONLY: i32 = 0x00000001;
#[cfg(windows)]
const SQLITE_OPEN_URI: i32 = 0x00000040;
#[cfg(windows)]
const SQLITE_ROW: i32 = 100;
#[cfg(windows)]
const SQLITE_OK: i32 = 0;
#[cfg(windows)]
const SQLITE_TRANSIENT: isize = -1;

#[cfg(not(windows))]
pub fn read_session_row(_db_path: &Path, _session_id: &str) -> Option<SessionTokens> {
    None
}

/// Compatibility wrapper: tokens only.
pub fn read_session_tokens(db_path: &Path, session_id: &str) -> Option<TokenTriple> {
    read_session_row(db_path, session_id).map(|r| r.tokens)
}

#[cfg(windows)]
pub fn read_session_row(db_path: &Path, session_id: &str) -> Option<SessionTokens> {
    use std::ffi::CString;

    if !db_path.exists() {
        return None;
    }
    let uri = path_to_sqlite_uri(db_path)?;
    let c_sql = CString::new(
        "SELECT id, tokens_input, tokens_output, tokens_cache_read, directory, agent FROM session WHERE id = ?1 LIMIT 1;",
    )
    .ok()?;
    let c_sid = CString::new(session_id).ok()?;
    open_query_one_row(&uri, &c_sql, &c_sid)
}

/// Latest session row for a workspace directory (`time_updated DESC`). Read-only.
#[cfg(windows)]
pub fn read_latest_session_for_directory(db_path: &Path, directory: &str) -> Option<SessionTokens> {
    use std::ffi::CString;

    if directory.is_empty() || !db_path.exists() {
        return None;
    }
    let uri = path_to_sqlite_uri(db_path)?;
    let c_sql = CString::new(
        "SELECT id, tokens_input, tokens_output, tokens_cache_read, directory, agent FROM session WHERE directory = ?1 ORDER BY time_updated DESC LIMIT 1;",
    )
    .ok()?;
    let c_dir = CString::new(directory).ok()?;
    open_query_one_row(&uri, &c_sql, &c_dir)
}

#[cfg(not(windows))]
pub fn read_latest_session_for_directory(_db_path: &Path, _directory: &str) -> Option<SessionTokens> {
    None
}

/// Resolve tokens row: real `ses_…` id by PK; directory / non-session string → latest row for that dir.
pub fn resolve_session_tokens(
    db_path: &Path,
    normalized_id: &str,
    project_hint: &str,
) -> Option<SessionTokens> {
    if normalized_id.is_empty() {
        return None;
    }
    if is_opencode_session_id(normalized_id) {
        return read_session_row(db_path, normalized_id);
    }
    let dir = if !project_hint.is_empty() {
        project_hint
    } else {
        normalized_id
    };
    if dir.is_empty() || is_opencode_session_id(dir) {
        return None;
    }
    read_latest_session_for_directory(db_path, dir)
}

#[cfg(windows)]
fn open_query_one_row(
    uri: &str,
    c_sql: &std::ffi::CString,
    bind1: &std::ffi::CString,
) -> Option<SessionTokens> {
    use std::ffi::CString;
    use std::ptr;

    let c_uri = CString::new(uri).ok()?;
    unsafe {
        let mut db: *mut std::ffi::c_void = ptr::null_mut();
        let flags = SQLITE_OPEN_READONLY | SQLITE_OPEN_URI;
        let rc = sqlite3_open_v2(c_uri.as_ptr(), &mut db, flags, ptr::null());
        if rc != SQLITE_OK || db.is_null() {
            if !db.is_null() {
                let _ = sqlite3_close(db);
            }
            return None;
        }

        let mut stmt: *mut std::ffi::c_void = ptr::null_mut();
        let rc = sqlite3_prepare_v2(db, c_sql.as_ptr(), -1, &mut stmt, ptr::null_mut());
        if rc != SQLITE_OK || stmt.is_null() {
            if !stmt.is_null() {
                let _ = sqlite3_finalize(stmt);
            }
            let _ = sqlite3_close(db);
            return None;
        }

        let rc = sqlite3_bind_text(stmt, 1, bind1.as_ptr(), -1, SQLITE_TRANSIENT);
        if rc != SQLITE_OK {
            let _ = sqlite3_finalize(stmt);
            let _ = sqlite3_close(db);
            return None;
        }

        let step = sqlite3_step(stmt);
        let result = if step == SQLITE_ROW {
            let id = column_text(stmt, 0);
            let input = sqlite3_column_int64(stmt, 1).max(0) as u64;
            let output = sqlite3_column_int64(stmt, 2).max(0) as u64;
            let cache_read = sqlite3_column_int64(stmt, 3).max(0) as u64;
            Some(SessionTokens {
                id,
                tokens: TokenTriple {
                    input,
                    output,
                    cache_read,
                },
                directory: column_text(stmt, 4),
                agent: column_text(stmt, 5),
            })
        } else {
            None
        };

        let _ = sqlite3_finalize(stmt);
        let _ = sqlite3_close(db);
        result
    }
}

#[cfg(windows)]
unsafe fn column_text(stmt: *mut std::ffi::c_void, col: i32) -> String {
    let ptr = sqlite3_column_text(stmt, col);
    if ptr.is_null() {
        return String::new();
    }
    let len = sqlite3_column_bytes(stmt, col);
    if len <= 0 {
        return String::new();
    }
    let slice = std::slice::from_raw_parts(ptr, len as usize);
    String::from_utf8_lossy(slice).into_owned()
}

/// Encode path for `file:<encoded>?mode=ro` so `#`, `%`, `?`, spaces stay path bytes.
#[cfg(windows)]
fn path_to_sqlite_uri(db_path: &Path) -> Option<String> {
    let path_str = db_path.to_str()?;
    let path_fwd = path_str.replace('\\', "/");
    let mut encoded = String::with_capacity(path_fwd.len() + 8);
    for b in path_fwd.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'/'
            | b':'
            | b'-'
            | b'_'
            | b'.'
            | b'~' => encoded.push(b as char),
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                encoded.push('%');
                encoded.push(HEX[(b >> 4) as usize] as char);
                encoded.push(HEX[(b & 0xf) as usize] as char);
            }
        }
    }
    Some(format!("file:{}?mode=ro", encoded))
}

/// Serialize `next`, write unique sibling temp, sync, atomically replace `path`.
/// On any failure: best-effort delete temp, leave target untouched, return false.
fn persist_snapshot_map(path: &Path, next: &HashMap<String, TokenTriple>) -> bool {
    #[cfg(test)]
    if tests::persist_fail_armed() {
        return false;
    }
    if let Some(dir) = path.parent() {
        if fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    let Ok(body) = serde_json::to_string_pretty(next) else {
        return false;
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let base = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("opencode_token_snapshots.json");
    let temp = match path.parent() {
        Some(dir) => dir.join(format!(
            "{}.{}.{}.tmp",
            base,
            std::process::id(),
            stamp
        )),
        None => PathBuf::from(format!("{}.{}.{}.tmp", base, std::process::id(), stamp)),
    };
    let ok = (|| {
        let mut f = fs::File::create(&temp).ok()?;
        f.write_all(body.as_bytes()).ok()?;
        f.sync_all().ok()?;
        drop(f);
        fs::rename(&temp, path).ok()?;
        Some(())
    })()
    .is_some();
    if !ok {
        let _ = fs::remove_file(&temp);
    }
    ok
}

/// Load result: only a missing snapshot file is an empty baseline.
/// Path unavailable / read error / corrupt JSON → disabled (no deltas, no overwrite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotLoadKind {
    EmptyBaseline,
    Loaded,
    Disabled,
}

#[derive(Default)]
pub struct SnapshotStore {
    path: Option<PathBuf>,
    /// When true, token accounting is off for this process/store; never mutate baseline.
    disabled: bool,
    map: Mutex<HashMap<String, TokenTriple>>,
}

impl SnapshotStore {
    pub fn load() -> Self {
        let (kind, path, map) = Self::load_from(snapshot_path());
        Self {
            path,
            disabled: kind == SnapshotLoadKind::Disabled,
            map: Mutex::new(map),
        }
    }

    /// Pure load policy (path + file bytes). Used by `load` and unit tests.
    fn load_from(path: Option<PathBuf>) -> (SnapshotLoadKind, Option<PathBuf>, HashMap<String, TokenTriple>) {
        let Some(path) = path else {
            return (SnapshotLoadKind::Disabled, None, HashMap::new());
        };
        if !path.exists() {
            // Missing file only → empty baseline (first-seen zero).
            return (SnapshotLoadKind::EmptyBaseline, Some(path), HashMap::new());
        }
        match fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<HashMap<String, TokenTriple>>(&s) {
                Ok(map) => (SnapshotLoadKind::Loaded, Some(path), map),
                Err(_) => (SnapshotLoadKind::Disabled, Some(path), HashMap::new()),
            },
            Err(_) => (SnapshotLoadKind::Disabled, Some(path), HashMap::new()),
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn get(&self, session_id: &str) -> Option<TokenTriple> {
        if self.disabled {
            return None;
        }
        self.map.lock().ok()?.get(session_id).copied()
    }

    /// Write-first / commit-memory-after-success for an absolute triple (no delta).
    pub fn put_and_save(&self, session_id: &str, triple: TokenTriple) -> bool {
        if self.disabled {
            return false;
        }
        let Ok(mut guard) = self.map.lock() else {
            return false;
        };
        let mut next = guard.clone();
        next.insert(session_id.to_string(), triple);
        if let Some(ref p) = self.path {
            if !persist_snapshot_map(p, &next) {
                return false;
            }
        }
        *guard = next;
        true
    }

    /// Write-first / commit-memory-after-success; snapshot is monotonic per session.
    /// 1) Compute delta against current in-memory snapshot (do not mutate yet).
    /// 2) Build next map with fieldwise max(current, snapshot) so a stale/lower read
    ///    never replaces a higher stored absolute field.
    /// 3) Serialize full map and write JSON to disk; on any create_dir/serialize/write
    ///    failure: leave memory and disk unchanged, return None (no emit; later hook retries).
    /// 4) Only after successful write: commit next map into memory.
    /// 5) Return Some(delta) only when delta.total > 0 OR delta.xp > 0 (stale/lower
    ///    reads yield zero delta → None; mixed per-field growth emits only positive fields).
    /// On lock failure or disabled store: return None; memory/disk unchanged.
    pub fn compute_and_commit(&self, session_id: &str, current: TokenTriple) -> Option<TokenDelta> {
        if self.disabled {
            return None;
        }
        let mut guard = self.map.lock().ok()?;
        let snap = guard.get(session_id).copied().unwrap_or_default();
        let delta = compute_delta(&current, &snap);
        // Fieldwise max: stale lower absolutes must not rewind the watermark.
        let merged = TokenTriple {
            input: current.input.max(snap.input),
            output: current.output.max(snap.output),
            cache_read: current.cache_read.max(snap.cache_read),
        };
        // Prepare next map without committing memory until disk write succeeds.
        let mut next = guard.clone();
        next.insert(session_id.to_string(), merged);
        if let Some(ref p) = self.path {
            if !persist_snapshot_map(p, &next) {
                return None;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Test-only: force `persist_snapshot_map` to fail on this thread only.
    thread_local! {
        static PERSIST_FAIL: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn persist_fail_armed() -> bool {
        PERSIST_FAIL.with(|c| c.get())
    }

    /// Arms thread-local PERSIST_FAIL; Drop restores false even if a panic unwinds.
    struct PersistFailGuard;

    impl PersistFailGuard {
        fn arm() -> Self {
            PERSIST_FAIL.with(|c| c.set(true));
            Self
        }
    }

    impl Drop for PersistFailGuard {
        fn drop(&mut self) {
            PERSIST_FAIL.with(|c| c.set(false));
        }
    }

    #[test]
    fn delta_positive_growth() {
        let snap = TokenTriple {
            input: 10,
            output: 5,
            cache_read: 2,
        };
        let cur = TokenTriple {
            input: 15,
            output: 8,
            cache_read: 4,
        };
        let d = compute_delta(&cur, &snap);
        assert_eq!(d.input, 5);
        assert_eq!(d.output, 3);
        assert_eq!(d.cache, 2);
        assert_eq!(d.total, 10);
        assert_eq!(d.xp, 8);
    }

    #[test]
    fn delta_clamps_when_current_below_snapshot() {
        let snap = TokenTriple {
            input: 100,
            output: 50,
            cache_read: 20,
        };
        let cur = TokenTriple {
            input: 90,
            output: 40,
            cache_read: 10,
        };
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
        let snap = TokenTriple {
            input: 10,
            output: 5,
            cache_read: 2,
        };
        let cur = TokenTriple {
            input: 10,
            output: 5,
            cache_read: 100,
        };
        let d = compute_delta(&cur, &snap);
        assert_eq!(d.input, 0);
        assert_eq!(d.output, 0);
        assert_eq!(d.cache, 98);
        assert_eq!(d.total, 98);
        assert_eq!(d.xp, 0);
    }

    #[test]
    fn missing_snapshot_treated_as_zero_baseline() {
        let cur = TokenTriple {
            input: 15,
            output: 8,
            cache_read: 4,
        };
        let d = compute_delta(&cur, &TokenTriple::default());
        assert_eq!(d.input, 15);
        assert_eq!(d.output, 8);
        assert_eq!(d.cache, 4);
        assert_eq!(d.total, 27);
        assert_eq!(d.xp, 23);
    }

    #[test]
    fn saturates_at_u64_max() {
        let max = TokenTriple {
            input: u64::MAX,
            output: u64::MAX,
            cache_read: u64::MAX,
        };
        assert_eq!(usage_total(&max), u64::MAX);
        assert_eq!(pet_xp(u64::MAX, u64::MAX), u64::MAX);
        let d = compute_delta(&max, &TokenTriple::default());
        assert_eq!(d.total, u64::MAX);
        assert_eq!(d.xp, u64::MAX);
    }

    #[test]
    fn normalize_strips_opencode_prefix() {
        assert_eq!(normalize_session_id("opencode:ses_abc"), "ses_abc");
        assert_eq!(normalize_session_id("ses_abc"), "ses_abc");
    }

    #[test]
    fn is_opencode_session_id_accepts_ses_only() {
        assert!(is_opencode_session_id("ses_abc"));
        assert!(is_opencode_session_id("opencode:ses_abc"));
        assert!(!is_opencode_session_id(""));
        assert!(!is_opencode_session_id("ses_"));
        assert!(!is_opencode_session_id("default"));
        assert!(!is_opencode_session_id(r"E:\agentpet\source"));
        assert!(!is_opencode_session_id("opencode:E:\\agentpet\\source"));
    }

    #[test]
    fn snapshot_load_missing_file_is_empty_baseline() {
        let dir = unique_temp_dir("snap-miss");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("opencode_token_snapshots.json");
        assert!(!path.exists());
        let (kind, p, map) = SnapshotStore::load_from(Some(path.clone()));
        assert_eq!(kind, SnapshotLoadKind::EmptyBaseline);
        assert_eq!(p, Some(path));
        assert!(map.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn snapshot_load_no_config_path_disables() {
        let (kind, p, map) = SnapshotStore::load_from(None);
        assert_eq!(kind, SnapshotLoadKind::Disabled);
        assert!(p.is_none());
        assert!(map.is_empty());
    }

    #[test]
    fn snapshot_load_corrupt_json_disables_and_does_not_emit() {
        let dir = unique_temp_dir("snap-corrupt");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("opencode_token_snapshots.json");
        let raw = b"{not valid json";
        fs::write(&path, raw).unwrap();
        let (kind, p, map) = SnapshotStore::load_from(Some(path.clone()));
        assert_eq!(kind, SnapshotLoadKind::Disabled);
        assert_eq!(p, Some(path.clone()));
        assert!(map.is_empty());
        // File must not be overwritten by load policy.
        assert_eq!(fs::read(&path).unwrap(), raw);
        let store = SnapshotStore {
            path: Some(path.clone()),
            disabled: true,
            map: Mutex::new(HashMap::new()),
        };
        let cur = TokenTriple {
            input: 15,
            output: 8,
            cache_read: 4,
        };
        assert!(store.compute_and_commit("ses_x", cur).is_none());
        assert!(!store.put_and_save("ses_x", cur));
        assert_eq!(fs::read(&path).unwrap(), raw);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn snapshot_load_read_failure_disables() {
        // Path exists as a directory → read_to_string fails → disabled.
        let dir = unique_temp_dir("snap-readfail");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("as_dir");
        fs::create_dir_all(&path).unwrap();
        let (kind, p, map) = SnapshotStore::load_from(Some(path.clone()));
        assert_eq!(kind, SnapshotLoadKind::Disabled);
        assert_eq!(p, Some(path));
        assert!(map.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "ap-{}-{}-{}",
            label,
            std::process::id(),
            stamp
        ))
    }

    #[test]
    fn snapshot_commit_avoids_duplicate_on_second_same_current() {
        let dir = unique_temp_dir("snap");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("opencode_token_snapshots.json");
        let store = SnapshotStore {
            path: Some(path.clone()),
            disabled: false,
            map: Mutex::new(HashMap::new()),
        };
        let cur = TokenTriple {
            input: 15,
            output: 8,
            cache_read: 4,
        };
        let d1 = store.compute_and_commit("ses_x", cur).expect("first");
        assert_eq!(d1.total, 27);
        assert_eq!(d1.xp, 23);
        let d2 = store.compute_and_commit("ses_x", cur);
        assert!(d2.is_none(), "same absolute counters must not return another delta");
        let d3 = store
            .compute_and_commit(
                "ses_x",
                TokenTriple {
                    input: 20,
                    output: 8,
                    cache_read: 4,
                },
            )
            .expect("growth");
        assert_eq!(d3.input, 5);
        assert_eq!(d3.xp, 5);
        assert_eq!(d3.total, 5);
        let _ = fs::remove_dir_all(dir);
    }

    /// Out-of-order / concurrent reads: 10→20 then stale 15 must not rewind watermark;
    /// later 20 must not re-emit the already-committed growth.
    #[test]
    fn snapshot_commit_monotonic_rejects_stale_and_avoids_duplicate() {
        let dir = unique_temp_dir("snap-mono");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("opencode_token_snapshots.json");
        let store = SnapshotStore {
            path: Some(path),
            disabled: false,
            map: Mutex::new(HashMap::new()),
        };
        let t10 = TokenTriple {
            input: 10,
            output: 10,
            cache_read: 10,
        };
        let t20 = TokenTriple {
            input: 20,
            output: 20,
            cache_read: 20,
        };
        let t15 = TokenTriple {
            input: 15,
            output: 15,
            cache_read: 15,
        };
        let d1 = store.compute_and_commit("ses_x", t10).expect("10");
        assert_eq!(d1.total, 30);
        let d2 = store.compute_and_commit("ses_x", t20).expect("20");
        assert_eq!(d2.total, 30);
        assert!(
            store.compute_and_commit("ses_x", t15).is_none(),
            "stale 15 must not re-emit"
        );
        assert_eq!(
            store.get("ses_x"),
            Some(t20),
            "watermark stays at 20 after stale 15"
        );
        assert!(
            store.compute_and_commit("ses_x", t20).is_none(),
            "repeat 20 must not duplicate"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// Mixed per-field growth: only fields above the watermark contribute delta;
    /// snapshot advances fieldwise max.
    #[test]
    fn snapshot_commit_mixed_field_growth() {
        let dir = unique_temp_dir("snap-mixed");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("opencode_token_snapshots.json");
        let store = SnapshotStore {
            path: Some(path),
            disabled: false,
            map: Mutex::new(HashMap::new()),
        };
        let base = TokenTriple {
            input: 10,
            output: 20,
            cache_read: 5,
        };
        store.compute_and_commit("ses_x", base).expect("base");
        let mixed = TokenTriple {
            input: 15,  // +5
            output: 18, // stale vs 20
            cache_read: 8, // +3
        };
        let d = store.compute_and_commit("ses_x", mixed).expect("mixed");
        assert_eq!(d.input, 5);
        assert_eq!(d.output, 0);
        assert_eq!(d.cache, 3);
        assert_eq!(d.total, 8);
        assert_eq!(d.xp, 5);
        assert_eq!(
            store.get("ses_x"),
            Some(TokenTriple {
                input: 15,
                output: 20,
                cache_read: 8,
            })
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// Persistence failure with an existing target must not advance memory or clobber
    /// that file; after injection clears, retry returns the original delta once.
    #[test]
    fn snapshot_persist_failure_keeps_file_and_baseline_retry_returns_delta_once() {
        let root = unique_temp_dir("snap-fail");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let baseline = TokenTriple {
            input: 10,
            output: 5,
            cache_read: 2,
        };
        let mut seed = HashMap::new();
        seed.insert("ses_x".to_string(), baseline);
        let seed_body = serde_json::to_string_pretty(&seed).unwrap();
        let seed_bytes = seed_body.as_bytes();

        let path = root.join("opencode_token_snapshots.json");
        fs::write(&path, seed_bytes).unwrap();

        let store = SnapshotStore {
            path: Some(path.clone()),
            disabled: false,
            map: Mutex::new(seed),
        };
        let cur = TokenTriple {
            input: 15,
            output: 8,
            cache_read: 4,
        };

        {
            let _fail = PersistFailGuard::arm();
            assert!(store.compute_and_commit("ses_x", cur).is_none());
            assert_eq!(store.get("ses_x"), Some(baseline));
            assert_eq!(fs::read(&path).unwrap(), seed_bytes);
            assert!(!store.put_and_save("ses_x", cur));
            assert_eq!(store.get("ses_x"), Some(baseline));
            assert_eq!(fs::read(&path).unwrap(), seed_bytes);
        }

        let d = store.compute_and_commit("ses_x", cur).expect("retry");
        assert_eq!(d.input, 5);
        assert_eq!(d.output, 3);
        assert_eq!(d.cache, 2);
        assert_eq!(d.total, 10);
        assert_eq!(d.xp, 8);
        assert!(store.compute_and_commit("ses_x", cur).is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_db_returns_none() {
        let p = PathBuf::from("C:\\this\\path\\does\\not\\exist\\opencode.db");
        assert!(read_session_tokens(&p, "ses_x").is_none());
    }

    #[test]
    fn read_failure_contract_is_none() {
        // Injectable: missing path → None (no panic)
        assert!(read_session_row(std::path::Path::new("Z:\\nope\\opencode.db"), "x").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn path_to_sqlite_uri_encodes_significant_bytes() {
        let p = PathBuf::from(r"C:\Users\a b\db#1%.db");
        let uri = path_to_sqlite_uri(&p).expect("uri");
        assert!(uri.starts_with("file:"));
        assert!(uri.ends_with("?mode=ro"));
        assert!(uri.contains("%20"), "space encoded: {uri}");
        assert!(uri.contains("%23"), "hash encoded: {uri}");
        assert!(uri.contains("%25"), "percent encoded: {uri}");
        assert!(!uri[..uri.len() - "?mode=ro".len()].contains('?') || uri.contains("%3F"));
        let with_q = path_to_sqlite_uri(Path::new(r"C:\q?x.db")).unwrap();
        assert!(with_q.contains("%3F"), "question encoded: {with_q}");
    }
}

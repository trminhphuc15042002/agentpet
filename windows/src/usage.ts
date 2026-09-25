// Per-project, per-agent token usage, ported from the macOS ProjectUsageStore.
// Data stays local until the user connects an optional web profile.

import { token as syncToken } from "./sync";

const BASE = "https://agentpet.thenightwatcher.online";
const STORE_KEY = "ap_usage";
const DIRTY_KEY = "ap_usage_dirty";

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

function fnv1a(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return "p" + (h >>> 0).toString(16).padStart(8, "0");
}

function projectIdentity(path: string): { id: string; name: string } {
  const parts = path.replace(/[\\/]+$/, "").split(/[\\/]/);
  const last = parts[parts.length - 1] || path;
  return { id: fnv1a(path), name: last.slice(0, 60) };
}

function today(): string {
  const d = new Date();
  const y = d.getFullYear(), m = d.getMonth() + 1, day = d.getDate();
  return y.toString().padStart(4, "0") + "-" + m.toString().padStart(2, "0") + "-" + day.toString().padStart(2, "0");
}

function load(): Record<string, UsageRow> {
  try { return JSON.parse(localStorage.getItem(STORE_KEY) || "{}"); } catch { return {}; }
}
function save(store: Record<string, UsageRow>) { localStorage.setItem(STORE_KEY, JSON.stringify(store)); }
function loadDirty(): Set<string> {
  try { return new Set(JSON.parse(localStorage.getItem(DIRTY_KEY) || "[]")); } catch { return new Set(); }
}
function saveDirty(d: Set<string>) { localStorage.setItem(DIRTY_KEY, JSON.stringify([...d])); }

function validUsageNum(n: unknown): n is number {
  return Number.isSafeInteger(n) && (n as number) >= 0;
}

/** Present optional fields must be safe non-neg ints; only undefined is absent → 0. null/invalid → reject. */
function optionalUsageField(n: unknown): number | null {
  if (n === undefined) return 0;
  return validUsageNum(n) ? n : null;
}

function isValidUsageRow(r: unknown): r is UsageRow {
  if (!r || typeof r !== "object") return false;
  const row = r as Record<string, unknown>;
  if (
    typeof row.projectId !== "string" ||
    typeof row.projectName !== "string" ||
    typeof row.agent !== "string" ||
    typeof row.day !== "string"
  ) return false;
  if (!validUsageNum(row.tokens) || !validUsageNum(row.sessions)) return false;
  if (optionalUsageField(row.input) === null || optionalUsageField(row.output) === null || optionalUsageField(row.cache) === null) {
    return false;
  }
  return true;
}

/** Returns the locally recorded rows, newest first. */
export function list(): UsageRow[] {
  return Object.values(load()).filter(isValidUsageRow).sort((a, b) =>
    b.day.localeCompare(a.day) || b.tokens - a.tokens || a.projectName.localeCompare(b.projectName) || a.agent.localeCompare(b.agent),
  );
}

function record(
  project: string,
  agent: string,
  tokens: number,
  sessions: number,
  breakdown?: { input: number; output: number; cache: number },
) {
  if (typeof project !== "string" || !project || typeof agent !== "string" || !agent) return;
  if (!validUsageNum(tokens) || !validUsageNum(sessions)) return;
  if (breakdown) {
    if (!validUsageNum(breakdown.input) || !validUsageNum(breakdown.output) || !validUsageNum(breakdown.cache)) return;
  }
  if (tokens <= 0 && sessions <= 0 && !(breakdown && (breakdown.input > 0 || breakdown.output > 0 || breakdown.cache > 0))) {
    return;
  }
  const { id, name } = projectIdentity(project);
  const day = today();
  const key = id + "|" + agent + "|" + day;
  const store = load();
  const existing = store[key];
  const reuse = isValidUsageRow(existing);
  let baseTokens = 0;
  let baseSessions = 0;
  let baseInput = 0;
  let baseOutput = 0;
  let baseCache = 0;
  if (reuse) {
    baseTokens = existing.tokens;
    baseSessions = existing.sessions;
    baseInput = existing.input ?? 0;
    baseOutput = existing.output ?? 0;
    baseCache = existing.cache ?? 0;
  }
  const nextTokens = baseTokens + tokens;
  const nextSessions = baseSessions + sessions;
  if (!validUsageNum(nextTokens) || !validUsageNum(nextSessions)) return;
  let nextInput: number | undefined;
  let nextOutput: number | undefined;
  let nextCache: number | undefined;
  if (breakdown) {
    nextInput = baseInput + breakdown.input;
    nextOutput = baseOutput + breakdown.output;
    nextCache = baseCache + breakdown.cache;
    if (!validUsageNum(nextInput) || !validUsageNum(nextOutput) || !validUsageNum(nextCache)) return;
  }
  // New writes always seed input/output/cache = 0; invalid existing is replaced, not mutated.
  const r: UsageRow = reuse
    ? { ...existing, projectName: name, tokens: nextTokens, sessions: nextSessions }
    : {
        projectId: id, projectName: name, agent, day,
        tokens: nextTokens, sessions: nextSessions,
        input: 0, output: 0, cache: 0,
      };
  if (breakdown) {
    r.input = nextInput;
    r.output = nextOutput;
    r.cache = nextCache;
  }
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

let pushTimer: number | undefined;
export function schedulePush(afterMs = 30_000) {
  if (!syncToken()) return;
  clearTimeout(pushTimer);
  pushTimer = window.setTimeout(() => { void push(); }, afterMs);
}

export async function push(): Promise<void> {
  const tok = syncToken();
  if (!tok) return;
  const dirty = loadDirty();
  if (!dirty.size) return;
  const store = load();
  const snapshot: UsageRow[] = [];
  for (const k of [...dirty]) {
    const row = store[k];
    if (isValidUsageRow(row)) snapshot.push(row);
    else dirty.delete(k);
  }
  saveDirty(dirty);
  if (!snapshot.length) return;
  try {
    const res = await fetch(BASE + "/api/usage/sync", {
      method: "POST",
      headers: { "content-type": "application/json", authorization: "Bearer " + tok },
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
    });
    if (res.status === 401) return;
    if (!res.ok) return;
    // Clear only rows unchanged since the snapshot, so tokens recorded during the
    // push stay dirty and get sent next time.
    const now = load();
    const still = loadDirty();
    for (const s of snapshot) {
      const k = s.projectId + "|" + s.agent + "|" + s.day;
      const cur = now[k];
      if (cur && cur.tokens === s.tokens && cur.sessions === s.sessions) still.delete(k);
    }
    saveDirty(still);
  } catch {}
}
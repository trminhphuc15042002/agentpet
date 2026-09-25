// Tracks live agent sessions and derives the pet's mood , a port of the macOS
// SessionStore + MoodResolver. The live activity text is formatted once per
// event (like the macOS app formats at payload-decode time), so the whimsical
// phrase doesn't re-roll on every render tick.

import { activityMessage } from "./activity";

export interface Session {
  agent: string;
  session: string;
  state: string;
  project: string;
  /// Live activity line frozen at event time ("Brewing…", "Editing X…").
  live: string;
  /// Conversation title from the transcript (Claude), when known.
  title: string;
  /// OpenCode agent/role currently driving the session (build, plan, a forge
  /// subagent like worker-high, ...). Empty for agents that do not report one.
  role: string;
  /// Model in use (OpenCode `session.model.selected`), e.g. "claude-sonnet-4-5".
  /// Empty for agents that do not report one.
  model: string;
  /// Cumulative USD cost reported for this session (OpenCode only), 0 otherwise.
  cost: number;
  tool: string;
  updatedAt: number;
  stateSince: number;
  /// Terminal this session runs in (for click-to-focus). Sticky.
  terminalProgram: string;
  terminalFocusUrl: string;
  /// A gated tool call awaiting the user's Allow/Deny, if any.
  pendingApproval?: { id: string; tool: string; summary: string };
  /// Live child agents inferred from Task/Agent dispatches and confirmed stops.
  /// This is data plumbing for Agent Party; it does not create a new session or
  /// affect the aggregate pet mood.
  subagents: Subagent[];
}

export interface Subagent {
  id: string;
  role: string;
  startedAt: number;
}

export interface SubagentEventPayload {
  action: "start" | "stop";
  session: string;
  id: string;
  role?: string;
  ts?: number;
}

export interface AgentEventPayload {
  agent: string;
  state: string;
  session: string;
  project: string;
  message: string;
  tool?: string;
  file?: string;
  desc?: string;
  event?: string;
  title?: string | null;
  role?: string;
  model?: string;
  cost?: number;
  ts?: number;
  terminalProgram?: string;
  terminalFocusUrl?: string;
}

const PRIORITY: Record<string, number> = { working: 4, waiting: 3, done: 2, registered: 1, idle: 0 };
// Timeouts mirror the macOS SessionStore: done sessions linger briefly, then
// drop; sessions that go quiet are removed (the agent died without a Stop).
const DONE_LINGER_MS = 30_000;
const STALE_ACTIVE_MS = 300_000;
const STALE_REGISTERED_MS = 90_000;

export class SessionStore {
  private sessions = new Map<string, Session>();
  // A subagent dispatch can arrive immediately before its parent session's
  // first working event. Keep it briefly so event ordering never loses it.
  private pendingSubagents = new Map<string, SubagentEventPayload[]>();

  update(e: AgentEventPayload) {
    const key = `${e.agent}:${e.session}`;
    // Queued events replay with their original timestamp so sessions that
    // ended while the app was closed prune instead of resurrecting.
    const now = e.ts && e.ts > 0 ? e.ts : Date.now();
    const prev = this.sessions.get(key);

    // Live activity: explicit description (Bash) wins, else the themed
    // formatter, else "Tool · file", else keep nothing (state label shows).
    const live =
      e.desc?.trim() ||
      activityMessage(e.event ?? "", e.tool ?? "", e.file || undefined, e.message) ||
      (e.tool && e.file ? `${e.tool} · ${basename(e.file)}` : "") ||
      (e.tool ? `Using ${e.tool}` : "") ||
      prev?.live ||
      "";

    const next: Session = {
      agent: e.agent,
      session: e.session,
      state: e.state,
      project: e.project || prev?.project || "",
      live,
      title: e.title ?? prev?.title ?? "",
      role: e.role ?? prev?.role ?? "",
      model: e.model ?? prev?.model ?? "",
      cost: e.cost ?? prev?.cost ?? 0,
      tool: e.tool ?? "",
      updatedAt: now,
      stateSince: prev && prev.state === e.state ? prev.stateSince : now,
      terminalProgram: e.terminalProgram || prev?.terminalProgram || "",
      terminalFocusUrl: e.terminalFocusUrl || prev?.terminalFocusUrl || "",
      subagents: prev?.subagents ?? [],
    };
    this.sessions.set(key, next);
    const pending = this.pendingSubagents.get(e.session);
    if (pending?.length) {
      this.pendingSubagents.delete(e.session);
      for (const child of pending) this.updateSubagent(child);
    }
  }

  /// Records an inferred subagent lifecycle event. A few agents only expose a
  /// completion id, so a stop without an exact id retires the oldest child.
  /// Bound the roster to avoid unbounded memory for a long-lived session.
  updateSubagent(e: SubagentEventPayload) {
    const now = e.ts && e.ts > 0 ? e.ts : Date.now();
    for (const s of this.sessions.values()) {
      if (s.session !== e.session) continue;
      const children = s.subagents ?? (s.subagents = []);
      if (e.action === "start") {
        if (children.some((child) => child.id === e.id)) return;
        children.push({ id: e.id, role: e.role || "Subagent", startedAt: now });
        if (children.length > 8) children.splice(0, children.length - 8);
      } else {
        const exact = children.findIndex((child) => child.id === e.id);
        children.splice(exact >= 0 ? exact : 0, 1);
      }
      return;
    }
    const pending = this.pendingSubagents.get(e.session) ?? [];
    pending.push(e);
    if (pending.length > 8) pending.splice(0, pending.length - 8);
    this.pendingSubagents.set(e.session, pending);
  }

  remove(session: string) {
    this.pendingSubagents.delete(session);
    for (const k of [...this.sessions.keys()]) {
      if (k.endsWith(`:${session}`)) this.sessions.delete(k);
    }
  }

  /// Attach / clear a pending approval on a session by its id (any agent).
  setApproval(session: string, approval: { id: string; tool: string; summary: string }) {
    for (const s of this.sessions.values()) {
      if (s.session === session) { s.pendingApproval = approval; return; }
    }
  }
  clearApproval(session: string) {
    for (const s of this.sessions.values()) {
      if (s.session === session) s.pendingApproval = undefined;
    }
  }

  /// Insert a session verbatim (snapshot sync between windows).
  seed(s: Session) {
    this.sessions.set(`${s.agent}:${s.session}`, s);
  }

  snapshot(): Session[] {
    return [...this.sessions.values()];
  }

  removeKey(key: string) {
    this.sessions.delete(key);
  }

  clear() {
    this.sessions.clear();
    this.pendingSubagents.clear();
  }

  /// Drop done/stale sessions; returns the list (highest priority first).
  active(): Session[] {
    const now = Date.now();
    for (const [k, s] of [...this.sessions]) {
      const quiet = now - s.updatedAt;
      if (s.state === "done" && quiet > DONE_LINGER_MS) this.sessions.delete(k);
      else if (s.state === "registered" && quiet > STALE_REGISTERED_MS) this.sessions.delete(k);
      else if ((s.state === "working" || s.state === "waiting") && quiet > STALE_ACTIVE_MS) this.sessions.delete(k);
      else s.subagents = s.subagents.filter((child) => now - child.startedAt <= STALE_ACTIVE_MS);
    }
    return [...this.sessions.values()].sort(
      (a, b) => (PRIORITY[b.state] ?? 0) - (PRIORITY[a.state] ?? 0) || b.updatedAt - a.updatedAt
    );
  }

  topState(): string {
    return this.active()[0]?.state ?? "idle";
  }
}

/// Aggregate pet mood (port of MoodResolver): running work wins; `registered`
/// (agent open but idle) is not "working". `celebrate` is a transient the
/// caller layers on top when entering done.
export function aggregateMood(sessions: Session[]): "working" | "waiting" | "done" | "idle" {
  if (sessions.some((s) => s.state === "working")) return "working";
  if (sessions.some((s) => s.state === "waiting")) return "waiting";
  if (sessions.some((s) => s.state === "done")) return "done";
  return "idle";
}

export function basename(p: string): string {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
}

/// Short display label for an agent kind (port of TickerFormatter.agentLabel).
export function agentLabel(kind: string): string {
  switch (kind) {
    case "claude": return "Claude";
    case "cursor": return "Cursor";
    case "codex": return "Codex";
    case "gemini": return "Gemini";
    case "opencode": return "Opencode";
    case "windsurf": return "Windsurf";
    case "antigravity": return "Antigravity";
    case "copilot": return "Copilot";
    case "kiro": return "Kiro";
    case "hermes": return "Hermes";
    case "openclaw": return "OpenClaw";
    // A custom agent hooked via `--agent <name>`: show its own name, not a
    // generic label, so distinct customs read distinctly (issue #56).
    default: return kind ? kind.charAt(0).toUpperCase() + kind.slice(1) : "Agent";
  }
}

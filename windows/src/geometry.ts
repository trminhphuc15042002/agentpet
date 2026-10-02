// Pure bubble / carousel / filter helpers (no DOM, no Tauri).
// Window clamp / DPI / persistence live in the Rust geometry module.

export function clamp(v: number, lo: number, hi: number): number {
  return lo > hi ? hi : Math.min(Math.max(v, lo), hi);
}

export function sizeDiffers(
  a: { w: number; h: number },
  b: { w: number; h: number },
  tol = 1,
): boolean {
  return Math.abs(a.w - b.w) > tol || Math.abs(a.h - b.h) > tol;
}

/// The live viewport, not a previous request, determines whether content fits.
export function shouldSendResize(
  next: { w: number; h: number },
  actual: { w: number; h: number },
  tol = 1,
): boolean {
  return sizeDiffers(next, actual, tol);
}

export interface BubbleLayout {
  bubbleShift: number;
  tailShift: number;
}

/// Port of PetWindowGeometry.bubbleLayout.
export function bubbleLayout(
  petOffset: number,
  windowWidth: number,
  bubbleWidth: number,
  bubbleInset = 12,
  tailClearance = 20,
): BubbleLayout {
  const room = Math.max(0, (windowWidth - bubbleWidth) / 2);
  const bubbleShift = clamp(petOffset, -room, room);
  const tailLimit = Math.max(0, (bubbleWidth - 2 * bubbleInset) / 2 - tailClearance);
  return { bubbleShift, tailShift: clamp(petOffset - bubbleShift, -tailLimit, tailLimit) };
}

export function carouselShown(index: number, count: number): number {
  return count > 0 ? ((index % count) + count) % count : 0;
}

export function carouselStep(index: number, by: number, count: number): number {
  if (count <= 0) return 0;
  return carouselShown(index + by, count);
}

export function sessionIsVisible(
  agent: string,
  state: string,
  hidden: string[],
  filter: string,
): boolean {
  if (hidden.includes(agent)) return false;
  switch (filter) {
    case "doneAndAbove": return state === "working" || state === "waiting" || state === "done";
    case "workingAndWaiting": return state === "working" || state === "waiting";
    case "workingOnly": return state === "working";
    default: return true;
  }
}

export function visibleSessions<T extends { agent: string; state: string }>(
  sessions: T[],
  hidden: string[],
  filter: string,
): T[] {
  return sessions.filter((s) => sessionIsVisible(s.agent, s.state, hidden, filter));
}

const RANK: Record<string, number> = { working: 4, waiting: 3, done: 2, registered: 1, idle: 0 };

export interface Groupable {
  agent: string;
  state: string;
  session: string;
  updatedAt: number;
}

export interface Group<T = Groupable> {
  session: T;
  count: number;
  id: string;
}

export interface GroupConfig {
  pinnedKey?: string;
  hidden: string[];
  filter: string;
  grouping: "byKind" | "all" | string;
  sortByKind: boolean;
  mode: "list" | "carousel" | "compact" | string;
  maxSessions: number;
}

/// Displayed session + party roster. Maintenance uses this so pruning a stale
/// child still invalidates when the top-level session count is unchanged.
export function maintenanceSig(
  sessions: Array<{ session: string; state: string; subagents?: Array<{ id: string }> }>,
): string {
  return sessions
    .map((s) => {
      const kids = s.subagents ?? [];
      return `${s.session}\t${s.state}\t${kids.length}\t${kids.map((c) => c.id).join(",")}`;
    })
    .join("\n");
}

export function groupSessions<T extends Groupable>(sessions: T[], cfg: GroupConfig): Group<T>[] {
  const filtered = visibleSessions(sessions, cfg.hidden, cfg.filter);
  const sortByKind = cfg.grouping === "byKind" || cfg.sortByKind;
  const sorted = [...filtered].sort((a, b) => {
    if (sortByKind && a.agent !== b.agent) return a.agent < b.agent ? -1 : 1;
    if ((RANK[a.state] ?? 0) !== (RANK[b.state] ?? 0)) return (RANK[b.state] ?? 0) - (RANK[a.state] ?? 0);
    return b.updatedAt - a.updatedAt;
  });

  let groups: Group<T>[];
  const pinned = sorted.find((s) => `${s.agent}:${s.session}` === cfg.pinnedKey);
  const rest = pinned ? sorted.filter((s) => s !== pinned) : sorted;
  if (cfg.grouping === "byKind") {
    const seen = new Map<string, number>();
    groups = [];
    for (const s of rest) {
      const idx = seen.get(s.agent);
      if (idx !== undefined) {
        groups[idx] = { ...groups[idx], count: groups[idx].count + 1 };
      } else {
        seen.set(s.agent, groups.length);
        groups.push({ session: s, count: 1, id: `${s.agent}-${s.session}` });
      }
    }
  } else {
    groups = rest.map((s) => ({ session: s, count: 1, id: `${s.agent}-${s.session}` }));
  }
  if (pinned) groups.unshift({ session: pinned, count: 1, id: `${pinned.agent}-${pinned.session}` });

  if (cfg.mode === "carousel") return groups;
  return groups.slice(0, cfg.maxSessions);
}

// Small, pure policies shared by the overlay and its controls. No polling.
export const WAIT_REMINDER_MS = 120_000;
export const sessionKey = (s: { agent: string; session: string }) => `${s.agent}:${s.session}`;

export function focusActive(until: number, now = Date.now()): boolean {
  return Number.isFinite(until) && until > now;
}

export interface WaitingSession {
  agent: string;
  session: string;
  state: string;
  stateSince: number;
  updatedAt: number;
}

export class WaitingReminders {
  private seen = new Map<string, number>();

  private pending(sessions: WaitingSession[]) {
    const waiting = sessions.filter((s) => s.state === "waiting");
    const live = new Map(waiting.map((s) => [sessionKey(s), s.stateSince]));
    for (const [key, since] of this.seen) if (live.get(key) !== since) this.seen.delete(key);
    return waiting.filter((s) => this.seen.get(sessionKey(s)) !== s.stateSince);
  }

  nextDelay(sessions: WaitingSession[], now: number): number | null {
    const pending = this.pending(sessions).filter((s) => Number.isFinite(s.stateSince) && s.stateSince <= now);
    return pending.length ? Math.max(0, Math.min(...pending.map((s) => s.stateSince + WAIT_REMINDER_MS - now))) : null;
  }

  takeDue<T extends WaitingSession>(sessions: T[], now: number): T[] {
    return this.pending(sessions).filter((s) => {
      const age = now - s.stateSince;
      if (age < WAIT_REMINDER_MS) return false;
      this.seen.set(sessionKey(s), s.stateSince);
      // Do not replay old reminders after sleep or resurrect a stale session.
      return age <= WAIT_REMINDER_MS + 60_000 && now - s.updatedAt < 300_000;
    }) as T[];
  }
}

export function readLastEvents(raw: string | null): Record<string, number> {
  try {
    const value = JSON.parse(raw || "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).filter((entry): entry is [string, number] =>
      /^[a-z0-9_-]{1,40}$/i.test(entry[0]) && typeof entry[1] === "number" && Number.isFinite(entry[1]) && entry[1] > 0));
  } catch { return {}; }
}

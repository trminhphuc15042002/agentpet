import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { isFresh, mayFetch } from "./limitTiers";

export interface LimitWindow { label: string; usedPct: number; resetsAtMs: number | null }
export interface ProviderLimit { id: string; name: string; windows: LimitWindow[] }

const KEY = "ap_limits"; // opt-in, off by default
const ON_KEY = "ap_limits_on_at";

let cache: ProviderLimit[] = [];
let fetchedAt: number | null = null; // performance.now() at call start (may be negative in a mirrored copy); null = never
let fetchedOn: string | null = null; // ap_limits_on_at captured at call start; data is valid only for that ON epoch
let lastCallAt: number | null = null; // budget clock (performance.now); only refreshLimits sets it, never reset

export const limitsEnabled = () => localStorage.getItem(KEY) === "1";

/** Toggle-ON epoch written by settings; null if unknown. */
const onAt = (): string | null => localStorage.getItem(ON_KEY);

const drop = () => { cache = []; fetchedAt = null; fetchedOn = null; };

/** Cached result; empty whenever the toggle is off, from another ON epoch, or too old. The single age rule. */
export function cachedLimits(): ProviderLimit[] {
  if (!limitsEnabled() || fetchedOn !== onAt()) { drop(); return []; }
  if (!isFresh(fetchedAt, performance.now())) return [];
  return cache;
}

/** Smallest fraction left (0..1) across all windows of the cached data, or null when off/empty. */
export function lowestFractionLeft(): number | null {
  const used = cachedLimits().flatMap((p) => p.windows.map((w) => w.usedPct)).filter(Number.isFinite);
  return used.length ? Math.max(0, 1 - Math.max(...used) / 100) : null;
}

/** A window has under 15% left (drives the anxious idle lines). */
export const limitLow = () => (lowestFractionLeft() ?? 1) < 0.15;

/** Keep only finite numbers; the list crosses a window boundary so it is not trusted. */
function sanitize(r: unknown): ProviderLimit[] {
  if (!Array.isArray(r)) return [];
  return r.map((p: ProviderLimit) => ({
    id: String(p?.id ?? ""), name: String(p?.name ?? ""),
    windows: (Array.isArray(p?.windows) ? p.windows : [])
      .filter((w) => Number.isFinite(w?.usedPct))
      .map((w) => ({ label: String(w.label ?? ""), usedPct: w.usedPct, resetsAtMs: Number.isFinite(w.resetsAtMs) ? w.resetsAtMs : null })),
  }));
}

/** Main window only: the single caller of usage_limits (toggle-gated, one call per 5 min) and the broadcaster. */
export async function refreshLimits(): Promise<void> {
  if (!limitsEnabled()) {
    const had = cache.length > 0;
    drop();
    if (had) void emit("limits-updated", payload());
    return;
  }
  const now = performance.now();
  if (!mayFetch(lastCallAt, now)) return;
  lastCallAt = now;
  const on0 = onAt();
  try {
    const r = sanitize(await invoke<ProviderLimit[]>("usage_limits"));
    if (!limitsEnabled() || onAt() !== on0) return; // toggled off (or off and on again) while in flight
    cache = r;
    fetchedAt = now;
    fetchedOn = on0;
  } catch {
    drop();
  }
  void emit("limits-updated", payload());
}

/** Age travels as a duration: performance.now() is per-webview. */
const payload = () => {
  const providers = cachedLimits();
  return { ageMs: providers.length && fetchedAt !== null ? performance.now() - fetchedAt : 0, fetchedOn, providers };
};

/** Main window: answer popover requests from current state, never a new fetch. */
export function serveLimits(): void {
  void listen("limits-request", () => void emit("limits-updated", payload()));
}

/** Popover: mirror what main broadcasts, then ask main for its latest. */
export function followLimits(onChange: () => void): void {
  void listen<{ ageMs?: number; fetchedOn?: string | null; providers?: ProviderLimit[] }>("limits-updated", (e) => {
    const p = e.payload;
    if (limitsEnabled() && Number.isFinite(p?.ageMs)) {
      cache = sanitize(p?.providers);
      fetchedAt = performance.now() - (p!.ageMs as number);
      fetchedOn = p?.fetchedOn ?? null;
    } else drop();
    onChange();
  });
  void emit("limits-request", null);
}

/** Popover: re-ask main (no fetch) when shown again. */
export const requestLimits = () => void emit("limits-request", null);

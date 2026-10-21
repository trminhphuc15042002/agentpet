// Rate-limit reaction tiers on the fraction of budget LEFT (mac ReactiveEngine.Thresholds.RateLimit).
// Import-free so scripts/check-companion.ts can run it under plain node.
export const RATE_LIMIT_TH = { silent: 0.5, low: 0.15, high: 0.05 };

/// Phrase key for the tightest window's fraction left; null while above 50% or unknown.
export function rateLimitKey(left: number | null): string | null {
  if (left == null || !Number.isFinite(left) || left > RATE_LIMIT_TH.silent) return null;
  if (left > RATE_LIMIT_TH.low) return "rateLimitLow";
  if (left > RATE_LIMIT_TH.high) return "rateLimitHigh";
  return "rateLimitCritical";
}

export const TTL_MS = 5 * 60_000; // one usage_limits call per this
export const MAX_AGE_MS = TTL_MS + 2 * 60_000; // older data reads as empty
// isFresh/mayFetch take performance.now() values (monotonic), never wall-clock.
/// null = never fetched. A mirrored copy in a younger webview can have a negative stamp; that is valid.
export const isFresh = (fetchedAt: number | null, now: number) => fetchedAt !== null && now >= fetchedAt && now - fetchedAt <= MAX_AGE_MS;
/// null = never called. A clock that went backwards never allows a fetch.
export const mayFetch = (lastCallAt: number | null, now: number) => lastCallAt === null || (now >= lastCallAt && now - lastCallAt >= TTL_MS);

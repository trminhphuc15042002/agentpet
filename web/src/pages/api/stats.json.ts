import type { APIRoute } from "astro";
import { getDB, ensureSchema } from "../../lib/db";

export const prerender = false;

// Install and download counts are full-table reads (thousands of rows) that move
// slowly, so they are cached for 10 minutes in the colo-wide Cache API and per
// isolate. The response's cache-control header alone does not help, because
// Cloudflare does not edge-cache Worker responses by default: every page load used to
// re-read both tables and used up the D1 free daily read budget.
const COUNTS_TTL_S = 600;
const COUNTS_KEY = "https://agentpet.cache/api/stats-counts";
type Counts = { installs: Record<string, number>; downloads: Record<string, number> };
let isolateCounts: { at: number; value: Counts } | null = null;

async function loadCounts(db: any): Promise<Counts> {
  if (isolateCounts && Date.now() - isolateCounts.at < COUNTS_TTL_S * 1000) return isolateCounts.value;

  const edge: Cache | undefined = (globalThis as any).caches?.default;
  const hit = await edge?.match(COUNTS_KEY).catch(() => undefined);
  if (hit) {
    const value = (await hit.json()) as Counts;
    isolateCounts = { at: Date.now(), value };
    return value;
  }

  const installs: Record<string, number> = {};
  const downloads: Record<string, number> = {};
  const ins: any = await db.prepare("SELECT slug, count FROM pet_installs WHERE count > 0").all();
  for (const r of ins?.results ?? []) installs[r.slug] = r.count;
  const dl: any = await db.prepare("SELECT slug, count FROM pet_downloads WHERE count > 0").all();
  for (const r of dl?.results ?? []) downloads[r.slug] = r.count;

  const value = { installs, downloads };
  isolateCounts = { at: Date.now(), value };
  await edge
    ?.put(COUNTS_KEY, new Response(JSON.stringify(value), {
      headers: { "content-type": "application/json", "cache-control": `public, max-age=${COUNTS_TTL_S}` },
    }))
    .catch(() => {});
  return value;
}

// Public like counts (slug -> count) from the small aggregate table, read fresh so a
// new like shows up right away. Per-user liked state lives in /api/my-likes.json.
export const GET: APIRoute = async () => {
  const db = getDB();
  const likes: Record<string, number> = {};
  let counts: Counts = { installs: {}, downloads: {} };
  if (db) {
    await ensureSchema(db);
    const rows: any = await db.prepare("SELECT slug, likes FROM pet_stats WHERE likes > 0").all();
    for (const r of rows?.results ?? []) likes[r.slug] = r.likes;
    counts = await loadCounts(db);
  }
  return new Response(JSON.stringify({ likes, ...counts }), {
    headers: { "content-type": "application/json", "cache-control": "public, max-age=120" },
  });
};

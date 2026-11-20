import type { APIRoute } from "astro";
import { adminUser } from "../../../lib/admin";
import { getDB, ensureSchema } from "../../../lib/db";

export const prerender = false;

// Admin-only dashboard counters: how many people connected the app, how much
// they've raised, plus the submission queue and community totals.
let cache: { at: number; data: any } | null = null;
const respond = (d: unknown) => new Response(JSON.stringify(d), { headers: { "content-type": "application/json", "cache-control": "no-store" } });

export const GET: APIRoute = async ({ cookies }) => {
  const user = await adminUser(cookies);
  if (!user) return new Response(JSON.stringify({ error: "forbidden" }), { status: 403 });

  const db = getDB();
  if (!db) return new Response(JSON.stringify({}), { status: 200 });
  await ensureSchema(db);

  // Counters change slowly: reuse the last result for 60s per isolate.
  if (cache && Date.now() - cache.at < 60_000) return respond(cache.data);

  // One round trip instead of 12 separate queries.
  const dayAgo = Date.now() - 86_400_000;
  let r: any = {};
  try {
    r = (await db.prepare(`SELECT
      (SELECT COUNT(DISTINCT user_id) FROM care_devices) AS connectedUsers,
      (SELECT COUNT(*) FROM care_devices) AS devices,
      (SELECT COUNT(*) FROM care_pets) AS raisedPets,
      (SELECT COUNT(DISTINCT user_id) FROM care_pets) AS ownersWithPets,
      (SELECT COALESCE(SUM(tokens),0) FROM care_pets) AS totalTokens,
      (SELECT COALESCE(SUM(meals),0) FROM care_pets) AS totalSessions,
      (SELECT COUNT(DISTINCT user_id) FROM care_pets WHERE updated_at > ?) AS activeToday,
      (SELECT COALESCE(SUM(likes),0) FROM pet_stats) AS totalLikes,
      (SELECT COUNT(*) FROM pet_requests) AS requests,
      (SELECT COUNT(*) FROM submissions WHERE status='pending') AS subPending,
      (SELECT COUNT(*) FROM submissions WHERE status='approved') AS subApproved,
      (SELECT COUNT(*) FROM submissions WHERE status='rejected') AS subRejected`).bind(dayAgo).first()) || {};
  } catch {}
  const n = (k: string) => Number(r[k] ?? 0);
  const data = {
    connectedUsers: n("connectedUsers"), devices: n("devices"), raisedPets: n("raisedPets"), ownersWithPets: n("ownersWithPets"),
    totalTokens: n("totalTokens"), totalSessions: n("totalSessions"), activeToday: n("activeToday"),
    totalLikes: n("totalLikes"), requests: n("requests"),
    submissions: { pending: n("subPending"), approved: n("subApproved"), rejected: n("subRejected") },
    trend: [] as any[],
  };

  // Daily snapshot (first read of the day writes it) so /admin can chart growth.
  try {
    const day = new Date().toISOString().slice(0, 10);
    await db.prepare("INSERT OR IGNORE INTO admin_snapshots (day, data, created_at) VALUES (?,?,?)")
      .bind(day, JSON.stringify({ connectedUsers: data.connectedUsers, raisedPets: data.raisedPets, totalTokens: data.totalTokens, totalSessions: data.totalSessions, activeToday: data.activeToday }), Date.now()).run();
    const t: any = await db.prepare("SELECT day, data FROM admin_snapshots ORDER BY day DESC LIMIT 30").all();
    data.trend = (t?.results ?? []).reverse().map((x: any) => ({ day: x.day, ...JSON.parse(x.data) }));
  } catch {}

  cache = { at: Date.now(), data };
  return respond(data);
};

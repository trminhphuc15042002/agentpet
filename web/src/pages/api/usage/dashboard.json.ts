import type { APIRoute } from "astro";
import { env } from "cloudflare:workers";
import { verifySession, SESSION_COOKIE } from "../../../lib/auth";
import { getDB, ensureSchema } from "../../../lib/db";

export const prerender = false;

const v = (n: string): string => {
  try { const e = (env as any)?.[n]; if (e) return String(e); } catch {}
  return (import.meta as any).env?.[n] ?? "";
};
const empty = { projects: [], agents: [], daily: [], monthly: [], totalTokens: 0, totalSessions: 0, totalCost: 0 };
const json = (obj: unknown) => new Response(JSON.stringify(obj), { headers: { "content-type": "application/json", "cache-control": "no-store" } });

// The signed-in user's own usage for the /dashboard charts.
// Filters: ?project=<id>&agent=<name>&days=7|30|90|365 (default 30).
// `daily` is per day AND per agent (for stacked charts) and covers 2x the window,
// so the page can compare against the previous period without a second request.
export const GET: APIRoute = async ({ cookies, url }) => {
  const token = cookies.get(SESSION_COOKIE)?.value || "";
  const user = token ? await verifySession(token, v("SESSION_SECRET")) : null;
  if (!user) return json(empty);

  const db = getDB();
  if (!db) return json(empty);
  await ensureSchema(db);

  const project = url.searchParams.get("project") || "";
  const agent = url.searchParams.get("agent") || "";
  const reqDays = Number(url.searchParams.get("days"));
  const days = [7, 30, 90, 365].includes(reqDays) ? reqDays : 30;
  const since = new Date(Date.now() - 2 * days * 86_400_000).toISOString().slice(0, 10);
  // Month-to-date for the forecast card: `daily` may start after the 1st (e.g. 7d range).
  // ?month=YYYY-MM is the browser's local month (days are stored in the app's local time).
  const reqMonth = url.searchParams.get("month") || "";
  const monthStart = (/^\d{4}-\d{2}$/.test(reqMonth) ? reqMonth : new Date().toISOString().slice(0, 7)) + "-01";

  const where = ["user_id=?"];
  const args: any[] = [user.id];
  if (project) { where.push("project_id=?"); args.push(project); }
  if (agent) { where.push("agent=?"); args.push(agent); }
  const W = where.join(" AND ");
  const SUMS = "SUM(tokens) AS tokens, SUM(sessions) AS sessions, SUM(cost_usd) AS cost, SUM(input_tokens) AS input, SUM(output_tokens) AS output, SUM(cache_read) AS cacheRead, SUM(cache_write) AS cacheWrite";

  const [projects, agents, daily, monthly, totals, topProjects, mtd] = await Promise.all([
    db.prepare("SELECT project_id AS id, project_name AS name, SUM(tokens) AS tokens, SUM(sessions) AS sessions, SUM(cost_usd) AS cost FROM usage_daily WHERE user_id=? GROUP BY project_id ORDER BY tokens DESC LIMIT 200").bind(user.id).all(),
    db.prepare("SELECT agent, SUM(tokens) AS tokens, SUM(cost_usd) AS cost FROM usage_daily WHERE user_id=? GROUP BY agent ORDER BY tokens DESC").bind(user.id).all(),
    db.prepare(`SELECT day, agent, ${SUMS} FROM usage_daily WHERE ${W} AND day>=? GROUP BY day, agent ORDER BY day`).bind(...args, since).all(),
    db.prepare(`SELECT substr(day,1,7) AS month, ${SUMS} FROM usage_daily WHERE ${W} GROUP BY month ORDER BY month DESC LIMIT 24`).bind(...args).all(),
    db.prepare(`SELECT ${SUMS} FROM usage_daily WHERE ${W}`).bind(...args).first(),
    // Heaviest projects inside the selected window (for "top project" + cost per project).
    db.prepare(`SELECT project_id AS id, project_name AS name, SUM(tokens) AS tokens, SUM(cost_usd) AS cost FROM usage_daily WHERE ${W} AND day>=? GROUP BY project_id ORDER BY tokens DESC LIMIT 10`)
      .bind(...args, new Date(Date.now() - days * 86_400_000).toISOString().slice(0, 10)).all(),
    db.prepare(`SELECT SUM(tokens) AS tokens, SUM(cost_usd) AS cost FROM usage_daily WHERE ${W} AND day>=?`).bind(...args, monthStart).first(),
  ]);

  const t: any = totals || {};
  const m: any = mtd || {};
  return json({
    days,
    projects: projects?.results ?? [],
    agents: agents?.results ?? [],
    topProjects: topProjects?.results ?? [],
    daily: daily?.results ?? [],                    // oldest -> newest, per day per agent
    monthly: (monthly?.results ?? []).reverse(),
    totalTokens: t.tokens ?? 0,
    totalSessions: t.sessions ?? 0,
    totalCost: t.cost ?? 0,
    totalInput: t.input ?? 0, totalOutput: t.output ?? 0,
    totalCacheRead: t.cacheRead ?? 0, totalCacheWrite: t.cacheWrite ?? 0,
    monthToDate: { tokens: m.tokens ?? 0, cost: m.cost ?? 0 },
  });
};

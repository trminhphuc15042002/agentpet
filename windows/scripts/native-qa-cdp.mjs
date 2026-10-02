#!/usr/bin/env node
// WebView2 CDP helper (Node stdlib only). Usage:
//   node native-qa-cdp.mjs list <port>
//   node native-qa-cdp.mjs wait <port> <urlIncludes> <timeoutMs>
//   node native-qa-cdp.mjs eval <port> <urlIncludes> <expression>
//   node native-qa-cdp.mjs eval-all <port> <expression>
//   node native-qa-cdp.mjs fnv <path...>

import { Buffer } from "node:buffer";
import { readFileSync } from "node:fs";

const [cmd, ...rest] = process.argv.slice(2);

function fail(msg, extra) {
  console.error(JSON.stringify({ ok: false, error: msg, ...extra }));
  process.exit(1);
}

function fnv1a(s) {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return "p" + (h >>> 0).toString(16).padStart(8, "0");
}

async function listTargets(port) {
  const urls = [
    `http://127.0.0.1:${port}/json/list`,
    `http://127.0.0.1:${port}/json`,
  ];
  let lastErr;
  for (const url of urls) {
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(2000) });
      if (!res.ok) {
        lastErr = `${url} HTTP ${res.status}`;
        continue;
      }
      const data = await res.json();
      if (Array.isArray(data)) return data;
      lastErr = `${url} not an array`;
    } catch (e) {
      lastErr = String(e?.message || e);
    }
  }
  throw new Error(lastErr || "cdp list failed");
}

function pickTarget(targets, includes) {
  const pages = targets.filter((t) => t.webSocketDebuggerUrl);
  if (!includes || includes === "*") {
    return pages[0];
  }
  const needle = includes.toLowerCase();
  if (needle === "main") {
    return pages.find((t) => {
      const u = (t.url || "").toLowerCase();
      try {
        const url = new URL(u);
        return url.hostname === "tauri.localhost" &&
          (url.pathname === "/" || url.pathname === "/index.html") &&
          !url.searchParams.has("project");
      } catch { return false; }
    });
  }
  return pages.find((t) => (t.url || "").toLowerCase().includes(needle));
}

async function cdpCall(wsUrl, method, params = {}, timeoutMs = 4000) {
  const ws = new WebSocket(wsUrl);
  const id = 1;
  const result = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      try { ws.close(); } catch {}
      reject(new Error(`CDP timeout ${method}`));
    }, timeoutMs);
    ws.addEventListener("open", () => {
      ws.send(JSON.stringify({ id, method, params }));
    });
    ws.addEventListener("message", (ev) => {
      let msg;
      try { msg = JSON.parse(typeof ev.data === "string" ? ev.data : Buffer.from(ev.data).toString("utf8")); }
      catch { return; }
      if (msg.id !== id) return;
      clearTimeout(timer);
      try { ws.close(); } catch {}
      if (msg.error) reject(new Error(msg.error.message || JSON.stringify(msg.error)));
      else resolve(msg.result);
    });
    ws.addEventListener("error", () => {
      clearTimeout(timer);
      reject(new Error("CDP websocket error"));
    });
  });
  return result;
}

async function evaluate(port, includes, expression, all) {
  const targets = await listTargets(port);
  const chosen = all
    ? targets.filter((t) => t.webSocketDebuggerUrl)
    : [pickTarget(targets, includes)].filter(Boolean);
  if (!chosen.length) {
    fail("no matching target", { includes, targets: targets.map((t) => ({ url: t.url, title: t.title })) });
  }
  const out = [];
  for (const t of chosen) {
    const result = await cdpCall(t.webSocketDebuggerUrl, "Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
      timeout: 10000,
    }, 12000);
    out.push({
      url: t.url,
      title: t.title,
      value: result?.result?.value,
      type: result?.result?.type,
      subtype: result?.result?.subtype,
      description: result?.result?.description,
      exception: result?.exceptionDetails || null,
    });
  }
  return all ? out : out[0];
}

async function waitPage(port, includes, timeoutMs) {
  const start = Date.now();
  let last = [];
  while (Date.now() - start < timeoutMs) {
    try {
      last = await listTargets(port);
      const hit = pickTarget(last, includes);
      if (hit) return { ok: true, target: { url: hit.url, title: hit.title, id: hit.id } };
    } catch {
      last = [];
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  fail("wait timeout", { includes, timeoutMs, targets: last.map((t) => ({ url: t.url, title: t.title })) });
}

try {
  if (cmd === "fnv") {
    const paths = rest.length ? rest : [];
    const mapped = {};
    for (const p of paths) mapped[p] = fnv1a(p);
    console.log(JSON.stringify({ ok: true, ids: mapped }));
    process.exit(0);
  }
  const port = Number(rest[0]);
  if (!Number.isInteger(port) || port < 1) fail("port required");
  if (cmd === "list") {
    const targets = await listTargets(port);
    console.log(JSON.stringify({
      ok: true,
      targets: targets.map((t) => ({
        id: t.id,
        type: t.type,
        title: t.title,
        url: t.url,
        ws: Boolean(t.webSocketDebuggerUrl),
      })),
    }));
    process.exit(0);
  }
  if (cmd === "wait") {
    const includes = rest[1] || "index.html";
    const timeoutMs = Number(rest[2] || 20000);
    const hit = await waitPage(port, includes, timeoutMs);
    console.log(JSON.stringify(hit));
    process.exit(0);
  }
  if (cmd === "eval" || cmd === "eval-file") {
    const includes = rest[1];
    let expression;
    if (cmd === "eval-file") {
      const file = rest[2];
      if (!includes || !file) fail("eval-file requires urlIncludes and path");
      expression = readFileSync(file, "utf8");
    } else {
      expression = rest.slice(2).join(" ");
    }
    if (!includes || !expression) fail("eval requires urlIncludes and expression");
    const result = await evaluate(port, includes, expression, false);
    console.log(JSON.stringify({ ok: true, ...result }));
    process.exit(0);
  }
  if (cmd === "eval-all") {
    const expression = rest.slice(1).join(" ");
    if (!expression) fail("eval-all requires expression");
    const result = await evaluate(port, "*", expression, true);
    console.log(JSON.stringify({ ok: true, results: result }));
    process.exit(0);
  }
  fail(`unknown command ${cmd || ""}`);
} catch (e) {
  fail(String(e?.message || e));
}

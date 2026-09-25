// The tray/right-click popover , a port of the macOS MenuContentView: live
// agent list with dismiss + Clear all, Show pet toggle, pet-size slider, and
// a Settings / Updates / Quit footer. Hides itself when it loses focus.

import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch, exit } from "@tauri-apps/plugin-process";
import { SessionStore, basename, type AgentEventPayload, type Session } from "./state";
import { agentIconUrl } from "./icons";
import { elapsedString } from "./bubble";
import { t } from "./i18n";
import * as care from "./care";
import * as sync from "./sync";
import { savedSlug, petDisplayName, getLibrary } from "./catalog";

const store = new SessionStore();
const list = document.getElementById("pop-list")!;
const empty = document.getElementById("pop-empty")!;
const sub = document.getElementById("pop-sub")!;
const clearBtn = document.getElementById("pop-clear") as HTMLButtonElement;

function esc(s: string): string {
  return s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c] || c));
}

/// Compact tamagotchi HUD (same engine + copy as the Settings Care tab), so the
/// pet's level/progress is visible without opening Settings.
function fmtNum(n: number): string {
  n = Number(n) || 0;
  if (n >= 1e9) return (n / 1e9).toFixed(1).replace(/\.0$/, "") + "B";
  if (n >= 1e6) return (n / 1e6).toFixed(1).replace(/\.0$/, "") + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(1).replace(/\.0$/, "") + "K";
  return String(n);
}

function petName(slug: string): string {
  const custom = petDisplayName(slug);
  if (custom !== slug) return custom;
  return getLibrary().find((p) => p.slug === slug)?.name || slug;
}

function renderCare() {
  const slug = savedSlug();
  if (!slug) return;
  const s = care.stateFor(slug);
  const internal = care.levelForXP(s.xp);
  const setTxt = (id: string, v: string) => { const el = document.getElementById(id); if (el) el.textContent = v; };
  setTxt("pop-care-name", petName(slug));
  setTxt("pop-care-level", `${t("Lv")} ${care.displayLevel(s.xp)}`);
  setTxt("pop-care-stage", t(care.stageName(internal)));
  setTxt("pop-care-hunger", t(care.hunger(s)));
  // Show that this pet is tied to a linked GitHub profile (levels restore from it).
  const syncBadge = document.getElementById("pop-care-sync");
  if (syncBadge) {
    const linked = sync.signedIn();
    syncBadge.hidden = !linked;
    if (linked) syncBadge.title = t("Connected to your profile");
  }
  const fill = document.getElementById("pop-care-xpfill");
  if (fill) fill.style.width = `${Math.round(care.levelProgress(s.xp) * 100)}%`;
  setTxt("pop-care-xp", `${s.xp} XP`);
  const toNext = care.tokensToNextLevel(s);
  setTxt("pop-care-tonext", toNext > 0 ? `≈ ${fmtNum(toNext)} ${t("tokens to next level")}` : "");
  setTxt("pop-care-today", fmtNum(s.tokensToday));
  setTxt("pop-care-streak", String(s.streakDays));
  setTxt("pop-care-lifetime", fmtNum(s.totalTokens));
  setTxt("pop-care-sessions", String(s.totalMeals));
  // Details: achievements + 7-day burn (kept collapsed to keep the popover short).
  const unlocked = new Set(s.unlockedAchievements || []);
  setTxt("pop-care-achcount", `${unlocked.size} / ${care.ACHIEVEMENTS.length}`);
  const badges = document.getElementById("pop-care-badges");
  if (badges) badges.innerHTML = care.ACHIEVEMENTS
    .map((a) => `<span class="care-badge${unlocked.has(a) ? " on" : ""}" title="${t(care.ACH_NAME[a])}">${care.ACH_ICON[a]}</span>`)
    .join("");
  const days = care.recentDays(s, 7);
  const max = Math.max(1, ...days.map((d) => d.tokens));
  setTxt("pop-care-burntotal", fmtNum(days.reduce((a, d) => a + d.tokens, 0)));
  const chart = document.getElementById("pop-care-chart");
  if (chart) chart.innerHTML = days
    .map((d) => `<div class="cbar-wrap" title="${fmtNum(d.tokens)}"><div class="cbar" style="height:${Math.max(3, Math.round((d.tokens / max) * 100))}%"></div><div class="cbar-lbl">${d.label}</div></div>`)
    .join("");
}

// Details toggle (achievements + burn chart), remembered across opens.
const careMore = document.getElementById("pop-care-more") as HTMLButtonElement;
const careDetails = document.getElementById("pop-care-details") as HTMLElement;
function syncCareMore() {
  const open = localStorage.getItem("ap_pop_care_details") === "1";
  careDetails.hidden = !open;
  careMore.textContent = `${t("Details")} ${open ? "▴" : "▾"}`;
}
careMore.onclick = () => {
  localStorage.setItem("ap_pop_care_details", careDetails.hidden ? "1" : "0");
  syncCareMore();
  fitWindow();
};

function applyStatic() {
  const set = (id: string, key: string) => { const el = document.getElementById(id); if (el) el.textContent = t(key); };
  set("t-pop-care-today", "Today");
  set("t-pop-care-streak", "Streak");
  set("t-pop-care-lifetime", "Lifetime");
  set("t-pop-care-sessions", "Sessions");
  set("t-pop-care-ach", "Achievements");
  set("t-pop-care-burn", "Burn, last 7 days");
  syncCareMore();
  set("t-pop-needs", "NEEDS YOU");
  set("t-pop-agents", "AGENTS");
  set("pop-clear", "Clear all");
  set("pop-empty", "Nothing running right now.");
  set("t-pop-showpet", "Show pet");
  set("t-pop-size", "Pet size");
  set("t-pop-settings", "Settings");
  set("t-pop-updates", "Updates");
  set("t-pop-quit", "Quit");
}

/// Like the macOS popover: working/waiting/done sessions, idle + registered hidden.
function visible(): Session[] {
  return store.active().filter((s) => s.state !== "idle" && s.state !== "registered");
}

function paint() {
  const sessions = visible();
  const running = sessions.filter((s) => s.state === "working").length;
  if (!sessions.length) {
    sub.textContent = t("No agents running");
  } else {
    const label = `${sessions.length} ${sessions.length === 1 ? t("agent") : t("agents")}`;
    sub.textContent = running > 0 ? `${label} · ${running} ${t("running")}` : label;
  }
  empty.style.display = sessions.length ? "none" : "";
  clearBtn.style.display = sessions.length ? "" : "none";

  list.innerHTML = "";
  for (const s of sessions) {
    const row = document.createElement("div");
    row.className = "pop-agent";
    row.dataset.state = s.state;
    const icon = agentIconUrl(s.agent);
    const stale = (s.state === "working" || s.state === "waiting") && Date.now() - s.updatedAt > 180_000;
    row.innerHTML =
      `<span class="sess-dot"></span>` +
      `<span class="pop-ameta"><b>${esc(s.project ? basename(s.project) : s.session)}</b>` +
      `<span class="cap">${esc([s.role, s.model, s.cost > 0 ? `$${s.cost.toFixed(2)}` : "", s.title || s.live || t(cap(s.state)), stale ? `${elapsedString(s.updatedAt)} ${t("no update")}` : ""].filter(Boolean).join(" · "))}</span></span>` +
      (icon ? `<img class="dp-icon" src="${icon}" alt="">` : "") +
      `<span class="sess-time">${timeString(s)}</span>`;
    const x = document.createElement("button");
    x.className = "sess-x";
    x.textContent = "✕";
    x.onclick = (ev) => {
      ev.stopPropagation();
      const key = `${s.agent}:${s.session}`;
      seenDone.add(key);
      store.removeKey(key);
      emit("session-dismiss", key);
      paint();
    };
    row.appendChild(x);
    // Clicking the row (not the ✕) opens the session in OpenChamber, then hides
    // the popover so the user lands straight on it.
    if (s.agent === "opencode" && s.session.startsWith("opencode:")) {
      row.classList.add("openable");
      row.onclick = () => {
        seenDone.add(`${s.agent}:${s.session}`);
        openSession(s);
      };
    }
    list.appendChild(row);
  }
}

function cap(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/// mac AgentRow.timeString: live elapsed while active, clock time once done.
function timeString(s: Session): string {
  if (s.state === "done") {
    return new Date(s.updatedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return elapsedString(s.stateSince);
}

// ---- needs you ---------------------------------------------------------------
// A priority section pinned above the agent list: what actually waits on the
// user (an approval gate, a question, or a turn that just finished). Working
// sessions never mask these here, unlike the single pet mood.
type NeedKind = "approval" | "reply" | "done";
interface Need { s: Session; kind: NeedKind }
const seenDone = new Set<string>();

function needOf(s: Session, now: number): NeedKind | null {
  if (s.pendingApproval) return "approval";
  if (s.state === "waiting") return "reply";
  if (s.state === "done" && now - s.updatedAt < 120_000 && !seenDone.has(`${s.agent}:${s.session}`)) return "done";
  return null;
}

function needs(): Need[] {
  const now = Date.now();
  const order: Record<NeedKind, number> = { approval: 0, reply: 1, done: 2 };
  return visible()
    .map((s) => ({ s, kind: needOf(s, now) }))
    .filter((x): x is Need => x.kind !== null)
    .sort((a, b) => order[a.kind] - order[b.kind] || b.s.updatedAt - a.s.updatedAt);
}

function needLabel(kind: NeedKind): string {
  return kind === "approval" ? t("Needs approval") : kind === "reply" ? t("Needs reply") : t("Just finished");
}

function openSession(s: Session) {
  if (s.agent === "opencode" && s.session.startsWith("opencode:")) {
    void invoke("open_session", { sessionId: s.session });
    void getCurrentWindow().hide();
  }
}

function renderNeeds() {
  const wrap = document.getElementById("pop-needs-wrap") as HTMLElement | null;
  const listEl = document.getElementById("pop-needs");
  const countEl = document.getElementById("pop-needs-count");
  if (!wrap || !listEl || !countEl) return;
  const items = needs();
  wrap.hidden = items.length === 0;
  countEl.textContent = items.length ? String(items.length) : "";
  listEl.innerHTML = "";
  for (const { s, kind } of items) {
    const row = document.createElement("div");
    row.className = "pop-need";
    row.dataset.kind = kind;
    const icon = agentIconUrl(s.agent);
    row.innerHTML =
      `<span class="pop-need-badge">${esc(needLabel(kind))}</span>` +
      `<span class="pop-ameta"><b>${esc(s.project ? basename(s.project) : s.session)}</b>` +
      `<span class="cap">${esc([s.role, s.model, kind === "approval" ? s.pendingApproval?.tool : "", s.title || s.live].filter(Boolean).join(" · "))}</span></span>` +
      (icon ? `<img class="dp-icon" src="${icon}" alt="">` : "") +
      `<span class="sess-time">${timeString(s)}</span>`;
    row.onclick = () => {
      seenDone.add(`${s.agent}:${s.session}`);
      openSession(s);
      paintAndFit();
    };
    listEl.appendChild(row);
  }
}

// ---- controls ----------------------------------------------------------------

(document.getElementById("pop-clear") as HTMLButtonElement).onclick = () => {
  store.clear();
  emit("sessions-clear", null);
  paint();
};

const showPet = document.getElementById("pop-showpet") as HTMLInputElement;
invoke<boolean>("get_pet_visible").then((v) => { showPet.checked = v; }).catch(() => { showPet.checked = true; });
showPet.onchange = () => invoke("set_pet_visible", { visible: showPet.checked }).catch(() => {});

const size = document.getElementById("pop-size") as HTMLInputElement;
size.value = localStorage.getItem("ap_pet_size") || "100";
size.oninput = () => {
  localStorage.setItem("ap_pet_size", size.value);
  emit("bubble-changed", null);
};

(document.getElementById("pop-settings") as HTMLButtonElement).onclick = () => {
  invoke("open_settings").catch(() => {});
  void getCurrentWindow().hide();
};
(document.getElementById("pop-quit") as HTMLButtonElement).onclick = () => { exit(0); };

const updatesBtn = document.getElementById("pop-updates") as HTMLButtonElement;
updatesBtn.onclick = async () => {
  const label = document.getElementById("t-pop-updates")!;
  label.textContent = t("Checking…");
  try {
    const update = await check();
    if (update) {
      label.textContent = t("Installing…");
      await update.downloadAndInstall();
      await relaunch();
    } else {
      label.textContent = t("Up to date");
      setTimeout(() => { label.textContent = t("Updates"); }, 2500);
    }
  } catch {
    label.textContent = t("Up to date");
    setTimeout(() => { label.textContent = t("Updates"); }, 2500);
  }
};

// ---- lifecycle ----------------------------------------------------------------

// Hide when clicking anywhere outside (the popover loses focus), like the
// macOS transient popover. Backed up by a Rust-side Focused(false) handler,
// a "popover-close" broadcast from the pet window, and the Escape key.
getCurrentWindow().onFocusChanged(({ payload: focused }) => {
  if (!focused) void getCurrentWindow().hide();
});
listen("popover-close", () => void getCurrentWindow().hide());
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") void getCurrentWindow().hide();
});

listen<AgentEventPayload>("agent-event", (e) => { store.update(e.payload); paintAndFit(); });
listen<string>("agent-end", (e) => { store.remove(e.payload); paintAndFit(); });
// The approval gate is broadcast to every window; mirror it here so the
// "Needs approval" bucket works in the popover too.
listen<{ id: string; session: string; tool: string; summary: string }>("agent-approval", (e) => {
  store.setApproval(e.payload.session, { id: e.payload.id, tool: e.payload.tool, summary: e.payload.summary });
  paintAndFit();
});
listen<{ id: string; session: string }>("agent-approval-resolved", (e) => {
  store.clearApproval(e.payload.session);
  paintAndFit();
});
listen<Session>("session-snapshot", (e) => { store.seed(e.payload); paintAndFit(); });
// Re-sync + refresh whenever the popover is shown again.
listen("popover-shown", () => {
  size.value = localStorage.getItem("ap_pet_size") || "100";
  invoke<boolean>("get_pet_visible").then((v) => { showPet.checked = v; }).catch(() => {});
  emit("sessions-request", null);
  // Refresh the level from the linked account so the HUD shows the cloud value.
  void sync.autoRestore().then(() => paintAndFit());
  paintAndFit();
});
emit("sessions-request", null);

// Hug the content height like the macOS popover (no dead space).
let lastH = 0;
function fitWindow() {
  const card = document.querySelector(".pop-card") as HTMLElement;
  if (!card) return;
  const h = Math.min(520, Math.max(220, card.scrollHeight + 20));
  if (Math.abs(h - lastH) < 2) return;
  lastH = h;
  getCurrentWindow().setSize(new LogicalSize(300, h)).catch(() => {});
}

const origPaint = paint;
function paintAndFit() { origPaint(); renderCare(); renderNeeds(); fitWindow(); }

listen("care-updated", () => paintAndFit());
setInterval(paintAndFit, 1000); // live elapsed + prune
applyStatic();
paintAndFit();

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
  const fill = document.getElementById("pop-care-xpfill");
  if (fill) fill.style.width = `${Math.round(care.levelProgress(s.xp) * 100)}%`;
  setTxt("pop-care-xp", `${s.xp} XP`);
  const toNext = care.tokensToNextLevel(s);
  setTxt("pop-care-tonext", toNext > 0 ? `≈ ${fmtNum(toNext)} ${t("tokens to next level")}` : "");
  setTxt("pop-care-today", fmtNum(s.tokensToday));
  setTxt("pop-care-today-sub", `${s.mealsToday} ${t("sessions")}`);
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
  set("pop-care-streak-sub", "days fed");
  set("pop-care-lifetime-sub", "tokens eaten");
  set("pop-care-sessions-sub", "completed");
  set("t-pop-care-ach", "Achievements");
  set("t-pop-care-burn", "Burn, last 7 days");
  syncCareMore();
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
    row.innerHTML =
      `<span class="sess-dot"></span>` +
      `<span class="pop-ameta"><b>${esc(s.project ? basename(s.project) : s.session)}</b>` +
      `<span class="cap">${esc([s.role, s.cost > 0 ? `$${s.cost.toFixed(2)}` : "", s.title || s.live || t(cap(s.state))].filter(Boolean).join(" · "))}</span></span>` +
      (icon ? `<img class="dp-icon" src="${icon}" alt="">` : "") +
      `<span class="sess-time">${timeString(s)}</span>`;
    const x = document.createElement("button");
    x.className = "sess-x";
    x.textContent = "✕";
    x.onclick = (ev) => {
      ev.stopPropagation();
      const key = `${s.agent}:${s.session}`;
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
        void invoke("open_session", { sessionId: s.session });
        void getCurrentWindow().hide();
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
listen<Session>("session-snapshot", (e) => { store.seed(e.payload); paintAndFit(); });
// Re-sync + refresh whenever the popover is shown again.
listen("popover-shown", () => {
  size.value = localStorage.getItem("ap_pet_size") || "100";
  invoke<boolean>("get_pet_visible").then((v) => { showPet.checked = v; }).catch(() => {});
  emit("sessions-request", null);
  paintAndFit();
});
emit("sessions-request", null);

// Hug the content height like the macOS popover (no dead space).
let lastH = 0;
function fitWindow() {
  const card = document.querySelector(".pop-card") as HTMLElement;
  if (!card) return;
  const h = Math.min(560, Math.max(220, card.scrollHeight + 20));
  if (Math.abs(h - lastH) < 2) return;
  lastH = h;
  getCurrentWindow().setSize(new LogicalSize(300, h)).catch(() => {});
}

const origPaint = paint;
function paintAndFit() { origPaint(); renderCare(); fitWindow(); }

listen("care-updated", () => paintAndFit());
setInterval(paintAndFit, 1000); // live elapsed + prune
applyStatic();
paintAndFit();

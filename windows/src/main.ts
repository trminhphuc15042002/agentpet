import { listen, emit } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Pet } from "./pet";
import { SessionStore, aggregateMood, basename, type AgentEventPayload, type SubagentEventPayload } from "./state";
import { BubbleRenderer, groupSessions, readBubbleConfig } from "./bubble";
import { bubbleLayout, maintenanceSig, shouldSendResize } from "./geometry";
import { loadCatalog, savedSlug, saveSlug } from "./catalog";
import { t, setLang, type Lang } from "./i18n";
import { bubbleLines, PET_CHAT } from "./activity";
import * as care from "./care";
import * as sync from "./sync";
import * as usage from "./usage";
import * as history from "./history";
import * as reactive from "./reactive";
import * as projectpets from "./projectpets";
import * as audio from "./audio";
import { focusActive, WaitingReminders, sessionKey, readLastEvents } from "./companion";
import { pettingLine } from "./personality";

// Which project THIS pet window represents. `null` = the main window (the
// default single pet). Split-pet spawns extra windows with `?project=<id>`.
const MY_PROJECT = new URLSearchParams(location.search).get("project");
const IS_MAIN = MY_PROJECT === null;

// A project window sets this the moment its project is un-split, so it stops
// feeding during the brief async gap before Rust closes it (else the main window
// , which now owns the project , and this dying window would both feed one event).
let windowDead = false;

/// Does this window own a session (feed its pet, count it, notify)? Split off:
/// the main window owns everything. Split on: a project window owns only its
/// project; the main window owns every unconfigured project.
function ownsProject(path: string): boolean {
  if (windowDead) return false;
  if (!projectpets.splitEnabled()) return IS_MAIN;
  const id = usage.projectId(path || "");
  if (MY_PROJECT) return id === MY_PROJECT;
  return !projectpets.configuredProjectIds().includes(id);
}

/// The pet slug this window raises (a project window uses its mapped pet).
function myPetSlug(): string | null {
  if (MY_PROJECT && projectpets.splitEnabled()) return projectpets.petForProject(MY_PROJECT) || savedSlug();
  return savedSlug();
}

/// Reconcile the per-project pet windows with the current config (main only).
function syncProjectWindows() {
  const ids = projectpets.splitEnabled() ? projectpets.configuredProjectIds() : [];
  void invoke("sync_project_windows", { projects: ids });
}
import { sendNotification, isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

// Auto-update on launch (no-op offline / when no signed release is published).
// Main window only , the per-project windows share the same binary.
if (IS_MAIN) (async () => {
  try {
    const update = await check();
    if (update) {
      await update.downloadAndInstall();
      await relaunch();
    }
  } catch {}
})();

const canvas = document.getElementById("pet") as HTMLCanvasElement;
const bubbleEl = document.getElementById("bubble") as HTMLDivElement;
const partyBadge = document.getElementById("party-badge") as HTMLDivElement;
const pet = new Pet(canvas);
const store = new SessionStore();
const bubble = new BubbleRenderer(bubbleEl);
const lastEvents = readLastEvents(localStorage.getItem("ap_last_events"));

// --- bubble appearance (theme / opacity / fonts) ------------------------------
const FONT_FAMILIES: Record<string, string> = {
  system: '"Segoe UI", system-ui, sans-serif',
  rounded: '"Segoe UI Rounded", "Nunito", "Segoe UI", sans-serif',
  mono: 'Consolas, "Courier New", monospace',
};

function applyBubble() {
  let theme = localStorage.getItem("ap_theme") || "dark";
  if (theme === "system") theme = matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  const op = (parseInt(localStorage.getItem("ap_opacity") || "92", 10) || 92) / 100;
  const r = document.documentElement.style;
  if (theme === "light") {
    r.setProperty("--bubble-bg", `rgba(255,255,255,${op})`);
    r.setProperty("--bubble-fg", "#1a1d2e");
    r.setProperty("--bubble-border", "rgba(0,0,0,0.08)");
  } else {
    r.setProperty("--bubble-bg", `rgba(22,24,38,${op})`);
    r.setProperty("--bubble-fg", "#ffffff");
    r.setProperty("--bubble-border", "rgba(255,255,255,0.10)");
  }
  r.setProperty("--bubble-font-size", `${parseInt(localStorage.getItem("ap_font_size") || "12", 10) || 12}px`);
  r.setProperty("--bubble-font-family", FONT_FAMILIES[localStorage.getItem("ap_font_family") || "system"] ?? FONT_FAMILIES.system);
}
applyBubble();

// Pet size + idle bob FX. Sized via layout (not transform) so the bubble
// always sits above the sprite instead of being painted over by it.
function applyPet() {
  const size = (parseInt(localStorage.getItem("ap_pet_size") || "100", 10) || 100) / 100;
  canvas.style.width = `${Math.round(160 * size)}px`;
  canvas.style.height = `${Math.round(180 * size)}px`;
  canvas.classList.toggle("bob", localStorage.getItem("ap_fx") === "1");
}
applyPet();

// Chimes + per-event enable live in ./audio (shared with Settings and the demo
// panel). Unlock the AudioContext on the first gesture so event-driven chimes
// are not muted by the WebView autoplay policy.
audio.bindAudioUnlock();

// --- pick + load a pet sprite -------------------------------------------------
(async () => {
  // A project window raises its mapped pet; the main window the selected one.
  if (MY_PROJECT) {
    const slug = projectpets.petForProject(MY_PROJECT);
    const url = slug ? projectpets.libUrlForSlug(slug) : null;
    if (url) { pet.load(url); return; }
  }
  // Library selection (Browse/Create) wins; legacy ap_pet_custom still honoured.
  const url = localStorage.getItem("ap_pet_custom") || localStorage.getItem("ap_pet_url");
  if (url) { pet.load(url); return; }
  // First run: no selection yet , pick a starter from the catalog.
  for (;;) {
    const pets = await loadCatalog();
    if (pets.length) {
      const slug = savedSlug();
      const chosen = pets.find((p) => p.slug === slug) ?? pets[Math.floor(pets.length / 2)];
      saveSlug(chosen.slug);
      localStorage.setItem("ap_pet_url", chosen.spritesheetUrl);
      pet.load(chosen.spritesheetUrl);
      return;
    }
    await new Promise((r) => setTimeout(r, 15000));
  }
})();

// --- mood + render loop --------------------------------------------------------
// Port of PetController: aggregate mood, 3s celebrate burst on entering done,
// a persistent idle line (re-picked on mood transitions, not blinking), and
// the structured multi-agent bubble while agents are active.
let lastResolved = "idle";
let celebrateUntil = 0;
let wasCelebrating = false;
let prevSimpleMood = "";
let moodLine = ""; // the single-bubble line for idle/done/celebrate
let reactiveLine = "";
let reactiveUntil = 0;
let pinnedKey = "";
let pettingUntil = 0;
let pettingText = "";
let previewUntil = 0;
const isFocused = () => focusActive(Number(localStorage.getItem("ap_focus_until")));

function publishPin() { void emit("session-pin-state", { project: MY_PROJECT, key: windowDead ? "" : pinnedKey }); }

/// Show a reactive comment for a few seconds (mac PetController.flashReactiveLine).
function flashReactive(line: string | null) {
  if (!line || isFocused()) return;
  reactiveLine = line;
  reactiveUntil = Date.now() + 5000;
  render();
}

/// Evaluate the care-driven reactive metrics after a feed / meal / hunger tick.
function evaluateCareMetrics() {
  const slug = myPetSlug();
  if (!slug) return;
  const s = care.stateFor(slug);
  flashReactive(reactive.evaluate("dailyTokens", s.tokensToday));
  flashReactive(reactive.evaluate("streak", s.streakDays));
  flashReactive(reactive.evaluate("dailyMeals", s.mealsToday));
  flashReactive(reactive.evaluate("hunger", care.hunger(s)));
}

function pickMoodLine(mood: string) {
  // Custom/system pools; working/waiting fall back to the PetChat lines so the
  // simple-bubble mode (multi-agent off) always has something to say.
  let pool = bubbleLines(null, mood);
  if (!pool.length) pool = PET_CHAT[mood] ?? [];
  moodLine = pool.length ? pool[Math.floor(Math.random() * pool.length)] : "";
}

function render() {
  const sessions = store.active().filter((s) => ownsProject(s.project));
  if (pinnedKey && !sessions.some((s) => sessionKey(s) === pinnedKey && s.state !== "done")) {
    pinnedKey = "";
    publishPin();
  }
  const focused = isFocused();
  const subagentCount = sessions.reduce((count, session) => count + session.subagents.length, 0);
  const subagentRoles = sessions.flatMap((session) => session.subagents.map((child) => child.role.trim() || "Subagent"));
  const waitingCount = sessions.filter((s) => s.state === "waiting" || s.pendingApproval).length;
  partyBadge.hidden = subagentCount === 0 && waitingCount === 0 && !focused;
  partyBadge.textContent = [focused ? "☕" : "", subagentCount ? `👥 ${subagentCount}` : "", waitingCount ? `⏳ ${waitingCount}` : ""].filter(Boolean).join(" · ");
  partyBadge.title = [focused ? t("Focusing") : "", waitingCount ? `${waitingCount} · ${t("Waiting for you")}` : "", ...subagentRoles].filter(Boolean).join("\n");
  const resolved = aggregateMood(sessions);

  if (!focused && resolved === "done" && lastResolved !== "done") {
    celebrateUntil = Date.now() + 3000; // celebrate burst, like macOS
    pickMoodLine("celebrate");
  }
  if (resolved !== lastResolved && Date.now() >= celebrateUntil) {
    if (resolved === "idle") pickMoodLine("idle");
    else if (resolved === "done") pickMoodLine("done");
  }
  lastResolved = resolved;

  const celebrating = !focused && Date.now() < celebrateUntil;
  if (wasCelebrating && !celebrating) {
    // The 3s burst ended , settle into the actual mood's line (mac
    // settleAfterCelebrate re-picks on the celebrate→done transition).
    pickMoodLine(resolved === "idle" ? "idle" : "done");
  }
  wasCelebrating = celebrating;
  const mood = celebrating ? "celebrate" : resolved;
  const needsInput = sessions.some((s) => s.state === "waiting" || s.pendingApproval);
  if (needsInput) pettingUntil = 0;
  const petting = Date.now() < pettingUntil && !focused;
  pet.setState(petting ? "celebrate" : mood);

  // A reactive comment briefly overrides the quiet single-line moods (not the
  // multi-agent working bubble, not the celebrate burst).
  const reactiveActive = !focused && Date.now() < reactiveUntil && !!reactiveLine;

  const multi = localStorage.getItem("ap_multi") !== "0";
  const activeRows = sessions.filter((s) => s.state !== "idle" && s.state !== "registered");
  const visibleGroups = multi ? groupSessions(activeRows, readBubbleConfig()) : [];
  if (petting) {
    bubble.renderLine(pettingText);
  } else if (focused && !needsInput) {
    bubble.renderLine(t("Focusing"));
  } else if (mood === "working" || mood === "waiting") {
    if (multi && visibleGroups.length) {
      bubble.render(activeRows, pinnedKey);
    } else {
      // Simple-bubble / personality fallback when multi is off OR filters hide every group.
      if (resolved !== prevSimpleMood) { pickMoodLine(mood); prevSimpleMood = resolved; }
      if (!moodLine) pickMoodLine(mood);
      bubble.renderLine(reactiveActive ? reactiveLine : moodLine);
    }
  } else if (mood === "celebrate") {
    bubble.renderLine(moodLine || t("Done"));
  } else if (mood === "done") {
    if (!moodLine) pickMoodLine("done");
    bubble.renderLine(reactiveActive ? reactiveLine : moodLine);
  } else {
    // idle: a persistent quiet line (mac shows it continuously, no blinking)
    if (reactiveActive) {
      bubble.renderLine(reactiveLine);
    } else if (Date.now() < previewUntil) {
      bubble.renderLine(t("Display test received"));
    } else if (localStorage.getItem("ap_idle") !== "0") {
      if (!moodLine) pickMoodLine("idle");
      bubble.renderLine(moodLine);
    } else {
      bubble.hide();
    }
  }

  snugBubble();
  applyBubbleGeometry();
  hugOverlay();
  // One global tray icon , the main window reports it, counting ALL sessions
  // (not just this window's owned subset).
  if (IS_MAIN) reportTrayStatus(store.active());
}

let maintTimer: number | null = null;
let clockTimer: number | null = null;
let dirtyTimer: number | null = null;
let hungerTimer: number | null = null;
let lastFocus = isFocused();

function maintain() {
  const focused = isFocused();
  if (focused !== lastFocus) { lastFocus = focused; moodLine = ""; render(); }
  if (pettingUntil && Date.now() >= pettingUntil) { pettingUntil = 0; render(); }
  if (previewUntil && Date.now() >= previewUntil) { previewUntil = 0; render(); }
  const before = maintenanceSig(store.snapshot());
  store.active();
  const now = Date.now();
  const celebrating = now < celebrateUntil;
  if (wasCelebrating && !celebrating) { render(); return; }
  if (reactiveLine && now >= reactiveUntil) { reactiveLine = ""; render(); return; }
  if (maintenanceSig(store.snapshot()) !== before) render();
}

function startUiTimers() {
  if (document.hidden) return;
  if (!maintTimer) maintTimer = window.setInterval(maintain, 500);
  if (!clockTimer) clockTimer = window.setInterval(() => bubble.tickClocks(), 1000);
  if (!dirtyTimer) dirtyTimer = window.setInterval(() => {
    if (bubble.dirty) { bubble.dirty = false; render(); }
  }, 120);
  if (!hungerTimer) hungerTimer = window.setInterval(() => {
    const slug = myPetSlug();
    if (slug) flashReactive(reactive.evaluate("hunger", care.hunger(care.stateFor(slug))));
  }, 60_000);
  bubble.resume();
}

function stopUiTimers() {
  if (maintTimer) { clearInterval(maintTimer); maintTimer = null; }
  if (clockTimer) { clearInterval(clockTimer); clockTimer = null; }
  if (dirtyTimer) { clearInterval(dirtyTimer); dirtyTimer = null; }
  if (hungerTimer) { clearInterval(hungerTimer); hungerTimer = null; }
  bubble.pause();
}

document.addEventListener("visibilitychange", () => {
  if (document.hidden) stopUiTimers();
  else { startUiTimers(); render(); }
});
startUiTimers();

// Pull the bubble down over the canvas's empty headroom so it sits right
// above the pet's head (the sprite rarely fills the whole canvas height).
function snugBubble() {
  const gap = Math.max(0, canvas.clientHeight * pet.headroom - 4);
  petRoot.style.setProperty("--bubble-snug", `${gap}px`);
}

let windowWidthLogical = 260;
let petOffsetLogical = 0;

function applyBubbleGeometry() {
  const bw = bubbleEl.hidden ? 0 : bubbleEl.offsetWidth;
  const layout = bubbleLayout(petOffsetLogical, windowWidthLogical, bw);
  petRoot.style.setProperty("--pet-offset", `${petOffsetLogical}px`);
  petRoot.style.setProperty("--bubble-shift", `${layout.bubbleShift}px`);
  petRoot.style.setProperty("--tail-shift", `${layout.tailShift}px`);
}

listen<{ petOffset: number; windowWidth: number }>("pet-geometry", (e) => {
  if (typeof e.payload?.petOffset === "number") petOffsetLogical = e.payload.petOffset;
  if (typeof e.payload?.windowWidth === "number" && e.payload.windowWidth > 0) {
    windowWidthLogical = e.payload.windowWidth;
  }
  applyBubbleGeometry();
  reportHitRect();
});

// Tray tooltip mirrors the macOS menu bar count (N working / N waiting).
let lastTray = "";
let lastTraySessions = "";
function reportTrayStatus(sessions: ReturnType<SessionStore["active"]>) {
  const working = sessions.filter((s) => s.state === "working").length;
  const waiting = sessions.filter((s) => s.state === "waiting").length;
  const sig = `${working}/${waiting}`;
  if (sig !== lastTray) {
    lastTray = sig;
    invoke("set_tray_status", { working, waiting }).catch(() => {});
  }
  // Tray "Sessions" submenu: only OpenCode sessions can be opened in OpenChamber.
  const rows = sessions
    .filter((s) => s.agent === "opencode" && s.session.startsWith("opencode:"))
    .slice(0, 12)
    .map((s) => ({
      session: s.session,
      label:
        `${s.project ? s.project.split(/[\\/]/).filter(Boolean).pop() : s.session}` +
        `${s.role ? ` · ${s.role}` : ""} · ${s.state}`,
    }));
  const rsig = rows.map((r) => `${r.session}|${r.label}`).join("\n");
  if (rsig !== lastTraySessions) {
    lastTraySessions = rsig;
    invoke("set_tray_sessions", { sessions: rows }).catch(() => {});
  }
}

// --- notifications ------------------------------------------------------------
let notifyReady = false;
(async () => {
  try { notifyReady = (await isPermissionGranted()) || (await requestPermission()) === "granted"; } catch {}
})();
const lastState = new Map<string, string>();
const sessionStarts = new Map<string, number>();
function maybeNotify(e: AgentEventPayload) {
  const key = `${e.agent}:${e.session}`;
  const prev = lastState.get(key);
  lastState.set(key, e.state);
  if (!sessionStarts.has(key) && (e.state === "working" || e.state === "registered")) {
    sessionStarts.set(key, Date.now());
  }
  if (e.state === prev) return;
  // A finished session is a "meal" for THIS window's pet , only the window that
  // owns the project records it, so split pets never double-feed.
  if (e.state === "done" && ownsProject(e.project)) {
    const slug = myPetSlug();
    if (slug) { care.mutate(slug, (s) => care.recordMeal(s)); emit("care-updated"); sync.schedulePush(); evaluateCareMetrics(); }
    if (e.project) usage.recordSession(e.project, e.agent);
    const now = Date.now();
    history.log({
      id: e.session, agent: e.agent, project: e.project ? basename(e.project) : "",
      title: e.title || "", startedAt: sessionStarts.get(key) ?? now, endedAt: now,
    });
  }
  // Chimes + notifications fire once , the main window only.
  if (!IS_MAIN) return;
  if (e.state !== "done" && e.state !== "waiting") return;
  if (e.state === "done" && isFocused()) return;
  void audio.playSound(e.state === "done" ? "done" : "waiting");
  if (!notifyReady || localStorage.getItem("ap_notify") === "0") return;
  const proj = (e.project ? basename(e.project) : "") || e.agent;
  // Same copy as the macOS notifications.
  const title = e.state === "done" ? `${proj} ${t("finished")}` : `${proj} ${t("needs input")}`;
  const body = e.state === "done"
    ? t("Agent completed its turn")
    : (e.message || t("Waiting for you"));
  try { sendNotification({ title, body }); } catch {}
}

const reminders = new WaitingReminders();
let reminderTimer: number | null = null;
function armReminder() {
  if (!IS_MAIN) return;
  if (reminderTimer) clearTimeout(reminderTimer);
  reminderTimer = null;
  if (localStorage.getItem("ap_wait_reminder") !== "1") return;
  const delay = reminders.nextDelay(store.snapshot(), Date.now());
  if (delay === null) return;
  reminderTimer = window.setTimeout(() => {
    reminderTimer = null;
    const due = reminders.takeDue(store.snapshot(), Date.now());
    if (due.length && !isFocused()) {
      if (notifyReady && localStorage.getItem("ap_notify") !== "0") {
        const body = due.map((s) => basename(s.project) || s.agent).join(", ");
        try { sendNotification({ title: `${due.length} · ${t("Still waiting for you")}`, body }); } catch {}
      }
      void audio.playSound("waiting");
    }
    armReminder();
  }, delay);
}

// --- agent events from the Rust listener -------------------------------------
listen<AgentEventPayload>("agent-event", (e) => {
  if (IS_MAIN) {
    const ts = Math.min(e.payload.ts || Date.now(), Date.now());
    if (/^[a-z0-9_-]{1,40}$/i.test(e.payload.agent) && ts > 0 && ts - (lastEvents[e.payload.agent] || 0) >= 10_000) {
      // Bound persistence frequency for chatty hooks; no periodic flush timer.
      lastEvents[e.payload.agent] = ts;
      const bounded = Object.fromEntries(Object.entries(lastEvents).sort((a, b) => b[1] - a[1]).slice(0, 32));
      for (const key of Object.keys(lastEvents)) if (!(key in bounded)) delete lastEvents[key];
      localStorage.setItem("ap_last_events", JSON.stringify(bounded));
      void emit("diagnostics-updated", null);
    }
  }
  maybeNotify(e.payload);
  store.update(e.payload);
  armReminder();
  const owned = store.active().filter((s) => ownsProject(s.project)).length;
  flashReactive(reactive.evaluate("sessionCount", owned));
  render();
});
// Agent Party plumbing: lifecycle is a small event alongside the parent
// session, not another pet window or an independently-polled session.
listen<SubagentEventPayload>("agent-subagent", (e) => {
  store.updateSubagent(e.payload);
  render();
});
// Approval gate: the daemon parked a gated PreToolUse , show Allow/Deny.
listen<{ id: string; session: string; tool: string; summary: string }>("agent-approval", (e) => {
  const p = e.payload;
  store.setApproval(p.session, { id: p.id, tool: p.tool, summary: p.summary });
  render();
});
listen<{ id: string; session: string }>("agent-approval-resolved", (e) => {
  store.clearApproval(e.payload.session);
  render();
});
listen<string>("agent-end", (e) => {
  for (const k of [...lastState.keys()]) if (k.endsWith(`:${e.payload}`)) lastState.delete(k);
  for (const k of [...sessionStarts.keys()]) if (k.endsWith(`:${e.payload}`)) sessionStarts.delete(k);
  store.remove(e.payload);
  armReminder();
  render();
});
// Tokens burned by an agent feed THIS window's pet , only the owning window, so
// split pets never double-feed the same tokens.
listen<{ agent: string; session: string; project: string; tokens: number; cost?: number }>("agent-tokens", (e) => {
  const n = e.payload?.tokens || 0;
  if (n <= 0) return;
  const p = e.payload;
  if (!ownsProject(p.project)) return;
  if (p.project) usage.recordTokens(p.project, p.agent, n, p.cost || 0);
  const slug = myPetSlug();
  if (!slug) return;
  care.mutate(slug, (s) => care.feedTokens(s, n));
  emit("care-updated");
  sync.schedulePush();
  evaluateCareMetrics();
});

// On launch: pull any cloud progress, then keep pushing in the background. The
// main window owns cloud sync (care state is shared across windows).
if (IS_MAIN && sync.signedIn()) {
  sync.restore().then(() => { emit("care-updated"); sync.schedulePush(5000); }).catch(() => {});
  usage.schedulePush(8000);
}
// Split-pet: the main window spawns/closes the per-project pet windows, and
// re-syncs whenever Settings changes the config.
if (IS_MAIN) {
  syncProjectWindows();
  listen("split-changed", () => syncProjectWindows());
} else {
  // A project window: once split is off or its project is no longer configured,
  // it's about to be closed , stop owning events immediately to avoid a
  // double-feed with the main window during teardown.
  listen("split-changed", () => {
    if (!projectpets.splitEnabled() || (MY_PROJECT && !projectpets.configuredProjectIds().includes(MY_PROJECT))) {
      windowDead = true;
      publishPin();
    }
  });
}
// Settings window: dismiss one session / clear all (mac popover actions).
listen<string>("session-dismiss", (e) => { store.removeKey(e.payload); armReminder(); render(); });
listen("sessions-clear", () => { store.clear(); armReminder(); render(); });
// A freshly opened Settings window asks for the current sessions.
listen("sessions-request", () => {
  for (const s of store.snapshot()) emit("session-snapshot", s);
  publishPin();
});
listen<string>("session-pin", (e) => {
  const session = store.active().find((s) => sessionKey(s) === e.payload && ownsProject(s.project));
  if (!session || session.state === "done") return;
  pinnedKey = pinnedKey === e.payload ? "" : e.payload;
  publishPin();
  render();
});
listen("pet-stroke", () => {
  if (!IS_MAIN || isFocused() || Date.now() < pettingUntil) return;
  if (store.active().some((s) => s.state === "waiting" || s.pendingApproval)) return;
  pettingUntil = Date.now() + 2000;
  pettingText = pettingLine();
  render();
});
listen<string>("diagnostics-test", (e) => {
  if (!IS_MAIN) return;
  previewUntil = Date.now() + 3000;
  render();
  void emit("diagnostics-test-received", e.payload);
});
// Pet changed from the Settings window.
listen<{ slug: string; url: string }>("set-pet", (e) => {
  pet.load(e.payload.url);
  saveSlug(e.payload.slug);
  localStorage.setItem("ap_pet_url", e.payload.url);
});
// Language changed from Settings , re-render the bubble in the new language.
listen<Lang>("lang-changed", (e) => { setLang(e.payload); render(); });
// Bubble theme / opacity / messages changed from Settings.
listen("bubble-changed", () => { applyBubble(); applyPet(); moodLine = ""; armReminder(); render(); });

// Tray "Check for updates" (Rust emits this to the main pet window only). The
// popover has a labelled Updates button, but on Linux appindicator can't open
// the popover from a tray click, so the tray menu item is the reliable entry
// point. Feedback goes through notifications since the tray has no live label.
listen("check-updates", async () => {
  if (!IS_MAIN) return;
  const note = (body: string) => { try { if (notifyReady) sendNotification({ title: "AgentPet", body }); } catch {} };
  note(t("Checking…"));
  try {
    const update = await check();
    if (update) {
      note(t("Installing…"));
      await update.downloadAndInstall();
      await relaunch();
    } else {
      note(t("Up to date"));
    }
  } catch {
    note(t("Up to date"));
  }
});

// --- interactions ------------------------------------------------------------
// Drag works only when grabbing the PET SPRITE itself or the bubble , clicks
// on the transparent area beside the pet fall through (like the macOS panel,
// where transparent pixels never catch the mouse).
canvas.addEventListener("mousedown", async (e) => {
  if (e.button !== 0) return;
  // While the sheet is still loading there is no sprite rect yet , allow the
  // drag anyway so the pet is never untouchable.
  if (pet.spriteRect && !pet.hitTest(e.offsetX, e.offsetY)) return;
  emit("popover-close", null);
  await getCurrentWindow().startDragging();
});
bubbleEl.addEventListener("mousedown", async (e) => {
  if (e.button !== 0) return;
  emit("popover-close", null);
  await getCurrentWindow().startDragging();
});
canvas.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  if (pet.hitTest(e.offsetX, e.offsetY)) invoke("open_popover").catch(() => {});
});
bubbleEl.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  invoke("open_popover").catch(() => {});
});

// Report the interactive region (physical px) for Windows click-through: the
// union of the SPRITE's true bounds and the visible bubble , not the whole
// canvas, so the empty space beside the pet passes clicks to apps below.
const petRoot = document.getElementById("pet-root") as HTMLElement;
let lastHitSig = "";

const RESIZE_TOL = 1;
let resizeInflight = false;
let resizeQueued: { w: number; h: number } | null = null;
let shrinkTimer: number | null = null;

function paddedSize(w: number, h: number) {
  return { w: w + 4, h: h + 4 };
}

function viewportSize() {
  return { w: window.innerWidth, h: window.innerHeight };
}

async function sendResize(size: { w: number; h: number }) {
  resizeQueued = size;
  if (resizeInflight) return;
  resizeInflight = true;
  try {
    while (resizeQueued) {
      const next = resizeQueued;
      resizeQueued = null;
      const actual = viewportSize();
      if (!shouldSendResize(next, actual, RESIZE_TOL)) continue;
      try {
        const result = await invoke<{ petOffset: number; windowWidth: number }>("resize_pet_window", {
          width: next.w,
          height: next.h,
          petWidth: canvas.offsetWidth || 160,
        });
        petOffsetLogical = result.petOffset;
        windowWidthLogical = result.windowWidth;
        applyBubbleGeometry();
        reportHitRect();
      } catch (err) {
        invoke("log_debug", { msg: `resize_pet_window failed: ${err}` }).catch(() => {});
      }
    }
  } finally {
    resizeInflight = false;
    if (resizeQueued) void sendResize(resizeQueued);
  }
}

function onContentSize(w: number, h: number) {
  if (w <= 0 || h <= 0) return;
  const target = paddedSize(w, h);
  const actual = viewportSize();
  const grow = {
    // A native resize event can reach the WebView before invoke resolves.
    // A remembered larger size would undo that shrink and start a grow/shrink
    // feedback loop. Only current content and the live viewport matter here.
    w: Math.max(target.w, actual.w),
    h: Math.max(target.h, actual.h),
  };
  if (grow.w > actual.w + RESIZE_TOL || grow.h > actual.h + RESIZE_TOL) {
    void sendResize(grow);
  }
  if (shrinkTimer) clearTimeout(shrinkTimer);
  shrinkTimer = window.setTimeout(() => { void sendResize(target); }, 50);
}

function overlayUnion() {
  const rects: { left: number; top: number; right: number; bottom: number }[] = [];
  if (!bubbleEl.hidden) {
    const b = bubbleEl.getBoundingClientRect();
    if (b.width > 0) rects.push({ left: b.left, top: b.top, right: b.right, bottom: b.bottom });
  }
  if (!partyBadge.hidden) {
    const b = partyBadge.getBoundingClientRect();
    if (b.width > 0) rects.push({ left: b.left, top: b.top, right: b.right, bottom: b.bottom });
  }
  const cr = canvas.getBoundingClientRect();
  const sr = pet.spriteRect;
  if (sr && canvas.width > 0) {
    const kx = cr.width / canvas.width;
    const ky = cr.height / canvas.height;
    rects.push({
      left: cr.left + sr.x * kx,
      top: cr.top + sr.y * ky,
      right: cr.left + (sr.x + sr.w) * kx,
      bottom: cr.top + (sr.y + sr.h) * ky,
    });
  } else if (cr.width > 0) {
    rects.push({ left: cr.left, top: cr.top, right: cr.right, bottom: cr.bottom });
  }
  if (!rects.length) return null;
  const left = Math.min(...rects.map((r) => r.left));
  const top = Math.min(...rects.map((r) => r.top));
  const right = Math.max(...rects.map((r) => r.right));
  const bottom = Math.max(...rects.map((r) => r.bottom));
  return { left, top, right, bottom, w: right - left, h: bottom - top };
}

function reportHitRect() {
  const d = window.devicePixelRatio || 1;
  const u = overlayUnion();
  if (!u) return;
  const sig = `${d}:` + [u.left, u.top, u.right, u.bottom].map((v) => Math.round(v)).join(",");
  if (sig === lastHitSig) return;
  lastHitSig = sig;
  invoke("set_hit_rect", { x: u.left * d, y: u.top * d, w: u.w * d, h: u.h * d })
    .catch((err) => invoke("log_debug", { msg: `set_hit_rect failed: ${err}` }).catch(() => {}));
}

function hugOverlay() {
  const u = overlayUnion();
  if (u && u.w > 0 && u.h > 0) onContentSize(u.w, u.h);
  reportHitRect();
}

new ResizeObserver(() => { hugOverlay(); }).observe(petRoot);
window.addEventListener("resize", hugOverlay);
hugOverlay();

render();

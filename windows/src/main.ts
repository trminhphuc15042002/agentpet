import { listen, emit } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { Pet, HIT_PAD } from "./pet";
import { SessionStore, aggregateMood, basename, type AgentEventPayload, type SubagentEventPayload } from "./state";
import { BubbleRenderer, groupSessions, readBubbleConfig } from "./bubble";
import { maintenanceSig } from "./geometry";
import { loadCatalog, savedSlug, saveSlug, rememberPetUrl, selectedPetUrl } from "./catalog";
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
import * as limits from "./limits";
import { projectLabel } from "./projectnames";

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
  const url = selectedPetUrl();
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
  // A nearly spent subscription makes the idle pet anxious (mac CareChat.idlePool).
  if (mood === "idle" && limits.limitLow(limits.cached())) pool = [...pool, ...limits.ANXIOUS.map(t)];
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
  reportHitRect();
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
// Subscription limits: only the main window calls the provider (every 5 min,
// like mac NativeUsageProbe); project windows read the shared cache. Each
// window comments on the tightest visible window via the rateLimit metric.
// Opt-in: refresh() and cached() are empty while the Settings toggle is off.
async function pollLimits() {
  const found = IS_MAIN ? await limits.refresh() : limits.cached();
  flashReactive(reactive.evaluate("rateLimit", limits.lowestFractionLeft(found)));
}
setTimeout(() => { void pollLimits(); }, IS_MAIN ? 5_000 : 15_000);
setInterval(() => { void pollLimits(); }, limits.POLL_MS);

// Pull the bubble down over the canvas's empty headroom so it sits right
// above the pet's head (the sprite rarely fills the whole canvas height).
function snugBubble() {
  const gap = Math.max(0, canvas.clientHeight * pet.headroom - 4);
  petRoot.style.setProperty("--bubble-snug", `${gap}px`);
}

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
    usage.recordSession(e.project, e.agent);
    const now = Date.now();
    history.log({
      id: e.session, agent: e.agent, project: e.project ? projectLabel(e.project) : "",
      title: e.title || "", startedAt: sessionStarts.get(key) ?? now, endedAt: now,
    });
  }
  // Chimes + notifications fire once , the main window only.
  if (!IS_MAIN) return;
  if (e.state !== "done" && e.state !== "waiting") return;
  if (e.state === "done" && isFocused()) return;
  void audio.playSound(e.state === "done" ? "done" : "waiting");
  if (!notifyReady || localStorage.getItem("ap_notify") === "0") return;
  const proj = projectLabel(e.project || "") || e.agent;
  // Same copy as the macOS notifications.
  const title = e.state === "done" ? `${proj} ${t("finished")}` : `${proj} ${t("needs input")}`;
  // jcode only: its hooks carry the agent's opening line (what this turn did),
  // with the session title as a fallback (it goes stale, one per session). Other
  // agents keep the generic text, like macOS.
  const doneBody = e.agent === "jcode"
    ? ((e.message ?? "").trim() || (store.snapshot().find((s) => s.agent === e.agent && s.session === e.session)?.title ?? "").trim()).slice(0, 200)
    : "";
  const body = e.state === "done"
    ? (doneBody || t("Agent completed its turn"))
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
listen<{ agent: string; session: string; title: string }>("agent-title", (e) => {
  store.setTitle(e.payload.agent, e.payload.session, e.payload.title);
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
listen<{ agent: string; session: string; project: string; tokens: number; cost?: number; input?: number; output?: number; cacheRead?: number; cacheWrite?: number }>("agent-tokens", (e) => {
  const n = e.payload?.tokens || 0;
  if (n <= 0) return;
  const p = e.payload;
  if (!ownsProject(p.project)) return;
  usage.recordTokens(p.project, p.agent, n, p.cost || 0, { input: p.input, output: p.output, cacheRead: p.cacheRead, cacheWrite: p.cacheWrite });
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
  rememberPetUrl(e.payload.url);
});
// Language changed from Settings , re-render the bubble in the new language.
listen<Lang>("lang-changed", (e) => { setLang(e.payload); render(); });
// Bubble theme / opacity / messages changed from Settings.
listen("bubble-changed", () => { if (IS_MAIN) void pollLimits(); applyBubble(); applyPet(); moodLine = ""; armReminder(); render(); });

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

// Last interactive region in CSS px (set by reportHitRect below). Where the OS
// can't make the transparent part click-through (Wayland: cursor_position
// fails, so the whole window stays interactive) a press on empty space must
// still do nothing, so the document-wide handlers check it. Same region Rust
// uses on Windows (sprite bounds + visible bubble), so the gap between bubble
// and sprite still counts, unlike the sprite-only alpha test `pet.hitTest`.
let hitCss: { left: number; top: number; right: number; bottom: number } | null = null;
function inHitRect(e: MouseEvent): boolean {
  const r = hitCss;
  if (!r) return true; // not reported yet: stay interactive
  return e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
}

// --- interactions ------------------------------------------------------------
// Rust makes the window click-through outside the pet's hit rect (sprite box
// + bubble), so any left press that reaches this document is on the pet.
// Listen on the whole document: the hit rect also covers the gap between the
// bubble and the sprite and the empty sides of the bubble row, which are
// body/#pet-root, not the canvas. Pressing there used to be swallowed (not
// passed to the window below) yet did nothing, so the pet felt stuck.
document.addEventListener("mousedown", async (e) => {
  if (e.button !== 0) return;
  // Bubble controls (fold, carousel dots, dismiss, Warp-focus rows) keep their clicks.
  if ((e.target as HTMLElement).closest("button, a, input, .car-dot, .focusable, [data-nodrag]")) return;
  if (!inHitRect(e)) return;
  emit("popover-close", null);
  await invoke("drag_pet");
});
// Right-click opens the pet's stats card (macOS PetStatsView); the tray icon
// opens the quick popover.
document.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  if (!inHitRect(e)) return;
  invoke("open_stats", { pet: myPetSlug() }).catch(() => {});
});

// Report the interactive region (physical px) for Windows click-through: the
// union of the SPRITE's true bounds and the visible bubble , not the whole
// canvas, so the empty space beside the pet passes clicks to apps below.
const petRoot = document.getElementById("pet-root") as HTMLElement;
let lastHitSig = "";

function reportHitRect() {
  const d = window.devicePixelRatio || 1;
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
  } else {
    rects.push({ left: cr.left, top: cr.top, right: cr.right, bottom: cr.bottom });
  }
  // The union below stays the "visible bounds" (keeps the pet on screen). Clicks
  // use the PARTS (bubble, party badge, sprite) separately: a wide bubble above a
  // narrow pet otherwise makes one big box that also grabs the empty corners
  // beside the pet.
  const pad = HIT_PAD; // px of slack so grabbing right at the edge still works
  const parts: number[][] = [];
  for (const el of [bubbleEl, partyBadge]) {
    if (el.hidden) continue;
    const b = el.getBoundingClientRect();
    if (b.width > 0) parts.push([(b.left - pad) * d, (b.top - pad) * d, (b.width + 2 * pad) * d, (b.height + 2 * pad) * d]);
  }
  if (pet.silhouette.length && canvas.width > 0) {
    // The pet's outline (bands of the current frame), not its bounding box.
    const kx = cr.width / canvas.width, ky = cr.height / canvas.height;
    for (const b of pet.silhouette) {
      parts.push([(cr.left + b.x * kx - pad) * d, (cr.top + b.y * ky - pad) * d, (b.w * kx + 2 * pad) * d, (b.h * ky + 2 * pad) * d]);
    }
  } else {
    parts.push([(cr.left - pad) * d, (cr.top - pad) * d, (cr.width + 2 * pad) * d, (cr.height + 2 * pad) * d]);
  }
  const sp = rects[rects.length - 1]; // the sprite alone (pushed last), for the off-screen clamp
  const left = Math.min(...rects.map((r) => r.left));
  const top = Math.min(...rects.map((r) => r.top));
  const right = Math.max(...rects.map((r) => r.right));
  const bottom = Math.max(...rects.map((r) => r.bottom));
  hitCss = { left, top, right, bottom };
  const sig = [left, top, right, bottom, sp.left, sp.top, sp.right, sp.bottom, ...parts.flat()].map((v) => Math.round(v)).join(",");
  if (sig === lastHitSig) return;
  lastHitSig = sig;
  if (IS_MAIN) invoke("set_hit_parts", { parts }).catch(() => {});
  invoke("set_hit_rect", { x: left * d, y: top * d, w: (right - left) * d, h: (bottom - top) * d,
    sprite: [sp.left * d, sp.top * d, (sp.right - sp.left) * d, (sp.bottom - sp.top) * d] })
    .catch((err) => invoke("log_debug", { msg: `set_hit_rect failed: ${err}` }).catch(() => {}));
}
pet.onFrame = reportHitRect; // the outline changes with every animation frame
new ResizeObserver(reportHitRect).observe(petRoot);
window.addEventListener("resize", reportHitRect);
reportHitRect();

render();

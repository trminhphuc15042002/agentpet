// Shared sound engine for every AgentPet window.
//
// Why this exists (the old per-window copies were unreliable):
//  - WebView2 starts an AudioContext "suspended" until the page sees a user
//    gesture, so a chime fired by a background agent event could be silent.
//    We resume() on demand and expose unlockAudio() for the first gesture.
//  - Custom uploads used to play through `new Audio(dataUrl).play()`, which the
//    page CSP blocked: with no media-src, `data:` inherits default-src 'self'.
//    We now decode the data: URL ourselves and play it with Web Audio (not
//    governed by media-src), and fall back to the built-in chime when a custom
//    file cannot be decoded, instead of going silent.
//  - The old code wrapped .play() in try/catch, which never caught the async
//    rejection, and returned early, so one bad custom file muted everything.

export type SoundEvent = "done" | "waiting";

const TOGGLE: Record<SoundEvent, string> = { done: "ap_sound_done", waiting: "ap_sound_waiting" };
const DATA: Record<SoundEvent, string> = { done: "ap_sound_done_data", waiting: "ap_sound_waiting_data" };
const VOLUME_KEY = "ap_volume";
const LEGACY_KEY = "ap_sound";

let ctx: AudioContext | null = null;
const decoded = new Map<string, AudioBuffer | null>();
const inFlight = new Map<string, Promise<AudioBuffer | null>>();

function ac(): AudioContext | null {
  try {
    if (!ctx) {
      const Ctor = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!Ctor) return null;
      ctx = new Ctor();
    }
    if (ctx.state === "suspended") void ctx.resume().catch(() => {});
    return ctx;
  } catch {
    return null;
  }
}

/// Resume the context from a real user gesture so later chimes fired by agent
/// events are allowed by the WebView autoplay policy. Safe to call repeatedly.
export function unlockAudio() {
  ac();
}

/// Bind unlockAudio() to the first pointer/key press in this window.
export function bindAudioUnlock() {
  const once = () => unlockAudio();
  window.addEventListener("pointerdown", once, true);
  window.addEventListener("keydown", once, true);
}

export function soundEnabled(ev: SoundEvent): boolean {
  const legacyOff = localStorage.getItem(LEGACY_KEY) === "0";
  return (localStorage.getItem(TOGGLE[ev]) ?? (legacyOff ? "0" : "1")) !== "0";
}

export function getVolume(): number {
  const raw = parseInt(localStorage.getItem(VOLUME_KEY) ?? "100", 10);
  const pct = Number.isFinite(raw) ? Math.min(100, Math.max(0, raw)) : 100;
  return Math.round(pct);
}

export function setVolume(pct: number) {
  const v = Math.min(100, Math.max(0, Math.round(pct)));
  localStorage.setItem(VOLUME_KEY, String(v));
}

/// Synthesized fallback chime: a two-note "glass" for done, a lower two-note
/// "submarine" for waiting. Longer and louder than the old 0.13s single tone so
/// it is actually audible at default volume.
function chime(ev: SoundEvent) {
  const c = ac();
  if (!c) return;
  const level = getVolume() / 100;
  if (level <= 0) return;
  const notes = ev === "done" ? [880, 1318.5] : [587.3, 440];
  const t0 = c.currentTime + 0.01;
  notes.forEach((freq, i) => {
    const o = c.createOscillator();
    const g = c.createGain();
    o.type = "sine";
    o.frequency.value = freq;
    const start = t0 + i * 0.095;
    const peak = Math.max(0.16 * level, 0.0002);
    g.gain.setValueAtTime(0.0001, start);
    g.gain.exponentialRampToValueAtTime(peak, start + 0.012);
    g.gain.exponentialRampToValueAtTime(0.0001, start + 0.2);
    o.connect(g);
    g.connect(c.destination);
    o.start(start);
    o.stop(start + 0.22);
  });
}

/// Decode a stored `data:` URL without fetch() (connect-src would block it too).
function dataUrlToArrayBuffer(url: string): ArrayBuffer | null {
  try {
    const comma = url.indexOf(",");
    if (comma < 0) return null;
    const meta = url.slice(0, comma);
    const body = url.slice(comma + 1);
    if (/;base64/i.test(meta)) {
      const bin = atob(body);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      return bytes.buffer;
    }
    const text = decodeURIComponent(body);
    const bytes = new Uint8Array(text.length);
    for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i);
    return bytes.buffer;
  } catch {
    return null;
  }
}

function decodeCustom(dataUrl: string): Promise<AudioBuffer | null> {
  if (decoded.has(dataUrl)) return Promise.resolve(decoded.get(dataUrl)!);
  const running = inFlight.get(dataUrl);
  if (running) return running;
  const c = ac();
  const bytes = c ? dataUrlToArrayBuffer(dataUrl) : null;
  if (!c || !bytes) return Promise.resolve(null);
  const p = c
    .decodeAudioData(bytes)
    .then((b) => { decoded.set(dataUrl, b); return b as AudioBuffer | null; })
    .catch(() => { decoded.set(dataUrl, null); return null; })
    .finally(() => { inFlight.delete(dataUrl); });
  inFlight.set(dataUrl, p);
  return p;
}

/// True when a stored custom file decodes (used to warn on a bad upload).
export async function customDecodes(ev: SoundEvent): Promise<boolean> {
  const data = localStorage.getItem(DATA[ev]);
  if (!data) return true;
  return (await decodeCustom(data)) !== null;
}

function playBuffer(buffer: AudioBuffer) {
  const c = ac();
  if (!c) return;
  const level = getVolume() / 100;
  if (level <= 0) return;
  const src = c.createBufferSource();
  const g = c.createGain();
  g.gain.value = level;
  src.buffer = buffer;
  src.connect(g);
  g.connect(c.destination);
  src.start();
}

/// Play the sound for an event. Returns false only when the event is muted.
/// A broken custom file falls back to the built-in chime, never silence.
export async function playSound(ev: SoundEvent, opts?: { force?: boolean }): Promise<boolean> {
  if (!opts?.force && !soundEnabled(ev)) return false;
  const data = localStorage.getItem(DATA[ev]);
  if (data) {
    const buffer = await decodeCustom(data);
    if (buffer) { playBuffer(buffer); return true; }
  }
  chime(ev);
  return true;
}

/// Settings preview: the same engine a real notification uses, ignoring the
/// per-event on/off toggle so "play" always demonstrates the sound.
export function previewSound(ev: SoundEvent): Promise<boolean> {
  return playSound(ev, { force: true });
}

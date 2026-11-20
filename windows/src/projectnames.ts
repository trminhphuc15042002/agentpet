// Custom display names for projects ("reup video" instead of the folder name).
// Keyed by the same stable id as usage.projectId / the per-project pet map
// (FNV-1a of the path), stored in localStorage. Display only: ids, usage and
// history keep the real path.
//
// simplify: the hash is repeated here (not imported from usage.ts) so the small
// pet/bubble windows don't pull sync/care in just to hash a path. Keep it equal
// to usage.ts fnv1a (the id scheme is shared with per-project pets).

const KEY = "ap_project_names";

export function idForPath(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return "p" + (h >>> 0).toString(16).padStart(8, "0");
}

function load(): Record<string, string> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || "{}");
    return v && typeof v === "object" && !Array.isArray(v) ? v : {};
  } catch { return {}; }
}

/// The name set for a project path or id, or "" when none.
export function customName(pathOrId: string): string {
  const m = load();
  return m[pathOrId] || m[idForPath(pathOrId)] || "";
}

/// Sets (or, with an empty/blank name, clears) the name for a project id.
export function setProjectName(id: string, name: string) {
  const m = load();
  const n = name.trim().slice(0, 60);
  if (n) m[id] = n; else delete m[id];
  localStorage.setItem(KEY, JSON.stringify(m));
}

/// What to show for a project path: the custom name, else the last path part.
/// `fallback` is used when there is no path at all.
export function projectLabel(path: string, fallback = ""): string {
  if (!path) return fallback;
  const custom = customName(path);
  if (custom) return custom;
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

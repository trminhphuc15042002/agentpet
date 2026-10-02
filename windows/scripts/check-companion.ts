// Node 22+ self-check: node --experimental-strip-types scripts/check-companion.ts
import assert from "node:assert/strict";
import { focusActive, WaitingReminders, readLastEvents } from "../src/companion.ts";
import { groupSessions, type GroupConfig } from "../src/geometry.ts";

assert.equal(focusActive(100, 99), true);
assert.equal(focusActive(100, 100), false);
assert.equal(focusActive(NaN, 10), false);
assert.equal(focusActive(Infinity, 10), false);
const waiting = { agent: "opencode", session: "s1", state: "waiting", stateSince: 1000, updatedAt: 1000 };
const reminders = new WaitingReminders();
assert.equal(reminders.nextDelay([waiting], 1000), 120000);
assert.deepEqual(reminders.takeDue([waiting], 120999), []);
assert.deepEqual(reminders.takeDue([waiting], 121000), [waiting]);
assert.deepEqual(reminders.takeDue([waiting], 130000), []);
assert.equal(reminders.nextDelay([waiting], 130000), null);
reminders.nextDelay([{ ...waiting, state: "working" }], 130001);
const nextWait = { ...waiting, stateSince: 140000, updatedAt: 140000 };
assert.deepEqual(reminders.takeDue([nextWait], 260000), [nextWait]);
const afterSleep = new WaitingReminders();
assert.deepEqual(afterSleep.takeDue([waiting], 400000), []);
assert.equal(afterSleep.nextDelay([waiting], 400000), null);
assert.equal(new WaitingReminders().nextDelay([{ ...waiting, stateSince: Infinity }], 1000), null);
assert.equal(new WaitingReminders().nextDelay([{ ...waiting, stateSince: 1e15 }], 1000), null);
assert.deepEqual(readLastEvents("broken"), {});
assert.deepEqual(readLastEvents('[1,2]'), {});
assert.deepEqual(readLastEvents('{"claude":123,"bad agent":1,"codex":"x"}'), { claude: 123 });

const sessions = [
  { agent: "claude", session: "a", state: "working", updatedAt: 10 },
  { agent: "claude", session: "b", state: "waiting", updatedAt: 9 },
  { agent: "codex", session: "c", state: "working", updatedAt: 11 },
];
const cfg: GroupConfig = { hidden: [], filter: "all", grouping: "byKind", sortByKind: false, mode: "carousel", maxSessions: 5, pinnedKey: "claude:b" };
const grouped = groupSessions(sessions, cfg);
assert.equal(grouped[0].session.session, "b");
assert.equal(grouped[0].count, 1);
assert.equal(grouped.reduce((n, g) => n + g.count, 0), 3);
assert.equal(new Set(grouped.map((g) => g.session.session)).size, 3);
assert.equal(groupSessions(sessions, { ...cfg, maxSessions: 1, mode: "list" })[0].session.session, "b");
assert.equal(groupSessions(sessions, { ...cfg, hidden: ["claude"] }).length, 1);
assert.equal(groupSessions(sessions, { ...cfg, filter: "workingOnly" }).some((g) => g.session.session === "b"), false);
assert.equal(groupSessions(sessions, { ...cfg, pinnedKey: "gone" }).reduce((n, g) => n + g.count, 0), 3);
console.log("PASS companion: focus, once-per-wait reminders, sleep suppression, diagnostics parsing, pinned groups/filters/caps");

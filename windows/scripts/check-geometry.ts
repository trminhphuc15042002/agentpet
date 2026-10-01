// Node self-test for windows/src/geometry.ts (no extra deps).
//   node --experimental-strip-types scripts/check-geometry.ts

import {
  bubbleLayout,
  carouselShown,
  carouselStep,
  groupSessions,
  maintenanceSig,
  sessionIsVisible,
  shouldSendResize,
  sizeDiffers,
  visibleSessions,
  type Groupable,
} from "../src/geometry.ts";

let failed = 0;
function eq(name: string, got: unknown, want: unknown) {
  const gs = JSON.stringify(got);
  const ws = JSON.stringify(want);
  if (gs !== ws) {
    failed++;
    console.error(`FAIL ${name}: got ${gs} want ${ws}`);
  }
}

function almost(name: string, got: number, want: number) {
  if (Math.abs(got - want) > 1e-9) {
    failed++;
    console.error(`FAIL ${name}: got ${got} want ${want}`);
  }
}

const wide = bubbleLayout(60, 364, 364, 10, 20);
almost("wide bubble shift", wide.bubbleShift, 0);
almost("wide tail shift", wide.tailShift, 60);

const narrow = bubbleLayout(60, 364, 120, 10, 20);
almost("narrow bubble shift", narrow.bubbleShift, 60);
almost("narrow tail shift", narrow.tailShift, 0);

const shrink = bubbleLayout(-100, 364, 150, 10, 20);
almost("shrink bubble shift", shrink.bubbleShift, -100);
almost("shrink tail shift", shrink.tailShift, 0);

const tail = bubbleLayout(50, 100, 100, 10, 20);
almost("tail clearance", tail.tailShift, 20);

const zero = bubbleLayout(0, 364, 200, 10, 20);
almost("no offset bubble", zero.bubbleShift, 0);
almost("no offset tail", zero.tailShift, 0);

eq("stale index 1/1", carouselShown(1, 1), 0);
eq("stale index 2/2", carouselShown(2, 2), 0);
eq("stale index 5/3", carouselShown(5, 3), 2);
eq("empty shown", carouselShown(3, 0), 0);
eq("empty step", carouselStep(3, 1, 0), 0);
eq("step wrap +", carouselStep(2, 1, 3), 0);
eq("step wrap -", carouselStep(0, -1, 3), 2);
eq("step +", carouselStep(0, 1, 3), 1);
for (let stale = 0; stale < 6; stale++) {
  eq(`step after shrink ${stale}`, carouselStep(stale, 1, 1), 0);
}

eq("hidden kind", sessionIsVisible("claude", "working", ["claude"], "all"), false);
eq("visible kind", sessionIsVisible("claude", "working", ["codex"], "all"), true);
eq("filter workingOnly waiting", sessionIsVisible("claude", "waiting", [], "workingOnly"), false);
eq("filter workingAndWaiting done", sessionIsVisible("claude", "done", [], "workingAndWaiting"), false);
eq("filter doneAndAbove done", sessionIsVisible("claude", "done", [], "doneAndAbove"), true);

const sessions: Groupable[] = [
  { agent: "claude", state: "working", session: "a", updatedAt: 2 },
  { agent: "codex", state: "waiting", session: "b", updatedAt: 1 },
  { agent: "claude", state: "waiting", session: "c", updatedAt: 3 },
];
eq("visible none when hidden+filter", visibleSessions(sessions, ["claude", "codex"], "all").length, 0);
eq("visible workingOnly", visibleSessions(sessions, [], "workingOnly").map((s) => s.session), ["a"]);

const grouped = groupSessions(sessions, {
  hidden: [],
  filter: "all",
  grouping: "byKind",
  sortByKind: false,
  mode: "list",
  maxSessions: 5,
});
eq("group byKind count", grouped.length, 2);
eq("group claude collapsed", grouped.find((g) => g.session.agent === "claude")?.count, 2);

const capped = groupSessions(sessions, {
  hidden: [],
  filter: "all",
  grouping: "all",
  sortByKind: false,
  mode: "list",
  maxSessions: 1,
});
eq("list cap", capped.length, 1);

const carousel = groupSessions(sessions, {
  hidden: [],
  filter: "all",
  grouping: "all",
  sortByKind: false,
  mode: "carousel",
  maxSessions: 1,
});
eq("carousel ignores cap", carousel.length, 3);

eq("fallback when filters hide all", groupSessions(sessions, {
  hidden: ["claude", "codex"],
  filter: "all",
  grouping: "byKind",
  sortByKind: false,
  mode: "list",
  maxSessions: 5,
}).length, 0);

const live = { session: "s1", state: "working", subagents: [{ id: "child-old" }, { id: "child-live" }] };
const afterChildExpiry = { session: "s1", state: "working", subagents: [{ id: "child-live" }] };
eq("expiry keeps session count", [live].length, [afterChildExpiry].length);
eq(
  "expiry child changes maintenance sig",
  maintenanceSig([live]) === maintenanceSig([afterChildExpiry]),
  false,
);
eq("stable party sig", maintenanceSig([live]), maintenanceSig([live]));

eq("sizeDiffers 349 vs 271", sizeDiffers({ w: 349, h: 200 }, { w: 271, h: 200 }), true);
eq(
  "send 349 after tracked 349 actual 271",
  shouldSendResize({ w: 349, h: 200 }, { w: 271, h: 200 }),
  true,
);
eq(
  "retry while actual viewport still too small",
  shouldSendResize({ w: 349, h: 200 }, { w: 271, h: 200 }),
  true,
);
eq(
  "skip when viewport already next",
  shouldSendResize({ w: 349, h: 200 }, { w: 349, h: 200 }),
  false,
);
eq(
  "shrink still sends",
  shouldSendResize({ w: 200, h: 180 }, { w: 349, h: 200 }),
  true,
);

if (failed) {
  console.error(`${failed} failed`);
  process.exit(1);
}
console.log("check-geometry: ok");

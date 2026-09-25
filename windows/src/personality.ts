// Personality packs , data-only voice presets for the pet's spontaneous speech.
//
// Same events, different voice: a pack swaps the reactive phrase pools and the
// idle/done/celebrate chat lines, and scales how often the pet speaks up. It is
// deliberately no assets, no timers, no network , switching a pack is a
// localStorage read, so it stays cheap (see the perf rules in the spec).
//
// Packs are partial: any key a pack does not override falls back to the "cozy"
// defaults, which are exactly the phrases the app shipped with. Unknown ids
// also resolve to "cozy", so a stale saved value can never blank the pet.

export type PersonalityID = "cozy" | "tsundere" | "chaotic";

/// Reactive pools a non-default pack may override. Keys match the metrics in
/// `reactive.ts` (tier suffix Low/Mid/High).
type ReactivePools = Record<string, string[]>;

/// Chat overrides. `working` is intentionally absent: live activity text wins.
type ChatPools = Partial<Record<"waiting" | "done" | "celebrate" | "idle", string[]>>;

interface PersonalityOverride {
  /// >1 speaks less often, <1 more often. Scales cooldowns only, never the
  /// metric thresholds, so progress still feels earned.
  chattiness: number;
  reactive: ReactivePools;
  chat: ChatPools;
}

/// The phrases the app shipped with (pre-personality), now the "cozy" voice.
const DEFAULT_REACTIVE: ReactivePools = {
  dailyTokensLow: ["Burned quite a few tokens today~", "Eaten a lot of tokens", "Token usage rising"],
  dailyTokensMid: ["Big appetite mode!", "Great appetite today~", "Tokens going fast"],
  dailyTokensHigh: ["Token usage off the charts today 🔥", "Token burn is extreme!", "Heavy burn today"],
  sessionCountLow: ["5 agents running at once~", "Lots of agents at work", "Parallelism is up"],
  sessionCountHigh: ["Command center mode 😳", "So many sessions!", "Full throttle"],
  hungerLow: ["A little hungry…", "Hmm… want food", "Tummy rumbling"],
  hungerMid: ["Haven't been fed in a while 😢", "Hungry…", "Want food…"],
  hungerHigh: ["Where did you go… 😭", "About to faint from hunger", "So hungry"],
  streakLow: ["Days in a row! Keep going", "Going strong~", "Keeping it up"],
  streakMid: ["A whole week straight!", "Such persistence~", "So consistent"],
  streakHigh: ["Legendary streak!", "Incredible!", "Unstoppable"],
  dailyMealsLow: ["Lots of sessions today~", "Good productivity", "Got quite a bit done"],
  dailyMealsMid: ["Fifty sessions! Efficiency beast", "50+!", "Super productive"],
  dailyMealsHigh: ["Over 100! Not sleeping today?", "100+ sessions!", "Superhuman"],
};

const OVERRIDES: Record<Exclude<PersonalityID, "cozy">, PersonalityOverride> = {
  tsundere: {
    chattiness: 0.7,
    reactive: {
      dailyTokensLow: ["A few tokens. Big deal.", "Fine, tokens were burned."],
      dailyTokensMid: ["Fine, you're working.", "Spending tokens again, huh."],
      dailyTokensHigh: ["You burned the whole budget. Obviously.", "Don't blame me when it runs out."],
      sessionCountLow: ["So many agents. Show-off.", "Fine, parallel work. Impressive. Maybe."],
      sessionCountHigh: ["This many sessions?! Unbelievable.", "You're going to lose track of them."],
      hungerLow: ["I'm not hungry. I just… wouldn't mind food.", "It's fine. I'm fine."],
      hungerMid: ["Hmph. Feed me. Not that I care.", "I've been waiting, you know."],
      hungerHigh: ["Are you seriously leaving me here?", "I-it's not like I need you or anything… but feed me."],
      streakLow: ["A few days. Don't get smug.", "Consistency? Hmph, fine."],
      streakMid: ["A week. I noticed. Don't make it weird.", "Still going? Okay, that's… something."],
      streakHigh: ["Unstoppable? Don't let it go to your head.", "Legendary. Fine, I said it."],
      dailyMealsLow: ["Some work got done. Whatever.", "Not bad. Not that I'm counting."],
      dailyMealsMid: ["Fifty sessions. Show-off.", "Fine, you were productive."],
      dailyMealsHigh: ["Over a hundred? You're impossible.", "Go to sleep. That's not concern. It isn't."],
    },
    chat: {
      done: ["Fine, it works. I wasn't worried.", "There. Happy now?", "It's done. Don't expect applause.", "See? I told you it would work."],
      waiting: ["It needs you. Obviously.", "Your turn. Don't keep it waiting."],
      celebrate: ["Hmph, fine — we won.", "Don't get used to this.", "Okay, that was… decent."],
      idle: ["No agents running. Not that I mind.", "The repo is quiet. Suspicious.", "You could start something. If you want.", "I'm not bored. You're bored."],
    },
  },
  chaotic: {
    chattiness: 1.3,
    reactive: {
      dailyTokensLow: ["Tokens burning. It begins.", "More fuel for the fire!"],
      dailyTokensMid: ["Token furnace online 🔥", "Feed the machine!", "Big appetite! Big output!"],
      dailyTokensHigh: ["TOKEN VOLCANO 🌋", "Rich in tokens, poor in sleep!", "Burn it ALL!"],
      sessionCountLow: ["A small army stirs.", "Agents assembling…"],
      sessionCountHigh: ["COMMAND CENTER MODE 😳", "So many agents! Chaos!"],
      hungerLow: ["Snack. Now. Please.", "Tummy rumble detected."],
      hungerMid: ["FEED ME 🔔", "I am a hollow vessel of hunger."],
      hungerHigh: ["I'm fading… tell my commits I love them.", "STARVATION PROTOCOL ENGAGED 😭"],
      streakLow: ["Streak! Don't break it!", "Days in a row! Momentum!"],
      streakMid: ["A WHOLE WEEK 🎉", "Unstoppable streak!"],
      streakHigh: ["LEGENDARY STREAK!!!", "You cannot be stopped!"],
      dailyMealsLow: ["Sessions happened! Good.", "Productivity detected ✓"],
      dailyMealsMid: ["FIFTY SESSIONS 🎯", "Efficiency overload!"],
      dailyMealsHigh: ["OVER 100! SLEEP IS A MYTH!", "SUPERHUMAN MODE 🚀"],
    },
    chat: {
      done: ["SHIP IT 🚀", "DONE! Celebrate!", "It is finished! 🎉", "Yes! YES!"],
      waiting: ["YOUR INPUT! NEEDED! NOW!", "Poke! It needs you!", "Human! Over here!"],
      celebrate: ["🎉 WOOHOO!", "WE DID IT! PARTY TIME!", "VICTORY SCREECH! 🦅"],
      idle: ["Let's grill some bugs. 🔥", "Nothing running. Let's cause trouble.", "Start a branch! Start everything!", "The quiet is unsettling. Break it."],
    },
  },
};

const CHATTINESS: Record<PersonalityID, number> = { cozy: 1, tsundere: 0.7, chaotic: 1.3 };

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}

export function personalityID(): PersonalityID {
  const v = localStorage.getItem("ap_personality");
  return v === "tsundere" || v === "chaotic" ? v : "cozy";
}

/// Reactive phrase pool for a metric tier, falling back to the cozy defaults.
export function reactivePool(key: string): string[] | null {
  const id = personalityID();
  if (id !== "cozy") {
    const override = OVERRIDES[id].reactive[key];
    if (override) return override;
  }
  return DEFAULT_REACTIVE[key] ?? null;
}

/// Chat override for a mood, or null when the pack keeps the built-in lines.
export function chatPool(mood: string): string[] | null {
  const id = personalityID();
  if (id !== "cozy") {
    const override = OVERRIDES[id].chat[mood as keyof ChatPools];
    if (override) return override;
  }
  return null;
}

/// Multiplier applied to the reactive cooldowns: chaotic speaks sooner (scale
/// < 1), tsundere later (scale > 1). Clamped so a pack can never spam.
export function cooldownScale(): number {
  return 1 / clamp(CHATTINESS[personalityID()], 0.5, 2);
}

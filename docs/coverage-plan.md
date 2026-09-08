# Full-coverage exploration through the LLM path — Implementation Plan

Replace "forty minutes of random play from twenty-six places" with "every action in the game, taken
once, through the stack exactly as it is deployed". A god party and the debug tier make battles
unlosable so the story can be finished in minutes rather than hours; the finished save is then the
starting point for an exhaustive walk of the world.

**Status.** Written 2026-09-06. Updated 2026-09-07 after the first two implementation passes, and
**2026-09-08 after the sweep loop**: nine sweeps, each one fixing what the last surfaced. **Nine
faults, eight fixed, two open work items** — the fixes are §5.2.6, the open items are §5.2.7 and are
written up for someone else to pick up. Six of the eight were agent bugs rather than harness ones,
which is the oracle doing its job.

| | | |
|---|---|---|
| **C0** | the LLM e2e harness | ✅ **built.** `integration_tests/llm_harness.rs`; `llm.rs` moved onto it; all seven faults have a test; the ⛔ 402 death loop of §2.2.1 is **fixed** — (a)–(d) below |
| **C1** | the cheat tier | ✅ **built.** Four new `debug_*` primitives and `integration_tests/cheats.rs`; `play_path_contains_no_debug_ram_writes` still passes unchanged |
| **C2** | the god run | ◐ **the machinery and the measurement.** `integration_tests/godmode.rs`: `Intent`, `ScriptedBrain`, the driver, and `godmode_turn_cost` behind `--features godmode`. **The run to the Hall of Fame is not built** — see §4.5 |
| **C3** | the exploration | ◐ **the oracle, the frontier and the sweep loop.** `integration_tests/coverage.rs`: `CoverageLog`, the verdict table, `ExploringBrain`, a 30-second progress heartbeat and an honest stop reason. ~535 ids across **38 maps** a walk at ~48× real time; nine faults found, eight fixed (§5.2.6), two open (§5.2.7). ⚠️ **38 of 248 maps is the walk's ceiling today** and closing it is W2. **Branch-point snapshots and the ROM cross-check are not built** |
| **C4** | the battle matrix | ◐ **the audit is done and committed** — §6.0. Six cells exist nowhere, sixteen are proved under `DeterministicPolicy` only. No test written yet |

⚠️ **Where this plan was wrong is recorded rather than edited out** (§11.5). Two so far, both in §3.3
— see §0.4 below.

**The point is the end-to-end.** Every phase runs the real agent, the real `LlmPolicy`, the real
worker and the real wire, against a mock endpoint in-process. A phase that could be done more cheaply
against `DeterministicPolicy` is deliberately not, because the bugs that have actually killed
deployed runs were in the turn loop and the prompt, not in the agent.

---

## 0. What changed from the first draft of this plan

Three corrections, all of them Alex's, all of them recorded because the first draft was wrong in
ways worth not repeating:

1. ⚠️ **The action universe cannot be enumerated from the ROM.** The first draft proposed deriving
   it from `Map::iter()`, `read_warp_events`, `header.connections()` and `map.sprites()` — 248 maps,
   1127 `MapSprite` constants. Those tables are real, but they are not the universe: whether a row
   appears in `MetaTileMap::actions()` is a function of *live* state — hidden-object flags, sprite
   `hidden` bits, event flags, tileset, `can_cut`/`can_surf`/`can_strength`, and reachability from
   where the player is standing. Red gates almost everything through runtime scripts. So the universe
   is **discovered**, not enumerated, and the termination condition is a fixpoint — "a full pass
   discovered no id we had not already seen" — not a match against a static list. §5.3 keeps the ROM
   tables as a *cross-check* only.
2. ⚠️ **Gating is not a constraint to respect, it is a thing to cheat past.** The first draft treated
   badge/HM/flag gates and one-shot content as a reason to accept partial coverage and write "names
   its blocker" beside each miss. Wrong trade: the debug tier exists, and a save state plus three
   tests covers a three-way branch exactly.
3. ⚠️ **Do not split the programme up to de-risk it.** The harness comes first *as a harness*, and
   everything after it is a client of that harness. Nothing gets a cheaper non-LLM path "for now".

### 0.4 What the first implementation pass found the plan wrong about

Recorded here rather than fixed in place, because the reasoning is the useful part.

1. ⚠️ **§3.3's god party cannot be two Pokémon.** "One Mewtwo… and a second slot for the HM slave"
   does not fit: Gen 1 has **five** HMs and a Pokémon has four move slots. Fly is the one that falls
   off, and it is not droppable — it is the only field move that *travels*, so a coverage walk
   without it can never reach a map whose only other approach is gated. The god party is therefore
   **three**: a fighter with the four attacks §3.3 argues for, a slave with Cut/Surf/Strength/Flash
   (the four the *action menu* gates rows on), and a slave with Fly. One more party slot is free;
   an unreachable third of the world is not. `cheats::FLIGHT_MOVES` carries the argument.
2. ⚠️ **§5.2's `CoverageLog` could not be written over `AgentEvent` as it stood.** An action id is
   `{map}:{x},{y}:{kind}` and no event carried the first three: `StartedOverworldAction` carried a
   `MetaTile` and `OverworldActionAborted` carried where the walk *stopped*, which is a different
   square. So the id is now minted once by `OverworldAction::id` — moved out of `llm::tools`, which
   `agent.rs` is not even compiled with — and carried on `StartedOverworldAction`. Pairing is
   positional: a start opens an id and the next terminal overworld event closes it.

---

## 1. The shape

Four things, in this order. The first is the only one that is pure infrastructure.

| | | |
|---|---|---|
| **C0** | The LLM e2e harness | A mock endpoint with a pluggable brain, a fault injector, and an assembled stack with a run directory |
| **C1** | The cheat tier | New `debug_*` primitives, and the sidecar that applies them between ticks |
| **C2** | The god run | A fresh save to the Hall of Fame through `LlmPolicy`, in minutes. Replaces `full_playthrough` |
| **C3** | The exploration | From C2's finished save: every reachable action, once, with a verdict on each |
| **C4** | The battle matrix | Every action class in every battle type, including the refusals |

### 1.1 The rule that makes cheating safe

`src/pokemon/postgame/debug.rs` already draws the line and a test already enforces it:

- **Play path** — anything reachable from `Policy::pick_*`: button input only, no exceptions.
- **Debug tier** — free to write RAM, for building fixtures and seeding tests.

`play_path_contains_no_debug_ram_writes` reads the play-path sources from disk and fails if `debug_`
appears in one. ⚠️ **That test must keep passing, unchanged.** It is the whole reason the findings
from this programme will be trustworthy: the cheats live in the *driver*, applied between agent
ticks, and reach the policy only through an ordinary `GameState`. `LlmPolicy` is byte-identical to
the deployed one, and a jam it walks into is a jam a paying run would have walked into.

### 1.2 The cheat that is not needed

⚠️ **C2 plays the story rather than skipping it.** The god party makes every battle unlosable; it
does not set the event flags that would let the run teleport to the end. So the save C3 explores from
is one the cartridge's own scripts produced, with every gate opened by the thing that opens it.

This matters more than it sounds. Setting `wEventFlags` wholesale desynchronises scripts from map
objects — an NPC whose flag says "gone" while the object is still in the map, a door whose script has
already run — and every stall found in such a save is a false positive that costs a day to
disbelieve. The only flag writes this plan admits are in §5.4, each one named and argued.

---

## 2. C0 — the LLM e2e harness

Today `integration_tests/llm.rs` builds the whole stack for one test: a real socket, a real
`text/event-stream` body, `OpenAiClient` parsing it, the real worker, the real `LlmPolicy`, the real
agent, the real emulator. Only the model is a stand-in, and it is forty lines that find the warp in
the menu. Everything below is a client of that same assembly, so it stops being bespoke.

### 2.1 `MockEndpoint` and `Brain`

Promote the mock into `integration_tests/llm_harness.rs`:

```rust
/// Everything a brain is allowed to see: the request, as strings.
pub struct TurnRequest {
    pub system: String,
    pub messages: Vec<Message>,   // role + rendered content
    pub tools: Vec<ToolSchema>,   // name + JSON schema
}

pub trait Brain: Send {
    fn respond(&mut self, request: &TurnRequest) -> Reply;
}

pub enum Reply {
    Calls(Vec<ToolCall>),   // fragmented across `data:` frames by the endpoint
    Fault(Fault),
}
```

⚠️ **A `Brain` sees strings and nothing else, and that is enforced by the type rather than by a
comment.** It gets no `GameState`, no `&mut GameBoy`, no fixture handle. If a brain cannot find what
it needs in the rendered situation and the action menu, a real model cannot either — and that is a
finding about `llm::prompt`, not a test to work around. This is the single most valuable property in
the whole plan and it is free.

⚠️ **Keep the argument fragmentation.** The existing mock splits tool-call arguments across several
`data:` frames on purpose; it is the guard on the one part of the wire format most likely to be got
wrong. Anything that replaces it inherits that behaviour.

### 2.2 Faults

`Fault` is why C0 comes first. These are the failures that have actually ended deployed runs, and not
one of them has a test:

| Fault | What it must do |
|---|---|
| `Http { status: 402, .. }` | An undated hard failure. ⛔ **The known-open death loop** — the whole diagnosis is §2.2.1 |
| `Http { status: 429, retry_after: Some(..) }` | Park the run: emulator stopped, cartridge clock stopped, page dimmed, same question re-asked when the window reopens |
| `Http { status: 429, retry_after: None }` | Ordinary transient: backoff in seconds, no park |
| `Timeout` | Exceeds `GB_REQUEST_TIMEOUT_SECS` with the body half-written |
| `MalformedToolArgs` | Valid SSE, arguments that are not JSON, or JSON missing a required field |
| `TruncatedStream` | Socket closed mid-`data:` |
| `EmptyChoice` | A completion with neither content nor a tool call |

Each gets one test asserting what the run does next, and — for the park — that the cartridge clock
did not advance, which is the figure the leaderboard ranks on.

### 2.2.1 ⛔ The 402 death loop, in full

✅ **Fixed 2026-09-07**, all four criteria, and the diagnosis below is kept exactly as it was written
because it is the evidence. What changed:

- (a) and (b) — `worker::TurnOpen` and `Worker::roll_back_failed_turn`. A turn records the history
  length **and** `turns_since_plan` before `sync_plan` runs, and a turn that produced no completion
  at all puts both back. The rollback is to a *length*, not one `pop`: a turn can fail on its second
  tool step with an assistant message and its results already appended.
- (c) — `Worker::drop_unanswered`, the last resort's last resort, plus a loud `error` notice when a
  compaction reclaims nothing at all. `trim_history` cuts only at turn boundaries and a plan message
  is deliberately not one, so a history made entirely of unanswered plans had nothing it could drop.
- (d) — `llm::an_undated_hard_failure_does_not_ratchet_the_history`, default tier, 25 consecutive
  402s. Against the old code it fails with `[3, 3, …, 4, 4, …, 5, 5, 5]`, one message per
  `PLAN_REFRESH_TURNS`, which is exactly the shape the deployed file had.

⚠️ **The undated 402 is still not parked**, deliberately: §2.2's own ⚠️ says a park would not have
fixed the ratchet, and the ratchet is what made this terminal. A run against a dead endpoint now
sits at a constant history size and a `wait` every 100 ticks, saying so on every turn.

Observed 2026-09-05 on `run-20260902-215720` (`glm-5.3-flash` via OpenRouter), which reached turn
**16 555** and a **403 300-token** history against a `GB_CONTEXT_LIMIT` of 100 000.

**The trigger.** The OpenRouter credit ran out. That does not present as the dated 429 the park in
`worker.rs` is built for. It is a **402 with no reset time**:

```
the endpoint returned 402: Prompt tokens limit exceeded: 361796 > 36238.
To increase, visit https://openrouter.ai/settings/credits and add more credits
```

So it is treated as an ordinary turn failure and retried at once, about once a second, for ever.

**Three faults compound into a ratchet**, and a fix needs all three:

1. **A failed turn's user message is kept.** `history.json` ended as 1377 messages of which **1375
   were `user`** — one assistant message and one tool message in the whole file. Nothing removes the
   situation message a turn was rejected on.
2. **The plan message is re-appended on a failed turn.** 1373 of those 1375 began `## Your plan`.
   `turns_since_plan` was stuck at 10, so the "every tenth overworld turn" re-send fired every turn:
   the counter is reset by a *completed* turn, and none completes. About 1 kB of tokens per second.
3. ⭐ **Compaction cannot recover and its fallback is a silent no-op.** Summarising is itself a
   request, so it 402s too; the fallback logs `could not summarise the history (…); dropping the
   oldest turns instead` and then reports `{"before":403300,"after":403300,"summarised":false}`. It
   dropped nothing, because the history holds no *completed* turns to drop — only unanswered user
   messages.

⚠️ **The UI and the transcript both look healthy throughout.** The `decision` event carries
`usage.context_tokens: 48450` against `context_limit: 100000` — the last *successful* figure, frozen
— while the real prompt is 361 796. `prompt_tokens`, `completions` and `completion_tokens` are frozen
too. ⚠️ And the agent keeps playing, so `kubectl logs` shows boulder pushes and won battles and looks
entirely fine; the errors are in `transcript.jsonl`, not in the pod log.

**The four acceptance criteria for the `402` fault test**, which is what makes this a work item
rather than a story:

- (a) the messages a failed turn appended are rolled back;
- (b) a failed turn does not count towards `turns_since_plan`;
- (c) the compaction fallback can drop unanswered user messages, and when it drops nothing it says
  so rather than emitting `before == after` as a success;
- (d) the history does not grow across N consecutive failures. This is the assertion that would have
  caught it, and it is one line.

⚠️ Treating an undated quota/credit 402 as a **park** stops the bleeding but does **not** fix the
ratchet, so it is not sufficient on its own. Any endpoint can produce an undated hard error.

### 2.3 `LlmRun`

The assembled stack, with the seams the current test omits:

- a **run directory**, so `history.json` / `conversation.jsonl` / `todo.json` / `battle-script.json`
  are written and read exactly as they are in deployment;
- **`step_coarse`**, not `step`. ⚠️ The test-suite doc is explicit that every test in the suite hands
  the agent exactly one tick's worth of time while both real drivers hand it however long their last
  loop iteration took, and that the defect of 2026-09-03 lived entirely in that gap. An e2e harness
  that reproduces deployment must use the driver's cadence;
- a **restart**, mid-run: drop the worker and the fixture, rebuild both from the run directory, and
  carry on. This is the only way `GB_RESTORE_HISTORY`, the re-minted system prompt and the
  "conversation is a little ahead of the save" line are ever exercised;
- a **battle script**, installed through the real `set_battle_script` tool so the seven made-up
  battles and the arming path are covered rather than bypassed.

### 2.4 Acceptance

`the_llm_plays_from_a_fixture`, `the_watchdog_asks_the_model_for_a_nudge_and_delivers_it` and the two
battle probes are rewritten on the harness with nothing lost, and the seven faults each have a test.
Default tier, seconds.

---

## 3. C1 — the cheat tier

### 3.1 New `debug_*` primitives

`postgame/debug.rs` has money, coins, items, dex, party, faint and options. Add:

- `debug_set_badges(Badges)` — `wObtainedBadges`.
- `debug_heal_party()` — every party member to full HP, status cleared.
- `debug_restore_pp()` — every move to its max PP.
- `debug_teach_move(slot, Move)` — for the HM slave, so field moves are available without the TM/HM
  bookkeeping.
- `debug_set_party` already exists and builds the god party.

### 3.2 The sidecar

A `Cheats` struct the driver applies between agent ticks, never inside a policy:

```rust
impl Cheats {
    /// Applied by the driver between ticks. Never from `pick_*`.
    fn apply(&mut self, api: &mut PokemonApi, state: &GameState);
}
```

⚠️ **Top up in the overworld only, never in a battle.** Gen 1 copies the active party member into
`wBattleMon` on send-out and writes it back on switch-out; a party-struct write mid-battle
desynchronises the two, and the symptom is a Pokémon that heals and then un-heals on the next switch.
Gate the top-up on `!in_battle` and on the black-out window (`wIsInBattle == $ff`) being closed.

### 3.3 The god party

One Mewtwo is the proposal and it is very nearly right. Two adjustments:

- ⚠️ **Level 100 with a wide move set, not a single move.** `battle.fight` picks from what is known,
  and a Psychic-only Mewtwo cannot touch a Gengar or a Persian resisting... it can, but the point is
  that the *script* is what is being tested, and a script with one legal choice tests nothing about
  move selection. Give it Psychic, Blizzard, Thunderbolt and Earthquake and the script has a type
  chart to be right about.
- ⚠️ **A second slot for the HM slave from the start.** Field moves are gated on badge *and* on a
  party member that knows the move; a one-Pokémon party makes every field move a slot decision.

⚠️ **The starter is still chosen by playing.** Oak's script is one of the things being tested, and
overwriting the party before it runs is the class of cheat §1.2 rules out.

### 3.4 Acceptance

`play_path_contains_no_debug_ram_writes` still passes, unchanged. Each new primitive has a
default-tier test that writes and reads back through the ordinary `GameState` path — not through the
symbol it wrote, which would only prove the write landed where the write went. The sidecar has a test
that it refuses to fire during a battle and during the black-out window.

---

## 4. C2 — the god run

A fresh save to the Hall of Fame, through `LlmPolicy`, with the battles won by a script.

### 4.1 What decides where to go

A `ScriptedBrain`: an intent list, each intent resolved against the **rendered action menu** rather
than against `GameState`.

```rust
enum Intent {
    GoTo(Map),
    Talk(MapSprite),
    Warp { to: Map },
    FieldMove(FieldMove),
    Battle,            // hand the turn to the script
    // …
}
```

`PolicyStep::complete_game_steps()` is the route and should be reused wherever a step maps to an
intent one for one. ⚠️ **Where it does not map, that is the finding.** A step that knows something
the menu does not say is a gap in `llm::prompt`, and the run should fail naming the intent it could
not resolve rather than reaching around to `GameState` for it.

### 4.2 What it replaces

`full_playthrough` goes. Its stated job is to prove the legs compose, and `hall_of_fame_playthrough`
— same fresh save, `complete_game_steps()` instead of `eight_badge_steps()` — is a strict superset
that keeps that job for the scripted route `--policy deterministic` actually deploys.

⚠️ **`hall_of_fame_playthrough` stays and stays maintained.** It is the only test of the scripted
route, and the scripted route is what is running in production today. The god run does not test it
at all: different policy, different decisions, different RNG.

What is lost by dropping `full_playthrough` is a **7-minute pre-push gate**. The god run has to
become that gate, so it has a wall-clock budget rather than an open one:

- no grind — the wild-battle grind that is most of the scripted route's length exists to make the
  Elite Four a certainty, and a level-100 Mewtwo needs none of them. ⚠️ Do not quote a figure for it
  from memory: `README.md` says ~840 battles for one fighter to lv85, and `Cargo.toml`'s
  `hall-of-fame` comment still says 2306 for three to lv75 — the route changed and that comment did
  not. Measure it if the number matters;
- no losses, so no black-out warps and no re-walks;
- battles decided by the script on the first poll, so no request and no round trip.

⚠️ **The turn cost is the unknown and must be measured before this is committed to.** A round trip to
a localhost mock plus a prompt build over a growing history is not free, and the history grows until
compaction bounds it. The first thing C2 produces is a number: milliseconds per overworld turn, and
turns to the Hall of Fame. If it lands above about ten minutes it goes behind its own feature and
`full_playthrough` stays as the gate.

### 4.2.1 ✅ The measurement, taken 2026-09-07

`godmode_turn_cost`, Viridian Mart to Pewter City through the deployed stack with the god party and
`battle_script::DETERMINISTIC` armed through the real `set_battle_script`:

```
requests           8 (0 of them battle turns)
game time          247 s          wall clock 4.4 s   (56x realtime)
turn latency       1 ms mean, 4 ms worst
turn rate          1.9 requests per game-minute
thinking : running 0.00
```

⭐ **The turn cost is not the risk, and that answers §4.2's question.** A turn against a localhost
mock costs about a millisecond of worker time; the emulator runs at ~56x realtime throughout, and the
two overlap, so a god run's wall clock is **its emulated game time divided by ~56** and nothing else.
At 1.9 requests per game-minute the endpoint would have to be ~30 000x slower before it became the
bound.

⚠️ **It is a lower bound and the test says so.** The latency was measured over a 25-message history;
a prompt is rebuilt from the whole conversation every turn, so this term grows until compaction
bounds it. Re-measure on a run long enough to compact before quoting it as the god run's figure.

⚠️ **So the remaining unknown is entirely "how much game time".** `full_playthrough` is ~7 min of wall
clock, i.e. ~390 game-minutes. A god run has no grind, no losses and no re-walks, so the comparison
to make is game-minutes against game-minutes — not turns, and not milliseconds.

### 4.3 Acceptance

Reaches `Map::HallOfFame` from `start-of-game-state.bin`, having actually played: assert on badges,
on the intent list being exhausted rather than the agent wandering into the credits (the bound
`hall_of_fame_playthrough` uses, and read its ⚠️ about *two* rather than one first), and on at least
one compaction having fired. Prints ms/turn and total turns on the way out — that number is the
deliverable, not a nice-to-have.

### 4.4 What it proves that nothing today proves

A full game's worth of: chaining (`then`), `resume_after_battle`, `take_over`, the battle report,
compaction firing for real, the plan surviving it, a restart resuming mid-run, and every terminal
tool being reachable. All of it against the wire.

### 4.5 What is built, and what the rest of C2 is

Built (`integration_tests/godmode.rs`, machinery default-tier, the measured run behind `godmode`):

- `Intent` — `Enter(map)`, `Row(kind)`, `Says(fragment)`, `Wait` — every variant answerable from the
  strings in a `TurnRequest`, with `names_map` so `Route1` cannot match `Route11`;
- `ScriptedBrain` — carries out an intent list, arms the battle script on its first overworld turn
  as a *read* tool paired with the action (one request, not two), asks for `resume_after_battle` on
  every walk, and **records** an unresolvable intent with the menu it was looking at rather than
  panicking on the mock's thread;
- the driver — `LlmRun` with the `Cheats` sidecar applied between ticks and a `CoverageLog` on.

What remains is **the intent list**, and it is the whole of the difference between Pewter City and
the Hall of Fame. `PolicyStep::complete_game_steps()` has variants with no menu row behind them at
all — `UseBagItem`, `Fish`, `UsePcBox`, `UseItemsInBattle`, `PartyScript`, `UseItemPc` — and §4.1 is
explicit that where a step does not map, **that is the finding**: either a gap in `llm::prompt` or an
`Intent` this file has to grow. Working through them one at a time, each with its own argument, is
the rest of C2.

---

## 5. C3 — the exploration

### 5.1 The frontier

From C2's finished save. An `ExploringBrain` — again reading only the rendered menu:

1. Every row in the menu has an id, `{map}:{x},{y}:{kind}` (`MetaTile::id_kind`).
2. Choose the first unvisited id on this map. Record the id and the outcome.
3. When the map has no unvisited rows, go to the nearest map that has one.
4. Stop when a full pass over the reachable world discovers no id not already seen.

⚠️ **The frontier is over ids, and ids are discovered by standing there.** A map is not finished the
first time its rows are exhausted: a row can appear later because a flag changed, a Pokémon learnt a
field move, or an item entered the bag. Hence the fixpoint in (4) rather than a per-map done-flag.

### 5.2 The verdict oracle

This is the part that answers the actual question — "the agent failed to run an action, and the
watchdog stepped in, or it gave up for a reason other than a battle starting". `OverworldActionAbortedReason`
already splits cleanly:

| Reason | Verdict |
|---|---|
| `Battle`, `NamingScreen` | expected. Retry the id |
| `Textbox`, `Script` | expected **once**. A repeat is the signal — the deployed run aborted on `the way into Route2` 143 times |
| `Unknown`, `DidNotArrive`, `NoRoute`, `WrongMap`, `NoAdjacentGrass` | **defect**. The menu offered a row the agent could not then execute |
| `WatchdogFired` | **defect**, always |

So each id ends in one of: `completed`, `blocked (quoted message)`, `defect (reason)`, `unreached`.
The run fails on any `defect`, and writes the whole table either way.

⚠️ **A `CoverageLog` over `AgentEvent`s, not over the policy.** It consumes the published event
stream, so it works under *any* driver — the god run, the exploration, `hall_of_fame_playthrough`,
the leg chain. Wiring it into the existing tests costs nothing and says how much of the world they
already touch, which is the number that decides how much C3 is really worth.

### 5.2.1 ✅ The oracle, built 2026-09-07

`integration_tests/coverage.rs`. `CoverageLog::observe(&AgentEvent)` folds the stream into a table
keyed on the action id, with the four verdicts §5.2 asks for: `Completed`, `Blocked { times,
message }`, `Defect { reason }`, `Unreached`. `REPEAT_IS_A_DEFECT` is 3 — the repeat is the signal,
not the first block. It writes a TSV to `target/test-artifacts/coverage/`.

Two things the plan did not anticipate:

- ⚠️ **The id had to be put on the event** — see §0.4.2.
- ⚠️ **A quote arrives *after* the abort it belongs to.** Red turns the player back by printing a
  message and *then* running a script that steps them backwards, so the walk is abandoned first and
  the reader drained afterwards. The log therefore keeps the id open across the block so the next
  `TextBox` lands on it.

Wired into `TestFixture::with_coverage()` (opt-in: the fixture then *owns* the event stream, because
the agent's buffer is drained rather than peeked and is capped at 100) and into
`early_game::can_navigate_to_pewter_city`, which now asserts no hard defect and no watchdog firing
and prints its own figure: **7 ids across 6 maps, 7 completed**. That is the number §5.2 said would
size the rest, and it says plainly that the existing leg tests touch almost nothing.

### 5.2.2 ✅ The frontier, and what its walks have found

`coverage::ExploringBrain`, behind `--features coverage-tests`. 90 game-minutes from
`pallet-town-state.bin` with the god party and the badges, ~95 s of wall clock.

The first walk, 2026-09-07:

```
frontier   352 ids offered, 307 chosen, across 28 maps in 344 turns
rate       3.9 ids discovered per game-minute
settled    false (stopped on the 90-game-minute budget with the frontier still open)
verdicts   221 completed, 18 blocked, 66 silent, 0 defects
silent     {"CutTree": 14, "Grass": 52}
```

**That is §5.6's number**: ~4 ids per game-minute, and the curve had not flattened when the budget
ran out. For scale, `can_navigate_to_pewter_city` — a whole leg of the existing suite — touches
**7**.

⭐ **Finding 1: `Grass` and `CutTree` are actions the model is never told the outcome of.** A new
verdict, `Silent`: the action was taken and no terminal event followed. Sixty-six of 307, and every
one of them one of those two kinds.

✅ **Closed the same day**, and it turned out to be worth more than a log line. The fix is in
`agent.rs`: `AgentState::PacingForEncounters` carries the `MetaTile` its row named and reports all
three of its exits (an encounter as `Battle`, the budget as a new `NothingAppeared`, a map change as
`WrongMap`), and `AgentState::CuttingTree` carries `from_row` and ends in
`OverworldActionCompleted { Cut }`. Both had been ending in a `TextBox` **the agent made up**, which
is the cartridge's voice used for the agent's own account, and one of them had the budget wrong
besides: a tick is 19.9996 ms and `as_millis()` is 19, so the model was told 57 seconds where the
budget is 60.

⭐ **And `resume_after_battle` was dead on tall grass, which nothing was looking for.** It keys on
`OverworldActionAborted { Battle }` and a pace emitted no abort at all, so every wild encounter in
grass dropped the queue as `Dropped::Unreported` and cost a fresh request. That is the commonest way
in the game to meet a wild Pokémon, and the feature the README describes had never once applied to
it. ⚠️ **No test under `src/llm/` could have seen this**:
`a_chain_does_not_advance_on_an_ending_the_agent_never_reported` pins what happens *given* silence
and cannot say which endings are silent. Scoring every id somebody chose is what found it.

⭐ **Finding 2 was in the plan's own oracle, not in the game.** `REPEAT_IS_A_DEFECT` was 3, on the
reasoning that "the second attempt is already a model that did not read the answer". That ignores the
honest case: a gate is worth **one** try per pass, because the thing that opens it may have happened
since. The walk re-tried Pewter City's east exit — Brock's gym guide, who blocks it until the Boulder
Badge — once on each of three sweeps, and was called a defect for diligence. It is 10 now: clearly
above once-per-pass and clearly below the 143 that made this a rule.

⭐ **Finding 3 was in the frontier heuristic, and the oracle caught it.** The first version scored an
exit by where it led (a map with no ids yet is the most promising) and tie-broke on whether it had
been taken. Being turned back at a gate changes nothing the brain can see, so the best-scoring row
stayed the best-scoring row: `PewterCity:40,18:Connection` was taken **59 times in one run**. The
same shape as the deployed run's 143. Ordering exits by *how often they have already been taken*
first, and promise second, takes every exit once before any twice — and moved the walk from 12 maps
to 28.

⭐ **Finding 4 is the brain's, and it is the price of Finding 1's fix.** The explorer asked for
`resume_after_battle` on every row. Once a pace started reporting its encounters, that finally *did*
something — and what it did was grind: a `Grass` row resumed through `MAX_BATTLE_RESUMES` battles
before handing back, so the same 90 game-minutes bought **16 maps and 258 ids** instead of 28 and
352. Resuming is the right answer for a model playing the game and the wrong one for a walk whose
whole job is breadth, so the brain now asks for it on everything except `Grass` and `Empty` — the two
rows whose entire purpose is to *start* a battle.

### 5.2.3 ✅ Finding 5: a walk that had already arrived, reported as a routing failure

With Finding 1 closed the walk scores every id it takes, and the first thing that came out of that
was a **defect** in about two runs in three. The run that caught three at once is the whole story:

```
Route2:8,0:Connection        there is no route to the way into PewterCity (standing at (8, 73))
Route2:9,0:Connection        there is no route to the way into PewterCity (standing at (9, 73))
ViridianCity:19,0:Connection there is no route to the way into Route2     (standing at (19, 37))
```

⭐ **The shape is the diagnosis.** Every target is on row **0** and every reported position on the
map's **last** row, in the same column. And the event straight after each abort says the walk
*worked*: the id after `ViridianCity:19,0` failing is `Route2:8,73:Connection`, and the id after
`Route2:8,0` failing is `PewterCity:40,18:Connection`. The agent was calling a successful walk a
pathfinder failure.

The save state dropped at the abort settled it in one probe:

```
tick 0   wCurMap=13 (Route2)     raw=(8, 255)    <- wYCoord is -1
tick 1   wCurMap=2  (PewterCity) raw=(18, 35)
```

Crossing a connection **north or west** leaves `wYCoord`/`wXCoord` at 255 for one agent tick while
`wCurMap` is still the old map, until `CheckMapConnections` runs. `MetaTileMap::new` clamps the
coordinate to keep `meta_tiles` indexing in range — and that clamp is
`(255 + north_extra).min(height - 1)`, which is the **opposite edge of the map**. From the wrong end
of Route 2 the BFS reaches nothing, `connection_action` answers `None`, and the walk is abandoned.
⚠️ The clamp's own comment already said "during map transitions wXCoord/wYCoord can briefly hold
values outside the new map's bounds"; what it could not do was say that the number it then produced
was a fiction.

✅ **Fixed**: `MetaTileMap::position_settled` says when the coordinate is a transition rather than a
position, and `OverworldMovement`'s **`NoRoute` arm** holds on such a tick instead of giving up,
pressing and releasing nothing; the map-change arm reports the arrival on the next tick, which is
what should always have happened.

⚠️ **The placement is the care in this fix, and the first version got it wrong.** Gating the whole
tick on `position_settled` is the obvious shape and it is what this was first written as. It works,
and it broke `full_playthrough` at 69% — because that test is a golden RNG replay and *any* tick that
presses a different button re-rolls every route after it. Narrowing the hold to the one arm that told
the lie changes behaviour only on the ticks that were already wrong. ⚠️ Southward and eastward crossings go one row *past* the map onto the
connection strip, which is a real reachable tile, so they stay settled; a check that merely looked
for an odd-looking coordinate would have broken every one of them.
`mechanics::a_coordinate_that_underflows_a_map_edge_is_not_a_position` pins both halves.

⚠️ **This is almost certainly the deployed defect of 2026-09-02 too.** That run read "there is no
route to the warp to Route8Gate" while standing two tiles from that warp and went looking for a
pathfinder bug; `DidNotArrive` was split out of `NoRoute` to stop one false version of that sentence
and this was the other one.

### 5.2.4 ⭐ What fixing §5.2.3 flushed out, and where it stopped

⚠️ **Recorded because the *method* is the transferable part.** The first version of §5.2.3's fix
gated the whole tick on `position_settled`. That is correct and it broke `full_playthrough` at 69% —
a golden RNG replay re-rolls every route after any tick that presses a different button. Narrowing
the hold to the one arm that told the lie restored it. But the stall the broad version landed on was
real, and it was worth chasing rather than waving away as divergence:

- it reproduces **deterministically** — same step (365/522), same maps, same coordinates, every run —
  so restoring the broad gate is a harness for diagnosing it;
- instrumenting that harness printed the world graph at the moment the route flips:
  Route 14 is split by ledges into sections the graph holds separately, and Route 13's exit to it
  resolves to a **9-edge** section at (19, 8) while walking it actually lands in a **1-edge** pocket
  at (19, 6) whose only exit is back to Route 13. The planner scores the door 7 hops from the
  Fuchsia Centre, takes it, arrives somewhere else, and re-plans identically. Thirty-three crossings
  and counting when the stall detector fired.

Two defects, and only one of them is fatal:

- ✅ **The missing bound is fixed.** The heal detour's `heal_route_stuck` counts polls where routing
  answered *nothing* and is reset by every hop that routes, so a detour that routes perfectly and
  never arrives resets it for ever. `MAX_HEAL_HOPS` is the bound every sibling routing step already
  had, and the code's own comment says so. Proved on the harness: the run that wedged for ever now
  gives up at 60 hops, latches, and **finishes the playthrough**. ⚠️ This is a production matter, not
  a test one: a wedged scripted run is silent, and `--policy deterministic` is what is deployed.
- ⛔ **The landing mismatch is not fixed**, and it is its own work item. `bfs_nodes` resolves an
  edge's *geometric* `to_position` to the nearest observed node, and here the right and wrong
  sections are **two tiles apart** — no distance threshold can separate them. The graph has to learn
  the landing a door actually deposits the player at, which is a change to routing that needs the
  leg chain re-verified behind it.

⚠️ **What is not built**: §5.4's branch-point snapshots, §5.3's ROM cross-check, and the fixpoint
actually being reached. The walk stops on a budget with the frontier open, which §9's last risk says
is the right way to fail — *cap the passes and report a non-empty frontier as a result rather than a
hang* — but it means "every reachable action" is not yet proven, only "~355 of them, and counting".

### 5.2.5 ⭐ The exhaustive sweep, and what each wall turned out to be

⚠️ **The walk starts from `postgame-phase0.bin`, not Pallet Town**, which is what §5.1 asked for all
along ("from C2's finished save") and took three sweeps to arrive at. Measured progression, each
number a wall coming down rather than a bigger budget:

| Sweep | Maps | Ids | What was actually limiting it |
|---|---|---|---|
| Pallet Town, no bag | 28 | 352 | — |
| Pallet Town, key items | 33 | 479 | ⭐ **the bag**: no rod means no `Fish` row *anywhere*, and no Bicycle/Silph Scope/Card Key/Lift Key/Secret Key/S.S. Ticket means whole regions are shut |
| finished save | 41 | 582 | ⭐ **Brock**: Pewter's east exit and Brock's own guide read the *event flag* for having beaten him, which `debug_set_badges` does not write, so everything east of Pewter was unreachable |
| + capped exit count | 21 | 331 | ⛔ **a regression of mine** — see the ⚠️ on the frontier's sort key |

⚠️ **Coverage is not a budget problem and it never was.** Every sweep so far ended `settled: true`
with hours of its budget unspent: the 24-game-hour walk stopped after 960 barren turns having used
2.6 of them. What binds is `patience` and the frontier heuristic, so `GB_COVERAGE_PATIENCE` is now
its own knob.

⚠️ **`debug_set_badges` opens badge gates and not event gates, and the difference is a third of the
map.** Route 23's guards check the badge byte and let a cheated walk through; Pewter's Youngster and
Brock's guide check `EVENT_BEAT_BROCK`. §1.2 rules out writing the flag — a save with flags set
behind its scripts makes every stall found in it a false positive — so the answer is to start from a
save the cartridge itself finished.

### 5.2.6 What the sweeps have found, and what was done about it

The sweeps of 2026-09-07/08 ran the walk nine times, fixing what each one surfaced and re-running.
**Nine faults, eight fixed**; the open two are §5.2.7. Every fix is verified against the default
tier, the leg chain and `full_playthrough` — that last one matters, because it is a golden RNG
replay and any behaviour change on any tick re-rolls every route after it.

| # | What it was | Root cause, and the test that pins it |
|---|---|---|
| 1 | The walk livelocked at **one action a minute** for 2½ hours on VictoryRoad3F | A `BoulderGoal`'s **id** carried the boulder and the square the walk started from, both of which move on every push — so one puzzle minted a fresh id per shove and the frontier never saw the same row twice. The target is the key now. `tile::a_boulder_goal_is_the_target_and_not_the_boulder` |
| 2 | *"given up after 60 s without getting there"*, **one square from the push tile** | Same root one layer down: `OverworldMovement` re-derives its row each tick with `==`, which stopped matching when `actions()` re-picked the nearest capable boulder. `MetaTile::is_same_row_as` compares the target |
| 3 | `ViridianGym:14,15:Fish` — a cast **inside a gym** | `is_water_tile_id` accepted the two *overworld* shore ids (`$32`, `$48`) in every tileset on `WaterTilesets`, and GYM is on that list because Cerulean's gym has a pool. The exclusion list had already been extended twice (SHIP_PORT, FOREST); it is a whitelist now. `map_metadata::a_shore_tile_id_is_only_a_shore_in_the_overworld` |
| 4 | Every `Fish` row `Silent` (14–16 a sweep) | Fishing was the **only overworld action in the game reporting no outcome at all** — `Idle` in silence on a miss, replaced by the battle on a bite. A miss now completes and says whether anything bit; a bite aborts with `Battle`, which `resume_after_battle` picks up. `postgame::fishing::the_action_menu_offers_a_cast_when_a_rod_is_in_the_bag` |
| 5 | `VictoryRoad3F:3,5` abandoned **three pushes from the end** | `MAX_PUSHES` was 24 on a floor that needs **27** (measured). A total cap cannot tell a hard puzzle from a stuck one, so the bound is `MAX_PUSHES_WITHOUT_PROGRESS` — shoves since the plan last got shorter. ⚠️ It shared `DidNotArrive` with the walk's 60-second bound, whose prose describes only the walk, so a puzzle out of shoves reported a failed *walk* — **that message cost two wrong investigations**; it has its own `PuzzleRanLong` now. `endgame::victory_roads_hardest_switch_is_one_decision_however_many_shoves_it_takes` |
| 6 | The `diagnostics` build was broken | Two `SolveBoulders` initializers missed when the `boulder` field was added. Invisible because that feature is not in the default tier |
| 7 | A wedged Strength floor blamed the pathfinder | *"there is no route to the boulder at (23, 16)"* is a claim the agent could not **walk** somewhere, said to a model that has just walked across that floor — the shape of sentence a deployed run filed five bug reports off. `PuzzleUnsolvable` names the real answer: leaving the floor and coming back resets every boulder. `endgame::a_wedged_strength_floor_is_reported_as_a_reset_rather_than_a_missing_route` |
| 8 | Solved puzzles scored `Silent` | **Two causes stacked.** A boulder landing on a switch runs the barrier script, so the tick a goal completes is a tick in `GameMode::Script` — which the driver treated as an interruption. And the event was discarded even when it fired: the arm pushed to `new_events` and then `return`ed, and an early return jumps over the drain at the bottom of `tick` (the ⚠️ on the black-out warp is about this exact mistake). **Every boulder-goal completion the feature ever emitted was thrown away**, so a model asked for a puzzle, the agent solved it, and the model was told nothing |

⚰️ **Two entries here were wrong before they were right, and both are worth remembering.** The
original `PushBoulder* × 7 Silent` row blamed "the shove runs as a script that takes the driver's
state away", which described a per-shove row that no longer existed; the truth was #1, then #8. And
the first attempt at the Hall of Fame (§5.2.7 W1) *filtered the rows out of the menu*, which could
not stop the walk arriving — it only left the brain with nothing to choose, so the sweep reported
**zero defects** while spending 20 059 of its 20 538 turns at the title screen. A terminus has to be
reported, never made unreachable.

### 5.2.7 Open work items

Two, and they are independent of each other. Both are written up from evidence the sweeps produced;
neither is started.

#### W1 — the agent has no `GameMode` for the title screen

**What happens.** With the god party the walk beat the Elite Four a second time. The Champion's room
offers no door to choose — the cartridge **force-walks** the player into the Hall of Fame — and
pokered then increments `wNumHoFTeams` on the ceremony's first frame, plays the parade, saves, and
**soft-resets to the title screen**. `PokemonAgent` has no state for that, so it went on reading
stale map RAM (`HallOfFame` at (4, 2)) and offering the two exit warps to a player no longer in the
world: **197 turns**, its busiest map, the exits tried 98 and 97 times, each giving up after 60 s of
game time. The saved screenshot is the CONTINUE menu.

**Why it is not already fatal.** `host.rs` watches `wNumHoFTeams`, archives the run and starts a new
one, so in the product nothing downstream meets the reset. Nothing *guarantees* that ordering, and
⚠️ **the watchdog cannot save it**: `GB_STUCK_TIMEOUT_SECS` fires on emulated *silence*, and an agent
walking into a wall and giving up every 60 seconds is not silent.

**Reproduce.** `target/test-artifacts/coverage/defect-HallOfFame-4-7-Warp_state.bin` is dropped by
the walk at the moment the verdict turns; load it with `TestFixture::new` and read `game_mode()` and
`map.map`. (Re-cut it by removing the `reached_the_end` stop in `ExploringBrain` and running the
walk with a large `GB_COVERAGE_MINUTES`.)

**Done looks like.** The agent recognises that the cartridge has reset — the obvious signal is
`wCurMap`/`wIsInBattle` being meaningless while the title screen's own state is live — and stops
offering overworld rows rather than acting on stale RAM. ⚠️ **Do not simply special-case
`Map::HallOfFame`**: the room is legitimate to stand in, and the fault is the *reset*, which is a
whole-cartridge event that a map check cannot see. Whatever is added needs a test that a bare
`PokemonAgent` (no `host.rs`) does not spend a budget at the title screen.

**The harness already stops there**, and that is deliberate rather than a workaround: reaching the
Hall of Fame ends the game, so `ExploringBrain::reached_the_end` makes the walk report a terminus and
stop. That is orthogonal to W1 and should stay whatever W1 does.

#### W2 — the walk reaches 38 maps of 248

**What happens.** Every sweep settles in north-west Kanto plus Victory Road and the Indigo Plateau.
Cerulean, Vermilion, Lavender, Celadon, Fuchsia, Saffron and Cinnabar are never entered.

**It is not a routing bug, and that was checked.** Route 4's west block — where Mt Moon's 1F door
lets out — is a genuine dead end: its east side is walled by west-only ledges, and `goto(CeruleanCity)`
fails from there for the scripted policy too. The way east is Mt Moon **B1F**'s far exit, which the
mainline reaches via `enter_at(MtMoonB1F, 23, 3)`. Three of B1F's eight warps are never opened,
because B1F's regions are **entry-dependent**: the walk keeps arriving in the same one, and the other
warps are never in the menu to be chosen.

**Why the obvious fixes do not work.**
- ⚠️ **The world graph cannot steer it.** `WorldGraph` is built incrementally *by traversal* — its
  own doc says routing to a not-yet-visited map is impossible — so it does not know where the
  unexplored maps are.
- ⚠️ **Promise-first ordering was tried and reverted**, twice. Leading with `promise_of` took the
  walk from 41 maps to 21 (§5.2.2), and unconditional priority for an exit into an unseen map is how
  Brock's gym guide held it on `PewterCity:40,18:Connection` for 59 attempts. The current ordering is
  `(times, promise)` with a *two-try* priority for unseen-map doors, which expires precisely so it
  cannot reproduce that loop. It did not move the number.
- ⚠️ **`promise_of` is per-map, but connectivity is per-region.** Route 4 counts as "seen" from its
  west block while 90% of it is unreachable, so the door to its east half scores as leading
  somewhere known.

**Two directions worth trying**, neither started:
1. **Regional sweeps.** Make the start fixture selectable (`GB_COVERAGE_START` over a table of
   `postgame-*.bin`, the `every_committed_fixture_decodes` pattern) and run one walk per region.
   Cheap, needs no new search, and multiplies reach immediately. It does not make any single walk
   better.
2. **Give the walk something to travel *with*.** The postgame party has **Fly**, and Fly is a real
   mechanism the model has too — `use_field_move` with a destination, the way `PC_OPS` already
   issues non-menu field moves from the brain. A walk that can fly to a town it has not explored
   turns a local random walk into something that can cross Kanto. ⚠️ Fly is outdoors-only
   (`Map::is_overworld`) and cannot escape a cave, so it is a complement to the frontier, not a
   replacement.

**Done looks like.** A number, not a feeling: maps reached, reported by the walk already. Anything
that does not move it past 38 has not worked, and the run is cheap enough (~5 minutes of wall clock
for 6 game-hours) to measure rather than argue about.

#### W3 — one undiagnosed row

`PokemonMansion1F:15,3:EscapeRope` scores **defect**: *"there is no route to Escape Rope"* — an item
row offered and then not routable. Seen on several sweeps, never investigated. Small, and the
reproduce-from-the-dropped-state recipe in W1 applies.

### 5.3 The ROM tables, demoted to a cross-check

`read_warp_events`, `header.connections()` and `map.sprites()` do not define the universe (§0.1), but
a warp in a map's header that **never once appeared as a row** is worth printing. It is either
correctly gated, or a row the model is never offered — and the second is invisible today. A report,
not an assertion.

### 5.4 Branch points

Content that is exclusive per save is covered by snapshot × N, exactly as proposed: one save state,
three tests. The branches worth the fixtures:

- the starter (3)
- the fossil (2)
- Hitmonlee / Hitmonchan (2)
- the Bike Voucher path versus buying (2)
- the in-game trades, each of which consumes a party member

⚠️ **Exploration is destructive and mostly one-shot.** A sprite talked to is often gone, an item
picked up is gone, a trainer beaten does not rebattle. So an id is visited once and the verdict is
final; there is no re-running a single id without re-running the run. Snapshot before each branch
point so a branch costs a resume rather than a replay.

⚠️ This is the one place event-flag writes may be justified, and none is admitted yet. If a branch
turns out to be unreachable by resume, argue the specific flag on the specific work item.

### 5.5 Acceptance

Fails on any `defect` verdict, naming the id and dropping a save state. Writes the full table to
`target/test-artifacts/coverage/` either way — every id, its kind, its verdict, and for a `blocked`
the message the game printed. ⚠️ **A second run does *not* produce the same table, and the plan was wrong to expect it to.**
The claim here was "if it does not, the exploration is not deterministic and that is a bug in the
brain" — but the brain is a pure function of the strings it is sent, and what is not deterministic is
underneath it: §2.3 chose `step_coarse` on purpose, so the agent is handed however long the last loop
iteration took, and there is a worker thread and a real socket in that loop. Four runs from
`pallet-town-state.bin` gave 352, 353, 355 and 360 ids, always 28 maps, and two of them failed on a
defect the other two did not see. So the totals are an
observation with a couple of per cent of noise on them, and *which* id fails is not reproducible by
re-running: that is what the save state dropped at the moment of the defect is for.

### 5.6 What "all nodes" costs

Unknown, and it should not be guessed at. C3's first deliverable is the same kind of number as
C2's: ids discovered per minute, and the shape of the curve. The budget follows the measurement.

---

## 6. C4 — the battle matrix

Through the same harness, with the script calling `battle.ask()` so the brain gets the turn.

The cells, and the ones that matter are the refusals:

- **Wild** — fight, switch, use item, ball (success and failure), run (success and failure), run
  from a Pokémon that cannot flee.
- **Trainer** — fight, switch, item, **run refused**, **ball refused**.
- **Safari** — ball, bait, rock, run, and the step counter running out mid-battle.
- **Old man's tutorial** — the catching demo, where the player has no input at all.
- **Gym leader / Elite Four** — `take_over` mid-battle, and the report of the turns the script took
  before it handed over.
- Cross-cutting: switching to a fainted member, using an item with none left, a move at 0 PP, Struggle,
  a party wipe (the black-out window), and the level-up / evolution / move-learn prompts.

Much of this exists in `mechanics.rs`; the work is the audit against this list and the promotion of
each case to run through the LLM path rather than the scripted one.

⚠️ **Start with the audit and commit it before writing a line of test.** The list above is written
from the game's rules rather than from the code, so some cells are already covered, some are covered
under `DeterministicPolicy` only, and some do not exist. A table of cell → existing test (or nothing)
is the deliverable of the first day, and it is what says how much C4 actually is.

### 6.0 ✅ The audit, taken 2026-09-07

Three columns of answer: **LLM** — the cell is exercised through `LlmPolicy` and the worker;
**scripted** — it happens, but under `DeterministicPolicy` or against a hand-built state, so nothing
about the tool surface or the turn loop is under test; **✗** — nothing anywhere.

| Cell | Where | State |
|---|---|---|
| **Wild** fight | `llm_policy::a_scripted_battle_is_fought_without_a_single_request`, `llm::what_the_enemy_did_is_reported_rather_than_only_what_we_did` | **LLM** |
| Wild switch | `battle_script::switching_parses_under_the_name_the_keyword_forced` (sandbox), `stalls::a_fainted_pokemon_chosen_in_battle_does_not_trap_the_party_menu` | scripted |
| Wild item | `postgame::items::can_use_the_stat_items_and_a_poke_doll_in_battle`, `stalls::a_key_item_used_in_battle_does_not_trap_the_bag` | scripted |
| Wild ball, success | `postgame::fishing::can_catch_a_magikarp_on_the_old_rod`, `postgame::legendaries::can_catch_{moltres,zapdos,mewtwo}` | scripted |
| Wild ball, **failure** | — | **✗** |
| Wild run, success | `postgame::fishing`, `mechanics` (`BattleAction::Run`) | scripted |
| Wild run, **failure** | — | **✗** |
| Wild run from something that cannot flee | — | **✗** |
| **Trainer** fight / switch / item | as wild | scripted |
| Trainer **run refused** | `battle_script::running_from_a_trainer_is_refused_with_the_reason` — the sandbox refusing, not the agent | scripted |
| Trainer **ball refused** | — | **✗** |
| **Safari** ball / bait / rock / run | `postgame::safari::can_catch_a_safari_exclusive`, `stalls::a_safari_menu_cursor_left_on_bait_does_not_repeat_itself` | scripted |
| Safari step counter running out **mid-battle** | `runs_the_step_budget_down_and_is_ejected` covers the ejection, not the mid-battle case | **✗** |
| **Old man's tutorial** (`wBattleType == 1`) | nothing anywhere. ⚠️ `read_battle_state` does not even name it — it reads as `BattleType::Wild` | **✗** |
| **`take_over` mid-battle** | `llm_policy::a_battle_the_model_takes_over_is_not_decided_by_the_script_again` | **LLM** |
| The report of what the script did first | `llm_policy::{what_the_script_did_reaches_the_model_on_the_next_turn, an_ask_says_what_the_script_did_since_the_model_last_chose}` | **LLM** (rig, not a real gym) |
| Switching to a fainted member | `stalls::a_fainted_pokemon_chosen_in_battle_does_not_trap_the_party_menu` | scripted |
| An item with none left | — | **✗** |
| A move at 0 PP | `stalls::a_move_with_no_pp_left_does_not_trap_the_battle` | scripted |
| Struggle | `stalls::a_party_with_no_pp_anywhere_still_gets_an_answer` | scripted |
| A party wipe / the black-out window | `mechanics::a_blackout_is_not_a_decision_point_until_the_warp_has_landed` | scripted |
| The move-learn prompt | `llm_policy::{a_forget_prompt_pre_empts_the_battle_turn_it_interrupts, a_forget_slot_the_pokemon_does_not_have_declines_instead_of_hanging}` | **LLM** |
| Level-up and evolution prompts | `saffron`, `postgame::trades` | scripted |

⭐ **What the audit says about the size of C4.** Six cells do not exist anywhere — every one of them
a **refusal** (a ball that fails, a run that fails, a ball at a trainer, an item with none left, the
Safari counter expiring mid-battle, the old man's tutorial), which is exactly what §6 predicted:
*"the cells that matter are the refusals"*. The rest is not missing, it is in the wrong column:
sixteen of the twenty-three are proved under `DeterministicPolicy`, so what a *model* is offered in
each of them — the id, the row, whether the tool refuses it — is untested. So C4 is one small piece
of new coverage and one larger promotion, and the promotion is the part with the findings in it.

### 6.1 Acceptance

Every cell is either a passing test through the LLM path or a row in the committed table naming why
not. Default tier where the cell is cheap; `slow-tests` where it needs a fixture walk.

---

## 7. The soak tier

**Not dropped in this plan, and not kept for ever either.** The honest position:

`RandomPolicy` and an exploring brain draw from the same vocabulary — `MetaTileMap::actions()` — so
the explorer strictly dominates on *coverage of that vocabulary*, and both bugs the soak actually
found (the PC menus, grass with nothing in it) are vocabulary bugs the explorer would also find. What
the explorer does not do is soak's **state-space randomness**: a jam that needs a particular action
in a particular state (poisoned, full bag, SHIFT battle style, a text box half-drawn) is an
interaction a once-through walk will miss.

So: keep the soak at a reduced budget while C3 lands, run both, and drop it when it has stopped
finding anything the explorer did not. Concretely — halve `soak::SOAK_GAME_TIME` (40 min of game time
per state today) rather than cutting states from `STATES`, because the tier's value is in the
starting points and its cost is in the minutes; the module's own ⚠️ on breadth-over-depth is the
argument, and it applies to itself. This repo measures the other decisions it makes — 565 → 21
kbit/s, one cancellation in 2430, deflate at +16.6% on Opus — and this one should be measured too,
not argued.

---

## 8. Order of work

1. **C0** — harness, brains, faults, `LlmRun` with a run directory and a restart. Existing LLM tests
   move onto it. *Default tier.*
2. **C1** — the new `debug_*` primitives and the sidecar. The play-path guard keeps passing.
3. **C2** — the god run. **Measure the turn cost first**; the decision to retire `full_playthrough`
   depends on the number.
4. **C3** — `CoverageLog` wired into the existing drivers (free, and it sizes the rest), then the
   frontier brain and the verdict oracle.
5. **C4** — the battle matrix audit and promotion.

---

## 9. Risks

| Risk | Mitigation |
|---|---|
| The god run is slower than `full_playthrough`, so the pre-push gate gets worse | Measure at C2 before retiring anything. If it is slow, it goes behind its own feature and `full_playthrough` stays |
| A cheat produces a save the cartridge could not have produced, and every finding from it is a false positive | §1.2 — play the story, cheat only the battles. No wholesale flag writes without a named argument |
| A party write lands mid-battle and desynchronises `wBattleMon` | §3.2 — gate the top-up on `!in_battle` and on the black-out window being closed |
| The scripted brain quietly reaches around the menu to `GameState` and the prompt-adequacy property is lost | §2.1 — `Brain` is handed strings by type. There is nothing to reach around to |
| Exploration is destructive, so a defect found on id *n* cannot be re-run in isolation | Snapshot per map entry as well as per branch point; the failure artifact is a save state, as the rest of the suite already does |
| C3 never terminates because the fixpoint keeps discovering rows | Cap the passes and report a non-empty frontier as a result rather than a hang |

---

## 10. What this does not prove

⚠️ The same caveat the README already carries, restated because this plan makes it easier to forget:
**a brain is not a model.** Everything here proves the plumbing can finish the game and that every
action in it is executable. It says nothing about whether any particular model will choose well. The
prompt-adequacy property in §2.1 is the closest this gets, and it is a lower bound: it proves the
information is *present*, not that it is *findable* by something that is not looking for it.

---

## 11. For the agent picking this up

### 11.1 Read these first, in this order

`CLAUDE.md` has the rules of the road and points at the rest. Beyond it, and before writing anything:

| Doc | Why, for this plan |
|---|---|
| [test-suite](test-suite.md) | Every tier, what each costs, the fixture chain, and ⚠️ the paragraph saying no test in the suite can see the agent's tick *rate* — §2.3 depends on it |
| [llm-turn-loop](llm-turn-loop.md) | The append-only history, the prompt cache, the plan message, compaction, the park. C0 and §2.2.1 are all here |
| [pokemon-agent](pokemon-agent.md) | The agent loop, the watchdog, the closed loops under A. C3's oracle reads these events |
| [deployed-run-defects](deployed-run-defects.md) | How to argue from a save state rather than a guess. The house standard of evidence for anything this plan finds |

Then read, in the code: `integration_tests/llm.rs` (the whole file — it is the thing C0 generalises),
`postgame/debug.rs`'s module comment (the play-path/debug-tier line), `integration_tests/soak.rs`'s
module comment (why breadth beats depth, measured), and `agent.rs`'s `AgentEvent` /
`OverworldActionAbortedReason`.

### 11.2 Where the code goes

| | |
|---|---|
| `src/pokemon/integration_tests/llm_harness.rs` | C0: `MockEndpoint`, `Brain`, `TurnRequest`, `Reply`, `Fault`, `LlmRun` |
| `src/pokemon/integration_tests/llm.rs` | C0: existing tests move onto the harness; the fault tests join them |
| `src/pokemon/postgame/debug.rs` | C1: the new `debug_*` primitives. Nothing else in the crate gains one |
| `src/pokemon/integration_tests/cheats.rs` | C1: the `Cheats` sidecar |
| `src/pokemon/integration_tests/godmode.rs` | C2: `ScriptedBrain`, `Intent`, the god run |
| `src/pokemon/integration_tests/coverage.rs` | C3: `CoverageLog`, `ExploringBrain`, the verdict oracle |
| `src/pokemon/data/*.bin` | C3: the branch-point snapshots. ⚠️ Writing one is a no-op without `--features regen-fixtures` |

### 11.3 The tiers

Two new features, each with a doc comment in `Cargo.toml` in the style of the eight already there
(what it holds, why it is separate, and the command that runs it):

```toml
# C2 — a fresh save played to the Hall of Fame through `LlmPolicy` against an in-process mock
# endpoint, with battles won by a god party and decided by a script. The pre-push gate
# `full_playthrough` used to be, and the only test that drives the deployed stack for a whole game.
#   cargo test --release --features godmode --bin gb -- godmode_playthrough --nocapture
godmode = []
# C3 — every reachable action in the finished game, taken once, with a verdict on each.
#   cargo test --release --features coverage-tests --bin gb -- coverage --nocapture
coverage-tests = []
```

⚠️ **A tier goes behind a feature, never behind `#[ignore]`.** The ignored list is a backlog of
*blocked* tests and is asserted to be exactly 18; adding to it breaks that assertion and, worse,
turns a slow test into an invisible one. C0's tests and C1's are default-tier and must stay under the
default tier's budget.

### 11.4 Running things

```shell
cargo test --release                                              # C0 and C1 live here
cargo test --release --features godmode --bin gb -- godmode_playthrough --nocapture
cargo test --release --features coverage-tests --bin gb -- coverage --nocapture

# Still the gate for the scripted route, and it does not go away:
cargo test --release --features hall-of-fame --bin gb -- hall_of_fame
```

Always `--release`; the crate is `--bin gb`, never `--lib`; agent and policy debugging goes to
stdout, so `--nocapture` when you care. Failure artifacts go to `target/test-artifacts/`, which is
where the rest of the suite puts a stall's save state and screenshot — follow it, and name the
subdirectory after the phase.

### 11.5 Things that will bite

- ⚠️ **`full_playthrough` is a golden RNG replay.** Anything that changes frame timing re-rolls the
  RNG for every route after it, so a *correct* change fails it hundreds of steps away from the cause.
  A/B against HEAD before debugging the stall site. The god run will have the same property.
- ⚠️ **A rig helper that reads published events after a `pump_*` must use `events_until`, not
  `try_iter`.** The `llm_policy` battle-script rig raced on exactly this; it was never machine load.
- ⚠️ **No em dashes in the strings the *agent* generates** — `AgentEvent`'s `Display`, `MetaTile`'s,
  a `Notice`, `learnset::teach_refusal`. The rule is deliberately narrow: prompts, tool descriptions
  and action-menu rows use them by design. Anything C3 adds to an event's prose is in scope.
- ⚠️ **A new invariant goes first into a comment on the code it constrains**, then one line in the
  doc for its area pointing at it. This file is a plan and stops being the place once a thing is
  built: rules from C0/C2 belong in [llm-turn-loop](llm-turn-loop.md), from C1/C3 in
  [pokemon-agent](pokemon-agent.md), and every tier here belongs in [test-suite](test-suite.md).
- ⚠️ **Update the status line at the top of this file as phases land**, and record where the plan was
  wrong rather than editing the mistake out. `llm-web-playthrough-plan.md` does that and it is the
  more useful document for it.

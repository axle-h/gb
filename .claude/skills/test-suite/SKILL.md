---
name: test-suite
description: "Every test command in this repo and what each tier costs, the soak/stalls jam hunt, why full_playthrough is not optional, the committed fixture chain and how to regenerate it, benchmarking on this machine and the blip golden vectors. Load before running anything but `cargo test --release`, before regenerating a fixture, and before adding a test."
---

# The test suite: tiers, jam hunting and fixtures

*Extracted from `CLAUDE.md`, which holds the rules of the road and the index of these skills. The
README is imported into `CLAUDE.md` and is not repeated there or here: this file has only the
invariants and the traps, nearly every one of which was learned by breaking something.*

## Tests

`src/pokemon/integration_tests/` is tiered by how much **game time** a test emulates, which is what it costs. The core
runs at **~91× realtime** on Pokémon Red and the agent costs **~35%** on top, giving **~50×** end to end (2026-08-06,
Ryzen 9 7900X, `bench_core_throughput` and `bench_emulation_throughput`), so wall clock ≈ emulated-minutes ÷ 48. Those
are post-Phase-C numbers — the core was 29× and the agent-inclusive figure 24× before it, a 3.1× speedup — and the
agent's share grew from ~16% to ~35% only because the emulator under it got faster, so **the agent is now worth
profiling and it was not before.**

```bash
# Default tier: unit tests + agent mechanics + stalls + two navigation smoke tests + web/host/llm. ~20 s, ~1345 tests.
cargo test --release

# Leg chain: one test per PolicyStep::*_steps() leg, seeded from a committed snapshot. ~167 tests, ~250 s.
cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests

# The Safari dex sweep: ~171 s for ~190 min of game time, more than the whole leg chain combined, which is why it is
# split out. ⚠️ `very-slow-tests` does not imply `slow-tests` and the module is behind that gate — pass both.
cargo test --release --features slow-tests,very-slow-tests --bin gb -- can_sweep_the_safari_zone

# The whole game to 8 badges from a fresh save, ~6 min. (The full run to the credits is ~50; see below.)
cargo test --release --features full-playthrough full_playthrough

# The stall hunt: 40 min of game time under RandomPolicy from each of 13 starting states, in parallel, ~60 s.
cargo test --release --features soak-tests --bin gb -- soak --nocapture

# A single test with output (file module included in the path).
cargo test --release --bin gb -- pokemon::integration_tests::mechanics::test_debouncing --exact --nocapture

# The PPU comparisons: dmg-acid2, cgb-acid2, Pokémon Red in colour.
cargo test --release --bin gb -- game_boy::tests::ppu

# ⚠️ The two probes worth running by hand. probe_map_images writes the real PNGs `read_map` sends, into
# target/map-renders/ — the only way to know a render is *right* rather than merely non-blank, so look before touching
# the palette, the labels or the tile lookup. probe_turn_requests writes each decision kind's first request whole into
# target/turn-requests/ (`.json` literal, `.md` with the newlines put back) — the only way to see what the model is
# actually sent, and what found `BattleAction`'s `{:?}` switch rows, ~500 bytes of Rust syntax per party member in the
# menu of every battle turn.
cargo test --release --features diagnostics --bin gb -- \
  llm::map_image::tests::probe_map_images --exact --ignored --nocapture
cargo test --release --features diagnostics --bin gb -- \
  llm::prompt::tests::probe_turn_requests --exact --ignored --nocapture

# ⚠️ Two more that answer a question rather than draw a picture, and both are ROM-only and instant.
# probe_grind_sites ranks every encounter block in the game by experience per knockout and per step, with the
# encounter rate, the level band and the Poison-type share beside each — which is what a `GrindUntilLevel` should be
# pointed at, and the only honest way to argue about a grind site. Read exp/KO first and exp/step second: an
# encounter cycle is ~40 s of cartridge time and under 7 s of that is the walk (1552 battles in 1229 s).
# probe_stall_actions prints a save's map, money, party (levels, HP, status), **bag** and every reachable action.
# It defaults to the last `target/test-artifacts/test_stall_state.bin` and takes any state through GB_PROBE_STATE,
# which is what makes it the first thing to reach for on a stalled leg: it tells "the route is wrong" from "there
# is no route". It proved Cerulean Cave's door is not in the reachable set at all, and it found a run standing one
# tile from the Silph Co Card Key with a bag on exactly 20 entries — ⚠️ **Gen 1's cap, and a full bag refuses every
# pickup in the game silently**, which is why the bag line is printed.
cargo test --release --features diagnostics --bin gb -- \
  pokemon::wild::tests::probe_grind_sites --exact --ignored --nocapture
cargo test --release --features diagnostics --bin gb -- probe_stall_actions --ignored --nocapture
GB_PROBE_STATE=src/pokemon/data/post-articuno.bin \
  cargo test --release --features diagnostics --bin gb -- probe_stall_actions --ignored --nocapture

# All the diagnostics and probes.
cargo test --release --features diagnostics,slow-tests --bin gb -- probe_ --ignored --nocapture

# Throughput: the agent (emulator + agent.step), then the core alone. `--exact` needs the full module path.
cargo test --release --bin gb -- \
  pokemon::integration_tests::fixture::bench_emulation_throughput --exact --ignored --nocapture
cargo test --release --features bench --bin gb -- game_boy::tests::bench_core_throughput --exact --nocapture

# What each stream costs and every alternative it was chosen over (~25 s for video).
# ⚠️ The kbit/s in the README and in the `web-streams` skill come from these two and nowhere else.
cargo test --release --features bench --bin gb -- video::bench --nocapture
cargo test --release --features bench --bin gb -- web::audio::bench --nocapture
```

**A test that is `#[ignore]`d should be blocked, not merely slow or not-a-test.** Everything else goes behind a Cargo
feature, so the ignored list stays a readable backlog:

| Feature | Holds |
|---|---|
| `slow-tests` / `very-slow-tests` / `full-playthrough` | Tiering by emulated game time |
| `diagnostics` | `probe_*`, `dump_fixture_states`, `capture_golden_input` — tools that print a report rather than assert. They keep `#[ignore]` on top of the gate because their pass/fail is not a signal: two legitimately end by exhausting their cycle budget *after* printing what was asked for |
| `bench` | `bench_core_throughput`, `bench_emulation_throughput`, `web::video::bench` (which also pulls in `flate2`) |
| `soak-tests` | `integration_tests::soak` — the fuzzer. Gated as a **module**, not with `#[ignore]`, so it never appears in the ignored list |

With every tier feature on and the tool features off, the ignored list is **18 blocked emulator tests** and nothing else
— 9 `oam_bug` and 9 `mem_timing`/`halt_bug`. Each names its blocker: a plan task ID, or why it will not be fixed. Keep it
that way. (The mooneye MBC suite is three tests and 0.7 s, so it runs by default; its ROMs are `cfg(test)` rather than
feature-gated, which keeps them out of the shipped binary — the only thing a feature was buying.) Failure artifacts — a
save state and a screenshot at the point of a stall or timeout — land in `target/test-artifacts/`, not the repo root.

One deliberate exception to the tiering: `the_hall_of_fame_is_announced_once_when_the_ceremony_starts` runs 1.7 s of real
ROM in the default tier, because it buys the only proof that the end of the game is detected at all.

### Turns the game takes back: `integration_tests::interruption`

`LlmPolicy` keys a turn by the decision kind it answers and cancels it when the agent asks something else — the shape
everyone expects is an overworld turn interrupted by a battle. ⚠️ **Measured, it does not happen.** The deployed run's
2428 turns contain **one** `turn_cancelled` for "the game moved on", and it is turn 1, the `POST /api/new-run` reset
bumping the generation. The structural reason is that **the agent presses nothing while a turn is in flight**:
`AwaitingOverworldAction` and `BattleState::AwaitingPolicy` tick a delay and poll, so the game sits at a static menu or a
stationary tile and cannot move on its own. Across the same run's 2430 `turn_started → decision` windows exactly **one**
agent event fired inside a window. A wild encounter or a trainer's line of sight fires *during a walk*, and no policy poll
happens during a walk — so the battle is the next turn rather than an interruption of the one in flight.

⚠️ **The 68 other `turn_cancelled` events in that run are a different thing wearing the same name**: `Worker::give_up`
publishes one when the model replies twice with no tool call, and those turns still return a forced `wait`. Counting
`turn_cancelled` without splitting on `reason` reports the rate as 2.8% when it is 0.04%.

What is kept is the guard, not the search. `SlowPolicy` wraps any ordinary policy and holds each answer back for a number
of agent ticks. ⚠️ **It has to key turns exactly the way `LlmPolicy` does**, `pick_field_move`'s exemption included, or it
reports a cancellation every time the agent asks anything at all.

- `leaving_oaks_lab_does_not_strand_a_turn_in_the_rivals_script` (default tier, ~2 s) is the case worth guarding: the
  rival's challenge is the longest scripted freeze the early game has, and the agent asks **nothing** for the ~900 ticks
  (18 s of game time) between the abort and the battle menu. Three latencies — instant, 5 s, 60 s — because the script
  fires on a *tile* and the answer lands on a *clock*. ⚠️ **The precondition is half the test**: a run that stops before
  the battle question is ever put proves nothing, which is how the first version passed at every latency below 600 ticks.
- `the_detector_notices_a_question_being_replaced` exists because both results are negative ones. Nothing in the game
  replaces a question mid-flight, so it is done by hand.

⚠️ **A broad sweep was written and deliberately not kept**: random play from six fixtures, 10 minutes of game time each
with a jittered latency, 346 turns and 0 abandoned. It cost 100 s of the `soak-tests` tier to re-derive a number the
structure already explains. If the rate is ever in doubt again, it is twenty lines around `SlowPolicy`.

### Finding jams: `soak`, and `stalls` beside it

⚠️ **`full_playthrough` proves one route still works; it cannot find a jam off that route.** The scripted policy never
chooses to walk into a PC, or into grass with nothing in it, or to pick a move the game will refuse — so none of those
were reachable by any test in the suite, and all of them wedged the deployed run instead. `soak` is the answer: hours of
`RandomPolicy`, which explores the agent's state machine far more widely than any route. It watches
`PokemonAgent::since_last_policy_poll` — the **same** value the watchdog reads — so it fails exactly when a deployed
`LlmPolicy` would have its watchdog fire. One definition of stuck.

⚠️ **Breadth beats depth here.** A random walker does not explore, it *diffuses*: it picks uniformly from the tiles the
current map offers, so hours from one starting point re-cross the same few maps. The budget buys **starting points**
rather than depth — one test per entry in `STATES`, 40 minutes each, over thirteen committed fixtures, in parallel for
less wall clock than the single five-hour test it replaced. ⚠️ **A state earns its place by what it makes *reachable***,
not by progression: a bicycle, a Safari step counter, a boulder, a PC with something in it, a bag with a TM in it. A
fresh save's bag is empty, which is why no amount of play from it can reach an item the game refuses in battle — and why
that jam survived five hours a day of fuzzing. More badges than its neighbour buys nothing.

⚠️ **The cartridge's own options are forced** — `InitOptions`' `TEXT_DELAY_MEDIUM`, battle animations *on*, battle style
SHIFT — rather than `TestFixture`'s `FAST_FIXTURE_OPTIONS`. `gb serve` runs on those and the soak exists to reproduce the
deployment, not to be cheap: the no-PP jam was a race with the character-by-character text renderer. It has to *write*
them, not merely leave them alone, because every fixture past `start-of-game-state.bin` was captured mid-leg by
`TestFixture` and carries fast text baked into `wOptions`. ⚠️ **Those options are not book-keeping — whole screens hang
off them.** Battle style SHIFT asks "…Will <PLAYER> change POKéMON?" on every enemy switch, and since every `TestFixture`
overwrites the options with SET, **no other test in the suite ever sees that prompt**. The agent answers *no* (switching
is a decision, made at the menu that follows); A there opens the party menu, which the party arm backs out of, which
brings the prompt round again.

⚠️ **`GB_SOAK_LIMIT_SECS` is how you find the *next* one.** The default is the watchdog's 300 s because that is the number
production cares about, but seed 1's worst healthy stretch across all thirteen states is **62 s** (2026-08-12), so a
near-miss can hide under the default for a long time. Running at 120–150 trips on anything twice as quiet as normal, and
that is how the pacing budget was found: 182 s of silence in Viridian Forest that turned out to be `PACING_BUDGET_TICKS`
running to the end on the rarest grass in the game (8/256), not a jam. Note that 62 s is now *by construction* —
`MAX_MOVEMENT_SILENCE` gives up on a walk at 60 — so the healthy distribution bunches just under it. ⚠️ **But its tail is
much longer, and a limit below ~150 s finds the tail rather than a bug**: Gen 1 skips the player's input while WRAP/BIND
runs, so a paralysed Pokémon in a wrap chain on Route 15's line of trainers measures **124 s** of legitimate silence (seed
837). ⚠️ **A budget that bounds silence is not sized to guarantee success** — giving up just means the policy gets asked
again, and the first version of that constant was three times too generous because it was sized to guarantee an encounter.

⚠️ **It is seeded (`GB_SOAK_SEED`, default 1) and must stay that way.** The first runs each failed somewhere different,
which is worse than useless: a failure that vanishes when you go back to look at it cannot verify its own fix, and CI
would flake. Seed 1 must stay green; vary the seed to hunt, `GB_SOAK_MINUTES` to go deeper from one state.

**Every jam it finds gets promoted to `integration_tests::stalls`**, in the *default* tier: the save state at the moment
the agent went quiet, replayed against a fresh agent, about two seconds each — the difference between a 4½-minute
reproduction and a one-second one. `stalls::probe_stall_artifacts` (`--features diagnostics`, `GB_STALL_DIR=…`) is the
bulk form, which is what a sweep across seeds leaves you holding. ⚠️ **Not every stall survives the trip**, because the
save state holds the emulator and not the agent: a jam the game's own screen re-creates reproduces perfectly, a jam that
lived in the agent's own state (an `OverworldMovement` route) does not. Watch a new case go red before committing it, or
it may be asserting nothing. ⚠️ **Artifacts are named per state *and* per seed** (`soak-<state>-seed<N>.{bin,png}`), or a
sweep has each failure overwrite the last — and the artifact is the whole value of a failure.

**Nearly everything it finds is one shape: a closed loop under A.** A menu or a script the agent's own A press re-enters
with the cursor untouched — the PC menus, a spent move, a key item in battle, the Cerulean badge house, Bill's PC, a
refused field move, a Card Key door, the Safari menu's sticky cursor, the START menu left on the trainer card. Five rules
cover the class, and they are worth knowing before adding another special case:

- ⚠️ **A give-up in a battle hands back *latched into B*** (`BattleState::backing_out`), because a plain `WaitingForMenu`
  opens by pressing A — into whatever menu is still on screen, which is how "give up" came to mean "select whatever is
  under the cursor".
- ⚠️ **The text reader escapes menus, not conversations.** After 30 s in which the agent reaches *no decision point*, and
  only when what is on screen is a list menu, a field-move box, a menu offering CANCEL, or the **START menu**, it presses
  B until a poll happens (which is what clears it — `poll_policy`, on the agent, so a flicker through `Idle` cannot reset
  it). ⚠️ **Not on a yes/no**, where B is an answer, and ⚠️ **not in a battle**, where B cancels the move being chosen and
  gym leaders are routinely quieter than 30 s. Without those two conditions it fires mid-fight and `full_playthrough`
  loses the Brock fight.
- ⚠️ **Silence bounds the drivers, not tick budgets.** A driver that runs its own menus is abandoned after
  `DRIVER_ESCAPE_SILENCE`, and a walk after `MAX_MOVEMENT_SILENCE`, rather than each of the nineteen carrying a counter.
  ⚠️ It has to be *silence*: a tick counter belongs to a state, and a state torn down by an interruption starts it over —
  the Seafoam current takes the player every few seconds and handed the walk a fresh budget each time.
- ⚠️ **A menu the agent did not open is closed, not confirmed** — `MENU_HANDOVER_TICKS`, armed in
  `assert_text_box_state`. That function is the funnel for "start reading a text box" and everything that drives menus on
  purpose is excluded from it by `drives_its_own_menus`, so arriving there with a *menu* on screen means something **left
  one behind**: a driver abandoned by `DRIVER_ESCAPE_SILENCE`, an aborted PC, a `press_buttons` batch. It reuses
  `escaping_menus`, so it is the 30 s rule acting immediately on evidence it can already trust. ⚠️ **It cannot be the
  single transition tick**, which is the version written first and detects nothing: `wFontLoaded` flips a third of a
  second *before* the menu draws itself — on the START menu, geometry is the previous menu's until tick 18 and `EXIT` does
  not reach the tile map until tick 21, against the reader's first A on tick 26. ⚠️ And it must stay a **short window**
  rather than a per-tick test: bounded, it only has to be right about the moment a box opens with no driver behind it;
  unbounded, it has to be right about every screen of every conversation.
- ⚠️ **A rule that runs at every text box may believe only the *screen*, never a lingering id — `MenuEvidence`.**
  `wTextBoxID` is written when a box is drawn and never cleared, so it goes on naming a menu that closed several maps ago.
  The 30 s rule can trust it because 30 s of silence has itself ruled out a conversation; the hand-over rule cannot,
  because it fires on conversations by definition. Getting that the same way round for both cost a nickname: the Silph Co
  lift left `ListMenuBox` behind, the agent talked to the rescued worker a few maps later, and B — an exit on a list, an
  **answer** on a yes/no — declined "Do you want to give a nickname to LAPRAS?".
  `a_full_party_sends_the_silph_lapras_to_the_box` is the only test in the suite that crosses a lift into a yes/no, and is
  now the guard for both things.

⚠️ **Each of those rules is a frame-timing change, so `full_playthrough` is the only thing that can price one.** The ⚠️s
in `agent.rs` name four wider versions that look obviously right and are not: latching the item driver's tick budget
cancels a ball mid-throw; escaping *any* text box after 30 s, or on a count of reopened boxes, blacks the mainline out in
Mt Moon; handing the turn to the policy from every battle-menu position re-times every battle in the game. Same lesson as
`with_original_battle_timing` — the leg chain and `stalls` cannot see any of it.

The rest of what it catches is a driver waiting for something that stopped coming, pressing buttons in silence. Traps in
fixing those:

- ⚠️ **A message box swallows directional input**, so a driver that is right about the next button still has to clear
  what is on top of it first. The forced-switch arm correctly wanted to walk the cursor off a fainted Pokémon and pressed
  Up into "There's no will to fight!", for ever. ⚠️ **And it is not always a message you can name**: `battle_menu_state`
  reads `wTopMenuItemX/Y`, which *linger*, so an ordinary battle line ("It's super effective!") over the party list
  reports as the party list. What tells them apart is the screen — a party list draws an HP bar per member, a message box
  draws the active mon's alone, so `>= 2` slashes means the list is really there. Same heuristic the item driver uses for
  "use on which POKéMON?".
- ⚠️ **A give-up that is not remembered is not a give-up.** `handle_card_key_door` spent 40 A presses on a door, declared
  it a wall and blocked it — then started another forty on the next tick because nothing read `blocked_tiles` back. Every
  press reprints "Darn! It needs a CARD KEY!", which is a text box, which is another A.
- ⚠️ **A counter outside the variant is reset by `set_state`.** `UsingItem` and `WaitingForMenu` rebuild themselves every
  tick with a `press`/toggle field flipped, so `set_state` sees a *new* state and zeroes anything counting from
  `PokemonAgent`. The first bound on each silently never fired. `OverworldMovement` is the one state where the
  agent-level `state_ticks` works, because it does not rebuild itself.
- ⚠️ **The branch that detects a problem is not always the branch that presses the wrong button.** `WaitingForMenu`'s
  `MoveList` arm had handled a spent move with B since an earlier hours-long wedge, and it still wedged — because the
  `screen.contains` check above it returns first while the message is up, and the *text reader* (in the `None` arm) was
  the thing mashing A. A fix has to sit above every branch that can press.

### ⚠️ Why `full_playthrough` is not optional

The leg tests each start from a committed fixture, so they prove the legs *individually*; only `full_playthrough` proves
they still **compose**, and the two come apart in ways nothing else catches:

- **A leg test can be green for a reason the mainline does not give it.** `run_leg` keeps stepping after the queue empties
  until the effect lands, so a leg whose `Interact` pops before its conversation still passes — while
  `complete_game_steps` walks straight on without the item. That is exactly how the Poké Flute broke. `run_leg` prints a
  ⚠️ when its post-exhaustion wait is long; **treat that warning as a failure in waiting.**
- **A fixture pins a party and a bag; the mainline earns them.** A leg seeded with 20 Hyper Potions says nothing about
  whether the run that reaches it can afford them.
- **Anything that changes frame timing re-rolls the RNG stream** (see `with_original_battle_timing`), and only a full run
  crosses every route that stream feeds.

Because it is opt-in and slow, it rotted once already: it sat broken while its own doc comment, the docs and the plan all
claimed it played to all 8 badges. When it fails it reports how far it got (`completed 488/516 policy steps (94%)`) and
drops its artifacts; `playthrough::probe_resume_playthrough` replays from there in seconds instead of re-running the 20
minutes up to the stall. **If you cannot make it pass, say so explicitly in the hand-off — do not leave a doc comment
claiming it works.**

### ⚠️ The chain has a root, and the mainline party is what invalidates it

⚠️ **`at-cerulean.bin` is what every leg fixture descends from, and until 2026-08-30 it was
`post-cascade.bin`, which no test produced.** That is fine right up until the mainline party changes:
swapping the starter meant every downstream leg resolved an HM teach, a grind or a lead against a
party the old root did not have, and a `PartyRef` that does not resolve **waits for ever** rather than
failing. Nine leg tests went red at once. `early_game::regen_at_cerulean_fixture` produces the root now
(Pallet → Brock → Mt Moon → the Cerulean Centre, ~90 s, `regen-fixtures` only), so the chain can be
re-cut from the top.

⚠️ **A fixture is cut where the mainline *stands*, not where it happens to be convenient.**
`cerulean_to_vermilion_steps` opens with `enter(CeruleanCity)`, meaning "walk out of the Centre" — a
root saved standing in the city instead makes that step hunt for a transition to the map it is already
on, and it walked out to Route 4 and stalled trying to reach Route 24 from there.

⚠️ **And it has to be cut where the party is *healed*, which is later than it looks.** `Interact` pops
the instant it issues the walk, so `Interact(NURSE)` used to be a *request* to heal: the first cut of
this root came out carrying **Water Gun on 6 of 25 PP**, and the leg seeded from it lost the Cerulean
rival ambush. The policy now holds the step until the party is at full HP, full PP and unstatused
(`party_is_fresh`, bounded by `MAX_HEAL_WAITS`) — which is a **frame-timing change**, so it re-rolls
every RNG stream in the run and `full_playthrough` is the only thing that can price it.

⚠️ **A leg that contains a grind needs a game-time budget sized to encounters, not to walking.** Two
legs went over on the starter swap — `can_return_to_cerulean` (30 → 240 min, it now catches the Route
11 Drowzee and grinds it to Hypno) and `can_get_rainbow_badge` (45 → 90, eight separate cuts before
Erika is even reached).

⚠️ **The chain had three more roots nothing produced, and each one pinned the old party.** Every one
was found the same way — a leg resolving a `PartyRef` against a party that no longer existed, which
*waits* rather than failing. `at-mansion-blizzard.bin` and `post-volcano-lone.bin` are gone
(`can_catch_articuno` and `can_get_earth_badge` now read `post-volcano-badge.bin` and
`post-articuno.bin`, which is the mainline's own order), and so is `at-saffron-post-silph.bin`: the
Silph leg walks itself back out to a liberated Saffron now, so `can_get_marsh_badge` reads its output.

⚠️ **And it skipped a badge for years.** `can_reach_rocket_hideout` was seeded from the *pre-gym*
`at-celadon.bin` — valid on its own, since the hideout needs Cut rather than the badge, and it was the
seed everything downstream inherited, so **the Rainbow Badge was never in the chain**. Nothing noticed
until Strength moved onto the starter: Strength's field use is gated on that badge, and the Seafoam
leg opened the party menu on a Blastoise that knows Strength and got STATS / SWITCH / CANCEL back.

⚠️ **A fixture's name has to be where it is cut.** `vr1f-strength.bin` was read by two tests for what
its name says — `mechanics::strength_switches_are_exposed` is a pure state read asserting VR1F's one
switch is exposed on it — while the leg that writes it ran the climb as well and left it on VR2F. The
approach and the climb are two tests and two fixtures (`vr1f-strength` and `vr2f-ladder`) now.
⚠️ Same shape for `at-indigo.bin`, whose only other reader is `llm::map_image`'s fixture list — chosen
for what each state makes *drawable*, and its entry is "a `Plateau` map whose strip tileset differs
from its own", meaning the open-air plateau rather than the lobby standing on it.

⚠️ **An `enter_at` that names the wrong landing only fails from a cold fixture.** Victory Road's
return trip asked for VR2F at (22,16) and the exit is not reachable from where that puts the player —
the mainline survived it because `EnterMap` falls back to re-routing over the *incremental* world
graph, which by then knew the (27,7) landing. A leg test's fresh agent does not, so it stalled at
eleven of fourteen steps. The `[policy]` line names the step; `probe_stall_actions` on the artifact
names the warps actually in reach, which is what identified the right landing.

### ⚠️ Fixtures are committed inputs

Each leg test snapshots its end state for the next leg, but the write is a no-op unless `--features regen-fixtures` is on
— otherwise every run silently changes the next run's inputs, and a leg "fails" only because an earlier one re-saved its
fixture. To regenerate after a deliberate change, run the affected legs **in chain order**:

```bash
cargo test --release --features slow-tests,regen-fixtures --bin gb -- can_clear_ss_anne --exact
```

### Benchmarking

⚠️ **Do not trust a single benchmark reading on this machine.** It has fast and slow states ~15% apart — the same
unmodified binary has measured `cpu_instrs` at 43.5× and 53.2× twenty minutes apart. Compare only adjacent paired runs of
the two builds, **alternate which one runs first**, and report both orders.

**`perf` works and needs no `sudo`** (`perf_event_paranoid` is 2). Build with `RUSTFLAGS="-C debuginfo=2"` into a scratch
`CARGO_TARGET_DIR`, then drive the benchmark with `BENCH_FRAMES=40000 BENCH_ONLY=pokemon` so there is enough wall clock to
sample and only one workload in the profile. ⚠️ Watch for sampling skid: a hot instruction is often paying for the *load*
feeding it, not for itself — that one cost an hour.

### Test ROMs and the resampler

`src/roms/` needs no pokered submodule. `cgb-acid2` **ships its own reference image**, so nothing in it was promoted from
`gb`'s own output; its README pins the 5-bit to 8-bit colour expansion as `(c << 3) | (c >> 2)` — the plain widening,
**not** a colour-correction curve — which is what `LcdColor::from_rgb555` implements. ⚠️ Adopting gambatte's `gbcToRgb32`
correction instead would break the comparison.

`src/audio/blip/tests.rs` checks the resampler two independent ways, and they fail differently. **Golden vectors** are
bit-exact comparisons against the original C++ (Blip_Buffer ships no test suite of its own); the fixtures in
`src/audio/data/blip_*.bin` come from linking the vendored library in `tools/blip-golden/`. Regenerate only after a
*deliberate* change to the algorithm or its parameters:

```bash
# 1. only if the realistic-signal input needs refreshing (writes src/audio/data/apu_capture_in.bin)
cargo test --release --bin gb -- audio::reference::tests::capture_golden_input --exact --ignored
# 2. always — reads apu_capture_in.bin, writes the other src/audio/data/blip_*.bin
tools/blip-golden/build.sh
```

⚠️ The goldens are pinned to `GOLDEN_TREBLE_DB` in the test module, deliberately *not* to `blip::DEFAULT_TREBLE_DB` — tone
is a taste knob, so changing what the emulator ships must not invalidate the port's correctness fixtures. **Invariants**
are the real regression net and need no C++ toolchain: every phase's taps summing to `kernel_unit`, a step depositing
exactly its own amplitude of DC, zero sample-count drift over ten emulated minutes, no aliasing on a 15 kHz square, and
surviving a minute of emulation with no audio consumer at all. There is deliberately **no WAV "ear check"** any more — it
was a listening aid rather than an assertion.

**Fast-forward.** The number keys `1`–`5` in the SDL UI scale emulation speed, and `render.rs` mirrors that into
`Audio::set_emulation_speed` so the resampler scales its *source clock* to match; without it a sped-up emulator produces
audio faster than the device drains it and the queue backs up. The speed is derived from `cycle_duration`, not from the
key pressed, so it tracks what the emulator actually targets — `REALTIME_CYCLE_DURATION / 5` truncates to 190 ns, which
is 5.016×.


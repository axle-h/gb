# The Pokémon agent, the policies, and what they report

Read before touching `src/pokemon/{agent,policy,text,tile_map,actions}.rs`, `AgentEvent` or any
`Display` it goes through, or `web/src/useEventStream.ts` and `Conversation.tsx`. Nearly every rule
below is a comment on the function or constant it names; this is the index of them.

## The agent loop

- `PokemonAgent::poll_policy` is the single seam every decision goes through, and it resets the
  clock the watchdog reads. Calling `policy.service_tools` from a new site makes the watchdog
  believe the run has been wedged for ever.
- The emulator never pauses while the model thinks. Tool batches are serviced only when `gb.run`
  advances the agent, so a pause spanning a tool call deadlocks. `HostConfig` carries the argument
  (`GB_PAUSE_WHILE_THINKING` lasted a day). The park on a spent quota is allowed because it happens
  after a request has already failed, with nothing outstanding.
- The watchdog (`Policy::{stuck_timeout, pick_unstick}`) raises a `Stuck` turn whose terminals are
  `press_buttons` and `wait`. It is asked on every tick of the jam, not once, and must not reset
  the clock it reads. It is blind to a policy that answers `None` for ever, because
  `since_last_policy_poll` resets on every poll whatever the answer: a battle menu the policy never
  answers looks healthy and prints nothing. Two such stalls shipped. Every move on zero PP
  (`battle_options` offers the moves anyway and the cartridge substitutes Struggle), and a ghost
  battle in Pokémon Tower (`battle::is_ghost_battle`; `battle_options` returns `Run` alone, for
  every policy including the scripted one). Their guards assert on actions taken or on the battle
  ending, never on silence.
- **One agent tick is 20 ms of *game* time, and every driver of a live emulator goes through
  `PokemonAgent::run` to get that.** `update` coalesces rather than catching up — hand it 250 ms and
  it runs the state machine once, 230 ms late — so `host.rs` and `sdl/render.rs`, which both pace on
  wall clock, used to make the agent's decision rate their own loop rate. A held direction keeps
  walking, so the agent has one step (267 ms) to notice a corner and turn; `host::MAX_CATCHUP` is
  250 ms. A deployed run oscillated across Route 12's one-wide corridor at (11, 63) until
  `MAX_MOVEMENT_SILENCE` gave up, three walks running, and filed a bug saying Route 11 was
  unreachable. ⚠️ **No test could see it**: `TestFixture::step` is `gb.run(AGENT_RESOLUTION)` and one
  `update` in lockstep, which is the one cadence at which it does not exist —
  `mechanics::a_corner_is_turned_at_a_coarse_host_tick` drives `TestFixture::step_coarse` instead.
- `MAX_MOVEMENT_SILENCE` (60 s) aborts a walk that never arrives, and reports it as
  `OverworldActionAbortedReason::DidNotArrive`, **never** as `NoRoute`. The two are opposite
  diagnoses: `NoRoute` says choose something else, `DidNotArrive` says the route was there and the
  walk did not finish it. Reporting the bound as `NoRoute` sent a deployed run hunting a pathfinder
  bug while standing two tiles from the warp it wanted.
- **The route is re-derived from `actions()` every tick and only `route[0]` is ever pressed**, so no
  recipe may depend on its own tail (the comment is on `AgentState::OverworldMovement`'s re-derive).
  A two-step plan only completes if the next recomputation independently picks step two first;
  walking is memoryless so it usually does, and the step-off/step-back a warp used to emit was the
  case where it did not.
- **A Surf mount is the one driver entered from the middle of a walk, so it is the one that has to
  give the walk back** (`AgentState::Surfing`'s `resume`). Three separate things were dropping it,
  and each cost a paid request on a route that crosses water: `assert_script_state` took the state
  over (the mount's own `.makePlayerMoveForward` is `GameMode::Script`, so `Surfing` is on the
  exemption list beside `UsingFieldMove`); the driver dropped to `Idle` rather than back to
  `OverworldMovement`; and it left on the first overworld tick, which is a flicker before the mount's
  text box is drawn, so `MOUNT_SETTLE_TICKS` waits for a sustained one exactly as `TeachingMove`'s
  `settle` does. ⚠️ **Whether the hijack fires at all depends on the map's NPCs** —
  `read_game_mode` needs `wScriptedNPCWalkCounter` non-zero, which is true only where someone has
  walked — so Pallet Town cannot test it and Cinnabar Island can.
- `GameMode::Script` during a walk is either a ledge hop (~660 ms, inside `DelayContext::long`'s
  rollback window) or an arrow-tile slide (up to 14 s). `wMovementFlags` bit 7 (`BIT_SPINNING`)
  says which, and the deadline is re-armed every tick while it is set. The guard asserts the abort
  count, because arrival was always true and the bug cost a paid request per hop.
- The screen lags RAM: `AutoBgMapTransfer` copies a third of the tilemap per V-blank, so a menu
  takes up to three frames to appear and `wTopMenuItemX/Y` is the authority on which menu is live.
  `BattleState::confirming` is the window after `Navigating` in which geometry is believed over
  text; without it a battle turn was decided twice, one paid request each.
- Which text reader to use is a fact about the game (`wIsInBattle`), not about `AgentState`.
- `wIsInBattle` has a **third** value: `$ff`, the loss sentinel `home/overworld.asm:355-359`
  writes before `HandleBlackOut` (`battle::LOST_BATTLE`). It means the battle is over and the
  player has not been moved yet, so nothing may read it as a battle (`read_battle_state`) and
  no decision may be put to the policy while it is set (`agent::blackout_in_flight`, which
  re-arms `AwaitingOverworldAction`'s delay). Without both, a black-out turn described the map
  the fight was on, a party on 0 HP and a battle that had ended; 31 of one deployed run's 38
  black-outs spent a request on it. The poison black-out is deliberately not covered — see the
  ⚠️ on `blackout_in_flight` for why waiting on `wOutOfBattleBlackout` deadlocks.

## Closed loops under A

Almost every jam is a menu the agent's own A press re-enters with the cursor untouched. Five rules
cover the class, each documented on its constant in `agent.rs`:

- A give-up in battle hands back latched into B (`BattleState::backing_out`).
- After 30 s with no decision point, and only on a list menu, a field-move box, a menu offering
  CANCEL or the START menu (never a yes/no, never in battle), the reader presses B until a poll.
- Silence bounds drivers (`DRIVER_ESCAPE_SILENCE`, `MAX_MOVEMENT_SILENCE`), not per-state tick
  counters, because a state torn down and rebuilt starts a counter over.
- A menu the agent did not open is closed, not confirmed (`MENU_HANDOVER_TICKS`, armed in
  `assert_text_box_state`; a short window, because `wFontLoaded` flips before the menu draws).
- A rule that runs at every text box trusts only the screen, never the lingering `wTextBoxID`
  (`MenuEvidence`).

Loops with a gate of their own: every PC menu (`in_pc_menu`, matched on `LOG OFF` too because the
item PC sets no flag); the START menu, six rows before the Pokédex and seven after
(`start_menu_row`); a TM or HM aimed at a Pokémon outside its learnset (`pokemon::learnset`, read
from the ROM; TMNUM and item id run in opposite directions); a key item the game refuses
(`item_use::field_use_refusal` from `ItemUsePtrTable`, plus an on-screen net latching B for
contextual refusals); a mart open while the policy is still thinking
(`PokemartState::AwaitingPolicy`, entered on sight and polled in the same tick, because one tick per
shop is a different RNG line). Each has a hand-rolled-policy test, since `DeterministicPolicy`
would skip the thing under test, and each is a frame-timing change that only `full_playthrough` can
price.

## What the map layer will and will not offer

- **A warp entry is not a door.** `MetaTileMap::warp_trigger` is a transcription of
  `home/overworld.asm`: a tile in the tileset's `warp_tile_ids` fires on the step onto it
  (`StepOn`), anything else needs `ExtraWarpCheck` — a warp carpet in front for the way you face
  (`TileSetId::warp_carpet_tile_ids`) or the map edge facing out — **and a direction held**
  (`HoldDirection`). Route 8's east gate has two entries and one of them, raw `$2C` at (9, 9), is a
  door the cartridge will not open from any approach. `actions()` gives a dud row up only when
  another warp to the same map is known to work, and standing on a `HoldDirection` entry emits a
  **one-button** route rather than a step off and back.
- ⚠️ `WarpTrigger::Unknown` exists because `_GetTileAndCoordsInFrontOfPlayer` reads the *screen*,
  so a tile on the map edge faces the border block, which `raw_tile_ids` does not hold. Three real
  doors sit there (the S.S. Anne gangway, Rock Tunnel's north mouth, Cerulean's badge house, whose
  SHIP tileset sends a house down the tile-in-front arm). Nothing is claimed about those.
- `MetaTileMap::crossings(to_map)` groups a border strip into the **runs** that are actually
  different decisions, with `reachable` per run. `boundary_blockers` names what the reachable
  region ends on, walls excluded because walls are true everywhere.
- ⚠️ **A neighbour the player cannot reach the edge of is said by the menu having no row for it,
  and by nothing else.** There was an `unreachable_connection_targets` and a `Fenced in:` line in
  the turn built on it; both are gone (the tombstone is in `prompt::situation`), so
  `tools::a_fenced_in_map_names_the_neighbours_it_cannot_reach` now asserts the menu's silences are
  *exactly* the fences, in both directions. A crossing wrongly dropped from `actions()` is no longer
  a row the model can miss, it is the whole answer.
- `actions()` still emits **one** crossing per adjacent map, the nearest, because emitting one per
  edge perturbs `route_toward` and the scripted run's timing. The others are named in the row's own
  prose and resolved by `tools::resolve_overworld`'s `connection_action` fallback.
- **A step from land onto water costs `SURF_MOUNT_COST` extra, and `bfs_from_player`'s `dist` is
  therefore a price rather than a step count.** Everything that asks "which of these is nearest"
  wants the price; `wander_action` is the one caller that means steps and takes them from
  `search_from_player`'s third map. It is a bucket queue rather than a heap **so that a map with no
  water routes exactly as it did under the plain BFS** — see the ⚠️ on `bfs_from_player`.
- ⚠️ **A square the *cartridge* refuses to let the player stand on is not in the block map, and
  routing over one loops rather than fails.** Gen 1 refuses by talking and then walking you off:
  `grep StartSimulatingJoypadStates pokered/scripts` finds a dozen, including Cinnabar's gym doorstep
  without the Secret Key, the Route 22 gate and Route 23's checkpoints without the badge. The walk is
  stopped by the text box, the policy re-plans the identical route, and it never ends.
  `PokemonAgent::turned_back_tiles` learns them from the cartridge's own behaviour — the test is *put
  back on the square you stepped from*, which a refusal does and progress never does — and
  `observe_state` overlays them beside `blocked_tiles`. ⚠️ **Cleared on map re-entry**, because what
  is remembered ("refused, given what I am carrying") expires, and a permanent entry would wall a run
  out of Victory Road.
- ⚠️ **A hard-coded obstacle was the first fix and it was the wrong shape**, worth knowing before
  reaching for one again. The square is reached by more than one route — on Cinnabar the walk that
  steps on the doorstep and the walk that avoids it are *both 29 steps*, a one-tile dogleg, so which
  comes out is a tie-break — and the table of coordinates would never have been finished.
- ⚠️ **A boulder push the cartridge refuses is refused in *silence*, so nothing downstream can see
  it and everything has to ask first.** `CheckForCollisionWhenPushingBoulder` adds two rules to the
  tileset's own collision list — no boulder onto a **staircase**, by raw tile id (`$15`), and none
  across a `TilePairCollisionsLand` boundary, where the pair tested is **(the player's tile, the
  destination)**, two squares apart and never the boulder's own — and answers a failure by returning
  with no text box, no animation and nothing on screen. `MetaTileMap::boulder_push_refusal` is that
  routine plus the one thing it has no reason to check (a push tile with no route to it, which is a
  push that can never be attempted), and all three seams go through it:
  `solve_boulder_push` will not plan one, `tools::resolve_field_move` will not accept one, and
  `AgentState::PushingBoulder` asks **every tick**, because the boulder moving is its only other
  exit and the map moves underneath it. A deployed run pushed VictoryRoad1F's boulder north into the
  alcove at (5, 14), under the staircase at (5, 13) that reads as ordinary walkable floor, and then
  held Up into it for `DRIVER_ESCAPE_SILENCE` five times, filing five issue reports calling the
  emulator broken. `endgame::a_boulder_that_cannot_move_is_refused_rather_than_shoved_at` runs on
  that exact save state. ⚠️ **A boulder with no push left is a reset rather than a dead end**, and
  the refusal says so: Gen 1 re-reads a map's objects on every `LoadMapData`, so walking out and back
  undoes every push on the floor (`endgame::leaving_a_map_puts_its_boulders_back`). Saying only the
  first half is how a model decides the game is broken.
- ⚠️ **"Can the player get to the push tile" is two questions, and asking only the second one shipped
  the same silent stall a second time.** `reachable_tiles` is not the set of squares the player can
  *stand on* — its own doc says so — so a wall touching floor is in it, and a boulder with rock on
  one side had the push from that side accepted: nothing to attempt, nothing on screen, sixty
  seconds. The stand tile must be `Empty`/`Grass`/`Warp` as well as reachable. Reported from a
  deployed run on 2026-09-04, and true of three of the four boulders in the committed fixtures,
  every one of them a **left** push with rock to the east.
- ⚠️ **A goal row's search is cached, because `actions()` runs on every 20 ms tick.** One capped
  BFS over boulder layouts per (boulder, target) pair, fifty times a second, measured **11.4 ms per
  call on Seafoam B3F and 3.1 ms on VictoryRoad1F against 82 us on a map with no boulders** — a tick
  costs ~0.4 ms of wall clock to emulate, so the menu alone took the agent from ~48x real time to
  **1.9x**. `PlanKey` is the memo and it is exact rather than hashed, because the cost of a
  collision is a boulder shoved somewhere nobody asked for. Two of its fields carry the design: the
  player's **reachability component** rather than their square (they move every tick, the region
  does not, and the search already keys its own states this way), and a **two-bit-per-tile
  walkability bitmap** rather than `meta_tiles` (which carries NPC sprites, so hashing it wholesale
  would invalidate every entry whenever anyone took a step). The pre-filter is inside the memo too:
  it is a BFS per pair and most pairs are hopeless, so an unsolvable pair is the one that most needs
  answering once. `cinnabar::a_boulder_floors_action_menu_is_not_a_search_per_tick` is the bound.
- ⚠️ **A boulder goal is bounded on *progress*, not on a shove count, and a fixed count was
  measured wrong.** `MAX_PUSHES` was 24, justified as "Victory Road's worst floor solves in well
  under ten"; VictoryRoad3F's (3, 5) switch needs **27** — a boulder walked across the floor one
  push per tile with three others shifted out of the corridor first — so the coverage walk of
  2026-09-08 was abandoned three pushes from the end having done nothing wrong. The planner returns
  a shortest path through boulder layouts, so a productive shove leaves strictly fewer to make:
  `MAX_PUSHES_WITHOUT_PROGRESS` (12) counts shoves since the plan last got shorter, which catches a
  loop in a dozen pushes and never fires on a hard puzzle, and `MAX_PUSHES` (120) is only a
  backstop. ⚠️ **And it aborts as `PuzzleRanLong`, not `DidNotArrive`** — the two shared a reason
  whose prose says "the walk was given up after 60 seconds of game time", so a puzzle out of shoves
  reported a failed *walk* and sent two investigations to the router.
  `endgame::victory_roads_hardest_switch_is_one_decision_however_many_shoves_it_takes`.
- ⚠️ **A cast reports its outcome, and for a long time it was the one action that did not.**
  `fishing::tick` dropped to `Idle` in silence on a miss, and on a bite the wild battle replaced the
  state before it could say anything — so a model that cast was told nothing at all, and the
  coverage walk scored every `Fish` row `Silent`. A miss now completes and says whether anything bit
  (and, on a Super Rod `wRodResponse` of 2, that the map has no fish at all, so casting again is
  pointless); a bite aborts with `Battle`, which is what `resume_after_battle` picks back up.
  `postgame::fishing::the_action_menu_offers_a_cast_when_a_rod_is_in_the_bag`.
- ⚠️ **A hole goal and a switch goal are *done* differently, and writing one test for both stalled
  the Seafoam leg for its whole budget.** A switch keeps its boulder, so
  `AgentState::SolvingBoulderPuzzle` finishes when `boulders()` contains the target. A **hole
  swallows it**: nothing is ever standing on a hole, so that same test is structurally unreachable
  there and the driver simply pushed to `MAX_PUSHES` — long enough, on Seafoam B3F, to shove a
  second boulder down the *other* hole in passing and strand the next goal. A hole is done on either
  of two sightings, because a boulder is drawn *on* the hole for the frame before it drops: a
  boulder on the target, or the tracked one gone with nothing on a neighbouring square for the
  re-acquire to pick up. Catching only the second reports the success as `NoRoute`.
  `PolicyStep::DropBoulderInHole` had the mirror of the same bug — a *count* baseline captured on
  the first tick the step is asked, which on that floor was after the drop, so a solved floor never
  popped. A step that names its boulder needs no baseline at all.
  `cinnabar::both_seafoam_holes_are_filled_by_the_only_boulders_that_can_reach_them` pins both, off
  `seafoam-b3f.bin`, in ten seconds rather than the leg's sixty game-minutes.
- ⚠️ **A cut tree and a boulder push finish their own action**, in `OverworldMovement`'s empty-route
  arm beside the fishing row (2026-09-04). Both walks end facing a thing with exactly one legal
  continuation, so ending there made the continuation a second decision — a paid request for the
  model, a step to forget for anything scripted — and the row completed looking like success.
  `MetaTile::Boulder { at, push }` and `MetaTile::Cut { at }` are synthesised actions like
  `MetaTile::Fish`, one row per shove `boulder_push_refusal` allows and one per reachable tree, gated
  on `MetaTileMap::can_strength` and `can_cut`. `AgentState::PushingBoulder` arms
  `BIT_STRENGTH_ACTIVE` itself through `UsingFieldMove`'s new `resume`, which is why there is no
  arming decision anywhere any more.
- ⚠️ **The arming's own text box is inside the decision, and `UsingFieldMove` needed
  `MOUNT_SETTLE_TICKS` to stop it eating the push.** "The overworld is back" is briefly true between
  the party menu closing and "SHELDON can move boulders!" being drawn, so handing the push back on
  the first overworld tick handed it into the gap: `PushingBoulder`'s next tick saw a text box,
  called it an interruption and dropped the decision, and the model paid a second request for the
  identical shove — *"Retrying the up push, the Strength text box interrupted the first attempt"*,
  2026-09-04, within an hour of the change that was supposed to make a push one decision. The settle
  is on the `resume` path only, exactly as `Surfing` counts only a mount that took: without a push to
  give back this state ends at the policy and `Idle` clears the box itself.
  `endgame::a_boulder_row_arms_strength_and_pushes_on_one_decision` asks for one push from a fixture
  with Strength unarmed and then answers nothing at all.
- ⚠️ **A boulder is pushed from squares `bfs_from_player` will not route to, and the two halves
  disagreed about exactly one of them.** `MetaTileMap::push_search` is the flood fill the boulder
  code uses instead: `solve_boulder_push`'s `floor` has always counted a warp tile as standable
  (VictoryRoad1F is unsolvable from its *starting* layout otherwise — measured, not assumed), while
  `boulder_push_refusal` asked `reachable_tiles`, which records a warp but never expands through one.
  With its boulder on (9, 16) the deployed run of 2026-09-04 was shown one row, `Down`, which puts
  the boulder on the bottom row where nothing can stand behind it again; the `Up` the solver wanted
  is pushed from the entrance warp at (9, 17). It saw it coming and had to take it anyway.
  ⚠️ **A warp is walked *over*, never *into***: a `StepOn` warp is not a square anybody can stand on,
  and a `HoldDirection` one must never be arrived at in the direction that fires it — entering
  (8, 17) from above is a step Down at the map edge, which is how `actions()` takes that warp on
  purpose, so the fill goes round through (7, 17) holding Right. ⚠️ **`warp_trigger` answers for any
  square, not only a warp** (`HoldDirection(Down)` for every tile on a bottom row, walls included),
  so that rule must be asked only of a `MetaTile::Warp` — asked of everything it sealed off the two
  ordinary squares that are the way round. `route_to_push_tile` is the same fill with the path kept,
  because `PushingBoulder` walking with `route_to` had the identical split one layer down.
  `endgame::victory_road_1f_is_solvable_from_the_action_menu_alone` plays the floor the way a model
  has to — `solve_boulder_push` for the next push, and it must be a row — and
  `a_push_from_a_warp_tile_is_offered_because_victory_road_needs_one` pins the square itself.
- ⛔ **The turn must not tell a model that a Strength floor is lost, and two versions of that
  sentence have now shipped and been reverted.** The tombstone is in `prompt::situation`. Read off
  `solve_boulder_push` for every switch and hole, "no plan" is *not* "unsolvable": the solver plans a
  boulder onto a target, and on a multi-stage floor no boulder reaching the switch yet is ordinary —
  VictoryRoad3F answers exactly that on arrival (four boulders, a switch behind a barrier, a hole to
  the floor below), so the line fired on the hardest puzzle in the game and a deployed run walked up
  from 2F and straight back down **twenty times**. Measured per boulder off `boulder_pushes` instead,
  it is a true sentence and still wrong: VictoryRoad1F's boulder at (14, 2) is walled in on its
  *starting* square, so it fires where nothing has gone wrong and advises a reset that changes
  nothing. Telling a stuck boulder from a decorative one needs the map's original object data, which
  that layer does not have. What the turn says is what is measured — these are the legal pushes, and
  leaving the map puts every boulder back.
- ⚠️ **A push cannot report its own completion, and an `OverworldActionCompleted { Boulder }` that
  never fired once was deleted rather than chased.** The shove and its dust animation are
  `GameMode::Script`, `assert_script_state` takes the state over on sight of one, and
  `PushingBoulder` is not on its exemption list — so the driver is replaced by `RunningScript` on the
  tick the boulder starts moving and never gets another. Putting it on that list would change how a
  push interacts with the machinery that stops it over-pushing, which is a large risk for a log line;
  `MetaTile::Fish` hands off to a driver without a completion for the same reason.
- ⚠️ **`MetaTile::Cut { at }` is a separate variant from the terrain `MetaTile::CutTree`, and the
  split is what lets a row name its tree.** `OverworldMovement` re-derives its target every tick by
  `a.tile == destination`, so while every cut row shared one anonymous tile that match found
  whichever sorted first — a row saying "cut down the tree at (5, 8)" could walk to a different one,
  and the walk was unstable when the nearest square beside a tree changed on approach. ⚠️ **`id_kind`
  still answers `CutTree`**: an id is a key a resumed run quotes back out of its own conversation.
- ⚠️ **`observe::map_view` filters people by reachability and *flags* warps.** Different answers on
  purpose: a person out of reach is someone nothing can be done with, a door out of reach is still
  where you come out if you get there. `WarpView::reachable_from_here` is the same call
  `read_route` makes for a hop it cannot start, and the picture's label is painted from it.
  `map_view_lists_only_the_people_the_menu_offers` and
  `map_view_flags_every_warp_the_menu_cannot_offer` hold the two views together across every
  committed fixture.

## The coverage oracle

`integration_tests/coverage.rs`, over the event stream, so it works under any driver. See
[test-suite](test-suite.md) for the tier and [coverage-plan](coverage-plan.md) §5 for the plan.

- Four verdicts and a fifth that was not in the plan: `Completed`, `Blocked { times, message }`,
  `Defect { reason }`, `Unreached`, and `Silent` — chosen, outcome never reported (the finding
  above).
- ⚠️ **`REPEAT_IS_A_DEFECT` is 10 and was 3.** A gate is worth one try per pass over a map, because
  the thing that opens it may have happened since; three sweeps of Pewter City re-tried its east
  exit three times and were called a defect for diligence. Ten sits clearly above once-per-pass and
  clearly below the 143 aborts that made this a rule.
- ⚠️ **An exploring frontier must order its exits by how often it has already taken them, not by
  where they lead.** Being turned back at a gate changes nothing the brain can see, so an exit that
  scores best on promise stays best for ever: the first version took
  `PewterCity:40,18:Connection` 59 times in one run. Count first, promise second, and every exit is
  taken once before any is taken twice — 12 maps became 28.

## Cheating past the gates, safely

`integration_tests/cheats.rs` and the `debug_*` half of `postgame/debug.rs`. See
`docs/coverage-plan.md` §3, and [test-suite](test-suite.md) for the tier.

- The line is unchanged: **play path — button input only; debug tier — free to write RAM**, and
  `play_path_contains_no_debug_ram_writes` reads the play-path sources from disk to enforce it.
  Nothing in `cheats.rs` is on the play path: a driver applies it *between* agent ticks, so a policy
  sees the result only through an ordinary `GameState`.
- Four new primitives: `debug_set_badges`, `debug_heal_party`, `debug_restore_pp`,
  `debug_teach_move`. ⚠️ **A badge is the one wholesale write admitted, and it is admitted because it
  is a *capability* rather than an event**: nothing in the game keys a script off `wObtainedBadges`.
  Writing `wEventFlags` desynchronises scripts from map objects and every stall found in such a save
  is a false positive.
- ⚠️ **Never write the party during a battle or inside the black-out window.** Gen 1 copies the
  active member into `wBattleMon` on send-out and writes it back on switch-out, so a party-struct
  write mid-battle un-heals on the next switch; and `wIsInBattle == LOST_BATTLE` is written by the
  overworld loop *before* `HandleBlackOut` heals and warps, so a write there is one the cartridge
  throws away. `Cheats::party_writes_are_safe` gates both, counts every refusal, and has a test.
- ⚠️ **Five HMs do not fit in four move slots**, so the god party is three: a fighter with four
  attacks and no HM (an HM is the one move `pick_move_to_forget` will never drop), a slave with
  Cut/Surf/Strength/Flash — the four the action menu gates rows on — and a slave with Fly, which is
  the only field move that travels. Whatever the game itself produced is kept behind them, as the
  evidence that the story ran rather than being skipped.

## The random policy

- `RandomPolicy::exploring` is the fuzzer `integration_tests::soak` drives: the ids of the last
  `EXPLORE_MEMORY` overworld actions are kept and each repeat multiplies that action's weight by
  `EXPLORE_DECAY`, because a uniform walker's distance from where it started grows as the square
  root of its steps and five hours from Pallet Town measured as five hours *of* Pallet Town. It is a
  weight rather than an exclusion (floors exist whose only exit is the one just used), it is
  recorded on what was chosen rather than on what the agent managed to do, and it leaves the battle
  draw uniform — a recency penalty there pushes a walker onto `Run` and `Item` until it blacks out,
  and a black-out warp throws away everything the starting state was chosen for. `--policy random`
  is the plain `RandomPolicy` and is unchanged.

## The scripted policy

- The party is one Squirtle that does all the fighting, an Oddish for Cut and a Machop for Strength.
  Surf is the only HM on the fighter: `pick_move_to_forget` never drops an HM, so an HM is a
  permanent slot. One fighter to lv85 beats three to lv75 because experience is cubic. The argument
  and the black-out table are on `PolicyStep::game_steps`.
- A field move is answered by whoever knows it, slot and move index both resolved by
  `policy::field_move_carrier`; a `PartyRef` on a step is only the fallback.
- A grind's trainee leads (switching it in halves the payout). Pacing uses `wander_action`, never
  a warp. Status is cured on sight, not on a threshold, because a Full Heal restores no HP. Check
  the mainline can afford the medicine a fixture is seeded with (`agent::affordable` trims the
  order). Argue about a site with `probe_grind_sites`. Cerulean Cave is gated by a guard's body and
  is unusable before the Elite Four. A grind belongs outdoors, since a cave black-out warps to a
  Centre the route cannot leave.
- Every routing branch is bounded (`MAX_HEAL_ROUTE_WAIT`, `MAX_GYM_ROUTE_WAIT`) and hands back to
  the queue rather than parking: the queue is a route, so its next step walks out of the dungeon.
  `route_toward` reads the incremental graph, keyed on entries the agent actually landed on, so a
  `Goto` cannot cross Kanto from a leg test; explicit `enter`/`enter_at` hops can, and gate houses
  need `enter_at`. Walk home and back once before a grind, or the return edge is missing.
- Black-outs went from twelve a run to none through `needs_a_centre` (a fainted lead, no PP, or
  hurt beyond what the bag can fix; allowed only where the route can get back), `party_is_fresh`
  (a heal is done when the party is full, not when the nurse speaks), the bag-aware flee threshold,
  `damage_per_turn` halving charge moves, and a damage gate replacing the level gate on switching.
  Each function carries its own reasoning.
- A `CatchPokemon` that gives up records the species in `DeterministicPolicy::catch_abandoned`, and
  `TeachMove` / `UseStrength` / `Dig` skip a `PartyRef` naming one. Their wait on a `Species` target
  is unbounded on purpose (the Celadon Eevee is still a Poké Ball on the floor when its step reaches
  the front), so the failed catch has to say so or the route stops for the life of the process, with
  no watchdog to see it — `stuck_timeout` is `None` for every scripted policy. Both halves are pinned
  by `abandoned_catch_tests`, since a plain pop passes the skip half and breaks the Eevee.
- The heal arm sits **above** the catch throw in `pick_battle_action`: a throw is a turn spent not
  defending, so a catch is the one place the lead takes damage for several turns running with nothing
  answering back, and below the throw the arm was unreachable for the whole hunt.
- ⚠️ Weakening before a ball is gated on a throw having **missed** (`catch_ball_baseline`, read off
  the bag because `pick_battle_action` is polled many times a turn). The level guard beside it
  suppresses weakening and throws at a **full-HP** target, which is how the 2026-09-05 run lost its
  Cut carrier; dropping it is correct in isolation (`pick_best_move`'s non-KO filter is the better
  test) but costs two turns at the Route 25 catch, re-rolling the RNG stream `full_playthrough` is a
  golden replay of — it then fails at 258/522 on `CutTree { CeladonGym }`. The golden stream lands
  that Oddish on its first ball, so gating on a miss leaves the recording untouched. ⚠️ **Mainline
  battle tactics are frozen: a change that costs a turn before the last mainline catch re-cuts the
  route.** The full argument is on the guard.
- `Policy::restart` rebuilds the policy from its seed, so a field added later is untainted by
  construction. `resuming_in` parks when the cursor file is missing on a mid-game save or the route
  has changed under it; neither can be told from "a new game" by the file's absence.
- Gen 1's bag holds 20 entries and the route runs at the cap, so every pickup is somebody's toss,
  silently. `Bag::best_pokeball` falls back to the Master Ball when a pinned ball runs out. Adding
  one permanent entry cost two tosses, and it saturated where nobody predicted: TM24 on the Celadon
  Mart roof (a vending machine that sells nothing is a step with no completion, so `full_playthrough`
  stalled at 385/521 rather than failing) and TM21 at the Indigo mart (four Revives had been buying
  nothing for as long as anyone had looked). A toss placed beside the pickup it is for frees nothing
  — put it where the bag binds.
- A full bag and an empty wallet are indistinguishable from the policy: `mart_baseline` fires when a
  visit moved neither counter, which is true of both, and it said "the wallet covers no more" over
  ¥31,434. `state.bag.len()` tells them apart, and is only safe to read now that all fifty TMs are
  named; an item already in the bag is exempt, because a stack grows without needing a slot.
- The Elite Four is 26 Pokémon against the starter's 35 PP, and no mart in Kanto stocks an Ether or
  an Elixer, so the Indigo Nurse is the last PP the run gets and the rooms cannot be left. The route
  carries the Pokémon Tower 4F **Elixer** — an Elixer rather than either S.S. Anne Ether, because
  only it restores all four moves, which is why `ItemUsePPRestore` skips the move menu for it. Spent
  in Lance's room, not the Champion's: the rival's script starts the battle on entry, so a step after
  `enter(ChampionsRoom)` is not reached until the fight is over. `items::blocked` applied Ether's
  precondition to Elixer until 2026-09-02; `.useElixir` only fails when *no* move took any PP.
- The Elite-Four switch tactic's `move_dmg` had no `pp > 0` filter while the two arms around it do,
  which is a livelock rather than a mis-rank: a 0-PP Surf scored 122, so the run ping-ponged between
  the starter and a lv24 Machop every turn until the party was wiped. Two arms over one decision must
  share a damage model.

## Prose the model and the page read

- `impl Display for AgentEvent` is a UI contract: `host.rs` formats it straight onto the page and
  `prompt::describe_event` sends it to the model. `MetaTile`'s `Display` names its target as a noun
  phrase (`the warp to OaksLab`, `Mom`); `MetaTile::kind` stays the variant name because
  `OverworldAction::id` mints `PalletTown:5,6:Warp` from it and the id is re-resolved by string
  equality. `id_kind` ends a person's id in their name with spaces stripped, and the word "sprite"
  appears nowhere a model reads.
- **`OverworldAction::id` is the one definition of an action id**, in `actions.rs` rather than in
  `llm::tools` — `agent.rs` is not compiled with the `llm` feature at all, and two spellings of an
  id would be two spellings of a key. `tools::overworld_id` is one line onto it, and
  `AgentEvent::StartedOverworldAction` carries it so something reading the event stream can key on
  the same string the model chose from. ⚠️ **It is not in the prose**: `Display` is a sentence, the
  id is a key.
- ⭐ **A sprite id has no coordinate — `ViridianCity:OldMan`, two fields where every other id has
  three** — because the square it used to carry was the *approach tile*, which `actions()` re-picks
  as the nearest of four every time the player moves. One object minted an id per square it could be
  faced from: the C3 sweep of 2026-09-08 held 270 sprite ids for 136 objects, 26% of the whole
  frontier redundant, eleven of them for one Viridian City Youngster. It is the boulder-goal fault
  one layer over and it has the same answer — key on what does not move. Two facts hold it up, both
  asserted in `actions::tests`: `map + name` is unique across all 208 maps (919 sprites, no repeat),
  and anything reading an id takes the map off the front and the kind off the back rather than
  counting fields. ⚠️ It also removed a *misreading*: the coordinate was the player's square and a
  deployed run read it as the object's, playing the Poké Flute at empty ground three times
  ([deployed-run-defects](deployed-run-defects.md), Route 16 Snorlax).
- ⭐ **Winning the game does not hand the world back, and the agent stops playing it.**
  `wNumHoFTeams` goes up on the ceremony's *first* frame and `scripts/HallOfFame.asm` only reaches
  its `jp Init` at the end of the credits — 12 s to 169 s of game time, measured — and for all of it
  `wCurMap` still reads `HallOfFame` and the coordinates still read the square the player was
  standing on. `PokemonAgent::ending` latches on that same edge and `update` returns above the
  watchdog, above `game_mode()` and above any policy poll, pressing nothing: 15 walks into the room's
  two exits became 1. ⚠️ **Not a `Map::HallOfFame` check** — the room is legitimate to stand in and
  the fault is the reset. ⚠️ **It clears itself**, on `a_game_is_loaded` (`wPlayerID`) going zero and
  then non-zero again, which is the only signal that separates a reset from an ordinary screen
  transition; `restart` clears it outright because a loaded save state never passes through zero.
  `postgame::phase0::the_agent_stops_playing_a_world_the_cartridge_has_reset`.
- ⚠️ **Only the *start* of a walk carries an id, so a reader pairs positionally**: a start opens an
  action and the next `OverworldActionCompleted`, `OverworldActionAborted`,
  `OverworldInteractionCompleted` or `OverworldPickupFailed` closes it. The terminal events carry a
  `MetaTile` and, for an abort, where the walk *stopped* — neither identifies the row that was
  chosen. `coverage::CoverageLog` is built on that pairing and counts the walks that interleave
  rather than overwriting a verdict; a walk re-issued after a battle is a normal one, and the count
  is a fact rather than a fault.
- `BattleActionStarted` carries the nickname and the opponent's species, read at the decision point
  (a trainer's lead is not loaded at `BattleStarted`).
- ⭐ **`MetaTileMap::position_settled` is false for one tick per northward or westward connection,
  and nothing may draw a conclusion from `player_position` while it is.** Crossing north or west
  leaves `wYCoord`/`wXCoord` at **255** (the ROM's −1) with `wCurMap` still the *old* map, until
  `CheckMapConnections` runs; `MetaTileMap::new`'s bounds clamp — which has to stay, it is what keeps
  `meta_tiles` indexing in range — turns that into `(255 + north_extra).min(height - 1)`, a plausible
  square at the **opposite** edge. The BFS then reaches nothing and the walk was abandoned with
  `NoRoute` one tick before it landed. ⚠️ Southward and eastward crossings go one row *past* the map
  onto the connection strip, which is a real reachable tile, so they must stay settled;
  `a_coordinate_that_underflows_a_map_edge_is_not_a_position` pins both halves. ⚠️ **The hold is in
  `OverworldMovement`'s `NoRoute` arm, not at the top of the arm**, and that placement is the whole
  care in the change: gating the whole tick also works and breaks `full_playthrough`, because a
  golden RNG replay re-rolls every route after any tick that presses a different button. Only the
  arm that told the lie changes, and it presses and releases nothing.
- `OverworldActionAborted` carries `at` in the expanded coordinate space, and its `Textbox` reason
  reads "the game stopped you to say something": "it was interrupted" made a deployed run file a bug
  about a locked gym. Nothing counts or withholds repeated aborts; noticing is the model's job.
  `OverworldInteractionCompleted` exists because a route to a sprite is `[A]` for ever once
  adjacent. Facing means what the game means, over a counter (`interaction_in_front` hops;
  `tile_in_front` must not).
- ⭐ **The heal-return detour is bounded by hops taken (`MAX_HEAL_HOPS`), not only by failures to
  route.** `heal_route_stuck` counts polls where `route_toward` answered nothing and is reset by
  every hop that *does* route, so a detour that routes perfectly and never arrives resets it for
  ever. Measured: 33 Route 13 ↔ Route 14 crossings and still going when the fixture's stall
  detector killed the run. ⚠️ A wedged scripted run is silent — no watchdog, and `/api/events` goes
  on looking healthy — and `--policy deterministic` is what is deployed.
- ⚠️ **The oscillation under it is a `WorldGraph` landing mismatch and is *not* fixed.** A map split
  by ledges is held as several sections keyed on the raw landing; an edge to it records the
  **geometric** border `to_position`, and `bfs_nodes`' `SNAP_THRESHOLD` resolves that to whichever
  observed node is nearest. Route 13's exit to Route 14 resolves to a 9-edge section at (19, 8)
  while walking it actually lands in a 1-edge pocket at (19, 6) whose only exit is back to Route 13
  — so the planner scores the door 7 hops from the Fuchsia Centre, takes it, arrives somewhere else
  and re-plans identically. The two sections are **two tiles apart**, so no distance threshold can
  separate them; the graph has to learn the landing a door actually deposits the player at.
- `AgentState::CheckingTrashCan` had three callers and now has two — the gym-bin puzzle and the
  Mansion/Rocket switches, both progression gates. Hidden-item collection and the `interact` tool
  that shared the driver are gone (2026-09-03); see [llm-turn-loop](llm-turn-loop.md). Its
  unreachable-target message names the tile it was actually aimed at, which it did not while
  `interact` existed: it read "Can't reach trash can at (23, 30)" wherever the model pointed it, and
  three of a deployed run's ten Cerulean issue reports quote that line as proof the map model is
  broken. There is no gym in Cerulean.
- ⭐ **Every door out of a driver state closes the action that opened it** — and `Grass` and
  `CutTree` closed none of theirs until 2026-09-07, which C3's first walk from Pallet Town found:
  66 of 307 chosen ids went silent and every one was one of those two.
  `AgentState::PacingForEncounters` now carries the `MetaTile` the row named and reports all three
  of its exits: an encounter as `Battle` (the same abort an interrupted walk gets, so
  `resume_after_battle` picks the patch back up by itself), the budget expiring as
  `NothingAppeared`, and a map change as `WrongMap`. `AgentState::CuttingTree` carries `from_row`
  and ends with `OverworldActionCompleted { Cut }`, whose sentence is "✓ cut down the tree at
  (5, 8)" rather than "✓ reached" it. ⚠️ Both replaced a `TextBox` **the agent had made up**, which
  is the cartridge's voice used for the agent's own account. A **boulder push is still silent and
  deliberately so** — see `AgentState::PushingBoulder`'s ⚠️, where the shove runs as a script that
  takes the state away before it can report. `coverage::Verdict::Silent` stays as the guard that
  finds the next one; [coverage-plan](coverage-plan.md) §5.2.2.
- `OverworldActionAbortedReason::NothingAppeared` is an abort the oracle scores a **completion**: the
  pace ran its whole `PACING_BUDGET_TICKS` and the game's own 8-in-256 roll came up empty, which is
  the action done rather than the action failed. ⚠️ Its sentence quotes the budget in seconds
  because "nothing appeared" without a number reads as "I did not walk far enough" — and
  `PACING_BUDGET_SECS` rounds in nanoseconds, since a tick is 19.9996 ms and `as_millis()` was
  telling the model 57.
- `check_pending_pickup` reports `OverworldPickupFailed` when the ball sprite is still there after
  the overworld returns, which is how a full bag refuses every pickup: armed on the interaction,
  answered later, latch cleared either way, keyed on `PictureId::PokeBall`.

## The text reader

- The screen is a page being typed. `PokemonTextReader` extends on a prefix relation either way,
  splices on the overlap against the page (never the buffer), commits nothing on a blank frame, and
  needs `MISMATCHES_BEFORE_PAGE_BREAK` (2) mismatches because `AutoBgMapTransfer` tears boxes.
  `commit_page` joins verbatim; both attempts at deduplicating deleted real battle text.
- `PPU::tile_coordinates` walks the 20×18 screen and decides the window per tile; Red parks the
  window off-screen at WY=144 with a stale enemy HUD still in it.
- A box is flushed wherever the reader stops being in charge, not only when it closes:
  `flush_text_reader` hangs off `set_state` and `backup_current_state`, because every blocker in
  Red prints a message and then runs a script. `take` reports the open page too and clears rather
  than replaces. `PokemonAgent::event` drops empty boxes. `### On screen` is a rolling fragment and
  not a substitute for the `TextBox` event.

## The page's copy

- `useEventStream`'s `fold` drops `text_box` and `overworld_interaction_completed` on the client.
  Never filter at the publish: the transcript writes what is published. A tool call and its result
  are two events paired by call id (`attachResult`), never by position. Pictures are referenced,
  not carried (`ToolResult.image`, a 16-entry ring keyed by the announcing seq,
  `/api/tool-image/{seq}/image.png`, 404 expected for old ones), and `MAX_TOOL_RESULT` truncates
  the broadcast copy only.
- The heartbeat is sent on change with a 2 s keepalive; `says_the_same_as` excludes the clocks, and
  anything added to `StatusSnapshot` must be compared there. `/api/events` opens with the latest
  heartbeat, plan and battle script (`join_events`); anything else that becomes send-on-change
  belongs there too. Speed is derived from consecutive heartbeats (`sampleSpeed`), never from the
  lifetime average, and needs no park case.
- `STALE_MS` (`api.ts`) is the reconnect signal on both streams, fed from the status heartbeat and
  the inflated video chunks, because a dead network produces no error. A reconnect of `/api/events`
  resets the fold inside `onopen` and refetches `/api/history` generation-guarded; a hidden tab is
  resynced on return.
- Every `UiEvent` carries `at` (Unix ms), the only clock the page can date a line by. The SPA's copy
  is optional and `signature` excludes it.

## The fishing row

`MetaTile::Fish { rod }` is an action minted by `actions()` on three gates (a rod in the bag, a
`WaterTilesets` tileset, castable water), never a tile. Its route ends facing the water with no A;
`OverworldMovement`'s empty-route arm enters `AgentState::Fishing` with the rod re-resolved from the
live bag, always the best rod. It is not a grind engine; the measurement is on
`PolicyStep::gauntlet_grind_steps`.

- ⚠️ **`nearest_castable_water` sweeps every water tile on the map, so it must run *one* search for
  the lot.** `route_to_face` is a whole Dijkstra, and `actions()` is re-derived on every 20 ms agent
  tick — one search per water tile put Route 23 at **117 ms a tick against a 20 ms budget**. Surf is
  what made it visible: water is a pass-through node only once the party can mount it, which tripled
  what each of the 369 searches explored. `MetaTileMap::route_to_face_within` takes a search already
  run (`search_for_faces`); the same map measures 0.37 ms. Any new caller asking about more than one
  target wants it too.
- ⚠️ **A tick that overruns its budget reads as a *stutter*, not as slow motion**, which is why the
  above was watched for a whole deployed run without being recognised as a performance fault.
  `host.rs` publishes one video frame per loop iteration and each iteration emulates up to
  `MAX_CATCHUP` (250 ms) of game time, so an iteration that spends 1.5 s of wall clock on twelve
  over-budget agent ticks moves the player about one whole walking step and shows nothing in
  between. Route 23 measured **1.9 s of wall clock per tile, worst 4.2 s** at 20 % speed — a
  character that jumps a tile, freezes for seconds, jumps another tile. The symptom to reach for
  here is the per-tick cost of `actions()`, not anything in the movement drivers.

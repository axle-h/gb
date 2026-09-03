# Deployed-run defects — routing, gates, targets and the plan

What the deployed run of 2026-09-02 (`run-20260902-215720`, `z-ai/glm-5.3-flash`, build `cebf2a5`)
actually walked into, why, and what to do about it. Written 2026-09-03 against a run in flight at
turn ~1287, 60.3 M prompt tokens, 17 issue reports filed by the model itself.

**Status:** six root causes, ten work items. **All ten are done (2026-09-03).** W10 is not from the
2026-09-02 evidence below: it came out of watching the *same run still going* on 2026-09-03, and it
is the one item here that removes a feature rather than fixing one.
The rules they left behind have moved into [pokemon-agent](pokemon-agent.md) and
[llm-turn-loop](llm-turn-loop.md), which are the indexes to keep current; what is kept here is the
evidence, because every one of these was argued from a save state rather than from a guess.

Every claim below is either a probe of a committed save state, a deterministic reproduction from a
committed fixture, or a quotation from the run's own `conversation.jsonl`. Where I could not prove
something it says so.

---

## 0. How to reproduce any of it

The run directory is the evidence. Pull it off the PVC (⚠️ `kubectl exec` is classifier-blocked,
`cp` is not):

```shell
kubectl -n gb cp gb-<pod>:/runs/run-20260902-215720 ./run --retries=3
```

`issues/turn-<id>/state.gbst` is an emulator state taken at the moment the turn was put to the
model. Two throwaway probes read them; both are in
`scratchpad/stalls-with-probes.rs` and are meant to be pasted onto the end of
`src/pokemon/integration_tests/stalls.rs`, not committed:

- `probe_issue_state` — `PROBE_STATE=<path> cargo test --release --bin gb -- --ignored --nocapture
  probe_issue_state`. Loads the state and prints `can_cut` / `can_surf`, the connection targets, the
  whole action menu, and an ASCII map of the `MetaTileMap` where **upper case means reachable from
  where the player is standing**. That last column is what every map defect below was found with.
- `probe_route8_gate` — drives the real agent from `at-lavender.bin` and then the joypad by hand.

⚠️ **The grid's letters must not be self-uppercasing.** The first draft used `C` for a connection
and `W` for a warp, so `to_ascii_uppercase` was a no-op and every tile read as reachable. Lower-case
base, upper-case for reachable.

---

## R1 — A map's header connectivity is not its walkable connectivity

`WorldGraph` joins maps by ROM map header. `observe::route` then names **the tile on this map to
leave by** and never asks whether the player can reach it. `MetaTileMap::actions()` does ask, so it
mints no row. The model is told to leave by a tile it is not offered and cannot walk to, and is
given no reason.

**Cerulean City, turns 233–306: 65 turns, ten issue reports, the second-largest sink in the run.**
Probe of `issues/turn-304/state.gbst`:

```
 28 #eeee#EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE#ee#e#   upper terrace, reachable
 29 #eeee############EE#T################ee#e#    (20,29) is a CutTree
 30 #eeee#eeeeeeeeee####eeeeeeeeeeeeeeeee#ee#e#   nothing below here is reachable
 37 ############cccc#cccccccc#cccc############    the Route 5 connection row
```

Cerulean is split into terraces. The ways down are a cut tree at (20,29) and the corridor at x=37/38,
which is reachable **only through `CeruleanTrashedHouse`'s back door**. `policy.rs:1353` already says
so in as many words — *"The trashed house is the only way between Cerulean's split terraces: its back
door lands in the Route-5 terrace"* — so the scripted route knows exactly what the LLM turn never
mentions. Meanwhile `read_route(to=Route5)` kept answering `Connection at (23,37)`, `(13,37)`,
`(20,37)`: a real row, on a terrace the player cannot stand on. The model filed six escalating
reports, wrote `BLOCKED:` into its plan four times, and ended up asking a developer to inspect the
save state.

Same shape, smaller bills: `RockTunnel1F` (turn 672 — the south exit is genuinely via B1F, and the
agent is right), `CeladonCity` (turn 847 — out of the gym into a terrace whose exit is a Cut tree).

## R2 — One crossing per adjacent map, and the model cannot ask for another

`actions()` emits the **nearest** reachable connection tile per neighbour. `connection_action(to_map,
to_position)` exists for a specific landing and `PolicyStep::enter_at` uses it, but `choose_action`
resolves ids by string equality against `actions()`, so a synthetic `Map:x,y:Connection` is refused.
The model has no way to say "same map, different door".

**Route 13 → Route 14, turns 988–1002.** Probe of `issues/turn-992/state.gbst`:

```
  5 #eee#ee###############
  6 #eee#eeeeeeeeeeeeSEEE@C    player at (20,6); S is Cooltrainer M2 at (16,6)
  7 #eee###########ee#####
```

`reachable tiles: 14`. Route 14's east edge is open at rows 0, 1, 2, 4, 6, 8 and 10
(`data/maps/headers/Route14.asm`: `connection east, Route13, ROUTE_13, 0`); row 6 is a six-tile
pocket whose only way west is *through* a trainer sprite. Rows 4 and 8 are the real path. The model
walked back to Route 13, walked east, came back in — and landed in the identical pocket, because
Route 13 also offers only its nearest crossing. Its report ("menu starvation") is an accurate
description of a two-row menu.

## R3 — A movement timeout that reports itself as a routing failure

`agent.rs:2487`: the 60 s `MAX_MOVEMENT_SILENCE` bound aborts with
`OverworldActionAbortedReason::NoRoute(destination)`. The walk did not fail to find a route, it
failed to arrive. The model reads *"there is no route to the warp to Route8Gate"* and goes hunting a
pathfinder bug that is not there.

**Route 8 gate — reproduced deterministically** from `at-lavender.bin` with
`[enter(Route8), enter(Route8Gate), enter(Route8), enter(Route8Gate)]`:

```
StartedOverworldAction { Warp { Route8Gate, (5,3) } }
OverworldActionAborted { reason: NoRoute(...), at: (10,9) }     ← for ever
```

Entering the gate from a distance works. Standing on the exit tile — padded (9,9), raw (8,9), which
is where the gate's east door puts you — it never does. The action's route is empty, so `actions()`
emits step-off/step-back; the agent re-derives the route from `actions()` **every tick** and
therefore only ever presses `route[0]`; so it oscillates (9,9) ↔ (10,9) until the 60 s bound fires.
That is the left-and-right shuffle visible on the page.

Driving the joypad by hand shows the tile is worse than awkward: 200 ticks holding Left at (9,9)
facing Left never warps. `is_step_on_warp((9,9))` is false (raw tile id 44;
`TileSetId::Overworld.warp_tile_ids() == [27, 88]`), so pokered needs `ExtraWarpCheck` →
`IsWarpTileInFrontOfPlayer` to pass and it does not.

✅ **The open question is answered (2026-09-03), and the answer is that (9,9) is not a door.**
`ExtraWarpCheck` dispatches on the overworld tileset to `IsWarpTileInFrontOfPlayer`, which reads the
tile in front against `data/tilesets/warp_carpet_tile_ids.asm` — a table keyed on the direction
faced, not on the tileset. Probed off the live emulator:

```
(9, 9)  raw $2C   west $17  north $3A  south $39  east $2C   -> no list matches, any facing
(9,10)  raw $39   west $4B (facing-left list) -> holding Left warps in, first try
```

So (9,9) can never be triggered from Route 8 by any approach: it exists so that leaving the gate's
warp 3 lands the player there. (9,10) is the door, and the recipe is **one held button**, either as
the last step of the walk onto it or as a bump into the gate wall from on top of it — the collision
arm of `CheckWarpsNoCollision` sends a collision *on a warp tile* straight to `CheckWarpsCollision`.
Neither half needed a step off and back.

Related, same map family: `Route7Gate` (turn 1038). Its four warps are `LAST_MAP`
(`data/maps/objects/Route7Gate.asm`), so all of them resolve to `Route7` and the menu shows three
near-identical `Warp → Route7 (18,9) / (11,10) / (18,10)` rows. **The agent's resolution is correct**
— checked against `data/maps/objects/Route7.asm`, where the gate has four warps on both sides of the
building and Saffron is a *connection* past it — but nothing tells the model which side of the gate
each door is on, and it filed a bug saying the Saffron warp was missing.

## R4 — Two coordinate conventions in one turn, and no validation

An action id's coordinate is the tile the **player stands on**. `use_field_move`'s `target` is the
tile the **thing is on**. Nothing says so, and `resolve_field_move`'s `UseItem` arm
(`src/llm/tools.rs:439`) passes `target` straight through with no check that anything is there.

**Route 16 Snorlax, turns 1019–1022, three wasted turns.** The menu row was
`Route16:27,10:Snorlax`; the Snorlax was at **(26,10)**. The model played the Poké Flute at (27,10)
three times, once from (39,10) and twice from (28,10). Each was answered `"Accepted. The agent is
carrying it out now; the next turn will tell you what happened."` and each did nothing at all. It
recovered only by calling `read_map`, which returned
`{"name":"Snorlax","position":{"x":26,"y":10}}`, and then targeted (26,10) and it worked first time.

So: the agent had the right position throughout, the turn showed the model a different one, and a
target with nothing on it was accepted silently.

The same clash makes `interact`'s refusal read `"Can't reach trash can at (23, 30)"`
(`agent.rs:4195`) whatever the model aimed at — `FieldMoveRequest::Interact` maps to
`FieldMove::CheckTrashCan`, and the message never learned the tool got a second caller. Three of the
ten Cerulean reports quote that line as proof the map model is broken.

## R5 — The plan's stale ids, its unnamed delete, and its unbounded retry

**Turn 1280 made 35 consecutive failing `todo_set`/`todo_complete` calls on id 5**, every one
answered `"There is no TODO 5. The list is in the turn you were just sent."` It burnt the whole turn
and only stopped when `GB_MAX_TOOL_STEPS` (16 on the live ConfigMap; a step may carry several
parallel calls, which is how 16 became 35) cut it off. Four defects compound.

**R5a — an unknown id silently *creates*.** `todo.rs:140`'s `(Some(id), Some(text))` arm is
"forgiving on purpose": an unknown id appends instead of failing. The model was using `text` as a
command word:

```
todo_set {"id": 5, "text": "Delete"}
  → There was no TODO 5, so this went on the end. Added TODO 12: Delete
todo_set {"id": 5, "text": "In Saffron. Silph Co. (warp 19,22): …"}
  → There was no TODO 5, so this went on the end. Added TODO 14: …
```

**Two of the five items in the live plan exist only because of that branch**: item 12 is literally
the word `Delete`, and item 14 is a byte-for-byte duplicate of item 11. Item 13 is a third near-copy.
`MAX_ITEMS` is 5 and **none of them is done**, so `add` will now refuse every new item with "Your
plan is full" — and the plan is the only thing the model writes that survives a compaction. The
forgiveness that was meant to rescue a stale id has instead destroyed the list.

**R5b — delete has no name.** It is spelled "call `todo_set` with an `id` and no `text`", an overload
of the setter. The model tried three encodings of it in one turn — `{"id":5,"text":"Delete"}`,
`{"id":5,"text":""}`, `{"id":5}` — plus `todo_complete{id:5}`, and only the last two are the intended
one. The sentence that names it correctly exists (`todo.rs:180`, "drop the one you no longer mean to
do with `todo_set` (its `id`, no `text`)") but only appears in the *plan-is-full* refusal, which the
model had not seen yet.

**R5c — the failure hands back nothing, and nothing bounds the repeat.** *"The list is in the turn
you were just sent"* points the model back at the message it has already mis-read, with no new
information; so the cheapest next move is to try again. This is the same shape as `press_buttons`
before it was withdrawn: **a call that fails cheaply and identically will be repeated until
something else stops it.**

**R5d — why it believed in TODO 5.** TODO 5 was real: *"Silph Scope got. Next: ride elevator/B2F
warps out to Game Corner…"*, added and completed several hundred turns earlier. `todo_complete` only
marks `done`; `add`/`trim_to_cap` then silently drop the oldest **done** item to make room, so the id
vanished with no event. Meanwhile the plan message is append-only by design — every stale copy stays
in the history — and compaction summarises older turns.

**R5e — and why it kept saying *five* in particular.** ⚠️ Found while implementing W8, and it is the
proximate cause the other four only enabled. Both `id` schemas declared `"maximum": MAX_ITEMS`, which
is 5. Ids come from a counter that only goes up and never reuses a number, so the plan held 10, 11,
12, 13, 14 while the schema said the largest legal id was 5. Every id the model could see was
out of range, and 5 was the biggest one it was allowed to name. It had used `id: 10` successfully
earlier in the run, so the endpoint does not enforce the schema — but a model that reads it will try
to obey it, and this one did, thirty-five times.

---

## The work

Ordered by turns burnt in the run, and all ten shipped on 2026-09-03. Each entry keeps what it was
for and adds what was actually done, because in three of them (W2, W5, W9) the fix that shipped is
not the one the entry proposed. W10 has no "what it was for": it was found while the run was still
going, after W1-W9 were written.

### W1 — `read_route` must not name a tile the player cannot reach — **done 2026-09-03**
*65 turns in one map, ten issue reports.*

`observe::route_from` checks the second hop's leaving tile — the only coordinate in a route that is
on ground the player is standing on — against `reachable_tiles()`, and hangs
`reachable_from_here: false` on that hop. `tools::route_answer` turns that into a `warning` saying
the route is correct and cannot be *started*, what the reachable region ends on
(`MetaTileMap::boundary_blockers`), that the graph is built from map headers and will go on
disagreeing with the menu, and that what joins two parts of one map is usually a door.

The route itself is still answered: it is the right answer to "which way is Route 5", and
withholding it would be a second lie.

Tests: `tools::a_route_off_a_terrace_the_player_is_not_on_says_it_cannot_be_started`, which rebuilds
the deployed graph by observing Cerulean's lower terrace and then its upper one, and
`tools::a_fenced_in_map_names_the_neighbours_it_cannot_reach` across all three committed states.
Watched failing first by forcing the flag to `true`, which also re-printed the run's own answer,
`Connection at (18, 37)`.

### W2 — A crossing the model can choose — **done 2026-09-03**
*15 turns on Route 14, and the generic escape from W3.*

Both halves, and neither is the expensive one the entry feared:

1. `resolve_overworld` falls back to `connection_action(to_map, to_position)` for an id naming a
   `Connection` tile `actions()` did not mint. The id is re-minted from the tile through
   `overworld_id` rather than parsed out of the string, so the resolver and the menu cannot drift.
2. `actions()` is **unchanged** — it still emits the nearest crossing per adjacent map, so
   `route_toward`, the grind and the scripted run's timing are untouched. What changed is the row's
   prose: it now names the other reachable landing groups by id (`MetaTileMap::crossings`, which
   floods a border strip into the runs that are genuinely different decisions). Ranking them by the
   destination's block data was the expensive half and turned out not to be needed; listing them is
   enough, because the model can then pick.

⚠️ An unreachable crossing is still refused, or the fallback would put the model straight back into
the failure W1 exists to stop.

Test: `tools::a_crossing_the_menu_did_not_offer_can_still_be_chosen`, on the Route 14 fixture with
the player moved out of the pocket and onto the road, where several openings are reachable at once.

### W3 — Say when the player is fenced in — **done 2026-09-03**
*Covers Cerulean, Celadon and Route 14 in one line of turn text.*

`MetaTileMap::unreachable_connection_targets` is the trigger and it is a *named neighbour* rather
than a tile count, because a tile count is true of a healthy map too. Water-only neighbours are
skipped: that is the Surf gate, and it is the mistake the `Blocked here: Water` line was deleted
for. The line says how much of the map is reachable, which neighbours are not, that `read_route`
will still name them and why, and then — the half that was missing every single time — that the way
between two parts of one map is a door, listing the warps that *can* be reached. In Cerulean that
list opens with `CeruleanCity:10,12:Warp` and `CeruleanCity:28,12:Warp`, the second of which is the
trashed house.

Rock Tunnel 1F is silent, correctly: it has no map connections and its south exit genuinely is the
B1F ladder, so the agent was right there all along.

Test: `prompt::a_map_the_player_is_fenced_in_on_says_so_and_says_which_way_out`.

### W4 — Stop lying about why a walk ended — **done 2026-09-03**

`OverworldActionAbortedReason::DidNotArrive`, a unit variant, replaces `NoRoute` at the
`MAX_MOVEMENT_SILENCE` bound and nowhere else — the two other `NoRoute` sites are genuinely "the
action is gone". `MAX_MOVEMENT_SILENCE` moved to module scope so the sentence can quote it: *"the
walk was given up after 60 seconds of game time without getting there"*, beside the `at` the event
already carried. No em dash: this is one of the four agent-generated string sites.

Test: `agent::a_walk_that_never_arrived_does_not_claim_there_was_no_route`.

### W5 — The Route 8 gate doorstep — **done 2026-09-03**

The open question above is answered, and both halves the entry asked for are in:

1. `MetaTileMap::warp_trigger` transcribes `IsPlayerStandingOnDoorTileOrWarpTile` and
   `ExtraWarpCheck` — including its dispatch, where SS Anne 3F and four named maps override their
   tileset — and returns `StepOn`, `HoldDirection(dir)`, `Impossible` or `Unknown`. `actions()` emits
   a **one-button** route for a `HoldDirection` entry the player is standing on, uses the cartridge's
   own direction as `enter_dir` rather than guessing from the map edge, and drops an `Impossible`
   row.
2. The comment about the route being re-derived every tick, and only its head pressed, is on
   `AgentState::OverworldMovement`.

⚠️ **Two guard rails, and the first draft needed both.** A dud row is given up **only when another
warp to the same map is known to work** — counted over triggerable warps, not over warps that exist,
because Cerulean's badge house has two doors, both of which looked impossible, and each was dropped
in favour of the other, sealing the house. And `Unknown` exists because
`_GetTileAndCoordsInFrontOfPlayer` reads the on-screen tilemap, so a warp on the map edge faces the
**border block**, which `raw_tile_ids` does not hold: the S.S. Anne gangway, Rock Tunnel's north
mouth and that badge house are all there. Across every committed fixture the predicate now drops
nothing at all; the only `Impossible` in the game that anything has found is Route 8's (9, 9).

Test: `stalls::the_route_8_gate_can_be_re_entered_from_its_own_doorstep`, the deployed sequence — in,
out, and straight back in — watched failing first. ⚠️ It waits for the landing *square*, not for the
map: a warp changes the map id a few frames before `wXCoord`/`wYCoord` settle, and a bare map test
reads a coordinate on neither side of the gate.

### W6 — Refuse a field move aimed at nothing — **done 2026-09-03**
*Three turns on Route 16; the mechanism is general.*

`resolve_field_move`'s `UseItem` arm refuses a target that is `Empty`, `Grass`, `Water` or off the
map, and refuses one the player cannot get next to and face — which is the precondition every one of
these items actually reads. The refusal says which convention is which and names whatever sprite is
*beside* the square aimed at, which is the recovery `read_map` sold the run for a whole round trip.

⚠️ The test is "is it open ground", not "is it a sprite": the Card Key acts on a door, which is an
`Obstacle` tile with nothing standing on it.

Then the convention gap itself. `Sprite` rows whose target gets asked for (a boulder, Snorlax) and
every `Switch` row now say both squares — where you stand and where the thing is — and only those
rows do, since a person the agent walks to needs no coordinate at all.
`AgentState::CheckingTrashCan`'s unreachable message names the tile it was aimed at instead of
always saying "trash can". ⚠️ **W10 then removed the caller that made that wrong**, so this survives
for the scripted policy alone.

Tests: `tools::a_field_item_aimed_at_open_ground_is_refused_and_says_what_is_beside_it`. ⚠️
`an_item_the_game_will_never_use_is_refused_here_instead` had to move its target off bare floor,
or it would have started passing for the new reason instead of the one it is about.

### W7 — Gate doors need a side — **done 2026-09-03**

`tools::door_side` labels a warp row by which side of the map its door is on, on whichever axis it
is further off centre, and only where a destination map is offered more than once — an ordinary
building's double-wide front door is two warp tiles onto one square and `warp_targets` already
collapses it. The turn adds one line where every warp on the map leads to one other map at several
different landings: which door you take decides where you come out, and **nothing beyond that map is
behind a door here**. True of a gate house, where the onward city is a connection past it, and true
of a cave floor with two ladders down.

Verified against `data/maps/objects/Route7Gate.asm` (warps at x=0 and x=5, all `LAST_MAP`) and
`Route7.asm` (the gate's warps at x=11 and x=18, either side of the building).

### W8 — The plan: name delete, stop guessing, stop repeating — **done 2026-09-03**
*One whole turn, and the live plan is 60% junk and at its cap.*

⚠️ **There was a fifth cause under the four in R5, and it is the one that actually picked the number
5.** `todo_set` and `todo_complete` both declared `"maximum": MAX_ITEMS` on `id`
(`tools.rs`, added with the `maxLength` on `text`). That confuses **how many items the list holds**
with **what they are called**: an id comes from `next_id`, a counter that only goes up and never
reuses a number, so a plan revised a dozen times holds 10-14 while the cap is 5. The schema was
telling the model that every id it could see was out of range, and 5 was the largest it allowed.
That is why 35 calls all named 5 and none named 12.

What shipped, all five:

1. **`maximum` is gone from both `id` schemas.** There is no upper bound to state; the list is the
   authority on which ids exist.
2. **An unknown id no longer creates.** `todo.rs`'s `(Some(id), Some(text))` arm refused to be
   "forgiving on purpose" any longer — that forgiveness is what produced `Added TODO 12: Delete`.
   It refuses, and every "there is no TODO n" now **names the ids that do exist** in place of "The
   list is in the turn you were just sent", which handed back nothing.
3. **`todo_delete` exists.** It parses to the same `TodoCall::Set { id, text: None }` arm, so the old
   overload still works for a resumed run imitating its own history, and is simply no longer the only
   way in. Cost: **324 bytes on every decision kind**, argued at the tool-array ceiling.
4. **A repeat is bounded.** `Worker::apply_todo` keeps the plan calls the list refused this turn and
   answers an identical one without running it, because `GB_MAX_TOOL_STEPS` bounds round trips and a
   step may carry any number of parallel calls — which is how 16 became 35.
5. **An eviction is named.** `add`'s answer says which finished item was squeezed out to make room,
   at the moment it happens, so a vanished id has an event.

Plus the system prompt gained a "the numbers are names, not places" bullet, and `Conversation.tsx`
renders `todo_delete`.

Tests: `todo::an_id_that_is_not_on_the_list_changes_nothing_and_the_answer_names_the_ones_that_are`,
`todo::the_item_squeezed_out_to_make_room_is_named`, and
`llm_policy::a_refused_plan_call_repeated_in_one_turn_is_not_serviced_twice`, which is turn 1280
shrunk. All three were watched failing against the old code first.

⚠️ **The live run still needs a hand, and none of this reaches it until a deploy.** Its plan holds
five items, none done, of which one is the word `Delete` and two are copies, so nothing new can be
added. Items 12 and 14 want deleting.

### W9 — Vermilion Gym, 87 turns — **done 2026-09-03, the non-cheating half only**
*The largest single sink in the run, and no issue report was filed about it.*

**Alex's call, 2026-09-03: nothing from RAM.** `GameState::trash_cans` stays unread by the prompt.
What the turn says is mechanism a player learns by playing, keyed on the map having
`HiddenObject::TrashCan` and on nothing else:

- a wrong guess at the *second* switch resets both locks **and moves the first one**, so every bin
  already eliminated goes back into the pool. Checked against
  `engine/events/hidden_events/vermilion_gym_trash.asm`, whose failure arm is
  `ResetEvent EVENT_1ST_LOCK_OPENED` and then `Random / and $e / ld [wFirstLockTrashCanIndex], a` —
  the doc's original wording had only the second lock resetting.
- `choose_action` takes three more ids in `then`, so a sweep is four bins per request rather than
  one. That has been in the tool schema throughout and was used for none of the fifteen; the one
  measurement this repo has about where a nudge lands is that the top of the turn works and the
  bottom of the system prompt does not.

Test: `prompt::the_gym_bins_say_what_a_sweep_costs_without_saying_where_the_switches_are`, whose
second half asserts the turn is **byte-identical** with the puzzle solved and unsolved. That is the
line that keeps the decision a decision.

### W10 — `interact` is gone, and with it hidden items — **done 2026-09-03**
*85 minutes and ~50 requests in one afternoon; the largest single stall of the run, and it is still
the same run.*

Found by reading `kubectl logs` while the 2026-09-02 run was **still going**, on build `06ebda5` with
none of W1–W9 deployed. Between 12:07 and 13:31 BST it made ~50 `use_field_move interact` calls
across Silph Co 8F, 10F and 11F, sweeping coordinates one at a time — `(14,2)`, `(14,3)`, `(14,4)` …
`(14,16)`, then back down. Each burned 60 s of game time *and* a full request against a ~60 M-token
history. It filed `report_issue` on turn 321 and then warped out on its own.

**The mechanism.** `FieldMoveRequest::Interact` mapped onto `FieldMove::CheckTrashCan`: walk to the
square, face it, press A. Nothing there means no text box, so the driver's `checked` never flipped
and it mashed A until `DRIVER_ESCAPE_SILENCE` fired at 60 s with *"got no answer from the game"* —
which is R3's disease in a second place, a malfunction reported where the truth is "there is nothing
there".

⚠️ **And the model had it filed under the wrong verb entirely.** Its report reads: *"standing at
(8,1) facing (8,2), an open floor tile, `use_field_move interact target=(8,2)` has now failed 4 times
in a row … the player visibly never moves … Expected: step to (8,2)."* It had adopted `interact` as a
**movement primitive** and concluded walking was broken on that floor. It was not: the warps and the
conversation with Rocket2 it says still worked are the proof. No refusal text unlearns that.

⚠️ **None of W1–W9 would have helped.** W6 added the aimed-at-nothing refusal to `use_item` only, and
`Interact` passed its target straight through; W6's message fix is on the *unreachable* arm, and here
the route existed. This is a sibling of R4 that this document did not identify.

**Why removal rather than a check.** `interact` exists *for* hidden items, which are invisible in
`meta_tiles` by construction, so `use_item`'s "is anything there" test cannot be reused. And there is
nothing to protect: all 212 rows of `data/events/hidden_events.asm` that yield an item yield a
consumable — PP Up ×5, Ultra Ball ×4, Rare Candy ×4, Nugget ×4, Max Elixer ×4, Full Restore ×4,
Elixer ×4, and 2 Moon Stones — with **no key item, no HM and no TM anywhere in the table**. The Mt
Moon Moon Stone *is* hidden (`MT_MOON_B2F`, y=18 x=12) and is still optional. No guide chapter
mentions a hidden item.

What went: `FieldMoveRequest::Interact` and its parse arm, schema line and the `facing` property only
it used; `PolicyStep::SearchHiddenItem` and its driver and baseline; `aides::{hidden_items,
HiddenItem, hidden_item_flag, pick, bag_quantity}` and the two ROM tables they crossed;
`MetaTileMap::hidden_items`; the H4 leg `can_collect_a_hidden_item` and the fixture
`postgame-hidden-item.bin`; three ROM cross-check tests; and `probe_town_hidden_item_reachability`.

What stayed: `FieldMove::CheckTrashCan`, `AgentState::CheckingTrashCan` and every `MetaTile::Switch`
row — the gym bins, the drink machines, the Mansion statues, the Game Corner poster and Bill's cell
separator. Four of those five are hard progression gates and the guide names two of them.

Two consequences worth their own lines:

- **H5 re-roots on H3.** `postgame-hidden-item.bin` is deleted and the three legs that read it now
  read `postgame-itemfinder.bin`, which differs by one Escape Rope and a few tiles of Route 11 and is
  the more generous of the two: it ends with a **free bag slot** where H4's ended full.
- **The PP Up is debug-seeded now**, because every PP Up in the game is a hidden item. ⚠️ It is
  seeded *after* the Ether is consumed and the test waits for the bag to actually give the row back:
  the bag is at its 20-slot cap, the PP assertion lands a few ticks before the item is removed, and
  seeding on it finds a full bag. What the leg tests — one ROM routine, two observables — is
  unchanged.

⚠️ **The 60 s driver bound was left alone, deliberately.** I offered to shorten it and then did not:
`DRIVER_ESCAPE_SILENCE` is one net across all nineteen drivers with a documented argument (a
multi-item mart trip, a PC deposit, a Fly animation all run without a poll), the only caller that
made it fire pathologically is now gone, and special-casing one state would re-time everything for no
remaining benefit.

Reclaimed: **370 bytes** on every Overworld turn, the largest single drop the tool-array budget has
recorded.

---

## What is still open

- ⚠️ **The live run still needs a hand, and none of this reaches it until a deploy.** Its plan holds
  five items, none done, of which one is the word `Delete` and two are copies, so nothing new can be
  added. Items 12 and 14 want deleting. (W8's entry says the same; it is repeated here because it is
  the one action item left in this file.)
- `read_route` is annotated but still routes out of whichever section of the current map the graph
  happens to have observed, rather than out of the one the player is standing in. Saying so was
  cheap and correct; making `shortest_path` section-aware for the *starting* map was not attempted.
- `actions()` still emits the nearest crossing per adjacent map and the others live in the row's
  prose. Ranking landing groups by the destination's block data — W2's expensive half — was not
  needed and was not done.

## Scope notes

- The previous run (`run-20260901-133042`, 8455 turns) filed 47 reports, about 25 of which are one
  story: the Rocket Hideout Lift Key elevator. It ran mostly on a build older than `cebf2a5`, and the
  current run walked the Hideout without trouble, so nothing here is planned against it. Worth a
  separate check before assuming it is gone.
- The battle-script report at turn 965 (a script countermanding the model's switches) is already
  fixed on the working tree by `9bbd1da`, which the deployed build predates.
- Abort reasons across the whole run: `the game stopped you to say something` ×83, `a battle started`
  ×65, `the game took over` ×25, `there is no route …` ×9. The NoRoute cases are rare but each one
  starts a two-turn ping-pong and sends the model looking for a bug.

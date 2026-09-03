# Deployed-run defects — routing, gates, targets and the plan

What the deployed run of 2026-09-02 (`run-20260902-215720`, `z-ai/glm-5.3-flash`, build `cebf2a5`)
actually walked into, why, and what to do about it. Written 2026-09-03 against a run in flight at
turn ~1287, 60.3 M prompt tokens, 17 issue reports filed by the model itself.

**Status:** six root causes, nine work items, ordered by turns burnt. **W8 is done (2026-09-03);
the other eight are not started.**

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

⚠️ **Open question.** The first entry, from (51,12), succeeded, targeting the *other* door tile
(9,10). Whatever approach satisfies `IsWarpTileInFrontOfPlayer` has not been identified, and W5
cannot be written until it is. `home/overworld.asm`'s `CheckWarpsNoCollision` is the code to read:
after the standing-on-a-door-tile test it also requires a directional button to be **held** at the
moment the step completes.

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

Ordered by turns burnt in the run. W1 and W4 are both small and between them account for most of the
waste.

### W1 — `read_route` must not name a tile the player cannot reach
*65 turns in one map, ten issue reports.*

When the leaving tile of the first hop is outside `reachable_tiles()`, say so, and say what is in the
way: the blocker class (cut tree, ledge, water, sprite) and — the part missing every single time —
that the way on may be a **warp through a building** rather than a walk. `observe::route` already has
the `MetaTileMap`; the check is one set lookup.

Test: a new `probe_split_maps` case per known split map (Cerulean, Celadon, Route 14) asserting that
`read_route` from the wrong terrace never names an unreachable tile.

### W2 — A crossing the model can choose
*15 turns on Route 14, and the generic escape from W3.*

Two halves, and **the first is worth shipping alone**:

1. `resolve_overworld` falls back to `connection_action(to_map, to_position)` when an id names a
   coordinate `actions()` did not mint, so the model can ask for Route 14 row 8 by name.
2. Emit one `Connection` row per reachable *landing group* rather than one per neighbour map, and
   rank them so the default is not a six-tile pocket. This needs the destination map's block data,
   which is in ROM and reachable through `MapMetadataCache`, and is the expensive half.

### W3 — Say when the player is fenced in
*Covers Cerulean, Celadon, Rock Tunnel 1F and Route 14 in one line of turn text.*

The turn already carries a "Blocked here" note about cut trees. Make it a statement about *this*
terrace: how many tiles are reachable, what the boundary is made of, and that a warp may be the way
out. Cerulean's answer was `Warp → CeruleanTrashedHouse (2,7)`, which sat in the model's own menu for
forty turns while it filed reports.

### W4 — Stop lying about why a walk ended

Split `OverworldActionAbortedReason::NoRoute` into `NoRoute` (the action genuinely vanished from
`actions()`) and a new `DidNotArrive` for the 60 s bound, whose text says the walk was abandoned
after 60 s of game time without arriving, and where it got to. Cheap; stops a stall being read as a
pathfinder bug.

⚠️ Keep the wording free of em dashes: `AgentEvent`'s `Display` is one of the four agent-generated
string sites the house rule covers.

### W5 — The Route 8 gate doorstep

Blocked on the open question in R3. When it is answered, two things need fixing:

1. `actions()` must emit an approach the cartridge accepts when the player is already standing on the
   warp tile. Step-off/step-back is not it.
2. **The agent re-derives the route every tick and so only ever presses `route[0]`.** Any recipe
   whose correctness depends on its own tail is inert today. That is a general property worth a
   comment on `AgentState::OverworldMovement` whatever else changes.

Promote `probe_route8_gate` into `stalls.rs`, and watch it go red before the fix — per that file's
own ⚠️, a case added without seeing it fail may be asserting nothing.

### W6 — Refuse a field move aimed at nothing
*Three turns on Route 16; the mechanism is general.*

In `resolve_field_move`'s `UseItem` arm, check the target holds the sprite or object the item acts on
and is adjacent-reachable, and refuse in the voice `PushBoulder` and the HM gates already use: say
what is at the target instead. Then close the convention gap that caused it:

- `Sprite` / `Switch` / Snorlax menu rows name **both** coordinates ("stand at (27,10), facing the
  Snorlax at (26,10)").
- `interact`'s refusal names the object it was aimed at rather than always "trash can".

### W7 — Gate doors need a side

Where several warps on one map resolve to the same destination map, distinguish the rows by which
side of the current map they are on ("the west door", "the east door"), and say once on a gate map
that both doors return to the same route and the onward city is a connection past it.

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

### W9 — Vermilion Gym, 87 turns
*The largest single sink in the run, and no issue report was filed about it.*

The model swept all fifteen bins by hand, one request each, and the puzzle reset twice.
`GameState` already carries `TrashCanPuzzle { first_target, second_target, first_opened,
second_opened }`, read from RAM in `pokemon/mod.rs`. **Whether to hand those over is a call for
Alex** — it is state a player cannot see. The non-cheating half is worth doing either way:

- Tell the turn that `choose_action`'s `then` takes three more ids, so a sweep costs four bins per
  request rather than one.
- Say that a wrong second guess re-rolls **both** locks, so the bins already checked stop being
  eliminated. The model worked this out from the resets, several sweeps in.

---

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

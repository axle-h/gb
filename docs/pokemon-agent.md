# The Pokémon agent, the policies, and what they report

Read before touching `poke-agent/src/pokemon/{agent,policy,text,tile_map,actions}.rs`, `AgentEvent`
or any `Display` it goes through, or the SPA's `useEventStream.ts`. Each line names the function or
constant carrying the argument. Rules a test already pins are not here; these are the ones a reader
can break in silence.

## The agent loop

- `poll_policy` is the single seam every decision goes through and it resets the clock the watchdog
  reads; calling `service_tools` from a new site makes the watchdog believe the run is wedged.
- The emulator never pauses while the model thinks, so a pause spanning a tool call deadlocks.
- The watchdog is blind to a policy that answers `None` for ever, because `since_last_policy_poll`
  resets on every poll whatever the answer. Guard that class on actions taken, never on silence.
- One agent tick is 20 ms of game time and `update` coalesces rather than catching up, so
  `host::MAX_CATCHUP` (250 ms) must stay under the one step (267 ms) a walk has to turn a corner in.
- `DidNotArrive` and `NoRoute` are opposite diagnoses. Reporting a time bound as `NoRoute` sends
  people hunting a pathfinder bug while standing two tiles from the warp they asked for.
- Silence bounds drivers (`DRIVER_ESCAPE_SILENCE`, `MAX_MOVEMENT_SILENCE`), not per-state tick
  counters: a state torn down and rebuilt starts a counter over. The driver hatch measures from the
  game's last answer, and only a landed shove may reset it.
- "There is no route" is a claim about a map people stand on. Past `MAX_ROUTE_LOST_TICKS`,
  `row_blocked_by_people` asks whether the row returns with everyone put back `underfoot`; only a yes
  buys `MAX_ROUTE_BLOCKED_TICKS`, and boulders are excluded because a rock will still be there.
- `AgentState::open_overworld_action` is the one list of states carrying a row, and every door out of
  any of them closes it. A new state that borrows a walk belongs in that list the day it is written.
- The Surf mount is the only driver entered from the middle of a walk, so it is the only one that
  hands a walk back — and for a water crossing it *finishes* the walk.
- The route is re-derived from `actions()` every tick and only `route[0]` is pressed, so no recipe may
  depend on its own tail.
- A battle menu outlives the choice made on it by a frame or two, so `BattleState::AwaitingPolicy`
  goes back to reading once the menu is off the screen, and only on a tick the policy did not answer:
  the scripted policies answer at once, and their timing is the golden replay.
- A catch's battle ends when the naming screen closes, not when `wIsInBattle` clears, so
  `BattleEnded` and the reader's last box are emitted there. Every tree cut on the map is forgotten
  with it: a battle reloads the map and they have all grown back.

## Closed loops under A

Almost every jam is a menu the agent's own A press re-enters with the cursor untouched.

- A menu the agent did not open is closed, not confirmed (`MENU_HANDOVER_TICKS`), and a rule running
  at every text box trusts the screen, never the lingering `wTextBoxID` (`MenuEvidence`).
- A party menu a conversation opened is answered on the tick it appears, and only an in-game trade
  has an answer: none of the three scripts that open one resets `wCurrentMenuItem`, so an A-mash acts
  on wherever the cursor was left.
- Each gated loop — the PC menus, the START menu, a TM outside its learnset, a refused key item, a
  mart open while the policy thinks — is a frame-timing change only `full_playthrough` can price.
- A teach whose move-to-forget is declined is over. The bag reopens where the use started, so a
  driver that reads "not done yet" and begins the chain again asks the same question for ever.

## What the map layer will and will not offer

- A warp entry is not a door. The held-direction path is unavailable while surfing and while standing
  on a warp the cartridge put you on, and both conditions live in two places — `actions()` builds the
  route, `OverworldMovement` tests for a border warp before consulting one. Fixing one half does
  nothing.
- `map_uses_runtime_blocks` lists every map `ReplaceTileBlock` rewrites. A map missing from it is
  offered rows through closed doors, and no finished-game fixture can show that.
- The floor menu is up when the screen says so, never when `wListMenuID` does: it still reads the
  floor list long after the menu closed, so a second ride navigates a menu that is not there.
- A lift's doors lead where the live `wWarpEntries` says, the floor it was entered from until the
  panel picks another; the ROM's table is written for one floor (`with_live_exits`).
- A map script can cancel a warp the tiles call fine. `map_warp_gate_specs` is deliberately tiny:
  withholding a real door is how a floor loses its only exit, so only a refusal proved in the
  cartridge's own source goes in. `WarpTrigger::Unknown` is never dropped either — unsure is not no.
- `actions()` emits one crossing per adjacent map *per kind*, land and water: one row per edge
  perturbs the scripted run's timing, and collapsing the kinds hides every neighbour whose land
  crossing is nearer than its water one.
- A person or item with only tall grass beside it is reached from the grass, and only then:
  floor is tried first, so no row that already existed moves.
- A `Pace` row stands only where the cartridge rolls an encounter off grass: any floor of an indoor
  map outside the forest tileset, and water whose bottom-right tile is `$14` (`paces_on`). A shore
  square is water that never rolls.
- A vending row names the drink it buys, one drink per machine, because the roof girl trades a
  different machine for each of the three and the menu opens on the cheapest.
- A walk to a counter waits on whoever is pacing in front of it (`BLOCKED_TICKS`) rather than
  reporting no route; the Game Corner's prize room has a gambler who walks across all three.
- A step onto water costs `SURF_MOUNT_COST`, so `bfs_from_player`'s `dist` is a price, not a step
  count; `wander_action` is the one caller that means steps.
- A square the cartridge refuses to let you stand on is not in the block map, so routing over one
  loops rather than fails. `turned_back_tiles` learns them from the cartridge's own behaviour and
  clears them on map re-entry, because "refused, given what I am carrying" expires. A hard-coded
  table of coordinates was tried first and is the wrong shape.
- A refused boulder push is refused in silence, so all three seams ask `boulder_push_refusal` first —
  the solver, the tool, and `PushingBoulder` on every tick. `reachable_tiles` includes a wall touching
  floor, so a stand tile must be walkable as well as reachable.
- The boulder code routes with `push_search`, not `bfs_from_player`: a warp tile is standable there,
  and Victory Road 1F is unsolvable otherwise.
- A goal row's search is memoised on `PlanKey`, or `actions()` runs a capped BFS over boulder layouts
  fifty times a second and the agent drops from 48x real time to 1.9x. The key is exact rather than
  hashed, and carries a reachability component and a walkability bitmap rather than anything that
  moves when an NPC steps.
- A boulder goal is bounded on progress (`MAX_PUSHES_WITHOUT_PROGRESS`), never on a shove count. A
  switch keeps its boulder and a hole swallows it, so one completion test for both stalls the floor.
- The turn must not tell a model that a Strength floor is lost — two versions of that sentence have
  shipped and been reverted, and the tombstone is in `prompt::situation`. Say what is measured: these
  are the legal pushes, and leaving the map puts every boulder back.
- `position_settled` is false while a map transition is in flight and nothing may read
  `player_position` while it is — every warp, and one tick per northward or westward connection where
  the coordinate underflows to 255 and the bounds clamp makes it a plausible square at the opposite
  edge. Hold only the arm that lies, not the whole tick, or a leg's wild encounter is re-rolled.

## The coverage oracle and the cheats

- `Silent` — chosen, outcome never reported — fails a walk without being a defect, and the two counts
  stay apart.
- `REPEAT_IS_A_DEFECT` is 10: a gate is worth one try per pass over a map, because the thing that
  opens it may have happened since.
- An exploring frontier orders its exits by how often it has taken them, not by where they lead:
  being turned back changes nothing the brain can see, so the best-scoring exit stays best for ever.
- A map no walk entered is not evidence anything is shut. `unreached` is a budget fact; only *absent*
  from the table is a bug.
- Play path, button input only; debug tier, free to write RAM. A badge is the one wholesale write
  admitted, because it is a capability rather than an event: writing `wEventFlags` desynchronises
  scripts from map objects, and every stall found in such a save is a false positive. That is also
  why a coverage start is a save the cartridge itself finished.

## The policies

- `RandomPolicy::exploring` weights recent overworld ids down, because a uniform walker's distance
  from its start grows as the square root of its steps. A weight rather than an exclusion, and the
  battle draw stays uniform — a recency penalty there walks into a black-out, which throws the
  starting state away.
- The scripted party is one Squirtle that fights, an Oddish for Cut and a Machop for Strength. Surf
  is the only HM on the fighter because `pick_move_to_forget` never drops one, and one fighter at
  lv85 beats three at lv75 because experience is cubic.
- Every routing branch is bounded and hands back to the queue rather than parking: the queue is a
  route, so its next step walks out of the dungeon.
- Mainline battle tactics are frozen. `full_playthrough` is a golden RNG replay, so a change costing
  a turn before the last mainline catch re-cuts the route and fails hundreds of steps away — A/B
  against HEAD before debugging the stall site.
- The heal arm sits above the catch throw in `pick_battle_action`, or it is unreachable for the whole
  hunt: a throw is a turn spent not defending.
- A `CatchPokemon` that gives up records the species and the steps naming a `PartyRef` skip it: their
  wait on a species target is unbounded, and no scripted policy has a watchdog.
- The bag holds twenty entries and the route runs at the cap, so every pickup is somebody's toss. Put
  the toss where the bag binds, not beside the pickup it is for.
- Two arms over one decision must share a damage model: a 0-PP move scoring 122 livelocked the
  Elite-Four switch tactic.
- A battle item that asks which Pokémon has a row per member it would help (`helps_in_battle`), so
  rows of one item compare equal; a scripted heal pins `target` to the one out, or its choice moves.
- A wedged scripted run is silent — no watchdog, and `/api/events` goes on looking healthy.
- `route_toward` scores every reachable crossing into a neighbour, not only the nearest one
  `actions()` offers, or a route out of a pocket takes the pocket's own crossing back in for ever.
  `with_every_crossing` keeps the menu's rows first so a tie is theirs: `full_playthrough` replays
  their choices.

## Prose the model and the page read

- `Display for AgentEvent` is a UI contract: the page formats it straight on and `describe_event`
  sends it to the model. `Display` is a sentence, an id is a key, and the two never mix.
- `OverworldAction::id` is the one definition of an action id. A sprite id has no coordinate, because
  the square it carried was the approach tile and `actions()` re-picks that whenever the player
  moves; a reader takes the map off the front and the kind off the back.
- Winning the game does not hand the world back. `PokemonAgent::ending` returns above the watchdog,
  the game mode and any poll for the whole ceremony, during which `wCurMap` still reads the room; it
  clears on `wPlayerID` going zero and then non-zero, the only signal separating a reset from a
  screen transition.
- Only the start of a walk carries an id, so a reader pairs positionally. Walks interleave, and the
  count is a fact rather than a fault.
- An abort's `Textbox` reason reads "the game stopped you to say something": "it was interrupted"
  made a deployed run file a bug about a locked gym. Nothing counts repeated aborts.
- Every driver exit reports, and a `TextBox` the agent made up is the cartridge's voice used for the
  agent's own account. A boulder push is the one action still invisible — the shove runs as a script
  that takes the state away, so the goal around it is what completes.

## The text reader

- The screen is a page being typed. The reader extends on a prefix relation either way, splices on
  the overlap against the page, commits nothing on a blank frame, and needs two mismatches because
  `AutoBgMapTransfer` tears boxes. `commit_page` joins verbatim: both attempts at deduplicating
  deleted real battle text.
- A box is flushed wherever the reader stops being in charge, not only when it closes, because every
  blocker in Red prints a message and *then* runs a script.
- A driver that presses its own buttons still reads, through `accumulate` — `update_with` without the
  toggle, called before the driver's press and gated on `MessageBox`, because an in-battle bag list
  is drawn in the rows a message-box reader reads. A battle sub-state's reader is carried out of it,
  never rebuilt.

## The page's copy

- Never filter at the publish: the transcript writes what is published, and `useEventStream`'s fold
  is where the client drops things. A tool call and its result pair by call id, never by position.
- The heartbeat is sent on change with a 2 s keepalive, and `says_the_same_as` excludes the clocks —
  anything added to `StatusSnapshot` must be compared there. `/api/events` opens with the latest
  heartbeat, plan and battle script.
- `STALE_MS` is the reconnect signal on both streams, because a dead network produces no error, and
  speed is derived from consecutive heartbeats rather than the lifetime average.

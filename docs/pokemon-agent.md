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
- `GameMode::Script` during a walk is either a ledge hop (~660 ms, inside `DelayContext::long`'s
  rollback window) or an arrow-tile slide (up to 14 s). `wMovementFlags` bit 7 (`BIT_SPINNING`)
  says which, and the deadline is re-armed every tick while it is set. The guard asserts the abort
  count, because arrival was always true and the bug cost a paid request per hop.
- The screen lags RAM: `AutoBgMapTransfer` copies a third of the tilemap per V-blank, so a menu
  takes up to three frames to appear and `wTopMenuItemX/Y` is the authority on which menu is live.
  `BattleState::confirming` is the window after `Navigating` in which geometry is believed over
  text; without it a battle turn was decided twice, one paid request each.
- Which text reader to use is a fact about the game (`wIsInBattle`), not about `AgentState`.

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
  `overworld_id` mints `PalletTown:5,6:Warp` from it and the id is re-resolved by string equality.
  `id_kind` ends a person's id in their name with spaces stripped, and the word "sprite" appears
  nowhere a model reads.
- `BattleActionStarted` carries the nickname and the opponent's species, read at the decision point
  (a trainer's lead is not loaded at `BattleStarted`).
- `OverworldActionAborted` carries `at` in the expanded coordinate space, and its `Textbox` reason
  reads "the game stopped you to say something": "it was interrupted" made a deployed run file a bug
  about a locked gym. Nothing counts or withholds repeated aborts; noticing is the model's job.
  `OverworldInteractionCompleted` exists because a route to a sprite is `[A]` for ever once
  adjacent. Facing means what the game means, over a counter (`interaction_in_front` hops;
  `tile_in_front` must not).
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

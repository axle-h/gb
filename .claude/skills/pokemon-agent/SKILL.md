---
name: pokemon-agent
description: "PokemonAgent, Policy and the watchdog: the closed loops A-only input walks into, screen-versus-RAM timing, the prose the model and the page read, what an abort reports, and the SPA event stream that folds it. Load before touching src/pokemon/{agent,policy,text,tile_map,actions}.rs, AgentEvent or any Display it goes through, or web/src/useEventStream + Conversation."
---

# The Pokémon agent and what it reports

*Extracted from `CLAUDE.md`, which holds the rules of the road and the index of these skills. The
README is imported into `CLAUDE.md` and is not repeated there or here: this file has only the
invariants and the traps, nearly every one of which was learned by breaking something.*

## Agent and policy invariants

⚠️ **`PokemonAgent::poll_policy` is the single seam every decision point goes through**, and it resets the clock
the stuck-run watchdog reads. Call `policy.service_tools` directly from a new poll site and the watchdog believes
the run has been wedged since that moment, forever.

⚠️ **The emulator never pauses while the model thinks, and must not be made to.** A tool batch is answered by
`Policy::service_tools`, which only runs when `gb.run` advances the agent — so any pause spanning an LLM tool call
deadlocks the run. `GB_PAUSE_WHILE_THINKING` was built in W4 and removed the same day; `HostConfig` carries the ⚠️.
**The one exception is the shape of the rule, not a hole in it**: the park on a spent quota (the `llm-turn-loop` skill) happens when a
request has already failed, with nothing outstanding to service. Any future pause has to clear that bar.

**The watchdog** (`Policy::{stuck_timeout, pick_unstick}`) raises a `DecisionKind::Stuck` turn whose only terminal
tools are `press_buttons` and `wait`. ⚠️ **It is asked on every tick of the jam, not once** — a tool batch is only
serviced inside `agent.update`, so a one-shot notification would hang any turn that wanted to read first. ⚠️ **And
it must not reset the clock it reads**, or the jam clears the instant it is noticed. It never fires in a healthy
run: `ordinary_play_stays_far_inside_the_stuck_timeout` measures the longest silence at ~6 s against the 300 s
default.

### Closed loops the agent walks into

⚠️ **Every Gen 1 PC menu is a closed loop under A-only input, and `ReadingTextBox` presses B when
`PokemonApiTrait::in_pc_menu` says so.** Each leaves only on B, and A on its resting cursor picks the first entry,
which bounces off a refusal message straight back with the cursor untouched (`PCMainMenu` → Bill's PC →
`WITHDRAW` → `NoMonText` → `BillsPCMenu`). Nothing moves the cursor, so A never reaches `LOG OFF`. This wedged the
deployed run **permanently**, eight tiles from a fresh save; party size is irrelevant. Two traps in the detection:
⚠️ **the item PC sets no flag** — `TextScript_ItemStoragePC` (Red's bedroom) calls `PlayerPC` directly and leaves
`BIT_USING_GENERIC_PC` clear, so the screen is matched on `LOG OFF` as well; ⚠️ **and `LOG OFF` alone is not enough
either**, because the parent tree's submenus do not show one. `UsingPcBox`/`UsingItemPc` are excluded from
`assert_text_box_state` so they never reach `ReadingTextBox`; their *abort* paths do, which the same line fixes.

⚠️ **The START menu is six rows before the Pokédex and seven after it, so a cursor index does not mean the same row
in both.** `DrawStartMenu` omits POKéDEX until `EVENT_GOT_POKEDEX` and `.displayMenuItem` puts it back with an
`inc a`, so **index 2 is ITEM with the Pokédex and the player-name row without it** — `StartMenu_TrainerInfo`, which
`WaitForTextScrollButtonPress` leaves on A *or* B straight into `jp RedisplayStartMenu` with the cursor restored. A
closed loop under A, flashing the screen white twice a cycle. `start_menu_row` is the one place that knows.
⚠️ **The window is not a corner** — Oak's Parcel is delivered *before* the Pokédex, so every run passes through it,
and the deployed run spent 55 minutes wedged there. ⚠️ **And no test tier could have caught it**: `RandomPolicy`
implements only `pick_overworld_action`/`pick_battle_action`, so `soak` never issues a field move, while the leg
chain and `full_playthrough` reach those drivers long after the Pokédex. Pre-Pokédex they are reachable by an LLM
policy and nothing else — which is the general lesson, not a fact about this menu.

⚠️ **A TM or HM aimed at a Pokémon outside its learnset is the same loop, and `TeachingMove` has no exit from
it.** `CanLearnTM` tests a bitfield in the base-stats entry; a miss prints `MonCannotLearnMachineMoveText` and
then `jr .chooseMon` (`engine/items/item_effects.asm`) — back to the party menu, cursor untouched — so the
driver navigates to the same slot, presses A and is refused again. Its only completion is "the mon knows the
move", which never comes, so the attempt is 60 s of A-mashing ended by `DRIVER_ESCAPE_SILENCE`, after which the
policy asks for the identical teach. The deployed run of 2026-08-27 lived there. ⚠️ **Compatibility is knowable
before a button is pressed** and `pokemon::learnset` reads it out of the ROM rather than transcribing 151 × 55
bits; ⚠️ **`\1_TMNUM` is not the item id and the two run in opposite directions** — the HMs are TMNUM 51-55 at
item ids `$C4-$C8`, *below* the fifty TMs at `$C9-$FA`, and the flag index is `TMNUM - 1`. Three gates, the same
shape as Cut's: `tools::resolve_field_move` refuses the call in the turn, `agent.rs` refuses on the way into the
state, and `DeterministicPolicy::pick_field_move` skips the step so a scripted leg cannot re-issue one every
tick. ⚠️ **What the refusal says is the alternative, not the refusal** — "got no answer from the game for 60s"
reads as a malfunction and gives a model nothing to do differently, which is "it was interrupted" all over
again, so `learnset::teach_refusal` names who in the party *can* take it or says outright that nobody can.
⚠️ **And the proactive half was pointing at the wrong errand**: `prompt`'s `Blocked here:` line said "an HM to
be found and taught, and needs the CascadeBadge" whatever the run was holding, so a party carrying HM01 with the
badge won and nothing able to learn Cut was told to go and find HM01. It names the half actually missing now.
`teaching_an_hm_to_a_mon_that_cannot_learn_it_does_not_wedge` is the guard, and ⚠️ **it runs on a hand-rolled
policy rather than `DeterministicPolicy`**, which would skip the teach before the agent ever saw it and pass
without touching the thing under test.

⚠️ **A bag item the game will not use is the same loop again, and `UsingFieldItem` had no exit from it
either.** `ItemUsePtrTable` (`engine/items/item_effects.asm`) sends most key items to `UnusableItem`, which is
`jp ItemUseNotTime`: "This isn't the time to use that!" and back to the bag list with the cursor untouched.
This driver's only completion is `game_mode == Overworld`, which a refusal never reaches, so it was 60 s of
A-mashing ended by `DRIVER_ESCAPE_SILENCE`. The deployed run of **2026-08-27** alternated a
`use_item HelixFossil` with talking to the Mt Moon Rocket whose flavour line is "if you find a fossil, give it
to me", four minutes of wall clock at a time. Two gates and a net, and all three are needed:
- **`item_use::field_use_refusal`**, read out of `ItemUsePtrTable` rather than transcribed, refuses the known
  ones in `tools::resolve_field_move` (no round trip) and again in `agent.rs` on the way into the state.
  ⚠️ **Reading the ROM is not pedantry here**: the Card Key, the Poké Flute and the Coin Case are usable while
  the Silph Scope and the Lift Key beside them are not, and item ids `$15`/`$16` are the Safari Zone's BAIT and
  ROCK as well as the first two badges, so "the badges are unusable" would have been wrong.
- ⚠️ **The on-screen net in the driver, because a refusal can be *contextual* and no table predicts one.**
  `ItemUseEscapeRope` outside `EscapeRopeTilesets` and `ItemUseBicycle` indoors are real effects that end at the
  same `ItemUseNotTime`. It reuses `shows_battle_refusal` (one list, not two) and **latches
  `BACKING_OUT_TICKS` of B** rather than pressing one or dropping to `Idle`: one B lands back on the bag list
  where the driver's own navigation presses A again, and ⚠️ **`Idle` with the bag still open costs 33 s** of
  the generic reader getting out and emits the whole screen it walked through as one `TextBox` — which is what
  the deployed run called an "unrelated TM34/bag menu prompt". Measured 60 s → 5.4 s.
`using_an_item_the_game_will_not_use_does_not_wedge` and
`an_item_the_map_refuses_backs_out_rather_than_mashing_for_a_minute` are the guards, both on hand-rolled
policies rather than `DeterministicPolicy` (which only ever reaches for the Poké Flute), both asserting that
**decisions keep coming** and that the *words* arrive. ⚠️ **The target has to be reachable** or the driver
reports "can't reach the field-item target" on its own and the test passes with the gate removed.

### Screen versus RAM

⚠️ **The screen the agent reads lags the game's own tilemap, and its three horizontal bands can disagree.**
`AutoBgMapTransfer` (`pokered/home/vcopy.asm`) copies `wTileMap` into VRAM one third per V-blank, rotating
top/middle/bottom, so a menu takes up to three frames (~50 ms, two and a half agent ticks) to appear and the band
it replaces still shows the old one. RAM is therefore *ahead* of the screen at every menu transition, and
`wTopMenuItemX/Y` — written by whoever is about to call `HandleMenuInput` — is the authority on which menu is live.

⚠️ **That cost a paid LLM turn per battle turn for months.** `WaitingForMenu` resolves the main battle menu from the
text (`FIGHT` … `RUN`) as well as from geometry, because after an item turn the geometry is genuinely ambiguous. But
the move list is drawn at `hlcoord 4, 12` over the battle menu's own box, so for a frame or two after
`MoveSelectionMenu` writes `(5, 12)` the bottom band still reads `FIGHT PKMN ITEM RUN` — the text test fired,
concluded the turn was the policy's again, and **threw away the move `Navigating` had just highlighted**.
`BattleState::confirming` is the fix: a bounded window after `Navigating` hands over in which the geometry is
believed and the text test is skipped. `a_battle_turn_is_decided_once_rather_than_twice` guards it, at two policy
latencies because it is a race the policy's own speed moves. ⚠️ **Counting decision points is not enough** — that
passes if the agent stops fighting, so it asserts one poll *per landed move*, read off the enemy's HP.

⚠️ **It re-timed the battles, exactly as `with_original_battle_timing` warns, and `can_reach_lavender` was the
casualty** — a leg pinned to one RNG line. It ran Razor Leaf dry crossing Rock Tunnel and failed at *every* window
length, so it is the shift and not the number. `back-in-cerulean.bin` was regenerated (`can_return_to_cerulean`
under `--features regen-fixtures`) and the chain is green from it — one fixture, not the cascade.

### Prose the model and the page read

⚠️ **`impl Display for AgentEvent` is a UI contract, not debugging output.** `host.rs` does `format!("{event}")`
straight into `UiEventBody::Agent { text }` and the page prints it verbatim, so a `{:?}` puts
`Fight { slot: 1, battle_move: PokemonMove { name: Growl, pp: 40 } }` on screen for every turn of every battle —
which it did, for months. It is also what `llm::prompt::describe_event` sends the *model*.
`a_battle_turn_reads_as_a_sentence` is the only thing that looks at the prose.

⚠️ `BattleActionStarted` carries the acting Pokémon's nickname **and the opponent's species**, because nothing
downstream can look either up: the host formats events off the emulator thread. ⚠️ The opponent is read at the
**decision point**, not at `BattleStarted`: `InitWildBattle` sets `wIsInBattle` *before* `LoadEnemyMonData` and a
trainer's lead is not loaded until they send it out, so reading it as the battle starts reports the previous battle.

⚠️ **An event that names no target is the same bug one level down.** Three quarters of a random run's log is the
agent walking somewhere, and every one of those lines is a `MetaTile`'s `Display` — which was `strum`'s derive, so
the page said `→ heading for Warp` and never which warp, which map or who. Each variant names its target now (`the
warp to OaksLab`, `the way into Route1`, `Mom`), as a **noun phrase**, because the same string has to read as English
in four frames: the three `AgentEvent` sentences and `NoRoute`'s "there is no route to {tile}". ⚠️ **`MetaTile::kind`
is the other half and must stay the variant name** — `llm::tools::overworld_id` mints `"PalletTown:5,6:Warp"` out of
it, the model quotes it back, it is re-resolved by string equality, and `Conversation.tsx` prints it verbatim, so the
prose is free to be reworded only because the key is a different function. `a_walk_says_where_it_is_going` and
`an_id_keeps_the_variant_name_the_prose_left_behind` hold them apart.

⚠️ **The word "sprite" appears nowhere a model reads, and `MetaTile::id_kind` enforces that.** It is the emulator's
vocabulary for a moving object on a screen and the model has no screen: it read as jargon and was the same word for
Professor Oak and for a boulder. So an id ends in the **person's name** — `OaksLab:2,2:Pokedex1`. ⚠️ **Spaces are
stripped**, because several `MapSprite` names have them ("Middle Aged Woman") and an id resolved by string equality
must not be whitespace-sensitive. ⚠️ **`MapView`'s key is `people`, spelled the id's way** (`Pokedex1`) and built by
calling `id_kind` rather than a second function that agrees with it. `kind()` still returns the variant name, for the
sort key and the tests.

### What the agent reports

⚠️ **A textbox is detected before its characters are drawn**, so the reader emits a stream of empty ones — on the
deployed run they were most of the log. `PokemonAgent::event` drops them, and it is the funnel *every* event goes
through (including those collected into `update`'s local `new_events`), so the transcript is clean as well as the page.

⚠️ **Reading a text box and *reporting* it are two moments, and treating them as one lost the most important text
boxes in the game.** The buffer used to be emitted only in `assert_text_box_state`'s "the box closed" arm, which
fires while the agent is still `ReadingTextBox` — but `assert_script_state` runs **before** it in `update` and swaps
the state out for `RunningScript`, so a box the game follows with a script never reached that arm. ⚠️ **That is the
shape of every blocker in Pokémon Red**: print a message, then `StartSimulatingJoypadStates` to shove the player back
a tile. Measured: a landed conversation was followed by a `TextBox` event **31 times out of 38**, an aborted walk
**2 of 28**. The run walked into the Route 22 gate, heard nothing, talked to the guard five times and filed a
`report_issue`; the words it could not see were "You don't have the BOULDERBADGE yet!".
`PokemonAgent::flush_text_reader` is the fix and `PokemonTextReader::take` what it drains with. ⚠️ **It hangs off the
two places that assign `self.state`** — `set_state` and `backup_current_state` — so the rule is structural rather than
a list of call sites, and a battle or a mart stealing the state is covered by the same line. ⚠️ **`take` clears rather
than replaces**, because `backup_current_state` keeps the state it flushed and a ledge jump restores it. The pair is
`a_guard_who_turns_you_back_is_quoted_rather_than_swallowed` and `talking_to_that_guard_reports_what_he_said`, on
`route22-gate.bin`, and ⚠️ **what they assert is that the *words* arrive** — an empty `TextBox` is dropped by `event`,
so counting events would pass on the stream of empty ones the bug already produced.

⚠️ **`### On screen` is not a second chance at this, and it looks like one.** `observe::screen_text` reads the tile map
as it stands, so through a conversation it returns a rolling fragment (`"Onl"`, `"Only truly skilled trainers are"`,
`""`, …) and by the time the abort has resolved into an overworld decision it is `None`. Only the reader accumulates
across pages, so the `TextBox` event is the only complete record of anything the game said.

⚠️ **Talking to someone is that action succeeding, and the text box is the only signal it gets.** The route to a sprite
ends by facing it and pressing A, and it is re-derived every tick — so once the player is standing in front of the
sprite that route is `[A]` for ever and the "the route ran out" branch that completes an ordinary walk is never
reached. Hence `AgentEvent::OverworldInteractionCompleted`; before it, every successful conversation was reported as
"✗ gave up on Mom: it was interrupted".

⚠️ **An abort also says *where* the walk stopped.** `OverworldActionAborted` carries `at`, so the line is "✗ gave up on
the way into Route2 at (19, 11): the game stopped you to say something". The deployed run produced that abort **143
times** — the Viridian old man blocks the north exit until Oak's Parcel is delivered — and concluded "the choose_action
pathfinding keeps failing". ⚠️ **Nothing counts those aborts, refuses a repeatedly-failing target, or drops one from
the menu**: noticing that the same square twenty times means a blocked route is deliberately the model's job, and a
menu that silently withheld a reachable-looking action would be a worse lie. ⚠️ **`at` is in the *expanded* coordinate
space** the ids, the map picture and every `Location:` line use, never `raw_player_coords`;
`MetaTileMap::player_position` is where it comes from, and the one abort that reports `None` on purpose is `WrongMap`.

Three traps in detecting a landed conversation, each paid for separately:

- ⚠️ **It is what the player is *facing*, not what it set out for.** A script can open a box mid-walk — the rival's,
  two tiles short of the aide in Oak's lab — and "my destination was a sprite" calls that a conversation the run never
  had. `a_script_that_interrupts_a_walk_is_still_an_abort`.
- ⚠️ **A PC is not in `meta_tiles`**, so the tile in front of a player using one reads as `Obstacle`. It is a hidden
  event, indistinguishable from the wall it is drawn on, which is why `pc_locations_for` is a transcribed table.
- ⚠️ **"Facing" has to mean what the game means, which reaches *over* a counter.** Gen 1 talks through
  `wTilesetTalkingOverTiles` (`MetaTile::Counter`) — a nurse, a mart clerk, every gym receptionist. `actions()` routes
  to the far side of the desk, so the tile in front is the counter and never the person; matching it literally reported
  every heal in every Pokémon Centre as "✗ gave up on Nurse". `MetaTileMap::interaction_in_front` is the one that hops;
  ⚠️ **`tile_in_front` must not**, because `cut` and the surf mount are about the literal tile.

⚠️ **And the *word* was the other half: "it was interrupted" reads as a malfunction, and being stopped is the game
working.** The deployed run walked at the Viridian Gym door with no badges, read that reason with "The GYM's doors are
locked..." on the line below, and filed a `report_issue`. Two changes, two audiences:
`OverworldActionAbortedReason::Textbox` reads **"the game stopped you to say something"**, pointing at the text box
that follows rather than describing the walk's failure; and `SYSTEM_PROMPT` says once, under "The game is not broken",
that being stopped is how the game tells you something. ⚠️ **Not "something was said"**, which buries the fact the
model needs. ⚠️ **And no `Blocked here:` line was added for it**, unlike the Cut/Surf case: the cartridge already says
the doors are locked, in a box quoted into the very next turn.

### The page's copy of all this

⚠️ **What the page shows and what the model is told are two different lists, and the split is on the client.**
`useEventStream`'s `fold` drops `text_box` and `overworld_interaction_completed` — the dialogue is already on screen in
the game's own font, and "✓ talked to Mom" says less than the line of Mom's that follows. Both still go to the model
and to `transcript.jsonl`. ⚠️ **Do not "simplify" this by filtering at the publish**: `run::transcript` writes what is
published, so that deletes the dialogue from `/api/history` and from the archived record, which is the one copy nothing
can rebuild.

⚠️ **A tool call and its result are two events and one row.** `UiEventBody::ToolCall` carries the endpoint's own call
`id`; `ToolResult` is paired to it by that id in `attachResult`. ⚠️ **Paired on the id, never on position or name** — a
turn can call several tools in one batch, and two `read_party` calls are indistinguishable by name. ⚠️ **Neither row
may go through `push`**, or an answer attaches to the wrong call.

⚠️ **A tool's *picture* is referenced, never carried.** A map render is a couple of hundred kilobytes and every
published event is also a line of `transcript.jsonl`. `ToolResult.image` is a flag; the bytes live in a 16-entry ring in
`Published` keyed by the **seq of the event that announced them** (so the publish must happen before the `put`), and
`/api/tool-image/{seq}/image.png` serves them. ⚠️ **A 404 there is the expected answer** — anything older than the last
handful is gone and the page shows the caption alone. `MAX_TOOL_RESULT` truncates the text server-side for the same
reason: a truncation the client applies has already been broadcast and written to disk. ⚠️ It sizes the *broadcast copy*
only — the model is always sent all of a tool result.

⚠️ **The status heartbeat is sent on change, not on a timer.** Sampled at `GB_STATUS_HZ` and published only when it says
something the last one did not, with a 2 s keepalive. At the original 10 Hz unconditional it measured **49.7 kbit/s per
viewer** — six times the idle video feed, nine of ten payloads byte-identical; it is now 5.2. Two consequences:
`StatusSnapshot` compares with `says_the_same_as`, which excludes the clocks and `frame_seq` (a derived `PartialEq`
would never match and the suppression would silently never fire), and `/api/events` **opens with the latest heartbeat**
(`join_events`, subscribe-then-read, the same handshake as the video keyframe) or a page opened during a quiet stretch
shows an empty panel. ⚠️ Anything *added* to the snapshot must be added to `says_the_same_as`'s destructuring **and
compared there**: the pattern is exhaustive, so a new field is a compile error, but binding it without comparing it is
not. `dropped_ms` is the counter-example to the clock exclusion: it stands still on a host that is keeping up, so it
forces no heartbeat in a healthy run and the moment it moves is worth telling a viewer about.

⚠️ **A lifetime average is not a rate, and `emulated_ms / wall_ms` is the average.** The page's speed line was that ratio
and read below 1× for ever on a host running at full speed. Two mechanisms, both permanent, since an average can only
converge on the truth from below: **`MAX_CATCHUP` drops emulated time on purpose**, so any iteration overrunning 250 ms
is subtracted from the numerator for the rest of the run (a **14.85 s** startup debt was still there five minutes in,
against an instantaneous speed of exactly 1×); and ⚠️ **`wall_ms` was measured from a clock stamped at construction while
`emulated` is zeroed by `start_new_run`**, so a run started in a long-lived process reported its first seconds against
those hours. `progress()` had always used the correctly-paired `run_started`; the heartbeat now does too.

⚠️ **The tell that it was accounting and not the emulator is that the deficit was *constant*.** Two samples 90 s apart put
`wall_ms − emulated_ms` at 14908 and 14845 ms — shrinking by 0.0704 ms/s, which is `to_duration` truncating 953.674 ns to
953, so **1.0007× is the ceiling and a healthy host reads exactly that**. A genuinely slow host shows a *growing* gap. The
page derives speed from consecutive heartbeats (`sampleSpeed`, a 500 ms window because at `GB_STATUS_HZ`'s default two
samples can be 100 ms apart). ⚠️ **A park needs no case in that and must not be given one**: the host stops the emulator
*and* subtracts the wait from `wall_ms`, so both counters freeze and the last live reading is held under the PAUSED plate
— where a `dw > 0` guard would report `0.00×`. ⚠️ **`RunProgress::wall_ms` was the same bug's other half**: `paused_total`
was subtracted by the heartbeat and not by `progress()`, which is what `meta.json` and the ledger record, so a run parked
overnight wrote the whole night down as play. Both paths subtract it now.

⚠️ **Send-on-change needs a cell per thing sent, and the plan was the one without.** `join_events` opened with the
heartbeat alone, so the plan — published only when it *changes* — was never replayed to a page that had just loaded.
`/api/history` was no better: it keeps `MAX_BACKLOG` (2000) events and a reasoning model publishes one per streamed token,
so the last `Plan` falls off within minutes. The symptom was neither, because `PlanPanel` renders nothing for an empty
list, so it read as a styling bug. `Published::latest_plan` is the fix, on the same handshake, and it works because a
`Plan` event is *absolutely* stated. ⚠️ **Anything else that becomes send-on-change belongs in `join_events` too.**

⚠️ **Both keepalives are load-bearing on the client, because a dead connection is otherwise indistinguishable from a quiet
one.** The page's retry loops were error-driven — `onerror` on the `EventSource`, `catch` on the video `fetch` — and both
are right about every case that *produces* an error. A network going away produces none: no FIN, no RST, and a stream we
only read from sends nothing that could time out, so the page froze on its last frame **still showing the green live pill**
(measured at 75 s, not going to recover). `STALE_MS` (`web/src/api.ts`, 4× the server's 2 s `KEEP_ALIVE`) is the fix:
silence, not an error, is the signal. ⚠️ **Not the SSE keep-alive** — that is a comment line `EventSource` hands to no
callback, so a watchdog on it starves and reconnects every 8 s for ever; the status heartbeat is the one that arrives.
⚠️ **On the video side, not the messages `readVideoStream` yields** — its keepalive is a zero-length message that yields
nothing, so a watchdog fed from messages would fire on a screen that is merely not moving; it is fed from the inflated
chunks. ⚠️ And `subscribeVideo` needs **one `AbortController` per attempt**, chained to the caller's: aborting the caller's
own signal is how that loop is told the component unmounted. Reproduce with `kill -STOP` — `docker stop` is a clean close
and exercises the path that already worked.

⚠️ **A reconnect of `/api/events` is also a reload of the transcript**, because a fresh connection opens with the latest
heartbeat and plan and nothing else. `/api/history` used to be fetched once at mount, so every reconnect resumed a log with
a hole nothing would fill and a dormant tab came back showing the hour-old log with the live pill green. Now `subscribe`
reports every `onopen` (the browser's own transparent retries included) and the hook resets everything the old connection
folded — entries, pending queue, plan, usage, speed anchor — and refetches `/api/history`. Three traps. ⚠️ **The reset has
to be inside `onopen`, before `alive`**, so it lands ahead of the opening heartbeat and plan rather than throwing them
away. ⚠️ **The backfill is generation-guarded**: a fetch started by the old connection can resolve after the new one has
reset the page. ⚠️ **A hidden tab is resynced on return whether or not its socket died** (`visibilitychange` after more
than `STALE_MS`, `pageshow` with `persisted`): a backgrounded tab gets no animation frames, so `pending` overflows on a
healthy connection, and the watchdog's own timer is throttled with everything else. A short tab flip keeps the connection.
This is deliberately a full reload and not a `since=` merge: `read_since` reads from the tail so a bare backlog is cheap,
and a merge of a *folded* log across a gap is where the bugs would live.

⚠️ **Every `UiEvent` carries `at`, a Unix-millisecond wall-clock stamp, and it is the only clock the page can date a line
by.** `wall_ms` and `emulated_ms` are elapsed times *since this process started*, and the browser cannot supply one either,
because `/api/history` replays a backlog that may be hours old. It is stamped in `publish_event`/`publish_status` and lands
in `transcript.jsonl` too. ⚠️ **The SPA's copy is optional and must stay optional** — the runs on disk predate the field.
⚠️ And `useEventStream`'s `signature` excludes it, for the reason it excludes `seq`: it differs on every event, so leaving
it in would stop identical rows ever collapsing into a `×3`.


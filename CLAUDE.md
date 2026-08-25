# CLAUDE.md

@README.md

**The README is imported above, so it is already in your context — do not re-read it, and do not
repeat it here.** It has what the project is, the `src/` tree, the policy/agent model, the run
directory, the endpoints, the environment block, the build and the deployment. This file has only
what the README deliberately leaves out: the invariants, the traps, and the test workflows. Nearly
every ⚠️ below was learned by breaking something.

The only surviving design doc is `docs/llm-web-playthrough-plan.md` (the LLM/web work, W0–W9, all
done). The emulator-accuracy plan it used to sit beside was deleted once its phases landed; where a
number below is attributed to "Phase C" or "Phase D", that is the history, not a file you can open.

## Rules of the road

- **Always `--release`.** The integration tests emulate every frame and are unusably slow in debug.
- The crate has **no lib target** — it is `--bin gb`, never `--lib`.
- Agent and policy debugging goes to stdout, so add `--nocapture` when you care about it.
- **Run `full_playthrough` after every major work item and before pushing.** See below for why the
  leg tier is not a substitute.

## Build inputs

⚠️ **pokered needs rgbds ≥ 1.0.0 and `rgbdscheck.asm` `fail`s the assembly below that** — a hard
error, not a warning, so an old rgbds does not produce a wrong ROM, it produces none. The container
pins 1.0.3 (`ARG RGBDS_VERSION`), which upstream's own `INSTALL.md` and CI name.

⚠️ **The symbol names are upstream's to change.** The 2026-08 bump renamed pokered's "hidden object"
and "missable object" vocabulary to **hidden event** and **toggleable object** (`HiddenEventMaps`,
`HiddenEventPointers`, `wToggleableObjectFlags`). That surfaces as a compile error rather than a
silent one only because `build.rs` emits constants for symbols that exist — nothing more.

⚠️ **`web/dist` must exist for the crate to compile at all** — `rust-embed`'s derive fails if the
folder is missing. That is why `web/dist/.gitkeep` is committed and why `vite build` (which empties
`dist`) copies it back from `web/public/`. A checkout that has never run `pnpm run build` compiles
and serves a page naming the two commands to run, so a missing UI is never a mystery.

⚠️ **`web/pnpm-workspace.yaml`'s `minimumReleaseAge` cooldown is checked on *every* install —
`--frozen-lockfile` included, not just `pnpm update`.** A lockfile pinning anything younger than the
window fails with `ERR_PNPM_NO_MATURE_MATCHING_VERSION`, so the lockfile has to be *generated* under
the same number the builds enforce. Raise the window without regenerating `pnpm-lock.yaml` and what
breaks is the container build, not the dev loop. The file says why it is 3 days and when that can
change. pnpm's version is pinned by `packageManager` in `web/package.json` and activated by corepack
— deliberately not named in the Dockerfile, so there is nothing there to drift from it.

## Emulator invariants

⚠️ **Every mapper resolves its bank register differently and the differences are not decoration.**
MBC1 remaps a zero selection *then* wraps, so a wrap can reach bank 0; MBC3 wraps *then* remaps, so
it never can; MBC2/MBC5/HuC1 have their own rules again. Same two operations, opposite order,
different answer — and it is what makes blargg's combined `dmg_sound.gb` terminate. `src/mbc.rs`'s
module docs have the table.

⚠️ **A save state is not tied to the machine that wrote it, and a DMG state in a CGB used to blank
the screen.** `GB_HARDWARE=cgb` runs the cartridge on a Game Boy Color, which for this DMG-only ROM
means compatibility mode and the boot ROM's title-derived palette. But every committed fixture and
every deployed `state.gbst` is a **DMG** capture, and a DMG's CGB palette RAM is
`PaletteBank::default()` — all-ones, i.e. white — so restoring that section painted the boot palette
out and rendered every shade white while the game underneath played perfectly. It cannot be repaired
from outside either: compatibility mode leaves `FF68`-`FF6B` unmapped, so the palette is a
**constant** rather than initial state. `MMU::read_sections` therefore re-installs it last, after the
`cgb` section has been read; `a_dmg_save_state_does_not_blank_a_compatibility_mode_screen` is the
guard. ⚠️ Fixture vintage is what made this look intermittent — states captured before the `cgb`
section existed carry no palette to restore and were always fine.

⚠️ **The RTC's time source is injectable and anything replayable must pin it**
(`MMU::set_rtc_time_source`) — the default is the host clock, so an RTC cartridge under a
fixture-driven test would fail only sometimes. Nothing committed has an RTC: `pokered.gbc` is `0x13`,
MBC3 with *no* timer.

**Save states.** `src/savestate/mod.rs`'s module docs are the authoritative reference. **Adding a
section is free; adding a field means appending it as an extra value within its section and bumping
that section's version** — neither churns fixtures. ⚠️ **Never reorder or retype an already-shipped
value without bumping the section version**: bincode is positional and has no schema migration. If
you find yourself about to write a legacy struct, check first whether you can re-cut the boundary
instead — that is how CGB support cost zero fixture regeneration, by keeping `wram`/`ppu`'s shipped
first value and appending the new banks as a second.

The **102** committed fixtures in `src/pokemon/data/*.bin` are `include_bytes!`'d;
`every_committed_fixture_decodes` in the default tier fails in seconds if a layout change breaks
them. ⚠️ **That test walks the directory and `load_state`s every `.bin` in it**, so `data/` is for
save states and nothing else — anything else goes in a subdirectory (`data/gfx/` is the one).
`pokemon-red.sav` is raw SRAM, not a save state — the SDL UI loads and writes it at runtime.

⚠️ **`Audio` and `PPU` exclude derived state from `PartialEq`** — the resampler output and the cached
mix (`mixed`/`levels`/`mix_dirty`), and the frame buffer plus the per-scanline sprite list
respectively. None of it is serialised, so none of it may take part in equality, or
`game_boy::tests::save_and_load_state` would compare restored state against state that was never
saved. `Schedule` is derived the same way and is not serialised at all — only the clock it is built
from (`MMU::now`, the `sched` section) is. Adding a field to `Audio` **is** safe now; the old
"nothing may be added to `Audio`" rule died with the sectioned format. The output sample rate is
applied by `Audio::set_output_sample_rate` rather than stored, so a caller that loads a save state
must re-apply it (see the `F9` handler in `render.rs`).

⚠️ **`PPU::draw_pixels_to` and the three DMA transfer loops are `#[inline(never)]`/`#[cold]` on
purpose.** `MMU::update` runs once per CPU instruction, and letting those inline into it grew it 60%
(3052 → 4893 bytes) and cost several percent of core throughput to instruction-cache pressure alone.
If you touch them, check with `nm -S --size-sort -C target/release/deps/gb-*` that `MMU::update` is
still around 3–4 KB (Phase C left it at 3764 bytes). `Serial::complete_transfer` and the APU's
`mix()` fell to the same rule.

⚠️ **`MachineCycles::to_duration` multiplies by 4e9, and that overflowed `u64` after ~73 minutes of
emulated time** — silently, because release builds wrap rather than panic. Everything that reports
emulated time over a long run went through it: `meta.json`'s `emulated_ms` and the status heartbeat
both wrapped every 73 minutes on the deployed run. It surfaced as `soak`'s progress line simply
stopping after 3600 s, which looked like a bug in the test. `from_duration` had always used `u128`;
now both do, and `cycles::tests::to_duration_survives_a_long_run` pins it out to 24 h.

**Tuned constants**, both arrived at empirically: `AGENT_RESOLUTION` (20 ms) — longer and the player
overshoots on the overworld, shorter and the game state does not settle between frames; and
`DelayContext`'s 2500 ms post-script delay, which covers the worst-case pre-battle animation gap
observed in practice.

## Agent and policy invariants

⚠️ **`PokemonAgent::poll_policy` is the single seam every decision point goes through**, and it is
not just a tidy-up: it resets the clock the stuck-run watchdog reads. Call `policy.service_tools`
directly from a new poll site and the watchdog will believe the run has been wedged since that
moment, forever.

⚠️ **The emulator never pauses while the model thinks, and must not be made to.** A tool batch is
answered by `Policy::service_tools`, which only runs when `gb.run` advances the agent — so any pause
spanning an LLM tool call deadlocks the run. A `GB_PAUSE_WHILE_THINKING` flag was built in W4 and
removed the same day; `HostConfig` in `src/host.rs` carries the ⚠️.

⚠️ **There is exactly one exception and it is the shape of the rule, not a hole in it**: the park on
a spent quota (`Worker::park_until`, see "When the endpoint says no" below). What deadlocks is a
pause that spans something *waiting on the emulator*. A park happens when a request has already
failed, before any tool was called, with nothing outstanding to service and nothing the agent could
be asked — so no work is blocked by the stop. Any future pause has to clear that same bar; a pause
taken with a tool batch in flight would hang the run exactly as W4's flag did.

**The watchdog** (`Policy::{stuck_timeout, pick_unstick}`) raises a `DecisionKind::Stuck` turn whose
only terminal tools are `press_buttons` and `wait`. Two ⚠️s, both learned in the design:

- **It is asked on every tick of the jam, not once.** A tool batch is only serviced inside
  `agent.update`, so a one-shot notification would hang any turn that wanted to read first.
- **It must not reset the clock it reads**, or the jam clears the instant it is noticed and the turn
  is never polled again.

In a healthy run it never fires — `mechanics::ordinary_play_stays_far_inside_the_stuck_timeout`
measures ordinary play's longest silence at ~6 s of game time against the 300 s default.

⚠️ **Every Gen 1 PC menu is a closed loop under A-only input, and `ReadingTextBox` presses B when
`PokemonApiTrait::in_pc_menu` says so.** Each PC menu leaves only on B, and A on its resting cursor
picks the first entry, which bounces off a refusal message straight back with the cursor untouched —
`PCMainMenu` → Bill's PC → `WITHDRAW` → `NoMonText` → `BillsPCMenu`, or `PlayerPCMenu` →
`WITHDRAW ITEM` → nothing stored → `PlayerPCMenu`. Nothing in the cycle moves the cursor, so A never
reaches `LOG OFF`. This wedged the deployed run **permanently**, eight tiles from a fresh save, and
it was not a Bill-event or empty-box problem: a full party or a one-mon party traps identically.

Two traps in the detection, both paid for once. **The item PC sets no flag** —
`TextScript_PokemonCenterPC` goes through `ActivatePC` and sets `wMiscFlags`' `BIT_USING_GENERIC_PC`,
but `TextScript_ItemStoragePC` (Red's bedroom, and the one that actually broke) calls `PlayerPC`
directly and deliberately leaves it clear, so the screen is matched on `LOG OFF` as well. And
**`LOG OFF` alone is not enough either**, because the parent tree's submenus do not show one — hence
both checks. `UsingPcBox`/`UsingItemPc` cannot collide with this: they are excluded from
`assert_text_box_state`, so they never reach `ReadingTextBox`. Their *abort* paths do, which is a
second bug fixed by the same line.

⚠️ **The START menu is six rows before the Pokédex and seven after it, so a cursor index does not
mean the same row in both.** `DrawStartMenu` omits the POKéDEX row until `EVENT_GOT_POKEDEX` and
`home/start_menu.asm`'s `.displayMenuItem` puts it back with an `inc a`, so **index 2 is ITEM with
the Pokédex and the player-name row without it** — `StartMenu_TrainerInfo`, which
`WaitForTextScrollButtonPress` leaves on A *or* B straight into `jp RedisplayStartMenu` with the
cursor restored from `wBattleAndStartSavedMenuItem`. A closed loop under A, and one that flashes the
screen white twice a cycle (`GBPalWhiteOut` at each end), which is what it looks like on the page.
`start_menu_row` is the one place that knows; three drivers hardcoded the 2 and said so in the
comment. ⚠️ **The window is not a corner** — Oak's Parcel is delivered *before* the Pokédex, so every
run passes through it; the deployed run spent 55 minutes wedged there in ViridianMart, escaping only
because the model eventually guessed B. ⚠️ **And no test tier could have caught it**: `RandomPolicy`
implements only `pick_overworld_action`/`pick_battle_action`, so `soak` can never issue a field move
and never enters those drivers, while the leg chain and `full_playthrough` reach them long after the
Pokédex. Pre-Pokédex, they are reachable by an LLM policy and nothing else — which is the general
lesson, not a fact about this menu.

⚠️ **`impl Display for AgentEvent` is a UI contract, not debugging output.** `host.rs` does
`format!("{event}")` straight into `UiEventBody::Agent { text }` and the page prints it verbatim, so
a `{:?}` in there puts `Fight { slot: 1, battle_move: PokemonMove { name: Growl, pp: 40 } }` on
screen for every turn of every battle — which it did, on the deployed run, for months. It is also
what `llm::prompt::describe_event` sends the *model*. `agent::tests::a_battle_turn_reads_as_a_sentence`
is the only thing that looks at the prose. ⚠️ **No em dashes in any of it** — not here, not in a
`Notice`, not in a turn headline, not in a decision summary. A colon, a semicolon or a full stop says
the same thing in a log row read at a glance on a phone; the punctuation in *this* file and in the
code comments is a different audience and is not the rule. (Prose written for the model —
`llm::prompt`, `llm::tools`, a rejection sentence — is that other audience too, and is left alone.) Relatedly, `BattleActionStarted` carries the acting
Pokémon's nickname **and the opponent's species**: nothing downstream can look either up, because
the host formats events off the emulator thread and the battle has moved on by then. ⚠️ The opponent
is read at the **decision point**, not at `BattleStarted`: `InitWildBattle` sets `wIsInBattle`
*before* `LoadEnemyMonData` and a trainer's lead is not loaded until they send it out, so anything
reading the enemy as the battle starts reports whatever the previous battle left behind.

⚠️ **An event that names no target is the same bug one level down.** Three quarters of a random
run's log is the agent walking somewhere, and every one of those lines is a `MetaTile`'s `Display` —
which was `strum`'s derive, so the page said `→ heading for Warp` / `✓ reached Warp` / `→ heading for
Sprite` and never which warp, which map or who. Each variant now names its target
(`the warp to OaksLab`, `the way into Route1`, `Mom`), as a **noun phrase**, because the same string
has to read as English in four frames — the three `AgentEvent` sentences and `NoRoute`'s "there is no
route to {tile}". ⚠️ **`MetaTile::kind` is the other half and must stay the variant name**:
`llm::tools::overworld_id` mints `"PalletTown:5,6:Warp"` out of it, the model quotes that back, it is
re-resolved by string equality, and `Conversation.tsx` prints it verbatim — so the prose is free to
be reworded only because the key is a different function. `agent::tests::a_walk_says_where_it_is_going`
and `an_id_keeps_the_variant_name_the_prose_left_behind` are the pair that hold it apart.

⚠️ **The word "sprite" does not appear anywhere a model reads, and `MetaTile::id_kind` is where that
is enforced.** It is the emulator's vocabulary for a moving object on a screen and the model has no
screen: it read as jargon, it was the same word for Professor Oak and for a boulder, and the menu row
beside `…:Sprite` then had to spend the name again to say who was actually standing there. So an id
ends in the **person's name** — `OaksLab:2,2:Pokedex1` — and the row is the bare distance. Two things
follow. ⚠️ **Spaces are stripped, and that is not cosmetic**: several `MapSprite` names have them
("Middle Aged Woman"), and an id resolved by string equality must not be whitespace-sensitive. ⚠️
**`MapView`'s key is `people`, spelled the id's way** (`Pokedex1`, not `Pokedex 1`) and built by
calling `id_kind` rather than by a second function that agrees with it — two spellings of one person
across two blocks of the same request is a way to be wrong with no upside. `kind()` still returns the
variant name, for the sort key and for the tests; `id_kind` is what ids are minted from.

⚠️ **A textbox is detected before its characters are drawn**, so the reader emits a stream of empty
ones — on the deployed run they were most of the log. `PokemonAgent::event` drops them, which is the
funnel *every* event goes through (including the ones collected into `update`'s local `new_events`),
so the transcript is clean as well as the page.

⚠️ **Talking to someone is that action succeeding, and the text box is the only signal it ever
gets.** The route to a sprite ends by facing it and pressing A, and it is re-derived every tick — so
once the player is standing in front of the sprite that route is `[A]` for ever and the "the route
ran out" branch that completes an ordinary walk is never reached. That is why a landed interaction is
its own event (`AgentEvent::OverworldInteractionCompleted`) rather than an
`OverworldActionCompleted`, and why it was reported as `OverworldActionAborted { reason: Textbox }`
for so long: "✗ gave up on Mom: it was interrupted", after a conversation that went perfectly. Not
just a word on a page — an abort reason is what `llm::prompt` calls the most useful thing the agent
can say, so every successful conversation was reported to the model as a failed one.

⚠️ **An abort also says *where* the walk stopped, and the reason alone was not enough to act on.**
`OverworldActionAborted` carries `at` — the square the player was actually standing on — so the line
is "✗ gave up on the way into Route2 at (19, 11): it was interrupted". The deployed run produced that
abort **143 times**: the Viridian old man blocks the north exit until Oak's Parcel is delivered, so
the walk was impossible every time and the log said only that something stopped it. The model
concluded, in its own `why` strings, that "the choose_action pathfinding keeps failing" and started
walking by hand. ⚠️ **Nothing counts those aborts, refuses a target that keeps failing, or drops one
from the menu** — noticing that the same square twenty times means a blocked route is deliberately
the model's job, and a menu that silently withheld a reachable-looking action would be a worse lie
than a repeated failure. ⚠️ **`at` is in the *expanded* coordinate space** the ids, the map picture
and every turn's `Location:` line use, never `raw_player_coords`, which is the same square before the
connection-strip offsets; `MetaTileMap::player_position` is where it comes from, and the one abort
that reports `None` on purpose is `WrongMap`, where a coordinate would be read against the wrong map.

Three traps in the detection, each paid for separately:

- ⚠️ **It is what the player is *facing*, not what it set out for.** A script can open a box
  mid-walk — the rival's, two tiles short of the aide in Oak's lab — and "my destination was a
  sprite" calls that a conversation the run never had.
  `mechanics::a_script_that_interrupts_a_walk_is_still_an_abort` is that case, on the fixture the
  first version of the test was written against.
- ⚠️ **A PC is not in `meta_tiles`**, so the tile in front of a player using one reads as
  `Obstacle` and matching on the tile answers no — silently, and for PCs only. It is a hidden event,
  indistinguishable from the wall it is drawn on, which is the whole reason `pc_locations_for` is a
  transcribed table; the coordinate is the only thing that identifies one.
- ⚠️ **"Facing" has to mean what the game means by it, which reaches *over* a counter.** Gen 1 talks
  through the tileset's `wTilesetTalkingOverTiles` (`MetaTile::Counter`), which is how a nurse, a
  mart clerk and every gym receptionist are spoken to: `actions()` routes to the far side of the
  desk, so the tile in front is the counter and never the person. Matching the literal front tile
  reported every heal in every Pokémon Centre — the most repeated action in the deployed run — as
  "✗ gave up on Nurse". `MetaTileMap::interaction_in_front` is the one that hops; ⚠️
  **`tile_in_front` must not**, because `cut` and the surf mount are about the literal tile.

⚠️ **What the page shows and what the model is told are two different lists, and the split is on the
client.** `useEventStream`'s `fold` drops `text_box` and `overworld_interaction_completed` — the
screen's dialogue is already on screen in the game's own font, and "✓ talked to Mom" says less than
the line of Mom's that follows it. Both still go to the model (`describe_event` renders every
`TextBox` into `### Since your last decision`) and both are still written to `transcript.jsonl`.
⚠️ **Do not "simplify" this by filtering at the publish instead**: `run::transcript` writes what is
published, so that deletes the dialogue from the run's archived record and from `/api/history`, which
is the one copy nothing can rebuild.

⚠️ **A tool call and its result are two events and one row.** `UiEventBody::ToolCall` carries the
endpoint's own call `id` and `CallKind`'s label; `ToolResult` is paired to it by that id, in
`useEventStream`'s `attachResult`. ⚠️ **Paired on the id, never on position or name** — a turn can
call several tools in one message and they are answered as a batch, so the second result is not the
second-to-last row and two `read_party` calls in a turn are indistinguishable by name. ⚠️ **Neither
row may go through `push`**, for the reason a streamed reply does not: a row that grows must never be
collapse-matched, or an answer attaches to the wrong call.

⚠️ **A tool's *picture* is referenced, never carried.** A map render is a couple of hundred kilobytes
and every published event is also a line of `transcript.jsonl`, so putting it on the event would grow
the archive by a base64'd PNG per read for the length of the run — the same arithmetic that took
base64 out of the video path. `ToolResult.image` is a flag; the bytes live in a 16-entry ring in
`Published` keyed by the **seq of the event that announced them** (so the publish must happen before
the `put`), and `/api/tool-image/{seq}/image.png` serves them. ⚠️ **A 404 there is the expected
answer, not a fault** — anything older than the last handful is gone and the page shows the caption
alone. The text is truncated at `MAX_TOOL_RESULT` for the same reason, server-side: a truncation the
client applies is one that has already been broadcast and written to disk.

⚠️ **The status heartbeat is sent on change, not on a timer.** Sampled at `GB_STATUS_HZ` and
published only when it says something the last one did not, with a 2 s keepalive so an idle run still
proves it is alive and `curl -N /api/events` still ticks. At the original 10 Hz unconditional it
measured **49.7 kbit/s per viewer** — six times the idle video feed, nine of ten payloads
byte-identical to the one before; it is now 5.2. Two consequences: `StatusSnapshot` compares with
`says_the_same_as`, which excludes the clocks and `frame_seq` (a derived `PartialEq` would never
match and the suppression would silently never fire), and `/api/events` **opens with the latest
heartbeat** — `Published::join_events`, subscribe-then-read, the same handshake as the video keyframe
— or a page opened during a quiet stretch shows an empty panel. ⚠️ Anything *added* to the snapshot
has to be added to `says_the_same_as`'s destructuring **and** compared there: the pattern is
exhaustive, so a new field is a compile error, but binding it and not comparing it is not — and a
change nobody compares is a change nobody is told about. `dropped_ms` is the newest one, and is the
counter-example to the exclusion above: it is a clock and it *is* compared, because it stands still
on a host that is keeping up, so it forces no heartbeat in a healthy run and the moment it moves is
the moment worth telling a viewer about.

⚠️ **A lifetime average is not a rate, and `emulated_ms / wall_ms` is the average.** The page's speed
line was that ratio, and it read below 1× for ever on a host running at full speed. Two mechanisms,
both permanent, neither recoverable — an average can only ever converge on the truth from below:

- **`MAX_CATCHUP` drops emulated time on purpose** (`host.rs`, and its own comment says "Better to
  drop the time"), so any iteration that overruns 250 ms — a busy container start, a long
  checkpoint, a descheduled process — is subtracted from the numerator for the rest of the run. The
  deployed run measured a **14.85 s** startup debt it was still carrying five minutes later, against
  an instantaneous speed of exactly 1×.
- ⚠️ **`wall_ms` was measured from a clock stamped at construction while `emulated` is zeroed by
  `start_new_run`.** Pairing a counter that resets with one that does not meant a run started in a
  process that had been up for hours opened by reporting its first seconds against those hours: the
  panel read near-0× after every `/reset-game` and Hall of Fame swap and crept up for hours.
  `progress()` had always used the correctly-paired `run_started`; the heartbeat now does too, and
  the construction clock is gone. (The comment that justified keeping it claimed it was what
  `/api/healthz` reports as uptime. It never was — that is `state.started` on the HTTP state.)

⚠️ **The tell that it was accounting and not the emulator is that the deficit was *constant*.** Two
`/api/events` samples 90 s apart put `wall_ms − emulated_ms` at 14908 ms and 14845 ms — shrinking by
0.0704 ms/s, which is `MachineCycles::to_duration` truncating 953.674 ns to 953: the loop spends
953 ns of wall clock per m-cycle and reports them back at the exact rate, so **1.0007× is the ceiling
and a healthy host reads exactly that**. A genuinely slow host shows a *growing* gap. `dropped_ms` on
the heartbeat now says it outright, and the page derives speed from the difference between
consecutive heartbeats (`useEventStream`'s `sampleSpeed`, a 500 ms window because at `GB_STATUS_HZ`'s
default two samples can be 100 ms apart and `ahead_by_cycles` makes a window that short meaningless).
⚠️ **A park needs no case in that and must not be given one**: the host stops the emulator *and*
subtracts the wait from `wall_ms`, so both counters freeze, no window closes, and the last live
reading is held under the PAUSED plate — where a `dw > 0` guard would report `0.00×`.

⚠️ **`RunProgress::wall_ms` was the same bug's other half**: the field doc said `paused_total` "is
subtracted from the `wall_ms` this run reports" and it was true of the heartbeat and false of
`progress()`, which is what `meta.json` and `hall-of-fame/ledger.jsonl` record — so a run parked
overnight on a spent quota wrote the whole night down as play. Both paths subtract it now.

⚠️ **Send-on-change needs a cell per thing sent, and the plan was the one without.** `join_events`
opened with the heartbeat alone, so the model's plan — published only when it *changes*, which can be
an hour — was never replayed to a page that had just loaded. The other route was no better:
`/api/history` keeps the most recent `MAX_BACKLOG` (2000) events, and a reasoning model publishes one
event **per streamed token**, so the last `Plan` falls off the end within minutes. Both failed for
different reasons and the symptom was neither: `PlanPanel` renders nothing for an empty list, so the
panel was simply absent and it read as a styling bug. `Published::latest_plan` is the fix, on the
same subscribe-then-read handshake as the heartbeat and the video keyframe, and it works because a
`Plan` event is *absolutely* stated — the whole list, every time — so replaying the newest is
complete. ⚠️ **Anything else that becomes send-on-change belongs in `join_events` too.**

⚠️ **Both keepalives are load-bearing on the client, because a dead connection is otherwise
indistinguishable from a quiet one.** The page's two retry loops were error-driven — `onerror` on the
`EventSource`, `catch` on the video `fetch` — and both are right about every case that *produces* an
error: the server closing, a refused connection, a restart. A network going away produces none. No
FIN and no RST arrive, the socket stays open as far as the browser is concerned, and a stream we only
ever read from sends nothing that could time out, so the page froze on its last frame **still showing
the green live pill**, for as long as it was left open. Measured at 75 s and it was not going to
recover. `STALE_MS` (`web/src/api.ts`, 4× the server's 2 s `KEEP_ALIVE`) is the fix: silence, not an
error, is the signal. Two traps in feeding it. ⚠️ **Not the SSE keep-alive** — that is a comment line
and `EventSource` hands it to no callback, so a watchdog on it starves and reconnects every 8 s for
ever; the status heartbeat is the one that arrives. ⚠️ **On the video side, not the messages
`readVideoStream` yields** — its keepalive is a zero-length message that yields nothing, on purpose,
so a watchdog fed from the messages would fire on a screen that is merely not moving. It is fed from
the inflated chunks instead. ⚠️ And `subscribeVideo` needs **one `AbortController` per attempt**,
chained to the caller's: aborting the caller's own signal is how that loop is told the component
unmounted, so a watchdog that used it would kill the retry along with the connection. Reproduce with
`kill -STOP` on the process — a blackhole, where `docker stop` or Ctrl-C is a clean close and
exercises the path that already worked.

⚠️ **A reconnect of `/api/events` is also a reload of the transcript, because the stream alone
cannot catch a page up.** A fresh connection opens with the latest heartbeat and the latest plan
(`join_events`) and nothing else, and `/api/history` used to be fetched once, at mount — so every
reconnect, the watchdog's included, resumed a log with a hole in it that nothing would ever fill. A
tab left dormant came back showing the hour-old log, for ever, with the live pill green. Now
`subscribe` reports every `onopen` (the browser's own transparent retries included, since their
`onopen` fires again and they are a gap too) and the hook answers it by resetting everything the old
connection folded — entries, the pending queue, plan, usage, the speed anchor — and fetching
`/api/history` afresh: the video path's "every connection opens with a keyframe", applied to the log.
Three traps. ⚠️ **The reset has to be inside `onopen`, before `alive`**, so it lands ahead of the
opening heartbeat and plan rather than throwing them away. ⚠️ **The backfill is generation-guarded**:
a fetch started by the old connection can resolve after the new one has reset the page, and its rows
are the stale ones. ⚠️ **A hidden tab is resynced on return whether or not its socket died**
(`visibilitychange` after more than `STALE_MS` away, `pageshow` with `persisted`): a backgrounded tab
gets no animation frames, so `pending` overflows on a perfectly healthy connection, and the
watchdog's own timer is throttled along with everything else, so waiting for it is waiting for
nothing. A short tab flip keeps the connection — it is not a gap. This is deliberately a full reload
and not a `since=` merge: `transcript::read_since` reads from the tail so a bare backlog is cheap,
and a merge of a *folded* log across a gap (a half-grown streaming row, a tool result straddling the
boundary) is where the bugs would live.

⚠️ **Every `UiEvent` carries `at`, a Unix-millisecond wall-clock stamp, and it is the only clock the
page can date a line by.** `wall_ms` and `emulated_ms` are both elapsed times *since this process
started*, so a run resumed nightly reports them from zero; the browser cannot supply one either,
because `/api/history` replays a backlog that may be hours old and a client-side clock would date the
whole backfill to the page load. It is stamped in `publish_event`/`publish_status` and therefore
lands in `transcript.jsonl` too. ⚠️ **The SPA's copy is optional and must stay optional** — the runs
on disk predate the field. Also ⚠️ `useEventStream`'s `signature` excludes it, for the reason it
excludes `seq`: it differs on every event, so leaving it in would stop identical rows ever collapsing
into a `×3` again.

## The model's side

⚠️ **Message 0 is a constant, and it has to stay one.** A prompt cache is keyed on the *prefix*, so
anything dynamic in the system message throws away the cached prefill of the **entire** conversation
the next time it changes — the cache discount on a hosted endpoint, and seconds of re-prefill on a
local one. The model's plan used to live there, re-rendered on every request, which meant every
`todo_add` paid that. It is now `prompt::plan_message`, a `user` message of its own, and
`Worker::sync_plan` emits it **only when it differs from the copy already in the history**:

- unchanged → nothing happens, the history grows purely by append and the cache is intact;
- changed → the stale copy is removed and a fresh one appended, so the break is at the last turn the
  plan changed rather than at the top. A model that edits often pays little, one that edits rarely
  pays rarely;
- absent (a compaction took it) → appended, which is what makes the whole thing self-healing.

⚠️ It is deliberately **not** a turn boundary (`compaction::is_turn_start` excludes it), or a cut
taken between the plan and the situation it belongs to would drop the one thing meant to survive.
`the_plan_is_carried_once_and_never_disturbs_the_cacheable_prefix` holds both halves.

⚠️ **Every terminal tool takes a required `summary`, enforced, and it is the only thing the model
says about a turn that outlives the turn.** Reasoning arrives on a channel that is deliberately
never sent back, and most models emit no `content` at all beside a tool call — so the assistant side
of the history was a column of bare JSON: every turn saying what it did, not one saying why. A model
reading that back has no record of having *tried* anything, which is the state in which it walks
into the same building for the fourth time. It rides on the terminal call's own arguments rather
than in a message of its own, because that is the one place a sentence costs no extra round trip,
cannot be separated from the decision it explains, and lands in the history by itself
(`Message::assistant` carries `tool_calls` **almost** verbatim; see the poisoning ⚠️ below). ⚠️ **It
used to be required in the schema and optional in the parser**, on the argument that enforcing it
would not get it filled in — a rejected call does not end the turn, it becomes another tool result
and spends another of `GB_MAX_TOOL_STEPS`, pushing a forgetful model towards the forced `wait`
rather than towards remembering. That was settled by measuring what enforcement would cost: across
the deployed run's **2427 decisions only 98 carried no summary and every one of them was a `wait`**
— the *synthesised* fallback wait, which never goes through `classify` at all. The model already
fills it in on every real action, so the rule costs that model nothing and closes the hole for the
one that would not. `classify` now rejects a terminal call without one, in one wrapper rather than a
check per arm. ⚠️ It is added by `add_summary_argument` post-hoc in `for_kind`, so it scales with
the *number of terminals a kind offers* rather than with the catalogue — which is what moved
`the_tool_array_stays_within_its_budget`'s ceilings, deliberately. It reaches the page as
`Decision.narration`, beside `worker::describe`'s mechanical `summary`.

⚠️ **`press_buttons` is offered on exactly one decision kind — `Stuck` — and everything below is the
history of finding that out.** It is an escape hatch for an agent that has stopped working, and on
any turn that has a menu it is a way to finish a turn without choosing from it. The deployed run
proved that is what a weak model uses it for: **749 presses, 738 of them overworld turns with a
perfectly good menu, 0 at the watchdog's turn**, ending in a run of **91 consecutive** presses
oscillating in a 13-tile box on Route 3 while `Route3:0,10:Connection — walk into PewterCity` sat in
the menu every time. ⚠️ **Nothing had failed *there***: the last `choose_action` before that run
succeeded, and no id was rejected anywhere near it. ⚠️ **The stronger claim this used to make — that
`choose_action` was never once rejected in the whole run — is false, and was measured wrong.** Across
the run **59 of 934 `choose_action` decisions named a map the player was not standing on**, every one
of which `resolve_overworld` had to refuse; see the ⚠️ on ids the turn never offered, below. It does
not change the conclusion about the presses, since none of those refusals sat anywhere near the Route
3 run, but it was quoted as evidence and it is not evidence. The tell that the pressing is
self-reinforcing rather than caused is the ratchet — 26% → 2% → 12% → 74% → 38% → 72% → 100%,
recovering to 2% for 600 turns in the middle, because the model reads its own last three turns back
on every request.

Two rounds of friction were tried on the tool and both failed, which is the general lesson:

- "A last resort" in the description. Prose cannot be checked afterwards.
- A required **`why`**. ⚠️ **It was required in the schema and optional in the parser** — the same
  trade `summary` made, on the same argument that a rejection spends a `GB_MAX_TOOL_STEPS` and pushes
  a forgetful model towards the forced `wait`. **543 of 749 presses left it null.** A field that is
  optional in practice is a field a weak model omits, so the record it existed to make readable was
  three quarters blank. It cost 239 bytes and bought nothing.

⚠️ **So on `Overworld` and `Battle` the tool is not in the catalogue at all**, and what replaced it is
`report_issue` — see below. `why` survives at `Stuck` and **is now enforced** (`tools::classify`),
because that is the one turn where a press is the right answer and so the one place the record is
still worth making readable. Both changes together took Overworld and Battle *down* ~450 bytes each.

⚠️ **A press at `Stuck` is not a fault**: there is no menu, so it is the model doing as it was asked.
`incident.json`'s `kind` still records which turn asked, and its `report` field says `press` or
`issue`, so the two can be counted apart without inferring it from which fields are null.
- **The conversation slice is image-evicted, and that is not an optimisation.** Three turns, cut on
  `compaction::is_turn_start`, through `compaction::evict_images(.., 0)`. A history holding a map
  render is hundreds of kilobytes of base64 *per message*, and a model that has decided to press
  buttons tends to do it again next turn.
- **Nothing new is published and no picture rides on an event** — same arithmetic as the tool-image
  ring. The page already shows the press as `Pressed a, b` off the `Decision` event. And the path is
  re-read from `run::CurrentRun` per record, never captured, or a press after `POST /api/new-run`
  lands in the run that was already set aside.

⚠️ **`report_issue` is what a turn with a menu gets instead, and the thing that makes it work is that
it does not end the turn.** The hatch was reached for because it was the one way to finish a turn
without choosing from the menu, so a *terminal* replacement would be the same tool renamed. The model
files the complaint and still has to call `choose_action` or `wait`; "the menu will not let me do X,
so I am doing Y" is one message, and both halves happen. Three ⚠️s:

- **Its `message` is enforced**, unlike everything before it. A report is *only* its message, so
  rejecting an empty one costs a tool step and loses nothing — which is the test the two enforced
  fields pass and `summary`'s old argument did not.
- **The answer must not read like a fix.** `Worker::file_issue` says filed, nobody is coming, this
  did not end your turn, try a different way. A reply that sounded like something had changed would
  have the model wait for the change and produce an identical turn.
- **It is offered on `Overworld`, `Battle` *and* `Stuck`** (`tools::offers_issue_report`). The
  watchdog's turn is where the agent is most likely to be genuinely wrong, so it is the last place to
  withhold it; the three single-question prompts have nothing for the agent to get wrong and carry
  neither tool.

Records land in `$GB_RUN_DIR/<run-id>/issues/turn-<id>/` beside `press-buttons/`, same shape:
`{incident.json, screen.png, state.gbst}`.

⚠️ **The save state is taken at the *start of the turn*, by the emulator thread, and left in
`Published`** — `EmulatorHost::tick` on the edge into `RunStatus::AwaitingLlm`, never on the level, or
it is 50 states a second for the length of every turn instead of one. It has to be there because
`GameBoy` exists on that thread and the worker has no way to ask for one. ⚠️ **The obvious cheap
version — copy the run's `state.gbst` — was written first and is wrong**: that is the last periodic
checkpoint, up to a minute behind, which is a minute of walking, several battles, or the very
transition being complained about. A state is **24 µs and 6.4 KB** (measured on Pokémon Red,
2026-08-25), so one per turn is cheaper than the copy it replaced. ⚠️ It is still not free enough to
do every tick: `MAX_CATCHUP` turns anything on that path into dropped emulated time rather than a
slow frame.

⚠️ **A tool call the model writes badly poisons the conversation, not the turn, and a router is what
made that a daily event.** `arguments` is a JSON string the *model* produces, and the assistant
message carrying it is replayed on every request for the rest of the run. So one completion emitting
`""` or a fragment cut off mid-object is rejected for ever after by any endpoint strict enough to
parse what it is sent: `tool_calls[].function.arguments must be a JSON object` (400),
`Expecting ',' delimiter: line 1 column 22 (char 21)` (400),
`Failed to apply prompt template: cannot convert value into pairs` (502) — 331 of them in one
backlog against `openrouter/free`. ⚠️ **The tell is that the parse error is character-identical every
time**: same column, same char, because it is one stored message being re-sent rather than a run of
unlucky completions. It never happened against a single local model, because a model that writes
clean arguments writes them every turn; a router hands the conversation to a different one each
request and it takes one. `protocol::history_safe` rewrites anything that is not a JSON object to
`{}` inside `Message::assistant`, which is the one funnel every history entry goes through. ⚠️ **In
place, never dropped** — removing the call would orphan its `tool_result` and break the invariant
one-step rollback rests on — and ⚠️ **only the broken ones**, since `serde_json` sorts keys and
canonicalising every call would reword the model's own history for nothing.

⚠️ **There is one notes mechanism and there used to be two.** `memory_write`/`memory_read` over a
`memories/` directory sat beside `todo_add`/`todo_complete` doing the same job in a different shape:
four tools' worth of schema in every request, two places for the same sentence, and a choice for the
model to get wrong. The plan won — it is the one that renders on the page, for both audiences — and
the freeform role it gave up is filled better by the compaction summary, which is written with the
whole history in view. What only the plan does is survive a *process* restart, since the history is
never persisted; that is why `MAX_TEXT` is long enough for an item to carry its own reason.
`run::files::MEMORIES` survives only so an archive of a run made before this is still complete.

⚠️ **A read the situation already answered is worse than no read at all** — it is a round trip bought
for nothing, and it teaches the model that a turn opens by reading. `read_screen_text` answered from
the same `observe::screen_text` the turn renders under `### On screen`; `read_trainer` returned
badges, money and play time, all of which are in the turn's header (its two genuinely absent figures,
the dex counts, moved there and the tool went). Same trap one level down: `MapView` carried `actions`
and `BattleView` carried `options`, which were second copies of the turn's own menus **without the
ids** — a list of choices every one of which would be rejected, since an id is minted in the tool
layer from `MetaTile::kind` and those views never had one.

⚠️ **Reads are scoped per decision kind, not just terminals.** Every kind used to be offered all
eight: a battle turn paid for `read_map`, a naming screen carried the whole catalogue to answer with
one word. Beyond the tokens, a tool that can only ever answer `null` is an invitation to spend a
round trip finding that out. `ReadTool::kinds` is the table; `non_terminal_names` is therefore
per-kind too, or the contract at the bottom of a turn would name a tool the request did not carry.
⚠️ A read that exists but is not offered *here* is rejected by name with the reason — falling through
to "there is no tool called `read_map`" is a lie the model cannot act on.

⚠️ **A menu row says what choosing it does, in words, and carries nothing else.** The row is
`` `{map}:{x},{y}:{kind}` — {verb phrase} ``: `take the warp to PalletTown`, `walk into Route1`,
`talk to Mom`, `pick up the Potion`, `read the Pokedex 1`. It has been two other things. First
`` `OaksLab:5,11:Warp` — Warp → PalletTown (12, 11) — 10 steps ``, where the verb is the `kind` said
twice and `(12, 11)` is a coordinate on a map the model cannot see; then, over-corrected, a person's
row was the bare distance (`` `MtMoonB2F:15,23:Rocket2` — 5 steps ``) and the deployed model took
Rocket2 for a warp forty-five times. The id is a key and the row must not make the model parse the
action out of it, so a person's name is now said once more, on purpose, after the verb. ⚠️ **The
verb comes from the sprite's `PictureId`, never its name**: everything the player can face is a
sprite to the game — items, boulders, fossils, the lab's Pokédex, Snorlax — and only the picture
tells `Potion1` from `Hiker`. The step count is gone for good; nothing ever used it.
`a_menu_row_explains_the_action_in_words` and `a_sprite_row_is_verbed_by_its_picture` hold it.
⚠️ **The `{map}` prefix of the id is not redundancy**: `resolve_overworld` re-mints ids against the
map the player is on *now*, and an answer can land after a warp — so without it, `5,6:Warp` chosen
in Oak's lab could match a warp that happens to sit at (5, 6) in Pallet Town and be carried out
silently.

⚠️ **An id that is not in the turn's own menu is refused by `tools::classify`, inside the turn,
before it can ever be a decision.** `TurnRequest` carries the menu's ids for exactly this
(`tools::not_on_the_menu`), and the check is worth the plumbing because of where the alternative
lands: an id the model invented used to be classified as a `Terminal`, published as a `Decision`,
sent to the policy and only then refused by `resolve_overworld` — at which point the turn is over and
the complaint can only ride on the *next* turn's situation, which is a second full prefill of a
~55 k-token history. Caught in `classify` it is an ordinary `CallKind::Rejected`, so it costs one
more completion and the model still acts in the same turn.

⚠️ **It is measured, not hypothetical: 59 of the deployed run's 934 `choose_action` decisions named a
map the player was not on** — `ViridianCity:33,8:Warp` while standing in `ViridianPokecenter`,
`MtMoon1F:25,15:Warp` while on B1F — because the model was quoting a menu from several turns earlier.
Since `overworld_id` prefixes with `state.map.map`, not one of them could ever have resolved.

Three traps in the check:

- ⚠️ **An empty menu checks nothing.** `Nickname`, `ForgetMove` and `Stuck` carry no menu at all, and
  reading "no menu" as "nothing is allowed" would reject every answer they give.
- ⚠️ **`resolve_overworld` is still the authority and must stay.** The menu is minted when the turn is
  built and re-minted when the answer lands; this catches what the model was never *offered*, and
  that one still catches what stopped being true in between. Belt and braces, not a replacement.
- ⚠️ **The complaint has to name the right mistake.** The policy's note used to say "the game moved on
  while you were deciding" for every failure, which for a stale id is a misdiagnosis that invites the
  model to try it again — the world had not moved, the model had. Both messages now compare the id's
  map prefix against the map the player is on and say which of the two mistakes it was. The check
  reads the current map out of the menu's own first id rather than being handed it separately, so
  the two cannot disagree about where the player is.

⚠️ **`PokemonStatus`' `Display` is `strum`'s derive, so a healthy Pokémon prints `None`** — and every
party line in every turn read `20/20 HP, None`, which is a missing value rather than good news.
`prompt::ailment` says nothing at all when there is nothing to say; the HP beside it already reports
how the mon is doing. Same class of bug as `MetaTile`'s old `strum` `Display` (see the agent
section): a derive is a debugging default, and every one of these strings is prose a model reads.

⚠️ **`read_route` runs the search; `read_world_graph` shipped the graph.** The old tool serialised
every visited `(map, entry)` node with all its edges — unbounded by construction, and by the late
game a meaningful fraction of the window in a single tool result. Nothing ever wanted the adjacency
list; the question is always "which way is Celadon". ⚠️ Its `None` is **negative**, and the wording
has to keep saying so: no route means "you have not walked there yet", never "unreachable" — a run
that read it the other way would stop exploring.

⚠️ **A reasoning model streams its thinking on a channel of its own, and it is not `content`.** LM
Studio, vLLM and DeepSeek send `reasoning_content`; OpenRouter sends `reasoning`; OpenAI sends
neither. Before `MessageDelta` knew the field, serde dropped it as an unknown key — the page showed a
blank turn for however long the model thought, and on a local 12B that is **three quarters of the
completion tokens of a trivial overworld step**. `read_stream` now reports a `Fragment::Content` or a
`Fragment::Reasoning`, which is two channels rather than one string because they have opposite fates:
⚠️ **the reply goes back into the history and the thinking never does.** It is billed once as
completion tokens; a copy in the history pays for it again on every turn for the rest of the run.
`Usage::estimate` counts it anyway — the endpoint charged for it, and that estimate is the bill.

⚠️ **A thought is closed by the next thing the *model* says — not by the turn ending, and not by the
next event of any kind.** Both wider rules were tried and both are wrong in a different direction. On
the turn: a turn that reads before it decides thinks once per completion, so grouping on `turn` welds
two thoughts around the tool call between them. On the next event: **the emulator never pauses while
the model thinks**, so the agent narrates over the top of every thought it has ("→ heading for Mom",
"✓ reached the warp to PalletTown") — and a fold that closed on those shredded a one-minute thought
into five rows, four of them collapsed to `thought for 9 words`. `useEventStream`'s `MODEL_SIDE` is
the line, `lastModelSide` finds the row, and ⚠️ **`Conversation.tsx` must read liveness the same
way** — one decides what the row contains and the other how it is drawn, and they cannot disagree.

⚠️ **The live thought scrolls in a box of its own, and pinning the log does not pin it.** It is capped
at a few lines so it cannot bury the log, which means the tokens arriving land *below* the visible
part of it: measured on the deployed run mid-thought at 222px of text in a 117px box with `scrollTop`
0, so what a viewer watched for the length of a completion was its first nine lines, frozen. The
`.body` element is followed separately, on the same terms as the pane above it.

⚠️ **An uncapped completion is bounded only by the context window, and that is not a bound.** A
reasoning model that falls into a repetition loop generates until the window fills: measured at
**~26 000 tokens** on turns that normally cost 24–2 000, twice in twenty-five minutes, each one
holding a single-slot endpoint for the full ten minutes our deadline allowed. `GB_MAX_TOKENS` (8192)
is the ceiling and `0` removes it. ⚠️ **A truncated reply is nudged differently from a silent one**
(`prompt::truncated_nudge`): told only "that reply contained no tool call", a model cut off
mid-thought concludes it forgot to call one and tries again at the same length, into the same
ceiling.

⚠️ **`reasoning_effort` is an on/off switch on LM Studio, not a dial — and it was measured rather
than assumed.** With gemma-4: `none` takes reasoning to *exactly zero* tokens while still answering
correctly, `low` is indistinguishable from the default (174 → 159 tokens, noise), and
`chat_template_kwargs` in either spelling (`thinking`, `enable_thinking` — the Qwen convention) is
accepted and silently ignored. `GB_REASONING_EFFORT` passes the string through unvalidated, because
the vocabulary belongs to the endpoint and refusing a value it would have taken is worse than
forwarding one it rejects in a 400 whose body we keep.

⚠️ **Giving up on a request is not free, and a timeout is not a transport failure.** A connection
that never opened consumed nothing at the far end, so retrying it is free and correct — that is what
`LlmError::Transport` means. A request the endpoint *accepted* is being worked on, and llama.cpp says
so when we hang up: "Stopping generation… (If the model is busy processing the prompt, it will finish
first.)". So `LlmError::Timeout` is a separate variant and is **not retryable**: on a server that
runs one request at a time, the retry queues behind the very request it replaces and can never be
faster, while adding a second generation nobody will read. `GB_REQUEST_TIMEOUT_SECS` (180) is the
matching knob and wants to be *raised* for a local endpoint rather than lowered — waiting costs a
stalled turn, giving up early costs the same turn plus the endpoint's next few minutes.

⚠️ **A failure can arrive *inside* a 200, carrying the status the retry table wants, and it used to
be thrown away twice over.** A router that cannot reach an upstream has already sent its headers, so
it says so in an ordinary `data:` frame: `{"provider":"Nvidia","choices":[],"error":{"code":504,
"message":"Provider timed out after 47709ms","metadata":{"error_type":"timeout"}}}`. Two faults, one
on top of the other. ⚠️ **`code` is an integer on OpenRouter and a string on OpenAI**
(`"insufficient_quota"`), and typing it as one made the other unparseable — so the whole chunk failed
and the run reported *its own parser* as malformed for what was a provider timing out. And ⚠️ **the
status was then flattened to `LlmError::Protocol`, which is not retryable**, so the textbook
transient failure the backoff exists for was the one thing it never saw. `ErrorCode` now takes both
spellings and `ApiError::into_failure` maps the status onto the same table a non-200 goes through
(429 → an *undated* `RateLimited`, since a body carries no headers to park until; 5xx/408 retryable;
other 4xx fatal). ⚠️ **A 504 here is not an `LlmError::Timeout`, despite the word** — that variant is
*our* deadline expiring on a request the far end is still working, which is precisely why it is not
retried; here the router has already given up and said so, so nothing is left running and another
attempt is ordinary. ⚠️ **And the classification is scoped to OpenRouter by the frame recognising
itself** — a chunk-level `provider` or an `error.metadata`, nothing else sends either — rather than
by sniffing `OPENAI_BASE_URL`, which is any OpenAI-compatible endpoint and has no vendor concept in
it. A bare `{"error": {…}}` keeps the old `Protocol` exactly, so no other provider's failures start
being retried on the strength of a number we decided to trust, and a proxy in front of OpenRouter
still works. `a_bare_error_frame_is_still_only_a_protocol_error` is the guard for that half.

### When the endpoint says no: the park

⚠️ **A rate limit is the one failure where the retry is itself the problem**, and the ordinary
backoff made it worse rather than better. Every attempt is another request counted against the very
quota that is exhausted, so five attempts against a daily cap spend four more of an allowance that
has already run out and fail four more times doing it — then the turn resolves to
`FAILURE_WAIT_TICKS` (2 s of game time) and the next decision point asks again. On OpenRouter's free
tier (50 requests/day below $10 of lifetime credit) that burns the whole day in under two minutes
and then hammers the endpoint for ever. `LlmError::RateLimited` is the separate variant, on the same
argument that made `Timeout` one.

What the endpoint hands back instead is a *time*, and the only thing that works is waiting for it:

- ⚠️ **The client must read the headers before the body**, because reading the body consumes the
  response. `Retry-After` and `X-RateLimit-Reset` are the whole of what makes a 429 actionable.
- ⚠️ **`X-RateLimit-Reset` has no agreed unit**, and `protocol::reset_at_ms` sniffs it from the
  magnitude: OpenRouter sends Unix **milliseconds**, several OpenAI-compatible servers send Unix
  **seconds**, others send **seconds from now**. Both misreadings are silent and bad in opposite
  directions — a Unix-second stamp read as a delta parks the run for thirty years, a delta read as a
  stamp resumes instantly into the same 429. ⚠️ And `None` means "the endpoint did not say", which is
  *not* a reason to park: an undated 429 is far more often a per-minute limit, so it keeps the
  ordinary backoff. `stream_with_retries` only declines to retry when the reset is further out than
  `policy.max`.
- **`Worker::park_until` waits it out**, publishing `RunStatus::Throttled { until_ms }` and stopping
  the emulator with `Published::set_throttled_until`. ⚠️ **The release is unconditional** — a return
  that leaves the cell set stops the game for the rest of the process. ⚠️ It is **clamped**
  (`MAX_PARK`, 25 h) because the number came from the endpoint, and ⚠️ the turn then re-sends **the
  same request**, which is only sound because the emulator was stopped: the situation it describes is
  still on screen when we wake.
- **`EmulatorHost::tick`'s pause seam** is where the game actually stops. ⚠️ **Below the reset and
  completed-run seams**, so a parked run still answers `POST /api/new-run` — that is how a park is
  escaped if anything goes wrong with it, and `start_new_run` releases the cell itself rather than
  depending on the parked thread. ⚠️ **It skips the emulator and the agent only**: the heartbeat, the
  video and the checkpoint still run, because a paused run that published nothing is *indistinguishable
  from a dead connection* to the page (`STALE_MS`) — and since send-on-change is silent while the
  screen is frozen, what actually feeds the page for the length of a park is the 2 s keepalive.
  ⚠️ **No catch-up debt**: `since_last_update` is zeroed, or the game fast-forwards through
  `MAX_CATCHUP` the moment the quota reopens. `a_parked_run_stops_the_game_but_keeps_the_page_fed`
  holds all of it.
- **The cartridge clock needs no help and the wall clock does.** `wPlayTime` is emulated, so stopping
  the emulator stops it — which is the whole reason the pause beats merely holding the requests back,
  since the leaderboard ranks on it. `wall_ms` is elapsed real time, so `paused_total` is subtracted
  from it: a run parked overnight must not report the wait as time it spent playing.
- **On the page** the last frame is dimmed under a PAUSED plate with a live countdown
  (`Screen.tsx`'s `PausedOverlay`). ⚠️ **The deadline is published once and the countdown is derived
  on the client**, so an hours-long park costs no traffic; that is also why `until_ms` is an absolute
  Unix millisecond rather than a remaining time, since it is replayed on every heartbeat and to every
  page that joins later. ⚠️ The plate sets its own `line-height`: `.screen` sets `line-height: 0` for
  the canvas, which collapses a stack of spans into one overlapping line.

⚠️ **A local endpoint's real limit is its KV cache, not its advertised window, and the arithmetic is
per *slot*.** A run against LM Studio wedged for 28 minutes at a time with no error anywhere: llama.cpp
was configured `n_parallel=4, n_ctx=60000, kv_unified=true`, so four request slots *shared* one
60160-token cache. Anything that made the server pick a fresh slot rather than reuse the prefix left
the old slot still holding its copy of the history, and four copies of a ~16 k conversation is 66 k
against 60 k — after which no request could be allocated a slot at all. The log's tell is a
`slot selection` line with **no `launch_slot_` after it**; prefill itself was never the problem
(360–490 tokens/s, and prefix reuse made it ~8 s). One slot with the whole window is the fix, and it
is the model server's setting, not ours.

**`GB_COMPACT_ABOVE` (0.85) is what 0.70 used to be**, and the old number was never measured against
anything: it was headroom picked for a 128 k window, where a fifth of the context is tens of
thousands of tokens held empty and a summarising completion — the most expensive thing the loop does
— is bought sooner and more often than it needs to be. ⚠️ **What the headroom actually has to cover
is absolute, not proportional**: compaction runs *between* turns, so a turn already under way grows
unchecked to `GB_MAX_TOOL_STEPS` completions and their results; and stage 2's request carries the
whole history plus room for the summary written back. 15% of 128 k is 19 k and comfortable, 15% of a
local model's 60 k is 9 k and merely adequate, 5% of 60 k will not fit a summary at all — which is
why the variable is refused outside 0.2–0.95 rather than clamped into it. Going over is not fatal in
either direction (a failed summary falls back to `trim_history`, a failed turn to a wait), but each
one costs the run its memory or a turn. ⚠️ The threshold is also a **test fixture**:
`a_full_context_is_summarised_and_the_next_turn_carries_the_summary` sizes its prose against it, and
a turn that lands under it makes that test pass by never compacting at all.

## The video stream

⚠️ **Quote the number for a screen that is *moving*.** W2 measured this honestly — the plan's §5.1
records 536 kbit/s walking against ~8 idle, and says so — but what reached the README was "about 19
kbit/s", which is neither. `src/web/video/bench.rs` puts four minutes of ordinary play through the
old stack at **565**, so the plan's worst case was the typical case. The bench is now the guard, and
it exists because the honest figure had to be re-derived from scratch to challenge the headline one:
seeded `RandomPolicy` over four fixtures with different screen behaviour, `--features bench`.

The stack is **v2 block diff → length-prefixed binary → one deflate stream per connection**, 21
kbit/s, and each layer earned its place against a measured alternative:

- ⚠️ **Compress the connection, not the message.** Per message it is 55 kbit/s; across the
  connection, 21. A Game Boy screen is repeated 8×8 tiles, so the same payload bytes recur within a
  frame *and* across frames, and only a window spanning the stream sees it. The cost is that the
  compressor is **per connection** — it cannot be done once in `Published` for everyone, which is why
  `VideoMessage` carries plain bytes now.
- ⚠️ **`VideoStream::frame` flushes after every message and must keep doing so.** Without the flush
  the encoder holds messages back until its buffer fills — correct for a file, and a livestream that
  arrives in bursts seconds apart. `the_video_stream_is_one_deflate_stream_of_length_prefixed_messages`
  inflates incrementally for exactly this reason; inflating at the end would pass either way.
- ⚠️ **Never base64 something you are going to compress.** It costs the well-known 33% before
  compression and **69–113% after**, because it shifts a repeating byte pattern into three alphabet
  phases and LZ77 stops seeing the repeat. That single fact is what took SSE off the table: SSE
  cannot carry binary.
- ⚠️ **The deflate is `Content-Type`, not `Content-Encoding`.** A declared encoding invites a proxy
  to inflate and re-deflate, which buffers whole messages and shows up as stutter only in production.
  The client inflates with `DecompressionStream('deflate')`.
- **Not a WebSocket**, though it was the obvious suggestion. Nothing here is bidirectional, and this
  module being unable to reach the emulator is the property `src/web/mod.rs` is built around. A
  chunked binary response gets the same bytes with no upgrade handshake and no ping/pong.
- **ffmpeg was measured, not dismissed.** x264 on the same footage is 45 kbit/s *lossless* and 25 at
  `-crf 28`, which visibly mangles four-shade pixel art. A macroblock DCT has nothing to offer a
  screen whose pixels take four values.

⚠️ **`gb serve` runs `GameBoy::dmg` unless `GB_HARDWARE=cgb`, so the screen is four shades — the
format is built on it.** On a CGB it is six (compatibility mode: BG and OBJ1 share the red ramp,
OBJ0 is the green one, white and black are common to both), which widens `bits_per_pixel` from 2 to
4 and measured **1.63× on the wire** — less than the 2× it costs before deflate, because the extra
bits repeat what the compressor is already matching. Nothing in the format changed to allow it: the
width has always been per message. ⚠️ **Non-power-of-two widths are not the saving they look like** —
3bpp for six colours is 25% fewer raw bits than 4bpp but only **13%** fewer on the wire, because an
unaligned bitstream shifts an identical tile payload into eight phases and LZ77 stops matching it.
Same mechanism as base64. v1
spent 4 bytes of every 23 on a per-block sub-palette that was always a permutation of `0,1,2,3`, plus
a mode tag and a block index, to carry a 16-byte payload. v2 has one index width for the whole
message (`bits_per_pixel`, 1/2/4/8, wide enough for the palette *after* this message's new entries)
and no per-block anything. The wide widths still work and are tested — a CGB stream would take 8 —
but nothing on this cartridge reaches them.

Two things left on the table, both measured in `bench_video_redundancy_still_on_the_table` and
neither built: 12–19% of changed blocks duplicate a block already on screen, and a global scroll
vector beats a straight diff on half to four fifths of moving frames. Deflate already collects most
of the first.

## Graphics out of the cartridge

No image is committed to this repo. The badges, the party sprites, the favicon and every tile,
person and letter of the map picture the model is sent are all read from `POKERED` at run time —
`src/pokemon/rom_gfx.rs` has the primitives, `badge_gfx`/`mon_gfx`/`map_gfx` the decoders, and
`src/web/sprites.rs` and `src/llm/map_image.rs` the palettes.

⚠️ **Bank 0 is not windowed and every other bank is.** A ROM pointer's address is a raw file offset
in bank 0 and a `0x4000`-based window everywhere else; `rom_gfx::rom_slice` is the one place that
knows. (`badge_gfx` used to have this wrong inline and got away with it because badges are bank 3.)

⚠️ **Tile order is row-major everywhere except a decompressed pic.** `rgbgfx` emits tiles left to
right then down unless pokered's Makefile passes `--columns`, and for these it does not — but
`AlignSpriteDataCentered` builds its buffer *column*-major. Comparing a decoded pic against
`pokered/gfx/pokemon/front/*.2bpp` with the wrong one differs on four fifths of the bytes with both
sides looking like drawn sprites.

⚠️ **`mon_gfx`'s differential decode runs along rows and resets per row**, which is the opposite axis
to the one the bitstream was written along. Get it backwards and you get the right Pokémon with
horizontal smears — the kind of wrong that looks right in a thumbnail.

**How the decompressor is trusted.** `the_decompressor_matches_upstreams_own_2bpp` compares all 151
against `make`'s own output, which is the only thing that can *prove* the port. ⚠️ Those files
**cannot be `include_bytes!`'d** — `.dockerignore` excludes `pokered/**/*.2bpp` and the Dockerfile's
build stage copies only `pokered.gbc` and `pokered.sym`, so a compile-time dependency would build
here and fail in the container. They are read from disk, and the test skips loudly when they are
absent. `every_front_pic_matches_its_committed_checksum` is what covers that case; regenerate it with
`dump_front_pic_checksums` (`--features diagnostics --ignored`) **only** when the 2bpp comparison is
green, since that is the whole of what makes the fixture mean anything.

⚠️ **That fixture lives in `src/pokemon/data/gfx/`, not `src/pokemon/data/`.**
`savestate::tests::every_committed_fixture_decodes` walks `data/*.bin` and tries to `load_state`
every one, so a fixture of any other kind dropped in beside the save states fails a test three
modules away with a save-state error message.

**Palettes are the web layer's business, not the decoders'.** Both decoders return 2bpp shade
indices and nothing else; `src/web/badges.rs` and `src/web/sprites.rs` choose the colours.

⚠️ **`badges.rs` inverts its ramp and `sprites.rs` must not**, and the difference is not the page —
both land on the same dark panel. A badge is *line art*: there is no fill, so inverting it turns
black-on-white into white-on-dark and loses nothing. A Pokémon pic is *filled*, so inverting it is
not a palette choice but a different picture — it shipped that way once and Gengar came out
white-bodied with a dark grin. The argument that talked me into it ("a black outline is invisible on
a near-black panel") is simply false: an outline is bounded by the body's own bright fill and reads
at full contrast, and only the outermost contour meets the panel. `the_ramp_is_not_inverted` pins
the direction, because nothing else would notice it flipping.

⚠️ **The background is found by flood-filling shade 0 from the border**, four-way, and it is
load-bearing rather than a refinement: shade 0 is a body's *white fill* as well as the surround, so
calling it transparent outright renders the whole Pokédex as wireframes (all 151 use it as fill),
and not finding it at all renders them as solid white blocks. A diagonal step leaks through any
outline drawn on the diagonal, which is why the fill is four-way.

## The map the model is sent

`read_map` answers with a rendered picture of the whole current map, not an ASCII grid.
`src/pokemon/map_gfx.rs` reads the graphics; `src/llm/map_image.rs` composes and colours them. Five
things were paid for building it.

⚠️ **The picture is drawn on the *worker* thread, and `service_read` must keep handing over data
rather than pixels.** Celadon is 460k pixels and Route 17 is 737k — tens to hundreds of milliseconds
of PNG encode, against an `AGENT_RESOLUTION` of 20 ms. Rendering inside `service_read` would spend
ten agent ticks inside one of them on nearly every overworld turn. What crosses the channel is a
`MetaTileMap` (`ToolAnswer.map`), which the policy already clones once per poll; `rom_gfx` reads a
`&'static` ROM slice so the worker needs no emulator. Same rule, same reason as `screenshot`.

⚠️ **`wSpriteStateData1 + 9` is `$0` down, `$4` up, `$8` left, `$C` right** (`ram/wram.asm:96`), and
that is **not** `PlayerFacingDirection`'s encoding (`Up = 8, Down = 4, Left = 2, Right = 1`, on
`wPlayerDirection`). The two collide on `4` and `8` meaning different things, so reading one with the
other's table points half the people on a map the wrong way and nothing fails.
`sprite_facing_is_the_sprite_bytes_encoding_and_not_the_players` is the guard.

⚠️ **Read the OAM layout out of `SpriteFacingAndAnimationTable`; do not mirror the sprite by hand.**
`.FlippedOAM` swaps the left and right *columns* as well as setting `OAM_XFLIP`, so assembling the
16×16 and flipping it is right only by coincidence. And ⚠️ **an immobile sprite falls back
wholesale**: item balls and boulders are 4-tile sheets, and pokered answers every facing from a
second half of the table that is `.StandingDown, .NormalOAM` — swapping only the tile ids and keeping
the flipped layout draws a right-facing Poké Ball as a *mirrored* one, which is a different picture.

⚠️ **`FontGraphics` is 1bpp**, 0x400 bytes, and `src/pokemon/font.rs`'s `FONT_BYTES` is the
compile-time doubling into 2bpp. Character code `C` → font tile `C - 0x80`, because
`LoadFontTilePatterns` copies the sheet to `vFont` (`$8800`) where the tile index and the character
code are the same number. The reverse charmap reuses `PokemonString::from_string` rather than
transcribing `charmap.asm` a third time, and `the_font_round_trips_through_the_decoder` pins it
against `render_font_string` — which is how a **six-year-old bug** surfaced: glyph 96 was decoded as
`,` when it is `'`. Same mark, different half of the cell (`,` is 116), so every contraction the game
printed came back through the text reader as "Let,s go".

⚠️ **A tileset sheet can run off the end of its bank.** `LoadTilesetTilePatternData` copies a fixed
`$60` tiles whatever the tileset's real size, so several sheets overrun their own label into the
blockset behind them, and `Underground` (`1b:7d60`) overruns the bank itself by 864 bytes. On
hardware nothing references a tile id that high. `map_gfx` clamps the sheet to the bank and answers a
blank tile for an id past the end.

⚠️ **A connection strip has its own tileset and it is often not the bordered map's** — Route 23 is
`Plateau` against Route 22's `Overworld`. `ConnectedMapStrip` carries `tileset` and `tileset_data`
for exactly this; drawing a strip against the map's own sheet produces plausible rubble rather than
an error. `MapMetadata::strip_cells` is shared between the classification and the drawing so the two
cannot place a strip differently — the arithmetic is four sign-sensitive cases and a border row one
tile out of true looks perfectly reasonable.

⚠️ **Labels are drawn last, so they can cover the player.** The red ring is where every coordinate
the model reads is measured from, so `layout_labels` reserves the player's cell before placing
anything. Vermilion is the map that found it. Relatedly, ⚠️ **a connection groups across the whole
edge and a warp only with its neighbours**: every cell of a map edge leads to the same place by
definition, but the strip is broken up by whatever is drawn along it, and Pallet Town's northern
fence line had the picture saying "Route1" four times. Two doors into the same building are *not*
the same door (Mt Moon B1F), so warps may not be merged on destination alone.

⚠️ **`MetaTileMap::reachable_tiles` is "routable to", not "standable on", and reading it the obvious
way produces a picture that still looks like a map.** It is the key set of `bfs_from_player`, which
records *every* neighbour of an open square — walls included — and only declines to expand them,
because a route has to be allowed to end at a door, a counter, a cut tree or a person. Dimming its
complement therefore lit every wall touching open floor and darkened only cells walled in on all four
sides: 18% of Pallet Town, in a pattern with no relation to anything, and it shipped in the first
screenshots. The renderer subtracts obstacles and un-surfable water itself;
`a_wall_is_dimmed_even_though_the_agent_can_route_to_it` is the guard.

⚠️ **Nothing in the renderer may iterate a `HashSet`.** `reachable_tiles`, `warp_targets` and
`connection_targets` are all sets, and a picture whose content depends on hash iteration order reads
to the model as the world having moved, and makes any committed render checksum flake rather than
fail. Every pass walks `meta_tiles` in index order.

⚠️ **`IMAGE_TOKENS = 85` is the `detail: "low"` price and a map is not that.** Measured across all 226
sized maps at 1× with OpenAI's tiling: median 765, mean 1041, max 3825 — the tail is twelve long thin
routes, because a narrow strip is scaled *up* until its short side is 768. `image_tokens` prices each
picture from its own dimensions, and it matters because `Accounting::occupancy` is what decides when
the history compacts: a full context priced at 85 an image never compacts at all.

## Starting a new run in place

`GET /reset-game` and `POST /api/new-run` restart the game without restarting the process. ⚠️ **They
are the only channel from the HTTP layer back into the emulator**, and `src/web/mod.rs`'s module doc
used to say there was none at all — that property was structural, so giving it up was a deliberate edit rather than a
drift. `host::NewRunRequests` is the whole of it: no data travels inwards, only the fact that someone
asked, and it is answered at the **top of `EmulatorHost::tick`**, which is the one point where
nothing is half-done.

⚠️ **A new game is named by the policy with a RAM write, because there is no screen left to type it
on.** `Policy::player_name` → `PokemonApiTrait::write_player_name` → `wPlayerName`, called from
`EmulatorHost::new` when `HostConfig::fresh_game` and from `start_new_run` always. The reason it
cannot go through the game's own name entry is that a run starts from `data::START_OF_GAME`, a save
state captured in Red's bedroom — past the title screen, past Oak's speech and past both name screens
— and the intro is invisible to the agent anyway (`game_mode` answers `None` throughout it, so
`agent.update` returns `Err("Not in game")` and no policy is ever polled). Every run before this was
called `CLAUDE`, which is what a human typed once when the fixture was captured.

⚠️ **Seven characters, not the nickname's ten** (`MAX_PLAYER_NAME`) — `naming_screen.asm` checks
player and rival names against `PLAYER_NAME_LENGTH - 1`. ⚠️ **A resume must never be renamed**: the
name is part of the save and the game has already printed it in a dozen places, so a process
restarted under a different `GB_MODEL` would silently rename a trainer mid-run. Random draws from a
list off its *seed* (not its stream, so a seeded soak run is unchanged), console is `HUMAN`, scripted
declines — a fixture chain that renamed the trainer would differ from every state it was captured
against — and LLM is `GB_MODEL` shortened by `config::player_name_for`. ⚠️ That shortening keeps
**whole segments** and stops at the first that will not fit: truncating the joined string invents
version numbers (`gemma-3-12b` → `GEMMA31`), and *skipping* a segment to fit a later one assembles a
different model (`gpt-4o-2024-08-06` → `GPT4O08`, the month). It is deliberately not asked of the
model: the name is written before the emulator's first instruction, so a completion there would put a
round trip, a timeout and a retry policy in front of every new run.

⚠️ **A run directory has exactly one writer, and five things had a copy of which one it was** — the
checkpointer, the transcript thread's open file, `/api/history`'s path, `/api/healthz`'s run id, and
the LLM worker's notes. They all read `run::CurrentRun` now. The transcript thread in particular
**re-reads the path per event**: a captured `PathBuf` keeps appending the new run's events to the old
run's file, and nothing notices until someone reads either one.

Three more that are easy to get wrong, each with a test:

- **Checkpoint the outgoing run before swapping.** Everything since its last periodic write — up to a
  minute — lives only in memory, and the directory left behind has to be resumable.
- **`VideoEncoder::restart`, not `VideoEncoder::default`.** Deltas are diffed against `last_sent`, so
  a state swap without it leaves fragments of the abandoned run on every viewer's screen. But `seq`
  must survive: `/api/video` drops anything at or below the seq a client opened with, so restarting
  the count at zero makes a live viewer discard the entire new run.
- **Clear `last_status`**, or the send-on-change rule suppresses the one heartbeat that says the run
  changed.

`GB_ADMIN_TOKEN` gates both and they **404 when unset** rather than 403ing — this serves the public
internet, and a challenge would tell a scanner the endpoint is there. Blank counts as unset, because
that is the shape a placeholder Secret takes.

The two differ only in how they ask. `/reset-game` answers an unauthenticated GET with
`WWW-Authenticate: Basic`, so the **browser** collects the password and the SPA holds no token at
all — that is why the "new run" button, its `confirm`, its `prompt` and its `sessionStorage` key are
gone. ⚠️ **Nothing links to it and nothing should**: a GET that resets the game must not be reachable
by a prefetch, a crawler or a middle-click. ⚠️ The username is ignored and the password is
everything after the **first** colon — a generated token may well contain one. And ⚠️ browsers cache
Basic credentials for the session, so a *refresh* of that page starts another run; the page says so,
because a viewer who does not expect it has no other way to find out.

## When the game ends

A win is **`wNumHoFTeams` going up**, and nothing else. pokered increments it at the top of
`AnimateHallOfFame` (`engine/movie/hall_of_fame.asm:27-32`) — the first frame of the ceremony, before
the party parade, the credits, the game's own save and its `jp Init` back to the title screen. It
saturates rather than wrapping and it lives inside the `wMainDataStart..wMainDataEnd` block
`engine/menus/save.asm` round-trips through SRAM, so it survives the credits' soft reset; the ROM's
own main menu reads it to warp a returning Champion home. `PokemonAgent::check_hall_of_fame` watches
the rising edge and emits `AgentEvent::HallOfFame`; `EmulatorHost::file_completed_run` archives the
run and starts the next one.

⚠️ **The two obvious alternatives are both wrong.** `badges.bits() == 255` is Viridian Gym, a good
hour early. `map == Map::HallOfFame` is a three-minute cutscene ending in a soft reset — an edge at
best, a level never — and `scripts/HallOfFame.asm` puts *three* script stages between arriving on
that map and the counter moving (the walk-in, Oak's congratulation, then
`HallOfFameResetEventsAndSaveScript` → `predef HallOfFamePC` → `AnimateHallOfFame`). That last fact
also means **`post-hall-of-fame.bin` has `wNumHoFTeams == 0`**: it is captured on arrival, so it is
the right *seed* for a detection test and a useless thing to assert against as loaded.

⚠️ **The detector reads the MMU, not `game_state()`**, and sits above `update`'s
`game_mode().ok_or(…)?`. That `?` returns on every screen transition and a ceremony is made of them.

⚠️ **The first tick only seeds the baseline** (`Option<u8>`, not `u8`). Seeding from RAM rather than
from zero is what stops a nightly resume — or any postgame fixture — from re-announcing a victory
that happened in another process. `RunMeta::completed` is the second guard, for the case the agent
cannot see: a process restarted from a checkpoint taken a moment *before* the increment replays those
seconds and detects it again.

**The archive** is `$GB_RUN_DIR/hall-of-fame/<stamp>-<run-id>/`, with `ledger.jsonl` beside it.
⚠️ **That one level of nesting is load-bearing.** `run::resumable` lists the *direct* children of
`$GB_RUN_DIR` holding a `state.gbst` and continues the newest; an archive is a complete run directory
written *after* the run it copied, so beside the runs it would be the newest resumable thing on the
volume and the next `gb serve` would resume a game that had already been won and filed.
`hall_of_fame::tests::an_archive_is_written_and_is_not_resumable` is the guard.

⚠️ **The transcript is followed, not copied.** The completion event is *published* in the tick the
archive is triggered from and written by a different thread, so `fs::copy` produces an archive of a
victory with no victory in it — and can catch a torn line between `writeln!` and `flush`.
`publish_event` returns the seq; the follow reads whole lines until it sees it, with a 5 s deadline.
⚠️ **And the whole archive happens before `start_new_run`**, blocking, because `transcript.rs`
re-reads the path *per event*: once a new run is current, an event published before the swap lands in
the new run's file.

⚠️ **A JSONL ledger, not SQLite** — a deliberate choice, not an oversight. Ten rows read and sorted in
memory is not a query workload, and `rusqlite` with `bundled` compiles SQLite's C amalgamation into a
container whose only non-Rust dependency is `ring`'s. Ranking is on the **cartridge's** clock
(`wPlayTime`), which survives every resume with no bookkeeping at all. ⚠️ Stored as *seconds*: the
hours field runs to 255, so `HH:MM:SS` is two digits below 100 hours and three above, and a lexical
sort puts `255:59:59` before `06:12:44`.

⚠️ **A run's figures used to be a process's.** `RunDir::checkpoint` *assigned* `emulated_ms` against a
host that always starts its clock at zero, so a run resumed nightly for a week reported the last night
as the whole playthrough — plausible, and therefore silent. `RunProgress` is now rebased onto the
baseline read at open. Tokens and turns reach the emulator thread through `Published`, which folds
them out of the `Decision` events the worker already publishes: ⚠️ **decisions that landed, not
`max(turn)`** — a turn id is `llm::worker`'s cancellation generation, which counts abandoned turns
and restarts per process.

⚠️ **After the credits pokered clears WRAM**, so `agent.update` answers `Err("Not in game")` on every
tick for ever. The host publishes an agent failure **on change only** for that reason; without it a
finished run puts fifty notices a second into the transcript and every open browser.

## Tests

`src/pokemon/integration_tests/` is tiered by how much **game time** a test emulates, which is what
it costs. The core runs at **~91× realtime** on Pokémon Red and the agent costs **~35%** on top,
giving **~50×** end to end (measured 2026-08-06 on a Ryzen 9 7900X by `bench_core_throughput` and
`bench_emulation_throughput`), so wall clock ≈ emulated-minutes ÷ 48.

Those are post-Phase-C numbers: the core was 29× and the agent-inclusive figure 24× before it, a
3.1× speedup. The agent's share grew from ~16% to ~35% for the obvious reason — it did not get
slower, the emulator under it got faster — so **the agent is now worth profiling and it was not
before.**

```bash
# Default tier: all unit tests + agent mechanics + two navigation smoke tests + web/host/llm.
# ~20s, 1310 tests. The `stalls` tier is most of the growth: eleven cases, three seeds each. The
# one deliberate exception to the tiering is `the_hall_of_fame_is_announced_once_when_the_ceremony_
# starts` (1.7 s of real ROM), which buys the only proof that the end of the game is detected at all.
cargo test --release

# Leg chain: one test per PolicyStep::*_steps() leg, each seeded from a committed snapshot.
# 142 tests in ~55s of wall clock (measured 2026-08-12; it took ~131s before Phase C).
cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests

# The one leg that costs more game time than the whole leg chain combined: the Safari dex sweep,
# 171s of wall clock for ~190 min of emulated game time (21 paid ¥500 trips chasing 4.3%-slot
# species; it was 381s before Phase C). Split out because it, alone, set the leg tier's wall clock
# at six minutes — with libtest printing nothing until it finished, so there was no way to see what
# was still running. ⚠️ `very-slow-tests` does not imply `slow-tests`, and the test's module is
# behind that gate, so pass **both** features or this matches zero tests.
cargo test --release --features slow-tests,very-slow-tests --bin gb -- can_sweep_the_safari_zone

# The whole game from a fresh save, ~5 min of wall clock (was ~11 min before Phase C).
cargo test --release --features full-playthrough full_playthrough

# The stall hunt: 40 min of game time under RandomPolicy from each of 13 starting states, in
# parallel, ~60 s of wall clock. Fails if the agent ever goes longer without reaching a decision
# point than the watchdog allows. Seeded — vary GB_SOAK_SEED to hunt for new jams (GB_SOAK_MINUTES
# to go deeper from one state); seed 1 is the one that must stay green.
cargo test --release --features soak-tests --bin gb -- soak --nocapture

# A single test with output (file module included in the path).
cargo test --release --bin gb -- pokemon::integration_tests::mechanics::test_debouncing --exact --nocapture

# The PPU comparisons: dmg-acid2, cgb-acid2, Pokémon Red in colour.
cargo test --release --bin gb -- game_boy::tests::ppu

# The map pictures `read_map` sends the model, as real PNGs in target/map-renders/, with each one's
# size and estimated token cost. ⚠️ The only way to know the render is *right* rather than merely
# non-blank — look at these before touching the palette, the labels or the tile lookup.
cargo test --release --features diagnostics --bin gb -- \
  llm::map_image::tests::probe_map_images --exact --ignored --nocapture

# Every decision kind's first request, whole, in target/turn-requests/: the `.json` is the literal
# ChatRequest body, the `.md` is the same with the newlines put back (a prompt read through JSON's
# `\n` escaping is not reviewable). ⚠️ The only way to see what the model is actually sent — reading
# it is what found `BattleAction`'s `{:?}` switch rows, ~500 bytes of Rust syntax per party member
# in the menu of every battle turn.
cargo test --release --features diagnostics --bin gb -- \
  llm::prompt::tests::probe_turn_requests --exact --ignored --nocapture

# The diagnostics and probes.
cargo test --release --features diagnostics,slow-tests --bin gb -- probe_ --ignored --nocapture

# Agent throughput (emulator + agent.step). `--exact` needs the full module path, or this
# matches zero tests.
cargo test --release --bin gb -- \
  pokemon::integration_tests::fixture::bench_emulation_throughput --exact --ignored --nocapture

# Emulator core alone — no agent, no policy, no observation. Three workloads. Behind the `bench`
# feature, so benchmarks never pad the ignored-test count.
cargo test --release --features bench --bin gb -- \
  game_boy::tests::bench_core_throughput --exact --nocapture

# What /api/video costs, and every alternative it was chosen over. ~25 s.
cargo test --release --features bench --bin gb -- video::bench --nocapture
```

**A test that is `#[ignore]`d should be blocked, not merely slow or not-a-test.** Everything else
goes behind a Cargo feature, so the ignored list stays a readable backlog:

| Feature | Holds |
|---|---|
| `slow-tests` / `very-slow-tests` / `full-playthrough` | Tiering by emulated game time |
| `diagnostics` | `probe_*`, `dump_fixture_states`, `capture_golden_input` — tools that print a report rather than assert. They keep `#[ignore]` on top of the gate because their pass/fail is not a signal: two legitimately end by exhausting their cycle budget *after* printing what was asked for |
| `bench` | `bench_core_throughput`, `bench_emulation_throughput`, `web::video::bench` (which also pulls in `flate2`) |
| `soak-tests` | `integration_tests::soak` — the fuzzer. Gated as a **module**, not with `#[ignore]`, so it never appears in the ignored list |

The mooneye MBC suite used to be a tier of its own (`hwtests`). It is not: three tests, 0.7 s, so it
runs by default. Its ROMs are `cfg(test)` rather than feature-gated, which keeps them out of the
shipped binary — the only thing the feature was really buying.

With every tier feature on and the tool features off, the ignored list is **18 blocked emulator
tests** and nothing else — 9 `oam_bug` and 9 `mem_timing`/`halt_bug`. (It was 19 until the combined
`dmg_sound` suite ROM was fixed.) Each names its blocker: a plan task ID, or why it will not be
fixed. Keep it that way.

Failure artifacts — a save state and a screenshot at the point of a stall or timeout — land in
`target/test-artifacts/`, not the repo root.

### Turns the game takes back: `integration_tests::interruption`

A turn abandoned mid-flight is paid for and thrown away, so before committing a run to a paid model
the rate matters. `LlmPolicy` keys a turn by the decision kind it answers and cancels it when the
agent asks something else — the shape everyone expects is an overworld turn interrupted by a battle.

⚠️ **Measured, it does not happen.** The deployed run's 2428 turns contain **one** `turn_cancelled`
for "the game moved on", and it is turn 1, the `POST /api/new-run` reset bumping the generation. The
structural reason is that **the agent presses nothing while a turn is in flight**:
`AwaitingOverworldAction` and `BattleState::AwaitingPolicy` tick a delay and poll, so the game sits
at a static menu or a stationary tile and cannot move on its own. Across the same run's 2430
`turn_started → decision` windows exactly **one** agent event fired inside a window. A wild encounter
or a trainer's line of sight fires *during a walk*, and no policy poll happens during a walk — so the
battle is the next turn rather than an interruption of the one in flight.

⚠️ **The 68 other `turn_cancelled` events in that run are a different thing wearing the same name**:
`Worker::give_up` publishes one when the model replies twice with no tool call, and those turns still
return a forced `wait`. Counting `turn_cancelled` without splitting on `reason` reports the rate as
2.8% when it is 0.04%.

What is kept is the guard, not the search. `SlowPolicy` wraps any ordinary policy and holds each
answer back for a number of agent ticks, which is the one property of an LLM turn that matters here.
⚠️ **It has to key turns exactly the way `LlmPolicy` does**, `pick_field_move`'s exemption included,
or it reports a cancellation every time the agent asks anything at all.

- `leaving_oaks_lab_does_not_strand_a_turn_in_the_rivals_script` (default tier, ~2 s) is the case
  worth guarding: the rival's challenge is the longest scripted freeze the early game has. The trace
  shows the walk to the door aborting into `RunningScript` and the agent asking **nothing** for the
  ~900 ticks (18 s of game time) between the abort and the battle menu. Three latencies — instant,
  5 s, 60 s — because the script fires on a *tile* and the answer lands on a *clock*, so each puts
  the walk in a different place relative to it. ⚠️ **The precondition is half the test**: a run that
  stops before the battle question is ever put proves nothing, which is how the first version passed
  at every latency below 600 ticks.
- `the_detector_notices_a_question_being_replaced` exists because both results are negative ones.
  Nothing in the game replaces a question mid-flight, so it is done by hand.

⚠️ **A broad sweep was written and deliberately not kept**: random play from six fixtures, 10 minutes
of game time each with a jittered latency, 346 turns and 0 abandoned. It cost 100 s of the
`soak-tests` tier to re-derive a number the structure already explains, over the same fixtures under
the same `RandomPolicy` that `soak` already walks. If the rate is ever in doubt again, it is twenty
lines around `SlowPolicy` rather than something to keep running.

### Finding jams: `soak`, and `stalls` beside it

⚠️ **`full_playthrough` proves one route still works; it cannot find a jam off that route.** The
scripted policy never chooses to walk into a PC, or into grass with nothing in it, or to pick a move
the game will refuse — so none of those were reachable by any test in the suite, and all of them
wedged the deployed run instead. `soak` is the answer: hours of `RandomPolicy`, which explores the
agent's state machine far more widely than any route.

It watches `PokemonAgent::since_last_policy_poll` — the **same** value W9's watchdog reads — so it
fails exactly when a deployed `LlmPolicy` would have its watchdog fire. One definition of stuck.

⚠️ **Breadth beats depth here, and the reason is what a random walker actually does.** It does not
explore, it *diffuses*: it picks uniformly from the tiles the current map offers, so hours from one
starting point re-cross the same few maps and the second hour visits what the first did. So the
budget buys **starting points** rather than depth — one test per entry in `STATES`, 40 minutes each,
over thirteen committed fixtures, run in parallel for less wall clock than the single five-hour test
it replaced. ⚠️ **A state earns its place by what it makes *reachable***, not by progression: a
bicycle, a Safari step counter, a boulder, a PC with something in it, a bag with a TM in it. A fresh
save's bag is empty, which is why no amount of play from it can reach an item the game refuses in
battle — and why that jam survived five hours a day of fuzzing. More badges than its neighbour buys
nothing.

⚠️ **Those options are not book-keeping — whole screens hang off them.** Battle style SHIFT asks
"<TRAINER> is about to use <MON>! Will <PLAYER> change POKéMON?" on every enemy switch, and since
every `TestFixture` overwrites the options with SET, **no other test in the suite ever sees that
prompt**. The agent answers *no* (switching is a decision, and the policy makes it at the menu that
follows); A there opens the party menu, which the party arm backs out of, which brings the prompt
round again.

⚠️ **It forces the cartridge's own options** — `InitOptions`' `TEXT_DELAY_MEDIUM`, battle animations
*on*, battle style SHIFT — rather than `TestFixture`'s `FAST_FIXTURE_OPTIONS`. `gb serve` runs on
those and the soak exists to reproduce the deployment, not to be cheap: the no-PP jam was a race with
the character-by-character text renderer, and fast text may well not reproduce it. It has to *write*
them, not merely leave them alone, because every fixture past `start-of-game-state.bin` was captured
mid-leg by `TestFixture` and carries fast text baked into `wOptions`.

⚠️ **`GB_SOAK_LIMIT_SECS` is how you find the *next* one.** The default is the watchdog's 300 s
because that is the number production cares about, but seed 1's worst healthy stretch across all
thirteen states is **62 s** (2026-08-12) — so a near-miss can hide comfortably under the default for a
long time. Running at `GB_SOAK_LIMIT_SECS=120`–150 trips on anything twice as quiet as normal, and
that is how the pacing budget was found: 182 s of silence in Viridian Forest that turned out to be
`PACING_BUDGET_TICKS` running to the end on the rarest grass in the game (8/256), not a jam at all.
Note that 62 s is now *by construction* — `MAX_MOVEMENT_SILENCE` gives up on a walk at 60 — so the
healthy distribution bunches just under it. ⚠️ **But its tail is much longer than that, and a limit
below ~150 s finds the tail rather than a bug**: a paralysed Pokémon in a wrap chain gets no menu for
several turns, because Gen 1 skips the player's input while WRAP/BIND runs, and on Route 15's line of
trainers that measures **124 s** of perfectly legitimate silence (seed 837).
⚠️ **A budget that bounds silence is not sized to guarantee success** — giving up just means the
policy gets asked again, and the first version of that constant was three times too generous because
it was sized to guarantee an encounter.

⚠️ **It is seeded (`GB_SOAK_SEED`, default 1) and must stay that way.** The first runs each failed
somewhere different, which is worse than useless: a failure that vanishes when you go back to look at
it cannot verify its own fix, and CI would flake. Seed 1 is the one that must stay green; vary the
seed to hunt.

**Every jam it finds gets promoted to `integration_tests::stalls`**, in the *default* tier: the save
state at the moment the agent went quiet, replayed against a fresh agent, about two seconds each.
`stalls::probe_stall_artifacts` (`--features diagnostics`, `GB_STALL_DIR=…`) is the bulk form — it
replays a whole directory of artifacts and prints which still reproduce, which is what a sweep across
seeds leaves you holding, and what tells you a fix covered four cases out of five. That
is what makes the fix loop tolerable — the difference between a 4½-minute reproduction and a
one-second one. ⚠️ **Not every stall survives the trip**, because the save state holds the emulator
and not the agent: a jam the game's own screen re-creates reproduces perfectly, a jam that lived in
the agent's own state (an `OverworldMovement` route) does not. Watch a new case go red before
committing it, or it may be asserting nothing.

⚠️ **Its artifacts are named per state *and* per seed** (`soak-<state>-seed<N>.{bin,png}`), because a
hunt that sweeps every state under every seed would otherwise have each failure overwrite the last —
and the artifact is the whole value of a failure, since it is what gets promoted into `stalls`.

**Nearly everything it finds is one shape: a closed loop under A.** A menu or a script the agent's
own A press re-enters, with the cursor untouched — the PC menus, a spent move, a key item in battle,
the Cerulean badge house, Bill's PC, a refused field move, a Card Key door, the Safari menu's sticky
cursor, the START menu left open on the trainer card. Four rules cover the class, and they are worth
knowing before adding another special case:

- ⚠️ **A give-up in a battle hands back *latched into B*** (`BattleState::backing_out`), because a
  plain `WaitingForMenu` opens by pressing A — into whatever menu is still on screen, which is how
  "give up" came to mean "select whatever is under the cursor".
- ⚠️ **The text reader escapes menus, not conversations.** After 30 s in which the agent reaches *no
  decision point*, and only when what is on screen is a list menu, a field-move box, a menu offering
  CANCEL, or the **START menu**, it presses B until a poll happens (which is what clears it — `poll_policy`, on the
  agent, so a flicker through `Idle` cannot reset it). ⚠️ **Not on a yes/no**, where B is an answer,
  and ⚠️ **not in a battle**, where B cancels the move being chosen and gym leaders are routinely
  quieter than 30 s. Without those two conditions it fires mid-fight and `full_playthrough` loses the
  Brock fight.
- ⚠️ **Silence bounds the drivers, not tick budgets.** A driver that runs its own menus is abandoned
  after `DRIVER_ESCAPE_SILENCE`, and a walk after `MAX_MOVEMENT_SILENCE`, rather than each of the
  nineteen carrying a counter of its own. ⚠️ It has to be *silence*: a tick counter belongs to a
  state, and a state torn down by an interruption starts it over — the Seafoam current takes the
  player every few seconds and handed the walk a fresh budget each time.
- ⚠️ **A menu the agent did not open is closed, not confirmed** — `MENU_HANDOVER_TICKS`, armed in
  `assert_text_box_state`. That function is the funnel for "start reading a text box" and everything
  that drives menus on purpose is excluded from it by `drives_its_own_menus`, so arriving there with
  a *menu* on screen means something **left one behind**: a driver abandoned by
  `DRIVER_ESCAPE_SILENCE`, an aborted PC, a `press_buttons` batch. It reuses `escaping_menus`, so it
  is the 30 s rule above acting immediately on evidence it can already trust. ⚠️ **It cannot be the
  single transition tick**, which is the version that was written first and detects nothing:
  `wFontLoaded` flips a third of a second *before* the menu draws itself — measured on the START
  menu, geometry is the previous menu's until tick 18 and `EXIT` does not reach the tile map until
  tick 21, against the reader's first A on tick 26. ⚠️ And it must stay a **short window** rather
  than becoming a per-tick test: bounded, it only has to be right about the moment a box opens with
  no driver behind it; unbounded, it has to be right about every screen of every conversation, which
  is the Mt Moon failure one paragraph down.
- ⚠️ **A rule that runs at every text box may believe only the *screen*, never a lingering id —
  `MenuEvidence`.** `wTextBoxID` is written when a box is drawn and never cleared, so it goes on
  naming a menu that closed several maps ago; the 30 s rule can trust it because 30 s of silence has
  itself ruled out a conversation, and the hand-over rule cannot, because it fires on conversations
  by definition. Getting that the same way round for both cost a nickname: the Silph Co lift left
  `ListMenuBox` behind, the agent talked to the rescued worker a few maps later, and B — an exit on a
  list, an **answer** on a yes/no — declined "Do you want to give a nickname to LAPRAS?".
  `a_full_party_sends_the_silph_lapras_to_the_box` is the only test in the suite that crosses a lift
  into a yes/no, and it is now the guard for both things.

⚠️ **Each of those rules is a frame-timing change, so `full_playthrough` is the only thing that can
price one.** The ⚠️s in `agent.rs` name four wider versions that look obviously right and are not:
latching the item driver's tick budget cancels a ball mid-throw; escaping *any* text box after 30 s,
or on a count of reopened boxes, blacks the mainline out in Mt Moon; handing the turn to the policy
from every battle-menu position re-times every battle in the game. Same lesson as
`with_original_battle_timing` — the leg chain and `stalls` cannot see any of it.

The rest of what it catches is a driver waiting for something that stopped coming, pressing buttons
in silence. Traps in fixing those:

- ⚠️ **A message box swallows directional input**, so a driver that is right about the next button
  still has to clear what is on top of it first. The forced-switch arm correctly wanted to walk the
  cursor off a fainted Pokémon and pressed Up into "There's no will to fight!", for ever.
  ⚠️ **And it is not always a message you can name**: `battle_menu_state` reads `wTopMenuItemX/Y`,
  which *linger*, so an ordinary battle line ("It's super effective!") over the party list reports as
  the party list. What tells them apart is the screen — a party list draws an HP bar per member, a
  message box draws the active mon's alone, so `>= 2` slashes means the list is really there. That is
  the same heuristic the item driver uses for "use on which POKéMON?".
- ⚠️ **A give-up that is not remembered is not a give-up.** `handle_card_key_door` spends 40 A presses
  on a door, declares it a wall and blocks it — then started another forty on the next tick because
  nothing read `blocked_tiles` back. Every press reprints "Darn! It needs a CARD KEY!", which is a
  text box, which is another A.
- ⚠️ **A counter outside the variant is reset by `set_state`.** `UsingItem` and `WaitingForMenu`
  rebuild themselves every tick with a `press`/toggle field flipped, so `set_state` sees a *new*
  state and zeroes anything counting from `PokemonAgent`. The first bound on each silently never
  fired. `OverworldMovement` is the one state where the agent-level `state_ticks` works, because it
  does not rebuild itself.
- ⚠️ **The branch that detects a problem is not always the branch that presses the wrong button.**
  `WaitingForMenu`'s `MoveList` arm had handled a spent move with B since an earlier hours-long
  wedge, and it still wedged — because the `screen.contains` check above it returns first while the
  message is up, and the *text reader* (in the `None` arm) was the thing mashing A. A fix has to sit
  above every branch that can press.

### ⚠️ Why `full_playthrough` is not optional

The leg tests each start from a committed fixture, so they prove the legs *individually*; only
`full_playthrough` proves they still **compose**, and the two come apart in ways nothing else
catches:

- **A leg test can be green for a reason the mainline does not give it.** `run_leg` keeps stepping
  after the queue empties until the effect lands, so a leg whose `Interact` pops before its
  conversation still passes — while `complete_game_steps` walks straight on without the item. That is
  exactly how the Poké Flute broke. `run_leg` now prints a ⚠️ when its post-exhaustion wait is long;
  **treat that warning as a failure in waiting.**
- **A fixture pins a party and a bag; the mainline earns them.** A leg seeded with 20 Hyper Potions
  says nothing about whether the run that reaches it can afford them.
- **Anything that changes frame timing re-rolls the RNG stream** (see `with_original_battle_timing`),
  and only a full run crosses every route that stream feeds.

Because it is opt-in and slow, it rotted once already: it sat broken while its own doc comment, this
file and the plan all claimed it played to all 8 badges. When it fails it now reports how far it got
(`completed 488/516 policy steps (94%)`) and drops its artifacts;
`playthrough::probe_resume_playthrough` replays from there in seconds instead of re-running the 20
minutes up to the stall. **If you cannot make it pass, say so explicitly in the hand-off — do not
leave a doc comment claiming it works.**

### ⚠️ Fixtures are committed inputs

Each leg test snapshots its end state for the next leg, but the write is a no-op unless
`--features regen-fixtures` is on — otherwise every run silently changes the next run's inputs, and a
leg "fails" only because an earlier one re-saved its fixture. To regenerate after a deliberate
change, run the affected legs **in chain order**:

```bash
cargo test --release --features slow-tests,regen-fixtures --bin gb -- can_clear_ss_anne --exact
```

### Benchmarking

⚠️ **Do not trust a single benchmark reading on this machine.** It has fast and slow states ~15%
apart — the same unmodified binary has measured `cpu_instrs` at 43.5× and 53.2× twenty minutes
apart. Compare only adjacent paired runs of the two builds, **alternate which one runs first**, and
report both orders.

**`perf` works and needs no `sudo`** (`perf_event_paranoid` is 2). Build with
`RUSTFLAGS="-C debuginfo=2"` into a scratch `CARGO_TARGET_DIR`, then drive the benchmark with
`BENCH_FRAMES=40000 BENCH_ONLY=pokemon` so there is enough wall clock to sample and only one workload
in the profile. ⚠️ Watch for sampling skid: a hot instruction is often paying for the *load* feeding
it, not for itself — that one cost an hour.

### Test ROMs and the resampler

`src/roms/` needs no pokered submodule. `cgb-acid2` **ships its own reference image**, so nothing in
it was promoted from `gb`'s own output; its README pins the 5-bit to 8-bit colour expansion as
`(c << 3) | (c >> 2)` — the plain widening, **not** a colour-correction curve — which is what
`LcdColor::from_rgb555` implements. ⚠️ Adopting gambatte's `gbcToRgb32` correction instead would
break the comparison.

`src/audio/blip/tests.rs` checks the resampler two independent ways, and they fail differently.
**Golden vectors** are bit-exact comparisons against the original C++ (Blip_Buffer ships no test
suite of its own, only interactive SDL demos); the fixtures in `src/audio/data/blip_*.bin` come from
linking the vendored library in `tools/blip-golden/`. Regenerate only after a *deliberate* change to
the algorithm or its parameters:

```bash
# 1. only if the realistic-signal input needs refreshing (writes src/audio/data/apu_capture_in.bin)
cargo test --release --bin gb -- audio::reference::tests::capture_golden_input --exact --ignored
# 2. always — reads apu_capture_in.bin, writes the other src/audio/data/blip_*.bin
tools/blip-golden/build.sh
```

⚠️ The goldens are pinned to `GOLDEN_TREBLE_DB` in the test module, deliberately *not* to
`blip::DEFAULT_TREBLE_DB` — tone is a taste knob, so changing what the emulator ships must not
invalidate the port's correctness fixtures. **Invariants** are the real regression net and need no
C++ toolchain: every phase's taps summing to `kernel_unit`, a step depositing exactly its own
amplitude of DC, zero sample-count drift over ten emulated minutes, no aliasing on a 15 kHz square,
and surviving a minute of emulation with no audio consumer at all (which is what the headless
integration tests do).

There is deliberately **no WAV "ear check"** any more — it was a listening aid rather than an
assertion, and was removed with `src/audio/wav.rs` rather than left in the ignored list looking like
a test.

**Fast-forward.** The number keys `1`–`5` in the SDL UI scale emulation speed, and `render.rs`
mirrors that into `Audio::set_emulation_speed` so the resampler scales its *source clock* to match.
Without it a sped-up emulator simply produces audio faster than the device drains it and the queue
backs up. The speed is derived from `cycle_duration`, not from the key pressed, so it tracks what the
emulator actually targets — `REALTIME_CYCLE_DURATION / 5` truncates to 190 ns, which is 5.016×.

## Shipping it

⚠️ **The cartridge is stage 1 of the Dockerfile, not an input**, and **the sha1 check that ends that
stage is load-bearing**: every committed fixture and every generated symbol is pinned to those exact
bytes, so a ROM that merely assembles is a different game and would fail somewhere deep in the agent
instead of at the build. `roms.sha1` is upstream's own manifest.

⚠️ **`.dockerignore` must exclude the host's pokered artifacts with `**`.** `pokered/*.o` leaves
`pokered/gfx/pics_red.o` in the context, and a stale object file from a *newer* rgbds stops the build
dead (`Unsupported object file … expected revision 12, got 13`). None of what it excludes is tracked;
every one is a `make` output.

⚠️ **`CMD` is exec form so `gb` is PID 1 and receives SIGTERM itself** — that signal is what
checkpoints the run. A shell in between means `docker stop` loses everything since the last periodic
checkpoint.

⚠️ **The build stamp (`/version`) is `ENV` in the runtime stage and must stay below the `COPY` of
the binary.** `GB_BUILD_DATE` changes on every build, so an `ARG` the cargo stage read would
invalidate stage 3 every single CI run — and `type=gha` caches *layers*, not the cache mounts the
cargo registry and target directory live on, so that is a full cold `cargo build --release` each
time rather than a cheap re-link. Below the binary's own layer there is nothing but metadata, so the
same three facts cost nothing. That is also why they are `std::env::var` in `src/web/version.rs`
rather than `env!()`, and why a `build.rs` git fallback was not added: it would either recompile the
crate on every commit or go quietly stale between them, and `null` is the honest answer for a build
nobody stamped.

**CI** (`.github/workflows/container.yml`) builds the image, smoke-tests the running container, and
only then pushes it to ghcr.io, tagged `latest` and the commit. ⚠️ The push steps are main-only: a
fork PR's `GITHUB_TOKEN` is read-only whatever the workflow's `permissions:` asks for.

⚠️ **In `k8s/`, everything unusual is the same fact — a run directory has exactly one writer.** One
replica, `strategy: Recreate`, a PVC rather than an `emptyDir`, and a 30 s grace period so the
SIGTERM checkpoint lands. There is also deliberately no CPU limit: the emulator thread is not
event-driven, and a CFS quota shows up as the game running below real time rather than as anything
that looks like a resource problem. The liveness probe proves the HTTP server only — `healthz` is
axum's and knows nothing about the emulator thread; the wedged-run case is the in-process watchdog.

---
name: run-lifecycle
description: "Starting a run in place and ending one: the single channel from HTTP back into the emulator, naming a new game by RAM write, the one-writer rule for a run directory, and detecting the Hall of Fame, archiving it and the ledger. Load before touching src/run/, host.rs's new-run or completion seams, /reset-game or POST /api/new-run."
---

# Starting a new run in place, and when the game ends

*Extracted from `CLAUDE.md`, which holds the rules of the road and the index of these skills. The
README is imported into `CLAUDE.md` and is not repeated there or here: this file has only the
invariants and the traps, nearly every one of which was learned by breaking something.*

## Starting a new run in place

⚠️ **`GET /reset-game` and `POST /api/new-run` are the only channel from the HTTP layer back into the
emulator**, and `src/web/mod.rs`'s module doc used to say there was none at all — that property was structural,
so giving it up was a deliberate edit rather than drift. `host::NewRunRequests` is the whole of it: no data
travels inwards, only the fact that someone asked, and it is answered at the **top of `EmulatorHost::tick`**,
the one point where nothing is half-done.

⚠️ **A new game is named by the policy with a RAM write, because there is no screen left to type it on.**
`Policy::player_name` → `PokemonApiTrait::write_player_name` → `wPlayerName`, called from `EmulatorHost::new`
when `HostConfig::fresh_game` and from `start_new_run` always. A run starts from `data::START_OF_GAME`, a save
state captured in Red's bedroom — past the title screen, Oak's speech and both name screens — and the intro is
invisible to the agent anyway (`game_mode` answers `None` throughout it, so `agent.update` returns
`Err("Not in game")` and no policy is ever polled).

⚠️ **Seven characters, not the nickname's ten** (`MAX_PLAYER_NAME`) — `naming_screen.asm` checks player and
rival names against `PLAYER_NAME_LENGTH - 1`. ⚠️ **A resume must never be renamed**: the name is part of the
save and the game has already printed it in a dozen places, so a process restarted under a different `GB_MODEL`
would silently rename a trainer mid-run. Random draws from a list off its *seed* (not its stream, so a seeded
soak run is unchanged), console is `HUMAN`, scripted declines — a fixture chain that renamed the trainer would
differ from every state it was captured against — and **LLM is the constant `AI`** (`llm_policy::PLAYER_NAME`).
The README says why `config::player_name_for` and its `GB_MODEL` shortening are gone.

⚠️ **The one property that outlived it is the check, now in
`policy::every_name_a_policy_can_choose_is_one_the_game_will_take`**: every name any policy can hand the game is
asserted non-empty, within `MAX_PLAYER_NAME`, not `NINTEN` (which is `DebugNewGamePlayerName`, what `game_mode`
compares `wPlayerName` against to decide the intro is still up), and encodable without a `0x00` — ⚠️ **tested
by running it through `PokemonString::from_string`, not by asking whether it is alphanumeric**, since `/` is a
perfectly writable `$F3`. The old test only ever saw the derived names; `RANDOM_NAMES` and `HUMAN` had no guard
at all.

⚠️ **A name is deliberately not asked of the model**: it is written before the emulator's first instruction, so
a completion there would put a round trip, a timeout and a retry policy in front of every new run.

⚠️ **`llm::history` is the one writer that deliberately does *not* follow `CurrentRun`.** Everything
below re-reads the current run per write; the conversation captures its directory instead, because a
turn in flight when the swap lands belongs to the old game and its messages must be filed with it.
See the `llm-turn-loop` skill.

⚠️ **A run directory has exactly one writer, and five things had a copy of which one it was** — the
checkpointer, the transcript thread's open file, `/api/history`'s path, `/api/healthz`'s run id, and the LLM
worker's notes. They all read `run::CurrentRun` now. ⚠️ The transcript thread in particular **re-reads the path
per event**: a captured `PathBuf` keeps appending the new run's events to the old run's file.

Three more that are easy to get wrong, each with a test:

- **Checkpoint the outgoing run before swapping.** Everything since its last periodic write — up to a minute —
  lives only in memory, and the directory left behind has to be resumable.
- **`VideoEncoder::restart`, not `VideoEncoder::default`.** Deltas are diffed against `last_sent`, so a state
  swap without it leaves fragments of the abandoned run on every viewer's screen. But `seq` must survive:
  `/api/video` drops anything at or below the seq a client opened with, so restarting the count at zero makes a
  live viewer discard the entire new run.
- **Clear `last_status`**, or send-on-change suppresses the one heartbeat that says the run changed.

`GB_ADMIN_TOKEN` gates both and they **404 when unset** rather than 403ing — this serves the public internet,
and a challenge would tell a scanner the endpoint is there. Blank counts as unset, because that is the shape a
placeholder Secret takes. The two differ only in how they ask: `/reset-game` answers an unauthenticated GET
with `WWW-Authenticate: Basic`, so the **browser** collects the password and the SPA holds no token at all —
which is why the "new run" button, its `confirm`, its `prompt` and its `sessionStorage` key are gone.
⚠️ **Nothing links to it and nothing should**: a GET that resets the game must not be reachable by a prefetch, a
crawler or a middle-click. ⚠️ The username is ignored and the password is everything after the **first** colon —
a generated token may well contain one. ⚠️ And browsers cache Basic credentials for the session, so a *refresh*
of that page starts another run; the page says so, because a viewer who does not expect it has no other way to
find out.

## When the game ends

A win is **`wNumHoFTeams` going up**, and nothing else. pokered increments it at the top of `AnimateHallOfFame`
(`engine/movie/hall_of_fame.asm:27-32`) — the first frame of the ceremony, before the party parade, the credits,
the game's own save and its `jp Init` back to the title screen. It saturates rather than wrapping and lives
inside the `wMainDataStart..wMainDataEnd` block `engine/menus/save.asm` round-trips through SRAM, so it survives
the credits' soft reset. `PokemonAgent::check_hall_of_fame` watches the rising edge and emits
`AgentEvent::HallOfFame`; `EmulatorHost::file_completed_run` archives the run and starts the next one.

⚠️ **The two obvious alternatives are both wrong.** `badges.bits() == 255` is Viridian Gym, a good hour early.
`map == Map::HallOfFame` is a three-minute cutscene ending in a soft reset — an edge at best, a level never —
and `scripts/HallOfFame.asm` puts *three* script stages between arriving on that map and the counter moving.
That last fact also means **`post-hall-of-fame.bin` has `wNumHoFTeams == 0`**: it is captured on arrival, so it
is the right *seed* for a detection test and a useless thing to assert against as loaded.

⚠️ **The detector reads the MMU, not `game_state()`**, and sits above `update`'s `game_mode().ok_or(…)?`. That
`?` returns on every screen transition and a ceremony is made of them. ⚠️ **The first tick only seeds the
baseline** (`Option<u8>`, not `u8`) — seeding from RAM rather than from zero is what stops a nightly resume, or
any postgame fixture, from re-announcing a victory that happened in another process. `RunMeta::completed` is the
second guard, for the case the agent cannot see: a process restarted from a checkpoint taken a moment *before*
the increment replays those seconds and detects it again.

**The archive** is `$GB_RUN_DIR/hall-of-fame/<stamp>-<run-id>/`, with `ledger.jsonl` beside it. ⚠️ **That one
level of nesting is load-bearing.** `run::resumable` lists the *direct* children of `$GB_RUN_DIR` holding a
`state.gbst` and continues the newest; an archive is a complete run directory written *after* the run it copied,
so beside the runs it would be the newest resumable thing on the volume and the next `gb serve` would resume a
game that had already been won and filed. `hall_of_fame::tests::an_archive_carries_the_whole_run_including_the_conversation_and_is_not_resumable`.

⚠️ **Every artifact the archive carries is named by hand, so a new one is dropped silently.** `archive` copies
`memories/`, `todo.json`, `battle-script.json` and both halves of the conversation by name — there is no "copy
everything" — and the assertion list in
`an_archive_carries_the_whole_run_including_the_conversation_and_is_not_resumable` is the only thing that would
notice one missing. Restore is the same shape and for the same reason: nothing central reloads a run directory,
so `TodoList::open`, `BattleScript::open` and `History::open` each read their own file, each tolerate it being
absent or corrupt, and each has to be reopened in `Worker::apply_restart` or a `POST /api/new-run` leaves the old
game's state in the worker.

⚠️ **The transcript is followed, not copied.** The completion event is *published* in the tick the archive is
triggered from and written by a different thread, so `fs::copy` produces an archive of a victory with no victory
in it — and can catch a torn line between `writeln!` and `flush`. `publish_event` returns the seq; the follow
reads whole lines until it sees it, with a 5 s deadline. ⚠️ **And the whole archive happens before
`start_new_run`**, blocking, because `transcript.rs` re-reads the path per event.

⚠️ **A JSONL ledger, not SQLite** — a deliberate choice. Ten rows read and sorted in memory is not a query
workload, and `rusqlite` with `bundled` compiles SQLite's C amalgamation into a container whose only non-Rust
dependency is `ring`'s. Ranking is on the **cartridge's** clock (`wPlayTime`), which survives every resume with
no bookkeeping. ⚠️ Stored as *seconds*: the hours field runs to 255, so `HH:MM:SS` is two digits below 100 hours
and three above, and a lexical sort puts `255:59:59` before `06:12:44`.

⚠️ **A run's figures used to be a process's.** `RunDir::checkpoint` *assigned* `emulated_ms` against a host that
always starts its clock at zero, so a run resumed nightly for a week reported the last night as the whole
playthrough — plausible, and therefore silent. `RunProgress` is rebased onto the baseline read at open. Tokens
and turns reach the emulator thread through `Published`, which folds them out of the `Decision` events the worker
already publishes: ⚠️ **decisions that landed, not `max(turn)`** — a turn id is `llm::worker`'s cancellation
generation, which counts abandoned turns and restarts per process.

⚠️ **The same bug's third half was on the wire, and it outlived the fix above by months.** `StatusSnapshot`'s
`wall_ms`/`emulated_ms` are still elapsed times *since this process started* — that is what the speed derivation
needs, since only a counter that returns to zero on a new run lets the browser spot the reset — so the panel's
"played" line reported the serving process's share as the run's total, and every rollout sent it back to `00:00`
beside a cartridge clock that carried on. `StatusSnapshot::run_emulated_ms` is `RunDir::baseline()` plus what
this process has emulated, which is exactly what the next checkpoint writes, so the page and `meta.json` cannot
disagree. ⚠️ **The baseline is asked of `CurrentRun` on every heartbeat rather than mirrored into a host field**:
`start_new_run` and `file_completed_run` both swap the directory, and a field is two more places to remember.
⚠️ And it is read **once at open** — `RunDir::baseline` is a copy taken before the first checkpoint overwrites
`meta.json`, so re-reading it after one would already hold this process's contribution and count it twice.

⚠️ **After the credits pokered clears WRAM**, so `agent.update` answers `Err("Not in game")` on every tick for
ever. The host publishes an agent failure **on change only** for that reason; without it a finished run puts
fifty notices a second into the transcript and every open browser.


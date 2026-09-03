# Run lifecycle: a new run in place, a cleared conversation, and the end of the game

Read before touching `src/run/`, `host.rs`'s new-run, clear and completion seams, `/reset-game`,
`POST /api/new-run` or `POST /api/clear`.

## Starting a new run

- `GET /reset-game`, `POST /api/new-run` and `POST /api/clear` are the only channel from HTTP back
  into the emulator. `host::ControlRequests` carries one `host::ControlRequest` — a closed enum of
  two, with no payload — and it is answered at the top of `EmulatorHost::tick`, where nothing is
  half-done. One mailbox for both commands, so either outstanding refuses the other; the refusal
  names which.
- All three are off unless `GB_ADMIN_TOKEN` is set and 404 (not 403) when it is not, because the server
  is public and should not advertise a reset endpoint. Blank counts as unset. When set,
  `/reset-game` answers an unauthenticated GET with a Basic challenge so the browser collects the
  password and the SPA holds no token; the username is ignored and the password is everything after
  the first colon. Nothing links to it, since a GET that resets the game must not be reachable by a
  prefetch or a middle-click. Browsers cache Basic credentials, so a refresh of that page starts
  another run, and the page says so.
- A new game is named by the policy with a RAM write (`Policy::player_name` →
  `write_player_name` → `wPlayerName`), because the run starts from `data::START_OF_GAME`, a state
  past both name screens. Seven characters (`MAX_PLAYER_NAME`), not the nickname's ten. A resume is
  never renamed: the name is in the save and the game has printed it everywhere. LLM is the constant
  `AI` (`llm_policy::PLAYER_NAME`), console is `HUMAN`, random draws off its seed, scripted declines.
  `policy::policy_helper_tests::every_name_a_policy_can_choose_is_one_the_game_will_take` checks
  every name by round-tripping it through `PokemonString::from_string`, not by asking whether it is
  alphanumeric.
- A run directory has exactly one writer, and everything that needs to know which one reads
  `run::CurrentRun` per write: the checkpointer, the transcript thread (per event, or a captured
  path appends the new run's events to the old file), `/api/history`, `/api/healthz`. The one
  deliberate exception is `llm::history`, which captures its directory so a turn in flight when the
  swap lands is filed with the old game; `Worker::apply_reset` therefore uses `History::fresh`,
  never `History::open`, and reopens `TodoList` and `BattleScript` from the new directory.
- Three traps on the swap, each with a test: checkpoint the outgoing run first (up to a minute lives
  only in memory); `VideoEncoder::restart`, not `default` (deltas diff against `last_sent`, but
  `seq` must survive or a live viewer discards the whole new run); clear `last_status`, or
  send-on-change suppresses the heartbeat that says the run changed.

## Clearing the conversation

- `POST /api/clear` is the *opposite* trade to a new run: the run, the save, the transcript and the
  battle script all carry on, and what goes is what the model remembers — the conversation and the
  plan. It is for the run that has talked itself into a corner, where the only cure used to be
  throwing the playthrough away with it. `EmulatorHost::clear_conversation` has the argument.
- ⚠️ **It is a request, not an act.** A run directory has one writer and it is the worker thread, so
  `history.json` and `todo.json` change at the top of the next `Worker::run_one`, not on the tick
  that answered. The endpoint's body says so. `LlmPolicy::clear_conversation` bumps the generation
  to make that next turn start now — for promptness, not for correctness; unlike a restart, a stale
  answer here is about a game that is still there.
- The two kinds share `Worker::apply_reset` and differ in two lines, both about files. `NewGame`
  *re-reads* the plan and the battle script from the directory it is given; `Cleared` *deletes* the
  plan from the directory it is already in (`TodoList::cleared`) and leaves the script alone. Get
  either backwards and the run plays on holding notes it was told to forget.
- `History::cleared`, never `open`: the clear's directory is the one that has been playing, so
  `open` would put the whole conversation straight back. It writes a `cleared` line into
  `conversation.jsonl` — the log keeps every message, as always — and ends the fresh history on
  `prompt::CLEARED_NOTE`, which is `RESUMED_NOTE`'s job on a harder case: a model shown six badges
  and no memory of earning any of them is a model about to file a bug.
- `Policy::clear_conversation` defaults to an **error**, not a no-op, so a run played by anything
  but `LlmPolicy` answers "this run is not being played by a model" instead of 200 to a request
  that did nothing.

## The end of the game

- A win is `wNumHoFTeams` going up, and nothing else. pokered increments it at the top of
  `AnimateHallOfFame`, the first frame of the ceremony, and it survives the credits' soft reset.
  `badges == 255` is Viridian Gym, an hour early; `map == HallOfFame` is a cutscene with three
  script stages before the counter moves, which is why `post-hall-of-fame.bin` has
  `wNumHoFTeams == 0` and is a seed for a detection test, not something to assert on as loaded.
- `PokemonAgent::check_hall_of_fame` reads the MMU directly, above `update`'s `game_mode()?` (which
  returns on every screen transition, and a ceremony is made of them), and seeds its baseline from
  RAM on the first tick (`Option<u8>`) so a resume or a postgame fixture does not re-announce a
  victory. `RunMeta::completed` is the second guard, for a checkpoint taken just before the
  increment. The comment on `check_hall_of_fame` has the whole argument.
- The archive is `$GB_RUN_DIR/hall-of-fame/<stamp>-<run-id>/` and that nesting is load-bearing:
  `run::resumable` lists direct children of `$GB_RUN_DIR` and continues the newest, and an archive
  beside the runs would be it.
- `archive` copies every artifact by name (state, SRAM, meta, `todo.json`, `battle-script.json`,
  `memories/`, both transcript files and both conversation files, rotated halves included). There
  is no "copy everything", so a new run-directory artifact is dropped silently until the assertion
  list in `an_archive_carries_the_whole_run_including_the_conversation_and_is_not_resumable` gains
  a line. `issues/` and `press-buttons/` are not copied today.
- The transcript is followed, not copied: the completion event is written by another thread after
  the archive is triggered, so `fs::copy` would miss it or tear a line. `publish_event` returns the
  seq and the follow reads until it sees it (5 s deadline), before `start_new_run`.
- The ledger is JSONL, not SQLite (ten rows sorted in memory is not a query workload, and
  `rusqlite` would add a C dependency). It ranks on the cartridge's `wPlayTime`, stored as seconds
  because the hours field runs to 255 and a lexical `HH:MM:SS` sort breaks past 100 hours.

## Figures across resumes and restarts

- `RunProgress` is rebased onto the baseline `RunDir::baseline` reads once at open, before the
  first checkpoint overwrites `meta.json`; otherwise a run resumed nightly reports the last night
  as the whole playthrough. Tokens and turns are counted from `Decision` events that landed, not
  `max(turn)`, because a turn id is the worker's cancellation generation.
- `StatusSnapshot::wall_ms` and `emulated_ms` are elapsed since this process started playing the
  current run, minus parked time, and restart at zero on a new run, which is what lets the page
  spot a reset. `run_emulated_ms` is baseline plus this process's share and is what the panel and
  the next checkpoint both use, asked of `CurrentRun` on every heartbeat rather than mirrored into
  a host field.
- After the credits pokered clears WRAM, so `agent.update` answers `Err("Not in game")` for ever.
  The host publishes an agent failure on change only, or a finished run floods the transcript and
  every open page fifty times a second.

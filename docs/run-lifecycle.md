# Run lifecycle: a new run in place, a cleared conversation, and the end of the game

Read before touching `poke-agent/src/run/`, `poke-agent-web/src/host.rs`'s new-run, clear and completion seams,
`/reset-game`, `POST /api/new-run` or `POST /api/clear`.

## Starting a new run

- The three admin endpoints are the only channel from HTTP back into the emulator. One mailbox
  carries a closed two-variant enum, answered at the top of `EmulatorHost::tick` where nothing is
  half-done; either request outstanding refuses the other, and the refusal names which.
- All three are off unless `GB_ADMIN_TOKEN` is set and 404 rather than 403 when it is not, because
  the server is public and should not advertise a reset endpoint. `/reset-game` answers with a Basic
  challenge so the browser collects the password and the SPA holds no token; nothing links to it,
  since a GET that resets the game must not be reachable by a prefetch or a middle-click. Browsers
  cache Basic credentials, so a refresh of that page starts another run and the page says so.
- A run directory has exactly one writer, and everything that needs to know which reads `CurrentRun`
  per write. The one exception is `llm::history`, which captures its directory so a turn in flight
  when the swap lands is filed with the old game.
- Three traps on the swap: checkpoint the outgoing run first, because up to a minute lives only in
  memory; restart the video encoder rather than replacing it, because `seq` must survive or a live
  viewer discards the whole new run; and clear `last_status`, or send-on-change suppresses the
  heartbeat that says the run changed.

## Clearing the conversation

- `POST /api/clear` is the opposite trade to a new run: the run, the save, the transcript and the
  battle script carry on, and what goes is the conversation and the plan.
- It is a request, not an act. The worker thread is the directory's only writer, so the files change
  at the top of the next turn; the endpoint's body says so, and the generation bump that makes that
  turn start now is for promptness rather than correctness.
- The two reset kinds share one path and differ in two lines, both about files: a new game *re-reads*
  the plan and the script from the directory it is given, a clear *deletes* the plan from the
  directory it is already in and leaves the script alone. Backwards, the run plays on holding notes
  it was told to forget.
- The fresh history ends on `CLEARED_NOTE`, because a model shown six badges and no memory of
  earning any of them is a model about to file a bug.

## The end of the game

- A win is `wNumHoFTeams` going up and nothing else. It is incremented on the ceremony's first frame
  and survives the credits' soft reset; `badges == 255` is Viridian Gym an hour early, and
  `map == HallOfFame` is a cutscene with three script stages before the counter moves.
- `check_hall_of_fame` reads the MMU directly, above `update`'s game-mode read (which returns on
  every screen transition, and a ceremony is made of them), and seeds its baseline from RAM on the
  first tick so a resume does not re-announce a victory.
- The archive nests under `hall-of-fame/`, and that is load-bearing: `run::resumable` lists direct
  children of `$GB_RUN_DIR` and continues the newest.
- `archive` copies every artifact by name. There is no "copy everything", so a new run-directory
  artifact is dropped silently until the assertion list in the archive test gains a line.
- The transcript is followed rather than copied: the completion event is written by another thread
  after the archive is triggered, so a plain copy would miss it or tear a line.

## Figures across resumes and restarts

- `RunProgress` is rebased onto the baseline read once at open, before the first checkpoint
  overwrites `meta.json`; otherwise a run resumed nightly reports the last night as the whole
  playthrough. Tokens and turns are counted from decisions that landed, not from `max(turn)`, because
  a turn id is the worker's cancellation generation.
- After the credits pokered clears WRAM, so the agent answers an error for ever. The host publishes
  that on change only, or a finished run floods every open page fifty times a second.

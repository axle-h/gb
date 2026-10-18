# The LLM turn loop, the tools and the prompt

Read before touching anything under `poke-agent/src/llm/`, `poke-agent/src/pokemon/llm_policy.rs`, or what the
model is sent. The README tells the story; this is what a change must not break, and where the
argument lives in the code.

## The history

- Message 0 is a constant re-minted on every start, because a prompt cache is keyed on the prefix.
  After it the history only grows at the end. The plan is a file and never sent with a turn; the
  turn closes on `prompt::PlanReminder`, louder on the first turn after a compaction, a restart or a
  clear, and a rollback puts the louder one back (`Worker::roll_back_failed_turn`).
- `protocol::history_safe` rewrites a tool call whose `arguments` is not a JSON object to `{}`, in
  place and only the broken ones: the assistant message is replayed on every request, so one bad
  completion poisons the rest of the run.
- Both history files are image-evicted, because a restored image defaults to 85 tokens against a real
  765-3825 and compaction would never fire again.
- The checkpoint sits between `decide` returning and `outcomes.send`: durability before visibility,
  since a winning action has the archiver copy the directory on the next tick.
- The stored system prompt is compared, never restored. `RESUMED_NOTE` tells the model once that the
  save is behind the conversation, and `CLEARED_NOTE` says an erasure was deliberate — a model
  looking at a run it cannot remember otherwise concludes the game is broken.

- A turn cancelled before it was answered hands its events to the turn that replaces it, and they
  are spent only when an answer lands. Otherwise what happened while the model was thinking is told
  to nobody.

## The tools

- Every turn ends with `choose_action`, whose spec is the same on every kind: what varies is the
  menu in the turn. Its `summary`, once per reply, is enforced by `classify_reply` rather than by the
  schema, because a field the schema requires and the parser allows is one a weak model omits.
- `press:` is a row on the watchdog turn only, and refused on every other. A write under `issues/`
  does not end the turn and its answer must not read like a fix. Both write a record whose save is taken for each edge into
  `AwaitingLlm` (`Published::asks`), never the periodic checkpoint, and waited for, since an
  endpoint can answer before the emulator thread's next tick.
- The tools are the same four on every request, the summary request's too (`worker::chat_request`),
  because every provider's cache prefix starts with them; a file is readable, and the writable ones
  writable, on every turn. `files::resolve` is the path table, and
  `Node::on_the_emulator_thread` says which reads need the game.
- Nothing a read answers may duplicate the situation. A route file's "not walked there yet" is
  never "unreachable" — it answers out of the map-header graph while the action menu answers out
  of walkable connectivity, so `files::route_text` turns a route that cannot be *started* into a
  warning naming what the terrace ends on.
- `not_on_the_menu`'s "that id is for another map" clause must test against `Map::iter()`. Testing
  "the first id contains a colon" made the commonest refusal in a battle — an item the bag has run
  out of — answer with three false statements about maps.
- `classify` takes the ids the situation was rendered from and nothing else, so anything a turn
  names has to be a row of its own. Naming an id in a row's prose instead put the walk in front of
  a wall with no way past: the turn advertised it and `choose_action` refused it, turn after turn.
- A refused battle id carries the cartridge's rule, read off the menu rather than the game, and says
  nothing where the menu cannot settle which rule it is. The turn's own `### On screen` line names
  the row, so a bare "not one of this turn's actions" is a contradiction with no way out.
- A row that acts on something on the map names the thing in its id, as
  `item:PokeFlute@Route12:Snorlax` does (`tools::thing_at`), never a square the model must work out.
- A field row is generated only where `resolve_field_move` accepts it (`tools::field_rows`), so the
  game's refusals belong there, and a use no driver can finish (a bare Poké Flute) gets no row.
  `items::blocked` is asked up front, because `ItemUseNotTime` consumes nothing and prints a box
  that reads like success.
- A `<slot>`, `<box slot>`, `<move>` or `<item>` row is one row for every value, an Ether's
  `<slot>:<move>` one row per item: `classify` takes any value on its syntax (`tools::fills`) and a
  refused one is refused when the row is taken, in `resolve_field_move`'s own sentence, so the row's
  prose names the values that work.
- A field move lands in a chain only when its driver says the game took it
  (`Policy::field_move_ended`, said once by both agents); a refusal drops the rest as a stopped
  walk does, and a driver that hands back mute is a refusal (`FIELD_MOVE_UNREPORTED`), never a
  landing.
- An action the game would refuse is not offered, on the map so scripted policies are held to it too.
  `prompt`'s `Blocked here:` line names what would clear a cuttable tree and nothing else — naming
  water fired on every coast.
- There is no way to name a party member, so the Day Care and the Name Rater are declined rather than
  guessed at. All nine in-game trades work, because each has exactly one legal answer and the agent
  gives it.
- Cut, Strength and Surf have no rows of their own: each is the ending of a walk row. As a second
  half of a pair whose first half was a menu row, one decision cost two paid requests and the first
  completed looking like success. A boulder row is one per *target*: its prose names the boulder, because a floor with two
  holes has exactly one boulder that can reach each — but its id names only the target, because the
  boulder and the start square move on every push and an id built from either is a new id after every
  shove. There is no per-shove row and no route step for one, so the two layers cannot disagree about
  a square.
- The overworld turn names a map's boulders, switches and cuttable trees at their own coordinates,
  unconditionally, and says either that each reachable target is a row below or which half of
  Strength or Cut is missing. The "no boulder rows" branch counts rows off `actions()`, never off
  `boulder_pushes()`: a floor can have legal shoves left and still offer no goal.
- The PC's rows come out of `usable_pc` rather than `pc_locations_for`: Bill's is his cell separator
  and the bedroom's stores items only.
- The bin line reads `hidden_objects_for(map)` and nothing else — handing over `trash_cans` would
  walk the gym in two requests, and a test asserts the turn is byte-identical whatever the puzzle
  state says.
- A reply's `choose_action` calls fold into one `Terminal` in `classify_reply`, all or none, each
  checked against the same menu, and each hop is re-resolved against the live game
  (`tools::resolve_row`). Only a landed action advances it. A walk a battle stopped is taken up
  again, capped, unless a party member fainted in that battle (`LlmPolicy::battle_faints`).
  A walk stopped by a message that left the player on the same square, with no battle after it, is
  taken up again without asking (`MAX_NOTICE_RESUMES`); a walk-back or a trainer still hands it back.
- A chain stalls on a pace that ended in an encounter and reported nothing. No test in this file
  could see it: they pin what happens *given* silence, and cannot say which endings are silent. A
  driver that reports nothing still exists, so the hole can be dug again.

## The battle script

- `battle_script::run` is called from `pick_battle_action` on the emulator thread, before `advance`,
  and returns early on a turn already in flight. The script filters `policy::battle_options` and
  never invents an action, so `facts` is computed from that same list — in a ghost battle every
  move's damage is 0 and `battle.ghost` says why.
- One engine builder for live runs and validation: operation, runtime, size and depth caps, `eval`
  disabled, never rhai's `unchecked`, under `catch_unwind`. `switch_to` and `move_type` are named
  around rhai reserved words.
- Validation is not a proof; the disarm is. One strike disarms for the run and keeps the source. The
  policy disarms through the live cell and never writes the file.
- On disk the source is history and the rest is state: every version is a line of
  `battle-script-history.jsonl`, and `battle-script-state.json` names the version it describes, so
  a replaced script's disarm is not believed (`battle_script::Current`). The plan is the same
  history alone, and an older run's files are split into them once, on open.
- A script opens with a comment saying what it is for, and a write leaving none is refused
  (`battle_script::purpose_of`): that line is what the turn quotes back. A write arms it from the
  next battle decision, whatever the turn it was written on.

## The battle report

- Damage is a diff of HP between consecutive decisions and the prose is the cartridge's, because
  there is no per-turn outcome event. The report is rendered into the situation, not appended as a
  message, and `events_mark` takes back the message boxes it already narrates.
- It takes back its own battle and no more: `events_end` is stamped when `BattleEnded` arrives, so a
  verdict that lands after it, such as a pickup a trainer's battle interrupted, is still news.
- A blackout is detected on the cartridge's own sentence, because `wBattleResult` is zeroed before
  anything here runs, and its arm sits above the in-battle arm.
- `handed_back` returns the turns the script took since the model last chose. Without it a model
  asked mid-battle sees a fight in which its own last decision was silently replaced.
- What the game says after the model's own last choice is `BattleReport::after_answered`, never the
  script's last turn: a model's turn opens no report turn, and a walk resumed after the battle asks
  nothing in between, so the faint, the level and the evolution had nowhere else to go.
- `take_over` stops the script deciding the rest of *this* battle and nothing more. It cannot be
  scoped to the run: a disarm reached for mid-battle is one nothing brings back. It is a row only on
  the turn the script handed back (`offer_take_over`), sent beside that turn's action.

## The plan and the prompt

- The system prompt has to keep saying: the game is not broken, retrying is not a plan, prior
  knowledge of Red is not evidence, what people say is the instruction, and how to play well. Two
  tests pin the wording.

## The wire and the endpoint

- A dated 429 parks: clamped, release unconditional, the same request re-sent because the world did
  not move. The pause seam sits below the reset seam and skips the emulator and the agent only. An
  undated 429 keeps the ordinary backoff.
- `anthropic::render` sends every reply back as it streamed, signed thinking included, wherever the
  tools and messages before it are the ones it was written after (`anthropic::Prefix`), so a reply
  loses its thinking only where the history was rewritten and each request's prefix is the last
  one's. The blocks ride on their `Message` (`anthropic::Reply`) and `history.json` saves them beside
  it, so a restart replays them too; `Prefix` is a stable hash because that digest is saved with them.
  An image `history.json` evicted is a rewrite, and costs the thinking from there on.
- `anthropic::request_body` puts a breakpoint on the system prompt as well as the automatic one at
  the end, because a last turn longer than the lookback would otherwise miss the tools and the
  system prompt too.
- `GB_COMPACT_ABOVE` headroom is absolute, because a turn grows unchecked to `GB_MAX_TOOL_STEPS`
  between compactions. A local endpoint's limit is its KV cache per slot, not its window.

## A turn that failed outright

- A failed turn is rolled back whole, to a *length* rather than by one `pop`: a turn can fail on its
  second tool step with an assistant message and its results already appended.
- A compaction asks for the summary of the history as it stands, nothing evicted first, and keeps
  the system prompt, the summary and the last two turns untouched; the instruction, the summary's
  own reads and its reply are not kept. Its reads are answered from whatever state is current
  (`ToolBatch::turn` is `None`), and only `plan.md` may be written.
- A summary request's failure is classified before anything is dropped (`Worker::stream_summary`):
  a transient one is retried until it clears, parked as a turn is; a genuine one is asked once more
  without the oldest half of the turns, if there is one to leave out, and a second keeps an earlier
  summary with a note after it that what followed went, or the note alone in the summary's place
  (`compaction::dropped_opening`). Still over the threshold, it keeps one turn, then none, then raises an error
  notice, because from there on every request may be over the window; with two turns or fewer
  since the last summary it goes straight there rather than pay for one that says nothing new.
- A turn's request the endpoint rejects as too long (`LlmError::is_context_overflow`) forces a
  compaction and asks the same question again, once (`Worker::decide`). Any other rejection fails
  the turn with the history as it was, since a compaction cures nothing else.
- An account refusal, a 401, 402 or 403, counts toward `RefusalPark`, and any other answer resets
  that streak; a timeout, a dropped connection or exhausted 5xx retries must neither count nor park,
  or one bad hour of a flaky endpoint stops the game for thirty minutes. Turns failed by a rejected
  request are a streak of their own on the same windows, reset by any completion
  (`Worker::fail_turn`), so a wrong setting parks rather than being asked about every turn.
- `Usage::prompt_tokens` counts cached tokens too; the cache reads and writes ride beside it
  (`cache_read_tokens`, `cache_write_tokens`) and reach the transcript on `UiEventBody::Usage`, one
  per completion, from `Worker::account_for` and `ask_for_summary`.

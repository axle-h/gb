# The LLM turn loop, the tools and the prompt

Read before touching anything under `poke-agent/src/llm/`, `poke-agent/src/pokemon/llm_policy.rs`, or what the
model is sent. The README tells the story; this is what a change must not break, and where the
argument lives in the code.

## The history

- Message 0 is a constant re-minted on every start, because a prompt cache is keyed on the prefix.
  The plan is a `user` message appended only when it differs from the last copy; stale copies stay,
  and `compaction::is_turn_start` refuses to cut between a plan and its turn. After message 0 the
  history only grows at the end.
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

## The tools

- Every terminal tool's `summary` is enforced by `classify` rather than by the schema, because a
  field the schema requires and the parser allows is one a weak model omits.
- `press_buttons` is offered on the watchdog turn only. `report_issue` does not end the turn and its
  answer must not read like a fix. Both write a record whose `state.gbst` is taken on the edge into
  `AwaitingLlm`, never the periodic checkpoint.
- Nothing a read answers may duplicate the situation. `read_route`'s `None` means "not walked there
  yet", never "unreachable" — it answers out of the map-header graph while the action menu answers
  out of walkable connectivity, so `route_answer` turns a route that cannot be *started* into a
  warning naming what the terrace ends on.
- `not_on_the_menu`'s "that id is for another map" clause must test against `Map::iter()`. Testing
  "the first id contains a colon" made the commonest refusal in a battle — an item the bag has run
  out of — answer with three false statements about maps.
- A refused battle id carries the cartridge's rule, read off the menu rather than the game, and says
  nothing where the menu cannot settle which rule it is. The turn's own `### On screen` line names
  the row, so a bare "not one of this turn's actions" is a contradiction with no way out.
- A row that leads to a coordinate being asked for names both squares: an id's coordinate is where
  the player stands, `use_field_move`'s `target` is where the thing is. A sprite id now carries no
  coordinate at all, so only boulder and field-move rows carry two.
- `use_item`'s `target` is optional: the Bicycle, a Repel and the Itemfinder have no tile to aim at,
  and requiring one had shut off half of `ItemUsePtrTable` with a driver and tests behind it.
  `items::blocked` is asked up front, because `ItemUseNotTime` consumes nothing and prints a box that
  reads like success.
- An action the game would refuse is not offered, on the map so scripted policies are held to it too.
  `prompt`'s `Blocked here:` line names what would clear a cuttable tree and nothing else — naming
  water fired on every coast.
- There is no way to name a party member, so the Day Care and the Name Rater are declined rather than
  guessed at. All nine in-game trades work, because each has exactly one legal answer and the agent
  gives it.
- `cut`, `push_boulder` and `strength` are not verbs. Each was the second half of a pair whose first
  half was a menu row, so one decision cost two paid requests and the first completed looking like
  success. A boulder row is one per *target*: its prose names the boulder, because a floor with two
  holes has exactly one boulder that can reach each — but its id names only the target, because the
  boulder and the start square move on every push and an id built from either is a new id after every
  shove. There is no per-shove row and no route step for one, so the two layers cannot disagree about
  a square.
- The overworld turn names a map's boulders, switches and cuttable trees at their own coordinates,
  unconditionally, and says either that each reachable target is a row below or which half of
  Strength or Cut is missing. The "no boulder rows" branch counts rows off `actions()`, never off
  `boulder_pushes()`: a floor can have legal shoves left and still offer no goal.
- The bin line reads `hidden_objects_for(map)` and nothing else — handing over `trash_cans` would
  walk the gym in two requests, and a test asserts the turn is byte-identical whatever the puzzle
  state says.
- A chain is checked against the same menu and each hop is re-resolved against a fresh `actions()`.
  Only a landed action advances it. `resume_after_battle` is opt-in, battles only, and capped.
- `resume_after_battle` was dead on tall grass, because a pace that ended in an encounter emitted no
  abort at all. No test in this file could see it: they pin what happens *given* silence, and cannot
  say which endings are silent. A driver that reports nothing still exists, so the hole can be dug
  again.

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

## The battle report

- Damage is a diff of HP between consecutive decisions and the prose is the cartridge's, because
  there is no per-turn outcome event. The report is rendered into the situation, not appended as a
  message, and `events_mark` takes back the message boxes it already narrates.
- A blackout is detected on the cartridge's own sentence, because `wBattleResult` is zeroed before
  anything here runs, and its arm sits above the in-battle arm.
- `handed_back` returns the turns the script took since the model last chose. Without it a model
  asked mid-battle sees a fight in which its own last decision was silently replaced.
- `take_over` stops the script deciding the rest of *this* battle and nothing more. It cannot be
  scoped to the run: a disarm reached for mid-battle is one nothing brings back.

## The plan and the prompt

- The system prompt has to keep saying: the game is not broken, retrying is not a plan, prior
  knowledge of Red is not evidence, what people say is the instruction, and how to play well. Two
  tests pin the wording.

## The wire and the endpoint

- A dated 429 parks: clamped, release unconditional, the same request re-sent because the world did
  not move. The pause seam sits below the reset seam and skips the emulator and the agent only. An
  undated 429 keeps the ordinary backoff.
- `GB_COMPACT_ABOVE` headroom is absolute, because a turn grows unchecked to `GB_MAX_TOOL_STEPS`
  between compactions. A local endpoint's limit is its KV cache per slot, not its window.

## A turn that failed outright

- A failed turn is rolled back whole, to a *length* rather than by one `pop`: a turn can fail on its
  second tool step with an assistant message and its results already appended.
- `turns_since_plan` is restored with it, or a run whose every request fails appends a fresh plan
  message every tenth attempt and nothing removes one — 1373 copies, measured.
- A compaction that can drop nothing says so. `trim_history` cuts only at turn boundaries and a plan
  message is not one, so a history of unanswered plans had no boundary anywhere in it and published
  `before == after` as a success. `drop_unanswered` is the pass below it. A compaction that still
  reclaims nothing raises an error notice, because from there on every request is over the window.
- Only a refusal counts toward `RefusalPark`: an `Http` that `is_retryable` rejects. A timeout, a
  dropped connection or exhausted 5xx retries must neither count nor park, and any other answer
  resets the streak, or one bad hour of a flaky endpoint stops the game for thirty minutes.

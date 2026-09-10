# The LLM turn loop, the tools and the prompt

Read before touching anything under `src/llm/`, `src/pokemon/llm_policy.rs`, or what the model is
sent. The README tells the story; this is what a change must not break, and where the argument
lives in the code.

## The history

- Message 0 is a constant, re-minted from `prompt::system_message()` on every start, because a
  prompt cache is keyed on the prefix. The plan is a `user` message (`prompt::plan_message`)
  appended by `Worker::sync_plan` only when it differs from the last `is_plan` copy; stale copies
  stay, and `compaction::is_turn_start` refuses to cut between a plan and its turn. After message 0
  the history is append-only.
- `protocol::history_safe` rewrites a tool call whose `arguments` is not a JSON object to `{}`, in
  place and only the broken ones: the assistant message is replayed on every request, so one bad
  completion (a router produces one daily) poisons the rest of the run.
- Two files: `history.json` (rewritten atomically, what a restart resumes) and
  `conversation.jsonl` (append-only, what a compaction replaced). Both are image-evicted, because a
  restored `ImageUrl` defaults to `IMAGE_TOKENS` (85) against a real 765–3825 and compaction would
  never fire again.
- The checkpoint sits between `decide` returning and `outcomes.send`: durability before visibility,
  since a winning action has the archiver copy the directory on the next tick. The run directory is
  captured, not re-read per write, and `apply_reset` uses `History::fresh` or `History::cleared`.
- `Accounting::resumed` carries the calibration, not the totals (those are rebased by
  `RunDir::checkpoint`). The stored system prompt is compared, never restored; a change logs a
  `warn` and one `system_prompt_changed` line. `prompt::RESUMED_NOTE` tells the model once that the
  save is behind the conversation; `GB_RESTORE_HISTORY=0` starts the conversation over.
- `POST /api/clear` starts it over on a live process instead: `Worker::apply_reset` with
  `ResetKind::Cleared` replaces the history (`History::cleared`), **deletes** `todo.json`
  (`TodoList::cleared`) and resets the accounting, at the top of the next turn because the worker
  thread is the run directory's only writer. `prompt::CLEARED_NOTE` is `RESUMED_NOTE`'s harder
  twin: it says the erasure was deliberate, so a model looking at a run it cannot remember does not
  conclude the game is broken. See `docs/run-lifecycle.md`.

## The tools

- Every terminal tool's `summary` is enforced by `classify` (added by `add_summary_argument` in
  `for_kind`), because a field the schema requires and the parser allows is a field a weak model
  omits. It reaches the page as `Decision.narration`.
- `press_buttons` is offered on `Stuck` only, with `why` enforced. `report_issue` is on Overworld,
  Battle and Stuck (`offers_issue_report`), does not end the turn, and its answer must not read
  like a fix. Records go to `issues/turn-<id>/` and `press-buttons/`: `incident.json` (holding a
  three-turn image-evicted conversation slice), `screen.png`, and a `state.gbst` the emulator thread
  takes on the edge into `AwaitingLlm`, never the periodic checkpoint. The path is re-read from
  `CurrentRun` per record.
- Reads are scoped per kind (`ReadTool::kinds`), `non_terminal_names` with them, and a read that
  exists but is not offered here is refused by name. Nothing a read answers may duplicate the
  situation (`read_screen_text` and `read_trainer` died that way; `MapView` and `BattleView` carry
  no menus). `read_route`'s `None` means "not walked there yet", never "unreachable". The plan is
  the only notes mechanism; `memory_*` is gone and `run::files::MEMORIES` survives for old
  archives.
- ⚠️ **`read_route` answers out of the map-header graph and the action menu answers out of walkable
  connectivity, and they disagree.** `observe::route_from` flags the second hop's leaving tile
  against `reachable_tiles()` and `route_answer` turns a `false` into a `warning` that says the
  route cannot be *started*, what the terrace ends on, and that the join between two parts of one
  map is usually a door. Cerulean City cost a deployed run 65 turns and ten issue reports; the
  answer was in its own action menu the whole time.
- A menu row is `` `{map}:{x},{y}:{kind}` — {verb phrase} `` (`take the warp to PalletTown,
  arriving at (12, 12)`, `talk to Mom`), the verb from the sprite's `PictureId`, no step count. The
  map prefix is not redundancy: `resolve_overworld` re-mints ids against the current map and an
  answer can land after a warp. `tools::not_on_the_menu` rejects an id not in the turn's own menu
  inside the turn (an empty menu checks nothing; `resolve_overworld` stays the authority for what
  changed since).
- ⚠️ **`use_field_move` has no `interact`, and the agent cannot collect hidden items at all**
  (2026-09-03). It let the model press A at any square it named, and its only remaining job was
  hidden items, since every hidden *object* a playthrough needs is a `MetaTile::Switch` row. All 212
  hidden events in `data/events/hidden_events.asm` yield loot and not one yields a key item, an HM or
  a TM, so nothing is gated behind one. It cost a deployed run 85 minutes across three Silph Co
  floors — and its own issue report shows it had decided `interact` was a way to *walk*, which no
  refusal text would have unlearned. `PolicyStep::SearchHiddenItem`, `aides::hidden_items` and
  `MetaTileMap::hidden_items` went with it; `FieldMove::CheckTrashCan` stays, because the gym bins,
  the Mansion statues and the Game Corner poster are the same mechanic and are progression gates.
  Removing the verb and the `facing` property only it used gave the Overworld tool array **370
  bytes** back, its largest single reclaim.
- ⚠️ **`not_on_the_menu`'s "that id is for another map" clause fires only where the name really is a
  `Map`.** It used to test "the menu's first id contains a colon", and a battle row does:
  `fight:Peck`, `item:PokeBall`, `switch:1`. So the commonest refusal in a battle — an item the bag
  has just run out of — was answered with *"That id is for `item` and you are in `fight`; ids are
  minted for the map you are standing on"*, three false statements about maps to a model standing in
  a fight. Checking against `Map::iter()` also stops an id the model invented being reported as
  another map's.
- ⭐ **A refused *battle* id carries the cartridge's rule, because the prompt forbids the model from
  knowing it.** The turn's own `### On screen` line reads `FIGHT Pokémon ITEM RUN`, so "`run` is not
  one of this turn's actions" is a contradiction with no way out of it — the shape behind the
  ViridianGym and Route 22 issue reports. `tools::battle_rule_behind` appends the reason where the
  *menu* can settle which one it is (no `run` row beside `fight:` rows can only be a trainer battle;
  a missing `item:` row means the bag is out) and **nothing** where it cannot — the Safari menu gets
  neither, and the ghost battle has no arm because `prompt` already says on the turn that no move,
  ball or switch does anything until the Silph Scope. Read off the menu rather than the game, because
  `classify` does not touch the game and that is worth keeping.
  `tools::a_refused_battle_id_carries_the_rule_and_says_nothing_about_maps`, and
  `docs/coverage-plan.md` step 7 for how both were found.
- ⚠️ **There is no way to name a party *member*, and what that costs is the Day Care and the Name
  Rater.** `FieldMoveRequest` leaves out `UsePartyScript` along with the other postgame mechanisms,
  on the argument that their arguments are internal types and none is on the path to the Hall of
  Fame — both still true. Out of battle, three scripts open a party menu (`grep DisplayPartyMenu`):
  an in-game **trade**, which needs no tool because it has exactly one legal answer and the agent
  gives it (`PartyMenuAnswer`, and [pokemon-agent](pokemon-agent.md)); and the Day Care and the Name
  Rater, which are real choices with no way to express them, so the agent **declines** them rather
  than guessing. A run therefore cannot board or rename a Pokémon through the LLM path today, and
  cannot lose one to a menu it never answered either. All nine trades work — each is worth two dex
  entries, and five species are obtainable no other way on one cartridge.
  `branch_points::a_trade_finds_the_give_species_wherever_it_is_in_the_party`,
  `postgame::trades::every_in_game_trade_can_be_made_by_talking_to_the_trader`, and
  `docs/coverage-plan.md` step 8.
- **A row that leads to a coordinate being asked for names both squares.** An id's coordinate is
  where the *player stands*; `use_field_move`'s `target` is where the *thing is*.
  `Route16:27,10:Snorlax` with the Snorlax on (26, 10) had a run play the Poké Flute at open
  ground three times, each answered "Accepted". ⭐ **A sprite id has carried no coordinate at all
  since 2026-09-08** — `OverworldAction::id` keys it on `map + name`, because the square it used to
  name was the player's approach tile and moved every time the player did — so the row above is
  `Route16:Snorlax` now and there is no second number to mistake for the first. The refusal below
  stays: it is what catches a target invented from somewhere else. `resolve_field_move` refuses a `use_item` whose
  target is `Empty`/`Grass`/`Water` or off the map, or that cannot be faced, and names what is
  beside it. Only those rows carry both coordinates: a person the agent walks to needs no number. A
  `MetaTile::Boulder` row carries them for the same reason — a Sokoban puzzle is reasoned about in
  the boulder's coordinates and walked in the player's.
- ⭐ **`use_item`'s `target` is optional since 2026-09-10, and requiring it had shut off half of
  `ItemUsePtrTable`.** The Bicycle has no tile to aim at, and neither does a Repel or the
  Itemfinder — so `FieldMove::UseBagItem` and the whole of `UseTarget::Nothing` had a driver
  (`postgame::items`), a refusal table and a test that rides a bike, with **no way in from any
  turn**. It surfaced as a coverage gap rather than as a tool bug: Route 17 is Cycling Road, its
  gate's guard stops a walker without the Bicycle, and it was `unreached` on every sweep. A `slot` rides
  along for the party items, so a Potion or a vitamin out of battle is reachable too;
  `UseTarget::Move` is deliberately still not, because it would need a `move_index` on the schema
  and nothing in the game is gated behind an Ether. ⚠️ A `target` that *is* given is checked exactly
  as before — a mistyped one must never read as "use it on myself" — and `items::blocked` is asked
  up front, because `ItemUseNotTime` consumes nothing and prints a box that reads like success.
  Costed at **+189 bytes** on the Overworld catalogue; the ledger is in
  `tools::tests::the_tool_array_stays_within_its_budget`.
- **`resolve_overworld` falls back to `connection_action`** for an id naming a `Connection` tile
  `actions()` did not mint, and a connection row lists the other reachable landing groups by id
  (`MetaTileMap::crossings`). Without it the model could not say "same map, different door", and one
  run crossed into Route 14's six-tile pocket twice running. The id is re-minted from the tile
  through `overworld_id`, never parsed, so the row's prose and the resolver cannot drift.
- **Several warps to one map are told apart by which side of the building they are on**
  (`tools::door_side`, only where a destination repeats, since a double-wide front door is two tiles
  onto one square). A gate house's warps are all `LAST_MAP`, so its menu was three identical
  `Warp → Route7` rows and a run filed a bug about a missing Saffron warp.
- An action the game would refuse is not offered. `MetaTileMap::can_cut`/`can_strength`/`can_surf`
  keep `CutTree` rows, boulder pushes and water crossings out of `actions()` (on the map, so
  scripted policies are held to it), `tools::hm_available` refuses the call against `HM_BADGES` with
  separate complaints for "nobody knows it" and "no badge", and `agent.rs` refuses on the way into
  `CuttingTree` and `PushingBoulder` with decisions still coming. `prompt`'s `Blocked here:` line
  names what would clear a `CutTree` and nothing else: naming water fired on every coast and one run
  spent 65 turns blaming it. `fly_bike::blocked_by` still does not check the Thunder Badge.
- ⚠️ **`cut`, `push_boulder` and `strength` are gone from `use_field_move` (2026-09-04), and what
  replaced them is that the walk finishes the job.** Each was the second half of a pair whose first
  half was a menu row, so one decision cost two paid requests and the first of them completed
  looking like success. `MetaTile::Cut { at }` now cuts the tree it walks up to and
  `MetaTile::BoulderGoal { boulder, at, hole }` is one row per **target** — one decision that arms
  `BIT_STRENGTH_ACTIVE`, walks, and keeps shoving that named boulder until it is on that switch or
  down that hole (`AgentState::SolvingBoulderPuzzle` → `PushingBoulder`); both hand off from
  `OverworldMovement`'s empty-route arm, the seam the fishing row already used. Both carry the square
  they are about, so a row can name it and the re-derived walk goes to *that* one; `Cut`'s `id_kind`
  is still `CutTree`, because an id is a key a resumed run quotes out of its own history. `strength`
  went because arming was never a decision — the flag is cleared by every map change and forgetting it is
  silent. A boulder's own sprite row is withheld from `overworld_menu` (pressing A at one does
  nothing), and the id is `{map}:{target}:PushBoulder{OntoSwitch,IntoHole}`. ⚠️ **The
  row's *prose* names the boulder** — Seafoam B3F has two holes and exactly one boulder that can
  reach each, so "whichever" spends the wrong one and strands the floor — but the **id names only
  the target**, because the boulder and the square the walk starts from both move on every push and
  an id built from either is a new id after every shove. The coverage walk of 2026-09-07 spent two
  and a half hours at one action a minute on VictoryRoad3F for exactly that reason: one puzzle was
  an unbounded family of rows the frontier had never seen.
  `endgame::a_boulder_goal_re_chosen_after_every_battle_still_arrives` pins it.
  There is **no per-shove row and no `FieldMove::PushBoulder` route step any more** — the scripted
  policy takes the same goal rows the model does, so the two layers cannot disagree about a square
  the way they did when VictoryRoad1F was lost.
- ⚠️ **The boulder refusal's "can you get to the push tile" arm was over-permissive and shipped a
  stall.** `MetaTileMap::reachable_tiles` is the key set of `bfs_from_player`, which records every
  *neighbour* of an open square — a route must be allowed to end at a door or a person — so a wall
  touching floor is in it and `reach.contains(stand)` put the player inside the wall. A deployed run
  was offered, and accepted, a push **left** on a boulder with solid rock to its east; three of the
  four boulders in the committed fixtures had the same hole and every one was that shape. The stand
  tile must now also be `Empty`/`Grass`/`Warp`, which is `solve_boulder_push`'s own `floor`
  predicate. `endgame::a_boulder_that_cannot_move_is_refused_rather_than_shoved_at` pins it.
- **The overworld turn names a map's boulders, its switches and its cuttable trees**, at their own
  coordinates, and says in the same line either that each reachable target is a row below or which
  half of Strength/Cut is missing. ⚠️ **The "there are no boulder rows" branch counts `BoulderGoal`
  rows off `actions()`, never `boulder_pushes()`**: a floor can have legal shoves left and still
  offer no goal (VictoryRoad1F once a run has sealed the only capable boulder), and promising rows
  the model then cannot find is what earned the issue report the tombstone above it records. Unconditional on purpose: a floor with three boulders and no
  Strength has no boulder rows at all, and a puzzle with no way to touch it and nothing said about
  why is what sent a run round Route 2 for eleven turns. ⚠️ The switches are named where
  `wFirstLockTrashCanIndex` is not, because a boulder switch is a tile the game *draws*.
- Two more lines on the overworld turn, each with its own narrow trigger, each silent otherwise.
  The gate line fires when
  `warp_targets` holds several landings on one map and says nothing beyond it is behind a door here.
  The bin line fires on a map with `HiddenObject::TrashCan` and says a wrong second guess re-rolls
  the first switch and that `then` chains four bins per request. ⚠️ **The bin line reads
  `hidden_objects_for(map)` and nothing else.** `GameState::trash_cans` holds both switch positions
  and handing them over would walk the gym in two requests;
  `the_gym_bins_say_what_a_sweep_costs_without_saying_where_the_switches_are` asserts the turn is
  byte-identical whatever the puzzle state says, so relaxing it is a decision rather than a
  refactor.
- `read_bag` names all fifty TMs. `Bag` drops ids `ItemId` cannot name, and the count is the
  dangerous half at a 20-slot cap.
- `choose_action`'s `then` (up to `MAX_CHAINED_ACTIONS`, 4) is checked against the same menu.
  `advance_queue` runs before `advance` and touches no turn state. Only a landed action
  (`LlmPolicy::outcome`, from three events) advances a chain, and `take_current` re-resolves each
  hop against a fresh `actions()`. `resume_after_battle` is opt-in, battles only,
  `MAX_BATTLE_RESUMES` (5). A single stopped action gets no policy note; the agent already
  reported it.
- ⭐ **`resume_after_battle` was dead on tall grass until 2026-09-07, which is the commonest way in
  the game to meet a wild Pokémon.** It keys on `OverworldActionAborted { Battle }`, and a pace that
  ended in an encounter emitted no abort at all, so the queue was dropped as `Dropped::Unreported`
  and the model paid a fresh request per encounter. The fix is in `agent.rs`
  ([pokemon-agent](pokemon-agent.md)); what is worth carrying here is that no test in this file
  could see it — `a_chain_does_not_advance_on_an_ending_the_agent_never_reported` pins what happens
  *given* silence and cannot say which endings are silent. C3's coverage oracle found it by scoring
  ids nobody had a verdict for. ⚠️ **A driver that reports nothing still exists** — the Surf mount,
  and a boulder push deliberately — so the same hole can be dug again.
- `set_nickname` is checked by `tools::unencodable`, which round-trips through the charmap: an
  unknown character becomes `0x00`, a control byte, not a failure.

## The battle script

- Every run starts on `battle_script::DEFAULT` (`battle.ask()`). `BattleScript::armed()` is false
  for it, `live_source` never sends it to `Live`, and `ScriptState::Unedited` is computed from
  `(armed, is_default)`. `set_battle_script` with no `script` returns to the default; `purpose` is
  enforced when a script is given. `is_default` is on the wire because a default and an unarmed
  written script are byte-identical.
- `battle_script::run` is called from `pick_battle_action` on the emulator thread, before
  `advance`, and returns early on a turn already in flight. The script filters
  `policy::battle_options` and never invents an action; `facts` (`can_run`, `usable`, `best_move`)
  is computed from that same options list, so in a ghost battle `best_move` is unset and every
  move's `damage` and `effectiveness` is 0, bench included, with `battle.ghost` saying why.
- `engine` is one builder for live runs and validation (`MAX_OPERATIONS`, `MAX_RUNTIME` 50 ms,
  size caps, `eval` disabled, never rhai's `unchecked`), under `catch_unwind`. `switch_to` and
  `move_type` are named around rhai reserved words;
  `every_name_the_docs_use_is_one_the_parser_accepts`. The choice cell keeps the first choice even
  when the abort is caught.
- Validation runs the `SCENARIOS` table, all at turn 1 (the ghost is the one that sets a map), and
  is not a proof; the disarm is. One strike disarms for the run and keeps the source. The policy
  disarms through `Live` and never writes the file; `run_one` drains the failure and the decided
  tally at the top of the next turn. `Live` is armed from the file at construction and reopened in
  `apply_restart`.
- The three script tools are on Overworld only (the measurement is on `offers_battle_script` in
  `tools.rs`), and every script state is said on the overworld turn
  (`prompt::overworld_script_line`), because that is the turn carrying the tools that change it.
  `ScriptStanding` carries `purpose`, the decided count and the failure (`MAX_FAILURE`, cut with an
  ellipsis, withheld while armed).
- `Worker::publish_battle_script` publishes on change with `published_script` seeded `None`, so the
  default is announced once; `Published::latest_battle_script` feeds late pages. Safari battles are
  never scripted.

## The battle report

Damage is a diff of HP between consecutive decisions and the prose is the cartridge's, because
there is no per-turn outcome event. A turn closes at the next decision (`finish` takes a final
state; `close` compares the side's name as well as its HP). The report is rendered into the
situation, not appended as a message. `events_mark` takes back the message boxes it already
narrates, and `reports` is a queue cleared by the turn that carried it. A blackout is detected by
`is_blackout` on the cartridge's own sentence (`wBattleResult` is zeroed before anything here runs),
quoted with `QUOTE_TAIL` so the ending survives, and its arm sits above the in-battle arm.

`handed_back` returns the turns the script took since the model last chose, rendered by the same
`turns_from` the finished report uses, and `told` is what stops them being sent twice. Without it a
model asked mid-battle sees a fight in which its own last decision was silently replaced.

## Taking a battle over

- `choose_battle_action`'s `take_over` stops the script deciding **the rest of this battle** and
  nothing more. `LlmPolicy::taken_over` is set only where the action resolved, is cleared by
  `AgentEvent::BattleEnded` and by `restart`, and is checked after the report is opened so a
  taken-over turn is counted through `handed_back` exactly as a `battle.ask()` turn is.
- ⚠️ **It cannot be scoped to the run, and the tool's ⚠️ has the measurement**: a disarm reached for
  mid-battle is one nothing brings back. `set_battle_script` on the overworld turn stays the way to
  turn a script off for good.
- The offer rides on the ask note only when the script has actually taken turns in between
  (`llm_policy.rs`'s `ScriptOutcome::Ask` arm), for `script_standing_line`'s reason: a sentence on
  every ask is one a model reads past. `DecisionKind::Battle`'s tool ceiling moved 4950 → 5250 for
  the 289 bytes.

## The plan and the prompt

- `MAX_ITEMS` is 5 and done items count. `add` evicts the oldest done item and refuses when none
  are; `open` trims on the way in. The list is rendered in the model's order, ticked in place.
  `PLAN_REFRESH_TURNS` (10) re-appends an unchanged plan on overworld turns only, and
  `PLAN_UNCHANGED` rides on every other turn. A compaction may drop the plan; `sync_plan`
  re-renders it from `todo.json`, and the summary deliberately does not quote it.
- The system prompt has to keep saying: the game is not broken, retrying is not a plan, prior
  knowledge of Red is not evidence, what people say is the instruction, and how to play well.
  `the_system_prompt_says_the_things_the_deployed_runs_needed_it_to_say` and
  `the_system_prompt_says_how_to_play_the_game_well` pin the wording. The guide bullet states a
  cadence, since `guide::chapter` is keyed on badges. `prompt::ailment` prints nothing for a healthy
  Pokémon; `strum`'s `Display` printed `None`.

## The wire and the endpoint

- `Fragment::Reasoning` is shown live and never sent back. A thought is closed by the next thing
  the model says (`MODEL_SIDE` in `useEventStream`), not by the turn ending or by agent events.
- `GB_MAX_TOKENS` (8192) bounds a reasoning loop, and `prompt::truncated_nudge` differs from the
  no-tool-call nudge. `GB_REASONING_EFFORT` passes through unvalidated.
- `LlmError::Timeout` is not retryable (the endpoint is still working on the request);
  `Transport` is. An error inside a 200 (`ApiError::into_failure`) maps onto the same table as a
  non-200, recognised by OpenRouter's own frame shape and nothing else, and `ErrorCode` takes an
  integer or a string.
- A dated 429 parks: `Worker::park_until`, `RunStatus::Throttled`, clamped to `MAX_PARK` (25 h),
  release unconditional, the same request re-sent because the world did not move.
  `EmulatorHost::tick`'s pause seam sits below the reset seam, skips the emulator and the agent
  only, and zeroes `since_last_update`. `X-RateLimit-Reset`'s unit is sniffed from magnitude
  (`reset_at_ms`); an undated 429 keeps the ordinary backoff. `paused_total` comes off `wall_ms`
  on both the heartbeat and `progress()`.
- `GB_COMPACT_ABOVE` (0.85) is refused outside 0.2–0.95. The headroom is absolute: a turn grows
  unchecked to `GB_MAX_TOOL_STEPS` between compactions. `image_tokens` prices a map from its
  dimensions. A local endpoint's limit is its KV cache per slot, not its window; run one slot with
  the whole window.

## A turn that failed outright

⛔ The 402 death loop of 2026-09-05, and the three faults it needed at once. `docs/coverage-plan.md`
§2.2.1 of the 2026-09-06 draft (git `7343616`) is the evidence; `worker::TurnOpen` carries the argument.

- **A failed turn is rolled back whole.** `Worker::run_one` records `history.len()` **and**
  `turns_since_plan` before `sync_plan` runs, and `roll_back_failed_turn` puts both back when the
  turn produced no completion at all. To a *length*, not one `pop`: a turn can fail on its second
  tool step with an assistant message and its results already appended. `History::rollback_to` is
  the only shortening that is not a compaction, and it is safe because the log is flushed at the end
  of a turn and this happens in the middle of one.
- **`turns_since_plan` is restored too, and that half is not bookkeeping.** It is what makes the
  plan refresh fire every ten *completed* overworld turns rather than every ten attempts. Without
  it, a run whose every request failed appended a fresh plan message every `PLAN_REFRESH_TURNS` and
  nothing ever removed one: 1373 copies in the deployed file.
- **A compaction that can drop nothing says so.** `trim_history` cuts only at turn boundaries and
  `compaction::is_turn_start` deliberately does not count a plan message, so a history of unanswered
  plans had no boundary anywhere in it — it dropped nothing and published `before == after` as a
  success. `Worker::drop_unanswered` is the pass below it: a `user` message immediately followed by
  another `user` message was never answered, so nothing depends on it and it cannot orphan a tool
  result. The tail (`KEEP_MESSAGES`) is left alone. A compaction that still reclaims nothing raises
  an `error` notice, because every request from there on is over the window and only an operator can
  act on that.
- ⚠️ **An undated hard failure is still not parked**, and that is the decision rather than an
  omission: parking stops the bleeding and does not touch the ratchet, and any endpoint can produce
  one. `llm::an_undated_hard_failure_does_not_ratchet_the_history` is the guard, and it fails
  against the old code with `[3, 3, …, 4, 4, …, 5, 5, 5]`.
- Every fault the endpoint can produce now has a test on the e2e harness — see
  [test-suite](test-suite.md)'s *The LLM end-to-end harness*.

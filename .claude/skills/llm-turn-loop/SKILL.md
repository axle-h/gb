---
name: llm-turn-loop
description: "The LLM side of a turn: the append-only history and the prompt cache, the tool catalogue and its budget, the action menu and its ids, the plan, the system prompt, the SSE wire and its error frames, the park on a spent quota, and compaction. Load before touching anything under src/llm/ or src/pokemon/llm_policy.rs, or before changing what a model is sent."
---

# The model's side: the turn loop, the tools and the prompt

*Extracted from `CLAUDE.md`, which holds the rules of the road and the index of these skills. The
README is imported into `CLAUDE.md` and is not repeated there or here: this file has only the
invariants and the traps, nearly every one of which was learned by breaking something.*

## The model's side

The README tells the *story* of `press_buttons`, `report_issue`, the nickname screen, the HM gating and the plan.
What follows is only what a change to them has to not break, and the measurements the arguments rest on.

### The history is append-only

⚠️ **Message 0 is a constant, and it has to stay one.** A prompt cache is keyed on the *prefix*, so anything dynamic
in the system message throws away the cached prefill of the **entire** conversation the next time it changes. The
plan used to live there, re-rendered every request, so every `todo_add` paid that. It is now `prompt::plan_message`,
a `user` message of its own, emitted by `Worker::sync_plan` only when it differs from the newest copy in the history:
unchanged → nothing happens; changed → a fresh copy is **appended** and the stale one left where it was; absent (a
compaction took it) → appended, which is what makes the whole thing self-healing.

⚠️ **So after the system prompt this history is append-only, with no exceptions**, and "the plan" means the last
`is_plan` message rather than the only one (`rposition`). ⚠️ It is deliberately **not** a turn boundary
(`compaction::is_turn_start` excludes it), or a cut taken between the plan and the situation it belongs to would drop
the one thing meant to survive. `the_plan_is_appended_and_never_disturbs_the_cacheable_prefix` holds both halves.

⚠️ **Leaving the stale copy is not laziness, it is the cache, and the two are within ~20%.** Worth writing down,
because "one copy" sounds obviously right. The plan is 1283 bytes (~320 tokens, `probe_turn_requests`) and sits
immediately before the *previous* turn's situation, so a removal re-prefills exactly one turn (~1250 tokens); leaving
it costs its 320 *cached* tokens — a tenth the price — on ~32 requests, half a compaction cycle. ~1250 against ~1020.
⚠️ **The tie is broken on structure, not on the 20%**: appending has no exceptions to get wrong, and the removal's
advantage depends on the endpoint's cached-token discount, which is not ours to control — at 2× rather than 10× the
removal wins outright.

⚠️ **A tool call the model writes badly poisons the conversation, not the turn, and a router made that a daily
event.** `arguments` is a JSON string the *model* produces, and the assistant message carrying it is replayed on every
request for the rest of the run — so one completion emitting `""` or a fragment cut off mid-object is rejected for ever
after by any endpoint strict enough to parse it (`tool_calls[].function.arguments must be a JSON object`,
`Expecting ',' delimiter: line 1 column 22 (char 21)`, `Failed to apply prompt template: cannot convert value into
pairs` — 331 in one backlog against `openrouter/free`). ⚠️ **The tell is that the parse error is character-identical
every time**: it is one stored message being re-sent. It never happened against a single local model, because a router
hands the conversation to a different one each request and it takes one. `protocol::history_safe` rewrites anything
that is not a JSON object to `{}` inside `Message::assistant`, the one funnel every history entry goes through. ⚠️ **In
place, never dropped** — removing the call would orphan its `tool_result` and break the invariant one-step rollback
rests on — and ⚠️ **only the broken ones**, since `serde_json` sorts keys and canonicalising every call would reword the
model's own history for nothing.

### The tool catalogue

⚠️ **Every terminal tool takes a required `summary`, enforced, and it is the only thing the model says about a turn
that outlives the turn.** It rides on the terminal call's own arguments because that is the one place a sentence costs
no extra round trip and cannot be separated from the decision it explains. ⚠️ **It used to be required in the schema and
optional in the parser**, on the argument that a rejected call spends one of `GB_MAX_TOOL_STEPS` and pushes a forgetful
model towards the forced `wait`. That was settled by measuring: across the deployed run's **2427 decisions only 98
carried no summary and every one was a `wait`** — the *synthesised* fallback wait, which never goes through `classify`.
`classify` now rejects a terminal call without one, in one wrapper rather than a check per arm. ⚠️ It is added by
`add_summary_argument` post-hoc in `for_kind`, so it scales with the *number of terminals a kind offers* rather than
with the catalogue — which is what moved `the_tool_array_stays_within_its_budget`'s ceilings, deliberately. It reaches
the page as `Decision.narration`.

⚠️ **`press_buttons` is offered on exactly one decision kind — `Stuck`.** The evidence, since the README gives the
conclusion: **749 presses, 738 on overworld turns with a perfectly good menu, 0 at the watchdog's turn**, ending in
**91 consecutive** presses in a 13-tile box on Route 3 while `Route3:0,10:Connection — walk into PewterCity` sat in the
menu every time; the last `choose_action` before that run succeeded, so ⚠️ **nothing had failed *there***. The ratchet
— 26% → 2% → 12% → 74% → 38% → 72% → 100%, recovering to 2% for 600 turns in the middle — is the tell that it is
self-reinforcing rather than caused. ⚠️ **The stronger claim this used to make, that `choose_action` was never once
rejected in the whole run, is false**: **59 of 934 named a map the player was not standing on**. It does not change the
conclusion, since none sat near the Route 3 run, but it was quoted as evidence and is not evidence. Of the two frictions
tried, prose ("a last resort") cannot be checked afterwards, and a `why` ⚠️ **required in the schema and optional in the
parser** left **543 of 749 presses null** for 239 bytes. `why` survives at `Stuck` and **is now enforced**; dropping the
tool from Overworld and Battle took each down ~450 bytes. ⚠️ **A press at `Stuck` is not a fault** — there is no menu,
so it is the model doing as it was asked. `incident.json`'s `kind` records which turn asked and `report` says `press` or
`issue`, so the two are counted apart without inferring it from which fields are null.

⚠️ **`report_issue` does not end the turn, and that is the whole design** — a *terminal* replacement would be the same
tool renamed. Three ⚠️s: its `message` is **enforced** (a report is *only* its message, so rejecting an empty one loses
nothing — the test the two enforced fields pass and `summary`'s old argument did not); **the answer must not read like a
fix** (`Worker::file_issue` says filed, nobody is coming, this did not end your turn, try a different way, or the model
waits for the change and produces an identical turn); and it is offered on `Overworld`, `Battle` *and* `Stuck`
(`tools::offers_issue_report`) — the watchdog's turn is where the agent is most likely to be genuinely wrong, so it is
the last place to withhold it. The three single-question prompts carry neither tool.

Records land in `$GB_RUN_DIR/<run-id>/issues/turn-<id>/` beside `press-buttons/`:
`{incident.json, screen.png, state.gbst}` plus the conversation slice. ⚠️ **That slice is image-evicted, and that is not
an optimisation** — three turns, cut on `is_turn_start`, through `compaction::evict_images(.., 0)`; a history holding a
map render is hundreds of kilobytes of base64 per message. ⚠️ **Nothing new is published and no picture rides on an
event**, and the path is re-read from `run::CurrentRun` per record, never captured, or a press after `POST /api/new-run`
lands in the run that was already set aside.

⚠️ **The save state is taken at the *start of the turn*, by the emulator thread, and left in `Published`** —
`EmulatorHost::tick` on the edge into `RunStatus::AwaitingLlm`, never on the level, or it is 50 states a second for the
length of every turn. It has to be there because `GameBoy` exists on that thread. ⚠️ **The obvious cheap version — copy
the run's `state.gbst` — is wrong**: that is the last periodic checkpoint, up to a minute behind, which is a minute of
walking or the very transition being complained about. A state is **24 µs and 6.4 KB**, so one per turn is cheaper than
the copy it replaced; ⚠️ still not free enough to do every tick, where `MAX_CATCHUP` turns it into dropped emulated time.

### What the turn already answers

⚠️ **There is one notes mechanism and there used to be two.** `memory_write`/`memory_read` over a `memories/` directory
sat beside `todo_add`/`todo_complete` doing the same job in a different shape: four tools' worth of schema in every
request and a choice for the model to get wrong. The plan won — it renders on the page, for both audiences — and the
freeform role it gave up is filled better by the compaction summary. What only the plan does is survive a *process*
restart, since the history is never persisted; that is why `MAX_TEXT` is long enough for an item to carry its own reason.
`run::files::MEMORIES` survives only so an archive of an older run is still complete.

⚠️ **A read the situation already answered is worse than no read at all** — a round trip bought for nothing, and it
teaches the model that a turn opens by reading. `read_screen_text` answered from the same `observe::screen_text` the turn
renders under `### On screen`; `read_trainer` returned badges, money and play time, all in the turn's header (its two
genuinely absent figures, the dex counts, moved there). Same trap one level down: `MapView` carried `actions` and
`BattleView` carried `options`, second copies of the turn's own menus **without the ids** — every one of which would be
rejected.

⚠️ **Reads are scoped per decision kind, not just terminals.** Every kind used to be offered all eight: a battle turn
paid for `read_map`, a naming screen carried the whole catalogue to answer with one word. Beyond the tokens, a tool that
can only ever answer `null` is an invitation to spend a round trip finding that out. `ReadTool::kinds` is the table;
`non_terminal_names` is therefore per-kind too, or the contract at the bottom of a turn would name a tool the request did
not carry. ⚠️ A read that exists but is not offered *here* is rejected by name with the reason — falling through to
"there is no tool called `read_map`" is a lie the model cannot act on.

⚠️ **`read_route` runs the search; `read_world_graph` shipped the graph.** The old tool serialised every visited
`(map, entry)` node with all its edges — unbounded by construction. Nothing ever wanted the adjacency list; the question
is always "which way is Celadon". ⚠️ Its `None` is **negative** and the wording has to keep saying so: no route means
"you have not walked there yet", never "unreachable".

### The action menu

⚠️ **A menu row says what choosing it does, in words, and carries nothing else.** The row is
`` `{map}:{x},{y}:{kind}` — {verb phrase} ``: `take the warp to PalletTown`, `walk into Route1`, `talk to Mom`,
`pick up the Potion`. It has been two other things. First `` `OaksLab:5,11:Warp` — Warp → PalletTown (12, 11) — 10
steps ``, where the verb is the `kind` said twice and the coordinate is on a map the model cannot see; then,
over-corrected, a person's row was the bare distance and the deployed model took `MtMoonB2F:15,23:Rocket2` for a warp
forty-five times. So a person's name is said once more, after the verb. ⚠️ **The verb comes from the sprite's
`PictureId`, never its name**: everything the player can face is a sprite to the game — items, boulders, fossils, the
lab's Pokédex, Snorlax — and only the picture tells `Potion1` from `Hiker`. The step count is gone for good; nothing used
it. ⚠️ **The `{map}` prefix is not redundancy**: `resolve_overworld` re-mints ids against the map the player is on *now*
and an answer can land after a warp, so without it `5,6:Warp` chosen in Oak's lab could match a warp at (5, 6) in Pallet
Town and be carried out silently. `a_menu_row_explains_the_action_in_words` and `a_sprite_row_is_verbed_by_its_picture`.

⚠️ **An id that is not in the turn's own menu is refused by `tools::classify`, inside the turn**, before it can be a
decision. `TurnRequest` carries the menu's ids for exactly this (`tools::not_on_the_menu`), and the plumbing is worth it
because of where the alternative lands: an invented id used to be classified as a `Terminal`, published as a `Decision`,
sent to the policy and only then refused by `resolve_overworld` — at which point the turn is over and the complaint rides
on the *next* turn, a second full prefill of a ~55 k-token history. Caught in `classify` it is an ordinary
`CallKind::Rejected` and the model still acts in the same turn. Measured: **59 of 934 `choose_action` decisions named a
map the player was not on** (`ViridianCity:33,8:Warp` while standing in `ViridianPokecenter`), because the model was
quoting a menu from several turns earlier. Three traps:

- ⚠️ **An empty menu checks nothing.** `Nickname`, `ForgetMove` and `Stuck` carry no menu, and reading "no menu" as
  "nothing is allowed" would reject every answer they give.
- ⚠️ **`resolve_overworld` is still the authority and must stay.** This catches what the model was never *offered*; that
  one catches what stopped being true in between.
- ⚠️ **The complaint has to name the right mistake.** The policy's note used to say "the game moved on while you were
  deciding" for every failure, which for a stale id invites the model to try again — the world had not moved, the model
  had. Both messages compare the id's map prefix against the map the player is on, and read the current map out of the
  menu's own first id rather than being handed it separately, so the two cannot disagree about where the player is.

⚠️ **An action the game will refuse is not an action, and offering one wedges the run rather than wasting a turn.**
pokered answers a missing badge with `.newBadgeRequired` → `jp .loop` — the same party menu, cursor untouched — and the
agent's driver is mashing A by then with no exit but "we came back to the overworld", so it ends sixty seconds later at
`DRIVER_ESCAPE_SILENCE`. The deployed run did it **eleven times** on Route 2 **with no badges at all**. Three gates,
different mechanisms on purpose:

- **`MetaTileMap::can_cut`**, set by `game_state()` beside `can_surf`, keeps `:CutTree` rows out of `actions()` entirely.
  ⚠️ **On the map rather than in `overworld_menu`**, so the scripted policy is held to it too. The same line stops
  `actions()` offering a `ConnectionWater` crossing to a party that cannot Surf: the BFS records the shore tile as a
  reachable *terminal*, so that row was a bump into the sea.
- **`tools::hm_available`**, in `resolve_field_move`, refuses the call against `HM_BADGES` (transcribed from
  `.outOfBattleMovePointers`). ⚠️ **The two halves are separate complaints** ("nobody knows it" vs "you have not won the
  badge") because they need different things done about them. ⚠️ `PushBoulder` additionally requires `strength_active`:
  Strength is *armed* once per map and cleared on every map change, and a push before that moves nothing and reports
  nothing.
- **`agent.rs`'s own check on the way into `CuttingTree`**, the one every request passes through whatever asked for it —
  the two above are two places to forget. It refuses with a `TextBox` and drops to `Idle`. ⚠️
  `a_cut_with_no_cut_never_opens_the_party_menu` asserts that **decisions keep coming**, not that the state was skipped:
  a guard that left the agent with nothing to do would pass "never entered `CuttingTree`" and still be the wedged run it
  replaces.

⚠️ **`fly_bike::blocked_by` was already doing this and still had the hole**: it checks the destination, the tileset and
that somebody knows Fly, and never checks the Thunder Badge.

⚠️ **Withholding a row silently is the same bug facing the other way**, which is why `prompt` gained a `Blocked here:`
line. It fires only while the map holds a `CutTree` or `Water` the party cannot get past, and names what would clear it.
Without it a model finds no way north out of Route 2, no reason why, and goes round the same four maps for forty turns.

⚠️ **The naming screen used to talk the model out of the only thing it is for** — `set_nickname` said the default "is the
ordinary answer", and across both deployed runs **all four** naming screens took it. ⚠️ **The name is written straight
into the naming screen's buffer, so nothing else checks it**: `PokemonString::from_string` maps an unknown character to
`0x00`, which is not the terminator (`0x50`) but a control byte — so `Poké` does not fail, it names the mon something
unreadable for the rest of the run. `tools::unencodable` rejects it by round-tripping through the charmap rather than by
asking whether it is alphanumeric, since `/` is a perfectly writable `$F3`.

### The plan

⚠️ **It holds five items (`MAX_ITEMS`) and finished ones count towards that; it used to hold 32 and they did not have
to.** At 32 the cap never bound and the model never deleted anything: the deployed run reached **13 items of which 11
were done**, with the two live ones at the bottom. ⚠️ **Counting done items against the cap is the whole of the fix** — a
cap on open items only moves the growth into the tail, which is exactly where it went. `add` evicts the oldest *done*
item and refuses outright when none are done, which is the message that teaches the rule rather than a silent truncation.
⚠️ **Finished items go first and oldest first, never live work.** ⚠️ **`TodoList::open` trims on the way in as well**, or
a run resumed across the change keeps its long list for ever — `add` only makes room when it needs some.

⚠️ **The list is rendered in the model's own order, ticked items in place.** `render` used to partition into
open-then-done and `PlanPanel.tsx` independently did the same (`[...open, ...done]`), so an item completed in the middle
jumped to the bottom and the numbering the model had written stopped matching what it read back. A plan is a sequence and
silently re-sorting one is a way to make it say something its author did not.
`the_list_is_rendered_in_the_order_the_model_maintains` is the guard, and ⚠️ **`SHOW_DONE` is gone with it** — hiding a
tail of finished items was a workaround for a cap that did not bind.

⚠️ **The rule is stated in three places and that is a split, not duplication.** The tool description is the only one on
*every* request, so it states the rule tersely; the plan message argues it; the system prompt frames it. That split is
what kept it affordable — the first draft put the argument in the tool description and
`the_tool_array_stays_within_its_budget` went red on **Overworld at 9522 against 9500**. No ceiling has ever moved for
this.

⚠️ **Emit-on-change keeps the prefix cache and buries the plan, and both deployed runs proved it.** A model that sets one
item on turn 1 and never touches it again (258 turns, one `todo_set`; 2430 turns, sixteen and a single `todo_complete`)
has the list it is meant to be revising as the *least* recent thing in every request. Three parts, three mechanisms:

- **`PLAN_REFRESH_TURNS` (10)** bounds the drift: an unchanged plan gets a fresh copy appended every tenth overworld
  turn. ⚠️ **The number is set by history growth, not by cache cost** — a refresh is one ~150-token message prefilled
  once and cached from then on, against a turn of one to two thousand, so every tenth turn is ~1% more context to carry
  and to compact and every turn would be ~10%. `a_plan_nobody_edits_is_brought_back_to_the_tail_of_the_history` asserts
  both halves, because a refresh that fired every turn would pass a test that only checked it comes back.
- ⚠️ **The refresh is gated to `DecisionKind::Overworld` and the edit is not.** There is nothing to be done about a list
  mid-battle, on a naming screen or at a mart; an *edit* has to land on any kind, because `non_terminal_names` chains the
  todo tools onto all of them unconditionally.
  `a_battle_turn_never_pays_to_reposition_a_plan_it_cannot_act_on` holds the pair apart.
- **`prompt::PLAN_UNCHANGED`** is appended to the situation on every turn that does *not* carry the plan. ⚠️ **A message
  the model can still see is not a message the model is still reading.** It costs nothing at the cache, because the
  situation is fresh tokens either way, and it says "unchanged" rather than restating the items.

⚠️ **A compaction can take the plan, and nothing else needs to.** `is_turn_start` refuses to cut between a plan and its
turn, so it is only dropped *with* that turn; `sync_plan`'s "there is no copy" arm then appends a fresh one re-rendered
from `todo.json`, which is the authority and cannot come back stale. ⚠️ **Which is why the summary deliberately does not
quote the plan**: a summary is written once and never rewritten, so a plan inside one is frozen at the moment of the
compaction and sits at message 1 contradicting the live copy for the rest of the run.
`a_compaction_that_drops_the_plan_does_not_break_the_chain` asserts the precondition — that the history was long enough
for the plan to be inside the dropped middle — or it would prove nothing. (The system prompt is never compacted:
`apply_summary` lifts `messages.first()` out when it is a `Role::System` and rebuilds around it.)

### The system prompt

⚠️ **Four things it has to keep saying, each bought with a measured failure.** They are prose, so nothing but
`the_system_prompt_says_the_things_the_deployed_runs_needed_it_to_say` notices them going — and all four *have* been
reworded away at some point:

- **The game is not broken and you are not debugging it.** 29 of one 258-turn run's own decision summaries called it
  buggy, glitched, broken or in need of a reset.
- **Retrying is not a plan.** Eleven cuts at the same tree; 91 consecutive `press_buttons` on Route 3. A model reads its
  own last turns back on every request, so the second attempt makes the tenth likely.
- **Prior knowledge of Pokémon Red is not evidence.** That run named Brock **88 times** without having met him or held a
  badge, and chose its starter on turn **7** for the "type advantage over Gary's likely Charmander". The one before it
  said "Pewter" 573 times and "HM01" 100.
- **What people say is the instruction.** It read `GARY: Yo AI! Gramps isn't around!` six times in Oak's lab and spent
  thirty turns re-talking to the same three people instead of going to look for Oak.

⚠️ **Those four say what not to do, and a run can obey all four and still play badly.** The 2026-08-26 run reached Mt
Moon on **92 minutes of cartridge time with one Lv19 starter as its whole party**: of 204 battle decisions **31 were
`run` and none was a Poké Ball**, and it bought nothing in a mart across 429 decisions. (It did heal, six times, and it
read the guide three times, so neither of those is the gap.) None of it is a malfunction, so nothing but prose can fix
it — hence the second section, "Playing it well, and the clock you are playing against", and
`the_system_prompt_says_how_to_play_the_game_well` beside the first test. ⚠️ **The blackout bullet is a cartridge fact
rather than advice, and the distinction is the whole of it**: `SetLastBlackoutMap` is called from
`DisplayPokemonCenterDialogue_` **only after the player answers yes to the heal**, so walking into a Centre does not
move where a blackout sends you, and `ResetStatusAndHalveMoneyOnBlackout` is where half the money goes. ⚠️ **The clock
the prompt says it is timed on is the cartridge's** — `wPlayTime`, in every turn's header and what the hall-of-fame
ledger ranks on — which is why asking for a thorough run *and* a fast one is not a contradiction, and why a park costs
it nothing. ⚠️ **The guide bullet states a cadence rather than encouraging the tool**: `guide::chapter` is keyed on
the badges alone, so every read before the next badge returns a word-for-word copy, and the only other moment worth
spending one on is *after a compaction*, which is when the chapter the model read is gone and the plan is all that is
left of it. "Read it whenever you are unsure" is therefore the wrong shape, and was the first draft.

⚠️ **`PokemonStatus`' `Display` is `strum`'s derive, so a healthy Pokémon prints `None`** — every party line in every turn
read `20/20 HP, None`, which is a missing value rather than good news. `prompt::ailment` says nothing when there is
nothing to say. Same class of bug as `MetaTile`'s old `strum` `Display`: a derive is a debugging default, and every one of
these strings is prose a model reads.

### The wire

⚠️ **A reasoning model streams its thinking on a channel of its own, and it is not `content`.** LM Studio, vLLM and
DeepSeek send `reasoning_content`; OpenRouter sends `reasoning`; OpenAI sends neither. Before `MessageDelta` knew the
field, serde dropped it as an unknown key — the page showed a blank turn for however long the model thought, and on a
local 12B that is **three quarters of the completion tokens of a trivial overworld step**. `read_stream` reports a
`Fragment::Content` or a `Fragment::Reasoning`, two channels rather than one string because they have opposite fates:
⚠️ **the reply goes back into the history and the thinking never does.** `Usage::estimate` counts it anyway — the endpoint
charged for it.

⚠️ **A thought is closed by the next thing the *model* says — not by the turn ending, and not by the next event of any
kind.** Both wider rules were tried and are wrong in different directions. On the turn: a turn that reads before it
decides thinks once per completion, so grouping on `turn` welds two thoughts around the tool call between them. On the
next event: **the emulator never pauses while the model thinks**, so the agent narrates over the top of every thought
("→ heading for Mom") and a fold that closed on those shredded a one-minute thought into five rows.
`useEventStream`'s `MODEL_SIDE` is the line, `lastModelSide` finds the row, and ⚠️ **`Conversation.tsx` must read
liveness the same way** — one decides what the row contains and the other how it is drawn.

⚠️ **The live thought scrolls in a box of its own, and pinning the log does not pin it.** It is capped at a few lines so
it cannot bury the log, which means arriving tokens land *below* the visible part: measured mid-thought at 222px of text
in a 117px box with `scrollTop` 0. The `.body` element is followed separately.

⚠️ **An uncapped completion is bounded only by the context window, and that is not a bound.** A reasoning model in a
repetition loop generates until the window fills: measured at **~26 000 tokens** on turns that normally cost 24–2 000,
twice in twenty-five minutes, each holding a single-slot endpoint for the full ten minutes our deadline allowed.
`GB_MAX_TOKENS` (8192) is the ceiling and `0` removes it. ⚠️ **A truncated reply is nudged differently from a silent one**
(`prompt::truncated_nudge`): told only "that reply contained no tool call", a model cut off mid-thought concludes it
forgot to call one and tries again at the same length, into the same ceiling.

⚠️ **`reasoning_effort` is an on/off switch on LM Studio, not a dial — measured rather than assumed.** With gemma-4:
`none` takes reasoning to *exactly zero* tokens while still answering correctly, `low` is indistinguishable from the
default (174 → 159 tokens), and `chat_template_kwargs` in either spelling (`thinking`, `enable_thinking`) is accepted and
silently ignored. `GB_REASONING_EFFORT` passes the string through unvalidated, because the vocabulary belongs to the
endpoint and refusing a value it would have taken is worse than forwarding one it rejects in a 400 whose body we keep.

⚠️ **Giving up on a request is not free, and a timeout is not a transport failure.** A connection that never opened
consumed nothing at the far end, so retrying it is free and correct — that is `LlmError::Transport`. A request the
endpoint *accepted* is being worked on, and llama.cpp says so when we hang up ("Stopping generation… (If the model is
busy processing the prompt, it will finish first.)"). So `LlmError::Timeout` is a separate variant and is **not
retryable**: on a server that runs one request at a time, the retry queues behind the very request it replaces and can
never be faster. `GB_REQUEST_TIMEOUT_SECS` (180) wants to be *raised* for a local endpoint rather than lowered — waiting
costs a stalled turn, giving up early costs the same turn plus the endpoint's next few minutes.

⚠️ **A failure can arrive *inside* a 200, carrying the status the retry table wants.** A router that cannot reach an
upstream has already sent its headers, so it says so in an ordinary `data:` frame:
`{"provider":"Nvidia","choices":[],"error":{"code":504,"message":"Provider timed out after 47709ms",
"metadata":{"error_type":"timeout"}}}`. Two faults, one on top of the other. ⚠️ **`code` is an integer on OpenRouter and
a string on OpenAI** (`"insufficient_quota"`), and typing it as one made the other unparseable — so the run reported *its
own parser* as malformed for what was a provider timing out. And ⚠️ **the status was then flattened to
`LlmError::Protocol`, which is not retryable**, so the textbook transient failure the backoff exists for was the one thing
it never saw. `ErrorCode` takes both spellings and `ApiError::into_failure` maps the status onto the same table a non-200
goes through (429 → an *undated* `RateLimited`; 5xx/408 retryable; other 4xx fatal). ⚠️ **A 504 here is not an
`LlmError::Timeout`, despite the word** — that variant is *our* deadline expiring on a request the far end is still
working; here the router has already given up, so another attempt is ordinary. ⚠️ **And the classification is scoped to
OpenRouter by the frame recognising itself** — a chunk-level `provider` or an `error.metadata`, nothing else sends either
— rather than by sniffing `OPENAI_BASE_URL`, which is any OpenAI-compatible endpoint. A bare `{"error": {…}}` keeps the
old `Protocol` exactly, so no other provider's failures start being retried on the strength of a number we decided to
trust. `a_bare_error_frame_is_still_only_a_protocol_error` guards that half.

### When the endpoint says no: the park

⚠️ **A rate limit is the one failure where the retry is itself the problem.** Every attempt is another request against the
very quota that is exhausted, then the turn resolves to `FAILURE_WAIT_TICKS` (2 s of game time) and the next decision
point asks again — on OpenRouter's free tier (50 requests/day) that burns the whole day in under two minutes and then
hammers the endpoint for ever. `LlmError::RateLimited` is the separate variant, on the same argument that made `Timeout`
one. What the endpoint hands back instead is a *time*, and the only thing that works is waiting for it:

- ⚠️ **The client must read the headers before the body**, because reading the body consumes the response. `Retry-After`
  and `X-RateLimit-Reset` are the whole of what makes a 429 actionable.
- ⚠️ **`X-RateLimit-Reset` has no agreed unit**, and `protocol::reset_at_ms` sniffs it from the magnitude: OpenRouter
  sends Unix **milliseconds**, several OpenAI-compatible servers send Unix **seconds**, others send **seconds from now**.
  Both misreadings are silent and bad in opposite directions — a Unix-second stamp read as a delta parks the run for
  thirty years, a delta read as a stamp resumes instantly into the same 429. ⚠️ And `None` means "the endpoint did not
  say", which is *not* a reason to park: an undated 429 is far more often a per-minute limit, so it keeps the ordinary
  backoff. `stream_with_retries` only declines to retry when the reset is further out than `policy.max`.
- **`Worker::park_until` waits it out**, publishing `RunStatus::Throttled { until_ms }` and stopping the emulator with
  `Published::set_throttled_until`. ⚠️ **The release is unconditional** — a return that leaves the cell set stops the game
  for the rest of the process. ⚠️ It is **clamped** (`MAX_PARK`, 25 h) because the number came from the endpoint, and ⚠️
  the turn then re-sends **the same request**, which is only sound because the emulator was stopped: the situation it
  describes is still on screen when we wake.
- **`EmulatorHost::tick`'s pause seam** is where the game actually stops. ⚠️ **Below the reset and completed-run seams**,
  so a parked run still answers `POST /api/new-run` — that is how a park is escaped, and `start_new_run` releases the cell
  itself rather than depending on the parked thread. ⚠️ **It skips the emulator and the agent only**: the heartbeat, the
  video and the checkpoint still run, because a paused run that published nothing is *indistinguishable from a dead
  connection* (`STALE_MS`), and since send-on-change is silent while the screen is frozen the 2 s keepalive is what feeds
  the page. ⚠️ **No catch-up debt**: `since_last_update` is zeroed, or the game fast-forwards through `MAX_CATCHUP` the
  moment the quota reopens. `a_parked_run_stops_the_game_but_keeps_the_page_fed` holds all of it.
- **The cartridge clock needs no help and the wall clock does.** `wPlayTime` is emulated, so stopping the emulator stops
  it — which is the whole reason the pause beats merely holding the requests back, since the leaderboard ranks on it.
  `paused_total` is subtracted from `wall_ms`.
- **On the page** the last frame is dimmed under a PAUSED plate with a live countdown (`Screen.tsx`'s `PausedOverlay`).
  ⚠️ **The deadline is published once and the countdown is derived on the client**, so an hours-long park costs no
  traffic; that is also why `until_ms` is an absolute Unix millisecond, since it is replayed on every heartbeat and to
  every page that joins later. ⚠️ The plate sets its own `line-height`: `.screen` sets `line-height: 0` for the canvas,
  which collapses a stack of spans into one overlapping line.

### Context

**`GB_COMPACT_ABOVE` (0.85) is what 0.70 used to be**, and the old number was never measured against anything: it was
headroom picked for a 128 k window, where a fifth of the context is tens of thousands of tokens held empty and a
summarising completion is bought sooner and more often than it needs to be. ⚠️ **What the headroom has to cover is
absolute, not proportional**: compaction runs *between* turns, so a turn already under way grows unchecked to
`GB_MAX_TOOL_STEPS` completions and their results, and stage 2's request carries the whole history plus room for the
summary written back. 15% of 128 k is 19 k and comfortable, 15% of a local model's 60 k is 9 k and merely adequate, 5% of
60 k will not fit a summary at all — which is why the variable is refused outside 0.2–0.95 rather than clamped into it.
Going over is not fatal either way (a failed summary falls back to `trim_history`, a failed turn to a wait), but each one
costs the run its memory or a turn. ⚠️ The threshold is also a **test fixture**:
`a_full_context_is_summarised_and_the_next_turn_carries_the_summary` sizes its prose against it, and a turn that lands
under it makes that test pass by never compacting at all.

⚠️ **A local endpoint's real limit is its KV cache, not its advertised window, and the arithmetic is per *slot*.** A run
against LM Studio wedged for 28 minutes at a time with no error anywhere: llama.cpp was configured `n_parallel=4,
n_ctx=60000, kv_unified=true`, so four request slots *shared* one 60160-token cache. Anything that made the server pick a
fresh slot rather than reuse the prefix left the old slot still holding its copy, and four copies of a ~16 k conversation
is 66 k against 60 k. The log's tell is a `slot selection` line with **no `launch_slot_` after it**; prefill itself was
never the problem (360–490 tokens/s). One slot with the whole window is the fix, and it is the model server's setting.

⚠️ **`IMAGE_TOKENS = 85` is the `detail: "low"` price and a map is not that.** Measured across all 226 sized maps at 1×
with OpenAI's tiling: median 765, mean 1041, max 3825 — the tail is twelve long thin routes, because a narrow strip is
scaled *up* until its short side is 768. `image_tokens` prices each picture from its own dimensions, and it matters
because `Accounting::occupancy` decides when the history compacts: a full context priced at 85 an image never compacts at
all.


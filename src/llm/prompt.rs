//! **W4 / §7.1, §7.5** — what the model is told.
//!
//! Two pieces, and the split matters:
//!
//! - [`SYSTEM_PROMPT`] is written once and **never compacted** (W6), so it is the copy of the turn
//!   contract that survives everything. ⚠️ It is also byte-identical for the whole run — see
//!   [`system_message`] for why nothing dynamic may go back into it.
//! - [`situation`] is rebuilt for every turn, and is deliberately *rich*. §7.1's finding is that the
//!   larger win is not batching read tools efficiently but not needing them: a turn that opens with
//!   the location, the party, the on-screen text, what happened since the last decision and the menu
//!   itself should need **zero** read calls. Tools are then for what does not fit — a picture of
//!   the map, the full bag, a route back to somewhere already walked.
//!
//! ⚠️ **Anything a read can answer from the situation should be in the situation, and then the read
//! should be deleted.** `read_screen_text` and `read_trainer` both were: a round trip whose answer
//! the model is already holding costs a completion and teaches it that a turn opens by reading.
//!
//! The contract is restated at the bottom of every turn (§7.5's second line of defence), which costs
//! two lines of tokens and makes the rule the most recent instruction in the context every time.

use crate::llm::battle_script::ScriptState;
use crate::llm::tools::{DecisionKind, MenuItem, terminal_names};
use crate::pokemon::battle::is_ghost_battle;
use crate::pokemon::GameState;
use crate::pokemon::agent::AgentEvent;

/// Index 0 of the history, and — since the notes came out of it — **byte-identical for the whole
/// run**.
///
/// ⚠️ **Nothing dynamic may go back in here.** It used to carry the model's TODO list, re-rendered
/// on every request; a prompt cache is keyed on the *prefix*, so each edit the model made to its own
/// plan invalidated the entire conversation for the next request. On a hosted endpoint that is the
/// cache discount thrown away, and on a local server it is re-prefilling tens of thousands of tokens
/// before a single new one is produced. The plan now rides in [`plan_message`], near the tail.
pub fn system_message() -> crate::llm::protocol::Message {
    crate::llm::protocol::Message::system(SYSTEM_PROMPT)
}

/// **W6b / §10** — the model's plan, as the message that carries it.
///
/// A message of its own rather than a block inside the situation, because the worker has to be able
/// to take the stale copy back *out* — see `Worker::sync_plan`. Removing a whole message is exact;
/// editing a slice out of the middle of one is a parser.
pub fn plan_message(todo: &crate::llm::todo::TodoList) -> crate::llm::protocol::Message {
    crate::llm::protocol::Message::user(todo.render())
}

/// What a turn says when the plan is not being re-sent with it — appended to the situation by
/// [`Worker::run_one`](crate::llm::worker::Worker).
///
/// ⚠️ **A message the model can still see is not a message the model is still reading.** The plan
/// rides in a `user` message of its own and is only re-emitted when it changes or when
/// [`PLAN_REFRESH_TURNS`](crate::llm::worker::PLAN_REFRESH_TURNS) falls due, so on most turns it is
/// tens of turns back in a conversation that is mostly menus. Both deployed runs behaved as though
/// it were not there at all — 258 turns with one `todo_set` on turn 1 and no edit after it. This
/// line is the cheap half of the fix (the refresh is the other): it is part of the situation, which
/// is fresh tokens every turn regardless, so it costs nothing at the prefix cache.
///
/// ⚠️ **It says "unchanged", not "here it is".** Restating the items would be a second copy of the
/// thing the whole one-message design exists to avoid, and a copy that ages into the history behind
/// every turn.
pub const PLAN_UNCHANGED: &str =
    "Your plan is unchanged since the last `## Your plan` message in this conversation — the one \
     nearest the end — and that copy is still the current one. Read it back before you decide; if it \
     no longer describes what you are doing, fix it with `todo_set` or `todo_complete` in this same \
     turn.";

/// Appended once, by [`History::open`](crate::llm::history::History::open), to a conversation that
/// has just come back off disk.
///
/// ⚠️ **This exists because half of the restore is a lie the model would otherwise have to explain
/// to itself.** `state.gbst` is the last periodic checkpoint — up to a minute old, and older still
/// on a SIGKILL — while `history.json` is the last completed turn, so a resumed run can open with
/// its own recent messages describing a Pokémon Centre it is no longer standing in. That is not a
/// small confusion: a situation contradicting the conversation is precisely the input that had one
/// deployed run call the game buggy, glitched, broken or in need of a reset in 29 of its 258
/// decision summaries. Naming the skew costs about sixty tokens once per process and removes the
/// only reading under which something is wrong.
///
/// ⚠️ **It says which side is the truth.** "Something may have changed" would leave the model to
/// pick, and it has no way to. The save point is authoritative by construction — it is what the
/// emulator actually loaded — so the sentence says so rather than describing a discrepancy.
///
/// ⚠️ **A legal [`compaction::is_turn_start`]**, so a compaction can eventually drop it like any
/// other turn. It is deliberately at the tail, where a message that differs on every process start
/// costs nothing at the prefix cache.
pub const RESUMED_NOTE: &str =
    "The program was restarted and this conversation was restored from disk. The game itself \
     resumed from its last save point, which may be up to a minute behind the last thing said \
     above. So the action you took in your most recent turn may not have happened, and a few \
     seconds of play may have been replayed. If what you are shown now does not match what you \
     thought you had just done, the save point is right and this conversation is ahead of it. \
     Nothing is broken and there is nothing to undo. Read the situation below and carry on from \
     where the game actually is.";

/// Whether this is a message [`plan_message`] produced.
///
/// ⚠️ **A plan is not a turn boundary** — see [`compaction::is_turn_start`]. It sits immediately
/// before the situation it belongs to, so a cut taken between the two would keep a turn whose plan
/// had been dropped. Being un-cuttable means the plan is only ever dropped *with* the turn after it,
/// and the next turn re-emits it because [`Worker::sync_plan`] cannot find one.
///
/// [`compaction::is_turn_start`]: crate::llm::compaction::is_turn_start
/// [`Worker::sync_plan`]: crate::llm::worker::Worker
pub fn is_plan(message: &crate::llm::protocol::Message) -> bool {
    message.role == crate::llm::protocol::Role::User
        && message.text().is_some_and(|text| text.starts_with(crate::llm::todo::PLAN_HEADING))
}

/// Never compacted. Everything that must stay true for the whole run lives here.
pub const SYSTEM_PROMPT: &str = "\
You are playing Pokémon Red on a Game Boy, through a text interface. You cannot see the screen \
unless you ask for it; instead, an agent reads the game's memory for you, tells you what is \
happening, and executes the decision you return.

Your goal is to play the game well: explore, catch and train Pokémon, beat the eight gym leaders, \
and finish the Elite Four. Take it at a sensible pace and think about what you are doing, but do \
not deliberate at length over routine steps — most decisions are simply 'walk to the next place'.

**Everything below is instruction rather than background**, and every line of it is here because a \
run before yours lost hours to the mistake it names. Two of them are worth doing in your first few \
turns, before you settle into playing: call `read_guide` for the stretch of the game you are in, \
and `set_battle_script` so that routine battles stop costing you a decision each. Both pay for \
themselves within the hour and neither gets cheaper by being put off.

How the interface works:

- The agent handles all button pressing, pathfinding and menu navigation. You choose *what* to do; \
  it works out *how*. Walking to a tile, talking to a person, taking a warp and picking a battle \
  move are all one decision each.
- Every turn you are shown the current situation and a menu of the actions available right now. \
  Each has an opaque `id`. Copy an id exactly; it is not a position in the list, and the list can \
  reorder between turns.
- The game keeps running while you think. A menu action can therefore disappear before your answer \
  lands — you will be told when that happens, and shown the current menu, so simply pick again.
- **One decision can be several actions.** Chain the steps you already know onto `choose_action` \
  with `then` — talk to the Nurse, then take the door out — and set `resume_after_battle` on a walk \
  a wild encounter might interrupt. Both are pure saving; their descriptions say what ends a chain.
- Read tools do not end the turn, and **most turns should need none of them**: the situation \
  already carries the party, the money, the badges, what is on screen and the menu, and a read \
  whose answer you are already holding costs a whole round trip for nothing. Ask for the ones you \
  do need together in one message; the list at the foot of each turn is the one that applies, and \
  `screenshot` is the expensive one, for something the others do not describe.
- Not everything is walking: `use_field_move` covers the rest, and its own description says what.
- **The agent can be wrong, and `report_issue` is how you say so.** If the action menu does not \
  describe what is in front of you, or an action keeps failing for a reason you cannot see, file \
  one: what you were trying to do, what you expected, what happened instead. An action the game \
  stopped with a message you were shown is **not** one of these — there the reason is the message, \
  and it is a thing to act on rather than to report. A developer reads these, and the screen and a \
  save state are filed with it. ⚠️ It does **not** end your turn and \
  nothing changes now — so having filed it, carry on and try a different way. Reporting a problem \
  and playing on are not alternatives. What can be wrong is the agent's *description* of the game — \
  a menu row, a route, a name — never the game itself.
- **This conversation is not your memory.** When it fills up it is replaced by a summary, and \
  everything not in that summary is gone. Your plan — `todo_set`, `todo_complete` and \
  `todo_delete`, shown to you every turn under 'Your plan' — is what survives that. It is the only \
  thing that does, so put \
  anything you will still need in an hour there, with the reason attached: somewhere you could not \
  get into, something a person asked you for, something that did not work.

The game is not broken, and you are not debugging it:

- This is the real 1996 cartridge, unmodified, running on an accurate emulator. It has been \
  finished many times. Nothing in it is glitched, stuck, or waiting to be reset, and there is no \
  developer to fix anything for you. You are a player, not a tester.
- So when something does not work, the explanation is almost always that **you have not done the \
  thing the game is waiting for** — a person you have not spoken to, an item you do not have yet, a \
  badge you have not won, a place you have not been. It is never that the game needs another go.
- **Being stopped is not a malfunction; it is how the game tells you something.** Guards, locked \
  doors, people with an errand and scripted scenes all halt you where you stand and put a message \
  on screen. The action you asked for is then reported back as given up on — 'the game stopped you \
  to say something' — and what it said is quoted in the very next lines under 'Since your last \
  decision'. That message is the answer, every time: a door that will not open yet, someone who \
  wants something first, somewhere you are not allowed past. Read it and act on it. A gym or a \
  building you cannot get into yet is ordinary — note on the plan what it is waiting for and go and \
  get that.
- **Doing the same thing again is not a plan.** If an action has failed twice, stop and change what \
  you are doing: go somewhere else, talk to someone you have not talked to, read what you were last \
  told. Doing it a third, fifth and tenth time is the single most expensive mistake available to \
  you, and it never once works.
- Restarting, resetting, backing out to another map to 'clear the state', and waiting for something \
  to settle are not moves this game has. Nothing about the world changes because you left and came \
  back.

Play the game in front of you, not the one you remember:

- You may recognise this game. **Do not act on that.** Anything you think you know about where to \
  go, who is where, what someone is called, what is in a building, or what an item does is a \
  memory of a different playthrough, and acting on it sends you to places you have no reason to be \
  and makes you stop looking for the reason you are stuck. This run's names, its rival, and the \
  order things happen in are whatever *this* game says they are.
- Act only on what you have been told this run: what a person said, what a sign said, what the \
  screen said, what you can see in the menu. If you cannot point to where you learnt something, \
  you did not learn it here.
- **Read what people say to you.** Almost everything the game wants you to do next is said out \
  loud by someone, once, in a text box — where to go, what to fetch, what is blocking you, what \
  they will give you. Those text boxes are quoted back to you under 'Since your last decision'. \
  They are the instructions. Reading 'GRAMPS ISN'T AROUND' and then talking to the same person \
  again is how a run ends up going nowhere.
- When someone tells you something you will need later — a place, an errand, a name, a condition — \
  put it on the plan straight away, with who said it. That sentence will not be shown to you twice.

Your plan, and keeping it:

- **Always have a plan, and treat it as a draft.** Keep two or three open items going even when \
  you are unsure — a rough plan you revise beats an empty one. Nothing on it is a commitment: when \
  an item turns out wrong or impossible, `todo_set` with its number rewrites it and `todo_delete` \
  with its number drops it. Replace it with what you now know rather than completing it or leaving \
  it to mislead you later.
- **The numbers are names, not places.** An item keeps the number it was given for as long as it \
  is on the list, new ones carry on counting up, and a number is never reused — so a plan you have \
  revised for a while holds numbers far higher than the number of items on it. Only ever use a \
  number you can see beside an item in the plan message nearest the end of this conversation. If a \
  call comes back saying there is no such item, it will tell you which numbers there are; use one \
  of those rather than sending the same call again.
- **It is short on purpose and finished items take up room in it.** The list is a plan, not a \
  record of the run: tick things off as you finish them, and delete a finished item once it no \
  longer explains what you are doing next. What you have already achieved comes back to you in the \
  summary of this conversation; it does not need a line here.
- **The order is yours and nothing reorders it.** Items stay where you put them, ticked or not, \
  and a new one goes on the end — so write them in the order you mean to do them, and rewrite them \
  when that changes.
- **Expect to touch it most turns you learn anything.** Finished something? `todo_complete` it. \
  Someone gave you an errand? Add it. Found out the way you meant to go is shut? Rewrite that item \
  to say so and say what you will do instead. A plan you have not changed in a long stretch of \
  turns is not a plan you are following; it is one you have forgotten about, and it is the first \
  thing to check when you cannot say what you are doing or why.
- Write items you could act on cold, after everything else you know has been thrown away — 'ask \
  the man in the Viridian mart what he wants, he would not let me past the north exit' rather than \
  'go north'.

Things worth knowing about this particular game:

- Talking to people is how almost everything progresses. If you are stuck, there is usually someone \
  you have not spoken to.
- Wild encounters happen in tall grass and in caves. A fainted lead Pokémon is not the end of a \
  battle: switch to another one, or use an item.
- The action list is what the agent can currently *reach*. If somewhere you want to go is not in it, \
  the way there is blocked, or it is on another map you have to walk to first.
- Some things stay shut until you have a particular move and the badge that lets you use it outside \
  battle. While that is true they are not offered to you at all, and the turn says so plainly under \
  'Blocked here'. That is a different errand, not a thing to keep trying.
- There is a walkthrough for this game, and `read_guide` hands you the stretch of it you are in \
  now. Read it **once at the start of each badge**, before you spend turns wandering to work out \
  where you are meant to be, and put what you need out of it on your plan — it is keyed on your \
  badges alone, so asking again before the next one buys a word-for-word copy. Read it again after \
  this conversation has been summarised: the chapter is not in the summary, and your plan is all \
  that is left of it.

Playing it well, and the clock you are playing against:

- **You are being timed.** The 'Play time' on every turn is the cartridge's own clock, and a \
  finished run is ranked on it. What is being asked of you is the whole game, eight badges and then \
  the Elite Four, played properly and finished as soon as you can manage it. Exploring is not the \
  opposite of being quick: the hours go on circling the same three maps not knowing where you are \
  supposed to be next, and what fixes that is `read_guide` and your plan, not hurrying.
- **Keep the party healthy.** Every turn lists each Pokémon's HP. A Pokémon Centre heals the whole \
  party, for nothing, in about two decisions: take the warp in, talk to the Nurse, accept. Do that \
  before a gym, a cave or a long route rather than after something has fainted.
- **If your whole party faints you black out**, lose half your money, and wake up at the last \
  Pokémon Centre where you accepted a heal, which is not the same as the last one you walked into. \
  So healing at the Centre nearest to wherever you are working is worth the two decisions even when \
  nobody is badly hurt: it is also where you would be sent back to, and the alternative is walking \
  the last three maps a second time.
- **Keep stocked up.** Poké Balls and Potions are what money is for; buy them whenever you are in a \
  mart and can afford to, and top up before setting out somewhere long. A few Antidotes and Paralyz \
  Heals earn their place too. Almost nothing else does, and money is tight early on.
- **Catch Pokémon.** The turn tells you how many species you own and how many you have seen. One you \
  always run from is one you never had: weaken it first, or put it to sleep or paralyse it, then \
  throw a Poké Ball from the battle's item rows. A party that covers several types is what gets \
  through a gym; a single strong Pokémon loses to the first thing it has no answer to, and takes \
  the run down with it. **Your party holds six.** Anything caught past that is stored in a PC you \
  have no way to reach through this interface, so the six you are carrying are the six you have.
- **Experience is only paid out for a knockout.** The cartridge awards it when the opposing \
  Pokémon faints and at no other moment: running away pays nothing, and neither does catching it. \
  So a fight broken off halfway is the one outcome that costs you turns and buys nothing at all. \
  If something is worth attacking it is worth finishing; if it is not, run on the first turn \
  rather than the third.
- **Bring the party up together.** A Pokémon that never battles never levels, and what a knockout \
  pays is divided between every Pokémon that was sent out during the battle — so sending a weaker \
  one in first in an easy fight is how it catches up, and a trained party is what makes a bad \
  matchup survivable instead of fatal.
- **Look round a town before you leave it.** Go into the buildings, read the signs, talk to everyone \
  once. Errands, free items, HMs and the directions you need next all come from people standing in \
  rooms you had no particular reason to enter, and each of them says it once. Items lying on the \
  ground appear in the action menu; pick them up as you pass.
- **Write your battles down.** Most battle turns are the same decision: hit it with whatever does \
  the most damage, heal or switch when you are nearly dead, throw a ball at something you want. \
  `set_battle_script` installs a short program that makes those decisions for you, and a turn it \
  answers costs nothing at all, so a wild encounter on the way somewhere stops interrupting you. \
  Read `get_battle_script_docs` once and write one early: the time it saves is the whole of the \
  clock above, and the moves and type matchups are worked out for you. Keep the fights that matter \
  by calling `battle.ask()` inside it, and change it when the report after a battle shows it doing \
  something you did not intend.
";

/// The line that ends every turn request, and the reason the loop can rely on exactly one terminal
/// call per turn. Regenerated per kind so it names only the tools that turn actually has.
/// The standing line an overworld turn carries while a script is deciding its battles.
///
/// ⚠️ **The first paragraph is a constant and the second is the whole point.** What a model does
/// with a sentence it has already read forty times is skip it, so the fixed half says what being
/// armed *means* — that the battles going past are free, and that a battle report is the only
/// account of them — and the variable half says which script, written for what, and how much it has
/// done. That second half is what a run standing in Pokémon Tower needs in order to notice that the
/// program choosing its moves opens `// Misty gym`.
///
/// ⚠️ **It states the facts and does not tell the model to go and look.** A line that nagged on
/// every overworld turn would be read as noise within an hour, and the run that has a good script
/// is the one the feature exists for: two deployed runs never called `set_battle_script` at all.
/// The same argument keeps `GuideStatus::Current` silent.
fn script_standing_line(standing: &crate::llm::battle_script::ScriptStanding) -> String {
    let mut out = String::from(
        "Your battle script is armed and is deciding your battle turns, so the battles going \
         past are not being put to you and are costing you nothing. A battle report is the only \
         account you get of what it chose. If one shows it losing a Pokémon, fleeing from \
         something worth catching, or attacking with a move the enemy shrugs off, that is the \
         script doing it and not the game: `read_battle_script` shows what it says and \
         `set_battle_script` replaces it.",
    );

    // ⚠️ **Quoted, and in the model's own words.** It is text this run wrote about itself, so it
    // is repeated rather than paraphrased: the whole value is recognising it.
    if let Some(purpose) = standing.purpose.as_deref() {
        out.push_str(&format!(" You installed it for: \"{purpose}\"."));
    }
    out.push_str(&match standing.decided {
        0 => " It has not decided a battle turn yet.".to_string(),
        1 => " It has decided 1 battle turn since you installed it.".to_string(),
        n => format!(" It has decided {n} battle turns since you installed it."),
    });
    // ⚠️ **Said once, at the end, and as a question rather than an instruction.** A script is meant
    // to outlive the fight it was written for; what this line is for is the one that outlived the
    // whole route.
    out.push_str(
        " It will go on deciding every battle until you replace it, so if that is no longer the \
         kind of fight you are in, this is the moment to say so.\n\n",
    );
    out
}

/// What the overworld turn says about the battle script, for every state it can be in.
///
/// ⚠️ **This exists because the warning and the tools that answer it used to be on different
/// turns, and a model cannot carry a fact between two turns.** `offers_battle_script` puts
/// `read_battle_script`, `get_battle_script_docs` and `set_battle_script` on `Overworld` and
/// nowhere else — for good reasons that have not changed — while `Unedited` and `Disarmed` were
/// said only under `TurnContext::Battle`, on the argument that those are the states that *have*
/// battle turns to be told on. They are, and those are exactly the turns that cannot act. The
/// deployed run of 2026-09-01 is the measurement: its script was disarmed at 22:06:33 UTC and in
/// the 25 minutes that followed it was told on **58 of 58 battle turns and 0 of 83 overworld
/// turns**, called neither `read_battle_script` nor `set_battle_script` once, and paid a full
/// prefill for every one of those 58. Nothing bridges a turn boundary either: the thinking is
/// dropped and only `summary` survives, and all 11 summaries in that window that contain the word
/// "script" are about the Tower's scripted Channelers.
///
/// ⚠️ **The disarm reason is carried here rather than left in `read_battle_script`.** The old
/// wording sent the model on a 6 kB round trip to find out what a sentence could have told it, on
/// the one turn where it was already being asked to spend its call budget on writing code. It is
/// `MAX_FAILURE`-capped for the same reason the purpose is capped: this rides on every overworld
/// turn until the script is fixed.
///
/// ⚠️ **`Armed` still says something different in kind, and that is not an inconsistency.** The
/// other two are a fault to be cleared and say so once each turn until they are; `Armed` is a
/// standing fact with no call to action, which is `script_standing_line`'s whole ⚠️ — a line that
/// nagged a working run every turn would be read as noise within the hour.
fn overworld_script_line(
    state: ScriptState,
    standing: &crate::llm::battle_script::ScriptStanding,
) -> String {
    // Named once and shared, because the three tools being *on this turn* is the entire point of
    // saying any of this here.
    const HERE: &str = "Those three tools are offered on an overworld turn and on no other kind, \
                        so this is a turn that can fix it.";
    match state {
        ScriptState::Armed => script_standing_line(standing),
        ScriptState::Unedited => format!(
            "⚠️ Your battle script is still the default one, which decides nothing and hands every \
             battle straight back to you, so every battle you fight is costing you a request. \
             `read_battle_script` shows you the file, `get_battle_script_docs` says what a script \
             can do, and `set_battle_script` replaces it. {HERE}\n\n",
        ),
        ScriptState::Disarmed => format!(
            "⚠️ Your battle script failed and is no longer deciding your battle turns, so they are \
             costing you a request each again.{} The script itself was kept, because it is the \
             thing to edit: `read_battle_script` shows it, `get_battle_script_docs` is the API it \
             is written against, and `set_battle_script` arms a corrected one. {HERE}\n\n",
            match standing.failure.as_deref() {
                // ⚠️ **Quoted rather than paraphrased, for `script_standing_line`'s reason one
                // state along.** It is the sentence the sandbox wrote about this script, and the
                // model has to match it against a line of its own code.
                //
                // ⚠️ **Punctuated here rather than trusted to arrive punctuated.** `describe`
                // ends a runtime error with "(at line 4, position 3)" and no full stop, while
                // `NO_ACTION` ends with one, so a bare `{why}` runs either into the next sentence
                // or doubles the stop. An ellipsis from `standing_failure` is already a terminator
                // and takes neither.
                Some(why) => match why.trim_end() {
                    cut if cut.ends_with('…') => format!(" It stopped because: {cut}"),
                    said => format!(" It stopped because: {}.", said.trim_end_matches('.')),
                },
                // A run resumed across the change that added the field, or one disarmed before it
                // existed. Nothing is said rather than "the reason was not recorded": the very
                // next sentence offers `read_battle_script`, which has it, so an apology here
                // would be a line of the turn spent naming the same tool twice.
                None => String::new(),
            },
        ),
    }
}

pub fn contract(kind: DecisionKind) -> String {
    format!(
        "End this turn by calling exactly one of: {}. Every one of them takes a `summary`: one or \
         two sentences saying what you are doing and why. Your thinking is not kept, so that \
         sentence is the only thing you will still have of this turn when you take the next one.\n\
         These do not end the turn ({}) — call as many as you need, in one message, then finish \
         with a terminal call.",
        terminal_names(kind).join(", "),
        crate::llm::tools::non_terminal_names(kind).join(", "),
    )
}

/// The parts of the situation that need a `PokemonApi` rather than a `GameState`.
///
/// Snapshotted at the tool poll, which is the only moment `LlmPolicy` holds one — see
/// [`Policy::service_tools`](crate::pokemon::policy::Policy::service_tools). Small on purpose: the
/// poll happens fifty times a second and everything that can come from `GameState` does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiSnapshot {
    /// `None` in the overworld, and that is correct rather than a failure — no dialogue font is
    /// loaded there, so there is nothing in video memory to decode.
    pub screen_text: Option<String>,
    /// `HH:MM:SS` of in-game play time.
    pub playtime: String,
    /// **W5** — what the mart the player is standing in front of sells, with each item's price. Read
    /// from `wCurMart`, so it is empty everywhere except inside a shop; a `MartPurchase` turn's whole
    /// menu comes from here, and nothing else can supply it — the stock is not in `GameState`.
    pub mart_stock: Vec<(crate::pokemon::item::ItemId, Option<u32>)>,
    /// Where the player entered the current map, and from where — [`WorldGraph::arrival`], which
    /// only the agent knows. `None` until the first map change this process sees. Not read from the
    /// API, so [`ApiSnapshot::read`] leaves it empty and the policy fills it in from the graph.
    pub arrival: Option<crate::pokemon::world_graph::Arrival>,
}

impl ApiSnapshot {
    pub fn read(api: &crate::pokemon::PokemonApi<'_>) -> Self {
        use crate::pokemon::PokemonApiTrait;
        Self {
            screen_text: crate::pokemon::observe::screen_text(api),
            playtime: crate::pokemon::observe::playtime(api),
            mart_stock: api.mart_item_list().into_iter().map(|item| (item, api.item_price(item))).collect(),
            arrival: None,
        }
    }
}

/// How many `AgentEvent`s one turn request carries. A long conversation emits one per text box, and
/// the twenty most recent are the ones that explain the situation; older than that is history the
/// transcript already holds.
const MAX_EVENTS: usize = 20;

/// The user message that opens a turn.
/// One [`AgentEvent`] as the model will read it.
///
/// Rendered at the moment the event arrives rather than stored: `AgentEvent` is `Debug`-only, and
/// deriving `Clone` on it to keep a buffer would mean touching `agent.rs` — which per `CLAUDE.md`
/// puts `full_playthrough` on the line for what is, here, purely a formatting decision.
pub fn describe_event(event: &AgentEvent) -> String {
    match event {
        AgentEvent::TextBox { message } => format!("Text: {}", message.trim()),
        other => format!("{other}"),
    }
}

/// The part of a question that is not in the [`GameState`] — because the agent passed it as an
/// argument to the `pick_*` that asked.
///
/// Two of the five poll sites are like this: the naming screen knows the species and nothing else
/// does, and the forget prompt knows the four moves and the incoming one. Reading them back out of
/// RAM would be a second source of truth for something already in hand.
#[derive(Debug, Clone, Copy, Default)]
pub enum TurnContext<'a> {
    #[default]
    None,
    Nickname(crate::pokemon::species::PokemonSpecies),
    ForgetMove {
        /// Which party member is learning it. See `Policy::pick_move_to_forget`'s ⚠️.
        slot: usize,
        current: &'a [crate::pokemon::move_name::PokemonMove],
        new: crate::pokemon::move_name::PokemonMoveName,
    },
    /// **W9 / §14** — the watchdog's turn: what the agent believes it is doing, and for how long.
    /// Carried rather than read from the state for the same reason the two above are — nothing but
    /// the agent knows it.
    Stuck { agent_state: &'a str, stuck_for: std::time::Duration },
    /// The two standing facts an overworld turn carries: whether a battle script is deciding
    /// battles the model is never asked about, and whether the walkthrough chapter has moved on
    /// since it last read one. Only the policy knows either — the script file is the worker's and
    /// `GameState` has never heard of it, and the last chapter read is a fact about the
    /// conversation rather than about the game.
    ///
    /// ⚠️ **All three script states are said here, because this is the only kind of turn carrying
    /// the tools that change any of them.** `Armed` has no other carrier at all — a working script
    /// produces no battle turns, so its line was reachable only on a Safari turn or a
    /// `battle.ask()`, and the 2026-09-01 run armed one at 13:35 and went ~80 overworld turns
    /// without being told a script existed. `Unedited` and `Disarmed` do have battle turns to be
    /// said on, and were said only there, which is the same hole facing the other way: see
    /// `overworld_script_line`'s ⚠️ for the 58-of-58 against 0-of-83 that closed it. They are
    /// therefore said in **both** places, in two different registers — the battle turn names the
    /// cost it is charging, this one names the fix and offers it.
    ///
    /// ⚠️ **`standing` is what stops the armed line being wallpaper.** The sentence below it is the
    /// same on every turn of the run, so a model that has read it forty times learns nothing from
    /// the forty-first and would have to spend a whole round trip on `read_battle_script` to find
    /// out that the script deciding its battles was written for a gym leader three towns back. The
    /// purpose and the tally are the parts that differ.
    Overworld {
        script: ScriptState,
        standing: &'a crate::llm::battle_script::ScriptStanding,
        guide: crate::llm::guide::GuideStatus,
    },
    /// Whether a script is deciding battle turns, on a turn it did not decide. Only the policy
    /// knows: the file is the worker's and `GameState` has never heard of it.
    ///
    /// ⚠️ **`TurnContext::None` is the fourth state and means "a note above already said it".** A
    /// script that calls `battle.ask()` or fails writes a note carrying what it printed, prepended
    /// to this whole situation, and that note is both more specific and more useful than a state
    /// line — so the two are alternatives rather than a pair, and the caller picks. Repeating it
    /// would be the same fact twice on the turn that is already the expensive one.
    ///
    /// ⚠️ **A battle the model has taken over is the case that makes the note *mandatory* rather
    /// than merely better.** `LlmPolicy::taken_over` suspends the script for one fight, so `Armed`
    /// here would tell the model a script was deciding its battle turns on every turn of the one
    /// battle where that is false. See `llm_policy::taken_over_note`.
    Battle { script: ScriptState },
}

pub fn situation(
    kind: DecisionKind,
    state: &GameState,
    snapshot: &ApiSnapshot,
    events: &[String],
    menu: &[MenuItem],
    context: TurnContext<'_>,
    // Battles the script fought without asking — see `crate::llm::battle_report`. Drained into the
    // next turn of **any** kind, because a battle can end on a naming screen or a mart.
    reports: &[String],
) -> String {
    let mut out = String::with_capacity(2048);

    out.push_str(match kind {
        DecisionKind::Overworld => "## Decision: what to do next in the overworld\n\n",
        DecisionKind::Battle => "## Decision: what to do this battle turn\n\n",
        DecisionKind::Nickname => "## Decision: name this Pokémon\n\n",
        DecisionKind::MartPurchase => "## Decision: what to buy here, if anything\n\n",
        DecisionKind::ForgetMove => "## Decision: which move to forget, if any\n\n",
        DecisionKind::Stuck => "## Decision: the game is stuck — get it moving\n\n",
    });

    match context {
        TurnContext::None => {}
        // ⚠️ **At the top of the turn, above the situation, on the same evidence that put
        // `read_guide`'s nudge there (7d521e6).** Both of these are facts the model would otherwise
        // have to remember across tens of turns, and the one thing this repo has measured about
        // where a nudge lands is that the bottom of a system prompt is not it.
        TurnContext::Overworld { script, standing, guide } => {
            out.push_str(&overworld_script_line(script, standing));
            // ⚠️ **Only the stale case is worth a line** — see `GuideStatus::Current`'s ⚠️ for why a
            // run that has never read one is silent here rather than nagged.
            if let crate::llm::guide::GuideStatus::Stale { index } = guide {
                out.push_str(&format!(
                    "⚠️ You have won a badge since you last read the walkthrough, and `read_guide` \
                     now answers with a different chapter: {}, and what stands in the way of it. \
                     What you read before is the stretch of the game you have already finished, so \
                     anything you are still going on from it is out of date. Read it again on this \
                     turn, before you decide where to go.\n\n",
                    crate::llm::guide::chapter_goal(index),
                ));
            }
        }
        TurnContext::Nickname(species) => out.push_str(&format!(
            // No article: a species name is one, and "a Eevee" / "a Omanyte" is what picking one
            // blind gets you in a sentence printed on every naming screen of the run.
            //
            // ⚠️ **Asking for a name is the whole point of this turn, and the wording used to talk
            // the model out of it.** It said the default "is the ordinary answer", and across two
            // deployed runs every single naming screen — four of them — was declined: *"Keep the
            // default name ZUBAT for the newly caught Pokémon"*. A turn that only ever has one
            // answer is a turn nobody needed to pay for, so it asks for the other one.
            "The naming screen is open for {species}. It has just been caught, hatched or given \
             to you.\n\n\
             Name it. Not the species again — a name that says what you make of *this* one: how you \
             came by it, what you plan to do with it, what the fight it came out of was like, what \
             it reminds you of. It is the name you will read in every message about it from here on, \
             so pick one you will recognise. Keep the default only if you truly have nothing to \
             say.\n\n",
        )),
        // ⚠️ **Three things this turn did not say, each of which decides it.** It opened "A Pokémon
        // is trying to learn Surf" — not which one, so the model matched four move names against the
        // party by eye; it priced neither the new move nor the four it would displace, so the swap
        // was judged on names alone; and it never marked an HM, while `DeterministicPolicy` has
        // always given HM moves max value so they are *never* forgotten. Losing Cut or Surf strands
        // a run behind terrain the menu then stops offering a way past, which is the most expensive
        // single answer available on this turn and the one nothing warned about.
        TurnContext::ForgetMove { slot, new, current } => {
            let metadata = new.metadata();
            let learner = state.pokemon.iter().nth(slot);
            out.push_str(&format!(
                "{} is trying to learn **{new}** ({}, {}, {} pp) but already knows four moves. Pick \
                 one to replace, or decline and keep all four.\n\n",
                match learner {
                    Some(mon) => format!("{} (slot {slot}, {})", named(mon), types_of(mon)),
                    None => "A Pokémon".to_string(),
                },
                metadata.move_type,
                match metadata.power {
                    Some(power) => format!("{power} power"),
                    None => "no damage".to_string(),
                },
                metadata.pp,
            ));
            if current.iter().any(|m| crate::llm::tools::hm_move(m.name).is_some()) {
                out.push_str(
                    "⚠️ One of the four is an HM move. An HM cannot be un-taught and cannot be \
                     re-learnt from the machine, so forgetting one means finding another Pokémon to \
                     teach it to before you can cross the terrain it clears again.\n\n",
                );
            }
        }
        // ⚠️ **At the top rather than as a footnote under `### Battle`, on the one piece of
        // evidence this repo has about where a nudge lands.** `read_guide` and `set_battle_script`
        // were both argued for in the system prompt at length and both were called zero times in 55
        // overworld turns; what moved `read_guide` was a concrete *when*, put at the top of the turn
        // ahead of the prose making the case (7d521e6). The system prompt is message 0 and is the
        // least recent thing in every request by hundreds of turns, so an argument living only there
        // is one the model reads once.
        TurnContext::Battle { script } => out.push_str(match script {
            // ⚠️ **The deployed run of 2026-08-27: 207 battle turns, 22.3 M prompt tokens, and
            // `set_battle_script` never called once.** Not weighed and rejected — never reached, the
            // same shape `read_guide` was in. This is the only line in the run that says, on the
            // turn actually being charged for, that the charge is optional.
            //
            // ⚠️ **It names a script that exists rather than one to invent.** Every run now starts on
            // `battle_script::DEFAULT`, which asks every turn, so what is being asked for here is an
            // edit to a file the model can open in one call rather than a blank page. That is the
            // whole of why the default is worth having, and why `read_battle_script` is named first.
            ScriptState::Unedited => {
                "⚠️ Your battle script is still the default one, which decides nothing and hands \
                 every battle straight back to you, so this turn costs a request exactly like \
                 every other battle turn. Your next overworld turn carries the tools that change \
                 that, and says so.\n\n"
            }
            // ⚠️ **Neither the reason nor the tool names are repeated here, and both used to be.**
            // A battle turn is for winning the battle in front of you: it cannot call
            // `set_battle_script` — `offers_battle_script` is `Overworld` only — so naming the
            // tool here was an instruction the turn could not carry out, and quoting the reason
            // was 240 bytes on every battle turn of a broken run to no end. Both are on the
            // overworld turn now, in `overworld_script_line`, where they can be acted on. What is
            // left is the one thing only this turn can say: that the request being spent right
            // now is the optional one.
            ScriptState::Disarmed => {
                "⚠️ Your battle script failed and is no longer deciding your battle turns, so this \
                 one is costing you a request again. Your next overworld turn carries the reason \
                 it stopped and the tools to fix it.\n\n"
            }
            // Armed, consulted or not, and it did not answer — a Safari battle, which is never
            // scripted, or a turn the model itself asked to `wait` through. Without this the run
            // looks to the model exactly like a script that has quietly stopped working, which is a
            // false alarm worth a request to avoid.
            ScriptState::Armed => {
                "Your battle script is armed and deciding your battle turns, but it did not decide \
                 this one.\n\n"
            }
        }),
        // **W9 / §14.** Said plainly, including that it is the agent's fault: a model told only "you
        // are stuck" tends to reason about the *game* being stuck and go looking for a puzzle.
        TurnContext::Stuck { agent_state, stuck_for } => out.push_str(&format!(
            "**The agent has not offered you a decision for {} seconds of game time.** It thinks it \
             is busy doing `{agent_state}`, and it is not asking anything — so this is a bug in the \
             agent rather than a puzzle in the game, and no action menu can be shown.\n\n\
             What usually clears it is one button: `A` to advance a text box or confirm a prompt, \
             `B` to back out of a menu, a direction to step off a tile it cannot leave. Look at the \
             screen if you are unsure — `screenshot` is worth it here, because the state description \
             is exactly what has gone wrong. If you think the game genuinely needs a moment, `wait`.\n\n",
            stuck_for.as_secs(),
        )),
    }

    out.push_str(&format!(
        "Location: {} at ({}, {}), facing {:?}\n",
        state.map.map, state.map.player_position.x, state.map.player_position.y, state.map.player_direction,
    ));
    // Which door the player came in by. On a map with several warps to the same place this is the
    // only thing that tells the one just used from the ones not yet tried — the row for that warp
    // says so too (`tools::overworld_menu`), so the two cannot be read apart.
    if let Some(arrival) = snapshot.arrival.filter(|a| a.map == state.map.map) {
        out.push_str(&match arrival.from {
            Some(from) => format!("Entered this map at ({}, {}) from {from}\n", arrival.at.x, arrival.at.y),
            None => format!("Entered this map at ({}, {})\n", arrival.at.x, arrival.at.y),
        });
    }
    // What the player is facing is the precondition for half of `use_field_move` — `cut` works on
    // the tile in front and nothing else — and it is one line against a whole `read_map`.
    if let Some((at, tile)) = state.map.tile_in_front() {
        out.push_str(&format!("Facing: {tile} at ({}, {})\n", at.x, at.y));
    }
    // ⚠️ **What the menu no longer offers still has to be explainable.** A cuttable tree is
    // impassable *for now* rather than for ever, and is withheld from the action menu until the
    // party can actually deal with it (`MetaTileMap::can_cut`) — because an action whose only
    // follow-up the game refuses is a menu loop, not a choice. Withholding it silently would leave a
    // model staring at a map with no way out and no idea why, which is the other half of the same
    // bug: it would go looking for a route that does not exist, or conclude the agent is broken. So
    // the obstacle is named once, in a sentence, with what it takes to pass it.
    //
    // ⚠️ **Only on the turn that has an action menu.** It is a fact about that menu, and the other
    // five kinds do not have one — on a naming screen it is overworld trivia in the middle of a
    // question about a word.
    //
    // ⚠️ **And it has to say which half is actually missing, or it sends the model at the one that
    // is not.** The line used to read "an HM to be found and taught, and needs the CascadeBadge"
    // whatever the run was holding, so a party carrying HM01 and the badge with nothing in it that
    // can learn Cut was told to go and find HM01. That is the state the deployed run of 2026-08-27
    // was in, and what it did instead was try the teach over and over.
    if kind == DecisionKind::Overworld {
        use crate::pokemon::badge::Badge;
        use crate::pokemon::item::ItemId;
        use crate::pokemon::learnset::can_learn;
        use crate::pokemon::tile::MetaTile;
        // ⚠️ **Water was the second entry here and is deliberately gone — it named an obstacle
        // that was almost never the obstacle.** A `CutTree` is a specific tile that is *the way on*;
        // water is scenery on most outdoor maps, so the line fired on Route 9, Route 10, Cerulean,
        // Vermilion and nearly everywhere else with a pond or a coast in it, on turns where nothing
        // about the water was in anybody's way. The 2026-09-02 deployed run is the measurement: 65
        // turns oscillating on the Rock Tunnel north entrance, where the only thing it had not done
        // was take the B1F ladder at (27, 3), and its own decision summaries blamed "water-block
        // geometry", "the water-block trap" and "the wrong side of water" — the one sentence in the
        // turn that offered a reason. ⚠️ **"Nothing in the menu below leads past it" is what did the
        // damage**: true of the water and read as true of the turn. Cut keeps the line because the
        // Route 2 failure it was written for is a tree that genuinely is the route.
        let obstacles: [(bool, fn(&MetaTile) -> bool, &str, ItemId, &str, Badge); 1] = [
            (!state.can_use_cut, |tile| matches!(tile, MetaTile::CutTree), "Cuttable trees",
             ItemId::Hm01Cut, "Cut", Badge::CascadeBadge),
        ];
        for (blocked, is_obstacle, noun, hm, name, badge) in obstacles {
            if !blocked || !state.map.meta_tiles.iter().any(is_obstacle) { continue; }
            let held = state.bag.iter().any(|entry| entry.id == hm);
            let taker = state.pokemon.iter().position(|mon| can_learn(mon.species, hm));
            let badged = state.badges.contains(badge);
            let what = match (held, taker, badged) {
                // Everything is in hand: this is one tool call away, so say which one.
                (true, Some(slot), true) => format!(
                    "teaching {name} to a party member. {hm} is in your bag and slot {slot} can learn \
                     it, so `use_field_move` with `teach` is all that is left"),
                (true, Some(slot), false) => format!(
                    "the {badge}, which you do not have yet. {hm} is already in your bag and slot \
                     {slot} can learn it, so the gym is the only thing in the way"),
                // The one the deployed run was in, and the one nothing used to say out loud.
                (true, None, _) => format!(
                    "a Pokémon that can learn {name}. {hm} is in your bag, but nothing in your party \
                     is in its learnset and the game refuses the teach, so catching or swapping one \
                     in is what this needs{}",
                    if badged { String::new() } else { format!(", and the {badge} after that") }),
                (false, _, true) => format!(
                    "{name}, which is taught by {hm}: you have the {badge} but not the HM, so finding \
                     it is the errand"),
                (false, _, false) => format!(
                    "{name}, which is an HM to be found and taught, and needs the {badge}"),
            };
            out.push_str(&format!(
                "Blocked here: {noun} on this map cannot be passed yet. That needs {what}. \
                 Nothing in the menu below leads past it, and retrying will not change that.\n",
            ));
        }
    }
    let badges: Vec<String> = state.badges.iter_names().map(|(name, _)| name.to_string()).collect();
    out.push_str(&format!(
        "Badges: {}\nMoney: ¥{}   Play time: {}\n",
        if badges.is_empty() { "none yet".to_string() } else { badges.join(", ") },
        state.money,
        snapshot.playtime,
    ));
    // The two lines that used to justify `read_trainer`. The rival's name is what half the game's
    // dialogue calls him, and the dex counts are the only measure of exploration the game keeps —
    // both are constants-per-turn a read should never have had to be spent on.
    out.push_str(&format!(
        "Trainer: {} (rival {})   Pokédex: {} owned / {} seen\n",
        state.name.to_default_string(),
        state.rival_name.to_default_string(),
        state.pokedex_owned.species().len(),
        state.pokedex_seen.species().len(),
    ));

    out.push_str("\n### Party\n");
    if state.pokemon.len() == 0 {
        out.push_str("(empty)\n");
    }
    for (slot, mon) in state.pokemon.iter().enumerate() {
        let moves: Vec<String> = mon
            .moves
            .iter()
            .flatten()
            .map(|m| format!("{} {}pp", m.name, m.pp))
            .collect();
        out.push_str(&format!(
            "{slot}. {} Lv{} ({}) — {}/{} HP{} — {}\n",
            named(mon),
            mon.level,
            types_of(mon),
            mon.current_hp,
            mon.stats.hp,
            ailment(mon.status),
            if moves.is_empty() { "no moves".to_string() } else { moves.join(", ") },
        ));
    }

    if let Some(battle) = state.battle.as_ref() {
        out.push_str("\n### Battle\n");
        // ⚠️ **`Display`, not `{:?}`.** Both were debug formatting reaching the model, which is the
        // bug `MetaTile`, `PokemonStatus` and `SwitchPokemon`'s menu row were each caught with — and
        // each of those read acceptably right up until the type behind it grew a field. The types
        // are the addition: a battle turn is decided on the matchup and nothing in the situation
        // said what the thing in front of you *is*.
        let side = |who: &str, mon: &crate::pokemon::pokemon::PokemonSummary| {
            let mut types: Vec<String> = mon.types.iter().map(|t| t.to_string()).collect();
            types.dedup();
            format!("{who}: {} Lv{} ({}) — {}/{} HP{}\n",
                    mon.species, mon.level, types.join("/"),
                    mon.current_hp, mon.stats.hp, ailment(mon.status))
        };
        out.push_str(&format!("{} battle\n", battle.battle_type));
        out.push_str(&side("Yours", &battle.player));
        out.push_str(&side("Enemy", &battle.enemy));
        // ⚠️ **Say why the menu has one row, or it reads as the agent being broken.** The run is
        // being told it may only flee a Gastly it is forty levels above, which is the shape of thing
        // that has twice now had a model conclude the game was malfunctioning and file an issue
        // instead of playing on (the locked gym door, the Route 22 guard). The cartridge does *not*
        // say this one out loud: "too scared to move" names the symptom and never the Silph Scope,
        // so unlike a guard who explains himself there is no quoted line below to do the work.
        if is_ghost_battle(state.map.map, &state.bag, battle.battle_type) {
            out.push_str("⚠️ This is a GHOST: no move, ball or switch does anything here until you \
                          are carrying the Silph Scope (it is in the Rocket Hideout, under the Game \
                          Corner in Celadon). Running always works. Nothing is broken.\n");
        }
        if battle.enemy_trapping {
            // ⚠️ The menu still opens and every option still looks available, but any *move* chosen is
            // replaced with "cannot move" — a decider that does not know this loops until the wrap ends.
            out.push_str("⚠️ You are trapped (Wrap/Bind/Fire Spin): a move will not execute this \
                          turn, but items, switching and running still work.\n");
        }
    }

    // ⚠️ **Above `### On screen` and below `### Battle`, which is where "what just happened" goes.**
    // It is rendered into the situation rather than appended as a message of its own: unlike the
    // plan, it is read once by the very next turn, so a message would be a permanent extra entry in
    // a history that is compacted for length. See `battle_report`'s last ⚠️.
    for report in reports {
        out.push('\n');
        out.push_str(report);
    }

    if let Some(text) = snapshot.screen_text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        out.push_str(&format!("\n### On screen\n{text}\n"));
    }

    let recent = summarise_events(events);
    if !recent.is_empty() {
        out.push_str("\n### Since your last decision\n");
        for line in recent {
            out.push_str(&format!("- {line}\n"));
        }
    }

    out.push_str(match kind {
        DecisionKind::Overworld => "\n### Actions available now\n",
        DecisionKind::Battle => "\n### Battle menu\n",
        DecisionKind::Nickname => "\n### Naming\n",
        DecisionKind::MartPurchase => "\n### For sale\n",
        DecisionKind::ForgetMove => "\n### The four moves it knows\n",
        DecisionKind::Stuck => "\n### What you can do\n",
    });
    if menu.is_empty() {
        out.push_str(match kind {
            DecisionKind::Stuck => {
                "(no menu — the agent is not offering actions, which is why you are being asked. \
                 `press_buttons`, or `wait`.)\n"
            }
            DecisionKind::Nickname => {
                "(there is no menu — call `set_nickname` with the name you have chosen. Omitting \
                 `name` keeps the species name, and is the answer only if nothing comes to mind.)\n"
            }
            DecisionKind::MartPurchase => {
                "(the shop's stock could not be read. Call `buy_item` with no `item` to leave.)\n"
            }
            DecisionKind::ForgetMove => {
                "(the move list could not be read. Call `forget_move` with no `slot` to decline.)\n"
            }
            _ => {
                "(nothing — the agent can reach no action from here. `wait` and look again; if it \
                 stays empty you are boxed in and the run needs a person.)\n"
            }
        });
    }
    for item in menu {
        out.push_str(&format!("- `{}` — {}\n", item.id, item.description));
    }

    out.push_str(&format!("\n{}\n", contract(kind)));
    out
}

/// A status worth mentioning, or nothing at all.
///
/// ⚠️ **`PokemonStatus`' `Display` is `strum`'s derive, so a healthy Pokémon prints `None`** — and
/// every party line said `20/20 HP, None`, which reads as a missing value rather than as good news
/// A party member as the header names it: the nickname, and the species behind it when the two
/// differ.
///
/// ⚠️ **The species used to be missing entirely, and the nickname prompt is what made that
/// expensive.** The line was `mon.nickname` alone, which is the species in capitals until the model
/// names something — and the naming screen now asks it, in as many words, for a name that says what
/// it makes of *this* Pokémon rather than the species again. So the better the model answers that
/// prompt, the more completely the species disappears from every turn for the rest of the run, and
/// the only way back was `read_party` — a whole round trip whose remaining content is species, types
/// and stats. The idiom is `learnset::teach_refusal`'s, which had the same problem first.
fn named(mon: &crate::pokemon::pokemon::Pokemon) -> String {
    let nickname = mon.nickname.to_default_string();
    let species = mon.species.to_string();
    match nickname.eq_ignore_ascii_case(&species) {
        true => nickname,
        false => format!("{nickname} the {species}"),
    }
}

/// A party member's types, deduplicated — the game stores a single-type mon's one type in both
/// slots, so an undeduplicated `Normal/Normal` is the derive showing through.
///
/// ⚠️ **On the party line rather than behind `read_party`, and the argument is the module's own
/// rule.** "Anything a read can answer from the situation should be in the situation": the prompt
/// tells the model that a party covering several types is what gets through a gym and that a single
/// strong Pokémon loses to the first thing it has no answer to, and then gave it no way to check
/// either without buying a read. ~12 bytes a line against a read that costs a completion.
fn types_of(mon: &crate::pokemon::pokemon::Pokemon) -> String {
    let mut types: Vec<String> = mon.types.iter().map(|t| t.to_string()).collect();
    types.dedup();
    types.join("/")
}

/// and cost six characters per member of every turn for the privilege. Poisoned is worth a word;
/// healthy is worth silence, and the HP beside it already says how the mon is doing.
fn ailment(status: crate::pokemon::status::PokemonStatus) -> String {
    match status {
        crate::pokemon::status::PokemonStatus::None => String::new(),
        other => format!(", {other}"),
    }
}

/// The events since the last turn, most useful last.
///
/// Consecutive duplicates are collapsed: the agent emits one `TextBox` per screen of dialogue and a
/// long conversation repeats the same line as the box scrolls, which would otherwise fill the turn
/// with the same sentence eight times.
fn summarise_events(events: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in events {
        if lines.last() == Some(line) {
            continue;
        }
        lines.push(line.clone());
    }
    if lines.len() > MAX_EVENTS {
        lines.drain(..lines.len() - MAX_EVENTS);
    }
    lines
}

/// What the worker sends when a tool batch was answered and the turn has one request left.
///
/// ⚠️ **It has to arrive while there is still a request to answer it with** — see the `+ 2` in
/// `Worker::decide`. A "call a terminal tool now" that lands after the last request is a sentence
/// the model can only read on the next turn.
pub const OUT_OF_STEPS: &str =
    "You have used every read this turn. Call a terminal tool now to end the turn.";

/// §7.5's fallback, quoted verbatim at a model that produced no tool call at all.
pub fn nudge(kind: DecisionKind) -> String {
    format!(
        "That reply contained no tool call, so nothing happened in the game. {}",
        contract(kind),
    )
}

/// [`nudge`] for the case the reply was *cut off* rather than finished — `GB_MAX_TOKENS` was reached.
///
/// ⚠️ **Worth telling apart, because the two ask for opposite corrections.** A model nudged with
/// "that reply contained no tool call" and no other information reasonably concludes it forgot to
/// call one, and tries again at the same length — into the same ceiling. What it needs to know is
/// that the thinking, not the tool call, is what ran out of room.
pub fn truncated_nudge(kind: DecisionKind) -> String {
    format!(
        "That reply was cut off before it finished: it hit the maximum length, so it carried no \
         tool call and nothing happened in the game. Think more briefly this time — a decision here \
         rarely needs more than a few sentences of reasoning. {}",
        contract(kind),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason};
    use crate::pokemon::tile::MetaTile;

    /// **Every decision kind's first request, written out whole, so a person can read what the model
    /// is actually sent.**
    ///
    /// Two files per kind in `target/turn-requests/`: the `.json` is the literal `ChatRequest` body
    /// that would go to the endpoint, assembled the way [`crate::llm::worker::Worker::decide`]
    /// assembles it, and the `.md` is the same thing with the newlines put back — a prompt read
    /// through JSON's `\n` escaping is not a prompt anyone can review.
    ///
    /// ⚠️ **Prints a report rather than asserting, so it is `#[ignore]`d on top of its feature
    /// gate**, as `CLAUDE.md`'s table requires of every `probe_`.
    ///
    /// ⚠️ **Two of the six situations cannot come from a save state alone, and the files say so.**
    /// A mart's stock lives in `wCurMart`, which is empty everywhere except inside a shop, and no
    /// committed fixture is standing at a counter; the forget-move prompt needs a move the party is
    /// about to learn, which is a fact about an event and not about memory. Both are supplied here.
    /// Everything else — the map, the party, the bag, the battle, the menus and their ids — is read
    /// out of a real fixture by the same functions the run uses.
    /// A battle report for the probe to render, built the way the policy builds one.
    ///
    /// ⚠️ **Several turns, not one.** The block that needs eyeballing is the one whose prose nothing
    /// else prints, and a one-hit battle exercises none of what makes it hard to read: the damage
    /// on both sides, a turn where nothing moved, an item, a switch, and the closing line.
    #[cfg(feature = "diagnostics")]
    fn probe_reports(kind: DecisionKind) -> Vec<String> {
        use crate::llm::battle_report::BattleReport;
        use crate::llm::battle_script::test_scenario;
        use crate::pokemon::battle::BattleAction;
        use crate::pokemon::item::ItemId;
        use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};

        if kind != DecisionKind::Overworld {
            return Vec::new();
        }
        // Both sides' HP at each decision, which is where the report's numbers come from.
        let at = |mine: u16, theirs: u16| {
            let mut state = test_scenario();
            if let Some(battle) = state.battle.as_mut() {
                battle.player.current_hp = mine;
                battle.enemy.current_hp = theirs;
            }
            state
        };
        let fight = |name| BattleAction::Fight { slot: 1, battle_move: PokemonMove::with_max_pp(name) };

        let opening = at(48, 25);
        let mut report = BattleReport::open(&opening, 0).expect("the scenario is a battle");

        report.decided(&opening, &fight(PokemonMoveName::Ember),
                       vec!["Ember x2 vs Grass, 17 expected".to_string()]);
        report.said("Enemy RATTATA used TACKLE!");

        let second = at(44, 8);
        report.decided(&second, &fight(PokemonMoveName::Growl), Vec::new());
        report.said("Enemy RATTATA's ATTACK fell!");

        let third = at(44, 8);
        report.decided(&third, &BattleAction::UseItem {
            slot: 0,
            item: crate::pokemon::bag::BagItem::new(ItemId::PokeBall, 4),
        }, vec!["8/25 HP, worth a ball".to_string()]);
        report.said("Darn! The POKéMON broke free!");

        let fourth = at(39, 8);
        report.decided(&fourth, &fight(PokemonMoveName::Ember), Vec::new());
        report.said("Enemy RATTATA fainted!");
        report.said("SPARKY gained 56 EXP. Points!");

        vec![report.finish(Some(&at(39, 0)))]
    }

    #[cfg(feature = "diagnostics")]
    #[test]
    #[ignore]
    fn probe_turn_requests() {
        use crate::llm::protocol::{ChatRequest, Message, StreamOptions};
        use crate::llm::tools;
        use crate::pokemon::integration_tests::fixture::TestFixture;
        use crate::pokemon::item::ItemId;
        use crate::pokemon::move_name::PokemonMoveName;
        use crate::pokemon::species::PokemonSpecies;
        use std::time::Duration;

        let out = std::path::Path::new("target/turn-requests");
        std::fs::create_dir_all(out).expect("a writable target directory");

        // A mid-game overworld save: eight badges' worth of bag, a full party, and a town with
        // enough on it that the action menu is a realistic length rather than a doorway.
        let overworld = include_bytes!("../pokemon/data/at-celadon.bin");
        let battle = include_bytes!("../pokemon/data/battle-state.bin");

        // What the agent said since the last decision. Rendered through `describe_event`, which is
        // the same funnel the live buffer goes through.
        let events: Vec<String> = [
            AgentEvent::StartedOverworldAction { destination: MetaTile::Sprite("Gym Guide") },
            AgentEvent::OverworldInteractionCompleted { target: MetaTile::Sprite("Gym Guide") },
            AgentEvent::TextBox { message: "Hey! You look weak! Let me give you some advice!".into() },
            AgentEvent::OverworldActionAborted {
                destination: MetaTile::Warp { to_map: crate::pokemon::map::Map::CeladonGym, to_position: crate::geometry::Point8 { x: 4, y: 17 } },
                reason: OverworldActionAbortedReason::Textbox,
                at: Some(crate::geometry::Point8 { x: 8, y: 19 }),
            },
        ]
        .iter()
        .map(describe_event)
        .collect();

        // A plan with something in it, because an empty one is not what a run looks like after the
        // first ten minutes — and because it is a message of its own in every request after it
        // changes (see `worker::sync_plan`).
        let mut todo = crate::llm::todo::TodoList::open(None);
        for item in [
            "beat Erika for the Rainbow Badge; the gym is the one behind the trees, cut them",
            "buy a Poke Doll in Celadon before Lavender: it is the only way past the Marowak ghost",
            "come back to Route 12 with the Poke Flute, the Snorlax blocks the only path south",
        ] {
            todo.apply(crate::llm::todo::TodoCall::Set { id: None, text: Some(item.to_string()) });
        }

        let config = LlmConfigForProbe::default();

        for kind in tools::ALL_KINDS {
            let mut fixture = TestFixture::new(
                match kind {
                    DecisionKind::Battle => battle,
                    _ => overworld,
                },
                Duration::from_secs(10),
                vec![],
            );
            let state = fixture.game_state();
            let mut snapshot = ApiSnapshot::read(&fixture.api());

            // The two facts no fixture carries. Prices are still the cartridge's own.
            if kind == DecisionKind::MartPurchase {
                let api = fixture.api();
                snapshot.mart_stock = [
                    ItemId::PokeBall, ItemId::GreatBall, ItemId::Potion,
                    ItemId::SuperPotion, ItemId::Antidote, ItemId::Repel,
                ]
                .into_iter()
                .map(|item| (item, { use crate::pokemon::PokemonApiTrait; api.item_price(item) }))
                .collect();
            }
            // `forget_menu` takes the four slots as the party stores them, `None` included.
            let party_moves: Vec<_> = state
                .pokemon
                .iter()
                .next()
                .map(|mon| mon.moves.iter().flatten().cloned().collect())
                .unwrap_or_default();

            // ⚠️ **A standing the armed line can actually say something with.** `Armed` renders
            // the purpose and the tally and nothing else — `standing()` withholds a failure while a
            // script is deciding turns — so an empty one would price the line at its cheapest and
            // hide the two fields it exists for. `MAX_PURPOSE` is 200; this is a realistic one at
            // about half that, which is what a model writes.
            let standing = crate::llm::battle_script::ScriptStanding {
                purpose: Some(
                    "Fight with the best damaging move, switch out below a third HP, and hand back \
                     any trainer battle so I can think about it."
                        .to_string(),
                ),
                decided: 340,
                failure: None,
            };
            let context = match kind {
                DecisionKind::Nickname => TurnContext::Nickname(PokemonSpecies::Eevee),
                DecisionKind::ForgetMove => {
                    TurnContext::ForgetMove { slot: 0, current: &party_moves, new: PokemonMoveName::Surf }
                }
                DecisionKind::Stuck => TurnContext::Stuck {
                    agent_state: "text→ReadingTextBox",
                    stuck_for: Duration::from_secs(300),
                },
                // The deployed shape rather than the flattering one: every run so far has reached
                // its battle turns with no script at all, and this is the turn the probe is read to
                // find out what that costs.
                DecisionKind::Battle => TurnContext::Battle { script: ScriptState::Unedited },
                // ⚠️ **Both overworld notes on at once, which is the dearest this turn gets rather
                // than the likeliest.** The probe is read to price a turn and to see what is
                // actually sent, so a context that renders neither line would hide them from the one
                // tool that shows the prose — and would go on hiding them after somebody quietly
                // changed the policy back to `TurnContext::None`. `Armed` is the standing state of
                // any run that has written a script; `Stale` lasts from a badge until the next
                // `read_guide`, so the two together are a real turn, just not a common one.
                // ⚠️ **The index comes from the fixture's own badges rather than a constant.** A
                // hardcoded one renders "go and read about the ThunderBadge" onto a turn whose
                // header already lists the ThunderBadge as won, which is a probe teaching its
                // reader something that cannot happen.
                DecisionKind::Overworld => TurnContext::Overworld {
                    script: ScriptState::Armed,
                    standing: &standing,
                    guide: crate::llm::guide::GuideStatus::Stale {
                        index: crate::llm::guide::chapter_index(state.badges),
                    },
                },
                _ => TurnContext::None,
            };
            let menu = match kind {
                DecisionKind::Overworld => tools::overworld_menu(&state, snapshot.arrival),
                DecisionKind::Battle => tools::battle_menu(&state),
                DecisionKind::MartPurchase => tools::mart_menu(&snapshot, &state),
                DecisionKind::ForgetMove => tools::forget_menu(&party_moves),
                DecisionKind::Nickname | DecisionKind::Stuck => Vec::new(),
            };

            // The message list a first turn goes out with, in the order `worker::run_one` appends
            // them: the constant system message, the plan, then the situation.
            let messages = vec![
                system_message(),
                plan_message(&todo),
                // ⚠️ A battle report on the battle turn, because that is where the probe is worth
                // reading: it is the one block whose prose nothing else prints.
                Message::user(situation(kind, &state, &snapshot, &events, &menu, context, &probe_reports(kind))),
            ];
            let request = ChatRequest {
                model: config.model.clone(),
                messages: messages.clone(),
                tools: tools::for_kind(kind),
                parallel_tool_calls: Some(true),
                max_tokens: config.max_tokens,
                reasoning_effort: None,
                temperature: config.temperature,
                stream: true,
                stream_options: StreamOptions { include_usage: true },
            };

            let label = kind.label();
            let json = serde_json::to_string_pretty(&request).expect("a request serialises");
            std::fs::write(out.join(format!("{label}.json")), &json).expect("writable");
            std::fs::write(out.join(format!("{label}.md")), readable(&request)).expect("writable");

            let prose: usize = messages.iter().filter_map(|m| m.text()).map(str::len).sum();
            let schema = serde_json::to_string(&request.tools).expect("specs serialise").len();
            println!(
                "{label:<14} {:>6} bytes of prose + {:>6} bytes of tool schema ({} tools) → {}",
                prose, schema, request.tools.len(), out.join(format!("{label}.md")).display(),
            );
        }
    }

    /// The request as something to read: the messages with their newlines intact, then one block per
    /// tool. The JSON beside it is the wire truth; this is the reviewable copy.
    #[cfg(feature = "diagnostics")]
    fn readable(request: &crate::llm::protocol::ChatRequest) -> String {
        let mut out = String::new();
        for message in &request.messages {
            out.push_str(&format!(
                "{}\n=== {:?} message ({} bytes) ===\n{}\n\n",
                "─".repeat(100),
                message.role,
                message.text().map_or(0, str::len),
                message.text().unwrap_or("(no text)"),
            ));
        }
        out.push_str(&format!("{}\n=== tools ({}) ===\n\n", "─".repeat(100), request.tools.len()));
        for tool in &request.tools {
            out.push_str(&format!(
                "── {} ──\n{}\n\nparameters:\n{}\n\n",
                tool.function.name,
                tool.function.description,
                serde_json::to_string_pretty(&tool.function.parameters).expect("a schema"),
            ));
        }
        out
    }

    /// The knobs the probe needs off an [`LlmConfig`](crate::llm::LlmConfig) without reading the
    /// environment — a probe that failed because `GB_MODEL` was unset would be reporting on the
    /// shell rather than on the prompt.
    #[cfg(feature = "diagnostics")]
    struct LlmConfigForProbe {
        model: String,
        temperature: f32,
        max_tokens: Option<u32>,
    }

    #[cfg(feature = "diagnostics")]
    impl Default for LlmConfigForProbe {
        fn default() -> Self {
            Self {
                model: "gpt-5".to_string(),
                temperature: 1.0,
                max_tokens: Some(crate::llm::config::DEFAULT_MAX_TOKENS),
            }
        }
    }

    /// §7.5's third and fourth lines of defence are the same sentence in two places. If the tool
    /// catalogue grows a terminal tool and the contract does not mention it, the model is being told
    /// something false at the end of every single turn.
    #[test]
    fn the_contract_names_every_tool_the_turn_is_actually_sent() {
        for kind in crate::llm::tools::ALL_KINDS {
            let contract = contract(kind);
            for tool in crate::llm::tools::for_kind(kind) {
                assert!(contract.contains(tool.function.name),
                        "{kind:?}'s contract does not mention `{}`", tool.function.name);
            }
            assert!(nudge(kind).contains(&contract), "the nudge quotes the contract verbatim");
        }
        assert!(SYSTEM_PROMPT.contains("do not end the turn"),
                "the system prompt is the copy of the contract that survives compaction");
    }

    /// ⚠️ **The system message is a constant, and this is the test that keeps it one.** It used to
    /// carry the model's TODO list and was rebuilt on every request, so every edit the model made to
    /// its own plan changed message 0 — and a prompt cache keyed on the prefix is worth nothing once
    /// the first message moves. The plan is a message of its own now; §9 still cannot compact it,
    /// because `is_turn_start` refuses to cut there and the worker re-emits it if it goes.
    #[test]
    fn the_system_message_never_changes_and_the_plan_is_not_in_it() {
        let mut todo = crate::llm::todo::TodoList::open(None);
        let before = system_message();
        todo.apply(crate::llm::todo::TodoCall::Set { id: None, text: Some("beat Brock".into()) });
        assert_eq!(system_message(), before, "writing a TODO must not touch the cacheable prefix");
        assert_eq!(before.text().expect("prose"), SYSTEM_PROMPT);

        let plan = plan_message(&todo);
        assert!(plan.text().expect("prose").contains("beat Brock"), "{plan:?}");
        // ⚠️ **The history can hold several of these**, because `sync_plan` appends rather than
        // moving — so the message has to say which one wins, and chronology is the only answer a
        // conversation implies on its own.
        assert!(plan.text().expect("prose").contains("replaces any earlier"), "{plan:?}");
        assert!(is_plan(&plan), "the worker finds the newest copy with this");
        assert!(!crate::llm::compaction::is_turn_start(&plan),
                "a cut between the plan and its turn would drop the one thing meant to survive");
        assert!(!is_plan(&crate::llm::protocol::Message::user("Location: PalletTown")),
                "an ordinary turn is not a plan");
    }

    /// ⚠️ **Every script state is said on the overworld turn, because that is the only kind of turn
    /// carrying the three tools that change one.** `Armed` has no other carrier at all: a working
    /// script produces no battle turns, so its line reached the model only on a Safari turn or a
    /// `battle.ask()`, and the 2026-09-01 run armed one that fought a Fire starter into Brock's
    /// Rock gym on Ember, blacked out twice, and went ~80 overworld turns without being told a
    /// script existed. `Unedited` and `Disarmed` were the same hole facing the other way — said on
    /// every battle turn, where `tools::offers_battle_script` offers nothing to act with. The same
    /// run, disarmed at 22:06:33 UTC on 2026-09-01: **58 of 58 battle turns carried the warning, 0
    /// of 83 overworld turns did**, and neither `read_battle_script` nor `set_battle_script` was
    /// called in the 25 minutes and 58 paid battle turns that followed.
    #[test]
    fn every_battle_script_state_is_said_on_the_turn_that_can_do_something_about_it() {
        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/at-vermilion.bin")).expect("the fixture loads");
        let state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("the fixture has a readable state");

        let overworld = |script| situation(
            DecisionKind::Overworld, &state, &ApiSnapshot::default(), &[], &[],
            TurnContext::Overworld { script, standing: &Default::default(), guide: crate::llm::guide::GuideStatus::Current }, &[],
        );

        let armed = overworld(ScriptState::Armed);
        assert!(armed.contains("battle script is armed"), "{armed}");
        assert!(armed.contains("read_battle_script"), "and how to look at it: {armed}");
        assert!(armed.contains("set_battle_script"), "and how to replace it: {armed}");
        // ⚠️ **It has to say that a bad battle is the script's doing rather than the game's.** The
        // report is the only account of a scripted battle, and a model that cannot connect a lost
        // Pokémon to the program that lost it has no reason to open the program.
        assert!(armed.contains("that is the script doing it"), "{armed}");

        // ⚠️ **The other two are said here too, and each has to claim the tools are on *this*
        // turn.** Without that clause the line is the one the model has already read on every
        // battle turn, and the thing it did with that line was defer it.
        for faulted in [ScriptState::Unedited, ScriptState::Disarmed] {
            let other = overworld(faulted);
            assert!(other.contains("battle script"), "{faulted:?} is silent here: {other}");
            assert!(other.contains("read_battle_script") && other.contains("set_battle_script"),
                    "{faulted:?} names the tools it is offered: {other}");
            assert!(other.contains("this is a turn that can fix it"), "{faulted:?}: {other}");
        }

        // ⚠️ **A disarm reason is quoted here rather than left in `read_battle_script`.** It is the
        // sentence the sandbox wrote about a line of the model's own code, and the alternative was
        // a 6 kB round trip on the one turn already being asked to spend its budget writing code.
        let reason = "`battle.fight` was given slot 3, which is not a move that can be used";
        let standing = crate::llm::battle_script::ScriptStanding {
            failure: Some(reason.to_string()), ..Default::default()
        };
        let with_reason = situation(
            DecisionKind::Overworld, &state, &ApiSnapshot::default(), &[], &[],
            TurnContext::Overworld {
                script: ScriptState::Disarmed,
                standing: &standing,
                guide: crate::llm::guide::GuideStatus::Current,
            },
            &[],
        );
        assert!(with_reason.contains(reason), "{with_reason}");
        // ⚠️ **And a run resumed from before the field existed says the fact without inventing a
        // reason**, rather than rendering an empty "It stopped because: ".
        let without = overworld(ScriptState::Disarmed);
        assert!(!without.contains("stopped because"), "no reason, no clause: {without}");
        assert!(without.contains("read_battle_script"), "and the tool that has it is still named: {without}");
    }

    /// ⚠️ **A badge is the only thing that changes what `read_guide` answers, and nothing used to
    /// say when it had.** `guide::chapter` is keyed on the first badge the player is missing, so
    /// every read between two badges returns a word-for-word copy and a nudge to re-read would be
    /// asking for the same bytes twice — but winning one swaps the chapter out in the same instant.
    /// The 2026-09-01 run read the guide once on turn 1, won the Boulder Badge 39 minutes later,
    /// and went on playing out of the chapter about how to beat Brock.
    #[test]
    fn a_badge_says_the_walkthrough_chapter_has_moved_on_under_the_model() {
        use crate::llm::guide::{status, GuideStatus};
        use crate::pokemon::badge::Badge;

        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/at-vermilion.bin")).expect("the fixture loads");
        let state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("the fixture has a readable state");

        let overworld = |guide| situation(
            DecisionKind::Overworld, &state, &ApiSnapshot::default(), &[], &[],
            TurnContext::Overworld { script: ScriptState::Unedited, standing: &Default::default(), guide }, &[],
        );

        // Read while the player still had no badges, now holding at least the first: the chapter
        // the model is working from is one it has already finished.
        let stale = status(state.badges, Some(0));
        assert!(matches!(stale, GuideStatus::Stale { .. }), "the fixture is past chapter 0: {stale:?}");
        let rendered = overworld(stale);
        assert!(rendered.contains("won a badge since you last read"), "{rendered}");
        assert!(rendered.contains("read_guide"), "and the tool that fixes it: {rendered}");
        // ⚠️ **It names the chapter's subject rather than saying "a different one".** "Go and read
        // it" is a chore; "the chapter about winning the ThunderBadge" is a reason.
        assert!(
            rendered.contains(&crate::llm::guide::chapter_goal(crate::llm::guide::chapter_index(state.badges))),
            "{rendered}",
        );

        // ⚠️ **Silent while the chapter is the one it read**, or the line is on every turn of the
        // run and is the thing a model learns to skip. Silent when it has never read one, too:
        // see `GuideStatus::Current`.
        for quiet in [status(state.badges, Some(crate::llm::guide::chapter_index(state.badges))), status(state.badges, None)] {
            let other = overworld(quiet);
            assert!(!other.contains("walkthrough"), "{quiet:?}: {other}");
        }

        // ⚠️ **The nudge is an overworld thing.** There is nothing to be done about a chapter in the
        // middle of a battle or on a naming screen, and those turns carry no `Overworld` context at
        // all — which is the property being asserted, since a fifth kind added later would have to
        // opt in rather than inherit it.
        let elsewhere = situation(
            DecisionKind::Battle, &state, &ApiSnapshot::default(), &[], &[],
            TurnContext::Battle { script: ScriptState::Armed }, &[],
        );
        assert!(!elsewhere.contains("walkthrough"), "{elsewhere}");

        // The Elite Four is chapter 8 and has no badge to name.
        assert_eq!(crate::llm::guide::chapter_goal(8), "the Elite Four");
        assert_eq!(crate::llm::guide::chapter_goal(1), format!("the {}", Badge::CascadeBadge));
    }

    /// **What the map will not let you do is said once, in the turn.**
    ///
    /// An action whose only follow-up the game refuses is withheld from the menu
    /// (`MetaTileMap::can_cut` / `can_surf`), which is what stops a cut with no Cut becoming sixty
    /// seconds of A-mashing in a party menu. But withholding it *silently* is the same bug facing
    /// the other way: the deployed run, having found no way north out of Route 2, went round the
    /// same four maps for forty turns and filed three issue reports saying the game was broken. So
    /// the obstacle is named, with what it takes to pass it, and only while it is actually blocking.
    ///
    /// ⚠️ **A cuttable tree is the only obstacle that earns the line, and water is the counterexample
    /// rather than the other half of the pair.** Both halves are asserted here, because "name what
    /// the party cannot pass" is the kind of rule that grows an entry back.
    #[test]
    fn an_obstacle_the_party_cannot_pass_is_named_rather_than_silently_dropped() {
        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/at-vermilion.bin")).expect("the fixture loads");
        let mut state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("the fixture has a readable state");
        assert!(!state.can_use_cut, "the fixture reaches Vermilion before the HM");

        let rendered = |kind, state: &GameState| situation(
            kind, state, &ApiSnapshot::default(), &[], &[], TurnContext::None, &[],
        );
        let blocked = rendered(DecisionKind::Overworld, &state);
        assert!(blocked.contains("Blocked here: Cuttable trees"), "{blocked}");
        assert!(blocked.contains("CascadeBadge"), "it has to say what would clear them: {blocked}");

        // ⚠️ **Water is not one of these and must not come back.** It was the second obstacle for a
        // while, and it named a wall that was almost never the wall: this fixture stands on a coast
        // with no Surf, and nothing about that sea is stopping it going anywhere. The 2026-09-02
        // deployed run read the line as the reason it could not get through Rock Tunnel — its own
        // summaries say "water-block geometry" and "the wrong side of water" — and spent 65 turns
        // going in and out of the entrance while the ladder it had never taken sat in the menu.
        // ⚠️ **The precondition is asserted first**, or this passes the day the fixture stops having
        // any water in it and proves nothing at all.
        assert!(!state.can_use_surf, "the fixture reaches Vermilion before Surf");
        assert!(state.map.meta_tiles.iter().any(|tile| matches!(
                    tile, crate::pokemon::tile::MetaTile::Water
                        | crate::pokemon::tile::MetaTile::ConnectionWater(_))),
                "Vermilion is on the coast, so a water line would have fired here");
        assert!(!blocked.contains("Blocked here: Water"), "water is scenery, not an errand: {blocked}");

        // ⚠️ **And only on the turn that has an action menu.** It is a fact about that menu; on a
        // battle turn or a naming screen it is overworld trivia in the middle of another question.
        for elsewhere in [DecisionKind::Battle, DecisionKind::Nickname, DecisionKind::Stuck] {
            let other = rendered(elsewhere, &state);
            assert!(!other.contains("Blocked here"), "{elsewhere:?} has no action menu: {other}");
        }

        // ⚠️ **Which half is missing is the *point* of the line, and getting it wrong sends the
        // model at the half that is not.** This fixture holds the CascadeBadge and no HM01, so the
        // errand is the HM; saying "and needs the CascadeBadge" here would be advice to go and win a
        // badge already on the trainer card.
        assert!(blocked.contains("not the HM"), "the badge is held; the HM is the errand: {blocked}");

        // ⚠️ **The case the deployed run of 2026-08-27 was actually in, and the one nothing said out
        // loud**: HM01 in the bag, the badge won, and a party with nothing in Cut's learnset. It
        // spent its life re-issuing a teach the cartridge refuses.
        let mut hopeless = state.clone();
        hopeless.bag.push(crate::pokemon::bag::BagItem { id: crate::pokemon::item::ItemId::Hm01Cut, quantity: 1 })
            .expect("the fixture's bag has room");
        hopeless.pokemon = Default::default();
        hopeless.pokemon.push(crate::pokemon::pokemon::Pokemon::maxed(
            crate::pokemon::species::PokemonSpecies::Pidgey, "MON",
            [crate::pokemon::move_name::PokemonMoveName::Gust; 4], "AI", 1)).expect("room for one");
        let none = rendered(DecisionKind::Overworld, &hopeless);
        assert!(none.contains("nothing in your party is in its learnset"),
                "it has to name the party rather than the HM it is already holding: {none}");

        // …and with a Pokémon that *can* take it, the line becomes the tool call to make.
        let mut ready = hopeless.clone();
        ready.pokemon.get_mut(0).expect("one member").species = crate::pokemon::species::PokemonSpecies::Venusaur;
        let teachable = rendered(DecisionKind::Overworld, &ready);
        assert!(teachable.contains("`use_field_move` with `teach`"), "{teachable}");

        // ⚠️ **And it stops the moment it stops being true**, or it is a line on every turn of the
        // rest of the game telling the model about a thing it can already do.
        state.can_use_cut = true;
        let cleared = rendered(DecisionKind::Overworld, &state);
        assert!(!cleared.contains("Blocked here: Cuttable trees"), "{cleared}");
    }

    /// **The four things the system prompt has to keep saying**, each bought with a measured
    /// failure of a deployed run rather than a guess:
    ///
    /// - the game is not broken and is not being debugged — 29 of one 258-turn run's own decision
    ///   summaries called it buggy, glitched or in need of a reset;
    /// - retrying is not a plan — that run cut the same Route 2 tree eleven times;
    /// - prior knowledge of Pokémon Red is not evidence — it named Brock 88 times without having
    ///   met him, and picked a starter on turn 7 for its "type advantage over Gary's likely
    ///   Charmander";
    /// - what people say is the instruction — it read "GRAMPS ISN'T AROUND" six times in Oak's lab
    ///   and talked to the same three people for thirty turns instead of going to find him.
    ///
    /// ⚠️ **A prose test, deliberately.** None of this can be asserted on behaviour without an
    /// endpoint, and all four were *removed* by rewording at some point in this file's history.
    #[test]
    fn the_system_prompt_says_the_things_the_deployed_runs_needed_it_to_say() {
        for phrase in [
            "The game is not broken",
            // The Viridian Gym door: a locked door the run had no badges for, reported as an
            // abandoned walk, answered with a `report_issue` asking a developer to check the
            // agent's warp targeting. Nothing was wrong. This bullet and
            // `OverworldActionAbortedReason::Textbox`'s wording are the two halves of saying so.
            "Being stopped is not a malfunction",
            "Doing the same thing again is not a plan",
            "not the one you remember",
            "Read what people say to you",
            "put it on the plan straight away",
            // ⚠️ **A tool nothing tells the model to reach for is a tool nothing reaches for**, and
            // this one is worth nothing unread: `then` and `resume_after_battle` are pure saving,
            // so a run that never uses them is exactly as expensive as one without them. Both
            // deployed runs are the precedent — the plan tools were offered every turn and edited
            // once in 258 turns and sixteen times in 2430 — which is why the habit is prose here
            // and not only a sentence in the tool's own description.
            "One decision can be several actions",
        ] {
            assert!(SYSTEM_PROMPT.contains(phrase), "the system prompt no longer says {phrase:?}");
        }

        // ⚠️ **The plan bullet's claim narrowed when the conversation started surviving a restart**
        // (`llm::history`), and the narrower claim is the true one: a compaction still empties the
        // history, so the plan is still what survives *that*. Pinned here because the sentence it
        // replaced — "and a restart of the program as well" — reads perfectly well and would be an
        // easy thing to put back, at which point the prompt is telling the model something false
        // about its own memory.
        assert!(
            SYSTEM_PROMPT.contains("is what survives that. It is the only thing that does"),
            "the plan bullet no longer says what the plan is for",
        );
        assert!(
            !SYSTEM_PROMPT.contains("restart of the program"),
            "the conversation survives a restart now, so the prompt must not claim only the plan does",
        );
    }

    /// **What a run that is following every rule above still leaves out.**
    ///
    /// The four bullets that test guards all say what *not* to do. The deployed run of 2026-08-26
    /// obeyed them and was still playing badly: 92 minutes of cartridge time, one badge, and a
    /// single Lv19 starter as the whole party. Of its 204 battle decisions 31 were `run` and **none
    /// was a Poké Ball**, it bought nothing in a mart in 429 decisions, and it read the guide three
    /// times. Nothing in that is a malfunction, which is why nothing but prose can fix it.
    ///
    /// ⚠️ **The blackout bullet is a fact about the cartridge, not advice**, and the distinction it
    /// draws is the part that is easy to get wrong: `SetLastBlackoutMap` is called from
    /// `DisplayPokemonCenterDialogue_` only after the player answers *yes* to the heal, so walking
    /// into a Centre does not move where a blackout sends you. `ResetStatusAndHalveMoneyOnBlackout`
    /// is where the money goes.
    #[test]
    fn the_system_prompt_says_how_to_play_the_game_well() {
        for phrase in [
            // Ranked on `wPlayTime`, which is on every turn: finishing is the goal, not wandering.
            "You are being timed",
            "Keep the party healthy",
            // Both halves: the penalty, and *which* Centre you wake up in.
            "faints you black out",
            "accepted a heal",
            "Keep stocked up",
            "Catch Pokémon",
            "Look round a town before you leave it",
            // ⚠️ **A cartridge fact, not advice, and the run that forced it damaged a Pidgey twice
            // and then tried to flee.** `GainExperience` has exactly two call sites, both under
            // `HandleEnemyMonFainted`; the capture path returns through
            // `.returnAfterCapturingMon`, which sets `wBattleResult` and never reaches them. So a
            // fight broken off pays nothing whatever, which is the half no model infers.
            "Experience is only paid out for a knockout",
            // The walkthrough is only worth carrying if the prompt says when to reach for it.
            "There is a walkthrough for this game",
            // ⚠️ The three tools exist and are described in the catalogue, but nothing there says a
            // battle turn is *worth avoiding* — that argument only fits in prose, and a model that
            // never installs a script pays for every battle it ever has. Both halves are pinned:
            // that scripting exists, and that `battle.ask()` is how the hard fights are kept.
            "Write your battles down",
            "battle.ask()",
            // ⚠️ **The two early actions are named at the top as well as argued below, because
            // the deployed run of 2026-08-27 did neither and never reasoned about either.** Across
            // 828 `assistant_reasoning` events the strings `read_guide` and `walkthrough` appear
            // zero times, and `set_battle_script` was never called in 55 overworld turns — while
            // both were in the tool array of every one of them and argued for in the prose below.
            // Emphasis is the weak lever here (`press_buttons`' "a last resort" moved nothing);
            // what this adds is a concrete *when*, which is the shape that worked for the nickname.
            "Everything below is instruction rather than background",
        ] {
            assert!(SYSTEM_PROMPT.contains(phrase), "the system prompt no longer says {phrase:?}");
        }
    }

    /// ⚠️ **A party line that names only the nickname stops naming the Pokémon at all.** The
    /// naming screen asks, in as many words, for a name that says what the model makes of *this*
    /// one rather than the species again — so the better it answers that prompt, the more
    /// completely the species disappears from every turn for the rest of the run, and the only way
    /// back was `read_party`, a whole round trip whose remaining content is species, types and
    /// stats. The types are the other half: the prompt tells the model a party covering several
    /// types is what gets through a gym, and then gave it no way to check without buying a read.
    #[test]
    fn a_party_line_names_the_species_and_its_types() {
        let mut fixture = crate::pokemon::integration_tests::fixture::TestFixture::new(
            include_bytes!("../pokemon/data/at-celadon.bin"),
            std::time::Duration::from_secs(10),
            vec![],
        );
        let state = fixture.game_state();
        let snapshot = ApiSnapshot::read(&fixture.api());
        let turn = situation(
            DecisionKind::Overworld, &state, &snapshot, &[],
            &crate::llm::tools::overworld_menu(&state, snapshot.arrival),
            TurnContext::None, &[],
        );
        let party = turn.split("### Party").nth(1).expect("a party block").lines().nth(1).unwrap();
        let mon = state.pokemon.iter().next().expect("the fixture has a party");
        assert!(party.contains(&mon.species.to_string()),
                "the species is on the line whatever the nickname is: {party}");
        assert!(party.contains(&mon.types[0].to_string()), "and so are its types: {party}");
        // ⚠️ A single-type mon stores its one type in both slots, so an undeduplicated line reads
        // `Normal/Normal` — the derive showing through, which is the bug class this file keeps
        // catching.
        assert!(!party.contains("Normal/Normal") && !party.contains("Water/Water"),
                "the duplicate type slot is folded: {party}");
    }

    /// ⚠️ **The one turn whose entire question is "what am I short of" kept the answer behind a
    /// read.** This module's own rule is that anything a read can answer from the situation belongs
    /// in the situation; a mart turn broke it, so playing the turn properly cost a `read_bag` round
    /// trip every time, and the deployed run bought nothing at a mart across 429 decisions. Zero is
    /// printed rather than omitted: "you have none" is the row that decides a purchase.
    #[test]
    fn a_mart_row_says_how_many_you_already_have() {
        use crate::pokemon::item::ItemId;
        let mut fixture = crate::pokemon::integration_tests::fixture::TestFixture::new(
            include_bytes!("../pokemon/data/at-celadon.bin"),
            std::time::Duration::from_secs(10),
            vec![],
        );
        let state = fixture.game_state();
        let mut snapshot = ApiSnapshot::read(&fixture.api());
        snapshot.mart_stock = vec![(ItemId::PokeBall, Some(200)), (ItemId::Potion, Some(300))];
        for row in crate::llm::tools::mart_menu(&snapshot, &state) {
            assert!(row.description.contains("you have"),
                    "every row says the holding, zero included: {}", row.description);
        }
    }

    /// A scrolling conversation emits the same line repeatedly. Twenty identical sentences is not
    /// twenty things that happened.
    #[test]
    fn repeated_text_boxes_collapse_and_the_tail_is_kept() {
        let mut events = vec![describe_event(&AgentEvent::BattleStarted)];
        for _ in 0..5 {
            events.push(describe_event(&AgentEvent::TextBox { message: "OAK: Hello!".into() }));
        }
        events.push(describe_event(&AgentEvent::TextBox { message: "OAK: Goodbye!".into() }));
        assert_eq!(summarise_events(&events), [
            "battle started",
            "Text: OAK: Hello!",
            "Text: OAK: Goodbye!",
        ]);

        // …and the cap keeps the most recent, because the recent ones are the ones that explain now.
        let many: Vec<String> = (0..40)
            .map(|i| describe_event(&AgentEvent::TextBox { message: format!("line {i}") }))
            .collect();
        let lines = summarise_events(&many);
        assert_eq!(lines.len(), MAX_EVENTS);
        assert_eq!(lines.last().unwrap(), "Text: line 39");
    }

    /// An abort reason is the single most useful thing the agent can tell a model — it is what stops
    /// it re-picking a route that cannot be walked — so it must survive into the turn.
    #[test]
    fn an_abort_reason_reaches_the_turn() {
        let events = [describe_event(&AgentEvent::OverworldActionAborted {
            destination: MetaTile::Grass,
            reason: OverworldActionAbortedReason::NoRoute(MetaTile::Grass),
            at: None,
        })];
        assert!(summarise_events(&events)[0].contains("no route"), "{:?}", summarise_events(&events));
    }
}

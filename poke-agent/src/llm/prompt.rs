use crate::llm::battle_script::ScriptState;
use crate::llm::tools::{DecisionKind, MenuItem, terminal_names};
use crate::pokemon::battle::is_ghost_battle;
use crate::pokemon::GameState;
use crate::pokemon::agent::AgentEvent;

/// Index 0 of the history, byte-identical for the whole run because a prompt cache keys on it.
pub fn system_message() -> crate::llm::protocol::Message {
    crate::llm::protocol::Message::system(SYSTEM_PROMPT)
}

/// The model's plan, as the message that carries it.
pub fn plan_message(todo: &crate::llm::todo::TodoList) -> crate::llm::protocol::Message {
    crate::llm::protocol::Message::user(todo.render())
}

/// Appended to the situation when the plan is not re-sent with it.
pub const PLAN_UNCHANGED: &str =
    "Your plan is unchanged since the last `## Your plan` message in this conversation — the one \
     nearest the end — and that copy is still the current one. Read it back before you decide; if it \
     no longer describes what you are doing, fix it with `todo_set` or `todo_complete` in this same \
     turn.";

/// Appended once by [`History::open`](crate::llm::history::History::open) to a restored history.
pub const RESUMED_NOTE: &str =
    "The program was restarted and this conversation was restored from disk. The game itself \
     resumed from its last save point, which may be up to a minute behind the last thing said \
     above. So the action you took in your most recent turn may not have happened, and a few \
     seconds of play may have been replayed. If what you are shown now does not match what you \
     thought you had just done, the save point is right and this conversation is ahead of it. \
     Nothing is broken and there is nothing to undo. Read the situation below and carry on from \
     where the game actually is.";

/// What the first turn after `POST /api/clear` opens on.
pub const CLEARED_NOTE: &str =
    "Your conversation and your plan were deliberately cleared by whoever runs this program. \
     The game itself was not touched and is exactly where you left it, possibly hours into a run \
     you can no longer remember any of. Nothing is broken, nothing was lost from the game, and \
     there is nothing to undo or repeat. Your battle script was not touched either, and the turn \
     below says what it is doing. Work out where you are from the situation below, use `read_map` \
     if you need to, and write yourself a fresh plan before you go far.";

/// Whether this is a message [`plan_message`] produced.
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
  back, with two exceptions the turn tells you about where they matter: a cut tree grows back, and \
  a floor's boulders go back to where they started, which is the only way to undo a Strength puzzle \
  you have pushed into a corner.

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

/// What a turn says about an armed battle script.
fn script_standing_line(standing: &crate::llm::battle_script::ScriptStanding) -> String {
    let mut out = String::from(
        "Your battle script is armed and is deciding your battle turns, so the battles going \
         past are not being put to you and are costing you nothing. A battle report is the only \
         account you get of what it chose. If one shows it losing a Pokémon, fleeing from \
         something worth catching, or attacking with a move the enemy shrugs off, that is the \
         script doing it and not the game: `read_battle_script` shows what it says and \
         `set_battle_script` replaces it.",
    );

    // Quoted, in the model's own words.
    if let Some(purpose) = standing.purpose.as_deref() {
        out.push_str(&format!(" You installed it for: \"{purpose}\"."));
    }
    out.push_str(&match standing.decided {
        0 => " It has not decided a battle turn yet.".to_string(),
        1 => " It has decided 1 battle turn since you installed it.".to_string(),
        n => format!(" It has decided {n} battle turns since you installed it."),
    });
    // Said once, at the end, and as a question rather than an instruction.
    out.push_str(
        " It will go on deciding every battle until you replace it, so if that is no longer the \
         kind of fight you are in, this is the moment to say so.\n\n",
    );
    out
}

/// What the overworld turn says about the battle script, for every state it can be in.
fn overworld_script_line(
    state: ScriptState,
    standing: &crate::llm::battle_script::ScriptStanding,
) -> String {
    // Each state has to say the three tools are on this turn.
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
                // Quoted rather than paraphrased.
                Some(why) => match why.trim_end() {
                    cut if cut.ends_with('…') => format!(" It stopped because: {cut}"),
                    said => format!(" It stopped because: {}.", said.trim_end_matches('.')),
                },
                // A disarm recorded without a reason.
                None => String::new(),
            },
        ),
    }
}

/// The line that ends every turn request, which is why the loop can rely on one terminal call.
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiSnapshot {
    /// `None` in the overworld, where no dialogue font is loaded to decode.
    pub screen_text: Option<String>,
    /// `HH:MM:SS` of in-game play time.
    pub playtime: String,
    /// The mart's stock and prices, from `wCurMart`: a `MartPurchase` turn's whole menu, which
    /// `GameState` does not carry.
    pub mart_stock: Vec<(crate::pokemon::item::ItemId, Option<u32>)>,
    /// `WorldGraph::arrival`, known only to the agent; `None` until this process sees a map change.
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

/// How many `AgentEvent`s one turn request carries.
const MAX_EVENTS: usize = 20;

/// One [`AgentEvent`] as the model will read it.
pub fn describe_event(event: &AgentEvent) -> String {
    match event {
        AgentEvent::TextBox { message } => format!("Text: {}", message.trim()),
        other => format!("{other}"),
    }
}

/// The part of a question the agent passed to `pick_*` rather than left in the [`GameState`].
#[derive(Debug, Clone, Copy, Default)]
pub enum TurnContext<'a> {
    #[default]
    None,
    Nickname(crate::pokemon::species::PokemonSpecies),
    ForgetMove {
        /// Which party member is learning it.
        slot: usize,
        current: &'a [crate::pokemon::move_name::PokemonMove],
        new: crate::pokemon::move_name::PokemonMoveName,
    },
    /// The watchdog's turn: what the agent believes it is doing, and for how long.
    Stuck { agent_state: &'a str, stuck_for: std::time::Duration },
    /// Whether a battle script is deciding battles, and whether the walkthrough chapter has moved.
    Overworld {
        script: ScriptState,
        standing: &'a crate::llm::battle_script::ScriptStanding,
        guide: crate::llm::guide::GuideStatus,
    },
    /// Whether a script is deciding battle turns, on a turn it did not decide.
    Battle { script: ScriptState },
}

/// The user message that opens a turn.
pub fn situation(
    kind: DecisionKind,
    state: &GameState,
    snapshot: &ApiSnapshot,
    events: &[String],
    menu: &[MenuItem],
    context: TurnContext<'_>,
    // Battles the script fought without asking — see `crate::llm::battle_report`.
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
        // At the top of the turn, above the situation, where a nudge is read.
        TurnContext::Overworld { script, standing, guide } => {
            out.push_str(&overworld_script_line(script, standing));
            // Only the stale case: a run that has never read the guide is not nagged.
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
            // No article, or it reads "a Eevee".
            "The naming screen is open for {species}. It has just been caught, hatched or given \
             to you.\n\n\
             Name it. Not the species again — a name that says what you make of *this* one: how you \
             came by it, what you plan to do with it, what the fight it came out of was like, what \
             it reminds you of. It is the name you will read in every message about it from here on, \
             so pick one you will recognise. Keep the default only if you truly have nothing to \
             say.\n\n",
        )),
        // Three things this turn did not say, each of which decides it.
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
        // At the top rather than under `### Battle`, where a nudge is read.
        TurnContext::Battle { script } => out.push_str(match script {
            ScriptState::Unedited => {
                "⚠️ Your battle script is still the default one, which decides nothing and hands \
                 every battle straight back to you, so this turn costs a request exactly like \
                 every other battle turn. Your next overworld turn carries the tools that change \
                 that, and says so.\n\n"
            }
            ScriptState::Disarmed => {
                "⚠️ Your battle script failed and is no longer deciding your battle turns, so this \
                 one is costing you a request again. Your next overworld turn carries the reason \
                 it stopped and the tools to fix it.\n\n"
            }
            // Armed but silent: a Safari battle, or a turn the model asked to `wait` through.
            ScriptState::Armed => {
                "Your battle script is armed and deciding your battle turns, but it did not decide \
                 this one.\n\n"
            }
        }),
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
    // Which door the player came in by.
    if let Some(arrival) = snapshot.arrival.filter(|a| a.map == state.map.map) {
        out.push_str(&match arrival.from {
            Some(from) => format!("Entered this map at ({}, {}) from {from}\n", arrival.at.x, arrival.at.y),
            None => format!("Entered this map at ({}, {})\n", arrival.at.x, arrival.at.y),
        });
    }
    // Half of `use_field_move` acts on the tile in front.
    if let Some((at, tile)) = state.map.tile_in_front() {
        out.push_str(&format!("Facing: {tile} at ({}, {})\n", at.x, at.y));
    }
    // What the menu does not offer still has to be explainable.
    if kind == DecisionKind::Overworld {
        use crate::pokemon::badge::Badge;
        use crate::pokemon::item::ItemId;
        use crate::pokemon::learnset::can_learn;
        use crate::pokemon::tile::MetaTile;
        // Where the trees actually are, which the line below either offers or explains.
        let trees: Vec<poke_core::geometry::Point8> = state.map.meta_tiles.iter().enumerate()
            .filter(|(_, tile)| **tile == MetaTile::CutTree)
            .map(|(index, _)| poke_core::geometry::Point8 {
                x: (index % state.map.width) as u8, y: (index / state.map.width) as u8 })
            .collect();
        if !trees.is_empty() && state.can_use_cut {
            out.push_str(&format!(
                "Cuttable trees on this map: {}. Each one you can reach is a row in the menu below \
                 and choosing it does the whole thing — walk over, use Cut, tree gone. A tree \
                 grows back when the map is reloaded, which a black-out or a trip through a door \
                 does.\n",
                point_list(&trees),
            ));
        }
        // Only what would clear a cuttable tree: naming water fired on every coast.
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
                "Blocked here: {noun} on this map cannot be passed yet, at {}. That needs {what}. \
                 Nothing in the menu below leads past it, and retrying will not change that.\n",
                point_list(&trees),
            ));
        }

        // Boulders, said every turn there are any, whether or not they can be moved.
        let boulders = state.map.boulders();
        if !boulders.is_empty() {
            out.push_str(&format!("Boulders on this map: {}.", point_list(&boulders)));
            if !state.map.strength_switches.is_empty() {
                out.push_str(&format!(" Boulder switches (push a boulder onto one to open the \
                    barrier it controls): {}.", point_list(&state.map.strength_switches)));
            }
            // The third kind of target, and the whole puzzle on the map that has them.
            if !state.map.holes.is_empty() {
                out.push_str(&format!(" Holes a boulder can be pushed into, dropping it to the \
                    floor below: {}. You fall through one yourself if you step on it.",
                    point_list(&state.map.holes)));
            }
            let known = state.pokemon.iter()
                .any(|mon| mon.moves.iter().flatten().any(|m| m.name == crate::pokemon::move_name::PokemonMoveName::Strength));
            let badged = state.badges.contains(Badge::RainbowBadge);
            // Counted off `actions()`, never `boulder_pushes()`: a floor can have legal shoves
            // left and still offer no goal.
            let boulder_goals = state.map.actions().iter()
                .filter(|action| matches!(action.tile,
                    crate::pokemon::tile::MetaTile::BoulderGoal { .. })).count();
            out.push_str(&match (known, badged) {
                // Zero rows is not "every legal shove is below".
                (true, true) if boulder_goals == 0 => " There are no boulder rows in the menu below. \
                    A boulder row is a whole job rather than a shove, and one is offered only when \
                    the pushes that finish it can be worked out from where the boulders are \
                    standing; none of these has one. Leaving the map and coming back puts every \
                    boulder on it back where it started.".to_string(),
                (true, true) => " Every boulder that can be pushed onto a switch or into a hole from \
                    where it is standing is a row in the menu below, one row per target, and the row \
                    names the boulder it will use. Choosing one does the whole job: it walks over, \
                    arms Strength for you, and keeps pushing until that boulder is on that target, \
                    however many shoves and however much walking round that takes. A target no \
                    boulder can reach is not offered rather than failing quietly. Leaving the map \
                    and coming back puts every boulder on it back where it started, which is how a \
                    puzzle that went wrong is undone.".to_string(),
                (false, true) => format!(
                    " No Pokémon in your party knows Strength, so there are no boulder actions in \
                     the menu below. {} You have the {}, so a Pokémon taught HM04 is all this needs.",
                    match state.bag.iter().any(|entry| entry.id == ItemId::Hm04Strength) {
                        true => "HM04 is in your bag.",
                        false => "HM04 has to be found first.",
                    },
                    Badge::RainbowBadge),
                (true, false) => format!(
                    " Strength needs the {} before the game will let it be used outside battle and \
                     you do not have it yet, so there are no boulder actions in the menu below.",
                    Badge::RainbowBadge),
                (false, false) => format!(
                    " Nothing in your party knows Strength and you do not have the {} either, so \
                     there are no boulder actions in the menu below and both have to come first.",
                    Badge::RainbowBadge),
            });
            out.push('\n');
        }

        if crate::pokemon::tile_map::hidden_objects_for(state.map.map).iter()
            .any(|site| site.object == crate::pokemon::tile::HiddenObject::TrashCan) {
            out.push_str(&format!(
                "The bins in this gym are a two-switch puzzle: one bin hides the first switch, and \
                    opening it puts the second in another bin. Two things are worth knowing before you \
                    start. Checking the wrong bin for the *second* switch resets both locks and moves \
                    the first switch somewhere new, so every bin you had eliminated goes back into the \
                    pool and a careful sweep can undo itself. And `choose_action` takes {} more ids in \
                    `then`, all carried out without asking you again, so a sweep is {} bins per request \
                    rather than one.\n",
                crate::llm::tools::MAX_CHAINED_ACTIONS - 1,
                crate::llm::tools::MAX_CHAINED_ACTIONS,
            ));
        }

        let doors: std::collections::HashSet<crate::pokemon::map::Map> =
            state.map.warp_targets.iter().map(|(to_map, _)| *to_map).collect();
        if state.map.warp_targets.len() > 1 && doors.len() == 1 {
            let to_map = doors.into_iter().next().expect("one");
            out.push_str(&format!(
                "Every way off this map leads back to {to_map}, at {} different places on it, so \
                    which one you take decides where you come out and they are not interchangeable — \
                    each row below says where it lands. Nothing beyond {to_map} can be reached through \
                    a door from here; to get anywhere further you leave onto {to_map} and go on from \
                    there.\n",
                state.map.warp_targets.len(),
            ));
        }
    }
    let badges: Vec<String> = state.badges.iter_names().map(|(name, _)| name.to_string()).collect();
    // Coins only once there is somewhere to keep them.
    let coins = match state.bag.iter().any(|item| item.id == crate::pokemon::item::ItemId::CoinCase) {
        true => format!("   Coins: {}", state.coins),
        false => String::new(),
    };
    out.push_str(&format!(
        "Badges: {}\nMoney: ¥{}{coins}   Play time: {}\n",
        if badges.is_empty() { "none yet".to_string() } else { badges.join(", ") },
        state.money,
        snapshot.playtime,
    ));
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
        // `Display`, not `{:?}`.
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
        // Say why the menu has one row, or it reads as the agent being broken.
        if is_ghost_battle(state.map.map, &state.bag, battle.battle_type) {
            out.push_str("⚠️ This is a GHOST: no move, ball or switch does anything here until you \
                          are carrying the Silph Scope (it is in the Rocket Hideout, under the Game \
                          Corner in Celadon). Running always works. Nothing is broken.\n");
        }
        if battle.enemy_trapping {
            // Every option still looks available, but a move chosen is replaced with "cannot move".
            out.push_str("⚠️ You are trapped (Wrap/Bind/Fire Spin): a move will not execute this \
                          turn, but items, switching and running still work.\n");
        }
    }

    // Above `### On screen` and below `### Battle`, which is where "what just happened" goes.
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

/// The nickname, and the species too when they differ.
fn named(mon: &crate::pokemon::pokemon::Pokemon) -> String {
    let nickname = mon.nickname.to_default_string();
    let species = mon.species.to_string();
    match nickname.eq_ignore_ascii_case(&species) {
        true => nickname,
        false => format!("{nickname} the {species}"),
    }
}

/// A list of squares as `(x, y), (x, y)`, bounded.
fn point_list(points: &[poke_core::geometry::Point8]) -> String {
    /// Enough for every Strength puzzle in the game and the trees on a route.
    const MAX: usize = 8;
    let shown: Vec<String> = points.iter().take(MAX)
        .map(|point| format!("({}, {})", point.x, point.y)).collect();
    match points.len() > MAX {
        false => shown.join(", "),
        true => format!("{}, and {} more", shown.join(", "), points.len() - MAX),
    }
}

/// A party member's types, deduplicated: a single type is stored in both slots.
fn types_of(mon: &crate::pokemon::pokemon::Pokemon) -> String {
    let mut types: Vec<String> = mon.types.iter().map(|t| t.to_string()).collect();
    types.dedup();
    types.join("/")
}

/// A status worth mentioning, or nothing at all.
fn ailment(status: crate::pokemon::status::PokemonStatus) -> String {
    match status {
        crate::pokemon::status::PokemonStatus::None => String::new(),
        other => format!(", {other}"),
    }
}

/// The events since the last turn, most useful last.
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
pub const OUT_OF_STEPS: &str =
    "You have used every read this turn. Call a terminal tool now to end the turn.";

pub fn nudge(kind: DecisionKind) -> String {
    format!(
        "That reply contained no tool call, so nothing happened in the game. {}",
        contract(kind),
    )
}

/// [`nudge`] for a reply cut off at `GB_MAX_TOKENS` rather than finished.
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

    /// Every decision kind's first request, written out whole for a person to read.
    #[cfg(feature = "slow-tests")]
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
            target: None,
        }, vec!["8/25 HP, worth a ball".to_string()]);
        report.said("Darn! The POKéMON broke free!");

        let fourth = at(39, 8);
        report.decided(&fourth, &fight(PokemonMoveName::Ember), Vec::new());
        report.said("Enemy RATTATA fainted!");
        report.said("SPARKY gained 56 EXP. Points!");

        vec![report.finish(Some(&at(39, 0)))]
    }

    #[cfg(feature = "slow-tests")]
    #[test]
    #[ignore = "probe: prints one turn request per decision kind"]
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

        // A mid-game save: a full bag and party, and a town whose menu is a realistic length.
        let overworld = include_bytes!("../pokemon/data/at-celadon.bin");
        let battle = include_bytes!("../pokemon/data/battle-state.bin");

        // What the agent said since the last decision.
        let events: Vec<String> = [
            AgentEvent::StartedOverworldAction {
                destination: MetaTile::Sprite("Gym Guide"),
                id: "ViridianGym:GymGuide".to_string(),
            },
            AgentEvent::OverworldInteractionCompleted { target: MetaTile::Sprite("Gym Guide") },
            AgentEvent::TextBox { message: "Hey! You look weak! Let me give you some advice!".into() },
            AgentEvent::OverworldActionAborted {
                destination: MetaTile::Warp { to_map: crate::pokemon::map::Map::CeladonGym, to_position: poke_core::geometry::Point8 { x: 4, y: 17 } },
                reason: OverworldActionAbortedReason::Textbox,
                at: Some(poke_core::geometry::Point8 { x: 8, y: 19 }),
            },
        ]
        .iter()
        .map(describe_event)
        .collect();

        // A plan with something in it, as a message of its own (see `worker::sync_plan`).
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

            // The two facts no fixture carries.
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

            // A standing the armed line can actually say something with.
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
                // The default script, which is what a battle turn costs without one.
                DecisionKind::Battle => TurnContext::Battle { script: ScriptState::Unedited },
                // Both overworld notes at once: the dearest this turn gets.
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

            // In the order `worker::run_one` appends them: system message, plan, situation.
            let messages = vec![
                system_message(),
                plan_message(&todo),
                // A battle report on the battle turn, the one block nothing else prints.
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

    /// The request as something to read: messages with newlines intact, then one block per tool.
    #[cfg(feature = "slow-tests")]
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

    /// An [`LlmConfig`](crate::llm::LlmConfig) that does not read the environment.
    #[cfg(feature = "slow-tests")]
    struct LlmConfigForProbe {
        model: String,
        temperature: f32,
        max_tokens: Option<u32>,
    }

    #[cfg(feature = "slow-tests")]
    impl Default for LlmConfigForProbe {
        fn default() -> Self {
            Self {
                model: "gpt-5".to_string(),
                temperature: 1.0,
                max_tokens: Some(crate::llm::config::DEFAULT_MAX_TOKENS),
            }
        }
    }

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

    /// The system message is a constant, and this is the test that keeps it one.
    #[test]
    fn the_system_message_never_changes_and_the_plan_is_not_in_it() {
        let mut todo = crate::llm::todo::TodoList::open(None);
        let before = system_message();
        todo.apply(crate::llm::todo::TodoCall::Set { id: None, text: Some("beat Brock".into()) });
        assert_eq!(system_message(), before, "writing a TODO must not touch the cacheable prefix");
        assert_eq!(before.text().expect("prose"), SYSTEM_PROMPT);

        let plan = plan_message(&todo);
        assert!(plan.text().expect("prose").contains("beat Brock"), "{plan:?}");
        // `sync_plan` appends rather than moves, so the message says which copy wins.
        assert!(plan.text().expect("prose").contains("replaces any earlier"), "{plan:?}");
        assert!(is_plan(&plan), "the worker finds the newest copy with this");
        assert!(!crate::llm::compaction::is_turn_start(&plan),
                "a cut between the plan and its turn would drop the one thing meant to survive");
        assert!(!is_plan(&crate::llm::protocol::Message::user("Location: PalletTown")),
                "an ordinary turn is not a plan");
    }

    /// Every script state is said on the overworld turn, the only one carrying the tools.
    #[test]
    fn every_battle_script_state_is_said_on_the_turn_that_can_do_something_about_it() {
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
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
        // It has to say that a bad battle is the script's doing rather than the game's.
        assert!(armed.contains("that is the script doing it"), "{armed}");

        // The other two are said here too, and each has to claim the tools are on this turn.
        for faulted in [ScriptState::Unedited, ScriptState::Disarmed] {
            let other = overworld(faulted);
            assert!(other.contains("battle script"), "{faulted:?} is silent here: {other}");
            assert!(other.contains("read_battle_script") && other.contains("set_battle_script"),
                    "{faulted:?} names the tools it is offered: {other}");
            assert!(other.contains("this is a turn that can fix it"), "{faulted:?}: {other}");
        }

        // A disarm reason is quoted here rather than left in `read_battle_script`.
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
        // A disarm with no recorded reason invents none.
        let without = overworld(ScriptState::Disarmed);
        assert!(!without.contains("stopped because"), "no reason, no clause: {without}");
        assert!(without.contains("read_battle_script"), "and the tool that has it is still named: {without}");
    }

    fn state_from(fixture: &[u8]) -> GameState {
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(fixture).expect("the committed fixture loads");
        { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("the fixture has a readable state")
    }

    fn overworld_turn(state: &GameState, menu: &[MenuItem]) -> String {
        situation(DecisionKind::Overworld, state, &ApiSnapshot::default(), &[], menu,
                  TurnContext::None, &[])
    }

    #[test]
    fn the_coins_are_shown_once_there_is_a_coin_case() {
        use crate::pokemon::item::ItemId;
        let mut state = state_from(include_bytes!("../pokemon/data/postgame-game-corner.bin"));
        state.coins = 180;
        let has_case = state.bag.iter().any(|item| item.id == ItemId::CoinCase);
        assert!(has_case, "the fixture holds the Coin Case");
        assert!(overworld_turn(&state, &[]).contains("Coins: 180"));
        let mut without = state_from(include_bytes!("../pokemon/data/at-vermilion.bin"));
        without.coins = 180;
        assert!(!overworld_turn(&without, &[]).contains("Coins:"), "no case, no coins line");
    }

    /// A map whose every door comes out on one other map says so.
    #[test]
    fn a_map_whose_every_door_comes_out_in_one_place_says_which_door_is_which() {
        use crate::pokemon::map::Map;
        let base = state_from(include_bytes!("../pokemon/data/at-vermilion.bin"));

        // A gate: several landings, all on one map.
        let mut gate = base.clone();
        gate.map.warp_targets = [
            (Map::Route7, poke_core::geometry::Point8 { x: 11, y: 10 }),
            (Map::Route7, poke_core::geometry::Point8 { x: 18, y: 9 }),
        ].into_iter().collect();
        let turn = overworld_turn(&gate, &[]);
        assert!(turn.contains("Every way off this map leads back to Route7"), "{turn}");
        assert!(turn.contains("2 different places"), "{turn}");
        // The fact no row can carry: there is nothing else here to look for.
        assert!(turn.contains("Nothing beyond Route7 can be reached through a door from here"), "{turn}");

        // A double-wide front door is two warp tiles onto one square, and is not this.
        let mut house = base.clone();
        house.map.warp_targets = [(Map::Route7, poke_core::geometry::Point8 { x: 11, y: 10 })]
            .into_iter().collect();
        assert!(!overworld_turn(&house, &[]).contains("Every way off this map"));

        // Nor is a map whose doors go to different places.
        let mut crossroads = base.clone();
        crossroads.map.warp_targets = [
            (Map::Route7, poke_core::geometry::Point8 { x: 11, y: 10 }),
            (Map::Route8, poke_core::geometry::Point8 { x: 1, y: 9 }),
        ].into_iter().collect();
        assert!(!overworld_turn(&crossroads, &[]).contains("Every way off this map"));
    }

    /// The Vermilion Gym bins, said without giving the puzzle away.
    #[test]
    fn the_gym_bins_say_what_a_sweep_costs_without_saying_where_the_switches_are() {
        use crate::pokemon::map::Map;
        let mut state = state_from(include_bytes!("../pokemon/data/at-vermilion.bin"));
        state.map.map = Map::VermilionGym;
        assert!(crate::pokemon::tile_map::hidden_objects_for(Map::VermilionGym).iter()
                    .any(|site| site.object == crate::pokemon::tile::HiddenObject::TrashCan),
                "the bins are what the line keys on");
        let turn = overworld_turn(&state, &[]);

        assert!(turn.contains("resets both locks"), "{turn}");
        assert!(turn.contains("moves the first switch"), "the eliminations go back in the pool: {turn}");
        assert!(turn.contains("`then`"), "and what a sweep can cost instead: {turn}");

        // The turn reads `hidden_objects_for` alone, so the puzzle's hidden state cannot move it.
        let mut solved = state.clone();
        solved.trash_cans = Some(crate::pokemon::TrashCanPuzzle {
            first_target: poke_core::geometry::Point8 { x: 1, y: 7 },
            second_target: poke_core::geometry::Point8 { x: 9, y: 9 },
            first_opened: true,
            second_opened: false,
        });
        assert_eq!(overworld_turn(&solved, &[]), turn,
                   "the turn changed with the puzzle's hidden state");

        // Silent everywhere else: there are bins in exactly one gym.
        let elsewhere = state_from(include_bytes!("../pokemon/data/split-cerulean.bin"));
        assert!(!overworld_turn(&elsewhere, &[]).contains("resets both locks"));
    }

    #[test]
    fn a_badge_says_the_walkthrough_chapter_has_moved_on_under_the_model() {
        use crate::llm::guide::{status, GuideStatus};
        use crate::pokemon::badge::Badge;

        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/at-vermilion.bin")).expect("the fixture loads");
        let state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("the fixture has a readable state");

        let overworld = |guide| situation(
            DecisionKind::Overworld, &state, &ApiSnapshot::default(), &[], &[],
            TurnContext::Overworld { script: ScriptState::Unedited, standing: &Default::default(), guide }, &[],
        );

        // Read before the first badge: a chapter the model has already finished.
        let stale = status(state.badges, Some(0));
        assert!(matches!(stale, GuideStatus::Stale { .. }), "the fixture is past chapter 0: {stale:?}");
        let rendered = overworld(stale);
        assert!(rendered.contains("won a badge since you last read"), "{rendered}");
        assert!(rendered.contains("read_guide"), "and the tool that fixes it: {rendered}");
        // It names the chapter's subject rather than saying "a different one".
        assert!(
            rendered.contains(&crate::llm::guide::chapter_goal(crate::llm::guide::chapter_index(state.badges))),
            "{rendered}",
        );

        // Silent while current, or it is a line on every turn that a model learns to skip.
        for quiet in [status(state.badges, Some(crate::llm::guide::chapter_index(state.badges))), status(state.badges, None)] {
            let other = overworld(quiet);
            assert!(!other.contains("walkthrough"), "{quiet:?}: {other}");
        }

        // The nudge is an overworld thing.
        let elsewhere = situation(
            DecisionKind::Battle, &state, &ApiSnapshot::default(), &[], &[],
            TurnContext::Battle { script: ScriptState::Armed }, &[],
        );
        assert!(!elsewhere.contains("walkthrough"), "{elsewhere}");

        // The Elite Four is chapter 8 and has no badge to name.
        assert_eq!(crate::llm::guide::chapter_goal(8), "the Elite Four");
        assert_eq!(crate::llm::guide::chapter_goal(1), format!("the {}", Badge::CascadeBadge));
    }

    /// What the map will not let you do is said once, in the turn.
    #[test]
    fn an_obstacle_the_party_cannot_pass_is_named_rather_than_silently_dropped() {
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
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

        // Water is not one of these and must not come back.
        assert!(!state.can_use_surf, "the fixture reaches Vermilion before Surf");
        assert!(state.map.meta_tiles.iter().any(|tile| matches!(
                    tile, crate::pokemon::tile::MetaTile::Water
                        | crate::pokemon::tile::MetaTile::ConnectionWater(_))),
                "Vermilion is on the coast, so a water line would have fired here");
        assert!(!blocked.contains("Blocked here: Water"), "water is scenery, not an errand: {blocked}");

        // And only on the turn that has an action menu.
        for elsewhere in [DecisionKind::Battle, DecisionKind::Nickname, DecisionKind::Stuck] {
            let other = rendered(elsewhere, &state);
            assert!(!other.contains("Blocked here"), "{elsewhere:?} has no action menu: {other}");
        }

        // Which half is missing is the point: getting it wrong sends the model after the other.
        assert!(blocked.contains("not the HM"), "the badge is held; the HM is the errand: {blocked}");

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

        // …and with a Pokémon that can take it, the line becomes the tool call to make.
        let mut ready = hopeless.clone();
        ready.pokemon.get_mut(0).expect("one member").species = crate::pokemon::species::PokemonSpecies::Venusaur;
        let teachable = rendered(DecisionKind::Overworld, &ready);
        assert!(teachable.contains("`use_field_move` with `teach`"), "{teachable}");

        // And it stops once it stops being true.
        state.can_use_cut = true;
        let cleared = rendered(DecisionKind::Overworld, &state);
        assert!(!cleared.contains("Blocked here: Cuttable trees"), "{cleared}");
        assert!(cleared.contains("Cuttable trees on this map:"), "{cleared}");
        assert!(cleared.contains("walk over, use Cut, tree gone"),
                "the row is the whole action now, and the line has to say so: {cleared}");
        let tree = state.map.meta_tiles.iter().position(|tile| *tile == crate::pokemon::tile::MetaTile::CutTree)
            .expect("Vermilion has trees");
        assert!(cleared.contains(&format!("({}, {})", tree % state.map.width, tree / state.map.width)),
                "and where they are: {cleared}");
    }

    /// A Strength puzzle is named every turn it is in the room, whatever can be done about it.
    #[test]
    fn a_strength_puzzle_names_its_boulders_whether_or_not_they_can_be_pushed() {
        use crate::pokemon::badge::Badge;
        use crate::pokemon::move_name::PokemonMoveName;
        let mut state = state_from(include_bytes!("../pokemon/data/vr1f-strength.bin"));
        let rendered = |state: &GameState| situation(
            DecisionKind::Overworld, state, &ApiSnapshot::default(), &[], &[], TurnContext::None, &[],
        );

        assert!(state.map.can_strength, "the deployed party can use Strength");
        let armed = rendered(&state);
        // Reading order, which is the order `read_map`'s picture is scanned in.
        assert!(armed.contains("Boulders on this map: (14, 2), (2, 10), (5, 15)."), "{armed}");
        assert!(armed.contains("Boulder switches"), "{armed}");
        assert!(armed.contains("(17, 13)"), "the switch VictoryRoad1F's puzzle is about: {armed}");
        assert!(armed.contains("is a row in the menu below, one row per target"), "{armed}");
        // A row is a goal, so the line promises a target rather than a shove.
        assert!(armed.contains("the row names the boulder it will use"), "{armed}");
        // The half that keeps a wedged floor from reading as a broken game.
        assert!(armed.contains("back where it started"), "{armed}");

        // Neither half of Strength: the boulders are still named, and so is the reason.
        let mut helpless = state.clone();
        helpless.badges.remove(Badge::RainbowBadge);
        for index in 0..helpless.pokemon.len() {
            let mon = helpless.pokemon.get_mut(index).expect("in range");
            for slot in mon.moves.iter_mut() {
                if slot.is_some_and(|m| m.name == PokemonMoveName::Strength) { *slot = None; }
            }
        }
        let none = rendered(&helpless);
        assert!(none.contains("Boulders on this map:"), "{none}");
        assert!(none.contains("Nothing in your party knows Strength"), "{none}");
        assert!(none.contains("RainbowBadge"), "{none}");
        assert!(none.contains("no boulder actions in the menu below"), "{none}");

        // The badge alone, which is the case the whole run is usually in: HM04 is the errand.
        let mut unbadged = state.clone();
        unbadged.badges.remove(Badge::RainbowBadge);
        assert!(rendered(&unbadged).contains("before the game will let it be used outside battle"),
                "{}", rendered(&unbadged));

        // An unsolvable floor says so, and says the one thing that undoes it.
        assert!(!armed.contains("cannot be solved"), "{armed}");
        let untouched = state_from(include_bytes!("../pokemon/data/vr1f-stuck-push.bin"));
        assert!(!rendered(&untouched).contains("cannot be solved"), "{}", rendered(&untouched));

        // A floor with no legal push says that, rather than promising rows that are not there.
        let mut none_left = state.clone();
        none_left.map.sprites.retain(|sprite| !sprite.name.starts_with("Boulder")
            || sprite.position == poke_core::geometry::Point8 { x: 14, y: 2 });
        assert!(none_left.map.boulder_pushes().is_empty(), "(14, 2) is walled in on its own square");
        // Chosen on goal rows, not `boulder_pushes()`: the two part company on a floor with legal
        // shoves and no reachable target.
        assert!(!none_left.map.actions().iter().any(|action| matches!(action.tile,
            crate::pokemon::tile::MetaTile::BoulderGoal { .. })), "and so has no goal row");
        let quiet = rendered(&none_left);
        assert!(quiet.contains("There are no boulder rows in the menu below"), "{quiet}");
        assert!(!quiet.contains("cannot be solved"), "no verdict on the floor: {quiet}");

        // Silent where there are no boulders, or it is a line on every turn of the game.
        state.map.sprites.retain(|sprite| !sprite.name.starts_with("Boulder"));
        assert!(!rendered(&state).contains("Boulders on this map"), "{}", rendered(&state));
    }

    #[test]
    fn the_system_prompt_says_the_things_the_deployed_runs_needed_it_to_say() {
        for phrase in [
            "The game is not broken",
            // A locked gym door is the game's rule, not a malfunction to report.
            "Being stopped is not a malfunction",
            "Doing the same thing again is not a plan",
            "not the one you remember",
            "Read what people say to you",
            "put it on the plan straight away",
            // `then` and `resume_after_battle` save only when used, so the prompt points at them.
            "One decision can be several actions",
        ] {
            assert!(SYSTEM_PROMPT.contains(phrase), "the system prompt no longer says {phrase:?}");
        }

        // A restart keeps the conversation, but a compaction empties it and the plan survives.
        assert!(
            SYSTEM_PROMPT.contains("is what survives that. It is the only thing that does"),
            "the plan bullet no longer says what the plan is for",
        );
        assert!(
            !SYSTEM_PROMPT.contains("restart of the program"),
            "the conversation survives a restart now, so the prompt must not claim only the plan does",
        );
    }

    /// What a run that is following every rule above still leaves out.
    #[test]
    fn the_system_prompt_says_how_to_play_the_game_well() {
        for phrase in [
            // Ranked on `wPlayTime`: finishing is the goal, not wandering.
            "You are being timed",
            "Keep the party healthy",
            // Both halves: the penalty, and which Centre you wake up in.
            "faints you black out",
            "accepted a heal",
            "Keep stocked up",
            "Catch Pokémon",
            "Look round a town before you leave it",
            // A cartridge fact, not advice.
            "Experience is only paid out for a knockout",
            // The walkthrough is only worth carrying if the prompt says when to reach for it.
            "There is a walkthrough for this game",
            // Only prose can argue that a battle turn is worth avoiding.
            "Write your battles down",
            "battle.ask()",
            "Everything below is instruction rather than background",
        ] {
            assert!(SYSTEM_PROMPT.contains(phrase), "the system prompt no longer says {phrase:?}");
        }
    }

    /// A party line that names only the nickname stops naming the Pokémon at all.
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
        // A single type is stored in both slots, so an undeduplicated line reads `Normal/Normal`.
        assert!(!party.contains("Normal/Normal") && !party.contains("Water/Water"),
                "the duplicate type slot is folded: {party}");
    }

    /// The one turn whose entire question is "what am I short of" kept the answer behind a read.
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

    /// A scrolling conversation emits the same line repeatedly.
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

        // The cap keeps the most recent, which explain now.
        let many: Vec<String> = (0..40)
            .map(|i| describe_event(&AgentEvent::TextBox { message: format!("line {i}") }))
            .collect();
        let lines = summarise_events(&many);
        assert_eq!(lines.len(), MAX_EVENTS);
        assert_eq!(lines.last().unwrap(), "Text: line 39");
    }

    /// An abort reason stops the model re-picking a route that cannot be walked.
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

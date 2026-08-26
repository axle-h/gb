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

use crate::llm::tools::{DecisionKind, MenuItem, terminal_names};
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

How the interface works:

- The agent handles all button pressing, pathfinding and menu navigation. You choose *what* to do; \
  it works out *how*. Walking to a tile, talking to a person, taking a warp and picking a battle \
  move are all one decision each.
- Every turn you are shown the current situation and a menu of the actions available right now. \
  Each has an opaque `id`. Copy an id exactly; it is not a position in the list, and the list can \
  reorder between turns.
- The game keeps running while you think. A menu action can therefore disappear before your answer \
  lands — you will be told when that happens, and shown the current menu, so simply pick again.
- Read tools (`read_map`, `read_party`, …) do not end the turn. Request every read you need in a \
  single message; they are all answered from one consistent snapshot of the game. Most turns should \
  need none of them: the situation you are shown already carries the party, the money, the badges, \
  what is on screen and the menu. Which reads exist depends on what is being decided, and the list \
  at the bottom of each turn is the one that applies.
- `screenshot` shows you the actual screen. It costs far more than a read does, so use it when you \
  want to see something the other tools do not describe, not as a matter of routine.
- Not everything is walking. `use_field_move` covers cutting a tree you are facing, using Strength \
  or Flash or Dig from the party menu, flying, teaching an HM, using an item on something, pushing a \
  boulder, and pressing A at a tile to find what is hidden there.
- **The agent can be wrong, and `report_issue` is how you say so.** If the action menu does not \
  describe what is in front of you, or an action keeps failing for a reason you cannot see, file \
  one: what you were trying to do, what you expected, what happened instead. An action the game \
  stopped with a message you were shown is **not** one of these — there the reason is the message, \
  and it is a thing to act on rather than to report. A developer reads these, and the screen and a \
  save state are filed with it. ⚠️ It does **not** end your turn and \
  nothing changes now — so having filed it, carry on and try a different way. Reporting a problem \
  and playing on are not alternatives. What can be wrong is the agent's *description* of the game — \
  a menu row, a route, a name. The game itself is not wrong; see below.
- **This conversation is not your memory.** When it fills up it is replaced by a summary, and \
  everything not in that summary is gone. Your plan — `todo_set` and `todo_complete`, shown to you \
  every turn under 'Your plan' — is what survives that, and a restart of the program as well. It is \
  the only thing that does, so put anything you will still need in an hour there, with the reason \
  attached: somewhere you could not get into, something a person asked you for, something that did \
  not work.

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
  an item turns out wrong or impossible, `todo_set` with its number rewrites it, or deletes it if \
  you send no text. Replace it with what you now know rather than completing it or leaving it to \
  mislead you later.
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
  battle — switch, or use an item.
- Money is finite early on. Potions and Poké Balls are worth buying; almost nothing else is.
- The action list is what the agent can currently *reach*. If somewhere you want to go is not in it, \
  the way there is blocked, or it is on another map you have to walk to first.
- Some things stay shut until you have a particular move and the badge that lets you use it outside \
  battle. While that is true they are not offered to you at all, and the turn says so plainly under \
  'Blocked here'. That is a different errand, not a thing to keep trying.
- There is a walkthrough for this game, and `read_guide` hands you the stretch of it you are in \
  now: the order to do things in, what is blocking the way and what the next Gym Leader has. It is \
  worth reading if you are unsure where to go next, or you notice you have been round the same few \
  maps. It is chosen from your badges alone, so it answers with the same text every time until you \
  win the next badge — read it once and put what you need on your plan rather than asking again.
";

/// The line that ends every turn request, and the reason the loop can rely on exactly one terminal
/// call per turn. Regenerated per kind so it names only the tools that turn actually has.
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
    ForgetMove { current: &'a [crate::pokemon::move_name::PokemonMove], new: crate::pokemon::move_name::PokemonMoveName },
    /// **W9 / §14** — the watchdog's turn: what the agent believes it is doing, and for how long.
    /// Carried rather than read from the state for the same reason the two above are — nothing but
    /// the agent knows it.
    Stuck { agent_state: &'a str, stuck_for: std::time::Duration },
}

pub fn situation(
    kind: DecisionKind,
    state: &GameState,
    snapshot: &ApiSnapshot,
    events: &[String],
    menu: &[MenuItem],
    context: TurnContext<'_>,
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
        TurnContext::ForgetMove { new, .. } => out.push_str(&format!(
            "A Pokémon is trying to learn **{new}** but already knows four moves. Pick one to \
             replace, or decline and keep all four.\n\n",
        )),
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
    // ⚠️ **What the menu no longer offers still has to be explainable.** A cuttable tree and a
    // stretch of water are the two things on a map that are impassable *for now* rather than for
    // ever, and both are withheld from the action menu until the party can actually deal with them
    // (`MetaTileMap::can_cut` / `can_surf`) — because an action whose only follow-up the game
    // refuses is a menu loop, not a choice. Withholding it silently would leave a model staring at a
    // map with no way out and no idea why, which is the other half of the same bug: it would go
    // looking for a route that does not exist, or conclude the agent is broken. So the obstacle is
    // named once, in a sentence, with what it takes to pass it.
    //
    // ⚠️ **Only on the turn that has an action menu.** It is a fact about that menu, and the other
    // five kinds do not have one — on a naming screen it is overworld trivia in the middle of a
    // question about a word.
    if kind == DecisionKind::Overworld {
        use crate::pokemon::tile::MetaTile;
        let obstacles: [(bool, fn(&MetaTile) -> bool, &str, &str); 2] = [
            (!state.can_use_cut, |tile| matches!(tile, MetaTile::CutTree), "Cuttable trees",
             "Cut, which is an HM to be found and taught, and needs the CascadeBadge"),
            // `ConnectionWater` too: the sea at a map edge is the same wall as the pond inside it.
            (!state.can_use_surf, |tile| matches!(tile, MetaTile::Water | MetaTile::ConnectionWater(_)),
             "Water", "Surf, which is an HM to be found and taught, and needs the SoulBadge"),
        ];
        for (blocked, is_obstacle, noun, what) in obstacles {
            if blocked && state.map.meta_tiles.iter().any(is_obstacle) {
                out.push_str(&format!(
                    "Blocked here: {noun} on this map cannot be passed yet — that needs {what}. \
                     Nothing in the menu below leads past that, and retrying will not change it.\n",
                ));
            }
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
            "{slot}. {} Lv{} — {}/{} HP{} — {}\n",
            mon.nickname.to_default_string(),
            mon.level,
            mon.current_hp,
            mon.stats.hp,
            ailment(mon.status),
            if moves.is_empty() { "no moves".to_string() } else { moves.join(", ") },
        ));
    }

    if let Some(battle) = state.battle.as_ref() {
        out.push_str("\n### Battle\n");
        let side = |who: &str, mon: &crate::pokemon::pokemon::PokemonSummary| {
            format!("{who}: {:?} Lv{} — {}/{} HP{}\n",
                    mon.species, mon.level, mon.current_hp, mon.stats.hp, ailment(mon.status))
        };
        out.push_str(&format!("{:?} battle\n", battle.battle_type));
        out.push_str(&side("Yours", &battle.player));
        out.push_str(&side("Enemy", &battle.enemy));
        if battle.enemy_trapping {
            // ⚠️ The menu still opens and every option still looks available, but any *move* chosen is
            // replaced with "cannot move" — a decider that does not know this loops until the wrap ends.
            out.push_str("⚠️ You are trapped (Wrap/Bind/Fire Spin): a move will not execute this \
                          turn, but items, switching and running still work.\n");
        }
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

            let context = match kind {
                DecisionKind::Nickname => TurnContext::Nickname(PokemonSpecies::Eevee),
                DecisionKind::ForgetMove => {
                    TurnContext::ForgetMove { current: &party_moves, new: PokemonMoveName::Surf }
                }
                DecisionKind::Stuck => TurnContext::Stuck {
                    agent_state: "text→ReadingTextBox",
                    stuck_for: Duration::from_secs(300),
                },
                _ => TurnContext::None,
            };
            let menu = match kind {
                DecisionKind::Overworld => tools::overworld_menu(&state, snapshot.arrival),
                DecisionKind::Battle => tools::battle_menu(&state),
                DecisionKind::MartPurchase => tools::mart_menu(&snapshot),
                DecisionKind::ForgetMove => tools::forget_menu(&party_moves),
                DecisionKind::Nickname | DecisionKind::Stuck => Vec::new(),
            };

            // The message list a first turn goes out with, in the order `worker::run_one` appends
            // them: the constant system message, the plan, then the situation.
            let messages = vec![
                system_message(),
                plan_message(&todo),
                Message::user(situation(kind, &state, &snapshot, &events, &menu, context)),
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

    /// **What the map will not let you do is said once, in the turn.**
    ///
    /// An action whose only follow-up the game refuses is withheld from the menu
    /// (`MetaTileMap::can_cut` / `can_surf`), which is what stops a cut with no Cut becoming sixty
    /// seconds of A-mashing in a party menu. But withholding it *silently* is the same bug facing
    /// the other way: the deployed run, having found no way north out of Route 2, went round the
    /// same four maps for forty turns and filed three issue reports saying the game was broken. So
    /// the obstacle is named, with what it takes to pass it, and only while it is actually blocking.
    #[test]
    fn an_obstacle_the_party_cannot_pass_is_named_rather_than_silently_dropped() {
        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/at-vermilion.bin")).expect("the fixture loads");
        let mut state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("the fixture has a readable state");
        assert!(!state.can_use_cut, "the fixture reaches Vermilion before the HM");

        let rendered = |kind, state: &GameState| situation(
            kind, state, &ApiSnapshot::default(), &[], &[], TurnContext::None,
        );
        let blocked = rendered(DecisionKind::Overworld, &state);
        assert!(blocked.contains("Blocked here: Cuttable trees"), "{blocked}");
        assert!(blocked.contains("CascadeBadge"), "it has to say what would clear them: {blocked}");

        // ⚠️ **And only on the turn that has an action menu.** It is a fact about that menu; on a
        // battle turn or a naming screen it is overworld trivia in the middle of another question.
        for elsewhere in [DecisionKind::Battle, DecisionKind::Nickname, DecisionKind::Stuck] {
            let other = rendered(elsewhere, &state);
            assert!(!other.contains("Blocked here"), "{elsewhere:?} has no action menu: {other}");
        }

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
        ] {
            assert!(SYSTEM_PROMPT.contains(phrase), "the system prompt no longer says {phrase:?}");
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

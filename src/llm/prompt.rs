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
- `press_buttons` presses the joypad yourself, and it is an **escape hatch rather than a shortcut**: \
  it pre-empts the agent, every use of it is recorded and read afterwards, and a turn that had an \
  action menu almost never needed it. Reach for it only where no action describes the game at all.
- **This conversation is not your memory.** When it fills up it is replaced by a summary, and \
  everything not in that summary is gone. Your plan — `todo_set` and `todo_complete`, shown to you \
  every turn under 'Your plan' — is what survives that, and a restart of the program as well. It is \
  the only thing that does, so put anything you will still need in an hour there, with the reason \
  attached: somewhere you could not get into, something a person asked you for, something that did \
  not work.
- **Always have a plan, and treat it as a draft.** Keep three to five open items going even when \
  you are unsure — a rough plan you revise beats an empty one. Nothing on it is a commitment: when \
  an item turns out wrong or impossible, `todo_set` with its number rewrites it, or deletes it if \
  you send no text. Replace it with what you now know rather than completing it or leaving it to \
  mislead you later.

Things worth knowing about this particular game:

- Talking to people is how almost everything progresses. If you are stuck, there is usually someone \
  you have not spoken to.
- Wild encounters happen in tall grass and in caves. A fainted lead Pokémon is not the end of a \
  battle — switch, or use an item.
- Money is finite early on. Potions and Poké Balls are worth buying; almost nothing else is.
- The action list is what the agent can currently *reach*. If somewhere you want to go is not in it, \
  the way there is blocked, or it is on another map you have to walk to first.
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
        DecisionKind::Nickname => "## Decision: name this Pokémon, or keep the default\n\n",
        DecisionKind::MartPurchase => "## Decision: what to buy here, if anything\n\n",
        DecisionKind::ForgetMove => "## Decision: which move to forget, if any\n\n",
        DecisionKind::Stuck => "## Decision: the game is stuck — get it moving\n\n",
    });

    match context {
        TurnContext::None => {}
        TurnContext::Nickname(species) => out.push_str(&format!(
            // No article: a species name is one, and "a Eevee" / "a Omanyte" is what picking one
            // blind gets you in a sentence printed on every naming screen of the run.
            "The naming screen is open for {species}. It has just been caught, hatched or given \
             to you.\n\n",
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
            DecisionKind::Nickname => "(there is no menu — call `set_nickname`, with or without a name.)\n",
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
        assert!(is_plan(&plan), "the worker finds the stale copy with this");
        assert!(!crate::llm::compaction::is_turn_start(&plan),
                "a cut between the plan and its turn would drop the one thing meant to survive");
        assert!(!is_plan(&crate::llm::protocol::Message::user("Location: PalletTown")),
                "an ordinary turn is not a plan");
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
        })];
        assert!(summarise_events(&events)[0].contains("no route"), "{:?}", summarise_events(&events));
    }
}

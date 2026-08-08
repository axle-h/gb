//! **W4 / §7.1, §7.5** — what the model is told.
//!
//! Two pieces, and the split matters:
//!
//! - [`SYSTEM_PROMPT`] is written once and **never compacted** (W6), so it is the copy of the turn
//!   contract that survives everything.
//! - [`situation`] is rebuilt for every turn, and is deliberately *rich*. §7.1's finding is that the
//!   larger win is not batching read tools efficiently but not needing them: a turn that opens with
//!   the location, the party, the on-screen text, what happened since the last decision and the menu
//!   itself should need **zero** read calls. Tools are then for what does not fit — the map grid, the
//!   world graph, the full bag.
//!
//! The contract is restated at the bottom of every turn (§7.5's second line of defence), which costs
//! two lines of tokens and makes the rule the most recent instruction in the context every time.

use crate::llm::tools::{DecisionKind, MenuItem, terminal_names};
use crate::pokemon::GameState;
use crate::pokemon::agent::AgentEvent;

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
  single message; they are all answered from one consistent snapshot of the game.

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
        "End this turn by calling exactly one of: {}.\n\
         Read tools ({}) do not end the turn — call as many as you need, in one message, then \
         finish with a terminal call.",
        terminal_names(kind).join(", "),
        crate::llm::tools::READ_TOOLS.iter().map(|t| t.name).collect::<Vec<_>>().join(", "),
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
}

impl ApiSnapshot {
    pub fn read(api: &crate::pokemon::PokemonApi<'_>) -> Self {
        Self {
            screen_text: crate::pokemon::observe::screen_text(api),
            playtime: crate::pokemon::observe::playtime(api),
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

pub fn situation(
    kind: DecisionKind,
    state: &GameState,
    snapshot: &ApiSnapshot,
    events: &[String],
    menu: &[MenuItem],
) -> String {
    let mut out = String::with_capacity(2048);

    out.push_str(match kind {
        DecisionKind::Overworld => "## Decision: what to do next in the overworld\n\n",
        DecisionKind::Battle => "## Decision: what to do this battle turn\n\n",
    });

    out.push_str(&format!(
        "Location: {} at ({}, {}), facing {:?}\n",
        state.map.map, state.map.player_position.x, state.map.player_position.y, state.map.player_direction,
    ));
    let badges: Vec<String> = state.badges.iter_names().map(|(name, _)| name.to_string()).collect();
    out.push_str(&format!(
        "Badges: {}\nMoney: ¥{}   Play time: {}\n",
        if badges.is_empty() { "none yet".to_string() } else { badges.join(", ") },
        state.money,
        snapshot.playtime,
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
            "{slot}. {} Lv{} — {}/{} HP, {} — {}\n",
            mon.nickname.to_default_string(),
            mon.level,
            mon.current_hp,
            mon.stats.hp,
            mon.status,
            if moves.is_empty() { "no moves".to_string() } else { moves.join(", ") },
        ));
    }

    if let Some(battle) = state.battle.as_ref() {
        out.push_str("\n### Battle\n");
        let side = |who: &str, mon: &crate::pokemon::pokemon::PokemonSummary| {
            format!("{who}: {:?} Lv{} — {}/{} HP, {}\n", mon.species, mon.level, mon.current_hp, mon.stats.hp, mon.status)
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
    });
    if menu.is_empty() {
        out.push_str(
            "(nothing — the agent can reach no action from here. `wait` and look again; if it \
             stays empty you are boxed in and the run needs a person.)\n",
        );
    }
    for item in menu {
        out.push_str(&format!("- `{}` — {}\n", item.id, item.description));
    }

    out.push_str(&format!("\n{}\n", contract(kind)));
    out
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

/// What the worker sends when a tool batch was answered but the turn has already run out of steps.
pub const OUT_OF_STEPS: &str =
    "You have used every read this turn. Call a terminal tool now to end the turn.";

/// §7.5's fallback, quoted verbatim at a model that produced no tool call at all.
pub fn nudge(kind: DecisionKind) -> String {
    format!(
        "That reply contained no tool call, so nothing happened in the game. {}",
        contract(kind),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason};
    use crate::pokemon::tile::MetaTile;

    /// §7.5's third and fourth lines of defence are the same sentence in two places. If the tool
    /// catalogue grows a terminal tool and the contract does not mention it, the model is being told
    /// something false at the end of every single turn.
    #[test]
    fn the_contract_names_every_tool_the_turn_is_actually_sent() {
        for kind in [DecisionKind::Overworld, DecisionKind::Battle] {
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
        assert!(summarise_events(&events)[0].contains("NoRoute"), "{:?}", summarise_events(&events));
    }
}

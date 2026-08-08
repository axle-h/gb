//! **W4 / §7.4–7.5** — the tool surface: what the model is offered, how a call is answered, and how
//! an answer becomes a decision.
//!
//! Two kinds of tool, and the difference is the whole turn contract:
//!
//! - **Read tools** answer a question and the turn continues. They are serviced at the policy poll
//!   from the [`observe`](crate::pokemon::observe) facade, against **one** `GameState`, so several
//!   reads in one assistant message cannot disagree with each other.
//! - **Terminal tools** end the turn. Exactly one per turn, and the array offered is **scoped to the
//!   decision kind being asked** — a battle turn is never sent `choose_action`, so the model cannot
//!   end a turn the wrong way because the wrong way is not there.
//!
//! ⚠️ **An id is never a list index.** `MetaTileMap::actions()` is `sort()`ed by `MetaTile` and can
//! reorder between the tick that rendered the menu and the tick the answer lands on;
//! `ConsolePolicy` learned this the hard way and matches by tile. The ids here are stable composite
//! keys — `"{map}:{x},{y}:{tile}"` for the overworld, `"fight:{move}"` / `"item:{item}"` /
//! `"switch:{slot}"` for a battle — and they are re-resolved against a **freshly recomputed** action
//! list at the moment the decision is applied. An id that no longer matches is a tool error fed back
//! to the model, never a panic and never a silent no-op.
//!
//! W5 adds `screenshot`, `press_buttons`, `use_field_move`, `set_nickname`, `buy_item` and
//! `forget_move`; the shape they slot into is [`READ_TOOLS`] and [`Terminal`].

use serde_json::{Value, json};

use crate::llm::protocol::{ToolCall, ToolSpec};
use crate::pokemon::GameState;
use crate::pokemon::PokemonApi;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::observe;
use crate::pokemon::policy::battle_options;
use crate::pokemon::world_graph::WorldGraph;

/// Which question the agent is asking. A turn is keyed by this, and a poll for a different kind
/// cancels the turn in flight (§7.2).
///
/// ⚠️ **`pick_field_move` is not a kind and must never become one.** It is called on every idle
/// overworld tick immediately before `pick_overworld_action`; given its own kind the two would
/// cancel each other fifty times a second and no turn would ever finish. A field move is one
/// possible *outcome* of an overworld turn, not a turn of its own.
///
/// W5 adds `Nickname`, `MartPurchase` and `ForgetMove`. Until then `LlmPolicy` inherits the
/// `Policy` trait's defaults for those three — decline the nickname, buy nothing, forget nothing —
/// which are safe answers rather than stalls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecisionKind {
    Overworld,
    Battle,
}

impl DecisionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overworld => "overworld",
            Self::Battle => "battle",
        }
    }
}

/// A terminal tool call, parsed. Resolving it against the live game is [`resolve_overworld`] /
/// [`resolve_battle`] — done at the poll, not here, because the world may have moved since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminal {
    ChooseAction { id: String },
    ChooseBattleAction { id: String },
    /// Do nothing for this many agent ticks (20 ms of emulated time each). The honest answer when
    /// the game is mid-animation, and the forced answer when a model will not call anything else.
    Wait { ticks: u16 },
}

/// A cap, because `wait { ticks: 100000 }` is a model stalling its own run and there is no legitimate
/// reason to sit out more than a few seconds of game time in one decision.
pub const MAX_WAIT_TICKS: u16 = 150;

/// What one call in an assistant message turned out to be.
pub enum CallKind {
    /// A read tool. Answer it and keep going.
    Read,
    /// The turn is over.
    Terminal(Terminal),
    /// Nothing this turn can use — an unknown name, a terminal tool belonging to the other decision
    /// kind, or arguments that would not parse. The string is the message the model is shown, and it
    /// is shown *as a tool result* so the turn can recover rather than being thrown away.
    Rejected(String),
}

// ── The catalogue ────────────────────────────────────────────────────────────────────────────────

pub struct ReadTool {
    pub name: &'static str,
    pub description: &'static str,
}

/// Non-terminal, callable any number of times within a turn, and available under every decision
/// kind. Most turns should need none of them — the turn request already carries the situation
/// (§7.1) — so these are for what does not fit or is rarely wanted.
pub const READ_TOOLS: &[ReadTool] = &[
    ReadTool {
        name: "read_map",
        description: "The current map as an ASCII grid with a legend, plus every visible sprite, \
                      every warp and its destination, the adjacent maps, and the full list of \
                      actions reachable from where the player is standing.",
    },
    ReadTool {
        name: "read_party",
        description: "Every party member: species, nickname, level, HP, status, types, stats and \
                      all four moves with their remaining PP.",
    },
    ReadTool {
        name: "read_bag",
        description: "Every item in the bag with its quantity and shop price, plus money and how \
                      many of the bag's 20 slots are used.",
    },
    ReadTool {
        name: "read_trainer",
        description: "Name, rival's name, badges earned, money, Pokédex owned/seen and play time.",
    },
    ReadTool {
        name: "read_screen_text",
        description: "The text currently on screen, decoded from video memory. Returns null in the \
                      overworld — no dialogue font is loaded there, so there is genuinely nothing \
                      to read, and that is not an error.",
    },
    ReadTool {
        name: "read_battle",
        description: "The live battle: both sides' species, level, HP, status and moves, the enemy's \
                      catch rate, and every legal battle action. Returns null outside a battle.",
    },
    ReadTool {
        name: "read_world_graph",
        description: "Every map the player has physically stood on and how they connect. An absent \
                      map means 'not visited yet', never 'does not exist'. Use it to plan a route \
                      back to somewhere you have already been.",
    },
];

fn is_read_tool(name: &str) -> bool {
    READ_TOOLS.iter().any(|tool| tool.name == name)
}

/// The `tools` array for one decision kind — §7.5's first line of defence.
pub fn for_kind(kind: DecisionKind) -> Vec<ToolSpec> {
    let mut tools: Vec<ToolSpec> = READ_TOOLS
        .iter()
        .map(|tool| ToolSpec::new(tool.name, tool.description, no_arguments()))
        .collect();

    match kind {
        DecisionKind::Overworld => tools.push(ToolSpec::new(
            "choose_action",
            "ENDS THE TURN. Walk to and take one of the actions listed in the turn's action menu. \
             `id` is the id from that menu, copied exactly — never a position in the list.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "An id from the action menu." } },
                "required": ["id"],
                "additionalProperties": false,
            }),
        )),
        DecisionKind::Battle => tools.push(ToolSpec::new(
            "choose_battle_action",
            "ENDS THE TURN. Take one of the actions listed in the turn's battle menu. `id` is the \
             id from that menu, copied exactly.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "An id from the battle menu." } },
                "required": ["id"],
                "additionalProperties": false,
            }),
        )),
    }

    tools.push(ToolSpec::new(
        "wait",
        format!(
            "ENDS THE TURN. Do nothing for `ticks` agent ticks (20 ms of game time each, so 50 is \
             one second) and then decide again. Use it when the game is mid-animation or mid-text \
             and the right move is to let it finish. Maximum {MAX_WAIT_TICKS}."
        ),
        json!({
            "type": "object",
            "properties": {
                "ticks": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_WAIT_TICKS,
                    "description": "How many 20 ms ticks to wait.",
                }
            },
            "required": ["ticks"],
            "additionalProperties": false,
        }),
    ));
    tools
}

/// A zero-parameter tool still needs a schema, and an empty object is what every endpoint accepts.
fn no_arguments() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

/// The terminal tool names a turn of this kind may end with, for the contract restated in the prompt.
pub fn terminal_names(kind: DecisionKind) -> &'static [&'static str] {
    match kind {
        DecisionKind::Overworld => &["choose_action", "wait"],
        DecisionKind::Battle => &["choose_battle_action", "wait"],
    }
}

// ── Classification ───────────────────────────────────────────────────────────────────────────────

/// Decide what a call is, without touching the game.
///
/// Everything recoverable becomes [`CallKind::Rejected`] carrying a sentence for the model, rather
/// than an error that ends the turn: a model that reaches for `choose_action` in a battle should be
/// told the battle menu is over there, not have its turn silently discarded.
pub fn classify(kind: DecisionKind, call: &ToolCall) -> CallKind {
    let name = call.function.name.as_str();
    if is_read_tool(name) {
        return CallKind::Read;
    }

    let arguments = match call.arguments() {
        Ok(arguments) => arguments,
        Err(failure) => {
            return CallKind::Rejected(format!(
                "{failure}. Send the arguments as a JSON object and try again."
            ));
        }
    };

    match name {
        "choose_action" if kind == DecisionKind::Overworld => match string_argument(&arguments, "id") {
            Ok(id) => CallKind::Terminal(Terminal::ChooseAction { id }),
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "choose_battle_action" if kind == DecisionKind::Battle => match string_argument(&arguments, "id") {
            Ok(id) => CallKind::Terminal(Terminal::ChooseBattleAction { id }),
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "wait" => match arguments.get("ticks").and_then(Value::as_u64) {
            Some(ticks) => CallKind::Terminal(Terminal::Wait {
                ticks: ticks.clamp(1, u64::from(MAX_WAIT_TICKS)) as u16,
            }),
            None => CallKind::Rejected("`wait` needs a whole number of `ticks`.".to_string()),
        },
        // A terminal tool from the other decision kind. It exists, so saying "unknown tool" would be
        // actively misleading; what the model needs is the name of the one that does apply.
        "choose_action" | "choose_battle_action" => CallKind::Rejected(format!(
            "`{name}` is not available in a {} turn. End this turn with one of: {}.",
            kind.label(),
            terminal_names(kind).join(", "),
        )),
        other => CallKind::Rejected(format!(
            "There is no tool called `{other}`. The read tools are {}; end the turn with one of: {}.",
            READ_TOOLS.iter().map(|t| t.name).collect::<Vec<_>>().join(", "),
            terminal_names(kind).join(", "),
        )),
    }
}

fn string_argument(arguments: &Value, key: &str) -> Result<String, String> {
    match arguments.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_string()),
        _ => Err(format!("`{key}` is required and must be a non-empty string.")),
    }
}

// ── Servicing a read ─────────────────────────────────────────────────────────────────────────────

/// Answer one read tool from the triple the policy holds at a poll.
///
/// ⚠️ Every call in a batch is answered from the **same** `state`, which is what makes `read_party`
/// and `read_map` in one assistant message guaranteed to agree.
pub fn service_read(
    call: &ToolCall,
    state: &GameState,
    api: &PokemonApi<'_>,
    graph: &WorldGraph,
) -> String {
    let value = match call.function.name.as_str() {
        "read_map" => serde_json::to_value(observe::map_view(state)),
        "read_party" => serde_json::to_value(observe::party(state)),
        "read_bag" => serde_json::to_value(observe::bag(state, api)),
        "read_trainer" => serde_json::to_value(observe::trainer(state, api)),
        // `null` is the answer in the overworld, and the tool's description says so — a model told
        // "error" would try again, a model told `null` moves on.
        "read_screen_text" => serde_json::to_value(json!({ "text": observe::screen_text(api) })),
        "read_battle" => serde_json::to_value(observe::battle(state)),
        "read_world_graph" => serde_json::to_value(observe::world_graph(graph)),
        other => Ok(json!({ "error": format!("`{other}` is not a read tool") })),
    };
    match value.and_then(|value| serde_json::to_string(&value)) {
        Ok(json) => json,
        // Serialising a view cannot fail in practice, but a tool result is a string and the
        // alternative to this line is an `unwrap` on the worker's critical path.
        Err(failure) => format!("{{\"error\": \"could not encode the result: {failure}\"}}"),
    }
}

// ── Menus and ids ────────────────────────────────────────────────────────────────────────────────

/// One row of the menu the turn request renders, and the only place an id is minted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub id: String,
    pub description: String,
}

/// The id of one overworld action: stable across a re-sort, unique within a map, and readable
/// enough that a model quoting it back is obviously quoting the right thing.
pub fn overworld_id(state: &GameState, action: &OverworldAction) -> String {
    let destination = action.destination;
    format!("{}:{},{}:{}", state.map.map, destination.x, destination.y, action.tile)
}

/// Everything reachable from where the player is standing. Sorted, so two reads of an unchanged map
/// produce the same menu — `actions()` walks a `HashSet` and would otherwise reshuffle, which reads
/// to a model as the world having moved.
pub fn overworld_menu(state: &GameState) -> Vec<MenuItem> {
    let mut actions = state.map.actions();
    actions.sort_by_key(|action| (action.destination.y, action.destination.x, format!("{}", action.tile)));
    actions
        .iter()
        .map(|action| MenuItem {
            id: overworld_id(state, action),
            description: format!("{action} — {} steps", action.route.len()),
        })
        .collect()
}

/// Match an id against a **freshly recomputed** action list. `None` means the action is gone, which
/// is a thing the model is told rather than a thing that crashes.
pub fn resolve_overworld(state: &GameState, id: &str) -> Option<OverworldAction> {
    state.map.actions().into_iter().find(|action| overworld_id(state, action) == id)
}

/// The id of one battle action. Keyed on what the action *is* rather than where it sat: a bag slot
/// shifts the moment an item runs out, and a move's PP — which is in `BattleAction`'s `Display` —
/// changes the moment it is used.
pub fn battle_id(action: &BattleAction) -> String {
    match action {
        BattleAction::Fight { battle_move, .. } => format!("fight:{}", battle_move.name),
        BattleAction::UseItem { item, .. } => format!("item:{:?}", item.id),
        BattleAction::SwitchPokemon { slot, .. } => format!("switch:{slot}"),
        BattleAction::Run => "run".to_string(),
        BattleAction::SafariBall => "ball".to_string(),
        BattleAction::SafariBait => "bait".to_string(),
        BattleAction::SafariRock => "rock".to_string(),
    }
}

pub fn battle_menu(state: &GameState) -> Vec<MenuItem> {
    battle_options(state)
        .unwrap_or_default()
        .iter()
        .map(|action| MenuItem { id: battle_id(action), description: format!("{action}") })
        .collect()
}

pub fn resolve_battle(state: &GameState, id: &str) -> Option<BattleAction> {
    battle_options(state)?.into_iter().find(|action| battle_id(action) == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::protocol::FunctionCall;

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: "c".into(),
            kind: "function".into(),
            function: FunctionCall { name: name.into(), arguments: arguments.into() },
        }
    }

    fn names(kind: DecisionKind) -> Vec<&'static str> {
        for_kind(kind).into_iter().map(|tool| tool.function.name).collect()
    }

    /// §7.5's first line of defence: the model cannot end a turn the wrong way because the wrong way
    /// is not in the array it was sent.
    #[test]
    fn terminal_tools_are_scoped_per_kind() {
        let overworld = names(DecisionKind::Overworld);
        let battle = names(DecisionKind::Battle);

        assert!(overworld.contains(&"choose_action"));
        assert!(!overworld.contains(&"choose_battle_action"));
        assert!(battle.contains(&"choose_battle_action"));
        assert!(!battle.contains(&"choose_action"));

        for kind in [DecisionKind::Overworld, DecisionKind::Battle] {
            let offered = names(kind);
            assert!(offered.contains(&"wait"), "{kind:?} must always be able to wait");
            for read in READ_TOOLS {
                assert!(offered.contains(&read.name), "{kind:?} is missing {}", read.name);
            }
            // The contract restated in the prompt has to match the array actually sent, or the two
            // drift and the model is told about a tool it does not have.
            for terminal in terminal_names(kind) {
                assert!(offered.contains(terminal), "{kind:?} promises {terminal} but does not offer it");
            }
            assert_eq!(offered.len(), READ_TOOLS.len() + terminal_names(kind).len());
        }
    }

    /// Every schema must be a JSON Schema object with the properties it claims — a malformed one is
    /// a 400 from the endpoint on the very first turn of a run.
    #[test]
    fn every_schema_is_a_well_formed_object() {
        for kind in [DecisionKind::Overworld, DecisionKind::Battle] {
            for tool in for_kind(kind) {
                let schema = &tool.function.parameters;
                assert_eq!(schema["type"], "object", "{}", tool.function.name);
                assert!(schema.get("properties").is_some(), "{}", tool.function.name);
                assert!(!tool.function.description.is_empty(), "{}", tool.function.name);
                for required in schema.get("required").and_then(Value::as_array).unwrap_or(&vec![]) {
                    let key = required.as_str().expect("required names are strings");
                    assert!(schema["properties"].get(key).is_some(),
                            "{} requires `{key}` but does not describe it", tool.function.name);
                }
            }
        }
    }

    /// A terminal call from the other kind is answerable, not fatal — and the answer names the tool
    /// that would have worked.
    #[test]
    fn a_terminal_tool_from_the_wrong_kind_is_rejected_with_the_right_one() {
        let CallKind::Rejected(complaint) =
            classify(DecisionKind::Battle, &call("choose_action", r#"{"id":"x"}"#))
        else {
            panic!("choose_action must not end a battle turn");
        };
        assert!(complaint.contains("choose_battle_action"), "{complaint}");

        let CallKind::Rejected(complaint) = classify(DecisionKind::Overworld, &call("teleport", "{}")) else {
            panic!("an invented tool is rejected");
        };
        assert!(complaint.contains("teleport") && complaint.contains("read_map"), "{complaint}");
    }

    #[test]
    fn arguments_are_parsed_or_complained_about() {
        assert!(matches!(
            classify(DecisionKind::Overworld, &call("choose_action", r#"{"id":"PalletTown:5,6:Warp"}"#)),
            CallKind::Terminal(Terminal::ChooseAction { ref id }) if id == "PalletTown:5,6:Warp",
        ));
        assert!(matches!(
            classify(DecisionKind::Battle, &call("wait", r#"{"ticks":25}"#)),
            CallKind::Terminal(Terminal::Wait { ticks: 25 }),
        ));
        // A model asking to sit out ten minutes of game time is stalling its own run.
        assert!(matches!(
            classify(DecisionKind::Battle, &call("wait", r#"{"ticks":99999}"#)),
            CallKind::Terminal(Terminal::Wait { ticks: MAX_WAIT_TICKS }),
        ));
        assert!(matches!(classify(DecisionKind::Overworld, &call("wait", "{}")), CallKind::Rejected(_)));
        assert!(matches!(
            classify(DecisionKind::Overworld, &call("choose_action", r#"{"id":""}"#)),
            CallKind::Rejected(_),
        ));
        assert!(matches!(
            classify(DecisionKind::Overworld, &call("choose_action", "not json at all")),
            CallKind::Rejected(_),
        ));
        // A zero-parameter read tool is routinely called with empty arguments rather than `{}`.
        assert!(matches!(classify(DecisionKind::Overworld, &call("read_map", "")), CallKind::Read));
    }

    /// A battle id must survive the thing that changes most often about a battle action: its PP,
    /// which `BattleAction`'s own `Display` includes.
    #[test]
    fn a_battle_id_ignores_the_volatile_parts() {
        use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
        let with_pp = |pp| BattleAction::Fight {
            slot: 0,
            battle_move: PokemonMove { name: PokemonMoveName::Tackle, pp },
        };
        assert_eq!(battle_id(&with_pp(35)), "fight:Tackle");
        assert_eq!(battle_id(&with_pp(1)), battle_id(&with_pp(35)));
        assert_ne!(format!("{}", with_pp(1)), format!("{}", with_pp(35)),
                   "…which the Display, and therefore a Display-keyed id, would not have");
        assert_eq!(battle_id(&BattleAction::Run), "run");
    }
}

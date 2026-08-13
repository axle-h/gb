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
//! **W5** added the rest of it: `screenshot`, `press_buttons`, `use_field_move`, `set_nickname`,
//! `buy_item` and `forget_move`, and the three decision kinds the last three answer.

use serde_json::{Value, json};

use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::llm::prompt::ApiSnapshot;
use crate::llm::todo::{MAX_TEXT as MAX_TODO_TEXT, TodoCall};
use crate::llm::protocol::{ToolCall, ToolSpec};
use crate::llm::worker::ToolAnswer;
use crate::pokemon::GameState;
use crate::pokemon::PokemonApi;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::agent::MANUAL_INPUT_CAPACITY;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::observe;
use crate::pokemon::policy::{FieldMove, battle_options, field_move_index};
use crate::pokemon::world_graph::WorldGraph;

/// Which question the agent is asking. A turn is keyed by this, and a poll for a different kind
/// cancels the turn in flight (§7.2).
///
/// These are exactly the agent's five policy poll sites, and there is no sixth: `Policy` has
/// `pick_overworld_action`, `pick_battle_action`, `pick_nickname`, `pick_mart_purchase` and
/// `pick_move_to_forget`, and each of them is one question with one answer.
///
/// ⚠️ **`pick_field_move` is not a kind and must never become one.** It is called on every idle
/// overworld tick immediately before `pick_overworld_action`; given its own kind the two would
/// cancel each other fifty times a second and no turn would ever finish. A field move is one
/// possible *outcome* of an overworld turn ([`Terminal::UseFieldMove`]), not a turn of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecisionKind {
    Overworld,
    Battle,
    /// The naming screen is open for a Pokémon just caught, hatched or given.
    Nickname,
    /// A mart's Buy/Sell/Quit menu just opened.
    MartPurchase,
    /// "Which move should be forgotten?" — a level-up learn, or teaching an HM to a mon that already
    /// knows four moves.
    ///
    /// ⚠️ **This legitimately pre-empts `Battle`**: the prompt fires mid-fight through the agent's
    /// global handler, and the prompt is the live question. Cancelling the battle turn to answer it
    /// is correct, and a fresh battle turn starts afterwards.
    ForgetMove,
    /// **W9 / §14** — the sixth kind, and the only one that is not a poll site: the agent has
    /// reached *no* decision point for `GB_STUCK_TIMEOUT_SECS` of emulated time and the watchdog is
    /// asking on its behalf.
    ///
    /// It is the exception that proves the rule above. The other five are questions the agent knows
    /// how to carry out an answer to; this one is "the agent is wedged", so the only terminal tools
    /// are `press_buttons` — which goes round the state machine entirely — and `wait`.
    Stuck,
}

impl DecisionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overworld => "overworld",
            Self::Battle => "battle",
            Self::Nickname => "nickname",
            Self::MartPurchase => "mart",
            Self::ForgetMove => "forget-move",
            Self::Stuck => "stuck",
        }
    }

    /// Whether the `GameState` cannot tell that this is the question being asked, so the only
    /// evidence is which poll site ran last — see `LlmPolicy::observed_kind`.
    ///
    /// True of the three transient menu prompts (a naming screen, a mart's Buy/Sell menu and the
    /// forget-move prompt all look like an ordinary overworld or battle state) and of **W9's
    /// `Stuck`**, which looks like whatever the agent was doing when it wedged. Getting this wrong
    /// is not a wasted round trip but an infinite loop: every read batch cancelled, every turn
    /// restarted.
    pub fn is_inferred_from_the_site(self) -> bool {
        matches!(self, Self::Nickname | Self::MartPurchase | Self::ForgetMove | Self::Stuck)
    }
}

/// A terminal tool call, parsed. Resolving it against the live game is [`resolve_overworld`],
/// [`resolve_battle`] and [`resolve_field_move`] — done at the poll, not here, because the world may
/// have moved since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminal {
    ChooseAction { id: String },
    ChooseBattleAction { id: String },
    /// Something the agent does *without* walking: cut a tree, teach an HM, push a boulder. Stashed
    /// by the policy and handed to the agent at the next `pick_field_move`, which is the tick after.
    UseFieldMove(FieldMoveRequest),
    /// The escape hatch (§17 risk 1): raw joypad presses, delivered ahead of the state machine.
    PressButtons { buttons: Vec<JoypadButton> },
    SetNickname { name: Option<String> },
    BuyItem { item: Option<BagItem> },
    ForgetMove { slot: Option<u8> },
    /// Do nothing for this many agent ticks (20 ms of emulated time each). The honest answer when
    /// the game is mid-animation, and the forced answer when a model will not call anything else.
    Wait { ticks: u16 },
}

/// A cap, because `wait { ticks: 100000 }` is a model stalling its own run and there is no legitimate
/// reason to sit out more than a few seconds of game time in one decision.
pub const MAX_WAIT_TICKS: u16 = 150;

/// The longest nickname the naming screen's buffer holds.
pub const MAX_NICKNAME: usize = 10;

/// What one call in an assistant message turned out to be.
pub enum CallKind {
    /// A read tool. Answer it at the policy poll and keep going.
    Read,
    /// `screenshot`. Answered by the **worker**, from the frame the host already published — see
    /// [`crate::llm::screenshot`]. It never reaches the emulator thread.
    Screenshot,
    /// **W6b** — a TODO operation. Answered by the worker too: none of it needs the emulator, so
    /// making it a batch for `service_tools` would cost a round trip for a file write.
    Todo(TodoCall),
    /// The turn is over.
    Terminal(Terminal),
    /// Nothing this turn can use — an unknown name, a terminal tool belonging to the other decision
    /// kind, or arguments that would not parse. The string is the message the model is shown, and it
    /// is shown *as a tool result* so the turn can recover rather than being thrown away.
    Rejected(String),
}

impl CallKind {
    /// The discriminant, for the page. Not `strum`'s derive: `Todo(TodoCall)` and
    /// `Terminal(Terminal)` would drag their payloads' names into a string the client matches on,
    /// and these four words are a wire contract with `api.ts`.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Read | Self::Screenshot => "read",
            Self::Todo(_) => "todo",
            Self::Terminal(_) => "terminal",
            Self::Rejected(_) => "rejected",
        }
    }
}

// ── Field moves ──────────────────────────────────────────────────────────────────────────────────

/// A `use_field_move` call, parsed but not yet resolved. [`resolve_field_move`] turns one of these
/// into a [`FieldMove`] against the live state, because two of them need the party to do it.
///
/// **This is a chosen subset of [`FieldMove`], not all of it.** The variants left out —
/// `Fish`, `UseItemPc`, `UsePcBox`, `SellToMart`, `RedeemPrize`, `UsePartyScript`, `UseElevator` —
/// are postgame mechanisms whose arguments are internal types (a `PcBoxOp`, a `Prize`, a
/// `PartyScript`) rather than anything a model could name from what it is shown, and none of them is
/// on the path to the Hall of Fame. Anything genuinely unreachable is what `press_buttons` is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldMoveRequest {
    /// Cut the tree the player is **currently facing** — so this is the second half of a pair: walk
    /// to the tree with `choose_action`, then cut it.
    Cut,
    /// A field move used from the party menu: Strength, Flash, Dig, Teleport, Softboiled. `slot` is
    /// optional — without it, the first party member that knows the move is used.
    ///
    /// Surf is deliberately not here: the agent mounts it by itself the moment a route steps onto
    /// water, so asking for it explicitly is at best redundant.
    PartyMove { name: PokemonMoveName, slot: Option<u8> },
    Fly { to: Map },
    /// Teach an HM or TM in the bag to a party member.
    Teach { item: ItemId, slot: u8 },
    /// Use an evolution stone from the bag on a party member.
    Evolve { stone: ItemId, slot: u8 },
    /// Face `target` and use a bag item on it — the Poké Flute on a sleeping Snorlax, the Card Key
    /// on a door.
    UseItem { item: ItemId, target: Point8 },
    /// Throw an item away to free one of the bag's 20 slots.
    TossItem { item: ItemId },
    /// Shove a boulder one tile. Strength must already be armed (use it from the party menu first).
    PushBoulder { boulder: Point8, direction: JoypadButton },
    /// Rearrange the party so `slot` leads. Instant — the agent writes it straight to RAM.
    ReorderParty { slot: u8 },
    /// Face a tile and press A. Every "hidden object" in the game is this: hidden items, the
    /// Vermilion Gym bins, the Pokémon Mansion statue switches, the Rocket Hideout poster.
    Interact { target: Point8, facing: Option<PlayerFacingDirection> },
}

/// Turn a request into the [`FieldMove`] the agent executes, or into the sentence the model is told
/// instead. Everything that can be checked from the state is checked here rather than left to fail
/// silently three seconds later inside a menu driver.
pub fn resolve_field_move(state: &GameState, request: &FieldMoveRequest) -> Result<FieldMove, String> {
    let party_slot = |slot: u8| -> Result<u8, String> {
        match (slot as usize) < state.pokemon.len() {
            true => Ok(slot),
            false => Err(format!(
                "There is no party member in slot {slot} — the party has {} (slots 0–{}).",
                state.pokemon.len(),
                state.pokemon.len().saturating_sub(1),
            )),
        }
    };
    let held = |item: ItemId| -> Result<ItemId, String> {
        match state.bag.iter().any(|entry| entry.id == item) {
            true => Ok(item),
            false => Err(format!("There is no {item} in the bag. `read_bag` lists what is there.")),
        }
    };

    Ok(match request {
        FieldMoveRequest::Cut => {
            // The driver cuts whatever is in front of the player, so a player facing anything else
            // walks into a menu it cannot use and comes back out having achieved nothing.
            match state.map.tile_in_front() {
                Some((_, crate::pokemon::tile::MetaTile::CutTree)) => FieldMove::CutTree,
                _ => {
                    return Err(
                        "Cut works on the tree the player is facing, and there is not one there. \
                         Use `choose_action` on a `:CutTree` entry in the action menu first — that \
                         walks up to a tree and faces it — then call `use_field_move` with `cut`."
                            .to_string(),
                    );
                }
            }
        }
        FieldMoveRequest::PartyMove { name, slot } => {
            let slot = match slot {
                Some(slot) => {
                    let slot = party_slot(*slot)?;
                    if !knows(state, slot, *name) {
                        return Err(format!("The Pokémon in slot {slot} does not know {name}."));
                    }
                    slot
                }
                None => match (0..state.pokemon.len() as u8).find(|&slot| knows(state, slot, *name)) {
                    Some(slot) => slot,
                    None => return Err(format!("No Pokémon in the party knows {name}.")),
                },
            };
            // ⚠️ The party menu lists a mon's field moves in **its own move-slot order**, so the
            // index depends on what else that mon knows. It is computed, never assumed.
            FieldMove::UseFieldMove { slot, move_index: field_move_index(state, slot, *name) }
        }
        FieldMoveRequest::Fly { to } => FieldMove::Fly { to: *to },
        FieldMoveRequest::Teach { item, slot } => {
            FieldMove::TeachMove { item: held(*item)?, target_slot: party_slot(*slot)? }
        }
        FieldMoveRequest::Evolve { stone, slot } => {
            let slot = party_slot(*slot)?;
            // Completion is "this slot's species changed", so the driver needs the species it
            // started from — which the model has no way to supply and no business supplying.
            let evolve_from = state
                .pokemon
                .get(slot as usize)
                .map(|mon| mon.species)
                .ok_or_else(|| format!("Slot {slot} is empty."))?;
            FieldMove::EvolveWithStone { stone: held(*stone)?, target_slot: slot, evolve_from }
        }
        FieldMoveRequest::UseItem { item, target } => {
            FieldMove::UseFieldItem { item: held(*item)?, target: *target }
        }
        FieldMoveRequest::TossItem { item } => FieldMove::TossItem { item: held(*item)? },
        FieldMoveRequest::PushBoulder { boulder, direction } => {
            FieldMove::PushBoulder { boulder: *boulder, dir: *direction }
        }
        FieldMoveRequest::ReorderParty { slot } => FieldMove::ReorderParty { slot: party_slot(*slot)? },
        FieldMoveRequest::Interact { target, facing } => {
            FieldMove::CheckTrashCan { target: *target, facing: *facing }
        }
    })
}

fn knows(state: &GameState, slot: u8, name: PokemonMoveName) -> bool {
    state
        .pokemon
        .get(slot as usize)
        .is_some_and(|mon| mon.moves.iter().flatten().any(|m| m.name == name))
}

/// The moves `use_field_move` accepts under [`FieldMoveRequest::PartyMove`], with what each one is
/// for. Also the tool description's own list, so the two cannot drift.
const PARTY_MOVES: &[(&str, PokemonMoveName, &str)] = &[
    ("strength", PokemonMoveName::Strength, "arm Strength so boulders can be pushed"),
    ("flash", PokemonMoveName::Flash, "light a dark map (Rock Tunnel)"),
    ("dig", PokemonMoveName::Dig, "warp straight out of a cave or dungeon"),
    ("teleport", PokemonMoveName::Teleport, "warp back to the last Pokémon Center"),
    ("softboiled", PokemonMoveName::Softboiled, "heal another party member from Chansey's HP"),
];

// ── The catalogue ────────────────────────────────────────────────────────────────────────────────

pub struct ReadTool {
    pub name: &'static str,
    pub description: &'static str,
    /// ⚠️ **Which turns this read is offered in, and it is not "all of them".** Every kind used to
    /// carry every read: a battle turn paid for `read_map`, a naming screen paid for the whole
    /// catalogue in order to answer with a word. Worse than the tokens is what an irrelevant tool
    /// invites — `read_battle` in the overworld can only ever answer `null`, and a model that calls
    /// it has spent a round trip finding that out.
    pub kinds: &'static [DecisionKind],
    /// `None` for the reads that take no arguments, which is all of them but [`READ_ROUTE`].
    pub parameters: Option<fn() -> Value>,
}

/// Non-terminal, callable any number of times within a turn. Most turns should need none of them —
/// the turn request already carries the situation (§7.1) — so these are for what does not fit or is
/// rarely wanted.
///
/// ⚠️ **Nothing here may duplicate the situation.** `read_screen_text` and `read_trainer` were both
/// deleted for it: the first answered from the very same `observe::screen_text` the turn already
/// renders under `### On screen`, and everything the second returned but the Pokédex counts was in
/// the turn's header. A read whose answer the model was already holding is a round trip bought for
/// nothing, and it teaches the model that reading is how a turn starts.
pub const READ_TOOLS: &[ReadTool] = &[
    ReadTool {
        name: "read_map",
        description: "A picture of the whole map, drawn from the game's own graphics: everyone \
                      where they stand and face, warps and map edges labelled with where they lead, \
                      unreachable ground dimmed, and a coordinate ruler along the top and left. It \
                      arrives as an image after the result, with everyone on the map and the warps \
                      as data. The actions you can take are in the turn's action menu, not here.",
        // Not in a battle: there is no map on screen and nothing on it can be acted on.
        kinds: &[DecisionKind::Overworld, DecisionKind::Stuck],
        parameters: None,
    },
    ReadTool {
        name: "read_party",
        description: "Every party member: species, nickname, level, HP, status, types, stats and \
                      all four moves with their remaining PP.",
        kinds: &[
            DecisionKind::Overworld,
            DecisionKind::Battle,
            DecisionKind::Nickname,
            DecisionKind::MartPurchase,
            DecisionKind::ForgetMove,
        ],
        parameters: None,
    },
    ReadTool {
        name: "read_bag",
        description: "Every item in the bag with its quantity and shop price, plus money and how \
                      many of the bag's 20 slots are used.",
        // The one read the situation genuinely cannot supply: the bag is nowhere in a turn request,
        // and `use_field_move` needs an item named exactly as the bag names it.
        kinds: &[DecisionKind::Overworld, DecisionKind::Battle, DecisionKind::MartPurchase],
        parameters: None,
    },
    ReadTool {
        name: "read_battle",
        description: "The live battle: both sides' species, level, HP, status and moves, the \
                      enemy's catch rate, and which of your moves Disable has locked out. The \
                      actions you can take are in the turn's own battle menu, not here.",
        // ⚠️ `ForgetMove` legitimately fires mid-fight, and which move to drop is a battle question.
        kinds: &[DecisionKind::Battle, DecisionKind::ForgetMove],
        parameters: None,
    },
    ReadTool {
        name: READ_ROUTE,
        description: "How to get somewhere you have already been. With `to`, the sequence of maps \
                      from here to that one; without it, every map you have set foot on. It knows \
                      only what has been walked, so a map missing from it means 'not visited yet', \
                      never 'does not exist'.",
        kinds: &[DecisionKind::Overworld],
        parameters: Some(read_route_arguments),
    },
    ReadTool {
        name: SCREENSHOT,
        description: "A picture of the Game Boy screen as it is right now. Everything the agent can \
                      read for you — the map, the party, the text on screen — is cheaper and more \
                      precise as one of the other reads; ask for this when you want to see \
                      something they do not model, such as an unfamiliar menu or an animation you \
                      are not sure has finished.",
        // Every kind: it is the only tool that can answer "what on earth is on screen", which is
        // exactly the question a nickname prompt, a mart menu or a wedged agent raises.
        kinds: &ALL_KINDS,
        parameters: None,
    },
];

/// Answered by the worker rather than at the policy poll, because PNG encoding does not belong on
/// the emulator thread. See [`CallKind::Screenshot`].
pub const SCREENSHOT: &str = "screenshot";

/// **The world graph, asked the question a model actually has.** It replaced `read_world_graph`,
/// which serialised every visited `(map, entry)` node with all of its edges — unbounded by
/// construction, and by the late game large enough to be a meaningful fraction of the window in a
/// single call. Nothing wanted the adjacency list; what a turn wants is "which way is Celadon", so
/// the routing runs here, where the graph already is, and what crosses into the context is the
/// answer.
pub const READ_ROUTE: &str = "read_route";

fn read_route_arguments() -> Value {
    json!({
        "type": "object",
        "properties": {
            "to": {
                "type": "string",
                "description": "A map to route to, e.g. `CeruleanCity`. Omit to list the maps you \
                                have visited.",
            }
        },
        "additionalProperties": false,
    })
}

/// Every [`DecisionKind`], for the reads that are offered in all of them — and, in the tests, so a
/// loop that meant "all of them" cannot quietly stop meaning it when a seventh is added.
pub const ALL_KINDS: [DecisionKind; 6] = [
    DecisionKind::Overworld,
    DecisionKind::Battle,
    DecisionKind::Nickname,
    DecisionKind::MartPurchase,
    DecisionKind::ForgetMove,
    DecisionKind::Stuck,
];

fn read_tool(name: &str) -> Option<&'static ReadTool> {
    READ_TOOLS.iter().find(|tool| tool.name == name)
}

fn reads_for(kind: DecisionKind) -> impl Iterator<Item = &'static ReadTool> {
    READ_TOOLS.iter().filter(move |tool| tool.kinds.contains(&kind))
}

// ── W6b: the plan (§10) ──────────────────────────────────────────────────────────────────────────

/// The two TODO tools, by name. Non-terminal like the reads, and named in the turn contract for the
/// same reason: a model that thinks `todo_add` ended its turn stops playing.
///
/// ⚠️ **There were four.** `memory_write` and `memory_read` sat beside these, doing the same job in
/// a different shape — see [`crate::llm::todo`]'s module docs for why one mechanism beat two.
pub const TODO_TOOL_NAMES: &[&str] = &["todo_add", "todo_complete"];

/// Their specs. A function rather than a const because a JSON Schema is not a `const` expression.
pub fn todo_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "todo_add",
            format!(
                "Add something to your plan. The list is shown to you every turn and it is the only \
                 thing you write that survives this conversation being summarised away, or the \
                 program restarting — so say the reason as well as the intent: `come back to Route \
                 12 with the Poké Flute, the Snorlax blocks the path south`. At most \
                 {MAX_TODO_TEXT} characters."
            ),
            json!({
                "type": "object",
                "properties": { "text": { "type": "string", "description": "What to do, and why." } },
                "required": ["text"],
                "additionalProperties": false,
            }),
        ),
        ToolSpec::new(
            "todo_complete",
            "Mark one item on your plan done, by the number shown beside it.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "integer", "minimum": 1, "description": "The item's number." } },
                "required": ["id"],
                "additionalProperties": false,
            }),
        ),
    ]
}

fn classify_todo(name: &str, arguments: &Value) -> Option<CallKind> {
    let call = match name {
        "todo_add" => match string_argument(arguments, "text") {
            Ok(text) => TodoCall::Add { text },
            Err(complaint) => return Some(CallKind::Rejected(complaint)),
        },
        "todo_complete" => match arguments.get("id").and_then(Value::as_u64) {
            Some(id) => TodoCall::Complete { id: id.min(u64::from(u32::MAX)) as u32 },
            None => return Some(CallKind::Rejected("`todo_complete` needs the item's `id`.".to_string())),
        },
        _ => return None,
    };
    Some(CallKind::Todo(call))
}

/// The `tools` array for one decision kind — §7.5's first line of defence.
pub fn for_kind(kind: DecisionKind) -> Vec<ToolSpec> {
    let mut tools: Vec<ToolSpec> = reads_for(kind)
        .map(|tool| {
            ToolSpec::new(tool.name, tool.description, tool.parameters.map_or_else(no_arguments, |f| f()))
        })
        .collect();
    tools.extend(todo_tools());

    match kind {
        DecisionKind::Overworld => {
            tools.push(ToolSpec::new(
                "choose_action",
                "ENDS THE TURN. Walk to and take one of the actions listed in the turn's action menu. \
                 `id` is the id from that menu, copied exactly — never a position in the list.",
                json!({
                    "type": "object",
                    "properties": { "id": { "type": "string", "description": "An id from the action menu." } },
                    "required": ["id"],
                    "additionalProperties": false,
                }),
            ));
            tools.push(use_field_move_spec());
            tools.push(press_buttons_spec());
        }
        DecisionKind::Battle => {
            tools.push(ToolSpec::new(
                "choose_battle_action",
                "ENDS THE TURN. Take one of the actions listed in the turn's battle menu. `id` is the \
                 id from that menu, copied exactly.",
                json!({
                    "type": "object",
                    "properties": { "id": { "type": "string", "description": "An id from the battle menu." } },
                    "required": ["id"],
                    "additionalProperties": false,
                }),
            ));
            tools.push(press_buttons_spec());
        }
        DecisionKind::Nickname => tools.push(ToolSpec::new(
            "set_nickname",
            format!(
                "ENDS THE TURN. Name the Pokémon on the naming screen. Omit `name` to keep the \
                 default, which is the species name in capitals — that is the ordinary answer. At \
                 most {MAX_NICKNAME} characters; anything longer is truncated."
            ),
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "maxLength": MAX_NICKNAME,
                        "description": "The nickname. Omit to keep the species name.",
                    }
                },
                "additionalProperties": false,
            }),
        )),
        DecisionKind::MartPurchase => tools.push(ToolSpec::new(
            "buy_item",
            "ENDS THE TURN. Buy one kind of item from the mart, then leave. `item` is a name from \
             the stock list, copied exactly. Omit `item` to walk away without buying anything. The \
             order is trimmed to what the money covers — Gen 1 sells you nothing at all if you \
             cannot afford the whole order — and you can talk to the clerk again to buy something \
             else.",
            json!({
                "type": "object",
                "properties": {
                    "item": { "type": "string", "description": "A name from the stock list." },
                    "quantity": { "type": "integer", "minimum": 1, "maximum": 99, "default": 1 },
                },
                "additionalProperties": false,
            }),
        )),
        // **W9.** `press_buttons` is pushed by the arms above for the two kinds that also have a
        // menu; here it is the only thing on offer besides `wait`.
        DecisionKind::Stuck => tools.push(press_buttons_spec()),
        DecisionKind::ForgetMove => tools.push(ToolSpec::new(
            "forget_move",
            "ENDS THE TURN. Answer the 'which move should be forgotten?' prompt. `slot` is the move \
             slot to replace, from the list in the turn. Omit `slot` to decline the new move and \
             keep all four.",
            json!({
                "type": "object",
                "properties": {
                    "slot": { "type": "integer", "minimum": 0, "maximum": 3, "description": "The move slot to forget." },
                },
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

    for tool in &mut tools {
        if terminal_names(kind).contains(&tool.function.name) {
            add_summary_argument(tool);
        }
    }
    tools
}

/// How long a turn summary may be. Long enough for the intent *and* the reason — "heading to
/// Viridian for Poké Balls, I have none and the grass north of here is where I can catch a second
/// mon" — and short enough that carrying one per turn for the length of a run is not what fills the
/// context window.
pub const MAX_SUMMARY: usize = 300;

/// Bolt a required `summary` onto a terminal tool's schema.
///
/// ⚠️ **This is the only thing the model says that survives its own turn.** A reasoning model's
/// thinking arrives on a channel of its own and is deliberately never sent back (it is billed as
/// completion tokens once, and a copy in the history pays for it again every turn afterwards), and
/// most models emit no `content` at all beside a tool call. So the assistant side of the history was
/// a column of bare JSON: what was done, never once why. A model reading that back has no record of
/// having *tried* anything, which is exactly the state in which it walks into the same building for
/// the fourth time.
///
/// It rides on the terminal call's own arguments rather than in a message of its own because that
/// is the one place a sentence can go that costs no extra round trip, cannot be separated from the
/// decision it explains, and lands in the history by itself — `Message::assistant` already carries
/// `tool_calls` verbatim, arguments included.
///
/// ⚠️ **Required in the schema, optional in the parser.** Saying it is required is what gets it
/// filled in; *enforcing* it would not, because a rejected call does not end the turn — it becomes
/// another tool result and spends another of `GB_MAX_TOOL_STEPS`, so a model that forgets it would
/// be pushed towards the forced `wait` rather than towards remembering. See `call_summary`.
fn add_summary_argument(tool: &mut ToolSpec) {
    let Some(properties) = tool.function.parameters.get_mut("properties").and_then(Value::as_object_mut)
    else {
        return;
    };
    properties.insert(
        "summary".to_string(),
        json!({
            "type": "string",
            "maxLength": MAX_SUMMARY,
            "description": "One or two sentences, in your own words, saying what you are doing and \
                            why. This is the only note you keep: your thinking is not retained, so \
                            on later turns this sentence is all you will have of this one. Say what \
                            you expect to happen, so a turn that did not work is one you can \
                            recognise instead of repeating.",
        }),
    );
    match tool.function.parameters.get_mut("required").and_then(Value::as_array_mut) {
        Some(required) => required.push(json!("summary")),
        None => {
            tool.function.parameters["required"] = json!(["summary"]);
        }
    }
}

/// The model's own account of a terminal call, if it gave one.
///
/// Trimmed and length-capped here rather than trusted: `maxLength` in a schema is a request, not a
/// guarantee, and this string goes to the page, the transcript and every later request.
pub fn call_summary(call: &ToolCall) -> Option<String> {
    let summary = call.arguments().ok()?.get("summary")?.as_str()?.trim().to_string();
    if summary.is_empty() {
        return None;
    }
    Some(match summary.char_indices().nth(MAX_SUMMARY) {
        Some((cut, _)) => summary[..cut].to_string(),
        None => summary,
    })
}

/// A zero-parameter tool still needs a schema, and an empty object is what every endpoint accepts.
fn no_arguments() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

/// One tool for every non-walking field action, discriminated by `move`, because a dozen separate
/// tools would be a dozen entries in every request's `tools` array for the sake of one call a
/// hundred turns.
fn use_field_move_spec() -> ToolSpec {
    let party_moves: Vec<String> =
        PARTY_MOVES.iter().map(|(name, _, why)| format!("`{name}` — {why}")).collect();
    ToolSpec::new(
        "use_field_move",
        format!(
            "ENDS THE TURN. Do something that is not walking. `move` picks which, and decides which \
             of the other arguments are needed:\n\
             - `cut` — cut down the tree the player is **facing**. Walk to a `:CutTree` action first.\n\
             - {}. Each takes an optional `slot`; without one the first Pokémon that knows the move \
             is used.\n\
             - `fly` — fly to `map`, which must be a town you have already visited with a Pokémon \
             Center.\n\
             - `teach` — teach the HM or TM `item` to the Pokémon in `slot`.\n\
             - `evolve` — use the evolution stone `item` on the Pokémon in `slot`.\n\
             - `use_item` — face `target` and use bag `item` on it (the Poké Flute on Snorlax, the \
             Card Key on a door).\n\
             - `toss_item` — throw `item` away to free a bag slot. The bag holds only 20 kinds.\n\
             - `push_boulder` — shove the boulder at `target` one tile in `direction`. Strength must \
             be armed first.\n\
             - `reorder_party` — make the Pokémon in `slot` the party leader.\n\
             - `interact` — stand next to `target`, face it and press A. This is how every hidden \
             thing in the game is found: hidden items, the Vermilion Gym bins, the Pokémon Mansion \
             switches.\n\
             Surf is not here — the agent mounts it by itself as soon as a route crosses water.",
            party_moves.join("\n- "),
        ),
        json!({
            "type": "object",
            "properties": {
                "move": {
                    "type": "string",
                    "enum": field_move_names(),
                    "description": "Which field action to take.",
                },
                "slot": { "type": "integer", "minimum": 0, "maximum": 5, "description": "A party slot, 0-based." },
                "item": { "type": "string", "description": "A bag item, named as `read_bag` names it." },
                "map": { "type": "string", "description": "A map name, for `fly`." },
                "target": {
                    "type": "object",
                    "properties": { "x": { "type": "integer" }, "y": { "type": "integer" } },
                    "required": ["x", "y"],
                    "additionalProperties": false,
                    "description": "A tile on the current map, in the coordinates `read_map` uses.",
                },
                "direction": { "type": "string", "enum": ["up", "down", "left", "right"] },
                "facing": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "For `interact`: approach so the player ends up facing this way. \
                                    Rarely needed; the Pokémon Mansion switches want `up`.",
                },
            },
            "required": ["move"],
            "additionalProperties": false,
        }),
    )
}

fn field_move_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = PARTY_MOVES.iter().map(|(name, _, _)| *name).collect();
    names.extend([
        "cut", "fly", "teach", "evolve", "use_item", "toss_item", "push_boulder", "reorder_party",
        "interact",
    ]);
    names
}

/// ⚠️ **The last resort, and it is offered as one.** Raw presses pre-empt the whole state machine and
/// reset it to idle afterwards, so a model that reaches for this instead of the action menu will
/// walk the player into a wall. It exists because the action menu is the agent's model of the game
/// rather than the game — §17's risk 1 — and somewhere it is incomplete a raw button is the only way
/// through.
fn press_buttons_spec() -> ToolSpec {
    ToolSpec::new(
        "press_buttons",
        format!(
            "ENDS THE TURN. Press these buttons in order, one at a time, then hand control back to \
             the agent. **A last resort.** The agent normally does all the button pressing, and \
             pressing them yourself interrupts whatever it was doing — use this only when the game \
             is somewhere the action menu does not describe, such as an unmodelled menu or a screen \
             that has stopped responding. Up to {MANUAL_INPUT_CAPACITY} presses; anything past that \
             is dropped."
        ),
        json!({
            "type": "object",
            "properties": {
                "buttons": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MANUAL_INPUT_CAPACITY,
                    "items": { "type": "string", "enum": ["up", "down", "left", "right", "a", "b", "start", "select"] },
                }
            },
            "required": ["buttons"],
            "additionalProperties": false,
        }),
    )
}

/// Every tool that does **not** end a turn, *for this kind*: the reads this kind is offered, the
/// screenshot and W6b's TODO tools. The contract at the bottom of each turn names them all, because
/// a model that believes `todo_add` was its terminal call simply stops playing.
///
/// ⚠️ **Per kind, since the reads are.** A contract that named a read the request did not carry
/// would be inviting exactly the call `classify` has to reject.
pub fn non_terminal_names(kind: DecisionKind) -> Vec<&'static str> {
    reads_for(kind).map(|tool| tool.name).chain(TODO_TOOL_NAMES.iter().copied()).collect()
}

pub fn terminal_names(kind: DecisionKind) -> &'static [&'static str] {
    match kind {
        DecisionKind::Overworld => &["choose_action", "use_field_move", "press_buttons", "wait"],
        DecisionKind::Battle => &["choose_battle_action", "press_buttons", "wait"],
        DecisionKind::Nickname => &["set_nickname", "wait"],
        DecisionKind::MartPurchase => &["buy_item", "wait"],
        DecisionKind::ForgetMove => &["forget_move", "wait"],
        // **W9.** There is no menu to choose from and no action the agent could execute, so the
        // escape hatch and doing nothing are the whole of it.
        DecisionKind::Stuck => &["press_buttons", "wait"],
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
    // ⚠️ A read that exists but is not offered in *this* kind is answered like a terminal tool from
    // the wrong kind: named, with the reason. Falling through to "there is no tool called
    // `read_map`" would be a lie, and one a model in a battle could not act on.
    if let Some(tool) = read_tool(name) {
        if !tool.kinds.contains(&kind) {
            return CallKind::Rejected(format!(
                "`{name}` is not available in a {} turn. The reads you have here are {}.",
                kind.label(),
                non_terminal_names(kind).join(", "),
            ));
        }
        return match name == SCREENSHOT {
            true => CallKind::Screenshot,
            false => CallKind::Read,
        };
    }

    let arguments = match call.arguments() {
        Ok(arguments) => arguments,
        Err(failure) => {
            return CallKind::Rejected(format!(
                "{failure}. Send the arguments as a JSON object and try again."
            ));
        }
    };

    if let Some(todo) = classify_todo(name, &arguments) {
        return todo;
    }

    match name {
        "choose_action" if kind == DecisionKind::Overworld => match string_argument(&arguments, "id") {
            Ok(id) => CallKind::Terminal(Terminal::ChooseAction { id }),
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "choose_battle_action" if kind == DecisionKind::Battle => match string_argument(&arguments, "id") {
            Ok(id) => CallKind::Terminal(Terminal::ChooseBattleAction { id }),
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "use_field_move" if kind == DecisionKind::Overworld => match field_move_arguments(&arguments) {
            Ok(request) => CallKind::Terminal(Terminal::UseFieldMove(request)),
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "press_buttons"
            if matches!(kind, DecisionKind::Overworld | DecisionKind::Battle | DecisionKind::Stuck) =>
        {
            match button_arguments(&arguments) {
                Ok(buttons) => CallKind::Terminal(Terminal::PressButtons { buttons }),
                Err(complaint) => CallKind::Rejected(complaint),
            }
        }
        "set_nickname" if kind == DecisionKind::Nickname => {
            // An absent `name` is the answer "keep the default", and so is an empty string — the
            // naming screen treats an empty buffer as a decline, so agreeing with it here means the
            // two cannot disagree.
            let name = arguments
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| name.chars().take(MAX_NICKNAME).collect::<String>());
            CallKind::Terminal(Terminal::SetNickname { name })
        }
        "buy_item" if kind == DecisionKind::MartPurchase => {
            match arguments.get("item").and_then(Value::as_str).filter(|name| !name.is_empty()) {
                None => CallKind::Terminal(Terminal::BuyItem { item: None }),
                Some(name) => match item_by_name(name) {
                    Some(item) => {
                        let quantity = arguments
                            .get("quantity")
                            .and_then(Value::as_u64)
                            .unwrap_or(1)
                            .clamp(1, 99) as u8;
                        CallKind::Terminal(Terminal::BuyItem { item: Some(BagItem::new(item, quantity)) })
                    }
                    None => CallKind::Rejected(format!(
                        "`{name}` is not an item this game has. Copy a name from the stock list exactly."
                    )),
                },
            }
        }
        "forget_move" if kind == DecisionKind::ForgetMove => {
            match arguments.get("slot").and_then(Value::as_u64) {
                None => CallKind::Terminal(Terminal::ForgetMove { slot: None }),
                Some(slot) if slot < 4 => CallKind::Terminal(Terminal::ForgetMove { slot: Some(slot as u8) }),
                Some(slot) => CallKind::Rejected(format!(
                    "There is no move slot {slot}; a Pokémon has four, numbered 0 to 3. Omit `slot` \
                     to decline the new move instead."
                )),
            }
        }
        "wait" => match arguments.get("ticks").and_then(Value::as_u64) {
            Some(ticks) => CallKind::Terminal(Terminal::Wait {
                ticks: ticks.clamp(1, u64::from(MAX_WAIT_TICKS)) as u16,
            }),
            None => CallKind::Rejected("`wait` needs a whole number of `ticks`.".to_string()),
        },
        // A terminal tool from another decision kind. It exists, so saying "unknown tool" would be
        // actively misleading; what the model needs is the name of the one that does apply.
        "choose_action" | "choose_battle_action" | "use_field_move" | "press_buttons"
        | "set_nickname" | "buy_item" | "forget_move" => CallKind::Rejected(format!(
            "`{name}` is not available in a {} turn. End this turn with one of: {}.",
            kind.label(),
            terminal_names(kind).join(", "),
        )),
        other => CallKind::Rejected(format!(
            "There is no tool called `{other}`. The tools that do not end the turn are {}; end the \
             turn with one of: {}.",
            non_terminal_names(kind).join(", "),
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

// ── Parsing the awkward arguments ────────────────────────────────────────────────────────────────

fn field_move_arguments(arguments: &Value) -> Result<FieldMoveRequest, String> {
    let which = string_argument(arguments, "move")?;
    let which = which.trim().to_ascii_lowercase();

    let slot = || -> Result<u8, String> {
        match arguments.get("slot").and_then(Value::as_u64) {
            Some(slot) if slot < 6 => Ok(slot as u8),
            Some(slot) => Err(format!("There is no party slot {slot}; a party has 0 to 5.")),
            None => Err(format!("`{which}` needs a `slot` — which party member to use it on.")),
        }
    };
    let item = |key: &str| -> Result<ItemId, String> {
        let name = string_argument(arguments, key)?;
        item_by_name(&name)
            .ok_or_else(|| format!("`{name}` is not an item this game has. `read_bag` names them as they are spelled."))
    };
    let target = || -> Result<Point8, String> {
        let target = arguments
            .get("target")
            .ok_or_else(|| format!("`{which}` needs a `target` tile, as `{{\"x\": …, \"y\": …}}`."))?;
        let coordinate = |axis: &str| {
            target
                .get(axis)
                .and_then(Value::as_u64)
                .filter(|value| *value < 256)
                .ok_or_else(|| format!("`target.{axis}` must be a tile coordinate on the current map."))
        };
        Ok(Point8 { x: coordinate("x")? as u8, y: coordinate("y")? as u8 })
    };

    if let Some((_, name, _)) = PARTY_MOVES.iter().find(|(label, _, _)| *label == which) {
        let slot = match arguments.get("slot") {
            Some(Value::Null) | None => None,
            Some(_) => Some(slot()?),
        };
        return Ok(FieldMoveRequest::PartyMove { name: *name, slot });
    }

    match which.as_str() {
        "cut" => Ok(FieldMoveRequest::Cut),
        "fly" => {
            let name = string_argument(arguments, "map")?;
            map_by_name(&name)
                .map(|to| FieldMoveRequest::Fly { to })
                .ok_or_else(|| format!("`{name}` is not a map. `read_world_graph` lists the ones you know."))
        }
        "teach" => Ok(FieldMoveRequest::Teach { item: item("item")?, slot: slot()? }),
        "evolve" => Ok(FieldMoveRequest::Evolve { stone: item("item")?, slot: slot()? }),
        "use_item" => Ok(FieldMoveRequest::UseItem { item: item("item")?, target: target()? }),
        "toss_item" => Ok(FieldMoveRequest::TossItem { item: item("item")? }),
        "push_boulder" => {
            let direction = string_argument(arguments, "direction")?;
            Ok(FieldMoveRequest::PushBoulder {
                boulder: target()?,
                direction: button_by_name(&direction)
                    .filter(|button| {
                        matches!(button, JoypadButton::Up | JoypadButton::Down | JoypadButton::Left | JoypadButton::Right)
                    })
                    .ok_or_else(|| format!("`{direction}` is not a direction: up, down, left or right."))?,
            })
        }
        "reorder_party" => Ok(FieldMoveRequest::ReorderParty { slot: slot()? }),
        "interact" => Ok(FieldMoveRequest::Interact {
            target: target()?,
            facing: match arguments.get("facing").and_then(Value::as_str) {
                None => None,
                Some(facing) => Some(facing_by_name(facing).ok_or_else(|| {
                    format!("`{facing}` is not a direction: up, down, left or right.")
                })?),
            },
        }),
        other => Err(format!(
            "`{other}` is not one of the field moves. They are: {}.",
            field_move_names().join(", "),
        )),
    }
}

fn button_arguments(arguments: &Value) -> Result<Vec<JoypadButton>, String> {
    let list = arguments
        .get("buttons")
        .and_then(Value::as_array)
        .ok_or_else(|| "`press_buttons` needs a `buttons` array.".to_string())?;
    if list.is_empty() {
        return Err("`buttons` was empty, so nothing would have been pressed.".to_string());
    }
    list.iter()
        .map(|button| {
            let name = button.as_str().unwrap_or_default();
            button_by_name(name).ok_or_else(|| {
                format!("`{name}` is not a button: up, down, left, right, a, b, start or select.")
            })
        })
        // Silently dropping the tail would be a lie about what was pressed, so say so instead.
        .take(MANUAL_INPUT_CAPACITY)
        .collect()
}

/// Compare two names the way a model spells them against the way the code spells them: `"HM01 Cut"`,
/// `"hm01_cut"` and `"Hm01Cut"` are all the same item, and none of the three is worth a rejection.
fn same_name(a: &str, b: &str) -> bool {
    let normalise = |name: &str| -> String {
        name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
    };
    normalise(a) == normalise(b)
}

/// ⚠️ `ItemId` has no `FromStr`, and giving it one would mean a `strum` derive on an enum three
/// hundred other lines already index by discriminant. Scanning 255 discriminants once per tool call
/// is free by comparison, and it cannot go stale.
pub fn item_by_name(name: &str) -> Option<ItemId> {
    (0..=u8::MAX).filter_map(ItemId::from_repr).find(|item| same_name(name, &item.to_string()))
}

pub fn map_by_name(name: &str) -> Option<Map> {
    use strum::IntoEnumIterator;
    Map::iter().find(|map| same_name(name, &map.to_string()))
}

fn button_by_name(name: &str) -> Option<JoypadButton> {
    use strum::IntoEnumIterator;
    JoypadButton::iter().find(|button| same_name(name, &button.to_string()))
}

fn facing_by_name(name: &str) -> Option<PlayerFacingDirection> {
    [
        PlayerFacingDirection::Up,
        PlayerFacingDirection::Down,
        PlayerFacingDirection::Left,
        PlayerFacingDirection::Right,
    ]
    .into_iter()
    .find(|facing| same_name(name, &facing.to_string()))
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
) -> ToolAnswer {
    // ⚠️ **The picture is not drawn here.** `read_map` hands the worker the map it already has and
    // the worker renders it — see [`crate::llm::map_image`]'s module note on why a PNG encode must
    // not happen on the thread running the game. The clone is of a `MetaTileMap` the policy is
    // already cloning once per poll.
    let map = match call.function.name.as_str() {
        "read_map" => Some(Box::new(state.map.clone())),
        _ => None,
    };
    let value = match call.function.name.as_str() {
        "read_map" => serde_json::to_value(observe::map_view(state)),
        "read_party" => serde_json::to_value(observe::party(state)),
        "read_bag" => serde_json::to_value(observe::bag(state, api)),
        "read_battle" => serde_json::to_value(observe::battle(state)),
        READ_ROUTE => serde_json::to_value(route_answer(call, state, graph)),
        other => Ok(json!({ "error": format!("`{other}` is not a read tool") })),
    };
    match value.and_then(|value| serde_json::to_string(&value)) {
        Ok(json) => ToolAnswer { json, map, is_dark: state.map_is_dark },
        // Serialising a view cannot fail in practice, but a tool result is a string and the
        // alternative to this line is an `unwrap` on the worker's critical path.
        Err(failure) => ToolAnswer::text(
            format!("{{\"error\": \"could not encode the result: {failure}\"}}")),
    }
}

/// [`READ_ROUTE`], answered. Four outcomes, and each is a different thing for the model to do next,
/// which is why none of them is an `error` string:
///
/// - **no `to`** — the maps that have been walked, which is the only set `to` can be drawn from.
/// - **a name that is not a map** — a spelling problem, and correctable.
/// - **a map that has not been visited** — genuinely useful: the way there has to be *found*, not
///   recalled, and the graph is saying so rather than failing.
/// - **a route** — the maps in order, with how each one is entered.
fn route_answer(call: &ToolCall, state: &GameState, graph: &WorldGraph) -> Value {
    let requested = call
        .arguments()
        .ok()
        .and_then(|arguments| arguments.get("to").and_then(Value::as_str).map(str::to_string))
        .filter(|name| !name.trim().is_empty());

    let visited = || -> Vec<String> {
        observe::known_maps(graph).into_iter().map(|map| format!("{map}")).collect()
    };

    let Some(requested) = requested else {
        return json!({ "from": format!("{}", state.map.map), "visited": visited() });
    };
    let Some(to) = map_by_name(&requested) else {
        return json!({
            "to": requested,
            "error": format!("`{requested}` is not a map in this game. `{READ_ROUTE}` with no `to` \
                              lists the ones you have visited."),
        });
    };

    match observe::route(graph, state.map.map, to) {
        Some(hops) => json!({ "from": format!("{}", state.map.map), "to": format!("{to}"), "route": hops }),
        None => json!({
            "to": format!("{to}"),
            "reachable": false,
            "note": format!("You have not been to {to} — or no route to it crosses ground you have \
                             already walked. You will have to explore towards it. `{READ_ROUTE}` \
                             with no `to` lists where you have been."),
        }),
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
///
/// ⚠️ **`MetaTile::id_kind`, never its `Display`.** The `Display` is prose written for the status
/// log ("the warp to OaksLab") and is free to be reworded; an id is a key that a model quotes back
/// and that is re-resolved by string equality, so it takes the variant name — except for a person,
/// who is named instead of being called a "sprite". See `MetaTile::id_kind`.
///
/// ⚠️ **The map prefix looks redundant beside the turn's own header and is not.** `resolve_overworld`
/// re-mints ids against whatever map the player is on *now*, and the answer to a turn can land after
/// a warp — so without the prefix, `5,6:Warp` chosen in Oak's lab could match a warp that happens to
/// sit at (5, 6) in Pallet Town and be carried out silently. With it, a stale id simply fails to
/// resolve, which is a sentence the model is told.
pub fn overworld_id(state: &GameState, action: &OverworldAction) -> String {
    let destination = action.destination;
    format!("{}:{},{}:{}", state.map.map, destination.x, destination.y, action.tile.id_kind())
}

/// What one menu row says *beyond* its id.
///
/// ⚠️ **Not `OverworldAction`'s `Display`, and the difference is what this exists for.** That prose
/// is written for a person reading the SDL console — it leads with a verb and names the tile — so
/// beside an id that already ends in `:Warp` it repeats the row's own key: "`…:Warp` — Warp →
/// PalletTown (12, 11)". Here the kind is in the id, so the row carries only what the id cannot:
/// *which* map a door leads to.
///
/// ⚠️ **A person's row is now the bare distance**, because `MetaTile::id_kind` puts the name in the
/// id itself: `` `OaksLab:2,2:Pokedex1` — 9 steps ``. Naming them again here would be the same
/// repetition this function exists to avoid, one variant later.
///
/// ⚠️ **A warp's `to_position` is dropped outright.** It is a coordinate on a map the model has not
/// seen and cannot act on — it does not choose where to land, only which warp to take — so it was
/// nine characters of noise on every door in the game.
fn overworld_description(action: &OverworldAction) -> String {
    let steps = action.route.len();
    let target = match action.tile {
        crate::pokemon::tile::MetaTile::Warp { to_map, .. }
        | crate::pokemon::tile::MetaTile::Connection { to_map, .. } => format!("to {to_map}, "),
        crate::pokemon::tile::MetaTile::ConnectionWater(to_map) => format!("surf to {to_map}, "),
        // Grass, a PC, a tree to cut, a person: the id's last field is the whole of what it is, and
        // "Walk in grass" beside `…:Grass` says it a second time.
        _ => String::new(),
    };
    format!("{target}{steps} steps")
}

/// Everything reachable from where the player is standing. Sorted, so two reads of an unchanged map
/// produce the same menu — `actions()` walks a `HashSet` and would otherwise reshuffle, which reads
/// to a model as the world having moved.
pub fn overworld_menu(state: &GameState) -> Vec<MenuItem> {
    let mut actions = state.map.actions();
    // `id_kind`, not `kind`: two people can share the tile an action approaches them from, and
    // "Sprite" == "Sprite" leaves that pair to `sort_by_key`'s stability over a `HashSet` walk.
    actions.sort_by_key(|action| (action.destination.y, action.destination.x, action.tile.id_kind()));
    actions
        .iter()
        .map(|action| MenuItem {
            id: overworld_id(state, action),
            description: overworld_description(action),
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

/// What the mart in front of the player sells, read from its own ROM list at the poll (see
/// [`ApiSnapshot`]). The id is the item's name, because that is what `buy_item` takes.
pub fn mart_menu(snapshot: &ApiSnapshot) -> Vec<MenuItem> {
    snapshot
        .mart_stock
        .iter()
        .map(|(item, price)| MenuItem {
            id: item.to_string(),
            description: match price {
                Some(price) => format!("¥{price}"),
                // Every mart item has a price; a missing one means the ROM's table did not have it,
                // which is worth showing rather than hiding behind a plausible number.
                None => "price unknown".to_string(),
            },
        })
        .collect()
}

/// The four moves the forget prompt is choosing between. The id is the slot, which is what
/// `forget_move` takes — there is no reordering hazard here, because the prompt itself is indexed by
/// slot and lives for as long as the question does.
pub fn forget_menu(current: &[PokemonMove]) -> Vec<MenuItem> {
    current
        .iter()
        .enumerate()
        .map(|(slot, known)| MenuItem {
            id: slot.to_string(),
            description: format!("{} — {} pp", known.name, known.pp),
        })
        .collect()
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

    /// Oak's lab just after the starter is taken: a party of one, an ordinary bag, and a map with
    /// no trees on it — which between them exercise every check [`resolve_field_move`] makes.
    fn fixture_state() -> GameState {
        let mut gb = crate::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/oaks-lab-just-got-squirtle.bin"))
            .expect("the committed fixture loads");
        { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }.expect("the fixture has a readable state")
    }

    /// Every kind, so a loop that meant "all of them" cannot quietly stop meaning it when a seventh
    /// is added.
    const KINDS: [DecisionKind; 6] = ALL_KINDS;

    /// **A menu row carries what its id cannot, and nothing else.**
    ///
    /// The action menu is in every overworld turn and is the longest thing in one — a city map runs
    /// to a couple of dozen rows — so what each row repeats, it repeats a couple of dozen times a
    /// turn, for the length of the run.
    #[test]
    fn a_menu_row_does_not_repeat_its_own_id() {
        let state = fixture_state();
        let menu = overworld_menu(&state);
        let rows: Vec<String> =
            menu.iter().map(|item| format!("- `{}` — {}", item.id, item.description)).collect();

        assert!(rows.contains(&"- `OaksLab:5,11:Warp` — to PalletTown, 10 steps".to_string()), "{rows:#?}");
        // ⚠️ A person is *named by the id* and the row is the bare distance. The old pair was
        // "`OaksLab:2,2:Sprite` — Pokedex 1, 9 steps", which spends a word of the emulator's own
        // vocabulary on the key and then has to say who is there anyway.
        assert!(rows.contains(&"- `OaksLab:2,2:Pokedex1` — 9 steps".to_string()), "{rows:#?}");
        assert!(!rows.iter().any(|row| row.contains("Sprite")),
                "no row may call a person a sprite: {rows:#?}");

        for item in &menu {
            // ⚠️ The verb is the id's own `kind` said twice: `…:Warp` — "Warp → …", `…:Sprite` —
            // "Talk to …". `OverworldAction`'s `Display` still says it that way for the SDL console,
            // where there is no id beside it.
            let kind = item.id.rsplit(':').next().expect("an id ends in its kind");
            assert!(!item.description.contains(kind),
                    "`{}` — {} repeats the kind already in its id", item.id, item.description);
            // …and a warp's landing coordinates are not in it either: the model picks which warp to
            // take, never where it comes out, so they were nine characters of noise per door.
            assert!(!item.description.contains('('), "{item:?} still carries a coordinate");
        }
    }

    /// **What the `tools` array costs, per kind, with a ceiling on each.**
    ///
    /// ⚠️ **It is paid per *completion*, not per turn.** The whole array goes out again with every
    /// request, and a turn that reads before it decides is several — so a tool description is
    /// multiplied by `GB_MAX_TOOL_STEPS` before anything the model actually says is counted.
    ///
    /// The ceilings are generous enough that rewording a description never trips them and tight
    /// enough that adding a tool to every kind, or unscoping the reads again, has to be a deliberate
    /// edit to this list. They are bytes of JSON — roughly four to the token — because that is what
    /// is measurable here; the token count depends on the endpoint's tokeniser.
    #[test]
    fn the_tool_array_stays_within_its_budget() {
        // Overworld is the big one: it carries `use_field_move`, which is a dozen field actions
        // behind one `move` discriminant precisely so it is one entry rather than twelve.
        for (kind, ceiling) in [
            // Measured 2026-08-13, after `summary` was added to every terminal tool: 8589, 4849,
            // 3220, 3642, 3538, 3953. Each ceiling has ~10% of headroom for rewording.
            //
            // ⚠️ **The jump from the 2026-08-12 figures (6875, 3773, 2530, 2952, 2848, 2877) is
            // `add_summary_argument`, and it is bought rather than leaked.** It is one property
            // repeated across every terminal tool a kind offers — `Stuck` has two and pays twice —
            // so it is the one addition here that scales with the *number* of terminals rather than
            // with the catalogue. What it buys is the only sentence the model keeps about its own
            // turn; see that function.
            (DecisionKind::Overworld, 9_400),
            (DecisionKind::Battle, 5_300),
            (DecisionKind::Nickname, 3_500),
            (DecisionKind::MartPurchase, 4_000),
            (DecisionKind::ForgetMove, 3_900),
            (DecisionKind::Stuck, 4_300),
        ] {
            let bytes = serde_json::to_string(&for_kind(kind)).expect("the specs serialise").len();
            assert!(bytes <= ceiling, "{kind:?}'s tools are {bytes} bytes, over the {ceiling} budget");
        }
    }

    /// [`READ_ROUTE`]'s four answers, which is the whole of it — and none of them is an `error`
    /// string, because each is a different thing for the model to do next.
    ///
    /// ⚠️ **An empty graph is the interesting case.** The tool replaced one that dumped every
    /// visited node, and the guarantee both share is *negative*: nothing here has been walked, so
    /// every route is `reachable: false` — which means "you have not been there", never "it does not
    /// exist". A run that read this as unreachable would stop exploring.
    #[test]
    fn a_route_answers_the_four_questions_and_never_bluffs() {
        let state = fixture_state();
        let graph = WorldGraph::new();
        let ask = |arguments: &str| -> Value {
            route_answer(&call(READ_ROUTE, arguments), &state, &graph)
        };

        // No `to`: what has been walked. Empty here, and an empty list is an answer.
        assert_eq!(ask("{}")["visited"], json!([]));
        assert_eq!(ask("{}")["from"], json!(format!("{}", state.map.map)));
        assert_eq!(ask(r#"{"to":""}"#)["visited"], json!([]), "a blank name is no name");

        // A name that is not a map at all: correctable, and it says how.
        let nonsense = ask(r#"{"to":"Kanto Safari Wildlife Park"}"#);
        assert!(nonsense["error"].as_str().expect("a sentence").contains("not a map"), "{nonsense}");

        // A real map nobody has walked to. ⚠️ Not an error: the way there has to be *found*.
        let unwalked = ask(r#"{"to":"CeruleanCity"}"#);
        assert_eq!(unwalked["reachable"], json!(false));
        assert_eq!(unwalked["to"], json!(format!("{}", Map::CeruleanCity)));
        assert!(unwalked["note"].as_str().expect("a sentence").contains("not been to"), "{unwalked}");

        // ⚠️ Spelled the way a model spells things, not the way the enum does — `map_by_name`
        // normalises, and a rejection over a space would be a rejection over nothing.
        assert_eq!(ask(r#"{"to":"cerulean city"}"#), unwalked);

        // The whole graph is never serialised, whatever is asked. That was the point.
        assert!(!ask("{}").to_string().contains("edges"));
    }

    /// ⚠️ **A battle menu row is prose, and `BattleAction`'s `Display` is what makes it so.**
    ///
    /// The switch rows were `{:?}` — `PKMN   PokemonSummary { species: Charizard, current_hp: 360,
    /// status: None, types: [Fire, Flying], moves: [Some(PokemonMove { name: Flamethrower, pp: 15
    /// }), …] }` — which is around 500 bytes of Rust syntax per switchable party member, in the menu
    /// of every battle turn for the length of a run. Same class of bug as `MetaTile`'s and
    /// `PokemonStatus`' old `strum` derives, and found the same way one would hope: by reading
    /// `prompt::tests::probe_turn_requests`' output.
    #[test]
    fn a_battle_menu_row_is_a_sentence_and_not_a_debug_dump() {
        let switch = BattleAction::SwitchPokemon {
            slot: 1,
            pokemon: crate::pokemon::pokemon::PokemonSummary {
                species: crate::pokemon::species::PokemonSpecies::Charizard,
                current_hp: 200,
                status: crate::pokemon::status::PokemonStatus::None,
                types: [crate::pokemon::pokemon::PokemonType::Fire; 2],
                level: 100,
                moves: [None, None, None, None],
                stats: crate::pokemon::pokemon::PokemonStats {
                    attack: 1, defense: 1, speed: 1, special: 1, hp: 360,
                },
                disabled_move_slot: None,
            },
        };
        assert_eq!(format!("{switch}"), "PKMN   Charizard Lv100 — 200/360 HP");

        // A healthy Pokémon says nothing about its status; `PokemonStatus`' own `Display` is
        // `strum`'s, so an unconditional one would read `, None` — a missing value, not good news.
        assert!(!format!("{switch}").contains("None"));
        let poisoned = match switch {
            BattleAction::SwitchPokemon { slot, mut pokemon } => {
                pokemon.status = crate::pokemon::status::PokemonStatus::Poisoned;
                BattleAction::SwitchPokemon { slot, pokemon }
            }
            other => other,
        };
        assert_eq!(format!("{poisoned}"), "PKMN   Charizard Lv100 — 200/360 HP, Poisoned");
    }

    /// **Every terminal tool asks for a summary, and nothing else does.**
    ///
    /// ⚠️ This is the only sentence the model keeps about its own turn. Reasoning arrives on a
    /// channel that is never sent back, and most models emit no `content` beside a tool call, so
    /// without it the assistant side of the history is a column of bare JSON: what was done, never
    /// why. A model reading that back has no record of having *tried* anything.
    ///
    /// It is not on the reads because a read is not a decision — one per turn is the point, and
    /// asking for one on `read_party` would buy the same sentence three times at three times the
    /// price.
    #[test]
    fn every_terminal_tool_asks_the_model_to_say_why() {
        for kind in ALL_KINDS {
            for tool in for_kind(kind) {
                let has_summary = tool.function.parameters["properties"].get("summary").is_some();
                let required = tool.function.parameters["required"]
                    .as_array()
                    .is_some_and(|required| required.iter().any(|name| name == "summary"));
                match terminal_names(kind).contains(&tool.function.name) {
                    true => {
                        assert!(has_summary, "{kind:?}'s `{}` has no summary", tool.function.name);
                        assert!(required, "{kind:?}'s `{}` does not require it", tool.function.name);
                    }
                    false => assert!(!has_summary, "`{}` is not a decision", tool.function.name),
                }
                // ⚠️ Every terminal schema is `additionalProperties: false`, so an argument that is
                // not declared is not merely ignored — the call is schema-invalid.
                assert_eq!(tool.function.parameters["additionalProperties"], json!(false),
                           "`{}` would accept an undeclared argument", tool.function.name);
            }
        }
    }

    /// ⚠️ **Required of the model, optional to the parser**, and the asymmetry is deliberate:
    /// rejecting a terminal call for a missing summary would not end the turn — it becomes another
    /// tool result and spends another of `GB_MAX_TOOL_STEPS` — so a model that forgot it would be
    /// pushed towards the forced `wait` rather than towards remembering.
    #[test]
    fn a_summary_is_read_off_the_call_and_never_demanded() {
        let call = |arguments: &str| ToolCall {
            id: "call_1".to_string(),
            kind: "function".to_string(),
            function: crate::llm::protocol::FunctionCall {
                name: "wait".to_string(),
                arguments: arguments.to_string(),
            },
        };

        assert_eq!(
            call_summary(&call(r#"{"ticks": 5, "summary": "  letting the battle text finish  "}"#)),
            Some("letting the battle text finish".to_string()),
            "trimmed, because it is printed on a page",
        );
        // A turn that omits it is still a turn: the decision is carried out either way.
        assert_eq!(call_summary(&call(r#"{"ticks": 5}"#)), None);
        assert_eq!(call_summary(&call(r#"{"ticks": 5, "summary": "   "}"#)), None, "blank is absent");
        assert_eq!(call_summary(&call("not json")), None, "and a broken call is not a panic");

        // `maxLength` in a schema is a request. This string reaches the page, the transcript and
        // every later request, so the cap is applied here rather than trusted.
        let long = "x".repeat(MAX_SUMMARY * 2);
        let capped = call_summary(&call(&format!(r#"{{"summary": "{long}"}}"#))).expect("present");
        assert_eq!(capped.chars().count(), MAX_SUMMARY);
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

        // The three menu prompts are single-question turns: their one terminal tool, and `wait`.
        // Offering `choose_action` at a naming screen would let a turn end in a way the poll site
        // cannot carry out.
        for kind in [DecisionKind::Nickname, DecisionKind::MartPurchase, DecisionKind::ForgetMove] {
            let offered = names(kind);
            for elsewhere in ["choose_action", "choose_battle_action", "use_field_move", "press_buttons"] {
                assert!(!offered.contains(&elsewhere), "{kind:?} must not offer {elsewhere}");
            }
        }
        assert!(names(DecisionKind::Nickname).contains(&"set_nickname"));
        assert!(names(DecisionKind::MartPurchase).contains(&"buy_item"));
        assert!(names(DecisionKind::ForgetMove).contains(&"forget_move"));
        // `press_buttons` is the escape hatch for a game the action menu does not describe, which is
        // the overworld and a battle — not a menu whose one question is already on offer.
        assert!(names(DecisionKind::Overworld).contains(&"press_buttons"));
        assert!(names(DecisionKind::Battle).contains(&"press_buttons"));

        // **W9.** The watchdog's turn is the one where `press_buttons` is not a last resort but the
        // only resort: there is no menu, because the agent is not offering one. Anything else on
        // offer would be a turn ending in a decision a wedged agent cannot carry out.
        let stuck = names(DecisionKind::Stuck);
        assert_eq!(
            stuck.iter().filter(|name| terminal_names(DecisionKind::Stuck).contains(name)).count(),
            2,
        );
        assert!(stuck.contains(&"press_buttons") && stuck.contains(&"wait"));
        for elsewhere in ["choose_action", "choose_battle_action", "use_field_move", "set_nickname",
                          "buy_item", "forget_move"] {
            assert!(!stuck.contains(&elsewhere), "a stuck turn must not offer {elsewhere}");
        }
        // …and the reads are all there, because working out *why* it is stuck is the useful thing to
        // do before pressing anything.
        assert!(stuck.contains(&"read_map") && stuck.contains(&SCREENSHOT));
        assert!(!names(DecisionKind::Battle).contains(&"use_field_move"), "field moves are overworld-only");

        for kind in KINDS {
            let offered = names(kind);
            assert!(offered.contains(&"wait"), "{kind:?} must always be able to wait");
            // The contract restated in the prompt has to match the array actually sent, or the two
            // drift and the model is told about a tool it does not have.
            for terminal in terminal_names(kind) {
                assert!(offered.contains(terminal), "{kind:?} promises {terminal} but does not offer it");
            }
            assert_eq!(
                offered.len(),
                reads_for(kind).count() + TODO_TOOL_NAMES.len() + terminal_names(kind).len(),
                "a turn is offered its own reads, W6b's TODO tools, and its own terminal tools",
            );
        }
    }

    /// **The reads are scoped too, and the reason is not only tokens.** A tool that can only ever
    /// answer `null` — `read_battle` in the overworld, a map in a battle — is a round trip the model
    /// has to spend to find that out, and an invitation to spend it. A nickname prompt used to carry
    /// the whole catalogue in order to answer with one word.
    #[test]
    fn reads_are_scoped_per_kind_too() {
        assert!(!names(DecisionKind::Battle).contains(&"read_map"), "there is no map in a battle");
        assert!(!names(DecisionKind::Battle).contains(&READ_ROUTE));
        assert!(!names(DecisionKind::Overworld).contains(&"read_battle"), "it can only answer null");

        // ⚠️ The forget-move prompt legitimately fires mid-fight — it is the one menu kind that
        // pre-empts a battle turn — so which move to drop is a question the battle can answer.
        assert!(names(DecisionKind::ForgetMove).contains(&"read_battle"));

        // The screen is the only thing that can explain an unfamiliar menu or a wedged agent, so it
        // is the one read every kind keeps.
        for kind in KINDS {
            assert!(names(kind).contains(&SCREENSHOT), "{kind:?} cannot look at the screen");
        }

        // A single-question turn carries almost nothing. Stated as a number so that adding a read
        // back to every kind has to be a deliberate edit to this line: naming a Pokémon used to
        // arrive with all eight reads and four note tools — fourteen entries to answer with a word.
        assert_eq!(names(DecisionKind::Nickname), ["read_party", SCREENSHOT, "todo_add", "todo_complete",
                                                   "set_nickname", "wait"]);

        // ⚠️ A read that exists but is not offered *here* is told which turn it belongs to. Falling
        // through to "there is no tool called `read_map`" would be a lie the model cannot act on.
        let rejected = classify(DecisionKind::Battle, &call("read_map", "{}"));
        let CallKind::Rejected(complaint) = rejected else { panic!("read_map is not a battle read") };
        assert!(complaint.contains("not available in a battle turn"), "{complaint}");
        assert!(complaint.contains("read_battle"), "it has to name what *is* here: {complaint}");
        assert!(matches!(classify(DecisionKind::Overworld, &call("read_map", "{}")), CallKind::Read));
    }

    /// Every schema must be a JSON Schema object with the properties it claims — a malformed one is
    /// a 400 from the endpoint on the very first turn of a run.
    #[test]
    fn every_schema_is_a_well_formed_object() {
        for kind in KINDS {
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

    // ── W5 ───────────────────────────────────────────────────────────────────────────────────────

    /// A model writes `"HM01 Cut"`, the code writes `Hm01Cut`, and `read_bag` writes `Hm01Cut`. All
    /// three name the same thing, and rejecting two of them would be a rejection the model cannot
    /// learn its way out of — nothing it is shown spells the item any other way.
    #[test]
    fn names_are_matched_the_way_a_model_spells_them() {
        for spelling in ["Hm01Cut", "hm01_cut", "HM01 Cut", "hm01cut"] {
            assert_eq!(item_by_name(spelling), Some(ItemId::Hm01Cut), "{spelling}");
        }
        assert_eq!(item_by_name("Poke Ball"), Some(ItemId::PokeBall));
        assert_eq!(item_by_name("a potion of healing"), None, "close is not the same as right");
        assert_eq!(map_by_name("pallet town"), Some(Map::PalletTown));
        assert_eq!(button_by_name("START"), Some(JoypadButton::Start));
        assert_eq!(facing_by_name("up"), Some(PlayerFacingDirection::Up));
        assert_eq!(button_by_name("shoulder"), None);
    }

    /// The escape hatch has to reject what it cannot press rather than silently drop it — a queue
    /// that is one button short walks the player somewhere nobody asked for.
    #[test]
    fn press_buttons_parses_a_sequence_and_refuses_what_is_not_one() {
        let CallKind::Terminal(Terminal::PressButtons { buttons }) = classify(
            DecisionKind::Overworld,
            &call("press_buttons", r#"{"buttons":["b","b","start"]}"#),
        ) else {
            panic!("a list of real buttons is a decision");
        };
        assert_eq!(buttons, [JoypadButton::B, JoypadButton::B, JoypadButton::Start]);

        for bad in [r#"{"buttons":["b","x"]}"#, r#"{"buttons":[]}"#, "{}"] {
            assert!(
                matches!(classify(DecisionKind::Overworld, &call("press_buttons", bad)), CallKind::Rejected(_)),
                "{bad} should have been rejected",
            );
        }
        // …and the agent's own cap is the cap here, so a runaway list is trimmed rather than
        // half-delivered by a queue that silently stops accepting.
        let many = format!(r#"{{"buttons":{}}}"#,
            serde_json::to_string(&vec!["a"; MANUAL_INPUT_CAPACITY * 2]).unwrap());
        let CallKind::Terminal(Terminal::PressButtons { buttons }) =
            classify(DecisionKind::Overworld, &call("press_buttons", &many))
        else {
            panic!("an over-long list is still a decision");
        };
        assert_eq!(buttons.len(), MANUAL_INPUT_CAPACITY);
    }

    /// `screenshot` is a read as far as the turn contract goes, but it never reaches the emulator
    /// thread — the worker answers it. Classifying it as an ordinary `Read` would send it to the
    /// policy, which has no idea what to do with it and would answer "not a read tool".
    #[test]
    fn a_screenshot_is_classified_apart_from_the_other_reads() {
        assert!(matches!(classify(DecisionKind::Battle, &call(SCREENSHOT, "{}")), CallKind::Screenshot));
        assert!(matches!(classify(DecisionKind::Battle, &call("read_party", "{}")), CallKind::Read));
        assert!(READ_TOOLS.iter().any(|tool| tool.name == SCREENSHOT), "it is still offered as a read");
    }

    /// Every argument shape `use_field_move` accepts, and the complaint each malformed one earns.
    #[test]
    fn a_field_move_call_parses_into_the_move_it_names() {
        let parse = |arguments: &str| classify(DecisionKind::Overworld, &call("use_field_move", arguments));
        let request = |arguments: &str| match parse(arguments) {
            CallKind::Terminal(Terminal::UseFieldMove(request)) => request,
            CallKind::Rejected(complaint) => panic!("{arguments} was rejected: {complaint}"),
            _ => panic!("{arguments} did not end the turn"),
        };

        assert_eq!(request(r#"{"move":"cut"}"#), FieldMoveRequest::Cut);
        assert_eq!(
            request(r#"{"move":"strength"}"#),
            FieldMoveRequest::PartyMove { name: PokemonMoveName::Strength, slot: None },
            "an omitted slot means 'whoever knows it', not slot 0",
        );
        assert_eq!(
            request(r#"{"move":"flash","slot":2}"#),
            FieldMoveRequest::PartyMove { name: PokemonMoveName::Flash, slot: Some(2) },
        );
        assert_eq!(request(r#"{"move":"fly","map":"Pewter City"}"#), FieldMoveRequest::Fly { to: Map::PewterCity });
        assert_eq!(
            request(r#"{"move":"teach","item":"Hm03Surf","slot":0}"#),
            FieldMoveRequest::Teach { item: ItemId::Hm03Surf, slot: 0 },
        );
        assert_eq!(
            request(r#"{"move":"use_item","item":"PokeFlute","target":{"x":12,"y":9}}"#),
            FieldMoveRequest::UseItem { item: ItemId::PokeFlute, target: Point8 { x: 12, y: 9 } },
        );
        assert_eq!(
            request(r#"{"move":"push_boulder","target":{"x":4,"y":5},"direction":"left"}"#),
            FieldMoveRequest::PushBoulder { boulder: Point8 { x: 4, y: 5 }, direction: JoypadButton::Left },
        );
        assert_eq!(
            request(r#"{"move":"interact","target":{"x":1,"y":2},"facing":"up"}"#),
            FieldMoveRequest::Interact {
                target: Point8 { x: 1, y: 2 },
                facing: Some(PlayerFacingDirection::Up),
            },
        );
        assert_eq!(request(r#"{"move":"reorder_party","slot":3}"#), FieldMoveRequest::ReorderParty { slot: 3 });

        // Every one of these is answerable — the model is told what is missing and can try again in
        // the same turn, which is the whole reason a bad call is a tool result and not a dead turn.
        for (arguments, expected) in [
            (r#"{"move":"teleportation"}"#, "not one of the field moves"),
            (r#"{"move":"fly","map":"Atlantis"}"#, "is not a map"),
            (r#"{"move":"teach","item":"Hm03Surf"}"#, "needs a `slot`"),
            (r#"{"move":"toss_item","item":"Sandwich"}"#, "is not an item"),
            (r#"{"move":"use_item","item":"PokeFlute"}"#, "needs a `target`"),
            (r#"{"move":"push_boulder","target":{"x":1,"y":1},"direction":"north"}"#, "is not a direction"),
            (r#"{"move":"reorder_party","slot":9}"#, "no party slot 9"),
            ("{}", "`move` is required"),
        ] {
            let CallKind::Rejected(complaint) = parse(arguments) else {
                panic!("{arguments} should have been rejected");
            };
            assert!(complaint.contains(expected), "{arguments} → {complaint}");
        }
    }

    /// The menu-prompt tools, whose whole subtlety is that *omitting* the argument is a real answer
    /// rather than a malformed call: no nickname, no purchase, no move forgotten.
    #[test]
    fn omitting_the_argument_is_an_answer_for_the_three_menu_prompts() {
        let parse = |kind, name, arguments: &str| classify(kind, &call(name, arguments));

        assert!(matches!(
            parse(DecisionKind::Nickname, "set_nickname", "{}"),
            CallKind::Terminal(Terminal::SetNickname { name: None }),
        ));
        // An empty buffer is how the naming screen itself says "keep the default", so agreeing with
        // it here means a blank `name` and an absent one cannot mean different things.
        assert!(matches!(
            parse(DecisionKind::Nickname, "set_nickname", r#"{"name":"   "}"#),
            CallKind::Terminal(Terminal::SetNickname { name: None }),
        ));
        let CallKind::Terminal(Terminal::SetNickname { name: Some(name) }) =
            parse(DecisionKind::Nickname, "set_nickname", r#"{"name":"ABCDEFGHIJKLMNOP"}"#)
        else {
            panic!("a name is a name");
        };
        assert_eq!(name.chars().count(), MAX_NICKNAME, "the buffer is {MAX_NICKNAME} characters");

        assert!(matches!(
            parse(DecisionKind::MartPurchase, "buy_item", "{}"),
            CallKind::Terminal(Terminal::BuyItem { item: None }),
        ));
        assert_eq!(
            match parse(DecisionKind::MartPurchase, "buy_item", r#"{"item":"Potion","quantity":4}"#) {
                CallKind::Terminal(Terminal::BuyItem { item }) => item,
                _ => panic!("a stocked item is a purchase"),
            },
            Some(BagItem::new(ItemId::Potion, 4)),
        );
        // An omitted quantity is one, not zero — zero would be an order the mart silently refuses.
        assert!(matches!(
            parse(DecisionKind::MartPurchase, "buy_item", r#"{"item":"Potion"}"#),
            CallKind::Terminal(Terminal::BuyItem { item: Some(BagItem { quantity: 1, .. }) }),
        ));

        assert!(matches!(
            parse(DecisionKind::ForgetMove, "forget_move", "{}"),
            CallKind::Terminal(Terminal::ForgetMove { slot: None }),
        ));
        assert!(matches!(
            parse(DecisionKind::ForgetMove, "forget_move", r#"{"slot":2}"#),
            CallKind::Terminal(Terminal::ForgetMove { slot: Some(2) }),
        ));
        assert!(
            matches!(parse(DecisionKind::ForgetMove, "forget_move", r#"{"slot":7}"#), CallKind::Rejected(_)),
            "a Pokémon has four move slots, and a cursor sent to a fifth never arrives",
        );
    }

    /// Resolution against a real game, which is where the checks that need the party and the bag
    /// live. `cut` is the sharp one: the driver cuts whatever is in front of the player, so a `cut`
    /// issued from the wrong tile opens a menu, achieves nothing and looks like the emulator hanging.
    #[test]
    fn a_field_move_is_resolved_against_the_party_and_the_bag_it_needs() {
        // Oak's lab: one Pokémon, no HMs, no trees.
        let state = fixture_state();
        let complaint = |request| match resolve_field_move(&state, &request) {
            Err(complaint) => complaint,
            Ok(resolved) => panic!("{request:?} should not have resolved to {resolved:?}"),
        };
        assert!(complaint(FieldMoveRequest::Cut).contains("facing"), "cut must check the tile in front");
        assert!(
            complaint(FieldMoveRequest::PartyMove { name: PokemonMoveName::Strength, slot: None })
                .contains("No Pokémon in the party knows"),
        );
        assert!(
            complaint(FieldMoveRequest::PartyMove { name: PokemonMoveName::Strength, slot: Some(0) })
                .contains("does not know"),
            "a named slot that does not know it is a different complaint from nobody knowing it",
        );
        assert!(complaint(FieldMoveRequest::ReorderParty { slot: 3 }).contains("no party member in slot 3"));
        assert!(complaint(FieldMoveRequest::TossItem { item: ItemId::Hm01Cut }).contains("no Hm01Cut in the bag"));

        // …and the ones that need nothing from the state resolve as themselves.
        assert_eq!(
            resolve_field_move(&state, &FieldMoveRequest::Fly { to: Map::PalletTown }),
            Ok(FieldMove::Fly { to: Map::PalletTown }),
        );
        assert_eq!(
            resolve_field_move(&state, &FieldMoveRequest::ReorderParty { slot: 0 }),
            Ok(FieldMove::ReorderParty { slot: 0 }),
        );
    }

    /// ⚠️ The party menu lists a mon's field moves in **its own move-slot order**, so the index of
    /// the one being asked for depends on what else that mon knows. Assuming zero works for an HM
    /// slave and silently uses the wrong move for anything else.
    #[test]
    fn a_party_field_moves_index_is_computed_from_the_moves_it_knows() {
        let mut state = fixture_state();
        state.pokemon.get_mut(0).expect("the fixture has a starter").moves = [
            Some(PokemonMove::with_max_pp(PokemonMoveName::Tackle)),
            Some(PokemonMove::with_max_pp(PokemonMoveName::Cut)),
            Some(PokemonMove::with_max_pp(PokemonMoveName::Strength)),
            None,
        ];

        assert_eq!(
            resolve_field_move(&state, &FieldMoveRequest::PartyMove {
                name: PokemonMoveName::Strength,
                slot: None,
            }),
            // Cut is a field move and sits in an earlier move slot, so Strength is the *second* row
            // of the field-move box — not the third, and not the first.
            Ok(FieldMove::UseFieldMove { slot: 0, move_index: 1 }),
        );
    }
}

use serde_json::{Value, json};

use poke_core::geometry::Point8;
use gb::joypad::JoypadButton;
use crate::llm::prompt::ApiSnapshot;
use crate::llm::battle_script::MAX_SOURCE as MAX_BATTLE_SCRIPT;

/// How many extra kinds one `buy_item` may order in a single mart visit.
const MAX_CHAINED_PURCHASES: usize = 3;
use crate::llm::todo::{MAX_ITEMS as MAX_TODO_ITEMS, MAX_TEXT as MAX_TODO_TEXT, TodoCall};
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
#[cfg(test)]
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::observe;
use crate::pokemon::policy::{FieldMove, battle_options, field_move_index};
use crate::pokemon::world_graph::WorldGraph;

/// Which question the agent is asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecisionKind {
    Overworld,
    Battle,
    /// The naming screen is open for a Pokémon just caught, hatched or given.
    Nickname,
    /// A mart's Buy/Sell/Quit menu just opened.
    MartPurchase,
    ForgetMove,
    /// Not a poll site: no decision point for `GB_STUCK_TIMEOUT_SECS`, so the watchdog asks.
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

    /// Whether only the last poll site, not the `GameState`, says this is the question; see
    /// `LlmPolicy::observed_kind`.
    pub fn is_inferred_from_the_site(self) -> bool {
        matches!(self, Self::Nickname | Self::MartPurchase | Self::ForgetMove | Self::Stuck)
    }
}

/// A terminal tool call, parsed; resolved against the live game at the poll, since the world may
/// have moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminal {
    /// One overworld action, and optionally the next few after it.
    ChooseAction {
        id: String,
        /// Further ids from the same turn's menu, in the order they are to be taken.
        then: Vec<String>,
        /// Opt-in: once an interrupting battle is over the action is taken again, not handed back.
        resume_after_battle: bool,
    },
    /// One battle action, and whether the rest of this battle belongs to the model.
    ChooseBattleAction { id: String, take_over: bool },
    /// Something the agent does without walking: teach an HM, use an item, ride a lift.
    UseFieldMove(FieldMoveRequest),
    PressButtons { buttons: Vec<JoypadButton> },
    SetNickname { name: Option<String> },
    BuyItem {
        item: Option<BagItem>,
        /// More kinds to buy in the same visit, in order.
        then: Vec<BagItem>,
    },
    ForgetMove { slot: Option<u8> },
    /// Do nothing for this many agent ticks (20 ms of emulated time each).
    Wait { ticks: u16 },
}

/// How many overworld actions one `choose_action` may carry, `id` included.
pub const MAX_CHAINED_ACTIONS: usize = 4;

/// A longer `wait` is a model stalling its own run.
pub const MAX_WAIT_TICKS: u16 = 150;

/// The longest nickname the naming screen's buffer holds.
pub const MAX_NICKNAME: usize = 10;

/// What one call in an assistant message turned out to be.
pub enum CallKind {
    /// A read tool.
    Read,
    /// `screenshot`.
    Screenshot,
    /// A TODO operation.
    Todo(TodoCall),
    /// `report_issue`: the model believes the agent is wrong.
    Issue(String),
    /// A battle-script operation.
    BattleScript(BattleScriptCall),
    /// The turn is over.
    Terminal(Terminal),
    /// An unknown name, a terminal tool of another decision kind, or unparseable arguments.
    Rejected(String),
}

impl CallKind {
    /// The discriminant, for the page: these words are a wire contract with `api.ts`.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Read | Self::Screenshot => "read",
            Self::Todo(_) => "todo",
            Self::Issue(_) => "issue",
            // Not a fifth word: `api.ts` matches on these.
            Self::BattleScript(_) => "todo",
            Self::Terminal(_) => "terminal",
            Self::Rejected(_) => "rejected",
        }
    }
}

// ── Field moves ──────────────────────────────────────────────────────────────────────────────────

/// A `use_field_move` call, parsed; [`resolve_field_move`] resolves it against the live state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldMoveRequest {
    PartyMove { name: PokemonMoveName, slot: Option<u8> },
    Fly { to: Map },
    /// Teach an HM or TM in the bag to a party member.
    Teach { item: ItemId, slot: u8 },
    /// Use an evolution stone from the bag on a party member.
    Evolve { stone: ItemId, slot: u8 },
    /// Use a bag item on `target` if given (the Poké Flute on Snorlax), else the player or `slot`.
    UseItem { item: ItemId, target: Option<Point8>, slot: Option<u8> },
    /// Throw an item away to free one of the bag's 20 slots.
    TossItem { item: ItemId },
    /// Rearrange the party so `slot` leads.
    ReorderParty { slot: u8 },
    /// Deposit, withdraw or change box at the PC.
    UsePcBox { op: crate::pokemon::postgame::pc_box::PcBoxOp },
    /// Move items between the bag and PC item storage.
    UseItemPc { op: crate::pokemon::postgame::item_storage::PcItemOp, item: ItemId, qty: u8 },
    /// Ride the lift the player is standing in to `to`.
    UseElevator { to: Map },
    /// Board the Pokémon in `deposit` at the Day Care, or collect the one boarded.
    DayCare { deposit: Option<u8> },
    /// Buy `prize` with coins in the Game Corner's prize room.
    RedeemPrize { prize: crate::pokemon::postgame::game_corner::Prize },
}

/// The badge each HM needs outside battle, from `.outOfBattleMovePointers` in
/// `engine/menus/start_sub_menus.asm`.
const HM_BADGES: &[(PokemonMoveName, crate::pokemon::badge::Badge)] = &[
    (PokemonMoveName::Flash, crate::pokemon::badge::Badge::BoulderBadge),
    (PokemonMoveName::Cut, crate::pokemon::badge::Badge::CascadeBadge),
    (PokemonMoveName::Fly, crate::pokemon::badge::Badge::ThunderBadge),
    (PokemonMoveName::Strength, crate::pokemon::badge::Badge::RainbowBadge),
    (PokemonMoveName::Surf, crate::pokemon::badge::Badge::SoulBadge),
];

/// Whether `name` is one of the five HM moves, and which HM teaches it.
pub fn hm_move(name: PokemonMoveName) -> Option<PokemonMoveName> {
    HM_BADGES.iter().find(|(hm, _)| *hm == name).map(|(hm, _)| *hm)
}

/// Refuse a field move the game itself would refuse, and say which half is missing.
fn hm_available(state: &GameState, name: PokemonMoveName) -> Result<(), String> {
    let Some(&(_, badge)) = HM_BADGES.iter().find(|(hm, _)| *hm == name) else { return Ok(()) };
    let known = (0..state.pokemon.len() as u8).any(|slot| knows(state, slot, name));
    match (known, state.badges.contains(badge)) {
        (true, true) => Ok(()),
        (false, true) => Err(format!(
            "No Pokémon in the party knows {name}, so it cannot be used. {name} is taught by an HM \
             you have to find first."
        )),
        (true, false) => Err(format!(
            "{name} needs the {badge} before the game will let it be used outside battle, and you \
             do not have it yet. Win the gym badge first."
        )),
        (false, false) => Err(format!(
            "{name} cannot be used yet: no Pokémon in the party knows it, and it also needs the \
             {badge}, which you do not have. Both have to come first."
        )),
    }
}

/// The PC on this map, or why there is none.
fn the_pc_here(state: &GameState) -> Result<Point8, String> {
    crate::pokemon::tile_map::pc_locations_for(state.map.map).first().copied().ok_or_else(|| {
        format!(
            "There is no PC on {}. Every Pokémon Centre has one, and so does the player's bedroom.",
            state.map.map,
        )
    })
}

/// Turn a request into the [`FieldMove`] the agent executes, or the sentence the model is told.
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
        FieldMoveRequest::PartyMove { name, slot } => {
            hm_available(state, *name)?;
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
            // The party menu lists field moves in move-slot order, so the index depends on the set.
            FieldMove::UseFieldMove { slot, move_index: field_move_index(state, slot, *name) }
        }
        FieldMoveRequest::Fly { to } => {
            hm_available(state, PokemonMoveName::Fly)?;
            FieldMove::Fly { to: *to }
        }
        FieldMoveRequest::Teach { item, slot } => {
            let item = held(*item)?;
            let slot = party_slot(*slot)?;
            // A machine the game refuses wedges the agent, so it is refused here first.
            if state.pokemon.get(slot as usize)
                .is_some_and(|mon| !crate::pokemon::learnset::can_learn(mon.species, item)) {
                return Err(crate::pokemon::learnset::teach_refusal(state, item, slot));
            }
            FieldMove::TeachMove { item, target_slot: slot }
        }
        FieldMoveRequest::Evolve { stone, slot } => {
            let slot = party_slot(*slot)?;
            // Completion is "this slot's species changed", so the driver needs the starting one.
            let evolve_from = state
                .pokemon
                .get(slot as usize)
                .map(|mon| mon.species)
                .ok_or_else(|| format!("Slot {slot} is empty."))?;
            FieldMove::EvolveWithStone { stone: held(*stone)?, target_slot: slot, evolve_from }
        }
        FieldMoveRequest::UseItem { item, target: None, slot } => {
            let item = held(*item)?;
            if let Some(refusal) = crate::pokemon::item_use::field_use_refusal(item) {
                return Err(refusal);
            }
            let target = match slot {
                Some(slot) => crate::pokemon::postgame::items::UseTarget::Party {
                    slot: party_slot(*slot)?,
                },
                None => crate::pokemon::postgame::items::UseTarget::Nothing,
            };
            // Asked up front: `ItemUseNotTime` consumes nothing and prints a box like success.
            if let Some(refusal) = crate::pokemon::postgame::items::blocked(state, item, target) {
                return Err(format!("The game will not do that: {refusal}."));
            }
            FieldMove::UseBagItem { item, target }
        }
        // A `slot` beside a `target` is ignored: an item that takes a target takes no party member.
        FieldMoveRequest::UseItem { item, target: Some(target), slot: _ } => {
            // As `Teach`'s gate, for an item the game will not use at all.
            let item = held(*item)?;
            if let Some(refusal) = crate::pokemon::item_use::field_use_refusal(item) {
                return Err(refusal);
            }
            let at = *target;
            let nothing_there = |noun: &str| -> String {
                // Whatever is beside the square it aimed at, saving a `read_map`.
                let beside = [
                    at.y.checked_sub(1).map(|y| Point8 { x: at.x, y }),
                    at.y.checked_add(1).map(|y| Point8 { x: at.x, y }),
                    at.x.checked_sub(1).map(|x| Point8 { x, y: at.y }),
                    at.x.checked_add(1).map(|x| Point8 { x, y: at.y }),
                ].into_iter().flatten().find_map(|next| match state.map.tile_at_checked(next) {
                    Some(crate::pokemon::tile::MetaTile::Sprite(name)) => Some((name, next)),
                    _ => None,
                });
                format!(
                    "There is nothing at ({}, {}) to use the {item} on: that square is {noun}. \
                        `target` is the square the *thing* is standing on, which is not the square an \
                        action id names: an id's coordinate is where the player stands to reach it.{}",
                    at.x, at.y,
                    match beside {
                        Some((name, next)) => format!(
                            " What is next to it is {name}, at ({}, {}). `read_map` gives the exact \
                                position of everything on this map.", next.x, next.y),
                        None => " `read_map` gives the exact position of everything on this map."
                            .to_string(),
                    },
                )
            };
            use crate::pokemon::tile::MetaTile;
            match state.map.tile_at_checked(at) {
                None => return Err(format!(
                    "({}, {}) is not a square on {}, which is {} wide and {} high.",
                    at.x, at.y, state.map.map, state.map.width, state.map.height)),
                Some(MetaTile::Empty) => return Err(nothing_there("open ground")),
                Some(MetaTile::Grass) => return Err(nothing_there("tall grass")),
                Some(MetaTile::Water) => return Err(nothing_there("water")),
                Some(_) => {}
            }
            // Every one of these items reads the tile in front of the player.
            if state.map.route_to_face(at).is_none() {
                return Err(format!(
                    "The player cannot get next to ({}, {}) to face it, and every one of these items \
                        acts on the square in front of the player. Whatever is there has to be reached \
                        first: the action menu is the list of what can be.",
                    at.x, at.y,
                ));
            }
            FieldMove::UseFieldItem { item, target: at }
        }
        FieldMoveRequest::TossItem { item } => FieldMove::TossItem { item: held(*item)? },
        FieldMoveRequest::ReorderParty { slot } => FieldMove::ReorderParty { slot: party_slot(*slot)? },
        FieldMoveRequest::UsePcBox { op } => {
            let pc = the_pc_here(state)?;
            // `blocked_by` is the game's own refusals, asked before a button is pressed.
            if let Some(refusal) = op.blocked_by(
                state.pokemon.len() as u8,
                state.boxed_pokemon.len() as u8,
                state.current_box,
            ) {
                return Err(format!("The game will not do that: {refusal}."));
            }
            FieldMove::UsePcBox { op: *op, pc }
        }
        FieldMoveRequest::UseItemPc { op, item, qty } => {
            use crate::pokemon::postgame::item_storage::PcItemOp;
            let pc = the_pc_here(state)?;
            if *qty == 0 {
                return Err("A quantity of 0 moves nothing.".to_string());
            }
            // A withdrawal reads PC storage, not the bag, so only a deposit asks `held`.
            if matches!(op, PcItemOp::Deposit) { held(*item)?; }
            FieldMove::UseItemPc { op: *op, item: *item, qty: *qty, pc }
        }
        FieldMoveRequest::DayCare { deposit } => {
            use crate::pokemon::postgame::gifts::{pick, PartyScript};
            if state.map.map != Map::Daycare {
                return Err("The Day Care is the house on Route 5, and this is done inside it, facing \
                            the gentleman.".to_string());
            }
            match deposit {
                Some(_) if state.day_care_in_use => return Err(
                    "The Day Care is already boarding a Pokémon, and it takes one at a time: \
                     `withdraw` it first.".to_string()),
                Some(_) if state.pokemon.len() < 2 => return Err(
                    "The gentleman will not take the only Pokémon you have with you.".to_string()),
                None if !state.day_care_in_use => return Err(
                    "Nothing is boarded at the Day Care, so there is nothing to collect.".to_string()),
                None if state.pokemon.len() >= 6 => return Err(
                    "The party is full, so a boarded Pokémon has nowhere to come back to.".to_string()),
                _ => {}
            }
            let slot = match deposit { Some(slot) => party_slot(*slot)?, None => 0 };
            if deposit.is_some()
                && let Some(hm) = state.pokemon.get(slot as usize).and_then(|mon| mon.moves.iter().flatten()
                    .find(|m| HM_BADGES.iter().any(|(name, _)| *name == m.name)))
            {
                return Err(format!(
                    "The gentleman says \"I can't accept a POKéMON that knows an HM move.\" The \
                     Pokémon in slot {slot} knows {}.", hm.name));
            }
            pick(state, PartyScript::Daycare, slot)
                .ok_or_else(|| "The gentleman cannot be reached from here.".to_string())?
        }
        FieldMoveRequest::RedeemPrize { prize } => {
            if state.map.map != Map::GameCornerPrizeRoom {
                return Err("Prizes are bought in the Game Corner's prize room, beside it in \
                            Celadon City, and this is done inside it.".to_string());
            }
            held(ItemId::CoinCase).map_err(|_| "Coins need the COIN CASE, which the Celadon \
                                               Diner's gym guide gives away.".to_string())?;
            if state.coins < prize.cost() {
                return Err(format!(
                    "{prize:?} costs {} coins and you have {}. The clerk in the Game Corner sells \
                     50 coins for ¥1000.", prize.cost(), state.coins));
            }
            // A machine is an item, and the clerk will not hand over what the bag has no room for.
            if let Some(tm) = prize.item()
                && state.bag.len() >= crate::pokemon::bag::Bag::MAX_ITEMS
                && !state.bag.contains(&tm)
            {
                return Err("The bag is full: it holds 20 kinds, and the clerk says \"Oops! You don't \
                            have enough room.\" Toss something first.".to_string());
            }
            FieldMove::RedeemPrize { prize: *prize }
        }
        FieldMoveRequest::UseElevator { to } => {
            let Some((panel, floors)) = crate::pokemon::tile_map::elevator_for(state.map.map) else {
                return Err(format!(
                    "There is no lift on {}. A lift is a room of its own, reached by a warp: the \
                     Rocket Hideout's, Celadon Mart's and Silph Co's are the three in the game.",
                    state.map.map,
                ));
            };
            let Some(floor) = floors.iter().position(|floor| floor == to) else {
                return Err(format!(
                    "This lift does not stop at {to}. It serves {}.",
                    floors.iter().map(|floor| floor.to_string()).collect::<Vec<_>>().join(", "),
                ));
            };
            // Only the Rocket Hideout's lift needs a key, per `RocketHideoutElevatorText`.
            if state.map.map == Map::RocketHideoutElevator {
                held(ItemId::LiftKey).map_err(|_| {
                    "This lift needs the LIFT_KEY, which is somewhere in the hideout. Without it \
                     the panel does nothing.".to_string()
                })?;
            }
            FieldMove::UseElevator { panel, floor: floor as u8 }
        }
    })
}

/// The first character of `name` the cartridge's charmap has no byte for, if there is one.
fn unencodable(name: &str) -> Option<char> {
    name.chars().find(|c| {
        crate::pokemon::strings::PokemonString::from_string(&c.to_string()).0.first() == Some(&0x00)
    })
}

fn knows(state: &GameState, slot: u8, name: PokemonMoveName) -> bool {
    state
        .pokemon
        .get(slot as usize)
        .is_some_and(|mon| mon.moves.iter().flatten().any(|m| m.name == name))
}

/// The moves `use_field_move` accepts under [`FieldMoveRequest::PartyMove`], and what each is for.
const PARTY_MOVES: &[(&str, PokemonMoveName, &str)] = &[
    ("flash", PokemonMoveName::Flash, "light a dark map (Rock Tunnel)"),
    ("dig", PokemonMoveName::Dig, "warp straight out of a cave or dungeon"),
    ("teleport", PokemonMoveName::Teleport, "warp back to the last Pokémon Center"),
    ("softboiled", PokemonMoveName::Softboiled, "heal another party member from Chansey's HP"),
];

// ── The catalogue ────────────────────────────────────────────────────────────────────────────────

pub struct ReadTool {
    pub name: &'static str,
    pub description: &'static str,
    /// Which turns this read is offered in.
    pub kinds: &'static [DecisionKind],
    /// `None` for the reads that take no arguments, which is all of them but [`READ_ROUTE`].
    pub parameters: Option<fn() -> Value>,
}

/// Non-terminal, callable any number of times within a turn.
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
        name: "read_pc",
        description: "What is in the PC: the Pokémon in the open box with the slot numbers \
                      `use_field_move` wants, what is in PC item storage, and which of the twelve \
                      boxes is open. Only the open box can be read.",
        kinds: &[DecisionKind::Overworld],
        parameters: None,
    },
    ReadTool {
        name: "read_bag",
        description: "Every item in the bag with its quantity and shop price, plus money and how \
                      many of the bag's 20 slots are used.",
        // The bag is nowhere in a turn request, and `use_field_move` needs its exact names.
        kinds: &[DecisionKind::Overworld, DecisionKind::Battle, DecisionKind::MartPurchase],
        parameters: None,
    },
    ReadTool {
        name: "read_battle",
        description: "The live battle: both sides' species, level, HP, status, types and moves, the \
                      enemy's catch rate, and which of your moves Disable has locked out. The \
                      turn's battle menu already costs your moves against it; this is the detail.",
        // `ForgetMove` legitimately fires mid-fight, and which move to drop is a battle question.
        kinds: &[DecisionKind::Battle, DecisionKind::ForgetMove],
        parameters: None,
    },
    ReadTool {
        name: READ_ROUTE,
        description: "How to get somewhere you have already been. With `to`, the sequence of maps \
                      from here to that one, each saying which warp or edge of the map before it \
                      to leave by; without it, every map you have set foot on. It knows \
                      only what has been walked, so a map missing from it means 'not visited yet', \
                      never 'does not exist'.",
        kinds: &[DecisionKind::Overworld],
        parameters: Some(read_route_arguments),
    },
    ReadTool {
        name: READ_GUIDE,
        description: "The walkthrough for the stretch of the game you are in now: where to go, in \
                      order, what is blocking the way and what the next Gym Leader has. It is chosen \
                      from your badges and it does not change until you win the next one, so there \
                      is no reason to ask twice. Place names in it are spelled exactly as the \
                      action menu and `read_route` spell them.",
        // It answers "where am I going", which no battle, name, mart or forgotten move raises.
        kinds: &[DecisionKind::Overworld, DecisionKind::Stuck],
        parameters: None,
    },
    ReadTool {
        name: SCREENSHOT,
        description: "A picture of the Game Boy screen as it is right now. Everything the agent can \
                      read for you — the map, the party, the text on screen — is cheaper and more \
                      precise as one of the other reads; ask for this when you want to see \
                      something they do not model, such as an unfamiliar menu or an animation you \
                      are not sure has finished.",
        // Every kind: only the screen explains an unfamiliar menu or a wedged agent.
        kinds: &ALL_KINDS,
        parameters: None,
    },
];

/// Answered by the worker, since PNG encoding does not belong on the emulator thread.
pub const SCREENSHOT: &str = "screenshot";

/// The world graph, asked the question a model has, and never serialised whole.
pub const READ_ROUTE: &str = "read_route";

/// The [`crate::llm::guide`] chapter the badges pick, so the tool takes no arguments.
pub const READ_GUIDE: &str = "read_guide";

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

/// Every [`DecisionKind`], so a loop over all of them keeps meaning it when one is added.
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

/// Named in the turn contract as non-terminal: a model that thinks `todo_set` ended its turn stops.
pub const TODO_TOOL_NAMES: &[&str] = &["todo_set", "todo_complete", "todo_delete"];

/// Their specs; a JSON Schema is not a `const` expression.
pub fn todo_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "todo_set",
            format!(
                "Add or rewrite one item on your plan: no `id` adds one on the end, an `id` \
                 rewrites that one where it is. The order is kept. Room for {MAX_TODO_ITEMS}, \
                 finished ones included. This is the only thing you write that outlives the \
                 conversation, so give the reason with the intent: `come back to Route 12 with the \
                 Poké Flute, the Snorlax blocks the path south`. At most {MAX_TODO_TEXT} characters."
            ),
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "minimum": 1,
                            "description": "An item's number, as shown in your plan. Omit to add a new one." },
                    // Not `required`, though the description reads as though it were.
                    "text": { "type": "string", "maxLength": MAX_TODO_TEXT,
                              "description": "What to do, and why." },
                },
                "additionalProperties": false,
            }),
        ),
        ToolSpec::new(
            "todo_complete",
            "Mark one item on your plan done, by the number shown beside it.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "integer", "minimum": 1,
                                        "description": "The item's number, as shown in your plan." } },
                "required": ["id"],
                "additionalProperties": false,
            }),
        ),
        ToolSpec::new(
            "todo_delete",
            "Drop one item off your plan for good, by its number — something you no longer mean to \
             do. `todo_complete` is for one you have done.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "integer", "minimum": 1,
                                        "description": "The item's number, as shown in your plan." } },
                "required": ["id"],
                "additionalProperties": false,
            }),
        ),
    ]
}

// ── The battle script ────────────────────────────────────────────────────────────────────────────

/// One tool call against the model's battle script, parsed and answered on the worker thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BattleScriptCall {
    /// `get_battle_script_docs`: the API reference, verbatim.
    Docs,
    /// `read_battle_script`: what is installed, and whether it is armed.
    Read,
    /// `set_battle_script`: a `script` of `None` goes back to the default.
    Set { script: Option<String>, purpose: Option<String> },
}

/// The three, by name. Non-terminal, so they are named in the turn contract beside the reads.
pub const BATTLE_SCRIPT_TOOL_NAMES: &[&str] =
    &["get_battle_script_docs", "read_battle_script", "set_battle_script"];

/// Offered on `Overworld` and nowhere else.
fn offers_battle_script(kind: DecisionKind) -> bool {
    kind == DecisionKind::Overworld
}

/// Their specs, a function for the reason [`todo_tools`] is one.
pub fn battle_script_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "get_battle_script_docs",
            "How to write a battle script: the language, everything a script can read about the battle, and a worked example. Read this before `set_battle_script`.",
            no_arguments(),
        ),
        ToolSpec::new(
            "read_battle_script",
            "The battle script you have installed, and whether it is still deciding your battle turns. Every run starts on a default one that decides nothing, so this always has something to show you.",
            no_arguments(),
        ),
        ToolSpec::new(
            "set_battle_script",
            format!(
                "Install a script that decides your battle turns for you, in Rhai. A turn it answers costs no request at all, so a routine wild encounter becomes free. It is run against seven example battles before it is installed and you are told what it chose in each; if it later fails it is disarmed and that turn comes back to you with the reason. Omit `script` to go back to the default, which hands you every turn. At most {MAX_BATTLE_SCRIPT} characters. Call `get_battle_script_docs` first."
            ),
            json!({
                "type": "object",
                "properties": {
                    "script": { "type": "string",
                                "description": "The script. Omit to go back to the default one." },
                    "purpose": { "type": "string",
                                 "description": "What it is for. Required." },
                },
                "additionalProperties": false,
            }),
        ),
    ]
}

fn classify_battle_script(name: &str, arguments: &Value) -> Option<CallKind> {
    let call = match name {
        "get_battle_script_docs" => BattleScriptCall::Docs,
        "read_battle_script" => BattleScriptCall::Read,
        // `null` and an absent `script` mean the same thing and both have to work.
        "set_battle_script" => BattleScriptCall::Set {
            script: arguments.get("script").and_then(Value::as_str).map(str::to_string),
            purpose: arguments.get("purpose").and_then(Value::as_str)
                .map(str::trim).filter(|purpose| !purpose.is_empty()).map(str::to_string),
        },
        _ => return None,
    };
    Some(CallKind::BattleScript(call))
}

fn classify_todo(name: &str, arguments: &Value) -> Option<CallKind> {
    let call = match name {
        "todo_set" | "todo_add" => TodoCall::Set {
            id: arguments.get("id").and_then(Value::as_u64).map(|id| id.min(u64::from(u32::MAX)) as u32),
            text: arguments.get("text").and_then(Value::as_str).map(str::to_string),
        },
        "todo_complete" => match arguments.get("id").and_then(Value::as_u64) {
            Some(id) => TodoCall::Complete { id: id.min(u64::from(u32::MAX)) as u32 },
            None => return Some(CallKind::Rejected("`todo_complete` needs the item's `id`.".to_string())),
        },
        // The named delete.
        "todo_delete" => match arguments.get("id").and_then(Value::as_u64) {
            Some(id) => TodoCall::Set { id: Some(id.min(u64::from(u32::MAX)) as u32), text: None },
            None => return Some(CallKind::Rejected("`todo_delete` needs the item's `id`.".to_string())),
        },
        _ => return None,
    };
    Some(CallKind::Todo(call))
}

pub fn for_kind(kind: DecisionKind) -> Vec<ToolSpec> {
    let mut tools: Vec<ToolSpec> = reads_for(kind)
        .map(|tool| {
            ToolSpec::new(tool.name, tool.description, tool.parameters.map_or_else(no_arguments, |f| f()))
        })
        .collect();
    tools.extend(todo_tools());
    if offers_battle_script(kind) {
        tools.extend(battle_script_tools());
    }

    match kind {
        DecisionKind::Overworld => {
            tools.push(ToolSpec::new(
                "choose_action",
                format!(
                    "ENDS THE TURN. Walk to and take one of the actions listed in the turn's action \
                     menu. `id` is the id from that menu, copied exactly — never a position in the \
                     list. `then` chains up to {} more ids from this same menu, taken in order \
                     without asking you again; that is worth doing where each is still true after \
                     the one before, as in heal then leave. It stops at the first that will not \
                     resolve or is stopped, and says where it got to. `resume_after_battle` is \
                     true unless you say otherwise: a battle interrupting the action does not end \
                     it, and it is taken up again after, up to {} times.",
                    MAX_CHAINED_ACTIONS - 1,
                    crate::pokemon::llm_policy::MAX_BATTLE_RESUMES,
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "An id from the action menu." },
                        "then": {
                            "type": "array",
                            "items": { "type": "string" },
                            "maxItems": MAX_CHAINED_ACTIONS - 1,
                            "description": "More ids from this menu, in order.",
                        },
                        "resume_after_battle": {
                            "type": "boolean",
                            "description": "Default true. False to be asked again instead.",
                        },
                    },
                    "required": ["id"],
                    "additionalProperties": false,
                }),
            ));
            tools.push(use_field_move_spec());
        }
        DecisionKind::Battle => {
            tools.push(ToolSpec::new(
                "choose_battle_action",
                "ENDS THE TURN. Take one of the actions listed in the turn's battle menu. `id` is the \
                 id from that menu, copied exactly. `take_over` takes the rest of *this* battle away \
                 from your battle script, so a switch or a plan of your own is not replaced on the \
                 next turn; the script decides battles again as soon as this one ends.",
                json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "An id from the battle menu." },
                        "take_over": { "type": "boolean",
                                       "description": "Decide the rest of this battle yourself." },
                    },
                    "required": ["id"],
                    "additionalProperties": false,
                }),
            ));
        }
        DecisionKind::Nickname => tools.push(ToolSpec::new(
            "set_nickname",
            format!(
                "ENDS THE TURN. **Give this Pokémon a nickname**: one that says what you make of \
                 it — how you came by it, what you mean to use it for, what it reminds you of. It \
                 is what every message about it will call it from now on. Omit `name` only if \
                 nothing comes to mind. At most {MAX_NICKNAME} characters; letters, digits, spaces \
                 and `.,:;'-?!()[]/` only."
            ),
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "maxLength": MAX_NICKNAME,
                        "description": "The nickname you have chosen. Omit to keep the species name.",
                    }
                },
                "additionalProperties": false,
            }),
        )),
        DecisionKind::MartPurchase => tools.push(ToolSpec::new(
            "buy_item",
            "ENDS THE TURN. Buy from the mart, then leave. `item` is a name from the stock list, \
             copied exactly, and `quantity` how many. `then` buys more kinds in the same visit \
             without asking you again — Poké Balls and Potions in one stop rather than two. Omit \
             `item` to walk away without buying anything. Each order is trimmed to what the money \
             covers, in order, because Gen 1 sells you nothing at all for an order you cannot \
             afford; the row for each item says how many you already have.",
            json!({
                "type": "object",
                "properties": {
                    "item": { "type": "string", "description": "A name from the stock list." },
                    "quantity": {
                        "type": "integer", "minimum": 1, "maximum": 99, "default": 1,
                        "description": "How many of `item` to buy.",
                    },
                    "then": {
                        "type": "array",
                        "description": "More kinds to buy in this same visit, in order.",
                        "maxItems": MAX_CHAINED_PURCHASES,
                        "items": {
                            "type": "object",
                            "properties": {
                                "item": { "type": "string" },
                                "quantity": { "type": "integer", "minimum": 1, "maximum": 99, "default": 1 },
                            },
                            "required": ["item"],
                            "additionalProperties": false,
                        },
                    },
                },
                "additionalProperties": false,
            }),
        )),
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

    if offers_issue_report(kind) {
        tools.push(report_issue_spec());
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

/// Long enough for the intent and the reason, short enough to carry one per turn for a whole run.
pub const MAX_SUMMARY: usize = 300;

/// `press_buttons`' `why` answers one narrow question, so it is shorter than a summary.
pub const MAX_REASON: usize = 200;

/// Bolt a required `summary` onto a terminal tool's schema.
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
pub fn call_summary(call: &ToolCall) -> Option<String> {
    call_string(call, "summary", MAX_SUMMARY)
}

/// `press_buttons`' `why`: what the model thinks is on the screen and what it is pressing at.
pub fn call_reason(call: &ToolCall) -> Option<String> {
    call_string(call, "why", MAX_REASON)
}

/// One free-text argument, trimmed and capped: a schema's `maxLength` is only a request.
fn call_string(call: &ToolCall, field: &str, cap: usize) -> Option<String> {
    let value = call.arguments().ok()?.get(field)?.as_str()?.trim().to_string();
    if value.is_empty() {
        return None;
    }
    Some(match value.char_indices().nth(cap) {
        Some((cut, _)) => value[..cut].to_string(),
        None => value,
    })
}

/// A zero-parameter tool still needs a schema, and every endpoint accepts an empty object.
fn no_arguments() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

/// One tool for every non-walking field action, discriminated by `move`, to keep `tools` short.
fn use_field_move_spec() -> ToolSpec {
    let party_moves: Vec<String> =
        PARTY_MOVES.iter().map(|(name, _, why)| format!("`{name}` — {why}")).collect();
    ToolSpec::new(
        "use_field_move",
        format!(
            "ENDS THE TURN. Do something that is not walking. `move` picks which, and decides which \
             of the other arguments are needed:\n\
             - {}. Each takes an optional `slot`; without one the first Pokémon that knows the move \
             is used.\n\
             - `fly` — fly to `map`, which must be a town you have already visited with a Pokémon \
             Center.\n\
             - `teach` — teach the HM or TM `item` to the Pokémon in `slot`.\n\
             - `evolve` — use the evolution stone `item` on the Pokémon in `slot`.\n\
             - `use_item` — use bag `item`: on `target`, facing it (the Poké Flute on Snorlax, the \
             Card Key on a door); on the party member in `slot` (a Potion, a vitamin); or on \
             neither, for the Bicycle, a Repel or the Itemfinder.\n\
             - `toss_item` — throw `item` away to free a bag slot. The bag holds only 20 kinds.\n\
             - `reorder_party` — make the Pokémon in `slot` the party leader.\n\
             - `pc_pokemon` — at a PC: `op` is `deposit` (party `slot` → box), `withdraw` or \
             `release` (a `box_slot`), or `change_box` (`box`, 1-12, which also saves the game). \
             Only the open box can be read; `read_pc` shows it.\n\
             - `pc_items` — at a PC: `op` `deposit` or `withdraw` moves `quantity` of `item` \
             between the bag and PC storage. The bag holds only 20 kinds.\n\
             - `elevator` — inside a lift, ride it to `map`. The three lifts are in the Rocket \
             Hideout, Celadon Mart and Silph Co.\n\
             - `day_care` — in the Day Care, `op` `deposit` boards `slot`; `withdraw` pays and \
             collects.\n\
             - `prize` — in the prize room, buy `item` with coins.\n\
             Cutting a tree, pushing a boulder and mounting Surf are not here: each of the three is \
             a walk with one legal ending, so the walk does it. A `:CutTree` row cuts the tree it \
             walks up to, a `:PushBoulder…` row names a boulder and a target and does the whole \
             puzzle, arming Strength and shoving until that boulder is on that switch or down that \
             hole, and any route that crosses water mounts Surf on its own. All three rows are withheld \
             while the game would refuse them, and the turn says so in a line above the menu.\n\
             `fly` and `flash` each need a Pokémon taught that HM *and* a particular badge; until \
             you have both the game refuses them, and retrying will not change it.",
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
                "map": { "type": "string", "description": "A map name, for `fly` and `elevator`." },
                "target": {
                    "type": "object",
                    "properties": { "x": { "type": "integer" }, "y": { "type": "integer" } },
                    "required": ["x", "y"],
                    "additionalProperties": false,
                    "description": "A tile on the current map, in the coordinates `read_map` uses. \
                                    For `use_item`, the square the *thing* is on; omit it for an \
                                    item used on nobody.",
                },
                "op": {
                    "type": "string",
                    "enum": ["deposit", "withdraw", "release", "change_box"],
                    "description": "For `pc_pokemon` and `pc_items`: which way things move.",
                },
                "box_slot": { "type": "integer", "minimum": 0, "maximum": 19, "description": "A slot in the open box, 0-based." },
                "box": { "type": "integer", "minimum": 1, "maximum": 12, "description": "Which box to open, for `change_box`." },
                "quantity": { "type": "integer", "minimum": 1, "maximum": 99, "description": "How many, for `pc_items`. Default 1." },
            },
            "required": ["move"],
            "additionalProperties": false,
        }),
    )
}

fn field_move_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = PARTY_MOVES.iter().map(|(name, _, _)| *name).collect();
    names.extend([
        "fly", "teach", "evolve", "use_item", "toss_item", "reorder_party",
        "pc_pokemon", "pc_items", "elevator", "day_care", "prize",
    ]);
    names
}

pub const REPORT_ISSUE: &str = "report_issue";

/// Longer than a summary: an issue is written to disk once and read by a person.
pub const MAX_ISSUE: usize = 1_000;

/// `report_issue`: the model believes the agent is wrong. It does not end the turn, and its
/// answer must not read like a fix.
fn report_issue_spec() -> ToolSpec {
    ToolSpec::new(
        REPORT_ISSUE,
        format!(
            "Report a problem with the agent itself: something it will not let you do, an action \
             menu that does not describe what is on the screen, a choice that keeps failing for a \
             reason you cannot see. **A developer reads these.** Write it as a bug report: what you \
             were trying to do, what you expected, and what happened instead. The game's state, the \
             screen and a save state are filed alongside it automatically, so describe rather than \
             transcribe. This does NOT end your turn and does NOT fix anything now: after filing \
             it, carry on and try a different way. At most {MAX_ISSUE} characters."
        ),
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "maxLength": MAX_ISSUE,
                    "description": "The report: what you tried, what you expected, what happened.",
                },
            },
            "required": ["message"],
            "additionalProperties": false,
        }),
    )
}

/// A `report_issue` call's message, trimmed and capped; [`classify`] rejects a call without one.
pub fn issue_message(call: &ToolCall) -> Option<String> {
    call_string(call, "message", MAX_ISSUE)
}

/// The last resort, on the watchdog's turn only: raw presses pre-empt the state machine.
fn press_buttons_spec() -> ToolSpec {
    ToolSpec::new(
        "press_buttons",
        format!(
            "ENDS THE TURN. Press these buttons in order, one at a time, then hand control back \
             to the agent. You are being offered this because the agent has stopped reaching \
             decision points on its own: there is no menu to choose from and a raw press is the way \
             out. Work out from the screen what is in front of you. B backs out of most menus and \
             closes most boxes; A advances text. Up to {MANUAL_INPUT_CAPACITY} presses; anything \
             past that is dropped."
        ),
        json!({
            "type": "object",
            "properties": {
                "buttons": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MANUAL_INPUT_CAPACITY,
                    "items": { "type": "string", "enum": ["up", "down", "left", "right", "a", "b", "start", "select"] },
                },
                "why": {
                    "type": "string",
                    "maxLength": MAX_REASON,
                    "description": "What you think is on the screen, and what these presses are \
                                    meant to do about it. Required: every press is recorded and \
                                    read afterwards by a person.",
                },
            },
            "required": ["buttons", "why"],
            "additionalProperties": false,
        }),
    )
}

pub fn non_terminal_names(kind: DecisionKind) -> Vec<&'static str> {
    reads_for(kind)
        .map(|tool| tool.name)
        .chain(TODO_TOOL_NAMES.iter().copied())
        .chain(offers_battle_script(kind).then(|| BATTLE_SCRIPT_TOOL_NAMES.iter().copied()).into_iter().flatten())
        .chain(offers_issue_report(kind).then_some(REPORT_ISSUE))
        .collect()
}

/// Which kinds can report that the agent itself is wrong.
pub fn offers_issue_report(kind: DecisionKind) -> bool {
    matches!(kind, DecisionKind::Overworld | DecisionKind::Battle | DecisionKind::Stuck)
}

pub fn terminal_names(kind: DecisionKind) -> &'static [&'static str] {
    match kind {
        DecisionKind::Overworld => &["choose_action", "use_field_move", "wait"],
        DecisionKind::Battle => &["choose_battle_action", "wait"],
        DecisionKind::Nickname => &["set_nickname", "wait"],
        DecisionKind::MartPurchase => &["buy_item", "wait"],
        DecisionKind::ForgetMove => &["forget_move", "wait"],
        DecisionKind::Stuck => &["press_buttons", "wait"],
    }
}

// ── Classification ───────────────────────────────────────────────────────────────────────────────

/// The complaint when the model chose an id this turn never offered, or `None` when it did.
fn not_on_the_menu(id: &str, menu: &[String]) -> Option<String> {
    if menu.is_empty() || menu.iter().any(|offered| offered == id) {
        return None;
    }
    // Tested against `Map::iter()`: a battle id such as `item:Potion` also has a colon.
    let a_map = |name: &str| {
        use strum::IntoEnumIterator;
        Map::iter().any(|map| map.to_string() == name)
    };
    let here = menu[0].split(':').next().filter(|name| a_map(name));
    let elsewhere = match (here, id.split_once(':')) {
        (Some(here), Some((named, _))) if named != here && a_map(named) => format!(
            " That id is for `{named}` and you are in `{here}`; ids are minted for the map you are \
             standing on, so one you read on an earlier turn never resolves."
        ),
        _ => String::new(),
    };
    Some(format!(
        "`{id}` is not one of this turn's actions.{elsewhere} The ids that work are the ones in the \
         list you were given: {}. Pick one of those.",
        menu.iter().map(|offered| format!("`{offered}`")).collect::<Vec<_>>().join(", "),
    ))
}

/// The cartridge's own rule behind a battle id the menu did not offer, or `""` where the menu
/// cannot say what it is.
fn battle_rule_behind(id: &str, menu: &[String]) -> &'static str {
    let has = |prefix: &str| menu.iter().any(|offered| offered.starts_with(prefix));
    if id == "run" && has("fight:") {
        return " There is no running from a trainer battle: the cartridge answers a RUN there by \
                printing so and putting the same menu straight back, which is why it is not \
                offered. Win the fight, or switch to something that can.";
    }
    if let Some(item) = id.strip_prefix("item:") && has("fight:") {
        let name = item.split('@').next().unwrap_or(item);
        let aimed = menu.iter().any(|offered| offered.starts_with(&format!("item:{name}@")));
        if !item.contains('@') && aimed {
            return " That item asks which Pokémon it is for, so its rows carry the party slot after an \
                    `@`, as `item:Potion@1` does. Use one of those.";
        }
        if item.contains('@') {
            return " Either the bag has none left, or it would do nothing for that Pokémon and the \
                    cartridge would only say \"It won't have any effect.\" An item that asks which \
                    Pokémon has a row for each one it would help: a Revive for one that has fainted, a \
                    potion for one that is hurt, a cure for one with that status. `read_bag` says what \
                    is left.";
        }
        return " That item is not in the bag. A bag row goes the moment the last one is used, so an \
                id you read on an earlier turn stops resolving; `read_bag` says what is left.";
    }
    ""
}

/// Decide what a call is, without touching the game.
pub fn classify(kind: DecisionKind, call: &ToolCall, menu: &[String]) -> CallKind {
    match classify_call(kind, call, menu) {
        CallKind::Terminal(_) if call_summary(call).is_none() => CallKind::Rejected(format!(
            "`{}` needs a `summary`: one or two sentences saying what you are doing and why. Your \
             thinking is not kept, so it is the only note you will have of this turn. Call it again \
             with one.",
            call.function.name,
        )),
        classified => classified,
    }
}

fn classify_call(kind: DecisionKind, call: &ToolCall, menu: &[String]) -> CallKind {
    let name = call.function.name.as_str();
    // A read not offered in this kind is answered like a terminal tool from the wrong kind.
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

    if BATTLE_SCRIPT_TOOL_NAMES.contains(&name) {
        // Named with the reason, like a read from the wrong kind above.
        if !offers_battle_script(kind) {
            return CallKind::Rejected(format!(
                "`{name}` is only offered on an overworld turn: mid-battle is not the moment to be \
                 writing one. Decide this turn, and set the script when you are back outside.",
            ));
        }
        if let Some(call) = classify_battle_script(name, &arguments) {
            return call;
        }
    }

    if name == REPORT_ISSUE {
        if !offers_issue_report(kind) {
            return CallKind::Rejected(format!(
                "`{REPORT_ISSUE}` is not available in a {} turn. The tools that do not end the turn \
                 are {}.",
                kind.label(),
                non_terminal_names(kind).join(", "),
            ));
        }
        return match issue_message(call) {
            Some(message) => CallKind::Issue(message),
            None => CallKind::Rejected(format!(
                "`{REPORT_ISSUE}` needs a `message` saying what you tried, what you expected and \
                 what happened. It does not end your turn: file it, then take an action."
            )),
        };
    }

    match name {
        "choose_action" if kind == DecisionKind::Overworld => match chosen_actions(&arguments, menu) {
            Ok(terminal) => CallKind::Terminal(terminal),
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "choose_battle_action" if kind == DecisionKind::Battle => match string_argument(&arguments, "id") {
            Ok(id) => match not_on_the_menu(&id, menu) {
                // An absent `take_over` is `false`, and so is one sent as anything but a boolean.
                None => CallKind::Terminal(Terminal::ChooseBattleAction {
                    id,
                    take_over: arguments.get("take_over").and_then(Value::as_bool).unwrap_or(false),
                }),
                Some(complaint) => {
                    CallKind::Rejected(format!("{complaint}{}", battle_rule_behind(&id, menu)))
                }
            },
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "use_field_move" if kind == DecisionKind::Overworld => match field_move_arguments(&arguments) {
            Ok(request) => CallKind::Terminal(Terminal::UseFieldMove(request)),
            Err(complaint) => CallKind::Rejected(complaint),
        },
        // `Stuck` only.
        "press_buttons" if kind == DecisionKind::Stuck => match button_arguments(&arguments) {
            Ok(buttons) => match call_reason(call) {
                Some(_) => CallKind::Terminal(Terminal::PressButtons { buttons }),
                // Enforced here, as the `why` is the headline of the press record.
                None => CallKind::Rejected(
                    "`press_buttons` needs a `why`: what you think is on the screen, and what \
                     these presses are meant to do about it. Every press is filed and read."
                        .to_string(),
                ),
            },
            Err(complaint) => CallKind::Rejected(complaint),
        },
        "set_nickname" if kind == DecisionKind::Nickname => {
            // Absent or empty is "keep the default", as the naming screen reads an empty buffer.
            let name = arguments
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| name.chars().take(MAX_NICKNAME).collect::<String>());
            // Written straight into the naming screen's buffer, so nothing else checks it.
            match name.as_deref().and_then(unencodable) {
                Some(bad) => CallKind::Rejected(format!(
                    "`{bad}` is not a character this game can write. A nickname may use letters, \
                     digits, spaces and `.,:;'-?!()[]/`, and nothing else — pick a name out of those \
                     and call `set_nickname` again."
                )),
                None => CallKind::Terminal(Terminal::SetNickname { name }),
            }
        }
        "buy_item" if kind == DecisionKind::MartPurchase => {
            let Some(name) = arguments.get("item").and_then(Value::as_str).filter(|n| !n.is_empty())
            else {
                return CallKind::Terminal(Terminal::BuyItem { item: None, then: Vec::new() });
            };
            let head = match purchase(name, arguments.get("quantity")) {
                Ok(item) => item,
                Err(failure) => return CallKind::Rejected(failure),
            };
            // Every chained order is parsed before any is placed, as `chosen_actions` does.
            let mut then = Vec::new();
            if let Some(more) = arguments.get("then") {
                let Some(list) = more.as_array() else {
                    return CallKind::Rejected(
                        "`then` has to be a list of `{item, quantity}` objects.".to_string());
                };
                if list.len() > MAX_CHAINED_PURCHASES {
                    return CallKind::Rejected(format!(
                        "`then` takes at most {MAX_CHAINED_PURCHASES} more kinds and you gave {}. \
                         Nothing was bought — ask again with a shorter list.", list.len()));
                }
                for entry in list {
                    let Some(name) = entry.get("item").and_then(Value::as_str) else {
                        return CallKind::Rejected(
                            "Every entry in `then` needs an `item` name from the stock list.".to_string());
                    };
                    match purchase(name, entry.get("quantity")) {
                        Ok(item) => then.push(item),
                        Err(failure) => return CallKind::Rejected(failure),
                    }
                }
            }
            CallKind::Terminal(Terminal::BuyItem { item: Some(head), then })
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
        // Not the generic sentence below.
        "press_buttons" => CallKind::Rejected(format!(
            "`press_buttons` is not available on a turn that has a menu; the agent presses the \
             buttons. Take one of: {}. If what you want genuinely is not in the menu, call \
             `{REPORT_ISSUE}` to say so and then take the closest action there is.",
            terminal_names(kind).join(", "),
        )),
        // A terminal tool from another decision kind.
        "choose_action" | "choose_battle_action" | "use_field_move"
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

/// One `{item, quantity}` order, resolved against the game's item list.
fn purchase(name: &str, quantity: Option<&Value>) -> Result<BagItem, String> {
    let Some(item) = item_by_name(name) else {
        return Err(format!(
            "`{name}` is not an item this game has. Copy a name from the stock list exactly."));
    };
    let quantity = quantity.and_then(Value::as_u64).unwrap_or(1).clamp(1, 99) as u8;
    Ok(BagItem::new(item, quantity))
}

fn string_argument(arguments: &Value, key: &str) -> Result<String, String> {
    match arguments.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_string()),
        _ => Err(format!("`{key}` is required and must be a non-empty string.")),
    }
}

// ── Parsing the awkward arguments ────────────────────────────────────────────────────────────────

/// A `choose_action` call: the id, whatever is chained behind it, and whether a battle ends it.
fn chosen_actions(arguments: &Value, menu: &[String]) -> Result<Terminal, String> {
    let id = string_argument(arguments, "id")?;
    if let Some(complaint) = not_on_the_menu(&id, menu) {
        return Err(complaint);
    }

    let malformed = || {
        "`then` is a list of further ids from this turn's action menu, as strings; omit it to take \
         one action."
            .to_string()
    };
    let then: Vec<String> = match arguments.get("then") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item.as_str().map(str::trim).filter(|next| !next.is_empty()) {
                Some(next) => match not_on_the_menu(next, menu) {
                    None => Ok(next.to_string()),
                    Some(complaint) => Err(complaint),
                },
                None => Err(malformed()),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(malformed()),
    };
    if then.len() + 1 > MAX_CHAINED_ACTIONS {
        return Err(format!(
            "That chains {} actions and one call may carry at most {MAX_CHAINED_ACTIONS}. Keep the \
             first {MAX_CHAINED_ACTIONS} and ask again once they are done; you will be shown a \
             fresh menu then anyway.",
            then.len() + 1,
        ));
    }

    Ok(Terminal::ChooseAction {
        id,
        then,
        resume_after_battle: arguments
            .get("resume_after_battle")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

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
        "fly" => {
            let name = string_argument(arguments, "map")?;
            map_by_name(&name)
                .map(|to| FieldMoveRequest::Fly { to })
                .ok_or_else(|| format!("`{name}` is not a map. `read_world_graph` lists the ones you know."))
        }
        "teach" => Ok(FieldMoveRequest::Teach { item: item("item")?, slot: slot()? }),
        "evolve" => Ok(FieldMoveRequest::Evolve { stone: item("item")?, slot: slot()? }),
        // Both optional, and a `target` that is present is still checked.
        "use_item" => Ok(FieldMoveRequest::UseItem {
            item: item("item")?,
            target: match arguments.get("target") {
                Some(Value::Null) | None => None,
                Some(_) => Some(target()?),
            },
            slot: match arguments.get("slot") {
                Some(Value::Null) | None => None,
                Some(_) => Some(slot()?),
            },
        }),
        "toss_item" => Ok(FieldMoveRequest::TossItem { item: item("item")? }),
        "pc_pokemon" => {
            use crate::pokemon::postgame::pc_box::{PcBoxOp, BOX_CAPACITY, BOX_COUNT};
            let op = string_argument(arguments, "op")?;
            let box_slot = || -> Result<u8, String> {
                match arguments.get("box_slot").and_then(Value::as_u64) {
                    Some(slot) if (slot as usize) < BOX_CAPACITY => Ok(slot as u8),
                    Some(slot) => Err(format!("There is no box slot {slot}; a box holds {BOX_CAPACITY}.")),
                    None => Err(format!("`{op}` needs a `box_slot` — `read_pc` numbers them.")),
                }
            };
            Ok(FieldMoveRequest::UsePcBox {
                op: match op.trim().to_ascii_lowercase().as_str() {
                    "deposit" => PcBoxOp::Deposit { slot: slot()? },
                    "withdraw" => PcBoxOp::Withdraw { box_slot: box_slot()? },
                    "release" => PcBoxOp::Release { box_slot: box_slot()? },
                    "change_box" => match arguments.get("box").and_then(Value::as_u64) {
                        // 1-based for the model, because the game's own menu is; 0-based inside.
                        Some(n) if (1..=u64::from(BOX_COUNT)).contains(&n) => PcBoxOp::ChangeBox { n: n as u8 - 1 },
                        Some(n) => return Err(format!("There is no box {n}; there are {BOX_COUNT}.")),
                        None => return Err("`change_box` needs a `box`, 1 to 12.".to_string()),
                    },
                    other => return Err(format!(
                        "`{other}` is not a PC operation: deposit, withdraw, release or change_box.")),
                },
            })
        }
        "pc_items" => {
            use crate::pokemon::postgame::item_storage::PcItemOp;
            let op = string_argument(arguments, "op")?;
            Ok(FieldMoveRequest::UseItemPc {
                op: match op.trim().to_ascii_lowercase().as_str() {
                    "deposit" => PcItemOp::Deposit,
                    "withdraw" => PcItemOp::Withdraw,
                    other => return Err(format!("`{other}` is not a PC item operation: deposit or withdraw.")),
                },
                item: item("item")?,
                qty: match arguments.get("quantity").and_then(Value::as_u64) {
                    Some(qty) if (1..=99).contains(&qty) => qty as u8,
                    Some(qty) => return Err(format!("{qty} is not a quantity the game can move; 1 to 99.")),
                    None => 1,
                },
            })
        }
        "elevator" => {
            let name = string_argument(arguments, "map")?;
            map_by_name(&name)
                .map(|to| FieldMoveRequest::UseElevator { to })
                .ok_or_else(|| format!("`{name}` is not a map. The lift's own panel lists the floors it serves."))
        }
        "reorder_party" => Ok(FieldMoveRequest::ReorderParty { slot: slot()? }),
        "day_care" => match string_argument(arguments, "op")?.trim().to_ascii_lowercase().as_str() {
            "deposit" => Ok(FieldMoveRequest::DayCare { deposit: Some(slot()?) }),
            "withdraw" => Ok(FieldMoveRequest::DayCare { deposit: None }),
            other => Err(format!("`{other}` is not a Day Care operation: deposit or withdraw.")),
        },
        "prize" => {
            let name = string_argument(arguments, "item")?;
            crate::pokemon::postgame::game_corner::Prize::named(&name)
                .map(|prize| FieldMoveRequest::RedeemPrize { prize })
                .ok_or_else(|| format!(
                    "`{name}` is not a prize. The counter sells Abra, Clefairy, Nidorina, Dratini, \
                     Scyther and Porygon, and TM23, TM15 and TM50."))
        }
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

/// Compare names ignoring case and punctuation: `"HM01 Cut"`, `"hm01_cut"` and `"Hm01Cut"` match.
#[cfg(test)]
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

fn same_name(a: &str, b: &str) -> bool {
    let normalise = |name: &str| -> String {
        name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
    };
    normalise(a) == normalise(b)
}

/// An item by name, scanning every discriminant, since `ItemId` has no `FromStr`.
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

// ── Servicing a read ─────────────────────────────────────────────────────────────────────────────

/// Answer one read tool from the triple the policy holds at a poll.
pub fn service_read(
    call: &ToolCall,
    state: &GameState,
    api: &PokemonApi<'_>,
    graph: &WorldGraph,
) -> ToolAnswer {
    // A `MetaTileMap`, never pixels: the picture is drawn on the worker thread.
    let map = match call.function.name.as_str() {
        "read_map" => Some(Box::new(state.map.clone())),
        _ => None,
    };
    // Answered before the JSON arms below and returned as raw text: see [`READ_GUIDE`].
    if call.function.name == READ_GUIDE {
        return ToolAnswer::text(crate::llm::guide::chapter(state.badges));
    }
    let value = match call.function.name.as_str() {
        "read_map" => serde_json::to_value(observe::map_view(state)),
        "read_party" => serde_json::to_value(observe::party(state)),
        "read_bag" => serde_json::to_value(observe::bag(state, api)),
        "read_pc" => serde_json::to_value(observe::pc(state, api)),
        "read_battle" => serde_json::to_value(observe::battle(state)),
        READ_ROUTE => serde_json::to_value(route_answer(call, state, graph)),
        other => Ok(json!({ "error": format!("`{other}` is not a read tool") })),
    };
    match value.and_then(|value| serde_json::to_string(&value)) {
        Ok(json) => ToolAnswer { json, map, is_dark: state.map_is_dark },
        // Cannot fail in practice, and this is on the worker's critical path.
        Err(failure) => ToolAnswer::text(
            format!("{{\"error\": \"could not encode the result: {failure}\"}}")),
    }
}

/// [`READ_ROUTE`], answered from the map-header graph: `None` means not walked yet, never
/// unreachable.
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

    match observe::route_from(graph, &state.map, to) {
        Some(hops) => {
            let mut answer = json!({
                "from": format!("{}", state.map.map), "to": format!("{to}"), "route": hops });
            // The graph knows which maps touch, not which parts of a map do, so say so.
            if let Some(unreachable) = hops.get(1).filter(|hop| hop.reachable_from_here == Some(false)) {
                let blockers = state.map.boundary_blockers();
                answer["warning"] = json!(format!(
                    "This route is correct and you cannot start it from where you are standing. It \
                     leaves {} by `{}`, and that square is not reachable on foot from here{}. The \
                     route graph is built from the game's own map headers, which say which maps \
                     touch each other, not which parts of a map touch each other; {} is split, and \
                     you are on the wrong part of it. What joins two parts of one map is usually a \
                     door: take a warp you can reach and look for a back exit, or leave by an edge \
                     you can reach and come back onto {} by a different one. The action menu is the \
                     list of what you can actually get to.",
                    state.map.map,
                    unreachable.via.as_deref().unwrap_or("that connection"),
                    match blockers.split_first() {
                        Some((first, _)) => format!(" — what your side of it ends on is {first}"),
                        None => String::new(),
                    },
                    state.map.map,
                    state.map.map,
                ));
            }
            answer
        }
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

/// The id of one overworld action, as the menu renders it.
pub fn overworld_id(_state: &GameState, action: &OverworldAction) -> String {
    action.id()
}

/// What one menu row says beyond its id: the action, in words.
fn overworld_description(state: &GameState, action: &OverworldAction) -> String {
    use crate::pokemon::sprite::PictureId;
    use crate::pokemon::tile::{HiddenObject, MetaTile};
    match action.tile {
        MetaTile::Warp { to_map, to_position } => {
            let (dx, dy) = crate::pokemon::map_header::strip_offset(to_map);
            format!("take the warp to {to_map}, arriving at ({}, {})", to_position.x + dx, to_position.y + dy)
        }
        // The menu holds one row per neighbour, so it says how many ways in there are.
        MetaTile::Connection { to_map, .. } => {
            let others: Vec<String> = state.map.crossings(to_map).into_iter()
                .filter(|crossing| crossing.reachable && crossing.at != action.destination)
                // Minted by `overworld_id`, so prose ids and resolvable ids cannot drift apart.
                .map(|crossing| overworld_id(state, &OverworldAction {
                    map: state.map.map,
                    origin: state.map.player_position,
                    destination: crossing.at,
                    tile: MetaTile::Connection { to_map, to_position: crossing.to_position },
                    route: vec![],
                }))
                .collect();
            match others.is_empty() {
                true => format!("walk into {to_map}"),
                false => format!(
                    "walk into {to_map}. This map has {} other opening{} into {to_map}, and each \
                     one lands you somewhere different on it: {}",
                    others.len(),
                    if others.len() == 1 { "" } else { "s" },
                    others.join(", "),
                ),
            }
        }
        MetaTile::ConnectionWater(to_map) => format!("surf into {to_map}"),
        MetaTile::Grass => "walk into tall grass to find wild Pokémon".to_string(),
        // Where there is no grass, as a cave's floor and the sea's surface roll the same.
        MetaTile::Pace { water: false } =>
            "walk up and down this floor to find wild Pokémon: here any floor rolls them".to_string(),
        MetaTile::Pace { water: true } =>
            "surf up and down on this water to find wild water Pokémon".to_string(),
        MetaTile::Pc => "use the PC".to_string(),
        // One press of A each; what it does is the cartridge's business.
        MetaTile::Switch { object, ordinal } => {
            let verb = match object {
                HiddenObject::TrashCan => "search this bin for one of the gym's two switches",
                HiddenObject::VendingMachine => match crate::pokemon::tile_map::VENDING_DRINKS.get(ordinal as usize - 1) {
                    Some((drink, price)) => return format!(
                        "buy a {drink} for ¥{price}: the three machines sell the same three drinks, and \
                         each row buys one of them (you stand at ({}, {}))",
                        action.destination.x, action.destination.y),
                    None => "buy a drink from this machine",
                },
                HiddenObject::Poster => "look behind the poster",
                HiddenObject::Statue => "press this statue's switch",
                HiddenObject::CellSeparator => "run the cell separator to turn Bill back into a person",
                HiddenObject::Quiz { yes } => {
                    let question = crate::pokemon::tile_map::CINNABAR_QUIZ_MACHINES.iter()
                        .find(|(x, y, _)| crate::pokemon::tile_map::hidden_objects_for(state.map.map)
                            .get(ordinal as usize - 1)
                            .is_some_and(|site| (site.at.x, site.at.y) == (*x, *y)))
                        .map_or("", |(_, _, question)| question);
                    return format!(
                        "answer {} to this quiz machine's question, \"{question}\" Right, and its gate \
                         opens; wrong, and a trainer battles you (you stand at ({}, {}))",
                        if yes { "YES" } else { "NO" }, action.destination.x, action.destination.y);
                }
            };
            match crate::pokemon::tile_map::hidden_objects_for(state.map.map)
                .get(ordinal as usize - 1) {
                Some(site) => format!("{verb} (you stand at ({}, {}); it is at ({}, {}))",
                                      action.destination.x, action.destination.y, site.at.x, site.at.y),
                None => verb.to_string(),
            }
        }
        // The verb is the whole action.
        MetaTile::Cut { at } => format!("cut down the tree at ({}, {})", at.x, at.y),
        // The row exists only with a rod and water, so it says what fishing is for.
        MetaTile::Fish { rod } => format!(
            "fish at the water's edge with the {} to find wild water Pokemon", rod.name()),
        MetaTile::Sprite(name) => {
            let sprite = state.map.sprites.iter().find(|s| s.name == name);
            // Both squares: the id's is where the player stands, `target` is where the thing is.
            let stand_and_target = |verb: &str| match sprite {
                Some(sprite) => format!(
                    "{verb} (you stand at ({}, {}); it is at ({}, {}), which is the `target`)",
                    action.destination.x, action.destination.y, sprite.position.x, sprite.position.y),
                None => verb.to_string(),
            };
            match sprite.map(|s| s.picture_id) {
                Some(PictureId::PokeBall) => format!("pick up the {}", name.trim_end_matches(|c: char| c.is_ascii_digit()).trim()),
                Some(PictureId::Fossil | PictureId::OldAmber | PictureId::UnusedOldAmber) => format!("examine the {name}"),
                Some(PictureId::Paper | PictureId::Pokedex | PictureId::Clipboard) => format!("read the {name}"),
                Some(PictureId::Snorlax) => stand_and_target("walk up to the sleeping Snorlax blocking the way"),
                _ => format!("talk to {name}"),
            }
        }
        other => format!("walk to {other}"),
    }
}

/// Everything reachable, sorted: `actions()` walks a `HashSet`, and a reshuffled menu reads as
/// the world having moved.
pub fn overworld_menu(state: &GameState, arrival: Option<crate::pokemon::world_graph::Arrival>) -> Vec<MenuItem> {
    use crate::pokemon::tile::MetaTile;
    let arrival = arrival.filter(|a| a.map == state.map.map);
    let way_back = |action: &OverworldAction| -> bool {
        let Some(arrival) = arrival else { return false };
        match action.tile {
            MetaTile::Warp { .. } => action.destination == arrival.at,
            MetaTile::Connection { to_map, .. } | MetaTile::ConnectionWater(to_map) => Some(to_map) == arrival.from,
            _ => false,
        }
    };
    let mut actions = state.map.actions();
    actions.retain(|action| !matches!(action.tile, MetaTile::Pc));
    actions.retain(|action| !matches!(action.tile, MetaTile::Sprite(name)
        if state.map.sprites.iter().any(|s| s.name == name
            && s.picture_id == crate::pokemon::sprite::PictureId::Boulder)));
    // `id_kind`, not `kind`: two people can share the tile an action approaches them from.
    actions.sort_by_key(|action| (action.destination.y, action.destination.x, action.tile.id_kind()));
    // Two doors of one building are two places, so the rows say which side.
    let mut repeated: std::collections::HashMap<crate::pokemon::map::Map, usize> =
        std::collections::HashMap::new();
    for action in &actions {
        if let MetaTile::Warp { to_map, .. } = action.tile {
            *repeated.entry(to_map).or_default() += 1;
        }
    }
    actions
        .iter()
        .map(|action| MenuItem {
            id: overworld_id(state, action),
            description: {
                let mut description = overworld_description(state, action);
                if let MetaTile::Warp { to_map, .. } = action.tile
                    && repeated.get(&to_map).copied().unwrap_or(0) > 1
                    && let Some(side) = door_side(&state.map, action.destination) {
                    description.push_str(&format!("; this one is on the {side} side of the map"));
                }
                if way_back(action) {
                    description.push_str("; the way you came in");
                }
                description
            },
        })
        .collect()
}

/// Which side of the map a door is on, as the word a person would use.
fn door_side(map: &crate::pokemon::tile_map::MetaTileMap, at: Point8) -> Option<&'static str> {
    let dx = at.x as f32 - (map.width.saturating_sub(1)) as f32 / 2.0;
    let dy = at.y as f32 - (map.height.saturating_sub(1)) as f32 / 2.0;
    match dx.abs() >= dy.abs() {
        true if dx < 0.0 => Some("west"),
        true if dx > 0.0 => Some("east"),
        false if dy < 0.0 => Some("north"),
        false if dy > 0.0 => Some("south"),
        // Dead centre on the deciding axis: there is no side to name.
        _ => None,
    }
}

/// Match an id against a fresh action list; `None` means the action is gone.
pub fn resolve_overworld(state: &GameState, id: &str) -> Option<OverworldAction> {
    use crate::pokemon::tile::MetaTile;
    if let Some(action) = state.map.actions().into_iter().find(|action| overworld_id(state, action) == id) {
        return Some(action);
    }
    let map = &state.map;
    map.meta_tiles.iter().enumerate().find_map(|(index, tile)| {
        let MetaTile::Connection { to_map, to_position } = *tile else { return None };
        let at = Point8 { x: (index % map.width) as u8, y: (index / map.width) as u8 };
        // Minted as the menu would; the route is a throwaway that `connection_action` replaces.
        let candidate = OverworldAction {
            map: map.map, origin: map.player_position, destination: at, tile: *tile, route: vec![] };
        (overworld_id(state, &candidate) == id).then(|| map.connection_action(to_map, to_position))?
    })
}

/// Keyed on what the action is: a bag slot shifts when an item runs out, and PP changes on use.
pub fn battle_id(action: &BattleAction) -> String {
    match action {
        BattleAction::Fight { battle_move, .. } => format!("fight:{}", battle_move.name),
        BattleAction::UseItem { item, target: None, .. } => format!("item:{:?}", item.id),
        BattleAction::UseItem { item, target: Some(target), .. } => format!("item:{:?}@{target}", item.id),
        BattleAction::SwitchPokemon { slot, .. } => format!("switch:{slot}"),
        BattleAction::Run => "run".to_string(),
        BattleAction::SafariBall => "ball".to_string(),
        BattleAction::SafariBait => "bait".to_string(),
        BattleAction::SafariRock => "rock".to_string(),
    }
}

/// The battle menu, with every `fight:` row costed against the Pokémon actually in front of you.
pub fn battle_menu(state: &GameState) -> Vec<MenuItem> {
    let sides = state.battle.as_ref().map(|battle| (&battle.player, &battle.enemy));
    battle_options(state)
        .unwrap_or_default()
        .iter()
        .map(|action| {
            let mut description = format!("{action}");
            if let (BattleAction::Fight { battle_move, .. }, Some((me, foe))) = (action, sides) {
                description.push_str(&fight_row_note(battle_move.name, me, foe));
            }
            if let BattleAction::UseItem { item, target: Some(target), .. } = action
                && let Some(mon) = state.pokemon.get(*target as usize)
            {
                let condition = match (mon.current_hp, mon.status) {
                    (0, _) => "fainted".to_string(),
                    (hp, crate::pokemon::status::PokemonStatus::None) => format!("{hp}/{} HP", mon.stats.hp),
                    (hp, status) => format!("{hp}/{} HP, {status}", mon.stats.hp),
                };
                description = format!("ITEM   {} ×{} on {} the {}, {condition}",
                    item.id, item.quantity, mon.nickname.to_default_string(), mon.species);
            }
            MenuItem { id: battle_id(action), description }
        })
        .collect()
}

/// Roughly what a `fight:` row would take off, and the cartridge's words for the multiplier.
fn fight_row_note(
    name: crate::pokemon::move_name::PokemonMoveName,
    me: &crate::pokemon::pokemon::PokemonSummary,
    foe: &crate::pokemon::pokemon::PokemonSummary,
) -> String {
    use crate::pokemon::damage::{effectiveness_phrase, expected_damage, is_damaging_move, type_multiplier};
    // The multiplier is only reported for a move that deals damage.
    if !is_damaging_move(name) {
        return String::new();
    }
    match (expected_damage(me, name, foe), effectiveness_phrase(type_multiplier(name, foe))) {
        // Immune: no damage, and the phrase is the whole decision.
        (_, Some(phrase @ "no effect")) => format!(" — {phrase}"),
        (Some(damage), Some(phrase)) => format!(
            " — ~{damage} damage ({}% of its HP), {phrase}", percent_of(damage, foe.stats.hp)),
        (Some(damage), None) => format!(
            " — ~{damage} damage ({}% of its HP)", percent_of(damage, foe.stats.hp)),
        // A damaging move the estimator declines to price.
        (None, _) => String::new(),
    }
}

/// Damage as a share of the defender's maximum HP, so a two-hit kill reads as one.
fn percent_of(damage: u16, max_hp: u16) -> u16 {
    match max_hp {
        0 => 0,
        max => ((damage as u32 * 100 / max as u32) as u16).min(100),
    }
}

pub fn resolve_battle(state: &GameState, id: &str) -> Option<BattleAction> {
    battle_options(state)?.into_iter().find(|action| battle_id(action) == id)
}

/// What the mart sells (see [`ApiSnapshot`]); the id is the item's name, as `buy_item` takes.
pub fn mart_menu(snapshot: &ApiSnapshot, state: &GameState) -> Vec<MenuItem> {
    snapshot
        .mart_stock
        .iter()
        .map(|(item, price)| {
            let held = state.bag.iter().find(|entry| entry.id == *item).map_or(0, |e| e.quantity);
            MenuItem {
                id: item.to_string(),
                description: format!(
                    "{} — you have {held}",
                    match price {
                        // A missing price is shown as missing, not as a plausible number.
                        Some(price) => format!("¥{price}"),
                        None => "price unknown".to_string(),
                    },
                ),
            }
        })
        .collect()
}

/// The four moves the forget prompt chooses between, keyed on slot as `forget_move` takes.
pub fn forget_menu(current: &[PokemonMove]) -> Vec<MenuItem> {
    current
        .iter()
        .enumerate()
        .map(|(slot, known)| {
            let metadata = known.name.metadata();
            MenuItem {
                id: slot.to_string(),
                description: format!(
                    "{} — {}, {}, {}/{} pp{}",
                    known.name,
                    metadata.move_type,
                    match metadata.power {
                        Some(power) => format!("{power} power"),
                        None => "no damage".to_string(),
                    },
                    known.pp,
                    metadata.pp,
                    match hm_move(known.name) {
                        Some(_) => " — ⚠️ an HM move, and it cannot be re-learnt",
                        None => "",
                    },
                ),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::protocol::FunctionCall;

    /// A call with a `summary` filled in, since [`classify`] refuses a terminal call without one.
    fn call(name: &str, arguments: &str) -> ToolCall {
        let arguments = match serde_json::from_str::<Value>(arguments) {
            Ok(Value::Object(mut object)) => {
                object.entry("summary").or_insert_with(|| json!("doing the thing"));
                Value::Object(object).to_string()
            }
            _ => arguments.to_string(),
        };
        bare(name, &arguments)
    }

    /// A call exactly as written, `summary` and all.
    fn bare(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: "c".into(),
            kind: "function".into(),
            function: FunctionCall { name: name.into(), arguments: arguments.into() },
        }
    }

    fn names(kind: DecisionKind) -> Vec<&'static str> {
        for_kind(kind).into_iter().map(|tool| tool.function.name).collect()
    }

    /// Oak's lab after the starter: a party of one, an ordinary bag, and no trees.
    fn fixture_state() -> GameState {
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/oaks-lab-just-got-squirtle.bin"))
            .expect("the committed fixture loads");
        { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }.expect("the fixture has a readable state")
    }

    fn state_from(fixture: &[u8]) -> GameState {
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(fixture).expect("the committed fixture loads");
        { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("the fixture has a readable state")
    }

    /// A route whose first step cannot be walked from here says so.
    #[test]
    fn a_route_off_a_terrace_the_player_is_not_on_says_it_cannot_be_started() {
        let state = state_from(include_bytes!("../pokemon/data/split-cerulean.bin"));
        assert_eq!(state.map.map, Map::CeruleanCity);

        let mut graph = WorldGraph::new();
        // The lower terrace, seen earlier.
        let mut lower = state.map.clone();
        lower.player_position = Point8 { x: 17, y: 36 };
        graph.observe(Map::CeruleanCity, lower.player_position, &lower);
        // …and where the run is standing now.
        graph.observe(Map::CeruleanCity, state.map.player_position, &state.map);

        let answer = route_answer(&call(READ_ROUTE, r#"{"to":"Route5"}"#), &state, &graph);
        let hops = answer["route"].as_array().expect("a route, because one exists");
        assert!(hops.len() >= 2, "Cerulean then Route 5: {answer}");

        // The route is still answered.
        assert_eq!(hops[1]["reachable_from_here"], json!(false), "{answer}");
        let warning = answer["warning"].as_str().expect("a warning beside it");
        assert!(warning.contains("cannot start it from where you are standing"), "{warning}");
        // The way between two parts of one map is a door.
        assert!(warning.contains("door"), "it has to name the way round: {warning}");
        assert!(warning.contains("map headers"), "and why the route disagrees with the menu: {warning}");

        // The reachable crossing is not flagged, or the flag says nothing.
        let route4 = route_answer(&call(READ_ROUTE, r#"{"to":"Route4"}"#), &state, &graph);
        assert!(route4.get("warning").is_none(), "Route 4 is walkable from here: {route4}");
    }

    /// A destination with no reachable crossing has no row in the action menu.
    #[test]
    fn a_fenced_in_map_names_the_neighbours_it_cannot_reach() {
        use crate::pokemon::tile::MetaTile;
        for (fixture, map, cut_off) in [
            (&include_bytes!("../pokemon/data/split-cerulean.bin")[..], Map::CeruleanCity,
             vec![Map::Route5, Map::Route9]),
            (&include_bytes!("../pokemon/data/split-celadon.bin")[..], Map::CeladonCity,
             vec![Map::Route16, Map::Route7]),
            (&include_bytes!("../pokemon/data/pocket-route14.bin")[..], Map::Route14,
             vec![Map::Route15]),
        ] {
            let state = state_from(fixture);
            assert_eq!(state.map.map, map);
            let mut expected = cut_off.clone();
            expected.sort_by_key(|m| format!("{m}"));

            let offered: Vec<Map> = state.map.actions().iter().filter_map(|action| match action.tile {
                MetaTile::Connection { to_map, .. } => Some(to_map),
                _ => None,
            }).collect();
            // A water seam is the Surf gate, excluded because `crossings` skips `ConnectionWater`.
            let mut missing: Vec<Map> = state.map.connection_targets.iter().copied()
                .filter(|to| !offered.contains(to) && !state.map.crossings(*to).is_empty())
                .collect();
            missing.sort_by_key(|m| format!("{m}"));
            assert_eq!(missing, expected, "on {map}: the menu's silences must be exactly the fences");

            for to in expected {
                // Every crossing into it exists and none of them can be walked to.
                let crossings = state.map.crossings(to);
                assert!(!crossings.is_empty(), "{map} does border {to}");
                assert!(crossings.iter().all(|c| !c.reachable), "{map} -> {to}: {crossings:?}");
            }
        }
    }

    /// The model can ask for a crossing the menu did not offer.
    #[test]
    fn a_crossing_the_menu_did_not_offer_can_still_be_chosen() {
        use crate::pokemon::tile::MetaTile;
        let mut state = state_from(include_bytes!("../pokemon/data/pocket-route14.bin"));
        state.map.player_position = Point8 { x: 10, y: 4 };

        let reachable: Vec<_> = state.map.crossings(Map::Route13).into_iter()
            .filter(|crossing| crossing.reachable).collect();
        assert!(reachable.len() > 1, "the road has several openings east: {reachable:?}");

        let menu = overworld_menu(&state, None);
        let offered: Vec<&MenuItem> = menu.iter()
            .filter(|row| row.id.ends_with(":Connection")).collect();
        // Still one row per adjacent map: this map borders Route 13 and Route 15.
        assert_eq!(offered.len(), 2, "one row per neighbour, not one per tile: {offered:?}");
        let offered: Vec<&MenuItem> = offered.into_iter()
            .filter(|row| row.description.contains(&format!("{}", Map::Route13))).collect();
        assert_eq!(offered.len(), 1, "{offered:?}");
        assert!(offered[0].description.contains("2 other openings"),
                "and it says the others are there: {}", offered[0].description);

        for crossing in reachable {
            let id = overworld_id(&state, &OverworldAction {
                map: state.map.map, origin: state.map.player_position, destination: crossing.at,
                tile: MetaTile::Connection { to_map: Map::Route13, to_position: crossing.to_position },
                route: vec![],
            });
            // Named in the row unless it is the row, and choosable either way.
            if id != offered[0].id {
                assert!(offered[0].description.contains(&id), "{} omits {id}", offered[0].description);
            }
            let resolved = resolve_overworld(&state, &id).unwrap_or_else(|| panic!("{id} resolves"));
            assert_eq!(resolved.destination, crossing.at);
            assert!(!resolved.route.is_empty(), "{id} comes with a walk to it");
        }

        // An unreachable crossing is still refused.
        let pocket = state.map.crossings(Map::Route13).into_iter()
            .find(|crossing| !crossing.reachable).expect("the pocket rows are not reachable from the road");
        let id = overworld_id(&state, &OverworldAction {
            map: state.map.map, origin: state.map.player_position, destination: pocket.at,
            tile: MetaTile::Connection { to_map: Map::Route13, to_position: pocket.to_position },
            route: vec![],
        });
        assert!(resolve_overworld(&state, &id).is_none(), "{id} cannot be walked to");
    }

    /// Refused inside the turn, rather than left for the policy to find out.
    #[test]
    fn a_pc_or_a_lift_that_is_not_here_is_refused_inside_the_turn() {
        use crate::pokemon::postgame::item_storage::PcItemOp;
        use crate::pokemon::postgame::pc_box::PcBoxOp;
        // Oak's lab: no PC, no lift.
        let state = fixture_state();

        let refused = |request| resolve_field_move(&state, &request).expect_err("refused");
        let no_pc = refused(FieldMoveRequest::UsePcBox { op: PcBoxOp::Deposit { slot: 0 } });
        assert!(no_pc.contains("no PC on"), "says which map has none: {no_pc}");
        assert!(no_pc.contains("Pokémon Centre"), "and where one is: {no_pc}");
        let no_pc_items = refused(FieldMoveRequest::UseItemPc {
            op: PcItemOp::Deposit, item: ItemId::Potion, qty: 1,
        });
        assert!(no_pc_items.contains("no PC on"), "{no_pc_items}");

        let no_lift = refused(FieldMoveRequest::UseElevator { to: Map::SilphCo5F });
        assert!(no_lift.contains("no lift on"), "{no_lift}");
        assert!(no_lift.contains("Silph"), "names where the three lifts are: {no_lift}");
    }

    /// A floor the lift does not stop at lists the floors: the model cannot see the panel.
    #[test]
    fn a_lift_says_which_floors_it_serves() {
        let mut state = fixture_state();
        state.map.map = Map::RocketHideoutElevator;
        let wrong_floor = resolve_field_move(&state, &FieldMoveRequest::UseElevator { to: Map::SilphCo5F })
            .expect_err("this lift does not go to Silph Co");
        assert!(wrong_floor.contains("does not stop at"), "{wrong_floor}");
        assert!(wrong_floor.contains("RocketHideoutB4F"), "lists the floors: {wrong_floor}");

        // The hideout's lift needs the key.
        let no_key = resolve_field_move(&state, &FieldMoveRequest::UseElevator { to: Map::RocketHideoutB4F })
            .expect_err("no Lift Key in Oak's lab");
        assert!(no_key.contains("LIFT_KEY"), "names the key: {no_key}");
    }

    #[test]
    fn read_pc_is_offered_on_the_only_turn_that_can_use_it() {
        assert!(names(DecisionKind::Overworld).contains(&"read_pc"));
        for elsewhere in [DecisionKind::Battle, DecisionKind::Nickname, DecisionKind::MartPurchase,
                          DecisionKind::ForgetMove, DecisionKind::Stuck] {
            assert!(!names(elsewhere).contains(&"read_pc"), "{elsewhere:?} must not offer read_pc");
        }
    }

    #[test]
    fn the_pc_and_the_lift_are_field_moves_a_model_can_name() {
        use crate::pokemon::postgame::item_storage::PcItemOp;
        use crate::pokemon::postgame::pc_box::PcBoxOp;
        let request = |arguments: &str| field_move_arguments(
            &serde_json::from_str::<Value>(arguments).expect("valid JSON")).expect("parses");

        assert_eq!(request(r#"{"move":"pc_pokemon","op":"deposit","slot":2}"#),
                   FieldMoveRequest::UsePcBox { op: PcBoxOp::Deposit { slot: 2 } });
        assert_eq!(request(r#"{"move":"pc_pokemon","op":"withdraw","box_slot":7}"#),
                   FieldMoveRequest::UsePcBox { op: PcBoxOp::Withdraw { box_slot: 7 } });
        // 1-based on the wire, as the cartridge's CHANGE BOX menu counts, and 0-based inside.
        assert_eq!(request(r#"{"move":"pc_pokemon","op":"change_box","box":1}"#),
                   FieldMoveRequest::UsePcBox { op: PcBoxOp::ChangeBox { n: 0 } });
        assert_eq!(request(r#"{"move":"pc_items","op":"withdraw","item":"Potion","quantity":3}"#),
                   FieldMoveRequest::UseItemPc { op: PcItemOp::Withdraw, item: ItemId::Potion, qty: 3 });
        // A quantity nobody gave is one, which is what a model omitting it means.
        assert_eq!(request(r#"{"move":"pc_items","op":"deposit","item":"Potion"}"#),
                   FieldMoveRequest::UseItemPc { op: PcItemOp::Deposit, item: ItemId::Potion, qty: 1 });
        assert_eq!(request(r#"{"move":"elevator","map":"SilphCo5F"}"#),
                   FieldMoveRequest::UseElevator { to: Map::SilphCo5F });

        // A misspelt operation is named rather than falling through to a wrong default.
        let bad = field_move_arguments(&serde_json::from_str::<Value>(
            r#"{"move":"pc_pokemon","op":"store","slot":0}"#).expect("valid JSON")).expect_err("refused");
        assert!(bad.contains("deposit, withdraw, release or change_box"), "{bad}");
    }

    /// Two menu rows may never share an id.
    #[test]
    fn no_two_menu_rows_can_share_an_id() {
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/gym-trash-solved.bin")).expect("the fixture loads");
        let state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("a readable state");
        let menu = overworld_menu(&state, None);
        assert!(
            menu.iter().filter(|row| row.id.contains(":TrashCan")).count() >= 15,
            "all fifteen bins are rows, or this proves nothing: {menu:#?}",
        );

        let mut seen = std::collections::HashSet::new();
        for row in &menu {
            assert!(seen.insert(row.id.clone()), "`{}` is offered twice; one of them can never be chosen", row.id);
        }

        // And every one of them still resolves.
        for row in &menu {
            assert!(resolve_overworld(&state, &row.id).is_some(), "`{}` is offered but does not resolve", row.id);
        }
    }

    const KINDS: [DecisionKind; 6] = ALL_KINDS;

    /// A menu row carries what its id cannot, and nothing else.
    #[test]
    fn the_warp_the_player_came_in_by_says_so() {
        use crate::pokemon::world_graph::Arrival;
        let state = fixture_state();
        let door = poke_core::geometry::Point8 { x: 5, y: 11 };
        let marked = overworld_menu(&state, Some(Arrival { map: state.map.map, at: door, from: Some(Map::PalletTown) }));
        let row = |menu: &[MenuItem], id: &str| menu.iter().find(|m| m.id == id).expect(id).description.clone();
        assert_eq!(row(&marked, "OaksLab:5,11:Warp"), "take the warp to PalletTown, arriving at (12, 12); the way you came in");
        assert_eq!(marked.iter().filter(|m| m.description.contains("came in")).count(), 1, "{marked:#?}");

        // An arrival on some other map says nothing about this one.
        let elsewhere = overworld_menu(&state, Some(Arrival { map: Map::PalletTown, at: door, from: None }));
        assert_eq!(row(&elsewhere, "OaksLab:5,11:Warp"), "take the warp to PalletTown, arriving at (12, 12)");
    }

    /// A row says what choosing it does, as a verb phrase; the id is a key, not a description.
    #[test]
    fn a_menu_row_explains_the_action_in_words() {
        let state = fixture_state();
        let menu = overworld_menu(&state, None);
        let rows: Vec<String> =
            menu.iter().map(|item| format!("- `{}` — {}", item.id, item.description)).collect();

        assert!(rows.contains(&"- `OaksLab:5,11:Warp` — take the warp to PalletTown, arriving at (12, 12)".to_string()), "{rows:#?}");
        // A person is named by the id and by the row: the name is what the verb needs.
        assert!(rows.contains(&"- `OaksLab:Pokedex1` — read the Pokedex 1".to_string()), "{rows:#?}");
        assert!(!rows.iter().any(|row| row.contains("Sprite")),
                "no row may call a person a sprite: {rows:#?}");

        for item in &menu {
            assert!(item.description.starts_with(|c: char| c.is_ascii_lowercase()), "{item:?}: not a verb phrase");
            assert!(!item.description.contains("steps"), "{item:?} still carries a distance");
        }
    }

    /// A landing is in the destination's picture coordinates, offset from the warp table's by
    /// its connection strips.
    #[test]
    fn a_landing_coordinate_is_where_the_destination_picture_puts_it() {
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/pallet-town-state.bin")).expect("the committed fixture loads");
        let pallet = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }.expect("readable");
        let lab_door = crate::pokemon::observe::map_view(&pallet).warps.into_iter()
            .find(|w| w.to_map == format!("{}", Map::OaksLab)).expect("Pallet Town has a door into the lab");
        let menu = overworld_menu(&fixture_state(), None);
        let out = menu.iter().find(|m| m.id == "OaksLab:5,11:Warp").expect("the lab's door").description.clone();
        assert_eq!(out, format!("take the warp to PalletTown, arriving at ({}, {})", lab_door.at.x, lab_door.at.y));
        assert_eq!(crate::pokemon::map_header::strip_offset(Map::MtMoon1F), (0, 0));
    }

    /// The verb comes from the sprite's picture, so an item on the ground is picked up and a
    /// person is spoken to, whatever either is called.
    #[test]
    fn a_sprite_row_is_verbed_by_its_picture() {
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/mt-moon.bin")).expect("the committed fixture loads");
        let state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }.expect("readable");
        let menu = overworld_menu(&state, None);
        let find = |needle: &str| menu.iter().find(|m| m.id.ends_with(needle)).unwrap_or_else(|| panic!("{needle}: {menu:#?}")).description.clone();
        assert_eq!(find(":Potion1"), "pick up the Potion");
        assert_eq!(find(":TMWaterGun"), "pick up the TM Water Gun");
        assert_eq!(find(":Hiker"), "talk to Hiker");
    }

    /// `read_bag` has to agree with the bag the game is holding, row for row.
    #[test]
    fn read_bag_counts_every_slot_the_game_counts() {
        use crate::pokemon::PokemonApiTrait;
        use crate::pokemon::symbols::DmgPointerRead;
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/post-ss-anne.bin")).expect("the committed fixture loads");
        let api = crate::pokemon::PokemonApi::new(&mut gb);
        let state = api.game_state().expect("readable");
        let view = crate::pokemon::observe::bag(&state, &api);

        let raw = api.mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wNumBagItems) as usize;
        assert_eq!(view.slots_used, raw, "read_bag says {} of the bag's slots are used, the game says {raw}", view.slots_used);
        assert_eq!(view.items.len(), raw, "and every one of them has to be listed: {:?}", view.items);

        // A machine's name is one the model can quote back into `toss_item` or `teach`.
        let named: Vec<&str> = view.items.iter().map(|i| i.item.as_str()).collect();
        assert!(named.contains(&"Tm34Bide"), "{named:?}");
        assert!(named.contains(&"Hm01Cut"), "{named:?}");
        for name in &named {
            assert!(item_by_name(name).is_some(), "{name} is listed but cannot be named back");
        }

        // Every machine id, which this fixture alone cannot cover.
        for id in 0xC4..=0xFAu8 {
            let item = ItemId::from_repr(id)
                .unwrap_or_else(|| panic!("${id:02X} is a machine and `read_bag` would drop it"));
            let name = item.to_string();
            assert_eq!(item_by_name(&name), Some(item), "{name} does not resolve back to ${id:02X}");
        }
    }

    /// A refused battle id is told the rule that kept it off the menu, and nothing about maps.
    #[test]
    fn a_refused_battle_id_carries_the_rule_and_says_nothing_about_maps() {
        let trainer: Vec<String> = ["fight:Peck", "item:GreatBall", "switch:1"]
            .iter().map(|id| id.to_string()).collect();

        let run = not_on_the_menu("run", &trainer).expect("`run` is not on a trainer menu");
        assert!(!run.contains("map"), "a battle refusal talked about maps: {run}");
        let run = format!("{run}{}", battle_rule_behind("run", &trainer));
        assert!(run.contains("no running from a trainer battle"), "no rule was given: {run}");

        let gone = not_on_the_menu("item:PokeBall", &trainer).expect("no Poké Balls on this menu");
        assert!(!gone.contains("map"), "a battle refusal talked about maps: {gone}");
        let gone = format!("{gone}{}", battle_rule_behind("item:PokeBall", &trainer));
        assert!(gone.contains("not in the bag"), "no rule was given: {gone}");

        // The Safari menu gets neither: `run` is offered and there is no bag.
        let safari: Vec<String> =
            ["ball", "bait", "rock", "run"].iter().map(|id| id.to_string()).collect();
        assert_eq!(battle_rule_behind("item:Potion", &safari), "");

        // An item that asks which Pokémon: a missing target, and a target it would not help.
        let hurt: Vec<String> = ["fight:Peck", "item:Potion@1"].iter().map(|id| id.to_string()).collect();
        assert!(battle_rule_behind("item:Potion", &hurt).contains("`@`"), "the slot was not asked for");
        assert!(battle_rule_behind("item:Revive@0", &hurt).contains("It won't have any effect."));
        assert_eq!(battle_rule_behind("fight:Tackle", &safari), "");

        let here: Vec<String> = vec!["ViridianCity:18,6:Sprite".to_string()];
        let stale = not_on_the_menu("ViridianForest:17,47:Warp", &here)
            .expect("an id from another map is not on this menu");
        assert!(stale.contains("ids are minted for the map"), "the map clause went missing: {stale}");
        // A prefix that is not a map is not reported as one — `foo` is a typo, not a place.
        let typo = not_on_the_menu("foo:bar", &here).expect("not on this menu either");
        assert!(!typo.contains("ids are minted for the map"), "`foo` was called a map: {typo}");
    }

    /// What the `tools` array costs, per kind, with a ceiling on each.
    #[test]
    fn the_tool_array_stays_within_its_budget() {
        // Overworld is the big one: it carries `use_field_move`.
        for (kind, ceiling) in [
            (DecisionKind::Overworld, 13_075),
            (DecisionKind::Battle, 5_575),
            (DecisionKind::Nickname, 4_075),
            (DecisionKind::MartPurchase, 4_825),
            (DecisionKind::ForgetMove, 4_275),
            (DecisionKind::Stuck, 6_025),
        ] {
            let bytes = serde_json::to_string(&for_kind(kind)).expect("the specs serialise").len();
            assert!(bytes <= ceiling, "{kind:?}'s tools are {bytes} bytes, over the {ceiling} budget");
        }
    }

    #[test]
    fn no_tool_text_carries_a_run_of_spaces() {
        fn walk(value: &Value, path: &str, kind: DecisionKind) {
            match value {
                Value::String(text) => assert!(
                    !text.contains("  "),
                    "{kind:?}: {path} has a run of spaces, a literal continued without `\\`: {text:?}",
                ),
                Value::Array(items) => {
                    items.iter().enumerate().for_each(|(i, item)| walk(item, &format!("{path}[{i}]"), kind))
                }
                Value::Object(fields) => {
                    fields.iter().for_each(|(key, field)| walk(field, &format!("{path}.{key}"), kind))
                }
                _ => {}
            }
        }
        for kind in [
            DecisionKind::Overworld,
            DecisionKind::Battle,
            DecisionKind::Nickname,
            DecisionKind::MartPurchase,
            DecisionKind::ForgetMove,
            DecisionKind::Stuck,
        ] {
            walk(&serde_json::to_value(for_kind(kind)).expect("the specs serialise"), "tools", kind);
        }
    }

    /// [`READ_ROUTE`]'s four answers, each a different next step for the model.
    #[test]
    fn a_route_answers_the_four_questions_and_never_bluffs() {
        let state = fixture_state();
        let graph = WorldGraph::new();
        let ask = |arguments: &str| -> Value {
            route_answer(&call(READ_ROUTE, arguments), &state, &graph)
        };

        // No `to`: what has been walked.
        assert_eq!(ask("{}")["visited"], json!([]));
        assert_eq!(ask("{}")["from"], json!(format!("{}", state.map.map)));
        assert_eq!(ask(r#"{"to":""}"#)["visited"], json!([]), "a blank name is no name");

        // A name that is not a map at all: correctable, and it says how.
        let nonsense = ask(r#"{"to":"Kanto Safari Wildlife Park"}"#);
        assert!(nonsense["error"].as_str().expect("a sentence").contains("not a map"), "{nonsense}");

        // A real map nobody has walked to.
        let unwalked = ask(r#"{"to":"CeruleanCity"}"#);
        assert_eq!(unwalked["reachable"], json!(false));
        assert_eq!(unwalked["to"], json!(format!("{}", Map::CeruleanCity)));
        assert!(unwalked["note"].as_str().expect("a sentence").contains("not been to"), "{unwalked}");

        // `map_by_name` normalises the way a model spells a map.
        assert_eq!(ask(r#"{"to":"cerulean city"}"#), unwalked);

        // The whole graph is never serialised, whatever is asked.
        assert!(!ask("{}").to_string().contains("edges"));
    }

    /// A battle menu row is prose, and `BattleAction`'s `Display` is what makes it so.
    fn summary(
        species: crate::pokemon::species::PokemonSpecies,
        types: [crate::pokemon::pokemon::PokemonType; 2],
        moves: &[PokemonMoveName],
    ) -> crate::pokemon::pokemon::PokemonSummary {
        let mut slots = [None, None, None, None];
        for (slot, name) in moves.iter().enumerate() {
            slots[slot] = Some(PokemonMove { name: *name, pp: 20 });
        }
        crate::pokemon::pokemon::PokemonSummary {
            species,
            current_hp: 100,
            status: crate::pokemon::status::PokemonStatus::None,
            types,
            level: 25,
            moves: slots,
            stats: crate::pokemon::pokemon::PokemonStats {
                attack: 50, defense: 50, speed: 50, special: 50, hp: 100,
            },
            disabled_move_slot: None,
        }
    }

    #[test]
    fn a_fight_row_is_costed_against_the_pokemon_in_front_of_it() {
        use crate::pokemon::pokemon::PokemonType::*;
        use crate::pokemon::species::PokemonSpecies;

        let me = summary(PokemonSpecies::Charmander, [Fire, Fire],
                         &[PokemonMoveName::Ember, PokemonMoveName::Growl, PokemonMoveName::Scratch]);
        let foe = summary(PokemonSpecies::Bulbasaur, [Grass, Poison], &[PokemonMoveName::Tackle]);

        let ember = fight_row_note(PokemonMoveName::Ember, &me, &foe);
        assert!(ember.contains("super effective"), "Fire on Grass is doubled: {ember}");
        assert!(ember.contains("damage") && ember.contains("% of its HP"),
                "a number and what share of the target it is: {ember}");

        // A status move gets no number, the `34 -> 34` rule.
        assert_eq!(fight_row_note(PokemonMoveName::Growl, &me, &foe), "",
                   "a status move is not priced");

        // The multiplier is withheld from a status move as well.
        let ghost = summary(PokemonSpecies::Gastly, [Ghost, Poison], &[PokemonMoveName::Lick]);
        assert_eq!(fight_row_note(PokemonMoveName::Growl, &me, &ghost), "",
                   "a status move is never labelled by the type chart");

        // A damaging move that genuinely cannot land says so, and says only that.
        let normal = summary(PokemonSpecies::Rattata, [Normal, Normal], &[PokemonMoveName::Tackle]);
        let row = fight_row_note(PokemonMoveName::Tackle, &normal, &ghost);
        assert_eq!(row, " — no effect", "immunity is the whole row: {row}");
    }

    #[test]
    fn the_menu_does_not_offer_a_pc_nothing_can_use() {
        use crate::pokemon::tile::MetaTile;
        let mut fixture = crate::pokemon::integration_tests::fixture::TestFixture::new(
            include_bytes!("../pokemon/data/at-celadon.bin"),
            std::time::Duration::from_secs(10),
            vec![],
        );
        let state = fixture.game_state();
        assert!(
            !overworld_menu(&state, None).iter().any(|row| row.id.ends_with(":Pc")),
            "no row offers the PC",
        );
        // The action itself is untouched, or every scripted deposit in `postgame::pc_box` breaks.
        assert_eq!(
            MetaTile::Pc.id_kind(), "Pc",
            "the id form stays, because `actions()` still yields it for the scripted policies",
        );
    }

    /// Four move names and their PP is not enough to choose between four moves.
    #[test]
    fn a_forget_row_says_what_losing_the_move_would_cost() {
        let moves = [
            PokemonMove { name: PokemonMoveName::Tackle, pp: 30 },
            PokemonMove { name: PokemonMoveName::Cut, pp: 30 },
            PokemonMove { name: PokemonMoveName::Growl, pp: 40 },
        ];
        let rows = forget_menu(&moves);
        assert!(rows[0].description.contains("Normal") && rows[0].description.contains("power"),
                "type and power: {}", rows[0].description);
        assert!(rows[1].description.contains("HM move"), "the HM is marked: {}", rows[1].description);
        assert!(!rows[0].description.contains("HM move"), "and only the HM is");
        assert!(rows[2].description.contains("no damage"),
                "a status move says so rather than showing 0 power: {}", rows[2].description);
    }

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

        // A healthy Pokémon says nothing about its status, rather than `, None`.
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

    /// Every terminal tool asks for a summary, and nothing else does.
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
                // `additionalProperties: false`, so an undeclared argument is schema-invalid.
                assert_eq!(tool.function.parameters["additionalProperties"], json!(false),
                           "`{}` would accept an undeclared argument", tool.function.name);
            }
        }
    }

    /// `call_summary` tolerates an absent summary; [`classify`] is what demands one.
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
        assert_eq!(call_summary(&call(r#"{"ticks": 5}"#)), None);
        assert_eq!(call_summary(&call(r#"{"ticks": 5, "summary": "   "}"#)), None, "blank is absent");
        assert_eq!(call_summary(&call("not json")), None, "and a broken call is not a panic");

        // `maxLength` in a schema is a request.
        let long = "x".repeat(MAX_SUMMARY * 2);
        let capped = call_summary(&call(&format!(r#"{{"summary": "{long}"}}"#))).expect("present");
        assert_eq!(capped.chars().count(), MAX_SUMMARY);
    }

    /// `press_buttons`' `why` is the headline of the record `llm::incident` writes.
    #[test]
    fn a_press_without_a_why_is_refused() {
        let schema = &press_buttons_spec().function.parameters;
        assert_eq!(schema["required"], json!(["buttons", "why"]), "the model is asked for it");

        let press = |arguments: &str| call("press_buttons", arguments);
        assert_eq!(
            call_reason(&press(r#"{"buttons":["a"], "why": "  no action opens the PC  "}"#)),
            Some("no action opens the PC".to_string()),
        );

        assert!(matches!(
            classify(DecisionKind::Stuck, &press(r#"{"buttons":["a"], "why": "a stuck text box"}"#), &[]),
            CallKind::Terminal(Terminal::PressButtons { .. }),
        ));
        for arguments in [r#"{"buttons":["a"]}"#, r#"{"buttons":["a"], "why": " "}"#] {
            let CallKind::Rejected(complaint) = classify(DecisionKind::Stuck, &press(arguments), &[]) else {
                panic!("a press with nothing to say is refused: {arguments}");
            };
            assert!(complaint.contains("why"), "{complaint}");
        }

        let long = "x".repeat(MAX_REASON * 2);
        let capped = call_reason(&press(&format!(r#"{{"why": "{long}"}}"#))).expect("present");
        assert_eq!(capped.chars().count(), MAX_REASON, "a schema's maxLength is a request");
    }

    /// The summary is the one note the model keeps about its own turn.
    #[test]
    fn a_terminal_call_must_say_what_it_is_doing() {
        for (kind, name, arguments) in [
            (DecisionKind::Overworld, "choose_action", r#"{"id":"PalletTown:5,6:Warp"}"#),
            (DecisionKind::Battle, "choose_battle_action", r#"{"id":"fight:Tackle"}"#),
            (DecisionKind::Nickname, "set_nickname", r#"{"name":"Bubbles"}"#),
            (DecisionKind::MartPurchase, "buy_item", "{}"),
            (DecisionKind::ForgetMove, "forget_move", r#"{"slot":1}"#),
            (DecisionKind::Overworld, "wait", r#"{"ticks":5}"#),
        ] {
            let CallKind::Rejected(complaint) = classify(kind, &bare(name, arguments), &[]) else {
                panic!("{name} on a {kind:?} turn must be made to say what it is doing");
            };
            assert!(complaint.contains("summary") && complaint.contains(name), "{complaint}");
            // The same call with one goes through.
            assert!(
                matches!(classify(kind, &call(name, arguments), &[]), CallKind::Terminal(_)),
                "{name} with a summary is fine",
            );
        }
    }

    /// `take_over` is off when absent and when not a boolean.
    #[test]
    fn take_over_is_off_unless_it_is_actually_asked_for() {
        let id = r#""fight:Tackle""#;
        for (arguments, expected) in [
            (format!(r#"{{"id":{id}}}"#), false),
            (format!(r#"{{"id":{id},"take_over":false}}"#), false),
            (format!(r#"{{"id":{id},"take_over":"yes"}}"#), false),
            (format!(r#"{{"id":{id},"take_over":true}}"#), true),
        ] {
            let classified = classify(DecisionKind::Battle, &call("choose_battle_action", &arguments), &[]);
            let CallKind::Terminal(Terminal::ChooseBattleAction { take_over, .. }) = classified else {
                panic!("{arguments} is a battle action");
            };
            assert_eq!(take_over, expected, "for {arguments}");
        }
    }

    /// `report_issue`, on every turn that has a menu.
    #[test]
    fn an_issue_report_does_not_end_the_turn_and_must_carry_a_message() {
        for kind in [DecisionKind::Overworld, DecisionKind::Battle, DecisionKind::Stuck] {
            assert!(offers_issue_report(kind));
            assert!(names(kind).contains(&REPORT_ISSUE), "{kind:?} offers it");
            assert!(
                !terminal_names(kind).contains(&REPORT_ISSUE),
                "{kind:?} must not let a turn end on it",
            );
            assert!(non_terminal_names(kind).contains(&REPORT_ISSUE), "the contract names it");

            let good = bare(REPORT_ISSUE, r#"{"message":"  the ladder is not in the menu  "}"#);
            let CallKind::Issue(message) = classify(kind, &good, &[]) else {
                panic!("{kind:?} should file it");
            };
            assert_eq!(message, "the ladder is not in the menu", "trimmed, like every other string");

            for arguments in ["{}", r#"{"message":"   "}"#] {
                let CallKind::Rejected(complaint) = classify(kind, &bare(REPORT_ISSUE, arguments), &[])
                else {
                    panic!("an empty report is not a report: {arguments}");
                };
                assert!(complaint.contains("message"), "{complaint}");
            }
        }

        // Single-question prompts do not carry it, and a call there is explained.
        for kind in [DecisionKind::Nickname, DecisionKind::MartPurchase, DecisionKind::ForgetMove] {
            assert!(!offers_issue_report(kind));
            assert!(!names(kind).contains(&REPORT_ISSUE), "{kind:?} does not offer it");
            let call = bare(REPORT_ISSUE, r#"{"message":"x"}"#);
            let CallKind::Rejected(complaint) = classify(kind, &call, &[]) else {
                panic!("{kind:?} does not offer it and must say so");
            };
            assert!(complaint.contains(REPORT_ISSUE), "{complaint}");
        }
    }

    #[test]
    fn terminal_tools_are_scoped_per_kind() {
        let overworld = names(DecisionKind::Overworld);
        let battle = names(DecisionKind::Battle);

        assert!(overworld.contains(&"choose_action"));
        assert!(!overworld.contains(&"choose_battle_action"));
        assert!(battle.contains(&"choose_battle_action"));
        assert!(!battle.contains(&"choose_action"));

        // The three menu prompts are single-question turns: their one terminal tool, and `wait`.
        for kind in [DecisionKind::Nickname, DecisionKind::MartPurchase, DecisionKind::ForgetMove] {
            let offered = names(kind);
            for elsewhere in
                ["choose_action", "choose_battle_action", "use_field_move", "press_buttons", REPORT_ISSUE]
            {
                assert!(!offered.contains(&elsewhere), "{kind:?} must not offer {elsewhere}");
            }
        }
        assert!(names(DecisionKind::Nickname).contains(&"set_nickname"));
        assert!(names(DecisionKind::MartPurchase).contains(&"buy_item"));
        assert!(names(DecisionKind::ForgetMove).contains(&"forget_move"));
        for kind in [DecisionKind::Overworld, DecisionKind::Battle] {
            assert!(!names(kind).contains(&"press_buttons"), "{kind:?} must not offer the hatch");
            assert!(names(kind).contains(&REPORT_ISSUE), "{kind:?} offers the replacement");
        }

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
        // The reads are all there, to work out why it is stuck before pressing anything.
        assert!(stuck.contains(&"read_map") && stuck.contains(&SCREENSHOT));
        assert!(!names(DecisionKind::Battle).contains(&"use_field_move"), "field moves are overworld-only");

        // The battle-script tools are on the overworld turn only, not even the battle turn.
        for name in BATTLE_SCRIPT_TOOL_NAMES {
            assert!(names(DecisionKind::Overworld).contains(name), "the overworld turn writes the script");
            for elsewhere in [DecisionKind::Battle, DecisionKind::Nickname, DecisionKind::MartPurchase,
                              DecisionKind::ForgetMove, DecisionKind::Stuck] {
                assert!(!names(elsewhere).contains(name), "{elsewhere:?} must not offer {name}");
            }
        }

        for kind in KINDS {
            let offered = names(kind);
            assert!(offered.contains(&"wait"), "{kind:?} must always be able to wait");
            // The contract in the prompt matches the array actually sent.
            for terminal in terminal_names(kind) {
                assert!(offered.contains(terminal), "{kind:?} promises {terminal} but does not offer it");
            }
            assert_eq!(
                offered.len(),
                reads_for(kind).count()
                    + TODO_TOOL_NAMES.len()
                    + match offers_battle_script(kind) { true => BATTLE_SCRIPT_TOOL_NAMES.len(), false => 0 }
                    + usize::from(offers_issue_report(kind))
                    + terminal_names(kind).len(),
                "a turn is offered its own reads, the TODO tools, the battle-script tools where \
                 they apply, `report_issue` where it applies, and its own terminal tools",
            );
        }
    }

    #[test]
    fn reads_are_scoped_per_kind_too() {
        assert!(!names(DecisionKind::Battle).contains(&"read_map"), "there is no map in a battle");
        assert!(!names(DecisionKind::Battle).contains(&READ_ROUTE));
        assert!(!names(DecisionKind::Overworld).contains(&"read_battle"), "it can only answer null");

        // The forget-move prompt fires mid-fight, so the battle is its business.
        assert!(names(DecisionKind::ForgetMove).contains(&"read_battle"));

        // The screen is the one read every kind keeps.
        for kind in KINDS {
            assert!(names(kind).contains(&SCREENSHOT), "{kind:?} cannot look at the screen");
        }

        // A single-question turn carries almost nothing.
        assert_eq!(names(DecisionKind::Nickname), ["read_party", SCREENSHOT, "todo_set", "todo_complete",
                                                   "todo_delete", "set_nickname", "wait"]);

        // A read that exists but is not offered here is told which turn it belongs to.
        let rejected = classify(DecisionKind::Battle, &call("read_map", "{}"), &[]);
        let CallKind::Rejected(complaint) = rejected else { panic!("read_map is not a battle read") };
        assert!(complaint.contains("not available in a battle turn"), "{complaint}");
        assert!(complaint.contains("read_battle"), "it has to name what *is* here: {complaint}");
        assert!(matches!(classify(DecisionKind::Overworld, &call("read_map", "{}"), &[]), CallKind::Read));
    }

    /// A malformed schema is a 400 on the first turn of a run.
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

    /// Every id in a `then` is held to the same menu, and an over-long chain is refused, not cut.
    #[test]
    fn every_id_in_a_chain_is_held_to_the_menu_the_turn_offered() {
        let menu = ["PalletTown:5,6:Warp".to_string(), "PalletTown:Mom".to_string()];
        let chain = |arguments: &str| classify(DecisionKind::Overworld, &call("choose_action", arguments), &menu);

        // The shape the whole feature is for, and both flags defaulted.
        let CallKind::Terminal(Terminal::ChooseAction { id, then, resume_after_battle }) = chain(
            r#"{"id":"PalletTown:5,6:Warp","then":["PalletTown:Mom"],"summary":"in, then talk"}"#,
        ) else {
            panic!("a chain of two ids from the menu is an ordinary call");
        };
        assert_eq!(id, "PalletTown:5,6:Warp");
        assert_eq!(then, ["PalletTown:Mom"]);
        assert!(resume_after_battle, "a battle does not end the action unless the model says so");

        // Each id is checked, not merely the count: a stale id is not let through by being second.
        let CallKind::Rejected(complaint) = chain(
            r#"{"id":"PalletTown:5,6:Warp","then":["OaksLab:5,11:Warp"],"summary":"a stale id"}"#,
        ) else {
            panic!("a chained id that was never offered is refused");
        };
        assert!(complaint.contains("OaksLab:5,11:Warp"), "it names the one that failed: {complaint}");

        let CallKind::Rejected(complaint) = chain(&format!(
            r#"{{"id":"PalletTown:5,6:Warp","then":[{}],"summary":"far too many"}}"#,
            ["\"PalletTown:Mom\""; MAX_CHAINED_ACTIONS].join(","),
        )) else {
            panic!("a chain longer than the cap is refused");
        };
        assert!(complaint.contains(&MAX_CHAINED_ACTIONS.to_string()), "it says what the cap is: {complaint}");

        // Not a list of strings at all.
        let CallKind::Rejected(complaint) =
            chain(r#"{"id":"PalletTown:5,6:Warp","then":"PalletTown:Mom","summary":"a bare string"}"#)
        else {
            panic!("`then` must be a list");
        };
        assert!(complaint.contains("list of further ids"), "{complaint}");

        let CallKind::Terminal(Terminal::ChooseAction { resume_after_battle, .. }) = chain(
            r#"{"id":"PalletTown:5,6:Warp","resume_after_battle":false,"summary":"stop and think"}"#,
        ) else {
            panic!("the flag is legal on its own");
        };
        assert!(!resume_after_battle, "and `false` is what turns it off");
    }

    /// A terminal call from another kind is answered with the tool that would have worked.
    #[test]
    fn a_terminal_tool_from_the_wrong_kind_is_rejected_with_the_right_one() {
        let CallKind::Rejected(complaint) =
            classify(DecisionKind::Battle, &call("choose_action", r#"{"id":"x"}"#), &[])
        else {
            panic!("choose_action must not end a battle turn");
        };
        assert!(complaint.contains("choose_battle_action"), "{complaint}");

        let CallKind::Rejected(complaint) = classify(DecisionKind::Overworld, &call("teleport", "{}"), &[]) else {
            panic!("an invented tool is rejected");
        };
        assert!(complaint.contains("teleport") && complaint.contains("read_map"), "{complaint}");
    }

    #[test]
    fn arguments_are_parsed_or_complained_about() {
        assert!(matches!(
            classify(DecisionKind::Overworld, &call("choose_action", r#"{"id":"PalletTown:5,6:Warp"}"#), &[]),
            CallKind::Terminal(Terminal::ChooseAction { ref id, .. }) if id == "PalletTown:5,6:Warp",
        ));
        assert!(matches!(
            classify(DecisionKind::Battle, &call("wait", r#"{"ticks":25}"#), &[]),
            CallKind::Terminal(Terminal::Wait { ticks: 25 }),
        ));
        // A model asking to sit out ten minutes of game time is stalling its own run.
        assert!(matches!(
            classify(DecisionKind::Battle, &call("wait", r#"{"ticks":99999}"#), &[]),
            CallKind::Terminal(Terminal::Wait { ticks: MAX_WAIT_TICKS }),
        ));
        assert!(matches!(classify(DecisionKind::Overworld, &call("wait", "{}"), &[]), CallKind::Rejected(_)));
        assert!(matches!(
            classify(DecisionKind::Overworld, &call("choose_action", r#"{"id":""}"#), &[]),
            CallKind::Rejected(_),
        ));
        assert!(matches!(
            classify(DecisionKind::Overworld, &call("choose_action", "not json at all"), &[]),
            CallKind::Rejected(_),
        ));
        // A zero-parameter read tool is routinely called with empty arguments rather than `{}`.
        assert!(matches!(classify(DecisionKind::Overworld, &call("read_map", ""), &[]), CallKind::Read));
    }

    /// A battle id survives its PP changing, which `BattleAction`'s `Display` includes.
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

    /// A model writes `"HM01 Cut"`, the code writes `Hm01Cut`, and `read_bag` writes `Hm01Cut`.
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

    /// A press queue one button short walks the player somewhere nobody asked for.
    #[test]
    fn an_id_the_turn_never_offered_is_refused_before_it_costs_the_turn() {
        let menu = [
            "ViridianPokecenter:3,7:Warp".to_string(),
            "ViridianPokecenter:Nurse".to_string(),
        ];
        let chose = |id: &str| call("choose_action", &format!(r#"{{"id":"{id}","summary":"s"}}"#));

        assert!(
            matches!(
                classify(DecisionKind::Overworld, &chose("ViridianPokecenter:3,7:Warp"), &menu),
                CallKind::Terminal(Terminal::ChooseAction { .. }),
            ),
            "an id from this turn's own menu is the decision it always was",
        );

        let CallKind::Rejected(complaint) =
            classify(DecisionKind::Overworld, &chose("ViridianCity:33,8:Warp"), &menu)
        else {
            panic!("an id for a map the player is not on can never resolve");
        };
        // The complaint has to name the right mistake.
        assert!(complaint.contains("ViridianCity") && complaint.contains("ViridianPokecenter"),
                "it must say which map the id is for and which map the player is on; got {complaint}");
        assert!(complaint.contains("ViridianPokecenter:3,7:Warp"),
                "and it must repeat what can be chosen instead; got {complaint}");

        // An empty menu checks nothing: `Nickname`, `ForgetMove` and `Stuck` have none.
        assert!(
            matches!(
                classify(DecisionKind::Overworld, &chose("anything at all"), &[]),
                CallKind::Terminal(_),
            ),
            "with no menu to check against, the id is the policy's to resolve as it always was",
        );
    }

    #[test]
    fn press_buttons_parses_a_sequence_and_refuses_what_is_not_one() {
        let press = |arguments: &str| call("press_buttons", arguments);
        let CallKind::Terminal(Terminal::PressButtons { buttons }) = classify(
            DecisionKind::Stuck,
            &press(r#"{"buttons":["b","b","start"],"why":"a menu nothing closes"}"#),
            &[],
        ) else {
            panic!("a list of real buttons is a decision");
        };
        assert_eq!(buttons, [JoypadButton::B, JoypadButton::B, JoypadButton::Start]);

        for bad in [r#"{"buttons":["b","x"],"why":"w"}"#, r#"{"buttons":[],"why":"w"}"#, r#"{"why":"w"}"#] {
            assert!(
                matches!(classify(DecisionKind::Stuck, &press(bad), &[]), CallKind::Rejected(_)),
                "{bad} should have been rejected",
            );
        }
        // The agent's queue capacity is the cap, so a runaway list is trimmed, not half-delivered.
        let many = format!(r#"{{"buttons":{},"why":"a menu nothing closes"}}"#,
            serde_json::to_string(&vec!["a"; MANUAL_INPUT_CAPACITY * 2]).unwrap());
        let CallKind::Terminal(Terminal::PressButtons { buttons }) =
            classify(DecisionKind::Stuck, &press(&many), &[])
        else {
            panic!("an over-long list is still a decision");
        };
        assert_eq!(buttons.len(), MANUAL_INPUT_CAPACITY);
    }

    /// `screenshot` is a read to the turn contract, but the worker answers it.
    #[test]
    fn a_screenshot_is_classified_apart_from_the_other_reads() {
        assert!(matches!(classify(DecisionKind::Battle, &call(SCREENSHOT, "{}"), &[]), CallKind::Screenshot));
        assert!(matches!(classify(DecisionKind::Battle, &call("read_party", "{}"), &[]), CallKind::Read));
        assert!(READ_TOOLS.iter().any(|tool| tool.name == SCREENSHOT), "it is still offered as a read");
    }

    /// Every argument shape `use_field_move` accepts, and the complaint each malformed one earns.
    #[test]
    fn a_field_move_call_parses_into_the_move_it_names() {
        let parse = |arguments: &str| classify(DecisionKind::Overworld, &call("use_field_move", arguments), &[]);
        let request = |arguments: &str| match parse(arguments) {
            CallKind::Terminal(Terminal::UseFieldMove(request)) => request,
            CallKind::Rejected(complaint) => panic!("{arguments} was rejected: {complaint}"),
            _ => panic!("{arguments} did not end the turn"),
        };

        assert_eq!(
            request(r#"{"move":"flash"}"#),
            FieldMoveRequest::PartyMove { name: PokemonMoveName::Flash, slot: None },
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
            FieldMoveRequest::UseItem { item: ItemId::PokeFlute, target: Some(Point8 { x: 12, y: 9 }), slot: None },
        );
        assert_eq!(
            request(r#"{"move":"use_item","item":"Bicycle"}"#),
            FieldMoveRequest::UseItem { item: ItemId::Bicycle, target: None, slot: None },
        );
        assert_eq!(
            request(r#"{"move":"use_item","item":"Potion","slot":1}"#),
            FieldMoveRequest::UseItem { item: ItemId::Potion, target: None, slot: Some(1) },
        );
        assert_eq!(request(r#"{"move":"reorder_party","slot":3}"#), FieldMoveRequest::ReorderParty { slot: 3 });

        // Each is a tool result the model can correct in the same turn.
        for (arguments, expected) in [
            (r#"{"move":"teleportation"}"#, "not one of the field moves"),
            (r#"{"move":"fly","map":"Atlantis"}"#, "is not a map"),
            (r#"{"move":"teach","item":"Hm03Surf"}"#, "needs a `slot`"),
            (r#"{"move":"toss_item","item":"Sandwich"}"#, "is not an item"),
            // A `target` that is present is still checked.
            (r#"{"move":"use_item","item":"PokeFlute","target":{"x":"here"}}"#, "must be a tile coordinate"),
            // `cut`, `push_boulder` and `strength` are menu rows, not verbs.
            (r#"{"move":"cut"}"#, "not one of the field moves"),
            (r#"{"move":"push_boulder","target":{"x":1,"y":1},"direction":"left"}"#, "not one of the field moves"),
            (r#"{"move":"strength"}"#, "not one of the field moves"),
            (r#"{"move":"reorder_party","slot":9}"#, "no party slot 9"),
            (r#"{"move":"interact","target":{"x":1,"y":2}}"#, "not one of the field moves"),
            ("{}", "`move` is required"),
        ] {
            let CallKind::Rejected(complaint) = parse(arguments) else {
                panic!("{arguments} should have been rejected");
            };
            assert!(complaint.contains(expected), "{arguments} → {complaint}");
        }
    }

    /// In the menu-prompt tools, omitting the argument is a real answer, not a malformed call.
    #[test]
    fn omitting_the_argument_is_an_answer_for_the_three_menu_prompts() {
        let parse = |kind, name, arguments: &str| classify(kind, &call(name, arguments), &[]);

        assert!(matches!(
            parse(DecisionKind::Nickname, "set_nickname", "{}"),
            CallKind::Terminal(Terminal::SetNickname { name: None }),
        ));
        // A blank `name` and an absent one both keep the default, as the naming screen does.
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
            CallKind::Terminal(Terminal::BuyItem { item: None, .. }),
        ));
        assert_eq!(
            match parse(DecisionKind::MartPurchase, "buy_item", r#"{"item":"Potion","quantity":4}"#) {
                CallKind::Terminal(Terminal::BuyItem { item, .. }) => item,
                _ => panic!("a stocked item is a purchase"),
            },
            Some(BagItem::new(ItemId::Potion, 4)),
        );
        // An omitted quantity is one: the mart silently refuses zero.
        assert!(matches!(
            parse(DecisionKind::MartPurchase, "buy_item", r#"{"item":"Potion"}"#),
            CallKind::Terminal(Terminal::BuyItem { item: Some(BagItem { quantity: 1, .. }), .. }),
        ));

        // A chain is parsed whole before any of it is spent.
        assert_eq!(
            match parse(DecisionKind::MartPurchase, "buy_item",
                        r#"{"item":"Potion","then":[{"item":"PokeBall","quantity":10}]}"#) {
                CallKind::Terminal(Terminal::BuyItem { then, .. }) => then,
                _ => panic!("a chained order is a purchase"),
            },
            vec![BagItem::new(ItemId::PokeBall, 10)],
        );
        assert!(matches!(
            parse(DecisionKind::MartPurchase, "buy_item",
                  r#"{"item":"Potion","then":[{"item":"Nonsense"}]}"#),
            CallKind::Rejected(_),
        ), "a name in the tail is checked exactly as the head is");
        assert!(matches!(
            parse(DecisionKind::MartPurchase, "buy_item",
                  r#"{"item":"Potion","then":[{"item":"Potion"},{"item":"Potion"},{"item":"Potion"},{"item":"Potion"}]}"#),
            CallKind::Rejected(_),
        ), "over-length is a rejection, not a truncation");

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

    /// Resolution against a real game, where the party and bag checks live.
    #[test]
    fn a_field_move_is_resolved_against_the_party_and_the_bag_it_needs() {
        // Oak's lab: one Pokémon, no HMs, no trees.
        let state = fixture_state();
        let complaint = |request| match resolve_field_move(&state, &request) {
            Err(complaint) => complaint,
            Ok(resolved) => panic!("{request:?} should not have resolved to {resolved:?}"),
        };
        assert!(complaint(FieldMoveRequest::ReorderParty { slot: 3 }).contains("no party member in slot 3"));
        assert!(complaint(FieldMoveRequest::TossItem { item: ItemId::Hm01Cut }).contains("no Hm01Cut in the bag"));

        // The one that needs nothing but its own HM resolves as itself.
        let mut flier = fixture_state();
        flier.badges |= crate::pokemon::badge::Badge::ThunderBadge;
        flier.pokemon.get_mut(0).expect("the fixture has a starter").moves[1] =
            Some(PokemonMove::with_max_pp(PokemonMoveName::Fly));
        assert_eq!(
            resolve_field_move(&flier, &FieldMoveRequest::Fly { to: Map::PalletTown }),
            Ok(FieldMove::Fly { to: Map::PalletTown }),
        );
        assert_eq!(
            resolve_field_move(&state, &FieldMoveRequest::ReorderParty { slot: 0 }),
            Ok(FieldMove::ReorderParty { slot: 0 }),
        );
    }

    /// The machine gate: a teach the party cannot learn is refused.
    #[test]
    fn a_machine_no_one_in_the_party_can_learn_is_refused_here_instead() {
        use crate::pokemon::pokemon::Pokemon;
        use crate::pokemon::species::PokemonSpecies;
        let with_hm = |item: ItemId, party: &[PokemonSpecies]| {
            let mut state = fixture_state();
            state.bag.push(BagItem { id: item, quantity: 1 }).expect("the fixture's bag has room");
            state.pokemon = Default::default();
            for species in party {
                state.pokemon.push(Pokemon::maxed(*species, "MON", [PokemonMoveName::Tackle; 4], "AI", 1))
                    .expect("six is the limit and these are fewer");
            }
            state
        };
        let complaint = |state: &GameState, request| match resolve_field_move(state, &request) {
            Err(complaint) => complaint,
            Ok(resolved) => panic!("{request:?} should not have resolved to {resolved:?}"),
        };

        // Somebody can: the answer names the slot.
        let mixed = with_hm(ItemId::Hm01Cut, &[PokemonSpecies::Venusaur, PokemonSpecies::Pidgey]);
        let wrong_slot = complaint(&mixed, FieldMoveRequest::Teach { item: ItemId::Hm01Cut, slot: 1 });
        assert!(wrong_slot.contains("cannot learn Cut"), "{wrong_slot}");
        assert!(wrong_slot.contains("slot 0"), "it has to name who can: {wrong_slot}");

        // The slot that can resolves.
        assert_eq!(
            resolve_field_move(&mixed, &FieldMoveRequest::Teach { item: ItemId::Hm01Cut, slot: 0 }),
            Ok(FieldMove::TeachMove { item: ItemId::Hm01Cut, target_slot: 0 }),
        );

        let none = with_hm(ItemId::Hm01Cut, &[PokemonSpecies::Pidgey, PokemonSpecies::Zubat]);
        let hopeless = complaint(&none, FieldMoveRequest::Teach { item: ItemId::Hm01Cut, slot: 0 });
        assert!(hopeless.contains("nor can anything else in the party"), "{hopeless}");
        assert!(!hopeless.contains("In the party,"), "there is nobody to name: {hopeless}");

        // A stone rides the same menu chain, and the machine check must not refuse it.
        let stone = with_hm(ItemId::WaterStone, &[PokemonSpecies::Eevee]);
        assert_eq!(
            resolve_field_move(&stone, &FieldMoveRequest::Evolve { stone: ItemId::WaterStone, slot: 0 }),
            Ok(FieldMove::EvolveWithStone { stone: ItemId::WaterStone, target_slot: 0,
                                            evolve_from: PokemonSpecies::Eevee }),
        );
    }

    /// The item gate: an item the game would not use is refused before a press.
    #[test]
    fn an_item_the_game_will_never_use_is_refused_here_instead() {
        let holding = |item: ItemId| {
            let mut state = fixture_state();
            state.bag.push(BagItem { id: item, quantity: 1 }).expect("the fixture's bag has room");
            state
        };
        let at = Point8 { x: 8, y: 4 };
        let complaint = |state: &GameState, item: ItemId| {
            match resolve_field_move(state, &FieldMoveRequest::UseItem { item, target: Some(at), slot: None }) {
                Err(complaint) => complaint,
                Ok(resolved) => panic!("{item} should not have resolved to {resolved:?}"),
            }
        };

        let fossil = complaint(&holding(ItemId::HelixFossil), ItemId::HelixFossil);
        assert!(fossil.contains("no bag use for HelixFossil"), "{fossil}");
        assert!(fossil.contains("carry"), "it has to say what to do instead: {fossil}");
        assert!(complaint(&holding(ItemId::SilphScope), ItemId::SilphScope).contains("no bag use"));

        // A ball is the same `ItemUseNotTime`, and the refusal names the alternative.
        let ball = complaint(&holding(ItemId::PokeBall), ItemId::PokeBall);
        assert!(ball.contains("choose_battle_action"), "{ball}");

        // A machine never reaches the table (`ItemUseTMHM`), so it is pointed at `teach`.
        let machine = complaint(&holding(ItemId::Hm01Cut), ItemId::Hm01Cut);
        assert!(machine.contains("teach"), "{machine}");

        // Not "refuse every key item": the Poké Flute is one, and the scripted route uses it.
        assert!(ItemId::PokeFlute.is_key_item(), "the point of the case");
        assert_eq!(
            resolve_field_move(&holding(ItemId::PokeFlute), &FieldMoveRequest::UseItem { item: ItemId::PokeFlute, target: Some(at), slot: None }),
            Ok(FieldMove::UseFieldItem { item: ItemId::PokeFlute, target: at }),
        );

        // `held` runs first, so "you do not have one" beats "it would not work".
        let empty = complaint(&fixture_state(), ItemId::HelixFossil);
        assert!(empty.contains("no HelixFossil in the bag"), "{empty}");
    }

    /// Which side of the map a repeated door is on.
    #[test]
    fn a_repeated_door_is_named_by_the_side_of_the_map_it_is_on() {
        let mut map = state_from(include_bytes!("../pokemon/data/split-cerulean.bin")).map;
        map.width = 6;
        map.height = 8;
        assert_eq!(door_side(&map, Point8 { x: 0, y: 3 }), Some("west"));
        assert_eq!(door_side(&map, Point8 { x: 5, y: 4 }), Some("east"));
        // Level on x, so the y axis decides.
        map.width = 8;
        assert_eq!(door_side(&map, Point8 { x: 3, y: 0 }), Some("north"));
        assert_eq!(door_side(&map, Point8 { x: 4, y: 7 }), Some("south"));
        // Dead centre on the axis that would have decided: no side to name.
        map.width = 7;
        map.height = 7;
        assert_eq!(door_side(&map, Point8 { x: 3, y: 3 }), None);
    }

    /// The Bicycle, and the half of the item table no turn could reach.
    #[test]
    fn a_bag_item_with_nothing_to_aim_at_is_a_call_that_can_be_made() {
        use crate::pokemon::policy::FieldMove;
        use crate::pokemon::postgame::items::UseTarget;

        // Route 11: outdoors, in `BikeRidingTilesets`, with the Bicycle in the bag.
        let outdoors = {
            let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
            gb.load_state(include_bytes!("../pokemon/data/route-11-youngster-on-the-pacing-tile.bin"))
                .expect("the committed fixture loads");
            { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
                .expect("a readable state")
        };
        assert!(outdoors.bag.iter().any(|item| item.id == ItemId::Bicycle),
            "the fixture has to be carrying the bike or this proves nothing");

        let ride = FieldMoveRequest::UseItem { item: ItemId::Bicycle, target: None, slot: None };
        assert_eq!(resolve_field_move(&outdoors, &ride).expect("a bike outdoors is a legal call"),
                   FieldMove::UseBagItem { item: ItemId::Bicycle, target: UseTarget::Nothing });

        // Oak's lab: indoors, so `IsBikeRidingAllowed` refuses, said before a button is pressed.
        let mut indoors = fixture_state();
        indoors.bag.push(BagItem { id: ItemId::Bicycle, quantity: 1 }).expect("room in the bag");
        let refusal = resolve_field_move(&indoors, &ride).expect_err("a bike indoors is refused");
        assert!(refusal.contains("cycling is not allowed"), "{refusal}");
        assert!(refusal.contains("OaksLab"), "it names where it is refusing: {refusal}");
    }

    /// `ItemUseBicycle` turns water down before it asks whether the map allows cycling.
    #[test]
    fn the_bicycle_is_refused_while_surfing() {
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/route21-islands.bin")).expect("the committed fixture loads");
        let state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("a readable state");
        assert!(state.map.surfing, "the fixture has to be on the water or this proves nothing");
        assert!(state.bag.iter().any(|item| item.id == ItemId::Bicycle), "and carrying the bike");
        assert!(crate::pokemon::postgame::items::bike_riding_allowed(&state), "on a map that allows it");

        let ride = FieldMoveRequest::UseItem { item: ItemId::Bicycle, target: None, slot: None };
        let refusal = resolve_field_move(&state, &ride).expect_err("a bike on water is refused");
        assert!(refusal.contains("surfing"), "{refusal}");
    }

    /// A field item aimed at nothing is refused, and told what is beside it.
    #[test]
    fn a_field_item_aimed_at_open_ground_is_refused_and_says_what_is_beside_it() {
        let mut state = fixture_state();
        state.bag.push(BagItem { id: ItemId::PokeFlute, quantity: 1 }).expect("room in the bag");
        let flute = |target| resolve_field_move(&state, &FieldMoveRequest::UseItem {
            item: ItemId::PokeFlute, target: Some(target), slot: None });

        // Oak's lab: the rival stands on (8, 4) and (7, 5) is bare floor beneath the player.
        let empty = flute(Point8 { x: 7, y: 5 }).expect_err("open ground is not a target");
        assert!(empty.contains("nothing at (7, 5)"), "{empty}");
        assert!(empty.contains("open ground"), "it says what the square is: {empty}");
        // Which coordinate convention is which.
        assert!(empty.contains("square the *thing* is standing on"), "{empty}");
        assert!(empty.contains("where the player stands"), "{empty}");
        // And what is actually next door.
        let beside = flute(Point8 { x: 7, y: 4 }).expect_err("the player's own square is not one either");
        assert!(beside.contains("Rival, at (8, 4)"), "it names what is beside the miss: {beside}");

        // A square that is not on the map at all is its own sentence rather than a silent accept.
        let off = flute(Point8 { x: 40, y: 40 }).expect_err("off the map");
        assert!(off.contains("not a square on"), "{off}");

        // No em dashes: a refusal is shown on the page as well as sent to the model.
        for refusal in [&empty, &beside, &off] {
            assert!(!refusal.contains('—'), "no em dashes in what the agent writes: {refusal}");
        }

        // And the thing itself still resolves.
        assert_eq!(
            flute(Point8 { x: 8, y: 4 }),
            Ok(FieldMove::UseFieldItem { item: ItemId::PokeFlute, target: Point8 { x: 8, y: 4 } }),
        );
    }

    /// The HM gate names which half is missing.
    #[test]
    fn an_hm_the_game_would_refuse_is_refused_here_instead() {
        let none = fixture_state();
        let complaint = |state: &GameState, request| match resolve_field_move(state, &request) {
            Err(complaint) => complaint,
            Ok(resolved) => panic!("{request:?} should not have resolved to {resolved:?}"),
        };

        // Every HM this tool can still name.
        for (name, badge) in HM_BADGES {
            let request = match name {
                PokemonMoveName::Cut | PokemonMoveName::Strength | PokemonMoveName::Surf => continue,
                PokemonMoveName::Fly => FieldMoveRequest::Fly { to: Map::PalletTown },
                other => FieldMoveRequest::PartyMove { name: *other, slot: None },
            };
            let complaint = complaint(&none, request);
            assert!(complaint.contains(&badge.to_string()), "{name} must name the {badge} it needs");
            assert!(complaint.contains("no Pokémon in the party knows it"), "{complaint}");
        }

        // The move but not the badge: the gym is the thing to go and do.
        let mut taught = fixture_state();
        taught.pokemon.get_mut(0).expect("the fixture has a starter").moves[1] =
            Some(PokemonMove::with_max_pp(PokemonMoveName::Flash));
        let unbadged = complaint(&taught, FieldMoveRequest::PartyMove {
            name: PokemonMoveName::Flash, slot: None });
        assert!(unbadged.contains("BoulderBadge"), "{unbadged}");
        assert!(!unbadged.contains("knows"), "the move is known; only the badge is missing: {unbadged}");

        // The badge but not the move: the HM is the thing to go and find.
        let mut badged = fixture_state();
        badged.badges |= crate::pokemon::badge::Badge::BoulderBadge;
        let untaught = complaint(&badged, FieldMoveRequest::PartyMove {
            name: PokemonMoveName::Flash, slot: None });
        assert!(untaught.contains("HM"), "{untaught}");
        assert!(!untaught.contains("BoulderBadge"), "the badge is held: {untaught}");
    }

    /// A cut or push the game would refuse is a row that is not there.
    #[test]
    fn a_cut_or_a_push_the_game_would_refuse_is_never_a_row() {
        use crate::pokemon::tile::MetaTile;
        let stuck = state_from(include_bytes!("../pokemon/data/vr1f-stuck-push.bin"));
        assert!(stuck.map.can_strength, "the deployed party can use Strength");
        assert!(stuck.map.boulders().contains(&Point8 { x: 5, y: 14 }), "the sealed boulder is there");

        for action in stuck.map.actions() {
            let MetaTile::BoulderGoal { boulder, at, .. } = action.tile else { continue };
            let plan = stuck.map.solve_boulder_push_for(boulder, at)
                .expect("a row is only minted for a goal that solves");
            let (first, push) = plan[0];
            assert_eq!(stuck.map.boulder_push_refusal(first, push), None,
                "the menu offered a goal opening on a push the cartridge would refuse: {}",
                overworld_id(&stuck, &action));
            assert_ne!(boulder, Point8 { x: 5, y: 14 },
                "the sealed boulder cannot be pushed any way at all, so no goal names it");
        }
        // On this floor that leaves no rows at all.
        assert!(!stuck.map.actions().iter().any(|a| matches!(a.tile, MetaTile::BoulderGoal { .. })),
            "the run sealed its own boulder, so nothing can reach the switch and nothing is offered");

        // The same floor before it was wedged is solvable, so it has rows.
        let pristine = state_from(include_bytes!("../pokemon/data/vr1f-strength.bin"));
        assert!(pristine.map.actions().iter().any(|a| matches!(a.tile, MetaTile::BoulderGoal { .. })),
            "VictoryRoad1F's switch is a goal row from its starting layout");

        // A boulder is not somebody to talk to.
        let menu = overworld_menu(&pristine, None);
        assert!(menu.iter().all(|item| !item.id.contains("Boulder") || item.id.contains("PushBoulder")),
            "a boulder's own sprite row is withheld: {menu:?}");
        assert!(menu.iter().any(|item| item.id.contains("PushBoulder")), "{menu:?}");

        // Without the move or the badge there is nothing to choose.
        let mut unarmed = pristine;
        unarmed.map.can_strength = false;
        assert!(!unarmed.map.actions().iter().any(|a| matches!(a.tile, MetaTile::BoulderGoal { .. })),
            "no Strength, no boulder rows");
    }

    /// The naming screen asks for a name, and takes only names the cartridge can write.
    #[test]
    fn a_nickname_is_asked_for_and_has_to_be_one_the_game_can_write() {
        let spec = for_kind(DecisionKind::Nickname);
        let described = spec.iter().find(|tool| tool.function.name == "set_nickname")
            .map(|tool| tool.function.description.clone())
            .expect("the naming turn offers set_nickname");
        assert!(described.contains("nickname"), "{described}");
        assert!(!described.contains("ordinary answer"),
                "the tool must not talk the model out of the one thing this turn is for: {described}");

        let parse = |arguments: &str| classify(DecisionKind::Nickname, &call("set_nickname", arguments), &[]);
        let named = |name: &str| parse(&json!({"name": name}).to_string());
        assert!(matches!(named("Rocky"), CallKind::Terminal(Terminal::SetNickname { name: Some(_) })));
        // `/` is `$F3` and a space `$7F`, so the check asks the charmap, not "is it alphanumeric".
        assert!(matches!(named("MT/MOON"), CallKind::Terminal(Terminal::SetNickname { name: Some(_) })));
        assert!(
            matches!(named("Poké"), CallKind::Rejected(_)),
            "an accented letter has no byte in this charmap and must not reach the buffer",
        );
        assert!(matches!(named("🔥"), CallKind::Rejected(_)));
        // An omitted or blank name is the decline, as the naming screen reads an empty buffer.
        assert!(matches!(
            parse("{}"),
            CallKind::Terminal(Terminal::SetNickname { name: None }),
        ));
        assert!(matches!(named("   "), CallKind::Terminal(Terminal::SetNickname { name: None })));
    }

    /// A tree nobody can cut is not an action.
    #[test]
    fn a_cut_tree_is_not_offered_to_a_party_that_cannot_cut() {
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/at-vermilion.bin"))
            .expect("the committed fixture loads");
        let mut state = { use crate::pokemon::PokemonApiTrait; crate::pokemon::PokemonApi::new(&mut gb).game_state() }
            .expect("the fixture has a readable state");
        assert!(
            state.map.meta_tiles.contains(&crate::pokemon::tile::MetaTile::CutTree),
            "this fixture's map has to have a tree on it for the test to mean anything",
        );

        let cut_rows = |state: &GameState| -> usize {
            overworld_menu(state, None).iter().filter(|item| item.id.ends_with(":CutTree")).count()
        };
        assert!(!state.can_use_cut, "the fixture reaches Vermilion before the HM");
        assert_eq!(cut_rows(&state), 0, "a tree is not an action without Cut");

        state.map.can_cut = true;
        assert!(cut_rows(&state) > 0, "and it is one with Cut — or this test would pass by accident");

        // Every row names its own tree, and the id still ends in `CutTree`.
        let trees: std::collections::BTreeSet<String> = overworld_menu(&state, None).into_iter()
            .filter(|item| item.id.ends_with(":CutTree"))
            .map(|item| item.description)
            .collect();
        assert_eq!(trees.len(), cut_rows(&state), "two rows must not describe the same tree: {trees:?}");
        for description in &trees {
            assert!(description.starts_with("cut down the tree at ("), "{description}");
        }
    }

    /// A field move's index in the party menu depends on what else that mon knows.
    #[test]
    fn a_party_field_moves_index_is_computed_from_the_moves_it_knows() {
        let mut state = fixture_state();
        // The HM gate comes before the index, so Flash has to be usable first.
        state.badges |= crate::pokemon::badge::Badge::BoulderBadge;
        state.pokemon.get_mut(0).expect("the fixture has a starter").moves = [
            Some(PokemonMove::with_max_pp(PokemonMoveName::Tackle)),
            Some(PokemonMove::with_max_pp(PokemonMoveName::Cut)),
            Some(PokemonMove::with_max_pp(PokemonMoveName::Flash)),
            None,
        ];

        assert_eq!(
            resolve_field_move(&state, &FieldMoveRequest::PartyMove {
                name: PokemonMoveName::Flash,
                slot: None,
            }),
            // Cut sits in an earlier move slot, so Flash is the second field-move row.
            Ok(FieldMove::UseFieldMove { slot: 0, move_index: 1 }),
        );
    }
}

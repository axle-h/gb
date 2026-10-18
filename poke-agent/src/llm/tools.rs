use serde_json::{Value, json};

use poke_core::geometry::Point8;
use gb::joypad::JoypadButton;
use crate::llm::prompt::ApiSnapshot;
use crate::llm::files::{self, FileCall, FileRead, Node};
use crate::llm::protocol::{ToolCall, ToolSpec};
use crate::pokemon::GameState;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::agent::MANUAL_INPUT_CAPACITY;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
#[cfg(test)]
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::policy::{FieldMove, battle_options, field_move_index};
#[cfg(test)]
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
    /// The player's Mimic is asking which of the enemy's moves to copy.
    MimicMove,
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
            Self::MimicMove => "mimic",
            Self::Stuck => "stuck",
        }
    }

    /// Whether only the last poll site, not the `GameState`, says this is the question; see
    /// `LlmPolicy::observed_kind`.
    pub fn is_inferred_from_the_site(self) -> bool {
        matches!(self, Self::Nickname | Self::MartPurchase | Self::ForgetMove | Self::MimicMove | Self::Stuck)
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
    },
    /// One battle action, and whether the rest of this battle belongs to the model.
    ChooseBattleAction { id: String, take_over: bool },
    PressButtons { buttons: Vec<JoypadButton> },
    SetNickname { name: Option<String> },
    BuyItem {
        item: Option<BagItem>,
        /// More kinds to buy in the same visit, in order.
        then: Vec<BagItem>,
    },
    ForgetMove { slot: Option<u8> },
    /// The row of the enemy's move Mimic copies.
    MimicMove { slot: u8 },
    /// Do nothing for this many agent ticks (20 ms of emulated time each).
    Wait { ticks: u16 },
}

/// How many `choose_action` calls one overworld or mart reply may carry.
pub const MAX_CHAINED_ACTIONS: usize = 4;

/// A longer `wait` is a model stalling its own run.
pub const MAX_WAIT_TICKS: u16 = 150;

/// What a bare `wait` waits.
pub const DEFAULT_WAIT_TICKS: u16 = 50;

/// The most of one item a mart order may ask for.
pub const MAX_ORDER: u32 = 99;

/// The action tool, the one tool that ends a turn.
pub const CHOOSE_ACTION: &str = "choose_action";

/// The longest nickname the naming screen's buffer holds.
pub const MAX_NICKNAME: usize = 10;

/// What one call in an assistant message turned out to be.
pub enum CallKind {
    /// A read the policy answers from the game at its next poll.
    Read(FileRead),
    /// `screen.png`, answered by the worker from the latest frame.
    Screenshot,
    /// A read the worker answers, or a write or an edit.
    File(FileCall),
    /// A write under `issues/`: the model believes the agent is wrong.
    Issue { name: String, message: String },
    /// The turn is over.
    Terminal(Terminal),
    /// A `choose_action` folded into the [`CallKind::Terminal`] earlier in the same reply.
    Chained,
    /// An unknown name, a terminal tool of another decision kind, or unparseable arguments.
    Rejected(String),
}

impl CallKind {
    /// The discriminant, for the page: these words are a wire contract with `api.ts`.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Read(_) | Self::Screenshot | Self::File(FileCall::Read(_)) => "read",
            // Not a fifth word: `api.ts` matches on these.
            Self::File(_) => "todo",
            Self::Issue { .. } => "issue",
            Self::Terminal(_) | Self::Chained => "terminal",
            Self::Rejected(_) => "rejected",
        }
    }
}

// ── Field moves ──────────────────────────────────────────────────────────────────────────────────

/// An overworld row that is not a walk, parsed from its id by [`field_request`];
/// [`resolve_field_move`] resolves it against the live state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldMoveRequest {
    PartyMove { name: PokemonMoveName, slot: Option<u8> },
    Fly { to: Map },
    /// Teach an HM or TM in the bag to a party member.
    Teach { item: ItemId, slot: u8 },
    /// Use an evolution stone from the bag on a party member.
    Evolve { stone: ItemId, slot: u8 },
    /// Use a bag item on `target` if given (the Poké Flute on Snorlax), else the player or `slot`.
    UseItem { item: ItemId, target: Option<Point8>, slot: Option<u8>, evolve: bool },
    /// Throw `count` of an item away; all of it frees one of the bag's 20 slots.
    TossItem { item: ItemId, count: u8 },
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

/// The PC a model can use on `map`, and whether it stores Pokémon as well as items. Bill's is his
/// cell separator (`BillsHousePC`), and the bedroom's is the player's own, items only (`OpenRedsPC`).
pub fn usable_pc(map: crate::pokemon::map::Map) -> Option<(Point8, bool)> {
    use crate::pokemon::map::Map;
    match map {
        Map::BillsHouse => None,
        _ => crate::pokemon::tile_map::pc_locations_for(map).first().map(|&at| (at, map != Map::RedsHouse2F)),
    }
}

/// The PC on this map, or why there is none.
fn the_pc_here(state: &GameState, for_pokemon: bool) -> Result<Point8, String> {
    match usable_pc(state.map.map) {
        Some((_, false)) if for_pokemon => Err(format!(
            "The PC on {} stores items only. Pokémon are stored at a Pokémon Centre's PC.",
            state.map.map,
        )),
        Some((at, _)) => Ok(at),
        None => Err(format!(
            "There is no PC on {}. Every Pokémon Centre has one, and so does the player's bedroom, \
             for items only.",
            state.map.map,
        )),
    }
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
            false => Err(format!("There is no {item} in the bag. `bag.md` lists what is there.")),
        }
    };

    Ok(match request {
        FieldMoveRequest::PartyMove { name, slot } => {
            hm_available(state, *name)?;
            match name {
                PokemonMoveName::Dig if !escape_allowed(state) => return Err(format!(
                    "Dig only gets out of somewhere the game counts as a dungeon: a cave, a forest, \
                     a building like the Pokémon Tower or Silph Co. On {} it answers \"This isn't \
                     the time to use that!\"", state.map.map)),
                PokemonMoveName::Teleport if !outside(state) => return Err(format!(
                    "Teleport only works outside, on a town or a route, and {} is not one.",
                    state.map.map)),
                _ => {}
            }
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
            if !outside(state) {
                return Err(format!(
                    "Fly only takes off outside, from a town or a route, and {} is not one. Walk out \
                     first.", state.map.map));
            }
            if !flies_to(state, *to) {
                return Err(format!(
                    "{to} is not on Fly's map: only towns you have set foot in are, and you have not \
                     been to that one, or it is not a town."));
            }
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
            // Refused by the game with nothing used up, which a teach driver would retry for ever.
            if let Some(taught) = crate::pokemon::learnset::machine_move(item)
                && knows(state, slot, taught) {
                return Err(format!("The Pokémon in slot {slot} already knows {taught}, and the game \
                                    will not teach a move twice."));
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
            let stone = held(*stone)?;
            if !stone_evolves(state, slot, stone) {
                return Err(format!(
                    "The {stone:?} does nothing for the {evolve_from} in slot {slot}: the game says \
                     \"It won't have any effect.\" A stone evolves only the species it is made for."));
            }
            FieldMove::EvolveWithStone { stone, target_slot: slot, evolve_from }
        }
        FieldMoveRequest::UseItem { item, target: None, slot, evolve } => {
            let item = held(*item)?;
            if let Some(refusal) = crate::pokemon::item_use::field_use_refusal(item) {
                return Err(refusal);
            }
            if crate::pokemon::item_use::battle_only(item) {
                return Err(format!("The {item:?} only does anything in a battle; out here the game \
                                    says \"This isn't the time to use that!\""));
            }
            if item == ItemId::EscapeRope && !escape_allowed(state) {
                return Err(format!(
                    "An Escape Rope only gets out of somewhere the game counts as a dungeon: a cave, \
                     a forest, a building like the Pokémon Tower or Silph Co. On {} it answers \
                     \"This isn't the time to use that!\"", state.map.map));
            }
            let target = match slot {
                Some(slot) => crate::pokemon::postgame::items::UseTarget::Party {
                    slot: party_slot(*slot)?,
                    evolve: *evolve,
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
        FieldMoveRequest::UseItem { item, target: Some(target), .. } => {
            // As `Teach`'s gate, for an item the game will not use at all.
            let item = held(*item)?;
            if let Some(refusal) = crate::pokemon::item_use::field_use_refusal(item) {
                return Err(refusal);
            }
            let at = *target;
            let nothing_there = |noun: &str| -> String {
                // Whatever is beside the square it aimed at, saving a read of `map.png`.
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
                            " What is next to it is {name}, at ({}, {}). `map.png` gives the exact \
                                position of everything on this map.", next.x, next.y),
                        None => " `map.png` gives the exact position of everything on this map."
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
        FieldMoveRequest::TossItem { item, count } => {
            let item = held(*item)?;
            if poke_core::item::is_key_item(item) {
                return Err(format!("The {item:?} cannot be thrown away: the game says \"That's too \
                                    important to toss!\" of every key item and HM."));
            }
            match *count >= crate::pokemon::postgame::items::bag_quantity(state, item) {
                true => FieldMove::TossItem { item },
                false => FieldMove::TossSome { item, count: (*count).max(1) },
            }
        }
        FieldMoveRequest::ReorderParty { slot } => FieldMove::ReorderParty { slot: party_slot(*slot)? },
        FieldMoveRequest::UsePcBox { op } => {
            let pc = the_pc_here(state, true)?;
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
            let pc = the_pc_here(state, false)?;
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

/// `CheckIfInOutsideMap`: the tilesets Fly and Teleport are allowed from.
fn outside(state: &GameState) -> bool {
    use crate::pokemon::map_header::TileSetId;
    matches!(state.map.tileset, TileSetId::Overworld | TileSetId::Plateau)
}

/// `ItemUseEscapeRope`, which Dig borrows: `EscapeRopeTilesets`, and never Agatha's room.
fn escape_allowed(state: &GameState) -> bool {
    state.map.map != Map::AgathasRoom
        && poke_core::tables::ESCAPE_ROPE_TILESETS.contains(&(state.map.tileset as u8))
}

/// Whether `to` is a town on Fly's map: `wTownVisitedFlag` has a bit per town id.
fn flies_to(state: &GameState, to: Map) -> bool {
    let id = to as u8;
    id < crate::pokemon::postgame::fly_bike::FLY_DESTINATIONS && state.towns_visited & (1 << id) != 0
}

/// Whether `stone` evolves the party member in `slot`, per `EvosMovesPointerTable`.
fn stone_evolves(state: &GameState, slot: u8, stone: ItemId) -> bool {
    use poke_core::evos_moves::{EvosMoves, Evolution};
    state.pokemon.get(slot as usize).is_some_and(|mon| EvosMoves::of(mon.species).evolutions.iter()
        .any(|evolution| matches!(*evolution, Evolution::Item { item, min_level, .. }
            if item == stone && mon.level >= min_level)))
}

fn knows(state: &GameState, slot: u8, name: PokemonMoveName) -> bool {
    state
        .pokemon
        .get(slot as usize)
        .is_some_and(|mon| mon.moves.iter().flatten().any(|m| m.name == name))
}

// ── Field rows ───────────────────────────────────────────────────────────────────────────────────

/// What a row that is not a walk resolves to against the live game.
pub enum Row {
    Walk(OverworldAction),
    /// The field move, or `resolve_field_move`'s sentence for why the game would refuse it.
    Field(Result<FieldMove, String>),
    /// A walk id the fresh action list no longer has.
    Gone,
}

/// Resolve one overworld id, a walk or a field row, against the live game.
pub fn resolve_row(state: &GameState, id: &str) -> Row {
    match field_request(state, id, None) {
        Some(request) => Row::Field(request.and_then(|request| resolve_field_move(state, &request))),
        None => resolve_overworld(state, id).map_or(Row::Gone, Row::Walk),
    }
}

/// The request a field row's id names, or `None` for a walk's id, whose first part is a map.
/// `actions` is the walk list to find a named thing in; `None` computes a fresh one if needed.
pub fn field_request(state: &GameState, id: &str, actions: Option<&[OverworldAction]>)
    -> Option<Result<FieldMoveRequest, String>> {
    use crate::pokemon::postgame::item_storage::PcItemOp;
    use crate::pokemon::postgame::pc_box::{PcBoxOp, BOX_COUNT};
    let (base, count) = match id.rsplit_once('*') {
        Some((base, count)) => (base, count.parse::<u32>().ok().filter(|count| *count >= 1)),
        None => (id, None),
    };
    let quantity = count.unwrap_or(1).min(MAX_ORDER) as u8;
    let slot = |text: &str| text.parse::<u8>()
        .map_err(|_| format!("`{id}`: `{text}` is not a slot number."));
    let item = |name: &str| item_by_name(name)
        .ok_or_else(|| format!("`{name}` is not an item this game has. `bag.md` spells them."));
    let map = |name: &str| map_by_name(name).ok_or_else(|| format!("`{name}` is not a map."));
    let (head, rest) = match base.split_once(':') {
        Some((head, rest)) => (head, Some(rest)),
        None => (base, None),
    };
    let party_move = |name| Ok(FieldMoveRequest::PartyMove { name, slot: None });
    let slotted = |rest: &str| -> Result<(ItemId, u8), String> {
        let (name, at) = rest.split_once('@').ok_or_else(|| format!("`{id}` needs a party slot after an `@`."))?;
        Ok((item(name)?, slot(at)?))
    };
    Some(match (head, rest) {
        ("dig", None) => party_move(PokemonMoveName::Dig),
        ("teleport", None) => party_move(PokemonMoveName::Teleport),
        ("flash", None) => party_move(PokemonMoveName::Flash),
        ("fly", Some(to)) => map(to).map(|to| FieldMoveRequest::Fly { to }),
        ("lift", Some(to)) => map(to).map(|to| FieldMoveRequest::UseElevator { to }),
        ("prize", Some(name)) => crate::pokemon::postgame::game_corner::Prize::named(name)
            .map(|prize| FieldMoveRequest::RedeemPrize { prize })
            .ok_or_else(|| format!("`{name}` is not one of the prize counter's prizes.")),
        ("lead", Some(at)) => slot(at).map(|slot| FieldMoveRequest::ReorderParty { slot }),
        ("toss", Some(name)) => item(name).map(|item| FieldMoveRequest::TossItem { item, count: quantity }),
        ("teach", Some(rest)) => slotted(rest).map(|(item, slot)| FieldMoveRequest::Teach { item, slot }),
        ("evolve", Some(rest)) => slotted(rest).map(|(stone, slot)| FieldMoveRequest::Evolve { stone, slot }),
        ("item", Some(rest)) => match rest.split_once('@') {
            None => item(rest).map(|item| FieldMoveRequest::UseItem { item, target: None, slot: None, evolve: true }),
            Some((name, at)) => {
                let (at, evolve) = match at.strip_suffix(STAY) {
                    Some(at) => (at, false),
                    None => (at, true),
                };
                match at.parse::<u8>() {
                    Ok(slot) => item(name).map(|item| FieldMoveRequest::UseItem { item, target: None, slot: Some(slot), evolve }),
                    // Anything else after the `@` is the id of the thing on the map it is used on.
                    Err(_) => item(name).and_then(|item| {
                        let fresh;
                        let actions = match actions {
                            Some(actions) => actions,
                            None => { fresh = state.map.actions(); &fresh }
                        };
                        thing_at(state, actions, at)
                            .map(|target| FieldMoveRequest::UseItem { item, target: Some(target), slot: None, evolve: true })
                            .ok_or_else(|| format!("`{at}` is not on this map any more, so there is nothing to use the {item:?} on."))
                    }),
                }
            }
        },
        ("pc", Some(rest)) => match rest.split_once(['@', ':']) {
            Some(("deposit", at)) => slot(at).map(|slot| FieldMoveRequest::UsePcBox { op: PcBoxOp::Deposit { slot } }),
            Some(("withdraw", at)) => slot(at).map(|box_slot| FieldMoveRequest::UsePcBox { op: PcBoxOp::Withdraw { box_slot } }),
            Some(("release", at)) => slot(at).map(|box_slot| FieldMoveRequest::UsePcBox { op: PcBoxOp::Release { box_slot } }),
            Some(("store", name)) => item(name).map(|item| FieldMoveRequest::UseItemPc { op: PcItemOp::Deposit, item, qty: quantity }),
            Some(("take", name)) => item(name).map(|item| FieldMoveRequest::UseItemPc { op: PcItemOp::Withdraw, item, qty: quantity }),
            None if rest == "box" => match count.unwrap_or(1) {
                n @ 1.. if n <= u32::from(BOX_COUNT) => Ok(FieldMoveRequest::UsePcBox { op: PcBoxOp::ChangeBox { n: n as u8 - 1 } }),
                n => Err(format!("There is no box {n}; the boxes are 1 to {BOX_COUNT}.")),
            },
            _ => Err(format!("`{id}` is not one of the PC's rows.")),
        },
        ("daycare", Some("withdraw")) => Ok(FieldMoveRequest::DayCare { deposit: None }),
        ("daycare", Some(rest)) => match rest.strip_prefix("deposit@") {
            Some(at) => slot(at).map(|slot| FieldMoveRequest::DayCare { deposit: Some(slot) }),
            None => Err(format!("`{id}` is not one of the Day Care's rows.")),
        },
        _ => return None,
    })
}

/// The suffix of the Rare Candy row that stops the evolution its level starts, as B does.
const STAY: &str = ":stay";

/// Where the person or object a walk row's id names is standing.
fn thing_at(state: &GameState, actions: &[OverworldAction], id: &str) -> Option<Point8> {
    use crate::pokemon::tile::MetaTile;
    let action = actions.iter().find(|action| action.id() == id)?;
    let MetaTile::Sprite(name) = action.tile else { return None };
    state.map.sprites.iter().find(|sprite| sprite.name == name).map(|sprite| sprite.position)
}

/// The items a row may use with nothing to aim at: the rest either ask which Pokémon, or have a
/// row of their own (a rod's `Fish`), or never report back from a bare use (the Poké Flute).
const UNAIMED_ITEMS: &[ItemId] = &[
    ItemId::Repel, ItemId::SuperRepel, ItemId::MaxRepel, ItemId::Bicycle, ItemId::EscapeRope,
    ItemId::Itemfinder,
];

/// The items a row may aim at someone on the map, and the picture that someone has.
const AIMED_ITEMS: &[(ItemId, crate::pokemon::sprite::PictureId)] =
    &[(ItemId::PokeFlute, crate::pokemon::sprite::PictureId::Snorlax)];

/// The overworld rows that are not walks, each kept only where [`resolve_field_move`] accepts it,
/// so a row is offered only when the game would allow it. A row with an `@<slot>` stands for one
/// per party slot, and its prose names the slots that work.
pub fn field_rows(state: &GameState, actions: &[OverworldAction]) -> Vec<MenuItem> {
    use crate::pokemon::postgame::pc_box::BOX_CAPACITY;
    let accepted = |id: &str| matches!(field_request(state, id, Some(actions)),
        Some(Ok(request)) if resolve_field_move(state, &request).is_ok());
    let party = 0..state.pokemon.len() as u8;
    let working = |make: &dyn Fn(u8) -> String| -> Vec<u8> {
        party.clone().filter(|&slot| accepted(&make(slot))).collect()
    };
    let who = |slot: u8| state.pokemon.get(slot as usize)
        .map_or_else(|| format!("{slot}"), |mon| format!("{slot} {}", crate::llm::prompt::named(mon)));
    let hurt = |slot: u8| state.pokemon.get(slot as usize).map_or_else(String::new, |mon| format!(
        "{} {}/{} HP{}", who(slot), mon.current_hp, mon.stats.hp, crate::llm::prompt::ailment(mon.status)));
    let list = |slots: &[u8], each: &dyn Fn(u8) -> String| slots.iter().map(|&slot| each(slot))
        .collect::<Vec<_>>().join("; ");
    let mut rows = Vec::new();
    let mut row = |id: String, description: String| rows.push(MenuItem { id, description });

    for to in (0..crate::pokemon::postgame::fly_bike::FLY_DESTINATIONS).filter_map(Map::from_repr) {
        let id = format!("fly:{to}");
        if to != state.map.map && accepted(&id) {
            row(id, format!("fly to {to}"));
        }
    }
    if accepted("dig") {
        row("dig".to_string(), "dig out of here, back to the town of the last Pokémon Center you used".to_string());
    }
    if accepted("teleport") {
        row("teleport".to_string(), "teleport to the town of the last Pokémon Center you used".to_string());
    }
    if state.map_is_dark && accepted("flash") {
        row("flash".to_string(), "light up this dark map with Flash".to_string());
    }

    for entry in state.bag.iter() {
        let (item, held) = (entry.id, entry.quantity);
        if UNAIMED_ITEMS.contains(&item) && accepted(&format!("item:{item:?}")) {
            row(format!("item:{item:?}"), match item {
                ItemId::Repel => format!("Repel ×{held}; keeps wild Pokémon weaker than your lead away for 100 steps"),
                ItemId::SuperRepel => format!("SuperRepel ×{held}; keeps wild Pokémon weaker than your lead away for 200 steps"),
                ItemId::MaxRepel => format!("MaxRepel ×{held}; keeps wild Pokémon weaker than your lead away for 250 steps"),
                ItemId::Bicycle if state.on_bicycle => "get off the Bicycle".to_string(),
                ItemId::Bicycle => "ride the Bicycle, twice as fast as walking".to_string(),
                ItemId::EscapeRope => format!("EscapeRope ×{held}; back to the town of the last Pokémon Center you used"),
                _ => format!("use the {item:?}"),
            });
        }
        if let Some(&(_, picture)) = AIMED_ITEMS.iter().find(|(aimed, _)| *aimed == item) {
            for action in actions {
                let crate::pokemon::tile::MetaTile::Sprite(name) = action.tile else { continue };
                let Some(sprite) = state.map.sprites.iter().find(|s| s.name == name && s.picture_id == picture) else { continue };
                let id = format!("item:{item:?}@{}", action.id());
                if accepted(&id) {
                    row(id, format!("walk up to the {name} at ({}, {}) and play the {item:?} to it",
                                    sprite.position.x, sprite.position.y));
                }
            }
        }
        if poke_core::item::opens_party_menu(item) && !is_stone(item) {
            let slots = working(&|slot| format!("item:{item:?}@{slot}"));
            if !slots.is_empty() {
                let each: &dyn Fn(u8) -> String = match restores_hp_or_status(item) { true => &hurt, false => &who };
                row(format!("item:{item:?}@<slot>"), format!("{item:?} ×{held} on a party member: {}", list(&slots, each)));
            }
            if item == ItemId::RareCandy {
                let evolving: Vec<u8> = slots.iter().copied().filter(|&slot| levels_into_evolution(state, slot)).collect();
                if !evolving.is_empty() {
                    row(format!("item:{item:?}@<slot>{STAY}"), format!(
                        "{item:?} ×{held} on a party member, stopping the evolution the new level starts: {}",
                        list(&evolving, &who)));
                }
            }
        }
        if is_stone(item) {
            let slots = working(&|slot| format!("evolve:{item:?}@{slot}"));
            if !slots.is_empty() {
                row(format!("evolve:{item:?}@<slot>"), format!("evolve a party member with the {item:?} ×{held}: {}",
                    list(&slots, &|slot| format!("{} can use it", who(slot)))));
            }
        }
        if let Some(taught) = crate::pokemon::learnset::machine_move(item) {
            let slots = working(&|slot| format!("teach:{item:?}@{slot}"));
            if !slots.is_empty() {
                let metadata = taught.metadata();
                let power = metadata.power.map_or_else(|| "no damage".to_string(), |power| format!("{power} power"));
                row(format!("teach:{item:?}@<slot>"), format!("{taught} ({}, {power}){}: {} can learn it",
                    metadata.move_type, if item.is_hm() { ", an HM, which cannot be forgotten" } else { "" },
                    list(&slots, &who)));
            }
        }
    }

    let leads: Vec<u8> = party.clone().skip(1).filter(|&slot| accepted(&format!("lead:{slot}"))).collect();
    if !leads.is_empty() {
        row("lead:<slot>".to_string(), format!("make that party member the leader: {}", list(&leads, &who)));
    }
    if state.bag.iter().any(|entry| accepted(&format!("toss:{:?}", entry.id))) {
        row("toss:<item>*N".to_string(), "throw away N of a bag item, as `toss:Antidote*3` does (default 1); all of \
             it frees its bag slot, and a key item or an HM cannot be thrown away. `bag.md` lists what you carry".to_string());
    }

    if let Some((_, pokemon)) = usable_pc(state.map.map) {
        let open = state.current_box + 1;
        let boxed = state.boxed_pokemon.len();
        if pokemon {
            let slots = working(&|slot| format!("pc:deposit@{slot}"));
            if !slots.is_empty() {
                row("pc:deposit@<slot>".to_string(), format!(
                    "put a party member into the PC's open box, Box {open} ({boxed} of {BOX_CAPACITY} used): {}",
                    list(&slots, &who)));
            }
            let in_box: Vec<String> = state.boxed_pokemon.iter().enumerate()
                .map(|(at, mon)| format!("{at} {} Lv{}", mon.species, mon.level)).collect();
            if accepted("pc:withdraw@0") {
                row("pc:withdraw@<box slot>".to_string(), format!(
                    "take a Pokémon out of Box {open} into the party: {}", in_box.join("; ")));
            }
            if accepted("pc:release@0") {
                row("pc:release@<box slot>".to_string(), format!(
                    "release one of Box {open}'s Pokémon for good, numbered as `pc:withdraw` numbers them"));
            }
            row("pc:box*N".to_string(), format!(
                "open box N of {} instead of Box {open}, as `pc:box*2` does; only the open box can be read or used",
                crate::pokemon::postgame::pc_box::BOX_COUNT));
        }
        if state.bag.iter().any(|entry| accepted(&format!("pc:store:{:?}", entry.id))) {
            row("pc:store:<item>*N".to_string(), "put N of a bag item into PC item storage, as `pc:store:Potion*5` \
                 does (default 1); `bag.md` lists what you carry".to_string());
        }
        row("pc:take:<item>*N".to_string(), "take N of an item out of PC item storage into the bag, as \
             `pc:take:Potion*5` does (default 1); `pc.md` lists what is stored".to_string());
    }

    if let Some((_, floors)) = crate::pokemon::tile_map::elevator_for(state.map.map) {
        for floor in floors {
            let id = format!("lift:{floor}");
            if accepted(&id) {
                row(id, format!("ride the lift to {floor}"));
            }
        }
    }
    if state.map.map == Map::Daycare {
        let slots = working(&|slot| format!("daycare:deposit@{slot}"));
        if !slots.is_empty() {
            row("daycare:deposit@<slot>".to_string(), format!(
                "board a party member at the Day Care, to gain a level for every so many steps you walk: {}",
                list(&slots, &who)));
        }
        if accepted("daycare:withdraw") {
            row("daycare:withdraw".to_string(), "pay for and collect the Pokémon boarded here".to_string());
        }
    }
    if state.map.map == Map::GameCornerPrizeRoom {
        use crate::pokemon::postgame::game_corner::Prize;
        for prize in [Prize::Abra, Prize::Clefairy, Prize::Nidorina, Prize::Dratini, Prize::Scyther,
                      Prize::Porygon, Prize::HyperBeam, Prize::DragonRage, Prize::Substitute] {
            let name = prize.item().map_or_else(|| format!("{prize:?}"), |item| format!("{item:?}"));
            let id = format!("prize:{name}");
            if accepted(&id) {
                row(id, format!("buy the {name} for {} coins", prize.cost()));
            }
        }
    }
    rows
}

/// The items whose row is worth showing each Pokémon's HP and status beside.
fn restores_hp_or_status(item: ItemId) -> bool {
    use ItemId::*;
    matches!(item, Potion | SuperPotion | HyperPotion | MaxPotion | FullRestore | FreshWater | SodaPop
        | Lemonade | Revive | MaxRevive | Antidote | BurnHeal | IceHeal | Awakening | ParlyzHeal | FullHeal)
}

/// The evolution stones, which have `evolve:` rows rather than `item:` ones.
fn is_stone(item: ItemId) -> bool {
    matches!(item, ItemId::MoonStone | ItemId::FireStone | ItemId::ThunderStone | ItemId::WaterStone | ItemId::LeafStone)
}

/// Whether one more level evolves the party member in `slot`.
fn levels_into_evolution(state: &GameState, slot: u8) -> bool {
    use poke_core::evos_moves::{EvosMoves, Evolution};
    state.pokemon.get(slot as usize).is_some_and(|mon| mon.level < 100
        && EvosMoves::of(mon.species).evolutions.iter()
            .any(|evolution| matches!(*evolution, Evolution::Level { level, .. } if mon.level + 1 >= level)))
}

// ── The catalogue ────────────────────────────────────────────────────────────────────────────────

/// Every [`DecisionKind`], so a loop over all of them keeps meaning it when one is added.
pub const ALL_KINDS: [DecisionKind; 7] = [
    DecisionKind::Overworld,
    DecisionKind::Battle,
    DecisionKind::Nickname,
    DecisionKind::MartPurchase,
    DecisionKind::ForgetMove,
    DecisionKind::MimicMove,
    DecisionKind::Stuck,
];

/// The four tools, the same on every kind of turn: what varies is the menu the turn carries.
pub fn catalogue() -> Vec<ToolSpec> {
    let mut tools = vec![choose_action_spec()];
    tools.extend(files::specs());
    tools
}

/// The one action tool, the same on every turn: what varies is the menu the turn carries.
fn choose_action_spec() -> ToolSpec {
    let mut spec = ToolSpec::new(
        CHOOSE_ACTION,
        format!(
            "ENDS THE TURN. Take one of the actions in this turn's menu. `id` is an id from that \
             menu, copied exactly — never a position in the list. An id holding `<slot>`, `<box \
             slot>` or `<item>` stands for one per slot or item: send it with that filled in, as \
             `item:Potion@2` or `teach:Tm28Dig@0`. `*N` on the end of a `buy:` row or a row whose id \
             ends in `*N` asks for N of it: `buy:PokeBall*10`, `toss:Antidote*3`. Every turn also takes `wait*N`, \
             which lets the game run N ticks of 20 ms (default {DEFAULT_WAIT_TICKS}, at most \
             {MAX_WAIT_TICKS}) and then asks again. On an overworld or mart turn up to \
             {MAX_CHAINED_ACTIONS} calls in one reply are taken in order without asking you again, \
             each checked against the game as its turn comes; the first that will not go or is \
             stopped drops the rest, and the next turn says where it got to. Every other turn takes \
             one call. The turn's own last line says which applies."
        ),
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "An id from this turn's menu." },
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
    );
    add_summary_argument(&mut spec);
    // Required on the first call of a reply only, which a schema cannot say: `classify_reply`.
    spec.function.parameters["required"] = json!(["id"]);
    if let Some(summary) = spec.function.parameters["properties"]["summary"]["description"].as_str() {
        let summary = format!("{summary} Required on the first `{CHOOSE_ACTION}` of a reply; the rest may leave it out.");
        spec.function.parameters["properties"]["summary"]["description"] = json!(summary);
    }
    spec
}

/// Long enough for the intent and the reason, short enough to carry one per turn for a whole run.
pub const MAX_SUMMARY: usize = 300;

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

/// Longer than a summary: an issue is written to disk once and read by a person.
pub const MAX_ISSUE: usize = 1_000;

/// The tools that do not end the turn.
pub fn non_terminal_names() -> [&'static str; 3] {
    [files::READ_FILE, files::WRITE_FILE, files::EDIT_FILE]
}

pub fn terminal_names() -> &'static [&'static str] {
    &[CHOOSE_ACTION]
}

// ── Classification ───────────────────────────────────────────────────────────────────────────────

/// The complaint when the model chose an id this turn never offered, or `None` when it did.
fn not_on_the_menu(id: &str, menu: &[String]) -> Option<String> {
    if menu.is_empty() || menu.iter().any(|offered| fills(offered.strip_suffix(COUNTED).unwrap_or(offered), id)) {
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

/// What a row's id ends in when it takes a count.
const COUNTED: &str = "*N";

/// Whether `sent` is the row `row`, or fills the one `<placeholder>` in it: a party slot, a box
/// slot or an item.
fn fills(row: &str, sent: &str) -> bool {
    if row == sent {
        return true;
    }
    let Some((prefix, rest)) = row.split_once('<') else { return false };
    let Some((placeholder, suffix)) = rest.split_once('>') else { return false };
    let Some(middle) = sent.strip_prefix(prefix).and_then(|tail| tail.strip_suffix(suffix)) else { return false };
    match placeholder {
        "slot" => middle.parse::<u8>().is_ok_and(|slot| slot < 6),
        "box slot" => middle.parse::<usize>().is_ok_and(|slot| slot < crate::pokemon::postgame::pc_box::BOX_CAPACITY),
        "item" => item_by_name(middle).is_some(),
        _ => false,
    }
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
        if menu.iter().any(|offered| offered.starts_with(&format!("item:{item}:"))) {
            return " That item asks which move as well, so its rows carry the move slot after a second \
                    `:`, as `item:Ether@0:1` does. Use one of those.";
        }
        if item.contains('@') {
            return " Either the bag has none left, or it would do nothing for that Pokémon and the \
                    cartridge would only say \"It won't have any effect.\" An item that asks which \
                    Pokémon has a row for each one it would help: a Revive for one that has fainted, a \
                    potion for one that is hurt, a cure for one with that status, an Ether for each \
                    move short of PP. `bag.md` says what is left.";
        }
        return " That item is not in the bag. A bag row goes the moment the last one is used, so an \
                id you read on an earlier turn stops resolving; `bag.md` says what is left.";
    }
    ""
}

/// Decide what one call is on its own, without touching the game; see [`classify_reply`].
pub fn classify(kind: DecisionKind, call: &ToolCall, menu: &[String]) -> CallKind {
    classify_reply(kind, std::slice::from_ref(call), menu)
        .pop()
        .unwrap_or_else(|| CallKind::Rejected("There was no call to classify.".to_string()))
}

/// Decide what every call in one reply is, without touching the game. The reply's `choose_action`
/// calls fold into one [`Terminal`], carried by the first of them, and the rest are
/// [`CallKind::Chained`]. They are taken all or none, so one refused refuses every one.
pub fn classify_reply(kind: DecisionKind, calls: &[ToolCall], menu: &[String]) -> Vec<CallKind> {
    let mut classified = Vec::with_capacity(calls.len());
    let mut picks: Vec<(usize, Result<Pick, String>)> = Vec::new();
    for (index, call) in calls.iter().enumerate() {
        match call.function.name == CHOOSE_ACTION {
            true => {
                picks.push((index, pick(kind, call, menu)));
                classified.push(CallKind::Chained);
            }
            false => classified.push(match classify_call(call) {
                CallKind::Terminal(_) if call_summary(call).is_none() => CallKind::Rejected(summary_missing(&call.function.name)),
                other => other,
            }),
        }
    }
    if picks.is_empty() {
        return classified;
    }

    let folded = match picks.iter().any(|(_, pick)| pick.is_err()) {
        true => Err(format!(
            "Not taken: another `{CHOOSE_ACTION}` in this reply was refused, and the actions in one \
             reply are taken all or none. Send them again with that one put right."
        )),
        false => fold(kind, picks.iter().filter_map(|(_, pick)| pick.clone().ok()).collect()).and_then(|terminal| {
            match picks.iter().any(|(index, _)| call_summary(&calls[*index]).is_some()) {
                true => Ok(terminal),
                false => Err(summary_missing(CHOOSE_ACTION)),
            }
        }),
    };
    match folded {
        Ok(terminal) => {
            let mut terminal = Some(terminal);
            for (index, _) in &picks {
                classified[*index] = match terminal.take() {
                    Some(terminal) => CallKind::Terminal(terminal),
                    None => CallKind::Chained,
                };
            }
        }
        Err(complaint) => {
            for (index, pick) in picks {
                classified[index] = CallKind::Rejected(pick.err().unwrap_or_else(|| complaint.clone()));
            }
        }
    }
    classified
}

/// The summary of a reply's decision: the first its action calls carry.
pub fn reply_summary(calls: &[ToolCall], classified: &[CallKind]) -> Option<String> {
    calls
        .iter()
        .zip(classified)
        .filter(|(_, kind)| matches!(kind, CallKind::Terminal(_) | CallKind::Chained))
        .find_map(|(call, _)| call_summary(call))
}

fn summary_missing(name: &str) -> String {
    format!(
        "`{name}` needs a `summary`: one or two sentences saying what you are doing and why. Your \
         thinking is not kept, so it is the only note you will have of this turn. Call it again \
         with one."
    )
}

/// One `choose_action` id, parsed for the kind of turn it was sent on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pick {
    Overworld(String),
    Battle(String),
    TakeOver,
    Buy(BagItem),
    Leave,
    Nickname(Option<String>),
    Forget(Option<u8>),
    Mimic(u8),
    Press(Vec<JoypadButton>),
    Wait(u16),
}

/// What a battle turn's menu calls the row that takes the rest of the battle from the script.
pub const TAKE_OVER: &str = "take_over";

/// The prefix of the stuck turn's raw presses.
const PRESS: &str = "press:";

/// The several picks of one reply as the one [`Terminal`] the agent carries out.
fn fold(kind: DecisionKind, picks: Vec<Pick>) -> Result<Terminal, String> {
    if let [Pick::Wait(ticks)] = picks.as_slice() {
        return Ok(Terminal::Wait { ticks: *ticks });
    }
    if picks.iter().any(|pick| matches!(pick, Pick::Wait(_))) {
        return Err("`wait` is a whole answer on its own: send it alone, or send the actions without it."
            .to_string());
    }
    let sent = picks.len();
    let one_only = move || {
        format!(
            "This turn takes one `{CHOOSE_ACTION}` and that reply sent {sent}. The next question is a \
             turn of its own with its own menu, so send just the one for this one."
        )
    };
    if chains(kind) && picks.len() > MAX_CHAINED_ACTIONS {
        return Err(format!(
            "That reply sent {} actions and one reply may carry at most {MAX_CHAINED_ACTIONS}. Nothing \
             was done: send the first {MAX_CHAINED_ACTIONS} and the rest once they are done; you will \
             be shown a fresh menu then anyway.",
            picks.len(),
        ));
    }
    match kind {
        DecisionKind::Overworld => {
            let mut ids = picks.into_iter().filter_map(|pick| match pick {
                Pick::Overworld(id) => Some(id),
                _ => None,
            });
            let id = ids.next().ok_or_else(one_only)?;
            Ok(Terminal::ChooseAction { id, then: ids.collect() })
        }
        DecisionKind::MartPurchase => {
            let leaves = picks.iter().position(|pick| *pick == Pick::Leave);
            if leaves.is_some_and(|at| at + 1 != picks.len()) {
                return Err("`leave` ends the visit, so it can only come last. Nothing was bought; send \
                            the orders first."
                    .to_string());
            }
            let mut orders = picks.into_iter().filter_map(|pick| match pick {
                Pick::Buy(item) => Some(item),
                _ => None,
            });
            Ok(Terminal::BuyItem { item: orders.next(), then: orders.collect() })
        }
        DecisionKind::Battle => {
            let take_over = picks.contains(&Pick::TakeOver);
            let actions: Vec<String> = picks
                .iter()
                .filter_map(|pick| match pick {
                    Pick::Battle(id) => Some(id.clone()),
                    _ => None,
                })
                .collect();
            match (actions.as_slice(), picks.len() - actions.len()) {
                ([id], 0 | 1) => Ok(Terminal::ChooseBattleAction { id: id.clone(), take_over }),
                ([], _) => Err(format!(
                    "`{TAKE_OVER}` hands you the rest of this battle, and this turn still needs its \
                     action: send it together with one of the other rows."
                )),
                _ => Err(format!(
                    "A battle turn takes one action, plus `{TAKE_OVER}` when it is offered. Send the \
                     one you want for this turn."
                )),
            }
        }
        _ => match <[Pick; 1]>::try_from(picks) {
            Ok([Pick::Nickname(name)]) => Ok(Terminal::SetNickname { name }),
            Ok([Pick::Forget(slot)]) => Ok(Terminal::ForgetMove { slot }),
            Ok([Pick::Mimic(slot)]) => Ok(Terminal::MimicMove { slot }),
            Ok([Pick::Press(buttons)]) => Ok(Terminal::PressButtons { buttons }),
            Ok([other]) => Err(format!("`{other:?}` cannot answer a {} turn.", kind.label())),
            Err(_) => Err(one_only()),
        },
    }
}

/// Whether one reply may carry several `choose_action` calls on this kind.
pub fn chains(kind: DecisionKind) -> bool {
    matches!(kind, DecisionKind::Overworld | DecisionKind::MartPurchase)
}

/// One `choose_action` call, read against the turn it was sent on.
fn pick(kind: DecisionKind, call: &ToolCall, menu: &[String]) -> Result<Pick, String> {
    let arguments = call
        .arguments()
        .map_err(|failure| format!("{failure}. Send the arguments as a JSON object and try again."))?;
    let id = string_argument(&arguments, "id")?;
    let id = id.trim();

    if id == "wait" {
        return Ok(Pick::Wait(DEFAULT_WAIT_TICKS));
    }
    if let Some(count) = id.strip_prefix("wait*") {
        return match count.parse::<u64>() {
            Ok(ticks) => Ok(Pick::Wait(ticks.clamp(1, u64::from(MAX_WAIT_TICKS)) as u16)),
            Err(_) => Err(format!("`{id}`: `wait*N` takes a whole number of ticks, as `wait*50` does.")),
        };
    }
    if kind == DecisionKind::Nickname {
        return nickname(id).map(Pick::Nickname);
    }
    if kind == DecisionKind::Stuck {
        return match id.strip_prefix(PRESS) {
            Some(buttons) => press(buttons).map(Pick::Press),
            None => Err(format!(
                "`{id}` is not something this turn can do. With no menu, the ids are `{PRESS}` and the \
                 buttons in order, as `{PRESS}b,a`, or `wait*N`."
            )),
        };
    }
    if id.starts_with(PRESS) {
        return Err(format!(
            "`{PRESS}` is offered only on a turn where the agent has stopped asking anything, and this \
             turn has a menu: the agent presses the buttons. Take one of the ids in the list. If what \
             you want genuinely is not in it, write a file under `issues/` to say so and then take \
             the closest action there is."
        ));
    }

    let (base, count) = match id.rsplit_once('*') {
        None => (id, None),
        Some((base, count)) => match count.parse::<u32>() {
            Ok(count) if count >= 1 => (base, Some(count)),
            _ => return Err(format!("`{id}`: `*` takes a whole number after it, as `buy:PokeBall*10` does.")),
        },
    };
    let counted = (kind == DecisionKind::MartPurchase && base.starts_with("buy:"))
        || (kind == DecisionKind::Overworld
            && menu.iter().any(|row| row.strip_suffix(COUNTED).is_some_and(|row| fills(row, base))));
    if count.is_some() && !counted {
        return Err(format!(
            "`{id}`: only a mart's `buy:` rows, the rows whose id ends in `{COUNTED}`, and `wait` take \
             a count. Send `{base}` on its own."
        ));
    }

    if kind == DecisionKind::Battle && base == TAKE_OVER {
        return match menu.iter().any(|offered| offered == TAKE_OVER) {
            true => Ok(Pick::TakeOver),
            false => Err(format!(
                "`{TAKE_OVER}` is offered only on a battle turn your armed battle script handed back \
                 to you, and this is not one."
            )),
        };
    }
    if let Some(complaint) = not_on_the_menu(base, menu) {
        return Err(match kind {
            DecisionKind::Battle => format!("{complaint}{}", battle_rule_behind(base, menu)),
            _ => complaint,
        });
    }

    match kind {
        DecisionKind::Overworld => Ok(Pick::Overworld(match count {
            Some(count) => format!("{base}*{count}"),
            None => base.to_string(),
        })),
        DecisionKind::Battle => Ok(Pick::Battle(base.to_string())),
        DecisionKind::MartPurchase => match base.strip_prefix("buy:") {
            None if base == "leave" => Ok(Pick::Leave),
            None => Err(format!("`{base}` is not a row of this shop. Buy with a `buy:` row, or `leave`.")),
            Some(name) => match item_by_name(name) {
                Some(item) => Ok(Pick::Buy(BagItem::new(item, count.unwrap_or(1).min(MAX_ORDER) as u8))),
                None => Err(format!("`{name}` is not an item this game has. Copy a `buy:` id from the list exactly.")),
            },
        },
        DecisionKind::ForgetMove => match base {
            "keep" => Ok(Pick::Forget(None)),
            _ => match base.strip_prefix("forget:").and_then(|slot| slot.parse::<u8>().ok()) {
                Some(slot) if slot < 4 => Ok(Pick::Forget(Some(slot))),
                _ => Err(format!(
                    "`{base}` is not one of the rows: a Pokémon has four move slots, `forget:0` to \
                     `forget:3`, and `keep` declines the new move."
                )),
            },
        },
        DecisionKind::MimicMove => match base.strip_prefix("mimic:").and_then(|row| row.parse::<u8>().ok()) {
            Some(row) if row < 4 => Ok(Pick::Mimic(row)),
            _ => Err(format!(
                "`{base}` is not one of the rows: the enemy's moves are `mimic:0` onwards, as the list \
                 in the turn shows them."
            )),
        },
        DecisionKind::Nickname | DecisionKind::Stuck => unreachable!("answered above"),
    }
}

/// The naming turn's id is the name itself, and `NONE` keeps the species name.
fn nickname(id: &str) -> Result<Option<String>, String> {
    if id.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let length = id.chars().count();
    if length > MAX_NICKNAME {
        return Err(format!(
            "`{id}` is {length} characters and the game holds at most {MAX_NICKNAME}. Pick a shorter \
             name."
        ));
    }
    // Written straight into the naming screen's buffer, so nothing else checks it.
    match unencodable(id) {
        Some(bad) => Err(format!(
            "`{bad}` is not a character this game can write. A nickname may use letters, digits, \
             spaces and `.,:;'-?!()[]/`, and nothing else; pick a name out of those and send it again."
        )),
        None => Ok(Some(id.to_string())),
    }
}

/// `press:`'s buttons, comma-separated, in order.
fn press(list: &str) -> Result<Vec<JoypadButton>, String> {
    let names: Vec<&str> = list.split([',', ' ']).map(str::trim).filter(|name| !name.is_empty()).collect();
    if names.is_empty() {
        return Err(format!("`{PRESS}` needs at least one button after it, as `{PRESS}b,a` does."));
    }
    names
        .iter()
        .map(|name| {
            button_by_name(name).ok_or_else(|| {
                format!("`{name}` is not a button: up, down, left, right, a, b, start or select.")
            })
        })
        .take(MANUAL_INPUT_CAPACITY)
        .collect()
}

/// Every call but `choose_action`.
pub(crate) fn classify_call(call: &ToolCall) -> CallKind {
    if let Some(parsed) = files::parse(call) {
        return match parsed {
            Err(complaint) => CallKind::Rejected(complaint),
            Ok(FileCall::Read(read)) => match read.node {
                Node::ScreenPng => CallKind::Screenshot,
                ref node if node.on_the_emulator_thread() => CallKind::Read(read),
                _ => CallKind::File(FileCall::Read(read)),
            },
            Ok(FileCall::Write { node: Node::Issue(name), content }) => match capped(&content, MAX_ISSUE) {
                Some(message) => CallKind::Issue { name, message },
                None => CallKind::Rejected(
                    "An issue needs something in it: what you tried, what you expected and what \
                     happened. Writing it does not end your turn: file it, then take an action."
                        .to_string(),
                ),
            },
            Ok(other) => CallKind::File(other),
        };
    }
    let name = call.function.name.as_str();
    match replaced_by(name) {
        Some(instead) => CallKind::Rejected(format!("There is no `{name}` any more: {instead}")),
        None => CallKind::Rejected(format!(
            "There is no tool called `{name}`. The tools that do not end the turn are {}; end the \
             turn with {}.",
            non_terminal_names().iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", "),
            terminal_names().iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(" or "),
        )),
    }
}

/// What an older conversation's tool became, for a resumed history that still shows the model
/// calling it.
fn replaced_by(name: &str) -> Option<&'static str> {
    Some(match name {
        "choose_battle_action" | "set_nickname" | "buy_item" | "forget_move" | "mimic_move"
        | "press_buttons" | "wait" | "use_field_move" => {
            "every turn ends with `choose_action`, and the id says what. Take an id from this turn's menu."
        }
        "read_map" => "`read_file` with `map.png` answers the map picture.",
        "screenshot" => "`read_file` with `screen.png` answers the screen.",
        "read_party" => "`read_file` with `party.md` answers the party.",
        "read_bag" => "`read_file` with `bag.md` answers the bag.",
        "read_pc" => "`read_file` with `pc.md` answers the PC.",
        "read_guide" => "`read_file` with `guide/current.md` answers the walkthrough chapter for your badges.",
        "read_route" => "`read_file` with `routes.md` lists the maps you have walked, and `routes/<Map>.md` is the way to one.",
        "read_battle" => "the battle turn itself shows both sides and costs your moves against the enemy.",
        "todo_set" | "todo_add" | "todo_complete" | "todo_delete" => {
            "your plan is the file `plan.md`: `read_file` it, and change it with `write_file` or `edit_file`."
        }
        "set_battle_script" | "read_battle_script" => {
            "your battle script is the file `battle-script.rhai`: `read_file` it, and change it with \
             `write_file` or `edit_file`."
        }
        "get_battle_script_docs" => "`read_file` with `battle-script.md` answers the script API.",
        "report_issue" => "`write_file` a new file under `issues/`, as `issues/missing-door.md`, to report a problem.",
        _ => return None,
    })
}

/// Trimmed and capped on a character boundary; `None` when nothing is left.
fn capped(text: &str, cap: usize) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(cap) {
        Some((cut, _)) => text[..cut].to_string(),
        None => text.to_string(),
    })
}

fn string_argument(arguments: &Value, key: &str) -> Result<String, String> {
    match arguments.get(key).and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => Ok(value.to_string()),
        _ => Err(format!("`{key}` is required and must be a non-empty string.")),
    }
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
        // A map edge says where it lands, as a warp does: a map with two openings into the same
        // neighbour has a row for each, and they come out in parts of it that may not reach one
        // another, so the landing is the only thing that tells the rows apart.
        MetaTile::Connection { to_map, to_position } => {
            let (dx, dy) = crate::pokemon::map_header::strip_offset(to_map);
            format!("walk into {to_map}, arriving at ({}, {})",
                    to_position.x + dx, to_position.y + dy)
        }
        MetaTile::ConnectionWater(to_map) => format!("surf into {to_map}"),
        // The hunts say they go nowhere, or a model reads "surf" as a way across the sea.
        MetaTile::Grass =>
            "pace back and forth in the nearest tall grass until a wild Pokémon appears; this goes nowhere".to_string(),
        // Where there is no grass, as a cave's floor and the sea's surface roll the same.
        MetaTile::Pace { water: false } =>
            "pace back and forth on the nearest floor until a wild Pokémon appears, as any floor here rolls them; \
             this goes nowhere".to_string(),
        MetaTile::Pace { water: true } =>
            "surf back and forth on the nearest water until a wild water Pokémon appears; this goes nowhere, \
             so to cross the water take a row that names where it leads".to_string(),
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
            let stand_and_target = |verb: &str| match sprite {
                Some(sprite) => format!(
                    "{verb} (you stand at ({}, {}); it is at ({}, {}))",
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
        // No switch and no hole on this map, so the shove is the whole action.
        MetaTile::BoulderPush { boulder, dir } => format!(
            "push the boulder at ({}, {}) one square {}, to get it out of the way (you stand at \
             ({}, {}))",
            boulder.x, boulder.y, crate::pokemon::tile_map::push_word(dir),
            action.destination.x, action.destination.y),
        MetaTile::Slots => format!(
            "play the slots at the nearest free machine (you stand at ({}, {})): up to {} spins, each \
             betting 3 coins, or all that is left below 3, with any win paid in coins; it stops early \
             if the coins run out",
            action.destination.x, action.destination.y, crate::pokemon::postgame::game_corner::SLOT_SPINS),
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
            // Either half of a two-square door: the row is the door, whichever half is nearer.
            MetaTile::Warp { .. } => action.destination == arrival.at
                || state.map.tile_at_checked(arrival.at) == Some(action.tile),
            MetaTile::Connection { to_map, .. } | MetaTile::ConnectionWater(to_map) => Some(to_map) == arrival.from,
            _ => false,
        }
    };
    let mut actions = state.map.actions();
    let field = field_rows(state, &actions);
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
        .chain(field)
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
    state.map.actions().into_iter().find(|action| overworld_id(state, action) == id)
}

/// Keyed on what the action is: a bag slot shifts when an item runs out, and PP changes on use.
pub fn battle_id(action: &BattleAction) -> String {
    match action {
        BattleAction::Fight { battle_move, .. } => format!("fight:{}", battle_move.name),
        BattleAction::UseItem { item, target: None, .. } => format!("item:{:?}", item.id),
        BattleAction::UseItem { item, target: Some(target), target_move: None, .. } => format!("item:{:?}@{target}", item.id),
        BattleAction::UseItem { item, target: Some(target), target_move: Some(target_move), .. } =>
            format!("item:{:?}@{target}:{target_move}", item.id),
        BattleAction::SwitchPokemon { slot, .. } => format!("switch:{slot}"),
        BattleAction::Run => "run".to_string(),
        BattleAction::SafariBall => "ball".to_string(),
        BattleAction::SafariBait => "bait".to_string(),
        BattleAction::SafariRock => "rock".to_string(),
    }
}

/// The battle menu, with every `fight:` row costed against the Pokémon actually in front of you and
/// saying what its move does.
pub fn battle_menu(state: &GameState) -> Vec<MenuItem> {
    let sides = state.battle.as_ref().map(|battle| (&battle.player, &battle.enemy, battle.ruleset));
    battle_options(state)
        .unwrap_or_default()
        .iter()
        .map(|action| {
            let mut description = format!("{action}");
            if let (BattleAction::Fight { battle_move, .. }, Some((me, foe, ruleset))) = (action, sides) {
                description.push_str(&fight_row_notes(battle_move.name, me, foe, ruleset));
            }
            if let BattleAction::UseItem { item, target: Some(target), target_move, .. } = action
                && let Some(mon) = state.pokemon.get(*target as usize)
            {
                use crate::pokemon::postgame::items::{max_pp, move_pp};
                let pp = |mv: &PokemonMove| format!("{}/{} PP", move_pp(mv), max_pp(mv));
                let condition = match (mon.current_hp, mon.status) {
                    (0, _) => "fainted".to_string(),
                    (hp, crate::pokemon::status::PokemonStatus::None) => format!("{hp}/{} HP", mon.stats.hp),
                    (hp, status) => format!("{hp}/{} HP, {status}", mon.stats.hp),
                };
                let who = format!("{} the {}", mon.nickname.to_default_string(), mon.species);
                description = match target_move.and_then(|slot| mon.moves.get(slot as usize).copied().flatten()) {
                    Some(mv) => format!("ITEM   {} ×{} on {who}'s {}, {}", item.id, item.quantity, mv.name, pp(&mv)),
                    None if crate::pokemon::item_use::restores_pp(item.id, &mon.moves).is_some() =>
                        format!("ITEM   {} ×{} on {who}, every move: {}", item.id, item.quantity,
                                mon.moves.iter().flatten().map(|mv| format!("{} {}", mv.name, pp(mv))).collect::<Vec<_>>().join(", ")),
                    None => format!("ITEM   {} ×{} on {who}, {condition}", item.id, item.quantity),
                };
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
    ruleset: poke_core::ruleset::Ruleset,
) -> String {
    use crate::pokemon::damage::{effectiveness_phrase, expected_damage, is_damaging_move, type_multiplier};
    // The multiplier is only reported for a move that deals damage.
    if !is_damaging_move(name) {
        return String::new();
    }
    match (expected_damage(me, name, foe, ruleset), effectiveness_phrase(type_multiplier(name, foe, ruleset))) {
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

/// What a move does besides its damage, under `ruleset`. A plain attack's "No additional effect."
/// is none, as it would be on most rows of every menu.
pub fn move_effect(name: crate::pokemon::move_name::PokemonMoveName, ruleset: poke_core::ruleset::Ruleset) -> Option<&'static str> {
    match &name.metadata().effect {
        crate::pokemon::move_name::PokemonMoveEffect::NoAdditionalEffect => None,
        effect => Some(effect.description(ruleset)),
    }
}

/// A `fight:` row's damage estimate, then what the move does besides under the battle's own rules.
fn fight_row_notes(
    name: crate::pokemon::move_name::PokemonMoveName,
    me: &crate::pokemon::pokemon::PokemonSummary,
    foe: &crate::pokemon::pokemon::PokemonSummary,
    ruleset: poke_core::ruleset::Ruleset,
) -> String {
    let damage = fight_row_note(name, me, foe, ruleset);
    match move_effect(name, ruleset) {
        None => damage,
        Some(effect) if damage.is_empty() => format!(" — {effect}"),
        Some(effect) => format!("{damage}. {effect}"),
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

/// The battle row an armed script's hand-back offers.
pub fn take_over_row() -> MenuItem {
    MenuItem {
        id: TAKE_OVER.to_string(),
        description: "decide the rest of this battle yourself: your battle script decides none of \
                      its turns, and goes back to deciding at the next battle. Send it as well as \
                      this turn's action"
            .to_string(),
    }
}

/// What the mart sells (see [`ApiSnapshot`]), each as `buy:` and the item as the battle menu spells
/// it, and the way out.
pub fn mart_menu(snapshot: &ApiSnapshot, state: &GameState) -> Vec<MenuItem> {
    let mut menu: Vec<MenuItem> = snapshot
        .mart_stock
        .iter()
        .map(|(item, price)| {
            let held = state.bag.iter().find(|entry| entry.id == *item).map_or(0, |e| e.quantity);
            MenuItem {
                id: format!("buy:{item:?}"),
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
        .collect();
    // An unread stock is an empty menu, which `not_on_the_menu` holds nothing to.
    if menu.is_empty() {
        return menu;
    }
    menu.push(MenuItem {
        id: "leave".to_string(),
        description: "walk away without buying anything; after the last order you leave anyway".to_string(),
    });
    menu
}

/// The four moves the forget prompt chooses between, keyed on slot, each saying what it does by
/// the run's rules, and the row that declines.
pub fn forget_menu(current: &[PokemonMove], ruleset: poke_core::ruleset::Ruleset) -> Vec<MenuItem> {
    let mut menu: Vec<MenuItem> = current
        .iter()
        .enumerate()
        .map(|(slot, known)| {
            let metadata = known.name.metadata();
            MenuItem {
                id: format!("forget:{slot}"),
                description: format!(
                    "{} — {}, {}, {}/{} pp{}{}",
                    known.name,
                    metadata.move_type,
                    match metadata.power {
                        Some(power) => format!("{power} power"),
                        None => "no damage".to_string(),
                    },
                    known.pp,
                    metadata.pp,
                    move_effect(known.name, ruleset).map_or(String::new(), |effect| format!(". {effect}")),
                    match hm_move(known.name) {
                        Some(_) => " — ⚠️ an HM move, and it cannot be re-learnt",
                        None => "",
                    },
                ),
            }
        })
        .collect();
    menu.push(MenuItem {
        id: "keep".to_string(),
        description: "decline the new move and keep all four".to_string(),
    });
    menu
}

/// The enemy's moves Mimic's menu chooses between, keyed on row, each costed
/// against the enemy from the mon out and saying what it does.
pub fn mimic_menu(state: &GameState, enemy_moves: &[PokemonMove]) -> Vec<MenuItem> {
    let sides = state.battle.as_ref().map(|battle| (&battle.player, &battle.enemy, battle.ruleset));
    enemy_moves
        .iter()
        .enumerate()
        .map(|(row, known)| {
            let metadata = known.name.metadata();
            let notes = match sides {
                Some((me, foe, ruleset)) => fight_row_notes(known.name, me, foe, ruleset),
                None => move_effect(known.name, state.ruleset).map_or(String::new(), |effect| format!(" — {effect}")),
            };
            MenuItem {
                id: format!("mimic:{row}"),
                description: format!(
                    "{} — {}, {}{notes}",
                    known.name,
                    metadata.move_type,
                    match metadata.power {
                        Some(power) => format!("{power} power"),
                        None => "no damage".to_string(),
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

    fn names() -> Vec<&'static str> {
        catalogue().into_iter().map(|tool| tool.function.name).collect()
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

        let answer = files::route_text(&state, &graph, Map::Route5);
        assert!(answer.starts_with("From CeruleanCity to Route5, as walked so far:"), "{answer}");
        assert!(answer.contains("cannot start it from where you are standing"), "{answer}");
        // The way between two parts of one map is a door.
        assert!(answer.contains("door"), "it has to name the way round: {answer}");
        assert!(answer.contains("map headers"), "and why the route disagrees with the menu: {answer}");

        // The reachable crossing is not flagged, or the flag says nothing.
        let route4 = files::route_text(&state, &graph, Map::Route4);
        assert!(!route4.contains("cannot start it"), "Route 4 is walkable from here: {route4}");
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

    /// Every opening into a neighbour is a row, and one that cannot be walked to is no row at all.
    #[test]
    fn every_opening_into_a_neighbour_is_a_row_of_its_own() {
        use crate::pokemon::tile::MetaTile;
        let mut state = state_from(include_bytes!("../pokemon/data/pocket-route14.bin"));
        state.map.player_position = Point8 { x: 10, y: 4 };

        let reachable: Vec<_> = state.map.crossings(Map::Route13).into_iter()
            .filter(|crossing| crossing.reachable).collect();
        assert!(reachable.len() > 1, "the road has several openings east: {reachable:?}");

        let menu = overworld_menu(&state, None);
        let rows: Vec<&MenuItem> = menu.iter().filter(|row| row.id.ends_with(":Connection")).collect();
        // This map borders Route 13, by three openings, and Route 15 by one.
        assert_eq!(rows.len(), reachable.len() + 1, "one row per opening, not per neighbour: {rows:?}");

        let (dx, dy) = crate::pokemon::map_header::strip_offset(Map::Route13);
        for crossing in reachable {
            let id = overworld_id(&state, &OverworldAction {
                map: state.map.map, key: crossing.key, destination: crossing.at,
                tile: MetaTile::Connection { to_map: Map::Route13, to_position: crossing.to_position },
                route: vec![],
            });
            let row = rows.iter().find(|row| row.id == id).unwrap_or_else(|| panic!("no row for {id}: {rows:?}"));
            // Openings into one map that come out in parts of it that cannot reach each other are
            // otherwise word for word the same row.
            let lands = format!("({}, {})", crossing.to_position.x + dx, crossing.to_position.y + dy);
            assert!(row.description.contains(&lands), "{} omits where it lands", row.description);
            let resolved = resolve_overworld(&state, &id).unwrap_or_else(|| panic!("{id} resolves"));
            assert_eq!(resolved.destination, crossing.at);
            assert!(!resolved.route.is_empty(), "{id} comes with a walk to it");
        }

        // An unreachable crossing is neither offered nor resolved.
        let pocket = state.map.crossings(Map::Route13).into_iter()
            .find(|crossing| !crossing.reachable).expect("the pocket rows are not reachable from the road");
        let id = overworld_id(&state, &OverworldAction {
            map: state.map.map, key: pocket.key, destination: pocket.at,
            tile: MetaTile::Connection { to_map: Map::Route13, to_position: pocket.to_position },
            route: vec![],
        });
        assert!(!rows.iter().any(|row| row.id == id), "{id} cannot be walked to, so it is no row");
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
    fn every_file_is_readable_on_every_turn() {
        for kind in KINDS {
            for path in ["map.png", "party.md", "bag.md", "pc.md", "guide/current.md", "routes.md", "routes/PewterCity.md"] {
                assert!(matches!(classify(kind, &call(files::READ_FILE, &json!({ "path": path }).to_string()), &[]),
                                 CallKind::Read(_)), "{kind:?} cannot read {path}");
            }
            for path in ["plan.md", "battle-script.rhai", "battle-script.md", "issues/", "guide/", "nowhere.md"] {
                assert!(matches!(classify(kind, &call(files::READ_FILE, &json!({ "path": path }).to_string()), &[]),
                                 CallKind::File(FileCall::Read(_))), "{kind:?} cannot read {path}");
            }
            assert!(matches!(classify(kind, &call(files::READ_FILE, r#"{"path":"screen.png"}"#), &[]), CallKind::Screenshot));
            // And the writable ones are writable everywhere, a battle script mid-battle included.
            assert!(matches!(classify(kind, &call(files::WRITE_FILE, r#"{"path":"battle-script.rhai","content":"// x"}"#), &[]),
                             CallKind::File(FileCall::Write { .. })));
        }
    }

    #[test]
    fn only_a_storage_pc_is_offered_and_the_bedroom_s_holds_items() {
        assert_eq!(usable_pc(Map::FuchsiaPokecenter), Some((Point8 { x: 13, y: 3 }, true)));
        assert_eq!(usable_pc(Map::RedsHouse2F), Some((Point8 { x: 0, y: 1 }, false)));
        assert_eq!(usable_pc(Map::BillsHouse), None, "Bill's PC is his cell separator");
    }

    #[test]
    fn the_pc_and_the_lift_are_rows_a_model_can_name() {
        use crate::pokemon::postgame::item_storage::PcItemOp;
        use crate::pokemon::postgame::pc_box::PcBoxOp;
        let state = GameState::default();
        let request = |id: &str| field_request(&state, id, Some(&[])).expect("a field row").expect("parses");

        assert_eq!(request("pc:deposit@2"), FieldMoveRequest::UsePcBox { op: PcBoxOp::Deposit { slot: 2 } });
        assert_eq!(request("pc:withdraw@7"), FieldMoveRequest::UsePcBox { op: PcBoxOp::Withdraw { box_slot: 7 } });
        // 1-based in the id, as the cartridge's CHANGE BOX menu counts, and 0-based inside.
        assert_eq!(request("pc:box*1"), FieldMoveRequest::UsePcBox { op: PcBoxOp::ChangeBox { n: 0 } });
        assert_eq!(request("pc:take:Potion*3"),
                   FieldMoveRequest::UseItemPc { op: PcItemOp::Withdraw, item: ItemId::Potion, qty: 3 });
        // A count nobody gave is one.
        assert_eq!(request("pc:store:Potion"),
                   FieldMoveRequest::UseItemPc { op: PcItemOp::Deposit, item: ItemId::Potion, qty: 1 });
        assert_eq!(request("toss:Antidote*3"), FieldMoveRequest::TossItem { item: ItemId::Antidote, count: 3 });
        assert_eq!(request("lift:SilphCo5F"), FieldMoveRequest::UseElevator { to: Map::SilphCo5F });
        assert_eq!(request("item:RareCandy@1:stay"),
                   FieldMoveRequest::UseItem { item: ItemId::RareCandy, target: None, slot: Some(1), evolve: false });
        assert!(field_request(&state, "PalletTown:5,5:Warp", Some(&[])).is_none(), "a walk is not a field row");

        // A box that does not exist is named rather than falling through to a wrong default.
        let bad = field_request(&state, "pc:box*13", Some(&[])).expect("a field row").expect_err("refused");
        assert!(bad.contains("1 to 12"), "{bad}");
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

        // And every one of them still resolves, a slot row with any slot it names.
        for row in menu.iter().filter(|row| !row.id.contains('<')) {
            assert!(matches!(resolve_row(&state, &row.id), Row::Walk(_) | Row::Field(Ok(_))),
                    "`{}` is offered but does not resolve", row.id);
        }
    }

    const KINDS: [DecisionKind; 7] = ALL_KINDS;

    /// A menu row carries what its id cannot, and nothing else.
    #[test]
    fn the_warp_the_player_came_in_by_says_so() {
        use crate::pokemon::world_graph::Arrival;
        let state = fixture_state();
        let door = poke_core::geometry::Point8 { x: 5, y: 11 };
        let marked = overworld_menu(&state, Some(Arrival { map: state.map.map, at: door, from: Some(Map::PalletTown) }));
        let row = |menu: &[MenuItem], id: &str| menu.iter().find(|m| m.id == id).expect(id).description.clone();
        assert_eq!(row(&marked, "OaksLab:4,11:Warp"), "take the warp to PalletTown, arriving at (12, 12); the way you came in");
        assert_eq!(marked.iter().filter(|m| m.description.contains("came in")).count(), 1, "{marked:#?}");
        // Through the other half of the same door.
        let other = overworld_menu(&state, Some(Arrival { map: state.map.map, at: poke_core::geometry::Point8 { x: 4, y: 11 }, from: Some(Map::PalletTown) }));
        assert_eq!(row(&other, "OaksLab:4,11:Warp"), row(&marked, "OaksLab:4,11:Warp"));

        // An arrival on some other map says nothing about this one.
        let elsewhere = overworld_menu(&state, Some(Arrival { map: Map::PalletTown, at: door, from: None }));
        assert_eq!(row(&elsewhere, "OaksLab:4,11:Warp"), "take the warp to PalletTown, arriving at (12, 12)");
    }

    /// A row says what choosing it does, as a verb phrase; the id is a key, not a description.
    #[test]
    fn a_menu_row_explains_the_action_in_words() {
        let state = fixture_state();
        let menu = overworld_menu(&state, None);
        let rows: Vec<String> =
            menu.iter().map(|item| format!("- `{}` — {}", item.id, item.description)).collect();

        assert!(rows.contains(&"- `OaksLab:4,11:Warp` — take the warp to PalletTown, arriving at (12, 12)".to_string()), "{rows:#?}");
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
        let out = menu.iter().find(|m| m.id == "OaksLab:4,11:Warp").expect("the lab's door").description.clone();
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

    /// `bag.md` has to agree with the bag the game is holding, row for row.
    #[test]
    fn the_bag_file_counts_every_slot_the_game_counts() {
        use crate::pokemon::PokemonApiTrait;
        use crate::pokemon::symbols::DmgPointerRead;
        let mut gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(include_bytes!("../pokemon/data/post-ss-anne.bin")).expect("the committed fixture loads");
        let api = crate::pokemon::PokemonApi::new(&mut gb);
        let state = api.game_state().expect("readable");
        let view = crate::pokemon::observe::bag(&state, &api);

        let raw = api.mmu().read_pointer(&crate::pokemon::symbols::pokered_symbols::wNumBagItems) as usize;
        assert_eq!(view.slots_used, raw, "bag.md says {} of the bag's slots are used, the game says {raw}", view.slots_used);
        assert_eq!(view.items.len(), raw, "and every one of them has to be listed: {:?}", view.items);

        // What the file says, a line per item after the money and the slots.
        let text = files::bag_text(&state, &api);
        assert!(text.contains(&format!("Slots: {raw} of 20 used")), "{text}");
        assert_eq!(text.lines().filter(|line| line.contains(" ×")).count(), raw, "{text}");

        // A machine's name is one the model can quote back into `toss:` or `teach:`.
        let named: Vec<&str> = view.items.iter().map(|i| i.item.as_str()).collect();
        assert!(named.contains(&"Tm34Bide"), "{named:?}");
        assert!(named.contains(&"Hm01Cut"), "{named:?}");
        for name in &named {
            assert!(item_by_name(name).is_some(), "{name} is listed but cannot be named back");
        }

        // Every machine id, which this fixture alone cannot cover.
        for id in 0xC4..=0xFAu8 {
            let item = ItemId::from_repr(id)
                .unwrap_or_else(|| panic!("${id:02X} is a machine and `bag.md` would drop it"));
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

        // An Ether asks which move as well.
        let spent: Vec<String> = ["fight:Peck", "item:Ether@0:1"].iter().map(|id| id.to_string()).collect();
        assert!(battle_rule_behind("item:Ether@0", &spent).contains("`item:Ether@0:1`"), "the move was not asked for");
        assert!(battle_rule_behind("item:Ether@0:0", &spent).contains("It won't have any effect."));

        let here: Vec<String> = vec!["ViridianCity:18,6:Sprite".to_string()];
        let stale = not_on_the_menu("ViridianForest:17,47:Warp", &here)
            .expect("an id from another map is not on this menu");
        assert!(stale.contains("ids are minted for the map"), "the map clause went missing: {stale}");
        // A prefix that is not a map is not reported as one — `foo` is a typo, not a place.
        let typo = not_on_the_menu("foo:bar", &here).expect("not on this menu either");
        assert!(!typo.contains("ids are minted for the map"), "`foo` was called a map: {typo}");
    }

    /// What the `tools` array costs, with a ceiling.
    #[test]
    fn the_tool_array_stays_within_its_budget() {
        const CEILING: usize = 3_900;
        let bytes = serde_json::to_string(&catalogue()).expect("the specs serialise").len();
        assert!(bytes <= CEILING, "the tools are {bytes} bytes, over the {CEILING} budget");
    }

    #[test]
    fn no_tool_text_carries_a_run_of_spaces() {
        fn walk(value: &Value, path: &str) {
            match value {
                Value::String(text) => assert!(
                    !text.contains("  "),
                    "{path} has a run of spaces, a literal continued without `\\`: {text:?}",
                ),
                Value::Array(items) => items.iter().enumerate().for_each(|(i, item)| walk(item, &format!("{path}[{i}]"))),
                Value::Object(fields) => fields.iter().for_each(|(key, field)| walk(field, &format!("{path}.{key}"))),
                _ => {}
            }
        }
        walk(&serde_json::to_value(catalogue()).expect("the specs serialise"), "tools");
    }

    /// `routes.md` and `routes/<Map>.md`, each a different next step for the model.
    #[test]
    fn a_route_answers_the_four_questions_and_never_bluffs() {
        let state = fixture_state();
        let graph = WorldGraph::new();

        // What has been walked.
        let walked = files::routes_text(&state, &graph);
        assert!(walked.starts_with(&format!("You are on {}.", state.map.map)), "{walked}");
        assert_eq!(files::resolve("routes/"), files::Node::Routes, "the directory is the same list");

        // A name that is not a map at all: correctable, and it says how.
        let nonsense = files::no_such_map("routes/Kanto Safari Wildlife Park.md");
        assert!(nonsense.contains("no map called"), "{nonsense}");
        assert!(nonsense.contains("routes.md"), "{nonsense}");

        // A real map nobody has walked to: not been there, never unreachable.
        let unwalked = files::route_text(&state, &graph, Map::CeruleanCity);
        assert!(unwalked.contains("not been to CeruleanCity"), "{unwalked}");
        assert!(!unwalked.contains("unreachable"), "{unwalked}");

        // `map_by_name` normalises the way a model spells a map.
        assert_eq!(files::resolve("routes/cerulean city"), files::Node::Route(Map::CeruleanCity));
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

    use poke_core::ruleset::Ruleset::{Gen1, Modern};

    #[test]
    fn a_fight_row_is_costed_against_the_pokemon_in_front_of_it() {
        use crate::pokemon::pokemon::PokemonType::*;
        use crate::pokemon::species::PokemonSpecies;

        let me = summary(PokemonSpecies::Charmander, [Fire, Fire],
                         &[PokemonMoveName::Ember, PokemonMoveName::Growl, PokemonMoveName::Scratch]);
        let foe = summary(PokemonSpecies::Bulbasaur, [Grass, Poison], &[PokemonMoveName::Tackle]);

        let ember = fight_row_note(PokemonMoveName::Ember, &me, &foe, Gen1);
        assert!(ember.contains("super effective"), "Fire on Grass is doubled: {ember}");
        assert!(ember.contains("damage") && ember.contains("% of its HP"),
                "a number and what share of the target it is: {ember}");

        // A status move gets no number, the `34 -> 34` rule.
        assert_eq!(fight_row_note(PokemonMoveName::Growl, &me, &foe, Gen1), "",
                   "a status move is not priced");

        // The multiplier is withheld from a status move as well.
        let ghost = summary(PokemonSpecies::Gastly, [Ghost, Poison], &[PokemonMoveName::Lick]);
        assert_eq!(fight_row_note(PokemonMoveName::Growl, &me, &ghost, Gen1), "",
                   "a status move is never labelled by the type chart");

        // A damaging move that genuinely cannot land says so, and says only that.
        let normal = summary(PokemonSpecies::Rattata, [Normal, Normal], &[PokemonMoveName::Tackle]);
        let row = fight_row_note(PokemonMoveName::Tackle, &normal, &ghost, Gen1);
        assert_eq!(row, " — no effect", "immunity is the whole row: {row}");
    }

    #[test]
    fn a_fight_row_says_what_its_move_does_by_the_battles_rules() {
        use crate::pokemon::pokemon::PokemonType::*;
        use crate::pokemon::species::PokemonSpecies;

        let me = summary(PokemonSpecies::Charmander, [Fire, Fire],
                         &[PokemonMoveName::Ember, PokemonMoveName::Scratch, PokemonMoveName::FocusEnergy]);
        let foe = summary(PokemonSpecies::Bulbasaur, [Grass, Poison], &[PokemonMoveName::Tackle]);

        let ember = fight_row_notes(PokemonMoveName::Ember, &me, &foe, Gen1);
        assert!(ember.starts_with(&fight_row_note(PokemonMoveName::Ember, &me, &foe, Gen1)), "the estimate first: {ember}");
        assert!(ember.contains("super effective. 10% chance of burning the opponent."), "{ember}");

        let scratch = fight_row_notes(PokemonMoveName::Scratch, &me, &foe, Gen1);
        assert_eq!(scratch, fight_row_note(PokemonMoveName::Scratch, &me, &foe, Gen1),
                   "a plain attack says nothing more");
        assert!(!scratch.contains("additional"), "{scratch}");

        // A status move has no estimate, so its description is the whole note.
        let gen1 = fight_row_notes(PokemonMoveName::FocusEnergy, &me, &foe, Gen1);
        assert!(gen1.starts_with(" — ") && gen1.contains("quarters the critical-hit rate"), "{gen1}");
        let modern = fight_row_notes(PokemonMoveName::FocusEnergy, &me, &foe, Modern);
        assert_eq!(modern, " — Doubles the user's critical-hit rate.");
    }

    #[test]
    fn only_a_dual_type_is_doubly_super_effective() {
        use crate::pokemon::pokemon::PokemonType::*;
        use crate::pokemon::species::PokemonSpecies;

        let squirtle = summary(PokemonSpecies::Squirtle, [Water, Water], &[PokemonMoveName::WaterGun]);
        let charmander = summary(PokemonSpecies::Charmander, [Fire, Fire], &[PokemonMoveName::Ember]);
        let water_gun = fight_row_note(PokemonMoveName::WaterGun, &squirtle, &charmander, Gen1);
        assert!(water_gun.ends_with("), super effective"), "{water_gun}");

        let lapras = summary(PokemonSpecies::Lapras, [Water, Ice], &[PokemonMoveName::IceBeam]);
        let dragonite = summary(PokemonSpecies::Dragonite, [Dragon, Flying], &[PokemonMoveName::Wrap]);
        let ice_beam = fight_row_note(PokemonMoveName::IceBeam, &lapras, &dragonite, Gen1);
        assert!(ice_beam.ends_with("doubly super effective"), "{ice_beam}");
    }

    /// The emulated game's chart gives Ghost no effect on Psychic; a Modern native run's doubles it.
    #[test]
    fn lick_on_a_psychic_type_is_costed_by_the_ruleset() {
        use crate::pokemon::pokemon::PokemonType::*;
        use crate::pokemon::species::PokemonSpecies;

        let me = summary(PokemonSpecies::Gastly, [Ghost, Poison], &[PokemonMoveName::Lick]);
        let foe = summary(PokemonSpecies::Slowbro, [Water, Psychic], &[PokemonMoveName::Tackle]);
        assert_eq!(fight_row_note(PokemonMoveName::Lick, &me, &foe, Gen1), " — no effect");
        let modern = fight_row_note(PokemonMoveName::Lick, &me, &foe, Modern);
        assert!(modern.contains("damage") && modern.ends_with("super effective"), "{modern}");
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
        let rows = forget_menu(&moves, Gen1);
        assert!(rows[0].description.contains("Normal") && rows[0].description.contains("power"),
                "type and power: {}", rows[0].description);
        assert!(rows[1].description.contains("HM move"), "the HM is marked: {}", rows[1].description);
        assert!(!rows[0].description.contains("HM move"), "and only the HM is");
        assert!(rows[2].description.contains("no damage"),
                "a status move says so rather than showing 0 power: {}", rows[2].description);
    }

    #[test]
    fn a_forget_row_says_what_its_move_does_by_the_runs_rules() {
        let moves = [
            PokemonMove { name: PokemonMoveName::Tackle, pp: 35 },
            PokemonMove { name: PokemonMoveName::Cut, pp: 30 },
            PokemonMove { name: PokemonMoveName::Growl, pp: 40 },
            PokemonMove { name: PokemonMoveName::FocusEnergy, pp: 30 },
        ];
        let gen1 = forget_menu(&moves, Gen1);
        assert_eq!(gen1[0].description, "Tackle — Normal, 35 power, 35/35 pp", "a plain attack says nothing more");
        assert!(gen1[1].description.ends_with("30/30 pp — ⚠️ an HM move, and it cannot be re-learnt"),
                "the HM flag stays last: {}", gen1[1].description);
        assert!(gen1[2].description.ends_with(
            "no damage, 40/40 pp. Lowers the opponent's Attack by 1 stage. The enemy's copy misses an extra 25% of the time."),
            "{}", gen1[2].description);
        assert!(gen1[3].description.contains("quarters the critical-hit rate"), "{}", gen1[3].description);

        let modern = forget_menu(&moves, Modern);
        assert!(modern[2].description.ends_with("40/40 pp. Lowers the opponent's Attack by 1 stage."), "{}", modern[2].description);
        assert!(modern[3].description.ends_with("Doubles the user's critical-hit rate."), "{}", modern[3].description);
    }

    /// A PP item's row names the mon and, for an Ether, the move it restores, with the PP it has.
    #[test]
    fn a_pp_item_row_says_whose_move_it_restores() {
        let state = crate::llm::battle_script::scenarios::short_of_pp();
        let menu = battle_menu(&state);
        let row = |id: &str| menu.iter().find(|row| row.id == id).map(|row| row.description.as_str())
            .unwrap_or_else(|| panic!("no `{id}` among {:?}", menu.iter().map(|row| &row.id).collect::<Vec<_>>()));
        assert_eq!(row("item:Ether@0:1"), "ITEM   Ether ×1 on SPARKY the Charmander's Ember, 3/25 PP");
        assert_eq!(row("item:MaxEther@1:1"), "ITEM   MaxEther ×1 on SHELLY the Squirtle's Water Gun, 0/25 PP");
        assert_eq!(row("item:Elixer@0"),
                   "ITEM   Elixer ×1 on SPARKY the Charmander, every move: Scratch 35/35 PP, Ember 3/25 PP, Growl 40/40 PP, Leer 30/30 PP");
        assert!(resolve_battle(&state, "item:Ether@1:1").is_some_and(|action|
            matches!(action, BattleAction::UseItem { target: Some(1), target_move: Some(1), .. })));
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
        for tool in catalogue() {
            let has_summary = tool.function.parameters["properties"].get("summary").is_some();
            let required = tool.function.parameters["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|name| name == "summary"));
            match terminal_names().contains(&tool.function.name) {
                true => {
                    assert!(has_summary, "`{}` has no summary", tool.function.name);
                    // Only the first `choose_action` of a reply needs one, which `classify_reply`
                    // enforces and a schema cannot say.
                    assert_eq!(required, tool.function.name != CHOOSE_ACTION,
                               "`{}`", tool.function.name);
                }
                false => assert!(!has_summary, "`{}` is not a decision", tool.function.name),
            }
            // `additionalProperties: false`, so an undeclared argument is schema-invalid.
            assert_eq!(tool.function.parameters["additionalProperties"], json!(false),
                       "`{}` would accept an undeclared argument", tool.function.name);
        }
    }

    /// `call_summary` tolerates an absent summary; [`classify`] is what demands one.
    #[test]
    fn a_summary_is_read_off_the_call_and_never_demanded() {
        let call = |arguments: &str| bare(CHOOSE_ACTION, arguments);

        assert_eq!(
            call_summary(&call(r#"{"id": "wait*5", "summary": "  letting the battle text finish  "}"#)),
            Some("letting the battle text finish".to_string()),
            "trimmed, because it is printed on a page",
        );
        assert_eq!(call_summary(&call(r#"{"id": "wait*5"}"#)), None);
        assert_eq!(call_summary(&call(r#"{"id": "wait*5", "summary": "   "}"#)), None, "blank is absent");
        assert_eq!(call_summary(&call("not json")), None, "and a broken call is not a panic");

        // `maxLength` in a schema is a request.
        let long = "x".repeat(MAX_SUMMARY * 2);
        let capped = call_summary(&call(&format!(r#"{{"summary": "{long}"}}"#))).expect("present");
        assert_eq!(capped.chars().count(), MAX_SUMMARY);
    }

    /// The summary is the one note the model keeps about its own turn, and a press's headline.
    #[test]
    fn a_terminal_call_must_say_what_it_is_doing() {
        for (kind, id) in [
            (DecisionKind::Overworld, "PalletTown:5,6:Warp"),
            (DecisionKind::Battle, "fight:Tackle"),
            (DecisionKind::Nickname, "Bubbles"),
            (DecisionKind::MartPurchase, "leave"),
            (DecisionKind::ForgetMove, "forget:1"),
            (DecisionKind::MimicMove, "mimic:1"),
            (DecisionKind::Overworld, "wait*5"),
            (DecisionKind::Stuck, "press:a"),
        ] {
            let arguments = json!({ "id": id }).to_string();
            let CallKind::Rejected(complaint) = classify(kind, &bare(CHOOSE_ACTION, &arguments), &[]) else {
                panic!("{id} on a {kind:?} turn must be made to say what it is doing");
            };
            assert!(complaint.contains("summary") && complaint.contains(CHOOSE_ACTION), "{complaint}");
            // The same call with one goes through.
            assert!(
                matches!(classify(kind, &call(CHOOSE_ACTION, &arguments), &[]), CallKind::Terminal(_)),
                "{id} with a summary is fine",
            );
        }

        // One summary covers a reply: the later calls may leave it out, and the first may too.
        let menu = ["PalletTown:5,6:Warp".to_string(), "PalletTown:Mom".to_string()];
        for (first, second) in [(true, false), (false, true)] {
            let with = |id: &str, summary: bool| match summary {
                true => call(CHOOSE_ACTION, &json!({ "id": id }).to_string()),
                false => bare(CHOOSE_ACTION, &json!({ "id": id }).to_string()),
            };
            let reply = [with("PalletTown:5,6:Warp", first), with("PalletTown:Mom", second)];
            let classified = classify_reply(DecisionKind::Overworld, &reply, &menu);
            assert!(matches!(classified[0], CallKind::Terminal(_)) && matches!(classified[1], CallKind::Chained));
            assert_eq!(reply_summary(&reply, &classified).as_deref(), Some("doing the thing"));
        }
    }

    /// `take_over` is a row, offered only where the script handed the turn back, and goes with an action.
    #[test]
    fn take_over_is_a_row_sent_beside_the_turns_action() {
        let reply = |ids: &[&str]| -> Vec<ToolCall> {
            ids.iter().map(|id| call(CHOOSE_ACTION, &json!({ "id": id }).to_string())).collect()
        };
        let offered = ["fight:Tackle".to_string(), "run".to_string(), TAKE_OVER.to_string()];
        let plain = &offered[..2];

        let classified = classify_reply(DecisionKind::Battle, &reply(&["fight:Tackle"]), &offered);
        assert!(matches!(&classified[0],
            CallKind::Terminal(Terminal::ChooseBattleAction { take_over: false, id }) if id == "fight:Tackle"));

        for order in [[TAKE_OVER, "fight:Tackle"], ["fight:Tackle", TAKE_OVER]] {
            let classified = classify_reply(DecisionKind::Battle, &reply(&order), &offered);
            assert!(matches!(&classified[0],
                CallKind::Terminal(Terminal::ChooseBattleAction { take_over: true, id }) if id == "fight:Tackle"),
                "{order:?}");
            assert!(matches!(classified[1], CallKind::Chained), "{order:?}");
        }

        let classified = classify_reply(DecisionKind::Battle, &reply(&[TAKE_OVER, "fight:Tackle"]), plain);
        let CallKind::Rejected(complaint) = &classified[0] else { panic!("not offered, so refused") };
        assert!(complaint.contains("handed back"), "{complaint}");
        assert!(matches!(classified[1], CallKind::Rejected(_)), "and the action with it: all or none");

        let CallKind::Rejected(complaint) = &classify_reply(DecisionKind::Battle, &reply(&[TAKE_OVER]), &offered)[0]
        else { panic!("a take-over alone decides nothing") };
        assert!(complaint.contains("action"), "{complaint}");

        let classified = classify_reply(DecisionKind::Battle, &reply(&["fight:Tackle", "run"]), &offered);
        assert!(classified.iter().all(|kind| matches!(kind, CallKind::Rejected(_))), "one action a battle turn");
    }

    /// A write under `issues/`, on every turn.
    #[test]
    fn an_issue_report_does_not_end_the_turn_and_must_carry_a_message() {
        for kind in KINDS {
            let good = bare(files::WRITE_FILE, r#"{"path":"issues/Missing Ladder","content":"  the ladder is not in the menu  "}"#);
            let CallKind::Issue { name, message } = classify(kind, &good, &[]) else {
                panic!("{kind:?} should file it");
            };
            assert_eq!(name, "Missing Ladder.md", "the model's name, ending in `.md`");
            assert_eq!(message, "the ladder is not in the menu", "trimmed, like every other string");

            for arguments in [r#"{"path":"issues/x.md"}"#, r#"{"path":"issues/x.md","content":"   "}"#] {
                let CallKind::Rejected(_) = classify(kind, &bare(files::WRITE_FILE, arguments), &[]) else {
                    panic!("an empty report is not a report: {arguments}");
                };
            }
            // Capped, not refused.
            let long = json!({ "path": "issues/long.md", "content": "é".repeat(MAX_ISSUE * 2) }).to_string();
            let CallKind::Issue { message, .. } = classify(kind, &bare(files::WRITE_FILE, &long), &[]) else { panic!() };
            assert_eq!(message.chars().count(), MAX_ISSUE);
        }
    }

    /// The four tools, and the contract in the prompt names what is actually sent.
    #[test]
    fn the_catalogue_is_the_action_tool_and_the_three_file_tools() {
        assert_eq!(names(), [CHOOSE_ACTION, files::READ_FILE, files::WRITE_FILE, files::EDIT_FILE]);
        for name in terminal_names().iter().chain(&non_terminal_names()) {
            assert!(names().contains(name), "the prompt promises {name} but it is not offered");
        }
    }

    /// A file call with a broken path or missing argument is refused with what to send.
    #[test]
    fn a_file_call_is_parsed_or_complained_about() {
        let refused = |name: &str, arguments: &str| match classify(DecisionKind::Battle, &call(name, arguments), &[]) {
            CallKind::Rejected(complaint) => complaint,
            _ => panic!("`{name}` {arguments} should be refused"),
        };
        assert!(refused(files::READ_FILE, "{}").contains("map.png"), "a missing path answers the tree");
        assert!(refused(files::WRITE_FILE, r#"{"path":"plan.md"}"#).contains("content"));
        assert!(refused(files::EDIT_FILE, r#"{"path":"plan.md","old_string":"","new_string":"x"}"#).contains("empty"));
        assert!(refused(files::EDIT_FILE, r#"{"path":"plan.md","old_string":"x","new_string":"x"}"#).contains("same"));
        let CallKind::Read(read) = classify(DecisionKind::Battle, &call(files::READ_FILE, r#"{"path":"party.md","offset":3,"limit":2}"#), &[])
        else { panic!("a party read") };
        assert_eq!((read.offset, read.limit), (Some(3), Some(2)));
    }

    /// A malformed schema is a 400 on the first turn of a run.
    #[test]
    fn every_schema_is_a_well_formed_object() {
        for tool in catalogue() {
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

    /// Several calls in one reply are one chain, every id held to the same menu, and an over-long
    /// chain is refused, not cut.
    #[test]
    fn every_id_in_a_chain_is_held_to_the_menu_the_turn_offered() {
        let menu = ["PalletTown:5,6:Warp".to_string(), "PalletTown:Mom".to_string()];
        let chain = |ids: &[&str]| -> Vec<CallKind> {
            let reply: Vec<ToolCall> =
                ids.iter().map(|id| call(CHOOSE_ACTION, &json!({ "id": id }).to_string())).collect();
            classify_reply(DecisionKind::Overworld, &reply, &menu)
        };

        let classified = chain(&["PalletTown:5,6:Warp", "PalletTown:Mom"]);
        let CallKind::Terminal(Terminal::ChooseAction { id, then }) = &classified[0] else {
            panic!("a chain of two ids from the menu is an ordinary reply");
        };
        assert_eq!(id, "PalletTown:5,6:Warp");
        assert_eq!(then, &["PalletTown:Mom"]);
        assert!(matches!(classified[1], CallKind::Chained));

        // Each id is checked, not merely the count: a stale id is not let through by being second.
        let classified = chain(&["PalletTown:5,6:Warp", "OaksLab:4,11:Warp"]);
        let CallKind::Rejected(complaint) = &classified[1] else {
            panic!("a chained id that was never offered is refused");
        };
        assert!(complaint.contains("OaksLab:4,11:Warp"), "it names the one that failed: {complaint}");
        let CallKind::Rejected(complaint) = &classified[0] else { panic!("and the rest go with it") };
        assert!(complaint.contains("all or none"), "{complaint}");

        let classified = chain(&["PalletTown:Mom"; MAX_CHAINED_ACTIONS + 1]);
        let CallKind::Rejected(complaint) = &classified[0] else {
            panic!("a chain longer than the cap is refused");
        };
        assert!(complaint.contains(&MAX_CHAINED_ACTIONS.to_string()), "it says what the cap is: {complaint}");

        // A wait is a whole answer.
        assert!(chain(&["PalletTown:Mom", "wait*10"]).iter().all(|kind| matches!(kind, CallKind::Rejected(_))));

        // The single-question turns take one call.
        let reply = [call(CHOOSE_ACTION, r#"{"id":"forget:0"}"#), call(CHOOSE_ACTION, r#"{"id":"forget:1"}"#)];
        assert!(classify_reply(DecisionKind::ForgetMove, &reply, &[]).iter().all(|kind| matches!(kind, CallKind::Rejected(_))));
    }

    /// An older conversation's tool is answered with the one that replaced it.
    #[test]
    fn a_tool_from_an_older_conversation_is_rejected_with_the_one_that_replaced_it() {
        let CallKind::Rejected(complaint) =
            classify(DecisionKind::Battle, &call("choose_battle_action", r#"{"id":"run"}"#), &[])
        else {
            panic!("choose_battle_action is gone");
        };
        assert!(complaint.contains(CHOOSE_ACTION), "{complaint}");

        let CallKind::Rejected(complaint) =
            classify(DecisionKind::Overworld, &call("use_field_move", r#"{"move":"dig"}"#), &[])
        else {
            panic!("use_field_move is gone");
        };
        assert!(complaint.contains(CHOOSE_ACTION), "{complaint}");

        let CallKind::Rejected(complaint) = classify(DecisionKind::Overworld, &call("teleport", "{}"), &[]) else {
            panic!("an invented tool is rejected");
        };
        assert!(complaint.contains("teleport") && complaint.contains(files::READ_FILE), "{complaint}");

        // Every read, plan and script tool of an older conversation names the file that replaced it.
        for (old, file) in [("read_map", "map.png"), ("screenshot", "screen.png"), ("read_party", "party.md"),
                            ("read_bag", "bag.md"), ("read_pc", "pc.md"), ("read_guide", "guide/current.md"),
                            ("read_route", "routes"), ("todo_set", "plan.md"), ("todo_complete", "plan.md"),
                            ("todo_delete", "plan.md"), ("set_battle_script", "battle-script.rhai"),
                            ("read_battle_script", "battle-script.rhai"), ("get_battle_script_docs", "battle-script.md"),
                            ("report_issue", "issues/")] {
            let CallKind::Rejected(complaint) = classify(DecisionKind::Overworld, &call(old, "{}"), &[]) else {
                panic!("{old} is gone");
            };
            assert!(complaint.contains(file), "{old}: {complaint}");
        }
    }

    #[test]
    fn arguments_are_parsed_or_complained_about() {
        let chose = |kind, id: &str| classify(kind, &call(CHOOSE_ACTION, &json!({ "id": id }).to_string()), &[]);
        assert!(matches!(
            chose(DecisionKind::Overworld, "PalletTown:5,6:Warp"),
            CallKind::Terminal(Terminal::ChooseAction { ref id, .. }) if id == "PalletTown:5,6:Warp",
        ));
        assert!(matches!(chose(DecisionKind::Battle, "wait*25"), CallKind::Terminal(Terminal::Wait { ticks: 25 })));
        assert!(matches!(
            chose(DecisionKind::Battle, "wait"),
            CallKind::Terminal(Terminal::Wait { ticks: DEFAULT_WAIT_TICKS }),
        ));
        // A model asking to sit out ten minutes of game time is stalling its own run.
        assert!(matches!(
            chose(DecisionKind::Battle, "wait*99999"),
            CallKind::Terminal(Terminal::Wait { ticks: MAX_WAIT_TICKS }),
        ));
        assert!(matches!(chose(DecisionKind::Overworld, "wait*soon"), CallKind::Rejected(_)));
        // Only a mart order takes a count.
        let CallKind::Rejected(complaint) = chose(DecisionKind::Overworld, "PalletTown:5,6:Warp*3") else {
            panic!("a walk has no count");
        };
        assert!(complaint.contains("*N"), "{complaint}");
        assert!(matches!(chose(DecisionKind::Overworld, ""), CallKind::Rejected(_)));
        assert!(matches!(
            classify(DecisionKind::Overworld, &call(CHOOSE_ACTION, "not json at all"), &[]),
            CallKind::Rejected(_),
        ));
        // A read with no arguments at all asks for nothing, and is told what there is.
        let CallKind::Rejected(complaint) = classify(DecisionKind::Overworld, &call(files::READ_FILE, ""), &[]) else {
            panic!("no path")
        };
        assert!(complaint.contains("party.md"), "{complaint}");
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

    /// A model writes `"HM01 Cut"`, the code writes `Hm01Cut`, and `bag.md` writes `Hm01Cut`.
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
        let chose = |id: &str| call(CHOOSE_ACTION, &format!(r#"{{"id":"{id}","summary":"s"}}"#));

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
    fn a_press_parses_a_sequence_on_the_stuck_turn_and_nowhere_else() {
        let press = |id: &str| call(CHOOSE_ACTION, &json!({ "id": id }).to_string());
        let CallKind::Terminal(Terminal::PressButtons { buttons }) =
            classify(DecisionKind::Stuck, &press("press:b,b,start"), &[])
        else {
            panic!("a list of real buttons is a decision");
        };
        assert_eq!(buttons, [JoypadButton::B, JoypadButton::B, JoypadButton::Start]);

        for bad in ["press:b,x", "press:", "b,a", "Route1:Grass"] {
            assert!(
                matches!(classify(DecisionKind::Stuck, &press(bad), &[]), CallKind::Rejected(_)),
                "{bad} should have been rejected",
            );
        }
        // The agent's queue capacity is the cap, so a runaway list is trimmed, not half-delivered.
        let many = format!("press:{}", vec!["a"; MANUAL_INPUT_CAPACITY * 2].join(","));
        let CallKind::Terminal(Terminal::PressButtons { buttons }) = classify(DecisionKind::Stuck, &press(&many), &[])
        else {
            panic!("an over-long list is still a decision");
        };
        assert_eq!(buttons.len(), MANUAL_INPUT_CAPACITY);

        // A raw press never appears on a turn with a menu.
        for kind in [DecisionKind::Overworld, DecisionKind::Battle, DecisionKind::MartPurchase] {
            let CallKind::Rejected(complaint) = classify(kind, &press("press:a"), &[]) else {
                panic!("{kind:?} must refuse a press");
            };
            assert!(complaint.contains("the agent presses the buttons"), "{complaint}");
        }
    }

    /// `screen.png` is a read to the model, but the worker answers it.
    #[test]
    fn a_screenshot_is_classified_apart_from_the_other_reads() {
        assert!(matches!(classify(DecisionKind::Battle, &call(files::READ_FILE, r#"{"path":"screen.png"}"#), &[]), CallKind::Screenshot));
        assert!(matches!(classify(DecisionKind::Battle, &call(files::READ_FILE, r#"{"path":"party.md"}"#), &[]), CallKind::Read(_)));
    }

    /// Every id shape a field row takes, and the ids a template refuses.
    #[test]
    fn a_field_row_parses_into_the_move_it_names() {
        let state = GameState::default();
        let request = |id: &str| field_request(&state, id, Some(&[])).expect("a field row").expect("parses");
        assert_eq!(request("flash"), FieldMoveRequest::PartyMove { name: PokemonMoveName::Flash, slot: None },
                   "a party move is whoever knows it, not slot 0");
        assert_eq!(request("fly:PewterCity"), FieldMoveRequest::Fly { to: Map::PewterCity });
        assert_eq!(request("teach:Hm03Surf@0"), FieldMoveRequest::Teach { item: ItemId::Hm03Surf, slot: 0 });
        assert_eq!(request("evolve:MoonStone@4"), FieldMoveRequest::Evolve { stone: ItemId::MoonStone, slot: 4 });
        assert_eq!(request("item:Bicycle"),
                   FieldMoveRequest::UseItem { item: ItemId::Bicycle, target: None, slot: None, evolve: true });
        assert_eq!(request("item:Potion@1"),
                   FieldMoveRequest::UseItem { item: ItemId::Potion, target: None, slot: Some(1), evolve: true });
        assert_eq!(request("lead:3"), FieldMoveRequest::ReorderParty { slot: 3 });
        assert_eq!(request("daycare:deposit@2"), FieldMoveRequest::DayCare { deposit: Some(2) });

        // A slot row's template takes any party slot in its place, and checks the rest of the id.
        let menu: Vec<String> = ["item:Potion@<slot>", "toss:<item>*N", "pc:withdraw@<box slot>", "pc:box*N"]
            .iter().map(|id| id.to_string()).collect();
        let chose = |id: &str| classify(DecisionKind::Overworld,
            &call(CHOOSE_ACTION, &json!({ "id": id, "summary": "s" }).to_string()), &menu);
        for (sent, taken) in [("item:Potion@2", "item:Potion@2"), ("toss:Antidote*3", "toss:Antidote*3"),
                              ("toss:Antidote", "toss:Antidote"), ("pc:withdraw@19", "pc:withdraw@19"),
                              ("pc:box*4", "pc:box*4")] {
            assert!(matches!(chose(sent), CallKind::Terminal(Terminal::ChooseAction { ref id, .. }) if id == taken),
                    "{sent} was not taken as {taken}");
        }
        for refused in ["item:Potion@6", "item:Potion", "item:Ether@1", "toss:Sandwich*2", "pc:withdraw@20",
                        "item:Potion@1*2", "lead:1"] {
            assert!(matches!(chose(refused), CallKind::Rejected(_)), "{refused} should have been refused");
        }
    }

    /// The menu prompts' ids: a mart order list, the forget rows and Mimic's.
    #[test]
    fn the_menu_prompts_answer_with_their_own_rows() {
        let reply = |kind, menu: &[String], ids: &[&str]| -> Vec<CallKind> {
            let calls: Vec<ToolCall> =
                ids.iter().map(|id| call(CHOOSE_ACTION, &json!({ "id": id }).to_string())).collect();
            classify_reply(kind, &calls, menu)
        };
        let shop: Vec<String> = ["buy:Potion", "buy:PokeBall", "buy:Antidote", "leave"].map(String::from).to_vec();
        let mart = |ids: &[&str]| reply(DecisionKind::MartPurchase, &shop, ids);

        assert!(matches!(&mart(&["leave"])[0], CallKind::Terminal(Terminal::BuyItem { item: None, then }) if then.is_empty()));
        // An omitted count is one: the mart silently refuses zero.
        assert!(matches!(&mart(&["buy:Potion"])[0],
            CallKind::Terminal(Terminal::BuyItem { item: Some(BagItem { id: ItemId::Potion, quantity: 1 }), .. })));
        let classified = mart(&["buy:Potion*4", "buy:PokeBall*10", "leave"]);
        let CallKind::Terminal(Terminal::BuyItem { item, then }) = &classified[0] else {
            panic!("an order list is a purchase: {:?}", classified.iter().map(CallKind::label).collect::<Vec<_>>());
        };
        assert_eq!(*item, Some(BagItem::new(ItemId::Potion, 4)));
        assert_eq!(*then, vec![BagItem::new(ItemId::PokeBall, 10)], "a trailing `leave` is what happens anyway");
        assert!(matches!(&mart(&["buy:Potion*500"])[0],
            CallKind::Terminal(Terminal::BuyItem { item: Some(BagItem { quantity: 99, .. }), .. })));
        assert!(mart(&["buy:Potion", "buy:Nonsense"]).iter().all(|kind| matches!(kind, CallKind::Rejected(_))),
                "a name in the tail is checked exactly as the head is");
        assert!(mart(&["leave", "buy:Potion"]).iter().all(|kind| matches!(kind, CallKind::Rejected(_))));
        assert!(mart(&["buy:Potion"; MAX_CHAINED_ACTIONS + 1]).iter().all(|kind| matches!(kind, CallKind::Rejected(_))),
                "over-length is a rejection, not a truncation");

        let forget: Vec<String> = ["forget:0", "forget:1", "forget:2", "forget:3", "keep"].map(String::from).to_vec();
        assert!(matches!(reply(DecisionKind::ForgetMove, &forget, &["keep"])[0],
            CallKind::Terminal(Terminal::ForgetMove { slot: None })));
        assert!(matches!(reply(DecisionKind::ForgetMove, &forget, &["forget:2"])[0],
            CallKind::Terminal(Terminal::ForgetMove { slot: Some(2) })));
        assert!(matches!(reply(DecisionKind::ForgetMove, &forget, &["forget:7"])[0], CallKind::Rejected(_)),
                "a Pokémon has four move slots, and a cursor sent to a fifth never arrives");
        assert!(matches!(reply(DecisionKind::ForgetMove, &forget, &["2"])[0], CallKind::Rejected(_)));

        let mimic: Vec<String> = ["mimic:0", "mimic:1"].map(String::from).to_vec();
        assert!(matches!(reply(DecisionKind::MimicMove, &mimic, &["mimic:1"])[0],
            CallKind::Terminal(Terminal::MimicMove { slot: 1 })));
        assert!(matches!(reply(DecisionKind::MimicMove, &mimic, &["keep"])[0], CallKind::Rejected(_)),
                "Mimic's menu has no B, so there is nothing to decline");
        assert!(matches!(reply(DecisionKind::MimicMove, &[], &["mimic:4"])[0], CallKind::Rejected(_)));
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
        assert!(complaint(FieldMoveRequest::TossItem { item: ItemId::Hm01Cut, count: 1 }).contains("no Hm01Cut in the bag"));

        // Fly needs its HM, its badge, to be outside and to have been to the town.
        let mut flier = fixture_state();
        flier.badges |= crate::pokemon::badge::Badge::ThunderBadge;
        flier.map.tileset = crate::pokemon::map_header::TileSetId::Overworld;
        flier.towns_visited = 1 << Map::PalletTown as u8;
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

    /// The game's own refusals are `resolve_field_move`'s, so a row it refuses is never offered.
    #[test]
    fn a_use_the_game_would_refuse_is_refused_in_the_game_s_terms() {
        use crate::pokemon::bag::BagItem;
        use crate::pokemon::map_header::TileSetId;
        let mut state = fixture_state();
        for item in [ItemId::OaksParcel, ItemId::MoonStone, ItemId::Tm11Bubblebeam, ItemId::EscapeRope, ItemId::XAttack] {
            state.bag.push(BagItem { id: item, quantity: 1 }).expect("room");
        }
        let starter = state.pokemon.get_mut(0).expect("the fixture has a starter");
        starter.moves[1] = Some(PokemonMove::with_max_pp(PokemonMoveName::Dig));
        starter.moves[2] = Some(PokemonMove::with_max_pp(PokemonMoveName::Teleport));
        starter.moves[3] = Some(PokemonMove::with_max_pp(PokemonMoveName::Bubblebeam));
        let refusal = |state: &GameState, id: &str| match field_request(state, id, Some(&[])) {
            Some(Ok(request)) => resolve_field_move(state, &request).err(),
            other => panic!("{id} did not parse: {other:?}"),
        };

        assert!(refusal(&state, "toss:OaksParcel").is_some_and(|why| why.contains("too important to toss")));
        assert!(refusal(&state, "evolve:MoonStone@0").is_some_and(|why| why.contains("won't have any effect")));
        assert!(refusal(&state, "teach:Tm11Bubblebeam@0").is_some_and(|why| why.contains("already knows")));
        assert!(refusal(&state, "item:XAttack@0").is_some_and(|why| why.contains("only does anything in a battle")));
        // Oak's lab is neither a dungeon nor outside.
        assert!(refusal(&state, "dig").is_some_and(|why| why.contains("dungeon")));
        assert!(refusal(&state, "item:EscapeRope").is_some_and(|why| why.contains("dungeon")));
        assert!(refusal(&state, "teleport").is_some_and(|why| why.contains("only works outside")));

        state.map.tileset = TileSetId::Cavern;
        assert_eq!(refusal(&state, "dig"), None, "a cave is dug out of");
        assert_eq!(refusal(&state, "item:EscapeRope"), None);
        state.map.tileset = TileSetId::Overworld;
        assert_eq!(refusal(&state, "teleport"), None, "a route is teleported from");

        // And a row is generated for exactly the uses that resolve.
        let ids: Vec<String> = field_rows(&state, &[]).into_iter().map(|row| row.id).collect();
        assert!(ids.contains(&"teleport".to_string()) && !ids.contains(&"dig".to_string()), "{ids:?}");
        assert!(!ids.iter().any(|id| id.starts_with("evolve:") || id.starts_with("teach:") || id.starts_with("item:XAttack")),
                "{ids:?}");
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
            match resolve_field_move(state, &FieldMoveRequest::UseItem { item, target: Some(at), slot: None, evolve: true }) {
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
        assert!(ball.contains("`item:` row"), "{ball}");

        // A machine never reaches the table (`ItemUseTMHM`), so it is pointed at `teach`.
        let machine = complaint(&holding(ItemId::Hm01Cut), ItemId::Hm01Cut);
        assert!(machine.contains("teach"), "{machine}");

        // Not "refuse every key item": the Poké Flute is one, and the scripted route uses it.
        assert!(ItemId::PokeFlute.is_key_item(), "the point of the case");
        assert_eq!(
            resolve_field_move(&holding(ItemId::PokeFlute), &FieldMoveRequest::UseItem { item: ItemId::PokeFlute, target: Some(at), slot: None, evolve: true }),
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

        let ride = FieldMoveRequest::UseItem { item: ItemId::Bicycle, target: None, slot: None, evolve: true };
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

        let ride = FieldMoveRequest::UseItem { item: ItemId::Bicycle, target: None, slot: None, evolve: true };
        let refusal = resolve_field_move(&state, &ride).expect_err("a bike on water is refused");
        assert!(refusal.contains("surfing"), "{refusal}");
    }

    /// A field item aimed at nothing is refused, and told what is beside it.
    #[test]
    fn a_field_item_aimed_at_open_ground_is_refused_and_says_what_is_beside_it() {
        let mut state = fixture_state();
        state.bag.push(BagItem { id: ItemId::PokeFlute, quantity: 1 }).expect("room in the bag");
        let flute = |target| resolve_field_move(&state, &FieldMoveRequest::UseItem {
            item: ItemId::PokeFlute, target: Some(target), slot: None, evolve: true });

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

    /// The naming turn's id is the name, and takes only names the cartridge can write.
    #[test]
    fn a_nickname_is_the_id_and_has_to_be_one_the_game_can_write() {
        let named = |name: &str| classify(DecisionKind::Nickname, &call(CHOOSE_ACTION, &json!({ "id": name }).to_string()), &[]);
        assert!(matches!(named("Rocky"), CallKind::Terminal(Terminal::SetNickname { name: Some(ref name) }) if name == "Rocky"));
        // `/` is `$F3` and a space `$7F`, so the check asks the charmap, not "is it alphanumeric".
        assert!(matches!(named("MT/MOON"), CallKind::Terminal(Terminal::SetNickname { name: Some(_) })));
        assert!(
            matches!(named("Pokè"), CallKind::Rejected(_)),
            "a letter with no byte in the charmap must not reach the buffer",
        );
        assert!(matches!(named("🔥"), CallKind::Rejected(_)));
        let CallKind::Rejected(complaint) = named("ABCDEFGHIJKLMNOP") else { panic!("too long is refused, not cut") };
        assert!(complaint.contains(&MAX_NICKNAME.to_string()), "{complaint}");
        // `NONE` in any case keeps the species name, as the naming screen reads an empty buffer.
        for none in ["NONE", "none", "None"] {
            assert!(matches!(named(none), CallKind::Terminal(Terminal::SetNickname { name: None })), "{none}");
        }
        // `wait` is still a wait, a `*` being no letter a name can hold.
        assert!(matches!(named("wait*10"), CallKind::Terminal(Terminal::Wait { ticks: 10 })));
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


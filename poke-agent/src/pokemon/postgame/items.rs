//! Bag items used from the START menu. The Bicycle skips USE/TOSS and closes the menus, an
//! `UsableItems_CloseMenu` item returns to the overworld, and everything else drops back into the
//! bag, after a party menu for `UsableItems_PartyMenu` and a move menu for the PP items.

use gb::joypad::JoypadButton;
use gb::mmu::MMU;
use crate::pokemon::agent::{start_menu_row, AgentEvent, AgentState, PokemonAgent, StartMenuRow};
use crate::pokemon::encoding::GameMode;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
use crate::pokemon::menu::{TextBoxId, START_MENU_ORIGIN};
use crate::pokemon::policy::{FieldMove, PolicyStep};
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::{GameState, PokemonApi, PokemonApiTrait};

/// What a bag item is used on, which is how many menus follow `USE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseTarget {
    Nothing,
    Party { slot: u8 },
    Move { slot: u8, move_index: u8 },
}

impl UseTarget {
    pub const fn slot(self) -> Option<u8> {
        match self {
            Self::Nothing => None,
            Self::Party { slot } | Self::Move { slot, .. } => Some(slot),
        }
    }
}

/// Where an item's effect shows up, which is also where the menus leave you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Consumed,
    /// `ItemUseBicycle` toggles, so completion is "it changed", not "it is 1".
    TogglesBicycle,
    /// A text box and nothing else.
    OneShot,
}

pub const fn effect(item: ItemId) -> Effect {
    match item {
        ItemId::Bicycle => Effect::TogglesBicycle,
        ItemId::Itemfinder => Effect::OneShot,
        _ => Effect::Consumed,
    }
}

/// `wRepelRemainingSteps`, which ticks down one per overworld step.
pub fn repel_steps(mmu: &MMU) -> u8 {
    mmu.read_pointer(&pokered_symbols::wRepelRemainingSteps)
}

/// `wWalkBikeSurfState` is 0 walking, 1 cycling, 2 surfing.
pub fn on_bicycle(mmu: &MMU) -> bool {
    mmu.read_pointer(&pokered_symbols::wWalkBikeSurfState) == 1
}

/// `PokemonMove::pp` is the raw byte: the PP Up count in bits 6–7, the PP in bits 0–5.
pub const PP_MASK: u8 = 0b0011_1111;

pub fn move_pp(mv: &crate::pokemon::move_name::PokemonMove) -> u8 { mv.pp & PP_MASK }

pub fn pp_ups(mv: &crate::pokemon::move_name::PokemonMove) -> u8 { mv.pp >> 6 }

/// `mv`'s maximum PP: `AddBonusPP` gives `base / 5` per PP Up.
pub fn max_pp(mv: &crate::pokemon::move_name::PokemonMove) -> u8 {
    let base = mv.name.metadata().pp;
    base + (base / 5) * pp_ups(mv)
}

pub fn bag_quantity(state: &GameState, item: ItemId) -> u8 {
    state.bag.iter().find(|i| i.id == item).map_or(0, |i| i.quantity)
}

/// The value [`goal_met`] compares against, captured when the step begins.
pub fn baseline(state: &GameState, item: ItemId) -> u8 {
    match effect(item) {
        Effect::TogglesBicycle => state.on_bicycle as u8,
        _ => bag_quantity(state, item),
    }
}

/// Has the use landed? `attempts` is how many times the driver has been handed the job.
pub fn goal_met(state: &GameState, item: ItemId, baseline: u8, attempts: u32) -> bool {
    match effect(item) {
        Effect::Consumed => bag_quantity(state, item) < baseline,
        Effect::TogglesBicycle => state.on_bicycle as u8 != baseline,
        Effect::OneShot => attempts > 0,
    }
}

/// Why the game would refuse this use.
pub fn blocked(state: &GameState, item: ItemId, target: UseTarget) -> Option<String> {
    if bag_quantity(state, item) == 0 {
        return Some(format!("{item:?} is not in the bag"));
    }
    let mon = target.slot().and_then(|s| state.pokemon.get(s as usize));
    match item {
        // `.healHP`: a Revive wants a fainted target and a potion a live, damaged one.
        ItemId::Revive | ItemId::MaxRevive => match mon {
            Some(p) if p.current_hp > 0 => Some(format!("slot {:?} has not fainted", target.slot())),
            _ => None,
        },
        ItemId::Potion | ItemId::SuperPotion | ItemId::HyperPotion | ItemId::MaxPotion
        | ItemId::FullRestore | ItemId::FreshWater | ItemId::SodaPop | ItemId::Lemonade => match mon {
            Some(p) if p.current_hp == 0 => Some("a potion cannot revive a fainted mon".into()),
            Some(p) if p.current_hp >= p.stats.hp => Some("already at full HP".into()),
            _ => None,
        },
        // `.cureStatusAilment`: the item's own status must be set; a Full Heal takes any.
        ItemId::Antidote | ItemId::BurnHeal | ItemId::IceHeal | ItemId::Awakening
        | ItemId::ParlyzHeal | ItemId::FullHeal => match mon {
            Some(p) if !cures(item, p.status) => Some(format!("no {item:?}-curable status")),
            _ => None,
        },
        // `.useEther`: the chosen move must be missing PP.
        ItemId::Ether | ItemId::MaxEther => {
            let UseTarget::Move { move_index, .. } = target else {
                return Some("a PP restore needs a move target".into());
            };
            match mon.and_then(|p| p.moves.get(move_index as usize).and_then(|m| m.as_ref())) {
                None => Some(format!("slot {:?} has no move {move_index}", target.slot())),
                Some(mv) if move_pp(mv) >= max_pp(mv) =>
                    Some(format!("{:?} is already at full PP ({}/{})", mv.name, move_pp(mv), max_pp(mv))),
                _ => None,
            }
        }
        // An Elixer's precondition is the mon, not one move.
        ItemId::Elixer | ItemId::MaxElixer => {
            let Some(mon) = mon else {
                return Some("a PP restore needs a party target".into());
            };
            mon.moves.iter().flatten().any(|mv| move_pp(mv) < max_pp(mv))
                .then_some(())
                .map_or(Some(format!("every move on slot {:?} is already at full PP", target.slot())), |()| None)
        }
        // `.usePPUp`: three is the cap, and a fourth keeps the item.
        ItemId::PpUp => {
            let UseTarget::Move { move_index, .. } = target else {
                return Some("a PP Up needs a move target".into());
            };
            match mon.and_then(|p| p.moves.get(move_index as usize).and_then(|m| m.as_ref())) {
                None => Some(format!("slot {:?} has no move {move_index}", target.slot())),
                Some(mv) if pp_ups(mv) >= 3 => Some(format!("{:?} is already PP-maxed", mv.name)),
                _ => None,
            }
        }
        // A refusal consumes nothing, so an unchecked one is an endless retry. `ItemUseBicycle`
        // turns down water before it asks where you are.
        ItemId::Bicycle if state.map.surfing =>
            Some("the Bicycle cannot be ridden while surfing; get back on land first".into()),
        ItemId::Bicycle if !bike_riding_allowed(state) =>
            Some(format!("cycling is not allowed on {} (tileset {:?})", state.map.map, state.map.tileset)),
        _ => None,
    }
}

/// `IsBikeRidingAllowed`: Route 23 and Indigo Plateau, or a tileset in `BikeRidingTilesets`.
pub fn bike_riding_allowed(state: &GameState) -> bool {
    if matches!(state.map.map, Map::Route23 | Map::IndigoPlateau) {
        return true;
    }
    rom_list(&pokered_symbols::BikeRidingTilesets).contains(&(state.map.tileset as u8))
}

/// The `$FF`-terminated byte list at `ptr`.
fn rom_list(ptr: &crate::pokemon::symbols::DmgPointer) -> Vec<u8> {
    let crate::pokemon::symbols::DmgBank::ROM { bank } = ptr.bank else { panic!("{ptr:?} is not in ROM") };
    let offset = if bank == 0 { ptr.address as usize }
                 else { bank as usize * 0x4000 + (ptr.address as usize - 0x4000) };
    crate::pokemon::roms::POKERED[offset..].iter().copied().take_while(|&b| b != 0xFF).collect()
}

/// Which status an item's `.checkMonStatus` branch tests.
fn cures(item: ItemId, status: PokemonStatus) -> bool {
    match item {
        ItemId::Antidote => status == PokemonStatus::Poisoned,
        ItemId::BurnHeal => status == PokemonStatus::Burned,
        ItemId::IceHeal => status == PokemonStatus::Frozen,
        ItemId::Awakening => matches!(status, PokemonStatus::Asleep { .. }),
        ItemId::ParlyzHeal => status == PokemonStatus::Paralyzed,
        ItemId::FullHeal => status != PokemonStatus::None,
        _ => false,
    }
}

/// A wedged use reports itself rather than mashing for the rest of the leg.
const TICK_BUDGET: u32 = 1800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BagItemState {
    pub item: ItemId,
    pub target: UseTarget,
    /// What completion is measured against, from [`baseline`].
    pub baseline: u8,
    /// Press/release alternation, so every input is a fresh rising edge.
    pub press: bool,
    pub entered_menu: bool,
    /// Stable-overworld ticks after the effect lands: finishing in a one-tick gap between closing
    /// menus hands a live bag to the generic A-mash, which uses a second item.
    pub settle: u8,
    pub ticks: u32,
}

impl BagItemState {
    pub fn new(item: ItemId, target: UseTarget, api: &PokemonApi<'_>) -> Self {
        Self {
            item, target,
            baseline: match effect(item) {
                Effect::TogglesBicycle => on_bicycle(api.mmu()) as u8,
                _ => api.bag_item_quantity(item),
            },
            press: true, entered_menu: false, settle: 0, ticks: 0,
        }
    }

    fn done(&self, api: &PokemonApi<'_>) -> bool {
        match effect(self.item) {
            Effect::Consumed => self.entered_menu && api.bag_item_quantity(self.item) < self.baseline,
            Effect::TogglesBicycle => on_bicycle(api.mmu()) as u8 != self.baseline,
            // The menus close themselves, so the return to the overworld is the effect.
            Effect::OneShot =>
                self.entered_menu && api.game_mode().unwrap_or(GameMode::Overworld) == GameMode::Overworld,
        }
    }
}

/// The policy's half: what to hand the driver, or why not to.
pub fn pick(state: &GameState, item: ItemId, target: UseTarget, baseline: u8, attempts: u32)
    -> Result<FieldMove, String> {
    if goal_met(state, item, baseline, attempts) {
        return Err(format!("{item:?} used"));
    }
    if let Some(why) = blocked(state, item, target) {
        return Err(format!("{item:?} would have no effect — {why}"));
    }
    Ok(FieldMove::UseBagItem { item, target })
}

pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: BagItemState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);

    // Done: back out of whatever menu the use left open, then settle in the overworld.
    if s.done(api) {
        if game_mode != GameMode::Overworld {
            api.release_all_buttons();
            if s.press { api.press_button(JoypadButton::B); }
            agent.set_state(AgentState::UsingBagItem(BagItemState { press: !s.press, settle: 0, ..s }));
            return Ok(());
        }
        if s.settle < 15 {
            api.release_all_buttons();
            agent.set_state(AgentState::UsingBagItem(BagItemState { settle: s.settle + 1, ..s }));
            return Ok(());
        }
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("Used {:?} ({:?})", s.item, s.target) });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // Fizzled: back in the overworld with nothing to show.
    if s.entered_menu && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
        return Ok(());
    }
    if s.ticks > TICK_BUDGET {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox {
            message: format!("bag-item: {:?} did nothing in {TICK_BUDGET} ticks", s.item) });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    let s = BagItemState { entered_menu: s.entered_menu || game_mode != GameMode::Overworld,
                           ticks: s.ticks + 1, ..s };
    if !s.press {
        api.release_all_buttons();
        agent.set_state(AgentState::UsingBagItem(BagItemState { press: true, ..s }));
        return Ok(());
    }

    let (top_x, top_y, current, scroll) = api.menu_geometry();
    let tbid = api.menu_state().map(|m| m.text_box_id);
    let text = api.on_screen_text(false).unwrap_or_default();
    let nav = |cur: u8, target: u8| -> JoypadButton {
        if cur < target { JoypadButton::Down }
        else if cur > target { JoypadButton::Up }
        else { JoypadButton::A }
    };

    let button = if game_mode == GameMode::Overworld {
        JoypadButton::Start
    } else if (top_x, top_y) == START_MENU_ORIGIN {
        // Asked of `start_menu_row` rather than assumed, though the Pokédex is owned here.
        nav(current, start_menu_row(api, StartMenuRow::Item))
    } else if text.to_ascii_uppercase().contains("TECHNIQUE") {
        // `MoveSelectionMenu`'s relearn layout, the PP-restore move list.
        match s.target {
            UseTarget::Move { move_index, .. } => nav(current, move_index + 1),
            _ => JoypadButton::B,
        }
    } else if tbid == Some(TextBoxId::ListMenuBox) {
        match api.bag_item_position(s.item) {
            Some(row) => nav(current + scroll, row),
            None => JoypadButton::B,
        }
    } else if tbid == Some(TextBoxId::UseTossMenuTemplate) {
        nav(current, 0) // USE / TOSS → USE. (The Bicycle never shows this menu.)
    } else if tbid == Some(TextBoxId::MessageBox) && top_x == 0 && (top_y == 1 || top_y == 3) {
        match s.target.slot() {
            Some(slot) => nav(current, slot),
            None => JoypadButton::B,
        }
    } else {
        JoypadButton::A // transitional text
    };

    // A per-tick trace, off unless `ITEMS_TRACE` is set.
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *TRACE.get_or_init(|| std::env::var("ITEMS_TRACE").is_ok()) {
        println!("[items] t{} mode {game_mode:?} tbid {tbid:?} geom ({top_x},{top_y}) cur {current} \
                  scroll {scroll} text {:?} -> {button:?}", s.ticks, text.chars().take(40).collect::<String>());
    }
    api.release_all_buttons();
    api.press_button(button);
    agent.set_state(AgentState::UsingBagItem(BagItemState { press: false, ..s }));
    Ok(())
}

/// The stat items, in the order [`PolicyStep::stat_item_steps`] spends them.
pub const STAT_ITEMS: &[ItemId] = &[
    ItemId::XAttack, ItemId::XDefend, ItemId::XSpeed, ItemId::XSpecial,
    ItemId::XAccuracy, ItemId::GuardSpec, ItemId::DireHit,
];

/// The four `ItemUseXStat` stat stages, neutral at 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatMods { pub attack: u8, pub defense: u8, pub speed: u8, pub special: u8 }

impl StatMods {
    pub const NEUTRAL: Self = Self { attack: 7, defense: 7, speed: 7, special: 7 };
}

/// Only meaningful inside a battle.
pub fn stat_mods(mmu: &MMU) -> StatMods {
    StatMods {
        attack: mmu.read_pointer(&pokered_symbols::wPlayerMonAttackMod),
        defense: mmu.read_pointer(&pokered_symbols::wPlayerMonDefenseMod),
        speed: mmu.read_pointer(&pokered_symbols::wPlayerMonSpeedMod),
        special: mmu.read_pointer(&pokered_symbols::wPlayerMonSpecialMod),
    }
}

/// `wPlayerBattleStatus2`, which X Accuracy, Guard Spec. and Dire Hit set bits of.
pub fn player_battle_status2(mmu: &MMU) -> u8 {
    mmu.read_pointer(&pokered_symbols::wPlayerBattleStatus2)
}

/// Bit positions in `wPlayerBattleStatus2`.
pub mod battle_status2 {
    pub const USING_X_ACCURACY: u8 = 1 << 0;
    pub const PROTECTED_BY_MIST: u8 = 1 << 1;
    pub const GETTING_PUMPED: u8 = 1 << 2;
}

impl PolicyStep {
    pub const fn use_medicine(item: ItemId, slot: u8) -> Self {
        Self::UseBagItem { item, target: UseTarget::Party { slot } }
    }

    pub const fn use_pp_restore(item: ItemId, slot: u8, move_index: u8) -> Self {
        Self::UseBagItem { item, target: UseTarget::Move { slot, move_index } }
    }

    /// An item with no target: a Repel, the Bicycle, the Itemfinder.
    pub const fn use_item(item: ItemId) -> Self {
        Self::UseBagItem { item, target: UseTarget::Nothing }
    }

    /// Press the Itemfinder where it can answer yes, buy a Repel, then press it where it cannot.
    pub fn press_the_itemfinder_steps(stand_near: Map) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::VermilionCity },
            Self::enter(stand_near),
            Self::enter(Map::VermilionCity),
            Self::use_item(ItemId::Itemfinder),
            Self::enter(Map::VermilionMart),
            Self::BuyFromMart { item: crate::pokemon::bag::BagItem::new(ItemId::Repel, 1),
                                map: Map::VermilionMart },
            Self::enter(Map::VermilionCity),
            Self::Fly { to: Map::FuchsiaCity },
            Self::use_item(ItemId::Itemfinder),
        ]
    }

    /// An Ether onto a spent move, then the hidden PP Up Celadon is sitting on.
    pub fn pp_restore_steps(ether: ItemId, slot: u8, ether_move: u8, pp_up_move: u8) -> Vec<Self> {
        vec![
            Self::use_pp_restore(ether, slot, ether_move),
            Self::Fly { to: Map::CeladonCity },
            Self::use_pp_restore(ItemId::PpUp, slot, pp_up_move),
        ]
    }

    /// Set a Repel counter running, then walk far enough to watch it tick down.
    pub fn repel_steps(item: ItemId, walk_to: Map) -> Vec<Self> {
        vec![Self::use_item(item), Self::enter(walk_to)]
    }

    pub fn ride_bicycle_steps(ride_to: Map) -> Vec<Self> {
        vec![Self::use_item(ItemId::Bicycle), Self::enter(ride_to), Self::use_item(ItemId::Bicycle)]
    }

    /// Buy the stat items and a Poké Doll, then spend them in one wild battle.
    pub fn stat_item_steps(shed: &[ItemId], on_map: Map, items: &'static [ItemId]) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::CeladonCity }, Self::enter(Map::CeladonPokecenter)];
        s.extend(shed.iter().map(|&item| Self::deposit_item(item, u8::MAX, Map::CeladonPokecenter)));
        s.extend([
            Self::enter(Map::CeladonCity),
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonMart2F),
            Self::enter(Map::CeladonMart3F),
            Self::enter(Map::CeladonMart4F),
            Self::BuyFromMart { item: crate::pokemon::bag::BagItem::new(ItemId::PokeDoll, 1),
                                map: Map::CeladonMart4F },
            Self::enter(Map::CeladonMart5F),
        ]);
        // `BuyFromMart` targets 5F's Clerk 1, who sells the stat items.
        s.extend(STAT_ITEMS.iter().map(|&item| Self::BuyFromMart {
            item: crate::pokemon::bag::BagItem::new(item, 1), map: Map::CeladonMart5F }));
        s.extend([
            Self::enter(Map::CeladonMart4F),
            Self::enter(Map::CeladonMart3F),
            Self::enter(Map::CeladonMart2F),
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonCity),
            Self::Fly { to: Map::ViridianCity },
            Self::enter(on_map),
            Self::UseItemsInBattle { on_map, items },
            Self::enter(Map::ViridianCity),
        ]);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`effect`]'s premises match `UsableItems_CloseMenu`, which the Bicycle is not in.
    #[test]
    fn close_menu_items_match_the_rom() {
        let names: Vec<ItemId> = rom_list(&pokered_symbols::UsableItems_CloseMenu)
            .iter().filter_map(|&b| ItemId::from_repr(b)).collect();
        assert_eq!(names, vec![ItemId::EscapeRope, ItemId::Itemfinder, ItemId::PokeFlute,
                               ItemId::OldRod, ItemId::GoodRod, ItemId::SuperRod],
            "UsableItems_CloseMenu changed — `Effect::OneShot`'s premise is that the Itemfinder is \
             in it, i.e. that using it returns straight to the overworld");
        assert!(!names.contains(&ItemId::Bicycle));
    }

    /// Every item in `UsableItems_PartyMenu` needs a [`UseTarget`] with a slot.
    #[test]
    fn party_menu_items_need_a_slot() {
        let party_items: Vec<ItemId> = rom_list(&pokered_symbols::UsableItems_PartyMenu)
            .into_iter().filter_map(ItemId::from_repr).collect();
        for item in [ItemId::Potion, ItemId::Revive, ItemId::FullHeal, ItemId::Ether,
                     ItemId::MaxElixer, ItemId::PpUp, ItemId::XAttack] {
            assert!(party_items.contains(&item), "{item:?} should open the party menu");
        }
        for item in [ItemId::Repel, ItemId::Itemfinder, ItemId::Bicycle, ItemId::PokeDoll,
                     ItemId::GuardSpec, ItemId::DireHit] {
            assert!(!party_items.contains(&item), "{item:?} should NOT open the party menu");
        }
    }

    #[test]
    fn the_bicycle_is_refused_on_water() {
        let mut state = GameState::default();
        state.bag.push(crate::pokemon::bag::BagItem { id: ItemId::Bicycle, quantity: 1 }).expect("room");
        state.map.map = Map::Route11;
        state.map.tileset = crate::pokemon::map_header::TileSetId::Overworld;
        assert!(bike_riding_allowed(&state), "the refusal has to be the water's, not the map's");
        assert_eq!(blocked(&state, ItemId::Bicycle, UseTarget::Nothing), None, "on land it rides");

        state.map.surfing = true;
        let refusal = blocked(&state, ItemId::Bicycle, UseTarget::Nothing).expect("refused on water");
        assert!(refusal.contains("surfing"), "{refusal}");
    }

    /// The Poké Doll ends the battle, so it is never in `STAT_ITEMS`.
    #[test]
    fn no_stat_item_ends_the_battle() {
        assert!(!STAT_ITEMS.contains(&ItemId::PokeDoll),
            "the Poké Doll ends the battle (wEscapedFromBattle), so it belongs after STAT_ITEMS, \
             never inside it");
    }
}

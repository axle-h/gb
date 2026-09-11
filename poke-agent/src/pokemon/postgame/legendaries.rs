//! The legendaries: Zapdos, Moltres, Mewtwo. A ball fails whenever `Rand1` less the status bonus
//! exceeds the catch rate, whatever the HP, so against catch rate 3 a status is the only lever.

use gb::geometry::Point8;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::{BattleAction, BattleType};
use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::policy::{PartyRef, PolicyStep};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::GameState;

/// The species with catch rate 3, which are thrown at and never fought.
pub const CATCH_RATE_3: [PokemonSpecies; 4] = [
    PokemonSpecies::Articuno,
    PokemonSpecies::Zapdos,
    PokemonSpecies::Moltres,
    PokemonSpecies::Mewtwo,
];

/// Slowpoke's slot, the one member that can learn Thunder Wave, in every fixture from
/// `postgame-fly-bike.bin` on.
pub const PARALYSER_SLOT: u8 = 3;

/// Where Moltres lands once caught, after the fixture's four.
const MOLTRES_SLOT: u8 = 4;

fn heal_amount(item: ItemId) -> u16 {
    match item {
        ItemId::Potion => 20,
        ItemId::SuperPotion => 50,
        ItemId::HyperPotion => 200,
        ItemId::MaxPotion | ItemId::FullRestore => u16::MAX,
        _ => 0,
    }
}

/// The cheapest sufficient heal on offer, or the biggest one if nothing covers the damage.
fn heal_action(actions: &[BattleAction], missing: u16) -> Option<&BattleAction> {
    let offered = || actions.iter().filter(|a| matches!(a,
        BattleAction::UseItem { item, .. } if heal_amount(item.id) > 0));
    let amount = |a: &BattleAction| match a {
        BattleAction::UseItem { item, .. } => heal_amount(item.id),
        _ => 0,
    };
    offered().filter(|a| amount(a) >= missing).min_by_key(|a| amount(a))
        .or_else(|| offered().max_by_key(|a| amount(a)))
}

/// A battle turn against a catch-rate-3 target, or `None` for the generic catch policy.
pub fn pre_catch_action(
    state: &GameState,
    target: PokemonSpecies,
    actions: &[BattleAction],
    throw_ball: Option<&BattleAction>,
) -> Option<BattleAction> {
    if !CATCH_RATE_3.contains(&target) {
        return None;
    }
    let battle = state.battle.as_ref()?;
    if battle.battle_type != BattleType::Wild || battle.enemy.species != target {
        return None;
    }
    // A Master Ball captures before `ItemUseBall` rolls anything.
    if matches!(throw_ball, Some(BattleAction::UseItem { item, .. }) if item.id == ItemId::MasterBall) {
        println!("[legendaries] Master Ball in the bag — throwing it at {target} immediately");
        return throw_ball.cloned();
    }

    let statused = battle.enemy.status != PokemonStatus::None;

    let switch_to = |slot: u8| actions.iter()
        .find(|a| matches!(a, BattleAction::SwitchPokemon { slot: s, .. } if *s == slot))
        .cloned();

    let paralyser = state.pokemon.iter().enumerate()
        .find(|(_, p)| p.current_hp > 0
            && p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::ThunderWave && m.pp > 0))
        .map(|(i, _)| i as u8);

    if !statused {
        let held = battle.player.available_battle_moves().into_iter()
            .find(|a| matches!(a, BattleAction::Fight { battle_move, .. }
                if battle_move.name == PokemonMoveName::ThunderWave));
        if let Some(paralyse) = held {
            println!("[legendaries] {target} has no status — Thunder Wave");
            return Some(paralyse);
        }
        if let Some(slot) = paralyser.filter(|s| *s != battle.active_party_slot) {
            if let Some(switch) = switch_to(slot) {
                println!("[legendaries] switching slot {slot} in to paralyse {target}");
                return Some(switch);
            }
        }
    } else {
        let healthiest = state.pokemon.iter().enumerate()
            .max_by_key(|(_, p)| p.current_hp).map(|(i, p)| (i as u8, p.current_hp));
        if let Some((slot, hp)) = healthiest {
            if slot != battle.active_party_slot && hp > battle.player.current_hp * 2 {
                if let Some(switch) = switch_to(slot) {
                    println!("[legendaries] {target} is {:?} — swapping slot {slot} in to soak the throws",
                        battle.enemy.status);
                    return Some(switch);
                }
            }
        }
    }

    if battle.player.remaining_hp() < 0.35 {
        let missing = battle.player.stats.hp.saturating_sub(battle.player.current_hp);
        if let Some(heal) = heal_action(actions, missing) {
            println!("[legendaries] {:?} at {:.0}% — {heal}", battle.player.species,
                battle.player.remaining_hp() * 100.0);
            return Some(heal.clone());
        }
    }

    throw_ball.cloned()
}

impl PolicyStep {
    /// The toolkit all three catches share: balls, potions, and TM45 Thunder Wave taught to Slowpoke.
    pub fn arm_for_legendaries_steps() -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeruleanCity },
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::Route24),
            Self::CollectItem(MapSprite::ROUTE24_TM_THUNDER_WAVE),
            Self::TeachMove { item: ItemId::Tm45ThunderWave, target: PartyRef::Slot(PARALYSER_SLOT) },
            Self::Fly { to: Map::CinnabarIsland },
            Self::BuyFromMart { item: BagItem::new(ItemId::UltraBall, 10), map: Map::CinnabarMart },
            Self::BuyFromMart { item: BagItem::new(ItemId::HyperPotion, 10), map: Map::CinnabarMart },
            Self::enter(Map::CinnabarIsland), // back outside, or the town map will not open
            Self::Fly { to: Map::CeruleanCity },
            Self::BuyFromMart { item: BagItem::new(ItemId::PokeBall, 40), map: Map::CeruleanMart },
            Self::enter(Map::CeruleanCity),
        ]
    }

    /// Moltres, on Victory Road 2F at (11,5).
    pub fn moltres_steps() -> Vec<Self> {
        let mut steps = vec![
            Self::Fly { to: Map::ViridianCity },
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::Route22),
            Self::enter(Map::Route22Gate),
            Self::Interact(MapSprite::ROUTE22GATE_GUARD), // badge check, and it flips the dynamic warp
            Self::enter(Map::Route23),
            Self::goto(Map::VictoryRoad1F),
            Self::UseStrength { target: PartyRef::Slot(PARALYSER_SLOT) },
            Self::SolveBoulders { switch: Point8 { x: 17, y: 13 }, boulder: None },
            Self::enter(Map::VictoryRoad2F),
            Self::UseStrength { target: PartyRef::Slot(PARALYSER_SLOT) },
            Self::SolveBoulders { switch: Point8 { x: 1, y: 16 }, boulder: None },
            Self::enter(Map::VictoryRoad3F),
        ];
        // Down through 3F's (2,0) warp into 2F's north strip, where Moltres is.
        steps.extend([
            Self::enter_at(Map::VictoryRoad2F, 1, 1),
            Self::CatchPokemon { species: PokemonSpecies::Moltres, on_map: Map::VictoryRoad2F,
                                 ball: None },
        ]);
        steps
    }

    /// Reach the Power Plant and catch Zapdos at (4,9).
    pub fn zapdos_steps() -> Vec<Self> {
        vec![
            Self::Dig { target: PartyRef::Slot(PARALYSER_SLOT) },
            Self::Fly { to: Map::CeruleanCity },
            Self::enter(Map::CeruleanTrashedHouse),   // main terrace front door
            Self::enter_at(Map::CeruleanCity, 27, 9), // back door → the Route-9 terrace
            Self::enter(Map::Route9),
            Self::CutTree { map: Map::Route9 },        // the (5,8) tree boxing in the west pocket
            Self::enter(Map::Route10),
            Self::enter(Map::PowerPlant),
            Self::CatchPokemon { species: PokemonSpecies::Zapdos, on_map: Map::PowerPlant,
                                 ball: None },
        ]
    }

    /// Cerulean Cave and Mewtwo at B1F (27,13).
    pub fn mewtwo_steps() -> Vec<Self> {
        let mut steps = vec![
            Self::enter(Map::Route10),
            Self::Fly { to: Map::CeruleanCity },
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::deposit_pokemon(MOLTRES_SLOT, Map::CeruleanPokecenter),
            Self::enter(Map::CeruleanCity),
        ];
        steps.extend([
            Self::enter(Map::Route24),
            Self::enter_at(Map::CeruleanCity, 14, 0),
            Self::enter(Map::CeruleanCave1F),
            // 1F's B1F ladder at (0,6) is behind a Cavern `TilePairCollisions` elevation boundary, so
            // it is reached by way of 2F.
            Self::enter_at(Map::CeruleanCave2F, 3, 11),
            Self::enter_at(Map::CeruleanCave1F, 1, 3),
            Self::enter(Map::CeruleanCaveB1F),
            Self::CatchPokemon { species: PokemonSpecies::Mewtwo, on_map: Map::CeruleanCaveB1F,
                                 ball: None },
        ]);
        steps
    }
}

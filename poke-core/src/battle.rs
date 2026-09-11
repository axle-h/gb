use std::fmt::{Display, Formatter};
use crate::bag::{Bag, BagItem};
use crate::item::ItemId;
use crate::map::Map;
use crate::move_name::PokemonMove;
use crate::pokemon::PokemonSummary;
use crate::status::PokemonStatus;

#[derive(Debug, Clone, Copy, Eq, PartialEq, strum_macros::Display)]
pub enum BattleType { Wild, Trainer, Safari }

/// A ghost battle, in which the cartridge lets no move execute.
pub fn is_ghost_battle(map: Map, bag: &Bag, battle_type: BattleType) -> bool {
    battle_type == BattleType::Wild
        && (Map::PokemonTower1F..=Map::PokemonTower7F).contains(&map)
        && !bag.contains(&ItemId::SilphScope)
}

#[derive(Debug, Copy, Clone)]
pub struct BattleState {
    pub battle_type:       BattleType,
    pub player:            PokemonSummary,
    pub enemy:             PokemonSummary,
    /// Party index of the active Pokémon.
    pub active_party_slot: u8,
    /// The enemy is part-way through Wrap, Fire Spin, Clamp or Bind.
    pub enemy_trapping: bool,
    /// `wEnemyMonActualCatchRate`, the rate `ItemUseBall` compares `Rand1` against.
    pub enemy_catch_rate: u8,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BattleAction {
    Fight { slot: u8, battle_move: PokemonMove },
    /// `target` is the party slot for an item that asks which Pokémon, as the agent's `item_use::helps_in_battle` decides.
    UseItem { slot: u8, item: BagItem, target: Option<u8> },
    SwitchPokemon { slot: u8, pokemon: PokemonSummary },
    /// Wild battles only.
    Run,
    // Offered only when `battle_type == Safari`.
    SafariBall,
    SafariBait,
    SafariRock,
}

impl Display for BattleAction {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            BattleAction::Fight { battle_move, .. } => write!(f, "FIGHT  {}  PP {}", battle_move.name, battle_move.pp),
            BattleAction::UseItem { item, target: None, .. } => write!(f, "ITEM   {} ×{}", item.id, item.quantity),
            BattleAction::UseItem { item, target: Some(target), .. } =>
                write!(f, "ITEM   {} ×{} on party slot {target}", item.id, item.quantity),
            BattleAction::SwitchPokemon { pokemon, .. } => {
                write!(f, "PKMN   {} Lv{} — {}/{} HP",
                       pokemon.species, pokemon.level, pokemon.current_hp, pokemon.stats.hp)?;
                // `PokemonStatus`' `Display` would print a healthy Pokémon as `None`.
                match pokemon.status {
                    PokemonStatus::None => Ok(()),
                    status => write!(f, ", {status}"),
                }
            }
            BattleAction::Run => write!(f, "RUN"),
            BattleAction::SafariBall => write!(f, "BALL"),
            BattleAction::SafariBait => write!(f, "BAIT"),
            BattleAction::SafariRock => write!(f, "ROCK"),
        }
    }
}

/// `wIsInBattle` when the player has just lost: not a battle, and not yet the overworld either.
pub const LOST_BATTLE: u8 = 0xff;

#[cfg(test)]
mod tests {
    use super::*;

    fn bag_with(items: &[ItemId]) -> Bag {
        Bag::new(items.iter().map(|&id| BagItem::new(id, 1)).collect())
    }

    /// Each of `IsGhostBattle`'s three conditions, and both edges of its map range.
    #[test]
    fn a_ghost_battle_is_a_wild_one_in_the_tower_without_the_scope() {
        let empty = bag_with(&[]);
        let scope = bag_with(&[ItemId::SilphScope]);

        for map in [Map::PokemonTower1F, Map::PokemonTower3F, Map::PokemonTower7F] {
            assert!(is_ghost_battle(map, &empty, BattleType::Wild), "{map} without the Scope");
            assert!(!is_ghost_battle(map, &scope, BattleType::Wild), "{map} carrying the Scope");
        }

        assert!(!is_ghost_battle(Map::PokemonTower3F, &empty, BattleType::Trainer));

        // Either side of the tower's `0x8E..=0x94`.
        assert!(!is_ghost_battle(Map::LavenderPokecenter, &empty, BattleType::Wild));
        assert!(!is_ghost_battle(Map::MrFujisHouse, &empty, BattleType::Wild));
    }
}

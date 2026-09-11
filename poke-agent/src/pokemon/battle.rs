use std::fmt::{Display, Formatter};
use gb::mmu::MMU;
use crate::pokemon::bag::{Bag, BagItem};
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
use gb::ram::ROM;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::pokemon::{PokemonStats, PokemonSummary, PokemonType};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::{pokered_symbols, DmgPointer, DmgPointerRead};

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
    UseItem { slot: u8, item: BagItem },
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
            BattleAction::UseItem { item, .. } => write!(f, "ITEM   {} ×{}", item.id, item.quantity),
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

pub trait BattleStateReader {
    fn read_battle_state(&self) -> Option<BattleState>;
}

impl BattleStateReader for MMU {
    fn read_battle_state(&self) -> Option<BattleState> {
        let is_in_battle = self.read_pointer(&pokered_symbols::wIsInBattle);
        // `LOST_BATTLE` has ended: read as a battle, a blackout's fainted Pokémon is still out.
        if is_in_battle == 0 || is_in_battle == LOST_BATTLE {
            return None;
        }
        // `wBattleType`: 0 normal, 1 old-man tutorial, 2 Safari Zone.
        let battle_type = if self.read_pointer(&pokered_symbols::wBattleType) == 2 {
            BattleType::Safari
        } else if is_in_battle == 2 {
            BattleType::Trainer
        } else {
            BattleType::Wild
        };

        // `wPlayerMonNumber` follows a mid-battle switch; `wBattleMonPartyPos` stays on the first mon.
        let active_party_slot = self.read_pointer(&pokered_symbols::wPlayerMonNumber);

        // High nibble the disabled move slot, 1-based; low nibble the turn counter.
        let disabled_slot = |raw: u8| -> Option<u8> {
            let slot = raw >> 4;
            (slot >= 1).then(|| slot - 1)
        };
        let player_disabled = disabled_slot(self.read_pointer(&pokered_symbols::wPlayerDisabledMove));
        let enemy_disabled = disabled_slot(self.read_pointer(&pokered_symbols::wEnemyDisabledMove));

        fn read_battle_moves(mmu: &MMU, move_base: DmgPointer, pp_base: DmgPointer) -> [Option<PokemonMove>; 4] {
            std::array::from_fn(|i| {
                let id = mmu.read(move_base.address + i as u16);
                PokemonMoveName::from_repr(id).map(|name| PokemonMove {
                    name,
                    pp: mmu.read(pp_base.address + i as u16),
                })
            })
        }

        Some(BattleState {
            battle_type,
            player: PokemonSummary {
                species: PokemonSpecies::from_repr(self.read_pointer(&pokered_symbols::wBattleMonSpecies2))?,
                level: self.read_pointer(&pokered_symbols::wBattleMonLevel),
                current_hp: self.read_pointer_u16_be(&pokered_symbols::wBattleMonHP),
                status: PokemonStatus::from(self.read_pointer(&pokered_symbols::wBattleMonStatus)),
                types: [
                    PokemonType::from_repr(self.read_pointer(&pokered_symbols::wBattleMonType1))?,
                    PokemonType::from_repr(self.read_pointer(&pokered_symbols::wBattleMonType2))?,
                ],
                moves: read_battle_moves(self, pokered_symbols::wBattleMonMoves, pokered_symbols::wBattleMonPP),
                disabled_move_slot: player_disabled,
                stats: PokemonStats {
                    hp: self.read_pointer_u16_be(&pokered_symbols::wBattleMonMaxHP),
                    attack: self.read_pointer_u16_be(&pokered_symbols::wBattleMonAttack),
                    defense: self.read_pointer_u16_be(&pokered_symbols::wBattleMonDefense),
                    speed: self.read_pointer_u16_be(&pokered_symbols::wBattleMonSpeed),
                    special: self.read_pointer_u16_be(&pokered_symbols::wBattleMonSpecial),
                },
            },
            enemy: PokemonSummary {
                species: PokemonSpecies::from_repr(self.read_pointer(&pokered_symbols::wEnemyMonSpecies2))?,
                level: self.read_pointer(&pokered_symbols::wEnemyMonLevel),
                current_hp: self.read_pointer_u16_be(&pokered_symbols::wEnemyMonHP),
                status: PokemonStatus::from(self.read_pointer(&pokered_symbols::wEnemyMonStatus)),
                types: [
                    PokemonType::from_repr(self.read_pointer(&pokered_symbols::wEnemyMonType1))?,
                    PokemonType::from_repr(self.read_pointer(&pokered_symbols::wEnemyMonType2))?,
                ],
                moves: read_battle_moves(self, pokered_symbols::wEnemyMonMoves, pokered_symbols::wEnemyMonPP),
                disabled_move_slot: enemy_disabled,
                stats: PokemonStats {
                    hp: self.read_pointer_u16_be(&pokered_symbols::wEnemyMonMaxHP),
                    attack: self.read_pointer_u16_be(&pokered_symbols::wEnemyMonAttack),
                    defense: self.read_pointer_u16_be(&pokered_symbols::wEnemyMonDefense),
                    speed: self.read_pointer_u16_be(&pokered_symbols::wEnemyMonSpeed),
                    special: self.read_pointer_u16_be(&pokered_symbols::wEnemyMonSpecial),
                },
            },
            active_party_slot,
            // `wEnemyBattleStatus1` bit 5 is `USING_TRAPPING_MOVE`.
            enemy_trapping: self.read_pointer(&pokered_symbols::wEnemyBattleStatus1) & (1 << 5) != 0,
            enemy_catch_rate: self.read_pointer(&pokered_symbols::wEnemyMonActualCatchRate),
        })
    }
}

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

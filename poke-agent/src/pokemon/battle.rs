use gb::mmu::MMU;
use gb::ram::ROM;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::pokemon::{PokemonStats, PokemonSummary, PokemonType};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::{pokered_symbols, DmgPointer, DmgPointerRead};
pub use poke_core::battle::*;

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


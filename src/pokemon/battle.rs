use crate::mmu::MMU;
use crate::ram::ROM;
use crate::pokemon::item::ItemId;
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BattleType { Wild, Trainer }

#[derive(Debug, Copy, Clone)]
pub struct BattleState {
    pub battle_type:       BattleType,
    pub player:            BattlePokemon,
    pub enemy:             BattlePokemon,
    /// 0-based index into the pokemon party for the currently-active Pokemon.
    pub active_party_slot: u8,
}

#[derive(Debug, Copy, Clone)]
pub struct BattlePokemon {
    pub species:    PokemonSpecies,
    pub level:      u8,
    pub current_hp: u16,
    pub max_hp:     u16,
    pub status:     PokemonStatus,
    pub moves:      [Option<BattleMove>; 4],
}

#[derive(Debug, Clone, Copy)]
pub struct BattleMove {
    pub name:       PokemonMoveName,
    pub current_pp: u8,
}

#[derive(Debug, Clone)]
pub struct BagItem {
    pub id:       ItemId,
    pub quantity: u8,
}

/// Action the player can take on their turn.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BattleAction {
    /// Use the move in the given slot (0–3).
    Fight(u8),
    /// Use the bag item at `bag_slot` (index into `BattleState.bag`).
    UseItem(u8),
    /// Switch to the party Pokemon at `party_slot` (index into `BattleState.party`).
    SwitchPokemon(u8),
    /// Attempt to flee (wild battles only).
    Run,
}

pub trait BattleStateReader {
    fn read_battle_state(&self) -> Option<BattleState>;
}

impl BattleStateReader for MMU {
    fn read_battle_state(&self) -> Option<BattleState> {
        let is_in_battle = self.read_pointer(&pokered_symbols::wIsInBattle);
        if is_in_battle == 0 {
            return None;
        }
        let battle_type = if is_in_battle == 2 { BattleType::Trainer } else { BattleType::Wild };

        let active_party_slot = self.read_pointer(&pokered_symbols::wBattleMonPartyPos);

        Some(BattleState {
            battle_type,
            player: read_battle_mon(self),
            enemy:  read_enemy_mon(self),
            active_party_slot,
        })
    }
}

fn read_battle_mon(mmu: &MMU) -> BattlePokemon {
    let species    = PokemonSpecies::from_repr(mmu.read_pointer(&pokered_symbols::wBattleMonSpecies2))
        .unwrap_or(PokemonSpecies::Bulbasaur);
    let level      = mmu.read_pointer(&pokered_symbols::wBattleMonLevel);
    let current_hp = mmu.read_u16_be(pokered_symbols::wBattleMonHP.address);
    let max_hp     = mmu.read_u16_be(pokered_symbols::wBattleMonMaxHP.address);
    let status     = PokemonStatus::from(mmu.read_pointer(&pokered_symbols::wBattleMonStatus));

    let move_base = pokered_symbols::wBattleMonMoves.address;
    let pp_base   = pokered_symbols::wBattleMonPP.address;
    let moves = std::array::from_fn(|i| {
        let id = mmu.read(move_base + i as u16);
        PokemonMoveName::from_repr(id).map(|name| BattleMove {
            name,
            current_pp: mmu.read(pp_base + i as u16),
        })
    });

    BattlePokemon { species, level, current_hp, max_hp, status, moves }
}

fn read_enemy_mon(mmu: &MMU) -> BattlePokemon {
    let species    = PokemonSpecies::from_repr(mmu.read_pointer(&pokered_symbols::wEnemyMonSpecies2))
        .unwrap_or(PokemonSpecies::Bulbasaur);
    let level      = mmu.read_pointer(&pokered_symbols::wEnemyMonLevel);
    let current_hp = mmu.read_u16_be(pokered_symbols::wEnemyMonHP.address);
    let max_hp     = mmu.read_u16_be(pokered_symbols::wEnemyMonMaxHP.address);
    let status     = PokemonStatus::from(mmu.read_pointer(&pokered_symbols::wEnemyMonStatus));

    let move_base = pokered_symbols::wEnemyMonMoves.address;
    let pp_base   = pokered_symbols::wEnemyMonPP.address;
    let moves = std::array::from_fn(|i| {
        let id = mmu.read(move_base + i as u16);
        PokemonMoveName::from_repr(id).map(|name| BattleMove {
            name,
            current_pp: mmu.read(pp_base + i as u16),
        })
    });

    BattlePokemon { species, level, current_hp, max_hp, status, moves }
}


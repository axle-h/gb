use crate::mmu::MMU;
use crate::ram::{RAM, ROM};
use crate::pokemon::encoding::PokemonEncoding;
use crate::pokemon::item::ItemId;
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::party::PokemonParty;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::{DmgPointerRead, pokered_symbols};

// ── Battle state ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BattleType { Wild, Trainer }

#[derive(Debug, Clone)]
pub struct BattleState {
    pub battle_type:       BattleType,
    pub player:            BattlePokemon,
    pub enemy:             BattlePokemon,
    /// Full party, needed for choosing who to switch in.
    pub party:             PokemonParty,
    /// 0-based index into `party` for the currently-active Pokemon.
    pub active_party_slot: u8,
    /// Items currently in the bag.
    pub bag:               Vec<BagItem>,
}

#[derive(Debug, Clone)]
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

// ── Battle actions ────────────────────────────────────────────────────────────

/// Action the player can take on their turn.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BattleAction {
    /// Use the move in the given slot (0–3).
    Fight(u8),
    /// Use the bag item at `bag_slot` (index into `BattleState.bag`).
    UseItem(usize),
    /// Switch to the party Pokemon at `party_slot` (index into `BattleState.party`).
    SwitchPokemon(usize),
    /// Attempt to flee (wild battles only).
    Run,
}

impl BattleAction {
    /// Sequence of `wCurrentMenuItem` targets to navigate to and confirm with A, in order.
    /// The main battle menu layout: FIGHT=0, PKMN=1, ITEM=2, RUN=3.
    /// Sub-menus (move/bag/party) also use `wCurrentMenuItem`.
    pub fn navigation_targets(self) -> Vec<u8> {
        match self {
            BattleAction::Fight(slot)          => vec![0, slot],
            BattleAction::UseItem(bag_slot)    => vec![2, bag_slot as u8],
            BattleAction::SwitchPokemon(slot)  => vec![1, slot as u8, 0], // 0 = SWITCH option
            BattleAction::Run                  => vec![3],
        }
    }
}

// ── Reading ───────────────────────────────────────────────────────────────────

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

        let party             = self.read_player_pokemon_party().ok()?;
        let active_party_slot = self.read_pointer(&pokered_symbols::wBattleMonPartyPos);
        let bag               = read_bag(self);

        Some(BattleState {
            battle_type,
            player: read_battle_mon(self),
            enemy:  read_enemy_mon(self),
            party,
            active_party_slot,
            bag,
        })
    }
}

fn read_battle_mon(mmu: &MMU) -> BattlePokemon {
    let species    = PokemonSpecies::from_repr(mmu.read_pointer(&pokered_symbols::wBattleMonSpecies))
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
    let species    = PokemonSpecies::from_repr(mmu.read_pointer(&pokered_symbols::wEnemyMonSpecies))
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

fn read_bag(mmu: &MMU) -> Vec<BagItem> {
    let count = mmu.read_pointer(&pokered_symbols::wNumBagItems) as usize;
    let base  = pokered_symbols::wBagItems.address;
    (0..count)
        .filter_map(|i| {
            let item_base = base + i as u16 * 2;
            if let Some(id) = ItemId::from_repr(mmu.read(item_base)) {
                Some(BagItem { id, quantity: mmu.read(item_base + 1) })
            } else {
                None
            }
        })
        .collect()
}

// ── Writing (test helpers) ────────────────────────────────────────────────────

pub trait BagWriter {
    /// Replaces the bag contents with the given `(item_id, quantity)` pairs.
    fn write_bag(&mut self, items: &[(u8, u8)]);
}

impl BagWriter for MMU {
    fn write_bag(&mut self, items: &[(u8, u8)]) {
        let count = items.len().min(20) as u8;
        self.write(pokered_symbols::wNumBagItems.address, count);
        let base = pokered_symbols::wBagItems.address;
        for (i, &(id, qty)) in items.iter().take(20).enumerate() {
            self.write(base + i as u16 * 2,     id);
            self.write(base + i as u16 * 2 + 1, qty);
        }
        // FF terminator
        self.write(base + count as u16 * 2, 0xFF);
    }
}

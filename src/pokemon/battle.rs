use std::fmt::{Display, Formatter};
use crate::mmu::MMU;
use crate::pokemon::bag::BagItem;
use crate::ram::ROM;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::pokemon::{PokemonStats, PokemonSummary, PokemonType};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::{pokered_symbols, DmgPointer, DmgPointerRead};

#[derive(Debug, Clone, Copy, Eq, PartialEq, strum_macros::Display)]
pub enum BattleType { Wild, Trainer, Safari }

#[derive(Debug, Copy, Clone)]
pub struct BattleState {
    pub battle_type:       BattleType,
    pub player:            PokemonSummary,
    pub enemy:             PokemonSummary,
    /// 0-based index into the pokemon party for the currently-active Pokemon.
    pub active_party_slot: u8,
    /// The enemy is part-way through a **partial-trapping move** — Wrap, Fire Spin, Clamp, Bind.
    ///
    /// This is not cosmetic: `MainInBattleLoop` checks exactly this bit and, when it is set, replaces
    /// whatever move the player chose with `CANNOT_MOVE` (`engine/battle/core.asm:316-322`). The battle
    /// menu still opens, so **items, switching and running all still work** — only moves are negated.
    /// A policy that keeps re-picking a move while this is set gets nothing done, which is the Gen 1
    /// "wrap lock" and is exactly what stops a slow Pokémon ever landing Thunder Wave on Moltres.
    pub enemy_trapping: bool,
    /// `wEnemyMonActualCatchRate` — the catch rate `ItemUseBall` actually compares `Rand1` against.
    ///
    /// Deliberately the **live** byte rather than the species' base stat: in the Safari Zone a ROCK
    /// doubles it and a BAIT halves it for the rest of the encounter
    /// (`engine/items/item_effects.asm:1432-1457`), so the base stat would misreport the odds of every
    /// throw after the first. Zero outside a battle-initialised state.
    pub enemy_catch_rate: u8,
}

/// Action the player can take on their turn.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BattleAction {
    /// Use the move in the given slot (0–3).
    Fight { slot: u8, battle_move: PokemonMove },
    /// Use the bag item at `bag_slot` (index into `BattleState.bag`).
    UseItem { slot: u8, item: BagItem },
    /// Switch to the party Pokemon at `party_slot` (index into `BattleState.party`).
    SwitchPokemon { slot: u8, pokemon: PokemonSummary },
    /// Attempt to flee (wild battles only).
    Run,
    // ── Safari Zone battle actions (only offered when `battle_type == Safari`) ──
    /// Throw a Safari Ball to try to catch the Pokémon.
    SafariBall,
    /// Throw bait — makes the Pokémon less likely to flee but harder to catch.
    SafariBait,
    /// Throw a rock — makes the Pokémon easier to catch but more likely to flee.
    SafariRock,
}

impl Display for BattleAction {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            BattleAction::Fight { battle_move, .. } => write!(f, "FIGHT  {}  PP {}", battle_move.name, battle_move.pp),
            BattleAction::UseItem { item, .. } => write!(f, "ITEM   {} ×{}", item.id, item.quantity),
            BattleAction::SwitchPokemon { pokemon, .. } => write!(f, "PKMN   {:?}", pokemon),
            BattleAction::Run => write!(f, "RUN"),
            BattleAction::SafariBall => write!(f, "BALL"),
            BattleAction::SafariBait => write!(f, "BAIT"),
            BattleAction::SafariRock => write!(f, "ROCK"),
        }
    }
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
        // wBattleType: 0 = normal, 1 = old-man tutorial, 2 = Safari Zone. Safari overrides the
        // wild/trainer split (the menu is BALL/BAIT/ROCK/RUN, not FIGHT/PKMN/ITEM/RUN).
        let battle_type = if self.read_pointer(&pokered_symbols::wBattleType) == 2 {
            BattleType::Safari
        } else if is_in_battle == 2 {
            BattleType::Trainer
        } else {
            BattleType::Wild
        };

        // wPlayerMonNumber is the party index of the active Pokémon and is updated on a mid-battle
        // switch (wBattleMonPartyPos is not — it stays at the battle's starting mon, which made a
        // trained bench mon look like it never came in and the policy re-switch forever).
        let active_party_slot = self.read_pointer(&pokered_symbols::wPlayerMonNumber);

        // wPlayer/EnemyDisabledMove: high nibble = disabled move slot (1-based), low = turn counter.
        // Zero means no move is disabled. Convert to a 0-based slot so the disabled move is excluded
        // from selectable moves (otherwise the policy re-picks it forever against a "disabled!" wall).
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
            // `wEnemyBattleStatus1` bit 5 = `USING_TRAPPING_MOVE`
            // (`pokered/constants/battle_constants.asm:86`).
            enemy_trapping: self.read_pointer(&pokered_symbols::wEnemyBattleStatus1) & (1 << 5) != 0,
            enemy_catch_rate: self.read_pointer(&pokered_symbols::wEnemyMonActualCatchRate),
        })
    }
}

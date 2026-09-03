use std::fmt::{Display, Formatter};
use crate::mmu::MMU;
use crate::pokemon::bag::{Bag, BagItem};
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
use crate::ram::ROM;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::pokemon::{PokemonStats, PokemonSummary, PokemonType};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::symbols::{pokered_symbols, DmgPointer, DmgPointerRead};

#[derive(Debug, Clone, Copy, Eq, PartialEq, strum_macros::Display)]
pub enum BattleType { Wild, Trainer, Safari }

/// Whether the cartridge will refuse to let *any* move execute this battle, because it is a
/// **ghost** battle.
///
/// ⚠️ **It is not only the Marowak.** `IsGhostBattle` (`engine/battle/core.asm`) tests three things
/// and the species is not one of them: a **wild** battle (`wIsInBattle == 1`), on Pokémon Tower
/// 1F-7F, with no Silph Scope in the bag. So every Gastly, Haunter and Cubone in the tower is a
/// ghost until the Scope is found in the Rocket Hideout.
///
/// ⚠️ **A ghost battle cannot be won, cannot be caught, and cannot end by itself.** The player's
/// turn prints "… is too scared to move!" and the ghost's prints "GHOST: Get out...", so no move
/// lands and neither side ever loses a hit point: nothing resolves it however many turns go by.
/// `item_effects.asm` reads the same predicate to hard-code the can't-be-caught value, so a Poké
/// Ball is refused too, and every party member is equally scared, so a switch buys nothing either.
///
/// ⚠️ **Running is the one exit, and it is guaranteed.** `TryRunningFromBattle` jumps straight to
/// `.canEscape` on this predicate, *above* the speed check every other wild flee has to pass. So
/// [`crate::pokemon::policy::battle_options`] offers `Run` and nothing else — the same rule the
/// Safari menu and the HM field moves follow, that an action the game would refuse is not offered
/// at all.
///
/// The alternative — offer the menu and describe the trap in prose — is what
/// [`BattleState::enemy_trapping`] does, and it is right *there* because a wrap ends by itself in a
/// few turns and items, switching and running all still work. Nothing about this one ends by
/// itself. The deployed run of 2026-09-01 sat on Pokémon Tower 3F choosing Slash against a Gastly
/// every 3.3 s, and could not have noticed: a battle script was answering on the emulator thread,
/// so no request was made and the model was never asked, while the watchdog stayed quiet because
/// the agent was reaching a decision point every tick. It is the silent stall the zero-PP arm of
/// `battle_options` warns about, arrived at from the other side.
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
            // ⚠️ **Not `{:?}`, which is what this was.** `llm::tools::battle_menu` puts this string
            // straight into the turn request, so every switchable party member cost ~500 bytes of
            // Rust debug syntax — `PokemonSummary { species: Charizard, current_hp: 360, status:
            // None, types: [Fire, Flying], moves: [Some(PokemonMove { … }), …] }` — in the menu of
            // every battle turn. Same class of bug as `MetaTile`'s and `PokemonStatus`'s old `strum`
            // derives: a derive is a debugging default, and these strings are prose a model reads.
            // Found by reading `probe_turn_requests`' output, which is what it is for.
            //
            // The stats and the full move list are deliberately *not* restated: `read_party` answers
            // that, and the point of a menu row is which Pokémon to send out.
            BattleAction::SwitchPokemon { pokemon, .. } => {
                write!(f, "PKMN   {} Lv{} — {}/{} HP",
                       pokemon.species, pokemon.level, pokemon.current_hp, pokemon.stats.hp)?;
                // ⚠️ `PokemonStatus`' `Display` is `strum`'s, so a healthy Pokémon prints `None` —
                // a missing value rather than good news. Say nothing when there is nothing to say.
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

/// `wIsInBattle` when the player has just **lost**: not a battle, and not yet the overworld either.
///
/// ⚠️ **It is written by the overworld loop, not by the battle engine, and that is why it is a
/// window rather than an instant.** `home/overworld.asm:355-359` — `.allPokemonFainted` — stores
/// `$ff` here and only then calls `HandleBlackOut`, which fades the screen, halves the money, heals
/// the party and warps the player to the Pokémon Centre they last accepted a heal at. So for the
/// whole of that sequence the byte reads `$ff`, the party is still down, the money is still whole
/// and `wCurMap` is still the map the fight happened on.
///
/// Everything that asks "is there a battle" has to answer **no** here — the fight is over — and
/// everything that asks "may I put a decision to the policy" has to answer **no** as well, because
/// the map it would describe is about to stop existing. The two callers are [`read_battle_state`]
/// below and [`crate::pokemon::agent::blackout_in_flight`].
///
/// (`ResetStatusAndHalveMoneyOnBlackout` clears it back to 0 three instructions in
/// (`engine/events/black_out.asm:3-6`), so nothing has to time this out.)
pub const LOST_BATTLE: u8 = 0xff;

pub trait BattleStateReader {
    fn read_battle_state(&self) -> Option<BattleState>;
}

impl BattleStateReader for MMU {
    fn read_battle_state(&self) -> Option<BattleState> {
        let is_in_battle = self.read_pointer(&pokered_symbols::wIsInBattle);
        // ⚠️ [`LOST_BATTLE`] is a battle that has **ended**, and reading it as one put a live
        // `### Battle` block — the fainted Pokémon still "out", the enemy still on its last HP —
        // into the overworld turn the model was asked after a blackout. `read_game_mode` has always
        // treated `$ff` as not-a-battle (its `_` arm); this is the same byte and now the same answer.
        if is_in_battle == 0 || is_in_battle == LOST_BATTLE {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bag_with(items: &[ItemId]) -> Bag {
        Bag::new(items.iter().map(|&id| BagItem::new(id, 1)).collect())
    }

    /// Each of `IsGhostBattle`'s three conditions, one at a time, plus the boundary of the range it
    /// checks. ⚠️ **The species is deliberately not among them**: reading this as "the Marowak" is
    /// the mistake that let the deployed run fight a Gastly for as long as anyone watched.
    #[test]
    fn a_ghost_battle_is_a_wild_one_in_the_tower_without_the_scope() {
        let empty = bag_with(&[]);
        let scope = bag_with(&[ItemId::SilphScope]);

        // Every floor the ROM's range covers, and a wild battle on each is a ghost.
        for map in [Map::PokemonTower1F, Map::PokemonTower3F, Map::PokemonTower7F] {
            assert!(is_ghost_battle(map, &empty, BattleType::Wild), "{map} without the Scope");
            // ⚠️ The Scope is what ends it, and it ends it everywhere at once.
            assert!(!is_ghost_battle(map, &scope, BattleType::Wild), "{map} carrying the Scope");
        }

        // ⚠️ **Trainer battles are excluded by `wIsInBattle == 1`.** The Channelers on these floors
        // fight normally, so a gate that keyed on the map alone would have the run fleeing them.
        assert!(!is_ghost_battle(Map::PokemonTower3F, &empty, BattleType::Trainer));

        // ⚠️ **The two maps immediately either side of the range**, which is what a range check
        // wants guarding: the tower is `0x8E..=0x94`, so these are `0x8D` and `0x95`.
        assert!(!is_ghost_battle(Map::LavenderPokecenter, &empty, BattleType::Wild));
        assert!(!is_ghost_battle(Map::MrFujisHouse, &empty, BattleType::Wild));
    }
}

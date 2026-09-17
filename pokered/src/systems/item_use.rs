//! `UseItem_`'s dispatch over `ItemUsePtrTable`, and the arithmetic of the items that change a
//! party mon: `ItemUseMedicine` up to its HP bar, `ItemUseVitamin` up to its text, the Rare Candy up
//! to its `RedrawPartyMenu`, `CanLearnTM`, and the party menu's test of a stone.
//!
//! Exact, and pinned by harvested fixtures: which medicine heals what and by how much, the caps,
//! the Full Restore on a mon at full HP that is used as a Full Heal instead, the vitamin's
//! stat-experience rule, the Rare Candy's level, experience, stats and HP, and which species a
//! machine can teach. The screens around them are `modes::use_item`'s.

use poke_core::base_stats::BaseStats;
use poke_core::evos_moves::{EvosMoves, Evolution};
use poke_core::item::ItemId;
use poke_core::move_name::PokemonMoveName;
use poke_core::rom_gfx::rom_slice;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_symbols;
use serde::{Deserialize, Serialize};
use crate::party::PartyMon;
use crate::systems::experience::calc_experience;
use crate::systems::stats::calc_stats;

/// `MAX_LEVEL`.
pub const MAX_LEVEL: u8 = 100;

/// The routine `ItemUsePtrTable` names for an item, with `UseItem_`'s test for the machines ahead
/// of it. One arm per routine, so a chunk that recreates one has exactly one place to take it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemUse {
    Ball,
    TownMap,
    Bicycle,
    Surfboard,
    Pokedex,
    EvoStone,
    Medicine,
    /// `ItemUseBait`, which a Boulder Badge's id reaches.
    Bait,
    /// `ItemUseRock`, the Cascade Badge's.
    Rock,
    /// `UnusableItem`, which is `ItemUseNotTime` and nothing else.
    Unusable,
    EscapeRope,
    Repel,
    SuperRepel,
    MaxRepel,
    Vitamin,
    RareCandy,
    XAccuracy,
    CardKey,
    PokeDoll,
    GuardSpec,
    DireHit,
    XStat,
    PokeFlute,
    CoinCase,
    OldRod,
    GoodRod,
    SuperRod,
    OaksParcel,
    Itemfinder,
    PpUp,
    PpRestore,
    /// `ItemUseTMHM`, for every id from `HM01` up.
    TmHm,
}

impl ItemUse {
    pub fn of(item: ItemId) -> Self {
        use ItemId::*;
        match item {
            MasterBall | UltraBall | GreatBall | PokeBall | SafariBall => Self::Ball,
            TownMap => Self::TownMap,
            Bicycle => Self::Bicycle,
            Surfboard => Self::Surfboard,
            Pokedex => Self::Pokedex,
            MoonStone | FireStone | ThunderStone | WaterStone | LeafStone => Self::EvoStone,
            Antidote | BurnHeal | IceHeal | Awakening | ParlyzHeal | FullRestore | MaxPotion
            | HyperPotion | SuperPotion | Potion | FullHeal | Revive | MaxRevive | FreshWater
            | SodaPop | Lemonade => Self::Medicine,
            BoulderBadge => Self::Bait,
            CascadeBadge => Self::Rock,
            EscapeRope => Self::EscapeRope,
            Repel => Self::Repel,
            SuperRepel => Self::SuperRepel,
            MaxRepel => Self::MaxRepel,
            HpUp | Protein | Iron | Carbos | Calcium => Self::Vitamin,
            RareCandy => Self::RareCandy,
            XAccuracy => Self::XAccuracy,
            CardKey => Self::CardKey,
            PokeDoll => Self::PokeDoll,
            GuardSpec => Self::GuardSpec,
            DireHit => Self::DireHit,
            XAttack | XDefend | XSpeed | XSpecial => Self::XStat,
            PokeFlute => Self::PokeFlute,
            CoinCase => Self::CoinCase,
            OldRod => Self::OldRod,
            GoodRod => Self::GoodRod,
            SuperRod => Self::SuperRod,
            OaksParcel => Self::OaksParcel,
            Itemfinder => Self::Itemfinder,
            PpUp => Self::PpUp,
            Ether | MaxEther | Elixer | MaxElixer => Self::PpRestore,
            _ if item as u8 >= Hm01Cut as u8 => Self::TmHm,
            _ => Self::Unusable,
        }
    }
}

/// `PartyMenuItemUseMessagePointers` below `RARE_CANDY_MSG`: what the party menu says afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MedicineMessage {
    Antidote,
    BurnHeal,
    IceHeal,
    Awakening,
    ParlyzHeal,
    Potion,
    FullHeal,
    Revive,
}

impl MedicineMessage {
    /// The far text each pointer reaches.
    pub fn text(self) -> &'static str {
        match self {
            Self::Antidote => "_AntidoteText",
            Self::BurnHeal => "_BurnHealText",
            Self::IceHeal => "_IceHealText",
            Self::Awakening => "_AwakeningText",
            Self::ParlyzHeal => "_ParlyzHealText",
            Self::Potion => "_PotionText",
            Self::FullHeal => "_FullHealText",
            Self::Revive => "_ReviveText",
        }
    }
}

/// What a medicine did, as the screen needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Medicine {
    /// `.healingItemNoEffect`: nothing changed and nothing is used up.
    NoEffect,
    /// A status cured, with no HP bar to animate.
    Cured(MedicineMessage),
    /// `wHPBarOldHP` and `wHPBarNewHP`, for `UpdateHPBar2`.
    Healed { old: u16, new: u16, message: MedicineMessage },
}

/// Status bits: `SLP_MASK` and `PSN`, `BRN`, `FRZ`, `PAR`.
const SLP_MASK: u8 = 0b111;
const PSN: u8 = 1 << 3;
const BRN: u8 = 1 << 4;
const FRZ: u8 = 1 << 5;
const PAR: u8 = 1 << 6;

/// `ItemUseMedicine` from `.checkItemType` to `.doneHealing`, out of battle and not Softboiled.
///
/// A cure zeroes the whole status byte, not the one bit it tested. Every HP item adds its amount
/// before capping, including a Revive, which adds 20 and then throws that away for half the max;
/// a Full Restore or Max Potion adds 200 and then goes to the max whatever the sum was.
pub fn use_medicine(item: ItemId, mon: &mut PartyMon) -> Medicine {
    let item = item as u8;
    if item >= ItemId::Revive as u8 || (ItemId::FullRestore as u8..=ItemId::Potion as u8).contains(&item) {
        return heal_hp(item, mon);
    }
    cure(item, mon)
}

fn cure(item: u8, mon: &mut PartyMon) -> Medicine {
    let (message, mask) = match ItemId::from_repr(item) {
        Some(ItemId::Antidote) => (MedicineMessage::Antidote, PSN),
        Some(ItemId::BurnHeal) => (MedicineMessage::BurnHeal, BRN),
        Some(ItemId::IceHeal) => (MedicineMessage::IceHeal, FRZ),
        Some(ItemId::Awakening) => (MedicineMessage::Awakening, SLP_MASK),
        Some(ItemId::ParlyzHeal) => (MedicineMessage::ParlyzHeal, PAR),
        _ => (MedicineMessage::FullHeal, 0xFF),
    };
    if mon.mon.status & mask == 0 {
        return Medicine::NoEffect;
    }
    mon.mon.status = 0;
    Medicine::Cured(message)
}

fn heal_hp(item: u8, mon: &mut PartyMon) -> Medicine {
    let is = |id: ItemId| item == id as u8;
    let old = mon.mon.hp;
    let max = mon.stats[0];
    let reviving = is(ItemId::Revive) || is(ItemId::MaxRevive);
    if (old == 0) != reviving {
        return Medicine::NoEffect;
    }
    if old == max {
        // A Full Restore with nothing to heal becomes a Full Heal, `wCurItem` and all.
        if !is(ItemId::FullRestore) || mon.mon.status == 0 {
            return Medicine::NoEffect;
        }
        return cure(ItemId::FullHeal as u8, mon);
    }
    let amount: u16 = if is(ItemId::SodaPop) {
        60
    } else if item > ItemId::SodaPop as u8 {
        80
    } else if is(ItemId::FreshWater) {
        50
    } else if item < ItemId::SuperPotion as u8 {
        200
    } else if is(ItemId::SuperPotion) {
        50
    } else {
        20
    };
    let mut new = old.wrapping_add(amount);
    if is(ItemId::Revive) {
        new = max >> 1;
    } else if new >= max || item < ItemId::HyperPotion as u8 || is(ItemId::MaxRevive) {
        new = max;
    }
    mon.mon.hp = new;
    if is(ItemId::FullRestore) {
        mon.mon.status = 0;
    }
    let message = if reviving { MedicineMessage::Revive } else { MedicineMessage::Potion };
    Medicine::Healed { old, new, message }
}

/// `.softboiled`'s share: a fifth of the giver's max HP, which the giver pays and the target is
/// healed by. The giver must have more than that left, so it never faints itself.
pub fn softboiled_share(giver: &PartyMon) -> u16 {
    giver.stats[0] / 5
}

/// `ItemUseMedicine` with `wPseudoItemID` set: a Potion whose amount is the share rather than 20.
pub fn softboiled(share: u16, target: &mut PartyMon) -> Medicine {
    let (old, max) = (target.mon.hp, target.stats[0]);
    if old == 0 || old == max {
        return Medicine::NoEffect;
    }
    let new = old.saturating_add(share).min(max);
    target.mon.hp = new;
    Medicine::Healed { old, new, message: MedicineMessage::Potion }
}

/// `VitaminStats`: the stat each vitamin raises, as the text names it.
pub fn vitamin_stat_name(item: ItemId) -> &'static str {
    ["HEALTH", "ATTACK", "DEFENSE", "SPEED", "SPECIAL"][item as usize - ItemId::HpUp as usize]
}

/// `.useVitamin` up to `.gotStatName`: false where the stat's experience is already 25600 or more.
///
/// Only the high byte is tested and raised, by 10, so the low byte rides along untouched. The stats
/// are worked out again, the max HP with them, but the current HP is left where it was.
pub fn use_vitamin(item: ItemId, mon: &mut PartyMon) -> bool {
    let stat = item as usize - ItemId::HpUp as usize;
    let [high, low] = mon.mon.stat_exp[stat].to_be_bytes();
    if high >= 100 {
        return false;
    }
    mon.mon.stat_exp[stat] = u16::from_be_bytes([high + 10, low]);
    let base = BaseStats::of(mon.mon.species).stats;
    mon.stats = calc_stats(base, mon.mon.dvs, Some(mon.mon.stat_exp), mon.level);
    true
}

/// `CanLearnTM`: the species' `tmhm` bit for the first machine in `TechnicalMachines` that teaches
/// the move. The search has no terminator, so it is only ever asked about a machine's move.
pub fn can_learn_tm(species: PokemonSpecies, mv: PokemonMoveName) -> bool {
    let index = rom_slice(pokered_symbols::TechnicalMachines).iter()
        .position(|&machine| machine == mv as u8)
        .expect("a machine teaches the move");
    BaseStats::of(species).tm_hm[index / 8] & 1 << (index % 8) != 0
}

/// `RedrawPartyMenu_.evolutionStoneMenu`: whether any of the species' item evolutions takes this
/// stone. The level an entry asks for is not looked at, so a mon too young to evolve still reads
/// `ABLE`.
pub fn evolves_with(species: PokemonSpecies, stone: ItemId) -> bool {
    EvosMoves::of(species).evolutions.iter().any(|evolution| matches!(evolution, Evolution::Item { item, .. } if *item == stone))
}

/// `.useRareCandy` up to its `RedrawPartyMenu`: false at level 100. Otherwise a level up, the
/// experience set to the least that level needs, the stats worked out again, and the HP raised by
/// what the max HP gained.
///
/// The test is for exactly 100, so a mon somehow past it levels on, and the level is a byte.
pub fn use_rare_candy(mon: &mut PartyMon) -> bool {
    if mon.level == MAX_LEVEL {
        return false;
    }
    mon.level = mon.level.wrapping_add(1);
    let base = BaseStats::of(mon.mon.species);
    mon.mon.exp = calc_experience(base.growth_rate, mon.level);
    let old_max = mon.stats[0];
    mon.stats = calc_stats(base.stats, mon.mon.dvs, Some(mon.mon.stat_exp), mon.level);
    mon.mon.hp = mon.mon.hp.wrapping_add(mon.stats[0].wrapping_sub(old_max));
    true
}

/// A `CanLearnTM` case as its fixture stores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineOnSpecies {
    pub species: PokemonSpecies,
    pub mv: PokemonMoveName,
}

/// A medicine or vitamin case as its fixture stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemOnMon {
    pub item: ItemId,
    pub mon: PartyMon,
}

#[cfg(test)]
mod tests {
    use poke_core::species::PokemonSpecies;
    use crate::fixtures::cases;
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use super::*;

    fn mon(hp: u16, status: u8) -> PartyMon {
        let mut mon = new_party_mon(PokemonSpecies::Pidgey, 20, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
        mon.mon.hp = hp;
        mon.mon.status = status;
        mon
    }

    #[test]
    fn a_potion_heals_twenty_and_stops_at_the_max() {
        let max = mon(0, 0).stats[0];
        let mut hurt = mon(max - 30, 0);
        assert_eq!(use_medicine(ItemId::Potion, &mut hurt),
            Medicine::Healed { old: max - 30, new: max - 10, message: MedicineMessage::Potion });
        assert_eq!(use_medicine(ItemId::Potion, &mut hurt),
            Medicine::Healed { old: max - 10, new: max, message: MedicineMessage::Potion });
        assert_eq!(use_medicine(ItemId::Potion, &mut hurt), Medicine::NoEffect, "already full");
    }

    #[test]
    fn a_revive_is_for_the_fainted_only_and_gives_half() {
        let max = mon(0, 0).stats[0];
        assert_eq!(use_medicine(ItemId::Revive, &mut mon(5, 0)), Medicine::NoEffect);
        assert_eq!(use_medicine(ItemId::Potion, &mut mon(0, 0)), Medicine::NoEffect);
        assert_eq!(use_medicine(ItemId::Revive, &mut mon(0, 0)),
            Medicine::Healed { old: 0, new: max / 2, message: MedicineMessage::Revive });
    }

    #[test]
    fn a_full_restore_at_full_hp_cures_as_a_full_heal() {
        let max = mon(0, 0).stats[0];
        let mut poisoned = mon(max, PSN);
        assert_eq!(use_medicine(ItemId::FullRestore, &mut poisoned), Medicine::Cured(MedicineMessage::FullHeal));
        assert_eq!(poisoned.mon.status, 0);
        assert_eq!(use_medicine(ItemId::FullRestore, &mut mon(max, 0)), Medicine::NoEffect);
    }

    #[test]
    fn a_cure_needs_its_own_ailment_and_clears_the_whole_byte() {
        assert_eq!(use_medicine(ItemId::Antidote, &mut mon(10, PAR)), Medicine::NoEffect);
        let mut asleep_and_poisoned = mon(10, PSN | 2);
        assert_eq!(use_medicine(ItemId::Awakening, &mut asleep_and_poisoned), Medicine::Cured(MedicineMessage::Awakening));
        assert_eq!(asleep_and_poisoned.mon.status, 0, "the poison goes with it");
    }

    #[test]
    fn a_vitamin_adds_ten_to_the_high_byte_until_it_reaches_a_hundred() {
        let mut fed = mon(10, 0);
        fed.mon.stat_exp[1] = 99 << 8 | 0x42;
        assert!(use_vitamin(ItemId::Protein, &mut fed));
        assert_eq!(fed.mon.stat_exp[1], 109 << 8 | 0x42);
        assert!(!use_vitamin(ItemId::Protein, &mut fed), "109 is past a hundred");
    }

    #[test]
    fn the_dispatch_reads_the_table_and_the_machines() {
        assert_eq!(ItemUse::of(ItemId::Lemonade), ItemUse::Medicine);
        assert_eq!(ItemUse::of(ItemId::Nugget), ItemUse::Unusable);
        assert_eq!(ItemUse::of(ItemId::Tm01MegaPunch), ItemUse::TmHm);
        assert_eq!(ItemUse::of(ItemId::Hm05Flash), ItemUse::TmHm);
        assert_eq!(ItemUse::of(ItemId::MaxElixer), ItemUse::PpRestore);
    }

    #[test]
    fn every_harvested_medicine_matches() {
        for (input, (medicine, mon), _) in cases::<ItemOnMon, (Medicine, PartyMon)>(include_str!("../../fixtures/items/use_medicine.jsonl")) {
            let mut got = input.mon.clone();
            assert_eq!((use_medicine(input.item, &mut got), got), (medicine, mon), "{input:?}");
        }
    }

    #[test]
    fn a_rare_candy_levels_up_and_keeps_the_damage_taken() {
        let mut candied = mon(10, 0);
        let (before_max, before_level) = (candied.stats[0], candied.level);
        assert!(use_rare_candy(&mut candied));
        assert_eq!(candied.level, before_level + 1);
        assert_eq!(candied.mon.hp, 10 + candied.stats[0] - before_max);
        candied.level = MAX_LEVEL;
        assert!(!use_rare_candy(&mut candied));
    }

    #[test]
    fn a_machine_and_a_stone_are_read_from_the_cartridge() {
        assert!(can_learn_tm(PokemonSpecies::Pidgey, PokemonMoveName::Fly));
        assert!(!can_learn_tm(PokemonSpecies::Pidgey, PokemonMoveName::Surf));
        assert!(evolves_with(PokemonSpecies::Pikachu, ItemId::ThunderStone));
        assert!(!evolves_with(PokemonSpecies::Pikachu, ItemId::MoonStone));
        assert!(evolves_with(PokemonSpecies::Eevee, ItemId::FireStone), "the second of three");
    }

    #[test]
    fn every_harvested_machine_matches() {
        for (input, able, _) in cases::<MachineOnSpecies, bool>(include_str!("../../fixtures/items/can_learn_tm.jsonl")) {
            assert_eq!(can_learn_tm(input.species, input.mv), able, "{input:?}");
        }
    }

    #[test]
    fn every_harvested_rare_candy_matches() {
        for (input, (used, mon), _) in cases::<PartyMon, (bool, PartyMon)>(include_str!("../../fixtures/items/use_rare_candy.jsonl")) {
            let mut got = input.clone();
            assert_eq!((use_rare_candy(&mut got), got), (used, mon), "{input:?}");
        }
    }

    #[test]
    fn every_harvested_vitamin_matches() {
        for (input, (used, mon), _) in cases::<ItemOnMon, (bool, PartyMon)>(include_str!("../../fixtures/items/use_vitamin.jsonl")) {
            let mut got = input.mon.clone();
            assert_eq!((use_vitamin(input.item, &mut got), got), (used, mon), "{input:?}");
        }
    }
}

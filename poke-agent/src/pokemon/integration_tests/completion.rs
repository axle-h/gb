//! What playing the whole game means, and the ledger that holds a run to it.
//!
//! Every entry the cartridge has a table for (its maps, `trainer` headers, item balls, key items,
//! machines and `TradeMons`) is read out of that table, so something that moves upstream fails a
//! test here rather than going missing from the run.

use std::collections::{BTreeSet, HashSet};

use gb::mmu::MMU;
use poke_core::pointer::DmgPointer;
use gb::ram::ROM;
use strum::IntoEnumIterator;

use crate::pokemon::GameState;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::Map;
use crate::pokemon::map_header::MapHeaderReader;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::{pokered_events, pokered_symbols, pokered_toggles, DmgPointerRead};

/// Headers the ROM carries that no warp in any map targets, so nothing can walk into one.
pub(crate) const UNREACHABLE_DUPLICATES: [Map; 4] = [
    Map::CeruleanTrashedHouseCopy,
    Map::CinnabarMartCopy,
    Map::UndergroundPathRoute6Copy,
    Map::UndergroundPathRoute7Copy,
];

/// Why a map number is or is not somewhere a run could stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapBucket { Padding, LinkCable, Duplicate, Reachable }

pub(crate) fn classify(map: Map, name: &str) -> MapBucket {
    match map {
        _ if name.starts_with("UnusedMap") => MapBucket::Padding,
        Map::Colosseum | Map::TradeCenter => MapBucket::LinkCable,
        _ if UNREACHABLE_DUPLICATES.contains(&map) => MapBucket::Duplicate,
        _ => MapBucket::Reachable,
    }
}

/// Every way of coming by a Pokémon, and every thing done with one, that the run must show once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum_macros::EnumIter)]
pub enum Way {
    WildInGrass,
    WildOnACaveFloor,
    WildWhileSurfing,
    OldRod,
    GoodRod,
    SuperRod,
    /// Talked into a battle and caught: Mewtwo, Articuno, Zapdos and Moltres are four entries.
    Legendary(Legend),
    /// Both Snorlax, woken with the Poké Flute.
    SnorlaxOnRoute12,
    SnorlaxOnRoute16,
    /// A Power Plant Voltorb or Electrode, which stands where an item ball would.
    PowerPlantBall,
    GiftStarter,
    GiftLapras,
    GiftEevee,
    GiftFightingDojo,
    BoughtMagikarp,
    RevivedFossil,
    RevivedOldAmber,
    GameCornerPrize,
    DayCareWithdrawn,
    SafariCatch,
    SafariBait,
    SafariRock,
    SafariRun,
    /// The Safari game ended on its step count, not its balls.
    SafariOutOfSteps,
    EvolvedInBattle,
    EvolvedByRareCandy,
    EvolvedByStone,
    EvolutionCancelled,
    PcDeposit,
    PcWithdraw,
    PcRelease,
    PcChangeBox,
    /// A catch with a full party, which the game sends to the box.
    CaughtToTheBox,
    NicknameGiven,
    NicknameDeclined,
    NicknameAfterACatch,
    NicknameAfterAGift,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, strum_macros::EnumIter)]
pub enum Legend { #[default] Mewtwo, Articuno, Zapdos, Moltres }

impl Legend {
    fn species(self) -> PokemonSpecies {
        match self {
            Self::Mewtwo => PokemonSpecies::Mewtwo,
            Self::Articuno => PokemonSpecies::Articuno,
            Self::Zapdos => PokemonSpecies::Zapdos,
            Self::Moltres => PokemonSpecies::Moltres,
        }
    }
}

/// One thing the completion run must have done by the end.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Entry {
    Map(Map),
    /// A `trainer` header: `index` within `map`'s script.
    Trainer { map: Map, index: u8 },
    /// An item ball: `object` is its 1-based object number on `map`.
    ItemBall { map: Map, object: u8, item: u8 },
    /// A TM or HM, by item id, held at some point.
    Machine(u8),
    /// A key item, held at some point; one of `alternatives` where the game offers a choice.
    KeyItem(Vec<u8>),
    Badge(u8),
    /// An in-game trade, by the species it wants.
    Trade(PokemonSpecies),
    /// A quiz machine in the Cinnabar Gym answered right, rather than its gate beaten open.
    CinnabarQuiz,
    HallOfFame,
    Way(Way),
}

/// How a run shows an [`Entry`] done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// Stood on the map.
    Visited(Map),
    /// A flag byte and mask in RAM, which the game never clears once set.
    Flag { address: u16, mask: u8 },
    /// A toggle that starts set: done once it has been seen clear and is set again.
    Toggled { address: u16, mask: u8 },
    /// Any of these item ids held at any point.
    Held(Vec<u8>),
    Badge(u8),
    /// Species owned in the Pokédex.
    Owned(PokemonSpecies),
    /// `wNumHoFTeams` above zero.
    HallOfFame,
    /// Seen by [`Ledger::observe`] as it happened, because nothing left in RAM records it.
    Observed,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub entry: Entry,
    pub check: Check,
}

fn event(index: u16) -> Check {
    Check::Flag { address: pokered_symbols::wEventFlags.address + index / 8, mask: 1 << (index % 8) }
}

fn toggle(index: u16) -> Check {
    Check::Flag { address: pokered_symbols::wToggleableObjectFlags.address + index / 8, mask: 1 << (index % 8) }
}

/// The map whose `Map` name is `name`, as a sym label spells it.
pub(crate) fn map_named(name: &str) -> Option<Map> {
    Map::iter().find(|map| format!("{map:?}") == name)
}

/// One object on a map, as its `object_event` laid it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MapObject {
    /// 1-based, as `ToggleableObjectStates` and `HideObject` number them.
    pub number: u8,
    pub item: Option<u8>,
    pub trainer: bool,
}

/// Every object on `map`, from the objects table its header points at.
pub(crate) fn map_objects(mmu: &MMU, map: Map) -> Vec<MapObject> {
    let Ok(header) = mmu.read_map_header(map) else { return Vec::new() };
    let objects = header.objects_pointer();
    // Border block, then the warps (four bytes each) and the signs (three).
    let mut at = objects + 1;
    let warps = mmu.read_pointer(&at) as u16;
    at = at + 1 + warps * 4;
    let signs = mmu.read_pointer(&at) as u16;
    at = at + 1 + signs * 3;
    let count = mmu.read_pointer(&at);
    at = at + 1;
    let mut found = Vec::new();
    for number in 1..=count {
        let text = mmu.read_pointer(&(at + 5));
        // `object_event`: six bytes, plus a class and set for a trainer or an item id for a ball.
        let (item, trainer, size) = match text {
            t if t & 0x40 != 0 => (None, true, 8),
            t if t & 0x80 != 0 => (Some(mmu.read_pointer(&(at + 6))), false, 7),
            _ => (None, false, 6),
        };
        found.push(MapObject { number, item, trainer });
        at = at + size;
    }
    found
}

/// `ToggleableObjectStates`: each toggle flag's `(map, object)` and whether it starts shown, in
/// table order.
fn toggle_index(mmu: &MMU) -> Vec<(u8, u8, bool)> {
    /// `toggle_object_state`'s `ON`.
    const ON: u8 = 0x15;
    let mut entries = Vec::new();
    let mut at: DmgPointer = pokered_symbols::ToggleableObjectStates;
    loop {
        let map = mmu.read_pointer(&at);
        if map == 0xFF {
            return entries;
        }
        entries.push((map, mmu.read_pointer(&(at + 1)), mmu.read_pointer(&(at + 2)) == ON));
        at = at + 3;
    }
}

/// Key items the flags table marks that no player ever holds: not items at all, or never given.
/// The badges are flagged too, and are counted as badges.
const NOT_HELD: [ItemId; 5] = [ItemId::Surfboard, ItemId::SafariBall, ItemId::Pokedex, ItemId::Coin,
                               ItemId::UnusedItem2C];

/// Gifts the flags table does not mark as key items.
const GIFTS: [ItemId; 1] = [ItemId::ExpAll];

/// Pairs the game makes exclusive per save: one arm is taken, and `branch_points` proves the other.
const EXCLUSIVE: [[ItemId; 2]; 1] = [[ItemId::DomeFossil, ItemId::HelixFossil]];

/// The `TradeMons` entry no script hands out.
const UNUSED_TRADE: usize = 2;

/// The whole of the game the completion run is held to.
pub fn checklist(mmu: &MMU) -> Vec<Item> {
    let mut items = Vec::new();
    let mut push = |entry, check| items.push(Item { entry, check });

    for map in Map::iter().filter(|map| classify(*map, &format!("{map:?}")) == MapBucket::Reachable) {
        push(Entry::Map(map), Check::Visited(map));
    }

    for &(name, index, header) in pokered_symbols::TRAINER_HEADERS {
        let map = map_named(name).unwrap_or_else(|| panic!("trainer header {index} follows `{name}_Script`, no map"));
        // `trainer`: the flag's bit, the sight range, then the byte it counts from.
        let bit = mmu.read_pointer(&header) as u16;
        let byte = mmu.read_pointer_u16_le(&(header + 2));
        push(Entry::Trainer { map, index }, Check::Flag { address: byte + bit / 8, mask: 1 << (bit % 8) });
    }

    let toggles = toggle_index(mmu);
    for map in Map::iter().filter(|map| classify(*map, &format!("{map:?}")) == MapBucket::Reachable) {
        for object in map_objects(mmu, map) {
            // Two people are declared with an item of 0, and are no ball.
            let Some(item) = object.item.filter(|&item| item != 0) else { continue };
            let flag = toggles.iter().position(|&(on, number, _)| (on, number) == (map as u8, object.number))
                .unwrap_or_else(|| panic!("the {item:#04x} ball, object {} on {map}, has no toggle", object.number));
            let check = match (toggle(flag as u16), toggles[flag].2) {
                (check, true) => check,
                // Shown later by a script, as the Silph Scope is once Giovanni is beaten.
                (Check::Flag { address, mask }, false) => Check::Toggled { address, mask },
                _ => unreachable!("`toggle` is a flag"),
            };
            push(Entry::ItemBall { map, object: object.number, item }, check);
        }
    }

    for id in ItemId::Hm01Cut as u8..=ItemId::Tm50Substitute as u8 {
        push(Entry::Machine(id), Check::Held(vec![id]));
    }

    let key_flags = mmu.rom_data_from_rom_pointer(&pokered_symbols::KeyItemFlags, 15).to_vec();
    let exclusive = |id: u8| EXCLUSIVE.iter().find(|pair| pair.iter().any(|item| *item as u8 == id));
    let mut paired = BTreeSet::new();
    // The table stops at the last ordinary item; the ids after it are floors and machines.
    for id in 1..=(key_flags.len() * 8) as u8 {
        let flagged = key_flags[(id as usize - 1) / 8] & (1 << ((id - 1) % 8)) != 0;
        let Some(item) = ItemId::from_repr(id) else { continue };
        let badge = (ItemId::BoulderBadge as u8..=ItemId::EarthBadge as u8).contains(&id);
        if !flagged || NOT_HELD.contains(&item) || badge {
            continue;
        }
        let ids: Vec<u8> = match exclusive(id) {
            Some(pair) if paired.insert(pair[0] as u8) => pair.iter().map(|item| *item as u8).collect(),
            Some(_) => continue,
            None => vec![id],
        };
        push(Entry::KeyItem(ids.clone()), Check::Held(ids));
    }

    for gift in GIFTS {
        push(Entry::KeyItem(vec![gift as u8]), Check::Held(vec![gift as u8]));
    }

    for bit in 0..8 {
        push(Entry::Badge(bit), Check::Badge(bit));
    }

    // `npctrade`: give species, get species, a dialogue set, then an eleven-byte nickname.
    for trade in 0..=9usize {
        if trade == UNUSED_TRADE {
            continue;
        }
        let give = mmu.read_pointer(&(pokered_symbols::TradeMons + trade as u16 * 14));
        let species = PokemonSpecies::from_repr(give).expect("`TradeMons` names a species");
        push(Entry::Trade(species), Check::Flag {
            address: pokered_symbols::wCompletedInGameTradeFlags.address + trade as u16 / 8,
            mask: 1 << (trade % 8),
        });
    }

    push(Entry::CinnabarQuiz, Check::Observed);
    push(Entry::HallOfFame, Check::HallOfFame);

    for way in ways() {
        let check = match way {
            Way::Legendary(legend) => Check::Owned(legend.species()),
            Way::SnorlaxOnRoute12 => toggle(pokered_toggles::TOGGLE_ROUTE_12_SNORLAX),
            Way::SnorlaxOnRoute16 => toggle(pokered_toggles::TOGGLE_ROUTE_16_SNORLAX),
            Way::GiftStarter => event(pokered_events::EVENT_GOT_STARTER),
            // `BIT_GOT_LAPRAS`, a status bit rather than an event.
            Way::GiftLapras => Check::Flag { address: pokered_symbols::wStatusFlags4.address, mask: 0x01 },
            Way::GiftEevee => toggle(pokered_toggles::TOGGLE_CELADON_MANSION_EEVEE_GIFT),
            Way::BoughtMagikarp => event(pokered_events::EVENT_BOUGHT_MAGIKARP),
            // Either ball, since taking one is the whole choice: the two toggles share a byte.
            Way::GiftFightingDojo => Check::Flag {
                address: pokered_symbols::wToggleableObjectFlags.address
                    + pokered_toggles::TOGGLE_FIGHTING_DOJO_GIFT_1 / 8,
                mask: (1 << (pokered_toggles::TOGGLE_FIGHTING_DOJO_GIFT_1 % 8))
                    | (1 << (pokered_toggles::TOGGLE_FIGHTING_DOJO_GIFT_2 % 8)),
            },
            // Aerodactyl comes from the Old Amber and nowhere else; the other fossils' two likewise.
            Way::RevivedOldAmber => Check::Owned(PokemonSpecies::Aerodactyl),
            // Porygon is sold at the prize counter and found nowhere else.
            // Any prize will do, and the dearest one cannot be bought at all: the clerk refuses
            // to sell a 50-coin lot above 9 940, so 9 990 is the most coins money can hold.
            Way::GameCornerPrize => Check::Observed,
            _ => Check::Observed,
        };
        push(Entry::Way(way), check);
    }

    items
}

/// Every [`Way`], the legends expanded.
pub fn ways() -> Vec<Way> {
    Way::iter()
        .flat_map(|way| match way {
            Way::Legendary(_) => Legend::iter().map(Way::Legendary).collect::<Vec<_>>(),
            way => vec![way],
        })
        .collect()
}

/// What a run has done, folded in tick by tick; flags are read at the end because they stay set.
#[derive(Debug, Default, Clone)]
pub struct Ledger {
    visited: BTreeSet<Map>,
    held: BTreeSet<u8>,
    observed: HashSet<Entry>,
    /// Every [`Check::Toggled`] in the list, and whether it has been seen clear yet.
    toggled: Vec<(u16, u8, bool)>,
}

impl Ledger {
    pub fn new(list: &[Item]) -> Self {
        let toggled = list.iter().filter_map(|item| match item.check {
            Check::Toggled { address, mask } => Some((address, mask, false)),
            _ => None,
        }).collect();
        Self { toggled, ..Self::default() }
    }

    /// Fold one tick's game state in.
    pub fn observe(&mut self, state: &GameState, mmu: &MMU) {
        self.visited.insert(state.map.map);
        self.held.extend(state.bag.iter().map(|item| item.id as u8));
        for (address, mask, seen_clear) in self.toggled.iter_mut() {
            *seen_clear |= mmu.read(*address) & *mask == 0;
        }
    }

    /// Record something only the moment shows, such as a quiz answered or a Pokémon released.
    pub fn saw(&mut self, entry: Entry) {
        self.observed.insert(entry);
    }

    pub fn done(&self, item: &Item, mmu: &MMU, state: &GameState) -> bool {
        match &item.check {
            Check::Visited(map) => self.visited.contains(map),
            Check::Flag { address, mask } => mmu.read(*address) & mask != 0,
            Check::Toggled { address, mask } => mmu.read(*address) & mask != 0
                && self.toggled.iter().any(|&(a, m, seen_clear)| (a, m) == (*address, *mask) && seen_clear),
            Check::Held(ids) => ids.iter().any(|id| self.held.contains(id)),
            Check::Badge(bit) => state.badges.bits() & (1 << bit) != 0,
            Check::Owned(species) => state.pokedex_owned.contains(species),
            Check::HallOfFame => state.hall_of_fame_teams > 0,
            Check::Observed => self.observed.contains(&item.entry),
        }
    }

    /// Every entry of `list` not done, for the assertion that ends a run.
    pub fn missing(&self, list: &[Item], mmu: &MMU, state: &GameState) -> Vec<Entry> {
        list.iter().filter(|item| !self.done(item, mmu, state)).map(|item| item.entry.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::integration_tests::TestFixture;
    use std::time::Duration;

    fn count(list: &[Item], kind: fn(&Entry) -> bool) -> usize {
        list.iter().filter(|item| kind(&item.entry)).count()
    }

    /// The checklist is the cartridge's own tables, read the way the game reads them.
    #[test]
    fn checklist() {
        let gb = gb::game_boy::GameBoy::dmg(crate::pokemon::roms::POKERED);
        let list = super::checklist(gb.core().mmu());

        assert_eq!(count(&list, |e| matches!(e, Entry::Map(_))), 220, "the sweep's denominator");
        assert_eq!(count(&list, |e| matches!(e, Entry::Trainer { .. })), pokered_symbols::TRAINER_HEADERS.len());
        assert_eq!(count(&list, |e| matches!(e, Entry::Machine(_))), 55, "fifty TMs and five HMs");
        assert_eq!(count(&list, |e| matches!(e, Entry::Badge(_))), 8);

        let trades: Vec<PokemonSpecies> = list.iter()
            .filter_map(|item| match item.entry { Entry::Trade(give) => Some(give), _ => None })
            .collect();
        let known: Vec<PokemonSpecies> = crate::pokemon::postgame::trades::TRADES.iter().map(|t| t.give).collect();
        assert_eq!(trades, known, "`TradeMons` and the trade table disagree");

        // Every ball sits where its toggle says, so a pickup is a flag the ledger can read.
        let balls = count(&list, |e| matches!(e, Entry::ItemBall { .. }));
        assert!(balls > 100, "only {balls} item balls were found in the maps' object tables");

        let keys: Vec<Vec<u8>> = list.iter()
            .filter_map(|item| match &item.entry { Entry::KeyItem(ids) => Some(ids.clone()), _ => None })
            .collect();
        for wanted in [ItemId::Bicycle, ItemId::SilphScope, ItemId::PokeFlute, ItemId::CardKey, ItemId::SSTicket] {
            assert!(keys.iter().any(|ids| ids.contains(&(wanted as u8))), "{wanted:?} is not a key item here");
        }
        assert!(keys.iter().any(|ids| ids.len() == 2), "the fossil choice is one entry with two arms");
        assert_eq!(count(&list, |e| matches!(e, Entry::Way(_))), ways().len());
    }

    /// A finished save shows the flags done and a fresh one shows nothing.
    #[test]
    fn the_ledger_reads_a_finished_game_as_finished() {
        let mut finished = TestFixture::new(
            include_bytes!("../data/postgame-entry.bin"), Duration::from_secs(10), vec![]);
        let state = finished.game_state();
        let list = super::checklist(finished.gb.core().mmu());
        let ledger = Ledger::new(&list);
        let mmu = finished.gb.core().mmu();
        let missing = ledger.missing(&list, mmu, &state);
        for done in [Entry::HallOfFame, Entry::Badge(7), Entry::Trainer { map: Map::ViridianGym, index: 0 }] {
            assert!(!missing.contains(&done), "{done:?} reads as not done on a finished save");
        }

        let mut fresh = TestFixture::new(
            include_bytes!("../data/start-of-game-state.bin"), Duration::from_secs(10), vec![]);
        let state = fresh.game_state();
        let mmu = fresh.gb.core().mmu();
        let flags_done = list.iter()
            .filter(|item| matches!(item.check, Check::Flag { .. } | Check::Toggled { .. } | Check::Badge(_) | Check::HallOfFame))
            .filter(|item| ledger.done(item, mmu, &state))
            .count();
        assert_eq!(flags_done, 0, "a fresh save has done something already");

        // What only the moment shows is folded in as it happens.
        let mut ledger = ledger;
        ledger.observe(&state, mmu);
        ledger.saw(Entry::CinnabarQuiz);
        let missing = ledger.missing(&list, mmu, &state);
        assert!(!missing.contains(&Entry::Map(Map::RedsHouse2F)), "the map stood on was not ticked off");
        assert!(!missing.contains(&Entry::CinnabarQuiz), "an observed entry was not ticked off");
        assert!(missing.contains(&Entry::Map(Map::PalletTown)), "a map never stood on was ticked off");
    }
}

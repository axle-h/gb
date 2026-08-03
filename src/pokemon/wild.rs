//! Wild encounter tables, decoded from the ROM.
//!
//! Which species a map can produce, at what level, and how often. Added for **H5** of
//! `docs/postgame-coverage-plan.md` — the Exp.All aide wants 50 species owned, which is a catching
//! errand, and a catching errand needs to know where things live.
//!
//! Derived rather than transcribed, for the reason [`crate::pokemon::world_graph`] builds itself from
//! ROM headers: the alternative is a table of a few hundred hand-copied rows, and a wrong one does not
//! error — the agent paces in grass for its whole budget waiting for a species that was never there.
//! It also keeps the numbers honest. `postgame::safari`'s hunt is fast because it hunts each species
//! where its slot is fattest, and the same species can sit at 4.3 % on one map and 1.2 % on another;
//! that is a fact about the ROM, and this is where it comes from.
//!
//! # Format
//!
//! `WildDataPointers` (bank 3) is one 2-byte pointer per map id. Each entry is
//! `db grass_rate`, ten `db level, species` slots, `db water_rate`, ten more — except that a **rate of
//! zero omits its ten slots entirely** (`macros/asserts.asm` asserts exactly that), so the water block
//! cannot be found at a fixed offset.
//!
//! The per-slot probabilities are `WildMonEncounterSlotChances`, a **cumulative** table: a slot's own
//! share is its threshold minus the previous one. Duplicated species share slots, which is why
//! [`WildEncounters::species`] sums them.

use crate::pokemon::map::Map;
use crate::pokemon::roms;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::{pokered_symbols, DmgBank, DmgPointer};

/// Cumulative thresholds from `data/wild/probabilities.asm`; slot *i*'s share is
/// `CHANCES[i] - CHANCES[i-1]` out of 256.
const SLOT_CHANCES: [u16; 10] = [51, 102, 141, 166, 191, 216, 229, 242, 253, 256];

/// The ten slots of one encounter block, in ROM order.
pub type Slots = [(u8, PokemonSpecies); 10];

/// One map's wild encounter data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WildEncounters {
    /// Steps-in-grass encounter rate out of 256. Zero means this map has no grass encounters at all,
    /// and then `grass` is empty.
    pub grass_rate: u8,
    pub grass: Vec<(u8, PokemonSpecies)>,
    /// Surfing encounter rate out of 256, and its own ten slots. Every water block in the game uses
    /// the same ten slots for one or two species.
    pub water_rate: u8,
    pub water: Vec<(u8, PokemonSpecies)>,
}

/// Where an encounter comes from — the two blocks are independent, and reaching the water one needs
/// Surf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terrain { Grass, Water }

impl WildEncounters {
    /// Distinct species in `terrain`'s block, each with its **summed** share of an encounter (0.0–1.0)
    /// and the highest level it appears at, most likely first.
    ///
    /// Summed because a species routinely occupies several slots — Route 11's Ekans holds three, which
    /// is 45 % of the route, not the 19.9 % its best slot would suggest.
    pub fn species(&self, terrain: Terrain) -> Vec<(PokemonSpecies, f64, u8)> {
        let (slots, rate) = match terrain {
            Terrain::Grass => (&self.grass, self.grass_rate),
            Terrain::Water => (&self.water, self.water_rate),
        };
        if rate == 0 { return Vec::new(); }
        let mut out: Vec<(PokemonSpecies, f64, u8)> = Vec::new();
        for (i, &(level, species)) in slots.iter().enumerate() {
            let share = (SLOT_CHANCES[i] - if i == 0 { 0 } else { SLOT_CHANCES[i - 1] }) as f64 / 256.0;
            match out.iter_mut().find(|(s, _, _)| *s == species) {
                Some(entry) => { entry.1 += share; entry.2 = entry.2.max(level); }
                None => out.push((species, share, level)),
            }
        }
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out
    }

    /// Every species this map can produce on foot *or* on the water.
    pub fn all_species(&self) -> Vec<PokemonSpecies> {
        let mut out: Vec<PokemonSpecies> = self.species(Terrain::Grass).into_iter()
            .chain(self.species(Terrain::Water))
            .map(|(s, _, _)| s).collect();
        out.dedup();
        out
    }
}

/// `map`'s wild encounter table, or `None` when it has none at all (every indoor map, and the maps
/// whose pointer is `NothingWildMons`).
pub fn encounters(map: Map) -> Option<WildEncounters> {
    let pointers = &pokered_symbols::WildDataPointers;
    let DmgBank::ROM { bank } = pointers.bank else { panic!("WildDataPointers is not in ROM") };
    let table = rom_at(pointers);
    let index = map as usize * 2;
    let data = rom_bank_at(bank, u16::from_le_bytes([table[index], table[index + 1]]));

    let mut i = 0;
    let mut block = |data: &[u8], i: &mut usize| -> (u8, Vec<(u8, PokemonSpecies)>) {
        let rate = data[*i];
        *i += 1;
        if rate == 0 { return (0, Vec::new()); }
        let slots = (0..10).map(|s| {
            let (level, id) = (data[*i + s * 2], data[*i + s * 2 + 1]);
            (level, PokemonSpecies::from_repr(id)
                .unwrap_or_else(|| panic!("wild slot {s} on {map} is species id ${id:02x}")))
        }).collect();
        *i += 20;
        (rate, slots)
    };
    let (grass_rate, grass) = block(data, &mut i);
    let (water_rate, water) = block(data, &mut i);
    if grass_rate == 0 && water_rate == 0 {
        return None;
    }
    Some(WildEncounters { grass_rate, grass, water_rate, water })
}

fn rom_at(ptr: &DmgPointer) -> &'static [u8] {
    let DmgBank::ROM { bank } = ptr.bank else { panic!("{ptr:?} is not in ROM") };
    rom_bank_at(bank, ptr.address)
}

fn rom_bank_at(bank: u8, address: u16) -> &'static [u8] {
    let offset = if bank == 0 { address as usize }
                 else { bank as usize * 0x4000 + (address as usize - 0x4000) };
    &roms::POKERED[offset..]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Route 11 spelled out against `data/wild/maps/Route11.asm`, including the `_RED` branch — the
    /// same file assembles a Blue table with Sandshrew where Ekans is, so this also pins that the
    /// bundled ROM is the Red one.
    #[test]
    fn route11_matches_the_rom() {
        let wild = encounters(Map::Route11).expect("Route 11 has grass");
        assert_eq!(wild.grass_rate, 15);
        assert_eq!(wild.water_rate, 0, "Route 11 has no water encounters");
        let grass = wild.species(Terrain::Grass);
        // Ekans holds slots 0/2/6 (40.2 %), Spearow 1/4/7 (34.8 %), Drowzee 3/5/8/9 (25.0 %) — so
        // the *rarest* species here has four of the ten slots. Slot count is not rarity order.
        assert_eq!(grass.iter().map(|(s, _, _)| *s).collect::<Vec<_>>(),
            vec![PokemonSpecies::Ekans, PokemonSpecies::Spearow, PokemonSpecies::Drowzee]);
        let ekans = grass.iter().find(|(s, _, _)| *s == PokemonSpecies::Ekans).unwrap();
        assert!((ekans.1 - (51.0 + 39.0 + 13.0) / 256.0).abs() < 1e-9, "Ekans holds three slots");
        assert_eq!(ekans.2, 15, "the highest Ekans slot is level 15");
    }

    /// The slot table is cumulative and must end at exactly 256, or every share is wrong by a little.
    #[test]
    fn slot_shares_sum_to_one() {
        assert_eq!(SLOT_CHANCES[9], 256);
        let wild = encounters(Map::Route11).unwrap();
        let total: f64 = wild.species(Terrain::Grass).iter().map(|(_, p, _)| p).sum();
        assert!((total - 1.0).abs() < 1e-9, "shares summed to {total}");
    }

    /// A water-only map: the grass rate is zero, which means its ten slots are **not in the ROM** and
    /// the water block starts one byte in. Getting that wrong reads twenty bytes of someone else's
    /// table and still parses.
    #[test]
    fn a_water_only_map_has_no_grass_slots() {
        let wild = encounters(Map::Route19).expect("Route 19 is open sea");
        assert_eq!(wild.grass_rate, 0);
        assert!(wild.grass.is_empty());
        assert!(wild.water_rate > 0 && !wild.water.is_empty());
        assert!(wild.species(Terrain::Water).iter().any(|(s, _, _)| *s == PokemonSpecies::Tentacool));
    }

    /// Indoor maps have no table at all — the decoder must say so rather than reading junk.
    #[test]
    fn an_indoor_map_has_no_encounters() {
        assert_eq!(encounters(Map::ViridianPokecenter), None);
        assert_eq!(encounters(Map::Route11Gate2F), None);
    }
}

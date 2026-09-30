//! Wild encounter tables.

use crate::map::Map;
use crate::species::PokemonSpecies;
use crate::tables::{WILD_DATA, WILD_MON_ENCOUNTER_SLOT_CHANCES};

/// Slot `i`'s share of encounters.
fn share(i: usize) -> f64 {
    f64::from(WILD_MON_ENCOUNTER_SLOT_CHANCES[i]) / 256.0
}

pub fn base_exp(species: PokemonSpecies) -> u8 {
    crate::base_stats::BaseStats::of(species).base_exp
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WildEncounters {
    /// Encounter rate per step out of 256; zero means no grass encounters and an empty `grass`.
    pub grass_rate: u8,
    pub grass: Vec<(u8, PokemonSpecies)>,
    /// Surfing encounter rate per step out of 256.
    pub water_rate: u8,
    pub water: Vec<(u8, PokemonSpecies)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terrain { Grass, Water }

impl WildEncounters {
    /// Distinct species with their summed share (0.0–1.0) and top level, most likely first.
    pub fn species(&self, terrain: Terrain) -> Vec<(PokemonSpecies, f64, u8)> {
        let (slots, rate) = match terrain {
            Terrain::Grass => (&self.grass, self.grass_rate),
            Terrain::Water => (&self.water, self.water_rate),
        };
        if rate == 0 { return Vec::new(); }
        let mut out: Vec<(PokemonSpecies, f64, u8)> = Vec::new();
        for (i, &(level, species)) in slots.iter().enumerate() {
            let share = share(i);
            match out.iter_mut().find(|(s, _, _)| *s == species) {
                Some(entry) => { entry.1 += share; entry.2 = entry.2.max(level); }
                None => out.push((species, share, level)),
            }
        }
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out
    }

    fn block(&self, terrain: Terrain) -> (u8, &[(u8, PokemonSpecies)]) {
        match terrain {
            Terrain::Grass => (self.grass_rate, &self.grass),
            Terrain::Water => (self.water_rate, &self.water),
        }
    }

    /// Experience from one KO by a single Pokémon: each slot's `base_exp * level / 7`, weighted.
    pub fn expected_exp(&self, terrain: Terrain) -> f64 {
        let (rate, slots) = self.block(terrain);
        if rate == 0 { return 0.0; }
        slots.iter().enumerate().map(|(i, &(level, species))| {
            share(i) * f64::from(base_exp(species)) * f64::from(level) / 7.0
        }).sum()
    }

    /// Experience per step: `TryDoWildEncounter` rolls once per step against the map's rate.
    pub fn exp_per_step(&self, terrain: Terrain) -> f64 {
        let (rate, _) = self.block(terrain);
        f64::from(rate) / 256.0 * self.expected_exp(terrain)
    }

    /// The share of `terrain`'s encounters that are Poison-type.
    pub fn poison_share(&self, terrain: Terrain) -> f64 {
        let (rate, slots) = self.block(terrain);
        if rate == 0 { return 0.0; }
        use crate::pokemon::PokemonType::Poison;
        slots.iter().enumerate().filter_map(|(i, &(_, species))| {
            let meta = species.metadata();
            (meta.type1 == Poison || meta.type2 == Some(Poison)).then(|| share(i))
        }).sum()
    }

}

/// `map`'s table, or `None` for every indoor map and those pointing at `NothingWildMons`.
pub fn encounters(map: Map) -> Option<WildEncounters> {
    let data = &WILD_DATA[map as usize];
    if data.grass_rate == 0 && data.water_rate == 0 {
        return None;
    }
    let slots = |slots: &[(u8, u8)]| slots.iter().enumerate().map(|(s, &(level, id))| {
        (level, PokemonSpecies::from_repr(id).unwrap_or_else(|| panic!("wild slot {s} on {map} is species id ${id:02x}")))
    }).collect();
    Some(WildEncounters { grass_rate: data.grass_rate, grass: slots(data.grass), water_rate: data.water_rate, water: slots(data.water) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Route 11 as `data/wild/maps/Route11.asm` has it; Ekans, not Blue's Sandshrew, pins a Red build.
    #[test]
    fn route11_is_the_red_list() {
        let wild = encounters(Map::Route11).expect("Route 11 has grass");
        assert_eq!(wild.grass_rate, 15);
        assert_eq!(wild.water_rate, 0, "Route 11 has no water encounters");
        let grass = wild.species(Terrain::Grass);
        // Ekans holds three slots and Drowzee four, yet Drowzee is the rarest.
        assert_eq!(grass.iter().map(|(s, _, _)| *s).collect::<Vec<_>>(),
            vec![PokemonSpecies::Ekans, PokemonSpecies::Spearow, PokemonSpecies::Drowzee]);
        let ekans = grass.iter().find(|(s, _, _)| *s == PokemonSpecies::Ekans).unwrap();
        assert!((ekans.1 - (51.0 + 39.0 + 13.0) / 256.0).abs() < 1e-9, "Ekans holds three slots");
        assert_eq!(ekans.2, 15, "the highest Ekans slot is level 15");
    }

    /// The chances sum to exactly 256, so the shares sum to one.
    #[test]
    fn slot_shares_sum_to_one() {
        assert_eq!(WILD_MON_ENCOUNTER_SLOT_CHANCES.iter().map(|&chance| u16::from(chance)).sum::<u16>(), 256);
        let wild = encounters(Map::Route11).unwrap();
        let total: f64 = wild.species(Terrain::Grass).iter().map(|(_, p, _)| p).sum();
        assert!((total - 1.0).abs() < 1e-9, "shares summed to {total}");
    }

    /// A zero grass rate means no grass slots.
    #[test]
    fn a_water_only_map_has_no_grass_slots() {
        let wild = encounters(Map::Route19).expect("Route 19 is open sea");
        assert_eq!(wild.grass_rate, 0);
        assert!(wild.grass.is_empty());
        assert!(wild.water_rate > 0 && !wild.water.is_empty());
        assert!(wild.species(Terrain::Water).iter().any(|(s, _, _)| *s == PokemonSpecies::Tentacool));
    }

    /// Indoor maps have no table.
    #[test]
    fn an_indoor_map_has_no_encounters() {
        assert_eq!(encounters(Map::ViridianPokecenter), None);
        assert_eq!(encounters(Map::Route11Gate2F), None);
    }

    #[test]
    fn base_exp_is_the_yield_not_the_catch_rate() {
        assert_eq!(base_exp(PokemonSpecies::Bulbasaur), 64);
        assert_eq!(base_exp(PokemonSpecies::Chansey), 255);
        assert_eq!(base_exp(PokemonSpecies::Rattata), 57);
        assert_eq!(base_exp(PokemonSpecies::Mew), 64, "Mew's entry is in a bank of its own");
    }

    /// Every grind site ranked by EXP per step (`--features slow-tests`, `--ignored --nocapture`).
    #[test]
    #[cfg(feature = "slow-tests")]
    #[ignore = "probe — run with --ignored --nocapture, see the doc comment"]
    fn probe_grind_sites() {
        use strum::IntoEnumIterator;

        let mut rows: Vec<(f64, String)> = Vec::new();
        for map in Map::iter() {
            let Some(wild) = encounters(map) else { continue };
            for terrain in [Terrain::Grass, Terrain::Water] {
                let (rate, slots) = wild.block(terrain);
                if rate == 0 { continue; }
                let per_step = wild.exp_per_step(terrain);
                let levels = slots.iter().map(|&(l, _)| l);
                let (lo, hi) = (levels.clone().min().unwrap(), levels.max().unwrap());
                let who = wild.species(terrain).iter().take(4)
                    .map(|(s, share, _)| format!("{s} {:.0}%", share * 100.0))
                    .collect::<Vec<_>>().join(", ");
                rows.push((per_step, format!(
                    "{per_step:7.1}  {:6.0}  {rate:3}/256  lv{lo:>2}-{hi:<2}  {:3.0}% poison  {map:?} ({terrain:?}): {who}",
                    wild.expected_exp(terrain), wild.poison_share(terrain) * 100.0)));
            }
        }
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("exp/step  exp/KO    rate    levels    poison  map");
        for (_, line) in &rows { println!("{line}"); }
        println!("\n{} encounter blocks", rows.len());
    }
}

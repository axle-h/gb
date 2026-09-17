//! `TryDoWildEncounter`: whether the square the player has stepped onto starts a wild battle, and
//! with what. The tiles come from the map's blocks, where the cartridge reads the drawn screen.

use poke_core::map::Map;
use poke_core::map_gfx::tileset_entry;
use poke_core::map_header::TileSetId;
use poke_core::map_objects::FIRST_INDOOR_MAP;
use poke_core::species::PokemonSpecies;
use poke_core::wild::encounters;
use serde::{Deserialize, Serialize};
use crate::rng::Rng;
use super::collision;

/// `WildMonEncounterSlotChances`: the cumulative chance of each of the ten slots, less one.
const WILD_MON_ENCOUNTER_SLOT_CHANCES: [u8; 10] = [50, 101, 140, 165, 190, 215, 228, 241, 252, 255];
/// The water tile, in every tileset that has one.
const WATER_TILE: u8 = 0x14;

/// `wGrassRate`, `wGrassMons`, `wWaterRate` and `wWaterMons`: `(level, species)` a slot. A map
/// without one of the two keeps what the last map with it loaded, and a left shore can read it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WildMons {
    pub grass_rate: u8,
    pub grass: [(u8, u8); 10],
    pub water_rate: u8,
    pub water: [(u8, u8); 10],
}

impl WildMons {
    /// `LoadWildData`.
    pub fn load(&mut self, map: Map) {
        let wild = encounters(map);
        let slots = |slots: &[(u8, PokemonSpecies)]| std::array::from_fn(|i| (slots[i].0, slots[i].1 as u8));
        self.grass_rate = wild.as_ref().map_or(0, |wild| wild.grass_rate);
        if let Some(wild) = wild.as_ref().filter(|wild| wild.grass_rate != 0) {
            self.grass = slots(&wild.grass);
        }
        self.water_rate = wild.as_ref().map_or(0, |wild| wild.water_rate);
        if let Some(wild) = wild.as_ref().filter(|wild| wild.water_rate != 0) {
            self.water = slots(&wild.water);
        }
    }
}

/// What the routine reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterInput {
    pub map: Map,
    pub tileset: TileSetId,
    /// Screen cells (8, 9) and (9, 9): the bottom of the half block the player stands in.
    pub bottom_left: u8,
    pub bottom_right: u8,
    pub x: u8,
    pub y: u8,
    /// `wCurMapWidth` and `wCurMapHeight`, in blocks.
    pub width: u8,
    pub height: u8,
    pub repel_steps: u8,
    /// `wPartyMon1Level`.
    pub lead_level: u8,
    pub wild: WildMons,
}

/// What it leaves: the mon that appears, and what is left of the repel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Encounter {
    pub mon: Option<(PokemonSpecies, u8)>,
    pub repel_steps: u8,
    /// The repel ran out on this step, which says so (`TEXT_REPEL_WORE_OFF`) and starts nothing.
    pub repel_wore_off: bool,
}

/// `TryDoWildEncounter` past its `wNPCMovementScriptPointerTableNum` and `wMovementFlags` tests,
/// which are the caller's. The two random bytes are `hRandomAdd` and `hRandomSub`, drawn once the
/// square is one an encounter can happen on.
pub fn try_do_wild_encounter(input: &EncounterInput, rng: &mut impl Rng) -> Encounter {
    let mut out = Encounter { mon: None, repel_steps: input.repel_steps, repel_wore_off: false };
    if collision::on_door_or_warp_tile(input.tileset, input.bottom_left).0 {
        return out;
    }
    // `IsPlayerJustOutsideMap`.
    let outside = |at: u8, size: u8| at == size.wrapping_mul(2) || at == 0xFF;
    if outside(input.y, input.height) || outside(input.x, input.width) {
        return out;
    }
    if out.repel_steps != 0 {
        out.repel_steps -= 1;
        if out.repel_steps == 0 {
            out.repel_wore_off = true;
            return out;
        }
    }
    let wild = &input.wild;
    let (grass_rate, water_rate) = (wild.grass_rate, wild.water_rate);
    let rate = if input.bottom_right == tileset_entry(input.tileset).grass_tile {
        grass_rate
    } else if input.bottom_right == WATER_TILE {
        water_rate
    } else if input.map as u8 >= FIRST_INDOOR_MAP && input.tileset != TileSetId::Forest {
        grass_rate
    } else {
        return out;
    };
    let (add, sub) = (rng.random(), rng.random());
    if add >= rate {
        return out;
    }
    let slot = WILD_MON_ENCOUNTER_SLOT_CHANCES.iter().position(|&chance| chance >= sub).expect("the last chance is 255");
    // A left shore's bottom right is water and its bottom left is not, so it finds grass mons.
    let mons = if input.bottom_left == WATER_TILE { &wild.water } else { &wild.grass };
    let (level, species) = mons[slot];
    // Species 0 is what an unloaded table holds, a glitch battle and a non-goal.
    let Some(species) = PokemonSpecies::from_repr(species) else { return out };
    if out.repel_steps != 0 && level < input.lead_level {
        return out;
    }
    out.mon = Some((species, level));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::cases;
    use crate::rng::GameRng;

    #[test]
    fn every_harvested_case_of_try_do_wild_encounter() {
        for (input, output, rng) in cases::<EncounterInput, Encounter>(include_str!("../../../fixtures/overworld/try_do_wild_encounter.jsonl")) {
            let mut tape = GameRng::tape(rng.clone());
            assert_eq!(try_do_wild_encounter(&input, &mut tape), output, "{input:?} {rng:?}");
            assert!(matches!(tape, GameRng::Tape { cursor, .. } if cursor == rng.len()), "every random byte taken: {input:?}");
        }
    }
}

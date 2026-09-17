//! `HealParty` and `SetLastBlackoutMap`.

use poke_core::map::Map;
use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::pokered_symbols;
use crate::party::{Named, PartyMon};
use poke_core::moves::MoveData;
use crate::systems::pp::{add_bonus_pp, max_pp, PP_UP_MASK};

/// `HealParty`, with `RestoreBonusPP` after it: full HP, no status, and every move's PP at its max
/// for the PP Ups it has had. An empty move slot stops the heal reaching the moves after it, since
/// the loop reads that slot again rather than stepping past it, but `RestoreBonusPP` still adds
/// their bonus to whatever PP they had.
pub fn heal_party(party: &mut [Named<PartyMon>]) {
    for named in party {
        let mon = &mut named.mon;
        mon.mon.status = 0;
        let mut healed = true;
        for slot in 0..mon.mon.moves.len() {
            let pp = mon.mon.pp[slot];
            match mon.mon.moves[slot] {
                Some(mv) if healed => mon.mon.pp[slot] = pp & PP_UP_MASK | max_pp(mv, pp),
                Some(mv) => mon.mon.pp[slot] = add_bonus_pp(pp, MoveData::of_move(mv).pp, false),
                None => healed = false,
            }
        }
        mon.mon.hp = mon.stats[0];
    }
}

/// `SetLastBlackoutMap`: the map the nurse's building was entered from, unless the building is one
/// of the Safari Zone's rest houses, which leave `wLastBlackoutMap` alone.
pub fn set_last_blackout_map(map: Map, last_map: Map, last_blackout_map: Map) -> Map {
    let rest_houses = rom_slice(pokered_symbols::SafariZoneRestHouses);
    if rest_houses.iter().take_while(|&&m| m != 0xFF).any(|&m| m == map as u8) {
        return last_blackout_map;
    }
    last_map
}

#[cfg(test)]
mod tests {
    use crate::fixtures::cases;
    use poke_core::move_name::PokemonMoveName;
    use poke_core::species::PokemonSpecies;
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use super::*;

    #[test]
    fn a_heal_restores_hp_status_and_pp_with_its_pp_ups() {
        let mut mon = new_party_mon(PokemonSpecies::Squirtle, 20, 1, &Origin::Given, &mut GameRng::seeded(3));
        mon.mon.hp = 1;
        mon.mon.status = 8;
        mon.mon.moves = [Some(PokemonMoveName::Tackle), Some(PokemonMoveName::TailWhip), None, None];
        mon.mon.pp = [0xC0, 3, 0, 0];
        let mut party = vec![Named { mon, ot: vec![], nick: vec![] }];
        heal_party(&mut party);
        let mon = &party[0].mon;
        assert_eq!((mon.mon.hp, mon.mon.status), (mon.stats[0], 0));
        assert_eq!(mon.mon.pp, [0xC0 | 56, 30, 0, 0], "Tackle's 35 with three PP Ups is 56");
    }

    #[test]
    fn every_harvested_case_of_heal_party() {
        let jsonl = include_str!("../../../fixtures/events/heal_party.jsonl");
        for (input, expected, _) in cases::<Vec<PartyMon>, Vec<(u16, u8, [u8; 4])>>(jsonl) {
            let mut party: Vec<_> = input.iter().cloned().map(|mon| Named { mon, ot: vec![], nick: vec![] }).collect();
            heal_party(&mut party);
            let healed: Vec<_> = party.iter().map(|named| (named.mon.mon.hp, named.mon.mon.status, named.mon.mon.pp)).collect();
            assert_eq!(healed, expected, "{input:?}");
        }
    }

    #[test]
    fn a_rest_house_does_not_move_the_blackout_map() {
        assert_eq!(set_last_blackout_map(Map::SafariZoneWestRestHouse, Map::SafariZoneWest, Map::FuchsiaCity), Map::FuchsiaCity);
        assert_eq!(set_last_blackout_map(Map::CeruleanPokecenter, Map::CeruleanCity, Map::PewterCity), Map::CeruleanCity);
    }
}

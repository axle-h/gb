//! The in-game trades.

use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::policy::PolicyStep;
use crate::pokemon::postgame::gifts::PartyScript;
use crate::pokemon::species::PokemonSpecies;

/// One in-game trade: hand over `give`, receive `get`, from `npc` on `at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InGameTrade {
    pub give: PokemonSpecies,
    pub get: PokemonSpecies,
    pub at: Map,
    pub npc: MapSprite,
}

impl InGameTrade {
    pub const fn script(self) -> PartyScript {
        PartyScript::Trade { at: self.at, npc: self.npc, give: self.give }
    }
}

/// The nine usable in-game trades, in `TradeMons` order minus the unused one.
pub const TRADES: &[InGameTrade] = &[
    InGameTrade { give: PokemonSpecies::Nidorino, get: PokemonSpecies::Nidorina,
        at: Map::Route11Gate2F, npc: MapSprite::ROUTE11GATE2F_YOUNGSTER },
    InGameTrade { give: PokemonSpecies::Abra, get: PokemonSpecies::MrMime,
        at: Map::Route2TradeHouse, npc: MapSprite::ROUTE2TRADEHOUSE_GAMEBOY_KID },
    InGameTrade { give: PokemonSpecies::Ponyta, get: PokemonSpecies::Seel,
        at: Map::CinnabarLabFossilRoom, npc: MapSprite::CINNABARLABFOSSILROOM_SCIENTIST2 },
    InGameTrade { give: PokemonSpecies::Spearow, get: PokemonSpecies::Farfetchd,
        at: Map::VermilionTradeHouse, npc: MapSprite::VERMILIONTRADEHOUSE_LITTLE_GIRL },
    InGameTrade { give: PokemonSpecies::Slowbro, get: PokemonSpecies::Lickitung,
        at: Map::Route18Gate2F, npc: MapSprite::ROUTE18GATE2F_YOUNGSTER },
    InGameTrade { give: PokemonSpecies::Poliwhirl, get: PokemonSpecies::Jynx,
        at: Map::CeruleanTradeHouse, npc: MapSprite::CERULEANTRADEHOUSE_GAMBLER },
    InGameTrade { give: PokemonSpecies::Raichu, get: PokemonSpecies::Electrode,
        at: Map::CinnabarLabTradeRoom, npc: MapSprite::CINNABARLABTRADEROOM_GRAMPS },
    InGameTrade { give: PokemonSpecies::Venonat, get: PokemonSpecies::Tangela,
        at: Map::CinnabarLabTradeRoom, npc: MapSprite::CINNABARLABTRADEROOM_BEAUTY },
    InGameTrade { give: PokemonSpecies::NidoranMale, get: PokemonSpecies::NidoranFemale,
        at: Map::UndergroundPathRoute5, npc: MapSprite::UNDERGROUNDPATHROUTE5_LITTLE_GIRL },
];

/// Look a trade up by the map and the sprite's name.
pub fn trade_at(map: Map, npc: &str) -> Option<InGameTrade> {
    TRADES.iter().copied().find(|trade| trade.at == map && trade.npc.name == npc)
}

/// Look a trade up by what it wants, which is unambiguous.
pub fn trade_for(give: PokemonSpecies) -> InGameTrade {
    *TRADES.iter().find(|t| t.give == give)
        .unwrap_or_else(|| panic!("no in-game trade wants a {give:?}"))
}

impl PolicyStep {
    /// Catch `give` in the grass on `catch_on`, then hand it over for its trade partner.
    pub fn trade_steps(give: PokemonSpecies, catch_on: Map, bank: &[u8], bank_at: Map) -> Vec<Self> {
        let trade = trade_for(give);
        let town = town_of(bank_at);
        let mut s = vec![Self::Fly { to: town }, Self::enter(bank_at)];
        // Deposit from the highest slot down: banking slot 3 first would renumber slot 4.
        let mut slots = bank.to_vec();
        slots.sort_unstable_by(|a, b| b.cmp(a));
        s.extend(slots.into_iter().map(|slot| Self::deposit_pokemon(slot, bank_at)));
        s.push(Self::withdraw_item(ItemId::GreatBall, u8::MAX, bank_at));
        s.extend([
            Self::enter(town),
        ]);
        s.extend(Self::to_hunting_ground(catch_on));
        s.extend([
            Self::CatchPokemon { species: give, on_map: catch_on, ball: None },
        ]);
        s.extend(Self::to_trade_npc(trade));
        // The driver finds the give-species itself, so the Cut holder stays in front.
        s.push(Self::PartyScript { script: trade.script(), slot: 0 });
        s.extend(out_of(trade.at).into_iter().map(Self::enter));
        s
    }

    pub fn walk_up_and_trade_steps(trade: InGameTrade) -> Vec<Self> {
        let mut s = Self::to_trade_npc(trade);
        s.extend(std::iter::repeat_n(Self::Interact(trade.npc), 3));
        s
    }

    /// The same trade, with the give-species already in a PC box.
    pub fn trade_boxed_steps(give: PokemonSpecies, box_slot: u8, bank: u8, bank_at: Map) -> Vec<Self> {
        let trade = trade_for(give);
        let town = town_of(bank_at);
        let mut s = vec![
            Self::Fly { to: town },
            Self::enter(bank_at),
            Self::deposit_pokemon(bank, bank_at),
            Self::withdraw_pokemon(box_slot, bank_at),
            Self::enter(town),
        ];
        s.extend(Self::to_trade_npc(trade));
        s.push(Self::PartyScript { script: trade.script(), slot: 0 });
        s.extend(out_of(trade.at).into_iter().map(Self::enter));
        s
    }

    fn to_hunting_ground(catch_on: Map) -> Vec<Self> {
        match catch_on {
            Map::Route15 => vec![
                Self::goto(Map::Route15),
                Self::enter(Map::Route15Gate1F),
                Self::enter_at(Map::Route15, 14, 8), // out the east door, onto the grass side
            ],
            other => vec![Self::goto(other)],
        }
    }

    /// The walk to a trade NPC's room, a lookup because each is a one-off.
    pub(crate) fn to_trade_npc(trade: InGameTrade) -> Vec<Self> {
        match trade.at {
            // The trade house is on Route 2's north half, and even from Pewter behind a tree.
            Map::Route2TradeHouse => vec![
                Self::Fly { to: Map::PewterCity },
                Self::enter(Map::Route2),
                Self::CutTree { map: Map::Route2 },
                Self::enter(Map::Route2TradeHouse),
            ],
            Map::VermilionTradeHouse => vec![
                Self::Fly { to: Map::VermilionCity }, Self::enter(Map::VermilionTradeHouse),
            ],
            Map::UndergroundPathRoute5 => vec![
                Self::Fly { to: Map::CeruleanCity },
                Self::enter(Map::CeruleanTrashedHouse),
                Self::enter_at(Map::CeruleanCity, 27, 9),
                Self::enter(Map::Route5),
                Self::enter(Map::UndergroundPathRoute5),
            ],
            Map::CinnabarLabTradeRoom => vec![
                Self::Fly { to: Map::CinnabarIsland },
                Self::enter(Map::CinnabarLab),
                Self::enter(Map::CinnabarLabTradeRoom),
            ],
            Map::CinnabarLabFossilRoom => vec![
                Self::Fly { to: Map::CinnabarIsland },
                Self::enter(Map::CinnabarLab),
                Self::enter(Map::CinnabarLabFossilRoom),
            ],
            Map::CeruleanTradeHouse => vec![
                Self::Fly { to: Map::CeruleanCity }, Self::enter(Map::CeruleanTradeHouse),
            ],
            Map::Route11Gate2F => vec![
                Self::Fly { to: Map::VermilionCity },
                Self::enter(Map::Route11),
                Self::enter(Map::Route11Gate1F),
                Self::enter(Map::Route11Gate2F),
            ],
            Map::Route18Gate2F => vec![
                Self::Fly { to: Map::FuchsiaCity },
                Self::enter(Map::Route18),
                Self::enter(Map::Route18Gate1F),
                Self::enter(Map::Route18Gate2F),
            ],
            other => panic!("no route recorded to the trade NPC on {other:?}"),
        }
    }
}

/// The warps back to open air from a trade NPC's room.
fn out_of(room: Map) -> Vec<Map> {
    match room {
        Map::Route2TradeHouse => vec![Map::Route2],
        Map::VermilionTradeHouse => vec![Map::VermilionCity],
        Map::UndergroundPathRoute5 => vec![Map::Route5],
        Map::CinnabarLabTradeRoom | Map::CinnabarLabFossilRoom => vec![Map::CinnabarLab, Map::CinnabarIsland],
        Map::CeruleanTradeHouse => vec![Map::CeruleanCity],
        Map::Route11Gate2F => vec![Map::Route11Gate1F, Map::Route11],
        Map::Route18Gate2F => vec![Map::Route18Gate1F, Map::Route18],
        other => panic!("no exit recorded from {other:?}"),
    }
}

/// The outdoor map a Pokémon Center sits in, so a leg can `Fly` to it.
fn town_of(centre: Map) -> Map {
    match centre {
        Map::ViridianPokecenter => Map::ViridianCity,
        Map::CeruleanPokecenter => Map::CeruleanCity,
        Map::VermilionPokecenter => Map::VermilionCity,
        Map::FuchsiaPokecenter => Map::FuchsiaCity,
        other => panic!("no town recorded for {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pokemon::symbols::{pokered_symbols, DmgBank};
    use crate::pokemon::roms;

    /// [`TRADES`]' give/get pairs match `TradeMons` bit for bit.
    #[test]
    fn trade_table_matches_the_rom() {
        /// `npctrade` is `db give, get, dialogset` then an 11-byte name.
        const STRIDE: usize = 3 + 11;
        /// The entry no script references: Butterfree → Beedrill, index 2.
        const UNUSED: usize = 2;

        let rom = roms::POKERED;
        let base = pokered_symbols::TradeMons;
        let byte = |offset: usize| -> u8 {
            let DmgBank::ROM { bank } = base.bank else { panic!("TradeMons is not in ROM") };
            rom[bank as usize * 0x4000 + (base.address as usize - 0x4000) + offset]
        };

        let mut expected = TRADES.iter();
        for i in 0..(TRADES.len() + 1) {
            if i == UNUSED {
                assert_eq!(byte(i * STRIDE), PokemonSpecies::Butterfree as u8,
                    "entry {i} should be the unused Butterfree trade");
                continue;
            }
            let trade = expected.next().expect("more ROM entries than table rows");
            assert_eq!(byte(i * STRIDE), trade.give as u8, "give species of trade {i}");
            assert_eq!(byte(i * STRIDE + 1), trade.get as u8, "get species of trade {i}");
        }
        assert!(expected.next().is_none(), "more table rows than ROM entries");
    }

    /// Every give-species has a wild encounter slot or a recorded evolution.
    #[test]
    fn every_trade_give_species_is_obtainable() {
        use crate::pokemon::wild::{self, Terrain};
        use strum::IntoEnumIterator;
        use crate::pokemon::map::Map;

        /// The give-species with no wild encounter slot in Red, and how to evolve each.
        const BY_EVOLUTION: &[(PokemonSpecies, &str)] = &[
            (PokemonSpecies::Poliwhirl, "Poliwag (Super Rod) at lv25"),
        ];

        let mut wrong = Vec::new();
        for trade in TRADES {
            let wild_home = Map::iter().find_map(|map| {
                let wild = wild::encounters(map)?;
                [Terrain::Grass, Terrain::Water].iter()
                    .flat_map(|t| wild.species(*t))
                    .any(|(s, _, _)| s == trade.give)
                    .then_some(map)
            });
            let evolution = BY_EVOLUTION.iter().find(|(s, _)| *s == trade.give).map(|(_, how)| *how);
            println!("   {:<12} → {:<12} on {:<24} — {}", format!("{:?}", trade.give),
                format!("{:?}", trade.get), format!("{}", trade.at),
                match (wild_home, evolution) {
                    (Some(map), _) => format!("wild on {map}"),
                    (None, Some(how)) => format!("evolve: {how}"),
                    (None, None) => "❌ NO KNOWN SOURCE".to_string(),
                });
            match (wild_home, evolution) {
                (Some(map), Some(_)) => wrong.push(format!(
                    "{:?} is listed as evolution-only but the ROM has it wild on {map}", trade.give)),
                (None, None) => wrong.push(format!(
                    "{:?} (traded for {:?} on {}) has no wild encounter anywhere and no recorded way \
                     to obtain it, so getting the give-species is no longer the only thing between \
                     a finished save and every in-game trade", trade.give, trade.get, trade.at)),
                _ => {}
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// Every give-species is distinct, so [`trade_for`] is unambiguous.
    #[test]
    fn every_trade_wants_a_different_species() {
        let mut seen: Vec<PokemonSpecies> = TRADES.iter().map(|t| t.give).collect();
        seen.sort_unstable_by_key(|s| *s as u8);
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "two trades want the same species");
    }
}

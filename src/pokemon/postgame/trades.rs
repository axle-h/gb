//! Workstream **G-trades — the in-game trades**. See `docs/postgame-coverage-plan.md` §6-G
//! (sub-steps G5–G6).
//!
//! Sibling of [`super::gifts`], claimed as a separate §9 row.
//!
//! # Why this module is mostly a table
//!
//! The *mechanic* lives in [`super::gifts`]: a trade NPC opens the same stale-cursor party menu the
//! Day Care and the Name Rater do, so a trade is a third [`PartyScript`] variant and not a fourth
//! driver. The reserved `PolicyStep::TradePokemon { give_slot, at }` seam (task 0.8) is therefore
//! **gone** — see §11; build trades with [`PolicyStep::trade_steps`].
//!
//! What this module adds is the **table** — which mon to hand over, to whom, and where — pinned
//! against the ROM, plus the step lists that go and catch the give-species first.
//!
//! That last part is the real cost, and §6-G says so: *"Each trade requires already owning the
//! give-species, so G5/G6 depend on catching those nine first — this is not the cheap dex win it
//! looks like."* Each completed trade is worth **two** dex entries though (the mon caught and the mon
//! received), and five species — Farfetch'd, Mr. Mime, Lickitung, Jynx and Tangela — are obtainable
//! **only** this way on a single cartridge.

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
    /// The [`PartyScript`] that runs this trade.
    pub const fn script(self) -> PartyScript {
        PartyScript::Trade { at: self.at, npc: self.npc, give: self.give }
    }
}

/// The **nine usable** in-game trades, in `TradeMons` order minus the unused one.
///
/// ⚠️ `data/events/trades.asm` lists **ten**; the third, Butterfree → Beedrill, is dead code — no
/// script sets `TRADE_FOR_CHIKUCHIKU`. The give/get pairs here are pinned against the ROM by
/// [`tests::trade_table_matches_the_rom`], which is worth having because the ROM addresses trades by
/// *index* and this table addresses them by *map*, and nothing else would catch the two drifting.
///
/// The `at`/`npc` columns are **not** in the ROM table — it has no idea where its trades live. They
/// come from the nine scripts that set `wWhichTrade`, and **which NPC in the room is not guessable**:
/// `Route2TradeHouse`'s trader is the Gameboy Kid, not the scientist standing in front of him;
/// `CeruleanTradeHouse`'s is the gambler, not the granny; and `CinnabarLabTradeRoom` holds *two*
/// trades whose NPCs are the **Gramps** and the **Beauty**, not the Super Nerd the room is named for.
/// Read the script, not the object list — and note that a wrong NPC does not error, it just talks.
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

/// Look a trade up by what it wants. The nine give-species are distinct, so this is unambiguous.
pub fn trade_for(give: PokemonSpecies) -> InGameTrade {
    *TRADES.iter().find(|t| t.give == give)
        .unwrap_or_else(|| panic!("no in-game trade wants a {give:?}"))
}

impl PolicyStep {
    /// **G5/G6** — catch `give` in the grass on `catch_on`, then hand it over for its trade partner.
    ///
    /// One constructor covers every trade, because the shape never changes: free party slots, catch,
    /// travel, trade. The only per-trade facts are the map to hunt on and where the NPC is, and the
    /// second comes out of [`TRADES`].
    ///
    /// ⚠️ **A party slot has to be free before the catch.** With six in the party a caught mon goes to
    /// the box, and D's §11 entry records the agent *wedging* on the nickname screen on that path.
    /// `bank` is the slots to deposit at `bank_at` first — passed in rather than guessed, because
    /// which party member is expendable depends on the fixture.
    pub fn trade_steps(give: PokemonSpecies, catch_on: Map, bank: &[u8], bank_at: Map) -> Vec<Self> {
        let trade = trade_for(give);
        let town = town_of(bank_at);
        let mut s = vec![Self::Fly { to: town }, Self::enter(bank_at)];
        // Deposit from the highest slot down: banking slot 3 first would renumber slot 4.
        let mut slots = bank.to_vec();
        slots.sort_unstable_by(|a, b| b.cmp(a));
        s.extend(slots.into_iter().map(|slot| Self::deposit_pokemon(slot, bank_at)));
        // ⚠️ **The bag has no Poké Balls.** G7 banked the whole Great Ball stack to free bag slots,
        // and `CatchPokemon` gives up immediately without one ("want to catch a X, but no Pokéballs
        // left!") — silently, from the policy's side. Withdrawing at the same PC costs nothing and
        // needs no money. `u8::MAX` means "however many are in there": `ItemPcState::new` clamps the
        // quantity to what the source actually holds, so this does not have to track the count.
        s.push(Self::withdraw_item(ItemId::GreatBall, u8::MAX, bank_at));
        s.extend([
            Self::enter(town),
        ]);
        s.extend(Self::to_hunting_ground(catch_on));
        s.extend([
            Self::CatchPokemon { species: give, on_map: catch_on, ball: None },
        ]);
        s.extend(Self::to_trade_npc(trade));
        // `slot` is ignored for a trade — the driver finds the give-species itself, so the party
        // never has to be reshuffled and the Cut holder stays in front.
        s.push(Self::PartyScript { script: trade.script(), slot: 0 });
        // ⚠️ Step back outside. Every trade NPC is indoors, and `FlyState::blocked_by` refuses a
        // flight from an interior — so a fixture saved in the room fails the *next* leg, with the
        // whole queue then discarded for want of a route. C and D each recorded this rule; it caught
        // this workstream too.
        s.extend(out_of(trade.at).into_iter().map(Self::enter));
        s
    }

    /// The walk to the grass. Usually just `goto`, but a route with a **gate building** in the middle
    /// is two maps, not one, and `goto` cannot see that: it pops the moment the map matches, and
    /// `CatchPokemon` then paces on a grassless strip until the budget runs out, silently, with no
    /// battle ever starting. Route 15 is the case in point — from Fuchsia its reachable set is the two
    /// gate doors and nothing else, and every blade of grass is on the far side.
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

    /// The walk to a trade NPC's room. Each is a one-off; there is no general rule, so this is a
    /// lookup rather than a route.
    fn to_trade_npc(trade: InGameTrade) -> Vec<Self> {
        match trade.at {
            // ⚠️ Route 2 is **two halves** split by `Route2Gate` at y=35/39, and the trade house is
            // the *north* one, at (15,19). Flying to Viridian lands the agent at the south end (y=72)
            // where `enter(Route2TradeHouse)` finds no route and simply stands still — the same
            // silent failure `postgame::gifts` records for Route 5. Approach from **Pewter**.
            Map::Route2TradeHouse => vec![
                Self::Fly { to: Map::PewterCity },
                Self::enter(Map::Route2),
                // …and even from Pewter the door is walled off: from the northern landing the
                // reachable set is the forest gate, Pewter, and **one cut tree**. Route 2's ledges
                // all sit under walls, so that tree at (5,10) is the only link between the north
                // strip and the eastern column the house is on. Venusaur leads (nothing here
                // reshuffles the party), which is what `CuttingTree` needs.
                Self::CutTree { map: Map::Route2 },
                Self::enter(Map::Route2TradeHouse),
            ],
            Map::VermilionTradeHouse => vec![
                Self::Fly { to: Map::VermilionCity }, Self::enter(Map::VermilionTradeHouse),
            ],
            // The Underground Path's north end is on Route 5, in the corridor the Day Care is *not*
            // in — G8b's §11 entry has the map. Reached from Cerulean over the trashed-house bridge.
            Map::UndergroundPathRoute5 => vec![
                Self::Fly { to: Map::CeruleanCity },
                Self::enter(Map::CeruleanTrashedHouse),
                Self::enter_at(Map::CeruleanCity, 27, 9),
                Self::enter(Map::Route5),
                Self::enter(Map::UndergroundPathRoute5),
            ],
            // Both Cinnabar Lab trades are in the same room, two doors deep off the island.
            Map::CinnabarLabTradeRoom => vec![
                Self::Fly { to: Map::CinnabarIsland },
                Self::enter(Map::CinnabarLab),
                Self::enter(Map::CinnabarLabTradeRoom),
            ],
            other => panic!("no route recorded to the trade NPC on {other:?}"),
        }
    }
}

/// The warps back to open air from a trade NPC's room. Most are one; the Cinnabar Lab's trade room is
/// two deep, and stopping in the lab would leave the fixture indoors — which is the whole point.
fn out_of(room: Map) -> Vec<Map> {
    match room {
        Map::Route2TradeHouse => vec![Map::Route2],
        Map::VermilionTradeHouse => vec![Map::VermilionCity],
        Map::UndergroundPathRoute5 => vec![Map::Route5],
        Map::CinnabarLabTradeRoom => vec![Map::CinnabarLab, Map::CinnabarIsland],
        other => panic!("no exit recorded from {other:?}"),
    }
}

/// The outdoor map a Pokémon Center sits in, so a leg can `Fly` to it. Only the centres these legs
/// use — a Pokémon Center's town is not derivable from `Map` today.
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
    use crate::pokemon::symbols::{pokered_symbols, DmgPointer, DmgBank};
    use crate::pokemon::roms;

    /// Pin [`TRADES`]' give/get pairs bit-for-bit against `TradeMons`.
    ///
    /// The ROM addresses a trade by its `TRADE_FOR_*` index and this table addresses it by map, so
    /// the two can drift silently — a wrong pair would present as `InGameTrade_DoTrade`'s "wrong
    /// mon" text, i.e. as a driver bug, several minutes into a leg.
    #[test]
    fn trade_table_matches_the_rom() {
        /// `npctrade` is `db give, get, dialogset` then an 11-byte name (`data/events/trades.asm`).
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

    /// Every give-species is distinct, which is what makes [`trade_for`] unambiguous.
    #[test]
    fn every_trade_wants_a_different_species() {
        let mut seen: Vec<PokemonSpecies> = TRADES.iter().map(|t| t.give).collect();
        seen.sort_unstable_by_key(|s| *s as u8);
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "two trades want the same species");
    }
}

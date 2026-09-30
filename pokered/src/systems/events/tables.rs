//! The events' smaller tables: the in-game trades, the Pokédex ratings, the prizes, the guards'
//! drinks and the Pewter guides' walks.

use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::tables::{DEX_RATINGS, GUARD_DRINKS_LIST, IN_GAME_TRADE_TEXTS, OWNED_MON_VALUES, PEWTER_GUYS_COORDS, PRIZE_MON_LEVEL_DICTIONARY, PRIZE_WINDOWS};
use serde::{Deserialize, Serialize};
use crate::audio::data::{AudioBank, Sound, SoundId};
use crate::systems::inventory::Inventory;

/// A row of `TradeMons`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InGameTrade {
    pub give: PokemonSpecies,
    pub receive: PokemonSpecies,
    /// `TRADE_DIALOGSET_*`: which of `TradeTextPointers1`-`3` it talks with.
    pub dialog_set: u8,
    /// The received mon's nickname, charmap bytes, unterminated.
    pub nick: Vec<u8>,
}

/// `TRADETEXT_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum TradeText {
    WannaTrade,
    NoTrade,
    WrongMon,
    Thanks,
    AfterTrade,
}

/// `NUM_NPC_TRADES`.
pub const NUM_NPC_TRADES: u8 = 10;

impl InGameTrade {
    /// `TRADE_FOR_*`.
    pub fn of(which: u8) -> Self {
        let row = poke_core::tables::TRADE_MONS[which as usize];
        let species = |id| PokemonSpecies::from_repr(id).expect("a trade names a species");
        let nick = poke_core::charmap::encode(row.nickname).expect("a nickname is in the charmap");
        Self { give: species(row.give), receive: species(row.get), dialog_set: row.dialog, nick }
    }

    /// The text `InGameTradeTextPointers` names for this trade's dialogue set.
    pub fn text(&self, text: TradeText) -> &'static str {
        IN_GAME_TRADE_TEXTS[self.dialog_set as usize][text as usize]
    }
}

/// `InGameTrade_CheckForTradeEvo`: whether the received mon evolves on arrival, by the first letters
/// of its species' name. It looks for a Graveler and a Spectre, and no English trade has either.
pub fn trade_evolves(receive: PokemonSpecies) -> bool {
    let name = receive.name();
    let letter = |i: usize| name.get(i).copied();
    letter(0) == Some(poke_core::charmap::encode("G").unwrap()[0])
        || letter(0) == Some(poke_core::charmap::encode("S").unwrap()[0]) && letter(1) == Some(poke_core::charmap::encode("P").unwrap()[0])
}

/// `DexRatingsTable`: the rating for this many owned.
pub fn dex_rating_text(owned: u8) -> &'static str {
    DEX_RATINGS.iter().find(|&&(below, _)| owned < below).expect("the last row is past every count").1
}

/// `PlayPokedexRatingSfx`: the sound for this many owned, from `OwnedMonValues`.
pub fn dex_rating_sound(owned: u8) -> Sound {
    let index = OWNED_MON_VALUES.iter().position(|&value| owned < value).unwrap_or(OWNED_MON_VALUES.len());
    let (id, bank) = poke_core::audio::POKEDEX_RATING_SFX[index];
    Sound { bank: AudioBank::from_rom_bank(bank).expect("an audio bank"), id: SoundId(id) }
}

/// `OaksAideScript`'s `hOaksAideResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OaksAideResult {
    BagFull,
    GotItem,
    NotEnoughMons,
    Refused,
}

/// `PrizeDifferentMenuPtrs`: a window's three prizes, and their prices in coins, BCD as the coin
/// case holds them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrizeWindow {
    pub prizes: [u8; 3],
    pub prices: [[u8; 2]; 3],
}

impl PrizeWindow {
    /// `wWhichPrizeWindow`: 0 and 1 are Pokémon, 2 is TMs.
    pub fn of(window: u8) -> Self {
        let (prizes, costs) = PRIZE_WINDOWS[window as usize];
        let bcd = |n: u16| [((n / 1000) << 4 | n / 100 % 10) as u8, ((n / 10 % 10) << 4 | n % 10) as u8];
        Self { prizes, prices: costs.map(bcd) }
    }
}

/// `GetPrizeMonLevel`. The dictionary has no terminator: a species not in it is not a prize.
pub fn prize_mon_level(species: PokemonSpecies) -> u8 {
    PRIZE_MON_LEVEL_DICTIONARY.iter().find(|&&(prize, _)| prize == species as u8)
        .expect("a prize mon has a level").1
}

/// `RemoveGuardDrink`: the first of `GuardDrinksList` in the bag, one of it taken away.
pub fn remove_guard_drink(bag: &mut Inventory) -> Option<ItemId> {
    let drink = GUARD_DRINKS_LIST.iter()
        .filter_map(|&d| ItemId::from_repr(d))
        .find(|&d| bag.quantity_of(d) != 0)?;
    let slot = bag.items.iter().position(|item| item.id == drink).expect("the drink is in the bag");
    bag.remove(slot, 1);
    Some(drink)
}

/// `PewterGuys`: the presses that bring the player from where they stand into line behind the museum
/// guide (`which` 0) or the gym guide (1), put on the end of the simulated presses. The first of them
/// goes over the last press already there.
pub fn pewter_guys(which: u8, x: u8, y: u8, presses: &mut Vec<u8>) {
    let Some((_, moves)) = PEWTER_GUYS_COORDS[which as usize].iter().find(|&&(at, _)| at == (x, y)) else { return };
    presses.pop();
    presses.extend_from_slice(moves);
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use super::*;

    #[test]
    fn the_trades_are_the_cartridge_s() {
        let dux = InGameTrade::of(4);
        assert_eq!((dux.give, dux.receive, dux.dialog_set), (PokemonSpecies::Spearow, PokemonSpecies::Farfetchd, 2));
        assert_eq!(dux.nick, encode("DUX").unwrap());
        assert_eq!(dux.text(TradeText::Thanks), "Thanks3Text");
        let spot = InGameTrade::of(9);
        assert_eq!((spot.give, spot.receive), (PokemonSpecies::NidoranMale, PokemonSpecies::NidoranFemale));
        assert_eq!(spot.text(TradeText::WannaTrade), "WannaTrade3Text");
        assert!((0..NUM_NPC_TRADES).all(|which| !trade_evolves(InGameTrade::of(which).receive)));
    }

    #[test]
    fn a_rating_is_the_first_row_past_the_count() {
        assert_eq!(dex_rating_text(9), "DexRatingText_Own0To9");
        assert_eq!(dex_rating_text(10), "DexRatingText_Own10To19");
        assert_eq!(dex_rating_text(151), "DexRatingText_Own150To151");
        assert_eq!(dex_rating_sound(9).id, crate::audio::data::sounds::SFX_DENIED);
        assert_eq!(dex_rating_sound(150).id, crate::audio::data::sounds::SFX_GET_ITEM_2);
    }

    #[test]
    fn red_s_prizes() {
        let mons = PrizeWindow::of(0);
        assert_eq!(mons.prizes, [PokemonSpecies::Abra as u8, PokemonSpecies::Clefairy as u8, PokemonSpecies::Nidorina as u8]);
        assert_eq!(mons.prices, [[0x01, 0x80], [0x05, 0x00], [0x12, 0x00]]);
        assert_eq!(PrizeWindow::of(1).prices[2], [0x99, 0x99]);
        assert_eq!(prize_mon_level(PokemonSpecies::Porygon), 26);
    }

    #[test]
    fn a_guard_takes_the_first_drink_on_his_list() {
        let mut bag = Inventory::bag(vec![poke_core::bag::BagItem::new(ItemId::Lemonade, 2), poke_core::bag::BagItem::new(ItemId::SodaPop, 1)]);
        assert_eq!(remove_guard_drink(&mut bag), Some(ItemId::SodaPop));
        assert_eq!(remove_guard_drink(&mut bag), Some(ItemId::Lemonade));
        assert_eq!(bag.quantity_of(ItemId::Lemonade), 1);
    }

    #[test]
    fn the_gym_guide_walks_the_player_round_from_the_side() {
        let mut presses = vec![0x40, 0x40];
        pewter_guys(1, 34, 16, &mut presses);
        assert_eq!(presses, [0x40, 0x20, 0x80, 0x80, 0x10], "left, down, down, right over the last");
    }
}

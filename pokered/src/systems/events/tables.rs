//! The events' smaller tables: the in-game trades, the Pokédex ratings, the prizes, the guards'
//! drinks and the Pewter guides' walks.

use poke_core::item::ItemId;
use poke_core::rom_gfx::rom_slice;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::{pokered_symbols, DmgPointer};
use serde::{Deserialize, Serialize};
use crate::audio::data::{AudioBank, Sound, SoundId};
use crate::systems::inventory::Inventory;

const TERMINATOR: u8 = 0x50;
const NAME_LENGTH: usize = 11;

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
        let row = rom_slice(pokered_symbols::TradeMons + which as u16 * (3 + NAME_LENGTH as u16));
        let species = |id| PokemonSpecies::from_repr(id).expect("a trade names a species");
        let nick = row[3..3 + NAME_LENGTH].iter().copied().take_while(|&b| b != TERMINATOR).collect();
        Self { give: species(row[0]), receive: species(row[1]), dialog_set: row[2], nick }
    }

    /// The text `InGameTradeTextPointers` names for this trade's dialogue set.
    pub fn text(&self, text: TradeText) -> DmgPointer {
        let bank = pokered_symbols::InGameTradeTextPointers.bank;
        let word = |at: DmgPointer| {
            let bytes = rom_slice(at);
            DmgPointer { bank, address: u16::from_le_bytes([bytes[0], bytes[1]]) }
        };
        let table = word(pokered_symbols::InGameTradeTextPointers + 2 * self.dialog_set as u16);
        word(table + 2 * text as u16)
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
pub fn dex_rating_text(owned: u8) -> DmgPointer {
    let table = pokered_symbols::DexRatingsTable;
    let row = rom_slice(table).chunks(3).find(|row| owned < row[0]).expect("the last row is past every count");
    DmgPointer { bank: table.bank, address: u16::from_le_bytes([row[1], row[2]]) }
}

/// `PlayPokedexRatingSfx`: the sound for this many owned, from `OwnedMonValues`.
pub fn dex_rating_sound(owned: u8) -> Sound {
    let values = rom_slice(pokered_symbols::OwnedMonValues);
    let index = values.iter().position(|&value| owned < value).expect("$ff ends the table");
    let row = rom_slice(pokered_symbols::PokedexRatingSfxPointers + 2 * index as u16);
    Sound { bank: AudioBank::from_rom_bank(row[1]).expect("an audio bank"), id: SoundId(row[0]) }
}

/// `OaksAideScript`'s `hOaksAideResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OaksAideResult {
    BagFull,
    GotItem,
    NotEnoughMons,
    Refused,
}

/// `PrizeDifferentMenuPtrs`: a window's three prizes, and their prices in coins, BCD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrizeWindow {
    pub prizes: [u8; 3],
    pub prices: [[u8; 2]; 3],
}

impl PrizeWindow {
    /// `wWhichPrizeWindow`: 0 and 1 are Pokémon, 2 is TMs.
    pub fn of(window: u8) -> Self {
        let table = pokered_symbols::PrizeDifferentMenuPtrs;
        let row = rom_slice(table + 4 * window as u16);
        let at = |i: usize| rom_slice(DmgPointer { bank: table.bank, address: u16::from_le_bytes([row[i], row[i + 1]]) });
        let (prizes, prices) = (at(0), at(2));
        Self {
            prizes: [prizes[0], prizes[1], prizes[2]],
            prices: [[prices[0], prices[1]], [prices[2], prices[3]], [prices[4], prices[5]]],
        }
    }
}

/// `GetPrizeMonLevel`. The dictionary has no terminator: a species not in it is not a prize.
pub fn prize_mon_level(species: PokemonSpecies) -> u8 {
    rom_slice(pokered_symbols::PrizeMonLevelDictionary).chunks(2).find(|row| row[0] == species as u8)
        .expect("a prize mon has a level")[1]
}

/// `RemoveGuardDrink`: the first of `GuardDrinksList` in the bag, one of it taken away.
pub fn remove_guard_drink(bag: &mut Inventory) -> Option<ItemId> {
    let drink = rom_slice(pokered_symbols::GuardDrinksList).iter().take_while(|&&d| d != 0)
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
    const ROWS: [usize; 2] = [4, 5];
    let table = pokered_symbols::PewterGuysCoordsTable;
    let pointer = rom_slice(table + 2 * which as u16);
    let rows = rom_slice(DmgPointer { bank: table.bank, address: u16::from_le_bytes([pointer[0], pointer[1]]) });
    let Some(row) = rows.chunks(4).take(ROWS[which as usize]).find(|row| row[0] == y && row[1] == x) else { return };
    let moves = rom_slice(DmgPointer { bank: table.bank, address: u16::from_le_bytes([row[2], row[3]]) });
    presses.pop();
    presses.extend(moves.iter().take_while(|&&m| m != 0xFF));
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
        assert_eq!(dux.text(TradeText::Thanks), pokered_symbols::Thanks3Text);
        let spot = InGameTrade::of(9);
        assert_eq!((spot.give, spot.receive), (PokemonSpecies::NidoranMale, PokemonSpecies::NidoranFemale));
        assert_eq!(spot.text(TradeText::WannaTrade), pokered_symbols::WannaTrade3Text);
        assert!((0..NUM_NPC_TRADES).all(|which| !trade_evolves(InGameTrade::of(which).receive)));
    }

    #[test]
    fn a_rating_is_the_first_row_past_the_count() {
        assert_eq!(dex_rating_text(9), pokered_symbols::DexRatingText_Own0To9);
        assert_eq!(dex_rating_text(10), pokered_symbols::DexRatingText_Own10To19);
        assert_eq!(dex_rating_text(151), pokered_symbols::DexRatingText_Own150To151);
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

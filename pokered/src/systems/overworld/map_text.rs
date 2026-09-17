use poke_core::map::Map;
use poke_core::map_objects::{map_text_pointer, text_pointer_in};
use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::DmgPointer;
use poke_core::text_script::{decode, TextCommand};

/// `TX_SCRIPT_VENDING_MACHINE`: from here up the first byte of a map text names a dispatcher.
const FIRST_TX_SCRIPT: u8 = 0xF5;
/// `TX_SCRIPT_MART`, followed by the count and the stock.
pub const TX_SCRIPT_MART: u8 = 0xFE;
pub const TX_SCRIPT_POKECENTER_NURSE: u8 = 0xFF;
pub const TX_SCRIPT_BILLS_PC: u8 = 0xFD;
pub const TX_SCRIPT_PLAYERS_PC: u8 = 0xFC;
pub const TX_SCRIPT_POKECENTER_PC: u8 = 0xF9;
pub const TX_SCRIPT_PRIZE_VENDOR: u8 = 0xF7;
pub const TX_SCRIPT_CABLE_CLUB_RECEPTIONIST: u8 = 0xF6;
pub const TX_SCRIPT_VENDING_MACHINE: u8 = 0xF5;

/// What `DisplayTextID` finds behind a map's text id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapText {
    /// Text the text engine prints on its own.
    Plain(Vec<TextCommand>),
    /// A `TX_SCRIPT_*` byte and where it is: a mart, a nurse, a PC and the rest of `DisplayTextID`'s
    /// dispatch.
    Dispatch(u8, DmgPointer),
    /// Text that runs code (`text_asm`), which is the script runtime's to recreate.
    Script(DmgPointer),
}

pub fn map_text(map: Map, text_id: u8) -> Result<MapText, String> {
    classify(map_text_pointer(map, text_id)?)
}

/// What a text id names in a text pointer table a script has swapped in.
pub fn map_text_in(table: DmgPointer, text_id: u8) -> Result<MapText, String> {
    classify(text_pointer_in(table, text_id)?)
}

/// `TextPredefs`' entry `text_id`, whose address is read in `bank`: `PrintPredefTextID` keeps the
/// caller's bank loaded rather than the map's.
pub fn text_predef(text_id: u8, bank: u8) -> Result<MapText, String> {
    let entries = rom_slice(poke_core::symbols::pokered_symbols::TextPredefs);
    let entry = (text_id as usize).checked_sub(1).ok_or("text predef 0")? * 2;
    let address = u16::from_le_bytes([entries[entry], entries[entry + 1]]);
    let bank = if address < 0x4000 { 0 } else { bank };
    classify(DmgPointer { bank: poke_core::symbols::DmgBank::ROM { bank }, address })
}

fn classify(pointer: DmgPointer) -> Result<MapText, String> {
    let first = rom_slice(pointer)[0];
    if first >= FIRST_TX_SCRIPT {
        return Ok(MapText::Dispatch(first, pointer));
    }
    let commands = decode(pointer)?;
    Ok(match commands.iter().find_map(|command| match command { TextCommand::Asm(at) => Some(*at), _ => None }) {
        Some(_) => MapText::Script(pointer),
        None => MapText::Plain(commands),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pallet_town_s_girl_speaks_plainly_and_oak_runs_code() {
        assert!(matches!(map_text(Map::PalletTown, 2), Ok(MapText::Plain(_))));
        assert!(matches!(map_text(Map::PalletTown, 1), Ok(MapText::Script(_))));
    }

    #[test]
    fn a_mart_clerk_is_a_dispatch() {
        assert!(matches!(map_text(Map::PewterMart, 1), Ok(MapText::Dispatch(TX_SCRIPT_MART, _))));
    }

    #[test]
    fn every_map_text_decodes_into_one_of_the_three() {
        for map in Map::all().filter(|map| map.header_pointer().is_some()) {
            let objects = poke_core::map_objects::MapObjects::read(map).unwrap();
            let ids = objects.objects.iter().map(|o| o.text_id).chain(objects.signs.iter().map(|s| s.text_id));
            for id in ids.filter(|&id| id != 0) {
                map_text(map, id).unwrap_or_else(|e| panic!("{map} text {id}: {e}"));
            }
        }
    }
}

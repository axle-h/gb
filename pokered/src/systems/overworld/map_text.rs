use poke_core::map::Map;
use poke_core::map_objects::TextPointers;
use poke_core::tables::{TextDispatch, TextPredef, TEXT_DISPATCHES};
use poke_core::text_script::{far_text, TextCommand};

/// What `DisplayTextID` finds behind a map's text id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapText {
    /// Text the text engine prints on its own.
    Plain(Vec<TextCommand>),
    /// A `script_*` text: a mart, a nurse, a PC and the rest of `DisplayTextID`'s dispatch.
    Dispatch(TextDispatch),
    /// Text that runs code (`text_asm`), by its label, which is the script runtime's to recreate.
    Script(&'static str),
}

pub fn map_text(map: Map, text_id: u8) -> Result<MapText, String> {
    map_text_in(TextPointers::of(map).ok_or_else(|| format!("{map} has no header of its own"))?, text_id)
}

/// What a text id names in a text pointer table, the header's or one a script has swapped in.
pub fn map_text_in(table: TextPointers, text_id: u8) -> Result<MapText, String> {
    classify(table.text(text_id)?)
}

/// `TextPredefs`' entry `text_id`.
pub fn text_predef(text_id: u8) -> Result<MapText, String> {
    classify(TextPredef::from_id(text_id).ok_or_else(|| format!("no text predef {text_id}"))?.label())
}

fn classify(label: &'static str) -> Result<MapText, String> {
    if let Ok(at) = TEXT_DISPATCHES.binary_search_by(|(name, _)| (*name).cmp(label)) {
        return Ok(MapText::Dispatch(TEXT_DISPATCHES[at].1));
    }
    let commands = far_text(label)?;
    Ok(if commands.iter().any(|command| matches!(command, TextCommand::Asm(_))) { MapText::Script(label) } else { MapText::Plain(commands) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pallet_town_s_girl_speaks_plainly_and_oak_runs_code() {
        assert!(matches!(map_text(Map::PalletTown, 2), Ok(MapText::Plain(_))));
        assert_eq!(map_text(Map::PalletTown, 1), Ok(MapText::Script("PalletTownOakText")));
    }

    #[test]
    fn a_mart_clerk_is_a_dispatch_with_its_stock() {
        use poke_core::item::ItemId::*;
        let stock = [PokeBall, Potion, EscapeRope, Antidote, BurnHeal, Awakening, ParlyzHeal].map(|item| item as u8);
        let Ok(MapText::Dispatch(TextDispatch::Mart(sold))) = map_text(Map::PewterMart, 1) else { panic!("not a mart") };
        assert_eq!(sold, stock);
    }

    #[test]
    fn every_map_text_decodes_into_one_of_the_three() {
        for map in Map::all().filter(|map| map.has_header()) {
            let objects = poke_core::map_objects::MapObjects::read(map).unwrap();
            let ids = objects.objects.iter().map(|o| o.text_id).chain(objects.signs.iter().map(|s| s.text_id));
            for id in ids.filter(|&id| id != 0) {
                map_text(map, id).unwrap_or_else(|e| panic!("{map} text {id}: {e}"));
            }
        }
    }

    /// Bar `UnusedPredefText`, a bare `db "@"` nothing reaches.
    #[test]
    fn every_text_predef_decodes() {
        for predef in TextPredef::ALL.into_iter().filter(|&predef| predef != TextPredef::UnusedPredefText) {
            text_predef(predef as u8).unwrap_or_else(|e| panic!("{predef:?}: {e}"));
        }
        assert!(text_predef(0).is_err() && text_predef(TextPredef::ALL.len() as u8 + 1).is_err());
    }
}

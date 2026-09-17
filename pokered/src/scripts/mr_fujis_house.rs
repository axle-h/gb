//! `MrFujisHouse_Script`: the house that is empty until the tower is cleared, and Mr Fuji handing
//! over the Poké Flute once he is back in it.

use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::{EVENT_GOT_POKE_FLUTE, EVENT_RESCUED_MR_FUJI};
use poke_core::symbols::pokered_local_labels as local;
use poke_core::symbols::pokered_map_scripts::{TEXT_MRFUJISHOUSE_LITTLE_GIRL, TEXT_MRFUJISHOUSE_MR_FUJI,
    TEXT_MRFUJISHOUSE_NIDORINO, TEXT_MRFUJISHOUSE_PSYDUCK, TEXT_MRFUJISHOUSE_SUPER_NERD};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Cry(u8),
    FluteOffered,
    FluteReceived,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    use local::MrFujisHouseLittleGirlText as girl;
    use local::MrFujisHouseMrFujiText as fuji;
    use local::MrFujisHouseSuperNerdText as nerd;
    Some(match text_id {
        TEXT_MRFUJISHOUSE_SUPER_NERD => {
            let said = match rt.check_event(EVENT_RESCUED_MR_FUJI) {
                true => nerd::MrFujiHadBeenPrayingText,
                false => nerd::MrFujiIsntHereText,
            };
            rt.print_text(text_at(said)).ret()
        }
        TEXT_MRFUJISHOUSE_LITTLE_GIRL => {
            let said = match rt.check_event(EVENT_RESCUED_MR_FUJI) {
                true => girl::PokemonAreNiceToHugText,
                false => girl::ThisIsMrFujisHouseText,
            };
            rt.print_text(text_at(said)).ret()
        }
        TEXT_MRFUJISHOUSE_PSYDUCK => {
            rt.print_text(text_at(sym::MrFujisHousePsyduckText)).then(Label::Cry(PokemonSpecies::Psyduck as u8))
        }
        TEXT_MRFUJISHOUSE_NIDORINO => {
            rt.print_text(text_at(sym::MrFujisHouseNidorinoText)).then(Label::Cry(PokemonSpecies::Nidorino as u8))
        }
        TEXT_MRFUJISHOUSE_MR_FUJI => match rt.check_event(EVENT_GOT_POKE_FLUTE) {
            true => rt.print_text(text_at(fuji::HasMyFluteHelpedYouText)).ret(),
            false => rt.print_text(text_at(fuji::IThinkThisMayHelpYourQuestText)).then(Label::FluteOffered),
        },
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    use local::MrFujisHouseMrFujiText as fuji;
    match label {
        Label::Cry(species) => {
            rt.play_cry(PokemonSpecies::from_repr(species).expect("a species the text named"));
            rt.wait_for_sound_to_finish().ret()
        }
        Label::FluteOffered => {
            if !rt.give_item(ItemId::PokeFlute, 1) {
                return rt.print_text(text_at(fuji::PokeFluteNoRoomText)).ret();
            }
            // The flag is set after the text, not before it, as `MrFujisHouse.asm` sets it.
            rt.print_text(text_at(fuji::ReceivedPokeFluteText)).then(Label::FluteReceived)
        }
        Label::FluteReceived => {
            rt.set_event(EVENT_GOT_POKE_FLUTE);
            Flow::Return
        }
    }
}

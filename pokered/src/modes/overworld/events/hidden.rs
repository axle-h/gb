//! `CheckForHiddenEventOrBookshelfOrCardKeyDoor` once it has found something, and the text predefs
//! whose texts run code.

use poke_core::item::{self, ItemId};
use poke_core::map_header::TileSetId;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::pokered_events::EVENT_BEAT_SILPH_CO_GIOVANNI;
use poke_core::symbols::{pokered_symbols as sym, DmgBank, DmgPointer};
use poke_core::text_script::{TextBuffer, TextMoney};
use crate::audio::data::sounds;
use crate::mode::Mode;
use crate::modes::text_box::TextBox;
use crate::systems::events::hidden_events::{self, HiddenEvent};
use crate::systems::math::{add_bcd, flag_action, FlagAction};
use crate::systems::pokedex::count_set_bits;
use super::super::script::{text_at, Block, Flow, Routine, Script, Then};
use super::{print, Hidden, Label};

/// `PrintPredefTextID` from a routine in `bank`, then return.
pub(super) fn predef_in(bank: DmgPointer, id: u8) -> Then {
    let DmgBank::ROM { bank } = bank.bank else { unreachable!("a routine is in ROM") };
    Then::call(Routine::PrintPredefTextId { id, bank })
}

/// The text a `text_asm` text prints before its code, over the box `DisplayTextID` drew.
pub(super) fn print_without_box(at: DmgPointer) -> Then {
    Then::block(Block::Mode(Box::new(Mode::TextBox(TextBox::without_box(text_at(at))))))
}

fn facing(s: &Script) -> u8 {
    s.ow.sprites[0].facing
}

pub(super) fn check(s: &mut Script) -> Flow {
    match s.ow.rt.events.hidden.expect("an A press found something") {
        Hidden::Event(event) => hidden_event(s, event),
        Hidden::Bookshelf(text) => {
            s.enable_auto_text_box_drawing();
            predef_in(sym::PrintBookshelfText, text).ret()
        }
        Hidden::CardKeyDoor => card_key(s),
    }
}

/// The hidden event's function, by its label. Whatever it does or declines to do, the press is used.
fn hidden_event(s: &mut Script, event: HiddenEvent) -> Flow {
    let up = facing(s) == SpriteFacing::Up as u8;
    let f = event.function;
    let simple = [
        (sym::OpenRedsPC, 0x03), (sym::PrintRedSNESText, 0x04), (sym::PrintBookcaseText, 0x0E),
        (sym::DisplayOakLabLeftPoster, 0x05), (sym::PrintMagazinesText, 0x30), (sym::PrintNewBikeText, 0x39),
        (sym::PrintFightingDojoText, 0x36), (sym::PrintFightingDojoText2, 0x37), (sym::PrintFightingDojoText3, 0x38),
        (sym::PrintTrashText, 0x26),
    ];
    if let Some(&(_, id)) = simple.iter().find(|&&(label, _)| label == f) {
        s.enable_auto_text_box_drawing();
        return predef_in(f, id).ret();
    }
    if f == sym::HiddenItems {
        return hidden_items(s, event);
    }
    if f == sym::HiddenCoins {
        return hidden_coins(s, event);
    }
    if f == sym::OpenPokemonCenterPC {
        if !up {
            return Flow::Return;
        }
        s.enable_auto_text_box_drawing();
        s.ow.rt.no_auto_text_box = true;
        return predef_in(f, 0x1F).ret();
    }
    if f == sym::DisplayOakLabRightPoster {
        s.enable_auto_text_box_drawing();
        let owned = count_set_bits(&s.ctx.world.pokedex.owned);
        return predef_in(f, if owned < 2 { 0x06 } else { 0x07 }).ret();
    }
    if f == sym::DisplayOakLabEmailText || f == sym::PrintIndigoPlateauHQText {
        if !up {
            return Flow::Return;
        }
        s.enable_auto_text_box_drawing();
        return predef_in(f, if f == sym::DisplayOakLabEmailText { 0x08 } else { 0x27 }).ret();
    }
    if f == sym::PrintBenchGuyText {
        s.enable_auto_text_box_drawing();
        return match hidden_events::bench_guy_text(s.ctx.world.location.map, facing(s)) {
            Some(id) => predef_in(f, id).ret(),
            None => Flow::Return,
        };
    }
    if f == sym::GymStatues {
        s.enable_auto_text_box_drawing();
        let Some(badge) = hidden_events::gym_badge(s.ctx.world.location.map).filter(|_| up) else { return Flow::Return };
        // `wBeatGymFlags`, which the cartridge keeps apart from the badges and always equal to them.
        return predef_in(f, if s.ctx.world.badges & badge == badge { 0x0D } else { 0x0C }).ret();
    }
    if f == sym::PrintNotebookText || f == sym::PrintBlackboardLinkCableText {
        s.enable_auto_text_box_drawing();
        s.set_do_not_wait_for_button_press(true);
        return predef_in(f, event.argument).ret();
    }
    super::api::hidden_event(s, event)
}

/// `HiddenItems`.
fn hidden_items(s: &mut Script, event: HiddenEvent) -> Flow {
    let index = hidden_events::hidden_item_index(s.ctx.world.location.map, event.x, event.y);
    s.ow.rt.events.hidden_item_or_coins_index = index;
    if flag_action(&mut s.ctx.world.hidden_items, index, FlagAction::Test) != 0 {
        return Flow::Return;
    }
    s.enable_auto_text_box_drawing();
    s.set_do_not_wait_for_button_press(true);
    let name = ItemId::from_repr(event.argument).map(item::name).unwrap_or_default();
    s.ctx.world.text.strings.insert(TextBuffer::NameBuffer, name);
    predef_in(event.function, 0x24).ret()
}

/// `FoundHiddenItemText`'s code.
pub(super) fn hidden_item_found(s: &mut Script) -> Flow {
    let event = match s.ow.rt.events.hidden {
        Some(Hidden::Event(event)) => event,
        _ => return Flow::Return,
    };
    let item = ItemId::from_repr(event.argument).expect("a hidden item is an item");
    if !s.give_item(item, 1) {
        return Then::block(Block::TextScrollButton).then(Label::HiddenItemBagFull);
    }
    let index = s.ow.rt.events.hidden_item_or_coins_index;
    flag_action(&mut s.ctx.world.hidden_items, index, FlagAction::Set);
    // `PlaySoundWaitForCurrent`, then `WaitForSoundToFinish`.
    Then::block(Block::Sound).then(Label::HiddenItemSound)
}

pub(super) fn hidden_item_sound(s: &mut Script) -> Flow {
    s.play_sound(sounds::SFX_GET_ITEM_2);
    s.wait_for_sound_to_finish().ret()
}

pub(super) fn hidden_item_bag_full(s: &mut Script) -> Flow {
    s.set_do_not_wait_for_button_press(false);
    print(sym::HiddenItemBagFullText).ret()
}

/// `HiddenCoins`: nothing without the Coin Case, and nothing twice. A purse topped up to 9999 says the
/// rest were dropped, even when none were.
fn hidden_coins(s: &mut Script, event: HiddenEvent) -> Flow {
    let world = &mut s.ctx.world;
    if world.bag.quantity_of(ItemId::CoinCase) == 0 {
        return Flow::Return;
    }
    let index = hidden_events::hidden_coin_index(world.location.map, event.x, event.y);
    s.ow.rt.events.hidden_item_or_coins_index = index;
    if flag_action(&mut world.hidden_coins, index, FlagAction::Test) != 0 {
        return Flow::Return;
    }
    let amount = hidden_events::hidden_coins_amount(event.argument);
    world.text.money.insert(TextMoney::Coins, amount.to_vec());
    add_bcd(&mut world.coins, &amount);
    flag_action(&mut world.hidden_coins, index, FlagAction::Set);
    let full = world.coins == [0x99, 0x99];
    s.enable_auto_text_box_drawing();
    predef_in(event.function, if full { 0x2C } else { 0x2B }).ret()
}

/// `GetCoordsInFrontOfPlayer`.
fn coords_in_front(s: &Script) -> (u8, u8) {
    let location = &s.ctx.world.location;
    hidden_events::in_front(location.x, location.y, facing(s))
}

/// `PrintCardKeyText` at a card key door.
fn card_key(s: &mut Script) -> Flow {
    if s.ctx.world.bag.quantity_of(ItemId::CardKey) == 0 {
        return predef_in(sym::PrintCardKeyText, 0x02).ret();
    }
    let (x, y) = coords_in_front(s);
    s.ow.rt.events.card_key_door = (x >> 1, y >> 1);
    predef_in(sym::PrintCardKeyText, 0x01).then(Label::CardKeyOpened { x: x >> 1, y: y >> 1 })
}

pub(super) fn card_key_opened(s: &mut Script, x: u8, y: u8) -> Flow {
    let block = hidden_events::card_key_door_block(s.ctx.world.location.map);
    s.replace_tile_block(x, y, block);
    s.ow.rt.cur_map_loaded[0] = true;
    s.play_sound(sounds::SFX_GO_INSIDE);
    Flow::Return
}

/// A text predef that runs code, entered after `DisplayTextID` has drawn its box.
pub fn predef_text(s: &mut Script, at: DmgPointer) -> Option<Flow> {
    Some(if at == sym::FoundHiddenItemText {
        print_without_box(at).then(Label::HiddenItemFound)
    } else if at == sym::SaffronCityPokecenterBenchGuyText {
        let text = if s.check_event(EVENT_BEAT_SILPH_CO_GIOVANNI) {
            sym::SaffronCityPokecenterBenchGuyText2
        } else {
            sym::SaffronCityPokecenterBenchGuyText1
        };
        print(text).ret()
    } else if at == sym::BookOrSculptureText {
        let diglett = s.ow.view.tileset == TileSetId::Mansion && s.ow.tile_map(s.ctx)[6 * 20 + 8] == 0x38;
        print(if diglett { sym::DiglettSculptureText } else { sym::PokemonBooksText }).ret()
    } else if at == sym::IndigoPlateauStatues {
        print(sym::IndigoPlateauStatuesText1).then(Label::IndigoPlateauStatues)
    } else if at == sym::TownMapText {
        print_without_box(at).then(Label::TownMap)
    } else {
        return super::api::predef_text(s, at);
    })
}

pub(super) fn indigo_plateau_statues(s: &mut Script) -> Flow {
    let text = if s.x() & 1 != 0 { sym::IndigoPlateauStatuesText2 } else { sym::IndigoPlateauStatuesText3 };
    print(text).ret()
}

/// `TownMapText`'s code: `DisplayTownMap`, then `CloseTextDisplay` before the text ends, so the box
/// is closed twice.
pub(super) fn town_map(s: &mut Script) -> Flow {
    s.set_do_not_wait_for_button_press(true);
    Then::call(Routine::DisplayTownMap).then(Label::TownMapClosed)
}

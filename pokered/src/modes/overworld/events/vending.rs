//! `VendingMachineMenu`: three drinks, a money box, and sixty rattles before one drops.

use poke_core::item::vending_prices;
use poke_core::symbols::pokered_symbols as sym;
use crate::audio::data::sounds;
use crate::gfx::text_boxes::money_box;
use crate::mode::{Mode, Outcome};
use crate::modes::cursor_menu::CursorMenu;
use crate::systems::math::sub_bcd;
use crate::systems::money::has_enough;
use super::super::script::{Block, Flow, Script, Then};
use super::{place_rom_string, print, Label};

const CANCEL: u8 = 3;
/// `.playDeliverySound`: sixty times, two frames apart.
const RATTLES: u8 = 60;
const RATTLE_FRAMES: u8 = 2;
/// `HasEnoughMoney` is asked about ¥200, whichever drink was chosen.
const PRICE_CHECKED: [u8; 3] = [0x00, 0x02, 0x00];

pub(super) fn vending_machine(_s: &mut Script) -> Flow {
    print(sym::VendingMachineText1).then(Label::VendingMachineMenu)
}

pub(super) fn menu(s: &mut Script) -> Flow {
    let money = s.ctx.world.money;
    let ui = &mut s.ctx.screen.ui;
    money_box(ui, &money);
    s.ctx.menu.last_item = 0;
    ui.text_box_border(0, 3, 12, 8);
    s.update_sprites();
    let ui = &mut s.ctx.screen.ui;
    place_rom_string(ui, 2, 5, sym::DrinkText);
    place_rom_string(ui, 9, 6, sym::DrinkPriceText);
    let menu = CursorMenu::new(0, CANCEL, (1, 5));
    Then::block(Block::Mode(Box::new(Mode::CursorMenu(menu)))).then(Label::VendingMachineChosen)
}

pub(super) fn chosen(s: &mut Script) -> Flow {
    let row = match s.ow.rt.outcome {
        Some(Outcome::Chosen(row)) if row != CANCEL => row,
        _ => return print(sym::VendingMachineText7).ret(),
    };
    if !has_enough(&s.ctx.world.money, &PRICE_CHECKED) {
        return print(sym::VendingMachineText4).ret();
    }
    let (drink, price) = vending_prices()[row as usize];
    if !s.give_item(drink, 1) {
        return print(sym::VendingMachineText6).ret();
    }
    s.ow.rt.events.vending_price = price;
    Flow::Jump(Label::VendingMachineDeliver(RATTLES).into())
}

pub(super) fn deliver(s: &mut Script, rattles: u8) -> Flow {
    if rattles == 0 {
        return print(sym::VendingMachineText5).then(Label::VendingMachinePaid);
    }
    s.delay_frames(RATTLE_FRAMES).then(Label::VendingMachineRattle(rattles))
}

pub(super) fn rattle(s: &mut Script, rattles: u8) -> Flow {
    s.play_sound(sounds::SFX_PUSH_BOULDER);
    Flow::Jump(Label::VendingMachineDeliver(rattles - 1).into())
}

/// `SubBCDPredef` stops at nothing, so a drink dearer than the ¥200 checked leaves an empty purse.
pub(super) fn paid(s: &mut Script) -> Flow {
    let price = s.ow.rt.events.vending_price;
    sub_bcd(&mut s.ctx.world.money, &price);
    let money = s.ctx.world.money;
    money_box(&mut s.ctx.screen.ui, &money);
    Flow::Return
}

//! A mart, against the cartridge: the Viridian clerk, a purchase and a sale, with the whole screen
//! compared wherever both are waiting for a button.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::ROM;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::pokemart::Pokemart;
use crate::pokemon::item::ItemId;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointerRead};
use super::item_menu::{recreated, screen, step, the_bag, the_game};
use super::{breakpoint, hurried_letter, joypad, recreation_until_polling, ARROW, BOX, CURSOR, LIST};

fn coords(gb: &GameBoy) -> (u8, u8) {
    let mmu = gb.core().mmu();
    (mmu.read_pointer(&sym::wXCoord), mmu.read_pointer(&sym::wYCoord))
}

/// Holds `button` until the player stands on `to`, then lets the step finish.
fn walk(gb: &mut GameBoy, button: Joypad, to: (u8, u8)) {
    gb.hold_buttons(joypad(button));
    for _ in 0..120 {
        if coords(gb) == to {
            break;
        }
        gb.run(MachineCycles::PER_FRAME);
    }
    gb.hold_buttons(JoypadButtonState::default());
    assert_eq!(coords(gb), to, "the walk was blocked");
    gb.run(MachineCycles::PER_FRAME * 30);
}

/// The Viridian Mart fixture walked up to the counter, and the clerk spoken to: stopped where the
/// greeting is about to print, with the stock in `wItemList`.
fn talk_to_the_clerk() -> (GameBoy, Vec<ItemId>) {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(include_bytes!("../pokemon/data/viridian-city-pokemart-shopping.bin")).unwrap();
    gb.run(MachineCycles::PER_FRAME * 30);
    assert_eq!(coords(&gb), (3, 7), "the fixture starts at the door");
    walk(&mut gb, Joypad::UP, (3, 6));
    walk(&mut gb, Joypad::UP, (3, 5));
    walk(&mut gb, Joypad::LEFT, (2, 5));
    gb.hold_buttons(joypad(Joypad::LEFT));
    gb.run(MachineCycles::PER_FRAME * 4);
    gb.hold_buttons(joypad(Joypad::A));
    let mart = breakpoint(sym::DisplayPokemartDialogue);
    let (stop, _) = gb.run_until(&[mart], MachineCycles::PER_FRAME * 120);
    assert_eq!(stop, Stop::Breakpoint(mart), "the clerk never opened the mart");
    gb.hold_buttons(JoypadButtonState::default());
    let list = gb.core().registers().hl() + 1;
    let mmu = gb.core().mmu();
    let count = mmu.read(list) as u16;
    let items = (1..=count).map(|i| ItemId::from_repr(mmu.read(list + i)).expect("an item")).collect();
    (gb, items)
}

#[test]
fn buying_and_selling_shows_what_the_cartridge_shows_at_every_poll() {
    let (mut gb, items) = talk_to_the_clerk();
    assert_eq!(items, [ItemId::PokeBall, ItemId::Antidote, ItemId::ParlyzHeal, ItemId::BurnHeal]);
    let mut game = the_game(&gb);
    let hurry = hurried_letter(&game);
    game.push(Mode::Pokemart(Pokemart::new(items)));
    super::cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::BuySellQuit);
    assert_eq!(screen(&gb), recreated(&game), "BUY/SELL/QUIT");

    step(&mut gb, &mut game, Joypad::A, Decision::List, BOX + hurry + LIST, "the stock");
    step(&mut gb, &mut game, Joypad::DOWN, Decision::List, CURSOR, "down to the Antidote");
    step(&mut gb, &mut game, Joypad::A, Decision::Quantity, 0, "how many");
    step(&mut gb, &mut game, Joypad::UP, Decision::Quantity, 0, "two");
    step(&mut gb, &mut game, Joypad::UP, Decision::Quantity, 0, "three");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + hurry + ARROW, "that will be");
    step(&mut gb, &mut game, Joypad::A, Decision::TwoOption, CURSOR, "OK?");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "here you are");
    assert_eq!((game.world().money, game.world().bag.clone()),
        (gb.core().mmu().read_slice(sym::wPlayerMoney.address, 3).try_into().unwrap(), the_bag(&gb)));
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the stock again");
    step(&mut gb, &mut game, Joypad::B, Decision::BuySellQuit, BOX + hurry + CURSOR, "anything else, and BUY/SELL/QUIT again");

    step(&mut gb, &mut game, Joypad::DOWN, Decision::BuySellQuit, CURSOR, "SELL");
    step(&mut gb, &mut game, Joypad::A, Decision::List, BOX + hurry + LIST, "the bag");
    let antidotes = game.world().bag.items.iter().position(|slot| slot.id == ItemId::Antidote).unwrap();
    for press in 0..antidotes {
        step(&mut gb, &mut game, Joypad::DOWN, Decision::List, CURSOR, &format!("down {press}"));
    }
    step(&mut gb, &mut game, Joypad::A, Decision::Quantity, 0, "how many to sell");
    step(&mut gb, &mut game, Joypad::DOWN, Decision::Quantity, 0, "all of them, round the bottom");
    step(&mut gb, &mut game, Joypad::A, Decision::TwoOption, BOX + hurry + CURSOR, "I can pay you");
    step(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the bag after the sale");
    assert_eq!((game.world().money, game.world().bag.clone()),
        (gb.core().mmu().read_slice(sym::wPlayerMoney.address, 3).try_into().unwrap(), the_bag(&gb)));
    step(&mut gb, &mut game, Joypad::B, Decision::BuySellQuit, BOX + hurry + CURSOR, "anything else, and BUY/SELL/QUIT once more");
}


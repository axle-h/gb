//! A Pokémon Center's PC against the cartridge: the Cerulean PC turned on, the player's PC, an item
//! deposited with a count, withdrawn, one tossed, and both menus logged off, with the screen compared
//! wherever both wait for a button and the bag and the PC's items compared at the end. Then BILL's PC
//! over a party and a box written into WRAM: a deposit, STATS and a withdrawal, a release refused and
//! then made, and a box change, with the party, the box and the box number compared along the way.
//! Then `<PKMN>LEAGUE` over a Hall of Fame record copied out of a finished game's SRAM.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::{RAM, ROM};
use poke_core::bag::BagItem;
use poke_core::species::PokemonSpecies;
use pokered::systems::hall_of_fame::{HallOfFameMon, HOF_TEAM_CAPACITY};
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::pc::PcMenu;
use pokered::rng::GameRng;
use pokered::systems::inventory::Inventory;
use pokered::{Game, Input, Pacing};
use crate::pokemon::item::ItemId;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointerRead};
use super::battle::letters;
use super::item_menu::{screen, the_bag, the_world};
use super::{assert_late, breakpoint, cartridge_until_polling, joypad, recreation_until_polling, tile_row, to_vblank,
            ARROW, BOX, CURSOR, DELAY3, LIST};

/// Where a Pokémon Center's PC is used from: below it, facing up.
const AT_THE_PC: (u8, u8) = (13, 4);

fn coords(gb: &GameBoy) -> (u8, u8) {
    let mmu = gb.core().mmu();
    (mmu.read_pointer(&sym::wXCoord), mmu.read_pointer(&sym::wYCoord))
}

/// Holds `button` until `arrived`, waiting out a wandering NPC in the way, then lets the step finish.
fn walk(gb: &mut GameBoy, button: Joypad, arrived: impl Fn((u8, u8)) -> bool) {
    gb.hold_buttons(joypad(button));
    for _ in 0..1200 {
        if arrived(coords(gb)) {
            break;
        }
        gb.run(MachineCycles::PER_FRAME);
    }
    gb.hold_buttons(JoypadButtonState::default());
    assert!(arrived(coords(gb)), "the walk stopped at {:?}", coords(gb));
    gb.run(MachineCycles::PER_FRAME * 30);
}

/// `post-cascade.bin` walked from the Cerulean Pokémon Center's door to its PC, and A pressed there:
/// stopped as `ActivatePC` begins.
fn turn_on_the_pc() -> GameBoy {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(include_bytes!("../pokemon/data/post-cascade.bin")).unwrap();
    gb.run(MachineCycles::PER_FRAME * 30);
    walk(&mut gb, Joypad::UP, |(_, y)| y == AT_THE_PC.1);
    walk(&mut gb, Joypad::RIGHT, |(x, _)| x == AT_THE_PC.0);
    gb.hold_buttons(joypad(Joypad::UP));
    gb.run(MachineCycles::PER_FRAME * 4);
    gb.hold_buttons(JoypadButtonState::default());
    gb.run(MachineCycles::PER_FRAME * 10);
    gb.hold_buttons(joypad(Joypad::A));
    let pc = breakpoint(sym::ActivatePC);
    let (stop, _) = gb.run_until(&[pc], MachineCycles::PER_FRAME * 120);
    assert_eq!(stop, Stop::Breakpoint(pc), "A never turned the PC on");
    gb.hold_buttons(JoypadButtonState::default());
    gb
}

/// An inventory written as given, `$FF` after it.
fn write_items(gb: &mut GameBoy, count: u16, at: u16, slots: &[(ItemId, u8)]) {
    let mmu = gb.core_mut().mmu_mut();
    mmu.write(count, slots.len() as u8);
    for (i, &(item, quantity)) in slots.iter().enumerate() {
        mmu.write(at + 2 * i as u16, item as u8);
        mmu.write(at + 2 * i as u16 + 1, quantity);
    }
    mmu.write(at + 2 * slots.len() as u16, 0xFF);
}

fn the_pc_items(gb: &GameBoy) -> Inventory {
    let mmu = gb.core().mmu();
    let count = mmu.read_pointer(&sym::wNumBoxItems) as u16;
    let at = sym::wBoxItems.address;
    Inventory::pc((0..count).map(|i| {
        BagItem::new(ItemId::from_repr(mmu.read(at + 2 * i)).expect("an item"), mmu.read(at + 2 * i + 1))
    }).collect())
}

/// The recreation standing where the cartridge stands, the party and the box included, the screen copied so what is behind the PC
/// compares as well.
fn the_game(gb: &GameBoy) -> Game {
    the_game_with(gb, Vec::new())
}

fn the_game_with(gb: &GameBoy, hall_of_fame: Vec<Vec<HallOfFameMon>>) -> Game {
    let mmu = gb.core().mmu();
    let mut world = the_world(gb);
    world.pc_items = the_pc_items(gb);
    world.hall_of_fame_teams = mmu.read_pointer(&sym::wNumHoFTeams);
    world.hall_of_fame = hall_of_fame;
    let (party, boxed, current) = the_mons(gb);
    world.party = party;
    world.current_box = current;
    world.boxes = vec![Vec::new(); current as usize + 1];
    world.boxes[current as usize] = boxed;
    let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
    let menu = game.menu_mut();
    menu.bag_saved = mmu.read_pointer(&sym::wBagSavedMenuItem);
    menu.list_scroll = mmu.read_pointer(&sym::wListScrollOffset);
    for y in 0..18 {
        for (x, tile) in tile_row(gb, y).into_iter().enumerate() {
            game.screen_mut().ui.set(x, y as usize, tile);
        }
    }
    game
}

/// The screen where the recreation draws over the map. The recreation has no overworld under the
/// PC, so once `ReloadMapData` has put the map back it is the menus alone that can be compared.
fn drawn(game: &Game) -> Vec<Vec<Option<u8>>> {
    (0..18).map(|y| (0..20).map(|x| game.ui().cover(x, y)).collect()).collect()
}

fn same_where_drawn(gb: &GameBoy, game: &Game, what: &str) {
    let cartridge = screen(gb);
    for (y, row) in drawn(game).into_iter().enumerate() {
        let ours: Vec<u8> = row.iter().zip(&cartridge[y]).map(|(ours, theirs)| ours.unwrap_or(*theirs)).collect();
        assert_eq!(ours, cartridge[y], "{what}: row {y}");
    }
}

/// What VRAM holds where the screen's tile map is transferred to: what a player sees, rather than
/// the tile map the cartridge is still filling in.
fn vram_row(gb: &GameBoy, y: u16) -> Vec<u8> {
    let mmu = gb.core().mmu();
    let at = sym::hAutoBGTransferDest.address;
    let dest = u16::from_le_bytes([mmu.read(at), mmu.read(at + 1)]);
    mmu.read_vram_slice(dest + 32 * y, 20).expect("the tile map is in VRAM").to_vec()
}

/// The frames a text was on a row half written and whole.
#[derive(Debug, Default, PartialEq, Eq)]
struct Seen {
    half: u32,
    whole: u32,
}

impl Seen {
    /// A row starting with the text's first letter has either all of it or part of it.
    fn count(&mut self, row: &[u8], text: &[u8]) {
        if row[1] != text[0] {
            return;
        }
        if row[1..1 + text.len()] == *text { self.whole += 1 } else { self.half += 1 }
    }
}

/// [`cartridge_until_polling`], watching `text` on row `y` of what is on the screen every frame.
fn cartridge_watching(gb: &mut GameBoy, text: &[u8], y: u16) -> (u32, Seen) {
    let poll = breakpoint(sym::JoypadLowSensitivity);
    let vblank = breakpoint(sym::VBlank);
    let mut seen = Seen::default();
    for frames in 0..600 {
        loop {
            let (stop, _) = gb.run_until(&[poll, vblank], MachineCycles::PER_FRAME * 2);
            if stop == Stop::Breakpoint(poll) {
                to_vblank(gb);
                seen.count(&vram_row(gb, y), text);
                return (frames + 1, seen);
            }
            assert_eq!(stop, Stop::Breakpoint(vblank), "{stop:?}");
            break;
        }
        seen.count(&vram_row(gb, y), text);
    }
    panic!("the cartridge never polled");
}

/// [`recreation_until_polling`], watched the same way.
fn recreation_watching(game: &mut Game, decision: Decision, text: &[u8], y: usize) -> (u32, Seen) {
    let mut seen = Seen::default();
    for frames in 1..600 {
        game.frame(Input::None);
        seen.count(game.ui().row(y), text);
        if game.status() == Status::Waiting(decision.clone()) {
            return (frames, seen);
        }
    }
    panic!("the recreation never polled");
}

/// A press, the screens the same, and `text` never half written on either: printed with the
/// background transfer off, it reaches the screen whole, in the frame the menu under it does.
fn poll_whole(gb: &mut GameBoy, game: &mut Game, text: &str, loading: u32, what: &str) {
    let text = poke_core::charmap::encode(text).expect("the text encodes");
    gb.hold_buttons(joypad(Joypad::A));
    game.frame(Input::Buttons(Joypad::A));
    to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    let (cartridge, theirs) = cartridge_watching(gb, &text, 14);
    let (recreation, ours) = recreation_watching(game, Decision::CursorMenu, &text, 14);
    assert_eq!(screen(gb), (0..18).map(|y| game.ui().row(y).to_vec()).collect::<Vec<_>>(), "{what}");
    assert_eq!((theirs.half, ours.half), (0, 0), "{what}: half written");
    assert_eq!(ours.whole, 1, "{what}: the recreation shows it in the frame the text ends");
    // What the cartridge holds it up for before it polls is the loading the recreation drops.
    assert!((1..=loading).contains(&theirs.whole), "{what}: the cartridge showed it for {} frames", theirs.whole);
    assert_late(cartridge, recreation, loading, what);
}

/// A press into both and both until they wait again: the frames each took.
fn press(gb: &mut GameBoy, game: &mut Game, button: Joypad, then: Decision) -> (u32, u32) {
    gb.hold_buttons(joypad(button));
    game.frame(Input::Buttons(button));
    to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    (cartridge_until_polling(gb), recreation_until_polling(game, then))
}

/// A press, the screens the same, and the cartridge late by `loading`.
fn step(gb: &mut GameBoy, game: &mut Game, button: Joypad, then: Decision, loading: u32, what: &str) {
    let (cartridge, recreation) = press(gb, game, button, then);
    assert_eq!(screen(gb), (0..18).map(|y| game.ui().row(y).to_vec()).collect::<Vec<_>>(), "{what}");
    assert_late(cartridge, recreation, loading, what);
}

#[test]
fn depositing_withdrawing_and_tossing_shows_what_the_cartridge_shows_at_every_poll() {
    let mut gb = turn_on_the_pc();
    write_items(&mut gb, sym::wNumBagItems.address, sym::wBagItems.address,
        &[(ItemId::Antidote, 5), (ItemId::TownMap, 1), (ItemId::PokeBall, 3)]);
    write_items(&mut gb, sym::wNumBoxItems.address, sym::wBoxItems.address, &[(ItemId::Potion, 1)]);
    let mut game = the_game(&gb);
    game.push(Mode::PcMenu(PcMenu::new()));
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::Text);
    assert_eq!(screen(&gb), (0..18).map(|y| game.ui().row(y).to_vec()).collect::<Vec<_>>(), "turned on");
    let dex_rows = game.world().events.is_set(poke_core::symbols::pokered_events::EVENT_GOT_POKEDEX);
    assert!(dex_rows, "the fixture has the Pokédex, so the box covers the player");

    // `ActivatePC`'s `Delay3` after putting the screen back.
    step(&mut gb, &mut game, Joypad::A, Decision::CursorMenu, DELAY3 + CURSOR, "the PC's menu");
    step(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "down to the player's PC");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "accessed my PC");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, ARROW, "item storage system");
    step(&mut gb, &mut game, Joypad::A, Decision::CursorMenu, BOX + CURSOR, "what do you want to do");

    step(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "DEPOSIT ITEM");
    step(&mut gb, &mut game, Joypad::A, Decision::List, BOX + LIST, "the bag");
    step(&mut gb, &mut game, Joypad::A, Decision::Quantity, BOX, "how many");
    step(&mut gb, &mut game, Joypad::UP, Decision::Quantity, 0, "two");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "stored via PC");
    step(&mut gb, &mut game, Joypad::A, Decision::List, BOX + LIST, "the bag again");
    step(&mut gb, &mut game, Joypad::B, Decision::CursorMenu, BOX + CURSOR, "the menu, on DEPOSIT");
    assert_eq!((game.world().bag.clone(), game.world().pc_items.clone()), (the_bag(&gb), the_pc_items(&gb)), "after the deposit");
    assert_eq!(game.world().pc_items.quantity_of(ItemId::Antidote), 2);

    step(&mut gb, &mut game, Joypad::UP, Decision::CursorMenu, CURSOR, "WITHDRAW ITEM");
    step(&mut gb, &mut game, Joypad::A, Decision::List, BOX + LIST, "the PC's items");
    step(&mut gb, &mut game, Joypad::A, Decision::Quantity, BOX, "how many to withdraw");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "withdrew");
    step(&mut gb, &mut game, Joypad::A, Decision::List, BOX + LIST, "the PC's items again");
    step(&mut gb, &mut game, Joypad::B, Decision::CursorMenu, BOX + CURSOR, "the menu, on WITHDRAW");

    step(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "DEPOSIT ITEM again");
    step(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "TOSS ITEM");
    step(&mut gb, &mut game, Joypad::A, Decision::List, BOX + LIST, "what to toss");
    step(&mut gb, &mut game, Joypad::A, Decision::Quantity, BOX, "how many to toss");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "is it OK");
    step(&mut gb, &mut game, Joypad::A, Decision::TwoOption, CURSOR, "yes or no");
    step(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "threw away");
    step(&mut gb, &mut game, Joypad::A, Decision::List, BOX + LIST, "what to toss again");
    step(&mut gb, &mut game, Joypad::B, Decision::CursorMenu, BOX + CURSOR, "the menu, on TOSS");
    assert_eq!((game.world().bag.clone(), game.world().pc_items.clone()), (the_bag(&gb), the_pc_items(&gb)), "after the toss");

    step(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "LOG OFF");
    // `ReloadMapData` turns the LCD off, so the way back to the PC's menu is untimed.
    gb.hold_buttons(joypad(Joypad::A));
    game.frame(Input::Buttons(Joypad::A));
    to_vblank(&mut gb);
    gb.hold_buttons(JoypadButtonState::default());
    let poll = breakpoint(sym::JoypadLowSensitivity);
    let (stop, _) = gb.run_until(&[poll], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(poll), "the cartridge never polled the PC's menu again");
    to_vblank(&mut gb);
    recreation_until_polling(&mut game, Decision::CursorMenu);
    same_where_drawn(&gb, &game, "the PC's menu over the map");

    gb.hold_buttons(joypad(Joypad::B));
    game.frame(Input::Buttons(Joypad::B));
    to_vblank(&mut gb);
    gb.hold_buttons(JoypadButtonState::default());
    let off = breakpoint(sym::CloseTextDisplay);
    let (stop, _) = gb.run_until(&[off], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(off), "the cartridge never logged off");
    for _ in 0..600 {
        if game.modes().is_empty() {
            break;
        }
        game.frame(Input::None);
    }
    assert!(game.modes().is_empty(), "the recreation never logged off");
    assert_eq!((game.world().bag.clone(), game.world().pc_items.clone()), (the_bag(&gb), the_pc_items(&gb)), "at the end");
}

const PARTY_STRUCT: u16 = 0x2C;
/// `BillsPCMenu`'s `CopyVideoData` of the ball: a frame for up to eight tiles.
const BALL_TILE: u32 = 1;
/// `LoadHpBarAndStatusTilePatterns` with the LCD on, eight tiles a frame.
fn hp_bar_tiles() -> u32 {
    let bytes = (sym::HpBarAndStatusGraphicsEnd.address - sym::HpBarAndStatusGraphics.address) as u32;
    (bytes / 16).div_ceil(8)
}
/// `EmptyAllSRAMBoxes` on a first box change: lag frames summing the checksums of both SRAM banks.
const EMPTY_ALL_SRAM_BOXES: u32 = 21;
/// `ChangeBox`'s two `CopyBoxToOrFromSRAM` and `SaveGameData`: lag frames, all of them checksums and
/// copies.
const CHANGE_BOX_SAVE: u32 = 37;
/// `BillsPCMenu` drawn: the ball, "What?"'s box, the `Delay3` before the menu, and the cursor.
const BILLS_MENU: u32 = BALL_TILE + BOX + DELAY3 + CURSOR;
const BOX_STRUCT: u16 = 0x21;
const NAME_LENGTH: u16 = 11;

/// `wBoxMons` and the names beside them: the current box as WRAM holds it.
fn the_box(gb: &GameBoy) -> Vec<pokered::party::Named<pokered::party::BoxMon>> {
    let mmu = gb.core().mmu();
    let name = |at: u16| (0..NAME_LENGTH).map(|i| mmu.read(at + i)).take_while(|&byte| byte != 0x50).collect::<Vec<u8>>();
    (0..mmu.read(sym::wBoxCount.address) as u16).map(|slot| {
        let b: Vec<u8> = (0..BOX_STRUCT).map(|i| mmu.read(sym::wBoxMons.address + BOX_STRUCT * slot + i)).collect();
        let word = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
        let mon = pokered::party::BoxMon {
            species: poke_core::species::PokemonSpecies::from_repr(b[0]).expect("a species"),
            hp: word(1),
            box_level: b[3],
            status: b[4],
            types: [b[5], b[6]],
            catch_rate: b[7],
            moves: [0, 1, 2, 3].map(|i| poke_core::move_name::PokemonMoveName::from_repr(b[8 + i])),
            ot_id: word(12),
            exp: u32::from_be_bytes([0, b[14], b[15], b[16]]),
            stat_exp: [0, 1, 2, 3, 4].map(|i| word(17 + 2 * i)),
            dvs: pokered::systems::stats::Dvs([b[27], b[28]]),
            pp: [b[29], b[30], b[31], b[32]],
        };
        pokered::party::Named { mon, ot: name(sym::wBoxMonOT.address + NAME_LENGTH * slot),
            nick: name(sym::wBoxMonNicks.address + NAME_LENGTH * slot) }
    }).collect()
}

/// A party of two, and a box of the same two as box mons, written over the fixture's.
fn write_party_and_box(gb: &mut GameBoy) {
    let mmu = gb.core_mut().mmu_mut();
    let count = mmu.read(sym::wPartyCount.address) as u16;
    assert!(count >= 1, "the fixture has a party");
    if count < 2 {
        mmu.write(sym::wPartySpecies.address + 1, mmu.read(sym::wPartySpecies.address));
        for i in 0..PARTY_STRUCT {
            mmu.write(sym::wPartyMons.address + PARTY_STRUCT + i, mmu.read(sym::wPartyMons.address + i));
        }
        for i in 0..NAME_LENGTH {
            mmu.write(sym::wPartyMonOT.address + NAME_LENGTH + i, mmu.read(sym::wPartyMonOT.address + i));
            mmu.write(sym::wPartyMonNicks.address + NAME_LENGTH + i, mmu.read(sym::wPartyMonNicks.address + i));
        }
    }
    mmu.write(sym::wPartySpecies.address + 2, 0xFF);
    mmu.write(sym::wPartyCount.address, 2);
    for slot in 0..2u16 {
        let from = 1 - slot;
        mmu.write(sym::wBoxCount.address + 1 + slot, mmu.read(sym::wPartySpecies.address + from));
        for i in 0..BOX_STRUCT {
            mmu.write(sym::wBoxMons.address + BOX_STRUCT * slot + i, mmu.read(sym::wPartyMons.address + PARTY_STRUCT * from + i));
        }
        // `MON_BOX_LEVEL` from the party mon's level.
        mmu.write(sym::wBoxMons.address + BOX_STRUCT * slot + 3, mmu.read(sym::wPartyMons.address + PARTY_STRUCT * from + 33));
        for i in 0..NAME_LENGTH {
            mmu.write(sym::wBoxMonOT.address + NAME_LENGTH * slot + i, mmu.read(sym::wPartyMonOT.address + NAME_LENGTH * from + i));
            mmu.write(sym::wBoxMonNicks.address + NAME_LENGTH * slot + i, mmu.read(sym::wPartyMonNicks.address + NAME_LENGTH * from + i));
        }
    }
    mmu.write(sym::wBoxCount.address + 3, 0xFF);
    mmu.write(sym::wBoxCount.address, 2);
}

fn the_mons(gb: &GameBoy) -> (Vec<pokered::party::Named<pokered::party::PartyMon>>, Vec<pokered::party::Named<pokered::party::BoxMon>>, u8) {
    let current = gb.core().mmu().read_pointer(&sym::wCurrentBoxNum) & 0x7F;
    (super::status_screen::the_party(gb), the_box(gb), current)
}

fn ours(game: &Game) -> (Vec<pokered::party::Named<pokered::party::PartyMon>>, Vec<pokered::party::Named<pokered::party::BoxMon>>, u8) {
    let world = game.world();
    let current = world.current_box;
    (world.party.clone(), world.boxes.get(current as usize).cloned().unwrap_or_default(), current)
}

/// [`step`], with both screens spelled out when they differ.
fn poll(gb: &mut GameBoy, game: &mut Game, button: Joypad, then: Decision, loading: u32, what: &str) {
    let (cartridge, recreation) = press(gb, game, button, then);
    let theirs = screen(gb);
    let mine: Vec<_> = (0..18).map(|y| game.ui().row(y).to_vec()).collect();
    if theirs != mine {
        let spelled = |rows: &[Vec<u8>]| rows.iter().map(|row| letters(row)).collect::<Vec<_>>().join("\n");
        panic!("{what}:\ncartridge\n{}\nrecreation\n{}\n{theirs:?}\n{mine:?}", spelled(&theirs), spelled(&mine));
    }
    assert_late(cartridge, recreation, loading, what);
}

/// A press into both and both until they wait again, the cartridge through an LCD-off stretch that
/// no frame count survives.
fn untimed(gb: &mut GameBoy, game: &mut Game, button: Joypad, then: Decision) {
    gb.hold_buttons(joypad(button));
    game.frame(Input::Buttons(button));
    to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    let poll = breakpoint(sym::JoypadLowSensitivity);
    let (stop, _) = gb.run_until(&[poll], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(poll), "the cartridge never polled again");
    to_vblank(gb);
    recreation_until_polling(game, then);
}

/// Both left polling long enough for the press sound a prompt played to end. The cartridge's loading
/// outlasts it, and a move waits for it, so without this the recreation's quicker menus would wait
/// where the cartridge does not.
fn let_the_sound_end(gb: &mut GameBoy, game: &mut Game) {
    gb.run(MachineCycles::PER_FRAME * 30);
    for _ in 0..30 {
        game.frame(Input::None);
    }
    assert!(game.audio().sound_finished());
}

#[test]
fn bills_pc_deposits_withdraws_releases_and_changes_box_as_the_cartridge_does() {
    let mut gb = turn_on_the_pc();
    write_party_and_box(&mut gb);
    let mut game = the_game(&gb);
    let hurry = super::hurried_letter(&game);
    game.push(Mode::PcMenu(PcMenu::new()));
    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::Text);
    poll(&mut gb, &mut game, Joypad::A, Decision::CursorMenu, DELAY3 + CURSOR, "the PC's menu");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "accessed someone's PC");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, ARROW, "storage system");
    poll(&mut gb, &mut game, Joypad::A, Decision::CursorMenu, hp_bar_tiles() + BILLS_MENU, "BILL's PC's menu");
    let_the_sound_end(&mut gb, &mut game);

    poll(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "DEPOSIT");
    poll(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the party");
    poll(&mut gb, &mut game, Joypad::DOWN, Decision::List, CURSOR, "the second mon");
    poll(&mut gb, &mut game, Joypad::A, Decision::CursorMenu, CURSOR, "DEPOSIT, STATS, CANCEL");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + hurry + ARROW, "stored in the box");
    assert_eq!(ours(&game), the_mons(&gb), "after the deposit");
    poll_whole(&mut gb, &mut game, "What?", BILLS_MENU, "the menu, on DEPOSIT");
    let_the_sound_end(&mut gb, &mut game);

    poll(&mut gb, &mut game, Joypad::UP, Decision::CursorMenu, CURSOR, "WITHDRAW");
    poll(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the box");
    poll(&mut gb, &mut game, Joypad::A, Decision::CursorMenu, CURSOR, "WITHDRAW, STATS, CANCEL");
    poll(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "STATS");
    // The status screen's pictures are loading, and its lockstep times them; here it is only seen.
    press(&mut gb, &mut game, Joypad::A, Decision::StatusScreen);
    press(&mut gb, &mut game, Joypad::A, Decision::StatusScreen);
    // `ReloadTilesetTilePatterns` turns the LCD off on the way back.
    untimed(&mut gb, &mut game, Joypad::A, Decision::CursorMenu);
    assert_eq!(screen(&gb), (0..18).map(|y| game.ui().row(y).to_vec()).collect::<Vec<_>>(), "back from the stats");
    poll(&mut gb, &mut game, Joypad::UP, Decision::CursorMenu, CURSOR, "WITHDRAW again");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "taken out");
    assert_eq!(ours(&game), the_mons(&gb), "after the withdrawal");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, ARROW, "got it");
    poll(&mut gb, &mut game, Joypad::A, Decision::CursorMenu, BILLS_MENU, "the menu, on WITHDRAW");
    let_the_sound_end(&mut gb, &mut game);

    poll(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "DEPOSIT again");
    poll(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "RELEASE");
    poll(&mut gb, &mut game, Joypad::A, Decision::List, LIST, "the box to release from");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "once released");
    poll(&mut gb, &mut game, Joypad::A, Decision::TwoOption, CURSOR, "OK?");
    poll(&mut gb, &mut game, Joypad::B, Decision::List, LIST, "NO, the list again");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "once released again");
    poll(&mut gb, &mut game, Joypad::A, Decision::TwoOption, CURSOR, "OK? again");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "released outside");
    assert_eq!(ours(&game), the_mons(&gb), "after the release");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, ARROW, "bye");
    poll(&mut gb, &mut game, Joypad::A, Decision::CursorMenu, BILLS_MENU, "the menu, on RELEASE");
    let_the_sound_end(&mut gb, &mut game);

    poll(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "CHANGE BOX");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, BOX + ARROW, "when you change a box");
    poll(&mut gb, &mut game, Joypad::A, Decision::Text, ARROW, "will be saved");
    poll(&mut gb, &mut game, Joypad::A, Decision::TwoOption, CURSOR, "is that okay");
    poll(&mut gb, &mut game, Joypad::A, Decision::CursorMenu, EMPTY_ALL_SRAM_BOXES + BOX + CURSOR, "the boxes");
    poll(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "BOX 2");
    poll(&mut gb, &mut game, Joypad::A, Decision::CursorMenu, CHANGE_BOX_SAVE + BILLS_MENU, "saved, and the menu");
    let_the_sound_end(&mut gb, &mut game);
    assert_eq!(ours(&game), the_mons(&gb), "after the change");
    assert_eq!(game.world().current_box, 1);

    poll(&mut gb, &mut game, Joypad::DOWN, Decision::CursorMenu, CURSOR, "SEE YA!");
    // `ReloadMapData` turns the LCD off, so the way back to the PC's menu is untimed.
    untimed(&mut gb, &mut game, Joypad::A, Decision::CursorMenu);
    same_where_drawn(&gb, &game, "the PC's menu over the map");
    assert_eq!(ours(&game), the_mons(&gb), "at the end");
}

/// `sHallOfFame`'s offset into the first SRAM bank, and `HOF_MON`.
const S_HALL_OF_FAME: usize = 0x598;
const HOF_MON: usize = 16;
const HOF_TEAM: usize = 6 * HOF_MON;
/// `LeaguePCShowMon`'s waits for the screen: `GBPalWhiteOutWithDelay3` and `ClearScreen`'s `Delay3`.
const HOF_MON_SCREEN: u32 = DELAY3 + DELAY3;

/// `sHallOfFame`'s team `index`, as far as its `$FF`.
fn sram_team(sram: &[u8], index: usize) -> Vec<HallOfFameMon> {
    let team = &sram[S_HALL_OF_FAME + index * HOF_TEAM..S_HALL_OF_FAME + (index + 1) * HOF_TEAM];
    team.chunks_exact(HOF_MON).take_while(|entry| entry[0] != 0xFF).map(|entry| HallOfFameMon {
        species: PokemonSpecies::from_repr(entry[0]).expect("a species"),
        level: entry[1],
        nick: entry[2..13].iter().copied().take_while(|&b| b != 0x50).collect(),
    }).collect()
}

/// The Cerulean PC turned on with a finished game's Hall of Fame: `post-cascade.bin` has never won,
/// so the record and `wNumHoFTeams` are copied from a fixture that has.
fn with_a_hall_of_fame() -> (GameBoy, Vec<Vec<HallOfFameMon>>) {
    let mut won = GameBoy::dmg(crate::pokemon::roms::POKERED);
    won.load_state(include_bytes!("../pokemon/data/postgame-post-credits.bin")).unwrap();
    let teams = won.core().mmu().read_pointer(&sym::wNumHoFTeams);
    assert!(teams > 0, "the fixture has a Hall of Fame record");
    let sram = won.dump_sram();
    let recorded: Vec<Vec<HallOfFameMon>> =
        (0..(teams as usize).min(HOF_TEAM_CAPACITY)).map(|i| sram_team(&sram, i)).collect();
    assert!(recorded.iter().all(|team| !team.is_empty()), "every team has a mon: {recorded:?}");
    let mut gb = turn_on_the_pc();
    gb.restore_sram(&sram).unwrap();
    gb.core_mut().mmu_mut().write(sym::wNumHoFTeams.address, teams);
    (gb, recorded)
}

/// The PC's menu with the league's row on it, and the cursor moved down to it.
fn at_the_league_row(gb: &mut GameBoy, game: &mut Game) {
    game.push(Mode::PcMenu(PcMenu::new()));
    cartridge_until_polling(gb);
    recreation_until_polling(game, Decision::Text);
    poll(gb, game, Joypad::A, Decision::CursorMenu, DELAY3 + CURSOR, "the PC's menu");
    for row in ["the player's PC", "PROF.OAK's PC", "<PKMN>LEAGUE"] {
        poll(gb, game, Joypad::DOWN, Decision::CursorMenu, CURSOR, row);
    }
}

/// A press into the next Hall of Fame screen, and the screens the same where it settles. The
/// cartridge's frames from `LoadFrontSpriteByMonIndex` to `PlayCry` are the picture's decompression
/// and its copy into VRAM: loading, and measured rather than named, since how long it takes is the
/// picture's own.
fn poll_mon(gb: &mut GameBoy, game: &mut Game, button: Joypad, what: &str) {
    gb.hold_buttons(joypad(button));
    game.frame(Input::Buttons(button));
    to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    let sprite = breakpoint(sym::LoadFrontSpriteByMonIndex);
    let cry = breakpoint(sym::PlayCry);
    let poll = breakpoint(sym::JoypadLowSensitivity);
    let vblank = breakpoint(sym::VBlank);
    let (mut frames, mut picture, mut decompressing) = (0, false, 0);
    let cartridge = loop {
        let (stop, _) = gb.run_until(&[sprite, cry, poll, vblank], MachineCycles::PER_FRAME * 2);
        if stop == Stop::Breakpoint(sprite) {
            picture = true;
        } else if stop == Stop::Breakpoint(cry) {
            picture = false;
        } else if stop == Stop::Breakpoint(poll) {
            to_vblank(gb);
            break frames + 1;
        } else {
            assert_eq!(stop, Stop::Breakpoint(vblank), "{what}");
            frames += 1;
            decompressing += u32::from(picture);
            assert!(frames < 600, "{what}: the cartridge never polled");
        }
    };
    let recreation = recreation_until_polling(game, Decision::Text);
    assert_eq!(screen(gb), (0..18).map(|y| game.ui().row(y).to_vec()).collect::<Vec<_>>(), "{what}");
    assert_late(cartridge, recreation, HOF_MON_SCREEN + decompressing, what);
}

/// The two paragraphs of `AccessedHoFPCText`, the last press opening the first mon's screen.
fn into_the_league_pc(gb: &mut GameBoy, game: &mut Game) {
    poll(gb, game, Joypad::A, Decision::Text, BOX + ARROW, "accessed the league's site");
    poll(gb, game, Joypad::A, Decision::Text, ARROW, "accessed the hall of fame list");
    poll_mon(gb, game, Joypad::A, "the first mon");
}

#[test]
fn the_league_pc_shows_every_hall_of_fame_team_as_the_cartridge_does() {
    let (mut gb, recorded) = with_a_hall_of_fame();
    let mut game = the_game_with(&gb, recorded.clone());
    at_the_league_row(&mut gb, &mut game);
    into_the_league_pc(&mut gb, &mut game);

    let mons: Vec<(usize, usize)> = recorded.iter().enumerate()
        .flat_map(|(team, mons)| (0..mons.len()).map(move |mon| (team, mon))).collect();
    for &(team, mon) in &mons[1..] {
        poll_mon(&mut gb, &mut game, Joypad::A, &format!("team {team} mon {mon}"));
    }
    // `ReloadMapData` turns the LCD off, so the way back to the PC's menu is untimed.
    untimed(&mut gb, &mut game, Joypad::A, Decision::CursorMenu);
    same_where_drawn(&gb, &game, "the PC's menu over the map");
}

#[test]
fn b_leaves_the_league_pc_as_the_cartridge_does() {
    let (mut gb, recorded) = with_a_hall_of_fame();
    assert!(recorded[0].len() > 1, "the first team has a mon to stop before");
    let mut game = the_game_with(&gb, recorded);
    at_the_league_row(&mut gb, &mut game);
    into_the_league_pc(&mut gb, &mut game);
    untimed(&mut gb, &mut game, Joypad::B, Decision::CursorMenu);
    same_where_drawn(&gb, &game, "the PC's menu over the map");
}


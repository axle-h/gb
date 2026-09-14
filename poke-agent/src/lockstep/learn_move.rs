//! `LearnMove`, which no committed fixture stands in front of. The Celadon fixture is stopped where
//! START calls `DisplayStartMenu`, the first party mon is given four moves one of which is an HM,
//! and the call is turned into a call of `LearnMove` with `wWhichPokemon`, `wMoveNum` and
//! `wStringBuffer` set as `LearnMoveFromLevelUp` sets them. The recreation starts from the same
//! party over the same tile map, and both are compared at every point either waits for a button.

use gb::game_boy::GameBoy;
use gb::ram::{RAM, ROM};
use poke_core::move_name::PokemonMoveName;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::learn_move::LearnMove;
use pokered::rng::GameRng;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols, DmgBank, DmgPointer};
use super::{assert_late, hurried_letter, open_the_start_menu, ARROW, BOX, CURSOR, DELAY3};
use super::status_screen::{cartridge_until_polling, ours, press, screen, the_world};

const MBC_ROM_BANK: u16 = 0x2000;
const TERMINATOR: u8 = 0x50;
const MON_MOVES: u16 = 8;
const MON_PP: u16 = 29;

/// Turns the call the cartridge is stopped at the top of into a call of `routine`: the return
/// address on the stack is the caller's either way.
pub(super) fn hijack(gb: &mut GameBoy, routine: DmgPointer) {
    let DmgBank::ROM { bank } = routine.bank else { panic!("{routine} is not code") };
    let mmu = gb.core_mut().mmu_mut();
    if routine.address >= 0x4000 {
        mmu.write(MBC_ROM_BANK, bank);
        mmu.write(pokered_symbols::hLoadedROMBank.address, bank);
    }
    gb.core_mut().registers_mut().pc = routine.address;
}

/// The cartridge's tile map, which the recreation draws over and `LearnMove` saves and puts back.
pub(super) fn with_the_tile_map(gb: &GameBoy, game: &mut Game) {
    let rows = screen(gb);
    for (y, row) in rows.iter().enumerate() {
        for (x, &tile) in row.iter().enumerate() {
            game.screen_mut().ui.set(x, y, tile);
        }
    }
}

/// Frames until the recreation waits for anything, and what.
pub(super) fn recreation_until_waiting(game: &mut Game) -> (u32, Decision) {
    for frames in 1..3000 {
        game.frame(Input::None);
        if let Status::Waiting(decision) = game.status() {
            return (frames, decision);
        }
    }
    panic!("the recreation never waited");
}

fn start(learning: PokemonMoveName, moves: [PokemonMoveName; 4]) -> (GameBoy, Game) {
    let mut gb = open_the_start_menu();
    let party_mon = pokered_symbols::wPartyMon1.address;
    let mmu = gb.core_mut().mmu_mut();
    for (i, known) in moves.into_iter().enumerate() {
        mmu.write(party_mon + MON_MOVES + i as u16, known as u8);
        mmu.write(party_mon + MON_PP + i as u16, 0x45);
    }
    mmu.write(pokered_symbols::wWhichPokemon.address, 0);
    mmu.write(pokered_symbols::wMoveNum.address, learning as u8);
    let mut name = learning.name();
    name.push(TERMINATOR);
    mmu.write_slice(pokered_symbols::wStringBuffer.address, &name);
    hijack(&mut gb, pokered_symbols::LearnMove);

    let mut game = Game::new(the_world(&gb), GameRng::seeded(0), Pacing::Faithful);
    with_the_tile_map(&gb, &mut game);
    game.push(Mode::LearnMove(LearnMove::new(0, learning)));
    (gb, game)
}

/// Both run to their next wait; the screens must match and the cartridge be late by the `loading` the
/// recreation leaves out on the way.
fn compare(gb: &mut GameBoy, game: &mut Game, expected: Decision, loading: u32, what: &str) {
    let cartridge = cartridge_until_polling(gb);
    let (recreation, decision) = recreation_until_waiting(game);
    assert_eq!(decision, expected, "{what}");
    assert_eq!(screen(gb), ours(game), "{what}");
    println!("{what}: cartridge {cartridge} frames, recreation {recreation}");
    assert_late(cartridge, recreation, loading, what);
}

/// Compares and answers every text box until something else waits, and compares that too. The first
/// wait is late by `first`; each after it is a `▼`, or the yes/no a `▼` leads to, and late by a
/// `Delay3`.
fn through_text(gb: &mut GameBoy, game: &mut Game, expected: Decision, first: u32, what: &str) {
    for page in 0..10 {
        let cartridge = cartridge_until_polling(gb);
        let (recreation, decision) = recreation_until_waiting(game);
        assert_eq!(screen(gb), ours(game), "{what}, wait {page}");
        println!("{what}, wait {page}: cartridge {cartridge} frames, recreation {recreation}");
        let loading = if page == 0 { first } else { DELAY3 };
        assert_late(cartridge, recreation, loading, &format!("{what}, wait {page}"));
        if decision == expected {
            return;
        }
        assert_eq!(decision, Decision::Text, "{what}, wait {page}");
        press(gb, game, Joypad::A);
    }
    panic!("{what}: never reached {expected:?}");
}

fn moves_of(gb: &GameBoy) -> Vec<u8> {
    let at = pokered_symbols::wPartyMon1.address + MON_MOVES;
    (0..4).map(|i| gb.core().mmu().read(at + i)).chain((0..4).map(|i| gb.core().mmu().read(at + 21 + i))).collect()
}

#[test]
fn learn_move_asks_refuses_an_hm_and_forgets_as_the_cartridge_does() {
    use PokemonMoveName::*;
    let (mut gb, mut game) = start(BodySlam, [Tackle, Cut, Growl, TailWhip]);
    let hurry = hurried_letter(&game);

    // `TryingToLearnText`: three paragraphs of two lines each, then the question.
    through_text(&mut gb, &mut game, Decision::TwoOption, BOX + ARROW, "delete an older move?");

    // YES, then CUT, which is refused, and the list again.
    press(&mut gb, &mut game, Joypad::A);
    compare(&mut gb, &mut game, Decision::ForgetMove, BOX + CURSOR, "which move");
    press(&mut gb, &mut game, Joypad::DOWN);
    compare(&mut gb, &mut game, Decision::ForgetMove, CURSOR, "down to CUT");
    press(&mut gb, &mut game, Joypad::A);
    compare(&mut gb, &mut game, Decision::Text, BOX + ARROW + hurry, "HM techniques can't be deleted");
    press(&mut gb, &mut game, Joypad::A);
    compare(&mut gb, &mut game, Decision::ForgetMove, BOX + CURSOR + hurry, "which move, again");

    // B, NO to abandoning, and the whole question once more.
    press(&mut gb, &mut game, Joypad::B);
    through_text(&mut gb, &mut game, Decision::TwoOption, BOX + CURSOR + hurry, "abandon learning?");
    press(&mut gb, &mut game, Joypad::B);
    through_text(&mut gb, &mut game, Decision::TwoOption, BOX + ARROW, "delete, asked again");
    press(&mut gb, &mut game, Joypad::A);
    compare(&mut gb, &mut game, Decision::ForgetMove, BOX + CURSOR, "the list a third time");

    // TACKLE goes: `1, 2 and... Poof!`, then what was forgotten, then what was learned.
    press(&mut gb, &mut game, Joypad::A);
    compare(&mut gb, &mut game, Decision::Text, BOX + ARROW + hurry, "forgot TACKLE");
    press(&mut gb, &mut game, Joypad::A);
    compare(&mut gb, &mut game, Decision::Text, ARROW, "forgot TACKLE, second line");
    press(&mut gb, &mut game, Joypad::A);
    compare(&mut gb, &mut game, Decision::Text, BOX + hurry, "learned BODY SLAM");

    let party = &game.world().party[0].mon.mon;
    let ours: Vec<u8> = party.moves.iter().map(|known| known.map_or(0, |known| known as u8))
        .chain(party.pp).collect();
    assert_eq!(moves_of(&gb), ours, "the moves and PP afterwards");
}

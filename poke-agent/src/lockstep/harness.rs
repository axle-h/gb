//! What a lockstep started from a cartridge standing in the overworld shares: the edits made to the
//! cartridge before both start, the recreation's start taken from it, and the recreation's side of a
//! talk and of the screen.

use gb::game_boy::GameBoy;
use gb::ram::{RAM, ROM};
use pokered::mode::Mode;
use pokered::modes::overworld::Overworld;
use pokered::rng::GameRng;
use pokered::input::Joypad;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointerRead};

const PARTY_STRUCT: u16 = 0x2C;
/// `BIT_NO_BATTLES`, in `wStatusFlags4`.
pub(super) const BIT_NO_BATTLES: u8 = 4;
/// The four sound effect channels.
const SFX_CHANNELS: [usize; 4] = [4, 5, 6, 7];

fn write(gb: &mut GameBoy, at: u16, value: u8) {
    gb.core_mut().mmu_mut().write(at, value);
}

/// The party back to full HP and no status, since a fixture saved after a gym leader's battle is in
/// no state to fight it again.
fn heal(gb: &mut GameBoy) {
    for i in 0..gb.core().mmu().read(sym::wPartyCount.address) as u16 {
        let mon = PARTY_STRUCT * i;
        for byte in 0..2 {
            let max = gb.core().mmu().read(sym::wPartyMon1MaxHP.address + mon + byte);
            write(gb, sym::wPartyMon1HP.address + mon + byte, max);
        }
        write(gb, sym::wPartyMon1Status.address + mon, 0);
    }
}

/// The last thing in a full bag thrown away, since a gym leader's TM needs a slot to go in.
pub(super) fn make_room(gb: &mut GameBoy) {
    /// `MAX_ITEMS`.
    const BAG_SIZE: u8 = 20;
    let count = gb.core().mmu().read(sym::wNumBagItems.address);
    if count == BAG_SIZE {
        write(gb, sym::wNumBagItems.address, count - 1);
        write(gb, sym::wBagItems.address + 2 * (count as u16 - 1), 0xFF);
    }
}

pub(super) fn clear_events(gb: &mut GameBoy, events: &[u16]) {
    for &event in events {
        let at = sym::wEventFlags.address + event / 8;
        let flags = gb.core().mmu().read(at);
        write(gb, at, flags & !(1 << (event % 8)));
    }
}

pub(super) fn clear_badge(gb: &mut GameBoy, bit: u8) {
    let badges = gb.core().mmu().read(sym::wObtainedBadges.address);
    write(gb, sym::wObtainedBadges.address, badges & !(1 << bit));
}

/// Clears `events` and the badge of `bit`, so a fixture standing in front of a beaten gym leader has
/// him to beat again, and heals the party it beat him with.
pub(super) fn forget_badge(gb: &mut GameBoy, events: &[u16], bit: u8) {
    heal(gb);
    make_room(gb);
    clear_events(gb, events);
    clear_badge(gb, bit);
}

/// What the recreation starts from, taken where the cartridge has just polled in the overworld.
pub(super) struct Start {
    world: pokered::world::World,
    overworld: Overworld,
    /// Where the start menu, the party menu and the bag reopen: `wBattleAndStartSavedMenuItem`,
    /// `wPartyAndBillsPCSavedMenuItem`, `wBagSavedMenuItem` and `wListScrollOffset`.
    menus: [u8; 4],
    sfx_note_delays: [u8; 4],
}

impl Start {
    pub(super) fn take(gb: &GameBoy) -> Self {
        let mmu = gb.core().mmu();
        let overworld = Overworld::standing(super::bridge::sprites(gb), mmu.read_pointer(&sym::wNumSprites),
            super::bridge::standing(gb))
            .with_battle_flags(
                mmu.read_pointer(&sym::wStatusFlags4) & 1 << BIT_NO_BATTLES != 0,
                mmu.read_pointer(&sym::wStatusFlags2) & 1 != 0,
                mmu.read_pointer(&sym::wNumberOfNoRandomBattleStepsLeft),
            )
            .with_step_counter(mmu.read_pointer(&sym::wStepCounter));
        let menus = [&sym::wBattleAndStartSavedMenuItem, &sym::wPartyAndBillsPCSavedMenuItem, &sym::wBagSavedMenuItem,
            &sym::wListScrollOffset].map(|at| mmu.read_pointer(at));
        let sfx_note_delays = SFX_CHANNELS.map(|c| mmu.read(sym::wChannelNoteDelayCounters.address + c as u16));
        Self { world: super::bridge::world(gb), overworld, menus, sfx_note_delays }
    }

    pub(super) fn game(&self, tape: Vec<u8>) -> Game {
        let mut game = Game::new(self.world.clone(), GameRng::tape(tape), Pacing::Faithful);
        let menu = game.menu_mut();
        [menu.battle_and_start, menu.party_and_bills, menu.bag_saved, menu.list_scroll] = self.menus;
        seed_sfx_note_delays(&mut game, self.sfx_note_delays);
        game.push(Mode::Overworld(self.overworld.clone()));
        game
    }
}

/// The idle sound effect channels' note delay counters, which a new engine starts at zero where the
/// cartridge's are wherever its last sound left them. A cry parks channel 7 on a `sound_ret` that a
/// zero counter reaches only 255 frames on, and a sound effect wanting the channel before then is
/// dropped.
fn seed_sfx_note_delays(game: &mut Game, counters: [u8; 4]) {
    let mut engine = serde_json::to_value(game.audio()).expect("the engine serialises");
    for (c, counter) in SFX_CHANNELS.into_iter().zip(counters) {
        if game.audio().channel_sound_id(c) == 0 {
            engine["channels"][c]["note_delay_counter"] = counter.into();
        }
    }
    *game.audio_mut() = serde_json::from_value(engine).expect("the engine deserialises");
}

pub(super) fn overworld(game: &Game) -> Option<&Overworld> {
    game.modes().iter().rev().find_map(|mode| match mode {
        Mode::Overworld(overworld) => Some(overworld),
        _ => None,
    })
}

/// The recreation's screen: what the UI covers, and the map through the view where it does not.
pub(super) fn recreated_screen(game: &Game) -> Vec<Vec<u8>> {
    let in_battle = game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_)));
    let map = overworld(game).filter(|_| !in_battle).map(|overworld| overworld.view().tile_map());
    (0..18).map(|y| (0..20).map(|x| {
        game.ui().cover(x, y).or_else(|| map.map(|tiles| tiles[y * 20 + x])).unwrap_or(game.ui().get(x, y))
    }).collect()).collect()
}

/// A held until something opens over the overworld, and the frames it took.
pub(super) fn recreation_talk(game: &mut Game, budget: u32) -> u32 {
    for frames in 1..=budget {
        game.frame(Input::Buttons(Joypad::A));
        if game.modes().len() > 1 {
            return frames;
        }
    }
    panic!("the recreation never talked");
}

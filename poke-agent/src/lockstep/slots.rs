//! The Game Corner's slot machine: the cartridge walked from `postgame-coins.bin` at the Game Corner's
//! door to a machine and into `MainSlotMachineLoop`, and the recreation's `SlotMachine` from the same
//! world there, fed the same presses and the cartridge's random bytes.
//!
//! The cartridge runs first and alone, taping every `Random` byte that is not VBlank's own and
//! recording what it shows at each poll; the recreation then replays the presses. While the wheels
//! turn the cartridge polls every step and the recreation is `Busy`, so a press there is held until
//! a poll has read it, and a poll in the recreation is the frame that reads the pad.

use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::ROM;
use poke_core::map::Map;
use poke_core::sprite::SpriteFacing;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::slots::SlotMachine;
use pokered::rng::GameRng;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::pokered_symbols as sym;
use super::battle::{cartridge_oam, letters};
use super::scripts::{Action, Cartridge as Walker, Kind};
use super::status_screen::{ours, screen};
use super::{breakpoint, joypad};

/// `wShadowOAMSprite00` to `Sprite35`: the three wheels' objects. The four after them are the
/// overworld's, which the recreation does not draw here.
const WHEEL_OBJECTS: usize = 36;
/// `rBGP` and `rOBP0`, which a win's flashes invert.
const R_BGP: u16 = 0xFF47;
const R_OBP0: u16 = 0xFF48;

/// What the cartridge showed at a poll.
#[derive(Debug)]
struct Poll {
    frames: u32,
    /// A poll of `SlotMachine_HandleInputWhileWheelsSpin` rather than a text's or a menu's.
    spinning: bool,
    screen: Vec<Vec<u8>>,
    objects: Vec<[u8; 4]>,
    coins: [u8; 2],
    palettes: (u8, u8),
}

/// The cartridge inside `MainSlotMachineLoop`, taping its random bytes.
struct Cabinet {
    gb: GameBoy,
    tape: Vec<u8>,
    /// Where `MainSlotMachineLoop` returns to, and the stack pointer it returns with.
    back: (Breakpoint, u16),
    ended: bool,
    spinning: bool,
}

impl Cabinet {
    /// Runs to the VBlank after the next poll, counting frames; `None` once the loop has returned.
    /// With `settled`, a wheel's poll after which the wheels come to rest before that VBlank is run
    /// past: the frame ends inside the texts and rolls that follow, where the recreation has already
    /// done them.
    fn run(&mut self, settled: bool) -> Option<u32> {
        let (poll, vblank, random, end) = (breakpoint(sym::JoypadLowSensitivity), breakpoint(sym::VBlank),
            breakpoint(sym::Random), self.back.0);
        let rested = breakpoint(sym::SlotMachine_CheckForMatches);
        let spin = sym::SlotMachine_HandleInputWhileWheelsSpin.address;
        let mut polled = false;
        let mut frames = 0;
        loop {
            let (stop, _) = self.gb.run_until(&[poll, vblank, random, end, rested], MachineCycles::PER_FRAME * 600);
            let Stop::Breakpoint(hit) = stop else { panic!("the cartridge stopped: {stop:?}") };
            if hit == rested {
                polled &= !(settled && self.spinning);
            } else if hit == poll {
                polled = true;
                self.spinning = (spin..spin + 0x10).contains(&self.gb.return_address());
            } else if hit == vblank {
                frames += 1;
                if polled {
                    return Some(frames);
                }
            } else if hit == random {
                let caller = self.gb.return_address();
                let (stop, _) = self.gb.run_to_return(MachineCycles::PER_FRAME * 10);
                assert!(matches!(stop, Stop::Returned { .. }));
                let vblank = sym::VBlank.address;
                if !(vblank..vblank + 0x80).contains(&caller) {
                    self.tape.push(self.gb.core().registers().a);
                }
            } else if hit == end && self.gb.core().registers().sp == self.back.1 {
                self.ended = true;
                return None;
            }
            assert!(frames < 5000, "the cartridge never polled");
        }
    }

    fn to_poll(&mut self) -> Option<Poll> {
        if self.ended {
            return None;
        }
        let frames = self.run(true)?;
        let mmu = self.gb.core().mmu();
        Some(Poll {
            frames,
            spinning: self.spinning,
            screen: screen(&self.gb),
            objects: cartridge_oam(&self.gb)[..WHEEL_OBJECTS].to_vec(),
            coins: [mmu.read(sym::wPlayerCoins.address), mmu.read(sym::wPlayerCoins.address + 1)],
            palettes: (mmu.read(R_BGP), mmu.read(R_OBP0)),
        })
    }

    /// `button` held until a poll has read it, or the machine has let go on it. A wheel's poll comes
    /// every few frames, a menu's every frame.
    fn press(&mut self, button: Joypad) {
        self.gb.hold_buttons(joypad(button));
        self.run(false);
        self.gb.hold_buttons(JoypadButtonState::default());
    }
}

/// `postgame-coins.bin` walked through the Game Corner's door to the machine at (18, 15), talked to
/// and answered YES: the cartridge at `MainSlotMachineLoop`'s first instruction, and the world the
/// recreation starts from there.
fn at_the_machine() -> (Cabinet, pokered::world::World, u8) {
    let mut walker = Walker::from_state(include_bytes!("../pokemon/data/postgame-coins.bin"));
    let mut kind = walker.to_poll().0;
    let route = [Action::Walk(Joypad::UP), Action::Walk(Joypad::UP), Action::Walk(Joypad::UP),
        Action::Walk(Joypad::RIGHT), Action::Walk(Joypad::RIGHT), Action::Press(Joypad::RIGHT, 2)];
    for action in route {
        while kind != Kind::Overworld {
            kind = walker.to_poll().0;
        }
        kind = walker.act(action).0;
    }
    while kind != Kind::Overworld {
        kind = walker.to_poll().0;
    }
    let location = walker.world().location;
    assert_eq!((location.map, location.x, location.y, location.facing), (Map::GameCorner, 17, 15, SpriteFacing::Right),
        "beside the machine");
    assert_ne!(walker.world().bag.quantity_of(poke_core::item::ItemId::CoinCase), 0, "a Coin Case");

    let gb = &mut walker.gb;
    gb.hold_buttons(joypad(Joypad::A));
    let prompt = breakpoint(sym::PromptUserToPlaySlots);
    let (stop, _) = gb.run_until(&[prompt], MachineCycles::PER_FRAME * 120);
    assert_eq!(stop, Stop::Breakpoint(prompt), "A never asked to play");
    gb.hold_buttons(JoypadButtonState::default());
    super::cartridge_until_polling(gb);
    gb.hold_buttons(joypad(Joypad::A));
    super::to_vblank(gb);
    gb.hold_buttons(JoypadButtonState::default());
    let main_loop = breakpoint(sym::MainSlotMachineLoop);
    let (stop, _) = gb.run_until(&[main_loop], MachineCycles::PER_FRAME * 600);
    assert_eq!(stop, Stop::Breakpoint(main_loop), "YES never started the machine");

    let (sp, address) = (gb.core().registers().sp, gb.return_address());
    let bank = if (0x4000..0x8000).contains(&address) { gb.core().mmu().rom_bank() as u8 } else { 0 };
    let chance = gb.core().mmu().read(sym::wSlotMachineSevenAndBarModeChance.address);
    let world = walker.world();
    let cabinet = Cabinet { gb: walker.gb, tape: vec![], back: (Breakpoint::new(bank, address), sp + 2), ended: false, spinning: false };
    (cabinet, world, chance)
}

fn machine(game: &Game) -> Option<&SlotMachine> {
    game.modes().iter().find_map(|mode| match mode {
        Mode::SlotMachine(machine) => Some(machine),
        _ => None,
    })
}

/// Whether the recreation's next frame reads the pad.
fn reads_the_pad(game: &Game) -> bool {
    match game.modes().last() {
        Some(Mode::SlotMachine(machine)) if machine.reading_the_pad() => true,
        _ => matches!(game.status(), Status::Waiting(_)),
    }
}

/// Frames until the recreation polls: a menu or a text waiting, or a wheel's step that has read the
/// pad and left the wheels turning. `None` once the machine has popped.
fn recreation_to_poll(game: &mut Game) -> Option<(u32, bool)> {
    for frames in 1..5000 {
        let reading = matches!(game.modes().last(), Some(Mode::SlotMachine(machine)) if machine.reading_the_pad());
        game.frame(Input::None);
        let spinning = machine(game)?.stopping_wheels() && reading;
        if spinning {
            return Some((frames, true));
        }
        let waiting = match game.modes().last() {
            Some(Mode::SlotMachine(machine)) => machine.reading_the_pad() && !machine.stopping_wheels(),
            _ => matches!(game.status(), Status::Waiting(_)),
        };
        if waiting {
            return Some((frames, false));
        }
    }
    panic!("the recreation never polled");
}

fn recreation_press(game: &mut Game, button: Joypad) {
    for _ in 0..10 {
        let reads = reads_the_pad(game);
        game.frame(Input::Buttons(button));
        if reads {
            return;
        }
    }
    panic!("the recreation never read {button:?}");
}

/// A press for what the cartridge shows: A to every text, the ×3 bet and every other step of the
/// wheels, and at the offer of another go YES until `spins` are played and one has won.
fn choose(poll: &Poll, spins: usize, won: bool, wanted: usize) -> Joypad {
    let text: String = poll.screen.iter().map(|row| letters(row)).collect::<Vec<_>>().join("/");
    if text.contains("One more") && text.contains("YES") {
        let on_yes = poll.screen[13][15] == 0xED;
        let done = spins >= wanted && won;
        return match (done, on_yes) {
            (true, true) => Joypad::DOWN,
            _ => Joypad::A,
        };
    }
    Joypad::A
}

/// Spins until one wins, at least `wanted` of them, compared at every poll: the screen, the wheels'
/// objects, the palettes and the coins.
fn play(wanted: usize) {
    let (mut cabinet, world, chance) = at_the_machine();
    let before = world.coins;
    let (mut polls, mut presses) = (vec![], vec![]);
    let (mut spins, mut won) = (0, false);
    while let Some(poll) = cabinet.to_poll() {
        let text: String = poll.screen.iter().map(|row| letters(row)).collect::<Vec<_>>().join("/");
        if poll.spinning && !polls.last().is_some_and(|last: &Poll| last.spinning) {
            spins += 1;
        }
        won |= text.contains("lined up");
        let button = choose(&poll, spins, won, wanted);
        if std::env::var("SHOW_CARTRIDGE").is_ok() {
            println!("cartridge poll {} after {} frames{}, pressing {button:?}", polls.len(), poll.frames,
                if poll.spinning { " spinning" } else { "" });
            for row in &poll.screen {
                println!("  |{}|", letters(row));
            }
        }
        cabinet.press(button);
        polls.push(poll);
        presses.push(button);
        assert!(presses.len() < 2000, "the machine goes on");
    }
    assert!(spins >= wanted, "{spins} spins");
    assert!(won, "no spin won");

    let mut game = Game::new(world, GameRng::tape(std::mem::take(&mut cabinet.tape)), Pacing::Faithful);
    game.push(Mode::SlotMachine(SlotMachine::new(chance)));
    for (i, (theirs, &button)) in polls.iter().zip(&presses).enumerate() {
        let (frames, spinning) = recreation_to_poll(&mut game).unwrap_or_else(|| panic!("poll {i}: the recreation let go"));
        println!("poll {i}: cartridge {} frames, recreation {frames}{}", theirs.frames, if spinning { " spinning" } else { "" });
        let mine = ours(&game);
        if theirs.screen != mine {
            for (a, b) in theirs.screen.iter().zip(&mine) {
                println!("  |{}|  |{}|", letters(a), letters(b));
            }
        }
        assert_eq!(theirs.screen, mine, "the screen at poll {i}");
        assert_eq!(spinning, theirs.spinning, "poll {i}: what is polled");
        let objects: Vec<[u8; 4]> = game.screen().sprites[..WHEEL_OBJECTS].iter()
            .map(|object| [object.y, object.x, object.tile, object.attributes]).collect();
        assert_eq!(theirs.objects, objects, "the wheels at poll {i}");
        let effects = &game.screen().effects;
        assert_eq!(theirs.palettes, (effects.bgp, effects.obp0), "the palettes at poll {i}");
        assert_eq!(theirs.coins, game.world().coins, "the coins at poll {i}");
        recreation_press(&mut game, button);
    }
    assert!(recreation_to_poll(&mut game).is_none(), "the recreation's machine lets go");
    let coins = [cabinet.gb.core().mmu().read(sym::wPlayerCoins.address), cabinet.gb.core().mmu().read(sym::wPlayerCoins.address + 1)];
    assert_eq!(coins, game.world().coins, "the coins afterwards");
    assert_ne!(coins, before, "the coins moved");
}

#[test]
fn spins_of_the_slot_machine_and_a_win_match_the_cartridge() {
    play(3);
}

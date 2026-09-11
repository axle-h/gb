//! Running to a routine and back out of it, for the oracle's harvester. This is a loop of its own
//! beside [`GameBoy::run`], so a breakpoint check never sits on the ordinary run's hot path.

use crate::core::CoreMode;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::joypad::{JoypadButton, JoypadButtonState, JoypadTape};
use crate::ram::ROM;
use strum::IntoEnumIterator;

/// An instruction address, and for `0x4000..=0x7FFF` the ROM bank that must be mapped there.
/// The bank is ignored everywhere else, so code copied to HRAM or WRAM breaks on its address alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Breakpoint {
    pub bank: u8,
    pub address: u16,
}

impl Breakpoint {
    pub const fn new(bank: u8, address: u16) -> Self {
        Self { bank, address }
    }

    fn matches(&self, pc: u16, rom_bank: usize) -> bool {
        pc == self.address && (!(0x4000..0x8000).contains(&pc) || rom_bank == self.bank as usize)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The next instruction to execute is at this breakpoint.
    Breakpoint(Breakpoint),
    /// The stack has unwound past the routine's entry: `pc` is where it went, normally the caller.
    Returned { pc: u16 },
    Budget,
    Crashed,
}

impl GameBoy {
    /// One instruction, or one span of HALT, which never moves the PC.
    fn step(&mut self, budget: MachineCycles) -> MachineCycles {
        if self.core.mode() == CoreMode::Halt {
            self.core.skip_halt(budget)
        } else {
            let opcode = self.core.fetch();
            self.core.execute(opcode)
        }
    }

    /// Run until the PC reaches `(bank, address)` or `budget` is spent. At least one instruction
    /// runs first, so calling this again from a hit finds the next one.
    pub fn run_until_pc(&mut self, bank: u8, address: u16, budget: MachineCycles) -> (Stop, MachineCycles) {
        self.run_until(&[Breakpoint::new(bank, address)], budget)
    }

    /// [`GameBoy::run_until_pc`] for any of several breakpoints; the stop names the one hit.
    pub fn run_until(&mut self, breakpoints: &[Breakpoint], budget: MachineCycles) -> (Stop, MachineCycles) {
        let mut cycles = MachineCycles::ZERO;
        while cycles < budget {
            cycles += self.step(budget - cycles);
            if self.core.mode() == CoreMode::Crash {
                return (Stop::Crashed, cycles);
            }
            let pc = self.core.registers().pc;
            let rom_bank = self.core.mmu().rom_bank();
            if let Some(hit) = breakpoints.iter().find(|b| b.matches(pc, rom_bank)) {
                return (Stop::Breakpoint(*hit), cycles);
            }
        }
        (Stop::Budget, cycles)
    }

    /// Called on a routine's first instruction, where `[SP]` is the return address: run until SP
    /// rises above its value now. An interrupt inside the routine pushes below it, so only the
    /// routine's own `ret`, or a `pop` of its return address, stops this.
    pub fn run_to_return(&mut self, budget: MachineCycles) -> (Stop, MachineCycles) {
        let entry_sp = self.core.registers().sp;
        let mut cycles = MachineCycles::ZERO;
        while cycles < budget {
            cycles += self.step(budget - cycles);
            if self.core.mode() == CoreMode::Crash {
                return (Stop::Crashed, cycles);
            }
            let registers = self.core.registers();
            if registers.sp > entry_sp {
                return (Stop::Returned { pc: registers.pc }, cycles);
            }
        }
        (Stop::Budget, cycles)
    }

    /// The return address a routine was called with, read on its first instruction.
    pub fn return_address(&self) -> u16 {
        self.core.mmu().read_u16_le(self.core.registers().sp)
    }

    /// Hold exactly `buttons`. A press raises the joypad interrupt as a real one would.
    pub fn hold_buttons(&mut self, buttons: JoypadButtonState) {
        let joypad = self.core.mmu_mut().joypad_mut();
        for button in JoypadButton::iter() {
            joypad.update_button(button, buttons.is_button_pressed(button));
        }
    }

    /// Frames `from..to` of `tape`, each holding the tape's buttons for that frame. A frame ends on
    /// the next multiple of [`MachineCycles::PER_FRAME`] on the machine's own clock, so a tape
    /// played in pieces, or across a save state, is the same run as one played whole.
    pub fn play_tape(&mut self, tape: &JoypadTape, from: u64, to: u64) -> MachineCycles {
        let per_frame = MachineCycles::PER_FRAME.m_cycles();
        let mut cycles = MachineCycles::ZERO;
        for frame in from..to {
            self.hold_buttons(tape.buttons_at(frame));
            let now = self.core.mmu().now();
            let boundary = (now / per_frame + 1) * per_frame;
            cycles += self.run(MachineCycles::from_m(boundary - now));
        }
        cycles
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{AT_CELADON, POKERED};

    /// `pokered.sym`, copied rather than generated, as `test_fixtures` does.
    const DELAY_FRAME: Breakpoint = Breakpoint::new(0x00, 0x20af);
    const RANDOM: Breakpoint = Breakpoint::new(0x00, 0x3e5c);
    const JOYPAD_BANKED: Breakpoint = Breakpoint::new(0x03, 0x4000);
    const SECOND: MachineCycles = MachineCycles::from_m(1_048_576);

    fn celadon() -> GameBoy {
        let mut gb = GameBoy::dmg(POKERED);
        gb.load_state(AT_CELADON).expect("load fixture");
        gb
    }

    #[test]
    fn a_home_bank_breakpoint_stops_on_its_address() {
        let mut gb = celadon();
        let (stop, cycles) = gb.run_until_pc(DELAY_FRAME.bank, DELAY_FRAME.address, SECOND);
        assert_eq!(stop, Stop::Breakpoint(DELAY_FRAME));
        assert_eq!(gb.core().registers().pc, DELAY_FRAME.address);
        assert!(cycles < MachineCycles::PER_FRAME * 2, "DelayFrame runs every frame");

        let (again, _) = gb.run_until_pc(DELAY_FRAME.bank, DELAY_FRAME.address, SECOND);
        assert_eq!(again, Stop::Breakpoint(DELAY_FRAME), "a second call finds the next hit");
    }

    #[test]
    fn a_banked_breakpoint_stops_only_with_its_bank_mapped() {
        let mut gb = celadon();
        let (stop, _) = gb.run_until(&[JOYPAD_BANKED], SECOND);
        assert_eq!(stop, Stop::Breakpoint(JOYPAD_BANKED));
        assert_eq!(gb.core().mmu().rom_bank(), 3);

        assert!(!JOYPAD_BANKED.matches(0x4000, 4));
        assert!(DELAY_FRAME.matches(0x20af, 7), "the home bank ignores the mapped bank");
    }

    #[test]
    fn the_budget_is_reported_when_nothing_is_hit() {
        let mut gb = celadon();
        let (stop, cycles) = gb.run_until_pc(0, 0x0000, MachineCycles::PER_FRAME);
        assert_eq!(stop, Stop::Budget);
        assert!(cycles >= MachineCycles::PER_FRAME);
    }

    #[test]
    fn run_to_return_lands_on_the_callers_return_address() {
        let mut gb = celadon();
        let (stop, _) = gb.run_until(&[RANDOM], SECOND * 5);
        assert_eq!(stop, Stop::Breakpoint(RANDOM), "Celadon's wandering NPCs roll Random");
        let entry_sp = gb.core().registers().sp;
        let caller = gb.return_address();

        let (stop, _) = gb.run_to_return(SECOND);
        assert_eq!(stop, Stop::Returned { pc: caller });
        assert_eq!(gb.core().registers().sp, entry_sp + 2);
    }

    #[test]
    fn the_same_tape_replays_the_same_machine() {
        let mut tape = JoypadTape::default();
        tape.hold(10, JoypadButtonState { down: true, ..Default::default() });
        tape.hold(40, JoypadButtonState::default());
        tape.hold(50, JoypadButtonState { a: true, ..Default::default() });
        tape.hold(52, JoypadButtonState::default());

        let mut first = celadon();
        let mut second = celadon();
        first.play_tape(&tape, 0, 120);
        second.play_tape(&tape, 0, 60);
        second.play_tape(&tape, 60, 120);
        assert!(first == second, "a tape played in two halves diverged from one played whole");
        assert!(first != celadon(), "the tape did nothing");
    }
}

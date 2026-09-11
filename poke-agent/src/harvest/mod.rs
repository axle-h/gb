//! Calls a cartridge routine on the emulator with chosen inputs and records what it returns, as
//! tier-1 fixtures for `pokered`.

mod math;
mod stats;

use gb::core::CoreMode;
use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::ram::{RAM, ROM};
use gb::registers::RegisterSet;
use crate::lockstep::breakpoint;
use crate::pokemon::symbols::{pokered_symbols, DmgBank, DmgPointer};

/// Where a called routine returns to: unusable memory, so nothing else ever executes there.
const TRAP: u16 = 0xFEA0;
const INTERRUPT_ENABLE: u16 = 0xFFFF;
const MBC_ROM_BANK: u16 = 0x2000;

pub struct Oracle {
    gb: GameBoy,
    stack: u16,
}

/// What a call did besides its outputs: the bytes `Random` returned to it, in order.
#[derive(Debug, Default)]
pub struct Called {
    pub rng: Vec<u8>,
}

/// One line of a fixture file.
#[cfg(feature = "slow-tests")]
#[derive(Debug, serde::Serialize)]
pub struct Case<I, O> {
    pub input: I,
    pub output: O,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rng: Vec<u8>,
}

impl Oracle {
    /// The machine from `state`, woken from any HALT and with every interrupt masked, so a call
    /// runs only the routine: no VBlank, and so no `Random` but the routine's own.
    pub fn from_state(state: &[u8]) -> Self {
        let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(state).expect("load the oracle's state");
        gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
        for _ in 0..1_000_000 {
            if gb.core().mode() == CoreMode::Normal {
                break;
            }
            gb.run_until(&[], MachineCycles::from_m(1));
        }
        assert_eq!(gb.core().mode(), CoreMode::Normal, "the CPU never woke");
        gb.core_mut().mmu_mut().write(INTERRUPT_ENABLE, 0);
        let stack = gb.core().registers().sp;
        Self { gb, stack }
    }

    pub fn registers_mut(&mut self) -> &mut RegisterSet {
        self.gb.core_mut().registers_mut()
    }

    pub fn registers(&self) -> &RegisterSet {
        self.gb.core().registers()
    }

    pub fn write(&mut self, at: DmgPointer, bytes: &[u8]) {
        assert!(!matches!(at.bank, DmgBank::ROM { .. }), "{at} is ROM");
        self.gb.core_mut().mmu_mut().write_slice(at.address, bytes);
    }

    pub fn read(&self, at: DmgPointer, len: usize) -> Vec<u8> {
        self.gb.core().mmu().read_slice(at.address, len)
    }

    /// `call routine`, with the registers as set, run until it returns. A banked routine gets its
    /// bank mapped and `hLoadedROMBank` set, as `farcall` would leave them.
    pub fn call(&mut self, routine: DmgPointer) -> Called {
        let mmu = self.gb.core_mut().mmu_mut();
        if let DmgBank::ROM { bank } = routine.bank && routine.address >= 0x4000 {
            mmu.write(MBC_ROM_BANK, bank);
            mmu.write(pokered_symbols::hLoadedROMBank.address, bank);
        }
        let sp = self.stack - 2;
        mmu.write_u16_le(sp, TRAP);
        let registers = self.gb.core_mut().registers_mut();
        registers.sp = sp;
        registers.pc = routine.address;

        let random = breakpoint(pokered_symbols::Random);
        let trap = Breakpoint::new(0, TRAP);
        let mut called = Called::default();
        let budget = MachineCycles::PER_FRAME * 600;
        loop {
            if self.registers().pc == random.address {
                let (stop, _) = self.gb.run_to_return(budget);
                assert!(matches!(stop, Stop::Returned { .. }), "Random did not return: {stop:?}");
                called.rng.push(self.registers().a);
            }
            match self.gb.run_until(&[trap, random], budget).0 {
                Stop::Breakpoint(hit) if hit == trap => return called,
                Stop::Breakpoint(_) => {}
                stop => panic!("{routine} did not return: {stop:?}"),
            }
        }
    }
}

/// Writes `pokered/fixtures/<system>/<routine>.jsonl`, and only under `GB_REGEN_FIXTURES=1`.
#[cfg(feature = "slow-tests")]
pub fn write_fixture<I: serde::Serialize, O: serde::Serialize>(system: &str, routine: &str, cases: &[Case<I, O>]) {
    if std::env::var("GB_REGEN_FIXTURES").as_deref() != Ok("1") {
        println!("GB_REGEN_FIXTURES is not 1; {} cases of {routine} not written", cases.len());
        return;
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pokered/fixtures").join(system);
    std::fs::create_dir_all(&dir).unwrap();
    let lines: String = cases.iter().map(|case| serde_json::to_string(case).unwrap() + "\n").collect();
    std::fs::write(dir.join(format!("{routine}.jsonl")), lines).unwrap();
    println!("wrote {} cases of {routine}", cases.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oracle() -> Oracle {
        Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"))
    }

    /// `call Random` twice then `ret`, assembled into `wBuffer`.
    #[test]
    fn every_byte_random_returns_is_recorded_in_order() {
        let mut oracle = oracle();
        let [low, high] = pokered_symbols::Random.address.to_le_bytes();
        oracle.write(pokered_symbols::wBuffer, &[0xCD, low, high, 0xCD, low, high, 0xC9]);
        let called = oracle.call(pokered_symbols::wBuffer);
        assert_eq!(called.rng.len(), 2);
        assert_eq!(called.rng[1], oracle.registers().a, "the last one is still in a");
        assert_eq!(oracle.read(pokered_symbols::hRandomAdd, 1)[0], called.rng[1]);
    }

    #[test]
    fn a_second_call_starts_from_the_same_stack() {
        let mut oracle = oracle();
        oracle.write(pokered_symbols::wBuffer, &[0xC9]);
        oracle.call(pokered_symbols::wBuffer);
        let sp = oracle.registers().sp;
        oracle.call(pokered_symbols::wBuffer);
        assert_eq!(oracle.registers().sp, sp);
    }
}

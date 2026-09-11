use crate::core::Core;
use crate::header::LoadError;
use crate::cycles::MachineCycles;
use crate::model::Model;
use crate::savestate::{SectionReader, SectionWriter};

mod harness;
pub use harness::{Breakpoint, Stop};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GameBoy {
    core: Core
}

impl GameBoy {
    /// A Game Boy (DMG). 89 call sites depend on this behaving exactly as it always has — add
    /// constructors beside it rather than changing it.
    pub fn dmg(cart: &[u8]) -> Self {
        Self::new(cart, Model::Dmg)
    }

    /// A Game Boy Color. A cartridge with CGB header support gets the full colour hardware; a
    /// DMG-only cartridge — Pokémon Red among them — runs in compatibility mode with the boot
    /// ROM's title-derived palette.
    pub fn cgb(cart: &[u8]) -> Self {
        Self::new(cart, Model::Cgb)
    }

    pub fn new(cart: &[u8], model: Model) -> Self {
        Self {
            core: Core::new(cart, model)
        }
    }

    /// The fallible constructor.
    pub fn try_new(cart: &[u8], model: Model) -> Result<Self, LoadError> {
        Ok(Self { core: Core::try_new(cart, model)? })
    }

    /// A Game Boy (DMG) from a cartridge that may not load. See [`GameBoy::try_new`].
    pub fn try_dmg(cart: &[u8]) -> Result<Self, LoadError> {
        Self::try_new(cart, Model::Dmg)
    }

    pub fn dmg_hello_world() -> Self {
        Self::dmg(crate::roms::acid::ROM)
    }

    pub fn core(&self) -> &Core {
        &self.core
    }

    pub fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    pub fn run(&mut self, min_cycles: MachineCycles) -> MachineCycles {
        let mut cycles = MachineCycles::ZERO;
        while cycles < min_cycles {
            if self.core.mode() == crate::core::CoreMode::Halt {
                cycles += self.core.skip_halt(min_cycles - cycles);
            } else {
                let opcode = self.core.fetch();
                cycles += self.core.execute(opcode);
            }
        }
        cycles
    }

    pub fn reset(&mut self) {
        self.core.reset();
    }

    pub fn dump_sram(&self) -> Vec<u8> {
        self.core.mmu().dump_sram()
    }

    pub fn dump_sram_to_file(&self, path: &str) -> Result<(), String> {
        let data = self.dump_sram();
        std::fs::write(path, &data).map_err(|e| e.to_string())
    }

    pub fn restore_sram(&mut self, data: &[u8]) -> Result<(), String> {
        self.core.mmu_mut().restore_sram(data)
    }

    pub fn restore_sram_from_file(&mut self, path: &str) -> Result<(), String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        self.restore_sram(&data)
    }

    /// Write every save-state section this build knows about. Exposed for tests that need to
    /// manipulate the container; ordinary callers want [`GameBoy::save_state`].
    pub(crate) fn write_sections(&self, writer: &mut SectionWriter) -> Result<(), String> {
        self.core.write_sections(writer)
    }

    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let mut writer = SectionWriter::new();
        self.write_sections(&mut writer)?;
        Ok(writer.finish())
    }

    pub fn save_state_to_file(&self, path: &str) -> Result<(), String> {
        let data = self.save_state()?;
        std::fs::write(path, &data).map_err(|e| e.to_string())
    }

    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        let reader = SectionReader::parse(data)?;

        // Applied to a copy so a failure part-way through cannot leave a half-loaded machine.
        let mut candidate = self.clone();
        candidate.core.read_sections(&reader)?;

        if candidate.core.mmu().header() != self.core.mmu().header() {
            return Err(format!("Incompatible save state, expected {:?}, got {:?}", self.core.mmu().header(), candidate.core.mmu().header()));
        }

        *self = candidate;
        Ok(())
    }

    pub fn load_state_from_file(&mut self, path: &str) -> Result<(), String> {
        let data = std::fs::read(path)
            .map_err(|e| e.to_string())?;
        self.load_state(&data)
    }

    pub fn save_screenshot_to_file(&self, path: &str) -> Result<(), String> {
        let image = self.core().mmu().ppu().screenshot();
        image.save(path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use image::RgbImage;
    use crate::ram::RAM;
    use crate::roms::roms::parse_png;
    use super::*;

    /// Throughput of the emulator core alone.
    /// ```text
    /// cargo test --release --features slow-tests --lib -- \
    ///   game_boy::tests::bench_core_throughput --exact --nocapture
    /// ```
    #[cfg(feature = "slow-tests")]
    #[test]
    fn bench_core_throughput() {
        use std::time::Instant;

        /// One frame: 154 scanlines x 456 T-cycles.
        const FRAME: MachineCycles = MachineCycles::from_t(70_224);
        const WARM_UP_FRAMES: usize = 60;

        // Two knobs, for `perf` rather than for the benchmark itself.
        let measured_frames: usize = std::env::var("BENCH_FRAMES")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(600);
        let only = std::env::var("BENCH_ONLY").unwrap_or_default();
        // A third knob, and this one changes what is being measured rather than how long for.
        let audio_output = std::env::var("BENCH_AUDIO").unwrap_or_default() != "off";

        fn pokemon_in_game() -> GameBoy {
            use crate::ram::ROM;
            use crate::test_fixtures::symbols;
            let mut gb = GameBoy::dmg(crate::test_fixtures::POKERED);
            gb.load_state(crate::test_fixtures::AT_CELADON).expect("load fixture");
            // The claim below is "a real game, mid-play", and nothing else here checks it.
            let mmu = gb.core().mmu();
            assert_eq!(
                (mmu.read(symbols::W_CUR_MAP), mmu.read(symbols::W_IS_IN_BATTLE)),
                (symbols::CELADON_CITY, 0),
                "the benchmark fixture is no longer standing in the Celadon overworld"
            );
            gb
        }

        let workloads: Vec<(&str, Box<dyn Fn() -> GameBoy>)> = vec![
            // The representative case: a real game, mid-play, HALTing as games do.
            ("pokemon-red (fixture)", Box::new(pokemon_in_game)),
            // Never HALTs, so it isolates raw dispatch from idle-skipping.
            ("cpu_instrs.gb", Box::new(|| GameBoy::dmg(crate::roms::blargg_cpu::ROM))),
            // PPU-heavy.
            ("dmg-acid2.gb", Box::new(|| GameBoy::dmg(crate::roms::acid::ROM))),
        ];

        println!("\naudio output side: {}", if audio_output { "on" } else { "gated (BENCH_AUDIO=off)" });
        println!("{:<24} {:>12} {:>16} {:>10}", "workload", "realtime", "t-cycles/s", "frames");
        println!("{}", "-".repeat(66));

        for (name, build) in &workloads {
            if !only.is_empty() && !name.contains(&only) {
                continue;
            }
            let mut gb = build();
            gb.core_mut().mmu_mut().audio_mut().set_output_enabled(audio_output);
            for _ in 0..WARM_UP_FRAMES {
                gb.run(FRAME);
            }

            let start = Instant::now();
            let mut cycles = MachineCycles::ZERO;
            for _ in 0..measured_frames {
                cycles += gb.run(FRAME);
            }
            let elapsed = start.elapsed();

            let t_cycles_per_sec = cycles.t_cycles() as f64 / elapsed.as_secs_f64();
            let realtime = t_cycles_per_sec / MachineCycles::CPU_FREQ as f64;
            println!(
                "{name:<24} {realtime:>11.1}x {:>16.0} {measured_frames:>10}",
                t_cycles_per_sec
            );
        }
        println!();
    }

    #[test]
    fn the_halt_fast_path_matches_stepping_cycle_by_cycle() {
        /// One frame: 154 scanlines x 456 T-cycles.
        const FRAME: MachineCycles = MachineCycles::from_t(70_224);
        const FRAMES: usize = 120;

        fn run_stepped(gb: &mut GameBoy, min_cycles: MachineCycles) {
            let mut cycles = MachineCycles::ZERO;
            while cycles < min_cycles {
                let opcode = gb.core_mut().fetch();
                cycles += gb.core_mut().execute(opcode);
            }
        }

        let pokemon = || {
            let mut gb = GameBoy::dmg(crate::test_fixtures::POKERED);
            gb.load_state(crate::test_fixtures::AT_CELADON).expect("load fixture");
            gb
        };
        let workloads: Vec<(&str, Box<dyn Fn() -> GameBoy>)> = vec![
            ("pokemon-red", Box::new(pokemon)),
            ("dmg-acid2", Box::new(|| GameBoy::dmg(crate::roms::acid::ROM))),
            // A CGB too: the skip converts the PPU and APU deadlines out of the video clock, and
            // that conversion only exists because a CGB can halve it.
            ("cgb-acid2", Box::new(|| GameBoy::cgb(crate::roms::cgb_acid::ROM))),
            ("pokemon-red (cgb compat)", Box::new(|| GameBoy::cgb(crate::test_fixtures::POKERED))),
        ];

        for (name, build) in &workloads {
            let (mut fast, mut slow) = (build(), build());
            let mut halted = false;
            for _ in 0..FRAMES {
                fast.run(FRAME);
                run_stepped(&mut slow, FRAME);
                halted |= slow.core().mode() == crate::core::CoreMode::Halt;
                assert_eq!(fast, slow, "{name}: machine state diverged");
                assert!(
                    fast.core().mmu().ppu().lcd() == slow.core().mmu().ppu().lcd(),
                    "{name}: framebuffer diverged",
                );
            }
            assert!(halted, "{name} never HALTs, so it proves nothing about the fast path");
        }
    }

    #[cfg(not(feature = "slow-tests"))]
    #[test]
    fn deadline_driving_the_channels_is_invisible_to_the_game() {
        let expected = parse_png(crate::roms::blargg_dmg_sound::EXPECTED_ALL);
        let mut batched = GameBoy::dmg(crate::roms::blargg_dmg_sound::ROM);
        let mut open = GameBoy::dmg(crate::roms::blargg_dmg_sound::ROM);
        batched.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
        open.core_mut().mmu_mut().audio_mut().set_channel_batching(false);

        let mut cycles = MachineCycles::ZERO;
        let mut ever_batched = false;
        let mut passed = false;
        while cycles < MachineCycles::from_m(60_000_000) {
            cycles += batched.run(MachineCycles::from_m(1000));
            open.run(MachineCycles::from_m(1000));

            let (a, b) = (batched.core().mmu().audio(), open.core().mmu().audio());
            assert!(a == b, "the APU diverged with the channels deadline-driven, at {cycles:?}");
            assert_eq!(b.pending_channel_cycles(), 0, "the control machine batched");
            ever_batched |= a.pending_channel_cycles() > 0;

            if batched.core().mmu().ppu().screenshot() == expected {
                passed = true;
                break;
            }
        }

        // Otherwise this is twelve sub-tests passing against a mechanism that never engaged.
        assert!(ever_batched, "the channels were never once left behind, so this proves nothing");
        if !passed {
            gb_test_failed_with_screenshot(
                batched.core().mmu().ppu().screenshot(),
                "audio-all-batched",
                "screenshot does not match",
            );
        }
    }

    /// The only thing standing between a batching APU and music that is quietly wrong.
    #[cfg(not(feature = "slow-tests"))]
    #[test]
    fn batching_the_channels_under_a_listener_is_inaudible() {
        /// One frame: 154 scanlines x 456 T-cycles.
        const FRAME: MachineCycles = MachineCycles::from_t(70_224);
        const FRAMES: usize = 300;

        // The same fixture the gating test uses: a real game mid-play with overworld music, so
        // all four channels are moving and there is something to get wrong.
        let build = || {
            let mut gb = GameBoy::dmg(crate::test_fixtures::POKERED);
            gb.load_state(crate::test_fixtures::AT_CELADON).expect("load fixture");
            gb
        };
        let (mut batched, mut control) = (build(), build());
        control.core_mut().mmu_mut().audio_mut().set_channel_batching(false);
        batched.core_mut().mmu_mut().audio_mut().capture_output_transitions();
        control.core_mut().mmu_mut().audio_mut().capture_output_transitions();

        fn drain(gb: &mut GameBoy, into: &mut Vec<f32>, scratch: &mut [f32]) {
            loop {
                let frames = gb.core_mut().mmu_mut().audio_mut().read_samples_f32(scratch);
                if frames == 0 {
                    return;
                }
                into.extend_from_slice(&scratch[..frames * 2]);
            }
        }

        let mut scratch = vec![0.0f32; 8192];
        let (mut batched_pcm, mut control_pcm) = (Vec::new(), Vec::new());
        let mut ever_batched = false;
        for frame in 0..FRAMES {
            batched.run(FRAME);
            control.run(FRAME);
            assert_eq!(batched, control, "the machine diverged at frame {frame}");
            assert!(
                batched.core().mmu().ppu().lcd() == control.core().mmu().ppu().lcd(),
                "the framebuffer diverged at frame {frame}",
            );
            ever_batched |= batched.core().mmu().audio().pending_channel_cycles() > 0;
            assert_eq!(
                control.core().mmu().audio().pending_channel_cycles(), 0,
                "the control machine batched, so it is not a control",
            );
            drain(&mut batched, &mut batched_pcm, &mut scratch);
            drain(&mut control, &mut control_pcm, &mut scratch);
        }

        // Otherwise both halves of this are comparing two silent machines that never batched.
        assert!(ever_batched, "the channels were never once left behind, so this proves nothing");
        assert!(control_pcm.len() > 100_000, "the fixture produced almost no audio: {}", control_pcm.len());

        let a = batched.core_mut().mmu_mut().audio_mut().take_output_transitions();
        let b = control.core_mut().mmu_mut().audio_mut().take_output_transitions();
        assert!(a.len() > 1000, "only {} transitions, so this proves little", a.len());
        assert_eq!(a.len(), b.len(), "a transition went missing or was invented");
        for (i, (x, y)) in a.iter().zip(&b).take(a.len() - 1).enumerate() {
            if x != y {
                // The neighbours, because one `(clocks, left, right)` on its own says nothing
                // about whether a transition moved, was lost, or was invented.
                for j in i.saturating_sub(4)..(i + 5).min(a.len()) {
                    println!("{j:6}  batched {:?}   control {:?}", a[j], b[j]);
                }
                panic!("transition {i} of {} differs: batched {x:?}, control {y:?}", a.len());
            }
        }
        let (last_a, last_b) = (a[a.len() - 1], b[b.len() - 1]);
        assert_eq!(
            (last_a.1, last_a.2), (last_b.1, last_b.2),
            "the final amplitude differs: batched {last_a:?}, control {last_b:?}",
        );

        // The frames a sink reads back.
        let common = batched_pcm.len().min(control_pcm.len());
        assert!(
            control_pcm.len() - common < 16,
            "the two machines produced very different amounts of audio: {} and {}",
            batched_pcm.len(), control_pcm.len(),
        );
        if let Some(i) = (0..common).find(|&i| batched_pcm[i] != control_pcm[i]) {
            panic!(
                "sample {i} of {common} differs: batched {}, control {}",
                batched_pcm[i], control_pcm[i],
            );
        }
    }

    /// The APU's output side is skipped when nobody is listening, and the machine must not be
    /// able to tell.
    #[test]
    fn silencing_the_output_side_is_invisible_to_the_game() {
        /// One frame: 154 scanlines x 456 T-cycles.
        const FRAME: MachineCycles = MachineCycles::from_t(70_224);
        const FRAMES: usize = 120;

        // A real game, mid-play with music playing — a workload whose DACs are on, so the mixing
        // being skipped is mixing that would otherwise have happened.
        let build = || {
            let mut gb = GameBoy::dmg(crate::test_fixtures::POKERED);
            gb.load_state(crate::test_fixtures::AT_CELADON).expect("load fixture");
            gb
        };
        let (mut gated, mut open) = (build(), build());
        gated.core_mut().mmu_mut().audio_mut().set_output_enabled(false);

        let mut scratch = vec![0.0f32; 8192];
        let (mut gated_frames, mut open_frames) = (0usize, 0usize);
        for _ in 0..FRAMES {
            gated.run(FRAME);
            open.run(FRAME);
            assert_eq!(gated, open, "the machine diverged with the APU's output side gated");
            assert!(
                gated.core().mmu().ppu().lcd() == open.core().mmu().ppu().lcd(),
                "the framebuffer diverged with the APU's output side gated",
            );
            gated_frames += gated.core_mut().mmu_mut().audio_mut().read_samples_f32(&mut scratch);
            open_frames += open.core_mut().mmu_mut().audio_mut().read_samples_f32(&mut scratch);
        }

        // Otherwise the equality above is comparing two machines that were both making no sound.
        assert!(open_frames > 0, "the fixture produced no audio at all, so this proves nothing");
        assert_eq!(gated_frames, 0, "the gate is open: {gated_frames} frames came out of it");

        // And it comes back.
        gated.core_mut().mmu_mut().audio_mut().set_output_enabled(true);
        let mut resumed = 0usize;
        for _ in 0..FRAMES {
            gated.run(FRAME);
            resumed += gated.core_mut().mmu_mut().audio_mut().read_samples_f32(&mut scratch);
        }
        assert!(resumed > 0, "the sound never came back after the gate reopened");
    }

    #[test]
    fn reset_matches_fresh_construction() {
        let mut a = GameBoy::dmg(crate::roms::acid::ROM);
        a.run(MachineCycles::from_m(500_000));
        assert_ne!(a, GameBoy::dmg(crate::roms::acid::ROM), "test is vacuous if running changed nothing");

        a.reset();
        assert_eq!(a, GameBoy::dmg(crate::roms::acid::ROM));
    }

    /// Reset matches construction on a CGB too, including re-applying the boot palette, which a
    /// fresh compatibility-mode machine has and a naively reset one would not.
    #[test]
    fn cgb_reset_matches_fresh_construction() {
        for cart in [crate::roms::cgb_acid::ROM, crate::test_fixtures::POKERED] {
            let mut a = GameBoy::cgb(cart);
            a.run(MachineCycles::from_m(500_000));
            assert_ne!(a, GameBoy::cgb(cart), "test is vacuous if running changed nothing");

            a.reset();
            assert_eq!(a, GameBoy::cgb(cart));
            assert_eq!(a.core().registers().a, 0x11, "and it still reports a CGB");
        }
    }

    /// Battery-backed cartridge RAM does not lose its contents when the console is reset.
    #[test]
    fn reset_preserves_sram() {
        let mut gb = GameBoy::dmg(crate::test_fixtures::POKERED);
        let sram: Vec<u8> = (0..gb.dump_sram().len()).map(|i| (i % 251) as u8).collect();
        gb.restore_sram(&sram).expect("restore sram");

        gb.reset();

        assert_eq!(gb.dump_sram(), sram);
    }

    #[test]
    fn run_terminates_after_stop() {
        let mut gb = GameBoy::dmg_hello_world();
        // STOP, then a pad byte, in work RAM — ROM ignores writes.
        gb.core_mut().mmu_mut().write(0xC000, 0x10);
        gb.core_mut().mmu_mut().write(0xC001, 0x00);
        gb.core_mut().registers_mut().pc = 0xC000;

        let cycles = gb.run(MachineCycles::from_m(10_000));

        assert!(cycles >= MachineCycles::from_m(10_000));
        assert_eq!(gb.core().mode(), crate::core::CoreMode::Stop, "nothing should have woken it");
    }

    #[test]
    fn save_and_load_state() {
        // Create a GameBoy and run it for some cycles to change its state
        let mut original_gb = GameBoy::dmg_hello_world();
        original_gb.run(MachineCycles::from_m(10_000));

        // Save the state
        let saved_state = original_gb.save_state()
            .expect("Failed to save state");

        // Create a new GameBoy and run it for different cycles
        let mut different_gb = GameBoy::dmg_hello_world();
        different_gb.run(MachineCycles::from_m(2000));

        // Load the saved state
        let mut loaded_gb = GameBoy::dmg_hello_world();
        loaded_gb.load_state(&saved_state).expect("Failed to load state");

        // Verify the loaded state matches the original
        assert_eq!(original_gb, loaded_gb);
    }

    mod blargg_cpu {
        use super::*;
        use crate::roms::blargg_cpu::*;

        #[test]
        fn cpu_01_special() {
            serial_console_test("cpu-01", SPECIAL_01);
        }

        #[test]
        fn cpu_02_interrupts() {
            serial_console_test("cpu-02", INTERRUPTS_02);
        }

        #[test]
        fn cpu_03_op_sp_hl() {
            serial_console_test("cpu-03", OP_SP_HL_03);
        }

        #[test]
        fn cpu_04_op_r_imm() {
            serial_console_test("cpu-04", OP_R_IMM_04);
        }

        #[test]
        fn cpu_05_op_rp() {
            serial_console_test("cpu-05", OP_RP_05);
        }

        #[test]
        fn cpu_06_ld_r_r() {
            serial_console_test("cpu-06", LD_R_R_06);
        }

        #[test]
        fn cpu_07_jr_jp_call_ret_rst() {
            serial_console_test("cpu-07", JR_JP_CALL_RET_RST_07);
        }

        #[test]
        fn cpu_08_misc_instrs() {
            serial_console_test("cpu-08", MISC_INSTRUCTIONS_08);
        }

        #[test]
        fn cpu_09_op_r_r() {
            serial_console_test("cpu-09", OP_R_R_09);
        }

        #[test]
        fn cpu_10_bit_ops() {
            serial_console_test("cpu-10", BIT_OPS_10);
        }

        #[test]
        fn cpu_11_op_a_hl() {
            serial_console_test("cpu-11", OP_A_HL_11);
        }

        /// The whole suite in one run, off the combined 64 KB ROM — so unlike the eleven
        /// sub-tests above, this one actually exercises MBC bank switching.
        #[test]
        fn all() {
            serial_console_test_within("cpu-all", ROM, MachineCycles::from_m(60_000_000));
        }

        #[test]
        fn instruction_timing() {
            serial_console_test("instruction-timing", INSTRUCTION_TIMING);
        }
    }

    mod blargg_dmg_sound {
        use crate::roms::blargg_dmg_sound::*;
        use super::*;

        /// The whole suite in one run, off the combined 64 KB ROM — so unlike the twelve
        /// sub-tests below, this one exercises MBC bank switching.
        #[test]
        fn all() {
            ppu_test_within("audio-all", ROM, EXPECTED_ALL, MachineCycles::from_m(60_000_000));
        }

        #[test]
        fn registers() {
            ppu_test("audio-registers", REGISTERS, EXPECTED_REGISTERS);
        }

        #[test]
        fn length_counter() {
            ppu_test("audio-length-counter", LENGTH_COUNTER, EXPECTED_LENGTH_COUNTER);
        }

        #[test]
        fn trigger() {
            ppu_test("audio-trigger", TRIGGER, EXPECTED_TRIGGER);
        }

        #[test]
        fn sweep() {
            ppu_test("audio-sweep", SWEEP, EXPECTED_SWEEP);
        }

        #[test]
        fn sweep_details() {
            ppu_test("audio-sweep-details", SWEEP_DETAILS, EXPECTED_SWEEP_DETAILS);
        }

        #[test]
        fn overflow_on_trigger() {
            ppu_test("audio-overflow-on-trigger", OVERFLOW_ON_TRIGGER, EXPECTED_OVERFLOW_ON_TRIGGER);
        }

        #[test]
        fn length_sweep_period_sync() {
            ppu_test("audio-length-sweep-period-sync", LENGTH_SWEEP_PERIOD_SYNC, EXPECTED_LENGTH_SWEEP_PERIOD_SYNC);
        }

        #[test]
        fn length_counter_during_power() {
            ppu_test("audio-length-counter-during-power", LENGTH_COUNTER_DURING_POWER, EXPECTED_LENGTH_COUNTER_DURING_POWER);
        }

        #[test]
        fn wave_read_while_on() {
            ppu_test("audio-wave-read-while-on", WAVE_READ_WHILE_ON, EXPECTED_WAVE_READ_WHILE_ON);
        }

        #[test]
        fn wave_trigger_while_on() {
            ppu_test("audio-wave-trigger-while-on", WAVE_TRIGGER_WHILE_ON, EXPECTED_WAVE_TRIGGER_WHILE_ON);
        }

        #[test]
        fn registers_after_power() {
            ppu_test("audio-registers-after-power", REGISTERS_AFTER_POWER, EXPECTED_REGISTERS_AFTER_POWER);
        }

        #[test]
        fn wave_write_while_on() {
            ppu_test("audio-wave-write-while-on", WAVE_WRITE_WHILE_ON, EXPECTED_WAVE_WRITE_WHILE_ON);
        }

    }

    mod joypad {
        use crate::joypad::JoypadButton;
        use super::*;
        use crate::roms::button_test::*;

        #[test]
        fn button_a() {
            test_button(JoypadButton::A, EXPECTED_A);
        }

        #[test]
        fn button_b() {
            test_button(JoypadButton::B, EXPECTED_B);
        }

        #[test]
        fn button_select() {
            test_button(JoypadButton::Select, EXPECTED_SELECT);
        }

        #[test]
        fn button_start() {
            test_button(JoypadButton::Start, EXPECTED_START);
        }

        #[test]
        fn button_up() {
            test_button(JoypadButton::Up, EXPECTED_UP);
        }

        #[test]
        fn button_down() {
            test_button(JoypadButton::Down, EXPECTED_DOWN);
        }

        #[test]
        fn button_left() {
            test_button(JoypadButton::Left, EXPECTED_LEFT);
        }

        #[test]
        fn button_right() {
            test_button(JoypadButton::Right, EXPECTED_RIGHT);
        }

        fn test_button(button: JoypadButton, expected_screenshot: &[u8]) {
            let mut gb = GameBoy::dmg(ROM);
            gb.run(MachineCycles::from_m(400_000));

            gb.core_mut().mmu_mut().joypad_mut()
                .press_button(button);

            gb.run(MachineCycles::from_m(20_000));

            gb.core_mut().mmu_mut().joypad_mut()
                .release_button(button);

            gb.run(MachineCycles::from_m(20_000));

            let result = gb.core().mmu().ppu().screenshot();

            let expected_screenshot = parse_png(expected_screenshot);
            if result != expected_screenshot {
                gb_test_failed_with_screenshot(result, &format!("{}-button", button), "screenshot does not match");
            }
        }
    }

    mod ppu {
        use std::io::BufReader;
        use image::{ImageFormat, ImageReader};
        use crate::roms::acid::*;
        use super::*;

        #[test]
        fn ppu() {
            let mut gb = GameBoy::dmg(ROM);
            gb.run(MachineCycles::from_m(180_000));

            let result = gb.core().mmu().ppu().screenshot();
            let expected_image = ImageReader::with_format(BufReader::new(std::io::Cursor::new(EXPECTED_DMG)), ImageFormat::Png)
                .decode()
                .expect("Failed to decode expected image")
                .to_rgb8();

            if result != expected_image {
                gb_test_failed_with_screenshot(result, "ppu", "screenshot does not match");
            }
        }

        #[test]
        fn cgb_ppu() {
            cgb_ppu_test("cgb-acid2", crate::roms::cgb_acid::ROM, crate::roms::cgb_acid::EXPECTED);
        }

        /// Pokémon Red is a DMG-only cartridge, so on a Game Boy Color the boot ROM picks its
        /// palette from the title checksum.
        #[test]
        fn pokemon_red_boots_in_colour_on_a_cgb() {
            use crate::lcd_palette::LcdColor;
            use crate::model::ColorMode;

            fn ramp(colors: [u16; 4]) -> [image::Rgb<u8>; 4] {
                colors.map(LcdColor::from_rgb555).map(|c| c.to_rgb())
            }
            // Combination 13: BG and OBJ1 from pool palette 4 (red), OBJ0 from palette 3 (green).
            let background = ramp([0x7FFF, 0x421F, 0x1CF2, 0x0000]);
            let object0 = ramp([0x7FFF, 0x1BEF, 0x0200, 0x0000]);
            let shades = [0xFFu8, 0xAA, 0x55, 0x00];

            let mut cgb = GameBoy::cgb(crate::test_fixtures::POKERED);
            let mut dmg = GameBoy::dmg(crate::test_fixtures::POKERED);
            assert_eq!(cgb.core().mmu().color_mode(), ColorMode::CgbCompat);
            assert_eq!(dmg.core().mmu().color_mode(), ColorMode::Dmg);

            let mut saw_colour = false;
            // The copyright screen, the Game Freak logo, and the Nidorino/Gengar intro.
            for checkpoint in [3_000_000u64, 8_000_000, 12_000_000] {
                let step = MachineCycles::from_m(checkpoint) - cgb_elapsed(checkpoint);
                cgb.run(step);
                dmg.run(step);
                let colour = cgb.core().mmu().ppu().screenshot();
                let grey = dmg.core().mmu().ppu().screenshot();

                for (x, y, pixel) in grey.enumerate_pixels() {
                    // The DMG path is untouched: still exactly gb's four greys.
                    assert_eq!(pixel.0[0], pixel.0[1], "DMG pixel ({x},{y}) is not grey");
                    let shade = shades.iter().position(|&s| s == pixel.0[0])
                        .unwrap_or_else(|| panic!("DMG pixel ({x},{y}) = {pixel:?} is not a gb shade"));

                    // ...and the CGB frame is the same shade, through the background ramp or —
                    // for a sprite pixel — the object ramp.
                    let actual = *colour.get_pixel(x, y);
                    assert!(
                        actual == background[shade] || actual == object0[shade],
                        "pixel ({x},{y}) at {checkpoint}M: shade {shade} rendered as {actual:?}, \
                         expected {:?} (BG) or {:?} (OBJ0)", background[shade], object0[shade]
                    );
                    saw_colour |= actual.0[0] != actual.0[1];
                }
            }
            assert!(saw_colour, "test is vacuous unless some CGB frame is actually coloured");
        }

        /// The checkpoints in [`pokemon_red_boots_in_colour_on_a_cgb`] are absolute, but `run`
        /// takes a delta; this turns one into the other.
        fn cgb_elapsed(checkpoint: u64) -> MachineCycles {
            const CHECKPOINTS: [u64; 3] = [3_000_000, 8_000_000, 12_000_000];
            let index = CHECKPOINTS.iter().position(|&c| c == checkpoint).expect("a known checkpoint");
            MachineCycles::from_m(if index == 0 { 0 } else { CHECKPOINTS[index - 1] })
        }

        /// The same ROM on a DMG, where it is supposed to say so and stop.
        #[test]
        fn cgb_acid2_on_a_dmg_stays_monochrome() {
            use crate::model::ColorMode;

            let mut gb = GameBoy::dmg(crate::roms::cgb_acid::ROM);
            gb.run(MachineCycles::from_m(2_000_000));

            assert_eq!(gb.core().mmu().color_mode(), ColorMode::Dmg);
            let shades: std::collections::HashSet<_> =
                gb.core().mmu().ppu().screenshot().pixels().copied().collect();
            assert!(
                shades.iter().all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2]),
                "a DMG must only ever emit greys, got {shades:?}"
            );
        }
    }

    /// `interrupt_time` on a DMG.
    mod blargg_timing {
        use super::*;
        use crate::roms::blargg_timing::*;

        #[test]
        fn interrupt_time() { ppu_test("interrupt_time", INTERRUPT_TIME, EXPECTED_INTERRUPT_TIME); }
    }

    fn serial_console_test(name: &str, cart: &[u8]) {
        serial_console_test_within(name, cart, MachineCycles::from_m(25_000_000));
    }

    /// [`serial_console_test`] with an explicit cycle budget, for ROMs that need longer than the
    /// default.
    fn serial_console_test_within(name: &str, cart: &[u8], budget: MachineCycles) {
        let mut gb = GameBoy::dmg(cart);
        gb.core.mmu_mut().serial_mut().enable_buffer();

        let mut max_cycles = budget;
        let mut cycles = MachineCycles::ZERO;
        let mut serial_output = String::new();
        let mut failed = false;
        while cycles < max_cycles {
            cycles += gb.run(MachineCycles::from_m(1000));

            serial_output = gb.core.mmu().serial()
                .buffered_bytes()
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();

            if serial_output.contains("Passed") {
                return;
            } else if !failed && serial_output.contains("Failed") {
                // Run for a few more cycles to collect more output
                max_cycles = cycles + MachineCycles::from_m(10_000);
                failed = true;
            }
        }

        gb_test_failed(&gb, name, &serial_output);
    }

    fn ppu_test(name: &str, cart: &[u8], expected_screenshot: &[u8]) {
        ppu_test_within(name, cart, expected_screenshot, MachineCycles::from_m(20_000_000));
    }

    /// [`ppu_test`] on a Game Boy Color.
    fn cgb_ppu_test(name: &str, cart: &[u8], expected_screenshot: &[u8]) {
        let expected_screenshot = parse_png(expected_screenshot);
        let mut gb = GameBoy::cgb(cart);
        let mut cycles = MachineCycles::ZERO;
        let mut last_screenshot = gb.core().mmu().ppu().screenshot();

        while cycles < MachineCycles::from_m(20_000_000) {
            cycles += gb.run(MachineCycles::from_m(1000));
            last_screenshot = gb.core().mmu().ppu().screenshot();
            if last_screenshot == expected_screenshot {
                return;
            }
        }
        gb_test_failed_with_screenshot(last_screenshot, name, "screenshot does not match");
    }

    /// [`ppu_test`] with an explicit cycle budget, for ROMs that need longer than the default.
    fn ppu_test_within(name: &str, cart: &[u8], expected_screenshot: &[u8], max_cycles: MachineCycles) {
        let expected_screenshot = parse_png(expected_screenshot);
        let mut gb = GameBoy::dmg(cart);
        let mut cycles = MachineCycles::ZERO;
        let mut last_screenshot = gb.core().mmu().ppu().screenshot();

        while cycles < max_cycles {
            cycles += gb.run(MachineCycles::from_m(1000));
            last_screenshot = gb.core().mmu().ppu().screenshot();

            if last_screenshot == expected_screenshot {
                return;
            }
        }

        gb_test_failed_with_screenshot(last_screenshot, name, "screenshot does not match");
    }

    mod mooneye {
        use super::*;

        const PASS: [u8; 6] = [3, 5, 8, 13, 21, 34];

        /// Run one ROM and return its verdict, or `Err` with what it did instead.
        fn run(compressed: &[u8]) -> Result<(), String> {
            let cart = crate::roms::mooneye::rom(compressed);
            let mut gb = GameBoy::try_dmg(&cart).map_err(|e| format!("would not load: {e}"))?;
            gb.core.mmu_mut().serial_mut().enable_buffer();

            let budget = MachineCycles::from_m(25_000_000);
            let mut cycles = MachineCycles::ZERO;
            while cycles < budget {
                cycles += gb.run(MachineCycles::from_m(10_000));
                let out = gb.core.mmu().serial().buffered_bytes().unwrap_or_default();
                if out.len() >= PASS.len() {
                    return if out[..PASS.len()] == PASS {
                        Ok(())
                    } else {
                        Err(format!("reported failure, sent {:?}", &out[..PASS.len()]))
                    };
                }
            }
            Err("never reported a result".to_string())
        }

        /// The one ROM `gb` is not expected to pass.
        const SKIPPED: &[&str] = &["mbc1-multicart_rom_8Mb"];

        /// Run every ROM whose name starts with `mapper`, reporting all failures at once — with
        /// 28 ROMs, stopping at the first one hides how much is broken.
        fn suite(mapper: &str) {
            let roms: Vec<_> = crate::roms::mooneye::ALL
                .iter()
                .filter(|(name, _)| name.starts_with(mapper))
                .filter(|(name, _)| {
                    if SKIPPED.contains(name) {
                        println!("skipping {name}: MBC1 multicart is not implemented");
                        false
                    } else {
                        true
                    }
                })
                .collect();
            assert!(!roms.is_empty(), "no ROMs matched {mapper}");

            let failures: Vec<String> = roms
                .iter()
                .filter_map(|(name, rom)| run(rom).err().map(|why| format!("  {name}: {why}")))
                .collect();

            assert!(
                failures.is_empty(),
                "{}/{} mooneye {mapper} tests failed:\n{}",
                failures.len(),
                roms.len(),
                failures.join("\n")
            );
        }

        #[test]
        fn mbc1() {
            suite("mbc1");
        }

        #[test]
        fn mbc2() {
            suite("mbc2");
        }

        #[test]
        fn mbc5() {
            suite("mbc5");
        }
    }

    fn gb_test_failed(gb: &GameBoy, name: &str, reason: &str) {
        let image = gb.core().mmu().ppu().screenshot();
        gb_test_failed_with_screenshot(image, name, reason);
    }

    fn gb_test_failed_with_screenshot(image: RgbImage, name: &str, reason: &str) {
        let result_path = &format!("target/test_failure_{}.png", name);
        image.save(result_path).expect("Failed to save result image");
        panic!("{} test failed, saved result image to {}, reason: {}", name, result_path, reason);
    }
}
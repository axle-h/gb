use gb::audio::Audio;
use gb::cycles::MachineCycles;
use gb::divider::Divider;
use gb::mmu::MMU;
use pokered::audio::{Voices, Write};

/// The APU puts a level change at the end of the update that finds it, and the emulator updates
/// once an instruction, so this steps finely enough to land within an instruction of it.
const STEP: u64 = 1;

/// The emulator's APU, driven by the engine's writes instead of a CPU.
#[derive(Clone)]
pub struct GbApu {
    audio: Audio,
    divider: Divider,
    now: u64,
}

impl GbApu {
    pub fn new(sample_rate: u32) -> Self {
        let mut audio = Audio::default();
        audio.set_output_sample_rate(sample_rate);
        let mut divider = Divider::default();
        divider.enable(0);
        Self { audio, divider, now: 0 }
    }

    /// Carries on from a running machine's APU, as it stands.
    pub fn from_machine(mmu: &MMU) -> Self {
        Self { audio: mmu.audio().clone(), divider: *mmu.divider(), now: mmu.now() }
    }

    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn advance_to(&mut self, now: u64) {
        while self.now < now {
            let step = (now - self.now).min(STEP);
            self.now += step;
            let clocks = self.divider.catch_up(self.now);
            self.audio.update(MachineCycles::from_m(step), clocks);
        }
    }

    pub fn write_register(&mut self, address: u16, value: u8) {
        self.audio.write(address, value);
    }
}

impl Voices for GbApu {
    fn write(&mut self, write: Write) {
        let (address, value) = write.register();
        self.write_register(address, value);
    }

    fn end_frame(&mut self) {
        self.advance_to(self.now + MachineCycles::PER_FRAME.m_cycles());
    }

    fn read_samples(&mut self, out: &mut [f32]) -> usize {
        self.audio.read_samples_f32(out)
    }
}

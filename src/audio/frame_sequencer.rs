use crate::divider::DividerClocks;
use bitflags::bitflags;

#[derive(Debug, Clone, Default)]
pub struct FrameSequencer {
    value: u8,
}

impl FrameSequencer {

    pub fn reset(&mut self) {
        self.value = 0;
    }

    pub fn update(&mut self, div_clocks: DividerClocks) -> FrameSequencerEvent {
        let mut events = FrameSequencerEvent::empty();
        // TODO bit 4 in normal speed mode, bit 5 in CBG (double) speed mode
        let delta = div_clocks.bit_fall_edge(4);
        for _ in 0..delta {
            self.value += 1;
            self.value %= 8;
            events |= self.current_events();
        }
        events
    }
    
    pub fn current_events(&self) -> FrameSequencerEvent {
        // see "FrameSequencer" in https://nightshade256.github.io/2021/03/27/gb-sound-emulation.html
        let mut events = FrameSequencerEvent::empty();
        match self.value {
            0 | 4 => events |= FrameSequencerEvent::LengthCounter,
            2 | 6 => events |= FrameSequencerEvent::Sweep | FrameSequencerEvent::LengthCounter,
            7 => events |= FrameSequencerEvent::VolumeEnvelope,
            _ => {}
        }
        events
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FrameSequencerEvent: u8 {
        const LengthCounter = 0x1;
        const VolumeEnvelope = 0x2;
        const Sweep = 0x4;
    }
}

impl FrameSequencerEvent {
    pub fn is_length_counter(&self) -> bool {
        self.contains(FrameSequencerEvent::LengthCounter)
    }
    
    pub fn is_volume_envelope(&self) -> bool {
        self.contains(FrameSequencerEvent::VolumeEnvelope)
    }
    
    pub fn is_sweep(&self) -> bool {
        self.contains(FrameSequencerEvent::Sweep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CLOCKS_PER_STEP: DividerClocks = DividerClocks { initial_value: 0, count: 32 };

    #[test]
    fn clocks_at_correct_rate() {
        let mut fs = FrameSequencer::default();
        assert_eq!(fs.value, 0);

        let events = fs.update(CLOCKS_PER_STEP);
        assert_eq!(fs.value, 1);
        assert_eq!(events, FrameSequencerEvent::empty());

        let events = fs.update(CLOCKS_PER_STEP);
        assert_eq!(fs.value, 2);
        assert_eq!(events, FrameSequencerEvent::LengthCounter | FrameSequencerEvent::Sweep);

        let events = fs.update(CLOCKS_PER_STEP);
        assert_eq!(fs.value, 3);
        assert_eq!(events, FrameSequencerEvent::empty());

        let events = fs.update(CLOCKS_PER_STEP);
        assert_eq!(fs.value, 4);
        assert_eq!(events, FrameSequencerEvent::LengthCounter);

        let events = fs.update(CLOCKS_PER_STEP);
        assert_eq!(fs.value, 5);
        assert_eq!(events, FrameSequencerEvent::empty());

        let events = fs.update(CLOCKS_PER_STEP);
        assert_eq!(fs.value, 6);
        assert_eq!(events, FrameSequencerEvent::LengthCounter | FrameSequencerEvent::Sweep);

        let events = fs.update(CLOCKS_PER_STEP);
        assert_eq!(fs.value, 7);
        assert_eq!(events, FrameSequencerEvent::VolumeEnvelope);

        let events = fs.update(CLOCKS_PER_STEP);
        assert_eq!(fs.value, 0);
        assert_eq!(events, FrameSequencerEvent::LengthCounter);
    }
}
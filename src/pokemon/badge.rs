use std::fmt::{Display, Formatter};
use bitflags::bitflags;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Badge(u8);

bitflags! {
    impl Badge: u8 {
        const BoulderBadge = 0x01;
        const CascadeBadge = 0x02;
        const ThunderBadge = 0x04;
        const RainbowBadge = 0x08;
        const SoulBadge = 0x10;
        const MarshBadge = 0x20;
        const VolcanoBadge = 0x40;
        const EarthBadge = 0x80;
    }
}

impl Display for Badge {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        bitflags::parser::to_writer(self, f)
    }
}
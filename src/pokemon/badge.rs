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

impl Badge {
    /// The badges in bit order, which is the order the game awards them, the order they are laid out
    /// on the trainer card, and the order their graphics appear in the ROM
    /// ([`crate::pokemon::badge_gfx`]). Index `i` is bit `i` — `badges_are_declared_in_bit_order`
    /// pins that, because a UI that lights badge 3 for bit 4 would look entirely plausible.
    pub const ORDER: [Badge; 8] = [
        Badge::BoulderBadge,
        Badge::CascadeBadge,
        Badge::ThunderBadge,
        Badge::RainbowBadge,
        Badge::SoulBadge,
        Badge::MarshBadge,
        Badge::VolcanoBadge,
        Badge::EarthBadge,
    ];
}

impl Display for Badge {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        bitflags::parser::to_writer(self, f)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_are_declared_in_bit_order() {
        for (index, badge) in Badge::ORDER.iter().enumerate() {
            assert_eq!(badge.bits(), 1 << index, "{badge} is not bit {index}");
        }
    }

    /// The name the UI labels a badge with is the flag's own name, so it is worth one assertion that
    /// `Display` gives a single flag its bare name rather than a set-shaped rendering.
    #[test]
    fn a_single_flag_displays_as_its_name() {
        assert_eq!(format!("{}", Badge::BoulderBadge), "BoulderBadge");
        assert_eq!(format!("{}", Badge::EarthBadge), "EarthBadge");
    }
}

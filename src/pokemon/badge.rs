use strum::IntoEnumIterator;

#[derive(Debug, Clone, Copy, Eq, PartialEq, strum_macros::Display, strum_macros::EnumIter)]
#[repr(u8)]
pub enum Badge {
    BoulderBadge = 0x01,
    CascadeBadge = 0x02,
    ThunderBadge = 0x04,
    RainbowBadge = 0x08,
    SoulBadge = 0x10,
    MarshBadge = 0x20,
    VolcanoBadge = 0x40,
    EarthBadge = 0x80,
}

impl Badge {
    pub fn parse_flags(flags: u8) -> Vec<Badge> {
        let mut badges = Vec::new();
        for badge in Badge::iter() {
            if flags & (badge as u8) != 0 {
                badges.push(badge);
            }
        }
        badges
    }
}
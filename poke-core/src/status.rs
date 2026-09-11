use bitflags::bitflags;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct PokemonStatusFlags: u8 {
        const Paralyzed = 0b0100_0000; // bit 6
        const Frozen = 0b0010_0000; // bit 5
        const Burned = 0b0001_0000; // bit 4
        const Poisoned = 0b0000_1000; // bit 3
        const Sleep2 = 0b0000_0100; // bit 2
        const Sleep1 = 0b0000_0010; // bit 1
        const Sleep0 = 0b0000_0001; // bit 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, strum_macros::Display)]
pub enum PokemonStatus {
    #[default]
    None,
    Paralyzed,
    Frozen,
    Burned,
    Poisoned,
    Asleep { counter: u8 },
}

impl From<PokemonStatusFlags> for PokemonStatus {
    fn from(value: PokemonStatusFlags) -> Self {
        if value.contains(PokemonStatusFlags::Paralyzed) {
            PokemonStatus::Paralyzed
        } else if value.contains(PokemonStatusFlags::Frozen) {
            PokemonStatus::Frozen
        } else if value.contains(PokemonStatusFlags::Burned) {
            PokemonStatus::Burned
        } else if value.contains(PokemonStatusFlags::Poisoned) {
            PokemonStatus::Poisoned
        } else if value.intersects(PokemonStatusFlags::Sleep2 | PokemonStatusFlags::Sleep1 | PokemonStatusFlags::Sleep0) {
            PokemonStatus::Asleep { counter: value.bits() & 0b111 }
        } else {
            PokemonStatus::None
        }
    }
}

impl From<u8> for PokemonStatus {
    /// Truncating, because the byte is read from a running game and not every bit of it is a
    /// status.
    fn from(value: u8) -> Self {
        PokemonStatusFlags::from_bits_truncate(value).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every byte a running game can put there decodes to something.
    #[test]
    fn every_status_byte_decodes_without_panicking() {
        for byte in 0..=u8::MAX {
            let status = PokemonStatus::from(byte);
            // The four exclusive statuses win over sleep, and sleep reports its counter; nothing
            // else is representable, so this is really asserting "it returned".
            if byte & 0b0100_0000 != 0 {
                assert_eq!(status, PokemonStatus::Paralyzed, "{byte:#010b}");
            }
        }
        assert_eq!(PokemonStatus::from(0b1000_0000), PokemonStatus::None, "an unmodelled bit alone");
        assert_eq!(PokemonStatus::from(0b1000_0011), PokemonStatus::Asleep { counter: 3 },
                   "an unmodelled bit beside a sleep counter");
    }
}

impl Into<PokemonStatusFlags> for PokemonStatus {
    fn into(self) -> PokemonStatusFlags {
        match self {
            PokemonStatus::None => PokemonStatusFlags::empty(),
            PokemonStatus::Paralyzed => PokemonStatusFlags::Paralyzed,
            PokemonStatus::Frozen => PokemonStatusFlags::Frozen,
            PokemonStatus::Burned => PokemonStatusFlags::Burned,
            PokemonStatus::Poisoned => PokemonStatusFlags::Poisoned,
            PokemonStatus::Asleep { counter } => PokemonStatusFlags::from_bits(counter & 0b111).unwrap(),
        }
    }
}

impl Into<u8> for PokemonStatus {
    fn into(self) -> u8 {
        let flags: PokemonStatusFlags = self.into();
        flags.bits()
    }
}
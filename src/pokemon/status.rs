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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    fn from(value: u8) -> Self {
        PokemonStatusFlags::from_bits(value).unwrap().into()
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
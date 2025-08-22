use strum::IntoEnumIterator;
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::FromRepr, strum_macros::EnumIter)]
#[repr(u8)]
pub enum Channel {
    Channel1 = 1,
    Channel2 = 2,
    Channel3 = 3,
    Channel4 = 4,
}

impl Channel {
    pub fn all() -> impl Iterator<Item = Self> {
        Self::iter()
    }
}


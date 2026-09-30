pub use crate::pointer::{DmgBank, DmgPointer};

include!(concat!(env!("OUT_DIR"), "/constants.rs"));

/// A label as a save holds it: by name, or by the address a save written before the runtime held
/// names has in its place.
#[derive(serde::Deserialize)]
#[serde(untagged)]
pub enum SavedLabel {
    Name(String),
    Address(DmgPointer),
}

impl SavedLabel {
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Name(name) => Some(name),
            Self::Address(at) => crate::saved_addresses::SAVED_ADDRESSES.iter()
                .find(|(address, _)| address == at)
                .map(|(_, name)| *name),
        }
    }

    /// The value `named` finds for the label, or an error naming what was saved.
    pub fn resolve<'de, T, D: serde::Deserializer<'de>>(deserializer: D, named: impl Fn(&str) -> Option<T>) -> Result<T, D::Error> {
        let saved = <Self as serde::Deserialize>::deserialize(deserializer)?;
        saved.name().and_then(named).ok_or_else(|| serde::de::Error::custom(match saved {
            Self::Name(name) => format!("nothing is labelled {name}"),
            Self::Address(at) => format!("no label here is at {at}"),
        }))
    }
}

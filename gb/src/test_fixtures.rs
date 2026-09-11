//! The Pokémon Red cartridge and a couple of its save states, for the emulator tests that need a
//! real commercial game rather than a test ROM. They live in `poke-agent`, which is the crate
//! that builds the cartridge; this module is `#[cfg(test)]`, so a published `gb` reaches outside
//! its own directory for nothing.

pub const POKERED: &[u8] = include_bytes!("../../vendor/pokered/pokered.gbc");

/// A mid-game save: standing in the Celadon overworld with music playing.
pub const AT_CELADON: &[u8] = include_bytes!("../../poke-agent/src/pokemon/data/at-celadon.bin");

/// Every committed save state, for the round-trip test that pins the section layout.
pub fn fixture_dir() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../poke-agent/src/pokemon/data")
}

#[cfg(feature = "slow-tests")]
/// Three values read out of `pokered.sym`, so the benchmark fixture can be checked for being the
/// game it claims to be without `gb` depending on the agent's generated symbol table.
pub mod symbols {
    pub const W_CUR_MAP: u16 = 0xd35e;
    pub const W_IS_IN_BATTLE: u16 = 0xd057;
    /// `Map::CeladonCity`, the map `AT_CELADON` stands in.
    pub const CELADON_CITY: u8 = 6;
}

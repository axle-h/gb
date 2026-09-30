//! The sound data, assembled by `build.rs` from `audio.asm`, and the tables that name a sound by its
//! header. `poke-agent`'s `rom_equality` checks every byte against the cartridge.

/// One of the three copies of the sound data: the header tables, the sound effects with the wave
/// samples, the engine's `CryRet` and the music, laid out from `$4000` in the cartridge's order.
/// Everything up to the engine's code is at the cartridge's own address; the music is not.
#[derive(Debug)]
pub struct AudioBankData {
    /// The ROM bank `layout.link` puts this copy in, which is what `BANK()` of a sound is.
    pub rom_bank: u8,
    /// From `$4000`.
    pub bytes: &'static [u8],
    /// Every label, a local one as `Global.local`, in address order.
    pub labels: &'static [(&'static str, u16)],
    /// Where each `dw` of a label sits.
    pub pointers: &'static [u16],
}

impl AudioBankData {
    pub fn label(&self, name: &str) -> Option<u16> {
        self.labels.iter().find(|(label, _)| *label == name).map(|&(_, at)| at)
    }
}

// `PITCHES` is `audio/notes.asm`, the twelve notes at octave 7, which every copy includes. `sound`
// is `constants/music_constants.asm`. `CRY_DATA` is `(sound, pitch, length)` with the base cry's
// own id, and `MAP_SONG_BANKS` and `POKEDEX_RATING_SFX` are `(sound, ROM bank)`.
include!(concat!(env!("OUT_DIR"), "/audio.rs"));

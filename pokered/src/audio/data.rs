//! The cartridge's audio data: the three parallel copies of the header table, the command streams,
//! the wave samples and the pitch table, plus the per-species cry modifiers.
//!
//! Recreates `audio/headers/{sfx,music}headers(1|2|3).asm`, `audio/notes.asm`,
//! `audio/wave_samples.asm` and `data/pokemon/cries.asm`, as `poke_core::audio` assembles them.
//! Nothing here computes anything: it is the data read the way `AudioN_PlaySound`,
//! `AudioN_GetNextMusicByte`, `AudioN_CalculateFrequency` and `AudioN_ApplyWavePatternAndFrequency`
//! read it.
//!
//! A sound id is an index into `SFX_Headers_(1|2|3)` at three bytes a row, which is exactly what
//! `music_const` computes, so music and sound effects share one numbering. The three tables all
//! start at `$4000` of their own bank, so the same id names the same sound in each — where the
//! bank has one.

use poke_core::audio::{sound, AudioBankData, AUDIO_BANKS, CRY_DATA, PITCHES};
use serde::{Deserialize, Serialize};
use crate::world::Ruleset;

/// Where `SFX_Headers_(1|2|3)` sits in each of the three banks, and what `music_const` subtracts.
const HEADERS: u16 = 0x4000;

/// `AUDIO_1`, `AUDIO_2` and `AUDIO_3`: the three copies of the engine, each a ROM bank carrying its
/// own header table, music, sound effects, wave samples and pitch table. `wAudioROMBank` names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioBank {
    One,
    Two,
    Three,
}

impl AudioBank {
    pub const ALL: [AudioBank; 3] = [AudioBank::One, AudioBank::Two, AudioBank::Three];

    fn data(self) -> &'static AudioBankData {
        &AUDIO_BANKS[self as usize]
    }

    pub fn rom_bank(self) -> u8 {
        self.data().rom_bank
    }

    pub fn from_rom_bank(bank: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|audio| audio.rom_bank() == bank)
    }

    /// The bank whose data holds `label`, as `BANK()` of it names.
    pub fn holding(label: &str) -> Self {
        Self::ALL.into_iter().find(|bank| bank.data().label(label).is_some()).unwrap_or_else(|| panic!("no audio bank holds {label}"))
    }

    /// Where `label` is in this bank, to point a channel at. Everything before the engine's code is
    /// at the cartridge's address; the music is not.
    pub fn label(self, label: &str) -> u16 {
        self.data().label(label).unwrap_or_else(|| panic!("{self:?} has no {label}"))
    }

    /// `MAX_SFX_ID_(1|2|3)`: at or below this an id is a sound effect, above it music.
    pub const fn max_sfx_id(self) -> SoundId {
        match self {
            AudioBank::One => SoundId(sound::MAX_SFX_ID_1),
            AudioBank::Two => SoundId(sound::MAX_SFX_ID_2),
            AudioBank::Three => SoundId(sound::MAX_SFX_ID_3),
        }
    }

    /// One byte of the bank, as `AudioN_GetNextMusicByte` reads it.
    pub fn byte(self, address: u16) -> u8 {
        let at = (address as usize).checked_sub(0x4000).expect("an audio pointer is in the bank window");
        *self.data().bytes.get(at).unwrap_or_else(|| panic!("{self:?} has no data at {address:04X}"))
    }

    /// Every sound the bank's table names, found by walking it: a header claims one row per
    /// channel, so the next sound's id is this one's plus its channel count. Row 0 is three `$ff`
    /// bytes of padding rather than a sound, and the walk ends with the last song because the table
    /// ends there.
    pub fn sound_ids(self) -> Vec<SoundId> {
        let mut ids = Vec::new();
        let mut id = 1u16;
        while id <= u8::MAX as u16 - 1 {
            ids.push(SoundId(id as u8));
            id += (self.byte(HEADERS + id * 3) >> 6) as u16 + 1;
        }
        ids
    }

    /// `AudioN_Pitches`: the frequency of each of the twelve notes at octave 7. The table is
    /// `audio/notes.asm` included three times, so all three banks hold the same twelve.
    pub fn pitch(self, note: u8) -> u16 {
        PITCHES[note as usize]
    }

    /// One of the nine `AudioN_WavePointers`, as the sixteen bytes copied into wave RAM.
    ///
    /// Pointers 5 to 8 name a label with no data under it, so the cartridge reads the sixteen bytes
    /// of whatever sound effect its bank stores next, a different instrument in each copy. Only
    /// Lavender Town and the Pokémon Tower play it, the tower's theme an arrangement of the town's,
    /// so all three banks play [`LAVENDER_WAVE`] unless Gen 1 asks for the overrun, which
    /// reads the next sound effect's bytes as the cartridge does.
    pub fn wave_sample(self, instrument: u8, ruleset: Ruleset) -> [u8; 16] {
        use poke_core::tables::{WAVE_POINTERS, WAVE_SAMPLES};
        if !ruleset.is_gen1() {
            let wave = WAVE_POINTERS[instrument as usize] as usize;
            return WAVE_SAMPLES.get(wave).copied().unwrap_or(LAVENDER_WAVE);
        }
        let pointers = self.label(["Audio1_WavePointers", "Audio2_WavePointers", "Audio3_WavePointers"][self as usize]);
        let row = pointers + instrument as u16 * 2;
        let at = u16::from_le_bytes([self.byte(row), self.byte(row + 1)]);
        std::array::from_fn(|i| self.byte(at + i as u16))
    }
}

/// The sixteen bytes the first bank's copy reads for its empty wave: the instrument Lavender Town
/// is heard with.
pub const LAVENDER_WAVE: [u8; 16] = [0x21, 0xE2, 0x33, 0x28, 0xE1, 0x22, 0xFF, 0xEA, 0x10, 0x14, 0xDC, 0x10, 0xE3, 0x41, 0x51, 0x73];

/// An index into `SFX_Headers_(1|2|3)`, three bytes a row: what `music_const` computes and what
/// `PlaySound` is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SoundId(pub u8);

impl SoundId {
    /// `SFX_STOP_ALL_MUSIC`, which silences everything rather than naming a header.
    pub const STOP_ALL_MUSIC: SoundId = SoundId(sound::SFX_STOP_ALL_MUSIC);
}

/// A piece of music, which is only playable with the bank it lives in: `PlayMusic` takes both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sound {
    pub bank: AudioBank,
    pub id: SoundId,
}

/// `NOISE_INSTRUMENTS_END`: below this a noise channel sound is a drum rather than a sound effect,
/// which is what lets a drum interrupt one sound effect but not another.
pub const NOISE_INSTRUMENTS_END: SoundId = SoundId(sound::NOISE_INSTRUMENTS_END);
/// `CRY_SFX_START`, the first of the thirty-eight base cries.
pub const CRY_SFX_START: SoundId = SoundId(sound::CRY_SFX_START);
/// `CRY_SFX_END`. Three past the last cry, because a cry header claims three channels.
pub const CRY_SFX_END: SoundId = SoundId(sound::CRY_SFX_END);
/// `BATTLE_SFX_START` and `BATTLE_SFX_END`, which only `AUDIO_2` tests.
pub const BATTLE_SFX_START: SoundId = SoundId(sound::BATTLE_SFX_START);
pub const BATTLE_SFX_END: SoundId = SoundId(sound::BATTLE_SFX_END);

/// One `channel` entry of a header: which of the eight software channels, and where its commands
/// start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderChannel {
    pub channel: usize,
    pub address: u16,
}

/// One row of `SFX_Headers_(1|2|3)`: `channel_count` in the top two bits of the first byte, then a
/// `channel` entry every three bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundHeader {
    pub channels: Vec<HeaderChannel>,
}

impl SoundHeader {
    pub fn read(bank: AudioBank, id: SoundId) -> Self {
        let row = |k: usize| HEADERS + id.0 as u16 * 3 + k as u16 * 3;
        let count = (bank.byte(row(0)) >> 6) as usize + 1;
        let channels = (0..count)
            .map(|k| HeaderChannel {
                channel: (bank.byte(row(k)) & 0x0F) as usize,
                address: u16::from_le_bytes([bank.byte(row(k) + 1), bank.byte(row(k) + 2)]),
            })
            .collect();
        Self { channels }
    }
}

/// `GetCryData`: a species' base cry and the two modifiers its pitch and length are bent by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cry {
    pub sound: SoundId,
    /// `wFrequencyModifier`, added to every frequency the cry plays.
    pub frequency_modifier: u8,
    /// `wTempoModifier`, which becomes the sfx tempo offset from `$80`.
    pub tempo_modifier: u8,
}

impl Cry {
    /// `index` is the internal species index the cartridge stores, one-based.
    pub fn of_species_index(index: u8) -> Self {
        let (sound, frequency_modifier, tempo_modifier) = CRY_DATA[index as usize - 1];
        Self { sound: SoundId(sound), frequency_modifier, tempo_modifier }
    }
}

/// Every music and sound effect the cartridge names, as `constants/music_constants.asm` numbers
/// them. A sound effect is an id alone, because the three banks each hold their own copy of it; a
/// piece of music carries the bank it is stored in, which is what `PlayMusic` is passed.
#[allow(dead_code)]
pub mod sounds {
    use super::{AudioBank, Sound, SoundId};
    use poke_core::audio::sound;

    pub const MUSIC_PALLET_TOWN: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_PALLET_TOWN) };
    pub const MUSIC_POKECENTER: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_POKECENTER) };
    pub const MUSIC_GYM: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_GYM) };
    pub const MUSIC_CITIES1: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_CITIES1) };
    pub const MUSIC_CITIES2: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_CITIES2) };
    pub const MUSIC_CELADON: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_CELADON) };
    pub const MUSIC_CINNABAR: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_CINNABAR) };
    pub const MUSIC_VERMILION: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_VERMILION) };
    pub const MUSIC_LAVENDER: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_LAVENDER) };
    pub const MUSIC_SS_ANNE: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_SS_ANNE) };
    pub const MUSIC_MEET_PROF_OAK: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_MEET_PROF_OAK) };
    pub const MUSIC_MEET_RIVAL: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_MEET_RIVAL) };
    pub const MUSIC_MUSEUM_GUY: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_MUSEUM_GUY) };
    pub const MUSIC_SAFARI_ZONE: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_SAFARI_ZONE) };
    pub const MUSIC_PKMN_HEALED: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_PKMN_HEALED) };
    pub const MUSIC_ROUTES1: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_ROUTES1) };
    pub const MUSIC_ROUTES2: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_ROUTES2) };
    pub const MUSIC_ROUTES3: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_ROUTES3) };
    pub const MUSIC_ROUTES4: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_ROUTES4) };
    pub const MUSIC_INDIGO_PLATEAU: Sound = Sound { bank: AudioBank::One, id: SoundId(sound::MUSIC_INDIGO_PLATEAU) };
    pub const MUSIC_GYM_LEADER_BATTLE: Sound = Sound { bank: AudioBank::Two, id: SoundId(sound::MUSIC_GYM_LEADER_BATTLE) };
    pub const MUSIC_TRAINER_BATTLE: Sound = Sound { bank: AudioBank::Two, id: SoundId(sound::MUSIC_TRAINER_BATTLE) };
    pub const MUSIC_WILD_BATTLE: Sound = Sound { bank: AudioBank::Two, id: SoundId(sound::MUSIC_WILD_BATTLE) };
    pub const MUSIC_FINAL_BATTLE: Sound = Sound { bank: AudioBank::Two, id: SoundId(sound::MUSIC_FINAL_BATTLE) };
    pub const MUSIC_DEFEATED_TRAINER: Sound = Sound { bank: AudioBank::Two, id: SoundId(sound::MUSIC_DEFEATED_TRAINER) };
    pub const MUSIC_DEFEATED_WILD_MON: Sound = Sound { bank: AudioBank::Two, id: SoundId(sound::MUSIC_DEFEATED_WILD_MON) };
    pub const MUSIC_DEFEATED_GYM_LEADER: Sound = Sound { bank: AudioBank::Two, id: SoundId(sound::MUSIC_DEFEATED_GYM_LEADER) };
    pub const MUSIC_TITLE_SCREEN: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_TITLE_SCREEN) };
    pub const MUSIC_CREDITS: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_CREDITS) };
    pub const MUSIC_HALL_OF_FAME: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_HALL_OF_FAME) };
    pub const MUSIC_OAKS_LAB: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_OAKS_LAB) };
    pub const MUSIC_JIGGLYPUFF_SONG: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_JIGGLYPUFF_SONG) };
    pub const MUSIC_BIKE_RIDING: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_BIKE_RIDING) };
    pub const MUSIC_SURFING: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_SURFING) };
    pub const MUSIC_GAME_CORNER: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_GAME_CORNER) };
    pub const MUSIC_INTRO_BATTLE: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_INTRO_BATTLE) };
    pub const MUSIC_DUNGEON1: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_DUNGEON1) };
    pub const MUSIC_DUNGEON2: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_DUNGEON2) };
    pub const MUSIC_DUNGEON3: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_DUNGEON3) };
    pub const MUSIC_CINNABAR_MANSION: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_CINNABAR_MANSION) };
    pub const MUSIC_POKEMON_TOWER: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_POKEMON_TOWER) };
    pub const MUSIC_SILPH_CO: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_SILPH_CO) };
    pub const MUSIC_MEET_EVIL_TRAINER: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_MEET_EVIL_TRAINER) };
    pub const MUSIC_MEET_FEMALE_TRAINER: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_MEET_FEMALE_TRAINER) };
    pub const MUSIC_MEET_MALE_TRAINER: Sound = Sound { bank: AudioBank::Three, id: SoundId(sound::MUSIC_MEET_MALE_TRAINER) };
    pub const SFX_NOISE_INSTRUMENT01: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT01);
    pub const SFX_NOISE_INSTRUMENT02: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT02);
    pub const SFX_NOISE_INSTRUMENT03: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT03);
    pub const SFX_NOISE_INSTRUMENT04: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT04);
    pub const SFX_NOISE_INSTRUMENT05: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT05);
    pub const SFX_NOISE_INSTRUMENT06: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT06);
    pub const SFX_NOISE_INSTRUMENT07: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT07);
    pub const SFX_NOISE_INSTRUMENT08: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT08);
    pub const SFX_NOISE_INSTRUMENT09: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT09);
    pub const SFX_NOISE_INSTRUMENT10: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT10);
    pub const SFX_NOISE_INSTRUMENT11: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT11);
    pub const SFX_NOISE_INSTRUMENT12: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT12);
    pub const SFX_NOISE_INSTRUMENT13: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT13);
    pub const SFX_NOISE_INSTRUMENT14: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT14);
    pub const SFX_NOISE_INSTRUMENT15: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT15);
    pub const SFX_NOISE_INSTRUMENT16: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT16);
    pub const SFX_NOISE_INSTRUMENT17: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT17);
    pub const SFX_NOISE_INSTRUMENT18: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT18);
    pub const SFX_NOISE_INSTRUMENT19: SoundId = SoundId(sound::SFX_NOISE_INSTRUMENT19);
    pub const SFX_CRY_00: SoundId = SoundId(sound::SFX_CRY_00);
    pub const SFX_CRY_01: SoundId = SoundId(sound::SFX_CRY_01);
    pub const SFX_CRY_02: SoundId = SoundId(sound::SFX_CRY_02);
    pub const SFX_CRY_03: SoundId = SoundId(sound::SFX_CRY_03);
    pub const SFX_CRY_04: SoundId = SoundId(sound::SFX_CRY_04);
    pub const SFX_CRY_05: SoundId = SoundId(sound::SFX_CRY_05);
    pub const SFX_CRY_06: SoundId = SoundId(sound::SFX_CRY_06);
    pub const SFX_CRY_07: SoundId = SoundId(sound::SFX_CRY_07);
    pub const SFX_CRY_08: SoundId = SoundId(sound::SFX_CRY_08);
    pub const SFX_CRY_09: SoundId = SoundId(sound::SFX_CRY_09);
    pub const SFX_CRY_0A: SoundId = SoundId(sound::SFX_CRY_0A);
    pub const SFX_CRY_0B: SoundId = SoundId(sound::SFX_CRY_0B);
    pub const SFX_CRY_0C: SoundId = SoundId(sound::SFX_CRY_0C);
    pub const SFX_CRY_0D: SoundId = SoundId(sound::SFX_CRY_0D);
    pub const SFX_CRY_0E: SoundId = SoundId(sound::SFX_CRY_0E);
    pub const SFX_CRY_0F: SoundId = SoundId(sound::SFX_CRY_0F);
    pub const SFX_CRY_10: SoundId = SoundId(sound::SFX_CRY_10);
    pub const SFX_CRY_11: SoundId = SoundId(sound::SFX_CRY_11);
    pub const SFX_CRY_12: SoundId = SoundId(sound::SFX_CRY_12);
    pub const SFX_CRY_13: SoundId = SoundId(sound::SFX_CRY_13);
    pub const SFX_CRY_14: SoundId = SoundId(sound::SFX_CRY_14);
    pub const SFX_CRY_15: SoundId = SoundId(sound::SFX_CRY_15);
    pub const SFX_CRY_16: SoundId = SoundId(sound::SFX_CRY_16);
    pub const SFX_CRY_17: SoundId = SoundId(sound::SFX_CRY_17);
    pub const SFX_CRY_18: SoundId = SoundId(sound::SFX_CRY_18);
    pub const SFX_CRY_19: SoundId = SoundId(sound::SFX_CRY_19);
    pub const SFX_CRY_1A: SoundId = SoundId(sound::SFX_CRY_1A);
    pub const SFX_CRY_1B: SoundId = SoundId(sound::SFX_CRY_1B);
    pub const SFX_CRY_1C: SoundId = SoundId(sound::SFX_CRY_1C);
    pub const SFX_CRY_1D: SoundId = SoundId(sound::SFX_CRY_1D);
    pub const SFX_CRY_1E: SoundId = SoundId(sound::SFX_CRY_1E);
    pub const SFX_CRY_1F: SoundId = SoundId(sound::SFX_CRY_1F);
    pub const SFX_CRY_20: SoundId = SoundId(sound::SFX_CRY_20);
    pub const SFX_CRY_21: SoundId = SoundId(sound::SFX_CRY_21);
    pub const SFX_CRY_22: SoundId = SoundId(sound::SFX_CRY_22);
    pub const SFX_CRY_23: SoundId = SoundId(sound::SFX_CRY_23);
    pub const SFX_CRY_24: SoundId = SoundId(sound::SFX_CRY_24);
    pub const SFX_CRY_25: SoundId = SoundId(sound::SFX_CRY_25);
    pub const SFX_GET_ITEM_2: SoundId = SoundId(sound::SFX_GET_ITEM_2);
    pub const SFX_TINK: SoundId = SoundId(sound::SFX_TINK);
    pub const SFX_HEAL_HP: SoundId = SoundId(sound::SFX_HEAL_HP);
    pub const SFX_HEAL_AILMENT: SoundId = SoundId(sound::SFX_HEAL_AILMENT);
    pub const SFX_START_MENU: SoundId = SoundId(sound::SFX_START_MENU);
    pub const SFX_PRESS_AB: SoundId = SoundId(sound::SFX_PRESS_AB);
    pub const SFX_GET_ITEM_1: SoundId = SoundId(sound::SFX_GET_ITEM_1);
    pub const SFX_POKEDEX_RATING: SoundId = SoundId(sound::SFX_POKEDEX_RATING);
    pub const SFX_GET_KEY_ITEM: SoundId = SoundId(sound::SFX_GET_KEY_ITEM);
    pub const SFX_POISONED: SoundId = SoundId(sound::SFX_POISONED);
    pub const SFX_TRADE_MACHINE: SoundId = SoundId(sound::SFX_TRADE_MACHINE);
    pub const SFX_TURN_ON_PC: SoundId = SoundId(sound::SFX_TURN_ON_PC);
    pub const SFX_TURN_OFF_PC: SoundId = SoundId(sound::SFX_TURN_OFF_PC);
    pub const SFX_ENTER_PC: SoundId = SoundId(sound::SFX_ENTER_PC);
    pub const SFX_SHRINK: SoundId = SoundId(sound::SFX_SHRINK);
    pub const SFX_SWITCH: SoundId = SoundId(sound::SFX_SWITCH);
    pub const SFX_HEALING_MACHINE: SoundId = SoundId(sound::SFX_HEALING_MACHINE);
    pub const SFX_TELEPORT_EXIT_1: SoundId = SoundId(sound::SFX_TELEPORT_EXIT_1);
    pub const SFX_TELEPORT_ENTER_1: SoundId = SoundId(sound::SFX_TELEPORT_ENTER_1);
    pub const SFX_TELEPORT_EXIT_2: SoundId = SoundId(sound::SFX_TELEPORT_EXIT_2);
    pub const SFX_LEDGE: SoundId = SoundId(sound::SFX_LEDGE);
    pub const SFX_TELEPORT_ENTER_2: SoundId = SoundId(sound::SFX_TELEPORT_ENTER_2);
    pub const SFX_FLY: SoundId = SoundId(sound::SFX_FLY);
    pub const SFX_DENIED: SoundId = SoundId(sound::SFX_DENIED);
    pub const SFX_ARROW_TILES: SoundId = SoundId(sound::SFX_ARROW_TILES);
    pub const SFX_PUSH_BOULDER: SoundId = SoundId(sound::SFX_PUSH_BOULDER);
    pub const SFX_SS_ANNE_HORN: SoundId = SoundId(sound::SFX_SS_ANNE_HORN);
    pub const SFX_WITHDRAW_DEPOSIT: SoundId = SoundId(sound::SFX_WITHDRAW_DEPOSIT);
    pub const SFX_CUT: SoundId = SoundId(sound::SFX_CUT);
    pub const SFX_GO_INSIDE: SoundId = SoundId(sound::SFX_GO_INSIDE);
    pub const SFX_SWAP: SoundId = SoundId(sound::SFX_SWAP);
    pub const SFX_59: SoundId = SoundId(sound::SFX_59);
    pub const SFX_PURCHASE: SoundId = SoundId(sound::SFX_PURCHASE);
    pub const SFX_COLLISION: SoundId = SoundId(sound::SFX_COLLISION);
    pub const SFX_GO_OUTSIDE: SoundId = SoundId(sound::SFX_GO_OUTSIDE);
    pub const SFX_SAVE: SoundId = SoundId(sound::SFX_SAVE);
    pub const SFX_POKEFLUTE: SoundId = SoundId(sound::SFX_POKEFLUTE);
    pub const SFX_SAFARI_ZONE_PA: SoundId = SoundId(sound::SFX_SAFARI_ZONE_PA);
    pub const SFX_LEVEL_UP: SoundId = SoundId(sound::SFX_LEVEL_UP);
    pub const SFX_BALL_TOSS: SoundId = SoundId(sound::SFX_BALL_TOSS);
    pub const SFX_BALL_POOF: SoundId = SoundId(sound::SFX_BALL_POOF);
    pub const SFX_FAINT_THUD: SoundId = SoundId(sound::SFX_FAINT_THUD);
    pub const SFX_RUN: SoundId = SoundId(sound::SFX_RUN);
    pub const SFX_DEX_PAGE_ADDED: SoundId = SoundId(sound::SFX_DEX_PAGE_ADDED);
    pub const SFX_CAUGHT_MON: SoundId = SoundId(sound::SFX_CAUGHT_MON);
    pub const SFX_PECK: SoundId = SoundId(sound::SFX_PECK);
    pub const SFX_FAINT_FALL: SoundId = SoundId(sound::SFX_FAINT_FALL);
    pub const SFX_BATTLE_09: SoundId = SoundId(sound::SFX_BATTLE_09);
    pub const SFX_POUND: SoundId = SoundId(sound::SFX_POUND);
    pub const SFX_BATTLE_0B: SoundId = SoundId(sound::SFX_BATTLE_0B);
    pub const SFX_BATTLE_0C: SoundId = SoundId(sound::SFX_BATTLE_0C);
    pub const SFX_BATTLE_0D: SoundId = SoundId(sound::SFX_BATTLE_0D);
    pub const SFX_BATTLE_0E: SoundId = SoundId(sound::SFX_BATTLE_0E);
    pub const SFX_BATTLE_0F: SoundId = SoundId(sound::SFX_BATTLE_0F);
    pub const SFX_DAMAGE: SoundId = SoundId(sound::SFX_DAMAGE);
    pub const SFX_NOT_VERY_EFFECTIVE: SoundId = SoundId(sound::SFX_NOT_VERY_EFFECTIVE);
    pub const SFX_BATTLE_12: SoundId = SoundId(sound::SFX_BATTLE_12);
    pub const SFX_BATTLE_13: SoundId = SoundId(sound::SFX_BATTLE_13);
    pub const SFX_BATTLE_14: SoundId = SoundId(sound::SFX_BATTLE_14);
    pub const SFX_VINE_WHIP: SoundId = SoundId(sound::SFX_VINE_WHIP);
    pub const SFX_BATTLE_16: SoundId = SoundId(sound::SFX_BATTLE_16);
    pub const SFX_BATTLE_17: SoundId = SoundId(sound::SFX_BATTLE_17);
    pub const SFX_BATTLE_18: SoundId = SoundId(sound::SFX_BATTLE_18);
    pub const SFX_BATTLE_19: SoundId = SoundId(sound::SFX_BATTLE_19);
    pub const SFX_SUPER_EFFECTIVE: SoundId = SoundId(sound::SFX_SUPER_EFFECTIVE);
    pub const SFX_BATTLE_1B: SoundId = SoundId(sound::SFX_BATTLE_1B);
    pub const SFX_BATTLE_1C: SoundId = SoundId(sound::SFX_BATTLE_1C);
    pub const SFX_DOUBLESLAP: SoundId = SoundId(sound::SFX_DOUBLESLAP);
    pub const SFX_BATTLE_1E: SoundId = SoundId(sound::SFX_BATTLE_1E);
    pub const SFX_HORN_DRILL: SoundId = SoundId(sound::SFX_HORN_DRILL);
    pub const SFX_BATTLE_20: SoundId = SoundId(sound::SFX_BATTLE_20);
    pub const SFX_BATTLE_21: SoundId = SoundId(sound::SFX_BATTLE_21);
    pub const SFX_BATTLE_22: SoundId = SoundId(sound::SFX_BATTLE_22);
    pub const SFX_BATTLE_23: SoundId = SoundId(sound::SFX_BATTLE_23);
    pub const SFX_BATTLE_24: SoundId = SoundId(sound::SFX_BATTLE_24);
    pub const SFX_BATTLE_25: SoundId = SoundId(sound::SFX_BATTLE_25);
    pub const SFX_BATTLE_26: SoundId = SoundId(sound::SFX_BATTLE_26);
    pub const SFX_BATTLE_27: SoundId = SoundId(sound::SFX_BATTLE_27);
    pub const SFX_BATTLE_28: SoundId = SoundId(sound::SFX_BATTLE_28);
    pub const SFX_BATTLE_29: SoundId = SoundId(sound::SFX_BATTLE_29);
    pub const SFX_BATTLE_2A: SoundId = SoundId(sound::SFX_BATTLE_2A);
    pub const SFX_BATTLE_2B: SoundId = SoundId(sound::SFX_BATTLE_2B);
    pub const SFX_BATTLE_2C: SoundId = SoundId(sound::SFX_BATTLE_2C);
    pub const SFX_PSYBEAM: SoundId = SoundId(sound::SFX_PSYBEAM);
    pub const SFX_BATTLE_2E: SoundId = SoundId(sound::SFX_BATTLE_2E);
    pub const SFX_BATTLE_2F: SoundId = SoundId(sound::SFX_BATTLE_2F);
    pub const SFX_PSYCHIC_M: SoundId = SoundId(sound::SFX_PSYCHIC_M);
    pub const SFX_BATTLE_31: SoundId = SoundId(sound::SFX_BATTLE_31);
    pub const SFX_BATTLE_32: SoundId = SoundId(sound::SFX_BATTLE_32);
    pub const SFX_BATTLE_33: SoundId = SoundId(sound::SFX_BATTLE_33);
    pub const SFX_BATTLE_34: SoundId = SoundId(sound::SFX_BATTLE_34);
    pub const SFX_BATTLE_35: SoundId = SoundId(sound::SFX_BATTLE_35);
    pub const SFX_BATTLE_36: SoundId = SoundId(sound::SFX_BATTLE_36);
    pub const SFX_TRAINER_APPEARED: SoundId = SoundId(sound::SFX_TRAINER_APPEARED);
    pub const SFX_INTRO_LUNGE: SoundId = SoundId(sound::SFX_INTRO_LUNGE);
    pub const SFX_INTRO_HIP: SoundId = SoundId(sound::SFX_INTRO_HIP);
    pub const SFX_INTRO_HOP: SoundId = SoundId(sound::SFX_INTRO_HOP);
    pub const SFX_INTRO_RAISE: SoundId = SoundId(sound::SFX_INTRO_RAISE);
    pub const SFX_INTRO_CRASH: SoundId = SoundId(sound::SFX_INTRO_CRASH);
    pub const SFX_INTRO_WHOOSH: SoundId = SoundId(sound::SFX_INTRO_WHOOSH);
    pub const SFX_SLOTS_STOP_WHEEL: SoundId = SoundId(sound::SFX_SLOTS_STOP_WHEEL);
    pub const SFX_SLOTS_REWARD: SoundId = SoundId(sound::SFX_SLOTS_REWARD);
    pub const SFX_SLOTS_NEW_SPIN: SoundId = SoundId(sound::SFX_SLOTS_NEW_SPIN);
    pub const SFX_SHOOTING_STAR: SoundId = SoundId(sound::SFX_SHOOTING_STAR);
}

#[cfg(test)]
mod tests {
    use super::*;
    use poke_core::tables::WAVE_POINTERS;

    #[test]
    fn the_three_banks_are_the_three_copies_of_the_engine() {
        assert_eq!(AudioBank::One.rom_bank(), 0x02);
        assert_eq!(AudioBank::Two.rom_bank(), 0x08);
        assert_eq!(AudioBank::Three.rom_bank(), 0x1F);
        assert_eq!(AudioBank::from_rom_bank(0x08), Some(AudioBank::Two));
        assert_eq!(AudioBank::from_rom_bank(0x03), None);
    }

    #[test]
    fn a_header_names_its_channels_and_where_their_commands_start() {
        let pallet_town = SoundHeader::read(AudioBank::One, sounds::MUSIC_PALLET_TOWN.id);
        assert_eq!(pallet_town.channels.len(), 3, "channel_count 3");
        assert_eq!(pallet_town.channels[0], HeaderChannel { channel: 0, address: AudioBank::One.label("Music_PalletTown_Ch1") });
        assert_eq!(pallet_town.channels[2].channel, 2);

        // A sound effect claims the sfx channels, 5 to 8, which are 4 to 7 stored.
        let instrument = SoundHeader::read(AudioBank::One, sounds::SFX_NOISE_INSTRUMENT01);
        assert_eq!(instrument.channels, vec![HeaderChannel { channel: 7, address: 0x42FD }]);
        assert_eq!(SoundHeader::read(AudioBank::One, sounds::MUSIC_CITIES1.id).channels.len(), 4);
    }

    /// The id is the header's offset into the table over three, so the same id is the same sound in
    /// whichever bank holds it, and the three tables agree wherever they overlap.
    #[test]
    fn an_id_is_its_headers_row() {
        assert_eq!(sounds::SFX_NOISE_INSTRUMENT01, SoundId(1));
        assert_eq!(CRY_SFX_START, SoundId(20));
        assert_eq!(CRY_SFX_END, SoundId(134));
        assert_eq!(NOISE_INSTRUMENTS_END, SoundId(20));
        assert_eq!(BATTLE_SFX_START, SoundId(157));
        assert_eq!(BATTLE_SFX_END, SoundId(234));
        assert_eq!(AudioBank::One.max_sfx_id(), SoundId(185));
        assert_eq!(AudioBank::Two.max_sfx_id(), SoundId(233));
        assert_eq!(AudioBank::Three.max_sfx_id(), SoundId(194));
        // The same sound effect, three copies, one id.
        for bank in AudioBank::ALL {
            assert_eq!(SoundHeader::read(bank, sounds::SFX_TINK).channels.len(), 1, "{bank:?}");
        }
    }

    /// Walking the table lands on every header the symbol file names and on nothing else, which is
    /// what makes a sweep over every sound in the cartridge possible at all.
    #[test]
    fn walking_the_table_finds_every_sound_and_stops_at_the_last_song() {
        let one = AudioBank::One.sound_ids();
        assert_eq!(one.len(), 115);
        assert_eq!(one.first(), Some(&sounds::SFX_NOISE_INSTRUMENT01));
        assert_eq!(one.last(), Some(&sounds::MUSIC_INDIGO_PLATEAU.id));
        assert!(one.contains(&sounds::MUSIC_PALLET_TOWN.id) && one.contains(&sounds::SFX_TINK));
        assert!(one.contains(&CRY_SFX_START), "the cries are three rows each");
        assert!(!one.contains(&SoundId(CRY_SFX_START.0 + 1)));

        let two = AudioBank::Two.sound_ids();
        assert_eq!((two.len(), two.last()), (126, Some(&sounds::MUSIC_DEFEATED_GYM_LEADER.id)));
        let three = AudioBank::Three.sound_ids();
        assert_eq!((three.len(), three.last()), (121, Some(&sounds::MUSIC_MEET_MALE_TRAINER.id)));
    }

    #[test]
    fn the_pitch_table_is_the_twelve_notes_and_the_same_in_all_three_banks() {
        assert_eq!(AudioBank::One.pitch(0), 0xF82C, "C_");
        assert_eq!(AudioBank::One.pitch(11), 0xFBDA, "B_");
        for bank in AudioBank::ALL {
            assert!((0..12).all(|note| bank.pitch(note) == AudioBank::One.pitch(note)), "{bank:?}");
        }
    }

    /// Four of the nine wave pointers name a label past the last table, whose sixteen bytes are the
    /// sound effect each bank stores next: three different waves in the cartridge, one here.
    #[test]
    fn the_empty_wave_is_lavender_towns_in_every_bank() {
        for ruleset in [Ruleset::Modern, Ruleset::Gen1] {
            assert_eq!(AudioBank::One.wave_sample(0, ruleset)[..4], [0x02, 0x46, 0x8A, 0xCE]);
            assert_eq!(AudioBank::One.wave_sample(5, ruleset), LAVENDER_WAVE);
            for bank in AudioBank::ALL {
                for instrument in 0..9 {
                    assert_eq!(bank.wave_sample(instrument, ruleset), bank.wave_sample(WAVE_POINTERS[instrument as usize], ruleset));
                }
                assert_eq!(bank.wave_sample(4, ruleset), AudioBank::One.wave_sample(4, Ruleset::Modern), "the real tables agree");
            }
        }
        for bank in AudioBank::ALL {
            assert_eq!(bank.wave_sample(5, Ruleset::Modern), LAVENDER_WAVE, "{bank:?}");
        }
        assert_ne!(AudioBank::Two.wave_sample(5, Ruleset::Gen1), LAVENDER_WAVE);
        assert_ne!(AudioBank::Three.wave_sample(5, Ruleset::Gen1), LAVENDER_WAVE);
    }

    #[test]
    fn a_cry_is_a_base_sound_three_ids_apart_and_two_modifiers() {
        let rhydon = Cry::of_species_index(1);
        assert_eq!(rhydon.sound, SoundId(CRY_SFX_START.0 + 3 * 0x11));
        assert_eq!((rhydon.frequency_modifier, rhydon.tempo_modifier), (0x00, 0x80));
        let clefairy = Cry::of_species_index(4);
        assert_eq!((clefairy.frequency_modifier, clefairy.tempo_modifier), (0xCC, 0x01));
        // Every cry lands inside the range the engine tests for.
        for index in 1..=190u8 {
            let cry = Cry::of_species_index(index);
            assert!((CRY_SFX_START..CRY_SFX_END).contains(&cry.sound), "index {index}: {cry:?}");
        }
    }
}

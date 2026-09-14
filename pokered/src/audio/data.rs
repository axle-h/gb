//! The cartridge's audio data: the three parallel copies of the header table, the command streams,
//! the wave samples and the pitch table, plus the per-species cry modifiers.
//!
//! Recreates `audio/headers/{sfx,music}headers(1|2|3).asm`, `audio/notes.asm`,
//! `audio/wave_samples.asm` and `data/pokemon/cries.asm`. Nothing here computes anything: it is the
//! ROM read the way `AudioN_PlaySound`, `AudioN_GetNextMusicByte`, `AudioN_CalculateFrequency` and
//! `AudioN_ApplyWavePatternAndFrequency` read it.
//!
//! A sound id is an index into `SFX_Headers_(1|2|3)` at three bytes a row, which is exactly what
//! `music_const` computes, so music and sound effects share one numbering. The three tables all
//! start at `$4000` of their own bank, so the same id names the same sound in each — where the
//! bank has one.

use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::{pokered_symbols, DmgBank, DmgPointer};
use serde::{Deserialize, Serialize};

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

    pub const fn rom_bank(self) -> u8 {
        match self {
            AudioBank::One => pokered_symbols::Audio1_UpdateMusic.bank.id(),
            AudioBank::Two => pokered_symbols::Audio2_UpdateMusic.bank.id(),
            AudioBank::Three => pokered_symbols::Audio3_UpdateMusic.bank.id(),
        }
    }

    pub fn from_rom_bank(bank: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|audio| audio.rom_bank() == bank)
    }

    /// `MAX_SFX_ID_(1|2|3)`: at or below this an id is a sound effect, above it music.
    pub const fn max_sfx_id(self) -> SoundId {
        match self {
            AudioBank::One => SoundId::of(pokered_symbols::SFX_Safari_Zone_PA),
            AudioBank::Two => SoundId::of(pokered_symbols::SFX_Trainer_Appeared),
            AudioBank::Three => SoundId::of(pokered_symbols::SFX_Shooting_Star),
        }
    }

    /// The bank's whole switchable window: every pointer in its data addresses into this.
    fn window(self) -> &'static [u8] {
        rom_slice(DmgPointer { bank: DmgBank::ROM { bank: self.rom_bank() }, address: 0x4000 })
    }

    /// One byte of the bank, as `AudioN_GetNextMusicByte` reads it.
    pub fn byte(self, address: u16) -> u8 {
        let at = (address as usize).checked_sub(0x4000).expect("an audio pointer is in the bank window");
        self.window()[at]
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

    fn symbol(self, one: DmgPointer, two: DmgPointer, three: DmgPointer) -> DmgPointer {
        match self {
            AudioBank::One => one,
            AudioBank::Two => two,
            AudioBank::Three => three,
        }
    }

    /// `AudioN_Pitches`: the frequency of each of the twelve notes at octave 7, little endian.
    /// The table is `audio/notes.asm` included three times, so all three banks hold the same twelve.
    pub fn pitch(self, note: u8) -> u16 {
        let table = rom_slice(self.symbol(
            pokered_symbols::Audio1_Pitches,
            pokered_symbols::Audio2_Pitches,
            pokered_symbols::Audio3_Pitches,
        ));
        u16::from_le_bytes([table[note as usize * 2], table[note as usize * 2 + 1]])
    }

    /// One of the nine `AudioN_WavePointers`, as the sixteen bytes copied into wave RAM.
    ///
    /// There are only six tables. Pointers 5 to 8 all name the last one, whose sixteen bytes run
    /// off the end of the table into whatever sound effect the bank happens to store next, so the
    /// instrument differs between the three copies of otherwise identical data.
    pub fn wave_sample(self, instrument: u8) -> [u8; 16] {
        let pointers = self.symbol(
            pokered_symbols::Audio1_WavePointers,
            pokered_symbols::Audio2_WavePointers,
            pokered_symbols::Audio3_WavePointers,
        );
        let table = rom_slice(pointers);
        let at = u16::from_le_bytes([table[instrument as usize * 2], table[instrument as usize * 2 + 1]]);
        std::array::from_fn(|i| self.byte(at + i as u16))
    }
}

/// An index into `SFX_Headers_(1|2|3)`, three bytes a row: what `music_const` computes and what
/// `PlaySound` is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SoundId(pub u8);

impl SoundId {
    pub const fn of(header: DmgPointer) -> Self {
        Self(((header.address - HEADERS) / 3) as u8)
    }

    /// `SFX_STOP_ALL_MUSIC`, which silences everything rather than naming a header.
    pub const STOP_ALL_MUSIC: SoundId = SoundId(0xFF);
}

/// A piece of music, which is only playable with the bank it lives in: `PlayMusic` takes both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sound {
    pub bank: AudioBank,
    pub id: SoundId,
}

/// `NOISE_INSTRUMENTS_END`: below this a noise channel sound is a drum rather than a sound effect,
/// which is what lets a drum interrupt one sound effect but not another.
pub const NOISE_INSTRUMENTS_END: SoundId = SoundId(SoundId::of(pokered_symbols::SFX_Noise_Instrument19_1).0 + 1);
/// `CRY_SFX_START`, the first of the thirty-eight base cries.
pub const CRY_SFX_START: SoundId = SoundId::of(pokered_symbols::SFX_Cry00_1);
/// `CRY_SFX_END`. Three past the last cry, because a cry header claims three channels.
pub const CRY_SFX_END: SoundId = SoundId(SoundId::of(pokered_symbols::SFX_Cry25_1).0 + 3);
/// `BATTLE_SFX_START` and `BATTLE_SFX_END`, which only `AUDIO_2` tests.
pub const BATTLE_SFX_START: SoundId = SoundId::of(pokered_symbols::SFX_Peck);
pub const BATTLE_SFX_END: SoundId = SoundId(SoundId::of(pokered_symbols::SFX_Trainer_Appeared).0 + 1);

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
        let row = &rom_slice(pokered_symbols::CryData)[(index as usize - 1) * 3..][..3];
        // Cry headers claim three channels each, so the base cry is three ids apart.
        let sound = SoundId(row[0].rotate_left(1).wrapping_add(row[0]).wrapping_add(CRY_SFX_START.0));
        Self { sound, frequency_modifier: row[1], tempo_modifier: row[2] }
    }
}

/// Every music and sound effect the cartridge names, as `constants/music_constants.asm` numbers
/// them. A sound effect is an id alone, because the three banks each hold their own copy of it; a
/// piece of music carries the bank it is stored in, which is what `PlayMusic` is passed.
#[allow(dead_code)]
pub mod sounds {
    use super::{AudioBank, Sound, SoundId};
    use poke_core::symbols::pokered_symbols;

    pub const MUSIC_PALLET_TOWN: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_PalletTown) };
    pub const MUSIC_POKECENTER: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Pokecenter) };
    pub const MUSIC_GYM: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Gym) };
    pub const MUSIC_CITIES1: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Cities1) };
    pub const MUSIC_CITIES2: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Cities2) };
    pub const MUSIC_CELADON: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Celadon) };
    pub const MUSIC_CINNABAR: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Cinnabar) };
    pub const MUSIC_VERMILION: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Vermilion) };
    pub const MUSIC_LAVENDER: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Lavender) };
    pub const MUSIC_SS_ANNE: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_SSAnne) };
    pub const MUSIC_MEET_PROF_OAK: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_MeetProfOak) };
    pub const MUSIC_MEET_RIVAL: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_MeetRival) };
    pub const MUSIC_MUSEUM_GUY: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_MuseumGuy) };
    pub const MUSIC_SAFARI_ZONE: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_SafariZone) };
    pub const MUSIC_PKMN_HEALED: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_PkmnHealed) };
    pub const MUSIC_ROUTES1: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Routes1) };
    pub const MUSIC_ROUTES2: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Routes2) };
    pub const MUSIC_ROUTES3: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Routes3) };
    pub const MUSIC_ROUTES4: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_Routes4) };
    pub const MUSIC_INDIGO_PLATEAU: Sound = Sound { bank: AudioBank::One, id: SoundId::of(pokered_symbols::Music_IndigoPlateau) };
    pub const MUSIC_GYM_LEADER_BATTLE: Sound = Sound { bank: AudioBank::Two, id: SoundId::of(pokered_symbols::Music_GymLeaderBattle) };
    pub const MUSIC_TRAINER_BATTLE: Sound = Sound { bank: AudioBank::Two, id: SoundId::of(pokered_symbols::Music_TrainerBattle) };
    pub const MUSIC_WILD_BATTLE: Sound = Sound { bank: AudioBank::Two, id: SoundId::of(pokered_symbols::Music_WildBattle) };
    pub const MUSIC_FINAL_BATTLE: Sound = Sound { bank: AudioBank::Two, id: SoundId::of(pokered_symbols::Music_FinalBattle) };
    pub const MUSIC_DEFEATED_TRAINER: Sound = Sound { bank: AudioBank::Two, id: SoundId::of(pokered_symbols::Music_DefeatedTrainer) };
    pub const MUSIC_DEFEATED_WILD_MON: Sound = Sound { bank: AudioBank::Two, id: SoundId::of(pokered_symbols::Music_DefeatedWildMon) };
    pub const MUSIC_DEFEATED_GYM_LEADER: Sound = Sound { bank: AudioBank::Two, id: SoundId::of(pokered_symbols::Music_DefeatedGymLeader) };
    pub const MUSIC_TITLE_SCREEN: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_TitleScreen) };
    pub const MUSIC_CREDITS: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_Credits) };
    pub const MUSIC_HALL_OF_FAME: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_HallOfFame) };
    pub const MUSIC_OAKS_LAB: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_OaksLab) };
    pub const MUSIC_JIGGLYPUFF_SONG: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_JigglypuffSong) };
    pub const MUSIC_BIKE_RIDING: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_BikeRiding) };
    pub const MUSIC_SURFING: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_Surfing) };
    pub const MUSIC_GAME_CORNER: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_GameCorner) };
    pub const MUSIC_INTRO_BATTLE: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_IntroBattle) };
    pub const MUSIC_DUNGEON1: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_Dungeon1) };
    pub const MUSIC_DUNGEON2: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_Dungeon2) };
    pub const MUSIC_DUNGEON3: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_Dungeon3) };
    pub const MUSIC_CINNABAR_MANSION: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_CinnabarMansion) };
    pub const MUSIC_POKEMON_TOWER: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_PokemonTower) };
    pub const MUSIC_SILPH_CO: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_SilphCo) };
    pub const MUSIC_MEET_EVIL_TRAINER: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_MeetEvilTrainer) };
    pub const MUSIC_MEET_FEMALE_TRAINER: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_MeetFemaleTrainer) };
    pub const MUSIC_MEET_MALE_TRAINER: Sound = Sound { bank: AudioBank::Three, id: SoundId::of(pokered_symbols::Music_MeetMaleTrainer) };
    pub const SFX_NOISE_INSTRUMENT01: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument01_1);
    pub const SFX_NOISE_INSTRUMENT02: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument02_1);
    pub const SFX_NOISE_INSTRUMENT03: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument03_1);
    pub const SFX_NOISE_INSTRUMENT04: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument04_1);
    pub const SFX_NOISE_INSTRUMENT05: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument05_1);
    pub const SFX_NOISE_INSTRUMENT06: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument06_1);
    pub const SFX_NOISE_INSTRUMENT07: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument07_1);
    pub const SFX_NOISE_INSTRUMENT08: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument08_1);
    pub const SFX_NOISE_INSTRUMENT09: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument09_1);
    pub const SFX_NOISE_INSTRUMENT10: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument10_1);
    pub const SFX_NOISE_INSTRUMENT11: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument11_1);
    pub const SFX_NOISE_INSTRUMENT12: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument12_1);
    pub const SFX_NOISE_INSTRUMENT13: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument13_1);
    pub const SFX_NOISE_INSTRUMENT14: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument14_1);
    pub const SFX_NOISE_INSTRUMENT15: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument15_1);
    pub const SFX_NOISE_INSTRUMENT16: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument16_1);
    pub const SFX_NOISE_INSTRUMENT17: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument17_1);
    pub const SFX_NOISE_INSTRUMENT18: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument18_1);
    pub const SFX_NOISE_INSTRUMENT19: SoundId = SoundId::of(pokered_symbols::SFX_Noise_Instrument19_1);
    pub const SFX_CRY_00: SoundId = SoundId::of(pokered_symbols::SFX_Cry00_1);
    pub const SFX_CRY_01: SoundId = SoundId::of(pokered_symbols::SFX_Cry01_1);
    pub const SFX_CRY_02: SoundId = SoundId::of(pokered_symbols::SFX_Cry02_1);
    pub const SFX_CRY_03: SoundId = SoundId::of(pokered_symbols::SFX_Cry03_1);
    pub const SFX_CRY_04: SoundId = SoundId::of(pokered_symbols::SFX_Cry04_1);
    pub const SFX_CRY_05: SoundId = SoundId::of(pokered_symbols::SFX_Cry05_1);
    pub const SFX_CRY_06: SoundId = SoundId::of(pokered_symbols::SFX_Cry06_1);
    pub const SFX_CRY_07: SoundId = SoundId::of(pokered_symbols::SFX_Cry07_1);
    pub const SFX_CRY_08: SoundId = SoundId::of(pokered_symbols::SFX_Cry08_1);
    pub const SFX_CRY_09: SoundId = SoundId::of(pokered_symbols::SFX_Cry09_1);
    pub const SFX_CRY_0A: SoundId = SoundId::of(pokered_symbols::SFX_Cry0A_1);
    pub const SFX_CRY_0B: SoundId = SoundId::of(pokered_symbols::SFX_Cry0B_1);
    pub const SFX_CRY_0C: SoundId = SoundId::of(pokered_symbols::SFX_Cry0C_1);
    pub const SFX_CRY_0D: SoundId = SoundId::of(pokered_symbols::SFX_Cry0D_1);
    pub const SFX_CRY_0E: SoundId = SoundId::of(pokered_symbols::SFX_Cry0E_1);
    pub const SFX_CRY_0F: SoundId = SoundId::of(pokered_symbols::SFX_Cry0F_1);
    pub const SFX_CRY_10: SoundId = SoundId::of(pokered_symbols::SFX_Cry10_1);
    pub const SFX_CRY_11: SoundId = SoundId::of(pokered_symbols::SFX_Cry11_1);
    pub const SFX_CRY_12: SoundId = SoundId::of(pokered_symbols::SFX_Cry12_1);
    pub const SFX_CRY_13: SoundId = SoundId::of(pokered_symbols::SFX_Cry13_1);
    pub const SFX_CRY_14: SoundId = SoundId::of(pokered_symbols::SFX_Cry14_1);
    pub const SFX_CRY_15: SoundId = SoundId::of(pokered_symbols::SFX_Cry15_1);
    pub const SFX_CRY_16: SoundId = SoundId::of(pokered_symbols::SFX_Cry16_1);
    pub const SFX_CRY_17: SoundId = SoundId::of(pokered_symbols::SFX_Cry17_1);
    pub const SFX_CRY_18: SoundId = SoundId::of(pokered_symbols::SFX_Cry18_1);
    pub const SFX_CRY_19: SoundId = SoundId::of(pokered_symbols::SFX_Cry19_1);
    pub const SFX_CRY_1A: SoundId = SoundId::of(pokered_symbols::SFX_Cry1A_1);
    pub const SFX_CRY_1B: SoundId = SoundId::of(pokered_symbols::SFX_Cry1B_1);
    pub const SFX_CRY_1C: SoundId = SoundId::of(pokered_symbols::SFX_Cry1C_1);
    pub const SFX_CRY_1D: SoundId = SoundId::of(pokered_symbols::SFX_Cry1D_1);
    pub const SFX_CRY_1E: SoundId = SoundId::of(pokered_symbols::SFX_Cry1E_1);
    pub const SFX_CRY_1F: SoundId = SoundId::of(pokered_symbols::SFX_Cry1F_1);
    pub const SFX_CRY_20: SoundId = SoundId::of(pokered_symbols::SFX_Cry20_1);
    pub const SFX_CRY_21: SoundId = SoundId::of(pokered_symbols::SFX_Cry21_1);
    pub const SFX_CRY_22: SoundId = SoundId::of(pokered_symbols::SFX_Cry22_1);
    pub const SFX_CRY_23: SoundId = SoundId::of(pokered_symbols::SFX_Cry23_1);
    pub const SFX_CRY_24: SoundId = SoundId::of(pokered_symbols::SFX_Cry24_1);
    pub const SFX_CRY_25: SoundId = SoundId::of(pokered_symbols::SFX_Cry25_1);
    pub const SFX_GET_ITEM_2: SoundId = SoundId::of(pokered_symbols::SFX_Get_Item2_1);
    pub const SFX_TINK: SoundId = SoundId::of(pokered_symbols::SFX_Tink_1);
    pub const SFX_HEAL_HP: SoundId = SoundId::of(pokered_symbols::SFX_Heal_HP_1);
    pub const SFX_HEAL_AILMENT: SoundId = SoundId::of(pokered_symbols::SFX_Heal_Ailment_1);
    pub const SFX_START_MENU: SoundId = SoundId::of(pokered_symbols::SFX_Start_Menu_1);
    pub const SFX_PRESS_AB: SoundId = SoundId::of(pokered_symbols::SFX_Press_AB_1);
    pub const SFX_GET_ITEM_1: SoundId = SoundId::of(pokered_symbols::SFX_Get_Item1_1);
    pub const SFX_POKEDEX_RATING: SoundId = SoundId::of(pokered_symbols::SFX_Pokedex_Rating_1);
    pub const SFX_GET_KEY_ITEM: SoundId = SoundId::of(pokered_symbols::SFX_Get_Key_Item_1);
    pub const SFX_POISONED: SoundId = SoundId::of(pokered_symbols::SFX_Poisoned_1);
    pub const SFX_TRADE_MACHINE: SoundId = SoundId::of(pokered_symbols::SFX_Trade_Machine_1);
    pub const SFX_TURN_ON_PC: SoundId = SoundId::of(pokered_symbols::SFX_Turn_On_PC_1);
    pub const SFX_TURN_OFF_PC: SoundId = SoundId::of(pokered_symbols::SFX_Turn_Off_PC_1);
    pub const SFX_ENTER_PC: SoundId = SoundId::of(pokered_symbols::SFX_Enter_PC_1);
    pub const SFX_SHRINK: SoundId = SoundId::of(pokered_symbols::SFX_Shrink_1);
    pub const SFX_SWITCH: SoundId = SoundId::of(pokered_symbols::SFX_Switch_1);
    pub const SFX_HEALING_MACHINE: SoundId = SoundId::of(pokered_symbols::SFX_Healing_Machine_1);
    pub const SFX_TELEPORT_EXIT_1: SoundId = SoundId::of(pokered_symbols::SFX_Teleport_Exit1_1);
    pub const SFX_TELEPORT_ENTER_1: SoundId = SoundId::of(pokered_symbols::SFX_Teleport_Enter1_1);
    pub const SFX_TELEPORT_EXIT_2: SoundId = SoundId::of(pokered_symbols::SFX_Teleport_Exit2_1);
    pub const SFX_LEDGE: SoundId = SoundId::of(pokered_symbols::SFX_Ledge_1);
    pub const SFX_TELEPORT_ENTER_2: SoundId = SoundId::of(pokered_symbols::SFX_Teleport_Enter2_1);
    pub const SFX_FLY: SoundId = SoundId::of(pokered_symbols::SFX_Fly_1);
    pub const SFX_DENIED: SoundId = SoundId::of(pokered_symbols::SFX_Denied_1);
    pub const SFX_ARROW_TILES: SoundId = SoundId::of(pokered_symbols::SFX_Arrow_Tiles_1);
    pub const SFX_PUSH_BOULDER: SoundId = SoundId::of(pokered_symbols::SFX_Push_Boulder_1);
    pub const SFX_SS_ANNE_HORN: SoundId = SoundId::of(pokered_symbols::SFX_SS_Anne_Horn_1);
    pub const SFX_WITHDRAW_DEPOSIT: SoundId = SoundId::of(pokered_symbols::SFX_Withdraw_Deposit_1);
    pub const SFX_CUT: SoundId = SoundId::of(pokered_symbols::SFX_Cut_1);
    pub const SFX_GO_INSIDE: SoundId = SoundId::of(pokered_symbols::SFX_Go_Inside_1);
    pub const SFX_SWAP: SoundId = SoundId::of(pokered_symbols::SFX_Swap_1);
    pub const SFX_59: SoundId = SoundId::of(pokered_symbols::SFX_59_1);
    pub const SFX_PURCHASE: SoundId = SoundId::of(pokered_symbols::SFX_Purchase_1);
    pub const SFX_COLLISION: SoundId = SoundId::of(pokered_symbols::SFX_Collision_1);
    pub const SFX_GO_OUTSIDE: SoundId = SoundId::of(pokered_symbols::SFX_Go_Outside_1);
    pub const SFX_SAVE: SoundId = SoundId::of(pokered_symbols::SFX_Save_1);
    pub const SFX_POKEFLUTE: SoundId = SoundId::of(pokered_symbols::SFX_Pokeflute);
    pub const SFX_SAFARI_ZONE_PA: SoundId = SoundId::of(pokered_symbols::SFX_Safari_Zone_PA);
    pub const SFX_LEVEL_UP: SoundId = SoundId::of(pokered_symbols::SFX_Level_Up);
    pub const SFX_BALL_TOSS: SoundId = SoundId::of(pokered_symbols::SFX_Ball_Toss);
    pub const SFX_BALL_POOF: SoundId = SoundId::of(pokered_symbols::SFX_Ball_Poof);
    pub const SFX_FAINT_THUD: SoundId = SoundId::of(pokered_symbols::SFX_Faint_Thud);
    pub const SFX_RUN: SoundId = SoundId::of(pokered_symbols::SFX_Run);
    pub const SFX_DEX_PAGE_ADDED: SoundId = SoundId::of(pokered_symbols::SFX_Dex_Page_Added);
    pub const SFX_CAUGHT_MON: SoundId = SoundId::of(pokered_symbols::SFX_Caught_Mon);
    pub const SFX_PECK: SoundId = SoundId::of(pokered_symbols::SFX_Peck);
    pub const SFX_FAINT_FALL: SoundId = SoundId::of(pokered_symbols::SFX_Faint_Fall);
    pub const SFX_BATTLE_09: SoundId = SoundId::of(pokered_symbols::SFX_Battle_09);
    pub const SFX_POUND: SoundId = SoundId::of(pokered_symbols::SFX_Pound);
    pub const SFX_BATTLE_0B: SoundId = SoundId::of(pokered_symbols::SFX_Battle_0B);
    pub const SFX_BATTLE_0C: SoundId = SoundId::of(pokered_symbols::SFX_Battle_0C);
    pub const SFX_BATTLE_0D: SoundId = SoundId::of(pokered_symbols::SFX_Battle_0D);
    pub const SFX_BATTLE_0E: SoundId = SoundId::of(pokered_symbols::SFX_Battle_0E);
    pub const SFX_BATTLE_0F: SoundId = SoundId::of(pokered_symbols::SFX_Battle_0F);
    pub const SFX_DAMAGE: SoundId = SoundId::of(pokered_symbols::SFX_Damage);
    pub const SFX_NOT_VERY_EFFECTIVE: SoundId = SoundId::of(pokered_symbols::SFX_Not_Very_Effective);
    pub const SFX_BATTLE_12: SoundId = SoundId::of(pokered_symbols::SFX_Battle_12);
    pub const SFX_BATTLE_13: SoundId = SoundId::of(pokered_symbols::SFX_Battle_13);
    pub const SFX_BATTLE_14: SoundId = SoundId::of(pokered_symbols::SFX_Battle_14);
    pub const SFX_VINE_WHIP: SoundId = SoundId::of(pokered_symbols::SFX_Vine_Whip);
    pub const SFX_BATTLE_16: SoundId = SoundId::of(pokered_symbols::SFX_Battle_16);
    pub const SFX_BATTLE_17: SoundId = SoundId::of(pokered_symbols::SFX_Battle_17);
    pub const SFX_BATTLE_18: SoundId = SoundId::of(pokered_symbols::SFX_Battle_18);
    pub const SFX_BATTLE_19: SoundId = SoundId::of(pokered_symbols::SFX_Battle_19);
    pub const SFX_SUPER_EFFECTIVE: SoundId = SoundId::of(pokered_symbols::SFX_Super_Effective);
    pub const SFX_BATTLE_1B: SoundId = SoundId::of(pokered_symbols::SFX_Battle_1B);
    pub const SFX_BATTLE_1C: SoundId = SoundId::of(pokered_symbols::SFX_Battle_1C);
    pub const SFX_DOUBLESLAP: SoundId = SoundId::of(pokered_symbols::SFX_Doubleslap);
    pub const SFX_BATTLE_1E: SoundId = SoundId::of(pokered_symbols::SFX_Battle_1E);
    pub const SFX_HORN_DRILL: SoundId = SoundId::of(pokered_symbols::SFX_Horn_Drill);
    pub const SFX_BATTLE_20: SoundId = SoundId::of(pokered_symbols::SFX_Battle_20);
    pub const SFX_BATTLE_21: SoundId = SoundId::of(pokered_symbols::SFX_Battle_21);
    pub const SFX_BATTLE_22: SoundId = SoundId::of(pokered_symbols::SFX_Battle_22);
    pub const SFX_BATTLE_23: SoundId = SoundId::of(pokered_symbols::SFX_Battle_23);
    pub const SFX_BATTLE_24: SoundId = SoundId::of(pokered_symbols::SFX_Battle_24);
    pub const SFX_BATTLE_25: SoundId = SoundId::of(pokered_symbols::SFX_Battle_25);
    pub const SFX_BATTLE_26: SoundId = SoundId::of(pokered_symbols::SFX_Battle_26);
    pub const SFX_BATTLE_27: SoundId = SoundId::of(pokered_symbols::SFX_Battle_27);
    pub const SFX_BATTLE_28: SoundId = SoundId::of(pokered_symbols::SFX_Battle_28);
    pub const SFX_BATTLE_29: SoundId = SoundId::of(pokered_symbols::SFX_Battle_29);
    pub const SFX_BATTLE_2A: SoundId = SoundId::of(pokered_symbols::SFX_Battle_2A);
    pub const SFX_BATTLE_2B: SoundId = SoundId::of(pokered_symbols::SFX_Battle_2B);
    pub const SFX_BATTLE_2C: SoundId = SoundId::of(pokered_symbols::SFX_Battle_2C);
    pub const SFX_PSYBEAM: SoundId = SoundId::of(pokered_symbols::SFX_Psybeam);
    pub const SFX_BATTLE_2E: SoundId = SoundId::of(pokered_symbols::SFX_Battle_2E);
    pub const SFX_BATTLE_2F: SoundId = SoundId::of(pokered_symbols::SFX_Battle_2F);
    pub const SFX_PSYCHIC_M: SoundId = SoundId::of(pokered_symbols::SFX_Psychic_M);
    pub const SFX_BATTLE_31: SoundId = SoundId::of(pokered_symbols::SFX_Battle_31);
    pub const SFX_BATTLE_32: SoundId = SoundId::of(pokered_symbols::SFX_Battle_32);
    pub const SFX_BATTLE_33: SoundId = SoundId::of(pokered_symbols::SFX_Battle_33);
    pub const SFX_BATTLE_34: SoundId = SoundId::of(pokered_symbols::SFX_Battle_34);
    pub const SFX_BATTLE_35: SoundId = SoundId::of(pokered_symbols::SFX_Battle_35);
    pub const SFX_BATTLE_36: SoundId = SoundId::of(pokered_symbols::SFX_Battle_36);
    pub const SFX_TRAINER_APPEARED: SoundId = SoundId::of(pokered_symbols::SFX_Trainer_Appeared);
    pub const SFX_INTRO_LUNGE: SoundId = SoundId::of(pokered_symbols::SFX_Intro_Lunge);
    pub const SFX_INTRO_HIP: SoundId = SoundId::of(pokered_symbols::SFX_Intro_Hip);
    pub const SFX_INTRO_HOP: SoundId = SoundId::of(pokered_symbols::SFX_Intro_Hop);
    pub const SFX_INTRO_RAISE: SoundId = SoundId::of(pokered_symbols::SFX_Intro_Raise);
    pub const SFX_INTRO_CRASH: SoundId = SoundId::of(pokered_symbols::SFX_Intro_Crash);
    pub const SFX_INTRO_WHOOSH: SoundId = SoundId::of(pokered_symbols::SFX_Intro_Whoosh);
    pub const SFX_SLOTS_STOP_WHEEL: SoundId = SoundId::of(pokered_symbols::SFX_Slots_Stop_Wheel);
    pub const SFX_SLOTS_REWARD: SoundId = SoundId::of(pokered_symbols::SFX_Slots_Reward);
    pub const SFX_SLOTS_NEW_SPIN: SoundId = SoundId::of(pokered_symbols::SFX_Slots_New_Spin);
    pub const SFX_SHOOTING_STAR: SoundId = SoundId::of(pokered_symbols::SFX_Shooting_Star);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(pallet_town.channels[0], HeaderChannel { channel: 0, address: 0x67C5 });
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

    /// Five of the nine wave pointers name the same last table, whose sixteen bytes run off the end
    /// of `wave_samples.asm` into the sound effect stored next, so that instrument is three
    /// different waves in the three banks.
    #[test]
    fn the_wave_samples_past_the_table_differ_between_the_banks() {
        assert_eq!(AudioBank::One.wave_sample(0)[..4], [0x02, 0x46, 0x8A, 0xCE]);
        for instrument in 5..9 {
            assert_eq!(AudioBank::One.wave_sample(instrument), AudioBank::One.wave_sample(5));
        }
        assert_ne!(AudioBank::One.wave_sample(5), AudioBank::Two.wave_sample(5));
        assert_ne!(AudioBank::One.wave_sample(5), AudioBank::Three.wave_sample(5));
        assert_eq!(AudioBank::One.wave_sample(0), AudioBank::Two.wave_sample(0), "the six real tables agree");
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

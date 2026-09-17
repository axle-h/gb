//! A map's `trainer` headers, the table `CheckForEngagingTrainers` walks, and the lists
//! `PlayTrainerMusic` picks a tune from.

use serde::{Deserialize, Serialize};
use crate::rom_gfx::rom_slice;
use crate::symbols::{pokered_symbols, DmgBank, DmgPointer};

/// `TRAINER_STRUCT_SIZE`.
const TRAINER_STRUCT_SIZE: u16 = 12;
/// `OPP_ID_OFFSET`: a class in `wCurOpponent` is stored this far up.
pub const OPP_ID_OFFSET: u8 = 200;

/// One `trainer` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainerHeader {
    /// Where the header is, which `wTrainerHeaderPtr` holds.
    pub at: DmgPointer,
    /// `TRAINER_EVENT_FLAG_BIT`, which is also the trainer's sprite index.
    pub sprite: u8,
    /// The view range in squares, stored as its high nybble.
    pub range: u8,
    /// The `wEventFlags` bit the flag byte and bit name together.
    pub event: u16,
    pub before_battle: DmgPointer,
    pub after_battle: DmgPointer,
    /// `TRAINER_WON_BATTLE_TEXT`, and the lost one the macro fills with the same text.
    pub end_battle: DmgPointer,
    pub lost_battle: DmgPointer,
}

impl TrainerHeader {
    pub fn read(at: DmgPointer) -> Self {
        let bytes = rom_slice(at);
        let word = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        let in_bank = |address: u16| DmgPointer { bank: if address < 0x4000 { DmgBank::ROM { bank: 0 } } else { at.bank }, address };
        let flag_byte = word(2) - pokered_symbols::wEventFlags.address;
        Self {
            at,
            sprite: bytes[0],
            range: bytes[1] >> 4,
            event: flag_byte * 8 + bytes[0] as u16,
            before_battle: in_bank(word(4)),
            after_battle: in_bank(word(6)),
            end_battle: in_bank(word(8)),
            lost_battle: in_bank(word(10)),
        }
    }

    /// The table from its first header to the `db -1` after the last.
    pub fn table(first: DmgPointer) -> Vec<Self> {
        (0..)
            .map(|i| first + i * TRAINER_STRUCT_SIZE)
            .take_while(|&at| rom_slice(at)[0] != 0xFF)
            .map(Self::read)
            .collect()
    }
}

/// `EncounterMusic`'s three: which `MUSIC_MEET_*` a trainer class engages to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncounterMusic {
    Evil,
    Female,
    Male,
}

fn listed(list: DmgPointer, class: u8) -> bool {
    rom_slice(list).iter().take_while(|&&byte| byte != 0xFF).any(|&byte| byte == class + OPP_ID_OFFSET)
}

/// `PlayTrainerMusic`'s choice for a trainer class: `EvilTrainerList`, then `FemaleTrainerList`.
pub fn encounter_music(class: u8) -> EncounterMusic {
    if listed(pokered_symbols::EvilTrainerList, class) {
        EncounterMusic::Evil
    } else if listed(pokered_symbols::FemaleTrainerList, class) {
        EncounterMusic::Female
    } else {
        EncounterMusic::Male
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::pokered_events::{self, EVENT_BEAT_VIRIDIAN_FOREST_TRAINER_0, EVENT_BEAT_VIRIDIAN_FOREST_TRAINER_2};

    #[test]
    fn viridian_forest_has_three_bug_catchers_seeing_four_four_and_one() {
        let table = TrainerHeader::table(pokered_symbols::ViridianForestTrainerHeaders);
        assert_eq!(table.iter().map(|h| (h.sprite, h.range)).collect::<Vec<_>>(), [(2, 4), (3, 4), (4, 1)]);
        assert_eq!(table[0].event, EVENT_BEAT_VIRIDIAN_FOREST_TRAINER_0);
        assert_eq!(table[2].event, EVENT_BEAT_VIRIDIAN_FOREST_TRAINER_2);
        assert_eq!(table[0].before_battle, pokered_symbols::ViridianForestYoungster2BattleText);
        assert_eq!(table[0].after_battle, pokered_symbols::ViridianForestYoungster2AfterBattleText);
        assert_eq!(table[0].end_battle, pokered_symbols::ViridianForestYoungster2EndBattleText);
    }

    /// Every header the build found decodes to an event in range and texts in the header's bank.
    #[test]
    fn every_trainer_header_decodes() {
        for &(map, index, at) in pokered_symbols::TRAINER_HEADERS {
            let header = TrainerHeader::read(at);
            assert!(header.event < 0xA00 && header.range <= 15, "{map} {index}: {header:?}");
            assert!(crate::text_script::decode(header.before_battle).is_ok(), "{map} {index}");
        }
    }

    /// The `trainer` macro asserts an event flag sits on the bit `def_trainers` is counting, and the
    /// assembled header carries the flag byte and bit the same event resolved to. Both pin every
    /// generated `EVENT_BEAT_*_TRAINER_*` to the cartridge, so a miscounted `const` block fails here.
    #[test]
    fn every_trainer_event_constant_matches_the_assembled_header() {
        let events: std::collections::HashMap<&str, u16> = pokered_events::NAMES.iter().copied().collect();
        let headers: std::collections::HashMap<(&str, u8), DmgPointer> =
            pokered_symbols::TRAINER_HEADERS.iter().map(|&(map, index, at)| ((map, index), at)).collect();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/pokered/scripts");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "asm") {
                continue;
            }
            let map = path.file_stem().unwrap().to_str().unwrap().to_string();
            let (mut bit, mut index) = (0u8, 0u8);
            for line in std::fs::read_to_string(&path).unwrap().lines() {
                let line = line.split(';').next().unwrap().trim();
                let Some((directive, argument)) = line.split_once(char::is_whitespace).or(Some((line, ""))) else { continue };
                match directive {
                    "def_trainers" => (bit, index) = (argument.trim().parse().unwrap_or(1), 0),
                    "trainer" => {
                        let name = argument.split(',').next().unwrap().trim();
                        let event = *events.get(name).unwrap_or_else(|| panic!("{map}: no such event {name}"));
                        assert_eq!(event % 8, bit as u16 % 8, "{map} trainer {index}: {name} is not on bit {bit}");
                        // A header whose label does not end in its index is not in `TRAINER_HEADERS`.
                        if let Some(&at) = headers.get(&(map.as_str(), index)) {
                            assert_eq!(TrainerHeader::read(at).event, event, "{map} trainer {index}: {name}");
                            checked += 1;
                        }
                        (bit, index) = (bit + 1, index + 1);
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(checked, pokered_symbols::TRAINER_HEADERS.len());
    }

    #[test]
    fn a_lass_is_female_and_a_rocket_evil() {
        const LASS: u8 = 3;
        const ROCKET: u8 = 30;
        const BUG_CATCHER: u8 = 2;
        assert_eq!(encounter_music(LASS), EncounterMusic::Female);
        assert_eq!(encounter_music(ROCKET), EncounterMusic::Evil);
        assert_eq!(encounter_music(BUG_CATCHER), EncounterMusic::Male);
    }
}

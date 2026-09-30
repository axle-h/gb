//! A map's `trainer` headers, the table `CheckForEngagingTrainers` walks, and the lists
//! `PlayTrainerMusic` picks a tune from.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use crate::symbols::SavedLabel;
use crate::tables::{Trainer, TRAINER_HEADERS};

/// `OPP_ID_OFFSET`: a class in `wCurOpponent` is stored this far up.
pub const OPP_ID_OFFSET: u8 = 200;

/// A `trainer` header, what `wTrainerHeaderPtr` holds: one of `TRAINER_HEADERS`' tables and a
/// header in it. A table's own label names its first header. Saved by the header's label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrainerRef {
    table: u8,
    trainer: u8,
}

impl TrainerRef {
    pub(crate) const fn new(table: u8, trainer: u8) -> Self {
        Self { table, trainer }
    }

    pub fn header(self) -> &'static Trainer {
        &TRAINER_HEADERS[self.table as usize].2[self.trainer as usize]
    }

    /// The headers from this one to the table's `db -1`, which `CheckForEngagingTrainers` walks.
    pub fn and_after(self) -> impl Iterator<Item = (Self, &'static Trainer)> {
        let table = TRAINER_HEADERS[self.table as usize].2;
        (self.trainer..table.len() as u8).map(move |trainer| (Self { trainer, ..self }, &table[trainer as usize]))
    }

    pub fn named(label: &str) -> Option<Self> {
        TRAINER_HEADERS.iter().enumerate().find_map(|(table, &(_, name, trainers))| {
            let trainer = if name == label { Some(0) } else { trainers.iter().position(|trainer| trainer.label == label) }?;
            Some(Self::new(table as u8, trainer as u8))
        })
    }
}

impl Serialize for TrainerRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.header().label)
    }
}

impl<'de> Deserialize<'de> for TrainerRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        SavedLabel::resolve(deserializer, Self::named)
    }
}

/// `EncounterMusic`'s three: which `MUSIC_MEET_*` a trainer class engages to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncounterMusic {
    Evil,
    Female,
    Male,
}

fn listed(list: &[u8], class: u8) -> bool {
    list.contains(&(class + OPP_ID_OFFSET))
}

/// `PlayTrainerMusic`'s choice for a trainer class: `EvilTrainerList`, then `FemaleTrainerList`.
pub fn encounter_music(class: u8) -> EncounterMusic {
    if listed(crate::tables::EVIL_TRAINER_LIST, class) {
        EncounterMusic::Evil
    } else if listed(crate::tables::FEMALE_TRAINER_LIST, class) {
        EncounterMusic::Female
    } else {
        EncounterMusic::Male
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::pokered_events::{EVENT_BEAT_VIRIDIAN_FOREST_TRAINER_0, EVENT_BEAT_VIRIDIAN_FOREST_TRAINER_2};

    #[test]
    fn viridian_forest_has_three_bug_catchers_seeing_four_four_and_one() {
        use crate::tables::trainers;
        let table: Vec<_> = trainers::ViridianForestTrainerHeaders.and_after().map(|(_, header)| header).collect();
        assert_eq!(table.iter().map(|h| (h.sprite, h.range)).collect::<Vec<_>>(), [(2, 4), (3, 4), (4, 1)]);
        assert_eq!(table[0].event, EVENT_BEAT_VIRIDIAN_FOREST_TRAINER_0);
        assert_eq!(table[2].event, EVENT_BEAT_VIRIDIAN_FOREST_TRAINER_2);
        assert_eq!(table[0].before_battle, "ViridianForestYoungster2BattleText");
        assert_eq!(table[0].after_battle, "ViridianForestYoungster2AfterBattleText");
        assert_eq!(table[0].end_battle, "ViridianForestYoungster2EndBattleText");
        assert_eq!(trainers::ViridianForestTrainerHeaders, trainers::ViridianForestTrainerHeader0);
        assert_eq!(trainers::ViridianForestTrainerHeader2.and_after().count(), 1);
    }

    /// A header saves as its label, and a save that held its address still loads.
    #[test]
    fn a_header_saves_by_label_and_loads_from_its_old_address() {
        use crate::symbols::{DmgBank, DmgPointer};
        use crate::tables::trainers;
        let header = trainers::ViridianForestTrainerHeader1;
        let saved = rmp_serde::to_vec_named(&header).unwrap();
        assert_eq!(rmp_serde::from_slice::<TrainerRef>(&saved).unwrap(), header);
        let old = rmp_serde::to_vec_named(&DmgPointer { bank: DmgBank::ROM { bank: 0x18 }, address: 0x514E }).unwrap();
        assert_eq!(rmp_serde::from_slice::<TrainerRef>(&old).unwrap(), header);
    }

    /// Every trainer the source declares has an event in range and texts that read.
    #[test]
    fn every_trainer_header_reads() {
        for &(map, _, trainers) in crate::tables::TRAINER_HEADERS {
            for trainer in trainers {
                assert!(trainer.event < 0xA00 && trainer.range <= 15, "{map}: {trainer:?}");
                assert_eq!(trainer.event % 8, trainer.sprite as u16 % 8, "{map}: {trainer:?}");
                for text in [trainer.before_battle, trainer.end_battle, trainer.after_battle] {
                    crate::text_script::far_text(text).unwrap_or_else(|why| panic!("{map}: {why}"));
                }
            }
        }
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

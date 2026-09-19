//! `PowerPlant_Script`: eight Voltorb and Electrode that look like item balls, Zapdos at the back,
//! and five real item balls `PickUpItemText` takes care of.

use poke_core::symbols::pokered_map_scripts::{TEXT_POWERPLANT_ELECTRODE1, TEXT_POWERPLANT_ELECTRODE2,
    TEXT_POWERPLANT_VOLTORB1, TEXT_POWERPLANT_VOLTORB2, TEXT_POWERPLANT_VOLTORB3, TEXT_POWERPLANT_VOLTORB4,
    TEXT_POWERPLANT_VOLTORB5, TEXT_POWERPLANT_VOLTORB6, TEXT_POWERPLANT_ZAPDOS};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use poke_core::species::PokemonSpecies;
use super::{text_at, Flow, Routine, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `wPowerPlantCurScript`.
    pub cur_script: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ld [wPowerPlantCurScript], a` after the table's routine.
    StoreCurScript,
    /// `PowerPlantInitBattleScript`'s own store, after `TalkToTrainer`.
    InitBattleStoreCurScript,
    /// `PowerPlantZapdosBattleText`, whose `text_asm` plays Zapdos's cry before the battle.
    ZapdosBattleText,
    ZapdosCry,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    let index = rt.maps().power_plant.cur_script;
    let index = rt.execute_cur_map_script_in_table(index, sym::PowerPlantTrainerHeaders);
    rt.trainer_script(index).then(Label::StoreCurScript)
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    // The nine headers are named `Voltorb0` to `Voltorb7` and then `Zapdos` whatever the object is,
    // so an Electrode's header carries a Voltorb's name.
    let header = match text_id {
        TEXT_POWERPLANT_VOLTORB1 => sym::Voltorb0TrainerHeader,
        TEXT_POWERPLANT_VOLTORB2 => sym::Voltorb1TrainerHeader,
        TEXT_POWERPLANT_VOLTORB3 => sym::Voltorb2TrainerHeader,
        TEXT_POWERPLANT_ELECTRODE1 => sym::Voltorb3TrainerHeader,
        TEXT_POWERPLANT_VOLTORB4 => sym::Voltorb4TrainerHeader,
        TEXT_POWERPLANT_VOLTORB5 => sym::Voltorb5TrainerHeader,
        TEXT_POWERPLANT_ELECTRODE2 => sym::Voltorb6TrainerHeader,
        TEXT_POWERPLANT_VOLTORB6 => sym::Voltorb7TrainerHeader,
        TEXT_POWERPLANT_ZAPDOS => {
            let before = Some(Label::ZapdosBattleText.into());
            let then = rt.talk_to_trainer_asm(sym::ZapdosTrainerHeader, before, None);
            return Some(then.then(Label::InitBattleStoreCurScript));
        }
        _ => return None,
    };
    Some(rt.talk_to_trainer(header).then(Label::InitBattleStoreCurScript))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::StoreCurScript | Label::InitBattleStoreCurScript => {
            rt.maps().power_plant.cur_script = rt.cur_map_script();
            Flow::Return
        }
        Label::ZapdosBattleText => rt.print_text(text_at(sym::PowerPlantZapdosBattleText)).then(Label::ZapdosCry),
        Label::ZapdosCry => {
            rt.play_cry(PokemonSpecies::Zapdos);
            rt.wait_for_sound_to_finish().then(Routine::TalkToTrainerNotYetFought)
        }
    }
}

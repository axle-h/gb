//! `SSAnneKitchen_Script`: the head cook, who announces one of three main courses.

use poke_core::symbols::pokered_local_labels::SSAnneKitchenCook7Text as cook;
use poke_core::symbols::pokered_map_scripts::TEXT_SSANNEKITCHEN_COOK7;
use serde::{Deserialize, Serialize};
use super::{text_at, Flow, Script};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `ldh a, [hRandomAdd]` after `.MainCourseIsText`.
    MainCourse,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    (text_id == TEXT_SSANNEKITCHEN_COOK7).then(|| rt.print_text(text_at(cook::MainCourseIsText)).then(Label::MainCourse))
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        // Bit 7 is tested before bit 4, so the salmon comes up half the time and the beef a quarter.
        Label::MainCourse => {
            let roll = rt.random();
            let dish = match roll {
                _ if roll & 0x80 != 0 => cook::SalmonDuSaladText,
                _ if roll & 0x10 != 0 => cook::EelsAuBarbecueText,
                _ => cook::PrimeBeefSteakText,
            };
            rt.print_text(text_at(dish)).ret()
        }
    }
}

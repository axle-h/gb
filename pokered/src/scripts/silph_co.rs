//! What Silph Co's floors share: the card key doors every floor draws shut, and the routine its
//! workers greet the player with either side of Giovanni being beaten.

use poke_core::symbols::pokered_events::EVENT_BEAT_SILPH_CO_GIOVANNI;
use poke_core::symbols::DmgPointer;
use super::{text_at, Flow, Script};

/// `SilphCo2F_SetCardKeyDoorYScript`, which the upper floors have their own byte-identical copy of:
/// `hUnlockedSilphCoDoors`, which is where in this floor's gate list the door a card key has just
/// opened is, or `None` for none of them. The coordinate is cleared by the match, so no other floor
/// opens a gate at the same square.
fn unlocked_door(rt: &mut Script, gates: &[(u8, u8)]) -> Option<usize> {
    let index = gates.iter().position(|&gate| gate == rt.card_key_door())?;
    rt.clear_card_key_door();
    Some(index)
}

/// A floor's `GateCallbackScript`: every gate the floor's events do not remember is redrawn shut on
/// every load, so a door opened on one visit is the only one that stays open. The block differs from
/// floor to floor, the floors being tiled differently.
pub(super) fn gate_callback(rt: &mut Script, gates: &[(u8, u8)], events: &[u16], closed_door: u8) {
    redraw_shut_gates(rt, gates, events, |_| closed_door);
}

/// The same for a floor whose own gates are not all tiled alike: one block per gate, in the order
/// the floor lists them.
pub(super) fn gate_callback_blocks(rt: &mut Script, gates: &[(u8, u8)], events: &[u16], closed_doors: &[u8]) {
    redraw_shut_gates(rt, gates, events, |index| closed_doors[index]);
}

fn redraw_shut_gates(rt: &mut Script, gates: &[(u8, u8)], events: &[u16], closed_door: impl Fn(usize) -> u8) {
    if !rt.check_and_reset_cur_map_loaded(1) {
        return;
    }
    if let Some(index) = unlocked_door(rt, gates) {
        rt.set_event(events[index]);
    }
    for (index, (&event, &(x, y))) in events.iter().zip(gates).enumerate() {
        if !rt.check_event(event) {
            rt.replace_tile_block(x, y, closed_door(index));
        }
    }
}

/// `SilphCo6FBeatGiovanniPrintDEOrPrintHLScript`: a worker's line while Team Rocket hold the
/// building, and the one they have once the building is theirs again.
pub(super) fn beat_giovanni_print_de_or_print_hl(rt: &mut Script, held: DmgPointer, freed: DmgPointer) -> Flow {
    let said = match rt.check_event(EVENT_BEAT_SILPH_CO_GIOVANNI) {
        true => freed,
        false => held,
    };
    rt.print_text(text_at(said)).ret()
}

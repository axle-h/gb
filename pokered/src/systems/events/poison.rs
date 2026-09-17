//! `ApplyOutOfBattlePoisonDamage`: every fourth step a poisoned mon loses a point of HP, a mon that
//! reaches 0 faints and is cured, and a party with nothing left standing blacks out.

use serde::{Deserialize, Serialize};
use crate::party::{BoxMon, Named, PartyMon};
use super::day_care::increment_day_care_mon_exp;

/// `PSN`'s bit in the status byte.
pub const PSN: u8 = 1 << 3;

/// What a step did, in the order the cartridge does it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoisonStep {
    /// Party slots that fainted, each with its `TEXT_MON_FAINTED` between it and the next.
    pub fainted: Vec<u8>,
    /// `ChangeBGPalColor0_4Frames` and `SFX_POISONED`: something is still poisoned.
    pub flash: bool,
    /// `TEXT_BLACKED_OUT`, then `HandleBlackOut`.
    pub blacked_out: bool,
}

/// One mon of `.applyDamageLoop`: `Some(fainted)` when it took the point. A borrow from the high
/// byte skips the faint test, which a mon that had 256 HP cannot need.
pub fn poison_mon(mon: &mut PartyMon) -> Option<bool> {
    let mon = &mut mon.mon;
    if mon.status & PSN == 0 || mon.hp == 0 {
        return None;
    }
    let borrowed = mon.hp & 0xFF == 0;
    mon.hp -= 1;
    if borrowed || mon.hp != 0 {
        return Some(false);
    }
    mon.status = 0;
    Some(true)
}

/// `.countPoisonedLoop`.
pub fn any_poisoned(party: &[Named<PartyMon>]) -> bool {
    party.iter().any(|named| named.mon.mon.status & PSN != 0)
}

/// `AnyPartyAlive`.
pub fn any_party_alive(party: &[Named<PartyMon>]) -> bool {
    party.iter().any(|named| named.mon.mon.hp != 0)
}

/// The whole routine at once, for a caller with no texts to show between the mons. `simulating` is
/// `BIT_SCRIPTED_MOVEMENT_STATE`, which skips it all, the day care's step included.
pub fn apply_out_of_battle_poison_damage(party: &mut [Named<PartyMon>], day_care: Option<&mut Named<BoxMon>>,
    step_counter: u8, simulating: bool) -> PoisonStep
{
    let mut step = PoisonStep::default();
    if simulating || party.is_empty() {
        return step;
    }
    if let Some(mon) = day_care {
        mon.mon.exp = increment_day_care_mon_exp(mon.mon.exp);
    }
    if step_counter & 3 != 0 {
        return step;
    }
    for (slot, named) in party.iter_mut().enumerate() {
        if poison_mon(&mut named.mon) == Some(true) {
            step.fainted.push(slot as u8);
        }
    }
    step.flash = any_poisoned(party);
    step.blacked_out = !any_party_alive(party);
    step
}

#[cfg(test)]
mod tests {
    use crate::fixtures::cases;
    use super::*;

    #[derive(Deserialize)]
    struct PoisonInput {
        party: Vec<PartyMon>,
        day_care: Option<BoxMon>,
        step_counter: u8,
        simulating: bool,
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct PoisonOutput {
        party: Vec<(u16, u8)>,
        day_care_exp: Option<u32>,
        step: PoisonStep,
    }

    fn named<M>(mon: M) -> Named<M> {
        Named { mon, ot: vec![], nick: vec![] }
    }

    #[test]
    fn every_harvested_case_of_apply_out_of_battle_poison_damage() {
        let jsonl = include_str!("../../../fixtures/events/apply_out_of_battle_poison_damage.jsonl");
        for (i, expected, _) in cases::<PoisonInput, PoisonOutput>(jsonl) {
            let mut party: Vec<_> = i.party.iter().cloned().map(named).collect();
            let mut day_care = i.day_care.clone().map(named);
            let step = apply_out_of_battle_poison_damage(&mut party, day_care.as_mut(), i.step_counter, i.simulating);
            let output = PoisonOutput {
                party: party.iter().map(|named| (named.mon.mon.hp, named.mon.mon.status)).collect(),
                day_care_exp: day_care.map(|named| named.mon.exp),
                step,
            };
            assert_eq!(output, expected, "{:?} with step {} simulating {}", i.party, i.step_counter, i.simulating);
        }
    }
}

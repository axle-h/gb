//! The sleeping Snorlax blocking Route 12 and Route 16, whose halves of `Route12_Script` and
//! `Route16_Script` are the same code over different events, texts and sprites.
//!
//! The Poké Flute sets the map's fight event; the map's next pass prints, arms a wild battle, takes
//! the sleeping sprite off the map and hands over to its post-battle script.

use poke_core::species::PokemonSpecies;
use crate::modes::battle::result;
use super::{Script, Then};

/// `ld a, 30 / ld [wCurEnemyLevel], a`.
const LEVEL: u8 = 30;

/// What one road's Snorlax differs from the other's by.
pub struct Snorlax {
    /// `EVENT_BEAT_ROUTE12_SNORLAX`: it has been fought, and the road stays clear.
    pub beat: u16,
    /// `EVENT_FIGHT_ROUTE12_SNORLAX`: the flute has just woken it.
    pub fight: u16,
    pub woke_up_text: u8,
    /// What it says as it lumbers off, which a Snorlax that was caught never prints.
    pub calmed_down_text: u8,
    pub toggle: u16,
    pub post_battle_script: u8,
}

/// The three ways out of `Route12SnorlaxPostBattleScript`.
pub enum PostBattle {
    /// `wIsInBattle` at `$ff`: the player blacked out, and the map's scripts start over.
    Lost,
    CalmedDown(Then),
    Caught,
}

impl Snorlax {
    /// `Route12DefaultScript` up to its `DisplayTextID`. `None` is no Snorlax to wake, which falls
    /// through to the map's trainers.
    pub fn woken(&self, rt: &mut Script) -> Option<Then> {
        if rt.check_event(self.beat) {
            return None;
        }
        // The flute's event is taken whether or not this is the pass that acts on it.
        let woken = rt.check_and_reset_event(self.fight);
        woken.then(|| rt.display_text_id(self.woke_up_text))
    }

    /// What follows that text: the wild battle armed and the sleeping sprite gone, leaving the
    /// index of the script that runs once the battle is over.
    pub fn wake_up(&self, rt: &mut Script) -> u8 {
        rt.start_wild_battle(PokemonSpecies::Snorlax, LEVEL);
        rt.hide_object(self.toggle);
        self.post_battle_script
    }

    /// `Route12SnorlaxPostBattleScript` as far as its `SetEvent`.
    pub fn post_battle(&self, rt: &mut Script) -> PostBattle {
        if rt.lost_battle() {
            return PostBattle::Lost;
        }
        rt.update_sprites();
        // `wBattleResult` is a draw where the Snorlax was caught rather than beaten.
        if rt.battle_result() == result::DRAW {
            return PostBattle::Caught;
        }
        PostBattle::CalmedDown(rt.display_text_id(self.calmed_down_text))
    }

    /// `.caught_snorlax`: the road is clear for good.
    pub fn beaten(&self, rt: &mut Script) -> Then {
        rt.set_event(self.beat);
        rt.delay3()
    }
}

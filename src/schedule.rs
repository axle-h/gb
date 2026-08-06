//! The event schedule — "when does the next interesting thing happen?".
//!
//! `gb` used to have no absolute clock at all: every peripheral kept a private accumulator and was
//! handed a delta once per CPU instruction ([`crate::mmu::MMU::update`]). That is finding **F1** in
//! [`docs/compatibility/01-architecture.md`] and it is what makes HALT cost full price — 65% of all
//! emulated m-cycles in Pokémon Red — because there is nothing to ask "how long may I sleep?".
//!
//! This module is the answer to that question. [`MMU::now`](crate::mmu::MMU) is the absolute
//! m-cycle count; each peripheral publishes the absolute time of its next observable event, and
//! `Schedule` keeps the minimum.
//!
//! **Why a flat array rather than gambatte's `MinKeeper`.** With [`N_EV`] = 8, a linear scan is
//! eight `cmp`/`cmov` pairs that the compiler auto-vectorises — smaller and simpler than the
//! template-unrolled tournament tree, and `set` usually takes the O(1) fast path anyway. Port
//! `MinKeeper` only if [`Schedule::recompute`] ever shows up in a profile.
//!
//! **[`DISABLED`] is a sentinel, not a flag.** `u64::MAX` simply never wins the minimum, so a
//! disabled event costs no branch and no `Option` discriminant.

use bincode::{Decode, Encode};

/// What is scheduled. The discriminants index [`Schedule::when`], so they must stay `0..N_EV`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
#[repr(u8)]
pub enum Ev {
    /// The end of the caller's `run` slice. Making it an event rather than a separate bound check
    /// is gambatte's trick (`memory.cpp:142-149`): the run loop then has exactly one exit test.
    EndOfSlice = 0,
    /// The PPU's next mode transition.
    Video = 1,
    /// TIMA's next increment.
    Timer = 2,
    /// DIV's next increment. Separate from [`Ev::Timer`] because the APU frame sequencer hangs off
    /// DIV bit 4, so DIV has observable effects even when TIMA is disabled.
    Divider = 3,
    /// The end of the current serial transfer.
    Serial = 4,
    /// The end of the current OAM DMA transfer.
    OamDma = 5,
    /// The APU's next output transition.
    Apu = 6,
    /// Anything that can raise an interrupt without a clock of its own — today, joypad input
    /// arriving from the host.
    Interrupt = 7,
}

pub const N_EV: usize = 8;

/// "This event will never happen." Chosen so it loses every `<` comparison without a branch.
pub const DISABLED: u64 = u64::MAX;

#[derive(Debug, Clone, Copy, Eq, Encode, Decode)]
pub struct Schedule {
    when: [u64; N_EV],
    /// Cached `when.iter().min()`. Derived — see the [`PartialEq`] impl.
    next: u64,
    /// Which entry `next` came from. Derived, and **tie-broken by update order rather than by
    /// index**, which is why it takes no part in equality.
    next_id: u8,
}

impl Default for Schedule {
    fn default() -> Self {
        Self { when: [DISABLED; N_EV], next: DISABLED, next_id: 0 }
    }
}

/// Only `when` is state; `next`/`next_id` are a cache of its minimum. Two schedules that agree on
/// `when` behave identically, but their `next_id` can differ if they reached the same set of
/// deadlines in a different order — so comparing it would make [`crate::game_boy::GameBoy`]
/// equality depend on history rather than on state.
impl PartialEq for Schedule {
    fn eq(&self, other: &Self) -> bool {
        self.when == other.when
    }
}

impl Schedule {
    /// Schedule `e` for absolute m-cycle `t`, or [`DISABLED`] to cancel it.
    #[inline(always)]
    pub fn set(&mut self, e: Ev, t: u64) {
        self.when[e as usize] = t;
        if t <= self.next {
            self.next = t;
            self.next_id = e as u8;
        } else if self.next_id == e as u8 {
            // We just relaxed the entry that *was* the minimum, so the cache is stale.
            self.recompute();
        }
    }

    #[inline(always)]
    pub fn get(&self, e: Ev) -> u64 {
        self.when[e as usize]
    }

    /// The absolute time of the earliest scheduled event, or [`DISABLED`] if there is none.
    #[inline(always)]
    pub fn next(&self) -> u64 {
        self.next
    }

    #[inline]
    fn recompute(&mut self) {
        let (mut best, mut id) = (DISABLED, 0u8);
        for (i, &t) in self.when.iter().enumerate() {
            if t < best {
                best = t;
                id = i as u8;
            }
        }
        self.next = best;
        self.next_id = id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_schedule_has_nothing_due() {
        assert_eq!(Schedule::default().next(), DISABLED);
    }

    #[test]
    fn next_tracks_the_minimum_through_both_paths() {
        let mut s = Schedule::default();
        s.set(Ev::Timer, 100);
        assert_eq!(s.next(), 100);

        // Tightening some other entry takes the O(1) fast path.
        s.set(Ev::Video, 40);
        assert_eq!(s.next(), 40);

        // Relaxing an entry that is *not* the minimum leaves the cache alone.
        s.set(Ev::Timer, 900);
        assert_eq!(s.next(), 40);

        // Relaxing the minimum itself forces a recompute — this is the case a naive `set` gets
        // wrong, leaving `next` pointing at a time that is no longer scheduled.
        s.set(Ev::Video, 5000);
        assert_eq!(s.next(), 900);

        // ...and cancelling it entirely falls back to whatever is left.
        s.set(Ev::Timer, DISABLED);
        assert_eq!(s.next(), 5000);
        s.set(Ev::Video, DISABLED);
        assert_eq!(s.next(), DISABLED);
    }

    /// Every discriminant must index its own slot, or one peripheral would overwrite another's
    /// deadline. Cheap to assert, and the enum is `#[repr(u8)]` precisely so it can be.
    #[test]
    fn every_event_has_its_own_slot() {
        let all = [
            Ev::EndOfSlice, Ev::Video, Ev::Timer, Ev::Divider,
            Ev::Serial, Ev::OamDma, Ev::Apu, Ev::Interrupt,
        ];
        assert_eq!(all.len(), N_EV);
        for (i, e) in all.into_iter().enumerate() {
            let mut s = Schedule::default();
            s.set(e, i as u64);
            assert_eq!(s.get(e), i as u64);
            assert_eq!(s.next(), i as u64, "{e:?} did not reach the minimum");
            assert_eq!(e as usize, i, "{e:?} has the wrong discriminant");
        }
    }

    /// The cache is not state. A schedule reached by a different route but holding the same
    /// deadlines has to compare equal, or restoring a save state would never match the machine it
    /// was taken from.
    #[test]
    fn equality_ignores_the_cached_minimum() {
        let mut a = Schedule::default();
        a.set(Ev::Timer, 10);
        a.set(Ev::Video, 10);

        let mut b = Schedule::default();
        b.set(Ev::Video, 10);
        b.set(Ev::Timer, 10);

        assert_ne!(a.next_id, b.next_id, "test is vacuous unless the caches differ");
        assert_eq!(a, b);
    }
}

//! Two tours played at once, one on each game, kept together a step at a time: a side that has
//! answered more steps than the other waits at its next overworld decision, the game running on
//! under it, until the other has answered as many. Finer units than a step drift apart, as the two
//! games roll different random numbers.

use std::sync::atomic::Ordering;

use crate::pokemon::GameState;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::agent::AgentEvent;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::observe::Readout;
use crate::pokemon::policy::{FieldMove, Jam, Policy, Trace};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::world_graph::WorldGraph;
use crate::tour::brain::TourProgress;

/// How far one side has got, as the gate reads it.
pub trait Progress {
    fn steps(&self) -> usize;
    /// Finished, stuck or failed: a side that takes no more steps holds nobody back.
    fn is_over(&self) -> bool;
}

impl Progress for TourProgress {
    fn steps(&self) -> usize {
        self.steps_done.load(Ordering::SeqCst)
    }

    fn is_over(&self) -> bool {
        TourProgress::is_over(self)
    }
}

/// Whether the side at `mine` waits for the side at `other`: while it is ahead of a side still
/// going. A count that goes back, a walk given up on putting a brain back on its step, needs nothing
/// more, since two sides cannot each be ahead of the other.
pub fn waits(mine: &impl Progress, other: &impl Progress) -> bool {
    !other.is_over() && mine.steps() > other.steps()
}

/// A tour's policy held by [`waits`]: above `LlmPolicy`, so its brain is not asked for the next
/// step while this side waits. Everything else goes straight through, `trace_to` included.
pub struct Paced<P: Progress> {
    inner: Box<dyn Policy>,
    mine: P,
    other: P,
}

impl<P: Progress> Paced<P> {
    pub fn new(inner: Box<dyn Policy>, mine: P, other: P) -> Self {
        Self { inner, mine, other }
    }

    fn held(&self) -> bool {
        waits(&self.mine, &self.other)
    }
}

impl<P: Progress> Policy for Paced<P> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn pick_overworld_action(&mut self, state: &GameState, world_graph: &WorldGraph) -> Option<OverworldAction> {
        if self.held() {
            return None;
        }
        self.inner.pick_overworld_action(state, world_graph)
    }

    fn pick_field_move(&mut self, state: &GameState) -> Option<FieldMove> {
        if self.held() {
            return None;
        }
        self.inner.pick_field_move(state)
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        self.inner.pick_battle_action(state)
    }

    fn player_name(&self) -> Option<String> {
        self.inner.player_name()
    }

    fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>> {
        self.inner.pick_nickname(species)
    }

    fn pick_mart_purchase(&mut self, state: &GameState) -> Option<Option<BagItem>> {
        self.inner.pick_mart_purchase(state)
    }

    fn next_mart_purchase(&mut self) -> Option<BagItem> {
        self.inner.next_mart_purchase()
    }

    fn pick_move_to_forget(&mut self, party_slot: usize, current_moves: &[PokemonMove], new_move: PokemonMoveName)
        -> Option<Option<usize>>
    {
        self.inner.pick_move_to_forget(party_slot, current_moves, new_move)
    }

    fn pick_move_to_mimic(&mut self, state: &GameState, enemy_moves: &[PokemonMove]) -> Option<usize> {
        self.inner.pick_move_to_mimic(state, enemy_moves)
    }

    fn on_event(&mut self, event: &AgentEvent) {
        self.inner.on_event(event)
    }

    fn service_tools(&mut self, state: &GameState, readout: &dyn Readout, graph: &WorldGraph) {
        self.inner.service_tools(state, readout, graph)
    }

    fn stuck_timeout(&self) -> Option<std::time::Duration> {
        self.inner.stuck_timeout()
    }

    fn pick_unstick(&mut self, state: &GameState, jam: Jam<'_>) {
        self.inner.pick_unstick(state, jam)
    }

    fn restart(&mut self, run_dir: Option<&std::path::Path>) {
        self.inner.restart(run_dir)
    }

    fn trace_to(&mut self, trace: Trace) {
        self.inner.trace_to(trace)
    }

    fn clear_conversation(&mut self, run_dir: Option<&std::path::Path>) -> Result<(), String> {
        self.inner.clear_conversation(run_dir)
    }

    fn take_manual_input(&mut self) -> Vec<gb::joypad::JoypadButton> {
        self.inner.take_manual_input()
    }

    fn is_exhausted(&self) -> bool {
        self.inner.is_exhausted()
    }

    fn steps_remaining(&self) -> Option<usize> {
        self.inner.steps_remaining()
    }

    fn current_step_is_long_running(&self) -> bool {
        self.inner.current_step_is_long_running()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    /// A side's counters, shared as `TourProgress`'s are between the brain and the gate.
    #[derive(Clone, Default)]
    struct Side {
        steps: Arc<AtomicUsize>,
        over: Arc<AtomicBool>,
    }

    impl Progress for Side {
        fn steps(&self) -> usize {
            self.steps.load(Ordering::SeqCst)
        }

        fn is_over(&self) -> bool {
            self.over.load(Ordering::SeqCst)
        }
    }

    impl Side {
        fn step(&self) {
            self.steps.fetch_add(1, Ordering::SeqCst);
        }

        fn end(&self) {
            self.over.store(true, Ordering::SeqCst);
        }
    }

    const TOTAL: usize = 50;

    /// Ticks until both are over, each side taking a step every `every` ticks unless it waits, and
    /// `meddle` run after every tick. Panics on a deadlock.
    fn play(sides: [&Side; 2], every: [usize; 2], mut meddle: impl FnMut(usize, [&Side; 2])) -> usize {
        for tick in 1..100_000 {
            for (i, side) in sides.into_iter().enumerate() {
                let other = sides[1 - i];
                if side.is_over() || tick % every[i] != 0 || waits(side, other) {
                    continue;
                }
                side.step();
                if side.steps() == TOTAL {
                    side.end();
                }
            }
            meddle(tick, sides);
            if sides.iter().all(|side| side.is_over()) {
                return tick;
            }
        }
        panic!("deadlocked at {} and {}", sides[0].steps(), sides[1].steps());
    }

    #[test]
    fn two_sides_at_different_speeds_never_differ_by_more_than_a_step() {
        for every in [[1, 1], [1, 7], [5, 2], [3, 13]] {
            let (fast, slow) = (Side::default(), Side::default());
            let mut widest = 0;
            play([&fast, &slow], every, |_, [a, b]| widest = widest.max(a.steps().abs_diff(b.steps())));
            assert_eq!((fast.steps(), slow.steps()), (TOTAL, TOTAL));
            assert!(widest <= 1, "{every:?}: {widest} apart");
        }
    }

    #[test]
    fn a_side_that_finishes_first_holds_the_other_back_from_nothing() {
        let (ahead, behind) = (Side::default(), Side::default());
        ahead.steps.store(TOTAL - 1, Ordering::SeqCst);
        play([&ahead, &behind], [1, 3], |_, _| {});
        assert_eq!(behind.steps(), TOTAL);
    }

    #[test]
    fn a_side_that_stalls_for_good_releases_the_other() {
        let (stalled, going) = (Side::default(), Side::default());
        play([&stalled, &going], [1, 2], |_, [stalled, _]| if stalled.steps() == 10 { stalled.end() });
        assert_eq!((stalled.steps(), going.steps()), (10, TOTAL));
    }

    #[test]
    fn a_count_that_goes_back_is_waited_for_again() {
        let (slipping, steady) = (Side::default(), Side::default());
        let mut slipped = false;
        let mut stepped_ahead = false;
        let mut before = 0;
        play([&slipping, &steady], [4, 1], |_, [slipping, steady]| {
            // Taking a step while ahead is what the gate forbids, wherever the count went.
            stepped_ahead |= steady.steps() > before && before > slipping.steps();
            before = steady.steps();
            if !slipped && slipping.steps() == 20 {
                slipping.steps.store(18, Ordering::SeqCst);
                slipped = true;
            }
        });
        assert!(slipped);
        assert!(!stepped_ahead, "the steady side stepped on while ahead");
        assert_eq!((slipping.steps(), steady.steps()), (TOTAL, TOTAL));
    }

    /// An inner policy that answers every field-move decision, counting the asks.
    struct Answers(Arc<AtomicUsize>);

    impl Policy for Answers {
        fn name(&self) -> &'static str {
            "answers"
        }

        fn pick_overworld_action(&mut self, _: &GameState, _: &WorldGraph) -> Option<OverworldAction> {
            None
        }

        fn pick_battle_action(&mut self, _: &GameState) -> Option<BattleAction> {
            None
        }

        fn pick_field_move(&mut self, _: &GameState) -> Option<FieldMove> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Some(FieldMove::CutTree)
        }
    }

    #[test]
    fn a_held_side_does_not_ask_its_brain() {
        let (mine, other) = (Side::default(), Side::default());
        let asked = Arc::new(AtomicUsize::new(0));
        let mut paced = Paced::new(Box::new(Answers(Arc::clone(&asked))), mine.clone(), other.clone());
        let state = GameState::default();
        mine.step();
        assert_eq!(paced.pick_field_move(&state), None);
        assert_eq!(asked.load(Ordering::SeqCst), 0);
        other.step();
        assert_eq!(paced.pick_field_move(&state), Some(FieldMove::CutTree));
        mine.step();
        other.end();
        assert_eq!(paced.pick_field_move(&state), Some(FieldMove::CutTree), "a side that is over holds nobody");
        assert_eq!(asked.load(Ordering::SeqCst), 2);
    }
}

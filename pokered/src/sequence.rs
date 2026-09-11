//! The runner for a long linear thing (a battle turn, a cutscene); each system has its own `Step`.

use serde::{Deserialize, Serialize};
use crate::mode::Ctx;

pub trait Step {
    fn run(&self, ctx: &mut Ctx) -> StepResult;
}

pub enum StepResult {
    Next,
    /// Frames until the next step runs.
    Wait(u16),
    /// Run this step again next frame.
    Again,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sequence<S> {
    steps: Vec<S>,
    cursor: usize,
    wait: u16,
}

impl<S: Step> Sequence<S> {
    pub fn new(steps: Vec<S>) -> Self {
        Self { steps, cursor: 0, wait: 0 }
    }

    pub fn is_done(&self) -> bool {
        self.cursor >= self.steps.len()
    }

    pub fn current(&self) -> Option<&S> {
        self.steps.get(self.cursor)
    }

    pub fn update(&mut self, ctx: &mut Ctx) {
        if self.wait > 0 {
            self.wait -= 1;
            if self.wait > 0 {
                return;
            }
            self.cursor += 1;
        }
        while let Some(step) = self.steps.get(self.cursor) {
            match step.run(ctx) {
                StepResult::Next => self.cursor += 1,
                StepResult::Wait(0) => self.cursor += 1,
                StepResult::Wait(frames) => {
                    self.wait = frames;
                    return;
                }
                StepResult::Again => return,
            }
        }
    }
}

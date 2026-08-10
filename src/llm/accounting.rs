//! **W6 / §9** — what the run has spent, and how full the context is.
//!
//! Two numbers that look the same and are not:
//!
//! - **What the endpoint reported.** `usage` on the last response: ground truth for the request that
//!   was actually sent, and what the UI shows. It says nothing about the messages appended since.
//! - **What we can measure ourselves.** [`Message::approximate_tokens`] over the current history,
//!   which can be recomputed at any moment — after a tool result, after an eviction — but is a
//!   character count and wrong by tens of percent.
//!
//! ⚠️ **Compaction has to be decided on one scale, and mixing them is how it goes wrong.** If the
//! endpoint says 90 k and our own count says 40 k, evicting three screenshots takes the estimate to
//! 39 k — below any threshold expressed in reported tokens — and stage 2 never runs on a context
//! that is genuinely nearly full. So every reported figure calibrates the estimator
//! ([`Accounting::calibration`]) and every *decision* is taken against the calibrated estimate. When
//! the endpoint reports nothing at all the ratio stays 1.0 and this degrades to the plain estimate,
//! which is the W4 behaviour.

use crate::llm::protocol::{Message, Usage};
use crate::web::published::UsageView;

/// Bounds on the calibration ratio. A endpoint that counts images, system overhead or a chat
/// template very differently from us is normal and worth tracking; one that appears to be off by a
/// factor of ten is a misunderstanding, and clamping keeps a misunderstanding from turning into
/// either a compaction that never fires or one that fires every turn.
const MIN_CALIBRATION: f64 = 0.25;
const MAX_CALIBRATION: f64 = 8.0;

#[derive(Debug, Clone)]
pub struct Accounting {
    limit: u64,
    /// Prompt + completion of the most recent response — the occupancy the UI shows.
    context_tokens: u64,
    prompt_total: u64,
    completion_total: u64,
    /// Completions billed this run. Larger than the number of turns: a turn that reads before it
    /// decides is two or more.
    completions: u64,
    /// Whether the most recent figures came from [`Usage::estimate`] rather than the endpoint.
    estimated: bool,
    /// Reported prompt tokens ÷ our own estimate of the very same messages. See the module note.
    calibration: f64,
}

impl Accounting {
    pub fn new(limit: u64) -> Self {
        Self {
            limit: limit.max(1),
            context_tokens: 0,
            prompt_total: 0,
            completion_total: 0,
            completions: 0,
            estimated: false,
            calibration: 1.0,
        }
    }

    /// Fold in one response. `sent` is the history as it went out — not as it stands now — because
    /// that is what the reported `prompt_tokens` counted.
    pub fn record(&mut self, usage: Usage, sent: &[Message]) {
        let ours: u64 = sent.iter().map(Message::approximate_tokens).sum();
        if !usage.estimated && usage.prompt_tokens > 0 && ours > 0 {
            self.calibration =
                (usage.prompt_tokens as f64 / ours as f64).clamp(MIN_CALIBRATION, MAX_CALIBRATION);
        }
        self.context_tokens = usage.prompt_tokens + usage.completion_tokens;
        self.prompt_total += usage.prompt_tokens;
        self.completion_total += usage.completion_tokens;
        self.completions += 1;
        self.estimated = usage.estimated;
    }

    /// What `messages` would cost, on the endpoint's scale as far as we can tell.
    pub fn tokens_in(&self, messages: &[Message]) -> u64 {
        let ours: u64 = messages.iter().map(Message::approximate_tokens).sum();
        (ours as f64 * self.calibration).round() as u64
    }

    /// [`Self::tokens_in`] as a fraction of the context limit. The number compaction triggers on.
    pub fn occupancy(&self, messages: &[Message]) -> f64 {
        self.tokens_in(messages) as f64 / self.limit as f64
    }

    /// `true` once anything has been reported or estimated, which is what stops the UI showing a
    /// context gauge reading 0 % before the first turn has finished.
    pub fn has_figures(&self) -> bool {
        self.completions > 0
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    pub fn view(&self) -> UsageView {
        UsageView {
            context_tokens: self.context_tokens,
            context_limit: self.limit,
            prompt_tokens: self.prompt_total,
            completion_tokens: self.completion_total,
            completions: self.completions,
            estimated: self.estimated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reported(prompt: u64, completion: u64) -> Usage {
        Usage { prompt_tokens: prompt, completion_tokens: completion, total_tokens: prompt + completion, estimated: false }
    }

    fn history(chars: usize) -> Vec<Message> {
        vec![Message::user("a".repeat(chars))]
    }

    /// The totals are cumulative across the run and the context figure is not — one is a bill, the
    /// other is a gauge.
    #[test]
    fn totals_accumulate_while_the_context_figure_is_the_latest_one() {
        let mut accounting = Accounting::new(100_000);
        assert!(!accounting.has_figures(), "nothing has been sent yet");

        accounting.record(reported(1_000, 50), &history(3_700));
        accounting.record(reported(1_400, 20), &history(5_180));

        let view = accounting.view();
        assert_eq!(view.context_tokens, 1_420, "the gauge is the last request, not the sum");
        assert_eq!(view.prompt_tokens, 2_400);
        assert_eq!(view.completion_tokens, 70);
        assert_eq!(view.completions, 2, "a turn can be several completions and the bill counts them");
        assert!(!view.estimated);
        assert!(accounting.has_figures());
    }

    /// ⚠️ The module's whole reason for existing: a decision taken after the history has changed must
    /// be on the endpoint's scale, not ours. Here the endpoint counts 3× what we do.
    #[test]
    fn the_estimator_is_calibrated_against_what_the_endpoint_reported() {
        let mut accounting = Accounting::new(30_000);
        let sent = history(37_000); // 10 000 tokens by our count
        accounting.record(reported(30_000, 100), &sent);

        assert_eq!(accounting.tokens_in(&sent), 30_000, "the estimate now agrees with the report");
        assert!((accounting.occupancy(&sent) - 1.0).abs() < 0.01);

        // Halve the history — as an eviction would — and the occupancy halves *on the same scale*.
        assert!((accounting.occupancy(&history(18_500)) - 0.5).abs() < 0.01);
    }

    /// An endpoint that reports nothing leaves the estimator alone, which is exactly W4's behaviour.
    #[test]
    fn an_endpoint_that_reports_nothing_degrades_to_the_plain_estimate() {
        let mut accounting = Accounting::new(10_000);
        let sent = history(37_000);
        let mut usage = Usage::estimate(&sent, &Default::default());
        assert!(usage.estimated);
        usage.prompt_tokens = 99_999; // even a nonsense one, because it is not a measurement
        accounting.record(usage, &sent);

        assert_eq!(accounting.tokens_in(&sent), 10_000, "10 000 characters-worth, uncalibrated");
        assert!(accounting.view().estimated, "and the UI is told the numbers are a guess");
    }

    /// A wildly disagreeing endpoint is clamped rather than believed: the failure it would otherwise
    /// cause is a compaction that never fires, or one that fires on every turn.
    #[test]
    fn an_absurd_ratio_is_clamped() {
        let mut accounting = Accounting::new(1_000);
        let sent = history(3_700); // 1 000 tokens by our count
        accounting.record(reported(1_000_000, 0), &sent);
        assert_eq!(accounting.tokens_in(&sent), 8_000, "clamped at 8×");

        let mut accounting = Accounting::new(1_000);
        accounting.record(reported(1, 0), &sent);
        assert_eq!(accounting.tokens_in(&sent), 250, "…and at a quarter");
    }
}

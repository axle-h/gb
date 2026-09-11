//! What the run has spent, and how full the context is.

use crate::llm::protocol::{Message, Usage};
use crate::published::UsageView;

/// Bounds on the calibration ratio.
const MIN_CALIBRATION: f64 = 0.25;
const MAX_CALIBRATION: f64 = 8.0;

#[derive(Debug, Clone)]
pub struct Accounting {
    limit: u64,
    /// Prompt + completion of the most recent response — the occupancy the UI shows.
    context_tokens: u64,
    prompt_total: u64,
    completion_total: u64,
    /// Completions billed this run.
    completions: u64,
    /// Whether the most recent figures came from [`Usage::estimate`] rather than the endpoint.
    estimated: bool,
    /// Reported prompt tokens ÷ our own estimate of the very same messages.
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

    /// The accounting for a conversation just read back by [`crate::llm::history`].
    pub fn resumed(limit: u64, calibration: f64) -> Self {
        Self {
            // Off disk, so checked rather than trusted: `clamp` propagates a NaN.
            calibration: match calibration.is_finite() {
                true => calibration.clamp(MIN_CALIBRATION, MAX_CALIBRATION),
                false => 1.0,
            },
            ..Self::new(limit)
        }
    }

    /// What the endpoint counts over what we estimate, persisted by [`crate::llm::history`].
    pub fn calibration(&self) -> f64 {
        self.calibration
    }

    /// Fold in one response; `sent` is the history as it went out, which `prompt_tokens` counted.
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

    /// Whether anything has been counted, so the UI shows no 0 % gauge before the first turn.
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

    #[test]
    fn the_estimator_is_calibrated_against_what_the_endpoint_reported() {
        let mut accounting = Accounting::new(30_000);
        let sent = history(37_000); // 10 000 tokens by our count
        accounting.record(reported(30_000, 100), &sent);

        assert_eq!(accounting.tokens_in(&sent), 30_000, "the estimate now agrees with the report");
        assert!((accounting.occupancy(&sent) - 1.0).abs() < 0.01);

        // Halving the history halves the occupancy on the same scale.
        assert!((accounting.occupancy(&history(18_500)) - 0.5).abs() < 0.01);
    }

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

    /// Believed, a wild ratio makes compaction never fire, or fire on every turn.
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

    #[test]
    fn a_resumed_run_measures_its_restored_history_on_the_endpoints_scale_not_ours() {
        // Sized so the two land either side of the default threshold.
        let limit = 10_000;
        let sent: Vec<Message> = (0..12)
            .map(|_| Message::user("x".repeat(1_000)))
            .collect();

        let cold = Accounting::new(limit);
        let warm = Accounting::resumed(limit, 3.0);
        let threshold = crate::llm::config::DEFAULT_COMPACT_ABOVE;

        assert!(
            cold.occupancy(&sent) < threshold,
            "at 1.0 this history looks like it fits: {}",
            cold.occupancy(&sent)
        );
        assert!(
            warm.occupancy(&sent) >= threshold,
            "and on the endpoint's own scale it does not: {}",
            warm.occupancy(&sent)
        );
    }

    #[test]
    fn a_calibration_read_off_disk_is_checked_rather_than_trusted() {
        assert_eq!(Accounting::resumed(1_000, 3.0).calibration(), 3.0, "an ordinary value is kept");
        assert_eq!(Accounting::resumed(1_000, 1e9).calibration(), MAX_CALIBRATION);
        assert_eq!(Accounting::resumed(1_000, 0.0001).calibration(), MIN_CALIBRATION);
        assert_eq!(Accounting::resumed(1_000, f64::NAN).calibration(), 1.0);
        assert_eq!(Accounting::resumed(1_000, f64::INFINITY).calibration(), 1.0);

        // Totals do not come back: `RunProgress` rebases them onto `meta.json`, so they would
        // count twice.
        let resumed = Accounting::resumed(1_000, 3.0);
        assert!(!resumed.has_figures(), "a resumed run has spent nothing yet");
        assert_eq!(resumed.view().prompt_tokens, 0);
    }
}

//! Harvested tier-1 fixtures: one `{input, output, rng}` a line.

use serde::de::DeserializeOwned;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case<I, O> {
    input: I,
    output: O,
    #[serde(default)]
    rng: Vec<u8>,
}

/// Every case, as `(input, output, rng)`.
pub fn cases<I: DeserializeOwned, O: DeserializeOwned>(jsonl: &str) -> Vec<(I, O, Vec<u8>)> {
    let cases: Vec<_> = jsonl.lines()
        .map(|line| serde_json::from_str::<Case<I, O>>(line).unwrap())
        .map(|case| (case.input, case.output, case.rng))
        .collect();
    assert!(cases.len() >= 100, "only {} cases", cases.len());
    cases
}

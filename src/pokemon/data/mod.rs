use rand::prelude::StdRng;
use rand::seq::SliceRandom;
use rand::{rng, SeedableRng};

static POKEMON_NAMES_CSV: &str = include_str!("pokemon-names.csv");

/// Picks Pokémon names one at a time in a random order, with no repeats until
/// every name has been picked (then it reshuffles and starts over).
#[derive(Debug, Clone)]
pub struct PokemonNamePicker {
    rng: StdRng,

    /// Shuffled subset that hasn't been picked yet this cycle.
    remaining: Vec<(usize, usize)>,
}

impl Default for PokemonNamePicker {
    fn default() -> Self {
        Self::seed_from_u64(42)
    }
}

impl PokemonNamePicker {
    pub fn seed_from_u64(seed: u64) -> PokemonNamePicker {
        let mut picker = Self {
            rng: StdRng::seed_from_u64(seed),
            remaining: Vec::new(),
        };
        picker.assert_not_empty();
        picker
    }

    /// Returns the next random name.  The returned `&str` points directly into
    /// the static CSV string — no allocation.
    pub fn pick(&mut self) -> &'static str {
        self.assert_not_empty();
        let (start, end) = self.remaining.pop().expect("remaining is non-empty");
        &POKEMON_NAMES_CSV[start..end]
    }

    fn assert_not_empty(&mut self) {
        if !self.remaining.is_empty() {
            return;
        }

        self.remaining = Self::build_offsets();
        self.remaining.shuffle(&mut self.rng);
    }

    fn build_offsets() -> Vec<(usize, usize)> {
        let base = POKEMON_NAMES_CSV.as_ptr() as usize;
        POKEMON_NAMES_CSV
            .split('\n')
            .filter_map(|line| {
                // `trim_matches` returns a sub-slice of the original static str,
                // so pointer arithmetic gives us the exact byte offsets.
                let trimmed = line.trim_matches(|c: char| c.is_ascii_whitespace());
                if trimmed.is_empty() {
                    return None;
                }
                let start = trimmed.as_ptr() as usize - base;
                Some((start, start + trimmed.len()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn picks_names_without_duplicates_in_one_cycle() {
        let mut picker = PokemonNamePicker::default();
        let total = picker.remaining.len();

        let mut seen = HashSet::new();
        for _ in 0..total {
            let name = picker.pick();
            assert!(!name.is_empty(), "name should not be empty");
            assert!(seen.insert(name), "duplicate name picked: {name}");
        }
        assert_eq!(seen.len(), total, "should have picked every name exactly once");
        assert_eq!(picker.remaining.len(), 0, "should be no more names left");

        // The second cycle should also yield a valid name
        let name = picker.pick();
        assert!(!name.is_empty());
        assert!(picker.remaining.len() > 0, "should have refilled");
    }

    #[test]
    fn picks_are_from_the_known_name_list() {
        let mut picker = PokemonNamePicker::default();
        // Build a reference set directly from the static string (no Vec<String> needed)
        let all: HashSet<&str> = POKEMON_NAMES_CSV
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();

        for _ in 0..20 {
            let name = picker.pick();
            assert!(all.contains(name), "unexpected name: {name}");
        }
    }
}


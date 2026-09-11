use rand::{RngExt, SeedableRng};
use rand_pcg::Pcg64;
use serde::{Deserialize, Serialize};

/// `Random`. A caller that also reads `hRandomSub` takes a second byte.
pub trait Rng {
    fn random(&mut self) -> u8;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameRng {
    Seeded(Pcg64),
    /// What the emulator's `Random` returned, for a harvested fixture.
    Tape { bytes: Vec<u8>, cursor: usize },
}

impl GameRng {
    pub fn from_entropy() -> Self {
        Self::Seeded(Pcg64::from_rng(&mut rand::rng()))
    }

    pub fn seeded(seed: u64) -> Self {
        Self::Seeded(Pcg64::seed_from_u64(seed))
    }

    pub fn tape(bytes: Vec<u8>) -> Self {
        Self::Tape { bytes, cursor: 0 }
    }
}

impl Rng for GameRng {
    fn random(&mut self) -> u8 {
        match self {
            Self::Seeded(rng) => rng.random(),
            Self::Tape { bytes, cursor } => {
                let byte = *bytes.get(*cursor)
                    .unwrap_or_else(|| panic!("the RNG tape ran out after {} bytes", bytes.len()));
                *cursor += 1;
                byte
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_is_a_sequence() {
        let (mut a, mut b) = (GameRng::seeded(7), GameRng::seeded(7));
        let draw = |rng: &mut GameRng| (0..64).map(|_| rng.random()).collect::<Vec<_>>();
        assert_eq!(draw(&mut a), draw(&mut b));
    }

    #[test]
    fn a_tape_plays_back_in_order() {
        let mut rng = GameRng::tape(vec![3, 1, 4]);
        assert_eq!([rng.random(), rng.random(), rng.random()], [3, 1, 4]);
    }

    #[test]
    #[should_panic(expected = "ran out after 1 bytes")]
    fn a_short_tape_says_so() {
        let mut rng = GameRng::tape(vec![0]);
        rng.random();
        rng.random();
    }
}

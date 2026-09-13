use pokered::systems::field_moves::{FieldMoves, FieldMovesInput, DEFAULT_LEFTMOST};
use crate::pokemon::symbols::pokered_symbols;
use super::Oracle;

/// `GetMonFieldMoves` reads the mon's moves and adds to what it finds, so the caller's zeroing has
/// to be done here too: `DisplayFieldMoveMonMenu` clears the four slots, the count and the column
/// before it calls.
fn get_mon_field_moves(oracle: &mut Oracle, input: FieldMovesInput) -> FieldMoves {
    oracle.write(pokered_symbols::wWhichPokemon, &[0]);
    oracle.write(pokered_symbols::wPartyMon1Moves, &input.moves);
    // Four slots and the count that follows them, in one write.
    oracle.write(pokered_symbols::wFieldMoves, &[0; 5]);
    oracle.write(pokered_symbols::wFieldMovesLeftmostXCoord, &[DEFAULT_LEFTMOST]);
    let called = oracle.call(pokered_symbols::GetMonFieldMoves);
    assert!(called.rng.is_empty());
    let names = oracle.read(pokered_symbols::wFieldMoves, 4);
    let count = oracle.read(pokered_symbols::wNumFieldMoves, 1)[0] as usize;
    FieldMoves {
        names: names[..count.min(4)].to_vec(),
        leftmost: oracle.read(pokered_symbols::wFieldMovesLeftmostXCoord, 1)[0],
    }
}

#[cfg(feature = "slow-tests")]
fn inputs() -> Vec<FieldMovesInput> {
    use poke_core::move_name::PokemonMoveName as Move;
    use rand::{RngExt, SeedableRng};
    const FIELD: [Move; 8] = [Move::Cut, Move::Fly, Move::Surf, Move::Strength, Move::Flash,
                              Move::Dig, Move::Teleport, Move::Softboiled];
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xF1E1D);
    // Every field move alone, every pair, and the empty-slot cases that end the scan early.
    let mut inputs: Vec<FieldMovesInput> = FIELD.iter()
        .map(|&one| FieldMovesInput { moves: [one as u8, 0, 0, 0] })
        .chain(FIELD.iter().flat_map(|&one| FIELD.map(move |two| FieldMovesInput {
            moves: [one as u8, two as u8, 0, 0],
        })))
        .collect();
    inputs.push(FieldMovesInput { moves: [0; 4] });
    inputs.push(FieldMovesInput { moves: [0, Move::Cut as u8, 0, 0] });
    inputs.push(FieldMovesInput { moves: [Move::Cut as u8, 0, Move::Fly as u8, 0] });
    while inputs.len() < 400 {
        inputs.push(FieldMovesInput { moves: std::array::from_fn(|_| rng.random_range(0..=165)) });
    }
    inputs
}

/// The oracle's own check: a mon that knows nothing usable outside a battle, and one that does.
#[test]
fn the_moves_it_finds_are_the_field_moves() {
    use poke_core::move_name::PokemonMoveName as Move;
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    let none = FieldMovesInput { moves: [Move::Tackle as u8, 0, 0, 0] };
    assert_eq!(get_mon_field_moves(&mut oracle, none), FieldMoves::default());
    let cut_and_strength = FieldMovesInput { moves: [Move::Cut as u8, Move::Strength as u8, 0, 0] };
    assert_eq!(get_mon_field_moves(&mut oracle, cut_and_strength),
        FieldMoves { names: vec![1, 5], leftmost: 0x0A }, "STRENGTH reaches further left than CUT");
}

#[test]
#[cfg(feature = "slow-tests")]
fn the_port_matches_the_cartridge() {
    use pokered::systems::field_moves::field_moves;
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    for input in inputs() {
        assert_eq!(field_moves(input.moves), get_mon_field_moves(&mut oracle, input), "{:?}", input.moves);
    }
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/field_moves/get_mon_field_moves.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_get_mon_field_moves() {
    use super::{write_fixture, Case};
    let mut oracle = Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    let cases: Vec<Case<FieldMovesInput, FieldMoves>> = inputs().into_iter()
        .map(|input| Case { input, output: get_mon_field_moves(&mut oracle, input), rng: vec![] })
        .collect();
    write_fixture("field_moves", "get_mon_field_moves", &cases);
}

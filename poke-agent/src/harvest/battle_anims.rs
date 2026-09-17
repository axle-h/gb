//! The battle animations' pure routines: `DrawFrameBlock`, `LoadSubanimation`, `GetMoveSound` and
//! the three `GetBattleTransitionID_*` bits.

use poke_core::map::Map;
use poke_core::species::PokemonSpecies;
use pokered::modes::battle::transition::Choice;
use pokered::systems::battle::Side;
use serde::{Deserialize, Serialize};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer};
use super::Oracle;

const OAM_ENTRY: usize = 4;
/// `FRAMEBLOCKMODE_02`, which neither waits nor clears.
const MODE_02: u8 = 2;
const NUM_FRAME_BLOCKS: u8 = 0x7A;
const NUM_SUBANIMS: u8 = 86;
/// `wPartyMon1HP`'s offset to the level in a party struct.
const HP_TO_LEVEL: u16 = 0x21 - 1;

fn oracle() -> Oracle {
    Oracle::from_state(include_bytes!("../pokemon/data/at-celadon.bin"))
}

fn turn_byte(side: Side) -> u8 {
    match side {
        Side::Player => 0,
        Side::Enemy => 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameBlockInput {
    pub frame_block: u8,
    pub base_coord: u8,
    pub transform: u8,
}

/// `DrawFrameBlock` into `wShadowOAM`, as `[y, x, tile, attributes]`.
fn draw_frame_block(oracle: &mut Oracle, input: FrameBlockInput) -> Vec<[u8; 4]> {
    let pointers = sym::FrameBlockPointers;
    let table = poke_core::rom_gfx::rom_slice(pointers);
    let at = u16::from_le_bytes([table[input.frame_block as usize * 2], table[input.frame_block as usize * 2 + 1]]);
    let count = poke_core::rom_gfx::rom_slice(DmgPointer { bank: pointers.bank, address: at })[0] as usize;
    let (y, x) = poke_core::battle_anims::base_coord(input.base_coord);
    oracle.write(sym::wSubAnimTransform, &[input.transform]);
    oracle.write(sym::wBaseCoordY, &[y]);
    oracle.write(sym::wBaseCoordX, &[x]);
    oracle.write(sym::wFBDestAddr, &sym::wShadowOAM.address.to_be_bytes());
    oracle.write(sym::wFBMode, &[MODE_02]);
    oracle.write(sym::wAnimationID, &[1]);
    oracle.write(sym::wShadowOAM, &[0; 160]);
    oracle.registers_mut().set_bc(at);
    let called = oracle.call(sym::DrawFrameBlock);
    assert!(called.rng.is_empty());
    oracle.read(sym::wShadowOAM, count * OAM_ENTRY).chunks_exact(OAM_ENTRY).map(|entry| entry.try_into().unwrap()).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubanimInput {
    pub subanimation: u8,
    pub turn: Side,
}

/// `LoadSubanimation`: `wSubAnimTransform`, `wSubAnimCounter` and the entry `wSubAnimSubEntryAddr`
/// points at.
fn load_subanimation(oracle: &mut Oracle, input: SubanimInput) -> (u8, u8, usize) {
    let pointer = sym::SubanimationPointers.address + 2 * input.subanimation as u16;
    oracle.write(sym::wSubAnimAddrPtr, &pointer.to_le_bytes());
    oracle.write(sym::hWhoseTurn, &[turn_byte(input.turn)]);
    let called = oracle.call(sym::LoadSubanimation);
    assert!(called.rng.is_empty());
    let table = poke_core::rom_gfx::rom_slice(sym::SubanimationPointers);
    let start = u16::from_le_bytes([table[input.subanimation as usize * 2], table[input.subanimation as usize * 2 + 1]]);
    let entry = u16::from_le_bytes(oracle.read(sym::wSubAnimSubEntryAddr, 2).try_into().unwrap());
    let transform = oracle.read(sym::wSubAnimTransform, 1)[0];
    let counter = oracle.read(sym::wSubAnimCounter, 1)[0];
    (transform, counter, ((entry - (start + 1)) / 3) as usize)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveSoundInput {
    pub index: u8,
    pub animation: u8,
    pub turn: Side,
    pub player: PokemonSpecies,
    pub enemy: PokemonSpecies,
}

/// `GetMoveSound`: the sound in `a` and the two modifiers.
fn get_move_sound(oracle: &mut Oracle, input: MoveSoundInput) -> (u8, u8, u8) {
    oracle.write(sym::wAnimationID, &[input.animation]);
    oracle.write(sym::hWhoseTurn, &[turn_byte(input.turn)]);
    oracle.write(sym::wBattleMonSpecies, &[input.player as u8]);
    oracle.write(sym::wEnemyMonSpecies, &[input.enemy as u8]);
    oracle.write(sym::wFrequencyModifier, &[0x5A]);
    oracle.write(sym::wTempoModifier, &[0xA5]);
    oracle.registers_mut().a = input.index;
    let called = oracle.call(sym::GetMoveSound);
    assert!(called.rng.is_empty());
    (oracle.registers().a, oracle.read(sym::wFrequencyModifier, 1)[0], oracle.read(sym::wTempoModifier, 1)[0])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionInput {
    pub choice: Choice,
    /// The first party mon has fainted, and the second is the one whose level counts.
    pub lead_fainted: bool,
}

/// `GetBattleTransitionID_WildOrTrainer`, `_CompareLevels` and `_IsDungeonMap` in turn: the id in
/// `c` and `wBattleTransitionSpiralDirection`.
fn get_battle_transition_id(oracle: &mut Oracle, input: TransitionInput) -> (u8, u8) {
    let choice = input.choice;
    let opponent = if choice.trainer { 200 + 1 } else { PokemonSpecies::Pidgey as u8 };
    oracle.write(sym::wCurOpponent, &[opponent]);
    oracle.write(sym::wCurEnemyLevel, &[choice.enemy_level]);
    oracle.write(sym::wCurMap, &[choice.map as u8]);
    let hp = sym::wPartyMon1HP.address;
    let second = hp + 0x2C;
    let lead = if input.lead_fainted { (0u16, 1u8) } else { (7, choice.player_level) };
    oracle.write(DmgPointer { address: hp, ..sym::wPartyMon1HP }, &lead.0.to_be_bytes());
    oracle.write(DmgPointer { address: hp + HP_TO_LEVEL, ..sym::wPartyMon1HP }, &[lead.1]);
    oracle.write(DmgPointer { address: second, ..sym::wPartyMon1HP }, &9u16.to_be_bytes());
    oracle.write(DmgPointer { address: second + HP_TO_LEVEL, ..sym::wPartyMon1HP }, &[choice.player_level]);
    oracle.registers_mut().c = 0;
    for routine in [sym::GetBattleTransitionID_WildOrTrainer, sym::GetBattleTransitionID_CompareLevels, sym::GetBattleTransitionID_IsDungeonMap] {
        let called = oracle.call(routine);
        assert!(called.rng.is_empty());
    }
    (oracle.registers().c, oracle.read(sym::wBattleTransitionSpiralDirection, 1)[0])
}

#[cfg(feature = "slow-tests")]
fn species(rng: &mut impl rand::RngExt) -> PokemonSpecies {
    loop {
        if let Some(species) = PokemonSpecies::from_repr(rng.random_range(1..=190)) {
            return species;
        }
    }
}

#[cfg(feature = "slow-tests")]
fn transition_inputs() -> Vec<TransitionInput> {
    use rand::{RngExt, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x7A5);
    let mut inputs = vec![];
    for map in (0..=247u8).filter_map(Map::from_repr) {
        for trainer in [false, true] {
            let player_level: u8 = rng.random_range(1..=100);
            let enemy_level = match rng.random_range(0..4) {
                0 => player_level.wrapping_add(3),
                1 => player_level.wrapping_add(2),
                _ => rng.random_range(1..=100),
            };
            let lead_fainted = rng.random_range(0..5) == 0;
            inputs.push(TransitionInput { choice: Choice { trainer, enemy_level, player_level, map }, lead_fainted });
        }
    }
    inputs.push(TransitionInput { choice: Choice { trainer: true, enemy_level: 5, player_level: 254, map: Map::PalletTown }, lead_fainted: false });
    inputs
}

#[test]
fn a_stronger_trainer_in_a_dungeon_is_the_split() {
    let mut oracle = oracle();
    let choice = Choice { trainer: true, enemy_level: 13, player_level: 10, map: Map::MtMoon1F };
    assert_eq!(get_battle_transition_id(&mut oracle, TransitionInput { choice, lead_fainted: false }), (0b111, 0));
    assert_eq!(choice.kind().0, pokered::modes::battle::transition::Kind::Split);
}

#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "a tool: writes pokered/fixtures/battle_anims/*.jsonl under GB_REGEN_FIXTURES=1"]
fn harvest_battle_anims() {
    use rand::{RngExt, SeedableRng};
    use super::{write_fixture, Case};
    let mut oracle = oracle();
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xA41);

    let cases: Vec<Case<FrameBlockInput, Vec<[u8; 4]>>> = (1..NUM_FRAME_BLOCKS)
        .flat_map(|frame_block| (0..=4).map(move |transform| (frame_block, transform)))
        .map(|(frame_block, transform)| {
            let input = FrameBlockInput { frame_block, base_coord: rng.random_range(0..0xB0), transform };
            Case { input, output: draw_frame_block(&mut oracle, input), rng: vec![] }
        })
        .collect();
    write_fixture("battle_anims", "draw_frame_block", &cases);

    let cases: Vec<Case<SubanimInput, (u8, u8, usize)>> = (0..NUM_SUBANIMS)
        .flat_map(|subanimation| [Side::Player, Side::Enemy].map(|turn| SubanimInput { subanimation, turn }))
        .map(|input| Case { input, output: load_subanimation(&mut oracle, input), rng: vec![] })
        .collect();
    write_fixture("battle_anims", "load_subanimation", &cases);

    let mut inputs: Vec<MoveSoundInput> = (0..=0xA4u8)
        .map(|index| MoveSoundInput { index, animation: index + 1, turn: Side::Player, player: PokemonSpecies::Pikachu, enemy: PokemonSpecies::Onix })
        .collect();
    for animation in [0x2Du8, 0x2E] {
        for _ in 0..60 {
            let turn = if rng.random_range(0..2) == 0 { Side::Player } else { Side::Enemy };
            let (player, enemy) = (species(&mut rng), species(&mut rng));
            inputs.push(MoveSoundInput { index: rng.random_range(0..=0xA4), animation, turn, player, enemy });
        }
    }
    let cases: Vec<Case<MoveSoundInput, (u8, u8, u8)>> = inputs.into_iter()
        .map(|input| Case { input, output: get_move_sound(&mut oracle, input), rng: vec![] })
        .collect();
    write_fixture("battle_anims", "get_move_sound", &cases);

    let cases: Vec<Case<TransitionInput, (u8, u8)>> = transition_inputs().into_iter()
        .map(|input| Case { input, output: get_battle_transition_id(&mut oracle, input), rng: vec![] })
        .collect();
    write_fixture("battle_anims", "get_battle_transition_id", &cases);
}

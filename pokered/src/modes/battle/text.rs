//! The battle's texts. `<USER>` and `<TARGET>` are spliced in before printing, as
//! `PlaceMoveUsersName` would print them: the player's mon's name, or `Enemy ` and the enemy's.
//! Texts the cartridge continues with a `text_asm` are put together here from their far halves.

use poke_core::text_script::{decode, far_text, TextCommand};
use crate::systems::battle::Side;

const USER: u8 = 0x5A;
const TARGET: u8 = 0x59;
/// `EnemyText`.
const ENEMY: [u8; 6] = [0x84, 0xAD, 0xA4, 0xAC, 0xB8, 0x7F];

/// A far text's commands, up to any `text_asm`, which the caller follows with what it chooses.
pub fn far(label: &str) -> Vec<TextCommand> {
    // The one far text whose label does not say `Text`, which the label table leaves out.
    let found = match label {
        "_StartedSleepingEffect" => decode(poke_core::symbols::pokered_symbols::_StartedSleepingEffect),
        _ => far_text(label),
    };
    found.unwrap_or_else(|error| panic!("{label}: {error}"))
        .into_iter()
        .take_while(|command| !matches!(command, TextCommand::Asm(_)))
        .collect()
}

/// A text at a local label, far halves followed, up to any `text_asm`.
pub fn local(at: poke_core::symbols::DmgPointer) -> Vec<TextCommand> {
    decode(at).unwrap_or_else(|error| panic!("{at}: {error}"))
        .into_iter()
        .take_while(|command| !matches!(command, TextCommand::Asm(_)))
        .collect()
}

/// Several far texts, one after another, as their `text_asm`s chain them.
pub fn chain(labels: &[&str]) -> Vec<TextCommand> {
    labels.iter().flat_map(|label| far(label)).collect()
}

/// `<USER>` for `whose_turn`'s mon and `<TARGET>` for the other's, spliced into every run.
pub fn spliced(commands: Vec<TextCommand>, whose_turn: Side, player_nick: &[u8], enemy_nick: &[u8]) -> Vec<TextCommand> {
    let name = |side: Side| -> Vec<u8> {
        match side {
            Side::Player => player_nick.to_vec(),
            Side::Enemy => ENEMY.iter().chain(enemy_nick).copied().collect(),
        }
    };
    commands.into_iter().map(|command| match command {
        TextCommand::Text(bytes) => TextCommand::Text(bytes.into_iter().flat_map(|byte| match byte {
            USER => name(whose_turn),
            TARGET => name(whose_turn.other()),
            byte => vec![byte],
        }).collect()),
        // `PrintNumber` asked for one digit falls through every place to the millions.
        TextCommand::Number { source, bytes, digits: 1 } => TextCommand::Number { source, bytes, digits: 7 },
        other => other,
    }).collect()
}

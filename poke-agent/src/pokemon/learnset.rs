//! What the agent says when a machine is refused; the rules themselves are in `poke_core`.

pub use poke_core::learnset::*;
use crate::pokemon::item::ItemId;
use crate::pokemon::GameState;

/// What to say when a teach is aimed at a Pokémon the game will refuse.
pub fn teach_refusal(state: &GameState, item: ItemId, slot: u8) -> String {
    let name = |mon: &crate::pokemon::pokemon::Pokemon| {
        let nickname = mon.nickname.to_default_string();
        match nickname.eq_ignore_ascii_case(&mon.species.to_string()) {
            true => nickname,
            false => format!("{nickname} the {}", mon.species),
        }
    };
    let subject = match state.pokemon.get(slot as usize) {
        Some(mon) => format!("{} in slot {slot}", name(mon)),
        None => format!("Slot {slot}"),
    };
    let taught = match machine_move(item) {
        Some(mv) => format!("{mv} ({item})"),
        None => item.to_string(),
    };
    let takers: Vec<String> = state.pokemon.iter().enumerate()
        .filter(|(_, mon)| can_learn(mon.species, item))
        .map(|(i, mon)| format!("slot {i} {}", name(mon)))
        .collect();
    match takers.as_slice() {
        [] => format!(
            "{subject} cannot learn {taught}, and nor can anything else in the party. Every machine \
             works on a fixed list of Pokémon and the game refuses the rest, so teaching this one \
             needs a party member that is on that list; nothing you own is. Catching or swapping in \
             a Pokémon that can learn it is the only way past."),
        _ => format!("{subject} cannot learn {taught}. In the party, {} can.", takers.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_with_no_taker_reads_as_one_sentence() {
        let refusal = teach_refusal(&GameState::default(), ItemId::Hm03Surf, 0);
        assert!(refusal.contains("nor can anything else in the party"), "the no-taker arm: {refusal}");
        assert!(!refusal.contains('—'), "no em dashes in what the agent writes: {refusal}");
        assert!(!refusal.contains("  "), "a `\\` was eaten out of a continued literal: {refusal}");
    }
}

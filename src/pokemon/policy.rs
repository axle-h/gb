use std::io::{self, BufRead, Write};
use rand::seq::IteratorRandom;
use crate::pokemon::GameState;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::{BattleAction, BattleState};
use crate::pokemon::encoding::MetaTile;
use crate::pokemon::status::PokemonStatus;

pub trait Policy {
    fn pick_action(&mut self, state: &GameState) -> Option<OverworldAction>;
    fn pick_battle_action(&mut self, state: &BattleState) -> BattleAction;
}

// ── Random ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct RandomPolicy;

impl Policy for RandomPolicy {
    fn pick_action(&mut self, state: &GameState) -> Option<OverworldAction> {
        state.map.actions().into_iter().choose(&mut rand::rng())
    }

    fn pick_battle_action(&mut self, state: &BattleState) -> BattleAction {
        let available: Vec<u8> = state.player.moves.iter().enumerate()
            .filter_map(|(i, m)| m.filter(|m| m.current_pp > 0).map(|_| i as u8))
            .collect();
        if let Some(&slot) = available.iter().choose(&mut rand::rng()) {
            BattleAction::Fight(slot)
        } else {
            BattleAction::Run
        }
    }
}

// ── Console (human-driven) ────────────────────────────────────────────────────

/// Presents available actions as a numbered menu and blocks until the human types a number.
#[derive(Default)]
pub struct ConsolePolicy;

impl Policy for ConsolePolicy {
    fn pick_action(&mut self, state: &GameState) -> Option<OverworldAction> {
        let actions = state.map.actions();
        if actions.is_empty() { return None; }

        println!("\nAvailable actions:");
        for (i, action) in actions.iter().enumerate() {
            let label = match &action.tile {
                MetaTile::Warp(map)       => format!("Warp → {map}"),
                MetaTile::Connection(map) => format!("Go to {map}"),
                MetaTile::Sprite(name)    => format!("Talk to {name}"),
                other                     => format!("{other}"),
            };
            println!("  {}. {}", i + 1, label);
        }

        let n = read_choice(actions.len());
        actions.into_iter().nth(n - 1)
    }

    fn pick_battle_action(&mut self, state: &BattleState) -> BattleAction {
        println!("\n═══ BATTLE ═══");
        println!("Enemy:  {:?} Lv.{}  HP {}/{}  {}",
            state.enemy.species, state.enemy.level,
            state.enemy.current_hp, state.enemy.max_hp,
            fmt_status(state.enemy.status));
        println!("Player: {:?} Lv.{}  HP {}/{}  {}",
            state.player.species, state.player.level,
            state.player.current_hp, state.player.max_hp,
            fmt_status(state.player.status));

        let mut options: Vec<(String, BattleAction)> = vec![];

        // FIGHT — one entry per move with PP remaining
        for (i, m) in state.player.moves.iter().enumerate() {
            if let Some(m) = m {
                let label = format!("FIGHT  {:?}  (PP {})", m.name, m.current_pp);
                options.push((label, BattleAction::Fight(i as u8)));
            }
        }

        // ITEM — one entry per bag item
        for (i, item) in state.bag.iter().enumerate() {
            options.push((
                format!("ITEM   {} ×{}", item.id, item.quantity),
                BattleAction::UseItem(i),
            ));
        }

        // PKMN — one entry per non-fainted party Pokemon that isn't currently active
        for (slot, _) in state.party.pokemon().iter().enumerate() {
            if slot == state.active_party_slot as usize { continue; }
            let p = &state.party.pokemon()[slot];
            if p.stats.hp == 0 { continue; } // fainted
            options.push((
                format!("PKMN   {:?} Lv.{}  HP {}/{}", p.species, p.level,
                        p.current_hp, p.stats.hp),
                BattleAction::SwitchPokemon(slot),
            ));
        }

        // RUN — always available
        options.push(("RUN".to_string(), BattleAction::Run));

        println!("\nBattle actions:");
        for (i, (label, _)) in options.iter().enumerate() {
            println!("  {}. {}", i + 1, label);
        }

        let n = read_choice(options.len());
        options.remove(n - 1).1
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn fmt_status(s: PokemonStatus) -> String {
    match s {
        PokemonStatus::None                => String::new(),
        PokemonStatus::Paralyzed           => "PAR".to_string(),
        PokemonStatus::Frozen              => "FRZ".to_string(),
        PokemonStatus::Burned              => "BRN".to_string(),
        PokemonStatus::Poisoned            => "PSN".to_string(),
        PokemonStatus::Asleep { counter }  => format!("SLP({})", counter),
    }
}

fn read_choice(max: usize) -> usize {
    loop {
        print!("Pick (1-{max}): ");
        io::stdout().flush().ok();
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).is_err() {
            return 1;
        }
        if let Ok(n) = line.trim().parse::<usize>() {
            if n >= 1 && n <= max { return n; }
        }
        println!("Invalid — enter a number between 1 and {max}.");
    }
}

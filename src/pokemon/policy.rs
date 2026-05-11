use std::io::{self, BufRead, Write};
use rand::seq::IteratorRandom;
use crate::pokemon::GameState;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::encoding::MetaTile;

pub trait Policy {
    fn pick_action(&mut self, state: &GameState) -> Option<OverworldAction>;
}

// ── Random ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct RandomPolicy;

impl Policy for RandomPolicy {
    fn pick_action(&mut self, state: &GameState) -> Option<OverworldAction> {
        state.map.actions().into_iter().choose(&mut rand::rng())
    }
}

// ── Console (human-driven) ────────────────────────────────────────────────────

/// Prints the available actions and blocks until the player types a number.
#[derive(Default)]
pub struct ConsolePolicy;

impl Policy for ConsolePolicy {
    fn pick_action(&mut self, state: &GameState) -> Option<OverworldAction> {
        let actions = state.map.actions();
        if actions.is_empty() {
            return None;
        }

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

        loop {
            print!("Pick (1-{}): ", actions.len());
            io::stdout().flush().ok();

            let mut line = String::new();
            if io::stdin().lock().read_line(&mut line).is_err() {
                return None;
            }
            if let Ok(n) = line.trim().parse::<usize>() {
                if n >= 1 && n <= actions.len() {
                    return actions.into_iter().nth(n - 1);
                }
            }
            println!("Invalid — enter a number between 1 and {}.", actions.len());
        }
    }
}

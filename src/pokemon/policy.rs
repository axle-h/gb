use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use rand::seq::IteratorRandom;
use crate::pokemon::GameState;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::encoding::MetaTile;

/// Non-blocking policy interface.
///
/// Both methods return `Option<_>`. `None` means "not ready yet — ask again next frame".
/// This keeps the game loop running while the policy waits for input.
pub trait Policy {
    fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction>;
    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction>;
}

// ── Random (always-ready) ─────────────────────────────────────────────────────

#[derive(Default)]
pub struct RandomPolicy;

impl Policy for RandomPolicy {
    fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
        state.map.actions().into_iter().choose(&mut rand::rng())
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        let available: Vec<u8> = state.battle.as_ref()?.player.moves.iter().enumerate()
            .filter_map(|(i, m)| m.filter(|m| m.current_pp > 0).map(|_| i as u8))
            .collect();
        Some(if let Some(&slot) = available.iter().choose(&mut rand::rng()) {
            BattleAction::Fight(slot)
        } else {
            BattleAction::Run
        })
    }
}

// ── Console (human-driven, non-blocking) ─────────────────────────────────────

/// Displays a numbered menu, then reads the user's choice from stdin on a
/// background thread so the game loop is never blocked.
pub struct ConsolePolicy {
    overworld_rx:   Option<Receiver<usize>>,
    battle_rx:      Option<Receiver<usize>>,
    ow_menu_shown:  bool,
    btl_menu_shown: bool,
    /// Tiles shown when the last overworld menu was displayed; used to match
    /// the user's selection by destination rather than by list index, since
    /// the action list can reorder between display and selection.
    ow_shown_tiles: Vec<MetaTile>,
}

impl Default for ConsolePolicy {
    fn default() -> Self {
        Self {
            overworld_rx:   None,
            battle_rx:      None,
            ow_menu_shown:  false,
            btl_menu_shown: false,
            ow_shown_tiles: vec![],
        }
    }
}

impl Policy for ConsolePolicy {
    fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
        let actions = state.map.actions();
        if actions.is_empty() { return None; }

        if !self.ow_menu_shown || self.overworld_rx.is_none() {
            println!("\nAvailable actions:");
            for (i, a) in actions.iter().enumerate() {
                let label = match &a.tile {
                    MetaTile::Warp(m)       => format!("Warp → {m}"),
                    MetaTile::Connection(m) => format!("Go to {m}"),
                    MetaTile::Sprite(n)     => format!("Talk to {n}"),
                    other                   => format!("{other}"),
                };
                println!("  {}. {}", i + 1, label);
            }
            let max = actions.len();
            // Cache the destinations so we can match by tile, not index.
            self.ow_shown_tiles = actions.iter().map(|a| a.tile.clone()).collect();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                loop {
                    print!("Pick (1-{max}): ");
                    io::stdout().flush().ok();
                    let mut line = String::new();
                    if io::stdin().read_line(&mut line).is_err() { break; }
                    if let Ok(n) = line.trim().parse::<usize>() {
                        if n >= 1 && n <= max { tx.send(n).ok(); break; }
                    }
                    println!("Invalid.");
                }
            });
            self.overworld_rx = Some(rx);
            self.ow_menu_shown = true;
        }

        if let Ok(n) = self.overworld_rx.as_ref().unwrap().try_recv() {
            let chosen_tile = self.ow_shown_tiles.get(n - 1).cloned();
            self.overworld_rx  = None;
            self.ow_menu_shown = false;
            self.ow_shown_tiles.clear();
            if let Some(tile) = chosen_tile {
                return actions.into_iter().find(|a| a.tile == tile);
            }
            return None;
        }
        None
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        if !self.btl_menu_shown || self.battle_rx.is_none() {
            let battle_state = state.battle.as_ref()?;

            let opts = battle_options(state)?;
            println!("\n═══ BATTLE ═══");
            println!("Enemy:  {:?} Lv.{}  HP {}/{}  {}",
                battle_state.enemy.species, battle_state.enemy.level,
                battle_state.enemy.current_hp, battle_state.enemy.max_hp,
                battle_state.enemy.status);
            println!("Player: {:?} Lv.{}  HP {}/{}  {}",
                battle_state.player.species, battle_state.player.level,
                battle_state.player.current_hp, battle_state.player.max_hp,
                battle_state.player.status);
            println!("\nBattle actions:");
            for (i, (label, _)) in opts.iter().enumerate() {
                println!("  {}. {}", i + 1, label);
            }
            let max = opts.len();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                loop {
                    print!("Pick (1-{max}): ");
                    io::stdout().flush().ok();
                    let mut line = String::new();
                    if io::stdin().read_line(&mut line).is_err() { break; }
                    if let Ok(n) = line.trim().parse::<usize>() {
                        if n >= 1 && n <= max { tx.send(n).ok(); break; }
                    }
                    println!("Invalid.");
                }
            });
            self.battle_rx    = Some(rx);
            self.btl_menu_shown = true;
        }

        if let Ok(n) = self.battle_rx.as_ref().unwrap().try_recv() {
            self.battle_rx     = None;
            self.btl_menu_shown = false;
            let mut opts = battle_options(state)?;
            return Some(opts.remove(n - 1).1);
        }
        None
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn battle_options(state: &GameState) -> Option<Vec<(String, BattleAction)>> {
    let battle_state = state.battle.as_ref()?;
    let mut opts: Vec<(String, BattleAction)> = vec![];
    for (i, m) in battle_state.player.moves.iter().enumerate() {
        if let Some(m) = m {
            opts.push((format!("FIGHT  {:?}  PP {}", m.name, m.current_pp),
                       BattleAction::Fight(i as u8)));
        }
    }
    for (i, item) in state.bag.iter().enumerate() {
        opts.push((format!("ITEM   {} ×{}", item.id, item.quantity), BattleAction::UseItem(i as u8)));
    }

    let pokemon = state.pokemon.pokemon();
    for (slot, _) in pokemon.iter().enumerate() {
        if slot == battle_state.active_party_slot as usize { continue; }
        let p = &pokemon[slot];
        if p.stats.hp == 0 { continue; }
        opts.push((format!("PKMN   {:?} Lv.{} HP {}/{}", p.species, p.level,
                           p.current_hp, p.stats.hp),
                   BattleAction::SwitchPokemon(slot as u8)));
    }
    opts.push(("RUN".to_string(), BattleAction::Run));
    Some(opts)
}

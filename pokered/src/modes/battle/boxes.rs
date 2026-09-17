//! Where a caught mon goes when the party is full, and the Safari Zone's own balls.

use crate::mode::Ctx;
use crate::party::{BoxMon, Named, NUM_STATS};
use crate::systems::battle::BattleMon;
use crate::systems::experience::calc_experience;

/// `MONS_PER_BOX`.
pub const MONS_PER_BOX: usize = 20;

fn current_box<'a>(ctx: &'a mut Ctx) -> &'a mut Vec<Named<BoxMon>> {
    let current = ctx.world.current_box as usize;
    if ctx.world.boxes.len() <= current {
        ctx.world.boxes.resize(current + 1, Vec::new());
    }
    &mut ctx.world.boxes[current]
}

/// `wBoxCount` at `MONS_PER_BOX`.
pub fn box_is_full(ctx: &mut Ctx) -> bool {
    current_box(ctx).len() >= MONS_PER_BOX
}

/// `SendNewMonToBox`: the enemy mon as the battle holds it, at its level's experience with no stat
/// experience, first in the current box.
pub fn send_new_mon_to_box(ctx: &mut Ctx, enemy: &BattleMon, ot: Vec<u8>, nick: Vec<u8>) {
    let growth = poke_core::base_stats::BaseStats::of(enemy.species).growth_rate;
    let mon = BoxMon {
        species: enemy.species,
        hp: enemy.hp,
        box_level: enemy.level,
        status: enemy.status,
        types: enemy.types,
        catch_rate: enemy.catch_rate,
        moves: enemy.moves,
        ot_id: ctx.world.player_id,
        exp: calc_experience(growth, enemy.level),
        stat_exp: [0; NUM_STATS],
        dvs: enemy.dvs,
        pp: enemy.pp,
    };
    current_box(ctx).insert(0, Named { mon, ot, nick });
}

/// `wNumSafariBalls`.
pub fn safari_balls(ctx: &Ctx) -> u8 {
    ctx.world.safari_balls
}

/// `ItemUseBall.safariZone`'s decrement.
pub fn use_safari_ball(ctx: &mut Ctx) {
    ctx.world.safari_balls = ctx.world.safari_balls.wrapping_sub(1);
}

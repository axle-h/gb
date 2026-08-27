// The deterministic policy's battle strategy, as a battle script.
//
// This is `DeterministicPolicy::pick_battle_action` (src/pokemon/policy.rs) with the arms that need
// the route plan taken out: Safari hunts, fishing, `UseItemsInBattle`, grinding hand-offs, catch
// targets and the centre-less-dungeon flee list are all decisions about *where the run is*, and a
// battle script cannot see that. What is left is the part that only looks at the battle, in the
// order the policy checks it.
//
// It is kept beside the sandbox as a worked example that is known to play the game: it is the
// strategy `full_playthrough` finishes with.

fn damaging_pp(moves) {
    // ⚠️ `power > 0` is a near-miss for the policy's `is_damaging_move`, which also counts the five
    // fixed-damage moves (Seismic Toss, Night Shade, Sonicboom, Dragon Rage, Psywave). None of them
    // is a starter's move and all five would only make this more willing to fight on.
    let now = 0;
    let max = 0;
    for mv in moves {
        if mv.power > 0 {
            now += mv.pp;
            max += mv.max_pp;
        }
    }
    [now, max]
}

fn healthiest(party, active) {
    let best = ();
    for mon in party {
        if mon.slot != active && !mon.fainted {
            if best == () || mon.hp > best.hp { best = mon; }
        }
    }
    best
}

fn best_damage(moves) {
    let best = 0;
    for mv in moves {
        if mv.usable && mv.damage > best { best = mv.damage; }
    }
    best
}

let me = battle.me;
let foe = battle.foe;

// 1. Damaging PP under a fifth of its maximum. The policy flees and routes to a Centre; a script
//    can only do the first half, and the report says why so the plan can do the second.
let pp = damaging_pp(me.moves);
if pp[1] > 0 && pp[0] * 5 < pp[1] && battle.can_run {
    print("damaging PP down to " + pp[0] + "/" + pp[1] + ", disengaging: this party needs a Centre");
    battle.run();
}

// 2. The forced-switch screen: send the one with the most HP left.
if me.fainted {
    let next = healthiest(battle.party, me.slot);
    if next != () { battle.switch_to(next); }
}

// 3. Under a quarter HP: the biggest heal in the bag, never the smallest. A Super Potion that only
//    cancels one hit stalemates forever against a fast super-effective attacker.
if me.hp_frac < 0.25 {
    for want in ["FullRestore", "MaxPotion", "HyperPotion", "SuperPotion", "Potion"] {
        for item in battle.bag {
            if item.name == want {
                print(me.name + " at " + me.hp + "/" + me.max_hp + ", using " + want);
                battle.use_item(want);
            }
        }
    }
}

// 4. Under 15% with nothing to heal with: a team-mate, but only a genuine one. Half its own HP or
//    better, no lower level, and actually better off than we are, or this hands a Pokemon over.
if me.hp_frac < 0.15 {
    let next = healthiest(battle.party, me.slot);
    if next != () && next.hp_frac > 0.5 && next.level >= me.level && next.hp > me.hp {
        print(me.name + " is nearly out, sending " + next.name);
        battle.switch_to(next);
    }
    // 5. Nobody to send either. Get out and heal.
    if battle.can_run {
        print(me.name + " at " + me.hp + "/" + me.max_hp + ", no heal and no switch: fleeing");
        battle.run();
    }
}

// 6. A trainer battle we are barely denting: pool the team's coverage instead of chipping away.
//    Guarded at 1.5x so it cannot thrash between two mons.
if battle.kind == "trainer" {
    let mine = best_damage(me.moves);
    if mine * 3 < foe.max_hp {
        let pick = ();
        let pick_damage = 0;
        for mon in battle.party {
            if mon.slot != me.slot && !mon.fainted && mon.hp_frac > 0.5 && mon.level + 8 >= me.level {
                let d = best_damage(mon.moves);
                if d > pick_damage { pick_damage = d; pick = mon; }
            }
        }
        if pick != () && pick_damage * 2 >= mine * 3 {
            print(me.name + " only does " + mine + " to a " + foe.max_hp + " HP " + foe.species
                  + ", " + pick.name + " does " + pick_damage);
            battle.switch_to(pick);
        }
    }
}

// 7. Otherwise hit it with the best thing we have.
if battle.best_move != () {
    battle.fight(battle.best_move);
}

// 8. Nothing we know can hurt it. Send someone whose moves can.
for mon in battle.party {
    if mon.slot != me.slot && !mon.fainted && best_damage(mon.moves) > 0 {
        print("nothing " + me.name + " knows touches " + foe.species + ", sending " + mon.name);
        battle.switch_to(mon);
    }
}

// 9. Out of ideas: leave, or ask.
if battle.can_run {
    print("no move does anything and nobody on the bench is better: fleeing");
    battle.run();
}
battle.ask();

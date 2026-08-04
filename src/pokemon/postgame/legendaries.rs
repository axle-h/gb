//! Workstream **D — Legendaries: Zapdos, Moltres, Mewtwo**. See `docs/postgame-coverage-plan.md` §6-D.
//!
//! Sub-steps: D1a Thunder Wave · D1b Moltres (Victory Road 2F) · D2 reach the Power Plant · D3 Zapdos ·
//! D5 Cerulean Cave · D6 Mewtwo, then `postgame-legendaries.bin`.
//!
//! # ⚠️ What this workstream covers, and what it deliberately does not
//!
//! It covers the **routes**: Victory Road 2F's third sealed region, the Power Plant across Route 10's
//! lake, and Cerulean Cave behind Route 24's river seam. Those are navigation problems and the tests
//! prove them.
//!
//! It does **not** cover *winning the fight*. Each of the three catches is thrown a **debug-seeded
//! Master Ball** (plan §3, the debug tier), and that is a scope decision rather than a gap: a
//! catch-rate-3 encounter that must not be KO'd, fled or lost comes down to battle tactics, and in
//! deployment those are the LLM's to make, not the deterministic policy's. Do not reopen it.
//!
//! The formula is kept below because it is the reference for *any* hard catch, not because a better
//! one is being chased. From `engine/items/item_effects.asm`, `ItemUseBall`, on the three remaining
//! legendaries' **catch rate 3** — the joint lowest in the game:
//!
//! ```text
//! Rand1 ∈ [0,255] Poké Ball / [0,200] Great Ball / [0,150] Ultra Ball
//! Status subtracts 12 (burn/paralysis/poison) or 25 (freeze/sleep) from Rand1; if that underflows,
//!   the Pokémon is caught outright.
//! Otherwise Rand1 - Status > catch rate ⇒ the ball fails, whatever the target's HP is.
//! ```
//!
//! So on a catch-rate-3 target the *only* lever that matters much is the status subtraction. Bare, an
//! Ultra Ball is `4/151` ≈ **2.6 %**; paralysed it is `12/151 + …` ≈ **8.6 %**, and a Poké Ball goes
//! from 0.5 % to 5.1 %. Weakening the target — the thing the generic catch policy does — moves only the
//! fraction of a percent that rides on the HP check, and risks killing a once-per-cartridge encounter.
//! Hence [`pre_catch_action`], which exists mainly to keep the generic policy from doing that: it
//! paralyses if it can, and otherwise throws and never attacks.
//!
//! Thunder Wave is free (TM45 lies on the ground on Route 24) and **Slowpoke is the only party member
//! that can learn it** — Venusaur, Articuno and Vaporeon are all incompatible
//! (`data/pokemon/base_stats/*.asm`, tm/hm learnset).

use crate::geometry::Point8;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::{BattleAction, BattleType};
use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::policy::PolicyStep;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::GameState;

/// The species this module treats as "throw, never fight": every legendary bird plus Mewtwo, all of
/// which have `db 3 ; catch rate` in their base stats. Articuno is listed for completeness — the main
/// playthrough Master-Balls it, so it never reaches [`pre_catch_action`] in practice.
pub const CATCH_RATE_3: [PokemonSpecies; 4] = [
    PokemonSpecies::Articuno,
    PokemonSpecies::Zapdos,
    PokemonSpecies::Moltres,
    PokemonSpecies::Mewtwo,
];

/// The party slot Thunder Wave is taught to: **Slowpoke**, the only compatible member (see the module
/// doc). It is slot 3 in every fixture from `postgame-fly-bike.bin` onward.
pub const PARALYSER_SLOT: u8 = 3;

/// Where Moltres sits once caught — it is appended after the four the fixture starts with. Banked
/// before Mewtwo, to keep a party slot free; see [`PolicyStep::mewtwo_steps`].
const MOLTRES_SLOT: u8 = 4;

/// How much HP each healing item restores. `u16::MAX` stands for "all of it".
fn heal_amount(item: ItemId) -> u16 {
    match item {
        ItemId::Potion => 20,
        ItemId::SuperPotion => 50,
        ItemId::HyperPotion => 200,
        ItemId::MaxPotion | ItemId::FullRestore => u16::MAX,
        _ => 0,
    }
}

/// The **cheapest sufficient** heal on offer, or the biggest one if nothing covers the damage.
///
/// Deliberately not the generic policy's "always use the biggest" rule. These fights are long — dozens
/// of ball throws against something that attacks every turn — and the mon that most needs topping up is
/// the lv30 Slowpoke, whose entire HP bar a Hyper Potion covers three times over. Spending a Full
/// Restore on it wastes 5/6 of the item and the stack has to last three legendaries.
fn heal_action(actions: &[BattleAction], missing: u16) -> Option<&BattleAction> {
    let offered = || actions.iter().filter(|a| matches!(a,
        BattleAction::UseItem { item, .. } if heal_amount(item.id) > 0));
    let amount = |a: &BattleAction| match a {
        BattleAction::UseItem { item, .. } => heal_amount(item.id),
        _ => 0,
    };
    offered().filter(|a| amount(a) >= missing).min_by_key(|a| amount(a))
        .or_else(|| offered().max_by_key(|a| amount(a)))
}

/// What to do on a battle turn against a catch-rate-3 target, or `None` to let the generic catch
/// policy in `pick_battle_action` have it.
///
/// Jobs, in priority order:
///
/// 1. **Paralyse, before anything else.** Not "when convenient" — *first*, ahead even of healing the
///    mon doing it. This is counter-intuitive and it is the difference between the two Moltres runs.
///    Moltres opens with **Fire Spin**, and a Gen 1 partial-trapping move skips the trapped Pokémon's
///    turn outright: on a trapped turn no choice of ours executes at all. So the turns that *do*
///    execute are precious, and Thunder Wave — 100 % accuracy, permanent, and it ends the problem —
///    should be what spends them. The first version healed at 60 % and spent **ten** free turns on
///    Hyper Potions while a lv30 Slowpoke was slowly pecked to death without ever landing the move.
///    Choosing Thunder Wave on a trapped turn costs nothing, so there is no reason to hedge.
/// 2. **Get the paralyser back out of the firing line** once the status is on. Its job is done and it
///    is the frailest thing in the party; every turn it stays in costs another potion.
/// 3. **Keep the mon in front standing.** The generic heal rule lives *below* the catch branch in
///    `pick_battle_action`, so a hook that returns an action every turn silently disables it — which is
///    how the first attempt ended in a blackout with the whole party down.
/// 4. **Throw, and only throw.** Never fall through to the generic "weaken below 50 % first" branch.
///    Against these four that branch is all downside: it buys a fraction of a percent (the HP term only
///    decides the sliver of outcomes where `Rand1 - 12` is still ≤ 3) and it can KO a target that
///    exists exactly once per cartridge.
///
/// Returns `None` when there is nothing to do at all, so a party that never picked up TM45 degrades to
/// the ordinary catch behaviour instead of standing still.
pub fn pre_catch_action(
    state: &GameState,
    target: PokemonSpecies,
    actions: &[BattleAction],
    throw_ball: Option<&BattleAction>,
) -> Option<BattleAction> {
    if !CATCH_RATE_3.contains(&target) {
        return None;
    }
    let battle = state.battle.as_ref()?;
    if battle.battle_type != BattleType::Wild || battle.enemy.species != target {
        return None;
    }
    // A Master Ball is an unconditional capture — `ItemUseBall` jumps straight to `.captured` before
    // it rolls anything — so skip the whole paralyse-and-nurse routine. It could only lose turns, and
    // against a trapping move losing turns is how the encounter is lost. This is the path the
    // debug-seeded tests take to prove the routing and the encounter machinery independently of the
    // fight; on a legitimate run the bag has no Master Ball and nothing changes.
    if matches!(throw_ball, Some(BattleAction::UseItem { item, .. }) if item.id == ItemId::MasterBall) {
        println!("[legendaries] Master Ball in the bag — throwing it at {target} immediately");
        return throw_ball.cloned();
    }

    let statused = battle.enemy.status != PokemonStatus::None;

    let switch_to = |slot: u8| actions.iter()
        .find(|a| matches!(a, BattleAction::SwitchPokemon { slot: s, .. } if *s == slot))
        .cloned();

    let paralyser = state.pokemon.iter().enumerate()
        .find(|(_, p)| p.current_hp > 0
            && p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::ThunderWave && m.pp > 0))
        .map(|(i, _)| i as u8);

    if !statused {
        // ⚠️ **Never run.** It is tempting: while the enemy is mid-Fire-Spin no move of ours resolves
        // at all (see `BattleState::enemy_trapping`), the battle menu still opens, so RUN is right
        // there, and a fresh battle would be a fresh untrapped turn 1. But these are `trainer`-flagged
        // objects (`scripts/VictoryRoad2F.asm:100`), and `EndTrainerBattle` sets the flag and calls
        // `HideObject` on **any** exit except a blackout — `home/trainers.asm:185-213` only skips that
        // path on `wIsInBattle == $ff`. Running therefore deletes the legendary from the save exactly
        // as thoroughly as killing it. (Measured: after one flee, `probe_stall_artifact` shows the
        // Moltres sprite `hidden=true`, and it stays hidden across map reloads.) Blacking out is the
        // only recoverable way to lose one of these fights.
        //
        // 1. Paralyse. `available_battle_moves` has already dropped moves at 0 PP and the one Disable
        // has locked out, so anything it offers is selectable. Note that on a trapped turn *no* move
        // resolves, so this can be re-picked for many turns without ever firing — see the module doc.
        let held = battle.player.available_battle_moves().into_iter()
            .find(|a| matches!(a, BattleAction::Fight { battle_move, .. }
                if battle_move.name == PokemonMoveName::ThunderWave));
        if let Some(paralyse) = held {
            println!("[legendaries] {target} has no status — Thunder Wave");
            return Some(paralyse);
        }
        if let Some(slot) = paralyser.filter(|s| *s != battle.active_party_slot) {
            if let Some(switch) = switch_to(slot) {
                println!("[legendaries] switching slot {slot} in to paralyse {target}");
                return Some(switch);
            }
        }
        // No paralyser available (never taught, fainted, or out of PP). Fall through: an unparalysed
        // throw is a poor bet but a better one than doing nothing.
    } else {
        // 2. Put the biggest HP bar in front for the throwing phase. Fires at most once: after the
        // switch the active mon *is* the healthiest, so the guard stops matching.
        let healthiest = state.pokemon.iter().enumerate()
            .max_by_key(|(_, p)| p.current_hp).map(|(i, p)| (i as u8, p.current_hp));
        if let Some((slot, hp)) = healthiest {
            if slot != battle.active_party_slot && hp > battle.player.current_hp * 2 {
                if let Some(switch) = switch_to(slot) {
                    println!("[legendaries] {target} is {:?} — swapping slot {slot} in to soak the throws",
                        battle.enemy.status);
                    return Some(switch);
                }
            }
        }
    }

    // 3. Heal. `missing` also stops an item being burned on a scratch.
    if battle.player.remaining_hp() < 0.35 {
        let missing = battle.player.stats.hp.saturating_sub(battle.player.current_hp);
        if let Some(heal) = heal_action(actions, missing) {
            println!("[legendaries] {:?} at {:.0}% — {heal}", battle.player.species,
                battle.player.remaining_hp() * 100.0);
            return Some(heal.clone());
        }
    }

    throw_ball.cloned()
}

impl PolicyStep {
    /// **D1a** — the toolkit all three catches share: a stack of balls, plus **TM45 Thunder Wave**
    /// picked up on Route 24 and taught to Slowpoke.
    ///
    /// The TM is a plain ball pickup at Route 24 (10,5) (`data/maps/objects/Route24.asm:26`), i.e. one
    /// `CollectItem`. Route 24 is Nugget Bridge, so the walk north to it crosses five trainers' lines
    /// of sight; heal in Cerulean first and let them come.
    ///
    /// Slowpoke's weakest move is Confusion (50 power), so `DeterministicPolicy::pick_move_to_forget`
    /// drops that one — **Strength and Dig survive**, which matters because Slowpoke is also the
    /// party's only HM slave.
    ///
    /// Poké Balls by the dozens and Ultra Balls by the handful, *on purpose*. Per yen the Poké Ball is
    /// the best ball in the game against a catch-rate-3 target — 5.1 % for ¥200 against the Ultra
    /// Ball's 8.6 % for ¥1,200 — but the number of throws is capped by the target's PP (it Struggles
    /// itself to death eventually), so the plan is "fill the turn budget with Poké Balls, and spend
    /// what is left making the *early* turns count". `Bag::best_pokeball` picks the lowest item id, so
    /// the Ultra Balls are the ones thrown first.
    ///
    /// The Hyper Potions are the other half of the budget and are not optional: paralysing takes as
    /// many turns as the target's Fire Spin trap says it does, and the paralyser is a lv30 Slowpoke.
    /// Hyper Potion (¥1,200, +200 HP) rather than Full Restore (¥3,000, +everything) because Slowpoke's
    /// whole bar is 98 — see [`heal_action`].
    ///
    /// ¥12,000 + ¥12,000 + ¥8,000 of the ¥41,209 the fixture carries. Cinnabar Mart is the one counter
    /// that sells Ultra Balls *and* Hyper Potions (`data/items/marts.asm`, `CinnabarMartClerkText`);
    /// Cerulean sells the Poké Balls and is Route 24's doorstep. The TM is collected **first** so the
    /// bag is at its emptiest for the shopping — it comes back out again the moment it is taught.
    pub fn arm_for_legendaries_steps() -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeruleanCity },
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::Route24),
            Self::CollectItem(MapSprite::ROUTE24_TM_THUNDER_WAVE),
            Self::TeachMove { item: ItemId::Tm45ThunderWave, target_slot: PARALYSER_SLOT },
            Self::Fly { to: Map::CinnabarIsland },
            Self::BuyFromMart { item: BagItem::new(ItemId::UltraBall, 10), map: Map::CinnabarMart },
            Self::BuyFromMart { item: BagItem::new(ItemId::HyperPotion, 10), map: Map::CinnabarMart },
            Self::enter(Map::CinnabarIsland), // back outside, or the town map will not open
            Self::Fly { to: Map::CeruleanCity },
            Self::BuyFromMart { item: BagItem::new(ItemId::PokeBall, 40), map: Map::CeruleanMart },
            // Finish outdoors. Every catch leg opens with a `Fly`, and `FlyState::blocked_by` refuses
            // one from inside a shop — it pops the step with a reason and the leg then tries to walk
            // to Viridian from the Cerulean Mart doormat.
            Self::enter(Map::CeruleanCity),
        ]
    }

    /// **D1b** — Moltres, on **Victory Road 2F** at (11,5).
    ///
    /// §6-D calls this near-trivial because "the map is already traversed by `complete_game_steps` —
    /// the agent walks straight past it". It does not walk past it: Moltres sits in a **third**,
    /// separately-sealed region of 2F, and getting into that region is the whole cost of this
    /// sub-step. Measured with `probe_route_to_moltres`, the three regions are
    ///
    /// | Entered from | Contains | Reaches Moltres? |
    /// |---|---|---|
    /// | Route 23's (14,31) door → (29,7) | that door, the (27,7) stairs | no — sealed pocket |
    /// | VR1F's ladder → (0,8), after the (1,16) switch | five trainers, three items, the (23,7) stairs | no |
    /// | VR3F's (2,0) warp → (1,1) | Super Nerd 2, the Guard Spec, **Moltres** | yes |
    ///
    /// So Victory Road has to be entered at the **bottom** and climbed: the Indigo Plateau end of
    /// Route 23 can only reach the 2F door, and the 1F door four tiles from it at (4,31) is on the
    /// other terrace. From Viridian that is Route 22 → the gate → Route 23 (Surfed, up the pools) →
    /// VR1F → its (17,13) boulder switch → VR2F → the (1,16) switch → VR3F → back down at (1,1).
    ///
    /// Both `SolveBoulders` calls are needed even though nothing on the way to Moltres is behind the
    /// second one: the (23,7) stairs to VR3F are.
    pub fn moltres_steps() -> Vec<Self> {
        let mut steps = vec![
            Self::Fly { to: Map::ViridianCity },
            // Victory Road is nine trainers and no Pokémon Center. Heal at the last one before it.
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::Route22),
            Self::enter(Map::Route22Gate),
            Self::Interact(MapSprite::ROUTE22GATE_GUARD), // badge check, and it flips the dynamic warp
            Self::enter(Map::Route23),
            Self::goto(Map::VictoryRoad1F),
            // VR1F: a boulder onto the (17,13) switch opens the (1,1) ladder up to 2F.
            Self::UseStrength { slot: PARALYSER_SLOT },
            Self::SolveBoulders { switch: Point8 { x: 17, y: 13 } },
            Self::enter(Map::VictoryRoad2F),
            // VR2F west: the (1,16) switch opens the corridor east to the (23,7) stairs.
            Self::UseStrength { slot: PARALYSER_SLOT },
            Self::SolveBoulders { switch: Point8 { x: 1, y: 16 } },
            Self::enter(Map::VictoryRoad3F),
        ];
        // …then down through VR3F's (2,0) warp into 2F's north strip, and walk into Moltres.
        //
        // The paralyser stays on the **bench**; `pre_catch_action` switches it in once the battle has
        // started. Leading with it looks better — turn 1 is the one turn a trapping move cannot already
        // be active on — but Victory Road's floor is a wild-encounter map: a lv30 Slowpoke walking
        // point met an Onix, failed to run from it, and was dead before it ever saw Moltres.
        //
        // ⚠️ **The fight is out of scope and the test seeds a Master Ball for it** (plan §3). Moltres
        // opens with Fire Spin about half the time, and a slower paralyser cannot act through the trap,
        // cannot outrun it and must not flee — so winning it honestly is a battle-tactics problem, and
        // battle tactics are the LLM's job in deployment, not the deterministic policy's. What this
        // list is for, and what the test proves, is the **route**.
        steps.extend([
            Self::enter_at(Map::VictoryRoad2F, 1, 1),
            Self::CatchPokemon { species: PokemonSpecies::Moltres, on_map: Map::VictoryRoad2F,
                                 ball: None },
        ]);
        steps
    }

    /// **D2 + D3** — reach the **Power Plant** and catch **Zapdos** at (4,9).
    ///
    /// Getting out of Victory Road is the first problem: it is a cave, so Fly refuses to open the town
    /// map there. `Dig` is the way out — Slowpoke carries it precisely for this — and it lands on
    /// `wLastBlackoutMap`, which [`Self::moltres_steps`] set to Viridian when it healed.
    ///
    /// Then Cerulean → Route 9 → Route 10, and the Power Plant door is `warp_event 6, 39, POWER_PLANT`
    /// on the far side of Route 10's lake (`data/maps/objects/Route10.asm:17`) — the BFS Surfs to it
    /// on its own. Route 9 needs Cut, which is why Venusaur has to stay in slot 0.
    ///
    /// ⚠️ Cerulean is **split by one-way ledges** and Fly lands on the Pokémon Center terrace, which
    /// cannot reach Route 9 at all — the leg stalled at (20,19) until this was added. The bridge is the
    /// **trashed house**: in its front door and out the back at (27,9), exactly as
    /// `cerulean_to_lavender_steps` does it.
    ///
    /// Zapdos is the *easier* bird: at lv50 it knows only Thundershock and Drill Peck
    /// (`base_stats/zapdos.asm:13`, learnset starts at 51), neither of which is a trapping move, so
    /// unlike Moltres it cannot lock a slow paralyser out of the fight.
    pub fn zapdos_steps() -> Vec<Self> {
        vec![
            Self::Dig { slot: PARALYSER_SLOT },
            Self::Fly { to: Map::CeruleanCity },
            Self::enter(Map::CeruleanTrashedHouse),   // main terrace front door
            Self::enter_at(Map::CeruleanCity, 27, 9), // back door → the Route-9 terrace
            Self::enter(Map::Route9),
            Self::CutTree { map: Map::Route9 },        // the (5,8) tree boxing in the west pocket
            Self::enter(Map::Route10),
            Self::enter(Map::PowerPlant),
            Self::CatchPokemon { species: PokemonSpecies::Zapdos, on_map: Map::PowerPlant,
                                 ball: None },
        ]
    }

    /// **D5 + D6** — **Cerulean Cave** and **Mewtwo** at B1F (27,13).
    ///
    /// Out of the Power Plant on foot (its door leads straight back to Route 10, so no Dig needed),
    /// Fly to Cerulean, heal — then **north to Route 24 and back down its left river seam**, which is
    /// the only way onto the terrace holding `warp_event 4, 11, CERULEAN_CAVE_1F`
    /// (`data/maps/objects/CeruleanCity.asm:24`); see the comment in the body. Then 1F's (0,6) ladder
    /// down to B1F. The cave is post-Champion-only content, which this save has been for a while.
    ///
    /// With five birds and a Slowpoke the party is full by now, so Mewtwo lands in **box 1** rather than
    /// slot 6 — `ItemUseBall` only refuses when the party *and* the box are full. The Pokédex flag is
    /// set either way, which is what `CatchPokemon` waits on.
    pub fn mewtwo_steps() -> Vec<Self> {
        let mut steps = vec![
            Self::enter(Map::Route10),
            Self::Fly { to: Map::CeruleanCity },
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            // Bank Moltres while we are standing at the PC. Not housekeeping: with six in the party a
            // caught Pokémon is sent to the box instead, and the nickname screen that follows *that*
            // path wedges the agent — it sat on `name:Mewtwo` and burned every remaining cycle, at 150
            // and at 240 emulated minutes alike. A free party slot puts the catch back on the ordinary
            // path, the one Moltres and Zapdos already proved.
            Self::deposit_pokemon(MOLTRES_SLOT, Map::CeruleanPokecenter),
            Self::enter(Map::CeruleanCity),
        ];
        // ⚠️ **The cave door is on the far side of Cerulean.** The city is split: the Fly landing, the
        // gym and the marts are all east of a lake and a solid wall at x=8, and the cave's strip — west
        // of that lake, ringed by ledges — touches none of it. There is no land route in at all. The
        // way through is by **water**: go north to Route 24, take the **left** of its two river seams,
        // and surf back down into Cerulean on the cave's side of everything.
        //
        // Asking for that seam takes `enter_at` rather than `enter`, because a `ConnectionWater` is
        // only ever offered by `actions()` when it is the *nearest* crossing to that map, and Route
        // 24's footbridge is two steps from where we arrive. The coordinate is Cerulean's own ROM x for
        // the seam (its land bridge is at x=20-21); no land crossing lands there, so the request falls
        // through to `water_connection_action`.
        steps.extend([
            Self::enter(Map::Route24),
            Self::enter_at(Map::CeruleanCity, 14, 0),
            Self::enter(Map::CeruleanCave1F),
            // 1F's B1F ladder at (0,6) is **not** reachable from the entrance: the strip in front of it
            // is raw tile 32 and the room below is tile 5, and `(32, 5)` is one of the Cavern tileset's
            // `TilePairCollisions` — an elevation boundary the player cannot step across. The way round
            // is the floor above: up at 1F (3,11), across 2F, and back down at 2F (1,3), which lands on
            // 1F (1,3) — inside the walled-off western section, a few tiles from the ladder. It has to
            // be *that* pair of ladders: 2F is itself cut into pieces, and the one 1F (23,7) leads to
            // reaches nothing but 1F (27,1) and itself.
            Self::enter_at(Map::CeruleanCave2F, 3, 11),
            Self::enter_at(Map::CeruleanCave1F, 1, 3),
            Self::enter(Map::CeruleanCaveB1F),
            Self::CatchPokemon { species: PokemonSpecies::Mewtwo, on_map: Map::CeruleanCaveB1F,
                                 ball: None },
        ]);
        steps
    }
}

//! Earth Badge → Victory Road → the Elite Four → the Hall of Fame.

use super::*;

/// From `post-volcano-badge.bin` (in Blaine's gym with 7 badges — exactly where the mainline is;
/// the Seafoam detour is no longer on the route): Surf back to Pallet and up to Viridian, then
/// clear Giovanni's Viridian Gym spinner-tile maze for the Earth Badge, the 8th and final gym
/// badge.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_get_earth_badge() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-volcano-badge.bin"),
        Duration::from_mins(40),
        PolicyStep::earth_badge_steps(),
    );
    let s = fixture.run_until(|s| s.badges.contains(Badge::EarthBadge));
    println!("on {} @ {} — badges = {:?}", s.map.map, s.map.player_position, s.badges);
    fixture.save_state_named("src/pokemon/data/post-earth-badge.bin").unwrap();
}

/// Victory Road 1F: reach the cave, catch a wild Machop with the Master Ball as a Strength
/// HM-slave, teach it HM04, then push a boulder onto the (17,13) switch to open the (1,1) ladder
/// and climb to VR2F.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_solve_victory_road_1f() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-earth-badge.bin"),
        Duration::from_mins(180),
        PolicyStep::victory_road_1f_approach_steps(),
    ).with_stall_tolerance(Duration::from_mins(30));
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    let has_strength = s.pokemon.iter()
        .any(|p| p.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Strength));
    println!("final: {} @ {}  strength={has_strength}", s.map.map, s.map.player_position);
    assert!(has_strength, "a party member should know Strength for the boulder puzzle");
    assert_eq!(s.map.map, Map::VictoryRoad1F, "should walk to Victory Road 1F");
    fixture.save_state_named("src/pokemon/data/vr1f-strength.bin").unwrap();
}

/// The 1F boulder onto (17,13) and the climb to VR2F, split out from the approach above.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_climb_victory_road_1f() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/vr1f-strength.bin"),
        Duration::from_mins(60),
        PolicyStep::victory_road_1f_climb_steps(),
    );
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("final: {} @ {}", s.map.map, s.map.player_position);
    assert_eq!(s.map.map, Map::VictoryRoad2F, "should solve VR1F and climb to VR2F");
    fixture.save_state_named("src/pokemon/data/vr2f-ladder.bin").unwrap();
}

/// The interconnected VR2F/VR3F Strength puzzle, through to the Indigo Plateau lobby: switch1 →
/// 3F → hole-drop reveals the hidden 2F boulder → fall → switch2 → return trip → exit.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_solve_victory_road_2f_3f() {
    // The 1F climb is part of this test, not a leftover.
    let mut steps = PolicyStep::victory_road_1f_climb_steps();
    steps.extend(PolicyStep::victory_road_2f_3f_steps());
    let mut fixture = TestFixture::new(
        include_bytes!("../data/vr1f-strength.bin"),
        Duration::from_mins(60),
        steps,
    );
    // Stopped on the plateau *outside* the lobby, and the reason is not this test.
    let s = fixture.run_until(|s| s.map.map == Map::IndigoPlateau);
    println!("final: {} @ {}", s.map.map, s.map.player_position);
    fixture.save_state_named("src/pokemon/data/at-indigo.bin").unwrap();
}

/// The Elite Four gauntlet, from the Indigo Plateau lobby to the credits: stock up, heal, then
/// Lorelei → Bruno → Agatha → Lance → the rival, and on through Oak's post-Champion script into
/// the Hall of Fame.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_beat_elite_four() {
    const FIXTURE: &[u8] = include_bytes!("../data/at-indigo-articuno.bin");

    // 180 min covers the five rooms plus Oak's post-Champion speech and the walk to the Hall of
    // Fame.
    let mut fixture = TestFixture::new(
        FIXTURE,
        Duration::from_mins(180),
        PolicyStep::elite_four_steps(),
    ).with_original_battle_timing();

    // The rival's battle starts from a map script rather than from a step, and once it is won the
    // agent hands itself to `drive_post_champion_cutscene`, which stops polling the policy — so
    // the last steps stay queued and "done" is never an empty queue.
    fixture.run_until(|s| s.map.map == Map::ChampionsRoom);
    const SCRIPT_OAK_ARRIVES: u8 = 4;
    while fixture.gb.core().mmu().read_pointer(&pokered_symbols::wChampionsRoomCurScript) < SCRIPT_OAK_ARRIVES {
        fixture.step();
    }
    // Bank the moment of victory: everything past here is Oak's script chain, and iterating on
    // that from a snapshot takes seconds instead of re-fighting five rooms.
    fixture.save_state_named("src/pokemon/data/post-champion.bin").unwrap();

    let s = fixture.run_until(|s| s.map.map == Map::HallOfFame);
    println!("HALL OF FAME — final team:");
    for p in s.pokemon.iter() { println!("  {:?} lv{} {}/{}hp", p.species, p.level, p.current_hp, p.stats.hp); }
    fixture.save_state_named("src/pokemon/data/post-hall-of-fame.bin").unwrap();
}

/// The post-Champion cutscene on its own, from `post-champion.bin` (rival beaten, Oak about to
/// walk in) to the credits — no policy steps at all, because `drive_post_champion_cutscene`
/// drives it.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_enter_hall_of_fame() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-champion.bin"),
        Duration::from_mins(30),
        vec![],
    );
    let s = fixture.run_until(|s| s.map.map == Map::HallOfFame);
    println!("credits rolling at {} @ {}", s.map.map, s.map.player_position);
}

/// The mainline's own tail: Victory Road 2F to the Hall of Fame, from the party the playthrough
/// actually arrives with.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_finish_from_victory_road() {
    let mut steps = PolicyStep::victory_road_2f_3f_steps();
    steps.extend(PolicyStep::elite_four_steps());

    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-victory-road-1f.bin"),
        Duration::from_mins(3000),
        steps,
    );
    seed_gauntlet_levels(&mut fixture);
    {
        let s = fixture.game_state();
        println!("start: {} @ {} | ¥{}", s.map.map, s.map.player_position, s.money);
        for p in s.pokemon.iter() { println!("  {:?} lv{}", p.species, p.level); }
    }

    let s = fixture.run_until(|s| s.map.map == Map::HallOfFame);
    println!("HALL OF FAME — final team:");
    for p in s.pokemon.iter() { println!("  {:?} lv{} {}/{}hp", p.species, p.level, p.current_hp, p.stats.hp); }
}

/// Wind the three gauntlet fighters up to what `victory_road_grind_steps` would have left them
/// at.
fn seed_gauntlet_levels(fixture: &mut TestFixture) {
    // The Elixer is seeded for the same reason the levels are, and it is not decoration.
    fixture.api().debug_take_item(crate::pokemon::item::ItemId::Tm06Toxic)
        .expect("the fixture carries TM06 and never teaches it");
    fixture.api().debug_give_item(crate::pokemon::item::ItemId::Elixer, 1)
        .expect("the toss above freed a slot");
    let mut party = fixture.game_state().pokemon;
    for slot in 0..party.len() {
        let mon = party.get_mut(slot).expect("in range");
        if !matches!(mon.species, PokemonSpecies::Blastoise) { continue }
        if mon.level >= PolicyStep::GAUNTLET_LEVEL { continue }
        mon.experience = mon.species.metadata().experience_group
            .experience_for_level(PolicyStep::GAUNTLET_LEVEL);
        mon.recalculate();
        mon.current_hp = mon.stats.hp;
    }
    fixture.api().debug_set_party(&party).expect("the party is unchanged in length");
}

/// The gauntlet grind on its own, from the fixture the route reaches it at.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "20 min, the slowest test in the repo but one — \
    run with --features slow-tests")]
fn can_grind_for_the_gauntlet() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/post-articuno.bin"),
        Duration::from_mins(3000),
        PolicyStep::gauntlet_grind_steps(),
    );
    {
        let s = fixture.game_state();
        println!("start: {} @ {}", s.map.map, s.map.player_position);
        for p in s.pokemon.iter() { println!("  {:?} lv{}", p.species, p.level); }
    }
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    for p in s.pokemon.iter() { println!("  {:?} lv{}", p.species, p.level); }
    // One fighter, not three: the route grinds `STARTER_LINE` alone, and by the Mansion it is a
    // Blastoise.
    let fighter = s.pokemon.iter().find(|p| p.species == PokemonSpecies::Blastoise)
        .expect("the grind's target is the starter line, which is a Blastoise by the Mansion");
    assert!(fighter.level >= PolicyStep::GAUNTLET_LEVEL,
        "the fighter only reached lv{}", fighter.level);
}

/// Hold a plan of buttons against a dropped save state and print the map, the position, the game
/// mode and `wMovementFlags` as it goes, plus the map's raw tile ids up front.
/// ```text
/// GB_PROBE_STATE=target/test-artifacts/coverage/defect-X_state.bin GB_PROBE_BUTTONS=down:60,up:40,down:120 \
/// cargo test --release --features slow-tests --lib -- probe_button_at_state --ignored --nocapture
/// ```
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "probe — run with --ignored --nocapture, see the doc comment"]
fn probe_button_at_state() {
    use gb::joypad::JoypadButton;
    let path = std::env::var("GB_PROBE_STATE").expect("GB_PROBE_STATE");
    let plan = std::env::var("GB_PROBE_BUTTONS").unwrap_or_else(|_| "down:600".into());
    let bytes = std::fs::read(&path).expect("state");
    let mut fixture = TestFixture::new(&bytes, Duration::from_secs(600), Vec::new());
    let s = fixture.game_state();
    println!("start: {} @ {} facing {:?}", s.map.map, s.map.player_position, s.map.player_direction);
    println!("raw ids:");
    for y in 0..s.map.height {
        let row: Vec<String> = (0..s.map.width)
            .map(|x| format!("{:02x}", s.map.raw_tile_ids[x + y * s.map.width])).collect();
        println!("   {y:>2}: {}", row.join(" "));
    }
    for leg in plan.split(',') {
        let (name, n) = leg.split_once(':').expect("button:ticks");
        let button = match name {
            "up" => Some(JoypadButton::Up), "left" => Some(JoypadButton::Left),
            "right" => Some(JoypadButton::Right), "a" => Some(JoypadButton::A),
            "down" => Some(JoypadButton::Down), _ => None,
        };
        for tick in 0..n.parse::<usize>().expect("ticks") {
            fixture.api().release_all_buttons();
            if let Some(button) = button { fixture.api().press_button(button); }
            fixture.gb.run(crate::pokemon::agent::AGENT_RESOLUTION);
            if tick % 25 == 0 {
                let flags = {
                    
                    fixture.gb.core().mmu().read(
                        crate::pokemon::symbols::pokered_symbols::wMovementFlags.address)
                };
                match fixture.try_game_state() {
                    Ok(s) => println!("  {name} t{tick:>3}: {:?} @ {} mode {:?} movementFlags {flags:#04x}",
                        s.map.map, s.map.player_position, s.mode),
                    Err(why) => println!("  {name} t{tick:>3}: unreadable: {why}"),
                }
            }
        }
    }
}

/// Dump what the agent can see and reach from a save — map, position, money, party, bag, tile
/// under foot, sprites and every action.
/// ```text
/// GB_PROBE_STATE=src/pokemon/data/post-articuno.bin \
/// cargo test --release --features slow-tests --lib -- probe_stall_actions --ignored --nocapture
/// ```
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "probe — run with --ignored --nocapture, see the doc comment"]
fn probe_stall_actions() {
    let path = std::env::var("GB_PROBE_STATE")
        .unwrap_or_else(|_| "target/test-artifacts/test_stall_state.bin".to_string());
    let Ok(bytes) = std::fs::read(&path) else {
        println!("no state at {path}");
        return;
    };
    let mut fixture = TestFixture::new(&bytes, Duration::from_secs(1), Vec::new());
    let s = fixture.game_state();
    println!("{} @ {}  money ¥{}", s.map.map, s.map.player_position, s.money);
    for mon in s.pokemon.iter() {
        println!("  party {:?} lv{} {}/{}hp {:?}", mon.species, mon.level, mon.current_hp,
            mon.stats.hp, mon.status);
    }
    println!("  bag: {:?}", s.bag.iter().map(|i| (i.id, i.quantity)).collect::<Vec<_>>());
    if let Some(battle) = s.battle.as_ref() {
        println!("  battle: {:?}  catch rate {}  enemy trapping {}",
            battle.battle_type, battle.enemy_catch_rate, battle.enemy_trapping);
        println!("    yours: {:?} lv{} {}/{}hp speed {}", battle.player.species, battle.player.level,
            battle.player.current_hp, battle.player.stats.hp, battle.player.stats.speed);
        println!("    enemy: {:?} lv{} {}/{}hp speed {}", battle.enemy.species, battle.enemy.level,
            battle.enemy.current_hp, battle.enemy.stats.hp, battle.enemy.stats.speed);
        for row in crate::llm::tools::battle_menu(&s) {
            println!("    menu `{}` — {}", row.id, row.description);
        }
        return;
    }
    println!("tile under player: {:?}", s.map.tile_at_checked(s.map.player_position));
    // The grid, because a missing row is usually a tile rather than a bug in `actions()`.
    println!("{}", s.map);
    for sprite in &s.map.sprites {
        println!("  sprite {:?} hidden={} @ {}", sprite.name, sprite.hidden, sprite.position);
    }
    for action in s.map.actions() {
        println!("  action {:?} → {} ({} steps)", action.tile, action.destination, action.route.len());
    }
}

/// A boulder that will not move says so, instead of being shoved at for a minute.
#[test]
fn a_boulder_that_cannot_move_is_refused_rather_than_shoved_at() {
    // The save state out of `issues/turn-1701/` of run-20260902-215720, as the model was handed
    // it.
    let mut stuck = TestFixture::new(VR1F_STUCK_PUSH, Duration::from_secs(30), Vec::new());
    let state = stuck.game_state();
    let map = &state.map;
    assert_eq!(map.player_position, Point8 { x: 5, y: 15 }, "the deployed run stood here");
    assert!(state.strength_active, "Strength was armed; the stall was not the arming gate");

    // The tile is `Empty`, and that is the point.
    assert_eq!(map.tile_at(Point8 { x: 5, y: 13 }), crate::pokemon::tile::MetaTile::Empty);
    let refusal = map.boulder_push_refusal(Point8 { x: 5, y: 14 }, JoypadButton::Up)
        .expect("the push the deployed run hung on must be refused");
    assert!(refusal.contains("stairs"), "the reason has to be the real one: {refusal}");
    assert!(refusal.contains("(5, 13)"), "and has to name the square: {refusal}");

    // Every way out of the alcove needs a push tile the boulder itself now seals off, so the
    // honest answer is that this one is finished — which is a thing to be told, not to be shoved
    // at.
    for dir in [JoypadButton::Down, JoypadButton::Left, JoypadButton::Right] {
        assert!(map.boulder_push_refusal(Point8 { x: 5, y: 14 }, dir).is_some(),
            "the alcove is sealed by the boulder in it, so no push works: {dir:?}");
    }
    assert!(map.solve_boulder_push(Point8 { x: 17, y: 13 }).is_none(),
        "and the planner must not offer a route through a push the cartridge refuses");
    // The half that keeps this from reading as "the game is broken".
    assert!(refusal.contains("Leaving this map"), "a sealed boulder must name the way out: {refusal}");

    // The same floor before the run shoved the boulder into the corner still solves, so the rules
    // added here refuse the impossible push without taking the possible one away.
    let mut pristine = TestFixture::new(VR1F_STRENGTH, Duration::from_secs(30), Vec::new());
    let fresh = pristine.game_state();
    assert_eq!(fresh.map.boulder_push_refusal(Point8 { x: 5, y: 15 }, JoypadButton::Down), None,
        "the first push of the real solution is legal");
    let solution = fresh.map.solve_boulder_push(Point8 { x: 17, y: 13 })
        .expect("VictoryRoad1F's puzzle is still solvable from its starting layout");
    assert_eq!(solution.first(), Some(&(Point8 { x: 5, y: 15 }, JoypadButton::Down)));

    assert_eq!(fresh.map.tile_at(Point8 { x: 6, y: 15 }), crate::pokemon::tile::MetaTile::Obstacle,
        "the square this push would be made from is solid rock");
    let nowhere = fresh.map.boulder_push_refusal(Point8 { x: 5, y: 15 }, JoypadButton::Left)
        .expect("a push with nowhere to stand must be refused");
    assert!(nowhere.contains("(6, 15)") && nowhere.contains("nowhere to stand"), "{nowhere}");
    // And it is refused at the *row*, which is the seam the model actually meets.
    let goals: Vec<_> = fresh.map.actions().into_iter()
        .filter(|action| matches!(action.tile, crate::pokemon::tile::MetaTile::BoulderGoal { .. }))
        .collect();
    assert!(!goals.is_empty(), "VictoryRoad1F's switch is a goal row");
    for goal in goals {
        let crate::pokemon::tile::MetaTile::BoulderGoal { boulder, at, .. } = goal.tile else { unreachable!() };
        let plan = fresh.map.solve_boulder_push_for(boulder, at)
            .expect("a row is only minted for a goal that solves");
        let (first, push) = plan[0];
        assert_eq!(fresh.map.boulder_push_refusal(first, push), None,
            "the row for {boulder} -> {at} opens on a push the cartridge would refuse");
    }

    // And the driver, which is the seam that actually burned the minute.
    let asked = PushOnce::new(Point8 { x: 5, y: 14 }, JoypadButton::Up);
    let mut fixture = TestFixture::with_policy(VR1F_STUCK_PUSH, Duration::from_secs(20), Box::new(asked));
    let mut said = None;
    while fixture.total_cycles < fixture.max_cycles && said.is_none() {
        fixture.step();
        for event in fixture.agent.drain_events() {
            if let AgentEvent::TextBox { message } = &event {
                if message.contains("stairs") { said = Some(message.clone()); }
            }
        }
    }
    let said = said.expect("the driver must report the refusal rather than hold the direction");
    assert!(said.contains("(5, 13)"), "{said}");
    assert!(fixture.total_cycles.to_duration() < DRIVER_ESCAPE_SILENCE_SECS,
        "the refusal must arrive long before the 60 s escape that used to report it as a malfunction; \
         took {:?}", fixture.total_cycles.to_duration());
}

/// ⚰️ `a_boulder_row_arms_strength_and_pushes_on_one_decision` and
/// `victory_road_1f_is_solvable_from_the_action_menu_alone` were both here, and both are
/// `a_strength_puzzle_is_one_decision_rather_than_one_per_shove` now.

/// The one square the planner and the menu disagreed about.
#[test]
fn a_push_from_a_warp_tile_is_offered_because_victory_road_needs_one() {
    use crate::pokemon::tile::MetaTile;
    let mut fixture = TestFixture::new(VR1F_STRENGTH, Duration::from_secs(10), Vec::new());
    let mut state = fixture.game_state();
    let width = state.map.width;
    let (from, to) = (Point8 { x: 5, y: 15 }, Point8 { x: 9, y: 16 });
    for sprite in state.map.sprites.iter_mut() {
        if sprite.position == from { sprite.position = to; }
    }
    state.map.meta_tiles[from.x as usize + from.y as usize * width] = MetaTile::Empty;
    state.map.meta_tiles[to.x as usize + to.y as usize * width] = MetaTile::Sprite("Boulder 1");
    state.map.player_position = Point8 { x: 9, y: 15 };

    // The square in question is a warp, and it is the only one the push can be made from.
    let stand = Point8 { x: 9, y: 17 };
    assert!(matches!(state.map.tile_at(stand), MetaTile::Warp { .. }), "(9, 17) is the entrance");
    assert_eq!(state.map.boulder_push_refusal(to, JoypadButton::Up), None,
        "the push the solver wants must be one the menu will offer");

    // And the walk to it must exist under the same rules, or the row is a decision the driver
    // cannot carry out — `PushingBoulder` finds no route and drops to `Idle` without a word.
    let route = state.map.route_to_push_tile(stand).expect("a walk to the push tile");
    assert!(!route.is_empty() && !route.contains(&JoypadButton::Start), "{route:?}");
    assert_eq!(route.last(), Some(&JoypadButton::Right), "it arrives from the west, not from above");

    // The way on has to be a *row*, and since the menu offers goals rather than shoves that means
    // the switch is still offered from this layout — with a plan that opens on the warp-tile
    // push.
    let switch = Point8 { x: 17, y: 13 };
    let goal = state.map.actions().into_iter()
        .find(|action| matches!(action.tile, MetaTile::BoulderGoal { at, .. } if at == switch))
        .expect("the way on has to be a row");
    let MetaTile::BoulderGoal { boulder, .. } = goal.tile else { unreachable!() };
    let plan = state.map.solve_boulder_push_for(boulder, switch).expect("solvable from here");
    assert!(plan.contains(&(to, JoypadButton::Up)),
        "the plan behind the row is the one that pushes from the warp tile: {plan:?}");
}

/// The bound `a_boulder_that_cannot_move_is_refused_rather_than_shoved_at` holds the driver to.
const DRIVER_ESCAPE_SILENCE_SECS: Duration = Duration::from_secs(10);

const VR1F_STUCK_PUSH: &[u8] = include_bytes!("../data/vr1f-stuck-push.bin");
const VR1F_STRENGTH: &[u8] = include_bytes!("../data/vr1f-strength.bin");

/// A policy that asks for one boulder push and nothing else, so the test drives the agent's
/// `PushingBoulder` seam directly.
struct PushOnce {
    boulder: Point8,
    dir: JoypadButton,
    asked: bool,
}

impl PushOnce {
    fn new(boulder: Point8, dir: JoypadButton) -> Self { Self { boulder, dir, asked: false } }
}

impl crate::pokemon::policy::Policy for PushOnce {
    fn name(&self) -> &'static str { "push-once" }
    fn pick_overworld_action(&mut self, _: &GameState, _: &crate::pokemon::world_graph::WorldGraph)
        -> Option<crate::pokemon::actions::OverworldAction> { None }
    fn pick_battle_action(&mut self, _: &GameState) -> Option<crate::pokemon::battle::BattleAction> { None }
    fn pick_field_move(&mut self, _: &GameState) -> Option<crate::pokemon::policy::FieldMove> {
        if self.asked { return None; }
        self.asked = true;
        Some(crate::pokemon::policy::FieldMove::PushBoulder { boulder: self.boulder, dir: self.dir })
    }
}

/// And the way out of a boulder that cannot be pushed is the door, which is worth pinning because
/// it is the sentence the refusal above ends on.
#[test]
fn leaving_a_map_puts_its_boulders_back() {
    let mut fixture = TestFixture::new(VR1F_STUCK_PUSH, Duration::from_mins(4), vec![
        PolicyStep::goto(Map::Route23),
        PolicyStep::goto(Map::VictoryRoad1F),
    ]);
    let state = fixture.run_leg(|s| s.map.map == Map::VictoryRoad1F
        && s.map.sprites.iter().any(|sprite| sprite.name == "Boulder 1" && !sprite.hidden
            && sprite.position == Point8 { x: 5, y: 15 }));
    assert_eq!(state.map.map, Map::VictoryRoad1F);
    // Where the map header puts it, not the alcove the run had shoved it into.
    let boulder = state.map.sprites.iter().find(|s| s.name == "Boulder 1")
        .expect("VictoryRoad1F has a boulder");
    assert_eq!(boulder.position, Point8 { x: 5, y: 15 }, "a re-entered map re-reads its objects");
    assert_eq!(state.map.boulder_push_refusal(Point8 { x: 5, y: 15 }, JoypadButton::Down), None,
        "and the puzzle is winnable again");
}

/// The whole Victory Road 1F puzzle as one decision, which is what `MetaTile::BoulderGoal` is
/// for.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn a_strength_puzzle_is_one_decision_rather_than_one_per_shove() {
    const SWITCH: Point8 = Point8 { x: 17, y: 13 };

    // Generous: the whole point is that this is several shoves, and the first one pays for the
    // Strength arming menu on top.
    let mut fixture = TestFixture::new(VR1F_STRENGTH, Duration::from_mins(30), vec![]);
    let state = fixture.game_state();
    assert!(state.map.can_strength, "the fixture carries Strength and the badge");
    assert!(!state.map.boulders().contains(&SWITCH), "nothing is on the switch yet");

    // The row exists, names the goal, and its id carries the target — see `MetaTile::id_kind`.
    let goal = state.map.actions().into_iter()
        .find(|action| matches!(action.tile, MetaTile::BoulderGoal { at, hole: false, .. } if at == SWITCH))
        .expect("the menu offers the switch as a goal");
    let MetaTile::BoulderGoal { boulder, .. } = goal.tile else { unreachable!() };
    // The row names the boulder as well as the target, so a floor with two of each is not
    // ambiguous — see `MetaTile::BoulderGoal`.
    assert!(format!("{}", goal.tile).contains("to push it onto the switch at (17, 13)"), "{}", goal.tile);
    assert!(format!("{}", goal.tile).contains(&format!("({}, {})", boulder.x, boulder.y)), "{}", goal.tile);

    // And the *id* names neither the boulder nor the square the walk starts from, because both
    // move on every push.
    assert_eq!(goal.id(), "VictoryRoad1F:17,13:PushBoulderOntoSwitch");
    let id = goal.id();

    // One action, then nothing.
    fixture.agent.take_overworld_action(goal);

    let shoved = fixture.run_until(|state| !state.map.boulders().contains(&boulder));
    let after = shoved.map.actions().into_iter()
        .find(|a| matches!(a.tile, MetaTile::BoulderGoal { at, .. } if at == SWITCH))
        .expect("the goal is still a row once its boulder has moved");
    assert_eq!(after.id(), id, "one puzzle is one id, however far along it is");
    let landed = fixture.run_until(|state| state.map.boulders().contains(&SWITCH));
    println!("boulder landed on the switch at {} after one decision", landed.map.player_position);

    // And it has to *say* it landed.
    let mut reported = false;
    for _ in 0..600 {
        for event in fixture.agent.drain_events() {
            if let AgentEvent::OverworldActionCompleted {
                destination: MetaTile::BoulderGoal { at, .. } } = event
            {
                if at == SWITCH { reported = true; }
            }
        }
        if reported { break }
        fixture.step();
    }
    assert!(reported, "the goal completed and never said so");
}

/// Scratch: a fixture standing on VictoryRoad3F with Strength armed, before its switch puzzle.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn cut_vr3f_fixture() {
    let mut steps = PolicyStep::victory_road_1f_climb_steps();
    // The 2F half, up to and including arming Strength on 3F — i.e. stop before `SolveBoulders`
    // for the (3, 5) switch, which is the puzzle under test.
    let half = PolicyStep::victory_road_2f_3f_steps();
    let stop = half.iter().enumerate()
        .filter(|(_, s)| matches!(s, PolicyStep::SolveBoulders { switch, .. } if *switch == Point8 { x: 3, y: 5 }))
        .map(|(i, _)| i).next().expect("the 3F switch step");
    steps.extend(half.into_iter().take(stop));
    let mut fixture = TestFixture::new(VR1F_STRENGTH, Duration::from_mins(60), steps);
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("stopped on {} @ {} boulders {:?}", s.map.map, s.map.player_position, s.map.boulders());
    assert_eq!(s.map.map, Map::VictoryRoad3F);
    assert!(s.map.can_strength);
    fixture.save_state_named("src/pokemon/data/vr3f-strength.bin").unwrap();
}

/// A policy that answers with the goal row for `switch` every time it is asked, which is what
/// both the coverage explorer and a model do: an aborted action comes back to be chosen again.
struct AlwaysTheGoal { switch: Point8, battles: crate::pokemon::policy::RandomPolicy }
impl AlwaysTheGoal {
    fn new(switch: Point8) -> Self {
        Self { switch, battles: crate::pokemon::policy::RandomPolicy::seeded(7) }
    }
}
impl crate::pokemon::policy::Policy for AlwaysTheGoal {
    fn name(&self) -> &'static str { "always-the-goal" }
    fn pick_overworld_action(&mut self, state: &GameState, _: &crate::pokemon::world_graph::WorldGraph)
        -> Option<crate::pokemon::actions::OverworldAction> {
        state.map.actions().into_iter().find(|a| matches!(a.tile,
            crate::pokemon::tile::MetaTile::BoulderGoal { at, .. } if at == self.switch))
    }
    fn pick_battle_action(&mut self, state: &GameState) -> Option<crate::pokemon::battle::BattleAction> {
        self.battles.pick_battle_action(state)
    }
    fn pick_field_move(&mut self, _: &GameState) -> Option<crate::pokemon::policy::FieldMove> { None }
}

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn victory_roads_hardest_switch_is_one_decision_however_many_shoves_it_takes() {
    use crate::pokemon::tile::MetaTile;
    const SWITCH: Point8 = Point8 { x: 3, y: 5 };
    let mut fixture = TestFixture::new(
        include_bytes!("../data/vr3f-strength.bin"), Duration::from_mins(30), vec![]);
    let state = fixture.game_state();
    assert!(!state.map.boulders().contains(&SWITCH), "nothing is on the switch yet");
    let goal = state.map.actions().into_iter()
        .find(|a| matches!(a.tile, MetaTile::BoulderGoal { at, .. } if at == SWITCH))
        .expect("the menu offers VictoryRoad3F's switch as a goal");
    let MetaTile::BoulderGoal { boulder, .. } = goal.tile else { unreachable!() };
    let plan = state.map.solve_boulder_push_for(boulder, SWITCH).expect("the floor is solvable");
    assert!(plan.len() > 24,
        "this test is worth nothing unless the floor needs more than the old 24-shove budget; \
         the plan is {} pushes", plan.len());

    fixture.agent.take_overworld_action(goal);
    let landed = fixture.run_until(|state| state.map.boulders().contains(&SWITCH));
    println!("a {}-push puzzle solved from one decision; ended at {}",
             plan.len(), landed.map.player_position);
}

/// A goal survives being re-chosen, on the floor where that is hardest.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn a_boulder_goal_re_chosen_after_every_battle_still_arrives() {
    const SWITCH: Point8 = Point8 { x: 3, y: 5 };
    let mut fixture = TestFixture::with_policy(
        include_bytes!("../data/vr3f-strength.bin"), Duration::from_mins(20),
        Box::new(AlwaysTheGoal::new(SWITCH)));
    let mut ids: std::collections::BTreeSet<String> = Default::default();
    let mut starts = 0u32;
    let pressed = |f: &mut TestFixture| f.game_state().map.boulders().contains(&SWITCH);
    assert!(!pressed(&mut fixture), "the fixture starts with the switch unpressed");
    while fixture.total_cycles < fixture.max_cycles && !pressed(&mut fixture) {
        fixture.step();
        for event in fixture.agent.drain_events() {
            if let AgentEvent::StartedOverworldAction { destination: MetaTile::BoulderGoal { .. }, id } = event {
                starts += 1;
                ids.insert(id);
            }
        }
    }
    println!("switch pressed after {starts} start(s); ids used: {ids:?}");
    assert!(pressed(&mut fixture),
        "a boulder has to reach {SWITCH} even though the row is re-chosen every time it aborts");
    assert_eq!(ids.len(), 1, "the goal must keep one id across every re-pick: {ids:?}");
}

/// A Strength floor that has been wedged says the door is the way out, not that the pathfinder is
/// broken.
#[test]
fn a_wedged_strength_floor_is_reported_as_a_reset_rather_than_a_missing_route() {
    use crate::pokemon::agent::OverworldActionAbortedReason;
    use crate::pokemon::tile::MetaTile;
    let reason = OverworldActionAbortedReason::PuzzleUnsolvable;
    let said = format!("{reason}");
    assert!(said.contains("no boulder on this floor"), "{said}");
    // It must not say "route", which is the word that reads as a pathfinder fault.
    assert!(!said.contains("route"), "the one word this sentence must not use: {said}");
    assert!(said.contains("leaving this floor and coming back"), "it has to name the way out: {said}");

    let event = crate::pokemon::agent::AgentEvent::OverworldActionAborted {
        destination: MetaTile::BoulderGoal {
            boulder: Point8 { x: 23, y: 16 }, at: Point8 { x: 9, y: 16 }, hole: false },
        reason,
        at: Some(Point8 { x: 5, y: 11 }),
    };
    let line = format!("{event}");
    assert!(line.contains("(9, 16)"), "the target belongs in the line: {line}");
    println!("{line}");
}

#[test]
fn a_boulder_goal_that_keeps_shoving_is_not_a_driver_the_game_has_gone_quiet_on() {
    use crate::pokemon::agent::DRIVER_ESCAPE_SILENCE;
    use crate::pokemon::tile::MetaTile;
    const SWITCH: Point8 = Point8 { x: 3, y: 5 };
    let mut fixture = TestFixture::new(
        include_bytes!("../data/vr3f-strength.bin"), Duration::from_mins(10), vec![]);
    let state = fixture.game_state();
    let goal = state.map.actions().into_iter()
        .find(|a| matches!(a.tile, MetaTile::BoulderGoal { at, .. } if at == SWITCH))
        .expect("the menu offers VictoryRoad3F's switch as a goal");
    fixture.agent.take_overworld_action(goal);

    let start = fixture.total_cycles;
    let mut ended: Option<String> = None;
    let mut silences: Vec<String> = Vec::new();
    let (mut peak_poll, mut peak_answer) = (Duration::ZERO, Duration::ZERO);
    while fixture.total_cycles < fixture.max_cycles && ended.is_none() {
        // Held up rather than set once: the counter is spent one per overworld step and this goal
        // walks further than the byte can count.
        fixture.api().debug_set_repel_steps(u8::MAX);
        fixture.step();
        peak_poll = peak_poll.max(fixture.agent.since_last_policy_poll());
        peak_answer = peak_answer.max(fixture.agent.since_driver_answer());
        for event in fixture.agent.drain_events() {
            match &event {
                AgentEvent::OverworldActionCompleted { destination: MetaTile::BoulderGoal { .. } } =>
                    ended = Some("completed".to_string()),
                AgentEvent::OverworldActionAborted { destination: MetaTile::BoulderGoal { .. }, reason, .. } =>
                    ended = Some(format!("aborted: {reason}")),
                AgentEvent::TextBox { message } if message.contains("no answer") =>
                    silences.push(message.clone()),
                _ => {}
            }
        }
    }
    let took = (fixture.total_cycles - start).to_duration();
    println!("one decision: {ended:?} after {took:?}; policy unasked for {peak_poll:?}, \
              longest the game went without answering the driver {peak_answer:?}");

    assert!(silences.is_empty(), "the game answered every shove: {silences:?}");
    assert_eq!(ended.as_deref(), Some("completed"),
        "a {took:?} boulder goal that lands every shove has to finish");
    // This is the line the fix is about.
    assert!(peak_poll > DRIVER_ESCAPE_SILENCE / 2,
        "the goal has to spend a good part of the bound with the policy unasked or this test proves \
         nothing; it was only {peak_poll:?}. Is the Repel holding?");
    assert!(peak_answer < DRIVER_ESCAPE_SILENCE / 2,
        "the game answered a shove every {peak_answer:?}, which the hatch must be measuring instead \
         of the {peak_poll:?} the policy went unasked");
}

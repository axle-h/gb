//! The whole game, start to finish, in one run.

use super::*;

/// Where along the eight-badge route [`super::soak`] turns its fuzzer loose, and the map each of
/// those save states is cut on.
///
/// ⚠️ **These are seeds for a fuzzer, not links in the fixture chain.** Nothing reads one as the
/// *input* to a route the way `at-cerulean.bin` feeds the leg tests, so the rules on
/// [`TestFixture::save_state_named`] about cutting where the mainline stands and where the party is
/// healed do not apply: a capture taken mid-dungeon, mid-errand and half-poisoned is a better
/// starting point for a random walker than a tidy one, because it is a state a deployed run can
/// really be in.
///
/// ⚠️ **The map is the whole specification, and [`regen_soak_checkpoints`] is what honours it.** A
/// checkpoint is the first moment the scripted run has stood on that map for
/// [`CHECKPOINT_SETTLE_TICKS`], so adding one is a line here and a 7-minute regeneration — no
/// fixture is cut by hand, and none of them can drift away from the route, because they are the
/// route. `soak`'s own `expect_map` re-asserts each one on the way in, so a stale capture fails
/// where it is used rather than silently fuzzing somewhere else.
///
/// ⚠️ **Chosen for what a fuzzer can reach from them, exactly as [`super::soak::STATES`] is** — a
/// dark cave, a quiz gym, a warp maze, a step counter — and deliberately *not* for the maps the
/// curated states already cover. This list and the hand-cut states are two halves of one budget:
/// these buy the ground the route crosses for nothing, and the curated ones buy the ground it never
/// does (a bicycle, a full PC box, a ledge pocket a real run got stuck in).
pub(super) const SOAK_CHECKPOINTS: &[(&str, Map)] = &[
    ("soak-mt-moon", Map::MtMoonB2F),
    ("soak-ss-anne", Map::SSAnne1F),
    ("soak-rock-tunnel", Map::RockTunnel1F),
    ("soak-pokemon-tower", Map::PokemonTower5F),
    ("soak-route12-snorlax", Map::Route12),
    ("soak-safari-zone", Map::SafariZoneCenter),
    ("soak-silph-co", Map::SilphCo3F),
    ("soak-saffron-gym", Map::SaffronGym),
    ("soak-pokemon-mansion", Map::PokemonMansionB1F),
    ("soak-cinnabar-gym", Map::CinnabarGym),
    ("soak-viridian-gym", Map::ViridianGym),
    ("soak-route23", Map::Route23),
];

/// How long the run has to have been standing on a checkpoint's map before the state is taken —
/// 50 ticks of [`AGENT_RESOLUTION`], one second of game time.
///
/// ⚠️ **Not the first tick the map id changes.** A warp lands with the map header read and the rest
/// of the world still being built: the connection strips, the sprite slots and the tile map arrive
/// over the following frames, and a state cut in that window loads into an agent that reads a map
/// half of which is the previous one. A second is far longer than that takes and far shorter than
/// the run spends anywhere it is worth starting a fuzzer from.
const CHECKPOINT_SETTLE_TICKS: u32 = 50;

/// Re-cut every [`SOAK_CHECKPOINTS`] state by playing the eight-badge route once.
///
/// ⚠️ **A test of its own rather than a hook inside [`full_playthrough`], and that is deliberate.**
/// `full_playthrough` is the gate everything else is measured against; it reads the game state once
/// per tick only when it has to, and it already rewrites one committed fixture under
/// `regen-fixtures`. Hanging a dozen more writes off it would mean the run that proves the route
/// still works is also the run that quietly replaces a dozen inputs, and any per-tick cost added
/// here would be paid by the seven-minute gate rather than by the regeneration nobody runs weekly.
///
/// ```text
/// cargo test --release --features full-playthrough,regen-fixtures --bin gb -- \
///   regen_soak_checkpoints --exact --nocapture
/// ```
///
/// It prints every distinct map the route visited, in order, which is the list to pick a new
/// checkpoint from — and it **fails** naming any declared checkpoint the run never stood on, so a
/// map that drops off the route cannot leave a stale `.bin` behind pretending to still be on it.
#[test]
#[cfg(all(feature = "full-playthrough", feature = "regen-fixtures"))]
fn regen_soak_checkpoints() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(800),
        PolicyStep::eight_badge_steps(),
    );

    let mut written: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut visited: Vec<Map> = Vec::new();
    let mut standing_on: Option<(Map, u32)> = None;

    while !fixture.agent.policy_exhausted() {
        fixture.step();
        let Ok(state) = fixture.try_game_state() else { continue };
        let map = state.map.map;

        standing_on = match standing_on {
            Some((was, ticks)) if was == map => Some((map, ticks + 1)),
            _ => {
                if visited.last() != Some(&map) { visited.push(map); }
                Some((map, 0))
            }
        };
        let Some((_, ticks)) = standing_on else { continue };
        if ticks != CHECKPOINT_SETTLE_TICKS { continue }

        for (name, want) in SOAK_CHECKPOINTS {
            if *want != map || written.contains(name) { continue }
            let path = format!("src/pokemon/data/{name}.bin");
            fixture.gb.save_state_to_file(&path).expect("write a soak checkpoint");
            written.insert(name);
            println!("[checkpoint] {name} — {map} at {} ({:?} of game time, party {:?})",
                     state.map.player_position, fixture.total_cycles.to_duration(),
                     state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
        }
    }

    println!("\n[checkpoint] maps visited, in order:");
    for map in &visited { println!("    {map}"); }

    let missed: Vec<&str> = SOAK_CHECKPOINTS.iter()
        .map(|(name, _)| *name).filter(|name| !written.contains(name)).collect();
    assert!(missed.is_empty(),
            "the route never stood on the map(s) these checkpoints name: {missed:?} — pick \
             replacements from the visited list above, or drop them from SOAK_CHECKPOINTS");
    println!("[checkpoint] re-cut {} soak fixtures", written.len());
}

/// Resume [`full_playthrough`] from the save state a stalled run drops in `target/test-artifacts/`,
/// with the steps it had left still queued — so a stall 270 steps in can be re-tested in seconds
/// instead of re-running the whole 20 minutes up to it.
///
/// A save state carries the emulator's RNG registers, so resuming continues the *same* stream the run
/// was on. That is what makes this valid for chasing a route bug and **invalid as a substitute for the
/// real run**: it proves a fix works from that point, not that the run still reaches that point.
/// Always finish with a clean `full_playthrough`.
///
/// `RESUME_QUEUE_LEN` is the `queue_len=` from the last `[policy]` line of the stalled run.
/// ```text
/// RESUME_QUEUE_LEN=233 cargo test --release --features full-playthrough --bin gb -- \
///   probe_resume_playthrough --exact --ignored --nocapture
/// ```
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "probe — run with --ignored --nocapture, see the doc comment"]
fn probe_resume_playthrough() {
    let Ok(bytes) = std::fs::read("target/test-artifacts/test_stall_state.bin") else {
        println!("no stall artifact — run full_playthrough first");
        return;
    };
    let all = PolicyStep::complete_game_steps();
    let remaining: usize = std::env::var("RESUME_QUEUE_LEN").ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(all.len());
    let from = all.len().saturating_sub(remaining);
    println!("resuming at step {from} of {} ({remaining} queued)", all.len());
    for (i, step) in all.iter().skip(from).take(6).enumerate() {
        println!("  [{}] {step:?}", from + i);
    }

    let mut fixture = TestFixture::new(&bytes, Duration::from_mins(800),
        all[from..].to_vec());
    let s = fixture.game_state();
    println!("resume state: {} @ {} — party {:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    // The bag and the reachable set are the two things a stall is usually *about*: an item a gift or a
    // purchase silently failed to deliver, or an exit the pathfinder cannot see from where it stands.
    println!("   bag[{}]: {:?}", s.bag.len(), s.bag.iter().map(|i| i.id).collect::<Vec<_>>());
    println!("   tile under player: {:?}", s.map.tile_at_checked(s.map.player_position));
    for sprite in &s.map.sprites {
        println!("   sprite {:?} hidden={} @ {}", sprite.name, sprite.hidden, sprite.position);
    }
    for action in s.map.actions() {
        println!("   action {:?} @ {} ({} steps)", action.tile, action.destination, action.route.len());
    }
    fixture.step_until_exhausted();
    let s = fixture.game_state();
    println!("resume ended: {} @ {} badges={:?}", s.map.map, s.map.player_position, s.badges);
}

/// Hunt the move-selection bug across the whole scripted run and drop a save state at each hit.
///
/// The agent confirms whatever row the cursor is on, so the move the cartridge executes is sometimes
/// not the move the policy chose. `mechanics::the_move_the_agent_confirms_is_the_move_the_policy_asked_for`
/// reproduces it from one fixture; this finds every occurrence on the mainline and writes the state
/// **as it stood a few ticks before each bad confirm**, so each one becomes a fixture a fix can be
/// tried against directly instead of by re-running seven minutes of game.
///
/// ⚠️ **The ring is what makes the artifact useful.** The mismatch is only *detectable* from the text
/// box after the fact, by which time the press that caused it is long gone — so a state saved at the
/// moment of detection reproduces nothing. States are kept from the ticks while a battle move list is
/// on screen and the oldest is written out, which is before the cursor was driven.
///
/// ```text
/// cargo test --release --features diagnostics,full-playthrough --bin gb -- \
///   probe_move_mismatches --exact --ignored --nocapture
/// ```
#[test]
#[cfg(all(feature = "diagnostics", feature = "full-playthrough"))]
#[ignore = "probe — run with --ignored --nocapture, see the doc comment"]
fn probe_move_mismatches() {
    use crate::pokemon::battle::BattleAction;

    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(800),
        PolicyStep::eight_badge_steps(),
    );

    let dir = std::path::Path::new("target/test-artifacts/move-mismatch");
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).expect("artifact dir");

    // Six ticks is 120 ms of game time: comfortably before the cursor was driven to the row that
    // was confirmed, and short enough that the state still describes the same battle turn.
    const RING: usize = 6;
    let mut ring: std::collections::VecDeque<(Vec<u8>, String)> = std::collections::VecDeque::new();
    let mut intent: Option<(String, u8, String)> = None;
    let mut found = 0usize;
    let mut matched = 0usize;

    while !fixture.agent.policy_exhausted() {
        // Snapshot only while a move list is up — a state is ~24 µs and 6.4 kB, which is affordable
        // per battle turn and not per tick of a seven-hour run.
        let on_move_list = {
            let api = fixture.api();
            use crate::pokemon::PokemonApiTrait;
            matches!(api.menu_state().and_then(|m| m.battle_menu_state()),
                     Some(crate::pokemon::menu::BattleMenuState::MoveList { .. }))
        };
        if on_move_list {
            let where_ = {
                let s = fixture.game_state();
                format!("{:?} @ {} party {:?}", s.map.map, s.map.player_position,
                        s.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>())
            };
            if let Ok(bytes) = fixture.gb.save_state() {
                ring.push_back((bytes, where_));
                while ring.len() > RING {
                    ring.pop_front();
                }
            }
        }

        fixture.step();

        for event in fixture.agent.drain_events() {
            match &event {
                AgentEvent::BattleActionStarted { actor, action: BattleAction::Fight { slot, battle_move }, .. } =>
                    intent = Some((actor.to_string(), *slot, battle_move.name.to_string())),
                AgentEvent::TextBox { message } => {
                    let Some((actor, slot, wanted)) = intent.clone() else { continue };
                    let Some(rest) = message.split(&format!("{actor} used ")).nth(1) else { continue };
                    let Some(actual) = rest.split('!').next() else { continue };
                    let normalise = |s: &str| s.to_lowercase().replace(['-', ' '], "");
                    if normalise(actual) == "struggle" {
                        intent = None;
                        continue;
                    }
                    if normalise(actual) == normalise(&wanted) {
                        matched += 1;
                    } else if let Some((bytes, where_)) = ring.front() {
                        found += 1;
                        let stem = dir.join(format!("{found:02}-slot{slot}-{wanted}-got-{}", normalise(actual)));
                        let _ = std::fs::write(stem.with_extension("bin"), bytes);
                        let _ = std::fs::write(
                            stem.with_extension("txt"),
                            format!("wanted slot {slot} {wanted}, cartridge did {actual}\n{where_}\n"),
                        );
                        println!("MISMATCH #{found}: slot {slot} {wanted} -> {actual}  [{where_}]");
                    }
                    intent = None;
                }
                _ => {}
            }
        }
    }

    println!("\n{matched} battle turns executed the chosen move, {found} did not");
    println!("states written to {}", dir.display());
}

/// A policy that fights with one fixed move slot, for replaying a captured mismatch.
#[cfg(feature = "diagnostics")]
struct FixedSlot {
    slot: u8,
    asked: std::rc::Rc<std::cell::RefCell<Option<String>>>,
}

#[cfg(feature = "diagnostics")]
impl crate::pokemon::policy::Policy for FixedSlot {
    fn name(&self) -> &'static str { "fixed-slot" }
    fn pick_overworld_action(&mut self, _s: &GameState, _g: &crate::pokemon::world_graph::WorldGraph)
        -> Option<crate::pokemon::actions::OverworldAction> { None }
    fn pick_battle_action(&mut self, state: &GameState) -> Option<crate::pokemon::battle::BattleAction> {
        use crate::pokemon::battle::BattleAction;
        let options = crate::pokemon::policy::battle_options(state)?;
        let chosen = options.iter()
            .find(|o| matches!(o, BattleAction::Fight { slot, .. } if *slot == self.slot))
            .or_else(|| options.iter().find(|o| matches!(o, BattleAction::Fight { .. })))?;
        if let BattleAction::Fight { battle_move, .. } = chosen {
            *self.asked.borrow_mut() = Some(battle_move.name.to_string());
        }
        Some(*chosen)
    }
}

/// Replay every state [`probe_move_mismatches`] captured and report which still pick the wrong move.
///
/// ⚠️ **This is the regression harness a fix has to satisfy, and it is not a substitute for the
/// run.** Each state is the same battle a few ticks before the bad confirm, so a fix can be tried
/// against a dozen real sites in seconds instead of five minutes — but a save state carries the RNG
/// registers, so this proves a fix works *from these points*, not that the run still reaches them.
/// Always finish with a clean `full_playthrough`.
///
/// ```text
/// cargo test --release --features diagnostics --bin gb -- \
///   pokemon::integration_tests::playthrough::probe_replay_move_mismatches --exact --ignored --nocapture
/// ```
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "probe — run with --ignored --nocapture, see the doc comment"]
fn probe_replay_move_mismatches() {
    let dir = std::path::Path::new("target/test-artifacts/move-mismatch");
    let Ok(entries) = std::fs::read_dir(dir) else {
        println!("no captured states — run probe_move_mismatches first");
        return;
    };
    let mut states: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "bin"))
        .collect();
    states.sort();

    // One state at a time when tracing: `GB_REPLAY_ONLY=02`.
    let only = std::env::var("GB_REPLAY_ONLY").ok();
    let mut wrong = 0usize;
    let mut right = 0usize;
    for path in &states {
        if let Some(only) = &only {
            if !path.file_name().unwrap().to_string_lossy().starts_with(only.as_str()) { continue }
        }
        // The filename carries what was asked for: NN-slotS-Wanted-got-actual.bin
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let Some(slot) = stem.split("-slot").nth(1).and_then(|r| r.as_bytes().first())
            .and_then(|b| (*b as char).to_digit(10)) else { continue };
        let asked = std::rc::Rc::new(std::cell::RefCell::new(None));
        let mut fixture = TestFixture::with_policy(
            &std::fs::read(path).expect("state"),
            Duration::from_secs(300),
            Box::new(FixedSlot { slot: slot as u8, asked: std::rc::Rc::clone(&asked) }),
        );

        let mut verdict = None;
        let mut intent: Option<(String, String)> = None;
        'ticks: for _ in 0..3_000 {
            fixture.step();
            for event in fixture.agent.drain_events() {
                match &event {
                    AgentEvent::BattleActionStarted { actor, action: crate::pokemon::battle::BattleAction::Fight { battle_move, .. }, .. } =>
                        intent = Some((actor.to_string(), battle_move.name.to_string())),
                    AgentEvent::TextBox { message } => {
                        let Some((actor, wanted)) = intent.clone() else { continue };
                        let Some(rest) = message.split(&format!("{actor} used ")).nth(1) else { continue };
                        let Some(actual) = rest.split('!').next() else { continue };
                        let n = |s: &str| s.to_lowercase().replace(['-', ' '], "");
                        if n(actual) == "struggle" { intent = None; continue }
                        verdict = Some((wanted, actual.to_string(), n(actual) == n(&intent.clone().unwrap().1)));
                        break 'ticks;
                    }
                    _ => {}
                }
            }
        }
        match verdict {
            Some((wanted, actual, true)) => { right += 1; println!("  OK   {stem}: asked {wanted}, got {actual}") }
            Some((wanted, actual, false)) => { wrong += 1; println!("  WRONG {stem}: asked {wanted}, got {actual}") }
            None => println!("  ????  {stem}: no battle turn resolved in budget"),
        }
    }
    println!("\n{} states: {right} correct, {wrong} still wrong", states.len());
}

/// The full end-to-end playthrough — the single source of truth for how far the agent can play. From a
/// fresh `RedsHouse2F` save it plays legitimately (button input only, starting from a **Squirtle**) and
/// earns **all 8 gym badges**: Boulder → (Nugget Bridge → Bill → ) Cascade → Thunder → Rainbow →
/// (Silph Scope → Poké Flute → Snorlax) → Soul → (Safari Surf/Strength) → Silph Co (Card Key → rival →
/// Giovanni → liberation) → Marsh → surf to Cinnabar → Pokémon Mansion Secret Key → Volcano → back to
/// the Viridian Gym for **Earth** (Giovanni), with the **Seafoam Islands** detour for Articuno slotted
/// in between Volcano and Earth.
///
/// It catches exactly two things on the way and neither of them fights: an **Oddish** on Route 25 to
/// carry Cut, and a **Machop** on Victory Road to carry Strength. Blastoise does everything else, with
/// Surf, Blizzard and Dig — Surf is the one HM on it, because Surf is a 95-power STAB attack that
/// happens to be an HM.
///
/// It emulates every frame, so even in `--release` it takes ~7 min of wall clock — hence its own
/// feature gate, separate from the leg chain:
/// `cargo test --release --features full-playthrough full_playthrough`. The per-leg tests (each seeded
/// from a saved fixture) cover the same ground quickly and in parallel.
///
/// ⚠️ **Run this after every major work item and always before pushing** (see CLAUDE.md). It is the
/// only test that proves the legs *compose*; the leg tier proves each leg from a committed fixture and
/// is systematically blind to three things — a leg that only passes because `run_leg` kept stepping
/// after its queue emptied, a fixture that hands a leg a party or a bag the run could not actually
/// have earned, and any change to frame timing re-rolling the RNG stream every route is tuned against.
///
/// ⚠️ **Its end point is Victory Road 2F, and that is a cost decision rather than a limitation.**
/// The route `gb serve --policy deterministic` plays goes all the way to the Hall of Fame — see
/// [`PolicyStep::complete_game_steps`] and `hall_of_fame_playthrough`, which is the same run carried
/// through the gauntlet grind, both Victory Road puzzles and the Elite Four. That takes **~26 min**
/// against this one's seven, most of it the grind's ~840 wild battles, so it lives behind its own
/// `hall-of-fame` feature and this stays the gate you run before pushing.
///
/// ⚠️ **This doc comment has lied before, so keep it honest.** The test sat broken for a long time
/// while this very paragraph claimed it reached the Hall of Fame. It does not, on purpose, and the
/// assertion at the bottom is what says where it does stop — not this.
#[test]
#[cfg_attr(not(feature = "full-playthrough"), ignore = "full playthrough; run with --features full-playthrough")]
fn full_playthrough() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(800),
        PolicyStep::eight_badge_steps(),
    );

    {
        let state = fixture.game_state();
        assert_eq!(state.map.map, Map::RedsHouse2F, "save state should be in RedsHouse2F");
        assert_eq!(state.pokemon.len(), 0, "player should have no pokemon before Oak's script");
    }

    // ⚠️ **`step_until_exhausted`, never `run_leg`.** The queue emptying is the *only* thing this
    // test is allowed to accept as "the run finished". `run_leg` would keep stepping afterwards until
    // the assertions happened to come true, which turns "the step list plays the game" into "the step
    // list plus whatever the agent does on its own eventually gets there" — and that is precisely the
    // hole the Poké Flute fell through for a long time (see `TestFixture::run_leg`). Everything below
    // has to be true the instant the last step pops.
    fixture.step_until_exhausted();
    let state = fixture.game_state();

    for pokemon in state.pokemon.iter() {
        println!("{}: {} lv.{}", pokemon.species, pokemon.nickname, pokemon.level);
    }
    println!("badges: {:?}", state.badges);
    println!("map: {:?}", state.map.map);
    println!("money: {}  bag: {:?}", state.money, state.bag.iter().collect::<Vec<_>>());

    assert!(state.badges.contains(Badge::BoulderBadge), "should have the Boulder Badge");
    assert!(state.badges.contains(Badge::CascadeBadge), "should have the Cascade Badge");
    assert!(state.badges.contains(Badge::ThunderBadge), "should have the Thunder Badge");
    assert!(state.badges.contains(Badge::RainbowBadge), "should have the Rainbow Badge");
    // Post-Rainbow: Silph Scope (Rocket Hideout) → Poké Flute → Snorlax → Soul Badge (Koga).
    assert!(state.bag.contains(&ItemId::SilphScope), "should have the Silph Scope");
    assert!(state.bag.contains(&ItemId::PokeFlute), "should have the Poké Flute");
    assert!(state.badges.contains(Badge::SoulBadge), "should have the Soul Badge");
    // Post-Soul: Safari HMs → Vaporeon → Silph (Marsh) → Cinnabar Mansion → Volcano → Viridian (Earth).
    assert!(state.bag.contains(&ItemId::Hm03Surf), "should have HM03 Surf");
    assert!(state.badges.contains(Badge::MarshBadge), "should have the Marsh Badge");
    assert!(state.badges.contains(Badge::VolcanoBadge), "should have the Volcano Badge");
    assert!(state.badges.contains(Badge::EarthBadge), "should have the Earth Badge (all 8 gym badges)");

    // ⚠️ **One fighter and two HM slaves, and the slaves are the *only* other members.** Measured at
    // the end of a clean run: Blastoise 59, Gloom 21 (the Oddish evolves on the way), Machop 24, all
    // eight badges and **no black-outs at all**. Anything else in the party is a regression — the
    // whole point of `gauntlet_grind_steps` is that one mon taken further is cheaper than three.
    assert_eq!(state.pokemon.len(), 3, "party should be the starter + the two HM slaves");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Blastoise),
        "the starter should have reached Blastoise");
    assert!(state.pokemon.iter().any(|p| matches!(p.species,
        PokemonSpecies::Oddish | PokemonSpecies::Gloom)), "should have caught the Route 25 Cut carrier");
    assert!(state.pokemon.iter().any(|p| p.species == PokemonSpecies::Machop),
        "should have caught the Victory Road Strength slave");

    // ⚠️ **Every HM this route needs, checked on the *party* rather than the bag**, because a carrier
    // that cannot learn one is exactly the failure the starter swap introduced: Cut lives on the
    // Oddish and Surf, Strength and Dig on Blastoise, and a step aimed at the wrong one waits for
    // ever rather than failing.
    for want in [PokemonMoveName::Cut, PokemonMoveName::Surf, PokemonMoveName::Strength] {
        assert!(state.pokemon.iter().any(|p| p.moves.iter().flatten().any(|m| m.name == want)),
            "a party member should know {want}");
    }
    assert_eq!(state.map.map, Map::VictoryRoad2F, "should have solved VR1F and climbed to Victory Road 2F");

    fixture.save_state_named("src/pokemon/data/post-victory-road-1f.bin").unwrap();
}

/// **The whole game, to the Hall of Fame** — [`PolicyStep::complete_game_steps`], which is what
/// `gb serve --policy deterministic` plays.
///
/// ⚠️ **Its own `hall-of-fame` feature because it is ~26 minutes against `full_playthrough`'s seven,
/// and a gate that long is a gate nobody runs.** Most of the difference is `gauntlet_grind_steps`:
/// ~840 wild battles in the Pokémon Mansion to bring one Blastoise to lv85. Run this when the endgame
/// changes — the grind, either Victory Road puzzle, or the Elite Four — and `full_playthrough` the
/// rest of the time.
///
/// ⚠️ **It has been fifty minutes twice, and both times the cause was the grind rather than the
/// route.** Once because a trainee was switched in rather than *leading*, which halves the payout and
/// costs the turn; and once because it was grinding three Pokémon to lv75 (1.4 M experience) instead
/// of one to lv85 (425 k). The whole argument is on `PolicyStep::gauntlet_grind_steps`.
///
/// ⚠️ **`run_until`, not `step_until_exhausted`, and this is the one test allowed that.** The final
/// step is the rival in the Champion's room, and beating him hands the agent to
/// `drive_post_champion_cutscene`, which stops polling the policy — so the queue never empties and
/// waiting on it would hang. The exception is kept honest by asserting afterwards that all but the
/// **last two** steps popped, so "reached the Hall of Fame" cannot be satisfied by the agent
/// wandering into the credits on its own.
///
/// ⚠️ **Two rather than one, and the difference is a frame-timing race rather than a step that did
/// not happen.** The bound was `<= 1` — the rival's own `BattleTrainer` — and that only held while
/// the agent happened to get one overworld poll between arriving in the Champion's room and the
/// rival challenging. He challenges *on entry*, as a script, so there is nothing to guarantee that
/// poll, and the `enter(ChampionsRoom)` in front of him pops on the tick after the map changes or
/// not at all. A run that took the room faster stopped getting it: the log shows Gary beaten, the
/// Champion's script played and the Hall of Fame reached with both steps still queued. It is the
/// same mechanism the paragraph above describes for the last step, one step earlier. Out of 516 the
/// guard is unchanged in what it is for.
#[test]
#[cfg_attr(not(feature = "hall-of-fame"), ignore = "~26 min — run with --features hall-of-fame")]
fn hall_of_fame_playthrough() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(6000),
        PolicyStep::complete_game_steps(),
    );

    let state = fixture.run_until(|state| state.map.map == Map::HallOfFame);
    let left = fixture.agent.policy_steps_remaining().expect("the scripted policy counts its queue");
    assert!(
        left <= 2,
        "{left} steps never ran — the run reached the Hall of Fame without playing the route",
    );

    for pokemon in state.pokemon.iter() {
        println!("{}: {} lv.{}", pokemon.species, pokemon.nickname, pokemon.level);
    }
    assert!(state.badges.contains(Badge::EarthBadge), "all eight badges");
    // ⚠️ **One fighter over the target, and it replaced "three fighters or you lose".** That rule
    // was true of three mons at *seventy-five*, with a lv26, a lv30 and a lv24 behind them; height
    // turned out to be the answer rather than depth of bench. See
    // `PolicyStep::gauntlet_grind_steps`.
    for species in [PokemonSpecies::Blastoise] {
        let mon = state.pokemon.iter().find(|p| p.species == species)
            .unwrap_or_else(|| panic!("the party should carry a {species:?}"));
        assert!(mon.level >= PolicyStep::GAUNTLET_LEVEL,
            "{species:?} is lv{} — the gauntlet grind did not run", mon.level);
    }
}

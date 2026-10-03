//! The tour's tests: each phase from its fixture, the whole tour from the fresh save on the
//! cartridge and on the recreation, and the tour replayed action for action. The tour itself is
//! [`crate::tour`].

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::tour::brain::{FinishFlag, Step, TourProgress, print_unresolved};
use crate::tour::{Ceremony, hold_stock, hold_stock_native};
use crate::tour::cheats::Cheats;
use crate::tour::completion::{checklist, Entry, Ledger, Legend, Way, WorldFlags};
use crate::pokemon::item::ItemId;
use crate::tour::phases::*;
use crate::pokemon::integration_tests::llm_harness::{LlmRun, NativeLlmRun};

/// What one phase leaves behind.
pub struct Played {
    pub run: LlmRun,
    pub ledger: Arc<Mutex<Ledger>>,
}

/// Play `steps` from `fixture`, feeding the ledger every tick, and fail with the stuck report.
pub fn play(fixture: &'static [u8], name: &'static str, steps: Vec<Step>, game_minutes: u64, wall: Duration) -> Played {
    play_phases(fixture, name, vec![steps], game_minutes, wall)
}

/// [`play`] for several phases back to back in one run, each with a brain of its own as it has
/// when played from its fixture, and one ledger across them all.
pub fn play_phases(fixture: &'static [u8], name: &'static str, phases: Vec<Vec<Step>>, game_minutes: u64, wall: Duration) -> Played {
    play_phases_with(fixture, name, phases, game_minutes, wall, None)
}

/// [`play_phases`], writing every overworld action into `recording` if there is one.
pub fn play_phases_with(fixture: &'static [u8], name: &'static str, phases: Vec<Vec<Step>>, game_minutes: u64,
                        wall: Duration, recording: Option<Arc<Mutex<crate::lockstep::action_for_action::Log>>>) -> Played {
    let (brain, progress) = FinishFlag::new(phases);
    let TourProgress { ledger, stuck, turns, finished: done, unresolved, total, .. } = progress;
    let mut builder = LlmRun::builder(fixture)
        .named(name)
        .game_time(Duration::from_mins(game_minutes))
        .options(crate::pokemon::options::SERVED_OPTIONS)
        .with_coverage();
    if let Some(log) = recording {
        builder = builder.recording(log);
    }
    let mut run = builder.start(Box::new(brain));
    run.with_cheats(Cheats::story(999_999));
    {
        let list = checklist(run.fixture().gb.core().mmu());
        *ledger.lock().expect("not poisoned") = Ledger::new(&list);
    }

    let started = std::time::Instant::now();
    let mut ceremony = Ceremony::default();
    run.tick_until(wall, |run| {
        if !ceremony.is_over() {
            let map = run.map_if_readable();
            ceremony.press(&mut run.fixture().api(), map);
        }
        if let Ok(state) = run.fixture().try_game_state() {
            ledger.lock().expect("not poisoned").observe(&state, run.fixture().gb.core().mmu());
            hold_stock(&mut run.fixture().api(), &state);
        }
        *done.lock().expect("not poisoned") || stuck.lock().expect("not poisoned").is_some()
    });
    let turns = *turns.lock().expect("not poisoned");
    println!("[completion:{name}] {total} steps, {turns} turns, {:?} of game time in {:?}",
             run.fixture().total_cycles.to_duration(), started.elapsed());
    print_unresolved(name, &unresolved);
    if let Some(why) = stuck.lock().expect("not poisoned").clone() {
        // Where it stuck, to load and look at.
        let at = std::env::temp_dir().join(format!("{name}-stuck.bin"));
        run.fixture().gb.save_state_to_file(at.to_str().expect("a UTF-8 path")).ok();
        panic!("[completion:{name}] stuck (saved to {}): {why}", at.display());
    }
    assert!(*done.lock().expect("not poisoned"), "[completion:{name}] ran out of wall clock");
    let log = run.coverage().expect("coverage was asked for");
    let defects: Vec<String> = log.entries()
        .filter(|entry| matches!(entry.verdict, crate::pokemon::integration_tests::coverage::Verdict::Defect { .. }))
        .map(|entry| entry.id.clone())
        .collect();
    assert!(defects.is_empty(), "[completion:{name}] the agent could not carry out: {defects:?}");
    Played { run, ledger }
}

/// What one phase on the recreation leaves behind.
pub struct NativePlayed {
    pub run: NativeLlmRun,
    pub ledger: Arc<Mutex<Ledger>>,
}

impl NativePlayed {
    /// Every entry of the checklist the run has not done.
    pub fn missing(&mut self) -> (Vec<crate::tour::completion::Item>, Vec<Entry>) {
        let agent = self.run.agent();
        let state = agent.game_state().expect("a game state");
        let list = checklist(agent.native().rom());
        let flags = WorldFlags { world: agent.game().world(), warped_from: (0, 0) };
        let missing = self.ledger.lock().expect("not poisoned").missing(&list, &flags, &state);
        (list, missing)
    }
}

/// Where a native phase that stuck is saved, to load and look at.
fn native_stuck_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("target/test-artifacts/{name}-stuck.pkrd"))
}

/// [`play_phases`] on the recreation: a new game from Red's room played by the same brain through
/// `LlmPolicy` and the native agent, with the cheats written into the world and the ledger read
/// from it.
pub fn play_phases_native(seed: u64, name: &'static str, phases: Vec<Vec<Step>>, game_minutes: u64, wall: Duration) -> NativePlayed {
    play_phases_native_on(seed, name, phases, game_minutes, wall, false)
}

/// [`play_phases_native`], the brain answering through the mock server or, `in_process`, behind a
/// [`BrainEndpoint`](crate::tour::endpoint::BrainEndpoint).
fn play_phases_native_on(seed: u64, name: &'static str, phases: Vec<Vec<Step>>, game_minutes: u64, wall: Duration,
                         in_process: bool) -> NativePlayed {
    let (brain, progress) = FinishFlag::new(phases);
    let TourProgress { ledger, stuck, turns, finished: done, unresolved, total, .. } = progress;
    // At a served game's pace: a text with no wait on it is on screen only for the frames it is
    // printed in, which instant pacing makes none.
    let game = crate::pokemon::integration_tests::playthrough::native_new_game(seed, true, pokered::Pacing::Faithful);
    let builder = LlmRun::builder(&[]).named(name).game_time(Duration::from_mins(game_minutes));
    let mut run = if in_process {
        builder.start_native_in_process(game, Box::new(brain))
    } else {
        builder.start_native(game, Box::new(brain))
    };
    *ledger.lock().expect("not poisoned") = Ledger::new(&checklist(run.agent().native().rom()));

    let mut cheats = Cheats::story(999_999);
    let mut warped_from = (0, 0);
    let started = std::time::Instant::now();
    let ticked = run.tick_until(wall, |run| {
        let agent = run.agent();
        warped_from = agent.native().warped_from().unwrap_or(warped_from);
        {
            let world = agent.game().world();
            let flags = WorldFlags { world, warped_from };
            ledger.lock().expect("not poisoned")
                .observe_on(world.location.map, world.bag.items.iter().map(|item| item.id as u8), &flags);
        }
        // Between decisions, as the emulated sidecar writes between ticks.
        if agent.is_free() && agent.game().frames() % 8 == 0 && let Ok(state) = agent.game_state() {
            let world = agent.game_mut().world_mut();
            cheats.apply_native(world, &state, true);
            hold_stock_native(world);
        }
        *done.lock().expect("not poisoned") || stuck.lock().expect("not poisoned").is_some()
    });
    let turns = *turns.lock().expect("not poisoned");
    let frames = run.agent().game().frames();
    println!("[completion:{name}] {total} steps, {turns} turns, {} of game time in {:?}",
             Duration::from_secs(frames / 60).as_secs(), started.elapsed());
    print_unresolved(name, &unresolved);
    let why = match ticked {
        Err(why) => Some(why),
        Ok(_) => stuck.lock().expect("not poisoned").clone(),
    };
    if let Some(why) = why {
        let at = native_stuck_path(name);
        std::fs::create_dir_all(at.parent().expect("a directory")).ok();
        std::fs::write(&at, run.agent().game().save()).ok();
        let location = &run.agent().game().world().location;
        panic!("[completion:{name}] stuck on {:?} at ({}, {}) (saved to {}): {why}",
               location.map, location.x, location.y, at.display());
    }
    assert!(*done.lock().expect("not poisoned"), "[completion:{name}] ran out of wall clock");
    NativePlayed { run, ledger }
}
/// Every entry on `maps`, and every one of `also`, the ledger has not ticked off: a phase's own
/// assertion.
pub fn missing_on(played: &mut Played, maps: &[crate::pokemon::map::Map], also: &[Entry]) -> Vec<Entry> {
    let state = played.run.fixture().game_state();
    let list = checklist(played.run.fixture().gb.core().mmu());
    let ledger = played.ledger.lock().expect("not poisoned");
    let mmu = played.run.fixture().gb.core().mmu();
    let missing = ledger.missing(&list, mmu, &state);

    // Every door and map edge the phase stood beside and did not cross, printed rather than
    // asserted: a door here is often the next phase's to cross, and only the whole run can say one
    // went unwalked.
    let stood_on: HashSet<crate::pokemon::map::Map> = missing.iter()
        .filter_map(|entry| match entry { Entry::Map(map) => Some(*map), _ => None }).collect();
    for entry in &missing {
        match entry {
            Entry::Warp { map, .. } | Entry::Connection { map, .. } if !stood_on.contains(map) =>
                println!("[phase] {entry:?} uncrossed"),
            _ => {}
        }
    }

    missing.into_iter()
        .filter(|entry| also.contains(entry) || match entry {
            Entry::Map(map) | Entry::Trainer { map, .. } | Entry::ItemBall { map, .. } => maps.contains(map),
            _ => false,
        })
        .collect()
}

/// Save where a phase ended, for the next phase to start from, under `GB_REGEN_FIXTURES=1`.
pub fn cut(played: &mut Played, name: &str) {
    if crate::pokemon::integration_tests::fixture::regenerating_fixtures() {
        played.run.fixture().save_state_named(&format!("src/pokemon/data/{name}.bin")).expect("the phase's end state saves");
    }
}
/// The first phase, from the fresh save.
#[test]
fn completion_phase_boulder_badge() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/start-of-game-state.bin"), "completion-boulder",
                          to_the_boulder_badge(), 240, Duration::from_secs(1800));
    let missing = missing_on(&mut played, &[
        Map::RedsHouse1F, Map::PalletTown, Map::BluesHouse, Map::OaksLab, Map::Route1,
        Map::ViridianCity, Map::ViridianMart, Map::ViridianPokecenter, Map::ViridianSchoolHouse,
        Map::ViridianNicknameHouse, Map::Route22, Map::ViridianForestSouthGate, Map::ViridianForest,
        Map::ViridianForestNorthGate, Map::PewterCity, Map::PewterGym, Map::PewterMart,
        Map::PewterPokecenter, Map::Museum1F, Map::Museum2F, Map::PewterNidoranHouse,
        Map::PewterSpeechHouse,
    ], &[Entry::Badge(0), Entry::Way(Way::GiftStarter), Entry::Way(Way::WildInGrass),
         Entry::Way(Way::EvolvedInBattle), Entry::Way(Way::NicknameAfterAGift)]);
    cut(&mut played, "completion-boulder");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

/// [`completion_phase_boulder_badge`] on the recreation, from a new game.
#[test]
fn native_completion_phase_boulder_badge() {
    native_boulder_badge(false);
}

/// [`native_completion_phase_boulder_badge`] with the brain answering in process, as a host with no
/// server plays the tour.
#[test]
fn native_completion_phase_boulder_badge_in_process() {
    native_boulder_badge(true);
}

fn native_boulder_badge(in_process: bool) {
    use crate::pokemon::map::Map;
    let mut played = play_phases_native_on(1, "native-completion-boulder", vec![to_the_boulder_badge()], 240,
                                           Duration::from_secs(1800), in_process);
    let (_, missing) = played.missing();
    let maps = [
        Map::RedsHouse1F, Map::PalletTown, Map::BluesHouse, Map::OaksLab, Map::Route1,
        Map::ViridianCity, Map::ViridianMart, Map::ViridianPokecenter, Map::ViridianSchoolHouse,
        Map::ViridianNicknameHouse, Map::Route22, Map::ViridianForestSouthGate, Map::ViridianForest,
        Map::ViridianForestNorthGate, Map::PewterCity, Map::PewterGym, Map::PewterMart,
        Map::PewterPokecenter, Map::Museum1F, Map::Museum2F, Map::PewterNidoranHouse,
        Map::PewterSpeechHouse,
    ];
    let also = [Entry::Badge(0), Entry::Way(Way::GiftStarter), Entry::Way(Way::WildInGrass),
                Entry::Way(Way::EvolvedInBattle), Entry::Way(Way::NicknameAfterAGift)];
    let missing: Vec<Entry> = missing.into_iter().filter(|entry| also.contains(entry) || match entry {
        Entry::Map(map) | Entry::Trainer { map, .. } | Entry::ItemBall { map, .. } => maps.contains(map),
        _ => false,
    }).collect();
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_bill() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-boulder.bin"), "completion-bill",
                          to_bill(), 240, Duration::from_secs(1800));
    let missing = missing_on(&mut played, &[
        Map::Route3, Map::MtMoonPokecenter, Map::MtMoon1F, Map::MtMoonB1F, Map::MtMoonB2F,
        Map::CeruleanPokecenter, Map::CeruleanMart, Map::CeruleanBadgeHouse, Map::CeruleanTradeHouse,
        Map::BikeShop, Map::CeruleanGym, Map::Route24, Map::Route25, Map::BillsHouse,
        Map::CeruleanTrashedHouse,
    ], &[Entry::Badge(1), Entry::Way(Way::BoughtMagikarp), Entry::KeyItem(vec![ItemId::SSTicket as u8])]);
    cut(&mut played, "completion-bill");
    // Behind a tree on Route 25, for the phase after Cut.
    let later = [Entry::ItemBall { map: Map::Route25, object: 10, item: ItemId::Tm19SeismicToss as u8 }];
    let missing: Vec<Entry> = missing.into_iter().filter(|entry| !later.contains(entry)).collect();
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_thunder_badge() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-bill.bin"), "completion-thunder",
                          to_the_thunder_badge(), 300, Duration::from_secs(2400));
    let missing = missing_on(&mut played, &[
        Map::Route5, Map::UndergroundPathRoute5, Map::UndergroundPathNorthSouth,
        Map::UndergroundPathRoute6, Map::Route6, Map::VermilionCity, Map::VermilionPokecenter,
        Map::VermilionMart, Map::PokemonFanClub, Map::VermilionOldRodHouse, Map::VermilionPidgeyHouse,
        Map::VermilionTradeHouse, Map::VermilionDock, Map::SSAnne1F, Map::SSAnne2F, Map::SSAnne3F,
        Map::SSAnneB1F, Map::SSAnneBow, Map::SSAnneKitchen, Map::SSAnneCaptainsRoom, Map::SSAnne1FRooms,
        Map::SSAnne2FRooms, Map::SSAnneB1FRooms, Map::VermilionGym,
    ], &[Entry::Badge(2), Entry::Way(Way::OldRod), Entry::Machine(ItemId::Hm01Cut as u8),
         Entry::Machine(ItemId::Tm24Thunderbolt as u8)]);
    cut(&mut played, "completion-thunder");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_celadon() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-thunder.bin"), "completion-celadon",
                          to_celadon(), 360, Duration::from_secs(2400));
    let missing = missing_on(&mut played, &[
        Map::DiglettsCaveRoute11, Map::DiglettsCave, Map::DiglettsCaveRoute2, Map::Route2, Map::Route2TradeHouse,
        Map::Route2Gate, Map::Daycare, Map::Route25, Map::Route9, Map::Route10, Map::RockTunnelPokecenter,
        Map::RockTunnel1F, Map::RockTunnelB1F, Map::LavenderTown, Map::LavenderPokecenter, Map::LavenderMart,
        Map::LavenderCuboneHouse, Map::MrFujisHouse, Map::NameRatersHouse, Map::PokemonTower1F,
        Map::PokemonTower2F, Map::Route12Gate1F, Map::Route12Gate2F, Map::Route8, Map::Route8Gate,
        Map::UndergroundPathRoute8, Map::UndergroundPathWestEast, Map::UndergroundPathRoute7, Map::Route7,
        Map::Route7Gate, Map::BikeShop,
    ], &[Entry::Way(Way::WildOnACaveFloor), Entry::KeyItem(vec![ItemId::Bicycle as u8])]);
    cut(&mut played, "completion-celadon");
    // On the water by the Power Plant, for the phase after Surf.
    let later = [Entry::Trainer { map: Map::Route10, index: 0 }];
    let missing: Vec<Entry> = missing.into_iter().filter(|entry| !later.contains(entry)).collect();
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_rainbow_badge() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-celadon.bin"), "completion-rainbow",
                          to_the_rainbow_badge(), 360, Duration::from_secs(2400));
    let missing = missing_on(&mut played, &[
        Map::CeladonCity, Map::CeladonPokecenter, Map::CeladonMart1F, Map::CeladonMart2F, Map::CeladonMart3F,
        Map::CeladonMart4F, Map::CeladonMart5F, Map::CeladonMartRoof, Map::CeladonMartElevator,
        Map::CeladonMansion1F, Map::CeladonMansion2F, Map::CeladonMansion3F, Map::CeladonMansionRoof,
        Map::CeladonMansionRoofHouse, Map::CeladonDiner, Map::CeladonHotel, Map::CeladonChiefHouse,
        Map::GameCorner, Map::GameCornerPrizeRoom, Map::CeladonGym, Map::Route16FlyHouse, Map::Route16Gate1F,
    ], &[Entry::Badge(3), Entry::Way(Way::GiftEevee), Entry::Way(Way::EvolvedByStone),
         Entry::Way(Way::GameCornerPrize), Entry::Way(Way::PcChangeBox),
         Entry::Machine(ItemId::Hm02Fly as u8), Entry::Machine(ItemId::Tm13IceBeam as u8),
         Entry::Machine(ItemId::Tm48RockSlide as u8), Entry::Machine(ItemId::Tm49TriAttack as u8),
         Entry::Machine(ItemId::Tm15HyperBeam as u8), Entry::Machine(ItemId::Tm23DragonRage as u8),
         Entry::Machine(ItemId::Tm50Substitute as u8)]);
    cut(&mut played, "completion-rainbow");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_poke_flute() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-rainbow.bin"), "completion-flute",
                          to_the_poke_flute(), 420, Duration::from_secs(2400));
    let missing = missing_on(&mut played, &[
        Map::RocketHideoutB1F, Map::RocketHideoutB2F, Map::RocketHideoutB3F, Map::RocketHideoutB4F,
        Map::RocketHideoutElevator, Map::PokemonTower1F, Map::PokemonTower2F, Map::PokemonTower3F,
        Map::PokemonTower4F, Map::PokemonTower5F, Map::PokemonTower6F, Map::PokemonTower7F,
        Map::Route12, Map::Route12SuperRodHouse, Map::Route16, Map::Route16Gate2F,
    ], &[Entry::Way(Way::SnorlaxOnRoute12), Entry::Way(Way::SnorlaxOnRoute16),
         Entry::KeyItem(vec![ItemId::SilphScope as u8]), Entry::KeyItem(vec![ItemId::PokeFlute as u8]),
         Entry::KeyItem(vec![ItemId::LiftKey as u8]), Entry::KeyItem(vec![ItemId::SuperRod as u8])]);
    cut(&mut played, "completion-flute");
    // South of the gate on Route 12, on the stretch the Fuchsia phase walks.
    let later = [Entry::ItemBall { map: Map::Route12, object: 9, item: ItemId::Tm16PayDay as u8 }];
    let missing: Vec<Entry> = missing.into_iter().filter(|entry| !later.contains(entry)).collect();
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_marsh_badge() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-flute.bin"), "completion-marsh",
                          to_the_marsh_badge(), 420, Duration::from_secs(3000));
    let missing = missing_on(&mut played, &[
        Map::SaffronCity, Map::SaffronPokecenter, Map::SaffronMart, Map::SaffronPidgeyHouse,
        Map::MrPsychicsHouse, Map::SaffronGym, Map::FightingDojo, Map::CopycatsHouse1F,
        Map::CopycatsHouse2F, Map::SilphCo1F, Map::SilphCo2F, Map::SilphCo3F, Map::SilphCo4F,
        Map::SilphCo5F, Map::SilphCo6F, Map::SilphCo7F, Map::SilphCo8F, Map::SilphCo9F,
        Map::SilphCo10F, Map::SilphCo11F, Map::SilphCoElevator,
    ], &[Entry::Badge(5), Entry::Way(Way::GiftLapras), Entry::Way(Way::GiftFightingDojo),
         Entry::KeyItem(vec![ItemId::CardKey as u8])]);
    cut(&mut played, "completion-marsh");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_soul_badge() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-marsh.bin"), "completion-soul",
                          to_the_soul_badge(), 300, Duration::from_secs(2400));
    let missing = missing_on(&mut played, &[
        Map::Route17, Map::Route18, Map::Route18Gate1F, Map::Route18Gate2F, Map::FuchsiaCity,
        Map::FuchsiaPokecenter, Map::FuchsiaMart, Map::FuchsiaBillsGrandpasHouse,
        Map::FuchsiaGoodRodHouse, Map::WardensHouse, Map::FuchsiaMeetingRoom, Map::FuchsiaGym,
    ], &[Entry::Badge(4), Entry::KeyItem(vec![ItemId::GoodRod as u8])]);
    cut(&mut played, "completion-soul");
    // The warden's own room is walked again when the teeth are brought to him.
    let later = [Entry::ItemBall { map: Map::WardensHouse, object: 2, item: ItemId::RareCandy as u8 }];
    let missing: Vec<Entry> = missing.into_iter().filter(|entry| !later.contains(entry)).collect();
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

/// The smallest thing that reproduces the Safari turnstile, from the state it happened in: a walk
/// that is given up on every time it is tried, beside a door that puts the player out of the area.
/// The gate's workers stand behind the prompt that asks whether you are leaving, so approaching one
/// answers it, ends the visit, and the run pays to come back. A target taken back for ever is one
/// the exploring returns to for ever. The money the gate charges is held up by the cheats, so what
/// this watches is the clock, which a loop spends and a finished walk does not.
#[test]
fn a_walk_given_up_on_every_time_is_not_tried_for_ever() {
    play(include_bytes!("../data/completion-soul.bin"), "safari-turnstile", vec![
        Step::Collect(false),
        Step::GoTo("SafariZoneGate"),
        Step::Explore { maps: &["SafariZoneGate", "SafariZoneCenter"], patience: 300 },
    ], 45, Duration::from_secs(900));
}

#[test]
fn completion_phase_surf() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-soul.bin"), "completion-surf",
                          to_surf(), 1200, Duration::from_secs(5400));
    let out_of_reach = out_of_reach();
    let missing = missing_on(&mut played, &[
        Map::SafariZoneGate, Map::SafariZoneCenter, Map::SafariZoneEast, Map::SafariZoneNorth,
        Map::SafariZoneWest, Map::SafariZoneCenterRestHouse, Map::SafariZoneEastRestHouse,
        Map::SafariZoneNorthRestHouse, Map::SafariZoneWestRestHouse, Map::SafariZoneSecretHouse,
        Map::WardensHouse,
    ], &[Entry::Machine(ItemId::Hm03Surf as u8), Entry::KeyItem(vec![ItemId::GoldTeeth as u8])])
        .into_iter().filter(|entry| !out_of_reach.contains(entry)).collect::<Vec<_>>();
    cut(&mut played, "completion-surf");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

/// Probe: stand on the open water of a sea route and report what the model is offered there, to
/// answer why every swimmer on Routes 19, 20 and 21 goes unfought. The last step names a row that
/// cannot exist, so the run reports itself stuck, and `play` both prints that turn and saves the
/// state for `probe_stall_actions`. It fails by design; what it is for is the turn it prints.
#[test]
#[ignore = "probe — run with --ignored --nocapture, see the doc comment"]
fn probe_sea_route_menu() {
    play(include_bytes!("../data/completion-surf.bin"), "probe-sea-route", vec![
        Step::Collect(false),
        Step::GoTo("Route19"),
        Step::Explore { maps: &["Route19"], patience: 300 },
        Step::GoTo("Route20"),
        Step::Talk("NoSuchRowEver"),
    ], 300, Duration::from_secs(1200));
}

#[test]
fn completion_phase_volcano_badge() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-surf.bin"), "completion-volcano",
                          to_the_volcano_badge(), 900, Duration::from_secs(4500));
    let missing = missing_on(&mut played, &[
        Map::Route19, Map::Route20, Map::Route21, Map::CinnabarIsland, Map::CinnabarPokecenter,
        Map::CinnabarMart,
        Map::CinnabarLab, Map::CinnabarLabTradeRoom, Map::CinnabarLabMetronomeRoom,
        Map::CinnabarLabFossilRoom, Map::PokemonMansion1F, Map::PokemonMansion2F,
        Map::PokemonMansion3F, Map::PokemonMansionB1F, Map::CinnabarGym,
    ], &[Entry::Badge(6), Entry::CinnabarQuiz, Entry::Way(Way::RevivedFossil),
         Entry::Machine(ItemId::Tm35Metronome as u8),
         Entry::KeyItem(vec![ItemId::SecretKey as u8])]);
    cut(&mut played, "completion-volcano");
    // 3F's scientist stands behind a block only the switch on opens, and this phase leaves the
    // switch off; the Cinnabar errands phase comes back for him.
    let later = [Entry::Trainer { map: Map::PokemonMansion3F, index: 1 }];
    let missing: Vec<Entry> = missing.into_iter().filter(|entry| !later.contains(entry)).collect();
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_seafoam() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-volcano.bin"), "completion-seafoam",
                          to_seafoam(), 600, Duration::from_secs(3000));
    let missing = missing_on(&mut played, &[
        Map::SeafoamIslands1F, Map::SeafoamIslandsB1F, Map::SeafoamIslandsB2F,
        Map::SeafoamIslandsB3F, Map::SeafoamIslandsB4F,
    ], &[Entry::Way(Way::Legendary(Legend::Articuno))]);
    cut(&mut played, "completion-seafoam");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_earth_badge() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-seafoam.bin"), "completion-earth",
                          to_the_earth_badge(), 300, Duration::from_secs(1800));
    // Giovanni hands over his machine once he is beaten, and the bag has room for it here.
    let missing = missing_on(&mut played, &[Map::ViridianGym],
                             &[Entry::Badge(7), Entry::Machine(ItemId::Tm27Fissure as u8)]);
    cut(&mut played, "completion-earth");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_power_plant() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-mewtwo.bin"), "completion-power-plant",
                          to_the_power_plant(), 300, Duration::from_secs(1800));
    let missing = missing_on(&mut played, &[Map::PowerPlant],
        &[Entry::Way(Way::PowerPlantBall), Entry::Way(Way::Legendary(Legend::Zapdos))]);
    cut(&mut played, "completion-power-plant");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_victory_road() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-earth.bin"), "completion-victory-road",
                          to_victory_road(), 600, Duration::from_secs(3000));
    let missing = missing_on(&mut played, &[
        Map::Route22, Map::Route22Gate, Map::Route23, Map::VictoryRoad1F, Map::VictoryRoad2F,
        Map::VictoryRoad3F, Map::IndigoPlateau, Map::IndigoPlateauLobby,
    ], &[Entry::Way(Way::Legendary(Legend::Moltres)), Entry::Way(Way::PcChangeBox)]);
    cut(&mut played, "completion-victory-road");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_hall_of_fame() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-victory-road.bin"), "completion-hall-of-fame",
                          to_the_hall_of_fame(), 300, Duration::from_secs(1800));
    let missing = missing_on(&mut played, &[
        Map::IndigoPlateauLobby, Map::LoreleisRoom, Map::BrunosRoom, Map::AgathasRoom,
        Map::LancesRoom, Map::ChampionsRoom, Map::HallOfFame,
    ], &[Entry::HallOfFame]);
    cut(&mut played, "completion-hall-of-fame");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_mewtwo() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-north.bin"), "completion-mewtwo",
                          to_mewtwo(), 600, Duration::from_secs(3000));
    let missing = missing_on(&mut played, &[
        Map::CeruleanCave1F, Map::CeruleanCave2F, Map::CeruleanCaveB1F,
    ], &[Entry::Way(Way::Legendary(Legend::Mewtwo)), Entry::Way(Way::WildOnACaveFloor)]);
    cut(&mut played, "completion-mewtwo");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}



#[test]
fn completion_phase_eastern_routes() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-power-plant.bin"), "completion-east",
                          to_the_eastern_routes(), 900, Duration::from_secs(3600));
    let missing = missing_on(&mut played, &[
        Map::Route13, Map::Route14, Map::Route15, Map::Route15Gate1F, Map::Route15Gate2F,
    ], &[Entry::ItemBall { map: Map::Route12, object: 9, item: ItemId::Tm16PayDay as u8 }]);
    cut(&mut played, "completion-east");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_safari_game() {
    let mut played = play(include_bytes!("../data/completion-east.bin"), "completion-safari",
                          to_the_safari_game(), 600, Duration::from_secs(3000));
    let missing = missing_on(&mut played, &[], &[
        Entry::Way(Way::SafariCatch), Entry::Way(Way::SafariBait), Entry::Way(Way::SafariRock),
        Entry::Way(Way::SafariRun), Entry::Way(Way::SafariOutOfSteps), Entry::Way(Way::GoodRod),
        Entry::Way(Way::WildWhileSurfing),
    ]);
    cut(&mut played, "completion-safari");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_north_errands() {
    use crate::pokemon::map::Map;
    let mut played = play(include_bytes!("../data/completion-hall-of-fame.bin"), "completion-north",
                          to_the_north_errands(), 600, Duration::from_secs(3000));
    let missing = missing_on(&mut played, &[Map::Route5Gate], &[
        Entry::Trainer { map: Map::Route4, index: 0 },
        Entry::Machine(ItemId::Tm42DreamEater as u8),
        Entry::Machine(ItemId::Hm05Flash as u8), Entry::KeyItem(vec![ItemId::OldAmber as u8]),
        Entry::Trade(crate::pokemon::species::PokemonSpecies::Poliwhirl),
        Entry::Way(Way::DayCareWithdrawn), Entry::Way(Way::SuperRod),
        Entry::ItemBall { map: Map::Route25, object: 10, item: ItemId::Tm19SeismicToss as u8 },
        Entry::Trainer { map: Map::Route10, index: 0 },
    ]);
    cut(&mut played, "completion-north");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_middle_errands() {
    use crate::pokemon::map::Map;
    use crate::pokemon::species::PokemonSpecies;
    let mut played = play(include_bytes!("../data/completion-safari.bin"), "completion-middle",
                          to_the_middle_errands(), 600, Duration::from_secs(3000));
    let missing = missing_on(&mut played, &[Map::Route6Gate, Map::Route11Gate1F, Map::Route11Gate2F], &[
        Entry::Machine(ItemId::Tm41Softboiled as u8),
        Entry::Machine(ItemId::Tm31Mimic as u8), Entry::KeyItem(vec![ItemId::Itemfinder as u8]),
        Entry::KeyItem(vec![ItemId::ExpAll as u8]),
        Entry::Trade(PokemonSpecies::Nidorino), Entry::Trade(PokemonSpecies::Slowbro),
        Entry::Trade(PokemonSpecies::NidoranMale), Entry::Way(Way::EvolvedByRareCandy),
    ]);
    cut(&mut played, "completion-middle");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn completion_phase_cinnabar_errands() {
    use crate::pokemon::species::PokemonSpecies;
    let mut played = play(include_bytes!("../data/completion-middle.bin"), "completion-cinnabar-errands",
                          to_the_cinnabar_errands(), 600, Duration::from_secs(3000));
    let missing = missing_on(&mut played, &[], &[
        Entry::Way(Way::RevivedOldAmber),
        Entry::Trade(PokemonSpecies::Ponyta), Entry::Trade(PokemonSpecies::Raichu),
        Entry::Trade(PokemonSpecies::Venonat),
        Entry::Machine(ItemId::Tm38FireBlast as u8),
        Entry::Way(Way::EvolutionCancelled),
        Entry::Trainer { map: crate::pokemon::map::Map::PokemonMansion3F, index: 1 },
    ]);
    cut(&mut played, "completion-cinnabar-errands");
    assert!(missing.is_empty(), "the phase left {missing:?}");
}

#[test]
fn the_tour_script_switches_a_frozen_lead_out() {
    use crate::llm::battle_script::{run, scenarios, Outcome};
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::status::PokemonStatus;
    let mut state = scenarios::hurt_trainer();
    state.pokemon.get_mut(0).expect("a lead").status = PokemonStatus::Frozen;
    state.battle.as_mut().expect("a battle").player.status = PokemonStatus::Frozen;
    let outcome = run(crate::tour::brain::SCRIPT, &state, 2).outcome;
    assert!(matches!(outcome, Outcome::Action(BattleAction::SwitchPokemon { slot: 1, .. })), "got {outcome:?}");
}

/// Before a switch, a move or handing a wild battle back: the status's own cure if the bag has it,
/// else a Full Heal.
#[test]
fn the_tour_script_cures_any_status_first() {
    use crate::llm::battle_script::{run, scenarios, Outcome};
    use crate::pokemon::bag::{Bag, BagItem};
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::status::PokemonStatus;
    let cases = [
        (scenarios::hurt_trainer(), PokemonStatus::Frozen, vec![ItemId::SuperPotion, ItemId::FullHeal], ItemId::FullHeal),
        (scenarios::last_mon(), PokemonStatus::Paralyzed, vec![ItemId::FullHeal, ItemId::ParlyzHeal], ItemId::ParlyzHeal),
        (scenarios::healthy_wild(), PokemonStatus::Burned, vec![ItemId::Potion, ItemId::FullHeal], ItemId::FullHeal),
    ];
    for (mut state, status, bag, want) in cases {
        state.bag = Bag::new(bag.into_iter().map(|id| BagItem::new(id, 3)).collect());
        state.pokemon.get_mut(0).expect("a lead").status = status;
        state.battle.as_mut().expect("a battle").player.status = status;
        let outcome = run(crate::tour::brain::SCRIPT, &state, 2).outcome;
        assert!(matches!(&outcome, Outcome::Action(BattleAction::UseItem { item, .. }) if item.id == want),
                "{status:?}: got {outcome:?}");
    }
}

/// Against a Rattata on 1 HP, Scratch: as sure as Ember, which `best_move` would pick, and surer
/// than the weaker Fire Spin.
#[test]
fn the_tour_script_knocks_out_with_the_surest_weakest_move_that_can() {
    use crate::llm::battle_script::{run, scenarios, Outcome};
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
    let mut state = scenarios::hurt_trainer();
    let battle = state.battle.as_mut().expect("a battle");
    battle.enemy.current_hp = 1;
    battle.player.moves[2] = Some(PokemonMove { name: PokemonMoveName::FireSpin, pp: 15 });
    let outcome = run(crate::tour::brain::SCRIPT, &state, 2).outcome;
    assert!(matches!(&outcome, Outcome::Action(BattleAction::Fight { battle_move, .. }) if battle_move.name == PokemonMoveName::Scratch),
            "got {outcome:?}");

    state.battle.as_mut().expect("a battle").enemy.current_hp = 999;
    let outcome = run(crate::tour::brain::SCRIPT, &state, 2).outcome;
    assert!(matches!(&outcome, Outcome::Action(BattleAction::Fight { battle_move, .. }) if battle_move.name == PokemonMoveName::Ember),
            "nothing knocks it out, so the best move: got {outcome:?}");
}
/// Every phase above, back to back in one run from the fresh save, and the whole ledger asserted:
/// each phase green from its own fixture proves every entry reachable, and only this proves one run
/// reaches them all.
#[test]
fn grand_tour() {
    let mut played = play_phases(include_bytes!("../data/start-of-game-state.bin"), "completion-run",
                                 all_phases(), 10_800, Duration::from_secs(4 * 3600));
    let state = played.run.fixture().game_state();
    let list = checklist(played.run.fixture().gb.core().mmu());
    let mmu = played.run.fixture().gb.core().mmu();
    let missing = played.ledger.lock().expect("not poisoned").missing(&list, mmu, &state);
    assert_the_tour_complete("completion-run", &list, missing);
}

/// [`grand_tour`] played on the cartridge, and every overworld action in it replayed on both halves
/// from where the cartridge stood: `lockstep::action_for_action`. `GB_A4A_PHASES` plays only the
/// first so many phases.
#[test]
fn action_for_action_tour() {
    let log = Arc::new(Mutex::new(crate::lockstep::action_for_action::Log::default()));
    let mut phases = all_phases();
    if let Some(first) = std::env::var("GB_A4A_PHASES").ok().and_then(|n| n.parse().ok()) {
        phases.truncate(first);
    }
    play_phases_with(include_bytes!("../data/start-of-game-state.bin"), "completion-a4a", phases, 10_800,
                     Duration::from_secs(4 * 3600), Some(Arc::clone(&log)));
    let segments = std::mem::take(&mut log.lock().expect("not poisoned").segments);
    let failed = crate::lockstep::action_for_action::replay_all(&segments);
    // Each differing segment's save and answers, for the fixture sweep's `GB_A4A_DIR` to replay alone.
    if let Ok(dir) = std::env::var("GB_A4A_DUMP") {
        for &(at, _) in &failed {
            let path = std::path::Path::new(&dir).join(format!("segment-{at}.bin"));
            std::fs::write(&path, &segments[at].state).expect("the dump directory is writable");
            std::fs::write(path.with_extension("json"), serde_json::to_vec(&segments[at].answers).expect("answers serialise"))
                .expect("the dump directory is writable");
        }
    }
    assert!(failed.is_empty(), "{} of {} actions differ", failed.len(), segments.len());
}

/// [`grand_tour`] on the recreation: a new game, its own RNG, the native agent, the same brain.
#[test]
fn native_grand_tour() {
    let mut played = play_phases_native(1, "native-completion-run", all_phases(), 10_800, Duration::from_secs(3600));
    let (list, missing) = played.missing();
    assert_the_tour_complete("native-completion-run", &list, missing);
}
/// The whole ledger asserted, bar what is out of reach, and counted as well: the count is what says
/// how much of the world a run that goes green actually walks.
fn assert_the_tour_complete(name: &str, list: &[crate::tour::completion::Item], missing: Vec<Entry>) {
    let excused = out_of_reach();
    let missing: Vec<Entry> = missing.into_iter().filter(|entry| !excused.contains(entry)).collect();
    let walked = |kind: fn(&Entry) -> bool| {
        let all = list.iter().filter(|item| kind(&item.entry)).count();
        (all - missing.iter().filter(|entry| kind(entry)).count(), all)
    };
    let (doors, all_doors) = walked(|entry| matches!(entry, Entry::Warp { .. }));
    let (edges, all_edges) = walked(|entry| matches!(entry, Entry::Connection { .. }));
    println!("[{name}] {doors} of {all_doors} doors and {edges} of {all_edges} map edges crossed");

    // Named, not just counted: the route that closes the gap is written from this list.
    let mut uncrossed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in &missing {
        match entry {
            Entry::Warp { map, index } => uncrossed.entry(map.to_string()).or_default().push(index.to_string()),
            Entry::Connection { map, direction } => uncrossed.entry(map.to_string()).or_default().push(format!("{direction:?}")),
            _ => {}
        }
    }
    for (map, mut left) in uncrossed {
        left.sort();
        println!("[{name}] {map} left {}", left.join(", "));
    }

    let elsewhere = |entry: &Entry| !matches!(entry, Entry::Warp { .. } | Entry::Connection { .. });
    let rest = list.len() - all_doors - all_edges;
    let others = missing.iter().filter(|entry| elsewhere(entry)).count();
    println!("[{name}] {} of {rest} entries met",
             rest - others - excused.iter().filter(|entry| elsewhere(entry)).count());
    assert!(missing.is_empty(), "the run left {} entries: {missing:?}", missing.len());
}


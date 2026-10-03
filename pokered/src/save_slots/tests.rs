use poke_core::charmap::encode;
use poke_core::map::Map;
use crate::command::Decision;
use crate::gfx::colour::Source;
use crate::gfx::compose::{HEIGHT, WIDTH};
use crate::input::Joypad;
use crate::mode::{Mode, Status};
use crate::modes::overworld::Overworld;
use crate::rng::GameRng;
use crate::systems::overworld::Location;
use crate::{Input, Pacing};
use super::*;

/// 2000-02-29 00:00:00 UTC.
const LEAP_DAY: i64 = 951_782_400;

fn on_the_overworld(location: Location) -> Game {
    let world = World { player_name: encode("RED").unwrap(), location, ..World::default() };
    let mut game = Game::new(world, GameRng::seeded(5), Pacing::Faithful);
    game.push(Mode::Overworld(Overworld::new()));
    until(&mut game, Decision::Overworld);
    game
}

fn until(game: &mut Game, decision: Decision) {
    for _ in 0..600 {
        if game.status() == Status::Waiting(decision.clone()) {
            return;
        }
        game.frame(Input::None);
    }
    panic!("never reached {decision:?}");
}

/// Walks back and forth, so the run draws on the RNG and the map scrolls.
fn play_on(game: &mut Game) {
    for frame in 0..240 {
        let held = if frame / 40 % 2 == 0 { Joypad::RIGHT } else { Joypad::LEFT };
        game.frame(Input::Buttons(held));
    }
}

fn framebuffer(shade_at: impl Fn(usize, usize) -> u8) -> Framebuffer {
    let shades = (0..HEIGHT).flat_map(|y| (0..WIDTH).map(move |x| (x, y))).map(|(x, y)| shade_at(x, y)).collect();
    Framebuffer { shades, sources: vec![Source::Background; WIDTH * HEIGHT] }
}

#[test]
fn a_summary_reads_the_world() {
    let mut world = World { player_name: encode("ASH").unwrap(), badges: 0b0000_0101, ..World::default() };
    world.pokedex.owned[0] = 0b0001_1111;
    world.play_time.hours = 12;
    world.play_time.minutes = 34;
    world.location.map = Map::OaksLab;
    let thumbnail = Thumbnail::of(&framebuffer(|_, _| 0));

    let summary = SlotSummary::of(&world, thumbnail.clone());

    assert_eq!(summary.player_name, encode("ASH").unwrap());
    assert_eq!(summary.badges, 0b0000_0101);
    assert_eq!(summary.owned, 5);
    assert_eq!((summary.play_time.hours, summary.play_time.minutes), (12, 34));
    assert_eq!(summary.thumbnail, thumbnail);
    assert_eq!(summary.location(), Some(encode("PALLET TOWN").unwrap()), "a building is named for its town");
}

/// Each pixel is a three-by-three block of the 144×120 starting eight lines down, rounded to the
/// nearer shade; the lines above it and the columns right of it are not in the picture.
#[test]
fn the_thumbnail_is_a_third_of_the_screen_around_the_player() {
    let frame = framebuffer(|x, y| match (x, y) {
        (_, 0..8) | (144.., _) => 3,
        (0..3, 8..11) => 3,
        // Five of nine at shade 1 round up; four round down.
        (3..6, 8..11) => u8::from((x - 3) + (y - 8) * 3 < 5),
        (6..9, 8..11) => u8::from((x - 6) + (y - 8) * 3 < 4),
        // One black pixel of nine is a shade 2, not white.
        (9, 8) => 3,
        (141..144, 125..128) => 2,
        _ => 0,
    });

    let thumbnail = Thumbnail::of(&frame);

    assert_eq!((0..4).map(|x| thumbnail.shade(x, 0)).collect::<Vec<_>>(), [3, 1, 0, 2]);
    assert_eq!(thumbnail.shade(Thumbnail::WIDTH - 1, Thumbnail::HEIGHT - 1), 2);
    let lit = (0..Thumbnail::HEIGHT).flat_map(|y| (0..Thumbnail::WIDTH).map(move |x| (x, y)))
        .filter(|&(x, y)| thumbnail.shade(x, y) != 0)
        .count();
    assert_eq!(lit, 4);
}

#[test]
fn a_slot_round_trips_through_a_store_and_plays_on_identically() {
    let mut game = on_the_overworld(Location { map: Map::PalletTown, x: 6, y: 8, ..Location::default() });
    let mut store = MemoryStore::default();
    game.set_slots(store.slots());
    let saved_at = SavedAt { unix_seconds: LEAP_DAY, utc_offset_minutes: 60 };

    let summary = SlotSummary::of(game.world(), Thumbnail::of(&game.screen().frame()));
    let request = game.slot_request(SlotAction::Save(2, summary));
    store.answer(&mut game, request, saved_at).unwrap();

    let slot = game.slots()[2].clone().expect("the host handed the slot back");
    assert_eq!(slot.saved_at, saved_at);
    assert_eq!(slot.summary.map, Map::PalletTown);
    assert_eq!(game.slots().iter().filter(|slot| slot.is_some()).count(), 1);

    let saved_frames = game.frames();
    play_on(&mut game);
    let played = game.save();

    store.answer(&mut game, SlotRequest::Load(2), SavedAt { unix_seconds: 0, utc_offset_minutes: 0 }).unwrap();
    assert_eq!(game.frames(), saved_frames);
    assert_eq!(game.slots()[2], Some(slot), "a loaded game is handed the slots again");
    play_on(&mut game);
    assert_eq!(game.save(), played);

    store.answer(&mut game, SlotRequest::Delete(2), saved_at).unwrap();
    assert!(game.slots().iter().all(Option::is_none));
    assert_eq!(store.read(2).unwrap_err().kind(), io::ErrorKind::NotFound);
}

#[test]
fn a_directory_store_keeps_its_slots_for_the_next_one() {
    let dir = std::env::temp_dir().join(format!("pokered-slots-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let contents = Slot {
        summary: SlotSummary::of(&World::default(), Thumbnail::of(&framebuffer(|x, y| ((x + y) % 4) as u8))),
        saved_at: SavedAt { unix_seconds: LEAP_DAY, utc_offset_minutes: -300 },
    };

    DirectoryStore::new(&dir).write(4, b"the game", contents.clone()).unwrap();
    fs::write(dir.join("slot-1.pkslot"), b"PKSL nonsense").unwrap();

    let mut store = DirectoryStore::new(&dir);
    let slots = store.slots();
    assert_eq!(slots.len(), SLOTS);
    assert_eq!(slots[4], Some(contents));
    assert_eq!(slots[1], None, "an unreadable slot is shown empty");
    assert_eq!(store.read(4).unwrap(), b"the game");
    assert_eq!(store.read(0).unwrap_err().kind(), io::ErrorKind::NotFound);
    assert_eq!(store.read(SLOTS as u8).unwrap_err().kind(), io::ErrorKind::InvalidInput);

    store.delete(4).unwrap();
    store.delete(4).unwrap();
    assert_eq!(store.slots()[4], None);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_saved_time_is_shown_in_the_host_s_zone() {
    let leap_day = SavedAt { unix_seconds: LEAP_DAY + 12 * 3600 + 34 * 60 + 56, utc_offset_minutes: 0 };
    assert_eq!(leap_day.local(), LocalTime { year: 2000, month: 2, day: 29, hour: 12, minute: 34 });
    let west = SavedAt { unix_seconds: LEAP_DAY, utc_offset_minutes: -90 };
    assert_eq!(west.local(), LocalTime { year: 2000, month: 2, day: 28, hour: 22, minute: 30 });
    let epoch = SavedAt::from_system_time(UNIX_EPOCH, 0);
    assert_eq!(epoch.local(), LocalTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0 });
}

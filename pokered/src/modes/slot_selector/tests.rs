use poke_core::map::Map;
use crate::command::Command;
use crate::gfx::compose::{HEIGHT, WIDTH};
use crate::mode::Status;
use crate::modes::overworld::Overworld;
use crate::modes::start_menu::StartMenuEntry;
use crate::rng::GameRng;
use crate::save_slots::{MemoryStore, SavedAt, SlotRequest, SlotStore};
use crate::systems::overworld::Location;
use crate::world::World;
use crate::{Game, Input, Pacing};
use super::*;

/// 2026-10-03 12:00 UTC.
const NOON: i64 = 1_791_028_800;

/// A game and the host's half: every slot request answered between frames, a minute apart.
struct Host {
    game: Game,
    store: MemoryStore,
    clock: i64,
}

impl Host {
    fn in_pallet_town() -> Self {
        let location = Location { map: Map::PalletTown, x: 6, y: 8, ..Location::default() };
        let world = World { player_name: encode("RED"), location, ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(5), Pacing::Faithful);
        game.push(Mode::Overworld(Overworld::new()));
        let store = MemoryStore::default();
        game.set_slots(store.slots());
        let mut host = Self { game, store, clock: NOON };
        host.until(Decision::Overworld);
        host
    }

    /// The frame, and what it asked of the slots once answered.
    fn frame(&mut self, input: Input) -> Option<SlotRequest> {
        let request = self.game.frame(input).slot?;
        self.clock += 60;
        let saved_at = SavedAt { unix_seconds: self.clock, utc_offset_minutes: 60 };
        self.store.answer(&mut self.game, request.clone(), saved_at).unwrap();
        Some(request)
    }

    fn until(&mut self, decision: Decision) -> u32 {
        for frames in 0..1000 {
            if self.game.status() == Status::Waiting(decision.clone()) {
                return frames;
            }
            assert_eq!(self.frame(Input::None), None, "a request before {decision:?}");
        }
        panic!("never reached {decision:?}: {:?}", self.game.modes().last());
    }

    fn press(&mut self, button: Joypad) -> Option<SlotRequest> {
        let request = self.frame(Input::Buttons(button));
        request.or_else(|| self.frame(Input::None))
    }

    fn command(&mut self, command: Command, settles: Decision) {
        self.game.frame(Input::Command(command));
        self.until(settles);
    }

    fn open(&mut self) {
        self.until(Decision::Overworld);
        self.command(Command::OpenStartMenu, Decision::StartMenu);
        self.command(Command::ChooseStartMenuEntry(StartMenuEntry::SaveReset), Decision::SlotSelector);
    }

    /// Frames with nothing pressed until a request comes.
    fn until_request(&mut self) -> SlotRequest {
        for _ in 0..1000 {
            if let Some(request) = self.frame(Input::None) {
                return request;
            }
        }
        panic!("never asked: {:?}", self.game.modes().last());
    }

    /// A at every `▼` until `decision`.
    fn read_until(&mut self, decision: Decision) {
        for _ in 0..1000 {
            match self.game.status() {
                Status::Waiting(waiting) if waiting == decision => return,
                Status::Waiting(Decision::Text) => assert_eq!(self.press(Joypad::A), None),
                _ => assert_eq!(self.frame(Input::None), None),
            }
        }
        panic!("never reached {decision:?}");
    }

    /// A on `slot` and the action on `row` of the menu it opens.
    fn choose(&mut self, slot: u8, row: u8) -> Option<SlotRequest> {
        self.move_to(slot);
        self.press(Joypad::A);
        self.until(Decision::CursorMenu);
        for _ in 0..row {
            self.press(Joypad::DOWN);
        }
        self.press(Joypad::A)
    }

    fn move_to(&mut self, slot: u8) {
        let Some(Mode::SlotSelector(selector)) = self.game.modes().last() else { panic!("the selector is not up") };
        let from = selector.current();
        let button = if from < slot { Joypad::DOWN } else { Joypad::UP };
        for _ in 0..from.abs_diff(slot) {
            self.press(button);
        }
    }

    /// Saves in `slot`, empty or not, from the overworld, and is back on it.
    fn save_in(&mut self, slot: u8) -> Vec<u8> {
        self.open();
        let used = self.game.slots()[slot as usize].is_some();
        self.choose(slot, 0);
        if used {
            self.read_until(Decision::TwoOption);
            self.press(Joypad::A);
        }
        let SlotRequest::Save { bytes, .. } = self.until_request() else { panic!("not a save") };
        self.until(Decision::Overworld);
        bytes
    }

    fn text(&self, x: usize, y: usize, text: &str) -> bool {
        let bytes = encode(text);
        self.game.ui().row(y)[x..x + bytes.len()] == bytes[..]
    }

    /// The screen as a PGM under `SLOT_SELECTOR_DUMP`, to look at.
    fn dump(&self, name: &str) {
        let Ok(dir) = std::env::var("SLOT_SELECTOR_DUMP") else { return };
        let mut pgm = format!("P5 {WIDTH} {HEIGHT} 255\n").into_bytes();
        pgm.extend(self.game.screen().frame().shades.iter().map(|&shade| 255 - 85 * shade));
        std::fs::write(format!("{dir}/{name}.pgm"), pgm).unwrap();
    }
}

/// Walks back and forth, so the run draws on the RNG and the map scrolls.
fn walk(game: &mut Game) {
    for frame in 0..240 {
        let held = if frame / 40 % 2 == 0 { Joypad::RIGHT } else { Joypad::LEFT };
        game.frame(Input::Buttons(held));
    }
}

/// The map's tiles the picture is loaded over, bar the flower and the water: reloading the tileset
/// starts their animation again, as it does after the trainer card.
fn picture_tiles(game: &Game) -> Vec<[u8; 16]> {
    (0..30).filter(|id| ![0x03, 0x14].contains(id)).map(|id| *game.screen().tiles.bg(id)).collect()
}

fn blank_row(game: &Game, y: usize) -> bool {
    game.ui().row(y)[1..19].iter().all(|&tile| tile == UiSurface::BLANK)
}

#[test]
fn an_empty_list_offers_only_save() {
    let mut host = Host::in_pallet_town();
    host.open();
    host.dump("empty");

    for slot in 0..SLOTS {
        let label = if slot == AUTOSAVE as usize { "A".to_string() } else { (slot + 1).to_string() };
        assert!(host.text(2, 10 + slot, &format!("{label} -------")), "row {slot}");
    }
    assert!(blank_row(&host.game, 1), "nothing to preview");
    assert_eq!(host.game.ui().get(1, 10), 0xED, "the cursor on the first slot");

    host.press(Joypad::A);
    host.until(Decision::CursorMenu);
    assert!(host.text(13, 11, "SAVE"));
    assert!(!host.text(13, 13, "LOAD"), "nothing to load");
    assert_eq!(host.game.ui().get(1, 10), 0xEC, "the list's cursor left unfilled");
}

/// The save comes in the frame both menus close, so the slot is the game on the overworld; the
/// cartridge's pacing comes before it.
#[test]
fn a_save_closes_the_menus_and_the_slot_is_the_game_on_the_overworld() {
    let mut host = Host::in_pallet_town();
    let before = Thumbnail::of(&host.game.screen().frame());
    let tiles = picture_tiles(&host.game);
    host.dump("overworld");
    host.open();
    host.press(Joypad::DOWN);
    host.press(Joypad::A);
    host.until(Decision::CursorMenu);
    let pressed = host.game.frames();
    assert_eq!(host.press(Joypad::A), None);
    assert!(host.text(1, 14, "Now saving..."));
    assert!(host.text(8, 1, "RED") && host.text(1, 6, "PALLET TOWN"), "the game being saved, previewed");
    host.dump("now-saving");

    let mut said = false;
    let bytes = loop {
        match host.frame(Input::None) {
            None => said |= host.text(1, 14, "RED saved"),
            Some(SlotRequest::Save { slot, bytes, summary }) => {
                assert_eq!(slot, 1);
                assert_eq!(summary.thumbnail, before, "the screen before START");
                assert_eq!(summary.map, Map::PalletTown);
                break bytes;
            }
            Some(other) => panic!("{other:?}"),
        }
    };
    assert!(said, "\"RED saved the game!\"");
    assert!(host.game.frames() - pressed > (NOW_SAVING + AFTER_SAVED) as u64);
    assert!(matches!(host.game.modes(), [Mode::Overworld(_)]), "{:?}", host.game.modes().len());
    assert_eq!(bytes, host.game.save(), "the slot is the game as it resumes");
    assert_eq!(picture_tiles(&host.game), tiles, "the map's tiles are back");
    let slot = host.game.slots()[1].clone().expect("the host handed it back");
    assert_eq!(slot.saved_at.unix_seconds, NOON + 60);

    let mut loaded = Game::load(&bytes, Pacing::Faithful).unwrap();
    walk(&mut host.game);
    walk(&mut loaded);
    assert_eq!(loaded.save(), host.game.save(), "the slot plays on as the game did");
}

#[test]
fn the_preview_follows_the_cursor() {
    let mut host = Host::in_pallet_town();
    host.save_in(0);
    host.game.world_mut().play_time.hours = 12;
    host.game.world_mut().badges = 0b0000_0111;
    walk(&mut host.game);
    host.save_in(3);
    host.open();
    host.dump("two-slots");

    assert!(host.text(2, 10, "1 RED"));
    assert!(host.text(2, 13, "4 RED"));
    assert_eq!(host.game.ui().get(1, 13), 0xED, "on the newest slot");
    assert!(host.text(8, 1, "RED"));
    assert!(host.text(8, 2, "BADGES"));
    assert_eq!(host.game.ui().get(18, 2), encode("3")[0]);
    assert!(host.text(14, 4, "12"));
    assert!(host.text(1, 6, "PALLET TOWN"));
    assert!(host.text(1, 7, "2026/10/03"), "the host's date");
    assert_eq!(host.game.ui().get(1, 1), 0, "the picture's first tile");
    assert!(host.game.screen().sprites.iter().all(|object| object.y == 0), "no sprite over the screen");

    host.move_to(0);
    assert_eq!(host.game.ui().get(18, 2), encode("0")[0], "the first slot's badges");
    assert_eq!(host.game.ui().get(14, 4), UiSurface::BLANK, "and its hours");
    host.move_to(1);
    assert!(blank_row(&host.game, 1), "an empty slot");
}

#[test]
fn writing_over_a_slot_asks_and_no_keeps_it() {
    let mut host = Host::in_pallet_town();
    let first = host.save_in(0);
    walk(&mut host.game);
    host.open();

    host.press(Joypad::A);
    host.until(Decision::CursorMenu);
    assert!(host.text(13, 11, "SAVE") && host.text(13, 13, "LOAD") && host.text(13, 15, "DELETE"));
    host.press(Joypad::A);
    host.until(Decision::Text);
    assert!(host.text(1, 14, "The older file"));
    assert!(host.text(2, 10, "1 RED") && !host.text(13, 11, "SAVE"), "the actions taken down");
    host.read_until(Decision::TwoOption);
    host.dump("overwrite");
    host.press(Joypad::DOWN);
    assert_eq!(host.press(Joypad::A), None);
    host.until(Decision::SlotSelector);
    assert_eq!(host.store.read(0).unwrap(), first, "NO keeps the slot");
    assert!(host.text(2, 10, "1 RED"), "the list is back");
    host.press(Joypad::B);
    host.until(Decision::StartMenu);
    host.command(Command::CloseStartMenu, Decision::Overworld);

    let second = host.save_in(0);
    assert_ne!(second, first);
    assert_eq!(host.store.read(0).unwrap(), second, "YES writes over it");
}

#[test]
fn delete_asks_and_yes_empties_the_slot() {
    let mut host = Host::in_pallet_town();
    host.save_in(2);
    host.open();

    assert_eq!(host.choose(2, 2), None);
    host.until(Decision::TwoOption);
    assert!(host.text(1, 14, "Delete the file"));
    assert!(host.text(1, 16, "in slot 3?"));
    host.dump("delete");
    assert_eq!(host.press(Joypad::A), None, "NO is first");
    host.until(Decision::SlotSelector);
    assert!(host.game.slots()[2].is_some());

    host.choose(2, 2);
    host.until(Decision::TwoOption);
    host.press(Joypad::DOWN);
    host.press(Joypad::A);
    assert_eq!(host.until_request(), SlotRequest::Delete(2));
    host.until(Decision::SlotSelector);
    assert!(host.game.slots().iter().all(Option::is_none));
    assert!(host.text(2, 12, "3 -------"), "drawn again from what the host holds");
}

/// The game a slot holds replaces the one in play and goes on from the overworld as it would have.
#[test]
fn a_loaded_slot_resumes_on_the_overworld_and_plays_on_identically() {
    let mut host = Host::in_pallet_town();
    let saved = host.save_in(4);
    walk(&mut host.game);
    host.open();

    assert_eq!(host.choose(4, 1), Some(SlotRequest::Load(4)));
    assert_eq!(host.game.save(), saved, "the game is the slot's");
    assert!(matches!(host.game.modes(), [Mode::Overworld(_)]));
    assert_eq!(host.game.slots()[4].as_ref().map(|slot| slot.summary.map), Some(Map::PalletTown));

    let mut reference = Game::load(&saved, Pacing::Faithful).unwrap();
    walk(&mut host.game);
    walk(&mut reference);
    assert_eq!(host.game.save(), reference.save());
}

#[test]
fn b_backs_out_to_the_start_menu_with_the_map_s_tiles_back() {
    let mut host = Host::in_pallet_town();
    host.save_in(0);
    let tiles = picture_tiles(&host.game);
    host.open();
    host.press(Joypad::B);
    host.until(Decision::StartMenu);
    assert!(host.text(12, 8, "SAVE"), "the start menu is redrawn");
    assert!(host.game.screen().sprites.iter().any(|object| object.y != 0), "and the sprites beside it");
    assert_eq!(picture_tiles(&host.game), tiles);
}

/// What CONTINUE opens: the newest slot under the cursor, nothing to save, and an empty slot inert.
#[test]
fn load_only_offers_load_and_delete_on_a_used_slot() {
    let mut host = Host::in_pallet_town();
    host.save_in(1);
    host.save_in(5);
    host.save_in(1);
    host.game.push(Mode::SlotSelector(SlotSelector::load_only()));
    host.until(Decision::SlotSelector);
    assert_eq!(host.game.ui().get(1, 11), 0xED, "on the newest");

    host.press(Joypad::A);
    host.until(Decision::CursorMenu);
    assert!(host.text(13, 11, "LOAD") && host.text(13, 13, "DELETE"));
    host.press(Joypad::B);
    host.until(Decision::SlotSelector);

    host.move_to(0);
    host.press(Joypad::A);
    assert_eq!(host.game.status(), Status::Waiting(Decision::SlotSelector), "nothing in it");
}

/// The Hall of Fame's slot, below the player's: loaded and deleted like any, never saved to.
#[test]
fn the_autosave_is_marked_and_offers_no_save() {
    let mut host = Host::in_pallet_town();
    host.open();
    host.move_to(AUTOSAVE);
    host.press(Joypad::A);
    assert_eq!(host.game.status(), Status::Waiting(Decision::SlotSelector), "nothing to do on an empty autosave");
    host.press(Joypad::B);
    host.until(Decision::StartMenu);
    host.command(Command::CloseStartMenu, Decision::Overworld);

    let summary = SlotSummary::of(host.game.world(), Thumbnail::of(&host.game.screen().frame()));
    let saved_at = SavedAt { unix_seconds: NOON, utc_offset_minutes: 0 };
    host.store.write(AUTOSAVE, &host.game.save(), Slot { summary, saved_at }).unwrap();
    host.game.set_slots(host.store.slots());
    host.open();
    assert!(host.text(2, 10 + AUTOSAVE as usize, "A RED"));
    assert_eq!(host.game.ui().get(1, 10 + AUTOSAVE as usize), 0xED, "on the newest");
    host.press(Joypad::A);
    host.until(Decision::CursorMenu);
    assert!(host.text(13, 11, "LOAD") && host.text(13, 13, "DELETE"));

    host.press(Joypad::DOWN);
    host.press(Joypad::A);
    host.until(Decision::TwoOption);
    assert!(host.text(1, 14, "Delete the") && host.text(1, 16, "autosave?"));
}

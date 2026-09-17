//! The overworld, against the cartridge: the same walk played on both, compared at every step's end.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::{RAM, ROM};
use poke_core::map::Map;
use poke_core::sprite::SpriteFacing;
use pokered::gfx::compose::WIDTH;
use pokered::gfx::tiles::V_CHARS2;
use pokered::command::Decision;
use pokered::mode::Status;
use pokered::audio::data::sounds;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::overworld::{Overworld, Standing};
use pokered::rng::GameRng;
use pokered::systems::overworld::sprites::{SpriteState, Sprites};
use pokered::systems::overworld::Location;
use pokered::systems::overworld::location::Ahead;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointerRead};
use super::{assert_late, breakpoint, joypad};

const STATE_BYTES: u16 = 16;
/// `wStatusFlags6`'s.
const BIT_ALWAYS_ON_BIKE: u8 = 5;
/// `wStatusFlags1`'s.
const BIT_STRENGTH_ACTIVE: u8 = 0;

fn local_label(label: &str) -> crate::pokemon::symbols::DmgPointer {
    use crate::pokemon::symbols::{DmgBank, DmgPointer};
    include_str!("../../../vendor/pokered/pokered.sym").lines()
        .find_map(|line| {
            let (at, name) = line.split_once(' ')?;
            let (bank, address) = at.split_once(':')?;
            (name == label).then(|| DmgPointer {
                bank: DmgBank::ROM { bank: u8::from_str_radix(bank, 16).unwrap() },
                address: u16::from_str_radix(address, 16).unwrap(),
            })
        })
        .unwrap_or_else(|| panic!("no {label} in pokered.sym"))
}

/// The cartridge, with every `Random` the NPC movement routines take recorded for the recreation.
struct Cartridge {
    gb: GameBoy,
    tape: Vec<u8>,
    frames: u32,
}

impl Cartridge {
    fn from_state(state: &[u8]) -> Self {
        let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(state).unwrap();
        gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
        Self { gb, tape: Vec::new(), frames: 0 }
    }

    fn read(&self, at: u16) -> u8 {
        self.gb.core().mmu().read(at)
    }

    /// Runs to the first of `points`, recording the NPC routines' `Random` bytes on the way. The
    /// LCD is off while a map loads, so a stretch without any of them is not an error.
    fn run_to(&mut self, points: &[gb::game_boy::Breakpoint]) -> gb::game_boy::Breakpoint {
        let random = breakpoint(sym::Random);
        let movement = sym::UpdateNPCSprite.address..sym::DoScriptedNPCMovement.address;
        let mut all = points.to_vec();
        all.push(random);
        loop {
            match self.gb.run_until(&all, MachineCycles::PER_FRAME * 2).0 {
                Stop::Breakpoint(hit) if hit == random => {
                    let from = self.gb.return_address();
                    let bank = self.gb.core().mmu().rom_bank() as u8;
                    if movement.contains(&from) && bank == sym::UpdateNPCSprite.bank.id() {
                        let (stop, _) = self.gb.run_to_return(MachineCycles::PER_FRAME);
                        assert!(matches!(stop, Stop::Returned { .. }));
                        self.tape.push(self.gb.core().registers().a);
                    }
                }
                Stop::Breakpoint(hit) => return hit,
                Stop::Budget => {}
                stop => panic!("{stop:?}"),
            }
        }
    }

    /// To the start of the next VBlank. `true` if the loop polled the pad with nothing moving,
    /// nothing scripted and the joypad enabled; then this frame runs on to the end of that pass,
    /// which a lag frame can push past the VBlank.
    fn frame(&mut self) -> bool {
        let vblank = breakpoint(sym::VBlank);
        if self.run_to(&[vblank, breakpoint(local_label("OverworldLoopLessDelay.noDirectionButtonsPressed"))]) == vblank {
            self.frames += 1;
            return false;
        }
        let flags5 = self.read(sym::wStatusFlags5.address);
        let movement_flags = self.read(sym::wMovementFlags.address);
        let polled = flags5 & (1 << 7 | 1 << 5) == 0 && movement_flags & 0b11 == 0
            && self.read(sym::wWalkCounter.address) == 0;
        let end = if polled { breakpoint(sym::OverworldLoop) } else { vblank };
        while self.run_to(&[end, vblank]) != end {
            self.frames += 1;
        }
        if polled {
            self.run_to(&[vblank]);
        }
        self.frames += 1;
        polled
    }

    fn location(&self) -> (Map, u8, u8, SpriteFacing) {
        let mmu = self.gb.core().mmu();
        (Map::from_repr(mmu.read_pointer(&sym::wCurMap)).unwrap(), mmu.read_pointer(&sym::wXCoord),
         mmu.read_pointer(&sym::wYCoord), SpriteFacing::from_repr(self.read(sym::wSpriteStateData1.address + 9)).unwrap())
    }

    fn sprites(&self) -> Sprites {
        let mmu = self.gb.core().mmu();
        std::array::from_fn(|slot| {
            let at = slot as u16 * STATE_BYTES;
            let data1 = mmu.read_slice(sym::wSpriteStateData1.address + at, 16);
            let data2 = mmu.read_slice(sym::wSpriteStateData2.address + at, 16);
            let map_data = if slot == 0 { [0, 0] } else {
                let entry = sym::wMapSpriteData.address + (slot as u16 - 1) * 2;
                [mmu.read(entry), mmu.read(entry + 1)]
            };
            SpriteState::from_bytes(&data1, &data2, map_data)
        })
    }

    fn lcd(&self) -> Vec<u8> {
        self.gb.core().mmu().ppu().screenshot().pixels().map(|p| match p.0[0] {
            0xFF => 0,
            0xAA => 1,
            0x55 => 2,
            _ => 3,
        }).collect()
    }
}


/// What each machine is asked to do between two points where it is waiting to be told.
#[derive(Debug, Clone, Copy)]
enum Move {
    /// Hold the direction until a step or a jump has begun.
    Walk(Joypad),
    /// The same, for a step that ends in a warp.
    Warp(Joypad),
    /// Hold the direction into something that stops it, until the bump sounds.
    Bump(Joypad),
}

/// A point both agree on, seen from one machine.
#[derive(Debug, PartialEq)]
struct Seen {
    frames: u32,
    location: (Map, u8, u8, SpriteFacing),
    /// `wNumSprites`.
    count: u8,
    /// The LCD's shades once the frame after the point has been drawn.
    lcd: Vec<u8>,
    /// Tiles `$03` and `$14`, which animate on VBlank's clock rather than the loop's.
    animated: Vec<u8>,
    sprites: Sprites,
}


const WALK_BUDGET: u32 = 600;
const CHAN5: u16 = 4;

impl Cartridge {
    fn walk(&mut self, step: Move) -> Seen {
        let start = self.frames;
        let from = self.location().0;
        let (Move::Walk(button) | Move::Warp(button) | Move::Bump(button)) = step;
        self.gb.hold_buttons(joypad(button));
        for held in 0.. {
            assert!(held < WALK_BUDGET, "{step:?}: the cartridge never moved");
            self.frame();
            let started = match step {
                Move::Walk(_) | Move::Warp(_) => self.read(sym::wWalkCounter.address) != 0 || self.location().0 != from || self.read(0xFF47) != 0xE4,
                Move::Bump(_) => self.read(sym::wChannelSoundIDs.address + CHAN5) == sounds::SFX_COLLISION.0,
            };
            if started {
                break;
            }
        }
        self.gb.hold_buttons(JoypadButtonState::default());
        for waited in 0.. {
            assert!(waited < WALK_BUDGET, "{step:?}: the cartridge never settled");
            let polled = self.frame();
            if polled {
                break;
            }
        }
        let frames = self.frames - start;
        let location = self.location();
        let count = self.read(sym::wNumSprites.address);
        let sprites = self.sprites();
        // VBlank copies `wShadowOAM` to OAM before it prepares the next, so the pass's sprites are
        // on the LCD two frames on.
        self.frame();
        self.frame();
        let vram = self.gb.core().mmu().read_vram_slice(0x9000, 0x60 * 16).unwrap();
        let animated = [0x03, 0x14].iter().flat_map(|&t: &usize| vram[t * 16..t * 16 + 16].to_vec()).collect();
        Seen { frames, location, count, lcd: self.lcd(), animated, sprites }
    }

    fn world(&self) -> pokered::world::World {
        let mmu = self.gb.core().mmu();
        let mut world = super::item_menu::the_world(&self.gb);
        let (map, x, y, facing) = self.location();
        world.location = Location {
            map, x, y, facing,
            last_map: Map::from_repr(mmu.read_pointer(&sym::wLastMap)).unwrap(),
            walk_bike_surf: mmu.read_pointer(&sym::wWalkBikeSurfState),
            hidden_objects: mmu.read_slice(sym::wToggleableObjectFlags.address, 32),
            towns_visited: u16::from_le_bytes([mmu.read(sym::wTownVisitedFlag.address), mmu.read(sym::wTownVisitedFlag.address + 1)]),
            last_blackout_map: Map::from_repr(mmu.read_pointer(&sym::wLastBlackoutMap)).unwrap(),
            repel_steps: mmu.read_pointer(&sym::wRepelRemainingSteps),
            always_on_bike: mmu.read_pointer(&sym::wStatusFlags6) & 1 << BIT_ALWAYS_ON_BIKE != 0,
            ahead: Ahead {
                tile: mmu.read_pointer(&sym::wTileInFrontOfPlayer),
                standing_on: mmu.read_pointer(&sym::wTilePlayerStandingOn),
                sprite: false,
            },
            strength_active: mmu.read_pointer(&sym::wStatusFlags1) & 1 << BIT_STRENGTH_ACTIVE != 0,
            used_field_move: None,
            fly_warp: None,
            escape_warp: false,
        };
        world
    }

    fn standing(&self) -> Standing {
        let mmu = self.gb.core().mmu();
        Standing {
            player_direction: mmu.read_pointer(&sym::wPlayerDirection),
            moving_direction: mmu.read_pointer(&sym::wPlayerMovingDirection),
            last_stop_direction: mmu.read_pointer(&sym::wPlayerLastStopDirection),
            check_for_180_degree_turn: mmu.read_pointer(&sym::wCheckFor180DegreeTurn),
            standing_on_warp: mmu.read_pointer(&sym::wMovementFlags) & 1 << 2 != 0,
            destination_warp: mmu.read_pointer(&sym::wDestinationWarpID),
        }
    }
}

fn overworld(game: &Game) -> &Overworld {
    match game.modes().last() {
        Some(Mode::Overworld(overworld)) => overworld,
        other => panic!("the overworld is not on top: {other:?}"),
    }
}

fn recreation_walk(game: &mut Game, step: Move) -> (Seen, pokered::gfx::Screen) {
    let from = game.world().location.map;
    let (Move::Walk(button) | Move::Warp(button) | Move::Bump(button)) = step;
    let mut frames = 0;
    for held in 0.. {
        assert!(held < WALK_BUDGET, "{step:?}: the recreation never moved");
        game.frame(Input::Buttons(button));
        frames += 1;
        let overworld = overworld(game);
        let started = match step {
            Move::Walk(_) | Move::Warp(_) => overworld.walk_counter() != 0 || game.world().location.map != from || game.screen().effects.bgp != 0xE4,
            Move::Bump(_) => game.audio().channel_sound_id(CHAN5 as usize) == sounds::SFX_COLLISION.0,
        };
        if started {
            break;
        }
    }
    for waited in 0.. {
        assert!(waited < WALK_BUDGET, "{step:?}: the recreation never settled");
        game.frame(Input::None);
        frames += 1;
        if game.status() == Status::Waiting(Decision::Overworld) {
            break;
        }
    }
    let location = &game.world().location;
    let location = (location.map, location.x, location.y, location.facing);
    let overworld = overworld(game);
    let (count, sprites) = (overworld.num_sprites(), *overworld.sprites());
    let screen = game.screen().clone();
    game.frame(Input::None);
    game.frame(Input::None);
    (Seen { frames, location, count, lcd: Vec::new(), animated: Vec::new(), sprites }, screen)
}

/// Pallet Town below Oak's lab, into Red's house and out, a bump into the fence, north through the
/// grass into Route 1, round the gap in the first row of ledges and back down over one.
const ROUTE: &[(Move, &str)] = &[
    (Move::Walk(Joypad::LEFT), "left, turning"),
    (Move::Walk(Joypad::LEFT), "left"),
    (Move::Walk(Joypad::LEFT), "left"),
    (Move::Walk(Joypad::LEFT), "left to (8, 12)"),
    (Move::Walk(Joypad::UP), "up, turning"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up to (8, 6)"),
    (Move::Walk(Joypad::LEFT), "left, turning"),
    (Move::Walk(Joypad::LEFT), "left"),
    (Move::Walk(Joypad::LEFT), "left to (5, 6)"),
    (Move::Warp(Joypad::UP), "through Red's door"),
    (Move::Warp(Joypad::DOWN), "out off the mat and down from the door"),
    (Move::Walk(Joypad::LEFT), "left to (4, 6)"),
    (Move::Bump(Joypad::UP), "into the fence"),
    (Move::Walk(Joypad::RIGHT), "right"),
    (Move::Walk(Joypad::RIGHT), "right"),
    (Move::Walk(Joypad::RIGHT), "right"),
    (Move::Walk(Joypad::RIGHT), "right"),
    (Move::Walk(Joypad::RIGHT), "right"),
    (Move::Walk(Joypad::RIGHT), "right to (10, 6)"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "into the grass"),
    (Move::Walk(Joypad::UP), "the edge of Pallet Town"),
    (Move::Walk(Joypad::UP), "into Route 1"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up"),
    (Move::Walk(Joypad::UP), "up to (10, 29)"),
    (Move::Walk(Joypad::UP), "up to (10, 28)"),
    (Move::Walk(Joypad::LEFT), "left"),
    (Move::Walk(Joypad::LEFT), "left"),
    (Move::Walk(Joypad::LEFT), "left to (7, 28)"),
    (Move::Walk(Joypad::UP), "up through the gap"),
    (Move::Walk(Joypad::UP), "up to (7, 26)"),
    (Move::Walk(Joypad::RIGHT), "right"),
    (Move::Walk(Joypad::RIGHT), "right"),
    (Move::Walk(Joypad::RIGHT), "right to (10, 26)"),
    (Move::Walk(Joypad::DOWN), "over the ledge"),
    (Move::Bump(Joypad::UP), "back into the ledge, which is a wall from below"),
    (Move::Walk(Joypad::LEFT), "and on along the grass"),
];

#[test]
fn a_walk_through_a_door_across_a_connection_and_over_a_ledge_matches_the_cartridge() {
    let mut cartridge = Cartridge::from_state(include_bytes!("../pokemon/data/pallet-town-state.bin"));
    // Grass on the way would start wild battles, which are the battle's.
    let flags4 = sym::wStatusFlags4.address;
    let value = cartridge.read(flags4) | 1 << 4;
    cartridge.gb.core_mut().mmu_mut().write(flags4, value);
    while !cartridge.frame() {}
    let world = cartridge.world();
    let (sprites, count, standing) = (cartridge.sprites(), cartridge.read(sym::wNumSprites.address), cartridge.standing());
    assert_eq!(world.location.map, Map::PalletTown);
    let seen: Vec<Seen> = ROUTE.iter().map(|&(step, _)| cartridge.walk(step)).collect();

    let mut game = Game::new(world, GameRng::tape(cartridge.tape.clone()), Pacing::Faithful);
    game.push(Mode::Overworld(Overworld::standing(sprites, count, standing).with_battle_flags(true, false, 0)));
    for (&(step, what), cartridge) in ROUTE.iter().zip(&seen) {
        let (recreation, mut screen) = recreation_walk(&mut game, step);
        assert_eq!(recreation.location, cartridge.location, "{what}");
        // `PrepareOAMData` and `DetectCollisionBetweenSprites` both write the adjusted coordinates,
        // in the other order on the cartridge, and nothing reads them before rewriting them.
        let compared = |sprites: &Sprites| sprites.iter().take(cartridge.count as usize + 1)
            .map(|&s| SpriteState { y_adjusted: 0, x_adjusted: 0, ..s }).collect::<Vec<_>>();
        let (ours, theirs) = (compared(&recreation.sprites), compared(&cartridge.sprites));
        if let Some(slot) = (0..ours.len()).find(|&slot| ours[slot] != theirs[slot]) {
            panic!("{what}: sprite slot {slot}\ncartridge  {:?}\nrecreation {:?}", theirs[slot], ours[slot]);
        }
        screen.tiles.load(V_CHARS2 + 0x03, &cartridge.animated[..16]);
        screen.tiles.load(V_CHARS2 + 0x14, &cartridge.animated[16..]);
        let lcd = screen.frame().shades;
        let wrong: Vec<(usize, usize)> = (0..cartridge.lcd.len())
            .filter(|&i| cartridge.lcd[i] != lcd[i])
            .map(|i| (i % WIDTH, i / WIDTH))
            .collect();
        assert!(wrong.is_empty(), "{what}: {} pixels differ, first {:?}", wrong.len(), &wrong[..wrong.len().min(8)]);
        // A warp turns the LCD off to load the map, so it has no frames to count.
        if !matches!(step, Move::Warp(_)) {
            assert_late(cartridge.frames, recreation.frames, 0, what);
        }
    }
}

impl Cartridge {
    /// Holds `button` for `PRESS_FRAMES`, long enough for a loop that polls every other frame, and
    /// runs to wherever the game next reads the pad for a text or for the overworld: `true` for the
    /// overworld.
    fn press_to_poll(&mut self, button: Joypad) -> bool {
        self.gb.hold_buttons(joypad(button));
        for _ in 0..PRESS_FRAMES {
            self.frame();
        }
        self.gb.hold_buttons(JoypadButtonState::default());
        let text = breakpoint(sym::JoypadLowSensitivity);
        let overworld = breakpoint(local_label("OverworldLoopLessDelay.noDirectionButtonsPressed"));
        let hit = self.run_to(&[text, overworld]);
        super::to_vblank(&mut self.gb);
        hit == overworld
    }

    fn text_rows(&self) -> Vec<Vec<u8>> {
        (12..18).map(|y| super::tile_row(&self.gb, y)).collect()
    }
}

const PRESS_FRAMES: u32 = 3;

/// Pallet Town's sign, read from below: the two prompts of its text and the map back afterwards.
#[test]
fn reading_a_sign_prints_and_closes_as_the_cartridge_does() {
    let mut cartridge = Cartridge::from_state(include_bytes!("../pokemon/data/pallet-town-state.bin"));
    while !cartridge.frame() {}
    let world = cartridge.world();
    let (sprites, count, standing) = (cartridge.sprites(), cartridge.read(sym::wNumSprites.address), cartridge.standing());
    let route = [Joypad::LEFT, Joypad::LEFT, Joypad::LEFT, Joypad::LEFT, Joypad::LEFT, Joypad::UP, Joypad::UP];
    let mut walked: Vec<Seen> = route.iter().map(|&button| cartridge.walk(Move::Walk(button))).collect();
    walked.push(cartridge.walk(Move::Bump(Joypad::UP)));
    assert_eq!(walked.last().unwrap().location, (Map::PalletTown, 7, 10, SpriteFacing::Up));
    let mut read = Vec::new();
    loop {
        let back = cartridge.press_to_poll(Joypad::A);
        read.push((cartridge.text_rows(), back));
        if back {
            break;
        }
        assert!(read.len() < 8, "the sign never closed");
    }
    assert_eq!(read.len(), 3, "the `cont`'s prompt, the wait after `done`, and the map back");

    let mut game = Game::new(world, GameRng::tape(cartridge.tape.clone()), Pacing::Faithful);
    game.push(Mode::Overworld(Overworld::standing(sprites, count, standing)));
    for &button in &route {
        recreation_walk(&mut game, Move::Walk(button));
    }
    recreation_walk(&mut game, Move::Bump(Joypad::UP));
    for (i, (rows, back)) in read.iter().enumerate() {
        for _ in 0..PRESS_FRAMES {
            game.frame(Input::Buttons(Joypad::A));
        }
        for frames in 0.. {
            assert!(frames < 600, "press {i}: the recreation never waited again");
            game.frame(Input::None);
            let status = game.status();
            if status == Status::Waiting(Decision::Overworld) || status == Status::Waiting(Decision::Text) {
                assert_eq!(status == Status::Waiting(Decision::Overworld), *back, "press {i}");
                break;
            }
        }
        if !back {
            let ours: Vec<Vec<u8>> = (12..18).map(|y| game.ui().row(y).to_vec()).collect();
            assert_eq!(&ours, rows, "press {i}: the text box");
        }
    }
    assert!(game.ui().cover(1, 14).is_none(), "the box is gone");
}

/// `wStatusFlags6`'s.
const BIT_FLY_WARP: u8 = 3;
const BIT_ESCAPE_WARP: u8 = 6;

/// The escape warp armed by hand on both machines: `ItemUseEscapeRope` sets exactly these two bits
/// and the loop does the rest. Pallet Town is the last blackout map in this state, so the player
/// spins out of it and back down onto its own fly-warp square.
#[test]
fn an_escape_warp_spins_out_and_lands_where_the_cartridge_does() {
    let mut cartridge = Cartridge::from_state(include_bytes!("../pokemon/data/pallet-town-state.bin"));
    while !cartridge.frame() {}
    let mut world = cartridge.world();
    let (sprites, count, standing) = (cartridge.sprites(), cartridge.read(sym::wNumSprites.address), cartridge.standing());
    let flags6 = sym::wStatusFlags6.address;
    let armed = cartridge.read(flags6) | 1 << BIT_FLY_WARP | 1 << BIT_ESCAPE_WARP;
    cartridge.gb.core_mut().mmu_mut().write(flags6, armed);

    // The leave animation runs with the LCD on, so its frames are countable; the map load that
    // follows turns the LCD off, and nothing after it is.
    let start = cartridge.frames;
    let prepare = breakpoint(local_label("PrepareForSpecialWarp"));
    let vblank = breakpoint(sym::VBlank);
    while cartridge.run_to(&[prepare, vblank]) != prepare {
        cartridge.frames += 1;
    }
    let leaving = cartridge.frames - start;
    for waited in 0.. {
        assert!(waited < 2000, "the cartridge never landed");
        if cartridge.frame() {
            break;
        }
    }
    let landed = cartridge.location();

    world.location.fly_warp = Some(world.location.last_blackout_map);
    world.location.escape_warp = true;
    let mut game = Game::new(world, GameRng::tape(cartridge.tape.clone()), Pacing::Faithful);
    game.push(Mode::Overworld(Overworld::standing(sprites, count, standing).with_battle_flags(true, false, 0)));
    // Both machines start at the loop's poll, so the recreation is told the pad is empty until the
    // warp has taken it away from there.
    let from = game.world().location.x;
    let (mut ours, mut left, mut started) = (0, None, false);
    for waited in 0.. {
        assert!(waited < 2000, "the recreation never landed: {:?}", game.status());
        let polling = game.status() == Status::Waiting(Decision::Overworld);
        if started && polling {
            break;
        }
        game.frame(if polling { Input::Buttons(Joypad::empty()) } else { Input::None });
        ours += 1;
        started |= !polling;
        if left.is_none() && game.world().location.x != from {
            left = Some(ours);
        }
    }
    let location = &game.world().location;
    assert_eq!((location.map, location.x, location.y, location.facing), landed);
    assert_eq!((location.fly_warp, location.escape_warp), (None, false), "and both flags are spent");
    assert_late(leaving, left.expect("the recreation warped"), 0, "the spin out and the fade");
    // The screen is not compared here: coming out of the arrival animation the recreation's NPCs
    // stand one `UpdateSprites` ahead of the cartridge's, which the walk above pins frame by frame.
}

/// The Viridian Gym's door from the square outside it, then up and west onto the arrow at (13, 16),
/// which is `ViridianGymArrowMovement11`: six squares left.
const SPINNER_ROUTE: &[(Move, &str)] = &[
    (Move::Warp(Joypad::UP), "through the gym's door"),
    (Move::Walk(Joypad::UP), "up to (16, 16)"),
    (Move::Walk(Joypad::LEFT), "left, turning"),
    (Move::Walk(Joypad::LEFT), "left to (14, 16)"),
    (Move::Walk(Joypad::LEFT), "onto the arrow tile, and along it"),
];

/// The badge is already won in this state, so nothing in the gym is left to see the player walk in.
#[test]
fn an_arrow_tile_slides_and_spins_the_player_as_the_cartridge_does() {
    let mut cartridge = Cartridge::from_state(include_bytes!("../pokemon/data/completion-earth.bin"));
    while !cartridge.frame() {}
    let world = cartridge.world();
    let (sprites, count, standing) = (cartridge.sprites(), cartridge.read(sym::wNumSprites.address), cartridge.standing());
    let seen: Vec<Seen> = SPINNER_ROUTE.iter().map(|&(step, _)| cartridge.walk(step)).collect();
    assert_eq!(seen.last().unwrap().location.1, 7, "the cartridge slid the player six squares west");

    let mut game = Game::new(world, GameRng::tape(cartridge.tape.clone()), Pacing::Faithful);
    game.push(Mode::Overworld(Overworld::standing(sprites, count, standing).with_battle_flags(true, false, 0)));
    let slide = SPINNER_ROUTE.len() - 1;
    for (step_index, (&(step, what), cartridge)) in SPINNER_ROUTE.iter().zip(&seen).enumerate() {
        let (recreation, _) = recreation_walk(&mut game, step);
        assert_eq!(recreation.location, cartridge.location, "{what}");
        assert_eq!(recreation.sprites[0].image_index, cartridge.sprites[0].image_index, "{what}: the facing the spin left");
        // A warp turns the LCD off to load the map, so it has no frames to count. Nor has the slide:
        // what makes the cartridge's take nearly three times as long is `CopyVideoData` waiting for
        // a transfer per arrow tile on every pass of it, which is loading rather than pacing.
        if !matches!(step, Move::Warp(_)) && step_index != slide {
            assert_late(cartridge.frames, recreation.frames, 0, what);
        }
    }
}

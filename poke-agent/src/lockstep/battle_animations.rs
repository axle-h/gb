//! The battle with its animations played on both sides: the transition, the silhouettes' slide,
//! the moves, the throw, compared on the LCD a frame at a time.
//!
//! The recreation reaches the screen whole where the cartridge moves a third of the tile map a
//! frame, writes registers partway down a frame and lags drawing frame blocks, so frames are compared
//! as runs rather than one to one, and only while an animation plays, since text printed a letter a
//! frame is never whole on the cartridge's screen. Every picture the recreation shows must arrive on
//! the cartridge in the same order and no earlier, a cartridge frame counting where each of its lines
//! is that picture's or a neighbour's; and every picture the cartridge holds for three frames or more
//! in between must be one the recreation shows. A change to the tile map alone may never reach
//! the cartridge's screen whole if it lasts fewer than three frames, or three with a register written
//! next, which lands in the frame the last third would have shown. A picture held for fewer is a frame
//! caught between two, one of a single shade is the screen blanked while it loads, and one held only while a
//! routine in `LOADING` runs is the cartridge moving data the recreation leaves out.

use gb::game_boy::GameBoy;
use gb::ram::ROM;
use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use pokered::gfx::layers::{MapLayer, Object};
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::{Game, Input};
use crate::pokemon::map_header::TileSetId;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointerRead};
use super::battle::{recreation_with, cartridge_oam, letters, policy, Cartridge, Lead, Opponent,
    BACK_PICTURE, CLEAR_SCREEN, MOVE_AND_BAR, PICTURES, SILHOUETTES};
use super::{assert_late, ARROW, BOX, CURSOR, DELAY3};

const MAP_BORDER: usize = 3;
const WIDTH: usize = 160;

pub(super) fn lcd_shades(gb: &GameBoy) -> Vec<u8> {
    gb.core().mmu().ppu().screenshot().pixels().map(|pixel| match pixel.0[0] {
        0xFF => 0,
        0xAA => 1,
        0x55 => 2,
        _ => 3,
    }).collect()
}

/// Routines whose frames are the cartridge moving data, not pacing: a picture held while one runs
/// is loading.
const LOADING: [&str; 9] = ["CopyVideoData", "CopyVideoDataDouble", "BattleAnimCopyTileMapToVRAM", "UncompressSpriteData",
    "_UncompressSpriteData", "UncompressSpriteDataLoop", "InterlaceMergeSpriteBuffers", "LoadMonFrontSprite", "ScaleSpriteByTwo"];

/// Each bank's labels, sorted, without their local parts.
fn labels() -> &'static std::collections::HashMap<u8, Vec<(u16, String)>> {
    static LABELS: std::sync::OnceLock<std::collections::HashMap<u8, Vec<(u16, String)>>> = std::sync::OnceLock::new();
    LABELS.get_or_init(|| {
        let mut banks: std::collections::HashMap<u8, Vec<(u16, String)>> = Default::default();
        for line in include_str!("../../../vendor/pokered/pokered.sym").lines() {
            let Some((at, name)) = line.split_once(' ') else { continue };
            let Some((bank, address)) = at.split_once(':') else { continue };
            let (Ok(bank), Ok(address)) = (u8::from_str_radix(bank, 16), u16::from_str_radix(address, 16)) else { continue };
            banks.entry(bank).or_default().push((address, name.split('.').next().unwrap_or(name).to_string()));
        }
        for labels in banks.values_mut() {
            labels.sort();
        }
        banks
    })
}

fn routine_at(bank: u8, address: u16) -> Option<&'static str> {
    let labels = labels().get(&bank)?;
    let at = labels.partition_point(|(a, _)| *a <= address).checked_sub(1)?;
    Some(labels[at].1.as_str())
}

/// Whether the cartridge, stopped at a VBlank, was inside a loading routine: the code it
/// interrupted, or any return address on the stack above it.
pub(super) fn loading(gb: &GameBoy) -> bool {
    let mmu = gb.core().mmu();
    let bank = mmu.read(sym::hLoadedROMBank.address);
    let sp = gb.core().registers().sp;
    if std::env::var("LOCKSTEP_STACKS").is_ok() {
        let names: Vec<String> = (0..12u16).map(|i| mmu.read_u16_le(sp.wrapping_add(2 * i))).filter(|&a| a < 0x8000)
            .map(|a| format!("{}", routine_at(if a < 0x4000 { 0 } else { bank }, a).unwrap_or("?"))).collect();
        println!("    stack {}", names.join(" < "));
    }
    (0..24u16).map(|i| mmu.read_u16_le(sp.wrapping_add(2 * i))).filter(|&address| address < 0x8000).any(|address| {
        let bank = if address < 0x4000 { 0 } else { bank };
        routine_at(bank, address).is_some_and(|name| LOADING.contains(&name))
    })
}

/// The overworld the fixture stands in, as the recreation's screen: VRAM's tiles, the tile map over
/// the map buffer and view, the shadow OAM and the palettes.
pub(super) fn seed_screen(gb: &GameBoy, game: &mut Game) {
    let mmu = gb.core().mmu();
    let screen = game.screen_mut();
    screen.tiles.load(0, mmu.read_vram_slice(0x8000, 384 * 16).unwrap());
    let tileset = TileSetId::from_repr(mmu.read_pointer(&sym::wCurMapTileset)).unwrap();
    let blocks_wide = mmu.read_pointer(&sym::wCurMapWidth) as usize + 2 * MAP_BORDER;
    let blocks_high = mmu.read_pointer(&sym::wCurMapHeight) as usize + 2 * MAP_BORDER;
    let view = mmu.read_pointer_u16_le(&sym::wCurrentTileBlockMapViewPointer) - sym::wOverworldMap.address;
    let (x_half, y_half) = (mmu.read_pointer(&sym::wXBlockCoord), mmu.read_pointer(&sym::wYBlockCoord));
    screen.map = MapLayer {
        tileset: Some(tileset),
        blocks_wide,
        blocks: mmu.read_pointer_vec(&sym::wOverworldMap, blocks_wide * blocks_high),
        camera: (
            (view as usize % blocks_wide) as i32 * 32 + x_half as i32 * 16,
            (view as usize / blocks_wide) as i32 * 32 + y_half as i32 * 16,
        ),
    };
    for (y, row) in (0..18).map(|y| super::tile_row(gb, y)).enumerate() {
        for (x, &tile) in row.iter().enumerate() {
            screen.ui.set(x, y, tile);
        }
    }
    screen.sprites = cartridge_oam(gb).iter().map(|&[y, x, tile, attributes]| Object { y, x, tile, attributes }).collect();
    screen.effects.bgp = mmu.read(0xFF47);
    screen.effects.obp0 = mmu.read(0xFF48);
    screen.effects.obp1 = mmu.read(0xFF49);
}

/// A picture held for `len` frames from `first`.
struct Run<'a> {
    image: &'a [u8],
    first: usize,
    len: usize,
}

fn runs(frames: &[Vec<u8>]) -> Vec<Run<'_>> {
    let mut runs: Vec<Run> = vec![];
    for (i, frame) in frames.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if run.image == frame.as_slice() => run.len += 1,
            _ => runs.push(Run { image: frame, first: i, len: 1 }),
        }
    }
    runs
}

fn dump(what: &str, name: &str, image: &[u8]) {
    let Ok(dir) = std::env::var("LOCKSTEP_DUMP") else { return };
    let img = image::GrayImage::from_fn(WIDTH as u32, 144, |x, y| image::Luma([255 - 85 * image[y as usize * WIDTH + x as usize]]));
    let path = format!("{dir}/{}-{name}.png", what.replace(' ', "_"));
    img.save(&path).unwrap();
}

/// Whether a cartridge frame shows `current` arriving: every line is `current`'s, or the picture
/// before or after it, or `late_objects`, the picture after with `current`'s objects; and at least one
/// is a line of `current` the picture before lacks, or of `late_objects` that neither neighbour has.
/// A register written in the middle of a frame or a transfer caught between thirds leaves a frame
/// that is part one picture and part the next, and OAM reaches the screen a frame after the palettes.
fn shows(frame: &[u8], previous: Option<&[u8]>, current: &[u8], next: Option<&[u8]>, late_objects: Option<&[u8]>) -> bool {
    if frame == current {
        return true;
    }
    fn line(image: &[u8], y: usize) -> &[u8] {
        &image[y * WIDTH..(y + 1) * WIDTH]
    }
    let mut arrived = false;
    for y in 0..144 {
        let theirs = line(frame, y);
        let is_current = theirs == line(current, y);
        let is_previous = previous.is_some_and(|previous| theirs == line(previous, y));
        let is_next = next.is_some_and(|next| theirs == line(next, y));
        let is_late = late_objects.is_some_and(|late| theirs == line(late, y));
        if !is_current && !is_previous && !is_next && !is_late {
            // A register written while the line is being drawn splits the line itself.
            let candidates: Vec<&[u8]> = [Some(current), previous, next, late_objects].into_iter().flatten().map(|image| line(image, y)).collect();
            let split = (1..WIDTH).any(|k| candidates.iter().any(|a| a[..k] == theirs[..k]) && candidates.iter().any(|b| b[k..] == theirs[k..]));
            if !split {
                return false;
            }
        }
        arrived |= (is_current && !is_previous) || (is_late && !is_previous && !is_next);
    }
    arrived
}

/// A frame of `AnimationWavyScreen`: a hidden window, and lines scrolled back as well as forward.
fn waving(screen: &pokered::gfx::Screen) -> bool {
    screen.background.is_some() && screen.effects.line_scx.as_ref().is_some_and(|lines| lines.iter().any(|&scx| scx == 0xFF) && lines.iter().any(|&scx| scx == 1))
}

/// A waving frame as the scroll that produced it: the line writes start at and the table entry
/// they start from, each line two entries on, with one scroll above. The cartridge's first line
/// moves with how long its VBlank handler ran, which is timing, so frames are matched to the model
/// rather than to each other.
fn wave_fit(frame: &[u8], base: &[u8]) -> Option<(usize, usize)> {
    const OFFSETS: [i8; 32] = [0, 0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 2, 2, 1, 1, 1, 0, 0, 0, 0, 0, -1, -1, -1, -2, -2, -2, -2, -2, -1, -1, -1];
    let fits: Vec<u16> = (0..144).map(|y| {
        (-4i32..=4).enumerate().filter(|&(_, shift)| {
            (0..WIDTH).all(|x| {
                let bx = (x as i32 + shift).rem_euclid(256) as usize;
                let shade = if bx < WIDTH { base[y * WIDTH + bx] } else { 0 };
                frame[y * WIDTH + x] == shade
            })
        }).fold(0u16, |bits, (i, _)| bits | 1 << i)
    }).collect();
    let bit = |scx: i8| 1u16 << (scx as i32 + 4);
    (0..=24).find_map(|first| {
        let above = (0..first).fold(0x1FF, |bits, y| bits & fits[y]);
        if first > 0 && above == 0 {
            return None;
        }
        (0..32).find(|&entry| (first..144).all(|y| fits[y] & bit(OFFSETS[(entry + 2 * (y - first)) % 32]) != 0))
            .map(|entry| (first, entry))
    })
}

/// Waving frames the cartridge may draw off the model, where a pass ran long enough to miss an
/// HBlank.
const MAX_UNFIT_WAVE_FRAMES: usize = 4;

/// A stretch of waving frames: every one the recreation shows fits the model, and the cartridge
/// shows as many, give or take one, from `at`. Where the cartridge's last waving frame is.
fn compare_wave(cartridge: &[Vec<u8>], ours: &[pokered::gfx::Screen], at: usize, what: &str) -> usize {
    let mut unscrolled = ours[0].clone();
    unscrolled.effects.line_scx = None;
    unscrolled.effects.scx = 0;
    let base = unscrolled.frame().shades;
    for (i, screen) in ours.iter().enumerate() {
        assert!(wave_fit(&screen.frame().shades, &base).is_some(), "{what}: the recreation's waving frame {i} fits no wave");
    }
    let first = (at..cartridge.len()).find(|&i| wave_fit(&cartridge[i], &base).is_some())
        .unwrap_or_else(|| panic!("{what}: the cartridge never waves after frame {at}"));
    let window = first..(first + ours.len() + 1).min(cartridge.len());
    let fitting: Vec<usize> = window.clone().filter(|&i| wave_fit(&cartridge[i], &base).is_some()).collect();
    let last = *fitting.last().expect("the first fits");
    let span = last + 1 - first;
    let unfit = span - fitting.len();
    println!("  {what}: the cartridge waves {span} frames from frame {first}, {unfit} of them off the model; the recreation {}", ours.len());
    assert!(span.abs_diff(ours.len()) <= 1 && unfit <= MAX_UNFIT_WAVE_FRAMES,
        "{what}: the cartridge waves {span} frames from frame {first}, {unfit} of them off the model, and the recreation {}", ours.len());
    last
}

/// Whether two screens differ in the tile map alone: `AutoBgMapTransfer` may take three VBlanks to
/// send the thirds such a change touches, so it can be overwritten before it is seen.
fn only_the_tile_map_moved(before: &pokered::gfx::Screen, after: &pokered::gfx::Screen) -> bool {
    let mut after = after.clone();
    after.ui = before.ui.clone();
    &after == before
}

fn differing(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(a, b)| a != b).count()
}

/// The two checks over the frames the recreation spent animating, a stretch at a time: `animating`
/// marks each recreation frame that ended with an animation still playing.
fn compare(captured: &[(Vec<u8>, bool)], screens: &[pokered::gfx::Screen], animating: &[bool], what: &str) -> Vec<i64> {
    let recreation: Vec<Vec<u8>> = screens.iter().map(|screen| screen.frame().shades).collect();
    let recreation = recreation.as_slice();
    let cartridge: Vec<Vec<u8>> = captured.iter().map(|(shades, _)| shades.clone()).collect();
    let cartridge = cartridge.as_slice();
    let theirs = runs(cartridge);
    if std::env::var("LOCKSTEP_DUMP_ALL").is_ok() {
        for run in &theirs {
            dump(what, &format!("all-cartridge-{:04}-x{}", run.first, run.len), run.image);
        }
        for run in runs(recreation) {
            dump(what, &format!("all-recreation-{:04}-x{}", run.first, run.len), run.image);
        }
    }
    let mut lateness = vec![];
    let mut at = 0;
    let mut frame = 0;
    while frame < recreation.len() {
        if !animating[frame] {
            frame += 1;
            continue;
        }
        let end = (frame..recreation.len()).find(|&i| !animating[i]).unwrap_or(recreation.len());
        let ours = runs(&recreation[frame..end]);
        let mut start = None;
        let mut wave_done = None;
        for (n, run) in ours.iter().enumerate() {
            let first = frame + run.first;
            if waving(&screens[first]) {
                if wave_done.is_some_and(|done| first < done) {
                    continue;
                }
                let done = (first..end).find(|&i| !waving(&screens[i])).unwrap_or(end);
                at = compare_wave(cartridge, &screens[first..done], at, what);
                start.get_or_insert(at);
                wave_done = Some(done);
                continue;
            }
            let last = first + run.len - 1;
            let previous = first.checked_sub(1).map(|i| recreation[i].as_slice());
            let next = recreation.get(last + 1).map(Vec::as_slice);
            let late_objects = screens.get(last + 1).map(|after| {
                let mut hybrid = after.clone();
                hybrid.sprites = screens[last].sprites.clone();
                hybrid.frame().shades
            });
            let may_be_unseen = first > 0 && only_the_tile_map_moved(&screens[first - 1], &screens[first])
                && (run.len < 3 || run.len == 3 && screens.get(last + 1).is_some_and(|after| !only_the_tile_map_moved(&screens[last], after)));
            // An unseen picture is matched only by a later repeat of it, after the next picture arrived.
            let next_arrives_before = |before: usize| ours.get(n + 1).is_some_and(|following| {
                let following_last = frame + following.first + following.len - 1;
                let after_following = recreation.get(following_last + 1).map(Vec::as_slice);
                (at..before).any(|i| shows(&cartridge[i], Some(run.image), following.image, after_following, None))
            });
            match (at..cartridge.len()).find(|&i| shows(&cartridge[i], previous, run.image, next, late_objects.as_deref())) {
                Some(i) if may_be_unseen && next_arrives_before(i) => {}
                Some(i) => {
                    lateness.push(i as i64 - first as i64);
                    at = i;
                    start.get_or_insert(i);
                }
                None if may_be_unseen => {}
                None => {
                    let nearest = (0..cartridge.len()).min_by_key(|&i| differing(&cartridge[i], run.image)).unwrap_or(0);
                    if let Some(previous) = previous {
                        dump(what, &format!("recreation-previous-frame-{}", first - 1), previous);
                    }
                    if let Some(next) = next {
                        dump(what, &format!("recreation-next-frame-{}", last + 1), next);
                    }
                    if let Some(late) = &late_objects {
                        dump(what, "recreation-late-objects", late);
                    }
                    for i in at.saturating_sub(2)..(at + 8).min(cartridge.len()) {
                        dump(what, &format!("cartridge-around-{i}"), &cartridge[i]);
                    }
                    dump(what, &format!("recreation-frame-{first}"), run.image);
                    dump(what, &format!("cartridge-nearest-frame-{nearest}"), &cartridge[nearest]);
                    panic!("{what}: the recreation's picture at frame {first} ({} frames, picture {n} of an animation from frame \
                            {frame}) is not on the cartridge after frame {at}; nearest is frame {nearest}, {} pixels off",
                        run.len, differing(&cartridge[nearest], run.image));
                }
            }
        }
        let blank = |image: &[u8]| image.iter().all(|&shade| shade == image[0]);
        let mut within = frame;
        let start = start.unwrap_or(at);
        // A frame shows what the code before its VBlank left, and a loading routine's caller goes on
        // to write the tile map, which `AutoBgMapTransfer` takes three VBlanks to send: the loading
        // picture is still up for three frames after the last loading one.
        let loaded = |run: &Run| (run.first..run.first + run.len)
            .all(|i| (i.saturating_sub(3)..=i).any(|before| captured[before].1));
        for run in theirs.iter().filter(|run| run.first > start && run.first < at && run.len >= 3 && !blank(run.image) && !loaded(run)) {
            match (within..end).find(|&i| recreation[i] == run.image) {
                Some(i) => within = i,
                None => {
                    let nearest = (frame..end).min_by_key(|&i| differing(&recreation[i], run.image)).unwrap_or(frame);
                    dump(what, &format!("cartridge-frame-{}", run.first), run.image);
                    dump(what, &format!("recreation-nearest-frame-{nearest}"), &recreation[nearest]);
                    panic!("{what}: the cartridge's picture at frame {} (held {} frames) is not in the recreation's animation \
                            from frame {frame} after frame {within}; nearest is frame {nearest}, {} pixels off",
                        run.first, run.len, differing(&recreation[nearest], run.image));
                }
            }
        }
        frame = end;
    }
    lateness
}

/// Both sides through a battle with animations, pressing what `choose` picks from the cartridge's
/// screen, the LCD compared between every poll and the tile map at each; the frames each took.
fn animated_battle(opponent: Opponent, lead: Lead, choose: impl Fn(&[Vec<u8>], usize) -> Joypad) -> Vec<(u32, u32)> {
    animated_battle_to(opponent, lead, choose, None)
}

/// `animated_battle`, compared to its poll `polls` rather than to its end.
fn animated_battle_to(opponent: Opponent, lead: Lead, choose: impl Fn(&[Vec<u8>], usize) -> Joypad, polls: Option<usize>) -> Vec<(u32, u32)> {
    animated_battle_timed(opponent, lead, choose, polls, &[])
}

/// `animated_battle_to`, with each poll's frames held to the recreation's plus the loading named for
/// it. A poll past the end of `loadings` is compared as pictures alone.
fn animated_battle_timed(opponent: Opponent, lead: Lead, choose: impl Fn(&[Vec<u8>], usize) -> Joypad, polls: Option<usize>,
    loadings: &[u32]) -> Vec<(u32, u32)> {
    let mut cartridge = Cartridge::animated(opponent, lead);
    let mut presses = vec![];
    // Two polls past the last one replayed: the recreation reaches a turn's random bytes in the two
    // frames after the last press, where the cartridge is still printing the text before them.
    let taped = polls.map(|polls| polls + 2);
    while let Some(poll) = cartridge.to_poll() {
        if taped == Some(presses.len()) {
            break;
        }
        let button = choose(&poll.screen, presses.len());
        presses.push(button);
        cartridge.press(button);
        assert!(presses.len() < 400, "the battle goes on");
    }
    let ended = cartridge.ended && polls.is_none();
    let tape = std::mem::take(&mut cartridge.tape);
    presses.truncate(polls.unwrap_or(presses.len()));

    let mut cartridge = Cartridge::animated(opponent, lead);
    let mut game = recreation_with(&cartridge.gb, opponent, lead, tape, false);
    let mut frames = vec![];
    for (poll, &button) in presses.iter().enumerate() {
        cartridge.lcd = Some(vec![]);
        let theirs = cartridge.to_poll().expect("polls");
        let cartridge_frames = cartridge.lcd.take().expect("collected");
        let mut ours = vec![];
        let mut animating = vec![];
        let recreation = loop {
            game.frame(Input::None);
            ours.push(game.screen().clone());
            animating.push(game.modes().iter().any(|mode| matches!(mode, Mode::Battle(battle) if battle.animating())));
            if let pokered::mode::Status::Waiting(_) = game.status() {
                break ours.len();
            }
            assert!(ours.len() < 5000, "the recreation never waited");
        };
        let what = format!("poll {poll}");
        println!("{what}: cartridge {} frames, recreation {recreation}", theirs.frames);
        let lateness = compare(&cartridge_frames, &ours, &animating, &what);
        println!("  lateness {:?}", lateness);
        let mine: Vec<Vec<u8>> = (0..18).map(|y| game.ui().row(y).to_vec()).collect();
        if theirs.screen != mine {
            for (a, b) in theirs.screen.iter().zip(&mine) {
                println!("  |{}|  |{}|", letters(a), letters(b));
            }
        }
        assert_eq!(theirs.screen, mine, "the tile map at poll {poll}");
        if let Some(&loading) = loadings.get(poll) {
            assert_late(theirs.frames, recreation as u32, loading, &what);
        }
        frames.push((theirs.frames, recreation as u32));
        game.frame(Input::Buttons(button));
        cartridge.press(button);
        game.frame(Input::None);
    }
    if !ended {
        return frames;
    }
    assert!(cartridge.to_poll().is_none(), "the cartridge's battle ends");
    for _ in 0..600 {
        if !game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))) {
            break;
        }
        game.frame(Input::None);
    }
    assert!(!game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))), "the recreation's battle ends");
    frames
}

fn only(first: PokemonMoveName) -> Option<[u8; 4]> {
    Some([first as u8, 0, 0, 0])
}

fn animated(moves: Option<[u8; 4]>) -> Lead {
    Lead { moves, animations: Some(true), ..Lead::default() }
}

/// `VIRIDIAN_FOREST`, in `DungeonMaps1`.
const VIRIDIAN_FOREST: u8 = 0x33;
const LANCE: u8 = 0x2F;

/// `BattleTransition`'s `LoadBattleTransitionTiles` and the screen it copies into `vBGMap`,
/// measured; the same for all seven, which differ only in the order they blank it.
const TRANSITION: u32 = 10;
/// `LoadAnimationTileset`'s `CopyVideoData` of one animation's tiles into `vSprites`, measured. An
/// animation the OPTION menu turned off loads nothing and delays instead.
const ANIMATION_TILESET: u32 = 10;
/// The opening: the transition, `LoadHudAndHpBarAndStatusTilePatterns` with both pictures, the
/// silhouettes and the appeared text.
const OPENING: u32 = TRANSITION + CLEAR_SCREEN / 2 + PICTURES + SILHOUETTES + BOX + ARROW;
/// `_LoadTrainerPic` and a trainer's picture decompressed behind the played transition, measured:
/// a YOUNGSTER's, and the champion's, which is the taller picture.
const YOUNGSTER_PICTURE: u32 = 77;
const CHAMPION_PICTURE: u32 = 89;

/// The opening against a trainer, whose picture is decompressed in place of the wild mon's.
const fn trainer_opening(picture: u32) -> u32 {
    TRANSITION + CLEAR_SCREEN / 2 + picture + SILHOUETTES + BOX + ARROW
}
/// `Go! Celina!`, the back picture, and the battle menu over an empty text box.
const SEND_OUT: u32 = BOX + CLEAR_SCREEN + BOX + BACK_PICTURE + BOX + BOX + CURSOR;

/// The same wild battle as `a_wild_battle_won_with_one_move_matches_the_cartridge`, timed against
/// the same loading with the transition and the send-out's poof added.
#[test]
fn a_wild_battle_won_with_tackle_animates_as_the_cartridge_does() {
    let loadings = [
        OPENING,
        SEND_OUT + ANIMATION_TILESET,
        // FIGHT: the move menu.
        DELAY3 + CURSOR,
        // TACKLE, the bar, the victory music's `Delay3`, and Enemy PIDGEY fainted!
        BOX + MOVE_AND_BAR + DELAY3 + BOX + ARROW,
        // The empty text after the faint, and Celina gained 23 EXP. Points!
        BOX + BOX + ARROW,
    ];
    let frames = animated_battle_timed(Opponent::Wild(PokemonSpecies::Pidgey, 3), animated(only(PokemonMoveName::Tackle)),
        policy, None, &loadings);
    println!("{frames:?}");
}

#[test]
fn the_same_battle_with_animations_off_shakes_and_blinks_as_the_cartridge_does() {
    let lead = Lead { animations: Some(false), ..animated(only(PokemonMoveName::Tackle)) };
    let loadings = [OPENING, SEND_OUT, DELAY3 + CURSOR, BOX + MOVE_AND_BAR + DELAY3 + BOX + ARROW, BOX + BOX + ARROW];
    animated_battle_timed(Opponent::Wild(PokemonSpecies::Pidgey, 3), lead, policy, None, &loadings);
}

/// Each of the seven transitions, to the battle's first poll: a wild mon three levels up or not, a
/// trainer three levels up or not, in the city and in a dungeon.
#[test]
fn every_battle_transition_matches_the_cartridge() {
    let dungeon = |lead: Lead| Lead { map: Some(VIRIDIAN_FOREST), ..lead };
    let youngster = trainer_opening(YOUNGSTER_PICTURE);
    let champion = trainer_opening(CHAMPION_PICTURE);
    let cases = [
        ("the circle", Opponent::Wild(PokemonSpecies::Pidgey, 60), animated(None), OPENING),
        ("the inward spiral", Opponent::Trainer(super::battle::YOUNGSTER, 1), animated(None), youngster),
        ("the outward spiral", Opponent::Trainer(LANCE, 1), animated(None), champion),
        ("the horizontal stripes", Opponent::Wild(PokemonSpecies::Pidgey, 3), dungeon(animated(None)), OPENING),
        ("the vertical stripes", Opponent::Wild(PokemonSpecies::Pidgey, 60), dungeon(animated(None)), OPENING),
        ("the shrink", Opponent::Trainer(super::battle::YOUNGSTER, 1), dungeon(animated(None)), youngster),
        ("the split", Opponent::Trainer(LANCE, 1), dungeon(animated(None)), champion),
    ];
    for (what, opponent, lead, opening) in cases {
        println!("{what}");
        animated_battle_timed(opponent, lead, policy, Some(1), &[opening]);
    }
}

/// Psychic: the long flash, then every line of the screen scrolled by the wave.
#[test]
fn psychic_waves_the_screen_as_the_cartridge_does() {
    animated_battle_to(Opponent::Wild(PokemonSpecies::Snorlax, 40), animated(only(PokemonMoveName::Psychic)), policy, Some(5));
}

/// A Poké Ball that breaks free after its shakes, then a Master Ball that holds.
#[test]
fn a_ball_that_breaks_free_and_one_that_catches_animate_as_the_cartridge_does() {
    use poke_core::item::ItemId;
    let used = std::rc::Rc::new(std::cell::Cell::new(0));
    let lead = Lead { bag: Some(super::battle::BATTLE_BAG), ..animated(None) };
    animated_battle(Opponent::Wild(PokemonSpecies::Chansey, 30), lead,
        super::battle::using(&[ItemId::PokeBall, ItemId::PokeBall, ItemId::MasterBall], used));
}

/// A trainer knocks the ball away.
#[test]
fn a_ball_thrown_at_a_trainer_is_blocked_as_the_cartridge_does() {
    use poke_core::item::ItemId;
    let used = std::rc::Rc::new(std::cell::Cell::new(0));
    let lead = Lead { bag: Some(super::battle::BATTLE_BAG), ..animated(None) };
    animated_battle_to(Opponent::Trainer(super::battle::YOUNGSTER, 1), lead, super::battle::using(&[ItemId::PokeBall], used), Some(8));
}

/// Substitute up, the doll hit and put back, and broken.
#[test]
fn a_substitute_hides_reshows_and_breaks_as_the_cartridge_does() {
    let moves = Some([PokemonMoveName::Substitute as u8, PokemonMoveName::Tackle as u8, 0, 0]);
    animated_battle_to(Opponent::Wild(PokemonSpecies::Snorlax, 40), animated(moves), policy, Some(24));
}

/// The lead splashes, so every attack is the enemy's: its subanimations take their own transform
/// where the player's take none, an enemy-type one plays unflipped, and the effects that name a side
/// swap over.
fn the_enemy_attacking(species: PokemonSpecies, level: u8, polls: usize) {
    animated_battle_to(Opponent::Wild(species, level), animated(only(PokemonMoveName::Splash)), policy, Some(polls));
}

/// Wild mons chosen for their movesets: between them the enemy plays a subanimation of each
/// transform and the effects that swap sides.
#[test]
fn the_enemy_s_moves_animate_as_the_cartridge_does() {
    let cases = [
        (PokemonSpecies::Pidgeotto, 30),
        (PokemonSpecies::Shellder, 34),
        (PokemonSpecies::Zubat, 22),
        (PokemonSpecies::Krabby, 30),
        (PokemonSpecies::Ponyta, 48),
        (PokemonSpecies::Kadabra, 30),
    ];
    for (species, level) in cases {
        println!("{species:?} at level {level}");
        the_enemy_attacking(species, level, 12);
    }
}

/// Every species as the enemy, attacking a lead that only splashes.
#[test]
#[cfg(feature = "slow-tests")]
fn every_species_s_moves_animate_from_the_enemy_side_as_the_cartridge_does() {
    let mut failed = vec![];
    for species in (1..=255).filter_map(PokemonSpecies::from_repr) {
        let result = std::panic::catch_unwind(|| the_enemy_attacking(species, 50, 10));
        if let Err(error) = result {
            let message = error.downcast_ref::<String>().cloned().unwrap_or_default();
            println!("FAILED {species:?}: {}", message.lines().next().unwrap_or(""));
            failed.push(species);
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}

/// Each non-volatile status on the lead: the cartridge animates it before the mon's turn, and a
/// frozen or sleeping one never gets a turn at all.
#[test]
fn a_status_on_the_lead_animates_as_the_cartridge_does() {
    let cases = [("poison", 1 << 3), ("burn", 1 << 4), ("freeze", 1 << 5), ("paralysis", 1 << 6), ("sleep", 3)];
    for (what, status) in cases {
        println!("{what}");
        let lead = Lead { status: Some(status), ..animated(only(PokemonMoveName::Tackle)) };
        animated_battle_to(Opponent::Wild(PokemonSpecies::Pidgey, 3), lead, policy, Some(8));
    }
}

/// Confuse Ray and Toxic on the enemy: the confusion and the poison animate on its side before each
/// of its turns.
#[test]
fn confusion_and_poison_on_the_enemy_animate_as_the_cartridge_does() {
    let moves = Some([PokemonMoveName::ConfuseRay as u8, PokemonMoveName::Toxic as u8, 0, 0]);
    animated_battle_to(Opponent::Wild(PokemonSpecies::Snorlax, 40), animated(moves), policy, Some(14));
}

/// Sludge on the enemy until it poisons: a side effect's status is `ENEMY_HUD_SHAKE_ANIM`, which
/// holds the screen under the enemy's HUD still in the window while the background, the player's
/// picture lifted out of it into OAM, shakes two pixels either way.
#[test]
fn a_side_effect_s_status_shakes_the_enemy_s_hud_as_the_cartridge_does() {
    animated_battle_to(Opponent::Wild(PokemonSpecies::Snorlax, 40), animated(only(PokemonMoveName::Sludge)), policy, Some(24));
}

/// `POKEMON_TOWER_6F`, the only floor whose ghost is the restless soul.
const POKEMON_TOWER_6F: u8 = 0x93;

/// `LoadHudAndHpBarAndStatusTilePatterns` with the ghost's picture and the back picture, measured.
const GHOST_PICTURES: u32 = 70;

/// The SILPH SCOPE, so the ghost is unveiled; the ball is first so the bag opens on it.
const GHOST_BAG: &[(poke_core::item::ItemId, u8)] =
    &[(poke_core::item::ItemId::PokeBall, 5), (poke_core::item::ItemId::SilphScope, 1)];

/// `MarowakAnim`: the ghost's picture flashes, fades out as sprites and Marowak fades back in; then
/// a Poké Ball the restless soul dodges.
#[test]
fn the_ghost_unveiled_as_marowak_and_dodging_a_ball_animates_as_the_cartridge_does() {
    use poke_core::item::ItemId;
    let used = std::rc::Rc::new(std::cell::Cell::new(0));
    let lead = Lead { map: Some(POKEMON_TOWER_6F), bag: Some(GHOST_BAG), ..animated(None) };
    // The opening, where `LoadGhostPic` copies an uncompressed picture in place of a decompressed one.
    let loadings = [TRANSITION + CLEAR_SCREEN / 2 + GHOST_PICTURES + SILHOUETTES + BOX + ARROW];
    animated_battle_timed(Opponent::Wild(PokemonSpecies::Marowak, 30), lead,
        super::battle::using(&[ItemId::PokeBall], used.clone()), Some(12), &loadings);
    assert_eq!(used.get(), 1, "the ball was thrown");
}

/// Rocks and bait in the Safari Zone, and the mon sliding away when it runs.
#[test]
fn rocks_bait_and_a_mon_running_in_the_safari_zone_animate_as_the_cartridge_does() {
    let lead = Lead { map: Some(super::battle::SAFARI_ZONE_EAST), ..animated(None) };
    animated_battle(Opponent::Wild(PokemonSpecies::Tauros, 25), lead, super::battle::safari_choices(&[1, 2, 1, 2, 2, 2, 2, 2, 2, 2]));
}

#[test]
#[ignore = "a probe: SPECIES at LEVEL attacking a lead that only splashes, to POLLS polls"]
fn probe_enemy_animations() {
    let var = |name: &str| std::env::var(name).ok();
    let species = var("SPECIES").map_or(PokemonSpecies::Tauros, |name| {
        (1..=255).filter_map(PokemonSpecies::from_repr).find(|s| format!("{s:?}") == name).expect("a species")
    });
    let level = var("LEVEL").map_or(50, |level| level.parse().unwrap());
    let polls = var("POLLS").map_or(10, |polls| polls.parse().unwrap());
    the_enemy_attacking(species, level, polls);
}

#[test]
#[ignore = "a probe: the player's lead with only MOVES (ids, comma separated) against SPECIES at LEVEL, to POLLS polls each"]
fn probe_move_animations() {
    let var = |name: &str| std::env::var(name).ok();
    let species = var("SPECIES").map_or(PokemonSpecies::Snorlax, |name| {
        (1..=255).filter_map(PokemonSpecies::from_repr).find(|s| format!("{s:?}") == name).expect("a species")
    });
    let level = var("LEVEL").map_or(40, |level| level.parse().unwrap());
    let polls = var("POLLS").map_or(14, |polls| polls.parse().unwrap());
    let animations = var("OFF").is_none();
    let mut failed = vec![];
    for id in var("MOVES").expect("MOVES").split(',') {
        let id: u8 = id.trim().parse().unwrap();
        let lead = Lead { moves: Some([id, 0, 0, 0]), animations: Some(animations), ..Lead::default() };
        let result = std::panic::catch_unwind(|| animated_battle_to(Opponent::Wild(species, level), lead, policy, Some(polls)));
        if let Err(error) = result {
            let message = error.downcast_ref::<String>().cloned().or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            println!("FAILED {:?}: {}", PokemonMoveName::from_repr(id), message.lines().next().unwrap_or(""));
            failed.push(id);
        }
    }
    println!("{} failed: {failed:?}", failed.len());
}

/// Every move's animation, the player's lead knowing only it, to the poll after its first use.
#[test]
#[cfg(feature = "slow-tests")]
fn every_move_s_animation_matches_the_cartridge() {
    let mut failed = vec![];
    for id in 1..=PokemonMoveName::Struggle as u8 {
        let result = std::panic::catch_unwind(|| {
            animated_battle_to(Opponent::Wild(PokemonSpecies::Snorlax, 40), animated(Some([id, 0, 0, 0])), policy, Some(8))
        });
        if let Err(error) = result {
            let message = error.downcast_ref::<String>().cloned().unwrap_or_default();
            println!("FAILED {:?}: {}", PokemonMoveName::from_repr(id), message.lines().next().unwrap_or(""));
            failed.push(PokemonMoveName::from_repr(id));
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}

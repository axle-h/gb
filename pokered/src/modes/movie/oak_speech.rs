//! `OakSpeech` (`oak_speech.asm`, `oak_speech2.asm`, `init_player_data.asm`): the new game's world,
//! Oak, Nidorino, the player's name and the rival's, and the player shrinking into the overworld.
//!
//! A name is chosen from Oak's list or typed on the naming screen; either way it lands in
//! `wPlayerName` or `wRivalName` before the text that says it.
//!
//! Loading, and not modelled: `ClearScreen`'s `Delay3` and the one after the naming screen's, and
//! the pictures' decompression and `CopyVideoData`. Kept as pacing: the slides' `Delay3` a column
//! (they are the animation), and the `Delay3` between a preset name's 10-frame hold and its slide.

use poke_core::default_names::{player_names, rival_names};
use poke_core::item::ItemId;
use poke_core::rom_gfx::rom_slice;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::{pokered_symbols as sym, DmgPointer};
use poke_core::text_script::{decode, TextBuffer};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, AudioBank, SoundId};
use crate::gfx::layers::{TileMap, Window};
use crate::gfx::tiles::V_CHARS0;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, Transition};
use crate::modes::menu_input::MenuInput;
use crate::modes::naming_screen::{NamingScreen, NamingScreenType};
use crate::modes::text_box::TextBox;
use crate::rng::Rng;
use crate::systems::inventory::Inventory;
use crate::systems::overworld::sprites::{prepare_oam, SpriteState, Sprites, NUM_SPRITES};
use crate::world::World;
use super::intro::{fade_in_from_white, fade_out_to_white, place_string_lines, FADE_STEP, FADE_STEPS};
use super::screen::{clear_screen, copy_pic_to_tile_map, load_mon_pic, load_pic, WX_LEFT};
use super::wait::{Tick, Wait};

/// `START_MONEY`, BCD.
const START_MONEY: [u8; 3] = [0x00, 0x30, 0x00];
/// `FadeInIntroPic`'s frames a palette.
const INTRO_FADE_STEP: u16 = 10;
/// `MovePicLeft`'s first `rWX`: the picture's left edge 112 pixels in.
const MOVE_PIC_LEFT_WX: u8 = 119;
/// `OakSpeechSlidePicCommon`'s `d` and `e`: six columns, over a 125-tile run of the tile map.
const SLIDE_COLUMNS: u8 = 6;
const SLIDE_REGION: usize = 6 * SCREEN_TILES_X + 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Who {
    Player,
    Rival,
}

impl Who {
    fn pic(self) -> DmgPointer {
        match self {
            Who::Player => sym::RedPicFront,
            Who::Rival => sym::Rival1Pic,
        }
    }
}

/// One statement of `OakSpeech`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Op {
    /// Up to the first picture: the music, the new game's world and the warp to Red's room.
    Prepare,
    ClearScreen,
    /// `IntroDisplayPicCenteredOrUpperRight`, centred.
    Pic(DmgPointer),
    /// Nidorino, turned to face the player.
    Nidorino,
    FadeInIntroPic,
    FadeOutToWhite,
    FadeInFromWhite,
    MovePicLeft,
    Text(DmgPointer),
    ChooseName(Who),
    Sound(SoundId),
    Delay(u16),
    /// `RedSprite` into `vSprites`.
    LoadRedSprite,
    /// The shrink's `ResetPlayerSpriteData` and the music fading into Pallet Town's bank.
    FadeMusic,
    /// The picture rubbed out, the text box tiles back and the player's sprite drawn in its place.
    ShowSprite,
}

const SPEECH: &[Op] = &[
    Op::Prepare, Op::Pic(sym::ProfOakPic), Op::FadeInIntroPic, Op::Text(sym::OakSpeechText1),
    Op::FadeOutToWhite, Op::ClearScreen, Op::Nidorino, Op::MovePicLeft, Op::Text(sym::OakSpeechText2),
    Op::FadeOutToWhite, Op::ClearScreen, Op::Pic(sym::RedPicFront), Op::MovePicLeft, Op::Text(sym::IntroducePlayerText),
    Op::ChooseName(Who::Player),
    Op::FadeOutToWhite, Op::ClearScreen, Op::Pic(sym::Rival1Pic), Op::FadeInIntroPic, Op::Text(sym::IntroduceRivalText),
    Op::ChooseName(Who::Rival),
    Op::FadeOutToWhite, Op::ClearScreen, Op::Pic(sym::RedPicFront), Op::FadeInFromWhite, Op::Text(sym::OakSpeechText3),
    Op::Sound(sounds::SFX_SHRINK), Op::Delay(4), Op::LoadRedSprite, Op::Pic(sym::ShrinkPic1), Op::Delay(4),
    Op::Pic(sym::ShrinkPic2), Op::FadeMusic, Op::Delay(20), Op::ShowSprite, Op::Delay(50),
    Op::FadeOutToWhite, Op::ClearScreen,
];

/// Where `ChooseName` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Naming {
    SlideRight(u8),
    Menu,
    /// The naming screen is up.
    Typing,
    /// A preset: the list rubbed out and held, then the `Delay3`, then the slide back.
    Chosen(u8),
    SlideLeft(u8),
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OakSpeech {
    op: usize,
    /// How far into the statement: a palette, a column or a step of `rWX`.
    progress: u8,
    naming: Naming,
    input: MenuInput,
    wait: Wait,
    /// A child mode is up and the statement goes on when it comes down.
    child: bool,
    /// `wUpdateSpritesEnabled` is 1, so `VBlank` draws the player's sprite.
    sprites: bool,
}

impl Default for OakSpeech {
    fn default() -> Self {
        Self::new()
    }
}

impl OakSpeech {
    pub fn new() -> Self {
        Self {
            op: 0,
            progress: 0,
            naming: Naming::SlideRight(0),
            input: MenuInput::new(0, 3, (1, 2), Joypad::A),
            wait: Wait::default(),
            child: false,
            sprites: false,
        }
    }

    pub fn is_done(&self) -> bool {
        self.op >= SPEECH.len()
    }

    /// Oak's list of names is waiting for a choice.
    pub fn is_choosing_name(&self) -> bool {
        matches!(SPEECH.get(self.op), Some(Op::ChooseName(_))) && self.naming == Naming::Menu && !self.child
            && self.input.is_polling()
    }

    pub fn selected(&self) -> u8 {
        self.input.current
    }

    pub fn update(&mut self, ctx: &mut Ctx) -> Transition {
        if self.sprites {
            let mut sprites = player_sprite();
            prepare_oam(&mut sprites, &mut ctx.screen.sprites, false);
        }
        if self.child {
            return Transition::Stay;
        }
        match self.wait.tick(ctx) {
            Tick::Waiting => return Transition::Stay,
            Tick::Done | Tick::Interrupted => {}
        }
        loop {
            if self.is_done() {
                return Transition::Stay;
            }
            match self.step(ctx) {
                Some(transition) => return transition,
                None if self.wait.is_running() || self.child => return Transition::Stay,
                None => {}
            }
        }
    }

    /// A child has come down.
    pub fn resume(&mut self, ctx: &mut Ctx) -> Transition {
        self.child = false;
        if let Some(Op::ChooseName(who)) = SPEECH.get(self.op).copied() {
            match self.naming {
                Naming::Typing => {
                    let name = ctx.world.text.string(TextBuffer::StringBuffer);
                    if name.is_empty() {
                        return self.push(Mode::NamingScreen(NamingScreen::new(naming_type(who), None)));
                    }
                    *name_of(ctx.world, who) = name;
                    clear_screen(&mut ctx.screen.ui);
                    load_pic(ctx, who.pic());
                    copy_pic_to_tile_map(&mut ctx.screen.ui, 6, 4, 0, false);
                    return self.name_said(who);
                }
                Naming::Text => self.next(),
                _ => {}
            }
        } else {
            self.next();
        }
        self.update(ctx)
    }

    fn next(&mut self) {
        self.op += 1;
        self.progress = 0;
        self.naming = Naming::SlideRight(0);
    }

    fn push(&mut self, mode: Mode) -> Transition {
        self.child = true;
        Transition::Push(mode)
    }

    fn step(&mut self, ctx: &mut Ctx) -> Option<Transition> {
        let op = SPEECH[self.op];
        match op {
            Op::Prepare => {
                prepare(ctx);
                self.next();
            }
            Op::ClearScreen => {
                clear_screen(&mut ctx.screen.ui);
                self.next();
            }
            Op::Pic(pic) => {
                load_pic(ctx, pic);
                copy_pic_to_tile_map(&mut ctx.screen.ui, 6, 4, 0, false);
                self.next();
            }
            Op::Nidorino => {
                load_mon_pic(ctx, PokemonSpecies::Nidorino, true);
                copy_pic_to_tile_map(&mut ctx.screen.ui, 6, 4, 0, true);
                self.next();
            }
            Op::FadeInIntroPic => {
                let palettes = rom_slice(sym::IntroFadePalettes);
                if self.progress < 6 {
                    ctx.screen.effects.bgp = palettes[self.progress as usize];
                    self.progress += 1;
                    self.wait = Wait::frames(INTRO_FADE_STEP);
                } else {
                    self.next();
                }
            }
            Op::FadeOutToWhite | Op::FadeInFromWhite => {
                if self.progress < FADE_STEPS {
                    if op == Op::FadeOutToWhite {
                        fade_out_to_white(ctx, self.progress);
                    } else {
                        fade_in_from_white(ctx, self.progress);
                    }
                    self.progress += 1;
                    self.wait = Wait::frames(FADE_STEP);
                } else {
                    self.next();
                }
            }
            Op::MovePicLeft => self.move_pic_left(ctx),
            Op::Text(text) => {
                let commands = decode(text).expect("Oak's texts decode");
                return Some(self.push(Mode::TextBox(TextBox::script(commands))));
            }
            Op::ChooseName(who) => return self.choose_name(ctx, who),
            Op::Sound(id) => {
                ctx.audio.play_sound(id);
                self.next();
            }
            Op::Delay(frames) => {
                if self.progress == 0 {
                    self.progress = 1;
                    self.wait = Wait::frames(frames);
                } else {
                    self.next();
                }
            }
            Op::LoadRedSprite => {
                ctx.screen.tiles.load(V_CHARS0, &rom_slice(sym::RedSprite)[..12 * 16]);
                self.next();
            }
            Op::FadeMusic => {
                ctx.audio.set_bank(AudioBank::from_rom_bank(sym::Music_PalletTown.bank.id()).expect("an audio bank"));
                ctx.audio.fade_out(10);
                ctx.audio.play_new_sound(SoundId::STOP_ALL_MUSIC);
                self.next();
            }
            Op::ShowSprite => {
                ctx.screen.ui.fill(6, 5, 7, 7, UiSurface::BLANK);
                ctx.screen.tiles.load_text_box_tiles();
                self.sprites = true;
                self.next();
            }
        }
        None
    }

    /// `MovePicLeft`: the window shown 112 pixels in, then eight pixels further left each frame, over
    /// a background left blank since the title.
    fn move_pic_left(&mut self, ctx: &mut Ctx) {
        let wx = match self.progress {
            0 => MOVE_PIC_LEFT_WX,
            1 => {
                ctx.screen.effects.bgp = super::screen::BGP_NORMAL;
                MOVE_PIC_LEFT_WX
            }
            _ => {
                let wx = ctx.screen.window.as_ref().map_or(WX_LEFT, |window| window.x).wrapping_sub(8);
                if wx == 0xFF {
                    ctx.screen.background = None;
                    ctx.screen.window = None;
                    self.next();
                    return;
                }
                wx
            }
        };
        let mut tiles = TileMap::filled(UiSurface::BLANK);
        for row in 0..18 {
            for column in 0..SCREEN_TILES_X {
                tiles.set(column, row, ctx.screen.ui.get(column, row));
            }
        }
        ctx.screen.background = Some(TileMap::filled(UiSurface::BLANK));
        ctx.screen.window = Some(Window { x: wx, y: 0, tiles });
        self.progress += 1;
        self.wait = Wait::frames(1);
    }

    fn name_said(&mut self, who: Who) -> Transition {
        self.naming = Naming::Text;
        let text = match who {
            Who::Player => sym::YourNameIsText,
            Who::Rival => sym::HisNameIsText,
        };
        self.push(Mode::TextBox(TextBox::script(decode(text).expect("the name texts decode"))))
    }

    fn choose_name(&mut self, ctx: &mut Ctx, who: Who) -> Option<Transition> {
        match self.naming {
            Naming::SlideRight(column) if column < SLIDE_COLUMNS => {
                slide(&mut ctx.screen.ui, 4 * SCREEN_TILES_X + 5 + SLIDE_REGION + column as usize, false);
                self.naming = Naming::SlideRight(column + 1);
                self.wait = Wait::frames(3);
            }
            Naming::SlideRight(_) => {
                // `DisplayIntroNameTextBox`.
                let ui = &mut ctx.screen.ui;
                ui.text_box_border(0, 0, 9, 10);
                place_string_lines(ui, 3, 0, &poke_core::charmap::encode("NAME").expect("NAME encodes"));
                let list = match who {
                    Who::Player => sym::DefaultNamesPlayer,
                    Who::Rival => sym::DefaultNamesRival,
                };
                place_string_lines(ui, 2, 2, rom_slice(list));
                ctx.menu.last_item = 0;
                self.input = MenuInput::new(0, 3, (1, 2), Joypad::A);
                self.input.call(ctx);
                self.naming = Naming::Menu;
                return self.choose_name(ctx, who);
            }
            Naming::Menu => {
                if self.input.update(ctx).is_none() {
                    self.wait = Wait::frames(1);
                    return None;
                }
                if self.input.current == 0 {
                    self.naming = Naming::Typing;
                    return Some(self.push(Mode::NamingScreen(NamingScreen::new(naming_type(who), None))));
                }
                // `OakSpeechSlidePicLeft`: the list rubbed out and held for 10 frames.
                ctx.screen.ui.fill(0, 0, 11, 12, UiSurface::BLANK);
                let names = match who {
                    Who::Player => player_names(),
                    Who::Rival => rival_names(),
                };
                *name_of(ctx.world, who) = names[self.input.current as usize].clone();
                self.naming = Naming::Chosen(0);
                self.wait = Wait::frames(10);
            }
            Naming::Chosen(_) => {
                self.naming = Naming::SlideLeft(0);
                self.wait = Wait::frames(3);
            }
            Naming::SlideLeft(column) if column < SLIDE_COLUMNS => {
                slide(&mut ctx.screen.ui, 12 + 4 * SCREEN_TILES_X - column as usize, true);
                self.naming = Naming::SlideLeft(column + 1);
                self.wait = Wait::frames(3);
            }
            Naming::SlideLeft(_) => return Some(self.name_said(who)),
            Naming::Typing | Naming::Text => {}
        }
        None
    }
}

/// One column of `OakSpeechSlidePicCommon`, over the tile map as the one run of bytes it is. Right
/// copies each tile of `[end - 124, end]` one on, from the end; left copies each of `[start,
/// start + 124]` one back, from the start, and blanks the last with tile 0.
fn slide(ui: &mut UiSurface, from: usize, left: bool) {
    let get = |ui: &UiSurface, i: usize| ui.get(i % SCREEN_TILES_X, i / SCREEN_TILES_X);
    if left {
        for i in from..from + SLIDE_REGION {
            let tile = get(ui, i);
            ui.set((i - 1) % SCREEN_TILES_X, (i - 1) / SCREEN_TILES_X, tile);
        }
        let last = from + SLIDE_REGION - 1;
        ui.set(last % SCREEN_TILES_X, last / SCREEN_TILES_X, 0);
    } else {
        for i in (from - SLIDE_REGION + 1..=from).rev() {
            let tile = get(ui, i);
            ui.set((i + 1) % SCREEN_TILES_X, (i + 1) / SCREEN_TILES_X, tile);
        }
    }
}

fn naming_type(who: Who) -> NamingScreenType {
    match who {
        Who::Player => NamingScreenType::Player,
        Who::Rival => NamingScreenType::Rival,
    }
}

fn name_of(world: &mut World, who: Who) -> &mut Vec<u8> {
    match who {
        Who::Player => &mut world.player_name,
        Who::Rival => &mut world.rival_name,
    }
}

/// `ResetPlayerSpriteData`: the player's slot alone, standing at the middle of the screen.
fn player_sprite() -> Sprites {
    let mut sprites = [SpriteState::default(); NUM_SPRITES];
    sprites[0] = SpriteState { picture_id: 1, image_base_offset: 1, y_pixels: 0x3C, x_pixels: 0x40, ..SpriteState::default() };
    sprites
}

/// `OakSpeech` up to its first picture: the music, `PrepareOakSpeech`, `InitPlayerData2`, the Potion
/// in the PC and `PrepareForSpecialWarp`.
fn prepare(ctx: &mut Ctx) {
    ctx.audio.play_sound(SoundId::STOP_ALL_MUSIC);
    ctx.audio.play_music(sounds::MUSIC_ROUTES2);
    clear_screen(&mut ctx.screen.ui);
    ctx.screen.tiles.load_text_box_tiles();

    // `PrepareOakSpeech` zeroes everything from `wPlayerName` to `wBoxDataEnd` but the options, the
    // letter delay and `wStatusFlags6`, then names both players for a debug new game. Its
    // `InitOptions` is the main menu's, which knows whether the option screen was visited.
    let old = std::mem::take(ctx.world);
    let world = &mut *ctx.world;
    world.options = old.options;
    world.no_text_delay = old.no_text_delay;
    world.one_frame_letter_delay = old.one_frame_letter_delay;
    world.hall_of_fame = old.hall_of_fame;
    world.player_name = poke_core::charmap::encode("NINTEN").expect("encodes");
    world.rival_name = poke_core::charmap::encode("SONY").expect("encodes");

    // `InitPlayerData2`: the ID from `hRandomSub` of one draw and `hRandomAdd` of the next.
    world.player_id = u16::from_be_bytes([ctx.rng.random(), ctx.rng.random()]);
    world.money = START_MONEY;
    world.pc_items = Inventory::pc(Vec::new());
    world.pc_items.add(ItemId::Potion, 1);
    ctx.screen.tiles.animation.kind = 0;
}


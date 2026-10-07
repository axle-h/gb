//! `InternalClockTradeAnim` (`engine/movie/trade.asm`, `trade2.asm`): the trade movie an in-game
//! trade plays between `ConnectCableText` and the exchange.
//!
//! In: the mon given and the mon received, each a species, an OT and an OT ID
//! (`wTradedPlayerMon*`, `wTradedEnemyMon*`), the other side's name (`wLinkEnemyTrainerName`) and
//! the palettes `Trade_Cleanup`'s `LoadGBPal` puts back. Out: `wStringBuffer` and `wNameBuffer`
//! hold the two species' names, and nothing else changes; the exchange is the caller's.
//!
//! `TradeAnimCommon` runs `InternalClockTradeFuncSequence`, a routine at a time:
//! `LoadTradingGFXAndMonNames`; `Trade_ShowPlayerMon` (the info box and the flipped picture slide in
//! over 63 frames, 80 held, the ball's poof and drop, the cry); `Trade_DrawOpenEndOfLinkCable` (10,
//! then the cable's end and `SFX_HEAL_HP`); `Trade_AnimateBallEnteringLinkCable` (the ball's shake, 10
//! frames, then the ball 16 steps into the cable at `Delay3` each, `SFX_TINK` between);
//! `Trade_AnimLeftToRight` (the circled icon over the left Game Boy, 10 frames, 6, 4 and 6 sixteen-pixel
//! scrolls of 8 frames, the cable and its colours flashing at each, then 4 steps right and 4 down at 8
//! frames); 100 frames; `Trade_ShowClearedWindow` and the three texts, each held 80 frames (the first
//! 200), the first and last boxes slid off over 50, 77 and 10 frames; `Trade_AnimRightToLeft`, the same
//! backwards; the cable's end again; `Trade_ShowEnemyMon` (the ball jumps, the info box, the picture
//! out of the poof, the cry, 100 frames, `TradeTakeCareText` and 80); 100 frames; `Trade_Cleanup`.
//! `ExternalClockTradeAnim` is the Cable Club's, the same routines in another order.
//!
//! Dropped as loading: the LCD-off loads, `CopyScreenTileBufferToVRAM`'s three frames,
//! `Trade_CopyTileMapToVRAM`'s and `ClearScreen`'s `Delay3`, `PrintText`'s box, the pictures'
//! decompression and `LoadMonPartySpriteGfx`'s copies. Kept: every other `DelayFrame` and
//! `DelayFrames`, the `Delay3` steps of the ball in the cable, and the cries and sounds waited on.
//!
//! The screen is the hardware's here, both background maps against the window as `MovieScreen`
//! keeps them: `Trade_ShowPlayerMon` and the cable scrolls show `vBGMap1` behind a window of
//! `vBGMap0`, the texts the window of `vBGMap1` over all of it. A palette command goes wherever the
//! cartridge sends one: `SET_PAL_POKEMON_WHOLE_SCREEN` with each picture and `SET_PAL_GENERIC` at the
//! cable's end.

use std::collections::VecDeque;
use poke_core::species::PokemonSpecies;
use poke_core::text_script::{far_text, TextBuffer};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::gfx::layers::{Object, SgbPick, TileMap};
use crate::gfx::mon_icons::{load_mon_party_sprite_gfx, write_mon_party_sprite_oam};
use crate::gfx::sgb::{determine_palette_id_out_of_battle, PaletteCommand};
use crate::gfx::tiles::{V_CHARS0, V_CHARS2};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::mode::{Ctx, Mode, Transition};
use crate::modes::battle::animation::{anim, AnimBattle, Animation, Routine};
use crate::modes::text_box::TextBox;
use crate::systems::battle::Side;
use crate::systems::print_num::{print_number, NumberFormat};
use super::intro::place_string_lines;
use super::screen::{copy_pic_to_tile_map, copy_tile_ids, load_mon_pic, Dest, MovieScreen, WX_LEFT};

/// `TILEMAP_GAME_BOY` and `TILEMAP_LINK_CABLE`.
const TILEMAP_GAME_BOY: usize = 6;
const TILEMAP_LINK_CABLE: usize = 7;
/// `TradingAnimationGraphics`' first tile in `vChars2`, and `TradingAnimationGraphics2`'s in `vSprites`.
const TRADE_TILES: usize = 0x31;
const CABLE_BALL_TILES: usize = 0x7C;
/// The cable's tiles: its plug, its run, its corner, its bend and its drop.
const CABLE_END: u8 = 0x5D;
const CABLE: u8 = 0x5E;
const CABLE_CORNER: u8 = 0x5F;
const CABLE_BEND: u8 = 0x60;
const CABLE_DOWN: u8 = 0x61;
/// The ball inside the cable, and its bulge, the tile after it.
const BALL_IN_CABLE: u8 = 0x7E;
/// `ICON_TRADEBUBBLE << 2` and `ICONOFFSET`.
const BUBBLE: u8 = 0x0E << 2;
const ICON_OFFSET: u8 = 0x40;
/// The objects of the circled mon: its icon and the circle's four blocks.
const CIRCLED_OBJECTS: usize = 0x14;
/// `rBGP`'s bits `Trade_AnimCircledMon` flips, which flash the cable.
const CABLE_FLASH: u8 = 0x3C;
/// The LCDC this movie switches between: `vBGMap1` behind a window of `vBGMap0`, and the default.
const SWAPPED: Lcdc = Lcdc { bg_map: 1, window_map: 0, window_on: true };
const SWAPPED_NO_WINDOW: Lcdc = Lcdc { bg_map: 1, window_map: 0, window_on: false };
const DEFAULT: Lcdc = Lcdc { bg_map: 0, window_map: 1, window_on: true };
const WINDOW_HIDDEN: u8 = 0x90;
/// `Trade_SlideTextBoxOffScreen`'s last `rWX`, past the screen's right edge.
const WX_OFF_SCREEN: u8 = 0xA1;
/// Where `Trade_ShowPlayerMon` starts sliding from.
const SLIDE_START: u8 = 0x7E;
const OBP0_TRADE: SgbPick = SgbPick { dmg: 0b1110_0100, sgb: 0b1111_0000 };
const OBP0_CABLE: u8 = 0b1110_0100;
const OBP1_ICONS: u8 = 0b1101_0000;

/// One side of the trade, as `InGameTrade_PrepareTradeData` writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradedMon {
    pub species: PokemonSpecies,
    pub ot: Vec<u8>,
    pub ot_id: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeData {
    /// The mon given, on the left Game Boy.
    pub player: TradedMon,
    /// The mon received, on the right.
    pub enemy: TradedMon,
    /// `wLinkEnemyTrainerName`.
    pub enemy_trainer: Vec<u8>,
    /// `rBGP`, `rOBP0` and `rOBP1` as `LoadGBPal` sets them on the map the trade is made on.
    pub palettes: [u8; 3],
}

/// `InternalClockTradeFuncSequence`'s routines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TradeFunc {
    LoadTradingGfxAndMonNames,
    ShowPlayerMon,
    DrawOpenEndOfLinkCable,
    AnimateBallEnteringLinkCable,
    AnimLeftToRight,
    Delay100,
    ShowClearedWindow,
    PrintTradeWentToText,
    PrintTradeForSendsText,
    PrintTradeFarewellText,
    AnimRightToLeft,
    ShowEnemyMon,
    Cleanup,
}

const INTERNAL_CLOCK_TRADE_FUNC_SEQUENCE: [TradeFunc; 16] = {
    use TradeFunc::*;
    [
        LoadTradingGfxAndMonNames, ShowPlayerMon, DrawOpenEndOfLinkCable, AnimateBallEnteringLinkCable, AnimLeftToRight,
        Delay100, ShowClearedWindow, PrintTradeWentToText, PrintTradeForSendsText, PrintTradeFarewellText, AnimRightToLeft,
        ShowClearedWindow, DrawOpenEndOfLinkCable, ShowEnemyMon, Delay100, Cleanup,
    ]
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Lcdc {
    bg_map: usize,
    window_map: usize,
    window_on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TradeText {
    WentTo,
    For,
    Sends,
    WavesFarewell,
    Transferred,
    TakeCare,
}

impl TradeText {
    fn label(self) -> &'static str {
        match self {
            Self::WentTo => "_TradeWentToText",
            Self::For => "_TradeForText",
            Self::Sends => "_TradeSendsText",
            Self::WavesFarewell => "_TradeWavesFarewellText",
            Self::Transferred => "_TradeTransferredText",
            Self::TakeCare => "_TradeTakeCareText",
        }
    }
}

/// Which mon an info box describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Shown {
    Given,
    Received,
}

/// One write or one wait of a routine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Op {
    /// `DelayFrames`.
    Wait(u16),
    /// `PlayCry`'s `WaitForSoundToFinish`.
    WaitForSound,
    /// `Trade_ShowAnimation`: `MoveAnimation` of an animation id.
    Animation(u8),
    Text(TradeText),
    LoadTradingGfx,
    /// `Trade_ClearTileMap`, and `ClearScreen` without its `Delay3`.
    ClearTileMap,
    /// `CopyScreenTileBufferToVRAM` into a map, or the transfer let finish there.
    CopyTileMapTo(usize),
    /// `Trade_CopyTileMapToVRAM`: the transfer on for its `Delay3`, then off.
    CopyTileMapToVram,
    /// `hAutoBGTransferEnabled`, and `Trade_LoadMonSprite`'s flip of it.
    Transfer(bool),
    ToggleTransfer,
    Lcdc(Lcdc),
    Wx(u8),
    Wy(u8),
    Scx(u8),
    ScxBy(u8),
    TextBoxBorder { x: usize, y: usize, width: usize, height: usize },
    ClearArea { x: usize, y: usize, width: usize, height: usize },
    MonInfo(Shown),
    MonPalette(PokemonSpecies),
    GenericPalette,
    /// `LoadFlippedFrontSpriteByMonIndex` at (7, 2).
    LoadPic(PokemonSpecies),
    Sound(SoundId),
    Cry(PokemonSpecies),
    Obp0(SgbPick),
    Obp1(u8),
    LinkCable,
    /// `Trade_CopyCableTilesOffScreen`: the tile map's rows 4 and 5 into `vBGMap1` from column
    /// `column` of row 4, wrapping.
    CableTilesOffScreen { column: usize },
    DrawLeftGameboy,
    DrawRightGameboy,
    DrawCableAcrossScreen,
    ClearSprites,
    /// `WriteOAMBlock` of `Trade_BallInsideLinkCableOAMBlock` at `x`, every tile `tile`.
    BallInCable { x: u8, tile: u8 },
    LoadPartyIcons,
    /// `Trade_WriteCircledMonOAM` at the base coordinates.
    CircledMon { species: PokemonSpecies, x: u8, y: u8 },
    /// `Trade_AddOffsetsToOAMCoords`.
    AddOffsets { x: u8, y: u8 },
    AnimCircledMon,
    Cleanup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeMovie {
    data: TradeData,
    /// The next routine of the sequence.
    next: usize,
    ops: VecDeque<Op>,
    wait: u16,
    waiting_for_sound: bool,
    animation: Option<Animation>,
    /// A text is up.
    child: bool,
    screen: MovieScreen,
    done: bool,
}

impl TradeMovie {
    pub fn new(data: TradeData) -> Self {
        // `TradeAnimCommon` zeroes `hSCX` and `hSCY`; the screen it starts from is the overworld's text.
        let mut screen = MovieScreen::default();
        screen.transfer = None;
        screen.wy = 0;
        Self {
            data, next: 0, ops: VecDeque::new(), wait: 0, waiting_for_sound: false, animation: None, child: false, screen,
            done: false,
        }
    }

    /// The routines of the sequence begun so far.
    pub fn routines_begun(&self) -> usize {
        self.next
    }

    pub fn open(&mut self, ctx: &mut Ctx) -> Transition {
        let transition = self.run(ctx);
        self.present(ctx);
        transition
    }

    pub fn update(&mut self, ctx: &mut Ctx) -> Transition {
        self.screen.vblank(ctx);
        let transition = self.run(ctx);
        self.present(ctx);
        transition
    }

    /// The text has ended, in this frame: the window shows it as the transfer has carried it.
    pub fn resume(&mut self, ctx: &mut Ctx) -> Transition {
        self.child = false;
        self.screen.vblank(ctx);
        let transition = self.run(ctx);
        self.present(ctx);
        transition
    }

    fn present(&mut self, ctx: &mut Ctx) {
        if !self.child && !self.done {
            self.screen.present(ctx);
        }
    }

    fn run(&mut self, ctx: &mut Ctx) -> Transition {
        let instant = ctx.pacing == crate::Pacing::Instant;
        loop {
            if self.animation.is_some() && !self.animate(ctx) {
                return Transition::Stay;
            }
            if self.wait > 0 {
                self.wait -= 1;
                if self.wait > 0 {
                    return Transition::Stay;
                }
            }
            if self.waiting_for_sound {
                if !instant && !ctx.audio.sound_finished() {
                    return Transition::Stay;
                }
                self.waiting_for_sound = false;
            }
            let Some(op) = self.ops.pop_front() else {
                let Some(&func) = INTERNAL_CLOCK_TRADE_FUNC_SEQUENCE.get(self.next) else {
                    self.done = true;
                    MovieScreen::release(ctx);
                    return Transition::Pop(crate::mode::Outcome::Done);
                };
                self.next += 1;
                self.expand(func);
                continue;
            };
            if let Some(transition) = self.op(op, ctx) {
                return transition;
            }
            if self.wait > 0 {
                if instant {
                    self.wait = 0;
                } else {
                    return Transition::Stay;
                }
            }
        }
    }

    /// The battle's animation engine on this screen: `wShadowOAM` and `hSCX` are the movie's, latched
    /// at the next `VBlank`.
    fn animate(&mut self, ctx: &mut Ctx) -> bool {
        let animation = self.animation.as_mut().expect("an animation is playing");
        let shown_sprites = std::mem::replace(&mut ctx.screen.sprites, self.screen.oam.clone());
        let shown_scx = std::mem::replace(&mut ctx.screen.effects.scx, self.screen.scx);
        let done = animation.update(ctx);
        self.screen.oam = std::mem::replace(&mut ctx.screen.sprites, shown_sprites);
        self.screen.scx = std::mem::replace(&mut ctx.screen.effects.scx, shown_scx);
        if done {
            self.animation = None;
        }
        done
    }

    fn push(&mut self, ops: impl IntoIterator<Item = Op>) {
        self.ops.extend(ops);
    }

    fn expand(&mut self, func: TradeFunc) {
        let (player, enemy) = (self.data.player.species, self.data.enemy.species);
        match func {
            TradeFunc::LoadTradingGfxAndMonNames => self.push([Op::ClearTileMap, Op::LoadTradingGfx]),
            TradeFunc::ShowPlayerMon => {
                self.push([
                    Op::Lcdc(SWAPPED), Op::Wy(0x50), Op::Wx(0x86), Op::Scx(0x86), Op::Transfer(false),
                    Op::TextBoxBorder { x: 4, y: 0, width: 10, height: 6 }, Op::MonInfo(Shown::Given), Op::CopyTileMapTo(0),
                    Op::ClearTileMap,
                ]);
                self.load_mon_sprite(player);
                for a in (2..=SLIDE_START).rev().step_by(2) {
                    self.push([Op::Wait(1), Op::Wx(a), Op::Scx(a)]);
                }
                self.push([
                    Op::Wait(80), Op::Animation(anim::TRADE_BALL_POOF_ANIM), Op::Animation(anim::TRADE_BALL_DROP_ANIM),
                    Op::Cry(player), Op::WaitForSound, Op::Transfer(false),
                ]);
            }
            TradeFunc::DrawOpenEndOfLinkCable => {
                self.push([Op::ClearTileMap, Op::CopyTileMapTo(0), Op::GenericPalette]);
                self.cable_tiles_off_screen(12);
                // The cable is drawn at `hSCX` $A0 and moved on by 80 pixels with no frame between.
                self.push([
                    Op::Scx(0xA0), Op::Wait(1), Op::Lcdc(SWAPPED_NO_WINDOW), Op::LinkCable, Op::CopyTileMapToVram,
                    Op::Sound(sounds::SFX_HEAL_HP), Op::Scx(0xA0 + 20 * 4),
                ]);
            }
            TradeFunc::AnimateBallEnteringLinkCable => {
                self.push([Op::Animation(anim::TRADE_BALL_SHAKE_ANIM), Op::Wait(10), Op::Obp0(SgbPick::both(OBP0_CABLE))]);
                let mut bulge = false;
                let mut x = 0x60u8;
                loop {
                    bulge = !bulge;
                    self.push([Op::BallInCable { x, tile: BALL_IN_CABLE + bulge as u8 }, Op::Wait(3)]);
                    x += 4;
                    if x >= 0xA0 {
                        break;
                    }
                    self.push([Op::Sound(sounds::SFX_TINK)]);
                }
                // `ClearScreen` with the transfer on, the tile map copied to `vBGMap0`, and a `Delay3`.
                self.push([Op::ClearSprites, Op::ClearTileMap, Op::CopyTileMapTo(1), Op::CopyTileMapTo(0)]);
            }
            TradeFunc::AnimLeftToRight => {
                self.init_gameboy_transfer_gfx();
                self.push([Op::Obp0(SgbPick::both(OBP0_CABLE)), Op::CircledMon { species: player, x: 0x54, y: 0x1C }]);
                self.push([Op::DrawLeftGameboy, Op::Wait(1), Op::CopyTileMapToVram, Op::DrawCableAcrossScreen]);
                self.cable_tiles_off_screen(12);
                self.horizontal(6, true);
                self.push([Op::Transfer(true), Op::DrawCableAcrossScreen]);
                self.horizontal(4, true);
                self.push([Op::DrawRightGameboy, Op::Wait(1)]);
                self.horizontal(6, true);
                self.push([Op::Transfer(false)]);
                self.vertical(true);
                self.push([Op::ClearSprites]);
            }
            TradeFunc::AnimRightToLeft => {
                self.init_gameboy_transfer_gfx();
                self.push([Op::CircledMon { species: enemy, x: 0x64, y: 0x44 }]);
                self.push([Op::DrawRightGameboy, Op::Wait(1), Op::CopyTileMapToVram, Op::DrawCableAcrossScreen]);
                self.cable_tiles_off_screen(20);
                self.vertical(false);
                self.horizontal(6, false);
                self.push([Op::Transfer(true), Op::DrawCableAcrossScreen]);
                self.horizontal(4, false);
                self.push([Op::DrawLeftGameboy, Op::Wait(1)]);
                self.horizontal(6, false);
                self.push([Op::Transfer(false), Op::ClearSprites]);
            }
            TradeFunc::Delay100 => self.push([Op::Wait(100)]),
            TradeFunc::ShowClearedWindow => self.show_cleared_window(),
            TradeFunc::PrintTradeWentToText => {
                self.push([Op::Text(TradeText::WentTo), Op::Wait(200)]);
                self.slide_text_box_off_screen();
            }
            TradeFunc::PrintTradeForSendsText => {
                self.push([Op::Text(TradeText::For), Op::Wait(80), Op::Text(TradeText::Sends), Op::Wait(80)]);
            }
            TradeFunc::PrintTradeFarewellText => {
                self.push([Op::Text(TradeText::WavesFarewell), Op::Wait(80), Op::Text(TradeText::Transferred), Op::Wait(80)]);
                self.slide_text_box_off_screen();
            }
            TradeFunc::ShowEnemyMon => {
                self.push([Op::Animation(anim::TRADE_BALL_TILT_ANIM)]);
                self.show_cleared_window();
                self.push([
                    Op::TextBoxBorder { x: 4, y: 10, width: 10, height: 6 }, Op::MonInfo(Shown::Received),
                    Op::CopyTileMapToVram, Op::Transfer(true),
                ]);
                self.load_mon_sprite(enemy);
                self.push([
                    Op::Animation(anim::TRADE_BALL_POOF_ANIM), Op::Transfer(true), Op::Cry(enemy), Op::WaitForSound,
                    Op::Wait(100), Op::ClearArea { x: 4, y: 10, width: 12, height: 8 }, Op::Text(TradeText::TakeCare),
                    Op::Wait(80),
                ]);
            }
            TradeFunc::Cleanup => self.push([Op::Cleanup]),
        }
    }

    /// `Trade_LoadMonSprite`: the palette, the transfer flipped, the picture, and 10 frames.
    fn load_mon_sprite(&mut self, species: PokemonSpecies) {
        self.push([Op::MonPalette(species), Op::ToggleTransfer, Op::LoadPic(species), Op::Wait(10)]);
    }

    /// `Trade_CopyCableTilesOffScreen`, which waits 10 frames for `RedrawRowOrColumn`.
    fn cable_tiles_off_screen(&mut self, column: usize) {
        self.push([Op::CableTilesOffScreen { column }, Op::Wait(10)]);
    }

    /// `Trade_ShowClearedWindow`: the window over all of the screen, showing the tile map.
    fn show_cleared_window(&mut self) {
        self.push([
            Op::Transfer(true), Op::ClearTileMap, Op::CopyTileMapTo(1), Op::Lcdc(DEFAULT), Op::Wx(WX_LEFT), Op::Wy(0),
            Op::Scx(0x90),
        ]);
    }

    /// `Trade_InitGameboyTransferGfx`.
    fn init_gameboy_transfer_gfx(&mut self) {
        self.push([
            Op::ClearTileMap, Op::CopyTileMapTo(1), Op::Transfer(false), Op::Obp1(OBP1_ICONS), Op::LoadPartyIcons, Op::Wait(1),
            Op::Lcdc(SWAPPED), Op::Scx(0), Op::Wy(WINDOW_HIDDEN),
        ]);
    }

    /// `Trade_SlideTextBoxOffScreen`.
    fn slide_text_box_off_screen(&mut self) {
        self.push([Op::Wait(50)]);
        for wx in (WX_LEFT + 2..=WX_OFF_SCREEN).step_by(2) {
            self.push([Op::Wait(1), Op::Wx(wx)]);
        }
        self.push([Op::ClearTileMap, Op::Wait(10), Op::Wx(WX_LEFT)]);
    }

    /// `Trade_AnimMonMoveHorizontal`: `units` of 16 pixels, two a frame, the mon animated after each.
    fn horizontal(&mut self, units: u8, right: bool) {
        let step = if right { 2 } else { 2u8.wrapping_neg() };
        for _ in 0..units {
            for _ in 0..8 {
                self.push([Op::ScxBy(step), Op::Wait(1)]);
            }
            self.push([Op::AnimCircledMon]);
        }
    }

    /// `Trade_AnimMonMoveVertical`: the mon itself moved where the cable turns, four steps each way.
    fn vertical(&mut self, right: bool) {
        let moves = if right { [(4, 0), (0, 10)] } else { [(0, 10u8.wrapping_neg()), (4u8.wrapping_neg(), 0)] };
        for (x, y) in moves {
            for _ in 0..4 {
                self.push([Op::AddOffsets { x, y }, Op::AnimCircledMon, Op::Wait(8)]);
            }
        }
    }

    /// One op. `Some` where the movie hands over to another mode.
    fn op(&mut self, op: Op, ctx: &mut Ctx) -> Option<Transition> {
        let ui = &mut ctx.screen.ui;
        match op {
            Op::Wait(frames) => self.wait = frames,
            Op::WaitForSound => self.waiting_for_sound = true,
            Op::Animation(id) => {
                let battle = AnimBattle {
                    player_species: self.data.player.species,
                    enemy_species: self.data.enemy.species,
                    damage_multipliers: 0,
                    trainer_battle: false,
                    item: 0,
                    ball_data: 0,
                    animations_on: true,
                    h_scx: self.screen.scx,
                    mons: Default::default(),
                    hp_bar_colours: Default::default(),
                    ruleset: ctx.world.ruleset,
                };
                self.animation = Some(Animation::new(Routine::MoveAnimation { id, kind: 0 }, Side::Player, battle));
            }
            Op::Text(text) => {
                self.child = true;
                MovieScreen::release(ctx);
                let script = far_text(text.label()).expect("the trade's texts are in the cartridge");
                return Some(Transition::Push(Mode::TextBox(TextBox::script(script))));
            }
            Op::LoadTradingGfx => self.load_trading_gfx(ctx),
            Op::ClearTileMap => ui.fill(0, 0, SCREEN_TILES_X, crate::gfx::ui::SCREEN_TILES_Y, UiSurface::BLANK),
            Op::CopyTileMapTo(map) => self.screen.copy_tile_map(ui, Dest { map, offset: 0 }),
            Op::CopyTileMapToVram => {
                self.screen.copy_tile_map(ui, Dest::BG_MAP1);
                self.screen.transfer = None;
            }
            Op::Transfer(on) => self.screen.transfer = on.then_some(Dest::BG_MAP1),
            Op::ToggleTransfer => {
                self.screen.transfer = if self.screen.transfer.is_some() { None } else { Some(Dest::BG_MAP1) };
            }
            Op::Lcdc(lcdc) => {
                self.screen.bg_map = lcdc.bg_map;
                self.screen.window_map = lcdc.window_map;
                self.screen.window_on = lcdc.window_on;
            }
            Op::Wx(wx) => self.screen.wx = wx,
            Op::Wy(wy) => self.screen.wy = wy,
            Op::Scx(scx) => self.screen.scx = scx,
            Op::ScxBy(step) => self.screen.scx = self.screen.scx.wrapping_add(step),
            Op::TextBoxBorder { x, y, width, height } => ui.text_box_border(x, y, width, height),
            Op::ClearArea { x, y, width, height } => ui.fill(x, y, width, height, UiSurface::BLANK),
            Op::MonInfo(side) => self.mon_info(ctx, side),
            Op::MonPalette(species) => {
                let mon = determine_palette_id_out_of_battle(species as u8);
                ctx.screen.sgb.run(&PaletteCommand::PokemonWholeScreen { mon, black: false });
            }
            Op::GenericPalette => ctx.screen.sgb.run(&PaletteCommand::Generic),
            Op::LoadPic(species) => {
                load_mon_pic(ctx, species, true);
                copy_pic_to_tile_map(&mut ctx.screen.ui, 7, 2, 0, true);
            }
            Op::Sound(sound) => ctx.audio.play_sound(sound),
            Op::Cry(species) => ctx.audio.play_cry(species as u8),
            Op::Obp0(pick) => ctx.screen.effects.pick_obp0(pick),
            Op::Obp1(obp1) => ctx.screen.effects.obp1 = obp1,
            Op::LinkCable => copy_tile_ids(ui, 6, 2, TILEMAP_LINK_CABLE, 0),
            Op::CableTilesOffScreen { column } => {
                for row in 4..6 {
                    for i in 0..SCREEN_TILES_X {
                        self.screen.maps[1].set((column + i) % TileMap::SIZE, row, ui.get(i, row));
                    }
                }
            }
            Op::DrawLeftGameboy => {
                clear(ui);
                ui.set(11, 4, CABLE_END);
                ui.fill(12, 4, 8, 1, CABLE);
                copy_tile_ids(ui, 5, 3, TILEMAP_GAME_BOY, 0);
                ui.text_box_border(4, 12, 7, 2);
                place_string_lines(ui, 5, 14, &ctx.world.player_name);
            }
            Op::DrawRightGameboy => {
                clear(ui);
                ui.fill(0, 4, 14, 1, CABLE);
                ui.set(14, 4, CABLE_CORNER);
                ui.fill(14, 5, 1, 4, CABLE_DOWN);
                ui.set(14, 9, CABLE_BEND);
                ui.set(13, 9, CABLE_END);
                copy_tile_ids(ui, 7, 8, TILEMAP_GAME_BOY, 0);
                ui.text_box_border(6, 0, 7, 2);
                place_string_lines(ui, 7, 2, &self.data.enemy_trainer);
            }
            Op::DrawCableAcrossScreen => {
                clear(ui);
                ui.fill(0, 4, SCREEN_TILES_X, 1, CABLE);
            }
            Op::ClearSprites => self.screen.clear_sprites(),
            Op::BallInCable { x, tile } => {
                let flips = [0, Object::X_FLIP, Object::Y_FLIP, Object::X_FLIP | Object::Y_FLIP];
                self.write_oam_block(0, 0x20, x, flips.map(|flip| (tile, flip)));
            }
            Op::LoadPartyIcons => load_mon_party_sprite_gfx(&mut ctx.screen.tiles),
            Op::CircledMon { species, x, y } => {
                write_mon_party_sprite_oam(&mut self.screen.oam, 0, species);
                self.write_circle_oam_blocks();
                self.add_offsets(x, y);
            }
            Op::AddOffsets { x, y } => self.add_offsets(x, y),
            Op::AnimCircledMon => {
                let bgp = ctx.screen.effects.bgp_pick();
                ctx.screen.effects.pick_bgp(SgbPick { dmg: bgp.dmg ^ CABLE_FLASH, sgb: bgp.sgb ^ CABLE_FLASH });
                for object in &mut self.screen.oam[..CIRCLED_OBJECTS] {
                    object.tile ^= ICON_OFFSET;
                }
            }
            Op::Cleanup => {
                let [bgp, obp0, obp1] = self.data.palettes;
                ctx.screen.effects.pick_bgp(SgbPick::both(bgp));
                ctx.screen.effects.pick_obp0(SgbPick::both(obp0));
                ctx.screen.effects.obp1 = obp1;
                ctx.world.no_text_delay = false;
            }
        }
        None
    }

    /// `LoadTradingGFXAndMonNames` past its `Trade_ClearTileMap`, all of it with the LCD off.
    fn load_trading_gfx(&mut self, ctx: &mut Ctx) {
        let tiles = &mut ctx.screen.tiles;
        tiles.load(V_CHARS2 + TRADE_TILES, poke_core::gfx::trade::GAME_BOY);
        tiles.load(V_CHARS2 + TRADE_TILES + poke_core::gfx::trade::GAME_BOY.len() / 16, poke_core::gfx::trade::LINK_CABLE);
        tiles.load(V_CHARS0 + CABLE_BALL_TILES, poke_core::gfx::trade::CABLE_BALL);
        self.screen.maps = [TileMap::filled(UiSurface::BLANK), TileMap::filled(UiSurface::BLANK)];
        self.screen.clear_sprites();
        ctx.world.no_text_delay = true;
        ctx.screen.effects.pick_obp0(OBP0_TRADE);
        self.screen.transfer = None;
        let strings = &mut ctx.world.text.strings;
        strings.insert(TextBuffer::StringBuffer, self.data.player.species.name());
        strings.insert(TextBuffer::NameBuffer, self.data.enemy.species.name());
        strings.insert(TextBuffer::LinkEnemyTrainerName, self.data.enemy_trainer.clone());
    }

    /// `Trade_PrintPlayerMonInfoText` or `Trade_PrintEnemyMonInfoText`.
    fn mon_info(&self, ctx: &mut Ctx, side: Shown) {
        let (mon, top) = match side {
            Shown::Given => (&self.data.player, 0),
            Shown::Received => (&self.data.enemy, 10),
        };
        let ui = &mut ctx.screen.ui;
        place_string_lines(ui, 5, top, &poke_core::tables::db_string("Trade_MonInfoText"));
        let leading_zeroes = |digits| NumberFormat { digits, left_align: false, leading_zeroes: true };
        let dex = mon.species.metadata().pokedex_number as u32;
        print_number(ui, top * SCREEN_TILES_X + 9, dex, leading_zeroes(3));
        place_string_lines(ui, 5, top + 2, &mon.species.name());
        place_string_lines(ui, 8, top + 4, &mon.ot);
        print_number(ui, (top + 6) * SCREEN_TILES_X + 8, mon.ot_id as u32, leading_zeroes(5));
    }

    /// `WriteOAMBlock`: a 2x2 block of objects at OAM `4 * block`, its top left at (`x`, `y`).
    fn write_oam_block(&mut self, block: usize, y: u8, x: u8, tiles: [(u8, u8); 4]) {
        for (i, (tile, attributes)) in tiles.into_iter().enumerate() {
            let (row, column) = ((i / 2) as u8, (i % 2) as u8);
            self.screen.oam[4 * block + i] = Object { y: y + 8 * row, x: x + 8 * column, tile, attributes };
        }
    }

    /// `Trade_WriteCircleOAMBlock`: the circle round the icon, `Trade_CircleOAMBlocks` in blocks 1 to 4.
    fn write_circle_oam_blocks(&mut self) {
        let pal1 = Object::OBP1;
        let (x, y, xy) = (Object::X_FLIP, Object::Y_FLIP, Object::X_FLIP | Object::Y_FLIP);
        let blocks: [(u8, u8, [(u8, u8); 4]); 4] = [
            (8, 8, [(0, 0), (1, 0), (2, 0), (3, 0)]),
            (24, 8, [(1, x), (0, x), (3, x), (2, x)]),
            (8, 24, [(2, y), (3, y), (0, y), (1, y)]),
            (24, 24, [(3, xy), (2, xy), (1, xy), (0, xy)]),
        ];
        for (i, (bx, by, tiles)) in blocks.into_iter().enumerate() {
            self.write_oam_block(i + 1, by, bx, tiles.map(|(tile, flip)| (BUBBLE + tile, pal1 | flip)));
        }
    }

    fn add_offsets(&mut self, x: u8, y: u8) {
        for object in &mut self.screen.oam[..CIRCLED_OBJECTS] {
            object.y = object.y.wrapping_add(y);
            object.x = object.x.wrapping_add(x);
        }
    }
}

fn clear(ui: &mut UiSurface) {
    ui.fill(0, 0, SCREEN_TILES_X, crate::gfx::ui::SCREEN_TILES_Y, UiSurface::BLANK);
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use crate::mode::{Mode, Status};
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Game, Input, Pacing};
    use super::*;

    fn data() -> TradeData {
        TradeData {
            player: TradedMon { species: PokemonSpecies::NidoranMale, ot: encode("RED").unwrap(), ot_id: 12345 },
            enemy: TradedMon { species: PokemonSpecies::NidoranFemale, ot: vec![0x5D], ot_id: 54321 },
            enemy_trainer: vec![0x5D],
            palettes: [0b1110_0100, 0b1101_0000, 0b1110_0000],
        }
    }

    fn game() -> Game {
        let world = World { player_name: encode("RED").unwrap(), ..World::default() };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::Movie(super::super::Movie::trade(data())));
        game
    }

    fn playing(game: &Game) -> bool {
        game.modes().iter().any(|mode| matches!(mode, Mode::Movie(_)))
    }

    /// The cartridge's 2375 frames from the predef to its return, less the 228 of loading the
    /// lockstep prices, with nothing asked of the player.
    #[test]
    fn the_movie_plays_its_frames_and_hands_the_screen_back() {
        let mut game = game();
        let mut frames = 0;
        while playing(&game) {
            assert_eq!(game.status(), Status::Busy);
            game.frame(Input::None);
            frames += 1;
            assert!(frames < 5000, "the movie never ended");
        }
        assert_eq!(frames, 2147);
        assert_eq!(game.screen().background, None);
        assert_eq!(game.screen().window, None);
        assert!(!game.world().no_text_delay);
        assert_eq!(game.world().text.string(TextBuffer::NameBuffer), PokemonSpecies::NidoranFemale.name());
    }

    #[test]
    fn a_save_mid_movie_resumes_identically() {
        for at in [150, 700, 1900] {
            let mut whole = game();
            for _ in 0..at {
                whole.frame(Input::None);
            }
            let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
            for frame in 0..300 {
                let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
                assert_eq!((whole.screen().frame(), a.audio), (restored.screen().frame(), b.audio), "{at} + {frame}");
            }
        }
    }
}

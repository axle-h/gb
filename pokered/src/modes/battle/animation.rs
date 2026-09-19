//! `engine/battle/animations.asm`: `MoveAnimation`, `PlayAnimation` over the cartridge's command
//! streams, subanimations and frame blocks, and every special effect, a frame at a time.
//!
//! An animation is two queues, as the battle is. A `Step` is a routine of the cartridge's, expanded
//! when it is reached into the `Op`s it performs, since what it does can depend on what the steps
//! before it left (the palette it saves, the counter a ball's shake rewinds). An `Op` is one write
//! or one wait; an op that reads the screen reads it when it runs.
//!
//! Exact: every frame count, every coordinate and tile, the flips, the sounds and when they start.
//! Faithful: `hWhoseTurn` is the animation's own, SGB palettes are the DMG's, and the waits for the
//! tile map to reach VRAM (`BattleAnimCopyTileMapToVRAM` and the `Delay3`s around a screen copy)
//! are not modelled.

use poke_core::battle_anims::{attack_animation, base_coord, frame_block, move_sound, subanimation, tile_id_list,
                              tilemap, AnimCommand, SubanimEntry, FIRST_SE_ID, NO_SOUND};
use poke_core::battle_anims::{frame_block_mode, subanim_type};
use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use crate::audio::data::{sounds, SoundId};
use crate::gfx::layers::{Object, TileMap, Window};
use crate::gfx::tiles::{V_CHARS0, V_CHARS2};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::mode::Ctx;
use crate::systems::battle::Side;
use super::hud;
use super::present::{set_pal_battle, HpBarColours, MonPalettes};

/// Animation ids: a move's own, and past the moves those of `constants/move_constants.asm`.
pub mod anim {
    use poke_core::move_name::PokemonMoveName as M;

    pub const POUND: u8 = M::Pound as u8;
    pub const GROWL: u8 = M::Growl as u8;
    pub const ROAR: u8 = M::Roar as u8;
    pub const ABSORB: u8 = M::Absorb as u8;
    pub const METRONOME: u8 = M::Metronome as u8;
    pub const MINIMIZE: u8 = M::Minimize as u8;
    pub const AMNESIA: u8 = M::Amnesia as u8;
    pub const REST: u8 = M::Rest as u8;
    pub const TELEPORT: u8 = M::Teleport as u8;
    pub const MEGA_PUNCH: u8 = M::MegaPunch as u8;
    pub const MEGA_KICK: u8 = M::MegaKick as u8;
    pub const GUILLOTINE: u8 = M::Guillotine as u8;
    pub const HEADBUTT: u8 = M::Headbutt as u8;
    pub const TAIL_WHIP: u8 = M::TailWhip as u8;
    pub const DISABLE: u8 = M::Disable as u8;
    pub const BLIZZARD: u8 = M::Blizzard as u8;
    pub const BUBBLEBEAM: u8 = M::Bubblebeam as u8;
    pub const HYPER_BEAM: u8 = M::HyperBeam as u8;
    pub const THUNDERBOLT: u8 = M::Thunderbolt as u8;
    pub const REFLECT: u8 = M::Reflect as u8;
    pub const SELFDESTRUCT: u8 = M::Selfdestruct as u8;
    pub const SPORE: u8 = M::Spore as u8;
    pub const EXPLOSION: u8 = M::Explosion as u8;
    pub const ROCK_SLIDE: u8 = M::RockSlide as u8;
    pub const STRUGGLE: u8 = M::Struggle as u8;
    pub const SHOWPIC_ANIM: u8 = 0xA6;
    pub const STATUS_AFFECTED_ANIM: u8 = 0xA7;
    pub const ENEMY_HUD_SHAKE_ANIM: u8 = 0xA9;
    pub const TRADE_BALL_DROP_ANIM: u8 = 0xAA;
    pub const TRADE_BALL_SHAKE_ANIM: u8 = 0xAB;
    pub const TRADE_BALL_TILT_ANIM: u8 = 0xAC;
    pub const TRADE_BALL_POOF_ANIM: u8 = 0xAD;
    pub const XSTATITEM_ANIM: u8 = 0xAE;
    pub const XSTATITEM_DUPLICATE_ANIM: u8 = 0xAF;
    pub const SHRINKING_SQUARE_ANIM: u8 = 0xB0;
    pub const BURN_PSN_ANIM: u8 = 0xBA;
    pub const SLP_PLAYER_ANIM: u8 = 0xBC;
    pub const SLP_ANIM: u8 = 0xBD;
    pub const CONF_PLAYER_ANIM: u8 = 0xBE;
    pub const CONF_ANIM: u8 = 0xBF;
    pub const SLIDE_DOWN_ANIM: u8 = 0xC0;
    pub const TOSS_ANIM: u8 = 0xC1;
    pub const SHAKE_ANIM: u8 = 0xC2;
    pub const POOF_ANIM: u8 = 0xC3;
    pub const BLOCKBALL_ANIM: u8 = 0xC4;
    pub const GREATTOSS_ANIM: u8 = 0xC5;
    pub const ULTRATOSS_ANIM: u8 = 0xC6;
    pub const SHAKE_SCREEN_ANIM: u8 = 0xC7;
    pub const HIDEPIC_ANIM: u8 = 0xC8;
    pub const ROCK_ANIM: u8 = 0xC9;
    pub const BAIT_ANIM: u8 = 0xCA;
}

/// `ANIMATIONTYPE_*`, what `PlayApplyingAttackAnimation` does after the move.
pub mod animation_type {
    pub const NONE: u8 = 0;
    pub const SHAKE_SCREEN_VERTICALLY: u8 = 1;
    pub const SHAKE_SCREEN_HORIZONTALLY_HEAVY: u8 = 2;
    pub const SHAKE_SCREEN_HORIZONTALLY_SLOW: u8 = 3;
    pub const BLINK_ENEMY_MON_SPRITE: u8 = 4;
    pub const SHAKE_SCREEN_HORIZONTALLY_LIGHT: u8 = 5;
    pub const SHAKE_SCREEN_HORIZONTALLY_SLOW_2: u8 = 6;
}

/// The special effects, `SE_*`.
pub mod se {
    pub const WAVY_SCREEN: u8 = 0xD8;
    pub const SUBSTITUTE_MON: u8 = 0xD9;
    pub const SHAKE_BACK_AND_FORTH: u8 = 0xDA;
    pub const SLIDE_ENEMY_MON_OFF: u8 = 0xDB;
    pub const SHOW_ENEMY_MON_PIC: u8 = 0xDC;
    pub const SHOW_MON_PIC: u8 = 0xDD;
    pub const BLINK_ENEMY_MON: u8 = 0xDE;
    pub const HIDE_ENEMY_MON_PIC: u8 = 0xDF;
    pub const FLASH_ENEMY_MON_PIC: u8 = 0xE0;
    pub const DELAY_ANIMATION_10: u8 = 0xE1;
    pub const SPIRAL_BALLS_INWARD: u8 = 0xE2;
    pub const SHAKE_ENEMY_HUD_2: u8 = 0xE3;
    pub const SHAKE_ENEMY_HUD: u8 = 0xE4;
    pub const SLIDE_MON_HALF_OFF: u8 = 0xE5;
    pub const PETALS_FALLING: u8 = 0xE6;
    pub const LEAVES_FALLING: u8 = 0xE7;
    pub const TRANSFORM_MON: u8 = 0xE8;
    pub const SLIDE_MON_DOWN_AND_HIDE: u8 = 0xE9;
    pub const MINIMIZE_MON: u8 = 0xEA;
    pub const BOUNCE_UP_AND_DOWN: u8 = 0xEB;
    pub const SHOOT_MANY_BALLS_UPWARD: u8 = 0xEC;
    pub const SHOOT_BALLS_UPWARD: u8 = 0xED;
    pub const SQUISH_MON_PIC: u8 = 0xEE;
    pub const HIDE_MON_PIC: u8 = 0xEF;
    pub const LIGHT_SCREEN_PALETTE: u8 = 0xF0;
    pub const RESET_MON_POSITION: u8 = 0xF1;
    pub const MOVE_MON_HORIZONTALLY: u8 = 0xF2;
    pub const BLINK_MON: u8 = 0xF3;
    pub const SLIDE_MON_OFF: u8 = 0xF4;
    pub const FLASH_MON_PIC: u8 = 0xF5;
    pub const SLIDE_MON_DOWN: u8 = 0xF6;
    pub const SLIDE_MON_UP: u8 = 0xF7;
    pub const FLASH_SCREEN_LONG: u8 = 0xF8;
    pub const DARKEN_MON_PALETTE: u8 = 0xF9;
    pub const WATER_DROPLETS_EVERYWHERE: u8 = 0xFA;
    pub const SHAKE_SCREEN: u8 = 0xFB;
    pub const RESET_SCREEN_PALETTE: u8 = 0xFC;
    pub const DARK_SCREEN_PALETTE: u8 = 0xFD;
    pub const DARK_SCREEN_FLASH: u8 = 0xFE;
}

const POKE_BALL: u8 = 0x04;
const GREAT_BALL: u8 = 0x03;
const ULTRA_BALL: u8 = 0x02;
/// `wAnimPalette` and the `rOBP0` `SetAnimationPalette` writes, off an SGB.
const ANIM_PALETTE: u8 = 0xE4;
const OBP1_PALETTE: u8 = 0x6C;
/// `rWY` and `rWX` as the battle keeps them.
const WX: u8 = 7;
/// `rWY` past the last line, which hides the window.
const WY_HIDDEN: u8 = 0x90;
/// The base tile id of the battle animation tiles in `vSprites`.
const ANIM_TILE: u8 = 0x31;
const PIC: usize = 7;
const BLANK: u8 = UiSurface::BLANK;

/// What an animation reads from the battle, as it stood when the animation was called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimBattle {
    /// `wBattleMonSpecies` and `wEnemyMonSpecies`.
    pub player_species: PokemonSpecies,
    pub enemy_species: PokemonSpecies,
    /// `wDamageMultipliers`, which picks the damage sound.
    pub damage_multipliers: u8,
    /// `wIsInBattle` is 2.
    pub trainer_battle: bool,
    /// `wCurItem`, the ball thrown.
    pub item: u8,
    /// `wPokeBallAnimData`: how many parts of the throw to play and how many shakes.
    pub ball_data: u8,
    /// `wOptions`' battle animation bit, clear.
    pub animations_on: bool,
    /// `hSCX`, which no window covers while the screen waves.
    #[serde(default)]
    pub h_scx: u8,
    /// What `ChangeMonPic`'s `SET_PAL_BATTLE` sends: the mons as the step left them, and the bars
    /// as the HUDs last drew them.
    #[serde(default)]
    pub mons: MonPalettes,
    #[serde(default)]
    pub hp_bar_colours: HpBarColours,
}

/// The routines the battle calls an animation through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Routine {
    /// `MoveAnimation` with `wAnimationID` and `wAnimationType`.
    MoveAnimation { id: u8, kind: u8 },
    /// `PlayMoveAnimation`: its `Delay3` before `MoveAnimation`, which the effect routines that
    /// reach `MoveAnimation` through the predef do not have.
    PlayMoveAnimation { id: u8, kind: u8 },
    /// `HideSubstituteShowMonAnim`: the substitute slides down if it broke, or off if it stands, and
    /// the mon comes back, minimized if it was.
    HideSubstituteShowMon { substitute_up: bool, minimized: bool },
    /// `ReshowSubstituteAnim`.
    ReshowSubstitute,
    /// `AnimationSubstitute`.
    Substitute,
    /// `SubstituteEffect_`'s choice: the move's own animation, or with animations off the doll.
    SubstituteEffect { id: u8 },
    /// `AnimationTransformMon`.
    TransformMon,
    /// `TransformEffect_`'s choice: the move's own animation, or with animations off the picture
    /// changed.
    TransformEffect { id: u8 },
    /// `AnimationMinimizeMon`.
    MinimizeMon,
    /// `AnimationSlideEnemyMonOff`, as `EnemyRan` calls it on the player's turn.
    SlideEnemyMonOff,
    /// `PredefShakeScreenHorizontally` with `b`.
    ShakeScreenHorizontally(u8),
    /// `MarowakAnim`: the ghost's picture flashes and fades out, and Marowak's fades in.
    Marowak,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Step {
    WaitForSound,
    /// `PlayAnimation` of `wAnimationID`.
    PlayAnimation,
    Command(AnimCommand),
    /// The next frame block of the subanimation playing.
    FrameBlock,
    /// `PlayAnimation`'s `pop af; ldh [rOBP0], a` after a subanimation.
    RestoreObp0(u8),
    SpecialEffect(u8),
    /// `PlayApplyingAttackAnimation`.
    ApplyingAttack,
    /// `TossBallAnimation`'s next animation from `.PokeBallAnimations`, `left` still to play.
    Toss { next: usize, left: u8 },
    /// A sound and then the block ball, in a trainer battle.
    BlockBall,
    /// `CallWithTurnFlipped`'s flip, either side of what it calls.
    FlipTurn,
    SetId(u8),
    Routine(Routine),
    /// `MoveAnimation.animationFinished`.
    Finish,
}

/// Tiles of one of the two pictures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Pic {
    /// `LoadMonFrontSprite` or `LoadMonBackPic` of a species.
    Species(PokemonSpecies),
    /// `wTempPic`, 49 tiles in the picture's own column-major order.
    Temp(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Op {
    /// `DelayFrames`.
    Wait(u16),
    WaitForSound,
    Bgp(u8),
    Obp0(u8),
    Obp1(u8),
    /// `rOBP0` exclusive-or'd, the flash of an Ultra or Master Ball.
    Obp0Xor(u8),
    /// `AnimationFlashScreen`'s `push af` and `pop af` of `rBGP`.
    SaveBgp,
    RestoreBgp,
    /// The screen split into window and background, the window at `rWX`/`rWY` showing the tile
    /// map from `first_row` and the background `vBGMap0`, a copy of the tile map or blank.
    Split { wx: u8, wy: u8, first_row: u8, background: Background },
    Wx(u8),
    Wy(u8),
    Scx(u8),
    /// `hSCX` moved by a signed step.
    ScxBy(u8),
    LineScx(Vec<u8>),
    /// A refreshed copy of the tile map into `vBGMap0`.
    CopyToBackground,
    /// The window and background back to the tile map, unscrolled.
    Unsplit,
    Sound(SoundId),
    /// `rAUD1SWEEP` written behind the engine's back.
    Sweep(u8),
    /// `GetMoveSound` and `PlaySound` for a command's sound byte.
    MoveSound(u8),
    /// `PlayApplyingAttackSound`, after its `WaitForSoundToFinish`.
    ApplyingAttackSound,
    LoadAnimTiles(u8),
    /// OAM entries from `index`.
    Objects { index: usize, objects: Vec<Object> },
    /// `CopyData` of `count` OAM entries from `from` to `to`.
    CopyObjects { from: usize, to: usize, count: usize },
    ClearSprites,
    /// `TradeShakePokeball`'s and `TradeJumpPokeball`'s step of the ball's four objects.
    MoveBall(u8),
    /// `CopyVideoData` of `vBackPic` to `vSprites`.
    BackPicToSprites,
    /// `CopyMonPicFromBGToSpriteVRAM`'s copy of `vFrontPic`.
    FrontPicToSprites,
    Obp1Xor(u8),
    Clear { x: usize, y: usize, width: usize, height: usize },
    /// `CopyTileIDs` of a tile id list's first `rows` rows, each id plus `base`.
    TileIds { x: usize, y: usize, list: u8, rows: usize, base: u8 },
    /// `AnimCopyRowLeft` or `AnimCopyRowRight` from `(x, y)` over `count` tiles, and the blank
    /// `_AnimationSquishMonPic` leaves behind, if any.
    CopyRow { x: usize, y: usize, count: usize, left: bool, blank: bool },
    /// One pass of `_AnimationSlideMonOff`'s tile loop.
    SlideOff(Side),
    /// One pass of `_AnimationSlideMonUp`, with the bottom row's leftmost tile.
    SlideUp { side: Side, bottom_left: u8 },
    LoadPic { side: Side, pic: Pic },
    /// `wTileMapBackup` or `wTileMapBackup2` written, which the battle keeps.
    SaveScreen(u8),
    /// `RunPaletteCommand` of `SET_PAL_BATTLE`.
    SetPalBattle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Background {
    Blank,
    TileMap,
}

/// The subanimation playing: `wSubAnimCounter`, `wSubAnimSubEntryAddr` as an entry index,
/// `wSubAnimTransform`, `wSubAnimFrameDelay` and `wFBDestAddr` as an OAM index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Sub {
    entries: Vec<SubanimEntry>,
    index: usize,
    counter: u8,
    transform: u8,
    delay: u8,
    oam: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    battle: AnimBattle,
    /// `hWhoseTurn` for the routines inside.
    turn: Side,
    /// `wAnimationID` and `wAnimationType`.
    id: u8,
    kind: u8,
    steps: VecDeque<Step>,
    ops: VecDeque<Op>,
    wait: u16,
    waiting_for_sound: bool,
    sub: Option<Sub>,
    /// `wNumShakes`.
    num_shakes: u8,
    saved_bgp: u8,
    /// `wTileMapBackup` and `wTileMapBackup2` as the animation left them, for the battle to take.
    pub screen1: Option<UiSurface>,
    pub screen2: Option<UiSurface>,
    /// `hSCX` as the animation left it, where it wrote it.
    pub h_scx: Option<u8>,
}

/// `LoadSubanimation`'s `wSubAnimTransform`, `wSubAnimCounter` and the entry it starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubanimStart {
    pub transform: u8,
    pub counter: u8,
    pub index: usize,
}

/// `LoadSubanimation`: on the player's turn an enemy subanimation is flipped and any other played as
/// written; on the enemy's turn an enemy subanimation is played as written and any other takes its
/// own type. A reversed one starts from its last entry.
pub fn load_subanimation(id: u8, turn: Side) -> SubanimStart {
    let subanimation = subanimation(id);
    let transform = match (subanimation.kind, turn) {
        (subanim_type::ENEMY, Side::Player) => subanim_type::HFLIP,
        (subanim_type::ENEMY, Side::Enemy) => subanim_type::NORMAL,
        (_, Side::Player) => subanim_type::NORMAL,
        (kind, Side::Enemy) => kind,
    };
    let counter = subanimation.entries.len() as u8;
    let index = if transform == subanim_type::REVERSE { counter as usize - 1 } else { 0 };
    SubanimStart { transform, counter, index }
}

/// `DrawFrameBlock`'s objects for frame block `id` at base coordinate `base`, under `transform`.
pub fn draw_frame_block(id: u8, base: u8, transform: u8) -> Vec<Object> {
    let (base_y, base_x) = base_coord(base);
    frame_block(id).iter().map(|tile| {
        let tile_id = tile.tile.wrapping_add(ANIM_TILE);
        match transform {
            subanim_type::HVFLIP => Object {
                y: 136u8.wrapping_sub(base_y.wrapping_add(tile.y)),
                x: 168u8.wrapping_sub(base_x.wrapping_add(tile.x)),
                tile: tile_id,
                attributes: match tile.flags {
                    0 => Object::Y_FLIP | Object::X_FLIP,
                    Object::X_FLIP => Object::Y_FLIP,
                    Object::Y_FLIP => Object::X_FLIP,
                    _ => 0,
                },
            },
            subanim_type::HFLIP => Object {
                y: base_y.wrapping_add(tile.y).wrapping_add(40),
                x: 168u8.wrapping_sub(base_x.wrapping_add(tile.x)),
                tile: tile_id,
                attributes: tile.flags ^ Object::X_FLIP,
            },
            subanim_type::COORDFLIP => Object {
                y: 136u8.wrapping_sub(base_y).wrapping_add(tile.y),
                x: 168u8.wrapping_sub(base_x).wrapping_add(tile.x),
                tile: tile_id,
                attributes: tile.flags,
            },
            _ => Object { y: base_y.wrapping_add(tile.y), x: base_x.wrapping_add(tile.x), tile: tile_id, attributes: tile.flags },
        }
    }).collect()
}

/// `GetMoveSound` for a command's sound byte during animation `id` on `turn`: the sound and the
/// frequency and tempo modifiers it plays at. Growl and Roar play the mon's cry, bent further.
pub fn move_sound_of(index: u8, id: u8, turn: Side, player: PokemonSpecies, enemy: PokemonSpecies) -> (SoundId, u8, u8) {
    let row = move_sound(index);
    if !matches!(id, anim::GROWL | anim::ROAR) {
        return (SoundId(row.sound), row.pitch, row.tempo);
    }
    let species = match turn { Side::Player => player, Side::Enemy => enemy };
    let cry = crate::audio::data::Cry::of_species_index(species as u8);
    (cry.sound, cry.frequency_modifier.wrapping_add(row.pitch), cry.tempo_modifier.wrapping_add(row.tempo))
}

/// The top left of the side's picture and its first tile id.
fn pic_at(side: Side) -> (usize, usize, u8) {
    match side {
        Side::Player => (1, 5, hud::BACK_PIC_TILE),
        Side::Enemy => (12, 0, hud::FRONT_PIC_TILE),
    }
}

impl Animation {
    pub fn new(routine: Routine, turn: Side, battle: AnimBattle) -> Self {
        let mut animation = Self {
            battle,
            turn,
            id: 0,
            kind: 0,
            steps: VecDeque::new(),
            ops: VecDeque::new(),
            wait: 0,
            waiting_for_sound: false,
            sub: None,
            num_shakes: 0,
            saved_bgp: 0,
            screen1: None,
            screen2: None,
            h_scx: None,
        };
        animation.steps.push_back(Step::Routine(routine));
        animation
    }

    /// One frame. True once the animation is over, in this frame.
    pub fn update(&mut self, ctx: &mut Ctx) -> bool {
        let instant = ctx.pacing == crate::Pacing::Instant;
        loop {
            if self.wait > 0 {
                self.wait -= 1;
                if self.wait > 0 {
                    return false;
                }
            }
            if self.waiting_for_sound {
                if !instant && !ctx.audio.sound_finished() {
                    return false;
                }
                self.waiting_for_sound = false;
            }
            if let Some(op) = self.ops.pop_front() {
                self.run(op, ctx);
                if instant {
                    self.wait = 0;
                    self.waiting_for_sound = false;
                } else if self.wait > 0 {
                    self.wait += 1;
                }
                continue;
            }
            let Some(step) = self.steps.pop_front() else { return true };
            self.expand(step, ctx);
        }
    }

    fn op(&mut self, op: Op) {
        self.ops.push_back(op);
    }

    fn call(&mut self, steps: impl IntoIterator<Item = Step>) {
        let steps: Vec<Step> = steps.into_iter().collect();
        for step in steps.into_iter().rev() {
            self.steps.push_front(step);
        }
    }

    fn expand(&mut self, step: Step, ctx: &mut Ctx) {
        match step {
            Step::WaitForSound => self.op(Op::WaitForSound),
            Step::SetId(id) => self.id = id,
            Step::FlipTurn => self.turn = self.turn.other(),
            Step::Routine(routine) => self.routine(routine),
            Step::PlayAnimation => {
                let commands = attack_animation(self.id).into_iter().map(Step::Command);
                self.call(commands);
            }
            Step::Command(AnimCommand::SpecialEffect { id, sound }) => {
                if sound != NO_SOUND {
                    self.op(Op::MoveSound(sound));
                }
                self.call([Step::SpecialEffect(id)]);
            }
            Step::Command(AnimCommand::Subanimation { tileset, delay, sound, id }) => {
                let saved = ctx.screen.effects.obp0;
                self.op(Op::Obp0(ANIM_PALETTE));
                self.op(Op::LoadAnimTiles(tileset));
                let start = load_subanimation(id, self.turn);
                let entries = subanimation(id).entries;
                self.sub = Some(Sub { entries, index: start.index, counter: start.counter, transform: start.transform, delay, oam: 0 });
                if sound != NO_SOUND {
                    self.op(Op::MoveSound(sound));
                }
                self.call([Step::FrameBlock, Step::RestoreObp0(saved)]);
            }
            Step::RestoreObp0(saved) => self.op(Op::Obp0(saved)),
            Step::FrameBlock => self.frame_block(),
            Step::SpecialEffect(id) => self.special_effect(id, ctx),
            Step::ApplyingAttack => self.applying_attack(),
            Step::Toss { next, left } => {
                if left == 0 {
                    return;
                }
                const POKE_BALL_ANIMATIONS: [u8; 5] = [anim::POOF_ANIM, anim::HIDEPIC_ANIM, anim::SHAKE_ANIM, anim::POOF_ANIM, anim::SHOWPIC_ANIM];
                self.call([Step::SetId(POKE_BALL_ANIMATIONS[next]), Step::PlayAnimation, Step::Toss { next: next + 1, left: left - 1 }]);
            }
            Step::BlockBall => {
                self.op(Op::Sound(sounds::SFX_FAINT_THUD));
                self.call([Step::SetId(anim::BLOCKBALL_ANIM), Step::PlayAnimation]);
            }
            Step::Finish => self.op(Op::WaitForSound),
        }
    }

    fn routine(&mut self, routine: Routine) {
        match routine {
            Routine::PlayMoveAnimation { id, kind } => {
                self.op(Op::Wait(3));
                self.routine(Routine::MoveAnimation { id, kind });
            }
            Routine::MoveAnimation { id, kind } => {
                self.id = id;
                self.kind = kind;
                self.op(Op::WaitForSound);
                // `SetAnimationPalette`.
                self.op(Op::Obp0(ANIM_PALETTE));
                self.op(Op::Obp1(OBP1_PALETTE));
                if id == 0 {
                    return self.call([Step::Finish]);
                }
                if id == anim::TOSS_ANIM {
                    return self.toss_ball_animation();
                }
                if self.battle.animations_on {
                    // `ShareMoveAnimations`.
                    if self.turn == Side::Enemy {
                        match id {
                            anim::AMNESIA => self.id = anim::CONF_ANIM,
                            anim::REST => self.id = anim::SLP_ANIM,
                            _ => {}
                        }
                    }
                    self.call([Step::PlayAnimation, Step::ApplyingAttack, Step::Finish]);
                } else {
                    self.op(Op::Wait(30));
                    self.call([Step::ApplyingAttack, Step::Finish]);
                }
            }
            Routine::HideSubstituteShowMon { substitute_up, minimized } => {
                if substitute_up {
                    self.slide_mon_off(8, 3);
                } else {
                    self.slide_mon_down();
                }
                if minimized {
                    self.minimize_mon();
                } else {
                    self.flash_mon_pic();
                    self.show_mon_pic();
                }
            }
            Routine::ReshowSubstitute => {
                self.slide_mon_off(8, 3);
                self.substitute();
                self.show_mon_pic();
            }
            Routine::Substitute => self.substitute(),
            Routine::SubstituteEffect { id } if self.battle.animations_on => self.routine(Routine::MoveAnimation { id, kind: 0 }),
            Routine::SubstituteEffect { .. } => self.substitute(),
            Routine::TransformEffect { id } if self.battle.animations_on => self.routine(Routine::MoveAnimation { id, kind: 0 }),
            Routine::TransformEffect { .. } => self.routine(Routine::TransformMon),
            Routine::TransformMon => self.change_mon_pic(self.battle.enemy_species, self.battle.player_species),
            Routine::MinimizeMon => self.minimize_mon(),
            Routine::SlideEnemyMonOff => {
                self.turn = self.turn.other();
                self.slide_mon_off(8, 3);
                self.turn = self.turn.other();
            }
            Routine::ShakeScreenHorizontally(b) => self.shake_horizontally(b),
            Routine::Marowak => self.marowak(),
        }
    }

    /// `MarowakAnim`. The tile map is not transferred between the ghost leaving it and the end, so
    /// Marowak's tile ids go down last.
    fn marowak(&mut self) {
        let objects: Vec<Object> = (0..6u8).flat_map(|column| (0..6u8).map(move |row| Object {
            y: 0x10 + 8 * (row + 1),
            x: 0x70 + 8 * column,
            tile: 8 + column * PIC as u8 + row,
            attributes: Object::OBP1,
        })).collect();
        self.ops.extend([Op::Obp1(0xE4), Op::FrontPicToSprites, Op::Objects { index: 0, objects: objects.clone() }]);
        self.op(Op::Clear { x: 12, y: 0, width: PIC, height: PIC });
        // `ChangeMonPic` for the enemy, with the tile ids held back.
        self.ops.extend([Op::LoadPic { side: Side::Enemy, pic: Pic::Species(PokemonSpecies::Marowak) }, Op::SetPalBattle]);
        for _ in 0..8 {
            self.ops.extend([Op::Obp1Xor(0x80), Op::Wait(10)]);
        }
        let mut obp1 = 0xE4u8;
        while obp1 != 0 {
            obp1 <<= 2;
            self.ops.extend([Op::Wait(10), Op::Obp1(obp1)]);
        }
        self.ops.extend([Op::ClearSprites, Op::FrontPicToSprites, Op::Objects { index: 0, objects }]);
        let mut b = 0xE4u8;
        while b != 0 {
            obp1 = (b & 2) << 6 | (b & 1) << 6 | obp1 >> 2;
            b >>= 2;
            self.ops.extend([Op::Wait(10), Op::Obp1(obp1)]);
        }
        self.ops.extend([
            Op::TileIds { x: 12, y: 0, list: tilemap::MON_PIC, rows: PIC, base: hud::FRONT_PIC_TILE },
            Op::ClearSprites,
        ]);
    }

    /// `TossBallAnimation`.
    fn toss_ball_animation(&mut self) {
        if self.battle.trainer_battle {
            return self.call([Step::SetId(anim::TOSS_ANIM), Step::PlayAnimation, Step::BlockBall, Step::Finish]);
        }
        let parts = self.battle.ball_data >> 4;
        self.num_shakes = self.battle.ball_data & 0xF;
        let toss = match self.battle.item {
            POKE_BALL => anim::TOSS_ANIM,
            GREAT_BALL => anim::GREATTOSS_ANIM,
            _ => anim::ULTRATOSS_ANIM,
        };
        self.call([Step::SetId(toss), Step::PlayAnimation, Step::Toss { next: 0, left: parts.wrapping_sub(1) }, Step::Finish]);
    }

    /// `PlayApplyingAttackAnimation`.
    fn applying_attack(&mut self) {
        use animation_type::*;
        let sound = [Op::WaitForSound, Op::ApplyingAttackSound];
        match self.kind {
            NONE => {}
            SHAKE_SCREEN_VERTICALLY => {
                self.ops.extend(sound);
                self.shake_vertically(8);
            }
            SHAKE_SCREEN_HORIZONTALLY_HEAVY => {
                self.ops.extend(sound);
                self.shake_horizontally(8);
            }
            SHAKE_SCREEN_HORIZONTALLY_SLOW => self.shake_horizontally_slow(6, 2),
            BLINK_ENEMY_MON_SPRITE => {
                self.ops.extend(sound);
                self.turn = self.turn.other();
                self.blink_mon();
                self.turn = self.turn.other();
            }
            SHAKE_SCREEN_HORIZONTALLY_LIGHT => {
                self.ops.extend(sound);
                self.shake_horizontally(2);
            }
            SHAKE_SCREEN_HORIZONTALLY_SLOW_2 => self.shake_horizontally_slow(3, 2),
            _ => {}
        }
    }

    /// `DrawFrameBlock` for the subanimation's current entry, then `DoSpecialEffectByAnimationId`,
    /// then `PlaySubanimation`'s count and step.
    fn frame_block(&mut self) {
        let sub = self.sub.as_mut().expect("a subanimation");
        let entry = sub.entries[sub.index];
        let objects = draw_frame_block(entry.frame_block, entry.base_coord, sub.transform);
        let count = objects.len();
        let index = sub.oam;
        let delay = sub.delay;
        self.ops.push_back(Op::Objects { index, objects });
        let sub = self.sub.as_mut().expect("a subanimation");
        match entry.mode {
            frame_block_mode::MODE_02 => sub.oam += count,
            mode => {
                self.ops.push_back(Op::Wait(if delay == 0 { 256 } else { delay as u16 }));
                match mode {
                    frame_block_mode::MODE_03 => sub.oam += count,
                    frame_block_mode::MODE_04 => {}
                    _ => {
                        if self.id != anim::GROWL {
                            self.ops.extend([Op::Wait(1), Op::ClearSprites]);
                        }
                        sub.oam = 0;
                    }
                }
            }
        }
        self.special_effect_by_animation_id();
        let sub = self.sub.as_mut().expect("a subanimation");
        sub.counter = sub.counter.wrapping_sub(1);
        if sub.counter == 0 {
            self.sub = None;
            return;
        }
        if sub.transform == subanim_type::REVERSE {
            sub.index = sub.index.wrapping_sub(1);
        } else {
            sub.index = sub.index.wrapping_add(1);
        }
        self.steps.push_front(Step::FrameBlock);
    }

    /// `DoSpecialEffectByAnimationId`, from `AnimationIdSpecialEffects`.
    fn special_effect_by_animation_id(&mut self) {
        let counter = self.sub.as_ref().expect("a subanimation").counter;
        match self.id {
            anim::MEGA_PUNCH | anim::GUILLOTINE | anim::MEGA_KICK | anim::HEADBUTT | anim::DISABLE | anim::BUBBLEBEAM
            | anim::REFLECT | anim::SPORE => self.flash_screen(),
            anim::TAIL_WHIP => {
                self.sub.as_mut().expect("a subanimation").counter = 1;
                self.op(Op::Wait(20));
            }
            anim::GROWL => {
                self.op(Op::CopyObjects { from: 0, to: 4, count: 4 });
                if counter == 1 {
                    self.ops.extend([Op::Wait(1), Op::ClearSprites]);
                }
            }
            anim::BLIZZARD => {
                if matches!(counter, 13 | 9 | 5 | 1) {
                    self.flash_screen();
                }
            }
            anim::HYPER_BEAM => {
                if counter & 3 == 0 {
                    self.flash_screen();
                }
            }
            anim::THUNDERBOLT => {
                if counter & 7 == 0 {
                    self.flash_screen();
                }
            }
            anim::SELFDESTRUCT | anim::EXPLOSION => {
                if counter == 1 {
                    self.hide_mon_pic();
                } else if counter & 3 == 0 {
                    self.flash_screen();
                }
            }
            anim::ROCK_SLIDE => match counter {
                12.. => {}
                8.. => {
                    self.shake_horizontally(1);
                    self.shake_vertically(1);
                }
                1 => self.flash_screen(),
                _ => {}
            },
            anim::TRADE_BALL_DROP_ANIM => {
                if counter == 6 {
                    self.op(Op::Clear { x: 7, y: 2, width: PIC, height: PIC });
                }
            }
            anim::TRADE_BALL_SHAKE_ANIM => {
                if counter == 1 {
                    for distance in rom_slice(sym::BallMoveDistances1).iter().copied().take_while(|&d| d != 0xFF) {
                        self.ops.extend([Op::MoveBall(distance), Op::Wait(3)]);
                    }
                    self.ops.extend([Op::Wait(1), Op::ClearSprites, Op::Sound(sounds::SFX_TRADE_MACHINE)]);
                }
            }
            anim::TRADE_BALL_TILT_ANIM => {
                let distances: Vec<u8> = rom_slice(sym::BallMoveDistances2).iter().copied().take_while(|&d| d != 0xFF).collect();
                for (i, &distance) in distances.iter().enumerate() {
                    self.op(Op::MoveBall(distance));
                    if matches!(distances.get(i + 1), Some(12) | None) {
                        self.op(Op::Sound(sounds::SFX_SWAP));
                    }
                    self.ops.extend([Op::Wait(5), Op::ScxBy(0xF8)]);
                }
                self.ops.extend([Op::Clear { x: 0, y: 0, width: SCREEN_TILES_X, height: SCREEN_TILES_Y }, Op::Wait(3)]);
            }
            anim::TOSS_ANIM | anim::GREATTOSS_ANIM | anim::ULTRATOSS_ANIM => self.ball_toss_special_effects(counter),
            anim::SHAKE_ANIM => {
                if counter == 4 {
                    self.ops.extend([Op::Sound(sounds::SFX_TINK), Op::Wait(40)]);
                }
                if counter == 1 {
                    self.num_shakes = self.num_shakes.wrapping_sub(1);
                    if self.num_shakes != 0 {
                        let sub = self.sub.as_mut().expect("a subanimation");
                        sub.index = sub.index.wrapping_sub(4);
                        sub.counter = 5;
                    }
                }
            }
            anim::POOF_ANIM => {
                if counter == 5 {
                    self.op(Op::Sound(sounds::SFX_BALL_POOF));
                }
            }
            _ => {}
        }
    }

    /// `DoBallTossSpecialEffects`.
    fn ball_toss_special_effects(&mut self, counter: u8) {
        if self.battle.item < ULTRA_BALL + 1 {
            self.op(Op::Obp0Xor(0b0011_1100));
        }
        if counter == 11 {
            self.op(Op::Sound(sounds::SFX_BALL_TOSS));
        }
        if self.battle.trainer_battle {
            if counter == 3 {
                self.sub.as_mut().expect("a subanimation").counter = 2;
            }
            return;
        }
        if self.battle.ball_data == 0x10 && matches!(counter, 1..=3) {
            for row in 0..PIC {
                self.op(Op::CopyRow { x: 17, y: row, count: PIC, left: false, blank: false });
            }
            self.op(Op::Sweep(0b0000_1000));
        }
    }

    fn special_effect(&mut self, id: u8, ctx: &mut Ctx) {
        assert!(id >= FIRST_SE_ID, "{id:#04x} is not a special effect");
        let flipped = |this: &mut Self, f: fn(&mut Self)| {
            this.turn = this.turn.other();
            f(this);
            this.turn = this.turn.other();
        };
        match id {
            se::DARK_SCREEN_FLASH => self.flash_screen(),
            se::DARK_SCREEN_PALETTE => self.op(Op::Bgp(0x6F)),
            se::RESET_SCREEN_PALETTE => self.op(Op::Bgp(0xE4)),
            se::DARKEN_MON_PALETTE => self.op(Op::Bgp(0xF9)),
            se::LIGHT_SCREEN_PALETTE => self.op(Op::Bgp(0x90)),
            se::SHAKE_SCREEN => self.shake_horizontally(8),
            se::WATER_DROPLETS_EVERYWHERE => self.water_droplets_everywhere(),
            se::FLASH_SCREEN_LONG => self.flash_screen_long(),
            se::SLIDE_MON_UP => self.slide_mon_up(),
            se::SLIDE_MON_DOWN => self.slide_mon_down(),
            se::FLASH_MON_PIC => self.flash_mon_pic(),
            se::SLIDE_MON_OFF => self.slide_mon_off(8, 3),
            se::BLINK_MON => self.blink_mon(),
            se::MOVE_MON_HORIZONTALLY => self.move_mon_horizontally(),
            se::RESET_MON_POSITION => self.reset_mon_position(),
            se::HIDE_MON_PIC => self.hide_mon_pic(),
            se::SQUISH_MON_PIC => self.squish_mon_pic(),
            se::SHOOT_BALLS_UPWARD => self.shoot_balls_upward(),
            se::SHOOT_MANY_BALLS_UPWARD => self.shoot_many_balls_upward(),
            se::BOUNCE_UP_AND_DOWN => {
                for _ in 0..5 {
                    self.slide_mon_down();
                }
                self.show_mon_pic();
            }
            se::MINIMIZE_MON => self.minimize_mon(),
            se::SLIDE_MON_DOWN_AND_HIDE => self.slide_mon_down_and_hide(),
            se::TRANSFORM_MON => self.change_mon_pic(self.battle.enemy_species, self.battle.player_species),
            se::LEAVES_FALLING => {
                let saved = ctx.screen.effects.obp0;
                self.op(Op::Obp0(ANIM_PALETTE));
                self.falling_objects(0x37, 3);
                self.op(Op::Obp0(saved));
            }
            se::PETALS_FALLING => {
                self.falling_objects(0x71, 20);
                self.op(Op::ClearSprites);
            }
            se::SLIDE_MON_HALF_OFF => {
                self.slide_mon_off(4, 4);
                self.op(Op::Wait(3));
            }
            se::SHAKE_ENEMY_HUD | se::SHAKE_ENEMY_HUD_2 => self.shake_enemy_hud(),
            se::SPIRAL_BALLS_INWARD => self.spiral_balls_inward(),
            se::DELAY_ANIMATION_10 => self.op(Op::Wait(10)),
            se::FLASH_ENEMY_MON_PIC => flipped(self, Self::flash_mon_pic),
            se::HIDE_ENEMY_MON_PIC => {
                flipped(self, Self::hide_mon_pic);
                self.op(Op::Wait(3));
            }
            se::BLINK_ENEMY_MON => flipped(self, Self::blink_mon),
            se::SHOW_MON_PIC => self.show_mon_pic(),
            se::SHOW_ENEMY_MON_PIC => flipped(self, Self::show_mon_pic),
            se::SLIDE_ENEMY_MON_OFF => flipped(self, |this| this.slide_mon_off(8, 3)),
            se::SHAKE_BACK_AND_FORTH => self.shake_back_and_forth(),
            se::SUBSTITUTE_MON => self.substitute(),
            se::WAVY_SCREEN => self.wavy_screen(),
            _ => panic!("no special effect {id:#04x}"),
        }
    }

    /// `AnimationFlashScreen`: inverted and white two frames each, then the palette it found.
    fn flash_screen(&mut self) {
        self.ops.extend([Op::SaveBgp, Op::Bgp(0x1B), Op::Wait(2), Op::Bgp(0), Op::Wait(2), Op::RestoreBgp]);
    }

    /// `PredefShakeScreenHorizontally`: `rWX` out by `b` and back, then back out by one less, and
    /// so on, a negative `hMutateWX` clamped to the edge.
    fn shake_horizontally(&mut self, b: u8) {
        self.op(Op::Split { wx: WX, wy: 0, first_row: 0, background: Background::Blank });
        let mut next = 0u8;
        for b in (1..=b).rev() {
            let mut mutate = next;
            for second in [false, true] {
                mutate ^= b;
                let wx = if mutate & 0x80 != 0 { 0 } else { mutate };
                self.ops.extend([Op::Wx(wx.wrapping_add(WX)), Op::Wait(4)]);
                if !second {
                    self.op(Op::Wait(1));
                }
            }
            next = b - 1;
        }
        self.ops.extend([Op::Wx(WX), Op::Unsplit]);
    }

    /// `PredefShakeScreenVertically`: `rWY` down by `b` and back, then by one less.
    fn shake_vertically(&mut self, b: u8) {
        self.op(Op::Split { wx: WX, wy: 0, first_row: 0, background: Background::Blank });
        let mut mutate = 0u8;
        for b in (1..=b).rev() {
            for _ in 0..2 {
                mutate ^= b;
                self.ops.extend([Op::Wy(mutate), Op::Wait(3)]);
            }
            mutate = b - 1;
        }
        self.ops.extend([Op::Wy(0), Op::Unsplit]);
    }

    /// `AnimationShakeScreenHorizontallySlow`: `rWX` a pixel right every two frames `b` times and
    /// back, `c` times.
    fn shake_horizontally_slow(&mut self, b: u8, c: u8) {
        self.op(Op::Split { wx: WX, wy: 0, first_row: 0, background: Background::Blank });
        let mut wx = WX;
        for _ in 0..c {
            for _ in 0..b {
                wx = wx.wrapping_add(1);
                self.ops.extend([Op::Wx(wx), Op::Wait(2)]);
            }
            for _ in 0..b {
                wx = wx.wrapping_sub(1);
                self.ops.extend([Op::Wx(wx), Op::Wait(2)]);
            }
        }
        self.op(Op::Unsplit);
    }

    /// `AnimationFlashScreenLong`: `FlashScreenLongMonochrome` three times, two frames a palette the
    /// first time and one after.
    fn flash_screen_long(&mut self) {
        let palettes: Vec<u8> = rom_slice(sym::FlashScreenLongMonochrome).iter().copied().take_while(|&bgp| bgp != 1).collect();
        for cycle in (1..=3).rev() {
            for &bgp in &palettes {
                self.ops.extend([Op::Bgp(bgp), Op::Wait(if cycle == 3 { 2 } else { 1 })]);
            }
        }
    }

    /// `AnimationWaterDropletsEverywhere`.
    fn water_droplets_everywhere(&mut self) {
        self.op(Op::LoadAnimTiles(0));
        let mut base_x = 0u8.wrapping_sub(16);
        for _ in 0..32 {
            for start_y in [16u8, 24] {
                let mut objects = vec![];
                let mut base_y = start_y;
                loop {
                    base_x = base_x.wrapping_add(27);
                    objects.push(Object { y: base_y, x: base_x, tile: 0x71, attributes: 0 });
                    if base_x < 144 {
                        continue;
                    }
                    base_x = base_x.wrapping_sub(168);
                    base_y = base_y.wrapping_add(16);
                    if base_y >= 112 {
                        break;
                    }
                }
                self.ops.extend([Op::Objects { index: 0, objects }, Op::Wait(1), Op::ClearSprites, Op::Wait(1)]);
            }
        }
    }

    /// `AnimationHideMonPic`.
    fn hide_mon_pic(&mut self) {
        let (x, y, _) = pic_at(self.turn);
        self.op(Op::Clear { x, y, width: PIC, height: PIC });
    }

    /// `CopyPicTiles` of `rows` rows of a tile id list, bottom-aligned in the picture's place.
    fn pic_tiles(&mut self, list: u8, rows: usize) {
        let (x, y, base) = pic_at(self.turn);
        self.op(Op::TileIds { x, y: y + PIC - rows, list, rows, base });
    }

    /// `AnimationShowMonPic`.
    fn show_mon_pic(&mut self) {
        self.pic_tiles(tilemap::MON_PIC, PIC);
        self.op(Op::Wait(3));
    }

    /// `AnimationSlideMonUp`.
    fn slide_mon_up(&mut self) {
        let mut bottom_left = match self.turn { Side::Player => 0x30u8, Side::Enemy => 0xFF };
        for _ in 0..7 {
            bottom_left = bottom_left.wrapping_add(1);
            self.ops.extend([Op::SlideUp { side: self.turn, bottom_left }, Op::Wait(2)]);
        }
    }

    /// `AnimationSlideMonDown`.
    fn slide_mon_down(&mut self) {
        for rows in (1..=PIC).rev() {
            self.pic_tiles(tilemap::MON_PIC, rows);
            self.op(Op::Wait(3));
            self.hide_mon_pic();
        }
    }

    /// `_AnimationSlideMonOff` by `tiles`, `delay` frames a tile.
    fn slide_mon_off(&mut self, tiles: u8, delay: u16) {
        for _ in 0..tiles {
            self.ops.extend([Op::SlideOff(self.turn), Op::Wait(delay)]);
        }
    }

    /// `AnimationBlinkMon`.
    fn blink_mon(&mut self) {
        for _ in 0..6 {
            self.hide_mon_pic();
            self.op(Op::Wait(5));
            self.show_mon_pic();
            self.op(Op::Wait(5));
        }
    }

    /// `AnimationFlashMonPic`: the picture loaded again.
    fn flash_mon_pic(&mut self) {
        self.change_mon_pic(self.battle.player_species, self.battle.enemy_species);
    }

    /// `ChangeMonPic`: the player's picture as `player`'s back, or the enemy's as `enemy`'s front.
    fn change_mon_pic(&mut self, player: PokemonSpecies, enemy: PokemonSpecies) {
        match self.turn {
            Side::Enemy => {
                self.op(Op::LoadPic { side: Side::Enemy, pic: Pic::Species(enemy) });
                self.op(Op::TileIds { x: 12, y: 0, list: tilemap::MON_PIC, rows: PIC, base: hud::FRONT_PIC_TILE });
            }
            Side::Player => {
                self.op(Op::Clear { x: 1, y: 5, width: 8, height: PIC });
                self.op(Op::LoadPic { side: Side::Player, pic: Pic::Species(player) });
                self.pic_tiles(tilemap::MON_PIC, PIC);
            }
        }
        self.op(Op::SetPalBattle);
    }

    /// `AnimationMoveMonHorizontally`.
    fn move_mon_horizontally(&mut self) {
        self.hide_mon_pic();
        let (x, y, base) = pic_at(self.turn);
        let x = match self.turn { Side::Player => x + 1, Side::Enemy => x - 1 };
        self.ops.extend([Op::TileIds { x, y, list: tilemap::MON_PIC, rows: PIC, base }, Op::Wait(3)]);
    }

    /// `AnimationResetMonPosition`.
    fn reset_mon_position(&mut self) {
        let (x, y, _) = pic_at(self.turn);
        let x = match self.turn { Side::Player => x + 1, Side::Enemy => x - 1 };
        self.op(Op::Clear { x, y, width: PIC, height: PIC });
        self.show_mon_pic();
    }

    /// `AnimationSquishMonPic`.
    fn squish_mon_pic(&mut self) {
        let (left_x, right_x, y) = match self.turn {
            Side::Player => (5, 3, 5),
            Side::Enemy => (16, 14, 0),
        };
        for _ in 0..4 {
            for (x, left) in [(left_x, true), (right_x, false)] {
                for row in 0..PIC {
                    self.op(Op::CopyRow { x, y: y + row, count: 3, left, blank: true });
                }
                self.op(Op::Wait(3));
            }
        }
        self.hide_mon_pic();
        self.op(Op::Wait(1));
    }

    /// `_AnimationShootBallsUpward`, `balls` balls tall, from the base coordinates.
    fn shoot_balls(&mut self, base_y: u8, base_x: u8, balls: usize) {
        self.op(Op::LoadAnimTiles(0));
        let mut objects: Vec<Object> = (1..=balls)
            .map(|i| Object { y: base_y.wrapping_add(8 * i as u8), x: base_x, tile: 0x7A, attributes: 0 })
            .collect();
        self.ops.extend([Op::Objects { index: 0, objects: objects.clone() }, Op::Wait(1)]);
        let mut left = balls as u8;
        while left != 0 {
            for object in objects.iter_mut() {
                if object.y == base_y.wrapping_add(8) {
                    object.y = 0;
                    left = left.wrapping_sub(1);
                } else {
                    object.y = object.y.wrapping_sub(4);
                }
            }
            self.ops.extend([Op::Objects { index: 0, objects: objects.clone() }, Op::Wait(1)]);
        }
    }

    /// `AnimationShootBallsUpward`.
    fn shoot_balls_upward(&mut self) {
        let (base_y, base_x) = match self.turn { Side::Player => (6 * 8, 5 * 8), Side::Enemy => (0, 16 * 8) };
        self.shoot_balls(base_y, base_x, 5);
        self.ops.extend([Op::Wait(1), Op::ClearSprites]);
    }

    /// `AnimationShootManyBallsUpward`.
    fn shoot_many_balls_upward(&mut self) {
        let (table, base_y) = match self.turn {
            Side::Player => (sym::UpwardBallsAnimXCoordinatesPlayerTurn, 0x50),
            Side::Enemy => (sym::UpwardBallsAnimXCoordinatesEnemyTurn, 0x28),
        };
        for &base_x in rom_slice(table).iter().take_while(|&&x| x != 0xFF) {
            self.shoot_balls(base_y, base_x, 4);
        }
        self.ops.extend([Op::Wait(1), Op::ClearSprites]);
    }

    /// `AnimationMinimizeMon`.
    fn minimize_mon(&mut self) {
        let mut pic = vec![0u8; PIC * PIC * TILE_BYTES];
        let at = (PIC * 3 + 4) * TILE_BYTES + TILE_BYTES / 4;
        for (row, &byte) in rom_slice(sym::MinimizedMonSprite)[..5].iter().enumerate() {
            pic[at + row * 2] = byte;
            pic[at + row * 2 + 1] = byte;
        }
        self.ops.extend([Op::LoadPic { side: self.turn, pic: Pic::Temp(pic) }, Op::Wait(3)]);
        self.show_mon_pic();
    }

    /// `AnimationSlideMonDownAndHide`.
    fn slide_mon_down_and_hide(&mut self) {
        for list in [tilemap::SLIDE_DOWN_MON_PIC_7X5, tilemap::SLIDE_DOWN_MON_PIC_7X3] {
            self.hide_mon_pic();
            let (_, rows, _) = tile_id_list(list);
            self.pic_tiles(list, rows);
            self.op(Op::Wait(8));
        }
        self.hide_mon_pic();
        self.op(Op::LoadPic { side: self.turn, pic: Pic::Temp(vec![0; PIC * PIC * TILE_BYTES]) });
    }

    /// `AnimationSubstitute`: the picture becomes the doll, and is shown.
    fn substitute(&mut self) {
        let mut pic = vec![0u8; PIC * PIC * TILE_BYTES];
        let sprite = rom_slice(sym::MonsterSprite);
        let placements: [(usize, usize); 4] = match self.turn {
            Side::Enemy => [(0, PIC * 2 + 4), (1, PIC * 3 + 4), (2, PIC * 2 + 5), (3, PIC * 3 + 5)],
            Side::Player => [(4, PIC * 3 + 4), (5, PIC * 4 + 4), (6, PIC * 3 + 5), (7, PIC * 4 + 5)],
        };
        for (tile, at) in placements {
            pic[at * TILE_BYTES..(at + 1) * TILE_BYTES].copy_from_slice(&sprite[tile * TILE_BYTES..(tile + 1) * TILE_BYTES]);
        }
        self.op(Op::LoadPic { side: self.turn, pic: Pic::Temp(pic) });
        self.show_mon_pic();
    }

    /// `AnimationFallingObjects`: `count` objects of tile `tile` down from the top until the first
    /// reaches line 104.
    fn falling_objects(&mut self, tile: u8, count: usize) {
        self.op(Op::LoadAnimTiles(1));
        let initial_x = rom_slice(sym::FallingObjects_InitialXCoords);
        let delta_x = rom_slice(sym::FallingObjects_DeltaXs);
        let mut movement: Vec<u8> = rom_slice(sym::FallingObjects_InitialMovementData)[..count].to_vec();
        let mut objects: Vec<Object> = (0..count)
            .map(|i| Object { y: 8 * (i as u8 + 1), x: initial_x[i], tile, attributes: 0 })
            .collect();
        objects[0].y = 0;
        loop {
            for (object, byte) in objects.iter_mut().zip(movement.iter_mut()) {
                let next = byte.wrapping_add(1);
                *byte = if next & 0x7F == 9 { (next & 0x80) ^ 0x80 } else { next };
                let y = object.y.wrapping_add(2);
                object.y = if y >= 112 { 160 } else { y };
                let delta = delta_x[(*byte & 0x7F) as usize];
                if *byte & 0x80 == 0 {
                    object.x = object.x.wrapping_add(delta);
                    object.attributes = 0;
                } else {
                    object.x = object.x.wrapping_sub(delta);
                    object.attributes = Object::X_FLIP;
                }
            }
            self.ops.extend([Op::Objects { index: 0, objects: objects.clone() }, Op::Wait(3)]);
            if objects[0].y == 104 {
                break;
            }
        }
    }

    /// `AnimationSpiralBallsInward`.
    fn spiral_balls_inward(&mut self) {
        let (base_y, base_x) = match self.turn { Side::Player => (0u8, 0u8), Side::Enemy => (0u8.wrapping_sub(40), 80) };
        self.op(Op::LoadAnimTiles(0));
        let mut objects: Vec<Object> = (1..=3).map(|i| Object { y: 8 * i, x: 0, tile: 0x7A, attributes: 0 }).collect();
        self.op(Op::Objects { index: 0, objects: objects.clone() });
        let coordinates = rom_slice(sym::SpiralBallAnimationCoordinates);
        let mut at = 0;
        'spiral: loop {
            for (i, object) in objects.iter_mut().enumerate() {
                let pair = at + 2 * i;
                // The coordinates run out part way through a round, and what that round wrote reaches
                // OAM only in the frame `AnimationCleanOAM` delays, which the flash below blacks out
                // as it is written: those last positions never show.
                if coordinates[pair] == 0xFF {
                    break 'spiral;
                }
                object.y = base_y.wrapping_add(coordinates[pair]);
                object.x = base_x.wrapping_add(coordinates[pair + 1]);
            }
            self.ops.extend([Op::Objects { index: 0, objects: objects.clone() }, Op::Wait(5)]);
            at += 2;
        }
        self.ops.extend([Op::Wait(1), Op::ClearSprites]);
        self.flash_screen();
    }

    /// `AnimationShakeBackAndForth`.
    fn shake_back_and_forth(&mut self) {
        let (left, right, y) = match self.turn { Side::Player => (0, 2, 5), Side::Enemy => (11, 13, 0) };
        let base = pic_at(self.turn).2;
        for _ in 0..16 {
            for x in [left, right] {
                self.ops.extend([
                    Op::TileIds { x, y, list: tilemap::MON_PIC, rows: PIC, base },
                    Op::Wait(3),
                    Op::Clear { x: left, y, width: 9, height: PIC },
                ]);
            }
        }
    }

    /// `AnimationShakeEnemyHUD`: the window holds everything under the HUD still, the top of the
    /// player's picture is copied into OAM, and the background, the picture gone from it, shakes.
    fn shake_enemy_hud(&mut self) {
        let objects: Vec<Object> = (0..PIC as u8).flat_map(|column| (0..5u8).map(move |row| Object {
            y: 0x30 + 8 * (row + 1),
            x: 0x10 + 8 * column,
            tile: column * PIC as u8 + row,
            attributes: 0,
        })).collect();
        self.ops.extend([
            Op::BackPicToSprites,
            Op::Scx(0),
            Op::Split { wx: WX, wy: 7 * 8, first_row: 7, background: Background::TileMap },
            Op::Objects { index: 0, objects },
        ]);
        self.hide_mon_pic();
        self.op(Op::CopyToBackground);
        for _ in 0..8 {
            self.ops.extend([Op::Scx(2), Op::Wait(2), Op::Scx(0xFE), Op::Wait(2)]);
        }
        self.op(Op::Scx(0));
        self.pic_tiles(tilemap::MON_PIC, PIC);
        self.ops.extend([Op::ClearSprites, Op::Unsplit, Op::SaveScreen(1)]);
    }

    /// `AnimationWavyScreen`: the background a copy of the tile map with the window gone, its lines
    /// scrolled by `WavyScreenLineOffsets`. `WavyScreen_SetSCX` busy-waits for HBlank and writes as
    /// often as HBlank lets it, twice a line, and the loop's check for the last line passes twice
    /// on that line, so each frame takes two of its 255 passes and starts two entries further on.
    /// The first writes land a few lines down, above which `hSCX` holds.
    fn wavy_screen(&mut self) {
        const FIRST_WRITTEN_LINE: usize = 3;
        let offsets: Vec<u8> = rom_slice(sym::WavyScreenLineOffsets).iter().copied().take_while(|&b| b != 0x80).collect();
        self.ops.extend([Op::Wait(1), Op::Split { wx: WX, wy: WY_HIDDEN, first_row: 0, background: Background::TileMap }]);
        for frame in 1..=127usize {
            let lines: Vec<u8> = (0..144).map(|line| match line {
                0..FIRST_WRITTEN_LINE => self.battle.h_scx,
                _ => offsets[(2 * frame + 2 * (line - FIRST_WRITTEN_LINE)) % offsets.len()],
            }).collect();
            self.ops.extend([Op::LineScx(lines), Op::Wait(1)]);
        }
        self.ops.extend([Op::Unsplit, Op::SaveScreen(2)]);
    }

    fn run(&mut self, op: Op, ctx: &mut Ctx) {
        let screen = &mut *ctx.screen;
        let ui = &mut screen.ui;
        match op {
            Op::Wait(frames) => self.wait = frames,
            Op::WaitForSound => self.waiting_for_sound = true,
            Op::Bgp(bgp) => screen.effects.bgp = bgp,
            Op::Obp0(obp0) => screen.effects.obp0 = obp0,
            Op::Obp1(obp1) => screen.effects.obp1 = obp1,
            Op::Obp0Xor(bits) => screen.effects.obp0 ^= bits,
            Op::Split { wx, wy, first_row, background } => {
                let mut tiles = TileMap::filled(BLANK);
                for row in first_row as usize..SCREEN_TILES_Y {
                    for column in 0..SCREEN_TILES_X {
                        tiles.set(column, row - first_row as usize, ui.get(column, row));
                    }
                }
                screen.window = Some(Window { x: wx, y: wy, tiles });
                screen.background = Some(match background {
                    Background::Blank => TileMap::filled(BLANK),
                    Background::TileMap => tile_map(ui),
                });
            }
            Op::Wx(wx) => screen.window.as_mut().expect("a window").x = wx,
            Op::Wy(wy) => screen.window.as_mut().expect("a window").y = wy,
            Op::Scx(scx) => {
                screen.effects.scx = scx;
                self.h_scx = Some(scx);
            }
            Op::ScxBy(step) => screen.effects.scx = screen.effects.scx.wrapping_add(step),
            Op::CopyToBackground => screen.background = Some(tile_map(ui)),
            Op::SaveBgp => self.saved_bgp = screen.effects.bgp,
            Op::RestoreBgp => screen.effects.bgp = self.saved_bgp,
            Op::Obp1Xor(bits) => screen.effects.obp1 ^= bits,
            Op::FrontPicToSprites => {
                for tile in 0..PIC * PIC {
                    let bytes = *screen.tiles.bg(hud::FRONT_PIC_TILE + tile as u8);
                    screen.tiles.load(V_CHARS0 + tile, &bytes);
                }
            }
            Op::BackPicToSprites => {
                for tile in 0..PIC * PIC {
                    let bytes = *screen.tiles.bg(hud::BACK_PIC_TILE + tile as u8);
                    screen.tiles.load(V_CHARS0 + tile, &bytes);
                }
            }
            Op::LineScx(lines) => screen.effects.line_scx = Some(lines),
            Op::Unsplit => {
                screen.window = None;
                screen.background = None;
                screen.effects.line_scx = None;
                screen.effects.scx = 0;
            }
            Op::Sound(id) => ctx.audio.play_sound(id),
            Op::Sweep(value) => ctx.audio.set_sweep(value),
            Op::MoveSound(index) => self.move_sound(index, ctx),
            Op::ApplyingAttackSound => {
                let (frequency, tempo, sound) = match self.battle.damage_multipliers & 0x7F {
                    0 => return,
                    10 => (0x20, 0x30, sounds::SFX_DAMAGE),
                    11.. => (0xE0, 0xFF, sounds::SFX_SUPER_EFFECTIVE),
                    _ => (0x50, 0x01, sounds::SFX_NOT_VERY_EFFECTIVE),
                };
                ctx.audio.set_modifiers(frequency, tempo);
                ctx.audio.play_sound(sound);
            }
            Op::LoadAnimTiles(tileset) => {
                let (pointer, count) = match tileset {
                    1 => (sym::MoveAnimationTiles1, 79),
                    2 => (sym::MoveAnimationTiles0, 64),
                    _ => (sym::MoveAnimationTiles0, 79),
                };
                screen.tiles.load(V_CHARS0 + ANIM_TILE as usize, &rom_slice(pointer)[..count * TILE_BYTES]);
            }
            Op::MoveBall(distance) => {
                for object in screen.sprites.iter_mut().take(4) {
                    object.y = object.y.wrapping_add(distance);
                }
            }
            Op::Objects { index, objects } => {
                if screen.sprites.len() < 40 {
                    screen.sprites.resize(40, Object::default());
                }
                for (i, object) in objects.into_iter().enumerate() {
                    if let Some(slot) = screen.sprites.get_mut(index + i) {
                        *slot = object;
                    }
                }
            }
            Op::CopyObjects { from, to, count } => {
                for i in 0..count {
                    screen.sprites[to + i] = screen.sprites[from + i];
                }
            }
            Op::ClearSprites => crate::gfx::mon_icons::clear_sprites(&mut screen.sprites),
            Op::Clear { x, y, width, height } => hud::clear_area(ui, x, y, width, height),
            Op::TileIds { x, y, list, rows, base } => {
                let (tiles, _, columns) = tile_id_list(list);
                for row in 0..rows {
                    for column in 0..columns {
                        ui.set(x + column, y + row, tiles[row * columns + column].wrapping_add(base));
                    }
                }
            }
            Op::CopyRow { x, y, count, left, blank } => {
                let at = |x: usize| y * SCREEN_TILES_X + x;
                let (get, set) = (|ui: &UiSurface, i: usize| ui.get(i % SCREEN_TILES_X, i / SCREEN_TILES_X),
                                  |ui: &mut UiSurface, i: usize, t: u8| ui.set(i % SCREEN_TILES_X, i / SCREEN_TILES_X, t));
                let start = at(x);
                if left {
                    for i in 0..count {
                        let tile = get(ui, start + i);
                        set(ui, start + i - 1, tile);
                    }
                    if blank {
                        set(ui, start + count - 1, 0x7F);
                    }
                } else {
                    for i in 0..count {
                        let tile = get(ui, start - i);
                        set(ui, start - i + 1, tile);
                    }
                    if blank {
                        set(ui, start + 1 - count, 0x7F);
                    }
                }
            }
            Op::SlideOff(side) => {
                let (x0, y0) = match side { Side::Player => (0, 5), Side::Enemy => (12, 0) };
                for row in 0..PIC {
                    for column in 0..8 {
                        let tile = ui.get(x0 + column, y0 + row);
                        let next = match side {
                            Side::Player => tile.wrapping_add(7),
                            Side::Enemy => tile.wrapping_sub(7),
                        };
                        let limit = match side { Side::Player => 0x61, Side::Enemy => 0x30 };
                        ui.set(x0 + column, y0 + row, if next < limit { next } else { BLANK });
                    }
                }
            }
            Op::SlideUp { side, bottom_left } => {
                let (x0, y0) = match side { Side::Player => (1, 5), Side::Enemy => (12, 0) };
                for row in 0..PIC - 1 {
                    for column in 0..PIC {
                        let tile = ui.get(x0 + column, y0 + row + 1);
                        ui.set(x0 + column, y0 + row, tile);
                    }
                }
                for column in 0..PIC {
                    ui.set(x0 + column, y0 + PIC - 1, bottom_left.wrapping_add(PIC as u8 * column as u8));
                }
            }
            Op::LoadPic { side, pic } => match (side, pic) {
                (Side::Enemy, Pic::Species(species)) => hud::load_front_pic(&mut screen.tiles, species),
                (Side::Player, Pic::Species(species)) => hud::load_back_pic(&mut screen.tiles, species),
                (side, Pic::Temp(bytes)) => screen.tiles.load(V_CHARS2 + pic_at(side).2 as usize, &bytes),
            },
            Op::SetPalBattle => screen.sgb.run(&set_pal_battle(self.battle.hp_bar_colours, self.battle.mons)),
            Op::SaveScreen(1) => self.screen1 = Some(ui.clone()),
            Op::SaveScreen(_) => self.screen2 = Some(ui.clone()),
        }
    }

    /// `GetMoveSound` and `PlaySound`.
    fn move_sound(&mut self, index: u8, ctx: &mut Ctx) {
        let (sound, frequency, tempo) = move_sound_of(index, self.id, self.turn, self.battle.player_species, self.battle.enemy_species);
        ctx.audio.set_modifiers(frequency, tempo);
        ctx.audio.play_sound(sound);
    }
}

/// The tile map as `vBGMap0` holds it after a copy: the 20 by 18, blank beyond.
fn tile_map(ui: &UiSurface) -> TileMap {
    let mut tiles = TileMap::filled(BLANK);
    for row in 0..SCREEN_TILES_Y {
        for column in 0..SCREEN_TILES_X {
            tiles.set(column, row, ui.get(column, row));
        }
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_animation_ids_past_the_moves_are_the_source_s() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/pokered/constants/move_constants.asm")).unwrap();
        let names: Vec<&str> = text.lines().map(str::trim).skip_while(|line| !line.starts_with("DEF NUM_ATTACKS"))
            .filter_map(|line| line.strip_prefix("const ")).map(|rest| rest.split_whitespace().next().unwrap()).collect();
        let id = |name: &str| 0xA6 + names.iter().position(|&n| n == name).unwrap_or_else(|| panic!("{name}")) as u8;
        assert_eq!(anim::BURN_PSN_ANIM, id("BURN_PSN_ANIM"));
        assert_eq!(anim::SLP_PLAYER_ANIM, id("SLP_PLAYER_ANIM"));
        assert_eq!(anim::CONF_ANIM, id("CONF_ANIM"));
        assert_eq!(anim::TOSS_ANIM, id("TOSS_ANIM"));
        assert_eq!(anim::POOF_ANIM, id("POOF_ANIM"));
        assert_eq!(anim::BAIT_ANIM, id("BAIT_ANIM"));
        assert_eq!(anim::XSTATITEM_ANIM, id("XSTATITEM_ANIM"));
        assert_eq!(anim::ENEMY_HUD_SHAKE_ANIM, id("ENEMY_HUD_SHAKE_ANIM"));
    }
}


#[cfg(test)]
mod fixture_tests {
    use crate::fixtures::cases;
    use super::*;

    #[derive(serde::Deserialize)]
    struct FrameBlockInput { frame_block: u8, base_coord: u8, transform: u8 }

    #[test]
    fn every_harvested_case_of_draw_frame_block() {
        for (input, objects, _) in cases::<FrameBlockInput, Vec<[u8; 4]>>(include_str!("../../../fixtures/battle_anims/draw_frame_block.jsonl")) {
            let ours: Vec<[u8; 4]> = draw_frame_block(input.frame_block, input.base_coord, input.transform).iter()
                .map(|object| [object.y, object.x, object.tile, object.attributes]).collect();
            assert_eq!(ours, objects, "frame block {:#04x}, base {:#04x}, transform {}", input.frame_block, input.base_coord, input.transform);
        }
    }

    #[derive(serde::Deserialize)]
    struct SubanimInput { subanimation: u8, turn: Side }

    #[test]
    fn every_harvested_case_of_load_subanimation() {
        for (input, (transform, counter, index), _) in cases::<SubanimInput, (u8, u8, usize)>(include_str!("../../../fixtures/battle_anims/load_subanimation.jsonl")) {
            assert_eq!(load_subanimation(input.subanimation, input.turn), SubanimStart { transform, counter, index },
                "subanimation {} on {:?}", input.subanimation, input.turn);
        }
    }

    #[derive(serde::Deserialize)]
    struct MoveSoundInput { index: u8, animation: u8, turn: Side, player: PokemonSpecies, enemy: PokemonSpecies }

    #[test]
    fn every_harvested_case_of_get_move_sound() {
        for (input, (sound, frequency, tempo), _) in cases::<MoveSoundInput, (u8, u8, u8)>(include_str!("../../../fixtures/battle_anims/get_move_sound.jsonl")) {
            let (ours, f, t) = move_sound_of(input.index, input.animation, input.turn, input.player, input.enemy);
            assert_eq!((ours.0, f, t), (sound, frequency, tempo), "sound {} for animation {:#04x}", input.index, input.animation);
        }
    }
}

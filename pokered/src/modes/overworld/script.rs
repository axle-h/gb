//! The script runtime: the call stack a map's code runs on, what it waits on, and the handle the code
//! is given. `DisplayTextID` and its closing lives here too, since a script calls it exactly as
//! pressing A does.

use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::map_objects::{MapObjects, ObjectKind};
use poke_core::symbols::{pokered_symbols, DmgPointer};
use poke_core::text_script::{decode, far_text, TextBuffer, TextCommand};
use serde::{Deserialize, Serialize};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Transition};
use crate::modes::cursor_menu::CursorMenu;
use crate::modes::pc::PcMenu;
use crate::modes::pc::bills_pc::BillsPc;
use crate::modes::pc::players_pc::PlayerPc;
use crate::modes::pokemart::Pokemart;
use crate::modes::start_menu::StartMenu;
use crate::modes::text_box::TextBox;
use crate::modes::town_map::TownMap;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::scripts::{self, Code, MapStates};
use crate::systems::overworld::map_text::{map_text, map_text_in, text_predef, MapText, TX_SCRIPT_BILLS_PC, TX_SCRIPT_MART,
    TX_SCRIPT_PLAYERS_PC, TX_SCRIPT_POKECENTER_NURSE, TX_SCRIPT_POKECENTER_PC, TX_SCRIPT_PRIZE_VENDOR,
    TX_SCRIPT_VENDING_MACHINE};
use crate::systems::math::{add_bcd, sub_bcd};
use crate::systems::print_num::{print_bcd, BcdFormat};
use crate::systems::overworld::sprites::{NpcPaths, SpriteState, NUM_SPRITES};
use crate::audio::data::{sounds, Sound, SoundId};
use super::{Overworld, Phase, TEXT_POLL_TIMER};

/// `TEXT_MON_FAINTED` to `TEXT_SAFARI_GAME_OVER`: the text ids `DisplayTextID` answers before the map's.
pub const TEXT_MON_FAINTED: u8 = 0xD0;
pub const TEXT_BLACKED_OUT: u8 = 0xD1;
pub const TEXT_REPEL_WORE_OFF: u8 = 0xD2;
pub const TEXT_SAFARI_GAME_OVER: u8 = 0xD3;

/// What a routine does next.
pub enum Flow {
    /// `ret`, or `jp TextScriptEnd`: back to whatever called it.
    Return,
    /// `jp`: on at once with another label.
    Jump(Code),
    /// `call` a routine, then carry on from the label.
    Call(Code, Code),
    /// Something that takes frames, then the label, or a return without one.
    Block(Block, Option<Code>),
}

/// What takes frames.
pub enum Block {
    /// `DelayFrames`.
    Frames(u8),
    /// `WaitForSoundToFinish`.
    Sound,
    /// `PrintText`.
    PrintText(Vec<TextCommand>),
    /// A mode on top until it pops: a menu, a mart.
    Mode(Box<Mode>),
    /// A battle on top, whose end `.battleOccurred` takes.
    Battle(Box<Mode>),
    /// A mode in the overworld's place, which nothing returns from.
    Replace(Box<Mode>),
    /// `WaitForTextScrollButtonPress`.
    TextScrollButton,
    /// `HoldTextDisplayOpen`: until A is let go.
    HoldOpen,
    /// `StopMusic`: until the fade has silenced the music.
    MusicStopped,
    /// A busy wait on `wChannelSoundIDs`: until the channel is no longer playing the sound.
    ChannelPlaying { channel: usize, id: u8 },
    /// A block, then one of the runtime's own routines, then wherever the caller said.
    Chain(Box<Block>, Routine),
}

/// A call or a block not yet told where to carry on.
#[must_use = "a wait carries on somewhere: `then` or `ret`"]
pub struct Then(Pending);

enum Pending {
    Call(Code),
    Block(Block),
}

impl Then {
    pub fn call(routine: impl Into<Code>) -> Self {
        Self(Pending::Call(routine.into()))
    }

    pub fn block(block: Block) -> Self {
        Self(Pending::Block(block))
    }

    /// Carry on from `label` once it is over.
    pub fn then(self, label: impl Into<Code>) -> Flow {
        match self.0 {
            Pending::Call(routine) => Flow::Call(routine, label.into()),
            Pending::Block(block) => Flow::Block(block, Some(label.into())),
        }
    }

    /// Return once it is over: a tail call.
    pub fn ret(self) -> Flow {
        match self.0 {
            Pending::Call(routine) => Flow::Jump(routine),
            Pending::Block(block) => Flow::Block(block, None),
        }
    }
}

/// The runtime's own labels, which a map calls or jumps to like any of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Routine {
    /// `RunMapScript`: the NPC movement script, then the map's.
    MapScript,
    /// The rest of `JoypadOverworld` and the loop body after it.
    AfterRunMapScript,
    /// `.displayDialogue`'s return to `.checkForOpponent`.
    AfterDisplayDialogue,
    DisplayTextId(u8),
    /// The code behind a text id has returned: `wDoNotWaitForButtonPressAfterDisplayingText`.
    AfterTextCode,
    AfterDisplayingTextId,
    HoldTextDisplayOpen,
    CloseTextDisplay,
    CloseTextDisplayRest,
    /// `PickUpItemText`'s `predef PickUpItem`.
    PickUpItem,
    /// `UpdateSprites`, then return.
    UpdateSprites,
    EmotionBubbleEnd,
    CheckFightingMapTrainers,
    AfterEngageBubble,
    DisplayEnemyTrainerTextAndStartBattle,
    StartTrainerBattle,
    EndTrainerBattle,
    /// `TalkToTrainer` with `hl` the trainer's header.
    TalkToTrainer(DmgPointer),
    TalkToTrainerNotYetFought,
    /// `.battleOccurred`'s `DelayFrames 10`, then `EnterMap`.
    EnterMapAfterBattle,
    /// `.allPokemonFainted`'s `RunMapScript`, then `HandleBlackOut`.
    HandleBlackOut,
    /// `HandleBlackOut` once the screen is black.
    BlackOutFaded,
    /// `SpecialEnterMap`'s `DelayFrames 20`, then `EnterMap`.
    SpecialEnterMap,
    /// `Music_Cities1AlternateTempo` after its `DelayFrames 100`.
    Cities1AlternateTempo,
    /// `StopAllSounds`, then return.
    StopAllSounds,
    /// `GivePokemon`, then `AskName`'s question, its yes/no, and the rest of `_AddPartyMon`.
    GivePokemon(PokemonSpecies, u8),
    GivePokemonAskName(PokemonSpecies, u8),
    GivePokemonYesNo(PokemonSpecies, u8),
    GivePokemonAnswered(PokemonSpecies, u8),
    GivePokemonNamed(PokemonSpecies, u8),
    /// `PrintPredefTextID`: `DisplayTextID` over `TextPredefs` rather than the map's texts, each text
    /// in the ROM bank of the routine that asked, which stays loaded.
    PrintPredefTextId { id: u8, bank: u8 },
    /// `jp OverworldLoop` from inside a routine: a hidden event has been dealt with.
    OverworldLoop,
    /// `CheckForHiddenEventOrBookshelfOrCardKeyDoor` has returned without finding a hidden event or a
    /// bookshelf: on to the sprite or sign in front.
    AfterCardKeyText,
    /// `NewBattle` has found no battle, a repel having worn off instead: `CheckWarpsNoCollision`.
    CheckWarpsNoCollision,
    /// `ApplyOutOfBattlePoisonDamage` has returned: `HandleBlackOut`, or `.newBattle`.
    AfterPoison,
    /// `GBFadeOutToBlack` and `GBFadeInFromBlack` from a map's own script, which leaves the map
    /// where it is: the palette to show now, then the rest.
    ScriptFadeOutToBlack(u8),
    ScriptFadeInFromBlack(u8),
    /// `GBFadeOutToWhite` and `GBFadeInFromWhite` from a map's own script, three palettes each.
    ScriptFadeOutToWhite(u8),
    ScriptFadeInFromWhite(u8),
    /// `PlayerPC`, reached from `TX_SCRIPT_PLAYERS_PC`: `PlayerPc::direct`.
    PlayerPc,
    /// `BillsPC_`, from `TX_SCRIPT_BILLS_PC`: `BillsPc::direct`.
    BillsPc,
    /// `ActivatePC`, from `TX_SCRIPT_POKECENTER_PC`: `PcMenu`.
    ActivatePc,
    /// `DisplayTownMap`, after `TownMapText`.
    DisplayTownMap,
    /// `_LeaveMapAnim` and `EnterMapAnim` for a fly, a label of each at a time, with
    /// `DoFlyAnimation`'s own two and the palettes of both fades.
    LeaveMapAnim,
    LeaveMapAnimFlap,
    LeaveMapAnimUp,
    LeaveMapAnimWait,
    LeaveMapAnimAway,
    FadeOutToWhite(u8),
    SpecialWarpFaded,
    EnterMapAnim,
    FadeInFromWhite(u8),
    EnterMapAnimFly,
    EnterMapAnimLanded,
    EnterMapAnimDone,
    FlyAnimStep,
    FlyAnimCoords,
    /// `_LeaveMapAnim` and `EnterMapAnim` where the player spins rather than flies: an escape warp,
    /// a warp pad or a hole, with `PlayerSpinInPlace`'s and `PlayerSpinWhileMovingUpOrDown`'s own
    /// iterations.
    LeaveMapAnimStopped,
    SpinOutInPlace,
    SpinInPlace,
    SpinWhileMoving,
    SpinWhileMovingUp,
    LeaveMapAnimSpun,
    LeaveMapThroughHole,
    LeaveMapThroughHoleHidden,
    LeaveMapThroughHoleDone,
    EnterMapAnimSpin,
    EnterMapAnimSpinDungeon,
    EnterMapAnimDungeon,
    EnterMapAnimSpun,
    EnterMapAnimMusic,
    /// A warp pad's leave animation is over: on into `EnterMap` where a warp's fade would have led.
    WarpPadFaded,
    /// `EnterMap` after the arrival animation: `CheckForceBikeOrSurf` and the rest.
    EnterMapRest,
    /// `SafariZoneGameOver`'s `jp nz, WarpFound2`.
    SafariWarp,
    /// `PlayDefaultMusic`: `WaitForSoundToFinish`, then `PlayDefaultMusicCommon`.
    PlayDefaultMusic,
    PlayDefaultMusicCommon,
    EnterMapEnd,
    StartStep,
    /// The start menu has closed, maybe having used a field move or cast a rod.
    AfterStartMenu,
    /// `FishingInit` after its text, `RodResponse`, and `FishingAnim` a label at a time: the cast
    /// with `wRodResponse`, the shakes left, the text, and the end.
    FishingInitSound(ItemId),
    RodResponse(ItemId),
    FishingCast(u8),
    FishingShake(u8),
    FishingBite,
    FishingText(DmgPointer),
    FishingEnd,
    /// `UsedCut` once `UsedCutText` has printed: the tree out of the map, then `AnimCut`.
    UsedCutAnimation,
    /// `AnimCut` and `AnimateBoulderDust`, with the steps each has left.
    AnimCut(u8),
    DoBoulderDustAnimation,
    AnimateBoulderDust(u8),
    /// `RunMapScript` after the boulder: `RunNPCMovementScript`, then the map's own script.
    RunNpcMovementScript,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Waiting {
    #[default]
    Nothing,
    Frames(u8),
    Sound,
    Child,
    Battle,
    TextScrollButton,
    HoldOpen,
    /// `StopMusic`'s wait for the fade.
    MusicStopped,
    ChannelPlaying { channel: usize, id: u8 },
}

/// The script globals the overworld keeps between passes: where the stack is, and the WRAM flags
/// and bytes the scripts and the trainer and battle routines share.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Runtime {
    pub stack: Vec<Code>,
    pub waiting: Waiting,
    /// What the last mode on top popped with.
    pub outcome: Option<Outcome>,
    pub paths: NpcPaths,
    /// `wUpdateSpritesEnabled` at `$ff`: an emotion bubble holds OAM still.
    pub sprites_frozen: bool,
    /// `BIT_NO_AUTO_TEXT_BOX`.
    pub no_auto_text_box: bool,
    /// `BIT_NO_SPRITE_UPDATES`: the next `DisplayTextIDInit` skips its `UpdateSprites`, and clears it.
    #[serde(default)]
    pub no_sprite_updates: bool,
    /// `wDoNotWaitForButtonPressAfterDisplayingText`.
    pub do_not_wait: bool,
    /// `wOverrideSimulatedJoypadStatesMask`.
    pub override_simulated: Joypad,
    /// `BIT_FORCED_WARP`: a warp an extra check has passed is taken with no direction held, which is
    /// how a current carries a surfing player onto a staircase.
    #[serde(default)]
    pub forced_warp: bool,
    /// `wSpriteIndex`.
    pub sprite_index: u8,
    /// `wCurOpponent`, `wCurEnemyLevel`, `wTrainerNo` and `wEnemyMonOrTrainerClass`.
    pub cur_opponent: u8,
    pub cur_enemy_level: u8,
    pub trainer_no: u8,
    pub enemy_mon_or_trainer_class: u8,
    /// `wEngagedTrainerClass` and `wEngagedTrainerSet`.
    pub engaged_class: u8,
    pub engaged_set: u8,
    /// `wTrainerHeaderPtr` and `wTrainerHeaderFlagBit`.
    pub trainer_header: Option<DmgPointer>,
    pub trainer_header_flag_bit: u8,
    /// `wEndBattleWinTextPointer`.
    pub end_battle_text: Option<DmgPointer>,
    /// `wGymLeaderNo`, the same byte as `wLoneAttackNo`: the leader's music and its last mon's lone
    /// move both come from it.
    pub gym_leader_no: u8,
    /// `BIT_SEEN_BY_TRAINER`, `BIT_TRAINER_BATTLE`, `BIT_USE_CUR_MAP_SCRIPT`, `BIT_TALKED_TO_TRAINER`,
    /// `BIT_PRINT_END_BATTLE_TEXT` and `BIT_UNKNOWN_5_4`.
    pub seen_by_trainer: bool,
    pub trainer_battle: bool,
    pub use_cur_map_script: bool,
    pub talked_to_trainer: bool,
    pub print_end_battle_text: bool,
    pub unknown_5_4: bool,
    /// `BIT_BATTLE_OVER_OR_BLACKOUT`.
    pub battle_over_or_blackout: bool,
    /// `wIsInBattle` at `$ff`: the battle was lost.
    pub lost_battle: bool,
    /// `wBattleResult`, as the last battle left it.
    #[serde(default)]
    pub battle_result: u8,
    /// `BIT_WARP_FROM_CUR_SCRIPT` with `hWarpDestinationMap` and `wDestinationWarpID`.
    #[serde(default)]
    pub script_warp: Option<(u8, u8)>,
    /// `BIT_NO_BATTLES`.
    pub no_battles: bool,
    /// `wBattleType` at `BATTLE_TYPE_OLD_MAN`: the catching lesson, which Oak fights.
    #[serde(default)]
    pub old_man_battle: bool,
    /// `BIT_WILD_ENCOUNTER_COOLDOWN` and `wNumberOfNoRandomBattleStepsLeft`.
    pub wild_encounter_cooldown: bool,
    pub no_random_battle_steps: u8,
    /// `wStepCounter`.
    pub step_counter: u8,
    /// `BIT_NO_MAP_MUSIC`.
    pub no_map_music: bool,
    /// `wNPCMovementScriptPointerTableNum` and `wNPCMovementScriptFunctionNum`.
    pub npc_movement_script_table: u8,
    pub npc_movement_script_function: u8,
    /// `wNumStepsToTake`.
    pub num_steps_to_take: u8,
    /// `wAddedToParty`.
    pub added_to_party: bool,
    /// `_GivePokemon`'s carry: the mon went into the party or a box.
    #[serde(default)]
    pub gave_pokemon: bool,
    /// The mon `LoadEnemyMonData` made for a box, waiting on its nickname.
    #[serde(default)]
    pub box_mon: Option<crate::systems::battle::BattleMon>,
    /// `wCurMapTextPtr` where a script has put another table in the header's place, until the next
    /// map load.
    pub text_pointers: Option<DmgPointer>,
    pub wild_mons: crate::systems::overworld::encounters::WildMons,
    /// `wCurrentMapScriptFlags`' `BIT_CUR_MAP_LOADED_1` and `_2`: set whenever the map is loaded
    /// afresh, for a script that changes its blocks to change them again.
    pub cur_map_loaded: [bool; 2],
    /// `wTileMapBackup`, for `AskName` to put back.
    pub saved_screen: Option<UiSurface>,
    /// `wTileMapBackup2`: `SaveScreenTilesToBuffer2`.
    #[serde(default)]
    pub saved_screen2: Option<UiSurface>,
    /// `wTextPredefFlag`'s `BIT_TEXT_PREDEF`: the next `DisplayTextID` reads `TextPredefs`.
    #[serde(default)]
    pub text_predef: Option<u8>,
    /// `wOutOfBattleBlackout`.
    #[serde(default)]
    pub out_of_battle_blackout: bool,
    /// `TryDoWildEncounter` ran the repel out on this step.
    #[serde(default)]
    pub repel_wore_off: bool,
    /// What the events keep between their frames.
    #[serde(default)]
    pub events: super::events::EventRuntime,
    /// `wMiscFlags`' `BIT_BOULDER_DUST`, `BIT_TRIED_PUSH_BOULDER` and `BIT_PUSHED_BOULDER`, with
    /// `wBoulderSpriteIndex`.
    #[serde(default)]
    pub boulder_dust: bool,
    #[serde(default)]
    pub tried_push_boulder: bool,
    #[serde(default)]
    pub pushed_boulder: bool,
    #[serde(default)]
    pub boulder_sprite: u8,
    /// `wDestinationMap` while a special warp is under way, and which arrival animation `EnterMap`
    /// runs, which also tells `LoadMapData` to leave the music to it.
    #[serde(default)]
    pub fly_destination: Option<poke_core::map::Map>,
    #[serde(default)]
    pub entered_by: Option<SpecialEnter>,
    #[serde(default)]
    pub fly_anim: super::fly::FlyAnim,
    #[serde(default)]
    pub saved_player: super::fly::SavedPlayer,
    #[serde(default)]
    pub spin: super::escape::SpinAnim,
    /// `wDungeonWarpDestinationMap` and `wWhichDungeonWarp`, with `BIT_DUNGEON_WARP`: a hole the
    /// map's script has dropped the player down, which the loop's next pass takes.
    #[serde(default)]
    pub dungeon_warp: Option<(poke_core::map::Map, u8)>,
}

/// `BIT_USED_FLY` and `BIT_DUNGEON_WARP` as `EnterMapAnim` reads them: which way the player arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecialEnter {
    Fly,
    /// An escape warp or a warp pad.
    Spin,
    Dungeon,
}

/// The handle a map's code runs with: the overworld and everything a frame reaches.
pub struct Script<'s, 'c> {
    pub(super) ow: &'s mut Overworld,
    pub(super) ctx: &'s mut Ctx<'c>,
}

/// `FadePal1` is black and `FadePal4` the palette an outdoor map is drawn in.
const BLACK: u8 = 1;
const NORMAL: u8 = 4;
/// `GBFadeOutToWhite` runs `FadePal6` to `FadePal8` and `GBFadeInFromWhite` `FadePal7` back to
/// `FadePal5`, so neither ends on the map's own palette and only the caller's redraw restores it.
const WHITE_OUT_FIRST: u8 = 6;
const WHITE: u8 = 8;
const WHITE_IN_FIRST: u8 = 7;
const WHITE_IN_LAST: u8 = 5;

/// A far text, by its label.
pub fn far(label: &str) -> Vec<TextCommand> {
    far_text(label).unwrap_or_else(|e| panic!("{e}"))
}

/// The text at a label, `TX_FAR`s followed, up to a `text_asm` if it has one.
pub fn text_at(at: DmgPointer) -> Vec<TextCommand> {
    decode(at).unwrap_or_else(|e| panic!("{e}"))
}

impl Script<'_, '_> {
    pub(crate) fn routine(&mut self, routine: Routine) -> Flow {
        let (ow, ctx) = (&mut *self.ow, &mut *self.ctx);
        match routine {
            Routine::MapScript => ow.run_map_script(ctx),
            Routine::DisplayTextId(id) => ow.display_text_id(ctx, id),
            Routine::AfterTextCode => {
                let next = if ow.rt.do_not_wait { Routine::HoldTextDisplayOpen } else { Routine::AfterDisplayingTextId };
                Flow::Jump(next.into())
            }
            Routine::AfterDisplayingTextId => Then::block(Block::TextScrollButton).then(Routine::HoldTextDisplayOpen),
            Routine::HoldTextDisplayOpen => Then::block(Block::HoldOpen).then(Routine::CloseTextDisplay),
            Routine::CloseTextDisplay => {
                ctx.screen.ui.uncover(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y);
                Then::block(Block::Frames(1)).then(Routine::CloseTextDisplayRest)
            }
            Routine::CloseTextDisplayRest => {
                ow.close_text_display_rest(ctx);
                Flow::Return
            }
            Routine::PlayDefaultMusic => Then::block(Block::Sound).then(Routine::PlayDefaultMusicCommon),
            Routine::PlayDefaultMusicCommon => {
                super::play_default_music(ctx);
                Flow::Return
            }
            Routine::AfterStartMenu => {
                ow.surf_step(ctx);
                if let Some(rod) = super::fishing::rod_chosen(ow.rt.outcome) {
                    return ow.fishing_init(ctx, rod);
                }
                match ow.used_field_move(ctx) {
                    Some(flow) => flow,
                    None => Flow::Jump(Routine::CloseTextDisplay.into()),
                }
            }
            Routine::FishingInitSound(rod) => ow.fishing_init_sound(ctx, rod),
            Routine::RodResponse(rod) => ow.rod_response(ctx, rod),
            Routine::FishingCast(response) => ow.fishing_cast(ctx, response),
            Routine::FishingShake(left) => ow.fishing_shake(ctx, left),
            Routine::FishingBite => ow.fishing_bite(ctx),
            Routine::FishingText(text) => ow.fishing_text(text),
            Routine::FishingEnd => ow.fishing_end(ctx),
            Routine::UsedCutAnimation => ow.used_cut_animation(ctx),
            Routine::AnimCut(left) => ow.anim_cut_frame(ctx, left),
            Routine::DoBoulderDustAnimation => ow.do_boulder_dust_animation(ctx),
            Routine::AnimateBoulderDust(left) => ow.animate_boulder_dust(ctx, left),
            Routine::RunNpcMovementScript => ow.run_npc_movement_script(ctx),
            Routine::PickUpItem => ow.pick_up_item(ctx),
            Routine::UpdateSprites => {
                ow.update_sprites(ctx);
                Flow::Return
            }
            Routine::EmotionBubbleEnd => {
                ow.rt.sprites_frozen = false;
                Then::block(Block::Frames(1)).then(Routine::UpdateSprites)
            }
            Routine::CheckFightingMapTrainers => ow.check_fighting_map_trainers(ctx),
            Routine::AfterEngageBubble => ow.after_engage_bubble(ctx),
            Routine::DisplayEnemyTrainerTextAndStartBattle => ow.display_enemy_trainer_text_and_start_battle(ctx),
            Routine::StartTrainerBattle => ow.start_trainer_battle(ctx),
            Routine::EndTrainerBattle => ow.end_trainer_battle(ctx),
            Routine::TalkToTrainer(header) => ow.talk_to_trainer(ctx, header),
            Routine::TalkToTrainerNotYetFought => ow.talk_to_trainer_not_yet_fought(ctx),
            Routine::BlackOutFaded => ow.black_out_faded(ctx),
            Routine::Cities1AlternateTempo => {
                ctx.audio.play_music(sounds::MUSIC_CITIES1);
                ctx.audio.overwrite_channel_pointer(0, pokered_symbols::Music_Cities1_Ch1_AlternateTempo.address);
                Flow::Return
            }
            Routine::StopAllSounds => {
                ctx.audio.stop_all_sounds();
                Flow::Return
            }
            Routine::GivePokemon(species, level) => ow.give_pokemon(ctx, species, level),
            Routine::GivePokemonAskName(species, level) => ow.give_pokemon_ask_name(ctx, species, level),
            Routine::GivePokemonYesNo(species, level) => ow.give_pokemon_yes_no(species, level),
            Routine::GivePokemonAnswered(species, level) => ow.give_pokemon_answered(ctx, species, level),
            Routine::GivePokemonNamed(species, level) => ow.give_pokemon_named(ctx, species, level),

            Routine::PrintPredefTextId { id, bank } => {
                ow.rt.text_predef = Some(bank);
                Flow::Jump(Routine::DisplayTextId(id).into())
            }
            Routine::ScriptFadeOutToBlack(palette) => {
                ow.set_fade_palette(ctx, palette);
                let block = Then::block(Block::Frames(super::FADE_FRAMES));
                match palette {
                    BLACK => block.ret(),
                    _ => block.then(Routine::ScriptFadeOutToBlack(palette - 1)),
                }
            }
            Routine::ScriptFadeInFromBlack(palette) => {
                ow.set_fade_palette(ctx, palette);
                let block = Then::block(Block::Frames(super::FADE_FRAMES));
                match palette {
                    NORMAL => {
                        ow.load_gb_pal(ctx);
                        block.ret()
                    }
                    _ => block.then(Routine::ScriptFadeInFromBlack(palette + 1)),
                }
            }
            Routine::ScriptFadeOutToWhite(palette) => {
                ow.set_fade_palette(ctx, palette);
                let block = Then::block(Block::Frames(super::FADE_FRAMES));
                match palette {
                    WHITE => block.ret(),
                    _ => block.then(Routine::ScriptFadeOutToWhite(palette + 1)),
                }
            }
            Routine::ScriptFadeInFromWhite(palette) => {
                ow.set_fade_palette(ctx, palette);
                let block = Then::block(Block::Frames(super::FADE_FRAMES));
                match palette {
                    WHITE_IN_LAST => block.ret(),
                    _ => block.then(Routine::ScriptFadeInFromWhite(palette - 1)),
                }
            }
            Routine::PlayerPc => Then::block(Block::Mode(Box::new(Mode::PlayerPc(PlayerPc::direct())))).ret(),
            Routine::ActivatePc => Then::block(Block::Mode(Box::new(Mode::PcMenu(PcMenu::new())))).ret(),
            Routine::BillsPc => Then::block(Block::Mode(Box::new(Mode::BillsPc(BillsPc::direct())))).ret(),
            Routine::DisplayTownMap => Then::block(Block::Mode(Box::new(Mode::TownMap(TownMap::item())))).ret(),

            Routine::LeaveMapAnim => ow.leave_map_anim(ctx),
            Routine::LeaveMapAnimFlap => ow.leave_map_anim_flap(ctx),
            Routine::LeaveMapAnimUp => ow.leave_map_anim_up(ctx),
            Routine::LeaveMapAnimWait => ow.leave_map_anim_wait(),
            Routine::LeaveMapAnimAway => ow.leave_map_anim_away(),
            Routine::FadeOutToWhite(palette) => ow.fade_out_to_white(ctx, palette),
            Routine::SpecialWarpFaded => ow.special_warp_faded(ctx),
            Routine::LeaveMapAnimStopped => ow.leave_map_anim_stopped(ctx),
            Routine::SpinOutInPlace => ow.spin_out_in_place(),
            Routine::SpinInPlace => ow.spin_in_place(ctx),
            Routine::SpinWhileMoving => ow.spin_while_moving(),
            Routine::SpinWhileMovingUp => ow.spin_while_moving_up(ctx),
            Routine::LeaveMapAnimSpun => ow.leave_map_anim_spun(ctx),
            Routine::LeaveMapThroughHole => ow.leave_map_through_hole(),
            Routine::LeaveMapThroughHoleHidden => ow.leave_map_through_hole_hidden(),
            Routine::LeaveMapThroughHoleDone => ow.leave_map_through_hole_done(),
            Routine::EnterMapAnimSpin => ow.enter_map_anim_spin(ctx, false),
            Routine::EnterMapAnimSpinDungeon => ow.enter_map_anim_spin(ctx, true),
            Routine::EnterMapAnimDungeon => ow.enter_map_anim_dungeon(),
            Routine::EnterMapAnimSpun => ow.enter_map_anim_spun(ctx),
            Routine::EnterMapAnimMusic => ow.enter_map_anim_music(),
            Routine::EnterMapAnim => ow.enter_map_anim(ctx),
            Routine::FadeInFromWhite(palette) => ow.fade_in_from_white(ctx, palette),
            Routine::EnterMapAnimFly => ow.enter_map_anim_fly(ctx),
            Routine::EnterMapAnimLanded => ow.enter_map_anim_landed(ctx),
            Routine::EnterMapAnimDone => ow.enter_map_anim_done(ctx),
            Routine::FlyAnimStep => ow.fly_anim_step(),
            Routine::FlyAnimCoords => ow.fly_anim_coords(),

            Routine::AfterRunMapScript | Routine::AfterDisplayDialogue | Routine::EnterMapAfterBattle
            | Routine::HandleBlackOut | Routine::SpecialEnterMap | Routine::OverworldLoop | Routine::AfterCardKeyText
            | Routine::CheckWarpsNoCollision | Routine::AfterPoison | Routine::SafariWarp | Routine::EnterMapEnd
            | Routine::EnterMapRest | Routine::StartStep | Routine::WarpPadFaded => unreachable!("the runner returns to the loop there"),
        }
    }

    // ---- State ----

    /// Every map's saved script state.
    pub fn maps(&mut self) -> &mut MapStates {
        &mut self.ctx.world.scripts.maps
    }

    /// The script state no one map owns: `wRivalStarter` and the rest.
    pub fn globals(&mut self) -> &mut crate::scripts::ScriptState {
        &mut self.ctx.world.scripts
    }

    /// `GetMonName`, which copies into `wNameBuffer`.
    pub fn get_mon_name(&mut self, species: PokemonSpecies) {
        self.ctx.world.text.strings.insert(TextBuffer::NameBuffer, species.name());
    }

    /// `IsPlayerOnDungeonWarp`'s effect: the hole under the player, named by the floor it drops onto
    /// and which of that floor's holes it is.
    pub fn fall_down_hole(&mut self, destination: poke_core::map::Map, which: u8) {
        self.ow.rt.dungeon_warp = Some((destination, which));
    }

    pub fn check_event(&self, event: u16) -> bool {
        self.ctx.world.events.is_set(event)
    }

    pub fn set_event(&mut self, event: u16) {
        self.ctx.world.events.set(event);
    }

    pub fn reset_event(&mut self, event: u16) {
        self.ctx.world.events.clear(event);
    }

    /// `CheckAndSetEvent`: whether it was already set.
    pub fn check_and_set_event(&mut self, event: u16) -> bool {
        let was = self.check_event(event);
        self.set_event(event);
        was
    }

    /// `CheckAndResetEvent`: whether it was set before this cleared it.
    pub fn check_and_reset_event(&mut self, event: u16) -> bool {
        let was = self.check_event(event);
        self.reset_event(event);
        was
    }

    /// `wXCoord` and `wYCoord`.
    pub fn x(&self) -> u8 {
        self.ctx.world.location.x
    }

    pub fn y(&self) -> u8 {
        self.ctx.world.location.y
    }

    /// `wLastMap`: where a `LAST_MAP` warp leads.
    pub fn set_last_map(&mut self, map: poke_core::map::Map) {
        self.ctx.world.location.last_map = map;
    }

    pub fn set_y(&mut self, y: u8) {
        self.ctx.world.location.y = y;
    }

    /// `wObtainedBadges`.
    pub fn badges(&self) -> u8 {
        self.ctx.world.badges
    }

    /// `ld [wCurMapTextPtr], hl`: the map's texts from `table` instead of its header's, until the map
    /// is loaded again.
    pub fn set_text_pointers(&mut self, table: DmgPointer) {
        self.ow.rt.text_pointers = Some(table);
    }

    /// `UpdateSprites`.
    pub fn update_sprites(&mut self) {
        self.ow.update_sprites(self.ctx);
    }

    /// `StartSimulatingJoypadStates` over `wSimulatedJoypadStatesEnd` as `DecodeRLEList` fills it from
    /// the list at `at`; the last press is made first.
    pub fn simulate_joypad_rle(&mut self, at: DmgPointer) {
        let mut presses = super::movement::decode_rle_list(at);
        presses.pop();
        let presses: Vec<Joypad> = presses.into_iter().map(Joypad::from_bits_truncate).collect();
        self.ow.start_simulating_joypad_states(presses);
    }

    /// `wCurMapScript`.
    pub fn cur_map_script(&self) -> u8 {
        self.ctx.world.scripts.cur_map_script
    }

    /// `ld [wCurMapScript], a`: the entry of the map's own table that runs next.
    pub fn set_cur_map_script(&mut self, index: u8) {
        self.ctx.world.scripts.cur_map_script = index;
    }

    /// `StartSimulatingJoypadStates` over presses a script wrote into `wSimulatedJoypadStatesEnd`
    /// itself; the last is made first.
    pub fn simulate_joypad_presses(&mut self, presses: Vec<Joypad>) {
        self.ow.start_simulating_joypad_states(presses);
    }

    /// `res BIT_SCRIPTED_MOVEMENT_STATE`: the pad is the player's again, whatever is left of the
    /// simulated presses.
    pub fn stop_simulating_joypad_states(&mut self) {
        self.ow.scripted = false;
    }

    /// `ArePlayerCoordsInArray`: `wCoordIndex`, which the cartridge counts from 1.
    pub fn are_player_coords_in_array(&self, coords: &[(u8, u8)]) -> Option<u8> {
        let here = (self.x(), self.y());
        coords.iter().position(|&coord| coord == here).map(|index| index as u8 + 1)
    }

    /// `DecodeArrowMovementRLE`'s search of `table`, without the decoding: the RLE list the arrow
    /// tile the player stands on presses, for `simulate_joypad_rle`.
    pub fn arrow_movement(&self, table: DmgPointer) -> Option<DmgPointer> {
        crate::systems::overworld::spinners::arrow_movement(table, self.x(), self.y())
    }

    /// `BIT_SPINNING`.
    pub fn set_spinning(&mut self, spinning: bool) {
        self.ow.spinning = spinning;
    }

    /// `LoadSpinnerArrowTiles`.
    pub fn load_spinner_arrow_tiles(&mut self) {
        self.ow.load_spinner_arrow_tiles(self.ctx);
    }

    /// `BIT_FORCED_WARP`.
    pub fn set_forced_warp(&mut self, forced: bool) {
        self.ow.rt.forced_warp = forced;
    }

    /// `BIT_ALWAYS_ON_BIKE`, which the Cycling Road's gates are the only things to clear.
    pub fn set_always_on_bike(&mut self, on_bike: bool) {
        self.ctx.world.location.always_on_bike = on_bike;
    }

    /// `IsItemInBag`.
    pub fn is_item_in_bag(&self, item: ItemId) -> bool {
        self.ctx.world.bag.quantity_of(item) != 0
    }

    /// `GetQuantityOfItemInBag`.
    pub fn get_quantity_of_item_in_bag(&self, item: ItemId) -> u8 {
        self.ctx.world.bag.quantity_of(item)
    }

    /// `wWalkBikeSurfState` and its copy, then `ForceBikeOrSurf`.
    pub fn force_bike_or_surf(&mut self, state: u8) -> Then {
        self.ctx.world.location.walk_bike_surf = state;
        self.ow.walk_bike_surf_copy = state;
        crate::systems::overworld::sprites::load_player_sprite_graphics(&mut self.ctx.screen.tiles,
            &mut self.ctx.world.location, self.ow.view.tileset);
        self.play_default_music()
    }

    /// `ExecuteCurMapScriptInTable`: `index`, unless a trainer routine asked for `wCurMapScript`
    /// instead, which is where the table's routines leave the next index. `headers` is the map's
    /// `TrainerHeaders`.
    pub fn execute_cur_map_script_in_table(&mut self, index: u8, headers: DmgPointer) -> u8 {
        self.ow.rt.trainer_header = Some(headers);
        let index = if std::mem::take(&mut self.ow.rt.use_cur_map_script) { self.cur_map_script() } else { index };
        self.ctx.world.scripts.cur_map_script = index;
        index
    }

    /// The three routines every trainer map's script table starts with, by index.
    pub fn trainer_script(&self, index: u8) -> Then {
        Then::call(match index {
            0 => Routine::CheckFightingMapTrainers,
            1 => Routine::DisplayEnemyTrainerTextAndStartBattle,
            _ => Routine::EndTrainerBattle,
        })
    }

    /// `EndTrainerBattle`, for a map whose script table is not a trainer table's.
    pub fn end_trainer_battle(&self) -> Then {
        Then::call(Routine::EndTrainerBattle)
    }

    /// `EnableAutoTextBoxDrawing`.
    pub fn enable_auto_text_box_drawing(&mut self) {
        self.ow.rt.no_auto_text_box = false;
        self.ow.rt.do_not_wait = false;
    }

    /// `BIT_NO_SPRITE_UPDATES`, so the next text opens with every sprite held where it is.
    pub fn set_no_sprite_updates(&mut self) {
        self.ow.rt.no_sprite_updates = true;
    }

    /// `DisableAutoTextBoxDrawing`.
    pub fn disable_auto_text_box_drawing(&mut self) {
        self.ow.rt.no_auto_text_box = true;
        self.ow.rt.do_not_wait = false;
    }

    /// `wDoNotWaitForButtonPressAfterDisplayingText`.
    pub fn set_do_not_wait_for_button_press(&mut self, value: bool) {
        self.ow.rt.do_not_wait = value;
    }

    /// `wJoyIgnore`.
    pub fn joy_ignore(&mut self, buttons: Joypad) {
        self.ctx.pad.ignore = buttons;
    }

    /// `xor a; ldh [hJoyHeld], a`.
    pub fn clear_joy_held(&mut self) {
        self.ctx.pad.held = Joypad::empty();
    }

    /// `wPlayerMovingDirection`, as `PLAYER_DIR_*` bits.
    pub fn set_player_moving_direction(&mut self, direction: u8) {
        self.ow.standing.moving_direction = direction;
    }

    /// `wSpritePlayerStateData1FacingDirection`.
    pub fn set_player_facing(&mut self, facing: u8) {
        self.ow.sprites[0].facing = facing;
    }

    pub fn sprite(&self, slot: u8) -> &SpriteState {
        &self.ow.sprites[slot as usize]
    }

    // ---- Sound ----

    pub fn play_sound(&mut self, sound: SoundId) {
        self.ctx.audio.play_sound(sound);
    }

    /// `PlayMusic`.
    pub fn play_music(&mut self, sound: Sound) {
        self.ctx.audio.play_music(sound);
    }

    /// `PlayDefaultMusic`: once the sound effects have finished, the map's song, or the bike's or the water's.
    pub fn play_default_music(&mut self) -> Then {
        Then::call(Routine::PlayDefaultMusic)
    }

    /// `Music_RivalAlternateStart`: the rival's theme from a different first measure.
    pub fn music_rival_alternate_start(&mut self) {
        self.ctx.audio.play_music(sounds::MUSIC_MEET_RIVAL);
        let starts = [pokered_symbols::Music_MeetRival_Ch1_AlternateStart, pokered_symbols::Music_MeetRival_Ch2_AlternateStart,
            pokered_symbols::Music_MeetRival_Ch3_AlternateStart];
        for (channel, start) in starts.into_iter().enumerate() {
            self.ctx.audio.overwrite_channel_pointer(channel, start.address);
        }
    }

    /// `Music_RivalAlternateTempo`: the rival's theme a little slower.
    pub fn music_rival_alternate_tempo(&mut self) {
        self.ctx.audio.play_music(sounds::MUSIC_MEET_RIVAL);
        self.ctx.audio.overwrite_channel_pointer(0, pokered_symbols::Music_MeetRival_Ch1_AlternateTempo.address);
    }

    /// `Music_RivalAlternateStartAndTempo`.
    pub fn music_rival_alternate_start_and_tempo(&mut self) {
        self.music_rival_alternate_start();
        self.ctx.audio.overwrite_channel_pointer(0, pokered_symbols::Music_MeetRival_Ch1_AlternateStartAndTempo.address);
    }

    /// `Music_Cities1AlternateTempo`, the Hall of Fame's: the music fades to silence over
    /// `DelayFrames 100`, then Cities1 starts with its first channel at the slower tempo.
    pub fn music_cities1_alternate_tempo(&mut self) -> Then {
        const FADE_FRAMES: u8 = 10;
        const WAIT_FRAMES: u8 = 100;
        self.ctx.audio.fade_out_to_silence(FADE_FRAMES);
        Then::block(Block::Chain(Box::new(Block::Frames(WAIT_FRAMES)), Routine::Cities1AlternateTempo))
    }

    /// `StopMusic`: the music faded out a step every `frames`, waited on, and `StopAllSounds`.
    pub fn stop_music(&mut self, frames: u8) -> Then {
        self.ctx.audio.stop_music(frames);
        Then::block(Block::Chain(Box::new(Block::MusicStopped), Routine::StopAllSounds))
    }

    /// `WaitForSoundToFinish`.
    pub fn wait_for_sound_to_finish(&mut self) -> Then {
        Then::block(Block::Sound)
    }

    /// `PlaySound` with `wNewSoundID` written first, as a caller starting music does.
    pub fn play_new_sound(&mut self, sound: SoundId) {
        self.ctx.audio.play_new_sound(sound);
    }

    /// `wMapMusicSoundID`: the current map's song.
    pub fn map_music_sound_id(&self) -> SoundId {
        SoundId(poke_core::map_objects::map_song(self.ctx.world.location.map).0)
    }

    /// `wChannelSoundIDs + channel`: what the channel is playing, 0 for nothing.
    pub fn channel_sound_id(&self, channel: usize) -> u8 {
        self.ctx.audio.channel_sound_id(channel)
    }

    /// A busy wait on `wChannelSoundIDs`: until `channel` is no longer playing `sound`.
    pub fn wait_while_channel_plays(&mut self, channel: usize, sound: SoundId) -> Then {
        Then::block(Block::ChannelPlaying { channel, id: sound.0 })
    }

    // ---- Frames ----

    /// `DelayFrames`.
    pub fn delay_frames(&mut self, frames: u8) -> Then {
        Then::block(Block::Frames(frames))
    }

    /// `Delay3`.
    pub fn delay3(&mut self) -> Then {
        self.delay_frames(3)
    }

    // ---- Text ----

    /// `DisplayTextID` with `hTextID` set: a sprite's text for an id up to `wNumSprites`, else the
    /// map's text of that id, in its own box, waited on and closed.
    pub fn display_text_id(&mut self, text_id: u8) -> Then {
        Then::call(Routine::DisplayTextId(text_id))
    }

    /// `PrintText`.
    pub fn print_text(&mut self, commands: Vec<TextCommand>) -> Then {
        Then::block(Block::PrintText(commands))
    }

    /// The text a `text_asm` returns in `hl`, which the text engine prints on in the box already up
    /// rather than drawing a new one.
    pub fn print_text_from_asm(&mut self, commands: Vec<TextCommand>) -> Then {
        Then::block(Block::Mode(Box::new(Mode::TextBox(TextBox::without_box(commands)))))
    }

    /// `YesNoChoice`; [`Script::chose_yes`] reads the answer.
    /// `WaitForTextScrollButtonPress`: the text already on the screen is held until a button is
    /// pressed, with no `▼` of its own.
    pub fn wait_for_text_scroll_button_press(&mut self) -> Then {
        Then::block(Block::TextScrollButton)
    }

    pub fn yes_no_choice(&mut self) -> Then {
        Then::block(Block::Mode(Box::new(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, (14, 7), false)))))
    }

    /// `wCurrentMenuItem` after a yes/no: the first option.
    pub fn chose_yes(&self) -> bool {
        self.ow.rt.outcome == Some(Outcome::Chosen(0))
    }

    /// `TalkToTrainer`.
    pub fn talk_to_trainer(&mut self, header: DmgPointer) -> Then {
        Then::call(Routine::TalkToTrainer(header))
    }

    // ---- Items and Pokémon ----

    /// `GiveItem`: into the bag with its name in `wStringBuffer`, or `false` with no room.
    pub fn give_item(&mut self, item: ItemId, quantity: u8) -> bool {
        self.ow.give_item(self.ctx, item, quantity)
    }

    /// `GivePokemon`: into the party with a nickname if the player wants one; see
    /// [`Script::added_to_party`].
    pub fn give_pokemon(&mut self, species: PokemonSpecies, level: u8) -> Then {
        Then::call(Routine::GivePokemon(species, level))
    }

    /// `wAddedToParty`, and so the carry `GivePokemon` returns for a party with room.
    pub fn added_to_party(&self) -> bool {
        self.ow.rt.added_to_party
    }

    /// `GivePokemon`'s carry: the mon went into the party or a box.
    pub fn gave_pokemon(&self) -> bool {
        self.ow.rt.gave_pokemon
    }

    /// `bit BIT_CUR_MAP_LOADED_n` then `res`: whether the map has been loaded since the script last
    /// asked, `n` being 1 or 2.
    pub fn check_and_reset_cur_map_loaded(&mut self, n: usize) -> bool {
        std::mem::take(&mut self.ow.rt.cur_map_loaded[n - 1])
    }

    /// `set BIT_CUR_MAP_LOADED_n`: what a script sets to have the next pass run its own load code.
    pub fn set_cur_map_loaded(&mut self, n: usize) {
        self.ow.rt.cur_map_loaded[n - 1] = true;
    }

    /// A background tile written at screen cell `(column, row)` over what the blocks draw there,
    /// staying with the map as the view moves until the map is loaded again.
    pub fn overwrite_bg_tile(&mut self, column: u8, row: u8, tile: u8) {
        let (x, y) = self.ow.view.camera();
        let at = (y / 8 + row as i32, x / 8 + column as i32);
        let overrides = &mut self.ow.view.tile_overrides;
        match overrides.binary_search_by_key(&at, |&(row, column, _)| (row, column)) {
            Ok(index) => overrides[index].2 = tile,
            Err(index) => overrides.insert(index, (at.0, at.1, tile)),
        }
    }

    /// `LoadCurrentMapView`'s tile at screen cell `(column, row)`: what `wTileMap` holds there.
    pub fn map_view_tile(&self, column: usize, row: usize) -> u8 {
        self.ow.view.tile(column, row)
    }

    /// `ReplaceTileBlock`: block `(x, y)` of the map, counted in blocks, becomes `block`. The screen
    /// draws from the blocks, so what `RedrawMapView` would show is shown.
    pub fn replace_tile_block(&mut self, x: u8, y: u8, block: u8) {
        let stride = self.ow.view.stride() as usize;
        let at = stride * 3 + 3 + stride * y as usize + x as usize;
        self.ow.view.blocks[at] = block;
    }

    // ---- Objects and sprites ----

    /// `HideObject`, by `TOGGLE_*`.
    pub fn hide_object(&mut self, toggle: u16) {
        self.ow.toggle_object(self.ctx, toggle, true);
    }

    /// `ShowObject`.
    pub fn show_object(&mut self, toggle: u16) {
        self.ow.toggle_object(self.ctx, toggle, false);
    }

    /// `SetSpriteFacingDirectionAndDelay`.
    pub fn set_sprite_facing_direction_and_delay(&mut self, slot: u8, facing: u8) -> Then {
        self.ow.sprites[slot as usize].facing = facing;
        self.delay_frames(6)
    }

    /// `MoveSprite`: `directions` (`NPC_MOVEMENT_*`, ending in `$ff`) for the sprite to walk, a square
    /// a time, while the player waits.
    pub fn move_sprite(&mut self, slot: u8, directions: &[u8]) {
        self.ow.set_sprite_movement_bytes_to_ff(slot);
        self.ow.move_sprite(self.ctx, slot, directions);
    }

    /// `SetSpriteMovementBytesToFF`, which [`Script::move_sprite`] does for itself: a script calls it
    /// on its own only where it may end up not moving the sprite after all.
    pub fn set_sprite_movement_bytes_to_ff(&mut self, slot: u8) {
        self.ow.set_sprite_movement_bytes_to_ff(slot);
    }

    /// `BIT_SCRIPTED_NPC_MOVEMENT`: a `MoveSprite` path is still being walked.
    pub fn npc_moving(&self) -> bool {
        self.ow.rt.paths.scripted_npc_movement
    }

    /// `bit BIT_PUSHED_BOULDER, [hl]` then `res`: a boulder finished a square since the map's script
    /// last asked. Seafoam's floors read it to see whether a boulder has gone down a hole, and a
    /// switch under one is read the same way.
    pub fn check_and_reset_pushed_boulder(&mut self) -> bool {
        std::mem::take(&mut self.ow.rt.pushed_boulder)
    }

    /// `CheckBoulderCoords`: where the last pushed boulder stands, against a script's list of
    /// squares. The index of the match is what a floor with two holes tells them apart by.
    pub fn check_boulder_coords(&self, coords: &[(u8, u8)]) -> Option<usize> {
        let boulder = &self.ow.sprites[self.ow.rt.boulder_sprite as usize];
        let (x, y) = (boulder.map_x.wrapping_sub(4), boulder.map_y.wrapping_sub(4));
        coords.iter().position(|&square| square == (x, y))
    }

    /// `EmotionBubble` over a sprite, `EXCLAMATION_BUBBLE` being 0.
    pub fn emotion_bubble(&mut self, slot: u8, bubble: u8) -> Then {
        self.ow.emotion_bubble(self.ctx, slot, bubble);
        Then::block(Block::Chain(Box::new(Block::Frames(super::movement::EMOTION_BUBBLE_FRAMES)), Routine::EmotionBubbleEnd))
    }

    /// `wSimulatedJoypadStatesIndex`.
    pub fn simulated_joypad_states_index(&self) -> u8 {
        self.ow.simulated_index
    }

    /// `CalcPositionOfPlayerRelativeToNPC` and `FindPathToPlayer` for the sprite in `slot`, with
    /// the Y distance changed by `adjust_y` in between as a script may. The directions, `$ff`-ended.
    pub fn find_path_to_player(&self, slot: u8, perspective: bool, adjust_y: i8) -> Vec<u8> {
        super::movement::find_path_to_player(&self.ow.sprites, slot, perspective, adjust_y)
    }

    /// `wNPCMovementScriptPointerTableNum`, `wNPCMovementScriptFunctionNum` and the sprite
    /// `wSpriteIndex` names, for `RunNPCMovementScript` to take over from the next pass.
    pub fn start_npc_movement_script(&mut self, table: u8, slot: u8) {
        self.ow.rt.sprite_index = slot;
        self.ow.rt.npc_movement_script_function = 0;
        self.ow.rt.npc_movement_script_table = table;
    }

    /// `wNPCMovementScriptPointerTableNum`.
    pub fn npc_movement_script_running(&self) -> bool {
        self.ow.rt.npc_movement_script_table != 0
    }

    /// `wSpritePlayerStateData1FacingDirection`.
    pub fn player_facing(&self) -> u8 {
        self.ow.sprites[0].facing
    }

    /// `wWalkBikeSurfState`.
    pub fn walk_bike_surf(&self) -> u8 {
        self.ctx.world.location.walk_bike_surf
    }

    /// `wSpriteIndex`, which a text leaves for the code the map's script runs next.
    pub fn sprite_index(&self) -> u8 {
        self.ow.rt.sprite_index
    }

    pub fn set_sprite_index(&mut self, slot: u8) {
        self.ow.rt.sprite_index = slot;
    }

    /// `wNPCNumScriptedSteps`: how many squares of a `MoveSprite` path are left, which a script
    /// watching a sprite walk away reads to turn the player after it.
    pub fn npc_steps_left(&self) -> u8 {
        self.ow.rt.paths.num_scripted_steps
    }

    /// `hSpriteFacingDirection` written straight into the sprite, which a script does where it wants
    /// no `Delay3` after it.
    pub fn set_sprite_facing(&mut self, slot: u8, facing: u8) {
        self.ow.sprites[slot as usize].facing = facing;
    }

    /// `hSpriteImageIndex` with `SetSpriteImageIndexAfterSettingFacingDirection`: the walking frame
    /// a sprite is drawn in, which a script picks where it puts one somewhere by hand.
    pub fn set_sprite_image_index(&mut self, slot: u8, index: u8) {
        self.ow.sprites[slot as usize].image_index = index;
    }

    /// `GetSpritePosition1` and `GetSpritePosition2`: where a sprite stands, to put back later.
    pub fn sprite_position(&self, slot: u8) -> SpritePosition {
        let sprite = &self.ow.sprites[slot as usize];
        SpritePosition { screen_y: sprite.y_pixels, screen_x: sprite.x_pixels, map_y: sprite.map_y, map_x: sprite.map_x }
    }

    /// `SetSpritePosition1` and `SetSpritePosition2`.
    pub fn set_sprite_position(&mut self, slot: u8, at: SpritePosition) {
        let sprite = &mut self.ow.sprites[slot as usize];
        sprite.y_pixels = at.screen_y;
        sprite.x_pixels = at.screen_x;
        sprite.map_y = at.map_y;
        sprite.map_x = at.map_x;
    }

    // ---- Battles a script starts itself ----

    /// `wCurOpponent` and `wTrainerNo` with `SaveEndBattleTextPointers`' win text, and the
    /// `wStatusFlags3` bits a scripted battle sets. The overworld's next pass starts the battle.
    pub fn start_trainer_battle(&mut self, opponent: u8, trainer_no: u8, win_text: DmgPointer) {
        self.ow.rt.cur_opponent = opponent;
        self.ow.rt.enemy_mon_or_trainer_class = opponent;
        self.ow.rt.trainer_no = trainer_no;
        self.save_end_battle_text(win_text);
    }

    /// `SaveEndBattleTextPointers` and the two `wStatusFlags3` bits beside it.
    pub fn save_end_battle_text(&mut self, win_text: DmgPointer) {
        self.ow.rt.end_battle_text = Some(win_text);
        self.ow.rt.talked_to_trainer = true;
        self.ow.rt.print_end_battle_text = true;
    }

    /// `wCurOpponent` as a species and `wCurEnemyLevel`: a wild battle with no grass in it.
    pub fn start_wild_battle(&mut self, species: PokemonSpecies, level: u8) {
        self.ow.rt.cur_opponent = species as u8;
        self.ow.rt.cur_enemy_level = level;
    }

    /// `wBattleType` at `BATTLE_TYPE_OLD_MAN`: the next wild battle is the catching lesson, which
    /// Oak fights in the player's place.
    pub fn set_old_man_battle(&mut self, old_man: bool) {
        self.ow.rt.old_man_battle = old_man;
    }

    /// `EngageMapTrainer` and `InitBattleEnemyParameters` for the sprite in `slot`, which a gym
    /// leader's own text does rather than being walked up to, then `wGymLeaderNo`. Set after
    /// `PlayTrainerMusic` has read it, so the leader's encounter music plays.
    pub fn engage_map_trainer(&mut self, slot: u8, gym_leader_no: u8) {
        self.ow.rt.sprite_index = slot;
        self.ow.engage_map_trainer(self.ctx);
        let class = self.ow.rt.engaged_class;
        self.ow.rt.cur_opponent = class;
        self.ow.rt.enemy_mon_or_trainer_class = class;
        self.ow.rt.trainer_no = self.ow.rt.engaged_set;
        if gym_leader_no != 0 {
            self.ow.rt.gym_leader_no = gym_leader_no;
        }
    }

    /// `wGymLeaderNo` set ahead of `EngageMapTrainer`, as `CinnabarGymBlaineText` does, which
    /// silences the leader's encounter music.
    pub fn set_gym_leader_no(&mut self, gym_leader_no: u8) {
        self.ow.rt.gym_leader_no = gym_leader_no;
    }

    /// `wIsInBattle` at `$ff`: the battle the script sent the player into was lost.
    pub fn lost_battle(&self) -> bool {
        self.ow.rt.lost_battle
    }

    /// `BIT_NO_BATTLES`.
    pub fn set_no_battles(&mut self, no_battles: bool) {
        self.ow.rt.no_battles = no_battles;
    }

    /// `BIT_NO_MAP_MUSIC`.
    pub fn set_no_map_music(&mut self, no_map_music: bool) {
        self.ow.rt.no_map_music = no_map_music;
    }

    // ---- The bag, the party and the badges ----

    /// `RemoveItemFromInventory` for one of `item`, wherever in the bag it is.
    pub fn remove_item(&mut self, item: ItemId, quantity: u8) {
        let bag = &mut self.ctx.world.bag;
        if let Some(slot) = bag.items.iter().position(|entry| entry.id == item) {
            bag.remove(slot, quantity);
        }
    }

    /// `AddPartyMon` from a script that has printed its own words: `AskName`'s offer of a nickname,
    /// then the mon into the party. A script reaching this has already made room for it.
    pub fn add_party_mon(&mut self, species: PokemonSpecies, level: u8) -> Then {
        Then::call(Routine::GivePokemonAskName(species, level))
    }

    /// `CountSetBits` over `wPokedexOwned`.
    pub fn pokedex_owned(&self) -> u8 {
        crate::systems::pokedex::count_set_bits(&self.ctx.world.pokedex.owned)
    }

    /// `GBFadeOutToBlack` where the map stays put, four palettes of eight frames.
    pub fn gb_fade_out_to_black(&mut self) -> Then {
        Then::call(Routine::ScriptFadeOutToBlack(NORMAL))
    }

    /// `GBFadeInFromBlack`, the same the other way.
    pub fn gb_fade_in_from_black(&mut self) -> Then {
        Then::call(Routine::ScriptFadeInFromBlack(BLACK))
    }

    /// `GBFadeOutToWhite`, three palettes of eight frames.
    pub fn gb_fade_out_to_white(&mut self) -> Then {
        Then::call(Routine::ScriptFadeOutToWhite(WHITE_OUT_FIRST))
    }

    /// `GBFadeInFromWhite`, the same the other way; the loop's own `LoadGBPal` finishes it.
    pub fn gb_fade_in_from_white(&mut self) -> Then {
        Then::call(Routine::ScriptFadeInFromWhite(WHITE_IN_FIRST))
    }

    /// `wBattleResult` as the battle last popped it: `0` won, `1` lost, `2` drew or ran.
    pub fn battle_result(&self) -> u8 {
        self.ow.rt.battle_result
    }

    /// `wTrainerHeaderFlagBit`: the sprite slot of whichever trainer `CheckFightingMapTrainers`
    /// engaged, and zero when none of them did.
    pub fn trainer_header_flag_bit(&self) -> u8 {
        self.ow.rt.trainer_header_flag_bit
    }

    /// `BIT_TALKED_TO_TRAINER`, which a trainer's own text sets and the battle clears.
    pub fn talked_to_trainer(&self) -> bool {
        self.ow.rt.talked_to_trainer
    }

    /// `BIT_SEEN_BY_TRAINER`.
    pub fn set_seen_by_trainer(&mut self, seen: bool) {
        self.ow.rt.seen_by_trainer = seen;
    }

    /// The `wToggleableObjectList` search a script does to take the sprite it just fought off the
    /// map, which only works because every such sprite is a toggleable object.
    pub fn hide_object_for_sprite(&mut self, slot: u8) {
        let Some(&(_, toggle)) = self.ow.toggleable.iter().find(|&&(sprite, _)| sprite == slot) else {
            return;
        };
        self.ow.toggle_object(self.ctx, toggle as u16, true);
    }

    /// `BIT_WARP_FROM_CUR_SCRIPT`: the loop takes this warp on its next pass rather than the one the
    /// player is standing on, and `wLastMap` is the script's to set.
    pub fn warp_from_cur_script(&mut self, destination: poke_core::map::Map, warp: u8) {
        self.ow.rt.script_warp = Some((destination as u8, warp));
    }

    /// `hRandomAdd`, which a text with more than one thing to say picks between by.
    pub fn random(&mut self) -> u8 {
        crate::rng::Rng::random(self.ctx.rng)
    }

    /// The `xor a / ld [hli], a / ld [hl], a` a Silph floor's gate callback does once it has
    /// matched the square, so no other floor opens a gate of its own at the same coordinate.
    pub fn clear_card_key_door(&mut self) {
        self.ow.rt.events.card_key_door = (0, 0);
    }

    /// `predef HallOfFamePC` and the tail its script ends with: the ceremony, the credits, the save
    /// and `jp Init`. It takes the overworld's place and replaces itself with power-on, so nothing
    /// of the game is left running under the title screen.
    pub fn hall_of_fame_pc(&mut self) -> Then {
        let movie = crate::modes::movie::Movie::hall_of_fame();
        Then::block(Block::Replace(Box::new(Mode::Movie(movie))))
    }

    /// `wOptions`' `BIT_BATTLE_ANIMATION`, which the champion's battle turns on whatever the player
    /// set and leaves that way.
    pub fn set_battle_animation(&mut self, on: bool) {
        self.ctx.world.options.battle_animation = on;
    }

    /// `wObtainedBadges`, by badge bit.
    pub fn set_badge(&mut self, bit: u8) {
        self.ctx.world.badges |= 1 << bit;
    }

    /// `BIT_GAVE_SAFFRON_GUARDS_DRINK`.
    pub fn gave_saffron_guards_drink(&self) -> bool {
        self.ctx.world.scripts.gave_saffron_guards_drink
    }

    /// `set BIT_GAVE_SAFFRON_GUARDS_DRINK`.
    pub fn set_gave_saffron_guards_drink(&mut self) {
        self.ctx.world.scripts.gave_saffron_guards_drink = true;
    }

    /// `BIT_GOT_OLD_ROD`.
    pub fn got_old_rod(&self) -> bool {
        self.ctx.world.scripts.got_old_rod
    }

    /// `BIT_GOT_GOOD_ROD`.
    pub fn got_good_rod(&self) -> bool {
        self.ctx.world.scripts.got_good_rod
    }

    /// `BIT_GOT_SUPER_ROD`.
    pub fn got_super_rod(&self) -> bool {
        self.ctx.world.scripts.got_super_rod
    }

    /// `wDestinationWarpID`, left from the warp the player came in by.
    pub fn destination_warp_id(&self) -> u8 {
        self.ow.standing.destination_warp
    }

    /// `dec [wNumberOfWarps]`: the map's last warp leads nowhere until the map is loaded again.
    pub fn decrement_number_of_warps(&mut self) {
        self.ow.warps.pop();
    }

    /// `wWarpedFromWhichWarp` and `wWarpedFromWhichMap`: the warp of the map the player came in
    /// through.
    pub fn warped_from(&self) -> (u8, u8) {
        self.ow.warped_from
    }

    /// One `wWarpEntries` entry's destination, which only an elevator rewrites.
    pub fn set_warp_destination(&mut self, index: usize, warp: u8, map: u8) {
        let door = &mut self.ow.warps[index];
        door.destination_warp = warp;
        door.destination_map = map;
    }

    /// `wUpdateSpritesEnabled`: off, OAM stays as a script writes it.
    pub fn set_update_sprites_enabled(&mut self, enabled: bool) {
        self.ow.rt.sprites_frozen = !enabled;
    }

    /// `wSpritePlayerStateData1ImageIndex`.
    pub fn set_player_image_index(&mut self, index: u8) {
        self.ow.sprites[0].image_index = index;
    }

    /// `LoadPlayerSpriteGraphics`.
    pub fn load_player_sprite_graphics(&mut self) {
        crate::systems::overworld::sprites::load_player_sprite_graphics(&mut self.ctx.screen.tiles,
            &mut self.ctx.world.location, self.ow.view.tileset);
    }

    /// The screen as the hardware holds it, for a script that draws on it by hand.
    pub fn screen(&mut self) -> &mut crate::gfx::Screen {
        &mut self.ctx.screen
    }

    /// `set BIT_GOT_OLD_ROD`.
    pub fn set_got_old_rod(&mut self) {
        self.ctx.world.scripts.got_old_rod = true;
    }

    /// `set BIT_GOT_GOOD_ROD`.
    pub fn set_got_good_rod(&mut self) {
        self.ctx.world.scripts.got_good_rod = true;
    }

    /// `set BIT_GOT_SUPER_ROD`.
    pub fn set_got_super_rod(&mut self) {
        self.ctx.world.scripts.got_super_rod = true;
    }

    // ---- The Game Corner's money and coins, which only its own people hand out ----

    /// `GameCornerDrawCoinBox`: the money and the coins, beside a question about buying coins.
    pub fn game_corner_draw_coin_box(&mut self) {
        let (money, coins) = (self.ctx.world.money, self.ctx.world.coins);
        self.ctx.screen.ui.text_box_border(11, 0, 7, 5);
        self.update_sprites();
        let ui = &mut self.ctx.screen.ui;
        ui.fill(12, 1, 7, 4, UiSurface::BLANK);
        ui.place(12, 2, &poke_core::charmap::encode("MONEY").expect("charmap"));
        print_bcd(ui, 3 * SCREEN_TILES_X + 12, &money, BcdFormat { money_sign: true, ..Default::default() });
        ui.place(12, 4, &poke_core::charmap::encode("COIN").expect("charmap"));
        print_bcd(ui, 5 * SCREEN_TILES_X + 15, &coins, BcdFormat::default());
    }

    /// `HasEnoughMoney`.
    pub fn has_enough_money(&self, price: [u8; 3]) -> bool {
        crate::systems::money::has_enough(&self.ctx.world.money, &price)
    }

    /// `SubBCDPredef` against `wPlayerMoney`.
    pub fn subtract_money(&mut self, price: [u8; 3]) {
        sub_bcd(&mut self.ctx.world.money, &price);
    }

    /// `DisplayTextBoxID` with `MONEY_BOX`: the purse, over whatever is on screen.
    pub fn money_box(&mut self) {
        let money = self.ctx.world.money;
        crate::gfx::text_boxes::money_box(&mut self.ctx.screen.ui, &money);
    }

    /// `wNumSafariBalls`, which only the Safari Zone gate sets.
    pub fn set_safari_balls(&mut self, balls: u8) {
        self.ctx.world.safari_balls = balls;
    }

    /// `wSafariSteps`, ditto.
    pub fn set_safari_steps(&mut self, steps: u16) {
        self.ctx.world.safari_steps = steps;
    }

    /// `wPlayerCoins`, two BCD bytes.
    pub fn coins(&self) -> [u8; 2] {
        self.ctx.world.coins
    }

    /// `Has9990Coins`: the case is full, so nobody may hand over any more.
    pub fn has_9990_coins(&self) -> bool {
        crate::systems::money::has_enough(&self.ctx.world.coins, &[0x99, 0x90])
    }

    /// `AddBCDPredef` onto `wPlayerCoins`, which saturates at 9999.
    pub fn add_coins(&mut self, coins: u8) {
        add_bcd(&mut self.ctx.world.coins, &[0, coins]);
    }

    /// `BIT_NO_NPC_FACE_PLAYER`: whoever is spoken to next stays facing the way they are.
    pub fn set_no_npc_face_player(&mut self, no_face: bool) {
        self.ow.no_face_player = no_face;
    }

    /// `PlayCry` without its wait, which the caller does with `wait_for_sound_to_finish`.
    pub fn play_cry(&mut self, species: PokemonSpecies) {
        self.ctx.audio.play_cry(species as u8);
    }

    // ---- The party menu and the naming screen, which only the name rater opens from a script ----

    /// `SaveScreenTilesToBuffer2`.
    pub fn save_screen_tiles(&mut self) {
        self.ow.rt.saved_screen2 = Some(self.ctx.screen.ui.clone());
    }

    /// `RestoreScreenTilesAndReloadTilePatterns`.
    pub fn restore_screen_tiles(&mut self) {
        if let Some(screen) = self.ow.rt.saved_screen2.take() {
            self.ctx.screen.ui = screen;
        }
    }

    /// `CeladonMartRoofScript_GiveDrinkToGirl`'s menu: the drinks the bag holds, two rows apart in a
    /// box of their own. It watches A and B, so backing out leaves no choice.
    pub fn drink_menu(&mut self, drinks: &[ItemId]) -> Then {
        self.ctx.screen.ui.text_box_border(0, 0, 12, drinks.len() * 2 - 1);
        self.update_sprites();
        for (row, &drink) in drinks.iter().enumerate() {
            self.ctx.screen.ui.place(2, 2 + 2 * row, &poke_core::item::name(drink));
        }
        let menu = CursorMenu::new(0, drinks.len() as u8 - 1, (1, 2));
        Then::block(Block::Mode(Box::new(Mode::CursorMenu(menu))))
    }

    /// `BikeShopClerkText`'s menu: `BICYCLE` and `CANCEL` with the price, in a box
    /// of their own at the top of the screen. The cursor waits in [`Script::handle_menu_input`].
    pub fn bike_shop_menu(&mut self) {
        self.ctx.screen.ui.text_box_border(0, 0, 15, 4);
        self.update_sprites();
        let ui = &mut self.ctx.screen.ui;
        ui.place(2, 2, &poke_core::charmap::encode("BICYCLE").expect("charmap"));
        ui.place(2, 4, &poke_core::charmap::encode("CANCEL").expect("charmap"));
        ui.place(8, 3, &poke_core::charmap::encode("¥1000000").expect("charmap"));
    }

    /// `HandleMenuInput` watching A and B over a menu the caller has drawn, with `wCurrentMenuItem`
    /// and `wLastMenuItem` zeroed first.
    pub fn handle_menu_input(&mut self, max: u8, top: (u8, u8)) -> Then {
        self.ctx.menu.last_item = 0;
        Then::block(Block::Mode(Box::new(Mode::CursorMenu(CursorMenu::new(0, max, top)))))
    }

    /// `BIT_NO_TEXT_DELAY`.
    pub fn set_no_text_delay(&mut self, on: bool) {
        self.ctx.world.no_text_delay = on;
    }

    /// `DisplayListMenuID` with `SPECIALLISTMENU` over `items`, opened at the caller's
    /// `wCurrentMenuItem` and `wListScrollOffset`; [`Script::chosen_row`] reads the entry.
    pub fn display_special_list_menu(&mut self, items: Vec<ItemId>, current: u8, scroll: u8) -> Then {
        let list = crate::modes::list_menu::ListMenu::special_at(items, current, scroll);
        Then::block(Block::Mode(Box::new(Mode::ListMenu(list))))
    }

    /// `wCurrentMenuItem` and `wListScrollOffset` as the last list left them.
    pub fn list_menu_position(&self) -> (u8, u8) {
        (self.ctx.menu.chosen_item, self.ctx.menu.list_scroll)
    }

    /// `wListScrollOffset`.
    pub fn set_list_scroll_offset(&mut self, scroll: u8) {
        self.ctx.menu.list_scroll = scroll;
    }

    /// `wCurrentMenuItem` as a cursor menu left it, `None` where the player backed out of it.
    pub fn chosen_row(&self) -> Option<u8> {
        match self.ow.rt.outcome {
            Some(Outcome::Chosen(row)) => Some(row),
            _ => None,
        }
    }

    /// `DisplayPartyMenu`.
    pub fn display_party_menu(&mut self) -> Then {
        let menu = crate::modes::party_menu::PartyMenu::new(crate::modes::party_menu::PartyMenuType::Normal);
        Then::block(Block::Mode(Box::new(Mode::PartyMenu(menu))))
    }

    /// `wWhichPokemon` as the party menu left it, `None` where the player backed out of it.
    pub fn chosen_party_mon(&self) -> Option<u8> {
        match self.ow.rt.outcome {
            Some(Outcome::Chosen(slot)) if (slot as usize) < self.ctx.world.party.len() => Some(slot),
            _ => None,
        }
    }

    /// `GetPartyMonName2` into `wStringBuffer`.
    pub fn get_party_mon_name2(&mut self, slot: u8) {
        let nick = self.ctx.world.party[slot as usize].nick.clone();
        self.ctx.world.text.strings.insert(TextBuffer::StringBuffer, nick);
    }

    /// `NameRatersHouseCheckMonOTScript`: whether the mon's OT name and OT id are both the player's,
    /// which is what the rater refuses to rename anything else on.
    pub fn party_mon_is_players(&self, slot: u8) -> bool {
        let mon = &self.ctx.world.party[slot as usize];
        mon.ot == self.ctx.world.player_name && mon.mon.mon.ot_id == self.ctx.world.player_id
    }

    /// `DisplayNameRaterScreen`'s naming screen; the name it leaves is `rename_party_mon`'s.
    pub fn name_rater_screen(&mut self, slot: u8) -> Then {
        let species = self.ctx.world.party[slot as usize].mon.mon.species;
        self.ctx.world.text.strings.insert(TextBuffer::StringBuffer, Vec::new());
        let screen = crate::modes::naming_screen::NamingScreen::new(
            crate::modes::naming_screen::NamingScreenType::Mon, Some(species));
        Then::block(Block::Mode(Box::new(Mode::NamingScreen(screen))))
    }

    /// `DisplayNameRaterScreen`'s carry: an empty name is a cancel and the mon keeps the one it has.
    pub fn rename_party_mon(&mut self, slot: u8) -> bool {
        let typed = self.ctx.world.text.string(TextBuffer::StringBuffer);
        if typed.is_empty() {
            return false;
        }
        self.ctx.world.party[slot as usize].nick = typed;
        true
    }

    /// `wNameBuffer`, which a `text_ram` in the text about to be printed reads.
    pub fn set_name_buffer(&mut self, name: &str) {
        let name = poke_core::charmap::encode(name).expect("a name buffer's string");
        self.ctx.world.text.strings.insert(TextBuffer::NameBuffer, name);
    }

    /// `LoadGymLeaderAndCityName`, which a gym runs on its first pass so its statue can read them.
    pub fn load_gym_leader_and_city_name(&mut self, city: &str, leader: &str) {
        let strings = &mut self.ctx.world.text.strings;
        strings.insert(TextBuffer::GymCityName, poke_core::charmap::encode(city).expect("a gym's city name"));
        strings.insert(TextBuffer::GymLeaderName, poke_core::charmap::encode(leader).expect("a gym leader's name"));
    }
}

/// Where a sprite stands, as the four bytes `GetSpritePosition1` saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpritePosition {
    pub screen_y: u8,
    pub screen_x: u8,
    pub map_y: u8,
    pub map_x: u8,
}

// ---- The runner ----

impl Overworld {
    /// `run_script` over a fresh stack, the last entry running first.
    pub(super) fn run_script_from(&mut self, ctx: &mut Ctx, stack: Vec<Code>) -> Transition {
        self.rt.stack = stack;
        self.run_script(ctx)
    }

    /// Runs the stack until something waits a frame or the loop takes over again.
    pub(super) fn run_script(&mut self, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Script;
        self.rt.waiting = Waiting::Nothing;
        for _ in 0..100_000 {
            let code = self.rt.stack.pop().expect("a script always returns to the overworld's loop");
            let flow = match code {
                Code::Runtime(Routine::AfterRunMapScript) => return self.after_run_map_script(ctx),
                Code::Runtime(Routine::AfterDisplayDialogue) => return self.check_for_opponent(ctx),
                Code::Runtime(Routine::EnterMapAfterBattle) => return self.enter_map(ctx),
                Code::Runtime(Routine::HandleBlackOut) => return self.handle_black_out(ctx),
                Code::Runtime(Routine::SpecialEnterMap) => return self.special_enter_map(ctx),
                Code::Runtime(Routine::OverworldLoop) => return self.overworld_loop(),
                Code::Runtime(Routine::AfterCardKeyText) => return self.sprite_or_sign_dialogue(ctx),
                Code::Runtime(Routine::CheckWarpsNoCollision) => return self.check_warps_no_collision(ctx),
                Code::Runtime(Routine::AfterPoison) => return self.after_poison(ctx),
                Code::Runtime(Routine::SafariWarp) => return self.safari_warp(ctx),
                Code::Runtime(Routine::EnterMapEnd) => return self.enter_map_end(ctx),
                Code::Runtime(Routine::EnterMapRest) => return self.enter_map_rest(ctx),
                Code::Runtime(Routine::StartStep) => return self.start_step(ctx),
                Code::Runtime(Routine::WarpPadFaded) => return self.warp_faded(ctx),
                code => scripts::resume(code, &mut Script { ow: self, ctx }),
            };
            match flow {
                Flow::Return => {}
                Flow::Jump(code) => self.rt.stack.push(code),
                Flow::Call(routine, then) => {
                    self.rt.stack.push(then);
                    self.rt.stack.push(routine);
                }
                Flow::Block(block, then) => {
                    if let Some(then) = then {
                        self.rt.stack.push(then);
                    }
                    if let Some(transition) = self.start_block(ctx, block) {
                        return transition;
                    }
                }
            }
        }
        panic!("a script never waited");
    }

    /// `None` when the block is already over.
    pub(super) fn start_block(&mut self, ctx: &mut Ctx, block: Block) -> Option<Transition> {
        match block {
            Block::Chain(block, routine) => {
                self.rt.stack.push(routine.into());
                self.start_block(ctx, *block)
            }
            Block::Frames(0) => None,
            Block::Frames(frames) => {
                self.rt.waiting = Waiting::Frames(frames);
                Some(Transition::Stay)
            }
            Block::Sound if ctx.audio.sound_finished() => None,
            Block::Sound => {
                self.rt.waiting = Waiting::Sound;
                Some(Transition::Stay)
            }
            Block::PrintText(commands) => {
                // `PrintText`'s `UpdateSprites` before its text; the `Delay3` after the box is loading.
                self.update_sprites(ctx);
                self.rt.waiting = Waiting::Child;
                Some(Transition::Push(Mode::TextBox(TextBox::script(commands))))
            }
            Block::Mode(mode) => {
                if let Mode::TwoOptionMenu(menu) = &*mode {
                    self.update_sprites_behind(ctx, menu);
                }
                self.rt.waiting = Waiting::Child;
                Some(Transition::Push(*mode))
            }
            Block::Battle(mode) => {
                self.rt.waiting = Waiting::Battle;
                Some(Transition::Push(*mode))
            }
            Block::Replace(mode) => Some(Transition::Replace(*mode)),
            Block::TextScrollButton => {
                self.rt.waiting = Waiting::TextScrollButton;
                self.text_scroll_button(ctx).then_some(Transition::Stay)
            }
            Block::HoldOpen => {
                self.rt.waiting = Waiting::HoldOpen;
                self.holding_open(ctx).then_some(Transition::Stay)
            }
            Block::MusicStopped => {
                self.rt.waiting = Waiting::MusicStopped;
                self.music_stopping(ctx).then_some(Transition::Stay)
            }
            Block::ChannelPlaying { channel, id } => {
                self.rt.waiting = Waiting::ChannelPlaying { channel, id };
                (ctx.audio.channel_sound_id(channel) == id).then_some(Transition::Stay)
            }
        }
    }

    /// `DisplayTwoOptionMenu`'s `UpdateSprites` between its border and its words, which hides a sprite
    /// the box covers: the menu is drawn on a copy of the screen for it.
    fn update_sprites_behind(&mut self, ctx: &mut Ctx, menu: &TwoOptionMenu) {
        let (ui, memory) = (ctx.screen.ui.clone(), *ctx.menu);
        menu.clone().enter(ctx);
        self.update_sprites(ctx);
        ctx.screen.ui = ui;
        *ctx.menu = memory;
    }

    /// A frame of whatever the script waits on.
    pub(super) fn script_frame(&mut self, ctx: &mut Ctx) -> Transition {
        let waiting = match self.rt.waiting {
            Waiting::Frames(1) | Waiting::Nothing => false,
            Waiting::Frames(frames) => {
                self.rt.waiting = Waiting::Frames(frames - 1);
                true
            }
            Waiting::Sound => !ctx.audio.sound_finished(),
            Waiting::Child | Waiting::Battle => true,
            Waiting::TextScrollButton => self.text_scroll_button(ctx),
            Waiting::HoldOpen => self.holding_open(ctx),
            Waiting::MusicStopped => self.music_stopping(ctx),
            Waiting::ChannelPlaying { channel, id } => ctx.audio.channel_sound_id(channel) == id,
        };
        if waiting { Transition::Stay } else { self.run_script(ctx) }
    }

    /// A mode the script pushed has popped.
    pub(super) fn script_resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        match self.rt.waiting {
            Waiting::Battle => self.battle_occurred(ctx, outcome),
            Waiting::Child => {
                self.rt.outcome = Some(outcome);
                self.run_script(ctx)
            }
            _ => Transition::Stay,
        }
    }

    /// `WaitForTextScrollButtonPress`'s poll: `true` while nothing is pressed.
    fn text_scroll_button(&mut self, ctx: &mut Ctx) -> bool {
        if !ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
            return true;
        }
        self.answered += 1;
        false
    }

    /// `HoldTextDisplayOpen`'s poll: `true` while A is held.
    fn holding_open(&mut self, ctx: &mut Ctx) -> bool {
        ctx.pad.poll();
        ctx.pad.held.contains(Joypad::A)
    }

    /// `StopMusic`'s wait, which spins until VBlank's fade has cleared `wAudioFadeOutControl`: `true`
    /// until then.
    fn music_stopping(&mut self, ctx: &mut Ctx) -> bool {
        ctx.audio.fading_out()
    }

    // ---- DisplayTextID ----

    /// `DisplayTextID`, from `DisplayTextIDInit` to what the text id names.
    fn display_text_id(&mut self, ctx: &mut Ctx, text_id: u8) -> Flow {
        let predef = std::mem::take(&mut self.rt.text_predef);
        if !self.rt.no_auto_text_box {
            if text_id == 0 {
                let height = if ctx.world.events.is_set(poke_core::symbols::pokered_events::EVENT_GOT_POKEDEX) { 14 } else { 12 };
                ctx.screen.ui.text_box_border(10, 0, 8, height);
            } else {
                ctx.screen.ui.text_box_border(0, 12, 18, 4);
            }
        }
        self.font_loaded = true;
        if !std::mem::take(&mut self.rt.no_sprite_updates) {
            self.update_sprites(ctx);
        }
        for sprite in self.sprites.iter_mut().skip(1) {
            sprite.orig_facing = sprite.facing;
        }
        for sprite in self.sprites.iter_mut() {
            if sprite.image_index != 0xFF {
                sprite.image_index &= 0xFC;
            }
        }
        ctx.screen.tiles.load_font();
        *ctx.frame_counter = TEXT_POLL_TIMER;
        self.rt.sprite_index = text_id;
        if text_id == 0 {
            ctx.audio.play_sound(sounds::SFX_START_MENU);
            self.walk_bike_surf_copy = ctx.world.location.walk_bike_surf;
            let (mut sprites, mut direction) = (self.sprites, 0);
            ctx.world.location.ahead.sprite = crate::systems::overworld::sprites::sprite_in_front_of_player(&mut sprites,
                self.num_sprites, 0x10, &mut direction) != 0;
            // `DisplayStartMenu` leaves through `CloseTextDisplay` itself.
            return Then::block(Block::Mode(Box::new(Mode::StartMenu(StartMenu::new())))).then(Routine::AfterStartMenu);
        }
        match text_id {
            TEXT_MON_FAINTED => return Then::block(Block::PrintText(text_at(pokered_symbols::PokemonFaintedText)))
                .then(Routine::AfterDisplayingTextId),
            // `DisplayPlayerBlackedOutText` holds the box open without waiting for a press.
            TEXT_BLACKED_OUT => {
                ctx.world.location.always_on_bike = false;
                return Then::block(Block::PrintText(text_at(pokered_symbols::PlayerBlackedOutText)))
                    .then(Routine::HoldTextDisplayOpen);
            }
            TEXT_REPEL_WORE_OFF => return Then::block(Block::PrintText(text_at(pokered_symbols::RepelWoreOffText)))
                .then(Routine::AfterDisplayingTextId),
            TEXT_SAFARI_GAME_OVER => {
                ctx.pad.ignore = Joypad::empty();
                return Then::call(super::events::Label::SafariGameOverText).then(Routine::AfterDisplayingTextId);
            }
            _ => {}
        }
        let text_id = if text_id <= self.num_sprites {
            // `UpdateSpriteFacingOffsetAndDelayMovement` works on `hCurrentSpriteOffset`, which
            // `UpdateSprites` has left on the last slot rather than the sprite spoken to.
            let last = &mut self.sprites[NUM_SPRITES - 1];
            last.movement_delay = 0x7F;
            last.anim_frame_counter = 0;
            last.intra_anim_frame_counter = 0;
            last.image_index |= last.facing;
            last.movement_status = 2;
            self.sprites[text_id as usize].text_id
        } else {
            text_id
        };
        let map = ctx.world.location.map;
        let text = match (predef, self.rt.text_pointers) {
            (Some(bank), _) => text_predef(text_id, bank),
            (None, Some(table)) => map_text_in(table, text_id),
            (None, None) => map_text(map, text_id),
        };
        match text {
            // `PrintText_NoCreatingTextBox`: `DisplayTextIDInit` drew the box, or was told not to.
            Ok(MapText::Plain(commands)) =>
                Then::block(Block::Mode(Box::new(Mode::TextBox(TextBox::without_box(commands))))).then(Routine::AfterTextCode),
            Ok(MapText::Dispatch(TX_SCRIPT_MART, at)) => {
                let list = poke_core::rom_gfx::rom_slice(at + 1);
                let items = list[1..=list[0] as usize].iter()
                    .map(|&id| ItemId::from_repr(id).expect("a mart sells items"))
                    .collect();
                Then::block(Block::Mode(Box::new(Mode::Pokemart(Pokemart::new(items))))).then(Routine::AfterDisplayingTextId)
            }
            Ok(MapText::Script(at)) if at == pokered_symbols::PickUpItemText => Then::call(Routine::PickUpItem).then(Routine::AfterTextCode),
            Ok(MapText::Script(at)) => {
                self.rt.stack.push(Routine::AfterTextCode.into());
                let rt = &mut Script { ow: self, ctx };
                let flow = if predef.is_some() { super::events::predef_text(rt, at) } else { scripts::text(map, rt, text_id) };
                match flow {
                    Some(flow) => flow,
                    None => {
                        // Code no map module recreates yet: the box comes down unread.
                        self.rt.stack.pop();
                        Flow::Jump(Routine::CloseTextDisplay.into())
                    }
                }
            }
            Ok(MapText::Dispatch(TX_SCRIPT_POKECENTER_NURSE, _)) =>
                Then::call(super::events::Label::PokemonCenter).then(Routine::AfterDisplayingTextId),
            Ok(MapText::Dispatch(TX_SCRIPT_VENDING_MACHINE, _)) =>
                Then::call(super::events::Label::VendingMachine).then(Routine::AfterDisplayingTextId),
            Ok(MapText::Dispatch(TX_SCRIPT_PRIZE_VENDOR, _)) =>
                Then::call(super::events::Label::PrizeMenu).then(Routine::HoldTextDisplayOpen),
            Ok(MapText::Dispatch(TX_SCRIPT_PLAYERS_PC, _)) => {
                self.rt.saved_screen2 = Some(ctx.screen.ui.clone());
                Then::call(Routine::PlayerPc).then(Routine::HoldTextDisplayOpen)
            }
            Ok(MapText::Dispatch(TX_SCRIPT_BILLS_PC, _)) => {
                self.rt.saved_screen2 = Some(ctx.screen.ui.clone());
                Then::call(Routine::BillsPc).then(Routine::HoldTextDisplayOpen)
            }
            Ok(MapText::Dispatch(TX_SCRIPT_POKECENTER_PC, _)) => Then::call(Routine::ActivatePc).then(Routine::HoldTextDisplayOpen),
            // The Cable Club is a non-goal.
            _ => Flow::Jump(Routine::CloseTextDisplay.into()),
        }
    }

    /// `CloseTextDisplay` after its `DelayFrame`.
    fn close_text_display_rest(&mut self, ctx: &mut Ctx) {
        self.load_gb_pal(ctx);
        for sprite in self.sprites.iter_mut().skip(1) {
            sprite.facing = sprite.orig_facing;
        }
        let location = &ctx.world.location;
        crate::systems::overworld::sprites::init_map_sprites(&mut self.sprites, &mut self.sprite_set, location.map,
            location.x, location.y, self.num_sprites, self.font_loaded, &mut ctx.screen.tiles);
        self.font_loaded = false;
        crate::systems::overworld::sprites::load_player_sprite_graphics(&mut ctx.screen.tiles, &mut ctx.world.location,
            self.view.tileset);
        self.update_sprites(ctx);
    }

    /// `PickUpItem`: the item ball the player spoke to into the bag and off the map.
    fn pick_up_item(&mut self, ctx: &mut Ctx) -> Flow {
        self.rt.no_auto_text_box = false;
        self.rt.do_not_wait = false;
        let slot = self.rt.sprite_index;
        let Some(&(_, toggle)) = self.toggleable.iter().find(|&&(sprite, _)| sprite == slot) else {
            return Flow::Return;
        };
        let objects = MapObjects::read(ctx.world.location.map).expect("the map has objects");
        let item = match objects.objects.get(slot as usize - 1).map(|object| object.kind) {
            Some(ObjectKind::Item(item)) => item,
            _ => 0,
        };
        let item = ItemId::from_repr(item).expect("an item ball holds an item");
        let found = if self.give_item(ctx, item, 1) {
            self.toggle_object(ctx, toggle as u16, true);
            self.rt.do_not_wait = true;
            pokered_symbols::FoundItemText
        } else {
            pokered_symbols::NoMoreRoomForItemText
        };
        Then::block(Block::PrintText(text_at(found))).ret()
    }

    /// `GiveItem`.
    pub(super) fn give_item(&mut self, ctx: &mut Ctx, item: ItemId, quantity: u8) -> bool {
        if !ctx.world.bag.add(item, quantity) {
            return false;
        }
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, poke_core::item::name(item));
        true
    }

    /// `HideObject` or `ShowObject`: the toggleable object's flag, then `UpdateSprites`.
    pub(super) fn toggle_object(&mut self, ctx: &mut Ctx, toggle: u16, hide: bool) {
        let flags = &mut ctx.world.location.hidden_objects;
        let (byte, bit) = (toggle as usize / 8, toggle % 8);
        if flags.len() <= byte {
            flags.resize(byte + 1, 0);
        }
        if hide {
            flags[byte] |= 1 << bit;
        } else {
            flags[byte] &= !(1 << bit);
        }
        self.update_sprites(ctx);
    }
}

#[cfg(test)]
mod tests {
    use poke_core::map::Map;
    use crate::audio::data::AudioBank;
    use crate::audio::engine::AudioEngine;
    use crate::gfx::Screen;
    use crate::input::Pad;
    use crate::mode::ModeUpdate;
    use crate::modes::menu_input::CursorMemory;
    use crate::rng::GameRng;
    use crate::systems::overworld::Location;
    use crate::world::World;
    use crate::Pacing;
    use super::*;

    #[test]
    fn a_replaced_block_is_the_block_the_map_draws_and_a_load_asks_for_it_again() {
        let mut world = World { location: Location { map: Map::PalletTown, x: 5, y: 8, ..Location::default() }, ..World::default() };
        let (mut pad, mut rng, mut counter, mut screen) = (Pad::default(), GameRng::seeded(1), 0, Screen::default());
        let (mut menu, mut audio, mut events) = (CursorMemory::default(), AudioEngine::new(AudioBank::One), Vec::new());
        let mut ctx = Ctx {
            world: &mut world, pad: &mut pad, rng: &mut rng, frame_counter: &mut counter, screen: &mut screen,
            menu: &mut menu, audio: &mut audio, events: &mut events, pacing: Pacing::Faithful,
            update_sprites: false, save_game: false, saved_player_id: None,
        };
        let mut overworld = Overworld::new();
        overworld.enter(&mut ctx);
        overworld.rt.cur_map_loaded = [true; 2];
        let mut rt = Script { ow: &mut overworld, ctx: &mut ctx };
        assert!(rt.check_and_reset_cur_map_loaded(1));
        assert!(!rt.check_and_reset_cur_map_loaded(1), "reset by the asking");
        assert!(rt.check_and_reset_cur_map_loaded(2));
        rt.replace_tile_block(2, 3, 0x0F);
        let stride = overworld.view.stride() as usize;
        assert_eq!(overworld.view.blocks[stride * 3 + 3 + stride * 3 + 2], 0x0F);
        overworld.present(&mut ctx);
        assert_eq!(ctx.screen.map.blocks, overworld.view.blocks, "the screen draws the new block");
    }

    /// `PewterGymBrockText` sets `wGymLeaderNo` after `EngageMapTrainer`, so his encounter music
    /// plays, and the same byte as `wLoneAttackNo` gives his Onix BIDE and the battle his music.
    #[test]
    fn brock_meets_the_player_to_his_music_and_fights_with_bide() {
        use poke_core::charmap::encode;
        use poke_core::move_name::PokemonMoveName;
        use poke_core::species::PokemonSpecies;
        use poke_core::sprite::SpriteFacing;
        use crate::audio::data::{sounds, Sound};
        use crate::command::{Command, Decision, Reply};
        use crate::mode::{Mode, Status};
        use crate::party::Named;
        use crate::systems::add_mon::{new_party_mon, Origin};
        use crate::{Game, Input};

        let playing = |game: &Game, sound: Sound| (0..4).any(|channel| game.audio().channel_sound_id(channel) == sound.id.0);
        let overworld = |game: &Game| game.modes().iter().find_map(|mode| match mode {
            Mode::Overworld(overworld) => Some(overworld.rt.gym_leader_no),
            _ => None,
        });
        let mut world = World { player_name: encode("RED").unwrap(), ..World::default() };
        world.location = Location { map: Map::PewterGym, x: 4, y: 2, facing: SpriteFacing::Up, last_map: Map::PewterCity, ..Location::default() };
        world.party = vec![Named {
            mon: new_party_mon(PokemonSpecies::Pidgey, 40, 1, &Origin::Trainer, &mut GameRng::tape(vec![])),
            ot: encode("RED").unwrap(),
            nick: encode("MON").unwrap(),
        }];
        world.events.set(poke_core::symbols::pokered_events::EVENT_FOLLOWED_OAK_INTO_LAB);
        let mut game = Game::new(world, GameRng::seeded(5), Pacing::Faithful);
        game.push(Mode::Overworld(Overworld::new()));
        let mut asked = false;
        for _ in 0..20_000 {
            if overworld(&game) == Some(1) {
                break;
            }
            let input = match game.status() {
                Status::Waiting(Decision::Overworld) if !asked => {
                    asked = true;
                    Input::Command(Command::Interact)
                }
                Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
                _ => Input::None,
            };
            if let Some(reply) = game.frame(input).reply {
                assert_eq!(reply, Reply::Accepted);
            }
        }
        assert_eq!(overworld(&game), Some(1), "Brock's text engages him");
        assert!(playing(&game, sounds::MUSIC_MEET_MALE_TRAINER), "the encounter music plays for a gym leader too");

        let battle = |game: &Game| game.modes().iter().find_map(|mode| match mode {
            Mode::Battle(battle) => battle.battle().map(|battle| battle.enemy_party.clone()),
            _ => None,
        });
        for _ in 0..20_000 {
            if battle(&game).is_some() && game.status() == Status::Waiting(Decision::Text) {
                break;
            }
            let input = match game.status() {
                Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
                _ => Input::None,
            };
            game.frame(input);
        }
        let onix = battle(&game).expect("the battle began")[1].clone();
        assert_eq!(onix.mon.species, PokemonSpecies::Onix);
        assert!(onix.mon.moves.contains(&Some(PokemonMoveName::Bide)), "{:?}", onix.mon.moves);
        assert!(playing(&game, sounds::MUSIC_GYM_LEADER_BATTLE));
    }
}

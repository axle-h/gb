//! `home/trainers.asm` and `engine/overworld/trainer_sight.asm`: a trainer seeing the player, the
//! `!`, the walk up, the words before the battle, and the flag once it is won.

use poke_core::map::Map;
use poke_core::map_objects::{MapObjects, ObjectKind};
use poke_core::symbols::DmgPointer;
use poke_core::trainer_headers::{encounter_music, EncounterMusic, TrainerHeader, OPP_ID_OFFSET};
use crate::audio::data::{sounds, SoundId};
use crate::input::Joypad;
use crate::mode::Ctx;
use crate::systems::overworld::sprites::{SPRITE_FACING_DOWN, SPRITE_FACING_LEFT, SPRITE_FACING_RIGHT, SPRITE_FACING_UP};
use poke_core::map_objects::STAY;
use super::script::{text_at, Block, Flow, Routine, Then};
use super::{Overworld, PAD_CTRL_PAD};

/// The player's sprite on the screen, which never moves.
const PLAYER_Y: u8 = 0x3C;
const PLAYER_X: u8 = 0x40;
const NPC_MOVEMENT_DOWN: u8 = 0x00;
const NPC_MOVEMENT_UP: u8 = 0x40;
const NPC_MOVEMENT_LEFT: u8 = 0x80;
const NPC_MOVEMENT_RIGHT: u8 = 0xC0;
/// `OPP_RIVAL1` to `OPP_RIVAL3`, whose engagement plays no music of its own.
pub(super) const RIVAL_CLASSES: [u8; 3] = [OPP_ID_OFFSET + 25, OPP_ID_OFFSET + 42, OPP_ID_OFFSET + 43];
/// `EXCLAMATION_BUBBLE`.
const EXCLAMATION_BUBBLE: u8 = 0;

impl Overworld {
    /// The header `wTrainerHeaderPtr` holds.
    fn trainer_header(&self) -> TrainerHeader {
        TrainerHeader::read(self.rt.trainer_header.expect("a trainer routine runs with a header"))
    }

    /// `CheckFightingMapTrainers`, with `CheckForEngagingTrainers` over the map's table.
    pub(super) fn check_fighting_map_trainers(&mut self, ctx: &mut Ctx) -> Flow {
        let first = self.rt.trainer_header.expect("the map's table");
        let mut engaging = None;
        for header in TrainerHeader::table(first) {
            self.rt.trainer_header = Some(header.at);
            self.rt.sprite_index = header.sprite;
            self.rt.trainer_header_flag_bit = header.sprite;
            if ctx.world.events.is_set(header.event) {
                continue;
            }
            if self.trainer_engage(ctx, header.sprite, header.range << 4) {
                engaging = Some(header.sprite);
                break;
            }
        }
        let Some(slot) = engaging else {
            self.rt.sprite_index = 0;
            self.rt.trainer_header_flag_bit = 0;
            return Flow::Return;
        };
        self.rt.trainer_battle = true;
        self.emotion_bubble(ctx, slot, EXCLAMATION_BUBBLE);
        Then::block(Block::Chain(Box::new(Block::Frames(super::movement::EMOTION_BUBBLE_FRAMES)), Routine::EmotionBubbleEnd))
            .then(Routine::AfterEngageBubble)
    }

    /// The rest of `CheckFightingMapTrainers` once the `!` is gone.
    pub(super) fn after_engage_bubble(&mut self, ctx: &mut Ctx) -> Flow {
        ctx.pad.ignore = PAD_CTRL_PAD;
        ctx.pad.held = Joypad::empty();
        self.trainer_walk_up_to_player(ctx);
        ctx.world.scripts.cur_map_script = ctx.world.scripts.cur_map_script.wrapping_add(1);
        Flow::Return
    }

    /// `TrainerEngage`: a trainer on screen, in line with the player, facing them, near enough.
    fn trainer_engage(&mut self, ctx: &mut Ctx, slot: u8, distance: u8) -> bool {
        let sprite = self.sprites[slot as usize];
        if sprite.image_index == 0xFF {
            return false;
        }
        let facing = sprite.facing;
        let (y, x) = (sprite.y_pixels, sprite.x_pixels);
        let gap = if y == PLAYER_Y {
            x.abs_diff(PLAYER_X)
        } else if x == PLAYER_X {
            y.abs_diff(PLAYER_Y)
        } else {
            return false;
        };
        if gap == 0 {
            return false;
        }
        // `CheckSpriteCanSeePlayer`.
        let lined_up = match facing {
            SPRITE_FACING_DOWN | SPRITE_FACING_UP => x == PLAYER_X,
            SPRITE_FACING_LEFT | SPRITE_FACING_RIGHT => y == PLAYER_Y,
            _ => false,
        };
        if distance < gap || !lined_up {
            return false;
        }
        // `CheckPlayerIsInFrontOfSprite`, bypassed in the Power Plant for its Voltorb items.
        let in_front = ctx.world.location.map == Map::PowerPlant || {
            let y = if y == 0xFC { 0x0C } else { y };
            match facing {
                SPRITE_FACING_DOWN => y < PLAYER_Y,
                SPRITE_FACING_UP => y >= PLAYER_Y,
                SPRITE_FACING_LEFT => x >= PLAYER_X,
                _ => x < PLAYER_X,
            }
        };
        if !in_front {
            return false;
        }
        self.rt.seen_by_trainer = true;
        self.engage_map_trainer(ctx);
        true
    }

    /// `EngageMapTrainer`: the class and party of the sprite in `wSpriteIndex`, and its music.
    pub(super) fn engage_map_trainer(&mut self, ctx: &mut Ctx) {
        let slot = self.rt.sprite_index;
        let objects = MapObjects::read(ctx.world.location.map).expect("the map has objects");
        let (class, set) = match objects.objects.get(slot as usize - 1).map(|object| object.kind) {
            Some(ObjectKind::Trainer { class, number }) => (class, number),
            Some(ObjectKind::Item(item)) => (item, 0),
            _ => (0, 0),
        };
        self.rt.engaged_class = class;
        self.rt.engaged_set = set;
        self.play_trainer_music(ctx);
    }

    /// `PlayTrainerMusic`.
    fn play_trainer_music(&mut self, ctx: &mut Ctx) {
        let class = self.rt.engaged_class;
        if RIVAL_CLASSES.contains(&class) || self.rt.gym_leader_no != 0 {
            return;
        }
        ctx.audio.fade_out(0);
        ctx.audio.play_sound(SoundId::STOP_ALL_MUSIC);
        let music = match encounter_music(class.wrapping_sub(OPP_ID_OFFSET)) {
            EncounterMusic::Evil => sounds::MUSIC_MEET_EVIL_TRAINER,
            EncounterMusic::Female => sounds::MUSIC_MEET_FEMALE_TRAINER,
            EncounterMusic::Male => sounds::MUSIC_MEET_MALE_TRAINER,
        };
        ctx.audio.play_music(music);
    }

    /// `TrainerWalkUpToPlayer`: a path to the square in front of the player.
    fn trainer_walk_up_to_player(&mut self, ctx: &mut Ctx) {
        let slot = self.rt.sprite_index;
        let sprite = self.sprites[slot as usize];
        let (gap, direction) = match sprite.facing {
            SPRITE_FACING_DOWN => (sprite.y_pixels.abs_diff(PLAYER_Y), NPC_MOVEMENT_DOWN),
            SPRITE_FACING_UP => (sprite.y_pixels.abs_diff(PLAYER_Y), NPC_MOVEMENT_UP),
            SPRITE_FACING_LEFT => (sprite.x_pixels.abs_diff(PLAYER_X), NPC_MOVEMENT_LEFT),
            _ => (sprite.x_pixels.abs_diff(PLAYER_X), NPC_MOVEMENT_RIGHT),
        };
        if gap == 0x10 {
            return;
        }
        let steps = gap.rotate_left(4).wrapping_sub(1);
        let mut path = vec![direction; steps as usize];
        path.push(STAY);
        self.rt.paths.directions2 = path.clone();
        self.move_sprite(ctx, slot, &path);
    }

    /// `DisplayEnemyTrainerTextAndStartBattle`: once the trainer has walked up, its words, then the
    /// battle.
    pub(super) fn display_enemy_trainer_text_and_start_battle(&mut self, ctx: &mut Ctx) -> Flow {
        if self.rt.paths.scripted_npc_movement {
            return Flow::Return;
        }
        ctx.pad.ignore = Joypad::empty();
        Then::call(Routine::DisplayTextId(self.rt.sprite_index)).then(Routine::StartTrainerBattle)
    }

    /// `StartTrainerBattle`, with `InitBattleEnemyParameters`.
    pub(super) fn start_trainer_battle(&mut self, ctx: &mut Ctx) -> Flow {
        ctx.pad.ignore = Joypad::empty();
        let class = self.rt.engaged_class;
        self.rt.cur_opponent = class;
        self.rt.enemy_mon_or_trainer_class = class;
        if class >= OPP_ID_OFFSET {
            self.rt.trainer_no = self.rt.engaged_set;
        } else {
            self.rt.cur_enemy_level = self.rt.engaged_set;
        }
        self.rt.talked_to_trainer = true;
        self.rt.print_end_battle_text = true;
        ctx.world.scripts.cur_map_script = ctx.world.scripts.cur_map_script.wrapping_add(1);
        Flow::Return
    }

    /// `EndTrainerBattle`: the trainer's flag set, a Pokémon-shaped opponent taken off the map, and
    /// the map's script back to its first routine.
    pub(super) fn end_trainer_battle(&mut self, ctx: &mut Ctx) -> Flow {
        self.rt.cur_map_loaded = [true; 2];
        self.rt.print_end_battle_text = false;
        self.rt.seen_by_trainer = false;
        if !self.rt.lost_battle {
            // The flag's byte is the table's own and its bit the one saved at the engagement.
            let header = self.trainer_header();
            let event = header.event - header.sprite as u16 + self.rt.trainer_header_flag_bit as u16;
            ctx.world.events.set(event);
            if self.rt.enemy_mon_or_trainer_class < OPP_ID_OFFSET {
                let slot = self.rt.sprite_index;
                if let Some(&(_, toggle)) = self.toggleable.iter().find(|&&(sprite, _)| sprite == slot) {
                    self.toggle_object(ctx, toggle as u16, true);
                }
            }
            if std::mem::take(&mut self.rt.unknown_5_4) {
                return Flow::Return;
            }
        }
        // `ResetButtonPressedAndMapScript`.
        ctx.pad.ignore = Joypad::empty();
        ctx.pad.held = Joypad::empty();
        ctx.pad.pressed = Joypad::empty();
        ctx.pad.released = Joypad::empty();
        ctx.world.scripts.cur_map_script = 0;
        Flow::Return
    }

    /// `TalkToTrainer`: the after-battle words for a trainer already beaten, else the words before
    /// the battle and the battle's end text saved.
    pub(super) fn talk_to_trainer(&mut self, ctx: &mut Ctx, at: DmgPointer) -> Flow {
        let header = TrainerHeader::read(at);
        if self.talk_to_trainer_header(ctx, at) {
            return Then::block(Block::PrintText(text_at(header.after_battle))).ret();
        }
        Then::block(Block::PrintText(text_at(header.before_battle))).then(Routine::TalkToTrainerNotYetFought)
    }

    /// `TalkToTrainer` up to its choice of text: the header saved, and whether its trainer is beaten.
    pub(super) fn talk_to_trainer_header(&mut self, ctx: &mut Ctx, at: DmgPointer) -> bool {
        self.rt.trainer_header = Some(at);
        let header = TrainerHeader::read(at);
        self.rt.trainer_header_flag_bit = header.sprite;
        ctx.world.events.is_set(header.event)
    }

    /// `.trainerNotYetFought` after its `PrintText`.
    pub(super) fn talk_to_trainer_not_yet_fought(&mut self, ctx: &mut Ctx) -> Flow {
        let header = self.trainer_header();
        self.rt.end_battle_text = Some(header.end_battle);
        self.rt.use_cur_map_script = true;
        if self.rt.seen_by_trainer {
            return Flow::Return;
        }
        self.engage_map_trainer(ctx);
        ctx.world.scripts.cur_map_script = ctx.world.scripts.cur_map_script.wrapping_add(1);
        Flow::Jump(Routine::StartTrainerBattle.into())
    }
}

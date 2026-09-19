//! `NewBattle` from the overworld's side: a trainer or a script's opponent, `TryDoWildEncounter`
//! over the map's own data, and `.battleOccurred` with `HandleBlackOut` when the party is spent.

use poke_core::map::Map;
use poke_core::map_header::TileSetId;
use poke_core::map_objects::fly_warp;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_events::EVENT_2A7;
use poke_core::trainer_headers::OPP_ID_OFFSET;
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, Outcome, Transition};
use crate::modes::battle::BattleMode;
use crate::systems::math::divide_bcd;
use crate::systems::overworld::encounters::{try_do_wild_encounter, EncounterInput};
use crate::systems::overworld::map_view::MapView;
use crate::systems::overworld::sprites::SpriteState;
use crate::systems::events::heal_party::heal_party;
use super::script::{text_at, Block, Flow, Routine, Then, Waiting};
use super::{AfterFade, Overworld, Phase};

/// `EnterMap`'s steps without a random battle after one.
pub(super) const NO_RANDOM_BATTLE_STEPS: u8 = 3;
/// `.battleOccurred`'s `DelayFrames 10`.
const AFTER_BATTLE_FRAMES: u8 = 10;
/// `SpecialEnterMap`'s `DelayFrames 20`.
pub(super) const SPECIAL_ENTER_MAP_FRAMES: u8 = 20;
/// `StopMusic`'s fade, a step every eight frames.
const STOP_MUSIC_FADE: u8 = 8;

impl Overworld {
    /// `IsPlayerCharacterBeingControlledByGame`.
    pub(super) fn controlled_by_game(&self) -> bool {
        self.rt.npc_movement_script_table != 0 || self.exiting_door || self.scripted
    }

    /// `.newBattle`: a battle, or `CheckWarpsNoCollision` when `NewBattle` finds none.
    pub(super) fn new_battle(&mut self, ctx: &mut Ctx) -> Transition {
        let battle = self.start_new_battle(ctx);
        self.standing.standing_on_warp = false;
        if let Some(text) = self.repel_wore_off_text(Routine::CheckWarpsNoCollision) {
            return self.run_script_from(ctx, text);
        }
        match battle {
            Some(transition) => transition,
            None => self.check_warps_no_collision(ctx),
        }
    }

    /// `NewBattle`: the battle pushed, if there is one.
    pub(super) fn start_new_battle(&mut self, ctx: &mut Ctx) -> Option<Transition> {
        let mode = self.init_battle(ctx)?;
        self.rt.stack.clear();
        self.phase = Phase::Script;
        self.rt.waiting = Waiting::Battle;
        Some(Transition::Push(Mode::Battle(mode)))
    }

    /// `NewBattle` and `InitBattle` up to the battle itself.
    fn init_battle(&mut self, ctx: &mut Ctx) -> Option<BattleMode> {
        if self.controlled_by_game() || self.rt.no_battles {
            return None;
        }
        let opponent = self.rt.cur_opponent;
        if opponent == 0 {
            if self.rt.no_random_battle_steps != 0 {
                return None;
            }
            let (species, level) = self.try_do_wild_encounter(ctx)?;
            return Some(BattleMode::wild(species, level));
        }
        if opponent < OPP_ID_OFFSET {
            let species = PokemonSpecies::from_repr(opponent).expect("a wild opponent is a species");
            let level = self.rt.cur_enemy_level;
            return Some(match self.rt.old_man_battle {
                true => BattleMode::old_man(species, level),
                false => BattleMode::wild(species, level),
            });
        }
        let battle = BattleMode::trainer(opponent - OPP_ID_OFFSET, self.rt.trainer_no, self.rt.gym_leader_no,
            ctx.world.scripts.rival_starter);
        Some(match self.rt.end_battle_text.filter(|_| self.rt.print_end_battle_text) {
            Some(words) => battle.with_end_battle_text(text_at(words)),
            None => battle,
        })
    }

    /// `.lastRepelStep`'s `DisplayTextID`, which `NewBattle` shows before it returns to `then`.
    pub(super) fn repel_wore_off_text(&mut self, then: Routine) -> Option<Vec<crate::scripts::Code>> {
        if !std::mem::take(&mut self.rt.repel_wore_off) {
            return None;
        }
        self.rt.no_auto_text_box = false;
        self.rt.do_not_wait = false;
        Some(vec![then.into(), Routine::DisplayTextId(super::script::TEXT_REPEL_WORE_OFF).into()])
    }

    /// `TryDoWildEncounter`: the species and level of a wild mon that appears, if one does. The tiles
    /// under the player come from the map's blocks rather than the drawn screen.
    fn try_do_wild_encounter(&mut self, ctx: &mut Ctx) -> Option<(PokemonSpecies, u8)> {
        if self.rt.npc_movement_script_table != 0 {
            return None;
        }
        // `wMovementFlags`.
        if self.standing_on_door || self.exiting_door || self.standing.standing_on_warp || self.jumping {
            return None;
        }
        let location = &ctx.world.location;
        let input = EncounterInput {
            map: location.map,
            tileset: self.view.tileset,
            bottom_left: self.view.tile(8, 9),
            bottom_right: self.view.tile(9, 9),
            x: location.x,
            y: location.y,
            width: self.view.width,
            height: self.view.height,
            repel_steps: location.repel_steps,
            lead_level: ctx.world.party.first().map_or(0, |mon| mon.mon.level),
            wild: self.rt.wild_mons,
        };
        let encounter = try_do_wild_encounter(&input, ctx.rng);
        ctx.world.location.repel_steps = encounter.repel_steps;
        self.rt.repel_wore_off = encounter.repel_wore_off;
        encounter.mon
    }

    /// `.battleOccurred`, with what `EndOfBattle` leaves for the overworld.
    pub(super) fn battle_occurred(&mut self, ctx: &mut Ctx, outcome: Outcome) -> Transition {
        if let Outcome::Chosen(result) = outcome {
            self.rt.battle_result = result;
        }
        let won_against_trainer = outcome == Outcome::Chosen(crate::modes::battle::result::WON) && self.rt.cur_opponent >= OPP_ID_OFFSET;
        if won_against_trainer && std::mem::take(&mut self.rt.print_end_battle_text) {
            self.set_enemy_trainer_to_stay_and_face_any_direction(ctx);
        }
        self.rt.cur_opponent = 0;
        self.standing.destination_warp = 0xFF;
        self.rt.wild_encounter_cooldown = true;
        self.rt.talked_to_trainer = false;
        self.rt.trainer_battle = false;
        ctx.pad.held = Joypad::empty();
        let map = ctx.world.location.map;
        if map == Map::CinnabarGym {
            ctx.world.events.set(EVENT_2A7);
        }
        self.rt.battle_over_or_blackout = true;
        self.rt.cur_map_loaded = [true; 2];
        let fainted = ctx.world.party.iter().all(|mon| mon.mon.mon.hp == 0);
        // No blacking out after losing to the rival in Oak's lab.
        if map != Map::OaksLab && fainted {
            self.rt.lost_battle = true;
            self.rt.stack = vec![Routine::HandleBlackOut.into(), Routine::MapScript.into()];
            return self.run_script(ctx);
        }
        self.rt.stack = vec![Routine::EnterMapAfterBattle.into()];
        self.rt.waiting = Waiting::Frames(AFTER_BATTLE_FRAMES);
        self.phase = Phase::Script;
        Transition::Stay
    }

    /// `PrintEndBattleText`'s `SetEnemyTrainerToStayAndFaceAnyDirection`: the beaten trainer stands
    /// and turns at random, but for the rival and Pokémon Tower 7F's Rockets, who leave.
    fn set_enemy_trainer_to_stay_and_face_any_direction(&mut self, ctx: &Ctx) {
        if ctx.world.location.map == Map::PokemonTower7F || super::trainers::RIVAL_CLASSES.contains(&self.rt.engaged_class) {
            return;
        }
        self.set_sprite_movement_bytes_to_ff(self.rt.sprite_index);
    }

    /// `HandleBlackOut`'s `GBFadeOutToBlack`.
    pub(super) fn handle_black_out(&mut self, ctx: &mut Ctx) -> Transition {
        self.fade_out_to_black(ctx, AfterFade::BlackOut)
    }

    /// `HandleBlackOut` once black: `StopMusic`, then the rest.
    pub(super) fn black_out_music(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.audio.stop_music(STOP_MUSIC_FADE);
        self.rt.stack = vec![Routine::BlackOutFaded.into()];
        self.phase = Phase::Script;
        match self.start_block(ctx, Block::MusicStopped) {
            Some(transition) => transition,
            None => self.run_script(ctx),
        }
    }

    /// The rest of `HandleBlackOut`: `ResetStatusAndHalveMoneyOnBlackout`, `PrepareForSpecialWarp`,
    /// the map's music, and `SpecialEnterMap` up to its wait.
    pub(super) fn black_out_faded(&mut self, ctx: &mut Ctx) -> Flow {
        // `StopMusic`'s `StopAllSounds`.
        ctx.audio.stop_all_sounds();
        self.rt.battle_over_or_blackout = false;
        self.rt.lost_battle = false;
        let location = &mut ctx.world.location;
        location.walk_bike_surf = 0;
        self.map_pal_offset = 0;
        self.rt.npc_movement_script_table = 0;
        self.rt.npc_movement_script_function = 0;
        ctx.pad.held = Joypad::empty();
        self.rt.seen_by_trainer = false;
        self.turning = false;
        let mut money = ctx.world.money;
        ctx.world.money = divide_bcd(&mut money, [0, 0, 2]);
        ctx.pad.ignore = Joypad::all();
        heal_party(&mut ctx.world.party);
        // `PrepareForSpecialWarp` to `wLastBlackoutMap`, from `FlyWarpDataPtr`.
        let location = &mut ctx.world.location;
        location.map = location.last_blackout_map;
        location.last_map = location.map;
        let warp = fly_warp(location.map).expect("a blackout returns to a town with a fly warp");
        self.view.view = MapView::view_from_address(warp.view);
        location.y = warp.y;
        location.x = warp.x;
        self.view.y_block = warp.y & 1;
        self.view.x_block = warp.x & 1;
        self.view.tileset = TileSetId::Overworld;
        super::play_default_music_fade_out_current(ctx, false);
        // `SpecialEnterMap`.
        ctx.pad.pressed = Joypad::empty();
        ctx.pad.held = Joypad::empty();
        ctx.world.play_time.counting = true;
        let player = SpriteState { picture_id: 1, image_base_offset: 1, y_pixels: 0x3C, x_pixels: 0x40, ..SpriteState::default() };
        self.sprites[0] = player;
        Then::block(Block::Frames(SPECIAL_ENTER_MAP_FRAMES)).then(Routine::SpecialEnterMap)
    }

    /// `SpecialEnterMap` after its wait.
    pub(super) fn special_enter_map(&mut self, ctx: &mut Ctx) -> Transition {
        self.enter_map(ctx)
    }

    /// `IsPlayerStandingOnWarp`.
    pub(super) fn is_player_standing_on_warp(&mut self, ctx: &Ctx) {
        let location = &ctx.world.location;
        if let Some(warp) = self.warps.iter().find(|warp| warp.y == location.y && warp.x == location.x) {
            self.standing.destination_warp = warp.destination_warp;
            self.warp_destination_map = warp.destination_map;
            self.standing.standing_on_warp = true;
        }
    }
}


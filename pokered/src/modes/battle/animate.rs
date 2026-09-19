//! Where the battle calls its animations: the `PlayMoveAnimation` family, each tracking
//! `wAnimationType` and `hWhoseTurn` as the cartridge writes them, and which animation a move
//! effect plays beside which of its texts.

use poke_core::move_name::PokemonMoveName;
use crate::systems::battle::effects::BattleText;
use crate::systems::battle::{effect, BattleKind, Side, Status1, Status2, Status3};
use super::animation::{anim, animation_type, AnimBattle, Routine};
use super::present::Present;
use super::BattleMode;

/// The battle state an effect's animation reads, taken before the effect runs.
#[derive(Debug, Clone, Copy)]
pub(super) struct Before {
    pub hp: [u16; 2],
    pub substitute: [bool; 2],
    pub minimized: [u8; 2],
    pub species: [poke_core::species::PokemonSpecies; 2],
    pub transformed: [bool; 2],
}

fn index(side: Side) -> usize {
    match side {
        Side::Player => 0,
        Side::Enemy => 1,
    }
}

impl Before {
    pub fn of(battle: &crate::systems::battle::Battle) -> Self {
        let sides = [&battle.player, &battle.enemy];
        Self {
            hp: sides.map(|side| side.mon.hp),
            substitute: sides.map(|side| side.status2.contains(Status2::HAS_SUBSTITUTE_UP)),
            minimized: sides.map(|side| side.minimized),
            species: sides.map(|side| side.mon.species),
            transformed: sides.map(|side| side.status3.contains(Status3::TRANSFORMED)),
        }
    }
}

impl BattleMode {
    /// The battle as an animation reads it now.
    pub(super) fn anim_battle(&self) -> AnimBattle {
        let battle = self.b();
        AnimBattle {
            player_species: battle.player.mon.species,
            enemy_species: battle.enemy.mon.species,
            damage_multipliers: battle.damage_multipliers,
            trainer_battle: battle.kind == BattleKind::Trainer,
            item: 0,
            ball_data: 0,
            animations_on: true,
            h_scx: self.h_scx,
            mons: self.mon_palettes(),
            // The bars as the HUDs last drew them are filled in when the animation starts.
            hp_bar_colours: Default::default(),
        }
    }

    pub(super) fn animate(&mut self, routine: Routine, turn: Side) {
        self.whose_turn = turn;
        let battle = self.anim_battle();
        self.push(Present::Animation { routine, turn, battle });
    }

    /// `PlayMoveAnimation`: animation `id` with `wAnimationType` as it stands.
    pub(super) fn play_move_animation(&mut self, id: u8, turn: Side) {
        self.animate(Routine::PlayMoveAnimation { id, kind: self.animation_type }, turn);
    }

    /// `PlayBattleAnimation` after its caller zeroes `wAnimationType`.
    pub(super) fn play_battle_animation(&mut self, id: u8, turn: Side) {
        self.animation_type = animation_type::NONE;
        self.play_move_animation(id, turn);
    }

    /// `PlayCurrentMoveAnimation`: the side's move, with no applying animation, unless it has none.
    pub(super) fn play_current_move_animation(&mut self, side: Side) {
        self.animation_type = animation_type::NONE;
        let id = self.b().side(side).current_move.animation;
        if id != 0 {
            self.play_move_animation(id, side);
        }
    }

    /// `PlayBattleAnimation2`: the screen shaken slowly after, three pixels for the player and six
    /// for the enemy.
    pub(super) fn play_battle_animation2(&mut self, id: u8, side: Side) {
        self.animation_type = match side {
            Side::Player => animation_type::SHAKE_SCREEN_HORIZONTALLY_SLOW_2,
            Side::Enemy => animation_type::SHAKE_SCREEN_HORIZONTALLY_SLOW,
        };
        self.play_move_animation(id, side);
    }

    /// `PlayCurrentMoveAnimation2`.
    pub(super) fn play_current_move_animation2(&mut self, side: Side) {
        let id = self.b().side(side).current_move.animation;
        if id != 0 {
            self.play_battle_animation2(id, side);
        }
    }

    /// `PlayPlayerMoveAnimation` and `PlayEnemyMoveAnimation`: the substitute out of the way, the
    /// move, Selfdestruct's and Explosion's extra punch, the HUD, and the substitute back.
    pub(super) fn move_animation(&mut self, side: Side, kind: u8) {
        let me = self.b().side(side);
        let (substitute, minimized, id) = (me.status2.contains(Status2::HAS_SUBSTITUTE_UP), me.minimized != 0, me.current_move.animation);
        if substitute {
            self.animate(Routine::HideSubstituteShowMon { substitute_up: true, minimized }, side);
        }
        self.animation_type = kind;
        self.play_move_animation(id, side);
        self.handle_exploding_animation(side);
        self.push_hud(side);
        if substitute {
            self.animate(Routine::ReshowSubstitute, side);
        }
    }

    /// `HandleExplodingAnimation`. On the enemy's turn it tests the enemy's own invulnerability.
    fn handle_exploding_animation(&mut self, side: Side) {
        const GHOST: u8 = 0x08;
        let battle = self.b();
        let id = battle.side(side).current_move.animation;
        if !matches!(id, anim::SELFDESTRUCT | anim::EXPLOSION) {
            return;
        }
        let target = &battle.side(side.other()).mon;
        if battle.enemy.status1.contains(Status1::INVULNERABLE) || target.types.contains(&GHOST) || battle.move_missed {
            return;
        }
        self.animation_type = animation_type::SHAKE_SCREEN_HORIZONTALLY_LIGHT;
        self.play_move_animation(anim::MEGA_PUNCH, side);
    }

    /// `ENEMY_HUD_SHAKE_ANIM` on the player's turn, `SHAKE_SCREEN_ANIM` on the enemy's.
    fn status_shake(side: Side) -> u8 {
        match side {
            Side::Player => anim::ENEMY_HUD_SHAKE_ANIM,
            Side::Enemy => anim::SHAKE_SCREEN_ANIM,
        }
    }

    /// What a move effect's routine plays before `text`, from the battle `before` it ran.
    pub(super) fn effect_animation(&mut self, side: Side, move_effect: u8, text: BattleText, before: &Before) {
        use effect::*;
        use BattleText::*;
        let me = index(side);
        match text {
            FellAsleepText => self.play_current_move_animation2(side),
            PoisonedText | BadlyPoisonedText if move_effect == POISON_EFFECT => self.play_current_move_animation2(side),
            PoisonedText | BadlyPoisonedText => self.play_battle_animation2(Self::status_shake(side), side),
            BurnedText | FrozenText | ParalyzedMayNotAttackText if side == Side::Player && move_effect != PARALYZE_EFFECT =>
                self.play_battle_animation(anim::ENEMY_HUD_SHAKE_ANIM, side),
            MonsStatsRoseText => {
                let minimize = self.b().side(side).current_move.animation == PokemonMoveName::Minimize as u8;
                let substitute = minimize && before.substitute[me];
                if substitute {
                    self.animate(Routine::HideSubstituteShowMon { substitute_up: true, minimized: before.minimized[me] != 0 }, side);
                }
                self.play_current_move_animation(side);
                if substitute {
                    self.animate(Routine::ReshowSubstitute, side);
                }
            }
            MonsStatsFellText if move_effect < ATTACK_DOWN_SIDE_EFFECT => self.play_current_move_animation2(side),
            BecameConfusedText if move_effect != CONFUSION_SIDE_EFFECT => self.play_current_move_animation2(side),
            MoveWasDisabledText => self.play_current_move_animation2(side),
            ConvertedTypeText | StatusChangesEliminatedText | ShroudedInMistText | GettingPumpedText | MimicLearnedMoveText
            | WasSeededText | LightScreenProtectedText | ReflectGainedArmorText => self.play_current_move_animation(side),
            NoEffectText if move_effect == SPLASH_EFFECT => self.play_current_move_animation(side),
            ChargeMoveEffectText => {
                let battle = self.b();
                let id = match (battle.side(side).current_move, side) {
                    (current, _) if current.animation == PokemonMoveName::Dig as u8 => anim::SLIDE_DOWN_ANIM,
                    (current, _) if current.effect == FLY_EFFECT => anim::TELEPORT,
                    (_, Side::Player) => anim::XSTATITEM_ANIM,
                    (_, Side::Enemy) => anim::XSTATITEM_DUPLICATE_ANIM,
                };
                self.play_battle_animation(id, side);
            }
            TransformedText => {
                if before.substitute[me] {
                    self.animate(Routine::HideSubstituteShowMon { substitute_up: true, minimized: before.minimized[me] != 0 }, side);
                }
                self.animation_type = animation_type::NONE;
                let id = self.b().side(side).current_move.animation;
                self.whose_turn = side;
                let mut battle = self.anim_battle();
                battle.player_species = before.species[0];
                battle.enemy_species = before.species[1];
                // `TransformEffect_` sets `TRANSFORMED` after the animation, so `ChangeMonPic`'s
                // palette is still the mon's own.
                battle.mons = self.mon_palettes_as(before.transformed);
                let routine = Routine::TransformEffect { id };
                self.push(Present::Animation { routine, turn: side, battle });
                if before.substitute[me] {
                    self.animate(Routine::ReshowSubstitute, side);
                }
            }
            _ => {}
        }
    }

    /// The animations an effect plays with no text: Bide's and Thrash's.
    pub(super) fn silent_effect_animation(&mut self, side: Side, move_effect: u8) {
        let turn = match side { Side::Player => 0, Side::Enemy => 1 };
        match move_effect {
            effect::BIDE_EFFECT => self.play_battle_animation2(anim::XSTATITEM_ANIM + turn, side),
            effect::THRASH_PETAL_DANCE_EFFECT => self.play_battle_animation2(anim::SHRINKING_SQUARE_ANIM + turn, side),
            _ => {}
        }
    }
}

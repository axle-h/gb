//! What the battle shows between its steps, each taking the frames the cartridge takes.

use poke_core::text_script::{TextBuffer, TextCommand, TextNumber};
use serde::{Deserialize, Serialize};
use crate::audio::data::{Sound, SoundId};
use crate::gfx::sgb::{determine_palette_id, PaletteCommand};
use crate::gfx::text_boxes::TextBoxId;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, Transition};
use crate::modes::text_box::TextBox;
use crate::systems::battle::{BattleMon, Side, Status3};
use crate::systems::hp_bar::HpBarColour;
use super::animation::{AnimBattle, Animation, Routine};
use super::hud::{self, Bar, HpBar};
use super::transition::{self, BattleTransition, Choice, Silhouettes};
use super::BattleMode;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Present {
    /// `PrintText`, already spliced for `<USER>` and `<TARGET>`.
    Text(Vec<TextCommand>),
    /// `PrintText` of a text reading buffers the step filled, which are set as it prints since a
    /// later step may fill them again first.
    TextWith { commands: Vec<TextCommand>, strings: Vec<(TextBuffer, Vec<u8>)>, numbers: Vec<(TextNumber, u32)> },
    /// `DrawAllPokeballs`: the player's HUD and party balls, and a trainer's.
    DrawAllPokeballs,
    /// `DrawEnemyPokeballs`.
    DrawEnemyPokeballs,
    /// `ClearSprites`.
    ClearSprites,
    /// `LoadHudAndHpBarAndStatusTilePatterns`.
    LoadHudAndHpBarTiles,
    /// `LoadMonBackPic`'s decompressing and scaling, and `LoadPlayerBackPic`'s.
    LoadBackPic(poke_core::species::PokemonSpecies),
    LoadPlayerBackPic { old_man: bool },
    /// A tile set by hand, as the old man's demo draws its cursors.
    Tile { x: usize, y: usize, tile: u8 },
    /// `LoadMonFrontSprite` and `_LoadTrainerPic`, into `vFrontPic`.
    LoadFrontPic(poke_core::species::PokemonSpecies),
    LoadTrainerPic(u8),
    /// `LoadMonFrontSprite` for `MON_GHOST`.
    LoadGhostPic,
    /// `PrintStatsBox` for the party slot's new stats, in the level-up box.
    StatsBox(u8),
    /// `LearnMoveFromLevelUp`'s copy of the moves and PP into the battle mon, when it is the one
    /// that learnt.
    SyncLearnedMove(u8),
    /// `DelayFrames`.
    Frames(u8),
    /// `PlayCry`, which waits for the cry.
    Cry(u8),
    /// `PlaySound`.
    Sound(SoundId),
    /// `PlaySoundWaitForCurrent`: the sound playing finishes, then this one starts.
    SoundAfterCurrent(SoundId),
    /// `WaitForSoundToFinish`.
    WaitForSound,
    Music(Sound),
    /// `UpdateHPBar2` from `old` to `new`, of the side's max HP.
    HpBar { side: Side, old: u16, new: u16 },
    /// The HUDs of the mon as the step left it, since a later step may change it first, each
    /// ending on `GetBattleHealthBarColor`.
    DrawPlayerHud { mon: Box<BattleMon>, nick: Vec<u8>, mons: MonPalettes },
    DrawEnemyHud { mon: Box<BattleMon>, nick: Vec<u8>, mons: MonPalettes },
    /// `GetBattleHealthBarColor` of a colour worked out by hand, as `ReplaceFaintedEnemyMon` does.
    HealthBarColour { side: Side, colour: HpBarColour, mons: MonPalettes },
    /// `RunPaletteCommand` of `SET_PAL_BATTLE_BLACK` and of `SET_PAL_BATTLE`.
    SetPalBattleBlack,
    SetPalBattle(MonPalettes),
    Clear { x: usize, y: usize, width: usize, height: usize },
    /// `FillMemory` of blanks over `count` tiles from `(x, y)`, running on past each row's end.
    ClearRun { x: usize, y: usize, count: usize },
    /// `Music_PokeFluteInBattle` over `PlaySoundWaitForCurrent`, and the wait for channel 7 to end.
    PokeFluteInBattle,
    /// The Safari Zone menu's `PrintNumber` of the balls left, at (7, 14).
    SafariBallCount(u8),
    /// `wFrequencyModifier` and `wTempoModifier`, as written before a sound.
    Modifiers { frequency: u8, tempo: u8 },
    /// `EndLowHealthAlarm`'s writes.
    EndLowHealthAlarm,
    /// `RemoveFaintedPlayerMon`: an alarm sounding is asked to stop, and its tone waited out.
    DisableLowHealthAlarm,
    /// `DisplayTextBoxID`.
    TextBox(BattleBox),
    SaveScreen1,
    LoadScreen1,
    SaveScreen2,
    LoadScreen2,
    /// `WaitForTextScrollButtonPress`: a press, with no `▼`.
    WaitButton,
    /// A mode of its own, whose outcome the next step reads.
    Push(Box<Mode>),
    /// `BattleTransition`, chosen from what the step read.
    Transition(Choice),
    /// `SlidePlayerAndEnemySilhouettesOnScreen`'s slide.
    Silhouettes,
    /// An animation routine on `turn`'s side, with the battle as the step left it.
    Animation { routine: Routine, turn: Side, battle: AnimBattle },
    /// `SlideDownFaintedMonPic`: the picture at `(x, y)` slides down a row every two frames.
    SlideDown { x: usize, y: usize, row: u8 },
    /// `SlideTrainerPicOffScreen` for the player: nine columns left, two frames a column.
    SlideTrainerOff { column: u8 },
    /// `SlideTrainerPicOffScreen` for the enemy's trainer: eight columns right from `(18, 0)`.
    SlideEnemyTrainerOff { column: u8 },
    /// `_ScrollTrainerPicAfterBattle`: the trainer's pic in from the right a column every four
    /// frames, `columns` of it showing.
    ScrollTrainerIn { columns: u8 },
    /// `AnimateSendingOutMon` with the ball at `at`, a stage at a time: `ball` is `$4C` plus
    /// `wIsInBattle`, and `base` is `hStartTileID`, which the picture ends up `$31` past.
    SendingOut { stage: u8, ball: u8, at: (usize, usize), base: u8 },
    /// `AnimateRetreatingPlayerMon`, a stage at a time.
    Retreating { stage: u8 },
    /// Tiles placed directly, as `CopyUncompressedPicToHL` and a single `ld [hl]` do.
    Pic { x: usize, y: usize, first: u8 },
    /// The battle's own menu, polled from the frame it is called.
    Menu,
}

/// `wPlayerHPBarColor` and `wEnemyHPBarColor`, which `InitBattleVariables` zeroes to green.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HpBarColours {
    pub player: HpBarColour,
    pub enemy: HpBarColour,
}

impl Default for HpBarColours {
    fn default() -> Self {
        Self { player: HpBarColour::Green, enemy: HpBarColour::Green }
    }
}

/// The two `DeterminePaletteID`s `SetPal_Battle` works out, as the step that asked left the mons.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonPalettes {
    pub player: u8,
    pub enemy: u8,
}

/// `SetPal_Battle`'s packet: both bars and both mons.
pub fn set_pal_battle(colours: HpBarColours, mons: MonPalettes) -> PaletteCommand {
    PaletteCommand::Battle { player_hp_bar: colours.player, enemy_hp_bar: colours.enemy, player: mons.player, enemy: mons.enemy }
}

/// The text boxes a battle draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattleBox {
    MessageBox,
    BattleMenu,
    SwitchStatsCancel,
    SafariBattleMenu,
}

impl BattleBox {
    fn id(self) -> TextBoxId {
        match self {
            BattleBox::MessageBox => TextBoxId::MessageBox,
            BattleBox::BattleMenu => TextBoxId::BattleMenu,
            BattleBox::SwitchStatsCancel => TextBoxId::SwitchStatsCancel,
            BattleBox::SafariBattleMenu => TextBoxId::SafariBattleMenu,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Waiting {
    Nothing,
    Frames(u8),
    Sound,
    /// A sound to start once the one playing has finished.
    SoundAfter(SoundId),
    HpBar(HpBar),
    Child,
    Menu,
    /// `Music_PokeFluteInBattle`: the sound playing ends, `SFX_CAUGHT_MON` starts with its channels
    /// pointed at the flute's, and the battle waits for channel 7 to go quiet.
    PokeFlute { started: bool },
    /// `WaitForTextScrollButtonPress` where no `▼` is left to blink.
    Button { polled: bool },
    Animation(Box<Animation>),
    Transition(Box<BattleTransition>),
    Silhouettes(Silhouettes),
}

impl BattleMode {
    /// `SetPal_Battle`'s mon palettes now: a transformed mon is grey, whatever it looks like.
    pub(super) fn mon_palettes(&self) -> MonPalettes {
        let battle = self.b();
        let transformed = |side: Side| battle.side(side).status3.contains(Status3::TRANSFORMED);
        self.mon_palettes_as([transformed(Side::Player), transformed(Side::Enemy)])
    }

    /// `SetPal_Battle`'s mon palettes with each side's `TRANSFORMED` bit given.
    pub(super) fn mon_palettes_as(&self, transformed: [bool; 2]) -> MonPalettes {
        MonPalettes {
            player: determine_palette_id(transformed[0], self.pal_species[0]),
            enemy: determine_palette_id(transformed[1], self.pal_species[1]),
        }
    }

    pub(super) fn push_set_pal_battle(&mut self) {
        let mons = self.mon_palettes();
        self.queue.push_back(Present::SetPalBattle(mons));
    }

    /// `GetBattleHealthBarColor`: the palette sent again only when the bar changed colour.
    fn battle_health_bar_colour(&mut self, side: Side, colour: HpBarColour, mons: MonPalettes, ctx: &mut Ctx) {
        let stored = match side {
            Side::Player => &mut self.hp_bar_colours.player,
            Side::Enemy => &mut self.hp_bar_colours.enemy,
        };
        if *stored != colour {
            *stored = colour;
            ctx.screen.sgb.run(&set_pal_battle(self.hp_bar_colours, mons));
        }
    }

    fn draw_enemy_pokeballs(&mut self, ctx: &mut Ctx) {
        hud::place_enemy_hud_tiles(&mut ctx.screen.ui);
        let battle = self.battle.as_ref().expect("a battle");
        let party: Vec<(u16, u8)> = battle.enemy_party.iter().map(|mon| (mon.mon.hp, mon.mon.status)).collect();
        hud::place_pokeballs(&mut ctx.screen.sprites, 6, (0x48, 0x20), -8, &party);
    }
}

/// `CopyTileIDs`: a `size` square of the table's tile ids, each `base` past.
fn copy_tile_ids(ui: &mut UiSurface, x: usize, y: usize, size: usize, base: u8, table: &[u8]) {
    for row in 0..size {
        for column in 0..size {
            ui.set(x + column, y + row, table[row * size + column].wrapping_add(base));
        }
    }
}

impl BattleMode {
    /// One presentation. `Some` when it waits a frame or hands over to a child mode.
    pub(super) fn present(&mut self, present: Present, ctx: &mut Ctx) -> Option<Transition> {
        let ui = &mut ctx.screen.ui;
        match present {
            Present::Text(commands) => {
                self.waiting = Waiting::Child;
                return Some(Transition::Push(Mode::TextBox(TextBox::script(commands))));
            }
            Present::TextWith { commands, strings, numbers } => {
                ctx.world.text.strings.extend(strings);
                ctx.world.text.numbers.extend(numbers);
                self.waiting = Waiting::Child;
                return Some(Transition::Push(Mode::TextBox(TextBox::script(commands))));
            }
            Present::DrawAllPokeballs => {
                hud::load_pokeball_gfx(&mut ctx.screen.tiles);
                hud::place_player_hud_tiles(ui);
                let party: Vec<(u16, u8)> = ctx.world.party.iter().map(|named| (named.mon.mon.hp, named.mon.mon.status)).collect();
                hud::place_pokeballs(&mut ctx.screen.sprites, 0, (0x60, 0x60), 8, &party);
                if self.battle.as_ref().is_some_and(|battle| battle.kind == crate::systems::battle::BattleKind::Trainer) {
                    self.draw_enemy_pokeballs(ctx);
                }
            }
            Present::DrawEnemyPokeballs => {
                hud::load_pokeball_gfx(&mut ctx.screen.tiles);
                self.draw_enemy_pokeballs(ctx);
            }
            Present::ClearSprites => crate::gfx::mon_icons::clear_sprites(&mut ctx.screen.sprites),
            Present::LoadHudAndHpBarTiles => {
                ctx.screen.tiles.load_hp_bar_and_status_tiles();
                hud::load_hud_tiles(&mut ctx.screen.tiles);
            }
            Present::LoadBackPic(species) => hud::load_back_pic(&mut ctx.screen.tiles, species),
            Present::LoadPlayerBackPic { old_man } => hud::load_player_back_pic(&mut ctx.screen.tiles, old_man),
            Present::Tile { x, y, tile } => ui.set(x, y, tile),
            Present::LoadFrontPic(species) => hud::load_front_pic(&mut ctx.screen.tiles, species),
            Present::LoadTrainerPic(class) => hud::load_trainer_pic(&mut ctx.screen.tiles, class),
            Present::LoadGhostPic => hud::load_ghost_pic(&mut ctx.screen.tiles),
            Present::StatsBox(slot) => {
                let stats = ctx.world.party[slot as usize].mon.stats;
                crate::systems::status_screen::print_stats_box(ui, crate::systems::status_screen::StatsBox::LevelUp, stats);
            }
            Present::SyncLearnedMove(slot) => {
                let battle = self.battle.as_mut().expect("a battle");
                if slot == battle.player_mon_number {
                    let mon = &ctx.world.party[slot as usize].mon.mon;
                    battle.player.mon.moves = mon.moves;
                    battle.player.mon.pp = mon.pp;
                }
            }
            Present::Frames(0) => {}
            Present::Frames(frames) => {
                if ctx.pacing != crate::Pacing::Instant {
                    self.waiting = Waiting::Frames(frames);
                    return Some(Transition::Stay);
                }
            }
            Present::Cry(species) => {
                ctx.audio.play_cry(species);
                self.waiting = Waiting::Sound;
            }
            Present::Sound(id) => ctx.audio.play_sound(id),
            Present::SoundAfterCurrent(id) => self.waiting = Waiting::SoundAfter(id),
            Present::WaitForSound => self.waiting = Waiting::Sound,
            Present::Music(sound) => ctx.audio.play_music(sound),
            Present::HpBar { side, old, new } => {
                let battle = self.battle.as_ref().expect("a battle");
                let bar = match side { Side::Player => Bar::Player, Side::Enemy => Bar::Enemy };
                if let Some(bar) = HpBar::new(bar, battle.side(side).mon.stats[0], old, new) {
                    self.waiting = Waiting::HpBar(bar);
                }
            }
            Present::DrawPlayerHud { mon, nick, mons } => {
                let colour = hud::draw_player_hud(ui, &mon, &nick);
                self.battle_health_bar_colour(Side::Player, colour, mons, ctx);
                self.update_low_health_alarm(ctx, colour, mon.hp == 0);
            }
            Present::DrawEnemyHud { mon, nick, mons } => {
                let colour = hud::draw_enemy_hud(ui, &mon, &nick);
                self.battle_health_bar_colour(Side::Enemy, colour, mons, ctx);
            }
            Present::HealthBarColour { side, colour, mons } => self.battle_health_bar_colour(side, colour, mons, ctx),
            Present::SetPalBattleBlack => ctx.screen.sgb.run(&PaletteCommand::BattleBlack),
            Present::SetPalBattle(mons) => ctx.screen.sgb.run(&set_pal_battle(self.hp_bar_colours, mons)),
            Present::Clear { x, y, width, height } => hud::clear_area(ui, x, y, width, height),
            Present::ClearRun { x, y, count } => {
                for at in y * SCREEN_TILES_X + x..y * SCREEN_TILES_X + x + count {
                    ui.set(at % SCREEN_TILES_X, at / SCREEN_TILES_X, UiSurface::BLANK);
                }
            }
            Present::TextBox(id) => id.id().draw(ui),
            Present::PokeFluteInBattle => self.waiting = Waiting::PokeFlute { started: false },
            Present::SafariBallCount(count) => {
                let format = crate::systems::print_num::NumberFormat { digits: 2, leading_zeroes: false, left_align: false };
                crate::systems::print_num::print_number(ui, 7 + 14 * SCREEN_TILES_X, count as u32, format);
            }
            Present::EndLowHealthAlarm => ctx.audio.end_low_health_alarm(),
            Present::Modifiers { frequency, tempo } => ctx.audio.set_modifiers(frequency, tempo),
            Present::DisableLowHealthAlarm => {
                if ctx.audio.low_health_alarm_on() {
                    ctx.audio.set_low_health_alarm(false);
                    self.waiting = Waiting::Sound;
                }
            }
            Present::SaveScreen1 => self.buffer1 = Some(ui.clone()),
            Present::LoadScreen1 => *ui = self.buffer1.clone().expect("a saved screen"),
            Present::SaveScreen2 => self.buffer2 = Some(ui.clone()),
            Present::LoadScreen2 => *ui = self.buffer2.clone().expect("a saved screen"),
            Present::WaitButton => self.waiting = Waiting::Button { polled: false },
            Present::Push(mode) => {
                self.waiting = Waiting::Child;
                return Some(Transition::Push(*mode));
            }
            Present::Transition(choice) => {
                if !self.skip_move_animations {
                    let transition = BattleTransition::new(choice, self.trainer_oam_block, ctx.screen);
                    self.waiting = Waiting::Transition(Box::new(transition));
                }
            }
            Present::Silhouettes => self.waiting = Waiting::Silhouettes(Silhouettes::new(ctx.screen)),
            Present::Animation { routine, turn, battle } => {
                let animated = matches!(routine, Routine::SubstituteEffect { .. } | Routine::TransformEffect { .. })
                    && ctx.world.options.battle_animation;
                let skippable = matches!(routine, Routine::MoveAnimation { .. } | Routine::PlayMoveAnimation { .. });
                if self.skip_move_animations && (skippable || animated) {
                    return None;
                }
                let animations_on = ctx.world.options.battle_animation;
                let battle = AnimBattle { animations_on, hp_bar_colours: self.hp_bar_colours, ..battle };
                self.waiting = Waiting::Animation(Box::new(Animation::new(routine, turn, battle)));
            }
            Present::SlideDown { x, y, row } => {
                // Each step copies the picture's rows down one, top row first blanked.
                for r in (1..7).rev() {
                    for column in 0..7 {
                        let tile = ui.get(x + column, y + r - 1);
                        ui.set(x + column, y + r, tile);
                    }
                }
                ui.fill(x, y, 7, 1, UiSurface::BLANK);
                if row + 1 < 7 {
                    self.queue.push_front(Present::SlideDown { x, y, row: row + 1 });
                }
                self.waiting = Waiting::Frames(2);
                return Some(Transition::Stay);
            }
            Present::SlideTrainerOff { column } => {
                // `.columnLoop` from (1, 5): each of the nine tiles moves one left.
                for r in 0..7 {
                    for c in 0..9 {
                        let tile = ui.get(c + 1, 5 + r);
                        ui.set(c, 5 + r, tile);
                    }
                }
                if column + 1 < 9 {
                    self.queue.push_front(Present::SlideTrainerOff { column: column + 1 });
                }
                self.waiting = Waiting::Frames(2);
                return Some(Transition::Stay);
            }
            Present::SlideEnemyTrainerOff { column } => {
                for r in 0..7 {
                    for x in (11..19).rev() {
                        let tile = ui.get(x, r);
                        ui.set(x + 1, r, tile);
                    }
                }
                if column + 1 < 8 {
                    self.queue.push_front(Present::SlideEnemyTrainerOff { column: column + 1 });
                }
                self.waiting = Waiting::Frames(2);
                return Some(Transition::Stay);
            }
            Present::ScrollTrainerIn { columns } => {
                for column in 0..columns as usize {
                    for row in 0..7 {
                        ui.set(20 - columns as usize + column, row, (column * 7 + row) as u8);
                    }
                }
                if columns + 1 < 7 {
                    self.queue.push_front(Present::ScrollTrainerIn { columns: columns + 1 });
                }
                self.waiting = Waiting::Frames(4);
                return Some(Transition::Stay);
            }
            Present::SendingOut { stage, ball, at, base } => return self.sending_out(stage, ball, at, base, ctx),
            Present::Retreating { stage } => return self.retreating(stage, ctx),
            Present::Pic { x, y, first } => hud::place_pic(ui, x, y, first),
            Present::Menu => {
                self.menu.as_mut().expect("a menu to call").call(ctx);
                self.waiting = Waiting::Menu;
            }
        }
        None
    }

    /// `AnimateSendingOutMon`: the ball, three frames; the 3x3 up and left of it, four; the 5x5 up and
    /// left again, five; then the whole picture.
    fn sending_out(&mut self, stage: u8, ball: u8, at: (usize, usize), base: u8, ctx: &mut Ctx) -> Option<Transition> {
        use poke_core::rom_gfx::rom_slice;
        use poke_core::symbols::pokered_symbols;
        let ui = &mut ctx.screen.ui;
        let (x, y) = at;
        let (frames, next) = match stage {
            0 => {
                ui.set(x, y, ball);
                (3, Some(1))
            }
            1 => {
                copy_tile_ids(ui, x - 1, y - 2, 3, base, rom_slice(pokered_symbols::DownscaledMonTiles_3x3));
                (4, Some(2))
            }
            2 => {
                copy_tile_ids(ui, x - 2, y - 4, 5, base, rom_slice(pokered_symbols::DownscaledMonTiles_5x5));
                (5, Some(3))
            }
            _ => {
                hud::place_pic(ui, x - 3, y - 6, base.wrapping_add(hud::BACK_PIC_TILE));
                (0, None)
            }
        };
        if let Some(stage) = next {
            self.queue.push_front(Present::SendingOut { stage, ball, at, base });
        }
        if frames > 0 {
            self.waiting = Waiting::Frames(frames);
            return Some(Transition::Stay);
        }
        None
    }

    /// `AnimateRetreatingPlayerMon`: the 5x5, four frames; the 3x3, three; then a ball at `(5, 11)`
    /// and the picture's area cleared under it.
    fn retreating(&mut self, stage: u8, ctx: &mut Ctx) -> Option<Transition> {
        use poke_core::rom_gfx::rom_slice;
        use poke_core::symbols::pokered_symbols;
        let ui = &mut ctx.screen.ui;
        hud::clear_area(ui, 1, 5, 7, 7);
        let frames = match stage {
            0 => {
                copy_tile_ids(ui, 3, 7, 5, 0, rom_slice(pokered_symbols::DownscaledMonTiles_5x5));
                4
            }
            1 => {
                copy_tile_ids(ui, 4, 9, 3, 0, rom_slice(pokered_symbols::DownscaledMonTiles_3x3));
                3
            }
            _ => {
                ui.set(5, 11, 0x4C);
                hud::clear_area(ui, 1, 5, 7, 7);
                return None;
            }
        };
        self.queue.push_front(Present::Retreating { stage: stage + 1 });
        self.waiting = Waiting::Frames(frames);
        Some(Transition::Stay)
    }

    /// `DrawPlayerHUDAndHPBar`'s end: the low health alarm on at a red bar, off at none.
    fn update_low_health_alarm(&mut self, ctx: &mut Ctx, colour: crate::systems::hp_bar::HpBarColour, fainted: bool) {
        if !fainted && self.low_health_alarm_disabled {
            return;
        }
        if !fainted && colour == crate::systems::hp_bar::HpBarColour::Red {
            return ctx.audio.set_low_health_alarm(true);
        }
        if ctx.audio.low_health_alarm_on() {
            ctx.audio.end_low_health_alarm();
        }
    }

    /// Whatever the battle is waiting on, for one frame. `None` when nothing is, and the battle goes
    /// on in this frame.
    pub(super) fn wait(&mut self, ctx: &mut Ctx) -> Option<Transition> {
        match &mut self.waiting {
            Waiting::Nothing => None,
            Waiting::Frames(frames) => {
                *frames -= 1;
                if *frames > 0 {
                    return Some(Transition::Stay);
                }
                self.waiting = Waiting::Nothing;
                None
            }
            Waiting::Sound => {
                if ctx.pacing != crate::Pacing::Instant && !ctx.audio.sound_finished() {
                    return Some(Transition::Stay);
                }
                self.waiting = Waiting::Nothing;
                None
            }
            Waiting::SoundAfter(id) => {
                if ctx.pacing != crate::Pacing::Instant && !ctx.audio.sound_finished() {
                    return Some(Transition::Stay);
                }
                ctx.audio.play_sound(*id);
                self.waiting = Waiting::Nothing;
                None
            }
            Waiting::HpBar(bar) => {
                if !bar.update(&mut ctx.screen.ui) {
                    return Some(Transition::Stay);
                }
                self.waiting = Waiting::Nothing;
                None
            }
            Waiting::PokeFlute { started } => {
                // An alarm sounding skips the tune.
                if !*started && ctx.audio.low_health_alarm_on() {
                    self.waiting = Waiting::Nothing;
                    return None;
                }
                if !*started {
                    if ctx.pacing != crate::Pacing::Instant && !ctx.audio.sound_finished() {
                        return Some(Transition::Stay);
                    }
                    *started = true;
                    use poke_core::symbols::pokered_symbols as sym;
                    ctx.audio.play_sound(crate::audio::data::sounds::SFX_CAUGHT_MON);
                    for (channel, pointer) in [(4, sym::SFX_Pokeflute_Ch5), (5, sym::SFX_Pokeflute_Ch6), (6, sym::SFX_Pokeflute_Ch7)] {
                        ctx.audio.overwrite_channel_pointer(channel, pointer.address);
                    }
                }
                if ctx.pacing != crate::Pacing::Instant && ctx.audio.channel_sound_id(6) != 0 {
                    return Some(Transition::Stay);
                }
                self.waiting = Waiting::Nothing;
                None
            }
            Waiting::Animation(animation) => {
                if !animation.update(ctx) {
                    return Some(Transition::Stay);
                }
                if let Some(screen) = animation.screen1.take() {
                    self.buffer1 = Some(screen);
                }
                if let Some(screen) = animation.screen2.take() {
                    self.buffer2 = Some(screen);
                }
                if let Some(scx) = animation.h_scx.take() {
                    self.h_scx = scx;
                }
                self.waiting = Waiting::Nothing;
                None
            }
            Waiting::Transition(transition) => {
                if !transition.update(ctx) {
                    return Some(Transition::Stay);
                }
                self.waiting = Waiting::Nothing;
                None
            }
            Waiting::Silhouettes(silhouettes) => {
                if !silhouettes.update(ctx.screen) {
                    return Some(Transition::Stay);
                }
                self.h_scx = transition::SILHOUETTES_SCX;
                self.waiting = Waiting::Nothing;
                None
            }
            Waiting::Child => Some(Transition::Stay),
            Waiting::Menu => {
                let menu = self.menu.as_mut().expect("a menu to poll");
                match menu.update(ctx) {
                    None => Some(Transition::Stay),
                    Some(keys) => {
                        self.waiting = Waiting::Nothing;
                        self.menu_answered(keys, ctx);
                        None
                    }
                }
            }
            Waiting::Button { polled } => {
                *polled = true;
                if ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
                    self.answered += 1;
                    self.waiting = Waiting::Nothing;
                    return None;
                }
                Some(Transition::Stay)
            }
        }
    }
}

//! `PromptUserToPlaySlots` and `MainSlotMachineLoop`: the Game Corner's slot machine, from the
//! question through as many spins as the player pays for.
//!
//! The machine decides whether the player may win before a wheel turns (`SlotMachine_SetFlags`),
//! and the wheels are then stopped where that answer needs them: wheels 1 and 2 have four frames of
//! "slip" each in which they may refuse to stop, and wheel 3 rolls a symbol at a time until the
//! line the flags allow comes up or the reroll counter runs out. All of that is
//! [`crate::systems::slots`]; this is the screen, the pacing and the coins.
//!
//! The cabinet is a tilemap and a tile set of its own, and the three wheels are thirty-six objects
//! redrawn every step, so this mode owns the whole screen. Its loading is not modelled: the
//! white-outs either side, the LCD-off tile copies, `LoadScreenTilesFromBuffer1`'s `Delay3`. What
//! is kept is what a player watches: two frames a step while the wheels wind up, three once they
//! can be stopped, five between flashes of a win, and four or eight frames a coin as the payout
//! counts up.
//!
//! `BIT_NO_TEXT_DELAY` is set for the whole session, so every text here appears whole.

use poke_core::text_script::{far_text, TextBuffer, TextCommand};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::gfx::layers::Effects;
use crate::gfx::sgb::PaletteCommand;
use crate::gfx::text_boxes::TextBoxId;
use crate::gfx::tiles::{V_CHARS0, V_CHARS2};
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::blink::ArrowBlink;
use crate::modes::cursor_menu::CursorMenu;
use crate::modes::text_box::TextBox;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::systems::math::{add_bcd, sub_bcd};
use crate::systems::print_num::{print_bcd, print_number, BcdFormat, NumberFormat};
use crate::systems::slots::{self, Matches, Wheels, BALL_LIT, BALL_OUT, BAR};
use crate::Pacing;

/// `wTileMap`, which `TX_MOVE` names a tile of by address.
const TILE_MAP: u16 = 0xC3A0;
/// `hlcoord 5, 1` and `hlcoord 11, 1`: the credit and the payout.
const CREDIT_AT: (usize, usize) = (5, 1);
const PAYOUT_AT: (usize, usize) = (11, 1);
/// `hlcoord 14, 11` with `lb bc, 5, 4`, and the cursor `wTopMenuItemX`/`Y` inside it.
const BET_BOX: (usize, usize, usize, usize) = (14, 11, 4, 5);
const BET_CURSOR: (u8, u8) = (15, 12);
/// `hlcoord 16, 12`, where `CoinMultiplierSlotMachineText` starts.
const MULTIPLIERS_AT: (usize, usize) = (16, 12);
/// `hlcoord 14, 12` as `DisplayTextBoxID` reads it: the yes/no's cursor.
const AGAIN_CURSOR: (usize, usize) = (15, 13);
/// `hlcoord 2, 14`, the winning symbol in the text box, and `hlcoord 18, 16`, the `▼` drawn with it.
const SYMBOL_AT: (usize, usize) = (2, 14);
const ARROW_AT: (usize, usize) = (18, 16);
/// `WaitForTextScrollButtonPress`'s iterations a frame, in hundredths.
const BLINK_PER_FRAME: u32 = 4454;
/// Where `vChars2` holds a second copy of the symbols, for the one drawn in the text box.
const SYMBOL_TILES: u8 = 0x25;

/// `SlotMachine_SpinWheels`' `.loop1`, twenty turns of `DelayFrames 2`, and `.loop2`'s `DelayFrame`
/// plus `DelayFrames 2` on a Game Boy that is not an SGB.
const WIND_TURNS: u8 = 20;
const WIND_FRAMES: u8 = 2;
const SPIN_FRAMES: u8 = 3;
/// `.flashScreenLoop`'s `DelayFrames 5`.
const FLASH_FRAMES: u8 = 5;
/// `SlotMachine_PayCoinsToPlayer`: eight frames a coin, halved for a seven or a bar, and the
/// symbols flash every five coins.
const COIN_FRAMES: u8 = 8;
const COIN_FLASH: u8 = 5;
/// `OutOfCoinsSlotMachineText`'s `DelayFrames 60`.
const OUT_OF_COINS_FRAMES: u8 = 60;
/// `rBGP` and `rOBP0` inverted, which is how both flashes are done.
const FLASH: u8 = 0x40;
/// `ld a, $e4; ldh [rOBP0], a`: the symbols are drawn from the background's own shades.
const SLOTS_OBP0: u8 = 0xE4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotMachine {
    /// `wSlotMachineSevenAndBarModeChance`, which is lower on the machine the Game Corner picked.
    chance: u8,
    /// `wSlotMachineFlags`, which outlives a spin.
    flags: u8,
    /// `wSlotMachineAllowMatchesCounter`, zeroed when the session opens and closes.
    allow_matches: u8,
    /// `wSlotMachineRerollCounter`.
    reroll: u8,
    /// `wSlotMachineBet`, one to three.
    bet: u8,
    /// `wPayoutCoins`.
    payout: u16,
    wheels: Wheels,
    /// `wStoppingWhichSlotMachineWheel`.
    stopping: u8,
    /// `wSlotMachineWinningSymbol`, as the tile value the wheels compare.
    won: Option<u8>,
    /// `wTileMapBackup`: the screen as `SaveScreenTilesToBuffer1` left it, with the question up and
    /// no bet menu over it.
    saved: Option<UiSurface>,
    /// Presses that have stopped a wheel, so a driver can see its own land.
    answered: u32,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// A pushed text box or menu, and what to do when it pops.
    Child(After),
    /// `.loop1`: the wheels wind up on their own.
    Winding { turns: u8, wait: u8 },
    /// `.loop2`: each press of A stops the next wheel.
    Spinning { wait: u8 },
    /// `.rollWheel3DownByOneSymbol`: two frames, half a symbol each.
    Rolling { left: u8 },
    /// `.flashScreenLoop`.
    Flashing { left: u8, wait: u8 },
    /// `SlotMachine_PayCoinsToPlayer`, a coin at a time.
    Paying { wait: u8, flash: u8 },
    /// `OutOfCoinsSlotMachineText`'s hold before the machine gives up.
    OutOfCoins(u8),
    /// `WaitForSoundToFinish`, before the spin's sound and before the payout's.
    WaitingForSound(Sound),
    /// `WaitForTextScrollButtonPress` under the win's text: no sound, and the `▼` left as it is.
    WaitingForPress(ArrowBlink),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Sound {
    NewSpin,
    Payout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    /// `BetHowManySlotMachineText`, printed once before the bet menu opens.
    ToldToBet,
    /// The bet menu.
    Bet,
    /// `NotEnoughCoinsSlotMachineText`, which goes back to the bet menu.
    Refused,
    /// `StartSlotMachineText`, after which the wheels turn.
    Started,
    /// `YeahText`, which `SlotReward300Func` prints before it plays its sound, carrying the flashes
    /// the win is owed.
    Cheered(u8),
    /// `NotThisTimeText`, after which the machine asks again.
    Lost,
    /// `OutOfCoinsSlotMachineText`, which holds for a second and ends the session.
    Spent,
    /// `SymbolLinedUpSlotMachineText`, whose `▼` this mode drew itself.
    Announced,
    /// `OneMoreGoSlotMachineText`, then its yes/no.
    Offered,
    Again,
}

impl SlotMachine {
    /// `chance` is `wSlotMachineSevenAndBarModeChance`, which `StartSlotMachine` works out from
    /// which machine the Game Corner made lucky this visit.
    pub fn new(chance: u8) -> Self {
        Self {
            chance,
            flags: 0,
            allow_matches: 0,
            reroll: 0,
            bet: 0,
            payout: 0,
            wheels: Wheels::new(),
            stopping: 0,
            won: None,
            saved: None,
            answered: 0,
            phase: Phase::Child(After::ToldToBet),
        }
    }

    /// Presses that have stopped a wheel.
    pub fn answered(&self) -> u32 {
        self.answered
    }

    /// Whether the wheels are turning.
    pub fn spinning(&self) -> bool {
        matches!(self.phase, Phase::Winding { .. } | Phase::Spinning { .. } | Phase::Rolling { .. })
    }

    /// Whether the wheels are in `SlotMachine_SpinWheels`' `.loop2`, taking presses to stop them.
    pub fn stopping_wheels(&self) -> bool {
        matches!(self.phase, Phase::Spinning { .. })
    }

    /// Whether the next frame reads the pad, which this mode's `Busy` status does not show:
    /// `SlotMachine_HandleInputWhileWheelsSpin`, and the `WaitForTextScrollButtonPress` under a win.
    pub fn reading_the_pad(&self) -> bool {
        matches!(self.phase, Phase::Spinning { wait: 0 } | Phase::WaitingForPress(_))
    }

    /// `LoadSlotMachineTiles`: the cabinet's own tiles and tilemap, the symbols in both `vChars0`
    /// for the wheels and `vChars2` for the one a win prints, and the wheels stepped once each.
    fn load(&mut self, ctx: &mut Ctx) {
        let tiles = &mut ctx.screen.tiles;
        tiles.load(V_CHARS0, slots::symbol_tiles());
        tiles.load(V_CHARS2, slots::cabinet_tiles());
        tiles.load(V_CHARS2 + SYMBOL_TILES as usize, slots::symbol_tiles());
        tiles.load_font();
        let ui = &mut ctx.screen.ui;
        for (i, &tile) in slots::screen_map().iter().enumerate() {
            ui.set(i % SCREEN_TILES_X, i / SCREEN_TILES_X, tile);
        }
        self.wheels = Wheels::new();
        self.step_wheels(ctx, |wheels| (0..3).for_each(|wheel| wheels.anim(wheel)));
        // `PromptUserToPlaySlots`' `FillMemory` from `wStoppingWhichSlotMachineWheel` zeroes the
        // offsets under the wheels just drawn, so the first step draws from 0 rather than on.
        self.wheels.offsets = [0; 3];
        ctx.screen.sgb.run(&PaletteCommand::Slots);
        ctx.screen.effects.obp0 = SLOTS_OBP0;
    }

    /// `step` over the wheels, each wheel it moved redrawn. `SlotMachine_AnimWheel` draws a wheel at
    /// its offset and then advances it, so what shows is the offset before the step, and a wheel
    /// that stopped keeps the objects it had. The twelve objects a wheel owns are among the first
    /// thirty-six; the four above them are the overworld's, which `wUpdateSpritesEnabled` of `$ff`
    /// leaves exactly where they were.
    fn step_wheels<R>(&mut self, ctx: &mut Ctx, step: impl FnOnce(&mut Wheels) -> R) -> R {
        let before = self.wheels.offsets;
        let result = step(&mut self.wheels);
        const WHEEL_OBJECTS: usize = 12;
        if ctx.screen.sprites.len() < 3 * WHEEL_OBJECTS {
            ctx.screen.sprites.resize(3 * WHEEL_OBJECTS, Default::default());
        }
        for w in (0..3).filter(|&w| self.wheels.offsets[w] != before[w]) {
            let objects = slots::anim_wheel_objects(w, before[w]);
            ctx.screen.sprites[w * WHEEL_OBJECTS..(w + 1) * WHEEL_OBJECTS].copy_from_slice(&objects);
        }
        result
    }

    /// `SlotMachine_PrintCreditCoins` and `SlotMachine_PrintPayoutCoins`.
    fn print_coins(&self, ctx: &mut Ctx) {
        print_credit(ctx);
        let at = PAYOUT_AT.1 * SCREEN_TILES_X + PAYOUT_AT.0;
        print_number(&mut ctx.screen.ui, at, self.payout as u32, NumberFormat { digits: 4, leading_zeroes: true, left_align: false });
    }

    /// `SlotMachine_LightBalls` and `SlotMachine_PutOutLitBalls`.
    fn balls(&self, ctx: &mut Ctx, rows: &[usize], tile: u8) {
        for &row in rows {
            for (at, tile) in slots::ball_tiles(row, tile) {
                ctx.screen.ui.set(at % SCREEN_TILES_X, at / SCREEN_TILES_X, tile);
            }
        }
    }

    fn print(&mut self, after: After, label: &str) -> Transition {
        self.phase = Phase::Child(after);
        let script = far_text(label).expect("the slot machine's texts are in the cartridge");
        Transition::Push(Mode::TextBox(TextBox::script(script)))
    }

    /// `MainSlotMachineLoop`'s head: the coins, then the question.
    fn main_loop(&mut self, ctx: &mut Ctx) -> Transition {
        self.payout = 0;
        self.print_coins(ctx);
        self.print(After::ToldToBet, "_BetHowManySlotMachineText")
    }

    /// `.loop`: the bet menu over whatever is on the screen.
    fn bet_menu(&mut self, ctx: &mut Ctx) -> Transition {
        let ui = &mut ctx.screen.ui;
        ui.text_box_border(BET_BOX.0, BET_BOX.1, BET_BOX.2, BET_BOX.3);
        place_rom_string(ui, MULTIPLIERS_AT, sym::CoinMultiplierSlotMachineText);
        self.phase = Phase::Child(After::Bet);
        Transition::Push(Mode::CursorMenu(CursorMenu::new(0, 2, BET_CURSOR)))
    }

    /// `.skip1`: the bet is paid for, the balls light and the machine decides what it will allow.
    fn place_bet(&mut self, ctx: &mut Ctx) -> Transition {
        self.restore(ctx);
        // `SlotMachine_SubtractBetFromPlayerCoins` falls into `SlotMachine_PrintCreditCoins`.
        sub_bcd(&mut ctx.world.coins, &[0, self.bet]);
        print_credit(ctx);
        self.balls(ctx, slots::lit_ball_rows(self.bet), BALL_LIT);
        slots::set_flags(&mut *ctx.rng, &mut self.flags, &mut self.allow_matches, self.chance);
        self.wheels.bet();
        self.reroll = slots::SLIP;
        self.phase = Phase::WaitingForSound(Sound::NewSpin);
        Transition::Stay
    }

    /// `LoadScreenTilesFromBuffer1`, whose `Delay3` is loading.
    fn restore(&mut self, ctx: &mut Ctx) {
        if let Some(saved) = self.saved.clone() {
            ctx.screen.ui = saved;
        }
    }

    /// `SlotMachine_CheckForMatches`, run until it pays, gives up or asks for another roll.
    fn check_for_matches(&mut self, ctx: &mut Ctx) -> Transition {
        match slots::check_for_matches(self.wheels.offsets, self.bet, self.flags, &mut self.reroll) {
            Matches::Reroll => {
                self.phase = Phase::Rolling { left: 2 };
                Transition::Stay
            }
            Matches::Lost => {
                self.won = None;
                self.print(After::Lost, "_NotThisTimeText")
            }
            Matches::Won(symbol) => {
                self.won = Some(symbol);
                let reward = slots::slot_reward(&mut *ctx.rng, symbol, &mut self.flags, &mut self.allow_matches);
                self.payout = reward.coins;
                if symbol == slots::SEVEN {
                    // Three sevens are cheered before they are paid, and the sound follows the text.
                    let mut script = far_text("_YeahText").expect("the slot machine's texts are in the cartridge");
                    script.push(TextCommand::Pause);
                    self.phase = Phase::Child(After::Cheered(reward.flashes));
                    return Transition::Push(Mode::TextBox(TextBox::script(script)));
                }
                if symbol == slots::BAR {
                    ctx.audio.play_sound(sounds::SFX_GET_KEY_ITEM);
                }
                self.phase = Phase::Flashing { left: reward.flashes, wait: 0 };
                Transition::Stay
            }
        }
    }

    /// `SymbolLinedUpSlotMachineText`: the symbol drawn into the box at `hlcoord 2, 14`, a `▼` of
    /// its own, and the text printed four tiles past where the box would have started it.
    fn announce(&mut self, ctx: &mut Ctx) -> Transition {
        let symbol = self.won.expect("a win has a symbol");
        self.print_coins(ctx);
        let ui = &mut ctx.screen.ui;
        // `PrintText`'s box first, then the `text_asm` that draws the symbol over its top left.
        TextBoxId::MessageBox.draw(ui);
        let tile = symbol - 2 + SYMBOL_TILES;
        ui.set(SYMBOL_AT.0, SYMBOL_AT.1 - 1, tile + 2);
        ui.set(SYMBOL_AT.0 + 1, SYMBOL_AT.1 - 1, tile + 3);
        ui.set(SYMBOL_AT.0, SYMBOL_AT.1, tile);
        ui.set(SYMBOL_AT.0 + 1, SYMBOL_AT.1, tile + 1);
        ui.set(ARROW_AT.0, ARROW_AT.1, crate::modes::place_string::ch::DOWN_ARROW);
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, slots::reward_text(symbol));
        // `inc bc` four times: the text starts past the symbol rather than at the box's own corner.
        let mut script = vec![TextCommand::Move(TILE_MAP + 14 * SCREEN_TILES_X as u16 + 5)];
        // `_LinedUpText` ends in `<DONE>`, which ends the box: the wait is the caller's.
        script.extend(far_text("_LinedUpText").expect("the slot machine's texts are in the cartridge"));
        self.phase = Phase::Child(After::Announced);
        Transition::Push(Mode::TextBox(TextBox::without_box(script)))
    }

    /// One coin of `SlotMachine_PayCoinsToPlayer`.
    fn pay_a_coin(&mut self, ctx: &mut Ctx) -> Transition {
        self.payout -= 1;
        add_bcd(&mut ctx.world.coins, &[0, 1]);
        self.print_coins(ctx);
        ctx.audio.play_sound(sounds::SFX_SLOTS_REWARD);
        let Phase::Paying { wait, flash } = &mut self.phase else { unreachable!("paying") };
        *flash -= 1;
        if *flash == 0 {
            *flash = COIN_FLASH;
            ctx.screen.effects.obp0 ^= FLASH;
        }
        let frames = if self.won.is_some_and(|symbol| symbol <= BAR) { COIN_FRAMES / 2 } else { COIN_FRAMES };
        // The frame this runs in is the first of the `DelayFrames`.
        *wait = delay(ctx, frames).saturating_sub(1);
        Transition::Stay
    }

    /// The coins are counted out; what the machine says next.
    fn after_payout(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.screen.effects.obp0 = SLOTS_OBP0;
        if ctx.world.coins == [0, 0] {
            return self.print(After::Spent, "_OutOfCoinsSlotMachineText");
        }
        self.print(After::Offered, "_OneMoreGoSlotMachineText")
    }

    /// Whether A would stop a wheel now: the first press always counts, and the next is ignored
    /// while the wheel it would follow is still turning.
    fn press_accepted(&self) -> bool {
        match self.stopping {
            1 | 2 => self.wheels.slip[self.stopping as usize - 1] == 0,
            _ => true,
        }
    }

    /// The tail of `PromptUserToPlaySlots`. Putting the overworld back is the caller's:
    /// `ReloadMapSpriteTilePatterns` needs the sprites this mode cannot see.
    fn leave(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.world.no_text_delay = false;
        self.allow_matches = 0;
        let normal = Effects::default();
        (ctx.screen.effects.bgp, ctx.screen.effects.obp0) = (normal.bgp, normal.obp0);
        Transition::Pop(Outcome::Done)
    }
}

impl ModeUpdate for SlotMachine {
    /// `PromptUserToPlaySlots` from `LoadSlotMachineTiles` on: the question and its yes/no are the
    /// caller's, because the smile bubble between them is the overworld's.
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.world.no_text_delay = true;
        self.allow_matches = 0;
        self.load(ctx);
        self.main_loop(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match &mut self.phase {
            Phase::Child(_) => Transition::Stay,
            Phase::WaitingForSound(sound) => {
                if !ctx.audio.sound_finished() {
                    return Transition::Stay;
                }
                match sound {
                    Sound::NewSpin => {
                        ctx.audio.play_sound(sounds::SFX_SLOTS_NEW_SPIN);
                        self.print(After::Started, "_StartSlotMachineText")
                    }
                    Sound::Payout => {
                        self.phase = Phase::Paying { wait: 0, flash: COIN_FLASH };
                        Transition::Stay
                    }
                }
            }
            Phase::Winding { turns, wait } => {
                if *wait > 0 {
                    *wait -= 1;
                    return Transition::Stay;
                }
                if *turns == 0 {
                    self.phase = Phase::Spinning { wait: 0 };
                    return Transition::Stay;
                }
                *turns -= 1;
                // The frame this runs in is the first of the two the step waits.
                *wait = delay(ctx, WIND_FRAMES).saturating_sub(1);
                self.step_wheels(ctx, |wheels| (0..3).for_each(|wheel| wheels.anim(wheel)));
                Transition::Stay
            }
            Phase::Spinning { wait } => {
                if *wait > 0 {
                    *wait -= 1;
                    return Transition::Stay;
                }
                // `SlotMachine_HandleInputWhileWheelsSpin`: A stops the wheel whose turn it is, and
                // is ignored while the one before it is still turning.
                if ctx.pad.low_sensitivity(ctx.frame_counter).contains(Joypad::A) && self.press_accepted() {
                    self.stopping += 1;
                    self.answered += 1;
                    ctx.audio.play_sound(sounds::SFX_SLOTS_STOP_WHEEL);
                }
                let (stopping, flags) = (self.stopping, self.flags);
                let stopped = self.step_wheels(ctx, |wheels| wheels.stop_or_anim(stopping, flags));
                if stopped {
                    return self.check_for_matches(ctx);
                }
                let Phase::Spinning { wait } = &mut self.phase else { unreachable!("spinning") };
                *wait = delay(ctx, SPIN_FRAMES).saturating_sub(1);
                Transition::Stay
            }
            Phase::Rolling { left } => {
                *left -= 1;
                let last = *left == 0;
                self.step_wheels(ctx, |wheels| wheels.anim(2));
                if last { self.check_for_matches(ctx) } else { Transition::Stay }
            }
            Phase::Flashing { left, wait } => {
                if *wait > 0 {
                    *wait -= 1;
                    return Transition::Stay;
                }
                if *left == 0 {
                    return self.announce(ctx);
                }
                *left -= 1;
                *wait = delay(ctx, FLASH_FRAMES).saturating_sub(1);
                ctx.screen.effects.bgp ^= FLASH;
                Transition::Stay
            }
            Phase::Paying { wait, .. } => {
                if *wait > 0 {
                    *wait -= 1;
                    return Transition::Stay;
                }
                if self.payout == 0 {
                    return self.after_payout(ctx);
                }
                self.pay_a_coin(ctx)
            }
            Phase::WaitingForPress(blink) => {
                if ctx.pad.low_sensitivity(ctx.frame_counter).intersects(Joypad::A | Joypad::B) {
                    self.phase = Phase::WaitingForSound(Sound::Payout);
                    return self.update(ctx);
                }
                if let Some(shown) = blink.tick(BLINK_PER_FRAME) {
                    let tile = if shown { crate::modes::place_string::ch::DOWN_ARROW } else { UiSurface::BLANK };
                    ctx.screen.ui.set(ARROW_AT.0, ARROW_AT.1, tile);
                }
                Transition::Stay
            }
            Phase::OutOfCoins(frames) => {
                *frames += 1;
                if *frames < delay(ctx, OUT_OF_COINS_FRAMES) {
                    return Transition::Stay;
                }
                self.leave(ctx)
            }
        }
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        let Phase::Child(after) = self.phase else { return Transition::Stay };
        match after {
            After::ToldToBet => {
                self.saved = Some(ctx.screen.ui.clone());
                self.bet_menu(ctx)
            }
            After::Bet => match outcome {
                Outcome::Chosen(row) => {
                    self.bet = 3 - row.min(2);
                    let coins = ctx.world.coins;
                    if coins[0] == 0 && coins[1] < self.bet {
                        return self.print(After::Refused, "_NotEnoughCoinsSlotMachineText");
                    }
                    self.place_bet(ctx)
                }
                _ => {
                    self.restore(ctx);
                    self.leave(ctx)
                }
            },
            After::Refused => self.bet_menu(ctx),
            After::Started => {
                self.stopping = 0;
                self.phase = Phase::Winding { turns: WIND_TURNS, wait: 0 };
                Transition::Stay
            }
            After::Cheered(flashes) => {
                ctx.audio.play_sound(sounds::SFX_GET_ITEM_2);
                self.phase = Phase::Flashing { left: flashes, wait: 0 };
                Transition::Stay
            }
            After::Lost => self.after_payout(ctx),
            After::Spent => {
                self.phase = Phase::OutOfCoins(0);
                Transition::Stay
            }
            // `WaitForTextScrollButtonPress` reads the pad in the frame `PrintText` returns.
            After::Announced => {
                self.phase = Phase::WaitingForPress(ArrowBlink::default());
                self.update(ctx)
            }
            After::Offered => {
                self.phase = Phase::Child(After::Again);
                Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::at_cursor(TwoOptionMenuId::YesNo, AGAIN_CURSOR, false)))
            }
            After::Again => {
                if outcome != Outcome::Chosen(0) {
                    return self.leave(ctx);
                }
                self.balls(ctx, slots::lit_ball_rows(3), BALL_OUT);
                self.main_loop(ctx)
            }
        }
    }

    fn status(&self) -> Status {
        Status::Busy
    }
}

/// `SlotMachine_PrintCreditCoins`.
fn print_credit(ctx: &mut Ctx) {
    let at = CREDIT_AT.1 * SCREEN_TILES_X + CREDIT_AT.0;
    print_bcd(&mut ctx.screen.ui, at, &ctx.world.coins, BcdFormat { skip_leading_zeroes: false, left_align: false, money_sign: false });
}

/// A wait that is presentation, which `Instant` pacing skips.
fn delay(ctx: &Ctx, frames: u8) -> u8 {
    if ctx.pacing == Pacing::Instant { 0 } else { frames }
}

/// `PlaceString` of a string in the cartridge, `<NEXT>` two rows down at the column it started in.
fn place_rom_string(ui: &mut UiSurface, (x, y): (usize, usize), at: poke_core::symbols::DmgPointer) {
    const NEXT: u8 = 0x4E;
    const END: u8 = 0x50;
    let (mut column, mut row) = (x, y);
    for &byte in poke_core::rom_gfx::rom_slice(at) {
        match byte {
            END => return,
            NEXT => {
                column = x;
                row += 2;
            }
            _ => {
                if row < SCREEN_TILES_Y && column < SCREEN_TILES_X {
                    ui.set(column, row, byte);
                }
                column += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::audio::data::AudioBank;
    use crate::audio::engine::AudioEngine;
    use crate::command::Decision;
    use crate::gfx::Screen;
    use crate::input::Pad;
    use crate::mode::{ModeUpdate, Status, Transition};
    use crate::modes::menu_input::CursorMemory;
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Event, Pacing};
    use super::*;

    /// The machine with whatever it pushes on top of it, run a frame at a time. `Mode` has no slot
    /// machine in it, so `Game` cannot hold one and this is its stack.
    struct Cabinet {
        machine: SlotMachine,
        modes: Vec<Mode>,
        world: World,
        rng: GameRng,
        pad: Pad,
        frame_counter: u8,
        screen: Screen,
        menu: CursorMemory,
        audio: AudioEngine,
        left: bool,
    }

    impl Cabinet {
        fn new(coins: [u8; 2], chance: u8) -> Self {
            let mut cabinet = Self {
                machine: SlotMachine::new(chance),
                modes: Vec::new(),
                world: World { coins, ..World::default() },
                rng: GameRng::seeded(7),
                pad: Pad::default(),
                frame_counter: 0,
                screen: Screen::default(),
                menu: CursorMemory::default(),
                audio: AudioEngine::new(AudioBank::One),
                left: false,
            };
            cabinet.with_ctx(|machine, modes, ctx| {
                machine.enter(ctx);
                let transition = machine.open(ctx);
                apply(machine, modes, transition, ctx)
            });
            cabinet
        }

        fn with_ctx(&mut self, f: impl FnOnce(&mut SlotMachine, &mut Vec<Mode>, &mut Ctx) -> bool) {
            let Self { machine, modes, world, rng, pad, frame_counter, screen, menu, audio, .. } = self;
            let mut events: Vec<Event> = Vec::new();
            let mut ctx = Ctx { world, pad, rng, screen, menu, audio, frame_counter, events: &mut events,
                                pacing: Pacing::Instant, update_sprites: false, save_game: false, saved_player_id: None };
            self.left |= f(machine, modes, &mut ctx);
        }

        fn frame(&mut self, buttons: Joypad) {
            self.pad.input = buttons;
            self.audio.frame();
            self.frame_counter = self.frame_counter.saturating_sub(1);
            self.with_ctx(|machine, modes, ctx| {
                let transition = match modes.last_mut() {
                    Some(top) => top.update(ctx),
                    None => machine.update(ctx),
                };
                apply(machine, modes, transition, ctx)
            });
        }

        fn status(&self) -> Status {
            match self.modes.last() {
                Some(top) => top.status(),
                None => self.machine.status(),
            }
        }

        /// A press of A every other frame, which answers a text, takes the bet the cursor starts on
        /// and stops a wheel; `again` is the answer to `OneMoreGoSlotMachineText`.
        fn play(&mut self, again: Joypad) {
            for frame in 0..8000 {
                if self.left {
                    return;
                }
                let held = match self.status() {
                    Status::Waiting(Decision::TwoOption) => again,
                    _ if frame % 2 == 0 => Joypad::A,
                    _ => Joypad::empty(),
                };
                self.frame(held);
            }
            panic!("the machine never let go: {:?}", self.machine.phase);
        }

        fn credit(&self) -> [u8; 2] {
            self.world.coins
        }

        fn ball(&self, row: usize) -> u8 {
            self.screen.ui.row(row)[3]
        }
    }

    /// `Game::apply`, over a stack whose bottom is the machine itself: true once that has popped.
    fn apply(machine: &mut SlotMachine, modes: &mut Vec<Mode>, transition: Transition, ctx: &mut Ctx) -> bool {
        match transition {
            Transition::Stay => false,
            Transition::Push(mode) => opened(machine, modes, mode, ctx),
            Transition::Replace(mode) => {
                modes.pop();
                opened(machine, modes, mode, ctx)
            }
            Transition::Pop(outcome) => {
                if modes.pop().is_none() {
                    return true;
                }
                let transition = match modes.last_mut() {
                    Some(parent) => parent.resume(outcome, ctx),
                    None => machine.resume(outcome, ctx),
                };
                apply(machine, modes, transition, ctx)
            }
        }
    }

    fn opened(machine: &mut SlotMachine, modes: &mut Vec<Mode>, mut mode: Mode, ctx: &mut Ctx) -> bool {
        mode.enter(ctx);
        modes.push(mode);
        let transition = modes.last_mut().expect("just pushed").open(ctx);
        apply(machine, modes, transition, ctx)
    }

    /// The offsets that put a seven in the middle row of all three wheels.
    fn seven_line() -> [u8; 3] {
        std::array::from_fn(|w| {
            (1..30).step_by(2).find(|&o| slots::wheel(w)[o + 2] == slots::SEVEN).expect("a seven in reach") as u8
        })
    }

    #[test]
    fn a_bet_is_paid_for_before_the_wheels_turn_and_three_presses_stop_them() {
        let mut cabinet = Cabinet::new([0x10, 0x00], slots::NOT_LUCKY);
        cabinet.play(Joypad::B);
        assert_eq!(cabinet.machine.answered(), 3, "one press a wheel");
        assert!(cabinet.left, "No ends the session");
        let [high, low] = cabinet.credit();
        assert!(high == 0x09 && low >= 0x97, "the ×3 bet is paid for and only a win gives it back: {high:02X}{low:02X}");
    }

    #[test]
    fn a_bet_the_player_cannot_afford_goes_back_to_the_bet_menu() {
        let mut cabinet = Cabinet::new([0x00, 0x02], slots::NOT_LUCKY);
        for _ in 0..200 {
            if cabinet.status() == Status::Waiting(Decision::CursorMenu) {
                break;
            }
            cabinet.frame(Joypad::A);
            cabinet.frame(Joypad::empty());
        }
        // The cursor starts on ×3, which two coins cannot pay for.
        cabinet.frame(Joypad::A);
        for _ in 0..200 {
            if cabinet.status() == Status::Waiting(Decision::CursorMenu) {
                break;
            }
            cabinet.frame(Joypad::A);
            cabinet.frame(Joypad::empty());
        }
        assert_eq!(cabinet.status(), Status::Waiting(Decision::CursorMenu), "the bet is asked again");
        assert_eq!(cabinet.credit(), [0x00, 0x02], "a refused bet costs nothing");
        assert!(!cabinet.left);
    }

    #[test]
    fn the_bet_lights_its_balls_and_the_next_game_puts_them_out() {
        let mut cabinet = Cabinet::new([0x10, 0x00], slots::NOT_LUCKY);
        for _ in 0..400 {
            if matches!(cabinet.machine.phase, Phase::Winding { .. } | Phase::Spinning { .. }) {
                break;
            }
            cabinet.frame(Joypad::A);
            cabinet.frame(Joypad::empty());
        }
        assert_eq!(cabinet.machine.bet, 3);
        for row in slots::lit_ball_rows(3) {
            assert_eq!(cabinet.ball(*row), BALL_LIT, "row {row}");
        }
        // One more go: the wheels are stopped, the offer taken, and the next bet asked for.
        for _ in 0..4000 {
            if cabinet.status() == Status::Waiting(Decision::CursorMenu) {
                break;
            }
            cabinet.frame(Joypad::A);
            cabinet.frame(Joypad::empty());
        }
        assert_eq!(cabinet.status(), Status::Waiting(Decision::CursorMenu), "the bet is asked again");
        for row in slots::lit_ball_rows(3) {
            assert_ne!(cabinet.ball(*row), BALL_LIT, "row {row} is put out before the next bet");
        }
    }

    /// Three sevens with seven-and-bar mode on: the cheer, the flashes, the box and 300 coins paid
    /// out a coin at a time.
    #[test]
    fn three_sevens_are_cheered_before_they_are_paid() {
        let mut cabinet = Cabinet::new([0x10, 0x00], slots::LUCKY);
        for _ in 0..400 {
            if matches!(cabinet.machine.phase, Phase::Spinning { .. }) {
                break;
            }
            cabinet.frame(Joypad::A);
            cabinet.frame(Joypad::empty());
        }
        assert!(matches!(cabinet.machine.phase, Phase::Spinning { .. }), "the wheels turn");
        cabinet.machine.flags = slots::CAN_WIN_WITH_7_OR_BAR;
        cabinet.machine.wheels = Wheels { offsets: seven_line(), slip: [0, 0] };
        cabinet.machine.stopping = 3;
        cabinet.play(Joypad::B);
        assert_eq!(cabinet.machine.won, Some(slots::SEVEN));
        // 1000 coins, less the three the bet cost, plus the 300 the line paid.
        assert_eq!(cabinet.credit(), [0x12, 0x97]);
        assert_eq!(cabinet.machine.payout, 0, "every coin is counted out");
    }
}

//! `OpenOaksPC`: whether to have the Pokédex rated, then `DisplayDexRating` on a YES, and the link
//! closed either way over the screen from before.
//!
//! The rating counts seen and owned, prints the completion and the rating for the owned count, and
//! plays the rating's sound after whatever is playing ends, then waits for A or B with no `▼`. With
//! `EVENT_HALL_OF_FAME_DEX_RATING` set the rating is only worked out for the Hall of Fame, which
//! clears the flag; here that prints nothing.

use poke_core::symbols::pokered_events::EVENT_HALL_OF_FAME_DEX_RATING;
use poke_core::symbols::{pokered_symbols as sym, DmgPointer};
use poke_core::text_script::{TextCommand, TextNumber};
use serde::{Deserialize, Serialize};
use crate::audio::data::SoundId;
use crate::gfx::ui::UiSurface;
use crate::mode::{Ctx, Mode, Outcome, Transition};
use crate::modes::text_box::TextBox;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::systems::events::tables::{dex_rating_sound, dex_rating_text};
use crate::systems::pokedex::count_set_bits;
use super::print_at;

/// `hlcoord 14, 7`: where `YesNoChoice` asks.
const YES_NO_AT: (usize, usize) = (14, 7);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OaksPc {
    /// `wTileMapBackup2`, put back as the link closes.
    saved: UiSurface,
    /// `wTileMapBackup`, from under the yes/no.
    under_yes_no: UiSurface,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Accessed,
    Asked,
    Answered,
    Completion,
    Rated,
    /// `PlaySoundWaitForCurrent`'s wait, before the rating's sound.
    Sound,
    Heard,
    Closed,
}

impl OaksPc {
    /// `OpenOaksPC` up to its first text.
    pub fn start(ctx: &mut Ctx) -> (Self, Transition) {
        let saved = ctx.screen.ui.clone();
        let pc = Self { saved, under_yes_no: UiSurface::default(), phase: Phase::Accessed };
        (pc, print_at(sym::AccessedOaksPCText, ctx))
    }

    pub fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Sound => self.rating_sound(ctx),
            _ => Transition::Stay,
        }
    }

    /// A child has closed. `Pop` means the link is closed too.
    pub fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Accessed => self.text(sym::GetDexRatedText, Phase::Asked, ctx),
            Phase::Asked => {
                self.under_yes_no = ctx.screen.ui.clone();
                self.phase = Phase::Answered;
                Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, YES_NO_AT, false)))
            }
            Phase::Answered => {
                ctx.screen.ui = self.under_yes_no.clone();
                if outcome == Outcome::Chosen(0) { self.rate(ctx) } else { self.close(ctx) }
            }
            Phase::Completion => {
                self.phase = Phase::Rated;
                let owned = count_set_bits(&ctx.world.pokedex.owned);
                print_at(dex_rating_text(owned), ctx)
            }
            Phase::Rated => self.rating_sound(ctx),
            Phase::Sound => Transition::Stay,
            Phase::Heard => self.close(ctx),
            Phase::Closed => {
                // `LoadScreenTilesFromBuffer2`; its `Delay3` is loading.
                ctx.screen.ui = self.saved.clone();
                Transition::Pop(Outcome::Done)
            }
        }
    }

    fn text(&mut self, at: DmgPointer, then: Phase, ctx: &mut Ctx) -> Transition {
        self.phase = then;
        print_at(at, ctx)
    }

    /// `DisplayDexRating` up to its first text.
    fn rate(&mut self, ctx: &mut Ctx) -> Transition {
        let seen = count_set_bits(&ctx.world.pokedex.seen) as u32;
        let owned = count_set_bits(&ctx.world.pokedex.owned) as u32;
        let numbers = &mut ctx.world.text.numbers;
        // `hDexRatingNumMonsSeen` is `hOaksAideRequirement`'s byte, and a decoded text names it by
        // the first of them, so the seen count goes under both.
        numbers.insert(TextNumber::DexRatingNumMonsSeenH, seen);
        numbers.insert(TextNumber::OaksAideRequirement, seen);
        numbers.insert(TextNumber::DexRatingNumMonsOwnedH, owned);
        if ctx.world.events.is_set(EVENT_HALL_OF_FAME_DEX_RATING) {
            ctx.world.events.clear(EVENT_HALL_OF_FAME_DEX_RATING);
            let numbers = &mut ctx.world.text.numbers;
            numbers.insert(TextNumber::DexRatingNumMonsSeen, seen);
            numbers.insert(TextNumber::DexRatingNumMonsOwned, owned);
            return self.close(ctx);
        }
        self.phase = Phase::Completion;
        print_at(sym::DexCompletionText, ctx)
    }

    /// `PlayPokedexRatingSfx`, then `WaitForTextScrollButtonPress`.
    fn rating_sound(&mut self, ctx: &mut Ctx) -> Transition {
        if !ctx.audio.sound_finished() {
            self.phase = Phase::Sound;
            return Transition::Stay;
        }
        ctx.audio.play_new_sound(SoundId::STOP_ALL_MUSIC);
        ctx.audio.play_music(dex_rating_sound(count_set_bits(&ctx.world.pokedex.owned)));
        crate::modes::overworld::play_default_music_common(ctx, 0);
        self.phase = Phase::Heard;
        Transition::Push(Mode::TextBox(TextBox::without_box(vec![TextCommand::WaitButton])))
    }

    fn close(&mut self, ctx: &mut Ctx) -> Transition {
        // `ClosedOaksPCText` is the text and then `text_waitbutton`, which is the wait for a press.
        self.text(sym::ClosedOaksPCText, Phase::Closed, ctx)
    }
}

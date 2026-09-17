//! `DoInGameTradeDialogue`: the offer, yes or no, the party menu, and the mon that comes back with
//! another trainer's name on it. The trade animation (`InternalClockTradeAnim`) is the movie's to
//! recreate, and the trade goes through without it.

use poke_core::symbols::pokered_symbols as sym;
use poke_core::text_script::TextBuffer;
use crate::mode::{Mode, Outcome};
use crate::modes::party_menu::{PartyMenu, PartyMenuType};
use crate::party::Named;
use crate::rng::Rng;
use crate::systems::add_mon::{add_party_mon, new_party_mon, Origin};
use crate::systems::events::tables::{InGameTrade, TradeText};
use super::super::script::{Block, Flow, Script, Then};
use super::{after_yes_no, print, restore_screen_tiles_and_reload_tile_patterns, save_screen_tiles_to_buffer2, yes_no, Label};

/// `InGameTrade_TrainerString`: the received mon's OT, the `<TRAINER>` control character.
const TRAINER: u8 = 0x5D;

fn trade(s: &Script) -> InGameTrade {
    InGameTrade::of(s.ow.rt.events.trade.expect("a trade is under way"))
}

pub(super) fn in_game_trade(s: &mut Script, which: u8) -> Flow {
    save_screen_tiles_to_buffer2(s);
    s.ow.rt.events.trade = Some(which);
    let trade = trade(s);
    let strings = &mut s.ctx.world.text.strings;
    strings.insert(TextBuffer::InGameTradeGiveMonName, trade.give.name());
    strings.insert(TextBuffer::InGameTradeReceiveMonName, trade.receive.name());
    if s.ctx.world.in_game_trades & 1 << which != 0 {
        s.ow.rt.events.trade_text = Some(TradeText::AfterTrade);
        return Flow::Jump(Label::InGameTradeText.into());
    }
    print(trade.text(TradeText::WannaTrade)).then(Label::InGameTradeAsk)
}

pub(super) fn ask(s: &mut Script) -> Flow {
    s.ow.rt.events.trade_text = Some(TradeText::NoTrade);
    yes_no(s).then(Label::InGameTradeAnswered)
}

/// `InGameTrade_DoTrade` up to the party menu.
pub(super) fn answered(s: &mut Script) -> Flow {
    if !after_yes_no(s) {
        return Flow::Jump(Label::InGameTradeText.into());
    }
    s.ow.rt.sprites_frozen = true;
    Then::block(Block::Mode(Box::new(Mode::PartyMenu(PartyMenu::new(PartyMenuType::Normal))))).then(Label::InGameTradeChoseMon)
}

/// `InGameTrade_RestoreScreen`, whose white-out, `Delay3`s and `DelayFrames 10` are all waiting on
/// the tiles it reloads.
fn restore_screen(s: &mut Script) {
    restore_screen_tiles_and_reload_tile_patterns(s);
}

pub(super) fn chose_mon(s: &mut Script) -> Flow {
    restore_screen(s);
    let Some(Outcome::Chosen(slot)) = s.ow.rt.outcome else {
        s.ow.rt.events.trade_text = Some(TradeText::NoTrade);
        return Flow::Jump(Label::InGameTradeText.into());
    };
    let trade = trade(s);
    if s.ctx.world.party[slot as usize].mon.mon.species != trade.give {
        s.ow.rt.events.trade_text = Some(TradeText::WrongMon);
        return Flow::Jump(Label::InGameTradeText.into());
    }
    let which = s.ow.rt.events.trade.expect("a trade");
    s.ctx.world.in_game_trades |= 1 << which;
    print(sym::ConnectCableText).then(Label::InGameTradeConnected(slot))
}

/// `InGameTrade_PrepareTradeData`'s OT ID, then the mons exchanged: the one given goes, and the one
/// received is made at its level and put last, with the trade's nickname and `<TRAINER>` for an OT.
pub(super) fn connected(s: &mut Script, slot: u8) -> Flow {
    let trade = trade(s);
    let level = s.ctx.world.party[slot as usize].mon.level;
    let ot_id = u16::from_be_bytes([s.ctx.rng.random(), s.ctx.rng.random()]);
    s.ctx.world.party.remove(slot as usize);
    let world = &mut *s.ctx.world;
    let mut mon = new_party_mon(trade.receive, level, world.player_id, &Origin::Given, s.ctx.rng);
    mon.mon.ot_id = ot_id;
    add_party_mon(&mut world.party, Named { mon, ot: vec![TRAINER], nick: trade.nick.clone() }, Some(&mut world.pokedex));
    restore_screen(s);
    s.ow.rt.events.trade_text = Some(TradeText::Thanks);
    print(sym::TradedForText).then(Label::InGameTradeText)
}

/// `.printText`: whichever of the dialogue set's texts the trade got to.
pub(super) fn print_text(s: &mut Script) -> Flow {
    let text = s.ow.rt.events.trade_text.expect("a trade text");
    print(trade(s).text(text)).ret()
}

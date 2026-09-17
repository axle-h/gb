//! `DaycareGentlemanText`: a mon left with the man, or collected with the levels it has grown and the
//! ¥100 a level it costs.

use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::pokered_local_labels::DaycareGentlemanText as text;
use poke_core::symbols::pokered_symbols as sym;
use poke_core::text_script::{TextBuffer, TextMoney, TextNumber};
use crate::audio::data::sounds;
use crate::gfx::text_boxes::money_box;
use crate::mode::{Mode, Outcome};
use crate::modes::party_menu::{PartyMenu, PartyMenuType};
use crate::party::{Named, PARTY_LENGTH};
use crate::systems::add_mon::{deposit, withdraw};
use crate::systems::events::day_care::collection;
use crate::systems::evos_moves::{write_mon_moves, Learner};
use crate::systems::math::sub_bcd;
use crate::systems::money::has_enough;
use super::super::script::{Block, Flow, Script, Then};
use super::{after_yes_no, print, restore_screen_tiles_and_reload_tile_patterns, save_screen_tiles_to_buffer2, yes_no, Label};

fn done(text: poke_core::symbols::DmgPointer) -> Flow {
    Flow::Jump(Label::DayCareDone(text).into())
}

pub(super) fn day_care(s: &mut Script) -> Flow {
    save_screen_tiles_to_buffer2(s);
    if s.ctx.world.day_care.is_some() {
        return Flow::Jump(Label::DayCareInUse.into());
    }
    print(text::IntroText).then(Label::DayCareAsk)
}

pub(super) fn ask(s: &mut Script) -> Flow {
    yes_no(s).then(Label::DayCareAnswered)
}

pub(super) fn answered(s: &mut Script) -> Flow {
    if !after_yes_no(s) {
        return done(text::ComeAgainText);
    }
    // `dec a` on the party count: a party of none is not a party of one.
    if s.ctx.world.party.len() == 1 {
        return done(text::OnlyHaveOneMonText);
    }
    print(text::WhichMonText).then(Label::DayCareWhichMon)
}

pub(super) fn which_mon(_s: &mut Script) -> Flow {
    Then::block(Block::Mode(Box::new(Mode::PartyMenu(PartyMenu::new(PartyMenuType::Normal))))).then(Label::DayCareChoseMon)
}

/// `KnowsHMMove`.
fn knows_hm_move(s: &Script, slot: usize) -> bool {
    let hms: Vec<u8> = rom_slice(sym::HMMoveArray).iter().copied().take_while(|&m| m != 0xFF).collect();
    s.ctx.world.party[slot].mon.mon.moves.iter().any(|mv| mv.is_some_and(|mv| hms.contains(&(mv as u8))))
}

pub(super) fn chose_mon(s: &mut Script) -> Flow {
    restore_screen_tiles_and_reload_tile_patterns(s);
    let Some(Outcome::Chosen(slot)) = s.ow.rt.outcome else { return done(text::AllRightThenText) };
    if knows_hm_move(s, slot as usize) {
        return done(text::CantAcceptMonWithHMText);
    }
    s.ctx.menu.party_and_bills = 0;
    let nick = s.ctx.world.party[slot as usize].nick.clone();
    s.ctx.world.text.strings.insert(TextBuffer::NameBuffer, nick);
    print(text::WillLookAfterMonText).then(Label::DayCareTakes(slot))
}

/// `MoveMon` into the day care, `RemovePokemon`, and the cry, which `PlayCry` waits out.
pub(super) fn takes(s: &mut Script, slot: u8) -> Flow {
    let named = s.ctx.world.party.remove(slot as usize);
    let species = named.mon.mon.species;
    s.ctx.world.day_care = Some(Named { mon: deposit(named.mon), ot: named.ot, nick: named.nick });
    s.ctx.audio.play_cry(species as u8);
    s.wait_for_sound_to_finish().then(Label::DayCareTaken)
}

pub(super) fn taken(_s: &mut Script) -> Flow {
    done(text::ComeSeeMeInAWhileText)
}

/// `.daycareInUse` up to its text: the level worked out, the box level moved to it while the man
/// talks, and what is owed.
pub(super) fn in_use(s: &mut Script) -> Flow {
    let world = &mut *s.ctx.world;
    let mon = world.day_care.as_mut().expect("the day care has a mon");
    world.text.strings.insert(TextBuffer::NameBuffer, mon.nick.clone());
    world.text.strings.insert(TextBuffer::DayCareMonName, mon.nick.clone());
    let collected = collection(&mon.mon);
    mon.mon.exp = collected.exp;
    mon.mon.box_level = collected.level;
    s.ow.rt.events.day_care_start_level = collected.start_level;
    world.text.numbers.insert(TextNumber::DayCareNumLevelsGrown, collected.levels_grown as u32);
    world.text.money.insert(TextMoney::DayCareTotalCost, collected.cost.to_vec());
    let words = if collected.levels_grown == 0 { text::MonNeedsMoreTimeText } else { text::MonHasGrownText };
    print(words).then(Label::DayCareGrown)
}

pub(super) fn grown(s: &mut Script) -> Flow {
    if s.ctx.world.party.len() == PARTY_LENGTH {
        return leave(s, text::NoRoomForMonText);
    }
    print(text::OweMoneyText).then(Label::DayCareOwe)
}

pub(super) fn owe(s: &mut Script) -> Flow {
    let money = s.ctx.world.money;
    money_box(&mut s.ctx.screen.ui, &money);
    yes_no(s).then(Label::DayCarePay)
}

fn cost(s: &Script) -> [u8; 3] {
    let cost = s.ctx.world.text.bcd(TextMoney::DayCareTotalCost);
    [0, cost[0], cost[1]]
}

pub(super) fn pay(s: &mut Script) -> Flow {
    if !after_yes_no(s) {
        return leave(s, text::AllRightThenText);
    }
    let cost = cost(s);
    if !has_enough(&s.ctx.world.money, &cost) {
        return leave(s, text::NotEnoughMoneyText);
    }
    s.ow.rt.events.day_care_collected = s.ctx.world.day_care.take();
    s.ctx.world.text.numbers.insert(TextNumber::DayCareNumLevelsGrown, 0);
    sub_bcd(&mut s.ctx.world.money, &cost);
    // `PlaySoundWaitForCurrent`.
    s.wait_for_sound_to_finish().then(Label::DayCarePurchaseSound)
}

pub(super) fn purchase_sound(s: &mut Script) -> Flow {
    s.play_sound(sounds::SFX_PURCHASE);
    let money = s.ctx.world.money;
    money_box(&mut s.ctx.screen.ui, &money);
    print(text::HeresYourMonText).then(Label::DayCareHeresYourMon)
}

/// `MoveMon` back into the party, with the moves learned since and full HP, and the cry.
pub(super) fn heres_your_mon(s: &mut Script) -> Flow {
    let named = s.ow.rt.events.day_care_collected.take().expect("a mon paid for");
    let start_level = s.ow.rt.events.day_care_start_level;
    let mut mon = withdraw(named.mon);
    let species = mon.mon.species;
    write_mon_moves(species, mon.level, &mut mon.mon.moves, &mut mon.mon.pp, Learner::DayCare { start_level });
    mon.mon.hp = mon.stats[0];
    s.ctx.world.party.push(Named { mon, ot: named.ot, nick: named.nick });
    s.ctx.audio.play_cry(species as u8);
    s.wait_for_sound_to_finish().then(Label::DayCareReturned)
}

pub(super) fn returned(_s: &mut Script) -> Flow {
    done(text::GotMonBackText)
}

/// `.leaveMonInDayCare`: the box level put back.
fn leave(s: &mut Script, words: poke_core::symbols::DmgPointer) -> Flow {
    let start_level = s.ow.rt.events.day_care_start_level;
    if let Some(mon) = &mut s.ctx.world.day_care {
        mon.mon.box_level = start_level;
    }
    done(words)
}

//! `StartMenu_Item`: the bag the start menu's `ITEM` row opens, the USE/TOSS menu under a chosen
//! item, `TossItem_`, and the dispatch to what using an item does.
//!
//! The list opens where it was left, on `wBagSavedMenuItem` and `wListScrollOffset`, and every way
//! back to it (`ItemMenuLoop`) puts the start menu's screen back first. B on the list returns to the
//! start menu. The Bicycle skips USE/TOSS. TOSS asks how many unless the item is a key item or an HM,
//! which are refused as too important, then asks to confirm, removes, and says so.
//!
//! USE follows `.useOrTossItem`: a machine and anything in `UsableItems_PartyMenu` goes through the
//! party-menu path, which after a party menu has been up whites out and reopens the bag; anything in `UsableItems_CloseMenu` would close the start menu; everything else is used
//! and the bag reopens. Out of battle the balls, the X items, Guard Spec, Dire Hit, X Accuracy and the
//! Poké Doll are all "not the time", and Oak's Parcel is "not yours to use".
//!
//! The Bicycle is ridden or put away from here (`ItemUseBicycle`), except on Cycling Road; getting on
//! or off closes the start menu, answering `Outcome::Chosen(BICYCLE)`. The Surfboard goes onto the
//! water or off it and reopens the bag, the overworld taking the step once the menu is closed.
//!
//! The Poké Flute is played from here too (`ItemUsePokeFlute`), over the map rather than the bag: a
//! Snorlax the player is standing beside is woken, and its road's own script fights it.
//!
//! An item whose effect is another chunk's is answered rather than run: the bag closes with
//! `Outcome::Chosen(item id)` and the start menu comes back. Each has one arm in `use_item`.

use poke_core::item::{self, ItemId};
use poke_core::map::Map;
use poke_core::map_header::MapHeader;
use poke_core::symbols::pokered_events::{EVENT_BEAT_ROUTE12_SNORLAX, EVENT_BEAT_ROUTE16_SNORLAX,
    EVENT_FIGHT_ROUTE12_SNORLAX, EVENT_FIGHT_ROUTE16_SNORLAX};
use poke_core::symbols::{pokered_symbols, DmgPointer};
use poke_core::text_script::{far_text, TextBuffer};
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, AudioBank, Sound, SoundId};
use crate::command::Decision;
use crate::gfx::text_boxes::TextBoxId;
use crate::gfx::ui::{UiSurface, SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::list_menu::{remove_from_bag, ListMenu};
use crate::modes::menu_input::MenuInput;
use crate::modes::overworld::bike_surf::{item_use_bicycle, item_use_surfboard, Used};
use crate::modes::overworld::escape::{arm_escape_warp, escape_rope_allowed};
use crate::modes::pokedex::PokedexMenu;
use crate::modes::quantity_menu::QuantityMenu;
use crate::modes::text_box::TextBox;
use crate::modes::town_map::TownMap;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::modes::use_item::{UseItem, NO_MENU};
use crate::systems::inventory::Inventory;
use crate::systems::item_use::ItemUse;

const TOSS: u8 = 1;
/// `hlcoord 14, 7`: where `TossItem_` asks its yes/no.
const YES_NO_AT: (usize, usize) = (14, 7);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemMenu {
    /// `wTileMapBackup2`, as the start menu saved it before calling in.
    saved: Option<UiSurface>,
    input: MenuInput,
    item: ItemId,
    /// `wWhichPokemon`: the chosen item's slot in the bag.
    slot: u8,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// A child mode is up, and this is what it was put up for.
    Child(After),
    UseToss,
    /// `PlayDefaultMusic`'s `WaitForSoundToFinish`, then its song and the text.
    Music { text: Option<DmgPointer>, after: After },
    /// `ItemUseEscapeRope`'s `DelayFrames 30` with the map back on screen.
    Delay(u8),
    /// `PlayedFluteHadEffectText`'s `text_asm`: the tune on channel 3, waited out.
    Flute,
}

/// `ItemUseEscapeRope`'s `DelayFrames 30`.
const ESCAPE_ROPE_FRAMES: u8 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    Bag,
    /// A text or a screen after which the bag reopens.
    MenuLoop,
    /// The party-menu path's `UseItem`.
    PartyMenuPath,
    Quantity,
    /// `IsItOKToTossItemText`, with the count chosen.
    AskToss(u8),
    ConfirmToss(u8),
    /// `CloseStartMenu`.
    CloseMenu,
    /// `PlayedFluteHadEffectText` is closed: the tune plays next.
    Flute,
}

/// `wChannelSoundIDs + CHAN3`, which the flute's header claims and `ItemUsePokeFlute` waits on.
const FLUTE_CHANNEL: usize = 2;

/// `ItemUsePokeFlute`'s map test: the event of the Snorlax the player is standing next to, if one is
/// left to wake. `Route12SnorlaxFluteCoords` and `Route16SnorlaxFluteCoords`.
fn snorlax_to_wake(ctx: &Ctx) -> Option<u16> {
    const ROUTE_12: [(u8, u8); 4] = [(9, 62), (10, 61), (10, 63), (11, 62)];
    const ROUTE_16: [(u8, u8); 2] = [(27, 10), (25, 10)];
    let (coords, beat, fight) = match ctx.world.location.map {
        Map::Route12 => (&ROUTE_12[..], EVENT_BEAT_ROUTE12_SNORLAX, EVENT_FIGHT_ROUTE12_SNORLAX),
        Map::Route16 => (&ROUTE_16[..], EVENT_BEAT_ROUTE16_SNORLAX, EVENT_FIGHT_ROUTE16_SNORLAX),
        _ => return None,
    };
    let here = (ctx.world.location.x, ctx.world.location.y);
    (!ctx.world.events.is_set(beat) && coords.contains(&here)).then_some(fight)
}

impl ItemMenu {
    pub fn new() -> Self {
        Self {
            saved: None,
            input: MenuInput::new(0, 1, (14, 11), Joypad::A | Joypad::B),
            item: ItemId::Potion,
            slot: 0,
            phase: Phase::Child(After::Bag),
        }
    }

    /// The USE/TOSS row under the cursor.
    pub fn selected(&self) -> u8 {
        self.input.current
    }

    /// `StartMenu_Item`'s list: the bag, where it was left.
    fn open_bag(&mut self, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Child(After::Bag);
        Transition::Push(Mode::ListMenu(ListMenu::bag(ctx.menu.bag_saved, ctx.menu.list_scroll)))
    }

    /// `ItemMenuLoop`: the start menu's screen back, then the bag again.
    fn menu_loop(&mut self, ctx: &mut Ctx) -> Transition {
        if let Some(saved) = &self.saved {
            ctx.screen.ui = saved.clone();
        }
        self.open_bag(ctx)
    }

    fn text(&mut self, label: &str, after: After) -> Transition {
        self.phase = Phase::Child(after);
        let script = far_text(label).expect("the bag's texts are in the cartridge");
        Transition::Push(Mode::TextBox(TextBox::script(script)))
    }

    fn text_at(&mut self, text: DmgPointer, after: After) -> Transition {
        self.phase = Phase::Child(after);
        Transition::Push(Mode::TextBox(TextBox::script(poke_core::text_script::decode(text).expect("the item's text decodes"))))
    }

    fn after(&mut self, ctx: &mut Ctx, after: After) -> Transition {
        match after {
            After::CloseMenu => Transition::Pop(Outcome::Chosen(self.item as u8)),
            _ => self.menu_loop(ctx),
        }
    }

    /// What `ItemUseBicycle` or `ItemUseSurfboard` came to: the song and the text, then the start
    /// menu closed where `closes` and the item worked, else the bag again.
    fn used(&mut self, ctx: &mut Ctx, used: Used, closes: bool) -> Transition {
        let after = if closes && used.result { After::CloseMenu } else { After::MenuLoop };
        if used.music {
            self.phase = Phase::Music { text: used.text, after };
            return self.update(ctx);
        }
        match used.text {
            Some(text) => self.text_at(text, after),
            None => self.after(ctx, after),
        }
    }

    /// Another chunk's effect: the bag answers with the item and closes.
    fn elsewhere(&self) -> Transition {
        Transition::Pop(Outcome::Chosen(self.item as u8))
    }

    /// `.choseItem`: the list's cursors rubbed out bar its `▷`, then USE/TOSS unless it is the bike.
    fn chosen(&mut self, ctx: &mut Ctx, slot: u8) -> Transition {
        self.slot = slot;
        self.item = ctx.world.bag.items[slot as usize].id;
        for y in [4, 6, 8, 10] {
            ctx.screen.ui.set(5, y, UiSurface::BLANK);
        }
        ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
        if self.item == ItemId::Bicycle {
            return self.use_or_toss(ctx);
        }
        TextBoxId::UseToss.draw(&mut ctx.screen.ui);
        self.input = MenuInput::new(0, 1, (14, 11), Joypad::A | Joypad::B);
        ctx.menu.last_item = 0;
        self.input.call(ctx);
        self.phase = Phase::UseToss;
        self.update(ctx)
    }

    /// `.useOrTossItem`.
    fn use_or_toss(&mut self, ctx: &mut Ctx) -> Transition {
        let name = item::name(self.item);
        ctx.world.text.strings.insert(TextBuffer::NameBuffer, name.clone());
        ctx.world.text.strings.insert(TextBuffer::StringBuffer, name);
        if self.item == ItemId::Bicycle {
            if ctx.world.location.always_on_bike {
                return self.text("_CannotGetOffHereText", After::MenuLoop);
            }
            let used = item_use_bicycle(ctx);
            return self.used(ctx, used, true);
        }
        if self.input.current == TOSS {
            return self.toss(ctx);
        }
        if self.item == ItemId::EscapeRope {
            return self.escape_rope(ctx);
        }
        if self.item == ItemId::PokeFlute {
            return self.poke_flute(ctx);
        }
        let party_menu_path = self.item as u8 >= ItemId::Hm01Cut as u8 || item::opens_party_menu(self.item);
        if !party_menu_path && item::closes_menu(self.item) {
            return self.elsewhere();
        }
        self.use_item(ctx, party_menu_path)
    }

    /// `UseItem`, one arm per routine `ItemUsePtrTable` names.
    fn use_item(&mut self, ctx: &mut Ctx, party_menu_path: bool) -> Transition {
        let after = if party_menu_path { After::PartyMenuPath } else { After::MenuLoop };
        match ItemUse::of(self.item) {
            ItemUse::Medicine | ItemUse::Vitamin | ItemUse::RareCandy | ItemUse::PpUp | ItemUse::PpRestore
            | ItemUse::EvoStone | ItemUse::TmHm => {
                self.phase = Phase::Child(After::PartyMenuPath);
                let flow = UseItem::new(self.item, self.slot).expect("one of the routines `UseItem` recreates");
                Transition::Push(Mode::UseItem(flow))
            }
            // Every one of these begins by testing `wIsInBattle`, and the bag is never in one.
            ItemUse::Unusable | ItemUse::Ball | ItemUse::XAccuracy | ItemUse::GuardSpec | ItemUse::DireHit
            | ItemUse::PokeDoll => self.text("_ItemUseNotTimeText", after),
            // `ItemUseXStat` answers 2 as well, so the bag reopens without the party-menu path's pause.
            ItemUse::XStat => self.text("_ItemUseNotTimeText", After::MenuLoop),
            ItemUse::OaksParcel => self.text("_ItemUseNotYoursToUseText", after),
            ItemUse::Pokedex => {
                self.phase = Phase::Child(after);
                Transition::Push(Mode::Pokedex(PokedexMenu::new()))
            }
            ItemUse::Surfboard => {
                let used = item_use_surfboard(ctx);
                self.used(ctx, used, false)
            }
            ItemUse::TownMap => {
                self.phase = Phase::Child(After::MenuLoop);
                Transition::Push(Mode::TownMap(TownMap::item()))
            }
            // The overworld's and the battle's.
            ItemUse::Bicycle | ItemUse::Bait | ItemUse::Rock | ItemUse::EscapeRope | ItemUse::Repel | ItemUse::SuperRepel | ItemUse::MaxRepel | ItemUse::CardKey
            | ItemUse::PokeFlute | ItemUse::CoinCase | ItemUse::OldRod | ItemUse::GoodRod | ItemUse::SuperRod
            | ItemUse::Itemfinder => self.elsewhere(),
        }
    }

    /// `ItemUseEscapeRope`: the escape warp armed, the map back on screen for thirty frames, and
    /// then the rope spent and the start menu closed. A map it does not work on says so and the bag
    /// comes back.
    fn escape_rope(&mut self, ctx: &mut Ctx) -> Transition {
        let tileset = MapHeader::read(ctx.world.location.map).expect("the player stands on a map with a header").tileset;
        if !escape_rope_allowed(ctx.world.location.map, tileset) {
            return self.text("_ItemUseNotTimeText", After::MenuLoop);
        }
        arm_escape_warp(ctx);
        ctx.screen.ui.uncover(0, 0, crate::gfx::ui::SCREEN_TILES_X, crate::gfx::ui::SCREEN_TILES_Y);
        self.phase = Phase::Delay(ESCAPE_ROPE_FRAMES);
        Transition::Stay
    }

    /// `ItemUsePokeFlute` out of battle: the tune played over the map rather than the bag, and a
    /// Snorlax standing beside the player woken, which its road's own script then fights.
    fn poke_flute(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.screen.ui.uncover(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y);
        if snorlax_to_wake(ctx).is_none() {
            return self.text("_PlayedFluteNoEffectText", After::CloseMenu);
        }
        self.text_at(pokered_symbols::PlayedFluteHadEffectText, After::Flute)
    }

    /// `.tossItem`: how many, unless `TossItem_` is going to refuse it anyway.
    fn toss(&mut self, ctx: &mut Ctx) -> Transition {
        if !Inventory::may_toss(self.item) {
            return self.text("_TooImportantToTossText", After::MenuLoop);
        }
        self.phase = Phase::Child(After::Quantity);
        let held = ctx.world.bag.items[self.slot as usize].quantity;
        Transition::Push(Mode::QuantityMenu(QuantityMenu::new(held, None)))
    }
}

impl Default for ItemMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeUpdate for ItemMenu {
    fn enter(&mut self, ctx: &mut Ctx) {
        self.saved = Some(ctx.screen.ui.clone());
    }

    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.open_bag(ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Child(_) => Transition::Stay,
            Phase::Music { .. } if !ctx.audio.sound_finished() => Transition::Stay,
            Phase::Music { text, after } => {
                crate::modes::overworld::play_default_music(ctx);
                match text {
                    Some(text) => self.text_at(text, after),
                    None => self.after(ctx, after),
                }
            }
            Phase::Flute if ctx.pacing != crate::Pacing::Instant
                && ctx.audio.channel_sound_id(FLUTE_CHANNEL) == sounds::SFX_POKEFLUTE.0 => Transition::Stay,
            Phase::Flute => {
                crate::modes::overworld::play_default_music(ctx);
                if let Some(event) = snorlax_to_wake(ctx) {
                    ctx.world.events.set(event);
                }
                Transition::Pop(Outcome::Chosen(self.item as u8))
            }
            Phase::Delay(1) => {
                remove_from_bag(ctx, self.slot as usize, 1);
                Transition::Pop(Outcome::Chosen(self.item as u8))
            }
            Phase::Delay(frames) => {
                self.phase = Phase::Delay(frames - 1);
                Transition::Stay
            }
            Phase::UseToss => {
                let Some(keys) = self.input.update(ctx) else { return Transition::Stay };
                ctx.menu.unfilled_cursor(&mut ctx.screen.ui);
                if keys.contains(Joypad::B) {
                    return self.menu_loop(ctx);
                }
                self.use_or_toss(ctx)
            }
        }
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        let Phase::Child(after) = self.phase else { return Transition::Stay };
        match (after, outcome) {
            (After::Bag, outcome) => {
                ctx.menu.bag_saved = ctx.menu.chosen_item;
                match outcome {
                    Outcome::Chosen(slot) => self.chosen(ctx, slot),
                    _ => Transition::Pop(Outcome::Done),
                }
            }
            (After::MenuLoop, _) => self.menu_loop(ctx),
            (After::PartyMenuPath, Outcome::Chosen(NO_MENU)) => self.menu_loop(ctx),
            // `GBPalWhiteOutWithDelay3` and `RestoreScreenTilesAndReloadTilePatterns`, whose delays
            // are loading and not modelled.
            (After::PartyMenuPath, _) => {
                crate::gfx::mon_icons::clear_sprites(&mut ctx.screen.sprites);
                if let Some(saved) = &self.saved {
                    ctx.screen.ui = saved.clone();
                }
                ctx.screen.tiles.load_text_box_tiles();
                self.open_bag(ctx)
            }
            (After::Quantity, Outcome::Chosen(quantity)) => self.text("_IsItOKToTossItemText", After::AskToss(quantity)),
            (After::Quantity, _) => self.menu_loop(ctx),
            (After::AskToss(quantity), _) => {
                self.phase = Phase::Child(After::ConfirmToss(quantity));
                Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, YES_NO_AT, false)))
            }
            (After::ConfirmToss(quantity), Outcome::Chosen(0)) => {
                remove_from_bag(ctx, self.slot as usize, quantity);
                self.text("_ThrewAwayItemText", After::MenuLoop)
            }
            (After::ConfirmToss(_), _) => self.menu_loop(ctx),
            (After::Flute, _) => {
                ctx.audio.play_sound(SoundId::STOP_ALL_MUSIC);
                ctx.audio.play_music(Sound { bank: AudioBank::One, id: sounds::SFX_POKEFLUTE });
                self.phase = Phase::Flute;
                Transition::Stay
            }
            (After::CloseMenu, _) => self.after(ctx, After::CloseMenu),
        }
    }

    fn status(&self) -> Status {
        match self.phase {
            Phase::UseToss if self.input.is_polling() => Status::Waiting(Decision::UseToss),
            _ => Status::Busy,
        }
    }
}

#[cfg(test)]
mod tests {
    use poke_core::bag::BagItem;
    use poke_core::charmap::encode;
    use poke_core::move_name::PokemonMoveName;
    use poke_core::species::PokemonSpecies;
    use crate::command::{Command, Reply};
    use crate::party::Named;
    use crate::rng::GameRng;
    use crate::systems::add_mon::{new_party_mon, Origin};
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    fn game(bag: &[(ItemId, u8)], hp: u16) -> Game {
        let mut mon = new_party_mon(PokemonSpecies::Pidgey, 20, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
        mon.mon.hp = hp;
        let party = vec![Named { mon, ot: encode("RED").unwrap(), nick: encode("BIRD").unwrap() }];
        let world = World {
            player_name: encode("RED").unwrap(),
            party,
            bag: Inventory::bag(bag.iter().map(|&(id, q)| BagItem::new(id, q)).collect()),
            ..World::default()
        };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::ItemMenu(ItemMenu::new()));
        game
    }

    /// Frames until something waits on the player, and what.
    fn settle(game: &mut Game) -> Decision {
        for _ in 0..2000 {
            if let Status::Waiting(decision) = game.status() {
                return decision;
            }
            game.frame(Input::None);
        }
        panic!("nothing ever waited: {:?}", game.modes().last());
    }

    /// A command at the next decision, which has to be the one expected, carried out.
    fn answer(game: &mut Game, decision: Decision, command: Command) {
        assert_eq!(settle(game), decision, "before {command:?}");
        assert_eq!(game.frame(Input::Command(command.clone())).reply, Some(Reply::Accepted), "{command:?}");
        for _ in 0..600 {
            if game.frame(Input::None).events.contains(&Event::CommandDone(command.clone())) {
                return;
            }
        }
        panic!("{command:?} never finished");
    }

    fn text_row(game: &Game, y: usize) -> Vec<u8> {
        let mut row = game.ui().row(y)[1..18].to_vec();
        while row.last() == Some(&UiSurface::BLANK) {
            row.pop();
        }
        row
    }

    #[test]
    fn b_on_the_bag_goes_back_to_the_start_menu() {
        let mut game = game(&[(ItemId::Potion, 3)], 10);
        answer(&mut game, Decision::List, Command::CancelList);
        assert!(game.modes().is_empty());
    }

    #[test]
    fn toss_asks_how_many_and_to_confirm_then_says_so() {
        let mut game = game(&[(ItemId::Antidote, 1), (ItemId::Potion, 5)], 10);
        answer(&mut game, Decision::List, Command::ChooseListEntry(1));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(TOSS));
        answer(&mut game, Decision::Quantity, Command::ChooseQuantity(3));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 16), encode("POTION?").unwrap());
        answer(&mut game, Decision::Text, Command::Advance);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("Threw away").unwrap());
        assert_eq!(game.world().bag.quantity_of(ItemId::Potion), 2);
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::List, "and the bag again");
        assert_eq!(game.menu().bag_saved, 1, "on the row it was left on");
    }

    #[test]
    fn no_keeps_the_items_and_a_key_item_is_never_asked_about() {
        let mut game = game(&[(ItemId::Potion, 5), (ItemId::TownMap, 1)], 10);
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(TOSS));
        answer(&mut game, Decision::Quantity, Command::ChooseQuantity(5));
        answer(&mut game, Decision::Text, Command::Advance);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(1));
        answer(&mut game, Decision::List, Command::ChooseListEntry(1));
        assert_eq!(game.world().bag.quantity_of(ItemId::Potion), 5);
        answer(&mut game, Decision::UseToss, Command::ChooseOption(TOSS));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("That's too impor-").unwrap());
    }

    #[test]
    fn a_potion_heals_the_mon_it_is_used_on_and_the_bag_comes_back() {
        let mut game = game(&[(ItemId::Potion, 2)], 10);
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(0));
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(game.world().party[0].mon.mon.hp, 30);
        assert_eq!(text_row(&game, 16), encode("recovered by 20!").unwrap());
        assert_eq!(game.world().bag.quantity_of(ItemId::Potion), 1);
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::List);
    }

    #[test]
    fn a_potion_on_a_healthy_mon_has_no_effect_and_is_kept() {
        let mut game = game(&[(ItemId::Potion, 2)], 10);
        let max = game.world().party[0].mon.stats[0];
        let mut world = game.world().clone();
        world.party[0].mon.mon.hp = max;
        game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::ItemMenu(ItemMenu::new()));
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(0));
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("It won't have any").unwrap());
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::List);
        assert_eq!(game.world().bag.quantity_of(ItemId::Potion), 2);
    }

    #[test]
    fn a_nugget_is_not_the_time_and_the_bag_comes_back() {
        let mut game = game(&[(ItemId::Nugget, 1), (ItemId::EscapeRope, 1)], 10);
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("OAK: RED!").unwrap());
        while settle(&mut game) == Decision::Text {
            answer(&mut game, Decision::Text, Command::Advance);
        }
        assert_eq!(settle(&mut game), Decision::List, "the bag again");
    }

    /// Red's bedroom has a tileset of its own, so the rope wants a cave, a building or a tower.
    #[test]
    fn an_escape_rope_underground_arms_the_warp_and_is_spent() {
        let mut game = game(&[(ItemId::EscapeRope, 1)], 10);
        let mut world = game.world().clone();
        world.location.map = poke_core::map::Map::MtMoon1F;
        game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::ItemMenu(ItemMenu::new()));
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(0));
        for _ in 0..ESCAPE_ROPE_FRAMES as u32 + 2 {
            if game.modes().is_empty() {
                break;
            }
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty(), "the start menu closes behind the rope");
        assert_eq!(game.world().bag.quantity_of(ItemId::EscapeRope), 0, "and the rope is spent");
        assert!(game.world().location.escape_warp);
        assert_eq!(game.world().location.fly_warp, Some(game.world().location.last_blackout_map));
    }

    #[test]
    fn an_escape_rope_out_in_the_open_is_not_the_time_and_the_bag_comes_back() {
        let mut game = game(&[(ItemId::EscapeRope, 1)], 10);
        let mut world = game.world().clone();
        world.location.map = poke_core::map::Map::PalletTown;
        game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::ItemMenu(ItemMenu::new()));
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        while settle(&mut game) == Decision::Text {
            answer(&mut game, Decision::Text, Command::Advance);
        }
        assert_eq!(settle(&mut game), Decision::List, "the bag again");
        assert_eq!(game.world().bag.quantity_of(ItemId::EscapeRope), 1, "and no rope spent");
        assert!(!game.world().location.escape_warp);
    }

    #[test]
    fn an_ether_asks_for_a_move_and_backing_out_asks_for_a_mon_again() {
        let mut game = game(&[(ItemId::Ether, 1)], 10);
        let mut world = game.world().clone();
        world.party[0].mon.mon.pp[0] = 3;
        game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::ItemMenu(ItemMenu::new()));
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(0));
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        answer(&mut game, Decision::MoveMenu, Command::CancelOption);
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        answer(&mut game, Decision::MoveMenu, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("PP was restored.").unwrap());
        assert_eq!(game.world().party[0].mon.mon.pp[0], 13);
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::List);
        assert!(game.world().bag.items.is_empty());
    }

    #[test]
    fn an_hp_up_raises_the_max_but_not_the_hp_and_a_pp_up_on_three_asks_again() {
        let mut game = game(&[(ItemId::HpUp, 1), (ItemId::PpUp, 1)], 10);
        let before = game.world().party[0].mon.stats[0];
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(0));
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 16), encode("HEALTH rose.").unwrap());
        assert!(game.world().party[0].mon.stats[0] >= before);
        assert_eq!(game.world().party[0].mon.mon.hp, 10);
        answer(&mut game, Decision::Text, Command::Advance);

        let mut world = game.world().clone();
        world.party[0].mon.mon.pp[0] |= 3 << 6;
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::ItemMenu(ItemMenu::new()));
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(0));
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        answer(&mut game, Decision::MoveMenu, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 16), encode("is maxed out.").unwrap());
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::MoveMenu, "which move, again");
    }

    /// A party of one mon of `species` at `level`, knowing only its first move, and a bag.
    fn bike_at(map: poke_core::map::Map) -> Game {
        let mut game = game(&[(ItemId::Bicycle, 1)], 10);
        let mut world = game.world().clone();
        world.location.map = map;
        game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::ItemMenu(ItemMenu::new()));
        game
    }

    #[test]
    fn the_bicycle_is_got_on_and_off_and_closes_the_menu_each_time() {
        use crate::systems::overworld::location::{BIKING, WALKING};
        let mut game = bike_at(poke_core::map::Map::PalletTown);
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("RED got on the").unwrap());
        assert_eq!(game.world().location.walk_bike_surf, BIKING);
        assert!(game.ui().cover(10, 4).is_none(), "the map is back over the bag");
        game.frame(Input::Command(Command::Advance));
        for _ in 0..100 {
            game.frame(Input::None);
        }
        assert!(game.modes().is_empty(), "the bag closed with the item");

        game.push(Mode::ItemMenu(ItemMenu::new()));
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("RED got off").unwrap());
        assert_eq!(game.world().location.walk_bike_surf, WALKING);
    }

    #[test]
    fn no_cycling_indoors_and_no_getting_off_on_cycling_road() {
        let mut game = bike_at(poke_core::map::Map::RedsHouse1F);
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("No cycling").unwrap());
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::List);

        let mut game = bike_at(poke_core::map::Map::Route17);
        let mut world = game.world().clone();
        world.location.always_on_bike = true;
        world.location.walk_bike_surf = crate::systems::overworld::location::BIKING;
        game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::ItemMenu(ItemMenu::new()));
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("You can't get off").unwrap());
    }

    fn party_of(species: PokemonSpecies, level: u8, bag: &[(ItemId, u8)]) -> Game {
        let mut mon = new_party_mon(species, level, 0, &Origin::Trainer, &mut GameRng::tape(vec![]));
        mon.mon.moves = [mon.mon.moves[0], None, None, None];
        let party = vec![Named { mon, ot: encode("RED").unwrap(), nick: encode("MON").unwrap() }];
        let world = World {
            player_name: encode("RED").unwrap(),
            party,
            bag: Inventory::bag(bag.iter().map(|&(id, q)| BagItem::new(id, q)).collect()),
            ..World::default()
        };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Instant);
        game.push(Mode::ItemMenu(ItemMenu::new()));
        game
    }

    /// Every text answered with a press until `decision` is up. A press rather than `Advance`,
    /// whose driver keeps pressing into a second text box that replaces the first in one frame.
    fn read_on_to(game: &mut Game, decision: Decision) {
        while settle(game) == Decision::Text && decision != Decision::Text {
            game.frame(Input::Buttons(Joypad::A));
            game.frame(Input::None);
        }
        assert_eq!(settle(game), decision);
    }

    fn use_first(game: &mut Game) {
        answer(game, Decision::List, Command::ChooseListEntry(0));
        answer(game, Decision::UseToss, Command::ChooseOption(0));
    }

    #[test]
    fn a_tm_is_used_up_teaching_and_an_hm_is_not() {
        let mut game = party_of(PokemonSpecies::Pidgey, 20, &[(ItemId::Tm06Toxic, 1), (ItemId::Hm02Fly, 1)]);
        use_first(&mut game);
        read_on_to(&mut game, Decision::TwoOption);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::PartyMenu);
        assert_eq!(game.ui().row(1)[12..16], encode("ABLE").unwrap()[..]);
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        read_on_to(&mut game, Decision::List);
        assert_eq!(game.world().party[0].mon.mon.moves[1], Some(PokemonMoveName::Toxic));
        assert_eq!(game.world().bag.items, [BagItem::new(ItemId::Hm02Fly, 1)]);

        use_first(&mut game);
        read_on_to(&mut game, Decision::TwoOption);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(0));
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        read_on_to(&mut game, Decision::List);
        assert_eq!(game.world().party[0].mon.mon.moves[2], Some(PokemonMoveName::Fly));
        assert_eq!(game.world().bag.items, [BagItem::new(ItemId::Hm02Fly, 1)], "an HM stays");
    }

    #[test]
    fn a_mon_that_cannot_learn_the_machine_is_refused_and_no_puts_it_away() {
        let mut game = party_of(PokemonSpecies::Rattata, 20, &[(ItemId::Hm02Fly, 1)]);
        use_first(&mut game);
        read_on_to(&mut game, Decision::TwoOption);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::PartyMenu);
        assert_eq!(game.ui().row(1)[12..20], encode("NOT ABLE").unwrap()[..]);
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("MON is not").unwrap());
        read_on_to(&mut game, Decision::PartyMenu);
        answer(&mut game, Decision::PartyMenu, Command::CancelOption);
        assert_eq!(settle(&mut game), Decision::List);

        use_first(&mut game);
        read_on_to(&mut game, Decision::TwoOption);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(1));
        assert_eq!(settle(&mut game), Decision::List, "NO goes straight back to the bag");
    }

    #[test]
    fn a_rare_candy_levels_up_shows_the_stats_and_offers_the_level_s_move() {
        let mut game = party_of(PokemonSpecies::Rattata, 13, &[(ItemId::RareCandy, 2)]);
        use_first(&mut game);
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 16), encode("to level 14!").unwrap());
        // The party menu's A is still the last thing polled, so the prompt needs it let go first.
        game.frame(Input::None);
        game.frame(Input::Buttons(Joypad::A));
        assert_eq!(settle(&mut game), Decision::Text, "the stats box, and a press");
        assert_eq!(game.ui().row(3)[11..17], encode("ATTACK").unwrap()[..]);
        game.frame(Input::None);
        game.frame(Input::Buttons(Joypad::A));
        game.frame(Input::None);
        read_on_to(&mut game, Decision::List);
        assert_eq!(game.world().party[0].mon.level, 14);
        assert_eq!(game.world().party[0].mon.mon.moves[1], Some(PokemonMoveName::HyperFang), "learned at 14");
        assert_eq!(game.world().bag.quantity_of(ItemId::RareCandy), 1);
    }

    #[test]
    fn a_stone_evolves_the_mon_it_suits_and_is_kept_by_one_it_does_not() {
        let mut game = party_of(PokemonSpecies::Pikachu, 5, &[(ItemId::MoonStone, 1), (ItemId::ThunderStone, 1)]);
        use_first(&mut game);
        assert_eq!(settle(&mut game), Decision::PartyMenu);
        assert_eq!(game.ui().row(1)[12..20], encode("NOT ABLE").unwrap()[..]);
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(text_row(&game, 14), encode("It won't have any").unwrap());
        read_on_to(&mut game, Decision::List);
        assert_eq!(game.world().bag.quantity_of(ItemId::MoonStone), 1);

        answer(&mut game, Decision::List, Command::ChooseListEntry(1));
        answer(&mut game, Decision::UseToss, Command::ChooseOption(0));
        answer(&mut game, Decision::PartyMenu, Command::ChooseOption(0));
        read_on_to(&mut game, Decision::List);
        assert_eq!(game.world().party[0].mon.mon.species, PokemonSpecies::Raichu);
        assert_eq!(game.world().bag.quantity_of(ItemId::ThunderStone), 0);
    }

    #[test]
    fn a_save_mid_heal_resumes_identically() {
        let mut whole = game(&[(ItemId::SuperPotion, 1)], 1);
        answer(&mut whole, Decision::List, Command::ChooseListEntry(0));
        answer(&mut whole, Decision::UseToss, Command::ChooseOption(0));
        assert_eq!(settle(&mut whole), Decision::PartyMenu);
        whole.frame(Input::Buttons(Joypad::A));
        for _ in 0..20 {
            whole.frame(Input::None);
        }
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..300 {
            let (a, b) = (whole.frame(Input::None), restored.frame(Input::None));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
        assert_eq!(restored.world(), whole.world());
    }
}

//! `DisplayPokemartDialogue` and `DisplayPokemartDialogue_`: a clerk's greeting, then buying and
//! selling until the player quits.
//!
//! BUY lists the mart's stock with prices; a count up to 99 is priced as it changes, and a yes
//! spends the money and fills the bag, or says there is not enough money or no room, in that order.
//! SELL lists the bag; a key item or an HM cannot be priced, anything else sells for half. Each
//! list reopens after a purchase or a sale at the top row of the window, keeping its scroll; leaving
//! one goes back to BUY/SELL/QUIT by way of "anything else". `wListScrollOffset` is zeroed at every
//! BUY/SELL/QUIT and put back as the clerk says goodbye.
//!
//! `wBoughtOrSoldItemInMart` is not recreated: the cartridge writes it and nothing reads it.

use poke_core::item::{self, ItemId};
use poke_core::text_script::{far_text, TextBuffer, TextMoney};
use serde::{Deserialize, Serialize};
use crate::audio::data::sounds;
use crate::gfx::text_boxes::money_box;
use crate::gfx::ui::UiSurface;
use crate::mode::{Ctx, Mode, ModeUpdate, Outcome, Status, Transition};
use crate::modes::buy_sell_quit::BuySellQuitMenu;
use crate::modes::list_menu::{remove_from_bag, ListMenu};
use crate::modes::menu_input::MenuExit;
use crate::modes::pc::update_sprites;
use crate::modes::quantity_menu::{Price, QuantityMenu};
use crate::modes::text_box::TextBox;
use crate::modes::two_option_menu::{TwoOptionMenu, TwoOptionMenuId};
use crate::systems::inventory::Inventory;
use crate::systems::money::{add_sold, subtract_paid};

const BUY: u8 = 0;
const SELL: u8 = 1;
/// `hlcoord 14, 7`, the yes/no's corner.
const YES_NO_AT: (usize, usize) = (14, 7);
/// `wMaxItemQuantity` for a purchase.
const MAX_PURCHASE: u8 = 99;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pokemart {
    /// `wItemList`.
    items: Vec<ItemId>,
    /// `wSavedListScrollOffset`.
    saved_scroll: u8,
    /// `wTileMapBackup`.
    saved: Option<UiSurface>,
    item: ItemId,
    /// The chosen entry: a place in the stock or in the bag.
    slot: u8,
    quantity: u8,
    /// `hMoney`.
    total: [u8; 3],
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Child(After),
    /// `PlaySoundWaitForCurrent SFX_PURCHASE`, then `WaitForSoundToFinish`, and what follows.
    WaitingToPlay(Paid),
    Playing(Paid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Paid {
    Bought,
    Sold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    Greeting,
    Menu,
    Goodbye,
    /// A text after which the screen is saved and this happens.
    ThenSave(Then),
    /// A text after which this happens.
    Then(Then),
    BuyList,
    BuyQuantity,
    BuyPrice,
    BuyConfirm,
    SellList,
    SellQuantity,
    SellPrice,
    SellConfirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Then {
    MainMenu,
    ReturnToMainMenu,
    BuyLoop,
    SellLoop,
}

impl Pokemart {
    /// `items` is the mart's stock, in the order its `script_mart` lists it.
    pub fn new(items: Vec<ItemId>) -> Self {
        Self {
            items,
            saved_scroll: 0,
            saved: None,
            item: ItemId::Potion,
            slot: 0,
            quantity: 0,
            total: [0; 3],
            phase: Phase::Child(After::Greeting),
        }
    }

    /// `PrintText`, whose `UpdateSprites` follows the box.
    fn text(&mut self, label: &str, after: After, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Child(after);
        update_sprites(ctx);
        let script = far_text(label).expect("the mart's texts are in the cartridge");
        Transition::Push(Mode::TextBox(TextBox::script(script)))
    }

    /// `.loop`.
    fn main_menu(&mut self, ctx: &mut Ctx) -> Transition {
        ctx.menu.list_scroll = 0;
        money_box(&mut ctx.screen.ui, &ctx.world.money);
        update_sprites(ctx);
        self.phase = Phase::Child(After::Menu);
        Transition::Push(Mode::BuySellQuitMenu(BuySellQuitMenu::new()))
    }

    /// `LoadScreenTilesFromBuffer1` and the money box, whose `DisplayTextBoxID` updates the sprites,
    /// as the list drawn next does.
    fn restore_screen(&self, ctx: &mut Ctx) {
        if let Some(saved) = &self.saved {
            ctx.screen.ui = saved.clone();
        }
        money_box(&mut ctx.screen.ui, &ctx.world.money);
        update_sprites(ctx);
    }

    /// `.returnToMainPokemartMenu`.
    fn return_to_main_menu(&mut self, ctx: &mut Ctx) -> Transition {
        self.restore_screen(ctx);
        self.text("_PokemartAnythingElseText", After::Then(Then::MainMenu), ctx)
    }

    fn buy_loop(&mut self, ctx: &mut Ctx) -> Transition {
        self.restore_screen(ctx);
        self.phase = Phase::Child(After::BuyList);
        Transition::Push(Mode::ListMenu(ListMenu::priced(self.items.clone(), 0, ctx.menu.list_scroll)))
    }

    fn sell_loop(&mut self, ctx: &mut Ctx) -> Transition {
        self.restore_screen(ctx);
        self.phase = Phase::Child(After::SellList);
        Transition::Push(Mode::ListMenu(ListMenu::bag(0, ctx.menu.list_scroll)))
    }

    fn then(&mut self, ctx: &mut Ctx, then: Then) -> Transition {
        match then {
            Then::MainMenu => self.main_menu(ctx),
            Then::ReturnToMainMenu => self.return_to_main_menu(ctx),
            Then::BuyLoop => self.buy_loop(ctx),
            Then::SellLoop => self.sell_loop(ctx),
        }
    }

    /// `DisplayTwoOptionMenu`, which updates the sprites as it draws and as it restores.
    fn yes_no(&mut self, after: After, ctx: &mut Ctx) -> Transition {
        self.phase = Phase::Child(after);
        update_sprites(ctx);
        Transition::Push(Mode::TwoOptionMenu(TwoOptionMenu::new(TwoOptionMenuId::YesNo, YES_NO_AT, false)))
    }

    fn quantity(&mut self, after: After, max: u8, halved: bool) -> Transition {
        self.phase = Phase::Child(after);
        let price = Price { each: item::price(self.item).unwrap_or_default(), halved };
        Transition::Push(Mode::QuantityMenu(QuantityMenu::new(max, Some(price))))
    }

    /// The count chosen and what it costs, into the buffers the price texts read.
    fn priced(&mut self, ctx: &mut Ctx, quantity: u8, halved: bool) {
        self.quantity = quantity;
        let each = item::price(self.item).unwrap_or_default();
        self.total = crate::systems::money::total_price(each, quantity, halved);
        ctx.world.text.money.insert(TextMoney::Money, self.total.to_vec());
    }
}

impl ModeUpdate for Pokemart {
    fn open(&mut self, ctx: &mut Ctx) -> Transition {
        self.saved_scroll = ctx.menu.list_scroll;
        update_sprites(ctx);
        self.text("_PokemartGreetingText", After::Greeting, ctx)
    }

    fn update(&mut self, ctx: &mut Ctx) -> Transition {
        match self.phase {
            Phase::Child(_) => Transition::Stay,
            Phase::WaitingToPlay(paid) => {
                if !ctx.audio.sound_finished() {
                    return Transition::Stay;
                }
                ctx.audio.play_sound(sounds::SFX_PURCHASE);
                self.phase = Phase::Playing(paid);
                Transition::Stay
            }
            Phase::Playing(paid) => {
                if !ctx.audio.sound_finished() {
                    return Transition::Stay;
                }
                match paid {
                    Paid::Bought => self.text("_PokemartBoughtItemText", After::Then(Then::BuyLoop), ctx),
                    Paid::Sold => self.sell_loop(ctx),
                }
            }
        }
    }

    fn resume(&mut self, outcome: Outcome, ctx: &mut Ctx) -> Transition {
        let Phase::Child(after) = self.phase else { return Transition::Stay };
        if matches!(after, After::BuyConfirm | After::SellConfirm) {
            // `TwoOptionMenu_RestoreScreenTiles`.
            update_sprites(ctx);
        }
        match after {
            After::Greeting => self.main_menu(ctx),
            After::Menu => match (ctx.menu.exit_method, ctx.menu.chosen_item) {
                (MenuExit::Chose, BUY) => self.text("_PokemartBuyingGreetingText", After::ThenSave(Then::BuyLoop), ctx),
                (MenuExit::Chose, SELL) if ctx.world.bag.items.is_empty() =>
                    self.text("_PokemartItemBagEmptyText", After::ThenSave(Then::ReturnToMainMenu), ctx),
                (MenuExit::Chose, SELL) => self.text("_PokemonSellingGreetingText", After::ThenSave(Then::SellLoop), ctx),
                _ => self.text("_PokemartThankYouText", After::Goodbye, ctx),
            },
            After::Goodbye => {
                update_sprites(ctx);
                ctx.menu.list_scroll = self.saved_scroll;
                Transition::Pop(Outcome::Done)
            }
            After::ThenSave(then) => {
                self.saved = Some(ctx.screen.ui.clone());
                self.then(ctx, then)
            }
            After::Then(then) => self.then(ctx, then),
            After::BuyList => match outcome {
                Outcome::Chosen(slot) => {
                    self.slot = slot;
                    self.item = self.items[slot as usize];
                    self.quantity(After::BuyQuantity, MAX_PURCHASE, false)
                }
                _ => self.return_to_main_menu(ctx),
            },
            After::BuyQuantity => match outcome {
                Outcome::Chosen(quantity) => {
                    self.priced(ctx, quantity, false);
                    ctx.world.text.strings.insert(TextBuffer::StringBuffer, item::name(self.item));
                    self.text("_PokemartTellBuyPriceText", After::BuyPrice, ctx)
                }
                _ => self.buy_loop(ctx),
            },
            After::BuyPrice => self.yes_no(After::BuyConfirm, ctx),
            After::BuyConfirm => {
                if outcome != Outcome::Chosen(0) {
                    return self.buy_loop(ctx);
                }
                if !crate::systems::money::has_enough(&ctx.world.money, &self.total) {
                    return self.text("_PokemartNotEnoughMoneyText", After::Then(Then::ReturnToMainMenu), ctx);
                }
                if !ctx.world.bag.add(self.item, self.quantity) {
                    return self.text("_PokemartItemBagFullText", After::Then(Then::ReturnToMainMenu), ctx);
                }
                // `SubtractAmountPaidFromMoney_` redraws the money box over the list's corner.
                subtract_paid(&mut ctx.world.money, &self.total);
                money_box(&mut ctx.screen.ui, &ctx.world.money);
                self.phase = Phase::WaitingToPlay(Paid::Bought);
                self.update(ctx)
            }
            After::SellList => match outcome {
                Outcome::Chosen(slot) => {
                    self.slot = slot;
                    let held = ctx.world.bag.items[slot as usize];
                    self.item = held.id;
                    if !Inventory::may_toss(self.item) {
                        return self.text("_PokemartUnsellableItemText", After::Then(Then::ReturnToMainMenu), ctx);
                    }
                    self.quantity(After::SellQuantity, held.quantity, true)
                }
                _ => self.return_to_main_menu(ctx),
            },
            After::SellQuantity => match outcome {
                Outcome::Chosen(quantity) => {
                    self.priced(ctx, quantity, true);
                    self.text("_PokemartTellSellPriceText", After::SellPrice, ctx)
                }
                _ => self.sell_loop(ctx),
            },
            After::SellPrice => self.yes_no(After::SellConfirm, ctx),
            After::SellConfirm => {
                if outcome != Outcome::Chosen(0) {
                    return self.sell_loop(ctx);
                }
                // `AddAmountSoldToMoney` redraws the money box and rings the till.
                add_sold(&mut ctx.world.money, &self.total);
                money_box(&mut ctx.screen.ui, &ctx.world.money);
                if remove_from_bag(ctx, self.slot as usize, self.quantity) {
                    self.saved_scroll = 0;
                }
                self.phase = Phase::WaitingToPlay(Paid::Sold);
                self.update(ctx)
            }
        }
    }

    fn status(&self) -> Status {
        Status::Busy
    }
}

#[cfg(test)]
mod tests {
    use poke_core::bag::BagItem;
    use poke_core::charmap::encode;
    use crate::command::{Command, Decision, Reply};
    use crate::input::Joypad;
    use crate::rng::GameRng;
    use crate::world::World;
    use crate::{Event, Game, Input, Pacing};
    use super::*;

    const STOCK: [ItemId; 3] = [ItemId::PokeBall, ItemId::Potion, ItemId::Antidote];

    fn game(money: [u8; 3], bag: &[(ItemId, u8)]) -> Game {
        let world = World {
            player_name: encode("RED").unwrap(),
            money,
            bag: Inventory::bag(bag.iter().map(|&(id, q)| BagItem::new(id, q)).collect()),
            ..World::default()
        };
        let mut game = Game::new(world, GameRng::seeded(0), Pacing::Faithful);
        game.push(Mode::Pokemart(Pokemart::new(STOCK.to_vec())));
        game
    }

    fn settle(game: &mut Game) -> Decision {
        for _ in 0..2000 {
            if let Status::Waiting(decision) = game.status() {
                return decision;
            }
            game.frame(Input::None);
        }
        panic!("nothing ever waited: {:?}", game.modes().last());
    }

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

    fn row(game: &Game, y: usize, x: usize, text: &str) -> (Vec<u8>, Vec<u8>) {
        let expected = encode(text).unwrap();
        (game.ui().row(y)[x..x + expected.len()].to_vec(), expected)
    }

    /// The price texts `cont` onto a second line, which is a press of its own.
    fn read_on_to(game: &mut Game, decision: Decision) {
        while settle(game) == Decision::Text {
            answer(game, Decision::Text, Command::Advance);
        }
        assert_eq!(settle(game), decision);
    }

    fn buy(game: &mut Game, entry: u8, quantity: u8) {
        answer(game, Decision::BuySellQuit, Command::ChooseOption(BUY));
        answer(game, Decision::List, Command::ChooseListEntry(entry));
        answer(game, Decision::Quantity, Command::ChooseQuantity(quantity));
        read_on_to(game, Decision::TwoOption);
        answer(game, Decision::TwoOption, Command::ChooseOption(0));
    }

    #[test]
    fn the_stock_hides_the_player_under_its_box() {
        use poke_core::map::Map;
        use poke_core::sprite::SpriteFacing;
        use crate::modes::overworld::Overworld;
        use crate::systems::overworld::Location;
        let world = World {
            player_name: encode("RED").unwrap(),
            location: Location { map: Map::PewterMart, x: 2, y: 5, facing: SpriteFacing::Left, last_map: Map::PewterCity,
                ..Location::default() },
            ..World::default()
        };
        let mut game = Game::new(world, GameRng::seeded(9), Pacing::Faithful);
        game.push(Mode::Overworld(Overworld::new()));
        assert_eq!(settle(&mut game), Decision::Overworld);
        let player = |game: &Game| match &game.modes()[0] {
            Mode::Overworld(overworld) => overworld.sprites()[0].image_index,
            _ => unreachable!(),
        };
        for _ in 0..4 {
            game.frame(Input::Buttons(Joypad::A));
        }
        read_on_to(&mut game, Decision::BuySellQuit);
        assert_ne!(player(&game), 0xFF, "the menu is clear of the player");
        answer(&mut game, Decision::BuySellQuit, Command::ChooseOption(BUY));
        assert_eq!(settle(&mut game), Decision::List);
        assert_eq!(player(&game), 0xFF, "the list covers the player's square");
    }

    #[test]
    fn buying_spends_the_money_and_fills_the_bag() {
        let mut game = game([0x00, 0x30, 0x00], &[]);
        answer(&mut game, Decision::BuySellQuit, Command::ChooseOption(BUY));
        assert_eq!(settle(&mut game), Decision::List);
        let (shown, expected) = row(&game, 5, 11, "   ¥200");
        assert_eq!(shown, expected, "a Poké Ball's price under its name, against the right");
        answer(&mut game, Decision::List, Command::ChooseListEntry(1));
        answer(&mut game, Decision::Quantity, Command::ChooseQuantity(4));
        read_on_to(&mut game, Decision::TwoOption);
        let (shown, expected) = row(&game, 16, 1, "¥1200. OK?");
        assert_eq!(shown, expected);
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(0));
        assert_eq!(settle(&mut game), Decision::Text);
        assert_eq!(game.world().money, [0x00, 0x18, 0x00]);
        assert_eq!(game.world().bag.quantity_of(ItemId::Potion), 4);
    }

    #[test]
    fn not_enough_money_is_said_before_the_bag_is_looked_at() {
        let full: Vec<(ItemId, u8)> = (1..=20).map(|i| (ItemId::from_repr(i).unwrap(), 1)).collect();
        let mut game = game([0x00, 0x01, 0x00], &full);
        buy(&mut game, 0, 1);
        assert_eq!(settle(&mut game), Decision::Text);
        let (shown, expected) = row(&game, 14, 1, "You don't have");
        assert_eq!(shown, expected);
        answer(&mut game, Decision::Text, Command::Advance);
        assert_eq!(settle(&mut game), Decision::BuySellQuit, "anything else, and the menu again");
    }

    #[test]
    fn a_full_bag_refuses_a_new_item_and_keeps_the_money() {
        let full: Vec<(ItemId, u8)> = (21..=40).map(|i| (ItemId::from_repr(i).unwrap(), 1)).collect();
        let mut game = game([0x00, 0x50, 0x00], &full);
        buy(&mut game, 2, 1);
        assert_eq!(settle(&mut game), Decision::Text);
        let (shown, expected) = row(&game, 14, 1, "You can't carry");
        assert_eq!(shown, expected);
        assert_eq!(game.world().money, [0x00, 0x50, 0x00]);
    }

    #[test]
    fn selling_pays_half_and_a_key_item_has_no_price() {
        let mut game = game([0; 3], &[(ItemId::TownMap, 1), (ItemId::Nugget, 3)]);
        answer(&mut game, Decision::BuySellQuit, Command::ChooseOption(SELL));
        answer(&mut game, Decision::List, Command::ChooseListEntry(1));
        answer(&mut game, Decision::Quantity, Command::ChooseQuantity(2));
        read_on_to(&mut game, Decision::TwoOption);
        let (shown, expected) = row(&game, 16, 1, "¥10000 for that.");
        assert_eq!(shown, expected, "two Nuggets at ¥10000 sell for half of ¥20000");
        answer(&mut game, Decision::TwoOption, Command::ChooseOption(0));
        assert_eq!(game.world().money, [0x01, 0x00, 0x00], "¥10000");
        assert_eq!(game.world().bag.quantity_of(ItemId::Nugget), 1);
        answer(&mut game, Decision::List, Command::ChooseListEntry(0));
        assert_eq!(settle(&mut game), Decision::Text);
        let (shown, expected) = row(&game, 14, 1, "I can't put a");
        assert_eq!(shown, expected);
    }

    #[test]
    fn quit_says_goodbye_and_puts_the_scroll_back() {
        let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
        game.menu_mut().list_scroll = 3;
        game.push(Mode::Pokemart(Pokemart::new(STOCK.to_vec())));
        answer(&mut game, Decision::BuySellQuit, Command::ChooseOption(2));
        assert_eq!(settle_or_gone(&mut game), None);
        assert_eq!(game.menu().list_scroll, 3);
    }

    fn settle_or_gone(game: &mut Game) -> Option<Decision> {
        for _ in 0..200 {
            if game.modes().is_empty() {
                return None;
            }
            if let Status::Waiting(decision) = game.status() {
                return Some(decision);
            }
            game.frame(Input::None);
        }
        panic!("the mart never finished");
    }

    #[test]
    fn a_save_mid_purchase_resumes_identically() {
        let mut whole = game([0x00, 0x30, 0x00], &[]);
        answer(&mut whole, Decision::BuySellQuit, Command::ChooseOption(BUY));
        assert_eq!(settle(&mut whole), Decision::List);
        whole.frame(Input::Buttons(Joypad::A));
        let mut restored = Game::load(&whole.save(), Pacing::Faithful).unwrap();
        for frame in 0..60 {
            let input = || if frame % 4 == 2 { Input::Buttons(Joypad::UP) } else { Input::None };
            let (a, b) = (whole.frame(input()), restored.frame(input()));
            assert_eq!((whole.ui(), a.events), (restored.ui(), b.events), "frame {frame}");
        }
    }
}

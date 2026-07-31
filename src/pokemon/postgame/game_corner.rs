//! Workstream **F — Game Corner economy**. See `docs/postgame-coverage-plan.md` §6-F.
//!
//! No slot machines (out of scope). Coins are bought at the counter, ¥1000 → 50 coins.
//!
//! Sub-steps: F1 Coin Case (from a man in the `CeladonDiner`, not the Mart) · F2 buy coins · F3 sell
//! to a mart — the mart driver only implements Buy today, and Porygon is 9999 coins ≈ ¥200 000 ·
//! F4 redeem a prize, then `postgame-game-corner.bin`.

use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::agent::{AgentEvent, AgentState, PokemonAgent};
use crate::pokemon::encoding::GameMode;
use crate::pokemon::item::ItemId;
use crate::pokemon::BagItem;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::menu::TextBoxId;
use crate::pokemon::policy::{DeterministicPolicy, FieldMove, PolicyStep};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::tile::MetaTile;
use crate::pokemon::world_graph::WorldGraph;
use crate::pokemon::{GameState, PokemonApi, PokemonApiTrait};

/// What one visit to the coin counter costs and yields (`scripts/GameCorner.asm:141-195`).
pub const COIN_PRICE: u32 = 1000;
/// Coins handed over per ¥1000. Fixed; there is no "how many?" prompt.
pub const COINS_PER_PURCHASE: u16 = 50;
/// The Coin Case holds four BCD digits, and the clerk refuses once fewer than 9 coins would fit
/// (`Has9990Coins`).
pub const COIN_CASE_CAPACITY: u16 = 9_990;

impl PolicyStep {
    /// **F1** — the **Coin Case**, from the gym guide in the `CeladonDiner`.
    ///
    /// Not the Mart, and not a purchase: `scripts/CeladonDiner.asm:26-40` is a plain `GiveItem
    /// COIN_CASE` behind `EVENT_GOT_COIN_CASE`, so this is one `Interact` and no menu — the same shape
    /// as B1's Fan Club chairman and C1's fishing gurus. Three interacts because the give is two text
    /// boxes deep and only the first A press of an `Interact` is guaranteed to land on the script.
    ///
    /// Ends outdoors so the next leg's `Fly` is not refused for being indoors (D's Moltres leg and C's
    /// rod pickups both record that rule).
    pub fn coin_case_steps() -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::CeladonCity }, Self::enter(Map::CeladonDiner)];
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::CELADONDINER_GYM_GUIDE), 3));
        s.push(Self::enter(Map::CeladonCity));
        s
    }

    /// **F2** — buy coins at the Game Corner counter until at least `target` are held, then step back
    /// outside.
    ///
    /// ¥1000 → 50 coins, one conversation each, so `target` is also `target/50` round trips through
    /// the clerk's text. That is the whole minigame as far as this plan is concerned: §3 rules the
    /// slot machines out, and the counter reaches every prize without driving an RNG-heavy reel.
    pub fn buy_coins_steps(target: u16) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeladonCity },
            Self::enter(Map::GameCorner),
            Self::BuyGameCoins { target },
            Self::enter(Map::CeladonCity),
        ]
    }

    /// **F3** — turn the junk TMs banked in PC item storage into money at the Viridian Mart.
    ///
    /// Selling is the half of the mart the agent has never used, and this is the leg that needs it:
    /// coins are ¥20 each and the top prizes run to 9999 coins, so the only lever on the prize
    /// economy that is not "already have the money" is selling.
    ///
    /// The three TMs are the ones Phase 0 banked and nothing has a plan for
    /// (`postgame-phase0.bin`'s PC holds six). **Mega Drain**, **Fissure** and **Bide** are chosen
    /// over the other three deliberately: Toxic is a status move and workstream D's catch arithmetic
    /// makes status the only thing that matters on a catch-rate-3 legendary, Thunderbolt is the
    /// party's only electric TM, and Bubblebeam is at least a real attack. Fissure is a OHKO move,
    /// which is precisely wrong for catching, and the other two are filler.
    ///
    /// Viridian rather than Celadon because the TMs have to be *withdrawn* first and the Viridian
    /// Pokémon Center is the PC Phase 0 parked at, one map from its Mart. Ends outdoors.
    pub fn sell_junk_tms_steps() -> Vec<Self> {
        const JUNK: [ItemId; 3] = [ItemId::Tm21MegaDrain, ItemId::Tm27Fissure, ItemId::Tm34Bide];
        let mut s = vec![Self::Fly { to: Map::ViridianCity }];
        s.extend(JUNK.map(|tm| Self::withdraw_item(tm, 1, Map::ViridianPokecenter)));
        s.push(Self::enter(Map::ViridianCity));
        s.push(Self::enter(Map::ViridianMart));
        s.extend(JUNK.map(|tm| Self::SellToMart { map: Map::ViridianMart, item: BagItem::new(tm, 1) }));
        s.push(Self::enter(Map::ViridianCity));
        s
    }

    /// **F4** — buy `prize` from the Game Corner prize room, topping the Coin Case up first.
    ///
    /// The `BuyGameCoins` step in front is what makes this composable: it is a no-op when the coins
    /// are already there and otherwise buys exactly as many as the prize needs, so a caller only has
    /// to name the prize. Ends outdoors.
    pub fn redeem_prize_steps(prize: crate::pokemon::postgame::game_corner::Prize) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeladonCity },
            Self::enter(Map::GameCorner),
            Self::BuyGameCoins { target: prize.cost() },
            Self::enter(Map::CeladonCity),
            Self::enter(Map::GameCornerPrizeRoom),
            Self::RedeemPrize { prize },
            Self::enter(Map::CeladonCity),
        ]
    }
}

/// One poll of [`PolicyStep::BuyGameCoins`], from the single delegating arm in `policy.rs`.
///
/// `Some(action)` is what the policy should do (possibly `None` = wait); the outer `None` means the
/// step is finished and should be popped.
///
/// Everything here is a *pre*-check, because every failure mode of the counter looks identical from
/// the outside: no Coin Case, not enough money and a full case all print one text box and return, so
/// a caller that just kept talking to the clerk would loop until its budget ran out. The clerk is
/// `GAMECORNER_CLERK1` — sprite name `"Clerk 1"` — and the *other* clerk on the same map (`"Clerk 2"`,
/// at (14,11)) only ever says how the prizes work.
pub fn buy_coins_action(
    state: &GameState,
    actions: &[OverworldAction],
    world_graph: &WorldGraph,
    target: u16,
) -> Option<Option<OverworldAction>> {
    if state.coins >= target {
        println!("[policy] BuyGameCoins: {} coins ≥ {target} — done", state.coins);
        return None;
    }
    if !state.bag.iter().any(|i| i.id == crate::pokemon::item::ItemId::CoinCase) {
        println!("[policy] BuyGameCoins: no Coin Case — the counter will refuse; skipping");
        return None;
    }
    if state.coins > COIN_CASE_CAPACITY {
        println!("[policy] BuyGameCoins: the Coin Case is full at {} coins", state.coins);
        return None;
    }
    if state.money < COIN_PRICE {
        println!("[policy] BuyGameCoins: ¥{} is not enough for another {COINS_PER_PURCHASE} coins",
            state.money);
        return None;
    }

    if state.map.map != Map::GameCorner {
        let action = DeterministicPolicy::route_toward(world_graph, actions, Map::GameCorner);
        if action.is_none() {
            println!("[policy] want to buy coins, but no path to the Game Corner!");
            return None;
        }
        return Some(action);
    }

    // On the map: talk to the counter clerk. Not popped on issue — the step is re-polled after every
    // purchase and stops itself on the coin count, so one dropped A press costs a lap, not the leg.
    Some(actions.iter()
        .find(|a| matches!(a.tile, MetaTile::Sprite(sprite) if sprite == MapSprite::GAMECORNER_CLERK1.name))
        .cloned())
}

// ═════════════════════════════════════════════════════════════════════════════════════════════════
// F3 — selling to a mart
// ═════════════════════════════════════════════════════════════════════════════════════════════════

/// Live state of an in-progress sale. Carried in [`AgentState::SellingToMart`].
///
/// The sale is its own driver rather than a branch of [`PokemartState`](crate::pokemon::agent) for
/// one blunt reason: `assert_pokemart_state` hands the Buy/Sell/Quit menu to a state machine that
/// only knows how to *buy*, so a sale that let it take over would answer its own menu with BUY. The
/// agent's exclusion list keeps this state's hands on that menu instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SellState {
    /// What to sell, and how many.
    pub item: BagItem,
    /// The tile the player must **face** to open the shop, and the direction to face it from.
    ///
    /// Not the clerk's own tile: a mart clerk stands behind a `Counter`, and pokered looks *through*
    /// a counter tile to the sprite on the far side. `route_to_face_dir` knows nothing about that, so
    /// asking it to face the clerk directly finds no reachable adjacent tile at all — which is
    /// exactly how the Viridian clerk at (0,5) came back as "can't reach". [`pick_sale`] resolves the
    /// counter-mediated standing position out of `MetaTileMap::actions()`, which does model it, and
    /// hands the driver the tile in front of that position.
    pub clerk: Point8,
    /// Which way to face `clerk` from — pinned so the driver cannot pick a different (and, across a
    /// counter, wrong) side on a later tick.
    facing: crate::pokemon::map_metadata::PlayerFacingDirection,
    /// Quantity held when the sale began — completion is measured against this, so selling 3 of a
    /// stack of 9 is detected as precisely as selling the lot.
    start_qty: u8,
    /// Press/release alternation, so every input is a fresh rising edge.
    press: bool,
    /// Set once the shop menu has been opened, i.e. we have left the overworld at least once.
    entered_menu: bool,
    /// Ticks spent driving, so a wedge reports itself instead of pulsing buttons for the whole budget.
    ticks: u16,
}

/// Ceiling on driver ticks for one sale — the greeting, four menus and the closing text are ~150.
const TICK_BUDGET: u16 = 1200;

impl SellState {
    pub fn new(item: BagItem, clerk: (Point8, crate::pokemon::map_metadata::PlayerFacingDirection), api: &PokemonApi<'_>) -> Self {
        Self {
            item,
            clerk: clerk.0,
            facing: clerk.1,
            start_qty: api.bag_item_quantity(item.id),
            press: true,
            entered_menu: false,
            ticks: 0,
        }
    }

    /// Why this sale cannot happen, if it cannot — checked before any menu is opened, because the
    /// clerk answers all three with one text box and a bounce back to the Buy/Sell/Quit menu, which a
    /// driver would then re-drive for ever.
    fn blocked_by(&self) -> Option<String> {
        if self.start_qty < self.item.quantity {
            return Some(format!("the bag holds {} {:?}, not {}", self.start_qty, self.item.id, self.item.quantity));
        }
        if self.item.id.is_key_item() {
            return Some(format!("{:?} is a key item — \"I can't put a PRICE on that!\"", self.item.id));
        }
        if self.item.id.is_hm() {
            return Some(format!("{:?} is an HM, which pokered's `IsItemHM` refuses to buy", self.item.id));
        }
        None
    }
}

/// `wListMenuID` while the choose-quantity box is open in the **sell** flow (`PRICEDITEMLISTMENU`).
///
/// This is the only signal that separates the quantity box from the bag list underneath it: both are
/// drawn with `wTextBoxID = ListMenuBox`, and `mart_in_quantity_selector()` cannot help because it
/// keys on `wMaxItemQuantity == 99`, which only the **buy** path writes (the sell path's maximum is
/// the size of the stack). `.sellMenuLoop` resets it to `ITEMLISTMENU` before each list, so the two
/// states are cleanly distinguishable (`engine/events/pokemart.asm:47-86`, `home/list_menu.asm:197`).
const PRICED_ITEM_LIST_MENU: u8 = 2;

/// Which sprite on this map takes money. Single-counter marts name their seller `Clerk`; the Celadon
/// department store's floors have `Clerk 1` (items) and `Clerk 2` (TMs) — either will buy, so the
/// first found is fine, unlike buying, where the two stock different things.
fn is_clerk(name: &str) -> bool {
    matches!(name, "Clerk" | "Clerk 1" | "Clerk 2")
}

/// Hand [`PolicyStep::SellToMart`] to the driver, once we are standing on the right map.
///
/// The walk is resolved here rather than in the driver because only `MetaTileMap::actions()` models
/// **counter-mediated** interaction: a mart clerk stands behind a `Counter` tile and pokered reaches
/// *through* it, so the player's standing position is two tiles from the sprite, not one. This takes
/// that position from `actions()` and converts it into the "face this tile, from this side" pair the
/// driver's `route_to_face_dir` wants — which stays correct if a battle or a script moves the player
/// mid-walk, whereas a stored button route would not.
pub fn pick_sale(state: &GameState, item: BagItem) -> Option<FieldMove> {
    use crate::pokemon::map_metadata::PlayerFacingDirection;

    let Some(clerk) = state.map.sprites.iter().find(|s| !s.hidden && is_clerk(&s.name)) else {
        println!("[policy] SellToMart: no clerk on {} — skipping", state.map.map);
        return None;
    };
    let Some(action) = state.map.actions().into_iter()
        .find(|a| a.tile == MetaTile::Sprite(clerk.name)) else {
        println!("[policy] SellToMart: the clerk on {} is not reachable — skipping", state.map.map);
        return None;
    };

    // The approach is axis-aligned in both cases `actions()` produces (directly adjacent, or one
    // further back across a counter), so the sign of the delta *is* the facing.
    let stand = action.destination;
    let (dx, dy) = (clerk.position.x as i16 - stand.x as i16, clerk.position.y as i16 - stand.y as i16);
    let facing = match (dx.signum(), dy.signum()) {
        (0, -1) => PlayerFacingDirection::Up,
        (0, 1) => PlayerFacingDirection::Down,
        (-1, 0) => PlayerFacingDirection::Left,
        (1, 0) => PlayerFacingDirection::Right,
        _ => {
            println!("[policy] SellToMart: clerk at {} is not aligned with {stand} — skipping", clerk.position);
            return None;
        }
    };
    let face_tile = match facing {
        PlayerFacingDirection::Up => Point8 { x: stand.x, y: stand.y.saturating_sub(1) },
        PlayerFacingDirection::Down => Point8 { x: stand.x, y: stand.y + 1 },
        PlayerFacingDirection::Left => Point8 { x: stand.x.saturating_sub(1), y: stand.y },
        PlayerFacingDirection::Right => Point8 { x: stand.x + 1, y: stand.y },
    };
    Some(FieldMove::SellToMart { item, clerk: (face_tile, facing) })
}

/// One agent tick of the sell driver. Called from `agent.rs` via a single delegating match arm.
///
/// # The menu chain
///
/// Read out of `engine/events/pokemart.asm:36-115` rather than guessed:
///
/// ```text
/// overworld     walk up to the clerk, face them, press A
///   → "May I help you?"                          mash A
///   → BUY / SELL / QUIT                          cursor → 1, A
///   → "What should I buy off you?" + bag list    cursor → the item's row, A
///   → the ×quantity / price box                  Up/Down to the quantity, A
///   → "I can pay ¥N. Is that OK?"                YES (index 0)
///   → "…" and back to the **bag list**           B out, then B again on BUY/SELL/QUIT
/// ```
///
/// The one thing worth knowing beyond that: a sale does **not** return to the Buy/Sell/Quit menu, it
/// returns to the bag list (`jp .sellMenuLoop`), so "done" has to be read from the bag rather than
/// from a screen — and B is the only way back out.
pub fn sell_tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: SellState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    let sold = s.start_qty.saturating_sub(api.bag_item_quantity(s.item.id));

    let abort = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, why: String| {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("sell: {why}") });
        agent.set_state(AgentState::Idle);
    };

    // ── Done: the requested quantity has left the bag ────────────────────────────────────────────
    if s.entered_menu && sold >= s.item.quantity {
        if game_mode != GameMode::Overworld {
            // The sale drops back to the bag list, so back out with B — twice, since the
            // Buy/Sell/Quit menu is between the list and the door.
            api.release_all_buttons();
            if s.press { api.press_button(JoypadButton::B); }
            agent.set_state(AgentState::SellingToMart(SellState { press: !s.press, ticks: s.ticks + 1, ..s }));
            return Ok(());
        }
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("sold {}x {:?}", s.item.quantity, s.item.id) });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    if !s.entered_menu {
        if let Some(why) = s.blocked_by() {
            abort(agent, api, format!("can't sell {:?} — {why}", s.item.id));
            return Ok(());
        }
    }
    if s.ticks > TICK_BUDGET {
        abort(agent, api, format!("{:?} made no progress in {TICK_BUDGET} ticks", s.item.id));
        return Ok(());
    }

    // ── Back outside having sold nothing — the attempt fizzled; let the policy carry on ──────────
    if s.entered_menu && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // ── Still outside: walk up to the clerk and press A ──────────────────────────────────────────
    if game_mode == GameMode::Overworld {
        let gs = agent.observe_state(api)?;
        match gs.map.route_to_face_dir(s.clerk, Some(s.facing)).as_deref() {
            Some([]) => {
                api.release_all_buttons();
                if s.press { api.press_button(JoypadButton::A); }
                agent.set_state(AgentState::SellingToMart(SellState { press: !s.press, ticks: s.ticks + 1, ..s }));
            }
            Some(&[btn, ..]) => {
                api.release_all_buttons();
                api.press_button(btn);
                agent.set_state(AgentState::SellingToMart(SellState { press: true, ticks: s.ticks + 1, ..s }));
            }
            _ => abort(agent, api, format!("can't reach the clerk at {}", s.clerk)),
        }
        return Ok(());
    }

    // ── Inside the menus ────────────────────────────────────────────────────────────────────────
    let s = SellState { entered_menu: true, ticks: s.ticks + 1, ..s };
    if !s.press {
        api.release_all_buttons();
        agent.set_state(AgentState::SellingToMart(SellState { press: true, ..s }));
        return Ok(());
    }

    let (_, _, cursor, scroll) = api.menu_geometry();
    let menu = api.menu_state();
    let tbid = menu.map(|m| m.text_box_id);
    let list_menu_id = api.mmu().read_pointer(&pokered_symbols::wListMenuID);
    let nav = |cur: u8, target: u8| -> JoypadButton {
        if cur < target { JoypadButton::Down }
        else if cur > target { JoypadButton::Up }
        else { JoypadButton::A }
    };

    let button = if tbid == Some(TextBoxId::TwoOptionMenu) {
        nav(cursor, 0) // "I can pay ¥N. Is that OK?" → YES
    } else if tbid == Some(TextBoxId::ListMenuBox) && list_menu_id == PRICED_ITEM_LIST_MENU {
        // The ×quantity box, drawn *over* the bag list without changing `wTextBoxID`. Read the live
        // `wItemQuantity` rather than counting presses, so a dropped input costs a tick, not the sale.
        let shown = api.mart_item_quantity();
        if shown < s.item.quantity { JoypadButton::Up }
        else if shown > s.item.quantity { JoypadButton::Down }
        else { JoypadButton::A }
    } else if tbid == Some(TextBoxId::ListMenuBox) {
        // The bag list. Addressed by raw `wBagItems` position, never by `GameState::bag`, which drops
        // every id `ItemId` cannot name and so mis-numbers the rows (§10 of the plan).
        match api.bag_item_position(s.item.id) {
            Some(row) => nav(cursor + scroll, row),
            None => JoypadButton::B,
        }
    } else if menu.map_or(false, |m| m.is_mart_buy_sell_menu()) {
        nav(cursor, 1) // BUY 0 / SELL 1 / QUIT 2
    } else {
        JoypadButton::A // greeting, "What should I buy off you?", the closing text
    };

    api.release_all_buttons();
    api.press_button(button);
    agent.set_state(AgentState::SellingToMart(SellState { press: false, ..s }));
    Ok(())
}

// ═════════════════════════════════════════════════════════════════════════════════════════════════
// F4 — the prize room
// ═════════════════════════════════════════════════════════════════════════════════════════════════

/// The nine Game Corner prizes, as they appear in **Red** (`data/events/prizes.asm`).
///
/// Blue swaps three of them — Nidorino for Nidorina and Pinsir for Scyther, at different prices — so
/// this table is version-specific, like everything else keyed off `_RED`.
///
/// Three separate vendors sell them, one per bg-event tile along the counter, and each shows a
/// **three-entry menu plus NO THANKS**. Which of the three a vendor shows is decided by the *text id*
/// that triggered it (`GetPrizeMenuId` subtracts `TEXT_GAMECORNERPRIZEROOM_PRIZE_VENDOR_1` from
/// `hTextID`), so the tile you stand in front of is the menu you get.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prize {
    Abra,
    Clefairy,
    Nidorina,
    Dratini,
    Scyther,
    Porygon,
    /// TM23 Dragon Rage.
    DragonRage,
    /// TM15 Hyper Beam.
    HyperBeam,
    /// TM50 Substitute.
    Substitute,
}

impl Prize {
    /// Which vendor sells it (0, 1 or 2 — mons, rarer mons, TMs) and its row in that vendor's menu.
    const fn window_and_row(self) -> (u8, u8) {
        match self {
            Self::Abra => (0, 0), Self::Clefairy => (0, 1), Self::Nidorina => (0, 2),
            Self::Dratini => (1, 0), Self::Scyther => (1, 1), Self::Porygon => (1, 2),
            Self::DragonRage => (2, 0), Self::HyperBeam => (2, 1), Self::Substitute => (2, 2),
        }
    }

    /// The vendor's bg-event tile (`data/maps/objects/GameCornerPrizeRoom.asm`): (2,2), (4,2), (6,2).
    pub const fn vendor_tile(self) -> Point8 {
        Point8 { x: 2 + 2 * self.window_and_row().0, y: 2 }
    }

    /// Cursor row in the prize menu. Row 3 is `NO THANKS`, which this never selects.
    pub const fn menu_row(self) -> u8 { self.window_and_row().1 }

    /// Price in coins.
    pub const fn cost(self) -> u16 {
        match self {
            Self::Abra => 180, Self::Clefairy => 500, Self::Nidorina => 1200,
            Self::Dratini => 2800, Self::Scyther => 5500, Self::Porygon => 9999,
            Self::DragonRage => 3300, Self::HyperBeam => 5500, Self::Substitute => 7700,
        }
    }

    /// The species, for the six prizes that are Pokémon. `None` for the three TMs, which take the
    /// `GiveItem` branch of `HandlePrizeChoice` instead of `GivePokemon`.
    pub const fn species(self) -> Option<PokemonSpecies> {
        match self {
            Self::Abra => Some(PokemonSpecies::Abra),
            Self::Clefairy => Some(PokemonSpecies::Clefairy),
            Self::Nidorina => Some(PokemonSpecies::Nidorina),
            Self::Dratini => Some(PokemonSpecies::Dratini),
            Self::Scyther => Some(PokemonSpecies::Scyther),
            Self::Porygon => Some(PokemonSpecies::Porygon),
            _ => None,
        }
    }
}

/// Live state of an in-progress prize purchase. Carried in [`AgentState::RedeemingPrize`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrizeState {
    pub prize: Prize,
    /// Coins held when the purchase began. The price leaves the Coin Case only after the prize has
    /// been handed over (`HandlePrizeChoice` subtracts *last*, and returns early without charging if
    /// the party and box were both full), so a drop is proof of delivery for all nine prizes —
    /// including the three TMs, where no party count moves.
    start_coins: u16,
    press: bool,
    entered_menu: bool,
    ticks: u16,
}

impl PrizeState {
    pub fn new(prize: Prize, api: &PokemonApi<'_>) -> Self {
        Self { prize, start_coins: read_coins(api), press: true, entered_menu: false, ticks: 0 }
    }
}

/// `wPlayerCoins`, two BCD bytes.
fn read_coins(api: &PokemonApi<'_>) -> u16 {
    crate::pokemon::encoding::reverse_bcd(
        api.mmu().read_pointer_u16_be(&pokered_symbols::wPlayerCoins) as u32) as u16
}

/// Hand [`PolicyStep::RedeemPrize`] to the driver, once we are standing in the prize room.
///
/// The affordability check is here rather than in the driver because `HandlePrizeChoice` answers
/// "not enough coins" with a text box and a return to the overworld — indistinguishable, from
/// outside, from a purchase that worked.
pub fn pick_prize(state: &GameState, prize: Prize) -> Option<FieldMove> {
    if state.coins < prize.cost() {
        println!("[policy] RedeemPrize: {prize:?} costs {} coins and only {} are held — skipping",
            prize.cost(), state.coins);
        return None;
    }
    Some(FieldMove::RedeemPrize { prize })
}

/// One agent tick of the prize-room driver. Called from `agent.rs` via a single delegating match arm.
///
/// # The menu chain
///
/// From `engine/events/prize_menu.asm`:
///
/// ```text
/// overworld     walk below the vendor's tile, face it, press A
///   → "Welcome! … exchange your coins for prizes."   mash A
///   → the prize menu, three prizes + NO THANKS       cursor → the row, A
///   → "So! You want <PRIZE>?"                        YES (index 0)
///   → "Here you go!" / the mon's arrival text        mash A until the overworld returns
/// ```
///
/// # Why the menu is matched on the words "NO THANKS"
///
/// `CeladonPrizeMenu` draws its box with `TextBoxBorder` directly and **never writes `wTextBoxID`**,
/// so the id on that screen is whatever the last real text box left behind — the same class of trap
/// as B's town map, one notch less severe. What it does do is `PlaceString` a fixed `NO THANKS` on
/// the fourth row, so the screen text is the reliable signal. The prices sit in the same box, so
/// matching on a prize *name* would also work but would need a name table; `NO THANKS` is one string
/// for all three vendors.
pub fn prize_tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: PrizeState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    let coins = read_coins(api);

    let abort = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, why: String| {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("prize: {why}") });
        agent.set_state(AgentState::Idle);
    };

    // ── Done: the coins have been spent, so the prize has been handed over ───────────────────────
    if s.entered_menu && coins < s.start_coins {
        if game_mode != GameMode::Overworld {
            api.release_all_buttons();
            if s.press { api.press_button(JoypadButton::A); }
            agent.set_state(AgentState::RedeemingPrize(PrizeState { press: !s.press, ticks: s.ticks + 1, ..s }));
            return Ok(());
        }
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox {
            message: format!("bought {:?} for {} coins ({coins} left)", s.prize, s.start_coins - coins),
        });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    if s.ticks > TICK_BUDGET {
        abort(agent, api, format!("{:?} made no progress in {TICK_BUDGET} ticks", s.prize));
        return Ok(());
    }

    // ── Back outside having bought nothing — the attempt fizzled; let the policy carry on ────────
    if s.entered_menu && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // ── Still outside: walk to the vendor's counter tile and press A ─────────────────────────────
    if game_mode == GameMode::Overworld {
        let gs = agent.observe_state(api)?;
        let tile = s.prize.vendor_tile();
        match gs.map.route_to_face_dir(tile, None).as_deref() {
            Some([]) => {
                api.release_all_buttons();
                if s.press { api.press_button(JoypadButton::A); }
                agent.set_state(AgentState::RedeemingPrize(PrizeState { press: !s.press, ticks: s.ticks + 1, ..s }));
            }
            Some(&[btn, ..]) => {
                api.release_all_buttons();
                api.press_button(btn);
                agent.set_state(AgentState::RedeemingPrize(PrizeState { press: true, ticks: s.ticks + 1, ..s }));
            }
            _ => abort(agent, api, format!("can't reach the {:?} vendor at {tile}", s.prize)),
        }
        return Ok(());
    }

    // ── Inside the menus ────────────────────────────────────────────────────────────────────────
    let s = PrizeState { entered_menu: true, ticks: s.ticks + 1, ..s };
    if !s.press {
        api.release_all_buttons();
        agent.set_state(AgentState::RedeemingPrize(PrizeState { press: true, ..s }));
        return Ok(());
    }

    let (_, _, cursor, _) = api.menu_geometry();
    let text = api.on_screen_text(false).unwrap_or_default();
    let tbid = api.menu_state().map(|m| m.text_box_id);
    let nav = |cur: u8, target: u8| -> JoypadButton {
        if cur < target { JoypadButton::Down }
        else if cur > target { JoypadButton::Up }
        else { JoypadButton::A }
    };

    // The prize list is checked *before* the yes/no, because `wTextBoxID` still reads `TwoOptionMenu`
    // from an earlier box while the prize box is drawn over it — the id is only written when a text
    // box is drawn, and this screen never draws one.
    let button = if text.contains("NO THANKS") {
        nav(cursor, s.prize.menu_row())
    } else if tbid == Some(TextBoxId::TwoOptionMenu) {
        nav(cursor, 0) // "So! You want …?" → YES
    } else {
        JoypadButton::A // the greeting, "Here you go!", and the mon's arrival text
    };

    api.release_all_buttons();
    api.press_button(button);
    agent.set_state(AgentState::RedeemingPrize(PrizeState { press: false, ..s }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmu::MMU;

    /// Both hand-written tables in this module, checked against the ROM they were transcribed from.
    ///
    /// They are the kind that fail *plausibly*: a wrong prize price buys the wrong prize, and a wrong
    /// key-item flag sends the sell driver into a menu that will never accept, which reads as a driver
    /// bug rather than a data one. Addressed through the generated symbols rather than raw offsets, so
    /// the symbol names are pinned too.
    fn rom() -> MMU { MMU::from_rom(crate::pokemon::roms::POKERED).unwrap() }

    /// `data/events/prizes.asm`: three vendors, three prizes each — a species/item byte and a `bcd2`
    /// price, in parallel arrays.
    #[test]
    fn prize_table_matches_the_rom() {
        let mmu = rom();
        let windows = [
            (pokered_symbols::PrizeMenuMon1Entries, pokered_symbols::PrizeMenuMon1Cost),
            (pokered_symbols::PrizeMenuMon2Entries, pokered_symbols::PrizeMenuMon2Cost),
            (pokered_symbols::PrizeMenuTMsEntries, pokered_symbols::PrizeMenuTMsCost),
        ];
        const ALL: [Prize; 9] = [Prize::Abra, Prize::Clefairy, Prize::Nidorina,
                                 Prize::Dratini, Prize::Scyther, Prize::Porygon,
                                 Prize::DragonRage, Prize::HyperBeam, Prize::Substitute];

        for prize in ALL {
            let (window, row) = prize.window_and_row();
            let (entries, costs) = windows[window as usize];

            let cost = crate::pokemon::encoding::reverse_bcd(
                mmu.read_pointer_u16_be(&(costs + 2 * row as u16)) as u32) as u16;
            assert_eq!(cost, prize.cost(), "{prize:?} price");

            let id = mmu.read_pointer(&(entries + row as u16));
            match prize.species() {
                Some(species) => assert_eq!(PokemonSpecies::from_repr(id), Some(species), "{prize:?} species"),
                // The TM window holds item ids and TM01 is `$c9`; no species byte reaches that high.
                None => assert!(id >= ItemId::Hm01Cut as u8, "{prize:?} should be a machine id, got ${id:02x}"),
            }
            // Each vendor's list is `db …, "@"`, so row 3 is the terminator — `$50` in pokered's
            // charmap, not ASCII. `NO THANKS` is placed by the menu code rather than read from here,
            // which is why [`prize_tick`] can match the screen on it for all three vendors.
            assert_eq!(mmu.read_pointer(&(entries + 3)), 0x50, "vendor {window} list terminator");
        }
    }

    /// `data/items/key_items.asm` — the bit array `IsKeyItem_` tests, one bit per item id from 1.
    ///
    /// Worth a ROM comparison rather than a spot check because the set is not guessable from the
    /// names (the fossils and the fishing rods are key items; the Nugget and the Poké Doll are not),
    /// and a wrong bit only shows up as a sale that silently never completes.
    #[test]
    fn key_item_predicate_matches_the_rom_bit_array() {
        let mmu = rom();
        // `IsKeyItem_` does `dec a` then `FlagAction FLAG_TEST`, so item id *n* is bit `n-1`,
        // LSB-first within each byte (`dbit`: `value |= bit << (length % 8)`).
        let is_key_in_rom = |id: u8| -> bool {
            let index = id as u16 - 1;
            mmu.read_pointer(&(pokered_symbols::KeyItemFlags + index / 8)) & (1 << (index % 8)) != 0
        };

        for raw in 1u8..=0xFA {
            let Some(item) = ItemId::from_repr(raw) else { continue };
            // The array is never consulted for `HM01` and above: `IsKeyItem_` falls through to
            // `IsItemHM` there, so HMs are unsellable and TMs are not, whatever bits follow the table.
            let expected = if raw >= ItemId::Hm01Cut as u8 { false } else { is_key_in_rom(raw) };
            assert_eq!(item.is_key_item(), expected, "{item:?} (${raw:02x}) key-item flag");
        }

        assert!(ItemId::Hm01Cut.is_hm() && ItemId::Hm05Flash.is_hm());
        assert!(!ItemId::Tm06Toxic.is_hm() && !ItemId::PokeBall.is_hm());
        // The four that make the point, in both directions.
        assert!(ItemId::HelixFossil.is_key_item() && ItemId::SuperRod.is_key_item());
        assert!(!ItemId::Nugget.is_key_item() && !ItemId::PokeDoll.is_key_item());
    }
}

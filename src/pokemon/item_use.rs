//! Whether the game will do anything at all when an item is chosen with USE, read out of the
//! cartridge's own `ItemUsePtrTable`.
//!
//! `UseItem_` (`engine/items/item_effects.asm`) dispatches on the item id through a table of effect
//! routines, and a great many of them are `UnusableItem`, which is `jp ItemUseNotTime`: it prints
//! "This isn't the time to use that!" and **hands the bag list straight back with the cursor where
//! it was**. That is the closed-loop-under-A shape this agent keeps meeting, and
//! [`AgentState::UsingFieldItem`](crate::pokemon::agent) has no exit from it: its only completion is
//! "we are back in the overworld", which a refusal never reaches. The attempt is 60 s of A-mashing
//! ended by `DRIVER_ESCAPE_SILENCE`, after which the model, told only that the game stopped
//! answering, asks for the identical use again.
//!
//! ⚠️ **The deployed run of 2026-08-27 lived in that loop**, on `MtMoonB2F` with the Helix Fossil: a
//! Team Rocket grunt says "If you find a fossil, give it to me and scram!", which is flavour and not
//! a handoff, and the model read it as an instruction and spent turn after turn alternating between
//! talking to him and a 60 s `use_item HelixFossil` that could never work. Every fossil, every
//! badge, the Silph Scope, the Lift Key, the S.S. Ticket, the Gold Teeth, the Secret Key, the
//! Bike Voucher, the Nugget, the Coin and the Exp. All are all `UnusableItem`.
//!
//! Read rather than transcribed, for [`crate::pokemon::learnset`]'s reason: the set is not guessable
//! from the names. The **Card Key** *is* usable (`ItemUseCardKey`), the Poké Flute is
//! (`ItemUsePokeFlute`) and the Coin Case is, while the Silph Scope and the Lift Key sitting right
//! beside them in the bag are not, and a second copy of that list is a second place to be wrong.

use crate::pokemon::item::ItemId;
use crate::pokemon::rom_gfx::rom_slice;
use crate::pokemon::symbols::pokered_symbols;

/// The last item id `ItemUsePtrTable` has a row for: `MAX_ELIXER`, `$53`.
///
/// ⚠️ **The table stops there and the machines are handled *before* it.** `UseItem_` does
/// `cp HM01 / jp nc, ItemUseTMHM` on the way in, so ids from `$C4` up never reach the table at all
/// and indexing it with one reads whatever `rgbasm` laid down next. Anything outside `1..=$53` is
/// therefore `None` rather than a lookup.
const LAST_TABLED_ITEM: u8 = ItemId::MaxElixer as u8;

/// The address of the effect routine `UseItem_` would `jp` to for `item`, or `None` for an id the
/// table does not cover.
pub fn use_effect(item: ItemId) -> Option<u16> {
    let id = item as u8;
    if id == 0 || id > LAST_TABLED_ITEM {
        return None;
    }
    let table = rom_slice(pokered_symbols::ItemUsePtrTable);
    let at = (id as usize - 1) * 2;
    Some(u16::from_le_bytes([table[at], table[at + 1]]))
}

/// Whether choosing USE on `item` can only ever answer "This isn't the time to use that!".
pub fn never_usable(item: ItemId) -> bool {
    use_effect(item) == Some(pokered_symbols::UnusableItem.address)
}

/// Whether `item` is one of the four Poké Balls, which `ItemUseBall` refuses outside a battle
/// (`ld a, [wIsInBattle] / and a / jp z, ItemUseNotTime`) by the same `ItemUseNotTime` that answers
/// an unusable one, and so wedges the same driver in the same way.
pub fn is_ball(item: ItemId) -> bool {
    matches!(item, ItemId::MasterBall | ItemId::UltraBall | ItemId::GreatBall | ItemId::PokeBall | ItemId::SafariBall)
}

/// What to say when `use_field_move`'s `use_item` is aimed at something the game will not use, or
/// `None` when the use is worth attempting.
///
/// ⚠️ **It names the alternative rather than the refusal**, which is
/// [`crate::pokemon::learnset::teach_refusal`]'s rule and the same lesson "it was interrupted"
/// taught: a model told only that something failed re-issues it. Each arm therefore answers the
/// decision that is actually on the table, and for the unusable items that decision is "stop trying
/// and carry it", which nothing in the situation would otherwise say.
///
/// ⚠️ **No em dashes**: this goes to the model as a tool refusal and onto the page as a `TextBox`.
pub fn field_use_refusal(item: ItemId) -> Option<String> {
    if item.is_hm() || is_machine(item) {
        return Some(format!(
            "{item} is a machine, and `use_item` cannot teach one: USE on a TM or an HM opens the \
             party menu, which this action does not drive. Use `use_field_move` with \
             `move: \"teach\"`, the `item` and the party `slot` that is to learn it."
        ));
    }
    if is_ball(item) {
        return Some(format!(
            "{item} is a Poké Ball and the game refuses one outside a battle. Throw it on a battle \
             turn with `choose_battle_action` instead."
        ));
    }
    if never_usable(item) {
        return Some(format!(
            "The game has no bag use for {item}. Choosing USE on it prints \"This isn't the time to \
             use that!\" and hands the bag straight back, every time, wherever you are standing and \
             whoever you are facing. It is an item to carry, not to use: whatever wants it takes it \
             from you in its own scene, so keep it in the bag and get on with something else."
        ));
    }
    None
}

/// TM01-TM50 (`$C9`-`$FA`). [`ItemId::is_hm`] covers `$C4`-`$C8`; between them they are every id
/// `UseItem_` sends to `ItemUseTMHM` before the table is consulted.
fn is_machine(item: ItemId) -> bool {
    (item as u8) >= 0xC9
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case the deployed run wedged on, and the neighbours that make the list unguessable.
    #[test]
    fn the_rom_says_which_items_have_no_bag_use() {
        assert!(never_usable(ItemId::HelixFossil), "the fossil the deployed run looped on");
        assert!(never_usable(ItemId::DomeFossil));
        assert!(never_usable(ItemId::OldAmber));
        assert!(never_usable(ItemId::SilphScope), "carried past Pokémon Tower, never used");
        assert!(never_usable(ItemId::LiftKey));
        assert!(never_usable(ItemId::SSTicket));
        assert!(never_usable(ItemId::GoldTeeth));
        assert!(never_usable(ItemId::SecretKey));
        assert!(never_usable(ItemId::BikeVoucher));
        assert!(never_usable(ItemId::Nugget));
        assert!(never_usable(ItemId::Coin));
        assert!(never_usable(ItemId::ExpAll));
        assert!(never_usable(ItemId::ThunderBadge), "the badge ids that are only badges");
        assert!(never_usable(ItemId::EarthBadge));
        // ⚠️ The two exceptions, and the reason this is a ROM read: item ids `$15` and `$16` are
        // the Safari Zone's BAIT and ROCK as well as the first two badges, so those two rows are
        // `ItemUseBait`/`ItemUseRock` rather than `UnusableItem`. Neither can turn up in a bag
        // (`wObtainedBadges` is a bitfield, not bag slots), so nothing has to be done about it,
        // but a transcribed "the badges are unusable" would have been wrong here.
        assert!(!never_usable(ItemId::BoulderBadge), "$15 is also SAFARI_BAIT");
        assert!(!never_usable(ItemId::CascadeBadge), "$16 is also SAFARI_ROCK");

        // The half that makes this a ROM read rather than a list: these look exactly like the ones
        // above and are perfectly usable.
        assert!(!never_usable(ItemId::CardKey), "ItemUseCardKey");
        assert!(!never_usable(ItemId::PokeFlute), "ItemUsePokeFlute");
        assert!(!never_usable(ItemId::CoinCase), "ItemUseCoinCase");
        assert!(!never_usable(ItemId::Itemfinder));
        assert!(!never_usable(ItemId::OaksParcel));
        assert!(!never_usable(ItemId::Bicycle));
        assert!(!never_usable(ItemId::TownMap));
        assert!(!never_usable(ItemId::EscapeRope));
        assert!(!never_usable(ItemId::Potion));
        assert!(!never_usable(ItemId::MoonStone));
    }

    /// ⚠️ The machines are dispatched before the table, so a lookup that indexed it with one would
    /// read past the end of the rows and answer from whatever follows.
    #[test]
    fn a_machine_never_reaches_the_table() {
        assert_eq!(use_effect(ItemId::Hm01Cut), None);
        assert_eq!(use_effect(ItemId::Tm34Bide), None);
        assert!(!never_usable(ItemId::Tm34Bide), "not usable, but not `UnusableItem` either");
        assert!(use_effect(ItemId::MaxElixer).is_some(), "the last row the table has");
    }

    /// The refusals are the whole point of the gate, so they are checked as prose: each has to name
    /// what to do instead, and none may carry an em dash (`CLAUDE.md`) or the continuation
    /// whitespace a `\`-broken literal grows when the backslash is lost.
    #[test]
    fn every_refusal_names_the_alternative() {
        let fossil = field_use_refusal(ItemId::HelixFossil).expect("refused");
        assert!(fossil.contains("no bag use"), "{fossil}");
        assert!(fossil.contains("carry"), "it has to say what to do instead: {fossil}");

        let ball = field_use_refusal(ItemId::PokeBall).expect("refused");
        assert!(ball.contains("choose_battle_action"), "{ball}");

        let machine = field_use_refusal(ItemId::Hm01Cut).expect("refused");
        assert!(machine.contains("teach"), "{machine}");
        assert!(field_use_refusal(ItemId::Tm34Bide).is_some(), "a TM is the same refusal");

        for item in [ItemId::PokeFlute, ItemId::CardKey, ItemId::EscapeRope] {
            assert_eq!(field_use_refusal(item), None, "{item} is usable");
        }

        for item in [ItemId::HelixFossil, ItemId::PokeBall, ItemId::Hm01Cut] {
            let refusal = field_use_refusal(item).expect("refused");
            assert!(!refusal.contains('—'), "no em dashes in what the agent writes: {refusal}");
            assert!(!refusal.contains("  "), "a `\\` was eaten out of a continued literal: {refusal}");
        }
    }
}

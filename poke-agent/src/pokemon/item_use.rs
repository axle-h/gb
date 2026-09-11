//! Whether the game will do anything at all when an item is chosen with USE, read out of the
//! cartridge's own `ItemUsePtrTable`.

use crate::pokemon::item::ItemId;
use crate::pokemon::rom_gfx::rom_slice;
use crate::pokemon::symbols::pokered_symbols;

/// The last item id `ItemUsePtrTable` has a row for.
const LAST_TABLED_ITEM: u8 = ItemId::MaxElixer as u8;

/// The effect routine `UseItem_` would `jp` to for `item`, or `None` outside the table.
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

/// `ItemUseBall` refuses outside a battle with the same `ItemUseNotTime` as an unusable item, and
/// wedges the driver the same way.
pub fn is_ball(item: ItemId) -> bool {
    matches!(item, ItemId::MasterBall | ItemId::UltraBall | ItemId::GreatBall | ItemId::PokeBall | ItemId::SafariBall)
}

/// What to say when `use_item` is aimed at something the game will not use.
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

/// TM01-TM50.
fn is_machine(item: ItemId) -> bool {
    (item as u8) >= 0xC9
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Ids `$15` and `$16` are also the Safari Zone's BAIT and ROCK.
        assert!(!never_usable(ItemId::BoulderBadge), "$15 is also SAFARI_BAIT");
        assert!(!never_usable(ItemId::CascadeBadge), "$16 is also SAFARI_ROCK");

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

    /// A machine is dispatched before the table and never indexes past its rows.
    #[test]
    fn a_machine_never_reaches_the_table() {
        assert_eq!(use_effect(ItemId::Hm01Cut), None);
        assert_eq!(use_effect(ItemId::Tm34Bide), None);
        assert!(!never_usable(ItemId::Tm34Bide), "not usable, but not `UnusableItem` either");
        assert!(use_effect(ItemId::MaxElixer).is_some(), "the last row the table has");
    }

    /// Each refusal names the alternative, with no em dash and no whitespace run from a lost `\`.
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

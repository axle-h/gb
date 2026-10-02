//! `BikeShop_Script`: the clerk, who swaps a Bike Voucher for a Bicycle and offers one for a
//! million to anyone without, and the two customers, whose words change once the Bicycle is had.

use poke_core::item::ItemId;
use poke_core::symbols::pokered_events::EVENT_GOT_BICYCLE;
use poke_core::symbols::pokered_map_scripts::{TEXT_BIKESHOP_CLERK, TEXT_BIKESHOP_MIDDLE_AGED_WOMAN, TEXT_BIKESHOP_YOUNGSTER};
use serde::{Deserialize, Serialize};
use super::{text_named, Flow, Script};

/// `BikeShopMenuPrice`: the only price the Bicycle has, since its `ItemPrices` entry is 0.
pub const BICYCLE_PRICE: u32 = 1_000_000;

/// `BikeShopMenuPrice` as the menu draws it.
pub fn menu_price_text() -> String {
    format!("¥{BICYCLE_PRICE}")
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// After `BikeShopClerkOhThatsAVoucherText`.
    VoucherSeen,
    /// After `BikeShopClerkWelcomeText`, after `BikeShopClerkDoYouLikeItText` over the menu, and
    /// once `HandleMenuInput` has returned.
    Welcomed,
    DoYouLikeIt,
    MenuChosen,
    /// `.cancel`.
    Cancel,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    Flow::Return
}

pub fn text(rt: &mut Script, text_id: u8) -> Option<Flow> {
    Some(match text_id {
        TEXT_BIKESHOP_CLERK => {
            if rt.check_event(EVENT_GOT_BICYCLE) {
                return Some(rt.print_text(text_named("BikeShopClerkHowDoYouLikeYourBicycleText")).ret());
            }
            if rt.is_item_in_bag(ItemId::BikeVoucher) {
                return Some(rt.print_text(text_named("BikeShopClerkOhThatsAVoucherText")).then(Label::VoucherSeen));
            }
            rt.print_text(text_named("BikeShopClerkWelcomeText")).then(Label::Welcomed)
        }
        TEXT_BIKESHOP_MIDDLE_AGED_WOMAN => rt.print_text(text_named("BikeShopMiddleAgedWomanText.Text")).ret(),
        TEXT_BIKESHOP_YOUNGSTER => {
            let words = match rt.check_event(EVENT_GOT_BICYCLE) {
                true => "BikeShopYoungsterText.CoolBikeText",
                false => "BikeShopYoungsterText.TheseBikesAreExpensiveText",
            };
            rt.print_text(text_named(words)).ret()
        }
        _ => return None,
    })
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::VoucherSeen => {
            if !rt.give_item(ItemId::Bicycle, 1) {
                return rt.print_text(text_named("BikeShopBagFullText")).ret();
            }
            rt.remove_item(ItemId::BikeVoucher, 1);
            rt.set_event(EVENT_GOT_BICYCLE);
            rt.print_text(text_named("BikeShopExchangedVoucherText")).ret()
        }
        Label::Welcomed => {
            rt.set_no_text_delay(true);
            rt.bike_shop_menu();
            rt.print_text(text_named("BikeShopClerkDoYouLikeItText")).then(Label::DoYouLikeIt)
        }
        Label::DoYouLikeIt => rt.handle_menu_input(1, (1, 2)).then(Label::MenuChosen),
        // B leaves `BIT_NO_TEXT_DELAY` set, so the goodbye and every text after it print at once
        // until something else clears the flag.
        Label::MenuChosen => {
            let Some(row) = rt.chosen_row() else { return resume(rt, Label::Cancel) };
            rt.set_no_text_delay(false);
            if row != 0 {
                return resume(rt, Label::Cancel);
            }
            rt.print_text(text_named("BikeShopCantAffordText")).then(Label::Cancel)
        }
        Label::Cancel => rt.print_text(text_named("BikeShopComeAgainText")).ret(),
    }
}

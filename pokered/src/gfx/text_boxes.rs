//! `DisplayTextBoxID_`: the templates a box id names. Three tables in the cartridge, one enum here,
//! since a template is a rectangle and at most one string drawn inside it.
//!
//! Ids are the cartridge's `const`s, in its numbering, gaps and all. Five templates are Japanese
//! and three are named by nothing, which is recorded rather than fixed: they are reachable only by
//! writing their id, and nothing does.

use crate::gfx::ui::{UiSurface, SCREEN_TILES_X};
use crate::systems::print_num::{print_bcd, BcdFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextBoxId {
    /// `$01`, the dialogue box every `PrintText` draws.
    MessageBox,
    /// `$03`, unused.
    MenuTemplate03,
    /// `$05`.
    JpMochimono,
    /// `$06`, the bag's submenu.
    UseToss,
    /// `$07`, unused.
    MenuTemplate07,
    /// `$08`.
    JpSaveMessage,
    /// `$09`.
    JpSpeedOptions,
    /// `$0b`.
    BattleMenu,
    /// `$0c`.
    SwitchStatsCancel,
    /// `$0d`, the list menu's frame.
    ListMenuBox,
    /// `$0e`, the frame `BuySellQuitMenu` draws itself into.
    BuySellQuitTemplate,
    /// `$0f`, the frame `money_box` draws itself into.
    MoneyBoxTemplate,
    /// `$10`, unused.
    MenuTemplate10,
    /// `$11`.
    MonSpritePopup,
    /// `$12`.
    JpAh,
    /// `$1a`.
    JpPokedex,
    /// `$1b`.
    SafariBattleMenu,
}

/// A row of `TextBoxCoordTable` or `TextBoxTextAndCoordTable`: the two corners, and the string with
/// its own corner. The cartridge stores corners and works the size out with `GetTextBoxIDCoords`.
struct Template {
    corners: (usize, usize, usize, usize),
    text: Option<(&'static str, usize, usize)>,
}

impl TextBoxId {
    fn template(self) -> Template {
        let box_only = |corners| Template { corners, text: None };
        let with_text = |corners, text| Template { corners, text: Some(text) };
        match self {
            Self::MessageBox => box_only((0, 12, 19, 17)),
            Self::MenuTemplate03 => box_only((0, 0, 19, 14)),
            Self::MenuTemplate07 => box_only((0, 0, 11, 6)),
            Self::ListMenuBox => box_only((4, 2, 19, 12)),
            Self::MenuTemplate10 => box_only((7, 0, 19, 17)),
            Self::MonSpritePopup => box_only((6, 4, 14, 13)),
            Self::JpMochimono => with_text((0, 0, 14, 17), ("もちもの", 3, 0)),
            Self::UseToss => with_text((13, 10, 19, 14), ("USE<NEXT>TOSS", 15, 11)),
            Self::JpSaveMessage => with_text((0, 0, 7, 5), ("きろく<NEXT>メッセージ", 2, 2)),
            Self::JpSpeedOptions => with_text((0, 6, 5, 10), ("はやい<NEXT>おそい", 2, 7)),
            Self::BattleMenu => with_text((8, 12, 19, 17), ("FIGHT <PK><MN><NEXT>ITEM  RUN", 10, 14)),
            Self::SafariBattleMenu =>
                with_text((0, 12, 19, 17), ("BALL×       BAIT<NEXT>THROW ROCK  RUN", 2, 14)),
            Self::SwitchStatsCancel =>
                with_text((11, 11, 19, 17), ("SWITCH<NEXT>STATS<NEXT>CANCEL", 13, 12)),
            Self::BuySellQuitTemplate => with_text((0, 0, 10, 6), ("BUY<NEXT>SELL<NEXT>QUIT", 2, 1)),
            Self::MoneyBoxTemplate => with_text((11, 0, 19, 2), ("MONEY", 13, 0)),
            Self::JpAh => with_text((7, 6, 11, 10), ("アッ！", 8, 8)),
            Self::JpPokedex =>
                with_text((11, 8, 19, 17), ("データをみる<NEXT>なきごえ<NEXT>ぶんぷをみる<NEXT>キャンセル", 12, 10)),
        }
    }

    /// The border, and the string if the template carries one. Printing is instant here because
    /// the cartridge sets `BIT_NO_TEXT_DELAY` around its `PlaceString`.
    pub fn draw(self, ui: &mut UiSurface) {
        let template = self.template();
        let (x, y, right, bottom) = template.corners;
        ui.text_box_border(x, y, right - x - 1, bottom - y - 1);
        if let Some((text, tx, ty)) = template.text {
            place_lines(ui, tx, ty, text);
        }
    }
}

/// `PlaceString` for a template's string, where the only control character is `<NEXT>`: two rows
/// down, back to the column the string started in.
fn place_lines(ui: &mut UiSurface, x: usize, y: usize, text: &str) {
    for (line, part) in text.split("<NEXT>").enumerate() {
        let bytes = poke_core::charmap::encode(part).expect("a template's string encodes");
        ui.place(x, y + line * 2, &bytes);
    }
}

/// `DisplayMoneyBox`. The six digits it clears first are wider than the five a full purse needs, so
/// a shorter number does not leave the last of the old one behind.
pub fn money_box(ui: &mut UiSurface, money: &[u8; 3]) {
    TextBoxId::MoneyBoxTemplate.draw(ui);
    ui.fill(13, 1, 6, 1, UiSurface::BLANK);
    let format = BcdFormat { skip_leading_zeroes: false, left_align: false, money_sign: true };
    print_bcd(ui, SCREEN_TILES_X + 12, money, format);
}

#[cfg(test)]
mod tests {
    use poke_core::charmap::encode;
    use super::*;

    const ALL: [TextBoxId; 17] = [
        TextBoxId::MessageBox, TextBoxId::MenuTemplate03, TextBoxId::JpMochimono, TextBoxId::UseToss,
        TextBoxId::MenuTemplate07, TextBoxId::JpSaveMessage, TextBoxId::JpSpeedOptions,
        TextBoxId::BattleMenu, TextBoxId::SwitchStatsCancel, TextBoxId::ListMenuBox,
        TextBoxId::BuySellQuitTemplate, TextBoxId::MoneyBoxTemplate, TextBoxId::MenuTemplate10,
        TextBoxId::MonSpritePopup, TextBoxId::JpAh, TextBoxId::JpPokedex, TextBoxId::SafariBattleMenu,
    ];

    /// Every template, including the five Japanese leftovers and the `<PK><MN>` and `×` the battle
    /// menus use, which is what proves the charmap covers them.
    #[test]
    fn every_template_draws_inside_the_screen() {
        for id in ALL {
            let mut ui = UiSurface::default();
            id.draw(&mut ui);
        }
    }

    #[test]
    fn the_message_box_is_the_one_every_text_box_draws() {
        let mut ui = UiSurface::default();
        TextBoxId::MessageBox.draw(&mut ui);
        assert_eq!(ui.get(0, 12), 0x79, "the upper left corner");
        assert_eq!(ui.get(19, 12), 0x7B, "the upper right");
        assert_eq!(ui.get(0, 17), 0x7D, "the lower left");
        assert_eq!(ui.get(19, 17), 0x7E, "the lower right");
    }

    /// `<NEXT>` is two rows, so a template's second line is not the row below its first.
    #[test]
    fn a_template_s_lines_are_two_rows_apart() {
        let mut ui = UiSurface::default();
        TextBoxId::UseToss.draw(&mut ui);
        assert_eq!(ui.row(11)[15..18], encode("USE").unwrap()[..]);
        assert_eq!(ui.row(13)[15..19], encode("TOSS").unwrap()[..]);
        assert_eq!(ui.row(12)[15..19], [UiSurface::BLANK; 4], "the row between stays empty");
    }

    #[test]
    fn three_rows_of_the_mart_s_menu() {
        let mut ui = UiSurface::default();
        TextBoxId::BuySellQuitTemplate.draw(&mut ui);
        assert_eq!(ui.row(1)[2..5], encode("BUY").unwrap()[..]);
        assert_eq!(ui.row(3)[2..6], encode("SELL").unwrap()[..]);
        assert_eq!(ui.row(5)[2..6], encode("QUIT").unwrap()[..]);
    }

    /// `LEADING_ZEROES` is set, so a purse of ¥123 prints its zeroes rather than spaces.
    #[test]
    fn the_money_box_prints_six_digits_behind_a_yen_sign() {
        let mut ui = UiSurface::default();
        money_box(&mut ui, &[0x00, 0x01, 0x23]);
        assert_eq!(ui.row(0)[13..18], encode("MONEY").unwrap()[..]);
        assert_eq!(ui.row(1)[12..19], encode("¥000123").unwrap()[..]);
    }
}

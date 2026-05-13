use crate::mmu::MMU;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};

#[derive(Debug, Clone, Copy, Eq, PartialEq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum TextBoxId {
    MessageBox = 0x01,
    FieldMoveMonMenu = 0x04,
    JpMochimonoMenuTemplate = 0x05,
    UseTossMenuTemplate = 0x06,
    JpSaveMessageMenuTemplate = 0x08,
    JpSpeedOptionsMenuTemplate = 0x09,
    BattleMenuTemplate = 0x0b,
    SwitchStatsCancelMenuTemplate = 0x0c,
    ListMenuBox = 0x0d,
    BuySellQuitMenuTemplate = 0x0e,
    MoneyBoxTemplate = 0x0f,
    MonSpritePopup = 0x11,
    JpAhMenuTemplate = 0x12,
    MoneyBox = 0x13,
    TwoOptionMenu = 0x14,
    BuySellQuitMenu = 0x15,
    JpPokedexMenuTemplate = 0x1a,
    SafariBattleMenuTemplate = 0x1b,
}

impl TextBoxId {
    pub fn is_battle_menu(&self) -> bool {
        self == &TextBoxId::BattleMenuTemplate || self == &TextBoxId::SafariBattleMenuTemplate
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct MenuState {
    pub text_box_id: TextBoxId,
    pub current_item: u8,
    pub last_item: u8,
    pub saved_item: u8,
    pub scroll_offset: u8,
    pub cursor_location: u8,
    pub top_menu_item_x: u8,
    pub top_menu_item_y: u8,
}

impl MenuState {
    pub fn is_battle_menu(&self) -> bool {
        self.text_box_id.is_battle_menu()
    }
}

pub trait MenuStateReader {
    fn read_menu_state(&self) -> Option<MenuState>;
}

impl MenuStateReader for MMU {
    fn read_menu_state(&self) -> Option<MenuState> {
        if let Some(text_box_id) = TextBoxId::from_repr(self.read_pointer(&pokered_symbols::wTextBoxID)) {
            Some(MenuState {
                text_box_id,
                current_item: self.read_pointer(&pokered_symbols::wCurrentMenuItem),
                last_item: self.read_pointer(&pokered_symbols::wLastMenuItem),
                saved_item: self.read_pointer(&pokered_symbols::wBattleAndStartSavedMenuItem),
                scroll_offset: self.read_pointer(&pokered_symbols::wListScrollOffset),
                cursor_location: self.read_pointer(&pokered_symbols::wMenuCursorLocation),
                top_menu_item_x: self.read_pointer(&pokered_symbols::wTopMenuItemX),
                top_menu_item_y: self.read_pointer(&pokered_symbols::wTopMenuItemY),
            })
        } else {
            None
        }
    }
}
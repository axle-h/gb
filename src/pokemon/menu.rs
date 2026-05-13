use crate::geometry::Point8;
use crate::mmu::MMU;
use crate::pokemon::battle::BattleAction;
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BattleMenuState {
    Fight,
    MoveList { index: u8 }, // 0-based
    Item,
    ItemList { index: u8 }, // 0-based
    Pokemon,
    PokemonList { index: u8 }, // 0-based
    Run,
}

impl BattleMenuState {
    pub fn from_action(action: BattleAction) -> Self {
        match action {
            BattleAction::Fight(index) => Self::MoveList { index },
            BattleAction::UseItem(index) => Self::ItemList { index },
            BattleAction::SwitchPokemon(index) => Self::PokemonList { index },
            BattleAction::Run => Self::Run,
        }
    }

    pub fn parent(self) -> Option<Self> {
        match self {
            BattleMenuState::MoveList { .. } => Some(BattleMenuState::Fight),
            BattleMenuState::ItemList { .. } => Some(BattleMenuState::Item),
            BattleMenuState::PokemonList { .. } => Some(BattleMenuState::Pokemon),
            _ => None
        }
    }

    pub fn location(&self) -> Point8 {
        match self {
            BattleMenuState::Fight => Point8 { x: 0, y: 0 },
            BattleMenuState::Pokemon => Point8 { x: 1, y: 0 },
            BattleMenuState::Item => Point8 { x: 0, y: 1 },
            BattleMenuState::Run => Point8 { x: 1, y: 1 },
            BattleMenuState::MoveList { index }
                | BattleMenuState::ItemList { index }
                | BattleMenuState::PokemonList { index } =>
                Point8 { x: 0, y: *index },
        }
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

    fn battle_menu_item(&self) -> u8 {
        let right_column = match self.text_box_id {
            TextBoxId::BattleMenuTemplate => self.top_menu_item_x == 15,
            TextBoxId::SafariBattleMenuTemplate => self.top_menu_item_x == 13,
            _ => return self.current_item,
        };
        if right_column { 2 + self.current_item } else { self.current_item }
    }

    /// True only when the 2×2 main battle menu grid is showing.
    /// False when a sub-menu (move list, bag, party) is open, even though
    /// text_box_id remains BattleMenuTemplate throughout.
    fn is_main_battle_menu(&self) -> bool {
        match self.text_box_id {
            TextBoxId::BattleMenuTemplate =>
                self.top_menu_item_x == 9 || self.top_menu_item_x == 15,
            TextBoxId::SafariBattleMenuTemplate =>
                self.top_menu_item_x == 1 || self.top_menu_item_x == 13,
            _ => false,
        }
    }

    pub fn battle_menu_state(&self) -> Option<BattleMenuState> {
        if !self.is_battle_menu() {
            return None;
        }
        if self.is_main_battle_menu() {
            return Some(match self.battle_menu_item() {
                0 => BattleMenuState::Fight,
                1 => BattleMenuState::Item,
                2 => BattleMenuState::Pokemon,
                3 => BattleMenuState::Run,
                _ => return None,
            });
        }
        // Move list: top_menu_item_x=5, current_item is 1-indexed.
        if self.text_box_id == TextBoxId::BattleMenuTemplate && self.top_menu_item_x == 5 {
            return Some(BattleMenuState::MoveList { index: self.current_item.saturating_sub(1) });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu_state(current_item: u8, cursor_location: u8, top_menu_item_x: u8, top_menu_item_y: u8) -> MenuState {
        MenuState {
            text_box_id: TextBoxId::BattleMenuTemplate,
            current_item,
            last_item: current_item,
            saved_item: 0,
            scroll_offset: 0,
            cursor_location,
            top_menu_item_x,
            top_menu_item_y,
        }
    }

    #[test]
    fn fight() {
        let s = menu_state(0, 193, 9, 14);
        assert_eq!(s.battle_menu_state(), Some(BattleMenuState::Fight));
    }

    #[test]
    fn item() {
        let s = menu_state(1, 233, 9, 14);
        assert_eq!(s.battle_menu_state(), Some(BattleMenuState::Item));
    }

    #[test]
    fn pokemon() {
        let s = menu_state(0, 199, 15, 14);
        assert_eq!(s.battle_menu_state(), Some(BattleMenuState::Pokemon));
    }

    #[test]
    fn run() {
        let s = menu_state(1, 239, 15, 14);
        assert_eq!(s.battle_menu_state(), Some(BattleMenuState::Run));
    }

    #[test]
    fn move_first() {
        let s = menu_state(1, 169, 5, 12);
        assert_eq!(s.battle_menu_state(), Some(BattleMenuState::MoveList { index: 0 }));
    }

    #[test]
    fn move_second() {
        let s = menu_state(2, 189, 5, 12);
        assert_eq!(s.battle_menu_state(), Some(BattleMenuState::MoveList { index: 1 }));
    }

    #[test]
    fn move_third() {
        let s = menu_state(3, 209, 5, 12);
        assert_eq!(s.battle_menu_state(), Some(BattleMenuState::MoveList { index: 2 }));
    }

    #[test]
    fn move_fourth() {
        let s = menu_state(4, 229, 5, 12);
        assert_eq!(s.battle_menu_state(), Some(BattleMenuState::MoveList { index: 3 }));
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
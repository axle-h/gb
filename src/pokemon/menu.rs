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
            BattleAction::Fight { slot, .. } => Self::MoveList { index: slot },
            BattleAction::UseItem { slot, .. } => Self::ItemList { index: slot },
            BattleAction::SwitchPokemon { slot, .. } => Self::PokemonList { index: slot },
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
    pub scroll_offset: u8,
    pub cursor_location: u8,
    pub top_menu_item_x: u8,
    pub top_menu_item_y: u8,
}

impl MenuState {
    pub fn new(text_box_id: TextBoxId, current_item: u8, scroll_offset: u8, cursor_location: u8, top_menu_item_x: u8, top_menu_item_y: u8) -> Self {
        Self { text_box_id, current_item, scroll_offset, cursor_location, top_menu_item_x, top_menu_item_y }
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

    pub fn is_mart_buy_sell_menu(&self) -> bool {
        matches!(self.text_box_id, TextBoxId::BuySellQuitMenu | TextBoxId::BuySellQuitMenuTemplate)
    }

    /// True when the mart's item-purchase list (PRICEDITEMLISTMENU) is visible.
    /// Distinct from the battle item list by topMenuItemY == 3 vs 4.
    pub fn is_mart_item_list(&self) -> bool {
        self.text_box_id == TextBoxId::ListMenuBox
            && self.top_menu_item_x == 5
            && self.top_menu_item_y == 4
    }

    pub fn is_yes_no_menu(&self) -> bool {
        self.text_box_id == TextBoxId::TwoOptionMenu
    }

    /// Absolute item index in the list (current_item + scroll_offset).
    pub fn list_absolute_index(&self) -> u8 {
        self.current_item.saturating_add(self.scroll_offset)
    }

    pub fn battle_menu_state(&self) -> Option<BattleMenuState> {
        if self.text_box_id.is_battle_menu() {
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
        } else if self.text_box_id == TextBoxId::ListMenuBox && self.top_menu_item_x == 5 && self.top_menu_item_y == 4 {
            return Some(BattleMenuState::ItemList {
                index: self.current_item.saturating_add(self.scroll_offset),
            });
        } else if self.text_box_id == TextBoxId::MessageBox && self.top_menu_item_x == 0 && self.top_menu_item_y == 1 {
            // on the pokemon list top x is always 0 & top y is always 1 see pokemon.asm::PartyMenuInit
            return Some(BattleMenuState::PokemonList {
                index: self.current_item,
            });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TextBoxId::*;

    #[test]
    fn battle_menu_base() {
        assert_eq!(
            MenuState::new(BattleMenuTemplate, 0, 0, 193, 9, 14).battle_menu_state(),
            Some(BattleMenuState::Fight)
        );

        assert_eq!(
            MenuState::new(BattleMenuTemplate, 1, 0, 233, 9, 14).battle_menu_state(),
            Some(BattleMenuState::Item)
        );

        assert_eq!(
            MenuState::new(BattleMenuTemplate, 0, 0, 199, 15, 14).battle_menu_state(),
            Some(BattleMenuState::Pokemon)
        );

        assert_eq!(
            MenuState::new(BattleMenuTemplate, 1, 0, 239, 15, 14).battle_menu_state(),
            Some(BattleMenuState::Run)
        );
    }

    #[test]
    fn battle_menu_moves() {
        for index in 0..4 {
            let cursor_location = 169 + index * 20;
            assert_eq!(
                MenuState::new(BattleMenuTemplate, index + 1, 0, cursor_location, 5, 12).battle_menu_state(),
                Some(BattleMenuState::MoveList { index })
            );
        }
    }

    #[test]
    fn battle_menu_items() {
        assert_eq!(
            MenuState::new(ListMenuBox, 0, 0, 245, 5, 4).battle_menu_state(),
            Some(BattleMenuState::ItemList { index: 0 })
        );
        assert_eq!(
            MenuState::new(ListMenuBox, 1, 0, 29, 5, 4).battle_menu_state(),
            Some(BattleMenuState::ItemList { index: 1 })
        );
        assert_eq!(
            MenuState::new(ListMenuBox, 2, 0, 69, 5, 4).battle_menu_state(),
            Some(BattleMenuState::ItemList { index: 2 })
        );

        for index in 3..20 {
            assert_eq!(
                MenuState::new(ListMenuBox, 2, index - 2, 69, 5, 4).battle_menu_state(),
                Some(BattleMenuState::ItemList { index })
            );
        }
    }

    #[test]
    fn battle_menu_pokemon() {
        assert_eq!(
            MenuState::new(MessageBox, 0, 0, 180, 0, 1).battle_menu_state(),
            Some(BattleMenuState::PokemonList { index: 0 })
        );
        assert_eq!(
            MenuState::new(MessageBox, 1, 0, 220, 0, 1).battle_menu_state(),
            Some(BattleMenuState::PokemonList { index: 1 })
        );
        assert_eq!(
            MenuState::new(MessageBox, 2, 0, 4, 0, 1).battle_menu_state(),
            Some(BattleMenuState::PokemonList { index: 2 })
        );
        assert_eq!(
            MenuState::new(MessageBox, 3, 0, 44, 0, 1).battle_menu_state(),
            Some(BattleMenuState::PokemonList { index: 3 })
        );
        assert_eq!(
            MenuState::new(MessageBox, 4, 0, 84, 0, 1).battle_menu_state(),
            Some(BattleMenuState::PokemonList { index: 4 })
        );
        assert_eq!(
            MenuState::new(MessageBox, 5, 0, 124, 0, 1).battle_menu_state(),
            Some(BattleMenuState::PokemonList { index: 5 })
        );
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
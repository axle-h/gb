/// `wTextBoxID` values indicating a 2×2 menu is on screen waiting for input.
/// Both the main battle menu (FIGHT/PKMN/ITEM/RUN) and every sub-menu (moves, bag,
/// party) share this same ID, so we track which step we're on via the nav queue.
const BATTLE_MENU_TEXT_BOX_IDS: [u8; 2] = [0x0B, 0x1B];

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub struct MenuState {
    pub text_box_id: u8,
    pub current_menu: u8,
}

impl MenuState {
    pub fn is_battle_menu(&self) -> bool {
        BATTLE_MENU_TEXT_BOX_IDS.contains(&self.text_box_id)
    }
}
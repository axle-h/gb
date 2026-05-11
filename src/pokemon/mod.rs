use itertools::Itertools;
use strum::IntoEnumIterator;
use badge::Badge;
use map::Map;
use species::PokemonSpecies;
use encoding::{GameMode, PokemonEncoding};
use party::PokemonParty;
use tile_map::MetaTileMap;
use crate::game_boy::GameBoy;
use crate::geometry::Point8;
use crate::joypad::{JoypadButton, JoypadButtonState};
use crate::mmu::MMU;
use crate::pokemon::font::{render_font_string, FontAware, FONT_BYTES};
use crate::pokemon::symbols::{DmgPointerRead, pokered_symbols};
use crate::pokemon::move_name::{PokemonMoveName};
use crate::pokemon::pokemon::Pokemon;
use crate::pokemon::strings::PokemonString;

pub mod badge;
pub mod map;
pub mod pokemon;
pub mod status;
pub mod species;
pub mod move_name;
pub mod sprite;
pub mod party;
pub mod agent;
pub mod actions;
pub mod policy;
pub mod tile_map;
pub mod encoding;
pub mod strings;
pub mod symbols;
pub mod font;
pub mod roms;
mod text;
mod map_header;

pub trait PokemonApiTrait {
    fn release_all_buttons(&mut self);
    fn press_button(&mut self, button: JoypadButton);
    fn read_joypad_state(&self) -> JoypadButtonState;
    fn game_mode(&self) -> Option<GameMode>;
    fn game_state(&self) -> Result<GameState, String>;
    fn on_screen_text(&self) -> Option<String>;
}

#[derive(Debug)]
pub struct PokemonApi<'a> {
    game_boy: &'a mut GameBoy
}

impl<'a> PokemonApi<'a> {
    pub fn new(game_boy: &'a mut GameBoy) -> Self {
        Self { game_boy }
    }

    fn mmu(&self) -> &MMU {
        self.game_boy.core().mmu()
    }

    fn mmu_mut(&mut self) -> &mut MMU {
        self.game_boy.core_mut().mmu_mut()
    }

    pub fn pimp_out_pokemon(&mut self) -> Result<(), String> {
        let player_state = self.game_state()?;
        let mut party = player_state.pokemon;
        let charizard = Pokemon::maxed(
            PokemonSpecies::Charizard,
            "CHARIZARD",
            [
                PokemonMoveName::Flamethrower,
                PokemonMoveName::Slash,
                PokemonMoveName::Fly,
                PokemonMoveName::Earthquake,
            ],
            player_state.name.clone(),
            player_state.player_id
        );
        party.push(charizard)?;
        let mewtwo = Pokemon::maxed(
            PokemonSpecies::Mewtwo,
            "MEWTWO",
            [
                PokemonMoveName::Psychic,
                PokemonMoveName::Thunderbolt,
                PokemonMoveName::IceBeam,
                PokemonMoveName::Recover,
            ],
            player_state.name,
            player_state.player_id
        );
        party.push(mewtwo)?;
        self.mmu_mut().write_player_pokemon_party(&party)

    }
}

impl<'a> PokemonApiTrait for PokemonApi<'a> {
    fn release_all_buttons(&mut self) {
        let joypad = self.mmu_mut().joypad_mut();
        for button in JoypadButton::iter() {
            joypad.release_button(button);
        }
    }
    fn press_button(&mut self, button: JoypadButton) {
        let joypad = self.mmu_mut().joypad_mut();
        joypad.press_button(button);
    }
    fn read_joypad_state(&self) -> JoypadButtonState {
        self.mmu().joypad().state()
    }

    fn game_state(&self) -> Result<GameState, String> {
        let mmu = self.mmu();
        Ok(GameState {
            player_id: mmu.read_pointer_u16_be(&pokered_symbols::wPlayerID),
            name: mmu.read_pointer_pokemon_string(&pokered_symbols::wPlayerName),
            rival_name: mmu.read_pointer_pokemon_string(&pokered_symbols::wRivalName),
            badges: Badge::parse_flags(mmu.read_pointer(&pokered_symbols::wObtainedBadges)),
            money: encoding::reverse_bcd(mmu.read_pointer_u24_be(&pokered_symbols::wPlayerMoney)),
            mode: mmu.read_game_mode(),
            pokemon: mmu.read_player_pokemon_party()?,
            map: MetaTileMap::new(&mmu.read_current_map()?),
        })
    }

    fn on_screen_text(&self) -> Option<String> {
        let mmu = self.mmu();
        if mmu.read_game_mode() == GameMode::Overworld || !mmu.pokemon_font_loaded() {
            return None;
        }
        let ppu = mmu.ppu();
        let font_tiles = ppu.tile_indexes_of_vram_addresses(pokered_symbols::vFont.address, FONT_BYTES.len());
        if font_tiles.is_empty() {
            return None;
        }
        let mut coordinates = ppu.tile_coordinates(&font_tiles);
        coordinates.sort_by_key(|(_, p)| *p);

        let mut lines = Vec::new();
        let mut current_line = Vec::new();
        let mut prev_pos: Option<Point8> = None;
        for (char_id, pos) in coordinates {
            if let Some(prev) = prev_pos {
                if pos.y != prev.y {
                    // line break
                    lines.push(current_line);
                    current_line = Vec::new();
                } else {
                    let is_space = pos.x.saturating_sub(prev.x) > 1;
                    // only add a space (char=64) if the previous character is not a space
                    if is_space && current_line.last() != Some(&64) {
                        current_line.push(64);
                    }
                }
            }

            current_line.push(char_id);
            prev_pos = Some(pos);
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }

        Some(
            lines.into_iter()
                .map(|line| render_font_string(&line, false).trim().to_string())
                .join(" ")
        )
    }

    fn game_mode(&self) -> Option<GameMode> {
        let mmu = self.mmu();
        let player_id = mmu.read_pointer_u16_be(&pokered_symbols::wPlayerID);
        if player_id == 0 {
            // intro screens
            return None;
        }

        let new_game_player = mmu.read_pointer_pokemon_string(&pokered_symbols::DebugNewGamePlayerName);
        let player_name = mmu.read_pointer_pokemon_string(&pokered_symbols::wPlayerName);
        if player_name == new_game_player {
            // on new game screen
            return None;
        }
        Some(mmu.read_game_mode())
    }
}

#[derive(Debug, Clone, Default)]
pub struct GameState {
    pub player_id: u16,
    pub name: PokemonString,
    pub rival_name: PokemonString,
    pub badges: Vec<Badge>,
    pub money: u32,
    pub pokemon: PokemonParty,
    pub mode: GameMode,
    pub map: MetaTileMap,
}


#[cfg(test)]
mod test {
    use crate::cycles::MachineCycles;
    use super::*;

    pub const PALLET_TOWN_STATE: &[u8] = include_bytes!("./test_data/pallet-town-state.bin");
    pub const ROUTE1_STATE: &[u8] = include_bytes!("./test_data/route1-state.bin");

    #[test]
    fn test_route_1() {
        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(ROUTE1_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));
        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();
        assert_eq!(state.map.map, Map::Route1);
        println!("{}", state.map);
    }

    #[test]
    fn test_pallet_town_actions() {
        use encoding::MetaTile;

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(PALLET_TOWN_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));
        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();
        let map = &state.map;

        let actions = map.actions();

        // Sanity-check that the test is actually exercising something.
        assert!(!map.warp_targets.is_empty(), "expected warp targets in Pallet Town");
        assert!(map.sprites.iter().any(|s| !s.hidden), "expected visible sprites in Pallet Town");

        // Every warp target must produce an action with a non-empty route.
        for &to_map in &map.warp_targets {
            let action = actions.iter().find(|a| a.tile == MetaTile::Warp(to_map));
            assert!(action.is_some(), "no action for warp to {to_map}");
            assert!(!action.unwrap().route.is_empty(), "empty route to warp {to_map}");
        }

        // Every visible (non-hidden) sprite must produce an action with a non-empty route.
        for sprite in map.sprites.iter().filter(|s| !s.hidden) {
            let action = actions.iter().find(|a| a.tile == MetaTile::Sprite(sprite.name));
            assert!(action.is_some(), "no action for sprite '{}'", sprite.name);
            assert!(!action.unwrap().route.is_empty(), "empty route to sprite '{}'", sprite.name);
        }

        // Route 1 is walkable from Pallet Town — must have a connection action.
        assert!(
            actions.iter().any(|a| a.tile == MetaTile::Connection(Map::Route1)),
            "missing connection action for Route1"
        );

        // Route 21 is water-only from Pallet Town — no walkable Connection action expected.
        assert!(
            !actions.iter().any(|a| a.tile == MetaTile::Connection(Map::Route21)),
            "unexpected walkable connection to Route21 (should be water-only)"
        );
    }
}


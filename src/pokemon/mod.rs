use strum::IntoEnumIterator;
use badge::Badge;
use map::Map;
use species::PokemonSpecies;
use actions::OverworldAction;
use encoding::{GameMode, MetaTile, PokemonEncoding};
use party::PokemonParty;
use tile_map::MetaTileMap;
use crate::game_boy::GameBoy;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::font::{render_font_string, FontAware, FONT_BYTES};
use crate::pokemon::symbols::{DmgPointerRead, pokered_symbols};
use crate::pokemon::move_name::{PokemonMoveName};
use crate::pokemon::pokemon::Pokemon;
use crate::pokemon::sprite::Sprite;
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
pub mod tile_map;
pub mod encoding;
pub mod strings;
pub mod symbols;
pub mod font;
pub mod roms;

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

    pub fn release_all_buttons(&mut self) {
        let joypad = self.mmu_mut().joypad_mut();
        for button in JoypadButton::iter() {
            joypad.release_button(button);
        }
    }

    pub fn press_button(&mut self, button: JoypadButton) {
        let joypad = self.mmu_mut().joypad_mut();
        joypad.press_button(button);
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

    pub fn game_state(&self) -> Result<GameState, String> {
        let mmu = self.mmu();
        let current_map = mmu.read_current_map()?;
        let meta_tile_map = MetaTileMap::new(&current_map);
        // println!("{}", meta_tile_map);
        let actions = meta_tile_map.actions();
        // for action in actions.iter() {
        //     println!("{:?}", action);
        // }

        Ok(GameState {
            player_id: mmu.read_pointer_u16_be(&pokered_symbols::wPlayerID),
            name: mmu.read_pointer_pokemon_string(&pokered_symbols::wPlayerName),
            rival_name: mmu.read_pointer_pokemon_string(&pokered_symbols::wRivalName),
            badges: Badge::parse_flags(mmu.read_pointer(&pokered_symbols::wObtainedBadges)),
            money: encoding::reverse_bcd(mmu.read_pointer_u24_be(&pokered_symbols::wPlayerMoney)),
            mode: mmu.read_game_mode(),
            pokemon: mmu.read_player_pokemon_party()?,
            map: meta_tile_map.map,
            player_position: meta_tile_map.player_position,
            player_tile: meta_tile_map.player_tile(),
            sprites: meta_tile_map.sprites,
            actions,
        })
    }

    pub fn on_screen_text(&self) -> Option<String> {
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

        let mut font_chars = Vec::new();
        let mut prev_pos: Option<Point8> = None;
        let mut prev_space = false;
        for (char_id, pos) in coordinates {
            if let Some(prev) = prev_pos {
                // Check for line breaks (different y coordinate) or spaces (x coordinate difference > 1)
                let is_space = pos.y != prev.y || pos.x.saturating_sub(prev.x) > 1;
                // only add a space (char=64) if the previous character is not a space
                if is_space && !prev_space {
                    font_chars.push(64);
                }
                prev_space = is_space;
            }

            font_chars.push(char_id);
            prev_pos = Some(pos);
        }

        Some(render_font_string(&font_chars))
    }
}

#[derive(Debug, Clone)]
pub struct GameState {
    pub player_id: u16,
    pub name: PokemonString,
    pub rival_name: PokemonString,
    pub badges: Vec<Badge>,
    pub money: u32,
    pub pokemon: PokemonParty,
    pub mode: GameMode,
    pub map: Map,
    pub player_position: Point8,
    pub player_tile: MetaTile,
    pub sprites: Vec<Sprite>,
    pub actions: Vec<OverworldAction>,
}

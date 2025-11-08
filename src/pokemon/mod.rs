use std::fmt::Display;
use std::ops::{Deref, DerefMut, Index, IndexMut};
use strum::IntoEnumIterator;
use badge::Badge;
use map::Map;
use species::PokemonSpecies;
use unicode_segmentation::UnicodeSegmentation;
use actions::OverworldAction;
use encoding::{GameMode, MetaTile, PokemonBlockAddresses, PokemonEncoding};
use party::PokemonParty;
use tile_map::MetaTileMap;
use crate::game_boy::GameBoy;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::pokemon::{Pokemon, PokemonStats, PokemonType};
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
mod tile_map;
mod encoding;
mod strings;
mod memory_map;

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
        self.mmu_mut().write_player_pokemon_party(party);
        Ok(())
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
            player_id: self.mmu().read(0xD359) as u16 * 256 + self.mmu().read(0xD35A) as u16,
            name: self.mmu().read_pokemon_string(0xD158),
            rival_name: self.mmu().read_pokemon_string(0xD34A),
            badges: Badge::parse_flags(self.mmu().read(0xD356)),
            money: encoding::reverse_bcd(self.mmu().read_u32_be(0xD346) & 0xFFFFFF),
            mode: mmu.read_game_mode(),
            pokemon: mmu.read_player_pokemon_party()?,
            map: meta_tile_map.map,
            player_position: meta_tile_map.player_position,
            player_tile: meta_tile_map.player_tile(),
            sprites: meta_tile_map.sprites,
            actions,
        })
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

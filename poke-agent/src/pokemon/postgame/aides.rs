//! Oak's aides, and the dex sweep that earns their prizes.

use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::policy::{PartyRef, PolicyStep};
use crate::pokemon::GameState;

/// The species `on_map`'s grass yields with an encounter share of at least `min_share` percent.
pub fn sweep_targets(on_map: Map, min_share: u8) -> Vec<crate::pokemon::species::PokemonSpecies> {
    use crate::pokemon::wild::{self, Terrain};
    wild::encounters(on_map).map_or_else(Vec::new, |wild| wild.species(Terrain::Grass).into_iter()
        .filter(|(_, share, _)| share * 100.0 >= min_share as f64)
        .map(|(species, _, _)| species)
        .collect())
}

/// What is left to catch here, empty when the step is done.
pub fn sweep_remaining(state: &GameState, on_map: Map, min_share: u8)
    -> Vec<crate::pokemon::species::PokemonSpecies> {
    sweep_targets(on_map, min_share).into_iter()
        .filter(|species| !state.pokedex_owned.contains(species))
        .collect()
}

pub fn sweep_wants(state: &GameState, enemy: crate::pokemon::species::PokemonSpecies) -> bool {
    !state.pokedex_owned.contains(&enemy)
}

impl PolicyStep {
    /// Collect HM05 Flash from the Route 2 Gate aide, then teach it and light a cave.
    pub fn flash_steps(withdraw: Option<u8>, shed: ItemId, flash_slot: u8) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::ViridianCity }, Self::enter(Map::ViridianPokecenter)];
        s.push(Self::deposit_item(shed, u8::MAX, Map::ViridianPokecenter));
        s.extend(withdraw.map(|box_slot| Self::withdraw_pokemon(box_slot, Map::ViridianPokecenter)));
        s.extend([
            Self::enter(Map::ViridianCity),
            Self::Fly { to: Map::PewterCity },
            Self::enter(Map::Route2),
            Self::CutTree { map: Map::Route2 },
            Self::enter(Map::Route2Gate),
        ]);
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::ROUTE2GATE_OAKS_AIDE), 3));
        s.extend([
            Self::enter(Map::Route2),
            Self::TeachMove { item: ItemId::Hm05Flash, target: PartyRef::Slot(flash_slot) },
            Self::Fly { to: Map::LavenderTown },
            Self::enter(Map::Route10),
            Self::enter(Map::RockTunnel1F),
            Self::UseFlash { slot: flash_slot },
        ]);
        s
    }

    /// Collect the Itemfinder from the `Route11Gate2F` aide.
    pub fn itemfinder_steps(shed: &[ItemId]) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::VermilionCity }, Self::enter(Map::VermilionPokecenter)];
        s.extend(shed.iter().map(|&item| Self::deposit_item(item, u8::MAX, Map::VermilionPokecenter)));
        s.extend([
            Self::enter(Map::VermilionCity),
            Self::goto(Map::Route11),
            Self::enter(Map::Route11Gate1F),
            Self::enter(Map::Route11Gate2F),
        ]);
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::ROUTE11GATE2F_OAKS_AIDE), 3));
        s.extend([
            Self::enter(Map::Route11Gate1F),
            Self::enter_at(Map::Route11, 50, 8),
        ]);
        s
    }

    /// The sweep's shopping trip: a bag slot, the balls, and an empty box.
    pub fn dex_sweep_outfit_steps(shed: &[ItemId], balls: u8, box_n: u8) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::VermilionCity }, Self::enter(Map::VermilionPokecenter)];
        s.extend(shed.iter().map(|&item| Self::deposit_item(item, u8::MAX, Map::VermilionPokecenter)));
        // `change_box` saves the game, so it belongs at the top of a leg rather than mid-sweep.
        s.push(Self::change_box(box_n, Map::VermilionPokecenter));
        s.extend([
            Self::enter(Map::VermilionCity),
            Self::enter(Map::VermilionMart),
            Self::BuyFromMart { item: crate::pokemon::bag::BagItem::new(ItemId::PokeBall, balls),
                                map: Map::VermilionMart },
            Self::enter(Map::VermilionCity),
        ]);
        s
    }

    /// Route 11 and Diglett's Cave.
    pub fn dex_sweep_vermilion_steps(min_share: u8) -> Vec<Self> {
        vec![
            Self::goto(Map::Route11),
            Self::sweep(Map::Route11, min_share),
            Self::enter(Map::DiglettsCaveRoute11),
            Self::enter(Map::DiglettsCave),
            Self::sweep(Map::DiglettsCave, min_share),
            Self::enter(Map::DiglettsCaveRoute11),
            Self::enter(Map::Route11),
        ]
    }

    /// Route 1 and Viridian Forest.
    pub fn dex_sweep_viridian_steps(min_share: u8) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::ViridianCity },
            Self::enter(Map::Route1),
            Self::sweep(Map::Route1, min_share),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::Route2),
            Self::enter(Map::ViridianForestSouthGate),
            Self::enter(Map::ViridianForest),
            // The forest's tail is cheap enough to wait for.
            Self::sweep(Map::ViridianForest, 5),
            Self::enter(Map::ViridianForestSouthGate),
            Self::enter(Map::Route2),
        ]
    }

    /// The Lavender grounds: Pokémon Tower 3F, and Rock Tunnel.
    pub fn dex_sweep_lavender_steps(min_share: u8) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::LavenderTown },
            Self::enter(Map::LavenderPokecenter),
            Self::deposit_item(ItemId::EscapeRope, u8::MAX, Map::LavenderPokecenter),
            Self::withdraw_item(ItemId::SilphScope, 1, Map::LavenderPokecenter),
            Self::enter(Map::LavenderTown),
            Self::enter(Map::PokemonTower1F),
            Self::enter(Map::PokemonTower2F),
            Self::enter(Map::PokemonTower3F),
            Self::sweep(Map::PokemonTower3F, min_share),
            Self::enter(Map::PokemonTower2F),
            Self::enter(Map::PokemonTower1F),
            Self::enter(Map::LavenderTown),
            Self::enter(Map::Route10),
            Self::enter(Map::RockTunnel1F),
            // Takes the Machop and leaves the rarer, harder Onix.
            Self::sweep(Map::RockTunnel1F, 10),
            Self::enter(Map::Route10),
        ]
    }

    /// Route 7, whose Growlithe sits below a 20 % floor.
    pub fn dex_sweep_celadon_steps() -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeladonCity },
            Self::enter(Map::Route7),
            Self::sweep(Map::Route7, 5),
            Self::enter(Map::CeladonCity),
        ]
    }

    /// Pokémon Mansion 1F, for the margin.
    pub fn dex_sweep_mansion_steps() -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CinnabarIsland },
            Self::enter(Map::PokemonMansion1F),
            Self::sweep(Map::PokemonMansion1F, 5),
            Self::enter(Map::CinnabarIsland),
        ]
    }

    /// Collect Exp.All from the `Route15Gate2F` aide at 50 species owned.
    pub fn exp_all_steps() -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::FuchsiaCity },
            Self::goto(Map::Route15),
            Self::enter(Map::Route15Gate1F),
            Self::enter(Map::Route15Gate2F),
        ];
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::ROUTE15GATE2F_OAKS_AIDE), 3));
        s.extend([Self::enter(Map::Route15Gate1F), Self::enter(Map::Route15)]);
        s
    }

    fn sweep(on_map: Map, min_share: u8) -> Self {
        Self::SweepDex { on_map, min_share, ball: Some(ItemId::PokeBall) }
    }
}

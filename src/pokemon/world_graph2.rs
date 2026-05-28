use std::collections::{HashMap, VecDeque};
use crate::geometry::Point8;
use crate::mmu::MMU;
use crate::pokemon::map::Map;
use crate::pokemon::map_metadata::{CurrentMap, MapMetadataCache, PlayerFacingDirection};
use crate::pokemon::tile::MetaTile;
use crate::pokemon::tile_map::MetaTileMap;

pub struct WorldGraph {
    adjacency: HashMap<Map, Vec<Map>>,
}

impl WorldGraph {
    pub fn new(mmu: &MMU, cache: &mut MapMetadataCache) -> Result<Self, String> {
        let mut adjacency: HashMap<Map, Vec<Map>> = HashMap::new();
        let mut queue: VecDeque<(Map, Point8)> = Default::default();

        // start in the middle of pallet town
        queue.push_back((Map::PalletTown, Point8 { x: 9, y: 7 }));

        while let Some((map, player_position)) = queue.pop_front() {
            let current_map = CurrentMap {
                player_position,
                player_direction: PlayerFacingDirection::Down,
                sprites: vec![],
                metadata: cache.read_map(mmu, map)?,
            };

            let tile_map = MetaTileMap::new(&current_map);
            for action in tile_map.actions() {
                match action.tile {
                    MetaTile::Warp { to_map, to_position } | MetaTile::Connection { to_map, to_position } => {
                        adjacency.entry(map).or_default().push(to_map);
                        if !adjacency.contains_key(&to_map) {
                            queue.push_back((to_map, to_position));
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(Self { adjacency })
    }
}
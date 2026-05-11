use std::collections::HashSet;
use std::fmt::{Display, Formatter};
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::map::Map;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::encoding::{CurrentMap, MetaTile, PlayerFacingDirection};
use crate::pokemon::sprite::Sprite;

#[derive(Debug, Clone, Default)]
pub struct MetaTileMap {
    pub player_position: Point8,
    pub player_direction: PlayerFacingDirection,
    pub map: Map,
    pub width: usize,
    pub height: usize,
    pub meta_tiles: Vec<MetaTile>,
    pub sprites: Vec<Sprite>,
    pub warp_targets: HashSet<Map>,
}

impl MetaTileMap {
    pub fn new(map: &CurrentMap) -> Self {
        let north_extra = map.north_extra();
        let west_extra  = map.west_extra();
        Self {
            player_position: Point8 {
                x: map.player_position.x + west_extra as u8,
                y: map.player_position.y + north_extra as u8,
            },
            player_direction: map.player_direction,
            map: map.map,
            width:  map.meta_width()  + west_extra + map.east_extra(),
            height: map.meta_height() + north_extra + map.south_extra(),
            meta_tiles: map.meta_tiles(),
            sprites: map.sprites.iter().map(|s| {
                let mut s = *s;
                s.position.x += west_extra as u8;
                s.position.y += north_extra as u8;
                s
            }).collect(),
            warp_targets: map.warp_events.iter()
                .map(|warp_event| warp_event.map_id)
                .collect()
        }
    }

    pub fn player_tile(&self) -> MetaTile {
        self.meta_tiles[self.player_position.x as usize + self.player_position.y as usize * self.width]
    }

    fn player_route(&self, destination: Point8) -> Option<OverworldAction> {
        use std::collections::{BinaryHeap, HashMap};
        use std::cmp::Ordering;
        use crate::joypad::JoypadButton;

        #[derive(Clone, Eq, PartialEq)]
        struct Node {
            position: Point8,
            cost: u32,
            heuristic: u32,
        }

        impl Ord for Node {
            fn cmp(&self, other: &Self) -> Ordering {
                (other.cost + other.heuristic).cmp(&(self.cost + self.heuristic))
            }
        }

        impl PartialOrd for Node {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        let heuristic = |pos: Point8| -> u32 {
            (pos.x.abs_diff(destination.x) + pos.y.abs_diff(destination.y)) as u32
        };

        let mut open_set = BinaryHeap::new();
        let mut came_from: HashMap<Point8, (Point8, JoypadButton)> = HashMap::new();
        let mut g_score: HashMap<Point8, u32> = HashMap::new();

        let origin = self.player_position;
        open_set.push(Node { position: origin, cost: 0, heuristic: heuristic(origin) });
        g_score.insert(origin, 0);

        while let Some(current) = open_set.pop() {
            if current.position == destination {
                let mut route = vec![];
                let mut pos = destination;
                while pos != origin {
                    if let Some((prev, dir)) = came_from.get(&pos) {
                        route.push(*dir);
                        pos = *prev;
                    }
                }
                route.reverse();
                return Some(
                    OverworldAction {
                        map: self.map,
                        origin,
                        destination,
                        tile: self.meta_tiles[destination.x as usize + destination.y as usize * self.width].clone(),
                        route,
                    }
                );
            }

            let neighbors = [
                (JoypadButton::Up, Point8 { x: current.position.x, y: current.position.y.saturating_sub(1) }),
                (JoypadButton::Down, Point8 { x: current.position.x, y: current.position.y + 1 }),
                (JoypadButton::Left, Point8 { x: current.position.x.saturating_sub(1), y: current.position.y }),
                (JoypadButton::Right, Point8 { x: current.position.x + 1, y: current.position.y }),
            ];

            for (dir, neighbor) in neighbors {
                if neighbor.x as usize >= self.width || neighbor.y as usize >= self.height {
                    continue;
                }

                let tile = &self.meta_tiles[neighbor.x as usize + neighbor.y as usize * self.width];
                if matches!(tile, MetaTile::Obstacle | MetaTile::Sprite(_) | MetaTile::Water | MetaTile::ConnectionWater(_)) && neighbor != destination {
                    continue;
                }

                let tentative_g = g_score.get(&current.position).unwrap_or(&u32::MAX) + 1;
                if tentative_g < *g_score.get(&neighbor).unwrap_or(&u32::MAX) {
                    came_from.insert(neighbor, (current.position, dir));
                    g_score.insert(neighbor, tentative_g);
                    open_set.push(Node {
                        position: neighbor,
                        cost: tentative_g,
                        heuristic: heuristic(neighbor),
                    });
                }
            }
        }
        None
    }

    pub fn actions(&self) -> Vec<OverworldAction> {
        // 1. routes to warps
        let mut routes = vec![];
        for to_map in &self.warp_targets {
            let target_tile = MetaTile::Warp(*to_map);
            let shortest_route = self.meta_tiles
                .iter()
                .enumerate()
                .filter(|(_, tile)| tile == &&target_tile)
                .map(|(index, _)| Point8 { x: (index % self.width) as u8, y: (index / self.width) as u8 })
                .filter_map(|to| self.player_route(to))
                .min_by(|a, b| a.route.len().cmp(&b.route.len()));

            if shortest_route.is_none() {
                continue;
            }

            let mut shortest_route = shortest_route.unwrap();

            let dest = shortest_route.destination;
            // Determine the direction the player should move to enter/trigger the warp
            let warp_enter_dir = if dest.x == 0 {
                JoypadButton::Left
            } else if dest.x == (self.width - 1) as u8 {
                JoypadButton::Right
            } else if dest.y == 0 {
                JoypadButton::Up
            } else if dest.y == (self.height - 1) as u8 {
                JoypadButton::Down
            } else {
                // Interior warp: use the approach direction, defaulting to Up
                *shortest_route.route.last().unwrap_or(&JoypadButton::Up)
            };

            if shortest_route.route.is_empty() {
                // Player is already on the warp tile — step off then back on to trigger the warp
                let step_off = match warp_enter_dir {
                    JoypadButton::Left => JoypadButton::Right,
                    JoypadButton::Right => JoypadButton::Left,
                    JoypadButton::Up => JoypadButton::Down,
                    JoypadButton::Down => JoypadButton::Up,
                    other => other,
                };
                shortest_route.route.push(step_off);
                shortest_route.route.push(warp_enter_dir);
            } else {
                // Player will walk onto the warp tile; push the enter direction to trigger it
                shortest_route.route.push(warp_enter_dir);
            }

            routes.push(shortest_route);
        }

        // 2. routes to sprites - keep shortest route per sprite
        for sprite in &self.sprites {
            if sprite.hidden {
                continue;
            }

            let sprite_pos = sprite.position;
            let adjacent_positions = [
                (PlayerFacingDirection::Down, Point8 { x: sprite_pos.x, y: sprite_pos.y.saturating_sub(1) }),
                (PlayerFacingDirection::Up, Point8 { x: sprite_pos.x, y: sprite_pos.y + 1 }),
                (PlayerFacingDirection::Right, Point8 { x: sprite_pos.x.saturating_sub(1), y: sprite_pos.y }),
                (PlayerFacingDirection::Left, Point8 { x: sprite_pos.x + 1, y: sprite_pos.y }),
            ];
            let shortest_route = adjacent_positions
                .into_iter()
                .filter(|(_, pos)| {
                    pos.x < self.width as u8 && pos.y < self.height as u8 &&
                        matches!(self.meta_tiles[pos.x as usize + pos.y as usize * self.width], MetaTile::Empty)
                })
                .filter_map(|(dir, to)| {
                    self.player_route(to).map(|route| (dir, route))
                })
                .min_by(|(_, a), (_, b)| a.route.len().cmp(&b.route.len()));

            if shortest_route.is_none() {
                continue;
            }
            let (sprite_direction, mut shortest_route) = shortest_route.unwrap();
            shortest_route.tile = MetaTile::Sprite(sprite.name);
            if shortest_route.route.is_empty() {
                if sprite_direction != self.player_direction {
                    // no more movement needed, just turn to face sprite
                    shortest_route.route.push(sprite_direction.into());
                }
            } else {
                // ensure we face the sprite before interacting
                let last_move = shortest_route.route.last().unwrap();
                let expected_button: JoypadButton = sprite_direction.into();
                if *last_move != expected_button {
                    shortest_route.route.push(expected_button);
                }
            }
            shortest_route.route.push(JoypadButton::A);
            routes.push(shortest_route);
        }

        // 3. routes to connections
        // Collect unique connected maps from connection tiles, then find the shortest
        // route to any connection tile for each map and add the edge-exit button press.
        // TODO can this be more efficient?
        let connection_maps: HashSet<Map> = self.meta_tiles.iter()
            .filter_map(|t| if let MetaTile::Connection(m) = t { Some(*m) } else { None })
            .collect();

        for to_map in &connection_maps {
            let target_tile = MetaTile::Connection(*to_map);
            let shortest_route = self.meta_tiles
                .iter()
                .enumerate()
                .filter(|(_, tile)| tile == &&target_tile)
                .map(|(index, _)| Point8 { x: (index % self.width) as u8, y: (index / self.width) as u8 })
                .filter_map(|to| self.player_route(to))
                .min_by(|a, b| a.route.len().cmp(&b.route.len()));

            let Some(mut shortest_route) = shortest_route else { continue };

            // Connection tiles are always on a map edge; the exit direction is unambiguous.
            let dest = shortest_route.destination;
            let enter_dir = if dest.y == 0 {
                JoypadButton::Up
            } else if dest.y == (self.height - 1) as u8 {
                JoypadButton::Down
            } else if dest.x == 0 {
                JoypadButton::Left
            } else {
                JoypadButton::Right
            };
            shortest_route.route.push(enter_dir);
            routes.push(shortest_route);
        }

        routes
    }
}

impl Display for MetaTileMap {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for y in 0..self.height {
            for x in 0..self.width {
                match self.meta_tiles[x + y * self.width] {
                    MetaTile::Empty => write!(f, "_")?,
                    MetaTile::Obstacle => write!(f, "O")?,
                    MetaTile::Water => write!(f, "X")?,
                    MetaTile::Sprite(_) => write!(f, "S")?,
                    MetaTile::Warp(_) => write!(f, "W")?,
                    MetaTile::Connection(_) => write!(f, "C")?,
                    MetaTile::ConnectionWater(_) => write!(f, "~")?,
                }
            }
            writeln!(f)?;
        }
        writeln!(f)
    }
}
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::map::Map;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::encoding::{CurrentMap, JumpDirection, MetaTile, PlayerFacingDirection};
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

    /// BFS from `player_position` outward.
    ///
    /// Returns `(dist, came_from)` where `dist[p]` is the minimum step-count to reach `p`
    /// and `came_from[p]` is the `(previous_position, direction)` on the shortest path.
    ///
    /// Passable tiles (Empty, Warp, Connection) are expanded. Impassable tiles
    /// (Obstacle, Sprite, Water, ConnectionWater) are recorded as reachable destinations
    /// but not expanded — they are dead ends for traversal.
    fn bfs_from_player(&self) -> (HashMap<Point8, u32>, HashMap<Point8, (Point8, JoypadButton)>) {
        use std::collections::{HashMap, VecDeque};

        let mut dist: HashMap<Point8, u32> = HashMap::new();
        let mut came_from: HashMap<Point8, (Point8, JoypadButton)> = HashMap::new();
        let mut queue = VecDeque::new();

        dist.insert(self.player_position, 0);
        queue.push_back(self.player_position);

        while let Some(pos) = queue.pop_front() {
            let d = dist[&pos];
            let neighbors = [
                (JoypadButton::Up,    Point8 { x: pos.x,                   y: pos.y.saturating_sub(1) }),
                (JoypadButton::Down,  Point8 { x: pos.x,                   y: pos.y + 1               }),
                (JoypadButton::Left,  Point8 { x: pos.x.saturating_sub(1), y: pos.y                   }),
                (JoypadButton::Right, Point8 { x: pos.x + 1,               y: pos.y                   }),
            ];
            for (dir, nb) in neighbors {
                if nb.x as usize >= self.width || nb.y as usize >= self.height { continue; }
                if dist.contains_key(&nb) { continue; }
                dist.insert(nb, d + 1);
                came_from.insert(nb, (pos, dir));
                let tile = &self.meta_tiles[nb.x as usize + nb.y as usize * self.width];
                if !matches!(tile, MetaTile::Obstacle | MetaTile::Sprite(_) | MetaTile::Water | MetaTile::ConnectionWater(_) | MetaTile::Jump(_)) {
                    queue.push_back(nb);
                }
            }
        }
        (dist, came_from)
    }

    pub fn actions(&self) -> Vec<OverworldAction> {
        // Single BFS from player_position computes shortest distances to every reachable tile.
        // All per-action lookups then become O(candidates) instead of O(candidates × map_size).
        let (dist, came_from) = self.bfs_from_player();

        // Reconstruct the step sequence from came_from back-pointers.
        let reconstruct = |dest: Point8| -> Vec<JoypadButton> {
            let mut route = vec![];
            let mut pos = dest;
            while pos != self.player_position {
                let &(prev, dir) = came_from.get(&pos).unwrap();
                route.push(dir);
                pos = prev;
            }
            route.reverse();
            route
        };

        // Find the nearest tile matching `pred` that BFS reached.
        let nearest = |pred: &dyn Fn(&MetaTile) -> bool| -> Option<Point8> {
            self.meta_tiles.iter()
                .enumerate()
                .filter(|(_, t)| pred(t))
                .map(|(i, _)| Point8 { x: (i % self.width) as u8, y: (i / self.width) as u8 })
                .filter(|p| dist.contains_key(p))
                .min_by_key(|p| dist[p])
        };

        let mut routes = vec![];

        // 1. Routes to warps
        for &to_map in &self.warp_targets {
            let Some(dest) = nearest(&|t| t == &MetaTile::Warp(to_map)) else { continue };
            let mut route = reconstruct(dest);

            let enter_dir = if dest.x == 0 { JoypadButton::Left }
                else if dest.x == (self.width - 1) as u8 { JoypadButton::Right }
                else if dest.y == 0 { JoypadButton::Up }
                else if dest.y == (self.height - 1) as u8 { JoypadButton::Down }
                else { *route.last().unwrap_or(&JoypadButton::Up) };

            if route.is_empty() {
                // Already on the warp tile: step off then back on to trigger it.
                route.push(opposite_dir(enter_dir));
                route.push(enter_dir);
            } else {
                route.push(enter_dir);
            }
            routes.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Warp(to_map), route });
        }

        // 2. Routes to sprites (route to an adjacent empty tile, then face the sprite)
        for sprite in self.sprites.iter().filter(|s| !s.hidden) {
            let sp = sprite.position;
            let adjacent = [
                (PlayerFacingDirection::Down,  Point8 { x: sp.x,                   y: sp.y.saturating_sub(1) }),
                (PlayerFacingDirection::Up,    Point8 { x: sp.x,                   y: sp.y + 1               }),
                (PlayerFacingDirection::Right, Point8 { x: sp.x.saturating_sub(1), y: sp.y                   }),
                (PlayerFacingDirection::Left,  Point8 { x: sp.x + 1,               y: sp.y                   }),
            ];

            let Some((face_dir, dest)) = adjacent.iter()
                .filter(|(_, p)| {
                    (p.x as usize) < self.width && (p.y as usize) < self.height
                    && matches!(self.meta_tiles[p.x as usize + p.y as usize * self.width], MetaTile::Empty)
                    && dist.contains_key(p)
                })
                .min_by_key(|(_, p)| dist[p])
                .copied()
            else { continue };

            let mut route = reconstruct(dest);
            let face_button: JoypadButton = face_dir.into();
            if route.is_empty() {
                if face_dir != self.player_direction { route.push(face_button); }
            } else if route.last() != Some(&face_button) {
                route.push(face_button);
            }
            route.push(JoypadButton::A);
            routes.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Sprite(sprite.name), route });
        }

        // 3. Routes to map connections (nearest reachable connection tile per adjacent map)
        let connection_maps: HashSet<Map> = self.meta_tiles.iter()
            .filter_map(|t| if let MetaTile::Connection(m) = t { Some(*m) } else { None })
            .collect();

        for to_map in connection_maps {
            let Some(dest) = nearest(&|t| t == &MetaTile::Connection(to_map)) else { continue };
            let mut route = reconstruct(dest);

            let enter_dir = if dest.y == 0 { JoypadButton::Up }
                else if dest.y == (self.height - 1) as u8 { JoypadButton::Down }
                else if dest.x == 0 { JoypadButton::Left }
                else { JoypadButton::Right };
            route.push(enter_dir);
            routes.push(OverworldAction { map: self.map, origin: self.player_position, destination: dest, tile: MetaTile::Connection(to_map), route });
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
                    MetaTile::Jump(JumpDirection::South) => write!(f, "v")?,
                    MetaTile::Jump(JumpDirection::West)  => write!(f, "<")?,
                    MetaTile::Jump(JumpDirection::East)  => write!(f, ">")?,
                }
            }
            writeln!(f)?;
        }
        writeln!(f)
    }
}

fn opposite_dir(dir: JoypadButton) -> JoypadButton {
    match dir {
        JoypadButton::Left  => JoypadButton::Right,
        JoypadButton::Right => JoypadButton::Left,
        JoypadButton::Up    => JoypadButton::Down,
        JoypadButton::Down  => JoypadButton::Up,
        other               => other,
    }
}
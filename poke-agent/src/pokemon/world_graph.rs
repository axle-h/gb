use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::{Display, Formatter};
use gb::geometry::Point8;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::map::Map;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::tile_map::MetaTileMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// A walk across a map border strip.
    Connection,
    /// A door, cave entrance or warp tile.
    Warp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapCoordinates {
    pub map: Map,
    pub location: Point8,
}

impl Display for MapCoordinates {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.map, self.location)
    }
}

impl MapCoordinates {
    pub fn new(map: Map, location: Point8) -> Self {
        Self { map, location }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    /// The tile that triggers the transition, in the source map's expanded tile coordinates.
    pub from: MapCoordinates,
    /// Where the player lands, in raw `wXCoord`/`wYCoord`.
    pub to: MapCoordinates,
    pub kind: EdgeKind,
}

impl Display for Edge {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} --{:?}--> {}", self.from, self.kind, self.to)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapStep {
    pub map: Map,
    /// How the player arrived; `None` for the starting map.
    pub via: Option<EdgeKind>,
    /// The tile on the previous map `via` leaves from, in action-id coordinates.
    pub via_at: Option<Point8>,
}

/// The maps observed so far, keyed by `(map, entry)` so disconnected sections of one map stay apart.
#[derive(Debug, Clone, Default)]
pub struct WorldGraph {
    adjacency: HashMap<(Map, Point8), Vec<Edge>>,
    arrival: Option<Arrival>,
}

/// Where the player came into the map they are on, and from where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrival {
    pub map: Map,
    /// In the tile map's coordinates, not the raw `wXCoord`/`wYCoord` the graph keys sections on.
    pub at: Point8,
    pub from: Option<Map>,
}

impl WorldGraph {
    pub fn new() -> Self {
        Self { adjacency: HashMap::new(), arrival: None }
    }

    fn edges_from_reachable(tile_map: &MetaTileMap, map: Map) -> Vec<Edge> {
        tile_map
            .all_reachable_warps_and_connections()
            .into_iter()
            .filter_map(|(src_pos, tile)| {
                let (to_map, raw_entry, kind) = match tile {
                    MetaTile::Warp { to_map, to_position } => (to_map, to_position, EdgeKind::Warp),
                    MetaTile::Connection { to_map, to_position } => (to_map, to_position, EdgeKind::Connection),
                    _ => return None,
                };
                Some(Edge {
                    from: MapCoordinates::new(map, src_pos),
                    to: MapCoordinates::new(to_map, raw_entry),
                    kind,
                })
            })
            .collect()
    }

    /// Refine the node `(map, entry)` from the live map view, once per arrival.
    pub fn observe(&mut self, map: Map, entry: Point8, tile_map: &MetaTileMap) {
        let edges = Self::edges_from_reachable(tile_map, map);
        self.adjacency.insert((map, entry), edges);
        let from = self.arrival.map(|a| a.map).filter(|&previous| previous != map);
        self.arrival = Some(Arrival { map, at: tile_map.player_position, from });
    }

    /// `None` before the first arrival this process sees, so a resumed run does not know until it
    /// changes map.
    pub fn arrival(&self) -> Option<Arrival> {
        self.arrival
    }

    pub fn neighbors(&self, map: Map) -> Vec<Edge> {
        self.adjacency.iter()
            .filter(|((m, _), _)| *m == map)
            .flat_map(|(_, edges)| edges)
            .copied()
            .collect()
    }

    pub fn map_count(&self) -> usize {
        self.adjacency.keys().map(|(m, _)| *m).collect::<HashSet<_>>().len()
    }

    fn bfs_to_map(&self, starts: &[(Map, Point8)], to: Map) -> Option<Vec<MapStep>> {
        self.bfs_nodes(starts, to)
            .map(|nodes| nodes.into_iter().map(|((m, _), via)| MapStep {
                map: m, via: via.map(|(kind, _)| kind), via_at: via.map(|(_, at)| at),
            }).collect())
    }

    fn bfs_nodes(&self, starts: &[(Map, Point8)], to: Map) -> Option<Vec<((Map, Point8), Option<(EdgeKind, Point8)>)>> {
        type Node = (Map, Point8);
        let mut dist: HashMap<Node, u32> = HashMap::new();
        let mut came_from: HashMap<Node, (Node, (EdgeKind, Point8))> = HashMap::new();
        let mut queue: VecDeque<Node> = VecDeque::new();

        // Snap to the nearest observed node, or keep the raw position when none is near.
        let resolve = |map: Map, pos: Point8| -> Point8 {
            if self.adjacency.contains_key(&(map, pos)) {
                return pos;
            }
            const SNAP_THRESHOLD: i32 = 8;
            self.adjacency.keys()
                .filter(|(m, _)| *m == map)
                .map(|(_, p)| *p)
                .map(|p| (p, (p.x as i32 - pos.x as i32).abs() + (p.y as i32 - pos.y as i32).abs()))
                .filter(|(_, d)| *d <= SNAP_THRESHOLD)
                .min_by_key(|(_, d)| *d)
                .map(|(p, _)| p)
                .unwrap_or(pos)
        };

        let start_set: HashSet<Node> = starts.iter()
            .map(|&(m, p)| (m, resolve(m, p)))
            .collect();

        for &s in &start_set {
            if dist.insert(s, 0).is_none() {
                queue.push_back(s);
            }
        }

        while let Some((m, p)) = queue.pop_front() {
            let cost = dist[&(m, p)];
            for edge in self.adjacency.get(&(m, p)).map(Vec::as_slice).unwrap_or(&[]) {
                // A connection's `to_position` can be a tile or two off the keyed landing, so snap.
                let next = (edge.to.map, resolve(edge.to.map, edge.to.location));
                if let std::collections::hash_map::Entry::Vacant(e) = dist.entry(next) {
                    e.insert(cost + 1);
                    came_from.insert(next, ((m, p), (edge.kind, edge.from.location)));
                    queue.push_back(next);
                }
            }
        }

        let goal = dist.keys()
            .filter(|(m, _)| *m == to)
            .min_by_key(|n| dist[n])
            .copied()?;

        let mut path_rev: Vec<(Node, Option<(EdgeKind, Point8)>)> = vec![(goal, None)];
        let mut current = goal;
        while !start_set.contains(&current) {
            let &(prev, kind) = came_from.get(&current)?;
            path_rev.last_mut().unwrap().1 = Some(kind);
            path_rev.push((prev, None));
            current = prev;
        }
        path_rev.reverse();

        Some(path_rev)
    }

    pub fn nodes(&self) -> Vec<((Map, Point8), Vec<Edge>)> {
        self.adjacency.iter().map(|(k, v)| (*k, v.clone())).collect()
    }

    /// Shortest path from any entry section of `from` to `to`.
    pub fn shortest_path(&self, from: Map, to: Map) -> Option<Vec<MapStep>> {
        if from == to {
            return Some(vec![MapStep { map: from, via: None, via_at: None }]);
        }
        let starts: Vec<(Map, Point8)> = self.adjacency.keys()
            .filter(|(m, _)| *m == from)
            .copied()
            .collect();
        self.bfs_to_map(&starts, to)
    }

    pub fn shortest_path_from_entry(&self, from: Map, from_entry: Point8, to: Map) -> Option<usize> {
        if from == to {
            return Some(1);
        }
        self.bfs_to_map(&[(from, from_entry)], to).map(|p| p.len())
    }

    /// Pick the action from `actions` that leads most directly toward `target`.
    pub fn pick_shortest_path_action(&self, actions: &[OverworldAction], target: Map) -> Option<OverworldAction> {
        actions.iter()
            .filter_map(|a| {
                let (to_map, to_position) = match a.tile {
                    MetaTile::Connection { to_map, to_position } => (to_map, to_position),
                    MetaTile::Warp      { to_map, to_position } => (to_map, to_position),
                    _ => return None,
                };
                // From the exact landing section, or Route 2's two halves would short-cut each other.
                let d = self.shortest_path_from_entry(to_map, to_position, target)?;
                Some((d, a.clone()))
            })
            .min_by_key(|(d, _)| *d)
            .map(|(_, a)| a)
    }
}

#[cfg(test)]
impl WorldGraph {
    /// Record a section with fixed edges, as `observe` would from a live map.
    pub(crate) fn observe_edges(&mut self, map: Map, entry: Point8, edges: &[(Point8, Map, Point8, EdgeKind)]) {
        let edges = edges
            .iter()
            .map(|&(from, to_map, to, kind)| Edge {
                from: MapCoordinates::new(map, from),
                to: MapCoordinates::new(to_map, to),
                kind,
            })
            .collect();
        self.adjacency.insert((map, entry), edges);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: u8, y: u8) -> Point8 { Point8 { x, y } }

    /// An arrival remembers where the player landed and the map before.
    #[test]
    fn an_arrival_remembers_where_it_came_from() {
        use crate::pokemon::integration_tests::fixture::TestFixture;
        let mut fixture = TestFixture::new(
            include_bytes!("../pokemon/data/pallet-town-state.bin"), std::time::Duration::from_secs(10), vec![]);
        let state = fixture.game_state();
        let mut g = WorldGraph::new();
        assert_eq!(g.arrival(), None);
        g.observe(Map::Route1, p(9, 8), &state.map);
        assert_eq!(g.arrival(), Some(Arrival { map: Map::Route1, at: state.map.player_position, from: None }));
        g.observe(Map::PalletTown, p(9, 0), &state.map);
        assert_eq!(g.arrival().map(|a| (a.map, a.from)), Some((Map::PalletTown, Some(Map::Route1))));
    }

    /// Pallet ⇄ Route 1 ⇄ Viridian, Pallet ⇄ Oak's Lab, Red's house by warps, and a dead-end Route 21.
    fn small_world() -> WorldGraph {
        use EdgeKind::*;
        let mut g = WorldGraph::new();
        g.observe_edges(Map::PalletTown, p(9, 7), &[
            (p(9, 0),  Map::Route1,      p(9, 8),  Connection),
            (p(12, 11), Map::OaksLab,    p(4, 11), Warp),
            (p(5, 5),  Map::RedsHouse1F, p(3, 7),  Warp),
            (p(9, 17), Map::Route21,     p(9, 0),  Connection),
        ]);
        g.observe_edges(Map::Route1, p(9, 8), &[
            (p(9, 17), Map::PalletTown,  p(9, 0),  Connection),
            (p(9, 0),  Map::ViridianCity, p(9, 17), Connection),
        ]);
        g.observe_edges(Map::ViridianCity, p(9, 17), &[
            (p(9, 17), Map::Route1,      p(9, 0),  Connection),
        ]);
        g.observe_edges(Map::OaksLab, p(4, 11), &[
            (p(4, 11), Map::PalletTown,  p(12, 11), Warp),
        ]);
        g.observe_edges(Map::RedsHouse1F, p(3, 7), &[
            (p(7, 1),  Map::RedsHouse2F, p(7, 1),  Warp),
        ]);
        g.observe_edges(Map::RedsHouse2F, p(7, 1), &[
            (p(7, 1),  Map::RedsHouse1F, p(3, 7),  Warp),
        ]);
        g.observe_edges(Map::Route21, p(9, 0), &[]); // dead-end (nothing observed onward)
        g
    }

    #[test]
    fn empty_graph_has_no_paths() {
        let g = WorldGraph::new();
        assert!(g.shortest_path(Map::PalletTown, Map::Route1).is_none());
        assert_eq!(g.shortest_path(Map::Route1, Map::Route1).unwrap().len(), 1);
    }

    #[test]
    fn observe_records_edges() {
        let g = small_world();
        let neighbors = g.neighbors(Map::PalletTown);
        assert!(neighbors.iter().any(|e| e.to.map == Map::Route1 && e.kind == EdgeKind::Connection));
        assert!(neighbors.iter().any(|e| e.to.map == Map::OaksLab && e.kind == EdgeKind::Warp));
    }

    #[test]
    fn trivial_path_same_map() {
        let g = small_world();
        let path = g.shortest_path(Map::PalletTown, Map::PalletTown).unwrap();
        assert_eq!(path, vec![MapStep { map: Map::PalletTown, via: None, via_at: None }]);
    }

    #[test]
    fn path_single_connection() {
        let g = small_world();
        let path = g.shortest_path(Map::PalletTown, Map::Route1).unwrap();
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].map, Map::PalletTown);
        assert_eq!(path[0].via, None);
        assert_eq!(path[1].map, Map::Route1);
        assert_eq!(path[1].via, Some(EdgeKind::Connection));
    }

    #[test]
    fn path_single_warp() {
        let g = small_world();
        let path = g.shortest_path(Map::PalletTown, Map::OaksLab).unwrap();
        assert_eq!(path.last().unwrap().map, Map::OaksLab);
        assert!(path.iter().any(|s| s.via == Some(EdgeKind::Warp)));
    }

    #[test]
    fn multi_hop_path() {
        let g = small_world();
        let path = g.shortest_path(Map::PalletTown, Map::ViridianCity).unwrap();
        let maps: Vec<_> = path.iter().map(|s| s.map).collect();
        assert_eq!(maps, vec![Map::PalletTown, Map::Route1, Map::ViridianCity]);
    }

    #[test]
    fn indoor_warp_only_path() {
        let g = small_world();
        let path = g.shortest_path(Map::RedsHouse1F, Map::RedsHouse2F).unwrap();
        for step in path.iter().skip(1) {
            assert_eq!(step.via, Some(EdgeKind::Warp));
        }
    }

    #[test]
    fn cyclic_graph_no_infinite_loop() {
        let g = small_world();
        let _ = g.shortest_path(Map::ViridianCity, Map::OaksLab);
    }

    #[test]
    fn no_path_to_unobserved_map() {
        let g = small_world();
        assert!(g.shortest_path(Map::PalletTown, Map::CeruleanCity).is_none());
        assert!(g.shortest_path(Map::Route21, Map::Route1).is_none());
    }

    #[test]
    fn disconnected_sections_keyed_separately() {
        use EdgeKind::Warp;
        let mut g = WorldGraph::new();
        g.observe_edges(Map::Route2, p(3, 5), &[(p(3, 5), Map::ViridianForest, p(5, 0), Warp)]);
        g.observe_edges(Map::Route2, p(3, 60), &[(p(3, 60), Map::PewterCity, p(14, 35), Warp)]);
        assert!(g.shortest_path_from_entry(Map::Route2, p(3, 5), Map::ViridianForest).is_some());
        assert!(g.shortest_path_from_entry(Map::Route2, p(3, 5), Map::PewterCity).is_none());
        assert!(g.shortest_path_from_entry(Map::Route2, p(3, 60), Map::PewterCity).is_some());
    }

    #[test]
    fn pick_shortest_path_action_routes_toward_target() {
        use crate::pokemon::tile::MetaTile;
        let g = small_world();
        let mk = |to_map: Map, to: Point8| OverworldAction {
            map: Map::PalletTown,
            origin: p(9, 7),
            destination: to,
            tile: MetaTile::Warp { to_map, to_position: to },
            route: vec![],
        };
        let actions = vec![
            mk(Map::OaksLab, p(4, 11)),
            OverworldAction {
                tile: MetaTile::Connection { to_map: Map::Route1, to_position: p(9, 8) },
                ..mk(Map::Route1, p(9, 8))
            },
        ];
        let chosen = g.pick_shortest_path_action(&actions, Map::ViridianCity).unwrap();
        assert_eq!(chosen.tile, MetaTile::Connection { to_map: Map::Route1, to_position: p(9, 8) });
    }
}

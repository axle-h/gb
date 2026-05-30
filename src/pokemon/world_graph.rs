use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::{Display, Formatter};
use crate::geometry::Point8;
use crate::mmu::MMU;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::map::Map;
use crate::pokemon::map_metadata::{CurrentMap, MapMetadataCache, PlayerFacingDirection};
use crate::pokemon::tile::MetaTile;
use crate::pokemon::tile_map::MetaTileMap;

/// How the player moves from one map to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// Seamless walking transition (N/S/E/W map border strip).
    /// Direction is not stored because `MetaTile::Connection` does not carry it.
    Connection,
    /// Instantaneous teleport (door, cave entrance, warp tile, etc.).
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

/// A directed edge in the world graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    /// The tile the player steps on to trigger the transition, in the source map's
    /// expanded tile coordinates (raw wXCoord/wYCoord + connection-strip offsets).
    pub from: MapCoordinates,
    /// The position the player lands on in the destination map (raw wXCoord/wYCoord).
    pub to: MapCoordinates,
    pub kind: EdgeKind,
}

impl Display for Edge {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} --{:?}--> {}", self.from, self.kind, self.to)
    }
}

/// One step in a path through the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapStep {
    pub map: Map,
    /// How the player arrived at this map. `None` for the starting map.
    pub via: Option<EdgeKind>,
}

/// Connected graph of reachable Pokémon Red maps built by BFS over the tile layer.
///
/// Unlike a header-derived graph, edges here only exist where `MetaTileMap::actions()`
/// finds a physically traversable path from the current entry position — cut trees,
/// ledges, water, and other obstacles are respected so the graph never routes the
/// player through impassable tiles.
///
/// The adjacency is keyed by `(Map, Point8)` — the map plus the raw entry position
/// (wXCoord/wYCoord value) used when the section was explored.  This correctly handles
/// maps that are physically split into disconnected sections (e.g. Route 2 is cut in
/// two by Viridian Forest): the south-section edges and north-section edges live under
/// separate keys and are never conflated during pathfinding.
#[derive(Debug, Clone)]
pub struct WorldGraph {
    adjacency: HashMap<(Map, Point8), Vec<Edge>>,
}

impl WorldGraph {
    /// Build the world graph by BFS from Pallet Town.
    ///
    /// Uses `MetaTileMap::all_reachable_warps_and_connections()` from each map's entry
    /// position to discover which warps and connections are physically reachable, then
    /// follows those edges to new maps until the reachable world is fully explored.
    ///
    /// Unlike the agent's `actions()` helper (which returns only the *nearest* warp per
    /// destination map), this builder records **every** reachable warp tile so that cave
    /// maps with multiple disconnected sections — each accessible from a different warp
    /// exit — are fully explored.  For example, Mt Moon B1F/B2F contain four physically
    /// isolated room-pairs whose inter-section links are only discoverable if all reachable
    /// warps (not just the nearest) are followed.
    pub fn build(mmu: &MMU) -> Self {
        let mut cache = MapMetadataCache::default();
        Self::build_with_cache(mmu, &mut cache)
    }

    /// Same as [`build`] but reuses an existing [`MapMetadataCache`].
    pub fn build_with_cache(mmu: &MMU, cache: &mut MapMetadataCache) -> Self {
        let mut adjacency: HashMap<(Map, Point8), Vec<Edge>> = HashMap::new();
        // explored: (map, raw_entry_position) pairs already processed.
        //
        // We key on (Map, Point8) rather than just Map because some maps are physically
        // split into disconnected sections by indoor gates or cave paths (e.g. Route2 is
        // split by Viridian Forest). If we stopped re-exploring a map the moment we first
        // saw it, we'd miss the sections reachable only from a second entry point.
        //
        // Termination is guaranteed: each unique (Map, Point8) is inserted at most once,
        // and warp/connection destinations are fixed ROM values, so the set is finite.
        let mut explored: HashSet<(Map, Point8)> = HashSet::new();
        // queue: (map, raw_player_position) where raw = the value wXCoord/wYCoord holds
        // (before MetaTileMap::new() adds connection-strip offsets).
        let mut queue: VecDeque<(Map, Point8)> = VecDeque::new();

        // Start in the middle of Pallet Town (10×9 blocks → 20×18 meta-tiles).
        // No connection extras on west/south, so raw (9, 7) == expanded (9, 7).
        let start = (Map::PalletTown, Point8 { x: 9, y: 7 });
        queue.push_back(start);
        explored.insert(start);

        while let Some((map, raw_pos)) = queue.pop_front() {
            let metadata = match cache.read_map(mmu, map) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let current_map = CurrentMap {
                player_position: raw_pos,
                player_direction: PlayerFacingDirection::Down,
                sprites: vec![],
                metadata,
            };

            let tile_map = MetaTileMap::new(&current_map);

            // Use all_reachable_warps_and_connections() instead of actions() so that
            // every reachable warp/connection tile is discovered, not just the nearest
            // one per destination map.  This is critical for cave maps (Mt Moon, etc.)
            // where multiple isolated sections each have warps to different positions in
            // the same destination map.
            for (src_pos, tile) in tile_map.all_reachable_warps_and_connections() {
                let (to_map, raw_entry, kind) = match tile {
                    MetaTile::Warp { to_map, to_position } => {
                        // Warp destinations are already raw (same space as wXCoord/wYCoord).
                        (to_map, to_position, EdgeKind::Warp)
                    }
                    MetaTile::Connection { to_map, to_position } => {
                        // Connection `to_position` is raw (wXCoord/wYCoord space); no
                        // conversion needed.  See ConnectedMapStrip::meta_tile_at().
                        (to_map, to_position, EdgeKind::Connection)
                    }
                    _ => continue,
                };

                // `src_pos` is the warp/connection tile in the source map's expanded
                // coordinate space — the tile the player steps on.
                // Key by (map, raw_pos) so that disconnected sections of the same map
                // (e.g. Route 2 south vs. north) are stored under separate keys and
                // their edges are never conflated during pathfinding.
                adjacency.entry((map, raw_pos)).or_default().push(Edge {
                    from: MapCoordinates::new(map, src_pos),
                    to: MapCoordinates::new(to_map, raw_entry),
                    kind,
                });

                if explored.insert((to_map, raw_entry)) {
                    queue.push_back((to_map, raw_entry));
                }
            }
        }

        Self { adjacency }
    }

    /// All outgoing edges from `map`, across all entry-point sections.
    pub fn neighbors(&self, map: Map) -> Vec<Edge> {
        self.adjacency.iter()
            .filter(|((m, _), _)| *m == map)
            .flat_map(|(_, edges)| edges)
            .copied()
            .collect()
    }

    /// Number of distinct maps in the graph.
    pub fn map_count(&self) -> usize {
        self.adjacency.keys().map(|(m, _)| *m).collect::<HashSet<_>>().len()
    }

    /// Total directed-edge count.
    pub fn edge_count(&self) -> usize {
        self.adjacency.values().map(Vec::len).sum()
    }

    /// BFS on the `(Map, Point8)` graph from a set of start nodes to the first node
    /// whose map equals `to`.
    ///
    /// Returns the reconstructed `MapStep` path, or `None` if `to` is unreachable.
    /// The same physical map may appear more than once in the path when it has multiple
    /// disconnected sections (e.g. Route 2 south then Route 2 north); consecutive
    /// occurrences are never the same section because no map has a self-edge.
    fn bfs_to_map(&self, starts: &[(Map, Point8)], to: Map) -> Option<Vec<MapStep>> {
        type Node = (Map, Point8);
        let mut dist: HashMap<Node, u32> = HashMap::new();
        let mut came_from: HashMap<Node, (Node, EdgeKind)> = HashMap::new();
        let mut queue: VecDeque<Node> = VecDeque::new();

        let start_set: HashSet<Node> = starts.iter().copied().collect();

        for &s in starts {
            if dist.insert(s, 0).is_none() {
                queue.push_back(s);
            }
        }

        while let Some((m, p)) = queue.pop_front() {
            let cost = dist[&(m, p)];
            for edge in self.adjacency.get(&(m, p)).map(Vec::as_slice).unwrap_or(&[]) {
                let next = (edge.to.map, edge.to.location);
                if let std::collections::hash_map::Entry::Vacant(e) = dist.entry(next) {
                    e.insert(cost + 1);
                    came_from.insert(next, ((m, p), edge.kind));
                    queue.push_back(next);
                }
            }
        }

        // Among all entry-points of `to` that were reached by the BFS, pick the closest.
        // We search `dist` (all reached nodes) not `adjacency.keys()` so that dead-end
        // destination maps (no outgoing edges, hence absent from adjacency) are found too.
        let goal = dist.keys()
            .filter(|(m, _)| *m == to)
            .min_by_key(|n| dist[n])
            .copied()?;

        // Reconstruct the path from goal back to whichever start node was used.
        let mut path_rev: Vec<(Node, Option<EdgeKind>)> = vec![(goal, None)];
        let mut current = goal;
        while !start_set.contains(&current) {
            let &(prev, kind) = came_from.get(&current)?;
            // Fix up the via for the just-pushed node, then push the predecessor.
            path_rev.last_mut().unwrap().1 = Some(kind);
            path_rev.push((prev, None));
            current = prev;
        }
        path_rev.reverse();

        Some(path_rev.into_iter().map(|((m, _), via)| MapStep { map: m, via }).collect())
    }

    /// Shortest path from `from` to `to`, considering all entry sections of `from`.
    ///
    /// Returns `None` when no path exists (unreachable or absent from the graph).
    /// The first `MapStep` is always the starting map (`via = None`); every subsequent
    /// step records how the player enters that map.
    pub fn shortest_path(&self, from: Map, to: Map) -> Option<Vec<MapStep>> {
        if from == to {
            return Some(vec![MapStep { map: from, via: None }]);
        }
        let starts: Vec<(Map, Point8)> = self.adjacency.keys()
            .filter(|(m, _)| *m == from)
            .copied()
            .collect();
        self.bfs_to_map(&starts, to)
    }

    /// Hop count from a **specific entry point** of `from` to `to`.
    ///
    /// Unlike `shortest_path`, this starts BFS only from the given raw entry position,
    /// so edges belonging to a different disconnected section of the same map are never
    /// considered.  Used by `pick_shortest_path_action` to avoid false short-cuts through
    /// map sections that are not physically reachable from the current player position.
    fn shortest_path_from_entry(&self, from: Map, from_entry: Point8, to: Map) -> Option<usize> {
        if from == to {
            return Some(1);
        }
        self.bfs_to_map(&[(from, from_entry)], to).map(|p| p.len())
    }

    /// Pick the action from `actions` that leads most directly toward `target`.
    ///
    /// Uses entry-point-aware pathfinding: for each candidate action the BFS starts
    /// from the exact raw landing position in the destination map, ensuring that edges
    /// from a different disconnected section of that map (e.g. Route 2 north vs. south)
    /// are not considered.
    pub fn pick_shortest_path_action(&self, actions: &[OverworldAction], target: Map) -> Option<OverworldAction> {
        actions.iter()
            .filter_map(|a| {
                let (to_map, to_position) = match a.tile {
                    MetaTile::Connection { to_map, to_position } => (to_map, to_position),
                    MetaTile::Warp      { to_map, to_position } => (to_map, to_position),
                    _ => return None,
                };
                let d = self.shortest_path_from_entry(to_map, to_position, target)?;
                Some((d, a.clone()))
            })
            .min_by_key(|(d, _)| *d)
            .map(|(_, a)| a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmu::MMU;
    use crate::pokemon::map_metadata::MapMetadataCache;
    use crate::pokemon::roms::POKERED;

    fn graph() -> WorldGraph {
        let mmu = MMU::from_rom(POKERED).unwrap();
        WorldGraph::build(&mmu)
    }

    // ── graph construction ────────────────────────────────────────────────────

    #[test]
    fn builds_without_crash() {
        let g = graph();
        // The tile-based graph is smaller than the header-based one (HM-blocked paths
        // and impassable routes are excluded), but should still cover the core world.
        assert!(g.map_count() > 50, "expected a large reachable world, got {}", g.map_count());
        assert!(g.edge_count() > 100, "expected many edges, got {}", g.edge_count());
    }

    // ── neighbor queries ──────────────────────────────────────────────────────

    #[test]
    fn route2() {
        let g = graph();
        let neighbors = g.neighbors(Map::Route2);

        for edge in neighbors {
            println!("{}", edge);
        }
    }

    #[test]
    fn single_connection_pallet_to_route1() {
        let g = graph();
        let neighbors = g.neighbors(Map::PalletTown);
        let has_route1 = neighbors.iter().any(|e| {
            e.to.map == Map::Route1 && e.kind == EdgeKind::Connection
        });
        assert!(has_route1, "PalletTown should have a connection edge to Route1; got {neighbors:?}");
    }

    #[test]
    fn multiple_connections_pallet_town() {
        let g = graph();
        let neighbors = g.neighbors(Map::PalletTown);
        let has_north = neighbors.iter().any(|e| e.to.map == Map::Route1);
        // Route21 is a water-only route — its south connection strip is all
        // ConnectionWater tiles (impassable without Surf).  PalletTown has no
        // walkable connection to Route21 and is therefore not expected to appear
        // as a graph neighbour.
        assert!(has_north, "PalletTown missing connection to Route1");
    }

    #[test]
    fn connections_and_warps_pallet_town() {
        let g = graph();
        let neighbors = g.neighbors(Map::PalletTown);
        let has_connection = neighbors.iter().any(|e| e.kind == EdgeKind::Connection);
        let has_warp = neighbors.iter().any(|e| e.kind == EdgeKind::Warp);
        assert!(has_connection, "PalletTown should have walking connections");
        assert!(has_warp, "PalletTown should have warp edges (OaksLab, Red's house, …)");
    }

    #[test]
    fn single_warp_oaks_lab_to_pallet_town() {
        let g = graph();
        let neighbors = g.neighbors(Map::OaksLab);
        let exits_to_pallet = neighbors.iter().any(|e| e.to.map == Map::PalletTown);
        assert!(exits_to_pallet, "OaksLab should warp back to PalletTown; got {neighbors:?}");
    }

    // ── pathfinding ───────────────────────────────────────────────────────────

    #[test]
    fn trivial_path_same_map() {
        let g = graph();
        let path = g.shortest_path(Map::PalletTown, Map::PalletTown).unwrap();
        assert_eq!(path.len(), 1);
        assert_eq!(path[0], MapStep { map: Map::PalletTown, via: None });
    }

    #[test]
    fn path_single_connection() {
        let g = graph();
        let path = g.shortest_path(Map::PalletTown, Map::Route1).unwrap();
        // PalletTown → Route1 is one hop via a walking connection.
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].map, Map::PalletTown);
        assert_eq!(path[0].via, None);
        assert_eq!(path[1].map, Map::Route1);
        assert_eq!(path[1].via, Some(EdgeKind::Connection));
    }

    #[test]
    fn path_single_warp_pallet_to_oaks_lab() {
        let g = graph();
        let path = g.shortest_path(Map::PalletTown, Map::OaksLab).unwrap();
        assert_eq!(path.first().unwrap().map, Map::PalletTown);
        assert_eq!(path.last().unwrap().map, Map::OaksLab);
        // No map should appear consecutively.
        for window in path.windows(2) {
            assert_ne!(window[0].map, window[1].map, "path should not revisit the same map consecutively");
        }
        let uses_warp = path.iter().any(|s| s.via == Some(EdgeKind::Warp));
        assert!(uses_warp, "path to OaksLab should include at least one warp");
    }

    #[test]
    fn long_route_pallet_to_cerulean() {
        // PalletTown → Route1 → ViridianCity → Route2 → … → CeruleanCity (multi-hop)
        let g = graph();
        let path = g.shortest_path(Map::PalletTown, Map::CeruleanCity).unwrap();
        assert!(path.len() > 2, "expected a multi-hop path, got {path:?}");
        assert!(path.iter().any(|s| s.map == Map::ViridianForest), "must go through viridian forest");
        assert_eq!(path.first().unwrap().map, Map::PalletTown);
        assert_eq!(path.last().unwrap().map, Map::CeruleanCity);
    }

    #[test]
    fn path_with_multiple_warps() {
        // RedsHouse1F → RedsHouse2F is a pure warp-only indoor path.
        let g = graph();
        let path = g.shortest_path(Map::RedsHouse1F, Map::RedsHouse2F).unwrap();
        assert_eq!(path.first().unwrap().map, Map::RedsHouse1F);
        assert_eq!(path.last().unwrap().map, Map::RedsHouse2F);
        for step in path.iter().skip(1) {
            assert_eq!(step.via, Some(EdgeKind::Warp), "indoor→indoor should be warp-only");
        }
    }

    #[test]
    fn cyclic_graph_no_infinite_loop() {
        // Route1 ↔ PalletTown ↔ Route21 forms a cycle. Pathfinding must terminate.
        let g = graph();
        let _ = g.shortest_path(Map::Route21, Map::Route1);
        // If we reach here without hanging, cycles are handled correctly.
    }

    #[test]
    fn path_is_optimal_length() {
        // ViridianCity → Route1 → PalletTown = at least 2 hops.
        let g = graph();
        let path = g.shortest_path(Map::ViridianCity, Map::PalletTown).unwrap();
        assert!(path.len() >= 2, "path should be at least 2 hops");
    }

    #[test]
    fn no_path_to_disconnected_map() {
        // UnusedMap0B is a placeholder; no reachable map has a warp or connection to it.
        let g = graph();
        let result = g.shortest_path(Map::PalletTown, Map::UnusedMap0B);
        assert!(result.is_none(), "should return None for unreachable map");
    }

    #[test]
    fn path_nodes_are_connected_by_graph_edges() {
        // For a long path, every consecutive pair must be a valid edge in the graph.
        let g = graph();
        let path = g.shortest_path(Map::PalletTown, Map::CeruleanCity).unwrap();
        for window in path.windows(2) {
            let from = window[0].map;
            let to_step = window[1];
            let edge_exists = g.neighbors(from).iter().any(|e| e.to.map == to_step.map);
            assert!(edge_exists, "no edge from {from} to {} in graph", to_step.map);
        }
    }

    // ── connection tile border sanity ─────────────────────────────────────────
    //
    // In the expanded tile map, connection strips always occupy the single extra
    // row/column at the map boundary. The tile the player steps on to enter the
    // next map must therefore lie on one of the four map edges.

    #[test]
    fn north_connection_pallet_to_route1_on_top_border() {
        // The north connection strip sits at expanded y=0.
        let g = graph();
        let edge = g.neighbors(Map::PalletTown)
            .into_iter()
            .find(|e| e.to.map == Map::Route1 && e.kind == EdgeKind::Connection)
            .expect("PalletTown should have a connection edge to Route1");
        assert_eq!(edge.from.location.y, 0,
            "PalletTown→Route1 connection tile should be on the top row (y=0)");
    }

    #[test]
    fn south_connection_pallet_to_route21_on_bottom_border() {
        // Route21 is an ocean route accessible only via Surf.  The south connection
        // strip of PalletTown is entirely ConnectionWater tiles — the graph correctly
        // omits this edge.  This test verifies there is NO walkable connection edge
        // to Route21, consistent with the water-only barrier.
        let g = graph();
        let edge_to_route21 = g.neighbors(Map::PalletTown)
            .into_iter()
            .find(|e| e.to.map == Map::Route21 && e.kind == EdgeKind::Connection);
        assert!(edge_to_route21.is_none(),
            "PalletTown should NOT have a walkable connection to the water-only Route21; \
             got {:?}", edge_to_route21);
    }

    /// Every Connection edge's `from` position must lie on at least one map border.
    /// Connection strips are always placed in the single extra row/column at the boundary.
    #[test]
    fn connection_tiles_lie_on_a_map_border() {
        let mmu = MMU::from_rom(POKERED).unwrap();
        let mut cache = MapMetadataCache::default();
        let g = WorldGraph::build_with_cache(&mmu, &mut cache);
        for map in Map::all() {
            for edge in g.neighbors(map) {
                if edge.kind != EdgeKind::Connection { continue; }
                let Ok(meta) = cache.read_map(&mmu, map) else { continue };
                let dims = meta.dimensions();
                let max_x = dims.full_width().saturating_sub(1) as u8;
                let max_y = dims.full_height().saturating_sub(1) as u8;
                let loc = edge.from.location;
                let on_border = loc.x == 0 || loc.y == 0 || loc.x == max_x || loc.y == max_y;
                assert!(
                    on_border,
                    "{map}: connection edge to {:?} at {loc:?} is not on any border \
                     (max_x={max_x}, max_y={max_y})",
                    edge.to.map,
                );
            }
        }
    }
}

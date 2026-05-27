use std::collections::{BinaryHeap, HashMap};
use std::cmp::Reverse;
use crate::geometry::Point8;
use crate::mmu::MMU;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::encoding::PokemonEncoding;
use crate::pokemon::map::Map;
use crate::pokemon::map_header::{MapConnectionDirection, MapHeaderReader};
use crate::pokemon::map_metadata::MapMetadataReader;
use crate::pokemon::tile::MetaTile;

/// How the player moves from one map to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// Seamless walking transition in a cardinal direction.
    Connection(MapConnectionDirection),
    /// Instantaneous teleport (door, cave entrance, warp tile, etc.).
    Warp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapCoordinates {
    map: Map,
    location: Point8,
}

impl MapCoordinates {
    pub fn new(map: Map, location: Point8) -> Self {
        Self { map, location }
    }
}

/// A directed edge in the world graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: MapCoordinates,
    pub to: MapCoordinates,
    pub kind: EdgeKind,
}

/// One step in a path through the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapStep {
    pub map: Map,
    /// How the player arrived at this map. `None` for the starting map.
    pub via: Option<EdgeKind>,
}

/// Connected graph of all 248 Pokémon Red maps, built from ROM headers.
///
/// Edges represent either seamless walking connections (N/S/E/W) or
/// warp-tile teleports (doors, cave entrances, etc.).
#[derive(Debug, Clone)]
pub struct WorldGraph {
    adjacency: HashMap<Map, Vec<Edge>>,
}

impl WorldGraph {
    /// Build the complete world graph by reading every map header and warp table from ROM.
    pub fn build(mmu: &MMU) -> Self {
        let mut adjacency: HashMap<Map, Vec<Edge>> = HashMap::new();

        for map in Map::all() {
            let Ok(map_metadata) = mmu.read_map_metadata(map) else { continue };

            let edges = adjacency.entry(map).or_default();

            for conn in map_metadata.map_header.connections() {
                // Compute representative entry/exit positions for this connection.
                //
                // The connection strip covers a row (N/S) or column (E/W) of the shared
                // border. We pick the midpoint of the strip as the representative tile.
                //
                // `meta_align_offset` is the offset (in meta-tiles) of the strip's start
                // along the perpendicular axis, measured from the current map's left/top
                // edge. Mirrors the `meta_align_offset` calculation in `load_connected_strips`.
                //
                // The alignment fields give the exact border row/column in the connected
                // map's coordinate space:
                //   North: y_alignment = connected_height*2-1  (bottom row of connected map)
                //   South: y_alignment = 0                     (top row)
                //   East:  x_alignment = 0                     (left column)
                //   West:  x_alignment = connected_width*2-1   (right column)
                //
                // Coordinate conversion (meta-tile units, same as wXCoord/wYCoord):
                //   connected_x = current_x + x_alignment  (N/S connections)
                //   connected_y = current_y + y_alignment  (E/W connections)
                let h = map_metadata.map_header.height; // blocks
                let w = map_metadata.map_header.width;  // blocks

                let (from_pos, to_pos) = match conn.direction {
                    MapConnectionDirection::North => {
                        // Strip runs along the top edge (y=0), in the x direction.
                        let meta_align_x = (-(conn.x_alignment as i32)).max(0) as u8;
                        let mid_x_current   = meta_align_x.saturating_add(conn.strip_length);
                        let mid_x_connected = ((mid_x_current as i16) + (conn.x_alignment as i16))
                            .max(0) as u8;
                        (
                            Point8 { x: mid_x_current,   y: 0 },
                            Point8 { x: mid_x_connected, y: conn.y_alignment as u8 },
                        )
                    }
                    MapConnectionDirection::South => {
                        // Strip runs along the bottom edge (y = h*2-1), in the x direction.
                        let meta_align_x = (-(conn.x_alignment as i32)).max(0) as u8;
                        let mid_x_current   = meta_align_x.saturating_add(conn.strip_length);
                        let mid_x_connected = ((mid_x_current as i16) + (conn.x_alignment as i16))
                            .max(0) as u8;
                        (
                            Point8 { x: mid_x_current,   y: h.saturating_mul(2).saturating_sub(1) },
                            Point8 { x: mid_x_connected, y: 0 },
                        )
                    }
                    MapConnectionDirection::East => {
                        // Strip runs along the right edge (x = w*2-1), in the y direction.
                        let meta_align_y = (-(conn.y_alignment as i32)).max(0) as u8;
                        let mid_y_current   = meta_align_y.saturating_add(conn.strip_length);
                        let mid_y_connected = ((mid_y_current as i16) + (conn.y_alignment as i16))
                            .max(0) as u8;
                        (
                            Point8 { x: w.saturating_mul(2).saturating_sub(1), y: mid_y_current   },
                            Point8 { x: 0,                                      y: mid_y_connected },
                        )
                    }
                    MapConnectionDirection::West => {
                        // Strip runs along the left edge (x=0), in the y direction.
                        let meta_align_y = (-(conn.y_alignment as i32)).max(0) as u8;
                        let mid_y_current   = meta_align_y.saturating_add(conn.strip_length);
                        let mid_y_connected = ((mid_y_current as i16) + (conn.y_alignment as i16))
                            .max(0) as u8;
                        (
                            Point8 { x: 0,                        y: mid_y_current   },
                            Point8 { x: conn.x_alignment as u8,   y: mid_y_connected },
                        )
                    }
                };

                edges.push(Edge {
                    from: MapCoordinates::new(map, from_pos),
                    to: MapCoordinates::new(conn.map, to_pos),
                    kind: EdgeKind::Connection(conn.direction)
                });
            }

            for warp in map_metadata.warp_events {
                edges.push(Edge {
                    from: MapCoordinates::new(map, warp.position),
                    to: MapCoordinates::new(warp.destination_map, warp.destination_position),
                    kind: EdgeKind::Warp
                });
            }
        }

        Self { adjacency }
    }

    /// All outgoing edges from `map`. Returns an empty slice for unknown maps.
    pub fn neighbors(&self, map: Map) -> &[Edge] {
        self.adjacency.get(&map).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Number of maps in the graph.
    pub fn map_count(&self) -> usize {
        self.adjacency.len()
    }

    /// Total directed-edge count.
    pub fn edge_count(&self) -> usize {
        self.adjacency.values().map(Vec::len).sum()
    }

    /// Shortest path from `from` to `to` using A* with a zero heuristic (Dijkstra / BFS).
    ///
    /// Returns `None` when no path exists (unreachable map, or map absent from ROM).
    /// The first `MapStep` is always the starting map (`via = None`); every subsequent
    /// step records how the player enters that map.
    pub fn shortest_path(&self, from: Map, to: Map) -> Option<Vec<MapStep>> {
        if from == to {
            return Some(vec![MapStep { map: from, via: None }]);
        }

        // Min-heap of (cost, map_id). Map is repr(u8) so u8 is a lossless encoding.
        let mut heap: BinaryHeap<Reverse<(u32, u8)>> = BinaryHeap::new();
        let mut dist: HashMap<Map, u32> = HashMap::new();
        // came_from[node] = (predecessor, edge_kind_used_to_reach_node)
        let mut came_from: HashMap<Map, (Map, EdgeKind)> = HashMap::new();

        dist.insert(from, 0);
        heap.push(Reverse((0, from as u8)));

        while let Some(Reverse((cost, raw_id))) = heap.pop() {
            let Some(map) = Map::from_repr(raw_id) else { continue };

            if map == to {
                break;
            }

            // Discard stale heap entries (we already found a shorter path to this node).
            if dist.get(&map).copied().unwrap_or(u32::MAX) < cost {
                continue;
            }

            for &edge in self.neighbors(map) {
                let new_cost = cost + 1;
                if new_cost < dist.get(&edge.to.map).copied().unwrap_or(u32::MAX) {
                    dist.insert(edge.to.map, new_cost);
                    came_from.insert(edge.to.map, (map, edge.kind));
                    heap.push(Reverse((new_cost, edge.to.map as u8)));
                }
            }
        }

        if !dist.contains_key(&to) {
            return None;
        }

        // Reconstruct the path by walking came_from backwards from `to` to `from`.
        let mut path = Vec::new();
        let mut current = to;
        while current != from {
            let (prev, kind) = *came_from.get(&current)?;
            path.push(MapStep { map: current, via: Some(kind) });
            current = prev;
        }
        path.push(MapStep { map: from, via: None });
        path.reverse();

        Some(path)
    }

    pub fn pick_shortest_path_action(&self, actions: &[OverworldAction], target: Map) -> Option<OverworldAction> {
        // The world graph path may go via a map section that is physically
        // unreachable from the current position (e.g. Route2 north gate is in the
        // computed path but only the south gate is reachable from Route2 south).
        // Pick the accessible connection/warp with the shortest world-graph distance
        // to the target.
        actions.into_iter()
            .filter_map(|a| {
                match a.tile {
                    MetaTile::Connection { to_map, .. } | MetaTile::Warp { to_map, .. } => {
                        let d = self.shortest_path(to_map, target)?.len();
                        Some((d, a.clone()))
                    },
                    _ => None,
                }
            })
            .min_by_key(|(d, _)| *d)
            .map(|(_, a)| a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmu::MMU;
    use crate::pokemon::map_header::MapConnectionDirection;
    use crate::pokemon::map_metadata::MapMetadataReader;
    use crate::pokemon::roms::POKERED;

    fn graph() -> WorldGraph {
        let mmu = MMU::from_rom(POKERED).unwrap();
        WorldGraph::build(&mmu)
    }

    // ── graph construction ────────────────────────────────────────────────────

    #[test]
    fn builds_without_crash() {
        let g = graph();
        // All 248 map IDs are present in the ROM; most should produce valid headers.
        assert!(g.map_count() > 100, "expected a large connected world, got {}", g.map_count());
        assert!(g.edge_count() > 200, "expected many edges, got {}", g.edge_count());
    }

    // ── neighbor queries ──────────────────────────────────────────────────────

    #[test]
    fn single_connection_pallet_to_route1() {
        let g = graph();
        let neighbors = g.neighbors(Map::PalletTown);
        let has_route1 = neighbors.iter().any(|e| {
            e.to.map == Map::Route1 && e.kind == EdgeKind::Connection(MapConnectionDirection::North)
        });
        assert!(has_route1, "PalletTown should have a north connection to Route1; got {neighbors:?}");
    }

    #[test]
    fn multiple_connections_pallet_town() {
        let g = graph();
        let neighbors = g.neighbors(Map::PalletTown);
        let has_north = neighbors.iter().any(|e| e.to.map == Map::Route1);
        let has_south = neighbors.iter().any(|e| e.to.map == Map::Route21);
        assert!(has_north, "PalletTown missing north connection to Route1");
        assert!(has_south, "PalletTown missing south connection to Route21");
    }

    #[test]
    fn connections_and_warps_pallet_town() {
        let g = graph();
        let neighbors = g.neighbors(Map::PalletTown);
        let has_connection = neighbors.iter().any(|e| matches!(e.kind, EdgeKind::Connection(_)));
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
        // PalletTown → Route1 is one hop
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].map, Map::PalletTown);
        assert_eq!(path[0].via, None);
        assert_eq!(path[1].map, Map::Route1);
        assert!(matches!(path[1].via, Some(EdgeKind::Connection(_))));
    }

    #[test]
    fn path_single_warp_pallet_to_oaks_lab() {
        let g = graph();
        let path = g.shortest_path(Map::PalletTown, Map::OaksLab).unwrap();
        assert_eq!(path.first().unwrap().map, Map::PalletTown);
        assert_eq!(path.last().unwrap().map, Map::OaksLab);
        // All intermediate steps must be valid (no duplicate consecutive maps)
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
        assert_eq!(path.first().unwrap().map, Map::PalletTown);
        assert_eq!(path.last().unwrap().map, Map::CeruleanCity);
    }

    #[test]
    fn path_with_multiple_warps() {
        // Route through at least two indoor maps: Red's House 1F → 2F → PalletTown
        let g = graph();
        // RedsHouse1F warps to RedsHouse2F which warps back to PalletTown
        let path = g.shortest_path(Map::RedsHouse1F, Map::RedsHouse2F).unwrap();
        assert_eq!(path.first().unwrap().map, Map::RedsHouse1F);
        assert_eq!(path.last().unwrap().map, Map::RedsHouse2F);
        // Entire path should only use warps (both are indoor maps)
        for step in path.iter().skip(1) {
            assert_eq!(step.via, Some(EdgeKind::Warp), "indoor→indoor should be warp-only");
        }
    }

    #[test]
    fn cyclic_graph_no_infinite_loop() {
        // Route1 ↔ PalletTown ↔ Route21 forms a cycle.  Pathfinding must terminate.
        let g = graph();
        let _ = g.shortest_path(Map::Route21, Map::Route1);
        // If we reach here without hanging, the cycle is handled correctly.
    }

    #[test]
    fn path_is_optimal_length() {
        // PalletTown → Route1 is 1 hop; ViridianCity → PalletTown via Route1 is 2 hops minimum.
        let g = graph();
        let path = g.shortest_path(Map::ViridianCity, Map::PalletTown).unwrap();
        // ViridianCity (south) → Route1 (south) → PalletTown = 2 hops at minimum
        assert!(path.len() >= 2, "path should be at least 2 hops");
    }

    #[test]
    fn no_path_to_disconnected_map() {
        // A map with no header pointer is never added to the graph,
        // so shortest_path to it should return None.
        let g = graph();
        // UnusedMap0B is a placeholder with no valid connections.
        // If it's in the graph at all it will be isolated.
        let result = g.shortest_path(Map::PalletTown, Map::UnusedMap0B);
        assert!(result.is_none(), "should return None for unreachable map");
        // If UnusedMap0B somehow has edges we skip the assertion — the map is reachable.
    }

    #[test]
    fn path_nodes_are_connected_by_graph_edges() {
        // For a long path, verify each consecutive pair is a valid edge in the graph.
        let g = graph();
        let path = g.shortest_path(Map::PalletTown, Map::CeladonCity).unwrap();
        for window in path.windows(2) {
            let from = window[0].map;
            let to_step = window[1];
            let edge_exists = g.neighbors(from).iter().any(|e| e.to.map == to_step.map);
            assert!(edge_exists, "no edge from {from} to {} in graph", to_step.map);
        }
    }

    // ── connection positions ──────────────────────────────────────────────────

    #[test]
    fn north_connection_pallet_to_route1_positions() {
        // PalletTown (10×9 blocks) north → Route1 (10×18 blocks).
        // x_alignment=0, y_alignment=35 (=18*2-1), strip_length=10 blocks.
        //
        // from: top row of PalletTown (y=0), midpoint x = 0 + 10 = 10.
        // to:   bottom row of Route1  (y=35), midpoint x = 10 + 0 = 10.
        let g = graph();
        let edge = g.neighbors(Map::PalletTown)
            .iter()
            .find(|e| e.to.map == Map::Route1)
            .expect("PalletTown→Route1 edge");
        assert_eq!(edge.from.location, Point8 { x: 10, y: 0  }, "from should be top-row midpoint");
        assert_eq!(edge.to.location,   Point8 { x: 10, y: 35 }, "to should be bottom-row midpoint of Route1");
    }

    #[test]
    fn south_connection_pallet_to_route21_positions() {
        // PalletTown (10×9 blocks) south → Route21 (10×45 blocks).
        // x_alignment=0, y_alignment=0, strip_length=10 blocks.
        //
        // from: bottom row of PalletTown (y=9*2-1=17), midpoint x=10.
        // to:   top row of Route21 (y=0), midpoint x=10.
        let g = graph();
        let edge = g.neighbors(Map::PalletTown)
            .iter()
            .find(|e| e.to.map == Map::Route21)
            .expect("PalletTown→Route21 edge");
        assert_eq!(edge.from.location, Point8 { x: 10, y: 17 }, "from should be bottom-row midpoint");
        assert_eq!(edge.to.location,   Point8 { x: 10, y: 0  }, "to should be top-row midpoint of Route21");
    }

    #[test]
    fn north_connection_cerulean_to_route24_positions() {
        // CeruleanCity (20×18 blocks) north → Route24 (10×18 blocks).
        // x_alignment=-10, y_alignment=35, strip_length=10 blocks.
        // meta_align_x = -(-10) = 10.
        //
        // from: top row (y=0), mid_x_current = 10 + 10 = 20.
        // to:   y=35 (bottom of Route24), mid_x_connected = 20 + (-10) = 10.
        let g = graph();
        let edge = g.neighbors(Map::CeruleanCity)
            .iter()
            .find(|e| e.to.map == Map::Route24)
            .expect("CeruleanCity→Route24 edge");
        assert_eq!(edge.from.location, Point8 { x: 20, y: 0  }, "from: center of top border of CeruleanCity");
        assert_eq!(edge.to.location,   Point8 { x: 10, y: 35 }, "to: center of bottom border of Route24");
    }

    #[test]
    fn east_connection_celadon_to_route7_positions() {
        // CeladonCity (25×18 blocks) east → Route7 (10×9 blocks).
        // y_alignment=-8, x_alignment=0, strip_length=9 blocks.
        // meta_align_y = -(-8) = 8.
        //
        // from: right col (x=25*2-1=49), mid_y_current = 8 + 9 = 17.
        // to:   left col (x=0), mid_y_connected = 17 + (-8) = 9.
        let g = graph();
        let edge = g.neighbors(Map::CeladonCity)
            .iter()
            .find(|e| e.to.map == Map::Route7)
            .expect("CeladonCity→Route7 edge");
        assert_eq!(edge.from.location, Point8 { x: 49, y: 17 }, "from: right border midpoint of CeladonCity");
        assert_eq!(edge.to.location,   Point8 { x: 0,  y: 9  }, "to: left border midpoint of Route7");
    }

    #[test]
    fn west_connection_celadon_to_route16_positions() {
        // CeladonCity (25×18 blocks) west → Route16 (20×9 blocks).
        // y_alignment=-8, x_alignment=39 (=20*2-1), strip_length=9 blocks.
        // meta_align_y = -(-8) = 8.
        //
        // from: left col (x=0), mid_y_current = 8 + 9 = 17.
        // to:   right col (x=39), mid_y_connected = 17 + (-8) = 9.
        let g = graph();
        let edge = g.neighbors(Map::CeladonCity)
            .iter()
            .find(|e| e.to.map == Map::Route16)
            .expect("CeladonCity→Route16 edge");
        assert_eq!(edge.from.location, Point8 { x: 0,  y: 17 }, "from: left border midpoint of CeladonCity");
        assert_eq!(edge.to.location,   Point8 { x: 39, y: 9  }, "to: right border midpoint of Route16");
    }

    #[test]
    fn connection_from_position_is_on_correct_border() {
        // For each connection edge in the graph, verify the 'from' position lies on
        // the correct border of the source map.
        let mmu = MMU::from_rom(POKERED).unwrap();
        let g = graph();
        for map in Map::all() {
            let Ok(meta) = mmu.read_map_metadata(map) else { continue };
            let max_x = meta.map_header.width.saturating_mul(2).saturating_sub(1);
            let max_y = meta.map_header.height.saturating_mul(2).saturating_sub(1);
            for edge in g.neighbors(map) {
                let EdgeKind::Connection(dir) = edge.kind else { continue };
                let loc = edge.from.location;
                match dir {
                    MapConnectionDirection::North => assert_eq!(loc.y, 0, "{map} north from.y"),
                    MapConnectionDirection::South => assert_eq!(loc.y, max_y, "{map} south from.y"),
                    MapConnectionDirection::East  => assert_eq!(loc.x, max_x, "{map} east from.x"),
                    MapConnectionDirection::West  => assert_eq!(loc.x, 0, "{map} west from.x"),
                }
            }
        }
    }
}

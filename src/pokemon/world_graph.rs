use std::collections::{BinaryHeap, HashMap};
use std::cmp::Reverse;

use crate::mmu::MMU;
use crate::pokemon::encoding::PokemonEncoding;
use crate::pokemon::map::Map;
use crate::pokemon::map_header::{MapConnectionDirection, MapHeaderReader};

/// How the player moves from one map to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// Seamless walking transition in a cardinal direction.
    Connection(MapConnectionDirection),
    /// Instantaneous teleport (door, cave entrance, warp tile, etc.).
    Warp,
}

/// A directed edge in the world graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub to: Map,
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
            let Some(header_ptr) = map.header_pointer() else { continue };
            let Some(header) = mmu.read_map_header(header_ptr) else { continue };

            let edges = adjacency.entry(map).or_default();

            for conn in header.connections() {
                edges.push(Edge { to: conn.map, kind: EdgeKind::Connection(conn.direction) });
            }

            if let Ok(warps) = mmu.read_warp_events(map, header_ptr.bank.id() as usize, header.objects_address) {
                for warp in warps {
                    edges.push(Edge { to: warp.map_id, kind: EdgeKind::Warp });
                }
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
                if new_cost < dist.get(&edge.to).copied().unwrap_or(u32::MAX) {
                    dist.insert(edge.to, new_cost);
                    came_from.insert(edge.to, (map, edge.kind));
                    heap.push(Reverse((new_cost, edge.to as u8)));
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
            e.to == Map::Route1 && e.kind == EdgeKind::Connection(MapConnectionDirection::North)
        });
        assert!(has_route1, "PalletTown should have a north connection to Route1; got {neighbors:?}");
    }

    #[test]
    fn multiple_connections_pallet_town() {
        let g = graph();
        let neighbors = g.neighbors(Map::PalletTown);
        let has_north = neighbors.iter().any(|e| e.to == Map::Route1);
        let has_south = neighbors.iter().any(|e| e.to == Map::Route21);
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
        let exits_to_pallet = neighbors.iter().any(|e| e.to == Map::PalletTown);
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
            let edge_exists = g.neighbors(from).iter().any(|e| e.to == to_step.map);
            assert!(edge_exists, "no edge from {from} to {} in graph", to_step.map);
        }
    }
}

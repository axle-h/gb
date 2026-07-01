use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::{Display, Formatter};
use crate::geometry::Point8;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::map::Map;
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
#[derive(Debug, Clone, Default)]
pub struct WorldGraph {
    adjacency: HashMap<(Map, Point8), Vec<Edge>>,
}

impl WorldGraph {
    /// A new, empty world graph.
    ///
    /// The graph is built **incrementally** as the agent physically traverses the world:
    /// each time the player lands in a map section, [`observe`] records that section's
    /// *live, sprite-resolved* reachable warps/connections. This avoids the phantom edges of
    /// an exhaustively pre-resolved graph (which, lacking runtime sprite state, routes through
    /// NPC-blocked cave paths). The trade-off is that deterministic navigation cannot rely on
    /// routing to a not-yet-visited map — forward map transitions must be specified explicitly.
    pub fn new() -> Self {
        Self { adjacency: HashMap::new() }
    }

    /// Derive the warp/connection edges reachable by BFS from the player position in `tile_map`.
    ///
    /// This is the single source of truth for edge construction, used by [`observe`], which
    /// records a node from the agent's *live* map — whose sprite overlay reflects the truly
    /// visible NPCs and so excludes phantom connections through NPC-blocked paths.
    fn edges_from_reachable(tile_map: &MetaTileMap, map: Map) -> Vec<Edge> {
        tile_map
            .all_reachable_warps_and_connections()
            .into_iter()
            .filter_map(|(src_pos, tile)| {
                // Both warp and connection `to_position` are already in raw
                // (wXCoord/wYCoord) space — no conversion needed.
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


    /// Refine the graph node `(map, entry)` from the agent's *live* map view.
    ///
    /// The initial graph is built without sprites, so it can contain phantom warp/connection
    /// edges through paths that are actually blocked by NPCs at runtime (e.g. the Rocket
    /// trainers on Mt Moon B2F). When the agent is physically standing in a map section, its
    /// `MetaTileMap` includes the real visible-sprite overlay, so the reachable warps/connections
    /// it observes are the ground truth. Overwriting the node with those edges lets the systematic
    /// maze solver score visited sections accurately (and recognise dead-ends). Unvisited nodes
    /// keep their optimistic edges, so connectivity toward the goal is never lost.
    ///
    /// `entry` must be the raw landing position the section is keyed under (equal to the agent's
    /// coordinates on maps without connection strips, e.g. all cave maps).
    pub fn observe(&mut self, map: Map, entry: Point8, tile_map: &MetaTileMap) {
        let edges = Self::edges_from_reachable(tile_map, map);
        self.adjacency.insert((map, entry), edges);
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
    #[allow(dead_code)]
    pub fn map_count(&self) -> usize {
        self.adjacency.keys().map(|(m, _)| *m).collect::<HashSet<_>>().len()
    }

    /// Total directed-edge count.
    #[allow(dead_code)]
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
        self.bfs_nodes(starts, to)
            .map(|nodes| nodes.into_iter().map(|((m, _), via)| MapStep { map: m, via }).collect())
    }

    /// Like [`bfs_to_map`] but keeps the full `(map, raw_entry)` node per step (with the
    /// `EdgeKind` used to enter it). The entry positions are exactly the raw `to_position`s
    /// needed to encode explicit [`PolicyStep::EnterMap`] transitions.
    fn bfs_nodes(&self, starts: &[(Map, Point8)], to: Map) -> Option<Vec<((Map, Point8), Option<EdgeKind>)>> {
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

        Some(path_rev)
    }

    /// The `(map, raw_entry)` node sequence of the shortest path from the **specific** section
    /// `(from, from_entry)` to `to`, or `None` if unreachable in the currently-observed graph.
    /// Used to derive explicit `EnterMap` steps: each node after the first is
    /// `EnterMap { to_map: node.0, to_position: Some(node.1) }`.
    pub fn shortest_node_path_from(&self, from: Map, from_entry: Point8, to: Map) -> Option<Vec<(Map, Point8)>> {
        if from == to {
            return None;
        }
        self.bfs_nodes(&[(from, from_entry)], to).map(|nodes| nodes.into_iter().map(|(n, _)| n).collect())
    }

    /// True if the section landed at `(map, entry)` has already been observed.
    pub fn has_node(&self, map: Map, entry: Point8) -> bool {
        self.adjacency.contains_key(&(map, entry))
    }

    /// All observed `(map, entry)` section nodes with their outgoing edges.
    pub fn nodes(&self) -> Vec<((Map, Point8), Vec<Edge>)> {
        self.adjacency.iter().map(|(k, v)| (*k, v.clone())).collect()
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
    pub fn shortest_path_from_entry(&self, from: Map, from_entry: Point8, to: Map) -> Option<usize> {
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
impl WorldGraph {
    /// Test-only: record a section `(map, entry)` with a fixed set of outgoing edges,
    /// mimicking what `observe` derives from a live map. `edges` is
    /// `(from_tile, to_map, to_landing, kind)`.
    fn observe_edges(&mut self, map: Map, entry: Point8, edges: &[(Point8, Map, Point8, EdgeKind)]) {
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

    /// A small synthetic world observed incrementally, mirroring how the agent would build it
    /// as it walks:  PalletTown ⇄ Route1 ⇄ ViridianCity, PalletTown ⇄ OaksLab (warp),
    /// RedsHouse1F ⇄ RedsHouse2F (warp-only), and PalletTown → Route21 (dead-end).
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
        // trivial same-map path is always available
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
        assert_eq!(path, vec![MapStep { map: Map::PalletTown, via: None }]);
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
        // PalletTown ⇄ Route1 ⇄ ViridianCity forms cycles; pathfinding must terminate.
        let g = small_world();
        let _ = g.shortest_path(Map::ViridianCity, Map::OaksLab);
    }

    #[test]
    fn no_path_to_unobserved_map() {
        let g = small_world();
        // We never observed anything reaching CeruleanCity — so it is unreachable, the
        // "hard fail" signal that a deterministic policy is under-specified.
        assert!(g.shortest_path(Map::PalletTown, Map::CeruleanCity).is_none());
        // Route21 is observed but a dead-end; still no path onward.
        assert!(g.shortest_path(Map::Route21, Map::Route1).is_none());
    }

    #[test]
    fn disconnected_sections_keyed_separately() {
        // The same map reached at two different landing positions must not conflate edges:
        // section A only reaches X, section B only reaches Y.
        use EdgeKind::Warp;
        let mut g = WorldGraph::new();
        g.observe_edges(Map::Route2, p(3, 5), &[(p(3, 5), Map::ViridianForest, p(5, 0), Warp)]);
        g.observe_edges(Map::Route2, p(3, 60), &[(p(3, 60), Map::PewterCity, p(14, 35), Warp)]);
        // From the south section we can reach ViridianForest but not PewterCity...
        assert!(g.shortest_path_from_entry(Map::Route2, p(3, 5), Map::ViridianForest).is_some());
        assert!(g.shortest_path_from_entry(Map::Route2, p(3, 5), Map::PewterCity).is_none());
        // ...and vice versa from the north section.
        assert!(g.shortest_path_from_entry(Map::Route2, p(3, 60), Map::PewterCity).is_some());
    }

    #[test]
    fn pick_shortest_path_action_routes_toward_target() {
        use crate::pokemon::tile::MetaTile;
        let g = small_world();
        // On PalletTown, two candidate warps/connections: toward Route1 (leads to Viridian)
        // and toward OaksLab (dead-endish). Target ViridianCity → should pick the Route1 one.
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

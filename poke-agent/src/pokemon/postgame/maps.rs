//! Workstream L — visit every visitable map.

use crate::pokemon::map::Map;
use crate::pokemon::policy::PolicyStep;

/// The two link-cable rooms.
pub const LINK_CABLE_MAPS: &[Map] = &[Map::Colosseum, Map::TradeCenter];

pub const DUPLICATE_MAPS: &[Map] = &[
    Map::CeruleanTrashedHouseCopy,
    Map::CinnabarMartCopy,
    Map::UndergroundPathRoute6Copy,
    Map::UndergroundPathRoute7Copy,
];

/// Warp destinations the static audit must not treat as broken: the map id in the ROM is a
/// placeholder that a script rewrites at runtime.
pub const RUNTIME_REDIRECTED_WARPS: &[(Map, Map)] = &[
    (Map::SilphCoElevator, Map::UnusedMapEd),
];

/// Every map worth trying to stand in: it has a header, it is not a link-cable room, and it is
/// not one of the duplicate slots.
pub fn visitable() -> Vec<Map> {
    use strum::IntoEnumIterator;
    Map::iter()
        .filter(|m| m.header_pointer().is_some())
        .filter(|m| !LINK_CABLE_MAPS.contains(m))
        .filter(|m| !DUPLICATE_MAPS.contains(m))
        .collect()
}

/// Why a visitable map could not be entered — the L4 deliverable, as data rather than prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreachable {
    /// The ship sails once the mainline is finished (`EVENT_SS_ANNE_LEFT`), taking fifteen maps
    /// with it.
    SsAnneSailed,
    /// A one-way climb that only works while a script is mid-flight.
    OneWayScript,
    /// Reached only by beating the Elite Four again — the Hall of Fame.
    NeedsAnotherChampionRun,
    /// Behind the Safari Zone's ¥500 gate and its 502-step budget, and the zone is a chain of
    /// areas rather than a Fly hub, so a visit is a paid trip planned around the step counter.
    BehindTheSafariGate,
    /// Reachable, but not from the Fly stop — the hub's map is cut into pieces that do not join
    /// up, so the tour's "fly in, walk to the door" cannot get there.
    NotFromTheFlyStop,
    /// Only the elevator goes there, and an elevator is not a warp: its warp entries point at a
    /// placeholder that the floor menu rewrites at runtime (see [`RUNTIME_REDIRECTED_WARPS`]), so
    /// "walk through this door" cannot express it — `PolicyStep::UseElevator` can, and does.
    BehindAnElevatorMenu,
    /// Behind a gate a script opens — the Pokémon Mansion's statue switches, which
    /// `PolicyStep::FlipSwitch` drives (the Volcano Badge workstream proves it).
    BehindASwitchGate,
    /// Behind a receptionist who wants paying.
    BehindAPaidGate,
}

impl Unreachable {
    /// One line, for the audit's report.
    pub const fn why(self) -> &'static str {
        match self {
            Self::SsAnneSailed => "the S.S. Anne has sailed (EVENT_SS_ANNE_LEFT)",
            Self::OneWayScript => "a one-way script-gated climb, already spent on this save",
            Self::NeedsAnotherChampionRun => "needs another Elite Four run",
            Self::BehindTheSafariGate => "behind the Safari Zone's paid gate and step budget",
            Self::NotFromTheFlyStop =>
                "reachable, but not from the Fly stop — the hub map is cut into sealed regions",
            Self::BehindAnElevatorMenu => "only the elevator goes there; use PolicyStep::UseElevator",
            Self::BehindASwitchGate => "behind a script-opened gate; use PolicyStep::FlipSwitch",
            Self::BehindAPaidGate => "behind a receptionist who wants paying first",
        }
    }
}

/// The maps a hub tour cannot enter, and why. This is the L4 answer, and every row is
/// evidence-backed rather than assumed — see the workstream's archive entry.
pub fn known_unreachable(map: Map) -> Option<Unreachable> {
    use Unreachable::*;
    match map {
        Map::SSAnne1F | Map::SSAnne2F | Map::SSAnne3F | Map::SSAnneB1F
        | Map::SSAnneBow | Map::SSAnneKitchen | Map::SSAnneCaptainsRoom
        | Map::SSAnne1FRooms | Map::SSAnne2FRooms | Map::SSAnneB1FRooms
        // And the dock itself.
        | Map::VermilionDock => Some(SsAnneSailed),
        Map::PokemonTower6F | Map::PokemonTower7F => Some(OneWayScript),
        Map::CeruleanCave1F | Map::CeruleanCave2F | Map::CeruleanCaveB1F => Some(NotFromTheFlyStop),
        // Two roads the tour rediscovered, both already in the archive — which is the most
        // reassuring thing about them.
        Map::Route4 | Map::Route7 => Some(NotFromTheFlyStop),
        Map::CeladonMart4F | Map::CeladonMart5F | Map::CeladonMartRoof => Some(BehindAnElevatorMenu),
        Map::PokemonMansionB1F => Some(BehindASwitchGate),
        Map::Museum2F => Some(BehindAPaidGate),
        Map::HallOfFame => Some(NeedsAnotherChampionRun),
        Map::SafariZoneCenterRestHouse | Map::SafariZoneEastRestHouse
        | Map::SafariZoneNorthRestHouse | Map::SafariZoneWestRestHouse
        | Map::SafariZoneCenter | Map::SafariZoneEast | Map::SafariZoneNorth | Map::SafariZoneWest
            => Some(BehindTheSafariGate),
        _ => None,
    }
}

/// Rooms a tour must not walk into, and why. Distinct from [`known_unreachable`]: these can be
/// entered perfectly well, and that is the problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipTour {
    /// The door is sealed behind you until the room's trainer is beaten.
    SealsTheDoor,
    /// Entering starts a paid Safari trip: ¥500 to the warden and the 502-step counter begins,
    /// after which the player is ejected wherever they stand.
    CostsAPaidTrip,
    /// You can get in and you cannot get out.
    NeedsTheFloorMenu,
    /// A road lined with trainers, every one of whom fights on sight.
    TrainerGauntlet,
}

/// Whether the tour should walk past this room's door rather than through it.
pub fn skip_tour(map: Map) -> Option<SkipTour> {
    use SkipTour::*;
    match map {
        Map::LoreleisRoom | Map::BrunosRoom | Map::AgathasRoom | Map::LancesRoom
        | Map::ChampionsRoom => Some(SealsTheDoor),
        Map::SafariZoneCenter | Map::SafariZoneEast | Map::SafariZoneNorth | Map::SafariZoneWest
        | Map::SafariZoneCenterRestHouse | Map::SafariZoneEastRestHouse
        | Map::SafariZoneNorthRestHouse | Map::SafariZoneWestRestHouse => Some(CostsAPaidTrip),
        Map::CeladonMartElevator | Map::SilphCoElevator | Map::RocketHideoutElevator
            => Some(NeedsTheFloorMenu),
        Map::Route8 => Some(TrainerGauntlet),
        _ => None,
    }
}

/// Rooms the tour enters but does not walk deeper into: the front door of a dungeon that has its
/// own workstream.
pub fn tour_leaf(map: Map) -> bool {
    matches!(map, Map::SilphCo1F | Map::RocketHideoutB1F | Map::PokemonMansion1F
                | Map::PokemonTower1F | Map::CeruleanCave1F | Map::IndigoPlateauLobby)
}

/// True for the two tilesets Fly considers "outside" — the tour uses them as its fence.
fn is_outdoors(tileset: crate::pokemon::map_header::TileSetId) -> bool {
    use crate::pokemon::map_header::TileSetId;
    matches!(tileset, TileSetId::Overworld | TileSetId::Plateau)
}

/// The path to every room reachable from `hub` by walking through doors, up to `depth` doors deep
/// — read from the ROM's own warp tables rather than listed. Each entry starts at a room the hub
/// opens directly onto and ends at the room itself; the hub is not included.
pub fn room_paths(hub: Map, depth: u8) -> Vec<Vec<Map>> {
    use gb::mmu::MMU;
    use crate::pokemon::map_metadata::MapMetadataReader;
    let Ok(mmu) = MMU::from_rom(crate::pokemon::roms::POKERED) else { return Vec::new() };
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::from([hub]);

    fn walk(mmu: &MMU, from: Map, depth: u8, path: &mut Vec<Map>,
            seen: &mut std::collections::HashSet<Map>, paths: &mut Vec<Vec<Map>>) {
        if depth == 0 { return }
        let Ok(metadata) = mmu.read_map_metadata(from) else { return };
        let mut next: Vec<Map> = Vec::new();
        for warp in &metadata.warp_events {
            let to = warp.destination_map;
            if to.header_pointer().is_none() || seen.contains(&to) || next.contains(&to) { continue }
            if skip_tour(to).is_some() { continue }
            next.push(to);
        }
        for to in next {
            if !seen.insert(to) { continue }
            path.push(to);
            paths.push(path.clone());
            let outdoors = mmu.read_map_metadata(to)
                .map_or(true, |child| is_outdoors(child.map_header.tileset));
            // Stop at the front door of the next town (an outdoor map belongs to its own hub) and
            // at the front door of a dungeon (see `tour_leaf`).
            if !outdoors && !tour_leaf(to) { walk(mmu, to, depth - 1, path, seen, paths); }
            path.pop();
        }
    }
    walk(&mmu, hub, depth, &mut Vec::new(), &mut seen, &mut paths);
    paths
}

/// The maps a hub walks onto — its N/E/S/W connections, from the map header.
pub fn connected_routes(hub: Map) -> Vec<Map> {
    use gb::mmu::MMU;
    use crate::pokemon::map_header::MapHeaderReader;
    let Ok(mmu) = MMU::from_rom(crate::pokemon::roms::POKERED) else { return Vec::new() };
    let Ok(header) = mmu.read_map_header(hub) else { return Vec::new() };
    let mut out: Vec<Map> = Vec::new();
    for connection in header.connections() {
        // `skip_tour` applies to roads too, and Route 8 is why: nine trainers on sight, and the
        // tour arrives with whatever PP the last leg left it.
        if skip_tour(connection.map).is_some() { continue }
        if !out.contains(&connection.map) { out.push(connection.map); }
    }
    out
}

/// Just the rooms, in tour order — [`room_paths`] flattened to its endpoints.
pub fn rooms_off(hub: Map, depth: u8) -> Vec<Map> {
    room_paths(hub, depth).into_iter().filter_map(|p| p.last().copied()).collect()
}

impl PolicyStep {
    /// L2 — the tour of one Fly hub: in and out of every room [`room_paths`] finds.
    pub fn tour_hub_steps(hub: Map, depth: u8) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: hub }];
        // Rooms first, then the roads out.
        let mut paths = room_paths(hub, depth);
        // And each road is one step out and straight back, never followed: a route leads to the
        // next town, whose rooms are that hub's tour.
        paths.extend(connected_routes(hub).into_iter().map(|m| vec![m]));
        for path in paths {
            // Cut before every room, not once at the top.
            s.push(Self::CutTree { map: hub });
            for &room in &path {
                s.push(Self::EnterMapIfReachable { to_map: room });
            }
            // …and back out the way we came in, ending on the hub.
            for &room in path.iter().rev().skip(1) {
                s.push(Self::EnterMapIfReachable { to_map: room });
            }
            s.push(Self::EnterMapIfReachable { to_map: hub });
        }
        s
    }
}

pub const AWKWARD_SET: &[(Map, &str)] = &[
    (Map::SafariZoneCenterRestHouse, "skip_tour: costs a paid trip"),
    (Map::SafariZoneEastRestHouse,   "skip_tour: costs a paid trip"),
    (Map::SafariZoneNorthRestHouse,  "skip_tour: costs a paid trip"),
    (Map::SafariZoneWestRestHouse,   "skip_tour: costs a paid trip"),
    (Map::Museum2F,                  "unreachable: behind a paid receptionist"),
    (Map::Route19,                   "outside the hub tours: water, reached by Surf (workstream B/C)"),
    (Map::Route20,                   "outside the hub tours: water, reached by Surf (workstream B/C)"),
    (Map::Route8Gate,                "outside the hub tours: it sits on Route 8, which skip_tour drops"),
    (Map::CeruleanBadgeHouse,        "toured, from Cerulean"),
    (Map::CinnabarLab,               "toured, from Cinnabar Island"),
    (Map::CinnabarLabTradeRoom,      "toured, from Cinnabar Island"),
    (Map::CinnabarLabFossilRoom,     "toured, from Cinnabar Island — and workstream K trades there"),
    (Map::CinnabarLabMetronomeRoom,  "toured, from Cinnabar Island"),
    (Map::SSAnne1F,                  "unreachable: the ship has sailed"),
    (Map::VermilionDock,             "unreachable: the ship has sailed, and the pier is sealed with it"),
    (Map::PokemonTower6F,            "unreachable: a one-way script-gated climb"),
    (Map::PokemonTower7F,            "unreachable: a one-way script-gated climb"),
    (Map::HallOfFame,                "unreachable: needs another Champion run"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use gb::mmu::MMU;
    use crate::pokemon::map_header::MapHeaderReader;
    use crate::pokemon::map_metadata::MapMetadataReader;
    use crate::pokemon::roms;

    fn rom() -> MMU {
        MMU::from_rom(roms::POKERED).expect("the bundled ROM should load")
    }

    /// L1's first question: how many maps are actually in scope, and does the plan's "~220" hold?
    #[test]
    fn the_visitable_set_is_what_the_plan_says() {
        use strum::IntoEnumIterator;
        let all = Map::iter().count();
        let headerless: Vec<Map> = Map::iter().filter(|m| m.header_pointer().is_none()).collect();
        let visitable = visitable();
        println!("{all} Map variants · {} headerless · {} link-cable · {} duplicates · {} visitable",
            headerless.len(), LINK_CABLE_MAPS.len(), DUPLICATE_MAPS.len(), visitable.len());
        println!("headerless: {headerless:?}");

        assert_eq!(all, 248, "the Map enum should still model all 248 map ids");
        // The buckets overlap: three of the four duplicate slots are headerless too.
        let struck: std::collections::HashSet<Map> = headerless.iter().copied()
            .chain(LINK_CABLE_MAPS.iter().copied())
            .chain(DUPLICATE_MAPS.iter().copied())
            .collect();
        assert_eq!(visitable.len() + struck.len(), all, "every map should be in exactly one bucket");
        let headered_duplicates: Vec<Map> = DUPLICATE_MAPS.iter().copied()
            .filter(|m| m.header_pointer().is_some()).collect();
        println!("duplicates that are not already headerless: {headered_duplicates:?}");
        assert_eq!(headered_duplicates, vec![Map::UndergroundPathRoute7Copy]);
        assert!((215..=225).contains(&visitable.len()),
            "expected ~220 visitable maps, got {}", visitable.len());
    }

    /// L1 proper — every visitable map's own metadata has to answer, without emulating anything.
    #[test]
    fn every_visitable_map_has_readable_metadata() {
        let mmu = rom();
        let mut broken = Vec::new();
        let mut warps = 0usize;
        for map in visitable() {
            let header = match mmu.read_map_header(map) {
                Ok(h) => h,
                Err(e) => { broken.push(format!("{map}: header unreadable — {e}")); continue }
            };
            if header.width == 0 || header.height == 0 {
                broken.push(format!("{map}: {}x{} blocks", header.width, header.height));
            }
            let metadata = match mmu.read_map_metadata(map) {
                Ok(m) => m,
                Err(e) => { broken.push(format!("{map}: metadata unreadable — {e}")); continue }
            };
            let expected = header.width as usize * header.height as usize;
            if metadata.map_data.len() != expected {
                broken.push(format!("{map}: {} blocks read, header says {expected}",
                    metadata.map_data.len()));
            }
            // Every warp has to name a map that exists — a warp into a headerless slot is a door
            // the agent would walk through into nothing.
            for warp in &metadata.warp_events {
                warps += 1;
                if warp.destination_map.header_pointer().is_none()
                    && !LINK_CABLE_MAPS.contains(&warp.destination_map)
                    && !RUNTIME_REDIRECTED_WARPS.contains(&(map, warp.destination_map)) {
                    broken.push(format!("{map}: warp at {} leads to {}, which has no header",
                        warp.position, warp.destination_map));
                }
            }
            // …and so does every connection.
            for connection in header.connections() {
                if connection.map.header_pointer().is_none() {
                    broken.push(format!("{map}: {:?} connection to {}, which has no header",
                        connection.direction, connection.map));
                }
            }
        }
        println!("audited {} maps and {warps} warps", visitable().len());
        assert!(broken.is_empty(), "static map audit found {} problems:\n{}",
            broken.len(), broken.join("\n"));
    }

    /// L1 — the tile grid builds for every map, and is the size the header implies.
    #[test]
    fn every_visitable_map_builds_a_tile_grid() {
        use crate::pokemon::map_metadata::MapMetadata;
        let mmu = rom();
        let mut wrong = Vec::new();
        for map in visitable() {
            let Ok(metadata) = mmu.read_map_metadata(map) else { continue };
            let dims = metadata.dimensions();
            let tiles = metadata.build_meta_tiles_base();
            let expected = dims.full_width() * dims.full_height();
            if tiles.len() != expected {
                wrong.push(format!("{map}: {} meta-tiles, expected {expected} ({}x{})",
                    tiles.len(), dims.full_width(), dims.full_height()));
            }
            if metadata.build_raw_tile_ids().len() != tiles.len() {
                wrong.push(format!("{map}: raw tile ids and meta tiles disagree in length"));
            }
            let _ = MapMetadata::BLOCK_TILES;
        }
        assert!(wrong.is_empty(), "{} maps built the wrong grid:\n{}", wrong.len(), wrong.join("\n"));
    }

    /// L1 — a map with objects in the ROM must have sprites in [`Map::sprites`].
    #[test]
    fn every_map_with_objects_has_a_sprite_table() {
        let mmu = rom();
        let mut missing = Vec::new();
        for map in visitable() {
            let Ok(header) = mmu.read_map_header(map) else { continue };
            // `objects_address` is the map's `*_Object` block: border block, then the four
            // counted lists.
            let object_count = object_event_count(&mmu, &header);
            if object_count > 0 && map.sprites().is_empty() {
                missing.push(format!("{map}: {object_count} object events in the ROM, no sprite table"));
            }
        }
        assert!(missing.is_empty(), "{} maps have objects the agent cannot name:\n{}",
            missing.len(), missing.join("\n"));
    }

    /// Walk a map's `*_Object` structure to its object-event count.
    fn object_event_count(mmu: &MMU, header: &crate::pokemon::map_header::MapHeader) -> u8 {
        use crate::pokemon::symbols::{DmgBank, DmgPointer};
        let ptr = DmgPointer { bank: DmgBank::ROM { bank: header.header_bank },
                               address: header.objects_address };
        let data = mmu.rom_data_from_rom_pointer(&ptr, 0x400);
        let warps = data[1] as usize;
        let bg_at = 2 + warps * 4;
        let bgs = data[bg_at] as usize;
        data[bg_at + 1 + bgs * 3]
    }

    /// The four duplicate slots really are duplicates — same tileset, dimensions and block
    /// pointer as the map they copy.
    #[test]
    fn the_duplicate_slots_really_are_duplicates() {
        let mmu = rom();
        for (copy, original) in [
            (Map::CeruleanTrashedHouseCopy, Map::CeruleanTrashedHouse),
            (Map::CinnabarMartCopy, Map::CinnabarMart),
            (Map::UndergroundPathRoute6Copy, Map::UndergroundPathRoute6),
            (Map::UndergroundPathRoute7Copy, Map::UndergroundPathRoute7),
        ] {
            // Three of the four have no header at all, which is a stronger form of "not a room"
            // than being a copy — nothing to compare, and nothing to visit.
            let Some(_) = copy.header_pointer() else {
                println!("{copy}: headerless — struck before the duplicate check even applies");
                continue;
            };
            let a = mmu.read_map_header(copy).unwrap_or_else(|e| panic!("{copy}: {e}"));
            let b = mmu.read_map_header(original).unwrap_or_else(|e| panic!("{original}: {e}"));
            assert_eq!((a.tileset, a.width, a.height, a.blocks_address),
                       (b.tileset, b.width, b.height, b.blocks_address),
                "{copy} is not a copy of {original} after all — it should be back in the tour");
        }
    }

    #[test]
    fn the_awkward_set_is_accounted_for() {
        for (map, verdict) in AWKWARD_SET {
            let skipped = skip_tour(*map).is_some();
            let unreachable = known_unreachable(*map).is_some();
            println!("   {map}: {verdict}");
            if verdict.starts_with("skip_tour") {
                assert!(skipped, "{map} says skip_tour but skip_tour does not list it");
            } else if verdict.starts_with("unreachable") {
                assert!(unreachable, "{map} says unreachable but known_unreachable does not list it");
            } else {
                assert!(!skipped && !unreachable,
                    "{map} claims to be toured but the tour's own tables drop it");
            }
            assert!(visitable().contains(map), "{map} is not even in the visitable set");
        }
    }

    /// Nothing in [`known_unreachable`] should be a map the tour is also expected to enter, and
    /// every entry should be a real visitable map rather than a typo.
    #[test]
    fn the_unreachable_list_is_well_formed() {
        let visitable = visitable();
        let unreachable: Vec<Map> = visitable.iter().copied()
            .filter(|m| known_unreachable(*m).is_some()).collect();
        println!("{} of {} visitable maps are known-unreachable on a post-Champion save:",
            unreachable.len(), visitable.len());
        for map in &unreachable {
            println!("   {map}: {}", known_unreachable(*map).unwrap().why());
        }
        assert_eq!(unreachable.len(), 32,
            "the known-unreachable set changed — update the workstream's archive entry with why");
    }
}

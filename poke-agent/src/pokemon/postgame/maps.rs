//! Visiting every map: which count, which cannot be entered and why, and the hub tours.

use crate::pokemon::map::Map;
use crate::pokemon::policy::PolicyStep;

pub const LINK_CABLE_MAPS: &[Map] = &[Map::Colosseum, Map::TradeCenter];

pub const DUPLICATE_MAPS: &[Map] = &[
    Map::CeruleanTrashedHouseCopy,
    Map::CinnabarMartCopy,
    Map::UndergroundPathRoute6Copy,
    Map::UndergroundPathRoute7Copy,
];

/// Warps whose destination in the ROM is a placeholder a script rewrites at runtime.
pub const RUNTIME_REDIRECTED_WARPS: &[(Map, Map)] = &[
    (Map::SilphCoElevator, Map::UnusedMapEd),
];

/// Every map with a header that is neither a link-cable room nor a duplicate slot.
pub fn visitable() -> Vec<Map> {
    use strum::IntoEnumIterator;
    Map::iter()
        .filter(|m| m.header_pointer().is_some())
        .filter(|m| !LINK_CABLE_MAPS.contains(m))
        .filter(|m| !DUPLICATE_MAPS.contains(m))
        .collect()
}

/// Why a visitable map cannot be entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreachable {
    SsAnneSailed,
    /// A one-way climb that only works while a script is mid-flight.
    OneWayScript,
    NeedsAnotherChampionRun,
    /// A paid trip around a step counter through a chain of areas, not a Fly hub.
    BehindTheSafariGate,
    /// The hub's map is cut into regions that do not join, so "fly in, walk to the door" fails.
    NotFromTheFlyStop,
    /// An elevator's warps point at a placeholder the floor menu rewrites, so only
    /// `PolicyStep::UseElevator` gets there.
    BehindAnElevatorMenu,
    /// Behind the Pokémon Mansion's statue switches, which `PolicyStep::FlipSwitch` drives.
    BehindASwitchGate,
    BehindAPaidGate,
}

impl Unreachable {
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

/// The maps a hub tour cannot enter, and why.
pub fn known_unreachable(map: Map) -> Option<Unreachable> {
    use Unreachable::*;
    match map {
        Map::SSAnne1F | Map::SSAnne2F | Map::SSAnne3F | Map::SSAnneB1F
        | Map::SSAnneBow | Map::SSAnneKitchen | Map::SSAnneCaptainsRoom
        | Map::SSAnne1FRooms | Map::SSAnne2FRooms | Map::SSAnneB1FRooms
        | Map::VermilionDock => Some(SsAnneSailed),
        Map::PokemonTower6F | Map::PokemonTower7F => Some(OneWayScript),
        Map::CeruleanCave1F | Map::CeruleanCave2F | Map::CeruleanCaveB1F => Some(NotFromTheFlyStop),
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

/// Rooms a tour can enter but must not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipTour {
    SealsTheDoor,
    CostsAPaidTrip,
    /// An elevator: you can get in, and only the floor menu gets you out.
    NeedsTheFloorMenu,
    TrainerGauntlet,
}

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

/// The front door of a dungeon with legs of its own, which the tour enters and goes no deeper.
pub fn tour_leaf(map: Map) -> bool {
    matches!(map, Map::SilphCo1F | Map::RocketHideoutB1F | Map::PokemonMansion1F
                | Map::PokemonTower1F | Map::CeruleanCave1F | Map::IndigoPlateauLobby)
}

/// The two tilesets Fly considers outside, which fence the tour.
fn is_outdoors(tileset: crate::pokemon::map_header::TileSetId) -> bool {
    use crate::pokemon::map_header::TileSetId;
    matches!(tileset, TileSetId::Overworld | TileSetId::Plateau)
}

/// The path to every room within `depth` doors of `hub`, from the ROM's warp tables. Each starts
/// at a room the hub opens onto and ends at the room; the hub is not included.
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
            // An outdoor map belongs to its own hub.
            if !outdoors && !tour_leaf(to) { walk(mmu, to, depth - 1, path, seen, paths); }
            path.pop();
        }
    }
    walk(&mmu, hub, depth, &mut Vec::new(), &mut seen, &mut paths);
    paths
}

/// The maps a hub's connections lead to.
pub fn connected_routes(hub: Map) -> Vec<Map> {
    use gb::mmu::MMU;
    use crate::pokemon::map_header::MapHeaderReader;
    let Ok(mmu) = MMU::from_rom(crate::pokemon::roms::POKERED) else { return Vec::new() };
    let Ok(header) = mmu.read_map_header(hub) else { return Vec::new() };
    let mut out: Vec<Map> = Vec::new();
    for connection in header.connections() {
        if skip_tour(connection.map).is_some() { continue }
        if !out.contains(&connection.map) { out.push(connection.map); }
    }
    out
}

/// [`room_paths`] flattened to its endpoints.
pub fn rooms_off(hub: Map, depth: u8) -> Vec<Map> {
    room_paths(hub, depth).into_iter().filter_map(|p| p.last().copied()).collect()
}

impl PolicyStep {
    /// In and out of every room [`room_paths`] finds, then one step down each road and back.
    pub fn tour_hub_steps(hub: Map, depth: u8) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: hub }];
        let mut paths = room_paths(hub, depth);
        paths.extend(connected_routes(hub).into_iter().map(|m| vec![m]));
        for path in paths {
            s.push(Self::CutTree { map: hub });
            for &room in &path {
                s.push(Self::EnterMapIfReachable { to_map: room });
            }
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

    /// Every map is in exactly one bucket, and about 220 are visitable.
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

    /// Every visitable map's metadata reads from the ROM, and every warp and connection has a header.
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
            for warp in &metadata.warp_events {
                warps += 1;
                if warp.destination_map.header_pointer().is_none()
                    && !LINK_CABLE_MAPS.contains(&warp.destination_map)
                    && !RUNTIME_REDIRECTED_WARPS.contains(&(map, warp.destination_map)) {
                    broken.push(format!("{map}: warp at {} leads to {}, which has no header",
                        warp.position, warp.destination_map));
                }
            }
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

    /// The tile grid builds for every map, and is the size the header implies.
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

    /// A map with objects in the ROM must have sprites in [`Map::sprites`].
    #[test]
    fn every_map_with_objects_has_a_sprite_table() {
        let mmu = rom();
        let mut missing = Vec::new();
        for map in visitable() {
            let Ok(header) = mmu.read_map_header(map) else { continue };
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

    /// A headered duplicate slot shares its original's tileset, dimensions and blocks.
    #[test]
    fn the_duplicate_slots_really_are_duplicates() {
        let mmu = rom();
        for (copy, original) in [
            (Map::CeruleanTrashedHouseCopy, Map::CeruleanTrashedHouse),
            (Map::CinnabarMartCopy, Map::CinnabarMart),
            (Map::UndergroundPathRoute6Copy, Map::UndergroundPathRoute6),
            (Map::UndergroundPathRoute7Copy, Map::UndergroundPathRoute7),
        ] {
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

    /// [`known_unreachable`] lists 32 visitable maps.
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

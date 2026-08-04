//! Workstream **L — visit every visitable map**. See `docs/postgame-coverage-plan.md` §8-L.
//!
//! Alex, 2026-08-04: *"visit ALL visitable rooms to check there are no broken map mechanics we
//! haven't covered in the agent."* The deliverable is as much the **list of maps that cannot be
//! entered, and why**, as the green test — so this module's job is to say precisely which of the 248
//! `Map` variants are in scope, and to give the audit something to assert against.
//!
//! # What "visitable" means
//!
//! §8-L strikes four groups and this module encodes exactly those, with the ROM as the arbiter
//! wherever there is one:
//!
//! | Struck | How it is decided |
//! |---|---|
//! | the unused map slots | `Map::header_pointer()` returns `None` — pokered has no `*_h` label for them |
//! | `Colosseum`, `TradeCenter` | link cable (§3); they have headers, so they are named here |
//! | the four duplicate slots | named here: `CeruleanTrashedHouseCopy`, `CinnabarMartCopy`, `UndergroundPathRoute6Copy`, `UndergroundPathRoute7Copy` |
//!
//! ⚠️ The plan's arithmetic is close but not exact — see [`tests::the_visitable_set_is_what_the_plan_says`],
//! which prints the real count rather than trusting "~220".

use crate::pokemon::map::Map;
use crate::pokemon::policy::PolicyStep;

/// The two link-cable rooms. Reachable in-game (the Cable Club receptionist walks you in), but
/// nothing can happen in them without a second cartridge, so §3 rules them out.
pub const LINK_CABLE_MAPS: &[Map] = &[Map::Colosseum, Map::TradeCenter];

/// The four duplicate map slots §8-L strikes.
///
/// ⚠️ **Three of the four are already headerless**, which is why the plan's arithmetic
/// ("22 `UnusedMap*`, `Colosseum` and `TradeCenter`, and the four duplicate slots") double-counts:
/// `CeruleanTrashedHouseCopy`, `CinnabarMartCopy` and `UndergroundPathRoute6Copy` have no `*_h`
/// label in pokered at all, so [`visitable`] has already dropped them before this list is consulted.
/// Only **`UndergroundPathRoute7Copy`** is a headered map that has to be struck by name.
pub const DUPLICATE_MAPS: &[Map] = &[
    Map::CeruleanTrashedHouseCopy,
    Map::CinnabarMartCopy,
    Map::UndergroundPathRoute6Copy,
    Map::UndergroundPathRoute7Copy,
];

/// Warp destinations the static audit must not treat as broken: the map id in the ROM is a
/// placeholder that a script rewrites at runtime.
///
/// One case, and it is a mechanic the agent already models: **`SilphCoElevator`'s two warp tiles
/// both point at `UnusedMapEd`**, because the floor menu writes the real destination into the warp
/// entry when you pick a floor (which is what `PolicyStep::UseElevator` drives). A static reader sees
/// a door into a map that does not exist. Finding this is L1 working — it is exactly the shape of
/// "missing metadata" the audit is for — and the answer is to name it, not to widen the check.
pub const RUNTIME_REDIRECTED_WARPS: &[(Map, Map)] = &[
    (Map::SilphCoElevator, Map::UnusedMapEd),
];

/// Every map worth trying to stand in: it has a header, it is not a link-cable room, and it is not
/// one of the duplicate slots.
pub fn visitable() -> Vec<Map> {
    use strum::IntoEnumIterator;
    Map::iter()
        .filter(|m| m.header_pointer().is_some())
        .filter(|m| !LINK_CABLE_MAPS.contains(m))
        .filter(|m| !DUPLICATE_MAPS.contains(m))
        .collect()
}

/// Why a visitable map could not be entered — the **L4 deliverable**, as data rather than prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreachable {
    /// The ship sails once the mainline is finished (`EVENT_SS_ANNE_LEFT`), taking fifteen maps with
    /// it. Nothing short of a fresh playthrough gets them back.
    SsAnneSailed,
    /// A one-way climb that only works while a script is mid-flight. Pokémon Tower 6F/7F is the
    /// known case: the mainline climb passes through while the Channelers are still being fought and
    /// collects the Rare Candy that unblocks 6F's chokepoint, so a save that has *finished* the tower
    /// cannot get back up (recorded by H5c).
    OneWayScript,
    /// Reached only by beating the Elite Four again — the Hall of Fame.
    NeedsAnotherChampionRun,
    /// Behind the Safari Zone's ¥500 gate and its 502-step budget, and the zone is a chain of areas
    /// rather than a Fly hub, so a visit is a paid trip planned around the step counter.
    BehindTheSafariGate,
    /// Reachable, but **not from the Fly stop** — the hub's map is cut into pieces that do not join
    /// up, so the tour's "fly in, walk to the door" cannot get there.
    ///
    /// Cerulean Cave is the case that found this: `CeruleanCave1F/2F/B1F` hang off Cerulean City in
    /// the warp table, and Cerulean City is one of the maps §10 lists as *several sealed regions*.
    /// The cave sits across the river, and workstream **D** reaches it the only way there is — over
    /// **Route 24's left river seam**, a `ConnectionWater` — after flying to Cerulean and walking
    /// north. So this is a limit of the *tour*, not of the game, and it is worth keeping distinct
    /// from the rows above: those are rooms nothing can reach again.
    NotFromTheFlyStop,
    /// Only the **elevator** goes there, and an elevator is not a warp: its warp entries point at a
    /// placeholder that the floor menu rewrites at runtime (see [`RUNTIME_REDIRECTED_WARPS`]), so
    /// "walk through this door" cannot express it — `PolicyStep::UseElevator` can, and does.
    ///
    /// Celadon Mart's 4F, 5F and roof are the case: the ROM's warp table makes them look one door
    /// from `CeladonMartElevator`, and they are actually three flights of stairs from the street.
    BehindAnElevatorMenu,
    /// Behind a gate a script opens — the Pokémon Mansion's statue switches, which
    /// `PolicyStep::FlipSwitch` drives (the Volcano Badge workstream proves it). The door simply is
    /// not there until the switch is thrown.
    BehindASwitchGate,
    /// Behind a receptionist who wants paying. Pewter's **Museum 2F** is up the stairs past the
    /// museum's ¥50 desk, and whether the tour gets in depends on whether its generic A-mash lands on
    /// YES before the walk is re-planned — so it is *sometimes* reachable, which is worse than never.
    /// Paying deliberately is a conversation, not a door.
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

/// The maps a **hub tour** cannot enter, and why. **This is the L4 answer**, and every row is
/// evidence-backed rather than assumed — see the workstream's archive entry.
///
/// Most rows are rooms this save can never reach again. [`Unreachable::NotFromTheFlyStop`] is the
/// exception and is deliberately in the same table: it is still "the tour did not get in", and
/// hiding it in prose is how a real gap becomes folklore.
///
/// The S.S. Anne's ten maps dominate the list, and they are the reason §8-L says to check
/// `EVENT_SS_ANNE_LEFT` *before* planning a visit: the ship is a one-time event in the mainline and
/// every postgame fixture is downstream of it.
pub fn known_unreachable(map: Map) -> Option<Unreachable> {
    use Unreachable::*;
    match map {
        Map::SSAnne1F | Map::SSAnne2F | Map::SSAnne3F | Map::SSAnneB1F
        | Map::SSAnneBow | Map::SSAnneKitchen | Map::SSAnneCaptainsRoom
        | Map::SSAnne1FRooms | Map::SSAnne2FRooms | Map::SSAnneB1FRooms
        // ⚠️ **And the dock itself.** The ship being gone does not merely empty the pier — it seals
        // it: `VermilionCityDefaultScript` (`scripts/VermilionCity.asm:41-58`) intercepts the player
        // at `SSAnneTicketCheckCoords` whenever they face **down**, and with `EVENT_SS_ANNE_LEFT` set
        // the sailor says the ship has set sail and pushes them back. Found the hard way: the tour
        // stood one tile above the door being handed a valid walk to it, over and over, and the map
        // never changed — which is what taught `EnterMapIfReachable` to count every poll rather than
        // only the ones with nowhere to go.
        | Map::VermilionDock => Some(SsAnneSailed),
        Map::PokemonTower6F | Map::PokemonTower7F => Some(OneWayScript),
        Map::CeruleanCave1F | Map::CeruleanCave2F | Map::CeruleanCaveB1F => Some(NotFromTheFlyStop),
        // ⚠️ Two **roads** the tour rediscovered, both already in the archive — which is the most
        // reassuring thing about them. Cerulean is one of §10's "several sealed regions" maps and its
        // Route 4 seam is on the far side of the river from where Fly lands; and Saffron's Route 7
        // connection drops into a ledge-sealed pocket with no path to `Route7Gate`, which is why
        // workstream H5d routes to Route 7 **from Celadon** instead.
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

/// The eleven Fly stops, which is how §8-L slices the tour: one test per hub, so a failure costs one
/// town rather than the whole tour.
pub const FLY_HUBS: &[Map] = &[
    Map::PalletTown, Map::ViridianCity, Map::PewterCity, Map::CeruleanCity, Map::LavenderTown,
    Map::VermilionCity, Map::CeladonCity, Map::FuchsiaCity, Map::CinnabarIsland,
    Map::SaffronCity, Map::IndigoPlateau,
];

/// Rooms a tour must **not walk into**, and why. Distinct from [`known_unreachable`]: these can be
/// entered perfectly well, and that is the problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipTour {
    /// The door is sealed behind you until the room's trainer is beaten. Every Elite Four room does
    /// this (`ReplaceTileBlock` on the way in), so a tour that steps into Lorelei's room is committed
    /// to the whole gauntlet — and a tour is not a gauntlet.
    SealsTheDoor,
    /// Entering starts a **paid** Safari trip: ¥500 to the warden and the 502-step counter begins,
    /// after which the player is ejected wherever they stand. Workstream E owns that mechanic; the
    /// tour has no business spending money to look at a room.
    CostsAPaidTrip,
    /// **You can get in and you cannot get out.** An elevator's exit is its *own* warp tile, whose
    /// destination the floor menu rewrites at runtime — so to a tour that only walks through doors it
    /// is a room with no exit. Saffron found this the expensive way: the tour stepped into
    /// `SilphCoElevator`, could not leave, and then reported the Pokémon Center and Mr Psychic's
    /// house as unenterable because it was still standing in the lift. `PolicyStep::UseElevator` is
    /// the mechanism, and the Silph and Rocket Hideout workstreams already prove it.
    NeedsTheFloorMenu,
    /// A road lined with **trainers**, every one of whom fights on sight. Walking onto it is a
    /// gauntlet, and a gauntlet is `PolicyStep::BattleTrainer`'s job on a healed party — not a
    /// tour's, which arrives with whatever PP the last leg left.
    ///
    /// Route 8 is the case. The archive already records what it looks like from either end — *"the
    /// two `Route8Gate` doors, the Underground Path and **nine trainers**"* — and the tour walked
    /// straight into them: Lavender's run stopped polling the policy altogether four steps from the
    /// end and spent its whole cycle budget in a fight it could not finish, taking Vermilion's tour
    /// with it.
    TrainerGauntlet,
}

impl SkipTour {
    pub const fn why(self) -> &'static str {
        match self {
            Self::SealsTheDoor => "the door seals behind you until the room's trainer is beaten",
            Self::CostsAPaidTrip => "entering costs ¥500 and starts the Safari step counter",
            Self::NeedsTheFloorMenu => "an elevator: the way out is the floor menu, not a door",
            Self::TrainerGauntlet => "a road lined with trainers; that is BattleTrainer's job",
        }
    }
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

/// Rooms the tour **enters but does not walk deeper into**: the front door of a dungeon that has its
/// own workstream.
///
/// Silph Co is the reason. Its 1F is an ordinary lobby and its 2F is the start of an eleven-floor
/// card-key maze that `postgame::gifts` and the Marsh Badge work already cross properly — and a tour
/// that wanders one floor into it spends its whole budget failing to come back out, taking Saffron's
/// remaining rooms with it. The same applies to the Rocket Hideout, the Pokémon Mansion and Pokémon
/// Tower: one door in is coverage, two is someone else's job.
pub fn tour_leaf(map: Map) -> bool {
    matches!(map, Map::SilphCo1F | Map::RocketHideoutB1F | Map::PokemonMansion1F
                | Map::PokemonTower1F | Map::CeruleanCave1F | Map::IndigoPlateauLobby)
}

/// True for the two tilesets Fly considers "outside" — the tour uses them as its fence.
///
/// A hub's tour follows **warps only**, never connections: a connection leads to a route, a route
/// leads to the next town, and one hub's tour would otherwise become the whole of Kanto. Routes are
/// visited as part of the hub they hang off, one hop deep, and then the tour turns round.
fn is_outdoors(tileset: crate::pokemon::map_header::TileSetId) -> bool {
    use crate::pokemon::map_header::TileSetId;
    matches!(tileset, TileSetId::Overworld | TileSetId::Plateau)
}

/// The **path** to every room reachable from `hub` by walking through doors, up to `depth` doors
/// deep — read from the ROM's own warp tables rather than listed. Each entry starts at a room the hub
/// opens directly onto and ends at the room itself; the hub is not included.
///
/// Paths rather than a flat list, and that is the whole design. The first version returned a set of
/// rooms and had the tour walk back to the hub with a plain `EnterMap` between each — which worked
/// for the ground floor and then quietly failed for everything above it: standing in Red's bedroom,
/// "go to Pallet Town" is not a transition that exists, and the fallback route over the *incremental*
/// world graph is only as good as what the agent happens to have observed. Two of Pallet Town's four
/// rooms were skipped that way. A tour that knows how it got in knows how to get out.
///
/// Two rules keep it bounded, both in [`is_outdoors`]'s doc: **warps only**, and the recursion stops
/// at an outdoor map. Rooms in [`skip_tour`] are not entered and not recursed through.
pub fn room_paths(hub: Map, depth: u8) -> Vec<Vec<Map>> {
    use crate::mmu::MMU;
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
            // Stop at the front door of the next town (an outdoor map belongs to its own hub) and at
            // the front door of a dungeon (see `tour_leaf`).
            if !outdoors && !tour_leaf(to) { walk(mmu, to, depth - 1, path, seen, paths); }
            path.pop();
        }
    }
    walk(&mmu, hub, depth, &mut Vec::new(), &mut seen, &mut paths);
    paths
}

/// The maps a hub **walks** onto — its N/E/S/W connections, from the map header.
///
/// Separate from [`room_paths`] because they are a different kind of edge and the tour treats them
/// differently: a warp is a door into a room that leads nowhere else, a connection is the road to the
/// next town. §8-L asks for "every building in the cluster **and every connected route**", so the
/// tour steps onto each road and comes straight back rather than following it.
pub fn connected_routes(hub: Map) -> Vec<Map> {
    use crate::mmu::MMU;
    use crate::pokemon::map_header::MapHeaderReader;
    let Ok(mmu) = MMU::from_rom(crate::pokemon::roms::POKERED) else { return Vec::new() };
    let Ok(header) = mmu.read_map_header(hub) else { return Vec::new() };
    let mut out: Vec<Map> = Vec::new();
    for connection in header.connections() {
        // ⚠️ `skip_tour` applies to roads too, and Route 8 is why: nine trainers on sight, and the
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
    /// **L2** — the tour of one Fly hub: in and out of every room [`room_paths`] finds.
    ///
    /// Every hop is [`Self::EnterMapIfReachable`], never `EnterMap`, because the *point* is to find
    /// the rooms that cannot be entered — a hard stall at the first locked door would report nothing
    /// about the rest of the town. Each room is approached along its own path from the hub and left
    /// the same way, so a door that refuses to open costs that one room instead of desynchronising
    /// everything after it.
    pub fn tour_hub_steps(hub: Map, depth: u8) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: hub }];
        // Rooms first, **then** the roads out. ⚠️ The order is not cosmetic: a road is a walk into
        // open country with wild encounters and one-way ledges on it, and stepping onto one can leave
        // the agent somewhere it cannot walk back from. Put the roads first and Cerulean's tour
        // reports nine unenterable buildings that are all perfectly enterable — it simply never got
        // back to the town. Last, a road that strands the tour costs only the roads after it.
        let mut paths = room_paths(hub, depth);
        // ⚠️ And each road is one step out and straight back, never followed: a route leads to the
        // next town, whose rooms are that hub's tour.
        paths.extend(connected_routes(hub).into_iter().map(|m| vec![m]));
        for path in paths {
            // ⚠️ **Cut before every room, not once at the top.** Celadon's gym door is behind
            // cuttable trees, and cutting them once at the start of the tour is not enough for two
            // compounding reasons from §10: a cut tree **regrows** whenever the map reloads, which
            // every trip in and out of a building does, and `PokemonAgent::cut_tiles` — the memory
            // that lets the router treat a cut tile as walkable — is **cleared on every map change**
            // for exactly that reason. So by the third room of the tour the trees are back and the
            // agent has forgotten it ever cut them. `CutTree` pops immediately when no tree is
            // reachable, so on the other ten hubs this is one wasted poll per room.
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

/// **L3** — the maps §8-L singles out as "the awkward set", each with what actually became of it.
///
/// The plan asks for these to be *checked for reachability before budgeting*, and this is that check
/// written down: every one is either toured by L2, deliberately not entered, or unreachable with a
/// reason. Kept as data so [`tests::the_awkward_set_is_accounted_for`] can prove none of them quietly
/// fell off the list.
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
    use crate::mmu::MMU;
    use crate::pokemon::map_header::MapHeaderReader;
    use crate::pokemon::map_metadata::MapMetadataReader;
    use crate::pokemon::roms;

    fn rom() -> MMU {
        MMU::from_rom(roms::POKERED).expect("the bundled ROM should load")
    }

    /// **L1's first question**: how many maps are actually in scope, and does the plan's "~220" hold?
    ///
    /// Printed as well as asserted, because the number is the thing the rest of the workstream is
    /// budgeted against and "about two hundred and twenty" is not a budget.
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
        // ⚠️ The buckets **overlap**: three of the four duplicate slots are headerless too. Count the
        // union, not the sum — adding the four lists gives 251 for 248 maps, which is how the overlap
        // was found in the first place.
        let struck: std::collections::HashSet<Map> = headerless.iter().copied()
            .chain(LINK_CABLE_MAPS.iter().copied())
            .chain(DUPLICATE_MAPS.iter().copied())
            .collect();
        assert_eq!(visitable.len() + struck.len(), all, "every map should be in exactly one bucket");
        let headered_duplicates: Vec<Map> = DUPLICATE_MAPS.iter().copied()
            .filter(|m| m.header_pointer().is_some()).collect();
        println!("duplicates that are not already headerless: {headered_duplicates:?}");
        assert_eq!(headered_duplicates, vec![Map::UndergroundPathRoute7Copy]);
        // §8-L says "~220 are visitable: strike the 22 UnusedMap*, Colosseum and TradeCenter, and the
        // four duplicate slots". The headerless set is what the ROM actually has no label for, and it
        // is the number to trust.
        assert!((215..=225).contains(&visitable.len()),
            "expected ~220 visitable maps, got {}", visitable.len());
    }

    /// **L1 proper** — every visitable map's own metadata has to answer, without emulating anything.
    ///
    /// The point is the plan's: *"anything it catches would otherwise show up as a silent stall an
    /// hour into a tour."* Four questions per map, and each has bitten this project somewhere else:
    /// the header resolves, the tileset is one the code models, the block data is the size the header
    /// claims, and every warp lands on a map that exists.
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
            // Every warp has to name a map that exists — a warp into a headerless slot is a door the
            // agent would walk through into nothing.
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

    /// **L1** — the tile grid builds for every map, and is the size the header implies.
    ///
    /// Separate from the header check because it is the expensive half (it decodes every block of
    /// every map) and because it is the one that would *panic* rather than return an error — which is
    /// exactly the failure a tour would hit as a crash mid-walk.
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

    /// **L1** — a map with objects in the ROM must have sprites in [`Map::sprites`].
    ///
    /// This is the "missing metadata" the plan is most worried about: `Map::sprites` is a hand-written
    /// table, and a room whose row is empty is a room where `Interact` can never find anybody — which
    /// presents as a step that waits for ever, not as an error.
    #[test]
    fn every_map_with_objects_has_a_sprite_table() {
        let mmu = rom();
        let mut missing = Vec::new();
        for map in visitable() {
            let Ok(header) = mmu.read_map_header(map) else { continue };
            // `objects_address` is the map's `*_Object` block: border block, then the four counted
            // lists. The object-event count is the fifth byte-counted list; reading it is cheaper and
            // more honest than re-deriving the whole structure, so just check the count byte.
            let object_count = object_event_count(&mmu, &header);
            if object_count > 0 && map.sprites().is_empty() {
                missing.push(format!("{map}: {object_count} object events in the ROM, no sprite table"));
            }
        }
        assert!(missing.is_empty(), "{} maps have objects the agent cannot name:\n{}",
            missing.len(), missing.join("\n"));
    }

    /// Walk a map's `*_Object` structure to its object-event count.
    ///
    /// Layout (`macros/scripts/maps.asm`): `db border_block`, then **warps** (`db n` then 4 bytes
    /// each), **bg events** (`db n` then 3 bytes each), then **object events** (`db n` then …). Only
    /// the counts are needed here.
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

    /// The four duplicate slots really are duplicates — same tileset, dimensions and block pointer as
    /// the map they copy. Checked rather than asserted in prose, because striking a map from the tour
    /// on the strength of its *name* is how a real room gets skipped.
    #[test]
    fn the_duplicate_slots_really_are_duplicates() {
        let mmu = rom();
        for (copy, original) in [
            (Map::CeruleanTrashedHouseCopy, Map::CeruleanTrashedHouse),
            (Map::CinnabarMartCopy, Map::CinnabarMart),
            (Map::UndergroundPathRoute6Copy, Map::UndergroundPathRoute6),
            (Map::UndergroundPathRoute7Copy, Map::UndergroundPathRoute7),
        ] {
            // Three of the four have no header at all, which is a stronger form of "not a room" than
            // being a copy — nothing to compare, and nothing to visit.
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

    /// **L3** — every map §8-L calls awkward has an answer, and the answer matches the code.
    ///
    /// The plan's instruction is *"check reachability before budgeting"*, and the failure it is
    /// guarding against is a room that is neither toured nor explained — which reads, in a report, as
    /// though it had been covered. This pins [`AWKWARD_SET`] against the two tables the tour actually
    /// consults, so a row cannot say "toured" while `skip_tour` quietly drops it.
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

    /// Nothing in [`known_unreachable`] should be a map the tour is also expected to enter, and every
    /// entry should be a real visitable map rather than a typo.
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

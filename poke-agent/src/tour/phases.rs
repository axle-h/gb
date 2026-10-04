//! The tour's route: twenty-one phases of [`Step`]s, from a fresh save to the credits and on
//! through the errands the story leaves behind.

use crate::tour::brain::{Pc, Step};
use crate::pokemon::item::ItemId;
use crate::tour::completion::{Entry, Legend, Way};

/// The key items and HMs nothing wants back once the Volcano Badge phase begins, bar the Secret Key
/// it is yet to find: the bag holds twenty kinds, Full Heals among them, and with these in it a gift
/// or a pickup finds no room. Deposited at the first PC of every phase from there on, since a phase
/// played from its fixture may still carry them; one already in the PC, or not found yet, is refused.
const SPENT: [&str; 13] = [
    r#"{"move":"pc_items","op":"deposit","item":"TownMap"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"SSTicket"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"OldRod"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"CoinCase"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"LiftKey"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"SilphScope"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"PokeFlute"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"CardKey"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"Hm01Cut"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"Hm02Fly"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"Hm03Surf"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"Hm04Strength"}"#,
    r#"{"move":"pc_items","op":"deposit","item":"SecretKey"}"#,
];

/// Pallet Town to the Boulder Badge: the starter, the parcel, the Pokédex and Viridian Forest.
pub fn to_the_boulder_badge() -> Vec<Step> {
    use Step::*;
    vec![
        Go(&["RedsHouse1F"]), Clear(&[]),
        Go(&["PalletTown"]), Clear(&[]),
        // In at the front door and back up the stairs: the run comes down them once, at the start,
        // and the doors it walks then are the far sides of these.
        Go(&["RedsHouse1F"]), Go(&["RedsHouse2F"]), Go(&["RedsHouse1F"]), Go(&["PalletTown"]),
        // Oak stops the walk north and takes the player to his lab.
        Take("Route1"),
        Gift("SquirtlePokeBall"),
        Clear(&[]),
        Go(&["PalletTown", "Route1"]), Clear(&[]),
        Go(&["ViridianCity"]), Clear(&[]),
        // The clerk hands over Oak's Parcel as the door opens.
        Go(&["ViridianMart"]), Clear(&[]),
        Go(&["ViridianCity", "Route1", "PalletTown", "OaksLab"]), Talk("Oak1"),
        Go(&["PalletTown", "BluesHouse"]), Clear(&[]),
        Go(&["PalletTown", "Route1", "ViridianCity", "ViridianPokecenter"]), Clear(&[]),
        Go(&["ViridianCity", "ViridianSchoolHouse"]), Clear(&[]),
        Go(&["ViridianCity", "ViridianNicknameHouse"]), Clear(&[]),
        Go(&["ViridianCity", "Route22"]), Clear(&[]),
        Go(&["ViridianCity", "Route2"]), Clear(&[]),
        Go(&["ViridianForestSouthGate"]), Clear(&[]),
        Go(&["ViridianForest"]),
        Hunt { species: "Weedle", row: "Grass", ball: "MasterBall", way: Way::WildInGrass, on: "" },
        // The catch is the last of three in the party: it leads, and fights until it evolves at 7.
        Field(r#"{"move":"reorder_party","slot":2}"#),
        // A Kakuna or Metapod only hardens, and the fight never ends.
        Train { row: "Grass", until: "evolved into", way: Way::EvolvedInBattle, flee: &["Kakuna", "Metapod"] },
        Field(r#"{"move":"reorder_party","slot":1}"#),
        Clear(&[]),
        Go(&["ViridianForestNorthGate"]), Clear(&[]),
        Go(&["Route2", "PewterCity"]), Clear(&[]),
        Go(&["PewterPokecenter"]), Clear(&[]),
        Go(&["PewterCity", "PewterMart"]), Clear(&[]),
        Go(&["PewterCity", "PewterNidoranHouse"]), Clear(&[]),
        Go(&["PewterCity", "PewterSpeechHouse"]), Clear(&[]),
        Go(&["PewterCity", "Museum1F"]), Clear(&[]),
        Go(&["Museum2F"]), Clear(&[]),
        Go(&["Museum1F", "PewterCity", "PewterGym"]), Clear(&[]),
        Go(&["PewterCity"]),
    ]
}

/// Walk into `building` from wherever the run is, clear it, and walk back out to `town`.
fn visit(building: &'static str, town: &'static str) -> [Step; 3] {
    [Step::GoTo(building), Step::Clear(&[]), Step::GoTo(town)]
}

// The legs between towns, hop by hop. A phase starts with the brain's graph empty and a turn
// offers only the passages out of the pocket the run stands in, so every map on the way has to be
// named: a `GoTo` over a region the phase has not walked yet has nothing to route over.
//
// Saffron's four gates are buildings standing in the middle of their route rather than doors into
// the city: both of a gate's doors come out on the same route, one on each side of it, and the
// city is entered over the map edge beyond. So a leg through one names the far door by where it
// lands, which is what tells the two apart.

/// Celadon to Saffron, through Route 7's gate.
fn celadon_to_saffron() -> Vec<Step> {
    use Step::*;
    vec![GoTo("Route7"), GoTo("Route7Gate"), Take("Route7, arriving at (19, "), GoTo("SaffronCity")]
}

fn saffron_to_celadon() -> Vec<Step> {
    use Step::*;
    // Saffron's west wall has two openings and only the northern one comes out beside the gate;
    // the other lands in a strip of Route 7 whose one row is the way back into the city.
    vec![Cross { map: "Route7", landing: (20, 0) }, GoTo("Route7Gate"),
         Take("Route7, arriving at (12, "), GoTo("CeladonCity")]
}

/// Saffron to Cerulean, through Route 5's gate.
fn saffron_to_cerulean() -> Vec<Step> {
    use Step::*;
    vec![GoTo("Route5"), GoTo("Route5Gate"), Take("Route5, arriving at (10, 30)"), GoTo("CeruleanCity")]
}

/// Saffron to Vermilion, through Route 6's gate.
fn saffron_to_vermilion() -> Vec<Step> {
    use Step::*;
    vec![GoTo("Route6"), GoTo("Route6Gate"), Take("Route6, arriving at (10, 8)"), GoTo("VermilionCity")]
}

fn vermilion_to_saffron() -> Vec<Step> {
    use Step::*;
    vec![GoTo("Route6"), GoTo("Route6Gate"), Take("Route6, arriving at (10, 2)"), GoTo("SaffronCity")]
}

/// Saffron to Lavender, through Route 8's gate.
fn saffron_to_lavender() -> Vec<Step> {
    use Step::*;
    vec![GoTo("Route8"), GoTo("Route8Gate"), Take("Route8, arriving at (9, "), GoTo("LavenderTown")]
}

fn lavender_to_saffron() -> Vec<Step> {
    use Step::*;
    vec![GoTo("Route8"), GoTo("Route8Gate"), Take("Route8, arriving at (2, "), GoTo("SaffronCity")]
}

/// Celadon to Lavender under Saffron rather than through it, which is the way while the guard at
/// each of Saffron's gates is still waiting for his drink. Both stairwells are on the far side of
/// their route's gate from the city, so neither gate is touched.
fn celadon_to_lavender() -> Vec<Step> {
    use Step::*;
    vec![GoTo("Route7"), GoTo("UndergroundPathRoute7"), GoTo("UndergroundPathWestEast"),
          GoTo("UndergroundPathRoute8"), GoTo("Route8"), GoTo("LavenderTown")]
}

fn lavender_to_celadon() -> Vec<Step> {
    use Step::*;
    vec![GoTo("Route8"), GoTo("UndergroundPathRoute8"), GoTo("UndergroundPathWestEast"),
          GoTo("UndergroundPathRoute7"), GoTo("Route7"), GoTo("CeladonCity")]
}

/// Pewter to Bill: Route 3, Mt Moon, Cerulean and the Cascade Badge, Nugget Bridge and Route 25.
pub fn to_bill() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        GoTo("Route3"), Clear(&[]),
        GoTo("Route4"), Clear(&[]),
        // Each edge is a crossing of its own in the direction it is walked, and the road west is
        // walked nowhere else: every later leg comes at Cerulean from the south.
        GoTo("Route3"), GoTo("PewterCity"), GoTo("Route3"), GoTo("Route4"),
        // The Magikarp salesman, who counts as a way of obtaining a Pokémon.
        GoTo("MtMoonPokecenter"), Clear(&[]),
        // In at the mountain's door and straight back out of it, because every other way out of
        // Mt Moon comes out on the far half of Route 4.
        GoTo("Route4"), GoTo("MtMoon1F"), GoTo("Route4"), GoTo("MtMoon1F"),
        Explore { maps: &["MtMoon1F", "MtMoonB1F", "MtMoonB2F"], patience: 600 },
        // The exploring stops once the pockets it has stood in hold nothing more, and a thing
        // further off on a floor was never in one of those menus. A Clear pass over each floor
        // walks to whatever it missed, which is the same answer naming Route 12's gate was.
        GoTo("MtMoon1F"), Clear(&[]), GoTo("MtMoonB1F"), Clear(&[]), GoTo("MtMoonB2F"), Clear(&[]),
        // Route 4 is two halves, and only B2F's east ladder comes out on the far one.
        GoTo("MtMoonB2F"), Take("MtMoonB1F, arriving at (23, 3)"), Take("Route4"), Tidy, Clear(&[]),
        // And in again by the door that comes out here, which is a door of its own.
        GoTo("MtMoonB1F"), Take("Route4"),
        GoTo("CeruleanCity"), Clear(&[]),
    ];
    for building in ["CeruleanPokecenter", "CeruleanMart", "CeruleanBadgeHouse", "CeruleanTradeHouse",
                     "BikeShop", "CeruleanGym"] {
        steps.extend(visit(building, "CeruleanCity"));
    }
    steps.extend([
        // The badge house has a back door onto the terrace above it, and the terrace is reached
        // from nowhere else, so the way back is the door it was left by.
        GoTo("CeruleanBadgeHouse"), Take("CeruleanCity, arriving at (10, 10)"),
        Take("CeruleanBadgeHouse, arriving at (2, 0)"), GoTo("CeruleanCity"),
        GoTo("Route24"), Clear(&[]),
        // For the Route 2 trade house, which wants an Abra.
        Hunt { species: "Abra", row: "Grass", ball: "MasterBall", way: Way::WildInGrass, on: "" },
        GoTo("Route25"), Clear(&[]),
        // Bill hands over nothing to a full bag.
        Tidy,
        GoTo("BillsHouse"), Talk("BillPokemon"), Talk("CellSeparator1"), Talk("Bill1"), Clear(&[]),
        // The officer at the robbed house steps aside once Bill is himself again.
        GoTo("CeruleanCity"), GoTo("CeruleanTrashedHouse"), Clear(&[]),
        // Its back door comes out on Cerulean's other terrace, where the thief is.
        Take("CeruleanCity, arriving at (28, 10)"), Clear(&[]),
    ]);
    steps
}

/// The Pokémon Tower-free half of the south: Route 5 and 6, Vermilion and the S.S. Anne, Cut, and
/// Lt. Surge.
pub fn to_the_thunder_badge() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        // Route 5 is reached only from the terrace behind the robbed house.
        GoTo("CeruleanTrashedHouse"), Take("CeruleanCity, arriving at (28, 10)"),
        GoTo("Route5"), Clear(&[]),
        GoTo("UndergroundPathRoute5"), Clear(&[]),
        GoTo("UndergroundPathNorthSouth"), Clear(&[]),
        GoTo("UndergroundPathRoute6"), Clear(&[]),
        GoTo("Route6"), Clear(&[]),
        GoTo("VermilionCity"), Clear(&[]),
        Tidy,
    ];
    for building in ["VermilionPokecenter", "VermilionMart", "PokemonFanClub", "VermilionOldRodHouse",
                     "VermilionPidgeyHouse"] {
        steps.extend(visit(building, "VermilionCity"));
    }
    steps.extend([
        // The Old Rod's one catch is a Magikarp, at any water's edge.
        Hunt { species: "Magikarp", row: "Fish", ball: "MasterBall", way: Way::OldRod, on: "" },
        GoTo("Route11"), Clear(&[]),
        // For the Vermilion trade, which gives a Farfetch'd that can learn both Cut and Fly.
        Hunt { species: "Spearow", row: "Grass", ball: "MasterBall", way: Way::WildInGrass, on: "" },
        // The party is full, so the catch went to the box.
        GoTo("VermilionPokecenter"),
        AtPc(Pc::Deposit("Kakuna")), AtPc(Pc::Withdraw("Spearow")), AtPc(Pc::Release("Kakuna")),
        GoTo("VermilionCity"), GoTo("VermilionTradeHouse"), Trade("LittleGirl"), Clear(&[]),
        GoTo("VermilionCity"), Tidy,
        GoTo("VermilionDock"), Clear(&[]),
        GoTo("SSAnne1F"),
        Explore { maps: &["SSAnne1F", "SSAnne2F", "SSAnne3F", "SSAnneB1F", "SSAnneBow", "SSAnneKitchen",
                          "SSAnneCaptainsRoom", "SSAnne1FRooms", "SSAnne2FRooms", "SSAnneB1FRooms"], patience: 800 },
        GoTo("VermilionDock"), GoTo("VermilionCity"),
        Teach { item: "Hm01Cut", species: "Farfetchd" },
        Take("cut down the tree"),
        GoTo("VermilionGym"), Clear(&[]), TrashCans, Clear(&[]),
        GoTo("VermilionCity"),
    ]);
    steps
}

/// Diglett's Cave and Route 2's east side, the Day Care, Route 25's tree, Rock Tunnel, Lavender
/// and the way west under Saffron to Celadon.
pub fn to_celadon() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        Collect(true),
        // Out of the gym's corner, which the tree closes again behind every visit.
        Take("cut down the tree"),
        GoTo("Route11"), GoTo("DiglettsCaveRoute11"), Clear(&[]), GoTo("DiglettsCave"),
        Hunt { species: "*", row: "Pace", ball: "MasterBall", way: Way::WildOnACaveFloor, on: "" },
        GoTo("DiglettsCaveRoute2"), Clear(&[]), GoTo("Route2"),
        Explore { maps: &["Route2", "Route2TradeHouse", "Route2Gate"], patience: 300 },
        // The exploring can spend its whole idle allowance re-cutting the trees every battle
        // regrows before the trade house's door comes up, and the Abra is for its trade.
        GoTo("Route2TradeHouse"), Clear(&[]),
        GoTo("DiglettsCaveRoute2"), GoTo("DiglettsCave"), GoTo("DiglettsCaveRoute11"), GoTo("Route11"),
        GoTo("VermilionCity"), GoTo("Route6"), GoTo("UndergroundPathRoute6"), GoTo("UndergroundPathNorthSouth"),
        GoTo("UndergroundPathRoute5"), GoTo("Route5"), GoTo("CeruleanCity"),
        // The Fan Club's voucher buys the bicycle, and the cycling road south of Celadon is the
        // only way to Fuchsia on foot, so the bike is fetched on the one walk through Cerulean
        // that happens after Vermilion.
        GoTo("BikeShop"), Clear(&[]), GoTo("CeruleanCity"),
        GoTo("CeruleanTrashedHouse"), GoTo("CeruleanCity"),
        // Cerulean has three ways down onto Route 5 and the Day Care stands in the pocket only the
        // middle one reaches, which is the one the tree on the main terrace opens. Its landings
        // run from (6, 1) to (10, 1), and the middle of that is what tells it from its neighbours
        // at (4, 1) and (15, 1) wherever the walk stands when it asks.
        Take("cut down the tree"), Cross { map: "Route5", landing: (9, 1) },
        GoTo("Daycare"), DayCare(Some("Squirtle")),
        GoTo("Route5"), GoTo("CeruleanCity"),
        GoTo("Route24"), GoTo("Route25"), Take("cut down the tree"), Talk("TMSeismicToss"),
        GoTo("Route24"), GoTo("CeruleanCity"),
        GoTo("CeruleanTrashedHouse"), Take("CeruleanCity, arriving at (28, 10)"),
        GoTo("Route9"), Explore { maps: &["Route9"], patience: 200 },
        GoTo("Route10"), Explore { maps: &["Route10", "RockTunnelPokecenter"], patience: 200 },
        GoTo("RockTunnel1F"), Explore { maps: &["RockTunnel1F", "RockTunnelB1F"], patience: 600 },
        GoTo("RockTunnel1F"), Take("Route10, arriving at (9, 5"), Clear(&[]),
        // In at the tunnel's south door and out of its north one, which no walk through it takes:
        // every pass so far has come down from the north and left by the south.
        GoTo("RockTunnel1F"), Take("Route10, arriving at (9, 17)"),
        GoTo("RockTunnel1F"), Take("Route10, arriving at (9, 5"),
        GoTo("LavenderTown"), Clear(&[]),
        // North out of the town and back: the road in came off the tunnel's own end.
        GoTo("Route10"), GoTo("LavenderTown"),
    ];
    for building in ["LavenderPokecenter", "LavenderMart", "LavenderCuboneHouse", "MrFujisHouse", "NameRatersHouse"] {
        steps.extend(visit(building, "LavenderTown"));
    }
    steps.extend([
        GoTo("PokemonTower1F"), Clear(&[]), GoTo("PokemonTower2F"), Clear(&[]),
        GoTo("PokemonTower1F"), GoTo("LavenderTown"),
        GoTo("Route12"), Clear(&["Snorlax"]), GoTo("Route12Gate1F"), Clear(&[]), GoTo("Route12Gate2F"), Clear(&[]),
        // The north door: the one nearest where the gate was entered is the stairs' side.
        GoTo("Route12Gate1F"), Take("Route12, arriving at (11, 16)"), GoTo("LavenderTown"),
        GoTo("Route8"), Explore { maps: &["Route8"], patience: 200 },
        GoTo("Route8Gate"), Clear(&[]), GoTo("Route8"),
        GoTo("UndergroundPathRoute8"), Clear(&[]), GoTo("UndergroundPathWestEast"), Clear(&[]),
        GoTo("UndergroundPathRoute7"), Clear(&[]), GoTo("Route7"), Clear(&[]),
        GoTo("Route7Gate"), Clear(&[]), GoTo("CeladonCity"),
    ]);
    steps
}

/// Celadon: the Mart and its roof, the Mansion's Eevee, the Game Corner's prizes, Erika, and Route
/// 16's Fly house.
pub fn to_the_rainbow_badge() -> Vec<Step> {
    use Step::*;
    vec![
        Collect(true),
        GoTo("CeladonPokecenter"), Clear(&[]),
        AtPc(Pc::Deposit("Magikarp")), AtPc(Pc::Deposit("Magikarp")), AtPc(Pc::ChangeBox(2)),
        GoTo("CeladonCity"), Explore { maps: &["CeladonCity"], patience: 200 },
        GoTo("CeladonMart1F"), Clear(&[]),
        // One `buy_item` takes four kinds at most, and the clerk says goodbye after it.
        GoTo("CeladonMart2F"), Tidy, Talk("Clerk2"),
        Buy(&[("Tm32DoubleTeam", 1), ("Tm33Reflect", 1), ("Tm02RazorWind", 1), ("Tm07HornDrill", 1)]),
        Tidy, Talk("Clerk2"),
        Buy(&[("Tm37EggBomb", 1), ("Tm01MegaPunch", 1), ("Tm05MegaKick", 1), ("Tm09TakeDown", 1)]),
        Tidy, Talk("Clerk2"), Buy(&[("Tm17Submission", 1)]),
        Tidy, Clear(&[]),
        GoTo("CeladonMart3F"), Clear(&[]),
        GoTo("CeladonMart4F"), Talk("Clerk"),
        Buy(&[("FireStone", 1), ("WaterStone", 1), ("ThunderStone", 1), ("LeafStone", 1)]), Clear(&[]),
        GoTo("CeladonMart5F"), Clear(&[]),
        GoTo("CeladonMartRoof"), Clear(&["LittleGirl"]),
        // The girl trades a different machine for each drink, and takes the one she is shown.
        Take("buy a FRESH WATER"), Talk("LittleGirl"), Take("buy a SODA POP"), Talk("LittleGirl"),
        Take("buy a LEMONADE"), Talk("LittleGirl"),
        // One more, for the guards at Saffron's gates.
        Take("buy a FRESH WATER"),
        // Getting out of the lift crosses the lift's own door and not the floor's; a floor's door
        // is crossed by walking into the lift from it, so the ride down is taken a floor at a time.
        GoTo("CeladonMart5F"),
        GoTo("CeladonMartElevator"), Field(r#"{"move":"elevator","map":"CeladonMart4F"}"#), GoTo("CeladonMart4F"),
        // And the stairs down from the fifth floor, which every walk up the shop climbs the other way.
        GoTo("CeladonMart5F"), GoTo("CeladonMart4F"),
        GoTo("CeladonMartElevator"), Field(r#"{"move":"elevator","map":"CeladonMart3F"}"#), GoTo("CeladonMart3F"),
        GoTo("CeladonMartElevator"), Field(r#"{"move":"elevator","map":"CeladonMart2F"}"#), GoTo("CeladonMart2F"),
        GoTo("CeladonMartElevator"), Field(r#"{"move":"elevator","map":"CeladonMart1F"}"#), GoTo("CeladonMart1F"),
        GoTo("CeladonMartElevator"), Field(r#"{"move":"elevator","map":"CeladonMart2F"}"#), GoTo("CeladonMart2F"),
        GoTo("CeladonMart1F"),
        // The mart's other street door, at the far end of its ground floor.
        Take("CeladonCity, arriving at (11, 13)"),
        // By the back door: its stairwell is the one that reaches the roof house.
        Take("CeladonMansion1F, arriving at (4, 0)"),
        Explore { maps: &["CeladonMansion1F", "CeladonMansion2F", "CeladonMansion3F", "CeladonMansionRoof",
                          "CeladonMansionRoofHouse"], patience: 200 },
        // The exploring took the Eevee in the roof house.
        Evolve { item: "FireStone", species: "Eevee" },
        GoTo("CeladonMansionRoof"), GoTo("CeladonMansion3F"), GoTo("CeladonMansion2F"), GoTo("CeladonMansion1F"),
        GoTo("CeladonCity"),
        // The mansion has two stairwells and two street doors, and the exploring walks one of
        // each: the front door opens onto the half of the ground floor the back door cannot reach,
        // and the stairs off it run all the way up. Each hop names where it lands, because the two
        // stairwells read alike otherwise.
        Take("CeladonMansion1F, arriving at (4, 11)"),
        Take("CeladonMansion2F, arriving at (7, 1)"),
        Take("CeladonMansion3F, arriving at (6, 1)"),
        Take("CeladonMansionRoof, arriving at (6, 1)"),
        Take("CeladonMansion3F, arriving at (7, 1)"),
        Take("CeladonMansion2F, arriving at (6, 1)"),
        Take("CeladonMansion1F, arriving at (7, 1)"),
        Take("CeladonCity, arriving at (25, 9)"),
        GoTo("CeladonDiner"), Clear(&[]), GoTo("CeladonCity"),
        GoTo("CeladonHotel"), Clear(&[]), GoTo("CeladonCity"),
        GoTo("CeladonChiefHouse"), Clear(&[]), GoTo("CeladonCity"),
        GoTo("GameCorner"), Clear(&[]),
        Coins(9900),
        GoTo("CeladonCity"), GoTo("GameCornerPrizeRoom"), Clear(&[]),
        // The machines are items, and the bag holds twenty kinds.
        Tidy,
        Prize("Abra"), Field(r#"{"move":"prize","item":"Tm15HyperBeam"}"#), Field(r#"{"move":"prize","item":"Tm23DragonRage"}"#),
        GoTo("CeladonCity"), GoTo("GameCorner"), Coins(7700),
        GoTo("CeladonCity"), GoTo("GameCornerPrizeRoom"), Field(r#"{"move":"prize","item":"Tm50Substitute"}"#),
        GoTo("CeladonCity"), GoTo("CeladonGym"), Explore { maps: &["CeladonGym"], patience: 200 },
        GoTo("CeladonCity"), GoTo("Route16"), Explore { maps: &["Route16"], patience: 100 },
        // The Fly house is on the west side, through the gate's top corridor, which has no guard.
        Take("Route16Gate1F, arriving at (7, 2)"), Take("Route16, arriving at (17, 4)"),
        Explore { maps: &["Route16", "Route16FlyHouse"], patience: 100 },
        GoTo("Route16"), Take("Route16Gate1F, arriving at (0, 2)"), Take("Route16, arriving at (24, 4)"),
        // Fly is a field move the model has to be able to use, and the tour uses it once, where the
        // cartridge makes it the natural thing: the flight back to Viridian for the last badge.
        Teach { item: "Hm02Fly", species: "Farfetchd" },
        GoTo("CeladonCity"),
    ]
}

/// The Rocket Hideout and the Silph Scope, Pokémon Tower and the Poké Flute, and the two Snorlax
/// the flute wakes.
pub fn to_the_poke_flute() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        Collect(true),
        GoTo("GameCorner"), Take("look behind the poster"),
        GoTo("RocketHideoutB1F"),
        Explore { maps: &["RocketHideoutB1F", "RocketHideoutB2F", "RocketHideoutB3F", "RocketHideoutB4F"],
                  patience: 900 },
        // The Lift Key drops only when its Rocket is spoken to after his battle, which the
        // exploring does only if he saw the player first and cut the walk to him short.
        GoTo("RocketHideoutB4F"), Talk("Rocket3"), Explore { maps: &["RocketHideoutB4F"], patience: 60 },
        // Each floor's lift lobby is a room of its own, and B4F's is the only way into Giovanni's
        // half of that floor, so the lift is both the ride and the route.
        GoTo("RocketHideoutElevator"),
        Field(r#"{"move":"elevator","map":"RocketHideoutB1F"}"#),
        // The lobby has a Rocket in it, and beating him is what opens its door.
        Talk("Rocket5"),
        // The ride ends outside the lift, so the next one starts by walking back in.
        GoTo("RocketHideoutElevator"), Field(r#"{"move":"elevator","map":"RocketHideoutB4F"}"#),
        Explore { maps: &["RocketHideoutB1F", "RocketHideoutB2F", "RocketHideoutB3F", "RocketHideoutB4F"],
                  patience: 900 },
        GoTo("RocketHideoutElevator"), Field(r#"{"move":"elevator","map":"RocketHideoutB2F"}"#),
        GoTo("RocketHideoutB1F"), GoTo("GameCorner"), GoTo("CeladonCity"),
    ];
    steps.extend(celadon_to_lavender());
    steps.extend([
        // The tower fills the bag, and Mr Fuji hands over nothing it has no room for.
        Tidy,
        GoTo("PokemonTower1F"),
        Explore { maps: &["PokemonTower1F", "PokemonTower2F", "PokemonTower3F", "PokemonTower4F",
                          "PokemonTower5F", "PokemonTower6F", "PokemonTower7F"], patience: 900 },
        // Mr Fuji's thanks puts the player in his house, with the flute.
        GoTo("LavenderTown"), Tidy, GoTo("MrFujisHouse"), Clear(&[]), GoTo("LavenderTown"),
        // Mr Fuji's thanks put the run in his house rather than back down the tower, so the stairs
        // off the top floor are walked on a climb of their own.
        GoTo("PokemonTower7F"), GoTo("PokemonTower6F"), GoTo("LavenderTown"),
        // Route 12 is two halves either side of its gate, and the Snorlax sleeps on the south one.
        GoTo("Route12"), GoTo("Route12Gate1F"), Take("Route12, arriving at (11, 2"),
        UseItemOn { item: "PokeFlute", row: "Snorlax" },
        Explore { maps: &["Route12"], patience: 400 },
        GoTo("Route12SuperRodHouse"), Clear(&[]), GoTo("Route12"),
        // Back north through the gate that splits the route, and under Saffron to Celadon.
        GoTo("Route12Gate1F"), Take("Route12, arriving at (11, 16)"), GoTo("LavenderTown"),
    ]);
    steps.extend(lavender_to_celadon());
    steps.extend([
        GoTo("Route16"), UseItemOn { item: "PokeFlute", row: "Snorlax" },
        // The gate's stairs are in its lower hall, past where the Snorlax slept.
        GoTo("Route16Gate1F"), GoTo("Route16Gate2F"), Clear(&[]), GoTo("Route16Gate1F"), Clear(&[]),
        // Out of the gate's lower hall onto the cycling road's side, where the bikers are. That
        // side is a pocket of its own, so the way back to Celadon is the hall it came out of.
        Take("Route16, arriving at (17, 1"), Clear(&[]),
        Take("Route16Gate1F, arriving at (0, 8)"), Take("Route16, arriving at (24, 1"),
        GoTo("CeladonCity"),
    ]);
    steps
}

/// Saffron: the gate guard's drink, Silph Co from the lobby to the president, Sabrina, the Dojo,
/// and the Copycat's Poké Doll.
pub fn to_the_marsh_badge() -> Vec<Step> {
    use Step::*;
    const SILPH: &[&str] = &["SilphCo1F", "SilphCo2F", "SilphCo3F", "SilphCo4F", "SilphCo5F",
                             "SilphCo6F", "SilphCo7F", "SilphCo8F", "SilphCo9F", "SilphCo10F",
                             "SilphCo11F"];
    let mut steps = vec![
        Collect(true),
        // The Copycat trades TM31 for a Poké Doll, sold on Celadon Mart's fourth floor, and the
        // run is still in Celadon: the shop comes before the walk east rather than a trip back.
        GoTo("CeladonMart1F"), GoTo("CeladonMart2F"), GoTo("CeladonMart3F"), GoTo("CeladonMart4F"),
        Talk("Clerk"), Buy(&[("PokeDoll", 1)]),
        // Out by the front door, which no other walk through the shop leaves by.
        GoTo("CeladonMart3F"), GoTo("CeladonMart2F"), GoTo("CeladonMart1F"), Take("CeladonCity, arriving at (9, 13)"),
        // The guard takes the drink as the player walks past him, without being talked to, and
        // the gate's far door names the map it was entered from until he does.
        // Both doors lead back onto Route 7: the guard is what blocks the room between them. His
        // thanks stop the walk and leave the player where they stood, so the walk goes on; the
        // door is repeated in case something else stops it.
        GoTo("Route7"), GoTo("Route7Gate"), Repeat("Route7, arriving at (19, "), GoTo("SaffronCity"),
    ];
    steps.extend([
        Explore { maps: &["SaffronCity", "SaffronPokecenter", "SaffronMart", "SaffronPidgeyHouse",
                          "MrPsychicsHouse", "CopycatsHouse1F", "CopycatsHouse2F", "FightingDojo"],
                  patience: 800 },
        // Silph Co hands over a Lapras and a Master Ball, and neither fits a full bag.
        Tidy,
        GoTo("SilphCo1F"), Explore { maps: SILPH, patience: 2500 },
        // 11F's office is walled off from its own lift and stairs. The way in is a chain of
        // teleport pads: 3F (11, 11) lands on 7F (5, 3), and 7F (5, 7) lands inside the office.
        GoTo("SilphCoElevator"), Field(r#"{"move":"elevator","map":"SilphCo3F"}"#),
        Take("warp to SilphCo7F, arriving at (5, 3)"),
        Take("warp to SilphCo11F, arriving at (3, 2)"),
        // The president reaches into his pocket for a Master Ball.
        Tidy,
        // Giovanni and the president are the exploring's, wherever it meets them: what the phase
        // asserts is the badge, the Lapras and the Master Ball, not who was talked to when.
        Explore { maps: SILPH, patience: 1500 },
        Take("warp to SilphCo7F, arriving at (5, 7)"),
        Take("warp to SilphCo3F, arriving at (11, 11)"),
        // Getting out of the lift crosses the lift's own door and not the floor's: a floor's door
        // is crossed by walking into the lift from it, so the ride down is taken a floor at a time.
        GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo11F"}"#), GoTo("SilphCo11F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo10F"}"#), GoTo("SilphCo10F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo9F"}"#), GoTo("SilphCo9F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo8F"}"#), GoTo("SilphCo8F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo7F"}"#), GoTo("SilphCo7F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo6F"}"#), GoTo("SilphCo6F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo5F"}"#), GoTo("SilphCo5F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo4F"}"#), GoTo("SilphCo4F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo3F"}"#), GoTo("SilphCo3F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo2F"}"#), GoTo("SilphCo2F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo1F"}"#), GoTo("SilphCo1F"), GoTo("SilphCoElevator"),
        Field(r#"{"move":"elevator","map":"SilphCo2F"}"#), GoTo("SilphCo2F"),
        GoTo("SilphCo1F"), GoTo("SaffronCity"),
        // Sabrina's gym opens once Silph Co is clear of Rockets, and freeing the president is what
        // clears the streets the Rockets were standing in.
        GoTo("SaffronGym"), Explore { maps: &["SaffronGym"], patience: 600 }, GoTo("SaffronCity"),
        // Three of the gym's thirty pads are the way back along a link the exploring only rode
        // one way, and a room is reached by its pads alone: each hop below names where it lands,
        // which is the only thing telling one pad in a room from another, and the pads the run
        // has taken before are the way round to the three it has not.
        GoTo("SaffronGym"),
        Take("SaffronGym, arriving at (19, 17)"), Take("SaffronGym, arriving at (19, 3)"),
        Take("SaffronGym, arriving at (1, 3)"), Take("SaffronGym, arriving at (1, 11)"),
        Take("SaffronGym, arriving at (9, 5)"), Take("SaffronGym, arriving at (5, 3)"),
        Take("SaffronGym, arriving at (1, 11)"), Take("SaffronGym, arriving at (5, 17)"),
        Take("SaffronGym, arriving at (15, 17)"), Take("SaffronGym, arriving at (11, 15)"),
        GoTo("SaffronCity"),
        // Freeing the president sends the Rockets home, and the doors they stood in open. The
        // Dojo's master has to be beaten before either of his Poké Balls will open.
        GoTo("FightingDojo"), Talk("KarateMaster"), Clear(&[]), GoTo("SaffronCity"),
        GoTo("SaffronPidgeyHouse"), Clear(&[]), GoTo("SaffronCity"),
        // Then the rest of the city, which was Rockets and shut doors the first time round.
        Explore { maps: &["SaffronCity", "SaffronPidgeyHouse", "CopycatsHouse1F", "CopycatsHouse2F",
                          "MrPsychicsHouse", "FightingDojo", "SaffronPokecenter", "SaffronMart"],
                  patience: 800 },
    ]);
    steps
}

/// Fuchsia: the cycling road down Route 16 to 18, the city behind its trees, and Koga.
pub fn to_the_soul_badge() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        Collect(true),
        // The phase before ended wherever its exploring left off.
        GoTo("SaffronCity"),
    ];
    steps.extend(saffron_to_celadon());
    steps.extend([
        // The cycling road is the way south, and the guard only lets a cyclist by.
        GoTo("Route16"), GoTo("Route16Gate1F"), Clear(&[]),
        Take("Route16, arriving at (17, 1"),
        GoTo("Route17"), Explore { maps: &["Route17"], patience: 400 },
        GoTo("Route18"), Explore { maps: &["Route18", "Route18Gate1F", "Route18Gate2F"], patience: 300 },
        // And back up the road before going on. Every leg over it runs south, because that is the
        // way its ledges drop and the way it carries a rider who lets go of the pad, so the edges
        // it is left by northward are crossings nothing else takes.
        GoTo("Route17"), GoTo("Route16"), GoTo("Route17"), GoTo("Route18"),
        // Fuchsia is carved into pockets by eight cuttable trees, so the doors are the walk's.
        GoTo("FuchsiaCity"),
        Explore { maps: &["FuchsiaCity", "FuchsiaPokecenter", "FuchsiaMart", "FuchsiaBillsGrandpasHouse",
                          "FuchsiaGoodRodHouse", "WardensHouse", "FuchsiaMeetingRoom"], patience: 800 },
        GoTo("FuchsiaGym"), Explore { maps: &["FuchsiaGym"], patience: 400 }, GoTo("FuchsiaCity"),
    ]);
    steps
}

/// The Safari Zone: the warden's teeth, the Surf in its secret house, and Strength from the
/// warden once his teeth are back.
pub fn to_surf() -> Vec<Step> {
    use Step::*;
    const SAFARI: &[&str] = &["SafariZoneGate", "SafariZoneCenter", "SafariZoneEast", "SafariZoneNorth",
                              "SafariZoneWest", "SafariZoneCenterRestHouse", "SafariZoneEastRestHouse",
                              "SafariZoneNorthRestHouse", "SafariZoneWestRestHouse", "SafariZoneSecretHouse"];
    vec![
        // The zone's grass is thick with species the run has never caught, and every one of them
        // is a catch that spends the five hundred steps a visit is worth.
        Collect(false), Tidy,
        // Nothing the run is carrying can take Surf and the zone's own grass is the nearest thing
        // that can, so the hunt goes first, on a whole visit rather than the tail of one: the zone
        // ends a visit after five hundred steps and an exploring spends them. A Safari battle
        // offers a ball rather than a bag, so the hunt needs no wording of its own.
        GoTo("SafariZoneGate"), Clear(&[]),
        GoTo("SafariZoneEast"),
        Hunt { species: "Kangaskhan", row: "Grass", ball: "MasterBall", way: Way::WildInGrass, on: "SafariZoneEast" },
        Explore { maps: SAFARI, patience: 1200 },
        // The west side and the house on it, named: the gate is a turnstile, and an exploring that
        // has to pay its way back in each time will not wander that far on its own.
        GoTo("SafariZoneWest"), Clear(&[]),
        GoTo("SafariZoneWestRestHouse"), Clear(&[]), GoTo("SafariZoneWest"),
        GoTo("SafariZoneSecretHouse"), Clear(&[]), GoTo("SafariZoneWest"),
        GoTo("SafariZoneNorth"), GoTo("SafariZoneNorthRestHouse"), Clear(&[]),
        // The centre is two halves that do not join: the gate opens on the south one, and what
        // stands in the north half is only reachable coming down from the north.
        GoTo("SafariZoneNorth"), GoTo("SafariZoneCenter"), Clear(&[]),
        GoTo("FuchsiaCity"), Collect(true),
        // The teeth buy HM04 in this house, and the house's own boulder stands on the one square
        // the Rare Candy can be faced from, so Strength is taught where it is handed over and the
        // shove that clears the square happens before the run leaves.
        GoTo("WardensHouse"), Clear(&[]),
        Teach { item: "Hm04Strength", species: "Mewtwo" },
        Talk("PushBoulderLeft"), Talk("RareCandy"),
        GoTo("FuchsiaCity"),
        // The party was full, so the catch went to the box and has to be fetched to be taught.
        GoTo("FuchsiaPokecenter"),
        AtPc(Pc::Deposit("Abra")), AtPc(Pc::Withdraw("Kangaskhan")),
        GoTo("FuchsiaCity"),
        Teach { item: "Hm03Surf", species: "Kangaskhan" },
        // The water the phases before could only look at from dry land is in the north and the
        // south, and the run is in Fuchsia: each stretch is taken by the phase that walks past it
        // rather than flown to from here.
    ]
}

/// What the game will not hand over, named so that each is out of reach for a reason somebody can
/// read rather than filtered away.
pub fn out_of_reach() -> Vec<Entry> {
    use crate::pokemon::map::Map;
    use crate::pokemon::map_header::MapConnectionDirection;
    let door = |map, index| Entry::Warp { map, index };
    let mut out = vec![
        // The Safari Zone's Nugget stands on an island and `TilePairCollisionsWater` refuses the
        // step from its banks, so nothing routes to it and Surf is no help.
        Entry::ItemBall { map: Map::SafariZoneCenter, object: 1, item: ItemId::Nugget as u8 },
        // Three warps the disassembly itself marks `; inaccessible`: a door into Celadon Mart's
        // fifth floor standing in a wall of the city, and two of Silph Co's teleport pads.
        door(Map::CeladonCity, 8), door(Map::SilphCo1F, 4), door(Map::SilphCo11F, 2),
    ];
    // The Elite Four's rooms are each walked one way. Lorelei, Bruno and Agatha turn the player
    // back from the door with "Don't run away!", Lance's entrance is walled up behind the run on
    // the way in, the Champion's room starts the rival the moment it is entered and ends with Oak
    // walking the player north, and the Hall of Fame ends in the credits and a reset.
    out.extend([Map::LoreleisRoom, Map::BrunosRoom, Map::AgathasRoom, Map::LancesRoom,
                Map::ChampionsRoom, Map::HallOfFame].map(|room| door(room, 0)));
    // Route 22 and Route 23 declare a border the cartridge calls unnecessary and the gate between
    // them is the way across: Route 22's side of it is a ledge and a mountain, and Route 23's is a
    // wall.
    out.extend([Entry::Connection { map: Map::Route22, direction: MapConnectionDirection::North },
                Entry::Connection { map: Map::Route23, direction: MapConnectionDirection::South }]);
    // The stairs off B4F's south east corner. They stand on the one square of a channel that is
    // not water: a walk comes down them and can only leave by surfing, and the cartridge fires no
    // warp under a surfing player, so nothing ever steps back onto them.
    out.push(door(Map::SeafoamIslandsB4F, 0));
    out
}

/// Cinnabar: the two sea routes south, the island and its lab, the fossil the run has carried
/// since Mt Moon, the Mansion's statue switches and the Secret Key behind them, then Blaine.
pub fn to_the_volcano_badge() -> Vec<Step> {
    use Step::*;
    const MANSION: &[&str] = &["PokemonMansion1F", "PokemonMansion2F", "PokemonMansion3F",
                               "PokemonMansionB1F"];
    let mut steps = vec![
        Collect(true),
        // The zone left the bag full, and a full bag refuses a pickup and a gift in silence.
        Tidy, GoTo("FuchsiaPokecenter"),
    ];
    steps.extend(SPENT.map(Field));
    steps.extend([
        GoTo("FuchsiaCity"),
        // South over the water: the sea routes, which no phase before this one could cross.
        GoTo("Route19"), Explore { maps: &["Route19"], patience: 500 },
        GoTo("Route20"), Explore { maps: &["Route20"], patience: 600 },
        GoTo("Route19"), GoTo("FuchsiaCity"),
    ]);
    // Route 20 is a north channel and a south channel with the islands between them, and they do
    // not meet: the north one runs east to Route 19 and the south one west to Cinnabar, and
    // neither the islands' ground floor nor the floor below it joins the two. So Cinnabar is
    // reached the way the cartridge intends, down Route 21 from Pallet, and on foot that is the
    // whole world away: the eastern routes up to Lavender, through Saffron to Vermilion, Diglett's
    // Cave to Route 2, and down through Viridian. Not the cycling road, which only goes south: its
    // ledge onto Route 18 is a crossing the menu offers and the walk cannot take.
    steps.extend([
        GoTo("Route15"), GoTo("Route15Gate1F"), Take("Route15, arriving at (15, "),
        GoTo("Route14"), GoTo("Route13"), GoTo("Route12"),
        GoTo("Route12Gate1F"), Take("Route12, arriving at (11, 16)"), GoTo("LavenderTown"),
    ]);
    steps.extend(lavender_to_saffron());
    steps.extend(saffron_to_vermilion());
    steps.extend([
        GoTo("Route11"), GoTo("DiglettsCaveRoute11"), GoTo("DiglettsCave"), GoTo("DiglettsCaveRoute2"),
        GoTo("Route2"), GoTo("Route2Gate"), Repeat("Route2, arriving at (15, 40)"),
        GoTo("ViridianCity"), GoTo("Route1"), GoTo("PalletTown"),
        GoTo("Route21"), Explore { maps: &["Route21"], patience: 500 },
        GoTo("CinnabarIsland"),
        // The south channel, which only Cinnabar's own shore opens onto.
        GoTo("Route20"), Explore { maps: &["Route20"], patience: 600 },
        GoTo("CinnabarIsland"),
        Explore { maps: &["CinnabarIsland", "CinnabarPokecenter", "CinnabarMart"], patience: 400 },
        // The lab is four maps: the hall and three back rooms.
        GoTo("CinnabarLab"), Clear(&[]),
        GoTo("CinnabarLabTradeRoom"), Clear(&[]), GoTo("CinnabarLab"),
        GoTo("CinnabarLabMetronomeRoom"), Clear(&[]), GoTo("CinnabarLab"),
        // The scientist takes the fossil, and hands the Pokemon over only after a walk out of the
        // room and back: leaving is what clears the flag saying he is still working on it. The
        // party is full, so it arrives in the box, which the dex entry records either way.
        GoTo("CinnabarLabFossilRoom"), Talk("Scientist1"),
        GoTo("CinnabarLab"), GoTo("CinnabarIsland"),
        GoTo("CinnabarLab"), GoTo("CinnabarLabFossilRoom"), Talk("Scientist1"), Clear(&[]),
        GoTo("CinnabarLab"), GoTo("CinnabarIsland"),
        // The Mansion. One switch toggles every floor's doors at once, so the statues are pressed
        // in the order the way down needs them. The exploring cannot disturb that: it takes people
        // and item balls, never a statue, whose row carries its coordinates and so two colons.
        GoTo("PokemonMansion1F"),
        Explore { maps: &["PokemonMansion1F", "PokemonMansion2F", "PokemonMansion3F"], patience: 1000 },
        // By the north stairs: the statue's side of 3F is not the side the middle stairs land on.
        GoTo("PokemonMansion2F"), Take("PokemonMansion3F, arriving at (6, 1)"), Talk("Statue1"),
        // 3F's holes are warps down to 1F's right side, which is the only way to the B1F stairs.
        GoTo("PokemonMansion1F"), GoTo("PokemonMansionB1F"),
        Talk("Statue2"),
        Explore { maps: MANSION, patience: 1200 },
        Talk("Statue1"), Clear(&[]),
        // The stairs up are shut by the two flips that opened the key's corner, so they are undone
        // in the order they were made.
        Talk("Statue1"), Talk("Statue2"),
        GoTo("PokemonMansion1F"), GoTo("CinnabarIsland"),
        // Six machines, six gates, and Blaine behind the lot of them. An exploring opens none of
        // them: it takes people and item balls, whose ids have one colon, and a machine's row
        // carries its coordinates and so has two. Every machine is answered YES, which is right
        // for the first, where the ledger's quiz entry comes from, and wrong for some of the rest,
        // whose gate opens anyway once the trainer the wrong answer sets on you is beaten.
        GoTo("CinnabarGym"),
        // A machine is answered and the step is done the moment its row is chosen, but the answer
        // is not: a wrong one walks the trainer over and fights him, and only winning that opens
        // the gate. So each press is given the turns to land before the next is asked for, or the
        // one after it hunts for a row on the far side of a gate that never opened.
        Talk("QuizYes1"), Explore { maps: &["CinnabarGym"], patience: 150 },
        Talk("QuizYes3"), Explore { maps: &["CinnabarGym"], patience: 150 },
        Talk("QuizYes5"), Explore { maps: &["CinnabarGym"], patience: 150 },
        Talk("QuizYes7"), Explore { maps: &["CinnabarGym"], patience: 150 },
        Talk("QuizYes9"), Explore { maps: &["CinnabarGym"], patience: 150 },
        Talk("QuizYes11"), Explore { maps: &["CinnabarGym"], patience: 900 },
        Talk("Blaine"),
        GoTo("CinnabarIsland"),
    ]);
    steps
}

/// Seafoam: the islands Route 20 is split by, four floors of holes down to the west lake, and
/// Articuno at the bottom of them.
pub fn to_seafoam() -> Vec<Step> {
    use Step::*;
    const SEAFOAM: &[&str] = &["SeafoamIslands1F", "SeafoamIslandsB1F", "SeafoamIslandsB2F",
                               "SeafoamIslandsB3F", "SeafoamIslandsB4F"];
    vec![
        // The floors are thick with wilds, and every new species is a catch that spends the turns
        // the boulders need, so the collecting waits for the bird the phase came for.
        Collect(false), Tidy,
        // The walk west filled the box, and a full box refuses every ball: a legendary met with
        // nothing to throw is run from, and that hides it for the rest of the game.
        GoTo("CinnabarPokecenter"), AtPc(Pc::ChangeBox(5)), GoTo("CinnabarIsland"),
        // Cinnabar's own shore is on Route 20's south channel, and the islands' east door with it.
        GoTo("Route20"), GoTo("SeafoamIslands1F"),
        // Out of the east door and in again: the floors below drain west, so a walk that comes in
        // by this door never takes it, and the pocket it opens into is the one it comes out of.
        Take("Route20, arriving at (59, 9)"), GoTo("SeafoamIslands1F"),
        Explore { maps: SEAFOAM, patience: 1200 },
        // B3F's two holes, each filled by the one boulder that can reach it. The row names both,
        // and arming Strength is the row's own business. The boulders' side is the west stairs'.
        Enter { map: "SeafoamIslandsB3F", landing: (5, 12) },
        Repeat("hole at (3, 16)"),
        Repeat("hole at (6, 16)"),
        // Down the hole just filled, into the west lake: the staircases land on the other side,
        // which the current walls off from the bird.
        Take("SeafoamIslandsB4F, arriving at (5, 14)"),
        // Hunted rather than talked to, as the other legendaries are: a talk is done the moment
        // its row is chosen, and the throw then rides on the collecting arm instead.
        Hunt { species: "Articuno", row: "Articuno", ball: "MasterBall",
               way: Way::Legendary(Legend::Articuno), on: "SeafoamIslandsB4F" },
        Collect(true),
        Explore { maps: SEAFOAM, patience: 600 },
        // The islands are a funnel: each door opens into a pocket of its own, both pockets drain
        // into the floors between, and nothing drains back. Whichever door the run came in by, the
        // way out is the west one, onto Route 20's north channel and back to Fuchsia.
        GoTo("SeafoamIslands1F"), Take("Route20, arriving at (49, 5)"),
        // And in by that door, which is a door of its own and opens onto the pocket just left.
        Take("SeafoamIslands1F, arriving at (4, 17)"), Take("Route20, arriving at (49, 5)"),
        GoTo("Route19"), GoTo("FuchsiaCity"),
    ]
}

/// The Earth Badge: off the water at Cinnabar, north to Viridian, and the gym that was shut when
/// the run first walked past it.
pub fn to_the_earth_badge() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        Collect(true), Tidy,
        // The one flight of the tour, and the one the cartridge makes the natural thing: Viridian
        // is the whole world away from Fuchsia, and the gym that was shut when the run first
        // walked past it is the last badge. Every other leg is walked.
        Field(r#"{"move":"fly","map":"ViridianCity"}"#), GoTo("ViridianCity"),
        GoTo("ViridianPokecenter"),
    ];
    steps.extend(SPENT.map(Field));
    steps.extend([
        GoTo("ViridianCity"),
        // Giovanni is only in the gym once Silph Co has sent the Rockets home. The arrow tiles are
        // the floor itself rather than an obstacle: `MetaTileMap` slides a route over them, so the
        // gym is walked like any other.
        GoTo("ViridianGym"), Explore { maps: &["ViridianGym"], patience: 600 },
        GoTo("ViridianCity"),
    ]);
    steps
}

/// The Power Plant: Zapdos, and an Electrode standing where an item ball would.
pub fn to_the_power_plant() -> Vec<Step> {
    use Step::*;
    vec![
        Collect(false), Tidy,
        // Two catches to make, and a full box refuses every ball: a static met with nothing to
        // throw is run from, which hides it for good.
        GoTo("CeruleanPokecenter"), AtPc(Pc::ChangeBox(3)), GoTo("CeruleanCity"),
        // The robbed house's back door is the only way onto the half of Cerulean that Route 9
        // opens off, which is how every phase before this one has reached that side.
        GoTo("CeruleanTrashedHouse"), Take("CeruleanCity, arriving at (28, 10)"),
        GoTo("Route9"), GoTo("Route10"), GoTo("PowerPlant"),
        // Six Voltorb and two Electrode stand where item balls would, and the turn offers each as
        // "pick up the Voltorb", which is the cartridge's own trick rather than ours. One is taken
        // by name before the exploring walks into the rest: only a hunt records the way a Pokemon
        // was come by, so an exploring that met one first would catch it and tell the ledger
        // nothing. It is an Electrode because the floor's grass rolls Voltorb, and a hunt takes
        // whichever of its species turns up first.
        Hunt { species: "Electrode", row: "Electrode1", ball: "MasterBall", way: Way::PowerPlantBall,
               on: "PowerPlant" },
        // Zapdos is hunted rather than talked to. A talk is done the moment its row is chosen, and
        // the walk to this one is interrupted by the floor's wilds over and over, so by the time it
        // lands the step has moved on and the throw rides on the collecting arm instead, which a
        // full box switches off for good. The collecting stays off all phase for the same reason:
        // the floor's wilds answer no ledger entry, and catching them fills the box with Pokemon
        // nothing asked for until the bird has nowhere to go.
        Hunt { species: "Zapdos", row: "Zapdos", ball: "MasterBall",
               way: Way::Legendary(Legend::Zapdos), on: "PowerPlant" },
        Explore { maps: &["PowerPlant"], patience: 800 },
        GoTo("Route10"),
    ]
}

/// Victory Road: Route 22's gate and Route 23's badge checks, the three Strength floors, Moltres on
/// the second of them, and out onto the plateau.
pub fn to_victory_road() -> Vec<Step> {
    use Step::*;
    const ROAD: &[&str] = &["VictoryRoad1F", "VictoryRoad2F", "VictoryRoad3F"];
    let mut steps = vec![
        Collect(false), Tidy,
        // The phases before filled the party and the box, and a full box refuses every ball, so the
        // bird would be met with nothing to throw.
        GoTo("ViridianPokecenter"), AtPc(Pc::ChangeBox(4)),
    ];
    steps.extend(SPENT.map(Field));
    steps.extend([
        GoTo("ViridianCity"),
        GoTo("Route22"), Explore { maps: &["Route22"], patience: 300 },
        GoTo("Route22Gate"), Clear(&[]),
        GoTo("Route23"), Explore { maps: &["Route23"], patience: 300 },
        // And back down through the gate: it stands between two routes rather than inside one, so
        // each of its doors and each of the tiles that open them is a crossing of its own.
        Take("Route22Gate, arriving at (4, 0)"), Take("Route22, arriving"),
        GoTo("Route22Gate"), GoTo("Route23"),
        GoTo("VictoryRoad1F"),
        // The road has a door at each end of Route 23's shelf and the walk through uses one of
        // each pair: in at the lower one and out at the upper. Out of the lower one and back in
        // while the run is still beside it, because the shelf's two ends do not join.
        Take("Route23, arriving at (4, 32)"), Take("VictoryRoad1F, arriving at (8, 17)"),
        // Four Strength goals and a hole across three floors, in the order the way up needs them.
        Repeat("switch at (17, 13)"), Explore { maps: &["VictoryRoad1F"], patience: 400 },
        GoTo("VictoryRoad2F"),
        Repeat("switch at (1, 16)"),
        // The shove ends wherever the boulder took the walk, which is sometimes down a hole.
        GoTo("VictoryRoad2F"),
        // 1F's north pocket is only reached down 2F's west ladder, and its two balls stand on a ledge
        // behind a boulder that serves no switch. Shoved all the way along, it ends beside the TM on
        // the square the Rare Candy is taken from, so the floor is left and re-entered to put it
        // back, and the second pass lifts it out of the row instead.
        Take("VictoryRoad1F, arriving at (1, 1)"),
        Take("boulder at (14, 2) one square left"), Take("boulder at (13, 2) one square left"),
        Take("boulder at (12, 2) one square left"), Take("boulder at (11, 2) one square left"),
        // The floors' pickups fill the bag, and a full bag refuses a ball in silence.
        Tidy, Talk("TMSkyAttack"),
        Take("VictoryRoad2F, arriving at (0, 8)"), Take("VictoryRoad1F, arriving at (1, 1)"),
        Take("boulder at (14, 2) one square left"), Take("boulder at (13, 2) one square left"),
        Take("boulder at (12, 2) one square left"), Take("boulder at (11, 2) one square up"),
        Talk("RareCandy"), GoTo("VictoryRoad2F"),
        GoTo("VictoryRoad3F"),
        // Moltres is on 2F's north strip, and 3F's (2, 0) ladder is the only way onto it.
        Take("VictoryRoad2F, arriving at (1, 1)"),
        Hunt { species: "Moltres", row: "Moltres", ball: "MasterBall",
               way: Way::Legendary(Legend::Moltres), on: "VictoryRoad2F" },
        GoTo("VictoryRoad3F"),
        Repeat("switch at (3, 5)"),
        Repeat("hole at (23, 15)"),
        Take("VictoryRoad2F, arriving at (22, 16)"),
        Repeat("switch at (9, 16)"),
        Take("VictoryRoad3F, arriving at (27, 15)"),
        Take("VictoryRoad2F, arriving at (27, 7)"),
        Explore { maps: ROAD, patience: 1200 },
        GoTo("Route23"),
        // And in at the upper door, which the walk out comes through.
        Take("VictoryRoad2F, arriving at (29, 7)"), GoTo("Route23"),
        GoTo("IndigoPlateau"), GoTo("IndigoPlateauLobby"), Clear(&[]),
        // Out of the lobby and off the plateau, then back: the walk up crosses neither.
        GoTo("IndigoPlateau"), GoTo("Route23"), GoTo("IndigoPlateau"), GoTo("IndigoPlateauLobby"),
    ]);
    steps
}

/// The Elite Four, the Champion, and the ceremony that follows: the four rooms in the one order the
/// cartridge allows, the rival, and the run handed back in Pallet Town once the game has saved.
pub fn to_the_hall_of_fame() -> Vec<Step> {
    use Step::*;
    vec![
        Collect(false),
        // Nothing can be bought or healed once the first door shuts behind the run.
        GoTo("IndigoPlateauLobby"), Talk("Nurse"),
        GoTo("LoreleisRoom"), Talk("Lorelei"),
        GoTo("BrunosRoom"), Talk("Bruno"),
        GoTo("AgathasRoom"), Talk("Agatha"),
        GoTo("LancesRoom"), Talk("Lance"),
        // The door is the last decision: the rival, Oak, the Hall of Fame, the credits and the
        // reset ask for nothing but the button the harness presses, so no turn is ever taken in the
        // Champion's room and arriving there cannot be what ends a step. The save comes back in
        // Pallet Town.
        Take("ChampionsRoom"),
        GoTo("PalletTown"),
    ]
}

/// Cerulean Cave, which the guard opens to a Champion: its three floors, a paced encounter on a
/// cave floor, and Mewtwo at the bottom.
pub fn to_mewtwo() -> Vec<Step> {
    use Step::*;
    const CAVE: &[&str] = &["CeruleanCave1F", "CeruleanCave2F", "CeruleanCaveB1F"];
    const UPPER: &[&str] = &["CeruleanCave1F", "CeruleanCave2F"];
    vec![
        Collect(false),
        // The cave stands on the water of Cerulean's north west pocket, which the city proper
        // cannot reach: the way in is down from Nugget Bridge, and the phase before ended there.
        GoTo("CeruleanCave1F"),
        // A cave floor is one of the two places an encounter has to be paced for.
        Hunt { species: "*", row: "Pace", ball: "MasterBall", way: Way::WildOnACaveFloor, on: "CeruleanCave1F" },
        // The Slowbro the trade over Route 18 wants; the cave's water is the Super Rod's.
        Hunt { species: "Slowbro", row: "Fish", ball: "MasterBall", way: Way::SuperRod, on: "CeruleanCave1F" },
        // The two upper floors first, which is where the way down is: 1F's ladder to B1F stands
        // behind an elevation boundary and is reached by way of 2F. Mewtwo is on the floor below,
        // so an exploring cannot walk up to it here, and one that did would start the battle and
        // hide it for the rest of the game by running.
        Explore { maps: UPPER, patience: 600 },
        GoTo("CeruleanCaveB1F"),
        Hunt { species: "Mewtwo", row: "Mewtwo", ball: "MasterBall", way: Way::Legendary(Legend::Mewtwo),
               on: "CeruleanCaveB1F" },
        Explore { maps: CAVE, patience: 400 },
        GoTo("CeruleanCave1F"), Clear(&[]), GoTo("CeruleanCave2F"), Clear(&[]),
        GoTo("CeruleanCaveB1F"), Clear(&[]),
        // The pocket the way out comes back to is a cul-de-sac: its own shore, Route 4 behind a
        // ledge, and the water up to Nugget Bridge. So the way back into the city proper is over
        // the water and in again by the opening the streets are on.
        GoTo("CeruleanCity"), Tidy,
        GoTo("Route24"), Cross { map: "CeruleanCity", landing: (40, 0) },
    ]
}

/// The eastern routes, which nothing before this has walked: Routes 13, 14 and 15 and their thirty
/// trainers, walked down from Lavender to Fuchsia.
pub fn to_the_eastern_routes() -> Vec<Step> {
    use Step::*;
    // The gates are part of their routes: a route read from the pocket the run stood in looks
    // finished while the half beyond its gate building has never been seen.
    const EAST: &[&str] = &["Route12", "Route12Gate1F", "Route12Gate2F", "Route13", "Route14",
                            "Route15", "Route15Gate1F", "Route15Gate2F"];
    let mut steps = vec![
        Collect(false),
        // Route 10 is two halves with Rock Tunnel between them, and the Power Plant is on the
        // north one: the tunnel is the way down to Lavender.
        GoTo("RockTunnel1F"),
        Explore { maps: &["RockTunnel1F", "RockTunnelB1F"], patience: 400 },
        GoTo("RockTunnel1F"), Take("Route10, arriving at (9, 5"),
        // Walked from the Lavender end, which is the way the ledges run: Route 15's north lane
        // holds a trainer and the TM Rage, and coming up from Fuchsia reaches neither. Route 12's
        // own water is here too, which no phase before this could surf.
        GoTo("LavenderTown"),
        GoTo("Route12"),
        Explore { maps: EAST, patience: 1200 },
        // And back up from the other end, the long way round, because there is no way through: the
        // lanes are ledged apart and each map's openings into the next come out in lanes that
        // cannot reach one another. North to Lavender, west under Saffron, and down the cycling
        // road, which runs one way and that way is south.
        GoTo("Route12"), GoTo("Route12Gate1F"), Take("Route12, arriving at (11, 16)"),
        GoTo("LavenderTown"),
    ];
    steps.extend(lavender_to_saffron());
    // East out of the city and back. Every other leg through Saffron arrives from Route 8 and
    // leaves by another gate, so the road east and the gate's own west door go unwalked.
    steps.extend(saffron_to_lavender());
    steps.extend(lavender_to_saffron());
    steps.extend(saffron_to_celadon());
    steps.extend([
        GoTo("Route16"), GoTo("Route16Gate1F"), Take("Route16, arriving at (17, 1"),
        GoTo("Route17"), GoTo("Route18"), GoTo("Route18Gate1F"), Take("Route18, arriving at (40, "),
        GoTo("FuchsiaCity"), GoTo("Route15"),
        Explore { maps: EAST, patience: 1200 },
        // Route 15's north strip, with a trainer and the TM Rage on it, is entered from Route 14
        // and left by a ledge, so neither exploring ever stands on it — and the Route 14 side of
        // it is a pocket of three tiles behind a tree. Nothing reaches it that does not cut that
        // tree.
        GoTo("Route15"), Cross { map: "Route14", landing: (4, 42) },
        Take("cut down the tree at (4, 42)"),
        Take("Route15"), Clear(&[]),
        Tidy,
    ]);
    steps
}

/// Fuchsia's loose ends: the PC sorted for the errands ahead, Route 15's Venonat, two Nidoran from
/// the Safari Zone, a Safari game played to the end of its steps, and the Good Rod.
pub fn to_the_safari_game() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        Collect(false), Tidy,
        // For the Cinnabar lab's Tangela, in the grass the eastern routes ended beside. The party is
        // still full, so it goes to the box.
        Hunt { species: "Venonat", row: "Grass", ball: "MasterBall", way: Way::WildInGrass, on: "Route15" },
        // The eastern routes ended on Route 15's north strip, which is walled off from the rest of
        // the route: its only way out is the gate, and the doors on the far side of it are the
        // ones that come out facing Fuchsia.
        GoTo("Route15Gate1F"), Take("Route15, arriving at (8, "),
        GoTo("FuchsiaCity"), GoTo("FuchsiaPokecenter"),
        // The trades ahead need party room for two Nidoran, and the bag is full of key items
        // nothing needs again. The party comes in full, and the collecting adds to the box.
        AtPc(Pc::Deposit("MrMime")), AtPc(Pc::Deposit("Flareon")),
    ];
    // A Fish row casts the best rod in the bag, so the Super Rod waits in the PC for the Good Rod.
    steps.push(Field(r#"{"move":"pc_items","op":"deposit","item":"SuperRod"}"#));
    steps.extend([
        GoTo("FuchsiaCity"), GoTo("SafariZoneGate"), GoTo("SafariZoneCenter"),
        // One for the underground trade, and one a Rare Candy makes the Nidorino Route 11 wants.
        Hunt { species: "NidoranMale", row: "Grass", ball: "MasterBall", way: Way::SafariCatch, on: "SafariZoneCenter" },
        Hunt { species: "NidoranMale", row: "Grass", ball: "MasterBall", way: Way::SafariCatch, on: "SafariZoneCenter" },
        Safari,
        GoTo("FuchsiaCity"),
        // Fuchsia's own water is not in reach of a cast, and Route 19's beach is the next map south.
        Hunt { species: "Poliwag", row: "Fish", ball: "MasterBall", way: Way::GoodRod, on: "Route19" },
        Hunt { species: "Tentacool", row: "PaceOnWater", ball: "MasterBall", way: Way::WildWhileSurfing, on: "Route19" },
    ]);
    steps
}

/// The north, walked from the Hall of Fame's own doorstep: the gifts a full bag refused in
/// Viridian and at Route 2's gate, Pikachu in the forest and the stone that makes it the Raichu
/// Cinnabar wants, the Old Amber from the museum's back room, then Mt Moon to Cerulean, its trade,
/// Routes 9 and 10 by water, the Day Care, and Nugget Bridge.
pub fn to_the_north_errands() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        Collect(false), Tidy,
        // The game hands the run back in Pallet Town, and the north is walked from there.
        GoTo("Route1"), GoTo("ViridianCity"),
        // Slot 3 is whatever the collecting caught last, and the party has to have room for the
        // Pikachu: a catch into a full party goes to the box, where a stone cannot reach it.
        GoTo("ViridianPokecenter"), AtPc(Pc::DepositSlot(3)),
    ];
    // A gift with no room for it is refused in silence. What is wanted again is fetched back where
    // it is wanted.
    steps.extend(SPENT.map(Field));
    steps.extend([
        GoTo("ViridianCity"),
        Talk("Fisher"),
        GoTo("ViridianGym"), Talk("Giovanni"), GoTo("ViridianCity"),
        GoTo("Route2"), GoTo("Route2Gate"), Talk("OaksAide"),
        GoTo("Route2"), GoTo("ViridianForestSouthGate"), GoTo("ViridianForest"),
        Hunt { species: "Pikachu", row: "Grass", ball: "MasterBall", way: Way::WildInGrass, on: "ViridianForest" },
        Evolve { item: "ThunderStone", species: "Pikachu" },
        GoTo("ViridianForestNorthGate"), GoTo("Route2"), GoTo("PewterCity"),
        // The scientist with the amber stands in the back room, behind the counter from the front,
        // and its door is behind a tree.
        Take("cut down the tree at (26, 4)"), Take("Museum1F, arriving at (16, 7)"), Talk("Scientist2"), GoTo("PewterCity"),
        // East the long way round rather than back through Mt Moon, whose floors come out on the
        // half of Route 4 that Cerulean cannot be walked back from: Diglett's Cave to Route 11,
        // then up through Vermilion and Saffron.
        // The forest walked from the north end: every pass through it comes up from Route 2's
        // south half, so its south doors and the north gate's own way in go unwalked.
        GoTo("Route2"), GoTo("ViridianForestNorthGate"), GoTo("ViridianForest"),
        GoTo("ViridianForestSouthGate"), GoTo("Route2"),
        GoTo("Route2Gate"), Repeat("Route2, arriving at (16, 36)"),
        GoTo("DiglettsCaveRoute2"), GoTo("DiglettsCave"), GoTo("DiglettsCaveRoute11"),
        GoTo("Route11"), GoTo("VermilionCity"),
    ]);
    steps.extend(vermilion_to_saffron());
    steps.extend(saffron_to_cerulean());
    steps.extend([
        GoTo("CeruleanPokecenter"), AtPc(Pc::Deposit("Raichu")), GoTo("CeruleanCity"),
        // Route 9 opens off the terrace behind the robbed house, and the house is the only way on
        // and off it.
        GoTo("CeruleanTrashedHouse"), Take("CeruleanCity, arriving at (28, 10)"),
        GoTo("Route9"), GoTo("Route10"),
        Hunt { species: "Poliwhirl", row: "Fish", ball: "MasterBall", way: Way::SuperRod, on: "Route10" },
        // The swimmer off Route 10's bank, whom the phases before could only look at.
        Explore { maps: &["Route10"], patience: 400 },
        GoTo("Route9"), GoTo("CeruleanCity"),
        // In by the terrace door and out by the front: the one walk that takes the terrace door in.
        Take("CeruleanTrashedHouse, arriving at (3, 0)"), Take("CeruleanCity, arriving at (28, 12)"),
        GoTo("CeruleanTradeHouse"), Trade("Gambler"), GoTo("CeruleanCity"),
        GoTo("CeruleanPokecenter"), AtPc(Pc::Deposit("Jynx")), GoTo("CeruleanCity"),
        // The tree on the main terrace is the way down to Route 5, into the pocket the Day Care
        // stands in. Route 5's ledges drop only south, so the Day Care comes before the path's
        // pocket below it.
        Take("cut down the tree"), Cross { map: "Route5", landing: (9, 1) },
        GoTo("Daycare"), DayCare(None), GoTo("Route5"),
        GoTo("Route5Gate"), GoTo("Route5"), GoTo("CeruleanCity"),
        GoTo("CeruleanPokecenter"), AtPc(Pc::Deposit("Squirtle")), GoTo("CeruleanCity"),
        // Nugget Bridge and the water off Route 25, then back down into the north west pocket,
        // which is where the cave is and where the phase after starts.
        GoTo("Route24"), GoTo("Route25"), Explore { maps: &["Route25"], patience: 300 },
        GoTo("Route24"), Take("CeruleanCity"),
        // Route 4's ledges drop east, so its last trainer stands where only the Cerulean end
        // reaches her: walking out of Mt Moon lands past her with no way back up.
        GoTo("Route4"), Talk("CooltrainerFemale2"), GoTo("CeruleanCity"),
    ]);
    steps
}

/// The middle of the map, walked back up from Fuchsia: the Slowbro trade over Route 18, the Exp.
/// All on Route 15, the eastern routes north to Lavender, Celadon's TM41 and the Copycat's Poké
/// Doll, the trade under Route 5, Lt. Surge's machine, and Route 11's trade and its aide.
pub fn to_the_middle_errands() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        Collect(false), Tidy,
        GoTo("FuchsiaCity"), GoTo("FuchsiaPokecenter"),
        // The Slowbro the trade over Route 18 wants was caught in Cerulean Cave, into the box the
        // north errands left current. The party comes in full, and slot 3 is the seat the
        // collecting fills, so that is the one that makes room. Lickitung and the Nidorino a candy
        // makes are two more species toward the fifty the Route 15 aide asks for.
        AtPc(Pc::ChangeBox(4)), AtPc(Pc::DepositSlot(3)), AtPc(Pc::Withdraw("Slowbro")),
        GoTo("FuchsiaCity"),
        GoTo("Route18"), GoTo("Route18Gate1F"), GoTo("Route18Gate2F"), Trade("Youngster"),
        // The gate stands on Route 18 with two doors on each side of it, and only the eastern
        // pair comes out on the half that reaches Fuchsia.
        GoTo("Route18Gate1F"), Take("Route18, arriving at (40, "), GoTo("FuchsiaCity"),
        // Level 22 from the Safari Zone, so one candy is the level-up that evolves it.
        Evolve { item: "RareCandy", species: "NidoranMale" },
        // Route 15's gate holds the last aide, who wants fifty species owned.
        GoTo("Route15"), GoTo("Route15Gate1F"), GoTo("Route15Gate2F"), Talk("OaksAide"),
        // North up the eastern routes, which is the only way back: the cycling road goes one way,
        // and its ledge onto Route 18 is a crossing the menu offers and the walk cannot take.
        GoTo("Route15Gate1F"), Take("Route15, arriving at (15, "),
        // Route 13's lanes are ledged apart and the nearest of its three openings from Route 14
        // is the one lane that reaches nothing: the other two are the road to Route 12.
        GoTo("Route14"), Cross { map: "Route13", landing: (1, 9) }, GoTo("Route12"),
        GoTo("Route12Gate1F"), Take("Route12, arriving at (11, 16)"), GoTo("LavenderTown"),
    ];
    steps.extend(lavender_to_saffron());
    steps.extend(saffron_to_celadon());
    steps.extend([
        Talk("Gramps3"),
        GoTo("CeladonMart1F"), GoTo("CeladonMart2F"), GoTo("CeladonMart3F"), GoTo("CeladonMart4F"),
        Talk("Clerk"), Buy(&[("PokeDoll", 1)]),
        GoTo("CeladonMart3F"), GoTo("CeladonMart2F"), GoTo("CeladonMart1F"), GoTo("CeladonCity"),
    ]);
    steps.extend(celadon_to_saffron());
    steps.extend([
        GoTo("CopycatsHouse1F"), GoTo("CopycatsHouse2F"), Talk("Copycat"),
        GoTo("CopycatsHouse1F"), GoTo("SaffronCity"),
        // The trade under Route 5, whose stairwell is north of the gate that splits the route.
        GoTo("Route5"), GoTo("Route5Gate"), Take("Route5, arriving at (10, 30)"),
        GoTo("UndergroundPathRoute5"), Trade("LittleGirl"), GoTo("Route5"),
        GoTo("Route5Gate"), Take("Route5, arriving at (10, 34)"), GoTo("SaffronCity"),
    ]);
    steps.extend(saffron_to_vermilion());
    steps.extend([
        // Lt. Surge hands his machine over at the end of his battle, and again on being talked to
        // if the bag had no room for it then. Which of the two it was is the bag's business.
        GoTo("VermilionGym"), Talk("LtSurge"), GoTo("VermilionCity"),
        GoTo("Route11"), GoTo("Route11Gate1F"), GoTo("Route11Gate2F"), Trade("Youngster"), Talk("OaksAide"),
        // The gate stands in the middle of Route 11 with a door on each side of it, and only the
        // eastern half touches Route 12. Both halves of that border are walked, and the east door
        // is walked back in at, which the trip out of it does not.
        GoTo("Route11Gate1F"), Take("Route11, arriving at (59, 8)"),
        GoTo("Route12"), GoTo("Route11"),
        Take("Route11Gate1F, arriving at (7, 4)"), Take("Route11, arriving at (50, 8)"),
        // Ending in the town rather than on the route, because the route's two halves look alike
        // to the phase that starts here and only the western one opens onto Diglett's Cave.
        GoTo("VermilionCity"),
    ]);
    steps
}

/// Cinnabar's loose ends, which close the tour's loop: the Mansion's Ponyta, the lab's three
/// trades, the Old Amber revived, and the TM Blaine held back for want of bag room.
pub fn to_the_cinnabar_errands() -> Vec<Step> {
    use Step::*;
    vec![
        Collect(false), Tidy,
        // The tour's last leg, and the one that closes its loop: Diglett's Cave to Route 2, down
        // through Viridian to Pallet, and Route 21 to the island, which is the only way to it.
        GoTo("Route11"), GoTo("DiglettsCaveRoute11"), GoTo("DiglettsCave"), GoTo("DiglettsCaveRoute2"),
        GoTo("Route2"), GoTo("Route2Gate"), Repeat("Route2, arriving at (15, 40)"),
        GoTo("ViridianCity"), GoTo("Route1"), GoTo("PalletTown"),
        GoTo("Route21"), GoTo("CinnabarIsland"),
        GoTo("CinnabarPokecenter"),
        // Three slots for the lab's three trades: the Raichu and the Venonat it wants, and the
        // Ponyta out of the mansion. The Venonat went into the box the safari errands left
        // current, which is not the one the middle errands did.
        AtPc(Pc::Deposit("Nidorina")), AtPc(Pc::Deposit("Lickitung")), AtPc(Pc::Deposit("NidoranFemale")),
        AtPc(Pc::Withdraw("Raichu")), AtPc(Pc::ChangeBox(3)), AtPc(Pc::Withdraw("Venonat")),
        // The gym's door is locked to anyone not carrying the key, Blaine beaten or not.
        Field(r#"{"move":"pc_items","op":"withdraw","item":"SecretKey"}"#),
        GoTo("CinnabarIsland"),
        GoTo("PokemonMansion1F"),
        Hunt { species: "Ponyta", row: "Pace", ball: "MasterBall", way: Way::WildOnACaveFloor, on: "PokemonMansion1F" },
        // 3F's scientist stands beyond a block the switch swaps: `ReplaceTileBlock` takes Y before X,
        // so the block is steps (14, 10) to (15, 11), the wall between the floor's middle corridor and
        // him, and it opens with the switch on. The corridor is reached from the (6, 1) staircase,
        // which the switch on shuts at 2F's end, so the switch is 3F's own, pressed once there and
        // once more before going back down.
        GoTo("PokemonMansion2F"), Take("PokemonMansion3F, arriving at (6, 1)"),
        Talk("Statue1"), Talk("Scientist"),
        // 2F's east corner is walled off from the rest of that floor, so the stairs down from 3F's
        // corner are the only way onto it and back up them the only way off. 3F's corner is behind
        // the switch as well, which is why this is done while it is on.
        Take("PokemonMansion2F, arriving at (25, 14)"), Take("PokemonMansion3F, arriving at (25, 14)"),
        Talk("Statue1"),
        Take("PokemonMansion2F, arriving at (6, 1)"), GoTo("PokemonMansion1F"),
        GoTo("CinnabarIsland"),
        GoTo("CinnabarLab"), GoTo("CinnabarLabTradeRoom"), Trade("Gramps"), Trade("Beauty"),
        GoTo("CinnabarLab"), GoTo("CinnabarLabFossilRoom"), Trade("Scientist2"),
        // As with the fossil, the Pokemon is handed over only after a walk out of the room and back.
        Talk("Scientist1"),
        GoTo("CinnabarLab"), GoTo("CinnabarIsland"),
        GoTo("CinnabarLab"), GoTo("CinnabarLabFossilRoom"), Talk("Scientist1"),
        GoTo("CinnabarLab"), GoTo("CinnabarIsland"),
        GoTo("CinnabarGym"), Talk("Blaine"), GoTo("CinnabarIsland"),
        // The machines this phase collected filled the bag again.
        Tidy,
        // The traded Seel arrives at 34, its evolution level, so the next one it gains evolves it.
        KeepFromEvolving("Seel"),
        // North over the water to Pallet Town, where the tour started: the island is reached down
        // Route 21 and nowhere else, so both of that route's edges are crossings of their own.
        GoTo("Route21"), GoTo("PalletTown"),
        // Route 15's aide again, for a run that met fewer than fifty species by the first visit:
        // this phase's catch and trades are the last new ones the tour makes.
        Tidy, Field(r#"{"move":"fly","map":"FuchsiaCity"}"#), GoTo("FuchsiaCity"),
        GoTo("Route15"), GoTo("Route15Gate1F"), GoTo("Route15Gate2F"), Talk("OaksAide"),
    ]
}

/// Every phase, in the order one run plays them.
pub fn all_phases() -> Vec<Vec<Step>> {
    vec![
        to_the_boulder_badge(), to_bill(), to_the_thunder_badge(), to_celadon(), to_the_rainbow_badge(),
        to_the_poke_flute(), to_the_marsh_badge(), to_the_soul_badge(), to_surf(), to_the_volcano_badge(),
        to_seafoam(), to_the_earth_badge(), to_victory_road(), to_the_hall_of_fame(),
        to_the_north_errands(), to_mewtwo(), to_the_power_plant(), to_the_eastern_routes(),
        to_the_safari_game(), to_the_middle_errands(), to_the_cinnabar_errands(),
    ]
}

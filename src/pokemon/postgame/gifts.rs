//! Workstream **G-gifts — fossil revival, gift Pokémon, one-off rooms**. See
//! `docs/postgame-coverage-plan.md` §6-G (sub-steps G1–G4, G7–G8).
//!
//! Splittable from [`super::trades`]: claim the separate §9 row if a second agent takes those.
//!
//! Sub-steps: G1 fossil revival (the agent already carries a Helix Fossil) · G2 Old Amber →
//! Aerodactyl · G3 Lapras on Silph 7F · G4 Fighting Dojo · G7 the skipped Silph floors ·
//! G8a the Saffron TM gifts · G8b the Day Care.
//!
//! # What varies here is the route, not the mechanic
//!
//! Six of the seven legs are `GivePokemon` / `GiveItem` scripts behind an event flag, i.e. the shape
//! B1's Fan Club chairman and C1's fishing gurus already proved: walk to the sprite, press A, let the
//! generic text-advance carry the rest. Their constructors are plain step lists. The work is in the
//! *routes* — three of them turned out to be terrace problems, each failing differently, and all
//! three are written up in §11 of the plan.
//!
//! The exception is the **Day Care** ([`tick`]), which needs a driver because its party menu opens on
//! a stale cursor. That driver is the reusable part of this module: G5/G6's in-game trades and the
//! Name Rater open the same script-driven party menu.
//!
//! ⚠️ A gift mon **is** offered a nickname — and so is a **boxed** one. `_GivePokemon` picks between
//! `AddPartyMon` (which names when `wMonDataLocation` is 0, `engine/pokemon/add_mon.asm:43-52`) and
//! `SendNewMonToBox`, and the latter runs its own `AskName`
//! (`engine/items/item_effects.asm:2731-2733`). A full party is therefore **not** a way past the
//! naming screen, which is what F's §11 entry says; both branches are covered here, at party 6 (G3)
//! and with a slot banked (G4), and the agent's generic handler answers both.

use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::agent::{AgentEvent, AgentState, PokemonAgent};
use crate::pokemon::encoding::GameMode;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::policy::{FieldMove, PolicyStep};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::ram::ROM;
use crate::pokemon::{GameState, PokemonApi, PokemonApiTrait};

/// Silph's lift panel — a bg-event at (3,0) in `SilphCoElevator`
/// (`data/maps/objects/SilphCoElevator.asm:8`), shared by every floor.
const SILPH_ELEVATOR_PANEL: Point8 = Point8 { x: 3, y: 0 };

/// The Silph floors G7 visits, as `(lift menu index, item balls, somewhere to walk)`. Menu index is
/// floor − 1 (`SilphCoElevatorFloors` lists all eleven in order). 2F and 8F carry no items and are
/// here for map coverage — they are two of the 96 maps §2 lists as never referenced by `policy.rs`.
///
/// Every ball listed is reachable from its own floor's lift landing; that is measured, not assumed
/// (`probe_silph_item_floors`), because 7F is *not* one room and cost G3 a run.
///
/// ⚠️ The **walk** column is load-bearing on the two floors with no items, not decoration. A
/// `UseElevator` issued while the player is still standing on (or beside) the lift tile they arrived
/// by silently rides them *back where they came from*, and then pops as complete because the map
/// changed. Collecting an item walks away from the door as a side effect; on a floor with nothing to
/// collect, something else has to. See the §11 entry — the real fix belongs in the shared executor.
const SILPH_FLOORS: &[SilphFloor] = &[
    SilphFloor { lift: 1, items: &[], walk_to: Some(MapSprite::SILPHCO2F_SILPH_WORKER_F) },
    SilphFloor { lift: 3, walk_to: None, items: &[
        MapSprite::SILPHCO4F_FULL_HEAL, MapSprite::SILPHCO4F_MAX_REVIVE, MapSprite::SILPHCO4F_ESCAPE_ROPE] },
    SilphFloor { lift: 5, walk_to: None, items: &[
        MapSprite::SILPHCO6F_HP_UP, MapSprite::SILPHCO6F_X_ACCURACY] },
    // 7F's *lift* side, which G3's route into the rival pocket could not reach.
    SilphFloor { lift: 6, walk_to: None, items: &[
        MapSprite::SILPHCO7F_CALCIUM, MapSprite::SILPHCO7F_TM_SWORDS_DANCE] },
    SilphFloor { lift: 7, items: &[], walk_to: Some(MapSprite::SILPHCO8F_SILPH_WORKER_M) },
    SilphFloor { lift: 9, walk_to: None, items: &[
        MapSprite::SILPHCO10F_TM_EARTHQUAKE, MapSprite::SILPHCO10F_RARE_CANDY, MapSprite::SILPHCO10F_CARBOS] },
];

struct SilphFloor {
    /// Lift menu index (floor − 1).
    lift: u8,
    /// Item balls to collect, in the order the walk is cheapest.
    items: &'static [MapSprite],
    /// An NPC to go and talk to when `items` is empty — see the warning above.
    walk_to: Option<MapSprite>,
}

impl PolicyStep {
    /// **G1** — hand the **Helix Fossil** to the Cinnabar Lab and come back for the **Omanyte**.
    ///
    /// This is a **two-visit** mechanic, which is the only thing about it that is not obvious:
    /// `CinnabarLabFossilRoomScientist1Text` takes the fossil and sets `EVENT_GAVE_FOSSIL_TO_LAB` +
    /// `EVENT_LAB_STILL_REVIVING_FOSSIL`, and on a second conversation prints "go for a walk" until
    /// the *reviving* flag clears. Nothing counts steps — the flag is cleared by
    /// `CinnabarIsland_Script` itself (`scripts/CinnabarIsland.asm:6`), which runs on every load of
    /// the island. So "a walk" is precisely **out of the lab and back**, and the trip is four warps.
    ///
    /// The fossil-choice menu is a bespoke `TextBoxBorder` + `HandleMenuInput` list of *the fossils in
    /// the bag* (`engine/events/cinnabar_lab.asm:1-70`), and this save carries exactly one, so the
    /// cursor already sits on the Helix Fossil and the A-mash selects it; the YES/NO that follows
    /// opens on YES, like every other giver in this workstream. Three `Interact`s per visit for the
    /// same reason `coin_case_steps` uses three — only the first A press is guaranteed to land on the
    /// script.
    ///
    /// Ends outdoors so the next leg's `Fly` is not refused for being indoors.
    pub fn fossil_revival_steps() -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::CinnabarIsland }];
        s.extend(Self::into_fossil_room());
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::CINNABARLABFOSSILROOM_SCIENTIST1), 3));
        // The walk: out to the island (which clears EVENT_LAB_STILL_REVIVING_FOSSIL) and back in.
        s.extend(Self::out_of_fossil_room());
        s.extend(Self::into_fossil_room());
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::CINNABARLABFOSSILROOM_SCIENTIST1), 3));
        s.extend(Self::out_of_fossil_room());
        s
    }

    /// **G2** — the **Old Amber** from the Pewter Museum, revived into an **Aerodactyl**.
    ///
    /// Two halves, and the first is the awkward one. The amber is *not* the `SPRITE_OLD_AMBER` object
    /// standing next to it — that sprite is scenery with a text pointer and no item id
    /// (`data/maps/objects/Museum1F.asm:25`); the giver is **`MUSEUM1F_SCIENTIST2`** at (15,2), whose
    /// script is a plain `GiveItem OLD_AMBER` behind `EVENT_GOT_OLD_AMBER`
    /// (`scripts/Museum1F.asm:190-205`). Both are in the museum's *back* room, which is why this
    /// enters through Pewter's second museum door at (19,5) — `enter_at(Museum1F, 16, 7)`, since
    /// Pewter has two warps to the same map and the front one lands at (10,7) on the ticket side.
    ///
    /// The second half is G1's mechanic again with a different fossil, so it reuses the same walk.
    pub fn old_amber_steps() -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::PewterCity },
            // Venusaur leads, because `CuttingTree` only ever asks party slot 0.
            Self::MovePokemonToFront { slot: 0 },
            Self::CutTree { map: Map::PewterCity },
            Self::enter_at(Map::Museum1F, 16, 7),
        ];
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::MUSEUM1F_SCIENTIST2), 3));
        s.push(Self::enter(Map::PewterCity));
        s.push(Self::Fly { to: Map::CinnabarIsland });
        s.extend(Self::into_fossil_room());
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::CINNABARLABFOSSILROOM_SCIENTIST1), 3));
        s.extend(Self::out_of_fossil_room());
        s.extend(Self::into_fossil_room());
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::CINNABARLABFOSSILROOM_SCIENTIST1), 3));
        s.extend(Self::out_of_fossil_room());
        s
    }

    /// **G3** — the **Lapras** the rescued Silph employee has been holding since the building was
    /// liberated.
    ///
    /// `SilphCo7FSilphWorkerM1Text` hands it over at level 15 behind `BIT_GOT_LAPRAS`
    /// (`scripts/SilphCo7F.asm:294-320`); the agent walked past him on the way to Giovanni and never
    /// spoke to him.
    ///
    /// ⚠️ **The lift does not reach him.** 7F has its own elevator door at (18,0) and
    /// `SilphCoElevatorFloors` lists all eleven floors, so riding to menu index 6 is one step — and
    /// lands in the *wrong pocket*. Measured by [`probe_silph_7f_pockets`]: from (18,0) the reachable
    /// set is workers 2/3/4, the Calcium, the TM and three warps; worker **M1 is not in it**. He is in
    /// the walled rival pocket with the 3F and 11F pads, so the route is the teleport-pad chain
    /// `silph_giovanni_steps` already threads — lift to 3F, then 3F's (11,11) pad — and the way out is
    /// the same pad back.
    ///
    /// [`probe_silph_7f_pockets`]: crate::pokemon::integration_tests::postgame::gifts
    pub fn lapras_steps() -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::SaffronCity },
            Self::enter(Map::SilphCo1F),
            Self::enter(Map::SilphCoElevator),
            Self::UseElevator { panel: SILPH_ELEVATOR_PANEL, floor: 2 }, // 3F = menu index 2
            Self::EnterMap { to_map: Map::SilphCo7F, to_position: Some(Point8 { x: 5, y: 3 }) },
        ];
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::SILPHCO7F_SILPH_WORKER_M1), 3));
        s.push(Self::EnterMap { to_map: Map::SilphCo3F, to_position: Some(Point8 { x: 11, y: 11 }) });
        s.extend(Self::out_of_silph());
        s
    }

    /// **G4** — beat the Saffron **Karate Master** and take a **Hitmonlee**.
    ///
    /// The dojo is a five-trainer room the agent has never opened, and the prize is one of two
    /// Poké Balls at the back: taking either hides the other (`SetEvents EVENT_GOT_HITMONLEE,
    /// EVENT_DEFEATED_FIGHTING_DOJO`, `scripts/FightingDojo.asm:226-255`), so this is a **one-shot
    /// choice** and Hitmonlee is spent on it. `CollectItem` is the right step even though the ball is
    /// a `GivePokemon` rather than an item: the sprite is `HideObject`ed on success, which is exactly
    /// the completion condition it waits for.
    ///
    /// The master is engaged through `Interact` rather than the coordinate trigger at (4,3) — his
    /// text script does the whole `EngageMapTrainer` + `InitBattleEnemyParameters` dance itself
    /// (`FightingDojoKarateMasterText`), so talking to him is a battle from any side. Repeated
    /// `InteractIfReachable` for the same reason `silph_giovanni_steps` uses it: once he is beaten the
    /// step needs to pop rather than wait forever, and the four blackbelts will interrupt the walk
    /// with their own line-of-sight battles.
    ///
    /// A party slot is freed first, deliberately: G3 covered the boxed branch, so this one lands in
    /// the party and covers `AddPartyMon`. Healing is not optional — the party has not seen a nurse
    /// since B, and Venusaur's Solarbeam is at 0 PP.
    pub fn hitmonlee_steps(bank_slot: u8) -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::SaffronCity },
            Self::enter(Map::SaffronPokecenter),
            Self::Interact(MapSprite::SAFFRONPOKECENTER_NURSE),
            Self::deposit_pokemon(bank_slot, Map::SaffronPokecenter),
            Self::enter(Map::SaffronCity),
            Self::enter(Map::FightingDojo),
        ];
        s.extend(std::iter::repeat_n(Self::InteractIfReachable(MapSprite::FIGHTINGDOJO_KARATE_MASTER), 8));
        s.push(Self::CollectItem(MapSprite::FIGHTINGDOJO_HITMONLEE_POKE_BALL));
        s.push(Self::enter(Map::SaffronCity));
        s
    }

    /// **G7** — the five Silph floors `complete_game_steps` never opened, plus the two items on 7F it
    /// walked past.
    ///
    /// `complete_game_steps` rides 1F → 5F → 9F → 3F → 7F → 11F on its way to Giovanni, so **2F, 4F,
    /// 6F, 8F and 10F** are among the 96 maps §2 lists as never referenced. Eight item balls sit on
    /// them (4F: Full Heal, Max Revive, Escape Rope · 6F: HP Up, X Accuracy · 10F: TM26 Earthquake,
    /// Rare Candy, Carbos; 2F and 8F carry none) and two more on 7F's *lift* side — the Calcium and
    /// TM03 Swords Dance that G3's pocket probe found unreachable from the rival room.
    ///
    /// Every one of those balls is reachable from its own floor's lift landing — measured, because 7F
    /// had already proved a Silph floor need not be one room. So the whole leg is lift rides, and the
    /// only real constraint is the **bag**: ten pickups against five free slots. Phase 0's item PC is
    /// what makes it fit, and `bank` is deliberately six dead entries — the S.S. Ticket, Lift Key and
    /// Silph Scope are spent key items, which the PC takes happily (`IsKeyItem` in
    /// `engine/menus/players_pc.asm:164` only suppresses the quantity prompt, it does not refuse).
    ///
    /// ⚠️ `bank` carries a **quantity**, and it has to be the whole stack. A bag *slot* is freed only
    /// when the last unit leaves it, so `deposit_item(GreatBall, 1)` on a stack of nine frees nothing
    /// — the leg then walks the whole building and wedges on "No more room for items!" at the last
    /// ball. See §11.
    pub fn silph_floors_steps(bank: &[(ItemId, u8)]) -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::SaffronCity },
            Self::enter(Map::SaffronPokecenter),
            Self::Interact(MapSprite::SAFFRONPOKECENTER_NURSE),
        ];
        s.extend(bank.iter().map(|&(item, qty)| Self::deposit_item(item, qty, Map::SaffronPokecenter)));
        s.extend([Self::enter(Map::SaffronCity), Self::enter(Map::SilphCo1F)]);
        // Floor order is the lift's own order, so each ride is one stop where it can be.
        for floor in SILPH_FLOORS {
            s.push(Self::enter(Map::SilphCoElevator));
            s.push(Self::UseElevator { panel: SILPH_ELEVATOR_PANEL, floor: floor.lift });
            s.extend(floor.items.iter().map(|&ball| Self::CollectItem(ball)));
            s.extend(floor.walk_to.map(Self::Interact));
        }
        s.extend(Self::out_of_silph());
        s
    }

    /// **G8a** — the two TM gifts in the one-off Saffron houses, and the purchase one of them needs.
    ///
    /// `MrPsychicsHouse` is a plain `GiveItem TM_PSYCHIC` behind `EVENT_GOT_TM29`. `CopycatsHouse2F`
    /// is the more interesting one and the only *conditional* gift in this workstream: the Copycat
    /// checks `IsItemInBag POKE_DOLL`, and with no doll her script simply falls through to
    /// `TextScriptEnd` — one indistinguishable text box, no refusal message
    /// (`scripts/CopycatsHouse2F.asm:14-40`). So the leg must **buy the doll first**, ¥1000 from the
    /// `CeladonMart4F` clerk, and the TM31 assertion is what proves the doll was actually held rather
    /// than the conversation merely happening.
    ///
    /// `bank` frees the bag first for the same reason G7's does — 19/20 with three items still to
    /// arrive. Bank quantities may overshoot; `ItemPcState::new` clamps them.
    pub fn saffron_tm_gifts_steps(bank: &[(ItemId, u8)]) -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::SaffronCity },
            Self::enter(Map::SaffronPokecenter),
        ];
        s.extend(bank.iter().map(|&(item, qty)| Self::deposit_item(item, qty, Map::SaffronPokecenter)));
        s.extend([
            Self::enter(Map::SaffronCity),
            // The Poké Doll, four floors up the department store. `BuyFromMart` owns the clerk.
            Self::Fly { to: Map::CeladonCity },
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonMart2F),
            Self::enter(Map::CeladonMart3F),
            Self::enter(Map::CeladonMart4F),
            Self::BuyFromMart { item: crate::pokemon::BagItem::new(ItemId::PokeDoll, 1), map: Map::CeladonMart4F },
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonCity),
            Self::Fly { to: Map::SaffronCity },
            Self::enter(Map::MrPsychicsHouse),
        ]);
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::MRPSYCHICSHOUSE_MR_PSYCHIC), 3));
        s.extend([
            Self::enter(Map::SaffronCity),
            Self::enter(Map::CopycatsHouse1F),
            Self::enter(Map::CopycatsHouse2F),
        ]);
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::COPYCATSHOUSE2F_COPYCAT), 3));
        s.extend([Self::enter(Map::CopycatsHouse1F), Self::enter(Map::SaffronCity)]);
        s
    }

    /// **G8b** — the **Day Care** on Route 5: leave a Pokémon, collect it, pay the bill.
    ///
    /// The only genuinely new *mechanic* left in G8, and the only one in this workstream that opens a
    /// party menu. Two things make it work without a driver:
    ///
    /// 1. **The party menu needs driving, so this is a `PartyScript` step and not an `Interact`.**
    ///    `DaycareGentlemanText` calls `DisplayPartyMenu` without resetting `wCurrentMenuItem`, so it
    ///    opens on whatever the last party menu left behind and an A-mash hands over an arbitrary mon
    ///    — which is refused, forever, if it carries an HM (`scripts/Daycare.asm:38-40`, and every
    ///    other member of this party carries Cut, Fly or Strength). `MovePokemonToFront` still runs
    ///    first so the driver's target is simply slot 0: `hm_free_slot` is the mon to send.
    /// 2. **Exactly one visit each way.** A second conversation would take the *other* branch —
    ///    `wDayCareInUse` is already set, so it would collect the mon straight back and charge for
    ///    the privilege. The driver completes on the party count changing, so it makes exactly one.
    ///
    /// Route 5 is the other half of the work; see the landing comment below.
    ///
    /// Nothing puts the old lead back afterwards, and nothing needs to: handing over slot 0 promotes
    /// the mon behind it, so the Cut holder is leading again by the time the gentleman finishes, and
    /// the collected mon is appended at the end.
    ///
    /// The bill is ¥100 × (levels grown + 1), so a same-visit round trip costs ¥100 — the mon gains
    /// one exp point per step walked and nothing at level 30 grows in a corridor. Paying at all is
    /// the observable; growing is a step counter, not a mechanism.
    pub fn daycare_steps(hm_free_slot: u8) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeruleanCity },
            // Cerulean is cut in two and Fly lands on the wrong half; the trashed house is the bridge
            // to the Route 5 terrace, exactly as `cerulean_to_lavender_steps` does it.
            Self::enter(Map::CeruleanTrashedHouse),
            Self::enter_at(Map::CeruleanCity, 27, 9),
            // ⚠️ **Not** a plain `enter(Route5)`. Route 5 is three parallel north-south corridors
            // walled from each other, joined only at its southern end, and every rung between them is
            // a one-way ledge. The Day Care sits in the **middle** corridor and its door is
            // unreachable from the corridor the nearest crossing lands in — `actions()` from (18,1)
            // lists the gate, the Underground Path and Cerulean, and no Day Care at all. The whole
            // top row is connection tiles, so the corridor is chosen by *which one you ask for*.
            Self::enter_at(Map::Route5, 10, 0),
            Self::enter(Map::Daycare),
            Self::MovePokemonToFront { slot: hm_free_slot },
            Self::PartyScript { script: PartyScript::Daycare, slot: 0 }, // deposit the lead
            Self::enter(Map::Route5),
            Self::enter(Map::Daycare),
            Self::PartyScript { script: PartyScript::Daycare, slot: 0 }, // collect, and pay
            Self::enter(Map::Route5),
        ]
    }

    /// **G8c** — rename a party mon at the Lavender **Name Rater**, then the four rooms that are
    /// nothing but text.
    ///
    /// The Name Rater is the Day Care's mechanic with a different ending: same stale-cursor party
    /// menu, so the same [`PartyScript`] driver, but it finishes on the chosen slot's **nickname**
    /// changing rather than on the party count. ⚠️ It only renames a mon whose OT name *and* ID match
    /// the player's, so a traded mon gets "a truly impeccable name!" and nothing happens — which is
    /// indistinguishable from success without the nickname check.
    ///
    /// The four rooms after it are pure map coverage: `ViridianSchoolHouse`, `CeladonHotel` and
    /// `CeladonChiefHouse` are on §2's never-visited list (`CeladonDiner` is not — F opens it for the
    /// Coin Case). Each is one `Interact` with its resident, which is the cheapest thing that proves
    /// the room was actually entered rather than merely routed past.
    pub fn name_rater_and_rooms_steps(rename_slot: u8) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::LavenderTown },
            Self::enter(Map::NameRatersHouse),
            Self::PartyScript { script: PartyScript::NameRater, slot: rename_slot },
            Self::enter(Map::LavenderTown),
            Self::Fly { to: Map::ViridianCity },
            Self::enter(Map::ViridianSchoolHouse),
            Self::Interact(MapSprite::VIRIDIANSCHOOLHOUSE_BRUNETTE_GIRL),
            Self::enter(Map::ViridianCity),
            Self::Fly { to: Map::CeladonCity },
            Self::enter(Map::CeladonHotel),
            Self::Interact(MapSprite::CELADONHOTEL_GRANNY),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::CeladonChiefHouse),
            Self::Interact(MapSprite::CELADONCHIEFHOUSE_CHIEF),
            Self::enter(Map::CeladonCity),
        ]
    }

    /// Ride back down to 1F (menu index 0) and step outside, so the next leg's `Fly` is allowed.
    fn out_of_silph() -> Vec<Self> {
        vec![
            Self::enter(Map::SilphCoElevator),
            Self::UseElevator { panel: SILPH_ELEVATOR_PANEL, floor: 0 },
            Self::enter(Map::SaffronCity),
        ]
    }

    /// Cinnabar Island → `CinnabarLab` → its testing room. Two warps, both plain doors
    /// (`data/maps/objects/CinnabarIsland.asm:11`, `CinnabarLab.asm:9-13`).
    fn into_fossil_room() -> Vec<Self> {
        vec![Self::enter(Map::CinnabarLab), Self::enter(Map::CinnabarLabFossilRoom)]
    }

    /// The reverse, ending on the island — which is both the "walk" the scientist asks for and the
    /// outdoor tile the next `Fly` needs.
    fn out_of_fossil_room() -> Vec<Self> {
        vec![Self::enter(Map::CinnabarLab), Self::enter(Map::CinnabarIsland)]
    }
}

// ── The Day Care driver (G8b) ───────────────────────────────────────────────────────────────────

/// A wedged conversation reports itself instead of pulsing A for the whole test budget. Generous
/// because a visit runs a party menu, a cry and four text boxes.
const TICK_BUDGET: u32 = 1200;

/// A one-NPC script that opens the **party menu** and needs the cursor driven to a chosen slot.
///
/// Both members share a driver because they share the only hard part. Neither script resets
/// `wCurrentMenuItem` before calling `DisplayPartyMenu`, so the list opens on whatever the previous
/// party menu left behind and an A-mash picks an arbitrary mon — which the Day Care *refuses* if it
/// carries an HM, and which the Name Rater silently renames instead of the one you meant.
///
/// ➡️ In-game trades (§6-G5/G6) are the third of these: `DoInGameTradeDialogue` opens the same menu.
/// Adding them should be a variant here plus a completion test, not a new driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartyScript {
    /// The Route 5 Day Care gentleman. One conversation, two branches — `wDayCareInUse` picks
    /// between "leave the mon in `slot`" and "collect whoever is here, and pay" — so one variant
    /// covers both and the caller never says which it wants.
    Daycare,
    /// The Lavender Name Rater. ⚠️ Only renames a mon whose OT name **and** ID match the player's
    /// (`NameRatersHouseCheckMonOTScript`), so a traded mon quietly gets "a truly impeccable name!"
    /// and nothing else.
    NameRater,
    /// One of the nine in-game trades (§6-G's table), identified by where it is rather than by its
    /// `TRADE_FOR_*` constant — the driver never needs `wWhichTrade`, only the NPC and what to hand
    /// over. `InGameTrade_DoTrade` checks the selected mon's species against the trade's
    /// `wInGameTradeGiveMonSpecies` and bails with "wrong mon" otherwise
    /// (`engine/events/in_game_trades.asm`), so picking the right slot is the whole job again.
    Trade { at: Map, npc: MapSprite, give: PokemonSpecies },
}

impl PartyScript {
    /// The map the NPC is on. The caller routes there; this driver only owns the last few tiles.
    pub const fn map(self) -> Map {
        match self {
            Self::Daycare => Map::Daycare,
            Self::NameRater => Map::NameRatersHouse,
            Self::Trade { at, .. } => at,
        }
    }

    const fn sprite(self) -> MapSprite {
        match self {
            Self::Daycare => MapSprite::DAYCARE_GENTLEMAN,
            Self::NameRater => MapSprite::NAMERATERSHOUSE_NAME_RATER,
            Self::Trade { npc, .. } => npc,
        }
    }
}

/// Length of a `wPartyMonNicks` entry (`NAME_LENGTH`, `pokered/macros/ram.asm`).
const NAME_LENGTH: usize = 11;

/// What "done" looks like, captured before the conversation starts.
///
/// Each script is measured on the thing it actually changes, and neither is visible in the queue —
/// the step pops the moment the driver takes over, exactly like `UseItemPc` and `UsePcBox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Baseline {
    /// Day Care: the party shrinks by one on a deposit and grows by one on a collection, so *any*
    /// change is completion and one test serves both branches.
    PartyCount(u8),
    /// Name Rater: the chosen slot's nickname bytes. `DeterministicPolicy` draws names without
    /// replacement, so a rename cannot land on the name it started with.
    Nickname([u8; NAME_LENGTH]),
    /// Trade: the give-species leaving the party. **Not** the species at `slot` — `InGameTrade_DoTrade`
    /// does `RemovePokemon` then `AddPartyMon`, so the received mon is *appended* and every slot after
    /// the one traded away shifts down. Watching the species disappear is index-independent.
    SpeciesGone(PokemonSpecies),
}

/// Live state of a party-menu script. Carried in [`AgentState::UsingPartyScript`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartyScriptState {
    pub script: PartyScript,
    /// Party slot to act on — the mon to hand over, or the one to rename. Ignored when collecting
    /// from the Day Care: the gentleman gives back what he has.
    pub slot: u8,
    /// Where to stand and which way to face to talk to the gentleman, resolved from `actions()`.
    pub stand: Point8,
    pub facing: PlayerFacingDirection,
    baseline: Baseline,
    /// Press/release alternation, so every input is a fresh rising edge.
    pub press: bool,
    pub entered_menu: bool,
    pub ticks: u32,
}

impl PartyScriptState {
    pub fn new(script: PartyScript, slot: u8, npc: (Point8, PlayerFacingDirection), api: &PokemonApi<'_>) -> Self {
        Self {
            script,
            slot,
            stand: npc.0,
            facing: npc.1,
            baseline: match script {
                PartyScript::Daycare => Baseline::PartyCount(party_count(api)),
                PartyScript::NameRater => Baseline::Nickname(nickname(api, slot)),
                PartyScript::Trade { give, .. } => Baseline::SpeciesGone(give),
            },
            press: true,
            entered_menu: false,
            ticks: 0,
        }
    }

    /// Has the script done its work? Read live and compared against [`Baseline`].
    fn done(&self, api: &PokemonApi<'_>) -> bool {
        match self.baseline {
            Baseline::PartyCount(before) => party_count(api) != before,
            Baseline::Nickname(before) => nickname(api, self.slot) != before,
            Baseline::SpeciesGone(give) => !party_holds(api, give),
        }
    }

    fn describe(&self, api: &PokemonApi<'_>) -> String {
        match self.baseline {
            Baseline::PartyCount(before) => format!("party {before} → {}", party_count(api)),
            Baseline::Nickname(_) => format!("slot {} renamed", self.slot),
            Baseline::SpeciesGone(give) => format!("traded away {give:?}"),
        }
    }
}

/// Is `species` still in the party? Read from `wPartySpecies`, the `$ff`-terminated list at the head
/// of the party struct, so this costs no `GameState` build.
fn party_holds(api: &PokemonApi<'_>, species: PokemonSpecies) -> bool {
    let base = pokered_symbols::wPartySpecies.address;
    (0..party_count(api)).any(|i| api.mmu().read(base + i as u16) == species as u8)
}

fn party_count(api: &PokemonApi<'_>) -> u8 {
    api.mmu().read_pointer(&pokered_symbols::wPartyCount)
}

/// The raw nickname bytes of party `slot` — compared, never decoded, so no charmap is involved.
fn nickname(api: &PokemonApi<'_>, slot: u8) -> [u8; NAME_LENGTH] {
    let base = pokered_symbols::wPartyMonNicks.address + slot as u16 * NAME_LENGTH as u16;
    std::array::from_fn(|i| api.mmu().read(base + i as u16))
}

/// Resolve the walk to the script's NPC, the way F's `pick_sale` resolves a mart clerk.
///
/// `route_to_face_dir` is not used to *find* the standing tile because it and `actions()` do not model
/// the same map (F's §11 entry) — `actions()` is the one that knows about counters, and both of these
/// NPCs stand behind one.
pub fn pick(state: &GameState, script: PartyScript, slot: u8) -> Option<FieldMove> {
    // A trade knows *what* it wants, so it finds its own slot rather than trusting the caller's.
    // That matters beyond tidiness: the alternative is `MovePokemonToFront`, and promoting the mon to
    // be traded demotes the Cut holder, which breaks the very next leg that meets a tree.
    let slot = match script {
        PartyScript::Trade { give, .. } => match state.pokemon.iter().position(|p| p.species == give) {
            Some(i) => i as u8,
            None => {
                println!("[policy] {script:?}: no {give:?} in the party to trade");
                return None;
            }
        },
        _ => slot,
    };
    let npc = state.map.sprites.iter()
        .find(|s| !s.hidden && s.name == script.sprite().name)?;
    let action = state.map.actions().into_iter()
        .find(|a| a.tile == MetaTile::Sprite(npc.name))?;

    let stand = action.destination;
    let (dx, dy) = (npc.position.x as i16 - stand.x as i16, npc.position.y as i16 - stand.y as i16);
    let facing = match (dx.signum(), dy.signum()) {
        (0, -1) => PlayerFacingDirection::Up,
        (0, 1) => PlayerFacingDirection::Down,
        (-1, 0) => PlayerFacingDirection::Left,
        (1, 0) => PlayerFacingDirection::Right,
        _ => {
            println!("[policy] {script:?}: NPC at {} is not aligned with {stand}", npc.position);
            return None;
        }
    };
    let face_tile = match facing {
        PlayerFacingDirection::Up => Point8 { x: stand.x, y: stand.y.saturating_sub(1) },
        PlayerFacingDirection::Down => Point8 { x: stand.x, y: stand.y + 1 },
        PlayerFacingDirection::Left => Point8 { x: stand.x.saturating_sub(1), y: stand.y },
        PlayerFacingDirection::Right => Point8 { x: stand.x + 1, y: stand.y },
    };
    Some(FieldMove::UsePartyScript { script, slot, npc: (face_tile, facing) })
}

/// One agent tick of a [`PartyScript`] conversation. Called from `agent.rs` via a delegating arm.
///
/// The whole reason these need a driver rather than an `Interact` is one missing instruction in
/// pokered: both scripts call `DisplayPartyMenu` **without resetting `wCurrentMenuItem`**
/// (`scripts/Daycare.asm:26-32`, `scripts/NameRatersHouse.asm:53-57`), so the list opens wherever the
/// last party menu left its cursor. An A-mash therefore acts on an arbitrary party member — which the
/// Day Care gentleman *refuses* if it carries an HM ("I can't accept a POKéMON that knows an HM
/// move"), forever, and which the Name Rater cheerfully renames instead of the mon you meant.
/// Navigating the cursor to a chosen slot is the entire job.
pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: PartyScriptState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);

    let abort = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, why: String| {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("party-script: {why}") });
        agent.set_state(AgentState::Idle);
    };

    // ── Done: whatever this script was supposed to change has changed ───────────────────────────
    if s.entered_menu && s.done(api) {
        if game_mode != GameMode::Overworld {
            api.release_all_buttons();
            if s.press { api.press_button(JoypadButton::A); } // clear the closing text
            agent.set_state(AgentState::UsingPartyScript(PartyScriptState { press: !s.press, ticks: s.ticks + 1, ..s }));
            return Ok(());
        }
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("{:?}: {}", s.script, s.describe(api)) });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    if s.ticks > TICK_BUDGET {
        abort(agent, api, format!("nothing changed in {TICK_BUDGET} ticks"));
        return Ok(());
    }

    // ── Still outside: walk up to the gentleman and press A ──────────────────────────────────────
    if game_mode == GameMode::Overworld && !s.entered_menu {
        let gs = agent.observe_state(api)?;
        match gs.map.route_to_face_dir(s.stand, Some(s.facing)).as_deref() {
            Some([]) => {
                api.release_all_buttons();
                if s.press { api.press_button(JoypadButton::A); }
                agent.set_state(AgentState::UsingPartyScript(PartyScriptState { press: !s.press, ticks: s.ticks + 1, ..s }));
            }
            Some(&[btn, ..]) => {
                api.release_all_buttons();
                api.press_button(btn);
                agent.set_state(AgentState::UsingPartyScript(PartyScriptState { press: true, ticks: s.ticks + 1, ..s }));
            }
            _ => abort(agent, api, format!("can't reach the NPC at {}", s.stand)),
        }
        return Ok(());
    }

    // ── In the conversation ─────────────────────────────────────────────────────────────────────
    let s = PartyScriptState { entered_menu: true, ticks: s.ticks + 1, ..s };
    if !s.press {
        api.release_all_buttons();
        agent.set_state(AgentState::UsingPartyScript(PartyScriptState { press: true, ..s }));
        return Ok(());
    }

    let (top_x, top_y, cursor, _) = api.menu_geometry();
    // The party list's box origin, the same signal `agent::field_move_menu_button` keys on. Every
    // other screen in both conversations — two YES/NOs, the money box, the closing text — opens on
    // entry 0, so `_ => A` covers all of them and there are exactly two cases.
    let button = if top_x == 0 && (top_y == 1 || top_y == 3) {
        if cursor < s.slot { JoypadButton::Down }
        else if cursor > s.slot { JoypadButton::Up }
        else { JoypadButton::A }
    } else {
        JoypadButton::A
    };

    api.release_all_buttons();
    api.press_button(button);
    agent.set_state(AgentState::UsingPartyScript(PartyScriptState { press: false, ..s }));
    Ok(())
}

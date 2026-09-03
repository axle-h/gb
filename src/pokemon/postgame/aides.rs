//! Workstream **H — Oak's aides**. See `docs/postgame-coverage-plan.md` §6-H. Depends on A–G for dex
//! count.
//!
//! Three items gated on **dex owned**: HM05 Flash at 10 (Route 2 Gate), the Itemfinder at 30
//! (`Route11Gate2F`), Exp.All at 50 (`Route15Gate2F`). Check the gate with `probe_coverage` before
//! travelling — don't guess.
//!
//! Sub-steps: H1 Flash · H2 teach Flash and prove it · H3 Itemfinder · H5 Exp.All, then
//! `postgame-aides.bin`.
//!
//! # H4 is gone, and so is hidden-item collection (2026-09-03)
//!
//! H4 collected Route 11's hidden Escape Rope, and this module read the whole hidden-item table out
//! of the ROM to do it. All of that is removed. The reason is not that it did not work — it worked,
//! and `hidden_items_match_the_rom_coord_table` pinned it against `HiddenItemCoords` — but that the
//! only *other* caller of the mechanic was the `use_field_move interact` tool, which let a model
//! press A at any square it liked. A deployed run spent 85 minutes doing that across three Silph Co
//! floors, having decided it was a way to walk. Nothing in the game is gated behind a hidden item:
//! all 212 hidden events yield loot, and not one yields a key item, an HM or a TM. So the tool went,
//! and the rest went with it rather than being left as a step nothing builds.
//!
//! The Itemfinder (H3) stays. It is a bag item with a text-box effect and `press_the_itemfinder_steps`
//! proves it works by standing where it answers *yes* — which needs a hidden item to *exist*, not to
//! be collectable.

use crate::geometry::Point8;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::policy::{FieldMove, PartyRef, PolicyStep};
use crate::pokemon::roms;
use crate::pokemon::symbols::{pokered_symbols, DmgBank, DmgPointer};
use crate::pokemon::GameState;

// ── H5: the dex sweep ────────────────────────────────────────────────────────────────────────────
//
// Exp.All's gate is **50 species owned**, and §3 ruled an exhaustive dex sweep out of scope — so this
// is the smallest sweep that clears the gate, not a completionist one. What makes it small is picking
// grounds off [`crate::pokemon::wild`] rather than off a walkthrough: the difference between hunting a
// species at 40 % and at 1.2 % is the difference between a leg and an afternoon.

/// The species `on_map`'s **grass** block can produce whose encounter share is at least `min_share`
/// percent — the ones worth standing there for.
///
/// Grass only. The water block needs Surf, and a sweep that mounted the water would wander off across
/// it; a map whose water holds something wanted gets its own step with the agent already afloat.
pub fn sweep_targets(on_map: Map, min_share: u8) -> Vec<crate::pokemon::species::PokemonSpecies> {
    use crate::pokemon::wild::{self, Terrain};
    wild::encounters(on_map).map_or_else(Vec::new, |wild| wild.species(Terrain::Grass).into_iter()
        .filter(|(_, share, _)| share * 100.0 >= min_share as f64)
        .map(|(species, _, _)| species)
        .collect())
}

/// What is left to catch here — the step's completion test, empty when it is done.
pub fn sweep_remaining(state: &GameState, on_map: Map, min_share: u8)
    -> Vec<crate::pokemon::species::PokemonSpecies> {
    sweep_targets(on_map, min_share).into_iter()
        .filter(|species| !state.pokedex_owned.contains(species))
        .collect()
}

/// Whether to throw a ball at `enemy` mid-sweep.
///
/// Deliberately **not** `sweep_targets().contains(enemy)`: anything the dex does not have is worth a
/// ball while we are already standing in the battle. `min_share` decides where to *stand*, and how
/// long to stay; it should not veto a rare species that turned up anyway.
pub fn sweep_wants(state: &GameState, enemy: crate::pokemon::species::PokemonSpecies) -> bool {
    !state.pokedex_owned.contains(&enemy)
}

impl PolicyStep {
    /// **H1/H2** — collect **HM05 Flash** from the Route 2 Gate aide, then teach it and light a cave.
    ///
    /// The aide's gate is a plain `hOaksAideRequirement = 10` against **dex owned**
    /// (`scripts/Route2Gate.asm:9-27`), so nothing here is conditional beyond having got there — but
    /// three preconditions are, and each would fail quietly:
    ///
    /// 1. **A free bag slot.** `OaksAideScript` refuses with "you have no room for this item" and the
    ///    conversation ends normally, so a full bag reads exactly like a successful one.
    /// 2. **A party member that can learn Flash.** Only **Slowpoke** and **Mr. Mime** can, of
    ///    everything this save has ever owned — Venusaur, Articuno, Vaporeon, Lapras, Aerodactyl,
    ///    Hitmonlee, Omanyte, Tangela and Farfetch'd all cannot. `flash_slot` is where the caller has
    ///    put one; `withdraw` names it if it has to come out of the box first.
    /// 3. **Route 2's gate is on its north half.** `Route2Gate`'s doors are Route 2 (16,35) and
    ///    (15,39), and the north strip only reaches them past the cut tree at (5,10) —
    ///    `postgame::trades` has the map.
    pub fn flash_steps(withdraw: Option<u8>, shed: ItemId, flash_slot: u8) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::ViridianCity }, Self::enter(Map::ViridianPokecenter)];
        // Free a bag slot for the HM, and fetch the only mon that can hold it.
        s.push(Self::deposit_item(shed, u8::MAX, Map::ViridianPokecenter));
        s.extend(withdraw.map(|box_slot| Self::withdraw_pokemon(box_slot, Map::ViridianPokecenter)));
        s.extend([
            Self::enter(Map::ViridianCity),
            Self::Fly { to: Map::PewterCity },
            Self::enter(Map::Route2),
            Self::CutTree { map: Map::Route2 },
            Self::enter(Map::Route2Gate),
        ]);
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::ROUTE2GATE_OAKS_AIDE), 3));
        s.extend([
            Self::enter(Map::Route2),
            Self::TeachMove { item: ItemId::Hm05Flash, target: PartyRef::Slot(flash_slot) },
            // …and prove it. Rock Tunnel is the game's one genuinely dark map: entering 1F sets
            // `wMapPalOffset = 6` and only Flash clears it.
            Self::Fly { to: Map::LavenderTown },
            Self::enter(Map::Route10),
            Self::enter(Map::RockTunnel1F),
            Self::UseFlash { slot: flash_slot },
        ]);
        s
    }

    /// **H3** — collect the **Itemfinder** from the `Route11Gate2F` aide.
    ///
    /// The gate is `hOaksAideRequirement = 30` against dex owned (`scripts/Route11Gate2F.asm`), which
    /// is why this row waited for **E**: the Safari sweep is what took the count from 19 to 31.
    ///
    /// `shed` is what goes into PC item storage on the way past — the bag is 20/20 on E's output, and
    /// `OaksAideScript` refuses a full bag with one text box and a normal-looking goodbye. Two slots,
    /// not one, because H4 used to pick something up afterwards without another detour to a PC; H4
    /// is gone and H5 inherits both slots, which is what `dex_sweep_outfit_steps` sheds into.
    ///
    /// The gate is a building sitting *in* Route 11 rather than between two maps: its west doors are
    /// Route 11 (49,8)/(49,9) and its east doors (58,8)/(58,9), both on the same route, so the whole
    /// trip stays on the Vermilion side and never needs Route 12.
    pub fn itemfinder_steps(shed: &[ItemId]) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::VermilionCity }, Self::enter(Map::VermilionPokecenter)];
        s.extend(shed.iter().map(|&item| Self::deposit_item(item, u8::MAX, Map::VermilionPokecenter)));
        s.extend([
            Self::enter(Map::VermilionCity),
            Self::goto(Map::Route11),
            Self::enter(Map::Route11Gate1F),
            Self::enter(Map::Route11Gate2F),
        ]);
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::ROUTE11GATE2F_OAKS_AIDE), 3));
        s.extend([
            Self::enter(Map::Route11Gate1F),
            // ⚠️ Name the landing. All four of `Route11Gate1F`'s doors are `LAST_MAP` warps and two of
            // them come out on the *east* side of the gate building, which is the wrong side of a
            // solid wall from both Vermilion and the hidden item at (49,5). (50,8) is the west pair.
            Self::enter_at(Map::Route11, 50, 8),
        ]);
        s
    }

    /// **H5** — the sweep's shopping trip: a bag slot, a hundred balls, and an empty box.
    ///
    /// All three are things whose absence does not error. A full **bag** means the balls are never
    /// bought (the clerk says so and moves on). No **balls** means `SweepDex` gives up on arrival. And
    /// a full **box** is the worst of the three: a catch with a full party goes to the PC, a full box
    /// refuses it, and the sweep throws balls at the same species until the budget runs out.
    ///
    /// Poké Balls, not Great Balls, and the arithmetic is why. The party is level 71 and the targets
    /// are level 3–24, so the policy never weakens anything (it would one-shot it) — every throw is at
    /// full HP, where the second roll is `86/256` for a Poké Ball and `128/256` for a Great Ball. A
    /// Great Ball is 1.5–1.9× the catch for **3×** the price, and the run has ¥35k and 19 species to
    /// find. Vermilion's mart is also the only one on this leg, and it sells nothing better.
    pub fn dex_sweep_outfit_steps(shed: &[ItemId], balls: u8, box_n: u8) -> Vec<Self> {
        let mut s = vec![Self::Fly { to: Map::VermilionCity }, Self::enter(Map::VermilionPokecenter)];
        s.extend(shed.iter().map(|&item| Self::deposit_item(item, u8::MAX, Map::VermilionPokecenter)));
        // ⚠️ `change_box` **saves the game** (see `postgame::pc_box`), so it belongs here at the top of
        // a leg rather than mid-sweep.
        s.push(Self::change_box(box_n, Map::VermilionPokecenter));
        s.extend([
            Self::enter(Map::VermilionCity),
            Self::enter(Map::VermilionMart),
            Self::BuyFromMart { item: crate::pokemon::bag::BagItem::new(ItemId::PokeBall, balls),
                                map: Map::VermilionMart },
            Self::enter(Map::VermilionCity),
        ]);
        s
    }

    /// **H5a** — the two grounds either side of where H4 leaves the agent standing.
    ///
    /// Route 11 owes **Ekans** (40.2 % of its grass) and **Drowzee** (25.0 %); Diglett's Cave is
    /// **94.5 % Diglett**, which makes it the cheapest single species in Kanto. Both are off the map
    /// H4 ends on, so this leg spends its walking on the shopping trip and almost none on travel.
    ///
    /// The cave is two warps deep — Route 11 (5,5) is the entrance *building*, not the cave — and the
    /// leg walks back out of both so it ends outdoors, where the next leg's `Fly` is legal.
    pub fn dex_sweep_vermilion_steps(min_share: u8) -> Vec<Self> {
        vec![
            Self::goto(Map::Route11),
            Self::sweep(Map::Route11, min_share),
            Self::enter(Map::DiglettsCaveRoute11),
            Self::enter(Map::DiglettsCave),
            Self::sweep(Map::DiglettsCave, min_share),
            Self::enter(Map::DiglettsCaveRoute11),
            Self::enter(Map::Route11),
        ]
    }

    /// **H5b** — Route 1 and Viridian Forest: seven species at levels 3–5, and the cheapest of the lot.
    ///
    /// Route 1 is half Pidgey and half Rattata, and the forest is the one ground worth *lingering* on
    /// — Weedle 45 % and Kakuna 39 % are what a 20 % floor would stop for, but Caterpie, Metapod and
    /// **Pikachu** sit behind them at 5 % each with catch rates of 255/120/190, so a 5 % floor takes
    /// five species from one patch of grass. It is swept last for that reason: everything caught here
    /// is caught at one-shot speed, with no weakening turn (the party out-levels it by sixty).
    pub fn dex_sweep_viridian_steps(min_share: u8) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::ViridianCity },
            Self::enter(Map::Route1),
            Self::sweep(Map::Route1, min_share),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::Route2),
            Self::enter(Map::ViridianForestSouthGate),
            Self::enter(Map::ViridianForest),
            // 5, not `min_share`: see above — the forest's tail is worth waiting for.
            Self::sweep(Map::ViridianForest, 5),
            Self::enter(Map::ViridianForestSouthGate),
            Self::enter(Map::Route2),
        ]
    }

    /// **H5c** — the Lavender grounds: Pokémon Tower **3F**, and Rock Tunnel.
    ///
    /// Four species from two grounds that share a Fly stop. Tower 3F is **89.5 % Gastly**, and Rock
    /// Tunnel is 55/25/15 Zubat/Geodude/Machop at catch rates 255/255/180 — three more species for
    /// almost no balls. The tunnel's Onix is left alone (5 %, catch rate 45).
    ///
    /// The tunnel is the game's dark map, and this walks in without using Flash: the agent routes off
    /// RAM collision rather than the visible screen, which H2 already recorded.
    ///
    /// Two grounds this leg does **not** visit, both for the same reason — the ROM has grass there and
    /// the agent cannot reach it, which is a stall rather than an error:
    ///
    /// - ⚠️ **Route 8.** From either end its action list is the two `Route8Gate` doors, the Underground
    ///   Path and nine trainers, with **no `Grass` at all**. Its Mankey and Growlithe come from Route 7
    ///   instead. Same shape as the Route 15 trap in §11; measured with `probe_timeout_artifact`.
    /// - ⚠️ **Tower 6F/7F.** `enter(PokemonTower6F)` finds no route from 5F on a save that has already
    ///   *finished* the tower: the mainline climb only works because it passes through while the
    ///   Channelers are still being fought and it collects the Rare Candy that unblocks 6F's chokepoint
    ///   on the way. 3F carries the Gastly this leg needs, so the climb stops there — Haunter and Cubone
    ///   (15 %/10 %, top floor only) are not worth reopening it for.
    pub fn dex_sweep_lavender_steps(min_share: u8) -> Vec<Self> {
        vec![
            Self::Fly { to: Map::LavenderTown },
            // ⚠️ **The Silph Scope has to be in the bag, not the PC.** `IsGhostBattle`
            // (`engine/battle/core.asm:3308`) turns *every* wild on `POKEMON_TOWER_1F..7F` into an
            // uncatchable GHOST unless `IsItemInBag SILPH_SCOPE` succeeds — and Phase 0 banked the
            // Scope to free bag space, so this chain arrives without it. Nothing says so: the balls
            // are thrown and consumed exactly as normal, and simply never catch. It cost 71 of them
            // before the zero nickname prompts gave it away. The Escape Rope goes the other way to
            // make room; H4 already proved its collection and the PC has another.
            Self::enter(Map::LavenderPokecenter),
            Self::deposit_item(ItemId::EscapeRope, u8::MAX, Map::LavenderPokecenter),
            Self::withdraw_item(ItemId::SilphScope, 1, Map::LavenderPokecenter),
            Self::enter(Map::LavenderTown),
            Self::enter(Map::PokemonTower1F),
            Self::enter(Map::PokemonTower2F),
            Self::enter(Map::PokemonTower3F),
            Self::sweep(Map::PokemonTower3F, min_share),
            Self::enter(Map::PokemonTower2F),
            Self::enter(Map::PokemonTower1F),
            Self::enter(Map::LavenderTown),
            Self::enter(Map::Route10),
            Self::enter(Map::RockTunnel1F),
            // 10 — takes the Machop (14.8 %, catch rate 180), leaves the catch-rate-45 Onix at 5 %.
            Self::sweep(Map::RockTunnel1F, 10),
            Self::enter(Map::Route10),
        ]
    }

    /// **H5d** — Route 7's Oddish, and Pokémon Mansion for the margin.
    ///
    /// Route 7 is reached from **Celadon**, not Saffron: the Saffron side is behind `Route7Gate`, and
    /// the plain Saffron→Route 7 connection lands in a ledge-sealed pocket with no path to the gate
    /// (`hm02_steps` records the same thing from the other direction).
    ///
    /// The Mansion is pure margin — 40 % Koffing, 40 % Ponyta, and Growlithe and Grimer behind them,
    /// all at catch rate 190. It is the densest ground in Kanto that a Fly reaches directly, which is
    /// what makes it the right place to make up any shortfall.
    pub fn dex_sweep_celadon_steps() -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CeladonCity },
            // 5, not `min_share`: Route 7 now owes **Mankey (30 %), Oddish (30 %) and Growlithe
            // (10 %)** — Route 8's share of the sweep moved here when its grass turned out to be
            // unreachable, and Growlithe sits below a 20 % floor.
            Self::enter(Map::Route7),
            Self::sweep(Map::Route7, 5),
            Self::enter(Map::CeladonCity),
        ]
    }

    /// **H5e** — Pokémon Mansion 1F. See [`Self::dex_sweep_celadon_steps`] for why it is the margin.
    pub fn dex_sweep_mansion_steps() -> Vec<Self> {
        vec![
            Self::Fly { to: Map::CinnabarIsland },
            Self::enter(Map::PokemonMansion1F),
            Self::sweep(Map::PokemonMansion1F, 5),
            Self::enter(Map::CinnabarIsland),
        ]
    }

    /// **H5** — collect **Exp.All** from the `Route15Gate2F` aide at 50 species owned.
    ///
    /// Reached from **Fuchsia**, and through the gate's ground floor: Route 15's own grass is all on
    /// the far side of that building, which is the trap `postgame::trades` recorded when a `goto` on
    /// this route paced a grassless strip for ninety emulated minutes.
    pub fn exp_all_steps() -> Vec<Self> {
        let mut s = vec![
            Self::Fly { to: Map::FuchsiaCity },
            Self::goto(Map::Route15),
            Self::enter(Map::Route15Gate1F),
            Self::enter(Map::Route15Gate2F),
        ];
        s.extend(std::iter::repeat_n(Self::Interact(MapSprite::ROUTE15GATE2F_OAKS_AIDE), 3));
        s.extend([Self::enter(Map::Route15Gate1F), Self::enter(Map::Route15)]);
        s
    }

    fn sweep(on_map: Map, min_share: u8) -> Self {
        Self::SweepDex { on_map, min_share, ball: Some(ItemId::PokeBall) }
    }
}

// ⚠️ **The three tests that were here went with the decoder they pinned.**
// `hidden_items_match_the_rom_coord_table`, `every_decoded_hidden_item_has_a_flag` and
// `route11_has_one_hidden_escape_rope` cross-checked `HiddenObjects` against `HiddenItemCoords` on
// all 54 rows, and there is nothing left for them to check: no code in this crate reads either
// table any more. They are named here so a future reader looking for the cross-check knows it
// existed and why it does not.

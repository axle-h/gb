//! Workstream **H — Oak's aides**. See `docs/postgame-coverage-plan.md` §6-H. Depends on A–G for dex
//! count.
//!
//! Three items gated on **dex owned**: HM05 Flash at 10 (Route 2 Gate), the Itemfinder at 30
//! (`Route11Gate2F`), Exp.All at 50 (`Route15Gate2F`). Check the gate with `probe_coverage` before
//! travelling — don't guess.
//!
//! Sub-steps: H1 Flash · H2 teach Flash and prove it · H3 Itemfinder · H4 hidden items ·
//! H5 Exp.All, then `postgame-aides.bin`.
//!
//! # Hidden items (H4)
//!
//! §6-H4 called for a `PolicyStep::SearchHiddenItem { at }` and an `AgentState` to drive it. Neither
//! is needed: a hidden item is picked up by **standing next to it and pressing A**, which is the
//! `CheckTrashCan` field move the gym cans and the Mansion/Rocket switches already share. So the
//! reserved `FieldMove::SearchHiddenItem` and `AgentState::SearchingHiddenItem` seams are **gone** —
//! same conclusion `postgame::trades` reached about `TradePokemon`. What is left is data, and this
//! module reads it out of the ROM rather than transcribing it: see [`hidden_items`].

use crate::geometry::Point8;
use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::policy::{FieldMove, PartyRef, PolicyStep};
use crate::pokemon::roms;
use crate::pokemon::symbols::{pokered_symbols, DmgBank, DmgPointer};
use crate::pokemon::GameState;

/// One row of the ROM's hidden-item table: an item lying on a tile with nothing to show for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiddenItem {
    /// **Raw** map coordinates, exactly as `HiddenItemCoords` stores them — with no connection-strip
    /// offset applied. Outdoor maps are widened by the strip of each neighbour they touch, so on
    /// Route 11 (a `WEST | EAST` map) every raw x is one short of the tile the agent walks on. The
    /// offset is applied in exactly one place, [`crate::pokemon::tile_map::MetaTileMap::new`], which
    /// is where the other raw-coordinate tables (Strength switches, floor holes) are corrected too.
    pub at: Point8,
    pub item: ItemId,
    /// Bit index into `wObtainedHiddenItemsFlags` — this row's position in `HiddenItemCoords`, which
    /// is how `FindHiddenItemOrCoinsIndex` addresses it.
    pub flag: u8,
}

/// Every hidden item on `map`, decoded from the ROM.
///
/// Two tables have to be crossed, because neither is complete on its own.
/// `data/events/hidden_objects.asm` holds the **item id** but mixes hidden items in with bookshelves,
/// PCs, gym statues and the Safari Zone's own scripts — the discriminator is the object's routine
/// pointer being `HiddenItems`. `data/events/hidden_item_coords.asm` holds only map/x/y, but its
/// **row order is the flag numbering**, which is the only way to ask "has this one been taken?".
///
/// Derived rather than transcribed for the reason `world_graph` builds itself from ROM headers: 55
/// rows of hand-copied coordinates is 55 chances to be silently one off, and a wrong coordinate here
/// does not error — the agent walks to a tile, presses A, and nothing at all happens.
pub fn hidden_items(map: Map) -> Vec<HiddenItem> {
    /// `hidden_object` is `db y, db x, db arg` then `dba` (bank + address) of its routine.
    const ENTRY: usize = 6;

    let Some(index) = rom_at(&pokered_symbols::HiddenObjectMaps).iter()
        .take_while(|&&b| b != 0xFF)
        .position(|&b| b == map as u8)
    else { return Vec::new() };

    let pointers = &pokered_symbols::HiddenObjectPointers;
    let DmgBank::ROM { bank } = pointers.bank else { panic!("HiddenObjectPointers is not in ROM") };
    let table = rom_at(pointers);
    let list = rom_bank_at(bank, u16::from_le_bytes([table[index * 2], table[index * 2 + 1]]));

    let items_routine = &pokered_symbols::HiddenItems;
    let DmgBank::ROM { bank: routine_bank } = items_routine.bank else { panic!("HiddenItems is not in ROM") };

    let mut found = Vec::new();
    let mut i = 0;
    while list[i] != 0xFF {
        let (y, x, arg) = (list[i], list[i + 1], list[i + 2]);
        let is_hidden_item = list[i + 3] == routine_bank
            && u16::from_le_bytes([list[i + 4], list[i + 5]]) == items_routine.address;
        if is_hidden_item {
            let item = ItemId::from_repr(arg)
                .unwrap_or_else(|| panic!("hidden item ${arg:02x} on {map} is not a known ItemId"));
            let at = Point8 { x, y };
            found.push(HiddenItem { at, item, flag: hidden_item_flag(map, at) });
        }
        i += ENTRY;
    }
    found
}

/// This row's index in `HiddenItemCoords` (`db map, y, x`), which is its bit in
/// `wObtainedHiddenItemsFlags`.
fn hidden_item_flag(map: Map, at: Point8) -> u8 {
    let coords = rom_at(&pokered_symbols::HiddenItemCoords);
    let mut i = 0;
    while coords[i] != 0xFF {
        if coords[i] == map as u8 && coords[i + 1] == at.y && coords[i + 2] == at.x {
            return (i / 3) as u8;
        }
        i += 3;
    }
    panic!("{map} {at} is a HiddenItems object with no HiddenItemCoords row");
}

/// The ROM from `ptr` onward. Banked pointers address `$4000..$7FFF` through the window.
fn rom_at(ptr: &DmgPointer) -> &'static [u8] {
    let DmgBank::ROM { bank } = ptr.bank else { panic!("{ptr:?} is not in ROM") };
    rom_bank_at(bank, ptr.address)
}

fn rom_bank_at(bank: u8, address: u16) -> &'static [u8] {
    let offset = if bank == 0 { address as usize }
                 else { bank as usize * 0x4000 + (address as usize - 0x4000) };
    &roms::POKERED[offset..]
}

/// **H4** — the field move that collects the hidden `item` on the current map.
///
/// `baseline` is how many of `item` the bag held when the step began: the completion test is that it
/// went **up**, not that the item is present, because the bag may already hold a stack of the same
/// thing (Route 11's hidden item is an Escape Rope, and the PC has one). Returns `None` when the pick
/// up has landed, which is the caller's signal to pop the step.
///
/// ⚠️ **A full bag reads as success.** `FoundHiddenItemText` prints "found ESCAPE ROPE!", fails
/// `GiveItem`, prints "you have no room" and leaves the flag *unset* — so a full bag here is an
/// endless retry, not a wrong answer. That is deliberate: it stalls the leg loudly instead of walking
/// on with nothing.
pub fn pick(state: &GameState, item: ItemId, baseline: u8) -> Option<FieldMove> {
    if bag_quantity(state, item) > baseline {
        return None;
    }
    let (target, _) = state.map.hidden_items.iter().copied()
        .find(|&(_, found)| found == item)?;
    // `facing: None` — the ROM only asks that the tile in front of the player match, so any reachable
    // side will do, and `route_to_face_dir` then takes the nearest one.
    Some(FieldMove::CheckTrashCan { target, facing: None::<PlayerFacingDirection> })
}

/// How many of `item` the bag holds. Zero for an item that is not in it.
pub fn bag_quantity(state: &GameState, item: ItemId) -> u8 {
    state.bag.iter().find(|i| i.id == item).map_or(0, |i| i.quantity)
}

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
    /// not one, so [`Self::hidden_item_steps`] can pick something up afterwards without another
    /// detour to a PC.
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

    /// **H4** — collect the hidden `item` on `map`.
    ///
    /// The step routes itself, so this is only a `goto` away from being one line; it exists to carry
    /// the two facts a caller needs. First, **a free bag slot** — see [`super::aides::pick`], where a
    /// full bag is an endless retry rather than a silent skip. Second, `item` picks *which* hidden
    /// item when a map has several, and five of the maps do (Route 17 has five).
    ///
    /// Route 11's is an **Escape Rope** at raw (48,5), one tile west of the wall the gate building
    /// stands against — so [`Self::itemfinder_steps`] ends two dozen steps away from it and H3's
    /// output is the natural entry fixture.
    pub fn hidden_item_steps(map: Map, item: ItemId) -> Vec<Self> {
        vec![Self::SearchHiddenItem { map, item }]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin [`hidden_items`] against `HiddenItemCoords` — the ROM's *own* index of hidden items, and
    /// the one this decoder does not read to find them (it walks the per-map `HiddenObjects` lists
    /// and filters on the routine pointer). The two are built from different tables in different
    /// banks, so agreeing on all 55 rows is a real cross-check rather than a tautology.
    #[test]
    fn hidden_items_match_the_rom_coord_table() {
        let coords = rom_at(&pokered_symbols::HiddenItemCoords);
        let mut rows = 0;
        let mut i = 0;
        while coords[i] != 0xFF {
            let (map_id, at) = (coords[i], Point8 { x: coords[i + 2], y: coords[i + 1] });
            let flag = (i / 3) as u8;
            i += 3;
            rows += 1;
            // Every row is a map `Map` models, including `UNUSED_MAP_6F` — the cut floor still has
            // its coords row, its hidden object and therefore its flag.
            let map = Map::from_repr(map_id).unwrap_or_else(|| panic!("no Map for id ${map_id:02x}"));
            let found = hidden_items(map);
            let entry = found.iter().find(|h| h.at == at)
                .unwrap_or_else(|| panic!("{map} {at} (flag {flag}) is in HiddenItemCoords but not in \
                    its HiddenObjects list — decoded {found:?}"));
            assert_eq!(entry.flag, flag, "flag index for {map} {at}");
        }
        assert_eq!(rows, 54, "HiddenItemCoords should have 54 rows");
    }

    /// …and the other direction: nothing the decoder finds is missing from the coords table. A
    /// `HiddenItems` object with no coords row would have **no flag**, so it could be collected over
    /// and over (`FindHiddenItemOrCoinsIndex` falls off the end of the list and returns a junk index).
    #[test]
    fn every_decoded_hidden_item_has_a_flag() {
        use strum::IntoEnumIterator;
        // `hidden_item_flag` panics on an object with no coords row, so reaching 54 at all is the
        // assertion; the count is here so a ROM change cannot quietly drop rows instead.
        let total: usize = Map::iter().map(|m| hidden_items(m).len()).sum();
        assert_eq!(total, 54);
    }

    /// The one this workstream actually collects, spelled out: H4's target is Route 11's Escape Rope.
    #[test]
    fn route11_has_one_hidden_escape_rope() {
        assert_eq!(hidden_items(Map::Route11),
            vec![HiddenItem { at: Point8 { x: 48, y: 5 }, item: ItemId::EscapeRope, flag: 36 }]);
    }
}

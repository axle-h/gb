//! Workstream A — Pokémon storage (PC boxes).

use gb::mmu::MMU;
use crate::pokemon::agent::PokemonAgent;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::strings::PokemonString;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::{PokemonApi, PokemonApiTrait};
use gb::ram::ROM;

/// Members one box holds. Twelve boxes of twenty, so 240 banked Pokémon in total.
pub const BOX_CAPACITY: usize = 20;

/// Number of boxes (`sBox1`…`sBox12`).
pub const BOX_COUNT: u8 = 12;

/// Size of one `box_struct` (`pokered/macros/ram.asm:9`).
const BOX_MON_SIZE: u16 = 0x21;

/// Length of a name field in the box's parallel OT / nickname arrays.
const NAME_SIZE: u16 = 0x0B;

/// A Pokémon in PC storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxedPokemon {
    pub species: PokemonSpecies,
    pub nickname: PokemonString,
    pub trainer_name: PokemonString,
    pub level: u8,
    pub current_hp: u16,
    pub status: PokemonStatus,
    pub moves: [Option<PokemonMove>; 4],
}

/// Read the currently open box (`wBoxCount` + `wBoxMons`, WRAM `$da80`).
pub fn read_current_box(mmu: &MMU) -> Vec<BoxedPokemon> {
    let count = mmu.read_pointer(&pokered_symbols::wBoxCount).min(BOX_CAPACITY as u8);
    (0..count as u16).map_while(|i| read_boxed_pokemon(mmu, i)).collect()
}

/// Which box is open (`wCurrentBoxNum`, 0-based). The low nibble only: bit 7 is the changed-box
/// flag.
pub fn current_box_num(mmu: &MMU) -> u8 {
    mmu.read_pointer(&pokered_symbols::wCurrentBoxNum) & 0x7F
}

fn read_boxed_pokemon(mmu: &MMU, index: u16) -> Option<BoxedPokemon> {
    let bytes = mmu.read_pointer_vec(
        &(pokered_symbols::wBoxMons + index * BOX_MON_SIZE),
        BOX_MON_SIZE as usize,
    );
    Some(BoxedPokemon {
        species: PokemonSpecies::from_repr(bytes.read(0))?,
        nickname: mmu.read_pointer_pokemon_string(&(pokered_symbols::wBoxMonNicks + index * NAME_SIZE)),
        trainer_name: mmu.read_pointer_pokemon_string(&(pokered_symbols::wBoxMonOT + index * NAME_SIZE)),
        current_hp: bytes.read_u16_be(1),
        level: bytes.read(3),
        status: bytes.read(4).into(),
        moves: std::array::from_fn(|i| {
            PokemonMoveName::from_repr(bytes.read(8 + i as u16))
                .map(|name| PokemonMove { name, pp: bytes.read(29 + i as u16) })
        }),
    })
}

/// What a box operation is doing. Carried in `AgentState::UsingPcBox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcBoxOp {
    /// Party slot → box (A3).
    Deposit { slot: u8 },
    /// Box slot → party (A4).
    Withdraw { box_slot: u8 },
    /// Switch to box `n` (0-based, 0..12).
    ChangeBox { n: u8 },
    /// Permanently release the box member at `box_slot` (A6).
    Release { box_slot: u8 },
}

impl PcBoxOp {
    fn menu_index(self) -> u8 {
        match self {
            Self::Withdraw { .. } => 0,
            Self::Deposit { .. } => 1,
            Self::Release { .. } => 2,
            Self::ChangeBox { .. } => 3,
        }
    }

    /// Row to put the cursor on in the mon list this op opens — a party slot for `Deposit`, a box
    /// slot for `Withdraw`/`Release`.
    fn list_row(self) -> Option<u8> {
        match self {
            Self::Deposit { slot } => Some(slot),
            Self::Withdraw { box_slot } | Self::Release { box_slot } => Some(box_slot),
            Self::ChangeBox { .. } => None,
        }
    }

    /// Why this op cannot run right now, if it cannot. Checked before any menu is opened, because
    /// pokered answers every one of these with a message and a bounce straight back to the
    /// Bill's-PC menu (`CantDepositLastMonText`, `BoxFullText`, `NoMonText`, `CantTakeMonText`) —
    /// from which a driver that re-picked the same entry would loop forever.
    pub fn blocked_by(self, party: u8, boxed: u8, current_box: u8) -> Option<String> {
        match self {
            Self::Deposit { slot } => {
                if slot >= party { Some(format!("party has no slot {slot} (count {party})")) }
                else if party <= 1 { Some("can't deposit the last Pokémon".into()) }
                else if boxed as usize >= BOX_CAPACITY { Some(format!("box {} is full", current_box + 1)) }
                else { None }
            }
            Self::Withdraw { box_slot } => {
                if box_slot >= boxed { Some(format!("box {} has no slot {box_slot} (count {boxed})", current_box + 1)) }
                else if party >= 6 { Some("party is full".into()) }
                else { None }
            }
            Self::Release { box_slot } => {
                if box_slot >= boxed { Some(format!("box {} has no slot {box_slot} (count {boxed})", current_box + 1)) }
                else { None }
            }
            Self::ChangeBox { n } => {
                if n >= BOX_COUNT { Some(format!("box {n} is out of range (0..{BOX_COUNT})")) }
                else if n == current_box { Some(format!("box {} is already open", n + 1)) }
                else { None }
            }
        }
    }
}

/// Live state of an in-progress box operation. Carried in `AgentState::UsingPcBox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcBoxState {
    pub op: PcBoxOp,
    /// Coordinate of the PC hidden object, from `MetaTileMap::pc_locations`.
    pub pc: gb::geometry::Point8,
    /// Party / box counts before any menu was touched — the baselines completion is measured
    /// against.
    start_party: u8,
    start_boxed: u8,
    /// Press/release alternation, so every input is a fresh rising edge.
    press: bool,
    /// Set once the PC menu has been opened, i.e. we have left the overworld at least once.
    entered_menu: bool,
    ticks: u16,
}

/// Ceiling on menu ticks for one operation.
const TICK_BUDGET: u16 = 1200;

impl PcBoxState {
    /// Baseline the party and box counts *now*, before any menu is touched.
    pub fn new(op: PcBoxOp, pc: gb::geometry::Point8, api: &PokemonApi<'_>) -> Self {
        Self {
            op,
            pc,
            start_party: party_count(api),
            start_boxed: api.mmu().read_pointer(&pokered_symbols::wBoxCount),
            press: true,
            entered_menu: false,
            ticks: 0,
        }
    }
}

fn party_count(api: &PokemonApi<'_>) -> u8 {
    api.mmu().read_pointer(&pokered_symbols::wPartyCount)
}

/// One agent tick of the box-menu driver. Called from `agent.rs` via a single delegating match
/// arm.
/// ```text
/// overworld     walk below the PC, face UP, press A
///   → "Switch on!"                                       mash A
///   → PC parent menu                                     cursor → 0, A
///        BILL's PC / <PLAYER>'s PC / [PROF.OAK's PC] / [<PKMN>LEAGUE] / LOG OFF
///   → "What?" + Bill's PC menu                           cursor → op index, A
///        WITHDRAW <PKMN> / DEPOSIT <PKMN> / RELEASE <PKMN> / CHANGE BOX / SEE YA!
///   ├─ deposit / withdraw / release → mon list           cursor → the row, A
///   │    ├─ deposit / withdraw → DEPOSIT|WITHDRAW / STATS / CANCEL   cursor → 0, A
///   │    └─ release            → "Once released … OK?"   YES (index 0)
///   │  → "… was stored in Box 1!" etc.                   mash A → back to the Bill's PC menu
///   └─ change box → "data will be saved. OK?"            YES, then the BOX 1…BOX12 list → n, A
///   → B until the overworld returns
/// ```
pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: PcBoxState) -> Result<(), String> {
    use gb::joypad::JoypadButton;
    use crate::pokemon::agent::{AgentEvent, AgentState};
    use crate::pokemon::encoding::GameMode;
    use crate::pokemon::map_metadata::PlayerFacingDirection;
    use crate::pokemon::menu::TextBoxId;

    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    let party = party_count(api);
    let boxed = api.mmu().read_pointer(&pokered_symbols::wBoxCount);
    let current = current_box_num(api.mmu());

    // Give up and hand back to the policy, leaving the reason on the event stream.
    let abort = |agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, why: String| {
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox { message: format!("PC box: {why}") });
        agent.set_state(AgentState::Idle);
    };

    // ── Done?
    let done = match s.op {
        PcBoxOp::Deposit { .. } => party < s.start_party && boxed > s.start_boxed,
        PcBoxOp::Withdraw { .. } => party > s.start_party && boxed < s.start_boxed,
        PcBoxOp::Release { .. } => boxed < s.start_boxed,
        PcBoxOp::ChangeBox { n } => current == n,
    };
    if s.entered_menu && done {
        if game_mode != GameMode::Overworld {
            // Gen 1 drops back to the Bill's PC menu after each operation, so back out with B.
            api.release_all_buttons();
            if s.press { api.press_button(JoypadButton::B); }
            agent.set_state(AgentState::UsingPcBox(PcBoxState { press: !s.press, ticks: s.ticks + 1, ..s }));
            return Ok(());
        }
        api.release_all_buttons();
        agent.event(AgentEvent::TextBox {
            message: format!("PC box: {:?} done — party {party}, box {} holds {boxed}", s.op, current + 1),
        });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // ── Refuse up front what the game would refuse with a message and a bounce
    // ────────────────────
    if !s.entered_menu {
        if let Some(why) = s.op.blocked_by(party, boxed, current) {
            abort(agent, api, format!("{:?} not possible — {why}", s.op));
            return Ok(());
        }
    }

    if s.ticks > TICK_BUDGET {
        abort(agent, api, format!("{:?} made no progress in {TICK_BUDGET} ticks", s.op));
        return Ok(());
    }

    // ── Back in the overworld having achieved nothing — the attempt fizzled.
    if s.entered_menu && game_mode == GameMode::Overworld {
        api.release_all_buttons();
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    // ── Still outside: walk to the tile below the PC and face up, then press A
    // ────────────────────
    if game_mode == GameMode::Overworld {
        let gs = agent.observe_state(api)?;
        match gs.map.route_to_face_dir(s.pc, Some(PlayerFacingDirection::Up)).as_deref() {
            Some([]) => {
                api.release_all_buttons();
                if s.press { api.press_button(JoypadButton::A); }
                agent.set_state(AgentState::UsingPcBox(PcBoxState { press: !s.press, ticks: s.ticks + 1, ..s }));
            }
            Some(&[btn, ..]) => {
                api.release_all_buttons();
                api.press_button(btn);
                agent.set_state(AgentState::UsingPcBox(PcBoxState { press: true, ticks: s.ticks + 1, ..s }));
            }
            _ => abort(agent, api, format!("can't reach the PC at {}", s.pc)),
        }
        return Ok(());
    }

    // ── Inside the menus
    // ─────────────────────────────────────────────────────────────────────────
    let s = PcBoxState { entered_menu: true, ticks: s.ticks + 1, ..s };
    if !s.press {
        api.release_all_buttons();
        agent.set_state(AgentState::UsingPcBox(PcBoxState { press: true, ..s }));
        return Ok(());
    }

    let (_, _, cursor, scroll) = api.menu_geometry();
    let tbid = api.menu_state().map(|m| m.text_box_id);
    let text = api.on_screen_text(false).unwrap_or_default();
    let nav = |cur: u8, target: u8| -> JoypadButton {
        if cur < target { JoypadButton::Down }
        else if cur > target { JoypadButton::Up }
        else { JoypadButton::A }
    };

    // Order matters.
    let button = if text.contains("STATS") && text.contains("CANCEL") {
        nav(cursor, 0) // DEPOSIT|WITHDRAW / STATS / CANCEL → confirm
    } else if text.contains("Choose a") {
        // BOX 1…BOX12.
        match s.op { PcBoxOp::ChangeBox { n } => nav(cursor, n), _ => JoypadButton::B }
    } else if tbid == Some(TextBoxId::TwoOptionMenu) {
        nav(cursor, 0) // "…OK?" (release confirm, change-box save confirm) → YES
    } else if tbid == Some(TextBoxId::ListMenuBox) {
        match s.op.list_row() {
            Some(row) => nav(cursor + scroll, row),
            None => JoypadButton::B,
        }
    } else if text.contains("SEE YA!") {
        nav(cursor, s.op.menu_index()) // Bill's PC menu
    } else if text.contains("LOG OFF") {
        nav(cursor, 0) // PC parent menu → BILL's/SOMEONE's PC (always index 0)
    } else {
        JoypadButton::A // transitional text: "Switch on!", "What?", "… was stored in Box 1!"
    };

    api.release_all_buttons();
    api.press_button(button);
    agent.set_state(AgentState::UsingPcBox(PcBoxState { press: false, ..s }));
    Ok(())
}

impl crate::pokemon::policy::PolicyStep {
    /// A3 — deposit the party member in `slot` into the open box, at the PC on `map`.
    pub const fn deposit_pokemon(slot: u8, map: crate::pokemon::map::Map) -> Self {
        Self::UsePcBox { op: PcBoxOp::Deposit { slot }, map }
    }

    /// A4 — withdraw the open box's member at `box_slot` into the party.
    pub const fn withdraw_pokemon(box_slot: u8, map: crate::pokemon::map::Map) -> Self {
        Self::UsePcBox { op: PcBoxOp::Withdraw { box_slot }, map }
    }

    /// A5 — switch to box `n` (0-based). Saves the game; see [`tick`].
    pub const fn change_box(n: u8, map: crate::pokemon::map::Map) -> Self {
        Self::UsePcBox { op: PcBoxOp::ChangeBox { n }, map }
    }

    /// A6 — permanently release the open box's member at `box_slot`.
    pub const fn release_pokemon(box_slot: u8, map: crate::pokemon::map::Map) -> Self {
        Self::UsePcBox { op: PcBoxOp::Release { box_slot }, map }
    }
}

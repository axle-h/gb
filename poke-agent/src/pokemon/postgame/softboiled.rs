//! Softboiled from the party menu: START, POKéMON, the giver, its field-move box, then the party
//! menu `.softboiled` brings back for the target, and B out of everything once the heal lands.

use gb::joypad::JoypadButton;
use crate::pokemon::agent::{field_move_menu_button, AgentState, PokemonAgent};
use crate::pokemon::encoding::{GameMode, PokemonBlockAddresses};
use crate::pokemon::symbols::pokered_symbols;
use crate::pokemon::{PokemonApi, PokemonApiTrait};
use gb::ram::ROM;

/// Ticks until the driver stops choosing and only backs out.
const TICK_BUDGET: u32 = 1500;
/// Ticks of backing out after that before it hands back wherever it is.
const LEAVE_BUDGET: u32 = 600;
/// Ticks a chosen target may show no heal before the driver backs out.
const AIM_BUDGET: u32 = 150;
/// Stable-overworld ticks before handing back, as the bag driver waits.
const SETTLE_TICKS: u8 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftboiledState {
    pub giver: u8,
    pub move_index: u8,
    pub target: u8,
    /// The target's HP before, which is how the heal is told from a refusal.
    pub before: u16,
    /// The giver's, which `.softboiled` takes the moment the target is chosen; the target's own
    /// rises only after the giver's bar has run down.
    giver_before: u16,
    /// Press/release alternation, so every input is a fresh rising edge.
    press: bool,
    entered_menu: bool,
    /// The party menu asking for a target has been up.
    chosen: bool,
    /// The tick A was first pressed on the target.
    aimed: Option<u32>,
    leaving: bool,
    settle: u8,
    ticks: u32,
}

impl SoftboiledState {
    pub fn new(giver: u8, move_index: u8, target: u8, api: &PokemonApi<'_>) -> Self {
        Self { giver, move_index, target, before: party_hp(api, target), giver_before: party_hp(api, giver),
               press: true, entered_menu: false,
               chosen: false, aimed: None, leaving: false, settle: 0, ticks: 0 }
    }
}

/// The current HP of party member `member`, which `ItemUseMedicine` writes before it animates it.
fn party_hp(api: &PokemonApi<'_>, member: u8) -> u16 {
    let at = pokered_symbols::wPartyMons.address + member as u16 * PokemonBlockAddresses::POKEMON_BLOCK_SIZE + 1;
    u16::from_be_bytes([api.mmu().read(at), api.mmu().read(at + 1)])
}

pub fn tick(agent: &mut PokemonAgent, api: &mut PokemonApi<'_>, s: SoftboiledState) -> Result<(), String> {
    let game_mode = api.game_mode().unwrap_or(GameMode::Overworld);
    let hp = party_hp(api, s.target);
    let mut s = SoftboiledState { entered_menu: s.entered_menu || game_mode != GameMode::Overworld,
                                  ticks: s.ticks + 1, ..s };
    api.release_all_buttons();

    if s.entered_menu && game_mode == GameMode::Overworld {
        if s.settle < SETTLE_TICKS {
            agent.set_state(AgentState::Softboiling(SoftboiledState { settle: s.settle + 1, ..s }));
            return Ok(());
        }
        let max = api.game_state().ok().and_then(|state| state.pokemon.get(s.target as usize).map(|mon| mon.stats.hp));
        agent.field_move_report(hp > s.before, match hp > s.before {
            true => format!("Softboiled from party slot {} healed party slot {} to {hp}/{} HP", s.giver, s.target,
                            max.unwrap_or(hp)),
            false => format!("Softboiled from party slot {} healed nobody: party slot {} is still at {hp} HP",
                             s.giver, s.target),
        });
        agent.set_state(AgentState::Idle);
        return Ok(());
    }
    s.settle = 0;
    if s.ticks > TICK_BUDGET + LEAVE_BUDGET {
        agent.field_move_report(false, format!(
            "Softboiled: the menus were still open after {} ticks", TICK_BUDGET + LEAVE_BUDGET));
        agent.set_state(AgentState::Idle);
        return Ok(());
    }

    let text = api.on_screen_text(false).unwrap_or_default().to_ascii_uppercase();
    let landed = s.aimed.is_some()
        && (hp > s.before || party_hp(api, s.giver) < s.giver_before || text.contains("EFFECT"));
    let gave_up = s.aimed.is_some_and(|at| s.ticks - at > AIM_BUDGET);
    // "Not healthy enough." is `.softboiled` refusing the giver before any target is asked for.
    if s.ticks > TICK_BUDGET || landed || gave_up || text.contains("HEALTHY") {
        s.leaving = true;
    }
    if !s.press {
        agent.set_state(AgentState::Softboiling(SoftboiledState { press: true, ..s }));
        return Ok(());
    }

    let (top_x, top_y, current, _) = api.menu_geometry();
    // The party menu `ItemUseMedicine` brings back asks which Pokémon to use the item on; the
    // giver's own says to choose one, and an A there would choose the giver again.
    let asks_for_a_target = top_x == 0 && (top_y == 1 || top_y == 3) && text.contains("USE ITEM");
    let button = if s.leaving {
        Some(JoypadButton::B)
    } else if game_mode == GameMode::Overworld {
        Some(JoypadButton::Start)
    } else if asks_for_a_target {
        // A is pressed again until the HP moves: a menu takes no press on the frame it is drawn.
        s.chosen = true;
        Some(match current.cmp(&s.target) {
            std::cmp::Ordering::Less => JoypadButton::Down,
            std::cmp::Ordering::Greater => JoypadButton::Up,
            std::cmp::Ordering::Equal => {
                s.aimed.get_or_insert(s.ticks);
                JoypadButton::A
            }
        })
    } else if s.chosen {
        None
    } else {
        Some(field_move_menu_button(api, s.giver, s.move_index))
    };
    // A per-tick trace, off unless `SOFTBOILED_TRACE` is set.
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *TRACE.get_or_init(|| std::env::var("SOFTBOILED_TRACE").is_ok()) {
        println!("[softboiled] t{} mode {game_mode:?} geom ({top_x},{top_y}) cur {current} hp {hp} {s:?} text {:?} -> {button:?}",
                 s.ticks, text.chars().take(40).collect::<String>());
    }
    if let Some(button) = button {
        api.press_button(button);
    }
    agent.set_state(AgentState::Softboiling(SoftboiledState { press: button.is_none(), ..s }));
    Ok(())
}

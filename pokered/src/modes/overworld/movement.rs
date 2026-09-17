//! Moving sprites for a script: `MoveSprite`, `RunNPCMovementScript`'s tables, `FindPathToPlayer`,
//! `EmotionBubble`, and the player's simulated presses.

use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::{pokered_symbols, DmgPointer};
use poke_core::symbols::pokered_toggles::TOGGLE_PALLET_TOWN_OAK;
use crate::gfx::layers::Object;
use crate::gfx::tiles::V_CHARS1;
use crate::input::Joypad;
use crate::mode::Ctx;
use crate::scripts;
use poke_core::map_objects::STAY;
use crate::systems::overworld::sprites::Sprites;
use super::script::{Flow, Routine, Script, Then};
use super::{Overworld, PAD_CTRL_PAD};

/// `EmotionBubble`'s `DelayFrames 60`.
pub(super) const EMOTION_BUBBLE_FRAMES: u8 = 60;
pub(crate) const NPC_MOVEMENT_DOWN: u8 = 0x00;
pub(crate) const NPC_MOVEMENT_UP: u8 = 0x40;
pub(crate) const NPC_MOVEMENT_LEFT: u8 = 0x80;
pub(crate) const NPC_MOVEMENT_RIGHT: u8 = 0xC0;
/// `NONE`, movement byte 2 of a sprite standing still.
const NONE: u8 = 0xFF;
/// `PALLET_MOVEMENT_SCRIPT`'s entry in `.NPCMovementScriptPointerTables`, counted from 1, and the
/// two Pewter guides' beside it.
pub const PALLET_MOVEMENT_SCRIPT: u8 = 1;
pub const PEWTER_MUSEUM_GUY_MOVEMENT_SCRIPT: u8 = 2;
pub const PEWTER_GYM_GUY_MOVEMENT_SCRIPT: u8 = 3;

impl Overworld {
    /// `RunMapScript`: `TryPushingBoulder`, the dust it leaves, `RunNPCMovementScript`, then the
    /// map's. The dust waits for the boulder to finish its square, which is what the check on
    /// `BIT_SCRIPTED_NPC_MOVEMENT` comes to.
    pub(super) fn run_map_script(&mut self, ctx: &mut Ctx) -> Flow {
        self.try_pushing_boulder(ctx);
        if self.rt.boulder_dust && !self.rt.paths.scripted_npc_movement {
            return Then::call(Routine::DoBoulderDustAnimation).then(Routine::RunNpcMovementScript);
        }
        self.run_npc_movement_script(ctx)
    }

    /// `RunNPCMovementScript` and the map's own script after it.
    pub(super) fn run_npc_movement_script(&mut self, ctx: &mut Ctx) -> Flow {
        if std::mem::take(&mut self.standing_on_door) {
            self.player_step_out_from_door(ctx);
        } else {
            match self.rt.npc_movement_script_table {
                0 => {}
                PALLET_MOVEMENT_SCRIPT => self.pallet_movement_script(ctx),
                PEWTER_MUSEUM_GUY_MOVEMENT_SCRIPT => self.pewter_movement_script(ctx, 0),
                PEWTER_GYM_GUY_MOVEMENT_SCRIPT => self.pewter_movement_script(ctx, 1),
                _ => {}
            }
        }
        let map = ctx.world.location.map;
        scripts::script(map, &mut Script { ow: self, ctx })
    }

    /// `SetSpriteMovementBytesToFF`.
    pub(super) fn set_sprite_movement_bytes_to_ff(&mut self, slot: u8) {
        let sprite = &mut self.sprites[slot as usize];
        sprite.movement1 = STAY;
        sprite.movement2 = NONE;
    }

    /// `MoveSprite_`: the path copied for `UpdateNPCSprite` to walk, and the player's input locked.
    pub(super) fn move_sprite(&mut self, ctx: &mut Ctx, slot: u8, directions: &[u8]) {
        self.sprites[slot as usize].movement1 = 0;
        let end = directions.iter().position(|&d| d == STAY).map_or(directions.len(), |i| i + 1);
        self.rt.paths.directions = directions[..end].to_vec();
        self.rt.paths.num_scripted_steps = end as u8;
        self.rt.paths.scripted_npc_movement = true;
        self.rt.override_simulated = Joypad::empty();
        if let Some(first) = self.simulated.first_mut() {
            *first = Joypad::empty();
        }
        ctx.pad.ignore = Joypad::all();
    }

    /// `EmotionBubble` up to its `DelayFrames 60`: the bubble's tiles, the objects moved four along to
    /// make room at the front, and OAM held still.
    pub(super) fn emotion_bubble(&mut self, ctx: &mut Ctx, slot: u8, bubble: u8) {
        let tiles = rom_slice(pokered_symbols::EmotionBubbles + bubble as u16 * 4 * TILE_BYTES as u16);
        ctx.screen.tiles.load(V_CHARS1 + 0x78, &tiles[..4 * TILE_BYTES]);
        self.rt.sprites_frozen = true;
        let objects = &mut ctx.screen.sprites;
        objects.resize(40, Object { y: 160, ..Object::default() });
        let last = if self.jumping { 31 } else { 35 };
        for i in (0..=last).rev() {
            objects[i + 4] = objects[i];
        }
        let sprite = &self.sprites[slot as usize];
        let (y, x) = (sprite.y_pixels, sprite.x_pixels.wrapping_add(8));
        for (i, (dy, dx)) in [(0, 0), (0, 8), (8, 0), (8, 8)].into_iter().enumerate() {
            objects[i] = Object { y: y.wrapping_add(dy), x: x.wrapping_add(dx), tile: 0xF8 + i as u8, attributes: 0 };
        }
    }

    /// `StartSimulatingJoypadStates`, with `presses` as `wSimulatedJoypadStatesEnd` holds them: the
    /// last is pressed first.
    pub(super) fn start_simulating_joypad_states(&mut self, presses: Vec<Joypad>) {
        self.simulated_index = presses.len() as u8;
        self.simulated = presses;
        self.rt.override_simulated = Joypad::empty();
        self.sprites[0].movement1 = 0;
        self.scripted = true;
    }

    /// `PalletMovementScriptPointerTable`: Oak leads the player from the edge of town into his lab.
    fn pallet_movement_script(&mut self, ctx: &mut Ctx) {
        let oak = self.rt.sprite_index;
        match self.rt.npc_movement_script_function {
            // `PalletMovementScript_OakMoveLeft`.
            0 => {
                let steps = ctx.world.location.x.wrapping_sub(10);
                self.rt.num_steps_to_take = steps;
                if steps == 0 {
                    self.rt.npc_movement_script_function = 3;
                } else {
                    let mut path = vec![NPC_MOVEMENT_LEFT; steps as usize];
                    path.push(STAY);
                    self.rt.paths.directions2 = path.clone();
                    self.set_sprite_movement_bytes_to_ff(oak);
                    self.move_sprite(ctx, oak, &path);
                    self.rt.npc_movement_script_function = 1;
                }
                self.rt.no_map_music = true;
                ctx.pad.ignore = Joypad::SELECT | Joypad::START | PAD_CTRL_PAD;
            }
            // `PalletMovementScript_PlayerMoveLeft`.
            1 => {
                if self.rt.paths.scripted_npc_movement {
                    return;
                }
                let steps = self.rt.num_steps_to_take;
                self.rt.paths.directions2_index = steps;
                let presses = convert_npc_movement_directions_to_joypad_masks(&self.rt.paths.directions2, steps);
                self.start_simulating_joypad_states(presses);
                self.rt.npc_movement_script_function = 2;
            }
            // `PalletMovementScript_WaitAndWalkToLab`, falling into `_WalkToLab`.
            2 if self.simulated_index != 0 => {}
            2 | 3 => {
                self.rt.override_simulated = Joypad::empty();
                self.rt.paths.script_sprite = oak;
                self.sprites[0].movement1 = 0;
                let presses = decode_rle_list(pokered_symbols::RLEList_PlayerWalkToLab);
                self.simulated_index = presses.len() as u8 - 1;
                self.simulated = presses.into_iter().map(Joypad::from_bits_truncate).collect();
                self.rt.paths.directions2 = decode_rle_list(pokered_symbols::RLEList_ProfOakWalkToLab);
                self.rt.paths.init_scripted_movement = false;
                self.scripted = true;
                self.rt.npc_movement_script_function = 4;
            }
            // `PalletMovementScript_Done`.
            _ => {
                if self.simulated_index != 0 {
                    return;
                }
                self.toggle_object(ctx, TOGGLE_PALLET_TOWN_OAK, true);
                self.scripted = false;
                self.rt.paths.init_scripted_movement = false;
                self.end_npc_movement_script();
            }
        }
    }

    /// `PewterMuseumGuyMovementScriptPointerTable` and `PewterGymGuyMovementScriptPointerTable`:
    /// a guide walks the player to the museum or the gym, `which` saying to which. The player's
    /// presses are the same every time, so `PewterGuys` prefixes the ones that line them up first.
    fn pewter_movement_script(&mut self, ctx: &mut Ctx, which: u8) {
        if self.rt.npc_movement_script_function != 0 {
            // `PewterMovementScript_Done`.
            if self.simulated_index == 0 {
                self.end_npc_movement_script();
            }
            return;
        }
        let (player, guide) = match which {
            0 => (pokered_symbols::RLEList_PewterMuseumPlayer, pokered_symbols::RLEList_PewterMuseumGuy),
            _ => (pokered_symbols::RLEList_PewterGymPlayer, pokered_symbols::RLEList_PewterGymGuy),
        };
        ctx.audio.play_music(crate::audio::data::sounds::MUSIC_MUSEUM_GUY);
        self.rt.paths.script_sprite = self.rt.sprite_index;
        if which == 1 {
            self.sprites[0].movement1 = 0;
        }
        let presses = decode_rle_list(player);
        self.start_simulating_joypad_states(presses.into_iter().map(Joypad::from_bits_truncate).collect());
        self.simulated_index -= 1;
        let mut presses: Vec<u8> = self.simulated.iter().map(|press| press.bits()).collect();
        presses.truncate(self.simulated_index as usize);
        crate::systems::events::tables::pewter_guys(which, ctx.world.location.x, ctx.world.location.y, &mut presses);
        self.simulated_index = presses.len() as u8;
        self.simulated = presses.into_iter().map(Joypad::from_bits_truncate).collect();
        self.rt.paths.directions2 = decode_rle_list(guide);
        self.rt.paths.init_scripted_movement = false;
        self.rt.npc_movement_script_function = 1;
    }

    /// `_EndNPCMovementScript`.
    fn end_npc_movement_script(&mut self) {
        self.scripted = false;
        self.rt.paths.init_scripted_movement = false;
        self.standing_on_door = false;
        self.exiting_door = false;
        self.rt.paths.script_sprite = 0;
        self.rt.npc_movement_script_table = 0;
        self.rt.npc_movement_script_function = 0;
        self.simulated_index = 0;
        if let Some(first) = self.simulated.first_mut() {
            *first = Joypad::empty();
        }
    }
}

/// `DecodeRLEList`: pairs of a value and a count, with the list's `$ff` kept at the end.
pub(super) fn decode_rle_list(at: DmgPointer) -> Vec<u8> {
    let bytes = rom_slice(at);
    let mut list: Vec<u8> = bytes.chunks(2).take_while(|pair| pair[0] != 0xFF)
        .flat_map(|pair| std::iter::repeat_n(pair[0], pair[1] as usize))
        .collect();
    list.push(0xFF);
    list
}

/// `ConvertNPCMovementDirectionsToJoypadMasks`: the first `count` directions as presses, reversed,
/// since the simulated presses are read from the end.
fn convert_npc_movement_directions_to_joypad_masks(directions: &[u8], count: u8) -> Vec<Joypad> {
    directions[..count as usize].iter().rev().map(|&direction| match direction {
        NPC_MOVEMENT_UP => Joypad::UP,
        NPC_MOVEMENT_DOWN => Joypad::DOWN,
        NPC_MOVEMENT_LEFT => Joypad::LEFT,
        NPC_MOVEMENT_RIGHT => Joypad::RIGHT,
        other => Joypad::from_bits_truncate(other),
    }).collect()
}

/// `CalcPositionOfPlayerRelativeToNPC`, the Y distance adjusted, then `FindPathToPlayer`.
pub(super) fn find_path_to_player(sprites: &Sprites, slot: u8, perspective: bool, adjust_y: i8) -> Vec<u8> {
    let (player, npc) = (&sprites[0], &sprites[slot as usize]);
    // `CalcDifference` sets carry when the NPC's coordinate is the smaller: the NPC is above or left.
    let (y_distance, npc_above) = (player.y_pixels.abs_diff(npc.y_pixels), npc.y_pixels < player.y_pixels);
    let (x_distance, npc_left) = (player.x_pixels.abs_diff(npc.x_pixels), npc.x_pixels < player.x_pixels);
    let mut player_lower_y = npc_above;
    let mut player_lower_x = npc_left;
    if perspective {
        player_lower_y = !player_lower_y;
        player_lower_x = !player_lower_x;
    }
    let y_distance = (y_distance / 16).wrapping_add(adjust_y as u8);
    let x_distance = x_distance / 16;
    let (mut y_progress, mut x_progress) = (0u8, 0u8);
    let mut path = Vec::new();
    loop {
        let d = y_progress.abs_diff(y_distance);
        let e = x_progress.abs_diff(x_distance);
        if d == 0 && e == 0 {
            break;
        }
        if e >= d {
            path.push(if player_lower_x { NPC_MOVEMENT_LEFT } else { NPC_MOVEMENT_RIGHT });
            x_progress += 1;
        } else {
            path.push(if player_lower_y { NPC_MOVEMENT_UP } else { NPC_MOVEMENT_DOWN });
            y_progress += 1;
        }
    }
    path.push(STAY);
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::systems::overworld::sprites::SpriteState;

    #[test]
    fn the_player_s_walk_to_the_lab_is_read_from_its_end() {
        let presses = decode_rle_list(pokered_symbols::RLEList_PlayerWalkToLab);
        assert_eq!(presses.len(), 2 + 3 + 5 + 1 + 6 + 1);
        assert_eq!((presses[0], presses[16], presses[17]), (Joypad::UP.bits(), Joypad::DOWN.bits(), 0xFF));
    }

    #[test]
    fn a_path_takes_the_longer_axis_first_and_alternates() {
        let mut sprites = [SpriteState::default(); 16];
        sprites[0].y_pixels = 0x3C;
        sprites[0].x_pixels = 0x40;
        sprites[1].y_pixels = 0x3C + 0x40;
        sprites[1].x_pixels = 0x40 + 0x10;
        // Oak below and right, seen from his side: the player is up and to his left.
        let path = find_path_to_player(&sprites, 1, true, -1);
        assert_eq!(path, [NPC_MOVEMENT_UP, NPC_MOVEMENT_UP, NPC_MOVEMENT_LEFT, NPC_MOVEMENT_UP, STAY]);
    }
}

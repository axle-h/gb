//! The battle: the cartridge hijacked into `InitBattle` from the Celadon fixture's start menu, and the
//! recreation's `BattleMode` from the same party, fed the same presses and the same random bytes.
//! The cartridge runs twice: once alone for its random bytes, since the recreation draws a turn's
//! bytes before it shows the turn, and once beside the recreation, compared at every poll.

use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::{RAM, ROM};
use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::{Mode, Status};
use pokered::modes::battle::BattleMode;
use pokered::modes::menu_input::CursorMemory;
use pokered::rng::GameRng;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer, DmgPointerRead};
use super::learn_move::hijack;
use super::{assert_late, breakpoint, joypad, open_the_start_menu, ARROW, BOX, CURSOR, DELAY3};
use super::status_screen::{ours, screen, the_party, the_world};

/// `OPP_ID_OFFSET`: `wCurOpponent` at or above it is a trainer class.
const OPP_ID_OFFSET: u8 = 200;
pub(super) const YOUNGSTER: u8 = 1;
pub(super) const POKEMON_TOWER_3F: u8 = 0x90;
pub(super) const SAFARI_ZONE_EAST: u8 = 0xD9;
const PARTY_STRUCT: u16 = 0x2C;
const BOX_STRUCT: u16 = 0x21;
const NAME_LENGTH: u16 = 11;

/// `wBoxMons` and the names beside them: the current box as WRAM holds it.
fn the_box(gb: &GameBoy) -> Vec<pokered::party::Named<pokered::party::BoxMon>> {
    let party: Vec<_> = the_party(gb);
    let _ = party;
    let mmu = gb.core().mmu();
    let name = |at: u16| (0..NAME_LENGTH).map(|i| mmu.read(at + i)).take_while(|&byte| byte != 0x50).collect::<Vec<u8>>();
    (0..mmu.read(sym::wBoxCount.address) as u16).map(|slot| {
        let b: Vec<u8> = (0..BOX_STRUCT).map(|i| mmu.read(sym::wBoxMons.address + BOX_STRUCT * slot + i)).collect();
        let word = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
        let mon = pokered::party::BoxMon {
            species: PokemonSpecies::from_repr(b[0]).expect("a species"),
            hp: word(1),
            box_level: b[3],
            status: b[4],
            types: [b[5], b[6]],
            catch_rate: b[7],
            moves: [0, 1, 2, 3].map(|i| poke_core::move_name::PokemonMoveName::from_repr(b[8 + i])),
            ot_id: word(12),
            exp: u32::from_be_bytes([0, b[14], b[15], b[16]]),
            stat_exp: [0, 1, 2, 3, 4].map(|i| word(17 + 2 * i)),
            dvs: pokered::systems::stats::Dvs([b[27], b[28]]),
            pp: [b[29], b[30], b[31], b[32]],
        };
        pokered::party::Named { mon, ot: name(sym::wBoxMonOT.address + NAME_LENGTH * slot),
            nick: name(sym::wBoxMonNicks.address + NAME_LENGTH * slot) }
    }).collect()
}

/// `vBackPic`.
const V_BACK_PIC: u16 = 0x9310;
const SAFARI_ZONE_CENTER_REST_HOUSE: u8 = 0xDD;

/// A screen row as letters, for reading a run.
pub(super) fn letters(row: &[u8]) -> String {
    row.iter().map(|&tile| match tile {
        0x80..=0x99 => (b'A' + tile - 0x80) as char,
        0xA0..=0xB9 => (b'a' + (tile - 0xA0)) as char,
        0xF6..=0xFF => (b'0' + (tile - 0xF6)) as char,
        0x7F => ' ',
        0xE7 => '!',
        0xE8 => '.',
        0xEE => 'v',
        0xED => '>',
        0x7C | 0x7B => '|',
        0x79 | 0x7A | 0x7D | 0x7E => '+',
        0x6D => ':',
        0x71 => 'L',
        _ => '#',
    }).collect()
}

/// The routines both sides skip: the transition into the battle, and every move's animation.
const SEAMS: [DmgPointer; 2] = [sym::BattleTransition, sym::MoveAnimation];

/// What is done to the Celadon party before the battle starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Lead {
    /// The lead's species and level, its stats left as they were.
    pub(super) species: Option<(PokemonSpecies, u8)>,
    /// The lead one point of experience short of its next level.
    pub(super) about_to_level: bool,
    /// The SHIFT battle style, which asks before a trainer's next mon.
    pub(super) shift: bool,
    /// `wCurMap` for the battle, which decides a ghost.
    pub(super) map: Option<u8>,
    /// The lead's HP, status and PP as written over the fixture's.
    pub(super) hp: Option<u16>,
    pub(super) status: Option<u8>,
    pub(super) pp: Option<[u8; 4]>,
    /// The bag, as written over the fixture's.
    pub(super) bag: Option<&'static [(ItemId, u8)]>,
    /// The party filled to six with copies of the lead, and the current box to this many.
    pub(super) full_party: bool,
    pub(super) box_count: Option<u8>,
    /// `BATTLE_TYPE_OLD_MAN`.
    pub(super) old_man: bool,
    /// The lead's moves, each at full PP.
    pub(super) moves: Option<[u8; 4]>,
    /// `wOptions`' battle animation setting, written over the fixture's.
    pub(super) animations: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Opponent {
    Wild(PokemonSpecies, u8),
    /// A trainer class and the number of its party, from 1.
    Trainer(u8, u8),
}

/// What one run of the cartridge to its next poll saw.
pub(super) struct Poll {
    pub frames: u32,
    pub screen: Vec<Vec<u8>>,
}

/// The cartridge in a battle, taping every `Random` byte that is not the VBlank handler's own and
/// returning from the seams unentered.
pub(super) struct Cartridge {
    pub gb: GameBoy,
    pub tape: Vec<u8>,
    /// Where `InitBattle` returns to, and the stack pointer it returns with.
    back: (Breakpoint, u16),
    /// `InitBattle` has returned.
    pub ended: bool,
    /// The routines returned from unentered.
    seams: Vec<DmgPointer>,
    /// The LCD's shades at every VBlank, and whether the cartridge was loading, while collecting.
    pub lcd: Option<Vec<(Vec<u8>, bool)>>,
}

impl Cartridge {
    /// The Celadon fixture's call of `DisplayStartMenu` turned into `InitBattle`.
    fn new(opponent: Opponent, lead: Lead) -> Self {
        Self { seams: SEAMS.to_vec(), ..Self::animated(opponent, lead) }
    }

    /// `new`, with every animation and the transition played.
    pub(super) fn animated(opponent: Opponent, lead: Lead) -> Self {
        let mut gb = open_the_start_menu();
        let mmu = gb.core_mut().mmu_mut();
        if let Some((species, level)) = lead.species {
            mmu.write(sym::wPartySpecies.address, species as u8);
            mmu.write(sym::wPartyMon1Species.address, species as u8);
            mmu.write(sym::wPartyMon1Level.address, level);
            mmu.write(sym::wPartyMon1Level.address - 0x21 + 3, level);
        }
        if let Some(hp) = lead.hp {
            mmu.write_slice(sym::wPartyMon1HP.address, &hp.to_be_bytes());
        }
        if let Some(status) = lead.status {
            mmu.write(sym::wPartyMon1Status.address, status);
        }
        if let Some(moves) = lead.moves {
            mmu.write_slice(sym::wPartyMon1Moves.address, &moves);
            let pp = moves.map(|id| if id == 0 { 0 } else { poke_core::moves::MoveData::of(id).pp });
            mmu.write_slice(sym::wPartyMon1PP.address, &pp);
        }
        if let Some(pp) = lead.pp {
            mmu.write_slice(sym::wPartyMon1PP.address, &pp);
        }
        if let Some(bag) = lead.bag {
            let mut bytes = vec![0xFF; 41];
            for (i, &(item, quantity)) in bag.iter().enumerate() {
                bytes[2 * i] = item as u8;
                bytes[2 * i + 1] = quantity;
            }
            mmu.write(sym::wNumBagItems.address, bag.len() as u8);
            mmu.write_slice(sym::wBagItems.address, &bytes);
        }
        if lead.old_man {
            mmu.write(sym::wBattleType.address, 1);
        }
        if lead.full_party {
            let count = mmu.read(sym::wPartyCount.address) as u16;
            for slot in count..6 {
                mmu.write(sym::wPartySpecies.address + slot, mmu.read(sym::wPartySpecies.address));
                for i in 0..PARTY_STRUCT {
                    mmu.write(sym::wPartyMons.address + PARTY_STRUCT * slot + i, mmu.read(sym::wPartyMons.address + i));
                }
                for i in 0..NAME_LENGTH {
                    mmu.write(sym::wPartyMonOT.address + NAME_LENGTH * slot + i, mmu.read(sym::wPartyMonOT.address + i));
                    mmu.write(sym::wPartyMonNicks.address + NAME_LENGTH * slot + i, mmu.read(sym::wPartyMonNicks.address + i));
                }
            }
            mmu.write(sym::wPartySpecies.address + 6, 0xFF);
            mmu.write(sym::wPartyCount.address, 6);
        }
        if let Some(count) = lead.box_count {
            // Copies of the box's first mon, or of the lead's front half for an empty box.
            for slot in 0..count as u16 {
                mmu.write(sym::wBoxCount.address + 1 + slot, mmu.read(sym::wPartySpecies.address));
                for i in 0..BOX_STRUCT {
                    mmu.write(sym::wBoxMons.address + BOX_STRUCT * slot + i, mmu.read(sym::wPartyMons.address + i));
                }
                for i in 0..NAME_LENGTH {
                    mmu.write(sym::wBoxMonOT.address + NAME_LENGTH * slot + i, mmu.read(sym::wPartyMonOT.address + i));
                    mmu.write(sym::wBoxMonNicks.address + NAME_LENGTH * slot + i, mmu.read(sym::wPartyMonNicks.address + i));
                }
            }
            mmu.write(sym::wBoxCount.address + 1 + count as u16, 0xFF);
            mmu.write(sym::wBoxCount.address, count);
        }
        if let Some(map) = lead.map {
            mmu.write(sym::wCurMap.address, map);
            mmu.write(sym::wNumSafariBalls.address, 30);
        }
        if let Some(on) = lead.animations {
            let options = mmu.read(sym::wOptions.address);
            mmu.write(sym::wOptions.address, if on { options & !(1 << 7) } else { options | 1 << 7 });
        }
        if lead.shift {
            let options = mmu.read(sym::wOptions.address);
            mmu.write(sym::wOptions.address, options & !(1 << 6));
        }
        if lead.about_to_level {
            let species = PokemonSpecies::from_repr(mmu.read(sym::wPartyMon1Species.address)).expect("a species");
            let level = mmu.read(sym::wPartyMon1Level.address);
            let growth = poke_core::base_stats::BaseStats::of(species).growth_rate;
            let exp = pokered::systems::experience::calc_experience(growth, level + 1) - 1;
            for (i, byte) in exp.to_be_bytes()[1..].iter().enumerate() {
                mmu.write(sym::wPartyMon1Exp.address + i as u16, *byte);
            }
        }
        match opponent {
            Opponent::Wild(species, level) => {
                mmu.write(sym::wCurOpponent.address, species as u8);
                mmu.write(sym::wCurEnemyLevel.address, level);
            }
            Opponent::Trainer(class, number) => {
                mmu.write(sym::wCurOpponent.address, OPP_ID_OFFSET + class);
                mmu.write(sym::wTrainerNo.address, number);
            }
        }
        let (sp, address) = (gb.core().registers().sp, gb.return_address());
        let bank = if (0x4000..0x8000).contains(&address) { gb.core().mmu().rom_bank() as u8 } else { 0 };
        hijack(&mut gb, sym::InitBattle);
        Self { gb, tape: vec![], back: (Breakpoint::new(bank, address), sp + 2), ended: false, seams: vec![], lcd: None }
    }

    /// Runs to the next VBlank, or with `to_poll` to the VBlank after the next poll, counting frames.
    fn run(&mut self, to_poll: bool) -> u32 {
        let (poll, vblank, random, end) = (breakpoint(sym::JoypadLowSensitivity), breakpoint(sym::VBlank),
            breakpoint(sym::Random), self.back.0);
        let seams: Vec<_> = self.seams.iter().map(|&seam| breakpoint(seam)).collect();
        let mut points = vec![poll, vblank, random, end];
        points.extend(&seams);
        let mut polled = !to_poll;
        let mut frames = 0;
        loop {
            let (stop, _) = self.gb.run_until(&points, MachineCycles::PER_FRAME * 600);
            let Stop::Breakpoint(hit) = stop else { panic!("the cartridge stopped: {stop:?}") };
            if hit == poll {
                // The evolution only looks for B, which the recreation's evolution does not wait on.
                let cancel = sym::Evolution_CheckForCancel.address;
                polled |= !(cancel..cancel + 0x10).contains(&self.gb.return_address());
            } else if hit == vblank {
                frames += 1;
                if let Some(lcd) = self.lcd.as_mut() {
                    lcd.push((super::battle_animations::lcd_shades(&self.gb), super::battle_animations::loading(&self.gb)));
                }
                if polled {
                    return frames;
                }
            } else if hit == random {
                let caller = self.gb.return_address();
                let (stop, _) = self.gb.run_to_return(MachineCycles::PER_FRAME * 10);
                assert!(matches!(stop, Stop::Returned { .. }));
                if !is_vblank_caller(caller) {
                    self.tape.push(self.gb.core().registers().a);
                    if std::env::var("LOG_RANDOM").is_ok() {
                        println!("    random #{} = {} from {}", self.tape.len(), self.gb.core().registers().a,
                            nearest_label(&self.gb, caller));
                    }
                }
            } else if hit == end {
                if self.gb.core().registers().sp == self.back.1 {
                    self.ended = true;
                    return frames;
                }
            } else {
                let sp = self.gb.core().registers().sp;
                let back = self.gb.core().mmu().read_u16_le(sp);
                let registers = self.gb.core_mut().registers_mut();
                registers.sp = sp + 2;
                registers.pc = back;
            }
            assert!(frames < 5000, "the cartridge never polled");
        }
    }

    /// The next poll, or `None` once the battle is over.
    pub(super) fn to_poll(&mut self) -> Option<Poll> {
        if self.ended {
            return None;
        }
        let frames = self.run(true);
        (!self.ended).then(|| Poll { frames, screen: screen(&self.gb) })
    }

    /// `button` for a frame: often answered within it.
    pub(super) fn press(&mut self, button: Joypad) {
        self.gb.hold_buttons(joypad(button));
        self.run(false);
        self.gb.hold_buttons(JoypadButtonState::default());
    }
}

fn is_vblank_caller(caller: u16) -> bool {
    let vblank = sym::VBlank.address;
    (vblank..vblank + 0x80).contains(&caller)
}

/// `wShadowOAM`, as `VBlank` copies it out.
pub(super) fn cartridge_oam(gb: &GameBoy) -> Vec<[u8; 4]> {
    let at = sym::wShadowOAM.address;
    (0..40).map(|i| std::array::from_fn(|j| gb.core().mmu().read(at + 4 * i + j as u16))).collect()
}

/// The objects on screen, in OAM order.
pub(super) fn visible_objects(oam: &[[u8; 4]]) -> Vec<[u8; 4]> {
    oam.iter().filter(|object| (1..160).contains(&object[0]) && (1..168).contains(&object[1])).copied().collect()
}

fn word(gb: &GameBoy, at: u16) -> u16 {
    u16::from_be_bytes([gb.core().mmu().read(at), gb.core().mmu().read(at + 1)])
}

/// The recreation from the cartridge's world, drawing the cartridge's random bytes, skipping what
/// `SEAMS` skips.
fn recreation(gb: &GameBoy, opponent: Opponent, lead: Lead, tape: Vec<u8>) -> Game {
    recreation_with(gb, opponent, lead, tape, true)
}

/// `recreation`, with the seams played or not.
pub(super) fn recreation_with(gb: &GameBoy, opponent: Opponent, lead: Lead, tape: Vec<u8>, skip_seams: bool) -> Game {
    let mmu = gb.core().mmu();
    let mut world = the_world(gb);
    world.player_id = word(gb, sym::wPlayerID.address);
    world.badges = mmu.read_pointer(&sym::wObtainedBadges);
    world.bag = super::item_menu::the_bag(gb);
    world.pokedex.owned = std::array::from_fn(|i| mmu.read(sym::wPokedexOwned.address + i as u16));
    world.pokedex.seen = std::array::from_fn(|i| mmu.read(sym::wPokedexSeen.address + i as u16));
    world.location.map = poke_core::map::Map::from_repr(mmu.read_pointer(&sym::wCurMap)).expect("a map");
    world.safari_balls = mmu.read_pointer(&sym::wNumSafariBalls);
    world.current_box = mmu.read_pointer(&sym::wCurrentBoxNum) & 0x7F;
    world.boxes = vec![Vec::new(); world.current_box as usize];
    world.boxes.push(the_box(gb));
    world.money = [0, 1, 2].map(|i| mmu.read(sym::wPlayerMoney.address + i));
    world.rival_name = (0..7).map(|i| mmu.read(sym::wRivalName.address + i)).take_while(|&byte| byte != 0x50).collect();
    let mut game = Game::new(world, GameRng::tape(tape), Pacing::Faithful);
    *game.menu_mut() = CursorMemory {
        battle_and_start: mmu.read_pointer(&sym::wBattleAndStartSavedMenuItem),
        ..CursorMemory::default()
    };
    let mode = match opponent {
        Opponent::Wild(species, level) if lead.old_man => BattleMode::old_man(species, level),
        Opponent::Wild(species, level) => BattleMode::wild(species, level),
        Opponent::Trainer(class, number) => BattleMode::trainer(class, number,
            mmu.read_pointer(&sym::wLoneAttackNo), mmu.read_pointer(&sym::wRivalStarter)),
    };
    let mode = if skip_seams {
        mode.without_move_animations()
    } else {
        super::battle_animations::seed_screen(gb, &mut game);
        mode
    };
    game.push(Mode::Battle(mode));
    game
}

/// Frames until the recreation waits, and for what; `None` once the battle has popped.
pub(super) fn recreation_to_poll(game: &mut Game) -> (u32, Option<Decision>) {
    for frames in 1..5000 {
        game.frame(Input::None);
        if !game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_))) {
            return (frames, None);
        }
        if let Status::Waiting(decision) = game.status() {
            return (frames, Some(decision));
        }
    }
    panic!("the recreation never waited");
}

fn press_both(cartridge: &mut Cartridge, game: &mut Game, button: Joypad) {
    game.frame(Input::Buttons(button));
    cartridge.press(button);
    game.frame(Input::None);
}

/// The battle state both keep, as the cartridge lays it out: HP, status, PP and the volatile bytes
/// for each side.
fn cartridge_state(gb: &GameBoy) -> Vec<u16> {
    let mmu = gb.core().mmu();
    let byte = |pointer: DmgPointer| mmu.read(pointer.address) as u16;
    let bytes = |pointer: DmgPointer, n: u16| (0..n).map(move |i| mmu.read(pointer.address + i) as u16);
    let mut state = vec![word(gb, sym::wBattleMonHP.address), byte(sym::wBattleMonStatus)];
    state.extend(bytes(sym::wBattleMonPP, 4));
    state.extend(bytes(sym::wBattleMonMoves, 4));
    state.extend([word(gb, sym::wEnemyMonHP.address), byte(sym::wEnemyMonStatus)]);
    state.extend(bytes(sym::wEnemyMonPP, 4));
    state.extend(bytes(sym::wPlayerBattleStatus1, 3));
    state.extend(bytes(sym::wEnemyBattleStatus1, 3));
    state
}

fn recreation_state(game: &Game) -> Vec<u16> {
    let Some(Mode::Battle(mode)) = game.modes().last() else { panic!("the battle is up") };
    let battle = mode.battle().expect("started");
    let mut state = vec![battle.player.mon.hp, battle.player.mon.status as u16];
    state.extend(battle.player.mon.pp.map(u16::from));
    state.extend(battle.player.mon.moves.map(|name| name.map_or(0, |name| name as u16)));
    state.extend([battle.enemy.mon.hp, battle.enemy.mon.status as u16]);
    state.extend(battle.enemy.mon.pp.map(u16::from));
    state.extend([battle.player.status1.bits(), battle.player.status2.bits(), battle.player.status3.bits()].map(u16::from));
    state.extend([battle.enemy.status1.bits(), battle.enemy.status2.bits(), battle.enemy.status3.bits()].map(u16::from));
    state
}

/// Both sides through a battle, pressing `presses` in turn at each poll: the screens must match at
/// every poll, the battle at each turn's menu, and the party at the end. Returns each poll's frames,
/// the cartridge's and the recreation's.
fn battle(opponent: Opponent, lead: Lead, presses: &[Joypad], loadings: Option<&[u32]>) -> Vec<(u32, u32)> {
    let mut cartridge = Cartridge::new(opponent, lead);
    for &button in presses {
        cartridge.to_poll().expect("the battle polls for every press");
        cartridge.press(button);
    }
    assert!(cartridge.to_poll().is_none(), "the battle ends after the last press");

    let tape = std::mem::take(&mut cartridge.tape);
    let mut cartridge = Cartridge::new(opponent, lead);
    let mut game = recreation(&cartridge.gb, opponent, lead, tape);
    let mut frames = vec![];
    for (poll, &button) in presses.iter().enumerate() {
        let theirs = cartridge.to_poll().expect("polls");
        let (recreation, decision) = recreation_to_poll(&mut game);
        println!("poll {poll} {decision:?}: cartridge {} frames, recreation {recreation}", theirs.frames);
        if std::env::var("LOG_RANDOM").is_ok() {
            let cursor = serde_json::to_value(&game).ok().and_then(|value| value["rng"]["Tape"]["cursor"].as_u64());
            println!("    random bytes {} and {}", cartridge.tape.len(), cursor.unwrap_or_default());
        }
        let mine = ours(&game);
        if theirs.screen != mine || std::env::var("SHOW").is_ok() {
            for (a, b) in theirs.screen.iter().zip(&mine) {
                println!("  |{}|  |{}|", letters(a), letters(b));
            }
            for (y, (a, b)) in theirs.screen.iter().zip(&mine).enumerate().filter(|(_, (a, b))| a != b) {
                println!("  row {y}: {a:02x?}\n         {b:02x?}");
            }
        }
        assert_eq!(theirs.screen, mine, "the screen at poll {poll}");
        // The naming screen's mon icon is its own mode's to draw.
        if decision != Some(Decision::NamingScreen) {
            assert_eq!(visible_objects(&cartridge_oam(&cartridge.gb)), visible_objects(&game.screen().sprites.iter()
            .map(|object| [object.y, object.x, object.tile, object.attributes]).collect::<Vec<_>>()), "the objects at poll {poll}");
        }
        // No mon is sent out in the Safari Zone, so the player's battle mon is whatever was left.
        let safari = lead.old_man || lead.map.is_some_and(|map| (SAFARI_ZONE_EAST..SAFARI_ZONE_CENTER_REST_HOUSE).contains(&map));
        if decision == Some(Decision::BattleMenu) && !safari {
            // `vBackPic`, as `LoadMonBackPic` decompressed and scaled it.
            let theirs: Vec<u8> = (0..49 * 16).map(|i| cartridge.gb.core().mmu().read(V_BACK_PIC + i)).collect();
            let mine: Vec<u8> = (0..49).flat_map(|i| *game.screen().tiles.bg(0x31 + i)).collect();
            let differing: Vec<usize> = (0..49).filter(|&t| theirs[t * 16..t * 16 + 16] != mine[t * 16..t * 16 + 16]).collect();
            assert!(differing.is_empty(), "the back pic at poll {poll}: tiles {differing:?} differ");
            assert_eq!(cartridge_state(&cartridge.gb), recreation_state(&game), "the battle at poll {poll}");
            // What a policy is shown of the battle, which is a reading of the same bytes.
            use crate::pokemon::battle::BattleStateReader;
            assert_eq!(format!("{:?}", cartridge.gb.core().mmu().read_battle_state()),
                       format!("{:?}", crate::pokemon::native::battle_state(&game)), "the policy's battle at poll {poll}");
        }
        if let Some(&loading) = loadings.and_then(|loadings| loadings.get(poll)) {
            assert_late(theirs.frames, recreation, loading, &format!("poll {poll}"));
        }
        frames.push((theirs.frames, recreation));
        press_both(&mut cartridge, &mut game, button);
    }
    assert!(cartridge.to_poll().is_none(), "the cartridge's battle ends");
    let (_, decision) = recreation_to_poll(&mut game);
    assert_eq!(decision, None, "the recreation's battle ends");
    assert_eq!(the_party(&cartridge.gb), game.world().party, "the party afterwards");
    assert_eq!(super::item_menu::the_bag(&cartridge.gb), game.world().bag, "the bag afterwards");
    let world = game.world();
    assert_eq!(the_box(&cartridge.gb), world.boxes[world.current_box as usize], "the current box afterwards");
    assert_eq!(cartridge.gb.core().mmu().read_pointer(&sym::wNumSafariBalls), world.safari_balls, "the Safari Balls afterwards");
    let mmu = cartridge.gb.core().mmu();
    let owned: [u8; 19] = std::array::from_fn(|i| mmu.read(sym::wPokedexOwned.address + i as u16));
    assert_eq!(owned, game.world().pokedex.owned, "the Pokédex afterwards");
    let mmu = cartridge.gb.core().mmu();
    let name: Vec<u8> = (0..NAME_LENGTH).map(|i| mmu.read(sym::wPlayerName.address + i)).take_while(|&byte| byte != 0x50).collect();
    assert_eq!(name, game.world().player_name, "the player's name afterwards");
    let money = [0, 1, 2].map(|i| cartridge.gb.core().mmu().read(sym::wPlayerMoney.address + i));
    assert_eq!(money, game.world().money, "the money afterwards");
    frames
}

/// A press for whatever the cartridge's screen shows: A through everything, except down the party
/// menu past a fainted lead, and down the move list every third poll.
pub(super) fn policy(screen: &[Vec<u8>], poll: usize) -> Joypad {
    let text: String = screen.iter().map(|row| letters(row)).collect::<Vec<_>>().join("/");
    if text.contains("Bring out which") {
        // Off a fainted mon: down while there is a row below, else up.
        if let Some(row) = (0..12).find(|&row| screen[row][0] == 0xED) {
            if letters(&screen[row]).contains("  0#") {
                let below = row + 2 < 12 && screen[row + 2][4] != 0x7F;
                return if below { Joypad::DOWN } else { Joypad::UP };
            }
        }
    }
    // The battle menu remembers ITEM, PKMN or RUN: back to FIGHT.
    if text.contains("FIGHT") && text.contains("RUN") {
        if screen[16][9] == 0xED {
            return Joypad::UP;
        }
        if screen[14][15] == 0xED || screen[16][15] == 0xED {
            return Joypad::LEFT;
        }
    }
    let move_menu = text.contains("TYPE#") || text.contains("|disabled!|");
    let no_pp = letters(&screen[11]).contains(" 0#");
    if move_menu && (poll % 3 == 0 || text.contains("|disabled!|") || no_pp) {
        return Joypad::DOWN;
    }
    Joypad::A
}

/// Both sides through a battle with the presses `choose` picks from the cartridge's screen.
fn battle_by(opponent: Opponent, lead: Lead, choose: impl Fn(&[Vec<u8>], usize) -> Joypad) -> Vec<Joypad> {
    battle_after(opponent, lead, &[], choose)
}

/// `battle_by`, with the first polls' frames bounded by `loadings`.
fn battle_timed(opponent: Opponent, lead: Lead, loadings: &[u32], choose: impl Fn(&[Vec<u8>], usize) -> Joypad) {
    let presses = battle_after(opponent, lead, &[], choose);
    battle(opponent, lead, &presses, Some(loadings));
}

/// `battle_by`, pressing `first` before `choose` takes over.
fn battle_after(opponent: Opponent, lead: Lead, first: &[Joypad], choose: impl Fn(&[Vec<u8>], usize) -> Joypad) -> Vec<Joypad> {
    let choose = |screen: &[Vec<u8>], poll: usize| first.get(poll).copied().unwrap_or_else(|| choose(screen, poll));
    let mut cartridge = Cartridge::new(opponent, lead);
    let mut presses = vec![];
    let mut already_out = false;
    while let Some(poll) = cartridge.to_poll() {
        let mut button = choose(&poll.screen, presses.len());
        // The mon out is refused, so the party menu after "already out!" moves off it.
        let text: String = poll.screen.iter().map(|row| letters(row)).collect();
        if already_out && text.contains("Bring out which") && button == Joypad::A {
            let row = (0..12).find(|&row| poll.screen[row][0] == 0xED).unwrap_or(0);
            button = if row + 2 < 12 && poll.screen[row + 2][4] != 0x7F { Joypad::DOWN } else { Joypad::UP };
        }
        already_out = text.contains("already out!");
        if std::env::var("SHOW_CARTRIDGE").is_ok() {
            println!("cartridge poll {} after {} frames, pressing {button:?}", presses.len(), poll.frames);
            for row in &poll.screen {
                println!("  |{}|", letters(row));
            }
        }
        presses.push(button);
        cartridge.press(button);
        assert!(presses.len() < 600, "the battle goes on");
    }
    battle(opponent, lead, &presses, None);
    presses
}

/// Loading the recreation leaves out, measured where it is the cartridge's own work rather than a
/// `Delay3`: `LoadHudAndHpBarAndStatusTilePatterns` and the two pictures decompressed.
pub(super) const PICTURES: u32 = 65;
/// `SlidePlayerAndEnemySilhouettesOnScreen`'s `Delay3`s either side of the slide.
pub(super) const SILHOUETTES: u32 = DELAY3 + DELAY3;
/// `LoadMonBackPic` for the fixture's lead, measured, and `SendOutMon`'s `Delay3` after it.
pub(super) const BACK_PICTURE: u32 = 32 + DELAY3;
/// `ClearScreen`'s `Delay3` and `_InitBattleCommon`'s own after it.
pub(super) const CLEAR_SCREEN: u32 = DELAY3 + DELAY3;
/// `PlayMoveAnimation`'s `Delay3` before the animation and `UpdateHPBar2`'s after the bar, each
/// with the frame the damage arithmetic runs past.
pub(super) const MOVE_AND_BAR: u32 = 2 * (DELAY3 + 1);

#[test]
fn a_wild_battle_won_with_one_move_matches_the_cartridge() {
    use Joypad as J;
    let loadings = [
        // Wild PIDGEY appeared!
        CLEAR_SCREEN / 2 + PICTURES + SILHOUETTES + BOX + ARROW,
        // Go! Celina!, which does not wait, and the battle menu with an empty text box behind it.
        BOX + CLEAR_SCREEN + BOX + BACK_PICTURE + BOX + BOX + CURSOR,
        // FIGHT: the move menu.
        DELAY3 + CURSOR,
        // TACKLE, the bar, the victory music's `Delay3`, and Enemy PIDGEY fainted!
        BOX + MOVE_AND_BAR + DELAY3 + BOX + ARROW,
        // The empty text after the faint, and Celina gained 23 EXP. Points!
        BOX + BOX + ARROW,
    ];
    let frames = battle(Opponent::Wild(PokemonSpecies::Pidgey, 3), Lead::default(), &[J::A; 5], Some(&loadings));
    println!("{frames:?}");
}

#[test]
fn running_from_a_wild_battle_matches_the_cartridge() {
    use Joypad as J;
    let loadings = [
        CLEAR_SCREEN / 2 + PICTURES + SILHOUETTES + BOX + ARROW,
        BOX + CLEAR_SCREEN + BOX + BACK_PICTURE + BOX + BOX + CURSOR,
        // RIGHT and DOWN, each a column or row of the battle menu.
        CURSOR,
        CURSOR,
        // RUN: Got away safely!
        BOX + ARROW,
    ];
    let presses = [J::A, J::RIGHT, J::DOWN, J::A, J::A];
    let frames = battle(Opponent::Wild(PokemonSpecies::Rattata, 5), Lead::default(), &presses, Some(&loadings));
    println!("{frames:?}");
}

#[test]
fn a_wild_battle_lost_sends_in_the_next_mon_and_blacks_out_as_the_cartridge_does() {
    battle_by(Opponent::Wild(PokemonSpecies::Mewtwo, 70), Lead::default(), policy);
}

#[test]
fn a_level_gained_in_battle_teaches_a_move_as_the_cartridge_does() {
    let lead = Lead { species: Some((PokemonSpecies::Ivysaur, 21)), about_to_level: true, ..Lead::default() };
    battle_by(Opponent::Wild(PokemonSpecies::Pidgey, 3), lead, policy);
}

#[test]
fn a_level_gained_in_battle_evolves_the_mon_afterwards_as_the_cartridge_does() {
    let lead = Lead { species: Some((PokemonSpecies::Ivysaur, 31)), about_to_level: true, ..Lead::default() };
    battle_by(Opponent::Wild(PokemonSpecies::Pidgey, 3), lead, policy);
}

#[test]
fn pkmn_shows_a_mon_s_stats_and_switches_it_in_as_the_cartridge_does() {
    use Joypad as J;
    // PKMN, the second mon, STATS through both pages, back out to the battle menu; then PKMN, where
    // the party menu remembers the second mon, and SWITCH.
    let first = [J::A, J::RIGHT, J::A, J::DOWN, J::A, J::DOWN, J::A, J::A, J::A, J::B, J::A, J::A, J::A];
    battle_after(Opponent::Wild(PokemonSpecies::Pidgey, 10), Lead::default(), &first, policy);
}

/// `_LoadTrainerPic` and the enemy's picture decompressed for the opening, measured.
const TRAINER_PICTURES: u32 = 79;
/// `EnemySendOutFirstMon`'s empty text and `TrainerSentOutText`, and `LoadMonFrontSprite`'s
/// decompressing and copying, measured.
const ENEMY_SEND_OUT: u32 = BOX + BOX + 28;

#[test]
fn a_trainer_battle_is_won_for_the_prize_as_the_cartridge_does() {
    let loadings = [
        // YOUNGSTER wants to fight!, after the trainer-appeared sound at its own tempo.
        CLEAR_SCREEN / 2 + TRAINER_PICTURES + SILHOUETTES + BOX + ARROW,
        // RATTATA sent out, Go! Celina!, and the battle menu.
        BOX + CLEAR_SCREEN + BOX + BACK_PICTURE + BOX + BOX + CURSOR + ENEMY_SEND_OUT,
        DELAY3 + CURSOR,
    ];
    battle_timed(Opponent::Trainer(YOUNGSTER, 1), Lead::default(), &loadings, policy);
}

#[test]
#[cfg(feature = "slow-tests")]
fn the_shift_style_offers_a_switch_before_a_trainer_s_next_mon_as_the_cartridge_does() {
    battle_by(Opponent::Trainer(YOUNGSTER, 1), Lead { shift: true, ..Lead::default() }, policy);
}

/// The classes whose `TrainerAIPointers` routine reaches for an item or a switch.
const JUGGLER: u8 = 0x15;
const BROCK: u8 = 0x22;
const COOLTRAINER_M: u8 = 0x1F;
const RIVAL2: u8 = 0x2A;

/// `policy`, keeping every screen the cartridge stopped on so a test can insist the AI acted.
fn policy_recording(printed: &std::cell::RefCell<String>) -> impl Fn(&[Vec<u8>], usize) -> Joypad + '_ {
    move |screen, poll| {
        printed.borrow_mut().extend(screen.iter().map(|row| letters(row) + "/"));
        policy(screen, poll)
    }
}

/// `AIIncreaseStat` raises the stat on the enemy's own turn as though a move had, and `AIRecoverHP`
/// moves its HP bar.
#[test]
fn a_trainer_s_x_attack_and_a_rival_s_potion_match_the_cartridge() {
    let lead = Lead { species: Some((PokemonSpecies::Mewtwo, 70)), ..Lead::default() };
    for (class, used) in [(COOLTRAINER_M, "used X ATTACK"), (RIVAL2, "used POTION")] {
        let printed = std::cell::RefCell::new(String::new());
        battle_by(Opponent::Trainer(class, 1), lead, policy_recording(&printed));
        assert!(printed.borrow().contains(used), "the cartridge's class {class} {used}");
    }
}

/// `BrockAI`: any status on his mon and he cures it, `AICureStatus` taking Toxic's flag with it.
#[test]
fn brock_s_full_heal_cures_the_toxic_the_lead_landed_as_the_cartridge_does() {
    use poke_core::move_name::PokemonMoveName as M;
    let lead = Lead {
        species: Some((PokemonSpecies::Mewtwo, 70)),
        moves: Some([M::Toxic as u8, M::Psychic as u8, 0, 0]),
        ..Lead::default()
    };
    let printed = std::cell::RefCell::new(String::new());
    battle_by(Opponent::Trainer(BROCK, 1), lead, policy_recording(&printed));
    assert!(printed.borrow().contains("used FULL HEAL"), "the cartridge used a FULL HEAL");
}

/// `AISwitchIfEnoughMons`: the switch takes the enemy's turn, and `SwitchEnemyMon` sends the next
/// mon out with no shift prompt behind it. A lead that barely scratches gives the AI its turns.
#[test]
fn a_juggler_s_switch_sends_out_the_next_mon_as_the_cartridge_does() {
    use poke_core::move_name::PokemonMoveName as M;
    let lead = Lead {
        species: Some((PokemonSpecies::Mewtwo, 30)),
        moves: Some([M::Tackle as u8, M::Tackle as u8, 0, 0]),
        ..Lead::default()
    };
    let printed = std::cell::RefCell::new(String::new());
    battle_by(Opponent::Trainer(JUGGLER, 3), lead, policy_recording(&printed));
    assert!(printed.borrow().contains("drew DROWZEE!"), "the cartridge's JUGGLER withdrew a mon");
}

#[test]
fn a_ghost_in_the_tower_scares_the_mon_and_lets_it_run_as_the_cartridge_does() {
    use Joypad as J;
    // It appears, cannot be identified, scares the mon out of its move and says get out; then RUN.
    let presses = [J::A, J::A, J::A, J::A, J::A, J::A, J::RIGHT, J::DOWN, J::A, J::A];
    let lead = Lead { map: Some(POKEMON_TOWER_3F), ..Lead::default() };
    battle(Opponent::Wild(PokemonSpecies::Gastly, 20), lead, &presses, None);
}

/// The player's Mimic: down the enemy's moves to TAIL WHIP, copied over MIMIC, then TACKLE to the
/// end.
#[test]
fn the_player_s_mimic_copies_the_enemy_move_chosen_as_the_cartridge_does() {
    use poke_core::move_name::PokemonMoveName as M;
    let lead = Lead { moves: Some([M::Mimic as u8, M::Tackle as u8, 0, 0]), ..Lead::default() };
    battle_by(Opponent::Wild(PokemonSpecies::Rattata, 5), lead, mimic_then_tackle);
}

fn mimic_then_tackle(screen: &[Vec<u8>], poll: usize) -> Joypad {
    const CURSOR_TILE: u8 = 0xED;
    let text: String = screen.iter().map(|row| letters(row)).collect::<Vec<_>>().join("/");
    if text.contains("WHICH TECHNIQUE") {
        return if screen[8][1] == CURSOR_TILE { Joypad::DOWN } else { Joypad::A };
    }
    if text.contains("TYPE#") {
        let target = if text.contains("MIMIC") { "MIMIC" } else { "TACKLE" };
        let on = (13..17).find(|&row| screen[row][5] == CURSOR_TILE).is_some_and(|row| letters(&screen[row]).contains(target));
        return if on { Joypad::A } else { Joypad::DOWN };
    }
    policy(screen, poll)
}

/// Every item a battle treats differently, a slot each.
pub(super) const BATTLE_BAG: &[(ItemId, u8)] = &[(ItemId::Potion, 3), (ItemId::XAttack, 2), (ItemId::GuardSpec, 1),
    (ItemId::PokeBall, 5), (ItemId::Antidote, 1), (ItemId::PokeFlute, 1), (ItemId::Ether, 1), (ItemId::PokeDoll, 1),
    (ItemId::Repel, 1), (ItemId::MasterBall, 1), (ItemId::FullRestore, 1)];

/// `policy`, but using `items` in turn from the battle menu first: ITEM, down the bag to each, and on
/// through its party menu, its move menu and its texts. A nickname is declined.
pub(super) fn using(items: &'static [ItemId], left: std::rc::Rc<std::cell::Cell<usize>>) -> impl Fn(&[Vec<u8>], usize) -> Joypad {
    move |screen, poll| {
        let text: String = screen.iter().map(|row| letters(row)).collect::<Vec<_>>().join("/");
        let goal = items.get(left.get());
        if text.contains("give a nickname") {
            return Joypad::B;
        }
        if let Some(&item) = goal {
            if text.contains("FIGHT") && text.contains("RUN") {
                return match (screen[14][9], screen[16][9]) {
                    (0xED, _) => Joypad::DOWN,
                    (_, 0xED) => Joypad::A,
                    _ => Joypad::LEFT,
                };
            }
            // The bag: its cursor on column 5, an entry's name one right of it.
            if let Some(row) = [4, 6, 8, 10].into_iter().find(|&row| screen[row][5] == 0xED) {
                let at = |item: ItemId| {
                    let name = poke_core::item::name(item);
                    screen[row][6..6 + name.len()] == name[..]
                };
                if at(item) {
                    left.set(left.get() + 1);
                    return Joypad::A;
                }
                let goal_index = BATTLE_BAG.iter().position(|&(id, _)| id == item);
                let here = BATTLE_BAG.iter().position(|&(id, _)| at(id));
                return if here.is_some_and(|here| Some(here) < goal_index) { Joypad::DOWN } else { Joypad::UP };
            }
        }
        // A bag up again with nothing left to use, after an item was refused: out of it.
        let bag = [5, 7, 9, 11].into_iter().any(|row| screen[row][14] == 0xF1) || text.contains("CANCEL");
        if goal.is_none() && bag && [4, 6, 8, 10].into_iter().any(|row| screen[row][5] == 0xED) {
            return Joypad::B;
        }
        policy(screen, poll)
    }
}

fn battle_with_items(opponent: Opponent, items: &'static [ItemId], lead: Lead) -> Vec<Joypad> {
    let used = std::rc::Rc::new(std::cell::Cell::new(0));
    let presses = battle_by(opponent, Lead { bag: Some(BATTLE_BAG), ..lead }, using(items, used.clone()));
    assert_eq!(used.get(), items.len(), "every item was chosen from the bag");
    presses
}

#[test]
fn a_potion_and_an_ether_on_the_lead_in_battle_match_the_cartridge() {
    let lead = Lead { hp: Some(30), pp: Some([2, 25, 10, 15]), ..Lead::default() };
    battle_with_items(Opponent::Wild(PokemonSpecies::Rattata, 20), &[ItemId::Potion, ItemId::Ether], lead);
}

#[test]
fn an_antidote_and_a_full_restore_cure_the_battle_mon_as_the_cartridge_does() {
    let lead = Lead { hp: Some(119), status: Some(1 << 3), ..Lead::default() };
    battle_with_items(Opponent::Wild(PokemonSpecies::Rattata, 20), &[ItemId::Antidote, ItemId::FullRestore], lead);
}

#[test]
fn x_attack_guard_spec_a_repel_and_a_poke_doll_in_a_wild_battle_match_the_cartridge() {
    use ItemId as I;
    battle_with_items(Opponent::Wild(PokemonSpecies::Rattata, 20), &[I::XAttack, I::GuardSpec, I::Repel, I::PokeDoll], Lead::default());
}

#[test]
fn the_poke_flute_wakes_a_sleeping_lead_as_the_cartridge_does() {
    battle_with_items(Opponent::Wild(PokemonSpecies::Rattata, 20), &[ItemId::PokeFlute], Lead { status: Some(3), ..Lead::default() });
}

#[test]
fn a_ball_that_breaks_and_a_master_ball_that_catches_match_the_cartridge() {
    battle_with_items(Opponent::Wild(PokemonSpecies::Chansey, 30), &[ItemId::PokeBall, ItemId::MasterBall], Lead::default());
}

#[test]
fn a_ball_thrown_at_a_trainer_s_mon_is_blocked_as_the_cartridge_does() {
    battle_with_items(Opponent::Trainer(YOUNGSTER, 1), &[ItemId::PokeBall], Lead::default());
}

#[test]
fn a_catch_with_a_full_party_goes_to_the_box_and_a_full_box_refuses_the_ball() {
    let lead = Lead { full_party: true, ..Lead::default() };
    battle_with_items(Opponent::Wild(PokemonSpecies::Chansey, 30), &[ItemId::MasterBall], lead);
    let lead = Lead { full_party: true, box_count: Some(20), ..Lead::default() };
    battle_with_items(Opponent::Wild(PokemonSpecies::Rattata, 30), &[ItemId::MasterBall], lead);
}

#[test]
fn the_old_man_catches_a_weedle_by_himself_as_the_cartridge_does() {
    let presses = battle_by(Opponent::Wild(PokemonSpecies::Weedle, 5), Lead { old_man: true, ..Lead::default() }, policy);
    assert_eq!(presses.len(), 3, "the appearance, the ball and the catch: nothing else asks");
}

/// The Safari Zone's menu rows as `DisplayBattleMenu` numbers them there.
const SAFARI_BALL: u8 = 0;
const SAFARI_ROCK: u8 = 1;
const SAFARI_BAIT: u8 = 2;

/// Down the Safari Zone's `choices` in turn, then RUN for good; a nickname is declined.
pub(super) fn safari_choices(choices: &'static [u8]) -> impl Fn(&[Vec<u8>], usize) -> Joypad {
    let made = std::cell::Cell::new(0);
    move |screen, _| {
        let text: String = screen.iter().map(|row| letters(row)).collect::<Vec<_>>().join("/");
        if text.contains("give a nickname") {
            return Joypad::B;
        }
        if !text.contains("THROW ROCK") {
            return Joypad::A;
        }
        let target = choices.get(made.get()).copied().unwrap_or(3);
        let (x, y) = ([1, 1, 13, 13][target as usize], [14, 16, 14, 16][target as usize]);
        let at = [(1, 14), (1, 16), (13, 14), (13, 16)].into_iter().find(|&(x, y)| screen[y][x] == 0xED).expect("a cursor");
        if at == (x, y) {
            made.set(made.get() + 1);
            return Joypad::A;
        }
        if at.0 != x {
            return if x > at.0 { Joypad::RIGHT } else { Joypad::LEFT };
        }
        if y > at.1 { Joypad::DOWN } else { Joypad::UP }
    }
}

#[test]
fn rocks_and_bait_in_the_safari_zone_match_the_cartridge() {
    let lead = Lead { map: Some(SAFARI_ZONE_EAST), ..Lead::default() };
    battle_by(Opponent::Wild(PokemonSpecies::Rhyhorn, 25), lead,
        safari_choices(&[SAFARI_ROCK, SAFARI_ROCK, SAFARI_BAIT, SAFARI_BAIT, SAFARI_ROCK]));
}

#[test]
fn safari_balls_until_the_mon_is_caught_or_gone_match_the_cartridge() {
    let lead = Lead { map: Some(SAFARI_ZONE_EAST), ..Lead::default() };
    battle_by(Opponent::Wild(PokemonSpecies::Rhyhorn, 25), lead, safari_choices(&[SAFARI_BALL; 30]));
}

#[test]
fn a_fast_safari_mon_runs_as_the_cartridge_does() {
    let lead = Lead { map: Some(SAFARI_ZONE_EAST), ..Lead::default() };
    battle_by(Opponent::Wild(PokemonSpecies::Tauros, 25), lead, safari_choices(&[SAFARI_BAIT; 10]));
}

fn species_named(name: &str) -> PokemonSpecies {
    (1..=255).filter_map(PokemonSpecies::from_repr).find(|species| format!("{species:?}") == name).expect("a species")
}

/// `SPECIES` and `LEVEL`, or `CLASS` and `NUMBER`.
fn opponent_from_env() -> Opponent {
    let var = |name: &str| std::env::var(name).ok();
    match (var("CLASS"), var("NUMBER")) {
        (Some(class), Some(number)) => Opponent::Trainer(class.parse().unwrap(), number.parse().unwrap()),
        _ => Opponent::Wild(species_named(&var("SPECIES").unwrap_or("Mewtwo".into())),
            var("LEVEL").map_or(70, |level| level.parse().unwrap())),
    }
}

/// `LEAD` as a species name and `LEAD_LEVEL`, and `ABOUT_TO_LEVEL`.
fn lead_from_env() -> Lead {
    let var = |name: &str| std::env::var(name).ok();
    Lead {
        species: var("LEAD").map(|name| (species_named(&name), var("LEAD_LEVEL").map_or(50, |level| level.parse().unwrap()))),
        about_to_level: var("ABOUT_TO_LEVEL").is_some(),
        shift: var("SHIFT").is_some(),
        map: var("MAP").map(|map| u8::from_str_radix(map.trim_start_matches("0x"), 16).unwrap()),
        bag: var("BAG").map(|_| BATTLE_BAG),
        moves: var("MOVES").map(|moves| std::array::from_fn(|i| moves.split(',').nth(i).and_then(|id| id.parse().ok()).unwrap_or(0))),
        ..Lead::default()
    }
}

/// `PRESSES` as letters: A, B, U, D, L, R.
fn presses_from_env() -> Vec<Joypad> {
    std::env::var("PRESSES").map_or(vec![], |presses| presses.chars().map(|c| match c {
        'R' => Joypad::RIGHT, 'L' => Joypad::LEFT, 'U' => Joypad::UP, 'D' => Joypad::DOWN, 'B' => Joypad::B,
        'S' => Joypad::SELECT, _ => Joypad::A,
    }).collect())
}

#[test]
#[ignore = "a probe: one battle against SPECIES at LEVEL, or trainer CLASS and NUMBER, pressing PRESSES first"]
fn probe_one_battle() {
    let presses = presses_from_env();
    battle_by(opponent_from_env(), lead_from_env(), |screen, poll| presses.get(poll).copied().unwrap_or_else(|| policy(screen, poll)));
}

#[test]
#[ignore = "a probe: wild battles against every species from FIRST to LAST, reporting each that differs"]
fn probe_many_wild_battles() {
    let first: u8 = std::env::var("FIRST").map_or(1, |id| id.parse().unwrap());
    let last: u8 = std::env::var("LAST").map_or(190, |id| id.parse().unwrap());
    let mut failed = vec![];
    for id in first..=last {
        let Some(species) = PokemonSpecies::from_repr(id) else { continue };
        let level = 5 + id % 60;
        let result = std::panic::catch_unwind(|| battle_by(Opponent::Wild(species, level), Lead::default(), policy));
        if let Err(error) = result {
            let message = error.downcast_ref::<String>().cloned()
                .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            println!("FAILED {species:?} {level}: {}", message.lines().next().unwrap_or(""));
            failed.push(species);
        }
    }
    println!("{} failed: {failed:?}", failed.len());
}

#[test]
#[ignore = "a probe: every trainer from class FIRST to LAST, reporting each that differs"]
fn probe_many_trainer_battles() {
    let first: u8 = std::env::var("FIRST").map_or(1, |id| id.parse().unwrap());
    let last: u8 = std::env::var("LAST").map_or(47, |id| id.parse().unwrap());
    let mut failed = vec![];
    for class in first..=last {
        for number in 1..=poke_core::trainers::parties(class).len() as u8 {
            let result = std::panic::catch_unwind(|| battle_by(Opponent::Trainer(class, number), lead_from_env(), policy));
            if let Err(error) = result {
                let message = error.downcast_ref::<String>().cloned()
                    .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                println!("FAILED {class} {number}: {}", message.lines().next().unwrap_or(""));
                failed.push((class, number));
            }
        }
    }
    println!("{} failed: {failed:?}", failed.len());
}

fn tile_row_of(gb: &GameBoy, y: u16) -> Vec<u8> {
    (0..20).map(|x| gb.core().mmu().read(sym::wTileMap.address + 20 * y + x)).collect()
}

/// The nearest label at or before `address` in the bank mapped there now.
fn nearest_label(gb: &GameBoy, address: u16) -> String {
    let bank = if (0x4000..0x8000).contains(&address) { gb.core().mmu().rom_bank() as u8 } else { 0 };
    include_str!("../../../vendor/pokered/pokered.sym").lines()
        .filter_map(|line| {
            let (at, name) = line.split_once(' ')?;
            let (b, a) = at.split_once(':')?;
            let (b, a) = (u8::from_str_radix(b, 16).ok()?, u16::from_str_radix(a, 16).ok()?);
            (b == bank && a <= address).then_some((a, name))
        })
        .max_by_key(|&(a, _)| a)
        .map_or(format!("{address:04x}"), |(a, name)| format!("{name}+{}", address - a))
}

#[test]
#[ignore = "a probe: the cartridge's call stack at each VBlank on the way to poll POLLS, pressing A before it"]
fn probe_the_cartridge_s_frames() {
    let polls: usize = std::env::var("POLLS").map_or(3, |polls| polls.parse().unwrap());
    let mut cartridge = Cartridge::new(opponent_from_env(), lead_from_env());
    let presses = presses_from_env();
    for poll in 0..polls {
        let seen = cartridge.to_poll().expect("polls");
        cartridge.press(presses.get(poll).copied().unwrap_or_else(|| policy(&seen.screen, poll)));
    }
    let gb = &mut cartridge.gb;
    let (poll, vblank) = (breakpoint(sym::JoypadLowSensitivity), breakpoint(sym::VBlank));
    let mut frame = 0;
    while frame < 2000 {
        let (stop, _) = gb.run_until(&[poll, vblank], MachineCycles::PER_FRAME * 600);
        if stop == Stop::Breakpoint(poll) {
            println!("{frame}: poll");
            break;
        }
        frame += 1;
        let sp = gb.core().registers().sp;
        let stack: Vec<String> = (0..12).map(|i| gb.core().mmu().read_u16_le(sp + 2 * i))
            .filter(|&a| (0x100..0x8000).contains(&a)).take(6).map(|a| nearest_label(gb, a)).collect();
        let row = std::env::var("ROW").ok().map(|row| format!(" row {:02x?}", tile_row_of(gb, row.parse().unwrap()))).unwrap_or_default();
        println!("{frame}: {}{row}", stack.join(" < "));
    }
}

impl Cartridge {
    /// A cartridge already running, compared at its polls for as long as the caller asks: nothing it
    /// runs returns to `back`.
    fn running(gb: GameBoy) -> Self {
        Self { gb, tape: vec![], back: (Breakpoint::new(0, 0), 0), ended: false, seams: SEAMS.to_vec(), lcd: None }
    }

    /// A held until `DisplayTextID` is entered, as `scripts.rs`'s `Action::Talk` holds it, taping on
    /// the way, and let go for a frame.
    fn talk(&mut self) {
        let (random, talk) = (breakpoint(sym::Random), breakpoint(sym::DisplayTextID));
        self.gb.hold_buttons(joypad(Joypad::A));
        loop {
            let (stop, _) = self.gb.run_until(&[random, talk], MachineCycles::PER_FRAME * 600);
            assert!(matches!(stop, Stop::Breakpoint(_)), "the cartridge never talked: {stop:?}");
            if stop == Stop::Breakpoint(talk) {
                break;
            }
            let caller = self.gb.return_address();
            let (stop, _) = self.gb.run_to_return(MachineCycles::PER_FRAME * 10);
            assert!(matches!(stop, Stop::Returned { .. }));
            if !is_vblank_caller(caller) {
                self.tape.push(self.gb.core().registers().a);
            }
        }
        self.gb.hold_buttons(JoypadButtonState::default());
        self.run(false);
    }
}

/// The recreation's side of `Cartridge::talk`: A until something opens over the overworld.
fn recreation_talk(game: &mut Game) {
    for _ in 0..600 {
        game.frame(Input::Buttons(Joypad::A));
        if game.modes().len() > 1 {
            return;
        }
    }
    panic!("the recreation never talked");
}

/// Where both start in front of a gym leader: the recreation's world and overworld as the cartridge
/// stands, and what the recreation's `Game` needs besides.
struct GymStart {
    world: pokered::world::World,
    overworld: pokered::modes::overworld::Overworld,
    /// `wBattleAndStartSavedMenuItem`, `wPartyAndBillsPCSavedMenuItem`, `wBagSavedMenuItem` and
    /// `wListScrollOffset`.
    menus: [u8; 4],
    /// The sound effect channels' note delay counters.
    sfx_note_delays: [u8; 4],
}

/// The sound effect channels, as `scripts.rs` seeds them.
const SFX_CHANNELS: [usize; 4] = [4, 5, 6, 7];

impl GymStart {
    fn game(&self, tape: Vec<u8>) -> Game {
        let mut game = Game::new(self.world.clone(), GameRng::tape(tape), Pacing::Faithful);
        let menu = game.menu_mut();
        [menu.battle_and_start, menu.party_and_bills, menu.bag_saved, menu.list_scroll] = self.menus;
        // `scripts.rs`'s `seed_sfx_note_delays`: an idle sound effect channel keeps the counter its last
        // sound left, which decides whether the next one gets the channel.
        let mut engine = serde_json::to_value(game.audio()).expect("the engine serialises");
        for (c, counter) in SFX_CHANNELS.into_iter().zip(self.sfx_note_delays) {
            if game.audio().channel_sound_id(c) == 0 {
                engine["channels"][c]["note_delay_counter"] = counter.into();
            }
        }
        *game.audio_mut() = serde_json::from_value(engine).expect("the engine deserialises");
        game.push(Mode::Overworld(self.overworld.clone()));
        game
    }
}

/// `pewter-gym.bin`, which stands below BROCK with his badge won, made to forget it as `scripts.rs`'s
/// badge test does, and run to its first overworld poll in the SET style. The recreation's start is
/// taken there as `scripts.rs`'s `Cartridge::start` takes it; those few lines are copied from it.
fn in_front_of_brock() -> (Cartridge, GymStart) {
    use poke_core::symbols::pokered_events::{EVENT_BEAT_BROCK, EVENT_GOT_TM34};
    use pokered::modes::overworld::{Overworld, Standing};
    use pokered::systems::overworld::sprites::SpriteState;
    let mut walker = super::scripts::Cartridge::from_state(include_bytes!("../pokemon/data/pewter-gym.bin"));
    let options = walker.read(sym::wOptions.address);
    walker.write(sym::wOptions.address, options | 1 << 6);
    while walker.to_poll().0 != super::scripts::Kind::Overworld {}
    for i in 0..walker.read(sym::wPartyCount.address) as u16 {
        let mon = PARTY_STRUCT * i;
        for byte in 0..2 {
            let max = walker.read(sym::wPartyMon1MaxHP.address + mon + byte);
            walker.write(sym::wPartyMon1HP.address + mon + byte, max);
        }
        walker.write(sym::wPartyMon1Status.address + mon, 0);
    }
    for event in [EVENT_BEAT_BROCK, EVENT_GOT_TM34] {
        let at = sym::wEventFlags.address + event / 8;
        let flags = walker.read(at);
        walker.write(at, flags & !(1 << (event % 8)));
    }
    let badges = walker.read(sym::wObtainedBadges.address);
    walker.write(sym::wObtainedBadges.address, badges & !1);

    let gb = &walker.gb;
    let mmu = gb.core().mmu();
    let sprites = std::array::from_fn(|slot| {
        let at = slot as u16 * 16;
        let data1 = mmu.read_slice(sym::wSpriteStateData1.address + at, 16);
        let data2 = mmu.read_slice(sym::wSpriteStateData2.address + at, 16);
        let map_data = if slot == 0 { [0, 0] } else {
            let entry = sym::wMapSpriteData.address + (slot as u16 - 1) * 2;
            [mmu.read(entry), mmu.read(entry + 1)]
        };
        SpriteState::from_bytes(&data1, &data2, map_data)
    });
    let standing = Standing {
        player_direction: mmu.read_pointer(&sym::wPlayerDirection),
        moving_direction: mmu.read_pointer(&sym::wPlayerMovingDirection),
        last_stop_direction: mmu.read_pointer(&sym::wPlayerLastStopDirection),
        check_for_180_degree_turn: mmu.read_pointer(&sym::wCheckFor180DegreeTurn),
        standing_on_warp: mmu.read_pointer(&sym::wMovementFlags) & 1 << 2 != 0,
        destination_warp: mmu.read_pointer(&sym::wDestinationWarpID),
    };
    let overworld = Overworld::standing(sprites, mmu.read_pointer(&sym::wNumSprites), standing)
        .with_battle_flags(mmu.read_pointer(&sym::wStatusFlags4) & 1 << 4 != 0, mmu.read_pointer(&sym::wStatusFlags2) & 1 != 0,
            mmu.read_pointer(&sym::wNumberOfNoRandomBattleStepsLeft))
        .with_step_counter(mmu.read_pointer(&sym::wStepCounter));
    let menus = [&sym::wBattleAndStartSavedMenuItem, &sym::wPartyAndBillsPCSavedMenuItem, &sym::wBagSavedMenuItem,
        &sym::wListScrollOffset].map(|at| mmu.read_pointer(at));
    let sfx_note_delays = SFX_CHANNELS.map(|c| mmu.read(sym::wChannelNoteDelayCounters.address + c as u16));
    let start = GymStart { world: walker.world(), overworld, menus, sfx_note_delays };
    (Cartridge::running(walker.gb), start)
}

/// The screen as the recreation shows it over the overworld: what the UI covers, and the map through
/// the view where it does not, as `scripts.rs` composes it.
fn over_the_map(game: &Game) -> Vec<Vec<u8>> {
    let in_battle = game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_)));
    let map = game.modes().iter().rev().find_map(|mode| match mode {
        Mode::Overworld(overworld) if !in_battle => Some(overworld.view().tile_map()),
        _ => None,
    });
    (0..18).map(|y| (0..20).map(|x| {
        game.ui().cover(x, y).or_else(|| map.map(|tiles| tiles[y * 20 + x])).unwrap_or(game.ui().get(x, y))
    }).collect()).collect()
}

/// Frames until the recreation waits on anything but the overworld, and for what.
fn gym_recreation_to_poll(game: &mut Game) -> (u32, Decision) {
    for frames in 1..5000 {
        game.frame(Input::None);
        if let Status::Waiting(decision) = game.status() && decision != Decision::Overworld {
            return (frames, decision);
        }
    }
    panic!("the recreation never waited");
}

/// `wChannelSoundIDs` for the four music channels.
fn cartridge_music(gb: &GameBoy) -> [u8; 4] {
    std::array::from_fn(|c| gb.core().mmu().read(sym::wChannelSoundIDs.address + c as u16))
}

fn in_battle(gb: &GameBoy) -> bool {
    gb.core().mmu().read(sym::wIsInBattle.address) != 0
}

/// BROCK talked to below him and fought through Pewter Gym's own script, so the battle is the one
/// the script builds rather than one written into WRAM: his lone move on ONIX, the gym leader's
/// battle music and the victory music. Compared at every poll from his first text to the first one
/// after the battle: the screen and the music channels, and at each battle poll the enemy's moves.
#[test]
fn brock_fought_through_the_gym_s_script_matches_the_cartridge() {
    use poke_core::move_name::PokemonMoveName;
    use pokered::audio::data::sounds;
    let (mut cartridge, _) = in_front_of_brock();
    cartridge.talk();
    let mut presses = vec![];
    let (mut fought, mut bide) = (false, false);
    loop {
        let poll = cartridge.to_poll().expect("the cartridge polls");
        let battling = in_battle(&cartridge.gb);
        fought |= battling;
        if fought && !battling {
            break;
        }
        let mmu = cartridge.gb.core().mmu();
        let onix = mmu.read(sym::wEnemyMonSpecies.address) == PokemonSpecies::Onix as u8;
        let text: String = poll.screen.iter().map(|row| letters(row)).collect();
        bide |= battling && onix && text.contains("FIGHT") && mmu.read_slice(sym::wEnemyMonMoves.address, 4).contains(&(PokemonMoveName::Bide as u8));
        let button = policy(&poll.screen, presses.len());
        presses.push(button);
        cartridge.press(button);
        assert!(presses.len() < 300, "the battle goes on");
    }
    assert!(bide, "ONIX came out knowing BIDE");

    let tape = std::mem::take(&mut cartridge.tape);
    let (mut cartridge, start) = in_front_of_brock();
    let mut game = start.game(tape);
    cartridge.talk();
    recreation_talk(&mut game);
    let mut heard = vec![];
    for (poll, button) in presses.iter().map(|&button| Some(button)).chain([None]).enumerate() {
        let theirs = cartridge.to_poll().expect("polls");
        let (frames, decision) = gym_recreation_to_poll(&mut game);
        println!("poll {poll} {decision:?}: cartridge {} frames, recreation {frames}", theirs.frames);
        let mine = over_the_map(&game);
        if theirs.screen != mine {
            for (a, b) in theirs.screen.iter().zip(&mine) {
                println!("  |{}|  |{}|", letters(a), letters(b));
            }
        }
        assert_eq!(theirs.screen, mine, "the screen at poll {poll}");
        let music = cartridge_music(&cartridge.gb);
        assert_eq!(music, std::array::from_fn(|c| game.audio().channel_sound_id(c)), "the music at poll {poll}");
        heard.push(music[0]);
        let battle = game.modes().iter().find_map(|mode| match mode {
            Mode::Battle(battle) => battle.battle(),
            _ => None,
        });
        assert_eq!(in_battle(&cartridge.gb), battle.is_some(), "a battle at poll {poll}");
        // Before the first mon is sent out `wEnemyMon` still holds the last battle's.
        if let Some(battle) = battle.filter(|_| decision == Decision::BattleMenu) {
            let theirs = cartridge.gb.core().mmu().read_slice(sym::wEnemyMonMoves.address, 4);
            let mine: Vec<u8> = battle.enemy.mon.moves.iter().map(|name| name.map_or(0, |name| name as u8)).collect();
            assert_eq!(theirs, mine, "the enemy's moves at poll {poll}");
            assert_eq!(cartridge_state(&cartridge.gb), recreation_state(&game), "the battle at poll {poll}");
        }
        if let Some(button) = button {
            press_both(&mut cartridge, &mut game, button);
        }
    }
    for music in [sounds::MUSIC_GYM_LEADER_BATTLE, sounds::MUSIC_DEFEATED_GYM_LEADER] {
        assert!(heard.contains(&music.id.0), "{music:?} was played");
    }
}

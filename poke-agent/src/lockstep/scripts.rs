//! Map scripts against the cartridge: a fixture's player walked into a trainer's sight, up to a
//! clerk, into Oak's cutscene and through the grass, the same presses into both machines and the two
//! compared at every point either waits for the player.
//!
//! The cartridge runs first and alone, taping every `Random` byte that is not VBlank's own and the
//! two `hRandomAdd`/`hRandomSub` bytes `TryDoWildEncounter` reads, since the recreation draws some of
//! them before it shows what they decide; `BattleTransition` and `MoveAnimation` are returned from
//! unentered, as the battle lockstep does. Then the recreation replays the presses on the tape.
//!
//! Every stretch between two points is timed against the loading the cartridge entered on the way
//! (`Loading`), counted by breakpoints on the routines that do it. A stretch through a battle or a
//! map load is compared but not timed: the battle lockstep times a battle's interior, and a map loads
//! with the LCD off.

use gb::cycles::MachineCycles;
use gb::game_boy::{Breakpoint, GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use gb::ram::{RAM, ROM};
use poke_core::map::Map;
use poke_core::species::PokemonSpecies;
use poke_core::sprite::SpriteFacing;
use poke_core::symbols::pokered_local_labels as local;
use pokered::command::Decision;
use pokered::gfx::compose::WIDTH;
use pokered::gfx::tiles::V_CHARS2;
use pokered::input::Joypad;
use pokered::mode::{Mode, ModeUpdate, Status};
use pokered::modes::overworld::{Overworld, Standing};
use pokered::party::Pokedex;
use pokered::rng::GameRng;
use pokered::systems::overworld::sprites::{SpriteState, Sprites, MAP_TILESET_SIZE};
use pokered::systems::overworld::Location;
use pokered::systems::overworld::location::Ahead;
use pokered::world::{BattleStyle, TextSpeed};
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointer, DmgPointerRead};
use super::{breakpoint, joypad};

const STATE_BYTES: u16 = 16;
const PARTY_STRUCT: u16 = 44;
/// `wStatusFlags6`'s.
const BIT_ALWAYS_ON_BIKE: u8 = 5;
/// `wStatusFlags1`'s.
const BIT_STRENGTH_ACTIVE: u8 = 0;
/// `wStatusFlags4`'s.
const BIT_GOT_STARTER: u8 = 3;
/// `wStatusFlags4`'s.
const BIT_NO_BATTLES: u8 = 4;
/// The routines both sides skip: the trade's animation is the movie's to recreate.
const SEAMS: [DmgPointer; 3] = [sym::BattleTransition, sym::MoveAnimation, sym::InternalClockTradeAnim];
/// Callers of `Random` that read `hRandomSub` as well, which the recreation takes as a second byte.
const READS_RANDOM_SUB: [(DmgPointer, DmgPointer); 1] = [(sym::InGameTrade_PrepareTradeData, sym::InGameTrade_CopyData)];
const BUDGET: u32 = 6000;
/// `TextCommand_PAUSE`'s `DelayFrames`.
const TX_PAUSE_FRAMES: u32 = 30;

/// Where the machine waits for the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// The overworld's loop, free to walk.
    Overworld,
    /// A text's `▼`, a menu, a battle prompt.
    Prompt,
    /// Not a poll: the frame `wCurMap` changed, which a cutscene that never lets the player go has
    /// in place of one.
    MapChange,
    /// Not a poll: the frame an `EmotionBubble` goes up, which splits a cutscene into stretches short
    /// enough for their timing to say something.
    Bubble,
}

/// What is pressed at a poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    /// A direction held until a step, a jump or a warp has begun.
    Walk(Joypad),
    /// A button held for a number of frames.
    Press(Joypad, u32),
    /// A held until a text starts: `DisplayTextID`.
    Talk,
}

/// A point both agree on, seen from one machine.
#[derive(Debug, PartialEq)]
pub(super) struct Seen {
    pub kind: Kind,
    frames: u32,
    pub location: (Map, u8, u8, SpriteFacing),
    pub screen: Vec<Vec<u8>>,
    pub events: Vec<u8>,
    money: [u8; 3],
    pub bag: Vec<(u8, u8)>,
    /// Outside a battle, the party and what the events move; `None` where `party_unsettled` says the
    /// cartridge is midway through writing the party and there is nothing yet to compare.
    pub held: Option<Held>,
    /// At an overworld poll: the sprite slots, and the LCD two frames on with the animated tiles.
    pub sprites: Option<(u8, Sprites)>,
    lcd: Option<(Vec<u8>, Vec<u8>)>,
    /// At an overworld poll after a battle: `wAudioFadeOutCounterReloadValue` and `wLastMusicSoundID`.
    music: Option<(u8, u8)>,
    /// The cartridge's loading on the way here.
    entered: Vec<Loading>,
}

impl Seen {
    /// What the events move, where the caller knows the cartridge has settled it: any overworld poll.
    pub fn settled(&self) -> &Held {
        self.held.as_ref().expect("the cartridge is midway through writing the party at this poll")
    }
}

/// The party's species, levels and HP, the badges, the coins, the day care's mon, the trades, the
/// hidden items and coins found and the blackout map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Held {
    pub badges: u8,
    pub party: Vec<(PokemonSpecies, u8)>,
    pub party_hp: Vec<u16>,
    pub coins: [u8; 2],
    pub day_care: Option<(PokemonSpecies, u8)>,
    pub trades: u16,
    pub hidden: Vec<u8>,
    pub last_blackout_map: Map,
}

pub(super) struct Cartridge {
    pub gb: GameBoy,
    tape: Vec<u8>,
    map: u8,
    /// The routines of `LOADING` entered since the last poll.
    entered: Vec<Loading>,
    /// `hl` at the last `PrintText`.
    text: u16,
    /// Frames since the last action began.
    acting: u32,
    /// Inside a routine of `LAG`: the stack pointer at its entry, its return, and `acting` then.
    lagging: Option<(u16, Breakpoint, u32)>,
    /// Inside a span of `LAG_SPANS`: its end, and `acting` at its start.
    spanning: Option<(Breakpoint, u32)>,
    /// A battle has been polled in since the start.
    fought: bool,
}

/// What the cartridge does between two points that the recreation leaves out, each entry to one of
/// these routines costing the frames `Loading::frames` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Loading {
    /// `DisplayTextIDInit`: `CopyScreenTileBufferToVRAM` and `LoadFontTilePatterns`.
    DisplayTextIdInit,
    /// `PrintText`'s `Delay3` after its box.
    PrintText,
    /// `ProtectedDelay3`, at a `▼`.
    Arrow,
    /// `EmotionBubble`'s `CopyVideoData` of the bubble's four tiles.
    EmotionBubble,
    /// `CloseTextDisplay`: `InitMapSprites` with the LCD on. The bool is `wFontLoaded`, which is
    /// what sends the walking halves through `CopyVideoData`, a sheet at a time; with the font
    /// already gone both halves of every sheet go through `FarCopyData2` instead.
    CloseTextDisplay(bool),
    /// `LoadPlayerSpriteGraphicsCommon` with the LCD on: two copies of a sheet.
    PlayerSpriteGraphics,
    /// `LoadMapData` turns the LCD off: no VBlanks to count.
    LoadMapData,
    /// `DisableLCD` anywhere else: the party menu's icons, a screen's tiles reloaded.
    DisableLcd,
    /// `HandleMenuInput_`'s `Delay3` after placing the cursor.
    HandleMenuInput,
    /// `RedrawPartyMenu_`'s `Delay3` once the list is drawn.
    PartyMenuDrawn,
    /// `LoadTextBoxTilePatterns` and `LoadHpBarAndStatusTilePatterns` with the LCD on, which jump to
    /// `CopyVideoData`: the tiles each copies.
    TilePatterns(u16),
    /// `DisplayListMenuID`: its box, its entries and its cursor.
    DisplayListMenuId,
    /// `CopyVideoData` outside the routines above, as the healing machine's tiles: eight tiles a
    /// frame, and `c` holds how many.
    CopyVideoData(u16),
    /// `GBPalWhiteOutWithDelay3`'s `Delay3`.
    WhiteOut,
    /// `ReloadMapData`, which turns the LCD off.
    ReloadMapData,
    /// A battle, whose interior is the battle lockstep's to time.
    InitBattle,
    /// A step begun, `.noCollision`: its first pass can run a frame long on the cartridge, which is
    /// lag, allowed rather than counted.
    Step,
    /// Not loading: a `TX_PAUSE` that opens a `PrintText`, and the frames since the action began.
    LeadingPause(u32),
    /// A routine of `LAG`, and the frames it outlasted.
    Lag(u32),
}

const LOADING: &[(Loading, DmgPointer)] = &[
    (Loading::DisplayTextIdInit, sym::DisplayTextIDInit), (Loading::PrintText, sym::PrintText),
    (Loading::Arrow, sym::ProtectedDelay3), (Loading::EmotionBubble, sym::EmotionBubble),
    (Loading::CloseTextDisplay(false), sym::CloseTextDisplay), (Loading::LoadMapData, sym::LoadMapData),
    (Loading::InitBattle, sym::InitBattleCommon), (Loading::Step, local::OverworldLoopLessDelay::noCollision),
    (Loading::DisableLcd, sym::DisableLCD), (Loading::HandleMenuInput, sym::HandleMenuInput_),
    (Loading::DisplayListMenuId, sym::DisplayListMenuID), (Loading::WhiteOut, sym::GBPalWhiteOutWithDelay3),
    (Loading::ReloadMapData, sym::ReloadMapData), (Loading::PartyMenuDrawn, local::RedrawPartyMenu_::done),
    (Loading::TilePatterns(TEXT_BOX_TILES), local::LoadTextBoxTilePatterns::on),
    (Loading::PlayerSpriteGraphics, sym::LoadPlayerSpriteGraphicsCommon),
    (Loading::TilePatterns(HP_BAR_AND_STATUS_TILES), local::LoadHpBarAndStatusTilePatterns::on),
];

const TEXT_BOX_TILES: u16 = (sym::TextBoxGraphicsEnd.address - sym::TextBoxGraphics.address) / 16;
const HP_BAR_AND_STATUS_TILES: u16 = (sym::HpBarAndStatusGraphicsEnd.address - sym::HpBarAndStatusGraphics.address) / 16;

/// Arithmetic long enough to run over frames, which the recreation does at once: lag, priced by the
/// VBlanks the cartridge takes between its entry and its return.
const LAG: [DmgPointer; 1] = [sym::CalcLevelFromExperience];
/// The same for a loop inside a routine that also waits: from the first time its start is reached
/// to its end.
const LAG_SPANS: [(DmgPointer, DmgPointer); 1] = [(local::RedrawPartyMenu_::r#loop, local::RedrawPartyMenu_::afterDrawingMonEntries)];

/// The callers of `CopyVideoData` whose copies are not already priced by the routine that calls.
const COPY_VIDEO_DATA_CALLERS: [(DmgPointer, DmgPointer); 3] = [
    (sym::AnimateHealingMachine, sym::PokeCenterFlashingMonitorAndHealBall),
    (sym::LoadSmokeTileFourTimes, sym::LoadSmokeTile),
    (sym::InitCutAnimOAM, sym::LoadCutGrassAnimationTilePattern),
];

/// `CopyVideoData` and `CopyVideoDataDouble`: eight tiles a frame, and a frame for the rest.
fn copy_video_data(tiles: u16) -> u32 {
    tiles as u32 / 8 + 1
}

/// `InitMapSprites` with the font already gone, which waits on no VBlank: its `FarCopyData2` of
/// every sheet runs a frame long whatever the map holds, and `CloseTextDisplay`'s `DelayFrame` is
/// the other.
const FAR_COPY_SHEETS: u32 = 2;
/// Tiles in a sprite's walking sheet and in the font.
const SPRITE_SHEET_TILES: u16 = 12;
const FONT_TILES: u16 = (sym::FontGraphicsEnd.address - sym::FontGraphics.address) / 8;

impl Loading {
    /// `None` where the stretch cannot be timed.
    fn frames(self, map: Map, x: u8, y: u8) -> Option<u32> {
        Some(match self {
            Loading::DisplayTextIdInit => super::DELAY3 + copy_video_data(FONT_TILES),
            Loading::PrintText => super::BOX,
            Loading::Arrow => super::ARROW,
            Loading::EmotionBubble => copy_video_data(4),
            Loading::CloseTextDisplay(font_loaded) => match font_loaded {
                true => walking_pictures(map, x, y) as u32 * copy_video_data(SPRITE_SHEET_TILES),
                false => FAR_COPY_SHEETS,
            },
            Loading::PlayerSpriteGraphics => 2 * copy_video_data(SPRITE_SHEET_TILES),
            Loading::Step | Loading::LeadingPause(_) => 0,
            Loading::Lag(frames) => frames,
            Loading::HandleMenuInput => super::CURSOR,
            Loading::DisplayListMenuId => super::LIST,
            Loading::CopyVideoData(tiles) | Loading::TilePatterns(tiles) => copy_video_data(tiles),
            Loading::WhiteOut | Loading::PartyMenuDrawn => super::DELAY3,
            Loading::LoadMapData | Loading::InitBattle | Loading::DisableLcd | Loading::ReloadMapData => return None,
        })
    }
}

/// The walking sheets `InitMapSprites` loads again with the font loaded: an outside map's sprite
/// set, or an inside map's own pictures, each once.
fn walking_pictures(map: Map, x: u8, y: u8) -> usize {
    use poke_core::map_objects::{sprite_set, sprite_set_id, MapObjects, FIRST_STILL_SPRITE};
    let pictures: Vec<u8> = match sprite_set_id(map, x, y) {
        Some(id) => sprite_set(id).to_vec(),
        None => MapObjects::read(map).unwrap().objects.iter().map(|object| object.picture).collect(),
    };
    let mut walking: Vec<u8> = pictures.into_iter().filter(|&picture| picture != 0 && picture < FIRST_STILL_SPRITE).collect();
    walking.sort();
    walking.dedup();
    walking.len()
}


impl Cartridge {
    pub fn from_state(state: &[u8]) -> Self {
        let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(state).unwrap();
        gb.core_mut().mmu_mut().audio_mut().set_output_enabled(false);
        let map = gb.core().mmu().read(sym::wCurMap.address);
        Self { gb, tape: Vec::new(), map, entered: Vec::new(), text: 0, acting: 0, lagging: None, spanning: None, fought: false }
    }

    pub fn read(&self, at: u16) -> u8 {
        self.gb.core().mmu().read(at)
    }

    pub fn write(&mut self, at: u16, value: u8) {
        self.gb.core_mut().mmu_mut().write(at, value);
    }

    /// Runs to the first of `points`, taping random bytes and skipping the seams on the way.
    fn run_to(&mut self, points: &[Breakpoint]) -> Breakpoint {
        let random = breakpoint(sym::Random);
        let encounter = breakpoint(local::TryDoWildEncounter::CanEncounter);
        let seams: Vec<_> = SEAMS.iter().map(|&seam| breakpoint(seam)).collect();
        let copy_video = breakpoint(sym::CopyVideoData);
        let pause = breakpoint(sym::TextCommand_PAUSE);
        let mut all = points.to_vec();
        all.extend([copy_video, pause]);
        all.extend([random, encounter]);
        all.extend(&seams);
        all.extend(LOADING.iter().map(|&(_, at)| breakpoint(at)));
        let lag: Vec<_> = LAG.iter().map(|&at| breakpoint(at)).collect();
        all.extend(&lag);
        all.extend(self.lagging.map(|(_, back, _)| back));
        let span_starts: Vec<_> = LAG_SPANS.iter().map(|&(start, _)| breakpoint(start)).collect();
        all.extend(&span_starts);
        all.extend(LAG_SPANS.iter().map(|&(_, end)| breakpoint(end)));
        loop {
            match self.gb.run_until(&all, MachineCycles::PER_FRAME * 2).0 {
                Stop::Breakpoint(hit) if span_starts.contains(&hit) => {
                    if self.spanning.is_none() {
                        let end = LAG_SPANS[span_starts.iter().position(|&start| start == hit).unwrap()].1;
                        self.spanning = Some((breakpoint(end), self.acting));
                    }
                }
                Stop::Breakpoint(hit) if LAG_SPANS.iter().any(|&(_, end)| breakpoint(end) == hit) => {
                    if self.spanning.is_some_and(|(end, _)| end == hit) {
                        let (_, from) = self.spanning.take().unwrap();
                        self.entered.push(Loading::Lag(self.acting - from));
                    }
                }
                Stop::Breakpoint(hit) if lag.contains(&hit) && self.lagging.is_none() => {
                    let back = self.gb.return_address();
                    let bank = if (0x4000..0x8000).contains(&back) { self.gb.core().mmu().rom_bank() as u8 } else { 0 };
                    let back = Breakpoint::new(bank, back);
                    self.lagging = Some((self.gb.core().registers().sp, back, self.acting));
                    all.push(back);
                }
                // `VBlank`'s own far calls return through the same address, below the routine's stack.
                Stop::Breakpoint(hit) if self.lagging.is_some_and(|(sp, back, _)| hit == back && self.gb.core().registers().sp > sp) => {
                    let (_, back, from) = self.lagging.take().unwrap();
                    all.retain(|&at| at != back);
                    self.entered.push(Loading::Lag(self.acting - from));
                }
                Stop::Breakpoint(hit) if self.lagging.is_some_and(|(_, back, _)| hit == back) => {}
                Stop::Breakpoint(hit) if hit == copy_video => {
                    if COPY_VIDEO_DATA_CALLERS.iter().any(|&(from, to)| (from.address..to.address).contains(&self.gb.return_address())) {
                        let tiles = self.gb.core().registers().c as u16;
                        self.entered.push(Loading::CopyVideoData(tiles));
                    }
                }
                Stop::Breakpoint(hit) if hit == pause => {
                    // `NextTextCommand` pushed `hl` past the command byte.
                    let sp = self.gb.core().registers().sp;
                    if self.gb.core().mmu().read_u16_le(sp).wrapping_sub(1) == self.text {
                        self.entered.push(Loading::LeadingPause(self.acting));
                    }
                }
                Stop::Breakpoint(hit) if !points.contains(&hit) && LOADING.iter().any(|&(_, at)| breakpoint(at) == hit) => {
                    if hit == breakpoint(sym::PrintText) {
                        let registers = self.gb.core().registers();
                        self.text = u16::from_be_bytes([registers.h, registers.l]);
                    }
                    let mut loading = LOADING.iter().find(|&&(_, at)| breakpoint(at) == hit).unwrap().0;
                    if let Loading::CloseTextDisplay(font_loaded) = &mut loading {
                        *font_loaded = self.read(sym::wFontLoaded.address) & 1 != 0;
                    }
                    self.entered.push(loading);
                }
                Stop::Breakpoint(hit) if hit == random => {
                    let caller = self.gb.return_address();
                    let (stop, _) = self.gb.run_to_return(MachineCycles::PER_FRAME * 10);
                    assert!(matches!(stop, Stop::Returned { .. }));
                    let vblank = sym::VBlank.address;
                    if !(vblank..vblank + 0x80).contains(&caller) {
                        self.tape.push(self.gb.core().registers().a);
                        if READS_RANDOM_SUB.iter().any(|&(from, to)| (from.address..to.address).contains(&caller)) {
                            self.tape.push(self.read(sym::hRandomSub.address));
                        }
                    }
                }
                Stop::Breakpoint(hit) if hit == encounter => {
                    self.tape.push(self.read(sym::hRandomAdd.address));
                    self.tape.push(self.read(sym::hRandomSub.address));
                }
                Stop::Breakpoint(hit) if seams.contains(&hit) => {
                    let sp = self.gb.core().registers().sp;
                    let back = self.gb.core().mmu().read_u16_le(sp);
                    let registers = self.gb.core_mut().registers_mut();
                    registers.sp = sp + 2;
                    registers.pc = back;
                }
                Stop::Breakpoint(hit) => return hit,
                Stop::Budget => {}
                stop => panic!("{stop:?}"),
            }
        }
    }

    /// To the start of the next VBlank, and whether the frame polled for the player. An overworld
    /// poll runs on to the end of its pass, which a lag frame can push past the VBlank.
    fn frame(&mut self) -> Option<Kind> {
        self.acting += 1;
        let vblank = breakpoint(sym::VBlank);
        let overworld = breakpoint(local::OverworldLoopLessDelay::noDirectionButtonsPressed);
        let prompt = breakpoint(sym::JoypadLowSensitivity);
        let bubble = breakpoint(sym::EmotionBubble);
        let hit = self.run_to(&[vblank, overworld, prompt, bubble]);
        if hit == bubble {
            self.entered.push(Loading::EmotionBubble);
            self.run_to(&[vblank]);
            return Some(Kind::Bubble);
        }
        if hit == vblank {
            let map = self.read(sym::wCurMap.address);
            return (std::mem::replace(&mut self.map, map) != map).then_some(Kind::MapChange);
        }
        if hit == prompt {
            let cancel = sym::Evolution_CheckForCancel.address;
            if (cancel..cancel + 0x10).contains(&self.gb.return_address()) {
                self.run_to(&[vblank]);
                return None;
            }
            self.run_to(&[vblank]);
            return Some(Kind::Prompt);
        }
        let flags5 = self.read(sym::wStatusFlags5.address);
        let polled = flags5 & (1 << 7 | 1 << 5 | 1 << 0) == 0
            && self.read(sym::wMovementFlags.address) & 0b11 == 0
            && self.read(sym::wWalkCounter.address) == 0
            && self.read(sym::wJoyIgnore.address) == 0
            && self.read(sym::wNPCMovementScriptPointerTableNum.address) == 0;
        let end = if polled { breakpoint(sym::OverworldLoop) } else { vblank };
        while self.run_to(&[end, vblank]) != end {}
        if polled {
            self.run_to(&[vblank]);
            return Some(Kind::Overworld);
        }
        None
    }

    pub(super) fn to_poll(&mut self) -> (Kind, u32) {
        for frames in 1..BUDGET {
            if let Some(kind) = self.frame() {
                return (kind, frames);
            }
        }
        panic!("the cartridge never polled");
    }

    pub(super) fn act(&mut self, action: Action) -> (Kind, u32) {
        self.acting = 0;
        let mut frames = 0;
        match action {
            Action::Walk(button) => {
                let from = self.location().0;
                self.gb.hold_buttons(joypad(button));
                loop {
                    assert!(frames < BUDGET, "{action:?}: the cartridge never moved");
                    let polled = self.frame();
                    frames += 1;
                    if matches!(polled, Some(Kind::Prompt | Kind::Bubble)) {
                        // A turn in grass can start a battle before any step.
                        self.gb.hold_buttons(JoypadButtonState::default());
                        return (polled.unwrap(), frames);
                    }
                    if self.read(sym::wWalkCounter.address) != 0 || self.location().0 != from || self.read(0xFF47) != 0xE4 {
                        break;
                    }
                }
            }
            Action::Press(button, held) => {
                self.gb.hold_buttons(joypad(button));
                for _ in 0..held {
                    self.frame();
                    frames += 1;
                }
            }
            Action::Talk => {
                self.gb.hold_buttons(joypad(Joypad::A));
                let (vblank, talk) = (breakpoint(sym::VBlank), breakpoint(sym::DisplayTextID));
                loop {
                    assert!(frames < BUDGET, "the cartridge never talked");
                    if self.run_to(&[vblank, talk]) == talk {
                        break;
                    }
                    frames += 1;
                }
                self.gb.hold_buttons(JoypadButtonState::default());
                self.frame();
                frames += 1;
            }
        }
        self.gb.hold_buttons(JoypadButtonState::default());
        let (kind, more) = self.to_poll();
        (kind, frames + more)
    }

    fn location(&self) -> (Map, u8, u8, SpriteFacing) {
        let mmu = self.gb.core().mmu();
        (Map::from_repr(mmu.read_pointer(&sym::wCurMap)).unwrap(), mmu.read_pointer(&sym::wXCoord),
         mmu.read_pointer(&sym::wYCoord), SpriteFacing::from_repr(self.read(sym::wSpriteStateData1.address + 9)).unwrap())
    }

    fn sprites(&self) -> Sprites {
        let mmu = self.gb.core().mmu();
        std::array::from_fn(|slot| {
            let at = slot as u16 * STATE_BYTES;
            let data1 = mmu.read_slice(sym::wSpriteStateData1.address + at, 16);
            let data2 = mmu.read_slice(sym::wSpriteStateData2.address + at, 16);
            let map_data = if slot == 0 { [0, 0] } else {
                let entry = sym::wMapSpriteData.address + (slot as u16 - 1) * 2;
                [mmu.read(entry), mmu.read(entry + 1)]
            };
            SpriteState::from_bytes(&data1, &data2, map_data)
        })
    }

    fn standing(&self) -> Standing {
        let mmu = self.gb.core().mmu();
        Standing {
            player_direction: mmu.read_pointer(&sym::wPlayerDirection),
            moving_direction: mmu.read_pointer(&sym::wPlayerMovingDirection),
            last_stop_direction: mmu.read_pointer(&sym::wPlayerLastStopDirection),
            check_for_180_degree_turn: mmu.read_pointer(&sym::wCheckFor180DegreeTurn),
            standing_on_warp: mmu.read_pointer(&sym::wMovementFlags) & 1 << 2 != 0,
            destination_warp: mmu.read_pointer(&sym::wDestinationWarpID),
        }
    }

    /// The party and what the events move, unless `party_unsettled`.
    fn held(&self) -> Option<Held> {
        if party_unsettled(&self.gb) {
            return None;
        }
        let mmu = self.gb.core().mmu();
        let count = mmu.read_pointer(&sym::wPartyCount) as u16;
        let hp = (0..count).map(|i| mmu.read_u16_be(sym::wPartyMon1HP.address + i * PARTY_STRUCT)).collect();
        Some(held(&self.world(), hp))
    }

    fn seen(&mut self, kind: Kind, frames: u32) -> Seen {
        let mmu = self.gb.core().mmu();
        let bag = mmu.read_pointer(&sym::wNumBagItems) as u16;
        let mut seen = Seen {
            kind,
            frames,
            location: self.location(),
            screen: (0..18).map(|y| super::tile_row(&self.gb, y)).collect(),
            events: mmu.read_slice(sym::wEventFlags.address, 320),
            money: mmu.read_slice(sym::wPlayerMoney.address, 3).try_into().unwrap(),
            bag: (0..bag).map(|i| (mmu.read(sym::wBagItems.address + 2 * i), mmu.read(sym::wBagItems.address + 2 * i + 1))).collect(),
            held: self.held(),
            sprites: None,
            lcd: None,
            music: None,
            entered: std::mem::take(&mut self.entered),
        };
        self.fought |= self.read(sym::wIsInBattle.address) != 0;
        if kind == Kind::Prompt && self.read(sym::wIsInBattle.address) == 0 && !behind_a_page(&self.gb) {
            seen.sprites = Some((self.read(sym::wNumSprites.address), self.sprites()));
        }
        if kind == Kind::MapChange {
            seen.screen.clear();
        }
        if kind == Kind::Overworld {
            seen.sprites = Some((self.read(sym::wNumSprites.address), self.sprites()));
            if self.fought {
                seen.music = Some((self.read(sym::wAudioFadeOutCounterReloadValue.address), self.read(sym::wLastMusicSoundID.address)));
            }
            // VBlank copies `wShadowOAM` before it prepares the next, so a pass's sprites are on the
            // LCD two frames on.
            self.frame();
            self.frame();
            let vram = self.gb.core().mmu().read_vram_slice(0x9000, 0x60 * 16).unwrap();
            let animated = [0x03usize, 0x14].iter().flat_map(|&t| vram[t * 16..t * 16 + 16].to_vec()).collect();
            let lcd = self.gb.core().mmu().ppu().screenshot().pixels().map(|p| match p.0[0] {
                0xFF => 0,
                0xAA => 1,
                0x55 => 2,
                _ => 3,
            }).collect();
            seen.lcd = Some((lcd, animated));
        }
        seen
    }

    /// The recreation's world, from where the cartridge stands.
    pub fn world(&self) -> pokered::world::World {
        let mmu = self.gb.core().mmu();
        let mut world = super::item_menu::the_world(&self.gb);
        world.party = super::status_screen::the_party(&self.gb);
        let options = mmu.read_pointer(&sym::wOptions);
        world.options.text_speed = match options & 0xF {
            1 => TextSpeed::Fast,
            5 => TextSpeed::Slow,
            _ => TextSpeed::Medium,
        };
        world.options.battle_animation = options & 1 << 7 == 0;
        world.options.battle_style = if options & 1 << 6 != 0 { BattleStyle::Set } else { BattleStyle::Shift };
        let (map, x, y, facing) = self.location();
        world.location = Location {
            map, x, y, facing,
            last_map: Map::from_repr(mmu.read_pointer(&sym::wLastMap)).unwrap(),
            walk_bike_surf: mmu.read_pointer(&sym::wWalkBikeSurfState),
            hidden_objects: mmu.read_slice(sym::wToggleableObjectFlags.address, 32),
            towns_visited: u16::from_le_bytes([mmu.read(sym::wTownVisitedFlag.address), mmu.read(sym::wTownVisitedFlag.address + 1)]),
            last_blackout_map: Map::from_repr(mmu.read_pointer(&sym::wLastBlackoutMap)).unwrap(),
            repel_steps: mmu.read_pointer(&sym::wRepelRemainingSteps),
            always_on_bike: mmu.read_pointer(&sym::wStatusFlags6) & 1 << BIT_ALWAYS_ON_BIKE != 0,
            ahead: Ahead {
                tile: mmu.read_pointer(&sym::wTileInFrontOfPlayer),
                standing_on: mmu.read_pointer(&sym::wTilePlayerStandingOn),
                sprite: false,
            },
            strength_active: mmu.read_pointer(&sym::wStatusFlags1) & 1 << BIT_STRENGTH_ACTIVE != 0,
            used_field_move: None,
            fly_warp: None,
            escape_warp: false,
        };
        world.player_id = mmu.read_u16_be(sym::wPlayerID.address);
        world.badges = mmu.read_pointer(&sym::wObtainedBadges);
        world.rival_name = (0..7).map(|i| mmu.read(sym::wRivalName.address + i)).take_while(|&byte| byte != 0x50).collect();
        world.coins = [mmu.read(sym::wPlayerCoins.address), mmu.read(sym::wPlayerCoins.address + 1)];
        world.hidden_items = mmu.read_slice(sym::wObtainedHiddenItemsFlags.address, 14).try_into().unwrap();
        world.hidden_coins = mmu.read_slice(sym::wObtainedHiddenCoinsFlags.address, 2).try_into().unwrap();
        world.in_game_trades = u16::from_le_bytes([mmu.read(sym::wCompletedInGameTradeFlags.address), mmu.read(sym::wCompletedInGameTradeFlags.address + 1)]);
        world.used_pokecenter = mmu.read_pointer(&sym::wStatusFlags4) & 1 << 2 != 0;
        world.safari_steps = mmu.read_u16_be(sym::wSafariSteps.address);
        world.safari_balls = mmu.read_pointer(&sym::wNumSafariBalls);
        if mmu.read_pointer(&sym::wDayCareInUse) != 0 {
            world.day_care = Some(super::events::box_mon_at(&self.gb, sym::wDayCareMon.address, sym::wDayCareMonOT.address,
                sym::wDayCareMonName.address));
        }
        world.scripts.trash_cans = [mmu.read_pointer(&sym::wFirstLockTrashCanIndex), mmu.read_pointer(&sym::wSecondLockTrashCanIndex)];
        world.pokedex = Pokedex {
            owned: mmu.read_slice(sym::wPokedexOwned.address, 19).try_into().unwrap(),
            seen: mmu.read_slice(sym::wPokedexSeen.address, 19).try_into().unwrap(),
        };
        let scripts = &mut world.scripts;
        scripts.cur_map_script = mmu.read_pointer(&sym::wCurMapScript);
        scripts.rival_starter = mmu.read_pointer(&sym::wRivalStarter);
        scripts.maps.pallet_town.cur_script = mmu.read_pointer(&sym::wPalletTownCurScript);
        scripts.maps.pallet_town.oak_walked_to_player = mmu.read_pointer(&sym::wOakWalkedToPlayer) != 0;
        scripts.got_starter = mmu.read_pointer(&sym::wStatusFlags4) & 1 << BIT_GOT_STARTER != 0;
        let oaks_lab = &mut scripts.maps.oaks_lab;
        oaks_lab.cur_script = mmu.read_pointer(&sym::wOaksLabCurScript);
        oaks_lab.player_starter = mmu.read_pointer(&sym::wPlayerStarter);
        oaks_lab.rival_starter_temp = mmu.read_pointer(&sym::wRivalStarterTemp);
        oaks_lab.rival_starter_ball = mmu.read_pointer(&sym::wRivalStarterBallSpriteIndex);
        oaks_lab.saved_steps = mmu.read_pointer(&sym::wSavedNPCMovementDirections2Index);
        let scripts = &mut world.scripts;
        scripts.maps.reds_house_2f.cur_script = mmu.read_pointer(&sym::wRedsHouse2FCurScript);
        scripts.maps.viridian_forest.cur_script = mmu.read_pointer(&sym::wViridianForestCurScript);
        scripts.maps.viridian_mart.cur_script = mmu.read_pointer(&sym::wViridianMartCurScript);
        scripts.maps.vermilion_gym.cur_script = mmu.read_pointer(&sym::wVermilionGymCurScript);
        scripts.maps.viridian_gym.cur_script = mmu.read_pointer(&sym::wViridianGymCurScript);
        scripts.maps.celadon_gym.cur_script = mmu.read_pointer(&sym::wCeladonGymCurScript);
        scripts.maps.fuchsia_gym.cur_script = mmu.read_pointer(&sym::wFuchsiaGymCurScript);
        scripts.maps.saffron_gym.cur_script = mmu.read_pointer(&sym::wSaffronGymCurScript);
        world
    }

    /// What the recreation starts from, taken where the cartridge has just polled in the overworld.
    fn start(&self) -> (pokered::world::World, Overworld) {
        let overworld = Overworld::standing(self.sprites(), self.read(sym::wNumSprites.address), self.standing())
            .with_battle_flags(
                self.read(sym::wStatusFlags4.address) & 1 << 4 != 0,
                self.read(sym::wStatusFlags2.address) & 1 != 0,
                self.read(sym::wNumberOfNoRandomBattleStepsLeft.address),
            )
            .with_step_counter(self.read(sym::wStepCounter.address));
        (self.world(), overworld)
    }
}

fn overworld(game: &Game) -> Option<&Overworld> {
    game.modes().iter().rev().find_map(|mode| match mode {
        Mode::Overworld(overworld) => Some(overworld),
        _ => None,
    })
}

fn recreation_frame(game: &mut Game, input: Input) {
    game.frame(input);
}

/// The bubble's first tile in the first object, where `EmotionBubble` puts it.
fn bubble_up(game: &Game) -> bool {
    game.screen().sprites.first().is_some_and(|object| object.tile == 0xF8)
}

fn recreation_to_poll(game: &mut Game) -> (Kind, u32) {
    for frames in 1..BUDGET {
        let map = game.world().location.map;
        let bubbled = bubble_up(game);
        game.frame(Input::None);
        if game.world().location.map != map {
            return (Kind::MapChange, frames);
        }
        if !bubbled && bubble_up(game) {
            return (Kind::Bubble, frames);
        }
        if let Status::Waiting(decision) = game.status() {
            return (if decision == Decision::Overworld { Kind::Overworld } else { Kind::Prompt }, frames);
        }
    }
    panic!("the recreation never polled: {:?}", game.modes().last().map(Mode::status));
}

fn recreation_act(game: &mut Game, action: Action) -> (Kind, u32) {
    let mut frames = 0;
    match action {
        Action::Walk(button) => {
            let from = game.world().location.map;
            loop {
                assert!(frames < BUDGET, "{action:?}: the recreation never moved");
                let bubbled = bubble_up(game);
                recreation_frame(game, Input::Buttons(button));
                frames += 1;
                if !bubbled && bubble_up(game) {
                    return (Kind::Bubble, frames);
                }
                if matches!(game.status(), Status::Waiting(decision) if decision != Decision::Overworld) {
                    return (Kind::Prompt, frames);
                }
                if overworld(game).is_some_and(|o| o.walk_counter() != 0)
                    || game.world().location.map != from || game.screen().effects.bgp != 0xE4
                {
                    break;
                }
            }
        }
        Action::Press(button, held) => {
            let top = |game: &Game| game.modes().last().map(std::mem::discriminant);
            let before = (game.status(), top(game));
            for _ in 0..held {
                recreation_frame(game, Input::Buttons(button));
                frames += 1;
            }
            // A menu put up in the frame the press lands is polled in that frame. The cartridge's poll
            // there follows `HandleMenuInput`'s `Delay3`, so the frame run on to release the press is
            // not counted.
            if let Status::Waiting(decision) = game.status()
                && held > 0 && decision != Decision::Overworld && decision != Decision::Text
                && (game.status(), top(game)) != before
            {
                let (kind, more) = recreation_to_poll(game);
                return (kind, frames + more - 1);
            }
        }
        Action::Talk => loop {
            assert!(frames < BUDGET, "the recreation never talked");
            recreation_frame(game, Input::Buttons(Joypad::A));
            frames += 1;
            if game.modes().len() > 1 {
                break;
            }
        },
    }
    let (kind, more) = recreation_to_poll(game);
    (kind, frames + more)
}

/// The recreation's screen: what the UI covers, and the map through the view where it does not.
fn recreated_screen(game: &Game) -> Vec<Vec<u8>> {
    let in_battle = game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_)));
    let map = overworld(game).filter(|_| !in_battle).map(|overworld| overworld.view().tile_map());
    (0..18).map(|y| (0..20).map(|x| {
        game.ui().cover(x, y).or_else(|| map.map(|tiles| tiles[y * 20 + x])).unwrap_or(game.ui().get(x, y))
    }).collect()).collect()
}

fn recreation_seen(game: &mut Game, kind: Kind, frames: u32) -> (Seen, Option<pokered::gfx::Screen>) {
    let world = game.world();
    let location = &world.location;
    let mut seen = Seen {
        kind,
        frames,
        location: (location.map, location.x, location.y, location.facing),
        screen: recreated_screen(game),
        events: (0..320u16).map(|byte| (0..8).fold(0, |bits, bit| bits | (world.events.is_set(byte * 8 + bit) as u8) << bit)).collect(),
        money: world.money,
        bag: world.bag.items.iter().map(|item| (item.id as u8, item.quantity)).collect(),
        held: Some(held(world, world.party.iter().map(|mon| mon.mon.mon.hp).collect())),
        sprites: None,
        lcd: None,
        music: None,
        entered: Vec::new(),
    };
    let mut screen = None;
    let in_battle = game.modes().iter().any(|mode| matches!(mode, Mode::Battle(_)));
    if kind == Kind::Prompt && !in_battle {
        let overworld = overworld(game).expect("the overworld is under the text");
        seen.sprites = Some((overworld.num_sprites(), *overworld.sprites()));
    }
    if kind == Kind::MapChange {
        seen.screen.clear();
    }
    if kind == Kind::Overworld {
        let overworld = overworld(game).expect("the overworld is up");
        seen.sprites = Some((overworld.num_sprites(), *overworld.sprites()));
        seen.music = Some(game.audio().music_state());
        screen = Some(game.screen().clone());
        game.frame(Input::None);
        game.frame(Input::None);
    }
    (seen, screen)
}

/// Runs `choose` on the cartridge from `state` until it returns `None`, then replays its presses into
/// the recreation, comparing every poll. `prepare` edits the cartridge first.
pub(super) fn lockstep(state: &[u8], prepare: impl FnOnce(&mut Cartridge), choose: impl FnMut(usize, &Seen) -> Option<(Action, &'static str)>) {
    lockstep_from(state, prepare, true, choose);
}

/// `lockstep`, started where the fixture stands when `from_poll` is false: a map script about to run
/// on the next pass leaves no overworld poll before it. The start is then neither compared nor timed,
/// since neither machine knows how far into its pass the other is.
fn lockstep_from(state: &[u8], prepare: impl FnOnce(&mut Cartridge), from_poll: bool,
    mut choose: impl FnMut(usize, &Seen) -> Option<(Action, &'static str)>)
{
    let mut cartridge = Cartridge::from_state(state);
    // The SET style, so a trainer's next mon asks nothing a pressed A would answer by switching.
    let options = cartridge.read(sym::wOptions.address);
    cartridge.write(sym::wOptions.address, options | 1 << 6);
    while from_poll && cartridge.frame() != Some(Kind::Overworld) {}
    // Edits are made where both start, so the recreation's world has them too.
    prepare(&mut cartridge);
    let (world, overworld) = cartridge.start();
    // Where the start menu, the party menu and the bag reopen.
    let mmu = cartridge.gb.core().mmu();
    let saved_menu_items = [&sym::wBattleAndStartSavedMenuItem, &sym::wPartyAndBillsPCSavedMenuItem, &sym::wBagSavedMenuItem,
        &sym::wListScrollOffset].map(|at| mmu.read_pointer(at));
    cartridge.tape.clear();
    cartridge.entered.clear();
    let mut polls = vec![cartridge.seen(Kind::Overworld, 0)];
    let mut script = Vec::new();
    while let Some((action, what)) = choose(script.len(), polls.last().unwrap()) {
        if std::env::var("LOG_LOCKSTEP").is_ok() {
            let seen = polls.last().unwrap();
            let text = seen.screen.get(14).map(|row| super::battle::letters(row)).unwrap_or_default();
            println!("{} {:?} at {:?}: {action:?} ({what}) |{text}| tape {}", script.len(), seen.kind, seen.location, cartridge.tape.len());
        }
        let (kind, frames) = cartridge.act(action);
        script.push((action, what));
        polls.push(cartridge.seen(kind, frames));
        assert!(script.len() < 2000, "the scenario never ended");
    }
    let mut game = Game::new(world, GameRng::tape(cartridge.tape.clone()), Pacing::Faithful);
    let menu = game.menu_mut();
    [menu.battle_and_start, menu.party_and_bills, menu.bag_saved, menu.list_scroll] = saved_menu_items;
    game.push(Mode::Overworld(overworld));
    let (seen, screen) = recreation_seen(&mut game, Kind::Overworld, 0);
    if from_poll {
        compare(&polls[0], &seen, screen, "the start");
    }
    for (i, &(action, what)) in script.iter().enumerate() {
        let (kind, frames) = recreation_act(&mut game, action);
        let (seen, screen) = recreation_seen(&mut game, kind, frames);
        if std::env::var("LOG_LOCKSTEP").is_ok() {
            let next = &polls[i + 1];
            let text = |y: usize| next.screen.get(y).map(|row| super::battle::letters(row)).unwrap_or_default();
            println!("{i} {what}: {:?} cartridge {} recreation {} late {} |{}|{}|", kind, next.frames, frames,
                next.frames as i64 - frames as i64, text(14), text(16));
        }
        compare(&polls[i + 1], &seen, screen, &format!("{i}: {what}"));
        if from_poll || i > 0 {
            time(&polls[i], &polls[i + 1], frames, action, &game, &format!("{i}: {what}"));
        }
    }
}

/// The tile `UpdatePlayerSprite` reads to decide the player is behind drawn text, which a page
/// covering the whole screen makes true of every sprite: their slots then hold what the page drew
/// rather than where the overworld put them, and the map redrawn behind it settles them again.
fn behind_a_page(gb: &GameBoy) -> bool {
    super::tile_row(gb, 9)[8] >= MAP_TILESET_SIZE
}

/// `_AddPartyMon` counts the new mon in and asks `AskName`'s question before it writes the slot, so
/// through that question and the naming screen behind it the last party slot is still zero: the
/// cartridge's party is not readable there, let alone settled enough to compare.
fn party_unsettled(gb: &GameBoy) -> bool {
    let mmu = gb.core().mmu();
    let count = mmu.read_pointer(&sym::wPartyCount) as u16;
    count > 0 && mmu.read(sym::wPartyMon1.address + PARTY_STRUCT * (count - 1)) == 0
}

fn held(world: &pokered::world::World, party_hp: Vec<u16>) -> Held {
    Held {
        badges: world.badges,
        party: world.party.iter().map(|mon| (mon.mon.mon.species, mon.mon.level)).collect(),
        party_hp,
        coins: world.coins,
        day_care: world.day_care.as_ref().map(|mon| (mon.mon.species, mon.mon.box_level)),
        trades: world.in_game_trades,
        hidden: world.hidden_items.iter().chain(&world.hidden_coins).copied().collect(),
        last_blackout_map: world.location.last_blackout_map,
    }
}

fn compare(cartridge: &Seen, recreation: &Seen, screen: Option<pokered::gfx::Screen>, what: &str) {
    assert_eq!(recreation.kind, cartridge.kind, "{what}: what is waited on");
    // A battle's prompts are the battle lockstep's to compare: its text updates the player's sprite
    // and pays the prize at moments of its own.
    let in_battle = recreation.kind == Kind::Prompt && recreation.sprites.is_none();
    // The frame `wCurMap` changes comes before the load that resets the player's sprite, as
    // `PrepareForSpecialWarp` does before `SpecialEnterMap`, so its facing is still the old map's.
    let facing_unsettled = in_battle || recreation.kind == Kind::MapChange;
    let location = |seen: &Seen| if facing_unsettled { (seen.location.0, seen.location.1, seen.location.2, SpriteFacing::Down) } else { seen.location };
    assert_eq!(location(recreation), location(cartridge), "{what}: where the player stands");
    if recreation.screen != cartridge.screen {
        let rows: Vec<String> = (0..18).filter(|&y| recreation.screen[y] != cartridge.screen[y])
            .map(|y| format!("row {y}\n  cartridge  {:02X?}\n  recreation {:02X?}", cartridge.screen[y], recreation.screen[y]))
            .collect();
        panic!("{what}: the screen differs\n{}", rows.join("\n"));
    }
    if let Some(event) = (0..cartridge.events.len() * 8).find(|&e| (cartridge.events[e / 8] ^ recreation.events[e / 8]) & 1 << (e % 8) != 0) {
        panic!("{what}: event {event:#X} is {} on the cartridge", cartridge.events[event / 8] & 1 << (event % 8) != 0);
    }
    if !in_battle {
        assert_eq!((recreation.money, &recreation.bag), (cartridge.money, &cartridge.bag), "{what}: money and bag");
        // Only where the cartridge has a party to compare against: `party_unsettled`.
        if cartridge.held.is_some() {
            assert_eq!(recreation.held, cartridge.held, "{what}: the party, badges, coins, day care, trades and hidden things");
        }
    }
    if cartridge.music.is_some() {
        assert_eq!(recreation.music, cartridge.music, "{what}: the fade after the battle and `wLastMusicSoundID`");
    }
    if let (Some((count, theirs)), Some((_, ours))) = (&cartridge.sprites, &recreation.sprites) {
        let compared = |sprites: &Sprites| sprites.iter().take(*count as usize + 1)
            .map(|&s| SpriteState { y_adjusted: 0, x_adjusted: 0, ..s }).collect::<Vec<_>>();
        let (ours, theirs) = (compared(ours), compared(theirs));
        if let Some(slot) = (0..ours.len()).find(|&slot| ours[slot] != theirs[slot]) {
            panic!("{what}: sprite slot {slot}\ncartridge  {:?}\nrecreation {:?}", theirs[slot], ours[slot]);
        }
    }
    if let (Some((lcd, animated)), Some(mut screen)) = (&cartridge.lcd, screen) {
        screen.tiles.load(V_CHARS2 + 0x03, &animated[..16]);
        screen.tiles.load(V_CHARS2 + 0x14, &animated[16..]);
        let ours = screen.frame().shades;
        let wrong: Vec<(usize, usize)> = (0..lcd.len()).filter(|&i| lcd[i] != ours[i]).map(|i| (i % WIDTH, i / WIDTH)).collect();
        assert!(wrong.is_empty(), "{what}: {} pixels differ, first {:?}", wrong.len(), &wrong[..wrong.len().min(8)]);
    }
}

/// The step from `from` to `to` against the loading the cartridge entered on the way: late by that
/// and at most two lag frames, and never early. A step that turns the LCD off, or that starts from
/// or runs through a battle, is untimed.
fn time(from: &Seen, to: &Seen, recreation: u32, action: Action, game: &Game, what: &str) {
    let in_battle = |seen: &Seen| seen.kind == Kind::Prompt && seen.sprites.is_none();
    let log = std::env::var("LOG_TIMING").is_ok();
    if in_battle(from) || in_battle(to) || to.kind == Kind::MapChange && to.entered.is_empty() && from.location.0 != to.location.0 {
        if log { println!("untimed {what}: a battle or a map change {:?}", to.entered); }
        return;
    }
    // Loading happens where the player stood when the step began.
    let (map, x, y, _) = from.location;
    let Some(mut loading) = to.entered.iter().map(|loading| loading.frames(map, x, y)).sum::<Option<u32>>() else {
        if log { println!("untimed {what}: {:?}", to.entered); }
        return;
    };
    // `LIST` already prices the cursor of the `HandleMenuInput` that `DisplayListMenuID` calls.
    let lists = to.entered.iter().filter(|&&l| l == Loading::DisplayListMenuId).count() as u32;
    loading -= lists * super::CURSOR;
    // A press still held when the first letter of a new text prints hurries it. A text closed on the
    // way spends the press first, since the recreation's own `Delay3` after it outlasts the press.
    let pressed_a = matches!(action, Action::Talk) || matches!(action, Action::Press(button, _) if button.contains(Joypad::A));
    let opens_text = to.entered.iter()
        .find(|&&l| matches!(l, Loading::CloseTextDisplay(_) | Loading::DisplayTextIdInit | Loading::PrintText))
        .is_some_and(|&l| !matches!(l, Loading::CloseTextDisplay(_)));
    if pressed_a && opens_text {
        loading += super::hurried_letter(game);
    }
    // And a pause at its start reached with nothing but loading on the way, which the box's `Delay3`
    // let the press outlast on the cartridge.
    if let Action::Press(button, held) = action
        && button.contains(Joypad::A)
    {
        let mut before = 0;
        for &entry in &to.entered {
            match entry {
                Loading::LeadingPause(at) if at <= held + before + 2 => loading += TX_PAUSE_FRAMES,
                _ => before += entry.frames(map, x, y).unwrap_or(0),
            }
        }
    }
    let steps = to.entered.iter().filter(|&&l| l == Loading::Step).count() as i64;
    let late = to.frames as i64 - recreation as i64;
    if log {
        println!("timing {what}: late {late} loading {loading} slack {} {:?}", late - loading as i64, to.entered);
    }
    assert!((loading as i64..=loading as i64 + 2 + steps).contains(&late),
        "{what}: the cartridge took {} frames and the recreation {recreation}, {late} late for {loading} of loading {:?}", to.frames, to.entered);
}

/// A button pressed at a prompt: one frame is an edge, and the prompt reads it in that frame.
pub(super) const PROMPT: Action = Action::Press(Joypad::A, 1);
/// Nothing pressed: the next poll.
pub(super) const WAIT: Action = Action::Press(Joypad::empty(), 1);

/// Where a sprite stands in squares.
pub(super) fn square(sprite: &SpriteState) -> (u8, u8) {
    (sprite.map_x.wrapping_sub(4), sprite.map_y.wrapping_sub(4))
}

/// Walks `route` at overworld polls and presses A at every prompt, which in a battle fights with the
/// first move, until `done`.
pub(super) fn walk_and_answer(route: &'static [(Action, &'static str)], mut done: impl FnMut(&Seen) -> bool)
    -> impl FnMut(usize, &Seen) -> Option<(Action, &'static str)>
{
    let mut walked = 0;
    move |_, seen| {
        if done(seen) {
            return None;
        }
        match seen.kind {
            Kind::Prompt => Some((PROMPT, "a prompt")),
            Kind::MapChange => Some((Action::Press(Joypad::empty(), 0), "a new map")),
            Kind::Bubble => Some((Action::Press(Joypad::empty(), 0), "a bubble")),
            Kind::Overworld => {
                let step = route.get(walked).copied();
                walked += 1;
                step
            }
        }
    }
}

/// Viridian Forest from its entrance: right along the bottom and up the column past the second Bug
/// Catcher's sight, who sees the player, walks up, speaks and fights; then the player turns to him
/// and he only talks.
#[test]
fn a_bug_catcher_sees_the_player_walks_up_battles_and_after_only_talks() {
    const ROUTE: &[(Action, &str)] = &[
        (Action::Walk(Joypad::RIGHT), "right, turning"),
        (Action::Walk(Joypad::RIGHT), "right"), (Action::Walk(Joypad::RIGHT), "right"), (Action::Walk(Joypad::RIGHT), "right"),
        (Action::Walk(Joypad::RIGHT), "right"), (Action::Walk(Joypad::RIGHT), "right"), (Action::Walk(Joypad::RIGHT), "right"),
        (Action::Walk(Joypad::RIGHT), "right"), (Action::Walk(Joypad::RIGHT), "right to (26, 43)"),
        (Action::Walk(Joypad::UP), "up, turning"),
        (Action::Walk(Joypad::UP), "up"), (Action::Walk(Joypad::UP), "up"), (Action::Walk(Joypad::UP), "up"),
        (Action::Walk(Joypad::UP), "up"), (Action::Walk(Joypad::UP), "up"), (Action::Walk(Joypad::UP), "up"),
        (Action::Walk(Joypad::UP), "up"), (Action::Walk(Joypad::UP), "up"),
        (Action::Walk(Joypad::UP), "up into the Bug Catcher's sight"),
        (Action::Press(Joypad::RIGHT, 2), "turn to him"),
        (Action::Talk, "talk to him"),
    ];
    let event = poke_core::symbols::pokered_events::EVENT_BEAT_VIRIDIAN_FOREST_TRAINER_0;
    let mut spoken = 0;
    lockstep(include_bytes!("../pokemon/data/viridian-forest.bin"), |_| {}, walk_and_answer(ROUTE, move |seen| {
        let beaten = seen.events[event as usize / 8] & 1 << (event % 8) != 0;
        if beaten && seen.kind == Kind::Overworld {
            spoken += 1;
        }
        spoken == 3
    }));
}

/// Route 1's clerk: the player waits beside his column for him to come alongside, turns to him and
/// takes the Potion.
#[test]
fn the_route_1_clerk_gives_a_potion_as_the_cartridge_does() {
    let event = poke_core::symbols::pokered_events::EVENT_GOT_POTION_SAMPLE;
    let got = move |seen: &Seen| seen.events[event as usize / 8] & 1 << (event % 8) != 0;
    let clear = move |cartridge: &mut Cartridge| {
        let at = sym::wEventFlags.address + event / 8;
        let flags = cartridge.read(at);
        cartridge.write(at, flags & !(1 << (event % 8)));
    };
    let mut asked = false;
    lockstep(include_bytes!("../pokemon/data/route1-state.bin"), clear, move |_, seen| match seen.kind {
        Kind::Prompt => {
            asked = true;
            Some((PROMPT, "a prompt"))
        }
        Kind::MapChange | Kind::Bubble => None,
        Kind::Overworld if got(seen) && asked => None,
        Kind::Overworld => {
            let (_, x, y, facing) = seen.location;
            let clerk = square(&seen.sprites.as_ref().expect("an overworld poll has sprites").1[1]);
            if clerk.0 != x + 1 || clerk.1 != y && !(20..=26).contains(&clerk.1) {
                Some((WAIT, "wait for the clerk"))
            } else if clerk.1 < y {
                Some((Action::Walk(Joypad::UP), "up beside the clerk"))
            } else if clerk.1 > y {
                Some((Action::Walk(Joypad::DOWN), "down beside the clerk"))
            } else if facing != SpriteFacing::Right {
                Some((Action::Press(Joypad::RIGHT, 2), "turn to the clerk"))
            } else {
                Some((Action::Talk, "talk to the clerk"))
            }
        }
    });
}

/// A new game down the stairs, out of the house and up to the grass, where Oak stops the player.
const TO_THE_GRASS: &[(Action, &str)] = &[
        (Action::Walk(Joypad::RIGHT), "right, turning"), (Action::Walk(Joypad::RIGHT), "right to (5, 6)"),
        (Action::Walk(Joypad::UP), "up, turning"), (Action::Walk(Joypad::UP), "up"), (Action::Walk(Joypad::UP), "up"),
        (Action::Walk(Joypad::UP), "up to (5, 2)"),
        (Action::Walk(Joypad::RIGHT), "right, turning"), (Action::Walk(Joypad::RIGHT), "right to (7, 2)"),
        (Action::Walk(Joypad::UP), "up the stairs"),
        (Action::Walk(Joypad::DOWN), "down, turning"), (Action::Walk(Joypad::DOWN), "down"), (Action::Walk(Joypad::DOWN), "down"),
        (Action::Walk(Joypad::DOWN), "down"), (Action::Walk(Joypad::DOWN), "down"), (Action::Walk(Joypad::DOWN), "down to (7, 7)"),
        (Action::Walk(Joypad::LEFT), "left, turning"), (Action::Walk(Joypad::LEFT), "left"), (Action::Walk(Joypad::LEFT), "left"),
        (Action::Walk(Joypad::LEFT), "left to the mat"),
        (Action::Walk(Joypad::DOWN), "out of the door"),
        (Action::Walk(Joypad::RIGHT), "right, turning"), (Action::Walk(Joypad::RIGHT), "right"), (Action::Walk(Joypad::RIGHT), "right"),
        (Action::Walk(Joypad::RIGHT), "right"), (Action::Walk(Joypad::RIGHT), "right to (10, 6)"),
        (Action::Walk(Joypad::UP), "up, turning"), (Action::Walk(Joypad::UP), "up"), (Action::Walk(Joypad::UP), "up"),
        (Action::Walk(Joypad::UP), "up"), (Action::Walk(Joypad::UP), "up to the grass"),
];

/// A new game walked down the stairs, out of the house and up to the grass, where Oak stops the
/// player, walks up, and leads them into his lab.
#[test]
fn oak_stops_the_player_at_the_grass_and_leads_them_to_the_lab_as_the_cartridge_does() {
    lockstep(include_bytes!("../pokemon/data/start-of-game-state.bin"), |_| {}, walk_and_answer(TO_THE_GRASS, |seen| {
        seen.kind == Kind::MapChange && seen.location.0 == Map::OaksLab
    }));
}

/// Whether an event flag is set.
fn event_set(seen: &Seen, event: u16) -> bool {
    seen.events[event as usize / 8] & 1 << (event % 8) != 0
}

/// The same walk carried on into the lab: Oak comes in behind the player and the player is walked the
/// eight squares up to the table, free again in front of the balls. The stretch that loads the lab is
/// untimed.
#[test]
fn oaks_lab_walks_the_player_to_the_table_as_the_cartridge_does() {
    lockstep(include_bytes!("../pokemon/data/start-of-game-state.bin"), |_| {}, walk_and_answer(TO_THE_GRASS, |seen| {
        seen.kind == Kind::Overworld && seen.location.0 == Map::OaksLab
    }));
}

/// Oak's speech from the table, which is four texts with a `Delay3` between each.
#[test]
fn oaks_lab_gives_the_choose_mon_speech_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::EVENT_OAK_ASKED_TO_CHOOSE_MON;
    let mut walked = 0;
    lockstep(include_bytes!("../pokemon/data/start-of-game-state.bin"), |_| {}, move |_, seen| match seen.kind {
        Kind::Prompt => Some((PROMPT, "a prompt")),
        Kind::MapChange => Some((Action::Press(Joypad::empty(), 0), "a new map")),
        Kind::Bubble => Some((Action::Press(Joypad::empty(), 0), "a bubble")),
        // The lab's script drives the player, so every free frame on the way is only waited out.
        Kind::Overworld if seen.location.0 == Map::OaksLab => {
            (!event_set(seen, EVENT_OAK_ASKED_TO_CHOOSE_MON)).then_some((WAIT, "wait for the lab's script"))
        }
        Kind::Overworld => {
            let step = TO_THE_GRASS.get(walked).copied();
            walked += 1;
            step
        }
    });
}

/// Route 1's grass: right from the clerk's column to the patch, then up and down it until a wild mon
/// appears, the battle fought with the first move, and a step on after.
#[test]
fn a_wild_mon_appears_in_the_grass_and_the_walk_goes_on_as_the_cartridge_does() {
    let mut fought = false;
    let mut after = 0;
    lockstep(include_bytes!("../pokemon/data/route1-state.bin"), |_| {}, move |_, seen| {
        match seen.kind {
            Kind::Prompt => {
                fought = true;
                return Some((PROMPT, "a prompt"));
            }
            Kind::MapChange | Kind::Bubble => return None,
            Kind::Overworld => {}
        }
        let (_, x, y, _) = seen.location;
        if fought {
            after += 1;
            return (after < 3).then_some((Action::Walk(if y == 24 { Joypad::DOWN } else { Joypad::UP }), "on after the battle"));
        }
        let clerk = square(&seen.sprites.as_ref().expect("an overworld poll has sprites").1[1]);
        Some(if x < 12 {
            if clerk == (x + 1, y) { (WAIT, "wait for the clerk to pass") } else { (Action::Walk(Joypad::RIGHT), "right to the grass") }
        } else if y == 24 {
            (Action::Walk(Joypad::DOWN), "down the grass")
        } else {
            (Action::Walk(Joypad::UP), "up the grass")
        })
    });
}

/// Viridian City's mart entered before the parcel: out of the door and back in, the clerk calls the
/// player back, the player is walked to the counter on simulated presses, and the parcel is handed
/// over. The fixture has the parcel delivered, which the cartridge is made to forget.
#[test]
fn the_viridian_clerk_hands_over_oak_s_parcel_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::{EVENT_GOT_OAKS_PARCEL, EVENT_OAK_GOT_PARCEL};
    let forget = |cartridge: &mut Cartridge| {
        for event in [EVENT_GOT_OAKS_PARCEL, EVENT_OAK_GOT_PARCEL] {
            let at = sym::wEventFlags.address + event / 8;
            let flags = cartridge.read(at);
            cartridge.write(at, flags & !(1 << (event % 8)));
        }
        cartridge.write(sym::wViridianMartCurScript.address, 0);
    };
    let got = |seen: &Seen| seen.events[EVENT_GOT_OAKS_PARCEL as usize / 8] & 1 << (EVENT_GOT_OAKS_PARCEL % 8) != 0;
    let mut walked = 0;
    lockstep(include_bytes!("../pokemon/data/viridian-city-pokemart-shopping.bin"), forget, move |_, seen| match seen.kind {
        Kind::Overworld if got(seen) => None,
        Kind::Prompt => Some((PROMPT, "a prompt")),
        Kind::MapChange | Kind::Bubble => Some((Action::Press(Joypad::empty(), 0), "a new map")),
        Kind::Overworld => {
            walked += 1;
            match walked {
                1 => Some((Action::Walk(Joypad::DOWN), "out of the door")),
                2 => Some((Action::Walk(Joypad::UP), "back in")),
                _ => Some((WAIT, "on")),
            }
        }
    });
}

/// Plays `script` in order, whatever each poll turns out to be.
fn in_order(script: &'static [(Action, &'static str)]) -> impl FnMut(usize, &Seen) -> Option<(Action, &'static str)> {
    move |i, _| script.get(i).copied()
}

/// START, which the overworld reads as a new press on the pass after its poll.
const START: Action = Action::Press(Joypad::START, 2);
const B: Action = Action::Press(Joypad::B, 1);

/// Pallet Town's pond, where the fixture stands on the shore facing the water with the Bicycle in its
/// bag: the bike moved to the top of the bag, and the start menu's cursor left on `ITEM`.
fn bicycle_on_top(cartridge: &mut Cartridge) {
    const BICYCLE: u8 = 6;
    let items = sym::wBagItems.address;
    let count = cartridge.read(sym::wNumBagItems.address) as u16;
    let slot = (0..count).find(|&i| cartridge.read(items + 2 * i) == BICYCLE).expect("the fixture has the Bicycle");
    for byte in 0..2 {
        let (top, bike) = (cartridge.read(items + byte), cartridge.read(items + 2 * slot + byte));
        cartridge.write(items + byte, bike);
        cartridge.write(items + 2 * slot + byte, top);
    }
    cartridge.write(sym::wBattleAndStartSavedMenuItem.address, 2);
    cartridge.write(sym::wBagSavedMenuItem.address, 0);
    cartridge.write(sym::wListScrollOffset.address, 0);
}

/// The Bicycle from the bag at Pallet Town's pond: on, ridden round the pond's corner and back, and
/// off again, every poll matched and timed.
#[test]
fn the_bicycle_is_got_on_ridden_and_put_away_as_the_cartridge_does() {
    const SCRIPT: &[(Action, &str)] = &[
        (START, "START"), (PROMPT, "ITEM"), (PROMPT, "the Bicycle"), (PROMPT, "got on the BICYCLE!"),
        (Action::Walk(Joypad::RIGHT), "right, turning"), (Action::Walk(Joypad::RIGHT), "right"),
        (Action::Walk(Joypad::RIGHT), "right"), (Action::Walk(Joypad::UP), "up, turning"), (Action::Walk(Joypad::UP), "up"),
        (Action::Walk(Joypad::LEFT), "left, turning"), (Action::Walk(Joypad::LEFT), "left"),
        (START, "START"), (PROMPT, "ITEM"), (PROMPT, "the Bicycle"), (PROMPT, "got off the BICYCLE."),
        (Action::Walk(Joypad::DOWN), "down, turning"), (Action::Walk(Joypad::DOWN), "down on foot"),
    ];
    lockstep(include_bytes!("../pokemon/data/postgame-fishing.bin"), bicycle_on_top, in_order(SCRIPT));
}

/// The fixture's third mon knows SURF and the party has every badge: the start menu's cursor left on
/// `POKéMON` and the party's on that mon.
fn surfer_chosen(cartridge: &mut Cartridge) {
    cartridge.write(sym::wBattleAndStartSavedMenuItem.address, 1);
    cartridge.write(sym::wPartyAndBillsPCSavedMenuItem.address, 2);
}

/// SURF from the party menu at Pallet Town's pond: onto the water, round in it, off onto the shore by
/// SURF again, refused on land, back on, and off by swimming into the shore.
#[test]
fn surf_goes_onto_the_water_and_off_it_as_the_cartridge_does() {
    const SURF: [(Action, &str); 4] = [(START, "START"), (PROMPT, "POKéMON"), (PROMPT, "the surfer"), (PROMPT, "SURF")];
    const SCRIPT: &[(Action, &str)] = &[
        SURF[0], SURF[1], SURF[2], SURF[3], (PROMPT, "got on"),
        (Action::Walk(Joypad::DOWN), "down in the water"), (Action::Walk(Joypad::RIGHT), "right, turning"),
        (Action::Walk(Joypad::UP), "up, turning, to the shore's edge"),
        SURF[0], SURF[1], SURF[2], SURF[3],
        SURF[0], SURF[1], SURF[2], SURF[3], (PROMPT, "no SURFing here"), (B, "back out of the party menu"), (B, "and the start menu"),
        (Action::Press(Joypad::DOWN, 2), "into the water, a wall on foot"),
        SURF[0], SURF[1], SURF[2], SURF[3], (PROMPT, "got on"),
        (Action::Walk(Joypad::UP), "up onto the shore, off the water"),
    ];
    lockstep(include_bytes!("../pokemon/data/postgame-fishing.bin"), surfer_chosen, in_order(SCRIPT));
}


/// The fixture's fourth mon knows CUT and the party has the Cascade Badge: the start menu's cursor
/// left on `POKéMON` and the party's on that mon.
fn cutter_chosen(cartridge: &mut Cartridge) {
    cartridge.write(sym::wBattleAndStartSavedMenuItem.address, 1);
    cartridge.write(sym::wPartyAndBillsPCSavedMenuItem.address, 3);
}

/// CUT from the party menu on Route 8, where the fixture already stands facing a tree: the tree's
/// block is swapped for the one without it, and the two steps after prove the map really changed.
/// The stretch that cuts is untimed, since `ReloadMapSpriteTilePatterns` turns the LCD off.
#[test]
fn cut_takes_a_tree_down_as_the_cartridge_does() {
    const SCRIPT: &[(Action, &str)] = &[
        (START, "START"), (PROMPT, "POKéMON"), (PROMPT, "the cutter"), (PROMPT, "CUT"),
        (Action::Walk(Joypad::LEFT), "left, where the tree was"),
        (Action::Walk(Joypad::LEFT), "left again"),
    ];
    lockstep(include_bytes!("../pokemon/data/route8-cut-trees.bin"), cutter_chosen, in_order(SCRIPT));
}

/// The fixture's third mon knows STRENGTH and the party has every badge: the start menu's cursor
/// left on `POKéMON` and the party's on that mon, with `BIT_NO_BATTLES` set so the walk to the
/// boulder reaches it. A wild battle on the way is not just noise: the battle menu shares
/// `wBattleAndStartSavedMenuItem` with the start menu, so it moves the cursor this sets.
fn the_strong_one_chosen(cartridge: &mut Cartridge) {
    cartridge.write(sym::wBattleAndStartSavedMenuItem.address, 1);
    cartridge.write(sym::wPartyAndBillsPCSavedMenuItem.address, 2);
    let flags4 = cartridge.read(sym::wStatusFlags4.address);
    cartridge.write(sym::wStatusFlags4.address, flags4 | 1 << BIT_NO_BATTLES);
}

/// STRENGTH on Victory Road 1F, then the boulder at (5, 15) shoved up from the square below it: the
/// walk there, the two texts, the push that only arms and the one that moves, and the step into
/// where the boulder stood. The stretch that arms STRENGTH is untimed, since `.goBackToMap` turns
/// the LCD off; the pushes are timed.
#[test]
fn strength_shoves_a_boulder_as_the_cartridge_does() {
    // The way from where the fixture stands to the square below the boulder, a leg at a time.
    const WAY: [(u8, u8); 4] = [(1, 15), (2, 15), (2, 16), (5, 16)];
    let mut leg = 0;
    let mut pushed = 0;
    lockstep(include_bytes!("../pokemon/data/vr1f-strength.bin"), the_strong_one_chosen, move |_, seen| {
        match seen.kind {
            Kind::Prompt => return Some((PROMPT, "a prompt")),
            Kind::MapChange | Kind::Bubble => return Some((WAIT, "on")),
            Kind::Overworld => {}
        }
        let (_, x, y, _) = seen.location;
        while leg < WAY.len() && WAY[leg] == (x, y) {
            leg += 1;
        }
        if let Some(&(to_x, to_y)) = WAY.get(leg) {
            let button = if to_x > x { Joypad::RIGHT } else if to_x < x { Joypad::LEFT }
                else if to_y > y { Joypad::DOWN } else { Joypad::UP };
            return Some((Action::Walk(button), "on to the boulder"));
        }
        pushed += 1;
        match pushed {
            1 => Some((START, "START")),
            2 => Some((Action::Press(Joypad::UP, 8), "into the boulder, which moves on the second push")),
            3 => Some((Action::Walk(Joypad::UP), "into where it stood")),
            _ => None,
        }
    });
}

/// The fixture's party member that knows FLY: the start menu's cursor left on `POKéMON` and the
/// party's on that mon.
fn flier_chosen(cartridge: &mut Cartridge) {
    const FLY: u8 = 19;
    const PARTY_MON_BYTES: u16 = 0x2C;
    let count = cartridge.read(sym::wPartyCount.address) as u16;
    let slot = (0..count)
        .find(|&i| (0u16..4).any(|m| cartridge.read(sym::wPartyMon1Moves.address + PARTY_MON_BYTES * i + m) == FLY))
        .expect("the fixture has a mon that knows FLY");
    cartridge.write(sym::wBattleAndStartSavedMenuItem.address, 1);
    cartridge.write(sym::wPartyAndBillsPCSavedMenuItem.address, slot as u8);
}

/// FLY from the party menu at Pallet Town: the town map's list stepped up one from PALLET TOWN, the
/// flight out and the landing in Viridian City, every poll matched. The flight is untimed, since it
/// loads a map.
#[test]
fn fly_lands_in_the_town_chosen_as_the_cartridge_does() {
    const OPENING: [(Action, &str); 6] = [
        (START, "START"), (PROMPT, "POKéMON"), (PROMPT, "the flier"), (PROMPT, "FLY"),
        (Action::Press(Joypad::UP, 1), "up the list, from PALLET TOWN to VIRIDIAN CITY"),
        (PROMPT, "fly there"),
    ];
    lockstep(include_bytes!("../pokemon/data/postgame-fishing.bin"), flier_chosen, |i, seen| {
        match OPENING.get(i) {
            Some(&step) => Some(step),
            // The flight has no poll of its own, so it is walked through to the first one after it.
            None if seen.kind != Kind::Overworld => Some((WAIT, "the flight")),
            None => {
                assert_eq!(seen.location.0, Map::ViridianCity, "the flight landed");
                None
            }
        }
    });
}

/// The Charmander ball taken from the table, which leaves the rival the Squirtle beside it: the
/// Pokédex page the ball shows, the choice, and the nickname declined. B answers `AskName`, since the
/// naming screen it opens is the naming lockstep's, and the polls of its question are the ones
/// `party_unsettled` covers.
#[test]
#[ignore = "the recreation leaves the dex page's blank tiles on screen: the cartridge's \
            `OaksLabShowPokeBallPokemonScript` calls `ReloadMapData` after `StarterDex`, and \
            `scripts/oaks_lab.rs`'s `Label::BallDexShown` goes straight to the offer"]
fn the_charmander_ball_is_taken_as_the_cartridge_does() {
    lockstep(include_bytes!("../pokemon/data/branch-oaks-lab.bin"), |_| {}, |_, seen| match seen.kind {
        Kind::Prompt if seen.held.is_none() => Some((Action::Press(Joypad::B, 1), "no nickname")),
        Kind::Prompt => Some((PROMPT, "a prompt")),
        Kind::MapChange | Kind::Bubble => Some((Action::Press(Joypad::empty(), 0), "the lab's script")),
        Kind::Overworld if !seen.settled().party.is_empty() => None,
        Kind::Overworld if seen.location.3 != SpriteFacing::Right => Some((Action::Press(Joypad::RIGHT, 2), "turn to the ball")),
        Kind::Overworld => Some((Action::Talk, "the Charmander ball")),
    });
}


/// Vermilion Gym, the fixture standing in front of LT.SURGE with his badge already won, which the
/// cartridge is made to forget: he is talked to, fought with the first move, and hands over the
/// Thunder Badge and TM24.
#[test]
fn lt_surge_hands_over_the_thunder_badge_and_tm24_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::{EVENT_BEAT_LT_SURGE, EVENT_GOT_TM24};
    const BIT_THUNDERBADGE: u8 = 2;
    let forget = |cartridge: &mut Cartridge| {
        for event in [EVENT_BEAT_LT_SURGE, EVENT_GOT_TM24] {
            let at = sym::wEventFlags.address + event / 8;
            let flags = cartridge.read(at);
            cartridge.write(at, flags & !(1 << (event % 8)));
        }
        let badges = cartridge.read(sym::wObtainedBadges.address);
        cartridge.write(sym::wObtainedBadges.address, badges & !(1 << BIT_THUNDERBADGE));
    };
    lockstep(include_bytes!("../pokemon/data/post-thunder-badge.bin"), forget, |_, seen| match seen.kind {
        Kind::Prompt => Some((PROMPT, "a prompt")),
        Kind::MapChange | Kind::Bubble => Some((Action::Press(Joypad::empty(), 0), "on")),
        Kind::Overworld if event_set(seen, EVENT_GOT_TM24) => None,
        Kind::Overworld => Some((Action::Talk, "talk to LT.SURGE")),
    });
}

/// The party back to full HP and no status, since a fixture saved after a gym leader's battle is in
/// no state to fight it again.
fn heal(cartridge: &mut Cartridge) {
    for i in 0..cartridge.read(sym::wPartyCount.address) as u16 {
        let mon = PARTY_STRUCT * i;
        let max = [0, 1].map(|byte| cartridge.read(sym::wPartyMon1MaxHP.address + mon + byte));
        for byte in 0..2 {
            cartridge.write(sym::wPartyMon1HP.address + mon + byte, max[byte as usize]);
        }
        cartridge.write(sym::wPartyMon1Status.address + mon, 0);
    }
}

/// The last thing in a full bag thrown away, since a gym leader's TM needs a slot to go in.
fn make_room(cartridge: &mut Cartridge) {
    /// `MAX_ITEMS`.
    const BAG_SIZE: u8 = 20;
    let count = cartridge.read(sym::wNumBagItems.address);
    if count == BAG_SIZE {
        cartridge.write(sym::wNumBagItems.address, count - 1);
        cartridge.write(sym::wBagItems.address + 2 * (count as u16 - 1), 0xFF);
    }
}

fn clear_events(cartridge: &mut Cartridge, events: &[u16]) {
    for &event in events {
        let at = sym::wEventFlags.address + event / 8;
        let flags = cartridge.read(at);
        cartridge.write(at, flags & !(1 << (event % 8)));
    }
}

/// Clears `events` and the badge of `bit`, so a fixture standing in front of a beaten gym leader has
/// him to beat again, and heals the party it beat him with.
fn forget_badge(events: &'static [u16], bit: u8) -> impl FnOnce(&mut Cartridge) {
    move |cartridge| {
        heal(cartridge);
        make_room(cartridge);
        clear_events(cartridge, events);
        let badges = cartridge.read(sym::wObtainedBadges.address);
        cartridge.write(sym::wObtainedBadges.address, badges & !(1 << bit));
    }
}

/// Talks to whoever is in front until `got` is set, pressing A at every prompt, which in a battle
/// fights with the first move.
fn talk_until(got: u16, what: &'static str) -> impl FnMut(usize, &Seen) -> Option<(Action, &'static str)> {
    move |_, seen| match seen.kind {
        Kind::Prompt => Some((PROMPT, "a prompt")),
        Kind::MapChange | Kind::Bubble => Some((Action::Press(Joypad::empty(), 0), "on")),
        Kind::Overworld if event_set(seen, got) => None,
        Kind::Overworld => Some((Action::Talk, what)),
    }
}

/// Viridian Gym, the fixture standing in front of GIOVANNI with his badge already won, which the
/// cartridge is made to forget: the Earth Badge and TM27 handed over after the battle.
#[test]
fn giovanni_hands_over_the_earth_badge_and_tm27_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::{EVENT_BEAT_VIRIDIAN_GYM_GIOVANNI, EVENT_GOT_TM27};
    /// `BIT_EARTHBADGE`.
    const BIT: u8 = 7;
    lockstep(include_bytes!("../pokemon/data/post-earth-badge.bin"),
        forget_badge(&[EVENT_BEAT_VIRIDIAN_GYM_GIOVANNI, EVENT_GOT_TM27], BIT),
        talk_until(EVENT_GOT_TM27, "talk to GIOVANNI"));
}

/// Mr Fuji's house once the tower is cleared, the fixture standing in front of him with the Poké
/// Flute already his, which the cartridge is made to forget: he is talked to and hands it over.
#[test]
fn mr_fuji_hands_over_the_poke_flute_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::EVENT_GOT_POKE_FLUTE;
    lockstep(include_bytes!("../pokemon/data/post-poke-flute.bin"), |cartridge| {
        clear_events(cartridge, &[EVENT_GOT_POKE_FLUTE]);
        make_room(cartridge);
    }, talk_until(EVENT_GOT_POKE_FLUTE, "talk to MR.FUJI"));
}

/// Vermilion Gym once the Thunder Badge is won: LT.SURGE is talked to twice and only says his
/// after-battle line, which is the branch the gym's script table takes instead of his trainer header.
#[test]
fn lt_surge_only_talks_once_his_badge_is_won_as_the_cartridge_does() {
    let mut spoken = 0;
    lockstep(include_bytes!("../pokemon/data/post-thunder-badge.bin"), |_| {}, move |_, seen| match seen.kind {
        Kind::Prompt => Some((PROMPT, "a prompt")),
        Kind::MapChange | Kind::Bubble => None,
        Kind::Overworld => {
            spoken += 1;
            (spoken <= 2).then_some((Action::Talk, "talk to LT.SURGE"))
        }
    });
}

/// Cuts a fixture standing in front of a gym leader from one standing on a door in the same city:
/// the door's entry in `wWarpEntries` pointed at the gym's first warp, a step `through` it, then
/// `route` walked, and the state saved at the overworld poll it ends on. Only under
/// `GB_REGEN_FIXTURES=1`.
#[cfg(feature = "slow-tests")]
fn cut_gym_fixture(state: &[u8], door: (u8, u8), through: Joypad, gym: Map, route: &[Action], at: (u8, u8), path: &str) {
    const WARP_ENTRY: u16 = 4;
    let mut cartridge = Cartridge::from_state(state);
    while cartridge.to_poll().0 != Kind::Overworld {}
    let warps = cartridge.read(sym::wNumberOfWarps.address) as u16;
    let entry = (0..warps).map(|i| sym::wWarpEntries.address + i * WARP_ENTRY)
        .find(|&a| (cartridge.read(a), cartridge.read(a + 1)) == (door.1, door.0))
        .expect("the door");
    cartridge.write(entry + 2, 0);
    cartridge.write(entry + 3, gym as u8);
    for &action in std::iter::once(&Action::Walk(through)).chain(route) {
        let mut kind = cartridge.act(action).0;
        while kind != Kind::Overworld {
            kind = cartridge.to_poll().0;
        }
        println!("{action:?} -> {:?}", cartridge.location());
    }
    assert_eq!(cartridge.location(), (gym, at.0, at.1, SpriteFacing::Up));
    if crate::pokemon::integration_tests::fixture::regenerating_fixtures() {
        cartridge.gb.save_state_to_file(path).unwrap();
    } else {
        println!("skipping fixture write to {path} (set GB_REGEN_FIXTURES=1)");
    }
}

/// Recuts `pewter-gym.bin` from `completion-boulder.bin`, which stands on the gym's own door: in, and
/// eleven squares up the middle to the square below BROCK.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts pewter-gym.bin; needs GB_REGEN_FIXTURES=1"]
fn regen_pewter_gym_fixture() {
    const ROUTE: [Action; 11] = [Action::Walk(Joypad::UP); 11];
    cut_gym_fixture(include_bytes!("../pokemon/data/completion-boulder.bin"), (16, 17), Joypad::UP,
        Map::PewterGym, &ROUTE, (4, 2), concat!(env!("CARGO_MANIFEST_DIR"), "/src/pokemon/data/pewter-gym.bin"));
}

/// Recuts `cerulean-gym.bin` from `postgame-thunder-wave.bin`, which stands on the mart's door, that
/// door pointed at the gym: in, and round the pools to the square below MISTY, turned to her.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts cerulean-gym.bin; needs GB_REGEN_FIXTURES=1"]
fn regen_cerulean_gym_fixture() {
    const UP: Action = Action::Walk(Joypad::UP);
    const ROUTE: [Action; 17] = [UP, UP, UP, UP, UP, Action::Walk(Joypad::RIGHT), UP, UP, UP,
        Action::Walk(Joypad::RIGHT), Action::Walk(Joypad::RIGHT), UP, UP,
        Action::Walk(Joypad::LEFT), Action::Walk(Joypad::LEFT), Action::Walk(Joypad::LEFT),
        Action::Press(Joypad::UP, 2)];
    cut_gym_fixture(include_bytes!("../pokemon/data/postgame-thunder-wave.bin"), (25, 25), Joypad::UP,
        Map::CeruleanGym, &ROUTE, (4, 3), concat!(env!("CARGO_MANIFEST_DIR"), "/src/pokemon/data/cerulean-gym.bin"));
}

/// Pewter Gym, the fixture standing in front of BROCK with his badge already won, which the cartridge
/// is made to forget: the Boulder Badge and TM34 handed over after the battle.
#[test]
fn brock_hands_over_the_boulder_badge_and_tm34_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::{EVENT_BEAT_BROCK, EVENT_GOT_TM34};
    /// `BIT_BOULDERBADGE`.
    const BIT: u8 = 0;
    lockstep(include_bytes!("../pokemon/data/pewter-gym.bin"),
        forget_badge(&[EVENT_BEAT_BROCK, EVENT_GOT_TM34], BIT),
        talk_until(EVENT_GOT_TM34, "talk to BROCK"));
}

/// Cerulean Gym, the fixture standing in front of MISTY with her badge already won, which the
/// cartridge is made to forget: the Cascade Badge and TM11 handed over after the battle.
#[test]
fn misty_hands_over_the_cascade_badge_and_tm11_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::{EVENT_BEAT_MISTY, EVENT_GOT_TM11};
    /// `BIT_CASCADEBADGE`.
    const BIT: u8 = 1;
    lockstep(include_bytes!("../pokemon/data/cerulean-gym.bin"),
        forget_badge(&[EVENT_BEAT_MISTY, EVENT_GOT_TM11], BIT),
        talk_until(EVENT_GOT_TM11, "talk to MISTY"));
}


/// Celadon Gym, the fixture standing in front of ERIKA with her badge already won: she is made to
/// forget the badge, TM21 and her seven trainers but not the battle, so her text takes
/// `CeladonGymReceiveTM21` rather than fighting again, and `.gymVictory` marks every trainer beaten
/// so nobody stops the way out. A gym leader's battle is the battle lockstep's to compare.
#[test]
fn erika_hands_over_the_rainbow_badge_and_tm21_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::{EVENT_BEAT_CELADON_GYM_TRAINER_0, EVENT_BEAT_CELADON_GYM_TRAINER_1,
        EVENT_BEAT_CELADON_GYM_TRAINER_2, EVENT_BEAT_CELADON_GYM_TRAINER_3, EVENT_BEAT_CELADON_GYM_TRAINER_4,
        EVENT_BEAT_CELADON_GYM_TRAINER_5, EVENT_BEAT_CELADON_GYM_TRAINER_6, EVENT_GOT_TM21};
    /// `BIT_RAINBOWBADGE`.
    const BIT: u8 = 3;
    const FORGOTTEN: [u16; 8] = [EVENT_GOT_TM21, EVENT_BEAT_CELADON_GYM_TRAINER_0,
        EVENT_BEAT_CELADON_GYM_TRAINER_1, EVENT_BEAT_CELADON_GYM_TRAINER_2, EVENT_BEAT_CELADON_GYM_TRAINER_3,
        EVENT_BEAT_CELADON_GYM_TRAINER_4, EVENT_BEAT_CELADON_GYM_TRAINER_5, EVENT_BEAT_CELADON_GYM_TRAINER_6];
    lockstep(include_bytes!("../pokemon/data/post-rainbow-badge.bin"), forget_badge(&FORGOTTEN, BIT),
        talk_until(EVENT_GOT_TM21, "talk to ERIKA"));
}

/// Fuchsia Gym, the fixture standing in front of KOGA with his badge already won: the same as
/// Erika's, with the battle left won so his text takes `FuchsiaGymReceiveTM06`.
#[test]
fn koga_hands_over_the_soul_badge_and_tm06_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::{EVENT_BEAT_FUCHSIA_GYM_TRAINER_0, EVENT_BEAT_FUCHSIA_GYM_TRAINER_1,
        EVENT_BEAT_FUCHSIA_GYM_TRAINER_2, EVENT_BEAT_FUCHSIA_GYM_TRAINER_3, EVENT_BEAT_FUCHSIA_GYM_TRAINER_4,
        EVENT_BEAT_FUCHSIA_GYM_TRAINER_5, EVENT_GOT_TM06};
    /// `BIT_SOULBADGE`.
    const BIT: u8 = 4;
    const FORGOTTEN: [u16; 7] = [EVENT_GOT_TM06, EVENT_BEAT_FUCHSIA_GYM_TRAINER_0,
        EVENT_BEAT_FUCHSIA_GYM_TRAINER_1, EVENT_BEAT_FUCHSIA_GYM_TRAINER_2, EVENT_BEAT_FUCHSIA_GYM_TRAINER_3,
        EVENT_BEAT_FUCHSIA_GYM_TRAINER_4, EVENT_BEAT_FUCHSIA_GYM_TRAINER_5];
    lockstep(include_bytes!("../pokemon/data/post-soul-badge.bin"), forget_badge(&FORGOTTEN, BIT),
        talk_until(EVENT_GOT_TM06, "talk to KOGA"));
}

/// Saffron Gym, the fixture standing in front of SABRINA with her badge already won: the same as
/// Erika's, with the battle left won so her text takes `SaffronGymSabrinaReceiveTM46Script`.
#[test]
fn sabrina_hands_over_the_marsh_badge_and_tm46_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::{EVENT_BEAT_SAFFRON_GYM_TRAINER_0,
        EVENT_BEAT_SAFFRON_GYM_TRAINER_1, EVENT_BEAT_SAFFRON_GYM_TRAINER_2, EVENT_BEAT_SAFFRON_GYM_TRAINER_3,
        EVENT_BEAT_SAFFRON_GYM_TRAINER_4, EVENT_BEAT_SAFFRON_GYM_TRAINER_5, EVENT_BEAT_SAFFRON_GYM_TRAINER_6,
        EVENT_GOT_TM46};
    /// `BIT_MARSHBADGE`.
    const BIT: u8 = 5;
    const FORGOTTEN: [u16; 8] = [EVENT_GOT_TM46, EVENT_BEAT_SAFFRON_GYM_TRAINER_0,
        EVENT_BEAT_SAFFRON_GYM_TRAINER_1, EVENT_BEAT_SAFFRON_GYM_TRAINER_2, EVENT_BEAT_SAFFRON_GYM_TRAINER_3,
        EVENT_BEAT_SAFFRON_GYM_TRAINER_4, EVENT_BEAT_SAFFRON_GYM_TRAINER_5, EVENT_BEAT_SAFFRON_GYM_TRAINER_6];
    lockstep(include_bytes!("../pokemon/data/post-marsh-badge.bin"), forget_badge(&FORGOTTEN, BIT),
        talk_until(EVENT_GOT_TM46, "talk to SABRINA"));
}

//! Save slots: whole `Game::save`s the player keeps, held by the host rather than by `Game`, so no
//! save nests another.
//!
//! The game describes what it would put in a slot (`SlotSummary`) and asks through `Frame::slot`;
//! the host keeps the bytes in a `SlotStore`, stamps each with its own wall clock, and hands the
//! list back through `Game::set_slots`. The game reads no clock: it only shows the `SavedAt` it is
//! given, so the same save, seed and input still give the same run.

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use poke_core::map::Map;
use poke_core::rom_gfx::TILE_BYTES;
use serde::{Deserialize, Serialize};
use crate::gfx::compose::{Framebuffer, WIDTH};
use crate::modes::town_map::load_town_map_entry;
use crate::systems::play_time::PlayTime;
use crate::systems::pokedex::count_set_bits;
use crate::world::World;
use crate::Game;

/// The player's six, then the one the Hall of Fame writes.
pub const SLOTS: usize = 7;
/// The Hall of Fame's own slot, which the player may load and delete but not save to, so finishing
/// the game never writes over a slot the player chose.
pub const AUTOSAVE: u8 = SLOTS as u8 - 1;

/// The screen in grey, a third of its size, two bits a pixel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumbnail {
    /// Four pixels a byte, leftmost in the top bits, a row at a time.
    packed: Vec<u8>,
}

impl Thumbnail {
    pub const WIDTH: usize = 48;
    pub const HEIGHT: usize = 40;
    const SCALE: usize = 3;
    /// The 144×120 of the screen centred on the player, who stands at (64, 60) to (80, 76).
    const CROP: (usize, usize) = (0, 8);

    /// Each pixel is the rounded mean of the three-by-three block of shades it stands for, but never
    /// more than one shade lighter than the block's darkest, so a thin dark line is not averaged away.
    pub fn of(frame: &Framebuffer) -> Self {
        let mut packed = vec![0; Self::WIDTH * Self::HEIGHT / 4];
        for y in 0..Self::HEIGHT {
            for x in 0..Self::WIDTH {
                let (left, top) = (Self::CROP.0 + x * Self::SCALE, Self::CROP.1 + y * Self::SCALE);
                let block = || {
                    (top..top + Self::SCALE)
                        .flat_map(|row| &frame.shades[row * WIDTH + left..row * WIDTH + left + Self::SCALE])
                };
                let sum: usize = block().map(|&shade| shade as usize).sum();
                let darkest = block().copied().max().unwrap_or(0);
                let cells = Self::SCALE * Self::SCALE;
                let shade = (((sum + cells / 2) / cells) as u8).max(darkest.saturating_sub(1));
                let index = y * Self::WIDTH + x;
                packed[index / 4] |= shade << (6 - 2 * (index % 4));
            }
        }
        Self { packed }
    }

    /// 0 white to 3 black.
    pub fn shade(&self, x: usize, y: usize) -> u8 {
        let index = y * Self::WIDTH + x;
        (self.packed[index / 4] >> (6 - 2 * (index % 4))) & 3
    }

    /// Tiles across and down.
    pub const TILES: (usize, usize) = (Self::WIDTH / 8, Self::HEIGHT / 8);

    /// The picture as 2bpp tiles a row at a time, the shade standing as the colour index.
    pub fn tiles(&self) -> Vec<[u8; TILE_BYTES]> {
        let (across, down) = Self::TILES;
        let mut tiles = vec![[0u8; TILE_BYTES]; across * down];
        for (index, tile) in tiles.iter_mut().enumerate() {
            let (left, top) = (index % across * 8, index / across * 8);
            for y in 0..8 {
                for x in 0..8 {
                    let shade = self.shade(left + x, top + y);
                    tile[y * 2] |= (shade & 1) << (7 - x);
                    tile[y * 2 + 1] |= (shade >> 1) << (7 - x);
                }
            }
        }
        tiles
    }
}

/// What a slot shows of the game in it, all read from the game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotSummary {
    /// Charmap bytes.
    pub player_name: Vec<u8>,
    /// `wObtainedBadges`, a bit per badge.
    pub badges: u8,
    /// Species owned in the Pokédex.
    pub owned: u8,
    pub play_time: PlayTime,
    pub map: Map,
    pub thumbnail: Thumbnail,
}

impl SlotSummary {
    pub fn of(world: &World, thumbnail: Thumbnail) -> Self {
        Self {
            player_name: world.player_name.clone(),
            badges: world.badges,
            owned: count_set_bits(&world.pokedex.owned),
            play_time: world.play_time,
            map: world.location.map,
            thumbnail,
        }
    }

    /// The map's name as the town map gives it, in charmap bytes; a building takes its town's.
    pub fn location(&self) -> Option<Vec<u8>> {
        load_town_map_entry(self.map as u8).map(|(_, name)| name)
    }
}

/// When the host stored a slot, by its own clock: the instant, and the offset it was local to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SavedAt {
    pub unix_seconds: i64,
    pub utc_offset_minutes: i16,
}

/// A wall-clock date and time, for showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i64,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

impl SavedAt {
    /// A time before the epoch counts as the epoch.
    pub fn from_system_time(time: SystemTime, utc_offset_minutes: i16) -> Self {
        let unix_seconds = time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs() as i64);
        Self { unix_seconds, utc_offset_minutes }
    }

    pub fn local(&self) -> LocalTime {
        let seconds = self.unix_seconds + self.utc_offset_minutes as i64 * 60;
        let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
        let (year, month, day) = civil_from_days(days);
        LocalTime { year, month, day, hour: (of_day / 3600) as u8, minute: (of_day / 60 % 60) as u8 }
    }
}

/// Howard Hinnant's `civil_from_days`: the proleptic Gregorian date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u8;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 } as u8;
    (year_of_era + era * 400 + (month <= 2) as i64, month, day)
}

/// A slot as the game is shown it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub summary: SlotSummary,
    pub saved_at: SavedAt,
}

/// The most recently saved of `slots`, which CONTINUE puts the cursor on.
pub fn newest(slots: &[Option<Slot>]) -> Option<u8> {
    slots.iter().enumerate()
        .filter_map(|(index, slot)| slot.as_ref().map(|slot| (slot.saved_at, index as u8)))
        .max()
        .map(|(_, index)| index)
}

/// What a mode asks of the slots; `Game::slot_request` turns it into what the host is handed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotAction {
    /// The summary is the asking mode's, which knows what was on screen before any menu.
    Save(u8, SlotSummary),
    Load(u8),
    Delete(u8),
    /// The Hall of Fame's save into [`AUTOSAVE`], of the game as CONTINUE would have resumed it.
    Autosave,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotRequest {
    /// Keep `bytes`, `Game::save`'s, in `slot`, stamped with the host's time.
    Save { slot: u8, bytes: Vec<u8>, summary: SlotSummary },
    /// Replace the game with the one in `slot`.
    Load(u8),
    Delete(u8),
}

/// Where a host keeps the slots. `answer` is the whole of a host's part: it carries out a request
/// and hands the game the slots as they now are.
pub trait SlotStore {
    fn slots(&self) -> Vec<Option<Slot>>;
    fn read(&self, slot: u8) -> io::Result<Vec<u8>>;
    fn write(&mut self, slot: u8, bytes: &[u8], contents: Slot) -> io::Result<()>;
    fn delete(&mut self, slot: u8) -> io::Result<()>;

    /// `saved_at` is the host's clock now; only a save keeps it.
    fn answer(&mut self, game: &mut Game, request: SlotRequest, saved_at: SavedAt) -> io::Result<()> {
        match request {
            SlotRequest::Save { slot, bytes, summary } => self.write(slot, &bytes, Slot { summary, saved_at })?,
            SlotRequest::Load(slot) => {
                let bytes = self.read(slot)?;
                *game = Game::load(&bytes, game.pacing()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            }
            SlotRequest::Delete(slot) => self.delete(slot)?,
        }
        game.set_slots(self.slots());
        Ok(())
    }
}

fn check(slot: u8) -> io::Result<usize> {
    let index = slot as usize;
    if index < SLOTS { Ok(index) } else { Err(io::Error::new(io::ErrorKind::InvalidInput, format!("no slot {slot}"))) }
}

fn empty(slot: u8) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("slot {slot} is empty"))
}

/// The slots in memory, for tests and for a host with nowhere to write.
#[derive(Debug, Clone)]
pub struct MemoryStore {
    slots: Vec<Option<(Slot, Vec<u8>)>>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self { slots: vec![None; SLOTS] }
    }
}

impl SlotStore for MemoryStore {
    fn slots(&self) -> Vec<Option<Slot>> {
        self.slots.iter().map(|slot| slot.as_ref().map(|(contents, _)| contents.clone())).collect()
    }

    fn read(&self, slot: u8) -> io::Result<Vec<u8>> {
        self.slots[check(slot)?].as_ref().map(|(_, bytes)| bytes.clone()).ok_or_else(|| empty(slot))
    }

    fn write(&mut self, slot: u8, bytes: &[u8], contents: Slot) -> io::Result<()> {
        self.slots[check(slot)?] = Some((contents, bytes.to_vec()));
        Ok(())
    }

    fn delete(&mut self, slot: u8) -> io::Result<()> {
        self.slots[check(slot)?] = None;
        Ok(())
    }
}

/// A file a slot in a directory, `slot-<n>.pkslot`, written whole and renamed into place:
///
/// | bytes | |
/// |---|---|
/// | 4 | `PKSL` |
/// | 4 | the length of the `Slot`, little-endian |
/// | n | the `Slot`, MessagePack with field names |
/// | rest | `Game::save`'s bytes |
#[derive(Debug, Clone)]
pub struct DirectoryStore {
    dir: PathBuf,
}

const SLOT_MAGIC: &[u8; 4] = b"PKSL";

impl DirectoryStore {
    /// The directory is made on the first write.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, slot: u8) -> PathBuf {
        self.dir.join(format!("slot-{slot}.pkslot"))
    }

    /// The header and the `Slot`, leaving the reader at the game's bytes.
    fn open(&self, slot: u8) -> io::Result<(Slot, fs::File)> {
        let mut file = fs::File::open(self.path(slot))?;
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        if &header[..4] != SLOT_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a pokered slot"));
        }
        let mut contents = vec![0; u32::from_le_bytes(header[4..].try_into().unwrap()) as usize];
        file.read_exact(&mut contents)?;
        let contents = rmp_serde::from_slice(&contents).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok((contents, file))
    }
}

impl SlotStore for DirectoryStore {
    /// A slot whose file is missing or unreadable is shown empty, so the player can write over it.
    fn slots(&self) -> Vec<Option<Slot>> {
        (0..SLOTS as u8).map(|slot| self.open(slot).ok().map(|(contents, _)| contents)).collect()
    }

    fn read(&self, slot: u8) -> io::Result<Vec<u8>> {
        check(slot)?;
        let (_, mut file) = self.open(slot)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn write(&mut self, slot: u8, bytes: &[u8], contents: Slot) -> io::Result<()> {
        check(slot)?;
        let contents = rmp_serde::to_vec_named(&contents).expect("a slot always serialises");
        let mut file = SLOT_MAGIC.to_vec();
        file.extend((contents.len() as u32).to_le_bytes());
        file.extend(contents);
        file.extend(bytes);
        fs::create_dir_all(&self.dir)?;
        let path = self.path(slot);
        let partial = path.with_extension("pkslot.partial");
        fs::write(&partial, file)?;
        fs::rename(partial, path)
    }

    fn delete(&mut self, slot: u8) -> io::Result<()> {
        check(slot)?;
        match fs::remove_file(self.path(slot)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

#[cfg(test)]
mod tests;

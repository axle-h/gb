//! Sectioned save-state container.
//!
//! # Wire format
//!
//! ```text
//! "GBST" | u16 container_version (LE) | lz4_prepend_size { section* }
//!
//! section := label bytes | 0x00 | u32 payload_len (LE) | payload[payload_len]
//! payload := u16 section_version (LE) | bincode(value)*
//! ```
//!
//! The point of the labels is tolerance, in both directions:
//!
//! * an **unknown label** is skipped — a reader can load a state written by a newer build that
//!   declares sections it has never heard of;
//! * a **missing label** is not an error — the component keeps whatever value the machine being
//!   loaded into already had (in practice `Default`, since you load into a freshly constructed
//!   machine).
//!
//! ## The rules for changing serialised state
//!
//! A section's payload is a *sequence* of bincode values, not one blob. That is what makes
//! growth cheap — the reader stops when it runs out of values, and ignores any it does not
//! recognise.
//!
//! * **Adding a section** — just add it. Free, forever. Old readers skip the label; old files
//!   simply lack it and the component keeps its current value. This is always available and is
//!   the least intrusive option.
//!
//! * **Adding a field to an existing section** — do **not** add it to the section struct, which
//!   is positional and frozen once shipped. Append it as an extra *value* and bump that section's
//!   version constant:
//!
//!   ```ignore
//!   // writing
//!   writer.write_fields(labels::DMA, DMA_SECTION_VERSION /* now 2 */, |fields| {
//!       fields.field(&self.state)?;   // v1
//!       fields.field(&self.pos)        // appended in v2
//!   })?;
//!
//!   // reading
//!   let Some(mut fields) = reader.section(labels::DMA)? else { return Ok(()) };
//!   if let Some(state) = fields.field::<Option<LcdDmaState>>()? { self.state = state; }
//!   if let Some(pos)   = fields.field::<u8>()?                  { self.pos = pos; }
//!   ```
//!
//!   A v1 payload yields `None` for `pos`; a v1 *reader* handed a v2 payload stops after
//!   `state`. Both directions work, and **no fixture needs regenerating** either way.
//!
//! * **Changing a field's type or size** — bump the section version and branch on
//!   [`FieldReader::version`], or (usually simpler) retire the label and introduce a new one.
//!
//! What you must *not* do is change an already-shipped value's shape in place — reorder a section
//! struct's fields, or change one's type — without bumping the version. bincode is positional and
//! has no schema migration of any kind.
//!
//! Sections reserved for later phases (`cgb`, `sched`, `mbc`) are simply not written yet. An
//! absent section costs zero bytes, so declaring the taxonomy up front is free.

use std::collections::HashMap;

use bincode::config::Configuration;
use bincode::{Decode, Encode};

/// Container magic. Chosen so it cannot collide with a pre-container save state, whose first four
/// bytes were an lz4 uncompressed-length prefix (`"GBST"` would be a 1.4 GB payload).
const MAGIC: &[u8; 4] = b"GBST";

/// Bumped only if the *container framing* changes — not when a section changes.
pub const CONTAINER_VERSION: u16 = 1;

/// Section labels. Kept here so the taxonomy is visible in one place.
pub mod labels {
    /// CPU registers, run mode, IME and the EI latch.
    pub const CPU: &str = "cpu";
    /// Cartridge header, bank registers, RAM enable, SRAM banks.
    pub const CART: &str = "cart";
    /// Work RAM.
    pub const WRAM: &str = "wram";
    /// High RAM.
    pub const HRAM: &str = "hram";
    /// VRAM, OAM, LCDC/STAT/scroll/window/palettes, mode counters.
    pub const PPU: &str = "ppu";
    /// APU channels, frame sequencer, panning, master volume.
    pub const APU: &str = "apu";
    /// Divider, timer, serial.
    pub const TIMER: &str = "timer";
    /// OAM DMA state.
    pub const DMA: &str = "dma";
    /// IE / IF.
    pub const IRQ: &str = "irq";
    /// Joypad register.
    pub const JOYP: &str = "joyp";

    // --- Reserved. Nothing writes these yet; readers already tolerate them. ---
    /// Phase B: speed switch, VBK/SVBK, CGB palette RAM, HDMA.
    pub const CGB: &str = "cgb";
    /// Phase C: absolute clock and the event schedule.
    pub const SCHED: &str = "sched";
    /// Phase D: mapper-specific state and RTC.
    pub const MBC: &str = "mbc";
}

fn config() -> Configuration {
    bincode::config::standard()
}

/// Accumulates sections, then frames and compresses them.
pub struct SectionWriter {
    stream: Vec<u8>,
}

impl SectionWriter {
    pub fn new() -> Self {
        Self { stream: Vec::new() }
    }

    /// Append a single-value section. This is the common case; [`SectionWriter::write_fields`]
    /// is the general form.
    pub fn write<T: Encode>(&mut self, label: &str, version: u16, value: &T) -> Result<(), String> {
        self.write_fields(label, version, |fields| fields.field(value))
    }

    /// Append a section whose payload is a *sequence* of independently decoded values.
    ///
    /// This is how a field is added to an existing section without invalidating states that
    /// predate it: emit the original struct as the first value and each later addition as its
    /// own value, then bump the section version. A reader of an older payload simply runs out of
    /// values — see [`FieldReader::field`].
    pub fn write_fields<F>(&mut self, label: &str, version: u16, fill: F) -> Result<(), String>
    where
        F: FnOnce(&mut FieldWriter) -> Result<(), String>,
    {
        if label.is_empty() || label.as_bytes().contains(&0) {
            return Err(format!("invalid save state section label {label:?}"));
        }

        let mut fields = FieldWriter { label, body: Vec::new() };
        fill(&mut fields)?;
        let body = fields.body;

        self.stream.extend_from_slice(label.as_bytes());
        self.stream.push(0);
        let len = u32::try_from(body.len() + 2)
            .map_err(|_| format!("section {label:?} is too large to encode"))?;
        self.stream.extend_from_slice(&len.to_le_bytes());
        self.stream.extend_from_slice(&version.to_le_bytes());
        self.stream.extend_from_slice(&body);
        Ok(())
    }

    /// Frame and compress everything written so far.
    pub fn finish(self) -> Vec<u8> {
        let compressed = lz4_flex::compress_prepend_size(&self.stream);
        let mut out = Vec::with_capacity(compressed.len() + MAGIC.len() + 2);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
        out.extend_from_slice(&compressed);
        out
    }
}

impl Default for SectionWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Accumulates the values that make up one section's payload.
pub struct FieldWriter<'a> {
    label: &'a str,
    body: Vec<u8>,
}

impl FieldWriter<'_> {
    pub fn field<T: Encode>(&mut self, value: &T) -> Result<(), String> {
        let encoded = bincode::encode_to_vec(value, config())
            .map_err(|e| format!("encoding section {:?}: {e}", self.label))?;
        self.body.extend_from_slice(&encoded);
        Ok(())
    }
}

/// A parsed container. Sections are held as raw payloads and decoded on demand, so an unknown
/// label costs nothing more than the memory it occupies.
#[derive(Debug)]
pub struct SectionReader {
    sections: HashMap<String, Vec<u8>>,
}

impl SectionReader {
    /// Returns `Err` only if this is not a container at all, or if its framing is corrupt.
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < MAGIC.len() + 2 || &data[..MAGIC.len()] != MAGIC {
            return Err(
                "not a gb save state: missing \"GBST\" container header. Save states written \
                 before the sectioned format are not supported."
                    .to_string(),
            );
        }
        let container_version = u16::from_le_bytes([data[4], data[5]]);
        if container_version > CONTAINER_VERSION {
            return Err(format!(
                "save state container version {container_version} is newer than this build \
                 supports ({CONTAINER_VERSION})"
            ));
        }

        let stream = lz4_flex::decompress_size_prepended(&data[MAGIC.len() + 2..])
            .map_err(|e| format!("decompressing save state: {e}"))?;

        let mut sections = HashMap::new();
        let mut pos = 0usize;
        while pos < stream.len() {
            let label_end = stream[pos..]
                .iter()
                .position(|&b| b == 0)
                .map(|i| pos + i)
                .ok_or_else(|| "truncated save state: unterminated section label".to_string())?;
            let label = std::str::from_utf8(&stream[pos..label_end])
                .map_err(|e| format!("invalid section label: {e}"))?
                .to_string();
            pos = label_end + 1;

            if pos + 4 > stream.len() {
                return Err(format!("truncated save state: section {label:?} has no length"));
            }
            let len = u32::from_le_bytes(stream[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            if pos + len > stream.len() {
                return Err(format!(
                    "truncated save state: section {label:?} claims {len} bytes, {} remain",
                    stream.len() - pos
                ));
            }
            sections.insert(label, stream[pos..pos + len].to_vec());
            pos += len;
        }

        Ok(Self { sections })
    }

    pub fn contains(&self, label: &str) -> bool {
        self.sections.contains_key(label)
    }

    /// Labels present in the file, sorted. Diagnostics and tests only.
    pub fn labels(&self) -> Vec<&str> {
        let mut labels: Vec<&str> = self.sections.keys().map(String::as_str).collect();
        labels.sort_unstable();
        labels
    }

    /// Decode a single-value section. `Ok(None)` means the section is absent, which is not an
    /// error — the caller should leave its state untouched. Any values appended by a later
    /// version are ignored, which is what makes this reader forward compatible.
    pub fn read<T: Decode<()>>(&self, label: &str) -> Result<Option<(u16, T)>, String> {
        let Some(mut fields) = self.section(label)? else {
            return Ok(None);
        };
        let version = fields.version();
        let value = fields
            .field::<T>()?
            .ok_or_else(|| format!("section {label:?} (version {version}) is empty"))?;
        Ok(Some((version, value)))
    }

    /// Open a section for sequential reads. Use this rather than [`SectionReader::read`] when the
    /// section has grown fields across versions.
    pub fn section(&self, label: &str) -> Result<Option<FieldReader<'_>>, String> {
        let Some(payload) = self.sections.get(label) else {
            return Ok(None);
        };
        if payload.len() < 2 {
            return Err(format!("section {label:?} is too short to hold a version"));
        }
        Ok(Some(FieldReader {
            label: label.to_string(),
            version: u16::from_le_bytes([payload[0], payload[1]]),
            rest: &payload[2..],
        }))
    }
}

/// A cursor over one section's payload.
pub struct FieldReader<'a> {
    label: String,
    version: u16,
    rest: &'a [u8],
}

impl FieldReader<'_> {
    /// The version the writer stamped on this section.
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Decode the next value. `Ok(None)` means the payload is exhausted — i.e. this state was
    /// written before the field was appended, so the caller should keep its current value:
    ///
    /// ```ignore
    /// // `pos` was appended in dma section version 2.
    /// if let Some(pos) = fields.field::<u8>()? {
    ///     self.pos = pos;
    /// }
    /// ```
    pub fn field<T: Decode<()>>(&mut self) -> Result<Option<T>, String> {
        if self.rest.is_empty() {
            return Ok(None);
        }
        let (value, read): (T, usize) = bincode::decode_from_slice(self.rest, config())
            .map_err(|e| {
                format!("decoding section {:?} (version {}): {e}", self.label, self.version)
            })?;
        self.rest = &self.rest[read..];
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cycles::MachineCycles;
    use crate::game_boy::GameBoy;
    use crate::ram::{RAM, ROM};

    #[derive(Debug, PartialEq, Eq, Encode, Decode)]
    struct Probe {
        a: u32,
        b: String,
    }

    fn probe() -> Probe {
        Probe { a: 0xDEAD_BEEF, b: "hello".to_string() }
    }

    #[test]
    fn round_trips_a_section() {
        let mut writer = SectionWriter::new();
        writer.write("probe", 3, &probe()).unwrap();
        let reader = SectionReader::parse(&writer.finish()).unwrap();

        let (version, value): (u16, Probe) = reader.read("probe").unwrap().unwrap();
        assert_eq!(version, 3);
        assert_eq!(value, probe());
    }

    #[test]
    fn rejects_a_non_container() {
        let err = SectionReader::parse(&[0u8; 64]).unwrap_err();
        assert!(err.contains("GBST"), "{err}");
    }

    #[test]
    fn rejects_a_newer_container_version() {
        let mut bytes = SectionWriter::new().finish();
        bytes[4] = 0xFF;
        let err = SectionReader::parse(&bytes).unwrap_err();
        assert!(err.contains("newer than this build"), "{err}");
    }

    /// The append path, in both directions: a v2 reader must cope with a v1 payload, and a v1
    /// reader with a v2 payload. This is the mechanism A7 (and every later field addition)
    /// depends on, so it is tested rather than assumed.
    #[test]
    fn appended_fields_are_compatible_both_ways() {
        // v1: one value. v2: the same value plus an appended one.
        let v1 = {
            let mut w = SectionWriter::new();
            w.write("grow", 1, &probe()).unwrap();
            w.finish()
        };
        let v2 = {
            let mut w = SectionWriter::new();
            w.write_fields("grow", 2, |fields| {
                fields.field(&probe())?;
                fields.field(&0x1234u32)
            })
            .unwrap();
            w.finish()
        };

        // A v2 reader against a v1 payload: base value present, appended value absent.
        let v1_reader = SectionReader::parse(&v1).unwrap();
        let mut fields = v1_reader.section("grow").unwrap().unwrap();
        assert_eq!(fields.version(), 1);
        assert_eq!(fields.field::<Probe>().unwrap(), Some(probe()));
        assert_eq!(fields.field::<u32>().unwrap(), None);

        // A v2 reader against a v2 payload: both present.
        let v2_reader = SectionReader::parse(&v2).unwrap();
        let mut fields = v2_reader.section("grow").unwrap().unwrap();
        assert_eq!(fields.version(), 2);
        assert_eq!(fields.field::<Probe>().unwrap(), Some(probe()));
        assert_eq!(fields.field::<u32>().unwrap(), Some(0x1234));

        // A v1 reader against a v2 payload: reads what it knows, ignores the rest.
        let (version, value): (u16, Probe) =
            SectionReader::parse(&v2).unwrap().read("grow").unwrap().unwrap();
        assert_eq!(version, 2);
        assert_eq!(value, probe());
    }

    /// Forward tolerance: a reader that has never heard of a label must skip it and still find
    /// the sections it does know.
    #[test]
    fn unknown_sections_are_skipped() {
        let mut writer = SectionWriter::new();
        writer.write("probe", 1, &probe()).unwrap();
        writer.write("from_the_future", 99, &vec![7u8; 300]).unwrap();
        writer.write("probe2", 1, &probe()).unwrap();
        let reader = SectionReader::parse(&writer.finish()).unwrap();

        assert_eq!(reader.labels(), vec!["from_the_future", "probe", "probe2"]);
        assert_eq!(reader.read::<Probe>("probe").unwrap().unwrap().1, probe());
        assert_eq!(reader.read::<Probe>("probe2").unwrap().unwrap().1, probe());
    }

    /// Backward tolerance: a section the writer never emitted reads as absent, not as an error.
    #[test]
    fn missing_sections_are_absent_not_errors() {
        let reader = SectionReader::parse(&SectionWriter::new().finish()).unwrap();
        assert!(reader.labels().is_empty());
        assert!(reader.read::<Probe>("probe").unwrap().is_none());
        assert!(!reader.contains(labels::CGB));
    }

    /// The same two properties, exercised through a real machine rather than a synthetic payload.
    #[test]
    fn game_boy_tolerates_extra_and_missing_sections() {
        let mut gb = GameBoy::dmg_hello_world();
        gb.run(MachineCycles::from_m(200_000));
        // dmg-acid2 never touches high RAM, so give the `hram` section something distinctive to
        // carry — otherwise the "missing section" half of this test would be vacuous.
        for (i, address) in (0xFF80u16..=0xFFFE).enumerate() {
            gb.core_mut().mmu_mut().write(address, i as u8 ^ 0xA5);
        }

        // Forward: splice in a section this build does not know about.
        let mut writer = SectionWriter::new();
        gb.write_sections(&mut writer).unwrap();
        writer.write("not_a_real_section", 42, &vec![0xABu8; 128]).unwrap();
        let with_extra = writer.finish();

        let mut loaded = GameBoy::dmg_hello_world();
        loaded.load_state(&with_extra).expect("extra section must be skipped");
        assert_eq!(gb, loaded);

        // Backward: drop a section entirely. The component keeps the target machine's value.
        let mut writer = SectionWriter::new();
        gb.write_sections(&mut writer).unwrap();
        let without_hram = strip_section(writer.finish(), labels::HRAM);

        let mut fresh = GameBoy::dmg_hello_world();
        let fresh_hram = fresh.core().mmu().high_ram().to_vec();
        fresh.load_state(&without_hram).expect("missing section must not be an error");
        assert_eq!(fresh.core().mmu().high_ram(), fresh_hram.as_slice());
        assert_ne!(
            gb.core().mmu().high_ram(),
            fresh_hram.as_slice(),
            "test is vacuous unless the running machine's HRAM actually differs"
        );
        // Everything else still loaded.
        assert_eq!(fresh.core().mmu().ppu().vram(), gb.core().mmu().ppu().vram());
    }

    /// Rewrite a container with one section removed, so the "missing section" path can be tested
    /// without a second build.
    fn strip_section(container: Vec<u8>, drop: &str) -> Vec<u8> {
        let reader = SectionReader::parse(&container).unwrap();
        let mut stream = Vec::new();
        for label in reader.labels() {
            if label == drop {
                continue;
            }
            let payload = &reader.sections[label];
            stream.extend_from_slice(label.as_bytes());
            stream.push(0);
            stream.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            stream.extend_from_slice(payload);
        }
        let compressed = lz4_flex::compress_prepend_size(&stream);
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
        out.extend_from_slice(&compressed);
        out
    }

    /// Phase B added a `cgb` section and grew `wram` and `ppu` by appending a field to each. All
    /// three have to survive a round trip, or a CGB save state silently loses its banking.
    #[test]
    fn a_cgb_machine_round_trips_its_extra_state() {
        let mut gb = GameBoy::cgb(crate::roms::cgb_acid::ROM);
        gb.run(MachineCycles::from_m(400_000));

        // Touch every Phase B register so none of them can round-trip by being at its default.
        for (address, value) in [
            (0xFF70u16, 0x05), // SVBK
            (0xFF4F, 0x01),    // VBK
            (0xFF6C, 0x01),    // OPRI
            (0xFF4D, 0x01),    // KEY1, armed
            (0xFF72, 0xA5),
            (0xFF68, 0x80),    // BCPS, auto-increment
        ] {
            gb.core_mut().mmu_mut().write(address, value);
        }
        for i in 0..64u8 {
            gb.core_mut().mmu_mut().write(0xFF69, i ^ 0x3C); // fill BG palette RAM
        }
        gb.core_mut().mmu_mut().write(0xD800, 0x9E); // WRAM bank 5
        gb.core_mut().mmu_mut().write(0x8100, 0x7B); // VRAM bank 1
        gb.core_mut().mmu_mut().write(0xFEA0, 0x11); // the CGB-only unusable RAM

        let saved = gb.save_state().expect("save");
        let reader = SectionReader::parse(&saved).unwrap();
        assert!(reader.contains(labels::CGB), "a CGB state must carry a cgb section");

        let mut loaded = GameBoy::cgb(crate::roms::cgb_acid::ROM);
        loaded.load_state(&saved).expect("load");
        assert_eq!(gb, loaded);

        // ...and spot-check through the bus, so this cannot pass by both sides being wrong.
        assert_eq!(loaded.core().mmu().read(0xFF70), 0xF8 | 5);
        assert_eq!(loaded.core().mmu().read(0xD800), 0x9E);
        assert_eq!(loaded.core().mmu().read(0x8100), 0x7B);
        assert_eq!(loaded.core().mmu().read(0xFEA0), 0x11);
        assert_eq!(loaded.core().mmu().read(0xFF4D) & 0x01, 1, "the armed speed switch survives");
    }

    /// The `wram` and `ppu` sections grew by *appending*, so a state written before Phase B must
    /// still load — with the banks it never had left at zero. `every_committed_fixture_decodes`
    /// proves the 91 real fixtures do; this proves the mechanism, by synthesising a v1 payload.
    #[test]
    fn a_pre_cgb_state_loads_with_its_missing_banks_zeroed() {
        use crate::mmu::{WRAM_WINDOW, WRAM_SECTION_VERSION};

        let mut gb = GameBoy::dmg_hello_world();
        gb.run(MachineCycles::from_m(200_000));
        gb.core_mut().mmu_mut().write(0xD000, 0x5C);

        // Re-emit the container with `wram` written the way a v1 build would have: one value,
        // the 8 KB window, and nothing after it.
        let mut writer = SectionWriter::new();
        gb.write_sections(&mut writer).unwrap();
        let full = SectionReader::parse(&writer.finish()).unwrap();
        let mut fields = full.section(labels::WRAM).unwrap().unwrap();
        assert_eq!(fields.version(), WRAM_SECTION_VERSION, "v2 today");
        let window = fields.field::<[u8; WRAM_WINDOW]>().unwrap().unwrap();

        let mut spliced = SectionWriter::new();
        gb.write_sections(&mut spliced).unwrap();
        let rewritten = replace_section(spliced.finish(), labels::WRAM, 1, &window);

        let mut loaded = GameBoy::dmg_hello_world();
        loaded.load_state(&rewritten).expect("a v1 wram payload must still load");
        assert_eq!(loaded.core().mmu().read(0xD000), 0x5C, "the window came back");
        assert_eq!(gb, loaded, "and banks 2-7, which a v1 state never had, default to zero");
    }

    /// Rewrite a container with one section replaced by a differently-versioned payload.
    fn replace_section<T: Encode>(container: Vec<u8>, label: &str, version: u16, value: &T) -> Vec<u8> {
        let reader = SectionReader::parse(&container).unwrap();
        let mut replacement = SectionWriter::new();
        replacement.write(label, version, value).unwrap();
        let replacement_stream =
            lz4_flex::decompress_size_prepended(&replacement.finish()[MAGIC.len() + 2..]).unwrap();

        let mut stream = Vec::new();
        for existing in reader.labels() {
            if existing == label {
                stream.extend_from_slice(&replacement_stream);
                continue;
            }
            let payload = &reader.sections[existing];
            stream.extend_from_slice(existing.as_bytes());
            stream.push(0);
            stream.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            stream.extend_from_slice(payload);
        }
        let compressed = lz4_flex::compress_prepend_size(&stream);
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&CONTAINER_VERSION.to_le_bytes());
        out.extend_from_slice(&compressed);
        out
    }

    /// Every committed fixture must decode with the *current* section layout. This lives in the
    /// default tier deliberately: it turns a layout break into a two-second failure here rather
    /// than a baffling `slow-tests` failure an hour later.
    #[test]
    fn every_committed_fixture_decodes() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/pokemon/data");
        let mut checked = 0;
        let mut failures = Vec::new();

        for entry in std::fs::read_dir(dir).expect("fixture directory") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                continue;
            }
            let bytes = std::fs::read(&path).expect("read fixture");
            let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
            match gb.load_state(&bytes) {
                Ok(()) => checked += 1,
                Err(e) => failures.push(format!("{}: {e}", path.display())),
            }
        }

        assert!(
            failures.is_empty(),
            "{} fixture(s) failed to decode. If you changed a serialised struct, see the rules \
             at the top of src/savestate/mod.rs — appending within a section should never do \
             this.\n{}",
            failures.len(),
            failures.join("\n")
        );
        assert!(checked >= 91, "expected at least 91 fixtures, found {checked}");
    }
}

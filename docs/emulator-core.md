# Emulator core

Read before touching `src/{mmu,mbc,ppu,savestate,schedule,cycles,game_boy}.rs`, before adding or
reordering a serialised field, and before adding a file to `src/pokemon/data/`. The full arguments
are in the module docs named below; this is the list of what not to break.

## Mappers

- Every mapper resolves its bank register differently. MBC1 remaps a zero selection *then* wraps,
  so a wrap can reach bank 0; MBC3 wraps *then* remaps, so it never can; MBC2, MBC5 and HuC1 just
  mask. The table is in `src/mbc.rs`'s module docs, and it is what makes blargg's combined
  `dmg_sound.gb` terminate.
- `pokered.gbc` is cart type `0x13`: MBC3 with no RTC, so nothing committed has one. The RTC's
  time source is injectable (`MMU::set_rtc_time_source`) and defaults to the host clock; anything
  replayable must pin it.

## Save states

- `src/savestate/mod.rs`'s module docs are authoritative. Adding a section is free. Adding a field
  means appending it inside its section and bumping that section's version. Never reorder or
  retype a shipped value: bincode is positional. Before writing a legacy struct, check whether the
  boundary can be re-cut instead; that is how CGB support cost zero fixture regeneration.
- Every `.bin` in `src/pokemon/data/` is `include_bytes!`'d and `every_committed_fixture_decodes`
  loads all of them, so a `.bin` there must be a save state. Other binary fixtures go in a
  subdirectory (`data/gfx/`). `pokemon-red.sav` at the repo root is raw SRAM, not a state.
- A DMG state restored under `GB_HARDWARE=cgb` must not blank the screen. Every fixture and every
  deployed `state.gbst` is a DMG capture whose CGB palette section is all-white, and compatibility
  mode leaves the palette registers unmapped, so `MMU::read_sections` re-installs the boot palette
  last. `a_dmg_save_state_does_not_blank_a_compatibility_mode_screen` guards it.
- `Audio` and `PPU` exclude derived state from `PartialEq` (resampler output, the mix cache, the
  frame buffer, the per-scanline sprite list), because `game_boy::tests::save_and_load_state`
  compares restored state and none of that is serialised. `Schedule` is derived and not serialised
  at all; only `MMU::now` is. Output sample rate and emulation speed are applied, not stored (see
  `tune_audio` in [web-streams](web-streams.md)).

## Performance

- `PPU::draw_pixels_to`, the DMA transfer loops and `Serial::complete_transfer` are
  `#[inline(never)]`/`#[cold]` on purpose: `MMU::update` runs once per instruction, and letting
  them inline grew it 60% and cost several percent of throughput to instruction-cache pressure.
  After touching them, check with `nm -S --size-sort -C target/release/deps/gb-*` that
  `MMU::update` is still 3–4 KB.
- `MachineCycles::to_duration` and `from_duration` use `u128`. The `u64` version overflowed after
  ~73 minutes of emulated time, silently, and everything reporting emulated time went through it.
  `cycles::tests::to_duration_survives_a_long_run` pins 24 h.
- `AGENT_RESOLUTION` (20 ms, `agent.rs`) is empirical: longer and the player overshoots on the
  overworld, shorter and the game does not settle between agent steps.

# Emulator core

Read before touching `gb/src/{mmu,mbc,ppu,savestate,schedule,cycles,game_boy}.rs`, before adding or
reordering a serialised field, and before adding a file to `poke-agent/src/pokemon/data/`. The full
arguments are in the module docs named below; this is the list of what not to break.

## Not modelled

- M-cycle memory timing, and the DMG OAM corruption bug. Neither has a test; blargg's `mem_timing`,
  `halt_bug` and `oam_bug` ROMs were carried as permanently-ignored expected failures and are gone.
  `interrupt_time` stays, and passes.

## Mappers

- Every mapper resolves its bank register differently. MBC1 remaps a zero selection *then* wraps,
  so a wrap can reach bank 0; MBC3 wraps *then* remaps, so it never can; MBC2, MBC5 and HuC1 just
  mask. The table is in `gb/src/mbc.rs`'s module docs, and it is what makes blargg's combined
  `dmg_sound.gb` terminate.

## Save states

- `gb/src/savestate/mod.rs`'s module docs are authoritative. Adding a section is free. Adding a field
  means appending it inside its section and bumping that section's version. Never reorder or
  retype a shipped value: bincode is positional. Before writing a legacy struct, check whether the
  boundary can be re-cut instead; that is how CGB support cost zero fixture regeneration.
- `every_committed_fixture_decodes` loads every `.bin` in `poke-agent/src/pokemon/data/`, so a
  `.bin` there must be a save state. Other binary fixtures go in a subdirectory (`data/gfx/`), and
  `poke-agent-sdl/pokemon-red.sav` is raw SRAM rather than a state.
- A DMG state restored under `GB_HARDWARE=cgb` must not blank the screen. Every fixture and every
  deployed `state.gbst` is a DMG capture whose CGB palette section is all-white, and compatibility
  mode leaves the palette registers unmapped, so `MMU::read_sections` re-installs the boot palette
  last. `a_dmg_save_state_does_not_blank_a_compatibility_mode_screen` guards it.
- The `apu` section and `Audio`'s `PartialEq` both write the channels *settled*. The four channels
  run up to a deadline behind the CPU while the output side is gated, so a save state taken mid-batch
  would be of a machine that never existed; anything reading the whole APU through a `&self` has to
  go through `Audio::settled`. `Schedule` is derived and not serialised at all, only `MMU::now` is,
  and the output sample rate and emulation speed are applied rather than stored (see
  [web-streams](web-streams.md)).

## Performance

- `PPU::draw_pixels_to`, the DMA transfer loops and `Serial::complete_transfer` are
  `#[inline(never)]`/`#[cold]` on purpose: `MMU::update` runs once per instruction, and letting them
  inline grew it 60% and cost several percent of throughput to instruction-cache pressure. After
  touching them, check with `nm -S --size-sort -C` that `MMU::update` is still 3-4 KB.
- `MachineCycles::to_duration` and `from_duration` use `u128`. The `u64` version overflowed after
  ~73 minutes of emulated time, silently, and everything reporting emulated time went through it.
  `cycles::tests::to_duration_survives_a_long_run` pins 24 h.
- `AGENT_RESOLUTION` (20 ms, in `poke-agent`) is empirical: longer and the player overshoots on the
  overworld, shorter and the game does not settle between agent steps.

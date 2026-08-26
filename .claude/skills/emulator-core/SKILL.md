---
name: emulator-core
description: "Save-state format and the committed fixtures, mapper bank-register rules, DMG-state-in-a-CGB, the RTC time source, the `#[inline(never)]` hot path and the `MachineCycles` overflow. Load before touching src/{mmu,mbc,ppu,savestate,schedule,cycles,game_boy}.rs, before adding or reordering a field in anything serialised, and before adding a file to src/pokemon/data/."
---

# The emulator core

*Extracted from `CLAUDE.md`, which holds the rules of the road and the index of these skills. The
README is imported into `CLAUDE.md` and is not repeated there or here: this file has only the
invariants and the traps, nearly every one of which was learned by breaking something.*

## Emulator invariants

⚠️ **Every mapper resolves its bank register differently.** MBC1 remaps a zero selection *then*
wraps, so a wrap can reach bank 0; MBC3 wraps *then* remaps, so it never can; MBC2/MBC5/HuC1 have
their own rules again. Same two operations, opposite order, different answer — and it is what makes
blargg's combined `dmg_sound.gb` terminate. `src/mbc.rs`'s module docs have the table.

⚠️ **A save state is not tied to the machine that wrote it, and a DMG state in a CGB used to blank
the screen.** `GB_HARDWARE=cgb` means compatibility mode and the boot ROM's title-derived palette for
this DMG-only ROM. But every committed fixture and every deployed `state.gbst` is a **DMG** capture,
whose CGB palette RAM is `PaletteBank::default()` — all-ones, i.e. white — so restoring that section
painted the boot palette out and rendered every shade white while the game underneath played
perfectly. It cannot be repaired from outside: compatibility mode leaves `FF68`-`FF6B` unmapped, so
the palette is a **constant** rather than initial state. `MMU::read_sections` re-installs it last,
after the `cgb` section; `a_dmg_save_state_does_not_blank_a_compatibility_mode_screen` is the guard.
⚠️ Fixture vintage made this look intermittent — states captured before the `cgb` section existed
carry no palette to restore.

⚠️ **The RTC's time source is injectable and anything replayable must pin it**
(`MMU::set_rtc_time_source`) — the default is the host clock. Nothing committed has an RTC:
`pokered.gbc` is `0x13`, MBC3 with *no* timer.

**Save states.** `src/savestate/mod.rs`'s module docs are authoritative. **Adding a section is free;
adding a field means appending it as an extra value within its section and bumping that section's
version** — neither churns fixtures. ⚠️ **Never reorder or retype an already-shipped value without
bumping the section version**: bincode is positional with no schema migration. If you find yourself
about to write a legacy struct, check first whether you can re-cut the boundary instead — that is how
CGB support cost zero fixture regeneration.

The **102** committed fixtures in `src/pokemon/data/*.bin` are `include_bytes!`'d;
`every_committed_fixture_decodes` fails in seconds if a layout change breaks them. ⚠️ **That test
walks the directory and `load_state`s every `.bin` in it**, so `data/` is for save states and nothing
else — anything else goes in a subdirectory (`data/gfx/` is the one). `pokemon-red.sav` is raw SRAM,
not a save state.

⚠️ **`Audio` and `PPU` exclude derived state from `PartialEq`** — the resampler output and the cached
mix (`mixed`/`levels`/`mix_dirty`), and the frame buffer plus the per-scanline sprite list. None of it
is serialised, so none of it may take part in equality, or `game_boy::tests::save_and_load_state`
would compare restored state against state that was never saved. `Schedule` is derived the same way
and is not serialised at all — only the clock it is built from (`MMU::now`, the `sched` section).
Adding a field to `Audio` **is** safe now; the old "nothing may be added to `Audio`" rule died with
the sectioned format. (Output sample rate and emulation speed are applied rather than stored — see
`tune_audio` in the `web-streams` skill.)

⚠️ **`PPU::draw_pixels_to` and the three DMA transfer loops are `#[inline(never)]`/`#[cold]` on
purpose.** `MMU::update` runs once per CPU instruction, and letting those inline into it grew it 60%
(3052 → 4893 bytes) and cost several percent of core throughput to instruction-cache pressure alone.
If you touch them, check with `nm -S --size-sort -C target/release/deps/gb-*` that `MMU::update` is
still around 3–4 KB (Phase C left it at 3764). `Serial::complete_transfer` and the APU's `mix()` fell
to the same rule.

⚠️ **`MachineCycles::to_duration` multiplies by 4e9, and that overflowed `u64` after ~73 minutes of
emulated time** — silently, because release builds wrap. Everything reporting emulated time over a
long run went through it: `meta.json`'s `emulated_ms` and the status heartbeat both wrapped every 73
minutes on the deployed run. It surfaced as `soak`'s progress line simply stopping after 3600 s.
Both `to_duration` and `from_duration` use `u128` now; `cycles::tests::to_duration_survives_a_long_run`
pins it out to 24 h.

**Tuned constants**, both empirical: `AGENT_RESOLUTION` (20 ms) — longer and the player overshoots on
the overworld, shorter and the game state does not settle between frames; and `DelayContext`'s
2500 ms post-script delay, which covers the worst-case pre-battle animation gap observed in practice.


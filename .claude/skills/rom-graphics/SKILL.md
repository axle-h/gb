---
name: rom-graphics
description: "Reading graphics out of the cartridge (badges, front pics, tilesets, the font) and composing the map picture read_map answers with: bank windowing, tile order, sprite facing and OAM layout, palettes, labels and the reachability mask. Load before touching src/pokemon/{rom_gfx,badge_gfx,mon_gfx,map_gfx,font}.rs, src/web/sprites.rs or src/llm/map_image.rs."
---

# Graphics out of the cartridge, and the map the model is sent

*Extracted from `CLAUDE.md`, which holds the rules of the road and the index of these skills. The
README is imported into `CLAUDE.md` and is not repeated there or here: this file has only the
invariants and the traps, nearly every one of which was learned by breaking something.*

## Graphics out of the cartridge

No image is committed to this repo — `src/pokemon/rom_gfx.rs` has the primitives, `badge_gfx`/`mon_gfx`/
`map_gfx` the decoders, and `src/web/sprites.rs` and `src/llm/map_image.rs` the palettes.

⚠️ **Bank 0 is not windowed and every other bank is.** A ROM pointer's address is a raw file offset in bank 0
and a `0x4000`-based window everywhere else; `rom_gfx::rom_slice` is the one place that knows. (`badge_gfx`
used to have this wrong inline and got away with it because badges are bank 3.)

⚠️ **Tile order is row-major everywhere except a decompressed pic.** `rgbgfx` emits tiles left to right then
down unless pokered's Makefile passes `--columns`, and for these it does not — but `AlignSpriteDataCentered`
builds its buffer *column*-major. Comparing a decoded pic against `pokered/gfx/pokemon/front/*.2bpp` with the
wrong one differs on four fifths of the bytes with both sides looking like drawn sprites.

⚠️ **`mon_gfx`'s differential decode runs along rows and resets per row**, which is the opposite axis to the
one the bitstream was written along. Get it backwards and you get the right Pokémon with horizontal smears —
the kind of wrong that looks right in a thumbnail.

**How the decompressor is trusted.** `the_decompressor_matches_upstreams_own_2bpp` compares all 151 against
`make`'s own output, which is the only thing that can *prove* the port. ⚠️ Those files **cannot be
`include_bytes!`'d** — `.dockerignore` excludes `pokered/**/*.2bpp` and the Dockerfile's build stage copies
only `pokered.gbc` and `pokered.sym`, so a compile-time dependency would build here and fail in the container.
They are read from disk and the test skips loudly when absent; `every_front_pic_matches_its_committed_checksum`
covers that case. Regenerate it with `dump_front_pic_checksums` (`--features diagnostics --ignored`) **only**
when the 2bpp comparison is green, since that is the whole of what makes the fixture mean anything. ⚠️ **That
fixture lives in `src/pokemon/data/gfx/`, not `src/pokemon/data/`** — see the fixture-directory ⚠️ in the `emulator-core` skill.

**Palettes are the web layer's business, not the decoders'.** Both decoders return 2bpp shade indices and
nothing else. ⚠️ **`badges.rs` inverts its ramp and `sprites.rs` must not**, and the difference is not the page
— both land on the same dark panel. A badge is *line art*: there is no fill, so inverting it turns
black-on-white into white-on-dark and loses nothing. A Pokémon pic is *filled*, so inverting it is not a
palette choice but a different picture — it shipped that way once and Gengar came out white-bodied with a dark
grin. The argument that talked me into it ("a black outline is invisible on a near-black panel") is false: an
outline is bounded by the body's own bright fill and only the outermost contour meets the panel.
`the_ramp_is_not_inverted` pins the direction, because nothing else would notice it flipping.

⚠️ **The background is found by flood-filling shade 0 from the border**, four-way, and it is load-bearing:
shade 0 is a body's *white fill* as well as the surround, so calling it transparent outright renders the whole
Pokédex as wireframes (all 151 use it as fill), and not finding it at all renders them as solid white blocks.
A diagonal step leaks through any outline drawn on the diagonal, which is why the fill is four-way.

## The map the model is sent

`read_map` answers with a rendered picture of the whole current map. `src/pokemon/map_gfx.rs` reads the
graphics; `src/llm/map_image.rs` composes and colours them. Five things were paid for building it.

⚠️ **The picture is drawn on the *worker* thread, and `service_read` must keep handing over data rather than
pixels.** Celadon is 460k pixels and Route 17 is 737k — tens to hundreds of milliseconds of PNG encode against
an `AGENT_RESOLUTION` of 20 ms, so rendering inside `service_read` would spend ten agent ticks inside one of
them on nearly every overworld turn. What crosses the channel is a `MetaTileMap` (`ToolAnswer.map`), which the
policy already clones once per poll; `rom_gfx` reads a `&'static` ROM slice so the worker needs no emulator.
Same rule, same reason as `screenshot`.

⚠️ **`wSpriteStateData1 + 9` is `$0` down, `$4` up, `$8` left, `$C` right** (`ram/wram.asm:96`), and that is
**not** `PlayerFacingDirection`'s encoding (`Up = 8, Down = 4, Left = 2, Right = 1`, on `wPlayerDirection`).
The two collide on `4` and `8` meaning different things, so reading one with the other's table points half the
people on a map the wrong way and nothing fails.
`sprite_facing_is_the_sprite_bytes_encoding_and_not_the_players` is the guard.

⚠️ **Read the OAM layout out of `SpriteFacingAndAnimationTable`; do not mirror the sprite by hand.**
`.FlippedOAM` swaps the left and right *columns* as well as setting `OAM_XFLIP`, so assembling the 16×16 and
flipping it is right only by coincidence. ⚠️ **An immobile sprite falls back wholesale**: item balls and
boulders are 4-tile sheets, and pokered answers every facing from a second half of the table that is
`.StandingDown, .NormalOAM` — swapping only the tile ids and keeping the flipped layout draws a right-facing
Poké Ball as a *mirrored* one, which is a different picture.

⚠️ **`FontGraphics` is 1bpp**, 0x400 bytes, and `src/pokemon/font.rs`'s `FONT_BYTES` is the compile-time
doubling into 2bpp. Character code `C` → font tile `C - 0x80`, because `LoadFontTilePatterns` copies the sheet
to `vFont` (`$8800`) where the tile index and the character code are the same number. The reverse charmap
reuses `PokemonString::from_string` rather than transcribing `charmap.asm` a third time, and
`the_font_round_trips_through_the_decoder` pins it against `render_font_string` — which is how a **six-year-old
bug** surfaced: glyph 96 was decoded as `,` when it is `'`. Same mark, different half of the cell (`,` is 116),
so every contraction the game printed came back through the text reader as "Let,s go".

⚠️ **A tileset sheet can run off the end of its bank.** `LoadTilesetTilePatternData` copies a fixed `$60` tiles
whatever the tileset's real size, so several sheets overrun their own label into the blockset behind them, and
`Underground` (`1b:7d60`) overruns the bank itself by 864 bytes. On hardware nothing references a tile id that
high. `map_gfx` clamps the sheet to the bank and answers a blank tile for an id past the end.

⚠️ **A connection strip has its own tileset and it is often not the bordered map's** — Route 23 is `Plateau`
against Route 22's `Overworld`. `ConnectedMapStrip` carries `tileset` and `tileset_data` for exactly this;
drawing a strip against the map's own sheet produces plausible rubble rather than an error.
`MapMetadata::strip_cells` is shared between the classification and the drawing so the two cannot place a strip
differently — the arithmetic is four sign-sensitive cases and a border row one tile out of true looks perfectly
reasonable.

⚠️ **Labels are drawn last, so they can cover the player.** The red ring is where every coordinate the model
reads is measured from, so `layout_labels` reserves the player's cell before placing anything. Vermilion is the
map that found it. Relatedly, ⚠️ **a connection groups across the whole edge and a warp only with its
neighbours**: every cell of a map edge leads to the same place by definition, but the strip is broken up by
whatever is drawn along it, and Pallet Town's northern fence line had the picture saying "Route1" four times.
Two doors into the same building are *not* the same door (Mt Moon B1F), so warps may not be merged on
destination alone.

⚠️ **`MetaTileMap::reachable_tiles` is "routable to", not "standable on", and reading it the obvious way
produces a picture that still looks like a map.** It is the key set of `bfs_from_player`, which records *every*
neighbour of an open square — walls included — and only declines to expand them, because a route has to be
allowed to end at a door, a counter, a cut tree or a person. Dimming its complement therefore lit every wall
touching open floor and darkened only cells walled in on all four sides: 18% of Pallet Town, in a pattern with
no relation to anything, and it shipped in the first screenshots. The renderer subtracts obstacles and
un-surfable water itself; `a_wall_is_dimmed_even_though_the_agent_can_route_to_it` is the guard.

⚠️ **Nothing in the renderer may iterate a `HashSet`.** `reachable_tiles`, `warp_targets` and
`connection_targets` are all sets, and a picture whose content depends on hash iteration order reads to the
model as the world having moved, and makes any committed render checksum flake rather than fail. Every pass
walks `meta_tiles` in index order.


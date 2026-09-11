# Graphics out of the cartridge, and the map picture

Read before touching `poke-agent/src/pokemon/{rom_gfx,badge_gfx,mon_gfx,map_gfx,font}.rs`,
`poke-agent-web/src/web/sprites.rs` or `poke-agent/src/llm/map_image.rs`. Every rule here is also a
comment where it applies.

- Bank 0 is a raw file offset; every other bank is a `0x4000` window. `rom_slice` is the one place
  that knows, bar `poke-agent/src/pokemon/font.rs`, which computes its offset in a `const`.
- Tiles are row-major everywhere except a decompressed pic, which is built column-major. The wrong
  order differs on four fifths of the bytes and still looks like a sprite.
- The differential decode runs along rows and resets per row, the opposite axis to the bitstream.
  Backwards gives the right Pokémon with horizontal smears.
- The decompressor is compared against upstream's own `.2bpp` output read from disk, and skips
  loudly when absent — the container has no `.2bpp` files, so a committed checksum fixture covers it
  there. Regenerate that fixture only when the 2bpp comparison is green.
- Character code `C` is font tile `C - 0x80`, and the font is 1bpp doubled to 2bpp at compile time.
  Glyph 96 is `'` and 116 is `,`; the text reader printed "Let,s go" for six years.
- A tileset sheet can run off the end of its bank, because the cartridge copies a fixed tile count.
  `map_gfx` clamps to the bank and answers blank past the end.
- Both decoders return shade indices; palettes belong to the callers. The badge ramp is inverted
  because it is line art, and the pic ramp must not be: a filled pic inverted is a different picture.
- The pic background is a four-way flood fill of shade 0 from the border. Shade 0 is also every
  body's white fill, so treating it as transparent renders wireframes, and a diagonal fill leaks
  through outlines.
- The map picture is drawn on the worker thread and `service_read` hands over a `MetaTileMap`, never
  pixels: a large map is tens to hundreds of milliseconds of PNG encode against a 20 ms agent tick.
- Sprite facing is its own encoding and not `PlayerFacingDirection`'s; the two collide on 4 and 8.
- Read the OAM layout out of `SpriteFacingAndAnimationTable` rather than mirroring by hand: the
  flipped layout swaps tile columns as well as setting the flag, and an immobile sprite falls back
  wholesale.
- A connection strip has its own tileset, often not the bordered map's. One shared routine places
  strips for both classification and drawing, so the two cannot disagree.
- Labels are drawn last, after the unreachable pass, and a label with no route to it is greyed —
  judged per cell rather than per destination, because one building can have doors on two terraces.
- `reachable_tiles` means routable to, not standable on: the BFS records walls and counters as
  terminals so a route can end at them, and the renderer subtracts them itself.
- Nothing in the renderer iterates a `HashSet`. A picture whose content depends on hash order reads
  to the model as the world having moved.

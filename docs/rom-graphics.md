# Graphics out of the cartridge, and the map picture

Read before touching `poke-core/src/{gfx,rom_gfx,badge_gfx,mon_gfx,map_gfx,font}.rs`, `poke-core/build/gfx.rs`,
`poke-agent-web/src/web/sprites.rs` or `poke-agent/src/llm/map_image.rs`. Every rule here is also a
comment where it applies.

- A table moved off the ROM onto `vendor/pokered`'s source goes into `poke_core::tables` with a
  check in `poke-agent/src/rom_equality.rs` decoding it from the cartridge too; the checks stay.
- A table row the recreation's runtime holds (a trainer header, a text pointer table, a hidden
  event's routine) is a handle saved by its label, and `symbols::SavedLabel` still loads a save that
  held its address, through `saved_addresses`: committed, since nothing the recreation builds from
  has addresses.
- The sound data (`poke-core/build/audio.rs`) is each bank's in the linker's order without the
  engine's code, so the music is not at the cartridge's addresses: a channel is pointed at a label
  (`AudioBank::label`), and a save holding the cartridge's addresses restarts its song
  (`AudioEngine::restart_saved_music`).
- Bank 0 is a raw file offset; every other bank is a `0x4000` window. `rom_slice` is the one place
  that knows, bar `poke-agent/src/pokemon/font.rs`, which computes its offset in a `const`.
- `poke_core::gfx` is the tile data built from the committed `.png`s with the Makefile's own per-file
  options, and an option `poke-core/build/gfx.rs` does not implement fails the build. A `.pic` is
  there as the `.2bpp` it is compressed from.
- A pic is its generated `.2bpp`, row-major, placed bottom-centred in the 7x7 sprite buffer by
  `mon_gfx::pic_shades`; its side comes from the tile count, as `pkmncompress` takes it. The buffer
  is column-major (`pokedex::pic_tiles`), and the wrong order still looks like a sprite.
- `rom_equality`'s `pics` has the cartridge decompress and place every pic on the emulator, from its
  own pointers, and compares the buffers. A committed checksum of the front pics is the ROM-free
  stand-in; regenerate it only while `pics` is green.
- Character code `C` is font tile `C - 0x80`, and the font is 1bpp doubled to 2bpp at compile time.
  Glyph 96 is `'` and 116 is `,`; the text reader printed "Let,s go" for six years.
- A tileset sheet is as short as `--trim-whitespace` left it, and the cartridge copies a fixed count
  past it, into its blockset; a sprite sheet with no walking frames has the next sheet copied as
  them. Nothing draws either, and the recreation leaves both blank (`rom_equality.rs` pins what
  the cartridge reads). The same holds for the healing machine's third tile, the slot symbols'
  last four and the SGB border's last 32: the recreation copies only the picture.
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
- A map's `.blk` shorter than the map reads on into the next `INCBIN` in `maps.asm`
  (`BlockFiles::bytes` in `poke-core/build/tables.rs`), and so does a tile list with no terminator
  of its own into the next label (`stream`): both are the source's layout, not the linker's. So
  does a mon icon: the helix is the four tiles its row reads past `PokeBallSprite`, `FossilSprite`
  (`mon_icons` in `poke-core/build/gfx.rs`).
- A connection strip has its own tileset, often not the bordered map's. One shared routine places
  strips for both classification and drawing, so the two cannot disagree.
- Labels are drawn last, after the unreachable pass, and a label with no route to it is greyed —
  judged per cell rather than per destination, because one building can have doors on two terraces.
- `reachable_tiles` means routable to, not standable on: the BFS records walls and counters as
  terminals so a route can end at them, and the renderer subtracts them itself.
- Nothing in the renderer iterates a `HashSet`. A picture whose content depends on hash order reads
  to the model as the world having moved.

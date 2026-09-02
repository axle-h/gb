# Graphics out of the cartridge, and the map picture

Read before touching `src/pokemon/{rom_gfx,badge_gfx,mon_gfx,map_gfx,font}.rs`,
`src/web/sprites.rs` or `src/llm/map_image.rs`. Every rule here is also a comment at the point it
applies; this is the index.

## Reading the ROM

- Bank 0 is a raw file offset; every other bank is a `0x4000` window. `rom_gfx::rom_slice` is the
  place that knows, with one exception: `font.rs` computes the offset in a `const` for a
  bank-nonzero pointer and cannot call it.
- Tiles are row-major everywhere except a decompressed pic, which `AlignSpriteDataCentered` builds
  column-major. The wrong order differs on four fifths of the bytes and still looks like a sprite.
- `mon_gfx`'s differential decode runs along rows and resets per row, the opposite axis to the
  bitstream. Backwards gives the right Pokémon with horizontal smears.
- `the_decompressor_matches_upstreams_own_2bpp` compares all 151 pics against `make`'s output read
  from disk and skips loudly when absent: `.dockerignore` excludes `pokered/**/*.2bpp` and the
  Dockerfile copies only `pokered.gbc` and `pokered.sym`, so they can never be `include_bytes!`'d.
  `every_front_pic_matches_its_committed_checksum` covers the container. Regenerate that fixture
  (`dump_front_pic_checksums`, `--features diagnostics --ignored`) only when the 2bpp comparison is
  green. It lives in `src/pokemon/data/gfx/`, not `data/` (see [emulator-core](emulator-core.md)).
- `FontGraphics` is 1bpp and `font.rs`'s `FONT_BYTES` doubles it to 2bpp at compile time. Character
  code `C` is font tile `C - 0x80`. Glyph 96 is `'` and 116 is `,`; the text reader printed
  "Let,s go" for six years before `the_font_round_trips_through_the_decoder`.
- A tileset sheet can run off the end of its bank (`Underground` overruns by 864 bytes) because
  `LoadTilesetTilePatternData` copies a fixed `$60` tiles. `map_gfx` clamps to the bank and answers
  blank for an id past the end.

## Palettes

- Both decoders return shade indices; palettes belong to `sprites.rs`, `badges.rs` and
  `map_image.rs`. `badges.rs` inverts its ramp (line art, no fill) and `sprites.rs` must not: a
  filled pic inverted is a different picture, and Gengar shipped white-bodied once.
  `the_ramp_is_not_inverted` pins it.
- The pic background is a four-way flood fill of shade 0 from the border. Shade 0 is also every
  body's white fill, so treating it as transparent outright renders wireframes, and a diagonal
  fill leaks through outlines.

## The map picture

- It is drawn on the worker thread. `service_read` hands over a `MetaTileMap`, never pixels:
  Celadon is 460k pixels and Route 17 is 737k, tens to hundreds of milliseconds of PNG encode
  against a 20 ms agent tick. `rom_gfx` reads a `&'static` ROM slice so the worker needs no
  emulator. Same rule as `screenshot`.
- Sprite facing is `wSpriteStateData1 + 9`: `$0` down, `$4` up, `$8` left, `$C` right. That is not
  `PlayerFacingDirection`'s encoding (`Up = 8, Down = 4, Left = 2, Right = 1`), and the two collide
  on 4 and 8. `sprite_facing_is_the_sprite_bytes_encoding_and_not_the_players` guards it.
- Read the OAM layout out of `SpriteFacingAndAnimationTable`; do not mirror by hand. `.FlippedOAM`
  swaps tile columns as well as setting `OAM_XFLIP`, and an immobile sprite (item ball, boulder)
  falls back wholesale to `.StandingDown, .NormalOAM`.
- A connection strip has its own tileset, often not the bordered map's (Route 23 is `Plateau`
  against Route 22's `Overworld`). `ConnectedMapStrip` carries it, and `MapMetadata::strip_cells`
  is shared between classification and drawing so the two cannot place a strip differently.
- Labels are drawn last and `layout_labels` reserves the player's cell first. A connection is
  labelled once per edge; a warp merges only with its neighbours, never on destination alone (two
  doors into Mt Moon B1F are two doors).
- `MetaTileMap::reachable_tiles` means "routable to", not "standable on": the BFS records walls and
  counters as terminals so a route can end at them. The renderer subtracts obstacles and unsurfable
  water itself. `a_wall_is_dimmed_even_though_the_agent_can_route_to_it`.
- Nothing in the renderer iterates a `HashSet`. A picture whose content depends on hash order reads
  to the model as the world having moved and makes a render checksum flake. Every pass walks
  `meta_tiles` in index order.

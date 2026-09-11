use poke_core::map::Map;
use poke_core::map_header::{MapConnection, MapHeader};
use poke_core::rom_gfx::rom_slice;
use poke_core::symbols::{pokered_symbols, DmgBank, DmgPointer};

pub const MAP_BORDER: usize = 3;
const OVERWORLD_MAP_BYTES: usize = 1300;

fn rom(bank: u8, address: u16) -> &'static [u8] {
    rom_slice(DmgPointer { bank: DmgBank::ROM { bank }, address })
}

/// `LoadTileBlockMap`: `wOverworldMap`, its 1300 bytes all the border block, then the map inside a
/// `MAP_BORDER`-block frame, then `MAP_BORDER` rows or columns of each neighbour.
pub fn tile_block_map(map: Map) -> Result<Vec<u8>, String> {
    let header = MapHeader::read(map)?;
    let border_block = rom(header.header_bank, header.objects_address)[0];
    let mut buffer = vec![border_block; OVERWORLD_MAP_BYTES];
    let (width, height) = (header.width as usize, header.height as usize);
    let stride = width + 2 * MAP_BORDER;
    let blocks = rom(header.header_bank, header.blocks_address);
    for row in 0..height {
        let at = (row + MAP_BORDER) * stride + MAP_BORDER;
        buffer[at..at + width].copy_from_slice(&blocks[row * width..(row + 1) * width]);
    }
    let dest = |connection: &MapConnection| (connection.strip_dest - pokered_symbols::wOverworldMap.address) as usize;
    let source = |connection: &MapConnection| {
        let bank = connection.map.header_pointer().expect("a connected map has a header").bank.id();
        rom(bank, connection.strip_src)
    };
    for connection in [&header.north_connection, &header.south_connection].into_iter().flatten() {
        let (from, to, strip) = (source(connection), dest(connection), connection.strip_length as usize);
        for row in 0..MAP_BORDER {
            let (src, dst) = (row * connection.connected_map_width as usize, to + row * stride);
            buffer[dst..dst + strip].copy_from_slice(&from[src..src + strip]);
        }
    }
    for connection in [&header.west_connection, &header.east_connection].into_iter().flatten() {
        let (from, to) = (source(connection), dest(connection));
        for row in 0..connection.strip_length as usize {
            let (src, dst) = (row * connection.connected_map_width as usize, to + row * stride);
            buffer[dst..dst + MAP_BORDER].copy_from_slice(&from[src..src + MAP_BORDER]);
        }
    }
    Ok(buffer)
}

/// The camera for a player standing at `(x, y)`: the player's square is the screen's ninth
/// column and row of tiles, two tiles to a square.
pub fn camera(x: u8, y: u8) -> (i32, i32) {
    let origin = |coordinate: u8| (MAP_BORDER as i32 * 32) + coordinate as i32 * 16 - 64;
    (origin(x), origin(y))
}

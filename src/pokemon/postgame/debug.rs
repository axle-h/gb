//! **Phase 0, task 0.7** — the debug tier: the one place RAM writes are allowed.
//!
//! `docs/postgame-coverage-plan.md` §3 keeps the repo's standing claim that *no RAM-write shortcuts
//! remain in the play path*, while admitting that fixture construction and test seeding need them.
//! The line it draws:
//!
//! - **Play path** — anything reachable from `Policy::pick_*` during a legitimate run: **button input
//!   only**, no exceptions.
//! - **Debug tier** — everything in this file: free to write RAM, used *only* for building fixtures,
//!   seeding tests, and diagnostics.
//!
//! A naming convention alone would rot, so [`play_path_contains_no_debug_ram_writes`] enforces it by
//! reading the play-path sources and failing if `debug_` appears in any of them. It scans the
//! `postgame/` directory from disk rather than a hard-coded list, so a workstream added later is
//! covered without anyone remembering to opt in.
//!
//! `PolicyStep::MovePokemonToFront` is a pre-existing exception (it writes party order directly).
//! It stays; nothing new joins it.

use crate::mmu::MMU;
use crate::pokemon::item::ItemId;
use crate::pokemon::party::PokemonParty;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::encoding::PokemonEncoding;
use crate::pokemon::PokemonApi;
use crate::ram::{RAM, ROM};

/// Encode `value` as the game's binary-coded decimal — the inverse of
/// [`crate::pokemon::encoding::reverse_bcd`]. Money and coins are stored this way.
fn to_bcd(mut value: u32, bytes: usize) -> Vec<u8> {
    let mut digits = Vec::with_capacity(bytes * 2);
    for _ in 0..bytes * 2 {
        digits.push((value % 10) as u8);
        value /= 10;
    }
    // Big-endian: most significant byte first, two digits per byte.
    (0..bytes)
        .rev()
        .map(|i| (digits[i * 2 + 1] << 4) | digits[i * 2])
        .collect()
}

impl<'a> PokemonApi<'a> {
    /// Overwrite the player's money (capped at the game's ¥999,999).
    pub fn debug_set_money(&mut self, amount: u32) {
        let bytes = to_bcd(amount.min(999_999), 3);
        let base = pokered_symbols::wPlayerMoney.address;
        for (i, b) in bytes.iter().enumerate() {
            self.mmu_mut().write(base + i as u16, *b);
        }
    }

    /// Overwrite the Game Corner coin count (capped at 9,999).
    pub fn debug_set_coins(&mut self, coins: u16) {
        let bytes = to_bcd(coins.min(9_999) as u32, 2);
        let base = pokered_symbols::wPlayerCoins.address;
        for (i, b) in bytes.iter().enumerate() {
            self.mmu_mut().write(base + i as u16, *b);
        }
    }

    /// Put `qty` of `item` in the bag, or top up the stack if it is already there.
    ///
    /// Writes raw `(id, quantity)` pairs so it can place ids [`ItemId`] cannot name (most TMs).
    /// Returns `Err` if the bag is full and the item is not already in it.
    pub fn debug_give_item(&mut self, item: ItemId, qty: u8) -> Result<(), String> {
        self.debug_give_item_id(item as u8, qty)
    }

    /// [`Self::debug_give_item`] by raw id, for the TMs and HMs `ItemId` does not name.
    pub fn debug_give_item_id(&mut self, id: u8, qty: u8) -> Result<(), String> {
        let count = self.mmu().read_pointer(&pokered_symbols::wNumBagItems) as usize;
        let base = pokered_symbols::wBagItems.address;
        if let Some(i) = (0..count).find(|&i| self.mmu().read(base + i as u16 * 2) == id) {
            let have = self.mmu().read(base + i as u16 * 2 + 1);
            self.mmu_mut().write(base + i as u16 * 2 + 1, have.saturating_add(qty).min(99));
            return Ok(());
        }
        if count >= crate::pokemon::bag::Bag::MAX_ITEMS {
            return Err(format!("bag is full ({count} items); cannot add {id:#04x}"));
        }
        self.mmu_mut().write(base + count as u16 * 2, id);
        self.mmu_mut().write(base + count as u16 * 2 + 1, qty);
        // The list is 0xFF-terminated after the last pair.
        self.mmu_mut().write(base + (count as u16 + 1) * 2, 0xFF);
        self.mmu_mut().write(pokered_symbols::wNumBagItems.address, count as u8 + 1);
        Ok(())
    }

    /// Mark `species` as owned **and** seen in the Pokédex. Used to seed the 10/30/50 gates the
    /// Oak's-aide items sit behind (workstream H) without playing through the catching first.
    pub fn debug_set_dex_owned(&mut self, species: PokemonSpecies) {
        for ptr in [&pokered_symbols::wPokedexOwned, &pokered_symbols::wPokedexSeen] {
            let bit = species.metadata().pokedex_number - 1;
            let addr = ptr.address + (bit / 8) as u16;
            let byte = self.mmu().read(addr);
            self.mmu_mut().write(addr, byte | (1 << (bit % 8)));
        }
    }

    /// Replace the whole party. Build members with `Pokemon::maxed` or `Pokemon::new`.
    pub fn debug_set_party(&mut self, party: &PokemonParty) -> Result<(), String> {
        self.mmu_mut().write_player_pokemon_party(party)
    }
}

/// **The boundary guard for task 0.7.** Fails if `debug_` appears anywhere in the play path.
///
/// The play path is `policy.rs`, `agent.rs` and every workstream file under `postgame/` — i.e.
/// everything reachable from `Policy::pick_*` during a legitimate run. Only this file is exempt.
///
/// Comments are stripped before matching so that prose about the debug tier (including this module's
/// own doc comment, were it ever moved) does not trip the guard.
#[cfg(test)]
#[test]
fn play_path_contains_no_debug_ram_writes() {
    let mut sources: Vec<std::path::PathBuf> = vec![
        "src/pokemon/policy.rs".into(),
        "src/pokemon/agent.rs".into(),
    ];
    // Scanned from disk, not a hard-coded list, so a workstream added later cannot quietly opt out.
    let postgame = std::path::Path::new("src/pokemon/postgame");
    for entry in std::fs::read_dir(postgame).expect("postgame module directory should exist") {
        let path = entry.expect("readable dir entry").path();
        if path.extension().map_or(false, |e| e == "rs") && path.file_name().unwrap() != "debug.rs" {
            sources.push(path);
        }
    }
    assert!(sources.len() > 3, "guard scanned almost nothing — is the working directory wrong?");

    let mut offenders = Vec::new();
    for path in &sources {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for (i, line) in src.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if code.contains("debug_") {
                offenders.push(format!("{}:{} — {}", path.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "RAM-write debug helpers must not be reachable from the play path \
         (docs/postgame-coverage-plan.md §3). Found:\n{}",
        offenders.join("\n")
    );
}

#[cfg(test)]
mod tests {
    use super::to_bcd;

    #[test]
    fn bcd_round_trips() {
        use crate::pokemon::encoding::reverse_bcd;
        for value in [0u32, 1, 12, 100, 3000, 37_774, 999_999] {
            let bytes = to_bcd(value, 3);
            let packed = (bytes[0] as u32) << 16 | (bytes[1] as u32) << 8 | bytes[2] as u32;
            assert_eq!(reverse_bcd(packed), value, "round trip failed for {value}");
        }
        // Two-byte form, as used for coins.
        let bytes = to_bcd(9_999, 2);
        assert_eq!(reverse_bcd((bytes[0] as u32) << 8 | bytes[1] as u32), 9_999);
    }
}

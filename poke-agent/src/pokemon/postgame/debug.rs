//! The debug tier: the one place RAM writes are allowed, and no play-path source may call it.

use crate::pokemon::item::ItemId;
use crate::pokemon::party::PokemonParty;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::encoding::PokemonEncoding;
use crate::pokemon::PokemonApi;
use gb::ram::{RAM, ROM};

/// Encode `value` as the game's binary-coded decimal — the inverse of
/// [`crate::pokemon::encoding::reverse_bcd`].
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

    /// Put `qty` of `item` in the bag, or top up the stack if it is already there.
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

    /// Remove `item` from the bag entirely, closing the gap behind it.
    pub fn debug_take_item(&mut self, item: ItemId) -> Result<(), String> {
        let count = self.mmu().read_pointer(&pokered_symbols::wNumBagItems) as usize;
        let base = pokered_symbols::wBagItems.address;
        let Some(i) = (0..count).find(|&i| self.mmu().read(base + i as u16 * 2) == item as u8) else {
            return Err(format!("{item:?} is not in the bag"));
        };
        for j in i..count - 1 {
            let (id, qty) = (self.mmu().read(base + (j as u16 + 1) * 2),
                             self.mmu().read(base + (j as u16 + 1) * 2 + 1));
            self.mmu_mut().write(base + j as u16 * 2, id);
            self.mmu_mut().write(base + j as u16 * 2 + 1, qty);
        }
        self.mmu_mut().write(base + (count as u16 - 1) * 2, 0xFF);
        self.mmu_mut().write(pokered_symbols::wNumBagItems.address, count as u8 - 1);
        Ok(())
    }

    /// Rewrite the bag so that it holds only the kinds in `keep`, and answer with the raw ids
    /// that were dropped.
    pub fn debug_keep_only_items(&mut self, keep: &[ItemId]) -> Vec<u8> {
        let count = self.mmu().read_pointer(&pokered_symbols::wNumBagItems) as usize;
        let base = pokered_symbols::wBagItems.address;
        let held: Vec<(u8, u8)> = (0..count)
            .map(|i| (self.mmu().read(base + i as u16 * 2), self.mmu().read(base + i as u16 * 2 + 1)))
            .collect();
        let (kept, dropped): (Vec<(u8, u8)>, Vec<(u8, u8)>) = held
            .into_iter()
            .partition(|(id, _)| keep.iter().any(|wanted| *wanted as u8 == *id));
        for (i, (id, qty)) in kept.iter().enumerate() {
            self.mmu_mut().write(base + i as u16 * 2, *id);
            self.mmu_mut().write(base + i as u16 * 2 + 1, *qty);
        }
        // The list is 0xFF-terminated after the last pair.
        self.mmu_mut().write(base + kept.len() as u16 * 2, 0xFF);
        self.mmu_mut().write(pokered_symbols::wNumBagItems.address, kept.len() as u8);
        dropped.into_iter().map(|(id, _)| id).collect()
    }

    /// Replace the whole party. Build members with `Pokemon::maxed` or `Pokemon::new`.
    pub fn debug_set_party(&mut self, party: &PokemonParty) -> Result<(), String> {
        self.mmu_mut().write_player_pokemon_party(party)
    }

    /// Knock the whole party out, active battle Pokémon included — i.e. make the next thing the
    /// cartridge checks a black-out.
    pub fn debug_faint_party(&mut self) {
        let base = pokered_symbols::wPartyMons.address;
        for index in 0..crate::pokemon::encoding::PokemonBlockAddresses::PARTY_MAX {
            // Offset 1 of the party struct is the big-endian current HP.
            let hp = base + index * crate::pokemon::encoding::PokemonBlockAddresses::POKEMON_BLOCK_SIZE + 1;
            self.mmu_mut().write(hp, 0);
            self.mmu_mut().write(hp + 1, 0);
        }
        let battle_hp = pokered_symbols::wBattleMonHP.address;
        self.mmu_mut().write(battle_hp, 0);
        self.mmu_mut().write(battle_hp + 1, 0);
    }

    /// Set party member `member`'s HP, and the battle copy's too when it is the one out.
    pub fn debug_set_hp(&mut self, member: usize, hp: u16) {
        use crate::pokemon::encoding::PokemonBlockAddresses;
        let at = pokered_symbols::wPartyMons.address + member as u16 * PokemonBlockAddresses::POKEMON_BLOCK_SIZE + 1;
        let out = self.mmu().read_pointer(&pokered_symbols::wPlayerMonNumber) as usize == member;
        let mut targets = vec![at];
        if out && self.mmu().read_pointer(&pokered_symbols::wIsInBattle) != 0 {
            targets.push(pokered_symbols::wBattleMonHP.address);
        }
        for address in targets {
            self.mmu_mut().write(address, (hp >> 8) as u8);
            self.mmu_mut().write(address + 1, (hp & 0xff) as u8);
        }
    }

    pub fn debug_set_catch_rate(&mut self, rate: u8) {
        self.mmu_mut().write(pokered_symbols::wEnemyMonActualCatchRate.address, rate);
    }

    pub fn debug_set_repel_steps(&mut self, steps: u8) {
        self.mmu_mut().write(pokered_symbols::wRepelRemainingSteps.address, steps);
    }

    pub fn debug_set_battle_speeds(&mut self, player: u16, enemy: u16) {
        for (ptr, value) in [
            (&pokered_symbols::wBattleMonSpeed, player),
            (&pokered_symbols::wEnemyMonSpeed, enemy),
        ] {
            // Big-endian, like every other 16-bit battle stat — see `read_pointer_u16_be`.
            self.mmu_mut().write(ptr.address, (value >> 8) as u8);
            self.mmu_mut().write(ptr.address + 1, (value & 0xff) as u8);
        }
    }

    pub fn debug_set_safari_balls(&mut self, count: u8) {
        self.mmu_mut().write(pokered_symbols::wNumSafariBalls.address, count);
    }

    pub fn debug_set_badges(&mut self, badges: crate::pokemon::badge::Badge) {
        self.mmu_mut().write(pokered_symbols::wObtainedBadges.address, badges.bits());
    }

    pub fn debug_heal_party(&mut self) -> Result<(), String> {
        let mut party = self.mmu().read_player_pokemon_party()?;
        for index in 0..party.len() {
            let member = &mut party[index];
            member.current_hp = member.stats.hp;
            member.status = crate::pokemon::status::PokemonStatus::None;
        }
        self.mmu_mut().write_player_pokemon_party(&party)
    }

    pub fn debug_restore_pp(&mut self) -> Result<(), String> {
        let mut party = self.mmu().read_player_pokemon_party()?;
        for index in 0..party.len() {
            for slot in party[index].moves.iter_mut().flatten() {
                slot.pp = slot.name.metadata().pp;
            }
        }
        self.mmu_mut().write_player_pokemon_party(&party)
    }

    pub fn debug_teach_move(
        &mut self,
        member: usize,
        slot: usize,
        battle_move: crate::pokemon::move_name::PokemonMoveName,
    ) -> Result<(), String> {
        let mut party = self.mmu().read_player_pokemon_party()?;
        if member >= party.len() {
            return Err(format!("no party member {member}; the party holds {}", party.len()));
        }
        let moves = &mut party[member].moves;
        if slot >= moves.len() {
            return Err(format!("no move slot {slot}; a Pokémon has {}", moves.len()));
        }
        moves[slot] = Some(crate::pokemon::move_name::PokemonMove::with_max_pp(battle_move));
        self.mmu_mut().write_player_pokemon_party(&party)
    }

    /// Force the OPTION menu's settings by writing `wOptions`, answering whether they had drifted.
    pub fn debug_set_options(&mut self, options: &crate::pokemon::options::GameOptions) -> bool {
        crate::pokemon::options::keep_game_options(self.mmu_mut(), options)
    }
}

/// No play-path source names a `debug_` helper outside a comment.
#[cfg(test)]
#[test]
fn play_path_contains_no_debug_ram_writes() {
    let mut sources: Vec<std::path::PathBuf> = vec![
        "src/pokemon/policy.rs".into(),
        "src/pokemon/agent.rs".into(),
    ];
    // Scanned from disk so a module added later cannot opt out.
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
        "RAM-write debug helpers must not be reachable from the play path: it plays on button \
         input only, and a RAM write desynchronises the game's scripts from its map objects. \
         Found:\n{}",
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

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

use crate::pokemon::item::ItemId;
use crate::pokemon::party::PokemonParty;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::encoding::PokemonEncoding;
use crate::pokemon::PokemonApi;
use gb::ram::{RAM, ROM};

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

/// **Workstream J** — the options every test fixture is loaded with.
///
/// Three bits, and only one of them is new:
///
/// - **Text speed FAST** (`wOptions & TEXT_DELAY_MASK == 1`) — already what the harness wrote.
/// - **Battle animations OFF** (bit 7 set) — J's actual change. Every battle in the suite pays for
///   the attack animations otherwise, and nothing in the agent watches them: the battle driver reads
///   `wIsInBattle` and the menu geometry, never the screen.
/// - **Battle style SET** (bit 6 set) — ⚠️ *kept*, not chosen. §8-J proposes `0b1000_0001`, which is
///   SHIFT, and §8-J's own last bullet says battle style must be left alone. The harness has written
///   SET since long before this workstream (`GameOptions::default`), so SET **is** what every driver
///   was tuned against; `0b1000_0001` would have silently reintroduced the "will you switch?" prompt
///   the whole suite has never seen. The value is therefore `0b1100_0001`.
pub const FAST_FIXTURE_OPTIONS: crate::pokemon::options::GameOptions = crate::pokemon::options::GameOptions {
    battle_animations_on: false,
    battle_style: crate::pokemon::options::BattleStyle::Set,
    text_speed: crate::pokemon::options::TextSpeed::Fast,
};

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

    /// Remove `item` from the bag entirely, closing the gap behind it.
    ///
    /// The counterpart to [`Self::debug_give_item`], and it exists for the same reason: a fixture cut
    /// before a route step was written has the bag that route no longer produces. ⚠️ **Gen 1's bag is
    /// twenty *entries*, so seeding one in usually means taking one out** — a `debug_give_item` onto a
    /// full bag is an `Err`, and a seed that quietly displaces a purchase is worse than one that fails.
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

    /// Rewrite the bag so that it holds only the kinds in `keep`, and answer with the raw ids that
    /// were dropped.
    ///
    /// ⭐ **The counterpart to [`Self::debug_give_item`]'s failure mode, and it is the whole of
    /// `docs/coverage-plan.md` step 6's first job.** Gen 1's bag is twenty *kinds*, and a save the
    /// cartridge itself played to the credits arrives with between fourteen and twenty of them
    /// used — five HMs, four TMs, a fossil, the stat items, the spare Revives. So a coverage walk
    /// asking for its fourteen key items got between one and nine of them refused, silently, and
    /// which ones depended on which finished save it started from: no S.S. Ticket is the whole of
    /// the S.S. Anne, no Lift Key is the Rocket Hideout, no rod is every `Fish` row in the game.
    ///
    /// ⚠️ **Dropped rather than moved to the PC, deliberately.** The obvious alternative is to
    /// deposit the junk, and it buys nothing: `wPCItems` is a *second* fifty-slot list that no
    /// coverage row reads, so a walk cannot tell a deposited Calcium from a discarded one. What it
    /// would cost is a second write path to get wrong.
    ///
    /// ⚠️ **Raw ids, because `ItemId` is not the bag.** [`crate::pokemon::bag`]'s reader drops an
    /// entry it cannot name, so a `keep` list matched through `ItemId` would leave an unnameable id
    /// in place and the slot with it. The returned ids are raw for the same reason: a caller that
    /// wants to print them can, and one that cannot name them still knows how many there were.
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
        // The list is 0xFF-terminated after the last pair, exactly as `debug_give_item_id` leaves it.
        self.mmu_mut().write(base + kept.len() as u16 * 2, 0xFF);
        self.mmu_mut().write(pokered_symbols::wNumBagItems.address, kept.len() as u8);
        dropped.into_iter().map(|(id, _)| id).collect()
    }

    /// Replace the whole party. Build members with `Pokemon::maxed` or `Pokemon::new`.
    pub fn debug_set_party(&mut self, party: &PokemonParty) -> Result<(), String> {
        self.mmu_mut().write_player_pokemon_party(party)
    }

    /// Knock the whole party out, active battle Pokémon included — i.e. make the next thing the
    /// cartridge checks a **black-out**.
    ///
    /// Called mid-battle this ends the fight: `MainInBattleLoop`'s `.checkAnyPartyAlive`
    /// (`engine/battle/core.asm:158-162`) and `HandlePlayerMonFainted` (`:703-706`) both run
    /// `AnyPartyAlive` and jump to `HandlePlayerBlackOut` when it answers no, so the loss arrives
    /// through the game's own path — the fade, the halved money, the heal and the warp to the last
    /// Pokémon Centre all happen exactly as they would have.
    ///
    /// ⚠️ **`wBattleMonHP` is the second write and it is not optional.** The battle's copy of the
    /// active Pokémon is a *separate* struct from its party entry, and the faint checks that run
    /// most often are on the copy — zero only the party and the fight simply carries on with a
    /// Pokémon the party says is dead. Harmless outside a battle, where the copy is stale anyway.
    ///
    /// The debug tier, so: fixtures, seeding and diagnostics only. Losing a battle *legitimately*
    /// is not something a test can arrange — it needs the enemy to roll enough damage.
    pub fn debug_faint_party(&mut self) {
        let base = pokered_symbols::wPartyMons.address;
        for index in 0..crate::pokemon::encoding::PokemonBlockAddresses::PARTY_MAX {
            // Offset 1 of the party struct is the big-endian current HP — see
            // `PokemonEncoding::read_pokemon`, which reads it from exactly here.
            let hp = base + index * crate::pokemon::encoding::PokemonBlockAddresses::POKEMON_BLOCK_SIZE + 1;
            self.mmu_mut().write(hp, 0);
            self.mmu_mut().write(hp + 1, 0);
        }
        let battle_hp = pokered_symbols::wBattleMonHP.address;
        self.mmu_mut().write(battle_hp, 0);
        self.mmu_mut().write(battle_hp + 1, 0);
    }

    /// **Step 7** — hold the enemy's live catch rate, so a Poké Ball's outcome stops being a
    /// coin flip.
    ///
    /// `docs/coverage-plan.md` step 7 needs *a ball that fails*, and Gen 1 gives no state in which
    /// one certainly does outside the two hard-coded uncatchables (an unidentified ghost, and the
    /// Marowak on Pokémon Tower 6F once the Scope makes it fightable). `ItemUseBall`'s first test is
    /// `Rand1 - Status > CatchRate → failedToCapture`, so a rate of **0** fails every throw whose
    /// `Rand1` is not itself 0: one throw in 256 gets past it and then has to beat a second roll
    /// against `X ≈ 85`, which leaves about **one throw in 720** catching anyway. The tests that call
    /// this say so and assert loudly rather than pretending the residue is not there.
    ///
    /// ⚠️ **The live byte, not the species' base rate, and it is written once rather than held.**
    /// `wEnemyMonActualCatchRate` is set by `LoadEnemyMonData` on send-out and afterwards only ever
    /// moved by a Safari ROCK or BAIT, so a single write before the first tick lasts the battle —
    /// which is also why holding it every tick would fight the Safari Zone's own arithmetic. A rate
    /// of 0 is a state the cartridge produces for itself; see [`BattleState::enemy_catch_rate`].
    ///
    /// [`BattleState::enemy_catch_rate`]: crate::pokemon::battle::BattleState::enemy_catch_rate
    pub fn debug_set_catch_rate(&mut self, rate: u8) {
        self.mmu_mut().write(pokered_symbols::wEnemyMonActualCatchRate.address, rate);
    }

    /// **Coverage plan step 1.1** — hold the Repel counter, so a long overworld action can be
    /// driven to its end without a wild battle in the middle of it.
    ///
    /// ⚠️ **Re-applied every tick by its caller rather than set once.** `TryDoWildEncounter`
    /// decrements this on every overworld step and prints "REPEL's effect wore off" on the step that
    /// takes it to zero, and 255 is the largest value the byte holds — which a Victory Road boulder
    /// goal walks through. A test that wants *no* encounters holds the counter up instead of
    /// spending it.
    ///
    /// It suppresses an encounter only where the lead party member out-levels the wild one
    /// (`wild_encounters.asm`'s `.CantEncounter2`), which is true of every committed endgame
    /// fixture and of the god party; there is no state here the cartridge could not produce, which
    /// is why this belongs beside the other `debug_*` writes rather than in the play path.
    pub fn debug_set_repel_steps(&mut self, steps: u8) {
        self.mmu_mut().write(pokered_symbols::wRepelRemainingSteps.address, steps);
    }

    /// **Step 7** — hold both sides' battle speed, so a *failed* escape can be arranged.
    ///
    /// `TryRunningFromBattle` leaves the battle outright when the player's speed is greater than or
    /// equal to the enemy's, and every committed mid-battle fixture is that way round: `run` on them
    /// always works, which is why nothing in the suite had ever seen "Can't escape!". With the
    /// player at 1 and the enemy at 255 the quotient the random roll is compared against is 0 on the
    /// first attempt, so it fails unless `BattleRandom` returns exactly 0 — **one attempt in 256** —
    /// and each further attempt adds 30 to the quotient, which is the cartridge's own way of making
    /// sure a player is never trapped.
    ///
    /// ⚠️ **The battle's copies, which is the only place a speed matters mid-fight.** Gen 1 reads
    /// `wBattleMonSpeed`/`wEnemyMonSpeed` for turn order and for this check, and both are re-derived
    /// from the party struct on send-out — so this is written once, mid-battle, and a switch would
    /// undo it. A slow lead against a fast wild Pokémon is the most ordinary situation in the game;
    /// nothing here is a state the cartridge could not produce.
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

    /// **Step 7** — how many Safari Balls are left, so the last one can be thrown on demand.
    ///
    /// A Safari game ends the moment the overworld loop sees `wNumSafariBalls == 0`
    /// (`SafariZoneCheck`, `engine/events/hidden_events/safari_game.asm`), and the ball that takes it
    /// to zero is thrown **inside a battle** — so this is the one way the Safari game really does
    /// end around a fight. ⚠️ The *step* counter cannot: `SafariZoneCheckSteps` runs in the
    /// overworld's step block above the `wIsInBattle` test and warps the player out on the step that
    /// exhausts it, before the encounter roll that step would otherwise make. See the test.
    ///
    /// Walking the count down legitimately means thirty throws, which is thirty paid turns of a
    /// default-tier test to reach the one that matters.
    pub fn debug_set_safari_balls(&mut self, count: u8) {
        self.mmu_mut().write(pokered_symbols::wNumSafariBalls.address, count);
    }

    /// **C1** — hand the player a set of gym badges outright.
    ///
    /// `docs/coverage-plan.md` §3.1. Every HM field move is gated on a badge *and* on a party member
    /// that knows it, and a missing badge is the one refusal the cartridge answers by dropping
    /// straight back to the same party menu with the cursor where it was — which the agent has no
    /// exit condition for. Seeding the badges is how a coverage walk reaches the terrain those moves
    /// unlock without playing eight gyms first.
    ///
    /// ⚠️ **A badge is a capability, not an event flag.** `wObtainedBadges` is read by
    /// `UsedCut`/`UsedSurf`/`UsedStrength` and by the trainer card, and nothing else in the game
    /// keys a *script* off it — which is why this is the one wholesale RAM write §1.2 of the plan
    /// admits. Writing `wEventFlags` instead would desynchronise scripts from map objects, and every
    /// stall found in such a save is a false positive.
    pub fn debug_set_badges(&mut self, badges: crate::pokemon::badge::Badge) {
        self.mmu_mut().write(pokered_symbols::wObtainedBadges.address, badges.bits());
    }

    /// **C1** — every party member to full HP with its status cleared.
    ///
    /// ⚠️ **Overworld only.** Gen 1 copies the active party member into `wBattleMon` on send-out and
    /// writes it back on switch-out, so a party-struct write mid-battle desynchronises the two and
    /// the symptom is a Pokémon that heals and then un-heals on the next switch. The sidecar that
    /// calls this gates on `!in_battle` for exactly that reason — see
    /// [`crate::pokemon::integration_tests::cheats::Cheats`]. Nothing here can enforce it, because
    /// the party struct is all this function can see.
    pub fn debug_heal_party(&mut self) -> Result<(), String> {
        let mut party = self.mmu().read_player_pokemon_party()?;
        for index in 0..party.len() {
            let member = &mut party[index];
            member.current_hp = member.stats.hp;
            member.status = crate::pokemon::status::PokemonStatus::None;
        }
        self.mmu_mut().write_player_pokemon_party(&party)
    }

    /// **C1** — every move of every party member back to its maximum PP.
    ///
    /// Separate from [`Self::debug_heal_party`] because they run at different rates: HP is topped up
    /// after every fight, PP only matters over a long run of them, and a caller that wants one
    /// rarely wants to pay for the other. The same battle caveat applies.
    pub fn debug_restore_pp(&mut self) -> Result<(), String> {
        let mut party = self.mmu().read_player_pokemon_party()?;
        for index in 0..party.len() {
            for slot in party[index].moves.iter_mut().flatten() {
                slot.pp = slot.name.metadata().pp;
            }
        }
        self.mmu_mut().write_player_pokemon_party(&party)
    }

    /// **C1** — put `battle_move` into `slot` of party member `member`, at full PP.
    ///
    /// For the HM slave. A field move needs a party member that knows it, and teaching one the
    /// legitimate way needs the TM/HM in the bag, the right species and a walk to wherever it lies
    /// on the floor — none of which is the thing under test when what is wanted is a boulder pushed.
    ///
    /// `Err` if the member is not there or the slot is out of range; a silent no-op would leave a
    /// caller believing a move is available that is not, which is the same wedge the badge gate is.
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

    /// **Workstream J1** — force the OPTION menu's settings by writing `wOptions` directly.
    ///
    /// §3 of the plan rules the OPTION *menu driver* out of scope — the options are worth setting and
    /// not worth driving — so this is where they get set. Returns `true` when the byte actually
    /// changed, which is what lets a caller re-apply it cheaply every tick and only say so when it
    /// had drifted.
    ///
    /// ⚠️ **It does drift.** `wOptions` lives inside `wMainDataStart..wMainDataEnd`, the block the
    /// game copies to `sMainData` on a save and copies back on CONTINUE (`engine/menus/save.asm:64`,
    /// `:220`) — so a soft reset, and anything that saves and reloads, restores whatever the
    /// cartridge had rather than what was written here. Writing the SRAM copy instead is **not** the
    /// fix: `sMainDataCheckSum` is computed over the whole block (`save.asm:240`), so a byte poked
    /// into `sMainData` fails the checksum and the game answers "the file data is destroyed". Re-apply
    /// instead; [`crate::pokemon::integration_tests::TestFixture::step`] does exactly that.
    pub fn debug_set_options(&mut self, options: &crate::pokemon::options::GameOptions) -> bool {
        use crate::pokemon::options::{GameOptionsReader, GameOptionsWriter};
        // An unreadable byte (a text speed the reader has no name for, which is what a fresh boot
        // leaves) counts as drifted — the point is to end up at `options` either way.
        let drifted = self.mmu().read_game_options().map_or(true, |live| live != *options);
        if drifted {
            self.mmu_mut().write_game_options(options).ok();
        }
        drifted
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

use crate::mmu::MMU;
use crate::pokemon::symbols::{DmgPointer, DmgPointerRead};
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::party::PokemonParty;
use crate::pokemon::pokemon::{Pokemon, PokemonStats, PokemonType};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::symbols::pokered_symbols;
use crate::ram::{RAM, ROM};

pub trait PokemonEncoding {

    fn read_pokemon_party(&self, base_pointer: &DmgPointer) -> Result<PokemonParty, String>;
    
    fn read_player_pokemon_party(&self) -> Result<PokemonParty, String> {
        self.read_pokemon_party(&pokered_symbols::wPartyDataStart)
    }

    fn read_pokemon(&self, party_base_pointer: &DmgPointer, index: u16) -> Result<Pokemon, String>;

    fn write_pokemon_party(&mut self, base_pointer: &DmgPointer, party: &PokemonParty) -> Result<(), String>;
    
    fn write_player_pokemon_party(&mut self, party: &PokemonParty) -> Result<(), String>{
        self.write_pokemon_party(&pokered_symbols::wPartyDataStart, party)
    }

    fn write_pokemon(&mut self, party_base_pointer: &DmgPointer, index: u16, pokemon: &Pokemon) -> Result<(), String>;

    fn read_game_mode(&self) -> GameMode;
}

impl PokemonEncoding for MMU {

    fn read_pokemon_party(&self, base_pointer: &DmgPointer) -> Result<PokemonParty, String> {
        let count = self.read_pointer(base_pointer);
        let mut party = PokemonParty::default();
        let pokemon_pointer = *base_pointer + 8;
        for i in 0..count {
            let pokemon = self.read_pokemon(&pokemon_pointer, i as u16)?;
            party.push(pokemon)?;
        }
        Ok(party)
    }

    fn read_pokemon(&self, party_base_pointer: &DmgPointer, index: u16) -> Result<Pokemon, String> {
        let addresses = PokemonBlockAddresses::of_indexed(*party_base_pointer, index);

        fn parse_move(pokemon_bytes: &[u8], offset: u16) -> Option<PokemonMove> {
            if let Some(name) = PokemonMoveName::from_repr(pokemon_bytes.read(8 + offset)) {
                Some(
                    PokemonMove {
                        name,
                        pp: pokemon_bytes.read(29 + offset)
                    }
                )
            } else {
                None
            }
        }

        fn read_stats(pokemon_bytes: &[u8], offset: u16) -> PokemonStats {
            PokemonStats {
                hp: pokemon_bytes.read_u16_be(offset),
                attack: pokemon_bytes.read_u16_be(offset + 2),
                defense: pokemon_bytes.read_u16_be(offset + 4),
                speed: pokemon_bytes.read_u16_be(offset + 6),
                special: pokemon_bytes.read_u16_be(offset + 8),
            }
        }

        let pokemon_bytes = self.read_pointer_vec(&addresses.pokemon, PokemonBlockAddresses::POKEMON_BLOCK_SIZE as usize);
        Ok(Pokemon {
            nickname: self.read_pointer_pokemon_string(&addresses.nickname),
            trainer_name: self.read_pointer_pokemon_string(&addresses.trainer_name),
            species: PokemonSpecies::from_repr(pokemon_bytes.read(0)).ok_or_else(|| "Invalid Pokemon species".to_string())?,
            current_hp: pokemon_bytes.read_u16_be(1),
            status: pokemon_bytes.read(4).into(),
            types: [
                PokemonType::from_repr(pokemon_bytes.read(5))
                    .ok_or_else(|| "Invalid Pokemon type".to_string())?,
                PokemonType::from_repr(pokemon_bytes.read(6))
                    .ok_or_else(|| "Invalid Pokemon type".to_string())?,
            ],
            moves: std::array::from_fn(|i| parse_move(&pokemon_bytes, i as u16)),
            trainer_id: pokemon_bytes.read_u16_be(12),
            experience: pokemon_bytes.read_u32_be(13) & 0xFFFFFF, // 3 bytes so read as u32 offset -1 and trim top byte
            effort_values: read_stats(&pokemon_bytes, 17),
            individual_values: PokemonStats::from_iv_bytes(
                pokemon_bytes.read(27),
                pokemon_bytes.read(28)
            ),
            level: pokemon_bytes.read(33),
            stats: read_stats(&pokemon_bytes, 34),
        })
    }

    fn write_pokemon_party(&mut self, base_pointer: &DmgPointer, party: &PokemonParty) -> Result<(), String> {
        self.write_pointer(base_pointer, party.len() as u8)?; // length

        let mut species_pointer = *base_pointer + 1;
        let pokemon_pointer = *base_pointer + 8;

        for (index, pokemon) in party.pokemon().iter().enumerate() {
            self.write_pokemon(&pokemon_pointer, index as u16, pokemon)?;
            self.write_pointer(&species_pointer, pokemon.species as u8)?;
            species_pointer += 1;
        }

        // write list end
        self.write_pointer(&species_pointer, 0xFF)
    }

    fn write_pokemon(&mut self, party_base_pointer: &DmgPointer, index: u16, pokemon: &Pokemon) -> Result<(), String> {
        let addresses = PokemonBlockAddresses::of_indexed(*party_base_pointer, index);

        fn write_move(pokemon_bytes: &mut Vec<u8>, offset: u16, move_: Option<PokemonMove>) {
            if let Some(move_) = move_ {
                pokemon_bytes.write(8 + offset, move_.name as u8);
                pokemon_bytes.write(29 + offset, move_.pp);
            } else {
                pokemon_bytes.write(8 + offset, 0x00);
                pokemon_bytes.write(29 + offset, 0x00);
            }
        }

        fn write_stats(pokemon_bytes: &mut Vec<u8>, offset: u16, stats: PokemonStats) {
            pokemon_bytes.write_u16_be(offset, stats.hp);
            pokemon_bytes.write_u16_be(offset + 2, stats.attack);
            pokemon_bytes.write_u16_be(offset + 4, stats.defense);
            pokemon_bytes.write_u16_be(offset + 6, stats.speed);
            pokemon_bytes.write_u16_be(offset + 8, stats.special);
        }

        self.write_pointer_pokemon_string(&addresses.nickname, &pokemon.nickname)?;
        self.write_pointer_pokemon_string(&addresses.trainer_name, &pokemon.trainer_name)?;

        let mut pokemon_bytes = self.read_pointer_vec(&addresses.pokemon, PokemonBlockAddresses::POKEMON_BLOCK_SIZE as usize);
        pokemon_bytes.write(0, pokemon.species as u8);
        pokemon_bytes.write_u16_be(1, pokemon.current_hp);
        pokemon_bytes.write(4, pokemon.status.into());
        pokemon_bytes.write(5, pokemon.types[0] as u8);
        pokemon_bytes.write(6, pokemon.types[1] as u8);
        for i in 0..4 {
            write_move(&mut pokemon_bytes, i as u16, pokemon.moves[i]);
        }
        pokemon_bytes.write_u32_be(13, pokemon.experience & 0xFFFFFF);
        pokemon_bytes.write_u16_be(12, pokemon.trainer_id);
        write_stats(&mut pokemon_bytes,17, pokemon.effort_values);

        let (attack_defense, speed_special) = pokemon.individual_values.into_iv_bytes();
        pokemon_bytes.write(27, attack_defense);
        pokemon_bytes.write(28, speed_special);
        pokemon_bytes.write(33, pokemon.level);
        write_stats(&mut pokemon_bytes, 34, pokemon.stats);

        self.write_pointer_slice(&addresses.pokemon, &pokemon_bytes)?;
        Ok(())
    }

    fn read_game_mode(&self) -> GameMode {
        // Naming screen detection must come before the wIsInBattle check: in Pokémon Red the
        // nickname prompt is shown inside the catch routine while wIsInBattle is still 1.
        // wNamingScreenType ($D07D) is aliased as wPartyMenuTypeOrMessageID and
        // wTempTilesetNumTiles, so it can hold arbitrary values during battle/menu code.
        // Four conditions together uniquely identify a freshly opened naming screen:
        //   1. wNamingScreenType == 2  (NAME_MON_SCREEN exactly; rules out aliased junk)
        //   2. wNamingScreenSubmitName == 0  (reset at screen open, set to 1 on submit)
        //   3. wFontLoaded == 1  (set by the text box that led to the YES/NO choice)
        //   4. wStringBuffer[0] == "@" (0x50)  (DisplayNamingScreen inits buffer empty)
        //      — rules out the false positive when the agent has already written a name
        //      into the buffer before the naming screen has been submitted.
        let font_loaded_byte = self.read_pointer(&pokered_symbols::wFontLoaded) & 0x01;
        if self.read_pointer(&pokered_symbols::wNamingScreenType) == 2
            && self.read_pointer(&pokered_symbols::wNamingScreenSubmitName) == 0
            && font_loaded_byte == 1
            && self.read_pointer(&pokered_symbols::wStringBuffer) == 0x50
        {
            return GameMode::NamingScreen;
        }

        // ; lost battle, this is -1
        // ; no battle, this is 0
        // ; wild battle, this is 1
        // ; trainer battle, this is 2
        match self.read_pointer(&pokered_symbols::wIsInBattle) {
            1 => {
                // The nickname screen appears inside the catch routine while wIsInBattle is
                // still 1.  Conditions 3 (wFontLoaded) and 4 (wStringBuffer) are dropped:
                // the naming screen's own render loop may reset wFontLoaded, and A-mashing
                // from the battle state may have already changed wStringBuffer[0] before this
                // tick runs.  Conditions 1+2 are specific enough in the battle context
                // (wNamingScreenSubmitName stays 0 until START is pressed on the grid).
                if self.read_pointer(&pokered_symbols::wNamingScreenType) == 2
                    && self.read_pointer(&pokered_symbols::wNamingScreenSubmitName) == 0
                {
                    return GameMode::NamingScreen;
                }
                GameMode::WildBattle
            },
            2 => GameMode::TrainerBattle,
            _ => {
                // wFontLoaded infers a text box is open
                // it is set in DisplayTextIDInit and reset in ReloadMapSpriteTilePatterns
                let font_loaded = self.read_pointer(&pokered_symbols::wFontLoaded) & 0x01 == 1;
                if font_loaded {
                    // TODO menu vs dialogue
                    // e.g. the game seems to set the textbox type like this for a message box
                    // 	ld a, MESSAGE_BOX
                    // 	ld [wTextBoxID], a
                    // see TextBoxFunctionTable:
                    GameMode::TextBox
                } else {
                    let flags5 = self.read_pointer(&pokered_symbols::wStatusFlags5);
                    // BIT_SCRIPTED_MOVEMENT_STATE (bit 7): set by StartSimulatingJoypadStates
                    // when the player is being moved by a script (e.g. following Oak to lab),
                    // cleared by player_animations when the movement finishes.
                    let scripted_movement_active = flags5 & 0x80 != 0;
                    let joy_ignore = self.read_pointer(&pokered_symbols::wJoyIgnore);

                    // wScriptedNPCWalkCounter is used by DoScriptedNPCMovement to pace NPC walk
                    // animations. It cycles 8→1 and never resets to 0, so we require
                    // BIT_SCRIPTED_MOVEMENT_STATE to be set to avoid false positives from its
                    // leftover non-zero value after the movement has finished.
                    if self.read_pointer(&pokered_symbols::wScriptedNPCWalkCounter) != 0
                        && scripted_movement_active
                    {
                        return GameMode::Script;
                    }
                    // BIT_SCRIPTED_NPC_MOVEMENT (bit 0) is set by MoveSprite for scripted NPC
                    // walks (e.g. Oak running toward the player in Pallet Town). It can remain
                    // stuck after the player warps away mid-walk, so we also require that the
                    // D-pad bits of wJoyIgnore (PAD_CTRL_PAD = 0xF0) are set — true when the
                    // player is frozen by an active script, but 0 once they are free.
                    if flags5 & 0x01 != 0 && joy_ignore & 0xF0 != 0 {
                        return GameMode::Script;
                    }
                    // wCurOpponent is set by trainer encounter scripts (e.g. rival in Oak's lab)
                    // before InitBattle is called. For trainer battles wIsInBattle is only set to 2
                    // *after* the transition animation, so this window would otherwise look like
                    // Overworld. Treat it as Script so the agent doesn't request an action during
                    // the animation.
                    if self.read_pointer(&pokered_symbols::wCurOpponent) != 0 {
                        return GameMode::Script;
                    }
                    GameMode::Overworld
                }
            }
        }
    }



}



#[derive(Debug, Copy, Clone, Eq, PartialEq, strum_macros::Display, Default)]
pub enum GameMode {
    #[default]
    Overworld,
    #[strum(serialize = "Wild Pokemon Battle")]
    WildBattle,
    #[strum(serialize = "Trainer Battle")]
    TrainerBattle,
    #[strum(serialize = "Text Box")]
    TextBox,
    /// A map script or NPC scripted walk is running (`wCurrentMapScriptFlags` is non-zero).
    /// The player is frozen; the agent should advance the script by pressing A.
    Script,
    /// The Pokémon nickname entry screen (`DisplayNamingScreen`) is active.
    /// Detected when `wNamingScreenType > 0` and `wNamingScreenSubmitName == 0`.
    #[strum(serialize = "Naming Screen")]
    NamingScreen,
}

pub struct PokemonBlockAddresses {
    pub pokemon: DmgPointer,
    pub trainer_name: DmgPointer,
    pub nickname: DmgPointer,
}

impl PokemonBlockAddresses {
    pub const PARTY_MAX: u16 = 6;
    pub const POKEMON_BLOCK_SIZE: u16 = 0x2C;
    pub const NAME_LENGTH: u16 = 0xB;

    fn of_indexed(party_base_pointer: DmgPointer, index: u16) -> Self {
        Self {
            pokemon: party_base_pointer + index * Self::POKEMON_BLOCK_SIZE,
            trainer_name: party_base_pointer + Self::PARTY_MAX * Self::POKEMON_BLOCK_SIZE + index * Self::NAME_LENGTH,
            nickname: party_base_pointer + Self::PARTY_MAX * Self::POKEMON_BLOCK_SIZE + Self::PARTY_MAX * Self::NAME_LENGTH + index * Self::NAME_LENGTH,
        }
    }
}

pub fn reverse_bcd(mut value: u32) -> u32 {
    let mut result = 0u32;
    let mut multiplier = 1u32;
    while value > 0 {
        let digit = value & 0xF;
        result += digit * multiplier;
        multiplier *= 10;
        value >>= 4;
    }
    result
}

#[cfg(test)]
mod tests {
    use crate::roms::blargg_cpu::ROM;
    use crate::pokemon::*;
    use crate::pokemon::encoding::reverse_bcd;

    #[test]
    fn test_reverse_bcd() {
        assert_eq!(reverse_bcd(0x3000), 3000);
        assert_eq!(reverse_bcd(0x1234), 1234);
        assert_eq!(reverse_bcd(0x0000), 0);
        assert_eq!(reverse_bcd(0x9999), 9999);
        assert_eq!(reverse_bcd(0x0001), 1);
        assert_eq!(reverse_bcd(0x0012), 12);
        assert_eq!(reverse_bcd(0x0100), 100);
    }

    #[test]
    fn test_full_pokemon_encoding() -> Result<(), String> {
        let mut mmu = MMU::from_rom(ROM)?;

        let mut party = PokemonParty::default();
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Charizard,
                "CHARIZARD",
                [
                    PokemonMoveName::Flamethrower,
                    PokemonMoveName::FireBlast,
                    PokemonMoveName::Fly,
                    PokemonMoveName::Slash,
                ],
                "TRAINER1",
                11111,
            )
        )?;
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Mewtwo,
                "MEWTWO",
                [
                    PokemonMoveName::Psychic,
                    PokemonMoveName::IceBeam,
                    PokemonMoveName::Thunderbolt,
                    PokemonMoveName::Recover,
                ],
                "TRAINER2",
                22222,
            )
        )?;
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Snorlax,
                "SNORLAX",
                [
                    PokemonMoveName::BodySlam,
                    PokemonMoveName::Rest,
                    PokemonMoveName::Bite,
                    PokemonMoveName::Earthquake,
                ],
                "TRAINER3",
                33333,
            )
        )?;
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Gyarados,
                "GYARADOS",
                [
                    PokemonMoveName::HydroPump,
                    PokemonMoveName::DragonRage,
                    PokemonMoveName::Bite,
                    PokemonMoveName::Surf,
                ],
                "TRAINER4",
                44444,
            )
        )?;
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Alakazam,
                "ALAKAZAM",
                [
                    PokemonMoveName::Psychic,
                    PokemonMoveName::Recover,
                    PokemonMoveName::Psybeam,
                    PokemonMoveName::Reflect,
                ],
                "TRAINER5",
                55555,
            )
        )?;
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Dragonite,
                "DRAGONITE",
                [
                    PokemonMoveName::HyperBeam,
                    PokemonMoveName::Fly,
                    PokemonMoveName::Thunderbolt,
                    PokemonMoveName::Surf,
                ],
                "TRAINER6",
                65535,
            )
        )?;

        mmu.write_player_pokemon_party(&party)?;

        let result = mmu.read_player_pokemon_party()?;

        assert_eq!(party, result);
        Ok(())
    }

    #[test]
    fn test_partial_pokemon_encoding() -> Result<(), String> {
        let mut mmu = MMU::from_rom(ROM)?;

        let mut party = PokemonParty::default();
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Charizard,
                "CHARIZARD",
                [
                    PokemonMoveName::Flamethrower,
                    PokemonMoveName::FireBlast,
                    PokemonMoveName::Fly,
                    PokemonMoveName::Slash,
                ],
                "TRAINER1",
                11111,
            )
        )?;

        mmu.write_player_pokemon_party(&party)?;

        let result = mmu.read_player_pokemon_party()?;

        assert_eq!(party, result);
        Ok(())
    }
}
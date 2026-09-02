# The walkthrough the model is given

Nine markdown files, one per badge the player has not won yet. `src/llm/guide.rs` picks one from
the badges and `read_guide` hands it over verbatim: no template, no rendering, no truncation. What
is in these files is exactly what the model sees, which is the only cheap way to review it.

## What is in a chapter, and what is deliberately not

Three things, in this order: **the route**, **the blockers**, **the boss**.

A chapter is not a list of everything in the area. Items are named only when the item *is* the
progression: Oak's Parcel, the S.S. Ticket, the five HMs, the Card Key, the Silph Scope, the Poké
Flute, the Secret Key, the Gold Teeth. Everything else a player would enjoy collecting is left out
on purpose, because the failure this exists to fix is a model that does not know where to go, and a
collectathon is a longer answer to a different question.

The blockers earn their place from the deployed runs. One produced the same impossible walk 143
times because an old man stands in the north exit of Viridian City until Oak's Parcel is delivered,
and nothing ever told it so. Every chapter says what is shut and what opens it.

## Where the facts come from

**`pokered`**, which is in this repo and is the only authority here that cannot go stale. Everything
in these chapters that is a number was read out of the disassembly:

| Claim | Checked against |
|---|---|
| Every Gym Leader's party | `data/trainers/parties.asm`, `BrockData` … `BlaineData` |
| Giovanni's three parties | `GiovanniData` — Rocket Hideout, Silph Co. 11F, Viridian Gym |
| The Elite Four and the Champion | `LoreleiData`, `BrunoData`, `AgathaData`, `LanceData`, `Rival3Data` |
| Every rival battle | `Rival1Data`, `Rival2Data` |
| Snorlax at 30, the tower's Marowak at 30 | `scripts/Route12.asm`, `scripts/PokemonTower6F.asm` (`RESTLESS_SOUL`) |
| The Silph Co. Lapras at 15 | `scripts/SilphCo7F.asm` (`lb bc, LAPRAS, 15`) |
| ₽500, 30 Safari Balls, 502 steps | `scripts/SafariZoneGate.asm` |
| SonicBoom always dealing 20 | `engine/battle/core.asm` (`SONICBOOM_DAMAGE`) |
| Viridian Gym opening on the other seven badges | `ViridianCityCheckGymOpenScript`, `wObtainedBadges` vs `~(1 << BIT_EARTHBADGE)` |
| The old man blocking Viridian City's north exit | `ViridianCityCheckGotPokedexScript` — tile (19, 9), until `EVENT_GOT_POKEDEX` |
| Oak stopping you at the north edge of Pallet Town | `PalletTownDefaultScript` — `wYCoord` 1, until `EVENT_FOLLOWED_OAK_INTO_LAB` |
| The starter Poké Balls being inert before that | `OaksLabSelectedPokeBallScript` — `EVENT_OAK_ASKED_TO_CHOOSE_MON`, else `_OaksLabThoseArePokeBallsText` |
| The boy turning you back from Route 3 | `scripts/PewterCity.asm` — `EVENT_BEAT_BROCK` + `PewterCityPlayerLeavingEastCoords` |
| The Saffron gate guards wanting a drink | `scripts/Route7Gate.asm` — `BIT_GAVE_SAFFRON_GUARDS_DRINK`, `RemoveGuardDrink` |
| The Lift Key needing a *second* talk to Rocket 3 | `scripts/RocketHideoutB4F.asm` — `RocketHideoutB4FRocket3AfterBattleText` sets `EVENT_ROCKET_DROPPED_LIFT_KEY` and `ShowObject`s the ball |
| Giovanni's door opening on both B4F Rockets | `RocketHideoutB4FDoorCallbackScript` — `EVENT_BEAT_ROCKET_HIDEOUT_4_TRAINER_0` and `_1` |
| The B1F lift door staying shut | `RocketHideoutB1FDoorCallbackScript` — `EVENT_BEAT_ROCKET_HIDEOUT_1_TRAINER_4` |
| The lift stopping at B1F, B2F and B4F only | `RocketHideoutElevatorFloors` |

A second oracle is worth naming, because it caught more than the disassembly did on its own:
**`PolicyStep::*_steps()` in `src/pokemon/policy.rs`**, the scripted route `full_playthrough` uses to
beat the game. It cannot be out of date about a gate, because a missing one would fail the test. A
sweep of it against these chapters turned up five things no chapter had said:

- a cut tree seals the way into `VermilionGym`, and another seals `CeladonGym` from the city;
- **a cut tree grows back whenever the map reloads**, so beating a gym leader shuts you back inside
  the enclosure you cut your way into;
- a Rocket stands in front of the Game Corner poster and has to be beaten before it can be examined;
- the Safari Zone's `SafariZoneWest` — which holds both the Gold Teeth and HM03 — is reachable only
  the long way round, Centre → East → North → West, because the Centre's own west exit is across
  unsurfable water;
- the Silph Co. Card Key is in a pocket that can only be *arrived* in, off the 5F↔9F teleport pair.

⚠️ **A chapter cannot see a gate that is not a badge, so one has to be repeated.** `chapter_index`
reads `wObtainedBadges` and nothing else, and the Silph Scope is not badge-gated: the Rocket Hideout
is chapter 3's and the Pokémon Tower is chapter 4's, but nothing in the cartridge makes you take the
hideout before Erika. The deployed run of 2026-09-02 beat her first, lost the only page that said
where the Scope was, guessed Silph Co from the name, and spent fifty turns walking between Lavender
and Celadon. Chapter 4 now opens with the hideout, guarded by "if it is not in the bag" — the same
duplication is owed to anything else a chapter assumes was picked up in the one before it.

⚠️ `BIT_STRENGTH_ACTIVE` is the same shape and is already a ⚠️ in `CLAUDE.md`: Strength is armed once
per map and cleared by every map change, so a boulder push on a floor you have just walked on to does
nothing. Chapters 6 and 8 say so, because `tools::hm_available` refuses the call and the model would
otherwise have no idea why.

⚠️ **That table is what has been checked, not a claim about every sentence.** The prose around those
numbers — which building a person stands in, roughly where an item lies, what a puzzle amounts to —
is not individually sourced, and a chapter that sends the model to the wrong side of a room is a bug
to fix rather than a surprise. Two have been found and fixed this way already, and both were the
same shape: **an unstated trigger**. Chapter 0 sent the model into Oak's lab for a starter without
saying that walking north out of Pallet Town is what unlocks it — which is precisely the state the
deployed run spent thirty turns in, re-reading "Gramps isn't around!" and re-picking the same three
inert Poké Balls. Chapter 1 had a policeman step aside from the Cerulean trashed house; there is no
such policeman, only two permanent flavour NPCs who say the place was robbed. A guide that names a
gate is only useful if the gate is real, so a claim of the form "X is now open" is the kind to check
against a script before writing it.

The order to do things in is the one thing the disassembly does not state, since it is route
planning rather than data. For that, zerokid's
[Pokémon Red walkthrough](https://gamefaqs.gamespot.com/gameboy/367023-pokemon-red-version/faqs/64175)
was read as a cross-check on the route. Where it and the disassembly disagreed, the disassembly won:
the Champion's fourth and fifth Pokémon vary with the starter you took, and `Rival3Data` is what
says so.

## The one hard rule

⚠️ **Backticks mean "this is a `Map` variant", and nothing else in the prose is backticked.** That is
what makes a place name here the same string as the turn's `Location:` line, the action menu's ids
and `read_route`'s argument, so the model can copy one straight across without translating.
`every_place_the_guide_names_is_a_real_map` fails the build if a chapter names somewhere that is not
a map — including the two mistakes that read perfectly well and are still wrong: a bare floor suffix
(`` `B4F` ``) and a collective name for a multi-floor dungeon (`` `SilphCo` ``, `` `VictoryRoad` ``).

The prose around the keys is free to be reworded at any time. The keys are not.

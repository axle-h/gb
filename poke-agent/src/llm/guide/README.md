# The walkthrough the model is given

Nine markdown files, one per badge the player has not won yet. `src/llm/guide.rs` picks one from
the badges and `read_guide` hands it over verbatim, so what is in these files is exactly what the
model sees.

## Rules for a chapter

- Three things, in this order: the route, the blockers, the boss. Items are named only when the item
  is the progression (Oak's Parcel, the S.S. Ticket, the HMs, the Card Key, the Silph Scope, the Poké
  Flute, the Secret Key, the Gold Teeth).
- Every chapter says what is shut and what opens it: an unstated gate is a walk the model repeats for
  ever.
- Backticks mean "this is a `Map` variant" and nothing else, so a place name is the same string as the
  turn's `Location:` line, the menu ids and `read_route`'s argument.
  `every_place_the_guide_names_is_a_real_map` fails on anything else, including a bare floor suffix
  (`` `B4F` ``) and a multi-floor collective name (`` `SilphCo` ``).
- `chapter_index` reads only `wObtainedBadges`, so a prerequisite that is not badge-gated (the Silph
  Scope) is repeated in the next chapter under "if it is not in the bag".
- Say only what the tool catalogue can act on: boulders are an action-menu row, so no chapter tells
  the model to arm Strength.
- A claim of the form "X is now open" is checked against the map's script before it is written.

## Where the facts come from

Every number is read out of `pokered`:

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

The gates the table does not list are checked against `PolicyStep::*_steps()` in
`src/pokemon/policy.rs`, which `full_playthrough` fails on if one is missing. The order of the route
is cross-checked against zerokid's
[Pokémon Red walkthrough](https://gamefaqs.gamespot.com/gameboy/367023-pokemon-red-version/faqs/64175);
where it disagrees with the disassembly, the disassembly wins.

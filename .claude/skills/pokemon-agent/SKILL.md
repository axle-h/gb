---
name: pokemon-agent
description: "PokemonAgent, Policy and the watchdog: the closed loops A-only input walks into, screen-versus-RAM timing, the prose the model and the page read, what an abort reports, and the SPA event stream that folds it. Load before touching src/pokemon/{agent,policy,text,tile_map,actions}.rs, AgentEvent or any Display it goes through, or web/src/useEventStream + Conversation."
---

# The Pokémon agent and what it reports

*Extracted from `CLAUDE.md`, which holds the rules of the road and the index of these skills. The
README is imported into `CLAUDE.md` and is not repeated there or here: this file has only the
invariants and the traps, nearly every one of which was learned by breaking something.*

## Agent and policy invariants

⚠️ **`PokemonAgent::poll_policy` is the single seam every decision point goes through**, and it resets the clock
the stuck-run watchdog reads. Call `policy.service_tools` directly from a new poll site and the watchdog believes
the run has been wedged since that moment, forever.

⚠️ **The emulator never pauses while the model thinks, and must not be made to.** A tool batch is answered by
`Policy::service_tools`, which only runs when `gb.run` advances the agent — so any pause spanning an LLM tool call
deadlocks the run. `GB_PAUSE_WHILE_THINKING` was built in W4 and removed the same day; `HostConfig` carries the ⚠️.
**The one exception is the shape of the rule, not a hole in it**: the park on a spent quota (the `llm-turn-loop` skill) happens when a
request has already failed, with nothing outstanding to service. Any future pause has to clear that bar.

**The watchdog** (`Policy::{stuck_timeout, pick_unstick}`) raises a `DecisionKind::Stuck` turn whose only terminal
tools are `press_buttons` and `wait`. ⚠️ **It is asked on every tick of the jam, not once** — a tool batch is only
serviced inside `agent.update`, so a one-shot notification would hang any turn that wanted to read first. ⚠️ **And
it must not reset the clock it reads**, or the jam clears the instant it is noticed. It never fires in a healthy
run: `ordinary_play_stays_far_inside_the_stuck_timeout` measures the longest silence at ~6 s against the 300 s
default.

### Grinding: who leads, and where

⚠️ **A grind's trainee must be the *lead*, not switched in — it is worth the turn and it is worth half the
experience.** `pick_battle_action`'s training block puts a bench trainee in on turn one of every battle, which
costs that turn *and* halves the payout: `DivideExpDataByNumMonsGainingExp` splits a knockout between everything
that took the field, and a lead that switches out has still taken it. `pick_field_move` promotes the trainee with
a direct RAM reorder instead, and the completion test is that `target.resolve` now says slot 0 — visible in RAM,
like `UseStrength`, rather than a latch a restart would forget. ⚠️ The in-battle switch is still needed and must
not be deleted: the **hand-off** to a tank on turn 1 is what stops a lv26 trainee fainting to a lv39 wild, and it
keys on the trainee being active, which a lead already is.

⚠️ **A grind *leaves its trainee leading*, and only the gauntlet one gets away with it.** The
promotion above is a direct RAM reorder and nothing undoes it; `elite_four_steps` happens to re-lead
the starter two steps later, so the endgame grind never showed the bug. A mid-game grind does: after
the Route 11 Drowzee grind the run walked to Celadon with a **lv24 Drowzee at 9/69 HP and poisoned**
leading every trainer battle in Rock Tunnel, the Pokémon Tower and Erika's gym, with the lv38
Blastoise behind it only ever switched in by the "HP critical" arm. Every `GrindUntilLevel` on a
bench mon needs a `MovePokemonToFront` after it.

⚠️ **The trainee-fainted heal detour re-arms itself, so the detour's own give-up is not one.**
`MAX_HEAL_ROUTE_WAIT` correctly abandons a heal it cannot route and sets `heal_unreachable` — and
`GrindUntilLevel`'s arm then set `heal_return` again on the very next tick, because the trainee is
still fainted and the step is still at the front. Measured on a Route 11 grind: **158 heal trips,
none of which moved a tile**, until the budget ran out. It honours `heal_unreachable` now by handing
the route back. ⚠️ **What made it unroutable is the incremental graph's *return* edge**: an
`enter(Route11)` records the way out and nothing records the way back, so `route_toward` could not
find Vermilion from one map away. Walking home and back once before the grind is the fix, and it is
the same shape as the Nugget Bridge's double crossing.

⚠️ **A catch leaves its catcher fainted, so a grind that follows one has to heal first.** The
trainee is switched in at whatever level it was caught and takes hits while the balls are thrown, so
`GrindUntilLevel` starts on a fainted mon and goes straight to the detour above.

⚠️ **A grind paces with `wander_action`, never toward a warp — and the warp fallback cost 4435 map changes.**
`GrindUntilLevel`'s cave/building branch walks to "the farthest reachable object", and on a floor whose only
sprites are item balls (excluded, because walking onto one loops on a full bag) that used to fall through to *the
farthest warp*. Measured on the gauntlet grind: 1097 walks out of the Pokémon Mansion onto Cinnabar Island and
1103 up to 2F, in 1552 battles — the run left the building and came back over two thousand times, and the arm
above routed it home each time. It buys nothing: the encounter roll is on the **step**, so pacing on the spot
finds battles at the same rate. `MetaTileMap::wander_action` targets only Empty/Grass/Water, `agent.rs` turns a
plain-floor destination straight into `PacingForEncounters`, and `CatchPokemon` and `SweepDex` had both been
using it for years. Six map changes now, all of them the walk in.

⚠️ **A grind that carries no medicine cannot heal, and the run reached its longest grind carrying none.**
`pick_battle_action`'s "HP critical" arm works and simply had nothing to reach for: measured on
`post-articuno.bin`, seventeen bag entries, ¥37,655, and not one potion or status cure — so the arm never fired
**once in 1552 battles** and the trainee was ticked to death by burn and poison twelve times, each one a
four-warp round trip to a Centre. The same gap, found separately, is what put Super Potions in
`back_to_cerulean_steps`. ⚠️ **Cure on a threshold, not on sight**: curing every status application spent twenty
Full Heals inside the first of three grinds, because a trainee that one-shots the floor is re-statused constantly
and almost none of those episodes would have reached zero.

⚠️ **And then check the *mainline* can pay for it, because this one cannot.** From the fixture the purchase lands
twenty Full Heals and the trips go to zero; the run that earns its own way to the same shop arrives on about
**¥2,000** against the fixture's ¥37,655, `agent::affordable` trims the order to **three**, and the trips come
back (13 on `hall_of_fame_playthrough`). That is the fixture-versus-mainline hole the `test-suite` skill warns
about, in its purest form — the leg test proved the mechanism, not the affordability. ⚠️ **The money is poor
because a black-out halves it and the route has eleven before this point**, so the medicine that would prevent
them is what they make unaffordable; the early game is upstream of the grind's economy.

⚠️ **Argue about a grind site with `wild::tests::probe_grind_sites` and never from memory** — it ranks every
encounter block in the ROM out of the cartridge's own tables. ⚠️ **Read experience per *knockout* first and per
step second**: the measured grind is 1552 battles in 1229 s, which is ~40 s of cartridge time an encounter cycle,
of which the 25.6 steps a 10/256 rate implies are under 7 s. Four fifths battle, one fifth walk — so a site with
twice the payout wins even at half the rate. `poison_share` is the
third column and it is a *travel* cost — Gen 1 ticks overworld poison at 1 HP per four steps and cures it only at
a Centre, so a poisonous site sends the trainee home over and over (`grind_heal_trips` counts it).

⚠️ **Cerulean Cave is the site the ranking points at and it cannot be used before the Elite Four — do not
re-propose it.** It is not gated by a script: `wNumHoFTeams` is read by four things in the ROM and none is in
Cerulean, and the man beside the door only *says* champions only. He gates it with his body, standing at (4,12)
on the one approach to `warp_event 4, 11`; probed from a pre-Champion save on the terrace, the warp is not in the
reachable set at all. The whole argument, and the twenty-two-hop walk that does work, is in
`PolicyStep::gauntlet_grind_steps`.

⚠️ **A `Goto` cannot cross Kanto from a leg test.** `route_toward` reads the incremental world graph, which lives
on the agent and not in the save state, so a fixture builds an agent that has observed nothing and routes nowhere
— measured, the step sat on one tile for a whole budget. Explicit `enter` hops read only the current map's
`actions()`, which is why they work from a cold fixture and why a traversal written that way is testable at all.
⚠️ And every gate house crossing must be `enter_at`: a gate warps to `LAST_MAP`, so a plain `enter` back onto the
route is satisfied by the door just walked in through, and `enter_map_action` prefers the nearest.

⚠️ **A black-out is reported against the last *battle* map, because the cartridge's sentence arrives late.** The
"blacked out" text box is committed only when the reader flushes, several overworld decisions after the warp — so
an overworld-map version prints the map the run has already walked back to (`lost on MtMoonB2F` for a black-out
that warps to Route 4). A full playthrough has twelve. ⚠️ **They are not a levelling problem and five attempts to
fix them all reduced the count and broke the run somewhere else** — the early route is tuned to one RNG stream
rather than robust to it, so anything added before Vermilion re-rolls onto a different pre-existing fault (the
Nugget Bridge, Route 6, a full bag at Silph Co, Erika's tree maze). The table of what each variant cost is on
`PolicyStep::game_steps`, at the lone-starter comment; read it before trying a sixth. Two of them are worth
fixing on their own account whatever happens to the route: ⚠️ **`CollectItem` has no give-up** — its completion
is the sprite disappearing, so a refused pickup spins silently until the cycle budget dies (16,763 polls one tile
from the Card Key) — and ⚠️ **the bag's 20-entry cap is load-bearing and invisible**, since a full bag refuses
every pickup in the game without saying so.

### Field moves

⚠️ **A field move is answered by whoever in the party knows it, and *both* halves of that used to be
assumed.** `CuttingTree` drove the party menu onto **slot 0** unconditionally — right only while the
starter is the Cut carrier, and a permanent wedge the moment an HM slave is (the driver's only exit
is the overworld coming back). `Surfing` hard-coded the **move index** to 0 — right only while the
surfer knows exactly one field move, and a Blastoise carrying Surf, Strength and Dig lists three.
`policy::field_move_carrier` is the one place that resolves both, first holder wins, and
`UseStrength`, `Dig`, `UseFlash`, the Cut driver and the surf mount all go through it;
`llm::tools::resolve_field_move` already did the same thing for the model's side. A `PolicyStep` that
still names a `PartyRef` uses it only as the fallback when nobody knows the move at all, which is the
case the refusal messages are for.

⚠️ **A policy that answers `None` for ever is indistinguishable from one that is thinking, and the
watchdog is blind to it by construction.** `since_last_policy_poll` is reset by `poll_policy`
*whatever the answer is*, so a policy asked every tick and answering nothing every tick looks
perfectly healthy: the agent sits in `BattleState::AwaitingPolicy` showing the main battle menu, the
emulator runs, and **nothing is printed**. Three `full_playthrough` runs ended in that silence —
twice on Erika's Victreebel, once on her Vileplume, one of them for **seven hundred minutes of game
time** — and each looked like a hung process rather than a bug.

The cause was `battle_options`: ⚠️ **every move at zero PP is Struggle, not "no move"**.
`available_battle_moves` filters on `pp > 0`, and in a *trainer* battle there is no `Run` and a party
with nothing else conscious offers no `SwitchPokemon`, so the whole option list was bag items — which
`pick_battle_action`'s last resort deliberately refuses to touch (the first bag entry is often a key
item the game will not use). The moves are offered anyway when the list would be empty, and the
cartridge substitutes Struggle. Two changes, both needed: the fix, and the one path out of
`pick_battle_action` that answers nothing now *says so* with the options attached — which is what
found it.

⚠️ **Its guard asserts on battle actions *taken*, not on silence**, for the same reason the watchdog
missed it: `a_party_with_no_pp_anywhere_still_gets_an_answer` counts `BattleActionStarted` events, and
a silence-based version passes on the very state that reproduces the bug.

⚠️ **And a battle can be answered every turn for ever, which is the same stall with the silence
taken out.** A *ghost* battle — `IsGhostBattle`, and it is **every wild battle on Pokémon Tower
1F-7F until the Silph Scope**, not merely the Marowak — executes no move at all: "too scared to
move" one way, "Get out..." the other, no hit points either side, so it can never end. The deployed
run of 2026-09-01 chose Slash against a Gastly every 3.3 s for as long as anyone watched. Nothing
could see it: a **battle script** was answering on the emulator thread, so no request was made and
the model was never asked, and the watchdog was quiet because the agent was reaching a decision
point every tick. ⚠️ **So the guard cannot count actions *or* silence** — there were 35 actions in
120 s of game time and no silence at all — it asserts the **battle ends**
(`a_ghost_battle_is_left_rather_than_fought_for_ever`, on the deployed save state itself). ⚠️ **And
it must not use `RandomPolicy`**, which draws `Run` out of a short list within a few turns and
passes whether or not anything is understood.

`battle::is_ghost_battle` is the predicate and `battle_options` returns `Run` **alone** on it —
running is not the best option there, it is guaranteed, since `TryRunningFromBattle` jumps to
`.canEscape` above the speed check. ⚠️ **Prose was not enough and `enemy_trapping` is not the
precedent**: a wrap ends by itself in a few turns and items, switching and running all still work,
so telling the model is right *there*; nothing about a ghost ends by itself, and a script never
reads the prompt at all. ⚠️ **`DeterministicPolicy` gets the same early return even though the route
holds the Scope long before the tower** — its flee arms are conditional on HP and PP, and the
fall-through past them looks for a move to use, which a one-element list does not hold, so it would
answer `None`, which means *still thinking*.

### One fighter, and what the party is for

⚠️ **Three Pokémon at lv75 is 1.4 M experience; one at lv85 is 425 k, and the one wins more
comfortably.** Experience is cubic, so the top of a single curve costs less than the middle of three —
measured on the three-target grind, Hypno 26→75 was ~404 k, Articuno 50→75 ~610 k and the starter
60→75 ~400 k, thirty minutes of `hall_of_fame_playthrough`. One starter to 85 is ~840 wild battles and
the run finishes in **26 minutes against 50**. And it is *stronger*: the Elite Four tops out at
Lance's lv62 Dragonite and the rival's lv65, so a lead that far ahead one-shots almost everything
rather than trading turns with it.

⚠️ **This replaced "three fighters or you lose the Champion's room", which was true of three at
seventy-five.** The party behind those two leads was a lv26 Vaporeon, a lv30 Slowpoke and a lv24
Machop, so the moment the second lead fell three fodder mons fainted in a row. Depth of bench was never
the answer; height was.

⚠️ **What that makes the binding constraint is PP, not power.** Five rooms is about twenty-six
knockouts against Surf's 15, Blizzard's 5 and Dig's 10 — so the fighter's four slots are worth
defending, which is the whole of the next rule.

⚠️ **Surf is the only HM the fighter carries.** An HM is the one move `pick_move_to_forget` will never
drop (`never_forgets_hm`), so teaching one spends a *permanent* slot in the only Pokémon that attacks.
Surf earns it at 95-power STAB; Strength at 80-power Normal does not, and it went back onto a Victory
Road Machop caught two tiles from the boulder it is for. Cut was never a choice — `wartortle.asm` has
no CUT — so an Oddish carries that. Neither slave ever fights.

⚠️ **A step list that drops a leg inherits that leg's *first* step.** Removing the Seafoam detour left
`gauntlet_grind_steps` opening on a Pokémon Centre two warps from Blaine's gym, because
`seafoam_articuno_steps` used to supply the `enter(CinnabarIsland)` that got out of it — and `EnterMap`
is a deliberate single hop. The same shape hit `back_to_cerulean_steps` when the Route 11 catch went.

⚠️ **Gen 1's bag holds 20 entries and the route runs on exactly that, so *every* pickup is somebody
else's toss.** Three failures in one run, each silent: the Card Key refused (3000 polls one tile away,
then the rest of Silph Co locked), TM14 Blizzard filling the slot the Rare Candy had just freed so the
**Secret Key** was refused, and the Silph President's Master Ball refused — which surfaces two legs
later as "no Pokéballs left" at Articuno. A TM is consumed when taught, so teaching it immediately is
itself a toss.

⚠️ **`Bag::best_pokeball` ranks by effectiveness, so a fallback spends the Master Ball.** A pinned
`ball` only holds while that ball is in stock: the Seafoam Slowpoke's Great Balls failed to buy on an
empty wallet, the catch fell back to "best", and the bird had nothing left to be caught with.

### Black-outs, and how the route got to none

⚠️ **A black-out is a walk to a Pokémon Centre with the money halved, and the route used to take
twelve of them a run.** Every one measured had the same shape: the lead walks into a fight already
worn from the last one, loses in a turn or two, and wakes in the Centre it should have walked to.
Nothing was malfunctioning — `pick_battle_action`'s "heal below 25%" arm fires correctly and simply
had nothing in the bag to reach for, and its flee-to-heal arm cannot fire in a *trainer* battle at
all. So the decision moved out of the battle and into the overworld tick before it. **Twelve to
zero**, in six changes, none of which is a bigger number in a step list:

- ⚠️ **The heal detour is armed proactively** (`needs_a_centre` + the arm above `heal_return`'s
  block). Three reasons to go, deliberately different: the lead is fainted; its attacks are spent
  (**no Gen 1 mart sells Ether or Elixer** — `data/items/marts.asm` — so a Centre is the only PP in
  the game before the S.S. Anne's floor items); or it is badly hurt and **the bag cannot fix it**.
  ⚠️ That last test is "can the best thing in the bag cover what is missing", not "is there
  medicine": a Potion is +20, which is a fifth of a lv21 Wartortle, and asking the weaker question
  kept the detour quiet while the run black-ed out on Route 3 anyway.
- ⚠️ **It is only allowed where the route can get back**, which is `Map::is_overworld` (towns and
  Routes — discriminants at or below `Route25`) **or a step that re-derives its own route**
  (`step_finds_its_own_way_back`: the grinds, the catches, `Goto`, `DefeatGymLeader`, `CutTree`).
  Map alone was too narrow — Koga's gym is six trainers and an invisible-wall maze one warp from the
  Fuchsia Centre, and the run black-ed out in it — and step alone would let a detour abandon a chain
  of single-hop `EnterMap`s through Mt Moon or Silph Co, which cannot be resumed from a Centre.
- ⚠️ **A heal is finished when the party is full, not when the conversation lands.** `Interact` pops
  the instant it issues the walk, so every `Interact(NURSE)` in the route was a *request* to heal:
  whether the party actually came back full depended on how long the next step happened to take.
  Caught by asserting it on a fixture, which came out carrying **Water Gun on 6 of 25 PP** one step
  after a Pokémon Centre. `party_is_fresh`, bounded by `MAX_HEAL_WAITS`.
- ⚠️ **The wild-flee threshold depends on whether the bag can answer, and running is worth it even
  with nowhere to go.** At a flat 15% the run was dying to the *next* hit; with an empty bag it now
  leaves at a third. The `last_pokemon_center` that used to gate the arm gated the wrong half — on
  the first errands out of Pallet there is no Centre yet, and fleeing is still better than fainting.
- ⚠️ **A charge move is half the damage and a free hit for the opponent** (`damage_per_turn`).
  `PokemonMoveEffect::Charge` — Skull Bash, Razor Wind, Solarbeam, Sky Attack and Dig — was ranked on
  raw power, so a Blastoise holding Surf *and* Dig picked **Skull Bash** into a Poison type, was
  badly poisoned mid-charge and fainted without landing it. Halving is enough on its own: Dig into
  Poison is 2× before the halving and still wins.
- ⚠️ **The "switch to a fresher attacker" arm had a level gate doing the damage gate's job badly.**
  "Within eight levels of the active" is a proxy for "not a sacrificial weakling", and the two lines
  below it already test exactly that — the bench mon must do 1.5× the active's damage *and* three-shot
  this enemy. A lv20 Hypno passes both against Erika's Grass/Poison and failed the proxy against a
  lv45 Blastoise, so the run lost that gym with its answer sitting on the bench.

⚠️ **A grind goes home on empty, not on low, and the difference is nineteen minutes.** Out on the
route a fifth of a tank is the right moment to turn back, because the next fight is a trainer who
cannot be fled. Inside the gauntlet grind every wild *can* be fled and the trainee is handed off to a
tank whenever one threatens it, so the same threshold cost **46 round trips** from the Pokémon Mansion
to Cinnabar and back — `hall_of_fame_playthrough` went from 31 minutes to 51 on 5% more battles.
`needs_a_centre` takes whether a grind is in front of the queue and uses "no damaging PP at all"
there.

⚠️ **And two of those are route facts rather than policy ones.** Pewter is the first mart on the
route that sells a Potion at all, so the early game carried nothing until it; and **Lt. Surge is
Electric into a Water starter**, which black-ed the run out twice in one gym until TM28 Dig moved from
the Seafoam Islands to before that fight. Ground is 2× on Electric and Dig's first turn is spent
underground.

⚠️ **A grind belongs outdoors.** Four levels are cheaper per battle inside Mt Moon than on Route 3 —
and a black-out on a cave floor warps the run to the Mt Moon Centre, whose traversal's next step is an
`enter_at` *between two of its own floors*, a warp `route_toward` cannot find from outside. The run
stalls there for good. Route 3 costs twice as many battles and every recovery works.

⚠️ **A Pokémon Centre is a building, and routing to a building is harder than routing to its town.**
`route_toward` now takes a transition off the current map's own `actions()` before asking the graph,
and the heal detour falls back to `Map::pokemon_center_town` — because the incremental graph can fail
to find a warp two hops away while the *connection* to the next town is sitting in `actions()`.
Measured: "no route from Route3 to PewterPokecenter to heal", on a grind whose only problem was that
it had walked east, after which the run fought on at zero PP until Struggle's recoil killed it.

### The scripted policy's detours, and what a new run forgets

⚠️ **Every branch of `DeterministicPolicy` that routes must be bounded, and the heal-return detour was
the one that was not.** `route_toward` reads the **incremental** world graph, whose nodes are keyed on the
entry the agent actually *landed* on — so a section reached any other way has no node, and `bfs_nodes`'
`SNAP_THRESHOLD` of 8 tiles will not reach it. The deployed run of **2026-08-28** blacked out in Mt Moon,
walked back in through B1F's (5,5), and fled a Zubat in the fossil chamber on 4 HP: its only two exits land
on B1F at (23,3) and (21,17), 20 and 28 tiles from the single observed node, so both resolve to dangling
targets and `pick_shortest_path_action` answered `None`. The branch returned that unconditionally — and,
being an early return **above** the `[policy] map=…` print, it went *silent*: no line, no counter, no
watchdog (the agent reaches a decision point every tick, so `GB_STUCK_TIMEOUT_SECS` never fires), the
emulator still running and the page still green. Hours, until a human looked. `MAX_HEAL_ROUTE_WAIT` is the
bound and it matches `MAX_GYM_ROUTE_WAIT`'s reasoning — a black-out warp leaves the map unsettled for a few
ticks, so waiting first is right and waiting for ever is not. ⚠️ **Handing back to the main queue is the
right give-up, not parking**: the queue is a *route*, so its next step walks out of the dungeon, which is
where the Pokémon Centre is. Fainting on the way is not a failure — the black-out heals the party and the
queue resumes. Guards: `a_heal_detour_that_cannot_route_hands_back_to_the_route` and
`a_heal_detour_that_is_moving_is_never_abandoned`, and ⚠️ **the second cannot be the first inverted** —
from inside Mt Moon no centre is an edge target of any observed node, so it returns `None` honestly and an
`if is_some()` around the assertions asserts nothing at all. It has to be Cerulean, where the centre is one
warp from the observed node.

⚠️ **`Policy::restart` resets a *run*, and the queue was only the half of it anyone had noticed.**
`POST /api/new-run` starts the cartridge over in a live process, so every field scoped to a run is now
about a game that no longer exists: `gym_beaten` would skip gyms the fresh save has not beaten,
`last_pokemon_center` and `heal_return` would send Red's bedroom detouring to Mt Moon, `train_slot` would
switch in a party slot that is not there. It survived only because the one deployed reset followed a
process wedged at step 0 with all of it still empty. The policy is rebuilt from its **seed** rather than
patched field by field, so a field added later is untainted by construction — which is the property a list
of assignments cannot promise, and the reason `seed` is stored at all.

⚠️ **"No cursor on disk" does not mean "a new game", and reading it that way destroys a run.** A run
started before `scripted-progress.json` existed has no file *and* a save in the middle of the game. The
rollout of 2026-08-28 resumed one standing in Victory Road, restarted the route at
`EnterMap { RedsHouse1F }`, and burned 745 polls failing to route out of a map the route knows nothing
about. The absence of the file cannot tell the two apart, so `resuming_in` takes the answer from the caller
(`Origin::Fresh`) and **parks** otherwise — the same argument as the changed-route branch beside it, which
was already right for exactly this reason.

### Closed loops the agent walks into

⚠️ **Every Gen 1 PC menu is a closed loop under A-only input, and `ReadingTextBox` presses B when
`PokemonApiTrait::in_pc_menu` says so.** Each leaves only on B, and A on its resting cursor picks the first entry,
which bounces off a refusal message straight back with the cursor untouched (`PCMainMenu` → Bill's PC →
`WITHDRAW` → `NoMonText` → `BillsPCMenu`). Nothing moves the cursor, so A never reaches `LOG OFF`. This wedged the
deployed run **permanently**, eight tiles from a fresh save; party size is irrelevant. Two traps in the detection:
⚠️ **the item PC sets no flag** — `TextScript_ItemStoragePC` (Red's bedroom) calls `PlayerPC` directly and leaves
`BIT_USING_GENERIC_PC` clear, so the screen is matched on `LOG OFF` as well; ⚠️ **and `LOG OFF` alone is not enough
either**, because the parent tree's submenus do not show one. `UsingPcBox`/`UsingItemPc` are excluded from
`assert_text_box_state` so they never reach `ReadingTextBox`; their *abort* paths do, which the same line fixes.

⚠️ **The START menu is six rows before the Pokédex and seven after it, so a cursor index does not mean the same row
in both.** `DrawStartMenu` omits POKéDEX until `EVENT_GOT_POKEDEX` and `.displayMenuItem` puts it back with an
`inc a`, so **index 2 is ITEM with the Pokédex and the player-name row without it** — `StartMenu_TrainerInfo`, which
`WaitForTextScrollButtonPress` leaves on A *or* B straight into `jp RedisplayStartMenu` with the cursor restored. A
closed loop under A, flashing the screen white twice a cycle. `start_menu_row` is the one place that knows.
⚠️ **The window is not a corner** — Oak's Parcel is delivered *before* the Pokédex, so every run passes through it,
and the deployed run spent 55 minutes wedged there. ⚠️ **And no test tier could have caught it**: `RandomPolicy`
implements only `pick_overworld_action`/`pick_battle_action`, so `soak` never issues a field move, while the leg
chain and `full_playthrough` reach those drivers long after the Pokédex. Pre-Pokédex they are reachable by an LLM
policy and nothing else — which is the general lesson, not a fact about this menu.

⚠️ **A TM or HM aimed at a Pokémon outside its learnset is the same loop, and `TeachingMove` has no exit from
it.** `CanLearnTM` tests a bitfield in the base-stats entry; a miss prints `MonCannotLearnMachineMoveText` and
then `jr .chooseMon` (`engine/items/item_effects.asm`) — back to the party menu, cursor untouched — so the
driver navigates to the same slot, presses A and is refused again. Its only completion is "the mon knows the
move", which never comes, so the attempt is 60 s of A-mashing ended by `DRIVER_ESCAPE_SILENCE`, after which the
policy asks for the identical teach. The deployed run of 2026-08-27 lived there. ⚠️ **Compatibility is knowable
before a button is pressed** and `pokemon::learnset` reads it out of the ROM rather than transcribing 151 × 55
bits; ⚠️ **`\1_TMNUM` is not the item id and the two run in opposite directions** — the HMs are TMNUM 51-55 at
item ids `$C4-$C8`, *below* the fifty TMs at `$C9-$FA`, and the flag index is `TMNUM - 1`. Three gates, the same
shape as Cut's: `tools::resolve_field_move` refuses the call in the turn, `agent.rs` refuses on the way into the
state, and `DeterministicPolicy::pick_field_move` skips the step so a scripted leg cannot re-issue one every
tick. ⚠️ **What the refusal says is the alternative, not the refusal** — "got no answer from the game for 60s"
reads as a malfunction and gives a model nothing to do differently, which is "it was interrupted" all over
again, so `learnset::teach_refusal` names who in the party *can* take it or says outright that nobody can.
⚠️ **And the proactive half was pointing at the wrong errand**: `prompt`'s `Blocked here:` line said "an HM to
be found and taught, and needs the CascadeBadge" whatever the run was holding, so a party carrying HM01 with the
badge won and nothing able to learn Cut was told to go and find HM01. It names the half actually missing now.
`teaching_an_hm_to_a_mon_that_cannot_learn_it_does_not_wedge` is the guard, and ⚠️ **it runs on a hand-rolled
policy rather than `DeterministicPolicy`**, which would skip the teach before the agent ever saw it and pass
without touching the thing under test.

⚠️ **A bag item the game will not use is the same loop again, and `UsingFieldItem` had no exit from it
either.** `ItemUsePtrTable` (`engine/items/item_effects.asm`) sends most key items to `UnusableItem`, which is
`jp ItemUseNotTime`: "This isn't the time to use that!" and back to the bag list with the cursor untouched.
This driver's only completion is `game_mode == Overworld`, which a refusal never reaches, so it was 60 s of
A-mashing ended by `DRIVER_ESCAPE_SILENCE`. The deployed run of **2026-08-27** alternated a
`use_item HelixFossil` with talking to the Mt Moon Rocket whose flavour line is "if you find a fossil, give it
to me", four minutes of wall clock at a time. Two gates and a net, and all three are needed:
- **`item_use::field_use_refusal`**, read out of `ItemUsePtrTable` rather than transcribed, refuses the known
  ones in `tools::resolve_field_move` (no round trip) and again in `agent.rs` on the way into the state.
  ⚠️ **Reading the ROM is not pedantry here**: the Card Key, the Poké Flute and the Coin Case are usable while
  the Silph Scope and the Lift Key beside them are not, and item ids `$15`/`$16` are the Safari Zone's BAIT and
  ROCK as well as the first two badges, so "the badges are unusable" would have been wrong.
- ⚠️ **The on-screen net in the driver, because a refusal can be *contextual* and no table predicts one.**
  `ItemUseEscapeRope` outside `EscapeRopeTilesets` and `ItemUseBicycle` indoors are real effects that end at the
  same `ItemUseNotTime`. It reuses `shows_battle_refusal` (one list, not two) and **latches
  `BACKING_OUT_TICKS` of B** rather than pressing one or dropping to `Idle`: one B lands back on the bag list
  where the driver's own navigation presses A again, and ⚠️ **`Idle` with the bag still open costs 33 s** of
  the generic reader getting out and emits the whole screen it walked through as one `TextBox` — which is what
  the deployed run called an "unrelated TM34/bag menu prompt". Measured 60 s → 5.4 s.
`using_an_item_the_game_will_not_use_does_not_wedge` and
`an_item_the_map_refuses_backs_out_rather_than_mashing_for_a_minute` are the guards, both on hand-rolled
policies rather than `DeterministicPolicy` (which only ever reaches for the Poké Flute), both asserting that
**decisions keep coming** and that the *words* arrive. ⚠️ **The target has to be reachable** or the driver
reports "can't reach the field-item target" on its own and the test passes with the gate removed.

⚠️ **A mart menu nobody has claimed yet is the same loop, and it is the first one that *spends
money*.** `drives_its_own_menus` is what keeps the generic text reader off a driver's menus and it
keys on the **state** — so a shop that is open while the agent is still `Idle` is not covered.
`assert_pokemart_state` used to enter `PokemartShopping` only when the policy *answered* what to
buy, which every scripted policy does on its first poll (the trait default is `Some(None)`) and
`LlmPolicy` does not do for the whole time the model is thinking. In that window the reader pressed
A through the shop: BUY, the first row of the stock list, quantity 1, confirm, YES, round again.
`PokemartState::AwaitingPolicy` is the fix — the same shape `BattleState::AwaitingPolicy` has always
had — entered **on sight of the menu**, pressing nothing, polling through `poll_policy` until an
answer comes. ⚠️ **And the policy is asked in the same tick the state is entered, which is frame
timing rather than tidiness.** The first version let the driver's next tick do the asking, which
costs a scripted policy (they answer on the first poll) **one agent tick per shop** — and six shops
into `full_playthrough` that is a different RNG line: the run failed at 512/516 steps waiting for a
wild Machop on Victory Road 1F that never appeared, with all six mart visits having succeeded on
attempt 1. Nothing else in the suite could see it; the default tier and the whole leg chain were
green. `ask_mart_policy` is one helper called from both ticks, because two copies are two places to
forget the `affordable` trim. ⚠️ **The `affordable` trim is not implicated and looking there first wastes an hour**:
`item_price(PokeBall)` reads 200 off the ROM correctly and the trim would have quit the shop
immediately, but the driver holding the trim was never running. ⚠️ **The deployed symptom is the
harmless face of it.** On the ¥137 the run actually had, it can afford nothing and merely loops on
"You don't have enough money."; seeded with ¥1200 the same fixture ends
`money now 0, bag [(TownMap, 1), (Potion, 1), (PokeBall, 6)]`. Guards:
`a_shop_left_waiting_on_the_policy_is_not_mashed_through` (¥137, counts refusals) and
`a_shop_is_not_raided_while_the_policy_is_still_thinking` (¥1200, asserts the wallet) — ⚠️ **and the
poor one cannot see what the rich one sees**, since a wallet too thin to buy anything is flat either
way. Two traps in writing them: ⚠️ **withhold the answer for ever rather than merely delaying it**
(the first draft used `SlowPolicy` at 200 ticks against a 30 s run and measured the *legitimate*
purchase landing at tick 560, which is the driver working); and ⚠️ **count refusals off the screen
on a rising edge, not out of `AgentEvent::TextBox`** — the box never closes on a run that short, so
an event-counting version reported zero on the very run that produced sixteen.

### Screen versus RAM

⚠️ **The screen the agent reads lags the game's own tilemap, and its three horizontal bands can disagree.**
`AutoBgMapTransfer` (`pokered/home/vcopy.asm`) copies `wTileMap` into VRAM one third per V-blank, rotating
top/middle/bottom, so a menu takes up to three frames (~50 ms, two and a half agent ticks) to appear and the band
it replaces still shows the old one. RAM is therefore *ahead* of the screen at every menu transition, and
`wTopMenuItemX/Y` — written by whoever is about to call `HandleMenuInput` — is the authority on which menu is live.

⚠️ **That cost a paid LLM turn per battle turn for months.** `WaitingForMenu` resolves the main battle menu from the
text (`FIGHT` … `RUN`) as well as from geometry, because after an item turn the geometry is genuinely ambiguous. But
the move list is drawn at `hlcoord 4, 12` over the battle menu's own box, so for a frame or two after
`MoveSelectionMenu` writes `(5, 12)` the bottom band still reads `FIGHT PKMN ITEM RUN` — the text test fired,
concluded the turn was the policy's again, and **threw away the move `Navigating` had just highlighted**.
`BattleState::confirming` is the fix: a bounded window after `Navigating` hands over in which the geometry is
believed and the text test is skipped. `a_battle_turn_is_decided_once_rather_than_twice` guards it, at two policy
latencies because it is a race the policy's own speed moves. ⚠️ **Counting decision points is not enough** — that
passes if the agent stops fighting, so it asserts one poll *per landed move*, read off the enemy's HP.

⚠️ **It re-timed the battles, exactly as `with_original_battle_timing` warns, and `can_reach_lavender` was the
casualty** — a leg pinned to one RNG line. It ran Razor Leaf dry crossing Rock Tunnel and failed at *every* window
length, so it is the shift and not the number. `back-in-cerulean.bin` was regenerated (`can_return_to_cerulean`
under `--features regen-fixtures`) and the chain is green from it — one fixture, not the cascade.

### Prose the model and the page read

⚠️ **`impl Display for AgentEvent` is a UI contract, not debugging output.** `host.rs` does `format!("{event}")`
straight into `UiEventBody::Agent { text }` and the page prints it verbatim, so a `{:?}` puts
`Fight { slot: 1, battle_move: PokemonMove { name: Growl, pp: 40 } }` on screen for every turn of every battle —
which it did, for months. It is also what `llm::prompt::describe_event` sends the *model*.
`a_battle_turn_reads_as_a_sentence` is the only thing that looks at the prose.

⚠️ `BattleActionStarted` carries the acting Pokémon's nickname **and the opponent's species**, because nothing
downstream can look either up: the host formats events off the emulator thread. ⚠️ The opponent is read at the
**decision point**, not at `BattleStarted`: `InitWildBattle` sets `wIsInBattle` *before* `LoadEnemyMonData` and a
trainer's lead is not loaded until they send it out, so reading it as the battle starts reports the previous battle.

⚠️ **An event that names no target is the same bug one level down.** Three quarters of a random run's log is the
agent walking somewhere, and every one of those lines is a `MetaTile`'s `Display` — which was `strum`'s derive, so
the page said `→ heading for Warp` and never which warp, which map or who. Each variant names its target now (`the
warp to OaksLab`, `the way into Route1`, `Mom`), as a **noun phrase**, because the same string has to read as English
in four frames: the three `AgentEvent` sentences and `NoRoute`'s "there is no route to {tile}". ⚠️ **`MetaTile::kind`
is the other half and must stay the variant name** — `llm::tools::overworld_id` mints `"PalletTown:5,6:Warp"` out of
it, the model quotes it back, it is re-resolved by string equality, and `Conversation.tsx` prints it verbatim, so the
prose is free to be reworded only because the key is a different function. `a_walk_says_where_it_is_going` and
`an_id_keeps_the_variant_name_the_prose_left_behind` hold them apart.

⚠️ **The word "sprite" appears nowhere a model reads, and `MetaTile::id_kind` enforces that.** It is the emulator's
vocabulary for a moving object on a screen and the model has no screen: it read as jargon and was the same word for
Professor Oak and for a boulder. So an id ends in the **person's name** — `OaksLab:2,2:Pokedex1`. ⚠️ **Spaces are
stripped**, because several `MapSprite` names have them ("Middle Aged Woman") and an id resolved by string equality
must not be whitespace-sensitive. ⚠️ **`MapView`'s key is `people`, spelled the id's way** (`Pokedex1`) and built by
calling `id_kind` rather than a second function that agrees with it. `kind()` still returns the variant name, for the
sort key and the tests.

### What the agent reports

⚠️ **The screen is a *page being typed*, and `PokemonTextReader` used to treat it as a stream to splice.** Frames of
one page are prefixes of one another; the page then clears and the next is typed. The old accumulator instead
appended `screen[longest_suffix_of_buffer_that_is_a_prefix_of_screen ..]` — which works while frames arrive in order
and fails **permanently** the first time one does not, because the re-appended screen makes the tail match again next
frame and not the frame after. A sawtooth that grows quadratically: **1456 bytes in one text box**, deployed. Three
rules now, and the second and third are the ones that look optional: a frame in a prefix relation either way extends
the page; anything else is spliced on the overlap **against the page, never against the buffer** (overlap against the
whole history is the old algorithm again); and ⚠️ **a blank frame commits nothing** — it is far more often a battle
animation mid-redraw than a page break, and committing on it was half the sawtooth. ⚠️ **One mismatching read is not a
page break either**, because `AutoBgMapTransfer` tears a two-line box across a frame
(`"Our POKéMON's an outsider, outsider, so it's"`); `MISMATCHES_BEFORE_PAGE_BREAK` is 2, the smallest number that
outlives a tear. ⚠️ **`take` reports the open page as well as the committed ones**, or every blocker's last page goes
on the floor — which is the bug `flush_text_reader` exists to fix, one level down.

⚠️ **`commit_page` joins pages verbatim, and both attempts to tidy up there deleted real text.** Splicing a page onto
the buffer's tail on their overlap, and dropping a page already contained in it, are the obvious cleanup for a tear
committed mid-scroll — and the cartridge repeats itself constantly (`"Ember used RAGE!"` once per turn of a five-turn
battle, `"Critical hit!"`, `"Got away safely!"`), so any lookback long enough to catch a tear catches those too.
Measured on the deployed states it took the worst box from 654 bytes to 398 **by deleting four turns of a battle**.
Deduplication belongs only where a frame is compared with the page it is redrawing.

**The numbers, and the recipe for reproducing any of this**: `issues/turn-{170,206,440}/state.gbst` from the deployed
run, driven by `RandomPolicy::seeded(1..=3)` for 600 s emulated, collecting `AgentEvent::TextBox`. Worst box
**1456 → 654 bytes**, and every box still over 200 is a real one — a Metapod HARDEN stalemate, the nurse's dialogue
with `HEAL CANCEL` genuinely on screen, the museum's money window. No sawtooth at any seed.

⚠️ **`PPU::tile_coordinates` answered with tiles that are not on the screen, and that is what fed the reader the
contamination.** It walked all 32×32 of both tile maps at raw map coordinates; a tile map is 256×256 pixels and the
screen shows 160×144 of it. Pokémon Red leaves a stale copy of the enemy HUD below the visible rows of the **window**
map, which sorts *after* the message box, so every frame of a battle message came back as
`"… Enemy GEODUDE's hurt by the burn! GEODUDE 10"` — the same nine invisible characters welded onto the end of every
frame, so no frame was ever a prefix of the next. It now walks the **20×18 screen** and returns screen coordinates
(which is what `MESSAGE_BOX_MIN_Y` always assumed). ⚠️ **The window is decided per tile, not once per frame**: the old
code read "window enabled" as "the window is everything", and Red parks the window at **WY=144, entirely off-screen**,
for the whole of the overworld while its map still holds the last screen drawn through it.

⚠️ **Which reader is a fact about the *game*, not about `AgentState`.** `assert_text_box_state` picked
`message_box_only` on `matches!(self.state, AgentState::Battle(_))` — but its own other arm drops to `Idle` when a box
closes, so the *first* box of a battle got the message-box reader and every one after it got the full-screen one, HUDs
and all. It reads `wIsInBattle` (`BattleStateReader::read_battle_state`) now, which is the condition that actually says
a HUD is on screen.

⚠️ **A pickup the game refuses is the same three events as one that works.** Every item on the floor is a sprite: walk
up, text box, back to the overworld, `✓ talked to Charmander Poke Ball` either way. The difference is that a real
pickup `HideObject`s the sprite, so `PokemonAgent::check_pending_pickup` asks the map again once the overworld is back
and emits `AgentEvent::OverworldPickupFailed` when it is still there. ⚠️ **Armed on the interaction and answered later,
never both at once**: the pickup runs as a script (box, `GiveItem`, `HideObject`), so testing while the box is open
calls every success a failure. ⚠️ **The latch clears whatever the answer is**, or one refusal is re-reported on every
overworld tick for the rest of the run. ⚠️ **`PictureId::PokeBall`, never the name** — the same test `llm::tools` verbs
the menu row with, which is what keeps "pick up the Potion" and "nothing was picked up" talking about one set. The
deployed run of 2026-08-27 spent turns 7 to 24 on Oak's starter balls and filed a `report_issue`; ⚠️ **the case it is
really for is a full bag**, which refuses every pickup in the game in exactly this shape and is otherwise reported
nowhere. Guards: `a_pickup_the_game_refuses_says_the_item_is_still_there` and
`a_pickup_that_works_reports_no_failure` — ⚠️ **both, because the first passes on its own if the check fires on
everything**, and ⚠️ the refusing fixture cannot use `step_until_exhausted` (`PolicyStep::Interact` pops when the item
reaches the bag, which is the thing that never happens).

⚠️ **A textbox is detected before its characters are drawn**, so the reader emits a stream of empty ones — on the
deployed run they were most of the log. `PokemonAgent::event` drops them, and it is the funnel *every* event goes
through (including those collected into `update`'s local `new_events`), so the transcript is clean as well as the page.

⚠️ **Reading a text box and *reporting* it are two moments, and treating them as one lost the most important text
boxes in the game.** The buffer used to be emitted only in `assert_text_box_state`'s "the box closed" arm, which
fires while the agent is still `ReadingTextBox` — but `assert_script_state` runs **before** it in `update` and swaps
the state out for `RunningScript`, so a box the game follows with a script never reached that arm. ⚠️ **That is the
shape of every blocker in Pokémon Red**: print a message, then `StartSimulatingJoypadStates` to shove the player back
a tile. Measured: a landed conversation was followed by a `TextBox` event **31 times out of 38**, an aborted walk
**2 of 28**. The run walked into the Route 22 gate, heard nothing, talked to the guard five times and filed a
`report_issue`; the words it could not see were "You don't have the BOULDERBADGE yet!".
`PokemonAgent::flush_text_reader` is the fix and `PokemonTextReader::take` what it drains with. ⚠️ **It hangs off the
two places that assign `self.state`** — `set_state` and `backup_current_state` — so the rule is structural rather than
a list of call sites, and a battle or a mart stealing the state is covered by the same line. ⚠️ **`take` clears rather
than replaces**, because `backup_current_state` keeps the state it flushed and a ledge jump restores it. The pair is
`a_guard_who_turns_you_back_is_quoted_rather_than_swallowed` and `talking_to_that_guard_reports_what_he_said`, on
`route22-gate.bin`, and ⚠️ **what they assert is that the *words* arrive** — an empty `TextBox` is dropped by `event`,
so counting events would pass on the stream of empty ones the bug already produced.

⚠️ **`### On screen` is not a second chance at this, and it looks like one.** `observe::screen_text` reads the tile map
as it stands, so through a conversation it returns a rolling fragment (`"Onl"`, `"Only truly skilled trainers are"`,
`""`, …) and by the time the abort has resolved into an overworld decision it is `None`. Only the reader accumulates
across pages, so the `TextBox` event is the only complete record of anything the game said.

⚠️ **Talking to someone is that action succeeding, and the text box is the only signal it gets.** The route to a sprite
ends by facing it and pressing A, and it is re-derived every tick — so once the player is standing in front of the
sprite that route is `[A]` for ever and the "the route ran out" branch that completes an ordinary walk is never
reached. Hence `AgentEvent::OverworldInteractionCompleted`; before it, every successful conversation was reported as
"✗ gave up on Mom: it was interrupted".

⚠️ **An abort also says *where* the walk stopped.** `OverworldActionAborted` carries `at`, so the line is "✗ gave up on
the way into Route2 at (19, 11): the game stopped you to say something". The deployed run produced that abort **143
times** — the Viridian old man blocks the north exit until Oak's Parcel is delivered — and concluded "the choose_action
pathfinding keeps failing". ⚠️ **Nothing counts those aborts, refuses a repeatedly-failing target, or drops one from
the menu**: noticing that the same square twenty times means a blocked route is deliberately the model's job, and a
menu that silently withheld a reachable-looking action would be a worse lie. ⚠️ **`at` is in the *expanded* coordinate
space** the ids, the map picture and every `Location:` line use, never `raw_player_coords`;
`MetaTileMap::player_position` is where it comes from, and the one abort that reports `None` on purpose is `WrongMap`.

Three traps in detecting a landed conversation, each paid for separately:

- ⚠️ **It is what the player is *facing*, not what it set out for.** A script can open a box mid-walk — the rival's,
  two tiles short of the aide in Oak's lab — and "my destination was a sprite" calls that a conversation the run never
  had. `a_script_that_interrupts_a_walk_is_still_an_abort`.
- ⚠️ **A PC is not in `meta_tiles`**, so the tile in front of a player using one reads as `Obstacle`. It is a hidden
  event, indistinguishable from the wall it is drawn on, which is why `pc_locations_for` is a transcribed table.
- ⚠️ **"Facing" has to mean what the game means, which reaches *over* a counter.** Gen 1 talks through
  `wTilesetTalkingOverTiles` (`MetaTile::Counter`) — a nurse, a mart clerk, every gym receptionist. `actions()` routes
  to the far side of the desk, so the tile in front is the counter and never the person; matching it literally reported
  every heal in every Pokémon Centre as "✗ gave up on Nurse". `MetaTileMap::interaction_in_front` is the one that hops;
  ⚠️ **`tile_in_front` must not**, because `cut` and the surf mount are about the literal tile.

⚠️ **And the *word* was the other half: "it was interrupted" reads as a malfunction, and being stopped is the game
working.** The deployed run walked at the Viridian Gym door with no badges, read that reason with "The GYM's doors are
locked..." on the line below, and filed a `report_issue`. Two changes, two audiences:
`OverworldActionAbortedReason::Textbox` reads **"the game stopped you to say something"**, pointing at the text box
that follows rather than describing the walk's failure; and `SYSTEM_PROMPT` says once, under "The game is not broken",
that being stopped is how the game tells you something. ⚠️ **Not "something was said"**, which buries the fact the
model needs. ⚠️ **And no `Blocked here:` line was added for it**, unlike the Cut/Surf case: the cartridge already says
the doors are locked, in a box quoted into the very next turn.

### The page's copy of all this

⚠️ **What the page shows and what the model is told are two different lists, and the split is on the client.**
`useEventStream`'s `fold` drops `text_box` and `overworld_interaction_completed` — the dialogue is already on screen in
the game's own font, and "✓ talked to Mom" says less than the line of Mom's that follows. Both still go to the model
and to `transcript.jsonl`. ⚠️ **Do not "simplify" this by filtering at the publish**: `run::transcript` writes what is
published, so that deletes the dialogue from `/api/history` and from the archived record, which is the one copy nothing
can rebuild.

⚠️ **A tool call and its result are two events and one row.** `UiEventBody::ToolCall` carries the endpoint's own call
`id`; `ToolResult` is paired to it by that id in `attachResult`. ⚠️ **Paired on the id, never on position or name** — a
turn can call several tools in one batch, and two `read_party` calls are indistinguishable by name. ⚠️ **Neither row
may go through `push`**, or an answer attaches to the wrong call.

⚠️ **A tool's *picture* is referenced, never carried.** A map render is a couple of hundred kilobytes and every
published event is also a line of `transcript.jsonl`. `ToolResult.image` is a flag; the bytes live in a 16-entry ring in
`Published` keyed by the **seq of the event that announced them** (so the publish must happen before the `put`), and
`/api/tool-image/{seq}/image.png` serves them. ⚠️ **A 404 there is the expected answer** — anything older than the last
handful is gone and the page shows the caption alone. `MAX_TOOL_RESULT` truncates the text server-side for the same
reason: a truncation the client applies has already been broadcast and written to disk. ⚠️ It sizes the *broadcast copy*
only — the model is always sent all of a tool result.

⚠️ **The status heartbeat is sent on change, not on a timer.** Sampled at `GB_STATUS_HZ` and published only when it says
something the last one did not, with a 2 s keepalive. At the original 10 Hz unconditional it measured **49.7 kbit/s per
viewer** — six times the idle video feed, nine of ten payloads byte-identical; it is now 5.2. Two consequences:
`StatusSnapshot` compares with `says_the_same_as`, which excludes the clocks and `frame_seq` (a derived `PartialEq`
would never match and the suppression would silently never fire), and `/api/events` **opens with the latest heartbeat**
(`join_events`, subscribe-then-read, the same handshake as the video keyframe) or a page opened during a quiet stretch
shows an empty panel. ⚠️ Anything *added* to the snapshot must be added to `says_the_same_as`'s destructuring **and
compared there**: the pattern is exhaustive, so a new field is a compile error, but binding it without comparing it is
not. `dropped_ms` is the counter-example to the clock exclusion: it stands still on a host that is keeping up, so it
forces no heartbeat in a healthy run and the moment it moves is worth telling a viewer about.

⚠️ **A lifetime average is not a rate, and `emulated_ms / wall_ms` is the average.** The page's speed line was that ratio
and read below 1× for ever on a host running at full speed. Two mechanisms, both permanent, since an average can only
converge on the truth from below: **`MAX_CATCHUP` drops emulated time on purpose**, so any iteration overrunning 250 ms
is subtracted from the numerator for the rest of the run (a **14.85 s** startup debt was still there five minutes in,
against an instantaneous speed of exactly 1×); and ⚠️ **`wall_ms` was measured from a clock stamped at construction while
`emulated` is zeroed by `start_new_run`**, so a run started in a long-lived process reported its first seconds against
those hours. `progress()` had always used the correctly-paired `run_started`; the heartbeat now does too.

⚠️ **The tell that it was accounting and not the emulator is that the deficit was *constant*.** Two samples 90 s apart put
`wall_ms − emulated_ms` at 14908 and 14845 ms — shrinking by 0.0704 ms/s, which is `to_duration` truncating 953.674 ns to
953, so **1.0007× is the ceiling and a healthy host reads exactly that**. A genuinely slow host shows a *growing* gap. The
page derives speed from consecutive heartbeats (`sampleSpeed`, a 500 ms window because at `GB_STATUS_HZ`'s default two
samples can be 100 ms apart). ⚠️ **A park needs no case in that and must not be given one**: the host stops the emulator
*and* subtracts the wait from `wall_ms`, so both counters freeze and the last live reading is held under the PAUSED plate
— where a `dw > 0` guard would report `0.00×`. ⚠️ **`RunProgress::wall_ms` was the same bug's other half**: `paused_total`
was subtracted by the heartbeat and not by `progress()`, which is what `meta.json` and the ledger record, so a run parked
overnight wrote the whole night down as play. Both paths subtract it now.

⚠️ **Send-on-change needs a cell per thing sent, and the plan was the one without.** `join_events` opened with the
heartbeat alone, so the plan — published only when it *changes* — was never replayed to a page that had just loaded.
`/api/history` was no better: it keeps `MAX_BACKLOG` (2000) events and a reasoning model publishes one per streamed token,
so the last `Plan` falls off within minutes. The symptom was neither, because `PlanPanel` renders nothing for an empty
list, so it read as a styling bug. `Published::latest_plan` is the fix, on the same handshake, and it works because a
`Plan` event is *absolutely* stated. ⚠️ **Anything else that becomes send-on-change belongs in `join_events` too.**

⚠️ **Both keepalives are load-bearing on the client, because a dead connection is otherwise indistinguishable from a quiet
one.** The page's retry loops were error-driven — `onerror` on the `EventSource`, `catch` on the video `fetch` — and both
are right about every case that *produces* an error. A network going away produces none: no FIN, no RST, and a stream we
only read from sends nothing that could time out, so the page froze on its last frame **still showing the green live pill**
(measured at 75 s, not going to recover). `STALE_MS` (`web/src/api.ts`, 4× the server's 2 s `KEEP_ALIVE`) is the fix:
silence, not an error, is the signal. ⚠️ **Not the SSE keep-alive** — that is a comment line `EventSource` hands to no
callback, so a watchdog on it starves and reconnects every 8 s for ever; the status heartbeat is the one that arrives.
⚠️ **On the video side, not the messages `readVideoStream` yields** — its keepalive is a zero-length message that yields
nothing, so a watchdog fed from messages would fire on a screen that is merely not moving; it is fed from the inflated
chunks. ⚠️ And `subscribeVideo` needs **one `AbortController` per attempt**, chained to the caller's: aborting the caller's
own signal is how that loop is told the component unmounted. Reproduce with `kill -STOP` — `docker stop` is a clean close
and exercises the path that already worked.

⚠️ **A reconnect of `/api/events` is also a reload of the transcript**, because a fresh connection opens with the latest
heartbeat and plan and nothing else. `/api/history` used to be fetched once at mount, so every reconnect resumed a log with
a hole nothing would fill and a dormant tab came back showing the hour-old log with the live pill green. Now `subscribe`
reports every `onopen` (the browser's own transparent retries included) and the hook resets everything the old connection
folded — entries, pending queue, plan, usage, speed anchor — and refetches `/api/history`. Three traps. ⚠️ **The reset has
to be inside `onopen`, before `alive`**, so it lands ahead of the opening heartbeat and plan rather than throwing them
away. ⚠️ **The backfill is generation-guarded**: a fetch started by the old connection can resolve after the new one has
reset the page. ⚠️ **A hidden tab is resynced on return whether or not its socket died** (`visibilitychange` after more
than `STALE_MS`, `pageshow` with `persisted`): a backgrounded tab gets no animation frames, so `pending` overflows on a
healthy connection, and the watchdog's own timer is throttled with everything else. A short tab flip keeps the connection.
This is deliberately a full reload and not a `since=` merge: `read_since` reads from the tail so a bare backlog is cheap,
and a merge of a *folded* log across a gap is where the bugs would live.

⚠️ **Every `UiEvent` carries `at`, a Unix-millisecond wall-clock stamp, and it is the only clock the page can date a line
by.** `wall_ms` and `emulated_ms` are elapsed times *since this process started*, and the browser cannot supply one either,
because `/api/history` replays a backlog that may be hours old. It is stamped in `publish_event`/`publish_status` and lands
in `transcript.jsonl` too. ⚠️ **The SPA's copy is optional and must stay optional** — the runs on disk predate the field.
⚠️ And `useEventStream`'s `signature` excludes it, for the reason it excludes `seq`: it differs on every event, so leaving
it in would stop identical rows ever collapsing into a `×3`.


## The fishing row in the action menu

`MetaTile::Fish { rod }` is an action, never a tile: `meta_tiles` never holds one, `player_tile()`
never equals one, and the two exhaustive matches over `MetaTile` that draw maps (`llm::map_image`'s
tint table and `MetaTileMap`'s own `Display`) treat it as ordinary ground. What mints it is
`actions()` section 6, on three conditions that are each a row the cartridge would otherwise refuse:

- a rod in the bag (`MetaTileMap::best_rod`, set by `game_state()` from the live bag, exactly as
  `can_surf` and `can_cut` are — the map builder has no bag access),
- a tileset in `WaterTilesets` (`fishing::tileset_holds_water`), which `FishingInit` checks *before*
  it looks at the tile in front, so a map that fails it answers every cast with "Not the time to use
  that!",
- water with a reachable `Empty` neighbour (`fishing::nearest_castable_water`, shared with
  `PolicyStep::Fish`'s own `pick` so the row and the cast cannot disagree about which shore).

⚠️ **The row's route ends facing the water and carries no `A`.** Pressing A at water does nothing; the
cast is a bag chain. The hand-off is in `AgentState::OverworldMovement`'s **empty-route arm** — the
same arm that would otherwise publish `OverworldActionCompleted` — which enters `AgentState::Fishing`
instead. Putting it there rather than in a policy is what makes every policy fish: scripted, random
and model-driven runs all take the row the same way.

⚠️ **The rod is re-resolved from the live bag at the hand-off**, not taken from the row. A row is
minted from a `GameState` and acted on a tick or more later, and `Rod::best_in_bag` is cheap.

⚠️ **Always the best rod, and that is not a preference.** `ItemUseOldRod` hard-codes a lv5 Magikarp
and the Good Rod's table is two lv10 mons, while the Super Rod reads the map's own group — there is no
map and no goal on which an earlier rod catches something a later one cannot, so there is nothing for
a policy to choose between and the row does not offer the choice.

⚠️ **It is not a grind engine, whatever the bite rate suggests** — see the ⚠️ on
`PolicyStep::gauntlet_grind_steps`, which carries the measurement. A cast is 8.46 s of cartridge time
against a step's 16 frames, so 50% bites a cast still lands an encounter behind walking's 10/256.

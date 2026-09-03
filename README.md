# gb

A Game Boy emulator written in Rust, repurposed as a platform for an LLM to play Pokémon Red —
entirely through text, with no screenshots required.

The emulator half is a real one: DMG and Game Boy Color, full CPU, PPU, APU, timer, DMA, interrupt
and joypad emulation, accurate enough to pass the standard hardware-compatibility test ROMs. The
other half reads the game's own memory — party, bag, map, battle state, on-screen text — using
symbols lifted from the [pokered](https://github.com/pret/pokered) disassembly, and drives the game
by synthesising joypad input. What the model sees is a description of where it is and what it can
do; what it sends back is an action, not a button press.

It runs headless, serves its own web UI, and keeps a run going across restarts.

```
docker run -d -p 8080:8080 -v gb-runs:/runs \
  -e OPENAI_API_KEY=sk-… -e GB_MODEL=gpt-5 ghcr.io/axle-h/gb:latest
```

Then open <http://localhost:8080> and watch it play.

## What works

**The emulator.** Blargg's `cpu_instrs` and `dmg_sound` (including both combined suite ROMs) and
`instr_timing`; the `dmg-acid2` and `cgb-acid2` PPU tests. Six memory bank controllers — `RomOnly`,
MBC1, MBC2, MBC3, MBC5, HuC1 — with the MBC3 real-time clock; 27 of mooneye's 28 MBC test ROMs pass,
the exception being MBC1 multicart, which is not implemented. A mapper `gb` cannot emulate fails
with a typed error rather than quietly running as something else.

**Game Boy Color**, as a first-class model rather than a coat of paint: VRAM/WRAM banking, palette
RAM, BG map attributes, OAM-index sprite priority, KEY1 double speed, HDMA/GDMA. A DMG-only
cartridge on a CGB gets compatibility mode including the boot ROM's title-derived palette, which is
why Pokémon Red comes out red-tinted here exactly as it does on real hardware.

**The game.** The agent layer can play Pokémon Red from a fresh save all the way to the credits,
because the emulator runs at roughly 50× real time with the agent on top of it: `full_playthrough`
reaches all eight badges in about seven minutes of wall clock, and `hall_of_fame_playthrough` carries
the same run on through Victory Road and the Elite Four in about twenty-six — most of that difference
being the ~840 wild battles it grinds to make the gauntlet a certainty rather than a coin flip.

**The LLM layer** drives that same agent over any OpenAI-compatible API. Its end-to-end tests run
against a mock server — a whole playthrough's worth of turns, cancellation and compaction included —
so what is proven here is the plumbing rather than any particular model's ability to actually finish
the game.

## Quick start

The container is the shortest path and needs nothing installed — see above. To build it yourself:

```shell
git clone --recursive https://github.com/axle-h/gb.git && cd gb
```

You need a Rust toolchain, [rgbds](https://rgbds.gbdev.io) ≥ 1.0.0 to assemble the cartridge, Node
with pnpm for the browser UI, and SDL2 if you want the desktop window.

```shell
# 1. The cartridge. `pokered/pokered.gbc` is embedded into the binary at compile time and
#    `pokered/pokered.sym` is parsed by build.rs, and neither is in git.
make -C pokered pokered.gbc

# 2. The browser UI. `web/dist` is baked into the binary, so this comes before cargo.
cd web && pnpm install && pnpm run build && cd ..

# 3. The binary.
cargo build --release
```

Then pick a way to run it:

```shell
# The web UI, played at random — no API key, no spend. http://localhost:8080
cargo run --release -- serve --policy random

# The web UI, playing the scripted route the full playthrough test plays, at 1x. Also free.
cargo run --release -- serve --policy deterministic --new-run

# The web UI, played by a model.
OPENAI_API_KEY=sk-… GB_MODEL=gpt-5 cargo run --release -- serve

# The SDL desktop window, with the game driven from your keyboard and the policy from stdin.
cargo run --release
```

`gb serve` **resumes** by default: the newest run under `$GB_RUN_DIR` (`./runs`) is continued in
place, plan and all. `--new-run` starts the game over in a directory of its own — or, on something
already running, opening `/reset-game` does the same thing without a restart (see below).

A new game names its trainer after whoever is about to play it, in the seven characters Gen 1 allows:
`AI` for any model, `HUMAN` at the desktop, something drawn from a list under `--policy random`. A
resume keeps the name it already has, because by then the game has printed it in a dozen places.

The LLM name used to be `GB_MODEL` shortened to fit, and it was wrong more often than it was right.
Seven characters cannot hold a model id, so every name was a guess at which half of one mattered, and
the guess kept producing models that do not exist — `openai/gpt-5.4-nano` came out `GPT54`. It was
also a lossy second copy of something already recorded exactly: `meta.json` and the hall-of-fame
ledger both carry the full id, and the trainer card could disagree with them, because the name is
written once into the save and `GB_MODEL` can change under a restart. So the save says `AI` and the
model id stays where it is unambiguous. The name and the trainer ID are both on the status panel,
beside the model the process is currently configured with.

## How the model plays

`PokemonAgent` advances the emulator 20 ms at a time and works out, from the game's memory, what
kind of decision is on the table — an overworld move, a battle turn, a nickname, a menu. It then
asks a `Policy`. The trait is non-blocking: every method returns `Option`, so the emulator keeps
running while the model thinks, which is the property everything else here is built around.

```rust
fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction>;
fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction>;
fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>>;
```

`LlmPolicy` is the interesting implementation. It turns each decision into a conversation turn with
a tool catalogue scoped to that kind of decision — a battle turn is offered no map, a naming screen
is offered almost nothing — and one terminal tool that commits to the action. The turn itself is
built to make reading unnecessary: the location, the party, the money, the badges, what is on screen
and the menu of what can be done are all in the request already, so most turns should need no read
at all.

One turn does not have to be one action. `choose_action` takes a `then`: up to three more ids from
the same menu, carried out in order without the model being asked again. Healing at a Pokémon Centre
is the case it was written for — talk to the Nurse, then take the door out — and it is two requests
otherwise, on something a player does a dozen times a run. A chain is a sequence of independent
decisions rather than a route the agent commits to: each id is resolved against the live game when
its turn comes round, exactly as the first one is, so anything that stops one stops the rest. Being
stopped is how this game says almost everything — a guard, a locked door, an errand — and carrying on
past that is the loop the whole agent exists to avoid, so a chain that ends early says where it got
to and hands the decision back.

The other half of the same saving is `resume_after_battle`. A wild Pokémon or a trainer interrupting
a walk says nothing about the walk: the battle ends by itself, the world is where it was, and the
walk was going to be re-issued word for word. So an action asked for with that flag is taken up again
once the battle is over rather than coming back to be chosen a second time. It is opt-in and only
ever a battle — a text box is the game talking, and resuming through one is exactly the wedge above —
and it gives up after five, because the model is answering battle turns throughout but is locked out
of the overworld, which is where "half the party has fainted, go and heal instead" lives.

The bigger saving is that the model can decide not to be asked at all. Battles are where a run's
tokens go and where its play is worst: one deployed run made **204 battle decisions of which 31 were
`run` and none was a Poké Ball**, and reached Mt Moon on 92 minutes of cartridge time with a single
Lv19 starter as its whole party — every one of those 204 a full request against a ~50 k-token
history, to answer a question that is usually mechanical. So `set_battle_script` lets it write the
mechanical part down once, as a short program in [Rhai](https://rhai.rs), and the policy runs it. The
script is evaluated **on the emulator thread**, inside `pick_battle_action`, so a scripted turn comes
back on the first poll: no request, no round trip, no latency. A wild encounter interrupting a walk
now costs nothing and resumes by itself.

It reads a `battle` object — both sides, the party, the bag — where every move already carries the
damage it would do to the Pokémon actually in front of it and the type multiplier that goes with it,
because a script that had to carry its own type chart is one no model would get right from memory.
It ends by calling exactly one of `battle.fight`, `battle.switch_to`, `battle.use_item`,
`battle.run` or `battle.ask` — the last of which hands *that one turn* back, so a gym leader can
still be thought about while the Rattatas are not. Whatever the script chooses is resolved against
the same `battle_options` list every other policy chooses from, so it can never take an action the
game would not have offered.

⚠️ **One turn was not enough, because a script asks about a *battle*.** The condition it hands back
on is almost always a property of the fight rather than of the turn — this one is a trainer, this one
is above my level — so it asks on turn 1, the model switches to the Pokémon it wants, and on turn 2
the script runs again and switches straight back. A deployed run spent the fights it most wanted to
think about being overruled by its own program, and nothing inside a battle turn could stop it: the
three script tools are on the overworld turn, and the only other lever was disarming for the rest of
the run — which is the wrong trade, since a script that is wrong for a gym leader is usually right
for the three hundred wild encounters after it. So `choose_battle_action` takes a **`take_over`**:
the rest of *this* battle is the model's, and the script is deciding again at the next one, with
nothing to remember. It is also told what it missed. A script that decides turns in between now hands
back the account of them — what it chose, what that did, what the game said — because otherwise the
model is looking at a battle in which its own last decision simply did not happen, which is a good
way to make it decide the game is broken.

Every run starts with a script already installed, and it is one that calls `battle.ask()` and nothing
else: it decides no turns and hands every battle straight back, so a run that never touches it plays
exactly as it did before there was a default. The point is that there is a file to edit rather than
one to invent. Two deployed runs never called `set_battle_script` once — 207 battle turns and 22.3 M
prompt tokens in the second of them — and never called `get_battle_script_docs` either, so the
feature was not weighed and rejected, it was never reached; `read_battle_script` used to answer a
whole round trip with "there is no battle script", which is a thing the model already knew. Now it
comes back with the file, and the battle turn says in one line that the default is what is deciding
nothing. Passing no `script` goes back to that default rather than to nothing at all.

Three things keep it honest. Nothing about it is trusted: the sandbox has no file, process or network
API at all, and the engine caps operations, wall-clock time, string and collection sizes and call
depth, because the thing being run was written by a model and the thread it runs on owns the
emulator. `set_battle_script` puts the script through seven made-up battles before arming it and
answers with a table of what it chose in each, which is the only chance the model gets to notice that
a rule it meant does something else. And a script that fails — crashes, runs too long, chooses
nothing, or names a move the Pokémon does not know — is **disarmed on the first failure**, with that
turn handed straight back and the reason attached: one strike, because each failure costs a whole
request to report, and a script that failed once will fail again.

After every scripted battle the model gets a report on its next turn: what it did each turn, the
damage either way, the cartridge's own sentences ("It's super effective!", "SPARKY gained 56 EXP!"),
anything the script printed, and how many decisions it took. That report is the entire feedback loop
— a scripted battle is otherwise invisible — so if the script is losing Pokémon or fleeing from
things worth catching, that is where the model finds out.

The model keeps a **plan** it edits itself, shown to it every turn and drawn on the page beside the
game. It is the only thing it writes that survives a context compaction, so an item is meant to carry
its reason as well as its intent. History is compacted once it passes `GB_COMPACT_ABOVE` of the
window: images are evicted first, then older turns are summarised.

The conversation itself survives a **restart**, which it did not used to. It is written to the run
directory at the end of every turn, so a rollout, a crash or a reboot resumes a run that still
remembers what it was doing rather than one that wakes up holding only its plan. Two files, because
the two jobs pull in opposite directions: `history.json` is rewritten each turn and is the smallest
correct copy of the live conversation, and `conversation.jsonl` is appended to and never rewritten,
so it keeps every message the model was ever sent — including the ones a compaction replaced with a
summary, which were previously gone the moment it ran. Neither carries the map pictures: those are
hundreds of kilobytes of base64 apiece, and a restored one would be priced at a twentieth of its real
weight in the token accounting, which is the sort of thing that quietly stops compaction ever firing
again. The caption stays and says the picture went.

⚠️ **A changed system prompt takes effect on the next restart, and says so.** Message 0 is never
stored — it is re-minted from the build that is running — so a deployment that edits the prompt gets
the edit in front of the model rather than pinned to whatever the last process happened to be running.
The conversation underneath it is kept, and the change is logged at `warn` level and marked in
`conversation.jsonl`. Reading that file back as one conversation therefore shows the system prompt
changing partway through, which looks odd and is the honest picture: it is what the model was
actually sent. It should be a rare event, and when it is not that is worth seeing.

A resumed conversation is a little ahead of the game it describes: the save state is the last
checkpoint and the conversation is the last completed turn, so up to a minute of play can be replayed
under it. The model is told so in one line, once, because a situation that contradicts the
conversation is exactly what makes a model decide the game is broken. `GB_RESTORE_HISTORY=0` turns
the restore off, which is the one way to hand a run a fresh conversation without also resetting the
cartridge.

⚠️ The plan rides in a message of its own near the end of the history rather than in the system
prompt, and is re-sent only when it has actually changed. A prompt cache is keyed on the prefix, so
the obvious placement — re-rendering the list into message 0 every request — throws the whole
conversation's cached prefill away every time the model ticks something off. Re-sending it appends a
new copy and leaves the old one alone, for the same reason: removing the stale one is a rewrite of
the middle of the conversation, and the message itself says the last copy is the one that counts.
After the system prompt this history only ever grows at the end.

The catch is that a model which never edits its plan never sees it move either, and both deployed
runs were exactly that — one `todo_set` in 258 turns, sixteen and a single `todo_complete` in 2430 —
so the list it was meant to be revising ended up the least recent thing in every request. A fresh
copy is now appended every tenth overworld turn even when nothing changed, and every turn that does
not carry one is told in a line that the plan is back there and still current. A compaction may drop
the plan along with the turn it belongs to; the next turn re-renders it from the file, so nothing is
lost.

A model that streams its thinking separately — `reasoning_content`, which most local servers send and
OpenAI does not — has it shown live in the log and collapsed to a line once the thought ends. It is
never sent back: reasoning is billed as completion tokens once, and a copy in the history would pay
for it again on every turn after that.

Which leaves a gap, and every tool that ends a turn is asked to fill it: each takes a required
`summary`, one or two sentences in the model's own words about what it is doing and why. Nothing else
it says about a turn survives the turn — the thinking is dropped by the paragraph above, and most
models write no prose at all beside a tool call — so without it the model's half of the conversation
is a column of bare JSON saying what it did and never once why, which is a good way to walk into the
same building four times. It rides on the terminal call's arguments, so it costs no extra round trip
and lands in the history by itself. It is also the line the page leads the decision with.

The action menu is a *model* of the game rather than the game, so sometimes it is wrong — and the
model needs somewhere to put that. For a long time the answer was `press_buttons`, which presses the
joypad directly, going round the agent's whole state machine. It did not work. A deployed run made
749 presses of which 738 were ordinary overworld turns that had a perfectly good menu, ending in 91
turns in a row spent walking into a ledge on Route 3 while the connection into Pewter City sat in the
menu the whole time. Neither "a last resort" in the description nor a required `why` moved that
number: three quarters of the presses left the `why` empty, because a field the schema calls required
and the parser lets through is a field a weak model omits. Nothing had actually failed, either — the
last menu action before that run worked, and nothing was rejected anywhere near it. A model reads its
own recent turns back on every request, so once it presses twice it keeps pressing.

So on any turn that has a menu the tool is simply not offered, and `report_issue` is there instead.
It takes a message — what you tried, what you expected, what happened — and **it does not end the
turn**: the model files the complaint and then still has to choose an action. That is the whole
design, because the reason the escape hatch was over-used is that it was the one way to finish a turn
without choosing, and a terminal replacement would be the same tool under a new name. Every report
writes `issues/turn-<id>/`: the message, the screen, a save state taken at the moment the turn was
put to the model, and the last three turns of conversation with the pictures taken out. Reporting a
problem and playing on stopped being alternatives.

`press_buttons` survives on exactly one turn — the watchdog's, where the agent has reached no
decision point at all, there is no menu to prefer, and a raw button really is the only way out. There
its `why` is enforced rather than merely requested. Prose the model is asked to believe cannot be
checked afterwards; a directory of presses can.

An action the game would refuse is not offered at all. Every HM field move — Cut, Fly, Surf,
Strength, Flash — needs both a Pokémon that has been taught it and a particular gym badge, and the
cartridge answers a missing badge by dropping straight back to the same party menu with the cursor
where it was. The agent has no exit condition for that, so it mashes A for sixty seconds and gives
up. A deployed run walked into it eleven times on one tree in Route 2 with no badges at all, filed
two issue reports saying the game was broken, and spent the rest of its life going round four maps
looking for a way past. So cuttable trees are kept out of the action menu until Cut can actually be
used, water crossings until Surf can, and `use_field_move` refuses the call itself and says which
half is missing. What the turn does say — once, while it is true — is that the trees are there and
what it would take to clear them, because a model that is simply shown no way forward starts
inventing reasons why.

What the game *does* refuse it refuses out loud, and that had the opposite problem: a word. Guards,
locked doors and scripted scenes stop the player where they stand and put a message on screen, so the
walk carrying out the model's action is abandoned — correctly — and the agent said so as "✗ gave up
on the warp to ViridianGym at (32, 9): it was interrupted". The next line quoted the game itself
saying "The GYM's doors are locked...", which is the whole answer: that gym opens on the eighth
badge. A deployed run read the two together, concluded the agent's warp targeting was broken, and
filed a bug asking a developer to look at it. Nothing was wrong except the sentence. Being stopped is
how this game tells you things, so the reason now reads "the game stopped you to say something" —
pointing at the message rather than describing the walk's failure — and the system prompt says once
that a building you cannot get into yet is ordinary, and that what stopped you is quoted in the lines
immediately below.

That last clause was a lie for most of the game's blockers, which is the more serious half of the
same story. Every text box is read character by character and reported once it closes — except that
Pokémon Red turns the player back by printing a message and *then* running a script to step them
backwards, and a script took the agent's state away before the words were reported. So they were
read in full and thrown on the floor. Across the same run a conversation the model walked into was
quoted back 31 times out of 38; a walk stopped by something was quoted 2 times out of 28. It reached
the Route 22 gate, was told its walk had stopped and nothing else, asked the guard directly five
times running, heard nothing each time, and filed a bug. What he actually says is "Only truly skilled
trainers are allowed through. You don't have the BOULDERBADGE yet!" — the whole answer, out loud, for
twelve turns. The reader is now drained wherever it stops being the thing in charge rather than only
when the box closes tidily.

Every Pokémon it catches gets a name it chose. That is a decision the game puts to a player and the
prompt used to talk the model out of it — the tool said keeping the species name "is the ordinary
answer", and across two deployed runs all four naming screens did exactly that. It is now asked for
a name that says what it makes of that particular Pokémon, and the name is checked against the
cartridge's own character set first: it goes straight into the naming screen's buffer, and a
character Gen 1 has no byte for does not fail, it just writes something unreadable for the rest of
the run.

`read_map` answers with a **picture**, not a description: the whole map the player is standing on,
drawn from the cartridge's own tile graphics, with every NPC where they are standing and facing where
they are facing, warps and map edges labelled with where they lead, ground the player cannot reach
dimmed, and a coordinate ruler so a square on the picture and a square in the JSON are the same
square. It is rendered on the worker thread, never the emulator's.

Anything the model does not decide, the agent handles: dialogue is advanced, menus are navigated,
paths across the map are computed from a graph of all 248 maps built out of the ROM's own headers.

A **watchdog** covers the one failure nothing else can see — the agent reaching no decision point at
all, so the policy is never consulted and cannot notice it is stuck. After
`GB_STUCK_TIMEOUT_SECS` of emulated silence (300 by default; ordinary play's longest gap is about
six seconds) the model is asked for a nudge, and every firing is reported to the UI, the transcript
and stdout.

When the endpoint's quota runs out, the run **pauses rather than fails**. A 429 that says when it
reopens is not something to retry — every attempt is another request against the very allowance that
is gone — so `gb` stops asking, and stops the emulator with it: the game is frozen mid-step, the
cartridge's own clock stops (which is the one the leaderboard ranks on), and the page dims the last
frame under a PAUSED plate counting down to the reset. Nothing is lost and nothing is spent; when the
window reopens the same question is put again, to a world that has not moved. A rate limit the
endpoint does *not* date is treated as the ordinary transient one and backed off from in seconds.

The other policies are `RandomPolicy`, `ConsolePolicy` (stdin, for the desktop UI) and
`DeterministicPolicy` (scripted, used by the tests — including the full playthrough).

`--policy deterministic` is that last one served rather than tested: the same queue, the same seed
and the same fresh save `full_playthrough` runs, played out on the page at 1× instead of as fast as
the emulator will go. It needs no API key and spends nothing, and it is the only way to watch the
game actually being *played well* rather than felt out. It plays the game **to the Hall of Fame** —
all eight badges, a grind in the Pokémon Mansion that brings Blastoise to lv85, both Victory Road
boulder puzzles and the Elite Four — and the finished run is then archived
and a new one started, exactly as a winning LLM run would be. When the queue does empty the policy
simply stops answering and the run parks where it stands.

It does it now without a single **black-out**, which was twelve a run when the starter was a
Bulbasaur and seven on the first Squirtle route. None of the six changes that closed the gap is a
bigger number in a step list: the run walks to a Pokémon Centre *before* the fight it would lose
rather than after it, it stops treating "there is a Potion in the bag" as an answer when the Potion is
+20 and the deficit is a hundred, a heal is finished when the party is full rather than when the nurse
starts talking, a charge move is priced at half its power because that is what it does per turn, the
bench is judged on whether it can win the fight rather than on its level, and Lt. Surge — Electric,
against a Water starter — is met with a Ground move the run has been carrying since Cerulean.

The party is **one Squirtle and two Pokémon that never fight**. Blastoise does all of it, with Surf,
Blizzard and Dig; the other two are there because it cannot carry every HM. An **Oddish** caught on
Route 25 holds Cut — `wartortle.asm`'s machine list has SURF and STRENGTH and no CUT, where Ivysaur's
has it, and the route needs Cut four times — and a **Machop** caught on Victory Road holds Strength,
two tiles from the boulder it is for.

⚠️ **Surf is the only HM on the starter, and that is a rule rather than an accident.** An HM is the one
move `pick_move_to_forget` will never drop, so teaching one spends a permanent slot in the only
Pokémon that attacks; Surf earns it by being a 95-power STAB attack that happens to be an HM, and
Strength — 80-power Normal it would never choose — does not.

⚠️ **And it is *one* fighter rather than three, which is faster because experience is cubic.** Taking
three Pokémon to lv75 is about 1.4 M experience; taking one to lv85 is about 425 k, under a third, and
it wins more comfortably — the Elite Four tops out at Lance's lv62 Dragonite and the rival's lv65, so
a lead that far ahead one-shots almost everything instead of trading turns with it. The rule this
replaced said three fighters or you lose the Champion's room; that was true of three at *seventy-five*,
and height turned out to be the answer rather than depth of bench.

⚠️ The grind is most of a scripted run's length and it is not optional: the Elite Four at the levels
the route otherwise arrives with is a coin flip, and losing it is terminal — a blackout inside the
gauntlet warps the player out and the route has no way back to the room it was in.

It **survives a restart**, which it did not used to. The route is rebuilt from the same pure function
on every process start, so a rollout used to resume the *save* wherever it was and the *route* at step
0 in Red's bedroom, and the two would then diverge until the run ended somewhere arbitrary — a
deployment that looked like a pause but was the end of the run. The cursor is now recorded beside the
save in `scripted-progress.json` and read back on the way up. ⚠️ If the route itself has changed under
a run in flight, that cursor counts steps in a list that no longer exists, so the run **parks** rather
than replaying a different route over a game part-way through the old one; start a new one to play the
new route. A fresh run has no cursor, which is read as "start at the beginning", so nothing special
happens on a first run.

Which policy runs is `--policy`, or `GB_POLICY` for a deployment that would rather edit a ConfigMap
than a command line. The flag wins where both are set.

## The run directory

Everything a run needs is one directory, `$GB_RUN_DIR/<run-id>/`:

| | |
|---|---|
| `meta.json` | run id, model, when it started |
| `state.gbst` | the save state — the emulator, exactly as it was |
| `sram.bin` | the cartridge's battery-backed save |
| `transcript.jsonl` | every event, appended; what `/api/history` replays into a page that just loaded |
| `todo.json` | the model's own plan — what outlives a compaction |
| `battle-script.json` | the program deciding its battle turns, and whether it is still armed — every run has one |
| `scripted-progress.json` | how far along the scripted route this run is — only under `--policy deterministic` |
| `history.json` | the live conversation, rewritten each turn — what a restart resumes on |
| `conversation.jsonl` | every message ever sent, appended; the record of what a compaction replaced |
| `issues/` | one directory per `report_issue`: the message, the screen, a save state, the conversation |
| `press-buttons/` | the same, for the watchdog turn's escape hatch: why, and what was pressed |

Copy that directory and the run moves with it. `gb` checkpoints periodically and on the way out —
Ctrl-C and SIGTERM both — so a restart, a rollout or a reboot resumes rather than starts over.

Beside the runs is `$GB_RUN_DIR/hall-of-fame/`: a copy of every run that has finished the game, and
an append-only `ledger.jsonl` of one line each. See below.

## The web UI

`web/` is a Vite + React + TypeScript SPA, embedded into the binary by `rust-embed` and served by
the same process that runs the emulator. Eleven read-only endpoints and three that are not:

| | |
|---|---|
| `/api/events` | SSE: status heartbeat, published on change, plus agent events as they happen |
| `/api/video` | binary: a keyframe, then 8×8 block deltas, deflated per connection — about 21 kbit/s |
| `/api/audio` | binary: a header, then raw Opus packets — 24 kbit/s, and nothing at all until a viewer asks |
| `/api/history?since=` | the transcript backlog, so a page that just loaded is not empty |
| `/api/leaderboard?limit=` | the runs that have finished the game, fastest first |
| `/api/badges.png` | the eight gym badges, decoded from the cartridge's own trainer-card graphics |
| `/api/pokemon/{dex}/front.png` | one Pokémon's battle sprite, decompressed from the cartridge |
| `/api/tool-image/{seq}/image.png` | the picture a tool answered with, while it is still held |
| `/favicon.png` | the overworld Poké Ball, ditto |
| `/api/healthz` | liveness |
| `/version` | which build is running: crate version, build date, branch, short commit |
| `/reset-game` | start the game over, in place — HTTP Basic, off unless `GB_ADMIN_TOKEN` is set |
| `POST /api/new-run` | the same thing for a script, with an `X-GB-Token` header |
| `POST /api/clear` | keep the run, wipe what the model remembers of it — same header |

The screen is streamed as block deltas rather than as images because it is a 160×144 screen that
mostly does not change; the decoder is a TypeScript port of the encoder, in `web/src/video.ts`.

Every tool the model calls is a line in the log, as a sentence rather than as a wire call — "Read the
map", "Chose `PalletTown:5,6:Warp`", "Planned: get the Boulder Badge" — and every one of them opens
onto what was asked and what came back, the map picture included. The picture is *fetched* rather
than carried on the event, out of a small ring on the server: a map render is a couple of hundred
kilobytes and everything published is also a line of the transcript, so a page watching live can open
the map the model was looking at, and one replaying an old backlog gets the caption on its own.

Under the plan is the **battle script**, syntax-highlighted and behind a disclosure — a tab of its own
on a phone. It is closed by default and its head is the part that earns a permanent line, because
`armed` is a live fact: it says whether the battles going past are being decided by that program or
one paid request at a time, and a script that failed is kept, disarmed, with the reason above the
code. A scripted battle is otherwise completely invisible from outside — no request, no turn, no
decision is published — so without this a viewer has no way of telling a run that is playing well
from one that has written down how to. It is published on change rather than on the heartbeat, and
held on the server for a page that opens an hour later: a script is written once and then decides
three hundred battles without another word. The chip in its head has three states rather than two,
because every run starts on the default: `default` says the run has not written one yet and its
battles are costing a request each, which is a fact rather than a fault, and is neither the `armed`
a working script earns nor the `disarmed` a broken one gets.

**No graphics are committed to this repo.** The badges, the party sprites, the favicon, and every
tile, person and letter in the map pictures the model is sent are all read out of the ROM at run
time. The Pokémon sprites are the interesting ones: Gen 1 pics are
compressed, so `src/pokemon/mon_gfx.rs` is a port of pokered's `UncompressSpriteData` — a bitstream
of two 1bpp planes, run-length-encoded zeros and an XOR delta between the planes. All 151 are checked
byte-for-byte against upstream's own build output.

`/api/video` is the one endpoint that is not SSE, and `src/web/video/bench.rs` is why. Measured
against four minutes of real play, the SSE version cost **565 kbit/s**; the "19" this file used to
claim was an idle screen, and an idle screen costs nothing at all. Three changes took that to **21**:
two bits per pixel against the stream's own palette rather than a per-block sub-palette, one deflate
stream across the whole connection rather than one per message (worth 5×, because a Game Boy screen
is repeated 8×8 tiles and a shared window sees every repeat), and dropping base64 — which costs 33%
before compression but 69–113% *after* it.
For comparison, the same footage through x264 is 45 kbit/s losslessly and 25 at a quality that
visibly mangles pixel art, so a real video codec was measured and rejected rather than assumed away.

### The sound

There is a speaker in the corner of the screen, and it is **off until you press it**. The stream is
Opus — 48 kHz mono at 24 kbit/s, one 20 ms packet at a time, length-prefixed down the same kind of
chunked binary response the picture uses and decoded in the browser by WebCodecs. It is deliberately
**not** compressed on top: Opus is already range-coded, and deflating it measures **+16.6%** — the
one place this stream had to ignore what the video stream spent a whole bench file learning.

The plan this replaces was raw 24 kHz stereo PCM at **768 kbit/s** and no encoder, which is 36× the
picture and hard to square with how hard 565 → 21 was fought for. The honest figure for a listener is
nearer **50 kbit/s** than 24, though: a 20 ms packet is only 60 bytes, and the length prefix, the
HTTP chunk header and the TCP segment around it come to about as much again. That is a floor rather
than something to tune away — 20 ms is the longest frame Opus's CELT-only mode has, and CELT-only is
what the encoder picks at this bitrate. Nobody who leaves the speaker alone pays any of it: the
emulator does not encode a single packet while nothing is listening.

The encoder is `opus-rs`, a pure-Rust port of libopus, chosen over the mature C bindings so the
container's only non-Rust dependency stays `ring`'s — the same trade `src/audio/blip/` already made
by being a hand-written port of Blip_Buffer rather than a binding to it. It is also three weeks old,
and it turns out to be **wrong at 24 kHz**: encode a chiptune at that rate and it comes back at
roughly the right loudness with the spectrum destroyed, tones 36 dB down, and no better at a higher
bitrate. At 48 kHz the same signal round-trips through real libopus to within 0.3 dB on every tone.
So the stream is 48 kHz; a test pins that with a spectral check rather than a waveform one — a
waveform comparison reports total failure on a *healthy* transform codec, which is the worst possible
answer to get from the test guarding a young dependency; and the encode call is wrapped in
`catch_unwind`, because a panic in a codec on the emulator thread would take the run's checkpoint
with it. If that ever fires the sound stops and the game plays on.

Audio sits about 0.2 s behind the picture and nothing synchronises the two. Within the stream, the
scheduler trims playback rate by at most ±0.5% to absorb drift: the emulator's clock and the
browser's separate by a couple of seconds an hour even when everything is healthy, and without the
trim that alone would force an audible cut every ten minutes. Real discontinuities — a run parked on
a spent quota for hours, a frozen tab, a dropped network — get one 8 ms fade instead.

### When a run finishes the game

A win is one byte: `wNumHoFTeams`, which pokered increments on the **first frame** of the Hall of Fame
ceremony — before the party parade, the credits, and the game's own save-and-soft-reset back to the
title screen. That is the moment of victory, with the winning party still in memory, so that is where
the record is taken.

What happens then, in order: the run is checkpointed, copied whole into
`$GB_RUN_DIR/hall-of-fame/<date>-<run-id>/` — save state, SRAM, the model's plan and the run's entire
transcript, gzipped — one line describing it is appended to `hall-of-fame/ledger.jsonl`, and the next
run starts automatically. Nothing is deleted: the finished run directory is left exactly where it was
and is still resumable.

The ledger row is the run's whole story in numbers: how long it took by the cartridge's own clock and
by ours, tokens spent, turns taken, which policy and model decided them, which version of `gb` played
it, how many times it was resumed, and what it finished with. `/api/leaderboard` reads it back and the
🏆 in the page's header shows the top ten, **fastest by in-game time** — the one figure that survives
a resume without any bookkeeping, because it lives in the save file.

### Starting a new run without a restart

Open **`/reset-game`** and the browser asks for a password — any user name, `GB_ADMIN_TOKEN` as the
password. The current run is checkpointed and left complete on disk, and the game starts again in a
fresh run directory: no restart, no downtime, and every open page follows on its own.

The page itself has no button and nothing links to that URL. A `WWW-Authenticate` challenge is the
browser's own dialog, so the SPA holds no token and needs no prompt; and a GET that resets the game
should not be reachable by a prefetch, a crawler or a middle-click. It is a URL you type.

For a script, the same thing with a header:

```shell
curl -X POST -H "X-GB-Token: $GB_ADMIN_TOKEN" https://your-host/api/new-run
# → {"run_id":"run-20260811-142233"}
```

Both are **off unless `GB_ADMIN_TOKEN` is set**, and both 404 rather than 403 when it is not — the
server is on the public internet and an endpoint that resets the game should not advertise itself to
whoever scans for it. Nothing is deleted: the old directory is a complete run and can be resumed by
pointing a process back at it.

### Clearing the conversation without losing the run

The opposite trade, for the run that has talked itself into a corner:

```shell
curl -X POST -H "X-GB-Token: $GB_ADMIN_TOKEN" https://your-host/api/clear
# → {"run_id":"run-20260811-142233","cleared":"the conversation and the plan; …","when":"at the model's next turn"}
```

The game, the save, the run directory, the transcript and the battle script all carry on untouched.
What goes is what the *model* remembers: the conversation starts again from the system prompt, and
`todo.json` is deleted, because the plan is the one thing written to outlive a conversation and a
plan composed by the conversation being deleted is the same corner in a file. `conversation.jsonl`
keeps every message it ever held, with a `cleared` line where the cut was.

It exists because a conversation only ever grows at the end. A model that has convinced itself the
game is broken reads that conclusion back on every request until a compaction happens to drop it,
and the only cure was `POST /api/new-run` — which throws eight hours of playthrough away to fix a
model's mood. Same admin token, same 404 when it is unset, and a run played by anything other than
a model answers "this run is not being played by a model" rather than pretending to have done
something.

⚠️ It lands at the model's **next turn** rather than at the moment it answers. A run directory has
one writer and it is the worker thread, so the emulator cannot delete those files itself; the turn
in flight is cancelled to make the next one start now. On a run parked on a spent quota, nothing
happens until the quota reopens.

The first turn after a clear opens on a line telling the model the erasure was deliberate, that the
game is sound, and to look around and write a fresh plan before it goes far — because a model shown
six badges and no memory of earning any of them is a model about to decide something is very wrong.

For a hot-reload loop, run `gb serve` on :8080 and `pnpm run dev` in `web/`, which proxies `/api` to
it. `GB_WEB_DEV=1` reads `web/dist` from disk instead of the embedded copy, which skips the cargo
rebuild after an SPA build.

## Configuration

All environment variables, never flags — the API key has to be one, so the rest followed it.
`gb --help` lists them and `src/llm/config.rs` documents them.

| | |
|---|---|
| `OPENAI_API_KEY` | required for `--policy llm` |
| `GB_MODEL` | required for `--policy llm` |
| `OPENAI_BASE_URL` | any OpenAI-compatible endpoint |
| `GB_CONTEXT_LIMIT` | the context window, in tokens — set it to the model's, not the default 128 k |
| `GB_COMPACT_ABOVE` | how full it gets before the turn loop compacts (`0.85`) |
| `GB_TEMPERATURE`, `GB_MAX_TOOL_STEPS` | the turn loop's shape |
| `GB_REQUEST_TIMEOUT_SECS` | how long an endpoint may take to answer (`180`) — raise it for a local one |
| `GB_MAX_TOKENS` | ceiling on one completion (`8192`); `0` removes it |
| `GB_REASONING_EFFORT` | sent as `reasoning_effort` when set — `none` turns thinking off entirely |
| `GB_STUCK_TIMEOUT_SECS` | the watchdog; `0` turns it off |
| `GB_POLICY` | what plays the game: `llm` (default), `random` or `deterministic`; `--policy` wins |
| `GB_RUN_DIR` | where runs live (default `./runs`) |
| `GB_PORT`, `GB_STATUS_HZ` | the server |
| `GB_AUDIO_BITRATE` | the Opus stream's target, bits/s (`24000`); `0` turns sound off entirely |
| `GB_HARDWARE` | which Game Boy the cartridge runs on: `dmg` (default) or `cgb` |
| `GB_RESTORE_HISTORY` | resume a run's conversation as well as its save (`1`); `0` starts the conversation over |
| `GB_ADMIN_TOKEN` | enables `/reset-game`, `POST /api/new-run` and `POST /api/clear`; unset means all three 404 |

## Deployment

The `Dockerfile` builds everything from a bare checkout in four stages — rgbds and the cartridge,
then the SPA, then the crate, then a 146 MB image of which the binary is 6.9 MB. `ghcr.io/axle-h/gb`
is published by CI on every push to main, after a smoke test that proves the image actually serves
and emulates.

```shell
OPENAI_API_KEY=sk-… GB_MODEL=… docker compose up -d
docker compose run --rm --service-ports gb gb serve --policy random   # no API key, no spend
```

### Which build is that?

```shell
curl -s https://your-host/version
# → {"version":"1.0.0","build_date":"2026-08-12T14:22:33Z","branch":"main","commit":"a1b2c3d"}
```

The crate version comes from `Cargo.toml`; the other three are stamped into the image by CI as build
args, and read from the environment (`GB_BUILD_DATE`, `GB_GIT_BRANCH`, `GB_GIT_SHA`) rather than
compiled in — the timestamp changes on every build, and an `env!()` would put it in the cargo layer's
inputs and buy a full cold `cargo build --release` on every CI run. `docker inspect` sees the same
commit as `org.opencontainers.image.revision`, in full. `gb serve` prints the lot on the way up, so
`docker logs` answers the question too, and a binary built from a working tree reports `null` for
what nobody told it. Nothing about this is on the page: it is an operator's question.

`k8s/` has manifests for k3s — one namespace, one pod, a volume for the run directory, and TLS
terminated outside by traefik and cert-manager. See [`k8s/README.md`](k8s/README.md).

## Architecture

```
src/
├── main.rs              — entry point: `gb` (SDL UI) or `gb serve` (web), dispatched from cli.rs
├── cli.rs               — hand-rolled arg parsing; `parse` is unit-testable without a process
├── host.rs              — headless emulator host: GameBoy + PokemonAgent + both encoders on one thread
├── run/                 — the run directory (`web` feature): checkpoint, resume, transcript
│   ├── mod.rs           — $GB_RUN_DIR/<run-id>/: meta.json, state.gbst, sram.bin; atomic writes
│   ├── transcript.rs    — transcript.jsonl writer thread + the /api/history backlog reader
│   └── hall_of_fame.rs  — a finished run: the archive, the ledger, and the leaderboard read back
├── web/                 — the axum server (`web` feature); read-only but for the reset
│   ├── published.rs     — the only interface between the emulator thread and HTTP
│   ├── audio.rs         — the Opus encoder and /api/audio's wire format
│   ├── video.rs         — 8×8 block-diff video codec + the reference decoder
│   │   └── bench.rs     — what the stream costs, and every alternative it was chosen over
│   ├── assets.rs        — the SPA: `web/dist` embedded, or read from disk under GB_WEB_DEV=1
│   ├── badges.rs        — /api/badges.png: the eight badges, decoded from the cartridge
│   ├── leaderboard.rs   — /api/leaderboard: the runs that have finished the game
│   ├── sprites.rs       — /api/pokemon/{dex}/front.png and /favicon.png, ditto
│   └── version.rs       — /version: the crate version, and what CI stamped into the image
├── llm/                 — the LLM client and turn loop (`llm` feature)
│   ├── config.rs        — the environment block: OPENAI_*, GB_MODEL, GB_MAX_TOOL_STEPS, …
│   ├── protocol.rs      — OpenAI wire types + the SSE accumulator (no HTTP; pure and testable)
│   ├── client.rs        — `ChatEndpoint` + `OpenAiClient` over ureq, and the retry policy
│   ├── tools.rs         — the tool catalogue, scoped per decision kind; ids; servicing
│   ├── prompt.rs        — the system prompt and the per-turn situation
│   ├── screenshot.rs    — one published frame as a PNG data URL, encoded on the worker thread
│   ├── map_image.rs     — the whole current map as a labelled picture, ditto
│   ├── accounting.rs    — tokens reported vs tokens estimated, and the calibration between them
│   ├── todo.rs          — the model's plan: the only thing it writes that survives a compaction
│   ├── battle_script.rs — the Rhai sandbox a scripted battle is decided in, and the script on disk
│   ├── battle_report.rs — what happened in a battle nobody was asked about
│   ├── history.rs       — the conversation on disk: restored on a restart, logged past a compaction
│   ├── compaction.rs    — image eviction + summarising compaction, as pure functions over the history
│   └── worker.rs        — the turn loop: stream → tool batch → terminal call, with cancellation
├── game_boy.rs          — top-level GameBoy struct (run loop, save/restore)
├── core.rs              — CPU + MMU wiring
├── opcode.rs            — full SM83 instruction set
├── mmu.rs               — memory map, bank switching, the absolute clock (`now`)
├── mbc.rs               — memory bank controllers (RomOnly/MBC1/2/3/5/HuC1), dispatched from CartType
├── schedule.rs          — event schedule: when each peripheral next does something
├── ppu.rs               — pixel processing unit (LCD rendering)
├── model.rs             — Model (Dmg/Cgb) + ColorMode (Dmg/CgbCompat/Cgb)
├── cgb_palette.rs       — CGB palette RAM (BCPS/BCPD, OCPS/OCPD)
├── boot_palette/        — the CGB boot ROM's DMG-compatibility palette tables
├── hdma.rs              — CGB VRAM DMA (GDMA + HBlank-paced HDMA)
├── audio/               — APU (4-channel Game Boy audio)
│   └── blip/            — band-limited synthesis + resampling to the sink's rate (Blip_Buffer port)
├── sdl/                 — SDL2 UI: renders LCD at 4× scale, drives audio, keyboard input
│   └── render.rs        — main render loop; instantiates GameBoy + PokemonAgent
├── roms/                — bundled test ROMs (cpu_instrs, dmg-acid2, cgb-acid2, etc.)
└── pokemon/             — Pokémon Red layer (everything below)
    ├── mod.rs            — PokemonApi / PokemonApiTrait / GameState
    ├── agent.rs          — PokemonAgent: drives joypad each frame, emits AgentEvents
    ├── policy.rs         — Policy trait + impls (Random, Console/stdin, Deterministic)
    ├── llm_policy.rs     — LlmPolicy: kind-keyed turns, cancellation, tool servicing (`llm` feature)
    ├── actions.rs        — OverworldAction (walk to tile, warp, talk to sprite)
    ├── encoding.rs       — reads/writes Pokémon data structures from MMU
    ├── symbols.rs        — DmgPointer / DmgBank types + include of generated symbols
    ├── world_graph.rs    — graph of all 248 maps built from ROM headers at runtime
    ├── tile_map.rs       — MetaTileMap: abstracts the current map into typed tiles
    ├── map.rs            — Map enum (all 248 maps)
    ├── battle.rs         — battle state reader + BattleAction
    ├── rom_gfx.rs        — reading ROM graphics: bank windowing, 2bpp tiles, the Poké Ball
    ├── badge_gfx.rs      — the badge sprites, decoded from the trainer card's ROM graphics
    ├── mon_gfx.rs        — the front pics, a port of pokered's `UncompressSpriteData`
    ├── map_gfx.rs        — tileset sheets, overworld sprite sheets and the game's own font
    ├── delay.rs          — DelayContext: cycle-accurate waits between agent steps
    ├── text.rs           — PokemonTextReader: reads on-screen text from VRAM
    ├── integration_tests/ — agent end-to-end tests, tiered by emulated game time
    ├── data/             — saved emulator state snapshots (.bin) used by tests
    └── roms.rs           — embeds pokered/pokered.gbc as a compile-time byte slice
```

Cargo features are `default = ["sdl", "web", "llm"]`. `web` and `llm` are on by default
deliberately: the video codec, the emulator host, the SSE parser, the turn-cancellation contract and
a mock-server playthrough are all default-tier tests, and behind an opt-in feature a plain
`cargo test` would silently skip every one of them. The container build is
`--no-default-features --features llm`, which drops the SDL2 link dependency entirely.

### Key choices

| Concern | Choice | Reason |
|---|---|---|
| Language | Rust | The emulator must run far faster than real time |
| Desktop UI | SDL2 (`sdl2` crate) | Lightweight, easy audio queue + video surface |
| Audio resampling | `src/audio/blip/`, no dependency | A Rust port of blargg's Blip_Buffer. Band-limited *step* synthesis rather than sinc resampling: the APU reports amplitude transitions and they are written straight into a buffer already at the output rate. 8 output samples of latency, no FFT, no crates |
| Serialisation | `bincode` + `lz4_flex` | Fast snapshot/restore for the save states the tests are built on |
| Save state format | labelled sections | `"GBST" \| version \| lz4 { [label][len][payload] }`. Unknown sections are skipped and missing ones are not errors, so adding one is free — CGB support doubled VRAM and quadrupled WRAM at the cost of zero fixture regeneration |
| Symbol codegen | `build.rs` + `pokered/pokered.sym` | Every RAM/ROM symbol becomes a typed `DmgPointer` constant, so an address that moves upstream is a compile error |
| pokered | a git submodule | The source of truth for the ROM, the symbol map and the game's data tables |
| LLM transport | `ureq` + hand-rolled SSE | The wire types and the stream accumulator are pure and testable without HTTP |
| Audio transport | raw Opus over the same chunked binary framing | No container and no muxer: the WebCodecs Opus registration takes bare packets, and supplying an `OpusHead` would put the decoder into Ogg mode instead. Not deflated either — measured at **+16.6%** on top of a codec that is already range-coded. Encoded once on the emulator thread for every listener, which is the opposite of the video path's per-connection deflate and for the same reason: there is no shared window here worth duplicating |
| Video transport | chunked binary + `flate2` | Not a WebSocket: nothing is bidirectional, and a plain response needs no upgrade, no ping/pong and no second reconnection story. The compression is the protocol rather than a `Content-Encoding`, so no proxy can decide to buffer and re-encode it |

## Tests

```shell
cargo test --release                                              # ~7 s, the default tier
cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests
cargo test --release --features full-playthrough full_playthrough # 8 badges, ~7 min
cargo test --release --features hall-of-fame --bin gb -- hall_of_fame # to the credits, ~26 min
```

Always `--release`: these tests emulate every frame and are unusably slow otherwise. The suite is
tiered by how much *game time* a test costs, since that is the only thing that matters to its wall
clock. `CLAUDE.md` has the full map of the tiers, the fixture chain and the benchmarking setup.

## A note on licences

There is no top-level `LICENSE` here yet. If one is added, the constraint to check first is
`src/audio/blip/`: it is a translation of blargg's Blip_Buffer 0.4.0, which is LGPL 2.1+. The
original C++ and its licence are vendored under `tools/blip-golden/`.

The other codec in the audio path constrains nothing: `opus-rs` is BSD-3-Clause, a Rust port of
libopus, which is BSD-3 itself — so it asks for attribution and nothing more. Neither does `rhai`,
the battle-script sandbox, which is MIT OR Apache-2.0. Both are named here because this paragraph is
the list, and a dependency absent from it is one nobody checked.

The ROM is not distributed and cannot be — `pokered/` is a submodule of the disassembly project, and
the cartridge is assembled locally from it.

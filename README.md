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

**The game.** The agent layer can play Pokémon Red from a fresh save to all eight badges and beyond;
`full_playthrough` is a test that does exactly that in about five minutes of wall clock, because the
emulator runs at roughly 50× real time with the agent on top of it.

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

The model keeps a **plan** it edits itself, shown to it every turn and drawn on the page beside the
game. It is the only thing it writes that survives both a context compaction and a restart of the
program, so an item is meant to carry its reason as well as its intent. History is compacted once it
passes `GB_COMPACT_ABOVE` of the window: images are evicted first, then older turns are summarised.

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

## The run directory

Everything a run needs is one directory, `$GB_RUN_DIR/<run-id>/`:

| | |
|---|---|
| `meta.json` | run id, model, when it started |
| `state.gbst` | the save state — the emulator, exactly as it was |
| `sram.bin` | the cartridge's battery-backed save |
| `transcript.jsonl` | every event, appended; what `/api/history` replays into a page that just loaded |
| `todo.json` | the model's own plan — the one thing it writes that outlives its conversation |
| `issues/` | one directory per `report_issue`: the message, the screen, a save state, the conversation |
| `press-buttons/` | the same, for the watchdog turn's escape hatch: why, and what was pressed |

Copy that directory and the run moves with it. `gb` checkpoints periodically and on the way out —
Ctrl-C and SIGTERM both — so a restart, a rollout or a reboot resumes rather than starts over.

Beside the runs is `$GB_RUN_DIR/hall-of-fame/`: a copy of every run that has finished the game, and
an append-only `ledger.jsonl` of one line each. See below.

## The web UI

`web/` is a Vite + React + TypeScript SPA, embedded into the binary by `rust-embed` and served by
the same process that runs the emulator. Ten read-only endpoints and two that are not:

| | |
|---|---|
| `/api/events` | SSE: status heartbeat, published on change, plus agent events as they happen |
| `/api/video` | binary: a keyframe, then 8×8 block deltas, deflated per connection — about 21 kbit/s |
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

The screen is streamed as block deltas rather than as images because it is a 160×144 screen that
mostly does not change; the decoder is a TypeScript port of the encoder, in `web/src/video.ts`.

Every tool the model calls is a line in the log, as a sentence rather than as a wire call — "Read the
map", "Chose `PalletTown:5,6:Warp`", "Planned: get the Boulder Badge" — and every one of them opens
onto what was asked and what came back, the map picture included. The picture is *fetched* rather
than carried on the event, out of a small ring on the server: a map render is a couple of hundred
kilobytes and everything published is also a line of the transcript, so a page watching live can open
the map the model was looking at, and one replaying an old backlog gets the caption on its own.

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
| `GB_RUN_DIR` | where runs live (default `./runs`) |
| `GB_PORT`, `GB_STATUS_HZ` | the server |
| `GB_HARDWARE` | which Game Boy the cartridge runs on: `dmg` (default) or `cgb` |
| `GB_ADMIN_TOKEN` | enables `/reset-game` and `POST /api/new-run`; unset means both 404 |

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
├── host.rs              — headless emulator host: GameBoy + PokemonAgent + video encoder on one thread
├── run/                 — the run directory (`web` feature): checkpoint, resume, transcript
│   ├── mod.rs           — $GB_RUN_DIR/<run-id>/: meta.json, state.gbst, sram.bin; atomic writes
│   ├── transcript.rs    — transcript.jsonl writer thread + the /api/history backlog reader
│   └── hall_of_fame.rs  — a finished run: the archive, the ledger, and the leaderboard read back
├── web/                 — the axum server (`web` feature); read-only but for the reset
│   ├── published.rs     — the only interface between the emulator thread and HTTP
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
│   ├── todo.rs          — the model's plan: the only thing it writes that survives a restart
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
| Video transport | chunked binary + `flate2` | Not a WebSocket: nothing is bidirectional, and a plain response needs no upgrade, no ping/pong and no second reconnection story. The compression is the protocol rather than a `Content-Encoding`, so no proxy can decide to buffer and re-encode it |

## Tests

```shell
cargo test --release                                              # ~7 s, the default tier
cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests
cargo test --release --features full-playthrough full_playthrough # the whole game, ~5 min
```

Always `--release`: these tests emulate every frame and are unusably slow otherwise. The suite is
tiered by how much *game time* a test costs, since that is the only thing that matters to its wall
clock. `CLAUDE.md` has the full map of the tiers, the fixture chain and the benchmarking setup.

## A note on licences

There is no top-level `LICENSE` here yet. If one is added, the constraint to check first is
`src/audio/blip/`: it is a translation of blargg's Blip_Buffer 0.4.0, which is LGPL 2.1+. The
original C++ and its licence are vendored under `tools/blip-golden/`.

The ROM is not distributed and cannot be — `pokered/` is a submodule of the disassembly project, and
the cartridge is assembled locally from it.

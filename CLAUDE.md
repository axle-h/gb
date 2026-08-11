# CLAUDE.md

@README.md

**The README is imported above, so it is already in your context — do not re-read it, and do not
repeat it here.** It has what the project is, the `src/` tree, the policy/agent model, the run
directory, the endpoints, the environment block, the build and the deployment. This file has only
what the README deliberately leaves out: the invariants, the traps, and the test workflows. Nearly
every ⚠️ below was learned by breaking something.

The only surviving design doc is `docs/llm-web-playthrough-plan.md` (the LLM/web work, W0–W9, all
done). The emulator-accuracy plan it used to sit beside was deleted once its phases landed; where a
number below is attributed to "Phase C" or "Phase D", that is the history, not a file you can open.

## Rules of the road

- **Always `--release`.** The integration tests emulate every frame and are unusably slow in debug.
- The crate has **no lib target** — it is `--bin gb`, never `--lib`.
- Agent and policy debugging goes to stdout, so add `--nocapture` when you care about it.
- **Run `full_playthrough` after every major work item and before pushing.** See below for why the
  leg tier is not a substitute.

## Build inputs

⚠️ **pokered needs rgbds ≥ 1.0.0 and `rgbdscheck.asm` `fail`s the assembly below that** — a hard
error, not a warning, so an old rgbds does not produce a wrong ROM, it produces none. The container
pins 1.0.3 (`ARG RGBDS_VERSION`), which upstream's own `INSTALL.md` and CI name.

⚠️ **The symbol names are upstream's to change.** The 2026-08 bump renamed pokered's "hidden object"
and "missable object" vocabulary to **hidden event** and **toggleable object** (`HiddenEventMaps`,
`HiddenEventPointers`, `wToggleableObjectFlags`). That surfaces as a compile error rather than a
silent one only because `build.rs` emits constants for symbols that exist — nothing more.

⚠️ **`web/dist` must exist for the crate to compile at all** — `rust-embed`'s derive fails if the
folder is missing. That is why `web/dist/.gitkeep` is committed and why `vite build` (which empties
`dist`) copies it back from `web/public/`. A checkout that has never run `pnpm run build` compiles
and serves a page naming the two commands to run, so a missing UI is never a mystery.

⚠️ **`web/pnpm-workspace.yaml`'s `minimumReleaseAge` cooldown is checked on *every* install —
`--frozen-lockfile` included, not just `pnpm update`.** A lockfile pinning anything younger than the
window fails with `ERR_PNPM_NO_MATURE_MATCHING_VERSION`, so the lockfile has to be *generated* under
the same number the builds enforce. Raise the window without regenerating `pnpm-lock.yaml` and what
breaks is the container build, not the dev loop. The file says why it is 3 days and when that can
change. pnpm's version is pinned by `packageManager` in `web/package.json` and activated by corepack
— deliberately not named in the Dockerfile, so there is nothing there to drift from it.

## Emulator invariants

⚠️ **Every mapper resolves its bank register differently and the differences are not decoration.**
MBC1 remaps a zero selection *then* wraps, so a wrap can reach bank 0; MBC3 wraps *then* remaps, so
it never can; MBC2/MBC5/HuC1 have their own rules again. Same two operations, opposite order,
different answer — and it is what makes blargg's combined `dmg_sound.gb` terminate. `src/mbc.rs`'s
module docs have the table.

⚠️ **The RTC's time source is injectable and anything replayable must pin it**
(`MMU::set_rtc_time_source`) — the default is the host clock, so an RTC cartridge under a
fixture-driven test would fail only sometimes. Nothing committed has an RTC: `pokered.gbc` is `0x13`,
MBC3 with *no* timer.

**Save states.** `src/savestate/mod.rs`'s module docs are the authoritative reference. **Adding a
section is free; adding a field means appending it as an extra value within its section and bumping
that section's version** — neither churns fixtures. ⚠️ **Never reorder or retype an already-shipped
value without bumping the section version**: bincode is positional and has no schema migration. If
you find yourself about to write a legacy struct, check first whether you can re-cut the boundary
instead — that is how CGB support cost zero fixture regeneration, by keeping `wram`/`ppu`'s shipped
first value and appending the new banks as a second.

The **91** committed fixtures in `src/pokemon/data/*.bin` are `include_bytes!`'d;
`every_committed_fixture_decodes` in the default tier fails in seconds if a layout change breaks
them. `pokemon-red.sav` is raw SRAM, not a save state — the SDL UI loads and writes it at runtime.

⚠️ **`Audio` and `PPU` exclude derived state from `PartialEq`** — the resampler output and the cached
mix (`mixed`/`levels`/`mix_dirty`), and the frame buffer plus the per-scanline sprite list
respectively. None of it is serialised, so none of it may take part in equality, or
`game_boy::tests::save_and_load_state` would compare restored state against state that was never
saved. `Schedule` is derived the same way and is not serialised at all — only the clock it is built
from (`MMU::now`, the `sched` section) is. Adding a field to `Audio` **is** safe now; the old
"nothing may be added to `Audio`" rule died with the sectioned format. The output sample rate is
applied by `Audio::set_output_sample_rate` rather than stored, so a caller that loads a save state
must re-apply it (see the `F9` handler in `render.rs`).

⚠️ **`PPU::draw_pixels_to` and the three DMA transfer loops are `#[inline(never)]`/`#[cold]` on
purpose.** `MMU::update` runs once per CPU instruction, and letting those inline into it grew it 60%
(3052 → 4893 bytes) and cost several percent of core throughput to instruction-cache pressure alone.
If you touch them, check with `nm -S --size-sort -C target/release/deps/gb-*` that `MMU::update` is
still around 3–4 KB (Phase C left it at 3764 bytes). `Serial::complete_transfer` and the APU's
`mix()` fell to the same rule.

⚠️ **`MachineCycles::to_duration` multiplies by 4e9, and that overflowed `u64` after ~73 minutes of
emulated time** — silently, because release builds wrap rather than panic. Everything that reports
emulated time over a long run went through it: `meta.json`'s `emulated_ms` and the status heartbeat
both wrapped every 73 minutes on the deployed run. It surfaced as `soak`'s progress line simply
stopping after 3600 s, which looked like a bug in the test. `from_duration` had always used `u128`;
now both do, and `cycles::tests::to_duration_survives_a_long_run` pins it out to 24 h.

**Tuned constants**, both arrived at empirically: `AGENT_RESOLUTION` (20 ms) — longer and the player
overshoots on the overworld, shorter and the game state does not settle between frames; and
`DelayContext`'s 2500 ms post-script delay, which covers the worst-case pre-battle animation gap
observed in practice.

## Agent and policy invariants

⚠️ **`PokemonAgent::poll_policy` is the single seam every decision point goes through**, and it is
not just a tidy-up: it resets the clock the stuck-run watchdog reads. Call `policy.service_tools`
directly from a new poll site and the watchdog will believe the run has been wedged since that
moment, forever.

⚠️ **The emulator never pauses while the model thinks, and must not be made to.** A tool batch is
answered by `Policy::service_tools`, which only runs when `gb.run` advances the agent — so any pause
spanning an LLM tool call deadlocks the run. A `GB_PAUSE_WHILE_THINKING` flag was built in W4 and
removed the same day; `HostConfig` in `src/host.rs` carries the ⚠️.

**The watchdog** (`Policy::{stuck_timeout, pick_unstick}`) raises a `DecisionKind::Stuck` turn whose
only terminal tools are `press_buttons` and `wait`. Two ⚠️s, both learned in the design:

- **It is asked on every tick of the jam, not once.** A tool batch is only serviced inside
  `agent.update`, so a one-shot notification would hang any turn that wanted to read first.
- **It must not reset the clock it reads**, or the jam clears the instant it is noticed and the turn
  is never polled again.

In a healthy run it never fires — `mechanics::ordinary_play_stays_far_inside_the_stuck_timeout`
measures ordinary play's longest silence at ~6 s of game time against the 300 s default.

⚠️ **Every Gen 1 PC menu is a closed loop under A-only input, and `ReadingTextBox` presses B when
`PokemonApiTrait::in_pc_menu` says so.** Each PC menu leaves only on B, and A on its resting cursor
picks the first entry, which bounces off a refusal message straight back with the cursor untouched —
`PCMainMenu` → Bill's PC → `WITHDRAW` → `NoMonText` → `BillsPCMenu`, or `PlayerPCMenu` →
`WITHDRAW ITEM` → nothing stored → `PlayerPCMenu`. Nothing in the cycle moves the cursor, so A never
reaches `LOG OFF`. This wedged the deployed run **permanently**, eight tiles from a fresh save, and
it was not a Bill-event or empty-box problem: a full party or a one-mon party traps identically.

Two traps in the detection, both paid for once. **The item PC sets no flag** —
`TextScript_PokemonCenterPC` goes through `ActivatePC` and sets `wMiscFlags`' `BIT_USING_GENERIC_PC`,
but `TextScript_ItemStoragePC` (Red's bedroom, and the one that actually broke) calls `PlayerPC`
directly and deliberately leaves it clear, so the screen is matched on `LOG OFF` as well. And
**`LOG OFF` alone is not enough either**, because the parent tree's submenus do not show one — hence
both checks. `UsingPcBox`/`UsingItemPc` cannot collide with this: they are excluded from
`assert_text_box_state`, so they never reach `ReadingTextBox`. Their *abort* paths do, which is a
second bug fixed by the same line.

⚠️ **The status heartbeat is sent on change, not on a timer.** Sampled at `GB_STATUS_HZ` and
published only when it says something the last one did not, with a 2 s keepalive so an idle run still
proves it is alive and `curl -N /api/events` still ticks. At the original 10 Hz unconditional it
measured **49.7 kbit/s per viewer** — six times the idle video feed, nine of ten payloads
byte-identical to the one before; it is now 5.2. Two consequences: `StatusSnapshot` compares with
`says_the_same_as`, which excludes the clocks and `frame_seq` (a derived `PartialEq` would never
match and the suppression would silently never fire), and `/api/events` **opens with the latest
heartbeat** — `Published::join_events`, subscribe-then-read, the same handshake as the video keyframe
— or a page opened during a quiet stretch shows an empty panel.

## Starting a new run in place

`POST /api/new-run` restarts the game without restarting the process. ⚠️ **It is the only channel
from the HTTP layer back into the emulator**, and `src/web/mod.rs`'s module doc used to say there was
none at all — that property was structural, so giving it up was a deliberate edit rather than a
drift. `host::NewRunRequests` is the whole of it: no data travels inwards, only the fact that someone
asked, and it is answered at the **top of `EmulatorHost::tick`**, which is the one point where
nothing is half-done.

⚠️ **A run directory has exactly one writer, and five things had a copy of which one it was** — the
checkpointer, the transcript thread's open file, `/api/history`'s path, `/api/healthz`'s run id, and
the LLM worker's notes. They all read `run::CurrentRun` now. The transcript thread in particular
**re-reads the path per event**: a captured `PathBuf` keeps appending the new run's events to the old
run's file, and nothing notices until someone reads either one.

Three more that are easy to get wrong, each with a test:

- **Checkpoint the outgoing run before swapping.** Everything since its last periodic write — up to a
  minute — lives only in memory, and the directory left behind has to be resumable.
- **`VideoEncoder::restart`, not `VideoEncoder::default`.** Deltas are diffed against `last_sent`, so
  a state swap without it leaves fragments of the abandoned run on every viewer's screen. But `seq`
  must survive: `/api/video` drops anything at or below the seq a client opened with, so restarting
  the count at zero makes a live viewer discard the entire new run.
- **Clear `last_status`**, or the send-on-change rule suppresses the one heartbeat that says the run
  changed.

`GB_ADMIN_TOKEN` gates it and **404s when unset** rather than 403ing — this serves the public
internet. Blank counts as unset, because that is the shape a placeholder Secret takes.

## Tests

`src/pokemon/integration_tests/` is tiered by how much **game time** a test emulates, which is what
it costs. The core runs at **~91× realtime** on Pokémon Red and the agent costs **~35%** on top,
giving **~50×** end to end (measured 2026-08-06 on a Ryzen 9 7900X by `bench_core_throughput` and
`bench_emulation_throughput`), so wall clock ≈ emulated-minutes ÷ 48.

Those are post-Phase-C numbers: the core was 29× and the agent-inclusive figure 24× before it, a
3.1× speedup. The agent's share grew from ~16% to ~35% for the obvious reason — it did not get
slower, the emulator under it got faster — so **the agent is now worth profiling and it was not
before.**

```bash
# Default tier: all unit tests + agent mechanics + two navigation smoke tests + web/host/llm.
# ~7s, 1162 tests.
cargo test --release

# Leg chain: one test per PolicyStep::*_steps() leg, each seeded from a committed snapshot.
# 131 tests in ~50s of wall clock (measured 2026-08-11; it took ~131s before Phase C).
cargo test --release --features slow-tests --bin gb -- pokemon::integration_tests

# The one leg that costs more game time than the whole leg chain combined: the Safari dex sweep,
# 171s of wall clock for ~190 min of emulated game time (21 paid ¥500 trips chasing 4.3%-slot
# species; it was 381s before Phase C). Split out because it, alone, set the leg tier's wall clock
# at six minutes — with libtest printing nothing until it finished, so there was no way to see what
# was still running. ⚠️ `very-slow-tests` does not imply `slow-tests`, and the test's module is
# behind that gate, so pass **both** features or this matches zero tests.
cargo test --release --features slow-tests,very-slow-tests --bin gb -- can_sweep_the_safari_zone

# The whole game from a fresh save, ~5 min of wall clock (was ~11 min before Phase C).
cargo test --release --features full-playthrough full_playthrough

# The stall hunt: 5 h of game time under RandomPolicy, ~4.5 min of wall clock. Fails if the agent
# ever goes longer without reaching a decision point than the watchdog allows. Seeded — vary
# GB_SOAK_SEED to hunt for new jams; seed 1 is the one that must stay green.
cargo test --release --features soak-tests --bin gb -- soak --nocapture

# A single test with output (file module included in the path).
cargo test --release --bin gb -- pokemon::integration_tests::mechanics::test_debouncing --exact --nocapture

# The PPU comparisons: dmg-acid2, cgb-acid2, Pokémon Red in colour.
cargo test --release --bin gb -- game_boy::tests::ppu

# The diagnostics and probes.
cargo test --release --features diagnostics,slow-tests --bin gb -- probe_ --ignored --nocapture

# Agent throughput (emulator + agent.step). `--exact` needs the full module path, or this
# matches zero tests.
cargo test --release --bin gb -- \
  pokemon::integration_tests::fixture::bench_emulation_throughput --exact --ignored --nocapture

# Emulator core alone — no agent, no policy, no observation. Three workloads. Behind the `bench`
# feature, so benchmarks never pad the ignored-test count.
cargo test --release --features bench --bin gb -- \
  game_boy::tests::bench_core_throughput --exact --nocapture
```

**A test that is `#[ignore]`d should be blocked, not merely slow or not-a-test.** Everything else
goes behind a Cargo feature, so the ignored list stays a readable backlog:

| Feature | Holds |
|---|---|
| `slow-tests` / `very-slow-tests` / `full-playthrough` | Tiering by emulated game time |
| `diagnostics` | `probe_*`, `dump_fixture_states`, `capture_golden_input` — tools that print a report rather than assert. They keep `#[ignore]` on top of the gate because their pass/fail is not a signal: two legitimately end by exhausting their cycle budget *after* printing what was asked for |
| `bench` | `bench_core_throughput`, `bench_emulation_throughput` |
| `soak-tests` | `integration_tests::soak` — the fuzzer. Gated as a **module**, not with `#[ignore]`, so it never appears in the ignored list |
| `hwtests` | The mooneye MBC suite and its ROMs. 22 MB raw, committed lz4-compressed (149 KB) and decompressed in memory by the fixture, so a default build carries none of it |

With every tier feature on and the tool features off, the ignored list is **18 blocked emulator
tests** and nothing else — 9 `oam_bug` and 9 `mem_timing`/`halt_bug`. (It was 19 until the combined
`dmg_sound` suite ROM was fixed.) Each names its blocker: a plan task ID, or why it will not be
fixed. Keep it that way.

Failure artifacts — a save state and a screenshot at the point of a stall or timeout — land in
`target/test-artifacts/`, not the repo root.

### Finding jams: `soak`, and `stalls` beside it

⚠️ **`full_playthrough` proves one route still works; it cannot find a jam off that route.** The
scripted policy never chooses to walk into a PC, or into grass with nothing in it, or to pick a move
the game will refuse — so none of those were reachable by any test in the suite, and all of them
wedged the deployed run instead. `soak` is the answer: hours of `RandomPolicy`, which explores the
agent's state machine far more widely than any route.

It watches `PokemonAgent::since_last_policy_poll` — the **same** value W9's watchdog reads — so it
fails exactly when a deployed `LlmPolicy` would have its watchdog fire. One definition of stuck.

⚠️ **It deliberately does *not* apply `FAST_FIXTURE_OPTIONS`**, unlike `TestFixture`. `gb serve` runs
on the cartridge's own defaults — `InitOptions` sets `TEXT_DELAY_MEDIUM` with battle animations *on* —
and the soak exists to reproduce the deployment, not to be cheap. That is not a detail: the no-PP jam
was a race with the character-by-character text renderer, and fast text may well not reproduce it.

⚠️ **`GB_SOAK_LIMIT_SECS` is how you find the *next* one.** The default is the watchdog's 300 s
because that is the number production cares about, but seed 1's worst healthy stretch is **62 s**
over the full five hours — so a near-miss can hide comfortably under the default for a long time.
Running at `GB_SOAK_LIMIT_SECS=120` trips on anything twice as quiet as normal, and that is how the
pacing budget was found: 182 s of silence in Viridian Forest that turned out to be
`PACING_BUDGET_TICKS` running to the end on the rarest grass in the game (8/256), not a jam at all.
⚠️ **A budget that bounds silence is not sized to guarantee success** — giving up just means the
policy gets asked again, and the first version of that constant was three times too generous because
it was sized to guarantee an encounter.

⚠️ **It is seeded (`GB_SOAK_SEED`, default 1) and must stay that way.** The first runs each failed
somewhere different, which is worse than useless: a failure that vanishes when you go back to look at
it cannot verify its own fix, and CI would flake. Seed 1 is the one that must stay green; vary the
seed to hunt.

**Every jam it finds gets promoted to `integration_tests::stalls`**, in the *default* tier: the save
state at the moment the agent went quiet, replayed against a fresh agent, about a second each. That
is what makes the fix loop tolerable — the difference between a 4½-minute reproduction and a
one-second one. ⚠️ **Not every stall survives the trip**, because the save state holds the emulator
and not the agent: a jam the game's own screen re-creates reproduces perfectly, a jam that lived in
the agent's own state (an `OverworldMovement` route) does not. Watch a new case go red before
committing it, or it may be asserting nothing.

The states it has caught all had the same shape — a driver waiting for something that had stopped
coming, pressing buttons in silence. Two traps in fixing them, both paid for twice:

- ⚠️ **A counter outside the variant is reset by `set_state`.** `HealingActive` and `WaitingForMenu`
  rebuild themselves every tick with a `press`/toggle field flipped, so `set_state` sees a *new*
  state and zeroes anything counting from `PokemonAgent`. The first bound on each silently never
  fired. `OverworldMovement` is the one state where the agent-level `state_ticks` works, because it
  does not rebuild itself.
- ⚠️ **The branch that detects a problem is not always the branch that presses the wrong button.**
  `WaitingForMenu`'s `MoveList` arm had handled a spent move with B since an earlier hours-long
  wedge, and it still wedged — because the `screen.contains` check above it returns first while the
  message is up, and the *text reader* (in the `None` arm) was the thing mashing A. A fix has to sit
  above every branch that can press.

### ⚠️ Why `full_playthrough` is not optional

The leg tests each start from a committed fixture, so they prove the legs *individually*; only
`full_playthrough` proves they still **compose**, and the two come apart in ways nothing else
catches:

- **A leg test can be green for a reason the mainline does not give it.** `run_leg` keeps stepping
  after the queue empties until the effect lands, so a leg whose `Interact` pops before its
  conversation still passes — while `complete_game_steps` walks straight on without the item. That is
  exactly how the Poké Flute broke. `run_leg` now prints a ⚠️ when its post-exhaustion wait is long;
  **treat that warning as a failure in waiting.**
- **A fixture pins a party and a bag; the mainline earns them.** A leg seeded with 20 Hyper Potions
  says nothing about whether the run that reaches it can afford them.
- **Anything that changes frame timing re-rolls the RNG stream** (see `with_original_battle_timing`),
  and only a full run crosses every route that stream feeds.

Because it is opt-in and slow, it rotted once already: it sat broken while its own doc comment, this
file and the plan all claimed it played to all 8 badges. When it fails it now reports how far it got
(`completed 488/516 policy steps (94%)`) and drops its artifacts;
`playthrough::probe_resume_playthrough` replays from there in seconds instead of re-running the 20
minutes up to the stall. **If you cannot make it pass, say so explicitly in the hand-off — do not
leave a doc comment claiming it works.**

### ⚠️ Fixtures are committed inputs

Each leg test snapshots its end state for the next leg, but the write is a no-op unless
`--features regen-fixtures` is on — otherwise every run silently changes the next run's inputs, and a
leg "fails" only because an earlier one re-saved its fixture. To regenerate after a deliberate
change, run the affected legs **in chain order**:

```bash
cargo test --release --features slow-tests,regen-fixtures --bin gb -- can_clear_ss_anne --exact
```

### Benchmarking

⚠️ **Do not trust a single benchmark reading on this machine.** It has fast and slow states ~15%
apart — the same unmodified binary has measured `cpu_instrs` at 43.5× and 53.2× twenty minutes
apart. Compare only adjacent paired runs of the two builds, **alternate which one runs first**, and
report both orders.

**`perf` works and needs no `sudo`** (`perf_event_paranoid` is 2). Build with
`RUSTFLAGS="-C debuginfo=2"` into a scratch `CARGO_TARGET_DIR`, then drive the benchmark with
`BENCH_FRAMES=40000 BENCH_ONLY=pokemon` so there is enough wall clock to sample and only one workload
in the profile. ⚠️ Watch for sampling skid: a hot instruction is often paying for the *load* feeding
it, not for itself — that one cost an hour.

### Test ROMs and the resampler

`src/roms/` needs no pokered submodule. `cgb-acid2` **ships its own reference image**, so nothing in
it was promoted from `gb`'s own output; its README pins the 5-bit to 8-bit colour expansion as
`(c << 3) | (c >> 2)` — the plain widening, **not** a colour-correction curve — which is what
`LcdColor::from_rgb555` implements. ⚠️ Adopting gambatte's `gbcToRgb32` correction instead would
break the comparison.

`src/audio/blip/tests.rs` checks the resampler two independent ways, and they fail differently.
**Golden vectors** are bit-exact comparisons against the original C++ (Blip_Buffer ships no test
suite of its own, only interactive SDL demos); the fixtures in `src/audio/data/blip_*.bin` come from
linking the vendored library in `tools/blip-golden/`. Regenerate only after a *deliberate* change to
the algorithm or its parameters:

```bash
# 1. only if the realistic-signal input needs refreshing (writes src/audio/data/apu_capture_in.bin)
cargo test --release --bin gb -- audio::reference::tests::capture_golden_input --exact --ignored
# 2. always — reads apu_capture_in.bin, writes the other src/audio/data/blip_*.bin
tools/blip-golden/build.sh
```

⚠️ The goldens are pinned to `GOLDEN_TREBLE_DB` in the test module, deliberately *not* to
`blip::DEFAULT_TREBLE_DB` — tone is a taste knob, so changing what the emulator ships must not
invalidate the port's correctness fixtures. **Invariants** are the real regression net and need no
C++ toolchain: every phase's taps summing to `kernel_unit`, a step depositing exactly its own
amplitude of DC, zero sample-count drift over ten emulated minutes, no aliasing on a 15 kHz square,
and surviving a minute of emulation with no audio consumer at all (which is what the headless
integration tests do).

There is deliberately **no WAV "ear check"** any more — it was a listening aid rather than an
assertion, and was removed with `src/audio/wav.rs` rather than left in the ignored list looking like
a test.

**Fast-forward.** The number keys `1`–`5` in the SDL UI scale emulation speed, and `render.rs`
mirrors that into `Audio::set_emulation_speed` so the resampler scales its *source clock* to match.
Without it a sped-up emulator simply produces audio faster than the device drains it and the queue
backs up. The speed is derived from `cycle_duration`, not from the key pressed, so it tracks what the
emulator actually targets — `REALTIME_CYCLE_DURATION / 5` truncates to 190 ns, which is 5.016×.

## Shipping it

⚠️ **The cartridge is stage 1 of the Dockerfile, not an input**, and **the sha1 check that ends that
stage is load-bearing**: every committed fixture and every generated symbol is pinned to those exact
bytes, so a ROM that merely assembles is a different game and would fail somewhere deep in the agent
instead of at the build. `roms.sha1` is upstream's own manifest.

⚠️ **`.dockerignore` must exclude the host's pokered artifacts with `**`.** `pokered/*.o` leaves
`pokered/gfx/pics_red.o` in the context, and a stale object file from a *newer* rgbds stops the build
dead (`Unsupported object file … expected revision 12, got 13`). None of what it excludes is tracked;
every one is a `make` output.

⚠️ **`CMD` is exec form so `gb` is PID 1 and receives SIGTERM itself** — that signal is what
checkpoints the run. A shell in between means `docker stop` loses everything since the last periodic
checkpoint.

**CI** (`.github/workflows/container.yml`) builds the image, smoke-tests the running container, and
only then pushes it to ghcr.io, tagged `latest` and the commit. ⚠️ The push steps are main-only: a
fork PR's `GITHUB_TOKEN` is read-only whatever the workflow's `permissions:` asks for.

⚠️ **In `k8s/`, everything unusual is the same fact — a run directory has exactly one writer.** One
replica, `strategy: Recreate`, a PVC rather than an `emptyDir`, and a 30 s grace period so the
SIGTERM checkpoint lands. There is also deliberately no CPU limit: the emulator thread is not
event-driven, and a CFS quota shows up as the game running below real time rather than as anything
that looks like a resource problem. The liveness probe proves the HTTP server only — `healthz` is
axum's and knows nothing about the emulator thread; the wedged-run case is the in-process watchdog.

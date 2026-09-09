# CLAUDE.md

@README.md

**The README is imported above; do not re-read it and do not repeat it here.** It has what the
project is, the `src/` tree, the policy/agent model, the run directory, the endpoints, the
environment block, the build and the deployment.

What the README leaves out (the invariants, the traps and the test workflows, nearly every one of
which was learned by breaking something) is in nine short documents under `docs/`, one per area,
indexed below. **They are not loaded automatically.** Read the one for the area you are about to
touch, before touching it. Each is a list of rules, and each rule points at the comment in the code
that carries the full argument, so the code stays the source of truth and the doc stays short.

Three docs below are plans rather than rule-indexes. `docs/llm-web-playthrough-plan.md` (W0–W9, all
done) is the design one; where a comment attributes a number to "Phase C" or "Phase D", that is
history, not a file you can open. `docs/deployed-run-defects.md` is the evidence one: what
the deployed run of 2026-09-02 walked into, six root causes and eleven work items, **all of them
shipped**. Read it before touching routing, map connections, `use_field_move` targets or the plan
tools — every item carries the save state or the fixture that reproduces it, and the rules they left
behind are indexed from [pokemon-agent](docs/pokemon-agent.md) and
[llm-turn-loop](docs/llm-turn-loop.md). `docs/coverage-plan.md` is the third and the only one that is
still ahead of the code: every action in the game taken once through the deployed `LlmPolicy`
against a mock endpoint, with a verdict on each and every defect fixed, until a sweep of Kanto comes
back clean. It was rewritten on 2026-09-09 as **a numbered step list (§4), from the red tier of today
to the fixpoint**; the harness, the cheats, the oracle and the walk are built, and §2 is the baseline
sweep of that same day — 153 maps of 248, 106 defects and 41 silences, and 82% of
every turn it took spent in Route 16's gate. Steps 1 to 5 are those; **steps 1 to 5 are done — the
silences are 0, all 106 defects are closed, the gate is closed (by a cut tree that regrows, not by
the scoring change step 4 predicted), and the union has gone from 159 maps to 186 and 1 410 ids to
1 813, with §2.1 the state to measure against now. What a sweep finds is that every start's bag is
full and refuses the key items that gate the S.S. Anne and every `Fish` row — step 6's first job —
and that five of the eight starts no longer reach anything the others do not**. ⚠️ **§3 is the rules and §6.2 is
how to read a number** — the walk's totals are a coin flip between several different walks, and CPU
load is an input to it, so never build while measuring; ⚠️ **§6.1's `TMPDIR` is not decoration**,
because a walk's run directory defaults into `/tmp` and writes hundreds of megabytes of RAM there.
§7 is the record: the faults the sweeps found and the fourteen places the plan was wrong, and **§7.3
maps the old section numbers that code comments still cite** onto where each argument lives now. Read it before
touching the soak tier, `postgame/debug.rs`, `postgame/fishing.rs`, `integration_tests/llm*.rs` or
anything under `integration_tests/{cheats,coverage,godmode}.rs`.

## Rules of the road

- **Always `--release`.** The integration tests emulate every frame and are unusably slow in debug.
  `cargo test --release` is the default tier, about 40 s on a warm build; every other command is in
  [docs/test-suite.md](docs/test-suite.md), and several of them are wrong in ways that pass.
- The crate has **no lib target**: it is `--bin gb`, never `--lib`.
- Agent and policy debugging goes to stdout, so add `--nocapture` when you care about it.
- **Run `full_playthrough` after every major work item and before pushing.** The leg tier is not a
  substitute; the test-suite doc says why.
- **No em dashes in the strings the *agent* generates**: `AgentEvent`'s `Display`, `MetaTile`'s, a
  `Notice`, `learnset::teach_refusal`. Those go to the page as well as to the model and are assembled
  a fragment at a time, where a dash reads as punctuation the writer did not choose. The rule is
  deliberately this narrow: the prompt, the tool descriptions and every action-menu row use em
  dashes by design, so a wider rule would be one the codebase breaks on purpose. Punctuation in this
  file, in `docs/` and in code comments is a different audience again.

## Where the rest of it lives

| Doc | What it holds | Read before |
|---|---|---|
| [emulator-core](docs/emulator-core.md) | save-state format and the committed fixtures, mapper bank registers, DMG state in a CGB, the RTC source, the `#[inline(never)]` hot path, the `MachineCycles` overflow | `src/{mmu,mbc,ppu,savestate,schedule,cycles,game_boy}.rs`, adding or reordering a serialised field, adding a file to `src/pokemon/data/` |
| [emulator-performance](docs/emulator-performance.md) | where the emulator's time actually goes, the profiling recipe, the ceilings measured by ablation for the PPU and the APU, the ranked list of what to do about them, the three that have been taken, and the five that measured worse or made no difference at all | optimising `src/{ppu,core,opcode,mmu}.rs` or `src/audio/`, or building a rig to measure them |
| [pokemon-agent](docs/pokemon-agent.md) | the agent loop and the watchdog, the closed loops A-only input walks into, the scripted policy's rules, screen versus RAM, the prose the model and the page read, the text reader, the SPA's fold | `src/pokemon/{agent,policy,text,tile_map,actions}.rs`, `AgentEvent` or any `Display` it goes through, `web/src/useEventStream.ts`, `Conversation.tsx` |
| [llm-turn-loop](docs/llm-turn-loop.md) | the append-only history and the prompt cache, the tool catalogue, the action menu and its ids, chaining, the battle script and its sandbox, the battle report, the plan, the system prompt, the wire, the park on a spent quota, compaction | anything under `src/llm/`, `src/pokemon/llm_policy.rs`, any change to what the model is sent |
| [web-streams](docs/web-streams.md) | `/api/video`'s block-delta codec and `/api/audio`'s Opus stream, both ends: the codec invariants, deflate per connection versus never, 48 kHz against `opus-rs`, the browser's jitter buffer and drift trim | `src/web/{video,audio}*`, `web/src/{stream,video,audio}.ts` |
| [rom-graphics](docs/rom-graphics.md) | bank windowing, tile order, sprite facing and OAM layout, the font, palettes, and the map picture `read_map` answers with | `src/pokemon/{rom_gfx,badge_gfx,mon_gfx,map_gfx,font}.rs`, `src/web/sprites.rs`, `src/llm/map_image.rs` |
| [run-lifecycle](docs/run-lifecycle.md) | the one channel from HTTP back into the emulator, naming a new game by RAM write, one writer per run directory, detecting the Hall of Fame and filing it, figures across resumes | `src/run/`, `host.rs`'s new-run and completion seams, `/reset-game`, `POST /api/new-run` |
| [test-suite](docs/test-suite.md) | every test command and what each tier costs, the soak/stalls jam hunt, why `full_playthrough` is not optional, the fixture chain and how to regenerate it, benchmarking on this machine, the blip goldens | running anything but `cargo test --release`, regenerating a fixture, adding a test |
| [build-and-ship](docs/build-and-ship.md) | rgbds and the pokered symbols, `web/dist`, the pnpm cooldown, the Dockerfile stages and the sha1 check, PID 1 and the non-graceful shutdown, the build stamp, CI, `k8s/` | a build that fails before it reaches Rust, the `Dockerfile`, `.dockerignore`, `.github/workflows`, `k8s/` |

**A new invariant goes first into a comment on the code it constrains, and then as one line in the
doc for its area pointing at it.** The docs are indexes, not the argument. Nothing goes above this
table unless it genuinely holds everywhere.

# CLAUDE.md

@README.md

**The README is imported above — do not re-read it and do not repeat it here.** It has what the
project is, the `src/` tree, the policy/agent model, the run directory, the endpoints, the
environment block, the build and the deployment.

What the README leaves out — the invariants, the traps and the test workflows, nearly every one of
which was learned by breaking something — is **not in this file either**. It is in eight repo-local
skills under `.claude/skills/`, one per area of the codebase, listed below. This file is only the
rules that hold everywhere and the index that says which skill to load.

The only surviving design doc is `docs/llm-web-playthrough-plan.md` (W0–W9, all done). Where a
number in one of those skills is attributed to "Phase C" or "Phase D", that is history, not a file
you can open.

## Rules of the road

- **Always `--release`.** The integration tests emulate every frame and are unusably slow in debug.
  `cargo test --release` is the default tier, ~20 s; every other command in this repo is in the
  `test-suite` skill, and several of them are wrong in ways that pass.
- The crate has **no lib target** — it is `--bin gb`, never `--lib`.
- Agent and policy debugging goes to stdout, so add `--nocapture` when you care about it.
- **Run `full_playthrough` after every major work item and before pushing.** The leg tier is not a
  substitute; the `test-suite` skill has the command and the argument.
- **No em dashes in anything a viewer or a model reads** — see the `Display` ⚠️ in the
  `pokemon-agent` skill. The punctuation in *this* file, in the skills and in code comments is a
  different audience.

## Where the rest of it lives

Load the skill for the area **before** touching it, not after something fails. Each one opens with
the same warning this file used to carry: nearly every ⚠️ in it is a bug that shipped.

| Skill | What it holds | Load it before |
|---|---|---|
| `emulator-core` | Save-state format and the committed fixtures, mapper bank registers, DMG state in a CGB, the RTC source, `#[inline(never)]` on the hot path, the `MachineCycles` overflow | `src/{mmu,mbc,ppu,savestate,schedule,cycles,game_boy}.rs`, adding or reordering a serialised field, adding a file to `src/pokemon/data/` |
| `pokemon-agent` | The closed loops A-only input walks into, screen versus RAM, the prose the model and the page read, what an abort reports, the SPA's fold of it | `src/pokemon/{agent,policy,text,tile_map,actions}.rs`, `AgentEvent` or any `Display` it goes through, `web/src/useEventStream` + `Conversation.tsx` |
| `llm-turn-loop` | The append-only history and the prompt cache, the tool catalogue and its budget, the action menu and its ids, the plan, the battle script and its sandbox, the battle report, the system prompt, the SSE wire, the park on a spent quota, compaction | anything under `src/llm/`, `src/pokemon/llm_policy.rs`, or any change to what the model is sent |
| `web-streams` | `/api/video`'s block-delta codec and `/api/audio`'s Opus stream, both ends: the four silent codec invariants, deflate per connection versus never, 48 kHz against `opus-rs`, the browser's jitter buffer and drift trim | `src/web/{video,audio}*`, `web/src/{stream,video,audio}.ts` |
| `rom-graphics` | Bank windowing, tile order, sprite facing and OAM layout, the font, palettes, and the map picture `read_map` answers with | `src/pokemon/{rom_gfx,badge_gfx,mon_gfx,map_gfx,font}.rs`, `src/web/sprites.rs`, `src/llm/map_image.rs` |
| `run-lifecycle` | The one channel from HTTP back into the emulator, naming a new game by RAM write, one writer per run directory, detecting the Hall of Fame and filing it | `src/run/`, `host.rs`'s new-run and completion seams, `/reset-game`, `POST /api/new-run` |
| `test-suite` | Every test command and what each tier costs, the soak/stalls jam hunt, why `full_playthrough` is not optional, the fixture chain and how to regenerate it, benchmarking on this machine, the blip goldens | running anything but `cargo test --release`, regenerating a fixture, adding a test |
| `build-and-ship` | rgbds and the pokered symbols, `web/dist`, the pnpm cooldown, the four Dockerfile stages and the sha1 check, PID 1 and the ungraceful shutdown, the build stamp, CI, `k8s/` | a build that fails before it reaches Rust, the `Dockerfile`, `.dockerignore`, `.github/workflows`, `k8s/` |

⚠️ **A new invariant belongs in the skill for its area, not here.** This file grew to 128 KB by being
the only place there was, which put the whole of it in front of every task; the split is what stops
that happening again. Anything that genuinely holds everywhere — a rule about the build, the
punctuation, the test tiers — is the only thing that goes above.

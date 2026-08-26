---
name: build-and-ship
description: "What the build needs before cargo (rgbds, pokered symbols, web/dist, the pnpm cooldown) and how the image ships (the four Dockerfile stages, the sha1 check, PID 1 and the non-graceful shutdown, the build stamp, CI and k8s). Load when a build fails before it reaches Rust, or when touching the Dockerfile, .dockerignore, .github/workflows or k8s/."
---

# Build inputs, and shipping it

*Extracted from `CLAUDE.md`, which holds the rules of the road and the index of these skills. The
README is imported into `CLAUDE.md` and is not repeated there or here: this file has only the
invariants and the traps, nearly every one of which was learned by breaking something.*

## Build inputs

⚠️ **pokered needs rgbds ≥ 1.0.0 and `rgbdscheck.asm` `fail`s below that** — a hard error, so an old
rgbds does not produce a wrong ROM, it produces none. The container pins 1.0.3 (`ARG RGBDS_VERSION`).

⚠️ **The symbol names are upstream's to change.** The 2026-08 bump renamed pokered's "hidden object"
and "missable object" vocabulary to **hidden event** and **toggleable object** (`HiddenEventMaps`,
`HiddenEventPointers`, `wToggleableObjectFlags`). That surfaces as a compile error only because
`build.rs` emits constants for symbols that exist.

⚠️ **`web/dist` must exist for the crate to compile at all** — `rust-embed`'s derive fails if the
folder is missing. Hence the committed `web/dist/.gitkeep`, and `vite build` (which empties `dist`)
copying it back from `web/public/`. A checkout that has never run `pnpm run build` compiles and
serves a page naming the two commands to run.

⚠️ **`web/pnpm-workspace.yaml`'s `minimumReleaseAge` cooldown is checked on *every* install,
`--frozen-lockfile` included.** A lockfile pinning anything younger than the window fails with
`ERR_PNPM_NO_MATURE_MATCHING_VERSION`, so the lockfile has to be *generated* under the same number
the builds enforce. Raise the window without regenerating `pnpm-lock.yaml` and what breaks is the
container build, not the dev loop. The file says why it is 3 days. pnpm's version is pinned by
`packageManager` in `web/package.json` and activated by corepack — deliberately not in the
Dockerfile, so there is nothing there to drift from it.

## Shipping it

⚠️ **The cartridge is stage 1 of the Dockerfile, not an input**, and **the sha1 check that ends that stage is
load-bearing**: every committed fixture and every generated symbol is pinned to those exact bytes, so a ROM that
merely assembles is a different game and would fail somewhere deep in the agent instead of at the build.
`roms.sha1` is upstream's own manifest.

⚠️ **`.dockerignore` must exclude the host's pokered artifacts with `**`.** `pokered/*.o` leaves
`pokered/gfx/pics_red.o` in the context, and a stale object file from a *newer* rgbds stops the build dead
(`Unsupported object file … expected revision 12, got 13`). None of what it excludes is tracked; every one is a
`make` output.

⚠️ **`CMD` is exec form so `gb` is PID 1 and receives SIGTERM itself** — that signal is what checkpoints the run.
A shell in between means `docker stop` loses everything since the last periodic checkpoint.

⚠️ **And it must not be a *graceful* shutdown, which is what `axum::serve` invites and what this had.**
`with_graceful_shutdown` stops accepting and then waits for the requests in flight — but `/api/events`,
`/api/video` and `/api/audio` never finish by construction, each held open by its own keepalive, so what it
actually waits for is every viewer to close their tab. A rollout with one browser on the page therefore refused
new connections, kept serving the old ones, and sat there until the kubelet's 30 s grace period ran out and
SIGKILL took the checkpoint with it — the exact loss the paragraph above exists to prevent, on the exact deploy
that causes it. `serve_http` now `select!`s the accept loop against `shutdown_signal()` and drops it, and
⚠️ **that alone is not enough**: axum spawns a task per connection and they outlive the future, so
`runtime.shutdown_timeout` below it is what actually ends the streams. Every endpoint here is read-only, so a
dropped connection costs a viewer a reconnect and nothing else.

⚠️ **The build stamp (`/version`) is `ENV` in the runtime stage and must stay below the `COPY` of the binary.**
`GB_BUILD_DATE` changes on every build, so an `ARG` the cargo stage read would invalidate stage 3 every CI run —
and `type=gha` caches *layers*, not the cache mounts the cargo registry and target directory live on, so that is
a full cold `cargo build --release` each time rather than a cheap re-link. Below the binary's own layer there is
nothing but metadata. That is also why they are `std::env::var` in `src/web/version.rs` rather than `env!()`, and
why a `build.rs` git fallback was not added: it would either recompile the crate on every commit or go quietly
stale, and `null` is the honest answer for a build nobody stamped.

**CI** (`.github/workflows/container.yml`) builds the image, smoke-tests the running container, and only then
pushes it to ghcr.io, tagged `latest` and the commit. ⚠️ The push steps are main-only: a fork PR's `GITHUB_TOKEN`
is read-only whatever the workflow's `permissions:` asks for.

⚠️ **In `k8s/`, everything unusual is the same fact — a run directory has exactly one writer.** One replica,
`strategy: Recreate`, a PVC rather than an `emptyDir`, and a 30 s grace period so the SIGTERM checkpoint lands.
There is also deliberately no CPU limit: the emulator thread is not event-driven, and a CFS quota shows up as the
game running below real time rather than as anything that looks like a resource problem. The liveness probe proves
the HTTP server only — `healthz` is axum's and knows nothing about the emulator thread; the wedged-run case is the
in-process watchdog.

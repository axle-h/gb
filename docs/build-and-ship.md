# Build and ship

Read when a build fails before it reaches Rust, and before touching the `Dockerfile`,
`.dockerignore`, `.github/workflows` or `k8s/`.

## Build inputs

- pokered needs rgbds ≥ 1.0.0 and `rgbdscheck.asm` fails hard below that, so an old rgbds produces
  no ROM rather than a wrong one. The container pins `ARG RGBDS_VERSION`.
- pokered's symbol names are upstream's to change (the 2026-08 bump renamed hidden and missable
  objects to hidden events and toggleable objects). `build.rs` emits a constant per symbol, so a
  rename is a compile error, which is the point.
- `web/dist` must exist for the crate to compile: `rust-embed`'s derive fails on a missing folder.
  `web/dist/.gitkeep` is committed and `vite build` copies it back from `web/public/`. A checkout
  that never ran `pnpm run build` compiles and serves a page naming the two commands to run.
- `web/pnpm-workspace.yaml`'s `minimumReleaseAge` cooldown is checked on every install,
  `--frozen-lockfile` included. Raise it without regenerating `pnpm-lock.yaml` and the container
  build fails with `ERR_PNPM_NO_MATURE_MATCHING_VERSION` while the dev loop keeps working. The file
  says why it is 3 days. pnpm's version is `packageManager` in `web/package.json` via corepack and is
  deliberately not repeated in the Dockerfile.

## The image

- The cartridge is stage 1 of the Dockerfile, and the sha1 check against upstream's `roms.sha1` is
  load-bearing: every fixture and every generated symbol is pinned to those bytes, and a ROM that
  merely assembles would fail deep inside the agent instead of at the build.
- `.dockerignore` excludes the submodule's build outputs with `**` patterns. `pokered/*.o` alone
  leaves `pokered/gfx/pics_red.o` in the context, and a stale object from a newer rgbds stops the
  build (`Unsupported object file … expected revision 12, got 13`).
- `CMD` is exec form so `gb` is PID 1 and receives SIGTERM itself. That signal is what checkpoints
  the run; a shell in between loses everything since the last periodic checkpoint on `docker stop`.
- Shutdown must not be axum's graceful one. `/api/events`, `/api/video` and `/api/audio` never
  finish, so `with_graceful_shutdown` waits for every viewer to close their tab, the grace period
  runs out, and SIGKILL takes the checkpoint. `serve_http` selects the accept loop against
  `shutdown_signal()`, and `runtime.shutdown_timeout` below it is what actually ends the
  per-connection tasks. The argument is in `src/web/mod.rs` above `serve_http`.
- The build stamp (`GB_BUILD_DATE`, `GB_GIT_BRANCH`, `GB_GIT_SHA`) is `ENV` in the runtime stage,
  below the `COPY` of the binary, and read with `std::env::var` in `src/web/version.rs` rather than
  `env!()`. An `ARG` the cargo stage read would invalidate that stage every CI run, and `type=gha`
  caches layers rather than the cargo cache mounts, so that is a full cold build each time. A
  binary nobody stamped reports `null`.
- CI (`.github/workflows/container.yml`) builds the image, smoke-tests the running container, then
  pushes `latest` and the commit tag. The push steps are main-only: a fork PR's `GITHUB_TOKEN` is
  read-only whatever `permissions:` asks for.
- `k8s/` is one replica, `strategy: Recreate`, a PVC, a 30 s grace period and no CPU limit, all
  for one reason: a run directory has exactly one writer, and a CFS quota shows up as the game
  running slow rather than as a resource error. The liveness probe proves the HTTP server only; a
  wedged run is the in-process watchdog's job.

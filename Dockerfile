# syntax=docker/dockerfile:1.7
# ⚠️ That line is a parser directive and only counts as one if it is the *first* line in the file.
#
# gb — one container, one process, one emulator. **W8** of `docs/llm-web-playthrough-plan.md`.
#
#   docker build -t gb .
#   docker run -d --name gb -p 8080:8080 -v gb-runs:/runs \
#              -e OPENAI_API_KEY=sk-… -e GB_MODEL=… gb
#
# Four stages, in dependency order, and the first two are the ones that are not obvious:
#
#   1. `rom`   — rgbds, then `pokered/pokered.gbc`. The ROM is `include_bytes!`'d at compile time
#                (`src/pokemon/roms.rs`) but is *gitignored*, so nothing in the build context has
#                it and the crate does not compile without it.
#   2. `web`   — `web/dist`, which `rust-embed` bakes into the binary. Same story: it must exist
#                before cargo runs, or the derive fails outright.
#   3. `build` — the crate, without SDL: `gb serve` never opens a window, and a server image
#                should not need `libsdl2` to link against.
#   4. runtime — the binary, a volume for the run directory, and nothing else.


##############################################################################
# stage 1 — the cartridge
##############################################################################
# Bump these two together; the checksum is version-specific and is what stops a silent substitution
# upstream. pokered wants rgbds 1.0.0 *or newer* — `pokered/rgbdscheck.asm` `fail`s the assembly
# outright below that, it is not a warning — and 1.0.3 is the version its own INSTALL.md names, so
# that is what we pin: a newer major release is free to change codegen, and this ROM's bytes are
# load-bearing (see the sha1 check below).
FROM debian:bookworm-slim AS rom
ARG RGBDS_VERSION=1.0.3
ARG RGBDS_SHA256=97b523435f7da0b6d2a58daff447bb2c8280895c3f49eb4e63e5df8da63dd64d

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential bison libpng-dev pkg-config make curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# rgbds from source rather than the release's prebuilt tarball: the prebuilt one is x86-64 only,
# and this stage is the only reason the image would not build on an arm64 host.
WORKDIR /rgbds
RUN curl -fsSL -o rgbds.tar.gz \
        "https://github.com/gbdev/rgbds/releases/download/v${RGBDS_VERSION}/rgbds-source.tar.gz" \
    && echo "${RGBDS_SHA256}  rgbds.tar.gz" | sha256sum -c - \
    && tar xzf rgbds.tar.gz --strip-components=1 \
    && make -j"$(nproc)" \
    && make install \
    && rgbasm --version

WORKDIR /pokered
COPY pokered/ ./

# `pokered/` is a git submodule. An uninitialised one is an empty directory, and the error you get
# from `make` for that ("No rule to make target 'main.asm'") says nothing about why.
RUN test -f main.asm || { \
        echo "the pokered submodule is empty — run: git submodule update --init --recursive" >&2; \
        exit 1; \
    }

# ⚠️ **The sha1 check is the point of this stage, not a formality.** All 91 committed fixtures in
# `src/pokemon/data/`, every symbol `build.rs` generates, and every address the Pokémon layer reads
# are pinned to *these* bytes. A ROM that merely assembles is not good enough — one built by a
# different rgbds is a different game, and it would fail somewhere deep in the agent rather than
# here. `roms.sha1` is upstream's own manifest; the grep takes the one line for the ROM we build.
RUN make -j"$(nproc)" pokered.gbc \
    && grep ' \*pokered\.gbc$' roms.sha1 | sha1sum -c - \
    && test -s pokered.sym


##############################################################################
# stage 2 — the SPA
##############################################################################
# The install is on its own layer so a change to the React source does not re-resolve the dependency
# tree, and the pnpm store is a cache mount for the same reason the cargo registry is one in stage 3.
FROM node:22-alpine AS web
# ⚠️ corepack is what pins the pnpm version, from `packageManager` in `web/package.json` — there is
# no pnpm version named in this file to drift from it. It is bundled with node 22; newer Node images
# unbundle it, so a base-image bump lands here as "corepack: not found" and wants `npm i -g pnpm@…`.
ENV PNPM_HOME=/pnpm PATH=/pnpm:$PATH
RUN corepack enable
WORKDIR /web
# ⚠️ `pnpm-workspace.yaml` carries the `minimumReleaseAge` cooldown and must arrive *before* the
# install, not with the source. Without it pnpm falls back to its own default and the image is built
# under a policy that is not the one the repo states.
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN --mount=type=cache,id=pnpm,target=/pnpm/store pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm run build


##############################################################################
# stage 3 — the binary
##############################################################################
FROM rust:1-bookworm AS build
WORKDIR /src

# Exactly what the compile reads, and nothing else: the manifest, the build script (which parses
# `pokered/pokered.sym`), the crate source, the ROM, and the SPA. Keeping it to this list means an
# edit to a doc or a script does not invalidate the cargo layer.
COPY Cargo.toml Cargo.lock build.rs ./
COPY src/ ./src/
COPY --from=rom /pokered/pokered.gbc /pokered/pokered.sym ./pokered/
COPY --from=web /web/dist ./web/dist

# ⚠️ The cache mounts are why the binary is copied out **inside this RUN**: a cache mount is not
# part of the image, so `/src/target` does not exist in any later layer.
#
# `--no-default-features --features llm` is the container build (`Cargo.toml` documents it):
# `llm` implies `web`, and dropping `sdl` drops the `libsdl2` link dependency entirely.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --no-default-features --features llm \
    && cp target/release/gb /usr/local/bin/gb


##############################################################################
# stage 4 — the image
##############################################################################
FROM debian:bookworm-slim

# ⚠️ Not decoration: GHCR reads `image.source` to link the published package to this repository,
# which is what makes the package page show this README — and what makes the package's
# "inherit access from repository" setting mean anything. Without it the package is an orphan owned
# by the user, and the CI push still works but nothing connects the two.
# No `image.licenses` here on purpose — the repo has no top-level LICENSE, and this label would be
# the first place one got asserted.
LABEL org.opencontainers.image.source="https://github.com/axle-h/gb" \
      org.opencontainers.image.description="A Game Boy emulator playing Pokémon Red, driven by an LLM over text"

# `curl` is here for the HEALTHCHECK. The LLM client needs no CA bundle — ureq is built against
# `webpki-roots`, so its trust store is compiled into the binary — but curl expects one.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /home/gb --uid 10001 gb \
    && mkdir -p /runs && chown gb:gb /runs

COPY --from=build /usr/local/bin/gb /usr/local/bin/gb

# ── which build this is ──────────────────────────────────────────────────────────────────────────
#
# `GET /version` serves these three beside the crate version, and `gb serve` prints them on the way
# up. CI fills them in (`.github/workflows/container.yml`); a local `docker build` leaves them empty,
# which reads as `null` rather than as a wrong answer.
#
# ⚠️ **Here, and not `env!()` in the binary, because of the build cache.** `GB_BUILD_DATE` changes on
# every build, so an ARG the cargo layer consumed would invalidate stage 3 every single run — and
# `type=gha` caches layers, not the cache mounts the cargo registry and target directory live on, so
# that is a full cold `cargo build --release` each time. Below the `COPY` above, the only layers left
# are metadata and rebuilding them is free.
#
# ⚠️ **And they must stay below it**: an ARG referenced above the COPY puts the timestamp back into
# the image's own layer graph and costs the binary layer instead.
ARG GB_BUILD_DATE=""
ARG GB_GIT_BRANCH=""
ARG GB_GIT_SHA=""
ENV GB_BUILD_DATE=$GB_BUILD_DATE \
    GB_GIT_BRANCH=$GB_GIT_BRANCH \
    GB_GIT_SHA=$GB_GIT_SHA
# The same facts where `docker inspect` and GHCR's package page look for them. ⚠️ `revision` is the
# **full** commit, not `$GB_GIT_SHA`: the label is not the place to abbreviate, and the full hash is
# what `$IMAGE:$GITHUB_SHA` is tagged with. No `image.version` label — OCI means the packaged
# software's version by that, so it would be the crate's, and one more build arg to carry a constant
# that is already in the binary and in `/version` is not worth the layer.
ARG GB_GIT_REVISION=""
LABEL org.opencontainers.image.revision="$GB_GIT_REVISION" \
      org.opencontainers.image.created="$GB_BUILD_DATE"

# The run directory is the whole of a run's state — `meta.json`, `state.gbst`, `sram.bin`,
# `transcript.jsonl` and the model's `memories/` and `todo.json` (`src/run/mod.rs`). Mount it and a
# run survives the container being replaced, not merely restarted. Owned by `gb` above, so an
# anonymous volume inherits the ownership rather than arriving root-owned.
ENV GB_RUN_DIR=/runs \
    GB_PORT=8080
VOLUME /runs
EXPOSE 8080

USER gb
WORKDIR /runs

HEALTHCHECK --interval=30s --timeout=3s --start-period=20s --retries=3 \
    CMD curl -fsS "http://127.0.0.1:${GB_PORT}/api/healthz" >/dev/null || exit 1

# ⚠️ Exec form, so `gb` is PID 1 and receives `docker stop`'s SIGTERM itself. The handler is what
# checkpoints the run on the way out (W7); with a shell in between, the signal goes to the shell,
# nothing is written, and up to a minute of play is lost to the next start. `gb serve` **resumes**
# the newest run under `$GB_RUN_DIR` by default — `--new-run` is how you start the game over.
CMD ["gb", "serve"]

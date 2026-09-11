#!/usr/bin/env bash
# Regenerate the Blip_Buffer golden vectors in gb/src/audio/data/.
#
# Run from the repo root. Never invoked by cargo — the C++ here is a reference implementation used
# once to produce fixtures, not a build dependency of the emulator.
#
#   vendor/Blip_Buffer/gen_golden/build.sh
#
# Depends on gb/src/audio/data/apu_capture_in.bin, which comes from the Rust side:
#   cargo test --release -p gb --features slow-tests -- capture_golden_input --exact --ignored
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
cd "$here/../../../gb"   # the gb crate root: gen_golden reads and writes src/audio/data

if [[ ! -f src/audio/data/apu_capture_in.bin ]]; then
    echo "missing gb/src/audio/data/apu_capture_in.bin — generate it first:" >&2
    echo "  cargo test --release -p gb --features slow-tests -- capture_golden_input --exact --ignored" >&2
    exit 1
fi

out=../target/blip-golden
mkdir -p "$out" src/audio/data

# -DNDEBUG would drop the library's internal assertions; keep them on so a fixture that overruns the
# buffer fails here rather than producing a quietly wrong golden.
g++ -O2 -Wall -o "$out/gen_golden" \
    "$here/gen_golden.cpp" \
    "$here/../Blip_Buffer.cpp"

"$out/gen_golden"

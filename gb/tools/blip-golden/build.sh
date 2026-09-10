#!/usr/bin/env bash
# Regenerate the Blip_Buffer golden vectors in src/audio/data/.
#
# Run from the repo root. Never invoked by cargo — the C++ here is a reference implementation used
# once to produce fixtures, not a build dependency of the emulator.
#
#   tools/blip-golden/build.sh
#
# Depends on src/audio/data/apu_capture_in.bin, which comes from the Rust side:
#   cargo test --release --bin gb -- audio::reference::tests::capture_golden_input --exact --ignored
set -euo pipefail

cd "$(dirname "$0")/../.."

if [[ ! -f src/audio/data/apu_capture_in.bin ]]; then
    echo "missing src/audio/data/apu_capture_in.bin — generate it first:" >&2
    echo "  cargo test --release --bin gb -- audio::reference::tests::capture_golden_input --exact --ignored" >&2
    exit 1
fi

out=target/blip-golden
mkdir -p "$out" src/audio/data

# -DNDEBUG would drop the library's internal assertions; keep them on so a fixture that overruns the
# buffer fails here rather than producing a quietly wrong golden.
g++ -O2 -Wall -o "$out/gen_golden" \
    tools/blip-golden/gen_golden.cpp \
    tools/blip-golden/vendor/Blip_Buffer.cpp

"$out/gen_golden"

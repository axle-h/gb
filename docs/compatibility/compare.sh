#!/usr/bin/env bash
# Paired, order-alternated benchmark comparison — the protocol §2.5 of the plan demands, because
# this machine has fast and slow states ~15% apart.
#
#   compare.sh <baseline-binary> <candidate-binary> [rounds]
set -euo pipefail
A="$1"; B="$2"; ROUNDS="${3:-3}"

run() {
  "$1" --exact game_boy::tests::bench_core_throughput --nocapture 2>/dev/null \
    | awk '/^(pokemon-red|cpu_instrs|dmg-acid2)/ { r=$(NF-2); gsub(/x$/,"",r); printf "%-12s %s\n", $1, r }'
}

for i in $(seq 1 "$ROUNDS"); do
  if (( i % 2 == 1 )); then
    echo "--- round $i (baseline first)"
    run "$A" | sed 's/^/BASE /'
    run "$B" | sed 's/^/CAND /'
  else
    echo "--- round $i (candidate first)"
    run "$B" | sed 's/^/CAND /'
    run "$A" | sed 's/^/BASE /'
  fi
done

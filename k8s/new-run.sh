#!/usr/bin/env bash
# Start a new game on the deployed instance, in place: POST /api/new-run via a port-forward,
# authenticated with GB_ADMIN_TOKEN read straight out of the `gb` Secret. The current run is
# checkpointed and left complete on the volume; every open page follows the new run by itself.
set -euo pipefail

NS=${NS:-gb}
LOCAL_PORT=${LOCAL_PORT:-18080}

token=$(kubectl -n "$NS" get secret gb -o jsonpath='{.data.GB_ADMIN_TOKEN}' | base64 -d)
if [[ -z "$token" ]]; then
  echo "GB_ADMIN_TOKEN is unset or blank in secret/gb: the endpoint 404s until it is set." >&2
  exit 1
fi

kubectl -n "$NS" port-forward deploy/gb "$LOCAL_PORT:8080" >/dev/null 2>&1 &
pf=$!
trap 'kill "$pf" 2>/dev/null' EXIT

# Wait for the forward to accept connections.
for _ in $(seq 1 50); do
  curl -fsS "http://127.0.0.1:$LOCAL_PORT/api/healthz" >/dev/null 2>&1 && break
  sleep 0.2
done

curl -fsS -X POST -H "X-GB-Token: $token" "http://127.0.0.1:$LOCAL_PORT/api/new-run"
echo

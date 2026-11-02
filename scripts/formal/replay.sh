#!/usr/bin/env bash
# Checks the settlement model and replays its traces against the rule core (docs/properties.md).
#   scripts/formal/replay.sh [traces] [steps]
# Needs quint (npm i -g @informalsystems/quint@0.33.1); `verify` also needs Java 21 for Apalache.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
N="${1:-600}"
STEPS="${2:-45}"
OUT="$ROOT/target/quint-traces"
cd "$ROOT/formal"
quint typecheck antibody.qnt
quint test antibody.qnt --main=antibody
quint run antibody.qnt --invariant=all_properties --max-samples=20000 --max-steps=50
rm -rf "$OUT" && mkdir -p "$OUT"
quint run antibody.qnt --max-samples="$N" --max-steps="$STEPS" --mbt --out-itf="$OUT/t_{seq}.itf.json" --n-traces="$N" >/dev/null
cd "$ROOT"
QUINT_TRACES="$OUT" cargo test --release -p antibody-rules --test quint_replay -- --nocapture

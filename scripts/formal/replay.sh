#!/usr/bin/env bash
# Checks the settlement model and replays its traces against the rule core (docs/properties.md).
#   scripts/formal/replay.sh [traces] [steps]      (QUINT_SEED=<seed> to generate a run's traces again)
# Needs quint (npm i -g @informalsystems/quint@0.33.1); `verify` also needs Java 21 for Apalache.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
N="${1:-600}"
STEPS="${2:-60}"
OUT="$ROOT/target/quint-traces"
cd "$ROOT/formal"
quint typecheck antibody.qnt
quint test antibody.qnt --main=antibody
quint run antibody.qnt --invariant=all_properties --max-samples=20000 --max-steps=50
rm -rf "$OUT" && mkdir -p "$OUT"
# The traces' seed, printed, so a run that diverges can be replayed: QUINT_SEED=<seed> scripts/formal/replay.sh
SEED="${QUINT_SEED:-0x$(od -An -N6 -tx1 /dev/urandom | tr -d ' 
')}"
echo "traces: $N of up to $STEPS steps, seed $SEED (QUINT_SEED=$SEED scripts/formal/replay.sh $N $STEPS generates the same)"
quint run antibody.qnt --max-samples="$N" --max-steps="$STEPS" --mbt --out-itf="$OUT/t_{seq}.itf.json" --n-traces="$N" --seed="$SEED" >/dev/null
cd "$ROOT"
QUINT_TRACES="$OUT" cargo test --release -p antibody-rules --test quint_replay -- --nocapture

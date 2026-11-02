#!/usr/bin/env bash
# Builds the three benchmark witnesses sequentially (read-only RPC). Logs to measurements/.
cd "$(dirname "$0")/.."
run() {
  local name=$1 tx=$2
  echo "=== $name $tx $(date -u +%FT%TZ)"
  /usr/bin/time -v true >/dev/null 2>&1
  local t0=$(date +%s)
  ./target/release/antibody-witness replay --tx "$tx" --out "witnesses/$name" --recent --min-agree 3
  echo "rc=$? wall_s=$(( $(date +%s) - t0 ))"
}
run flashloan   0x2c678eb5a234f5879f8e1424d02840d6caa7af816e59e6fd906af023c4e859d0
run liquidation 0xb0f24735e6dd5dde222b9812af0556ee1f4dc91577eb42aa022f91b6a60ff458
run aggswap     0xb90768e57a5c39d1bdb90a8cf0313913ea67b3229c7000961eade51093f0aecf

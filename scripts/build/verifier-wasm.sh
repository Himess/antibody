#!/usr/bin/env bash
# The verifier OutLayer runs (antibody-verify.wasm), built reproducibly: the toolchain rust-toolchain.toml
# pins, the locked dependencies, and every build path remapped to a fixed prefix, so the same source
# gives the same bytes in any directory and on any Linux x86_64 machine (Dockerfile.verifier pins the
# rest). Prints the sha256 a program pins as its `wasm_hash`.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
export RUSTFLAGS="--remap-path-prefix=$root=/antibody --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$rustup_home=/rustup"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/reproducible}"
rustup target add wasm32-wasip2 >/dev/null 2>&1 || true
cargo build --release --locked --target wasm32-wasip2 -p antibody-verifier --bin antibody-verify >&2
wasm="$CARGO_TARGET_DIR/wasm32-wasip2/release/antibody-verify.wasm"
echo "$(sha256sum "$wasm" | cut -d' ' -f1)  $wasm"

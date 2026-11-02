# Antibody: the one-command offline demo in a container, for any OS with Docker. No keys and no
# testnet funds. It runs near-sandbox, two anvils, and MOCK OutLayer and MOCK MPC driven by the real
# verifier, and asserts every expected outcome.
#
#   docker build --platform linux/amd64 -t antibody-demo .
#   docker run --rm --platform linux/amd64 antibody-demo
#
# near-sandbox 2.14.0 is published for Linux x86_64 (and macOS arm64), hence linux/amd64; Apple
# Silicon runs it under emulation. The toolchains come from the repository's pins:
# - rust-toolchain.toml: stable + wasm32-wasip2;
# - Makefile: Rust 1.85.0 for the NEAR contracts;
# - Foundry v1.8.5, as in CI;
# - scripts/demo/get-sandbox.sh: near-sandbox, sha256-pinned.
FROM rust:bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 curl git ca-certificates pkg-config libssl-dev clang \
 && rm -rf /var/lib/apt/lists/*

ENV PATH="/root/.foundry/bin:${PATH}" CARGO_BUILD_JOBS=4
RUN curl -sSL https://foundry.paradigm.xyz | bash && foundryup --install v1.8.5

WORKDIR /antibody
COPY . .
# Contracts (NEAR wasm + EVM), verifier (native + wasm32-wasip2), CLI and services, then near-sandbox.
RUN make build && make sandbox

# Recorded fixtures (native == wasm), contract rule tests, then the sandbox end to end.
CMD ["make", "fixtures", "contract-tests", "e2e"]

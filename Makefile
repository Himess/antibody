# Antibody — one-command offline demo (no keys, no testnet funds).
#   make demo   build everything, verify the recorded fixtures natively and in wasm, run the contract
#               tests, then run the full flow on near-sandbox and anvil with MOCK OutLayer and MOCK
#               DCAP (the real T1 checks on MOCK quotes), and print the results table.
# Linux x86_64 or macOS arm64 with Rust (rustup), Foundry and Python 3. Elsewhere: see Dockerfile.

SHELL := /bin/bash
NEAR_TOOLCHAIN := 1.85.0

.PHONY: demo build contracts verifier verifier-wasm fixtures contract-tests sandbox e2e clean

demo: build fixtures contract-tests e2e

build: contracts verifier

contracts:
	rustup toolchain install $(NEAR_TOOLCHAIN) --profile minimal --target wasm32-unknown-unknown
	cd contracts/near-antibody && cargo build --release --target wasm32-unknown-unknown \
		-p near-antibody -p antibody-tusd -p antibody-mock-outlayer -p antibody-mock-mpc -p antibody-mock-dcap -p antibody-mock-lc
	cd contracts/near-guardian && cargo build --release --target wasm32-unknown-unknown
	cd contracts/practice-vending && cargo build --release --target wasm32-unknown-unknown
	cd contracts/verdict-guard && cargo build --release --target wasm32-unknown-unknown
	cd contracts/evm && forge build

verifier:
	cargo build --release -p antibody-verifier -p antibody-witness -p antibody-runner -p antibody-cli -p antibody-services
	cargo build --release --target wasm32-wasip2 -p antibody-verifier --bin antibody-verify

# The verifier wasm as programs pin it (path-remapped, locked; prints its sha256).
verifier-wasm:
	bash scripts/build/verifier-wasm.sh

fixtures:
	python3 scripts/demo/check_fixtures.py fixtures/acme-local
	python3 scripts/demo/check_fixtures.py fixtures/acme-local/round1

contract-tests:
	cd contracts/evm && forge test
	cd contracts/near-antibody && cargo test --lib -p near-antibody -p antibody-tusd
	cd contracts/near-guardian && cargo test --lib
	cd contracts/practice-vending && cargo test --lib
	cd contracts/verdict-guard && cargo test --lib
	cargo test --release -p antibody-verifier -p antibody-cli

sandbox:
	@bash scripts/demo/get-sandbox.sh >/dev/null

e2e: sandbox
	./target/release/antibody-services demo --sandbox-bin .tools/near-sandbox

clean:
	rm -rf target/demo

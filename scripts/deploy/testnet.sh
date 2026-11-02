#!/usr/bin/env bash
# Antibody testnet deployment runbook (NEAR testnet + Ethereum Sepolia + Base Sepolia).
#
# Needs funded TESTNET keys in the git-ignored .secrets/ directory (never printed):
#   .secrets/near_guardian.json      NEAR implicit account (near-cli key format), >= 60 NEAR
#   .secrets/sepolia_deployer.json   EVM deployer, Sepolia + Base Sepolia ETH
# and archive RPCs with eth_getProof in .env: SEPOLIA_RPC, BASE_SEPOLIA_RPC.
# Every step checks whether it already ran, so the script can be re-run after a failure.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
set -a; [ -f .env ] && source .env; set +a
: "${SEPOLIA_RPC:?set SEPOLIA_RPC in .env}"
: "${BASE_SEPOLIA_RPC:?set BASE_SEPOLIA_RPC in .env}"

HQ="${HQ:-antibody-hq.testnet}"
KEYS="$ROOT/.secrets/near"
AB="$ROOT/target/release/antibody"
DEP="$ROOT/deployments/near-testnet.json"
WASM_DIR="$ROOT/contracts/near-antibody/target/wasm32-unknown-unknown/release"
GUARDIAN_WASM="$ROOT/contracts/near-guardian/target/wasm32-unknown-unknown/release/antibody_guardian.wasm"
VERIFIER_WASM="$ROOT/target/wasm32-wasip2/release/antibody-verify.wasm"
mkdir -p "$KEYS"
ab() { "$AB" --network testnet --deployments "$DEP" "$@"; }
exists() { curl -s https://rpc.testnet.fastnear.com -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"finality\":\"final\",\"account_id\":\"$1\"}}" \
    | grep -q '"amount"'; }

echo "== 1. NEAR accounts under $HQ"
if ! exists "$HQ"; then
  ab --key-file .secrets/near_guardian.json admin create-account --new-account-id "$HQ" --amount-near 45 --key-out "$KEYS/$HQ.json"
fi
for n in bounty tusd guardian acme agent-a agent-b agent-c agent-d agent-e agent-f; do
  a="$n.$HQ"
  amount=2
  case "$n" in bounty) amount=15 ;; guardian|tusd) amount=6 ;; acme) amount=3 ;; esac
  exists "$a" || ab --key-file "$KEYS/$HQ.json" admin create-account --new-account-id "$a" --amount-near "$amount" --key-out "$KEYS/$a.json"
done

echo "== 2. Guardian EVM address (Chain Signatures derivation for guardian.$HQ, path guardian/acme)"
GUARDIAN_EVM="$(cd scripts/guardian && node derive.mjs "guardian.$HQ" guardian/acme testnet \
  | python3 -c 'import sys,json; d=json.load(sys.stdin); assert d["match"], "local and on-chain derivation differ"; print(d["address_onchain"])')"
echo "guardian EVM address: $GUARDIAN_EVM"

echo "== 3. Acme Lend on Sepolia (seeded) and Base Sepolia (pause-only copy), same CREATE2 addresses"
export DEPLOYER_KEY="$(python3 -c 'import json; print(json.load(open(".secrets/sepolia_deployer.json"))["private_key"])')"
(cd contracts/evm
  [ -f deployments/11155111.json ] || GUARDIAN_ADDRESS="$GUARDIAN_EVM" SEED=true GUARDIAN_FUND_WEI=20000000000000000 \
    forge script script/DeployAcme.s.sol --rpc-url "$SEPOLIA_RPC" --broadcast --slow
  [ -f deployments/84532.json ] || GUARDIAN_ADDRESS="$GUARDIAN_EVM" SEED=false GUARDIAN_FUND_WEI=5000000000000000 \
    forge script script/DeployAcme.s.sol --rpc-url "$BASE_SEPOLIA_RPC" --broadcast --slow)
unset DEPLOYER_KEY
ACME_GUARD="$(python3 -c 'import json; print(json.load(open("contracts/evm/deployments/11155111.json"))["AcmeGuard"])')"

echo "== 4. Verifier wasm on a GitHub release (OutLayer WasmUrl)"
WASM_HASH="$(sha256sum "$VERIFIER_WASM" | cut -d' ' -f1)"
TAG="verifier-${WASM_HASH:0:12}"
gh release view "$TAG" >/dev/null 2>&1 || gh release create "$TAG" "$VERIFIER_WASM" --title "verifier $WASM_HASH" \
  --notes "antibody-verify.wasm (wasm32-wasip2), sha256 $WASM_HASH. Pinned by near-antibody programs."
WASM_URL="https://github.com/Himess/antibody/releases/download/$TAG/antibody-verify.wasm"

echo "== 5. Contracts on NEAR testnet"
deploy() { # account wasm init-args
  local code; code="$(curl -s https://rpc.testnet.fastnear.com -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"finality\":\"final\",\"account_id\":\"$1\"}}" | python3 -c 'import sys,json; print(json.load(sys.stdin)["result"]["code_hash"])')"
  if [ "$code" = "11111111111111111111111111111111" ]; then
    ab --key-file "$KEYS/$1.json" admin deploy --wasm "$2" --init-method new --init-args "$3"
  fi
}
deploy "tusd.$HQ" "$WASM_DIR/antibody_tusd.wasm" "{\"owner\":\"$HQ\"}"
VERIFIER_CFG="{\"wasm_url\":\"$WASM_URL\",\"wasm_hash\":\"$WASM_HASH\",\"pubkey\":null,\"secrets_owner\":\"$HQ\",\"secrets_profile\":\"antibody-v1\",\"max_instructions\":5000000000,\"max_memory_mb\":512,\"max_execution_seconds\":60,\"deposit\":\"20000000000000000000000\",\"tier\":\"T0 (OutLayer operator-attested)\"}"
deploy "bounty.$HQ" "$WASM_DIR/near_antibody.wasm" "{\"owner\":\"$HQ\",\"token\":\"tusd.$HQ\",\"outlayer\":\"outlayer.testnet\",\"light_client\":\"client-eth2.sepolia.testnet\",\"verifier\":$VERIFIER_CFG}"
deploy "guardian.$HQ" "$GUARDIAN_WASM" "{\"owner\":\"$HQ\",\"mpc\":\"v1.signer-prod.testnet\",\"path\":\"guardian/acme\",\"guardian_address\":\"$GUARDIAN_EVM\"}"

echo "== 6. Verifier key: PROTECTED_ secret in OutLayer's keystore, then the pubkey run"
ab --key-file "$KEYS/$HQ.json" admin outlayer-secret --wasm-hash "$WASM_HASH" --contract "bounty.$HQ"
ab --key-file "$KEYS/$HQ.json" admin call --contract "bounty.$HQ" --method refresh_verifier_pubkey --gas-tgas 150

echo "== 7. Guardian targets and authorization"
for c in "11155111 sepolia https://sepolia.etherscan.io/tx/ 20000000000 1500000000" "84532 base-sepolia https://sepolia.basescan.org/tx/ 1000000000 100000000"; do
  set -- $c
  ab --key-file "$KEYS/$HQ.json" admin call --contract "guardian.$HQ" --method set_chain --args \
    "{\"target\":{\"chain_id\":$1,\"name\":\"$2\",\"to\":\"$ACME_GUARD\",\"calldata\":\"0x8456cb59\",\"gas_limit\":80000,\"max_fee_per_gas\":\"$4\",\"max_priority_fee_per_gas\":\"$5\",\"next_nonce\":0,\"explorer\":\"$3\"}}"
done
ab --key-file "$KEYS/$HQ.json" admin call --contract "guardian.$HQ" --method set_authorized --args "{\"account_id\":\"bounty.$HQ\",\"enabled\":true}"

echo "== 8. tUSD for the project and the scripted agents (testnet token, no value)"
ab --key-file "$KEYS/$HQ.json" admin call --contract "tusd.$HQ" --method mint --args "{\"account_id\":\"acme.$HQ\",\"amount\":\"30000000000\"}"
for x in a b c d e f; do
  ab --key-file "$KEYS/$HQ.json" admin call --contract "tusd.$HQ" --method mint --args "{\"account_id\":\"agent-$x.$HQ\",\"amount\":\"200000000\"}"
done

python3 - "$HQ" > "$DEP" <<'EOF'
import json, sys
hq = sys.argv[1]
print(json.dumps({"network": "testnet", "antibody": f"bounty.{hq}", "tusd": f"tusd.{hq}", "guardian": f"guardian.{hq}",
                  "outlayer": "outlayer.testnet", "light_client": "client-eth2.sepolia.testnet",
                  "rpc": "https://rpc.testnet.fastnear.com"}, indent=2))
EOF
echo "deployed: $DEP"
ab admin call --help >/dev/null
"$AB" --network testnet --deployments "$DEP" programs | head -20

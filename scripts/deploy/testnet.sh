#!/usr/bin/env bash
# Antibody on NEAR testnet: one deployment under antibody.testnet, deployed and tested unlocked, then
# locked as the launch by a separate, irreversible step.
#
#   scripts/deploy/testnet.sh plan       per-account amounts from the built wasm sizes, and the root's
#                                        balance (sends nothing)
#   scripts/deploy/testnet.sh deploy     accounts, contracts, wNEAR registrations, worker images, and the
#                                        deployments file
#   scripts/deploy/testnet.sh practice   the practice program's vending contract (after its program exists)
#   scripts/deploy/testnet.sh lock       deletes the full-access keys of near-antibody, near-guardian and the
#                                        vending contract (irreversible: LOCK=I-UNDERSTAND)
#
# Needs: the root account's full-access key at .secrets/near/antibody.testnet.json (git-ignored, never
# printed); `make contracts verifier-wasm`; wasm-opt (binaryen) for the smaller code stake; and the
# verifier wasm published where OutLayer fetches it: VERIFIER_URL, or PUBLISH_VERIFIER=1 to create the
# GitHub release `verifier-<hash>` with gh (a public release). Every step checks whether it already ran,
# so the script can be re-run after a failure. Transactions are testnet only; the payout token is wNEAR.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
set -a; [ -f .env ] && source .env; set +a

NET=testnet
RPC="${NEAR_RPC:-https://rpc.testnet.fastnear.com}"
HQ="${HQ:-antibody.testnet}"
KEYS="$ROOT/.secrets/near"
AB="$ROOT/target/release/antibody"
DEP="$ROOT/deployments/near-testnet.json"
OUT="$ROOT/target/deploy"
REL=contracts/near-antibody/target/wasm32-unknown-unknown/release
TOKEN=wrap.testnet
OUTLAYER=outlayer.testnet
REGISTRY=worker.outlayer.testnet
DCAP=tee-verifier-2026-07-22.testnet
LIGHT_CLIENT=client-eth2.sepolia.testnet
MPC=v1.signer-prod.testnet
# The verifier's resource limits and OutLayer deposit per run (0.02 NEAR attached; OutLayer refunds what
# a run does not use: ~0.007 at the 60 s limit, docs/measurements.md).
RUN_DEPOSIT=20000000000000000000000
# Images OutLayer approves after a program's creation count for it after this delay (spec §15); longer
# than any program's challenge window.
IMAGE_DELAY_SECS="${IMAGE_DELAY_SECS:-172800}"

ab() { "$AB" --network "$NET" --deployments "$DEP" "$@"; }
rpc() { curl -s "$RPC" -H 'content-type: application/json' -d "$1"; }
balance_of() { # yoctoNEAR, or empty when the account does not exist
  rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"finality\":\"final\",\"account_id\":\"$1\"}}" \
    | python3 -c 'import sys,json; r=json.load(sys.stdin).get("result"); print(r["amount"] if r else "")'; }
code_of() {
  rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"finality\":\"final\",\"account_id\":\"$1\"}}" \
    | python3 -c 'import sys,json; r=json.load(sys.stdin).get("result"); print(r["code_hash"] if r else "")'; }
# near_view <contract> <method> <json args>: the JSON result of a view call; stops the runbook if the call
# fails, so an RPC error is never read as "not set yet".
near_view() {
  rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"$1\",\"method_name\":\"$2\",\"args_base64\":\"$(printf '%s' "$3" | base64 -w0)\"}}" \
    | python3 -c 'import sys,json; r=json.load(sys.stdin); res=(r.get("result") or {}).get("result")
if res is None: sys.exit("view failed: " + json.dumps(r.get("error") or (r.get("result") or {}).get("error"))[:300])
print(bytes(res).decode())'; }
near() { python3 -c "import sys; print(f'{int(sys.argv[1]) / 1e24:.4f}')" "$1"; }

# The deployable contracts, optimized for size when wasm-opt is installed (the code stake is 1 NEAR per
# 100 kB, and a locked contract's stake never comes back).
wasm_for() { # <name> <cargo output>
  mkdir -p "$OUT"
  local src="$2" dst="$OUT/$1.wasm"
  [ -f "$src" ] || { echo "missing $src: run make contracts" >&2; exit 1; }
  if [ ! -f "$dst" ] || [ "$src" -nt "$dst" ]; then
    if command -v wasm-opt >/dev/null; then wasm-opt -Oz "$src" -o "$dst"; else cp "$src" "$dst"; fi
  fi
  echo "$dst"
}
ANTIBODY_WASM="$(wasm_for near_antibody "$REL/near_antibody.wasm")"
GUARDIAN_WASM="$(wasm_for antibody_guardian contracts/near-guardian/target/wasm32-unknown-unknown/release/antibody_guardian.wasm)"
VENDING_WASM="$(wasm_for antibody_practice_vending contracts/practice-vending/target/wasm32-unknown-unknown/release/antibody_practice_vending.wasm)"
VERIFIER_WASM="$ROOT/target/reproducible/wasm32-wasip2/release/antibody-verify.wasm"

# Per account: the code stake (its wasm's bytes), the state it will hold, and its gas and run deposits.
stake() { python3 -c "import os,sys; print(int((os.path.getsize(sys.argv[1]) + int(sys.argv[2])) * 10**19 + float(sys.argv[3]) * 10**24))" "$@"; }
declare -A AMOUNT=(
  [bounty]="$(stake "$ANTIBODY_WASM" 30000 0.02)"    # state: programs, commits, runs, images (~30 kB)
  [guardian]="$(stake "$GUARDIAN_WASM" 8000 0.12)"   # state: chains, permits; the deploy prepays its gas from here
  [acme]=850000000000000000000000                     # gas; NEAR credit (program storage, activation, rounds);
                                                      # 0.4 wrapped as the escrow; 0.1 near-guardian's MPC credit
  [agent-a]=120000000000000000000000                  # gas; one run deposit at a time (0.02), storage; a bond
  [agent-b]=120000000000000000000000
  [agent-c]=120000000000000000000000
  [agent-d]=120000000000000000000000
  [agent-e]=120000000000000000000000
  [agent-f]=120000000000000000000000
  [agent-g]=120000000000000000000000
  [keeper]=600000000000000000000000                   # gas; second runs and re-runs at each fix (0.02 deposit each)
  [relayer]=700000000000000000000000                  # T1 relays (verify_quote ~0.0176 each, ~30 runs)
)
ORDER=(bounty guardian acme agent-a agent-b agent-c agent-d agent-e agent-f agent-g keeper relayer)
# NEAR sets aside a call's prepaid gas at ten times the gas price before running it: a 300 Tgas call (a
# reveal, an activation, a T1 relay) needs 0.301 NEAR free, and burns about a tenth of it. The live run
# (`antibody-services live`) tops each account up to 0.36 NEAR free from the root before such calls and
# returns the agents' floats after their proofs; a vended practice account gets 0.34 (its commit and reveal).
PRACTICE_AMOUNT="$(stake "$VENDING_WASM" 8000 0.83)"  # code, state, two vends' gas funds (0.34 each), deploy margin

plan() {
  local total=0 a
  printf '%-34s %12s  %s\n' account NEAR exists
  for n in "${ORDER[@]}"; do
    a="$n.$HQ"
    printf '%-34s %12s  %s\n' "$a" "$(near "${AMOUNT[$n]}")" "$([ -n "$(balance_of "$a")" ] && echo yes || echo no)"
    total=$(python3 -c "print($total + ${AMOUNT[$n]})")
  done
  printf '%-34s %12s  %s\n' "practice.$HQ (optional)" "$(near "$PRACTICE_AMOUNT")" "$([ -n "$(balance_of "practice.$HQ")" ] && echo yes || echo no)"
  echo "wasm: near-antibody $(stat -c %s "$ANTIBODY_WASM") B, guardian $(stat -c %s "$GUARDIAN_WASM") B, vending $(stat -c %s "$VENDING_WASM") B$(command -v wasm-opt >/dev/null || echo ' (no wasm-opt: unoptimized)')"
  echo "sub-accounts: $(near "$total") NEAR (+ $(near "$PRACTICE_AMOUNT") for the practice program)"
  local have; have="$(balance_of "$HQ")"
  echo "$HQ holds: $( [ -n "$have" ] && near "$have" || echo 'no account') NEAR; it keeps ~0.35 for T1 relays, registrations and gas"
}

deploy() {
  [ -f "$KEYS/$HQ.json" ] || { echo "no root key at $KEYS/$HQ.json" >&2; exit 1; }
  [ -f "$VERIFIER_WASM" ] || { echo "missing $VERIFIER_WASM: run make verifier-wasm" >&2; exit 1; }
  local hash url
  hash="$(sha256sum "$VERIFIER_WASM" | cut -d' ' -f1)"

  echo "== 1. Sub-accounts of $HQ (only its key can create them)"
  for n in "${ORDER[@]}"; do
    a="$n.$HQ"
    [ -n "$(balance_of "$a")" ] && { echo "$a exists"; continue; }
    ab --key-file "$KEYS/$HQ.json" admin create-account --new-account-id "$a" --amount-near "$(near "${AMOUNT[$n]}")" --key-out "$KEYS/$a.json"
  done

  echo "== 2. The verifier wasm where OutLayer fetches it (sha256 $hash)"
  url="${VERIFIER_URL:-https://github.com/Himess/antibody/releases/download/verifier-${hash:0:12}/antibody-verify.wasm}"
  if ! curl -sfIL "$url" >/dev/null; then
    [ "${PUBLISH_VERIFIER:-0}" = 1 ] || { echo "$url is not published: set VERIFIER_URL, or PUBLISH_VERIFIER=1 for a public GitHub release" >&2; exit 1; }
    gh release create "verifier-${hash:0:12}" "$VERIFIER_WASM" --title "verifier ${hash:0:12}" \
      --notes "antibody-verify.wasm (wasm32-wasip2), sha256 $hash. Rebuild: make verifier-wasm. Pinned by near-antibody programs."
  fi
  curl -sL "$url" | sha256sum | grep -q "^$hash " || { echo "$url does not serve the built wasm" >&2; exit 1; }

  echo "== 3. near-antibody and near-guardian"
  local defaults
  defaults="{\"verifier\":{\"wasm_url\":\"$url\",\"wasm_hash\":\"$hash\",\"max_instructions\":5000000000,\"max_memory_mb\":512,\"max_execution_seconds\":60,\"deposit\":\"$RUN_DEPOSIT\"},\
\"outlayer\":\"$OUTLAYER\",\"dcap_verifier\":\"$DCAP\",\"accepted_tcb\":[\"UpToDate\"],\"image_delay_secs\":$IMAGE_DELAY_SECS,\
\"light_client\":\"$LIGHT_CLIENT\",\"light_client_chain_id\":11155111,\"guardian\":\"guardian.$HQ\",\"treasury\":\"$HQ\",\"image_registry\":\"$REGISTRY\"}"
  [ "$(code_of "bounty.$HQ")" = 11111111111111111111111111111111 ] && ab --key-file "$KEYS/bounty.$HQ.json" admin deploy --wasm "$ANTIBODY_WASM" \
    --init-method new --init-args "{\"owner\":\"$HQ\",\"token\":\"$TOKEN\",\"defaults\":$defaults}"
  [ "$(code_of "guardian.$HQ")" = 11111111111111111111111111111111 ] && ab --key-file "$KEYS/guardian.$HQ.json" admin deploy --wasm "$GUARDIAN_WASM" \
    --init-method new --init-args "{\"owner\":\"$HQ\",\"mpc\":\"$MPC\",\"antibody\":\"bounty.$HQ\"}"

  echo "== 4. wNEAR storage for near-antibody (it receives escrows and bonds) and the treasury (fees)"
  for a in "bounty.$HQ" "$HQ"; do
    if [ "$(near_view "$TOKEN" storage_balance_of "{\"account_id\":\"$a\"}")" = null ]; then
      ab --key-file "$KEYS/$HQ.json" admin call --contract "$TOKEN" --method storage_deposit \
        --args "{\"account_id\":\"$a\",\"registration_only\":true}" --deposit-yocto 1250000000000000000000 --gas-tgas 30
    fi
  done

  echo "== 5. OutLayer's approved worker images (the owner's sync is paid from near-antibody's balance)"
  if [ "$(near_view "bounty.$HQ" get_images '{}' | python3 -c 'import sys,json; print(len(json.load(sys.stdin)[0]))')" = 0 ]; then
    ab --key-file "$KEYS/$HQ.json" admin call --contract "bounty.$HQ" --method sync_images --args '{}' --gas-tgas 60
  fi

  echo "== 6. The deployments file"
  python3 - "$HQ" "$DEP" <<'EOF'
import json, sys
hq, path = sys.argv[1], sys.argv[2]
try:
    d = json.load(open(path))
except FileNotFoundError:
    d = {}
d.update({"network": "testnet", "antibody": f"bounty.{hq}", "token": "wrap.testnet", "token_decimals": 24,
          "token_symbol": "wNEAR", "guardian": f"guardian.{hq}", "outlayer": "outlayer.testnet",
          "light_client": "client-eth2.sepolia.testnet", "rpc": "https://rpc.testnet.fastnear.com"})
json.dump(d, open(path, "w"), indent=2)
open(path, "a").write("\n")
EOF
  ab --key-file "$KEYS/$HQ.json" admin record-deployment --git-commit "$(git rev-parse HEAD)"
  ab judge | head -40
}

practice() {
  : "${PRACTICE_PROGRAM:?set PRACTICE_PROGRAM to the practice program id (docs/practice.md)}"
  local a="practice.$HQ"
  [ -n "$(balance_of "$a")" ] || ab --key-file "$KEYS/$HQ.json" admin create-account --new-account-id "$a" \
    --amount-near "$(near "$PRACTICE_AMOUNT")" --key-out "$KEYS/$a.json"
  # Practice bonds and payouts are tiny amounts of wNEAR; the owner funds near-antibody credit for runs.
  [ "$(code_of "$a")" = 11111111111111111111111111111111 ] && ab --key-file "$KEYS/$a.json" practice deploy \
    --wasm "$VENDING_WASM" --owner "$HQ" --program "$PRACTICE_PROGRAM" --vaults 24 --bond-budget "${PRACTICE_BOND_BUDGET:-0.01}" --per-commit-cap "${PRACTICE_BOND:-0.001}"
  ab --key-file "$KEYS/$HQ.json" practice fund --vending "$a" --near "${PRACTICE_NEAR:-0.15}" --tokens "${PRACTICE_TOKENS:-0.01}"
}

lock() {
  [ "${LOCK:-}" = I-UNDERSTAND ] || { echo "locking is irreversible: run with LOCK=I-UNDERSTAND once the live story has run" >&2; exit 1; }
  for a in "bounty.$HQ" "guardian.$HQ" "practice.$HQ"; do
    [ -n "$(balance_of "$a")" ] || continue
    ab --key-file "$KEYS/$a.json" admin lock --confirm-irreversible "$a"
  done
  ab admin record-deployment --git-commit "$(git rev-parse HEAD)"
  ab judge | head -40
}

case "${1:-plan}" in
  plan) plan ;;
  deploy) deploy ;;
  practice) practice ;;
  lock) lock ;;
  *) echo "usage: $0 plan|deploy|practice|lock" >&2; exit 2 ;;
esac

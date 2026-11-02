#!/usr/bin/env bash
# Local end-to-end check of the guardian transaction encoder (contracts/near-guardian/src/eip1559.rs)
# on anvil with chain id 11155111. A local key stands in for the MPC-derived guardian key.
set -euo pipefail
RPC=${RPC:-http://127.0.0.1:8546}
SCR=${SCR:-/tmp}
ENC=contracts/near-guardian/encoder-check/target/release/encoder-check.exe
newkey() { cast wallet new --json | python -I -c "import json,sys;d=json.load(sys.stdin);d=d['data'] if isinstance(d,dict) and 'data' in d else d;d=d[0] if isinstance(d,list) else d;print(d['private_key'])"; }
ADMIN_KEY=$(newkey); ADMIN=$(cast wallet address --private-key $ADMIN_KEY)
G_KEY=$(newkey); G=$(cast wallet address --private-key $G_KEY)
cast rpc anvil_setBalance $ADMIN 0x56BC75E2D63100000 --rpc-url $RPC >/dev/null
cast rpc anvil_setBalance $G 0xDE0B6B3A7640000 --rpc-url $RPC >/dev/null
echo "chain_id=$(cast chain-id --rpc-url $RPC) admin=$ADMIN guardian(stand-in)=$G"
VAULT=$(cd contracts/evm && forge create src/GuardedVault.sol:GuardedVault --rpc-url $RPC --private-key $ADMIN_KEY --broadcast --constructor-args $G | awk '/Deployed to/{print $3}')
echo "vault=$VAULT paused=$(cast call $VAULT 'paused()(bool)' --rpc-url $RPC)"
NONCE=$(cast nonce $G --rpc-url $RPC); BASE=$(cast base-fee --rpc-url $RPC)
PRIO=1000000000; MAXFEE=$(( BASE * 2 + PRIO )); GAS=100000
UNSIGNED=$($ENC unsigned 11155111 $NONCE $PRIO $MAXFEE $GAS $VAULT 0x8456cb59)
HASH=$(cast keccak $UNSIGNED)
SIG=$(cast wallet sign --no-hash --private-key $G_KEY $HASH)
R=${SIG:2:64}; S=${SIG:66:64}; V=$(( 16#${SIG:130:2} )); YP=$(( V >= 27 ? V - 27 : V ))
SIGNED=$($ENC signed 11155111 $NONCE $PRIO $MAXFEE $GAS $VAULT 0x8456cb59 $YP $R $S)
echo "payload(keccak of unsigned)=$HASH yParity=$YP"
TX=$(cast publish $SIGNED --rpc-url $RPC | python -I -c "import json,sys;print(json.load(sys.stdin)['transactionHash'])")
echo "tx=$TX status=$(cast receipt $TX status --rpc-url $RPC) from=$(cast tx $TX from --rpc-url $RPC)"
echo "paused=$(cast call $VAULT 'paused()(bool)' --rpc-url $RPC) pauseCount=$(cast call $VAULT 'pauseCount()(uint256)' --rpc-url $RPC)"
# Idempotency: a second, independently signed pause (nonce+1) must succeed and not change pauseCount.
N2=$((NONCE+1)); U2=$($ENC unsigned 11155111 $N2 $PRIO $MAXFEE $GAS $VAULT 0x8456cb59); H2=$(cast keccak $U2); S2=$(cast wallet sign --no-hash --private-key $G_KEY $H2)
SG2=$($ENC signed 11155111 $N2 $PRIO $MAXFEE $GAS $VAULT 0x8456cb59 $(( 16#${S2:130:2} >= 27 ? 16#${S2:130:2} - 27 : 16#${S2:130:2} )) ${S2:2:64} ${S2:66:64})
TX2=$(cast publish $SG2 --rpc-url $RPC | python -I -c "import json,sys;print(json.load(sys.stdin)['transactionHash'])")
echo "duplicate pause tx status=$(cast receipt $TX2 status --rpc-url $RPC) pauseCount=$(cast call $VAULT 'pauseCount()(uint256)' --rpc-url $RPC) gasUsed=$(cast receipt $TX2 gasUsed --rpc-url $RPC)"
# Replay protection: re-publishing the first raw tx must be rejected (nonce already used).
if cast publish $SIGNED --rpc-url $RPC >/dev/null 2>&1; then echo "replay: ACCEPTED (unexpected)"; else echo "replay of the same raw tx: rejected (nonce used)"; fi

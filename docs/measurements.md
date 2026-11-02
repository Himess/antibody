# Measurements

Every number here comes either from public NEAR testnet data, read from the RPC and FastNEAR's
transaction API, or from this repository's own runs. Each table states its method and the window it
covers.

## Antibody's own runs

| What | Value | Source |
|---|---|---|
| Verifier work per verification (program mode, Acme fixture) | 462–476 M wasm instructions | `python3 scripts/demo/check_fixtures.py fixtures/acme-local` (wasmtime with OutLayer's engine config; also `round1`) |
| Native vs wasm output | Byte-identical for every fixture (full receipt and facts) | Same script; CI job `verifier` |
| Sandbox end to end | All expected outcomes, including round 1 and disclosure, in 376 s (CI run 41438293042) | `make demo`, CI job `e2e`, `target/demo/results.json` |
| Guardian pause after a verified Critical (sandbox, MOCK MPC) | 0.5 s to `paused() == true` on both chains | Same run (`guardian.pause_seconds_after_verdict`) |
| TEE re-run of one claimed finding (sandbox, MOCK OutLayer) | 7.8–7.9 s each, from the witness build to the recorded result | Same run, round-1 step |
| near-antibody wasm | 937,426 B (fits one deploy transaction) | `contracts/near-antibody/target/wasm32-unknown-unknown/release/near_antibody.wasm` |

## OutLayer on testnet (`outlayer.testnet`)

Window: 244 requests from 2026-11-01 11:27 to 2026-11-02 06:17 UTC, from 28 callers (192 through a contract).

| Metric | p50 | p90 | p95 | max |
|---|---|---|---|---|
| Latency, `request_execution` receipt to `on_execution_response` receipt (n = 241) | 5.1 s | 7.3 s | 12.1 s | 111.1 s (yield timeout) |
| Compute charged | 0.00103 NEAR | — | 0.0016 NEAR | 0.0041 NEAR |

- **Cost per run.** Gas for the whole transaction tree is 0.00124 NEAR at p50, so a short run costs
  about 0.0023 NEAR in total.
- **Method.** For latency: the block timestamps of the two receipts within one transaction tree. For
  cost: the `yNEAR charged` log of `on_execution_response`.

## NEAR Chain Signatures on testnet (`v1.signer-prod.testnet`)

| Metric | Value |
|---|---|
| Sign latency, 96 requests (2026-11-02 07:17–07:41 UTC, domains 0 and 1) | p50 3.44 s, p90 3.68 s, max 4.79 s, 0 timeouts |
| Deposit / gas | 1 yoctoNEAR (excess refunded); at least 15 Tgas (`near-guardian` attaches 40 Tgas) |

## DCAP verifier (`tee-verifier-2026-07-22.testnet`)

| Metric | Value |
|---|---|
| `verify_quote` gas, 200 recent calls | 175.5–175.8 Tgas (about 0.0176 NEAR) |
| OutLayer request 6496 (public attestation) | `Verified`, TCB `UpToDate`; `report_data` binds the task; worker image = OutLayer approved entry #109 (`antibody tee check --request-id 6496`) |

## Ethereum Sepolia light client (`client-eth2.sepolia.testnet`)

| Metric | Value |
|---|---|
| Lag behind finalized, 30 samples every 40 s (2026-11-02 07:41–08:01 UTC) | 0 blocks in 24 samples; one epoch (31–32 blocks) for at most about 81 s after a finality step |
| Update cadence | 108 updates in 9.7 h, median gap 6.53 min |
| Window | About 51,000 blocks (about 7.1 days) of `block_hash_safe` history |

## FastFS (sealed-blob storage)

| Metric | Value |
|---|---|
| Upload cost | A 1,048,686 B chunk cost 0.0054 NEAR |
| Hosting the verifier wasm | GitHub release assets; 85 of 93 sampled OutLayer runs with GitHub-hosted wasm succeeded |

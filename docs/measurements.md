# Measurements

Every number here comes either from public NEAR testnet data, read from the RPC and FastNEAR's
transaction API, or from this repository's own runs. Each table states its method and the window it
covers. The testnet run's numbers (real OutLayer, DCAP and MPC) replace the sandbox's when it runs.

## The sandbox run

`make serve` (`antibody-services demo --serve`) on a Linux VPS, 2026-11-03 05:16 UTC, commit 374eaa8:
near-sandbox and two anvils, the real verifier in TEE mode on MOCK OutLayer, MOCK DCAP quotes checked by the real T1 code,
MOCK MPC. Results: [results/sandbox-2026-11-03.json](results/sandbox-2026-11-03.json). Each moment is
timed by its own block or receipt (NEAR events at their blocks, as the indexer recorded them; each EVM
pause when its receipt came back): the times show the flow's own latency, not OutLayer's or the MPC
network's.

| What | Value | Field |
|---|---|---|
| The whole story, every check passed | 526.3 s | `seconds`, `all_ok` |
| Activation (base witness pinned, sealing key T1-confirmed) | 12.7 s | `activation.seconds` |
| Agent A over MCP, from its commit's block: the verdict, T1 and the payment at their NEAR blocks | 7.9 s, 11.5 s, 24.9 s | `one_moment.seconds_from_commit` |
| The pause, asked for before the payment: permits signed by near-guardian (MOCK MPC), then each chain's receipt | signed 21.2 s and 23.1 s; paused on 31337 at 22.7 s, on 31338 at 24.5 s | same |
| Commit to verdict, agents B–G | 9.7–10.4 s (from each submission's start: witness, seal, commit, reveal) | `submissions[].seconds` |
| A round (the keeper re-runs every eligible proof, then the round opens): round 1, round 2 | 66.8 s, 72.7 s | `rounds[].seconds` |
| Release after round 2 opens: both proxies on both chains | 1.1 s | `release.seconds` |
| Practice try, vend to payout (two judges, no account) | 17.0 s each | `practice.tries[].seconds` |
| Fork mode, the tutorial proof to payout | 37.0 s | `fork_mode.seconds` |
| A submission's records at near-antibody by its verdict (B) | 903 B | `costs.contract_storage_bytes` |
| Its NEAR credit spent: the run deposit and answer allowance (0.02), storage with the 1 KiB settlement prepay (0.0166) | 0.0366 NEAR | `costs.near_credit_spent_yocto` |
| API: endpoints answering, events indexed | 17 of 17, 233 events | `api` |

The sandbox charges ten times testnet's gas, so wallet deltas from it are not testnet costs.

## Contracts and the verifier

| What | Value | Source |
|---|---|---|
| Verifier work per verification (program mode, Acme fixtures, rounds 0–2, 17 runs) | 467–487 M wasm instructions | `python3 scripts/demo/check_fixtures.py fixtures/acme-local` (wasmtime with OutLayer's engine config; also `round1`, `round2`) |
| Native vs wasm output | Byte-identical for every fixture (full receipt and facts) | Same script; CI job `verifier` |
| near-antibody wasm | 910,470 B built, 741,744 B after `wasm-opt -Oz` (binaryen 133): 7.42 NEAR of code stake | `scripts/deploy/testnet.sh plan` |
| near-guardian wasm | 298,001 B built, 244,883 B after `wasm-opt -Oz`: 2.45 NEAR | Same |
| Practice vending wasm | 78,832 B built, 64,241 B after `wasm-opt -Oz`: 0.64 NEAR | Same |
| Settlement storage a commit prepays | 1 KiB (0.0102 NEAR); measured worst case 920 B (a first holder with four keys and a challenge record) | `a_commits_settlement_is_prepaid` (near-antibody unit test) |

NEAR storage costs 1 NEAR per 100 kB, staked, not spent; a locked contract's stake stays with it.

## Bond floor

The recommended floor is `3 × p95(cost of verifying one submission, T1 relay included)`, valued at the
NEAR/token rate the project uses (docs/spec.md §12.2). With today's testnet values:

| Input | Value | Source |
|---|---|---|
| OutLayer's charge for one run, upper bound | 0.007 NEAR (a run at the 60 s limit; `estimate_execution_cost`) | OutLayer view call; measured runs below charge less (p95 0.0016) |
| The run's transaction gas | 0.0013 NEAR | OutLayer table below (p50 0.00124) |
| T1 relay (`verify_quote`) | 0.0176 NEAR (175.8 Tgas) | DCAP table below |
| **Cost of one verification** | **≈ 0.026 NEAR** | sum |
| **Floor** | **≈ 0.078 NEAR per commit** | × 3 |

The sandbox demo prices bonds in its test token; the testnet story will state its demo bond and whether it
is below this floor.

## Cost per submission for a hunter (testnet)

A hunter attaches the program's run deposit (0.02 NEAR) from its NEAR credit at near-antibody; OutLayer
charges its compute (≤ 0.007) and refunds the rest to the hunter's wallet. near-antibody keeps a 0.01 NEAR
allowance for the storage of the run's answer, and the commit's own records plus its 1 KiB settlement
prepay (≈ 0.013 NEAR together), and the hunter pays its transactions' gas (≈ 0.002). The T1 relay is paid
by whoever relays (Antibody's relayer by default). The bond comes back unless the proof breaks nothing.

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

## NEAR's MPC network on testnet (`v1.signer-prod.testnet`)

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

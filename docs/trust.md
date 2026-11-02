# Trust table

What you have to trust, and how to check it, for each part of Antibody. "Pre-launch" is the real testnet
deployment. "Live (MOCK)" describes the mainnet design, which the API shows only as recorded data with a
MOCK badge. No mainnet transactions are sent.

| Component | Pre-launch (testnet, real) | Live (MOCK) | What you trust | How to check it yourself |
|---|---|---|---|---|
| Anchor block hash | `client-eth2.sepolia.testnet` checks `block_hash_safe(M) == H` at program creation; otherwise the program is labelled *project-attested* | `client-eth2.bridge.near` (sync committee); the v0.3 forged-header test was rejected there | **Testnet:** the light client's single trusted relayer. **Live:** Ethereum's sync committee | Compare `anchor_trust` in `get_program` with any Sepolia RPC |
| State at the anchor | MPT proofs verified inside the verifier | Same | Nothing (hashes) | `antibody check` / `antibody verify` re-run it |
| EVM semantics | revm 38 + reth v2.2.0 under the program's committed `evm_spec` (Osaka) | Same | That the pinned EVM matches the committed rules | Native and wasm runs are byte-identical; the receipt hash is reproducible |
| Who runs the verifier | OutLayer TEE worker, wasm pinned by sha256 (`WasmUrl`) | Same | **T0:** the OutLayer operator delivers the worker's output. **T1:** the run's Intel TDX quote, verified on NEAR (below). **T2:** zk (roadmap) | Above the instant cap: `challenge()` during the delay. `antibody tee check --request-id N` for any run. After disclosure: recompute every verdict |
| Verifier key (sealed PoCs) | OutLayer keystore `PROTECTED_ANTIBODY_SEED`, bound to the wasm hash and to near-antibody as caller | Same | The keystore TEE and its governance (see OutLayer's documentation) | The verifier source (`crates/verifier/src/tee.rs`) never outputs the seed; `pubkey` mode output. The wasm build is not yet bit-for-bit reproducible (it embeds build paths) |
| Fallback key (if the keystore is unavailable) | Manual secret known to the deployer, labelled **weaker** | — | The deployer | — |
| PoC confidentiality | TEE until disclosure; the project sees Violations only | Same | TEE + keystore | Disclosure publishes every content key; check that nothing leaked earlier |
| Settlement rules | near-antibody (immutable per program) | Same, with NEAR Intents payouts | NEAR consensus | Read the contract, call `get_program` |
| Payout asset | tUSD (testnet token, no value) | NEAR Intents (shown as MOCK) | — | `ft_balance_of` |
| Guardian | near-guardian + `v1.signer-prod.testnet` (11 of 16) | `v1.signer` | MPC threshold; the contract builds every transaction | `cast decode-transaction` on the signed transaction; `paused()` on each chain |
| Blob storage | FastFS (NEAR transaction arguments) | Same | Nothing (sha256 commitment) | `antibody verify` fetches by hash |
| Relayer / indexer / API / notifier | Informational, permissionless | Same | Nothing | Everything is re-derivable from NEAR |
| Sandbox demo (`make demo`) | MOCK OutLayer, MOCK MPC, TEST keys | — | Nothing (local) | It is a test harness |

## T1: TDX quotes verified on NEAR

OutLayer's worker signs one TDX quote per execution. Its `report_data` is SHA-256 over the task fields
(OutLayer `worker/src/tdx_attestation.rs`): the wasm hash, the input hash, the output hash, the caller,
the secrets, and the time. The quote is public at
`testnet-api.outlayer.ai/attestations/by-request/{id}`.

**`confirm_tee`.** Anyone may relay a quote with `confirm_tee(commit, round?, attestation, quote,
collateral)`. near-antibody accepts it only if:
1. the attestation describes exactly the run near-antibody requested:
   - the program's pinned wasm hash;
   - SHA-256 of the input it sent and of the bytes it received, both recorded when the run happened;
   - as the caller, the signer of the transaction that requested the run (near-antibody records it),
     or near-antibody itself for a relayed meta-transaction: OutLayer attests the transaction's signer;
   - the program's secrets;
2. `tee-verifier-2026-07-22.testnet.verify_quote` (NEAR's DCAP verifier) returns `Verified` with an
   accepted TCB status;
3. the quote's `report_data` equals the recomputed hash;
4. the worker image (MRTD, RTMR0–3) is in the pinned allowlist, copied from OutLayer's
   `get_approved_measurements`.

The verdict's tier then becomes `T1 (TDX quote verified on NEAR)`. A T1 payout is released without
waiting for the challenge window. A challenged T1 payout is upheld by `resolve_challenge_t1`, without
the arbiter, because the quote proves the pinned verifier produced exactly these facts.

**Evidence today (live, no funds needed).** `antibody --network testnet tee check --request-id 6496`
checks a public OutLayer run end to end. The quote's report_data binds the task, `verify_quote`, run as a
free view call, returns `Verified` / `UpToDate`, and the worker image is entry #109 of OutLayer's 111
approved measurements. `fixtures/t1/` records that data, and the contract test
`t1_accepts_a_real_quote_only_for_the_run_it_binds` feeds it through `confirm_tee` and
`on_tee_verified`, accepting it only for the run it binds.

**Limits.**
- **Collateral freshness.** OutLayer's newest on-chain collateral expires on 2026-11-08 (`nextUpdate`).
  `tee check`, `tee confirm` and the T1 relayer then fetch fresh collateral from Intel PCS: the TCB
  info, QE identity and CRLs for the FMSPC in the quote's PCK certificate, signed by Intel.
  `--collateral auto` (the default) tries OutLayer's copies first, then PCS. On 2026-11-02, PCS
  collateral (TCB info issued that day, next update 2026-12-02) verified request 6496's quote as
  `Verified` / `UpToDate`.
- **Settlement timing.** T1 for Antibody's own runs needs the live OutLayer deployment, and a T0
  verdict settles before a relayer can confirm it.

## What is mock

- **The mainnet view.** It uses recorded data only, served by `/v1/mock/mainnet` with `"mock": true`.
- **The sandbox demo.** OutLayer and the MPC are replaced there by `mock-outlayer` and `mock-mpc`, resolved by the real verifier (TEST seed) and a TEST secp256k1 key.
- **Test-only keys in fixtures.** The fp_tags in `fixtures/acme-local/expected.json` use a TEST `k_program`.

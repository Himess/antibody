# Trust

What you have to trust, and how to check it, for each part of Antibody. No mainnet transaction is ever
sent; the mainnet explainer (`/v1/mock/mainnet`) is recorded data, marked `"mock": true`.

| Part | What you trust | How to check it yourself |
|---|---|---|
| **The anchor block** | The Ethereum light client near-antibody reads at program creation (on testnet, `client-eth2.sepolia.testnet` and its single relayer). An anchor it cannot confirm is labelled project-attested | `antibody judge` shows each program's anchor and its check; compare the hash with any Sepolia RPC |
| **State at the anchor** | Nothing: Merkle-Patricia proofs, verified inside the verifier against the anchor's state root, every trie node's kind validated (SA-3) | `antibody check` re-runs any proof locally |
| **EVM semantics** | That revm 38 and reth 2.2, under the program's committed `evm_spec`, match the committed rules | Native and wasm runs are byte-identical; the receipt hash is reproducible |
| **The verifier build** | Nothing beyond the source: a program pins the wasm's sha256 | `make verifier-wasm` (or `Dockerfile.verifier`) rebuilds it reproducibly and prints the hash to compare with the program's `wasm_hash` |
| **Who runs the verifier** | **T0:** OutLayer's operator delivers the TEE worker's output. **T1:** the run's Intel TDX quote, verified on NEAR (below) | `antibody tee check --request-id <id>`, for any run; after disclosure, recompute every verdict |
| **The verifier key (sealed proofs)** | OutLayer's keystore, which derives the program's key for the pinned wasm called by near-antibody | The verifier never outputs the seed; the program's sealing key counts only once its activation run is T1-confirmed |
| **Confidentiality of proofs** | The TEE and the keystore until disclosure; the project reads only Violations it pays for | Disclosure publishes every content key: check nothing leaked earlier |
| **Settlement** | NEAR consensus and near-antibody's code, pinned per program | `antibody judge`: code hashes and locked status; a live program's pins never change (property 12) |
| **Permits** | The MPC network signs what near-guardian builds; each permit is for one program, one round and one chain | `AntibodyAdapter` recomputes the EIP-712 digest (a golden test pins both sides); `antibody judge` lists every permit |
| **Blob storage** | Nothing: FastFS, checked by sha256 against the commitment | `antibody status` fetches by hash |
| **Relayers, keeper, indexer, API, notifier** | Nothing: permissionless calls and derived data | Everything is re-derivable from NEAR views |
| **The sandbox demo** | Nothing (local): MOCK OutLayer, DCAP and MPC, TEST keys; the T1 checks on its MOCK quotes are the real code | It is a test harness |

## T1: TDX quotes verified on NEAR

OutLayer's worker signs one TDX quote per execution. Its `report_data` is SHA-256 over the task: the wasm
hash, the input hash, the output hash, the caller, the secrets and the time. The quote is public at
`testnet-api.outlayer.ai/attestations/by-request/{id}`.

Anyone may relay a quote (`confirm_tee`, or the T1 relayer). near-antibody accepts it only if:
1. the attestation describes exactly the run near-antibody requested: the program's pinned wasm hash, the
   SHA-256 of the input it sent and of the bytes it received (both recorded with the run), the caller,
   and the program's secrets;
2. NEAR's DCAP verifier returns `Verified` with an accepted TCB status;
3. the quote's `report_data` equals the recomputed hash;
4. the worker image (MRTD, RTMR0–3) is in the program's pinned allowlist (an image approved later counts
   only after a delay longer than any challenge window).

The run is then **T1**. A T1 payout under the program's instant cap is paid in seconds; above the cap it
also needs an independent second run that agrees, and waits its challenge window.

**Evidence (no funds needed).** `antibody tee check --request-id 6496` checks a public OutLayer run end
to end: the report_data binds the task, `verify_quote` (a free view call) returns `Verified` /
`UpToDate`, and the worker image is in OutLayer's approved measurements. Independently of Antibody's
code, OutLayer's open-source `outlayer-verify` re-runs the same checks with Intel's root compiled in:
`outlayer-verify job <request id> --network testnet`, or `outlayer-verify tx <the transaction that
requested the run>` (testnet runs only; the sandbox's quotes are MOCK). `fixtures/t1/` records it, and
the contract test `t1_accepts_a_real_quote_only_for_the_run_it_binds` accepts it only for the run it
binds.

**Limits.**
- **Collateral freshness.** When OutLayer's on-chain collateral expires, the CLI and the T1 relayer fetch
  fresh collateral from Intel PCS (`--collateral auto` tries OutLayer's copies first).
- **Availability.** A run whose quote cannot be had stays T0: its payout waits the window, and a re-run
  may be asked for.

## What is mock

- **The mainnet explainer** (`/v1/mock/mainnet`): recorded data only.
- **The sandbox demo:** MOCK OutLayer, DCAP and MPC contracts, resolved by the real verifier with a TEST
  seed and a TEST secp256k1 key.
- **Test keys in fixtures:** the fingerprint tags in `fixtures/acme-local/expected.json` use a TEST key.

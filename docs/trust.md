# Trust

What you have to trust, and how to check it, for each part of Antibody. No mainnet transaction is ever
sent; "How mainnet works" (docs/mainnet.md, `/v1/mainnet`) makes view calls only.

| Part | What you trust | How to check it yourself |
|---|---|---|
| **The anchor block** | The Ethereum light client near-antibody reads at program creation (on testnet, `client-eth2.sepolia.testnet` and its single relayer). An anchor it cannot confirm is labelled project-attested | `antibody judge` shows each program's anchor and its check; compare the hash with any Sepolia RPC |
| **State at the anchor** | Nothing: Merkle-Patricia proofs, verified inside the verifier against the anchor's state root, every trie node's kind validated (SA-3) | `antibody check` re-runs any proof locally |
| **EVM semantics** | That revm 38 and reth 2.2, under the program's committed `evm_spec`, match the committed rules | Native and wasm runs are byte-identical; the receipt hash is reproducible for the same sealed input (a local `check` seals afresh, so its envelope, and the receipt's `witness_sha256`, differ run to run; the verdict and the rule bits do not) |
| **The verifier build** | Nothing beyond the source: a program pins the wasm's sha256 | `make verifier-wasm` (or `Dockerfile.verifier`) rebuilds it reproducibly and prints the hash to compare with the program's `wasm_hash` |
| **Who runs the verifier** | **T0:** OutLayer's operator delivers the TEE worker's output. **T1:** the run's Intel TDX quote, verified on NEAR (below). **T2** (a disclosed verdict): SP1's proof system, for a Groth16 proof that near-t2 checks against near-antibody's records (below) | `antibody tee check --request-id <id>`, for any run; after disclosure, recompute every verdict, or `antibody zk status --run <id>` |
| **The verifier key (sealed proofs)** | OutLayer's keystore, which derives the program's key for the pinned wasm called by near-antibody | The verifier never outputs the seed; the program's sealing key counts only once its activation run is T1-confirmed |
| **Confidentiality of proofs** | The TEE and the keystore until disclosure; the project reads only Violations it pays for | Disclosure publishes every content key: check nothing leaked earlier |
| **Settlement** | NEAR consensus and near-antibody's code, pinned per program | `antibody judge`: code hashes and locked status; a live program's pins never change (property 12) |
| **Permits** | The MPC network signs what near-guardian builds; each permit is for one program, one round and one chain | `AntibodyAdapter` recomputes the EIP-712 digest (a golden test pins both sides); `antibody judge` lists every permit |
| **Blob storage** | Nothing: FastFS, checked by sha256 against the commitment | `antibody status` fetches by hash |
| **Relayers, keeper, indexer, API, notifier** | Nothing for safety: permissionless calls and derived data. For liveness, payouts and pauses wait for someone to call them: our keeper and relayers, or anyone running `antibody keeper` | Everything is re-derivable from NEAR views |
| **The sandbox demo** | Nothing (local): MOCK OutLayer, DCAP and MPC, TEST keys; the T1 checks on its MOCK quotes are the real code | It is a test harness |

## By actor

| Actor | Trusted for | How anyone checks it |
|---|---|---|
| The light client | The anchor block's hash | The program's anchor check (`antibody judge`) against any RPC |
| The TEE (OutLayer worker) | Running the pinned verifier on the sealed proof | **T1:** the run's TDX quote is verified on NEAR; `antibody tee check --request-id <id>` reads OutLayer's attestation (testnet: the sandbox's OutLayer is a MOCK) |
| The verifier | Nothing beyond its pinned build | Native and wasm builds agree byte for byte; `antibody check` re-runs a proof locally; after disclosure every verdict can be recomputed |
| The MPC network | Signing the pause and release permits near-guardian builds | `AntibodyAdapter` recomputes each permit's EIP-712 digest on chain |
| near-antibody's owner | The defaults of new programs only | A live program's pins never change (property 12); `antibody judge` shows whether the contract is locked |
| The project | Its manifest, escrow and fixes | The manifest hash is committed; known issues are sealed before open; a fix is released only after every paid proof re-runs against it and fails |
| Relayers, keeper, indexer, API | Nothing: permissionless calls and derived data | Everything they show is re-derivable from NEAR |

The verification levels: **T0** (OutLayer delivers the TEE's output), **T1** (its quote verified on NEAR).
Instant payouts need T1. Above the cap, a payout also needs a second run that agrees, and waits its
challenge window. **T2** (a validity proof of a disclosed verdict, verified on NEAR) claims no
privacy, and no payout depends on it.

## T1: TDX quotes verified on NEAR

OutLayer's worker signs one TDX quote per execution. Its `report_data` is SHA-256 over the task: the wasm
hash, the input hash, the output hash, the caller, the secrets and the time. The quote is public at
`testnet-api.outlayer.ai/attestations/by-request/{id}`.

Anyone may relay a quote (`confirm_tee`, or the T1 relayer). near-antibody accepts it only if:
1. the attestation's task fields name the run near-antibody requested: an execution built for
   `wasm32-wasip2`, the program's pinned wasm hash, the SHA-256 of the input it sent and of the bytes it
   received (both recorded with the run), the caller, that the run used no secrets, and a `WasmUrl`
   source;
2. NEAR's DCAP verifier returns `Verified` with an accepted TCB status;
3. the quote's `report_data` equals the recomputed hash;
4. the worker image (MRTD, RTMR0–3) is in the program's pinned allowlist (an image approved later counts
   only after a delay longer than any challenge window).

The run is then **T1**. A T1 payout under the program's instant cap is paid in seconds; above the cap it
also needs a second run that agrees, and waits its challenge window.

**Evidence (no funds needed).** `antibody tee check --request-id 8839` checks the recorded moment's run
(run 33, agent A's Critical) end to end: its caller is `agent-a.antibody.testnet`, its `input_hash` and
`output_hash` are run 33's `request_sha256` and `output_sha256` on near-antibody (`get_run`), its
`wasm_hash` is the pinned verifier (`ccc27f0c…`), the report_data binds the task, `verify_quote` (a free
view call) returns `Verified` / `UpToDate`, and the worker image is in OutLayer's approved measurements.
`T1-eligible` means exactly those checks pass, the ones near-antibody repeats on chain before it marks a
run T1. Independently of Antibody's
code, OutLayer's open-source `outlayer-verify` re-runs the same checks with Intel's root compiled in:
`outlayer-verify job <request id> --network testnet`, or `outlayer-verify tx <the transaction that
requested the run>` (testnet runs only; the sandbox's quotes are MOCK). `fixtures/t1/` records it, and
the contract test `t1_accepts_a_real_quote_only_for_the_run_it_binds` accepts it only for the run it
binds.

**Limits.**
- **Collateral freshness.** When OutLayer's on-chain collateral expires, the CLI and the T1 relayer fetch
  fresh collateral from Intel PCS (`--collateral auto` tries OutLayer's copies first; the services take
  `--t1-collateral`). On testnet OutLayer's copies for its workers' platform expire on 2026-11-07 at
  23:46:20 UTC (QE identity; TCB info at 00:29:39 the next day). Agent A's run verified against fresh
  Intel PCS collateral before that, `Verified` / `UpToDate`, the status programs accept
  (`docs/results/t1-collateral-2026-11-06.json`). After it, on 2026-11-08, the services' T1 relayer
  relayed two runs with fresh Intel PCS collateral, and both are T1 on NEAR (runs 68 and 69, the first
  automated drill's: `docs/results/t1-collateral-2026-11-08.json`). OutLayer has since renewed its copies
  (valid to 2026-12-08 for FMSPC B0C06F000000 on 2026-11-09; `antibody tee check --request-id 8839` reads
  `outlayer-onchain`, `UpToDate`). `/v1/status` shows when the copies expire, says when relays use Intel
  PCS (`t1-collateral-pcs`), and raises an alert for every T1 relay that fails.
- **Worker images.** OutLayer adds worker images to its approved list. A program accepts the images it
  pinned at creation, and later ones from its pinned registry only after the image delay (2 days on
  testnet), once near-antibody lists them (`sync_images`, permissionless). Until then a run on a new
  image gets no T1: its payout waits the window, and its pause waits too, since a pause needs T1. The
  keeper lists new images within 10 minutes, the T1 relayer sends nothing for a run whose image does not
  count yet (it retries when it does), and `/v1/status` warns while an image is pending. On 2026-11-06
  OutLayer approved six new images:
  - two listed by hand and two by the keeper, half an hour later; they count for programs 5–7 from
    2026-11-08 13:15:22 and 13:47:47 UTC;
  - two more that evening, listed at 20:18:16 UTC (by hand: the keeper's 0.298 NEAR could not set aside
    the 0.301 NEAR its 300 Tgas call needs); they count from 2026-11-08 20:18:16 UTC.

  While a worker runs an image that does not count yet, its runs stay T0.
  - A practice try is then still paid, after the 60 s challenge window.
  - A practice program created after new images are listed counts them at once. The runbook
    ([runbooks/worker-images.md](runbooks/worker-images.md)) moves the practice vending contract to such a
    program, through its owner's `set_terms`.
- **Availability.** A run whose quote cannot be had stays T0: its payout waits the window, and a re-run
  may be asked for.
- **Quote metadata** (threat model I4b). OutLayer's task hash concatenates its fields without length
  prefixes, and its metadata (task id, block height, timestamp, attached USD) is not checked: a collision
  would need the TEE to attest a run whose metadata embeds this run's hashes.

## T2: validity proofs of disclosed verdicts

Once a verdict's proof is disclosed, its re-run can be a Groth16 proof (SP1 6.10.0) that near-t2 checks on
NEAR ([zk.md](zk.md)). near-t2 then compares the proof's public values with near-antibody's records:
- the request hash (the TEE's exact input);
- the commit's blob, the base, the manifest and the anchor;
- the round, the verifier, the receipt, the verdict's kind and the keys' slots.

What equal means: the verifier, re-run on exactly the TEE's input, gave the TEE's receipt (`t2_confirmed`).
The same input with another receipt is a recorded contradiction (`t2_contradicts`).

What you trust:
- SP1's proof system;
- that the guest mirrors the TEE's `verify` path (four testnet runs reproduce, byte for byte);
- SP1 6.10.0's Groth16 key, which near-t2 has built in and `scripts/zk/sp1_groth16_vk.py` checks against
  the key SP1 ships.

What it does not do:
- It claims no privacy: the proof re-runs a disclosed proof.
- Payouts in the locked deployment do not depend on it.

On testnet (2026-11-07): near-t2 is `zk.antibody.testnet`, locked (code hash `3HFJXFtL…`, wasm sha256
`21e0d696…`: the same hash in hex). It holds four `confirmed` records, for runs 33, 36, 35 and 53, whose
content keys are published in `fixtures/zk`; a tampered proof was refused (an invalid G1 point). Each
record links its proof's request on SP1's prover network (JUDGES.md row 16,
`antibody zk status --run 33 --contract zk.antibody.testnet`).

## What is mock

- **The sandbox demo:** MOCK OutLayer, DCAP and MPC contracts, resolved by the real verifier with a TEST
  seed and a TEST secp256k1 key. Its near-t2 takes proofs under a TEST Groth16 key that anyone can make
  proofs for (`fixtures/zk/t2-test-key.json`), from a MOCK prover. The MOCK worker lies once on purpose,
  and near-t2 records the contradiction.
- **Test keys in fixtures:** the fingerprint tags in `fixtures/acme-local/expected.json` use a TEST key.

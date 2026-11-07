# T2: a validity proof of a disclosed verdict

**What T2 is.** Once a verdict's proof is disclosed, anyone can re-run the verifier on it. T2 makes that
re-run a Groth16 proof (SP1 6.10.0) that a NEAR contract, near-t2, verifies. near-t2 then compares the
proof's public values with near-antibody's own records of the run.

**What it is not.**
- It claims no privacy. The proof is about a proof that is already public, and its public values say so.
- Payouts in the locked deployment do not depend on it. near-antibody settles on T0 and T1, and T2 sits
  beside it.

**The line it supports.** The verifier gives the same receipt, byte for byte, in three places:
- natively;
- in the TEE's wasm, which CI's recorded fixtures compare with native;
- in the zkVM, for the four testnet runs whose content keys are published (`fixtures/zk`; on chain they are still sealed).

The feasibility study (G0) is [zk-feasibility.md](zk-feasibility.md). This document is G1 to G3: the
proofs, the contract, the tools.

**Status.**
- Built and tested:
  - near-t2, with unit tests on the four runs' public values against near-antibody's testnet records;
  - the sandbox step (an honest run confirmed, a lying MOCK TEE contradicted);
  - the CLI, the API and the prover service.
- Proven on SP1's prover network (2026-11-07): the four real Groth16 proofs, in `fixtures/zk/<case>/proof.bin`
  with their requests, costs and times in `proof.json`. Each verifies against SP1 6.10.0's key and fails with
  any public value changed (near-t2's tests, in CI).
- Pending: near-t2 on testnet (`zk.antibody.testnet`), which needs testnet NEAR for its deployment.
- The README and JUDGES.md list T2 as "not yet" until near-t2 holds those four records on testnet.

## The pieces

| Piece | What it does |
|---|---|
| `crates/zk/core` | The re-run: the TEE's `verify` path on the exact request near-antibody sent, opened with its content key (disclosed, or published as for `fixtures/zk`), and the public values a proof commits (layout v1) |
| `crates/zk/guest` | The SP1 program: it reads the five inputs, re-runs, and commits the public values. A run that would stop before a verdict panics, so it has no proof |
| `fixtures/zk/guest/antibody-zk-guest.elf` | The guest, built in SP1's docker image. sha256 `7fea93b8…9702`, program vkey `0x00e87aaa…8203`. Three docker builds on fresh machines gave the same bytes (the `zk` workflow) |
| `crates/zk/prove` | `antibody-zk-prove`: Groth16 proofs on SP1's prover network or a local CUDA machine. It refuses an ELF whose vkey is not the pinned one, verifies each proof locally, and writes `proof.bin`, `public_values.bin` and `proof.json` |
| `contracts/near-t2` | The contract (below) |
| `antibody zk prove / submit / status` | Gather a run from NEAR, re-run it natively, prove it, submit it, read its record |
| `antibody judge`, `/v1/t2`, `/v1/runs/{id}` | Each run's tier (T0, T1 or T2) and near-t2's records and events |
| The T2 prover service | Proves every practice try and submits it (`antibody-services all --t2-prover-account`) |

## near-t2

**Pins (set at creation; the contract has no owner and no method that changes them).**

| Pin | Value |
|---|---|
| Groth16 key | SP1 6.10.0's, built in (`vk.rs`, checked point by point against the `groth16_vk.bin` SP1 ships). `get_pins` says `is_sp1_v6_10_0`. Only the sandbox passes another key, the TEST key below |
| Program vkey | `0x00e87aaad0ca36c285eec6bccab0cace523784873ad65e45b3b65e2759288203` (the docker build) |
| near-antibody | `bounty.antibody.testnet`: a program's key is `<account>/<id>` |
| The verifier | The receipt's version string, and the sha256 of the TEE wasm that programs pin (`ccc27f0c…28c1`): the guest re-runs that wasm's core |

**`submit(run_id, public_values, proof)`, by anyone.** Attach 0.02 NEAR for the record's storage (about
0.004 NEAR is used and the rest is refunded) and about 200 Tgas.
1. The proof must verify: about 34 Tgas, with NEAR's alt_bn128 host functions.
   - It must be for this key (the selector) and the pinned vk root.
   - The exit code must be 0.
   - The public inputs must be the pinned program vkey and sha256 of these public values.
   - A proof about another run is refused before any read.
2. It reads near-antibody's records by cross-contract views: the run, its commit, the program, its
   metadata and its pins.
3. It compares every bound field (`compare.rs`).

| Field | Compared with | If it differs |
|---|---|---|
| request sha256 | the run's `request_sha256`: the proof ran on exactly the TEE's input | rejected |
| run, round, commit | the run's, the commit's | rejected |
| blob sha256 | the commit's `blob_sha256` | rejected |
| manifest sha256 | the round's manifest (the program's, or the round's) | rejected |
| base sha256 | the base witness near-antibody names for that round | rejected |
| anchor | the program's anchor block (a later round's base binds its own) | rejected |
| program key, verifier | the program's key; the pinned version and the wasm the program pins | rejected |
| each broken slot's severity | the program's price for that slot | rejected |
| receipt sha256 | the TEE's, in the run's answer | **contradiction** |
| kind | the TEE's facts | **contradiction** |
| the keys' slots | the TEE's keys (a Violation's first four broken slots, in order) | **contradiction** |

**Outcomes and events (standard `antibody-t2`).**
- Everything equal: the run is recorded as T2 and `t2_confirmed` is emitted.
- The same input with another receipt, kind or slots: the contradiction is recorded and `t2_contradicts`
  is emitted. In the locked deployment it reverses nothing. Consumers (a verdict guard, a UI) can act on
  it.
- Anything else: `t2_rejected`. Nothing is stored and the deposit goes back.
- A run gets at most one record.

**Views:** `get_pins`, `get_record(run_id)`, `list_records`, `get_stats`, and `verify_proof(public_values,
proof)`. The last checks a proof against the pinned keys only, for free.

**The wasm deployed on testnet.**
- Built with Rust 1.85.0 on Linux: `cargo build --release --target wasm32-unknown-unknown` in
  `contracts/near-t2`.
- Then `wasm-opt -Oz` (binaryen 133): 224,694 bytes, sha256 `21e0d69653f3…c99c`.
- Its storage stake, about 2.25 NEAR, is most of what the account costs.

### Why a contradiction cannot be faked

A contradiction needs a valid proof whose request hash equals the TEE's but whose receipt does not.
- The guest is deterministic, and every input is bound to the request:
  - the base, the manifest and the blob by sha256;
  - an inline blob is inside the request itself.
- The one input the chain does not hash is the content key. ChaCha20-Poly1305 is not key-committing, so
  a hunter could in principle craft a blob that decrypts under two keys. But any key that opens the blob
  must decode a payload that starts with the same fixed bytes:
  - the magic, the program key and the hunter, about 50 bytes, all checked by the re-run;
  - two ChaCha20 keystreams would have to agree on all of them under the blob's nonce;
  - that is a 400-bit condition, out of reach.
- The keys' tags are MACs under the program's key, which only the TEE holds. T2 compares their slots, not
  the tags.

What T2 relies on: SP1's proof system, and that the guest mirrors the TEE's `verify` path. Parity on the
four testnet runs, and CI's native == wasm, are the evidence for the second.

## Proving

The prover gets the docker-built ELF and five inputs:
- the request;
- the base witness;
- the round's manifest (empty for round 0);
- the blob (empty when it is inline);
- the content key.

```
antibody zk prove --run 33 --key <the content key: disclosed, or fixtures/zk/a/content_key.hex>
```

1. It finds the transaction that requested the run: `--tx`, your own commit records, or `--api`.
2. It reads the exact request from that transaction, with an archival RPC for old ones.
3. It reads the base and the blob from FastFS. Each must hash to what the chain holds.
4. It re-runs natively and compares the receipt with the TEE's. A difference is printed: a proof would
   record a contradiction.
5. It calls `antibody-zk-prove`.

`--mode none` stops after the native re-run. On run 33 that path reproduces the TEE's receipt and the
exact public values of `fixtures/zk/a`.

**SP1's prover network.**
- A requester account: a secp256k1 key in `.secrets/sp1-requester.key` (`NETWORK_PRIVATE_KEY` also
  works). It is never printed.
- Deposits are in PROVE on Ethereum mainnet. They are credited to the depositing address, and the
  explorer's Transfer moves them to the requester.
- Each proof costs a base fee plus the auction's price per prover gas unit (PGU).

| Run | Cycles | PGUs | Request to proof | Cost |
|---|---|---|---|---|
| 33 (A, Violation Critical) | 29,358,505 | 34,802,407 | 61.4 s | 0.4221 PROVE |
| 36 (D, a duplicate) | 28,581,912 | 34,096,961 | 52.2 s | 0.4220 PROVE |
| 35 (C, NoViolation) | 25,841,727 | 31,567,333 | 44.9 s | 0.4214 PROVE |
| 53 (practice) | 33,865,585 | 37,035,732 | 78.9 s | 0.4217 PROVE |

Measured on SP1's prover network on 2026-11-07 (Groth16, mainnet auction; each request is linked in its
`proof.json`). The cost is almost all the base fee (0.414 PROVE); the PGUs the network charged equal the ones
`antibody-zk-prove --mode execute` simulates with the docker ELF, whose public values equal the ones
`crates/zk/core` computes natively, byte for byte.

**Budget (the prover service).**
- At most 6 proofs an hour and 40 a day.
- It pauses, with an alert, below 2 PROVE.
- `/v1/status` shows the balance, the budget, each failure and any contradiction (critical).

## Practice tries reach T2 too

A practice try's proof is the published tutorial, so its content key is public by construction:
`sha256("antibody/practice/public-content-key/v1|<program key>|<hunter>")`.
- `antibody try` seals with it.
- Nothing has to be handed to the prover: the service derives the key itself.
- For each practice run it saw requested, the service gathers the inputs from the indexed transaction,
  proves them and submits.
- `/v1/runs/{id}` and the feed rows then show the run as T2.

The UI can rely on it. A try made through any client that uses this crate gets a key the prover service
can derive.

## In the sandbox

The sandbox has no SP1 prover, so near-t2 is deployed there with a TEST Groth16 key
(`fixtures/zk/t2-test-key.json`, written by `scripts/zk/t2_test_key.py`):
- Its β, γ and δ are all the G2 generator, so anyone can make a proof for any public values: B = G2, any C,
  A = α + L + C.
- The MOCK prover does that for real public values: the practice runs, re-run natively on the exact
  requests the MOCK worker received.
- The rest is the real contract and the real near-antibody.

In `make e2e` (CI run 42283001842):
- An honest try (run 22) is `confirmed`.
- A tampered proof is refused.
- The MOCK worker lied once: for the next practice try (run 26) it reported NoViolation, with a receipt
  hash that is not the receipt's.
  - near-antibody recorded the lie, and the try's bond was forfeited.
  - near-t2 recorded `contradicts` with `differ: ["receipt", "kind", "slots"]`: the proof says Violation.

## Reproduce

```
cd crates/zk/guest && cargo prove build --docker --tag v6.10.0 --workspace-directory ../../.. --locked
sha256sum target/elf-compilation/docker/*/release/antibody-zk-guest   # 7fea93b8…9702
cargo build --release --manifest-path crates/zk/prove/Cargo.toml
crates/zk/prove/target/release/antibody-zk-prove --mode execute fixtures/zk/guest/antibody-zk-guest.elf fixtures/zk/a
cd contracts/near-t2 && cargo test --lib     # the contract against the four runs' records
```

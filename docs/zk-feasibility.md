# T2 feasibility (G0): a validity proof of a disclosed verdict

**Status: G0, a feasibility study** (2026-11-06). G1 to G3 (the proofs, near-t2, the tools) are in
[zk.md](zk.md). The spike below became `contracts/near-t2`. The locked deployment does not use T2, no
payout depends on it, and the README and JUDGES.md list T2 as "not yet" until near-t2 holds the first
proofs on testnet.

What T2 would be: once a verdict's proof is disclosed, anyone can re-run the verifier on it. T2 makes
that re-run a validity proof that a NEAR contract checks. It claims no privacy. The proof is about a
proof that is already public, and its public values say so.

The question G0 answers: can our verifier run in SP1, reproduce the TEE's receipt, at what cost, and
can NEAR check the result?

## The answer

**Go.** Four testnet runs re-ran in SP1's executor, and each one reproduced the TEE's receipt byte for
byte. The receipt's sha256 is the one near-antibody recorded for the run. The runs are a paid
Critical, a duplicate, a NoViolation and a practice try. Each took 26 to 34 million cycles.
SP1's published Groth16 fixture proof verifies on NEAR with the alt_bn128 host functions, at
**34.2 Tgas** for the whole transaction. What G1 needs is a prover: a rented GPU, or SP1's prover
network. None of our machines meets SP1's proving minimum.

## Pinned versions

| What | Version | Why |
|---|---|---|
| SP1 | **6.10.0** (`sp1-zkvm`, `sp1-core-executor`, `cargo prove` d83ed91) | The version rsp `5afe72e` uses with our revm 38, reth 2.2 and alloy |
| Target | `riscv64im-succinct-zkvm-elf` (toolchain `succinct`, rustc 1.96.0-dev) | SP1 6's |
| Patched crypto | sp1-patches `sha2` (patch-sha2-0.10.9-sp1-6.10.0), `sha3` (patch-sha3-0.11.0-sp1-6.0.0), `k256` (patch-k256-13.4-sp1-6.10.0), `p256` (patch-p256-13.2-sp1-6.10.0), `substrate-bn` (patch-0.6.0-sp1-6.10.0-substrate-bn) | The same tags rsp's client uses. Same outputs, fewer cycles |
| Our patches | `kzg-rs`, `bitvec`, `revm-bytecode` (`third_party/patches`) | The verifier behaves as in the TEE |
| Versions of everything else | The root `Cargo.lock`, copied into the guest's | A fresh resolution picked an `alloy-trie` that reth 2.2 does not build with |

The guest has its own workspace (`crates/zk/guest`), so SP1's patched crates never reach the verifier
the programs pin. That verifier is the root workspace's, and its wasm is unchanged.

## What the program proves

`antibody-zk-core::rerun` is the TEE's `verify` path (`antibody_verifier::tee`). The one difference
is the key: it opens the sealed blob with the disclosed content key instead of the program's sealing
key. Its input:

| Input | Bound to near-antibody's records by |
|---|---|
| The exact request near-antibody sent to OutLayer (`input_data`) | its sha256 = the run's `request_sha256` |
| The program's base witness | its sha256 = the one the request names (the program's pin) |
| The round's manifest (round 0: the base's) | its sha256 = the request's `manifest_sha256` |
| The sealed blob (inline blobs are inside the request) | its sha256 = the one the request names (the commit's) |
| The content key | it opens the blob (ChaCha20-Poly1305), and the payload names the program and the hunter |

It makes every check the TEE makes, assembles the witness as the TEE does, and runs
`run_program`. A run that would stop before a verdict is not provable.

**Public values (layout v1, integers big-endian).** A NEAR contract hashes these bytes and compares
the hash with the proof's public input. It then reads the fields:

| Field | Size | Compared with |
|---|---|---|
| version | 1 | 1 |
| request_sha256 | 32 | `get_run(run).request_sha256` |
| receipt_sha256 | 32 | `get_run(run).outcome.Answered.facts.receipt_sha256` |
| blob_sha256 | 32 | the commit's blob hash |
| base_sha256 | 32 | the program's pinned base |
| manifest_sha256 | 32 | the round's manifest |
| anchor block hash | 32 | the program's anchor |
| round, commit, run | 4 + 8 + 8 | the run's |
| kind | 1 | the facts' kind (0 Violation, 1 NoViolation, …) |
| program key | 2 + n | `<near-antibody>/<program id>` |
| verifier | 2 + n | `antibody-verifier 0.1.0 revm 38.0.0 reth v2.2.0 rsp 5afe72e…` |
| broken | 1 + 2 per finding | (slot, severity 0 low … 3 critical): the keys' slots |

**What it cannot prove: the keys' tags.** A tag is a MAC under the program's key, and only the TEE
holds that key. The proof binds each broken slot and its severity. near-antibody's recorded keys name
the same slots.

**Where the inputs came from.** For each run, `fixtures/zk/` holds:
- the request, taken from the revealing transaction and checked against `request_sha256`;
- the blob, as committed;
- the base, from its FastFS upload;
- the content key, from the hunter's own commit record (our fixture agents);
- `expected.json`: what near-antibody recorded for the run.

Publishing the key discloses the proof. For A, the project gets the same key through the release wrap
(run 40, `antibody program read`).

## Parity: the TEE's receipt, reproduced

| Run | What | TEE receipt (sha256, on chain) | Native re-run | SP1 executor | Cycles (docker ELF) |
|---|---|---|---|---|---|
| 33 (commit 13, A) | Violation, Critical (slot 0), paid | `0ea67c6a…` | equal | equal | 29,358,505 |
| 36 (commit 16, D) | Violation, a duplicate, not paid | `fa62fbea…` | equal | equal | 28,581,912 |
| 35 (commit 15, C) | NoViolation, bond forfeited | `0326cfc9…` | equal | equal | 25,841,727 |
| 53 (commit 20, practice) | Violation, Low (slot 0), the tutorial proof | `e51759c6…` | equal | equal | 33,865,585 |

Both ELFs reproduce every receipt: the docker build's on GitHub, and a local build on the VPS. The local
build's cycle counts differ by up to 432 cycles, because its ELF embeds other paths.

How each column was checked:
- **Native re-run:** `cargo test -p antibody-zk-core`, run in CI.
- **SP1 executor:** `antibody-zk <elf> fixtures/zk/{a,d,c,practice}`, using `sp1-core-executor` with no
  proof. Each case executes in about 0.1 s on the VPS, at 200–316 MHz.
- **Negative check:** a wrong key, a changed request or a changed base does not reproduce the
  receipt.

The practice run costs the most cycles even though its blob is the smallest (570 bytes). The
verifier checks the whole pinned base: the practice base is 111 KB, Acme's is 79 KB.

The guest prints `WARNING: Using insecure random number generator`: SP1 seeds hash maps with a fixed
generator. The receipts show that the output does not depend on it.

## The program's identity

| | ELF sha256 | Program vkey (`cargo prove vkey`) |
|---|---|---|
| **Docker build** (`cargo prove build --docker --tag v6.10.0 --workspace-directory ../../.. --locked`, the `zk` workflow on GitHub) | `7fea93b80cc71db7ccd2315acc9b85c583fd3164bc95c6e6b68b6119d1c39702` | **`0x00e87aaad0ca36c285eec6bccab0cace523784873ad65e45b3b65e2759288203`** |
| Local build on the VPS (`cargo prove build`) | `dfeb2b9ef534d4f7ffb0b98ac3c8cba2ce1451c8fd9cfc41cc8e350d7861d8c9` | `0x000765e6238eedeea6bb4e7218798dbc4a6bf55a5322ffd6cae277f77219a5ae` |

**The local build is not reproducible across machines.** Its ELF embeds the build's absolute paths, and
so does its vkey. The docker build runs at a fixed path in SP1's own image, so it is the one a NEAR
contract pins.

Two docker builds on two fresh GitHub machines gave the same ELF and the same vkey (`zk` runs
42233167302 and 42234906992), and both reproduced the four receipts. The `zk` workflow (run by hand)
rebuilds it and prints both values. The vkey takes about 2–3 minutes on 4 cores and 7 GB. The guest's
`Cargo.lock` is committed.

## Proving: cycles to time and cost

Every case is about 30 million cycles. SP1's published requirements, at its current version, are:
- core and compressed proofs: 16+ cores and 16 GB+ RAM;
- Groth16: 16 GB+ RAM, with about 14 GB for its final wrap;
- GPU: CUDA compute capability 8.0+, 24 GB+ VRAM, CUDA 12, Linux.

The three setups:

**(a) Our machines: no proving.**
- *The VPS* (4 vCPU, 7 GB, no GPU, shared with other services) is below the minimum. It runs the
  executor and computes the vkey.
- *Semih's PC* (Ryzen 5 5600, 6 cores, 16 GB, a Radeon RX 570, Windows without a Linux distribution)
  has no CUDA GPU. CPU proving would need WSL2 at the 16 GB minimum, and the 14 GB wrap does not fit
  next to Windows.

So no proof was made in G0. The toolchain is validated up to the ELF, the executor and the vkey.

**(b) One rented GPU.**
- *Hardware:* an RTX 4090 (24 GB) or 5090 (32 GB), 64 GB RAM, CUDA 12, Ubuntu.
- *Throughput, as published:* SP1 Turbo reached 3.0 MHz with recursion on one 4090. Hypercube is
  about twice as fast: a third-party measurement put a mainnet block on one 4090 at 1 min 55 s,
  against 3.5–4 min for Turbo.
- *Estimate for 30M cycles:* core in about 10 s, compressed in tens of seconds. The Groth16 wrap
  takes 30–90 s and about 14 GB of RAM. That is about 1–2 minutes per proof once warm. The first run
  downloads the circuit artifacts (a few GB).
- *Cost:* a 4090 rents for about $0.3–0.7 an hour, so a 1–2 hour G1 session costs about $1–2.

**(c) SP1's prover network.**
- *Pricing:* a base fee plus an auctioned price per prover gas unit. The documented examples are a
  0.2 PROVE base fee (compressed mode) and a prover's calibrated bid of 0.28 PROVE per billion PGUs.
- *Estimate:* at about 30M cycles the gas part is cents, so a proof costs mostly its base fee. That
  is on the order of one PROVE per Groth16 proof, about $0.2–0.3 at 2026 prices; check the live price.
- *Privacy:* not a concern, because only disclosed proofs are proven.
- *Setup:* a requester key and a PROVE deposit.

These are estimates from published figures, not measurements. G1 measures them.

## Verifying on NEAR (spike)

`contracts/sp1-groth16-spike` is not deployed. It verifies an SP1 6.x Groth16 proof with NEAR's
alt_bn128 host functions, in code written for it (it uses no SP1 crate).

**Proof bytes (356).**
- 4 bytes: the selector, the first 4 bytes of sha256 of the Groth16 key;
- 32 bytes each: the exit code, the vk root and the proof nonce;
- 256 bytes: gnark's proof. A and C are in G1, B in G2. All are uncompressed and big-endian, with G2
  written as (x.c1, x.c0, y.c1, y.c0).

**Public inputs, in order.**
1. The program vkey hash.
2. sha256(public values) with the top 3 bits cleared.
3. The exit code (must be 0).
4. The vk root (pinned per SP1 version).
5. The nonce.

Each must be below BN254's r.

**Check.** e(A, B) · e(C, −δ) · e(α, −β) · e(L, −γ) = 1, where L = IC₀ + Σ inputᵢ · ICᵢ₊₁. It costs
one `alt_bn128_g1_multiexp` of 5 points, one `alt_bn128_g1_sum` of 2 and one
`alt_bn128_pairing_check` of 4 pairs.

**Encoding.** NEAR reads field elements little-endian and G2 as (x.c0, x.c1, y.c0, y.c1). SP1, gnark
and the EVM write them big-endian, with the imaginary part first. The proof is converted in the
contract. The keys are converted once by `scripts/zk/sp1_groth16_vk.py`, from SP1's published Solidity
verifiers.

**Keys.** SP1 6.10.0's Groth16 key is v6.1.0's (selector `0x4388a21c`, vk root `0x002f850e…`). The
script checks it point by point against the `groth16_vk.bin` SP1 ships. SP1's only published fixture
proof (`sp1-project-template`, Fibonacci n = 20) is a v6.0.0 proof (selector `0x0e78f4db`). It
verifies against v6.0.0's key. The code is the same for both keys.

**Results.**

| | |
|---|---|
| Fixture proof verifies (unit test and sandbox) | yes |
| A changed public value, program vkey or nonce | does not verify |
| Another key's proof, a nonzero exit, a short proof | refused |
| Host-function gas (unit test) | 32.47 Tgas |
| The verify call, wasm included (near-sandbox 2.14.0) | 33.63 Tgas |
| The whole transaction | **34.20 Tgas** (≈ 0.0034 NEAR at testnet's gas price) |
| Spike wasm | 95,619 bytes after `wasm-opt -Oz` |

A G2 contract adds the reads of near-antibody's records (cross-contract views) and a few comparisons.
That still fits well within one 300 Tgas call.

## What G1 needs from Semih

Either one:

1. **SP1's prover network (recommended).** A fresh secp256k1 key as the requester, holding test funds
   only. Its private key goes in `.secrets/`, never in the repository. A small PROVE deposit
   (5–10 PROVE, a few dollars) into the network. The proofs are of disclosed verdicts, so nothing
   private is sent.
2. **One rented GPU for 1–2 hours.**
   - Hardware: an RTX 4090 or 5090 with 64 GB RAM, Ubuntu 22.04/24.04 and CUDA 12.
   - Access: SSH for this repository's work, about $1–2 in total.

## Plan

- **G1, a real proof.** A Groth16 proof of run 33 (A), plus the practice run. It uses SP1 6.10.0's
  SDK, on the network or the GPU, with the vkey from the docker build. Store the proof, its public
  values and its vkey in `fixtures/zk/`. Verify the proof with `sp1-verifier` natively and with the
  spike against 6.10.0's key. Record the time, memory and cost.
- **G2, a NEAR verifier contract.** It is a separate, locked contract: near-antibody is locked, and
  T2 lives beside it.
  - It pins the program vkey and SP1 6.10.0's Groth16 key.
  - `submit(run_id, public_values, proof)` checks the proof (≈ 34 Tgas). It then reads the run and
    the commit from near-antibody and compares every hash and the slots.
  - It records the run as T2, with a `t2_confirmed` event.
  - It is deployed to testnet, locked, and listed in the deployment record.
- **G3, CLI and docs.** `antibody zk prove` (the disclosed inputs, from chain) and `antibody zk
  submit`. JUDGES.md rows, the README's tiers, and then the T2 badge in the UI (phase D).

## Files

- `crates/zk/core`: the re-run and the public values. Its parity tests run on `fixtures/zk`.
- `crates/zk/guest`: the SP1 program, with its own workspace and lockfile.
- `crates/zk/host`: `antibody-zk`, the SP1 executor (no proof), with cycles and receipts.
- `fixtures/zk/`: four disclosed testnet runs and their on-chain records.
- `contracts/sp1-groth16-spike`: Groth16 on NEAR's alt_bn128 host functions.
- `scripts/zk/sp1_groth16_vk.py`: SP1's keys in NEAR's encoding, checked against SP1's file.
- `.github/workflows/zk.yml`: the docker build, the vkey and the executor, run by hand.

Sources for the published figures:
- SP1's hardware requirements: docs.succinct.xyz, getting-started/hardware-requirements.
- SP1 Turbo's single-GPU MHz: blog.succinct.xyz/sp1-turbo.
- SP1 Hypercube's cluster and timings: blog.succinct.xyz/sp1-hypercube.
- The prover network's auction: docs.succinct.xyz, protocol/spn/auction and the prover quickstart.
- NEAR's alt_bn128 costs: nearcore's `runtime_configs/parameters.yaml`.

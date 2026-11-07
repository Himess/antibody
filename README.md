<div align="center">

<img src="web/app/icon.svg" width="64" height="64" alt="Antibody logo" />

# Antibody

**Bug bounties that pay on proof.** We don't read reports; we run them.

[Live site](https://antibody-three.vercel.app) · [Recorded demo](https://antibody-three.vercel.app/moment) · [For judges](JUDGES.md) · [For AI agents](AGENTS.md)

[![CI](https://github.com/Himess/antibody/actions/workflows/ci.yml/badge.svg)](https://github.com/Himess/antibody/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Himess/antibody)](https://github.com/Himess/antibody/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

</div>

Bug bounties judge text: a person reads every report. Once AI makes reports free to write, that stops
scaling. In January 2026, curl ended its bug bounty after a flood of AI-generated reports
([The Register](https://www.theregister.com/2026/01/21/curl_ends_bug_bounty/),
[BleepingComputer](https://bleepingcomputer.com/news/security/curl-ending-bug-bounty-program-after-flood-of-ai-slop-reports/)).

**Antibody judges code instead.** A hunter, a person or an AI agent, submits a sealed proof of an exploit.
A verifier re-runs it inside secure hardware, against the rules the project committed on NEAR. If a rule
breaks, NEAR pays the bounty automatically, and a critical bug pauses the protocol on every chain it runs
on. Nobody has to read a report.

It runs on NEAR testnet today, with every contract locked.

## Try it in two minutes

The release binary has the testnet deployment built in: no repository, no account and no key.

**1. Download it and check its sha256.**

```bash
# Linux x86_64
V=v0.5.0; F=antibody-x86_64-unknown-linux-gnu
curl -fsSLO https://github.com/Himess/antibody/releases/download/$V/$F
curl -fsSLO https://github.com/Himess/antibody/releases/download/$V/$F.sha256
sha256sum -c $F.sha256 && chmod +x $F && mv $F antibody
```

```bash
# macOS (Apple silicon)
V=v0.5.0; F=antibody-aarch64-apple-darwin
curl -fsSLO https://github.com/Himess/antibody/releases/download/$V/$F
curl -fsSLO https://github.com/Himess/antibody/releases/download/$V/$F.sha256
shasum -a 256 -c $F.sha256 && chmod +x $F && mv $F antibody
# downloaded with a browser instead of curl: xattr -d com.apple.quarantine ./antibody
```

```powershell
# Windows x86_64 (PowerShell): prints OK, or MISMATCH (then delete antibody.exe and download it again)
$V="v0.5.0"; $U="https://github.com/Himess/antibody/releases/download/$V"
Invoke-WebRequest "$U/antibody-x86_64-pc-windows-msvc.exe" -OutFile antibody.exe -UseBasicParsing
Invoke-WebRequest "$U/antibody-x86_64-pc-windows-msvc.sha256" -OutFile antibody.sha256 -UseBasicParsing
if ((Get-FileHash antibody.exe -Algorithm SHA256).Hash.ToLower() -eq (Get-Content antibody.sha256).Split(" ")[0]) { "OK" } else { "MISMATCH" }
```

On Windows, type `.\antibody.exe` wherever this page says `./antibody`. Each file also has a build
provenance attestation: `gh attestation verify <file> -R Himess/antibody`.

**2. Read the live deployment.** View calls only:

```bash
./antibody judge
```

It prints each contract's code hash and locked status, and every program's verdicts, payouts and pauses.

**3. Run a practice bounty end to end.**

```bash
./antibody try --name <you>     # any new lowercase name
```

It gets a practice account, seals the public tutorial proof, commits and reveals it, and settles it, with
a link for every transaction. It ends `Done:Paid` in under a minute when the run is attested on NEAR
(the recorded tries took 43.7 s and 39.9 s). While new worker images wait their two-day delay, the payout
waits the program's 60 s challenge window instead: about a minute and a half (90 to 137 s, measured on
2026-11-07). The output says which path it took. The same binary runs in Docker:
`docker run --rm ghcr.io/himess/antibody:v0.5.0 judge`.

## How it works

1. **A project opens a program.** Before a launch or an upgrade, it commits on NEAR its rules as code, a
   payout table and an escrow.
2. **Anyone submits a sealed proof:** a sequence of calls, with a small bond. Nobody, the project
   included, can read it before its verdict.
3. **A verifier re-runs it inside an enclave** (a TEE), on chain state proven at the program's anchor
   block, and reports which rule broke, where, and the impact.
4. **NEAR pays automatically.** The first proof of each bug (each rule and entry point) is paid, in
   seconds below the program's instant cap. A proof that breaks nothing loses its bond to the program's
   escrow.
5. **A critical bug pauses the protocol on every chain** it runs on, and the project's fix ships only
   once every paid proof fails against it.

**What NEAR makes possible:**

- **One contract settles everything.** near-antibody holds each program's terms and escrow, and applies
  them to every verdict in seconds.
- **Hardware attestation, verified on chain.** NEAR checks the enclave's Intel TDX quote before an
  instant payout.
- **Pauses on other chains.** NEAR's MPC network signs the pause and release permits that the protocol's
  Ethereum contracts accept.
- **Proven state.** The Ethereum light client on NEAR confirms the block every proof runs on.
- **Agents that cannot run off with funds.** An AI agent works through a NEAR sub-account that cannot
  withdraw, within a bond budget.

## The one moment

On 2026-11-04, on the locked testnet deployment, our scripted test agent A submitted over MCP a sealed
proof of a critical bug in Acme Lend, our deliberately vulnerable test protocol on Sepolia and Base
Sepolia. Counted from the block of its commit:

| Time | What happened |
|---:|---|
| 18.5 s | The enclave's verdict: a critical rule broke |
| 29.1 s | The verdict attested on NEAR |
| 71.0 s | 0.1 wNEAR paid to agent A |
| 90.6 s | The protocol paused on both chains |

Replay it with every transaction: [antibody-three.vercel.app/moment](https://antibody-three.vercel.app/moment).
Check every claim yourself: [JUDGES.md](JUDGES.md) gives each one a command, the output to expect and the
transaction behind it.

## How a verdict is checked

Every verdict carries one of three levels. The CLI and JUDGES.md print them as T0, T1 and T2.

| Level | What vouches for it |
|---|---|
| **Enclave result** (`T0`) | OutLayer's operator delivers the enclave's output. A payout waits the program's challenge window, and above the instant cap a second, independent run must agree. |
| **Attested on NEAR** (`T1`) | The run's Intel TDX quote, which binds the pinned verifier, its input and its output, is verified on NEAR. Instant payouts need it. |
| **Zero-knowledge proof** (`T2`) | An SP1 proof that re-running the verifier on the same input gives the same result, checked on NEAR by near-t2. Live on testnet since 2026-11-07, for disclosed verdicts; no payout depends on it. |

All three rest on the same determinism: the native, enclave and zero-knowledge builds of the verifier give
the same receipt, byte for byte. Details and checks: [docs/trust.md](docs/trust.md).

## Bring your own proof

Write a proof yourself, by hand or with an agent, still with no account. `hunt start` gets an account and
writes a workspace (the rules, the sources, a Foundry harness, the proof format), `check` runs the verifier
on your machine, and `hunt submit` seals, commits and settles your proof:

```bash
./antibody hunt start --name <you> --vending practice.antibody.testnet
./antibody check --program 6 --bundle hunt-<you>/proof.bundle.json
./antibody hunt submit --name <you> --vending practice.antibody.testnet --poc hunt-<you>/proof.bundle.json
```

On testnet (2026-11-07): two accounts paid on two vaults, a proof that breaks no rule ending `NoViolation`
with its bond forfeited, and a malformed one refused as `InvalidPoC`
([results](docs/results/byo-2026-11-07.json)).

## For AI agents

`antibody mcp` serves the same loop as MCP tools; `hunt_start`, `check_poc` and `hunt_submit` need no
account. Every result is marked as untrusted data, and file access stays inside a workspace. An agent
account is a NEAR sub-account that cannot withdraw, with a bond budget and a per-commit cap. Setup:
[AGENTS.md](AGENTS.md). Measured runs of an off-the-shelf agent completing the loop:
[docs/agent-runs.md](docs/agent-runs.md) (not run yet).

## For projects

- **Rules as code:** an `IAntibodyInvariants` contract or Foundry `invariant_*` functions
  ([docs/foundry.md](docs/foundry.md)). The drafter proposes a first set
  ([docs/invariants-drafting.md](docs/invariants-drafting.md)), and `antibody invariants validate`
  checks a manifest before it goes on chain.
- **Rule strength:** `antibody invariants strength` counts the code mutants your rules catch; Acme's
  catch 8 of 28 ([docs/rule-strength.md](docs/rule-strength.md)).
- **No staging deployment needed:** fork mode deploys the program's contracts inside the verifier
  ([docs/fork-mode.md](docs/fork-mode.md)).
- **NEAR apps can act on verdicts,** like a treasury that freezes on a critical one
  ([docs/verdict-guard.md](docs/verdict-guard.md)).
- **Immune memory on pull requests** (a prototype): a GitHub Action re-runs every paid proof it can read
  against a pull request's build, and fails the check if one still breaks a rule
  ([docs/immune.md](docs/immune.md)).

## Checked and attacked

- **12 settlement properties,** each a Quint invariant checked by Apalache and 20,000 random traces, and
  replayed against the rule core near-antibody runs; **41 Kani proofs** of the payout arithmetic and every
  escrow movement ([docs/properties.md](docs/properties.md)).
- **25 findings from attacking our own system,** each fixed and pinned by a test, and what every attack
  costs the attacker ([docs/attacks.md](docs/attacks.md); threat model:
  [docs/threat-model.md](docs/threat-model.md)).
- **Fuzzing and golden vectors:** the parsers (manifests, witnesses, sealed payloads) are fuzzed, every
  format two components share has golden vectors, and the native and wasm verifiers agree byte for byte
  on every fixture.
- **CI on every change:** the model and its trace replay, the Kani proofs, the verifier (native and wasm),
  the contracts, the site, and the whole story end to end in a sandbox
  ([latest runs](https://github.com/Himess/antibody/actions/workflows/ci.yml)).

## Run it from source

The whole story also runs offline, in a sandbox with mocked external services (Linux or macOS). This
builds everything, runs the story with every outcome asserted (about nine minutes), then keeps the
sandbox up:

```bash
make serve
```

What it runs and how to query it: [docs/sandbox.md](docs/sandbox.md). The site: [web/README.md](web/README.md).
The architecture: [docs/architecture.md](docs/architecture.md).

## Deployment

On NEAR testnet, every contract locked (no full-access keys):

| Contract | Account |
|---|---|
| near-antibody: programs, verdicts, payouts | `bounty.antibody.testnet` |
| near-guardian: pause and release permits | `guardian.antibody.testnet` |
| Practice vending: accounts for the practice program | `practice.antibody.testnet` |
| Verdict guard: a NEAR app that reacts to verdicts | `guard.antibody.testnet` |
| near-t2: zero-knowledge proofs | `zk.antibody.testnet` |

Code hashes, the verifier build and the source commit are pinned in
[deployments/near-testnet.json](deployments/near-testnet.json); `./antibody judge` reads them live.
Full table: [docs/deployment.md](docs/deployment.md).

## Limits

Testnet only: wNEAR on testnet has no value, and Acme Lend and the practice vaults are our own deliberately
vulnerable test fixtures. Only rule breaks are paid automatically, a proof runs within one block, and
Ethereum (Sepolia) is the chain proofs run against; other chains are pause and release targets. The full
list: [docs/limitations.md](docs/limitations.md).

## Roadmap

- **Pull requests:** sealed proofs re-run inside the enclave, and a bounty window per pull request (the
  immune-memory check is a prototype today).
- **Zero-knowledge by default** above a payout cap (near-t2 is live on testnet).
- **NEAR contracts** as a second target VM.
- **Multi-block proofs** with a bounded time warp.
- **The mainnet path:** external audits, coverage partners, the fee model in production.

All 17 items, with what is done: [docs/roadmap.md](docs/roadmap.md).

## Hackathon

- **Tracks:** 06 AI Security & Defense (primary): proofs, not reports, judged in an enclave, with a pause
  on every chain. 07 Agent Economy: agents earn on proof, through accounts that contain them.
- **Built during the hackathon:** all code was written after the October 19, 2026 announcement, and the
  commits reflect when it was written. Tags: `hackathon-start`, milestone tags, and `hackathon-submission`
  at submission.
- **AI-assisted:** written with Claude Code (Claude Opus 5.5). The design, the rules and every decision on
  scope were reviewed by the team.
- **Business model:** the requester pays verification at cost. Revenue is a fee on payouts and a fee per
  release permit, both pinned per program, and later a paid tier for the drafter, rule strength and
  analytics. Forfeited bonds are never revenue: they go to the program's escrow.

<details>
<summary><b>Libraries and tools</b></summary>

Direct dependencies, by license:

- MIT or Apache-2.0: alloy, reth 2.2, rsp, serde, serde_json, clap, reqwest, borsh, sha2, hkdf,
  chacha20poly1305, k256, getrandom, near-sdk 5.9, proptest, futures-util;
- MIT: revm 38, axum, tokio, bincode;
- BSD-3-Clause: ed25519-dalek, x25519-dalek;
- Apache-2.0 with LLVM exception: wasmtime 28.
- T2 (`crates/zk/*`; no payout depends on it), MIT or Apache-2.0: SP1 6.10.0's `sp1-zkvm`,
  `sp1-core-executor` and `sp1-sdk` (the prover client, `crates/zk/prove`), and sp1-patches' `sha2`,
  `sha3`, `k256`, `p256` and `substrate-bn` (the versions rsp uses).
  - `contracts/near-t2` uses no SP1 crate. Its Groth16 check is written for NEAR's alt_bn128 host
    functions, and its tests read SP1's published verifying keys and fixture proof.
  - `substrate-bn` (MIT or Apache-2.0) also runs the sandbox's MOCK T2 prover.
- Tools: Foundry, Quint and Apalache, Kani, near-sandbox, SP1's `cargo prove` 6.10.0; proofs come from
  SP1's prover network.

Patches: `third_party/patches/` holds two small build fixes (for `kzg-rs` and `near-vm-runner`).

</details>

## Repository layout

| Path | Contents |
|---|---|
| `crates/verifier` | The deterministic verifier: program mode, rounds, fork mode, receipts, TEE modes, sealed submissions |
| `crates/witness` | Witness builder: traces a proof through the verifier's own path, base witnesses, submissions from the base |
| `crates/rules` | The settlement rule core near-antibody runs; Kani proofs |
| `crates/cli` | `antibody`: CLI, MCP server, practice, judge view, rule strength, Foundry compatibility, drafter |
| `crates/services` | Indexer, API, keeper, relayers, notifier, monitoring, sandbox mocks, the demo |
| `crates/zk` | Zero-knowledge proofs of disclosed verdicts (SP1): the guest program and the prover client |
| `contracts/near-antibody`, `near-guardian`, `near-t2`, `practice-vending`, `practice-account`, `verdict-guard` | The NEAR contracts |
| `contracts/evm` | Acme Lend (deliberately vulnerable), its rules, proofs and fixes; the practice vaults; the adapter |
| `web/` | The site: the one moment, programs, commits, the browser try, judges' checks, trust, status |
| `formal/` | The Quint model of the settlement rules |
| `fixtures/`, `golden/` | Recorded witnesses and verdicts; golden vectors |

## Safety

Acme Lend and the practice vaults are **deliberately vulnerable test fixtures**, with textbook bugs labeled
in their code. Their proofs exist only for those bugs, and everything runs on testnets or local chains.
Antibody verifies; it does not hunt. See [SECURITY.md](SECURITY.md). Keys live only in the git-ignored
`.secrets/` and `.env`, and gitleaks runs in the pre-commit hook and in CI.

## License

MIT (see [LICENSE](LICENSE)). Third-party code is used under its own licenses.

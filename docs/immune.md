# Immune memory on pull requests (prototype)

A program's paid proofs are its immune memory. Inside a program, every paid proof is re-run against each
proposed fix, and a fix ships only when all of them fail (docs/rounds.md). `antibody immune` brings the same
check to every pull request, before any escrow: it re-runs each paid proof the project can read against the
pull request's compiled code. If one still breaks a committed rule, the pull request re-opens a paid hole and
the check fails. Ship only what survived, continuously.

This is a prototype of roadmap item 1: readable proofs, re-run by the local verifier. Re-running sealed
proofs inside the TEE, and a bounty window per pull request, are next and not built.

```
forge build
antibody immune --json immune.json --md immune.md
```

It sends nothing: view calls on NEAR, then local verifier runs.

## What a run does

1. **Reads the program** with view calls: its round-0 manifest and its current round's manifest (each checked
   against the hash near-antibody holds), its pinned base witnesses (the activation's and each round's, each
   checked against its pinned hash), and its commits.
2. **Builds the pull request's code.** For each contract the mapping names, the runtime code comes from its
   Foundry artifact: the creation code with the mapping's constructor arguments, run at the target's own
   address on an empty state, under the anchor's EVM rules. A constructor may set immutables; it may not read
   other contracts (give that contract's runtime code instead).
3. **Assembles the code under test** as a round's fix is committed: the program's round-0 manifest with code
   overrides at the same anchor. A target the repository builds gets the pull request's code; every other
   target keeps the current round's code (the committed fix) or, without one, the anchor's. The rules are the
   current round's. `--manifest-out` writes it: it is the round manifest the pull request would propose.
4. **Opens each paid proof it can read.** Paid proofs are the commits paid or owed: paid, holding a key,
   promoted, or a valid claim left unpaid (an exhausted budget, a default at exit). A proof opens through:
   - **disclosure**: the program's disclosure keys (`request_disclosure`, spec §11.6) open the content key
     from the verify run's `wrap_d`; anyone can read these;
   - **a published content key**, given in the mapping or with `--content-key`;
   - **the project key**, through the release wrap a paid finding already has (`antibody program read`
     requests it once; the immune check never sends a transaction).

   The sealed blob comes from the mapping's `blobs` directory (any file, matched by sha256: a blob is public
   once revealed, its key is not) or from an Antibody API (`--api`).
5. **Reproduces the proof as the TEE ran it**: the submission merged with the base witness of its round,
   under that round's manifest. The local receipt's hash must equal the one the TEE committed to on chain.
6. **Re-runs it against the pull request's code**: the same submission, merged with every base witness the
   program pinned, under the code under test.

## Results

| Result | Meaning |
|---|---|
| still breaks | A committed rule breaks. The report names the rule, its severity and the entry point (target and function, named from the artifacts) |
| fixed | The proof ran and no committed rule broke, and its run as the TEE ran it reproduced the TEE's receipt |
| inconclusive | The re-run could not judge the proof, and the report says exactly why: the state the code read that the proof's witness does not prove (an account, a storage slot, a code hash, a block hash), rules that could not be evaluated, the program's checks failing at the anchor with this code, or a receipt that did not reproduce. Never counted as fixed |
| not re-run (sealed) | The proof cannot be read here; the report says why (not disclosed, no published key, no project key or no release wrap yet) |

Exit codes: **0** no re-run paid proof breaks a rule; **2** a paid proof still breaks a rule; **3** none
breaks, but a re-run was inconclusive (with `--strict`, also when a paid proof could not be read); **1** the
check could not run. `--json` and `--md` write the report (the Markdown is a job summary).

A proof read with the project key is still sealed. The report names a rule it breaks only when the commit's
keys on chain already name that rule's slot; any other rule it breaks now (the code changed, or a later round
added the rule) shows as `another rule (withheld)`. It withholds where, the entry point its on-chain tag hides,
and what its run read. `--reveal-sealed` shows them all, for a local run.

## The mapping

The project commits `.antibody/immune.toml`. Relative paths start at `--root` (the current directory).

```toml
program = 5                 # the program's id on near-antibody
artifacts = "out"           # Foundry's output directory
blobs = ".antibody/proofs"  # revealed sealed blobs, found by sha256

[content_keys]              # content keys published with a proof: commit = hex, or a file
13 = ".antibody/keys/13.hex"

[[contract]]
contract = "AcmeLendFinal"  # the artifact <artifacts>/<File>.sol/AcmeLendFinal.json (or `artifact = <path>`)
target = "AcmeLendImpl"     # the program's target whose code it replaces: a name or an address
args = ["AcmeGuard", "acUSD", "acETH", "AcmePool", "2000000000000000000000"]
```

Constructor arguments are static ABI values: an address (`0x…`, or a target's name, which stands for its
address), a number (an integer, or a decimal or `0x` string), a bool, or `bytesN` as hex. `args_hex` takes the
encoded arguments instead, for dynamic types; `runtime = <file>` takes the runtime code itself (hex). Map the
implementation behind a proxy, not the proxy: a target's storage stays the anchor's.

**`.antibody/` is also the CLI's private state directory.** Run in the repository, `antibody` keeps there the
content keys and salts of commits (`submissions/`, `known/`), account keys (`practice/`, `hunt/`) and
witnesses with their salts (`witness-cache/`). None of that may be committed: the project's `.gitignore` lets
only the immune memory's files through.

```gitignore
.antibody/*
!.antibody/immune.toml
!.antibody/proofs/
!.antibody/keys/
```

`.antibody/keys/` holds content keys already published, nothing else.

## Trust

- **The verifier is the pinned verifier's code.** The CLI links `crates/verifier`, the crate the TEE's pinned
  wasm build is made from, used as a library as it is; CI checks that its native and wasm builds give the
  same verdicts on the recorded fixtures. Each proof's own receipt is reproduced first, so a run that does not
  match the TEE's never counts as fixed.
- **The state is proven.** Every account and slot a re-run reads comes from the proof's witness and the
  program's base witnesses, checked against the anchor's state root. Code that reads anything else stops the
  run as inconclusive, naming what is missing; nothing is fetched or guessed.
- **The code is the build.** Overrides use the verifier's own round mechanism: the pull request's runtime code
  at the target's address, with the anchor's storage. Acme Lend's final fix, built from its sources through a
  mapping like the one above, hashes to exactly the code its release permits installed on testnet
  (`0x7deb43a5…`, `0x8281ee18…`).
- **The binary is the release's.** The GitHub Action checks the binary's sha256 against the release's, and
  again just before it runs it.

What it does not show: that the pull request has no new bug. It shows that each paid exploit it can read
stops working. Finding the next bug is what a program's hunters are paid for.

## The GitHub Action

`.github/actions/immune-memory` downloads and checks the release binary, installs Foundry, runs `forge
build` and `antibody immune`, writes the Markdown to the job summary and fails the job when a paid proof
still breaks a rule ([its README](../.github/actions/immune-memory/README.md)).

- **Use the `pull_request` event, never `pull_request_target`**: `pull_request_target` runs with the
  repository's secrets on the pull request's code.
- **The project key, if used, is a repository secret.** GitHub does not give secrets to pull requests from
  forks, so those re-run disclosed proofs (and published ones) only, and the summary lists the rest as not
  re-run.
- **Any code the job runs can reach a secret given to the job**, the pull request's `forge build` included: it
  follows the pull request's `foundry.toml`, which can name a program to run as the compiler. The step that
  gets the key checks the binary's sha256 again first, which catches a replaced binary, not a runner the
  build took over. The safer pattern is two jobs: build with no secret, then run the checked binary with the
  key in a job that runs none of the pull request's code (`build: false`; the README shows both jobs).

## Limits

- **Sealed proofs outside the TEE.** A proof the run cannot read is listed, not re-run. Re-running sealed
  proofs inside the TEE, where the result is all anyone sees, is next.
- **A bounty window per pull request** (a fork-mode round for the pull request's build) is next.
- **One anchor.** The re-runs use the state the proofs proved at the anchor; code that reads other state is
  inconclusive, and a change of storage layout cannot be expressed as an override.
- **Builds.** Contracts that link libraries, and constructors that read other contracts, need `runtime`.
- **EVM targets only**, as programs are today.

## Evidence

- `crates/cli/src/immune.rs`, tests with no network: the override derivation from artifacts and the mapping;
  the code under test (only what the build replaces, under the current round's rules); disclosure keys
  opening `wrap_d`, and release wraps opening with the project key only; the results and exit codes; the
  Markdown, sealed details withheld, and a sealed proof's rules named only where its commit's keys name them.
  End to end on the recorded fixture (`fixtures/acme-local`): bug B's paid proof still breaks `fees_to_treasury` at
  `AcmeVault.sweepFees(address)` on the vault as deployed and breaks no rule on `AcmeVaultFixed`, each built
  from an artifact through the mapping; its receipt, assembled as the TEE ran it, is the fixture's; and a
  witness that does not prove what the code reads is inconclusive, naming it.
- On testnet (2026-11-07, view calls only): program 5's commit 13 (A, paid, its content key published in
  `fixtures/zk/a`) reproduces the TEE's receipt and is fixed on the final fix built from source. Commits 14
  and 18 are listed as not re-run until their tags are disclosed (both qualify), and 19, a gap claim with no
  tag to disclose, as sealed.

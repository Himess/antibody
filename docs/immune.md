# Immune memory on pull requests

A program's paid proofs are its immune memory. Inside a program, every paid proof is re-run against each
proposed fix, and a fix ships only when all of them fail (docs/rounds.md). Immune memory brings the same
check to every pull request: if a paid proof still breaks a committed rule against the pull request's code,
the pull request re-opens a paid hole and the check fails. Ship only what survived, continuously.

A pull request is checked two ways (roadmap item 1):
- **On the runner** (`antibody immune`, the `immune-memory` Action): each paid proof the project can read,
  re-run by the local verifier. It sends nothing. The sections from [What a run does](#what-a-run-does) to
  [Evidence](#evidence) describe it.
- **In the enclave, through near-antibody's rounds** (the `immune-enclave` Action): the pull request's code is
  proposed as the next round of a program kept for these checks, and every paid proof, sealed or not, re-runs
  inside the enclave. Only which keys still break is published. It works on testnet:
  [Re-runs in the enclave](#re-runs-in-the-enclave).

Both catch a paid defect coming back, never a new one. A new defect is what a bounty window per pull request
is for: built, not yet run on testnet ([Bounty windows](#bounty-windows)).

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
- **A bounty window per pull request** is built (below) and not yet run on testnet.
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

## Re-runs in the enclave

The check on the runner re-runs only the proofs it can read. The `immune-enclave` Action re-runs every paid
proof, sealed or not, inside the enclave, through near-antibody's rounds: the same machinery that checks a
project's own fix (docs/rounds.md).

1. **A job with no secret builds the pull request's code** and writes it as a round manifest (`antibody
   immune --manifest-out`, the `immune-memory` Action's `manifest-out`).
2. **A job that runs none of the pull request's code proposes it** as the next round of a program kept for
   these checks (`antibody round propose`). Its key is limited to `propose_round` and the manifest's upload.
3. **near-antibody's round check runs in the enclave.** Once it answers `ok` and is attested on NEAR (T1), the
   keeper requests a re-run of every paid proof at that round. Each re-run runs inside the enclave and is
   attested on NEAR.
4. **The check reads near-antibody** (`antibody round results --round R --check-run RUN`). Per paid key the
   answer is: still breaks, fixed, or no verdict yet. Only which keys still break is published; where a proof
   breaks stays sealed (SA-19).
5. **A bot key limited to `open_round` and `allocate` opens the round** once its re-run window ends, so the
   next pull request finds the slot free.

**What it trusts.**
- The enclave's runs, attested on NEAR (T1), and near-antibody's round rules.
- The pull request's code is built where no secret is, and the keys never meet that code.
- A pull request from a fork gets no secret and runs the check on the runner only.

**A program of its own, and why.** Anyone may open an accepted proposal once its windows allow, so a pull
request proposed on a real program could become that program's next round. The checks therefore propose on a
program kept for them, with paid proofs of its own (a proof is bound to its program).
- On testnet this is program 12: Acme Lend at new addresses, its findings submitted and paid as in the recorded
  run, and fixed in rounds 1 and 2.
- Every check opens its round there. That program's current code is therefore the last checked pull request's.
  Its paid proofs are re-run against every proposal, whatever the current round.

**One proposal at a time.** An accepted proposal is cleared only by opening its round, so checks queue (the
Action waits for the slot).
- A manifest built while another pull request held the slot names a round that has opened since. `round
  propose` renumbers it to the round it proposes; only the number changes.
- A check reads its own round alone. A round check near-antibody refused is an error, never a pass.
- `round results` reads each commit's latest re-run, so query a round while it is the latest.

**Measured on testnet** (2026-11-08, [Himess/antibody-immune-demo](https://github.com/Himess/antibody-immune-demo),
program 12, three paid keys and one gap claim):

| | A harmless change (PR #1) | Reintroduces a paid defect (PR #2) |
|---|---|---|
| Round and its check | round 3, run 115: `ok`, T1 16 s after the request | round 4, run 121: `ok`, T1 15 s after the request |
| Re-runs | runs 116–119, all T1: every key fixed | runs 122–125, all T1: run 123 breaks slot 1 again (`fees_to_treasury`, high); the two `solvency` keys fixed |
| Verdict | green | red, naming `fees_to_treasury` |
| Every re-run answered | 141 s after the round check's request | 119 s after the round check's request |
| Job, until its round opened | 12 min 46 s | 11 min 2 s |

PR #2's first run was proposed while PR #1 held the slot. Its manifest still named round 3, near-antibody
refused it (run 120: "wrong round number"), and that version of the check read PR #1's round as green. The
renumbering and the per-proposal reading above came from that run.

**Costs.** The project's NEAR credit paid 0.102 NEAR for 11 runs (three round checks and eight re-runs). That
is about 0.009 NEAR a run after OutLayer's refunds, or about 0.05 NEAR for one check (its round check and four
re-runs).
Each run's TDX quote is relayed to NEAR separately, at about 0.018 NEAR of gas a run. The proposal's gas
comes from its key's allowance.

## Bounty windows

Immune memory re-runs what was already paid for. A bounty window pays hunters, people or agents, to look for
what was not: **a window is a fork-mode program for the pull request's build; X wNEAR on testnet, which has no
value; proofs committed before its deadline are judged and paid.** The site, the commit statuses, the webhook
message and MCP's `list_programs` all say it in these words.

Built: `antibody window open|status|close|list` (`crates/cli/src/window.rs`) and two workflows for the demo
repository (`examples/immune-demo`, to copy). Not yet run on testnet.

### What happens

1. Someone with write access labels a pull request `bounty-window`. `window.yml` refuses a pull request from a
   fork, and a labeller whose permission (from the GitHub API) is not write or admin.
2. A job with no secret checks out the pull request's head and the base branch and runs `forge build` on both.
3. A job that runs none of the pull request's code, and holds the bot account's key, reads `.antibody/window.toml`
   from the base branch and runs `antibody window open`:
   - **The anchor** is the newest Sepolia block NEAR's Ethereum light client holds (`last_block_number`, then
     `block_hash_safe`), checked against the Ethereum RPC's header for that block. `create_program` asks the
     same light client, so the anchor is confirmed at creation and nothing waits.
   - **The deployment** (docs/fork-mode.md): the config's transactions from the fork-mode deployer, in order: the
     pull request's contracts with their constructor arguments, the calls that seed them, and the rules contract
     last, from the **base branch's** build, so a pull request cannot weaken the rules its own window is judged by.
     `cmd::deployed_code` runs the deployment at the anchor for the targets' code hashes and the rules' runtime
     code (immutables included), as `fork_practice_manifest` does for the practice program in fork mode.
   - **The terms**: Low, Medium and High only (a fork-mode program has nothing to pause, so no Critical and no
     pause slots); a T1-confirmed finding up to the largest payout is paid in seconds, others after a 5-minute
     challenge window; the deadline `hours` after the manifest is written, just before creation; reveal 10
     minutes, exit grace 10 minutes; no gap lane
     and no known issues. near-antibody's bounds are checked before anything is sent (`Params::check`).
   - **The budget guard**, then the program is created as the bot (the escrow with `ft_transfer_call`),
     activated (its base witness pinned; if no relayer has made the activation T1 within 45 s, the bot relays
     the TDX quote itself, about 0.018 NEAR) and opened.
   - Its name: `PR #12 window (antibody-immune-demo @ 1a2b3c4)`, the head's short hash in it.
4. **Announced**: a pending commit status on the head, `Bounty window open until <UTC>, <X> wNEAR at stake
   (testnet)`, in the context `antibody/bounty-window (testnet)`; a message to each webhook in `ANTIBODY_WEBHOOKS`;
   `window: true` on the program in MCP's `list_programs` (and kind `window` in `antibody judge`); the
   "Pull-request window" label on the site's programs page and program page.
5. **Every 15 minutes**, `window-close.yml` lists the bot's windows for the repository (`window list --project
   <bot> --repo <repo>`), reads each with `window status`, and sets the head's final status:
   - green: `Held <N> h, <X> wNEAR at stake, 0 valid findings`;
   - red: `<k> valid findings (<tiers>), merge blocked`, each finding's severity and tier (`2 valid findings
     (High T1, Medium T0), merge blocked`; one finding reads `1 valid finding (…)`), linked to the program page,
     and each commit's page in the job summary.

   Once the deadline and the exit grace have passed, `window close` expires unrevealed commits past their
   reveal window, times out stuck runs, allocates and pays, then exits (permissionless: the escrow's rest goes
   to the bot's token credit), closes the records and withdraws the credit to the bot's wallet. The escrow comes
   back minus the payouts.

```
antibody window open --config .antibody/window.toml --artifacts out --pr 12 --repo antibody-immune-demo --head <sha> --dry-run
antibody window status --program <id>
antibody --account <bot> window close --program <id>
antibody window list --project <bot> --repo antibody-immune-demo
```

`--dry-run` writes the manifest and prints what the program would be: no key, nothing sent. It still reads NEAR
(the light client) and the Ethereum RPC (the deployment at the anchor).

### The window config

The repository commits `.antibody/window.toml` (let it through `.gitignore` as `!.antibody/window.toml`). The demo
repository's is `examples/immune-demo/window.toml`; the unit tests' is `fixtures/window/window.toml`.

```toml
artifacts = "out"                 # Foundry's output directory
repository = "antibody-immune-demo"
hours = 24                        # the deadline, from creation
escrow = "0.03"                   # wNEAR on testnet (no value)
daily_cap = 4                     # windows the bot opens in 24 hours, at most

[[tx]]                            # a creation: `name` (a target unless `target = false`) and its artifact
name = "AcmeVault"
contract = "AcmeVaultFixed"
args = ["AcmeGuard", "acUSD", "AcmeLend", "AcmeTreasury", "deployer"]

[[tx]]                            # a call: an earlier contract, a function and its arguments (or `data`)
call = "AcmeVault"
function = "deposit(uint256)"
args = ["1000000000000000000000000"]

[rules]                           # created last; its runtime code is what the program commits
contract = "AcmeInvariants"
args = ["AcmeLend", "AcmeVault", "acUSD", "AcmeTreasury", "2000000000000000000000"]

[[slot]]
bit = 0
name = "solvency"
severity = "high"                 # low, medium or high

[payouts]                         # wNEAR per severity
high = "0.01"
```

Arguments are those of the immune mapping, plus `deployer` (the fork-mode deployer's address), a contract created
before it by its `name`, `string`, and `bytes` as hex. `args_hex` takes encoded constructor arguments; a call's
`data` takes its calldata; `value` sends wei. Optional: `description`, `bond` and `challenge_bond` (wNEAR),
`min_near`, `[impact]` (`protocol_accounts` by name, default every target; `native_usd`; `[[impact.token]]` with
`symbol`, `contract`, `decimals`, `usd`), `[capabilities]` and `[windows]` (`reveal`, `challenge`, `resolve`,
`infra_grace`, `run_timeout`, `exit_grace`, in seconds). The rules contract must be in the base branch's build and
the config in the base branch: merge them before the first window.

### Verdicts

| Verdict | When | Commit status |
|---|---|---|
| `opening` | created, activation not confirmed yet | pending |
| `open` | open, before the deadline | pending |
| `settling` | after the deadline, no valid finding yet, and a proof committed before it still in flight | pending |
| `green` | after the deadline: no valid finding, nothing in flight | success |
| `red` | after the deadline: at least one valid finding | failure |
| `void` | the anchor was rejected, the program failed its checks, or it never opened by its deadline | error |

A valid finding is a commit whose verdict is a Violation that holds its keys: paid, owed, or with its rule's budget
spent. A duplicate repeats a finding already counted and is not counted again. The tier is the verdict's (T0, or T1
when its TDX quote was verified on NEAR).

### Guards

- **One window per pull request head.** The name carries the head's short hash: opening the same head again
  reports the open window, resumes one still in setup, and is refused once it has run. A new commit gets a window
  when the label is added again.
- **Forks are refused**, and so is a labeller without write access. GitHub gives no secret to a fork's workflow,
  and a fork's code must not be built where the key is: a maintainer can push the change to a branch of the
  repository and label that pull request.
- **The budget guard**, before each window: fewer than `daily_cap` (`--daily-cap`) windows in the last 24 hours,
  the escrow in the bot's wallet and credit, `min_near` NEAR liquid (default 1), and the escrow at most
  `--max-escrow` (the workflow's default 0.05).
- **The terms are the base branch's**: the config and the rules contract come from it, the pull request's build
  only supplies the contracts under test.
- `window-close.yml` counts only the bot's own windows: anyone can name a program like a window, and nobody else
  can sign as the bot.

### The key

- The bot account needs a **full-access key**: creating a program sends the escrow with `ft_transfer_call` and
  one yoctoNEAR, and recovering it needs `withdraw`, which also attaches one yoctoNEAR. Use an account that holds
  only what windows need.
- The key lives only in jobs that run none of the pull request's code: the open job, in the environment
  `bounty-window` (give it a required reviewer), and the scheduled close job, in the environment
  `bounty-window-close` (deployment branches: the default branch only; a reviewer there would hold every run).
- `pull_request`, never `pull_request_target`. On `pull_request` the workflow file itself comes from the pull
  request, so anyone who can push a branch could make a run read the key: the reviewer approves each run.
- The project key, which reads findings paid to the window (`antibody program read`), is derived from the bot's
  key (NEP-413): whoever holds the bot's key can read them.
- Secrets: `ANTIBODY_WINDOW_KEY` (the near-cli key file's JSON), `SEPOLIA_RPC`, optionally `ANTIBODY_WEBHOOKS`;
  variables: `ANTIBODY_WINDOW_ACCOUNT`, optionally `ANTIBODY_WINDOW_MAX_ESCROW`, `ANTIBODY_WINDOW_DAILY_CAP`,
  `ANTIBODY_SITE`.

### Costs (estimated from the measurements, not measured for windows yet)

- **The escrow**, returned at exit minus the payouts.
- **NEAR, about 0.1 per window**: the program's storage at near-antibody (from the bot's NEAR credit, kept), the
  activation run's deposit and answer allowance (0.02 + 0.01; OutLayer refunds the compute it did not use), the
  manifest and base witness uploads, the transactions' gas, and the activation's T1 relay (0.0176) when the bot
  relays it. The gas set aside while a call runs (0.3 NEAR for the creation, 0.2 for the activation) comes back.
- **An Ethereum RPC** that serves `eth_getProof` at the light client's block, some 60 to 130 blocks behind the head:
  public endpoints often refuse proofs that far back (`SEPOLIA_RPC`).
- **GitHub Actions**: two `forge build`s per window, and a run every 15 minutes.

### Limits

- **Not yet run on testnet.** The unit tests run the fixture's deployment and the verifier's activation checks
  with no network, and a proof against the fixture breaks its rule.
- **One build per window.** The deployment is the config's, with the pull request's contracts; a contract that
  links libraries, or whose deployment reads state the anchor does not have, needs its deployment written out.
- **The base branch's rules.** A pull request that changes the rules is judged by the old ones until it merges.
- **Agents bound to programs** (`agent register --program`) hold at most 16: an agent that hunts windows is
  registered without a program list, or re-bound.
- **Testnet only**, as everything here: wNEAR on testnet has no value.

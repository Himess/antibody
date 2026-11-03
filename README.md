# Antibody: bug bounties that pay on proof

[![ci](https://github.com/Himess/antibody/actions/workflows/ci.yml/badge.svg)](https://github.com/Himess/antibody/actions/workflows/ci.yml)

> **We don't read reports. We run them.**

**The problem.** Bug bounties judge text: a person reads every report. In January 2026, curl ended its
bug bounty after a flood of AI-generated reports
([The Register](https://www.theregister.com/2026/01/21/curl_ends_bug_bounty/),
[BleepingComputer](https://bleepingcomputer.com/news/security/curl-ending-bug-bounty-program-after-flood-of-ai-slop-reports/)).
When reports cost nothing to produce, judging text does not scale.

**Antibody judges code.** It is an auto-pay lane next to a project's existing bounty:
- **A project opens a program** before a launch or an upgrade. It commits on NEAR an anchor block, its
  rules as code (an invariants contract, a severity and payout table) and an escrow.
- **Anyone, a person or an AI agent, submits a sealed proof**: a sequence of calls. Nobody, the project
  included, can read it before its verdict, and the project reads only the proofs it pays for.
- **A deterministic verifier re-runs the proof** on the proven state at the anchor, inside a TEE, and
  reports which committed rule broke, where, and the impact. Its TDX quote is then verified on NEAR (T1).
- **near-antibody applies the committed terms**: the first proof of each (rule, entry point) is paid,
  in seconds under the instant cap (larger payouts after a challenge window); a duplicate gets the
  earlier claim's proof; bonds go to the program's escrow when a proof breaks nothing.
- **A Critical pauses the protocol on every chain** where it runs (MPC-signed permits). The project's fix
  is re-checked against every paid proof, and only a fix they all fail against is released.

**Agents can't be trusted; their proofs can.** An agent works through an account that cannot withdraw,
with a bond budget, and its proofs are judged like anyone's.

**Pay on proof. Pause on proof. Ship only what survived.**

## Check it

[JUDGES.md](JUDGES.md) lists every claim with the command that checks it, the output to expect and the
transaction behind it, generated from a run's results file, so it cannot drift from what ran.

**Status:** Antibody is deployed on NEAR testnet and **locked** (2026-11-03: near-antibody, near-guardian
and the practice vending contract hold no full-access keys). Before the lock, the whole story ran on the
same contracts as a staging run ([docs/results/testnet-staging-2026-11-03.json](docs/results/testnet-staging-2026-11-03.json)),
with Acme Lend on Sepolia and Base Sepolia and the real services: OutLayer's TEE workers, their TDX quotes
verified on NEAR (T1), the MPC network signing the permits, the Ethereum light client. From agent A's
commit's block, its Critical was verified at 18.8 s, T1-confirmed at 26.3 s and paid 0.1 wNEAR at 42.0 s;
both chains were paused by 48.1 s. JUDGES.md comes from that run, and its commands read testnet. The story
is running again on the locked deployment, and its results replace JUDGES.md when it ends.

The same story runs offline in a sandbox. On Linux or macOS:

```bash
make serve       # builds everything, runs the whole story (every outcome asserted), then keeps the sandbox up
```

The first build takes a while (the verifier, the contracts, the CLI); the story then runs for about nine
minutes and ends with `--serve: near-sandbox http://127.0.0.1:3030 …`. Then, in a second terminal in the
repository (bash or zsh):

```bash
A() { ./target/release/antibody --network sandbox --rpc http://127.0.0.1:3030 --deployments target/demo/deployments.json "$@"; }
A judge                  # view calls only: locked status, T1 confirmations, payouts, permits, audits (--json: everything)
A try --name <you>       # a new lowercase name: the practice program (a tutorial), no account, no key; paid in seconds
```

`make demo` runs the same story once and stops (`docker build -t antibody-demo . && docker run --rm
antibody-demo` elsewhere). On testnet, a build of the CLI from this repository has the deployment built in
(`cargo build --release -p antibody-cli`): `./target/release/antibody judge`, and
`./target/release/antibody try --name <you>` for the practice program, with no account and no key.

The sandbox run (`target/demo/results.json`; the recorded one is in [docs/results/](docs/results/)) asserts
every outcome of the story:

| Step | What happens | Recorded |
|---|---|---|
| Program | Acme Lend's manifest (schema v2) and escrow on NEAR; the anchor checked by the light client; activation pins the base witness and the program's sealing key | T1 |
| The one moment | Agent A, over MCP, submits a sealed proof of a Critical. From its commit's block: verified at 7.9 s, T1-confirmed at 11.5 s, **both chains paused** by 24.5 s, and A **paid** at 24.9 s (the pause is asked for before the payment) | each moment's own block or receipt, sandbox (MOCK OutLayer, DCAP and MPC) |
| A NEAR app reacts | A DAO treasury that watches the program reads near-antibody and freezes on A's verified Critical | its next transfer is refused |
| B–G | B paid; C (a prompt injection in its report) breaks nothing and forfeits its bond; D a duplicate, confirmed at the fix; E a known issue; F paid; G harms a protected user without breaking a rule: refunded, recorded, and paid once round 1 adds the rule | every state |
| The project reads it | After payment, A's proof is wrapped to the project's NEP-413-derived key; `antibody program read` opens it and re-runs it locally, and near-antibody refuses to wrap D's duplicate | the local receipt hash equals the TEE's |
| Rounds | Round 1's partial fix: every paid proof re-run at it, two still break, so the release gate stays shut and no release permit is signed; round 2's final fix: all fail, the release permits upgrade both proxies on both chains, and B's exploit reverts | the release gate |
| Practice | Two judges with no account vend practice accounts and run the tutorial proof; it is sealed against the pinned base alone | Done:Paid |
| Fork mode | The practice program again, deployed only inside the verifier: nothing of it is on chain | Done:Paid |
| Audits | Every conservation equation, per program and for the contract | ok |

## Deployment

| | Account / contract | Status |
|---|---|---|
| near-antibody | `bounty.antibody.testnet` | locked (code `C9LGvUpq…`, [CkenQk4B…](https://testnet.nearblocks.io/txns/CkenQk4BfJi9uYW72siKVqTRAAYUXxAXKErtNvQWKqDb); the staging run ran on `HMmyk9i7…`, the same code but one view) |
| near-guardian | `guardian.antibody.testnet` | locked (code `4SdG5PK9…`) |
| Practice vending | `practice.antibody.testnet` | locked (code `AdXwpxi5…`; its public vend key stays); practice program 2, its fork-mode twin program 3 |
| Payout token | `wrap.testnet` (wNEAR: testnet NEAR, with no value) | existing |
| OutLayer, light client, MPC | `outlayer.testnet`, `client-eth2.sepolia.testnet`, `v1.signer-prod.testnet` | existing |
| Acme Lend, adapters | Sepolia and Base Sepolia, at the same CREATE2 addresses | deployed (staging run) |

`antibody admin record-deployment` pins each contract's code hash, its locked status, the verifier build
and the source commit into `deployments/near-testnet.json` and `deployments/history/`. Every transaction of
the staging run is linked from [JUDGES.md](JUDGES.md). Payouts, escrows and bonds on testnet are in wNEAR,
a hundred-thousandth of the sandbox's tUSD amounts. What a mainnet program would change is in
[docs/mainnet.md](docs/mainnet.md), with a live read-only check of NEAR mainnet's Ethereum light client
(`antibody mainnet-check`: view calls only).

## The trust panel

| Actor | Trusted for | How anyone checks it |
|---|---|---|
| The light client | The anchor block's hash | The program's anchor check (`antibody judge`) against any RPC |
| The TEE (OutLayer worker) | Running the pinned verifier on the sealed proof | **T1:** the run's TDX quote is verified on NEAR; `antibody tee check --request-id <id>` reads OutLayer's attestation (testnet: the sandbox's OutLayer is a MOCK) |
| The verifier | Nothing beyond its pinned build | Native and wasm builds agree byte for byte; `antibody check` re-runs a proof locally; after disclosure every verdict can be recomputed |
| The MPC network | Signing the pause and release permits near-guardian builds | `AntibodyAdapter` recomputes each permit's EIP-712 digest on chain |
| near-antibody's owner | The defaults of new programs only | A live program's pins never change (property 12); `antibody judge` shows whether the contract is locked |
| The project | Its manifest, escrow and fixes | The manifest hash is committed; known issues are sealed before open; a fix is released only after every paid proof re-runs against it and fails |
| Relayers, keeper, indexer, API | Nothing: permissionless calls and derived data | Everything they show is re-derivable from NEAR |

Tiers: **T0** (OutLayer delivers the TEE's output), **T1** (its quote verified on NEAR). Instant payouts
need T1. Above the cap, a payout also needs an independent second run that agrees, and waits its
challenge window. Full table: [docs/trust.md](docs/trust.md).

## The properties

Twelve settlement properties, each a Quint invariant checked by 24 scenarios, 20,000 random traces and
Apalache, and each replayed against the rule core near-antibody runs (600 traces of 60 steps, no
divergence). Kani proves the payout arithmetic and every escrow movement (41 proofs). Details and the
check behind each: [docs/properties.md](docs/properties.md).

| # | Property |
|---|---|
| 1 | Escrow conservation: `funded = available + reserved + paid + fees + returned`, per program and for the contract |
| 2 | At most one payout per key per round, within each rule's budget |
| 3 | Payment goes to the earliest valid commit |
| 4 | Duplicates are never paid, except through the one-way appeal |
| 5 | A round never removes a payment or a rule |
| 6 | Every in-flight state reaches a terminal state; the exit is never blocked |
| 7 | Bonds are never lost: refunded or forfeited to the program's escrow |
| 8 | Instant payments happen only with T1, below the cap, within their window's budget |
| 9 | No known issue is added after open |
| 10 | Disclosure cannot be blocked once its condition holds |
| 11 | Permits follow the committed policy: pause tickets only for T1 holders, releases only when the release gate holds |
| 12 | The owner cannot change a live program |

Beside them: parser fuzzing (manifests, witnesses, sealed payloads, the verifier on mutated witnesses),
golden vectors for every format two components share (the commitment, addresses, EIP-712 permit
digests recomputed in Solidity), native == wasm on every fixture, and the sandbox story end to end.

## The self-attack log

We attack our own system. Each row below was found by us, fixed, and is pinned by a test: SA-1 to SA-8
while building, SA-9 on in a fresh-context review of the contracts, the crypto and the new mechanisms
before the launch deployment is locked.

| Id | What broke | Fix | Test |
|---|---|---|---|
| SA-1 | **Fingerprint steering.** Evaluations inside a frame that later reverted stayed on the timeline, so one bug could be re-keyed to any entry point and paid again | A failing frame truncates the evaluation timeline | `fixtures/regressions` (`steering`), native and wasm |
| SA-2 | **Tampered jump tables.** A witness's jump table was trusted, so a hunter could make the anchor snapshot fail and close the program | Every bytecode is re-analysed from its hashed bytes | `fixtures/regressions` (`jump-table-*`) |
| SA-3 | **Relabelled trie nodes.** A leaf and an extension can encode alike, so a witness could forge absent state under a valid root | Every trie node's kind is validated | `trie::tests::relabelled_extension_is_refused` |
| SA-4 | **A partial fix locked a program out of releases.** Re-runs were never reset between proposals, so a key one fix carried could never be re-run at the next | Re-runs count per proposal | `later_fix_releases_carried_key`, Quint `laterFixReleasesCarriedKeyTest` |
| SA-5 | **A release permit was not bound to the checked code**: it could install code the re-runs never ran | The round check binds each release's proxy and code hash | `a_round_check_binds_its_releases` |
| SA-6 | **An unjudged re-run counted as fixed.** A re-run that returned `InvalidPoC` opened the release gate | Only a T1 Violation or NoViolation is evidence | `an_unjudged_rerun_is_no_evidence` |
| SA-7 | **A round could drop the rule the round before added** | A round keeps every slot of the open round | `a_round_keeps_the_invariants_of_the_round_before` |
| SA-8 | **Re-runs used the witness's rule code**, not the round manifest's | The manifest's code is the rule code | `a_rerun_uses_the_rounds_invariants_code` |
| SA-9 | **Copies of a paid proof locked an escrow.** Once it read a paid proof, a project could commit copies from other accounts: each reserved its full amount, later holders became debts `pay` refused, and at exit the debts defaulted before the reservations went back to the project | Copies of a held proof share one reservation per key; the exit releases reservations, then funds debts, then pays | `copies_of_a_held_proof_take_one_reservation`, `exit_funds_owed_claims_before_releasing_duplicates` |
| SA-10 | **A disclosed tag stayed open.** Disclosure keys were per tag, so a later round's new proof of the same tag was readable as soon as its answer landed; a tag counted as fixed even when its release gate failed | One key per round and tag; a tag counts as fixed only once released | `a_released_fix_discloses_its_round_only`, `a_fix_that_does_not_ship_discloses_nothing` |
| SA-11 | **The distinctness top-up was an oracle.** Its requester chose the earlier finding and the patch: a no-op patch bypassed the slot budget, and a predicate patch leaked a bit of any sealed proof per run | The top-up is off | `per_invariant_budget_and_no_distinct_top_up` |
| SA-12 | **Gap claims and `unevaluable: violation` paid per copy**, above the cap without a second run | One claim per key and round, copies are duplicates; above the cap they wait for a second run | `one_gap_bug_is_promoted_once`, `an_unevaluable_violation_is_paid_once_per_round_and_waits_above_the_cap` |
| SA-13 | **An agent's budget bounded locked bonds, not losses**: through a hijacked agent, a program built to forfeit bonds drained the owner's credit | Losses count against the budget; an agent may be bound to programs; practice accounts commit to the practice program only | `agent_containment`, `an_agent_commits_only_to_its_programs` |
| SA-14 | **`exit` and `open_round` scanned every commit in one call**: a few hundred commits would stop a program's rounds and its exit for good | Both work a page per call and keep their cursor; the exit's own work is bounded by the program's keys | `the_exit_is_bounded_by_keys_and_records_close_in_pages`, the Quint replay (pages of three) |

The same review's medium and low findings:

| Id | What broke | Fix | Test |
|---|---|---|---|
| SA-15 | A hunter's own unreadable blob location held the allocation queue for free | Unreadable for the whole grace forfeits the bond | `a_blob_nobody_can_read_for_the_whole_grace_forfeits` |
| SA-16 | The exit refunded final findings not yet allocated | The exit waits until every commit is allocated or closed | `exit_settles_everything` |
| SA-17 | A round check or re-run answered without a quote blocked the program's rounds | Past the run timeout it can be replaced | `a_round_check_without_a_quote_can_be_replaced`, `a_rerun_without_a_quote_can_be_requested_again` |
| SA-18 | Relaying an earlier round's permit could downgrade a proxy | A proxy's releases only move forward | `test_releasesOnlyMoveForward` (forge) |
| SA-19 | A re-run at the project's fix code published impact and harm numbers of unpaid sealed proofs | A re-run publishes only which keys still break | `a_rerun_judges_the_same_poc_at_the_fix` |
| SA-20 | The owner could extend a live program's worker images through another registry | A program pins its registry; later images count only from it, after the delay | `the_owner_cannot_reach_a_live_program_through_another_image_registry` |
| SA-21 | A release wrap the TEE never backed could leave the project unable to read what it paid for | A wrap no quote confirmed can be replaced after the run timeout | `a_release_wrap_no_quote_confirms_can_be_requested_again` |
| SA-22 | The verdict guard froze again on duplicates and known issues, read the first round's slots only, and acted on unconfirmed anchors | First findings only, the slots as of now, light-client anchors only | `only_first_critical_t1_findings_against_a_confirmed_anchor_count` |
| SA-23 | A parent could lock a sub-account's credit by registering it as an agent | An account with credit or commits of its own never becomes an agent | `a_parent_never_makes_an_agent_of_an_account_with_value_of_its_own` |
| SA-24 | near-antibody's own balance paid for others' storage (up to 256 image entries per free sync) | Syncs only from named registries, paid by their caller; run requests charge their payer; each commit prepays its settlement's bytes | `only_a_named_registry_is_synced`, `a_commits_settlement_is_prepaid` |
| SA-25 | Our parser fuzzing found three panics on malformed witnesses in dependencies (bitvec's `set_len` assert and `todo!()`, revm-bytecode's unchecked `original_len`); in wasm a panic is a trap, not a typed verdict | Patched copies in `third_party/patches`: each is a decode error | `a_mutated_witness_is_refused_or_decodes`, `the_verifier_answers_any_mutated_program_witness` (20,000 cases each) |

The review's two remaining notes are stated as residuals in the [threat model](docs/threat-model.md) (I4b,
P13): OutLayer's quote metadata is not checked, and a page that gets a project's wallet to sign the
fixed NEP-413 message derives its key.

Building the end-to-end story found more (fixed, with tests): a failed round check stranded its proposal;
four permits did not fit one call; the gap lane's model allowed a re-run after exit. See
[docs/properties.md](docs/properties.md).

## The cost of an attack

| Attack | What it costs the attacker | What the defense gives up | Check |
|---|---|---|---|
| Slop and spam | A bond per commit, forfeited to the program's escrow, plus the verification run | Nothing: bonds are sized from measured cost | Property 7 |
| Prompt injection in a report | The bond | Nothing: the verifier never reads reports | Story step C |
| Copying or front-running a proof | Impossible before its verdict: proofs are sealed to the verifier and priority is the commit | — | AEAD binds the hunter |
| A forged or tampered witness | The bond (`InvalidPoC`) | Nothing: proofs, code and tries are validated | SA-2, SA-3, SA-25, fuzzing |
| Locking an escrow with copies of a paid proof | A bond per copy | Nothing: copies share one reservation per key | SA-9 |
| Filling near-antibody's storage | The storage, paid by the caller (each commit prepays its settlement) | Nothing | SA-24 |
| Steering a finding to a new key to be paid twice | The bond | Nothing | SA-1 |
| A dishonest TEE operator | It can delay; it cannot pay without a quote verified on NEAR | T0 payouts above the cap wait their window and a second run | Property 8 |
| Challenging honest payouts to delay them | A challenge bond, lost when T1 upholds the payout | A delay up to the resolve window | Threat P12 |
| A project closing its program to escape payouts | Needs a T1 `ProgramMisconfigured` or the window; the escrow stays until exit | — | Property 6 |
| A compromised owner | Can change the defaults of new programs only | — | Property 12 |
| A hijacked agent | Bonds within its per-commit cap and a budget that counts losses, on its own programs only; its key cannot withdraw | Its owner's capped credit | Containment tests, SA-13 |
| Draining the practice faucet | Rate-limited per hour; the public vend key's allowance caps the gas | A small, refilled float | `/v1/status` alerts |

## Agents

`antibody mcp` serves 8 tools over stdio. Every result is marked untrusted data; file access is confined
to a workspace and key files are refused. An agent account is a sub-account whose bonds come from its
owner's credit within a per-commit cap and a budget that counts bonds lost as well as locked, which may be
bound to a list of programs, whose payouts go to the owner, and whose key cannot withdraw. See
[AGENTS.md](AGENTS.md), and [docs/agent-runs.md](docs/agent-runs.md) for how runs of an off-the-shelf
agent completing the loop on the practice program are measured (the loop, not bug finding); those runs
are not done yet.

## For projects

- **Rules as code:** an `IAntibodyInvariants` contract, or Foundry-style `invariant_*` functions
  (`antibody invariants from-foundry`, [docs/foundry.md](docs/foundry.md)); the drafter
  ([docs/invariants-drafting.md](docs/invariants-drafting.md)) proposes a first set.
- **The validate gate:** `antibody invariants validate` checks a manifest as creation and activation will.
- **Rule strength:** `antibody invariants strength` mutates the code and counts the mutants the rules
  catch under the fuzzer: Acme's catch 8 of 28 at the final fix
  ([docs/rule-strength.md](docs/rule-strength.md)). It measures the rules, not the code's security.
- **No staging deployment:** fork mode deploys the program's contracts inside the verifier
  ([docs/fork-mode.md](docs/fork-mode.md)).
- **NEAR apps can act on verdicts** ([docs/verdict-guard.md](docs/verdict-guard.md)).

## Limitations

- **Only rule breaks are paid automatically.** Anything else is at the project's discretion (`top_up`).
- **Duplicates are judged by (rule, entry point).** Two bugs behind one entry point count as one until
  the first is fixed (rounds).
- **One block.** A proof runs in the anchor block's environment: no time travel, no multi-block attacks.
- **Ethereum L1 for verification** (Sepolia). Other chains are pause and release targets.
- **The EVM rules are the manifest's, not the chain's.** Sepolia runs the Glamsterdam upgrade (its headers
  carry `blockAccessListHash` and `slotNumber`, and the anchor check hashes them). A proof runs under the
  rules its manifest commits (`evm_spec: osaka`), so gas is counted by osaka's costs: state creation costs
  five to seven times more on Sepolia itself.
- **Public Sepolia RPCs prove about 128 blocks back**, so witnesses are built within minutes of an anchor,
  and the light client runs about 80 blocks behind: a program is created some 15 minutes after its anchor.
- **Claims are paid in commit order.** A commit that is never revealed holds the later ones until its
  reveal window ends and the keeper expires it (on testnet, a practice try's payout waited ten minutes
  this way).
- **The testnet light client has one trusted relayer.** Anchors it cannot confirm are labelled
  project-attested.
- **TEE trust is tiered, not removed.** T1 needs DCAP collateral; above the cap a second run and a
  window are required.
- **Old anchors.** A proof that reads beyond the pinned base needs an RPC that keeps the anchor's state.
- **Rule strength is the rules' measure,** and a survivor is not necessarily a bug.
- **Testnet only.** The payout token, wNEAR on testnet, has no value.

## Provenance

- **Built during the hackathon:** all code was written after the October 19, 2026 announcement, and
  the commits reflect when it was written. Tags: `hackathon-start`, milestone tags, and
  `hackathon-submission` at submission.
- **AI-assisted:** written with Claude Code (Claude Opus 5.5). The design, the rules and every decision
  on scope were reviewed by the team.
- **Libraries** (direct dependencies, by license):
  - MIT or Apache-2.0: alloy, reth 2.2, rsp, serde, serde_json, clap, reqwest, borsh, sha2, hkdf,
    chacha20poly1305, k256, getrandom, near-sdk 5.9, proptest, futures-util;
  - MIT: revm 38, axum, tokio, bincode;
  - BSD-3-Clause: ed25519-dalek, x25519-dalek;
  - Apache-2.0 with LLVM exception: wasmtime 28.
  - Tools: Foundry, Quint and Apalache, Kani, near-sandbox.
- **Patches:** `third_party/patches/` holds two small build fixes (for `kzg-rs` and `near-vm-runner`).

## Tracks

**06 AI Security & Defense** (primary): proofs, not reports, judged by a deterministic verifier in a TEE,
with pause and release on every chain. **07 Agent Economy**: agents earn on proof, through accounts that
contain them. **03 Privacy & Private AI** once caller-derived keys, scheduled disclosure and
private-inference drafting ship.

## Business model

The requester pays verification at cost. Revenue is a fee on payouts and a fee per release permit,
both pinned per program, and later a paid tier for the drafter, rule strength and analytics. Forfeited
bonds are never revenue: they go to the program's escrow.

## Roadmap

1. **Antibody for pull requests** (a GitHub Action, then a GitHub App), on repositories whose owner
   installs it:
   - *Immune memory:* every paid proof of a program re-runs against each pull request's build (sealed
     ones inside the TEE, where the result is all anyone sees), and the check goes red if one still
     breaks a rule.
   - *A bounty window per pull request:* a fork-mode round for the pull request's build with a small
     escrow, announced to agents over MCP or a webhook. When it ends, the check reads "held N hours, X at
     stake, 0 valid findings", or blocks the merge on a valid Critical.

   Sealed proofs never appear in public logs.
2. NEAR contracts as a second target VM: proofs against NEAR wasm contracts from committed, proven state.
3. T2: a zk refutation path, with proofs of disclosed verdicts verified on NEAR; later zk by default above a cap.
4. Multi-block proofs with a bounded time warp: interest, TWAP and time-dependent bugs.
5. The next Ethereum execution rules (an EVM upgrade and fork-switch execution).
6. L2 state verification through MPC foreign-chain verification or L2 light clients; more chains for pause and release.
7. Several independent TEE operators by default; a strict "no quote, no pay" mode once attestation is reliable.
8. One sub-account per program: isolated escrow, keys and signing path.
9. Payouts in any asset through NEAR Intents (mainnet).
10. Agent capital: backer vaults fund bonds for agents with verified records; bond discounts from proof-based reputation.
11. Security jobs on NEAR's agent market whose acceptance test is an Antibody verdict.
12. A public verdict registry as a NEAR primitive for insurance, lending markets and DAOs.
13. Continuous programs for live protocols in mainnet mode, with honest latency limits.
14. Private and invite-only programs; optional checks before large withdrawals; safe-harbor templates.
15. Post-quantum readiness: ML-DSA accounts and hybrid ML-KEM sealing for long embargoes.
16. The mainnet path: external audits, Antibody's own adapters under an Antibody program, coverage partners, the fee model in production.
17. A paid tier for the drafter, rule strength and analytics.

## Repository layout

| Path | Contents |
|---|---|
| `crates/verifier` | The deterministic verifier: program mode, rounds, fork mode, receipts, TEE modes, sealed submissions |
| `crates/witness` | Witness builder: traces a proof through the verifier's own path, base witnesses, submissions from the base |
| `crates/rules` | The settlement rule core near-antibody runs; Kani proofs |
| `crates/cli` | `antibody`: CLI, MCP server, practice, judge view, rule strength, Foundry compatibility, drafter |
| `crates/services` | Indexer, API, keeper, relayers, notifier, monitoring, sandbox mocks, the demo |
| `contracts/near-antibody`, `near-guardian`, `practice-vending`, `practice-account`, `verdict-guard` | The NEAR contracts |
| `contracts/evm` | Acme Lend (deliberately vulnerable), its rules, proofs and fixes; the practice vaults; the adapter |
| `formal/` | The Quint model of the settlement rules |
| `fixtures/`, `golden/` | Recorded witnesses and verdicts; golden vectors |

## Safety

Acme Lend and the practice vaults are **deliberately vulnerable test fixtures**, with textbook bugs
labeled in their code. Their proofs exist only for those bugs, and everything runs on testnets or local
chains. Antibody verifies; it does not hunt. See [SECURITY.md](SECURITY.md). Keys live only in the
git-ignored `.secrets/` and `.env`, and gitleaks runs in the pre-commit hook and in CI.

## License

MIT (see [LICENSE](LICENSE)). Third-party code is used under its own licenses.

# Settlement properties

Twelve properties of Antibody's settlement rules ([spec.md](spec.md)), and the checks behind each one.

## How they are checked

1. **The model.** [`formal/antibody.qnt`](../formal/antibody.qnt) models one program: commits, reveals,
   T0 and T1 verdicts from an adversarial operator, challenges, the allocation cursor, payments, debts,
   rounds with immune memory and appeals, known issues, the gap lane, permits, disclosure and the exit.
   Each property below is a Quint invariant, checked three ways:
   - by 21 deterministic scenarios (`quint test`);
   - on every state of 20,000 random traces (`quint run --invariant=all_properties`);
   - by bounded model checking with Apalache (`quint verify`).
2. **The code against the model.** `crates/rules` (the settlement engine near-antibody runs) replays
   model traces step by step (`crates/rules/tests/quint_replay.rs`). For each action the model takes,
   the engine must accept it. After every step, both sides must agree on:
   - money and bonds;
   - every commit's state;
   - key holders, tickets, permits and disclosures.

   `scripts/formal/replay.sh` runs it: 600 traces of 60 steps (36,600 steps; about 5% of the traces go
   through both round openings), no divergence.
3. **The code on its own.** Scenario tests mirror the model's scenarios (`crates/rules/tests/scenarios.rs`).
   After each scenario, `audit()` checks every conservation equation.
4. **The arithmetic.** Kani proves the payout formulas and every movement of a program's escrow
   (`crates/rules/src/proofs.rs`, 41 proofs, `cargo kani -p antibody-rules`). The engine calls exactly
   these functions (`crates/rules/src/arith.rs` and the `Ledger` methods), so the proofs cover the code
   that runs. The inputs are everything the program bounds allow (spec §3.3). No step overflows or
   panics, and:
   - the protocol fee is never more than is available, and a rate of a bounded amount is exact;
   - the impact bonus stays within its cap for every token precision from 0 to 24 decimals, and the
     nominal payout cannot overflow;
   - a slot never pays past its budget; an instant window never pays past its budget or above the cap,
     and a new window starts empty;
   - a claim's reservation and its debt add up to the claim, nothing is reserved while earlier debts
     wait, and a debt stays unfunded only when nothing is available;
   - every ledger movement (deposit, forfeit, award, reserve, release, payment, fee, return) keeps
     `funded = available + reserved + paid + fees + returned`.

   Three facts need 128-bit multiplication and division that the solver does not finish in 20 minutes:
   a rate is never above its base, the protocol fee is at most a fifth of the payout, and a larger
   impact never earns a smaller bonus. Property tests check them instead (`crates/rules/tests/arith.rs`:
   10,000 random cases each over the whole range, plus every boundary value).

## The properties

| # | Property | Model invariant | Scenarios (model / code); Kani proofs |
|---|---|---|---|
| 1 | **Escrow conservation.** Per program, `funded = available + reserved + paid + fees + returned`, and `reserved` is the sum of the commits' reservations. For the contract, `tokens held = escrow + credits + locked bonds + locked challenge bonds` | `p1_escrow`, `p1_tokens` | every scenario; `audit()` after every replayed step; Kani: every ledger movement |
| 2 | **At most one payout per key per round, within each invariant's budget** | `p2_onePerKey`, `p2_budget` | `appealPromotesDistinctBugTest` / `per_invariant_budget_and_distinct_top_up`; Kani: `a_slot_never_pays_past_its_budget` |
| 3 | **Payment goes to the earliest valid commit once the reveal windows close**: a duplicate always duplicates an earlier holder, and no holder is later than a duplicate of its key | `p3_priority` | `challengeTimeoutKeepsPriorityTest` / `challenge_timeout_keeps_priority` |
| 4 | **Duplicates are never paid, except through the one-way appeal**: a promotion needs a released key and a re-run that still breaks it | `p4_dups` | `appealPromotesDistinctBugTest`, `confirmedDuplicateReleasesReservationTest` |
| 5 | **A round never removes a payment or an invariant** | `p5_monotone` | `gapLanePromotedTest` / `gap_lane_promoted` |
| 6 | **Every in-flight state reaches a terminal state**: the exit is never blocked after the deadline and its grace, and after it nothing is in flight or locked | `p6_exit` | `exitSettlesEverythingTest` / `exit_settles_everything` |
| 7 | **Bonds are never lost**: each one is refunded or forfeited to the program's escrow | `p7_bonds` | `t0WaitsWindowTest`, `disagreeingSecondRunVoidsTest` / `infra_error_keeps_priority_then_refunds` |
| 8 | **Instant payments happen only with T1, below the cap, and within the budget of their window** | `p8_instant` | `instantPayoutTest`, `instantBudgetTest`, `aboveCapNeedsSecondRunTest` / `instant_payout`, `instant_budget_exhausted_waits_window`, `above_cap_needs_second_run`; Kani: `the_instant_budget_holds_in_every_window` |
| 9 | **No known issue is added after open, and known issues stay within their cap** | `p9_known` | `knownIssueTest` / `known_issue_needs_t1_cap_and_setup` |
| 10 | **Disclosure cannot be blocked once its condition holds**: the condition depends on nobody's consent, and once true it stays true | `p10_disclosure` | `disclosure_needs_condition_and_t1` |
| 11 | **Permits follow the committed policy**: pause tickets only for T1 holders of policy keys, at most one per round; release permits only when the release gate holds | `p11_permits` | `releaseGateBlocksUnfixedTest`, `laterFixReleasesCarriedKeyTest`, `regressionBlocksReleaseTest`, `canariesBlockReleaseTest`, `unexaminedDuplicateBlocksReleaseTest` / `release_gate_blocks_unfixed`, `later_fix_releases_carried_key`, `regression_blocks_release`, `unexamined_duplicate_blocks_release` |
| 12 | **The owner cannot change a live program** | `p12_pins` | `ownerCannotChangeLiveProgramTest` |

## What the model found

These counterexamples changed the design (spec §20, D7):
- **A release permit signed while a distinct Critical still broke the fix.** The first release gate
  checked only the holders' re-runs. A duplicate that a fix did not stop, a second bug behind the same
  key, still left the gate open. The gate now requires every Critical or High PoC of the round,
  duplicates included, to be re-run and fixed.
- **A release over an unexamined duplicate.** The gate allowed a release when nobody had re-run a
  Critical duplicate. It now requires that re-run too.

The replay found two crashes in the engine, both fixed: a round opening over an expired commit, and a
second run answering after its verdict was voided.

Reading the engine for a second round found what a one-opening model could not reach: a re-run was
never reset, so a key a partial fix carried could not be re-run at the next fix, and no later fix of
that program could ever be released. The same held for promoted holders. Re-runs now count for their
own proposal only. Every paid PoC and every holder of a key, from any round, is re-run at each fix
(immune memory), so a fix that brings an old bug back is not released either. The model now opens two
rounds.

## What building the rounds end to end found

Running the gap lane through two rounds on the sandbox (`make demo`) found four more, all fixed with tests:
- **An unjudged re-run counted as fixed.** Every re-run that was not a Violation counted as fixed, so an
  `InvalidPoC` re-run released its key and opened the release gate. Only a T1 Violation or NoViolation is
  evidence now (`an_unjudged_rerun_is_no_evidence`).
- **Re-runs used a witness's own invariants code**, so a round that brings new invariants judged nothing.
  Runs now use the manifest's code (`a_rerun_uses_the_rounds_invariants_code`).
- **A round could drop the invariant the round before added**, since rounds were checked against round 0
  only. The TEE now checks each proposal against the open round (`a_round_keeps_the_invariants_of_the_round_before`).
- **A round check that failed without a verdict blocked the program's rounds for good** (one proposal at a
  time). It now drops the proposal (`a_failed_round_check_lets_the_project_propose_again`).

## Inputs, formats and builds

Beyond the settlement rules, the checks that keep the verifier, the contracts and the CLI agreeing, and
the verifier answering whatever it is given. Every one runs in CI (`.github/workflows/ci.yml`).

| Property | Check | CI job |
|---|---|---|
| **Every parser of untrusted bytes, and the verifier's program path, ends in a typed result**: manifests, witness envelopes, sealed payloads and mutated program witnesses never panic (in wasm a panic is a trap) | `crates/verifier/tests/fuzz_parsers.rs`: property tests seeded with the recorded fixtures, mutated by bit flips, overwrites, insertions, deletions and truncation (512 cases per parser, 48 whole verifier runs); a parsed mutated manifest re-serializes canonically | verifier |
| **Formats two components share do not drift**: the commitment a reveal matches, fresh sender, reserved and fork-mode deployer addresses, the public vend key, `vend`'s Borsh arguments | `golden/vectors.json`, written by `crates/cli/tests/golden.rs` and checked again by near-antibody (`golden_commitments_match_the_cli`) and the vending contract (`golden_vend_args_match_the_cli`) | verifier, NEAR contracts |
| **near-guardian signs exactly what AntibodyAdapter verifies**: the EIP-712 pause and release digests | `golden/permits.json`, written by near-guardian's tests and recomputed in Solidity by `GoldenPermits.t.sol` | NEAR contracts, fixture contracts |
| **The native and wasm verifiers agree** byte for byte on every recorded fixture | `scripts/demo/native_vs_wasm.py` (facts and full output) | verifier |
| **Recorded verdicts hold**: every fixture submission (rounds 0 to 2) and the self-attack regressions | `scripts/demo/check_fixtures.py`, `scripts/demo/check_regressions.py` | verifier |
| **A round only changes target code and adds slots** (its rule strength aside), and a fork-mode deployment stays the program's | `round_rules` unit tests and `tee_fixture.rs` | verifier |
| **The whole flow**: every expected outcome of the sandbox story (verdicts, T1, payouts, pauses, rounds, releases, the practice program, fork mode, the verdict guard, audits) | `antibody-services demo` | sandbox end-to-end |

## Limits

- The model covers one program with four commits, three keys (two sharing an invariant) and three
  rounds (two openings).
  It has one key per verdict; multi-key verdicts are covered by the engine's own tests.
- Apalache checks bounded depths; the random simulations go deeper but are not exhaustive.
- Liveness is checked as safety: the exit's guard is time alone, and after the exit nothing is in flight.
- Kani proves the formulas and single ledger movements. Whole engine methods, and the contract-wide
  equation, are covered by the replay and `audit()`, not by Kani.

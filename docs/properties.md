# Settlement properties

Twelve properties of Antibody's settlement rules ([spec.md](spec.md)), and the checks behind each one.

## How they are checked

1. **The model.** [`formal/antibody.qnt`](../formal/antibody.qnt) models one program: commits, reveals,
   T0 and T1 verdicts from an adversarial operator, challenges, the allocation cursor, payments, debts,
   rounds with immune memory and appeals, known issues, the gap lane, permits, disclosure and the exit.
   Each property below is a Quint invariant, checked three ways:
   - by 19 deterministic scenarios (`quint test`);
   - on every state of 20,000 random traces (`quint run --invariant=all_properties`);
   - by bounded model checking with Apalache (`quint verify`).
2. **The code against the model.** `crates/rules` (the settlement engine near-antibody runs) replays
   model traces step by step (`crates/rules/tests/quint_replay.rs`). For each action the model takes,
   the engine must accept it. After every step, both sides must agree on:
   - money and bonds;
   - every commit's state;
   - key holders, tickets, permits and disclosures.

   `scripts/formal/replay.sh` runs it: 600 traces, 27,600 steps, no divergence.
3. **The code on its own.** Scenario tests mirror the model's scenarios (`crates/rules/tests/scenarios.rs`).
   After each scenario, `audit()` checks every conservation equation.

## The properties

| # | Property | Model invariant | Scenarios (model / code) |
|---|---|---|---|
| 1 | **Escrow conservation.** Per program, `funded = available + reserved + paid + fees + returned`, and `reserved` is the sum of the commits' reservations. For the contract, `tokens held = escrow + credits + locked bonds + locked challenge bonds` | `p1_escrow`, `p1_tokens` | every scenario; `audit()` after every replayed step |
| 2 | **At most one payout per key per round, within each invariant's budget** | `p2_onePerKey`, `p2_budget` | `appealPromotesDistinctBugTest` / `per_invariant_budget_and_distinct_top_up` |
| 3 | **Payment goes to the earliest valid commit once the reveal windows close**: a duplicate always duplicates an earlier holder, and no holder is later than a duplicate of its key | `p3_priority` | `challengeTimeoutKeepsPriorityTest` / `challenge_timeout_keeps_priority` |
| 4 | **Duplicates are never paid, except through the one-way appeal**: a promotion needs a released key and a re-run that still breaks it | `p4_dups` | `appealPromotesDistinctBugTest`, `confirmedDuplicateReleasesReservationTest` |
| 5 | **A round never removes a payment or an invariant** | `p5_monotone` | `gapLanePromotedTest` / `gap_lane_promoted` |
| 6 | **Every in-flight state reaches a terminal state**: the exit is never blocked after the deadline and its grace, and after it nothing is in flight or locked | `p6_exit` | `exitSettlesEverythingTest` / `exit_settles_everything` |
| 7 | **Bonds are never lost**: each one is refunded or forfeited to the program's escrow | `p7_bonds` | `t0WaitsWindowTest`, `disagreeingSecondRunVoidsTest` / `infra_error_keeps_priority_then_refunds` |
| 8 | **Instant payments happen only with T1, below the cap, and within the budget of their window** | `p8_instant` | `instantPayoutTest`, `instantBudgetTest`, `aboveCapNeedsSecondRunTest` / `instant_payout`, `instant_budget_exhausted_waits_window`, `above_cap_needs_second_run` |
| 9 | **No known issue is added after open, and known issues stay within their cap** | `p9_known` | `knownIssueTest` / `known_issue_needs_t1_cap_and_setup` |
| 10 | **Disclosure cannot be blocked once its condition holds**: the condition depends on nobody's consent, and once true it stays true | `p10_disclosure` | `disclosure_needs_condition_and_t1` |
| 11 | **Permits follow the committed policy**: pause tickets only for T1 holders of policy keys, at most one per round; release permits only when the release gate holds | `p11_permits` | `releaseGateBlocksUnfixedTest`, `canariesBlockReleaseTest`, `unexaminedDuplicateBlocksReleaseTest` / `release_gate_blocks_unfixed`, `unexamined_duplicate_blocks_release` |
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

## Limits

- The model covers one program with four commits, three keys (two sharing an invariant) and two rounds.
  It has one key per verdict; multi-key verdicts are covered by the engine's own tests.
- Apalache checks bounded depths; the random simulations go deeper but are not exhaustive.
- Liveness is checked as safety: the exit's guard is time alone, and after the exit nothing is in flight.

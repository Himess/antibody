# Rounds: paying for a second bug in the same function

A finding's key is `(invariant, function where it broke)`. Two different bugs that break the same
invariant inside the same function therefore share a key, and the second one is a duplicate. That is
the price of judging duplicates without reading anyone's text. Rounds remove that price once the
project fixes the first bug.

## The idea

1. The project ships a fix as a **round manifest**: the program's manifest plus code overrides. Each
   override is the fixed runtime code for a target, at the same address and the same anchor.
2. The TEE **re-runs** every claimed finding against the fix. The PoC is the original one (same
   `poc_hash`); only the code under test changes.
3. When the round **opens**, the contract releases each key whose PoC no longer breaks it. A released
   key can be claimed again. Every other key carries over to its original finder.

So in round 1, the bug that was fixed is gone. A second bug in the same function, if one exists, now
breaks the invariant on a free key and is paid like any new finding.

## Why code overrides at the same anchor

- **No redeploy and no new anchor.** The fix is just bytes in the manifest, committed by hash. The
  state is the same proven anchor state, so re-runs compare like with like.
- **The storage layout must not change.** Overrides replace code only; storage stays the anchor's.
  The fixture's fix (`contracts/evm/src/acme/AcmeLendFixed.sol`) inherits from `AcmeLend` and keeps its
  reference price in an immutable.
- **Base Sepolia is unaffected.** It stays pause-only. A real deployment of the fix is the
  project's business; the program keeps judging against the committed manifest.

## What cannot change in a round

`propose_round(program_id, base_manifest, manifest, manifest_url)` takes the program's own manifest,
which must hash to the committed one, and the round manifest. The contract checks that they are
identical, except for:

- `round`, which must be the next round number;
- `overrides`, which must be non-empty, with each `code_hash = keccak256(runtime_code)` and each
  address a target;
- the overridden targets' `code_hash`, which must equal the override's hash.

Rules, payouts, bond, windows, deadline, invariants, anchor, capabilities and impact config cannot
change. A project cannot use a round to make its program cheaper or its invariants weaker.

## Flow and states

```
round r open ──propose_round──▶ round r+1 Proposed ──open_round──▶ round r+1 Open
                                   │  new commits verify against r+1 (and wait to settle)
                                   │  request_rerun(commit) for each claimed finding → on_rerun
```

- **New commits.** From the proposal on, they belong to round r+1 and verify against its manifest. They
  finalize only after the round opens, when the carried keys are known.
- **Re-runs.** `request_rerun(commit_id, sealed)` is open to the project or the finder, only for a
  commit that holds a key in the open round, up to 3 attempts per round.
  - `sealed` is a fresh witness of the same bundle, built at the round manifest and sealed to the
    verifier.
  - The project builds it from its key wrap (`antibody program round-rerun --commit N --project-key ...`).
  - The finder builds it from their local record.
  - After disclosure, anyone can.
- **The TEE** (`{"mode":"rerun"}`, see docs/crypto-spec.md §8.1):
  - checks the blob like a submission;
  - requires `poc_hash(bundle)` to equal the original verdict's;
  - runs the PoC against the round manifest;
  - returns facts without a project wrap.
- **`on_rerun`** records the keys the PoC still breaks. A re-run that fails, executes another PoC or
  comes back `InvalidPoC` is recorded as `Failed` and is no evidence; it can be retried.
- **`open_round`** (the project) needs every earlier-round commit settled and no re-run in flight. For
  each key of the previous round:
  - it is **released** if its holder's re-run is `Done` and no longer breaks it;
  - otherwise it **carries over** to the same holder.
  Carried claims take priority over claims made by the new round's own commits, because they were
  found first.

## Safety properties

| Property | Why it holds |
|---|---|
| A key is released only if its PoC really stopped breaking it | Release needs a `Done` re-run of the same `poc_hash`, at the committed round manifest, on the proven anchor state. The project cannot fabricate one, only choose the fix |
| A lazy or hostile project changes nothing for finders | Keys that are not re-run carry over. Payouts already decided in a round are never revisited |
| Rules stay immutable | `check_round` allows only code overrides |
| Re-runs stay confidential until disclosure | Re-run blobs are sealed to the verifier like submissions. The disclosure run also publishes their content keys (entries with `round`) |
| No unbounded loops | One proposed round at a time. Re-runs are limited to claim holders and 3 attempts. `open_round` walks the program's claims once |

## Evidence

- `fixtures/acme-local/round1/` holds the fix (`AcmeLendFixed`'s runtime code), the round manifest,
  and re-run witnesses for every claimed finding, with expected verdicts.
  - A and D (the spot-price bug) are `NoViolation` at the fix.
  - B, F and the known issue keep their exact keys.
  - CI checks native == wasm (`scripts/demo/check_fixtures.py fixtures/acme-local/round1`).
- `crates/verifier/tests/tee_fixture.rs` (`rerun_mode_runs_the_same_poc_at_the_fix`) covers the TEE
  re-run mode and its rejections: another PoC, the wrong round's manifest, round 0.
- `contracts/near-antibody/src/tests.rs` covers the contract rules:
  - `rounds_release_fixed_keys_and_carry_the_rest`: G's new finding on A's released key is paid, H's on
    F's carried key is a duplicate, K's on the known issue's key is known;
  - `round_manifests_may_only_change_target_code`;
  - `rounds_wait_for_earlier_commits_and_need_a_claim_to_rerun`.
- `contracts/evm/test/acme/AcmeFixture.t.sol` (`test_round1_fix_*`) runs the fix in Foundry with `vm.etch`.
- The sandbox demo (`make demo`, CI job `e2e`) proposes round 1, re-runs KNOWN, A, B and F through the
  MOCK OutLayer path and opens the round: 3 keys carried, 1 released.

## Limits

- **One anchor.** A fix that needs a migration (new storage) cannot be expressed as a code override.
  Such a fix needs a new program.
- **Projects can still pay more.** A second bug inside the same function before the first one is fixed
  is still a duplicate; `top_up` remains the way to reward it.

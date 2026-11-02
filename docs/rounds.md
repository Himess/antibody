# Rounds: fixes, immune memory and the release gate

A finding's key is `(invariant, function where it broke)`. Two different bugs that break the same
invariant inside the same function share a key, so the second one is a duplicate. Rounds remove that
price once the project fixes the first bug. They are also how a fix ships: a release permit is signed
only for code that every paid exploit was re-run against and no longer breaks (spec §8–§9).

## A round manifest

The project ships a fix as a **round manifest**: the program's round-0 manifest plus code overrides.
Each override is the fixed runtime code of a target, at the same address and the same anchor.

```
antibody manifest-round --base manifest.json --round 1 --override <implementation>=<fixed runtime hex> --out round1.json
```

- **No redeploy and no new anchor.** The fix is bytes in the manifest, committed by hash. Re-runs use
  the same proven anchor state, so they compare like with like.
- **The storage layout must not change.** Overrides replace code only. The fixture's fixes inherit from
  the contracts they fix (`AcmeLendFixed`, `AcmeLendFinal`, `AcmeVaultFixed`).
- **Proxies.** A target behind an ERC-1967 proxy is fixed by overriding its implementation. That is also
  what a release replaces on chain.

A round may replace target code and add invariants. It may never remove or weaken an invariant, add known
issues, change payouts or windows, or touch anything else.

## Proposing

```
antibody round propose --program N --manifest round1.json --release <proxy>:<implementation>:<code hash>
```

The manifest goes up with FastFS. The releases name each proxy to upgrade, the implementation to install
(deployed beforehand, the same address on every chain) and the code hash that implementation must have.
near-antibody sends both to the TEE as a `check_round` run, which verifies:
- only target code changed, the new code hashes match the overrides, and the old slots are identical;
- `check()` holds at the anchor with the new code, and the canaries still succeed;
- every release names a target that is an ERC-1967 proxy at the anchor, and the code hash of the
  override that replaces that proxy's implementation. Base witnesses prove every target's
  implementation slot for this.

The command also builds the round's **base witness**: the anchor state both the round-0 checks and the
round's own checks read (an added invariant, the fixed code), proven against the anchor. It needs an RPC
that serves `eth_getProof` at the anchor block, which is an archive endpoint once the anchor is older than
the provider's proof window. The proposal counts once the check is T1-confirmed.

## Immune memory: re-runs

Within `rerun_window` of the confirmed check, anyone may ask for, and pay for, a re-run at the fix
(`antibody round rerun --commit N`) of:
- every paid PoC, from any round, and every commit that ever held a key;
- the round's duplicates (their appeal) and gap claims.

The TEE re-runs the original sealed PoC (the same `poc_hash`) under the round manifest. A re-run counts
for its own proposal only, so the next fix is checked against every paid exploit again, and a fix that
brings an old bug back is caught. `antibody keeper` requests every eligible re-run while a proposal is open.

## Opening

`antibody round open --program N` is permissionless once the round has run for `round_min`, the re-run
window has passed with nothing in flight, and every commit of the closing round is settled. For each key
held in the round:
- **released** if its holder's re-run shows it fixed (or nobody asked for that re-run);
- **carried** to the same holder if it still breaks, or if its re-run has no verdict (only a T1
  Violation or NoViolation counts; a re-run the verifier could not judge shows nothing fixed).

A duplicate of a released key whose own re-run still breaks it is **promoted** (the one-way appeal). A fix
can only add payments.

The **release gate** holds when:
1. every paid PoC, from any round, was re-run at this fix and shows no violation;
2. every Critical or High PoC of the round, duplicates included, was re-run and is fixed;
3. no gap claim still breaks;
4. the canaries pass;
5. every holder of a Critical or High key, from any round, was re-run at this fix and shows no violation.

When it holds, near-antibody asks near-guardian for a release permit per proxy and chain: EIP-712,
signed by Chain Signatures under the project's path. The program's adapter upgrades a proxy only with a
valid permit, and only to an implementation whose on-chain code hash is the permit's.

## The demo

`make demo` runs both rounds after the payouts:
- **Round 1, a partial fix:** `AcmeLendFixed` (planted bug #1). The keeper re-runs A, B and F (paid) and
  the duplicate D. A and D no longer break anything; B and F still do. B's and F's keys are carried and
  the gate stays shut: no release.
- **Round 2, the final fix:** `AcmeLendFinal` (bugs #1 and #3) and `AcmeVaultFixed` (bug #2). A, B and F
  are re-run again and all are fixed, so the gate holds. The guardian signs four permits (two proxies, two
  chains), the relayer submits them, both proxies run the final fix on both chains, and B's fee-sweep
  exploit reverts against the released code.

The declared known issue (#4) is not part of the fix and still breaks its invariant. It is known, so it
blocks no release.

## Evidence

- `fixtures/acme-local/round1/` and `round2/`: each round's fixed runtime code, round manifest and
  re-run witnesses for every finding that held a key, with expected verdicts
  (`scripts/demo/acme_round.py --round N`, checked by `scripts/demo/check_fixtures.py`).
- `crates/verifier/tests/tee_fixture.rs`, `a_round_check_binds_its_releases`: a release to other code,
  to a target that is not the override's proxy, or to a non-target is refused.
- `crates/rules/tests/scenarios.rs`: `release_gate_blocks_unfixed`, `later_fix_releases_carried_key`,
  `regression_blocks_release`, `canaries_block_release`, `unexamined_duplicate_blocks_release`,
  `gap_lane_promoted`.
- `formal/antibody.qnt` (two round openings): `laterFixReleasesCarriedKeyTest`,
  `regressionBlocksReleaseTest`, and the gate as invariant `p11_permits` (docs/properties.md).
- `contracts/evm/test/acme/AcmeFixture.t.sol`: `test_round1_*` and `test_final_*`, including a release
  permit per proxy through the adapter.

## Limits

- **One anchor.** A fix that needs new storage cannot be expressed as a code override; it needs a new
  program.
- **Code, not intent.** The gate shows that every paid exploit stops working, the canaries still pass
  and the code shipped is the code checked. It does not show that the fix has no other bug: that is what
  the next round's hunters are paid for.

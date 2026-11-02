# R3 experiment E1: fingerprint steering (MEC-1)

**Setup.** Fixture anchor: block 22 of the local anvil (`fixtures/local/anvil-state.json`). Bundle
`STEER.bundle.json` deploys `contracts/evm/test/review/PocSteer.sol`, which:
1. runs the declared known issue, `AcmeVault.withdraw(1)` × 5;
2. restores the share price with `donateYield` inside a helper frame that reverts;
3. calls AcmeVault with selector `0xdeadbeef`, which reverts because no such function exists.

**Before (main, `3ebdeb6`).** Violation, mask 0x4, attributed to `share_price -> AcmeVault 0xdeadbeef`, tag
`6ac02de0…`. The known issue's tag is `05dbd72c…`, so the same bug becomes a new paid key, and any selector
gives another one.

**Fix** (`crates/verifier/src/program.rs`):
- every frame records the length of the timeline at its start;
- a frame that fails (revert or halt) truncates the timeline back to that mark;
- reverted entry calls are not evaluated;
- the `max_entry_calls` cap counts evaluations performed, in a separate counter.

**After.** `share_price -> AcmeVault 0x2e1a7d4d` (`withdraw(uint256)`), tag `05dbd72c…` = the known issue,
so the verdict is Known. All 7 recorded fixtures (KNOWN, A–F) keep the same verdict kinds, fp_tags,
receipt hashes and evaluation counts: the fix is backward compatible on honest PoCs.

**Not covered.** Coarseness, where the same root cause reached through a different entry function gives a
different key, is a separate design question: the duplicate rule.

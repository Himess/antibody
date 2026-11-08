# Runbook: OutLayer rotates its worker images

OutLayer adds worker images (TDX measurements) to its approved list from time to time. On 2026-11-06 it
added six, in two batches. This runbook covers what that does to T1, what the locked contracts allow, and the steps.

## What happens

- **near-antibody's rule** (`image_allowed`). A run's image counts for T1 if it is in the program's
  snapshot (the images near-antibody listed when the program was created). Otherwise it must come from
  the program's pinned registry and have been listed at least the image delay ago (2 days on testnet).
- **The keeper** lists new images within 10 minutes (`sync_images`, permissionless).
- **`/v1/status`** shows each stage:
  - `images-unlisted` (critical) until the images are listed;
  - `images-pending` (warn), with the time each one counts from;
  - `t1-waiting-<run>` for a run that waits.
- **The effect.** On programs 5–7, a run on a new image stays T0 for those 2 days.
  - The T1 relayer sends nothing for it, and retries when the image counts.
  - A T0 payout waits the program's challenge window. That is 60 s on the practice program, so a judge's
    `antibody try` is still paid, once the window has passed instead of in seconds. `try` (and the MCP
    `practice_try`) waits for T1 and the window together and settles at the first: here, as soon as a
    final NEAR block is past the answer plus the window (NEAR's clock, not the judge's). Its summary
    says so: `T0, paid after the 60 s challenge window` (`payout_path` in `--json`).
- **T2 is not affected:** near-t2 checks proofs, not images.

## The fallback the locked contracts allow

A program created **after** the new images are listed has them in its snapshot. They count for it at
once, because the delay applies only to images listed after a program's creation.
- The practice vending contract's code is locked, but its owner (`antibody.testnet`) may point it at
  another practice program. The code allows that: `set_terms`.
- `antibody try` follows the vending contract's program (`get_config`), so judges need no new client.
- The T2 prover service follows it too.

So during judging, a rotation is handled by a fresh practice program and a retarget. Nothing pinned
changes: near-antibody, near-guardian, the vending code and the verifier wasm stay as they are.

## Steps (on the operations machine, in the repository)

1. **Check that the new images are listed.** In `/v1/status`, `images-unlisted` must be gone. If it is
   not, the keeper's next pass lists them. To list them by hand (permissionless; the caller pays their
   storage):
   ```
   antibody --account antibody.testnet admin call --contract bounty.antibody.testnet --method sync_images --gas-tgas 300
   ```
2. **Decide.** If an image counts within the next hour (`images-pending` gives the time), wait.
   Otherwise go on.
3. **New practice programs at a fresh anchor:**
   ```
   ./target/release/antibody-services live --redo-practice
   ./target/release/antibody-services live --until practice
   ```
   - The first command picks a Sepolia block, rebuilds the practice and fork-mode bases, and waits for
     the light client to reach the block (about 16 minutes).
   - The second creates and activates the new practice program. Its activation is T1 on the new image.
     It then opens the program, points the vending contract at it (`set_terms` alone, by
     `antibody.testnet`: the hourly limit and the practice-account code stay as they are) and runs two
     judge tries. By hand, the same pointing is
     `antibody practice terms --account antibody.testnet --vending practice.antibody.testnet --program <new id>`.
   - Needs: `SEPOLIA_RPC` with `eth_getProof` near the head (`.env`); about 0.03 wNEAR of escrow and 0.35
     NEAR free on `antibody.testnet`.
4. **The deployments file.** Set `practice.program` in `deployments/near-testnet.json` to the new id and
   commit it. `try` already follows the vending contract; the file keeps `judge` and the docs in step.
5. **Check.** `antibody try --name runbook-<n>` must give `Done:Paid` with T1 in seconds (its summary:
   `T1-confirmed: paid at once`). Then
   regenerate JUDGES.md from the new results (`scripts/docs/judges.py`).

**Rollback.** None is needed: program 6 stays valid. Once its pending images count, `set_terms` can
point the vending contract back, or simply leave it.

**The same steps when the vaults run out.** Each of the 24 vaults pays once per program: once they are
proved, a try is a duplicate with no payout (`practice-vaults-low`, then `practice-vault-proved` in
`/v1/status`). Steps 3 to 5 give every vault back on a fresh program, with no image rotation involved.

## Prevention

Semih asks OutLayer not to rotate worker images during judging (2026-11-14 to 11-18). This runbook is
the fallback if they do.

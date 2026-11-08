# Runbook: the judging days (16–18 November 2026), and uptime to 25 November

Everything a judge touches runs on testnet: the locked contracts, the services (keeper, T1 relayer,
indexer, permit relay, T2 prover), the public API and the site. This runbook keeps them
answering. Nothing here changes a locked contract: every action is a top-up, an owner call the
locked contracts already allow, or a restart.

## The morning check (every day, before 09:00 UTC)

1. **`/v1/status` shows 0 warnings.** `curl -s $API/v1/status | jq '{ok, alerts}'`. `ok` is false
   only on a critical alert; aim for an empty `alerts` list. The site's `/status` page shows the same.
2. **Floats** (`components.floats`): every operating account at or above its float.
   - keeper ≥ 0.8 NEAR. `sync_images` sets 300 Tgas aside, about 0.301 NEAR, and so does opening a
     pull-request check's round (the keeper's account is the demo repository's bot): under 0.3 NEAR it can
     do neither;
   - T1 relayer ≥ 0.5 NEAR (each T1 costs it about 0.018 NEAR, one DCAP verification);
   - T2 prover ≥ 0.3 NEAR (`prover.antibody.testnet`: it pays the gas of each near-t2 submission).
3. **Credits** (`components.practice`, and the hunt's when it runs): the vending contract's NEAR
   credit at near-antibody at or above 0.1 NEAR. One try takes about 0.048 NEAR of it (a verification
   run's deposit and storage, measured 2026-11-07). The vending balance and the public vend key's
   allowance at or above their floors. The keeper also needs NEAR **credit** at near-antibody, beside its
   float: `sync_images` pays the storage of each image entry it lists from that credit, and fails with
   "not enough NEAR credit for storage" without it (fixed on 2026-11-07 with a 0.1 NEAR `storage_deposit`).
   **Vaults** (`components.practice.instances`): more than six of the 24 left unproved (`fresh`), and
   `next_is_fresh` true. Each vault pays once per program; when they run out, open a fresh practice
   program ([worker-images.md](worker-images.md), steps 3 to 5) before the judges' tries become
   duplicates.
4. **Worker images** (`components.images`): no unlisted image, and none still inside its delay. A
   pending image means runs on it get no T1 until it counts, and tries take the T0 path (paid after
   the challenge window, about a minute and a half).
5. **Collateral** (`components.t1`): the collateral source is `auto`, with no `t1-failed-*` or
   `t1-gave-up-*` alert.
6. **The drill** (when drills run): the latest `drill-*.json` in `/v1/moments` is from today and
   succeeded.
7. **T2 prover** (`components.t2_prover`): no `t2-prover-*` or `t2-contradicts` alert; the PROVE balance is
   above its 2 PROVE floor (one proof costs about 0.42 PROVE on SP1's prover network; the budget caps it at 6
   an hour and 40 a day).
8. **The site**: `/`, `/moment`, `/try`, `/judges` and one commit page answer 200.

## On a timer (the host's service manager, persistent across reboots)

| Timer | When (UTC) | What |
|---|---|---|
| `antibody-perhour-up` | 2026-11-15 18:00 | the practice vending contract's hourly limit to 30 (`antibody practice terms --per-hour 30`) |
| `antibody-practice-refresh` | 16, 17 and 18 November, 06:30 | a fresh practice program when 12 or fewer vaults are fresh (worker-images.md, steps 3 and 4, with the drill's kill switch set while it runs) |
| `antibody-perhour-down` | 2026-11-25 12:00 | the hourly limit back to 6 |

Each writes to the operations log. Check after each run that `/v1/status` shows the new `per_hour`, or
a new `practice.program` with 24 fresh vaults; the deployments file follows the program by hand (step 4).

## Pull-request checks (the demo repository)

[Himess/antibody-immune-demo](https://github.com/Himess/antibody-immune-demo) proposes every same-repository
pull request as a round of program 12 (docs/immune.md#re-runs-in-the-enclave).
- **Project key:** `acme.antibody.testnet`, a function-call key limited to `propose_round` and
  `__fastdata_fastfs`. Its NEAR credit at near-antibody pays the round check and every re-run, about
  0.05 NEAR a check: keep it above 0.3 NEAR (`antibody --account acme.antibody.testnet storage-deposit --near 0.5`).
- **Bot key:** `keeper.antibody.testnet`, limited to `open_round` and `allocate`. Opening a round needs about
  0.3 NEAR liquid on the keeper (its float, above).
- **When a check queues** ("the program's proposal slot stayed taken"): `antibody round state --program 12`
  says why. An accepted proposal past its `opens_from` opens with
  `antibody --account keeper.antibody.testnet --key-file <bot key> round open --program 12`; then re-run the
  pull request's checks.
- The bounty-window workflows (`examples/immune-demo/`) are not installed there: windows have not run on
  testnet.

## Top-ups

All from the root account, with the CLI (`antibody --network testnet ...`, signer and key file from
`.secrets/`; never print a key):

```bash
# a float: NEAR to an operating account
antibody admin send-near --to keeper.antibody.testnet --amount-near 0.2 --account antibody.testnet --key-file <root key file>
antibody admin send-near --to relayer.antibody.testnet --amount-near 0.5 --account antibody.testnet --key-file <root key file>

# a vending contract's NEAR credit at near-antibody (it pays each try's run and storage)
antibody practice fund --vending practice.antibody.testnet --near 0.5 --account antibody.testnet --key-file <root key file>

# a vending contract's balance (it lends each practice account its gas)
antibody admin send-near --to practice.antibody.testnet --amount-near 1 --account antibody.testnet --key-file <root key file>

# an account's NEAR credit at near-antibody for storage (the keeper's, for sync_images): 0.1 NEAR
antibody admin call --contract bounty.antibody.testnet --method storage_deposit --args '{"account_id":"keeper.antibody.testnet"}' --deposit-yocto 100000000000000000000000 --account antibody.testnet --key-file <root key file>

# the hourly vend limit for the judging days (usually 6), with set_terms alone; restore it after 25 November
antibody practice terms --vending practice.antibody.testnet --per-hour 30 --account antibody.testnet --key-file <root key file>

# ETH for the EVM relayer or the drill's base deployer (Sepolia, Base Sepolia): send it to the address
# /v1/status shows in components.evm_balances; the relayer pays each pause permit's gas
```

PROVE for the T2 prover is an Ethereum-mainnet token: the team deposits it by hand on SP1's prover network and
transfers it to the requester account. Nobody here signs a mainnet transaction.

Record each top-up (time, amount, transaction) in the day's operations notes.

## When something breaks

**A worker image rotation** (`images-unlisted` or `images-pending`): follow
[worker-images.md](worker-images.md). The keeper lists a new image within ten minutes if its float
allows; the image then counts for existing programs after the 2-day delay. Until then tries settle
on the T0 path, which the site says before a try starts. Nothing to fix on the site.

**An RPC outage** (components going stale, reads failing on the site): the services and the API fail
over to the RPCs in `$NEAR_RPC_EXTRA`. If the primary stays down, restart the services so they pick
the next one first. The site's checks read NEAR from the visitor's browser with their own fallbacks;
they say when an RPC does not answer.

**A failed drill**: the drill's results file says which step failed. A balance under what a drill
needs makes the drill skip itself and alert. Use the kill switch (see docs/drill.md) to stop drills
until the cause is fixed. The recorded run (2026-11-04) stays the site's hero either way.

**A service down**: every service runs under the host's service manager with automatic restart.
Check its state and the last lines of its log; restart it. The API keeps serving its last good
answers (stale-while-revalidate) while a dependency recovers.

## Reading the alerts

Each alert is `{level, component, key, message}`. Critical alerts make `ok` false.

| Key | Level | Meaning | Action |
|---|---|---|---|
| `float-<role>` | warn / critical | an operating account under its float (critical under a quarter) | top it up |
| `vending-credit` | warn | the vending contract's credit at near-antibody under 0.1 NEAR | `antibody practice fund` |
| `vending-balance` | critical | the vending contract cannot lend practice accounts their gas | `send-near` to it |
| `vend-key-allowance`, `vend-key-missing` | warn, critical | the public vend key is running out, or gone | add allowance / the key with `antibody practice deploy` |
| `escrow-<program>` | warn | a program's available escrow is under its largest payout | the project tops it up |
| `images-unlisted`, `images-pending` | critical, warn | a worker image OutLayer approved is not listed, or still in its delay | worker-images.md |
| `t1-waiting-<run>`, `t1-failed-<run>`, `t1-gave-up-<run>` | warn, warn, critical | a run's T1 relay waits, failed, or was given up | check the run's image and the collateral |
| `t1-collateral-expired`, `t1-collateral-pcs` | critical, info | OutLayer's collateral copy expired; with `auto`, fresh Intel PCS collateral is used | nothing with `auto` |
| `practice-vaults-low`, `practice-vault-proved` | warn, critical | six or fewer practice vaults left unproved, or the next try's vault is already proved (its try would be a duplicate) | a fresh practice program (worker-images.md, steps 3 to 5) |
| `relayer-pending` | warn | signed pause permits not yet relayed to their chain | check the EVM relayer key's balances |
| `evm-relayer-low-<chain>`, `evm-deployer-low-<chain>` | critical | the EVM relayer or the drill's base deployer under its floor on a chain | send it ETH on that chain |
| `drill-failed`, `drill-stale`, `drill-skipped` | critical, warn, warn | the latest drill stopped at a step, is older than a day plus six hours, or skipped itself (a balance under what a drill needs) | read its results file; top up; the kill switch if it must stop (docs/drill.md) |
| `keeper-error-<program>` | warn | a keeper pass on a program failed | read the keeper's log |
| `<component>-stale` | warn | a service has not advanced for 5 minutes | restart it |
| `t2-prover-balance`, `t2-prover-budget`, `t2-prover-failed`, `t2-contradicts` | warn / critical | the T2 prover is low on PROVE, at its budget, failing, or found a contradiction | top up PROVE; a contradiction is investigated before anything else |

## After 25 November

The `antibody-perhour-down` timer restores the practice vending's hourly limit to 6 (check it in
`/v1/status`). Stop the drills, and leave the services running as long as the floats last.

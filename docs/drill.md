# The daily drill

The recorded run (`docs/results/testnet-2026-11-04.json`) proves the one moment worked on that day. A
**drill** runs the same moment again on NEAR testnet every day, on a fresh instance of the Acme Lend
fixture, so the system is shown to work *today* and breakage is caught before judges see it: worker
images, DCAP collateral, the RPCs, floats, the relayer keys. The recorded run stays the hero on the site;
each drill is listed under it, labelled "automated drill".

```bash
antibody-services drill                 # one drill (resumable); writes drill-YYYY-MM-DD.json
antibody-services drill --dry-run       # read the guards only, spend nothing
antibody-services drill --cleanup       # close exitable drill programs (escrow back)
antibody-services drill --results-dir docs/results   # where the drill file is written (the API's --results-dir)
```

## What one drill is

One drill is the one moment, no more. Rounds, the other agents and the practice program stay in the full
story (`antibody-services live`). A drill, in order (each step resumable, recorded in `<work>/drill.json`):

1. **A fresh Acme Lend** on Sepolia (verifies and pauses) and Base Sepolia (pauses). The fixture's CREATE2
   addresses depend on the deployer, so each drill uses a **per-drill deployer** derived from the base
   deployer key with the drill's date — fresh addresses every day, no change to `contracts/`. The derived
   deployer is funded from the base deployer where it needs it.
2. **A new program** on a light-client anchor (a recent Sepolia block the light client holds), created by
   `acme.antibody.testnet`, named `Acme Lend drill YYYY-MM-DD`, then **activated** (its sealing key, T1).
3. **The scripted fixture agent A** submits its sealed Critical **over MCP** (exactly as in the recorded
   run: `submit` with `wait`).
4. **The verdict, then T1** (the run's TDX quote verified on NEAR).
5. **Pause permits** signed by the MPC network and relayed on both chains (the permit relayer uses the EVM
   key in `$RELAYER_EVM_KEY`).
6. **The payout**: an instant payout (the Critical is under the program's instant cap, paid on T1).

The drill program's settlement is the live story's Acme terms with a short deadline
(`DRILL_DEADLINE_SECS`, 12 h), so the program finishes in minutes and becomes exitable the same day.

## The schedule

A drill runs once a day. The timer (below) runs it; the cadence `/v1/status` expects is a day plus a
margin (see the alerts). Run it when EVM gas and the testnet are quiet; it takes about 20 minutes (most of
it waiting for the light client to reach the fresh anchor).

## The RPCs

- **Sepolia** (`SEPOLIA_TX_RPC`, else the first of `SEPOLIA_RPC`): forge forks the chain and reads
  accounts by block hash, so this RPC must answer those reads consistently. A load-balanced public RPC
  can answer "block not found" from a node a block behind (measured on 2026-11-08: one read in ten). The
  deployments are sent without `--slow`, so forge does not ask for the sender's nonce after each one.
- **NEAR** (`$NEAR_RPC`, else the deployments file's; `$NEAR_RPC_EXTRA` is the failover): on a host
  that also runs the services, give the drill another primary RPC than theirs, so the two do not share
  one rate limit. On 2026-11-08 a shared limit made the keeper pay agent A four seconds after the
  challenge window, so that payment was not instant and the drill reported it.

## The guards

Before anything is spent, the drill reads the balances a drill needs and **skips with a reason** if any is
low — it never fails half-way for want of funds:

- **NEAR**: `antibody.testnet` (tops the others up), `acme` (escrow + runs), `agent-a` (the run), `keeper`
  (the pause ticket and the payment), `relayer` (the T1 relays), each against its floor.
- **EVM**: the base deployer and the relayer on both chains, against what a drill's deploys and relays cost
  at the node's gas price.

A skipped drill writes a results file that says it was skipped and why (`all_ok: false`, so it is left off
the site), and `/v1/status` raises `drill-skipped`.

## The kill switch

One documented kill switch: a file **`<work-dir>/drill.disabled`**, or the env var
**`$ANTIBODY_DRILL_OFF`** (non-empty). The timer checks it; when present the drill does nothing, says so,
and writes a skipped results file. Remove the file (or unset the var) to re-enable. Ask OutLayer not to
rotate worker images during judging (docs/runbooks/worker-images.md); if a drill would be noisy during a
demo, set the kill switch.

## Timeouts

Each step has a timeout (the light client up to 90 min, T1 up to 15 min, the pauses up to 15 min). A step
that times out or fails ends the drill with `all_ok: false` and the step's error in the file, so `/moment`
and `/v1/status` show where it stopped. A failed `prebuild`/`anchor` is retried from a fresh anchor on the
next run (public RPCs prove only ~128 blocks back).

## The cost line

The results file's `cost` is read from balance deltas over the drill:

- `near`: each signer's balance change (negative means it received more than it spent — agent A's payout
  comes back to it);
- `sepolia_eth` / `base_sepolia_eth`: the deployer's and the relayer's ETH change per chain;
- `outlayer_run_deposits`: the gross NEAR deposit attached to the two OutLayer runs (activation and A's
  verify); the unused part is refunded, so it is already netted into the NEAR deltas.

## Cleanup

A drill program closes when its windows allow, so its escrow (0.4 wNEAR) returns to the project.
`antibody-services drill --cleanup` (a later drill may do the same) exits every drill program past its
deadline and the exit grace, then closes its commit records a page at a time. Drill programs are labelled
by their name (`Acme Lend drill …`); `antibody judge` marks them `an automated drill`.

## Reading a drill

- **`/moment`**: the drill appears in "The runs" under the recorded run, labelled "automated drill"; pick
  it to replay its steps (verified, T1, pause permits signed, paid, each chain paused), each with its
  transaction. The API serves it at `/v1/moments` (the `drill-*.json` files; no API change — `moments()`
  reads them in the one-moment shape).
- **`/status`**: the `drill` component shows the latest drill's date, whether it passed, its steps and its
  cost; the `evm_balances` component shows the relayer and deployer balances against their floors. Alerts:
  - `drill-failed` (critical) when the latest drill did not pass, naming the step it stopped at;
  - `drill-stale` (warn) when the latest drill is older than the cadence (24 h) plus the margin (6 h);
  - `drill-skipped` (warn) with the reason a drill was skipped;
  - `evm-relayer-low-<chain id>` / `evm-deployer-low-<chain id>` (critical) when an EVM balance is below
    its floor.

## The systemd service and timer

Generic templates (no host details) are in `docs/systemd/`. Install them on the operations machine,
filling the placeholders (`<install-dir>`, `<user>`, the environment file). The timer runs the drill once
a day; the service runs `antibody-services drill` in the repository with `.env` and `.secrets/` in place.

```ini
# /etc/systemd/system/antibody-drill.service  (see docs/systemd/antibody-drill.service)
[Unit]
Description=Antibody daily drill

[Service]
Type=oneshot
WorkingDirectory=<install-dir>
EnvironmentFile=<install-dir>/.env
ExecStart=<install-dir>/target/release/antibody-services drill --results-dir <install-dir>/docs/results
```

```ini
# /etc/systemd/system/antibody-drill.timer  (see docs/systemd/antibody-drill.timer)
[Timer]
OnCalendar=*-*-* 03:00:00 UTC
Persistent=true
```

To pause the drill without touching systemd, set the kill switch:
`touch <install-dir>/target/drill/drill.disabled`.

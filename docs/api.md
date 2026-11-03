# API and services (`antibody-services`)

The backend around the contracts. Nothing it does is trusted by them: the API serves near-antibody's
own views and the indexer's event log, and the bots only make permissionless calls.

```bash
# indexer + relayer + notifier + API, and the keeper bot and T1 relayer with their own accounts
antibody-services --network testnet all --listen 127.0.0.1:8787 \
  --keeper-account keeper.antibody.testnet --t1-account relayer.antibody.testnet
antibody-services --network testnet api                     # the API alone (reads the indexer's state dir)
antibody-services --network testnet keeper --account keeper.antibody.testnet --once
```

| Service | What it does |
|---|---|
| `indexer` | Every NEP-297 event of near-antibody, the token and near-guardian, with its transaction, height and time, in `events.jsonl` (append-only, `seq` ordered). A cursor makes it resume without losing or repeating events; `--from-block` backfills. |
| `keeper` | On every open program, every 30 s: allocate in commit order, pay funded claims, the round's pause ticket, debts, re-runs at a proposed fix, pending permits (`keeper.json` records each pass). |
| `t1` | Relays each answered run's TDX quote to near-antibody, which verifies it on chain (the run becomes T1). |
| `relayer` | Submits near-guardian's signed pause and release permits to each chain's adapter. |
| `notifier` | Posts new events, and each new monitoring alert once, to the webhooks in `$ANTIBODY_WEBHOOKS` (never printed). |
| `api` | The endpoints below. |

## Endpoints

The machine-readable form is [`openapi.yaml`](openapi.yaml); its examples are the sandbox demo's
payloads ([`results/api-examples.json`](results/api-examples.json)).

| Endpoint | Returns |
|---|---|
| `GET /health` | The network, the contract ids, the version and the endpoints |
| `GET /v1/status` | Each service's last progress, permits not yet confirmed, each open program's escrow against its largest payout, the practice faucet (below), and the alerts |
| `GET /v1/stats` | Contract totals, and commit-to-verdict and commit-to-payout latencies (p50, p95) from the event log |
| `GET /v1/programs` | Every program: name, project, phase, round, anchor, commits, ledger |
| `GET /v1/programs/{id}` | The program, its rules (each slot's severity and payout, the payout table, bond and instant terms, escrow coverage, the sealed known-issue count, the anchor and its check), its meta, its record (bait metrics), its audit, its commits and its events |
| `GET /v1/programs/{id}/feed` | The sealed feed: who committed when, the state, the verdict (kind, tier, keys) and the payout. Never the PoC |
| `GET /v1/commits/{id}` | One commit as stored, its feed row, its events with transaction links, and its proof card (the facts, the receipt hash, whether its proof is still sealed, how to check it). Its `run_requested` events name the transactions that carried the sealed blob: `antibody program read --api` finds the blob there |
| `GET /v1/runs/{id}` | One verification run: kind, keys, impact, tier |
| `GET /v1/accounts/{id}` | Credit at near-antibody, the agent statement (bonds, verification spend, payouts, containment events), the token balance |
| `GET /v1/leaderboard` | Paid findings and payouts by beneficiary, with duplicates and forfeited commits |
| `GET /v1/events?after=&limit=` | Events in `seq` order, each with `tx`, `block_height`, `block_ts_ns`, `explorer` |
| `GET /v1/guardian` | near-guardian's configuration, its signed permits, pending permits, the relayer's state |
| `GET /v1/judge` | `antibody judge`: code hashes, locked status, pins, T1 confirmations, payouts, permits, rule strength |
| `GET /v1/programs/{id}/reflex` | The reflex and release panel: per chain, live, paused or released, with each permit and its transaction there |
| `GET /v1/trust` | The trust panel: each actor, what it is trusted for, and how anyone checks it |
| `GET /v1/practice` | The try-it panel: the practice program, its faucet's balance, the public vend key, the steps |
| `GET /v1/stream?after=` | The live timeline: every new event as a server-sent event (`event: antibody`, the event's JSON as data) |
| `GET /v1/mock/mainnet` | `{"mock": true, …}`: recorded data, never live |

## Monitoring

`/v1/status` raises an alert when:
- a service's state has not advanced for 5 minutes (indexer, keeper, notifier), or a keeper pass failed;
- a permit is not yet confirmed on its chain;
- an open program's available escrow is below its largest payout;
- **the practice faucet** runs low (faucet-drain protection): the vending contract below 0.5 NEAR
  (critical), the public vend key gone (critical) or below 0.05 NEAR of allowance, or the vending
  contract's NEAR credit at near-antibody below 0.1 NEAR. Vending is also rate-limited per hour by the
  contract itself, and the key's allowance caps what calls to `vend` can burn.

## Events

near-antibody's events (standard `antibody`): `program_created`, `program_escrowed`, `anchor_checked`,
`program_activated`, `program_opened`, `program_rejected`, `commit`, `reveal`, `holder`, `paid`,
`duplicate`, `duplicate_confirmed`, `known_match`, `forfeited`, `gap_claim`, `gap_promoted`, `promoted`,
`challenge`, `challenge_upheld`, `challenge_overturned`, `pause_ticket`, `round_proposed`,
`round_rejected`, `rerun_requested`, `round_opened`, `disclosed`, `run_failed`, `retry_available`,
`withdraw`, `top_up`, `debt_funded`, `program_exited`, `program_closed`, `containment`, and the
deployment's own (`images_synced`, `tee_confirm_requested`, `tee_rejected`, `defaults_changed`).
near-antibody also emits `permit_requested` when it asks near-guardian to sign. near-guardian's (standard
`antibody-guardian`): `program_registered`, `chains_set`, `permit_queued`, `permit_signed`, `permit_skipped`,
`permit_failed`.
The practice vending contract (`antibody-practice`): `vended`. The verdict guard
(`antibody-verdict-guard`): `frozen`.

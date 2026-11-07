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
| `GET /v1/status` | Each service's last progress, permits not yet confirmed, each open program's escrow against its largest payout, the practice faucet (below), the T1 relayer (its collateral source, when OutLayer's on-chain DCAP collateral expires, the runs it could not confirm), OutLayer's worker images against near-antibody's list (unlisted ones, and listed ones still inside the image delay), and the alerts |
| `GET /v1/stats` | Contract totals, and commit-to-verdict and commit-to-payout latencies (p50, p95) from the event log |
| `GET /v1/programs` | Every program: name, project, phase, round, anchor, commits, ledger |
| `GET /v1/programs/{id}` | The program, its rules (each slot's severity and payout, the payout table, bond and instant terms, escrow coverage, the sealed known-issue count, the anchor and its check), its meta, its record (bait metrics), its audit, its commits and its events |
| `GET /v1/programs/{id}/feed` | The sealed feed: who committed when, the state, the verdict (kind, tier, keys, and `t2`: near-t2's outcome for its run, when there is one) and the payout. Never the PoC |
| `GET /v1/commits/{id}` | One commit as stored, its feed row, its events with transaction links, and its proof card (the facts, the receipt hash, whether its proof is still sealed, how to check it: `antibody status`, `antibody tee check`, and on testnet `outlayer-verify tx <the transaction that requested the run>`). Its `run_requested` events name the transactions that carried the sealed blob: `antibody program read --api` finds the blob there |
| `GET /v1/runs/{id}` | One verification run: kind, keys, impact, its TEE tier, and `tier` with T2 (T0, T1, or T2 when near-t2 confirmed a validity proof of it), with near-t2's record (`t2`) |
| `GET /v1/accounts/{id}` | Credit at near-antibody, the agent statement (bonds, verification spend, payouts, containment events), the token balance |
| `GET /v1/leaderboard` | Paid findings and payouts by beneficiary, with duplicates and forfeited commits |
| `GET /v1/events?after=&limit=` | Events in `seq` order, each with `tx`, `block_height`, `block_ts_ns`, `explorer` |
| `GET /v1/guardian` | near-guardian's configuration, its signed permits, pending permits, the relayer's state |
| `GET /v1/judge` | `antibody judge`: code hashes, locked status, pins, T1 confirmations, payouts, permits, rule strength |
| `GET /v1/programs/{id}/reflex` | The reflex and release panel: per chain, live, paused or released, with each permit and its transaction there |
| `GET /v1/trust` | The trust panel: each actor, what it is trusted for, and how anyone checks it |
| `GET /v1/practice` | The try-it panel: the practice program, its faucet's balance, the public vend key, the steps |
| `GET /v1/stream?after=` | The live timeline: every new event as a server-sent event (`event: antibody`, the event's JSON as data) |
| `GET /v1/mainnet` | How mainnet works (docs/mainnet.md), with a live read-only check of NEAR mainnet's Ethereum light client against Ethereum; view calls only, `transactions_sent: 0` |
| `GET /v1/t2` | T2 (docs/zk.md): near-t2's pins (SP1 6.10.0's Groth16 key, the guest's program vkey, near-antibody, the verifier), its counts, every record (confirmed or contradicted, with the receipts compared), the indexed `t2_*` events and the prover service's state. A validity proof of a disclosed verdict: no privacy is claimed, and payouts in the locked deployment do not depend on it |
| `GET /v1/moments` | The one moment's runs, from the results files (`docs/results/`): recorded runs (`testnet-YYYY-MM-DD.json`) and scheduled drills (`drill-*.json`), newest first; each step with its seconds from the commit's NEAR block and its transaction (the results file's, else the indexer's event) |
| `POST /v1/interest` | The pilot and early-access form (the web app's `/pilot`): checked, kept in a private file on the API's host (`$ANTIBODY_INTEREST_FILE`, never served or committed) and announced through `$ANTIBODY_WEBHOOKS`. Only the site's origins (`$ANTIBODY_WEB_ORIGINS`), five an hour per client; a filled honeypot field is answered as accepted and kept nowhere |

`/v1/events` and `/v1/stream` take `scope=antibody` for Antibody's own events only (standards `antibody*`): the
indexer also records the other events of the transactions it reads, the payout token's among them. `/v1/stats` adds
`verdict_latency`: from each commit's NEAR block to its verify run's answer (the TEE's verdict).

**Serving it in public.** `antibody-services api --chain <id>=<rpc> --results-dir docs/results` behind a TLS reverse
proxy (Caddy on our host; the live stream passed on unbuffered). Reads are public (CORS `*`); a token bucket per client
(the proxy's `X-Forwarded-For`), `cache-control` per route and a short server cache for the views that read many
contracts (`/v1/judge` takes about 90 s of view calls on testnet and is warmed at start). With `--chain`, a permit the
relayer has no record of shows as applied when its adapter's `used(key)` view says so, and a recorded run's relay
(`docs/results/`) gives its transaction.

## Monitoring

`/v1/status` raises an alert when:
- a service's state has not advanced for 5 minutes (indexer, keeper, notifier), or a keeper pass failed;
- a permit is not yet confirmed on its chain;
- an open program's available escrow is below its largest payout;
- **the practice faucet** runs low (faucet-drain protection): the vending contract below 0.5 NEAR
  (critical), the public vend key gone (critical) or below 0.05 NEAR of allowance, or the vending
  contract's NEAR credit at near-antibody below 0.1 NEAR. Vending is also rate-limited per hour by the
  contract itself, and the key's allowance caps what calls to `vend` can burn.

More alerts:
- an operating account (the keeper, the T1 relayer, the T2 prover) has less NEAR liquid than its float
  (`--keeper-float`, `--relayer-float`, `--prover-float`): a warning, and critical below a quarter of it;
- the T2 prover: its PROVE balance is low (it pauses), its budget is reached, a proof failed, or near-t2
  recorded a contradiction (critical).

**A second NEAR RPC.** `$NEAR_RPC_EXTRA` (comma-separated URLs, a keyed one included) is tried in turn
when the primary RPC fails or rate-limits. `$NEAR_RPC_API_KEY` goes to FastNear hosts as a bearer token.
Neither is ever printed.

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
(`antibody-verdict-guard`): `frozen`. near-t2 (`antibody-t2`): `t2_confirmed` (a run's validity proof
equals near-antibody's record of it), `t2_contradicts` (the same input re-ran to another receipt, kind or
slots: recorded, it reverses nothing in the locked deployment), `t2_rejected` (the proof is about
something else; nothing stored, the deposit refunded).

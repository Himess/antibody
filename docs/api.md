# API (`antibody-services api`)

The API is read-only JSON over HTTP, with CORS open for GET. It serves:

- live views of the near-antibody contract;
- the indexer's event history, where every event carries its NEAR transaction hash and explorer link;
- the relayer's pause status;
- MOCK mainnet data.

```bash
antibody-services --network testnet all --listen 127.0.0.1:8787     # indexer + relayer + notifier + api
antibody-services --network testnet api                              # api only (reads the indexer's state dir)
```

| Endpoint | Returns |
|---|---|
| `GET /health` | Network and contract ids |
| `GET /v1/stats` | Contract stats (programs, escrow, findings, paid, duplicates, slop) and latencies from the event log: submit→verdict and submit→payout (p50/p95) |
| `GET /v1/programs` | All programs |
| `GET /v1/programs/{id}` | Program rules and anchor trust, claimed keys, commits, and the program's events with tx links |
| `GET /v1/programs/{id}/feed` | Sealed submissions feed: commit hash, hunter, height, status, verdict kind, keys, outcome, receipt hash. Never PoC content |
| `GET /v1/commits/{id}` | One commit, its duplicate/known proof (earlier claim, equal keys, both receipt hashes, tier, sealed-until; content keys after disclosure), its events, and the local verify command |
| `GET /v1/leaderboard` | Hunters by earnings, then valid findings; shows valid-but-late (duplicates) and slop counts |
| `GET /v1/accounts/{id}` | Internal balance, reputation, tUSD balance |
| `GET /v1/events?after=&limit=` | NEP-297 events in order (`seq`), each with `tx`, `block_height`, `block_ts_ns`, `explorer` |
| `GET /v1/guardian` | Registered chains, the latest signed pause per chain, and the relayer state (EVM tx hash, receipt, `paused()`) |
| `GET /v1/mock/mainnet` | `{"mock": true, …}` recorded data: v0.3 evidence (replays, full-block validation, light-client anchoring, forged header) and a mock live program |

Event names (standard `antibody`): `program_created`, `anchor_checked`, `program_rejected`, `program_funded`,
`known_issue_submitted`, `known_issue_claimed`, `program_opened`, `deposit`, `committed`, `submitted`,
`verify_failed`, `verdict`, `verdict_error`, `pause_requested`, `duplicate`, `known`, `payout_delayed`, `paid`,
`payout_failed`, `challenged`, `challenge_resolved`, `expired`, `top_up`, `withdraw`, `escrow_withdrawn`,
`disclosed`, `disclosure_failed`, `program_misconfigured`, `verifier_set`, `verifier_pubkey`, and for rounds
`round_proposed`, `rerun_requested`, `rerun_result`, `rerun_failed`, `round_opened`, and for T1
`tee_config_set`, `tee_confirm_requested`, `tee_confirmed`, `tee_rejected`. The guardian
emits `pause_signed` (standard `antibody-guardian`).

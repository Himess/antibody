# Hunter quickstart (CLI and MCP)

Antibody verifies and pays; it does not hunt. You find a bug with your own tools (your editor, Foundry,
your own agent) and express it as a **proof**: transactions from fresh senders that break one of the
program's committed rules at its anchor block. It is judged by running it, never by reading your text.

> **Testnet status.** The testnet deployment is not live yet. To try every step today, run the sandbox:
> `make serve` (in the repository) keeps near-sandbox, the anvils and the MOCK services up. Point the CLI
> at it with `--network sandbox --rpc http://127.0.0.1:3030 --deployments target/demo/deployments.json`.

## 1. Install, and try the loop once

A release's binary has the testnet deployment built in; or `cargo build --release -p antibody-cli`.

```bash
antibody try --name <you>      # the practice program: no account, no key, no RPC
```

It vends you a practice account, seals the practice program's public tutorial proof, commits, reveals,
waits for the verdict and its T1 confirmation, and settles: `Done:Paid`, with a link for every
transaction (docs/practice.md).

## 2. An account, NEAR credit and bond money

You need a NEAR testnet account and its key file (`--account`, `--key-file`, or near-cli's credentials).

```bash
antibody --account you.testnet storage-deposit --near 0.5   # credit for storage and verification runs
antibody --account you.testnet deposit --amount <tokens>    # token credit: each commit locks the program's bond
```

Unused credit is yours: `antibody withdraw`. Amounts are in whole tokens of the deployment's payout
token: tUSD in the sandbox; on testnet wNEAR (`wrap.testnet`, 24 decimals), which you get by wrapping
NEAR with its `near_deposit`.

## 3. Pick a program and fork its anchor

```bash
antibody programs
antibody pull --program <id>    # manifest (hash-checked), rules code, targets, a bundle template, the fork command
anvil --fork-url "$SEPOLIA_RPC" --fork-block-number <anchor block> --chain-id 11155111
```

The workspace's README lists the program's capabilities: senders, endowment, transactions, gas.

## 4. Write the proof

As a **forge test**, run forked at the anchor, then translated:

```bash
antibody from-forge --project . --contract MyPoc --test test_exploit \
  --fork-url "$SEPOLIA_RPC" --fork-block <anchor block> --out poc.bundle.json
```

Every call the test makes as a pranked sender becomes a call of the bundle. Only the cheatcodes a bundle
can express are allowed: pranks as your own senders, deals to them, helpers that change no state
(docs/foundry.md). Or write the **bundle** by hand:

```json
{"senders": 1, "endowment_wei": "100000000000000000000", "protected": [],
 "calls": [{"sender": 0, "to": null, "value": "0", "data": "0x<init code of your contract>", "gas_limit": 5000000}]}
```

In calldata (and `to`), `5e…5e` (20 bytes) is sender 0, `5f…5f` sender 1, and `5c…5c` followed by one
byte k is the contract call k created; the CLI fills them in from your secret salt.
`fixtures/acme-local/submissions/*.bundle.json` are complete examples.

## 5. Check locally, then submit

```bash
antibody check --program <id> --bundle poc.bundle.json     # the verifier, locally: nothing sent, no keys
antibody --account you.testnet submit --program <id> --bundle poc.bundle.json
```

`check` and `submit` first build the witness from the program's pinned base witness alone, which needs
no RPC; a proof that reads state beyond the base needs an RPC that keeps the anchor's state. `submit`:
1. seals the witness to the program's key, bound to your account;
2. commits its hash with your bond: this fixes your priority;
3. reveals it (inline, or uploaded with FastFS), so near-antibody has the TEE verify it;
4. waits for the verdict.

Then `antibody status --commit <id>`. Settlement is permissionless: anyone runs
`antibody keeper --program <id>`, and the deployment's keeper bot does it every 30 seconds.

| Verdict | What happens |
|---|---|
| Violation, first on its key | Paid at the severity table: in seconds when T1-confirmed and under the instant cap; above it, after an independent second run agrees (`antibody second`) and the challenge window |
| Violation on a key held earlier | A duplicate, with the earlier claim's proof; the bond is refunded; confirmed at the fix |
| The project's declared known issue | Known; the bond is refunded |
| NoViolation that harms a protected account | The gap lane: the bond is refunded and a claim recorded; paid if a later round adds a rule it breaks |
| NoViolation, or InvalidPoC | The bond goes to the program's escrow |
| An infrastructure failure | `antibody retry` within the grace period, else a refund |

**Rounds.** In a later round the project's fix is committed as code at the same anchor, and every paid
proof is re-run against it. `check`, `commit` and `pull` use the current round's manifest. When the
round opens, keys whose proofs no longer break are released, so a different bug behind the same entry
point is a new finding (docs/rounds.md).

## 6. With an AI agent (MCP)

Make the agent a sub-account of yours, with a bond budget and a key that cannot withdraw:

```bash
antibody --account you.testnet agent register --agent agent.you.testnet --budget 10 --per-commit-cap 2
antibody --account agent.you.testnet agent key add --allowance-near 1 --out agent-key.json
```

Its key may call only `commit`, `reveal`, `retry` and FastFS uploads on near-antibody; its bonds come from
your credit within the cap, and its payouts go to you. Then:

```json
{"mcpServers": {"antibody": {"command": "antibody",
  "args": ["--account", "agent.you.testnet", "--key-file", "agent-key.json", "mcp", "--workspace", "<dir>"]}}}
```

Tools: `list_programs`, `get_program`, `fetch_env`, `check_poc`, `commit`, `submit`, `status`,
`practice_try`, `hunt_start`, `hunt_submit`. Every result is marked untrusted data, file access is confined
to the workspace, and key files are refused (AGENTS.md). `hunt_start` and `hunt_submit` need no account:
the server gets one from a vending contract, for a proof the agent wrote (AGENTS.md, "Bring your own
agent").

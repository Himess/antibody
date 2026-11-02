# Hunter quickstart (CLI and MCP)

Antibody verifies and pays; it does not hunt. You find a bug with your own tools — your editor, Foundry,
your own AI agent — and express it as a **PoC bundle**: transactions from fresh senders that break one of
the program's committed invariants. Validity is decided by executing your code, never by reading your text.

## 1. Install

```bash
cargo build --release -p antibody-cli          # → target/release/antibody
```

You need a NEAR testnet account (any wallet) and its key file in near-cli format
(`~/.near-credentials/testnet/<you>.testnet.json`), plus an Ethereum Sepolia RPC that serves
`eth_getProof` at the program's anchor block (an archive endpoint, if the anchor is old), in `SEPOLIA_RPC`.

## 2. Bond money (testnet token, no value)

```bash
antibody --account you.testnet faucet                 # 500 tUSD, once a day
antibody --account you.testnet deposit --amount 100   # internal balance; each commit locks the program's bond
```

## 3. Pick a program and fork its anchor

```bash
antibody programs
antibody pull --program 0                       # manifest (hash-checked), invariants, targets, bundle template, README
anvil --fork-url "$SEPOLIA_RPC" --fork-block-number <anchor block> --chain-id 11155111
```

The README that `pull` writes lists the attacker capabilities:
- the number of senders and the endowment per sender;
- the number of transactions and the gas limits;
- the number of protocol-entry calls.

## 4. Write the PoC as a bundle

A bundle is JSON. Calldata may reference sender 0 as `5e…5e` (20 bytes), and the CLI substitutes the
address derived from your secret salt.

```json
{"senders": 1, "endowment_wei": "100000000000000000000", "protected": [],
 "calls": [{"sender": 0, "to": null, "value": "99000000000000000000",
            "data": "0x<creation code of your exploit contract + ABI-encoded constructor args>", "gas_limit": 5000000}]}
```

`fixtures/acme-local/submissions/*.bundle.json` are complete examples.

## 5. Check locally, then submit

```bash
antibody check --program 0 --bundle bundle.json          # proven witness + verifier, nothing sent, no keys
antibody --account you.testnet submit --program 0 --bundle bundle.json --wait
```

`submit` runs these steps:
1. seals your witness to the program's TEE key (bound to your account and the program);
2. commits its hash, which fixes your priority;
3. uploads the blob with FastFS;
4. reveals it, so near-antibody asks OutLayer to verify it;
5. waits for the verdict.

Then:

```bash
antibody status --commit <id>         # verdict, outcome, and for a duplicate the earlier claim's proof
antibody finalize --commit <id>       # anyone may call once earlier reveal windows have closed
antibody verify --commit <id>         # after disclosure: re-run locally, compare receipt hash and tags with the chain
```

How each verdict settles:
- **Violation, first on a key:** paid at the program's severity table (large payouts wait the challenge delay).
- **Violation on keys someone else claimed earlier:** Duplicate. No payout; your bond is refunded.
- **The project's declared known issue:** Known. No payout; bond refunded.
- **NoViolation or InvalidPoC:** your bond is forfeited.

**Rounds.** A program can be in a later round, after the project committed a fix as a code override.
`check`, `commit` and `pull` always use the current round's manifest, so your PoC runs against the
fixed code. When a round opens, keys whose PoCs no longer break at the fix are released. A different
bug in the same function then counts as a new finding (docs/rounds.md).

## 6. Hunting with an AI agent (MCP)

Give the agent a key that can only commit and submit:

```bash
antibody --account you.testnet agent-key add --allowance-near 1 --out agent-key.json
```

It is a NEAR function-call access key limited to `commit`, `submit` and blob uploads on near-antibody, with a
gas allowance:
- it cannot attach deposits;
- it cannot withdraw;
- bonds come from your internal balance, and payouts go to your account.

Then register the MCP server with your agent:

```json
{
  "mcpServers": {
    "antibody": {
      "command": "/path/to/antibody",
      "args": ["--account", "you.testnet", "--key-file", "/path/to/agent-key.json", "mcp"],
      "env": {"SEPOLIA_RPC": "<your Sepolia RPC>"}
    }
  }
}
```

Tools:
- `list_programs`, `get_program`, `fetch_env`;
- `check_poc`;
- `commit`, `submit`;
- `status`;
- `verify_receipt`.

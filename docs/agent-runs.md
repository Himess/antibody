# Measured agent runs

Can an off-the-shelf AI agent, given only Antibody's MCP server, complete the whole loop: get an
account, seal a proof, commit, have it verified in the TEE, and be paid? These runs measure that, on the
practice program, with the tutorial proof (a public, textbook bug: docs/practice.md). They measure
**completing the loop**. They do not measure finding bugs.

## Method

`scripts/agents/measured_runs.py` runs the same task N times:
- **The agent:** Claude Code (`claude -p`), as installed, with its default model. Every built-in tool is
  off (`--tools ""`) and only Antibody's MCP server is configured (`--strict-mcp-config`), so the agent
  can act only through the MCP tools.
- **A fresh workspace per run**, and a fresh practice account name.
- **The task** is one fixed prompt: complete the loop once with the tutorial proof and report the
  verdict, the final state and the payout.
- **Success is read from near-antibody**, never from the agent's words: the practice account's commit
  must end `Done:Paid`.
- **Recorded per run:** wall time, the tool calls, the verification spend and bonds from the agent
  statement (`agent_statement`), the payout, the agent's own cost as Claude Code reports it, and the
  full transcript.

## Results

| Where | Runs | Report |
|---|---|---|
| Sandbox (`antibody-services demo --serve`: near-sandbox and anvil, MOCK OutLayer and MOCK DCAP, the real verifier) | see the report | [`agent-runs/sandbox-2026-11-03/REPORT.md`](agent-runs/sandbox-2026-11-03/REPORT.md) |

Each report links the transcripts (`logs/run-NN.jsonl`). In the sandbox the TEE quote is a MOCK
(the T1 checks on it are the real code); a testnet run uses OutLayer's real TEE workers.

## Reproduce

```
antibody-services demo --serve        # on Linux; leaves the practice program up
python3 scripts/agents/measured_runs.py --runs 10 --network sandbox --rpc http://127.0.0.1:3030 \
  --deployments target/demo/deployments.json --antibody target/release/antibody --out <dir>
```

With `--remote "ssh <host>"`, the MCP server runs on the host next to the chains and the agent runs
locally, talking to it over SSH stdio.

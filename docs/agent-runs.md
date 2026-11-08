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
- **The task** is one fixed prompt per task: (a) complete the loop once with the tutorial proof through
  `practice_try`; (a2) write the proof yourself with `hunt_start`, check it with `check_poc`, submit it with
  `hunt_submit` (the agent also has file tools in its workspace: Read, Grep, Glob, Write). Each reports the
  verdict, the final state and the payout.
- **Success is read from near-antibody**, never from the agent's words: the practice account's commit
  must end `Done:Paid`.
- **Recorded per run:** wall time, the tool calls, the verification spend and bonds from the agent
  statement (`agent_statement`), the payout, the agent's own cost as Claude Code reports it, and the
  full transcript.

## Results

**On testnet, 2026-11-08**, with the release binary (antibody 0.5.0, as judges download it) as the MCP
server, Claude Code 2.1.319 with its default model (claude-opus-5-5), on practice program 10:

| Task | Runs | Paid within the agent's own call | Paid on near-antibody, read again | Median time (paid in its call) | Agent cost |
|---|---|---|---|---|---|
| [a](agent-runs/testnet-2026-11-08/task-a/REPORT.md): the tutorial proof through `practice_try`, no built-in tools | 10 | 7 | 10 | 98 s | $1.86 |
| [a2](agent-runs/testnet-2026-11-08/task-a2/REPORT.md): the agent writes the proof itself (`hunt_start`, `check_poc`, `hunt_submit`; file tools in its workspace) | 6 | 3 | 6 | 91 s | $2.47 |

- **Every commit ended `Done:Paid`** (commits 38 to 53, each listed in its report with its transaction
  links in the transcript).
- **In task a2 every agent wrote the same correct proof**: the call that drains its own vault through
  the unguarded `sweep`. Each checked it with `check_poc` before it submitted, and each got a T1
  `Violation`.
- **The six calls that did not see their payout.** These runs set `$NEAR_RPC_EXTRA` to a free dRPC
  endpoint as the failover. The v0.5.0 binary also sends transactions through it, and the free plan
  timed out four of them ("Request timeout on the free plan"): tasks a 4 and 7, and a2 2 and 3. Their
  calls stopped after the commit and the reveal had landed. Two more calls (a 2, a2 5) returned while
  the payout waited in the program's queue, where claims go in commit order. The services' keeper
  settled all six as paid within minutes. Since v0.5.1 the CLI sends transactions on the primary RPC
  only.
- **What these runs measure:** completing the loop, with an off-the-shelf agent and only Antibody's
  tools. They do not measure finding bugs: the practice program's bug and proof are public.

Each report has the prompt, every run's commit, state, time, tool calls and cost, and the full
transcripts (`logs/run-NN.jsonl`, Claude Code's stream-json, local paths replaced). Re-read a run's commit
with `antibody status --commit <id>`.

## Bring your own agent

`practice_try` always sends the tutorial proof. An agent can also write the proof itself and submit it
through an account the server vends it, with no NEAR account configured:

```json
{"mcpServers": {"antibody": {"command": "antibody", "args": ["mcp", "--workspace", "<dir>"]}}}
```

1. `hunt_start` (`name`; `vending`: the deployment's hunt vending contract by default, or
   `practice.antibody.testnet` for the practice program): the account, its instance's addresses, the rules
   and what each pays, the program's limits, and a workspace `hunt-<name>/` (the proof format, and for the
   practice program its sources and a Foundry harness that runs offline). The account's key stays with
   the server.
2. `check_poc` (`program_id`, `bundle_path`): the verifier on the agent's machine, nothing sent.
3. `hunt_submit` (`name`, `poc`): refused before anything is spent when the bundle is malformed, calls
   another account's instance, or seals to more than 15 KB; otherwise sealed, committed and revealed, the
   verdict waited for with T1 or the end of the challenge window on NEAR's clock, settled, and the account
   closed.

The tools vend only from the deployment's hunt and practice vending contracts. From a terminal the same
steps are `antibody hunt start`, `antibody check`, `antibody hunt submit` and `antibody hunt status`
(docs/practice.md). On the practice program the agent can write the tutorial proof itself, from the
workspace's `PROOF.md`. Task a2 above measures exactly this: an agent writing and submitting a proof (not
finding a bug).

## Reproduce

```
make serve                            # on Linux; leaves the practice program up
python3 scripts/agents/measured_runs.py --runs 10 --network sandbox --rpc http://127.0.0.1:3030 \
  --deployments target/demo/deployments.json --antibody target/release/antibody --out <dir>
```

With `--remote "ssh <host>"`, the MCP server runs on the host next to the chains and the agent runs
locally, talking to it over SSH stdio.

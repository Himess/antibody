# Measured agent runs: task a2, testnet, 2026-11-08 09:19 UTC

An off-the-shelf agent (Claude Code) with Antibody's MCP server and file tools in its workspace (Read, Grep, Glob, Write) writes the practice program's proof itself (a public, textbook bug): `hunt_start` on the practice vending contract, the bundle written from the workspace's sources and proof format, `check_poc`, `hunt_submit`. This measures completing the loop: sealing, verification and payment. It is not bug finding. Success is read from near-antibody, not from the agent.

- Versions: antibody 0.5.0 (the MCP server); Claude Code 2.1.319 (Claude Code)
- Runs: 6; paid within the agent's own call: 3 (50%)
- Paid on near-antibody when read again (2026-11-08 09:40 UTC): 6 of 6 (a payout the agent's call did not wait for, or did not live to see, settled by the services' keeper)
- Wall time of a run paid within its call: median 90.7 s (min 89.7, max 95.2)
- Paid (all runs): 0.003 wNEAR within the calls, 0.006 wNEAR when read again; bonds lost: 0 wNEAR
- Agent cost (all runs, USD, as Claude Code reports it): 2.473; models: claude-opus-5-5

| Run | Paid in its call | Commit | State at its end | State read again | Seconds | Tool calls | Verification (NEAR) | Bonds lost (wNEAR) | Paid (wNEAR, read again) | Net (wNEAR, in its call) | Cost (USD) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | yes | 48 | Done:Paid | Done:Paid | 95.2 | hunt_start, fetch_env, Read, Read, Read, Write, check_poc, hunt_submit | 0.0000 | 0 | 0.001 | 0.001 | 0.291 |
| 1 | yes | 49 | Done:Paid | Done:Paid | 89.7 | hunt_start, Read, Read, Read, Read, Write, check_poc, hunt_submit | 0.0000 | 0 | 0.001 | 0.001 | 0.241 |
| 2 | no | 50 | Verifying | Done:Paid | 102.4 | hunt_start, Read, Read, Read, Write, check_poc, hunt_submit, status, status, status | 0.0000 | 0 | 0.001 | 0 | 0.523 |
| 3 | no | 51 | Verifying | Done:Paid | 141.7 | hunt_start, Read, Read, Read, Read, Write, check_poc, hunt_submit, status, status, status, status, Write, Write | 0.0000 | 0 | 0.001 | 0 | 0.930 |
| 4 | yes | 52 | Done:Paid | Done:Paid | 90.7 | hunt_start, Read, Read, Read, Write, check_poc, hunt_submit | 0.0000 | 0 | 0.001 | 0.001 | 0.232 |
| 5 | no | 53 | Holder | Done:Paid | 84.9 | hunt_start, Read, Read, Read, Read, Write, check_poc, hunt_submit | 0.0000 | 0 | 0.001 | 0 | 0.256 |

These runs set `$NEAR_RPC_EXTRA` to a free dRPC endpoint as the failover RPC, as task a did. Every agent wrote the same correct proof (its own vault's `sweep`, which has no access control), checked it with `check_poc` before submitting, and got a T1 `Violation`. Runs 2 and 3 stopped on dRPC's free-plan timeout after their commit and reveal landed; run 5 ended `Holder`, its payout waiting behind those two in the queue. The services' keeper settled all three as paid.

Full transcripts: `logs/run-NN.jsonl` (Claude Code's stream-json, local paths replaced).

The prompt, the same for every run:

> You have one MCP server, `antibody`, and file tools that work in this folder. Antibody pays for verified proofs that a bounty program's own invariants break. This deployment has a practice program: a tutorial whose bug is public, so anyone can practise writing a proof. Write the proof yourself and submit it once: get an account named `{name}` and the instance it may prove with hunt_start (vending `{vending}`), read the program's rules and sources in the workspace it creates, write the proof bundle there, check it with check_poc, then submit it with hunt_submit. Report the verdict, the commit's final state and the payout, with the transaction links. Use only the antibody tools and the file tools.

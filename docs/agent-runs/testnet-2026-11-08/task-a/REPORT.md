# Measured agent runs: task a, testnet, 2026-11-08 09:04 UTC

An off-the-shelf agent (Claude Code, no built-in tools) with only Antibody's MCP server completes the practice program's loop with the tutorial proof (a public, textbook bug), through `practice_try`. This measures completing the loop: sealing, verification and payment. It is not bug finding. Success is read from near-antibody, not from the agent.

- Versions: antibody 0.5.0 (the MCP server); Claude Code 2.1.319 (Claude Code)
- Runs: 10; paid within the agent's own call: 7 (70%)
- Paid on near-antibody when read again (2026-11-08 09:38 UTC): 10 of 10 (a payout the agent's call did not wait for, or did not live to see, settled by the services' keeper)
- Wall time of a run paid within its call: median 98.2 s (min 68.8, max 174.4)
- Paid (all runs): 0.007 wNEAR within the calls, 0.01 wNEAR when read again; bonds lost: 0 wNEAR
- Agent cost (all runs, USD, as Claude Code reports it): 1.859; models: claude-opus-5-5

| Run | Paid in its call | Commit | State at its end | State read again | Seconds | Tool calls | Verification (NEAR) | Bonds lost (wNEAR) | Paid (wNEAR, read again) | Net (wNEAR, in its call) | Cost (USD) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | yes | 38 | Done:Paid | Done:Paid | 76.3 | practice_try, status | 0.0000 | 0 | 0.001 | 0.001 | 0.072 |
| 1 | yes | 39 | Done:Paid | Done:Paid | 70.6 | practice_try, status | 0.0000 | 0 | 0.001 | 0.001 | 0.071 |
| 2 | no | 40 | Holder | Done:Paid | 72.2 | practice_try, status | 0.0000 | 0 | 0.001 | 0 | 0.071 |
| 3 | yes | 41 | Done:Paid | Done:Paid | 174.4 | practice_try, status, practice_try, status, list_programs, status, practice_try, get_program, get_program, get_program, status, status, status | 0.0000 | 0 | 0.001 | 0.001 | 0.928 |
| 4 | no | 42 | Verifying | Done:Paid | 50.2 | practice_try, status, list_programs, status | 0.0000 | 0 | 0.001 | 0 | 0.090 |
| 5 | yes | 43 | Done:Paid | Done:Paid | 112.9 | practice_try, status | 0.0000 | 0 | 0.001 | 0.001 | 0.071 |
| 6 | yes | 44 | Done:Paid | Done:Paid | 115.1 | practice_try, status | 0.0000 | 0 | 0.001 | 0.001 | 0.071 |
| 7 | no | 45 | Verifying | Done:Paid | 43.6 | practice_try, status | 0.0000 | 0 | 0.001 | 0 | 0.055 |
| 8 | yes | 46 | Done:Paid | Done:Paid | 98.2 | practice_try, status, list_programs, get_program, status, status, practice_try, status, status | 0.0000 | 0 | 0.001 | 0.001 | 0.359 |
| 9 | yes | 47 | Done:Paid | Done:Paid | 68.8 | practice_try, status | 0.0000 | 0 | 0.001 | 0.001 | 0.071 |

These runs set `$NEAR_RPC_EXTRA` to a free dRPC endpoint as the failover RPC. The release binary (v0.5.0) also sends transactions through the failover, and dRPC's free plan timed out two of them ("Request timeout on the free plan"): runs 4 and 7 stopped before they settled, and the services' keeper settled both commits as paid. Since 2026-11-08 main sends transactions on the primary RPC only. Run 2 ended `Holder`: its payout waited in the program's queue (claims go in commit order) and the keeper paid it.

Full transcripts: `logs/run-NN.jsonl` (Claude Code's stream-json, local paths replaced).

The prompt, the same for every run:

> You have one MCP server, `antibody`. Antibody pays for verified proofs that a bounty program's own invariants break. This deployment has a practice program: a tutorial whose bug and proof are public, so anyone can practise the loop. Complete the loop once with the tutorial proof: get a practice account named `{name}`, have the proof sealed, committed, verified and settled, then report the verdict, the commit's final state and the payout, with the transaction links. Use only the antibody tools.

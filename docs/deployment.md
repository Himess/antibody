# Deployment (NEAR testnet)

Every contract Antibody runs on NEAR testnet is **locked**: it holds no full-access key, so nobody, its
deployer included, can change its code. `antibody judge` reads each one's code hash and locked status live.

| | Account / contract | Status |
|---|---|---|
| near-antibody | `bounty.antibody.testnet` | locked (code `C9LGvUpq…`, [CkenQk4B…](https://testnet.nearblocks.io/txns/CkenQk4BfJi9uYW72siKVqTRAAYUXxAXKErtNvQWKqDb); the staging run ran on `HMmyk9i7…`, the same code but one view) |
| near-guardian | `guardian.antibody.testnet` | locked (code `4SdG5PK9…`) |
| Practice vending | `practice.antibody.testnet` | locked (code `AdXwpxi5…`; its public vend key stays); practice program 14 since 2026-11-09 01:30 UTC (programs 6, 10 and 13 before it; a timer opens a fresh one every two hours when the next vault is proved or 8 or fewer are fresh: each vault pays once per program, [runbook](runbooks/worker-images.md)); the recorded run's were practice program 6 and its fork-mode twin program 7 |
| Verdict guard | `guard.antibody.testnet` | locked (code `7Wotb7nn…`): a DAO treasury the Acme project owns, watching program 5. It froze on A's T1-confirmed Critical (commit 13), not on D's duplicate (commit 16), and refused the next transfer ([JUDGES.md](../JUDGES.md), row 15) |
| near-t2 | `zk.antibody.testnet` | locked (code `3HFJXFtL…`): checks zero-knowledge proofs of disclosed verdicts against near-antibody's records ([zk.md](zk.md)) |
| Payout token | `wrap.testnet` (wNEAR: testnet NEAR, with no value) | existing |
| OutLayer, light client, MPC | `outlayer.testnet`, `client-eth2.sepolia.testnet`, `v1.signer-prod.testnet` | existing |
| Acme Lend, adapters | Sepolia and Base Sepolia, at the same CREATE2 addresses | deployed (program 5; addresses in the results file) |

`antibody admin record-deployment` pins each contract's code hash, its locked status, the verifier build
and the source commit into [`deployments/near-testnet.json`](../deployments/near-testnet.json) and
`deployments/history/`. Every transaction of the recorded run is linked from [JUDGES.md](../JUDGES.md).
Payouts, escrows and bonds on testnet are in wNEAR, a hundred-thousandth of the sandbox's tUSD amounts.

The whole story ran on the locked deployment with the final code
([results/testnet-2026-11-04.json](results/testnet-2026-11-04.json), all checks passed), with Acme Lend on
Sepolia and Base Sepolia and the real services: OutLayer's TEE workers, their TDX quotes verified on NEAR
(T1), the MPC network signing the permits, the Ethereum light client. A staging run on the same contracts
before the lock is kept in [results/testnet-staging-2026-11-03.json](results/testnet-staging-2026-11-03.json).

What a mainnet program would change is in [mainnet.md](mainnet.md), with a live read-only check of NEAR
mainnet's Ethereum light client (`antibody mainnet-check`: view calls only).

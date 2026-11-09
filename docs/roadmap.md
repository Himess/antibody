# Roadmap

Where Antibody goes after the hackathon. Items marked **done** or **prototype** exist in this repository today;
everything else is not built yet.

1. **Antibody for pull requests** (a GitHub Action, then a GitHub App), on repositories whose owner
   installs it:
   - *Immune memory:* every paid proof of a program re-runs against each pull request's build, and the
     check goes red if one still breaks a rule.
   - **Working on testnet** ([immune.md](immune.md#re-runs-in-the-enclave)): every paid proof, sealed or not,
     re-runs inside the enclave through near-antibody's rounds (the `immune-enclave` Action), on the pull
     requests of [the demo repository](https://github.com/Himess/antibody-immune-demo).
   - **On the runner** ([immune.md](immune.md)): readable proofs (disclosed, published, or opened with the
     project key), re-run by the local verifier, with `antibody immune` and a GitHub Action.
   - Next: a check-only proposal mode, so a pull request's check never becomes the program's next round.
   - **Prototype, not yet run on testnet** ([immune.md](immune.md#bounty-windows)): *a bounty window per pull
     request*, a fork-mode program for the pull request's build with a small escrow, announced over MCP, a
     webhook and the site. When it ends, the check reads "Held N h, X wNEAR at stake, 0 valid findings", or
     blocks the merge on valid findings.

   Sealed proofs never appear in public logs.
2. NEAR contracts as a second target VM: proofs against NEAR wasm contracts from committed, proven state. The
   engine and the v2 binary are built and tested; nothing is deployed ([near-targets.md](near-targets.md)).
3. T2: validity proofs of disclosed verdicts, verified on NEAR; later zk by default above a cap.
   - **Done:** the feasibility study ([zk-feasibility.md](zk-feasibility.md)). The verifier runs in SP1
     6.10.0 and reproduces the TEE's receipts in about 30 million cycles.
   - **Done** ([zk.md](zk.md)): near-t2, which checks a Groth16 proof in about 34 Tgas and compares it with
     near-antibody's records; `antibody zk prove|submit|status`; the API; a prover service for every
     practice try.
   - **Done** (2026-11-07): the recorded run's four runs proven on SP1's prover network (0.42 PROVE and
     45-79 s each) and near-t2 on testnet (`zk.antibody.testnet`, locked), all four records `confirmed`;
     since then the prover service proves practice tries while its PROVE budget lasts (21 runs `confirmed` by
     2026-11-09, when it paused at its floor).
   - Next: zk by default above a cap.
   - Next: a challenger in the keeper, so that no T0 Violation is paid unchallenged: a challenge demands
     T1, and with none the verdict is void and both bonds are refunded (spec §6.4). And a pause on two
     agreeing T0 runs from different workers while a new worker image waits its delay.
4. Multi-block proofs with a bounded time warp: interest, TWAP and time-dependent bugs.
5. The next Ethereum execution rules (an EVM upgrade and fork-switch execution).
6. L2 state verification through MPC foreign-chain verification or L2 light clients; more chains for pause and release.
7. Several independent TEE operators by default; a strict "no quote, no pay" mode once attestation is reliable.
8. One sub-account per program: isolated escrow, keys and signing path.
9. Payouts in any asset through NEAR Intents (mainnet).
10. Agent capital: backer vaults fund bonds for agents with verified records; bond discounts from proof-based reputation.
11. Security jobs on NEAR's agent market whose acceptance test is an Antibody verdict.
12. A public verdict registry as a NEAR primitive for insurance, lending markets and DAOs.
13. Continuous programs for live protocols in mainnet mode, with honest latency limits.
14. Private and invite-only programs; optional checks before large withdrawals; safe-harbor templates.
15. Post-quantum readiness: ML-DSA accounts and hybrid ML-KEM sealing for long embargoes.
16. The mainnet path: external audits, Antibody's own adapters under an Antibody program, coverage partners, the fee model in production.
17. A paid tier for the drafter, rule strength and analytics.

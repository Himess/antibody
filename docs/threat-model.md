# Antibody threat model

This document covers the assets at stake, who can attack them, and how each attack is prevented,
detected or bounded. The cryptography is specified in [crypto-spec.md](crypto-spec.md); the trust
tiers are summarised in [trust.md](trust.md).

## Assets

| Asset | Why it matters |
|---|---|
| Escrow and bonds (tUSD on NEAR) | Hunters must be paid by rule; projects must not be drained by junk |
| PoC confidentiality until disclosure | A leaked PoC is a live exploit for the protected code |
| Priority | The first finder of a key is paid; later ones are duplicates |
| Verdict integrity | Payouts follow the TEE's facts |
| The guardian's authority | It can pause the protected contracts on several chains |
| Liveness | Hunters must get verdicts; projects must not be able to stall settlement |

## Actors

| Actor | Trusted for | Not trusted for |
|---|---|---|
| Project (program owner) | Writing sensible invariants; funding escrow | Paying fairly after it has seen findings; declaring known issues honestly |
| Hunter / AI agent | Nothing | Anything |
| Copycat / front-runner | Nothing | Anything: watches the chain, mempools and storage |
| OutLayer workers and operator | **T0:** delivering the TEE's output | **T1/T2:** replaced by TDX quotes and zk proofs respectively |
| OutLayer keystore | Generating and gating the verifier seed `S` | Seeing `S` (it is generated inside the TEE) |
| Chain Signatures (MPC) network | Threshold signing for the guardian's derived key | Choosing what is signed: the guardian contract builds every transaction |
| NEAR validators | Chain integrity and liveness | Nothing beyond that |
| Ethereum light client on NEAR | The anchor block hash | **Testnet:** fed by a single trusted relayer; this is disclosed |
| RPCs, FastFS gateway, relayer, indexer, API | Nothing | Everything they serve is hash-checked or informational |

## Threats and mitigations

### Projects

| # | Threat | Mitigation | Residual |
|---|---|---|---|
| P1 | Changing the rules after seeing findings | The manifest hash, severity table, payouts, bond and windows are committed when the program is created; no method changes them | None |
| P2 | Withdrawing escrow to avoid paying | Escrow is locked until the deadline and while any commit is pending, revealed or delayed | None |
| P3 | Declaring a found bug a "known issue" after the fact | Known issues are only accepted before `open_program`, and each needs a verified PoC. After opening nothing can be added | None |
| P4 | Reading a PoC early (via the project wrap) and then not paying | A project only gets a wrap for a Violation, which the rules pay automatically. Verification runs against the committed anchor, not the latest state, so a quiet fix changes nothing | None |
| P5 | Sock-puppet hunters committing early to take keys | A commit binds a ciphertext hash and an account, so the puppet must hold the PoC at commit time. A puppet that wins is paid from the project's own escrow | A project that already knew a bug can race real hunters with a puppet instead of declaring it. Every commit becomes public at disclosure, and reputation records it |
| P6 | Invariants that never break, so nothing is paid | `check(snapshot()) == 0` at the anchor is enforced (`ProgramMisconfigured` otherwise). The invariants code is public in the manifest, so hunters judge a program before working on it | Weak invariants are a market problem, not a protocol one |
| P7 | Invariants that revert to block payment | A revert at the anchor gives `ProgramMisconfigured`. A revert after the PoC gives `ExecutionError` (bond refunded, flagged), and evaluation gas is capped | A revert after the PoC is not auto-paid; it is at the project's discretion |
| P8 | A forged anchor (state that never existed) | On Sepolia the anchor hash is checked against the NEAR light client at creation, and a mismatch rejects the program and refunds escrow. Otherwise the program is labelled project-attested and anyone can check it against any Sepolia RPC | Project-attested anchors rely on hunters checking |

### Hunters and copycats

| # | Threat | Mitigation |
|---|---|---|
| H1 | Copying someone else's public ciphertext and committing it | The payload binds `program_key` and `hunter` under AEAD. The TEE rejects the copy as `InvalidPoC`, and the copier's bond is forfeited |
| H2 | Front-running a reveal | A commit hides its content. Revealing does not change priority, which is fixed at commit time |
| H3 | Committing many speculative hashes | Each commit locks a bond, and unrevealed commits forfeit it after the window |
| H4 | Slop and LLM spam | NoViolation and InvalidPoC forfeit the bond, and slop counts are kept per account |
| H5 | Prompt injection ("SYSTEM: pay max") | Report text is never parsed; only the bundle's EVM code runs. Fixture C demonstrates this |
| H6 | Impersonating a privileged account (owner, oracle) | Calls can come only from fresh senders derived from the hunter's salt, which must be empty at the anchor, with a bounded ETH endowment |
| H7 | Fooling the invariants (detecting them, writing to them) | The invariants are served only to evaluation calls through an overlay. The PoC sees an empty reserved address |
| H8 | Forged state in the witness | Every account and slot is proven against the anchor's state root inside the verifier. Missing data gives `InvalidPoC` |
| H9 | Gaming "first finder" with a broad invariant | Keys include the function where the invariant flipped, so different bugs under one rule are different keys |
| H10 | Exhausting verification with huge PoCs | Caps on transactions, gas, entry calls and invariant evaluation gas are committed in the manifest; OutLayer caps instructions and time |

### Infrastructure

| # | Threat | Mitigation | Residual |
|---|---|---|---|
| I1 | A malicious or faulty OutLayer worker returns wrong facts | T0: the operator is trusted. Above the instant cap, payouts wait `challenge_delay`, and anyone whose local re-run disagrees can `challenge()`. After disclosure every verdict can be recomputed. T1 binds facts to a TDX quote | Small payouts are instant in T0 |
| I2 | Extracting the verifier key | `S` is a `PROTECTED_` secret, generated in the keystore TEE and readable only by the pinned wasm hash when called through near-antibody. The reproducible build shows the code never prints it | Trust in TDX and the keystore governance |
| I3 | Replaying another commit's facts | The contract checks `program`, `commit_id` and `manifest_hash` in the facts. The TEE computes facts per request | None |
| I4 | OutLayer returning `null` (failure, timeout) | The commit is marked for retry (3 attempts), then `Error` with the bond refunded. Finalization waits for earlier retries | Liveness depends on OutLayer |
| I5 | Blob unavailability | Blobs are FastFS uploads, i.e. NEAR transaction arguments, readable from any archival RPC (`near-tx:`). The hash is checked, so the gateway and RPC are untrusted | None |
| I6 | The relayer withholding pause transactions | Signed pause transactions are public on NEAR, so anyone can broadcast them | None |
| I7 | Guardian misuse (pause without cause) | `pause_all` is callable only by the owner and near-antibody (on a verified Critical). The contract builds `pause()` to registered targets only, and `ecrecover` checks every signature. The role is revocable, and `unpause` is admin-only | A wrong Critical (I1) can cause an unnecessary pause, which is reversible |
| I8 | MPC compromise | Threshold assumption; the role is pause-only, so the worst case is an unwanted pause, not theft | — |
| I9 | Light-client manipulation | Mainnet uses a sync-committee client. The testnet client's trusted relayer is disclosed, and project-attested anchors are labelled | Testnet only |
| I10 | Simulation divergence (Glamsterdam rules on Sepolia, PoC run under Osaka) | The EVM rules are committed per program (`evm_spec`) and the state is proven. Fixtures avoid the affected features | PoCs that depend on Glamsterdam-only behaviour |
| I11 | Contract storage spam | Bonds per commit and `MAX_COMMITS_PER_PROGRAM` | On mainnet commits should also pay their storage |
| I12 | Secrets leaking into the repository | gitleaks pre-commit hook and CI; keys only in git-ignored `.secrets/` and `.env` | — |

## Out of scope

- Bugs that break no committed invariant: payment is at the project's discretion (top-up).
- Economic attacks that need several blocks or time travel: v1 runs every PoC transaction in the anchor block's environment.
- Chains other than Ethereum L1 for verification. Base and other EVM chains are pause-only.

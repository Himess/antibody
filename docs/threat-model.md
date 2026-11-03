# Antibody threat model (v2)

Who can do what to an Antibody program, what stops them, and what trust is left. The design is in
[spec.md](spec.md). The properties that pin these mitigations, and the checks behind each one, are in
[properties.md](properties.md).

## Assets

| Asset | Why it matters |
|---|---|
| Escrow, bonds, challenge bonds, credits | Hunters must be paid by rule; projects must not be drained by junk or forgeries |
| PoC confidentiality until disclosure | A leaked PoC is a live exploit |
| Priority | The earliest valid commit holds a key; later ones are duplicates |
| Verdict integrity | Money moves on facts |
| Program keys (sealing, tags, disclosure) | They open every PoC of a program |
| The project's X25519 key | It opens what the project paid for |
| Pause and release authority | A permit pauses or upgrades the project's contracts |
| Liveness | Every commit must reach an end; nothing may stay locked |

## Actors and capabilities

"Can" is what the actor can do to an Antibody program. "Cannot" is what the design prevents, and "Check"
is how anyone can verify it.

| Actor | Can | Cannot | Residual trust | Check |
|---|---|---|---|---|
| **Deployer** (owner of near-antibody) | Set defaults for **new** programs (verifier build, OutLayer, TEE policy, fees); veto an OutLayer image, which only forces the window | Touch a live program's pins, rules or money (§15); read PoCs: the launch accounts are locked, and the program keys exist only for runs that near-antibody itself requests (§11.1); resolve challenges (no arbiter) | None on live programs | `antibody judge`: locked status (no full-access keys), per-program pins |
| **OutLayer operator** | Withhold, delay or cancel runs; withhold attestations; return arbitrary T0 facts | Make a T0 fact final if anyone challenges it within the window (§6.4); forge T1; take an honest bond: infrastructure failures are never forfeited (§5.3) | **T0 window:** an unchallenged lie in a T0 fact finalizes when its window passes. Liveness | `antibody tee check --request-id N`; the keeper requests T1 for every run |
| **OutLayer governance** (image approvals, keystore DAO) | Approve a worker image; govern the keystore master | Reach a live program with a new image before `image_delay`, which is longer than the challenge window (§15); read program keys without running near-antibody's build for near-antibody | A malicious image approved, delayed and unchallenged | The pinned measurement set per program; `get_approved_measurements` |
| **Physical attacker** (TDX key extraction) | Forge T1 quotes | Be paid before a window passes beyond the **instant budget per window**; be paid above the cap without an agreeing second run (§6.2–6.3) | Up to `instant_budget` per window | `audit()`; instant payments in the timeline |
| **NEAR MPC network** (threshold) | Sign permits under any project's path; derive CKD keys | Be asked for anything near-antibody's rules did not request | Threshold honesty. A rogue threshold could pause or upgrade (bounded by the adapter's code-hash rules) or derive keys | Permit events on NEAR; adapter events on each chain |
| **Light-client relayer and its DAO** | Stall updates (liveness); the client's ACL admins can upgrade it | Forge a header: updates carry BLS-verified sync-committee signatures | Liveness; the admins' upgrade power | `antibody anchor-check --block N` |
| **DCAP verifier admin, Intel PCS** | Upgrade the verifier contract; be unavailable; issue a TCB recovery | Turn an invalid quote into a valid one without Intel's keys | Intel's roots of trust; liveness (T1 falls back to the window) | `verify_quote` as a view call |
| **Project** | Write weak invariants; ship no fix round; challenge T0 payouts | Change rules after open; withdraw escrow before exit; add known issues after open or above the cap; read a duplicate's PoC (no wrap); veto with its own rules: reverts are `Unevaluable` under a committed policy, and `check()` cannot run code the PoC deployed; release code that still fails a Critical PoC | Weak rules are a market problem: rule strength (§19.3) shows them. Without a fix round, duplicates are never promoted | Manifest, rule strength, `get_keys`, the timeline |
| **Hunter** | Submit anything; commit and never reveal | Copy another's blob (AEAD binds the hunter); front-run (priority is the commit); be paid twice per key; steer a fingerprint (SA-1); close a program with a tampered witness (SA-2) | None | Self-attack regressions in CI |
| **Agent** (a hunter sub-account) | Spend its owner's credit within its bond budget | Withdraw, transfer, attach deposits, or bond above its caps; be steered by program text (MCP delimits it as data) | Its owner's budget | Agent statement: containment events |
| **Antibody backend** (keeper, relayers, indexer, API) | Stop running | Change any outcome: every action it takes is permissionless and re-derivable from NEAR | Liveness only: anyone can run the keeper | Run `antibody-services keeper` yourself |
| **RPCs and FastFS gateways** | Lag, lie, go down | Change a byte that is checked against its hash; forfeit a bond (unavailability is `InfraError`) | Availability | Two RPCs inside the guest; inline blobs |

## Threats and mitigations

### Projects

| # | Threat | Mitigation | Residual |
|---|---|---|---|
| P1 | Changing the rules after seeing findings | Manifest hash, payouts, windows and pins fixed at creation; rounds may only add fixes and invariants | None |
| P2 | Escaping payment | Escrow locked until exit; valid claims queue as debt in commit order, never cut; an unpaid debt at exit is recorded as defaulted | A project can underfund; its program shows the coverage and any default |
| P3 | Declaring a found bug "known" | Known issues: only before open, verified, sealed, capped by count and share of escrow | None |
| P4 | Reading PoCs without paying | `verify` emits no wrap; `release_wrap` serves paid first findings and promoted duplicates only; a re-run at the project's fix code publishes only which keys still break, never impact or harm numbers | A fix written to probe a sealed PoC learns at most which of the program's keys break, a few bits per re-run |
| P5 | A puppet hunter committing first | The puppet must hold the PoC at commit; it is paid from the project's own escrow; every commit is public at disclosure | A project that knew a bug can race with a puppet instead of declaring it |
| P6 | Rules that never break | `check()` must hold at the anchor; the invariants are public; rule strength is published | A market problem |
| P7 | Rules that revert to block payment | `Unevaluable` under a committed policy; the lint and the evaluation whitelist | With policy `refund`, a revert after the PoC pays nothing (stated in the manifest) |
| P8 | Forged anchor state | The light client checks the anchor hash; unconfirmed anchors are labelled and cannot pause | — |
| P9 | A round that weakens the rules | `check_round`: only target code changes and new slots; old slots byte-identical | None |
| P10 | A fake fix that releases keys | Keys are released only by a final re-run showing the PoC fixed, or by nobody re-running; promotions pay distinct bugs; the release gate needs every Critical or High PoC fixed | A project that never re-runs releases keys by default, which can only add payments |
| P11 | Shipping no fix round to deny duplicates | Without a fix, duplicates are judged by keys alone; their proof opens at disclosure | A project that ships no round pays no duplicates |
| P12 | Challenging every T0 payout to delay it | Each challenge costs a bond, lost to the hunter when T1 upholds the payout | Delay up to `resolve` per challenge when no quote can be had |
| P13 | Losing or leaking its X25519 key | `rotate_project_key` (future wraps); NEP-413 derivation lets it be re-derived; the message names near-antibody and the project and says to sign it nowhere but the CLI | A leaked key exposes past paid findings. With `--nep413`, a page that gets the project's wallet to sign that fixed message derives the key and reads every paid finding; a random key file (`keygen x25519` alone) has no such message |
| P14 | A "fix" that bricks the targets | Canaries must keep succeeding | Canaries cover what the project lists |

### Hunters and agents

| # | Threat | Mitigation |
|---|---|---|
| H1 | Copying a public ciphertext | AEAD binds `program_key` and `hunter`; the copy is `InvalidPoC` and forfeits |
| H2 | Front-running a reveal | Priority is the commit, which hides the content |
| H3 | Speculative commits | A bond per commit; unrevealed commits forfeit; requester-paid storage |
| H4 | Slop and spam | Forfeits to the program's escrow; bonds sized from measured cost (§12.2) |
| H5 | Prompt injection | Report text is never parsed; only EVM code runs (fixture C) |
| H6 | Impersonating a privileged account | Fresh salted senders, empty at the anchor, bounded endowment |
| H7 | Detecting or writing the invariants | Overlay at a per-program reserved address; the PoC sees it empty; evaluation whitelist |
| H8 | Forged state | Every account and slot proven against the anchor's state root; trie node kinds validated; code re-analysed from its hashed bytes |
| H9 | Steering a fingerprint to a new key | Reverted frames truncate the evaluation timeline (SA-1) |
| H10 | Exhausting verification | Budgets enforced inside the EVM; size and depth bounds outside |
| H11 | Closing a program with a tampered witness | Canonical code (SA-2); `ProgramMisconfigured` is final only with T1 or after the window |
| H12 | Re-finding a known Critical to force a pause | Tickets only for T1 holders of new keys in the pause policy, one per round |
| H13 | Burning an earlier commit's retries during an outage | Retries are paid by the requester, and infrastructure failures keep priority until `infra_grace` |
| H14 | Pre-funding the reserved address | The address is derived per program from the anchor hash, and a balance is tolerated |
| H15 | A hostile program draining an agent's bond | Per-commit cap, and a bond budget that counts bonds lost as well as locked; an agent may be bound to a list of programs (SA-13) |
| H16 | Many entry points into one bug | A per-invariant budget per round; custom groups |

### Infrastructure

| # | Threat | Mitigation | Residual |
|---|---|---|---|
| I1 | Wrong T0 facts | Provisional until T1 or the window; any challenge demands T1; no T1 voids | An unchallenged lie finalizes at its window |
| I2 | Forged T1 (physical access) | Instant budget per window; above the cap, window plus an agreeing second run | `instant_budget` per window |
| I3 | Reading PoCs through the keys | Keys exist only for runs of the pinned build called by near-antibody, whose accounts are locked | The keystore TEE and its governance |
| I4 | Replaying facts | Facts bind the request hash, program, round, commit and run id | None |
| I4b | A quote bound to another run | T1 checks the quote's task fields that name this run: the request hash, the output hash, the pinned wasm hash and build target, the caller, no secrets, a `WasmUrl` source | OutLayer's task hash concatenates its fields without length prefixes, and its metadata (task id, block height, timestamp, attached USD) is not checked: a collision would need the TEE to attest a run whose metadata embeds this run's hashes |
| I5 | Runs that never answer | `time_out`, void, retry with priority, then refund | Liveness depends on OutLayer |
| I6 | Blob or witness unavailability | Inline blobs; the guest tries each archival RPC until one serves bytes matching the hash; unavailability is `InfraError`, except a hunter's own location still unreadable after the grace, which forfeits | All the guest's RPCs down for a whole grace |
| I7 | Nobody relays a permit | Antibody's relayer by default; relaying is permissionless | A permit needs a funded sender per chain |
| I8 | A pause without cause | Tickets: T1, policy slots, minimum impact, one per round; the adapter only calls `pause()` on targets fixed at its deployment; no unpause power | MPC threshold |
| I9 | A stale or censored light client | BLS-verified updates; `recheck_anchor`; unconfirmed anchors labelled | Liveness |
| I10 | EVM rule drift (Sepolia's Glamsterdam) | Rules pinned per program; every receipt carries its label | PoCs depending on newer rules are judged under the pinned ones |
| I11 | Storage spam | Requester-paid storage: every call that adds storage charges its caller or its run's payer; each commit prepays its settlement's bytes; image syncs come only from named registries, paid by their caller | — |
| I12 | Locked contracts with a bug | Exit after deadline plus grace; new versions serve new programs | A program's own run can still be wrong until its exit |
| I13 | Secrets in the repository | gitleaks hook and CI; never-public path checks | — |

## Out of scope

- Bugs that break no committed invariant and harm no protected account.
- Attacks spanning several blocks or needing time to pass: every PoC runs in the anchor block's environment.
  Multi-block PoCs are on the roadmap.
- Verification on chains other than Ethereum L1. Other EVM chains receive permits only.

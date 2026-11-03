# Antibody specification (v2)

**Pay on proof. Pause on proof. Ship only what survived.**

This is the normative description of Antibody v2: what each component does, in which order, and why.
`contracts/near-antibody` (settlement), `crates/rules` (the pure rule core), `crates/verifier` (the
deterministic judge), `contracts/near-guardian` and `contracts/evm/src/antibody` (pause and release
permits) implement it. `formal/antibody.qnt` models §4–§14. `crates/rules` is checked against it by replaying the model's
traces step by step, and `docs/properties.md` lists every property and the checks behind it. Where this document and older documents disagree, this document
wins.

"‖" is byte concatenation. Amounts are in the program's token (tUSD on testnet, 6 decimals). Times are
NEAR block timestamps in seconds.

---

## 1. What Antibody is

A project runs a bug bounty whose rules are code:
- invariants, which state what must never happen;
- severities, payouts and windows;
- the state the bounty is judged on: an Ethereum block that NEAR's light client has verified.

A hunter, human or AI agent, commits to a sealed proof of concept (PoC). A pinned, deterministic verifier
runs it inside a TEE on Merkle-proven state. NEAR verifies the TEE's quote of that run, applies the
committed rules and pays. A verified Critical pauses the project on every chain it registered, through
an EIP-712 permit that NEAR's Chain Signatures sign under the project's own key. Every paid PoC is re-run
against the project's fix (immune memory), and a release permit for the fixed code is signed only when
every one of them fails.

**Nobody reads a report.** The verifier never sees prose. A submission is code plus state proofs, and its
verdict is a function of committed inputs alone.

## 2. Actors

| Actor | Does | Detail |
|---|---|---|
| Project | Creates and funds a program, commits its rules, proposes fix rounds, tops up | §3, §8 |
| Hunter | Commits, reveals, withdraws credits | §4 |
| Agent | A hunter sub-account created by an owner, with a bond budget and a key that cannot withdraw | §13 |
| Anyone (keeper) | Advances every state: relays quotes, allocates, pays, times out, opens rounds, discloses, exits | §14 |
| OutLayer | Runs the verifier (T0 facts); its TDX workers produce quotes | §5, §6 |
| NEAR DCAP verifier | Verifies quotes on NEAR (T1) | §6 |
| NEAR MPC (Chain Signatures) | Signs permits under each project's key; derives confidential keys (CKD) | §10, §11 |
| NEAR Ethereum light client | Certifies the anchor block hash | §17 |
| Permit adapter (one per project per EVM chain) | Checks a permit and pauses or upgrades the project's contracts | §10 |

`docs/threat-model.md` gives every actor's capabilities and the residual trust in each.

## 3. Programs

### 3.1 Lifecycle

```
create ──(anchor check)──▶ Setup ──activate (T1)──▶ Activated ──known issues (final)──▶ Open
                                                                                        │
     Exit ◀── deadline + exit grace (anything stuck is settled) ◀── rounds 0, 1, 2 … ◀──┘
```

1. **Create.** The project transfers the escrow with `ft_transfer_call` and sends manifest v2 (§3.2). The
   contract validates bounds (§3.3), pins every dependency (§15), stores the essentials and asks the light
   client for the anchor's block hash. **The program can be used only when that check completes.**
   - If the light client confirms the hash, the anchor is `LightClient`.
   - If it contradicts the hash, the program is rejected and the escrow is refunded.
   - If the call fails, or the chain or the block is outside the client's window, the anchor is
     `Unconfirmed`. It is labelled so and cannot trigger pauses.
   - `recheck_anchor` is permissionless while the block is inside the client's window.
2. **Activate.** `activate(program_id, base)` pins the base witness (§4.4). One verifier run in
   `activate` mode checks the program at the anchor, derives the program's keys (§11) and outputs the
   program's sealing public key and key epoch. The key is accepted only when that run is **T1-confirmed**
   (§6). Activation retries until it is; each retry costs one run.
3. **Known issues** (optional, before open only). §7.6.
4. **Open.** The project calls `open_program` once activation is T1 and no known-issue run is in flight.
   Round 0 starts. From now on the owner of near-antibody cannot change anything this program depends on
   (§15).
5. **Rounds.** §8.
6. **Deadline and exit.** No commit is accepted after `deadline`. After `deadline + exit_grace`, `exit`
   (permissionless) settles everything (§14.3).

A verified `ProgramMisconfigured` (§5.3) closes the program early. Every bond is refunded, and `exit`
becomes available at once.

### 3.2 Manifest v2

The manifest's exact bytes are committed: their SHA-256 is the program's `manifest_hash`, and every reader
hashes the bytes it receives, never a re-encoding. The CLI writes them in one canonical form (compact
JSON, fields in a fixed order, the `settlement` object's keys sorted). Schema: `antibody.manifest.v2`.
near-antibody reads the identity, the anchor, the slots and `settlement`. The verifier reads the rest; it
receives the round-0 manifest inside the base witness (§4.4) and a later round's manifest by hash. A v2
manifest has a `settlement` section and none of v1's `payout` and `rules` sections, so the terms have one
source.

| Field | Meaning |
|---|---|
| `program` | `name`, `project` (NEAR account), `description` |
| `anchor` | `chain_id`, `block_number`, `block_hash`, `evm_spec` (the EVM rules, §18), `trust` |
| `targets[]` | `name`, `address`, `code_hash` |
| `invariants` | `runtime_code`, `runtime_code_hash`, `gas_limit` (per evaluation), `slots[]` {`bit`, `name`, `severity`}; `address` is ignored: the verifier serves the invariants at a reserved address per program (§5.7) |
| `impact` | `protocol_accounts[]`, `native_usd`, `tokens[]` {`symbol`, `address`, `decimals`, `usd`}: committed prices (§5.6) |
| `capabilities` | `max_senders`, `max_endowment_wei`, `max_txs`, `max_gas_per_tx`, `max_total_gas`, `max_blocks`, `max_timestamp_delta`, `max_entry_calls`, `max_eval_gas` |
| `groups[]` | Optional `{id, name, members[] {target, selector}}`; absent, every entry point is its own group (§7.2) |
| `protected_accounts[]` | Optional `{address, kind}`, `kind` being `user` or `protocol` (§7.7) |
| `canaries[]` | Optional `{to, data}`: calls that must keep succeeding after a fix (§8.1) |
| `round`, `overrides[]` | A round's manifest only (§8.1) |
| `deploy` | Optional fork mode: `txs[]` {`to`?, `value`, `data`} from the deployer, run on top of the anchor before anything else; the targets are what they deploy (docs/fork-mode.md) |
| `rule_strength` | Optional: the rule strength measured at this round's code, as `antibody invariants strength` reported it; the verifier does not read it (docs/rule-strength.md) |
| `settlement` | Every term near-antibody enforces (below) |

`settlement` (amounts are decimal strings in raw token units; windows are seconds):

| Field | Meaning |
|---|---|
| `payouts` | `critical`, `high`, `medium`, `low`, `impact_bps`, `impact_cap`, `decimals` |
| `instant` | `cap`, `budget`, `window_secs` (§6.3) |
| `fees` | `protocol_bps`, `release` (§12.3) |
| `bond`, `challenge_bond` | §12.2, §6.4 |
| `windows` | `reveal`, `challenge`, `resolve`, `infra_grace`, `run_timeout`, `round_min`, `rerun_window`, `disclosure_max_delay`, `deadline` (unix seconds), `exit_grace` |
| `slot_budgets[]` | `{slot, budget}` per round (§7.4); a slot not listed has the escrow as its budget |
| `unevaluable` | `"refund"` or `{"violation": severity}` (§5.4) |
| `gap` | Optional `threshold_usd_micro`, `claim_amount` (§7.7) |
| `known` | `max_count`, `max_share_bps` (§7.6) |
| `pause` | `slots[]`, `min_impact_usd_micro` (§10) |
| `terms_sha256` | The terms text (§16) |

The base witness is pinned at activation, and the pins (§15) come from the deployment's defaults at
creation: neither is part of the manifest.

### 3.3 Bounds

`create_program` refuses a manifest unless all of the following hold.
- Every window is within its range (`docs/api.md`).
- `max_eval_gas ≥ (max_entry_calls + 2) × invariants.gas_limit`, and the final evaluation runs with its
  own reserved `gas_limit`, outside the total.
- Every amount is at most 2^100, so the payout arithmetic cannot overflow `u128`. A payout the escrow
  does not cover is owed through the shortfall queue (§7.8).
- Every string has a length cap.
- At most 32 targets, 64 slots, 64 groups and 8 canaries.
- The bond and the challenge bond are positive (§12.2 gives the recommended floor).
- The project's X25519 key is not a low-order point (§11.5).

`antibody invariants validate` runs the same checks off chain, plus a dry run of `check()` at the
anchor (§5.4).

## 4. Submissions

### 4.1 Commit

`commit(program_id, round, commitment, terms_sha256)` with a bond (`ft_transfer_call`, or from the
account's credit).
- `commitment = SHA-256("antibody/commit/v2" ‖ u32_be(len) ‖ program_key ‖ u32_be(round) ‖ u32_be(len) ‖
  hunter ‖ blob_sha256)`, with the program key and the hunter length-prefixed. It binds the hunter, so a
  commitment cannot be copied.
- `round` must be the program's current round: **commits are bound to their round**.
- `terms_sha256` must equal the program's. Committing accepts the terms (§16).
- Storage is paid from the beneficiary's NEP-145 credit (§12.1).
- An agent's commit is checked against its bond budget and per-commit cap (§13).
- An agent's bond comes from its owner's credit; anyone else's comes from their credit or arrives with
  `ft_transfer_call`.

Commit order (a per-program index) is priority.

### 4.2 Reveal

`reveal(commit_id, blob)` within `reveal` seconds of the commit.
- `blob` is `{"inline": hex}` (at most 15 KB, carried in the call itself), or
  `{"near_tx": {"location": "near-tx:<hash>:<signer>", "sha256"}}` for a larger blob uploaded with FastFS.
  Every request near-antibody sends stays within OutLayer's 32 KB of free input, so a run never needs an
  extra deposit.
- The commitment is checked against `SHA-256(blob)` for inline blobs and against the given `sha256` for
  FastFS blobs, whose fetched bytes the verifier checks. A FastFS location is kept for later runs; an
  inline blob is not stored, and a later run of the commit passes it again.
- The reveal starts a verification run (§5) paid from the beneficiary's credit.

A commit not revealed in time **expires** (`expire`, permissionless), and its bond is forfeited to the
program's escrow.

### 4.3 The sealed blob

`ABSEAL2` (§11.3): the PoC bundle and its extra witness, encrypted to the program's sealing key. Only the
verifier can open it, until disclosure.

### 4.4 Witnesses

- **Base witness.** The anchor header, the invariants and proofs for every account, slot and code the
  activation checks read (`antibody program activate` builds it by tracing them), plus the anchor's fee
  recipient, which every transaction of a PoC credits. Published by the project with FastFS and pinned
  at activation. Re-runs and duplicates all use it.
- **Extra witness.** Proofs for any other account or slot the PoC touches. It travels inside the sealed
  blob, so disclosure releases everything a re-run needs. A sealed blob may also carry a complete program
  witness; the verifier merges an extra witness with the base and uses a complete one as it is.
- **Senders.** A sender's address is a hash of the bundle's salt, so no salt reaches an account that
  exists (that would take a keccak preimage). The verifier sets each sender's account (the endowment, no
  code, nonce 0) and never reads it at the anchor, so no witness carries senders.
- **Proofs that read only base state.** When a PoC reads nothing the base does not prove, its extra
  witness proves nothing (the state root's digest alone) and the sealed blob is little more than the
  bundle. The CLI tries this first: it fetches the pinned base and checks the PoC on it locally, so such
  a proof needs no Ethereum RPC at all, however old the anchor. Otherwise it builds the extra witness from
  an RPC at the anchor, which for an old anchor must keep historical state.

The verifier fetches `near-tx` blobs and the base witness from at least two NEAR RPCs and accepts only
bytes that match the committed hash.

## 5. Verification

### 5.1 Request

near-antibody calls OutLayer's `request_execution` with:
- `source: WasmUrl {url, hash, build_target}`: the program's pinned verifier build;
- `payer_account_id`: the requester, so unused deposit returns to them;
- `input_data`: the request below, built from near-antibody's own records.

```
{ "v": 2, "mode": "activate" | "verify" | "second" | "rerun" | "check_round" | "distinct" | "release_wrap" | "disclose",
  "program_key", "manifest_sha256", "round", "key_epoch", "commit_id", "run_id",
  "base": {"near_tx", "sha256"},                      // the pinned base witness
  "round_manifest": {"near_tx", "sha256"},            // rounds after 0
  "hunter", "blob": {"inline"} | {"near_tx", "sha256"}, // submissions
  "project_pk", "earlier", "patch", "slot", "tags" }   // release_wrap, distinct, disclose
```

`run_id` is the id the run will get, so the facts can name it. `request_sha256 = SHA-256(input_data)`
is stored with the run; the facts repeat it, so T0 and T1 bind to the same bytes. Each request emits a
`run_requested` event, from which relayers find the OutLayer request behind a run.

### 5.2 Facts v2

At most 1 KB of canonical JSON. near-antibody hashes the bytes it receives (`output_sha256`), and T1 checks
that hash against the quote.

| Field | Meaning |
|---|---|
| `v`, `mode` | 2, and the request's mode |
| `request_sha256`, `manifest_sha256`, `program_key`, `round`, `key_epoch`, `commit_id`, `run_id` | Bind the facts to one request |
| `kind` | `Violation`, `NoViolation`, `InvalidPoC`, `Unevaluable`, `ProgramMisconfigured`, `InfraError` |
| `keys[]` | `{slot, tag}`: the root-cause keys (§7.2); the tag hides which group it names |
| `impact_usd_micro` | min(protocol loss, attacker gain) at committed prices (§5.6) |
| `protected_harm_usd_micro` | Loss on declared protected accounts (§7.7) |
| `receipt_sha256` | Hash of the full receipt, which is deterministic and byte-identical on every platform |
| `poc_sha256` | Hash of the decrypted bundle |
| `reason` | For `InvalidPoC`, `InfraError`, `ProgramMisconfigured`: a short code |

Mode-specific fields: `activate` adds `sealing_pk`; `check_round` adds `round_check {ok, canaries_ok}`;
`verify` and `second` may add `wrap_d`; `release_wrap` adds `wrap`; `disclose` adds `disclosed[]`;
`distinct` adds `distinct`. Facts that do not parse, or that bind another request, are the run's fault:
near-antibody records them as `InfraError`.

### 5.3 Outcomes

| Kind | When | Effect (once final, §6) |
|---|---|---|
| `Violation` | An invariant that held at the anchor fails after the PoC | Allocation (§7) |
| `NoViolation` | The PoC ran and nothing broke | Bond forfeited to escrow, unless the gap lane applies (§7.7) |
| `InvalidPoC` | A failure the hunter can compute locally: wrong commitment or hunter, a blob that matches its hash but does not open or decode, a malformed bundle, a budget overrun | Bond forfeited to escrow |
| `Unevaluable` | `check()` reverts or runs out of gas after the PoC | The manifest's `unevaluable` policy (§5.4) |
| `ProgramMisconfigured` | The program's own inputs are broken: invariants fail at the anchor, the reserved address holds code or storage, a target's code hash differs | Bond refunded; the program closes |
| `InfraError` | A failure that is not the hunter's: a blob or witness no RPC served, an RPC whose bytes do not match their hash, an unexpected verifier error | Retried with priority kept, until `infra_grace` passes; then void and refunded. One exception: a blob location the hunter named that no RPC could read (or whose bytes never matched) for the whole grace forfeits the bond, so a never-broadcast upload cannot hold the cursor for free |

### 5.4 `check()`

- A static call with the committed `gas_limit`, run at the anchor (it must succeed, or the program is
  misconfigured) and after every entry call of the PoC.
- A revert or an out-of-gas after the PoC is `Unevaluable`, handled by the committed policy:
  - `refund` (default): bond refunded, nothing paid, recorded;
  - `violation`: paid at the committed severity under one evaluability key per round: the first such
    proof holds it, later ones are its duplicates, and above the instant cap it needs an agreeing second
    run like any claim.
- **Lint at creation:** the invariants' code may not use environment-dependent opcodes (`BLOCKHASH`,
  `COINBASE`, `PREVRANDAO`, `GASPRICE`, `GAS`, `BALANCE` of non-target accounts, `SELFBALANCE`, `CREATE`,
  `CREATE2`, `CALL`/`DELEGATECALL` outside the targets). Reads outside the targets are rejected.
- **Evaluation whitelist:** code deployed by the PoC never runs during an evaluation. A call into it from
  `check()` fails the evaluation (`Unevaluable`), so a PoC cannot make a rule judge itself.

### 5.5 Budgets

Enforced inside the EVM, so the outcome never depends on the platform or the worker:
- **Per evaluation:** each `check()` gets exactly `invariants.gas_limit`.
- **Total PoC budget:** `max_total_gas` across the PoC's calls, and `max_eval_gas` across evaluations,
  apart from the final evaluation. Going over either is `InvalidPoC`, because the hunter can compute it
  locally.
- Work outside the EVM (decoding, proof checks) is bounded by size and depth limits.
- OutLayer's instruction limit is set to its cap, so the committed budget decides, never fuel.

### 5.6 Impact

`impact_usd_micro = min(protocol loss, attacker gain)`, both valued at the manifest's committed prices
over the targets' and the senders' balances.

### 5.7 Determinism and canonical inputs

The verifier is a total function of its committed inputs. Every input gives a typed outcome, and native
and wasm builds give byte-identical output.
- **Code** is re-analysed from its hashed bytes. A witness's jump table is never trusted (SA-2).
- **Trie node kinds** are validated against their encoding.
- **The manifest** is the request's own, checked against its hash, never the witness's.
- **The reserved invariants address** is derived per program,
  `keccak256("antibody/invariants/v2" ‖ program_key ‖ anchor_hash)[12..]`. Any balance there is tolerated.
- **Steering:** evaluations recorded inside a reverted frame are discarded (SA-1).

### 5.8 Modes

| Mode | Purpose |
|---|---|
| `activate` | Derive the program's keys; output the sealing public key (§11) |
| `verify` | Judge a submission |
| `second` | The same request on an independent run, required above the instant cap (§6.2) |
| `rerun` | A paid PoC, duplicate or gap claim at a newer round's code and invariants (§8) |
| `distinct` | A distinctness top-up (§7.5) |
| `release_wrap` | Wrap a paid finding's content key to the project's key (§11.4) |
| `disclose` | Release disclosure keys (§11.6) |

## 6. Tiers and the settlement gate

### 6.1 Tiers

- **T0.** OutLayer returned the facts. The OutLayer operator vouches for them.
- **T1.** Someone relayed the TDX quote of that run and NEAR's DCAP verifier accepted it:
  - OutLayer's attestation of the run describes exactly what near-antibody recorded: the pinned wasm,
    `input_hash = request_sha256`, `output_hash = output_sha256` (the bytes near-antibody received), the
    run's signer (or near-antibody) as caller, and no secrets;
  - the quote's `report_data` is the SHA-256 of those attestation fields, followed by zeros;
  - its measurements are in the program's pinned image set (§15), and its TCB status is accepted.

  `confirm_tee` is permissionless.

### 6.2 Finality

Every state-changing fact is **provisional until it is final**: payouts, forfeits, duplicates, known
matches, gap claims, `ProgramMisconfigured`.

**Structural facts need T1.** Activation keys (§11.2), known issues (§7.6), round checks (§8.1), re-run
results (§8.2), disclosure keys (§11.6) and pause tickets (§10.1) count only when their run is
T1-confirmed. A T0 fact only ever settles a submission, and only after its window. A lying operator
therefore cannot release a key with a fake "fixed", block one with a fake known issue, or mark a tag
disclosed with a wrong key.

A verdict is final when no challenge is open, it is not `InfraError`, and one of these holds:
1. it is T1 and its payout is at most `instant_cap`; or
2. its challenge window (`challenge` seconds) has passed. Above `instant_cap`, an independent `second`
   run of the same PoC must also agree: same kind, keys and impact. Anyone may request it and pays for
   it. A second run that disagrees voids the verdict (§6.5).

Allocation waits at a commit that is not final (§7.1), so whoever is owed a later payout has a reason to
request the second run that unblocks it.

A T1 verdict above the cap still waits the window and needs an agreeing second run, because a TDX quote
can be forged with physical access to a machine. Exposure to that attack is bounded by §6.3.

### 6.3 Instant payments

A holder (§7) is paid **in seconds** when all of the following hold:
- its verdict is T1;
- its amount is at most `instant_cap`;
- this window's instant payments plus this one stay within `instant_budget` (fixed windows of
  `instant_window` seconds).

Otherwise it is paid when its window has passed. "Paid in seconds under the cap; larger payouts after the
window."

Missing quote (about 10% of runs today). A submission's verdict falls back to its T0 window. A
structural run (activation, a known issue) is simply requested again; a round check without a quote
past `run_timeout` lets the project propose again (§8.1), and a re-run without one may be requested
again within the re-run window (§8.2).

### 6.4 Challenges: settled by re-execution

Anyone may challenge a verdict that is not final, within its window, with `challenge_bond`. A challenge
is a demand for T1:
- it requests a re-run of the same request (the challenger pays);
- **any T1 result for that request settles it:**
  - identical facts **uphold** the verdict. The challenger's bond goes to the hunter, or to the escrow if
    the hunter challenged their own adverse verdict;
  - different facts **replace** it. The challenger's bond is refunded;
- **no T1 result within `resolve` seconds voids the verdict** (§6.5). Both bonds are refunded and nobody
  gains.

No person resolves a challenge. There is no arbiter in the default path.

### 6.5 Void

A void verdict sends the commit back to verification with its priority kept. A run that never answers
(`run_timeout`), a disagreeing second run, and a challenge without a T1 result all end here. After
`infra_grace` without a non-infrastructure verdict, the commit closes: bond refunded, nothing paid.

## 7. Allocation and payments

### 7.1 The cursor

Each program has an **allocation cursor** over commit indices. The commit at the cursor is allocated when
it is terminal or its verdict is final. Allocation happens strictly in commit order, so a key always goes
to the earliest valid commit, and a later finding never waits on a later one. Anyone may advance the
cursor (`allocate`, bounded batches).

A commit at the cursor that is still unrevealed or in flight holds the cursor until it resolves. The
reveal window, `run_timeout`, `resolve` and `infra_grace` bound how long that can be.

### 7.2 Keys and groups

A violation's root cause is a set of **keys** `(slot, group)`, where:
- `slot` is the broken invariant;
- `group` is the entry point that broke it.
  - `per-function` (default): the target and the selector.
  - `custom`: committed groups of entry points. For example, `borrow` and `withdrawCollateral` reach one
    code path and share a group.

`tag = mac("antibody/v2/tag" ‖ program_key ‖ u8(slot) ‖ group_id)` (§11.2). Tags are opaque until
disclosure.

### 7.3 Holders, duplicates and known matches

At allocation of a final `Violation` in round r:
- each key not held in round r and not known **is newly held** by this commit;
- if the commit newly holds at least one key, it is a **holder** and is paid for them (§7.4);
- if every key is held, it is a **duplicate** of the earliest holder of its highest-severity key;
- if every key is a known issue, it is a **known match**.

Duplicates and known matches are **never paid**, except through the appeal (§8.4). Their bonds are
refunded. A duplicate keeps a **proof record**: its keys, receipt hash and tx links. The record is
blurred until disclosure, when it opens with the claim it duplicates.

### 7.4 Amounts and budgets

`nominal = base(highest severity among the newly held keys) + min(impact × impact_bps / 10⁴, impact_cap)`.

`amount = min(nominal, budget(slot) − spent(round, slot))`, where `slot` is the slot of that
highest-severity key. **Each invariant has a budget per round**, so many entry points into one root cause
cannot drain the escrow. A holder whose budget is exhausted holds its keys and is paid nothing
(`exhausted`), unless it proves distinctness (§7.5).

### 7.5 Distinctness top-up (off)

The design: a hunter whose invariant budget is exhausted supplies a patch, and a `distinct` run re-runs
the earlier holder's sealed PoC and the hunter's own at the patched code; if the patch stops the earlier
PoC and not the hunter's, the bugs are distinct and the hunter is paid beyond the budget. It is **off**
(`request_distinct` is refused): the requester chooses both the earlier finding and the patch, so a
patch that breaks the slot only on some property of the earlier sealed PoC reads it one bit per run,
and an earlier finding that never broke the slot passes a no-op patch. An exhausted finding is recorded
(`exhausted`); the next round's fix shows whether its bug was distinct (§8.4). A safe version would judge
only against a T1-checked round's code and the holder that used up the budget (roadmap).

### 7.6 Known issues (sealed)

Before open only, the project submits sealed PoCs of known issues. Each is verified like a submission and
counts only when T1-confirmed.
- **Public:** the count, the slots, the severities and the impact bounds.
- **Sealed:** the content, until disclosure.
- **Caps:** at most `max_count` issues, and `Σ base(severity) ≤ max_share_bps × escrow`.
- **Never after open:** a round may not add known issues.
- A known key blocks proofs of that key for the program's life.

### 7.7 Gap lane

A `NoViolation` whose `protected_harm_usd_micro` exceeds `gap_lane.threshold` on a declared protected
account (user or protocol funds) is a **gap claim**:
- the bond is refunded;
- `claim_amount` is reserved;
- the claim is recorded at its priority, sealed.

When a later round adds an invariant that the claim's PoC breaks (its `rerun`), the claim is paid at its
original priority. Profit-only results, such as arbitrage that harms no protected account, stay
`NoViolation`.

### 7.8 Reservations and the shortfall queue

- A holder reserves its amount from the escrow's available balance when allocated.
- A duplicate reserves its **promotion amount**, so an appeal pays at its original priority.
- A gap claim reserves `claim_amount`.

If the escrow cannot cover a holder, the remainder is **debt**. Debts are queued in commit order.
`settle_debts` (permissionless) funds the queue in order from whatever becomes available (top-ups,
forfeits, released reservations), and `exit` funds it once more. A later claim never reserves while an
earlier one is in debt. **Never a pro-rata cut.**

### 7.9 Paying

A funded holder is paid by crediting its beneficiary's internal balance (§13: an agent's owner):
- the protocol fee, `protocol_fee_bps` of the amount, is taken from the escrow after the hunter, never
  from a top-up, and only if the escrow covers it;
- the hunter's bond was already refunded at allocation;
- after payment, the project (or anyone) may request its `release_wrap` run (§11.4); `antibody program read`
  does it.

**Credit, then withdraw:** `withdraw` is the only outbound transfer, and a failed transfer is credited
back.

### 7.10 Conservation (`audit()`)

For every program:
- `funded = escrow + top-ups + forfeited bonds + challenge bonds awarded to escrow`;
- `funded = available + reserved + paid + fees + returned`;
- `reserved = Σ reservations`.

For the contract:
- `token balance = Σ(available + reserved) + Σ credits + locked bonds + locked challenge bonds`.

`audit(program_id)` returns both sides of each equation.

## 8. Rounds and immune memory

### 8.1 Proposing a fix

`propose_round(program_id, overrides, new_slots, manifest)`. A round may:
- **replace target code** at the same anchor (code overrides);
- **add invariants** (new slots).

It may never remove or weaken an invariant, add known issues, change payouts or windows, or touch
anything else. A `check_round` run (TEE) verifies:
- only target code changed, and the new code hashes match the overrides;
- the old slots are byte-identical;
- `check()` holds at the anchor with the new code;
- the **canaries** still succeed. A "fix" that makes the targets revert everything is rejected;
- the round keeps every invariant of the open round: its slots extend the open round's (near-antibody sends
  that manifest along), and the invariants code changes only together with new slots;
- the **new slots** the proposal will price are exactly the ones the round manifest adds, at their
  severities;
- every proposed **release** names a target that is an ERC-1967 proxy at the anchor, and the code hash of
  the override that replaces that proxy's implementation.

The proposal brings the round's **base witness**: the anchor state that the round-0 checks and the round's
own checks read (an added invariant, a fix), proven against the anchor. It must carry the program's
round-0 manifest. near-antibody uses it for the round's check, its re-runs and the round's later runs. A release permit can then only install code
  the round's re-runs ran. Base witnesses prove every target's implementation slot for this.

### 8.2 Immune memory

While a round is proposed, and within `rerun_window` of its T1-confirmed round check, anyone may request
(and pay for) a `rerun`, at the proposed fix, of:
- every paid PoC, from any round, and every commit that ever held a key;
- every duplicate of the round (its appeal);
- every gap claim of the round.

Each runs at the new code and invariants, and counts only when T1-confirmed. A re-run that failed, did
not judge, or answered at T0 with no quote past `run_timeout` may be requested again, so
`rerun_window` should exceed `run_timeout`. A re-run counts for its own proposal only: the next proposal
re-runs everything again, so a fix that brings an old bug back is caught. The keeper requests every paid
PoC's re-run.

### 8.3 Opening the round

`open_round` is permissionless once:
- the round has run for at least `round_min`;
- the re-run window has passed, with no requested re-run still in flight;
- every commit of the closing round is allocated or closed.

At opening, for each key held in round r:
- **released** if its holder's re-run shows it fixed, or if nobody requested that re-run;
- **carried** into round r+1 (same holder, no new payment) if its re-run still breaks it, or if a requested
  re-run has no T1 verdict. Only a Violation or a NoViolation is a verdict: a re-run the verifier could
  not judge (InvalidPoC, Unevaluable, a misconfigured round) shows nothing fixed, and may be asked again.

### 8.4 The one-way appeal

A duplicate of a key that was released is **promoted** if its own re-run still breaks that key. The bug
it proves is distinct, since the fix stopped the first one and not this one. It becomes the key's holder
in round r+1 and is paid **at its original priority**, from the reservation it took in commit order. Only
the earliest such duplicate per key is promoted. A duplicate whose re-run shows it fixed, or whose key
was carried, is a **confirmed duplicate**: its reservation is released and its proof opens at disclosure.

**A fix can only add payments, never remove one.** Without a fix, duplicates are judged by keys alone and
see their proof at disclosure.

### 8.5 Gap claims at opening

A gap claim whose re-run breaks an invariant this round added is promoted and paid at its original
priority. Otherwise its reservation is released and the claim stays on record.

## 9. Release gate and release permits

A round's fix may ship only if, at opening:
1. every paid PoC, from any round, was re-run at this fix and shows no violation (immune memory);
2. **every Critical or High PoC of the round, duplicates included, was re-run and is fixed**. A duplicate
   that still breaks is an open Critical even when the first finding was fixed;
3. no gap claim still breaks;
4. the canaries pass;
5. every holder of a Critical or High key, from any round, was re-run at this fix and shows no violation.

When the gate holds, near-antibody asks the guardian for a **release permit** per proxy and chain, which
pays `release_fee`:

```
ReleasePermit(bytes32 program, uint32 round, address proxy, bytes32 codeHash, uint256 deadline)
```

It is EIP-712 (domain in §10.2), signed by Chain Signatures under the project's path. The program's adapter
upgrades `proxy` only to an implementation whose on-chain code hash equals `codeHash`, once per round and
proxy, so **only code that survived is shipped**. Each signature needs its own gas; permits beyond one
call's are queued at the guardian, and anyone's `issue_pending` asks for them.

The program record shows "survived N days, X at stake, N verified attempts", plus rule strength
(§19.3). It never says "secure".

## 10. Pause permits

### 10.1 Policy

The manifest commits:
- which slots may pause;
- a minimum proven impact;
- per chain, the adapter and the targets with their anchor code hashes.

**A pause ticket** is issued for a holder whose verdict is **T1**, whose key is in the pause policy and
whose impact reaches the minimum, on a program whose anchor is `LightClient`. At most one ticket per
program per round.

### 10.2 Permit

The guardian (`contracts/near-guardian`) asks the MPC to sign

```
PausePermit(bytes32 program, uint32 round, uint64 ticket, uint256 deadline)
```

as an EIP-712 digest under the domain `{name "Antibody", version "1", chainId, verifyingContract = adapter}`.
The derivation path is `antibody/<project account>/<program key>`, so each program of each project has its
own guardian address; the guardian pins it when the project registers, and the program's adapter trusts
only it. The guardian holds no other path and has no other calldata: `pause()` is fixed in code.

### 10.3 Adapter

`AntibodyAdapter` (`contracts/evm/src/antibody/AntibodyAdapter.sol`), deployed with CREATE2 per program
(salt `keccak256(program key)`), so it has the same address on every chain. It:
- checks the program, the deadline and the signature (`ecrecover` against the pinned guardian address;
  low-s only), and uses each replay key once: `("pause", round, ticket)` or `("release", round, proxy)`.
  Replay is keyed on the permit's meaning, not the signature;
- `pause`: calls `pause()` on each target fixed at deployment and emits a result per target (**best
  effort per target**);
- `release`: upgrades a registered `AntibodyProxy` (ERC-1967) to an implementation whose runtime code
  hash is the permit's.

The adapter can never unpause; that stays with the project.

### 10.4 Relaying

Relaying is permissionless. Antibody's relayer (funded from fees) submits every permit, and anyone else
may. No EVM nonces, gas prices or fees live on NEAR.

## 11. Keys and confidentiality

### 11.1 The root key

The verifier wasm declares one OutLayer encryption key in its `outlayer.manifest` section:

```
"encryption_keys": [{"path": "antibody", "bind": "wasm", "caller": "predecessor"}]
```

OutLayer's keystore derives it inside its TEE from the build's SHA-256 and the account that called
OutLayer. **Only this exact build, called by near-antibody, ever holds it.** No secret row, owner or access
rule exists that anyone could change. The guest never sees the key. It asks the host for
`mac(path, data)` (HMAC-SHA256 under a key derived for MACs alone).

### 11.2 Per-program keys (domain-separated)

```
label(l, rest)  = l ‖ 0x00 ‖ u32_be(len(program_key)) ‖ program_key ‖ rest
seal_sk(p)      = mac(label("antibody/v2/seal", ""))                X25519 secret, clamped
tag(p, s, g)    = mac(label("antibody/v2/tag", u8(s) ‖ g))
disclose(p, t, r) = mac(label("antibody/v2/disclose", t ‖ u32(r)))  one key per tag and round
```

The `activate` run outputs `seal_pk(p) = X25519(seal_sk(p), 9)` and `key_epoch = SHA-256(seal_pk)[0..8]`.
The contract accepts them only from a T1-confirmed activation (§3.1). A rotated key changes the epoch, and
a submission sealed to another epoch is `InfraError`, never `InvalidPoC`.

### 11.3 Sealed blob

```
κ = random content key;  wrap_v = κ wrapped to seal_pk (X25519 ephemeral + HKDF + AEAD, bound to program_key)
blob = "ABSEAL1\0" ‖ wrap_v ‖ nonce ‖ AEAD(κ, program_key ‖ hunter ‖ witness ‖ report, ad = program_key)
```

The program and the hunter are inside the AEAD, so a blob replayed under another account is
`InvalidPoC` for the replayer; the commitment binds the round (§4.1). The exact construction is in
docs/crypto-spec.md §4.

### 11.4 `release_wrap`

The verify run emits **no wrap**. Facts are public in OutLayer's resolve transaction, so a wrap there
would let a project read a duplicate's PoC for free. After payment, a `release_wrap` run wraps κ to the
project's X25519 key, for paid first findings and promoted duplicates only. Its ephemeral key is
derived from κ and the project's key, so the output is deterministic. The project therefore sees exactly
what it paid for: `antibody program read` requests the wrap, opens the proof and re-runs it locally.

### 11.5 Project key

The project's X25519 key is derived from a NEP-413 signature of a fixed message
(`antibody keygen x25519 --nep413`; the key file is its export). The message names near-antibody and the
project, and says never to sign it anywhere but Antibody's CLI. Low-order points are refused at creation;
the verifier never panics on one. `rotate_project_key` changes the key for future wraps.

### 11.6 Disclosure

Disclosure is per (round, tag): the proofs of a tag committed in round `r` become public when either:
- the tag, held in round `r`, was **fixed** by the next round's code **and the release gate held**, so
  the fixed code ships (§8.3, §9): a disclosed proof never works against the code a failed gate kept; or
- the tag's holder in round `r` was paid (the project has seen it, §11.4) and `disclosure_max_delay` has
  passed since the claim.

After `exit`, every claimed (round, tag) qualifies. **No live bug goes public before the project has seen
it**, and a later round's proof of the same tag (a new bug behind the same entry point) stays sealed
under its own round's key.

**Paged disclosure (shipped).** `request_disclosure(program_id, [{round, tag}])` is permissionless and
anyone may fund it. A `disclose` run outputs `disclose(p, t, r)` for each qualifying item. Every round-`r`
fact of that tag carries `wrap_d = AEAD(disclose(p, t, r), κ)`, so one key opens all of the tag's round-`r`
PoCs, duplicates included. The condition is monotone and depends on nobody's consent, so disclosure cannot
be blocked once it holds.

**Scheduled disclosure (target, gated by the CKD spike).** Per-tag keys come from NEAR MPC confidential
key derivation (`request_app_private_key`):
- near-antibody is the `app_id`, and the program and tag form the derivation string;
- every published key is verified with the pairing check;
- near-antibody requests the public key when the condition holds, so neither OutLayer nor Antibody can
  delay it.

## 12. Economics

### 12.1 Requester pays

- Each account holds an NEP-145-style **credit** in NEAR for storage and verification.
  - `storage_deposit` tops it up; a new account's entry is paid from its own deposit.
  - An agent key can spend it but not withdraw it.
  - The storage a call adds (a program, a commit, an agent, a proposal, a disclosure request) is
    charged to the payer's credit.
  - Every run is charged the program's pinned deposit plus a 0.01 NEAR allowance for the storage of its
    answer. OutLayer refunds the unused part of the deposit to the requester (`payer_account_id`).
- Withdrawals of token credit and of NEAR credit attach one yoctoNEAR, so only a full-access key withdraws.
- OutLayer keeps its base fee (0.001 NEAR on testnet) on every attempt, failed ones included.
- Projects pay activation, known issues, `check_round` and re-runs of their own choosing. Whoever
  requests a disclosure pays for it.
- near-antibody's own balance never pays for anyone's call.

### 12.2 Bonds

The recommended floor is `3 × p95(cost of verifying one submission, T1 relay included)`, valued at the
NEAR/tUSD rate the project uses; `docs/measurements.md` shows the formula with today's measured values.
The contract requires a positive bond. Bonds price slop:
- a final `NoViolation`, a final `InvalidPoC` or an expired commit **forfeits the bond to the program's
  escrow**, where it pays future findings;
- the protocol never earns from forfeits.

### 12.3 Revenue

- `protocol_fee_bps` on payouts.
- `release_fee` per release permit.
- Later, a paid drafter and rule-strength tier (§19).

## 13. Agents

- **Registry.** `register_agent(agent, bond_budget, per_commit_cap)` is called by the owner. The agent is
  a sub-account of the owner, so only the owner could create it.
- **Key.** A function-call key on near-antibody, limited to `commit`, `reveal`, `retry` and FastFS
  uploads. It cannot withdraw (one yoctoNEAR is required), transfer tokens, or attach deposits.
- **Bonds.** Agent bonds come from the owner's credit, within `bond_budget` (total locked) and
  `per_commit_cap`. A commit over either is refused: the call returns no commit and records a
  **containment event** (counted in the statement, and logged).
- **Payouts and refunds** go to the owner's credit.
- **Agent statement** (view): bonds locked and lost, verification spend, payouts, net, key permissions,
  containment events, and the record of its proofs (verdict, tier, receipt, tx links).

## 14. Liveness

### 14.1 Timeouts

Every in-flight state carries its run id, and stale callbacks are ignored. `time_out(commit_id)` is
permissionless:
- after `run_timeout` (twice OutLayer's yield timeout), a run that never answered is void (§6.5);
- after `resolve`, an unresolved challenge is void;
- after `infra_grace`, an infrastructure failure closes with a refund.

Callbacks parse raw bytes, and every decode failure is a typed error, never a panic.

### 14.2 Keeper

The keeper is permissionless and advances everything:
- relays quotes (§6.1);
- requests the second run an above-cap verdict needs (§6.2);
- allocates, pays and funds the shortfall queue;
- requests immune-memory re-runs;
- opens rounds;
- discloses;
- exits.

`crates/services` runs one, and anyone can.

### 14.3 Exit

After `deadline + exit_grace`, once every commit is allocated or closed, `exit(program_id)` settles every
state. Nothing in flight can wait it out: unrevealed commits `expire` after the reveal window and runs
`time_out` after their timeout and grace (permissionless; the keeper bot calls them), and `allocate`
moves the cursor past every final verdict, so the exit voids nothing that could still be paid. The
exit's work is bounded by the program's keys, not its commits (reservations, debts and owed claims are
tracked lists):
1. duplicates' and gap claims' reservations are released first, so they fund the owed claims;
2. debts are funded in commit order from what is available;
3. owed claims are paid what is reserved for them: in full, or in part when the escrow never covered them;
4. the rest of the escrow is returned to the project's credit.

Then `close_commits(program_id, max)` (permissionless, paged) closes the other records: duplicates
confirmed, gap claims closed. A program closed early as misconfigured (§5.3) exits at once; there
`close_commits` also expires unrevealed commits, refunds in-flight verdicts and open challenges, and
returns any bond that expired to the project. No program's size can exceed a call's gas.

The part of a debt the project never covered is recorded as **defaulted**.

## 15. Pins and versions

At creation each program pins:
- the verifier build (`wasm_url`, `wasm_sha256`);
- the OutLayer account;
- the TEE policy: the DCAP verifier account, the accepted TCB statuses, the measurement set and the
  image delay;
- the light client;
- the guardian.

The payout token is fixed for the deployment's life, since every credit is in it. Fee rates are terms
of the manifest (§3.2); fees accrue to the deployment's treasury.

The owner's setters change the **defaults for new programs only**.

- **Measurements.** The set is a snapshot of OutLayer's approved images at creation (`sync_images`,
  permissionless, copies a registry's list), and the program pins that registry. Later approvals count
  for a program only if its own registry lists them, and only after `image_delay`, which is longer than
  the challenge window: a registry the owner names later never reaches a live program. A veto can only
  remove an image, which forces the window, and can never cancel a payout.
- **Locked deployment.** The launch accounts (near-antibody, guardian, practice vending) hold no
  full-access keys, and their builds are reproducible (NEP-330 metadata).
- **Versions.** A new contract version is deployed beside the old one and serves new programs. Old
  programs run to their exit.

## 16. Terms

The manifest commits `terms_sha256`: safe harbor, disclosure policy and payment terms. A hunter accepts
them by committing with that hash.

## 17. Light client

The anchor is checked against `client-eth2.sepolia.testnet` (v0.8.0), which runs with
`validate_updates: true` and `verify_bls_signatures: true`:
- **correctness** rests on Ethereum's sync committee;
- **liveness** rests on the trusted relayer;
- the client is upgradable by its ACL admins.

Demo programs are created with anchors the client already has. A program whose anchor could not be
confirmed is labelled `Unconfirmed` and cannot pause.

## 18. EVM rules

Each program pins its EVM rules, `osaka` today, with the fork policy `pin`: the rules never change during
a program. Every receipt carries the label **"judged under the rules the launch chain runs"**. Testnet
programs read Sepolia state and are judged under the Osaka rules that Ethereum mainnet runs. A
`fork_switch` policy (judge each block under its own fork) is on the roadmap.

## 19. Rule quality

### 19.1 Drafting

`antibody invariants draft` proposes invariants from the targets' sources, deterministically or with an
LLM. A blind evaluation measures it, and every draft must pass `invariants validate`.

### 19.2 Fork-mode programs

A program may commit bytecode plus a deterministic deploy script instead of a live deployment. The
verifier deploys the script at the anchor, so no staging deployment is needed.

### 19.3 Rule strength

Mutation testing of a program's invariants: missing access checks, flipped comparisons, unchecked
transfers. Published as "caught X of N mutants", labelled, and shown on the program record and on
permits. It is not a security score.

## 20. Decisions

The decisions behind this specification, and the alternatives rejected.

| # | Decision | Rejected | Why |
|---|---|---|---|
| D1 | No instant payment without T1; instant budget per window; above the cap, window plus an agreeing second run | Cap per payout only | Sub-cap forged claims could drain the escrow one by one |
| D2 | Challenges are settled by re-execution: T1 decides, no T1 voids | A human arbiter; T0 majority | A person in the path; a lying operator agrees with itself |
| D3 | Every adverse T0 fact is provisional | Forfeits and duplicates at T0 | An operator could forfeit honest bonds or close programs |
| D4 | One allocation cursor over final verdicts | Pay as verdicts arrive | A late reveal of an earlier commit would make two holders of one key |
| D5 | Reservations in commit order; debt queue | Pro-rata | Earlier finders keep their priority; nobody is cut |
| D6 | Duplicates: one-way appeal with reservations | Pay duplicates; never pay | A fix can only add payments; distinct bugs behind one key are paid |
| D7 | Release gate counts duplicates | Holders only | The model found a release permit signed while a distinct Critical still broke the fix |
| D8 | Keys from OutLayer encryption keys (wasm, predecessor) | A stored secret with an access rule | No row, owner or rule left to move |
| D9 | `verify` emits no wrap; `release_wrap` after payment | Wrap in every verdict | A public wrap gives free reads of duplicates |
| D10 | Infrastructure failures are never forfeited | Unfetchable blob = `InvalidPoC` | One lagging RPC must not take honest bonds |
| D11 | Forfeits go to the program's escrow | To the treasury or the project | Nobody profits from rejecting |
| D12 | Pause permits under the project's own key, adapter per chain, best effort, ticket replay keys | A shared guardian building transactions | Cross-tenant pauses; fragile nonces |
| D13 | Exit after deadline plus grace | Locked contracts without exit | Nothing can stay stuck |
| D14 | Known issues sealed, capped, only before open | Unlimited, public, anytime | Projects could pre-claim the escrow or leak live bugs |
| D15 | Gap lane for protected-account harm | Pay any harm | Profit-only arbitrage is not a bug in a rule |

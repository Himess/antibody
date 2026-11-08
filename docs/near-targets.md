# NEAR contracts as targets: verifier v2 (design)

**Status: designed and reviewed on 2026-11-08 (§13); the engine and the v2 binary are built and tested (§14);
nothing is deployed and no program pins v2.** CI runs the engine's tests, checks that v2's native and
wasm32-wasip2 builds give byte-identical output on two NEAR fixtures, and checks v1's recorded EVM fixtures
through v2 byte for byte. What stays to do before a program pins v2 is §15.

Today the verifier judges EVM contracts only, while NEAR settles, attests and pauses. Verifier v2 lets a
program commit NEAR contracts (wasm) as its targets, so Antibody protects NEAR applications themselves. v2
is v1 plus a NEAR execution engine. The EVM path stays byte for byte. Only programs whose targets are NEAR
contracts pin v2.

## 1. What near-antibody already allows, and what stays as it is

near-antibody is locked: it is not redeployed or upgraded, and no live program's pins change. Three of
its properties make NEAR targets possible without touching it:

- **The verifier is pinned per program, at creation, from the owner's defaults.** `create_program` copies
  `defaults.verifier` (wasm URL, wasm hash, OutLayer resource limits and the run deposit) into the
  program's `Pins` (`contracts/near-antibody/src/lib.rs`, `create_program`). Live programs keep their
  pins (docs/spec.md §15). `set_defaults` replaces only the defaults; the image registries list changes
  only if the registry account changes (`lib.rs`, `set_defaults`).
- **The manifest parser reads a fixed set of fields and ignores the rest.** `ManifestIn` reads `schema`
  (it must be `antibody.manifest.v2`), `program`, `anchor` (`chain_id` and `block_number` as integers,
  `block_hash` as 32 bytes of hex, `evm_spec` as a string), `targets` (`name`, `address` and `code_hash`
  as strings, at most 32), `invariants` (`slots`, `gas_limit`) and `settlement`
  (`contracts/near-antibody/src/settlement.rs`). Its serde derive does not deny unknown fields, so v2's new
  fields pass through. A target's address and code hash are kept as strings
  (`ProgramMeta.targets`), never parsed as EVM addresses.
- **The anchor is checked only on the light client's chain.** `checkable` is true only when
  `anchor.chain_id` equals the light client's chain (Sepolia today); any other anchor is labelled
  project-attested (`anchor_uncheckable`) (`lib.rs`, `create_program`).

Everything NEAR-specific therefore lives in the verifier. near-antibody keeps settling verdicts by
their facts (kind, keys, impact, receipt hash) as it does for EVM programs.

## 2. The manifest: NEAR targets in `antibody.manifest.v2`

The fields near-antibody reads keep their types and meaning:

| Field | For a NEAR program |
|---|---|
| `anchor.chain_id` | 398, which v2 reads as NEAR testnet. It is not the light client's chain, so near-antibody labels the anchor project-attested |
| `anchor.block_number`, `anchor.block_hash` | The NEAR block whose context the run uses (fork mode), or whose state it proves (live state, §9). The hash is the block hash's 32 bytes, in hex |
| `anchor.evm_spec` | `near` (the field is a free string; v2 reads it with the new `vm` field below and refuses a mismatch) |
| `targets[]` | `name`; `address`, the NEAR account; `code_hash`, the lowercase hex SHA-256 of its wasm |
| `invariants.slots`, `invariants.gas_limit` | The rules' slots, as on EVM; `gas_limit` becomes the rules module's instruction budget per evaluation |
| `settlement` | Unchanged |

New fields, which only v2 reads:

| Field | Meaning |
|---|---|
| `vm` | `"near"`. Absent, the program is an EVM program and v2 behaves exactly as v1 |
| `near.protocol` | The NEAR protocol version whose runtime semantics and fee schedule v2 reproduces (pinned per release of v2) |
| `near.block` | The fixed block context: `height`, `timestamp_ns`, `epoch_height`, `random_seed` (hex), `gas_price` |
| `near.contracts[]` | Each target's `account`, `wasm` (`url`, `sha256`) and `init` (`method`, `args` base64, `deposit`, `gas`) |
| `near.accounts[]` | The accounts the run starts with: `account`, `amount` and `locked` (yoctoNEAR) |
| `near.setup[]` | Calls run before anything else, as `deploy` does in EVM fork mode: `signer`, `receiver`, `method`, `args`, `deposit`, `gas` |
| `near.rules` | The rules module: `wasm` (`url`, `sha256`) and its source's location |
| `near.impact` | `native_usd` (NEAR's committed price), `tokens[]` (NEP-141 contracts: `contract`, `symbol`, `decimals`, `usd`) and `protocol_accounts[]` |
| `near.capabilities` | `max_senders`, `max_endowment_yocto`, `max_calls`, `max_receipts`, `max_instructions_per_call`, `max_total_instructions`, `max_eval_instructions` |

A proof (the hunter's bundle) for a NEAR program is a list of top-level calls: `signer` (one of at most
`max_senders` fresh accounts the run creates, each endowed with at most `max_endowment_yocto`),
`receiver`, `method`, `args` (base64), `deposit` and `gas`. As on EVM, the bundle is sealed, committed and
revealed, and its size is capped.

## 3. The execution engine

**The interpreter.** v2 embeds wasmi, a WebAssembly interpreter written in Rust, pinned exactly
(`wasmi = "=2.0.0"`) and vendored into the repository if the reproducible build needs it, with its
SHA-256 recorded.
- **Why an interpreter:** the verifier itself runs as wasm inside OutLayer's TEE workers, where a compiler
  to native code cannot run. wasmi is deterministic, has no system dependencies, builds for
  `wasm32-wasip2` and for the native targets, and meters execution with fuel.
- **What it is not:** nearcore's own VMs compile contracts to native code; v2 reproduces their semantics,
  measured against nearcore's sandbox (§8), not their implementation.

**Validation.** Each module is validated against the wasm features the pinned NEAR protocol version
enables, and nothing else. A module that uses another feature, imports a host function v2 does not
implement (§4), or exceeds a limit (§10) is refused. A target refused at activation makes the program
`ProgramMisconfigured`. A module deployed during a run and refused there makes that run `Unevaluable`.

**Determinism.** The engine is a total function of the manifest, the base inputs and the bundle. It reads
no clock, no randomness and no environment: the block context, the accounts and the random seed all come
from the manifest. The native and wasm builds must give byte-identical receipts on every fixture (§8).

**Floats.** nearcore canonicalises the NaN results of float instructions. v2 does not run float code
until it does exactly the same, checked against the sandbox: until then, a run that executes a float
instruction is `Unevaluable`. Modules that only contain float code (as many do, through formatting
libraries) are accepted, since what counts is what runs.

**Gas.** v2 charges gas with the pinned protocol version's fee schedule: wasm instructions, host
functions, actions, and the gas each receipt is given. Its instruction count is not nearcore's exact
metering, so no verdict may rest on gas:
- **Exhaustion:** a receipt that runs out of gas in v2, or ends within 10% of its prepaid gas, makes the run
  `Unevaluable`. It is never judged as a NEAR out-of-gas failure.
- **Perturbation:** the bundle runs a second time with every wasm and host function cost doubled. If any
  receipt outcome, return value, log, storage write, balance or rule result differs, the run is
  `Unevaluable`. This covers contracts that read `used_gas` or `prepaid_gas`, and gas distributed by
  weight.
- **Fuel:** wasmi's fuel is the hard bound on all execution (`max_instructions_per_call`,
  `max_total_instructions`). Running out of it is `Unevaluable` for the bundle's calls and `InvalidPoC`
  for budgets the hunter can compute locally, as on EVM (docs/spec.md §5.5).
- **Balances:** gas fees are not charged to balances, and impact excludes them (§6).

## 4. Host functions

near-sdk-rs 5.x contracts import NEAR's host functions from `env`. Each one is either implemented with
nearcore's semantics or refused. A refused or unknown import makes the module refused (§3). v2 never
stubs a host function silently.

| Group | Host functions | v2 |
|---|---|---|
| Registers | `read_register`, `register_len`, `write_register` | Implemented (nearcore's register limits) |
| Context | `current_account_id`, `signer_account_id`, `signer_account_pk`, `predecessor_account_id`, `input`, `block_index`, `block_timestamp`, `epoch_height`, `storage_usage` | Implemented; block values from the fixed context (§5) |
| Economics | `account_balance`, `account_locked_balance`, `attached_deposit`, `prepaid_gas`, `used_gas` | Implemented; `prepaid_gas` and `used_gas` under the gas rules (§3) |
| Math | `random_seed`, `sha256`, `keccak256`, `keccak512`, `ripemd160`, `ecrecover`, `ed25519_verify` | Implemented, with byte-for-byte vectors from the sandbox |
| Math (pairings) | `alt_bn128_g1_multiexp`, `alt_bn128_g1_sum`, `alt_bn128_pairing_check`, the BLS12-381 functions | Refused in the first release |
| Results | `value_return`, `panic`, `panic_utf8`, `log_utf8`, `log_utf16`, `abort` | Implemented (log limits as nearcore's) |
| Promises | `promise_create`, `promise_then`, `promise_and`, `promise_batch_create`, `promise_batch_then`, `promise_results_count`, `promise_result`, `promise_return` | Implemented (§5) |
| Actions | `promise_batch_action_create_account`, `_deploy_contract`, `_function_call`, `_function_call_weight`, `_transfer`, `_add_key_with_full_access`, `_add_key_with_function_call`, `_delete_key`, `_delete_account` | Implemented (keys are recorded state: v2 checks no signatures) |
| Actions (refused) | `promise_batch_action_stake`, `promise_yield_create`, `promise_yield_resume`, `validator_stake`, `validator_total_stake` | Refused: v2 models no validators and no yielded receipts |
| Storage | `storage_write`, `storage_read`, `storage_remove`, `storage_has_key` | Implemented, with storage usage as nearcore counts it |

The list is fixed per protocol version. An import outside it is refused by name.

## 5. Receipts, promises and accounts

**Order.** v2 runs receipts in one deterministic order, as one shard would:
- **Top-level calls:** each call of the bundle becomes a receipt; the next call starts once the previous
  call's whole receipt tree has finished.
- **Receipts within a call:** outgoing receipts join a FIFO queue in the order they were created. A
  receipt that waits on data (a callback) joins the queue when its last input arrives.
- **Blocks:** each pass over the queue is one block; the height goes up by one and the timestamp by one
  second per pass, from the fixed context.

NEAR gives no synchronous cross-contract state, so contract semantics do not depend on this order beyond
what any valid NEAR order allows. A proof is judged on v2's order, which the hunter reproduces locally.

**Semantics** (nearcore's):
- **Failures:** a failed receipt rolls back its own changes only: storage, balances and the accounts and
  code its actions created. Its outgoing promises are not created.
- **Callbacks** get their promises' results: success with the returned data, or failure.
- **Refunds:** a failed receipt's attached deposit returns to its predecessor as a refund receipt.
- **Batch actions** run in order within a receipt: account creation (a sub-account of the predecessor, or
  a top-level account as nearcore allows on testnet), contract deploys, function calls, transfers, keys
  and account deletion with a beneficiary.
- **Bounds:** the number of receipts per call and per run (`max_receipts`), the promise depth, and
  nearcore's limits on promises per function call.

**Accounts.**
- **Balances:** each account has `amount`, `locked` and its storage usage, counted as nearcore counts it
  (the account record, data keys and values, code and access keys).
- **Storage staking:** after each receipt, an account whose balance no longer covers its storage
  (`storage_usage × storage byte cost`) fails that receipt, which rolls back, as nearcore does.
- **Context:** the block context (height, timestamp, epoch height, random seed and gas price) is fixed by
  the manifest and advances only as above.

## 6. Rules, keys and impact

**Rules as code.** A NEAR program's rules are a wasm module written in Rust with a small SDK crate
(`antibody-near-rules`, new).
- **Shape:** the module exports `check`, which returns the mask of broken slots.
- **What it can read:** synchronous view calls into the targets (read-only, under their own instruction
  budget), account balances, and NEP-141 balances. The verifier provides these; the module cannot change
  any state.
- **When it runs:** at the anchor (all rules must hold, or the program is misconfigured) and after each
  top-level call's receipt tree has finished, as EVM's `check()` runs after each entry call. Not after
  each receipt: in the middle of a cross-contract flow, funds are legitimately in flight, and a rule
  checked there would break on correct code.
- **Slots and severities** as on EVM. A rule that panics or runs out of its budget after the proof is
  `Unevaluable`, handled by the committed policy (docs/spec.md §5.4).

**The finding's key.** `(slot, group)`, as on EVM (docs/spec.md §7.2). The default group is the entry
point: the top-level call's receiver account and method name. Custom groups name sets of
`(account, method)` pairs. Tags are the same MACs.

**Impact.** `impact_usd_micro = min(protocol loss, attacker gain)` at the committed prices, as on EVM:
- the NEAR balances of the protocol accounts and the senders, gas excluded;
- the listed NEP-141 tokens' balances, read through `ft_balance_of` views.

**The receipt.** v2 writes the same receipt format the rest of the system reads (`antibody.receipt.v1`)
and the same facts (`kind`, `keys`, `impact_usd_micro`, `receipt_sha256`):
- **What changes:** `chain_id` is the NEAR value, `env` holds the fixed block context, and the bundle
  commitment hashes the NEAR calls.
- **What is added:** a `near` section with each receipt's outcome.

## 7. Fork mode first, live state second

**Fork mode (first).** The targets run inside the verifier from their published wasm: the manifest's
accounts, deployed contracts, `init` calls and `setup` calls build the starting state, as EVM fork mode's
`deploy` does (docs/fork-mode.md). Nothing in it reads a real NEAR block's state. The anchor names the
block whose context is used, and near-antibody labels it project-attested.

**Live NEAR state (second, §9).** The targets are contracts as deployed on NEAR testnet, at a NEAR block.

## 8. Correctness evidence (CI)

- **Conformance with nearcore's sandbox (the reference runtime):**
  - **Corpus:** near-sdk-rs contracts (a fungible token, a non-fungible token, a status message) and the
    NEAR fixtures.
  - **Method:** each call sequence runs in the sandbox and in v2. The suite compares return values, logs,
    final storage, account balances and promise outcomes. Amounts that depend on gas are compared with gas
    excluded, and the report says which ones.
- **Native == wasm:** on every NEAR and EVM fixture.
- **v1 receipts reproduced by v2:** byte for byte, on every EVM fixture. EVM programs keep pinning v1.
- **Fuzzing:** the NEAR bundle parser, malformed wasm modules, and host-function edge cases.
- **Limits:** one test per limit in §10.

## 9. Live NEAR state (N7)

- **Anchor:** a NEAR block (height and hash).
- **Witness:**
  - the targets' code, account records and storage, with state proofs from NEAR's RPC (`view_state` with
    proofs);
  - the block header, and the chunk headers that carry each shard's state root.
- **In the enclave:**
  - v2 checks that the header hashes to the anchor's hash and that each chunk header is in the block;
  - it then checks every state proof against its shard's state root.
  - v2 does not check NEAR's consensus: the block is the project's claim.
- **On chain:** near-antibody labels the anchor project-attested, since it checks Ethereum anchors only.
  The docs and the site say what the enclave verified and what near-antibody did not.

## 10. Limits (v2's, proposed; checked against OutLayer's measured resources)

| Limit | Proposed |
|---|---|
| A contract's wasm | 4 MiB (nearcore's maximum contract size) |
| Linear memory | nearcore's memory limit for the pinned protocol version |
| Receipts per run | 256 |
| Promise depth | 16 |
| Logs | nearcore's per-receipt count and byte limits |
| Storage written per run | 1 MiB |
| Instructions | per call and per run, set from OutLayer's measured throughput so the slowest allowed run ends within the pinned `max_execution_seconds` |
| The bundle | at most 15 KB sealed, as on EVM |

v2's pin may set its own OutLayer limits (`max_instructions`, `max_memory_mb`, `max_execution_seconds`,
`deposit`), since `VerifierPin` is per program. They are measured on OutLayer before the defaults window.

## 11. How programs pin v2

During the defaults window, the owner (`antibody.testnet`):
1. reads `get_defaults` and records it;
2. sets v2 as the default verifier, with every other field identical (`set_defaults`);
3. creates the NEAR-target programs;
4. restores the recorded defaults exactly;
5. reads `get_defaults` again and compares it with step 1, byte for byte.

Every call is recorded with its transaction. No live program's pins change: pins are copied at creation
only. EVM programs keep v1.

## 12. T2

v2 programs have no T2. near-t2 pins v1's guest key and wasm hash, and has no owner. A second near-t2
pinning a v2 guest is optional (N8): only if everything else ships.

## 13. Decisions (reviewed 2026-11-08)

- **NEAR testnet's `anchor.chain_id`:** 398.
- **Rules granularity:** after each top-level call's receipt tree, not after each receipt.
- **Gas:** the second run with every cost doubled, and the 10% margin, as in §3.
- **Floats:** refused (a run that executes one is `Unevaluable`) through the hackathon, whatever the
  conformance suite shows.
- **Pairing and BLS host functions:** refused in the first release.

## 14. Implementation notes (the first build)

The first engine (`crates/verifier-near`, the new `antibody-verifier-v2` binary) follows this design,
with these deviations, each made to match the reference runtime on real near-sdk-rs contracts:

- **A refused host function is refused when called, not when imported (§4).** §4 refuses the module that
  imports a refused host function. But near-sdk-rs links its whole promise surface, so every near-sdk
  contract imports `promise_batch_action_stake` whether or not it stakes, and the reference runtime
  accepts the module (the import resolves) — only the action is unmodelled. The engine therefore links
  the refused host functions and stops the run **when one is called**, by name: `Unevaluable` during a
  run, `ProgramMisconfigured` when it is reached at activation. Importing one, without calling it, is
  accepted, exactly as the reference runtime accepts it. This is not a silent stub: a call deterministically
  stops the run with the named reason. Modules importing an **unknown** host function, or a refused one
  with the wrong signature, are still refused outright.
- **Reference-types parsing is enabled, then its constructs are refused by name (§3).** near-sdk-rs
  output, as the Rust toolchain links it, encodes the `call_indirect` table immediate as an overlong LEB
  zero. The parser wasmi 2.0.0 uses reads that only with reference-types parsing on; the reference
  runtime's own (older) parser reads it either way and so accepts these modules. The engine
  turns on reference-types *parsing* and then refuses, by name, every instruction and construct the
  reference-types proposal adds on top of the MVP (`ref.null`, `ref.func`, `table.*`, typed `select`,
  more than one table, non-funcref tables, reference types in signatures), plus bulk-memory and tail-call
  operators. The accepted feature surface is therefore exactly the reference runtime's; only the encoding
  is tolerated.
- **Gas metering is wasmi fuel, one unit per operator (§3).** The design already says the engine's
  instruction count is not the reference runtime's exact metering and that no verdict may rest on gas
  (hence the doubled-cost run and the 10% margin). The engine counts wasm instructions with wasmi's
  fuel (one unit per executed operator) and does not reproduce the reference runtime's per-stack-frame
  gas or its trie-node storage fees (which depend on the trie's shape). Gas is never charged to balances
  and is excluded from impact, as §3 and §6 require.
- **The 10% margin is measured on a call's own gas (§3).** Read literally ("ends within 10% of its
  prepaid gas"), the margin would flag every call that splits its unused gas by weight (after the split,
  used gas equals prepaid gas by construction, and near-sdk-rs attaches weight 1 by default) and every
  call that forwards most of its gas to the receipt it creates. The engine measures the gas a call burnt
  against the gas it kept for itself (prepaid gas minus what it explicitly attached to outgoing receipts):
  a call that burns more than 90% of that is `Unevaluable`, as is any call that exhausts its gas. Whether
  forwarded gas sufficed is left to the doubled-cost run, which changes every such amount.
- **The doubled-cost run doubles wasm and host function costs only, from the same anchor (§3).** Action
  fees stay as they are (§3 names wasm and host function costs), and activation (`init`, `setup`) runs
  once: the second run replays the bundle from the anchor state with the doubled schedule. Compared:
  every receipt's outcome, returned value and logs, the rules' masks after each call, and the final state
  (storage, balances, keys, code), gas amounts excluded.
- **Rules ABI (§6).** `check` returns an `i64` mask, so slot bits are below 64 (the manifest is refused
  otherwise). The evaluation budget (`invariants.gas_limit` instructions) is one pool that covers the
  rules module and every view it makes, rather than a separate budget per view. An evaluation that fails
  after an intermediate call is skipped, as v1 skips one; the final evaluation failing is `Unevaluable`.
- **The receipt's schema string is `antibody.receipt.v2` (§6).** That is the string v1's program receipts
  carry (`antibody.receipt.v1` is v1's replay and bundle receipt). The NEAR receipt keeps v1's program
  receipt fields in order, with NEAR types (account ids, yoctoNEAR), `chain_id` 398 and the added `near`
  section; the facts are v1's facts structure exactly.

## 15. Before a program pins v2 (the 2026-11-09 review)

A fresh-context review of the engine found these; the first two are fixed.

- **Fixed: memory the host cannot back** traps (a resource limiter on every store). It used to fail quietly:
  `memory.grow` returned -1, or a value stack out of memory became a NEAR failure, in the enclave under its
  memory cap but not natively. It is now `Unevaluable`; it should become an InfraError (retried) when that
  outcome reaches the engine. Every one of wasmi's trap codes is named.
- **Fixed: contract-supplied promise and result indices** no longer wrap on a 32-bit build.
- **To do:**
  - **Resources.**
    - Enforce §10's limit on the storage a run writes.
    - Charge each instantiation, and each rules view's 64 MiB memory, a fixed fuel cost calibrated on OutLayer.
    - Cap the views of one evaluation.
    - Stop cloning an account's whole state on a receipt's first write.
  - **Gas fidelity.** Charge an upper bound of nearcore's trie-node fees on every storage operation in both
    runs, and price wasmi's free control instructions, so the doubled run bounds NEAR's cost.
  - **`random_seed`.** Derive a seed per action, or make a bundle call that reads it `Unevaluable`. Today every
    call gets the manifest's constant.
  - **Receipts.**
    - System receipts (refunds, deletion payouts) should not count against the depth bound.
    - A receipt's result should be its last action's.
    - Model nearcore's receipt-size and recorded-storage limits, and its 256 KiB stack accounting.
    - Refuse `attached_deposit` in view mode.
  - **Activation.**
    - Check each impact token's `ft_balance_of` at activation.
    - Refuse element segments and reference-typed globals, or say here that they are allowed.
  - **Tests.**
    - A byte-identity case for v1's TEE request path through v2.
    - The crypto vectors under wasm.
    - Conformance against nearcore's sandbox (§8).
    - Fuzzing.

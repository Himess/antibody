# A NEAR app consuming verdicts

Antibody's verdicts are NEAR state: any NEAR contract can read them and act. `contracts/verdict-guard`
is the smallest example, a treasury that freezes when the program it watches gets a verified Critical.

## How it decides

Anyone calls `report(commit_id)`. The guard then reads near-antibody itself, by cross-contract view
calls, and trusts the reporter for nothing:

1. `get_commit(commit_id)`: the commit must be to the watched program, with a verification run;
2. `get_run(run)` and `get_program(program)`: the run must be that commit's own, **T1-confirmed** (its
   TDX quote verified on NEAR), and a **Violation** whose keys include a slot the program prices
   **Critical** now (a round may add slots). The program's anchor must be **confirmed by the light
   client**: on a project-attested anchor a verdict is about state the project chose, and the guard
   does not act on it;
3. only the **first finding** of that key freezes. A held or paid commit is one. A commit not allocated
   yet is one unless `get_holder(program, round, tag)` names another commit. A duplicate, or a match of
   a known issue, never freezes again after the owner unfroze.

If so, the guard freezes and emits `frozen` (the commit, the run, the slot, who reported it). While
frozen, the owner's `transfer` is refused. Only the owner unfreezes: after the fix is released, or if a
challenge overturns the verdict.

The decision is one pure function, `critical_finding(program_id, commit, run, program)`, unit-tested
on its own: a High or Medium slot, a NoViolation, a T0 verdict, an unconfirmed anchor, another program,
a run that is not the commit's own, a duplicate or a known issue never freeze. Two commits that find
the same key before either is allocated can each freeze once (the owner unfreezes the second).

## In the demo

The sandbox demo deploys the guard as a DAO treasury (owner: the Acme project), watching the Acme
program. A transfer goes through; then the keeper reports agent A's commit (a T1-confirmed Critical,
solvency); the guard freezes on slot 0, and the next transfer is refused. `results.json` records it under
`verdict_consumer`.

## On testnet

`guard.antibody.testnet` is the same guard on the locked deployment, owned by `acme.antibody.testnet` and
watching program 5 (`antibody-services live --verdict-consumer`, after the story ran). The owner's first
transfer went through. Then D's commit 16 was reported: it breaks the same Critical key as A, but as a
duplicate it is not a first finding, so `report` returned `false` and nothing froze. A's commit 13 (T1,
Critical, light-client anchor) froze it on slot 0, and the next transfer was refused. Then its full-access
key was deleted: the guard is locked (code `7Wotb7nn…`, in `deployments/near-testnet.json`). The steps and
their transactions are under `verdict_consumer` in `docs/results/testnet-2026-11-04.json`.

```
antibody view --contract guard.antibody.testnet --method get_state
```

The wasm is built with Rust 1.85.0 (`cargo build --release --target wasm32-unknown-unknown` in
`contracts/verdict-guard`) and `wasm-opt -Oz` (binaryen 133): sha256
`60c8fbac0a331d4e07ae557f7d10d2f228d3f8fbdff9aa1ae599928927811798`, NEAR code hash
`7Wotb7nnbSnZnC8Dbx6iSCB785Dojai2HoVq7AqCbsdh`.

## Other uses

The same reads fit any NEAR app that depends on a protocol Antibody watches: a lending market that stops
accepting a bridged asset as collateral, a DAO that blocks a proposal touching the protocol, an
insurance pool that opens claims. Each decides on near-antibody's records, not on anyone's word.

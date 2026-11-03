# A NEAR app consuming verdicts

Antibody's verdicts are NEAR state: any NEAR contract can read them and act. `contracts/verdict-guard`
is the smallest example, a treasury that freezes when the program it watches gets a verified Critical.

## How it decides

Anyone calls `report(commit_id)`. The guard then reads near-antibody itself, by cross-contract view
calls, and trusts the reporter for nothing:

1. `get_commit(commit_id)`: the commit must be to the watched program, with a verification run;
2. `get_run(run)` and `get_program_meta(program)`: the run must be that commit's own, **T1-confirmed**
   (its TDX quote verified on NEAR), and a **Violation** whose keys include a slot the program's
   manifest declares **Critical**.

If so, the guard freezes and emits `frozen` (the commit, the run, the slot, who reported it). While
frozen, the owner's `transfer` is refused. Only the owner unfreezes: after the fix is released, or if a
challenge overturns the verdict.

The decision is one pure function, `critical_slot(program_id, commit, run, meta)`, unit-tested on its
own: a High or Medium slot, a NoViolation, a T0 verdict, another program, or a run that is not the
commit's own never freeze.

## In the demo

The sandbox demo deploys the guard as a DAO treasury (owner: the Acme project), watching the Acme
program. A transfer goes through; then the keeper reports agent A's commit (a T1-confirmed Critical,
solvency); the guard freezes on slot 0, and the next transfer is refused. `results.json` records it under
`verdict_consumer`.

## Other uses

The same reads fit any NEAR app that depends on a protocol Antibody watches: a lending market that stops
accepting a bridged asset as collateral, a DAO that blocks a proposal touching the protocol, an
insurance pool that opens claims. Each decides on near-antibody's records, not on anyone's word.

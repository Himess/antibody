# Rule strength

How many deliberately broken versions of a program's code do its rules catch? `antibody invariants
strength` measures it: it mutates the protocol's sources one line at a time, in labeled ways, and runs the
project's Foundry invariant test (which asserts the program's rules, `check(snapshot()) == 0`) on every
mutant with a fixed seed. The result reads **"caught X of N mutants"**.

It measures the rules and the test's reach. It is not a security score: a mutant the rules miss is not
necessarily a bug anyone can exploit, and the code's other defects are not mutants at all.

## The mutants

| Class | One line changed |
|---|---|
| `missing-access-check` | an `only…` modifier, or a `require`/`if … revert` that compares the caller, is deleted |
| `flipped-comparison` | a comparison in a condition is negated (`<` becomes `>=`, `==` becomes `!=`) |
| `unchecked-transfer` | a token transfer's (or a low-level call's) success check is dropped |
| `dropped-guard` | any other guard (`require`, `if … revert`), such as a health check, is deleted |

A mutant is **caught** when the fuzzer breaks a rule within the budget. A mutant that does not compile,
breaks the test's own setup (the fixture cannot even be deployed with it), errs or times out is listed
but not scored.

## Acme Lend at the final fix

```
antibody invariants strength --project contracts/evm --test AcmeStrengthTest \
  --mutate src/acme/AcmeLend.sol --mutate src/acme/AcmeVault.sol --mutate src/acme/AcmePool.sol \
  --mutate src/acme-fixes/AcmeLendFixed.sol --mutate src/acme-fixes/AcmeFinalFix.sol --runs 512 --depth 100 --seed 1
```

The test (`contracts/evm/test/strength/AcmeStrength.t.sol`) deploys the fixture, installs round 2's code
(the final fix) and asserts the program's rules (`AcmeInvariantsV2`). A handler drives every entry point
as an arbitrary actor: amounts within and beyond its balances, borrowing to the protocol's own limit,
repaying without the funds, swaps large enough to move the price. Its withdrawals are exact, so the
project's declared known issue (share rounding) never fires. The unmutated code holds over 51,200 calls
at three seeds.

**Caught 8 of 28 mutants** (3 more break the fixture's setup and are not scored), in 212 s on four cores.
Report: [`results/rule-strength-acme.json`](results/rule-strength-acme.json).

| Class | Caught |
|---|---|
| missing-access-check | 3 of 4 |
| dropped-guard | 2 of 5 |
| flipped-comparison | 1 of 4 |
| unchecked-transfer | 2 of 15 |

Caught: the fee rescue and the deposit's own-funds check of the final fix (planted bugs #2 and #5 put
back), the treasury's admin check, an unchecked repayment (bug #3's class), the reference price flipped
to the spot price (bug #1's class), an unchecked supply, and the health checks of borrowing and of
withdrawing collateral.

The 20 survivors, and why the rules miss each:
- **The pool** (8): no rule covers AcmePool's reserves, its slippage bound or who adds liquidity.
- **Only the caller loses** (4): an unchecked payout to the caller (a withdrawal, a borrow, a supply
  withdrawal), or a deposit that mints no shares, harms only the caller, and the program protects only
  the saver and the protocol's own solvency, fees and share price.
- **A gap in the rules** (2): the fees rule counts accrued fees as a number. With the deposit's or the
  donation's transfer unchecked, a deposit that never arrives still supplies the lender, from the fee
  tokens the vault holds, and no rule notices. A rule on the vault's actual acUSD would.
- **Nothing changes** (6): acETH reverts on a failed transfer, so its unchecked transfers behave alike;
  the fees always cover their transfer to the treasury; the base sweep is replaced by the final fix's;
  the treasury's call check only matters for a call that fails; the fixed sweep is the admin's alone.

The survivors are where a project would add a rule (a pool invariant, users' withdrawals, the fee tokens) if it wants
them covered. The score says how much of this kind of breakage the rules catch, nothing more.

# Foundry compatibility

Hunters and projects already write Foundry tests. Antibody takes them as they are, within the limits of
what a sealed proof can express.

## A forge-test PoC becomes a bundle

```
antibody from-forge --project <foundry dir> --contract <TestContract> --test <test_fn> \
  --fork-url <rpc> --fork-block <the program's anchor block> --out poc.bundle.json
antibody check --program <id> --bundle poc.bundle.json
```

`from-forge` runs the test once (`forge test -vvvv --json`, forked at the anchor) and reads its execution
trace. Every call and contract creation the test makes **as a pranked sender**, in order, becomes a call
of the bundle. Its senders become the bundle's fresh senders: their addresses are replaced by the
placeholders `5e…5e` and `5f…5f`, and a contract that call k created by `5c…5c<k>`, so the bundle holds
for any salt (`antibody commit` draws a fresh one each time).

Example: `contracts/evm/test/compat/PracticeForgePoc.t.sol`, the practice program's public proof written
as a forge test. `vm.deal(me, 0.01 ether); vm.prank(me, me); vault.sweep(me);` becomes one call to the
vault, `sweep(<sender 0>)`, with a 0.01 ether endowment.

## The cheatcodes a bundle can express

| Allowed | Becomes |
|---|---|
| `prank`, `startPrank`, `stopPrank`, as the test's own senders (at most two; `tx.origin` the sender itself) | the bundle's senders |
| `deal` to those senders | the endowment (the largest deal; gas is paid from it) |
| `expectRevert`, `label`, `addr`, `assume`, `load`, `getNonce`, `recordLogs`, `toString`, `envOr` | nothing: they change no chain state |

Everything else is refused, with the cheatcode's name:
- **pranking as anyone else**, such as the protocol's admin: a third sender, or a `tx.origin` that is not
  the sender;
- **storage writes and code changes** (`store`, `etch`, `mockCall`, `setNonce`);
- **balances for other accounts** (`deal` to anyone but the senders);
- **time travel** (`warp`, `roll`): the verifier runs every call in the anchor block;
- **a call the test makes as itself**, rather than as a sender.

The endowment must stay within the program's `max_endowment_wei`, and the calls within its gas caps;
`antibody check` reports both, as the verifier would.

## Invariant suites

A program's rules are an `IAntibodyInvariants` contract (`snapshot()`, `check(pre)`). The rule strength
test (docs/rule-strength.md) runs them as a Foundry invariant suite: a handler drives every entry point,
and the invariant asserts `check(pre) == 0`. A project with its own Foundry invariant suite writes its
handler the same way; `contracts/evm/test/strength/AcmeStrength.t.sol` is the example.

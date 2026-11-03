# Fork mode

A program normally points at contracts that exist on chain at its anchor block. In **fork mode** they
don't need to: the manifest commits the transactions that deploy them, and the verifier runs those
transactions inside its own copy of the chain, on top of the anchor's state, before anything else. A
project can open a program on code it has not deployed anywhere, against the real state it will meet
(tokens, oracles, pools at the anchor), with no staging deployment.

## The manifest

```json
"deploy": {"txs": [
  {"data": "0x<init code + constructor arguments>", "value": "0x<wei>"},
  {"to": "0x<a contract>", "data": "0x<calldata>"}
]}
```

- The transactions come from the **deployer**, `keccak256("antibody/deployer/v1")[12..]`: a hash, nobody's
  key. Its account is never read at the anchor (it is fresh by construction), and its transaction i
  creates at `deployer.create(i)`.
- They run in order, in the anchor block, each with the block's gas limit (at most 2^24, EIP-7825's cap)
  at the base fee. The deployer is funded with exactly what the transactions need.
- A transaction that reverts makes the program `ProgramMisconfigured`, as any other configuration error.
- At most 64 transactions, each within EIP-3860's init code limit.

Everything after the deployment is as for any program. The **targets** are the deployed contracts, with
the code hashes they have once deployed. The **rules** run on the deployed state: `snapshot()` and
`check(snapshot()) == 0` at the anchor, then every proof. **Rounds** replace deployed code with their
overrides, and **releases** read the deployed proxies.

## Witnesses

The base witness proves what the deployment reads at the anchor (the accounts its constructors call,
the addresses it creates at, which must be empty) along with what the rules read. A contract the
deployment created has only the storage the deployment wrote, so its other slots are zero without
asking the anchor. A proof that touches only the deployed contracts and the base's state is sealed
against the base alone, with no RPC (docs/spec.md §4.4).

## Writing one

`antibody_witness::program::deployed_code` (and `cmd::deployed_code` in the CLI library) run a manifest's
deployment at its anchor from an RPC and return the code left at any address: the targets' code
hashes, and the rules' runtime code taken from an instance the deployment creates (immutables
included).

The sandbox demo opens the practice program in fork mode (`fork_practice_manifest` in
crates/services/src/demo.rs): two transactions deploy the factory with its 24 vaults and the rule; the
factory and the vaults exist only inside the verifier (`eth_getCode` at a vault is `0x`), and the
tutorial proof against vault 3 is verified, T1-confirmed and paid like on the on-chain program.

## Limits

- No flash-capital facility yet: a proof's capital is its senders' endowment and what the protocol
  itself lends.
- The deployment is the program's: a round cannot change it, only override the deployed code.

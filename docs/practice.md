# The practice program (a tutorial)

The practice program lets anyone run Antibody's whole loop on testnet in a few minutes: commit a sealed
proof, have it verified, and see it paid. Its target is a **tutorial**: 24 small vaults with one
textbook bug, published here with its proof. Nothing here is a real vulnerability, and the practice
program is counted apart from real programs.

## Try it

```
antibody --network testnet try --vending practice.antibody.testnet --program <id> --name <you>
```

No NEAR account, no key, no API key and no Ethereum RPC are needed. The command:
1. vends `<you>.practice.antibody.testnet` for a key it generates locally (`.antibody/practice/<you>/`);
2. builds the tutorial proof against the vault assigned to you from the program's pinned base witness
   (it already covers the tutorial proof, for every vault), seals it to the program's key, commits, and
   reveals it;
3. waits for the verdict and its T1 confirmation;
4. calls `allocate` and `pay` (both permissionless), then closes the practice account (`close`, then the
   vending contract's `reclaim`): its NEAR goes back to the vending contract. It prints the state
   (`Done:Paid`) with a link for every transaction.

## The target

`contracts/evm/src/practice/Practice.sol`:
- `PracticeVault` holds a little ether for its owner. Its `sweep(to)` has **no access check**, so anyone
  can send the vault's ether anywhere. That is the tutorial bug.
- `PracticeFactory` deploys 24 vaults at CREATE2 addresses anyone can compute (`vaultAt(i)`).
- `PracticeInvariants` is the program's one rule, as code: bit 0, Low, *no practice vault loses its ether*.

Each vault is a separate target, so a proof against vault 3 and a proof against vault 7 are different
findings. Each practice account is assigned its own vault.

## The proof

One call, from your sender, to your vault:

```json
{"senders": 1, "endowment_wei": "1000000000000000", "protected": [],
 "calls": [{"sender": 0, "to": "<your vault>", "value": "0",
            "data": "0x01681a62000000000000000000000000000000000000000000000000000000000000dead",
            "gas_limit": 100000}]}
```

`0x01681a62` is `sweep(address)`, and the argument is `0x…dEaD`. At the anchor the vault loses its ether,
so `vault_keeps_its_ether` breaks: a Violation keyed to your vault and `sweep`.

## Practice accounts

`contracts/practice-vending` creates `<name>.practice.<root>.testnet` for a key you hold:
- a little NEAR for its storage and gas, the code of `contracts/practice-account`, and one function-call
  key for that account itself. Its contract passes `commit`, `reveal` and `retry` and the permissionless
  `allocate` and `pay` on to near-antibody, for that key only, and `close` tells the vending contract
  the try is done. The key cannot withdraw or transfer, and a proof it sends travels inline (at most
  15 KB sealed);
- registered as an **agent** of the vending contract: near-antibody takes its bonds (within a small
  cap), its verification runs and its storage from the vending contract's credit, and its payouts go
  back to that credit;
- its own vault (`vault_of`).

When a try is done, the practice key calls `close`, and the public vend key calls the vending contract's
`reclaim`, which deletes the practice account: its NEAR goes back to the vending contract, what its gas
did not burn and its storage stake. A try costs the vending contract the gas it burned, not the gas
funds. (The deletion is not the practice account's own transaction: an account deleted in a
transaction it signed loses the gas that transaction set aside.) If a try stops before `close`, anyone
may call `reclaim` an hour after the vend.

Anyone may call `vend`. So that a first try needs no account, the operator adds a **public vend key**
to the vending account: a function-call key for `vend` and `reclaim` only, within an allowance. Its
secret is derived from a published seed (`practice::PUBLIC_VEND_SEED`); it is deliberately public and can
do nothing else.

## Operating it

```
# on the vending account: deploy, initialise, and add the public vend key
antibody practice deploy --account practice.antibody.testnet --wasm antibody_practice_vending.wasm \
  --account-wasm antibody_practice_account.wasm \
  --owner antibody.testnet --program <id> --bond-budget <bond> --per-commit-cap <bond>
# as the owner: NEAR credit at near-antibody (runs, storage) and tokens for the bonds
antibody practice fund --account antibody.testnet --vending practice.antibody.testnet --near 0.5 --tokens <n>
```

The practice program's base witness is built with `antibody program activate --cover <bundle>`: the base
then also holds the state the tutorial proof reads (the `0x…dEaD` account it sweeps to), so every vault's
proof seals against the pinned base alone.

The sandbox demo (`make demo`) runs all of this: it opens the practice program, deploys and funds the
vending contract, and two judges with no account try it.

## Limits

- Practice payouts are small and go to the vending contract, which paid the bonds and the runs.
- With 24 vaults, the 25th account in a round gets a vault someone already proved: its proof is a
  duplicate, as on any program.
- Vending is limited per hour, and the public vend key's allowance caps the gas that calls to `vend`
  can spend; the operator refills both.
- A practice account is an agent bound to the practice program: it cannot commit anywhere else. Its
  cap is the practice bond, and its payouts go to the vending contract.
- The proof is public. The point is the loop (sealing, verification, payment), not the finding.

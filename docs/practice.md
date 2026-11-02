# The practice program (a tutorial)

The practice program lets anyone run Antibody's whole loop on testnet in a few minutes: commit a sealed
proof, have it verified, and see it paid. Its target is a **tutorial**: 24 small vaults with one
textbook bug, published here with its proof. Nothing here is a real vulnerability, and the practice
program is counted apart from real programs.

## Try it

```
antibody --network testnet try --vending practice.antibody.testnet --program <id> --name <you>
```

No NEAR account, no key and no API key are needed. The command:
1. vends `<you>.practice.antibody.testnet` for a key it generates locally (`.antibody/practice/<you>/`);
2. builds the tutorial proof against the vault assigned to you, seals it to the program's key, commits,
   and reveals it;
3. waits for the verdict and its T1 confirmation;
4. calls `allocate` and `pay` (both permissionless) and prints the state (`Done:Paid`) with a link for
   every transaction.

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
- a little NEAR for gas, and one function-call key that may only call near-antibody's `commit`, `reveal`
  and `retry`, the permissionless `allocate` and `pay`, and FastFS uploads. It cannot withdraw or
  transfer;
- registered as an **agent** of the vending contract: near-antibody takes its bonds (within a small
  cap), its verification runs and its storage from the vending contract's credit, and its payouts go
  back to that credit;
- its own vault (`vault_of`).

Anyone may call `vend`. So that a first try needs no account, the operator adds a **public vend key**
to the vending account: a function-call key for `vend` only, within an allowance. Its secret is derived
from a published seed (`practice::PUBLIC_VEND_SEED`); it is deliberately public and can do nothing else.

## Operating it

```
# on the vending account: deploy, initialise, and add the public vend key
antibody practice deploy --account practice.antibody.testnet --wasm antibody_practice_vending.wasm \
  --owner antibody.testnet --program <id> --bond-budget <bond> --per-commit-cap <bond>
# as the owner: NEAR credit at near-antibody (runs, storage) and tokens for the bonds
antibody practice fund --account antibody.testnet --vending practice.antibody.testnet --near 0.5 --tokens <n>
```

The sandbox demo (`make demo`) runs all of this: it opens the practice program, deploys and funds the
vending contract, and two judges with no account try it.

## Limits

- Practice payouts are small and go to the vending contract, which paid the bonds and the runs.
- With 24 vaults, the 25th account in a round gets a vault someone already proved: its proof is a
  duplicate, as on any program.
- Vending is limited per hour, and the public vend key's allowance caps the gas that calls to `vend`
  can spend; the operator refills both.
- A practice account is an agent on any program whose bond fits its cap. The cap is the practice bond,
  and its payouts go to the vending contract, so a practice key gains nothing elsewhere.
- The proof is public. The point is the loop (sealing, verification, payment), not the finding.

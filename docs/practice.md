# The practice program (a tutorial)

The practice program lets anyone run Antibody's whole loop on testnet in a few minutes: commit a sealed
proof, have it verified, and get paid. Its target is a **tutorial**: 24 small vaults with one textbook
bug, published here with its proof. Nothing here is a real vulnerability, and the practice program is
counted apart from real programs.

## The target

`contracts/evm/src/practice/Practice.sol`:
- `PracticeVault` holds a little ether for its owner. Its `sweep(to)` has **no access check**, so anyone
  can send the vault's ether anywhere. That is the tutorial bug.
- `PracticeFactory` deploys 24 vaults at CREATE2 addresses anyone can compute (`vaultAt(i)`).
- `PracticeInvariants` is the program's one rule, as code: bit 0, Low, *no practice vault loses its ether*.

Each vault is a separate target, so a proof against vault 3 and a proof against vault 7 are different
findings. Each practice account is assigned its own vault and is paid for it.

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
  and `retry` (and upload with FastFS). It cannot withdraw or transfer;
- NEAR credit at near-antibody for its verification runs;
- registered as an **agent** of the vending contract: its bonds come from the contract's credit within a
  small cap, and its payouts go back to the contract;
- its own vault (`vault_of`).

Vending is limited per hour and the amounts are small; the operator refills it.

## Limits

- Practice payouts are small and go to the vending contract, which pays for the bonds.
- With 24 vaults, the 25th account in a round gets a vault someone already proved: its proof is a
  duplicate, as on any program.
- The proof is public. The point is the loop (sealing, verification, payment), not the finding.

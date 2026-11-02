# Project quickstart

Open a program before a launch or an upgrade. Your invariants are your bounty terms: a submission is paid
only if it breaks one of them, at the severity you committed.

## 1. Write invariants

Implement `IAntibodyInvariants` (`contracts/evm/src/antibody/IAntibodyInvariants.sol`) as a pure view
contract. Point it at your contracts with immutables, and put the reference prices it needs into immutables
too. The verifier injects its runtime code at a reserved address in its own fork; it is never deployed.

- `snapshot()` returns what `check()` compares against, read at the anchor.
- `check(pre)` returns a bitmask: bit *i* = invariant *i* broken.

`contracts/evm/src/acme/AcmeInvariants.sol` is a complete example (solvency at reference prices, fees never
decrease, share price never falls).

To start from a draft, run `antibody invariants draft --source <your contracts> --out draft/`. Add `--ai`
for Claude's extension. Review the draft, edit it and approve it (docs/invariants-drafting.md).

## 2. Deploy staging and pick an anchor

1. Deploy your contracts to Sepolia (and, if you want, pause-only copies elsewhere).
2. Grant `PAUSER` to your guardian's EVM address (`scripts/guardian/derive.mjs` derives it from the guardian contract and path).
3. Pick a finalized block `M` after the deployment: the anchor.
4. Create the program within about 7 days of `M`. Then NEAR's testnet light client can confirm `M`'s hash on-chain; otherwise the program is labelled project-attested.

## 3. Commit the rules

Write `antibody.toml`; `fixtures/acme-local/antibody.toml` is a complete example. It contains:
- the anchor;
- the targets with their code hashes;
- the invariants runtime code and its hash;
- the severity per invariant;
- the impact config;
- the attacker capabilities;
- the payout table, bond, reveal window, deadline and disclosure delay.

Then:

```bash
antibody manifest antibody.toml --out manifest.json           # prints its sha256: the committed hash
antibody keygen x25519 --out project.x25519.json              # findings are re-encrypted to this key; keep it secret
antibody --account acme.testnet program create --manifest manifest.json --escrow 25000 \
         --project-key project.x25519.json --guardian guardian.antibody-hq.testnet
```

`program create` uploads the manifest, escrows the tUSD and commits the manifest hash in one transaction.
For a Sepolia anchor, near-antibody checks the anchor hash against the light client right away, and
refunds the escrow if they differ.

## 4. Declare known issues, then open

```bash
antibody --account acme.testnet program known --program <id> --bundle known-issue.json   # same TEE verification
antibody --account acme.testnet program open --program <id>
```

Known issues claim their keys at t0. Nothing can be added after opening, and nothing can be claimed without a
PoC that verifies.

## 5. Read and reward findings

```bash
antibody --account acme.testnet decrypt --commit <id> --project-key project.x25519.json --out finding-<id>/
antibody --account acme.testnet top-up --commit <id> --amount 500    # pay more, never less
```

You can decrypt any Violation; non-violations are never disclosed to you before disclosure time.

## 6. Ship a fix (rounds)

When you fix a bug, commit the fix as a code override at the same anchor. Then a second, different bug
in the same function can be found and paid (docs/rounds.md).

```bash
antibody manifest-round --base manifest.json --round 1 \
         --override <target address>=<fixed runtime code hex file> --out round1.json
antibody --account acme.testnet program round-propose --program <id> --manifest round1.json
antibody --account acme.testnet program round-rerun --commit <id> --project-key project.x25519.json   # each claimed finding
antibody --account acme.testnet program round-open --program <id>
```

- **What the round may change.** Only target code. The contract rejects any other change to the
  committed manifest.
- **New commits** verify against the fix as soon as you propose it.
- **Re-runs.** Each re-run executes the finder's own PoC against the fix in the TEE. Keys whose PoCs no
  longer break are released when the round opens; the rest stay with their finders.

## 7. Close

```bash
antibody program disclose --program <id>           # anyone, after deadline + disclosure delay
antibody --account acme.testnet program withdraw-escrow --program <id>
```

Disclosure makes every content key and `k_program` public, so anyone can re-verify every receipt and every
duplicate decision. Escrow stays locked while any commit is pending.

# Project quickstart

> **Testnet status.** The testnet deployment is not live yet. The whole flow can be rehearsed on the
> local sandbox (`antibody-services demo --serve`); `antibody init --out my-program --project <you>.testnet`
> scaffolds a workspace.

Open a program before a launch or an upgrade, next to your existing bounty. Your rules are your terms: a
proof is paid only if it breaks one of them, at the severity you committed, from an escrow you fund.

## 1. Write the rules

Implement `IAntibodyInvariants` (`contracts/evm/src/antibody/IAntibodyInvariants.sol`) as a view
contract: `snapshot()` reads what `check(pre)` compares against, and `check` returns a bitmask (bit i =
rule i broken). Point it at your contracts with immutables. The verifier serves its runtime code at a
reserved address; it is never deployed, and it has no storage. `contracts/evm/src/acme/AcmeInvariants.sol`
is a complete example.

Other ways in:
- **Foundry-style:** write `invariant_*()` view functions and generate the rules contract with
  `antibody invariants from-foundry` (docs/foundry.md).
- **A draft:** `antibody invariants draft --source <your contracts> --out draft/` (add `--ai` for a model's
  extension), then review it (docs/invariants-drafting.md).

## 2. Choose where it runs

- **On chain:** deploy to Sepolia (pause-only copies elsewhere if you want) and pick a finalized block
  after the deployment as the anchor. Create the program while the light client still confirms it
  (about a week); otherwise it is labelled project-attested.
- **Fork mode:** commit your contracts' deployment in the manifest instead. The verifier deploys them on
  top of the anchor's real state, so no staging deployment is needed (docs/fork-mode.md).

For pausing and releases, put your proxies behind the program's `AntibodyAdapter` (one per chain, at the
same CREATE2 address) and register it with near-guardian.

## 3. Commit the terms, and check them

`fixtures/acme-local/antibody.toml` is a complete example: the anchor, the targets and their code hashes,
the rules' code and the severity of each slot, the impact prices, the proofs' capabilities, and the
settlement terms (payouts, bond, instant cap and budget, windows, the gap lane, known issues, the pause
policy).

```bash
antibody manifest antibody.toml --out manifest.json         # prints its sha256: the committed hash
antibody invariants validate --manifest manifest.json      # as creation and activation will check it; nothing sent
antibody invariants strength --project <foundry dir> --test <your strength test> --mutate src/X.sol   # optional
antibody keygen x25519 --out project.x25519.json            # findings are re-encrypted to it; keep it secret
antibody --account you.testnet program create --manifest manifest.json --escrow <tokens> --project-key project.x25519.json
```

`program create` uploads the manifest with FastFS, escrows the tokens and commits the manifest hash;
near-antibody checks the anchor against the light client.

## 4. Activate, declare known issues, open

```bash
antibody --account you.testnet program activate --program <id>   # pins the base witness; the TEE checks the rules hold at the anchor
antibody --account you.testnet program known --program <id> --bundle known-issue.json   # sealed, verified like any proof
antibody --account you.testnet program open --program <id>
```

Known issues count only once T1-confirmed, and nothing is added after opening.

## 5. Findings

A Violation's content key is wrapped to your project key, so you can read what you pay for;
`antibody --account you.testnet program show --program <id>` lists every commit with its verdict.
`program top-up` adds escrow. A Critical in your pause policy makes near-guardian sign pause permits for
every registered chain; anyone relays them.

## 6. Ship a fix (rounds)

Commit the fix as code at the same anchor; near-antibody has the TEE check it, every paid proof is
re-run against it, and the release gate decides what may ship (docs/rounds.md).

```bash
antibody manifest-round --base manifest.json --round 1 --override <target>=<fixed runtime code hex> \
  [--invariants-code <new rules> --add-slot <bit>:<name>:<severity>] [--rule-strength strength.json] --out round1.json
antibody --account you.testnet round propose --program <id> --manifest round1.json --release <proxy>:<implementation>:<code hash>
antibody round rerun --commit <id>       # anyone; the keeper bot re-runs every eligible commit
antibody round open --program <id>       # once the round's windows pass
```

- **What a round may change:** target code, and new slots appended to the rules. Everything else stays
  byte-identical.
- **The release gate:** every paid proof re-run against the fix and failing, the canaries passing, no
  Critical or High key open. Then near-guardian signs a release permit per proxy and chain, for exactly
  the code the re-runs ran.

## 7. Close

```bash
antibody --account you.testnet program exit --program <id>   # after the deadline and its grace
```

The rest of the escrow returns to you; nothing in flight is left behind (property 6).

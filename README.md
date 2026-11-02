# Antibody — bug bounties that pay on proof

> Proof, not reports. Your invariants are your bounty terms.

Antibody is a bug-bounty settlement layer on NEAR. A project opens a program before a launch or an
upgrade and commits, immutably, to:

- a staging deployment and an anchor block on a testnet;
- its rules as code: an invariants contract and a severity/payout table;
- an escrowed reward pool.

Hunters (people or AI agents) submit sealed proofs of concept. A deterministic verifier re-executes each
PoC on the proven anchor state, inside an OutLayer TEE, and reports which committed invariants broke,
where, and the measured impact. The NEAR contract then applies the committed rules:

- the first valid finder is paid;
- duplicates are shown verifiable proof of the earlier claim;
- known issues, bonds and disclosure are handled;
- a Critical triggers a Chain Signatures pause on every chain where the code runs.

No LLM and no human decides who gets paid.

**Status: work in progress (backend).** This README grows as each part lands, and every claim it makes is
backed by a test, a measurement or a transaction link.

## What works today (milestone M0)

- **Program-mode verifier** (`crates/verifier`). It:
  1. binds the witness to the program manifest and the anchor header;
  2. verifies every state proof;
  3. injects the program's invariants at a reserved address, seen only by evaluation calls;
  4. requires `check(snapshot()) == 0` at the anchor;
  5. runs the PoC from fresh salted senders with a bounded endowment;
  6. records, for each broken invariant, the protocol-entry call where it last flipped from holding to
     broken (its root-cause fingerprint).
- **Recorded fixtures** (`fixtures/acme-local/`): six scripted submissions plus the project's known
  issue, against the fictional "Acme Lend" protocol. The verdicts, keys, fingerprint tags and receipt
  hashes are in `expected.json`. CI re-checks them, natively and as `wasm32-wasip2` under wasmtime with
  OutLayer's engine configuration, and requires byte-identical output.

| Submission | Verdict | Key (invariant, function) |
|---|---|---|
| A: spot-price collateral via a swap | Violation | solvency, `AcmeLend.borrow` |
| B: `sweepFees` without access control | Violation | fees_to_treasury, `AcmeVault.sweepFees` |
| C: profitable arbitrage plus a prompt-injected report | NoViolation | none |
| D: A's bug via donation + `sync()` | Violation | solvency, `AcmeLend.borrow` (same key as A) |
| E: withdraw rounding | Violation | share_price, `AcmeVault.withdraw` (the project's known issue) |
| F: `repay` ignores `transferFrom`'s return | Violation | solvency, `AcmeLend.repay` (new key, same rule) |

Reproduce:

```bash
cargo build --release -p antibody-verifier -p antibody-witness -p antibody-runner
cargo build --release --target wasm32-wasip2 -p antibody-verifier --bin antibody-verify
python3 scripts/demo/check_fixtures.py fixtures/acme-local
cd contracts/evm && forge test
```

## Repository layout

| Path | Contents |
|---|---|
| `crates/verifier` | Deterministic verifier: replay, block, bundle and program modes; receipts and facts |
| `crates/witness` | Host-side witness builder: RPC quorum, trace → proofs → strict fixpoint, manifest compiler |
| `crates/runner` | wasmtime host with OutLayer's engine config, for local wasm runs |
| `contracts/evm` | The Acme Lend fixture (deliberately vulnerable, testnet only), invariants, PoCs, deploy script |
| `contracts/near-guardian` | NEAR contract that signs EVM `pause()` transactions through Chain Signatures |
| `fixtures/` | Recorded witnesses, manifests and expected verdicts |
| `scripts/` | Fixture pipeline and measurement scripts |

## Safety

`contracts/evm/src/acme/` is a **deliberately vulnerable test fixture**: a fictional protocol with
bugs planted on purpose. Never deploy it to mainnet. Everything runs on testnets or local chains. See
[SECURITY.md](SECURITY.md).

## License

MIT (see [LICENSE](LICENSE)). Third-party code is used under its own licenses. `third_party/patches/`
holds a build-script fix for `kzg-rs` (MIT); see its README.

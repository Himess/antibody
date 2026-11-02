# Self-attack regressions

We attack our own verifier. Each case here is an input that once broke it, reproduced against the
fictional Acme Lend fixture, fixed, and pinned in CI by `scripts/demo/check_regressions.py` (native and
wasm builds, verdicts and root-cause keys). `*.before-fix.json` keeps the receipt the old code produced.

| Id | Severity | What broke | Fix | Pinned result |
|---|---|---|---|---|
| SA-1 | Critical | **Fingerprint steering.** Evaluations recorded inside a frame that later reverted stayed on the timeline, so one bug could be re-keyed to any (target, selector): the known issue, broken again and followed by a call with selector `0xdeadbeef`, became a new paid key | A frame that fails truncates the evaluation timeline back to its start; reverted entry calls are not evaluated; the entry-call cap counts evaluations separately (`crates/verifier/src/program.rs`) | `steering.witness.bin` (PoC: `contracts/evm/test/regressions/PocSteer.sol`) keeps the key `(share_price, AcmeVault.withdraw)`, the same as the known issue |
| SA-2 | Critical | **Tampered jump tables.** revm's serde accepted a witness's jump table as given, and the code hash covers only the original bytes. A hunter could mark every `JUMPDEST` invalid, make the anchor snapshot fail, and so get `ProgramMisconfigured`, which closes the program | Every bytecode is re-analysed from its hashed bytes before use (`crates/verifier/src/run.rs`) | `jump-table-{KNOWN,B}.witness.bin` (made with `antibody-witness tamper --kind jump-table`) give the honest verdicts and keys |

Both fixes left every honest fixture byte-identical, receipt hashes included.

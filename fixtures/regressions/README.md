# Self-attack regressions

We attack our own verifier. Each case here is an input that once broke it, reproduced against the
fictional Acme Lend fixture, fixed, and pinned in CI by `scripts/demo/check_regressions.py` (native and
wasm builds, verdicts and root-cause keys). `*.before-fix.json` keeps the receipt the old code produced.

| Id | Severity | What broke | Fix | Pinned result |
|---|---|---|---|---|
| SA-1 | Critical | **Fingerprint steering.** Evaluations recorded inside a frame that later reverted stayed on the timeline, so one bug could be re-keyed to any (target, selector): the known issue, broken again and followed by a call with selector `0xdeadbeef`, became a new paid key | A frame that fails truncates the evaluation timeline back to its start; reverted entry calls are not evaluated; the entry-call cap counts evaluations separately (`crates/verifier/src/program.rs`) | `steering.witness.bin` (PoC: `contracts/evm/test/regressions/PocSteer.sol`) keeps the key `(share_price, AcmeVault.withdraw)`, the same as the known issue |
| SA-2 | Critical | **Tampered jump tables.** revm's serde accepted a witness's jump table as given, and the code hash covers only the original bytes. A hunter could mark every `JUMPDEST` invalid, make the anchor snapshot fail, and so get `ProgramMisconfigured`, which closes the program | Every bytecode is re-analysed from its hashed bytes before use (`crates/verifier/src/run.rs`) | `jump-table-{KNOWN,B}.witness.bin` (made with `antibody-witness tamper --kind jump-table`) give the honest verdicts and keys |
| SA-3 | Critical | **Relabelled trie nodes.** rsp's partial trie stores each node's kind as given, and a leaf whose value is 32 bytes encodes like an extension pointing at a hashed branch, so the state root stays the same. Lookups never check the hex-prefix flag that tells the two apart, so a witness could relabel an extension as a leaf and make every key below it read as absent: forged zero state | Every trie node is validated before use: a leaf must carry a leaf flag and an extension an extension flag pointing at a branch; branches have at least two children (`crates/verifier/src/trie.rs`) | `trie::tests::relabelled_extension_is_refused`; every fixture witness passes the validator |

The inputs are judged under the acme-local fixture's manifest, so `scripts/demo/build_regressions.py`
rebuilds them whenever that fixture is regenerated (the steering PoC against the fixture's addresses; the
jump-table cases from its KNOWN and B witnesses).

SA-1 and SA-2 left every honest fixture byte-identical, receipt hashes included. The v2 verifier then
regenerated the fixtures (per-program reserved address, facts v2), with the same verdicts, keys and impacts.

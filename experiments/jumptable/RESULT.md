# R3 experiment E2: tampered jump table (input integrity)

**Setup.** A new `antibody-witness tamper --kind jump-table` (program witnesses) rebuilds the largest
bytecode with `Bytecode::new_analyzed` from the same bytes, with every JUMPDEST marked invalid. The code
hash is unchanged, because `hash_slow()` covers only the original bytes. revm-bytecode 10.0.0's serde accepts
the table as given (`serde_impl.rs:33-52`).

**Before (main, `3ebdeb6`).** The KNOWN and B fixture witnesses, tampered, give `ProgramMisconfigured:
snapshot() failed at the anchor`. In near-antibody that sets `program.misconfigured = true`, and every later
commit is refused. A hunter can therefore close any program for the cost of one bond, which is refunded on
`ProgramMisconfigured`.

**Fix** (`crates/verifier/src/run.rs`, program mode). Every bytecode is re-analysed with
`Bytecode::new_raw(original_bytes)` before it is used. The same change is needed in the replay and bundle
modes.

**After.** The tampered KNOWN and B witnesses give their honest Violation verdicts and fp_tags. (Their
receipt hash differs only through `witness_sha256`, a hash of the witness bytes.) All 7 honest fixtures
(KNOWN, A–F) stay byte-identical, receipt hashes included, with both fixes (steering and re-analysis)
applied.

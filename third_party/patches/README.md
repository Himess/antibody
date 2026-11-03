# Build patches

## kzg-rs 0.2.9

A copy of [`kzg-rs` 0.2.9](https://crates.io/crates/kzg-rs) (MIT, © Bhargav Annem; see `kzg-rs-0.2.9/LICENSE`),
applied through `[patch.crates-io]` in the workspace `Cargo.toml`.

- **Change:** only `build.rs`. Its trusted-setup conversion now runs on a thread with a 64 MiB stack.
- **Why:** the 1 MiB main-thread stack of Windows build scripts overflows (`STATUS_STACK_BUFFER_OVERRUN`).
- **Logic and generated data:** unchanged.
- **Omitted here:** the upstream `tests/` directory. It is not needed to build the crate (`autotests = false`).

## near-vm-runner 0.29.2 (test-only)

A copy of [`near-vm-runner` 0.29.2](https://crates.io/crates/near-vm-runner) (MIT OR Apache-2.0, © NEAR Inc.;
see `near-vm-runner-0.29.2/LICENSE-*`). The NEAR contract workspaces apply it through `[patch.crates-io]`.

- **Who uses it:** only near-sdk's `unit-testing` feature, i.e. contract unit tests. Contract wasm builds never link it.
- **Change:** `FilesystemContractRuntimeCache` and its re-export are gated behind `cfg(unix)`. They use `rustix::fs`, which does not exist on Windows. Behaviour on Unix is unchanged.
- **Omitted here:** the test-only `src/tests` and `src/logic/tests` directories.

## bitvec 1.0.1

A copy of [`bitvec` 1.0.1](https://crates.io/crates/bitvec) (MIT, © myrrlyn; see `bitvec-1.0.1/LICENSE.txt`),
applied through `[patch.crates-io]` in the workspace `Cargo.toml`.

- **Change:** only `src/serdes/slice.rs`, the `BitVec` deserializer. A bit length larger than the
  deserialized storage now returns an error; upstream calls `set_len`, which asserts and panics, and
  maps every assembly error to `todo!()`, also a panic.
- **Why:** revm's jump tables are `BitVec`s inside the witnesses the verifier decodes from untrusted
  input. The parser fuzz test (`a_mutated_witness_is_refused_or_decodes`) found a mutated witness that
  panicked while decoding; in wasm a panic is a trap, not a typed `InvalidPoC`.
- **Everything else:** unchanged, including the serialized format.

## revm-bytecode 10.0.0

A copy of [`revm-bytecode` 10.0.0](https://crates.io/crates/revm-bytecode) (MIT, © Dragan Rakita; see
`revm-bytecode-10.0.0/LICENSE`), applied through `[patch.crates-io]` in the workspace `Cargo.toml`.

- **Change:** only `src/bytecode/serde_impl.rs`. Deserializing an analyzed bytecode now checks the
  invariants `Bytecode::new_analyzed` asserts (`original_len` within the bytes and the jump table) and
  returns an error when they fail; upstream builds the value unchecked, and a later
  `original_bytes()` slices out of bounds and panics.
- **Why:** witnesses carry bytecodes the verifier decodes from untrusted input; the parser fuzz test
  (`the_verifier_answers_any_mutated_program_witness`) found the panic.
- **Everything else:** unchanged, including the serialized format.

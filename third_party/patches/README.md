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

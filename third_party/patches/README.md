# Build patches

## kzg-rs 0.2.9

A copy of [`kzg-rs` 0.2.9](https://crates.io/crates/kzg-rs) (MIT, © Bhargav Annem; see `kzg-rs-0.2.9/LICENSE`),
applied through `[patch.crates-io]` in the workspace `Cargo.toml`.

- **Change:** only `build.rs`. Its trusted-setup conversion now runs on a thread with a 64 MiB stack.
- **Why:** the 1 MiB main-thread stack of Windows build scripts overflows (`STATUS_STACK_BUFFER_OVERRUN`).
- **Logic and generated data:** unchanged.
- **Omitted here:** the upstream `tests/` directory. It is not needed to build the crate (`autotests = false`).

//! Verifier v2's NEAR engine (docs/near-targets.md): NEAR contracts (wasm) executed deterministically
//! under wasmi, with NEAR's host functions, receipts, accounts and fee schedule for protocol 74.
//!
//! The engine is a total function of its inputs (the manifest bytes, the code, the bundle): it reads no
//! clock, no randomness and no environment, iterates no hash map into its output, and uses no floats.

pub mod config;
pub mod crypto;
pub mod exec;
pub mod gas;
pub mod logic;
pub mod manifest;
pub mod prepare;
pub mod receipt;
pub mod rules;
pub mod run;
pub mod runtime;
pub mod state;
pub mod util;
pub mod witness;

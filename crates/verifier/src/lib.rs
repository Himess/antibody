//! Antibody verifier: binds a witness to a claimed Ethereum block hash, verifies all state proofs
//! (reusing rsp), re-executes with revm/reth, and emits a deterministic receipt with measured impact.
//! No network access: the same code runs natively, under wasmtime, and inside an OutLayer TEE.

pub mod bundle;
pub mod db;
pub mod facts;
pub mod impact;
pub mod manifest;
pub mod program;
pub mod receipt;
pub mod run;
pub mod witness;

pub use run::{
    execute_program_witness, execute_replay, run, run_program, verify_only, ProgramContext, ReplayExec, RunOptions,
    VerifyError,
};
pub use witness::{
    BundleWitness, Envelope, Payload, PriceFeed, ProgramWitness, QuorumVote, ReplayWitness, WITNESS_VERSION,
};

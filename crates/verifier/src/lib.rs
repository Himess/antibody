//! Antibody verifier: binds a witness to a claimed Ethereum block hash, verifies all state proofs
//! (reusing rsp), re-executes with revm/reth, and emits a deterministic receipt with measured impact.
//! No network access: the same code runs natively, under wasmtime, and inside an OutLayer TEE.

pub mod bundle;
pub mod db;
pub mod facts;
pub mod fastfs;
pub mod impact;
pub mod keys;
pub mod lint;
pub mod manifest;
pub mod program;
pub mod receipt;
pub mod run;
pub mod sealed;
pub mod tee;
pub mod trie;
pub mod witness;

pub use run::{
    execute_program_witness, execute_replay, run, run_program, verify_only, ProgramContext, ReplayExec, RunOptions,
    VerifyError,
};
pub use witness::{
    BaseWitness, BundleWitness, Envelope, Payload, PriceFeed, ProgramWitness, QuorumVote, ReplayWitness, SubmissionWitness,
    WITNESS_VERSION,
};

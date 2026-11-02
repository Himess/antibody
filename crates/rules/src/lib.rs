//! Antibody's settlement rules as a pure state machine (docs/spec.md §3–§14).
//!
//! The contract (`contracts/near-antibody`) keeps the state in NEAR storage through [`store::Store`],
//! calls one [`engine::Engine`] method per contract method or callback, and turns the returned
//! [`engine::Effect`]s into promises. Nothing here touches NEAR: the same code runs in the contract, in
//! the services and in the tests, which replay the traces of `formal/antibody.qnt`.
//!
//! Money is conserved at every step; [`audit`] returns both sides of every equation of spec §7.10.

pub mod audit;
pub mod engine;
pub mod facts;
pub mod settlement;
pub mod state;
pub mod store;
pub mod types;

pub use engine::{Ctx, Effect, Engine, Error};
pub use facts::{Facts, FactsSummary};
pub use state::*;
pub use store::{IdKind, MemStore, Store};
pub use types::*;

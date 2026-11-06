//! SP1 program (docs/zk-feasibility.md): re-runs one disclosed verification run with the verifier core
//! the TEE ran and commits the public values a NEAR contract compares with near-antibody's records.
//!
//! Input, five byte strings in order: the request near-antibody sent to OutLayer, the program's base
//! witness, the round's manifest (empty for round 0), the sealed blob (empty when the request carries
//! it inline), the 32-byte content key. A run that does not reach a verdict panics: no proof.

#![no_main]
sp1_zkvm::entrypoint!(main);

use antibody_zk_core::{rerun, Disclosed};

pub fn main() {
    let request = sp1_zkvm::io::read_vec();
    let base = sp1_zkvm::io::read_vec();
    let round_manifest = sp1_zkvm::io::read_vec();
    let blob = sp1_zkvm::io::read_vec();
    let key = sp1_zkvm::io::read_vec();
    let content_key: [u8; 32] = key.try_into().expect("a 32-byte content key");
    let d = Disclosed {
        request: &request,
        base: &base,
        round_manifest: (!round_manifest.is_empty()).then_some(&round_manifest[..]),
        blob: (!blob.is_empty()).then_some(&blob[..]),
        content_key,
    };
    let r = rerun(&d).unwrap_or_else(|e| panic!("no verdict to prove: {e}"));
    sp1_zkvm::io::commit_slice(&r.public_values());
}

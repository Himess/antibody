//! Parser fuzzing (docs/properties.md, "Inputs"): every parser of untrusted bytes, and the verifier's
//! program path itself, take any input without panicking. In wasm a panic is a trap, which the caller
//! cannot tell from a crash, so each must end in a typed result. The seeds are the recorded fixtures;
//! the mutations flip, overwrite, insert and delete bytes and truncate. `PROPTEST_CASES=<n>` runs more.
//!
//!   cargo test --release -p antibody-verifier --test fuzz_parsers

use std::path::PathBuf;

use antibody_verifier::{manifest::Manifest, run_program, sealed::SealedPayload, Envelope, Payload, ProgramContext};
use proptest::prelude::*;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(rel: &str) -> Vec<u8> {
    std::fs::read(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// One edit of a byte string: (kind, position, byte).
fn edits() -> impl Strategy<Value = Vec<(u8, usize, u8)>> {
    prop::collection::vec((0u8..5, any::<usize>(), any::<u8>()), 1..24)
}

fn mutate(seed: &[u8], edits: &[(u8, usize, u8)]) -> Vec<u8> {
    let mut v = seed.to_vec();
    for &(kind, pos, b) in edits {
        if v.is_empty() {
            v.push(b);
            continue;
        }
        let at = pos % v.len();
        match kind {
            0 => v[at] ^= 1 << (b % 8),
            1 => v[at] = b,
            2 => v.insert(at, b),
            3 => {
                v.remove(at);
            }
            _ => v.truncate(at),
        }
    }
    v
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn manifest_parse_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..1024)) {
        let _ = Manifest::parse(&bytes);
    }

    /// A mutated manifest either fails to parse or parses to one whose canonical form is stable.
    #[test]
    fn a_mutated_manifest_is_refused_or_canonical(e in edits()) {
        let m = mutate(&fixture("fixtures/acme-local/manifest.json"), &e);
        if let Ok(x) = Manifest::parse(&m) {
            let c = x.canonical();
            prop_assert_eq!(Manifest::parse(&c).map(|y| y.canonical()), Ok(c));
        }
    }

    #[test]
    fn envelope_decode_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = Envelope::decode(&bytes);
    }

    #[test]
    fn a_mutated_witness_is_refused_or_decodes(e in edits()) {
        let _ = Envelope::decode(&mutate(&fixture("fixtures/acme-local/submissions/A.witness.bin"), &e));
    }

    #[test]
    fn sealed_payload_decode_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _ = SealedPayload::decode(&bytes);
    }
}

/// The recorded witnesses decode as program witnesses (or the structure-aware test below is vacuous).
#[test]
fn the_seed_witnesses_are_program_witnesses() {
    for agent in ["A", "B", "C", "D", "F"] {
        let env = Envelope::decode(&fixture(&format!("fixtures/acme-local/submissions/{agent}.witness.bin"))).unwrap();
        assert!(matches!(env.payload, Payload::Program(_)), "{agent}");
    }
}

proptest! {
    // Each case runs the whole program path (proofs, EVM, rules): fewer cases.
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Decoded, then one part mutated (the header, the manifest, the rules' code, the proof's calldata, or
    /// the bundle's endowment), the witness still gets a typed receipt from the whole verifier.
    #[test]
    fn the_verifier_answers_structurally_mutated_program_witnesses(part in 0u8..5, e in edits(), agent in prop::sample::select(vec!["A", "B", "F"])) {
        let env = Envelope::decode(&fixture(&format!("fixtures/acme-local/submissions/{agent}.witness.bin"))).unwrap();
        let Payload::Program(mut w) = env.payload.clone() else { unreachable!("checked above") };
        match part {
            0 => w.header_rlp = mutate(&w.header_rlp, &e).into(),
            1 => w.manifest_json = mutate(&w.manifest_json, &e).into(),
            2 => w.invariants_code = mutate(&w.invariants_code, &e).into(),
            3 => {
                if let Some(c) = w.bundle.calls.first_mut() {
                    c.data = mutate(&c.data, &e).into();
                }
            }
            _ => w.bundle.endowment_wei = w.bundle.endowment_wei.wrapping_mul(e.len() as u128 + 2),
        }
        let mut env2 = env.clone();
        env2.payload = Payload::Program(w.clone());
        let r = run_program(&env2, &w, [0u8; 32], &ProgramContext::default());
        let kind = format!("{:?}", r.kind);
        prop_assert!(!kind.is_empty());
    }
}

proptest! {
    // Each case runs the whole program path (proofs, EVM, rules): fewer cases.
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// The verifier on a mutated program witness always answers with a typed receipt.
    #[test]
    fn the_verifier_answers_any_mutated_program_witness(e in edits(), agent in prop::sample::select(vec!["A", "B", "C", "D", "F"])) {
        let bytes = mutate(&fixture(&format!("fixtures/acme-local/submissions/{agent}.witness.bin")), &e);
        if let Ok(env) = Envelope::decode(&bytes) {
            if let Payload::Program(w) = &env.payload {
                let r = run_program(&env, w, [0u8; 32], &ProgramContext::default());
                let kind = format!("{:?}", r.kind);
                prop_assert!(!kind.is_empty());
            }
        }
    }
}

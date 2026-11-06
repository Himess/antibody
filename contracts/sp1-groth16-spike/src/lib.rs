//! Spike, not deployed (docs/zk-feasibility.md): verifies an SP1 Groth16 proof on NEAR with the
//! alt_bn128 host functions, as a NEAR contract that checks validity proofs of disclosed verdicts would.
//!
//! SP1 6.x proof bytes: the verifier selector (the first 4 bytes of sha256 of the Groth16 key), then
//! three 32-byte words (exit code, vk root, proof nonce), then gnark's proof (A in G1, B in G2, C in G1,
//! uncompressed, big-endian, G2 as (x.c1, x.c0, y.c1, y.c0)). The five public inputs, in order: the
//! program's vkey hash, sha256 of the public values with the top 3 bits cleared, the exit code, the vk
//! root and the nonce. The check: e(A, B) · e(C, −δ) · e(α, −β) · e(L, −γ) = 1, with
//! L = IC₀ + Σ inputᵢ · ICᵢ₊₁ (one multiexp, one sum, one pairing check of four pairs).
//!
//! NEAR's host functions read field elements little-endian and a G2 point as (x.c0, x.c1, y.c0, y.c1):
//! the proof is converted here, the keys once (`vk.rs`, scripts/zk/sp1_groth16_vk.py).

use near_sdk::{env, json_types::Base64VecU8, near};

pub mod vk;

/// An SP1 Groth16 verifying key in NEAR's encoding (G1: 64 bytes, G2: 128 bytes, little-endian).
pub struct Sp1Groth16Vk {
    pub selector: [u8; 4],
    /// Big-endian, as SP1 writes it into the proof.
    pub vk_root: [u8; 32],
    pub alpha: [u8; 64],
    pub beta_neg: [u8; 128],
    pub gamma_neg: [u8; 128],
    pub delta_neg: [u8; 128],
    /// IC₀ (the constant) and one point per public input.
    pub ic: [[u8; 64]; 6],
}

/// BN254's scalar field order r, big-endian: a public input must be below it.
const R: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d, 0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91,
    0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

pub const PROOF_LEN: usize = 4 + 3 * 32 + 256;

fn rev(b: &[u8]) -> Vec<u8> {
    b.iter().rev().copied().collect()
}

/// A big-endian G1 point (x, y) in NEAR's encoding.
fn g1(b: &[u8]) -> Vec<u8> {
    [rev(&b[..32]), rev(&b[32..64])].concat()
}

/// A big-endian G2 point (x.c1, x.c0, y.c1, y.c0) in NEAR's encoding (x.c0, x.c1, y.c0, y.c1).
fn g2(b: &[u8]) -> Vec<u8> {
    [rev(&b[32..64]), rev(&b[..32]), rev(&b[96..128]), rev(&b[64..96])].concat()
}

/// SP1's public input for the public values: sha256 with the top 3 bits cleared (it fits BN254's field).
pub fn public_values_digest(public_values: &[u8]) -> [u8; 32] {
    let mut d = env::sha256_array(public_values);
    d[0] &= 0x1f;
    d
}

/// Verifies an SP1 Groth16 proof of `program_vkey` (the program's vkey hash, big-endian) committing
/// `public_values`. Ok(false): a well-formed proof that does not verify. Err: malformed, or not a
/// proof this key or this program's exit can accept.
pub fn verify(vk: &Sp1Groth16Vk, program_vkey: &[u8; 32], public_values: &[u8], proof: &[u8]) -> Result<bool, &'static str> {
    if proof.len() != PROOF_LEN {
        return Err("an SP1 Groth16 proof is 356 bytes");
    }
    if proof[..4] != vk.selector {
        return Err("the proof is for another Groth16 key (verifier selector)");
    }
    let (exit_code, vk_root, nonce, gnark) = (&proof[4..36], &proof[36..68], &proof[68..100], &proof[100..]);
    if exit_code.iter().any(|b| *b != 0) {
        return Err("the program did not exit with 0");
    }
    if vk_root != vk.vk_root {
        return Err("another recursion vk root");
    }
    let digest = public_values_digest(public_values);
    let inputs: [&[u8]; 5] = [program_vkey, &digest, exit_code, vk_root, nonce];
    if inputs.iter().any(|x| *x >= &R[..]) {
        return Err("a public input outside BN254's scalar field");
    }
    // L = IC₀ + Σ inputᵢ · ICᵢ₊₁.
    let mut msm = Vec::with_capacity(5 * 96);
    for (i, x) in inputs.iter().enumerate() {
        msm.extend_from_slice(&vk.ic[i + 1]);
        msm.extend(rev(x));
    }
    let l = env::alt_bn128_g1_multiexp(&msm);
    let mut sum = Vec::with_capacity(2 * 65);
    sum.push(0);
    sum.extend_from_slice(&l);
    sum.push(0);
    sum.extend_from_slice(&vk.ic[0]);
    let l = env::alt_bn128_g1_sum(&sum);
    // e(A, B) · e(C, −δ) · e(α, −β) · e(L, −γ) = 1.
    let mut pairs = Vec::with_capacity(4 * 192);
    pairs.extend(g1(&gnark[..64]));
    pairs.extend(g2(&gnark[64..192]));
    pairs.extend(g1(&gnark[192..256]));
    pairs.extend_from_slice(&vk.delta_neg);
    pairs.extend_from_slice(&vk.alpha);
    pairs.extend_from_slice(&vk.beta_neg);
    pairs.extend_from_slice(&l);
    pairs.extend_from_slice(&vk.gamma_neg);
    Ok(env::alt_bn128_pairing_check(&pairs))
}

fn vkey32(s: &str) -> [u8; 32] {
    let s = s.trim_start_matches("0x");
    let mut out = [0u8; 32];
    if s.len() != 64 {
        env::panic_str("the program vkey is 32 bytes, 0x-hex");
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap_or_else(|_| env::panic_str("the program vkey is 0x-hex"));
    }
    out
}

#[near(contract_state)]
#[derive(Default)]
pub struct Spike {}

#[near]
impl Spike {
    /// Verifies a proof against SP1 6.10.0's Groth16 key (the version crates/zk/guest pins).
    pub fn verify_sp1_6_10(&self, program_vkey: String, public_values: Base64VecU8, proof: Base64VecU8) -> bool {
        verify(&vk::SP1_V6_10_0, &vkey32(&program_vkey), &public_values.0, &proof.0).unwrap_or_else(|e| env::panic_str(e))
    }

    /// Verifies a proof against SP1 v6.0.0's Groth16 key (SP1's published fixture proof).
    pub fn verify_sp1_6_0(&self, program_vkey: String, public_values: Base64VecU8, proof: Base64VecU8) -> bool {
        verify(&vk::SP1_V6_0_0, &vkey32(&program_vkey), &public_values.0, &proof.0).unwrap_or_else(|e| env::panic_str(e))
    }
}

#[cfg(test)]
mod tests {
    use near_sdk::{test_utils::VMContextBuilder, testing_env, Gas};

    use super::*;

    struct Fixture {
        vkey: [u8; 32],
        public_values: Vec<u8>,
        proof: Vec<u8>,
    }

    /// SP1's published v6.0.0 Groth16 fixture (sp1-project-template, contracts/src/fixtures).
    fn fixture() -> Fixture {
        let f: serde_json::Value = serde_json::from_str(include_str!("../tests/data/sp1-v6.0.0-groth16-fixture.json")).unwrap();
        let h = |k: &str| hex::decode(f[k].as_str().unwrap().trim_start_matches("0x")).unwrap();
        Fixture { vkey: h("vkey").try_into().unwrap(), public_values: h("publicValues"), proof: h("proof") }
    }

    fn env_with_gas() {
        testing_env!(VMContextBuilder::new().prepaid_gas(Gas::from_tgas(300)).build());
    }

    #[test]
    fn sp1s_published_groth16_fixture_verifies_on_nears_host_functions() {
        env_with_gas();
        let f = fixture();
        let before = env::used_gas();
        assert_eq!(verify(&vk::SP1_V6_0_0, &f.vkey, &f.public_values, &f.proof), Ok(true));
        let used = env::used_gas().as_gas() - before.as_gas();
        // The host functions' gas (unit tests do not meter the contract's own instructions).
        println!("host gas of one SP1 Groth16 verification: {:.2} Tgas", used as f64 / 1e12);
        assert!(used < 40_000_000_000_000, "{used}");
    }

    #[test]
    fn a_changed_public_value_or_program_does_not_verify() {
        env_with_gas();
        let f = fixture();
        let mut pv = f.public_values.clone();
        *pv.last_mut().unwrap() ^= 1;
        assert_eq!(verify(&vk::SP1_V6_0_0, &f.vkey, &pv, &f.proof), Ok(false), "public values");
        let mut vkey = f.vkey;
        vkey[31] ^= 1;
        assert_eq!(verify(&vk::SP1_V6_0_0, &vkey, &f.public_values, &f.proof), Ok(false), "program");
        let mut nonce = f.proof.clone();
        nonce[99] ^= 1;
        assert_eq!(verify(&vk::SP1_V6_0_0, &f.vkey, &f.public_values, &nonce), Ok(false), "nonce");
    }

    #[test]
    fn a_proof_for_another_key_or_with_another_exit_is_refused() {
        env_with_gas();
        let f = fixture();
        assert!(verify(&vk::SP1_V6_10_0, &f.vkey, &f.public_values, &f.proof).is_err(), "a v6.0.0 proof is not for 6.10.0's key");
        let mut exit = f.proof.clone();
        exit[35] = 1;
        assert!(verify(&vk::SP1_V6_0_0, &f.vkey, &f.public_values, &exit).is_err());
        assert!(verify(&vk::SP1_V6_0_0, &f.vkey, &f.public_values, &f.proof[..300]).is_err());
    }

    #[test]
    fn sp1_6_10s_key_is_the_one_sp1_ships() {
        // Its selector is the first 4 bytes of sha256(groth16_vk.bin) (the points themselves are checked by
        // scripts/zk/sp1_groth16_vk.py against that file).
        let shipped = include_bytes!("../tests/data/sp1-6.10.0-groth16_vk.bin");
        env_with_gas();
        assert_eq!(env::sha256_array(shipped)[..4], vk::SP1_V6_10_0.selector);
    }

    // near-sdk declares the yield/resume host functions; Windows links the test binary against these
    // symbols anyway (never called).
    #[cfg(windows)]
    #[no_mangle]
    pub extern "C" fn promise_yield_create(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> u64 {
        unreachable!()
    }
    #[cfg(windows)]
    #[no_mangle]
    pub extern "C" fn promise_yield_resume(_: u64, _: u64, _: u64, _: u64) -> u32 {
        unreachable!()
    }
}

//! The math host functions' primitives, with nearcore's acceptance rules.

pub fn sha256(data: &[u8]) -> [u8; 32] {
    crate::util::sha256(data)
}

pub fn keccak256(data: &[u8]) -> Vec<u8> {
    use sha3::Digest;
    sha3::Keccak256::digest(data).to_vec()
}

pub fn keccak512(data: &[u8]) -> Vec<u8> {
    use sha3::Digest;
    sha3::Keccak512::digest(data).to_vec()
}

pub fn ripemd160(data: &[u8]) -> Vec<u8> {
    use ripemd::Digest;
    ripemd::Ripemd160::digest(data).to_vec()
}

/// secp256k1 group order n, big-endian.
const SECP256K1_N: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48,
    0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
];
/// n / 2 + 1, big-endian: with the malleability flag, `s` must be below it.
const SECP256K1_N_HALF_ONE: [u8; 32] = [
    0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4,
    0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b, 0x20, 0xa1,
];

/// nearcore's `check_signature_values`: `r < n` and `s < n` (or `s < n/2 + 1` when rejecting upper `s`).
/// Big-endian byte arrays compare like the numbers they encode.
pub fn check_signature_values(sig: &[u8; 64], reject_upper: bool) -> bool {
    let r: [u8; 32] = sig[..32].try_into().unwrap();
    let s: [u8; 32] = sig[32..].try_into().unwrap();
    r < SECP256K1_N && s < if reject_upper { SECP256K1_N_HALF_ONE } else { SECP256K1_N }
}

/// Recovers the 64-byte uncompressed public key (no 0x04 prefix) from a hash, a 64-byte `r ‖ s` signature
/// and a recovery id 0..=3, as nearcore's `ecrecover` does. A high `s` is accepted (the caller has already
/// applied the malleability flag): the same key is recovered from `(r, n - s)` with the y parity flipped.
pub fn ecrecover(hash: &[u8; 32], sig: &[u8; 64], v: u8) -> Option<[u8; 64]> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    let mut signature = Signature::from_slice(sig).ok()?;
    let mut recid = RecoveryId::from_byte(v)?;
    if let Some(normalized) = signature.normalize_s() {
        signature = normalized;
        recid = RecoveryId::new(!recid.is_y_odd(), recid.is_x_reduced());
    }
    let key = VerifyingKey::recover_from_prehash(hash, &signature, recid).ok()?;
    let point = key.to_encoded_point(false);
    point.as_bytes().get(1..65)?.try_into().ok()
}

/// nearcore's `ed25519_verify` after its length checks, i.e. the non-strict RFC 8032 check nearcore's
/// ed25519 library performs: false for a signature whose last byte has any of the top three bits set
/// (nearcore's own pre-check), for an `s` that is not a canonical scalar, for a public key that does not
/// decompress, or unless `[s]B - [k]A` compresses to `R`, with `k = SHA-512(R ‖ A ‖ M) mod l`. No
/// small-order or canonical-`R` checks (those belong to strict verification, which nearcore does not use).
pub fn ed25519_verify(sig: &[u8; 64], message: &[u8], public_key: &[u8; 32]) -> bool {
    use curve25519_dalek::{edwards::CompressedEdwardsY, scalar::Scalar, EdwardsPoint};
    use sha2::{Digest, Sha512};
    if sig[63] & 0b1110_0000 != 0 {
        return false;
    }
    let r: [u8; 32] = sig[..32].try_into().unwrap();
    let s_bytes: [u8; 32] = sig[32..].try_into().unwrap();
    let Some(s) = Option::<Scalar>::from(Scalar::from_canonical_bytes(s_bytes)) else {
        return false;
    };
    let Some(a) = CompressedEdwardsY(*public_key).decompress() else {
        return false;
    };
    let h: [u8; 64] = Sha512::new().chain_update(r).chain_update(public_key).chain_update(message).finalize().into();
    let k = Scalar::from_bytes_mod_order_wide(&h);
    EdwardsPoint::vartime_double_scalar_mul_basepoint(&k, &(-a), &s).compress().to_bytes() == r
}

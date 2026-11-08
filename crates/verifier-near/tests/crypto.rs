//! The math host functions against the reference runtime's own test vectors (byte for byte): the hashes
//! of "tesdsst", the 161 ecrecover cases (tests/data/ecrecover-tests.json, copied from the reference
//! runtime's test suite), and its ed25519 signature, message and key with their forged variants.

use antibody_verifier_near::crypto;

#[test]
fn hashes_match_the_reference_vectors() {
    let data = b"tesdsst";
    assert_eq!(
        crypto::sha256(data).to_vec(),
        vec![18, 176, 115, 156, 45, 100, 241, 132, 180, 134, 77, 42, 105, 111, 199, 127, 118, 112, 92, 255, 88, 43, 83, 147, 122, 55, 26, 36, 42, 156, 160, 158]
    );
    assert_eq!(
        crypto::keccak256(data),
        vec![104, 110, 58, 122, 230, 181, 215, 145, 231, 229, 49, 162, 123, 167, 177, 58, 26, 142, 129, 173, 7, 37, 9, 26, 233, 115, 64, 102, 61, 85, 10, 159]
    );
    assert_eq!(
        crypto::keccak512(data),
        vec![
            55, 134, 96, 137, 168, 122, 187, 95, 67, 76, 18, 122, 146, 11, 225, 106, 117, 194, 154, 157, 48, 160, 90, 146, 104, 209, 118, 126, 222, 230,
            200, 125, 48, 73, 197, 236, 123, 173, 192, 197, 90, 153, 167, 121, 100, 88, 209, 240, 137, 86, 239, 41, 87, 128, 219, 249, 136, 203, 220, 109,
            46, 168, 234, 190
        ]
    );
    assert_eq!(crypto::ripemd160(data), vec![21, 102, 156, 115, 232, 3, 58, 215, 35, 84, 129, 30, 143, 86, 212, 104, 70, 97, 14, 225]);
}

#[derive(serde::Deserialize)]
struct EcrecoverCase {
    m: String,
    v: u8,
    sig: String,
    mc: bool,
    res: Option<String>,
}

#[test]
fn ecrecover_matches_all_reference_vectors() {
    let cases: Vec<EcrecoverCase> = serde_json::from_str(include_str!("data/ecrecover-tests.json")).unwrap();
    assert_eq!(cases.len(), 161);
    for (i, c) in cases.iter().enumerate() {
        let m: [u8; 32] = hex::decode(&c.m).unwrap().try_into().unwrap();
        let sig: [u8; 64] = hex::decode(&c.sig).unwrap().try_into().unwrap();
        // The host function's order: the signature values are checked first, then the key is recovered.
        let got = if crypto::check_signature_values(&sig, c.mc) { crypto::ecrecover(&m, &sig, c.v) } else { None };
        let want = c.res.as_ref().map(|r| hex::decode(r).unwrap());
        assert_eq!(got.map(|k| k.to_vec()), want, "case {i}");
    }
}

const SIGNATURE: [u8; 64] = [
    145, 193, 203, 18, 114, 227, 14, 117, 33, 213, 121, 66, 130, 14, 25, 4, 36, 120, 46, 142, 226, 215, 7, 66, 122, 112, 97, 30, 249, 135, 61, 165, 221,
    249, 252, 23, 105, 40, 56, 70, 31, 152, 236, 141, 154, 122, 207, 20, 75, 118, 79, 90, 168, 6, 221, 122, 213, 29, 126, 196, 216, 104, 191, 6,
];
const PUBLIC_KEY: [u8; 32] =
    [32, 122, 6, 120, 146, 130, 30, 37, 215, 112, 241, 251, 160, 196, 124, 17, 255, 75, 129, 62, 84, 22, 46, 206, 158, 184, 57, 224, 118, 35, 26, 182];
const MESSAGE: [u8; 32] =
    [107, 97, 106, 100, 108, 102, 107, 106, 97, 108, 107, 102, 106, 97, 107, 108, 102, 106, 100, 107, 108, 97, 100, 106, 102, 107, 108, 106, 97, 100, 115, 107];

#[test]
fn ed25519_matches_the_reference_cases() {
    assert!(crypto::ed25519_verify(&SIGNATURE, &MESSAGE, &PUBLIC_KEY));
    let mut forged_key = PUBLIC_KEY;
    forged_key[31] = 0b1110_0001;
    assert!(!crypto::ed25519_verify(&SIGNATURE, &MESSAGE, &forged_key), "a key that does not decompress");
    let mut forged_sig = SIGNATURE;
    forged_sig[63] = 0b1110_0001;
    assert!(!crypto::ed25519_verify(&forged_sig, &MESSAGE, &PUBLIC_KEY), "the top bits pre-check");
    assert!(!crypto::ed25519_verify(&[1; 64], &MESSAGE, &PUBLIC_KEY), "a bad signature");
    let mut other = MESSAGE;
    other[0] ^= 1;
    assert!(!crypto::ed25519_verify(&SIGNATURE, &other, &PUBLIC_KEY), "another message");
}

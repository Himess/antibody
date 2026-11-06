//! The disclosed testnet runs in `fixtures/zk/`, re-run natively: each receipt must equal, byte for byte,
//! the one the TEE produced (near-antibody records its sha256 in the run's facts).

use std::path::{Path, PathBuf};

use antibody_zk_core::{decode_public_values, kind_byte, rerun, Disclosed};
use serde_json::Value;

fn dir(case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/zk").join(case)
}

struct Case {
    request: Vec<u8>,
    base: Vec<u8>,
    blob: Option<Vec<u8>>,
    key: [u8; 32],
    expected: Value,
}

fn load(case: &str) -> Case {
    let d = dir(case);
    let expected: Value = serde_json::from_slice(&std::fs::read(d.join("expected.json")).unwrap()).unwrap();
    let base_sha = expected["base_sha256"].as_str().unwrap();
    let key = hex::decode(std::fs::read_to_string(d.join("content_key.hex")).unwrap().trim()).unwrap();
    Case {
        request: std::fs::read(d.join("request.json")).unwrap(),
        base: std::fs::read(d.join("../base").join(format!("{base_sha}.bin"))).unwrap(),
        blob: std::fs::read(d.join("blob.bin")).ok(),
        key: key.try_into().unwrap(),
        expected,
    }
}

fn disclosed(c: &Case) -> Disclosed<'_> {
    Disclosed { request: &c.request, base: &c.base, round_manifest: None, blob: c.blob.as_deref(), content_key: c.key }
}

#[test]
fn each_disclosed_run_reproduces_the_tees_receipt() {
    for case in ["a", "d", "c", "practice"] {
        let c = load(case);
        let r = rerun(&disclosed(&c)).unwrap_or_else(|e| panic!("{case}: {e}"));
        let e = &c.expected;
        assert_eq!(hex::encode(r.request_sha256), e["request_sha256"].as_str().unwrap(), "{case}: the request near-antibody sent");
        assert_eq!(hex::encode(r.blob_sha256), e["blob_sha256"].as_str().unwrap(), "{case}: the committed blob");
        assert_eq!(hex::encode(r.receipt_sha256), e["tee"]["receipt_sha256"].as_str().unwrap(), "{case}: the receipt equals the TEE's");
        assert_eq!(r.run_id, e["run_id"].as_u64().unwrap());
        assert_eq!(r.commit_id, e["commit_id"].as_u64().unwrap());
        let kind = serde_json::to_value(r.receipt.kind).unwrap();
        assert_eq!(kind, e["tee"]["kind"], "{case}: the verdict");
        // The keys' tags are the TEE's MACs; their slots are the broken slots.
        let slots: Vec<u64> = e["tee"]["keys"].as_array().unwrap().iter().map(|k| k["slot"].as_u64().unwrap()).collect();
        let broken: Vec<u64> = r.receipt.broken.iter().map(|f| f.slot as u64).collect();
        assert_eq!(broken, slots, "{case}: the keys' slots");
        // The public values round-trip.
        let pv = decode_public_values(&r.public_values()).unwrap();
        assert_eq!((pv.receipt_sha256, pv.run_id, pv.kind), (r.receipt_sha256, r.run_id, kind_byte(r.receipt.kind)));
        assert_eq!(pv.broken.len(), r.receipt.broken.len());
    }
}

#[test]
fn a_wrong_key_or_a_changed_request_does_not_reproduce_it() {
    let c = load("a");
    let mut wrong = disclosed(&c);
    wrong.content_key[0] ^= 1;
    let err = rerun(&wrong).err().expect("a wrong key opens nothing");
    assert!(err.contains("does not open"), "{err}");
    // Another commit id in the request: another request hash, and another receipt.
    let req = String::from_utf8(c.request.clone()).unwrap().replace("\"commit_id\":13", "\"commit_id\":12");
    assert_ne!(req.as_bytes(), &c.request[..], "the fixture names commit 13");
    let d = Disclosed { request: req.as_bytes(), ..disclosed(&c) };
    let r = rerun(&d).unwrap();
    assert_ne!(hex::encode(r.receipt_sha256), c.expected["tee"]["receipt_sha256"].as_str().unwrap());
    // Another base: refused.
    let mut base = c.base.clone();
    base[100] ^= 1;
    assert!(rerun(&Disclosed { base: &base, ..disclosed(&c) }).is_err());
}

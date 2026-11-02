//! TEE mode end to end on the recorded fixture: each witness is sealed exactly as a hunter's CLI
//! would (docs/crypto-spec.md), then handled by `tee::handle` the way OutLayer runs the verifier.
//! The facts must equal the recorded ones (same receipt hash, kind, mask and fp_tags) and carry a
//! project wrap that opens with the project's key only.

use std::path::PathBuf;

use antibody_verifier::{
    facts::{sha256, Facts, Kind},
    sealed::{self, SealedPayload},
    tee::{handle, TeeEnv, TeeError},
};
use x25519_dalek::StaticSecret;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The fixture's TEST-ONLY secret (scripts/demo/check_fixtures.py).
fn test_seed() -> [u8; 32] {
    sha256(b"antibody/TEST-ONLY-secret/never-use-in-production")
}

const PROJECT_SK: [u8; 32] = [77u8; 32];

fn sealed_request(agent: &str, program_id: &str, commit_id: u64, hunter: &str, expected: &serde_json::Value) -> (Vec<u8>, Vec<u8>) {
    let envelope = std::fs::read(root().join(format!("fixtures/acme-local/submissions/{agent}.witness.bin"))).unwrap();
    let pk_v = sealed::verifier_public(&test_seed());
    let k = sha256(format!("content-key/{agent}").as_bytes());
    let wrap = sealed::wrap_v(&k, &pk_v, program_id, sha256(format!("eph/{agent}").as_bytes()), [1u8; 12]);
    let payload = SealedPayload { program_key: program_id.into(), hunter: hunter.into(), envelope, report: b"ignored".to_vec() };
    let blob = sealed::seal(&k, &payload, &wrap, [2u8; 12]);
    let req = serde_json::json!({
        "mode": "verify",
        "program_id": program_id,
        "commit_id": commit_id,
        "hunter": hunter,
        "manifest_hash": expected["manifest_sha256"].as_str().unwrap().trim_start_matches("0x"),
        "sealed": {"url": "https://blobs.example/x", "sha256": hex::encode(sha256(&blob)), "wrap_v": hex::encode(wrap)},
        "project_pubkey": hex::encode(sealed::x25519_public(PROJECT_SK)),
    });
    (serde_json::to_vec(&req).unwrap(), blob)
}

#[test]
fn tee_mode_reproduces_the_recorded_facts() {
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("fixtures/acme-local/expected.json")).unwrap()).unwrap();
    let program_id = expected["program_id"].as_str().unwrap().to_string();
    for (agent, e) in expected["submissions"].as_object().unwrap() {
        let commit_id = e["commit_id"].as_u64().unwrap();
        let (req, blob) = sealed_request(agent, &program_id, commit_id, "agent.testnet", &expected);
        let fetch = |_: &str| Ok(blob.clone());
        let env = TeeEnv { seed: Some(test_seed()), predecessor: None, fetch: &fetch, wasm_hash: None };
        let facts: Facts = serde_json::from_slice(&handle(&req, &env).unwrap()).unwrap();
        assert_eq!(facts.receipt_hash, e["receipt_hash"].as_str().unwrap(), "{agent}: receipt hash");
        assert_eq!(format!("{:?}", facts.kind), e["kind"].as_str().unwrap(), "{agent}: kind");
        assert_eq!(facts.broken_mask, e["broken_mask"].as_str().unwrap(), "{agent}: mask");
        let tags: Vec<String> = facts.fp_tags.iter().map(|t| t.tag.clone()).collect();
        let want: Vec<String> = e["fp_tags"].as_array().unwrap().iter().map(|t| t["tag"].as_str().unwrap().to_string()).collect();
        assert_eq!(tags, want, "{agent}: fp_tags");
        assert!(serde_json::to_vec(&facts).unwrap().len() <= 1024, "{agent}: facts <= 1 KB");
        match facts.kind {
            Kind::Violation => {
                let wrap = hex::decode(facts.project_wrap.as_ref().expect("project wrap")).unwrap();
                let k = sealed::project_unwrap(&wrap, &StaticSecret::from(PROJECT_SK), &program_id).unwrap();
                let p = sealed::open_with_key(&blob, &k, &program_id).unwrap();
                assert_eq!(p.hunter, "agent.testnet");
            }
            _ => assert!(facts.project_wrap.is_none(), "{agent}: non-violations are never disclosed to the project"),
        }
    }
}

#[test]
fn tampering_and_rebinding_become_invalid_poc() {
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("fixtures/acme-local/expected.json")).unwrap()).unwrap();
    let pid = "bounty.testnet/0";
    // A copycat commits someone else's blob under their own account: rejected (payload binds the hunter).
    let (req, blob) = sealed_request("A", pid, 5, "agent-a.testnet", &expected);
    let mut r: serde_json::Value = serde_json::from_slice(&req).unwrap();
    r["hunter"] = "copycat.testnet".into();
    let fetch = |_: &str| Ok(blob.clone());
    let env = TeeEnv { seed: Some(test_seed()), predecessor: Some("bounty.testnet".into()), fetch: &fetch, wasm_hash: None };
    let f: Facts = serde_json::from_slice(&handle(&serde_json::to_vec(&r).unwrap(), &env).unwrap()).unwrap();
    assert_eq!(f.kind, Kind::InvalidPoC);
    // A blob that does not hash to the committed sha256: rejected.
    let mut tampered = blob.clone();
    *tampered.last_mut().unwrap() ^= 1;
    let fetch2 = |_: &str| Ok(tampered.clone());
    let env2 = TeeEnv { seed: Some(test_seed()), predecessor: None, fetch: &fetch2, wasm_hash: None };
    let f: Facts = serde_json::from_slice(&handle(&req, &env2).unwrap()).unwrap();
    assert_eq!(f.kind, Kind::InvalidPoC);
    // A request that does not come from the program's contract is refused outright.
    let env3 = TeeEnv { seed: Some(test_seed()), predecessor: Some("someone-else.testnet".into()), fetch: &fetch, wasm_hash: None };
    assert!(matches!(handle(&req, &env3), Err(TeeError::Fatal(_))));
    // A fetch failure is transient (retry), not a verdict.
    let fail = |_: &str| Err("502".to_string());
    let env4 = TeeEnv { seed: Some(test_seed()), predecessor: None, fetch: &fail, wasm_hash: None };
    assert!(matches!(handle(&req, &env4), Err(TeeError::Transient(_))));
}

#[test]
fn pubkey_and_disclosure_modes() {
    let fetch = |_: &str| Err("unused".to_string());
    let env = TeeEnv { seed: Some(test_seed()), predecessor: Some("bounty.testnet".into()), fetch: &fetch, wasm_hash: None };
    let pk: serde_json::Value = serde_json::from_slice(&handle(br#"{"mode":"pubkey"}"#, &env).unwrap()).unwrap();
    assert_eq!(pk["pubkey"].as_str().unwrap(), hex::encode(sealed::verifier_public(&test_seed())));
    let pid = "bounty.testnet/0";
    let k = [5u8; 32];
    let wrap = sealed::wrap_v(&k, &sealed::verifier_public(&test_seed()), pid, [6u8; 32], [7u8; 12]);
    let req = serde_json::json!({"mode": "disclose", "program_id": pid, "entries": [{"commit_id": 3, "wrap_v": hex::encode(wrap)}]});
    let out: serde_json::Value = serde_json::from_slice(&handle(&serde_json::to_vec(&req).unwrap(), &env).unwrap()).unwrap();
    assert_eq!(out["keys"][0]["content_key"].as_str().unwrap(), hex::encode(k));
    assert_eq!(out["k_program"].as_str().unwrap().len(), 64);
}

//! Web vectors (golden/web-vectors.json): what the web app's browser try must produce byte for byte
//! (web/lib/practice): the tutorial proof's submission envelope built from the practice program's pinned
//! base witness, its seal (`ABSEAL1`) under the public practice content key, the commitment, and signed
//! NEAR transactions. Each value is computed here with the code `antibody try` runs, and checked again by
//! the web app's tests (`npm test` in web/), so neither side can drift alone. `GOLDEN_WRITE=1` rewrites
//! the file.
//!
//! The base is the practice program's (program 6 on testnet), as its pinned location serves it
//! (fixtures/web/practice-base.bin, sha256 bed565c1…ed39). Fixed bytes stand in for the random ones (the
//! salt, the ephemeral key and the nonces); the browser draws them from its CSPRNG.

use std::path::PathBuf;

use alloy_primitives::B256;
use antibody_cli::{blob, cmd, near, practice};
use antibody_verifier::{
    facts::sha256,
    sealed,
    witness::{Envelope, Payload},
};
use antibody_witness::build::{parse_bundle, BundleFile};
use base64::Engine as _;
use serde_json::{json, Value};

const BASE_SHA256: &str = "bed565c109e123ddc1b7bce134e15bd8ad95a1bed94d3f6fd2a631e05ab0ed39";
const PROGRAM_KEY: &str = "bounty.antibody.testnet/6";
/// Program 6's sealing key (`get_program`'s `sealing_pk`), recorded by its T1-confirmed activation.
const SEALING_PK: &str = "ea66fa087c57c5b32066c97a9791b0c1d5cfa2e96db22ea33cc693d0a91b764f";
const VENDING: &str = "practice.antibody.testnet";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn path() -> PathBuf {
    root().join("golden/web-vectors.json")
}

/// One practice try's sealed submission, as `antibody try` builds it for `hunter` against vault `vault`.
fn try_vector(base: &[u8], manifest_json: &[u8], hunter: &str, vault_index: u32, vault: &str, salt: B256, e: [u8; 32], nonce_w: [u8; 12], nonce_b: [u8; 12]) -> Value {
    let bundle_json = practice::tutorial_bundle(vault);
    let bf: BundleFile = serde_json::from_value(bundle_json.clone()).unwrap();
    let bundle = parse_bundle(&bf, salt).unwrap();
    let (envelope, _meta, receipt) = antibody_witness::program::build_submission_from_base(base, manifest_json, bundle, PROGRAM_KEY).unwrap();
    let k = blob::practice_content_key(PROGRAM_KEY, hunter);
    let pk_v: [u8; 32] = hex::decode(SEALING_PK).unwrap().try_into().unwrap();
    let wrap = sealed::wrap_v(&k, &pk_v, PROGRAM_KEY, e, nonce_w);
    let payload = sealed::SealedPayload { program_key: PROGRAM_KEY.into(), hunter: hunter.into(), envelope: envelope.clone(), report: vec![] };
    let blob_bytes = sealed::seal(&k, &payload, &wrap, nonce_b);
    let blob_sha = sha256(&blob_bytes);
    // The TEE's side: the program's key opens it (here the content key, as anyone can for a practice try).
    assert_eq!(sealed::open_with_key(&blob_bytes, &k, PROGRAM_KEY).unwrap(), payload);
    json!({
        "hunter": hunter,
        "vault_index": vault_index,
        "vault": vault,
        "salt": format!("{salt:#x}"),
        "bundle": bundle_json,
        "envelope_hex": hex::encode(&envelope),
        "envelope_sha256": hex::encode(sha256(&envelope)),
        "local_verdict": {"kind": format!("{:?}", receipt.kind), "broken_slots": receipt.broken.iter().map(|b| b.slot).collect::<Vec<_>>()},
        "content_key": hex::encode(k),
        "e": hex::encode(e),
        "nonce_w": hex::encode(nonce_w),
        "nonce_b": hex::encode(nonce_b),
        "wrap_v": hex::encode(wrap),
        "payload_hex": hex::encode(payload.encode()),
        "blob_hex": hex::encode(&blob_bytes),
        "blob_sha256": hex::encode(blob_sha),
        "commitment": hex::encode(cmd::commitment(PROGRAM_KEY, 0, hunter, &blob_sha)),
    })
}

/// A signed transaction as the CLI sends it (`Near::send`): borsh(Transaction V0), then the ed25519
/// signature over its sha256.
fn tx_vector(seed: [u8; 32], signer_id: &str, nonce: u64, receiver_id: &str, block_hash: [u8; 32], method: &str, args: Vec<u8>, tgas: u64, deposit: u128) -> Value {
    let signer = near::Signer::from_seed(signer_id.to_string(), seed);
    let action = near::Action::FunctionCall { method: method.into(), args: args.clone(), gas: tgas * near::TGAS, deposit };
    let tx = near::encode_tx(signer_id, &signer.public_key(), nonce, receiver_id, &block_hash, &[action]);
    let (signed, hash) = near::sign_tx(&tx, &signer.key);
    json!({
        "seed": hex::encode(seed),
        "public_key": signer.public_key_str(),
        "signer_id": signer_id,
        "nonce": nonce,
        "receiver_id": receiver_id,
        "block_hash": bs58::encode(block_hash).into_string(),
        "method": method,
        "args_hex": hex::encode(&args),
        "gas": (tgas * near::TGAS).to_string(),
        "deposit": deposit.to_string(),
        "tx_hex": hex::encode(&tx),
        "tx_hash": bs58::encode(hash).into_string(),
        "signed_base64": base64::engine::general_purpose::STANDARD.encode(&signed),
    })
}

fn vectors() -> Value {
    let base = std::fs::read(root().join("fixtures/web/practice-base.bin")).expect("fixtures/web/practice-base.bin");
    assert_eq!(hex::encode(sha256(&base)), BASE_SHA256, "the practice base fixture changed");
    let env = Envelope::decode(&base).unwrap();
    let Payload::Base(b) = &env.payload else { panic!("not a base witness") };
    let header: alloy_consensus::Header = alloy_rlp::decode_exact(&b.header_rlp).unwrap();
    // Round 0: the run is judged under the base's own manifest (the program's).
    let manifest_json = b.manifest_json.to_vec();
    let manifest = antibody_verifier::manifest::Manifest::parse(&manifest_json).unwrap();
    let vault = |i: u32| format!("{:#x}", manifest.targets.iter().find(|t| t.name == format!("vault-{i}")).unwrap().address);
    let hunters = ["web-vector.practice.antibody.testnet", "a-b_c-9.practice.antibody.testnet"];
    let tries = vec![
        try_vector(&base, &manifest_json, hunters[0], 4, &vault(4), B256::repeat_byte(0x07), [0x21; 32], [0x22; 12], [0x23; 12]),
        try_vector(&base, &manifest_json, hunters[1], 23, &vault(23), B256::repeat_byte(0xa5), [0x31; 32], [0x32; 12], [0x33; 12]),
    ];
    let commit_args = serde_json::to_vec(&json!({"program_id": 6, "round": 0, "commitment": tries[0]["commitment"], "terms_sha256": "2a714747a7fe11e7c534424ec7c9f9b2baca7cb04928aa3aa3b69c32aeffae67"})).unwrap();
    let vend_key = practice::public_vend_signer(VENDING);
    let practice_key = near::Signer::from_seed(hunters[0].into(), [0x41; 32]);
    json!({
        "_note": "Generated by crates/cli/tests/web_vectors.rs (GOLDEN_WRITE=1) with the code `antibody try` runs; checked there and by the web app's tests (web/)",
        "practice": {
            "base": {
                "file": "fixtures/web/practice-base.bin",
                "sha256": BASE_SHA256,
                "chain_id": env.chain_id,
                "anchor_hash": format!("{:#x}", env.claimed_block_hash),
                "anchor_block": header.number,
                "state_root": format!("{:#x}", header.state_root),
                "quorum": env.quorum.iter().map(|q| json!({"source": q.source, "block_hash": format!("{:#x}", q.block_hash)})).collect::<Vec<_>>(),
                "manifest_sha256": hex::encode(sha256(&manifest_json)),
            },
            "program_key": PROGRAM_KEY,
            "round": 0,
            "sealing_pk": SEALING_PK,
            "tries": tries,
        },
        "transactions": [
            tx_vector([0x41; 32], hunters[0], 271_000_000_001, hunters[0], [0x51; 32], "commit", commit_args, 25, 0),
            tx_vector(sha256(practice::PUBLIC_VEND_SEED.as_bytes()), VENDING, 199_000_000_123, VENDING, [0x52; 32], "vend",
                      practice::vend_args("web-vector", &practice_key.public_key()), practice::VEND_TGAS, 0),
            tx_vector([0x41; 32], hunters[0], 271_000_000_002, hunters[0], [0x53; 32], "close", b"{}".to_vec(), practice::CLOSE_TGAS, 0),
        ],
        "public_vend_key": vend_key.public_key_str(),
        "public_vend_seed": practice::PUBLIC_VEND_SEED,
    })
}

#[test]
fn web_vectors_hold() {
    let now = vectors();
    if std::env::var("GOLDEN_WRITE").is_ok() {
        std::fs::write(path(), serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
        return;
    }
    let committed: Value = serde_json::from_str(&std::fs::read_to_string(path()).expect("golden/web-vectors.json (GOLDEN_WRITE=1 writes it)")).unwrap();
    assert_eq!(now, committed, "a format the web app shares changed: if on purpose, rewrite with GOLDEN_WRITE=1 and rerun the web tests");
}

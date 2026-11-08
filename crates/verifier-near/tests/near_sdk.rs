//! A real near-sdk-rs contract (fixtures/near/status-message, near-sdk 5.9.0 built with toolchain 1.85.0)
//! executed end to end: deployment, `init`, state-changing calls (JSON arguments, borsh state, logs, a
//! payable method, a refund promise), views, and a program run whose rules read the contract by views.

mod common;

use antibody_verifier_near::{
    receipt::Kind,
    rules::{view, EvalEnv, ViewResult},
    run::{run, Input},
    util::{sha256, unb64},
    witness::{Bundle, Call},
};
use common::*;
use serde_json::json;

const WASM: &[u8] = include_bytes!("../../../fixtures/near/status-message/status_message.wasm");
const WASM_SHA256: &str = include_str!("../../../fixtures/near/status-message/status_message.wasm.sha256");

#[test]
fn the_committed_wasm_matches_its_sha256_and_is_accepted() {
    assert_eq!(hex::encode(sha256(WASM)), WASM_SHA256.split_whitespace().next().unwrap());
    let cfg = antibody_verifier_near::config::Config::protocol_74();
    let p = antibody_verifier_near::prepare::prepare(WASM, antibody_verifier_near::prepare::Kind::Contract, &cfg.limits).expect("near-sdk output is accepted");
    assert!(p.imports.iter().any(|i| i == "storage_write"), "{:?}", p.imports);
}

#[test]
fn init_calls_and_views_run_end_to_end() {
    let mut w = world(&[]);
    deploy(&mut w, "status.near", WASM, 10 * NEAR);
    let mut rt = runtime(w);
    let first = rt.transaction("status.near", pk(), "status.near", call("new", b"{}", 0, 30 * TGAS), "init", 0).unwrap();
    assert!(rt.records[first].success, "{:?}", rt.records[first].error);

    let r = tx(&mut rt, "status.near", "set_status", br#"{"message":"hello"}"#, 0);
    assert!(r[0].success, "{:?}", r[0].error);
    assert_eq!(r[0].logs, vec!["alice.near set the status to hello".to_string()]);

    let r = tx(&mut rt, "status.near", "updates", b"", 0);
    assert_eq!(r[0].value.as_deref(), Some(b64(b"1").as_str()));

    // A payable method with a deposit, and a non-payable one refusing it.
    let r = tx(&mut rt, "status.near", "donate", b"", 3 * NEAR);
    assert_eq!(unb64(r[0].value.as_deref().unwrap()).unwrap(), format!("\"{}\"", 3 * NEAR).into_bytes());
    let r = tx(&mut rt, "status.near", "set_status", br#"{"message":"x"}"#, 1);
    assert!(r[0].error.as_deref().unwrap().contains("doesn't accept deposit"), "{:?}", r[0].error);
    // A panic with near-sdk's message, and bad JSON.
    let r = tx(&mut rt, "status.near", "set_status", format!(r#"{{"message":"{}"}}"#, "y".repeat(300)).as_bytes(), 0);
    assert!(r[0].error.as_deref().unwrap().contains("the message is too long"), "{:?}", r[0].error);
    let r = tx(&mut rt, "status.near", "set_status", b"not json", 0);
    assert!(!r[0].success);

    // A promise: refund sends 1 NEAR of the deposits back to the caller in the next block.
    let alice = rt.world.accounts["alice.near"].amount;
    let r = tx(&mut rt, "status.near", "refund", format!(r#"{{"amount":"{NEAR}"}}"#).as_bytes(), 0);
    assert!(r.iter().all(|x| x.success), "{:?}", r.iter().map(|x| x.error.clone()).collect::<Vec<_>>());
    assert_eq!(r[1].receiver, "alice.near");
    assert_eq!(rt.world.accounts["alice.near"].amount, alice + NEAR);

    // Views, in NEAR's view mode, on the state the calls left.
    let (height, timestamp_ns) = rt.now();
    let env = EvalEnv {
        engine: rt.vm.engine.clone(),
        cfg: rt.cfg.clone(),
        modules: std::sync::Arc::new(rt.vm.compiled()),
        anchor_code: std::sync::Arc::new(rt.world.accounts.iter().filter_map(|(a, x)| x.code.map(|h| (a.clone(), h))).collect()),
        height,
        timestamp_ns,
        epoch_height: 7,
        random_seed: [9; 32],
    };
    let (got, _) = view(&env, &rt.world, "status.near", "get_status", br#"{"account_id":"alice.near"}"#, 100_000_000);
    assert!(matches!(got, ViewResult::Value(v) if v == b"\"hello\""));
    let (got, _) = view(&env, &rt.world, "status.near", "deposits", b"", 100_000_000);
    assert!(matches!(got, ViewResult::Value(v) if v == format!("\"{}\"", 2 * NEAR).into_bytes()));
    // A state-changing method fails in view mode.
    let (got, _) = view(&env, &rt.world, "status.near", "set_status", br#"{"message":"v"}"#, 100_000_000);
    assert!(matches!(got, ViewResult::Failed));
}

/// Slot 0: fewer than two status updates (`updates()` returns one ASCII digit below '2').
const RULES: &str = r#"(module
  (import "antibody" "view" (func $view (param i64 i64 i64 i64 i64 i64) (result i64)))
  (import "antibody" "result_len" (func $result_len (result i64)))
  (import "antibody" "result_read" (func $result_read (param i64)))
  (memory (export "memory") 1)
  (data (i32.const 0) "status.near")
  (data (i32.const 16) "updates")
  (func (export "check") (result i64)
    (if (i64.ne (call $view (i64.const 11) (i64.const 0) (i64.const 7) (i64.const 16) (i64.const 0) (i64.const 0)) (i64.const 0))
      (then (return (i64.const 1))))
    (if (i64.ne (call $result_len) (i64.const 1)) (then (return (i64.const 1))))
    (call $result_read (i64.const 64))
    (i64.extend_i32_u (i32.ge_u (i32.load8_u (i32.const 64)) (i32.const 50)))))"#;

#[test]
fn a_near_sdk_program_runs_through_the_whole_pipeline() {
    let rules = wasm(RULES);
    let (ch, rh) = (hex::encode(sha256(WASM)), hex::encode(sha256(&rules)));
    let manifest = serde_json::to_vec(&json!({
        "schema": "antibody.manifest.v2",
        "program": {"name": "status", "project": "project.testnet", "description": "near-sdk fixture"},
        "anchor": {"chain_id": 398, "block_number": 5000, "block_hash": "ab".repeat(32), "evm_spec": "near"},
        "targets": [{"name": "StatusMessage", "address": "status.near", "code_hash": ch}],
        "invariants": {"slots": [{"bit": 0, "name": "at most one update", "severity": "high"}], "gas_limit": 200_000_000u64},
        "settlement": {},
        "vm": "near",
        "near": {
            "protocol": 74,
            "block": {"height": 5000, "timestamp_ns": 1_750_000_000_000_000_000u64, "epoch_height": 3, "random_seed": "cd".repeat(32), "gas_price": "100000000"},
            "contracts": [{"account": "status.near", "wasm": {"url": "fixtures/near/status-message/status_message.wasm", "sha256": ch},
                           "init": {"method": "new", "args": b64(b"{}"), "gas": 30 * TGAS}}],
            "accounts": [{"account": "status.near", "amount": (20 * NEAR).to_string()}],
            "rules": {"wasm": {"url": "inline", "sha256": rh}, "source": "crates/verifier-near/tests/near_sdk.rs"},
            "impact": {"native_usd": "3.5", "protocol_accounts": ["status.near"]},
            "capabilities": {"max_senders": 1, "max_endowment_yocto": (5 * NEAR).to_string(), "max_calls": 3, "max_receipts": 32,
                             "max_instructions_per_call": 300_000_000u64, "max_total_instructions": 900_000_000u64, "max_eval_instructions": 900_000_000u64}
        }
    }))
    .unwrap();
    let codes = vec![WASM.to_vec(), rules];
    let call = |m: &str| Call { signer: 0, receiver: "status.near".into(), method: "set_status".into(), args: b64(format!(r#"{{"message":"{m}"}}"#).as_bytes()), deposit: "0".into(), gas: 30 * TGAS };
    let bundle = |calls: Vec<Call>| Bundle { sender_salt: "44".repeat(32), senders: 1, endowment_yocto: NEAR.to_string(), calls };
    let go = |b: &Bundle| run(&Input { manifest_json: &manifest, codes: &codes, bundle: Some(b), program: "antibody.testnet/12", commit_id: 1, witness_sha256: [2; 32], expected_manifest_hash: Some(sha256(&manifest)) });

    let one = go(&bundle(vec![call("gm")]));
    assert_eq!(one.kind, Kind::NoViolation, "{:?}", one.reason);
    assert!(one.checks.as_ref().unwrap().doubled_gas_run_identical);
    let two_bundle = bundle(vec![call("gm"), call("gn")]);
    let two = go(&two_bundle);
    assert_eq!(two.kind, Kind::Violation, "{:?}", two.reason);
    let f = &two.broken[0];
    assert_eq!((f.slot, f.method.as_str(), f.call_index), (0, "set_status", Some(1)));
    assert_eq!(two.canonical(), go(&two_bundle).canonical(), "deterministic");

    // When ANTIBODY_WRITE_FIXTURES=1, write the witness JSON the antibody-verify-v2 binary runs locally.
    if std::env::var("ANTIBODY_WRITE_FIXTURES").as_deref() == Ok("1") {
        let witness = serde_json::json!({
            "schema": "antibody.near-witness.v1",
            "manifest": String::from_utf8(manifest.clone()).unwrap(),
            "codes": codes.iter().map(|c| b64(c)).collect::<Vec<_>>(),
            "bundle": two_bundle,
        });
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/near/status-witness.json");
        std::fs::write(path, serde_json::to_vec(&witness).unwrap()).unwrap();
        eprintln!("wrote {path}");
    }
}

//! A NEAR program end to end (run::run): manifest and module checks, fork-mode activation, the rules at
//! the anchor and after each call, root causes and groups, impact, the doubled-cost run, the bundle's
//! bounds, and determinism (same inputs, same receipt bytes).

mod common;

use antibody_verifier_near::{
    receipt::Kind,
    run::{run, Input},
    util::sha256,
    witness::{Bundle, Call},
};
use common::*;
use serde_json::{json, Value};

/// The vault: `deposit` adds the attached deposit to "total"; `total` returns it (u128 LE); `withdraw`
/// sends anyone any amount (16-byte LE input): the bug.
fn vault() -> String {
    contract(
        r#"
  (data (i32.const 0) "total")
  (func $load_total
    (if (i64.eq (call $storage_read (i64.const 5) (i64.const 0) (i64.const 0)) (i64.const 1))
      (then (call $read_register (i64.const 0) (i64.const 96)))
      (else (i64.store (i32.const 96) (i64.const 0)) (i64.store (i32.const 104) (i64.const 0)))))
  (func (export "deposit") (local $lo i64)
    (call $load_total)
    (call $attached_deposit (i64.const 64))
    (local.set $lo (i64.add (i64.load (i32.const 96)) (i64.load (i32.const 64))))
    (i64.store (i32.const 104)
      (i64.add (i64.add (i64.load (i32.const 104)) (i64.load (i32.const 72)))
               (i64.extend_i32_u (i64.lt_u (local.get $lo) (i64.load (i32.const 96))))))
    (i64.store (i32.const 96) (local.get $lo))
    (drop (call $storage_write (i64.const 5) (i64.const 0) (i64.const 16) (i64.const 96) (i64.const 1))))
  (func (export "total") (call $load_total) (call $value_return (i64.const 16) (i64.const 96)))
  (func (export "withdraw") (local $p i64)
    (call $input (i64.const 0))
    (call $read_register (i64.const 0) (i64.const 128))
    (call $predecessor_account_id (i64.const 1))
    (call $read_register (i64.const 1) (i64.const 256))
    (local.set $p (call $promise_batch_create (call $register_len (i64.const 1)) (i64.const 256)))
    (call $promise_batch_action_transfer (local.get $p) (i64.const 128)))
  (func (export "noop"))
"#,
    )
}

/// Slot 0 ("solvent"): the vault's balance covers its recorded deposits. `trap` makes a broken rule trap
/// instead of returning its bit (an evaluation that fails).
fn rules(trap: bool) -> String {
    let broken = if trap { "(unreachable)" } else { "(return (i64.const 1))" };
    format!(
        r#"(module
  (import "antibody" "view" (func $view (param i64 i64 i64 i64 i64 i64) (result i64)))
  (import "antibody" "result_read" (func $result_read (param i64)))
  (import "antibody" "balance" (func $balance (param i64 i64 i64) (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 0) "vault.near")
  (data (i32.const 16) "total")
  (func (export "check") (result i64)
    (if (i64.ne (call $view (i64.const 10) (i64.const 0) (i64.const 5) (i64.const 16) (i64.const 0) (i64.const 0)) (i64.const 0))
      (then {broken}))
    (call $result_read (i64.const 64))
    (drop (call $balance (i64.const 10) (i64.const 0) (i64.const 96)))
    (if (i64.lt_u (i64.load (i32.const 104)) (i64.load (i32.const 72))) (then {broken}))
    (if (i64.gt_u (i64.load (i32.const 104)) (i64.load (i32.const 72))) (then (return (i64.const 0))))
    (if (i64.lt_u (i64.load (i32.const 96)) (i64.load (i32.const 64))) (then {broken}))
    (i64.const 0)))"#
    )
}

const ALWAYS_HOLDS: &str = r#"(module (memory (export "memory") 1) (func (export "check") (result i64) (i64.const 0)))"#;

struct Program {
    manifest: Vec<u8>,
    codes: Vec<Vec<u8>>,
}

fn program(target: &str, target_wat: &str, rules_wat: &str, edit: impl FnOnce(&mut Value)) -> Program {
    let code = wasm(target_wat);
    let rules = wasm(rules_wat);
    let (ch, rh) = (hex::encode(sha256(&code)), hex::encode(sha256(&rules)));
    let mut m = json!({
        "schema": "antibody.manifest.v2",
        "program": {"name": "vault", "project": "project.testnet", "description": "a NEAR program under test"},
        "anchor": {"chain_id": 398, "block_number": 1000, "block_hash": "11".repeat(32), "evm_spec": "near"},
        "targets": [{"name": "Target", "address": target, "code_hash": ch}],
        "invariants": {"slots": [{"bit": 0, "name": "solvent", "severity": "critical"}], "gas_limit": 50_000_000},
        "settlement": {},
        "vm": "near",
        "near": {
            "protocol": 74,
            "block": {"height": 1000, "timestamp_ns": 1_700_000_000_000_000_000u64, "epoch_height": 7, "random_seed": "22".repeat(32), "gas_price": "100000000"},
            "contracts": [{"account": target, "wasm": {"url": "fixtures/target.wasm", "sha256": ch}}],
            "accounts": [
                {"account": target, "amount": (100 * NEAR).to_string()},
                {"account": "alice.near", "amount": (1000 * NEAR).to_string()}
            ],
            "setup": [{"signer": "alice.near", "receiver": target, "method": "deposit", "deposit": (10 * NEAR).to_string(), "gas": 30 * TGAS}],
            "rules": {"wasm": {"url": "fixtures/rules.wasm", "sha256": rh}, "source": "crates/verifier-near/tests/program.rs"},
            "impact": {"native_usd": "2", "tokens": [], "protocol_accounts": [target]},
            "capabilities": {
                "max_senders": 2, "max_endowment_yocto": (10 * NEAR).to_string(), "max_calls": 4, "max_receipts": 64,
                "max_instructions_per_call": 100_000_000u64, "max_total_instructions": 1_000_000_000u64, "max_eval_instructions": 1_000_000_000u64
            }
        }
    });
    edit(&mut m);
    Program { manifest: serde_json::to_vec(&m).unwrap(), codes: vec![code, rules] }
}

fn vault_program(edit: impl FnOnce(&mut Value)) -> Program {
    program("vault.near", &vault(), &rules(false), edit)
}

fn bundle(calls: &[(&str, &str, Vec<u8>)]) -> Bundle {
    Bundle {
        sender_salt: "33".repeat(32),
        senders: 1,
        endowment_yocto: NEAR.to_string(),
        calls: calls
            .iter()
            .map(|(r, m, a)| Call { signer: 0, receiver: r.to_string(), method: m.to_string(), args: b64(a), deposit: "0".into(), gas: 30 * TGAS })
            .collect(),
    }
}

fn verdict(p: &Program, b: Option<&Bundle>) -> antibody_verifier_near::receipt::NearReceipt {
    run(&Input { manifest_json: &p.manifest, codes: &p.codes, bundle: b, program: "antibody.testnet/9", commit_id: 3, witness_sha256: [1; 32], expected_manifest_hash: None })
}

fn amount(n: u128) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}

#[test]
fn activation_alone_checks_the_program_at_the_anchor() {
    let r = verdict(&vault_program(|_| {}), None);
    assert_eq!(r.kind, Kind::NoViolation, "{:?}", r.reason);
    assert!(r.checks.as_ref().unwrap().anchor_rules_hold);
    assert_eq!(r.anchor.as_ref().unwrap().chain_id, 398);
    // The setup deposit ran at the anchor.
    assert!(r.near.receipts.iter().any(|x| x.phase == "setup" && x.success));
}

#[test]
fn a_draining_call_is_a_violation_with_its_entry_point_and_impact() {
    let p = vault_program(|_| {});
    let r = verdict(&p, Some(&bundle(&[("vault.near", "noop", vec![]), ("vault.near", "withdraw", amount(105 * NEAR))])));
    assert_eq!(r.kind, Kind::Violation, "{:?}", r.reason);
    assert_eq!(r.broken_mask, "0x1");
    let f = &r.broken[0];
    assert_eq!((f.slot, f.receiver.as_str(), f.method.as_str(), f.call_index, f.group), (0, "vault.near", "withdraw", Some(1), None));
    assert!(r.checks.as_ref().unwrap().doubled_gas_run_identical);
    let i = r.impact.as_ref().unwrap();
    // 105 NEAR at $2: $210 lost by the vault, gained by the sender.
    assert_eq!((i.protocol_loss_usd_micro.as_str(), i.attacker_gain_usd_micro.as_str(), i.impact_usd_micro.as_str()), ("210000000", "210000000", "210000000"));
    assert_eq!(i.impact_usd, "210.00");
    assert_eq!(r.senders.len(), 1);
    assert_eq!(r.senders[0].len(), 64, "a NEAR-implicit sender");

    // When ANTIBODY_WRITE_FIXTURES=1, write this program's witness (small WAT modules only) as a fixture.
    if std::env::var("ANTIBODY_WRITE_FIXTURES").as_deref() == Ok("1") {
        let b = bundle(&[("vault.near", "noop", vec![]), ("vault.near", "withdraw", amount(105 * NEAR))]);
        let witness = serde_json::json!({
            "schema": "antibody.near-witness.v1",
            "manifest": String::from_utf8(p.manifest.clone()).unwrap(),
            "codes": p.codes.iter().map(|c| b64(c)).collect::<Vec<_>>(),
            "bundle": b,
        });
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/near/vault-witness.json");
        std::fs::write(path, serde_json::to_vec(&witness).unwrap()).unwrap();
    }
}

#[test]
fn a_harmless_bundle_is_no_violation() {
    let r = verdict(&vault_program(|_| {}), Some(&bundle(&[("vault.near", "withdraw", amount(NEAR))])));
    assert_eq!(r.kind, Kind::NoViolation, "{:?}", r.reason);
    assert!(r.broken.is_empty());
}

#[test]
fn a_committed_group_names_the_root_cause() {
    let p = vault_program(|m| {
        m["near"]["groups"] = json!([{"id": 3, "name": "exits", "members": [{"account": "vault.near", "method": "withdraw"}]}]);
    });
    let r = verdict(&p, Some(&bundle(&[("vault.near", "withdraw", amount(105 * NEAR))])));
    assert_eq!(r.broken[0].group, Some(3));
}

#[test]
fn the_rules_must_hold_and_evaluate_at_the_anchor() {
    let p = program("vault.near", &vault(), r#"(module (memory (export "memory") 1) (func (export "check") (result i64) (i64.const 1)))"#, |_| {});
    let r = verdict(&p, Some(&bundle(&[("vault.near", "noop", vec![])])));
    assert_eq!(r.kind, Kind::ProgramMisconfigured);
    assert!(r.reason.as_deref().unwrap().contains("do not hold at the anchor"));
    let p = program("vault.near", &vault(), r#"(module (memory (export "memory") 1) (func (export "check") (result i64) (unreachable)))"#, |_| {});
    assert_eq!(verdict(&p, None).kind, Kind::ProgramMisconfigured);
}

#[test]
fn a_rule_that_fails_after_the_proof_is_unevaluable() {
    let p = program("vault.near", &vault(), &rules(true), |_| {});
    let r = verdict(&p, Some(&bundle(&[("vault.near", "withdraw", amount(105 * NEAR))])));
    assert_eq!(r.kind, Kind::Unevaluable);
    assert!(r.reason.as_deref().unwrap().contains("the rules failed after the proof"), "{:?}", r.reason);
}

#[test]
fn a_refused_host_function_is_accepted_when_imported_and_stops_the_run_when_called() {
    // Importing a refused host function is accepted (near-sdk links the whole surface), as nearcore accepts it.
    let imports_pairing = vault().replacen("(module ", "(module (import \"env\" \"alt_bn128_pairing_check\" (func $bn (param i64 i64) (result i64))) ", 1);
    let r = verdict(&program("vault.near", &imports_pairing, &rules(false), |_| {}), None);
    assert_eq!(r.kind, Kind::NoViolation, "importing but not calling it: {:?}", r.reason);
    // Calling one makes the run Unevaluable, by name.
    let calls_pairing = vault().replacen(
        "(module ",
        "(module (import \"env\" \"alt_bn128_pairing_check\" (func $bn (param i64 i64) (result i64))) ",
        1,
    )
    .replace("(func (export \"noop\"))", "(func (export \"noop\") (drop (call $bn (i64.const 0) (i64.const 0))))");
    let r = verdict(&program("vault.near", &calls_pairing, &rules(false), |_| {}), Some(&bundle(&[("vault.near", "noop", vec![])])));
    assert_eq!(r.kind, Kind::Unevaluable, "{:?}", r.reason);
    assert!(r.reason.as_deref().unwrap().contains("alt_bn128_pairing_check"), "{:?}", r.reason);
}

#[test]
fn bad_manifests_are_the_programs_fault() {
    for (field, value) in [("chain_id", json!(1)), ("evm_spec", json!("osaka"))] {
        let r = verdict(&vault_program(|m| m["anchor"][field] = value), None);
        assert_eq!(r.kind, Kind::ProgramMisconfigured, "{field}");
    }
    let r = verdict(&vault_program(|m| m["targets"][0]["code_hash"] = json!("00".repeat(32))), None);
    assert_eq!(r.kind, Kind::ProgramMisconfigured);
}

#[test]
fn the_bundle_is_held_to_the_committed_capabilities() {
    let p = vault_program(|_| {});
    let mut b = bundle(&[("vault.near", "noop", vec![])]);
    b.senders = 3;
    assert_eq!(verdict(&p, Some(&b)).kind, Kind::InvalidPoC, "senders");
    let mut b = bundle(&[("vault.near", "noop", vec![])]);
    b.endowment_yocto = (11 * NEAR).to_string();
    assert_eq!(verdict(&p, Some(&b)).kind, Kind::InvalidPoC, "endowment");
    let b = bundle(&vec![("vault.near", "noop", vec![]); 5]);
    assert_eq!(verdict(&p, Some(&b)).kind, Kind::InvalidPoC, "calls");
    let mut b = bundle(&[("vault.near", "noop", vec![])]);
    b.calls[0].gas = 301 * TGAS;
    assert_eq!(verdict(&p, Some(&b)).kind, Kind::InvalidPoC, "gas");
}

/// `gassy` stores the low byte of `used_gas` under "k" (so its storage depends on the gas schedule);
/// `steady` stores a constant. Both ignore the input.
fn gas_branch() -> String {
    contract(
        r#"
  (data (i32.const 0) "k")
  (data (i32.const 8) "Z")
  (func (export "deposit"))
  (func (export "gassy")
    (i64.store (i32.const 16) (call $used_gas))
    (drop (call $storage_write (i64.const 1) (i64.const 0) (i64.const 1) (i64.const 16) (i64.const 1))))
  (func (export "steady")
    (drop (call $storage_write (i64.const 1) (i64.const 0) (i64.const 1) (i64.const 8) (i64.const 1))))
"#,
    )
}

#[test]
fn a_gas_dependent_outcome_is_unevaluable_by_the_doubled_run() {
    let p = program("branch.near", &gas_branch(), ALWAYS_HOLDS, |_| {});
    // `used_gas` doubles between the two runs, so the stored byte differs: Unevaluable.
    let r = verdict(&p, Some(&bundle(&[("branch.near", "gassy", vec![])])));
    assert_eq!(r.kind, Kind::Unevaluable, "{:?}", r.reason);
    assert!(r.reason.as_deref().unwrap().contains("doubled-cost run differs"), "{:?}", r.reason);
    // A call whose storage does not depend on gas agrees in both runs.
    let r = verdict(&p, Some(&bundle(&[("branch.near", "steady", vec![])])));
    assert_eq!(r.kind, Kind::NoViolation, "{:?}", r.reason);
    assert!(r.checks.unwrap().doubled_gas_run_identical);
}

#[test]
fn the_same_inputs_give_the_same_receipt_bytes() {
    let p = vault_program(|_| {});
    let b = bundle(&[("vault.near", "withdraw", amount(105 * NEAR))]);
    let (a, c) = (verdict(&p, Some(&b)), verdict(&p, Some(&b)));
    assert_eq!(a.canonical(), c.canonical());
    assert_eq!(a.hash(), c.hash());
}

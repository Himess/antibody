//! Receipts and promises: cross-contract calls, callbacks and their results, promise_return forwarding,
//! the per-pass block step, rollback of a failed receipt, deposit refunds, storage staking, batch actions
//! and their permissions, weighted gas, and the receipt and depth bounds.

mod common;

use antibody_verifier_near::runtime::{Halt, ReceiptRecord};
use common::*;

/// `callee.near`: `echo` returns its input; `fail` panics; `write_then_fail` writes then panics;
/// `height` returns the block height; `pgas` returns its prepaid gas.
fn callee() -> String {
    contract(
        r#"
  (data (i32.const 0) "x")
  (data (i32.const 8) "no")
  (func (export "echo") (call $input (i64.const 0)) (call $value_return (i64.const -1) (i64.const 0)))
  (func (export "fail") (call $panic_utf8 (i64.const 2) (i64.const 8)))
  (func (export "write_then_fail")
    (drop (call $storage_write (i64.const 1) (i64.const 0) (i64.const 1) (i64.const 0) (i64.const 0)))
    (call $panic_utf8 (i64.const 2) (i64.const 8)))
  (func (export "height") (i64.store (i32.const 16) (call $block_index)) (call $value_return (i64.const 8) (i64.const 16)))
  (func (export "pgas") (i64.store (i32.const 16) (call $prepaid_gas)) (call $value_return (i64.const 8) (i64.const 16)))
"#,
    )
}

/// `caller.near`: calls `callee.near` and handles the result in `on_result`, which returns the value
/// or "FAILED".
fn caller() -> String {
    contract(
        r#"
  (data (i32.const 0) "callee.near")
  (data (i32.const 16) "echo")
  (data (i32.const 24) "fail")
  (data (i32.const 32) "caller.near")
  (data (i32.const 48) "on_result")
  (data (i32.const 64) "hi")
  (data (i32.const 72) "FAILED")
  (data (i32.const 80) "forward")
  (data (i32.const 96) "fw")
  (data (i32.const 104) "pgas")
  (func $then (param $p i64)
    (drop (call $promise_then (local.get $p) (i64.const 11) (i64.const 32) (i64.const 9) (i64.const 48) (i64.const 0) (i64.const 0) (i64.const 200) (i64.const 10000000000000))))
  (func (export "call_echo")
    (call $then (call $promise_create (i64.const 11) (i64.const 0) (i64.const 4) (i64.const 16) (i64.const 2) (i64.const 64) (i64.const 200) (i64.const 10000000000000))))
  (func (export "call_fail")
    (call $then (call $promise_create (i64.const 11) (i64.const 0) (i64.const 4) (i64.const 24) (i64.const 0) (i64.const 0) (i64.const 200) (i64.const 10000000000000))))
  (func (export "forward")
    (call $promise_return (call $promise_create (i64.const 11) (i64.const 0) (i64.const 4) (i64.const 16) (i64.const 2) (i64.const 96) (i64.const 200) (i64.const 10000000000000))))
  (func (export "call_forward")
    (call $then (call $promise_create (i64.const 11) (i64.const 32) (i64.const 7) (i64.const 80) (i64.const 0) (i64.const 0) (i64.const 200) (i64.const 30000000000000))))
  (func (export "on_result")
    (if (i64.eq (call $promise_result (i64.const 0) (i64.const 0)) (i64.const 1))
      (then (call $value_return (i64.const -1) (i64.const 0)))
      (else (call $value_return (i64.const 6) (i64.const 72)))))
  (func (export "weighted") (local $p i64)
    (local.set $p (call $promise_batch_create (i64.const 11) (i64.const 0)))
    (call $promise_batch_action_function_call_weight (local.get $p) (i64.const 4) (i64.const 104) (i64.const 0) (i64.const 0) (i64.const 200) (i64.const 1000000000000) (i64.const 1)))
"#,
    )
}

fn errors(r: &[ReceiptRecord]) -> Vec<String> {
    r.iter().map(|x| x.error.clone().unwrap_or_default()).collect()
}

#[test]
fn a_callback_gets_the_callees_value_one_block_later() {
    let mut rt = runtime(world(&[("caller.near", &caller()), ("callee.near", &callee())]));
    let r = tx(&mut rt, "caller.near", "call_echo", b"", 0);
    let route: Vec<(&str, &str)> = r.iter().map(|x| (x.receiver.as_str(), x.actions[0].as_str())).collect();
    assert_eq!(route, vec![("caller.near", "FunctionCall(call_echo, 0)"), ("callee.near", "FunctionCall(echo, 0)"), ("caller.near", "FunctionCall(on_result, 0)")]);
    assert!(r.iter().all(|x| x.success), "{:?}", errors(&r));
    assert_eq!(r[2].value.as_deref(), Some(b64(b"hi").as_str()));
    // One pass per block: the callee runs one block after the call, the callback one after that.
    assert_eq!((r[0].block_height, r[1].block_height, r[2].block_height), (1000, 1001, 1002));
    assert_eq!(r[1].predecessor, "caller.near");
}

#[test]
fn a_callback_sees_a_failed_promise() {
    let mut rt = runtime(world(&[("caller.near", &caller()), ("callee.near", &callee())]));
    let r = tx(&mut rt, "caller.near", "call_fail", b"", 0);
    assert!(!r[1].success && r[1].error.as_deref().unwrap().contains("GuestPanic"));
    assert!(r[2].success);
    assert_eq!(r[2].value.as_deref(), Some(b64(b"FAILED").as_str()));
}

#[test]
fn a_returned_promise_forwards_its_value_to_the_callback() {
    let mut rt = runtime(world(&[("caller.near", &caller()), ("callee.near", &callee())]));
    let r = tx(&mut rt, "caller.near", "call_forward", b"", 0);
    // call_forward → forward (returns echo's promise) → echo("fw") → on_result gets "fw".
    let last = r.last().unwrap();
    assert_eq!(last.actions[0], "FunctionCall(on_result, 0)");
    assert_eq!(last.value.as_deref(), Some(b64(b"fw").as_str()), "{:?}", errors(&r));
    let fwd = r.iter().find(|x| x.actions[0].starts_with("FunctionCall(forward")).unwrap();
    assert!(fwd.value.as_deref().unwrap().starts_with("receipt:"));
}

#[test]
fn a_failed_receipt_rolls_back_only_its_own_changes() {
    let mut rt = runtime(world(&[("callee.near", &callee())]));
    let before = rt.world.accounts["callee.near"].clone();
    let alice = rt.world.accounts["alice.near"].amount;
    let r = tx(&mut rt, "callee.near", "write_then_fail", b"", 2 * NEAR);
    assert!(!r[0].success);
    assert_eq!(rt.world.accounts["callee.near"], before, "storage, usage and balance are as before");
    assert_eq!(rt.world.accounts["alice.near"].amount, alice, "the deposit was refunded");
}

#[test]
fn storage_staking_fails_a_receipt_that_cannot_pay_for_its_state() {
    // `fat` writes 10 KB: 10^4 bytes x 10^19 yocto = 0.1 NEAR of stake, more than the account holds.
    let fat = contract(
        r#"(func (export "grow") (drop (call $storage_write (i64.const 1) (i64.const 0) (i64.const 10000) (i64.const 0) (i64.const 0))))"#,
    );
    let mut w = world(&[]);
    deploy(&mut w, "fat.near", &wasm(&fat), NEAR / 50);
    let mut rt = runtime(w);
    let before = rt.world.accounts["fat.near"].clone();
    let r = tx(&mut rt, "fat.near", "grow", b"", 0);
    assert!(r[0].error.as_deref().unwrap().contains("LackBalanceForState"), "{:?}", errors(&r));
    assert_eq!(rt.world.accounts["fat.near"], before);
    // With enough balance the same call succeeds.
    rt.world.accounts.get_mut("fat.near").unwrap().amount = NEAR;
    assert!(tx(&mut rt, "fat.near", "grow", b"", 0)[0].success);
}

#[test]
fn weighted_gas_goes_to_the_weighted_call() {
    let mut rt = runtime(world(&[("caller.near", &caller()), ("callee.near", &callee())]));
    let r = tx(&mut rt, "caller.near", "weighted", b"", 0);
    let v = antibody_verifier_near::util::unb64(r[1].value.as_deref().unwrap()).unwrap();
    let prepaid = u64::from_le_bytes(v.try_into().unwrap());
    assert!(prepaid > 50 * TGAS, "1 Tgas attached plus the caller's unused gas: {prepaid}");
}

/// `factory.near`: `make` creates `sub.factory.near` with 5 NEAR, a key and the callee's code;
/// `make_bad` tries to create a top-level account; `deploy_other` deploys to another account;
/// `bye` deletes itself in favour of alice.
fn factory(code_len: usize) -> String {
    let five = wat_bytes(&(5 * NEAR).to_le_bytes());
    contract(&format!(
        r#"
  (data (i32.const 0) "sub.factory.near")
  (data (i32.const 32) "{five}")
  (data (i32.const 64) "\00\01\02\03\04\05\06\07\08\09\0a\0b\0c\0d\0e\0f\10\11\12\13\14\15\16\17\18\19\1a\1b\1c\1d\1e\1f\20")
  (data (i32.const 112) "toplevel")
  (data (i32.const 128) "callee.near")
  (data (i32.const 144) "alice.near")
  (func (export "make") (local $p i64)
    (call $input (i64.const 0))
    (call $read_register (i64.const 0) (i64.const 4096))
    (local.set $p (call $promise_batch_create (i64.const 16) (i64.const 0)))
    (call $promise_batch_action_create_account (local.get $p))
    (call $promise_batch_action_transfer (local.get $p) (i64.const 32))
    (call $add_key (local.get $p) (i64.const 33) (i64.const 64) (i64.const 0))
    (call $promise_batch_action_deploy_contract (local.get $p) (i64.const {code_len}) (i64.const 4096)))
  (func (export "make_bad") (local $p i64)
    (local.set $p (call $promise_batch_create (i64.const 8) (i64.const 112)))
    (call $promise_batch_action_create_account (local.get $p))
    (call $promise_batch_action_transfer (local.get $p) (i64.const 32)))
  (func (export "deploy_other") (local $p i64)
    (local.set $p (call $promise_batch_create (i64.const 11) (i64.const 128)))
    (call $promise_batch_action_deploy_contract (local.get $p) (i64.const 8) (i64.const 64)))
  (func (export "bye")
    (call $current_account_id (i64.const 1))
    (call $read_register (i64.const 1) (i64.const 160))
    (call $promise_batch_action_delete_account
      (call $promise_batch_create (call $register_len (i64.const 1)) (i64.const 160)) (i64.const 10) (i64.const 144)))
"#
    ))
}

#[test]
fn batch_actions_create_fund_key_and_deploy_with_nearcores_permissions() {
    let code = wasm(&callee());
    let mut rt = runtime(world(&[("factory.near", &factory(code.len())), ("callee.near", &callee())]));
    let r = tx(&mut rt, "factory.near", "make", &code, 0);
    assert!(r.iter().all(|x| x.success), "{:?}", errors(&r));
    let sub = &rt.world.accounts["sub.factory.near"];
    assert_eq!(sub.amount, 5 * NEAR);
    assert_eq!(sub.keys.len(), 1);
    assert_eq!(sub.code, Some(antibody_verifier_near::util::sha256(&code)));
    // 100 (account) + 33 + 9 + 40 (the key) + the code.
    assert_eq!(sub.storage_usage, 100 + 82 + code.len() as u64);
    assert_eq!(rt.created_accounts, vec!["sub.factory.near".to_string()]);
    // The new account runs its code.
    assert_eq!(tx(&mut rt, "sub.factory.near", "echo", b"yo", 0)[0].value.as_deref(), Some(b64(b"yo").as_str()));

    // A short top-level account needs the registrar: the receipt fails and the 5 NEAR come back.
    let before = rt.world.accounts["factory.near"].amount;
    let r = tx(&mut rt, "factory.near", "make_bad", b"", 0);
    assert!(r[1].error.as_deref().unwrap().contains("CreateAccountOnlyByRegistrar"), "{:?}", errors(&r));
    assert_eq!(r[2].receiver, "factory.near", "refund");
    assert_eq!(rt.world.accounts["factory.near"].amount, before);
    assert!(!rt.world.accounts.contains_key("toplevel"));

    // Deploying to another account is not the caller's to do.
    let r = tx(&mut rt, "factory.near", "deploy_other", b"", 0);
    assert!(r[1].error.as_deref().unwrap().contains("ActorNoPermission"), "{:?}", errors(&r));

    // Deleting an account sends its balance to the beneficiary.
    let alice = rt.world.accounts["alice.near"].amount;
    let left = rt.world.accounts["factory.near"].amount;
    let r = tx(&mut rt, "factory.near", "bye", b"", 0);
    assert!(r.iter().all(|x| x.success), "{:?}", errors(&r));
    assert!(!rt.world.accounts.contains_key("factory.near"));
    assert_eq!(rt.world.accounts["alice.near"].amount, alice + left);
}

/// `loop.near`'s `go` calls itself: an endless promise chain.
fn looping() -> String {
    contract(
        r#"
  (data (i32.const 0) "loop.near")
  (data (i32.const 16) "go")
  (func (export "go")
    (call $promise_batch_action_function_call_weight
      (call $promise_batch_create (i64.const 9) (i64.const 0))
      (i64.const 2) (i64.const 16) (i64.const 0) (i64.const 0) (i64.const 200) (i64.const 0) (i64.const 1)))
"#,
    )
}

#[test]
fn receipt_and_depth_bounds_stop_the_run() {
    let mut rt = runtime(world(&[("loop.near", &looping())]));
    let e = rt.transaction("alice.near", pk(), "loop.near", call("go", b"", 0, 300 * TGAS), "call", 0).unwrap_err();
    assert!(matches!(&e, Halt::Bound(m) if m.contains("deeper than 16")), "{e:?}");
    let mut rt = runtime(world(&[("loop.near", &looping())]));
    rt.max_depth = 1_000;
    rt.max_receipts = 10;
    let e = rt.transaction("alice.near", pk(), "loop.near", call("go", b"", 0, 300 * TGAS), "call", 0).unwrap_err();
    assert!(matches!(&e, Halt::Bound(m) if m.contains("more than 10 receipts")), "{e:?}");
}

#[test]
fn a_transfer_creates_a_near_implicit_account() {
    let mut rt = runtime(world(&[]));
    let id = "ab".repeat(32);
    let first = rt
        .transaction("alice.near", pk(), &id, vec![antibody_verifier_near::logic::Action::Transfer { deposit: NEAR }], "call", 0)
        .unwrap();
    assert!(rt.records[first].success);
    let a = &rt.world.accounts[&id];
    assert_eq!((a.amount, a.storage_usage, a.keys.len()), (NEAR, 182, 1));
}

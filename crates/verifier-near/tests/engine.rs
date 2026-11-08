//! The engine core: one function call end to end, and each host function group.

mod common;

use common::*;

/// `set(input)`: stores the input under "k" and logs it; `get()`: returns the stored value; `del()`:
/// removes "k" and returns whether it is still there.
fn store_contract() -> String {
    contract(
        r#"
  (data (i32.const 0) "k")
  (func (export "set")
    (call $input (i64.const 0))
    (call $read_register (i64.const 0) (i64.const 100))
    (drop (call $storage_write (i64.const 1) (i64.const 0) (call $register_len (i64.const 0)) (i64.const 100) (i64.const 1)))
    (call $log_utf8 (call $register_len (i64.const 0)) (i64.const 100)))
  (func (export "get")
    (if (i64.eq (call $storage_read (i64.const 1) (i64.const 0) (i64.const 2)) (i64.const 1))
      (then
        (call $read_register (i64.const 2) (i64.const 200))
        (call $value_return (call $register_len (i64.const 2)) (i64.const 200)))))
  (func (export "del")
    (drop (call $storage_remove (i64.const 1) (i64.const 0) (i64.const 3)))
    (i64.store8 (i32.const 300) (call $storage_has_key (i64.const 1) (i64.const 0)))
    (call $value_return (i64.const 1) (i64.const 300)))
"#,
    )
}

#[test]
fn a_function_call_runs_end_to_end() {
    let mut rt = runtime(world(&[("store.near", &store_contract())]));
    let before = rt.world.accounts["store.near"].storage_usage;
    let r = tx(&mut rt, "store.near", "set", b"hello", 0);
    assert_eq!(r.len(), 1);
    assert!(r[0].success, "{:?}", r[0].error);
    assert_eq!(r[0].logs, vec!["hello".to_string()]);
    assert!(r[0].gas_burnt > 0);
    let acc = &rt.world.accounts["store.near"];
    assert_eq!(acc.data.get(b"k".as_slice()).map(|v| v.as_slice()), Some(b"hello".as_slice()));
    assert_eq!(acc.storage_usage, before + 1 + 5 + 40, "a new record counts key + value + 40 bytes");
    let r = tx(&mut rt, "store.near", "get", b"", 0);
    assert_eq!(r[0].value.as_deref(), Some(b64(b"hello").as_str()));
    // Overwriting changes the usage by the value's length difference only.
    tx(&mut rt, "store.near", "set", b"hi", 0);
    assert_eq!(rt.world.accounts["store.near"].storage_usage, before + 1 + 2 + 40);
    // Removing gives the record's bytes back.
    let r = tx(&mut rt, "store.near", "del", b"", 0);
    assert_eq!(r[0].value.as_deref(), Some(b64(&[0]).as_str()), "has_key after remove is 0");
    assert_eq!(rt.world.accounts["store.near"].storage_usage, before);
    assert!(rt.world.accounts["store.near"].data.is_empty());
}

fn context_contract() -> String {
    contract(
        r#"
  (data (i32.const 100) "oops")
  (data (i32.const 200) "tesdsst")
  (func (export "who") (call $predecessor_account_id (i64.const 0)) (call $value_return (i64.const -1) (i64.const 0)))
  (func (export "me") (call $current_account_id (i64.const 0)) (call $value_return (i64.const -1) (i64.const 0)))
  (func (export "signer") (call $signer_account_id (i64.const 0)) (call $value_return (i64.const -1) (i64.const 0)))
  (func (export "height") (i64.store (i32.const 0) (call $block_index)) (call $value_return (i64.const 8) (i64.const 0)))
  (func (export "time") (i64.store (i32.const 0) (call $block_timestamp)) (call $value_return (i64.const 8) (i64.const 0)))
  (func (export "deposit") (call $attached_deposit (i64.const 0)) (call $value_return (i64.const 16) (i64.const 0)))
  (func (export "balance") (call $account_balance (i64.const 0)) (call $value_return (i64.const 16) (i64.const 0)))
  (func (export "registers")
    (call $write_register (i64.const 5) (i64.const 4) (i64.const 100))
    (i64.store (i32.const 0) (call $register_len (i64.const 5)))
    (i64.store (i32.const 8) (call $register_len (i64.const 6)))
    (call $value_return (i64.const 16) (i64.const 0)))
  (func (export "bad_register") (call $read_register (i64.const 42) (i64.const 0)))
  (func (export "boom") (call $panic_utf8 (i64.const 4) (i64.const 100)))
  (func (export "logs") (local $i i32)
    (loop $l
      (call $log_utf8 (i64.const 1) (i64.const 100))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const 101)))))
  (func (export "sha") (call $sha256 (i64.const 7) (i64.const 200) (i64.const 0)) (call $value_return (i64.const -1) (i64.const 0)))
  (func (export "iter") (drop (call $storage_iter_prefix (i64.const 0) (i64.const 0))))
  (func (export "oob") (call $value_return (i64.const 16) (i64.const 67108860)))
  (func (export "trap") unreachable)
"#,
    )
}

#[test]
fn context_registers_math_logs_and_errors() {
    let mut rt = runtime(world(&[("ctx.near", &context_contract())]));
    let val = |r: &[antibody_verifier_near::runtime::ReceiptRecord]| r[0].value.clone().unwrap_or_default();
    let err = |r: &[antibody_verifier_near::runtime::ReceiptRecord]| r[0].error.clone().unwrap_or_default();
    assert_eq!(val(&tx(&mut rt, "ctx.near", "who", b"", 0)), b64(b"alice.near"));
    assert_eq!(val(&tx(&mut rt, "ctx.near", "me", b"", 0)), b64(b"ctx.near"));
    assert_eq!(val(&tx(&mut rt, "ctx.near", "signer", b"", 0)), b64(b"alice.near"));
    // Each transaction above took one pass (one block): the fourth runs at height 1000 + 3.
    assert_eq!(val(&tx(&mut rt, "ctx.near", "height", b"", 0)), b64(&1003u64.to_le_bytes()));
    assert_eq!(val(&tx(&mut rt, "ctx.near", "time", b"", 0)), b64(&(block().timestamp_ns + 4_000_000_000).to_le_bytes()));
    assert_eq!(val(&tx(&mut rt, "ctx.near", "deposit", b"", 7)), b64(&7u128.to_le_bytes()));
    let bal = rt.world.accounts["ctx.near"].amount;
    assert_eq!(val(&tx(&mut rt, "ctx.near", "balance", b"", 5)), b64(&(bal + 5).to_le_bytes()), "the balance includes the deposit");
    let mut regs = 4u64.to_le_bytes().to_vec();
    regs.extend_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(val(&tx(&mut rt, "ctx.near", "registers", b"", 0)), b64(&regs), "an unused register's length is u64::MAX");
    assert!(err(&tx(&mut rt, "ctx.near", "bad_register", b"", 0)).contains("InvalidRegisterId { register_id: 42 }"));
    let sha = antibody_verifier_near::crypto::sha256(b"tesdsst");
    assert_eq!(val(&tx(&mut rt, "ctx.near", "sha", b"", 0)), b64(&sha));
    assert!(err(&tx(&mut rt, "ctx.near", "iter", b"", 0)).contains("Deprecated"));
    assert!(err(&tx(&mut rt, "ctx.near", "oob", b"", 0)).contains("MemoryAccessViolation"));
    assert!(err(&tx(&mut rt, "ctx.near", "trap", b"", 0)).contains("WasmTrap(Unreachable)"));
    let r = tx(&mut rt, "ctx.near", "logs", b"", 0);
    assert!(err(&r).contains("NumberOfLogsExceeded { limit: 100 }"));
    assert_eq!(r[0].logs.len(), 100, "a failed call keeps the logs it wrote");
    // A panic fails the receipt with its message and refunds the deposit to the predecessor.
    let alice = rt.world.accounts["alice.near"].amount;
    let r = tx(&mut rt, "ctx.near", "boom", b"", 3 * NEAR);
    assert!(err(&r).contains("GuestPanic { panic_msg: \"oops\" }"), "{}", err(&r));
    assert_eq!(r.len(), 2, "the failed receipt and its refund");
    assert_eq!((r[1].predecessor.as_str(), r[1].receiver.as_str(), r[1].success), ("system", "alice.near", true));
    assert_eq!(rt.world.accounts["alice.near"].amount, alice, "the deposit came back");
    let r = tx(&mut rt, "ctx.near", "missing", b"", 0);
    assert!(err(&r).contains("MethodResolveError(MethodNotFound)"));
}

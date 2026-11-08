//! Test helpers: a world with WAT contracts, a runtime over it, and one-call transactions.
#![allow(dead_code)]

use std::sync::Arc;

use antibody_verifier_near::{
    config::Config,
    exec::Vm,
    logic::Action,
    runtime::{Block, ReceiptRecord, Runtime},
    state::{Account, World},
    util::sha256,
};

pub const NEAR: u128 = 1_000_000_000_000_000_000_000_000;
pub const TGAS: u64 = 1_000_000_000_000;

pub fn wasm(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).expect("valid WAT")
}

pub fn block() -> Block {
    Block { height: 1_000, timestamp_ns: 1_700_000_000_000_000_000, epoch_height: 7, random_seed: [9; 32] }
}

/// A world with `alice.near` (no code) and each `(account, wat)` deployed with 100 NEAR.
pub fn world(contracts: &[(&str, &str)]) -> World {
    let mut w = World::default();
    w.accounts.insert("alice.near".into(), Account::new(100 * NEAR, 0));
    for (account, wat) in contracts {
        deploy(&mut w, account, &wasm(wat), 100 * NEAR);
    }
    w
}

pub fn deploy(w: &mut World, account: &str, code: &[u8], amount: u128) {
    let h = sha256(code);
    let mut a = Account::new(amount, 0);
    a.storage_usage += code.len() as u64;
    a.code = Some(h);
    w.codes.insert(h, Arc::new(code.to_vec()));
    w.accounts.insert(account.into(), a);
}

pub fn runtime(w: World) -> Runtime {
    Runtime::new(Vm::default(), Arc::new(Config::protocol_74()), w, block())
}

pub fn call(method: &str, args: &[u8], deposit: u128, gas: u64) -> Vec<Action> {
    vec![Action::FunctionCall { method: method.into(), args: args.to_vec(), deposit, gas, weight: 0 }]
}

pub fn pk() -> Vec<u8> {
    let mut v = vec![0u8];
    v.extend_from_slice(&[7; 32]);
    v
}

/// Runs `method` on `receiver` from alice; returns the records of the whole receipt tree.
pub fn tx(rt: &mut Runtime, receiver: &str, method: &str, args: &[u8], deposit: u128) -> Vec<ReceiptRecord> {
    let first = rt.transaction("alice.near", pk(), receiver, call(method, args, deposit, 100 * TGAS), "call", 0).expect("no halt");
    rt.records[first..].to_vec()
}

pub fn b64(s: &[u8]) -> String {
    antibody_verifier_near::util::b64(s)
}

/// The WAT import list of a contract module.
pub const IMPORTS: &str = r#"
  (import "env" "input" (func $input (param i64)))
  (import "env" "register_len" (func $register_len (param i64) (result i64)))
  (import "env" "read_register" (func $read_register (param i64 i64)))
  (import "env" "value_return" (func $value_return (param i64 i64)))
  (import "env" "log_utf8" (func $log_utf8 (param i64 i64)))
  (import "env" "panic_utf8" (func $panic_utf8 (param i64 i64)))
  (import "env" "storage_write" (func $storage_write (param i64 i64 i64 i64 i64) (result i64)))
  (import "env" "storage_read" (func $storage_read (param i64 i64 i64) (result i64)))
  (import "env" "storage_remove" (func $storage_remove (param i64 i64 i64) (result i64)))
  (import "env" "storage_has_key" (func $storage_has_key (param i64 i64) (result i64)))
  (import "env" "storage_usage" (func $storage_usage (result i64)))
  (import "env" "predecessor_account_id" (func $predecessor_account_id (param i64)))
  (import "env" "current_account_id" (func $current_account_id (param i64)))
  (import "env" "signer_account_id" (func $signer_account_id (param i64)))
  (import "env" "account_balance" (func $account_balance (param i64)))
  (import "env" "attached_deposit" (func $attached_deposit (param i64)))
  (import "env" "block_index" (func $block_index (result i64)))
  (import "env" "block_timestamp" (func $block_timestamp (result i64)))
  (import "env" "prepaid_gas" (func $prepaid_gas (result i64)))
  (import "env" "used_gas" (func $used_gas (result i64)))
  (import "env" "promise_create" (func $promise_create (param i64 i64 i64 i64 i64 i64 i64 i64) (result i64)))
  (import "env" "promise_then" (func $promise_then (param i64 i64 i64 i64 i64 i64 i64 i64 i64) (result i64)))
  (import "env" "promise_and" (func $promise_and (param i64 i64) (result i64)))
  (import "env" "promise_batch_create" (func $promise_batch_create (param i64 i64) (result i64)))
  (import "env" "promise_batch_action_transfer" (func $promise_batch_action_transfer (param i64 i64)))
  (import "env" "promise_batch_action_create_account" (func $promise_batch_action_create_account (param i64)))
  (import "env" "promise_batch_action_deploy_contract" (func $promise_batch_action_deploy_contract (param i64 i64 i64)))
  (import "env" "promise_batch_action_function_call" (func $promise_batch_action_function_call (param i64 i64 i64 i64 i64 i64 i64)))
  (import "env" "promise_batch_action_function_call_weight" (func $promise_batch_action_function_call_weight (param i64 i64 i64 i64 i64 i64 i64 i64)))
  (import "env" "promise_batch_action_delete_account" (func $promise_batch_action_delete_account (param i64 i64 i64)))
  (import "env" "promise_batch_action_add_key_with_full_access" (func $add_key (param i64 i64 i64 i64)))
  (import "env" "promise_results_count" (func $promise_results_count (result i64)))
  (import "env" "promise_result" (func $promise_result (param i64 i64) (result i64)))
  (import "env" "promise_return" (func $promise_return (param i64)))
  (import "env" "sha256" (func $sha256 (param i64 i64 i64)))
  (import "env" "storage_iter_prefix" (func $storage_iter_prefix (param i64 i64) (result i64)))
  (import "env" "write_register" (func $write_register (param i64 i64 i64)))
"#;

/// Bytes as a WAT string literal's escapes.
pub fn wat_bytes(b: &[u8]) -> String {
    b.iter().map(|x| format!("\\{x:02x}")).collect()
}

/// A contract module: the imports above, a memory, `data` segments and `body` functions.
pub fn contract(body: &str) -> String {
    format!("(module {IMPORTS} (memory (export \"memory\") 1) {body})")
}

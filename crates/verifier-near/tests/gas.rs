//! The gas rules (docs/near-targets.md §3, §13): fuel and gas exhaustion and the 10% margin make a run
//! `Unevaluable`; a float instruction makes it `Unevaluable` when executed, not when merely present.

mod common;

use antibody_verifier_near::runtime::Halt;
use common::*;

fn gassy() -> String {
    contract(
        r#"
  (func (export "spin") (loop $l (br $l)))
  ;; Loops until it has used `num/10` of its prepaid gas (num from the input's first byte).
  (func (export "burn") (local $num i64)
    (call $input (i64.const 0))
    (call $read_register (i64.const 0) (i64.const 0))
    (local.set $num (i64.load8_u (i32.const 0)))
    (loop $l
      (br_if $l (i64.lt_u (i64.mul (call $used_gas) (i64.const 10)) (i64.mul (call $prepaid_gas) (local.get $num))))))
  (func (export "float_run") (drop (f64.add (f64.const 1) (f64.const 2))))
  (func (export "float_dead") (local $x i32)
    (if (local.get $x) (then (drop (f64.add (f64.const 1) (f64.const 2))))))
"#,
    )
}

fn halt(method: &str, input: &[u8], gas: u64, fuel: u64) -> Result<bool, Halt> {
    let mut rt = runtime(world(&[("gas.near", &gassy())]));
    rt.fuel_per_call = fuel;
    let first = rt.transaction("alice.near", pk(), "gas.near", call(method, input, 0, gas), "call", 0)?;
    Ok(rt.records[first].success)
}

fn unevaluable(r: Result<bool, Halt>) -> String {
    match r {
        Err(Halt::Unevaluable(why)) => why,
        other => panic!("expected Unevaluable, got {other:?}"),
    }
}

#[test]
fn exhaustion_is_unevaluable_never_a_near_failure() {
    // Gas runs out first: 300 Tgas buys about 364 million instructions at 822,756 gas each.
    let why = unevaluable(halt("spin", b"", 300 * TGAS, 10_000_000_000));
    assert!(why.starts_with("gas:"), "{why}");
    // The instruction budget runs out first.
    let why = unevaluable(halt("spin", b"", 300 * TGAS, 1_000_000));
    assert!(why.starts_with("instructions:"), "{why}");
}

#[test]
fn the_ten_percent_margin() {
    assert_eq!(halt("burn", &[5], 20 * TGAS, u64::MAX), Ok(true), "half the gas is fine");
    assert!(unevaluable(halt("burn", &[95 / 10], 20 * TGAS, u64::MAX)).contains("within 10%"), "90% or more is not");
}

#[test]
fn floats_are_refused_when_executed_not_when_present() {
    assert!(unevaluable(halt("float_run", b"", 10 * TGAS, u64::MAX)).starts_with("float:"));
    assert_eq!(halt("float_dead", b"", 10 * TGAS, u64::MAX), Ok(true));
}

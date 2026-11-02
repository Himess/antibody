use k256::ecdsa::SigningKey;
use near_sdk::{json_types::U128, test_utils::VMContextBuilder, testing_env, AccountId, Gas, PromiseError};

use super::*;

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn ctx(pred: &str) {
    let mut b = VMContextBuilder::new();
    b.current_account_id(acc("guardian.testnet"))
        .predecessor_account_id(acc(pred))
        .block_index(77)
        .prepaid_gas(Gas::from_tgas(1000));
    testing_env!(b.build());
}

/// Deterministic test key standing in for the MPC network.
fn test_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32].into()).unwrap()
}

fn address_of(k: &SigningKey) -> String {
    let pt = k.verifying_key().to_encoded_point(false);
    format!("0x{}", to_hex(&env::keccak256(&pt.as_bytes()[1..])[12..]))
}

fn target(chain_id: u64, name: &str) -> ChainTarget {
    ChainTarget {
        chain_id,
        name: name.into(),
        to: "0x53ED61ba0bC2CF14DF82d0bDE82C4e8ac2A82462".into(),
        calldata: PAUSE_CALLDATA.into(),
        gas_limit: 80_000,
        max_fee_per_gas: U128(30_000_000_000),
        max_priority_fee_per_gas: U128(2_000_000_000),
        next_nonce: 0,
        explorer: "https://example.org/tx/".into(),
    }
}

fn guardian() -> Guardian {
    ctx("owner.testnet");
    let mut g = Guardian::new(acc("owner.testnet"), acc("v1.signer-prod.testnet"), "guardian/acme".into(), address_of(&test_key()));
    g.set_chain(target(11155111, "sepolia"));
    g.set_chain(target(84532, "base-sepolia"));
    g.set_authorized(acc("bounty.testnet"), true);
    g
}

/// What the MPC network returns: big_r = compressed R (parity from recovery id), s, recovery id.
fn mpc_sign(payload_hex: &str) -> SignatureResponse {
    let hash = hex_to_bytes(payload_hex).unwrap();
    let (sig, recid) = test_key().sign_prehash_recoverable(&hash).unwrap();
    let (r, s) = sig.split_bytes();
    let parity = recid.is_y_odd() as u8;
    SignatureResponse {
        big_r: BigR { affine_point: format!("{:02x}{}", 2 + parity, to_hex(&r)) },
        s: Scalar { scalar: to_hex(&s) },
        recovery_id: parity,
    }
}

#[test]
fn pause_all_signs_every_chain_with_its_own_nonce() {
    let mut g = guardian();
    let p0_sep = g.preview(11155111, 0);
    let p0_base = g.preview(84532, 0);
    assert_ne!(p0_sep, p0_base, "chain id is part of the signed payload");
    ctx("bounty.testnet");
    let _ = g.pause_all("bounty.testnet/0".into(), 1);
    let chains = g.get_chains();
    assert!(chains.iter().all(|c| c.next_nonce == 1));
    ctx("guardian.testnet");
    let out = g
        .on_signed(Ok(mpc_sign(&p0_sep)), 0, 11155111, 0, p0_sep.clone(), "bounty.testnet/0#1".into(), 77)
        .expect("signature accepted");
    assert_eq!(out.recovered_address, address_of(&test_key()));
    assert!(out.signed_tx.starts_with("0x02"), "EIP-1559 typed transaction");
    assert_eq!(out.tx_hash, format!("0x{}", to_hex(&env::keccak256(&hex_to_bytes(&out.signed_tx).unwrap()))));
    let out_b = g.on_signed(Ok(mpc_sign(&p0_base)), 1, 84532, 0, p0_base, "bounty.testnet/0#1".into(), 77).unwrap();
    assert_ne!(out.signed_tx, out_b.signed_tx);
    assert_eq!(g.get_latest(84532).unwrap().request_id, 1);
    // Printed for the external check with `cast decode-transaction` (scripts/demo/check_guardian_tx.sh).
    println!("GUARDIAN_ADDRESS={}", address_of(&test_key()));
    println!("SIGNED_SEPOLIA={}", out.signed_tx);
    println!("SIGNED_BASE={}", out_b.signed_tx);
}

#[test]
fn only_owner_and_authorized_callers_can_pause() {
    let mut g = guardian();
    ctx("mallory.testnet");
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = g.pause_all("x".into(), 0);
    }))
    .is_err());
    ctx("mallory.testnet");
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.set_chain(target(1, "mainnet")))).is_err());
}

#[test]
fn signatures_from_another_key_or_mpc_failures_are_rejected() {
    let mut g = guardian();
    let payload = g.preview(11155111, 0);
    ctx("bounty.testnet");
    let _ = g.pause_all("bounty.testnet/0".into(), 1);
    ctx("guardian.testnet");
    let other = SigningKey::from_bytes(&[9u8; 32].into()).unwrap();
    let (sig, recid) = other.sign_prehash_recoverable(&hex_to_bytes(&payload).unwrap()).unwrap();
    let (r, s) = sig.split_bytes();
    let forged = SignatureResponse {
        big_r: BigR { affine_point: format!("{:02x}{}", 2 + recid.is_y_odd() as u8, to_hex(&r)) },
        s: Scalar { scalar: to_hex(&s) },
        recovery_id: recid.is_y_odd() as u8,
    };
    assert!(g.on_signed(Ok(forged), 0, 11155111, 0, payload.clone(), "r".into(), 77).is_none());
    assert!(g.on_signed(Err(PromiseError::Failed), 1, 84532, 0, payload, "r".into(), 77).is_none());
    assert_eq!(g.get_config()["failures"], near_sdk::serde_json::json!(2));
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn promise_yield_create(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> u64 {
    unreachable!()
}
#[cfg(windows)]
#[no_mangle]
pub extern "C" fn promise_yield_resume(_: u64, _: u64, _: u64, _: u64) -> u32 {
    unreachable!()
}

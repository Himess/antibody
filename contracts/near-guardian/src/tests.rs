use k256::ecdsa::SigningKey;
use near_sdk::{test_utils::get_logs, test_utils::VMContextBuilder, testing_env, AccountId, Gas, NearToken, PromiseError};

use super::*;

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn ctx(pred: &str, deposit: u128) {
    let mut b = VMContextBuilder::new();
    b.current_account_id(acc("guardian.testnet"))
        .predecessor_account_id(acc(pred))
        .attached_deposit(NearToken::from_yoctonear(deposit))
        .block_timestamp(1_800_000_000 * 1_000_000_000)
        .prepaid_gas(Gas::from_tgas(300));
    testing_env!(b.build());
}

/// Deterministic test key standing in for the MPC network's derived key.
fn test_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32].into()).unwrap()
}

fn derived(k: &SigningKey) -> String {
    let pt = k.verifying_key().to_encoded_point(false);
    format!("secp256k1:{}", bs58::encode(&pt.as_bytes()[1..]).into_string())
}

const ADAPTER: &str = "0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f";
const PROXY: &str = "0x00000000000000000000000000000000000000aa";
const PROGRAM: &str = "antibody.testnet/0";

fn chains() -> Vec<ChainAdapter> {
    vec![
        ChainAdapter { chain_id: 11155111, adapter: ADAPTER.into(), proxies: vec![PROXY.into()] },
        ChainAdapter { chain_id: 84532, adapter: ADAPTER.into(), proxies: vec![] },
    ]
}

/// A guardian with acme's program registered and its derived key pinned.
fn registered() -> Guardian {
    ctx("owner.testnet", 0);
    let mut g = Guardian::new(acc("owner.testnet"), acc("v1.signer-prod.testnet"), acc("antibody.testnet"));
    ctx("acme.testnet", 1_000);
    let _ = g.register(PROGRAM.into(), chains());
    ctx("guardian.testnet", 0);
    let address = g.on_key(PROGRAM.into(), acc("acme.testnet"), Ok(derived(&test_key()))).unwrap();
    assert_eq!(g.get_program(PROGRAM.into(), acc("acme.testnet")).unwrap().address, Some(address));
    g
}

fn sign(digest: &str, k: &SigningKey) -> SignatureResponse {
    let d = hex::decode(digest).unwrap();
    let (sig, recid) = k.sign_prehash_recoverable(&d).unwrap();
    let (r, s) = sig.split_bytes();
    let parity = recid.is_y_odd() as u8;
    SignatureResponse {
        big_r: BigR { affine_point: format!("{:02x}{}", 2 + parity, hex::encode(r)) },
        s: Scalar { scalar: hex::encode(s) },
        recovery_id: parity,
    }
}

fn stub(body: PermitBody) -> SignedPermit {
    let deadline = 1_800_000_000 + PERMIT_TTL_SECS;
    SignedPermit {
        id: 0,
        program_key: PROGRAM.into(),
        project: acc("acme.testnet"),
        chain_id: 11155111,
        adapter: ADAPTER.into(),
        round: 0,
        digest: hex::encode(permit_digest(11155111, ADAPTER, PROGRAM, 0, &body, deadline)),
        body,
        deadline,
        v: 0,
        r: String::new(),
        s: String::new(),
        requested_ns: 0,
        signed_ns: 0,
    }
}

#[test]
fn digests_match_the_adapter() {
    // Computed with `cast` from AntibodyAdapter's type strings; contracts/evm pins the same values.
    ctx("x.testnet", 0);
    let pause = permit_digest(31337, ADAPTER, PROGRAM, 1, &PermitBody::Pause { ticket: 7 }, 1_800_000_600);
    assert_eq!(hex::encode(pause), "44a11a91a91f7ecaa83ec2bccee5ee7930b1d7e6ac43a64a57aa33599fad2cf6");
    let release = permit_digest(31337, ADAPTER, PROGRAM, 2, &PermitBody::Release { proxy: PROXY.into(), code_hash: "cd".repeat(32) }, 1_800_000_600);
    assert_eq!(hex::encode(release), "2fb89742e62ff6246b32da341ad8a57667a3938dd15b403c3aee01e88e32b487");
}

#[test]
fn only_near_antibody_asks_and_each_chain_gets_a_permit() {
    let mut g = registered();
    ctx("antibody.testnet", 0);
    assert_eq!(g.issue_pause(PROGRAM.into(), acc("acme.testnet"), 0, 3), 2);
    assert_eq!(g.get_program(PROGRAM.into(), acc("acme.testnet")).unwrap().credit.0, 1_000 - 2 * SIGN_FEE);
}

#[test]
#[should_panic(expected = "only near-antibody asks for permits")]
fn nobody_else_asks() {
    let mut g = registered();
    ctx("acme.testnet", 0);
    g.issue_pause(PROGRAM.into(), acc("acme.testnet"), 0, 3);
}

#[test]
#[should_panic(expected = "no guardian registration")]
fn a_registration_by_another_account_is_never_used() {
    // eve registers acme's program key; near-antibody asks for acme's: eve's entry does not count.
    ctx("owner.testnet", 0);
    let mut g = Guardian::new(acc("owner.testnet"), acc("mpc.testnet"), acc("antibody.testnet"));
    ctx("eve.testnet", 1_000);
    let _ = g.register(PROGRAM.into(), chains());
    ctx("antibody.testnet", 0);
    g.issue_pause(PROGRAM.into(), acc("acme.testnet"), 0, 3);
}

#[test]
fn releases_only_for_registered_proxies() {
    let mut g = registered();
    ctx("antibody.testnet", 0);
    let r = vec![ReleaseIn { proxy: PROXY.into(), code_hash: "cd".repeat(32) }, ReleaseIn { proxy: ADAPTER.into(), code_hash: "cd".repeat(32) }];
    // One registered proxy, on one chain.
    assert_eq!(g.issue_release(PROGRAM.into(), acc("acme.testnet"), 1, r), 1);
}

/// A round that releases two proxies on two chains needs four signatures, more than a call from
/// near-antibody (200 Tgas) has gas for: the rest are queued, and anyone's `issue_pending` asks for them.
#[test]
fn permits_beyond_one_calls_gas_are_queued_then_issued() {
    let mut g = registered();
    ctx("acme.testnet", 0);
    let both = vec![PROXY.to_string(), "0x00000000000000000000000000000000000000bb".to_string()];
    g.set_chains(PROGRAM.into(), vec![
        ChainAdapter { chain_id: 11155111, adapter: ADAPTER.into(), proxies: both.clone() },
        ChainAdapter { chain_id: 84532, adapter: ADAPTER.into(), proxies: both.clone() },
    ]);
    let mut b = VMContextBuilder::new();
    b.current_account_id(acc("guardian.testnet")).predecessor_account_id(acc("antibody.testnet")).prepaid_gas(Gas::from_tgas(200));
    testing_env!(b.build());
    let r: Vec<ReleaseIn> = both.iter().map(|p| ReleaseIn { proxy: p.clone(), code_hash: "cd".repeat(32) }).collect();
    let first = g.issue_release(PROGRAM.into(), acc("acme.testnet"), 2, r) as usize;
    assert!((1..4).contains(&first), "one call signs some, not all");
    assert_eq!(first + g.pending_permits().len(), 4, "nothing is dropped");
    ctx("keeper.testnet", 0);
    assert_eq!(g.issue_pending() as usize, 4 - first);
    assert!(g.pending_permits().is_empty());
    assert_eq!(g.get_program(PROGRAM.into(), acc("acme.testnet")).unwrap().permits, 4);
}

#[test]
fn a_signature_is_kept_only_if_it_recovers_to_the_programs_address() {
    let mut g = registered();
    ctx("guardian.testnet", 0);
    let s = stub(PermitBody::Pause { ticket: 3 });
    let id = g.on_signed(s.clone(), Ok(sign(&s.digest, &test_key()))).expect("kept");
    let p = g.get_permit(id).unwrap();
    assert!(p.v == 27 || p.v == 28);
    assert!(hex::decode(&p.s).unwrap().as_slice() <= &HALF_N[..], "low-s form");
    // A signature by another key is dropped.
    let other = SigningKey::from_bytes(&[9u8; 32].into()).unwrap();
    assert_eq!(g.on_signed(s.clone(), Ok(sign(&s.digest, &other))), None);
    assert!(get_logs().iter().any(|l| l.contains("permit_failed")));
    // The MPC network failing is a typed failure, not a panic.
    assert_eq!(g.on_signed(s, Err(PromiseError::Failed)), None);
}

#[test]
fn a_high_s_signature_is_normalised() {
    let mut g = registered();
    ctx("guardian.testnet", 0);
    let s = stub(PermitBody::Pause { ticket: 4 });
    let mut sig = sign(&s.digest, &test_key());
    // The malleable twin: n - s with the other parity recovers the same key.
    let s32: [u8; 32] = hex::decode(&sig.s.scalar).unwrap().try_into().unwrap();
    sig.s.scalar = hex::encode(neg_mod_n(&s32));
    sig.recovery_id ^= 1;
    let id = g.on_signed(s, Ok(sig)).expect("normalised and kept");
    assert!(hex::decode(&g.get_permit(id).unwrap().s).unwrap().as_slice() <= &HALF_N[..]);
}

#[test]
fn no_permit_without_a_derived_key_or_credit() {
    ctx("owner.testnet", 0);
    let mut g = Guardian::new(acc("owner.testnet"), acc("mpc.testnet"), acc("antibody.testnet"));
    ctx("acme.testnet", 0);
    let _ = g.register(PROGRAM.into(), chains());
    ctx("antibody.testnet", 0);
    assert_eq!(g.issue_pause(PROGRAM.into(), acc("acme.testnet"), 0, 3), 0, "the key is not pinned yet");
    ctx("guardian.testnet", 0);
    g.on_key(PROGRAM.into(), acc("acme.testnet"), Ok(derived(&test_key())));
    ctx("antibody.testnet", 0);
    assert_eq!(g.issue_pause(PROGRAM.into(), acc("acme.testnet"), 0, 3), 0, "no MPC credit");
}

// near-sdk 5.9's mocked blockchain does not implement yield/resume; the MSVC linker wants the
// symbols anyway (never called).
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

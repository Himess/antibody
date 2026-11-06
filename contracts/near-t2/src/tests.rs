//! The contract's tests. Real SP1 proofs (fixtures/zk/<case>/proof.bin, from SP1's prover network) check
//! the pinned key; proofs under the TEST key (fixtures/zk/t2-test-key.json, anyone can make one) drive
//! the submit and record paths with the real public values and near-antibody's testnet records.

use near_sdk::{
    serde_json::{self, json},
    test_utils::{get_logs, VMContextBuilder},
    testing_env, Gas, NearToken,
};

use super::*;

const ANTIBODY: &str = "bounty.antibody.testnet";
const VERIFIER: &str = "antibody-verifier 0.1.0 revm 38.0.0 reth v2.2.0 rsp 5afe72eb12a2d6accd9d7fdff66785d91cbc8e72";
const WASM: &str = "ccc27f0ca60c503c46f6f1b2024b54481709fdd0c8849fe8d9a8912c637b28c1";
/// The guest's program vkey: SP1's docker build of crates/zk/guest (docs/zk.md).
const PROGRAM_VKEY: &str = "0x00e87aaad0ca36c285eec6bccab0cace523784873ad65e45b3b65e2759288203";

fn root(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn test_key() -> Groth16KeyJson {
    serde_json::from_slice(&std::fs::read(root("../../fixtures/zk/t2-test-key.json")).unwrap()).unwrap()
}

fn chain(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root(&format!("tests/data/chain/{name}.json"))).unwrap()).unwrap()
}

fn public_values(case: &str) -> Vec<u8> {
    std::fs::read(root(&format!("../../fixtures/zk/{case}/public_values.bin"))).unwrap()
}

/// The four disclosed testnet runs: (case, run, commit, program).
const CASES: [(&str, u64, u64, u32); 4] = [("a", 33, 13, 5), ("d", 36, 16, 5), ("c", 35, 15, 5), ("practice", 53, 20, 6)];

fn ctx(predecessor: &str, deposit: NearToken) {
    let mut b = VMContextBuilder::new();
    b.current_account_id("zk.antibody.testnet".parse().unwrap())
        .predecessor_account_id(predecessor.parse().unwrap())
        .signer_account_id(predecessor.parse().unwrap())
        .attached_deposit(deposit)
        .account_balance(NearToken::from_near(5))
        .prepaid_gas(Gas::from_tgas(300));
    testing_env!(b.build());
}

fn contract(key: Option<Groth16KeyJson>, program_vkey: &str) -> T2 {
    ctx("antibody.testnet", NearToken::from_yoctonear(0));
    T2::new(ANTIBODY.parse().unwrap(), program_vkey.into(), VERIFIER.into(), WASM.into(), key)
}

/// A G1 point (NEAR's encoding) as big-endian (x, y).
fn g1_be(p: &[u8]) -> Vec<u8> {
    [p[..32].iter().rev().copied().collect::<Vec<_>>(), p[32..64].iter().rev().copied().collect()].concat()
}

/// A proof under the TEST key: B = G2, C = IC₀, A = α + L + C.
fn forge(program_vkey: &str, pv: &[u8]) -> Vec<u8> {
    let k = Sp1Groth16Vk::try_from(&test_key()).unwrap();
    let digest = groth16::public_values_digest(pv);
    let nonce = [0u8; 32];
    let vkey = unhex::<32>(program_vkey).unwrap();
    let inputs: [&[u8]; 5] = [&vkey, &digest, &[0u8; 32], &k.vk_root, &nonce];
    let mut msm = vec![];
    for (i, x) in inputs.iter().enumerate() {
        msm.extend_from_slice(&k.ic[i + 1]);
        msm.extend(x.iter().rev());
    }
    let l = env::alt_bn128_g1_multiexp(&msm);
    let c = k.ic[0];
    let mut sum = vec![];
    for p in [&k.alpha[..], &l[..], &k.ic[0][..], &c[..]] {
        sum.push(0);
        sum.extend_from_slice(p);
    }
    let a = env::alt_bn128_g1_sum(&sum);
    // The G2 generator, big-endian (x.c1, x.c0, y.c1, y.c0): the key's −G2 with y negated back is the
    // generator; its x is the key's x.
    let neg = &k.beta_neg;
    let rev = |b: &[u8]| b.iter().rev().copied().collect::<Vec<u8>>();
    let p_minus = |y: &[u8]| -> Vec<u8> {
        // p − y, little-endian in, big-endian out.
        const P: [u8; 32] = [
            0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d, 0x97, 0x81, 0x6a, 0x91, 0x68, 0x71, 0xca,
            0x8d, 0x3c, 0x20, 0x8c, 0x16, 0xd8, 0x7c, 0xfd, 0x47,
        ];
        let y = rev(y);
        let mut out = [0u8; 32];
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let mut d = P[i] as i16 - y[i] as i16 - borrow;
            borrow = if d < 0 { 1 } else { 0 };
            if d < 0 {
                d += 256;
            }
            out[i] = d as u8;
        }
        out.to_vec()
    };
    let b = [rev(&neg[32..64]), rev(&neg[..32]), p_minus(&neg[96..128]), p_minus(&neg[64..96])].concat();
    let mut proof = k.selector.to_vec();
    proof.extend([0u8; 32]);
    proof.extend(k.vk_root);
    proof.extend(nonce);
    proof.extend(g1_be(&a));
    proof.extend(b);
    proof.extend(g1_be(&c));
    proof
}

fn submits(t: &mut T2, run_id: u64, pv: &[u8], proof: &[u8]) -> Result<(), String> {
    ctx("anyone.testnet", RECORD_DEPOSIT);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = t.submit(U64(run_id), Base64VecU8(pv.to_vec()), Base64VecU8(proof.to_vec()));
    }))
    .map_err(|e| e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default())
}

fn callback(t: &mut T2, run_id: u64, pv: &[u8], run: Value) -> bool {
    let (_, _, commit, program) = CASES.iter().find(|c| c.1 == run_id).copied().unwrap();
    ctx("zk.antibody.testnet", NearToken::from_yoctonear(0));
    t.on_records(
        Base64VecU8(pv.to_vec()),
        "00".repeat(32),
        "anyone.testnet".parse().unwrap(),
        U128(RECORD_DEPOSIT.as_yoctonear()),
        Ok(Some(run)),
        Ok(Some(chain(&format!("commit-{commit}")))),
        Ok(Some(chain(&format!("program-{program}")))),
        Ok(Some(chain(&format!("meta-{program}")))),
        Ok(Some(chain(&format!("pins-{program}")))),
    )
}

fn event(name: &str) -> Option<Value> {
    get_logs().iter().filter_map(|l| l.strip_prefix("EVENT_JSON:")).map(|l| serde_json::from_str::<Value>(l).unwrap()).find(|e| e["event"] == name)
}

#[test]
fn a_valid_proof_of_each_disclosed_run_is_recorded_as_t2() {
    let mut t = contract(Some(test_key()), PROGRAM_VKEY);
    for (case, run, _, _) in CASES {
        let pv = public_values(case);
        let proof = forge(PROGRAM_VKEY, &pv);
        submits(&mut t, run, &pv, &proof).unwrap_or_else(|e| panic!("{case}: {e}"));
        assert!(callback(&mut t, run, &pv, chain(&format!("run-{run}"))), "{case}");
        let e = event("t2_confirmed").unwrap_or_else(|| panic!("{case}: {:?}", get_logs()));
        assert_eq!(e["standard"], "antibody-t2");
        assert_eq!(e["data"][0]["run_id"], run);
        let r = t.get_record(U64(run)).unwrap();
        assert_eq!((r["outcome"].as_str(), r["receipt_sha256"].clone()), (Some("confirmed"), r["tee_receipt_sha256"].clone()));
        // Once recorded, a run takes no second record.
        assert!(submits(&mut t, run, &pv, &proof).unwrap_err().contains("already"), "{case}");
    }
    assert_eq!(t.get_stats(), json!({"records": 4, "confirmed": 4, "contradicted": 0}));
    assert_eq!(t.list_records(None, None).len(), 4);
    assert_eq!(t.get_pins()["groth16"]["is_sp1_v6_10_0"], false, "the TEST key");
}

#[test]
fn a_tampered_proof_a_wrong_vkey_another_runs_proof_or_changed_public_values_are_refused() {
    let mut t = contract(Some(test_key()), PROGRAM_VKEY);
    let pv = public_values("a");
    let proof = forge(PROGRAM_VKEY, &pv);
    // Tampered: A's x changed.
    let mut bad = proof.clone();
    bad[100 + 31] ^= 1;
    assert!(submits(&mut t, 33, &pv, &bad).is_err(), "a tampered proof");
    // Another program's proof (its vkey is a public input).
    let other = forge("0x0011223344556677889900112233445566778899001122334455667788990011", &pv);
    assert!(submits(&mut t, 33, &pv, &other).unwrap_err().contains("does not verify"), "a wrong vkey");
    // Another run's proof, submitted for this run.
    let d = public_values("d");
    assert!(submits(&mut t, 33, &d, &forge(PROGRAM_VKEY, &d)).unwrap_err().contains("another run"));
    // Changed public values under the original proof.
    let mut changed = pv.clone();
    changed[40] ^= 1;
    assert!(submits(&mut t, 33, &changed, &proof).unwrap_err().contains("does not verify"), "changed public values");
    // Not enough deposit for the record.
    ctx("anyone.testnet", NearToken::from_millinear(1));
    let short = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = t.submit(U64(33), Base64VecU8(pv.clone()), Base64VecU8(proof.clone()));
    }));
    assert!(short.is_err());
    // The contract pinned to another program vkey refuses the guest's proofs.
    let mut other_contract = contract(Some(test_key()), "0x0011223344556677889900112233445566778899001122334455667788990011");
    assert!(submits(&mut other_contract, 33, &pv, &proof).unwrap_err().contains("does not verify"));
    assert!(submits(&mut t, 33, &pv, &proof).is_ok(), "the untouched proof still verifies");
}

#[test]
fn a_tee_that_lied_is_contradicted_and_a_proof_about_another_input_is_rejected() {
    let mut t = contract(Some(test_key()), PROGRAM_VKEY);
    let pv = public_values("a");
    // The TEE reported NoViolation for A's input (another receipt, no keys).
    let mut lied = chain("run-33");
    lied["outcome"]["Answered"]["facts"]["receipt_sha256"] = json!("11".repeat(32));
    lied["outcome"]["Answered"]["facts"]["kind"] = json!("NoViolation");
    lied["outcome"]["Answered"]["facts"]["keys"] = json!([]);
    assert!(callback(&mut t, 33, &pv, lied));
    let e = event("t2_contradicts").expect("t2_contradicts");
    assert_eq!(e["data"][0]["differ"], json!(["receipt", "kind", "slots"]));
    assert_eq!(e["data"][0]["kind"], "Violation");
    assert_eq!(e["data"][0]["tee_kind"], "NoViolation");
    assert_eq!(t.get_stats()["contradicted"], 1);
    // Another request hash: not the TEE's input. Rejected, nothing stored, the deposit refunded.
    let pv = public_values("c");
    let mut other = chain("run-35");
    other["request_sha256"] = json!("00".repeat(32));
    assert!(!callback(&mut t, 35, &pv, other));
    assert!(event("t2_rejected").is_some());
    assert!(t.get_record(U64(35)).is_none());
    // near-antibody's views failed: rejected too.
    ctx("zk.antibody.testnet", NearToken::from_yoctonear(0));
    assert!(!t.on_records(
        Base64VecU8(pv),
        "00".repeat(32),
        "anyone.testnet".parse().unwrap(),
        U128(RECORD_DEPOSIT.as_yoctonear()),
        Err(near_sdk::PromiseError::Failed),
        Ok(None),
        Ok(None),
        Ok(None),
        Ok(None),
    ));
}

#[test]
fn the_deployed_contract_pins_sp1_6_10_and_the_docker_builds_vkey() {
    let t = contract(None, PROGRAM_VKEY);
    let p = t.get_pins();
    assert_eq!(p["groth16"]["is_sp1_v6_10_0"], true);
    assert_eq!(p["groth16"]["selector"], "0x4388a21c");
    assert_eq!(p["program_vkey"], PROGRAM_VKEY);
}

/// The real proofs from SP1's prover network (C1): each verifies against SP1 6.10.0's key and the docker
/// build's vkey, and fails with any public value changed.
#[test]
fn the_real_sp1_proofs_verify_against_sp1_6_10s_key() {
    let mut found = 0;
    for (case, run, _, _) in CASES {
        let Ok(proof) = std::fs::read(root(&format!("../../fixtures/zk/{case}/proof.bin"))) else { continue };
        found += 1;
        let pv = public_values(case);
        let mut t = contract(None, PROGRAM_VKEY);
        ctx("anyone.testnet", NearToken::from_yoctonear(0));
        let before = env::used_gas();
        assert!(t.verify_proof(Base64VecU8(pv.clone()), Base64VecU8(proof.clone())), "{case}");
        println!("{case}: host gas {:.2} Tgas", (env::used_gas().as_gas() - before.as_gas()) as f64 / 1e12);
        let mut changed = pv.clone();
        changed[1] ^= 1;
        assert!(!t.verify_proof(Base64VecU8(changed), Base64VecU8(proof.clone())), "{case}: changed public values");
        submits(&mut t, run, &pv, &proof).unwrap_or_else(|e| panic!("{case}: {e}"));
        assert!(callback(&mut t, run, &pv, chain(&format!("run-{run}"))), "{case}");
        assert_eq!(t.get_record(U64(run)).unwrap()["outcome"], "confirmed", "{case}");
    }
    if found == 0 {
        println!("no fixtures/zk/*/proof.bin yet (C1)");
    }
}

// near-sdk declares the yield/resume host functions; Windows links the test binary against these
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

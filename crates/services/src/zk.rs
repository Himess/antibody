//! T2 (docs/zk.md): validity proofs of disclosed verdicts, checked by near-t2.
//!
//! - [`inputs`]: a disclosed run's inputs for the zk guest (the request near-antibody sent, the base, the
//!   round's manifest, the blob), each checked against the hash the request names.
//! - [`mock_proof`]: the sandbox's MOCK prover. It makes a proof under the TEST key
//!   (fixtures/zk/t2-test-key.json), which anyone can make proofs for: the sandbox has no SP1 prover, and
//!   this lets near-t2's comparisons run end to end on real public values.
//! - [`sandbox`]: the sandbox demo's T2 step: an honest practice try is confirmed, and a try the MOCK
//!   worker lied about (another verdict for the same input) is contradicted.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use antibody_cli::{cmd, ctx::Ctx};
use antibody_verifier::facts::sha256;
use antibody_zk_core::{rerun, Disclosed};
use base64::Engine as _;
use bn::{AffineG1, Fq, Fr, G1};
use eyre::{eyre, Result};
use serde_json::{json, Value};

/// The guest's program vkey (SP1's docker build of crates/zk/guest): near-t2 pins it on testnet.
pub const PROGRAM_VKEY: &str = "0x00e87aaad0ca36c285eec6bccab0cace523784873ad65e45b3b65e2759288203";

/// A disclosed run's guest inputs besides the request and the content key.
pub struct Inputs {
    pub base: Vec<u8>,
    pub round_manifest: Option<Vec<u8>>,
    pub blob: Option<Vec<u8>>,
}

fn fetched(fetch: &dyn Fn(&str) -> Result<Vec<u8>>, r: &Value, what: &str) -> Result<Vec<u8>> {
    let loc = r["near_tx"].as_str().ok_or_else(|| eyre!("{what}: no location"))?;
    let want = r["sha256"].as_str().unwrap_or_default();
    let b = fetch(loc)?;
    if hex::encode(sha256(&b)) != want {
        return Err(eyre!("{what}: {loc} does not hash to {want}"));
    }
    Ok(b)
}

/// The base witness, the round's manifest and the blob a request names (an inline blob stays in it).
pub fn inputs(request: &[u8], fetch: &dyn Fn(&str) -> Result<Vec<u8>>) -> Result<Inputs> {
    let r: Value = serde_json::from_slice(request)?;
    Ok(Inputs {
        base: fetched(fetch, &r["base"], "the base witness")?,
        round_manifest: if r["round_manifest"].is_object() { Some(fetched(fetch, &r["round_manifest"], "the round's manifest")?) } else { None },
        blob: if r["blob"]["inline"].is_string() { None } else { Some(fetched(fetch, &r["blob"], "the blob")?) },
    })
}

fn rev(b: &[u8]) -> Vec<u8> {
    b.iter().rev().copied().collect()
}

fn fq(le: &[u8]) -> Result<Fq> {
    Fq::from_slice(&rev(le)).map_err(|e| eyre!("{e:?}"))
}

/// A G1 point in NEAR's encoding (little-endian x, y).
fn g1(h: &str) -> Result<G1> {
    let b = hex::decode(h)?;
    if b.len() != 64 {
        return Err(eyre!("a G1 point is 64 bytes"));
    }
    Ok(G1::from(AffineG1::new(fq(&b[..32])?, fq(&b[32..])?).map_err(|e| eyre!("{e:?}"))?))
}

fn fq_be(x: Fq) -> Result<Vec<u8>> {
    let mut o = [0u8; 32];
    x.to_big_endian(&mut o).map_err(|e| eyre!("{e:?}"))?;
    Ok(o.to_vec())
}

fn g1_be(p: G1) -> Result<Vec<u8>> {
    let a = AffineG1::from_jacobian(p).ok_or_else(|| eyre!("the point at infinity"))?;
    Ok([fq_be(a.x())?, fq_be(a.y())?].concat())
}

/// SP1's public input for the public values: sha256 with the top 3 bits cleared.
pub fn public_values_digest(pv: &[u8]) -> [u8; 32] {
    let mut d = sha256(pv);
    d[0] &= 0x1f;
    d
}

/// A proof, in SP1 6's 356-byte Groth16 layout, under the TEST key: B = G2 (the key's β = γ = δ), C = IC₀,
/// A = α + L + C with L = IC₀ + Σ inputᵢ·ICᵢ₊₁. It verifies only against that key.
pub fn mock_proof(key: &Value, program_vkey: &[u8; 32], pv: &[u8]) -> Result<Vec<u8>> {
    let h = |k: &str| key[k].as_str().ok_or_else(|| eyre!("the TEST key has no {k}"));
    let ic: Vec<G1> = key["ic"].as_array().ok_or_else(|| eyre!("ic"))?.iter().map(|p| g1(p.as_str().unwrap_or_default())).collect::<Result<_>>()?;
    if ic.len() != 6 {
        return Err(eyre!("the TEST key has 6 IC points"));
    }
    let vk_root = hex::decode(h("vk_root")?)?;
    let digest = public_values_digest(pv);
    let zero = [0u8; 32];
    let inputs: [&[u8]; 5] = [program_vkey, &digest, &zero, &vk_root, &zero];
    let mut l = ic[0];
    for (i, x) in inputs.iter().enumerate() {
        l = l + ic[i + 1] * Fr::from_slice(x).map_err(|e| eyre!("{e:?}"))?;
    }
    let c = ic[0];
    let a = g1(h("alpha")?)? + l + c;
    // The G2 generator, from the key's −G2 (NEAR's encoding: x.c0, x.c1, (−y).c0, (−y).c1).
    let n = hex::decode(h("beta_neg")?)?;
    let b = [rev(&n[32..64]), rev(&n[..32]), fq_be(-fq(&n[96..128])?)?, fq_be(-fq(&n[64..96])?)?].concat();
    let mut proof = hex::decode(h("selector")?)?;
    proof.extend(zero);
    proof.extend(&vk_root);
    proof.extend(zero);
    proof.extend(g1_be(a)?);
    proof.extend(b);
    proof.extend(g1_be(c)?);
    Ok(proof)
}

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// One sandbox run to prove: its id and the hunter's local record (the content key).
pub struct Case<'a> {
    pub label: &'a str,
    pub run_id: u64,
    pub record: &'a Path,
}

/// The sandbox demo's T2 step. `zk` deploys near-t2 on its own account under the TEST key; `submitter`
/// submits; `inputs` holds the requests the MOCK worker received; `fetch` reads FastFS locations.
#[allow(clippy::too_many_arguments)]
pub fn sandbox(
    zk: &Ctx,
    submitter: &Ctx,
    wasm: &Path,
    test_key: &Path,
    verifier_wasm: &str,
    cases: &[Case],
    requests: &Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
    fetch: &dyn Fn(&str) -> Result<Vec<u8>>,
) -> Result<Value> {
    let key: Value = serde_json::from_slice(&std::fs::read(test_key)?)?;
    let vkey: [u8; 32] = hex::decode(PROGRAM_VKEY.trim_start_matches("0x"))?.try_into().map_err(|_| eyre!("vkey"))?;
    // Each run, re-run natively on the exact request near-antibody sent.
    let mut proven = vec![];
    for c in cases {
        let request = requests.lock().map_err(|_| eyre!("requests lock"))?.get(&c.run_id).cloned().ok_or_else(|| eyre!("{}: the MOCK worker saw no request for run {}", c.label, c.run_id))?;
        let rec: Value = serde_json::from_slice(&std::fs::read(c.record)?)?;
        let key_bytes: [u8; 32] = hex::decode(rec["content_key"].as_str().unwrap_or_default())?.try_into().map_err(|_| eyre!("content key"))?;
        let i = inputs(&request, fetch)?;
        let r = rerun(&Disclosed { request: &request, base: &i.base, round_manifest: i.round_manifest.as_deref(), blob: i.blob.as_deref(), content_key: key_bytes })
            .map_err(|e| eyre!("{}: {e}", c.label))?;
        proven.push((c, r));
    }
    let verifier = proven.first().map(|(_, r)| r.verifier()).ok_or_else(|| eyre!("no case"))?;
    let account = zk.account.clone().ok_or_else(|| eyre!("no account for near-t2"))?;
    let init = json!({"antibody": zk.dep.antibody, "program_vkey": PROGRAM_VKEY, "verifier": verifier, "verifier_wasm": verifier_wasm,
                      "groth16": {"selector": key["selector"], "vk_root": key["vk_root"], "alpha": key["alpha"], "beta_neg": key["beta_neg"],
                                  "gamma_neg": key["gamma_neg"], "delta_neg": key["delta_neg"], "ic": key["ic"]}});
    let deployed = cmd::admin_deploy(zk, wasm, Some("new".into()), Some(init.to_string()))?;
    let signer = submitter.signer()?;
    let submit = |run_id: u64, pv: &[u8], proof: &[u8]| -> Result<Value> {
        let args = json!({"run_id": run_id.to_string(), "public_values": b64(pv), "proof": b64(proof)});
        let tx = submitter.near.function_call(&signer, &account, "submit", args, 200, 20 * 10u128.pow(21))?;
        cmd::ensure_ok(submitter, &tx, "submit")
    };
    let mut out = vec![];
    let mut tampered_refused = None;
    for (c, r) in &proven {
        let pv = r.public_values();
        let proof = mock_proof(&key, &vkey, &pv)?;
        if tampered_refused.is_none() {
            // A tampered proof is refused: the transaction fails and nothing is recorded.
            let mut bad = proof.clone();
            bad[100 + 63] ^= 1;
            tampered_refused = Some(submit(c.run_id, &pv, &bad).is_err());
        }
        let s = submit(c.run_id, &pv, &proof)?;
        // Views read a final block, a moment behind the transaction just executed.
        let t0 = std::time::Instant::now();
        let record = loop {
            let r = submitter.near.view(&account, "get_record", json!({"run_id": c.run_id.to_string()}))?;
            if !r.is_null() || t0.elapsed().as_secs() > 30 {
                break r;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        };
        out.push(json!({"case": c.label, "run_id": c.run_id, "tx": s["tx"], "gas_burnt": s["gas_burnt"], "record": record,
                        "proven": {"kind": format!("{:?}", r.receipt.kind), "receipt_sha256": hex::encode(r.receipt_sha256)}}));
    }
    let stats = submitter.near.view(&account, "get_stats", json!({}))?;
    let pins = submitter.near.view(&account, "get_pins", json!({}))?;
    Ok(json!({"contract": account, "deploy": deployed, "pins": pins, "submissions": out, "tampered_refused": tampered_refused, "stats": stats}))
}

#[cfg(test)]
mod tests {
    use bn::{pairing_batch, AffineG2, Fq2, Gt, G2};

    use super::*;

    fn g2_near(h: &str) -> G2 {
        let b = hex::decode(h).unwrap();
        let f = |i: usize| fq(&b[32 * i..32 * i + 32]).unwrap();
        G2::from(AffineG2::new(Fq2::new(f(0), f(1)), Fq2::new(f(2), f(3))).unwrap())
    }

    fn g1_be_point(b: &[u8]) -> G1 {
        G1::from(AffineG1::new(Fq::from_slice(&b[..32]).unwrap(), Fq::from_slice(&b[32..64]).unwrap()).unwrap())
    }

    /// near-t2's check, natively: e(A, B) · e(C, −δ) · e(α, −β) · e(L, −γ) = 1.
    fn verifies(key: &Value, vkey: &[u8; 32], pv: &[u8], proof: &[u8]) -> bool {
        let digest = public_values_digest(pv);
        let ic: Vec<G1> = key["ic"].as_array().unwrap().iter().map(|p| g1(p.as_str().unwrap()).unwrap()).collect();
        let inputs: [&[u8]; 5] = [vkey, &digest, &proof[4..36], &proof[36..68], &proof[68..100]];
        let mut l = ic[0];
        for (i, x) in inputs.iter().enumerate() {
            l = l + ic[i + 1] * Fr::from_slice(x).unwrap();
        }
        let g = &proof[100..];
        let fbe = |b: &[u8]| Fq::from_slice(b).unwrap();
        let b = G2::from(AffineG2::new(Fq2::new(fbe(&g[96..128]), fbe(&g[64..96])), Fq2::new(fbe(&g[160..192]), fbe(&g[128..160]))).unwrap());
        let s = |k: &str| key[k].as_str().unwrap().to_string();
        pairing_batch(&[
            (g1_be_point(&g[..64]), b),
            (g1_be_point(&g[192..256]), g2_near(&s("delta_neg"))),
            (g1(&s("alpha")).unwrap(), g2_near(&s("beta_neg"))),
            (l, g2_near(&s("gamma_neg"))),
        ]) == Gt::one()
    }

    #[test]
    fn a_mock_proof_verifies_under_the_test_key_only_for_its_public_values() {
        let key: Value = serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/zk/t2-test-key.json")).unwrap()).unwrap();
        let pv = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/zk/a/public_values.bin")).unwrap();
        let vkey: [u8; 32] = hex::decode(PROGRAM_VKEY.trim_start_matches("0x")).unwrap().try_into().unwrap();
        let proof = mock_proof(&key, &vkey, &pv).unwrap();
        assert_eq!(proof.len(), 356);
        assert!(verifies(&key, &vkey, &pv, &proof));
        let mut other = pv.clone();
        other[1] ^= 1;
        assert!(!verifies(&key, &vkey, &other, &proof), "other public values");
        let mut bad = proof.clone();
        bad[100 + 63] ^= 1;
        assert!(std::panic::catch_unwind(|| verifies(&key, &vkey, &pv, &bad)).map(|ok| !ok).unwrap_or(true), "a tampered proof");
    }
}

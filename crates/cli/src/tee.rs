//! T1: OutLayer's per-execution TDX quotes, checked against NEAR's DCAP verifier (docs/trust.md).
//!
//! OutLayer's worker signs one TDX quote per execution. The first half of the quote's report_data is
//! SHA-256 over the task fields (OutLayer `worker/src/tdx_attestation.rs`, layout V1), including the
//! wasm hash and the hashes of the exact input and output. `check` re-derives that binding and has
//! the DCAP verifier on NEAR (`verify_quote`, called as a free view) check the quote. `confirm`
//! relays it to near-antibody, which repeats every check on chain and marks the verdict T1.

use base64::Engine;
use eyre::{eyre, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{ctx::Ctx, near::Near};

pub const DEFAULT_API: &str = "https://testnet-api.outlayer.ai";
pub const DEFAULT_VERIFIER: &str = "tee-verifier-2026-07-22.testnet";
pub const OUTLAYER_WORKER: &str = "worker.outlayer.testnet";

/// Offsets inside a TDX v4 quote: 48-byte header, then the TD 1.0 report body.
const BODY: usize = 48;

pub fn fetch_attestation(api: &str, request_id: u64) -> Result<Value> {
    let r = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .get(format!("{api}/attestations/by-request/{request_id}"))
        .send()?;
    if !r.status().is_success() {
        return Err(eyre!("no OutLayer attestation for request {request_id} (HTTP {})", r.status()));
    }
    Ok(r.json()?)
}

/// SHA-256 over the attestation's task fields in OutLayer's order (report data layout V1).
pub fn task_hash(a: &Value) -> [u8; 32] {
    let s = |k: &str| a[k].as_str().map(|x| x.as_bytes().to_vec());
    let mut h = Sha256::new();
    h.update(a["task_type"].as_str().unwrap_or_default().as_bytes());
    h.update(a["task_id"].as_i64().unwrap_or_default().to_le_bytes());
    for k in ["repo", "commit", "build_target", "wasm_hash", "input_hash"] {
        if let Some(b) = s(k) {
            h.update(b);
        }
    }
    h.update(a["output_hash"].as_str().unwrap_or_default().as_bytes());
    if let Some(b) = a["block_height"].as_u64() {
        h.update(b.to_le_bytes());
    }
    for k in ["caller_account_id", "project_id", "secrets_ref"] {
        if let Some(b) = s(k) {
            h.update(b);
        }
    }
    h.update(a["timestamp"].as_i64().unwrap_or_default().to_le_bytes());
    if let Some(b) = s("attached_usd") {
        h.update(b);
    }
    h.finalize().into()
}

pub struct QuoteView {
    pub report_data: [u8; 64],
    pub mrtd: String,
    pub rtmr: [String; 4],
}

pub fn parse_quote(q: &[u8]) -> Result<QuoteView> {
    if q.len() < BODY + 584 || u16::from_le_bytes([q[0], q[1]]) != 4 || u32::from_le_bytes([q[4], q[5], q[6], q[7]]) != 0x81 {
        return Err(eyre!("not a TDX v4 quote"));
    }
    let b = &q[BODY..BODY + 584];
    let mut report_data = [0u8; 64];
    report_data.copy_from_slice(&b[520..584]);
    let rt = |i: usize| hex::encode(&b[328 + 48 * i..376 + 48 * i]);
    Ok(QuoteView { report_data, mrtd: hex::encode(&b[136..184]), rtmr: [rt(0), rt(1), rt(2), rt(3)] })
}

fn borsh_vec(out: &mut Vec<u8>, b: &[u8]) {
    out.extend((b.len() as u32).to_le_bytes());
    out.extend(b);
}

/// dcap-qvl JSON collateral (byte fields hex) → the borsh `Collateral` `verify_quote` takes.
pub fn collateral_borsh(c: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let st = |k: &str| c[k].as_str().ok_or_else(|| eyre!("collateral: no {k}"));
    let hx = |k: &str| -> Result<Vec<u8>> { Ok(hex::decode(st(k)?)?) };
    borsh_vec(&mut out, st("pck_crl_issuer_chain")?.as_bytes());
    borsh_vec(&mut out, &hx("root_ca_crl")?);
    borsh_vec(&mut out, &hx("pck_crl")?);
    borsh_vec(&mut out, st("tcb_info_issuer_chain")?.as_bytes());
    borsh_vec(&mut out, st("tcb_info")?.as_bytes());
    borsh_vec(&mut out, &hx("tcb_info_signature")?);
    borsh_vec(&mut out, st("qe_identity_issuer_chain")?.as_bytes());
    borsh_vec(&mut out, st("qe_identity")?.as_bytes());
    borsh_vec(&mut out, &hx("qe_identity_signature")?);
    match c["pck_certificate_chain"].as_str() {
        Some(p) => {
            out.push(1);
            borsh_vec(&mut out, p.as_bytes());
        }
        None => out.push(0),
    }
    Ok(out)
}

/// Runs `verify_quote` as a view (no transaction, no fee). Returns (verified, TCB status or error).
pub fn verify_view(near: &Near, verifier: &str, quote: &[u8], collateral: &[u8]) -> Result<(bool, String)> {
    let mut args = Vec::new();
    borsh_vec(&mut args, quote);
    args.extend(collateral);
    let r = near.view_raw(verifier, "verify_quote", &args)?;
    let read_str = |b: &[u8]| -> String {
        if b.len() < 4 {
            return String::new();
        }
        let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        String::from_utf8_lossy(&b[4..(4 + n).min(b.len())]).into_owned()
    };
    match r.first() {
        Some(0) => Ok((true, read_str(&r[1..]))),
        Some(1) => Ok((false, read_str(&r[2..]))),
        _ => Err(eyre!("unexpected verify_quote result")),
    }
}

/// OutLayer's on-chain collaterals, newest first, each as borsh.
pub fn outlayer_collaterals(near: &Near) -> Result<Vec<Vec<u8>>> {
    let v = near.view(OUTLAYER_WORKER, "get_collaterals", json!({}))?;
    let mut out = Vec::new();
    for c in v.as_array().into_iter().flatten().rev() {
        let c: Value = match c.as_str() {
            Some(s) => serde_json::from_str(s)?,
            None => c.clone(),
        };
        out.push(collateral_borsh(&c)?);
    }
    Ok(out)
}

/// The OutLayer request id of a near-antibody verification, from the transaction that requested it.
pub fn request_id_from_tx(ctx: &Ctx, tx: &str, signer: &str) -> Result<u64> {
    let out = ctx.near.call("EXPERIMENTAL_tx_status", json!({"tx_hash": tx, "sender_account_id": signer, "wait_until": "FINAL"}))?;
    for r in out["receipts_outcome"].as_array().into_iter().flatten() {
        for l in r["outcome"]["logs"].as_array().into_iter().flatten() {
            let Some(j) = l.as_str().and_then(|s| s.strip_prefix("EVENT_JSON:")) else { continue };
            let Ok(ev) = serde_json::from_str::<Value>(j) else { continue };
            if ev["event"] != "execution_requested" {
                continue;
            }
            let rd = &ev["data"][0]["request_data"];
            let rd: Value = match rd.as_str() {
                Some(s) => serde_json::from_str(s)?,
                None => rd.clone(),
            };
            if let Some(id) = rd["request_id"].as_u64() {
                return Ok(id);
            }
        }
    }
    Err(eyre!("no OutLayer execution_requested event in {tx}"))
}

/// Independent T1 check of any OutLayer run: the quote binds the attested fields, and NEAR's DCAP
/// verifier accepts the quote. No keys, no transactions.
pub fn check(ctx: &Ctx, api: &str, verifier: &str, request_id: u64) -> Result<Value> {
    let a = fetch_attestation(api, request_id)?;
    let quote = base64::engine::general_purpose::STANDARD.decode(a["tdx_quote"].as_str().unwrap_or_default())?;
    let qv = parse_quote(&quote)?;
    let th = task_hash(&a);
    let binds = qv.report_data[..32] == th && qv.report_data[32..] == [0u8; 32];
    let mut verdict = json!(null);
    for (i, c) in outlayer_collaterals(&ctx.near)?.iter().enumerate() {
        let (ok, status) = verify_view(&ctx.near, verifier, &quote, c)?;
        verdict = json!({"verified": ok, "status": status, "collateral_index_newest_first": i});
        if ok {
            break;
        }
    }
    let approved = ctx.near.view(OUTLAYER_WORKER, "get_approved_measurements", json!({}))?;
    let pinned = approved.as_array().into_iter().flatten().position(|m| {
        m["mrtd"] == json!(qv.mrtd) && (0..4).all(|i| m[format!("rtmr{i}")] == json!(qv.rtmr[i]))
    });
    Ok(json!({
        "request_id": request_id, "task_id": a["task_id"], "caller": a["caller_account_id"], "wasm_hash": a["wasm_hash"],
        "input_hash": a["input_hash"], "output_hash": a["output_hash"],
        "report_data_binds_task": binds, "task_hash": hex::encode(th),
        "dcap": verdict, "verifier": verifier,
        "measurements": {"mrtd": qv.mrtd, "rtmr3": qv.rtmr[3], "outlayer_approved_index": pinned},
        "tier": if binds && verdict["verified"] == json!(true) && pinned.is_some() { "T1-eligible" } else { "not T1" },
    }))
}

/// Relays the quote of a commit's verification (or its round-`round` re-run) to near-antibody.
pub fn confirm(ctx: &Ctx, api: &str, commit_id: u64, round: Option<u32>, request_id: u64) -> Result<Value> {
    let signer = ctx.signer()?;
    let cfg = ctx.view("get_tee_config", json!({}))?;
    let verifier = cfg["verifier"].as_str().ok_or_else(|| eyre!("near-antibody has no DCAP verifier configured"))?.to_string();
    let a = fetch_attestation(api, request_id)?;
    let quote = base64::engine::general_purpose::STANDARD.decode(a["tdx_quote"].as_str().unwrap_or_default())?;
    let mut chosen = None;
    for c in outlayer_collaterals(&ctx.near)? {
        if verify_view(&ctx.near, &verifier, &quote, &c)?.0 {
            chosen = Some(c);
            break;
        }
    }
    let collateral = chosen.ok_or_else(|| eyre!("no stored collateral verifies this quote (collateral may be stale)"))?;
    let att = json!({
        "task_type": a["task_type"], "task_id": a["task_id"], "repo": a["repo"], "commit": a["commit"],
        "build_target": a["build_target"], "wasm_hash": a["wasm_hash"], "input_hash": a["input_hash"],
        "output_hash": a["output_hash"], "block_height": a["block_height"], "caller_account_id": a["caller_account_id"],
        "project_id": a["project_id"], "secrets_ref": a["secrets_ref"], "timestamp": a["timestamp"], "attached_usd": a["attached_usd"],
    });
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let out = ctx.near.function_call(
        &signer,
        &ctx.dep.antibody,
        "confirm_tee",
        json!({"commit_id": commit_id, "round": round, "attestation": att, "quote": b64(&quote), "collateral": b64(&collateral)}),
        300,
        0,
    )?;
    let failures = crate::near::failures(&out);
    if !failures.is_empty() {
        return Err(eyre!("confirm_tee failed: {}", failures.join("; ")));
    }
    let events = crate::near::events(&out);
    let confirmed = events.iter().any(|e| e["event"] == "tee_confirmed");
    Ok(json!({"commit_id": commit_id, "round": round, "request_id": request_id, "confirmed": confirmed,
              "events": events, "tx": ctx.near.explorer_tx(out["transaction"]["hash"].as_str().unwrap_or_default())}))
}

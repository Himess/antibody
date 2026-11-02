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
/// Intel's Provisioning Certification Service: TCB info, QE identity and CRLs for any FMSPC, signed by
/// Intel (no API key needed for these endpoints). Fresh collateral keeps T1 working after the copies
/// OutLayer stores on chain expire.
pub const INTEL_PCS: &str = "https://api.trustedservices.intel.com";
pub const INTEL_ROOT_CA_CRL: &str = "https://certificates.trustedservices.intel.com/IntelSGXRootCA.der";

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

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The exact text of the JSON object under `key`, as Intel signed it (re-serialising would break the
/// signature).
pub fn raw_json_object<'a>(body: &'a str, key: &str) -> Result<&'a str> {
    let at = body.find(&format!("\"{key}\"")).ok_or_else(|| eyre!("no {key} in the PCS response"))?;
    let start = at + body[at..].find('{').ok_or_else(|| eyre!("{key} is not an object"))?;
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for (k, c) in body[start..].char_indices() {
        if in_str {
            match c {
                _ if esc => esc = false,
                '\\' => esc = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(&body[start..start + k + 1]);
                }
            }
            _ => {}
        }
    }
    Err(eyre!("unterminated {key}"))
}

/// The PCK leaf certificate (DER) embedded in a TDX v4 quote's certification data.
fn quote_pck_cert(q: &[u8]) -> Result<Vec<u8>> {
    let text = String::from_utf8_lossy(q);
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let a = text.find(BEGIN).ok_or_else(|| eyre!("no PCK certificate chain in the quote"))? + BEGIN.len();
    let b = a + text[a..].find(END).ok_or_else(|| eyre!("unterminated PCK certificate"))?;
    let b64: String = text[a..b].chars().filter(|c| !c.is_whitespace()).collect();
    Ok(base64::engine::general_purpose::STANDARD.decode(b64)?)
}

/// FMSPC (hex, upper case) and PCK CA ("platform" or "processor") of the platform that signed a quote:
/// the SGX extension OID 1.2.840.113741.1.13.1.4 of the PCK certificate, then an OCTET STRING of 6
/// bytes; the issuer names the CA.
pub fn quote_platform(q: &[u8]) -> Result<(String, &'static str)> {
    let der = quote_pck_cert(q)?;
    const FMSPC_OID: [u8; 12] = [0x06, 0x0a, 0x2a, 0x86, 0x48, 0x86, 0xf8, 0x4d, 0x01, 0x0d, 0x01, 0x04];
    let at = der.windows(FMSPC_OID.len()).position(|w| w == FMSPC_OID).ok_or_else(|| eyre!("no FMSPC in the PCK certificate"))?;
    let v = &der[at + FMSPC_OID.len()..];
    if v.len() < 8 || v[0] != 0x04 || v[1] != 0x06 {
        return Err(eyre!("unexpected FMSPC encoding"));
    }
    let ca = if der.windows(12).any(|w| w == b"Processor CA") { "processor" } else { "platform" };
    Ok((hex::encode_upper(&v[2..8]), ca))
}

/// Fresh DCAP collateral for `quote` from Intel PCS, in the JSON shape `collateral_borsh` reads.
pub fn pcs_collateral_json(quote: &[u8], pcs: &str) -> Result<Value> {
    let (fmspc, ca) = quote_platform(quote)?;
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
    let get = |url: String| -> Result<reqwest::blocking::Response> {
        let r = client.get(&url).send()?;
        if !r.status().is_success() {
            return Err(eyre!("Intel PCS {url}: HTTP {}", r.status()));
        }
        Ok(r)
    };
    let issuer = |r: &reqwest::blocking::Response, h: &str| -> Result<String> {
        Ok(percent_decode(r.headers().get(h).ok_or_else(|| eyre!("PCS response lacks {h}"))?.to_str()?))
    };
    let r = get(format!("{pcs}/tdx/certification/v4/tcb?fmspc={fmspc}"))?;
    let tcb_chain = issuer(&r, "TCB-Info-Issuer-Chain")?;
    let tcb_body = r.text()?;
    let tcb_sig = serde_json::from_str::<Value>(&tcb_body)?["signature"].as_str().unwrap_or_default().to_string();
    let r = get(format!("{pcs}/tdx/certification/v4/qe/identity"))?;
    let qe_chain = issuer(&r, "SGX-Enclave-Identity-Issuer-Chain")?;
    let qe_body = r.text()?;
    let qe_sig = serde_json::from_str::<Value>(&qe_body)?["signature"].as_str().unwrap_or_default().to_string();
    let r = get(format!("{pcs}/sgx/certification/v4/pckcrl?ca={ca}&encoding=der"))?;
    let crl_chain = issuer(&r, "SGX-PCK-CRL-Issuer-Chain")?;
    let pck_crl = r.bytes()?;
    let root_crl = get(INTEL_ROOT_CA_CRL.to_string())?.bytes()?;
    Ok(json!({
        "pck_crl_issuer_chain": crl_chain, "root_ca_crl": hex::encode(root_crl), "pck_crl": hex::encode(pck_crl),
        "tcb_info_issuer_chain": tcb_chain, "tcb_info": raw_json_object(&tcb_body, "tcbInfo")?, "tcb_info_signature": tcb_sig,
        "qe_identity_issuer_chain": qe_chain, "qe_identity": raw_json_object(&qe_body, "enclaveIdentity")?,
        "qe_identity_signature": qe_sig, "fmspc": fmspc,
    }))
}

/// Picks the first collateral that NEAR's DCAP verifier accepts for `quote`: OutLayer's on-chain
/// copies (newest first) and/or fresh Intel PCS collateral, per `source` ("auto", "outlayer", "pcs").
/// Returns (borsh collateral, where it came from, TCB status).
pub fn choose_collateral(near: &Near, verifier: &str, quote: &[u8], source: &str) -> Result<(Vec<u8>, String, String)> {
    let mut tried = Vec::new();
    if source == "auto" || source == "outlayer" {
        for (i, c) in outlayer_collaterals(near)?.into_iter().enumerate() {
            let (ok, status) = verify_view(near, verifier, quote, &c)?;
            if ok {
                return Ok((c, format!("outlayer-onchain[{i}]"), status));
            }
            tried.push(format!("outlayer-onchain[{i}]: {status}"));
        }
    }
    if source == "auto" || source == "pcs" {
        let c = collateral_borsh(&pcs_collateral_json(quote, INTEL_PCS)?)?;
        let (ok, status) = verify_view(near, verifier, quote, &c)?;
        if ok {
            return Ok((c, "intel-pcs".into(), status));
        }
        tried.push(format!("intel-pcs: {status}"));
    }
    Err(eyre!("no collateral verifies this quote ({})", tried.join("; ")))
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
pub fn check(ctx: &Ctx, api: &str, verifier: &str, request_id: u64, source: &str) -> Result<Value> {
    let a = fetch_attestation(api, request_id)?;
    let quote = base64::engine::general_purpose::STANDARD.decode(a["tdx_quote"].as_str().unwrap_or_default())?;
    let qv = parse_quote(&quote)?;
    let th = task_hash(&a);
    let binds = qv.report_data[..32] == th && qv.report_data[32..] == [0u8; 32];
    let verdict = match choose_collateral(&ctx.near, verifier, &quote, source) {
        Ok((_, from, status)) => json!({"verified": true, "status": status, "collateral": from}),
        Err(e) => json!({"verified": false, "error": e.to_string()}),
    };
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

/// The attestation fields near-antibody takes (`confirm_tee`), from OutLayer's attestation record.
pub fn attestation_args(a: &Value) -> Value {
    json!({
        "task_type": a["task_type"], "task_id": a["task_id"], "repo": a["repo"], "commit": a["commit"],
        "build_target": a["build_target"], "wasm_hash": a["wasm_hash"], "input_hash": a["input_hash"],
        "output_hash": a["output_hash"], "block_height": a["block_height"], "caller_account_id": a["caller_account_id"],
        "project_id": a["project_id"], "secrets_ref": a["secrets_ref"], "timestamp": a["timestamp"], "attached_usd": a["attached_usd"],
    })
}

/// Calls `confirm_tee` for run `run_id`: near-antibody checks that the attestation describes the run
/// it requested, has the program's pinned DCAP verifier check the quote, and accepts only the
/// program's worker images.
fn relay(ctx: &Ctx, run_id: u64, attestation: Value, quote: &[u8], collateral: &[u8]) -> Result<Value> {
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let out = ctx.near.function_call(
        &ctx.signer()?,
        &ctx.dep.antibody,
        "confirm_tee",
        json!({"run_id": run_id, "attestation": attestation, "quote": b64(quote), "collateral": b64(collateral)}),
        300,
        0,
    )?;
    let failures = crate::near::failures(&out);
    if !failures.is_empty() {
        return Err(eyre!("confirm_tee failed: {}", failures.join("; ")));
    }
    let events = crate::near::events(&out);
    let confirmed = events.iter().any(|e| e["event"] == "t1_confirmed");
    let rejected = events.iter().find(|e| e["event"] == "tee_rejected").map(|e| e["data"][0]["reason"].clone());
    Ok(json!({"run_id": run_id, "confirmed": confirmed, "rejected": rejected, "events": events,
              "tx": ctx.near.explorer_tx(out["transaction"]["hash"].as_str().unwrap_or_default())}))
}

/// Relays OutLayer's TDX quote for run `run_id` (OutLayer request `request_id`) to near-antibody.
pub fn confirm(ctx: &Ctx, api: &str, run_id: u64, request_id: u64, source: &str) -> Result<Value> {
    let r = ctx.view("get_run", json!({"run_id": run_id}))?;
    let program = r["program"].as_u64().ok_or_else(|| eyre!("run {run_id} does not exist"))?;
    let pins = ctx.view("get_pins", json!({"program_id": program}))?;
    let verifier = pins["dcap_verifier"].as_str().ok_or_else(|| eyre!("program {program} has no pinned DCAP verifier"))?.to_string();
    let a = fetch_attestation(api, request_id)?;
    let quote = base64::engine::general_purpose::STANDARD.decode(a["tdx_quote"].as_str().unwrap_or_default())?;
    let (collateral, collateral_from, _) = choose_collateral(&ctx.near, &verifier, &quote, source)?;
    let mut v = relay(ctx, run_id, attestation_args(&a), &quote, &collateral)?;
    v["request_id"] = request_id.into();
    v["collateral"] = collateral_from.into();
    Ok(v)
}

/// Sandbox only: relays the MOCK attestation that `mock-outlayer` recorded for the run (its MOCK quote
/// verifies only against `mock-dcap`). near-antibody runs the same checks as on testnet.
pub fn confirm_mock(ctx: &Ctx, mock_outlayer: &str, run_id: u64) -> Result<Value> {
    let r = ctx.view("get_run", json!({"run_id": run_id}))?;
    let input_hash = r["request_sha256"].as_str().ok_or_else(|| eyre!("run {run_id} does not exist"))?;
    let at = ctx.near.view(mock_outlayer, "find_attestation", json!({"input_hash": input_hash}))?;
    if at.is_null() {
        return Err(eyre!("no MOCK attestation for run {run_id} yet"));
    }
    let a: Value = serde_json::from_str(at["attestation"].as_str().unwrap_or("{}"))?;
    let quote = base64::engine::general_purpose::STANDARD.decode(at["quote"].as_str().unwrap_or_default())?;
    let mut v = relay(ctx, run_id, attestation_args(&a), &quote, &[])?;
    v["request_id"] = at["request_id"].clone();
    v["mock"] = true.into();
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorded_quote() -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/t1/outlayer-attestation-request-6496.json");
        let a: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        base64::engine::general_purpose::STANDARD.decode(a["tdx_quote"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn platform_of_a_recorded_quote() {
        // The FMSPC Intel's TCB info and OutLayer's stored collateral name for this worker.
        assert_eq!(quote_platform(&recorded_quote()).unwrap(), ("B0C06F000000".to_string(), "platform"));
    }

    #[test]
    fn signed_objects_are_cut_verbatim() {
        let body = r#"{"tcbInfo":{"id":"TDX","s":"a}\"b{","n":[{"x":1}]},"signature":"ab"}"#;
        assert_eq!(raw_json_object(body, "tcbInfo").unwrap(), r#"{"id":"TDX","s":"a}\"b{","n":[{"x":1}]}"#);
        assert_eq!(percent_decode("-----BEGIN%20CERT%0A-----"), "-----BEGIN CERT\n-----");
    }
}

//! `antibody zk` (docs/zk.md): T2, a validity proof of a disclosed verdict.
//!
//! - `prove --run <id>`: gathers the run's inputs from NEAR (the exact request near-antibody sent the TEE,
//!   from the transaction that requested the run; the base witness, the round's manifest and the blob,
//!   from FastFS), each checked against the hash the chain holds; re-runs the verifier natively and
//!   compares the receipt with the TEE's; then proves it with `antibody-zk-prove` (SP1 6.10.0, Groth16)
//!   on SP1's prover network or a local CUDA machine.
//! - `submit --run <id>`: sends the proof to near-t2, which verifies it, reads near-antibody's records of
//!   the run and records T2 (or a contradiction).
//! - `status --run <id>`: the run's T2 record.
//!
//! It claims no privacy: it proves a verdict whose proof is disclosed (its content key published).
//! Payouts in the locked deployment do not depend on it.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

use antibody_verifier::facts::sha256;
use antibody_zk_core::{decode_public_values, rerun, Disclosed};
use base64::Engine as _;
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{blob, cmd, ctx::Ctx};

/// The guest's program vkey: SP1's docker build of crates/zk/guest (the zk workflow), pinned by near-t2.
pub const PROGRAM_VKEY: &str = "0x00e87aaad0ca36c285eec6bccab0cace523784873ad65e45b3b65e2759288203";
/// That build's ELF: the prover is sent these exact bytes.
pub const ELF_SHA256: &str = "7fea93b80cc71db7ccd2315acc9b85c583fd3164bc95c6e6b68b6119d1c39702";
/// Where the repository keeps it.
pub const ELF_PATH: &str = "fixtures/zk/guest/antibody-zk-guest.elf";
/// near-t2's storage prepayment for a record (the unused part is refunded).
pub const RECORD_DEPOSIT: u128 = 20 * 10u128.pow(21);
pub const SUBMIT_TGAS: u64 = 200;

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// The exact request near-antibody sent OutLayer for `run_id`, from a transaction that requested it: the
/// `request_execution` call whose `input_data` hashes to the run's `request_sha256`.
pub fn request_in_tx(ctx: &Ctx, hash: &str, signer: &str, want: &str) -> Result<Vec<u8>> {
    let params = json!({"tx_hash": hash, "sender_account_id": signer, "wait_until": "FINAL"});
    // A regular RPC keeps a few days of transactions; an archival one keeps them all.
    let t = match ctx.near.call("EXPERIMENTAL_tx_status", params.clone()) {
        Ok(t) => t,
        Err(_) if ctx.near.network != "sandbox" => {
            crate::near::Near::new(&ctx.near.network, Some(antibody_verifier::fastfs::archival_rpc(&ctx.near.network).into())).call("EXPERIMENTAL_tx_status", params)?
        }
        Err(e) => return Err(e),
    };
    for r in t["receipts"].as_array().into_iter().flatten() {
        for a in r["receipt"]["Action"]["actions"].as_array().into_iter().flatten() {
            let fc = &a["FunctionCall"];
            if fc["method_name"] != "request_execution" {
                continue;
            }
            let Some(args) = fc["args"].as_str().and_then(|a| base64::engine::general_purpose::STANDARD.decode(a).ok()) else { continue };
            let Ok(v) = serde_json::from_slice::<Value>(&args) else { continue };
            if let Some(data) = v["input_data"].as_str() {
                if hex::encode(sha256(data.as_bytes())) == want {
                    return Ok(data.as_bytes().to_vec());
                }
            }
        }
    }
    Err(eyre!("transaction {hash} sent OutLayer no request hashing to {want} (an old transaction may need an archival --rpc)"))
}

/// Where the transaction that requested a run comes from.
pub enum TxFrom {
    /// Given (`--tx`); its signer is the run's.
    Given(String),
    /// An Antibody API: the commit's `run_requested` events name their transactions.
    Api(String),
    /// The hunter's own records in the state directory.
    Local,
}

/// Every commit record under the state directory (a hunter's, a practice try's, and a hunt account's).
fn local_records(state: &Path) -> Vec<Value> {
    let mut dirs = vec![state.join("submissions")];
    if let Ok(rd) = std::fs::read_dir(state.join("practice")) {
        dirs.extend(rd.flatten().map(|e| e.path().join("state/submissions")));
    }
    // `hunt/<vending>/<name>` (crate::hunt::account_dir).
    for vending in std::fs::read_dir(state.join("hunt")).into_iter().flatten().flatten() {
        dirs.extend(std::fs::read_dir(vending.path()).into_iter().flatten().flatten().map(|e| e.path().join("state/submissions")));
    }
    dirs.iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten())
        .filter_map(|e| serde_json::from_slice::<Value>(&std::fs::read(e.path()).ok()?).ok())
        .collect()
}

fn requested_by(ctx: &Ctx, run: &Value, from: &TxFrom) -> Result<Vec<u8>> {
    let id = run["id"].as_u64().unwrap_or_default();
    let want = run["request_sha256"].as_str().ok_or_else(|| eyre!("run {id} has no request hash"))?;
    let signer = run["signer"].as_str().unwrap_or_default();
    let txs: Vec<(String, String)> = match from {
        TxFrom::Given(h) => vec![(h.clone(), signer.to_string())],
        TxFrom::Api(api) => {
            let commit = run["commit"].as_u64().ok_or_else(|| eyre!("run {id} names no commit: pass --tx"))?;
            let v: Value = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()?
                .get(format!("{}/v1/commits/{commit}", api.trim_end_matches('/')))
                .send()?
                .json()?;
            v["events"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| e["event"] == "run_requested" && e["data"]["run_id"].as_u64() == Some(id))
                .filter_map(|e| Some((e["tx"].as_str()?.to_string(), e["signer"].as_str().unwrap_or(signer).to_string())))
                .collect()
        }
        TxFrom::Local => local_records(&ctx.state_dir)
            .iter()
            .filter(|r| r["commit_id"] == run["commit"])
            .filter_map(|r| r["reveal_tx"].as_str().map(|h| (h.to_string(), signer.to_string())))
            .collect(),
    };
    let mut why = vec![];
    for (h, s) in &txs {
        match request_in_tx(ctx, h, s, want) {
            Ok(r) => return Ok(r),
            Err(e) => why.push(format!("{h}: {e}")),
        }
    }
    Err(eyre!("found no transaction that requested run {id} ({}): pass --tx <hash> (the reveal or retry that started it) or --api <url>",
              if why.is_empty() { "none to try".into() } else { why.join("; ") }))
}

/// The run's content key: given, else the hunter's record, else a practice try's public key.
fn content_key(ctx: &Ctx, run: &Value, program_key: &str, hunter: &str, given: Option<&str>) -> Result<([u8; 32], &'static str)> {
    let parse = |h: &str| -> Result<[u8; 32]> { hex::decode(h.trim().trim_start_matches("0x"))?.try_into().map_err(|_| eyre!("a content key is 32 bytes")) };
    if let Some(k) = given {
        return Ok((parse(k)?, "given"));
    }
    if let Some(r) = local_records(&ctx.state_dir).iter().find(|r| r["commit_id"] == run["commit"] && r["program_key"] == json!(program_key)) {
        if let Some(k) = r["content_key"].as_str() {
            return Ok((parse(k)?, "the hunter's record"));
        }
    }
    if ctx.dep.practice.as_ref().map(|p| p.program as u64) == run["program"].as_u64() {
        return Ok((blob::practice_content_key(program_key, hunter), "a practice try's public key"));
    }
    Err(eyre!("no content key for run {}: a disclosed proof's key is published with it (--key <hex>)", run["id"]))
}

/// Gathers run `run_id`'s inputs into `out`, re-runs it natively and writes the public values a proof
/// would commit. Errors when the inputs do not match the chain's hashes.
pub fn gather(ctx: &Ctx, run_id: u64, key: Option<&str>, from: &TxFrom, out: &Path) -> Result<Value> {
    let run = cmd::run(ctx, run_id)?;
    let program = run["program"].as_u64().ok_or_else(|| eyre!("run {run_id}: no program"))? as u32;
    let p = ctx.program(program)?;
    let program_key = p["key"].as_str().unwrap_or_default().to_string();
    let commit = match run["commit"].as_u64() {
        Some(c) => ctx.commit(c)?,
        None => Value::Null,
    };
    let hunter = commit["hunter"].as_str().unwrap_or_default().to_string();
    let request = requested_by(ctx, &run, from)?;
    let req: Value = serde_json::from_slice(&request)?;
    let fetch = |r: &Value, what: &str| -> Result<Vec<u8>> {
        let loc = r["near_tx"].as_str().ok_or_else(|| eyre!("{what}: no location in the request"))?;
        let b = blob::fetch(&ctx.near, loc)?;
        if Some(hex::encode(sha256(&b)).as_str()) != r["sha256"].as_str() {
            return Err(eyre!("{what}: {loc} does not hash to the request's {}", r["sha256"]));
        }
        Ok(b)
    };
    let base = fetch(&req["base"], "the base witness")?;
    let round_manifest = if req["round_manifest"].is_object() { Some(fetch(&req["round_manifest"], "the round's manifest")?) } else { None };
    let blob_bytes = if req["blob"]["inline"].is_string() { None } else { Some(fetch(&req["blob"], "the blob")?) };
    let (k, key_source) = content_key(ctx, &run, &program_key, &hunter, key)?;
    let r = rerun(&Disclosed { request: &request, base: &base, round_manifest: round_manifest.as_deref(), blob: blob_bytes.as_deref(), content_key: k })
        .map_err(|e| eyre!("run {run_id} does not re-run: {e}"))?;
    let pv = r.public_values();
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("request.json"), &request)?;
    std::fs::write(out.join("base.bin"), &base)?;
    if let Some(m) = &round_manifest {
        std::fs::write(out.join("round_manifest.bin"), m)?;
    }
    if let Some(b) = &blob_bytes {
        std::fs::write(out.join("blob.bin"), b)?;
    }
    std::fs::write(out.join("content_key.hex"), hex::encode(k) + "\n")?;
    std::fs::write(out.join("public_values.bin"), &pv)?;
    let tee = &run["outcome"]["Answered"]["facts"];
    let receipt = hex::encode(r.receipt_sha256);
    let expected = json!({
        "network": ctx.near.network, "program": program, "commit_id": run["commit"], "run_id": run_id, "mode": run["mode"], "hunter": hunter,
        "request_sha256": run["request_sha256"], "base_sha256": req["base"]["sha256"],
        "tee": {"tier": run["outcome"]["Answered"]["tier"], "kind": tee["kind"], "keys": tee["keys"], "receipt_sha256": tee["receipt_sha256"]},
    });
    std::fs::write(out.join("expected.json"), serde_json::to_vec_pretty(&expected)?)?;
    Ok(json!({
        "run_id": run_id, "program": program, "dir": out, "content_key_from": key_source,
        "native": {"kind": format!("{:?}", r.receipt.kind), "receipt_sha256": receipt, "verifier": r.verifier()},
        "tee": {"kind": tee["kind"], "receipt_sha256": tee["receipt_sha256"], "tier": run["outcome"]["Answered"]["tier"]},
        "receipt_equals_the_tees": tee["receipt_sha256"].as_str() == Some(receipt.as_str()),
        "public_values_sha256": hex::encode(sha256(&pv)), "public_values_bytes": pv.len(),
    }))
}

/// Proves a case directory with `antibody-zk-prove` (its JSON line), after checking the ELF is the
/// pinned build. `mode`: network or cuda.
pub fn prove(dir: &Path, elf: &Path, prover: &Path, mode: &str, key_file: Option<&Path>) -> Result<Value> {
    let bytes = std::fs::read(elf).map_err(|e| eyre!("the guest ELF {}: {e} (it is in the repository: {ELF_PATH})", elf.display()))?;
    let got = hex::encode(sha256(&bytes));
    if got != ELF_SHA256 {
        return Err(eyre!("{} has sha256 {got}, not the pinned docker build's {ELF_SHA256}: near-t2 would refuse its proofs", elf.display()));
    }
    let mut c = Command::new(prover);
    c.args(["--mode", mode, "--vkey", PROGRAM_VKEY]);
    if let Some(k) = key_file {
        c.arg("--key-file").arg(k);
    }
    c.arg(elf).arg(dir);
    let out = c.output().map_err(|e| eyre!("{}: {e} (build it: cargo build --release --manifest-path crates/zk/prove/Cargo.toml)", prover.display()))?;
    if !out.status.success() {
        return Err(eyre!("antibody-zk-prove failed: {}", String::from_utf8_lossy(&out.stderr).lines().rev().take(6).collect::<Vec<_>>().join(" | ")));
    }
    let line = String::from_utf8_lossy(&out.stdout).lines().rev().find(|l| l.starts_with('{')).map(String::from).ok_or_else(|| eyre!("antibody-zk-prove printed no result"))?;
    Ok(serde_json::from_str(&line)?)
}

/// near-t2's account: given, else the deployment's.
pub fn contract(ctx: &Ctx, given: Option<&str>) -> Result<String> {
    given.map(String::from).or_else(|| ctx.dep.t2.clone()).ok_or_else(|| eyre!("the {} deployment names no near-t2 contract (pass --contract)", ctx.near.network))
}

/// Submits a case directory's proof to near-t2: a free view check first, then the transaction (anyone
/// may submit; the record's storage is prepaid and the rest refunded), then the record.
pub fn submit(ctx: &Ctx, t2: &str, run_id: u64, dir: &Path) -> Result<Value> {
    let pv = std::fs::read(dir.join("public_values.bin")).map_err(|e| eyre!("{}: public_values.bin: {e}", dir.display()))?;
    let proof = std::fs::read(dir.join("proof.bin")).map_err(|e| eyre!("{}: proof.bin: {e} (antibody zk prove first)", dir.display()))?;
    let decoded = decode_public_values(&pv).map_err(|e| eyre!("{e}"))?;
    if decoded.run_id != run_id {
        return Err(eyre!("{} holds run {}'s proof, not run {run_id}'s", dir.display(), decoded.run_id));
    }
    let args = json!({"public_values": b64(&pv), "proof": b64(&proof)});
    if ctx.near.view(t2, "verify_proof", args.clone())? != json!(true) {
        return Err(eyre!("{t2} does not verify this proof against its pinned keys (view verify_proof): not sent"));
    }
    if let Some(r) = existing(ctx, t2, run_id)? {
        return Ok(json!({"run_id": run_id, "already": true, "record": r}));
    }
    let signer = ctx.signer()?;
    let mut a = args;
    a["run_id"] = json!(run_id.to_string());
    let out = ctx.near.function_call(&signer, t2, "submit", a, SUBMIT_TGAS, RECORD_DEPOSIT)?;
    let s = cmd::ensure_ok(ctx, &out, "submit")?;
    let events: Vec<Value> = crate::near::events(&out).into_iter().filter(|e| e["standard"] == "antibody-t2").collect();
    // Views read a final block, a moment behind the transaction.
    let t0 = std::time::Instant::now();
    let record = loop {
        if let Some(r) = existing(ctx, t2, run_id)? {
            break Some(r);
        }
        if t0.elapsed().as_secs() > 30 {
            break None;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    };
    Ok(json!({"run_id": run_id, "contract": t2, "tx": s, "events": events, "record": record}))
}

fn existing(ctx: &Ctx, t2: &str, run_id: u64) -> Result<Option<Value>> {
    let r = ctx.near.view(t2, "get_record", json!({"run_id": run_id.to_string()}))?;
    Ok((!r.is_null()).then_some(r))
}

pub fn status(ctx: &Ctx, t2: &str, run_id: u64) -> Result<Value> {
    Ok(json!({"run_id": run_id, "contract": t2, "record": existing(ctx, t2, run_id)?, "pins": ctx.near.view(t2, "get_pins", json!({}))?}))
}

/// Every near-t2 record, by run.
pub fn records(ctx: &Ctx, t2: &str) -> Result<BTreeMap<u64, Value>> {
    let mut out = BTreeMap::new();
    loop {
        let page = ctx.near.view(t2, "list_records", json!({"from": out.len(), "limit": 100}))?;
        let page = page.as_array().cloned().unwrap_or_default();
        if page.is_empty() {
            return Ok(out);
        }
        for r in page {
            if let Some(id) = r["run_id"].as_u64() {
                out.insert(id, r);
            }
        }
    }
}

/// A run's tier: T2 when near-t2 confirmed it, else the TEE tier near-antibody recorded (T1 or T0).
pub fn tier(run: &Value, t2: Option<&Value>) -> String {
    match t2.map(|r| r["outcome"].as_str().unwrap_or_default()) {
        Some("confirmed") => "T2".into(),
        Some("contradicts") => "contradicted (T2)".into(),
        _ => cmd::run_view(run)["tier"].as_str().unwrap_or("-").to_string(),
    }
}

/// The default case directory for a run.
pub fn case_dir(ctx: &Ctx, run_id: u64) -> PathBuf {
    ctx.state_dir.join("zk").join(run_id.to_string())
}

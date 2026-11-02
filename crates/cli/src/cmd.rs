//! Command implementations. Each returns JSON (explorer links included); `main` prints it and the
//! MCP server returns it to agents.

use std::path::Path;

use alloy_primitives::B256;
use antibody_verifier::{
    facts::{sha256, ProgramReceipt},
    manifest::{Manifest, MANIFEST_SCHEMA_V2},
    sealed,
};
use antibody_witness::build::{parse_bundle, BundleFile};
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{
    blob,
    ctx::Ctx,
    near::{self, Action, Permission, Signer, ONE_NEAR, ONE_YOCTO, TGAS},
};

/// Gas for calls that start a verifier run: the method, OutLayer (100) and the answer's callback (60).
pub const RUN_TGAS: u64 = 300;
pub const TUSD_DECIMALS: u32 = 6;
/// Larger sealed blobs go through FastFS: near-antibody keeps every request within OutLayer's free input.
pub const MAX_INLINE_BLOB: usize = 15 * 1024;
/// NEAR credit kept for storage and verification runs (NEP-145 style; unused credit is withdrawable).
pub const MIN_NEAR_CREDIT: u128 = ONE_NEAR / 4;

pub fn parse_tusd(s: &str) -> Result<u128> {
    let (i, f) = s.split_once('.').unwrap_or((s, ""));
    if f.len() > TUSD_DECIMALS as usize || i.is_empty() || !i.chars().chain(f.chars()).all(|c| c.is_ascii_digit()) {
        return Err(eyre!("bad tUSD amount {s:?}"));
    }
    Ok(i.parse::<u128>()? * 10u128.pow(TUSD_DECIMALS) + format!("{f:0<6}").parse::<u128>()?)
}

pub fn fmt_tusd(raw: u128) -> String {
    format!("{}.{:06}", raw / 1_000_000, raw % 1_000_000)
}

pub fn u128_of(v: &Value) -> u128 {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64().map(|x| x as u128)).unwrap_or(0)
}

fn write_json(path: &Path, v: &Value) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(v)?)?;
    Ok(())
}

fn tx_summary(ctx: &Ctx, out: &Value) -> Value {
    let h = near::tx_hash(out);
    json!({"tx": h, "explorer": ctx.near.explorer_tx(&h), "failures": near::failures(out), "events": near::events(out)})
}

fn ensure_ok(ctx: &Ctx, out: &Value, what: &str) -> Result<Value> {
    let s = tx_summary(ctx, out);
    let f = near::failures(out);
    if !f.is_empty() {
        return Err(eyre!("{what} failed: {} ({})", f.join("; "), s["explorer"].as_str().unwrap_or_default()));
    }
    Ok(s)
}

/// The data of the first event `name` in a transaction summary.
pub fn event(summary: &Value, name: &str) -> Option<Value> {
    summary["events"].as_array().into_iter().flatten().find(|e| e["event"] == name).map(|e| e["data"][0].clone())
}

/// The run a transaction started (near-antibody's `run_requested` event).
pub fn run_of(summary: &Value) -> Result<u64> {
    event(summary, "run_requested").and_then(|d| d["run_id"].as_u64()).ok_or_else(|| eyre!("the transaction started no run"))
}

fn ft_transfer_call(ctx: &Ctx, signer: &Signer, amount: u128, msg: Value) -> Result<Value> {
    ctx.near.function_call(
        signer,
        &ctx.dep.tusd,
        "ft_transfer_call",
        json!({"receiver_id": ctx.dep.antibody, "amount": amount.to_string(), "msg": msg.to_string()}),
        300,
        ONE_YOCTO,
    )
}

pub fn call(ctx: &Ctx, method: &str, args: Value, tgas: u64) -> Result<Value> {
    let out = ctx.near.function_call(&ctx.signer()?, &ctx.dep.antibody, method, args, tgas, 0)?;
    ensure_ok(ctx, &out, method)
}

// ------------------------------------------------------------------------- keys

pub fn keygen_x25519(out: &Path) -> Result<Value> {
    let sk = blob::random32()?;
    let pk = sealed::x25519_public(sk);
    write_json(out, &json!({"type": "x25519", "public_key": hex::encode(pk), "secret_key": hex::encode(sk)}))?;
    Ok(json!({"public_key": hex::encode(pk), "file": out}))
}

pub fn keygen_near(account: Option<String>, out: &Path) -> Result<Value> {
    let s = Signer::generate(account)?;
    write_json(out, &s.to_file_json())?;
    Ok(json!({"account_id": s.account_id, "public_key": s.public_key_str(), "file": out}))
}

fn project_public(path: &Path) -> Result<String> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    Ok(v["public_key"].as_str().ok_or_else(|| eyre!("no public_key in {}", path.display()))?.to_string())
}

// ------------------------------------------------------------------------ views

pub fn meta(ctx: &Ctx, program: u32) -> Result<Value> {
    let m = ctx.view("get_program_meta", json!({"program_id": program}))?;
    if m.is_null() {
        return Err(eyre!("program {program} does not exist on {}", ctx.dep.antibody));
    }
    Ok(m)
}

pub fn run(ctx: &Ctx, id: u64) -> Result<Value> {
    let r = ctx.view("get_run", json!({"run_id": id}))?;
    if r.is_null() {
        return Err(eyre!("run {id} does not exist on {}", ctx.dep.antibody));
    }
    Ok(r)
}

pub fn account(ctx: &Ctx, account: &str) -> Result<Value> {
    ctx.view("get_account", json!({"account_id": account}))
}

/// A run's verdict in one object: kind, keys, impact and tier, or its failure.
pub fn run_view(r: &Value) -> Value {
    let o = &r["outcome"];
    if let Some(a) = o.get("Answered") {
        let f = &a["facts"];
        json!({"run_id": r["id"], "mode": r["mode"], "commit_id": r["commit"], "kind": f["kind"], "keys": f["keys"],
               "impact_usd_micro": f["impact_usd_micro"], "protected_harm_usd_micro": f["protected_harm_usd_micro"],
               "reason": f["reason"], "receipt_sha256": f["receipt_sha256"], "tier": a["tier"], "answered_at": a["at"]})
    } else if let Some(f) = o.get("Failed") {
        json!({"run_id": r["id"], "mode": r["mode"], "commit_id": r["commit"], "failed": f["reason"]})
    } else {
        json!({"run_id": r["id"], "mode": r["mode"], "commit_id": r["commit"], "pending": true})
    }
}

/// A commit state's name (`Holder`, `Done:Paid`, …).
pub fn state_label(s: &Value) -> String {
    match s {
        Value::String(x) => x.clone(),
        Value::Object(m) => match m.iter().next() {
            Some((k, Value::Object(inner))) if k == "Done" => format!("Done:{}", inner.keys().next().cloned().unwrap_or_default()),
            Some((k, Value::String(inner))) if k == "Done" => format!("Done:{inner}"),
            Some((k, _)) => k.clone(),
            None => String::new(),
        },
        _ => String::new(),
    }
}

/// Amount paid to a commit (0 unless `Done:Paid`).
pub fn paid_of(c: &Value) -> u128 {
    u128_of(&c["state"]["Done"]["Paid"]["amount"])
}

// ---------------------------------------------------------------------- program

pub fn manifest_compile(toml: &Path, out: &Path) -> Result<Value> {
    let (bytes, hash) = antibody_witness::program::compile_manifest(&std::fs::read_to_string(toml)?)?;
    std::fs::write(out, &bytes)?;
    Ok(json!({"manifest": out, "bytes": bytes.len(), "sha256": format!("{hash:#x}")}))
}

/// The manifest of `round` (0: the program's own), fetched from its committed location and checked
/// against the committed hash.
pub fn program_manifest_at(ctx: &Ctx, program: u32, round: u32) -> Result<Vec<u8>> {
    let p = ctx.program(program)?;
    let m = meta(ctx, program)?;
    let (location, want) = match round {
        0 => (m["manifest_url"].as_str().unwrap_or_default().to_string(), p["manifest_sha256"].as_str().unwrap_or_default().to_string()),
        r => {
            let i = (r - 1) as usize;
            (m["round_manifests"][i]["location"].as_str().unwrap_or_default().to_string(), p["round_manifests"][i].as_str().unwrap_or_default().to_string())
        }
    };
    let bytes = blob::fetch(&ctx.near, &location)?;
    let h = hex::encode(sha256(&bytes));
    if h != want {
        return Err(eyre!("round {round} manifest at {location} hashes to {h}, not the committed {want}"));
    }
    Ok(bytes)
}

/// The manifest new commits are judged under: the current round's.
pub fn program_manifest(ctx: &Ctx, program: u32) -> Result<Vec<u8>> {
    let round = ctx.program(program)?["round"].as_u64().unwrap_or(0) as u32;
    program_manifest_at(ctx, program, round)
}

pub fn storage_deposit(ctx: &Ctx, near_amount: &str) -> Result<Value> {
    let signer = ctx.signer()?;
    let out = ctx.near.function_call(&signer, &ctx.dep.antibody, "storage_deposit", json!({}), 30, parse_near(near_amount)?)?;
    let s = ensure_ok(ctx, &out, "storage_deposit")?;
    Ok(json!({"account": signer.account_id, "credit": account(ctx, &signer.account_id)?["near"], "tx": s}))
}

/// Tops the signer's NEAR credit up to `want` yoctoNEAR (storage and verification are paid from it).
fn ensure_near_credit(ctx: &Ctx, signer: &Signer, want: u128) -> Result<Option<Value>> {
    let have = u128_of(&account(ctx, &signer.account_id)?["near"]);
    if have >= want {
        return Ok(None);
    }
    let out = ctx.near.function_call(signer, &ctx.dep.antibody, "storage_deposit", json!({}), 30, want - have)?;
    Ok(Some(ensure_ok(ctx, &out, "storage_deposit")?))
}

pub fn program_create(ctx: &Ctx, manifest: &Path, escrow: &str, project_key: &Path, manifest_url: Option<String>) -> Result<Value> {
    let signer = ctx.signer()?;
    let bytes = std::fs::read(manifest)?;
    let m = Manifest::parse(&bytes).map_err(|e| eyre!(e))?;
    if m.schema != MANIFEST_SCHEMA_V2 {
        return Err(eyre!("near-antibody v2 takes {MANIFEST_SCHEMA_V2} manifests (with a settlement section)"));
    }
    if m.program.project != signer.account_id {
        return Err(eyre!("manifest.program.project is {}, but you sign as {}", m.program.project, signer.account_id));
    }
    let upload = match manifest_url {
        Some(u) => json!({"location": u}),
        None => blob::upload(ctx, &signer, &format!("antibody/manifests/{}.json", hex::encode(sha256(&bytes))), &bytes)?,
    };
    let credit = ensure_near_credit(ctx, &signer, MIN_NEAR_CREDIT)?;
    let msg = json!({"create_program": {"manifest": String::from_utf8(bytes.clone())?, "manifest_url": upload["location"],
                     "project_pk": project_public(project_key)?}});
    let out = ft_transfer_call(ctx, &signer, parse_tusd(escrow)?, msg)?;
    let s = ensure_ok(ctx, &out, "create_program")?;
    let created = event(&s, "program_created").ok_or_else(|| eyre!("no program_created event"))?;
    Ok(json!({"program": created, "anchor": event(&s, "anchor_checked").or_else(|| event(&s, "program_rejected")),
              "manifest_upload": upload, "manifest_sha256": hex::encode(sha256(&bytes)), "storage_deposit": credit, "tx": s}))
}

/// The program's base witness (docs/spec.md §4.4): the anchor header, the invariants, and every
/// account and slot the activation checks read at the anchor, proven against its state root. The
/// verifier's own activation check passes on it.
pub fn build_base(ctx: &Ctx, manifest_json: &[u8], program_key: &str) -> Result<(Vec<u8>, Value)> {
    let pool = ctx.eth_pool()?;
    let (bytes, meta) = antibody_witness::program::build_base(&pool, manifest_json, 1, program_key)?;
    Ok((bytes, serde_json::to_value(&meta)?))
}

/// Pins the base witness and asks the verifier for the program's sealing key (T1 makes it count).
pub fn program_activate(ctx: &Ctx, program: u32, wait: bool) -> Result<Value> {
    let signer = ctx.signer()?;
    let p = ctx.program(program)?;
    let key = p["key"].as_str().unwrap_or_default().to_string();
    let manifest_json = program_manifest_at(ctx, program, 0)?;
    let (base, built) = build_base(ctx, &manifest_json, &key)?;
    let sha = hex::encode(sha256(&base));
    let up = blob::upload(ctx, &signer, &format!("antibody/base/{sha}.bin"), &base)?;
    ensure_near_credit(ctx, &signer, MIN_NEAR_CREDIT)?;
    let loc = json!({"location": up["near_tx"], "sha256": sha});
    let out = ctx.near.function_call(&signer, &ctx.dep.antibody, "activate", json!({"program_id": program, "base": loc}), RUN_TGAS, 0)?;
    let s = ensure_ok(ctx, &out, "activate")?;
    let run_id = run_of(&s)?;
    let mut res = json!({"program": program, "run_id": run_id, "base": {"bytes": base.len(), "sha256": sha, "upload": up,
                         "fixpoint_passes": built["fixpoint_passes"]}, "tx": s});
    if wait {
        res["result"] = wait_run(ctx, run_id, 240)?;
    }
    Ok(res)
}

pub fn program_open(ctx: &Ctx, program: u32) -> Result<Value> {
    call(ctx, "open_program", json!({"program_id": program}), 30)
}

pub fn programs(ctx: &Ctx, from: u32, limit: u32) -> Result<Value> {
    let ps = ctx.view("list_programs", json!({"from": from, "limit": limit}))?;
    let mut rows = Vec::new();
    for p in ps.as_array().into_iter().flatten() {
        let id = p["id"].as_u64().unwrap_or_default() as u32;
        let m = meta(ctx, id)?;
        let l = &p["ledger"];
        rows.push(json!({"id": id, "key": p["key"], "name": m["name"], "project": p["project"], "phase": p["phase"], "anchor": p["anchor"],
                         "round": p["round"], "commits": p["commits"], "escrow_tusd": fmt_tusd(u128_of(&l["available"]) + u128_of(&l["reserved"])),
                         "paid_tusd": fmt_tusd(u128_of(&l["paid"]))}));
    }
    Ok(json!({"contract": ctx.dep.antibody, "programs": rows, "stats": ctx.view("get_stats", json!({}))?}))
}

pub fn list_commits(ctx: &Ctx, program: u32) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    loop {
        let page = ctx.view("list_commits", json!({"program_id": program, "from": out.len(), "limit": 50}))?;
        let page = page.as_array().cloned().unwrap_or_default();
        if page.is_empty() {
            return Ok(out);
        }
        out.extend(page);
    }
}

pub fn program_show(ctx: &Ctx, program: u32) -> Result<Value> {
    let p = ctx.program(program)?;
    let mut rows = Vec::new();
    for c in list_commits(ctx, program)? {
        let verdict = match c["run"].as_u64() {
            Some(r) => run_view(&run(ctx, r)?),
            None => Value::Null,
        };
        rows.push(json!({"id": c["id"], "index": c["index"], "round": c["round"], "hunter": c["hunter"], "state": state_label(&c["state"]),
                         "paid_tusd": fmt_tusd(paid_of(&c)), "verdict": verdict}));
    }
    Ok(json!({"program": p, "meta": meta(ctx, program)?, "record": ctx.view("program_record", json!({"program_id": program}))?,
              "audit": ctx.view("audit", json!({"program_id": program}))?, "commits": rows}))
}

pub fn top_up(ctx: &Ctx, program: u32, amount: &str) -> Result<Value> {
    let out = ft_transfer_call(ctx, &ctx.signer()?, parse_tusd(amount)?, json!({"top_up": {"program_id": program}}))?;
    ensure_ok(ctx, &out, "top_up")
}

// ------------------------------------------------------------------ witnesses

/// Builds a program witness for `bundle_file` from the Ethereum RPC (state at the anchor).
pub fn build_witness(ctx: &Ctx, manifest_json: &[u8], bundle_file: &Path, salt: Option<B256>, program_key: &str) -> Result<(Vec<u8>, Value, ProgramReceipt, B256)> {
    let m = Manifest::parse(manifest_json).map_err(|e| eyre!(e))?;
    let bf: BundleFile = serde_json::from_str(&std::fs::read_to_string(bundle_file)?)?;
    let salt = match salt {
        Some(s) => s,
        None => B256::from(blob::random32()?),
    };
    let bundle = parse_bundle(&bf, salt)?;
    let pool = ctx.eth_pool()?;
    let (bytes, meta, receipt) =
        antibody_witness::program::build_program(&pool, manifest_json, &m.invariants.runtime_code, bundle, 1, program_key)?;
    Ok((bytes, serde_json::to_value(&meta)?, receipt, salt))
}

fn verdict_view(r: &ProgramReceipt) -> Value {
    json!({"kind": r.kind, "reason": r.reason, "broken_mask": r.broken_mask,
           "broken": r.broken.iter().map(|b| json!({"invariant": b.name, "severity": b.severity,
                "function": format!("{}.{}", b.target_name.clone().unwrap_or_else(|| b.target.to_string()), b.selector)})).collect::<Vec<_>>(),
           "impact_usd": r.impact.as_ref().map(|i| i.impact_usd.clone()), "evaluations": r.evaluations,
           "calls": r.calls.iter().map(|c| json!({"success": c.success, "gas_used": c.gas_used})).collect::<Vec<_>>()})
}

/// Local, keyless pre-check of a PoC: proven witness at the anchor + native verifier run.
pub fn check(ctx: &Ctx, program: Option<u32>, manifest: Option<&Path>, bundle: &Path, salt: Option<B256>) -> Result<Value> {
    let manifest_json = match (program, manifest) {
        (_, Some(m)) => std::fs::read(m)?,
        (Some(p), None) => program_manifest(ctx, p)?,
        _ => return Err(eyre!("pass --program or --manifest")),
    };
    let key = match program {
        Some(p) => ctx.program(p)?["key"].as_str().unwrap_or("local").to_string(),
        None => "local".to_string(),
    };
    let (bytes, meta, receipt, salt) = build_witness(ctx, &manifest_json, bundle, salt, &key)?;
    Ok(json!({"verdict": verdict_view(&receipt), "witness_bytes": bytes.len(), "fixpoint_passes": meta["fixpoint_passes"],
              "salt_note": "a fresh random salt is drawn per submission unless --salt is given", "salt": format!("{salt:#x}"),
              "receipt": receipt}))
}

// --------------------------------------------------------------------- hunting

/// Tokens into your credit at near-antibody (bonds, and an agent's bonds, are taken from it).
pub fn deposit(ctx: &Ctx, amount: &str) -> Result<Value> {
    let signer = ctx.signer()?;
    let credit = ensure_near_credit(ctx, &signer, MIN_NEAR_CREDIT)?;
    let out = ft_transfer_call(ctx, &signer, parse_tusd(amount)?, json!("deposit"))?;
    let mut s = ensure_ok(ctx, &out, "deposit")?;
    s["storage_deposit"] = credit.unwrap_or(Value::Null);
    Ok(s)
}

pub fn faucet(ctx: &Ctx) -> Result<Value> {
    let out = ctx.near.function_call(&ctx.signer()?, &ctx.dep.tusd, "faucet", json!({}), 30, 0)?;
    ensure_ok(ctx, &out, "faucet")
}

/// Credit, then withdraw (one yoctoNEAR: only a full-access key withdraws).
pub fn withdraw(ctx: &Ctx, amount: &str) -> Result<Value> {
    let out = ctx.near.function_call(&ctx.signer()?, &ctx.dep.antibody, "withdraw", json!({"amount": parse_tusd(amount)?.to_string()}), 60, ONE_YOCTO)?;
    ensure_ok(ctx, &out, "withdraw")
}

/// `SHA-256("antibody/commit/v2" ‖ u32_be(len) ‖ program_key ‖ u32_be(round) ‖ u32_be(len) ‖ hunter ‖ blob_sha256)`,
/// as near-antibody computes it: the commitment binds the hunter and the round.
pub fn commitment(program_key: &str, round: u32, hunter: &str, blob_sha256: &[u8; 32]) -> [u8; 32] {
    let mut m = b"antibody/commit/v2".to_vec();
    m.extend((program_key.len() as u32).to_be_bytes());
    m.extend(program_key.as_bytes());
    m.extend(round.to_be_bytes());
    m.extend((hunter.len() as u32).to_be_bytes());
    m.extend(hunter.as_bytes());
    m.extend(blob_sha256);
    sha256(&m)
}

fn sealing_pk(p: &Value) -> Result<String> {
    p["sealing_pk"].as_str().map(String::from).ok_or_else(|| eyre!("program {} is not activated yet: it has no sealing key", p["id"]))
}

/// A blob argument for near-antibody: inline up to 15 KB, otherwise a FastFS upload.
fn blob_arg(ctx: &Ctx, signer: &Signer, blob_bytes: &[u8], dir: &str) -> Result<(Value, Value)> {
    let sha = hex::encode(sha256(blob_bytes));
    if blob_bytes.len() <= MAX_INLINE_BLOB {
        return Ok((json!({"inline": hex::encode(blob_bytes)}), json!({"inline_bytes": blob_bytes.len()})));
    }
    let up = blob::upload(ctx, signer, &format!("antibody/{dir}/{sha}.bin"), blob_bytes)?;
    Ok((json!({"near_tx": {"location": up["near_tx"], "sha256": sha}}), up))
}

/// Builds the witness, seals it to the program's key, and commits with the bond (the blob stays
/// private until `reveal`). An agent's bond comes from its owner's credit; anyone else's from their
/// credit when it covers the bond, otherwise it is attached with `ft_transfer_call`.
pub fn commit(ctx: &Ctx, program: u32, bundle: &Path, salt: Option<B256>, report: Option<&Path>) -> Result<Value> {
    let signer = ctx.signer()?;
    let p = ctx.program(program)?;
    let round = p["round"].as_u64().unwrap_or(0) as u32;
    let key = p["key"].as_str().unwrap_or_default().to_string();
    let pk = sealing_pk(&p)?;
    let manifest_json = program_manifest(ctx, program)?;
    let (envelope, _meta, local, salt) = build_witness(ctx, &manifest_json, bundle, salt, &key)?;
    let report = match report {
        Some(r) => std::fs::read(r)?,
        None => Vec::new(),
    };
    let s = blob::seal(&pk, &key, &signer.account_id, envelope, report)?;
    let blob_sha = sha256(&s.blob);
    let cm = hex::encode(commitment(&key, round, &signer.account_id, &blob_sha));
    let terms = meta(ctx, program)?["terms_sha256"].clone();
    let bond = u128_of(&p["params"]["bond"]);
    let me = account(ctx, &signer.account_id)?;
    let agent = !me["agent"].is_null();
    let from_credit = agent || u128_of(&me["tokens"]) >= bond;
    let (commit_id, summary) = if from_credit {
        let out = ctx.near.function_call(&signer, &ctx.dep.antibody, "commit",
            json!({"program_id": program, "round": round, "commitment": cm, "terms_sha256": terms}), 30, 0)?;
        let summary = ensure_ok(ctx, &out, "commit")?;
        let id = near::return_value(&out).and_then(|v| v.as_u64()).ok_or_else(|| {
            eyre!("containment: the commit was refused (the bond is over this agent's cap or remaining budget)")
        })?;
        (id, summary)
    } else {
        ensure_near_credit(ctx, &signer, MIN_NEAR_CREDIT)?;
        let msg = json!({"commit": {"program_id": program, "round": round, "commitment": cm, "terms_sha256": terms}});
        let out = ft_transfer_call(ctx, &signer, bond, msg)?;
        let summary = ensure_ok(ctx, &out, "commit")?;
        let id = event(&summary, "commit").and_then(|d| d["commit_id"].as_u64()).ok_or_else(|| eyre!("no commit event"))?;
        (id, summary)
    };
    let blob_path = ctx.state_dir.join("blobs").join(format!("{}.bin", s.sha256));
    std::fs::create_dir_all(blob_path.parent().unwrap())?;
    std::fs::write(&blob_path, &s.blob)?;
    let record = json!({"program": program, "program_key": key, "round": round, "commit_id": commit_id, "hunter": signer.account_id,
                        "commitment": cm, "sha256": s.sha256, "blob": blob_path, "content_key": hex::encode(s.content_key),
                        "salt": format!("{salt:#x}"), "local_verdict": verdict_view(&local), "commit_tx": summary["tx"]});
    write_json(&ctx.record_path(program, commit_id), &record)?;
    Ok(json!({"commit_id": commit_id, "commitment": cm, "bond_from_credit": from_credit,
              "local_verdict": verdict_view(&local), "record": ctx.record_path(program, commit_id), "tx": summary,
              "note": "the content key and salt are in the local record (.antibody/): keep it private"}))
}

/// Reveals the sealed blob (inline, or uploaded with FastFS): near-antibody asks OutLayer to verify it.
pub fn reveal(ctx: &Ctx, program: u32, commit_id: u64, wait: bool) -> Result<Value> {
    let signer = ctx.signer()?;
    let path = ctx.record_path(program, commit_id);
    let mut rec: Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    let blob_bytes = std::fs::read(rec["blob"].as_str().unwrap_or_default())?;
    let (arg, up) = blob_arg(ctx, &signer, &blob_bytes, "sealed")?;
    if account(ctx, &signer.account_id)?["agent"].is_null() {
        ensure_near_credit(ctx, &signer, MIN_NEAR_CREDIT)?;
    }
    let out = ctx.near.function_call(&signer, &ctx.dep.antibody, "reveal", json!({"commit_id": commit_id, "blob": arg}), RUN_TGAS, 0)?;
    let s = ensure_ok(ctx, &out, "reveal")?;
    // Retries of an inline blob pass it again: keep the argument with the record.
    rec["blob_arg"] = arg;
    write_json(&path, &rec)?;
    let mut res = json!({"commit_id": commit_id, "run_id": run_of(&s)?, "upload": up, "tx": s});
    if wait {
        res["result"] = wait_verdict(ctx, commit_id, 240)?;
    }
    Ok(res)
}

/// One step for agents: build, seal, commit, reveal, and wait for the verdict.
pub fn submit(ctx: &Ctx, program: u32, bundle: &Path, salt: Option<B256>, report: Option<&Path>, wait: bool) -> Result<Value> {
    let c = commit(ctx, program, bundle, salt, report)?;
    let id = c["commit_id"].as_u64().unwrap();
    let r = reveal(ctx, program, id, wait)?;
    Ok(json!({"commit": c, "reveal": r}))
}

/// A new verification attempt after an infrastructure failure (the hunter, within the grace period).
pub fn retry(ctx: &Ctx, program: u32, commit_id: u64, wait: bool) -> Result<Value> {
    let rec: Value = serde_json::from_str(&std::fs::read_to_string(ctx.record_path(program, commit_id))?)?;
    let blob = match &rec["blob_arg"] {
        a if a.get("inline").is_some() => a.clone(),
        _ => Value::Null,
    };
    let s = call(ctx, "retry", json!({"commit_id": commit_id, "blob": blob}), RUN_TGAS)?;
    let mut res = json!({"commit_id": commit_id, "run_id": run_of(&s)?, "tx": s});
    if wait {
        res["result"] = wait_verdict(ctx, commit_id, 240)?;
    }
    Ok(res)
}

/// The independent second run a verdict above the instant cap needs before it is final (docs/spec.md
/// §6.2). The requester pays; anyone may ask.
pub fn second(ctx: &Ctx, program: u32, commit_id: u64, wait: bool) -> Result<Value> {
    let blob = std::fs::read_to_string(ctx.record_path(program, commit_id))
        .ok()
        .and_then(|r| serde_json::from_str::<Value>(&r).ok())
        .map(|r| r["blob_arg"].clone())
        .filter(|a| a.get("inline").is_some())
        .unwrap_or(Value::Null);
    let signer = ctx.signer()?;
    ensure_near_credit(ctx, &signer, MIN_NEAR_CREDIT)?;
    let s = call(ctx, "request_second", json!({"commit_id": commit_id, "blob": blob}), RUN_TGAS)?;
    let run_id = run_of(&s)?;
    let mut res = json!({"commit_id": commit_id, "run_id": run_id, "tx": s});
    if wait {
        res["result"] = wait_run(ctx, run_id, 240)?;
    }
    Ok(res)
}

/// The project declares a known issue before open: a sealed PoC verified like a submission, which
/// counts once T1-confirmed (docs/spec.md §7.6).
pub fn submit_known(ctx: &Ctx, program: u32, bundle: &Path, salt: Option<B256>, report: Option<&Path>, wait: bool) -> Result<Value> {
    let signer = ctx.signer()?;
    let p = ctx.program(program)?;
    let key = p["key"].as_str().unwrap_or_default().to_string();
    let pk = sealing_pk(&p)?;
    let manifest_json = program_manifest_at(ctx, program, 0)?;
    let (envelope, _, local, salt) = build_witness(ctx, &manifest_json, bundle, salt, &key)?;
    let report = match report {
        Some(r) => std::fs::read(r)?,
        None => Vec::new(),
    };
    let s = blob::seal(&pk, &key, &signer.account_id, envelope, report)?;
    let (arg, up) = blob_arg(ctx, &signer, &s.blob, "known")?;
    ensure_near_credit(ctx, &signer, MIN_NEAR_CREDIT)?;
    let out = ctx.near.function_call(&signer, &ctx.dep.antibody, "submit_known", json!({"program_id": program, "blob": arg}), RUN_TGAS, 0)?;
    let summary = ensure_ok(ctx, &out, "submit_known")?;
    let run_id = run_of(&summary)?;
    write_json(&ctx.state_dir.join("known").join(format!("{program}-{run_id}.json")),
        &json!({"program": program, "program_key": key, "run_id": run_id, "hunter": signer.account_id, "sha256": s.sha256,
                "content_key": hex::encode(s.content_key), "salt": format!("{salt:#x}")}))?;
    let mut res = json!({"run_id": run_id, "local_verdict": verdict_view(&local), "upload": up, "tx": summary});
    if wait {
        res["result"] = wait_run(ctx, run_id, 240)?;
    }
    Ok(res)
}

/// Waits until OutLayer answered run `run_id` (or the call failed).
pub fn wait_run(ctx: &Ctx, run_id: u64, timeout_s: u64) -> Result<Value> {
    let t0 = std::time::Instant::now();
    loop {
        let r = run(ctx, run_id)?;
        if r["outcome"] != json!("Pending") {
            let mut v = run_view(&r);
            v["seconds"] = t0.elapsed().as_secs().into();
            return Ok(v);
        }
        if t0.elapsed().as_secs() > timeout_s {
            return Ok(json!({"run_id": run_id, "pending": true, "note": "still verifying; check `antibody status --run`"}));
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

pub fn wait_verdict(ctx: &Ctx, commit_id: u64, timeout_s: u64) -> Result<Value> {
    let c = ctx.commit(commit_id)?;
    let r = c["run"].as_u64().ok_or_else(|| eyre!("commit {commit_id} has no verification run yet"))?;
    wait_run(ctx, r, timeout_s)
}

pub fn status(ctx: &Ctx, commit: Option<u64>, run_id: Option<u64>, program: Option<u32>, of: Option<String>) -> Result<Value> {
    let mut out = json!({});
    if let Some(c) = commit {
        let cv = ctx.commit(c)?;
        out["commit"] = json!({"state": state_label(&cv["state"]), "paid_tusd": fmt_tusd(paid_of(&cv)), "raw": cv});
        if let Some(r) = cv["run"].as_u64() {
            out["verdict"] = run_view(&run(ctx, r)?);
        }
    }
    if let Some(r) = run_id {
        out["run"] = run_view(&run(ctx, r)?);
    }
    if let Some(p) = program {
        out["program"] = program_show(ctx, p)?;
    }
    if let Some(a) = of.or_else(|| ctx.account.clone()) {
        let acc = account(ctx, &a)?;
        out["account"] = json!({"id": a, "credit_tusd": fmt_tusd(u128_of(&acc["tokens"])), "near_credit": acc["near"],
                                "agent": ctx.view("agent_statement", json!({"agent": a}))?,
                                "tusd_balance": fmt_tusd(u128_of(&ctx.near.view(&ctx.dep.tusd, "ft_balance_of", json!({"account_id": a}))?))});
    }
    Ok(out)
}

// ---------------------------------------------------------------------- keeper

/// The permissionless steps that move money once verdicts are final (docs/spec.md §7): allocate in
/// commit order, pay every funded claim whose window allows, issue the round's pause ticket, and fund
/// the debt queue.
pub fn keeper(ctx: &Ctx, program: u32) -> Result<Value> {
    let mut steps = Vec::new();
    loop {
        match call(ctx, "allocate", json!({"program_id": program, "max": 20}), 150) {
            Ok(s) => steps.push(json!({"allocate": s["tx"], "events": s["events"].as_array().map(|e| e.len())})),
            Err(e) => {
                let e = e.to_string();
                if !e.contains("nothing to allocate") {
                    steps.push(json!({"allocate_stopped": e}));
                }
                break;
            }
        }
    }
    for c in list_commits(ctx, program)? {
        let st = state_label(&c["state"]);
        let claim = &c["state"][st.as_str()];
        let funded = u128_of(&claim["debt"]) == 0 && u128_of(&claim["reserved"]) >= u128_of(&claim["amount"]);
        if (st == "Holder" || st == "Promoted") && funded {
            match call(ctx, "pay", json!({"commit_id": c["id"]}), 60) {
                Ok(s) => steps.push(json!({"paid": c["id"], "event": event(&s, "paid"), "tx": s["tx"]})),
                Err(e) => steps.push(json!({"not_paid": c["id"], "why": e.to_string()})),
            }
        }
    }
    // The round's first finding the pause policy covers gets its ticket (spec §10.1): one per round,
    // T1-confirmed, a key in the pause slots. Ineligible commits are refused by near-antibody.
    let p = ctx.program(program)?;
    if !p["ticket_rounds"].as_array().is_some_and(|r| r.contains(&p["round"])) {
        for c in list_commits(ctx, program)? {
            let st = state_label(&c["state"]);
            if c["round"] == p["round"] && c["ticket"] == json!(false) && (st == "Holder" || st == "Done:Paid") {
                if let Ok(s) = call(ctx, "pause_ticket", json!({"commit_id": c["id"]}), 300) {
                    steps.push(json!({"pause_ticket": c["id"], "tx": s["tx"], "permit_requested": event(&s, "permit_requested")}));
                    break;
                }
            }
        }
    }
    if ctx.program(program)?["debtors"].as_array().is_some_and(|d| !d.is_empty()) {
        match call(ctx, "settle_debts", json!({"program_id": program}), 100) {
            Ok(s) => steps.push(json!({"settle_debts": s["tx"]})),
            Err(e) => steps.push(json!({"settle_debts_stopped": e.to_string()})),
        }
    }
    Ok(json!({"program": program, "steps": steps}))
}

// ------------------------------------------------------------------------- init

const INIT_TOML: &str = r#"# Antibody program manifest: your bounty terms as code. Its sha256 is what you commit on NEAR.
# Fill every <...>, then:  antibody manifest antibody.toml --out manifest.json
# A complete example: fixtures/acme-local/antibody.toml in the Antibody repository.
schema = "antibody.manifest.v2"

[program]
name = "<program name>"
project = "<PROJECT>"
description = "<one line: what is in scope>"

[anchor]
# A finalized block of your staging deployment. Create the program within about 7 days of it so that
# NEAR's Ethereum light client can confirm its hash (otherwise the anchor is recorded as unconfirmed).
# `antibody anchor-check --block <n>` shows what the light client has for it.
chain_id = <CHAIN_ID>
block_number = 0
block_hash = "0x<block hash>"
evm_spec = "osaka"
trust = "light-client"

# One entry per contract in scope; code_hash = keccak256 of its runtime code at the anchor.
[[targets]]
name = "<Contract>"
address = "0x<address>"
code_hash = "0x<code hash>"

[invariants]
# The verifier serves your invariants at a reserved address derived from the program; keep this one.
address = "0xe88d533fa3f57c4d6fa6489a13e3430d30003596"
# Runtime code of your IAntibodyInvariants contract (with your addresses as immutables), and its hash.
# `antibody invariants draft --source <your contracts> --out draft/` writes a first draft.
runtime_code = "0x<runtime code>"
runtime_code_hash = "0x<keccak256 of runtime_code>"
gas_limit = 5000000

[[invariants.slots]]
bit = 0
name = "<invariant name>"
severity = "critical"

[impact]
# Accounts whose losses count as protocol loss, and committed reference prices (USD).
protocol_accounts = ["0x<address>"]
native_usd = "2000"

[[impact.tokens]]
symbol = "<SYM>"
address = "0x<token address>"
decimals = 18
usd = "1"

[capabilities]
# What a PoC may do: fresh senders with this much ETH each, this many transactions and this much gas.
max_senders = 2
max_endowment_wei = "100000000000000000000"
max_txs = 4
max_gas_per_tx = 16777216
max_total_gas = 30000000
max_blocks = 1
max_timestamp_delta = 0
max_entry_calls = 256

# Everything near-antibody enforces. Amounts are raw units of the payout token (tUSD: 6 decimals,
# 10000000000 = 10,000 tUSD); windows are seconds.
[settlement]
bond = "25000000"
challenge_bond = "100000000"
unevaluable = "refund"
terms_sha256 = "<sha256 of your terms text, 64 hex>"

[settlement.payouts]
critical = "10000000000"
high = "3000000000"
medium = "1000000000"
low = "0"
impact_bps = 0
impact_cap = "0"
decimals = 6

[settlement.instant]
# Paid in seconds when T1 and at most `cap`; larger payouts wait the challenge window.
cap = "5000000000"
budget = "10000000000"
window_secs = 86400

[settlement.fees]
protocol_bps = 0
release = "0"

[settlement.windows]
reveal = 3600
challenge = 259200
resolve = 86400
infra_grace = 86400
run_timeout = 900
round_min = 86400
rerun_window = 259200
disclosure_max_delay = 2592000
deadline = <DEADLINE>
exit_grace = 604800

[settlement.known]
max_count = 4
max_share_bps = 5000

[settlement.pause]
slots = [0]
min_impact_usd_micro = "0"
"#;

/// Scaffolds a program workspace: an annotated antibody.toml, the project's X25519 key (released
/// findings are wrapped to it), and the next steps.
pub fn init(out: &Path, project: &str, chain_id: u64, days: u64) -> Result<Value> {
    std::fs::create_dir_all(out)?;
    let toml_path = out.join("antibody.toml");
    if toml_path.exists() {
        return Err(eyre!("{} exists; not overwriting", toml_path.display()));
    }
    let deadline = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() + days * 86_400;
    let toml = INIT_TOML
        .replace("<PROJECT>", project)
        .replace("<CHAIN_ID>", &chain_id.to_string())
        .replace("<DEADLINE>", &deadline.to_string());
    std::fs::write(&toml_path, toml)?;
    let key = out.join("project.x25519.json");
    let k = if key.exists() { json!({"kept": key}) } else { keygen_x25519(&key)? };
    let steps = format!(
        "# Next steps\n\n\
         1. Write or draft your invariants: `antibody invariants draft --source <contracts> --out draft/`.\n\
         2. Deploy to staging, pick a finalized anchor block (`antibody anchor-check --block <n>`), and fill `antibody.toml`.\n\
         3. `antibody manifest antibody.toml --out manifest.json`, then `antibody check --manifest manifest.json --bundle draft/noop.bundle.json` must say NoViolation.\n\
         4. `antibody --account {project} program create --manifest manifest.json --escrow <tUSD> --project-key project.x25519.json`.\n\
         5. `program activate` (pins the base witness; the program's sealing key counts once T1-confirmed), declare known issues (`program known`), then `program open`.\n\n\
         Keep `project.x25519.json` secret: findings released to you are wrapped to it.\n"
    );
    std::fs::write(out.join("NEXT_STEPS.md"), steps)?;
    Ok(json!({"out": out, "manifest_template": toml_path, "project_key": k, "deadline_unix": deadline,
              "next": out.join("NEXT_STEPS.md")}))
}

// ---------------------------------------------------------------------- anchors

/// Read-only: compares a Sepolia block's header hash with NEAR's Ethereum light client, the check
/// near-antibody runs when a program is created. With `forge`, it also shows that a forged header
/// (one bit of the state root flipped) disagrees with the light client, so a program anchored on it
/// would be rejected and its escrow returned.
pub fn anchor_check(ctx: &Ctx, block: u64, forge: bool) -> Result<Value> {
    let lc = ctx.dep.light_client.clone().ok_or_else(|| eyre!("the deployments file names no light client"))?;
    let pool = ctx.eth_pool()?;
    let header = antibody_witness::build::fetch_header(&pool, block)?;
    let real = header.hash_slow();
    let raw = ctx.near.view_raw(&lc, "block_hash_safe", &block.to_le_bytes())?;
    let client = match raw.first() {
        Some(1) if raw.len() == 33 => Some(B256::from_slice(&raw[1..])),
        _ => None,
    };
    let mut out = json!({
        "light_client": lc, "block": block, "header_hash": format!("{real:#x}"),
        "light_client_hash": client.map(|h| format!("{h:#x}")),
        "matches": client == Some(real),
        "verdict": match client {
            Some(h) if h == real => "light-client: create_program would accept this anchor",
            Some(_) => "mismatch: create_program would reject this anchor and return the escrow",
            None => "outside the light-client window: the anchor would be recorded as unconfirmed",
        },
    });
    if forge {
        let mut forged = header.clone();
        forged.state_root.0[31] ^= 1;
        let fh = forged.hash_slow();
        out["forged"] = json!({
            "what": "the same header with one bit of state_root flipped",
            "forged_hash": format!("{fh:#x}"),
            "matches_light_client": client == Some(fh),
            "verdict": if client.is_some() && client != Some(fh) {
                "rejected: a program anchored on the forged header is rejected and its escrow returned"
            } else {
                "not checkable: the block is outside the light-client window"
            },
        });
    }
    Ok(out)
}

// ----------------------------------------------------------------------- rounds

/// Derives the round manifest of a fix from the program's round-0 manifest: each `(address, code
/// file)` replaces that target's runtime code at the same anchor (docs/rounds.md).
pub fn manifest_round(base: &Path, round: u32, overrides: &[String], out: &Path) -> Result<Value> {
    let m = Manifest::parse(&std::fs::read(base)?).map_err(|e| eyre!(e))?;
    let mut code = Vec::new();
    for o in overrides {
        let (addr, file) = o.split_once('=').ok_or_else(|| eyre!("--override takes <address>=<file with runtime code hex>"))?;
        let hexs = std::fs::read_to_string(file)?;
        let bytes = hex::decode(hexs.trim().trim_start_matches("0x"))?;
        code.push((addr.parse::<alloy_primitives::Address>()?, alloy_primitives::Bytes::from(bytes)));
    }
    let r = m.with_overrides(round, &code).map_err(|e| eyre!(e))?;
    let bytes = r.canonical();
    std::fs::write(out, &bytes)?;
    Ok(json!({"manifest": out, "round": round, "bytes": bytes.len(), "sha256": format!("{:#x}", Manifest::hash(&bytes)),
              "overrides": r.overrides.iter().map(|o| json!({"address": o.address, "code_hash": o.code_hash})).collect::<Vec<_>>()}))
}

// -------------------------------------------------------------------- agents

/// Methods an agent's function-call key may call on near-antibody (and FastFS uploads). No
/// withdrawal, transfer or deposit: those need the owner's full-access key and one yoctoNEAR.
pub const AGENT_METHODS: &[&str] = &["commit", "reveal", "retry", "__fastdata_fastfs"];

/// The owner registers its sub-account as an agent: bonds come from the owner's credit, within the
/// budget and the per-commit cap; payouts go to the owner (docs/spec.md §13).
pub fn agent_register(ctx: &Ctx, agent: &str, budget: &str, per_commit_cap: &str) -> Result<Value> {
    let signer = ctx.signer()?;
    ensure_near_credit(ctx, &signer, MIN_NEAR_CREDIT)?;
    let out = ctx.near.function_call(
        &signer,
        &ctx.dep.antibody,
        "register_agent",
        json!({"agent": agent, "bond_budget": parse_tusd(budget)?.to_string(), "per_commit_cap": parse_tusd(per_commit_cap)?.to_string()}),
        30,
        ONE_YOCTO,
    )?;
    ensure_ok(ctx, &out, "register_agent")
}

/// Adds a function-call access key limited to [`AGENT_METHODS`] on near-antibody, with a NEAR gas
/// allowance, to the signer's account (the agent's own sub-account).
pub fn agent_key_add(ctx: &Ctx, allowance_near: f64, out: &Path) -> Result<Value> {
    let owner = ctx.signer()?;
    let agent = Signer::generate(Some(owner.account_id.clone()))?;
    let allowance = (allowance_near * 1e6) as u128 * (ONE_NEAR / 1_000_000);
    let res = ctx.near.send(
        &owner,
        &owner.account_id,
        vec![Action::AddKey {
            public_key: agent.public_key(),
            permission: Permission::FunctionCall {
                allowance: Some(allowance),
                receiver_id: ctx.dep.antibody.clone(),
                method_names: AGENT_METHODS.iter().map(|s| s.to_string()).collect(),
            },
        }],
        "EXECUTED_OPTIMISTIC",
    )?;
    let s = ensure_ok(ctx, &res, "add_key")?;
    write_json(out, &agent.to_file_json())?;
    Ok(json!({"account": owner.account_id, "agent_public_key": agent.public_key_str(), "key_file": out,
              "receiver": ctx.dep.antibody, "methods": AGENT_METHODS, "allowance_near": allowance_near, "tx": s}))
}

pub fn pull(ctx: &Ctx, program: u32, out: &Path) -> Result<Value> {
    let p = ctx.program(program)?;
    let manifest_json = program_manifest(ctx, program)?;
    let m = Manifest::parse(&manifest_json).map_err(|e| eyre!(e))?;
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("manifest.json"), &manifest_json)?;
    std::fs::write(out.join("invariants.hex"), format!("{}\n", m.invariants.runtime_code))?;
    write_json(&out.join("program.json"), &p)?;
    let caps = &m.capabilities;
    let template = json!({
        "senders": 1,
        "endowment_wei": caps.max_endowment_wei,
        "protected": [],
        "calls": [{"sender": 0, "to": null, "value": "0", "data": "0x<creation code of your PoC contract + ABI-encoded constructor args>", "gas_limit": 5_000_000}],
    });
    write_json(&out.join("bundle.template.json"), &template)?;
    let mut readme = format!(
        "# {name}\n\nProgram `{key}` on `{contract}`. Anchor: chain {chain}, block {block} `{hash}` ({anchor}).\n\n",
        name = m.program.name, key = p["key"].as_str().unwrap_or_default(), contract = ctx.dep.antibody,
        chain = m.anchor.chain_id, block = m.anchor.block_number, hash = m.anchor.block_hash, anchor = p["anchor"]
    );
    readme += "## Fork the anchor\n\n```bash\nanvil --fork-url \"$SEPOLIA_RPC\" --fork-block-number ";
    readme += &format!("{} --chain-id {}\n```\n\n", m.anchor.block_number, m.anchor.chain_id);
    readme += "## Targets\n\n| Name | Address |\n|---|---|\n";
    for t in &m.targets {
        readme += &format!("| {} | `{}` |\n", t.name, t.address);
    }
    readme += "\n## Invariants (rules as code)\n\n| Bit | Name | Severity |\n|---|---|---|\n";
    for s in &m.invariants.slots {
        readme += &format!("| {} | {} | {:?} |\n", s.bit, s.name, s.severity);
    }
    readme += &format!(
        "\n## Your PoC runs as\n\n- up to {} fresh senders derived from your secret salt, each credited at most {} wei;\n- at most {} transactions, {} gas each, {} gas in total, in the anchor block's environment;\n- at most {} protocol-entry calls (each one evaluates the invariants).\n\nA submission is valid only if a committed invariant breaks. Profit alone is not a bug, and report text is never read.\n\n",
        caps.max_senders, caps.max_endowment_wei, caps.max_txs, caps.max_gas_per_tx, caps.max_total_gas, caps.max_entry_calls
    );
    readme += "## Check, then submit\n\n```bash\nantibody check --program ";
    readme += &format!("{program} --bundle bundle.json\nantibody submit --program {program} --bundle bundle.json --wait\n```\n");
    std::fs::write(out.join("README.md"), readme)?;
    Ok(json!({"program": program, "out": out, "files": ["manifest.json", "invariants.hex", "program.json", "bundle.template.json", "README.md"]}))
}

// --------------------------------------------------------------------- admin

pub fn parse_near(s: &str) -> Result<u128> {
    let (i, f) = s.split_once('.').unwrap_or((s, ""));
    if f.len() > 24 || i.is_empty() || !i.chars().chain(f.chars()).all(|c| c.is_ascii_digit()) {
        return Err(eyre!("bad NEAR amount {s:?}"));
    }
    Ok(i.parse::<u128>()? * ONE_NEAR + format!("{f:0<24}").parse::<u128>()?)
}

fn parse_pk(s: &str) -> Result<[u8; 32]> {
    bs58::decode(s.trim_start_matches("ed25519:")).into_vec()?.try_into().map_err(|_| eyre!("public key must be 32 bytes"))
}

/// Creates `new_account_id`: as a sub-account of the signer (CreateAccount + Transfer + AddKey), or a
/// top-level `.testnet` name through the `testnet` registrar. A fresh key is written to `key_out`
/// unless `public_key` is given.
pub fn admin_create_account(ctx: &Ctx, new_account_id: &str, amount_near: &str, public_key: Option<String>, key_out: Option<&Path>) -> Result<Value> {
    let signer = ctx.signer()?;
    if ctx.near.account(new_account_id)?.is_some() {
        return Err(eyre!("{new_account_id} already exists"));
    }
    let (pk, key_file) = match (public_key, key_out) {
        (Some(p), _) => (parse_pk(&p)?, None),
        (None, Some(out)) => {
            let s = Signer::generate(Some(new_account_id.to_string()))?;
            write_json(out, &s.to_file_json())?;
            (s.public_key(), Some(out.to_path_buf()))
        }
        (None, None) => return Err(eyre!("pass --public-key or --key-out")),
    };
    let amount = parse_near(amount_near)?;
    let out = if new_account_id.ends_with(&format!(".{}", signer.account_id)) {
        ctx.near.send(
            &signer,
            new_account_id,
            vec![Action::CreateAccount, Action::Transfer(amount), Action::AddKey { public_key: pk, permission: Permission::FullAccess }],
            "EXECUTED_OPTIMISTIC",
        )?
    } else {
        let registrar = new_account_id.rsplit('.').next().ok_or_else(|| eyre!("bad account id"))?;
        let pk_str = format!("ed25519:{}", bs58::encode(pk).into_string());
        ctx.near.function_call(&signer, registrar, "create_account", json!({"new_account_id": new_account_id, "new_public_key": pk_str}), 100, amount)?
    };
    let s = ensure_ok(ctx, &out, "create_account")?;
    Ok(json!({"account_id": new_account_id, "key_file": key_file, "tx": s}))
}

/// Deploys `wasm` to the signer's own account and optionally calls an init method, in one transaction.
pub fn admin_deploy(ctx: &Ctx, wasm: &Path, init_method: Option<String>, init_args: Option<String>) -> Result<Value> {
    let signer = ctx.signer()?;
    let code = std::fs::read(wasm)?;
    let mut actions = vec![Action::DeployContract(code.clone())];
    if let Some(m) = init_method {
        let args = init_args.unwrap_or_else(|| "{}".into());
        serde_json::from_str::<Value>(&args).map_err(|e| eyre!("init args: {e}"))?;
        actions.push(Action::FunctionCall { method: m, args: args.into_bytes(), gas: 100 * TGAS, deposit: 0 });
    }
    let out = ctx.near.send(&signer, &signer.account_id, actions, "EXECUTED_OPTIMISTIC")?;
    let s = ensure_ok(ctx, &out, "deploy")?;
    Ok(json!({"account_id": signer.account_id, "wasm": wasm, "bytes": code.len(), "sha256": hex::encode(sha256(&code)), "tx": s}))
}

pub fn admin_call(ctx: &Ctx, contract: &str, method: &str, args: &str, deposit_yocto: u128, gas_tgas: u64) -> Result<Value> {
    let a: Value = serde_json::from_str(args).map_err(|e| eyre!("args: {e}"))?;
    let out = ctx.near.function_call(&ctx.signer()?, contract, method, a, gas_tgas, deposit_yocto)?;
    let s = ensure_ok(ctx, &out, method)?;
    Ok(json!({"result": near::return_value(&out), "tx": s}))
}

pub fn admin_send_near(ctx: &Ctx, to: &str, amount_near: &str) -> Result<Value> {
    let out = ctx.near.send(&ctx.signer()?, to, vec![Action::Transfer(parse_near(amount_near)?)], "EXECUTED_OPTIMISTIC")?;
    ensure_ok(ctx, &out, "transfer")
}

pub fn tx(ctx: &Ctx, hash: &str, signer: &str) -> Result<Value> {
    let out = ctx.near.tx_status(hash, signer)?;
    Ok(json!({"explorer": ctx.near.explorer_tx(hash), "failures": near::failures(&out), "events": near::events(&out),
              "status": out["status"]}))
}

//! Command implementations. Each returns JSON (explorer links included); `main` prints it and the
//! MCP server returns it to agents.

use std::path::Path;

use alloy_primitives::B256;
use antibody_verifier::{
    facts::{sha256, ProgramReceipt},
    manifest::Manifest,
    run::{run_program, ProgramContext},
    sealed, Envelope, Payload,
};
use antibody_witness::build::{parse_bundle, BundleFile};
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{
    blob,
    ctx::Ctx,
    near::{self, Action, Permission, Signer, ONE_NEAR, ONE_YOCTO, TGAS},
};

/// Gas for `submit` / `submit_known`: OutLayer call (100) + verdict callback (160, incl. the guardian pause) + margin.
pub const SUBMIT_TGAS: u64 = 290;
pub const TUSD_DECIMALS: u32 = 6;

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

fn u128_of(v: &Value) -> u128 {
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
    // `__fastdata_fastfs` uploads fail by design; everything else must succeed.
    if !f.is_empty() {
        return Err(eyre!("{what} failed: {} ({})", f.join("; "), s["explorer"].as_str().unwrap_or_default()));
    }
    Ok(s)
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

fn project_secret(path: &Path) -> Result<[u8; 32]> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    hex::decode(v["secret_key"].as_str().ok_or_else(|| eyre!("no secret_key in {}", path.display()))?)?
        .try_into()
        .map_err(|_| eyre!("x25519 secret must be 32 bytes"))
}

fn project_public(path: &Path) -> Result<String> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    Ok(v["public_key"].as_str().ok_or_else(|| eyre!("no public_key in {}", path.display()))?.to_string())
}

// ---------------------------------------------------------------------- program

pub fn manifest_compile(toml: &Path, out: &Path) -> Result<Value> {
    let (bytes, hash) = antibody_witness::program::compile_manifest(&std::fs::read_to_string(toml)?)?;
    std::fs::write(out, &bytes)?;
    Ok(json!({"manifest": out, "bytes": bytes.len(), "sha256": format!("{hash:#x}")}))
}

/// The manifest hash committed for `round` (0: the program's own manifest).
pub fn round_manifest_hash(p: &Value, round: u64) -> String {
    match round {
        0 => p["manifest_hash"].as_str().unwrap_or_default().to_string(),
        r => p["rounds"][(r - 1) as usize]["manifest_hash"].as_str().unwrap_or_default().to_string(),
    }
}

/// Fetches the manifest of the program's current round (new commits verify against it) and
/// checks it against the committed hash.
pub fn program_manifest(ctx: &Ctx, p: &Value) -> Result<Vec<u8>> {
    program_manifest_at(ctx, p, p["round"].as_u64().unwrap_or(0))
}

pub fn program_manifest_at(ctx: &Ctx, p: &Value, round: u64) -> Result<Vec<u8>> {
    let url = match round {
        0 => p["manifest_url"].as_str().unwrap_or_default().to_string(),
        r => p["rounds"][(r - 1) as usize]["manifest_url"].as_str().unwrap_or_default().to_string(),
    };
    let expected = round_manifest_hash(p, round);
    let bytes = blob::fetch(&ctx.near, &url)?;
    let h = hex::encode(sha256(&bytes));
    if h != expected {
        return Err(eyre!("round {round} manifest at {url} hashes to {h}, not the committed {expected}"));
    }
    Ok(bytes)
}

pub fn program_create(ctx: &Ctx, manifest: &Path, escrow: &str, project_key: &Path, guardian: Option<String>, manifest_url: Option<String>) -> Result<Value> {
    let signer = ctx.signer()?;
    let bytes = std::fs::read(manifest)?;
    let m = Manifest::parse(&bytes).map_err(|e| eyre!(e))?;
    if m.program.project != signer.account_id {
        return Err(eyre!("manifest.program.project is {}, but you sign as {}", m.program.project, signer.account_id));
    }
    let manifest_str = String::from_utf8(bytes.clone())?;
    let upload = match manifest_url {
        Some(u) => json!({"location": u}),
        None => blob::upload(ctx, &signer, &format!("antibody/manifests/{}.json", hex::encode(sha256(&bytes))), &bytes)?,
    };
    let msg = json!({"create_program": {"manifest": manifest_str, "manifest_url": upload["location"],
                     "project_pubkey": project_public(project_key)?, "guardian": guardian}});
    let out = ft_transfer_call(ctx, &signer, parse_tusd(escrow)?, msg)?;
    let s = ensure_ok(ctx, &out, "create_program")?;
    let created = s["events"].as_array().into_iter().flatten().find(|e| e["event"] == "program_created").cloned();
    let anchor = s["events"].as_array().into_iter().flatten().find(|e| e["event"] == "anchor_checked" || e["event"] == "program_rejected").cloned();
    Ok(json!({"program": created.map(|e| e["data"][0].clone()), "anchor": anchor.map(|e| e["data"][0].clone()),
              "manifest_upload": upload, "manifest_sha256": hex::encode(sha256(&bytes)), "tx": s}))
}

pub fn program_open(ctx: &Ctx, program: u32) -> Result<Value> {
    let out = ctx.near.function_call(&ctx.signer()?, &ctx.dep.antibody, "open_program", json!({"program_id": program}), 30, 0)?;
    ensure_ok(ctx, &out, "open_program")
}

pub fn program_simple(ctx: &Ctx, method: &str, args: Value, tgas: u64) -> Result<Value> {
    let out = ctx.near.function_call(&ctx.signer()?, &ctx.dep.antibody, method, args, tgas, 0)?;
    ensure_ok(ctx, &out, method)
}

pub fn programs(ctx: &Ctx, from: u32, limit: u32) -> Result<Value> {
    let ps = ctx.view("list_programs", json!({"from": from, "limit": limit}))?;
    let rows: Vec<Value> = ps
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            json!({"id": p["id"], "name": p["name"], "project": p["project"], "status": p["status"],
                   "anchor": p["anchor"], "anchor_trust": p["anchor_trust"], "escrow_tusd": fmt_tusd(u128_of(&p["escrow"])),
                   "paid_tusd": fmt_tusd(u128_of(&p["paid"])), "commits": p["commits"].as_array().map(|c| c.len())})
        })
        .collect();
    Ok(json!({"contract": ctx.dep.antibody, "programs": rows, "stats": ctx.view("get_stats", json!({}))?}))
}

pub fn program_show(ctx: &Ctx, program: u32) -> Result<Value> {
    let p = ctx.program(program)?;
    let keys = ctx.view("get_keys", json!({"program_id": program}))?;
    let commits = ctx.view("list_commits", json!({"program_id": program, "from": 0, "limit": 100}))?;
    let rows: Vec<Value> = commits
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| json!({"id": c["id"], "hunter": c["hunter"], "height": c["height"], "known_issue": c["known_issue"],
                        "status": c["status"], "kind": c["verdict"]["kind"], "outcome": c["outcome"]}))
        .collect();
    Ok(json!({"program": p, "claimed_keys": keys, "commits": rows}))
}

// ------------------------------------------------------------------ witnesses

/// Builds a program witness for `bundle_file` from the Ethereum RPC (state at the anchor).
pub fn build_witness(ctx: &Ctx, manifest_json: &[u8], bundle_file: &Path, salt: Option<B256>) -> Result<(Vec<u8>, Value, ProgramReceipt, B256)> {
    let m = Manifest::parse(manifest_json).map_err(|e| eyre!(e))?;
    let bf: BundleFile = serde_json::from_str(&std::fs::read_to_string(bundle_file)?)?;
    let salt = match salt {
        Some(s) => s,
        None => B256::from(blob::random32()?),
    };
    let bundle = parse_bundle(&bf, salt)?;
    let pool = ctx.eth_pool()?;
    let (bytes, meta, receipt) =
        antibody_witness::program::build_program(&pool, manifest_json, &m.invariants.runtime_code, bundle, 1)?;
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
        (Some(p), None) => program_manifest(ctx, &ctx.program(p)?)?,
        _ => return Err(eyre!("pass --program or --manifest")),
    };
    let (bytes, meta, receipt, salt) = build_witness(ctx, &manifest_json, bundle, salt)?;
    Ok(json!({"verdict": verdict_view(&receipt), "witness_bytes": bytes.len(), "fixpoint_passes": meta["fixpoint_passes"],
              "salt_note": "a fresh random salt is drawn per submission unless --salt is given", "salt": format!("{salt:#x}"),
              "receipt": receipt}))
}

// --------------------------------------------------------------------- hunting

pub fn deposit(ctx: &Ctx, amount: &str) -> Result<Value> {
    let out = ft_transfer_call(ctx, &ctx.signer()?, parse_tusd(amount)?, json!({"deposit": {}}))?;
    ensure_ok(ctx, &out, "deposit")
}

pub fn faucet(ctx: &Ctx) -> Result<Value> {
    let out = ctx.near.function_call(&ctx.signer()?, &ctx.dep.tusd, "faucet", json!({}), 30, 0)?;
    ensure_ok(ctx, &out, "faucet")
}

pub fn withdraw(ctx: &Ctx, amount: &str) -> Result<Value> {
    let out = ctx.near.function_call(&ctx.signer()?, &ctx.dep.antibody, "withdraw", json!({"amount": parse_tusd(amount)?.to_string()}), 60, 0)?;
    ensure_ok(ctx, &out, "withdraw")
}

pub fn commitment(program_key: &str, hunter: &str, sha: &str) -> String {
    hex::encode(sha256(format!("antibody/commit/v1|{program_key}|{hunter}|{sha}").as_bytes()))
}

/// Builds the witness, seals it, and commits (the blob stays private until `reveal`).
pub fn commit(ctx: &Ctx, program: u32, bundle: &Path, salt: Option<B256>, report: Option<&Path>) -> Result<Value> {
    let signer = ctx.signer()?;
    let p = ctx.program(program)?;
    let manifest_json = program_manifest(ctx, &p)?;
    let (envelope, _meta, local, salt) = build_witness(ctx, &manifest_json, bundle, salt)?;
    let key = p["key"].as_str().unwrap_or_default().to_string();
    let pubkey = p["verifier"]["pubkey"].as_str().ok_or_else(|| eyre!("program has no verifier pubkey"))?;
    let report = match report {
        Some(r) => std::fs::read(r)?,
        None => Vec::new(),
    };
    let s = blob::seal(pubkey, &key, &signer.account_id, envelope, report)?;
    let ch = commitment(&key, &signer.account_id, &s.sha256);
    let out = ctx.near.function_call(&signer, &ctx.dep.antibody, "commit", json!({"program_id": program, "commit_hash": ch}), 30, 0)?;
    let summary = ensure_ok(ctx, &out, "commit")?;
    let commit_id = near::return_value(&out).and_then(|v| v.as_u64()).ok_or_else(|| eyre!("commit returned no id"))?;
    let blob_path = ctx.state_dir.join("blobs").join(format!("{}.bin", s.sha256));
    std::fs::create_dir_all(blob_path.parent().unwrap())?;
    std::fs::write(&blob_path, &s.blob)?;
    let record = json!({"program": program, "program_key": key, "commit_id": commit_id, "hunter": signer.account_id,
                        "commit_hash": ch, "sha256": s.sha256, "wrap_v": s.wrap_v, "blob": blob_path,
                        "content_key": hex::encode(s.content_key), "salt": format!("{salt:#x}"),
                        "local_verdict": verdict_view(&local), "commit_tx": summary["tx"]});
    write_json(&ctx.record_path(program, commit_id), &record)?;
    Ok(json!({"commit_id": commit_id, "commit_hash": ch, "local_verdict": verdict_view(&local),
              "record": ctx.record_path(program, commit_id), "tx": summary,
              "note": "the content key and salt are in the local record (.antibody/): keep it private until disclosure"}))
}

/// Uploads the sealed blob and reveals it: the contract asks OutLayer to verify it.
pub fn reveal(ctx: &Ctx, program: u32, commit_id: u64, wait: bool) -> Result<Value> {
    let signer = ctx.signer()?;
    let rec: Value = serde_json::from_str(&std::fs::read_to_string(ctx.record_path(program, commit_id))?)?;
    let blob_bytes = std::fs::read(rec["blob"].as_str().unwrap_or_default())?;
    let sha = rec["sha256"].as_str().unwrap_or_default();
    let up = blob::upload(ctx, &signer, &format!("antibody/sealed/{sha}.bin"), &blob_bytes)?;
    let sealed_ref = json!({"url": up["location"], "sha256": sha, "wrap_v": rec["wrap_v"]});
    let out = ctx.near.function_call(&signer, &ctx.dep.antibody, "submit", json!({"commit_id": commit_id, "sealed": sealed_ref}), SUBMIT_TGAS, 0)?;
    let s = ensure_ok(ctx, &out, "submit")?;
    let mut res = json!({"commit_id": commit_id, "upload": up, "submit": s});
    if wait {
        res["result"] = wait_verdict(ctx, commit_id, 240)?;
    }
    Ok(res)
}

/// One step for agents: build, seal, commit, upload, reveal, and wait for the verdict.
pub fn submit(ctx: &Ctx, program: u32, bundle: &Path, salt: Option<B256>, report: Option<&Path>, wait: bool) -> Result<Value> {
    let c = commit(ctx, program, bundle, salt, report)?;
    let id = c["commit_id"].as_u64().unwrap();
    let r = reveal(ctx, program, id, wait)?;
    Ok(json!({"commit": c, "reveal": r}))
}

pub fn submit_known(ctx: &Ctx, program: u32, bundle: &Path, salt: Option<B256>, report: Option<&Path>, wait: bool) -> Result<Value> {
    let signer = ctx.signer()?;
    let p = ctx.program(program)?;
    let manifest_json = program_manifest(ctx, &p)?;
    let (envelope, _, local, _) = build_witness(ctx, &manifest_json, bundle, salt)?;
    let key = p["key"].as_str().unwrap_or_default().to_string();
    let pubkey = p["verifier"]["pubkey"].as_str().ok_or_else(|| eyre!("program has no verifier pubkey"))?;
    let report = match report {
        Some(r) => std::fs::read(r)?,
        None => Vec::new(),
    };
    let s = blob::seal(pubkey, &key, &signer.account_id, envelope, report)?;
    let up = blob::upload(ctx, &signer, &format!("antibody/sealed/{}.bin", s.sha256), &s.blob)?;
    let sealed_ref = json!({"url": up["location"], "sha256": s.sha256, "wrap_v": s.wrap_v});
    let out = ctx.near.function_call(&signer, &ctx.dep.antibody, "submit_known", json!({"program_id": program, "sealed": sealed_ref}), SUBMIT_TGAS, 0)?;
    let summary = ensure_ok(ctx, &out, "submit_known")?;
    let id = summary["events"].as_array().into_iter().flatten().find(|e| e["event"] == "known_issue_submitted")
        .and_then(|e| e["data"][0]["commit_id"].as_u64()).ok_or_else(|| eyre!("no known_issue_submitted event"))?;
    write_json(&ctx.record_path(program, id), &json!({"program": program, "program_key": key, "commit_id": id, "hunter": signer.account_id,
        "sha256": s.sha256, "wrap_v": s.wrap_v, "content_key": hex::encode(s.content_key), "known_issue": true}))?;
    let mut res = json!({"commit_id": id, "local_verdict": verdict_view(&local), "upload": up, "tx": summary});
    if wait {
        res["result"] = wait_verdict(ctx, id, 240)?;
    }
    Ok(res)
}

pub fn wait_verdict(ctx: &Ctx, commit_id: u64, timeout_s: u64) -> Result<Value> {
    let t0 = std::time::Instant::now();
    loop {
        let c = ctx.commit(commit_id)?;
        let st = c["status"].as_str().unwrap_or_default().to_string();
        if st != "Verifying" {
            return Ok(json!({"status": st, "verdict": c["verdict"], "outcome": c["outcome"], "seconds": t0.elapsed().as_secs()}));
        }
        if t0.elapsed().as_secs() > timeout_s {
            return Ok(json!({"status": "Verifying", "note": "still verifying; check `antibody status --commit`"}));
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

pub fn status(ctx: &Ctx, commit: Option<u64>, program: Option<u32>, account: Option<String>) -> Result<Value> {
    let mut out = json!({});
    if let Some(c) = commit {
        let cv = ctx.commit(c)?;
        out["commit"] = cv.clone();
        if matches!(cv["outcome"].as_object().map(|o| o.contains_key("Duplicate") || o.contains_key("Known")), Some(true)) {
            out["duplicate_proof"] = ctx.view("get_duplicate_proof", json!({"commit_id": c}))?;
        }
    }
    if let Some(p) = program {
        out["program"] = program_show(ctx, p)?;
    }
    let acct = account.or_else(|| ctx.account.clone());
    if let Some(a) = acct {
        out["account"] = json!({"id": a, "internal_balance_tusd": fmt_tusd(u128_of(&ctx.view("get_balance", json!({"account_id": a}))?)),
                                "reputation": ctx.view("get_reputation", json!({"account_id": a}))?,
                                "tusd_balance": fmt_tusd(u128_of(&ctx.near.view(&ctx.dep.tusd, "ft_balance_of", json!({"account_id": a}))?))});
    }
    Ok(out)
}

pub fn leaderboard(ctx: &Ctx, limit: u32) -> Result<Value> {
    ctx.view("leaderboard", json!({"limit": limit}))
}

pub fn top_up(ctx: &Ctx, commit_id: u64, amount: &str, note: Option<String>) -> Result<Value> {
    let out = ft_transfer_call(ctx, &ctx.signer()?, parse_tusd(amount)?, json!({"top_up": {"commit_id": commit_id, "note": note}}))?;
    ensure_ok(ctx, &out, "top_up")
}

pub fn fund(ctx: &Ctx, program: u32, amount: &str) -> Result<Value> {
    let out = ft_transfer_call(ctx, &ctx.signer()?, parse_tusd(amount)?, json!({"fund_program": {"program_id": program}}))?;
    ensure_ok(ctx, &out, "fund_program")
}

// ------------------------------------------------------------------------- init

const INIT_TOML: &str = r#"# Antibody program manifest: your bounty terms as code. Its sha256 is what you commit on NEAR.
# Fill every <...>, then:  antibody manifest antibody.toml --out manifest.json
# A complete example: fixtures/acme-local/antibody.toml in the Antibody repository.
schema = "antibody.manifest.v1"

[program]
name = "<program name>"
project = "<PROJECT>"
description = "<one line: what is in scope>"

[anchor]
# A finalized block of your staging deployment. Create the program within about 7 days of it so that
# NEAR's Ethereum light client can confirm its hash (otherwise it is labelled project-attested).
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
# Reserved address where the verifier injects your invariants; keep it as is.
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

[payout]
# Raw units of the payout token (tUSD has 6 decimals: 10000000000 = 10,000 tUSD).
token = "tusd.antibody-hq.testnet"
decimals = 6
critical = "10000000000"
high = "3000000000"
medium = "1000000000"
low = "0"
impact_bps = 0
impact_cap = "0"

[rules]
bond = "25000000"
reveal_window_blocks = 300
disclosure_after_secs = 86400
deadline_unix = <DEADLINE>
instant_payout_cap = "5000000000"
challenge_delay_secs = 259200
"#;

/// Scaffolds a program workspace: an annotated antibody.toml, the project's X25519 key (findings are
/// re-encrypted to it), and the next steps.
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
         5. Declare known issues (`program known`), then `program open`.\n\n\
         Keep `project.x25519.json` secret: it decrypts every valid finding.\n"
    );
    std::fs::write(out.join("NEXT_STEPS.md"), steps)?;
    Ok(json!({"out": out, "manifest_template": toml_path, "project_key": k, "deadline_unix": deadline,
              "next": out.join("NEXT_STEPS.md")}))
}

// ---------------------------------------------------------------------- anchors

/// Read-only: compares a Sepolia block's header hash with NEAR's Ethereum light client, the check
/// near-antibody runs when a program is created. With `forge`, it also shows that a forged header
/// (one bit of the state root flipped) disagrees with the light client, so a program anchored on it
/// would be rejected and its escrow refunded (`on_anchor_checked`).
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
            Some(_) => "mismatch: create_program would reject this anchor and refund the escrow",
            None => "outside the light-client window: the program would be labelled project-attested",
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
                "rejected: a program anchored on the forged header is refunded (near-antibody on_anchor_checked)"
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

/// The project commits its fix: uploads the round manifest and proposes the round on NEAR.
pub fn round_propose(ctx: &Ctx, program: u32, manifest: &Path) -> Result<Value> {
    let signer = ctx.signer()?;
    let p = ctx.program(program)?;
    let base = program_manifest_at(ctx, &p, 0)?;
    let bytes = std::fs::read(manifest)?;
    let m = Manifest::parse(&bytes).map_err(|e| eyre!(e))?;
    let up = blob::upload(ctx, &signer, &format!("antibody/manifests/{}.json", hex::encode(sha256(&bytes))), &bytes)?;
    let out = ctx.near.function_call(
        &signer,
        &ctx.dep.antibody,
        "propose_round",
        json!({"program_id": program, "base_manifest": String::from_utf8(base)?, "manifest": String::from_utf8(bytes.clone())?,
               "manifest_url": up["location"]}),
        150,
        0,
    )?;
    let s = ensure_ok(ctx, &out, "propose_round")?;
    Ok(json!({"program": program, "round": m.round, "manifest_sha256": hex::encode(sha256(&bytes)), "upload": up, "tx": s}))
}

/// Re-runs a claimed finding against the proposed round's fix: recovers the PoC (the project from
/// its key wrap, the finder from the local record, anyone after disclosure), builds a fresh witness
/// of the same bundle at the round manifest, seals it to the verifier and asks near-antibody to run
/// it in the TEE.
pub fn round_rerun(ctx: &Ctx, commit_id: u64, project_key: Option<&Path>, wait: bool) -> Result<Value> {
    let signer = ctx.signer()?;
    let c = ctx.commit(commit_id)?;
    let p = ctx.program(c["program"].as_u64().unwrap_or_default() as u32)?;
    let key = p["key"].as_str().unwrap_or_default().to_string();
    let round = p["round"].as_u64().unwrap_or(0);
    if round == 0 || round != p["open_round"].as_u64().unwrap_or(0) + 1 {
        return Err(eyre!("no proposed round is waiting for re-runs"));
    }
    let (k, source) = match project_key {
        Some(pk) => {
            let wrap = hex::decode(c["verdict"]["project_wrap"].as_str().ok_or_else(|| eyre!("commit {commit_id} has no project wrap"))?)?;
            let sk = x25519_dalek::StaticSecret::from(project_secret(pk)?);
            (sealed::project_unwrap(&wrap, &sk, &key).map_err(|e| eyre!("{e}"))?, "project wrap")
        }
        None => content_key_for(ctx, &p, commit_id)?,
    };
    let blob_bytes = blob::fetch(&ctx.near, c["sealed"]["url"].as_str().unwrap_or_default())?;
    let payload = sealed::open_with_key(&blob_bytes, &k, &key).map_err(|e| eyre!("{e}"))?;
    let env = Envelope::decode(&payload.envelope).map_err(|e| eyre!(e))?;
    let Payload::Program(w) = env.payload else { return Err(eyre!("not a program witness")) };
    let manifest_json = program_manifest_at(ctx, &p, round)?;
    let m = Manifest::parse(&manifest_json).map_err(|e| eyre!(e))?;
    let pool = ctx.eth_pool()?;
    let (envelope, _meta, local) = antibody_witness::program::build_program(&pool, &manifest_json, &m.invariants.runtime_code, w.bundle, 1)?;
    let pubkey = p["verifier"]["pubkey"].as_str().ok_or_else(|| eyre!("program has no verifier pubkey"))?;
    let hunter = c["hunter"].as_str().unwrap_or_default();
    let s = blob::seal(pubkey, &key, hunter, envelope, Vec::new())?;
    let up = blob::upload(ctx, &signer, &format!("antibody/reruns/{}.bin", s.sha256), &s.blob)?;
    let sealed_ref = json!({"url": up["location"], "sha256": s.sha256, "wrap_v": s.wrap_v});
    let out = ctx.near.function_call(&signer, &ctx.dep.antibody, "request_rerun", json!({"commit_id": commit_id, "sealed": sealed_ref}), SUBMIT_TGAS, 0)?;
    let tx = ensure_ok(ctx, &out, "request_rerun")?;
    let mut res = json!({"commit_id": commit_id, "round": round, "key_source": source, "local_verdict": verdict_view(&local),
                         "upload": up, "tx": tx});
    if wait {
        res["result"] = wait_rerun(ctx, commit_id, round, 240)?;
    }
    Ok(res)
}

pub fn wait_rerun(ctx: &Ctx, commit_id: u64, round: u64, timeout_s: u64) -> Result<Value> {
    let t = std::time::Instant::now();
    loop {
        let c = ctx.commit(commit_id)?;
        if let Some(r) = c["reruns"].as_array().into_iter().flatten().find(|r| r["round"].as_u64() == Some(round)) {
            if r["status"] != json!("Requested") {
                return Ok(r.clone());
            }
        }
        if t.elapsed().as_secs() > timeout_s {
            return Err(eyre!("no re-run result for commit {commit_id} after {timeout_s}s"));
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// Opens the proposed round: released keys become claimable again; the rest carry over.
pub fn round_open(ctx: &Ctx, program: u32) -> Result<Value> {
    let out = ctx.near.function_call(&ctx.signer()?, &ctx.dep.antibody, "open_round", json!({"program_id": program}), 200, 0)?;
    let s = ensure_ok(ctx, &out, "open_round")?;
    let p = ctx.program(program)?;
    let keys = ctx.view("get_keys", json!({"program_id": program}))?;
    Ok(json!({"program": program, "open_round": p["open_round"], "round": p["rounds"][p["open_round"].as_u64().unwrap_or(1) as usize - 1],
              "keys": keys, "tx": s}))
}

// --------------------------------------------------------------- verification

fn content_key_for(ctx: &Ctx, program: &Value, commit_id: u64) -> Result<([u8; 32], &'static str)> {
    if let Some(k) = program["disclosure"]["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|k| k["commit_id"].as_u64() == Some(commit_id) && k["round"].is_null())
    {
        let k: [u8; 32] = hex::decode(k["content_key"].as_str().unwrap_or_default())?.try_into().map_err(|_| eyre!("bad key"))?;
        return Ok((k, "disclosure"));
    }
    let id = program["id"].as_u64().unwrap_or_default() as u32;
    if let Ok(s) = std::fs::read_to_string(ctx.record_path(id, commit_id)) {
        let rec: Value = serde_json::from_str(&s)?;
        let k: [u8; 32] = hex::decode(rec["content_key"].as_str().unwrap_or_default())?.try_into().map_err(|_| eyre!("bad key"))?;
        return Ok((k, "local record"));
    }
    Err(eyre!("commit {commit_id} is still sealed (no disclosure yet and no local record)"))
}

/// Re-runs one commit's verification locally from its sealed blob and compares with the chain.
fn reverify(ctx: &Ctx, program: &Value, c: &Value, k: &[u8; 32]) -> Result<Value> {
    let id = c["id"].as_u64().unwrap_or_default();
    let key = program["key"].as_str().unwrap_or_default();
    let url = c["sealed"]["url"].as_str().ok_or_else(|| eyre!("commit {id} has no revealed blob"))?;
    let blob_bytes = blob::fetch(&ctx.near, url)?;
    let sha_ok = hex::encode(sha256(&blob_bytes)) == c["sealed"]["sha256"].as_str().unwrap_or_default();
    let payload = sealed::open_with_key(&blob_bytes, k, key).map_err(|e| eyre!("{e}"))?;
    let env = Envelope::decode(&payload.envelope).map_err(|e| eyre!(e))?;
    let Payload::Program(w) = &env.payload else { return Err(eyre!("not a program witness")) };
    let mh = B256::from_slice(&hex::decode(round_manifest_hash(program, c["round"].as_u64().unwrap_or(0)))?);
    let receipt = run_program(&env, w, sha256(&payload.envelope), &ProgramContext { program_id: key.into(), commit_id: id, expected_manifest_hash: Some(mh) });
    let rh = hex::encode(receipt.hash());
    let onchain_rh = c["verdict"]["receipt_hash"].as_str().unwrap_or_default();
    let kp = program["disclosure"]["k_program"].as_str().and_then(|s| hex::decode(s).ok()).and_then(|b| <[u8; 32]>::try_from(b).ok());
    let tags_match = kp.map(|kp| {
        let f = receipt.facts(Some(&kp), None, None);
        let onchain: Vec<(u64, String)> = c["verdict"]["keys"].as_array().into_iter().flatten()
            .map(|k| (k["slot"].as_u64().unwrap_or(0), k["tag"].as_str().unwrap_or_default().to_string())).collect();
        let mine: Vec<(u64, String)> = f.fp_tags.iter().map(|t| (t.slot as u64, t.tag.clone())).collect();
        mine == onchain
    });
    Ok(json!({"commit_id": id, "hunter": payload.hunter, "blob_sha256_matches": sha_ok, "receipt_hash": rh,
              "receipt_hash_matches_chain": rh == onchain_rh, "fp_tags_match_chain": tags_match,
              "verdict": verdict_view(&receipt), "report_bytes": payload.report.len(), "committed_at_height": c["height"]}))
}

/// After disclosure (or with your own local record): re-verify a commit and, for a duplicate or a
/// known issue, the earlier claim too — "same rule, same function, committed earlier".
pub fn verify(ctx: &Ctx, commit_id: u64) -> Result<Value> {
    let c = ctx.commit(commit_id)?;
    let p = ctx.program(c["program"].as_u64().unwrap_or_default() as u32)?;
    let (k, source) = content_key_for(ctx, &p, commit_id)?;
    let mine = reverify(ctx, &p, &c, &k)?;
    let mut out = json!({"commit": mine, "key_source": source, "outcome": c["outcome"]});
    let earlier = c["outcome"]["Duplicate"]["of"].as_u64().or(c["outcome"]["Known"]["of"].as_u64());
    if let Some(e) = earlier {
        let ec = ctx.commit(e)?;
        let (ek, esrc) = content_key_for(ctx, &p, e)?;
        let theirs = reverify(ctx, &p, &ec, &ek)?;
        let same_keys = c["verdict"]["keys"] == ec["verdict"]["keys"];
        out["earlier"] = theirs;
        out["earlier_key_source"] = esrc.into();
        out["same_rule_same_function"] = same_keys.into();
        out["committed_earlier"] = (ec["height"].as_u64() <= c["height"].as_u64() && e < commit_id).into();
    }
    Ok(out)
}

/// The project reads a valid finding: unwrap `project_wrap`, decrypt, re-run, write the PoC out.
pub fn decrypt(ctx: &Ctx, commit_id: u64, project_key: &Path, out_dir: &Path) -> Result<Value> {
    let c = ctx.commit(commit_id)?;
    let p = ctx.program(c["program"].as_u64().unwrap_or_default() as u32)?;
    let wrap = hex::decode(c["verdict"]["project_wrap"].as_str().ok_or_else(|| eyre!("no project wrap: only Violations are readable before disclosure"))?)?;
    let sk = x25519_dalek::StaticSecret::from(project_secret(project_key)?);
    let key = p["key"].as_str().unwrap_or_default();
    let k = sealed::project_unwrap(&wrap, &sk, key).map_err(|e| eyre!("{e}"))?;
    let blob_bytes = blob::fetch(&ctx.near, c["sealed"]["url"].as_str().unwrap_or_default())?;
    let payload = sealed::open_with_key(&blob_bytes, &k, key).map_err(|e| eyre!("{e}"))?;
    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join("witness.bin"), &payload.envelope)?;
    std::fs::write(out_dir.join("report.md"), &payload.report)?;
    let r = reverify(ctx, &p, &c, &k)?;
    write_json(&out_dir.join("reverify.json"), &r)?;
    Ok(json!({"commit_id": commit_id, "hunter": payload.hunter, "out": out_dir, "reverify": r}))
}

// -------------------------------------------------------------------- agents

pub const AGENT_METHODS: &[&str] = &["commit", "submit", "__fastdata_fastfs"];

/// Adds a function-call access key that can only commit/submit (and upload blobs) on near-antibody,
/// with a NEAR gas allowance. It cannot attach deposits or withdraw: bonds come from the owner's
/// internal balance and payouts go to the owner account.
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
    Ok(json!({"owner": owner.account_id, "agent_public_key": agent.public_key_str(), "key_file": out,
              "receiver": ctx.dep.antibody, "methods": AGENT_METHODS, "allowance_near": allowance_near, "tx": s}))
}

pub fn pull(ctx: &Ctx, program: u32, out: &Path) -> Result<Value> {
    let p = ctx.program(program)?;
    let manifest_json = program_manifest(ctx, &p)?;
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
        "# {name}\n\nProgram `{key}` on `{contract}`. Anchor: chain {chain}, block {block} `{hash}` ({trust}).\n\n",
        name = m.program.name, key = p["key"].as_str().unwrap_or_default(), contract = ctx.dep.antibody,
        chain = m.anchor.chain_id, block = m.anchor.block_number, hash = m.anchor.block_hash, trust = p["anchor_trust"]
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

fn parse_near(s: &str) -> Result<u128> {
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

/// Creates the verifier seed as a `PROTECTED_` secret inside OutLayer's keystore (generated in its TEE;
/// nobody sees the value) and stores the row so that only the verifier build `wasm_hash`, called by
/// `contract`, can read it: `And[WasmHash{h}, Predecessor{Whitelist[contract]}]`.
pub fn admin_outlayer_secret(ctx: &Ctx, api: &str, outlayer: &str, wasm_hash: &str, profile: &str, contract: &str) -> Result<Value> {
    let signer = ctx.signer()?;
    let body = json!({"accessor": {"type": "WasmHash", "hash": wasm_hash}, "owner": signer.account_id, "profile": profile,
                      "encrypted_secrets_base64": null,
                      "new_secrets": [{"name": "PROTECTED_ANTIBODY_SEED", "generation_type": "hex32"}]});
    let r = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?
        .post(format!("{api}/secrets/add_generated_secret")).json(&body).send()?;
    let status = r.status();
    let v: Value = r.json().unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(eyre!("add_generated_secret: HTTP {status}: {v}"));
    }
    let ct = v["encrypted_data_base64"].as_str().ok_or_else(|| eyre!("add_generated_secret: no ciphertext in the response"))?.to_string();
    let access = json!({"Logic": {"operator": "And", "conditions": [
        {"WasmHash": {"hash": wasm_hash}},
        {"Predecessor": {"condition": {"Whitelist": {"accounts": [contract]}}}}]}});
    // 1e19 yocto per stored byte; the contract refunds any excess.
    let deposit = (ct.len() as u128 + 2048) * 10_000_000_000_000_000_000;
    let out = ctx.near.function_call(&signer, outlayer, "store_secrets",
        json!({"accessor": {"WasmHash": {"hash": wasm_hash}}, "profile": profile, "encrypted_secrets_base64": ct,
               "access": access, "vault_id": null}), 100, deposit)?;
    let s = ensure_ok(ctx, &out, "store_secrets")?;
    Ok(json!({"owner": signer.account_id, "profile": profile, "wasm_hash": wasm_hash, "keys": v["all_keys"],
              "access": access, "tx": s}))
}

pub fn tx(ctx: &Ctx, hash: &str, signer: &str) -> Result<Value> {
    let out = ctx.near.tx_status(hash, signer)?;
    Ok(json!({"explorer": ctx.near.explorer_tx(hash), "failures": near::failures(&out), "events": near::events(&out),
              "status": out["status"]}))
}

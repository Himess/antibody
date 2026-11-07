//! `antibody judge`: what anyone can check with view calls only, with no account and no key. For each
//! contract: its code hash and whether it is locked (no full-access key). The verifier, image, DCAP and
//! light-client pins. For each program: its anchor, its verdicts and how many are T1-confirmed or carry a
//! T2 validity proof (near-t2), its payouts, and the pause and release permits near-guardian signed for
//! it. Accounts come with explorer links.

use std::collections::BTreeMap;

use eyre::Result;
use serde_json::{json, Value};

use crate::{cmd, ctx::Ctx};

/// How an automated drill names its program (`antibody-services drill`, docs/drill.md): a fresh instance of
/// the Acme Lend fixture, proven, paused and paid once a day, then closed. The rest of the name is the
/// drill's date.
pub const DRILL_PROGRAM_PREFIX: &str = "Acme Lend drill ";

/// Whether a program is an automated drill, by the name the drill gives it. A label only: a program's name
/// is its project's to choose, and nothing else depends on it.
pub fn is_drill(name: &str) -> bool {
    name.strip_prefix(DRILL_PROGRAM_PREFIX).is_some_and(|rest| rest.len() >= 10 && rest.as_bytes()[..4].iter().all(u8::is_ascii_digit))
}

/// A contract account: its code hash, its full-access keys (none: locked) and its explorer page.
fn contract(ctx: &Ctx, role: &str, account: &str) -> Result<Value> {
    let a = ctx.near.account(account)?;
    let full: Vec<Value> = match &a {
        Some(_) => ctx.near.access_keys(account)?.into_iter().filter(|k| k["access_key"]["permission"] == json!("FullAccess")).map(|k| k["public_key"].clone()).collect(),
        None => vec![],
    };
    Ok(json!({"role": role, "account": account, "exists": a.is_some(), "code_hash": a.as_ref().map(|a| a["code_hash"].clone()),
              "locked": a.is_some() && full.is_empty(), "full_access_keys": full, "link": ctx.near.explorer_account(account)}))
}

/// Every permit near-guardian signed.
fn permits(ctx: &Ctx, guardian: &str) -> Result<Vec<Value>> {
    let mut out: Vec<Value> = Vec::new();
    loop {
        let page = ctx.near.view(guardian, "list_permits", json!({"from": out.len(), "limit": 100}))?;
        let page = page.as_array().cloned().unwrap_or_default();
        if page.is_empty() {
            return Ok(out);
        }
        out.extend(page);
    }
}

fn program(ctx: &Ctx, id: u32, signed: &[Value], t2: &BTreeMap<u64, Value>) -> Result<Value> {
    let p = ctx.program(id)?;
    let meta = cmd::meta(ctx, id)?;
    let record = ctx.view("program_record", json!({"program_id": id}))?;
    let audit = ctx.view("audit", json!({"program_id": id}))?;
    let (mut runs, mut t1, mut t2_confirmed, mut t2_contradicted) = (0u32, 0u32, 0u32, 0u32);
    let mut states: BTreeMap<String, u32> = BTreeMap::new();
    let mut payouts = Vec::new();
    let mut tiers = Vec::new();
    for c in cmd::list_commits(ctx, id)? {
        let st = cmd::state_label(&c["state"]);
        *states.entry(st.clone()).or_default() += 1;
        if let Some(r) = c["run"].as_u64() {
            runs += 1;
            let run = cmd::run(ctx, r)?;
            if cmd::run_view(&run)["tier"] == json!("T1") {
                t1 += 1;
            }
            let proof = t2.get(&r);
            match proof.map(|x| x["outcome"].as_str().unwrap_or_default()) {
                Some("confirmed") => t2_confirmed += 1,
                Some("contradicts") => t2_contradicted += 1,
                _ => {}
            }
            tiers.push(json!({"commit": c["id"], "run": r, "tier": crate::zk::tier(&run, proof), "tee_tier": cmd::run_view(&run)["tier"]}));
        }
        if st == "Done:Paid" {
            let to = c["beneficiary"].as_str().unwrap_or_default();
            payouts.push(json!({"commit": c["id"], "hunter": c["hunter"], "paid_to": to, "amount_raw": cmd::paid_of(&c).to_string(),
                                "link": ctx.near.explorer_account(to)}));
        }
    }
    let key = p["key"].as_str().unwrap_or_default();
    let mine: Vec<Value> = signed
        .iter()
        .filter(|x| x["program_key"] == json!(key))
        .map(|x| json!({"id": x["id"], "chain_id": x["chain_id"], "adapter": x["adapter"], "round": x["round"], "body": x["body"], "deadline": x["deadline"]}))
        .collect();
    let pauses = mine.iter().filter(|x| x["body"].get("Pause").is_some()).count();
    // The current round's rule strength, as its manifest commits it (docs/rule-strength.md).
    let strength = cmd::program_manifest(ctx, id).ok().and_then(|m| serde_json::from_slice::<Value>(&m).ok()).map(|m| m["rule_strength"].clone()).unwrap_or(Value::Null);
    let drill = meta["name"].as_str().is_some_and(is_drill);
    Ok(json!({
        "program_id": id, "name": meta["name"], "kind": if drill { "drill" } else { "program" }, "drill": drill,
        "project": p["project"], "phase": p["phase"], "round": p["round"], "program_key": key,
        "anchor": {"chain_id": meta["chain_id"], "block": meta["anchor_block"], "hash": meta["anchor_hash"], "status": record["anchor"]},
        "terms_sha256": meta["terms_sha256"], "pins": ctx.view("get_pins", json!({"program_id": id}))?,
        "verdicts": {"runs": runs, "t1_confirmed": t1, "t2_confirmed": t2_confirmed, "t2_contradicted": t2_contradicted, "tiers": tiers},
        "commits": states, "payouts": payouts,
        "pauses": pauses, "releases": mine.len() - pauses, "permits": mine, "rule_strength": strength,
        "record": record, "audit_ok": audit["ok"], "link": ctx.near.explorer_account(p["project"].as_str().unwrap_or_default()),
    }))
}

/// A versioned deployment record (`antibody admin record-deployment`): what runs where, as the chain
/// shows it now. Each contract's code hash and whether it is locked, the verifier build programs pin
/// for new programs, and the source commit the operator names.
pub fn deployment_record(ctx: &Ctx, git_commit: Option<&str>) -> Result<Value> {
    let mut contracts = vec![contract(ctx, "near-antibody", &ctx.dep.antibody)?, contract(ctx, "payout token", &ctx.dep.token)?];
    if let Some(g) = &ctx.dep.guardian {
        contracts.push(contract(ctx, "near-guardian", g)?);
    }
    if let Some(p) = &ctx.dep.practice {
        contracts.push(contract(ctx, "practice vending", &p.vending)?);
    }
    if let Some(h) = &ctx.dep.hunt {
        contracts.push(contract(ctx, "hunt vending", &h.vending)?);
    }
    if let Some(g) = &ctx.dep.verdict_guard {
        contracts.push(contract(ctx, "verdict guard", g)?);
    }
    if let Some(t) = &ctx.dep.t2 {
        contracts.push(contract(ctx, "T2 (near-t2)", t)?);
    }
    let defaults = ctx.view("get_defaults", json!({}))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    Ok(json!({"recorded_at": now, "network": ctx.near.network, "git_commit": git_commit,
              "contracts": contracts.iter().map(|c| json!({"role": c["role"], "account": c["account"], "code_hash": c["code_hash"], "locked": c["locked"]})).collect::<Vec<_>>(),
              "verifier": {"wasm_hash": defaults["verifier"]["wasm_hash"], "wasm_url": defaults["verifier"]["wasm_url"]},
              "outlayer": defaults["outlayer"], "light_client": defaults["light_client"]}))
}

/// The whole view: one program with `only`, otherwise every program.
pub fn judge(ctx: &Ctx, only: Option<u32>) -> Result<Value> {
    if ctx.near.account(&ctx.dep.antibody)?.is_none() {
        return Err(eyre::eyre!(
            "near-antibody is not deployed at {} on {} yet (the README's Status says when it is); \
             the sandbox runs the same checks: `make serve` in the repository, then antibody --network sandbox \
             --rpc http://127.0.0.1:3030 --deployments target/demo/deployments.json judge",
            ctx.dep.antibody, ctx.near.network
        ));
    }
    let mut contracts = vec![contract(ctx, "near-antibody", &ctx.dep.antibody)?, contract(ctx, "payout token", &ctx.dep.token)?];
    let mut signed = Vec::new();
    let mut guardian = Value::Null;
    if let Some(g) = &ctx.dep.guardian {
        contracts.push(contract(ctx, "near-guardian", g)?);
        signed = permits(ctx, g)?;
        guardian = json!({"config": ctx.near.view(g, "get_config", json!({}))?, "pending_permits": ctx.near.view(g, "pending_permits", json!({}))?});
    }
    let mut verdict_guard = Value::Null;
    if let Some(v) = &ctx.dep.verdict_guard {
        contracts.push(contract(ctx, "verdict guard", v)?);
        verdict_guard = ctx.near.view(v, "get_state", json!({}))?;
    }
    // T2: validity proofs of disclosed verdicts, checked by near-t2 against near-antibody's records.
    let (mut t2, mut t2_records) = (Value::Null, BTreeMap::new());
    if let Some(t) = &ctx.dep.t2 {
        contracts.push(contract(ctx, "T2 (near-t2)", t)?);
        if ctx.near.account(t)?.is_some() {
            t2_records = crate::zk::records(ctx, t)?;
            t2 = json!({"pins": ctx.near.view(t, "get_pins", json!({}))?, "stats": ctx.near.view(t, "get_stats", json!({}))?,
                        "note": "a validity proof of a disclosed verdict: no privacy is claimed, and payouts do not depend on it"});
        }
    }
    let ids: Vec<u32> = match only {
        Some(i) => vec![i],
        None => {
            let n = ctx.view("get_stats", json!({}))?["next_program"].as_u64().unwrap_or(0) as u32;
            (0..n).collect()
        }
    };
    let programs = ids.into_iter().map(|i| program(ctx, i, &signed, &t2_records)).collect::<Result<Vec<_>>>()?;
    Ok(json!({"network": ctx.near.network, "note": "view calls only: no account and no key",
              "contracts": contracts, "defaults": ctx.view("get_defaults", json!({}))?, "images": ctx.view("get_images", json!({}))?,
              "audit_contract": ctx.view("audit_contract", json!({}))?, "guardian": guardian, "verdict_guard": verdict_guard, "t2": t2, "programs": programs}))
}

/// `judge`'s one-screen summary: the lines JUDGES.md expects, in the same words (`--json` for the record).
pub fn summary(j: &Value, decimals: u32, symbol: &str) -> String {
    let mut out = vec![format!("Antibody on {} (view calls only: no account, no key)", j["network"].as_str().unwrap_or("?"))];
    for c in j["contracts"].as_array().into_iter().flatten() {
        out.push(format!("  {:<17} {:<32} locked: {}", c["role"].as_str().unwrap_or(""), c["account"].as_str().unwrap_or(""), c["locked"]));
    }
    out.push(format!("  contract audit ok: {} (every credit, escrow and bond it holds balances)", j["audit_contract"]["ok"]));
    let programs = j["programs"].as_array().cloned().unwrap_or_default();
    let drills = programs.iter().filter(|p| p["drill"] == json!(true)).count();
    out.push(if drills > 0 { format!("{} programs ({drills} of them automated drills)", programs.len()) } else { format!("{} programs", programs.len()) });
    for p in &programs {
        out.push(format!(
            "  program {} ({}{}): phase {}, anchor {}, {} of {} verdicts T1-confirmed, {} with a T2 proof, {} payouts, {} pause and {} release permits, audit ok: {}",
            p["program_id"], p["name"].as_str().unwrap_or(""), if p["drill"] == json!(true) { ", an automated drill" } else { "" },
            p["phase"].as_str().unwrap_or("?"), p["anchor"]["status"].as_str().unwrap_or("?"),
            p["verdicts"]["t1_confirmed"], p["verdicts"]["runs"], p["verdicts"]["t2_confirmed"], p["payouts"].as_array().map(|x| x.len()).unwrap_or(0),
            p["pauses"], p["releases"], p["audit_ok"]
        ));
        let tiers: Vec<String> = p["verdicts"]["tiers"].as_array().into_iter().flatten().map(|t| format!("run {} {}", t["run"], t["tier"].as_str().unwrap_or("-"))).collect();
        if !tiers.is_empty() {
            out.push(format!("    tiers: {}", tiers.join(", ")));
        }
        if p["verdicts"]["t2_contradicted"].as_u64().unwrap_or(0) > 0 {
            out.push(format!("    T2 contradicted {} verdict(s): near-t2 recorded another receipt for the same input", p["verdicts"]["t2_contradicted"]));
        }
        if let Some(s) = p["rule_strength"]["statement"].as_str() {
            out.push(format!("    rule strength: {s} (it measures the rules, not the code's security)"));
        }
        for x in p["payouts"].as_array().into_iter().flatten() {
            let raw = x["amount_raw"].as_str().and_then(|a| a.parse::<u128>().ok()).unwrap_or(0);
            out.push(format!("    paid: commit {} to {}, {} {symbol}", x["commit"], x["paid_to"].as_str().unwrap_or(""), cmd::human_tokens(raw, decimals)));
        }
        for x in p["permits"].as_array().into_iter().flatten() {
            let kind = if x["body"].get("Pause").is_some() { "pause" } else { "release" };
            out.push(format!("    permit {}: {kind}, chain {}, round {}", x["id"], x["chain_id"], x["round"]));
        }
    }
    out.push("The full record, with code hashes, pins and links: add --json".into());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drills_are_labelled_by_their_name() {
        assert!(is_drill("Acme Lend drill 2026-11-10"));
        assert!(is_drill("Acme Lend drill 2026-11-10T1405"));
        for other in ["Acme Lend v1 — pre-launch (testnet)", "Practice program — a tutorial (testnet)", "Acme Lend drill", "Acme Lend drill soon", "acme lend drill 2026-11-10"] {
            assert!(!is_drill(other), "{other}");
        }
        let j = json!({"network": "testnet", "contracts": [], "audit_contract": {"ok": true}, "programs": [
            {"program_id": 5, "name": "Acme Lend v1 — pre-launch (testnet)", "drill": false, "phase": "Open", "anchor": {"status": "light-client"}, "verdicts": {}},
            {"program_id": 9, "name": "Acme Lend drill 2026-11-10", "drill": true, "phase": "Exited", "anchor": {"status": "light-client"}, "verdicts": {}},
        ]});
        let s = summary(&j, 24, "wNEAR");
        assert!(s.contains("2 programs (1 of them automated drills)"), "{s}");
        assert!(s.contains("program 9 (Acme Lend drill 2026-11-10, an automated drill): phase Exited"), "{s}");
        assert!(s.contains("program 5 (Acme Lend v1 — pre-launch (testnet)): phase Open"), "{s}");
    }
}

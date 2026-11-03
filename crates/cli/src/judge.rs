//! `antibody judge`: what anyone can check with view calls only, with no account and no key. For each
//! contract: its code hash and whether it is locked (no full-access key). The verifier, image, DCAP and
//! light-client pins. For each program: its anchor, its verdicts and how many are T1-confirmed, its
//! payouts, and the pause and release permits near-guardian signed for it. Accounts come with explorer
//! links.

use std::collections::BTreeMap;

use eyre::Result;
use serde_json::{json, Value};

use crate::{cmd, ctx::Ctx};

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

fn program(ctx: &Ctx, id: u32, signed: &[Value]) -> Result<Value> {
    let p = ctx.program(id)?;
    let meta = cmd::meta(ctx, id)?;
    let record = ctx.view("program_record", json!({"program_id": id}))?;
    let audit = ctx.view("audit", json!({"program_id": id}))?;
    let (mut runs, mut t1) = (0u32, 0u32);
    let mut states: BTreeMap<String, u32> = BTreeMap::new();
    let mut payouts = Vec::new();
    for c in cmd::list_commits(ctx, id)? {
        let st = cmd::state_label(&c["state"]);
        *states.entry(st.clone()).or_default() += 1;
        if let Some(r) = c["run"].as_u64() {
            runs += 1;
            if cmd::run_view(&cmd::run(ctx, r)?)["tier"] == json!("T1") {
                t1 += 1;
            }
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
    Ok(json!({
        "program_id": id, "name": meta["name"], "project": p["project"], "phase": p["phase"], "round": p["round"], "program_key": key,
        "anchor": {"chain_id": meta["chain_id"], "block": meta["anchor_block"], "hash": meta["anchor_hash"], "status": record["anchor"]},
        "terms_sha256": meta["terms_sha256"], "pins": ctx.view("get_pins", json!({"program_id": id}))?,
        "verdicts": {"runs": runs, "t1_confirmed": t1}, "commits": states, "payouts": payouts,
        "pauses": pauses, "releases": mine.len() - pauses, "permits": mine, "rule_strength": strength,
        "record": record, "audit_ok": audit["ok"], "link": ctx.near.explorer_account(p["project"].as_str().unwrap_or_default()),
    }))
}

/// The whole view: one program with `only`, otherwise every program.
pub fn judge(ctx: &Ctx, only: Option<u32>) -> Result<Value> {
    let mut contracts = vec![contract(ctx, "near-antibody", &ctx.dep.antibody)?, contract(ctx, "payout token", &ctx.dep.tusd)?];
    let mut signed = Vec::new();
    let mut guardian = Value::Null;
    if let Some(g) = &ctx.dep.guardian {
        contracts.push(contract(ctx, "near-guardian", g)?);
        signed = permits(ctx, g)?;
        guardian = json!({"config": ctx.near.view(g, "get_config", json!({}))?, "pending_permits": ctx.near.view(g, "pending_permits", json!({}))?});
    }
    let ids: Vec<u32> = match only {
        Some(i) => vec![i],
        None => {
            let n = ctx.view("get_stats", json!({}))?["next_program"].as_u64().unwrap_or(0) as u32;
            (0..n).collect()
        }
    };
    let programs = ids.into_iter().map(|i| program(ctx, i, &signed)).collect::<Result<Vec<_>>>()?;
    Ok(json!({"network": ctx.near.network, "note": "view calls only: no account and no key",
              "contracts": contracts, "defaults": ctx.view("get_defaults", json!({}))?, "images": ctx.view("get_images", json!({}))?,
              "guardian": guardian, "programs": programs}))
}

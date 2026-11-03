//! Health and monitoring (`GET /v1/status`, the notifier's alerts): each service's progress from its
//! state file, and the balances that keep the deployment running, against thresholds. The practice
//! vending contract is watched closely (faucet-drain protection): its balance, the public vend key's
//! remaining allowance and its credit at near-antibody.

use std::path::Path;

use antibody_cli::{ctx::Deployments, near::Near};
use eyre::Result;
use serde_json::{json, Value};

const ONE_NEAR: f64 = 1e24;

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn read_json(path: &Path) -> Value {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

/// Seconds since a state file last changed, if it exists.
fn age(path: &Path) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(std::time::SystemTime::now().duration_since(m).map(|d| d.as_secs()).unwrap_or(0))
}

fn yocto(v: &Value) -> f64 {
    v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64()).unwrap_or(0.0)
}

/// Thresholds below which an alert is raised.
pub struct Thresholds {
    pub stale_s: u64,
    pub vending_near: f64,
    pub vend_key_near: f64,
    pub vending_credit_near: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self { stale_s: 300, vending_near: 0.5, vend_key_near: 0.05, vending_credit_near: 0.1 }
    }
}

pub fn compute(near: &Near, dep: &Deployments, state_dir: &Path, t: &Thresholds) -> Result<Value> {
    let mut alerts = Vec::new();
    let mut alert = |level: &str, component: &str, key: String, message: String| {
        alerts.push(json!({"level": level, "component": component, "key": key, "message": message}));
    };
    let mut components = serde_json::Map::new();

    // The services, from their state files (absent: the service is not run here).
    for (name, file) in [("indexer", "cursor.json"), ("keeper", "keeper.json"), ("relayer", "relayer.json"), ("notifier", "notifier.cursor")] {
        let a = age(&state_dir.join(file));
        components.insert(name.into(), json!({"state_file": file, "updated_s_ago": a}));
        if let Some(a) = a {
            if a > t.stale_s && name != "relayer" {
                alert("warn", name, format!("{name}-stale"), format!("{name} has not advanced for {a} s"));
            }
        }
    }
    let relayer = read_json(&state_dir.join("relayer.json"));
    let pending: Vec<Value> = relayer.as_object().into_iter().flatten().filter(|(_, v)| v["status"] != json!("confirmed")).map(|(k, v)| json!({"permit": k, "status": v["status"], "chain_id": v["chain_id"]})).collect();
    if !pending.is_empty() {
        alert("warn", "relayer", "relayer-pending".into(), format!("{} permit(s) not yet confirmed on their chain", pending.len()));
    }
    components.insert("permits_pending".into(), json!(pending));
    let keeper = read_json(&state_dir.join("keeper.json"));
    for p in keeper["programs"].as_array().into_iter().flatten().filter(|p| p.get("error").is_some()) {
        alert("warn", "keeper", format!("keeper-error-{}", p["program"]), format!("keeper pass on program {} failed: {}", p["program"], p["error"].as_str().unwrap_or_default()));
    }

    // Programs: an open program's escrow should cover its largest payout.
    let stats = near.view(&dep.antibody, "get_stats", json!({}))?;
    let n = stats["next_program"].as_u64().unwrap_or(0);
    let mut programs = Vec::new();
    for id in 0..n {
        let p = near.view(&dep.antibody, "get_program", json!({"program_id": id}))?;
        if p.is_null() {
            continue;
        }
        let available = yocto(&p["ledger"]["available"]);
        let largest = ["critical", "high", "medium", "low"].iter().map(|k| yocto(&p["params"][k])).fold(0.0, f64::max);
        if p["phase"] == json!("Open") && available < largest {
            alert("warn", "escrow", format!("escrow-{id}"), format!("program {id}'s available escrow ({available}) is below its largest payout ({largest}): top it up"));
        }
        programs.push(json!({"program_id": id, "phase": p["phase"], "round": p["round"], "available_raw": p["ledger"]["available"], "largest_payout_raw": largest.to_string()}));
    }
    components.insert("programs".into(), json!(programs));

    // The practice vending contract (faucet-drain protection).
    if let Some(pr) = &dep.practice {
        let acc = near.account(&pr.vending)?;
        let balance = acc.as_ref().map(|a| yocto(&a["amount"])).unwrap_or(0.0) / ONE_NEAR;
        let public = antibody_cli::practice::public_vend_signer(&pr.vending).public_key_str();
        let key = near.access_keys(&pr.vending)?.into_iter().find(|k| k["public_key"] == json!(public));
        let allowance = key.as_ref().map(|k| yocto(&k["access_key"]["permission"]["FunctionCall"]["allowance"]) / ONE_NEAR);
        let credit = near.view(&dep.antibody, "get_account", json!({"account_id": pr.vending}))?;
        let credit_near = yocto(&credit["near"]) / ONE_NEAR;
        if balance < t.vending_near {
            alert("critical", "practice", "vending-balance".into(), format!("the vending contract holds {balance:.3} NEAR: refill it"));
        }
        match allowance {
            None => alert("critical", "practice", "vend-key-missing".into(), "the public vend key is gone: add it again (`antibody practice deploy` adds it)".into()),
            Some(a) if a < t.vend_key_near => alert("warn", "practice", "vend-key-allowance".into(), format!("the public vend key has {a:.4} NEAR of allowance left")),
            _ => {}
        }
        if credit_near < t.vending_credit_near {
            alert("warn", "practice", "vending-credit".into(), format!("the vending contract's NEAR credit at near-antibody is {credit_near:.3}: `antibody practice fund`"));
        }
        components.insert("practice".into(), json!({"vending": pr.vending, "program": pr.program, "balance_near": balance,
                                                   "public_vend_key_allowance_near": allowance, "credit_near": credit_near, "credit_tokens_raw": credit["tokens"]}));
    }
    let ok = !alerts.iter().any(|a| a["level"] == json!("critical"));
    Ok(json!({"at": now_s(), "ok": ok, "network": near.network, "components": components, "alerts": alerts}))
}

//! Health and monitoring (`GET /v1/status`, the notifier's alerts): each service's progress from its
//! state file, and the balances that keep the deployment running, against thresholds. The practice
//! vending contract is watched closely (faucet-drain protection): its balance, the public vend key's
//! remaining allowance and its credit at near-antibody. The T1 relayer's failures are alerts, and so is
//! the expiry of OutLayer's on-chain DCAP collateral.

use std::{
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};

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
    compute_with(near, dep, state_dir, t, &Default::default(), &Default::default())
}

/// [`compute`], where a permit the relayer has not confirmed counts as pending only if it is not carried
/// out already: a recorded run relayed it (`recorded`, from the results files), or its adapter's `used`
/// view says it was applied (`chains`: EVM RPCs by chain id). A relayer that starts from a new state file
/// meets earlier runs' permits that are long applied.
pub fn compute_with(
    near: &Near,
    dep: &Deployments,
    state_dir: &Path,
    t: &Thresholds,
    chains: &std::collections::BTreeMap<u64, String>,
    recorded: &std::collections::BTreeMap<(String, u64), Value>,
) -> Result<Value> {
    let mut alerts = Vec::new();
    let mut alert = |level: &str, component: &str, key: String, message: String| {
        alerts.push(json!({"level": level, "component": component, "key": key, "message": message}));
    };
    let mut components = serde_json::Map::new();

    // The services, from their state files (absent: the service is not run here).
    for (name, file) in [("indexer", "cursor.json"), ("keeper", "keeper.json"), ("relayer", "relayer.json"), ("notifier", "notifier.cursor"), ("t1", "t1.json")] {
        let a = age(&state_dir.join(file));
        components.insert(name.into(), json!({"state_file": file, "updated_s_ago": a}));
        if let Some(a) = a {
            if a > t.stale_s && name != "relayer" {
                alert("warn", name, format!("{name}-stale"), format!("{name} has not advanced for {a} s"));
            }
        }
    }
    let relayer = read_json(&state_dir.join("relayer.json"));
    // The permits' bodies (their replay keys), read once if a check needs them.
    let bodies: Value = if chains.is_empty() { Value::Null } else { dep.guardian.as_ref().and_then(|g| near.view(g, "list_permits", json!({"from": 0, "limit": 100})).ok()).unwrap_or(Value::Null) };
    let mut applied = Vec::new();
    let mut pending = Vec::new();
    for (k, v) in relayer.as_object().into_iter().flatten().filter(|(_, v)| v["status"] != json!("confirmed")) {
        let id = v["permit_id"].as_u64();
        let rec = id.and_then(|i| recorded.get(&(v["program_key"].as_str().unwrap_or_default().to_string(), i)));
        let permit = bodies.as_array().and_then(|b| b.iter().find(|p| p["id"].as_u64() == id && p["chain_id"] == v["chain_id"])).cloned();
        let state = match (rec, permit) {
            (Some(r), _) => Some(json!({"permit": k, "chain_id": v["chain_id"], "status": "confirmed", "tx": r["tx_hash"], "source": r["results_file"]})),
            (None, Some(p)) => {
                let st = crate::web::permit_state(v, None, &p, chains);
                (st["status"] == "applied").then(|| json!({"permit": k, "chain_id": v["chain_id"], "status": "applied", "source": st["source"]}))
            }
            _ => None,
        };
        match state {
            Some(s) => applied.push(s),
            None => pending.push(json!({"permit": k, "status": v["status"], "chain_id": v["chain_id"]})),
        }
    }
    if !pending.is_empty() {
        alert("warn", "relayer", "relayer-pending".into(), format!("{} permit(s) not yet confirmed on their chain", pending.len()));
    }
    components.insert("permits_pending".into(), json!(pending));
    if !applied.is_empty() {
        components.insert("permits_applied".into(), json!(applied));
    }
    let keeper = read_json(&state_dir.join("keeper.json"));
    for p in keeper["programs"].as_array().into_iter().flatten().filter(|p| p.get("error").is_some()) {
        alert("warn", "keeper", format!("keeper-error-{}", p["program"]), format!("keeper pass on program {} failed: {}", p["program"], p["error"].as_str().unwrap_or_default()));
    }

    // The T1 relayer (if it runs here): failed relays, and the collateral it relays with.
    let t1 = read_json(&state_dir.join("t1.json"));
    if !t1.is_null() {
        let collateral = t1["_collateral"].as_str().unwrap_or("auto").to_string();
        // The chain decides: a run that is T1 now (any relayer, or by hand) is settled, and so is one
        // whose commit is done (paid after the window, or otherwise).
        let mut wants = |run: u64| -> bool {
            let Ok(r) = near.view(&dep.antibody, "get_run", json!({"run_id": run})) else { return true };
            if !crate::t1::wants_t1(&r) {
                return false;
            }
            match r["commit"].as_u64() {
                Some(c) => near.view(&dep.antibody, "get_commit", json!({"commit_id": c})).map(|c| c["state"].get("Done").is_none()).unwrap_or(true),
                None => true,
            }
        };
        let (found, failing) = t1_alerts(&t1, &mut wants);
        for (level, key, message) in found {
            alert(level, "t1", key, message);
        }
        let windows = collateral_windows(near);
        let until = windows.as_array().into_iter().flatten().filter_map(|w| w["valid_until"].as_str()).max().map(String::from);
        let expired = until.as_deref().and_then(unix_of_rfc3339).map(|t| t <= now_s());
        match (expired, collateral.as_str()) {
            (Some(true), "outlayer") => alert("critical", "t1", "t1-collateral-expired".into(),
                format!("OutLayer's on-chain DCAP collateral expired at {}, and the T1 relayer uses only those copies: restart it with `--t1-collateral auto`", until.as_deref().unwrap_or_default())),
            (Some(true), _) => alert("info", "t1", "t1-collateral-pcs".into(),
                format!("OutLayer's on-chain DCAP collateral expired at {}: T1 relays use fresh Intel PCS collateral ({collateral})", until.as_deref().unwrap_or_default())),
            _ => {}
        }
        components.insert("t1".into(), json!({"state_file": "t1.json", "updated_s_ago": age(&state_dir.join("t1.json")), "collateral": collateral,
                                             "outlayer_collateral_valid_until": until, "outlayer_collateral": windows, "failing": failing}));
    }

    // OutLayer's worker images: a run on an image near-antibody does not list, or listed too recently for
    // the program's image delay, gets no T1 (its payout waits the window).
    let images = image_status(near, dep);
    if let Some(n) = images["unlisted"].as_u64().filter(|n| *n > 0) {
        alert("critical", "images", "images-unlisted".into(), format!(
            "OutLayer approves {n} worker image(s) near-antibody does not list: a run on one gets no T1 until `sync_images` (the keeper calls it) and the image delay ({} h) have passed",
            images["delay_secs"].as_u64().unwrap_or(0) / 3600));
    }
    if let Some(from) = images["pending"].as_array().and_then(|p| p.iter().filter_map(|i| i["counts_from"].as_u64()).min()) {
        alert("warn", "images", "images-pending".into(), format!(
            "{} recently listed worker image(s) count for existing programs only from {}: a run on one gets no T1 before then",
            images["pending"].as_array().map(|p| p.len()).unwrap_or(0), rfc3339_of_unix(from)));
    }
    if !images.is_null() {
        components.insert("images".into(), images);
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

    // The operating accounts' floats (`floats.json`, written by `antibody-services all`): the NEAR each
    // keeps liquid for its gas. Below the float: warn; below a quarter of it: critical.
    let floats = read_json(&state_dir.join("floats.json"));
    let mut float_rows = vec![];
    for f in floats.as_array().into_iter().flatten() {
        let (role, account, min) = (f["role"].as_str().unwrap_or("?"), f["account"].as_str().unwrap_or_default(), f["min_near"].as_f64().unwrap_or(0.0));
        let Ok(Some(acc)) = near.account(account) else { continue };
        let liquid = (yocto(&acc["amount"]) - acc["storage_usage"].as_f64().unwrap_or(0.0) * 1e19) / ONE_NEAR;
        if liquid < min / 4.0 {
            alert("critical", "floats", format!("float-{role}"), format!("{role} ({account}) has {liquid:.3} NEAR liquid, under a quarter of its {min} NEAR float: top it up"));
        } else if liquid < min {
            alert("warn", "floats", format!("float-{role}"), format!("{role} ({account}) has {liquid:.3} NEAR liquid, under its {min} NEAR float"));
        }
        float_rows.push(json!({"role": role, "account": account, "liquid_near": liquid, "float_near": min}));
    }
    if !float_rows.is_empty() {
        components.insert("floats".into(), json!(float_rows));
    }

    // The T2 prover service (`prover.json`).
    let prover = read_json(&state_dir.join("prover.json"));
    if prover.is_object() {
        let (found, component) = crate::prover::status(&prover);
        for (level, key, message) in found {
            alert(level, "t2-prover", key, message);
        }
        components.insert("t2_prover".into(), component);
    }

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

/// The T1 relayer's failed relays (`t1.json`) whose run still wants T1, as (level, key, message), and the
/// failing runs. A failed attempt warns; a run the relayer gave up on is critical: it stays T0, so its
/// payout waits the window (a judge's practice try is not paid at once).
pub fn t1_alerts(state: &Value, still_wants_t1: &mut dyn FnMut(u64) -> bool) -> (Vec<(&'static str, String, String)>, Vec<Value>) {
    let (mut alerts, mut failing) = (Vec::new(), Vec::new());
    for (k, row) in state.as_object().into_iter().flatten() {
        // Rows are keyed by run id; "_from" and "_collateral" are the relayer's own.
        let Ok(run) = k.parse::<u64>() else { continue };
        if row["confirmed"] == json!(true) || row.get("settled").is_some() || !still_wants_t1(run) {
            continue;
        }
        let why: String = match (row["error"].as_str(), &row["rejected"]) {
            (Some(e), _) => e.chars().take(200).collect(),
            (None, r) if !r.is_null() => format!("near-antibody rejected the quote: {}", r.as_str().map(String::from).unwrap_or_else(|| r.to_string())),
            _ => "not confirmed".into(),
        };
        let attempts = row["attempts"].as_u64().unwrap_or(0);
        let given_up = row["given_up"] == json!(true);
        if let Some(w) = row["waiting"].as_str() {
            // Not relayed on purpose: near-antibody would reject the worker image until then.
            let at = row["next_at"].as_u64().map(rfc3339_of_unix).unwrap_or_default();
            alerts.push(("warn", format!("t1-waiting-{run}"), format!("run {run} gets no T1 yet ({w}); the relayer tries again at {at}")));
            failing.push(json!({"run": run, "attempts": attempts, "given_up": false, "waiting": w, "next_at": row["next_at"]}));
            continue;
        }
        if given_up {
            alerts.push(("critical", format!("t1-gave-up-{run}"),
                format!("the T1 relayer gave up on run {run} after {attempts} attempts ({why}): the run stays T0 and its payout waits the window; relay it with `antibody tee confirm --run {run}`")));
        } else {
            alerts.push(("warn", format!("t1-failed-{run}"), format!("T1 relay of run {run} failed (attempt {attempts}): {why}")));
        }
        failing.push(json!({"run": run, "attempts": attempts, "given_up": given_up, "reason": why}));
    }
    (alerts, failing)
}

/// OutLayer's on-chain collateral windows, read at most once an hour (each status call would otherwise
/// download every collateral).
fn collateral_windows(near: &Near) -> Value {
    static CACHE: Mutex<Option<(Instant, Value)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, v)) = c.as_ref() {
        if at.elapsed() < Duration::from_secs(3600) {
            return v.clone();
        }
    }
    match antibody_cli::tee::outlayer_collateral_windows(near) {
        Ok(w) => {
            let v = Value::from(w);
            *c = Some((Instant::now(), v.clone()));
            v
        }
        Err(_) => Value::Null,
    }
}

/// OutLayer's approved worker images against near-antibody's list (its default registry), read at most
/// every 10 minutes: how many are unlisted, and which listed ones are still inside the image delay.
fn image_status(near: &Near, dep: &Deployments) -> Value {
    static CACHE: Mutex<Option<(Instant, Value)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, v)) = c.as_ref() {
        if at.elapsed() < Duration::from_secs(600) {
            return v.clone();
        }
    }
    let read = || -> Result<Value> {
        let defaults = near.view(&dep.antibody, "get_defaults", json!({}))?;
        let registry = defaults["image_registry"].as_str().unwrap_or_default().to_string();
        let delay = defaults["image_delay_secs"].as_u64().unwrap_or(0);
        let approved = near.view(&registry, "get_approved_measurements", json!({}))?;
        let known = near.view(&dep.antibody, "get_images", json!({}))?;
        let listed: Vec<&Value> = known[0].as_array().into_iter().flatten().filter(|k| k["registry"] == json!(registry)).collect();
        let unlisted = approved.as_array().into_iter().flatten().filter(|m| !listed.iter().any(|k| &k["m"] == *m)).count();
        let now = now_s();
        let pending: Vec<Value> = listed
            .iter()
            .filter_map(|k| {
                let from = k["added_at"].as_u64()? + delay;
                (from > now).then(|| json!({"rtmr3": k["m"]["rtmr3"], "listed_at": k["added_at"], "counts_from": from, "counts_from_utc": rfc3339_of_unix(from)}))
            })
            .collect();
        Ok(json!({"registry": registry, "approved": approved.as_array().map(|a| a.len()), "listed": listed.len(), "unlisted": unlisted,
                  "delay_secs": delay, "pending": pending}))
    };
    match read() {
        Ok(v) => {
            *c = Some((Instant::now(), v.clone()));
            v
        }
        Err(_) => Value::Null,
    }
}

/// "YYYY-MM-DDTHH:MM:SSZ" of unix seconds.
pub fn rfc3339_of_unix(t: u64) -> String {
    let (days, secs) = ((t / 86_400) as i64, t % 86_400);
    // The civil date of a day count (proleptic Gregorian), 1970-01-01 = 0.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3_600, secs % 3_600 / 60, secs % 60)
}

/// Unix seconds of a UTC timestamp as Intel writes them ("2026-11-07T23:46:20Z").
pub fn unix_of_rfc3339(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return None;
    }
    let n = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse().ok() };
    let (y, m, d, hh, mm, ss) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    // Days from the civil date (proleptic Gregorian), 1970-01-01 = 0.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3_600 + mm * 60 + ss).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_t1_relay_is_an_alert_until_the_run_is_t1() {
        let state = json!({
            "_from": 3, "_collateral": "auto",
            "3": {"attempts": 1, "confirmed": false, "error": "no collateral verifies this quote (intel-pcs: TCBInfo expired)"},
            "4": {"attempts": 5, "confirmed": false, "rejected": "TCB status OutOfDate not accepted", "given_up": true},
            "5": {"attempts": 1, "confirmed": true, "collateral": "intel-pcs"},
            "6": {"attempts": 2, "confirmed": false, "error": "timeout", "settled": true},
            "7": {"attempts": 0, "confirmed": false, "waiting": "the worker image counts for program 6 only from unix 1794143722", "next_at": 1_794_143_727u64},
        });
        let (alerts, failing) = t1_alerts(&state, &mut |_| true);
        let keys: Vec<(&str, &str)> = alerts.iter().map(|(l, k, _)| (*l, k.as_str())).collect();
        assert_eq!(keys, vec![("warn", "t1-failed-3"), ("critical", "t1-gave-up-4"), ("warn", "t1-waiting-7")]);
        assert!(alerts[2].2.contains("2026-11-08T13:15:27Z"), "{}", alerts[2].2);
        assert_eq!(failing.len(), 3);
        assert!(alerts[1].2.contains("OutOfDate"), "the reason is in the message");
        // Run 3 became T1 (another relayer, or by hand): the chain decides, its alert is gone.
        let (alerts, _) = t1_alerts(&state, &mut |run| run != 3);
        assert_eq!(alerts.iter().map(|(_, k, _)| k.as_str()).collect::<Vec<_>>(), vec!["t1-gave-up-4", "t1-waiting-7"]);
        // Retries: every 10 s for two minutes, then doubling up to an hour.
        assert_eq!((crate::t1::backoff(1), crate::t1::backoff(11), crate::t1::backoff(12), crate::t1::backoff(13)), (10, 10, 10, 20));
        assert_eq!((crate::t1::backoff(20), crate::t1::backoff(21), crate::t1::backoff(59)), (2_560, 3_600, 3_600));
    }

    #[test]
    fn intel_timestamps() {
        assert_eq!(unix_of_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        // OutLayer's testnet QE identity (FMSPC B0C06F000000) expires then.
        assert_eq!(unix_of_rfc3339("2026-11-07T23:46:20Z"), Some(1_794_095_180));
        assert_eq!(unix_of_rfc3339("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(unix_of_rfc3339("2026-11-07 23:46:20"), None);
        // The two worker images listed on 2026-11-06 count for programs 5–7 two days later.
        assert_eq!(rfc3339_of_unix(1_793_970_922 + 172_800), "2026-11-08T13:15:22Z");
        for t in [0, 951_782_400, 1_709_208_000, 1_794_095_180, 4_102_444_800] {
            assert_eq!(unix_of_rfc3339(&rfc3339_of_unix(t)), Some(t));
        }
    }
}

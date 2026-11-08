//! What the web app needs from the API (docs/api.md), and the HTTP policy of a public API.
//! - `/v1/moments`: the one moment's runs, read from the results files (`docs/results/`), each step with
//!   its transaction. A scheduled drill's results file (`drill-*.json`) appears there with no change to
//!   the API or the site.
//! - `POST /v1/interest`: the pilot and early-access form. Validated, appended to a private file (never
//!   served, never in the repository) and announced through the notifier's webhooks.
//! - CORS: reads are public (`*`); the form accepts only the site's origins (`$ANTIBODY_WEB_ORIGINS`).
//! - A per-client rate limit, cache headers, and a short cache for the views that read many contracts
//!   (each NEAR RPC rate-limits its clients).
//! - Permits applied on their chain: the adapter's own `used(key)` view says whether a permit was carried
//!   out, by any relayer, so a state file that missed it cannot show a paused or released chain as live.

use std::{
    collections::{BTreeMap, HashMap},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use alloy_primitives::keccak256;
use serde_json::{json, Value};

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ------------------------------------------------------------------------------------ the one moment

/// What a results file records: `testnet-YYYY-MM-DD.json` is a recorded run of the whole story,
/// `drill-*.json` a scheduled drill (a fresh fixture, the one moment live). Staging runs are left out.
pub fn moment_kind(file_name: &str) -> Option<&'static str> {
    let stem = file_name.strip_suffix(".json")?;
    if stem.starts_with("drill-") {
        return Some("drill");
    }
    let (net, date) = stem.split_once('-')?;
    let dated = date.len() == 10 && date.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
    (matches!(net, "testnet" | "mainnet") && dated).then_some("recorded")
}

pub fn chain_name(id: u64) -> &'static str {
    match id {
        11155111 => "Sepolia",
        84532 => "Base Sepolia",
        1 => "Ethereum",
        8453 => "Base",
        _ => "EVM chain",
    }
}

fn evm_explorer(chain_id: u64, tx: &str) -> Option<String> {
    match chain_id {
        11155111 => Some(format!("https://sepolia.etherscan.io/tx/{tx}")),
        84532 => Some(format!("https://sepolia.basescan.org/tx/{tx}")),
        1 => Some(format!("https://etherscan.io/tx/{tx}")),
        8453 => Some(format!("https://basescan.org/tx/{tx}")),
        _ => None,
    }
}

fn utc(ts_ns: u64) -> Value {
    if ts_ns == 0 {
        return Value::Null;
    }
    json!(crate::status::rfc3339_of_unix(ts_ns / 1_000_000_000))
}

/// One run's one moment, from its results file and the indexer's events: the commit (t = 0, its NEAR
/// block) and each step with its seconds from that block, its chain and its transaction.
pub fn moment(file_name: &str, kind: &str, r: &Value, events: &[Value], near_explorer: &dyn Fn(&str) -> String) -> Option<Value> {
    let om = r.get("one_moment").filter(|v| v.is_object())?;
    let commit = om["commit"].as_u64()?;
    let secs = &om["seconds_from_commit"];
    let ev = |name: &str, pred: &dyn Fn(&Value) -> bool| events.iter().find(|e| e["event"] == name && pred(&e["data"])).cloned();
    let of_commit = |d: &Value| d["commit_id"].as_u64() == Some(commit);
    let verify = |d: &Value| of_commit(d) && d["mode"] == "verify";
    let sub = r["submissions"].as_array().and_then(|s| s.iter().find(|s| s["commit"].as_u64() == Some(commit))).cloned().unwrap_or(Value::Null);
    let commit_ev = ev("commit", &of_commit);
    let program_id = commit_ev.as_ref().and_then(|e| e["data"]["program_id"].as_u64()).or(r["program"]["program_id"].as_u64());
    let program_key = program_id.map(|p| format!("{}/{p}", r["deployment"]["near_antibody"].as_str().unwrap_or("bounty.antibody.testnet")));
    let commit_tx = sub["txs"]["commit"].as_str().map(String::from).or_else(|| commit_ev.as_ref().and_then(|e| e["tx"].as_str().map(String::from)));
    let near_link = |tx: Option<String>| -> Value {
        match tx {
            Some(t) => json!({"tx": t, "explorer": near_explorer(&t)}),
            None => json!({"tx": null, "explorer": null}),
        }
    };
    let commit_ts = commit_ev.as_ref().and_then(|e| e["block_ts_ns"].as_u64()).unwrap_or(0);
    let mut steps: Vec<Value> = Vec::new();
    let mut push = |key: &str, label: &str, seconds: Option<f64>, chain: &str, link: Value, detail: Value| {
        if let Some(s) = seconds {
            let mut step = json!({"key": key, "label": label, "seconds": (s * 10.0).round() / 10.0, "seconds_exact": s, "chain": chain, "detail": detail});
            step["tx"] = link["tx"].clone();
            step["explorer"] = link["explorer"].clone();
            steps.push(step);
        }
    };
    let answered = ev("run_answered", &verify);
    let kind_of = answered.as_ref().map(|e| e["data"]["kind"].clone()).unwrap_or(sub["verdict"].clone());
    let slots: Vec<Value> = answered.as_ref().and_then(|e| e["data"]["keys"].as_array().cloned()).unwrap_or_default().iter().map(|k| k["slot"].clone()).collect();
    push("verified", "Enclave verdict", secs["verdict"].as_f64(), "near", near_link(answered.as_ref().and_then(|e| e["tx"].as_str().map(String::from))),
         json!({"kind": kind_of.clone(), "slots": slots.clone(), "by": "the enclave's answer (T0), in the block that resolved the run"}));
    let t1 = ev("t1_confirmed", &verify);
    push("t1", "Attested on NEAR", secs["t1"].as_f64(), "near", near_link(t1.as_ref().and_then(|e| e["tx"].as_str().map(String::from))),
         json!({"by": "the run's TDX quote, verified on NEAR"}));
    // The pause is asked for (the keeper's `pause_ticket`) before the payment.
    let ticket = ev("pause_ticket", &|d: &Value| d["ticket"].as_u64() == Some(commit));
    if let (Some(t), true) = (&ticket, commit_ts > 0) {
        let s = (t["block_ts_ns"].as_u64().unwrap_or(0) as f64 - commit_ts as f64) / 1e9;
        push("pause_asked", "Pause asked", Some(s), "near", near_link(t["tx"].as_str().map(String::from)),
             json!({"by": "a pause ticket: near-guardian asks the MPC network to sign a pause permit for each chain", "minor": true}));
    }
    let pauses: Vec<Value> = om["permits"]
        .as_object()
        .map(|m| m.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p["kind"] == "pause" && p["status"] == "confirmed" && Some(p["program_key"].as_str().unwrap_or_default().to_string()) == program_key)
        .collect();
    let signed_ids: Vec<u64> = pauses.iter().filter_map(|p| p["permit_id"].as_u64()).collect();
    let signed = ev("permit_signed", &|d: &Value| d["id"].as_u64().is_some_and(|i| signed_ids.contains(&i)));
    let signed_s = pauses.iter().filter_map(|p| secs[format!("permit_signed_{}", p["chain_id"])].as_f64()).fold(None, |m: Option<f64>, s| Some(m.map_or(s, |m| m.min(s))));
    push("permits_signed", "Pause permits signed", signed_s, "near", near_link(signed.as_ref().and_then(|e| e["tx"].as_str().map(String::from))),
         json!({"permits": signed_ids, "by": "the MPC network, for near-guardian: one EIP-712 permit per chain"}));
    // Who was paid: the indexed `paid` event, else the keeper's step the results file recorded.
    let paid_data = ev("paid", &of_commit).map(|e| e["data"].clone()).or_else(|| {
        om["keeper"]["steps"].as_array().and_then(|s| s.iter().find(|s| s["paid"].as_u64() == Some(commit))).map(|s| s["event"].clone())
    });
    let beneficiary = paid_data.as_ref().map(|d| d["beneficiary"].clone()).unwrap_or(Value::Null);
    let paid_tx = om["pay_tx"].as_str().map(String::from).or_else(|| ev("paid", &of_commit).and_then(|e| e["tx"].as_str().map(String::from)));
    push("paid", "Paid", secs["paid"].as_f64(), "near", near_link(paid_tx),
         json!({"amount_raw": om["paid_raw"], "instant": om["instant"], "beneficiary": beneficiary.clone()}));
    for p in &pauses {
        let chain_id = p["chain_id"].as_u64().unwrap_or(0);
        let tx = p["tx_hash"].as_str().map(String::from);
        let explorer = p["explorer"].as_str().map(String::from).or_else(|| tx.as_deref().and_then(|t| evm_explorer(chain_id, t)));
        push(&format!("paused_{chain_id}"), &format!("{} paused", chain_name(chain_id)), secs[format!("paused_{chain_id}")].as_f64(),
             &format!("evm:{chain_id}"), json!({"tx": tx, "explorer": explorer}),
             json!({"chain_id": chain_id, "permit": p["permit_id"], "adapter": p["adapter"], "block": p["block"], "targets_paused": p["targets_paused"],
                    "by": "anyone relays the signed permit to the chain's AntibodyAdapter, which checks it and pauses the targets"}));
    }
    steps.sort_by(|a, b| a["seconds_exact"].as_f64().unwrap_or(0.0).total_cmp(&b["seconds_exact"].as_f64().unwrap_or(0.0)));
    let stem = file_name.trim_end_matches(".json");
    // What is true of this run only, shown with its times (`run_notes`); `notes` adds what holds for every
    // run, which the page already says in its own words.
    let mut run_notes = Vec::new();
    // A drill resumed after A's commit (its MCP call cut off by a rate-limited RPC) revealed it later than
    // the MCP server would have: every time from the commit includes that wait, and the page says so.
    if r["mcp"]["resumed"] == json!(true) {
        run_notes.push(json!("This run's reveal came late: the MCP call that committed A's proof stopped (a rate-limited NEAR RPC), and the drill revealed the commit when it resumed. Every time from the commit includes that wait."));
    }
    let mut notes = run_notes.clone();
    notes.push(json!("The pause is asked for before the payment."));
    notes.push(json!("NEAR steps are timed at their blocks, each pause when the relayer got its chain's receipt; seconds from the commit's NEAR block."));
    Some(json!({
        "id": stem,
        "kind": kind,
        "network": r["network"],
        "all_ok": r["all_ok"],
        "results_file": format!("docs/results/{file_name}"),
        "started_unix": r["started_unix"],
        "program_id": program_id,
        "program_key": program_key,
        "commit_id": commit,
        "run_id": sub["run"],
        "hunter": commit_ev.as_ref().map(|e| e["data"]["hunter"].clone()).filter(|h| h.is_string()).unwrap_or(beneficiary),
        "via": sub["agent"],
        "verdict": {"kind": kind_of, "slots": slots, "tier": sub["tier"]},
        "paid_raw": om["paid_raw"],
        "instant": om["instant"],
        "paused_everywhere": om["paused_both"],
        "commit": {"tx": commit_tx, "explorer": commit_tx.as_deref().map(near_explorer), "block_height": commit_ev.as_ref().map(|e| e["block_height"].clone()),
                   "block_ts_ns": commit_ts, "at_utc": utc(commit_ts)},
        "clock": om["clock"],
        "notes": notes,
        "run_notes": run_notes,
        "steps": steps,
    }))
}

/// Every one moment the results directory records, newest first.
pub fn moments(results_dir: &Path, events: &[Value], near_explorer: &dyn Fn(&str) -> String) -> Value {
    let mut out = Vec::new();
    let mut names: Vec<String> = std::fs::read_dir(results_dir)
        .map(|d| d.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str().map(String::from)).collect())
        .unwrap_or_default();
    names.sort();
    for name in names {
        let Some(kind) = moment_kind(&name) else { continue };
        let Ok(text) = std::fs::read_to_string(results_dir.join(&name)) else { continue };
        let Ok(r) = serde_json::from_str::<Value>(&text) else { continue };
        if r["all_ok"] != json!(true) {
            continue;
        }
        if let Some(m) = moment(&name, kind, &r, events, near_explorer) {
            out.push(m);
        }
    }
    out.sort_by(|a, b| b["started_unix"].as_u64().unwrap_or(0).cmp(&a["started_unix"].as_u64().unwrap_or(0)));
    json!({"moments": out, "latest": out.first().map(|m| m["id"].clone()),
           "note": "recorded runs and scheduled drills (results files); a visitor's own practice try is the live one"})
}

/// Permit relays a results file recorded (a run's own relayer state), keyed by program key and permit id.
pub fn recorded_relays(results_dir: &Path) -> BTreeMap<(String, u64), Value> {
    let mut out = BTreeMap::new();
    let Ok(dir) = std::fs::read_dir(results_dir) else { return out };
    for e in dir.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().to_string();
        if moment_kind(&name) != Some("recorded") {
            continue;
        }
        let Some(r) = std::fs::read_to_string(e.path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { continue };
        let mut all: Vec<Value> = r["one_moment"]["permits"].as_object().map(|m| m.values().cloned().collect()).unwrap_or_default();
        all.extend(r["release"]["permits"].as_array().cloned().unwrap_or_default());
        for p in all.into_iter().filter(|p| p["status"] == "confirmed" && p["tx_hash"].is_string()) {
            let (Some(k), Some(id)) = (p["program_key"].as_str(), p["permit_id"].as_u64()) else { continue };
            let mut v = p.clone();
            v["results_file"] = json!(format!("docs/results/{name}"));
            out.insert((k.to_string(), id), v);
        }
    }
    out
}

// --------------------------------------------------------------------- permits applied on their chain

fn word_u32(x: u32) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[28..].copy_from_slice(&x.to_be_bytes());
    w
}

/// `abi.encode(tag, round, last)`: the head (the string's offset, then the two words), then the string.
fn abi_tag(tag: &str, round: u32, last: [u8; 32]) -> Vec<u8> {
    let mut o = Vec::with_capacity(160);
    o.extend(word_u32(0x60));
    o.extend(word_u32(round));
    o.extend(last);
    o.extend(word_u32(tag.len() as u32));
    let mut s = [0u8; 32];
    s[..tag.len()].copy_from_slice(tag.as_bytes());
    o.extend(s);
    o
}

/// The replay key AntibodyAdapter records when it carries a permit out (`_use`):
/// `keccak256(abi.encode("pause", round, ticket))` or `keccak256(abi.encode("release", round, proxy))`.
pub fn permit_used_key(body: &Value, round: u32) -> Option<[u8; 32]> {
    if let Some(t) = body["Pause"]["ticket"].as_u64() {
        let mut w = [0u8; 32];
        w[24..].copy_from_slice(&t.to_be_bytes());
        return Some(keccak256(abi_tag("pause", round, w)).0);
    }
    let proxy = body["Release"]["proxy"].as_str()?;
    let a = hex::decode(proxy.trim_start_matches("0x")).ok().filter(|a| a.len() == 20)?;
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(&a);
    Some(keccak256(abi_tag("release", round, w)).0)
}

/// What the adapters answered, per (adapter, key): a used key stays used; an unused one is asked again after
/// [`UNUSED_TTL`].
type Seen = HashMap<(String, [u8; 32]), (bool, Instant)>;
static SEEN: std::sync::LazyLock<Mutex<Seen>> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
const UNUSED_TTL: Duration = Duration::from_secs(120);

/// `used(key)` on the permit's adapter: whether the permit was carried out on its chain, by any relayer. One
/// attempt with a short timeout (a view the API's answer waits for); an error leaves the relayer's state.
pub fn permit_applied(rpc: &str, adapter: &str, key: [u8; 32]) -> eyre::Result<bool> {
    let id = (adapter.to_lowercase(), key);
    if let Some((used, at)) = SEEN.lock().ok().and_then(|m| m.get(&id).copied()) {
        if used || at.elapsed() < UNUSED_TTL {
            return Ok(used);
        }
    }
    let mut data = keccak256(b"used(bytes32)")[..4].to_vec();
    data.extend(key);
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(6)).build()?;
    let v: Value = client
        .post(rpc)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "eth_call", "params": [{"to": adapter, "data": format!("0x{}", hex::encode(data))}, "latest"]}))
        .send()
        .and_then(|r| r.json())
        // Without the URL: RPC URLs may carry keys.
        .map_err(|e| eyre::eyre!("eth_call: {}", e.without_url()))?;
    if let Some(e) = v.get("error") {
        return Err(eyre::eyre!("eth_call: {e}"));
    }
    let used = v["result"].as_str().map(|s| s.trim_start_matches("0x").trim_start_matches('0') == "1").unwrap_or(false);
    if let Ok(mut m) = SEEN.lock() {
        m.insert(id, (used, Instant::now()));
    }
    Ok(used)
}

/// A permit's state as a UI shows it: the relayer's record, else a recorded run's relay, else the
/// adapter's own `used` view (applied by some relayer, transaction unknown here).
pub fn permit_state(relayed: &Value, recorded: Option<&Value>, permit: &Value, chains: &BTreeMap<u64, String>) -> Value {
    if relayed["status"] == "confirmed" {
        return json!({"status": "confirmed", "tx": relayed["tx_hash"], "explorer": relayed["explorer"], "source": "relayer"});
    }
    if let Some(r) = recorded {
        return json!({"status": "confirmed", "tx": r["tx_hash"], "explorer": r["explorer"], "source": r["results_file"]});
    }
    let chain = permit["chain_id"].as_u64().unwrap_or(0);
    let round = permit["round"].as_u64().unwrap_or(0) as u32;
    if let (Some(rpc), Some(key), Some(adapter)) = (chains.get(&chain), permit_used_key(&permit["body"], round), permit["adapter"].as_str()) {
        match permit_applied(rpc, adapter, key) {
            Ok(true) => return json!({"status": "applied", "tx": null, "explorer": null, "source": "the adapter's used() view"}),
            Ok(false) => {}
            Err(e) => return json!({"status": relayed["status"].as_str().unwrap_or("signed"), "tx": relayed["tx_hash"], "explorer": relayed["explorer"], "check_error": format!("{e:#}")}),
        }
    }
    json!({"status": relayed["status"].as_str().unwrap_or("signed"), "tx": relayed["tx_hash"], "explorer": relayed["explorer"], "source": "relayer"})
}

// ------------------------------------------------------------------------------------- the pilot form

pub const INTEREST_ROLES: &[&str] = &["project", "hunter", "agent", "other"];

/// Trims and checks one text field: `max` characters at most, no control characters (newlines allowed
/// only where `multiline`).
fn text(v: &Value, key: &str, min: usize, max: usize, multiline: bool) -> Result<String, String> {
    let s = match &v[key] {
        Value::Null => String::new(),
        Value::String(s) => s.trim().to_string(),
        _ => return Err(format!("{key}: a string")),
    };
    let n = s.chars().count();
    if n < min || n > max {
        return Err(if min > 0 { format!("{key}: {min} to {max} characters") } else { format!("{key}: at most {max} characters") });
    }
    if s.chars().any(|c| c.is_control() && !(multiline && (c == '\n' || c == '\r' || c == '\t'))) {
        return Err(format!("{key}: no control characters"));
    }
    Ok(s)
}

/// Checks a form submission. `Ok(None)`: the honeypot field is filled (a bot): answered as accepted,
/// stored nowhere. Unknown fields are ignored and never stored.
pub fn validate_interest(v: &Value) -> Result<Option<Value>, String> {
    if !v.is_object() {
        return Err("a JSON object".into());
    }
    if v["website"].as_str().is_some_and(|s| !s.trim().is_empty()) {
        return Ok(None);
    }
    let role = text(v, "role", 1, 16, false)?;
    if !INTEREST_ROLES.contains(&role.as_str()) {
        return Err(format!("role: one of {}", INTEREST_ROLES.join(", ")));
    }
    if v["consent"] != json!(true) {
        return Err("consent: required".into());
    }
    Ok(Some(json!({
        "role": role,
        "name": text(v, "name", 1, 100, false)?,
        "contact": text(v, "contact", 3, 200, false)?,
        "project": text(v, "project", 0, 200, false)?,
        "chains": text(v, "chains", 0, 200, false)?,
        "timeline": text(v, "timeline", 0, 100, false)?,
        "notes": text(v, "notes", 0, 2000, true)?,
        "consent": true,
    })))
}

pub struct Interest {
    /// Private: never served, never in the repository (`$ANTIBODY_INTEREST_FILE`, else the state dir).
    pub file: PathBuf,
    pub webhooks: Vec<String>,
    pub origins: Origins,
    pub per_ip: Limiter,
    pub global: Limiter,
}

impl Interest {
    pub fn from_env(state_dir: &Path) -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Self {
            file: env("ANTIBODY_INTEREST_FILE").map(PathBuf::from).unwrap_or_else(|| state_dir.join("interest.jsonl")),
            webhooks: env("ANTIBODY_WEBHOOKS").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
            origins: Origins::parse(&env("ANTIBODY_WEB_ORIGINS").unwrap_or_default()),
            // Five requests an hour from one client, two hundred a day in all.
            per_ip: Limiter::new(5.0, 5.0 / 3600.0),
            global: Limiter::new(200.0, 200.0 / 86400.0),
        }
    }

    /// Appends one record (owner-only file) and posts a notice to the webhooks in the background.
    pub fn store(&self, rec: &Value) -> eyre::Result<String> {
        use std::io::Write;
        let at = now_s();
        let id = hex::encode(&antibody_verifier::facts::sha256(format!("{at}|{rec}").as_bytes())[..6]);
        let mut line = rec.clone();
        line["id"] = json!(id);
        line["at"] = json!(at);
        line["at_utc"] = json!(crate::status::rfc3339_of_unix(at));
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut o = std::fs::OpenOptions::new();
        o.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&self.file)?;
        writeln!(f, "{line}")?;
        let msg: String = format!(
            "New {} request ({}): {} · {} · {} · chains: {} · timeline: {}{}",
            if rec["role"] == "project" { "pilot" } else { "early-access" },
            rec["role"].as_str().unwrap_or("?"),
            rec["name"].as_str().unwrap_or(""),
            rec["contact"].as_str().unwrap_or(""),
            rec["project"].as_str().unwrap_or("-"),
            rec["chains"].as_str().unwrap_or("-"),
            rec["timeline"].as_str().unwrap_or("-"),
            rec["notes"].as_str().filter(|n| !n.is_empty()).map(|n| format!(" · notes: {n}")).unwrap_or_default(),
        )
        .chars()
        .take(1800)
        .collect();
        let hooks = self.webhooks.clone();
        std::thread::spawn(move || {
            let Ok(client) = reqwest::blocking::Client::builder().timeout(Duration::from_secs(15)).build() else { return };
            for url in &hooks {
                if let Err(err) = client.post(url).json(&json!({"content": msg, "text": msg})).send() {
                    eprintln!("interest: webhook delivery failed: {}", err.without_url());
                }
            }
        });
        Ok(id)
    }
}

// ------------------------------------------------------------------------------------------- origins

/// The site's origins: exact (`https://antibody.example`) or with one `*` (a preview deployment's
/// `https://antibody-*-team.vercel.app`).
#[derive(Clone, Debug, Default)]
pub struct Origins(Vec<String>);

impl Origins {
    pub fn parse(list: &str) -> Self {
        Self(list.split(',').map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty()).collect())
    }

    pub fn allows(&self, origin: &str) -> bool {
        self.0.iter().any(|p| match p.split_once('*') {
            None => p == origin,
            Some((a, b)) => origin.len() >= a.len() + b.len() && origin.starts_with(a) && origin.ends_with(b) && !origin[a.len()..origin.len() - b.len()].contains(['/', '.', ':']),
        })
    }
}

// ---------------------------------------------------------------------------------------- rate limit

/// A token bucket per client: `burst` requests at once, refilled at `rate` per second.
pub struct Limiter {
    burst: f64,
    rate: f64,
    buckets: Mutex<HashMap<IpAddr, (f64, Instant)>>,
}

impl Limiter {
    pub fn new(burst: f64, rate: f64) -> Self {
        Self { burst, rate, buckets: Mutex::new(HashMap::new()) }
    }

    /// Takes `cost` tokens from `ip`'s bucket; `false` when it is empty (the request is refused).
    pub fn allow(&self, ip: IpAddr, cost: f64) -> bool {
        let Ok(mut m) = self.buckets.lock() else { return true };
        let now = Instant::now();
        if m.len() > 20_000 {
            let (burst, rate) = (self.burst, self.rate);
            m.retain(|_, (t, at)| *t + now.duration_since(*at).as_secs_f64() * rate < burst);
        }
        let (tokens, at) = m.entry(ip).or_insert((self.burst, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * self.rate).min(self.burst);
        *at = now;
        if *tokens >= cost {
            *tokens -= cost;
            true
        } else {
            false
        }
    }
}

/// The client's address, as the policy layer found it (a request extension).
#[derive(Clone, Copy, Debug)]
pub struct ClientIp(pub IpAddr);

/// The client's address: the reverse proxy's `X-Forwarded-For` (its last entry, the address the proxy
/// saw) when the connection comes from this host, the peer otherwise.
pub fn client_ip(peer: SocketAddr, forwarded: Option<&str>) -> IpAddr {
    if peer.ip().is_loopback() {
        if let Some(ip) = forwarded.and_then(|f| f.rsplit(',').next()).and_then(|s| s.trim().parse().ok()) {
            return ip;
        }
    }
    peer.ip()
}

// --------------------------------------------------------------------------------------------- cache

struct Slot {
    value: Option<(Instant, Value)>,
    lock: Arc<tokio::sync::Mutex<()>>,
}

/// A short cache in front of the views that read many contracts: a fresh value is served as is, a stale
/// one while a single refresh runs, and only one request computes a missing value. Errors are not kept.
#[derive(Default)]
pub struct Cache {
    slots: Mutex<HashMap<String, Slot>>,
}

impl Cache {
    fn peek(&self, key: &str) -> (Option<(Instant, Value)>, Arc<tokio::sync::Mutex<()>>) {
        let mut m = self.slots.lock().expect("cache lock");
        if m.len() > 4096 {
            m.retain(|_, s| s.value.as_ref().is_some_and(|(t, _)| t.elapsed() < Duration::from_secs(600)));
        }
        let s = m.entry(key.to_string()).or_insert_with(|| Slot { value: None, lock: Arc::new(tokio::sync::Mutex::new(())) });
        (s.value.clone(), s.lock.clone())
    }

    fn put(&self, key: &str, v: Value) {
        if let Some(s) = self.slots.lock().expect("cache lock").get_mut(key) {
            s.value = Some((Instant::now(), v));
        }
    }

    /// The value under `key`: fresh for `ttl`, served stale for `stale` more while one refresh runs.
    pub async fn get<F>(self: &Arc<Self>, key: &str, ttl: Duration, stale: Duration, f: F) -> Result<Value, String>
    where
        F: FnOnce() -> eyre::Result<Value> + Send + 'static,
    {
        let (have, lock) = self.peek(key);
        if let Some((t, v)) = &have {
            if t.elapsed() < ttl {
                return Ok(v.clone());
            }
            if t.elapsed() < ttl + stale {
                if let Ok(guard) = lock.clone().try_lock_owned() {
                    let (me, k) = (self.clone(), key.to_string());
                    tokio::spawn(async move {
                        let _g = guard;
                        if let Ok(Ok(v)) = tokio::task::spawn_blocking(f).await {
                            me.put(&k, v);
                        }
                    });
                }
                return Ok(v.clone());
            }
        }
        let _g = lock.lock().await;
        if let (Some((t, v)), _) = self.peek(key) {
            if t.elapsed() < ttl {
                return Ok(v);
            }
        }
        match tokio::task::spawn_blocking(f).await {
            Ok(Ok(v)) => {
                self.put(key, v.clone());
                Ok(v)
            }
            Ok(Err(e)) => Err(format!("{e:#}")),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// How long a client and a CDN may keep each route's answer.
pub fn cache_control(path: &str) -> &'static str {
    if path == "/health" || path.starts_with("/v1/interest") {
        "no-store"
    } else if path.starts_with("/v1/stream") {
        "no-cache"
    } else if path == "/v1/judge" {
        "public, max-age=60, stale-while-revalidate=600"
    } else if matches!(path, "/v1/moments" | "/v1/trust" | "/v1/mainnet") {
        "public, max-age=60, stale-while-revalidate=300"
    } else {
        "public, max-age=5, stale-while-revalidate=30"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_files_are_named_runs() {
        assert_eq!(moment_kind("testnet-2026-11-04.json"), Some("recorded"));
        assert_eq!(moment_kind("drill-2026-11-20T09.json"), Some("drill"));
        for other in ["testnet-staging-2026-11-03.json", "api-examples.json", "sandbox-2026-11-03.json", "t1-collateral-2026-11-06.json", "testnet-2026-11-04.txt"] {
            assert_eq!(moment_kind(other), None, "{other}");
        }
    }

    #[test]
    fn the_recorded_one_moment_has_every_step_and_transaction() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/results");
        let r: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("testnet-2026-11-04.json")).unwrap()).unwrap();
        let m = moment("testnet-2026-11-04.json", "recorded", &r, &[], &|t| format!("https://testnet.nearblocks.io/txns/{t}")).unwrap();
        let keys: Vec<&str> = m["steps"].as_array().unwrap().iter().map(|s| s["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["verified", "t1", "permits_signed", "paid", "paused_84532", "paused_11155111"]);
        let s = |k: &str| m["steps"].as_array().unwrap().iter().find(|s| s["key"] == k).unwrap().clone();
        assert_eq!((s("verified")["seconds"].as_f64(), s("t1")["seconds"].as_f64(), s("paid")["seconds"].as_f64()), (Some(18.5), Some(29.1), Some(71.0)));
        assert_eq!((s("paused_84532")["seconds"].as_f64(), s("paused_11155111")["seconds"].as_f64()), (Some(87.2), Some(90.6)));
        assert_eq!(s("permits_signed")["seconds"].as_f64(), Some(62.7));
        // The transactions the results file holds (the event log adds the NEAR ones).
        assert_eq!(s("paid")["tx"], "4W4CYBQucfJL8bawGXJZL4ufAKR2A2Se45hBvT7YmtfZ");
        assert_eq!(s("paused_11155111")["tx"], "0x224f6f77ffd081042f634f2a214c1d23d5c232114e35a62e7935981a1ee779e4");
        assert_eq!(m["commit"]["tx"], "CynCsx8PNsFAYnYp6Zk5HjpSg3fWUrHSKfJ9CD9HQ3nP");
        assert_eq!((m["program_id"].as_u64(), m["commit_id"].as_u64()), (Some(5), Some(13)));
    }

    /// A sample drill results file (the shape `antibody-services drill` writes): `moments()` parses it into
    /// a moment with kind "drill", with every step and its transaction.
    fn sample_drill() -> Value {
        let permit = |id: u64, chain: u64, tx: &str, block: &str, ex: &str| {
            json!({"permit_id": id, "chain_id": chain, "kind": "pause", "status": "confirmed", "program_key": "bounty.antibody.testnet/9",
                   "tx_hash": tx, "explorer": ex, "block": block, "targets_paused": [true], "confirmed_ms": 1_793_900_000_000u64, "signed_ns": 1_793_899_950_000_000_000u64,
                   "adapter": "0xad6bc84c8c5a9fb5aca79b13bb867b6e7930d49b"})
        };
        json!({
            "network": "testnet", "all_ok": true, "started_unix": 1_793_900_000u64, "finished_unix": 1_793_900_400u64,
            "program": {"program_id": 9}, "deployment": {"network": "testnet", "near_antibody": "bounty.antibody.testnet"},
            "submissions": [{"agent": "A (MCP)", "commit": 42, "run": 70, "verdict": "Violation", "tier": "T1", "keys": [{"slot": 0, "tag": "cc52"}],
                             "state": {"Done": {"Paid": {"amount": "100000000000000000000000", "instant": true}}}, "paid_raw": "100000000000000000000000",
                             "txs": {"commit": "CommitTxDrill", "reveal": "RevealTxDrill", "pay": "PayTxDrill"}}],
            "one_moment": {"commit": 42, "paid_raw": "100000000000000000000000", "instant": true, "paused_both": true, "pay_tx": "PayTxDrill",
                "clock": "from the commit's NEAR block", "verdict_from": "its event's block",
                "seconds_from_commit": {"verdict": 17.2, "t1": 28.4, "paid": 69.1, "permit_signed_11155111": 60.0, "permit_signed_84532": 60.0, "paused_11155111": 88.0, "paused_84532": 85.0},
                "permits": {"0": permit(6, 11155111, "0xpauseS", "0xb81300", "https://sepolia.etherscan.io/tx/0xpauseS"),
                            "1": permit(7, 84532, "0xpauseB", "0x33e8000", "https://sepolia.basescan.org/tx/0xpauseB")}},
            "drill": {"date": "2026-11-10", "cadence_label": "daily", "steps": {"evm": {"ok": true}, "one-moment": {"ok": true}}},
        })
    }

    #[test]
    fn a_drill_results_file_parses_into_a_drill_moment() {
        let dir = std::env::temp_dir().join(format!("drill-moments-{}", now_s()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("drill-2026-11-10.json"), serde_json::to_vec(&sample_drill()).unwrap()).unwrap();
        let ms = moments(&dir, &[], &|t| format!("https://testnet.nearblocks.io/txns/{t}"));
        let list = ms["moments"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        let m = &list[0];
        assert_eq!((m["id"].as_str(), m["kind"].as_str()), (Some("drill-2026-11-10"), Some("drill")));
        assert_eq!((m["program_id"].as_u64(), m["commit_id"].as_u64(), m["run_id"].as_u64()), (Some(9), Some(42), Some(70)));
        assert_eq!(m["via"], "A (MCP)");
        assert_eq!(m["paused_everywhere"], json!(true));
        let keys: Vec<&str> = m["steps"].as_array().unwrap().iter().map(|s| s["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["verified", "t1", "permits_signed", "paid", "paused_84532", "paused_11155111"]);
        let s = |k: &str| m["steps"].as_array().unwrap().iter().find(|s| s["key"] == k).unwrap().clone();
        assert_eq!(s("paid")["tx"], "PayTxDrill");
        assert_eq!(s("paused_11155111")["tx"], "0xpauseS");
        assert_eq!(m["commit"]["tx"], "CommitTxDrill");
        assert_eq!(ms["latest"], "drill-2026-11-10");
        assert_eq!(m["notes"].as_array().unwrap().len(), 2, "a drill that was not resumed has the two usual notes");
        assert_eq!(m["run_notes"], json!([]), "and nothing of its own");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_resumed_drill_says_its_reveal_came_late() {
        let mut r = sample_drill();
        r["mcp"] = json!({"commit_id": 42, "resumed": true, "revealed_by": "the CLI, after the MCP call stopped"});
        let m = moment("drill-2026-11-10.json", "drill", &r, &[], &|t| format!("https://testnet.nearblocks.io/txns/{t}")).unwrap();
        let notes = m["notes"].as_array().unwrap();
        assert_eq!(notes.len(), 3);
        assert!(notes[0].as_str().unwrap().starts_with("This run's reveal came late"));
        // The run's own note, which the page shows with the run's times.
        assert_eq!(m["run_notes"].as_array().map(|n| n.len()), Some(1));
        assert_eq!(m["run_notes"][0], notes[0]);
    }

    #[test]
    fn the_adapters_replay_keys() {
        // abi.encode("pause", uint32 0, uint64 13): offset, round, ticket, length, the string.
        let enc = abi_tag("pause", 0, {
            let mut w = [0u8; 32];
            w[31] = 13;
            w
        });
        assert_eq!(enc.len(), 160);
        assert_eq!(enc[31], 0x60);
        assert_eq!(enc[127], 5);
        assert_eq!(&enc[128..133], b"pause");
        let pause = permit_used_key(&json!({"Pause": {"ticket": 13}}), 0).unwrap();
        assert_eq!(pause, keccak256(&enc).0);
        let release = permit_used_key(&json!({"Release": {"proxy": "0xca1fafbcfc14d665d76045a018b215987f39f8a7", "code_hash": "0x00"}}), 2).unwrap();
        assert_ne!(pause, release);
        assert_eq!(permit_used_key(&json!({"Release": {"proxy": "0x12"}}), 2), None);
    }

    #[test]
    fn the_form_checks_its_fields() {
        let ok = json!({"role": "project", "name": " Ann ", "contact": "ann@example.org", "project": "example.org", "chains": "Ethereum",
                        "timeline": "Q1", "notes": "line one\nline two", "consent": true, "extra": "dropped"});
        let rec = validate_interest(&ok).unwrap().unwrap();
        assert_eq!(rec["name"], "Ann");
        assert!(rec.get("extra").is_none());
        // The honeypot: accepted, stored nowhere.
        let mut bot = ok.clone();
        bot["website"] = json!("http://spam");
        assert_eq!(validate_interest(&bot).unwrap(), None);
        for (k, bad) in [("role", json!("admin")), ("consent", json!(false)), ("name", json!("")), ("contact", json!("ab")), ("notes", json!("x".repeat(2001))), ("name", json!("a\u{7}b"))] {
            let mut v = ok.clone();
            v[k] = bad;
            assert!(validate_interest(&v).is_err(), "{k}");
        }
        assert!(validate_interest(&json!([1])).is_err());
    }

    #[test]
    fn origins_and_clients() {
        let o = Origins::parse("https://antibody.example, https://antibody-*-team.vercel.app/");
        assert!(o.allows("https://antibody.example"));
        assert!(o.allows("https://antibody-git-main-team.vercel.app"));
        assert!(!o.allows("https://antibody-x.evil.com-team.vercel.app"));
        assert!(!o.allows("https://evil.example"));
        assert!(!Origins::parse("").allows("https://antibody.example"));
        let local: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let remote: SocketAddr = "203.0.113.9:5000".parse().unwrap();
        assert_eq!(client_ip(local, Some("198.51.100.7")), "198.51.100.7".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip(local, Some("10.0.0.1, 198.51.100.7")), "198.51.100.7".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip(remote, Some("198.51.100.7")), remote.ip());
    }

    #[test]
    fn the_limiter_refills() {
        let l = Limiter::new(2.0, 1000.0);
        let ip: IpAddr = "198.51.100.7".parse().unwrap();
        assert!(l.allow(ip, 1.0) && l.allow(ip, 1.0));
        let slow = Limiter::new(1.0, 0.0001);
        assert!(slow.allow(ip, 1.0));
        assert!(!slow.allow(ip, 1.0));
        assert!(slow.allow("198.51.100.8".parse().unwrap(), 1.0));
    }
}

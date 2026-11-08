//! HTTP API (read-only JSON) for UIs, agents and judges (docs/api.md, docs/openapi.yaml): near-antibody's
//! live views, the indexer's event history with transaction links, the services' health and alerts, and
//! the judge view. `/v1/mainnet` explains how mainnet works, with a live read-only check of NEAR
//! mainnet's Ethereum light client (view calls only).

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use antibody_cli::{
    cmd,
    ctx::{Ctx, Deployments},
    near::Near,
};
use axum::{
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

use crate::{
    indexer::{metrics, read_events},
    web,
};

#[derive(Clone)]
pub struct Api {
    pub near: Near,
    pub dep: Deployments,
    pub state_dir: PathBuf,
    /// The deployments file, read again for near-t2 while it names none (a later deployment lights the
    /// T2 views up with no restart).
    pub deployments_file: Option<PathBuf>,
    /// EVM RPCs by chain id, for the adapters' `used` view (whether a permit was applied on its chain).
    pub chains: BTreeMap<u64, String>,
    /// The results files (`docs/results/`): the one moment's runs and the relays they recorded.
    pub results_dir: PathBuf,
    pub web: Arc<WebState>,
}

/// The public API's own state: a short cache, the rate limit and the pilot form.
pub struct WebState {
    pub cache: Arc<web::Cache>,
    pub limiter: web::Limiter,
    pub streams: web::Limiter,
    pub interest: web::Interest,
    t2: Mutex<(Option<Instant>, Option<String>)>,
}

type Shared = Arc<Api>;

/// Every route, for `/health` and the startup line.
pub const ENDPOINTS: &[&str] = &[
    "/health",
    "/v1/status",
    "/v1/stats",
    "/v1/programs",
    "/v1/programs/{id}",
    "/v1/programs/{id}/feed",
    "/v1/commits/{id}",
    "/v1/runs/{id}",
    "/v1/accounts/{id}",
    "/v1/leaderboard",
    "/v1/events",
    "/v1/guardian",
    "/v1/judge",
    "/v1/programs/{id}/reflex",
    "/v1/trust",
    "/v1/practice",
    "/v1/stream",
    "/v1/mainnet",
    "/v1/t2",
    "/v1/moments",
    "/v1/interest",
];

impl Api {
    pub fn new(near: Near, dep: Deployments, state_dir: PathBuf, deployments_file: Option<PathBuf>, chains: BTreeMap<u64, String>, results_dir: PathBuf) -> Self {
        let web = Arc::new(WebState {
            cache: Arc::new(web::Cache::default()),
            // About two requests a second from one client, in bursts of 120; at most 8 live streams opened
            // a minute.
            limiter: web::Limiter::new(120.0, 2.0),
            streams: web::Limiter::new(8.0, 8.0 / 60.0),
            interest: web::Interest::from_env(&state_dir),
            t2: Mutex::new((None, None)),
        });
        Self { near, dep, state_dir, deployments_file, chains, results_dir, web }
    }

    /// near-t2's account: the deployment's, else the deployments file's as it reads now (at most once a
    /// minute), so a near-t2 recorded after the API started shows with no restart.
    fn t2_account(&self) -> Option<String> {
        if self.dep.t2.is_some() {
            return self.dep.t2.clone();
        }
        let path = self.deployments_file.as_ref()?;
        let mut g = self.web.t2.lock().ok()?;
        if g.0.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return g.1.clone();
        }
        let fresh = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Deployments>(&t).ok()).and_then(|d| d.t2);
        *g = (Some(Instant::now()), fresh.clone());
        fresh
    }

    /// A read-only CLI context: no account, no key.
    fn ctx(&self) -> Ctx {
        let mut dep = self.dep.clone();
        dep.t2 = self.t2_account();
        Ctx { near: self.near.clone(), dep, account: None, key_file: None, eth_rpc: None, state_dir: self.state_dir.join("api"), via: None }
    }
}

fn err(code: StatusCode, msg: impl ToString) -> Response {
    (code, Json(json!({"error": msg.to_string()}))).into_response()
}

/// A blocking computation (view calls) behind the short cache: a fresh answer for `ttl` seconds, a stale
/// one for `stale` more while a single refresh runs (docs/api.md: the views read many contracts, and RPCs
/// rate-limit).
async fn cached<F: FnOnce() -> eyre::Result<Value> + Send + 'static>(a: &Api, key: String, ttl: u64, stale: u64, f: F) -> Response {
    match a.web.cache.get(&key, Duration::from_secs(ttl), Duration::from_secs(stale), f).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

fn read_json(path: PathBuf) -> Value {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

fn events_for(a: &Api, pred: impl Fn(&Value) -> bool) -> Vec<Value> {
    read_events(&a.state_dir.join("events.jsonl"), 0, usize::MAX).into_iter().filter(|e| pred(e)).collect()
}

/// A commit as the feed and the leaderboard show it: its state, its verdict (kind and tier, from its
/// run) and its payout. Never the PoC: sealed blobs are not in near-antibody's views.
fn commit_row(ctx: &Ctx, c: &Value) -> Value {
    let verdict = c["run"].as_u64().and_then(|r| cmd::run(ctx, r).ok()).map(|r| cmd::run_view(&r)).unwrap_or(Value::Null);
    let t2 = c["run"].as_u64().map(|r| t2_record(ctx, r)).unwrap_or(Value::Null);
    json!({"commit_id": c["id"], "program_id": c["program"], "index": c["index"], "round": c["round"], "hunter": c["hunter"],
           "beneficiary": c["beneficiary"], "committed_at": c["committed_at"], "state": cmd::state_label(&c["state"]), "run_id": c["run"],
           "verdict": {"kind": verdict["kind"], "tier": verdict["tier"], "keys": verdict["keys"], "t2": t2_badge(&t2)},
           "paid_raw": cmd::paid_of(c).to_string(), "sealed": true})
}

/// A run's near-t2 record (null when the deployment has no near-t2 or the run has no record).
fn t2_record(ctx: &Ctx, run_id: u64) -> Value {
    match &ctx.dep.t2 {
        Some(t) => ctx.near.view(t, "get_record", json!({"run_id": run_id.to_string()})).unwrap_or(Value::Null),
        None => Value::Null,
    }
}

/// What a UI shows for T2: confirmed (a validity proof of the disclosed verdict, checked on NEAR),
/// contradicted, or nothing yet.
fn t2_badge(r: &Value) -> Value {
    match r["outcome"].as_str() {
        Some(o) => json!({"outcome": o, "differ": r["differ"], "receipt_sha256": r["receipt_sha256"], "at": r["at"], "block": r["block"]}),
        None => Value::Null,
    }
}

async fn health(State(a): State<Shared>) -> Response {
    Json(json!({"ok": true, "network": a.near.network, "version": env!("CARGO_PKG_VERSION"), "contracts": a.dep, "endpoints": ENDPOINTS})).into_response()
}

async fn status(State(a): State<Shared>) -> Response {
    let b = a.clone();
    cached(&a, "status".into(), 15, 900, move || {
        let recorded = web::recorded_relays(&b.results_dir);
        crate::status::compute_with(&b.near, &b.dep, &b.state_dir, &b.results_dir, &crate::status::Thresholds::default(), &b.chains, &recorded)
    })
    .await
}

async fn stats(State(a): State<Shared>) -> Response {
    let b = a.clone();
    cached(&a, "stats".into(), 10, 900, move || {
        let stats = b.near.view(&b.dep.antibody, "get_stats", json!({}))?;
        let events = read_events(&b.state_dir.join("events.jsonl"), 0, usize::MAX);
        Ok(json!({"contract": b.dep.antibody, "stats": stats, "latency": metrics(&events), "verdict_latency": crate::indexer::answer_metrics(&events)}))
    })
    .await
}

/// The one moment's runs (docs/results/): the recorded runs, and scheduled drills once a later backend adds
/// them, newest first; each step with its seconds from the commit's NEAR block and its transaction.
async fn moments(State(a): State<Shared>) -> Response {
    let b = a.clone();
    cached(&a, "moments".into(), 60, 600, move || {
        let events = read_events(&b.state_dir.join("events.jsonl"), 0, usize::MAX);
        let near = b.near.clone();
        Ok(web::moments(&b.results_dir, &events, &|t| near.explorer_tx(t)))
    })
    .await
}

/// The pilot and early-access form (`POST /v1/interest`): checked, kept in a private file on this host and
/// announced through the notifier's webhooks. A filled honeypot is answered as accepted and kept nowhere.
async fn interest(State(a): State<Shared>, axum::Extension(web::ClientIp(ip)): axum::Extension<web::ClientIp>, body: axum::body::Bytes) -> Response {
    if body.len() > 8 * 1024 {
        return err(StatusCode::PAYLOAD_TOO_LARGE, "at most 8 KB");
    }
    if !a.web.interest.per_ip.allow(ip, 1.0) || !a.web.interest.global.allow(std::net::IpAddr::from([0, 0, 0, 0]), 1.0) {
        return err(StatusCode::TOO_MANY_REQUESTS, "too many requests: try again in an hour");
    }
    let v: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return err(StatusCode::BAD_REQUEST, "a JSON object"),
    };
    match web::validate_interest(&v) {
        Err(e) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
        Ok(None) => Json(json!({"ok": true})).into_response(),
        Ok(Some(rec)) => match a.web.interest.store(&rec) {
            Ok(id) => Json(json!({"ok": true, "id": id})).into_response(),
            Err(e) => {
                eprintln!("interest: {e:#}");
                err(StatusCode::INTERNAL_SERVER_ERROR, "not stored: try again later")
            }
        },
    }
}

async fn programs(State(a): State<Shared>) -> Response {
    let b = a.clone();
    let a = b.clone();
    cached(&b, "programs".into(), 10, 900, move || {
        let n = a.near.view(&a.dep.antibody, "get_stats", json!({}))?["next_program"].as_u64().unwrap_or(0);
        let mut out = Vec::new();
        for id in 0..n {
            let p = a.near.view(&a.dep.antibody, "get_program", json!({"program_id": id}))?;
            let m = a.near.view(&a.dep.antibody, "get_program_meta", json!({"program_id": id}))?;
            out.push(json!({"program_id": id, "name": m["name"], "project": p["project"], "phase": p["phase"], "round": p["round"],
                            "anchor": {"chain_id": m["chain_id"], "block": m["anchor_block"]}, "commits": p["commits"], "ledger": p["ledger"]}));
        }
        Ok(json!({"programs": out}))
    })
    .await
}

async fn program(State(a): State<Shared>, Path(id): Path<u32>) -> Response {
    let b = a.clone();
    let a = b.clone();
    cached(&b, format!("program:{id}"), 10, 900, move || {
        let ctx = a.ctx();
        let p = ctx.program(id)?;
        let commits: Vec<Value> = cmd::list_commits(&ctx, id)?.iter().map(|c| commit_row(&ctx, c)).collect();
        let events = events_for(&a, |e| e["data"]["program_id"].as_u64() == Some(id as u64));
        let meta = cmd::meta(&ctx, id)?;
        let record = ctx.view("program_record", json!({"program_id": id}))?;
        Ok(json!({"program": p, "rules": rules(&p, &meta, &record), "meta": meta, "record": record,
                  "audit": ctx.view("audit", json!({"program_id": id}))?, "commits": commits, "events": events}))
    })
    .await
}

/// A program's rules as a UI shows them: each slot with its severity and payout, the payout table, how
/// much of the largest payout the available escrow covers, the sealed known issues (a count only),
/// and the anchor's check.
fn rules(p: &Value, meta: &Value, record: &Value) -> Value {
    let amount = |k: &str| p["params"][k].as_str().map(String::from).unwrap_or_else(|| p["params"][k].to_string());
    let slots: Vec<Value> = meta["slots"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| json!({"bit": s[0], "name": s[1], "severity": s[2], "payout_raw": amount(s[2].as_str().unwrap_or("low"))}))
        .collect();
    let num = |v: &Value| v.as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    let largest = ["critical", "high", "medium", "low"].iter().map(|k| num(&p["params"][k])).fold(0.0, f64::max);
    let available = num(&p["ledger"]["available"]);
    json!({"slots": slots,
           "payouts_raw": {"critical": amount("critical"), "high": amount("high"), "medium": amount("medium"), "low": amount("low")},
           "bond_raw": amount("bond"), "instant": {"cap_raw": amount("instant_cap"), "budget_raw": amount("instant_budget"), "window_secs": p["params"]["instant_window"]},
           "escrow_coverage": if largest > 0.0 { json!(available / largest) } else { Value::Null },
           "known_issues_sealed": p["known"].as_array().map(|k| k.len()).unwrap_or(0),
           "anchor": {"chain_id": meta["chain_id"], "block": meta["anchor_block"], "hash": meta["anchor_hash"], "check": record["anchor"]}})
}

/// The reflex and release panel: per chain, whether the program's targets are live, paused or
/// released, with each permit and its transaction there.
async fn reflex(State(a): State<Shared>, Path(id): Path<u32>) -> Response {
    let b = a.clone();
    let a = b.clone();
    cached(&b, format!("reflex:{id}"), 30, 1800, move || {
        let ctx = a.ctx();
        let p = ctx.program(id)?;
        let key = p["key"].clone();
        let g = a.dep.guardian.clone().ok_or_else(|| eyre::eyre!("no guardian configured"))?;
        let guard = a.near.view(&g, "get_program", json!({"program_key": key, "project": p["project"]}))?;
        let permits = Value::Array(crate::relayer::list_all_permits(&a.near, &g)?);
        let relayed = read_json(a.state_dir.join("relayer.json"));
        // A permit's state: this relayer's record, else the relay a recorded run made, else the adapter's
        // own `used` view (a permit carried out by any relayer).
        let recorded = web::recorded_relays(&a.results_dir);
        let mut chains = Vec::new();
        for c in guard["chains"].as_array().into_iter().flatten() {
            let mine: Vec<Value> = permits
                .as_array()
                .into_iter()
                .flatten()
                .filter(|x| x["program_key"] == key && x["chain_id"] == c["chain_id"])
                .map(|x| {
                    let r = relayed.as_object().and_then(|m| m.values().find(|v| v["permit_id"] == x["id"] && v["chain_id"] == c["chain_id"])).cloned().unwrap_or(Value::Null);
                    let rec = x["id"].as_u64().and_then(|i| recorded.get(&(key.as_str().unwrap_or_default().to_string(), i)));
                    let st = web::permit_state(&r, rec, x, &a.chains);
                    json!({"permit_id": x["id"], "round": x["round"], "body": x["body"], "status": st["status"], "tx": st["tx"], "explorer": st["explorer"],
                           "source": st["source"], "signed_ns": x["signed_ns"], "deadline": x["deadline"], "digest": x["digest"]})
                })
                .collect();
            let confirmed = |kind: &str| mine.iter().any(|m| m["body"].get(kind).is_some() && (m["status"] == json!("confirmed") || m["status"] == json!("applied")));
            let state = if confirmed("Release") { "released" } else if confirmed("Pause") { "paused" } else { "live" };
            chains.push(json!({"chain_id": c["chain_id"], "adapter": c["adapter"], "proxies": c["proxies"], "state": state, "permits": mine}));
        }
        Ok(json!({"program_id": id, "guardian_address": guard["address"], "chains": chains}))
    })
    .await
}

/// The trust panel: who is trusted for what, and how anyone checks it.
async fn trust(State(a): State<Shared>) -> Response {
    let d = &a.dep;
    Json(json!({"actors": [
        {"actor": "the light client", "trusted_for": "the anchor block's hash", "account": d.light_client,
         "check": "the program's anchor check (`antibody judge`: anchor status light-client) against any RPC's block hash"},
        {"actor": "the TEE (OutLayer worker)", "trusted_for": "running the pinned verifier on the sealed proof", "account": d.outlayer,
         "check": "T1: the run's TDX quote is verified on NEAR (`antibody tee check --request-id <id>`); the verifier's wasm hash is pinned per program"},
        {"actor": "the verifier", "trusted_for": "nothing beyond its pinned build", "account": null,
         "check": "native and wasm builds agree byte for byte; `antibody check` re-runs any proof locally; after disclosure every verdict can be recomputed"},
        {"actor": "the MPC network", "trusted_for": "signing pause and release permits near-guardian builds", "account": d.guardian,
         "check": "each permit's EIP-712 digest is recomputed by AntibodyAdapter on chain; `antibody judge` lists every permit"},
        {"actor": "near-antibody's owner", "trusted_for": "the defaults of new programs only", "account": d.antibody,
         "check": "a live program's pins never change (property 12); `antibody judge` shows whether the contract is locked"},
        {"actor": "the project", "trusted_for": "its manifest, its escrow and its fixes", "account": null,
         "check": "the manifest hash is committed; known issues are sealed before open; a fix is released only after every paid proof re-runs clean"},
        {"actor": "relayers, keeper, indexer, API", "trusted_for": "nothing (permissionless calls, derived data)", "account": null,
         "check": "every value they show is re-derivable from NEAR views"}
    ]})).into_response()
}

/// The try-it panel: the practice program, its faucet's health and the steps.
async fn practice(State(a): State<Shared>) -> Response {
    let b = a.clone();
    let a = b.clone();
    cached(&b, "practice".into(), 10, 900, move || {
        let pr = a.dep.practice.clone().ok_or_else(|| eyre::eyre!("this deployment names no practice program"))?;
        let balance = a.near.account(&pr.vending)?.map(|x| x["amount"].clone()).unwrap_or(Value::Null);
        Ok(json!({"vending": pr.vending, "program_id": pr.program, "vending_balance_yocto": balance,
                  "public_vend_key": antibody_cli::practice::public_vend_signer(&pr.vending).public_key_str(),
                  "steps": ["antibody try --name <you>: vend a practice account (the public vend key, no account needed)",
                            "the tutorial proof is sealed against the pinned base witness (no RPC) and committed",
                            "it is revealed and verified in the TEE; the verdict is T1-confirmed",
                            "allocate and pay: Done:Paid, with a link for every transaction"],
                  "docs": "docs/practice.md"}))
    })
    .await
}

#[derive(serde::Deserialize)]
struct StreamQ {
    after: Option<u64>,
    /// As for `/v1/events`.
    scope: Option<String>,
}

/// The live timeline as server-sent events: every new indexer event, in order, as JSON.
async fn stream(State(a): State<Shared>, Query(q): Query<StreamQ>) -> axum::response::sse::Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    let path = a.state_dir.join("events.jsonl");
    let start = q.after.unwrap_or(0);
    let scope = q.scope.clone();
    let s = futures_util::stream::unfold((path, start, scope), |(path, after, scope)| async move {
        loop {
            let next = read_events(&path, after, 100);
            if !next.is_empty() {
                let last = next.last().and_then(|e| e["seq"].as_u64()).unwrap_or(after);
                let events: Vec<Result<Event, std::convert::Infallible>> =
                    next.iter().filter(|e| in_scope(e, scope.as_deref())).map(|e| Ok(Event::default().event("antibody").data(e.to_string()))).collect();
                // Only others' events this time: an empty batch, and the cursor moves on.
                return Some((futures_util::stream::iter(events), (path, last + 1, scope)));
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Sse::new(futures_util::StreamExt::flatten(s)).keep_alive(KeepAlive::default())
}

/// The sealed submissions feed: who committed when, and how it settled. Never the PoC.
async fn feed(State(a): State<Shared>, Path(id): Path<u32>) -> Response {
    let b = a.clone();
    let a = b.clone();
    cached(&b, format!("feed:{id}"), 5, 300, move || {
        let ctx = a.ctx();
        let rows: Vec<Value> = cmd::list_commits(&ctx, id)?.iter().map(|c| commit_row(&ctx, c)).collect();
        Ok(json!({"program_id": id, "submissions": rows}))
    })
    .await
}

async fn commit(State(a): State<Shared>, Path(id): Path<u64>) -> Response {
    let b = a.clone();
    let a = b.clone();
    cached(&b, format!("commit:{id}"), 5, 300, move || {
        let ctx = a.ctx();
        let c = ctx.commit(id)?;
        let events = events_for(&a, |e| e["data"]["commit_id"].as_u64() == Some(id));
        let row = commit_row(&ctx, &c);
        // The proof card: the facts and the receipt, and how to check them; a duplicate's own proof stays
        // sealed until disclosure.
        let run = c["run"].as_u64().and_then(|r| cmd::run(&ctx, r).ok()).map(|r| cmd::run_view(&r)).unwrap_or(Value::Null);
        let sealed = !events.iter().any(|e| e["event"] == json!("disclosed"));
        let mut check = vec![format!("antibody status --commit {id}"), "antibody tee check --request-id <the run's OutLayer request>".to_string()];
        // OutLayer's own verifier re-checks the run's quote from the transaction that requested it
        // (real TEE runs only: the sandbox's quotes are MOCK).
        let requested_in = events.iter().find(|e| e["event"] == "run_requested" && e["data"]["run_id"] == c["run"]).and_then(|e| e["tx"].as_str());
        if let (Some(tx), false) = (requested_in, a.near.network == "sandbox") {
            check.push(format!("outlayer-verify tx {tx}"));
        }
        Ok(json!({"commit": c, "row": row, "events": events,
                  "proof_card": {"facts": {"kind": run["kind"], "keys": run["keys"], "impact_usd_micro": run["impact_usd_micro"], "tier": run["tier"]},
                                 "receipt_sha256": run["receipt_sha256"], "sealed_until_disclosure": sealed, "check": check}}))
    })
    .await
}

async fn run(State(a): State<Shared>, Path(id): Path<u64>) -> Response {
    let b = a.clone();
    let a = b.clone();
    cached(&b, format!("run:{id}"), 3, 300, move || {
        let ctx = a.ctx();
        let r = cmd::run(&ctx, id)?;
        let t2 = t2_record(&ctx, id);
        Ok(json!({"run": cmd::run_view(&r), "tier": antibody_cli::zk::tier(&r, (!t2.is_null()).then_some(&t2)), "t2": t2, "raw": r}))
    })
    .await
}

async fn account(State(a): State<Shared>, Path(id): Path<String>) -> Response {
    // A NEAR account id (2 to 64 of a-z, 0-9, ., - and _): anything else is no account, and stays out of
    // the cache.
    if id.len() < 2 || id.len() > 64 || !id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'_')) {
        return err(StatusCode::BAD_REQUEST, "not a NEAR account id");
    }
    let b = a.clone();
    let a = b.clone();
    cached(&b, format!("account:{id}"), 5, 120, move || {
        Ok(json!({"account": id,
                  "credit": a.near.view(&a.dep.antibody, "get_account", json!({"account_id": id}))?,
                  "agent": a.near.view(&a.dep.antibody, "agent_statement", json!({"agent": id}))?,
                  "token_balance_raw": a.near.view(&a.dep.token, "ft_balance_of", json!({"account_id": id}))?}))
    })
    .await
}

/// Paid findings and payouts by beneficiary, across programs; valid-but-late (duplicates) and
/// no-violation counts beside them.
async fn leaderboard(State(a): State<Shared>) -> Response {
    let b = a.clone();
    let a = b.clone();
    cached(&b, "leaderboard".into(), 15, 900, move || {
        let ctx = a.ctx();
        let n = ctx.view("get_stats", json!({}))?["next_program"].as_u64().unwrap_or(0) as u32;
        let mut by: BTreeMap<String, (u128, u32, u32, u32)> = BTreeMap::new();
        for id in 0..n {
            for c in cmd::list_commits(&ctx, id)? {
                let who = c["beneficiary"].as_str().unwrap_or_default().to_string();
                let e = by.entry(who).or_default();
                match cmd::state_label(&c["state"]).as_str() {
                    "Done:Paid" => {
                        e.0 += cmd::paid_of(&c);
                        e.1 += 1;
                    }
                    "Done:DuplicateConfirmed" | "Duplicate" => e.2 += 1,
                    "Done:Forfeited" => e.3 += 1,
                    _ => {}
                }
            }
        }
        let mut rows: Vec<Value> = by
            .into_iter()
            .map(|(k, (paid, findings, dups, forfeited))| json!({"account": k, "paid_raw": paid.to_string(), "paid_findings": findings, "duplicates": dups, "forfeited": forfeited}))
            .collect();
        rows.sort_by(|x, y| y["paid_raw"].as_str().unwrap_or("0").parse::<u128>().unwrap_or(0).cmp(&x["paid_raw"].as_str().unwrap_or("0").parse::<u128>().unwrap_or(0)));
        Ok(json!({"leaderboard": rows}))
    })
    .await
}

#[derive(serde::Deserialize)]
struct EventsQ {
    after: Option<u64>,
    limit: Option<usize>,
    /// `antibody`: only Antibody's own events (standards `antibody*`); the indexer also records the other
    /// events of the transactions it reads (the payout token's, for instance).
    scope: Option<String>,
}

/// Whether an event belongs to `scope` (`antibody`: Antibody's own standards; none: every event).
fn in_scope(e: &Value, scope: Option<&str>) -> bool {
    match scope {
        Some("antibody") => e["standard"].as_str().is_some_and(|s| s.starts_with("antibody")),
        _ => true,
    }
}

async fn events(State(a): State<Shared>, Query(q): Query<EventsQ>) -> Response {
    let limit = q.limit.unwrap_or(200).min(1000);
    let scope = q.scope.as_deref();
    let all = read_events(&a.state_dir.join("events.jsonl"), q.after.unwrap_or(0), usize::MAX);
    let picked: Vec<Value> = all.into_iter().filter(|e| in_scope(e, scope)).take(limit).collect();
    Json(json!({"events": picked})).into_response()
}

async fn guardian(State(a): State<Shared>) -> Response {
    let b = a.clone();
    let a = b.clone();
    cached(&b, "guardian".into(), 15, 900, move || {
        let g = a.dep.guardian.clone().ok_or_else(|| eyre::eyre!("no guardian configured"))?;
        Ok(json!({"guardian": g, "config": a.near.view(&g, "get_config", json!({}))?,
                  "permits": crate::relayer::list_all_permits(&a.near, &g)?,
                  "pending_permits": a.near.view(&g, "pending_permits", json!({}))?,
                  "relayer": read_json(a.state_dir.join("relayer.json"))}))
    })
    .await
}

/// `antibody judge` as JSON: code hashes, locked status, pins, T1 confirmations, payouts, permits.
/// T2 (docs/zk.md): near-t2's pins and counts, every record (newest last), the indexed `t2_*` events,
/// and the prover service's state. A validity proof of a disclosed verdict: no privacy is claimed, and
/// payouts in the locked deployment do not depend on it.
async fn t2(State(a): State<Shared>) -> Response {
    let b = a.clone();
    let a = b.clone();
    cached(&b, "t2".into(), 10, 900, move || {
        let ctx = a.ctx();
        let t = ctx.dep.t2.clone().ok_or_else(|| eyre::eyre!("this deployment names no near-t2 contract yet"))?;
        let records: Vec<Value> = antibody_cli::zk::records(&ctx, &t)?.into_values().collect();
        Ok(json!({"contract": t, "pins": a.near.view(&t, "get_pins", json!({}))?, "stats": a.near.view(&t, "get_stats", json!({}))?,
                  "records": records, "events": events_for(&a, |e| e["standard"] == "antibody-t2"),
                  "prover": read_json(a.state_dir.join("prover.json")),
                  "note": "a validity proof of a disclosed verdict: no privacy is claimed, and payouts in the locked deployment do not depend on it",
                  "docs": "docs/zk.md"}))
    })
    .await
}

async fn judge(State(a): State<Shared>) -> Response {
    // About 90 s of view calls on testnet: computed in the background and served from the cache.
    let b = a.clone();
    cached(&a, "judge".into(), 120, 3600, move || antibody_cli::judge::judge(&b.ctx(), None)).await
}

/// "How mainnet works" (docs/mainnet.md): what a mainnet program changes, and a live read-only check of
/// NEAR mainnet's Ethereum light client against Ethereum. View calls only: nothing is signed or sent.
async fn mainnet(State(a): State<Shared>) -> Response {
    cached(&a, "mainnet".into(), 300, 3600, move || {
        let check = cmd::mainnet_check(None, None).unwrap_or_else(|e| json!({"error": format!("{e:#}")}));
        Ok(json!({
            "title": "How mainnet works",
            "same": ["the contracts and their settlement rules", "the verifier build, pinned by its hash",
                     "sealed proofs, T1 quotes verified on NEAR, rounds and the release gate"],
            "changes": [
                {"part": "anchor", "testnet": "client-eth2.sepolia.testnet (one relayer)",
                 "mainnet": "client-eth2.bridge.near: Ethereum mainnet's finalized chain, from BLS-verified sync-committee updates"},
                {"part": "verification", "testnet": "OutLayer on testnet", "mainnet": "OutLayer's mainnet workers; their TDX quotes verified on NEAR mainnet"},
                {"part": "pauses and releases", "testnet": "the MPC network on testnet, adapters on Sepolia and Base Sepolia",
                 "mainnet": "the MPC network on NEAR mainnet, the program's adapters on Ethereum and other EVM chains"},
                {"part": "payouts", "testnet": "wNEAR (wrap.testnet)", "mainnet": "the NEP-141 token the program commits"},
            ],
            "latency": "a program's anchor must be final on Ethereum (about 13 minutes) and held by the light client, which trails Ethereum by `lag_blocks` below",
            "status": "no mainnet deployment; this page makes view calls only",
            "light_client_check": check,
            "transactions_sent": 0,
        }))
    })
    .await
}

pub fn router(api: Api) -> Router {
    let state = Arc::new(api);
    Router::new()
        .route("/health", get(health))
        .route("/v1/status", get(status))
        .route("/v1/stats", get(stats))
        .route("/v1/programs", get(programs))
        .route("/v1/programs/{id}", get(program))
        .route("/v1/programs/{id}/feed", get(feed))
        .route("/v1/commits/{id}", get(commit))
        .route("/v1/runs/{id}", get(run))
        .route("/v1/accounts/{id}", get(account))
        .route("/v1/leaderboard", get(leaderboard))
        .route("/v1/events", get(events))
        .route("/v1/guardian", get(guardian))
        .route("/v1/judge", get(judge))
        .route("/v1/programs/{id}/reflex", get(reflex))
        .route("/v1/trust", get(trust))
        .route("/v1/practice", get(practice))
        .route("/v1/stream", get(stream))
        .route("/v1/mainnet", get(mainnet))
        .route("/v1/t2", get(t2))
        .route("/v1/moments", get(moments))
        .route("/v1/interest", post(interest).options(|| async { StatusCode::NO_CONTENT }))
        .layer(axum::middleware::from_fn_with_state(state.clone(), policy))
        .with_state(state)
}

/// The public API's HTTP policy (docs/api.md): a rate limit per client, CORS (reads are public, the form
/// takes requests from the site's origins only), and cache headers.
async fn policy(State(a): State<Shared>, mut req: Request, next: Next) -> Response {
    // The peer is known when served with connect info (`serve`); the sandbox demo serves without it.
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0).unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 0)));
    let ip = web::client_ip(peer, req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()));
    req.extensions_mut().insert(web::ClientIp(ip));
    let path = req.uri().path().to_string();
    let origin = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()).map(String::from);
    let form = path == "/v1/interest";
    let site = origin.as_deref().filter(|o| a.web.interest.origins.allows(o)).and_then(|o| HeaderValue::from_str(o).ok());
    let cors = |mut r: Response| -> Response {
        let h = r.headers_mut();
        match (&site, form) {
            (Some(o), true) => {
                h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o.clone());
                h.insert(header::VARY, HeaderValue::from_static("origin"));
            }
            (None, true) => {}
            (_, false) => {
                h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
            }
        }
        r
    };
    if form && req.method() == Method::OPTIONS {
        if site.is_none() {
            return StatusCode::FORBIDDEN.into_response();
        }
        let mut r = cors(StatusCode::NO_CONTENT.into_response());
        let h = r.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("POST, OPTIONS"));
        h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("content-type"));
        h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
        return r;
    }
    if form && req.method() == Method::POST && site.is_none() {
        return err(StatusCode::FORBIDDEN, "this form takes requests from the site only");
    }
    let stream = path.starts_with("/v1/stream");
    let cost = if path == "/v1/judge" { 5.0 } else { 1.0 };
    if (stream && !a.web.streams.allow(ip, 1.0)) || !a.web.limiter.allow(ip, cost) {
        let mut r = err(StatusCode::TOO_MANY_REQUESTS, "rate limited: retry in a few seconds");
        r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("10"));
        r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        return cors(r);
    }
    let mut r = cors(next.run(req).await);
    let ok = r.status() == StatusCode::OK;
    let h = r.headers_mut();
    if stream {
        // Proxies must pass each event on as it comes.
        h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    }
    if !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static(if ok { web::cache_control(&path) } else { "no-store" }));
    }
    r
}

pub async fn serve(api: Api, listen: &str) -> eyre::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    println!("{}", json!({"api": format!("http://{listen}"), "endpoints": ENDPOINTS}));
    // `antibody judge` takes about 90 s of view calls on testnet, the status a few seconds: fill their caches
    // in the background now, so no visitor waits for the first computation.
    let warm = api.clone();
    tokio::spawn(async move {
        let b = warm.clone();
        let _ = warm.web.cache.get("judge", Duration::from_secs(120), Duration::from_secs(3600), move || antibody_cli::judge::judge(&b.ctx(), None)).await;
    });
    let warm = api.clone();
    tokio::spawn(async move {
        let b = warm.clone();
        let _ = warm
            .web
            .cache
            .get("status", Duration::from_secs(15), Duration::from_secs(900), move || {
                let recorded = web::recorded_relays(&b.results_dir);
                crate::status::compute_with(&b.near, &b.dep, &b.state_dir, &b.results_dir, &crate::status::Thresholds::default(), &b.chains, &recorded)
            })
            .await;
    });
    axum::serve(listener, router(api).into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

//! HTTP API (read-only JSON) for UIs, agents and judges (docs/api.md, docs/openapi.yaml): near-antibody's
//! live views, the indexer's event history with transaction links, the services' health and alerts, and
//! the judge view. `/v1/mock/mainnet` serves the recorded MOCK dataset, flagged as such.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use antibody_cli::{
    cmd,
    ctx::{Ctx, Deployments},
    near::Near,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};

use crate::indexer::{metrics, read_events};

#[derive(Clone)]
pub struct Api {
    pub near: Near,
    pub dep: Deployments,
    pub state_dir: PathBuf,
    pub mock_dir: PathBuf,
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
    "/v1/mock/mainnet",
];

impl Api {
    /// A read-only CLI context: no account, no key.
    fn ctx(&self) -> Ctx {
        Ctx { near: self.near.clone(), dep: self.dep.clone(), account: None, key_file: None, eth_rpc: None, state_dir: self.state_dir.join("api") }
    }
}

fn err(code: StatusCode, msg: impl ToString) -> Response {
    (code, Json(json!({"error": msg.to_string()}))).into_response()
}

async fn blocking<F: FnOnce() -> eyre::Result<Value> + Send + 'static>(f: F) -> Response {
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_GATEWAY, format!("{e:#}")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
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
    json!({"commit_id": c["id"], "program_id": c["program"], "index": c["index"], "round": c["round"], "hunter": c["hunter"],
           "beneficiary": c["beneficiary"], "committed_at": c["committed_at"], "state": cmd::state_label(&c["state"]), "run_id": c["run"],
           "verdict": {"kind": verdict["kind"], "tier": verdict["tier"], "keys": verdict["keys"]}, "paid_raw": cmd::paid_of(c).to_string(), "sealed": true})
}

async fn health(State(a): State<Shared>) -> Response {
    Json(json!({"ok": true, "network": a.near.network, "version": env!("CARGO_PKG_VERSION"), "contracts": a.dep, "endpoints": ENDPOINTS})).into_response()
}

async fn status(State(a): State<Shared>) -> Response {
    blocking(move || crate::status::compute(&a.near, &a.dep, &a.state_dir, &crate::status::Thresholds::default())).await
}

async fn stats(State(a): State<Shared>) -> Response {
    blocking(move || {
        let stats = a.near.view(&a.dep.antibody, "get_stats", json!({}))?;
        let events = read_events(&a.state_dir.join("events.jsonl"), 0, usize::MAX);
        Ok(json!({"contract": a.dep.antibody, "stats": stats, "latency": metrics(&events)}))
    })
    .await
}

async fn programs(State(a): State<Shared>) -> Response {
    blocking(move || {
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
    blocking(move || {
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
    blocking(move || {
        let ctx = a.ctx();
        let p = ctx.program(id)?;
        let key = p["key"].clone();
        let g = a.dep.guardian.clone().ok_or_else(|| eyre::eyre!("no guardian configured"))?;
        let guard = a.near.view(&g, "get_program", json!({"program_key": key, "project": p["project"]}))?;
        let permits = a.near.view(&g, "list_permits", json!({"from": 0, "limit": 100}))?;
        let relayed = read_json(a.state_dir.join("relayer.json"));
        let mut chains = Vec::new();
        for c in guard["chains"].as_array().into_iter().flatten() {
            let mine: Vec<Value> = permits
                .as_array()
                .into_iter()
                .flatten()
                .filter(|x| x["program_key"] == key && x["chain_id"] == c["chain_id"])
                .map(|x| {
                    let r = relayed.as_object().and_then(|m| m.values().find(|v| v["permit_id"] == x["id"] && v["chain_id"] == c["chain_id"])).cloned().unwrap_or(Value::Null);
                    json!({"permit_id": x["id"], "round": x["round"], "body": x["body"], "status": r["status"], "tx": r["tx_hash"], "explorer": r["explorer"]})
                })
                .collect();
            let confirmed = |kind: &str| mine.iter().any(|m| m["body"].get(kind).is_some() && m["status"] == json!("confirmed"));
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
    blocking(move || {
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
}

/// The live timeline as server-sent events: every new indexer event, in order, as JSON.
async fn stream(State(a): State<Shared>, Query(q): Query<StreamQ>) -> axum::response::sse::Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    let path = a.state_dir.join("events.jsonl");
    let start = q.after.unwrap_or(0);
    let s = futures_util::stream::unfold((path, start), |(path, after)| async move {
        loop {
            let next = read_events(&path, after, 100);
            if !next.is_empty() {
                let last = next.last().and_then(|e| e["seq"].as_u64()).unwrap_or(after);
                let events: Vec<Result<Event, std::convert::Infallible>> = next.iter().map(|e| Ok(Event::default().event("antibody").data(e.to_string()))).collect();
                return Some((futures_util::stream::iter(events), (path, last + 1)));
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Sse::new(futures_util::StreamExt::flatten(s)).keep_alive(KeepAlive::default())
}

/// The sealed submissions feed: who committed when, and how it settled. Never the PoC.
async fn feed(State(a): State<Shared>, Path(id): Path<u32>) -> Response {
    blocking(move || {
        let ctx = a.ctx();
        let rows: Vec<Value> = cmd::list_commits(&ctx, id)?.iter().map(|c| commit_row(&ctx, c)).collect();
        Ok(json!({"program_id": id, "submissions": rows}))
    })
    .await
}

async fn commit(State(a): State<Shared>, Path(id): Path<u64>) -> Response {
    blocking(move || {
        let ctx = a.ctx();
        let c = ctx.commit(id)?;
        let events = events_for(&a, |e| e["data"]["commit_id"].as_u64() == Some(id));
        let row = commit_row(&ctx, &c);
        // The proof card: the facts and the receipt, and how to check them; a duplicate's own proof stays
        // sealed until disclosure.
        let run = c["run"].as_u64().and_then(|r| cmd::run(&ctx, r).ok()).map(|r| cmd::run_view(&r)).unwrap_or(Value::Null);
        let sealed = !events.iter().any(|e| e["event"] == json!("disclosed"));
        Ok(json!({"commit": c, "row": row, "events": events,
                  "proof_card": {"facts": {"kind": run["kind"], "keys": run["keys"], "impact_usd_micro": run["impact_usd_micro"], "tier": run["tier"]},
                                 "receipt_sha256": run["receipt_sha256"], "sealed_until_disclosure": sealed,
                                 "check": [format!("antibody status --commit {id}"), "antibody tee check --request-id <the run's OutLayer request>".to_string()]}}))
    })
    .await
}

async fn run(State(a): State<Shared>, Path(id): Path<u64>) -> Response {
    blocking(move || {
        let r = cmd::run(&a.ctx(), id)?;
        Ok(json!({"run": cmd::run_view(&r), "raw": r}))
    })
    .await
}

async fn account(State(a): State<Shared>, Path(id): Path<String>) -> Response {
    blocking(move || {
        Ok(json!({"account": id,
                  "credit": a.near.view(&a.dep.antibody, "get_account", json!({"account_id": id}))?,
                  "agent": a.near.view(&a.dep.antibody, "agent_statement", json!({"agent": id}))?,
                  "token_balance_raw": a.near.view(&a.dep.tusd, "ft_balance_of", json!({"account_id": id}))?}))
    })
    .await
}

/// Paid findings and payouts by beneficiary, across programs; valid-but-late (duplicates) and
/// no-violation counts beside them.
async fn leaderboard(State(a): State<Shared>) -> Response {
    blocking(move || {
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
}

async fn events(State(a): State<Shared>, Query(q): Query<EventsQ>) -> Response {
    Json(json!({"events": read_events(&a.state_dir.join("events.jsonl"), q.after.unwrap_or(0), q.limit.unwrap_or(200).min(1000))})).into_response()
}

async fn guardian(State(a): State<Shared>) -> Response {
    blocking(move || {
        let g = a.dep.guardian.clone().ok_or_else(|| eyre::eyre!("no guardian configured"))?;
        Ok(json!({"guardian": g, "config": a.near.view(&g, "get_config", json!({}))?,
                  "permits": a.near.view(&g, "list_permits", json!({"from": 0, "limit": 100}))?,
                  "pending_permits": a.near.view(&g, "pending_permits", json!({}))?,
                  "relayer": read_json(a.state_dir.join("relayer.json"))}))
    })
    .await
}

/// `antibody judge` as JSON: code hashes, locked status, pins, T1 confirmations, payouts, permits.
async fn judge(State(a): State<Shared>) -> Response {
    blocking(move || antibody_cli::judge::judge(&a.ctx(), None)).await
}

async fn mock_mainnet(State(a): State<Shared>) -> Response {
    let mut items = serde_json::Map::new();
    if let Ok(rd) = std::fs::read_dir(&a.mock_dir) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|x| x.to_str()) == Some("json") {
                items.insert(e.file_name().to_string_lossy().trim_end_matches(".json").to_string(), read_json(e.path()));
            }
        }
    }
    Json(json!({"mock": true, "notice": "MOCK: recorded data, not live. No mainnet transactions are sent.", "data": items})).into_response()
}

pub fn router(api: Api) -> Router {
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
        .route("/v1/mock/mainnet", get(mock_mainnet))
        .layer(axum::middleware::map_response(|mut r: Response| async move {
            r.headers_mut().insert("access-control-allow-origin", HeaderValue::from_static("*"));
            r
        }))
        .with_state(Arc::new(api))
}

pub async fn serve(api: Api, listen: &str) -> eyre::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    println!("{}", json!({"api": format!("http://{listen}"), "endpoints": ENDPOINTS}));
    axum::serve(listener, router(api)).await?;
    Ok(())
}

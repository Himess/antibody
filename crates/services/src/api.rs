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
        Ok(json!({"program": p, "meta": cmd::meta(&ctx, id)?, "record": ctx.view("program_record", json!({"program_id": id}))?,
                  "audit": ctx.view("audit", json!({"program_id": id}))?, "commits": commits, "events": events}))
    })
    .await
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
        Ok(json!({"commit": c, "row": commit_row(&ctx, &c), "events": events, "check": format!("antibody status --commit {id}")}))
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

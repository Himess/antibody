//! HTTP API (read-only JSON) for UIs, agents and judges. Live contract views plus the indexer's
//! event history (tx links), metrics and the relayer's pause status. `/v1/mock/mainnet` serves the
//! recorded MOCK dataset, clearly flagged.

use std::{path::PathBuf, sync::Arc};

use antibody_cli::{ctx::Deployments, near::Near};
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

async fn health(State(a): State<Shared>) -> Response {
    Json(json!({"ok": true, "network": a.near.network, "contracts": a.dep})).into_response()
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
    blocking(move || a.near.view(&a.dep.antibody, "list_programs", json!({"from": 0, "limit": 50}))).await
}

fn events_for(a: &Api, pred: impl Fn(&Value) -> bool) -> Vec<Value> {
    read_events(&a.state_dir.join("events.jsonl"), 0, usize::MAX).into_iter().filter(|e| pred(e)).collect()
}

async fn program(State(a): State<Shared>, Path(id): Path<u64>) -> Response {
    blocking(move || {
        let p = a.near.view(&a.dep.antibody, "get_program", json!({"program_id": id}))?;
        if p.is_null() {
            return Err(eyre::eyre!("no program {id}"));
        }
        let keys = a.near.view(&a.dep.antibody, "get_keys", json!({"program_id": id}))?;
        let commits = a.near.view(&a.dep.antibody, "list_commits", json!({"program_id": id, "from": 0, "limit": 100}))?;
        let events = events_for(&a, |e| e["data"]["program_id"].as_u64() == Some(id));
        Ok(json!({"program": p, "claimed_keys": keys, "commits": commits, "events": events}))
    })
    .await
}

/// The sealed submissions feed: who committed what, when, and how it settled — never the PoC.
async fn feed(State(a): State<Shared>, Path(id): Path<u64>) -> Response {
    blocking(move || {
        let commits = a.near.view(&a.dep.antibody, "list_commits", json!({"program_id": id, "from": 0, "limit": 100}))?;
        let rows: Vec<Value> = commits
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| json!({"commit_id": c["id"], "hunter": c["hunter"], "known_issue": c["known_issue"], "height": c["height"],
                            "commit_hash": c["commit_hash"], "status": c["status"], "kind": c["verdict"]["kind"],
                            "broken_mask": c["verdict"]["broken_mask"], "keys": c["verdict"]["keys"], "outcome": c["outcome"],
                            "receipt_hash": c["verdict"]["receipt_hash"], "sealed": true}))
            .collect();
        Ok(json!({"program_id": id, "submissions": rows}))
    })
    .await
}

async fn commit(State(a): State<Shared>, Path(id): Path<u64>) -> Response {
    blocking(move || {
        let c = a.near.view(&a.dep.antibody, "get_commit", json!({"commit_id": id}))?;
        if c.is_null() {
            return Err(eyre::eyre!("no commit {id}"));
        }
        let proof = a.near.view(&a.dep.antibody, "get_duplicate_proof", json!({"commit_id": id}))?;
        let events = events_for(&a, |e| e["data"]["commit_id"].as_u64() == Some(id));
        Ok(json!({"commit": c, "duplicate_proof": proof, "events": events,
                  "verify_locally": format!("antibody verify --commit {id}")}))
    })
    .await
}

async fn leaderboard(State(a): State<Shared>) -> Response {
    blocking(move || a.near.view(&a.dep.antibody, "leaderboard", json!({"limit": 50}))).await
}

async fn account(State(a): State<Shared>, Path(id): Path<String>) -> Response {
    blocking(move || {
        Ok(json!({"account": id,
                  "internal_balance": a.near.view(&a.dep.antibody, "get_balance", json!({"account_id": id}))?,
                  "reputation": a.near.view(&a.dep.antibody, "get_reputation", json!({"account_id": id}))?,
                  "tusd": a.near.view(&a.dep.tusd, "ft_balance_of", json!({"account_id": id}))?}))
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
        let chains = a.near.view(&g, "get_chains", json!({}))?;
        let mut latest = Vec::new();
        for c in chains.as_array().into_iter().flatten() {
            latest.push(a.near.view(&g, "get_latest", json!({"chain_id": c["chain_id"]}))?);
        }
        Ok(json!({"guardian": g, "config": a.near.view(&g, "get_config", json!({}))?, "latest_signed": latest,
                  "relayer": read_json(a.state_dir.join("relayer.json"))}))
    })
    .await
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
        .route("/v1/stats", get(stats))
        .route("/v1/programs", get(programs))
        .route("/v1/programs/{id}", get(program))
        .route("/v1/programs/{id}/feed", get(feed))
        .route("/v1/commits/{id}", get(commit))
        .route("/v1/leaderboard", get(leaderboard))
        .route("/v1/accounts/{id}", get(account))
        .route("/v1/events", get(events))
        .route("/v1/guardian", get(guardian))
        .route("/v1/mock/mainnet", get(mock_mainnet))
        .layer(axum::middleware::map_response(|mut r: Response| async move {
            r.headers_mut().insert("access-control-allow-origin", HeaderValue::from_static("*"));
            r
        }))
        .with_state(Arc::new(api))
}

pub async fn serve(api: Api, listen: &str) -> eyre::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    println!("{}", json!({"api": format!("http://{listen}"), "endpoints": ["/health", "/v1/stats", "/v1/programs", "/v1/programs/{id}", "/v1/programs/{id}/feed", "/v1/commits/{id}", "/v1/leaderboard", "/v1/accounts/{id}", "/v1/events", "/v1/guardian", "/v1/mock/mainnet"]}));
    axum::serve(listener, router(api)).await?;
    Ok(())
}

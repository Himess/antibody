//! T1 relayer (permissionless). For each run OutLayer answered that is not T1 yet:
//! 1. find its attestation: on testnet, the OutLayer request in the transaction that carried the run's
//!    `run_requested` event (indexer events), then OutLayer's published quote; in the sandbox, the MOCK
//!    attestation `mock-outlayer` recorded;
//! 2. relay it to near-antibody (`confirm_tee`).
//! near-antibody repeats every check itself, so the relayer is trusted for nothing (docs/trust.md).

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use antibody_cli::{ctx::Ctx, tee};
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::indexer::read_events;

pub enum Source {
    /// OutLayer's attestation API (testnet).
    OutLayer { api: String },
    /// The sandbox's `mock-outlayer` (MOCK quotes, accepted only by `mock-dcap`).
    Mock { outlayer: String },
}

pub struct T1Relayer {
    pub ctx: Ctx,
    pub source: Source,
    pub state_dir: PathBuf,
    /// Give up on a run after this many failed relays (e.g. no attestation published).
    pub max_attempts: u32,
    /// Where the DCAP collateral comes from: "auto" (OutLayer's on-chain copies while they verify, then
    /// fresh Intel PCS collateral once they expire), "outlayer" or "pcs".
    pub collateral: String,
}

/// Relays near-antibody rejected on chain before the relayer gives up on a run (each costs a DCAP
/// verification's gas, about 0.02 NEAR on testnet).
const MAX_REJECTIONS: u64 = 3;

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Seconds before retrying after `attempts` failed relays: every 10 s for two minutes (an attestation
/// OutLayer publishes late, a rate-limited RPC; a practice try pays T0 at the window meanwhile), then doubling up to an hour.
pub fn backoff(attempts: u32) -> u64 {
    if attempts < 12 {
        10
    } else {
        (10u64 << (attempts - 12).min(9)).min(3_600)
    }
}

/// Whether a run needs T1: answered at T0 with anything but an infrastructure error.
pub fn wants_t1(r: &Value) -> bool {
    let a = &r["outcome"]["Answered"];
    a["tier"] == json!("T0") && a["facts"]["kind"] != json!("InfraError")
}

impl T1Relayer {
    fn state_path(&self) -> PathBuf {
        self.state_dir.join("t1.json")
    }

    pub fn load_state(&self) -> BTreeMap<String, Value> {
        std::fs::read(self.state_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// The OutLayer request behind a run, from the transaction that requested it.
    fn request_id(&self, run_id: u64) -> Result<u64> {
        let events = read_events(&self.state_dir.join("events.jsonl"), 0, usize::MAX);
        let e = events
            .iter()
            .find(|e| e["event"] == "run_requested" && e["data"]["run_id"].as_u64() == Some(run_id))
            .ok_or_else(|| eyre!("run {run_id}: no run_requested event indexed yet"))?;
        let (Some(tx), Some(signer)) = (e["tx"].as_str(), e["signer"].as_str()) else {
            return Err(eyre!("run {run_id}: the indexed event has no transaction"));
        };
        tee::request_id_from_tx(&self.ctx, tx, signer)
    }

    fn relay(&self, run_id: u64) -> Result<tee::Relay> {
        match &self.source {
            Source::Mock { outlayer } => tee::confirm_mock(&self.ctx, outlayer, run_id).map(tee::Relay::Sent),
            Source::OutLayer { api } => tee::confirm_or_wait(&self.ctx, api, run_id, self.request_id(run_id)?, &self.collateral),
        }
    }

    /// One pass over the runs not yet settled; returns what was attempted.
    pub fn tick(&self) -> Result<Vec<Value>> {
        let next = self.ctx.view("get_stats", json!({}))?["next_run"].as_u64().unwrap_or(0);
        let mut state = self.load_state();
        let from = state.get("_from").and_then(|v| v.as_u64()).unwrap_or(0);
        let mut out = Vec::new();
        let mut lowest_open = None;
        for run_id in from..next {
            let label = run_id.to_string();
            let prev = state.get(&label).cloned().unwrap_or_default();
            let r = self.ctx.view("get_run", json!({"run_id": run_id}))?;
            if r["outcome"] == json!("Pending") {
                lowest_open.get_or_insert(run_id);
                continue;
            }
            let attempts = prev["attempts"].as_u64().unwrap_or(0) as u32;
            // The chain decides: a run that is T1 no longer wants T1, whatever this file remembers. A
            // failure recorded for it is settled (another relayer, or a manual `antibody tee confirm`).
            if !wants_t1(&r) {
                if !prev.is_null() && prev["confirmed"] != json!(true) && prev.get("settled").is_none() {
                    let mut row = prev;
                    row["settled"] = json!(true);
                    state.insert(label, row);
                }
                continue;
            }
            if attempts >= self.max_attempts || prev["given_up"] == json!(true) {
                continue;
            }
            lowest_open.get_or_insert(run_id);
            let now = now_s();
            // Backing off after a failed relay, or waiting for the worker image to count: not yet.
            if prev["next_at"].as_u64().is_some_and(|t| t > now) {
                continue;
            }
            let mut rejections = prev["rejections"].as_u64().unwrap_or(0);
            let mut row = match self.relay(run_id) {
                Ok(tee::Relay::Sent(v)) => {
                    rejections += u64::from(!v["rejected"].is_null());
                    json!({"attempts": attempts + 1, "confirmed": v["confirmed"], "rejected": v["rejected"], "rejections": rejections,
                           "request_id": v["request_id"], "collateral": v["collateral"], "tx": v["tx"]})
                }
                // Nothing sent: near-antibody would reject the worker image for now (no gas, no attempt).
                Ok(tee::Relay::Wait { until, why }) => {
                    json!({"attempts": attempts, "confirmed": false, "waiting": why, "next_at": until.map_or(now + 600, |t| t.max(now) + 5)})
                }
                Err(e) => json!({"attempts": attempts + 1, "confirmed": false, "rejections": rejections, "error": e.to_string()}),
            };
            if row.get("waiting").is_none() && row["confirmed"] != json!(true) {
                // A rejection on chain costs a DCAP verification's gas: few retries, far apart. A failure
                // before sending (RPC, attestation not published yet) costs nothing: retried soon, then
                // less often. `/v1/status` raises a critical alert for a run the relayer gave up on.
                if attempts + 1 >= self.max_attempts || rejections >= MAX_REJECTIONS {
                    row["given_up"] = json!(true);
                } else {
                    row["next_at"] = json!(now + if rejections > 0 { 1_800 * rejections } else { backoff(attempts + 1) });
                }
            }
            out.push(json!({"run": run_id, "result": row}));
            state.insert(label, row);
        }
        state.insert("_from".into(), json!(lowest_open.unwrap_or(next)));
        state.insert("_collateral".into(), json!(self.collateral));
        std::fs::create_dir_all(&self.state_dir)?;
        std::fs::write(self.state_path(), serde_json::to_vec_pretty(&state)?)?;
        Ok(out)
    }

    pub fn run(&self, interval_ms: u64) -> Result<()> {
        loop {
            match self.tick() {
                Ok(rows) => {
                    for r in rows {
                        println!("{r}");
                    }
                }
                Err(e) => eprintln!("t1 relayer: {e}"),
            }
            std::thread::sleep(Duration::from_millis(interval_ms));
        }
    }
}

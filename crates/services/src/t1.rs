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

    fn relay(&self, run_id: u64) -> Result<Value> {
        match &self.source {
            Source::Mock { outlayer } => tee::confirm_mock(&self.ctx, outlayer, run_id),
            Source::OutLayer { api } => tee::confirm(&self.ctx, api, run_id, self.request_id(run_id)?, "auto"),
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
            if !wants_t1(&r) || prev["confirmed"] == json!(true) || attempts >= self.max_attempts {
                continue;
            }
            lowest_open.get_or_insert(run_id);
            let row = match self.relay(run_id) {
                Ok(v) => json!({"attempts": attempts + 1, "confirmed": v["confirmed"], "rejected": v["rejected"], "request_id": v["request_id"], "tx": v["tx"]}),
                Err(e) => json!({"attempts": attempts + 1, "confirmed": false, "error": e.to_string()}),
            };
            out.push(json!({"run": run_id, "result": row}));
            state.insert(label, row);
        }
        state.insert("_from".into(), json!(lowest_open.unwrap_or(next)));
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

//! The T2 prover service (docs/zk.md): every practice try's verdict, proven and checked on NEAR.
//!
//! A practice try's proof is the published tutorial and its content key is public by construction
//! (`antibody_cli::blob::practice_content_key`), so nothing has to be handed over. For each answered run
//! of the practice program that near-t2 has no record of, it:
//! 1. gathers the run's inputs: the request from the transaction the indexer saw request it, the base and
//!    the blob from FastFS, all checked against the chain's hashes (`antibody_cli::zk::gather`);
//! 2. proves it on SP1's prover network (`antibody-zk-prove`, Groth16);
//! 3. submits the proof to near-t2, which verifies it and compares it with near-antibody's records.
//!
//! Budget guards: at most `max_per_hour` and `max_per_day` proofs, and an alert when the requester's
//! PROVE balance runs low. Its state is `prover.json`, shown in `/v1/status` and `/v1/t2`.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use antibody_cli::{ctx::Ctx, zk};
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::indexer::read_events;

pub struct Prover {
    /// Signs near-t2 submissions (each prepays 0.02 NEAR of storage, mostly refunded).
    pub ctx: Ctx,
    pub t2: String,
    /// `antibody-zk-prove` and the pinned guest ELF.
    pub bin: PathBuf,
    pub elf: PathBuf,
    /// The requester's key file (never printed).
    pub key_file: PathBuf,
    /// Where the indexer writes `events.jsonl`; the service writes `prover.json` and its case directories.
    pub state_dir: PathBuf,
    pub max_per_hour: u32,
    pub max_per_day: u32,
    /// Runs requested before this time are not proven (the service starts with new tries).
    pub since: u64,
    /// Alert below this PROVE balance.
    pub min_balance: f64,
}

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// How often the requester's balance is read.
const BALANCE_SECS: u64 = 600;
static LAST_BALANCE: AtomicU64 = AtomicU64::new(0);
/// A run is given up after this many failed attempts.
const MAX_ATTEMPTS: u64 = 3;

impl Prover {
    fn state_file(&self) -> PathBuf {
        self.state_dir.join("prover.json")
    }

    fn load(&self) -> Value {
        std::fs::read(self.state_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({"runs": {}}))
    }

    fn save(&self, s: &Value) -> Result<()> {
        std::fs::write(self.state_file(), serde_json::to_vec_pretty(s)?)?;
        Ok(())
    }

    /// The requester's PROVE balance (`antibody-zk-prove balance`).
    fn balance(&self) -> Result<f64> {
        let out = Command::new(&self.bin).args(["balance", "--key-file"]).arg(&self.key_file).output()?;
        let line = String::from_utf8_lossy(&out.stdout).lines().rev().find(|l| l.starts_with('{')).map(String::from).ok_or_else(|| eyre!("no balance"))?;
        let v: Value = serde_json::from_str(&line)?;
        v["balance_prove"].as_f64().ok_or_else(|| eyre!("no balance in {line}"))
    }

    /// Practice runs the indexer saw requested (run id → the requesting transaction), since `since`.
    fn practice_runs(&self) -> Vec<(u64, String)> {
        let Some(pr) = self.ctx.dep.practice.as_ref() else { return vec![] };
        // The vending contract's current program (its owner may point it at a new one).
        let pp = antibody_cli::practice::vending_program(&self.ctx.near, &pr.vending).unwrap_or(pr.program) as u64;
        let mut out = BTreeMap::new();
        for e in read_events(&self.state_dir.join("events.jsonl"), 0, usize::MAX) {
            if e["event"] != "run_requested" || e["data"]["program_id"].as_u64() != Some(pp) || e["data"]["mode"] != json!("verify") {
                continue;
            }
            if e["block_ts_ns"].as_u64().unwrap_or(0) / 1_000_000_000 < self.since {
                continue;
            }
            if let (Some(run), Some(tx)) = (e["data"]["run_id"].as_u64(), e["tx"].as_str()) {
                out.insert(run, tx.to_string());
            }
        }
        out.into_iter().collect()
    }

    /// One pass: at most one proof (a proof takes minutes; the next pass takes the next run).
    pub fn tick(&self) -> Result<Value> {
        let mut s = self.load();
        let now = now_s();
        if now >= LAST_BALANCE.load(Ordering::Relaxed) + BALANCE_SECS {
            LAST_BALANCE.store(now, Ordering::Relaxed);
            s["balance"] = match self.balance() {
                Ok(b) => json!({"prove": b, "at": now, "low": b < self.min_balance, "min": self.min_balance}),
                Err(e) => json!({"error": format!("{e:#}"), "at": now}),
            };
        }
        // Budget: proofs started in the last hour and day.
        let started: Vec<u64> = s["runs"].as_object().into_iter().flatten().filter_map(|(_, r)| r["started_at"].as_u64()).collect();
        let (hour, day) = (started.iter().filter(|t| now - **t < 3_600).count() as u32, started.iter().filter(|t| now - **t < 86_400).count() as u32);
        s["budget"] = json!({"per_hour": self.max_per_hour, "per_day": self.max_per_day, "last_hour": hour, "last_day": day,
                             "capped": hour >= self.max_per_hour || day >= self.max_per_day});
        s["at"] = json!(now);
        let todo = self.practice_runs();
        s["seen"] = json!(todo.len());
        let mut picked = None;
        for (run, tx) in &todo {
            let r = &s["runs"][run.to_string()];
            if r["status"] == "submitted" || r["status"] == "recorded" || r["attempts"].as_u64().unwrap_or(0) >= MAX_ATTEMPTS {
                continue;
            }
            // near-t2 already has it (someone else submitted): nothing to do.
            if let Ok(rec) = self.ctx.near.view(&self.t2, "get_record", json!({"run_id": run.to_string()})) {
                if !rec.is_null() {
                    s["runs"][run.to_string()] = json!({"status": "recorded", "outcome": rec["outcome"], "by": rec["submitter"]});
                    continue;
                }
            }
            // Only answered runs (the verdict exists).
            let answered = antibody_cli::cmd::run(&self.ctx, *run).map(|x| x["outcome"].get("Answered").is_some()).unwrap_or(false);
            if answered {
                picked = Some((*run, tx.clone()));
                break;
            }
        }
        let Some((run, tx)) = picked else {
            self.save(&s)?;
            return Ok(s);
        };
        if s["budget"]["capped"] == json!(true) || s["balance"]["low"] == json!(true) {
            s["runs"][run.to_string()]["status"] = json!("waiting: budget or balance");
            self.save(&s)?;
            return Ok(s);
        }
        let attempts = s["runs"][run.to_string()]["attempts"].as_u64().unwrap_or(0) + 1;
        s["runs"][run.to_string()] = json!({"status": "proving", "started_at": now, "attempts": attempts, "tx": tx});
        self.save(&s)?;
        let dir = self.state_dir.join("zk").join(run.to_string());
        let result = (|| -> Result<Value> {
            let gathered = zk::gather(&self.ctx, run, None, &zk::TxFrom::Given(tx.clone()), &dir)?;
            let proof = zk::prove(&dir, &self.elf, &self.bin, "network", Some(&self.key_file))?;
            let proved_at = now_s();
            let submitted = zk::submit(&self.ctx, &self.t2, run, &dir)?;
            Ok(json!({"gathered": {"receipt_equals_the_tees": gathered["receipt_equals_the_tees"], "content_key_from": gathered["content_key_from"]},
                      "proof": proof["proof"], "proved_at": proved_at, "submit_tx": submitted["tx"]["tx"],
                      "outcome": submitted["record"]["outcome"], "record": submitted["record"]}))
        })();
        let entry = &mut s["runs"][run.to_string()];
        match result {
            Ok(r) => {
                entry["status"] = json!("submitted");
                entry["done_at"] = json!(now_s());
                entry["seconds"] = json!(now_s() - now);
                for (k, v) in r.as_object().into_iter().flatten() {
                    entry[k] = v.clone();
                }
            }
            Err(e) => {
                entry["status"] = json!("failed");
                entry["error"] = json!(format!("{e:#}").chars().take(400).collect::<String>());
            }
        }
        self.save(&s)?;
        Ok(s)
    }

    pub fn run(&self, interval_ms: u64) {
        loop {
            if let Err(e) = self.tick() {
                eprintln!("t2 prover: {e:#}");
            }
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        }
    }
}

/// `/v1/status`'s view of the prover: its alerts (level, code, message) and its component.
pub fn status(state: &Value) -> (Vec<(&'static str, String, String)>, Value) {
    let mut alerts = vec![];
    if state["balance"]["low"] == json!(true) {
        alerts.push(("warn", "t2-prover-balance".to_string(), format!("the T2 prover's PROVE balance is {} (below {}): proofs pause", state["balance"]["prove"], state["balance"]["min"])));
    }
    if state["budget"]["capped"] == json!(true) {
        alerts.push(("info", "t2-prover-budget".to_string(), format!("the T2 prover reached its budget ({} in the last hour, {} today)", state["budget"]["last_hour"], state["budget"]["last_day"])));
    }
    for (run, r) in state["runs"].as_object().into_iter().flatten() {
        if r["status"] == "failed" {
            alerts.push(("warn", "t2-prover-failed".to_string(), format!("run {run}: {}", r["error"].as_str().unwrap_or("-"))));
        }
        if r["outcome"] == "contradicts" {
            alerts.push(("critical", "t2-contradicts".to_string(), format!("near-t2 recorded a contradiction for run {run}: the TEE's receipt is not the verifier's")));
        }
    }
    let proven = state["runs"].as_object().map(|m| m.values().filter(|r| r["status"] == "submitted").count()).unwrap_or(0);
    (alerts, json!({"at": state["at"], "proven": proven, "balance": state["balance"], "budget": state["budget"], "seen": state["seen"]}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_provers_alerts() {
        let s = json!({"at": 1, "balance": {"prove": 0.5, "low": true, "min": 2.0}, "budget": {"capped": true, "last_hour": 6, "last_day": 9},
                       "runs": {"60": {"status": "failed", "error": "network: insufficient balance"}, "61": {"status": "submitted", "outcome": "confirmed"},
                                "62": {"status": "submitted", "outcome": "contradicts"}}});
        let (alerts, c) = status(&s);
        let keys: Vec<&str> = alerts.iter().map(|a| a.1.as_str()).collect();
        assert_eq!(keys, ["t2-prover-balance", "t2-prover-budget", "t2-prover-failed", "t2-contradicts"]);
        assert_eq!(alerts[3].0, "critical", "a contradiction is critical");
        assert_eq!(c["proven"], 2);
        assert!(status(&json!({"runs": {}})).0.is_empty());
    }
}

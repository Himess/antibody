//! T1 relayer (permissionless). For each verdict, and each completed re-run, that has no T1
//! confirmation yet:
//! 1. find the OutLayer request behind it, from the transaction that asked for it (indexer events);
//! 2. fetch that run's TDX quote;
//! 3. relay it to near-antibody (`confirm_tee`).
//! near-antibody repeats every check itself, so the relayer is trusted for nothing (docs/trust.md).

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use antibody_cli::{ctx::Ctx, tee};
use eyre::Result;
use serde_json::{json, Value};

use crate::indexer::read_events;

pub struct T1Relayer {
    pub ctx: Ctx,
    pub api: String,
    pub state_dir: PathBuf,
    /// Give up on a run after this many failed relays (e.g. no attestation published).
    pub max_attempts: u32,
}

impl T1Relayer {
    fn state_path(&self) -> PathBuf {
        self.state_dir.join("t1.json")
    }

    pub fn load_state(&self) -> BTreeMap<String, Value> {
        std::fs::read(self.state_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// One pass over the indexed events; returns what was attempted.
    pub fn tick(&self) -> Result<Vec<Value>> {
        let events = read_events(&self.state_dir.join("events.jsonl"), 0, usize::MAX);
        // (commit, round) -> (requesting tx, its signer), and the runs that have a result.
        let mut requested: BTreeMap<(u64, Option<u64>), (String, String)> = BTreeMap::new();
        let mut finished: Vec<(u64, Option<u64>)> = Vec::new();
        let mut confirmed: Vec<(u64, Option<u64>)> = Vec::new();
        for e in &events {
            let d = &e["data"];
            let (Some(tx), Some(signer)) = (e["tx"].as_str(), e["signer"].as_str()) else { continue };
            let id = d["commit_id"].as_u64();
            match (e["event"].as_str(), id) {
                (Some("submitted" | "known_issue_submitted"), Some(c)) => {
                    requested.insert((c, None), (tx.into(), signer.into()));
                }
                (Some("rerun_requested"), Some(c)) => {
                    requested.insert((c, d["round"].as_u64()), (tx.into(), signer.into()));
                }
                (Some("verdict"), Some(c)) => finished.push((c, None)),
                (Some("rerun_result"), Some(c)) => finished.push((c, d["round"].as_u64())),
                (Some("tee_confirmed"), Some(c)) => confirmed.push((c, d["round"].as_u64())),
                _ => {}
            }
        }
        let mut state = self.load_state();
        let mut out = Vec::new();
        for key in finished {
            if confirmed.contains(&key) {
                continue;
            }
            let label = format!("{}:{}", key.0, key.1.map(|r| r.to_string()).unwrap_or_else(|| "-".into()));
            let attempts = state.get(&label).and_then(|v| v["attempts"].as_u64()).unwrap_or(0) as u32;
            if attempts >= self.max_attempts || state.get(&label).map(|v| v["confirmed"] == json!(true)).unwrap_or(false) {
                continue;
            }
            let Some((tx, signer)) = requested.get(&key) else { continue };
            let res = tee::request_id_from_tx(&self.ctx, tx, signer)
                .and_then(|rid| tee::confirm(&self.ctx, &self.api, key.0, key.1.map(|r| r as u32), rid, "auto"));
            let row = match res {
                Ok(v) => json!({"attempts": attempts + 1, "confirmed": v["confirmed"], "request_id": v["request_id"], "tx": v["tx"]}),
                Err(e) => json!({"attempts": attempts + 1, "confirmed": false, "error": e.to_string()}),
            };
            out.push(json!({"run": label, "result": row}));
            state.insert(label, row);
        }
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

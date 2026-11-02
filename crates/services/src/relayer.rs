//! Relayer: broadcasts the guardian's signed `pause()` transactions to every registered chain and
//! confirms `paused() == true`. Relaying is permissionless — the signed transactions are public on
//! NEAR — so this service holds no keys; anyone can run it (or press "broadcast" in a UI).

use std::{collections::BTreeMap, path::PathBuf};

use antibody_cli::near::Near;
use eyre::Result;
use serde_json::{json, Value};

use crate::evm::Evm;

pub struct Relayer {
    pub near: Near,
    pub guardian: String,
    /// chain id → (RPC URL, explorer tx base URL)
    pub chains: BTreeMap<u64, (String, String)>,
    pub state_file: PathBuf,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Relayer {
    pub fn load_state(&self) -> BTreeMap<String, Value> {
        std::fs::read_to_string(&self.state_file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save_state(&self, st: &BTreeMap<String, Value>) -> Result<()> {
        if let Some(d) = self.state_file.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&self.state_file, serde_json::to_vec_pretty(st)?)?;
        Ok(())
    }

    /// One pass: broadcast new signed pauses, track receipts and `paused()`. Returns the state.
    pub fn tick(&self) -> Result<BTreeMap<String, Value>> {
        let mut st = self.load_state();
        let targets = self.near.view(&self.guardian, "get_chains", json!({}))?;
        for t in targets.as_array().into_iter().flatten() {
            let chain_id = t["chain_id"].as_u64().unwrap_or_default();
            let Some((rpc, explorer)) = self.chains.get(&chain_id) else { continue };
            let latest = self.near.view(&self.guardian, "get_latest", json!({"chain_id": chain_id}))?;
            if latest.is_null() {
                continue;
            }
            let evm = Evm::new(rpc);
            let tx_hash = latest["tx_hash"].as_str().unwrap_or_default().to_string();
            let key = format!("{chain_id}");
            let mut entry = st.get(&key).cloned().unwrap_or(json!({}));
            if entry["tx_hash"] != json!(tx_hash) {
                entry = json!({"chain_id": chain_id, "name": t["name"], "target": t["to"], "request_id": latest["request_id"],
                               "nonce": latest["nonce"], "tx_hash": tx_hash, "explorer": format!("{explorer}{tx_hash}"),
                               "signed_at_near_block": latest["signed_at_block"], "reason": latest["reason"],
                               "first_seen_ms": now_ms(), "status": "signed"});
            }
            if entry["status"] == "signed" {
                match evm.send_raw(latest["signed_tx"].as_str().unwrap_or_default()) {
                    Ok(_) => {
                        entry["status"] = "broadcast".into();
                        entry["broadcast_ms"] = now_ms().into();
                    }
                    Err(e) => {
                        let s = e.to_string();
                        // Someone else relayed it first, or it is already mined: both are fine.
                        if s.contains("already known") || s.contains("nonce too low") || s.contains("AlreadyKnown") {
                            entry["status"] = "broadcast".into();
                            entry["broadcast_ms"] = now_ms().into();
                        } else {
                            entry["last_error"] = s.into();
                        }
                    }
                }
            }
            if entry["status"] == "broadcast" {
                if let Some(r) = evm.receipt(&tx_hash)? {
                    entry["status"] = if r["status"] == "0x1" { "confirmed" } else { "reverted" }.into();
                    entry["block"] = r["blockNumber"].clone();
                    entry["gas_used"] = r["gasUsed"].clone();
                    entry["confirmed_ms"] = now_ms().into();
                }
            }
            if let Some(to) = t["to"].as_str() {
                if let Ok(p) = evm.paused(to) {
                    entry["paused"] = p.into();
                }
            }
            st.insert(key, entry);
        }
        self.save_state(&st)?;
        Ok(st)
    }

    pub fn run(&self, interval_ms: u64) -> Result<()> {
        loop {
            match self.tick() {
                Ok(st) => {
                    for (k, v) in &st {
                        if v["status"] == "confirmed" && v["logged"].is_null() {
                            println!("{}", json!({"relayer": "pause confirmed", "chain_id": k, "tx": v["tx_hash"], "paused": v["paused"]}));
                        }
                    }
                }
                Err(e) => eprintln!("relayer: {e:#}"),
            }
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        }
    }
}

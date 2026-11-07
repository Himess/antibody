//! Indexer: rebuilds the full NEP-297 event history of the Antibody contracts (with transaction
//! hashes, receipt heights and times) and keeps a state snapshot. Two sources:
//! - `fastnear`: FastNEAR's transaction API (per-account history + full receipts), for testnet/mainnet;
//! - `blocks`: block-by-block scan through any NEAR RPC (used on sandbox; works anywhere).
//! Output: `events.jsonl` (append-only, `seq` ordered), `snapshot.json`, `metrics.json`.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::PathBuf,
};

use antibody_cli::near::Near;
use eyre::{eyre, Result};
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub enum Source {
    FastNear { base: String },
    Blocks { from: u64 },
}

pub struct Indexer {
    pub near: Near,
    pub contracts: Vec<String>,
    pub antibody: String,
    pub guardian: Option<String>,
    pub source: Source,
    pub state_dir: PathBuf,
}

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A transaction's receipt tree may still grow this long after its newest receipt: OutLayer answers a
/// run by resuming the request's receipt, some seconds to minutes after the transaction that asked.
const SETTLE_SECS: u64 = 600;

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Cursor {
    /// Transactions whose receipts are all read (their tree settled).
    seen_txs: BTreeSet<String>,
    /// Transactions read but maybe not finished: read again each pass, with their signer.
    #[serde(default)]
    open_txs: BTreeMap<String, String>,
    /// Every event appended, by receipt and position, so a transaction read again (or an indexer
    /// restarted) appends each event once.
    #[serde(default)]
    emitted: BTreeSet<String>,
    last_block: u64,
    next_seq: u64,
}

/// Whether a transaction's receipts are all in: its newest receipt is older than [`SETTLE_SECS`].
fn settled(receipts: &[(u64, u64, String, Vec<String>)]) -> bool {
    let newest_s = receipts.iter().map(|r| r.1 / 1_000_000_000).max().unwrap_or(0);
    now_s().saturating_sub(newest_s) > SETTLE_SECS
}

/// Events from one transaction's receipts: (block height, block ns, receipt id, its position among the
/// receipt's events, event JSON).
fn events_of_receipts(receipts: &[(u64, u64, String, Vec<String>)]) -> Vec<(u64, u64, String, usize, Value)> {
    let mut out = Vec::new();
    for (h, ts, id, logs) in receipts {
        let events = logs.iter().filter_map(|l| l.strip_prefix("EVENT_JSON:")).filter_map(|j| serde_json::from_str::<Value>(j).ok());
        for (n, v) in events.enumerate() {
            out.push((*h, *ts, id.clone(), n, v));
        }
    }
    out.sort_by_key(|e| (e.0, e.1));
    out
}

impl Indexer {
    fn path(&self, f: &str) -> PathBuf {
        self.state_dir.join(f)
    }

    fn load_cursor(&self) -> Cursor {
        std::fs::read_to_string(self.path("cursor.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn http(&self) -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(60)).build().expect("client")
    }

    /// (tx hash, signer, receipts) for every new transaction touching the watched contracts.
    #[allow(clippy::type_complexity)]
    fn new_transactions(&self, cur: &mut Cursor) -> Result<Vec<(String, String, Vec<(u64, u64, String, Vec<String>)>)>> {
        match &self.source {
            Source::FastNear { base } => {
                // Transactions whose trees may still grow are read again.
                let mut hashes: BTreeSet<String> = cur.open_txs.keys().cloned().collect();
                for c in &self.contracts {
                    let v: Value = self.http().post(format!("{base}/v0/account")).json(&json!({"account_id": c})).send()?.json()?;
                    for t in v["account_txs"].as_array().into_iter().flatten() {
                        if let Some(h) = t["transaction_hash"].as_str() {
                            if !cur.seen_txs.contains(h) {
                                hashes.insert(h.to_string());
                            }
                        }
                    }
                }
                let mut out = Vec::new();
                let list: Vec<String> = hashes.into_iter().collect();
                for chunk in list.chunks(20) {
                    let v: Value = self.http().post(format!("{base}/v0/transactions")).json(&json!({"tx_hashes": chunk})).send()?.json()?;
                    for t in v["transactions"].as_array().into_iter().flatten() {
                        let hash = t["transaction"]["hash"].as_str().unwrap_or_default().to_string();
                        let signer = t["transaction"]["signer_id"].as_str().unwrap_or_default().to_string();
                        let mut receipts = Vec::new();
                        let txo = &t["execution_outcome"];
                        receipts.push((
                            txo["block_height"].as_u64().unwrap_or(0),
                            txo["block_timestamp"].as_u64().unwrap_or(0),
                            txo["id"].as_str().unwrap_or_default().to_string(),
                            txo["outcome"]["logs"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)).collect(),
                        ));
                        for r in t["receipts"].as_array().into_iter().flatten() {
                            let eo = &r["execution_outcome"];
                            receipts.push((
                                eo["block_height"].as_u64().unwrap_or(0),
                                eo["block_timestamp"].as_u64().unwrap_or(0),
                                eo["id"].as_str().unwrap_or_default().to_string(),
                                eo["outcome"]["logs"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)).collect(),
                            ));
                        }
                        // An empty receipt list is still executing: read again next pass.
                        if t["receipts"].as_array().map(|a| !a.is_empty()).unwrap_or(false) {
                            out.push((hash, signer, receipts));
                        } else {
                            cur.open_txs.insert(hash, signer);
                        }
                    }
                }
                Ok(out)
            }
            Source::Blocks { from } => {
                let final_h = self.near.call("block", json!({"finality": "final"}))?["header"]["height"].as_u64().ok_or_else(|| eyre!("height"))?;
                let start = cur.last_block.max(from.saturating_sub(1)) + 1;
                let mut out = Vec::new();
                let mut heights: BTreeMap<String, (u64, u64)> = BTreeMap::new();
                for h in start..=final_h.min(start + 400) {
                    // A height with no block (skipped, or garbage-collected by the node) is an answer:
                    // no retry.
                    let b = match self.near.call_once("block", json!({"block_id": h})) {
                        Ok(b) => b,
                        Err(e) if e.to_string().contains("UNKNOWN_BLOCK") => continue,
                        Err(e) => return Err(e),
                    };
                    for ch in b["chunks"].as_array().into_iter().flatten() {
                        if ch["tx_root"].as_str() == Some("11111111111111111111111111111111") {
                            continue;
                        }
                        let c = self.near.call("chunk", json!({"chunk_id": ch["chunk_hash"]}))?;
                        for tx in c["transactions"].as_array().into_iter().flatten() {
                            let recv = tx["receiver_id"].as_str().unwrap_or_default();
                            if !self.contracts.iter().any(|x| x == recv) {
                                continue;
                            }
                            let hash = tx["hash"].as_str().unwrap_or_default().to_string();
                            let signer = tx["signer_id"].as_str().unwrap_or_default().to_string();
                            if cur.seen_txs.contains(&hash) {
                                continue;
                            }
                            let st = self.near.call("tx", json!({"tx_hash": hash, "sender_account_id": signer, "wait_until": "FINAL"}))?;
                            let mut receipts = Vec::new();
                            let mut push = |o: &Value, near: &Near| -> Result<()> {
                                let bh = o["block_hash"].as_str().unwrap_or_default().to_string();
                                let (height, ts) = match heights.get(&bh) {
                                    Some(x) => *x,
                                    None => {
                                        let hb = near.call("block", json!({"block_id": bh}))?;
                                        let x = (hb["header"]["height"].as_u64().unwrap_or(0), hb["header"]["timestamp"].as_u64().unwrap_or(0));
                                        heights.insert(bh, x);
                                        x
                                    }
                                };
                                receipts.push((height, ts, o["id"].as_str().unwrap_or_default().to_string(),
                                    o["outcome"]["logs"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)).collect()));
                                Ok(())
                            };
                            push(&st["transaction_outcome"], &self.near)?;
                            for r in st["receipts_outcome"].as_array().into_iter().flatten() {
                                push(r, &self.near)?;
                            }
                            out.push((hash, signer, receipts));
                        }
                    }
                    cur.last_block = h;
                }
                Ok(out)
            }
        }
    }

    /// One pass: append new events, refresh the snapshot and the metrics.
    pub fn tick(&self) -> Result<Vec<Value>> {
        std::fs::create_dir_all(&self.state_dir)?;
        let mut cur = self.load_cursor();
        let txs = self.new_transactions(&mut cur)?;
        let mut new_events = Vec::new();
        let mut all: Vec<(u64, u64, String, String, String, Value)> = Vec::new();
        // FastNEAR lists a transaction as soon as its first receipts ran; one whose tree is not settled
        // stays open and is read again (the blocks source reads each one when final).
        let reread = matches!(self.source, Source::FastNear { .. });
        for (hash, signer, receipts) in txs {
            for (h, ts, rid, n, ev) in events_of_receipts(&receipts) {
                if cur.emitted.insert(format!("{rid}:{n}")) {
                    all.push((h, ts, rid, hash.clone(), signer.clone(), ev));
                }
            }
            if reread && !settled(&receipts) {
                cur.open_txs.insert(hash, signer);
            } else {
                cur.open_txs.remove(&hash);
                cur.seen_txs.insert(hash);
            }
        }
        all.sort_by_key(|e| (e.0, e.1));
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.path("events.jsonl"))?;
        for (h, ts, rid, hash, signer, ev) in all {
            let e = json!({"seq": cur.next_seq, "network": self.near.network, "standard": ev["standard"], "event": ev["event"],
                           "data": ev["data"][0], "tx": hash, "signer": signer, "receipt": rid, "block_height": h,
                           "block_ts_ns": ts, "explorer": self.near.explorer_tx(&hash)});
            writeln!(f, "{e}")?;
            cur.next_seq += 1;
            new_events.push(e);
        }
        std::fs::write(self.path("cursor.json"), serde_json::to_vec(&cur)?)?;
        self.snapshot()?;
        Ok(new_events)
    }

    pub fn snapshot(&self) -> Result<Value> {
        let stats = self.near.view(&self.antibody, "get_stats", json!({}))?;
        let programs = self.near.view(&self.antibody, "list_programs", json!({"from": 0, "limit": 50}))?;
        let mut progs = Vec::new();
        for p in programs.as_array().into_iter().flatten() {
            let id = p["id"].as_u64().unwrap_or(0);
            let commits = self.near.view(&self.antibody, "list_commits", json!({"program_id": id, "from": 0, "limit": 50}))?;
            let record = self.near.view(&self.antibody, "program_record", json!({"program_id": id}))?;
            progs.push(json!({"program": p, "record": record, "commits": commits}));
        }
        let guardian = match &self.guardian {
            Some(g) => json!({"config": self.near.view(g, "get_config", json!({}))?, "pending_permits": self.near.view(g, "pending_permits", json!({}))?}),
            None => Value::Null,
        };
        let snap = json!({"taken_at": now_s(), "stats": stats, "programs": progs, "guardian": guardian});
        std::fs::write(self.path("snapshot.json"), serde_json::to_vec_pretty(&snap)?)?;
        std::fs::write(self.path("metrics.json"), serde_json::to_vec_pretty(&metrics(&read_events(&self.path("events.jsonl"), 0, usize::MAX)))?)?;
        Ok(snap)
    }

    pub fn run(&self, interval_ms: u64) -> Result<()> {
        loop {
            match self.tick() {
                Ok(evs) => {
                    for e in evs {
                        println!("{}", json!({"event": e["event"], "seq": e["seq"], "tx": e["tx"]}));
                    }
                }
                Err(e) => eprintln!("indexer: {e:#}"),
            }
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        }
    }
}

pub fn read_events(path: &std::path::Path, after: u64, limit: usize) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["seq"].as_u64().map(|s| s >= after).unwrap_or(false))
        .take(limit)
        .collect()
}

fn pct(mut v: Vec<f64>, p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    Some(v[i])
}

/// The events that settle a commit's verification (its verdict, as near-antibody applied it).
pub const VERDICT_EVENTS: &[&str] = &["holder", "duplicate", "known_match", "forfeited", "gap_claim", "unevaluable", "run_failed"];

/// Verdict latency from the event log: from each commit's block to its verify run's answer (the TEE's
/// verdict, `run_answered`), in seconds and blocks. `metrics` measures to the settlement events instead,
/// which wait for the program's earlier commits (claims go in commit order).
pub fn answer_metrics(events: &[Value]) -> Value {
    let mut committed: BTreeMap<u64, (u64, u64)> = BTreeMap::new();
    let mut secs = Vec::new();
    let mut blocks = Vec::new();
    let mut seen = BTreeSet::new();
    for e in events.iter().filter(|e| e["standard"] == "antibody") {
        let Some(c) = e["data"]["commit_id"].as_u64() else { continue };
        let (ts, h) = (e["block_ts_ns"].as_u64().unwrap_or(0), e["block_height"].as_u64().unwrap_or(0));
        match e["event"].as_str() {
            Some("commit") => {
                committed.insert(c, (ts, h));
            }
            Some("run_answered") if e["data"]["mode"] == "verify" && seen.insert(c) => {
                if let Some((t0, h0)) = committed.get(&c) {
                    secs.push((ts as f64 - *t0 as f64) / 1e9);
                    blocks.push(h.saturating_sub(*h0) as f64);
                }
            }
            _ => {}
        }
    }
    json!({"commit_to_answer_s": {"n": secs.len(), "p50": pct(secs.clone(), 0.5), "p95": pct(secs, 0.95)},
           "commit_to_answer_blocks": {"p50": pct(blocks.clone(), 0.5), "p95": pct(blocks, 0.95)},
           "note": "from each commit's NEAR block to its verify run's answer (the TEE's verdict)"})
}

/// Latencies from the event log: commit → verdict and commit → payout, per commit.
pub fn metrics(events: &[Value]) -> Value {
    let mut submitted: BTreeMap<u64, u64> = BTreeMap::new();
    let mut verdict: BTreeMap<u64, u64> = BTreeMap::new();
    let mut paid: BTreeMap<u64, u64> = BTreeMap::new();
    let mut verdict_blocks: Vec<f64> = Vec::new();
    let mut submit_block: BTreeMap<u64, u64> = BTreeMap::new();
    for e in events {
        let c = e["data"]["commit_id"].as_u64();
        let ts = e["block_ts_ns"].as_u64().unwrap_or(0);
        match (e["event"].as_str(), c) {
            (Some("commit"), Some(c)) => {
                submitted.insert(c, ts);
                submit_block.insert(c, e["block_height"].as_u64().unwrap_or(0));
            }
            (Some(v), Some(c)) if VERDICT_EVENTS.contains(&v) && !verdict.contains_key(&c) => {
                verdict.insert(c, ts);
                if let Some(b) = submit_block.get(&c) {
                    verdict_blocks.push((e["block_height"].as_u64().unwrap_or(0) - b) as f64);
                }
            }
            (Some("paid"), Some(c)) => {
                paid.insert(c, ts);
            }
            _ => {}
        }
    }
    let secs = |a: &BTreeMap<u64, u64>| -> Vec<f64> {
        a.iter().filter_map(|(c, t)| submitted.get(c).map(|s| (*t as f64 - *s as f64) / 1e9)).collect()
    };
    let to_verdict = secs(&verdict);
    let to_paid = secs(&paid);
    json!({
        "events": events.len(),
        "submit_to_verdict_s": {"n": to_verdict.len(), "p50": pct(to_verdict.clone(), 0.5), "p95": pct(to_verdict, 0.95)},
        "submit_to_verdict_blocks": {"p50": pct(verdict_blocks.clone(), 0.5), "p95": pct(verdict_blocks, 0.95)},
        "submit_to_payout_s": {"n": to_paid.len(), "p50": pct(to_paid.clone(), 0.5), "p95": pct(to_paid, 0.95)},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_that_may_still_grow_stays_open_and_each_event_is_numbered_in_its_receipt() {
        let ev = |e: &str| format!("EVENT_JSON:{{\"standard\":\"antibody\",\"version\":\"1\",\"event\":\"{e}\",\"data\":[{{}}]}}");
        let ns = |s: u64| s * 1_000_000_000;
        let fresh = vec![(10, ns(now_s() - 5), "r1".to_string(), vec![ev("reveal"), "a plain log".into(), ev("run_requested")])];
        let got = events_of_receipts(&fresh);
        assert_eq!(got.iter().map(|e| (e.2.as_str(), e.3, e.4["event"].as_str().unwrap())).collect::<Vec<_>>(),
                   vec![("r1", 0, "reveal"), ("r1", 1, "run_requested")]);
        // OutLayer's answer resumes the request's receipt later: the tree is open until it settles.
        assert!(!settled(&fresh));
        assert!(settled(&[(10, ns(now_s() - SETTLE_SECS - 1), "r1".to_string(), vec![])]));
    }
}

//! Notifier: turns new indexer events into human-readable notices and posts them to webhooks
//! (Discord/Slack-compatible JSON). Webhook URLs come from the environment and are never printed.

use std::path::PathBuf;

use eyre::Result;
use serde_json::{json, Value};

use crate::indexer::read_events;

pub struct Notifier {
    pub state_dir: PathBuf,
    pub webhooks: Vec<String>,
    pub program: Option<u64>,
}

pub fn message(e: &Value) -> Option<String> {
    let d = &e["data"];
    let link = e["explorer"].as_str().unwrap_or_default();
    let s = match e["event"].as_str()? {
        "program_created" => format!("Program #{} created by {}: escrow {} (raw tUSD), anchor chain {} block {}", d["program_id"], d["project"].as_str().unwrap_or_default(), d["escrow"].as_str().unwrap_or_default(), d["chain_id"], d["anchor_block"]),
        "anchor_checked" => format!("Program #{} anchor: {}", d["program_id"], d["trust"].as_str().unwrap_or_default()),
        "program_opened" => format!("Program #{} is open for submissions", d["program_id"]),
        "known_issue_claimed" => format!("Program #{}: known issue claimed at t0 (commit #{})", d["program_id"], d["commit_id"]),
        "committed" => format!("Commit #{} on program #{} by {} (bond locked)", d["commit_id"], d["program_id"], d["hunter"].as_str().unwrap_or_default()),
        "submitted" => format!("Commit #{} revealed; verification requested", d["commit_id"]),
        "verdict" => format!("Commit #{}: {} mask {} impact {} micro-USD", d["commit_id"], d["kind"].as_str().unwrap_or_default(), d["broken_mask"].as_str().unwrap_or_default(), d["impact_usd_micro"].as_str().unwrap_or_default()),
        "paid" => format!("Paid {} raw tUSD to {} for commit #{} ({})", d["amount"].as_str().unwrap_or_default(), d["hunter"].as_str().unwrap_or_default(), d["commit_id"], d["severity"].as_str().unwrap_or_default()),
        "payout_delayed" => format!("Commit #{}: payout {} delayed for the challenge window", d["commit_id"], d["amount"].as_str().unwrap_or_default()),
        "duplicate" => format!("Commit #{}: duplicate of #{} (no payout, bond refunded)", d["commit_id"], d["of"]),
        "known" => format!("Commit #{}: matches the project's known issue #{} (no payout, bond refunded)", d["commit_id"], d["of"]),
        "challenged" => format!("Commit #{}: payout challenged by {}", d["commit_id"], d["by"].as_str().unwrap_or_default()),
        "pause_requested" => format!("Program #{}: verified Critical (commit #{}) — guardian pause requested", d["program_id"], d["commit_id"]),
        "pause_signed" => format!("Guardian: pause() signed for chain {} (EVM tx {})", d["chain_id"], d["tx_hash"].as_str().unwrap_or_default()),
        "disclosed" => format!("Program #{}: disclosure published ({} content keys)", d["program_id"], d["keys"]),
        _ => return None,
    };
    Some(format!("{s} — {link}"))
}

impl Notifier {
    pub fn tick(&self) -> Result<usize> {
        let cursor_file = self.state_dir.join("notifier.cursor");
        let after: u64 = std::fs::read_to_string(&cursor_file).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let events = read_events(&self.state_dir.join("events.jsonl"), after, 200);
        let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(15)).build()?;
        let mut sent = 0;
        let mut next = after;
        for e in &events {
            next = e["seq"].as_u64().unwrap_or(next) + 1;
            if let Some(p) = self.program {
                if e["data"]["program_id"].as_u64().is_some_and(|x| x != p) {
                    continue;
                }
            }
            let Some(msg) = message(e) else { continue };
            println!("{msg}");
            for url in &self.webhooks {
                if let Err(err) = client.post(url).json(&json!({"content": msg, "text": msg})).send() {
                    eprintln!("notifier: webhook delivery failed: {}", err.without_url());
                }
            }
            sent += 1;
        }
        std::fs::write(&cursor_file, next.to_string())?;
        Ok(sent)
    }

    pub fn run(&self, interval_ms: u64) -> Result<()> {
        loop {
            if let Err(e) = self.tick() {
                eprintln!("notifier: {e:#}");
            }
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        }
    }
}

//! Notifier: turns new indexer events into human-readable notices and posts them to webhooks
//! (Discord/Slack-compatible JSON). Webhook URLs come from the environment and are never printed.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use eyre::Result;
use serde_json::{json, Value};

use crate::indexer::read_events;

/// How often the monitoring alerts are read and posted.
const ALERTS_EVERY_SECS: u64 = 60;

pub struct Notifier {
    pub state_dir: PathBuf,
    /// The results directory (`docs/results/`): the drill files the status reads.
    pub results_dir: PathBuf,
    pub webhooks: Vec<String>,
    pub program: Option<u64>,
    /// Where to read the monitoring alerts from (`/v1/status`); none: events only.
    pub status: Option<(antibody_cli::near::Near, antibody_cli::ctx::Deployments)>,
    /// EVM RPCs by chain id, as the API's status reads them: the EVM balances and the adapters' `used`.
    pub chains: std::collections::BTreeMap<u64, String>,
}

pub fn message(e: &Value) -> Option<String> {
    let d = &e["data"];
    let link = e["explorer"].as_str().unwrap_or_default();
    let t = |k: &str| d[k].as_str().map(String::from).unwrap_or_else(|| d[k].to_string());
    let s = match e["event"].as_str()? {
        "program_created" => format!("Program #{} created by {}", d["program_id"], t("project")),
        "program_escrowed" => format!("Program #{}: escrow {} (raw units)", d["program_id"], t("amount")),
        "anchor_checked" => format!("Program #{}: anchor checked by the light client", d["program_id"]),
        "program_activated" => format!("Program #{}: base witness pinned, sealing key T1-confirmed", d["program_id"]),
        "program_opened" => format!("Program #{} is open for submissions", d["program_id"]),
        "program_rejected" => format!("Program #{} rejected: {}", d["program_id"], t("reason")),
        "commit" => format!("Commit #{} on program #{} by {} (bond locked, sealed)", d["commit_id"], d["program_id"], t("hunter")),
        "reveal" => format!("Commit #{} revealed: verification run #{} started", d["commit_id"], d["run_id"]),
        "holder" => format!("Commit #{} holds its key: a claim of {} (raw units)", d["commit_id"], t("amount")),
        "paid" => format!("Paid {} (raw units) to {} for commit #{}", t("amount"), t("beneficiary"), d["commit_id"]),
        "duplicate" => format!("Commit #{}: a duplicate of an earlier holder (its proof is recorded)", d["commit_id"]),
        "duplicate_confirmed" => format!("Commit #{}: confirmed a duplicate at the fix", d["commit_id"]),
        "known_match" => format!("Commit #{}: matches a known issue the project declared", d["commit_id"]),
        "forfeited" => format!("Commit #{}: no violation; the bond goes to the program's escrow", d["commit_id"]),
        "gap_claim" => format!("Commit #{}: harm to a protected account that no invariant catches; bond refunded, claim recorded", d["commit_id"]),
        "gap_promoted" | "promoted" => format!("Commit #{}: promoted to a paid claim", d["commit_id"]),
        "challenge" => format!("Commit #{}: verdict challenged", d["commit_id"]),
        "challenge_upheld" | "challenge_overturned" => format!("Commit #{}: challenge {}", d["commit_id"], e["event"].as_str().unwrap_or_default().trim_start_matches("challenge_")),
        "pause_ticket" => format!("Program #{}: a pause ticket for commit #{}: the guardian signs pause permits", d["program_id"], d["commit_id"]),
        "permit_requested" => format!("near-guardian asked to sign permits for program #{}", d["program_id"]),
        "round_proposed" => format!("Program #{}: round {} proposed; the TEE checks the fix", d["program_id"], d["round"]),
        "round_rejected" => format!("Program #{}: round {} rejected", d["program_id"], d["round"]),
        "rerun_requested" => format!("Commit #{}: re-run at the proposed fix", d["commit_id"]),
        "round_opened" => format!("Program #{}: round {} open", d["program_id"], d["round"]),
        "disclosed" => format!("Program #{}: disclosure published", d["program_id"]),
        "run_failed" => format!("Run #{} failed: {}", d["run_id"], t("reason")),
        "retry_available" => format!("Commit #{}: the run failed on infrastructure; the hunter may retry", d["commit_id"]),
        "withdraw" => format!("{} withdrew {} (raw units)", t("account"), t("amount")),
        "program_exited" => format!("Program #{} exited; the rest of the escrow returns to the project", d["program_id"]),
        "containment" => format!("Containment: agent {} refused on program #{}: {}", t("agent"), d["program_id"], t("reason")),
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
        // The alerts read a dozen views (every program, the practice faucet): once a minute is enough, and
        // keeps the services under a public RPC's rate limit.
        static LAST_ALERTS: AtomicU64 = AtomicU64::new(0);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if now >= LAST_ALERTS.load(Ordering::Relaxed) + ALERTS_EVERY_SECS {
            LAST_ALERTS.store(now, Ordering::Relaxed);
            sent += self.alerts(&client)?;
        }
        Ok(sent)
    }

    /// Monitoring alerts (docs/api.md, `/v1/status`): each new one is posted once, and again only after it
    /// cleared.
    fn alerts(&self, client: &reqwest::blocking::Client) -> Result<usize> {
        let Some((near, dep)) = &self.status else { return Ok(0) };
        // As the API computes it: with the chains and the recorded relays, so the EVM balance alerts fire and
        // a permit another run carried out does not count as pending.
        let recorded = crate::web::recorded_relays(&self.results_dir);
        let status = crate::status::compute_with(near, dep, &self.state_dir, &self.results_dir, &crate::status::Thresholds::default(), &self.chains, &recorded)?;
        let seen_file = self.state_dir.join("notifier.alerts.json");
        let seen: Vec<String> = std::fs::read_to_string(&seen_file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let now: Vec<(String, String)> = status["alerts"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| (a["key"].as_str().unwrap_or_default().to_string(), format!("[{}] {}", a["level"].as_str().unwrap_or_default(), a["message"].as_str().unwrap_or_default())))
            .collect();
        let mut sent = 0;
        for (key, msg) in &now {
            if seen.contains(key) {
                continue;
            }
            println!("{msg}");
            for url in &self.webhooks {
                if let Err(err) = client.post(url).json(&json!({"content": msg, "text": msg})).send() {
                    eprintln!("notifier: webhook delivery failed: {}", err.without_url());
                }
            }
            sent += 1;
        }
        std::fs::write(&seen_file, serde_json::to_vec(&now.iter().map(|(k, _)| k).collect::<Vec<_>>())?)?;
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

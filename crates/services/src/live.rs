//! `antibody-services live`: the Acme story on public testnets with nothing mocked.
//!
//! The networks are NEAR testnet, Ethereum Sepolia (verification and pause) and Base Sepolia
//! (pause). The services are the real OutLayer TEE, NEAR Chain Signatures, the NEAR Ethereum light
//! client and NEAR's DCAP verifier.
//!
//! Prerequisites (README, "Deployment"):
//! 1. `scripts/deploy/testnet.sh`;
//! 2. the Sepolia fixture: `scripts/demo/acme_fixture.py --net sepolia --rpc $SEPOLIA_RPC --anchor N`,
//!    where N is a finalized block the light client already has (`antibody anchor-check --block N`).
//!
//! The steps follow the sandbox demo:
//! - program (the anchor checked by the light client);
//! - known issue;
//! - agent A over MCP;
//! - the MPC-signed pause on both chains;
//! - agents B–F;
//! - finalize;
//! - a challenge of A's T0-delayed payout, settled by A's TDX quote verified on NEAR (T1), and F's
//!   delayed payout released early the same way;
//! - round 1;
//! - disclosure, and `verify` of D's duplicate proof.
//!
//! Every step records its transactions in `<work>/live.json`. A re-run skips finished steps, so a
//! failure costs only the step that failed; `--redo <step>` runs one again. `<work>/results.json`
//! gathers tx links, verdicts, outcomes and timings, measured from block timestamps. RPC URLs are
//! never written or printed.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use alloy_primitives::{Address, Bytes};
use antibody_cli::{
    cmd,
    ctx::{load_dotenv, Ctx, Deployments},
    near::Near,
    tee,
};
use antibody_verifier::{facts::sha256, manifest::Manifest};
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{
    demo::{ctx_for, mcp_submit, outcome_label, step, wait_for, ONE_TUSD},
    evm::Evm,
    relayer::Relayer,
};

pub struct LiveArgs {
    pub root: PathBuf,
    pub work: PathBuf,
    pub deployments: PathBuf,
    /// NEAR key files `<account>.json` (git-ignored `.secrets/near`).
    pub keys: PathBuf,
    /// `fixtures/acme-sepolia`: manifest, bundles and expected verdicts at the Sepolia anchor.
    pub fixture: PathBuf,
    /// `contracts/evm/deployments/11155111.json`.
    pub acme: PathBuf,
    pub antibody_bin: PathBuf,
    pub outlayer_api: String,
    pub deadline_minutes: u64,
    pub redo: Vec<String>,
}

/// The steps, in order.
pub const STEPS: &[&str] = &[
    "preflight", "program", "known", "open", "bonds", "agent-a", "pause", "agents", "finalize", "challenge", "t1", "round1", "disclose",
];

/// Expected verdict and final outcome of each submission (the same bundles and rules as
/// `fixtures/acme-local`). A and F are above the instant cap: T0 delays them, and T1 settles them.
const EXPECTED: [(&str, &str, &str); 7] = [
    ("KNOWN", "Violation", "KnownIssue"),
    ("A", "Violation", "Paid"),
    ("B", "Violation", "Paid"),
    ("C", "NoViolation", "Rejected"),
    ("D", "Violation", "Duplicate"),
    ("E", "Violation", "Known"),
    ("F", "Violation", "Paid"),
];

const SEPOLIA: u64 = 11155111;
const BASE_SEPOLIA: u64 = 84532;

fn unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn env_or_dotenv(root: &Path, key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| load_dotenv(&root.join(".env")).into_iter().find(|(k, _)| k == key).map(|(_, v)| v))
}

fn u128_of(v: &Value) -> u128 {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64().map(u128::from)).unwrap_or(0)
}

/// A transaction hash from a CLI summary: `{"tx": hash}`, or an explorer link.
fn tx_hash_of(v: &Value) -> Option<String> {
    let s = v.get("tx").unwrap_or(v).as_str()?;
    Some(s.rsplit('/').next().unwrap_or(s).to_string())
}

/// `[[yNEAR charged: "N"]]`, OutLayer's log of what a run cost.
pub fn parse_charged(log: &str) -> Option<u128> {
    let rest = log.split("yNEAR charged:").nth(1)?;
    rest.trim().trim_start_matches('"').split('"').next()?.parse().ok()
}

fn percentile(v: &[f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let rank = (p / 100.0 * (s.len() - 1) as f64).round() as usize;
    Some(s[rank.min(s.len() - 1)])
}

/// One receipt of a transaction tree, with the time of the block that executed it.
#[derive(Debug, Clone)]
struct Receipt {
    executor: String,
    method: String,
    height: u64,
    ts_ns: u64,
    gas_burnt: u64,
    tokens_burnt: u128,
    logs: Vec<String>,
}

impl Receipt {
    fn event(&self, name: &str) -> Option<Value> {
        self.logs.iter().filter_map(|l| l.strip_prefix("EVENT_JSON:")).filter_map(|j| serde_json::from_str::<Value>(j).ok()).find(|e| e["event"] == name)
    }
}

fn secs(from: u64, to: u64) -> f64 {
    to.saturating_sub(from) as f64 / 1e9
}

struct Live {
    a: LiveArgs,
    near: Near,
    dep: Deployments,
    hq: String,
    sepolia: String,
    base: String,
    st: Value,
    blocks: BTreeMap<String, (u64, u64)>,
}

impl Live {
    fn state_path(&self) -> PathBuf {
        self.a.work.join("live.json")
    }

    fn save(&self) -> Result<()> {
        std::fs::write(self.state_path(), serde_json::to_vec_pretty(&self.st)?)?;
        Ok(())
    }

    /// Keeps RPC URLs (which may carry API keys) out of every message and file.
    fn redact(&self, s: &str) -> String {
        let mut s = s.to_string();
        for u in [&self.sepolia, &self.base] {
            if !u.is_empty() {
                s = s.replace(u.as_str(), "<rpc>");
            }
        }
        s
    }

    fn acct(&self, who: &str) -> String {
        if who == "hq" {
            self.hq.clone()
        } else {
            format!("{who}.{}", self.hq)
        }
    }

    fn ctx(&self, who: &str) -> Ctx {
        let account = self.acct(who);
        ctx_for(&self.near, &self.dep, &account, &self.a.keys.join(format!("{account}.json")), &self.sepolia, &self.a.work.join("state").join(who))
    }

    fn pid(&self) -> Result<u32> {
        self.st["program_id"].as_u64().map(|p| p as u32).ok_or_else(|| eyre!("no program yet: run the program step"))
    }

    fn commit_id(&self, name: &str) -> Result<u64> {
        self.st["commits"][name]["id"].as_u64().ok_or_else(|| eyre!("no commit for {name} yet"))
    }

    fn bundle(&self, name: &str) -> PathBuf {
        self.a.fixture.join("submissions").join(format!("{name}.bundle.json"))
    }

    fn project_key(&self) -> PathBuf {
        self.a.keys.join(format!("{}.x25519.json", self.acct("acme")))
    }

    fn set_commit(&mut self, name: &str, field: &str, v: Value) -> Result<()> {
        self.st["commits"][name][field] = v;
        self.save()
    }

    fn final_height(&self) -> Result<u64> {
        let b = self.near.call("block", json!({"finality": "final"}))?;
        b["header"]["height"].as_u64().ok_or_else(|| eyre!("block without height"))
    }

    /// Block time of the latest final block, in nanoseconds.
    fn chain_time_ns(&self) -> Result<u64> {
        let b = self.near.call("block", json!({"finality": "final"}))?;
        b["header"]["timestamp_nanosec"].as_str().and_then(|s| s.parse().ok()).ok_or_else(|| eyre!("block without timestamp"))
    }

    /// Every receipt of `tx`, with its method, block time, gas, burnt tokens and logs.
    fn timeline(&mut self, tx: &str, signer: &str) -> Result<Vec<Receipt>> {
        let out = self.near.call("EXPERIMENTAL_tx_status", json!({"tx_hash": tx, "sender_account_id": signer, "wait_until": "FINAL"}))?;
        let mut methods = BTreeMap::new();
        for r in out["receipts"].as_array().into_iter().flatten() {
            let m = r["receipt"]["Action"]["actions"].as_array().into_iter().flatten().find_map(|a| a["FunctionCall"]["method_name"].as_str());
            methods.insert(r["receipt_id"].as_str().unwrap_or_default().to_string(), m.unwrap_or_default().to_string());
        }
        let mut v = Vec::new();
        for ro in out["receipts_outcome"].as_array().into_iter().flatten() {
            let bh = ro["block_hash"].as_str().unwrap_or_default().to_string();
            let (height, ts_ns) = match self.blocks.get(&bh) {
                Some(x) => *x,
                None => {
                    let b = self.near.call("block", json!({"block_id": bh}))?;
                    let x = (b["header"]["height"].as_u64().unwrap_or(0), b["header"]["timestamp_nanosec"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0));
                    self.blocks.insert(bh, x);
                    x
                }
            };
            let o = &ro["outcome"];
            v.push(Receipt {
                executor: o["executor_id"].as_str().unwrap_or_default().to_string(),
                method: methods.get(ro["id"].as_str().unwrap_or_default()).cloned().unwrap_or_default(),
                height,
                ts_ns,
                gas_burnt: o["gas_burnt"].as_u64().unwrap_or(0),
                tokens_burnt: u128_of(&o["tokens_burnt"]),
                logs: o["logs"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)).collect(),
            });
        }
        v.sort_by_key(|r| (r.height, r.ts_ns));
        let fee = u128_of(&out["transaction_outcome"]["outcome"]["tokens_burnt"]);
        if let Some(first) = v.first_mut() {
            first.tokens_burnt += fee;
        }
        Ok(v)
    }

    // ------------------------------------------------------------------ steps

    fn preflight(&mut self) -> Result<Value> {
        let hq = self.ctx("hq");
        let cfg = hq.view("get_config", json!({}))?;
        if cfg["verifier"]["pubkey"].is_null() {
            return Err(eyre!("near-antibody has no verifier pubkey yet (testnet.sh step 6: refresh_verifier_pubkey)"));
        }
        let tee_cfg = hq.view("get_tee_config", json!({}))?;
        if tee_cfg["measurements"].as_array().map(|m| m.is_empty()).unwrap_or(true) {
            return Err(eyre!("no T1 measurements configured (testnet.sh step 7b)"));
        }
        let guardian = self.dep.guardian.clone().ok_or_else(|| eyre!("the deployments file names no guardian"))?;
        let gcfg = self.near.view(&guardian, "get_config", json!({}))?;
        let chains = self.near.view(&guardian, "get_chains", json!({}))?;
        let gaddr = gcfg["guardian_address"].as_str().unwrap_or_default().to_string();

        let mut accounts = BTreeMap::new();
        let mut short = Vec::new();
        let need_near = |who: &str| -> f64 {
            match who {
                "hq" => 2.0,
                "bounty" => 1.0,
                _ => 0.5,
            }
        };
        let names: Vec<String> = ["hq", "bounty", "acme"].iter().map(|s| s.to_string()).chain(["a", "b", "c", "d", "e", "f"].iter().map(|x| format!("agent-{x}"))).collect();
        for who in &names {
            let acct = self.acct(who);
            let near_bal = self.near.account(&acct)?.map(|a| u128_of(&a["amount"]) as f64 / 1e24).unwrap_or(0.0);
            let tusd = u128_of(&self.near.view(&self.dep.tusd, "ft_balance_of", json!({"account_id": acct}))?);
            let internal = u128_of(&hq.view("get_balance", json!({"account_id": acct}))?);
            if near_bal < need_near(who) {
                short.push(format!("{acct}: {near_bal:.2} NEAR (needs {})", need_near(who)));
            }
            if who == "acme" && tusd < 25_000 * ONE_TUSD && self.st["program_id"].is_null() {
                short.push(format!("{acct}: {} tUSD (needs 25,000 for the escrow)", tusd / ONE_TUSD));
            }
            if who.starts_with("agent") && tusd + internal < 100 * ONE_TUSD {
                short.push(format!("{acct}: {} tUSD (needs 100 for bonds)", (tusd + internal) / ONE_TUSD));
            }
            accounts.insert(acct, json!({"near": near_bal, "tusd": tusd / ONE_TUSD, "internal_tusd": internal / ONE_TUSD}));
        }

        let exp: Value = serde_json::from_str(&std::fs::read_to_string(self.a.fixture.join("expected.json")).map_err(|e| {
            eyre!("{}: {e} (build it with scripts/demo/acme_fixture.py --net sepolia)", self.a.fixture.join("expected.json").display())
        })?)?;
        if exp["chain_id"].as_u64() != Some(SEPOLIA) {
            return Err(eyre!("the fixture is not a Sepolia fixture (chain_id {})", exp["chain_id"]));
        }
        let man = std::fs::read(self.a.fixture.join("manifest.json"))?;
        if hex::encode(sha256(&man)) != exp["manifest_sha256"].as_str().unwrap_or_default() {
            return Err(eyre!("fixture manifest.json does not match expected.json"));
        }
        for (name, kind, _) in EXPECTED {
            if !self.bundle(name).exists() {
                return Err(eyre!("missing bundle {}", self.bundle(name).display()));
            }
            let want = exp["submissions"][name]["kind"].as_str().unwrap_or_default();
            if want != kind {
                return Err(eyre!("the fixture's native run of {name} gave {want}, expected {kind}"));
            }
        }
        let anchor = exp["anchor"]["number"].as_u64().ok_or_else(|| eyre!("fixture without an anchor"))?;

        let sep = Evm::new(&self.sepolia);
        let base = Evm::new(&self.base);
        if sep.chain_id()? != SEPOLIA || base.chain_id()? != BASE_SEPOLIA {
            return Err(eyre!("SEPOLIA_RPC / BASE_SEPOLIA_RPC point at the wrong chains"));
        }
        let acme: Value = serde_json::from_str(&std::fs::read_to_string(&self.a.acme)?)?;
        let lend = acme["AcmeLend"].as_str().unwrap_or_default();
        sep.call("eth_getProof", json!([lend, [], format!("{anchor:#x}")]))
            .map_err(|e| eyre!("SEPOLIA_RPC cannot serve eth_getProof at the anchor (an archive node is needed): {}", self.redact(&e.to_string())))?;
        let lc = cmd::anchor_check(&hq, anchor, false)?;
        if lc["matches"] != json!(true) {
            return Err(eyre!("anchor {anchor}: {} (re-run acme_fixture.py with an anchor the light client has)", lc["verdict"]));
        }

        let mut gas = BTreeMap::new();
        for (rpc, id) in [(&sep, SEPOLIA), (&base, BASE_SEPOLIA)] {
            let t = chains.as_array().into_iter().flatten().find(|c| c["chain_id"].as_u64() == Some(id)).ok_or_else(|| eyre!("guardian has no chain {id}"))?;
            let need = u128::from(t["gas_limit"].as_u64().unwrap_or(0)) * u128_of(&t["max_fee_per_gas"]);
            let bal = u128::from_str_radix(rpc.call("eth_getBalance", json!([gaddr, "latest"]))?.as_str().unwrap_or("0x0").trim_start_matches("0x"), 16).unwrap_or(0);
            if bal < need {
                short.push(format!("guardian {gaddr} on chain {id}: {bal} wei, needs {need} for one pause()"));
            }
            gas.insert(id.to_string(), json!({"balance_wei": bal.to_string(), "needed_wei": need.to_string()}));
        }
        if !short.is_empty() {
            return Err(eyre!("not funded:\n  {}", short.join("\n  ")));
        }
        Ok(json!({"accounts": accounts, "anchor": lc, "guardian": {"address": gaddr, "chains": gas},
                  "verifier": cfg["verifier"], "tee_measurements": tee_cfg["measurements"].as_array().map(|m| m.len())}))
    }

    fn program(&mut self) -> Result<Value> {
        let acme = self.ctx("acme");
        let mut m = Manifest::parse(&std::fs::read(self.a.fixture.join("manifest.json"))?).map_err(|e| eyre!(e))?;
        m.program.name = "Acme Lend v1: live on Sepolia (testnet)".into();
        m.program.project = self.acct("acme");
        m.payout.token = self.dep.tusd.clone();
        m.anchor.trust = "light-client".into();
        m.rules.deadline_unix = unix() + self.a.deadline_minutes * 60;
        m.rules.reveal_window_blocks = 120;
        m.rules.disclosure_after_secs = 60;
        m.rules.instant_payout_cap = (5_000 * ONE_TUSD).to_string();
        m.rules.challenge_delay_secs = 1_800;
        let path = self.a.work.join("manifest.json");
        std::fs::write(&path, m.canonical())?;
        if !self.project_key().exists() {
            cmd::keygen_x25519(&self.project_key())?;
        }
        let created = cmd::program_create(&acme, &path, "25000", &self.project_key(), self.dep.guardian.clone(), None)?;
        let pid = created["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no program_created event: {created}"))?;
        self.st["program_id"] = json!(pid);
        self.save()?;
        let p = acme.program(pid as u32)?;
        if p["status"].get("Rejected").is_some() {
            return Err(eyre!("the light client rejected the anchor: {}", p["status"]));
        }
        Ok(json!({"program_id": pid, "manifest_sha256": created["manifest_sha256"], "anchor_trust": p["anchor_trust"],
                  "status": p["status"], "deadline_unix": m.rules.deadline_unix, "tx": created["tx"]["tx"], "explorer": created["tx"]["explorer"]}))
    }

    fn known(&mut self) -> Result<Value> {
        let acme = self.ctx("acme");
        let r = cmd::submit_known(&acme, self.pid()?, &self.bundle("KNOWN"), None, None, true)?;
        let id = r["commit_id"].as_u64().ok_or_else(|| eyre!("no commit id"))?;
        self.st["commits"]["KNOWN"] = json!({"id": id, "tx": r["tx"]["tx"], "signer": self.acct("acme"), "revealed": true});
        self.save()?;
        Ok(json!({"commit_id": id, "result": r["result"], "explorer": r["tx"]["explorer"]}))
    }

    fn open(&mut self) -> Result<Value> {
        let r = cmd::program_open(&self.ctx("acme"), self.pid()?)?;
        Ok(json!({"tx": r["tx"], "explorer": r["explorer"]}))
    }

    fn bonds(&mut self) -> Result<Value> {
        let mut out = BTreeMap::new();
        for x in ["a", "b", "c", "d", "e", "f"] {
            let ctx = self.ctx(&format!("agent-{x}"));
            let acct = self.acct(&format!("agent-{x}"));
            let bal = u128_of(&ctx.view("get_balance", json!({"account_id": acct}))?);
            if bal < 100 * ONE_TUSD {
                let r = cmd::deposit(&ctx, "100")?;
                out.insert(acct, r["tx"].clone());
            } else {
                out.insert(acct, json!("already deposited"));
            }
        }
        Ok(json!(out))
    }

    fn agent_a(&mut self) -> Result<Value> {
        let acct = self.acct("agent-a");
        let fc = self.a.keys.join(format!("{acct}.fc-key.json"));
        if !fc.exists() {
            cmd::agent_key_add(&self.ctx("agent-a"), 2.0, &fc)?;
        }
        let state = self.a.work.join("state/agent-a-mcp");
        std::fs::create_dir_all(&state)?;
        let mcp = mcp_submit(&self.a.antibody_bin, &["--network", "testnet"], &self.a.deployments, &acct, &fc, None, &state, self.pid()?, &self.bundle("A"))?;
        let id = mcp["result"]["commit"]["commit_id"].as_u64().ok_or_else(|| eyre!("MCP submit: no commit id: {}", self.redact(&mcp.to_string())))?;
        let tx = tx_hash_of(&mcp["result"]["reveal"]["submit"]);
        self.st["commits"]["A"] = json!({"id": id, "tx": tx, "signer": acct, "revealed": true, "via": "MCP"});
        self.st["a_verdict_unix"] = json!(unix());
        self.save()?;
        Ok(json!({"commit_id": id, "mcp_tools": mcp["mcp_tools"], "result": mcp["result"]["reveal"]["result"]}))
    }

    fn pause(&mut self) -> Result<Value> {
        let guardian = self.dep.guardian.clone().ok_or_else(|| eyre!("no guardian"))?;
        let relayer = Relayer {
            near: self.near.clone(),
            guardian,
            chains: BTreeMap::from([
                (SEPOLIA, (self.sepolia.clone(), "https://sepolia.etherscan.io/tx/".to_string())),
                (BASE_SEPOLIA, (self.base.clone(), "https://sepolia.basescan.org/tx/".to_string())),
            ]),
            state_file: self.a.work.join("relayer.json"),
        };
        let t = Instant::now();
        let last = std::cell::RefCell::new(String::new());
        wait_for("pause() confirmed on Sepolia and Base Sepolia", Duration::from_secs(900), || match relayer.tick() {
            Ok(st) => st.values().filter(|v| v["paused"] == json!(true) && v["status"] == "confirmed").count() == 2,
            Err(e) => {
                *last.borrow_mut() = e.to_string();
                std::thread::sleep(Duration::from_secs(5));
                false
            }
        })
        .map_err(|e| eyre!("{e} (last relayer error: {})", self.redact(&last.borrow())))?;
        let after_verdict = self.st["a_verdict_unix"].as_u64().map(|v0| unix().saturating_sub(v0));
        Ok(json!({"relayer_seconds": t.elapsed().as_secs_f64(), "seconds_after_a_verdict_wall_clock": after_verdict,
                  "chains": relayer.load_state()}))
    }

    fn agents(&mut self) -> Result<Value> {
        let pid = self.pid()?;
        let mut out = BTreeMap::new();
        for x in ["b", "c", "d", "e", "f"] {
            let name = x.to_uppercase();
            if self.st["commits"][&name]["revealed"] == json!(true) {
                continue;
            }
            let who = format!("agent-{x}");
            let ctx = self.ctx(&who);
            let id = match self.st["commits"][&name]["id"].as_u64() {
                Some(id) => id,
                None => {
                    let report = (x == "c").then(|| self.a.fixture.join("submissions/C.report.md"));
                    let c = cmd::commit(&ctx, pid, &self.bundle(&name), None, report.as_deref())?;
                    let id = c["commit_id"].as_u64().ok_or_else(|| eyre!("commit returned no id"))?;
                    self.st["commits"][&name] = json!({"id": id, "commit_tx": c["tx"]["tx"], "signer": self.acct(&who)});
                    self.save()?;
                    id
                }
            };
            let r = cmd::reveal(&ctx, pid, id, true)?;
            self.set_commit(&name, "tx", json!(tx_hash_of(&r["submit"])))?;
            self.set_commit(&name, "revealed", json!(true))?;
            out.insert(name, r["result"].clone());
        }
        Ok(json!(out))
    }

    fn finalize(&mut self) -> Result<Value> {
        let hq = self.ctx("hq");
        let p = hq.program(self.pid()?)?;
        let window = p["reveal_window_blocks"].as_u64().unwrap_or(0);
        let mut last = 0;
        for (name, ..) in &EXPECTED[1..] {
            last = last.max(hq.commit(self.commit_id(name)?)?["height"].as_u64().unwrap_or(0));
        }
        let t = Instant::now();
        while self.final_height()? <= last + window {
            if t.elapsed() > Duration::from_secs(1800) {
                return Err(eyre!("the reveal windows did not close within 30 minutes"));
            }
            std::thread::sleep(Duration::from_secs(5));
        }
        let mut out = BTreeMap::new();
        for (name, ..) in &EXPECTED[1..] {
            let id = self.commit_id(name)?;
            if hq.commit(id)?["outcome"] == json!("Pending") {
                let r = cmd::program_simple(&hq, "finalize", json!({"commit_id": id}), 60)?;
                self.set_commit(name, "finalize_tx", r["tx"].clone())?;
            }
            out.insert(name.to_string(), hq.commit(id)?["outcome"].clone());
        }
        Ok(json!(out))
    }

    fn challenge(&mut self) -> Result<Value> {
        let id = self.commit_id("A")?;
        let watcher = self.ctx("agent-c");
        let c = watcher.commit(id)?;
        if c["outcome"].get("Delayed").is_none() {
            return Err(eyre!("A's payout is not T0-delayed: {}", c["outcome"]));
        }
        let reason = "Live demo: a watcher challenges a T0-delayed payout. The run's TDX quote, verified on NEAR, settles it without the arbiter.";
        let r = cmd::program_simple(&watcher, "challenge", json!({"commit_id": id, "reason": reason}), 30)?;
        self.set_commit("A", "challenge_tx", r["tx"].clone())?;
        Ok(json!({"tx": r["tx"], "explorer": r["explorer"], "outcome": watcher.commit(id)?["outcome"]}))
    }

    fn t1(&mut self) -> Result<Value> {
        let hq = self.ctx("hq");
        let mut out = BTreeMap::new();
        for name in ["A", "F"] {
            let id = self.commit_id(name)?;
            if hq.commit(id)?["tee"].is_null() {
                let (tx, signer) = (self.st["commits"][name]["tx"].as_str().unwrap_or_default().to_string(), self.st["commits"][name]["signer"].as_str().unwrap_or_default().to_string());
                let rid = tee::request_id_from_tx(&hq, &tx, &signer)?;
                let last = std::cell::RefCell::new(String::new());
                let res = std::cell::RefCell::new(Value::Null);
                // OutLayer publishes the attestation shortly after the run; retry until it is there.
                wait_for(&format!("{name}'s T1 confirmation"), Duration::from_secs(600), || match tee::confirm(&hq, &self.a.outlayer_api, id, None, rid, "auto") {
                    Ok(v) if v["confirmed"] == json!(true) => {
                        *res.borrow_mut() = v;
                        true
                    }
                    Ok(v) => {
                        *last.borrow_mut() = v.to_string();
                        std::thread::sleep(Duration::from_secs(15));
                        false
                    }
                    Err(e) => {
                        *last.borrow_mut() = e.to_string();
                        std::thread::sleep(Duration::from_secs(15));
                        false
                    }
                })
                .map_err(|e| eyre!("{e} (last: {})", self.redact(&last.borrow())))?;
                let v = res.into_inner();
                self.set_commit(name, "request_id", json!(rid))?;
                self.set_commit(name, "t1_tx", json!(tx_hash_of(&v)))?;
            }
            let outcome = hq.commit(id)?["outcome"].clone();
            if outcome.get("Challenged").is_some() {
                let r = cmd::program_simple(&hq, "resolve_challenge_t1", json!({"commit_id": id}), 60)?;
                self.set_commit(name, "payout_tx", r["tx"].clone())?;
            } else if outcome.get("Delayed").is_some() {
                let r = cmd::program_simple(&hq, "release", json!({"commit_id": id}), 60)?;
                self.set_commit(name, "payout_tx", r["tx"].clone())?;
            }
            let c = hq.commit(id)?;
            out.insert(name.to_string(), json!({"tee": c["tee"], "outcome": c["outcome"]}));
        }
        Ok(json!(out))
    }

    fn round1(&mut self) -> Result<Value> {
        let acme = self.ctx("acme");
        let pid = self.pid()?;
        let dep: Value = serde_json::from_str(&std::fs::read_to_string(&self.a.acme)?)?;
        let fixed = dep["fixedLendRuntimeCode"].as_str().ok_or_else(|| eyre!("the Sepolia deployment has no fixedLendRuntimeCode"))?;
        let lend: Address = dep["AcmeLend"].as_str().unwrap_or_default().parse()?;
        let m = Manifest::parse(&std::fs::read(self.a.work.join("manifest.json"))?).map_err(|e| eyre!(e))?;
        let r1 = m.with_overrides(1, &[(lend, Bytes::from(hex::decode(fixed.trim_start_matches("0x"))?))]).map_err(|e| eyre!(e))?;
        let r1_path = self.a.work.join("round1.json");
        std::fs::write(&r1_path, r1.canonical())?;
        if acme.program(pid)?["round"].as_u64() == Some(0) {
            let r = cmd::round_propose(&acme, pid, &r1_path)?;
            self.st["round1"]["propose_tx"] = r["tx"]["tx"].clone();
            self.save()?;
        }
        let mut reruns = BTreeMap::new();
        for (name, want) in [("KNOWN", "Violation"), ("A", "NoViolation"), ("B", "Violation"), ("F", "Violation")] {
            if self.st["round1"]["reruns"][name]["done"] == json!(true) {
                continue;
            }
            let r = cmd::round_rerun(&acme, self.commit_id(name)?, Some(&self.project_key()), true)?;
            let kind = r["result"]["kind"].as_str().unwrap_or("-").to_string();
            if r["result"]["status"] != json!("Done") || kind != want {
                return Err(eyre!("re-run of {name}: {} (expected {want})", r["result"]));
            }
            self.st["round1"]["reruns"][name] = json!({"done": true, "tx": r["tx"]["tx"], "signer": self.acct("acme"), "kind": kind,
                                                       "still_broken": r["result"]["still_broken"]});
            self.save()?;
            reruns.insert(name, kind);
        }
        if acme.program(pid)?["open_round"].as_u64() == Some(0) {
            let opened = cmd::round_open(&acme, pid)?;
            self.st["round1"]["open"] = json!({"carried": opened["round"]["carried"], "released": opened["round"]["released"]});
            self.save()?;
        }
        let open = &self.st["round1"]["open"];
        if open["carried"].as_u64() != Some(3) || open["released"].as_u64() != Some(1) {
            return Err(eyre!("round 1 opened with {open}, expected 3 carried and 1 released"));
        }
        Ok(json!({"reruns": reruns, "open": open}))
    }

    fn disclose(&mut self) -> Result<Value> {
        let hq = self.ctx("hq");
        let pid = self.pid()?;
        if hq.program(pid)?["disclosure"].is_null() {
            let p = hq.program(pid)?;
            let at = p["deadline_ns"].as_u64().unwrap_or(0) + p["disclosure_after_ns"].as_u64().unwrap_or(0);
            loop {
                let now = self.chain_time_ns()?;
                if now >= at {
                    break;
                }
                let left = (at - now) / 1_000_000_000;
                println!("disclosure opens in {left}s (deadline + disclosure delay, in block time)");
                std::thread::sleep(Duration::from_secs(left.clamp(5, 60)));
            }
            let last = std::cell::RefCell::new(String::new());
            wait_for("the disclosure transaction", Duration::from_secs(300), || match cmd::program_simple(&hq, "disclose", json!({"program_id": pid}), 300) {
                Ok(r) => {
                    self.st["disclose_tx"] = r["tx"].clone();
                    true
                }
                Err(e) => {
                    *last.borrow_mut() = e.to_string();
                    std::thread::sleep(Duration::from_secs(10));
                    false
                }
            })
            .map_err(|e| eyre!("{e} (last error: {})", last.borrow()))?;
            self.save()?;
            wait_for("the disclosure result", Duration::from_secs(300), || hq.program(pid).map(|p| !p["disclosure"].is_null()).unwrap_or(false))?;
        }
        let v = cmd::verify(&hq, self.commit_id("D")?)?;
        let ok = v["same_rule_same_function"] == json!(true) && v["committed_earlier"] == json!(true);
        if !ok {
            return Err(eyre!("D's duplicate proof did not verify: {v}"));
        }
        Ok(json!({"disclose_tx": self.st["disclose_tx"], "verify_d": v}))
    }

    // ---------------------------------------------------------------- results

    /// Tx links, verdicts and outcomes, and timings from block timestamps: OutLayer per run,
    /// Chain Signatures per chain, DCAP gas, and submit-to-payout.
    fn results(&mut self) -> Result<Value> {
        let hq = self.ctx("hq");
        let antibody = self.dep.antibody.clone();
        let outlayer = self.dep.outlayer.clone().unwrap_or_else(|| "outlayer.testnet".into());
        let guardian = self.dep.guardian.clone().unwrap_or_default();
        let verifier = hq.view("get_tee_config", json!({}))?["verifier"].as_str().unwrap_or_default().to_string();
        let mut runs = Vec::new();
        let mut table = Vec::new();
        let mut all_ok = true;
        let mut mpc = Value::Null;
        let mut dcap = Vec::new();
        let (mut ol_secs, mut ol_charged, mut ol_burnt) = (Vec::new(), Vec::new(), Vec::new());

        let mut measure_run = |live: &mut Live, label: String, tx: &str, signer: &str| -> Result<Value> {
            let rs = live.timeline(tx, signer)?;
            let start = rs.first().map(|r| r.ts_ns).unwrap_or(0);
            let req = rs.iter().find(|r| r.executor == outlayer && r.method == "request_execution").cloned();
            let resp = rs.iter().find(|r| r.executor == outlayer && r.method == "on_execution_response").cloned();
            let cb = rs.iter().find(|r| r.executor == antibody && r.method.starts_with("on_")).cloned();
            let charged = rs.iter().flat_map(|r| r.logs.iter()).find_map(|l| parse_charged(l));
            let burnt: u128 = rs.iter().map(|r| r.tokens_burnt).sum();
            let ol = match (&req, resp.as_ref().or(cb.as_ref())) {
                (Some(a), Some(b)) => Some(secs(a.ts_ns, b.ts_ns)),
                _ => None,
            };
            if let Some(s) = ol {
                ol_secs.push(s);
            }
            if let Some(c) = charged {
                ol_charged.push(c as f64 / 1e24);
            }
            ol_burnt.push(burnt as f64 / 1e24);
            Ok(json!({"run": label, "tx": live.near.explorer_tx(tx), "outlayer_seconds": ol,
                      "tx_to_callback_seconds": cb.map(|c| secs(start, c.ts_ns)), "outlayer_charged_near": charged.map(|c| c as f64 / 1e24),
                      "gas_fees_near": burnt as f64 / 1e24, "receipts": rs.len()}))
        };

        let commits = self.st["commits"].clone();
        for (name, kind, outcome) in EXPECTED {
            let c = &commits[name];
            let (Some(id), Some(tx), Some(signer)) = (c["id"].as_u64(), c["tx"].as_str(), c["signer"].as_str()) else {
                all_ok = false;
                table.push(json!({"submission": name, "missing": true}));
                continue;
            };
            runs.push(measure_run(self, format!("verify {name}"), tx, signer)?);
            let onchain = hq.commit(id)?;
            let got_kind = onchain["verdict"]["kind"].as_str().unwrap_or("-").to_string();
            let got_out = outcome_label(&onchain["outcome"]);
            let ok = got_kind == kind && got_out == outcome;
            all_ok &= ok;
            let mut row = json!({"submission": name, "commit": id, "verdict": got_kind, "keys": onchain["verdict"]["keys"],
                                 "outcome": onchain["outcome"], "tier": onchain["verdict"]["tier"], "t1": !onchain["tee"].is_null(),
                                 "submit_tx": self.near.explorer_tx(tx), "ok": ok});
            // Submit to payout, in block time.
            if let Some(ptx) = c["payout_tx"].as_str().or(c["finalize_tx"].as_str()) {
                let rs = self.timeline(tx, signer)?;
                let paid = self.timeline(ptx, &self.acct("hq"))?;
                if let (Some(s), Some(p)) = (rs.first(), paid.iter().find(|r| r.event("paid").is_some())) {
                    row["submit_to_payout_seconds"] = json!(secs(s.ts_ns, p.ts_ns));
                    row["payout_tx"] = json!(self.near.explorer_tx(ptx));
                }
            }
            if name == "A" {
                let rs = self.timeline(tx, signer)?;
                let sign0 = rs.iter().filter(|r| r.method == "sign" && r.executor != guardian).map(|r| r.ts_ns).min();
                let signed: Vec<Value> = rs
                    .iter()
                    .filter(|r| r.executor == guardian && r.method == "on_signed")
                    .filter_map(|r| r.event("pause_signed").map(|e| (r.ts_ns, e)))
                    .map(|(ts, e)| json!({"chain_id": e["data"][0]["chain_id"], "mpc_seconds": sign0.map(|s0| secs(s0, ts)),
                                          "evm_tx": e["data"][0]["tx_hash"]}))
                    .collect();
                mpc = json!({"pause_signatures": signed});
            }
            if let Some(t1) = c["t1_tx"].as_str() {
                let rs = self.timeline(t1, &self.acct("hq"))?;
                let gas = rs.iter().filter(|r| r.executor == verifier).map(|r| r.gas_burnt).sum::<u64>();
                let t0 = rs.first().map(|r| r.ts_ns).unwrap_or(0);
                let done = rs.iter().find(|r| r.event("tee_confirmed").is_some()).map(|r| secs(t0, r.ts_ns));
                dcap.push(json!({"commit": name, "tx": self.near.explorer_tx(t1), "verify_quote_tgas": gas as f64 / 1e12, "seconds": done}));
            }
            table.push(row);
        }
        let reruns = self.st["round1"]["reruns"].clone();
        for (name, r) in reruns.as_object().into_iter().flatten() {
            if let (Some(tx), Some(signer)) = (r["tx"].as_str(), r["signer"].as_str()) {
                runs.push(measure_run(self, format!("re-run {name} at round 1"), tx, signer)?);
            }
        }
        if let Some(tx) = self.st["disclose_tx"].as_str().map(String::from) {
            let hq_acct = self.acct("hq");
            runs.push(measure_run(self, "disclosure".into(), &tx, &hq_acct)?);
        }
        let summary = json!({
            "outlayer": {"runs": ol_secs.len(), "p50_seconds": percentile(&ol_secs, 50.0), "p95_seconds": percentile(&ol_secs, 95.0),
                         "max_seconds": ol_secs.iter().cloned().fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x)))),
                         "charged_near_p50": percentile(&ol_charged, 50.0), "tx_tree_fees_near_p50": percentile(&ol_burnt, 50.0)},
            "chain_signatures": mpc,
            "dcap": dcap,
        });
        let out = json!({"program_id": self.st["program_id"], "submissions": table, "runs": runs, "summary": summary,
                         "pause": self.st["steps"]["pause"]["out"], "round1": self.st["round1"], "disclosure": self.st["steps"]["disclose"]["out"],
                         "all_ok": all_ok, "finished_unix": unix()});
        std::fs::write(self.a.work.join("results.json"), serde_json::to_vec_pretty(&out)?)?;
        println!("{:<6} {:>6} {:<12} {:<11} {:>4} {:>10}", "sub", "commit", "verdict", "outcome", "T1", "to payout");
        for r in &table {
            println!(
                "{:<6} {:>6} {:<12} {:<11} {:>4} {:>10} {}",
                r["submission"].as_str().unwrap_or_default(),
                r["commit"],
                r["verdict"].as_str().unwrap_or("-"),
                outcome_label(&r["outcome"]),
                if r["t1"] == json!(true) { "yes" } else { "" },
                r["submit_to_payout_seconds"].as_f64().map(|s| format!("{s:.0}s")).unwrap_or_default(),
                if r["ok"] == json!(true) { "OK" } else { "MISMATCH" }
            );
        }
        println!("OutLayer: {}", summary["outlayer"]);
        Ok(out)
    }
}

pub fn run(a: LiveArgs) -> Result<Value> {
    std::fs::create_dir_all(&a.work)?;
    let dep: Deployments = serde_json::from_str(&std::fs::read_to_string(&a.deployments).map_err(|e| eyre!("{}: {e}", a.deployments.display()))?)?;
    if dep.network != "testnet" {
        return Err(eyre!("live runs on NEAR testnet only (the deployments file says {})", dep.network));
    }
    let hq = dep.antibody.split_once('.').map(|(_, h)| h.to_string()).ok_or_else(|| eyre!("cannot derive the HQ account from {}", dep.antibody))?;
    let sepolia = env_or_dotenv(&a.root, "SEPOLIA_RPC").ok_or_else(|| eyre!("set SEPOLIA_RPC (archive, eth_getProof) in .env"))?;
    let base = env_or_dotenv(&a.root, "BASE_SEPOLIA_RPC").ok_or_else(|| eyre!("set BASE_SEPOLIA_RPC in .env"))?;
    let near = Near::new("testnet", dep.rpc.clone());
    let st = std::fs::read(a.work.join("live.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({"steps": {}}));
    let mut live = Live { a, near, dep, hq, sepolia, base, st, blocks: BTreeMap::new() };
    for name in STEPS {
        let redo = live.a.redo.iter().any(|r| r == name);
        if live.st["steps"][*name]["done"] == json!(true) && !redo {
            println!("== {name}: done earlier, skipped");
            continue;
        }
        step(&format!("{name} (live)"));
        let t = Instant::now();
        let r = match *name {
            "preflight" => live.preflight(),
            "program" => live.program(),
            "known" => live.known(),
            "open" => live.open(),
            "bonds" => live.bonds(),
            "agent-a" => live.agent_a(),
            "pause" => live.pause(),
            "agents" => live.agents(),
            "finalize" => live.finalize(),
            "challenge" => live.challenge(),
            "t1" => live.t1(),
            "round1" => live.round1(),
            "disclose" => live.disclose(),
            _ => unreachable!(),
        };
        match r {
            Ok(out) => {
                println!("{}", live.redact(&serde_json::to_string_pretty(&out)?));
                live.st["steps"][*name] = json!({"done": true, "seconds": t.elapsed().as_secs_f64(), "finished_unix": unix(), "out": out});
                live.save()?;
            }
            Err(e) => {
                let msg = live.redact(&format!("{e:#}"));
                live.st["steps"][*name] = json!({"done": false, "error": msg, "failed_unix": unix()});
                live.save()?;
                return Err(eyre!("step {name} failed: {msg}\n(fix it and re-run: finished steps are skipped)"));
            }
        }
    }
    step("results");
    let out = live.results()?;
    if out["all_ok"] != json!(true) {
        return Err(eyre!("live run finished with mismatches (see {})", live.a.work.join("results.json").display()));
    }
    println!("\nALL OK, results: {}", live.a.work.join("results.json").display());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlayer_charge_log() {
        assert_eq!(parse_charged("[[yNEAR charged: \"1030000000000000000000\"]]"), Some(1_030_000_000_000_000_000_000));
        assert_eq!(parse_charged("something else"), None);
    }

    #[test]
    fn percentiles_and_hashes() {
        assert_eq!(percentile(&[3.0, 1.0, 2.0], 50.0), Some(2.0));
        assert_eq!(percentile(&[], 95.0), None);
        assert_eq!(tx_hash_of(&json!({"tx": "https://testnet.nearblocks.io/txns/AbC"})).as_deref(), Some("AbC"));
        assert_eq!(tx_hash_of(&json!("AbC")).as_deref(), Some("AbC"));
    }
}

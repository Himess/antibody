//! `antibody-services drill`: the daily live drill. It runs "the one moment" again on NEAR testnet, with
//! Sepolia and Base Sepolia, on a FRESH instance of the Acme Lend fixture (docs/drill.md). It shows the
//! system still works today and catches breakage (worker images, collateral, RPCs, floats, relayer keys)
//! before judges do.
//!
//! One drill: deploy a fresh Acme Lend (a new deployer, so new CREATE2 addresses) on both chains → a new
//! program on a light-client anchor, activated → the scripted fixture agent A submits its sealed Critical
//! over MCP → the verdict, T1, the pause permits signed by the MPC network and relayed on both chains, and
//! the instant payout. Rounds and the other agents stay in the full story (`live`), not here.
//!
//! The steps run in order and record what they made in `<work>/drill.json`; a run resumes at the first step
//! not done. A finished drill writes `drill-YYYY-MM-DD.json` into the results directory, in the same shape
//! the API's `moments()` reads for the one moment (crates/services/src/web.rs), plus a `drill` section (per
//! step status and errors, and a `cost` line) the kind-independent extras.
//!
//! Guards (docs/drill.md): before anything is spent it reads the balances a drill needs and skips with a
//! reason if any is low (so `/v1/status` alerts instead of the drill failing mid-way); a kill switch (a
//! file in the work dir, or `$ANTIBODY_DRILL_OFF`) stops it; each step has a timeout, and a failed step ends
//! the drill with `all_ok: false` and the step's error, so `/moment` shows where it stopped. No testnet or
//! EVM transaction happens unless a drill is actually run: `--dry-run` only reads the guards.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use alloy_primitives::B256;
use antibody_cli::{
    cmd,
    ctx::{Ctx, Deployments},
    near::{Action, Near, ONE_NEAR},
};
use antibody_verifier::facts::sha256;
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{
    demo::{self, step, Broadcaster},
    evm::{Evm, EvmSigner},
    live::{
        acme_settlement, forge_retry, lc_block_hash, lc_last_block, mpc_address, permits, unix, wnear_of, wrap, BASESCAN, BASE_SEPOLIA, ESCROW, ETHERSCAN,
        FASTNEAR_TX, FREE_FOR_RUNS, HQ, MILLI, SEPOLIA, T1_WAIT, TERMS, TESTNET,
    },
    relayer::Relayer,
    t1::{Source, T1Relayer},
};

/// A drill program lives long enough to finish and be cleaned up the next day: it opens, is proven and paid
/// in minutes, and becomes exitable after its deadline and the exit grace. A daily drill (or `--cleanup`)
/// then closes the previous day's programs and their escrow comes back.
const DRILL_DEADLINE_SECS: u64 = 12 * 3_600;

/// The steps, in order. Resumable: `<work>/drill.json` records which are done.
const STEPS: &[&str] = &["evm", "anchor", "prebuild", "light-client", "program", "guardian", "activate", "agent", "one-moment", "results"];

/// The NEAR accounts a drill uses (sub-accounts of `antibody.testnet`), each a free function of HQ.
fn acct(n: &str) -> String {
    format!("{n}.{HQ}")
}

/// The NEAR each account a drill uses must keep free of its storage stake for the drill to run without
/// topping it up from HQ: the project's escrow and runs, the hunter's run, the keeper's pause ticket and
/// payment, the T1 relayer's relays. HQ itself must be able to top the others up.
fn near_floor(role: &str) -> u128 {
    match role {
        // Escrow (0.4) wrapped, plus a run's 0.35 free and the register's MPC credit.
        "acme" => cmd::parse_near("0.9").unwrap(),
        "agent-a" => FREE_FOR_RUNS,
        "keeper" => FREE_FOR_RUNS,
        "relayer" => cmd::parse_near("0.5").unwrap(),
        // HQ tops the others up: enough for the escrow and a few floats.
        "hq" => cmd::parse_near("1.5").unwrap(),
        _ => FREE_FOR_RUNS,
    }
}

pub struct DrillArgs {
    pub root: PathBuf,
    /// The drill's work directory (state, witnesses, keys): resumable across runs. Default `target/drill`.
    pub work: PathBuf,
    pub forge_bin: PathBuf,
    pub antibody_bin: PathBuf,
    /// NEAR keys, `<account>.json` (`.secrets/near`).
    pub keys: PathBuf,
    /// The base EVM deployer's key file (`{"address","private_key"}`): a per-drill deployer is derived from
    /// it (so each drill deploys at fresh addresses) and funded from it.
    pub base_evm_key: PathBuf,
    /// The relayer's EVM key (`$RELAYER_EVM_KEY`/.env): it submits the pause permits and pays its own gas.
    pub relayer_evm_key: String,
    pub sepolia_rpc: String,
    pub sepolia_tx_rpc: String,
    pub base_rpc: String,
    /// Where the finished `drill-YYYY-MM-DD.json` is written (the API's `--results-dir`).
    pub results_dir: PathBuf,
    /// Read the guards and stop, spending nothing (CI, a dry check).
    pub dry_run: bool,
    /// Close exitable drill programs (escrow back) instead of running a drill.
    pub cleanup: bool,
}

/// "YYYY-MM-DD" of a Unix time, as the drill names its file and program.
fn date_of(unix_s: u64) -> String {
    crate::status::rfc3339_of_unix(unix_s)[..10].to_string()
}

/// Whether the kill switch is set: a file `<work>/drill.disabled` or `$ANTIBODY_DRILL_OFF` not empty.
fn kill_switch(work: &Path) -> Option<String> {
    if let Ok(v) = std::env::var("ANTIBODY_DRILL_OFF") {
        if !v.trim().is_empty() {
            return Some(format!("$ANTIBODY_DRILL_OFF is set ({})", v.trim().chars().take(100).collect::<String>()));
        }
    }
    let f = work.join("drill.disabled");
    f.exists().then(|| format!("the kill switch file {} is present", f.display()))
}

struct Drill {
    a: DrillArgs,
    near: Near,
    dep: Deployments,
    s: Value,
    date: String,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Drill {
    fn ctx_at(&self, account: &str, state: &str) -> Ctx {
        demo::ctx_for(&self.near, &self.dep, account, &self.a.keys.join(format!("{account}.json")), &self.a.sepolia_rpc, &self.a.work.join("state").join(state))
    }

    fn ctx_of(&self, n: &str) -> Ctx {
        self.ctx_at(&acct(n), n)
    }

    fn root_ctx(&self) -> Ctx {
        self.ctx_at(HQ, "root")
    }

    fn mcp_dir(&self) -> PathBuf {
        self.a.work.join("state/agent-a-mcp")
    }

    fn services_dir(&self) -> PathBuf {
        self.a.work.join("services/api")
    }

    fn relayer_file(&self) -> PathBuf {
        self.a.work.join("services/relayer.json")
    }

    fn sepolia(&self) -> String {
        self.a.sepolia_tx_rpc.clone()
    }

    /// The per-drill deployer: derived from the base key with the drill's date, so each drill deploys the
    /// fixture at fresh CREATE2 addresses without touching contracts/.
    fn deployer(&self) -> Result<EvmSigner> {
        let base = EvmSigner::from_hex(&self.base_evm_key()?)?;
        base.derive(format!("drill-{}", self.date).as_bytes())
    }

    fn base_evm_key(&self) -> Result<String> {
        let v: Value = serde_json::from_slice(&std::fs::read(&self.a.base_evm_key)?)?;
        v["private_key"].as_str().map(String::from).ok_or_else(|| eyre!("{}: no private_key", self.a.base_evm_key.display()))
    }

    fn pid(&self) -> u32 {
        self.s["program"]["program_id"].as_u64().unwrap_or(u64::MAX) as u32
    }

    fn key(&self) -> String {
        format!("{}/{}", self.dep.antibody, self.pid())
    }

    fn done(&self, step: &str) -> bool {
        self.s["done"].as_array().is_some_and(|d| d.iter().any(|x| x == step))
    }

    fn set_done(&mut self, step: &str, done: bool) {
        let mut d: Vec<Value> = self.s["done"].as_array().cloned().unwrap_or_default().into_iter().filter(|x| x != step).collect();
        if done {
            d.push(json!(step));
        }
        self.s["done"] = Value::Array(d);
    }

    fn save(&self) -> Result<()> {
        std::fs::write(self.a.work.join("drill.json"), serde_json::to_vec_pretty(&self.s)?)?;
        Ok(())
    }

    /// Tops `account` up from HQ to `min` yoctoNEAR free of its storage stake (as live's `ensure_liquid`).
    fn ensure_liquid(&self, account: &str, min: u128) -> Result<u128> {
        let a = self.near.account(account)?.ok_or_else(|| eyre!("no account {account}"))?;
        let free = cmd::u128_of(&a["amount"]).saturating_sub(a["storage_usage"].as_u64().unwrap_or(0) as u128 * 10u128.pow(19));
        if free >= min {
            return Ok(0);
        }
        if account == HQ {
            return Err(eyre!("{HQ} has {} NEAR free and this step needs {}", cmd::fmt_tokens(free, 24), cmd::fmt_tokens(min, 24)));
        }
        let root = self.root_ctx();
        let out = root.near.send(&root.signer()?, account, vec![Action::Transfer(min - free)], "EXECUTED_OPTIMISTIC")?;
        cmd::ensure_ok(&root, &out, "transfer")?;
        crate::live::poll(&format!("{account}'s top-up in a final block"), Duration::from_secs(60), Duration::from_secs(2), || {
            let a = self.near.call("query", json!({"request_type": "view_account", "finality": "final", "account_id": account})).ok()?;
            let seen = cmd::u128_of(&a["amount"]).saturating_sub(a["storage_usage"].as_u64().unwrap_or(0) as u128 * 10u128.pow(19));
            (seen >= min).then_some(())
        })?;
        Ok(min - free)
    }

    // ------------------------------------------------------------------------------ balances and guards

    /// The NEAR amount (yoctoNEAR) and liquid-of-storage of an account now, or null if it does not exist.
    fn near_balance(&self, account: &str) -> Value {
        match self.near.account(account) {
            Ok(Some(a)) => {
                let amount = cmd::u128_of(&a["amount"]);
                let liquid = amount.saturating_sub(a["storage_usage"].as_u64().unwrap_or(0) as u128 * 10u128.pow(19));
                json!({"amount_yocto": amount.to_string(), "liquid_yocto": liquid.to_string()})
            }
            _ => Value::Null,
        }
    }

    /// A snapshot of every balance a drill spends from: the NEAR accounts, and the EVM deployer and relayer
    /// on both chains. Read once before the drill and once after, so the cost line is the delta.
    fn balances(&self) -> Value {
        let mut near = serde_json::Map::new();
        for role in ["hq", "acme", "agent-a", "keeper", "relayer"] {
            let who = if role == "hq" { HQ.to_string() } else { acct(role) };
            near.insert(role.into(), json!({"account": who, "balance": self.near_balance(&who)}));
        }
        let deployer = self.deployer().map(|d| format!("{:?}", d.address)).unwrap_or_default();
        let relayer = EvmSigner::from_hex(&self.a.relayer_evm_key).map(|r| format!("{:?}", r.address)).unwrap_or_default();
        let mut evm = serde_json::Map::new();
        for (chain, rpc) in [(SEPOLIA, self.sepolia()), (BASE_SEPOLIA, self.a.base_rpc.clone())] {
            let e = Evm::new(&rpc);
            let bal = |addr: &str| {
                if addr.is_empty() {
                    Value::Null
                } else {
                    e.balance(addr).map(|w| json!(w.to_string())).unwrap_or(Value::Null)
                }
            };
            evm.insert(
                chain.to_string(),
                json!({"deployer": {"address": deployer, "wei": bal(&deployer)}, "relayer": {"address": relayer, "wei": bal(&relayer)}}),
            );
        }
        json!({"at_unix": unix(), "near": near, "evm": evm})
    }

    /// Reads the balances a drill needs against the floors; returns the shortfalls (empty: ready).
    fn guard_balances(&self) -> Result<Vec<Value>> {
        let mut low = Vec::new();
        for role in ["hq", "acme", "agent-a", "keeper", "relayer"] {
            let who = if role == "hq" { HQ.to_string() } else { acct(role) };
            let b = self.near_balance(&who);
            let liquid = b["liquid_yocto"].as_str().and_then(|s| s.parse::<u128>().ok());
            if let Some(s) = near_shortfall(role, &who, liquid) {
                low.push(s);
            }
        }
        // The EVM deployer and the relayer must each hold enough on both chains: the deployer the deploys,
        // the relayer the pauses. A derived deployer is funded from the base, so the base deployer must cover
        // the funding; the relayer is funded directly.
        let deployer = self.deployer()?;
        let base = EvmSigner::from_hex(&self.base_evm_key()?)?;
        let relayer = EvmSigner::from_hex(&self.a.relayer_evm_key)?;
        for (chain, rpc) in [(SEPOLIA, self.sepolia()), (BASE_SEPOLIA, self.a.base_rpc.clone())] {
            let e = Evm::new(&rpc);
            let gas_price = e.gas_price().unwrap_or(2_000_000_000);
            let l1 = if chain == BASE_SEPOLIA { 2_000_000_000_000_000u128 } else { 0 };
            let deploy_need = deploy_need(chain, gas_price);
            let relay_need = EvmSigner::transfer_reserve(gas_price, l1) + 400_000 * gas_price * 3;
            // The base funds the derived deployer: its own balance must cover the funding the deploy needs
            // (less whatever the derived address already holds from an earlier drill).
            let have_derived = e.balance(&format!("{:?}", deployer.address)).unwrap_or(0);
            let to_fund = deploy_need.saturating_sub(have_derived);
            let base_have = e.balance(&format!("{:?}", base.address)).unwrap_or(0);
            if let Some(s) = evm_shortfall("evm-deployer", chain, &format!("{:?}", base.address), base_have, to_fund) {
                low.push(s);
            }
            let relayer_have = e.balance(&format!("{:?}", relayer.address)).unwrap_or(0);
            if let Some(s) = evm_shortfall("evm-relayer", chain, &format!("{:?}", relayer.address), relayer_have, relay_need) {
                low.push(s);
            }
        }
        Ok(low)
    }

    // ------------------------------------------------------------------------------------------- steps

    /// A fresh Acme Lend on both chains, deployed by the per-drill deployer (new CREATE2 addresses). The
    /// deployer is funded from the base deployer first where it needs it.
    fn evm(&mut self) -> Result<()> {
        let deployer = self.deployer()?;
        let base = EvmSigner::from_hex(&self.base_evm_key()?)?;
        let zero = "0x0000000000000000000000000000000000000000";
        let (root, forge, w) = (&self.a.root, &self.a.forge_bin, &self.a.work);
        for (chain, rpc) in [(SEPOLIA, self.sepolia()), (BASE_SEPOLIA, self.a.base_rpc.clone())] {
            let e = Evm::new(&rpc);
            let gas_price = e.gas_price().unwrap_or(2_000_000_000);
            let need = deploy_need(chain, gas_price);
            let have = e.balance(&format!("{:?}", deployer.address))?;
            if have < need {
                let tx = base.transfer(&e, chain, deployer.address, need - have)?;
                e.wait_receipt(&tx, Duration::from_secs(180))?;
                println!("  funded the drill deployer {:?} on {chain}: +{} ({tx})", deployer.address, wei_eth(need - have));
            }
        }
        let key = deployer.secret_hex();
        let by = Broadcaster::Key(&key);
        let (sepolia, base_rpc) = (self.sepolia(), self.a.base_rpc.clone());
        let dep_a =
            forge_retry(deployer.address, &sepolia, "Acme on Sepolia", || demo::deploy_acme(root, forge, &sepolia, &by, zero, &w.join("acme-sepolia.json")))?;
        let dep_b = forge_retry(deployer.address, &base_rpc, "Acme on Base Sepolia", || {
            demo::deploy_acme(root, forge, &base_rpc, &by, zero, &w.join("acme-base-sepolia.json"))
        })?;
        let same = ["AcmeGuard", "AcmeLend", "AcmeLendImpl", "AcmeVault", "AcmeVaultImpl", "AcmeLendFixed", "AcmeLendFinal", "AcmeVaultFixed"]
            .iter()
            .all(|k| dep_a[k] == dep_b[k]);
        if !same {
            return Err(eyre!("the Acme addresses differ between Sepolia and Base Sepolia"));
        }
        println!("fresh Acme Lend on {SEPOLIA} and {BASE_SEPOLIA} (same addresses: {same}); drill deployer {:?}", deployer.address);
        self.s["evm"] = json!({"acme": dep_a, "acme_base_sepolia": dep_b, "deployer": format!("{:?}", deployer.address), "same_addresses": same,
                               "chains": {"verifies_and_pauses": SEPOLIA, "pauses": BASE_SEPOLIA}});
        Ok(())
    }

    /// A recent Sepolia block whose state has the fresh fixture.
    fn anchor(&mut self) -> Result<()> {
        let evm = Evm::new(&self.sepolia());
        let last = self.s["evm"]["acme"]["AcmeVaultFixed"].clone();
        let n = crate::live::poll("a Sepolia block with the fixture", Duration::from_secs(600), Duration::from_secs(12), || {
            let head = evm.call("eth_blockNumber", json!([])).ok().and_then(|v| crate::evm::hex_u64(&v).ok())?;
            let n = head.checked_sub(8)?;
            let has = evm.call("eth_getCode", json!([last, format!("{n:#x}")])).ok().and_then(|c| c.as_str().map(|c| c.len() > 2)).unwrap_or(false);
            has.then_some(n)
        })?;
        let b = evm.read("eth_getBlockByNumber", json!([format!("{n:#x}"), false]))?;
        println!("anchor: Sepolia block {n} ({})", b["hash"]);
        self.s["anchor"] = json!({"block": n, "hash": b["hash"], "timestamp": crate::evm::hex_u64(&b["timestamp"])?, "chosen_unix": unix()});
        Ok(())
    }

    /// The drill program's manifest and base witness (while the anchor is recent), and agent A's bundle and
    /// witness (cached in A's MCP state so the MCP submit seals against it). The program's id is the
    /// contract's next.
    fn prebuild(&mut self) -> Result<()> {
        let t = Instant::now();
        let n = self.s["anchor"]["block"].as_u64().ok_or_else(|| eyre!("no anchor"))?;
        let h: B256 = self.s["anchor"]["hash"].as_str().unwrap_or_default().parse()?;
        let evm = Evm::new(&self.sepolia());
        let head = crate::evm::hex_u64(&evm.read("eth_blockNumber", json!([]))?)?;
        if head > n + 90 {
            self.set_done("evm", false);
            self.set_done("anchor", false);
            return Err(eyre!("the anchor {n} is {} blocks old; public RPCs prove about 128 back: the next run picks a fresh anchor", head - n));
        }
        let next =
            self.near.view(&self.dep.antibody, "get_stats", json!({}))?["next_program"].as_u64().ok_or_else(|| eyre!("get_stats: no next_program"))? as u32;
        let deadline = unix() + DRILL_DEADLINE_SECS;
        let w = self.a.work.clone();
        let dep_a = self.s["evm"]["acme"].clone();
        let mut m = demo::acme_manifest(&dep_a, &evm, n, h, &acct("acme"), &TESTNET, acme_settlement(deadline))?;
        // The drill's own program name (docs/drill.md): `antibody judge` labels it a drill by this.
        m.program.name = format!("{}{}", antibody_cli::judge::DRILL_PROGRAM_PREFIX, self.date);
        m.program.description =
            "An automated daily drill on a fresh Acme Lend instance (DELIBERATELY VULNERABLE TEST FIXTURE): the one moment, run again.".into();
        let mj = m.canonical();
        std::fs::write(w.join("manifest.json"), &mj)?;
        std::fs::write(w.join("terms.txt"), TERMS)?;
        let acme = self.ctx_of("acme");
        let key = format!("{}/{next}", self.dep.antibody);
        let tb = Instant::now();
        let (base, meta) = cmd::build_base(&acme, &mj, &key)?;
        // Agent A's bundle at the fresh addresses (scripts/demo/acme_bundles.py), and A's witness, cached in
        // A's MCP state so the MCP server seals against it.
        let fx = w.join("submissions");
        let st = Command::new("python3").arg(self.a.root.join("scripts/demo/acme_bundles.py")).arg(w.join("acme-sepolia.json")).arg(&fx).status()?;
        if !st.success() {
            return Err(eyre!("acme_bundles.py failed"));
        }
        let mcp = Ctx { state_dir: self.mcp_dir().join(".antibody"), ..self.ctx_of("agent-a") };
        let (abytes, _, receipt, _) = cmd::build_witness(&mcp, None, &mj, &fx.join("A.bundle.json"), None, &key)?;
        let r = serde_json::to_value(&receipt)?;
        println!(
            "prebuilt the drill program {next}'s base ({} B) and agent A's witness ({} B, local verdict {}) in {:.0} s",
            base.len(),
            abytes.len(),
            r["kind"],
            t.elapsed().as_secs_f64()
        );
        self.s["prebuild"] = json!({"program_id": next, "deadline": deadline, "base_bytes": base.len(), "base_cached": meta["cached"],
                                    "a_witness_bytes": abytes.len(), "a_local_verdict": r["kind"], "a_broken_mask": r["broken_mask"], "seconds": tb.elapsed().as_secs_f64()});
        Ok(())
    }

    /// Waits until the light client holds the anchor, and checks the hash it holds there.
    fn light_client(&mut self) -> Result<()> {
        let n = self.s["anchor"]["block"].as_u64().unwrap_or(0);
        let want: B256 = self.s["anchor"]["hash"].as_str().unwrap_or_default().parse()?;
        let t = Instant::now();
        let lc = self.dep.light_client.clone().ok_or_else(|| eyre!("no light client in the deployments"))?;
        crate::live::poll(&format!("{lc} to reach block {n}"), Duration::from_secs(5_400), Duration::from_secs(30), || {
            lc_last_block(&self.near, &lc).ok().filter(|b| *b >= n)
        })?;
        let got = lc_block_hash(&self.near, &lc, n)?;
        if got != Some(want) {
            return Err(eyre!("{lc} holds {got:?} for block {n}, not the anchor's {want:#x}"));
        }
        println!("{lc} holds the anchor (block {n}) after {:.0} s", t.elapsed().as_secs_f64());
        self.s["light_client"] = json!({"contract": lc, "anchor_confirmed_unix": unix(), "waited_seconds": t.elapsed().as_secs_f64()});
        Ok(())
    }

    /// The drill program: the escrow wrapped, the NEP-413 project key, the manifest committed.
    fn program(&mut self) -> Result<()> {
        let acme = self.ctx_of("acme");
        let want = self.s["prebuild"]["program_id"].as_u64().ok_or_else(|| eyre!("no prebuilt program id"))? as u32;
        let escrow = cmd::parse_tokens(ESCROW, 24)?;
        if wnear_of(&acme, &acct("acme"))? < escrow {
            self.ensure_liquid(&acct("acme"), escrow + FREE_FOR_RUNS)?;
            wrap(&acme, escrow)?;
        }
        self.ensure_liquid(&acct("acme"), FREE_FOR_RUNS + 100 * MILLI)?;
        let project_key = self.a.work.join("keys/acme.x25519.json");
        let derived = cmd::keygen_x25519_nep413(&acme, &project_key)?;
        let manifest = self.a.work.join("manifest.json");
        let created = cmd::program_create(&acme, &manifest, ESCROW, &project_key, None)?;
        let pid = created["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no program id: {created}"))? as u32;
        if pid != want {
            return Err(eyre!(
                "program {pid} was created, but the witnesses were built for {want} (the contract's next program moved: the next run picks it up)"
            ));
        }
        println!("drill program {pid} created: {}", created["tx"]["tx"]);
        self.s["program"] = json!({"program_id": pid, "name": format!("{}{}", antibody_cli::judge::DRILL_PROGRAM_PREFIX, self.date),
                                   "manifest_sha256": hex::encode(sha256(&std::fs::read(&manifest)?)), "anchor_block": self.s["anchor"]["block"], "anchor_hash": self.s["anchor"]["hash"],
                                   "create_tx": created["tx"]["tx"], "anchor": created["anchor"], "project_key": {"public_key": derived["public_key"], "nep413": derived["nep413"]}, "terms": TERMS});
        Ok(())
    }

    /// The program's adapter on both chains (the derived deployer), made Acme's pauser and the proxies'
    /// admin for the MPC key near-guardian pins; then the registration.
    fn guardian(&mut self) -> Result<()> {
        let guardian = self.dep.guardian.clone().ok_or_else(|| eyre!("no guardian"))?;
        let key = self.key();
        let path = format!("antibody/{}/{key}", acct("acme"));
        let address = mpc_address(&self.near, &guardian, &path)?;
        let deployer = self.deployer()?;
        let evm_key = deployer.secret_hex();
        let by = Broadcaster::Key(&evm_key);
        let (w, root, forge) = (&self.a.work, &self.a.root, &self.a.forge_bin);
        let (sepolia, base) = (self.sepolia(), self.a.base_rpc.clone());
        let ad_a = forge_retry(deployer.address, &sepolia, "the adapter on Sepolia", || {
            demo::deploy_adapter(root, forge, &sepolia, &by, &address, &key, &w.join("acme-sepolia.json"), &w.join("adapter-sepolia.json"))
        })?;
        let ad_b = forge_retry(deployer.address, &base, "the adapter on Base Sepolia", || {
            demo::deploy_adapter(root, forge, &base, &by, &address, &key, &w.join("acme-base-sepolia.json"), &w.join("adapter-base-sepolia.json"))
        })?;
        let adapter = ad_a["adapter"].as_str().unwrap_or_default().to_lowercase();
        if ad_b["adapter"].as_str().unwrap_or_default().to_lowercase() != adapter {
            return Err(eyre!("the adapter's address differs between chains"));
        }
        let dep = &self.s["evm"]["acme"];
        let proxies: Vec<String> = ["AcmeLend", "AcmeVault"].iter().map(|k| dep[*k].as_str().unwrap_or_default().to_lowercase()).collect();
        let chains = json!([{"chain_id": SEPOLIA, "adapter": adapter, "proxies": proxies}, {"chain_id": BASE_SEPOLIA, "adapter": adapter, "proxies": proxies}]);
        let acme = self.ctx_of("acme");
        self.ensure_liquid(&acct("acme"), 210 * MILLI)?;
        let reg = acme.near.function_call(&acme.signer()?, &guardian, "register", json!({"program_key": key, "chains": chains}), 100, ONE_NEAR / 10)?;
        cmd::ensure_ok(&acme, &reg, "register")?;
        let pinned = crate::live::poll("near-guardian to pin the MPC key", Duration::from_secs(180), Duration::from_secs(5), || {
            self.near.view(&guardian, "get_program", json!({"program_key": key, "project": acct("acme")})).ok().filter(|p| p["address"].is_string())
        })?;
        if pinned["address"].as_str().map(|x| x.eq_ignore_ascii_case(&address)) != Some(true) {
            return Err(eyre!("near-guardian pinned {} for the program, not the adapter's {address}", pinned["address"]));
        }
        println!("adapter {adapter} on {SEPOLIA} and {BASE_SEPOLIA}; near-guardian pinned {}", pinned["address"]);
        self.s["guardian"] = json!({"adapter": adapter, "proxies": proxies, "guardian_address": pinned["address"], "mpc": crate::live::MPC, "path": path,
                                    "register_tx": antibody_cli::near::tx_hash(&reg), "adapters": {"sepolia": ad_a, "base_sepolia": ad_b}});
        Ok(())
    }

    /// Activation (the first real OutLayer run, the sealing key, T1), then the opening. The drill carries no
    /// known issue: it proves one Critical and pays it.
    fn activate(&mut self) -> Result<()> {
        let acme = self.ctx_of("acme");
        let pid = self.pid();
        let t = Instant::now();
        self.ensure_liquid(&acct("acme"), FREE_FOR_RUNS)?;
        let act = cmd::program_activate(&acme, pid, &[], true)?;
        let act_run = act["run_id"].as_u64().unwrap_or_default();
        demo::wait_t1_for(&acme, act_run, T1_WAIT)?;
        let p = acme.program(pid)?;
        if p["sealing_pk"].is_null() {
            return Err(eyre!("activation run {act_run} is T1 but the program has no sealing key: {}", act["result"]));
        }
        cmd::program_open(&acme, pid)?;
        println!("activated in {:.1} s: sealing key {}; program {pid} open", t.elapsed().as_secs_f64(), p["sealing_pk"]);
        self.s["activation"] = json!({"run_id": act_run, "sealing_pk": p["sealing_pk"], "key_epoch": p["key_epoch"], "base": act["base"], "seconds": t.elapsed().as_secs_f64(), "tx": act["tx"]["tx"]});
        Ok(())
    }

    /// Agent A's funding and MCP key (reused across drills in the work dir).
    fn agent(&mut self) -> Result<()> {
        let c = self.ctx_of("agent-a");
        let me = acct("agent-a");
        self.ensure_liquid(&me, FREE_FOR_RUNS)?;
        let have = cmd::account(&c, &me)?;
        if wnear_of(&c, &me)? + cmd::u128_of(&have["tokens"]) < 2 * MILLI {
            wrap(&c, 3 * MILLI)?;
        }
        // A's MCP server signs with a function-call key, which attaches no deposit: A's credit covers its
        // commit and run up front.
        let credit = cmd::parse_near("0.07")?;
        let near_credit = cmd::u128_of(&have["near"]);
        if near_credit < credit {
            cmd::storage_deposit(&c, &cmd::fmt_tokens(credit - near_credit, 24))?;
        }
        let tokens = cmd::u128_of(&cmd::account(&c, &me)?["tokens"]);
        if tokens < 2 * MILLI {
            self.ensure_liquid(&me, FREE_FOR_RUNS)?;
            cmd::deposit(&c, &cmd::fmt_tokens(2 * MILLI - tokens, 24))?;
        }
        let agent_key = self.a.work.join("keys/agent-a.fc-key.json");
        if !agent_key.exists() {
            cmd::agent_key_add(&self.ctx_of("agent-a"), 1.0, &agent_key)?;
        }
        self.s["agent"] = cmd::account(&c, &me)?;
        Ok(())
    }

    /// Agent A over MCP; its verdict T1, its instant payment, and the pause on both chains. Writes the
    /// `one_moment` section the API's `moment()` reads.
    fn one_moment(&mut self) -> Result<()> {
        let pid = self.pid();
        let root_ctx = self.root_ctx();
        let fx = self.a.work.join("submissions");
        let state_a = self.mcp_dir();
        std::fs::create_dir_all(&state_a)?;
        let rpc = self.dep.rpc.clone().unwrap_or_default();
        let net = ["--network", "testnet", "--rpc", rpc.as_str()];
        let dep_file = self.a.root.join("deployments/near-testnet.json");
        let events_file = self.services_dir().join("events.jsonl");
        let tx_of = |event: &str, commit: u64| {
            crate::indexer::read_events(&events_file, 0, usize::MAX)
                .into_iter()
                .find(|e| e["event"] == event && e["data"]["commit_id"] == json!(commit))
                .and_then(|e| e["tx"].as_str().map(String::from))
                .unwrap_or_default()
        };
        let a_commit = match self.s["mcp"]["commit_id"].as_u64() {
            Some(c) => c,
            None => {
                self.ensure_liquid(&acct("agent-a"), FREE_FOR_RUNS)?;
                let mcp = demo::mcp_submit(
                    &self.a.antibody_bin,
                    &net,
                    &dep_file,
                    &acct("agent-a"),
                    &self.a.work.join("keys/agent-a.fc-key.json"),
                    Some(self.a.sepolia_rpc.as_str()),
                    &state_a,
                    pid,
                    &fx.join("A.bundle.json"),
                )?;
                let c = mcp["result"]["commit"]["commit_id"].as_u64().ok_or_else(|| eyre!("MCP submit: no commit id: {mcp}"))?;
                self.s["mcp"] = json!({"tools": mcp["mcp_tools"], "tool_names": mcp["tool_names"], "commit_id": c, "witness_source": mcp["result"]["commit"]["witness_source"],
                                       "commit_tx": mcp["result"]["commit"]["tx"]["tx"], "reveal_tx": mcp["result"]["reveal"]["tx"]["tx"]});
                self.save()?;
                c
            }
        };
        let a_run = root_ctx.commit(a_commit)?["run"].as_u64().unwrap_or_default();
        demo::wait_t1_for(&root_ctx, a_run, T1_WAIT)?;
        self.ensure_liquid(&acct("keeper"), FREE_FOR_RUNS)?;
        let keeper_a = cmd::keeper(&self.ctx_of("keeper"), pid)?;
        let steps = keeper_a["steps"].as_array().cloned().unwrap_or_default();
        if !steps.iter().any(|x| x.get("pause_ticket").is_some())
            && self.near.view(&self.dep.antibody, "get_program", json!({"program_id": pid}))?["ticket_rounds"] == json!([])
        {
            let refused: Vec<&Value> = steps.iter().filter(|x| x.get("pause_ticket_refused").is_some()).collect();
            return Err(eyre!("no pause ticket for A's Critical: {}", serde_json::to_string(&refused)?));
        }
        let paid_a = cmd::paid_of(&root_ctx.commit(a_commit)?);
        let relayer_file = self.relayer_file();
        let key = self.key();
        let mut confirmed: BTreeMap<String, Value> = BTreeMap::new();
        crate::live::poll("pause() on both chains", Duration::from_secs(900), Duration::from_secs(3), || {
            for v in permits(&relayer_file).values().filter(|v| v["program_key"] == json!(key)) {
                if v["kind"] == "pause"
                    && v["status"] == "confirmed"
                    && v["targets_paused"].as_array().is_some_and(|p| !p.is_empty() && p.iter().all(|x| x == true))
                {
                    confirmed.entry(v["chain_id"].to_string()).or_insert_with(|| v.clone());
                }
            }
            (confirmed.len() == 2).then_some(())
        })?;
        let commit_tx = self.s["mcp"]["commit_tx"].as_str().map(String::from).filter(|t| !t.is_empty()).unwrap_or_else(|| tx_of("commit", a_commit));
        let reveal_tx = self.s["mcp"]["reveal_tx"].as_str().map(String::from).filter(|t| !t.is_empty()).unwrap_or_else(|| tx_of("reveal", a_commit));
        let pay_tx = keeper_a["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["paid"] == json!(a_commit))
            .and_then(|s| s["tx"].as_str())
            .map(String::from)
            .unwrap_or_else(|| tx_of("paid", a_commit));
        let answered_at = root_ctx.view("get_run", json!({"run_id": a_run}))?["outcome"]["Answered"]["at"].as_u64();
        let mut near_at: BTreeMap<&str, u128> = BTreeMap::new();
        if let Some(at) = answered_at {
            near_at.insert("verdict", at as u128 * 1_000_000_000);
        }
        crate::live::poll("the indexer to reach A's payment", Duration::from_secs(300), Duration::from_secs(3), || {
            for e in crate::indexer::read_events(&events_file, 0, usize::MAX) {
                let (Some(ts), d) = (e["block_ts_ns"].as_u64(), &e["data"]) else { continue };
                let k = match e["event"].as_str() {
                    Some("commit") if d["commit_id"] == json!(a_commit) => "commit",
                    Some("run_answered") if d["run_id"] == json!(a_run) => "verdict_event",
                    Some("t1_confirmed") if d["run_id"] == json!(a_run) => "t1",
                    Some("paid") if d["commit_id"] == json!(a_commit) => "paid",
                    _ => continue,
                };
                near_at.entry(k).or_insert(ts as u128);
            }
            (["commit", "t1", "paid"].iter().all(|k| near_at.contains_key(k)) && (near_at.contains_key("verdict") || near_at.contains_key("verdict_event")))
                .then_some(())
        })?;
        let verdict_from = if near_at.contains_key("verdict_event") { "its event's block" } else { "near-antibody's record of the answer, to the second" };
        if let Some(e) = near_at.get("verdict_event").copied() {
            near_at.insert("verdict", e);
        }
        let committed_ns = near_at["commit"];
        let since = |ns: u128| ns.saturating_sub(committed_ns) as f64 / 1e9;
        let ms = |v: &Value, k: &str| v[k].as_u64().map(|m| since(m as u128 * 1_000_000));
        let (t_verdict, t_t1, t_paid) = (since(near_at["verdict"]), since(near_at["t1"]), since(near_at["paid"]));
        let at = |c: &str, k: &str| confirmed.get(c).and_then(|v| ms(v, k));
        let signed = |c: &str| confirmed.get(c).and_then(|v| v["signed_ns"].as_u64()).map(|n| since(n as u128));
        let (s, b) = (SEPOLIA.to_string(), BASE_SEPOLIA.to_string());
        let guard = self.s["evm"]["acme"]["AcmeGuard"].as_str().unwrap_or_default().to_string();
        let paused_both = Evm::new(&self.sepolia()).paused(&guard)? && Evm::new(&self.a.base_rpc).paused(&guard)?;
        let run_view = cmd::run_view(&cmd::run(&root_ctx, a_run)?);
        println!(
            "A, from its commit's block: verdict {t_verdict:.1} s, T1 {t_t1:.1} s, paid {} wNEAR at {t_paid:.1} s; paused on both chains: {paused_both}",
            cmd::fmt_tokens(paid_a, 24)
        );
        self.s["submissions"] = json!([{"agent": "A (MCP)", "commit": a_commit, "run": a_run, "verdict": run_view["kind"], "tier": run_view["tier"], "keys": run_view["keys"],
                                        "state": root_ctx.commit(a_commit)?["state"], "paid_raw": paid_a.to_string(), "seconds": t_verdict, "ok": paid_a > 0,
                                        "txs": {"commit": commit_tx, "reveal": reveal_tx, "pay": pay_tx}}]);
        self.s["one_moment"] = json!({"commit": a_commit, "seconds_from_commit": {"verdict": t_verdict, "t1": t_t1, "paid": t_paid,
                                      format!("permit_signed_{SEPOLIA}"): signed(&s), format!("permit_signed_{BASE_SEPOLIA}"): signed(&b),
                                      format!("paused_{SEPOLIA}"): at(&s, "confirmed_ms"), format!("paused_{BASE_SEPOLIA}"): at(&b, "confirmed_ms")},
                                      "clock": "from the commit's NEAR block: T1 and the payment at their NEAR blocks (the indexer's events), the verdict at its answer's block, each permit when near-guardian signed it, each pause when its chain's receipt came back",
                                      "verdict_from": verdict_from,
                                      "pay_tx": pay_tx, "paid_raw": paid_a.to_string(), "instant": root_ctx.commit(a_commit)?["state"]["Done"]["Paid"]["instant"],
                                      "paused_both": paused_both, "keeper": keeper_a, "permits": permits(&relayer_file)});
        Ok(())
    }

    /// Every expected outcome, the cost line, and the API shape into the results file.
    fn results(&mut self) -> Result<()> {
        let pid = self.pid();
        let one = &self.s["one_moment"];
        let paid = cmd::u128_of(&one["paid_raw"]);
        let ok = one["paused_both"] == json!(true)
            && one["instant"] == json!(true)
            && paid == 100 * MILLI
            && self.s["submissions"][0]["verdict"] == json!("Violation")
            && self.s["submissions"][0]["tier"] == json!("T1")
            && cmd::state_label(&self.s["submissions"][0]["state"]) == "Done:Paid";
        self.s["after"] = self.balances();
        self.s["cost"] = self.cost();
        self.s["network"] = json!("testnet");
        self.s["deployment"] = json!({"network": "testnet", "near_antibody": self.dep.antibody, "guardian": self.dep.guardian, "token": self.dep.token});
        self.s["all_ok"] = json!(ok);
        self.s["finished_unix"] = json!(unix());
        let file = self.a.results_dir.join(format!("drill-{}.json", self.date));
        std::fs::create_dir_all(&self.a.results_dir)?;
        std::fs::write(&file, serde_json::to_vec_pretty(&self.s)?)?;
        std::fs::write(self.a.work.join("results.json"), serde_json::to_vec_pretty(&self.s)?)?;
        println!("\n{} — drill {}: program {pid}, {}", if ok { "ALL OK" } else { "MISMATCH" }, self.date, file.display());
        if !ok {
            return Err(eyre!("the drill finished with mismatches (see {})", file.display()));
        }
        Ok(())
    }

    /// The cost line from the balance deltas (`cost_from`), with the OutLayer run deposit read from the
    /// contract defaults.
    fn cost(&self) -> Value {
        // Two real OutLayer runs are requested (the activation and A's verify); the challenge-free instant
        // payout needs no second run.
        let deposit =
            cmd::u128_of(&self.near.view(&self.dep.antibody, "get_defaults", json!({})).map(|d| d["verifier"]["deposit"].clone()).unwrap_or(Value::Null));
        cost_from(&self.s["before"], &self.s["after"], deposit, 2)
    }

    // ------------------------------------------------------------------------------------- services

    /// The indexer, the T1 relayer and the permit relayer, as `live` runs them, from the program's creation
    /// on. The drill reads A's steps and the pause permits from their state.
    fn start_services(&mut self) -> Result<()> {
        if !self.threads.is_empty() {
            return Ok(());
        }
        let dir = self.services_dir();
        std::fs::create_dir_all(&dir)?;
        let guardian = self.dep.guardian.clone().ok_or_else(|| eyre!("no guardian in the deployments"))?;
        let ix = crate::indexer::Indexer {
            near: self.near.clone(),
            contracts: vec![self.dep.antibody.clone(), guardian.clone()],
            antibody: self.dep.antibody.clone(),
            guardian: Some(guardian.clone()),
            source: crate::indexer::Source::FastNear { base: FASTNEAR_TX.into() },
            state_dir: dir.clone(),
        };
        let t1 = T1Relayer {
            ctx: self.ctx_of("relayer"),
            source: Source::OutLayer { api: antibody_cli::tee::DEFAULT_API.into() },
            state_dir: dir.clone(),
            max_attempts: 60,
            collateral: "auto".into(),
        };
        let relayer = Relayer {
            near: self.near.clone(),
            guardian,
            antibody: self.dep.antibody.clone(),
            chains: BTreeMap::from([(SEPOLIA, (self.sepolia(), ETHERSCAN.to_string())), (BASE_SEPOLIA, (self.a.base_rpc.clone(), BASESCAN.to_string()))]),
            signer: Some(EvmSigner::from_hex(&self.a.relayer_evm_key)?),
            state_file: self.relayer_file(),
        };
        let stop = self.stop.clone();
        self.threads.push(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = ix.tick() {
                    eprintln!("indexer: {e:#}");
                }
                // The services share one public NEAR RPC's rate limit.
                std::thread::sleep(Duration::from_secs(5));
            }
        }));
        let stop = self.stop.clone();
        self.threads.push(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = t1.tick() {
                    eprintln!("t1 relayer: {e:#}");
                }
                std::thread::sleep(Duration::from_secs(8));
            }
        }));
        let stop = self.stop.clone();
        self.threads.push(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = relayer.tick() {
                    eprintln!("permit relayer: {e:#}");
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        }));
        Ok(())
    }

    fn stop_services(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in std::mem::take(&mut self.threads) {
            let _ = h.join();
        }
    }

    // --------------------------------------------------------------------------------------- cleanup

    /// Closes drill programs that are exitable (past their deadline and the exit grace), so their escrow
    /// returns to the project: `exit`, then `close_commits` a page at a time.
    fn cleanup(&self) -> Result<Value> {
        let acme = self.ctx_of("acme");
        let n = self.near.view(&self.dep.antibody, "get_stats", json!({}))?["next_program"].as_u64().unwrap_or(0) as u32;
        let now = unix();
        let mut closed = Vec::new();
        for id in 0..n {
            let meta = cmd::meta(&acme, id)?;
            if !meta["name"].as_str().is_some_and(antibody_cli::judge::is_drill) {
                continue;
            }
            let p = acme.program(id)?;
            let phase = p["phase"].as_str().unwrap_or_default().to_string();
            if matches!(phase.as_str(), "Exited" | "Rejected" | "Pending") && p["close_cursor"].as_u64() >= p["commits"].as_u64() {
                continue;
            }
            let deadline = p["params"]["windows"]["deadline"].as_u64().unwrap_or(u64::MAX);
            let grace = p["params"]["windows"]["exit_grace"].as_u64().unwrap_or(60);
            if phase != "Exited" && now < deadline + grace {
                closed.push(json!({"program_id": id, "name": meta["name"], "skipped": "not yet exitable", "exitable_at": deadline + grace}));
                continue;
            }
            // The project exits its own program; `keeper` first moves every commit to final or allocated.
            self.ensure_liquid(&acct("acme"), FREE_FOR_RUNS)?;
            self.ensure_liquid(&acct("keeper"), FREE_FOR_RUNS)?;
            let _ = cmd::keeper(&self.ctx_of("keeper"), id);
            let r = match cmd::program_exit(&acme, id) {
                Ok(v) => json!({"program_id": id, "name": meta["name"], "exited": v}),
                Err(e) => json!({"program_id": id, "name": meta["name"], "error": e.to_string()}),
            };
            closed.push(r);
        }
        Ok(json!({"closed": closed, "at_unix": now}))
    }
}

/// wei as whole ether, 18 decimals.
fn wei_eth(wei: u128) -> String {
    cmd::fmt_tokens(wei, 18)
}

/// Gas forge's simulation counts for a drill's deployments on one chain, with margin: its gas limits on
/// Sepolia on 2026-11-08 were 94.2M for Acme's 29 transactions and 10.3M for the adapter's 4, nine times
/// their simulation.
const DEPLOY_SIM_GAS: u128 = 13_000_000;

/// What a drill's deployments on a chain may need at the node's gas price: forge's gas limits (its
/// simulation times `FORGE_GAS_MULTIPLIER_PCT`) at forge's fee cap ([`demo::forge_max_fee`]), a fifth more
/// for a price that moves, and an OP Stack chain's L1 fee margin. A node must see each transaction's limit
/// times its fee cap in the balance before it accepts it. At Sepolia's near-zero price (19 wei on
/// 2026-11-07) the tip is nearly all of it: three times the price alone funded 0.000000000612 ETH.
fn deploy_need(chain: u64, gas_price: u128) -> u128 {
    let l1 = if chain == BASE_SEPOLIA { 2_000_000_000_000_000u128 } else { 0 };
    DEPLOY_SIM_GAS * demo::FORGE_GAS_MULTIPLIER_PCT / 100 * demo::forge_max_fee(gas_price) * 6 / 5 + l1
}

/// A NEAR account's shortfall against its drill floor, or None when it is ready. `liquid` is its NEAR free
/// of its storage stake (None: the account does not exist).
fn near_shortfall(role: &str, account: &str, liquid: Option<u128>) -> Option<Value> {
    let floor = near_floor(role);
    match liquid {
        Some(l) if l >= floor => None,
        Some(l) => Some(json!({"kind": "near", "role": role, "account": account, "have_yocto": l.to_string(), "need_yocto": floor.to_string(),
                               "have_near": cmd::fmt_tokens(l, 24), "need_near": cmd::fmt_tokens(floor, 24)})),
        None => Some(json!({"kind": "near", "role": role, "account": account, "error": "account not found"})),
    }
}

/// An EVM account's shortfall against what a drill needs on a chain, or None when it is ready.
fn evm_shortfall(kind: &str, chain: u64, address: &str, have_wei: u128, need_wei: u128) -> Option<Value> {
    (have_wei < need_wei).then(|| {
        json!({"kind": kind, "chain_id": chain, "address": address, "have_wei": have_wei.to_string(), "need_wei": need_wei.to_string(),
               "have_eth": wei_eth(have_wei), "need_eth": wei_eth(need_wei)})
    })
}

/// The cost line from a before/after balance snapshot (`balances()`): each signer's NEAR delta, the deployer
/// and relayer ETH delta per chain, and `runs` OutLayer run deposits at `deposit` each (gross; the unused
/// part is refunded, so it is already netted into the NEAR deltas).
fn cost_from(before: &Value, after: &Value, deposit: u128, runs: u128) -> Value {
    let yocto = |v: &Value| v.as_str().and_then(|s| s.parse::<i128>().ok()).unwrap_or(0);
    let mut near = Vec::new();
    for role in ["hq", "acme", "agent-a", "keeper", "relayer"] {
        let (b, a) = (&before["near"][role], &after["near"][role]);
        let spent = yocto(&b["balance"]["amount_yocto"]) - yocto(&a["balance"]["amount_yocto"]);
        near.push(json!({"role": role, "account": b["account"], "spent_yocto": spent.to_string(), "spent_near": fmt_signed(spent, 24)}));
    }
    let wei = |v: &Value| v.as_str().and_then(|s| s.parse::<i128>().ok()).unwrap_or(0);
    let mut chains = serde_json::Map::new();
    for chain in [SEPOLIA, BASE_SEPOLIA] {
        let (b, a) = (&before["evm"][chain.to_string()], &after["evm"][chain.to_string()]);
        let spent = |who: &str| {
            let s = wei(&b[who]["wei"]) - wei(&a[who]["wei"]);
            json!({"address": b[who]["address"], "spent_wei": s.to_string(), "spent_eth": fmt_signed(s, 18)})
        };
        chains.insert(chain.to_string(), json!({"deployer": spent("deployer"), "relayer": spent("relayer")}));
    }
    json!({"note": "NEAR: each signer's balance delta over the drill (negative means it received more than it spent, e.g. a payout); ETH: the deployer's and relayer's deltas per chain; OutLayer deposits are gross (the unused part is refunded).",
           "near": near, "sepolia_eth": chains.get(&SEPOLIA.to_string()), "base_sepolia_eth": chains.get(&BASE_SEPOLIA.to_string()),
           "outlayer_run_deposits": {"runs": runs, "deposit_each_yocto": deposit.to_string(), "total_yocto": (runs * deposit).to_string(), "total_near": cmd::fmt_tokens(runs * deposit, 24)}})
}

/// A signed yoctoNEAR amount as whole NEAR (negative kept).
fn fmt_signed(raw: i128, decimals: u32) -> String {
    format!("{}{}", if raw < 0 { "-" } else { "" }, cmd::fmt_tokens(raw.unsigned_abs(), decimals))
}

/// Writes a skipped drill results file (`all_ok: false`, with the reason), so `/v1/status` alerts and
/// `moments()` leaves it out of the site. Returns the file path.
fn write_skipped(results_dir: &Path, date: &str, dep: &Deployments, reason: &str, detail: Value) -> Result<PathBuf> {
    let file = results_dir.join(format!("drill-{date}.json"));
    std::fs::create_dir_all(results_dir)?;
    let s = json!({"network": "testnet", "all_ok": false, "started_unix": unix(), "finished_unix": unix(),
                   "deployment": {"network": "testnet", "near_antibody": dep.antibody, "guardian": dep.guardian, "token": dep.token},
                   "drill": {"skipped": true, "reason": reason, "detail": detail}, "done": []});
    std::fs::write(&file, serde_json::to_vec_pretty(&s)?)?;
    Ok(file)
}

pub fn run(a: DrillArgs) -> Result<Value> {
    std::env::set_var("ANTIBODY_MIN_NEAR_CREDIT", crate::live::MIN_NEAR_CREDIT);
    let dep: Deployments = serde_json::from_str(&std::fs::read_to_string(a.root.join("deployments/near-testnet.json"))?)?;
    let near = Near::new("testnet", dep.rpc.clone());
    std::fs::create_dir_all(a.work.join("keys"))?;
    let date = date_of(unix());
    let s: Value = std::fs::read(a.work.join("drill.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|v: &Value| v["date"] == json!(date))
        .unwrap_or_else(|| json!({"date": date, "done": [], "started_unix": unix()}));
    let mut d = Drill { a, near, dep, s, date: date.clone(), stop: Arc::new(AtomicBool::new(false)), threads: Vec::new() };

    if d.a.cleanup {
        step("drill: cleanup (close exitable drill programs)");
        return d.cleanup();
    }

    // The kill switch: the drill does nothing and says so.
    if let Some(why) = kill_switch(&d.a.work) {
        println!("drill disabled: {why}");
        let file = write_skipped(&d.a.results_dir, &date, &d.dep, "the kill switch is set", json!({"why": why}))?;
        return Ok(json!({"skipped": true, "reason": "kill switch", "why": why, "results_file": file}));
    }

    // The guards: read the balances a drill needs; skip with a reason if any is low.
    let low = d.guard_balances()?;
    if !low.is_empty() {
        println!("drill skipped: {} balance(s) below what a drill needs", low.len());
        for l in &low {
            println!("  {l}");
        }
        let file = write_skipped(&d.a.results_dir, &date, &d.dep, "a balance a drill needs is below its floor", json!({"low": low}))?;
        return Ok(json!({"skipped": true, "reason": "low balance", "low": low, "results_file": file}));
    }
    if d.a.dry_run {
        println!("drill dry run: the guards pass and the kill switch is off; nothing spent.");
        return Ok(json!({"dry_run": true, "ready": true, "date": date}));
    }

    // The balances before the drill, so the cost line is the delta (kept across a resume).
    if !d.s["before"].is_object() {
        d.s["before"] = d.balances();
        d.save()?;
    }

    let services_from = STEPS.iter().position(|s| *s == "program").unwrap_or(0);
    let mut drill_steps = d.s["drill"]["steps"].as_object().cloned().unwrap_or_default();
    for (i, stage) in STEPS.iter().enumerate() {
        if d.done(stage) {
            continue;
        }
        if i >= services_from {
            d.start_services()?;
            d.ensure_liquid(&acct("relayer"), near_floor("relayer"))?;
        }
        step(&format!("drill: {stage}"));
        let t = Instant::now();
        let r = match *stage {
            "evm" => d.evm(),
            "anchor" => d.anchor(),
            "prebuild" => d.prebuild(),
            "light-client" => d.light_client(),
            "program" => d.program(),
            "guardian" => d.guardian(),
            "activate" => d.activate(),
            "agent" => d.agent(),
            "one-moment" => d.one_moment(),
            _ => d.results(),
        };
        let secs = t.elapsed().as_secs_f64();
        match &r {
            Ok(_) => {
                drill_steps.insert(stage.to_string(), json!({"ok": true, "seconds": secs}));
                d.set_done(stage, true);
            }
            Err(e) => {
                drill_steps.insert(stage.to_string(), json!({"ok": false, "seconds": secs, "error": format!("{e:#}").chars().take(500).collect::<String>()}));
            }
        }
        d.s["drill"]["steps"] = Value::Object(drill_steps.clone());
        d.s["drill"]["date"] = json!(date);
        d.s["drill"]["cadence_label"] = json!("daily");
        d.save()?;
        if let Err(e) = r {
            // A failed step ends the drill with all_ok: false and the step's error in the file.
            d.s["all_ok"] = json!(false);
            if !d.s["after"].is_object() {
                d.s["after"] = d.balances();
                d.s["cost"] = d.cost();
            }
            d.s["network"] = json!("testnet");
            d.s["deployment"] = json!({"network": "testnet", "near_antibody": d.dep.antibody, "guardian": d.dep.guardian, "token": d.dep.token});
            let file = d.a.results_dir.join(format!("drill-{date}.json"));
            let _ = std::fs::create_dir_all(&d.a.results_dir);
            let _ = std::fs::write(&file, serde_json::to_vec_pretty(&d.s)?);
            d.stop_services();
            return Err(e.wrap_err(format!("drill step {stage}")));
        }
    }
    d.stop_services();
    Ok(d.s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kill_switch_is_a_file_or_an_env_var() {
        let dir = std::env::temp_dir().join(format!("drill-kill-{}", unix()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::remove_var("ANTIBODY_DRILL_OFF");
        assert!(kill_switch(&dir).is_none());
        let f = dir.join("drill.disabled");
        std::fs::write(&f, "").unwrap();
        assert!(kill_switch(&dir).unwrap().contains("kill switch file"));
        std::fs::remove_file(&f).unwrap();
        std::env::set_var("ANTIBODY_DRILL_OFF", "maintenance");
        assert!(kill_switch(&dir).unwrap().contains("ANTIBODY_DRILL_OFF"));
        std::env::remove_var("ANTIBODY_DRILL_OFF");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_low_balance_is_a_shortfall_and_a_funded_one_is_not() {
        // acme needs 0.9 NEAR free: 0.5 is short, 1.0 is enough, a missing account is a shortfall.
        assert!(near_shortfall("acme", "acme.antibody.testnet", Some(cmd::parse_near("0.5").unwrap())).is_some());
        assert!(near_shortfall("acme", "acme.antibody.testnet", Some(cmd::parse_near("1.0").unwrap())).is_none());
        let miss = near_shortfall("keeper", "keeper.antibody.testnet", None).unwrap();
        assert_eq!(miss["error"], "account not found");
        // The EVM deployer short on a chain, and the relayer with enough.
        let s = evm_shortfall("evm-deployer", SEPOLIA, "0xabc", 1, 1_000).unwrap();
        assert_eq!((s["kind"].as_str(), s["chain_id"].as_u64()), (Some("evm-deployer"), Some(SEPOLIA)));
        assert!(evm_shortfall("evm-relayer", BASE_SEPOLIA, "0xdef", 1_000, 1_000).is_none());
    }

    #[test]
    fn a_drill_deployer_is_funded_for_forges_limits_and_fee_cap() {
        // Sepolia at 16 wei (2026-11-08): 117M gas of limits at forge's cap (twice the price plus the 0.001
        // gwei tip), a fifth more: about 0.00014 ETH, where three times the price alone gave 0.000000000612.
        assert_eq!(demo::forge_max_fee(16), 1_000_032);
        assert_eq!(deploy_need(SEPOLIA, 16), 117_000_000 * 1_000_032 * 6 / 5);
        // It covers the limits forge set that night (94.2M for Acme, 10.3M for the adapter) at that cap.
        assert!(deploy_need(SEPOLIA, 16) > (94_207_698 + 10_259_055) * 1_000_032);
        // Base Sepolia at 0.006 gwei: the same, and the 0.002 ETH L1 fee margin.
        assert_eq!(deploy_need(BASE_SEPOLIA, 6_000_000), 117_000_000 * 13_000_000 * 6 / 5 + 2_000_000_000_000_000);
    }

    #[test]
    fn the_cost_line_is_the_balance_delta() {
        let n = |amount: &str| json!({"balance": {"amount_yocto": amount, "liquid_yocto": amount}});
        let e = |wei: &str| json!({"address": "0xabc", "wei": wei});
        let before = json!({"near": {"hq": {"account": "antibody.testnet", "balance": {"amount_yocto": "0"}}, "acme": n("3000000000000000000000000"),
                             "agent-a": n("1000000000000000000000000"), "keeper": n("0"), "relayer": n("0")},
                            "evm": {SEPOLIA.to_string(): {"deployer": e("10000000000000000"), "relayer": e("5000000000000000")},
                                    BASE_SEPOLIA.to_string(): {"deployer": e("0"), "relayer": e("0")}}});
        let after = json!({"near": {"hq": {"account": "antibody.testnet", "balance": {"amount_yocto": "0"}}, "acme": n("2500000000000000000000000"),
                            "agent-a": n("1100000000000000000000000"), "keeper": n("0"), "relayer": n("0")},
                           "evm": {SEPOLIA.to_string(): {"deployer": e("9000000000000000"), "relayer": e("4900000000000000")},
                                   BASE_SEPOLIA.to_string(): {"deployer": e("0"), "relayer": e("0")}}});
        // deposit 0.02 NEAR, two runs.
        let cost = cost_from(&before, &after, cmd::parse_near("0.02").unwrap(), 2);
        let near = cost["near"].as_array().unwrap();
        let of = |role: &str| near.iter().find(|r| r["role"] == role).unwrap().clone();
        // acme spent 0.5 NEAR; agent-a received 0.1 (its payout), so a negative spend.
        assert_eq!(of("acme")["spent_near"], "0.500000000000000000000000");
        assert_eq!(of("agent-a")["spent_near"], "-0.100000000000000000000000");
        assert_eq!(cost["sepolia_eth"]["deployer"]["spent_wei"], "1000000000000000");
        assert_eq!(cost["sepolia_eth"]["relayer"]["spent_eth"], "0.000100000000000000");
        assert_eq!(cost["outlayer_run_deposits"]["total_near"], "0.040000000000000000000000");
    }
}

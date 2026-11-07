//! `antibody-services live`: the story on NEAR testnet, on our own deployment (deployments/near-testnet.json)
//! and our own fixtures only: Acme Lend (a DELIBERATELY VULNERABLE TEST FIXTURE) on Sepolia, which verifies
//! and pauses, and on Base Sepolia, which only pauses; the practice program (a tutorial) on Sepolia. Real
//! OutLayer runs, DCAP verification on NEAR, the MPC signer and the Ethereum light client; payouts in
//! wNEAR. Every transaction is on a testnet.
//!
//! The stages run in order and record what they made in `<work>/live.json`; a run resumes at the first
//! stage not done, and `--until <stage>` stops after it. Public RPCs prove only recent Sepolia state (about
//! 128 blocks) and the light client confirms a block some minutes after it: `prebuild` builds every witness
//! the story needs while the anchor is recent (the CLI keeps them, `cmd::build_witness`), and the programs
//! are created once the light client has the anchor. When `prebuild` fails, the next run picks a new anchor.

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

use alloy_primitives::{keccak256, B256};
use antibody_cli::{
    cmd,
    ctx::{Ctx, Deployments, PracticeDeployment},
    near::{Action, Near, ONE_NEAR},
    practice,
};
use antibody_rules::settlement::{Fees, Gap, Instant as InstantRule, Known, Pause, Payouts, SWindows, Settlement};
use antibody_verifier::facts::sha256;
use base64::Engine;
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{
    demo::{self, step, Broadcaster, Net},
    evm::{hex_u64, Evm, EvmSigner},
    relayer::Relayer,
    t1::{Source, T1Relayer},
};

pub(crate) const SEPOLIA: u64 = 11_155_111;
pub(crate) const BASE_SEPOLIA: u64 = 84_532;
pub(crate) const TESTNET: Net = Net { chain_id: SEPOLIA, trust: "light-client", label: "testnet" };
/// The deployment's root account; the story's accounts are its sub-accounts.
pub(crate) const HQ: &str = "antibody.testnet";
pub(crate) const MPC: &str = "v1.signer-prod.testnet";
pub(crate) const FASTNEAR_TX: &str = "https://tx.test.fastnear.com";
pub(crate) const ETHERSCAN: &str = "https://sepolia.etherscan.io/tx/";
pub(crate) const BASESCAN: &str = "https://sepolia.basescan.org/tx/";
/// A thousandth of a wNEAR (24 decimals).
pub(crate) const MILLI: u128 = ONE_NEAR / 1_000;
/// The testnet programs' terms texts; their sha256 is committed in each manifest and named by every commit.
pub(crate) const TERMS: &str = "Acme Lend (testnet) terms: a DELIBERATELY VULNERABLE TEST FIXTURE. Payouts in wNEAR on NEAR testnet, a testnet token with no value.";
const PRACTICE_TERMS: &str =
    "Practice program (testnet) terms: a TUTORIAL TARGET whose bug and proof are public. Payouts in wNEAR on NEAR testnet, a testnet token with no value.";
pub(crate) const ESCROW: &str = "0.4";
const PRACTICE_ESCROW: &str = "0.03";
/// A real OutLayer run and its T1 relay (DCAP on NEAR) take longer than the sandbox's MOCK.
pub(crate) const T1_WAIT: Duration = Duration::from_secs(900);
/// NEAR free of storage stake an account needs for its largest call: 300 Tgas (a round's opening, a pause
/// ticket), set aside up front at ten times the gas price (0.301 NEAR, most of it refunded), with a margin.
/// A run's 200 Tgas (`cmd::RUN_TGAS`) fits within it.
pub(crate) const FREE_FOR_RUNS: u128 = 350 * MILLI;
/// What each vended practice account gets: its contract's storage (0.03), and the gas its commit and its
/// reveal set aside (0.025 and 0.201: the reveal goes out before the commit's unused gas has come back);
/// closing the account after its try returns what is left to the vending contract.
const PRACTICE_GAS_FUNDS: u128 = 280 * MILLI;
/// A vended practice account's key allowance: every call's set-aside gas counts against it (about 0.31).
const PRACTICE_ALLOWANCE: u128 = 400 * MILLI;
/// The practice tries' names (with the program's id, each names a new practice account).
const TRIERS: [&str; 2] = ["judge-1", "judge-2"];
/// The verdict guard's treasury on testnet: enough for one transfer, then a frozen balance.
const GUARD_TREASURY: u128 = 50 * MILLI;
/// The NEAR credit the CLI keeps at near-antibody for each account: one run (0.02 deposit, 0.01 storage
/// allowance) and a commit's storage, within the story accounts' lean funding (scripts/deploy/testnet.sh).
pub(crate) const MIN_NEAR_CREDIT: &str = "0.045";

const STAGES: &[&str] = &[
    "evm", "anchor", "prebuild", "light-client", "program", "guardian", "activate", "agents", "one-moment", "submissions", "settle",
    "project-read", "round-1", "round-2", "release", "practice", "fork", "results",
];

pub struct LiveArgs {
    pub root: PathBuf,
    pub work: PathBuf,
    pub forge_bin: PathBuf,
    pub antibody_bin: PathBuf,
    /// NEAR keys, `<account>.json` (`.secrets/near`).
    pub keys: PathBuf,
    /// The EVM deployer's key file (`{"address", "private_key"}`): it deploys the fixtures and relays the permits.
    pub evm_key: PathBuf,
    /// Sepolia RPC URLs that prove recent state (`eth_getProof`), comma-separated: the witness builder asks
    /// each.
    pub sepolia_rpc: String,
    /// The Sepolia RPC for transactions and every other read (forge, the permit relayer, code at the
    /// anchor, checks): public proof endpoints limit bursts like a forge simulation's.
    pub sepolia_tx_rpc: String,
    pub base_rpc: String,
    pub until: Option<String>,
    /// Run the practice and fork-mode stages again, on new programs at a new anchor (their bases rebuilt
    /// from the RPC): the rest of the story stays as it ran.
    pub redo_practice: bool,
    /// After the story: the verdict guard on testnet (`Live::verdict_consumer`).
    pub verdict_consumer: bool,
    /// After the story: near-t2 on testnet with the four disclosed runs' proofs (`Live::t2`).
    pub t2: bool,
}

/// A contract's wasm to deploy: the size-optimized copy in target/deploy (scripts/deploy/testnet.sh makes
/// it) when it is at least as new as cargo's output, else cargo's output.
fn deployable(root: &Path, name: &str, cargo_out: &str) -> Result<PathBuf> {
    let (opt, built) = (root.join(format!("target/deploy/{name}.wasm")), root.join(cargo_out));
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (modified(&opt), modified(&built)) {
        (Some(o), Some(b)) if o >= b => Ok(opt),
        (Some(_), None) => Ok(opt),
        (_, Some(_)) => Ok(built),
        (None, None) => Err(eyre!("no {name}.wasm: run make contracts")),
    }
}

/// Acme's terms on testnet: the sandbox's payouts, escrow and instant budget in wNEAR (a hundred-thousandth
/// of the tUSD amounts), and windows lived in real time (testnet has no fast-forward).
pub(crate) fn acme_settlement(deadline: u64) -> Settlement {
    let w = |m: u128| (m * MILLI).to_string();
    Settlement {
        payouts: Payouts { critical: w(100), high: w(30), medium: w(10), low: "0".into(), impact_bps: 0, impact_cap: "0".into(), decimals: 24 },
        // A Critical is paid in seconds when T1; the budget holds A and B, so F waits its window.
        instant: InstantRule { cap: w(100), budget: w(150), window_secs: 3_600 },
        fees: Fees { protocol_bps: 0, release: "0".into() },
        bond: w(1),
        challenge_bond: w(5),
        windows: SWindows {
            reveal: 600,
            challenge: 60,
            resolve: 300,
            infra_grace: 600,
            run_timeout: 240,
            round_min: 120,
            rerun_window: 600,
            disclosure_max_delay: 600,
            deadline,
            exit_grace: 60,
        },
        slot_budgets: vec![],
        unevaluable: json!("refund"),
        gap: Some(Gap { threshold_usd_micro: "100000000".into(), claim_amount: w(10) }),
        known: Known { max_count: 4, max_share_bps: 5_000 },
        pause: Pause { slots: vec![0], min_impact_usd_micro: "0".into() },
        terms_sha256: hex::encode(sha256(TERMS.as_bytes())),
    }
}

/// The practice program's terms on testnet: one Low slot paid a thousandth of a wNEAR under the instant
/// cap, a ten-thousandth bond, no pause, no gap lane, no known issues.
fn practice_settlement(deadline: u64) -> Settlement {
    let w = |m: u128| (m * MILLI).to_string();
    Settlement {
        payouts: Payouts { critical: "0".into(), high: "0".into(), medium: "0".into(), low: w(1), impact_bps: 0, impact_cap: "0".into(), decimals: 24 },
        instant: InstantRule { cap: w(1), budget: w(24), window_secs: 3_600 },
        fees: Fees { protocol_bps: 0, release: "0".into() },
        bond: (MILLI / 10).to_string(),
        challenge_bond: w(1),
        windows: SWindows {
            reveal: 600,
            challenge: 60,
            resolve: 300,
            infra_grace: 600,
            run_timeout: 240,
            round_min: 120,
            rerun_window: 600,
            disclosure_max_delay: 600,
            deadline,
            exit_grace: 60,
        },
        slot_budgets: vec![],
        unevaluable: json!("refund"),
        gap: None,
        known: Known { max_count: 0, max_share_bps: 0 },
        pause: Pause { slots: vec![], min_impact_usd_micro: "0".into() },
        terms_sha256: hex::encode(sha256(PRACTICE_TERMS.as_bytes())),
    }
}

/// Each round's code: (implementation the fix overrides, the fix) and (proxy, the fix it is released to).
fn round_code(round: u32) -> (&'static [(&'static str, &'static str)], &'static [(&'static str, &'static str)]) {
    match round {
        1 => (&[("AcmeLendImpl", "AcmeLendFixed")], &[("AcmeLend", "AcmeLendFixed")]),
        _ => (&[("AcmeLendImpl", "AcmeLendFinal"), ("AcmeVaultImpl", "AcmeVaultFixed")], &[("AcmeLend", "AcmeLendFinal"), ("AcmeVault", "AcmeVaultFixed")]),
    }
}

pub(crate) fn unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Polls `f` every `every` until it returns `Some`, for at most `timeout`, saying each minute what it waits for.
pub(crate) fn poll<T>(what: &str, timeout: Duration, every: Duration, mut f: impl FnMut() -> Option<T>) -> Result<T> {
    let t0 = Instant::now();
    let mut said = 0;
    loop {
        if let Some(v) = f() {
            return Ok(v);
        }
        if t0.elapsed() >= timeout {
            return Err(eyre!("timed out after {:.0} s waiting for {what}", t0.elapsed().as_secs_f64()));
        }
        let m = t0.elapsed().as_secs() / 60;
        if m > said {
            said = m;
            println!("  … waiting for {what} ({m} min)");
        }
        std::thread::sleep(every);
    }
}

/// A Borsh view's raw result (the light client answers in Borsh).
fn view_raw(near: &Near, contract: &str, method: &str, args: &[u8]) -> Result<Vec<u8>> {
    let r = near.call("query", json!({"request_type": "call_function", "finality": "final", "account_id": contract, "method_name": method,
                                      "args_base64": base64::engine::general_purpose::STANDARD.encode(args)}))?;
    let bytes = r["result"].as_array().ok_or_else(|| eyre!("{contract}.{method}: no result in {r}"))?;
    Ok(bytes.iter().map(|b| b.as_u64().unwrap_or(0) as u8).collect())
}

/// The light client's newest Sepolia block.
pub(crate) fn lc_last_block(near: &Near, lc: &str) -> Result<u64> {
    let b = view_raw(near, lc, "last_block_number", &[])?;
    Ok(u64::from_le_bytes(b.get(..8).ok_or_else(|| eyre!("last_block_number: {} bytes", b.len()))?.try_into()?))
}

/// The hash the light client holds for Sepolia block `n`, if any.
pub(crate) fn lc_block_hash(near: &Near, lc: &str, n: u64) -> Result<Option<B256>> {
    let b = view_raw(near, lc, "block_hash_safe", &n.to_le_bytes())?;
    Ok(match b.first() {
        Some(1) if b.len() >= 33 => Some(B256::from_slice(&b[1..33])),
        _ => None,
    })
}

/// The EVM address of the MPC network's key for `path`, derived for near-guardian: what near-guardian pins.
pub(crate) fn mpc_address(near: &Near, guardian: &str, path: &str) -> Result<String> {
    let pk = near.view(MPC, "derived_public_key", json!({"path": path, "predecessor": guardian, "domain_id": 0}))?;
    let s = pk.as_str().ok_or_else(|| eyre!("derived_public_key: {pk}"))?;
    let xy = bs58::decode(s.trim_start_matches("secp256k1:")).into_vec()?;
    if xy.len() != 64 {
        return Err(eyre!("derived_public_key: {} bytes, not 64", xy.len()));
    }
    Ok(format!("0x{}", hex::encode(&keccak256(&xy)[12..])))
}

/// Wraps `amount` yoctoNEAR into wNEAR for the signer, registering it with the token first.
pub(crate) fn wrap(ctx: &Ctx, amount: u128) -> Result<Value> {
    let me = ctx.signer()?;
    cmd::ensure_token_registered(ctx, &me, &me.account_id)?;
    let out = ctx.near.function_call(&me, &ctx.dep.token, "near_deposit", json!({}), 30, amount)?;
    cmd::ensure_ok(ctx, &out, "near_deposit")
}

pub(crate) fn wnear_of(ctx: &Ctx, account: &str) -> Result<u128> {
    Ok(cmd::u128_of(&ctx.near.view(&ctx.dep.token, "ft_balance_of", json!({"account_id": account}))?))
}

/// The relayer's permits, by id.
pub(crate) fn permits(file: &Path) -> BTreeMap<String, Value> {
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Runs a forge deploy by `deployer` on `rpc`, again (three tries) when it failed before sending anything:
/// a public RPC's backends sometimes disagree on the latest block during forge's simulation. Once a
/// transaction is out, a failure stops the run (forge's `--resume` finishes it by hand).
pub(crate) fn forge_retry(deployer: alloy_primitives::Address, rpc: &str, what: &str, deploy: impl Fn() -> Result<Value>) -> Result<Value> {
    let evm = Evm::new(rpc);
    let nonce = || {
        (0..5).find_map(|i| {
            std::thread::sleep(Duration::from_secs(3 * i));
            evm.call("eth_getTransactionCount", json!([deployer, "pending"])).ok().and_then(|v| hex_u64(&v).ok())
        })
    };
    let before = nonce();
    for attempt in 1.. {
        match deploy() {
            Ok(v) => return Ok(v),
            Err(e) if attempt < 4 && before.is_some() && nonce() == before => {
                println!("  {what}: try {attempt} failed before sending anything ({e}); again in 30 s");
                std::thread::sleep(Duration::from_secs(30));
            }
            Err(e) => return Err(e.wrap_err(what.to_string())),
        }
    }
    unreachable!()
}

struct Live {
    a: LiveArgs,
    near: Near,
    dep: Deployments,
    s: Value,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Live {
    fn acct(n: &str) -> String {
        format!("{n}.{HQ}")
    }

    fn ctx_at(&self, account: &str, state: &str) -> Ctx {
        demo::ctx_for(&self.near, &self.dep, account, &self.a.keys.join(format!("{account}.json")), &self.a.sepolia_rpc, &self.a.work.join("state").join(state))
    }

    fn ctx_of(&self, n: &str) -> Ctx {
        self.ctx_at(&Self::acct(n), n)
    }

    fn root_ctx(&self) -> Ctx {
        self.ctx_at(HQ, "root")
    }

    /// A judge: no account, no key and no Ethereum RPC.
    fn judge_ctx(&self, dep: &Deployments) -> Ctx {
        Ctx { near: self.near.clone(), dep: dep.clone(), account: None, key_file: None, eth_rpc: None, state_dir: self.a.work.join("state/judges"), via: None }
    }

    /// Agent A's MCP workspace; the server keeps its state (and A's prebuilt witness) in its `.antibody`.
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

    fn evm_key(&self) -> Result<String> {
        let v: Value = serde_json::from_slice(&std::fs::read(&self.a.evm_key)?)?;
        v["private_key"].as_str().map(String::from).ok_or_else(|| eyre!("{}: no private_key", self.a.evm_key.display()))
    }

    fn pid(&self, which: &str) -> u32 {
        self.s["prebuild"]["programs"][which].as_u64().unwrap_or(u64::MAX) as u32
    }

    fn key(&self, which: &str) -> String {
        format!("{}/{}", self.dep.antibody, self.pid(which))
    }

    fn done(&self, stage: &str) -> bool {
        self.s["done"].as_array().is_some_and(|d| d.iter().any(|x| x == stage))
    }

    fn set_done(&mut self, stage: &str, done: bool) {
        let mut d: Vec<Value> = self.s["done"].as_array().cloned().unwrap_or_default().into_iter().filter(|x| x != stage).collect();
        if done {
            d.push(json!(stage));
        }
        self.s["done"] = Value::Array(d);
    }

    fn save(&self) -> Result<()> {
        std::fs::write(self.a.work.join("live.json"), serde_json::to_vec_pretty(&self.s)?)?;
        Ok(())
    }

    /// A NEAR app consuming the run's verdicts (docs/verdict-guard.md): a DAO treasury the Acme project
    /// owns, watching its program. The owner's transfer goes through; D's duplicate does not freeze it; A's
    /// T1-confirmed Critical on the light-client-confirmed anchor does, and the next transfer is refused.
    /// Then its full-access key is deleted. Each step lands under `verdict_consumer`; a rerun resumes after
    /// the last one done. Returns whether every step went as expected.
    fn verdict_consumer(&mut self) -> Result<bool> {
        let guard = Self::acct("guard");
        let owner = Self::acct("acme");
        let pid = self.pid("acme");
        let subs = self.s["submissions"].as_array().cloned().unwrap_or_default();
        let commit_of = |letter: &str| {
            subs.iter().find(|s| s["agent"].as_str().is_some_and(|a| a.starts_with(letter))).and_then(|s| s["commit"].as_u64())
        };
        let (a_commit, d_commit) = (commit_of("A").ok_or_else(|| eyre!("no commit of A in the run"))?, commit_of("D").ok_or_else(|| eyre!("no commit of D"))?);
        let mut vc = self.s["verdict_consumer"].clone();
        if !vc.is_object() || vc.get("skipped").is_some() {
            vc = json!({});
        }
        vc["contract"] = json!(guard);
        vc["owner"] = json!(owner);
        vc["watched_program"] = json!(pid);
        let (root, acme, guard_ctx) = (self.root_ctx(), self.ctx_of("acme"), self.ctx_at(&guard, "guard"));
        let wasm = deployable(&self.a.root, "antibody_verdict_guard", "contracts/verdict-guard/target/wasm32-unknown-unknown/release/antibody_verdict_guard.wasm")?;
        let keep = |s: &mut Value, vc: &Value| -> Result<()> {
            s["verdict_consumer"] = vc.clone();
            std::fs::write(self.a.work.join("live.json"), serde_json::to_vec_pretty(s)?)?;
            Ok(())
        };
        let mut s = self.s.clone();
        // The account: its code stake (the size-optimized wasm), its record and key, the treasury, and the
        // gas its deploy sets aside (10x the gas price for the init's 100 Tgas, refunded after) and its lock
        // burns. A locked contract's stake never comes back.
        if vc.get("create").is_none() {
            let bytes = std::fs::metadata(&wasm)?.len() as u128;
            let amount = bytes * 10u128.pow(19) + 3 * MILLI + GUARD_TREASURY + 120 * MILLI;
            let near = format!("{}.{:024}", amount / ONE_NEAR, amount % ONE_NEAR);
            println!("verdict guard: {guard}, {bytes} B of code, {near} NEAR from {HQ}");
            let r = cmd::admin_create_account(&root, &guard, &near, None, Some(&self.a.keys.join(format!("{guard}.json"))))?;
            vc["create"] = json!({"tx": r["tx"]["tx"], "amount_yocto": amount.to_string(), "code_bytes": bytes});
            keep(&mut s, &vc)?;
        }
        if vc.get("deploy").is_none() {
            let init = json!({"owner": owner, "antibody": self.dep.antibody, "program_id": pid}).to_string();
            let r = cmd::admin_deploy(&guard_ctx, &wasm, Some("new".into()), Some(init))?;
            vc["deploy"] = json!({"tx": r["tx"]["tx"], "bytes": r["bytes"], "sha256": r["sha256"]});
            keep(&mut s, &vc)?;
        }
        let pay_out = |ctx: &Ctx| cmd::admin_call(ctx, &guard, "transfer", &json!({"to": HQ, "amount": (10 * MILLI).to_string()}).to_string(), 1, 30);
        if vc.get("transfer_before").is_none() {
            let r = pay_out(&acme)?;
            vc["transfer_before"] = json!(true);
            vc["transfer_before_tx"] = r["tx"]["tx"].clone();
            keep(&mut s, &vc)?;
        }
        // A refusal: D's commit breaks the same Critical key, but as a duplicate it is not a first finding.
        if vc.get("refusal").is_none() {
            let r = cmd::admin_call(&root, &guard, "report", &json!({"commit_id": d_commit}).to_string(), 0, 150)?;
            let state = self.near.view(&guard, "get_state", json!({}))?;
            let d_state = root.commit(d_commit)?["state"].clone();
            vc["refusal"] = json!({"commit": d_commit, "agent": "D", "commit_state": d_state, "returned": r["result"], "frozen_after": state["frozen"],
                                   "tx": r["tx"]["tx"], "ok": r["result"] == json!(false) && state["frozen"].is_null()});
            keep(&mut s, &vc)?;
        }
        if vc.get("report_tx").is_none() {
            let r = cmd::admin_call(&root, &guard, "report", &json!({"commit_id": a_commit}).to_string(), 0, 150)?;
            vc["report_tx"] = r["tx"]["tx"].clone();
            vc["report_returned"] = r["result"].clone();
            vc["reported_by"] = json!(HQ);
            keep(&mut s, &vc)?;
        }
        vc["frozen"] = self.near.view(&guard, "get_state", json!({}))?["frozen"].clone();
        if vc.get("transfer_after_refused").is_none() {
            let refused = pay_out(&acme).err().map(|e| e.to_string()).unwrap_or_default();
            vc["transfer_after_refused"] = json!(refused.contains("frozen"));
            vc["transfer_after_error"] = json!(refused.chars().take(300).collect::<String>());
            keep(&mut s, &vc)?;
        }
        // Locked: nobody can deploy to it or sign for it with full access again.
        if vc.get("lock").is_none() {
            vc["lock"] = cmd::admin_lock(&guard_ctx, &guard)?;
            keep(&mut s, &vc)?;
        }
        let account = self.near.account(&guard)?.unwrap_or_default();
        let full_access = self.near.access_keys(&guard)?.iter().filter(|k| k["access_key"]["permission"] == json!("FullAccess")).count();
        vc["code_hash"] = account["code_hash"].clone();
        vc["locked"] = json!(full_access == 0);
        vc["treasury_left_yocto"] = account["amount"].clone();
        let ok = vc["transfer_before"] == json!(true)
            && vc["refusal"]["ok"] == json!(true)
            && vc["frozen"]["commit_id"] == json!(a_commit)
            && vc["transfer_after_refused"] == json!(true)
            && vc["locked"] == json!(true);
        vc["ok"] = json!(ok);
        vc["at_unix"] = json!(unix());
        keep(&mut s, &vc)?;
        self.s = s;
        println!("verdict guard {guard}: transfer before {}, D refused {}, frozen by commit {} (slot {}), transfer after refused {}, locked {}",
                 vc["transfer_before"], vc["refusal"]["ok"], vc["frozen"]["commit_id"], vc["frozen"]["slot"], vc["transfer_after_refused"], vc["locked"]);
        Ok(ok)
    }

    /// T2 on testnet (docs/zk.md): near-t2 on its own account (SP1 6.10.0's Groth16 key built in, the
    /// docker build's program vkey, near-antibody, the verifier programs pin), the four disclosed runs'
    /// proofs (fixtures/zk/<case>/proof.bin, from SP1's prover network) checked with the free view and
    /// submitted, one tampered proof refused, then the account locked. Each step lands under `t2`; a
    /// rerun resumes after the last one done.
    fn t2(&mut self) -> Result<bool> {
        use base64::Engine as _;
        let zk = Self::acct("zk");
        let cases = [("a", 33u64), ("d", 36), ("c", 35), ("practice", 53)];
        let fixtures = self.a.root.join("fixtures/zk");
        let read = |case: &str, f: &str| std::fs::read(fixtures.join(case).join(f)).map_err(|e| eyre!("fixtures/zk/{case}/{f}: {e} (prove it first: docs/zk.md)"));
        let mut t2 = self.s["t2"].clone();
        if !t2.is_object() {
            t2 = json!({});
        }
        t2["contract"] = json!(zk);
        let (root, zk_ctx) = (self.root_ctx(), self.ctx_at(&zk, "zk"));
        let wasm = deployable(&self.a.root, "near_t2", "contracts/near-t2/target/wasm32-unknown-unknown/release/near_t2.wasm")?;
        let keep = |s: &mut Value, t2: &Value| -> Result<()> {
            s["t2"] = t2.clone();
            std::fs::write(self.a.work.join("live.json"), serde_json::to_vec_pretty(s)?)?;
            Ok(())
        };
        let mut s = self.s.clone();
        // The verifier the guest re-runs: the version string its receipts name, and the wasm programs pin.
        let pv_a = read("a", "public_values.bin")?;
        let verifier = antibody_zk_core::decode_public_values(&pv_a).map_err(|e| eyre!("{e}"))?.verifier;
        let verifier_wasm = root.view("get_pins", json!({"program_id": self.pid("acme")}))?["verifier"]["wasm_hash"].as_str().unwrap_or_default().to_string();
        // The account: its code stake, its state and the four records, and the gas its deploy sets aside
        // (10x the gas price for the init's 100 Tgas, refunded after) and its lock burns.
        if t2.get("create").is_none() {
            let bytes = std::fs::metadata(&wasm)?.len() as u128;
            let amount = bytes * 10u128.pow(19) + 30 * MILLI + 120 * MILLI;
            let near = format!("{}.{:024}", amount / ONE_NEAR, amount % ONE_NEAR);
            println!("near-t2: {zk}, {bytes} B of code, {near} NEAR from {HQ}");
            let r = cmd::admin_create_account(&root, &zk, &near, None, Some(&self.a.keys.join(format!("{zk}.json"))))?;
            t2["create"] = json!({"tx": r["tx"]["tx"], "amount_yocto": amount.to_string(), "code_bytes": bytes});
            keep(&mut s, &t2)?;
        }
        if t2.get("deploy").is_none() {
            let init = json!({"antibody": self.dep.antibody, "program_vkey": crate::zk::PROGRAM_VKEY, "verifier": verifier, "verifier_wasm": verifier_wasm}).to_string();
            let r = cmd::admin_deploy(&zk_ctx, &wasm, Some("new".into()), Some(init))?;
            t2["deploy"] = json!({"tx": r["tx"]["tx"], "bytes": r["bytes"], "sha256": r["sha256"]});
            keep(&mut s, &t2)?;
        }
        t2["pins"] = self.near.view(&zk, "get_pins", json!({}))?;
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let submit = |run: u64, pv: &[u8], proof: &[u8]| -> Result<Value> {
            let args = json!({"run_id": run.to_string(), "public_values": b64(pv), "proof": b64(proof)});
            let out = self.near.function_call(&root.signer()?, &zk, "submit", args, antibody_cli::zk::SUBMIT_TGAS, antibody_cli::zk::RECORD_DEPOSIT)?;
            cmd::ensure_ok(&root, &out, "submit")
        };
        // One refusal first: A's proof with one byte of its A point changed.
        if t2.get("refusal").is_none() {
            let (pv, mut proof) = (read("a", "public_values.bin")?, read("a", "proof.bin")?);
            proof[100 + 63] ^= 1;
            let refused = submit(33, &pv, &proof).err().map(|e| e.to_string()).unwrap_or_default();
            t2["refusal"] = json!({"what": "run 33's proof with one byte of its A point changed", "refused": !refused.is_empty(),
                                   "error": refused.chars().take(300).collect::<String>(), "record_after": self.near.view(&zk, "get_record", json!({"run_id": "33"}))?});
            keep(&mut s, &t2)?;
        }
        let mut subs = t2["submissions"].as_array().cloned().unwrap_or_default();
        for (case, run) in cases {
            if subs.iter().any(|x| x["run_id"] == json!(run)) {
                continue;
            }
            let (pv, proof) = (read(case, "public_values.bin")?, read(case, "proof.bin")?);
            let verifies = self.near.view(&zk, "verify_proof", json!({"public_values": b64(&pv), "proof": b64(&proof)}))?;
            let r = submit(run, &pv, &proof)?;
            // Views read a final block, a moment behind the transaction just executed.
            let t0 = Instant::now();
            let record = loop {
                let rec = self.near.view(&zk, "get_record", json!({"run_id": run.to_string()}))?;
                if !rec.is_null() || t0.elapsed().as_secs() > 60 {
                    break rec;
                }
                std::thread::sleep(Duration::from_secs(2));
            };
            let proof_json: Value = std::fs::read(fixtures.join(case).join("proof.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            subs.push(json!({"case": case, "run_id": run, "verify_proof_view": verifies, "tx": r["tx"], "explorer": r["explorer"],
                             "events": r["events"], "record": record, "network_request": proof_json["request_id"]}));
            t2["submissions"] = Value::Array(subs.clone());
            keep(&mut s, &t2)?;
        }
        if t2.get("lock").is_none() {
            t2["lock"] = cmd::admin_lock(&zk_ctx, &zk)?;
            keep(&mut s, &t2)?;
        }
        let account = self.near.account(&zk)?.unwrap_or_default();
        let full_access = self.near.access_keys(&zk)?.iter().filter(|k| k["access_key"]["permission"] == json!("FullAccess")).count();
        t2["code_hash"] = account["code_hash"].clone();
        t2["locked"] = json!(full_access == 0);
        t2["stats"] = self.near.view(&zk, "get_stats", json!({}))?;
        let confirmed = subs.iter().filter(|x| x["record"]["outcome"] == json!("confirmed")).count();
        let ok = confirmed == cases.len() && t2["refusal"]["refused"] == json!(true) && t2["locked"] == json!(true)
            && t2["pins"]["groth16"]["is_sp1_v6_10_0"] == json!(true) && t2["pins"]["program_vkey"] == json!(crate::zk::PROGRAM_VKEY);
        t2["ok"] = json!(ok);
        t2["at_unix"] = json!(unix());
        keep(&mut s, &t2)?;
        self.s = s;
        println!("near-t2 {zk}: {confirmed} of {} runs T2-confirmed, tampered proof refused {}, locked {}", cases.len(), t2["refusal"]["refused"], t2["locked"]);
        Ok(ok)
    }

    /// The indexer (FastNear's tx API: every transaction touching near-antibody or near-guardian), the T1
    /// relayer (OutLayer's attestations, through the indexed requests) and the permit relayer (Sepolia and
    /// Base Sepolia), from the program's creation on.
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
            signer: Some(EvmSigner::from_hex(&self.evm_key()?)?),
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

    /// Tops `account` up from the root account to `min` yoctoNEAR free of its storage stake. NEAR sets
    /// aside a call's whole prepaid gas at ten times the gas price before running it (a 300 Tgas call
    /// needs 0.301 NEAR free and burns a tenth of that), so a lean account cannot send one without this.
    fn ensure_liquid(&self, account: &str, min: u128) -> Result<u128> {
        let a = self.near.account(account)?.ok_or_else(|| eyre!("no account {account}"))?;
        let free = cmd::u128_of(&a["amount"]).saturating_sub(a["storage_usage"].as_u64().unwrap_or(0) as u128 * 10u128.pow(19));
        if free >= min {
            return Ok(0);
        }
        if account == HQ {
            return Err(eyre!("{HQ} has {} NEAR free and this step needs {}: {} NEAR short", cmd::fmt_tokens(free, 24), cmd::fmt_tokens(min, 24), cmd::fmt_tokens(min - free, 24)));
        }
        let root = self.root_ctx();
        let out = root.near.send(&root.signer()?, account, vec![Action::Transfer(min - free)], "EXECUTED_OPTIMISTIC")?;
        cmd::ensure_ok(&root, &out, "transfer")?;
        println!("  {account}: +{} NEAR from {HQ} (to {} free)", cmd::fmt_tokens(min - free, 24), cmd::fmt_tokens(min, 24));
        // The next transaction may be checked against a node a block or two behind, which still sees the old
        // balance (NotEnoughBalance): the top-up is waited for in a final block.
        poll(&format!("{account}'s top-up in a final block"), Duration::from_secs(60), Duration::from_secs(2), || {
            let a = self.near.call("query", json!({"request_type": "view_account", "finality": "final", "account_id": account})).ok()?;
            let seen = cmd::u128_of(&a["amount"]).saturating_sub(a["storage_usage"].as_u64().unwrap_or(0) as u128 * 10u128.pow(19));
            (seen >= min).then_some(())
        })?;
        Ok(min - free)
    }

    /// Returns what story account `n` holds beyond `keep` (free of its storage stake) to the root account.
    fn sweep(&self, n: &str, keep: u128) -> Result<u128> {
        let me = Self::acct(n);
        let a = self.near.account(&me)?.ok_or_else(|| eyre!("no account {me}"))?;
        let free = cmd::u128_of(&a["amount"]).saturating_sub(a["storage_usage"].as_u64().unwrap_or(0) as u128 * 10u128.pow(19));
        // Not worth a transaction below a hundredth of a NEAR.
        if free <= keep + 10 * MILLI {
            return Ok(0);
        }
        let ctx = self.ctx_of(n);
        let out = ctx.near.send(&ctx.signer()?, HQ, vec![Action::Transfer(free - keep)], "EXECUTED_OPTIMISTIC")?;
        cmd::ensure_ok(&ctx, &out, "transfer")?;
        Ok(free - keep)
    }

    fn stop_services(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for h in std::mem::take(&mut self.threads) {
            let _ = h.join();
        }
    }

    fn forge_deploy(&self, rpc: &str, what: &str, deploy: impl Fn() -> Result<Value>) -> Result<Value> {
        forge_retry(EvmSigner::from_hex(&self.evm_key()?)?.address, rpc, what, deploy)
    }

    // ------------------------------------------------------------------------------------------ stages

    /// Acme Lend on Sepolia and Base Sepolia (CREATE2: the same addresses on both) with no pauser yet, and
    /// the practice program's 24 vaults on Sepolia; the deployer's key comes from its file.
    fn evm(&mut self) -> Result<()> {
        let key = self.evm_key()?;
        let by = Broadcaster::Key(&key);
        let (root, forge, w) = (&self.a.root, &self.a.forge_bin, &self.a.work);
        let zero = "0x0000000000000000000000000000000000000000";
        let (sepolia, base) = (self.sepolia(), self.a.base_rpc.clone());
        let dep_a = self.forge_deploy(&sepolia, "Acme on Sepolia", || demo::deploy_acme(root, forge, &sepolia, &by, zero, &w.join("acme-sepolia.json")))?;
        let dep_b = self.forge_deploy(&base, "Acme on Base Sepolia", || demo::deploy_acme(root, forge, &base, &by, zero, &w.join("acme-base-sepolia.json")))?;
        let same = ["AcmeGuard", "AcmeLend", "AcmeLendImpl", "AcmeVault", "AcmeVaultImpl", "AcmeLendFixed", "AcmeLendFinal", "AcmeVaultFixed"].iter().all(|k| dep_a[k] == dep_b[k]);
        if !same {
            return Err(eyre!("the Acme addresses differ between Sepolia and Base Sepolia"));
        }
        let dep_p = self.forge_deploy(&sepolia, "the practice vaults on Sepolia", || demo::deploy_practice(root, forge, &sepolia, &by, &w.join("practice-sepolia.json")))?;
        let deployer = EvmSigner::from_hex(&key)?.address;
        println!("Acme Lend on {SEPOLIA} and {BASE_SEPOLIA} (same addresses: {same}); practice factory {} with {} vaults; deployer {deployer}",
                 dep_p["factory"], dep_p["vaults"].as_array().map(|v| v.len()).unwrap_or(0));
        self.s["evm"] = json!({"acme": dep_a, "acme_base_sepolia": dep_b, "practice": dep_p, "deployer": deployer, "same_addresses": same,
                               "chains": {"verifies_and_pauses": SEPOLIA, "pauses": BASE_SEPOLIA}});
        Ok(())
    }

    /// A recent Sepolia block whose state has both fixtures (their last contracts have code there).
    fn anchor(&mut self) -> Result<()> {
        let evm = Evm::new(&self.sepolia());
        let last = [self.s["evm"]["acme"]["AcmeVaultFixed"].clone(), self.s["evm"]["practice"]["vaults"].as_array().and_then(|v| v.last().cloned()).unwrap_or(Value::Null)];
        let n = poll("a Sepolia block with both fixtures", Duration::from_secs(600), Duration::from_secs(12), || {
            let head = evm.call("eth_blockNumber", json!([])).ok().and_then(|v| hex_u64(&v).ok())?;
            // Eight blocks back: no anchor a reorganisation could still replace, and one every backend of a
            // load-balanced RPC already holds (a backend behind the anchor answers its proofs wrongly).
            let n = head.checked_sub(8)?;
            let has = |a: &Value| evm.call("eth_getCode", json!([a, format!("{n:#x}")])).ok().and_then(|c| c.as_str().map(|c| c.len() > 2)).unwrap_or(false);
            last.iter().all(has).then_some(n)
        })?;
        let b = evm.read("eth_getBlockByNumber", json!([format!("{n:#x}"), false]))?;
        println!("anchor: Sepolia block {n} ({})", b["hash"]);
        self.s["anchor"] = json!({"block": n, "hash": b["hash"], "timestamp": hex_u64(&b["timestamp"])?, "chosen_unix": unix()});
        Ok(())
    }

    /// Every witness the story builds from the RPC, while the anchor is recent: the three programs' base
    /// witnesses (Acme, practice, practice in fork mode), the known issue and A–G, and both rounds' bases. The
    /// programs' ids (and so their keys) are the contract's next three.
    /// The practice program and its fork-mode twin (created by the root account) at anchor `n`: their
    /// manifests and base witnesses. Their bases also cover the tutorial proof, so a try seals against the
    /// pinned base alone: no Ethereum RPC for whoever tries it. The vaults differ only in their address:
    /// each one is checked.
    fn prebuild_practice(&mut self, evm: &Evm, n: u64, h: B256, deadline: u64, built: &mut serde_json::Map<String, Value>) -> Result<()> {
        let w = self.a.work.clone();
        let root_ctx = self.root_ctx();
        let pm = demo::practice_manifest(&self.s["evm"]["practice"], evm, n, h, HQ, &TESTNET, practice_settlement(deadline))?;
        std::fs::write(w.join("practice-manifest.json"), pm.canonical())?;
        std::fs::write(w.join("practice-terms.txt"), PRACTICE_TERMS)?;
        let pcover = demo::tutorial_cover(&w, "practice", &pm)?;
        let tb = Instant::now();
        let (pb, _) = cmd::build_base_covering(&root_ctx, &pm.canonical(), &self.key("practice"), &[pcover])?;
        built.insert("practice_base".into(), json!({"bytes": pb.len(), "seconds": tb.elapsed().as_secs_f64(), "covers": "the tutorial proof, every vault"}));
        let fm = demo::fork_practice_manifest(&self.a.root, &root_ctx, n, h, HQ, &TESTNET, practice_settlement(deadline))?;
        std::fs::write(w.join("practice-fork-manifest.json"), fm.canonical())?;
        let fcover = demo::tutorial_cover(&w, "practice-fork", &fm)?;
        let tb = Instant::now();
        let (fb, _) = cmd::build_base_covering(&root_ctx, &fm.canonical(), &self.key("fork"), &[fcover])?;
        built.insert("fork_base".into(), json!({"bytes": fb.len(), "seconds": tb.elapsed().as_secs_f64(), "covers": "the tutorial proof, every vault"}));
        for (name, m, base, key) in [("practice", &pm, &pb, self.key("practice")), ("fork mode", &fm, &fb, self.key("fork"))] {
            for t in m.targets.iter().filter(|t| t.name.starts_with("vault-")) {
                let bf: antibody_witness::build::BundleFile = serde_json::from_value(practice::tutorial_bundle(&format!("{:#x}", t.address)))?;
                let bundle = antibody_witness::build::parse_bundle(&bf, B256::ZERO)?;
                antibody_witness::program::build_submission_from_base(base, &m.canonical(), bundle, &key)
                    .map_err(|e| eyre!("{name}: the tutorial proof against {} does not seal against the base alone: {e}", t.name))?;
            }
        }
        Ok(())
    }

    /// `--redo-practice`: new practice and fork-mode programs (the contract's next two ids) at a new
    /// anchor, their bases rebuilt; the practice, fork and results stages run again.
    fn redo_practice(&mut self) -> Result<()> {
        step("live: practice again, at a new anchor");
        let t = Instant::now();
        let evm = Evm::new(&self.sepolia());
        let last = self.s["evm"]["practice"]["vaults"].as_array().and_then(|v| v.last().cloned()).ok_or_else(|| eyre!("no practice vaults: run the evm stage"))?;
        let head = hex_u64(&evm.read("eth_blockNumber", json!([]))?)?;
        // As the anchor stage chooses: eight blocks back, with the vaults' code there.
        let n = head - 8;
        if evm.read("eth_getCode", json!([last, format!("{n:#x}")]))?.as_str().is_none_or(|c| c.len() <= 2) {
            return Err(eyre!("the practice vaults have no code at block {n}"));
        }
        let b = evm.read("eth_getBlockByNumber", json!([format!("{n:#x}"), false]))?;
        let h: B256 = b["hash"].as_str().unwrap_or_default().parse()?;
        let next = self.near.view(&self.dep.antibody, "get_stats", json!({}))?["next_program"].as_u64().ok_or_else(|| eyre!("get_stats: no next_program"))? as u32;
        let earlier = json!({"practice": self.s["practice"].clone(), "fork_mode": self.s["fork_mode"].clone(), "anchor": self.s["practice_anchor"].clone(),
                             "programs": {"practice": self.pid("practice"), "fork": self.pid("fork")}});
        self.s["prebuild"]["programs"]["practice"] = json!(next);
        self.s["prebuild"]["programs"]["fork"] = json!(next + 1);
        let mut built = serde_json::Map::new();
        self.prebuild_practice(&evm, n, h, unix() + 30 * 86_400, &mut built)?;
        println!("practice and fork-mode programs {next} and {} at Sepolia block {n}: bases built in {:.0} s", next + 1, t.elapsed().as_secs_f64());
        let mut redone = self.s["practice_redone"].as_array().cloned().unwrap_or_default();
        redone.push(earlier);
        self.s["practice_redone"] = Value::Array(redone);
        self.s["practice_anchor"] = json!({"block": n, "hash": h, "timestamp": hex_u64(&b["timestamp"])?, "chosen_unix": unix(), "witnesses": built});
        self.save()?;
        // The programs' anchor checks ask the light client, which runs some 80 blocks behind.
        self.s["practice_anchor"]["light_client"] = self.wait_light_client(n, h)?;
        self.s["practice"] = Value::Null;
        self.s["fork_mode"] = Value::Null;
        for stage in ["practice", "fork", "results"] {
            self.set_done(stage, false);
        }
        self.save()
    }

    fn prebuild(&mut self) -> Result<()> {
        let t = Instant::now();
        let n = self.s["anchor"]["block"].as_u64().ok_or_else(|| eyre!("no anchor"))?;
        let h: B256 = self.s["anchor"]["hash"].as_str().unwrap_or_default().parse()?;
        // Code at the anchor (the manifests' code hashes, the fixes' code) from a node with state history;
        // the witnesses' proofs from the proof endpoints.
        let evm = Evm::new(&self.sepolia());
        let head = hex_u64(&evm.read("eth_blockNumber", json!([]))?)?;
        if head > n + 90 {
            return Err(eyre!("the anchor {n} is {} blocks old, and public RPCs prove about 128 back: rerun for a new anchor", head - n));
        }
        let next = self.near.view(&self.dep.antibody, "get_stats", json!({}))?["next_program"].as_u64().ok_or_else(|| eyre!("get_stats: no next_program"))? as u32;
        self.s["prebuild"] = json!({"programs": {"acme": next, "practice": next + 1, "fork": next + 2}});
        let deadline = unix() + 30 * 86_400;
        let w = self.a.work.clone();
        let dep_a = self.s["evm"]["acme"].clone();
        let mut built = serde_json::Map::new();

        let m = demo::acme_manifest(&dep_a, &evm, n, h, &Self::acct("acme"), &TESTNET, acme_settlement(deadline))?;
        let mj = m.canonical();
        std::fs::write(w.join("manifest.json"), &mj)?;
        std::fs::write(w.join("terms.txt"), TERMS)?;
        let acme = self.ctx_of("acme");
        let key = self.key("acme");
        let tb = Instant::now();
        let (base, meta) = cmd::build_base(&acme, &mj, &key)?;
        built.insert("acme_base".into(), json!({"bytes": base.len(), "seconds": tb.elapsed().as_secs_f64(), "cached": meta["cached"]}));

        // The submissions at this deployment's addresses (scripts/demo/acme_bundles.py), and G's.
        let fx = w.join("submissions");
        let st = Command::new("python3").arg(self.a.root.join("scripts/demo/acme_bundles.py")).arg(w.join("acme-sepolia.json")).arg(&fx).status()?;
        if !st.success() {
            return Err(eyre!("acme_bundles.py failed"));
        }
        demo::gap_bundle(&self.a.root, &fx, &dep_a)?;
        let mcp = Ctx { state_dir: self.mcp_dir().join(".antibody"), ..self.ctx_of("agent-a") };
        let mut jobs: Vec<(String, Ctx)> = vec![("KNOWN".into(), self.ctx_of("acme")), ("A".into(), mcp)];
        for x in ["b", "c", "d", "e", "f", "g"] {
            jobs.push((x.to_uppercase(), self.ctx_of(&format!("agent-{x}"))));
        }
        for (name, ctx) in &jobs {
            let tb = Instant::now();
            let (bytes, meta, receipt, _) = cmd::build_witness(ctx, None, &mj, &fx.join(format!("{name}.bundle.json")), None, &key)?;
            let r = serde_json::to_value(&receipt)?;
            println!("  {name}: {} bytes, local verdict {} ({:.1} s)", bytes.len(), r["kind"], tb.elapsed().as_secs_f64());
            built.insert(name.clone(), json!({"bytes": bytes.len(), "local_verdict": r["kind"], "broken_mask": r["broken_mask"], "seconds": tb.elapsed().as_secs_f64(), "cached": meta["cached"]}));
        }

        // Both rounds: the manifests (the fixes' code as overrides, the new invariant; round 2 carries the
        // rule strength measured on the same fixture), and their base witnesses.
        for round in [1u32, 2] {
            let dir = w.join(format!("round{round}"));
            std::fs::create_dir_all(&dir)?;
            let mut flags = Vec::new();
            for (target, fix) in round_code(round).0 {
                let file = dir.join(format!("{fix}.runtime.hex"));
                std::fs::write(&file, evm.read("eth_getCode", json!([dep_a[*fix], format!("{n:#x}")]))?.as_str().unwrap_or_default())?;
                flags.push(format!("{}={}", dep_a[*target].as_str().unwrap_or_default(), file.display()));
            }
            let inv = dir.join("invariants.hex");
            std::fs::write(&inv, dep_a["invariantsV2RuntimeCode"].as_str().unwrap_or_default())?;
            let added = vec!["3:protected_balances:medium".to_string()];
            let strength = self.a.root.join("docs/results/rule-strength-acme.json");
            let strength = (round == 2 && strength.exists()).then_some(strength);
            let manifest = dir.join("manifest.json");
            cmd::manifest_round(&w.join("manifest.json"), round, &flags, Some((inv.as_path(), added.as_slice())), strength.as_deref(), &manifest)?;
            let tb = Instant::now();
            let (rb, _) = cmd::build_round_base(&acme, &mj, &std::fs::read(&manifest)?, &key)?;
            built.insert(format!("round{round}_base"), json!({"bytes": rb.len(), "seconds": tb.elapsed().as_secs_f64()}));
        }

        self.prebuild_practice(&evm, n, h, deadline, &mut built)?;

        let head_after = hex_u64(&evm.read("eth_blockNumber", json!([]))?)?;
        println!("prebuilt {} witnesses in {:.0} s; the anchor is now {} blocks back", built.len(), t.elapsed().as_secs_f64(), head_after - n);
        self.s["prebuild"]["witnesses"] = Value::Object(built);
        self.s["prebuild"]["seconds"] = json!(t.elapsed().as_secs_f64());
        self.s["prebuild"]["blocks_behind_head"] = json!(head_after - n);
        self.s["prebuild"]["deadline"] = json!(deadline);
        Ok(())
    }

    /// Waits until the light client holds the anchor, and checks the hash it holds.
    fn light_client(&mut self) -> Result<()> {
        let n = self.s["anchor"]["block"].as_u64().unwrap_or(0);
        let want: B256 = self.s["anchor"]["hash"].as_str().unwrap_or_default().parse()?;
        self.s["light_client"] = self.wait_light_client(n, want)?;
        Ok(())
    }

    /// Waits until the light client holds block `n`, and checks it holds the anchor's hash there.
    fn wait_light_client(&self, n: u64, want: B256) -> Result<Value> {
        let t = Instant::now();
        let lc = self.dep.light_client.clone().ok_or_else(|| eyre!("no light client in the deployments"))?;
        poll(&format!("{lc} to reach block {n}"), Duration::from_secs(5_400), Duration::from_secs(30), || {
            lc_last_block(&self.near, &lc).ok().filter(|b| *b >= n)
        })?;
        let got = lc_block_hash(&self.near, &lc, n)?;
        if got != Some(want) {
            return Err(eyre!("{lc} holds {got:?} for block {n}, not the anchor's {want:#x}"));
        }
        println!("{lc} holds the anchor (block {n}) after {:.0} s", t.elapsed().as_secs_f64());
        Ok(json!({"contract": lc, "anchor_confirmed_unix": unix(), "waited_seconds": t.elapsed().as_secs_f64()}))
    }

    /// Acme's program: the escrow wrapped, the NEP-413 project key, the manifest committed.
    fn program(&mut self) -> Result<()> {
        let acme = self.ctx_of("acme");
        let want = self.pid("acme");
        let escrow = cmd::parse_tokens(ESCROW, 24)?;
        if wnear_of(&acme, &Self::acct("acme"))? < escrow {
            // The escrow is acme's NEAR, wrapped: topped up from the root first (an earlier run's sweep may
            // have left acme with its storage alone).
            self.ensure_liquid(&Self::acct("acme"), escrow + FREE_FOR_RUNS)?;
            wrap(&acme, escrow)?;
        }
        // The creation: acme's NEAR credit at near-antibody (storage) and the call's 300 Tgas set aside.
        self.ensure_liquid(&Self::acct("acme"), FREE_FOR_RUNS + 100 * MILLI)?;
        let project_key = self.a.work.join("keys/acme.x25519.json");
        let derived = cmd::keygen_x25519_nep413(&acme, &project_key)?;
        let manifest = self.a.work.join("manifest.json");
        let created = cmd::program_create(&acme, &manifest, ESCROW, &project_key, None)?;
        let pid = created["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no program id: {created}"))? as u32;
        if pid != want {
            return Err(eyre!("program {pid} was created, but the witnesses were built for program {want}"));
        }
        let anchor = acme.view("program_record", json!({"program_id": pid})).map(|r| r["anchor"].clone()).unwrap_or(Value::Null);
        println!("program {pid} created: {} (anchor {}, {anchor})", created["tx"]["tx"], created["anchor"]["anchor"]);
        self.s["program"] = json!({"program_id": pid, "manifest_sha256": hex::encode(sha256(&std::fs::read(&manifest)?)), "anchor_block": self.s["anchor"]["block"],
                                   "anchor_hash": self.s["anchor"]["hash"], "create_tx": created["tx"]["tx"], "anchor": created["anchor"], "anchor_status": anchor,
                                   "project_key": {"public_key": derived["public_key"], "nep413": derived["nep413"]}, "terms": TERMS});
        Ok(())
    }

    /// The program's adapter on both chains, made Acme's pauser and the proxies' admin, for the MPC key
    /// near-guardian will pin; then the registration (with 0.1 NEAR of MPC credit).
    fn guardian(&mut self) -> Result<()> {
        let guardian = self.dep.guardian.clone().ok_or_else(|| eyre!("no guardian"))?;
        let key = self.key("acme");
        let path = format!("antibody/{}/{key}", Self::acct("acme"));
        let address = mpc_address(&self.near, &guardian, &path)?;
        let evm_key = self.evm_key()?;
        let by = Broadcaster::Key(&evm_key);
        let (w, root, forge) = (&self.a.work, &self.a.root, &self.a.forge_bin);
        let (sepolia, base) = (self.sepolia(), self.a.base_rpc.clone());
        let ad_a = self.forge_deploy(&sepolia, "the adapter on Sepolia", || {
            demo::deploy_adapter(root, forge, &sepolia, &by, &address, &key, &w.join("acme-sepolia.json"), &w.join("adapter-sepolia.json"))
        })?;
        let ad_b = self.forge_deploy(&base, "the adapter on Base Sepolia", || {
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
        // The MPC credit (0.1) and the call's 100 Tgas set aside.
        self.ensure_liquid(&Self::acct("acme"), 210 * MILLI)?;
        let reg = acme.near.function_call(&acme.signer()?, &guardian, "register", json!({"program_key": key, "chains": chains}), 100, ONE_NEAR / 10)?;
        cmd::ensure_ok(&acme, &reg, "register")?;
        let pinned = poll("near-guardian to pin the MPC key", Duration::from_secs(180), Duration::from_secs(5), || {
            self.near.view(&guardian, "get_program", json!({"program_key": key, "project": Self::acct("acme")})).ok().filter(|p| p["address"].is_string())
        })?;
        println!("adapter {adapter} on {SEPOLIA} and {BASE_SEPOLIA}; near-guardian pinned {} (derived {address})", pinned["address"]);
        if pinned["address"].as_str().map(|x| x.eq_ignore_ascii_case(&address)) != Some(true) {
            return Err(eyre!("near-guardian pinned {} for the program, not the adapter's {address}", pinned["address"]));
        }
        self.s["guardian"] = json!({"adapter": adapter, "proxies": proxies, "guardian_address": pinned["address"], "mpc": MPC, "path": path,
                                    "register_tx": antibody_cli::near::tx_hash(&reg), "adapters": {"sepolia": ad_a, "base_sepolia": ad_b}});
        Ok(())
    }

    /// Activation (the first real OutLayer run: the program's sealing key, T1 once relayed), the known
    /// issue, and the opening.
    fn activate(&mut self) -> Result<()> {
        let acme = self.ctx_of("acme");
        let pid = self.pid("acme");
        let t = Instant::now();
        self.ensure_liquid(&Self::acct("acme"), FREE_FOR_RUNS)?;
        let act = cmd::program_activate(&acme, pid, &[], true)?;
        let act_run = act["run_id"].as_u64().unwrap_or_default();
        demo::wait_t1_for(&acme, act_run, T1_WAIT)?;
        let p = acme.program(pid)?;
        if p["sealing_pk"].is_null() {
            return Err(eyre!("activation run {act_run} is T1 but the program has no sealing key: {}", act["result"]));
        }
        println!("activated in {:.1} s: sealing key {}, base witness {} bytes", t.elapsed().as_secs_f64(), p["sealing_pk"], act["base"]["bytes"]);
        self.s["activation"] = json!({"run_id": act_run, "sealing_pk": p["sealing_pk"], "key_epoch": p["key_epoch"], "base": act["base"], "seconds": t.elapsed().as_secs_f64(),
                                      "tx": act["tx"]["tx"]});
        let fx = self.a.work.join("submissions");
        self.ensure_liquid(&Self::acct("acme"), FREE_FOR_RUNS)?;
        let known = cmd::submit_known(&acme, pid, &fx.join("KNOWN.bundle.json"), None, None, true)?;
        let known_run = known["run_id"].as_u64().unwrap_or_default();
        demo::wait_t1_for(&acme, known_run, T1_WAIT)?;
        let known_n = acme.program(pid)?["known"].as_array().map(|k| k.len()).unwrap_or(0);
        let opened = cmd::program_open(&acme, pid)?;
        println!("known issues: {known_n}; program {pid} open");
        self.s["known"] = json!({"run_id": known_run, "count": known_n, "tx": known["tx"]["tx"], "open_tx": opened["tx"]});
        Ok(())
    }

    /// Each agent: wNEAR for its bonds and NEAR credit for its run; agent A also a function-call key for MCP.
    fn agents(&mut self) -> Result<()> {
        let mut out = serde_json::Map::new();
        for x in ["a", "b", "c", "d", "e", "f", "g"] {
            let c = self.ctx_of(&format!("agent-{x}"));
            let me = Self::acct(&format!("agent-{x}"));
            self.ensure_liquid(&me, FREE_FOR_RUNS)?;
            // Each step only when missing: a stage that stopped part-way runs again.
            let have = cmd::account(&c, &me)?;
            if wnear_of(&c, &me)? + cmd::u128_of(&have["tokens"]) < 2 * MILLI {
                wrap(&c, 3 * MILLI)?;
            }
            // A's MCP server signs with a function-call key, which attaches no deposit: A's credit covers
            // its commit and run up front.
            let credit = cmd::parse_near(if x == "a" { "0.07" } else { MIN_NEAR_CREDIT })?;
            let near_credit = cmd::u128_of(&have["near"]);
            if near_credit < credit {
                cmd::storage_deposit(&c, &cmd::fmt_tokens(credit - near_credit, 24))?;
            }
            let tokens = cmd::u128_of(&have["tokens"]);
            if tokens < 2 * MILLI {
                // An ft_transfer_call with 300 Tgas: 0.301 NEAR free again.
                self.ensure_liquid(&me, FREE_FOR_RUNS)?;
                cmd::deposit(&c, &cmd::fmt_tokens(2 * MILLI - tokens, 24))?;
            }
            out.insert(me.clone(), cmd::account(&c, &me)?);
        }
        let agent_key = self.a.work.join("keys/agent-a.fc-key.json");
        if !agent_key.exists() {
            // An allowance pays each call's set-aside gas (a 300 Tgas reveal: 0.301 NEAR), not its burn.
            cmd::agent_key_add(&self.ctx_of("agent-a"), 1.0, &agent_key)?;
        }
        self.s["agents"] = Value::Object(out);
        Ok(())
    }

    /// Agent A over MCP; its verdict T1, its payment, and the pause on both chains, each on its own clock.
    fn one_moment(&mut self) -> Result<()> {
        let pid = self.pid("acme");
        let root_ctx = self.root_ctx();
        let fx = self.a.work.join("submissions");
        let state_a = self.mcp_dir();
        std::fs::create_dir_all(&state_a)?;
        let rpc = self.dep.rpc.clone().unwrap_or_default();
        let net = ["--network", "testnet", "--rpc", rpc.as_str()];
        let dep_file = self.a.root.join("deployments/near-testnet.json");
        let events_file = self.services_dir().join("events.jsonl");
        // A event's transaction, as the indexer recorded it.
        let tx_of = |event: &str, commit: u64| {
            crate::indexer::read_events(&events_file, 0, usize::MAX)
                .into_iter()
                .find(|e| e["event"] == event && e["data"]["commit_id"] == json!(commit))
                .and_then(|e| e["tx"].as_str().map(String::from))
                .unwrap_or_default()
        };
        // A submits once: a stage stopped after A's commit goes on from it.
        let a_commit = match self.s["mcp"]["commit_id"].as_u64() {
            Some(c) => c,
            None => {
                self.ensure_liquid(&Self::acct("agent-a"), FREE_FOR_RUNS)?;
                let mcp = demo::mcp_submit(&self.a.antibody_bin, &net, &dep_file, &Self::acct("agent-a"), &self.a.work.join("keys/agent-a.fc-key.json"),
                                           Some(self.a.sepolia_rpc.as_str()), &state_a, pid, &fx.join("A.bundle.json"))?;
                let c = mcp["result"]["commit"]["commit_id"].as_u64().ok_or_else(|| eyre!("MCP submit: no commit id: {mcp}"))?;
                self.s["mcp"] = json!({"tools": mcp["mcp_tools"], "tool_names": mcp["tool_names"], "commit_id": c, "witness_source": mcp["result"]["commit"]["witness_source"],
                                       "commit_tx": mcp["result"]["commit"]["tx"]["tx"], "reveal_tx": mcp["result"]["reveal"]["tx"]["tx"]});
                self.save()?;
                c
            }
        };
        let a_run = root_ctx.commit(a_commit)?["run"].as_u64().unwrap_or_default();
        demo::wait_t1_for(&root_ctx, a_run, T1_WAIT)?;
        // The keeper asks for the pause ticket (300 Tgas set aside) before it pays.
        self.ensure_liquid(&Self::acct("keeper"), FREE_FOR_RUNS)?;
        let keeper_a = cmd::keeper(&self.ctx_of("keeper"), pid)?;
        let steps = keeper_a["steps"].as_array().cloned().unwrap_or_default();
        if !steps.iter().any(|x| x.get("pause_ticket").is_some()) && self.near.view(&self.dep.antibody, "get_program", json!({"program_id": pid}))?["ticket_rounds"] == json!([]) {
            let refused: Vec<&Value> = steps.iter().filter(|x| x.get("pause_ticket_refused").is_some()).collect();
            return Err(eyre!("no pause ticket for A's Critical: {}", serde_json::to_string(&refused)?));
        }
        let paid_a = cmd::paid_of(&root_ctx.commit(a_commit)?);
        let relayer_file = self.relayer_file();
        let key = self.key("acme");
        let mut confirmed: BTreeMap<String, Value> = BTreeMap::new();
        poll("pause() on both chains", Duration::from_secs(900), Duration::from_secs(3), || {
            for v in permits(&relayer_file).values().filter(|v| v["program_key"] == json!(key)) {
                if v["kind"] == "pause" && v["status"] == "confirmed" && v["targets_paused"].as_array().is_some_and(|p| !p.is_empty() && p.iter().all(|x| x == true)) {
                    confirmed.entry(v["chain_id"].to_string()).or_insert_with(|| v.clone());
                }
            }
            (confirmed.len() == 2).then_some(())
        })?;
        // Each moment on its own chain's clock: on NEAR, the block of the receipt that emitted its event
        // (the commit, the verdict, T1, the payment, as the indexer recorded them); each permit when
        // near-guardian signed it; each pause when its chain's receipt came back.
        let commit_tx = self.s["mcp"]["commit_tx"].as_str().map(String::from).filter(|t| !t.is_empty()).unwrap_or_else(|| tx_of("commit", a_commit));
        let reveal_tx = self.s["mcp"]["reveal_tx"].as_str().map(String::from).filter(|t| !t.is_empty()).unwrap_or_else(|| tx_of("reveal", a_commit));
        let pay_tx = keeper_a["steps"].as_array().into_iter().flatten().find(|s| s["paid"] == json!(a_commit)).and_then(|s| s["tx"].as_str()).map(String::from)
            .unwrap_or_else(|| tx_of("paid", a_commit));
        // OutLayer resumes the reveal's receipt to answer, after the indexer has read the reveal: the answer's
        // time is near-antibody's record of it (its receipt's block, to the second).
        let answered_at = root_ctx.view("get_run", json!({"run_id": a_run}))?["outcome"]["Answered"]["at"].as_u64();
        let mut near_at: BTreeMap<&str, u128> = BTreeMap::new();
        if let Some(at) = answered_at {
            near_at.insert("verdict", at as u128 * 1_000_000_000);
        }
        poll("the indexer to reach A's payment", Duration::from_secs(300), Duration::from_secs(3), || {
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
            (["commit", "t1", "paid"].iter().all(|k| near_at.contains_key(k)) && (near_at.contains_key("verdict") || near_at.contains_key("verdict_event"))).then_some(())
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
        println!("A, from its commit's block: verdict {t_verdict:.1} s, T1 {t_t1:.1} s, paid {} wNEAR at {t_paid:.1} s (its block); pause permits signed at {:.1} s and {:.1} s; paused on {SEPOLIA} at {:.1} s and {BASE_SEPOLIA} at {:.1} s",
                 cmd::fmt_tokens(paid_a, 24), signed(&s).unwrap_or(-1.0), signed(&b).unwrap_or(-1.0), at(&s, "confirmed_ms").unwrap_or(-1.0), at(&b, "confirmed_ms").unwrap_or(-1.0));
        self.s["rows"] = json!([{"agent": "A (MCP)", "commit": a_commit, "seconds": t_verdict, "txs": {"commit": commit_tx, "reveal": reveal_tx}}]);
        self.s["one_moment"] = json!({"commit": a_commit, "seconds_from_commit": {"verdict": t_verdict, "t1": t_t1, "paid": t_paid,
                                      format!("permit_signed_{SEPOLIA}"): signed(&s), format!("permit_signed_{BASE_SEPOLIA}"): signed(&b),
                                      format!("paused_{SEPOLIA}"): at(&s, "confirmed_ms"), format!("paused_{BASE_SEPOLIA}"): at(&b, "confirmed_ms")},
                                      "clock": "from the commit's NEAR block: T1 and the payment at their NEAR blocks (the indexer's events), the verdict at its answer's block, each permit when near-guardian signed it, each pause when its chain's receipt came back",
                                      "verdict_from": verdict_from,
                                      "pay_tx": pay_tx, "paid_raw": paid_a.to_string(), "instant": root_ctx.commit(a_commit)?["state"]["Done"]["Paid"]["instant"],
                                      "paused_both": paused_both, "keeper": keeper_a, "permits": permits(&relayer_file)});
        if let Err(e) = self.sweep("agent-a", 60 * MILLI) {
            println!("  agent-a: float not returned ({e})");
        }
        Ok(())
    }

    /// Agents B–G through the CLI; D commits after A; G harms the protected saver and breaks no invariant
    /// (the gap lane). B's costs are measured on testnet's own gas and run deposits.
    fn submissions(&mut self) -> Result<()> {
        let pid = self.pid("acme");
        let root_ctx = self.root_ctx();
        let fx = self.a.work.join("submissions");
        let (near, antibody) = (self.near.clone(), self.dep.antibody.clone());
        let costs_of = |who: &str| -> Result<(u64, u128, u128)> {
            let c = near.account(&antibody)?.ok_or_else(|| eyre!("no contract account"))?;
            let w = near.account(&Self::acct(who))?.ok_or_else(|| eyre!("no account {who}"))?;
            Ok((c["storage_usage"].as_u64().unwrap_or(0), cmd::u128_of(&cmd::account(&root_ctx, &Self::acct(who))?["near"]), cmd::u128_of(&w["amount"])))
        };
        let mut rows = self.s["rows"].as_array().cloned().unwrap_or_default();
        let mut prev: Option<&str> = None;
        for x in ["b", "c", "d", "e", "f", "g"] {
            // Each agent submits once: a stage stopped part-way goes on from the next.
            if rows.iter().any(|r| r["agent"] == x.to_uppercase()) {
                continue;
            }
            // The previous agent's float back, now that its reveal's unused gas has had a submission's
            // time to come back: two floats out at most, not six.
            if let Some(p) = prev.replace(x) {
                if let Err(e) = self.sweep(&format!("agent-{p}"), 60 * MILLI) {
                    println!("  agent-{p}: float not returned yet ({e})");
                }
            }
            let t = Instant::now();
            let report = (x == "c").then(|| fx.join("C.report.md"));
            self.ensure_liquid(&Self::acct(&format!("agent-{x}")), FREE_FOR_RUNS)?;
            let costs_before = if x == "b" { Some(costs_of("agent-b")?) } else { None };
            let r = cmd::submit(&self.ctx_of(&format!("agent-{x}")), pid, &fx.join(format!("{}.bundle.json", x.to_uppercase())), None, report.as_deref(), true)?;
            if let Some((s0, c0, w0)) = costs_before {
                let (s1, c1, w1) = costs_of("agent-b")?;
                self.s["costs"] = json!({"submission": "B", "contract_storage_bytes": s1.saturating_sub(s0), "near_credit_spent_yocto": c0.saturating_sub(c1).to_string(),
                                         "wallet_delta_yocto": (w1 as i128 - w0 as i128).to_string(), "inline_blob_bytes": r["reveal"]["upload"]["inline_bytes"],
                                         "note": "testnet: the run deposit is OutLayer's, and gas at testnet's price"});
            }
            println!("{}: commit {} ({} witness), {:.1} s", x.to_uppercase(), r["commit"]["commit_id"], r["commit"]["witness_source"], t.elapsed().as_secs_f64());
            rows.push(json!({"agent": x.to_uppercase(), "commit": r["commit"]["commit_id"], "seconds": t.elapsed().as_secs_f64(),
                             "txs": {"commit": r["commit"]["tx"]["tx"], "reveal": r["reveal"]["tx"]["tx"]}}));
            self.s["rows"] = Value::Array(rows.clone());
            self.save()?;
        }
        for row in &rows {
            if let Some(r) = root_ctx.commit(row["commit"].as_u64().unwrap_or_default())?["run"].as_u64() {
                demo::wait_t1_for(&root_ctx, r, T1_WAIT)?;
            }
        }
        // The floats still out back, once every reveal's unused gas has come back to its agent.
        for x in ["b", "c", "d", "e", "f", "g"] {
            if let Err(e) = self.sweep(&format!("agent-{x}"), 60 * MILLI) {
                println!("  agent-{x}: float not returned ({e})");
            }
        }
        Ok(())
    }

    /// Above the instant cap: a second run; then the challenge window; the keeper allocates in commit
    /// order and pays; B withdraws its payout.
    fn settle(&mut self) -> Result<()> {
        let pid = self.pid("acme");
        let root_ctx = self.root_ctx();
        let keeper = self.ctx_of("keeper");
        self.ensure_liquid(&Self::acct("keeper"), FREE_FOR_RUNS)?;
        let rows = self.s["rows"].as_array().cloned().unwrap_or_default();
        let program = root_ctx.program(pid)?;
        let mut seconds = Vec::new();
        for row in &rows {
            let id = row["commit"].as_u64().unwrap_or_default();
            let c = root_ctx.commit(id)?;
            let v = cmd::run_view(&cmd::run(&root_ctx, c["run"].as_u64().unwrap_or_default())?);
            if c["state"] == json!("Verifying") && demo::above_cap(&program, &v) {
                let s = cmd::second(&keeper, pid, id, true)?;
                demo::wait_t1_for(&root_ctx, s["run_id"].as_u64().unwrap_or_default(), T1_WAIT)?;
                seconds.push(json!({"finding": row["agent"], "commit": id, "run": s["run_id"], "kind": s["result"]["kind"]}));
            }
        }
        self.s["second_runs"] = Value::Array(seconds);
        // The challenge window, in real time.
        std::thread::sleep(Duration::from_secs(70));
        let mut keeper_runs = Vec::new();
        poll("every claim settled", Duration::from_secs(1_800), Duration::from_secs(20), || {
            keeper_runs.push(cmd::keeper(&keeper, pid).unwrap_or_else(|e| json!({"error": e.to_string()})));
            rows.iter()
                .all(|r| root_ctx.commit(r["commit"].as_u64().unwrap_or_default()).map(|c| !["Verifying", "Holder"].contains(&cmd::state_label(&c["state"]).as_str())).unwrap_or(false))
                .then_some(())
        })?;
        let b = self.ctx_of("agent-b");
        self.ensure_liquid(&Self::acct("agent-b"), 200 * MILLI)?;
        let before = wnear_of(&b, &Self::acct("agent-b"))?;
        cmd::withdraw(&b, "0.03")?;
        let after = wnear_of(&b, &Self::acct("agent-b"))?;
        self.s["keeper"] = keeper_runs.last().cloned().unwrap_or(Value::Null);
        self.s["keeper_runs"] = Value::Array(keeper_runs);
        self.s["withdrawal"] = json!({"account": Self::acct("agent-b"), "wallet_before": cmd::fmt_tokens(before, 24), "wallet_after": cmd::fmt_tokens(after, 24),
                                      "ok": after == before + 30 * MILLI});
        Ok(())
    }

    /// The project reads what it paid for: A's proof, wrapped to its NEP-413 key after payment; D's
    /// duplicate is never wrapped.
    fn project_read(&mut self) -> Result<()> {
        let t = Instant::now();
        let acme = self.ctx_of("acme");
        self.ensure_liquid(&Self::acct("acme"), FREE_FOR_RUNS)?;
        let rows = self.s["rows"].as_array().cloned().unwrap_or_default();
        let reveal_of = |name: &str| rows.iter().find(|r| r["agent"] == name).map(|r| (r["commit"].as_u64().unwrap_or_default(), r["txs"]["reveal"].as_str().unwrap_or_default().to_string()));
        let project_key = self.a.work.join("keys/acme.x25519.json");
        let (a_id, a_reveal) = reveal_of("A (MCP)").ok_or_else(|| eyre!("no row for A"))?;
        let read = cmd::program_read(&acme, a_id, &project_key, &self.a.work.join("read/A"), &cmd::BlobFrom::Tx { hash: a_reveal, signer: None }, false)?;
        let again = self.a.work.join("keys/acme.x25519.again.json");
        let rederived = cmd::keygen_x25519_nep413(&acme, &again)?["public_key"] == self.s["program"]["project_key"]["public_key"];
        let (d_id, d_reveal) = reveal_of("D").ok_or_else(|| eyre!("no row for D"))?;
        let d_blob = cmd::blob_arg_in_tx(&acme, &d_reveal, &Self::acct("agent-d"), d_id)?;
        // A 300 Tgas call: near-antibody must see it to refuse it (0.301 NEAR free again after the read).
        self.ensure_liquid(&Self::acct("acme"), FREE_FOR_RUNS)?;
        let d_refused = cmd::call(&acme, "release_wrap", json!({"commit_id": d_id, "blob": d_blob}), cmd::RUN_TGAS).err().map(|e| e.to_string()).unwrap_or_default();
        let ok = read["local_verdict"]["kind"] == json!("Violation") && read["receipt_matches_the_tee"] == json!(true) && rederived && d_refused.contains("only a paid finding");
        println!("project read A's proof (commit {a_id}): local verdict {}, receipt {} the TEE's; key re-derived: {rederived}; D's release wrap: {}",
                 read["local_verdict"]["kind"], if read["receipt_matches_the_tee"] == json!(true) { "equals" } else { "DIFFERS FROM" },
                 if d_refused.contains("only a paid finding") { "refused" } else { "NOT refused" });
        self.s["project_read"] = json!({"commit": a_id, "read": read, "key_rederived": rederived, "duplicate": d_id, "duplicate_refused": d_refused, "ok": ok,
                                        "seconds": t.elapsed().as_secs_f64()});
        Ok(())
    }

    /// One round of a fix (docs/rounds.md), in real time: the project proposes the prebuilt round manifest
    /// with its releases and the TEE checks both (check_round); the keeper re-runs every eligible commit at
    /// the fix; the round opens once its minimum duration and re-run window have passed.
    fn round(&mut self, round: u32) -> Result<()> {
        let t = Instant::now();
        let (acme, keeper, pid) = (self.ctx_of("acme"), self.ctx_of("keeper"), self.pid("acme"));
        self.ensure_liquid(&Self::acct("acme"), FREE_FOR_RUNS)?;
        self.ensure_liquid(&Self::acct("keeper"), FREE_FOR_RUNS)?;
        let dep = self.s["evm"]["acme"].clone();
        let manifest = self.a.work.join(format!("round{round}/manifest.json"));
        let rel: Vec<Value> =
            round_code(round).1.iter().map(|(proxy, fix)| json!({"proxy": dep[*proxy], "implementation": dep[*fix], "code_hash": dep[format!("codehash{fix}")]})).collect();
        let proposed = cmd::round_propose(&acme, pid, &manifest, &rel, None, true)?;
        let check = proposed["run_id"].as_u64().unwrap_or_default();
        demo::wait_t1_for(&acme, check, T1_WAIT)?;
        let accepted = Instant::now();
        let rc = cmd::run(&acme, check)?["outcome"]["Answered"]["facts"]["extra"]["RoundCheck"].clone();
        if rc["ok"] != json!(true) {
            return Err(eyre!("round {round}: the TEE did not accept the proposal (run {check}): {rc}"));
        }
        let k = cmd::keeper(&keeper, pid)?;
        let runs: Vec<u64> = k["steps"].as_array().into_iter().flatten().filter_map(|s| s["run_id"].as_u64()).collect();
        for r in &runs {
            demo::wait_t1_for(&acme, *r, T1_WAIT)?;
        }
        let mut reruns = Vec::new();
        for r in &runs {
            let v = cmd::run_view(&cmd::run(&acme, *r)?);
            reruns.push(json!({"commit": v["commit_id"], "run": r, "kind": v["kind"], "tier": v["tier"]}));
        }
        self.ensure_liquid(&Self::acct("keeper"), FREE_FOR_RUNS)?;
        // The re-run window runs from the proposal's T1; the round opens after it.
        let window = Duration::from_secs(600 + 20);
        if let Some(left) = window.checked_sub(accepted.elapsed()) {
            println!("  round {round}: {} re-runs T1; the re-run window closes in {:.0} s", reruns.len(), left.as_secs_f64());
            std::thread::sleep(left);
        }
        let opened = poll(&format!("round {round} to open"), Duration::from_secs(900), Duration::from_secs(60), || {
            match cmd::round_open(&keeper, pid) {
                Ok(v) => Some(v["round_opened"].clone()),
                Err(e) => {
                    println!("  round {round} not open yet: {e}");
                    None
                }
            }
        })?;
        // Release permits beyond one call's gas are queued at near-guardian; the keeper asks for them.
        let k2 = cmd::keeper(&keeper, pid)?;
        let n = |v: &Value| v.as_u64().unwrap_or(0);
        let keys = |k: u64| if k == 1 { "key" } else { "keys" };
        let summary = format!("{} re-runs ({} fixed); release gate {} ({}); {} {} released, {} carried", reruns.len(),
                              reruns.iter().filter(|r| r["kind"] == "NoViolation").count(), opened["release_gate"],
                              if opened["release_gate"] == json!(true) { "release permits signed" } else { "no release permit" },
                              n(&opened["released"]), keys(n(&opened["released"])), n(&opened["carried"]));
        println!("round {round}: {summary}");
        let mut rounds = self.s["rounds"].as_array().cloned().unwrap_or_default();
        rounds.push(json!({"round": round, "manifest_sha256": proposed["manifest_sha256"], "check_round_run": check, "canaries_ok": rc["canaries_ok"],
                           "releases": rel, "reruns": reruns, "opened": opened, "summary": summary, "keeper": [k, k2], "propose_tx": proposed["tx"]["tx"],
                           "seconds": t.elapsed().as_secs_f64()}));
        self.s["rounds"] = Value::Array(rounds);
        Ok(())
    }

    /// The guardian signs a release permit per proxy and chain; the relayer upgrades both proxies on both
    /// chains; B's exploit reverts against the released code.
    fn release(&mut self) -> Result<()> {
        let t = Instant::now();
        let relayer_file = self.relayer_file();
        let key = self.key("acme");
        let ours = |v: &Value| v["program_key"] == json!(key) && v["kind"] == "release";
        poll("4 releases confirmed", Duration::from_secs(1_200), Duration::from_secs(5), || {
            (permits(&relayer_file).values().filter(|v| ours(v) && v["status"] == "confirmed").count() == 4).then_some(())
        })?;
        let mut released = Vec::new();
        for (chain, rpc, dep) in [(SEPOLIA, self.sepolia(), &self.s["evm"]["acme"]), (BASE_SEPOLIA, self.a.base_rpc.clone(), &self.s["evm"]["acme_base_sepolia"])] {
            let evm = Evm::new(&rpc);
            for (proxy, fix) in round_code(2).1 {
                // A load-balanced RPC may answer from a node a block behind the upgrade: the slot is read
                // until it shows the fix, for a minute and a half at most.
                let want = dep[*fix].as_str().unwrap_or_default().to_string();
                let until = Instant::now() + Duration::from_secs(90);
                let now = loop {
                    let word = evm.read("eth_getStorageAt", json!([dep[*proxy], format!("{:#x}", antibody_verifier::run::EIP1967_IMPLEMENTATION_SLOT), "latest"]))?;
                    let now = format!("0x{}", &word.as_str().unwrap_or_default().trim_start_matches("0x")[24..]);
                    if now.eq_ignore_ascii_case(&want) || Instant::now() > until {
                        break now;
                    }
                    std::thread::sleep(Duration::from_secs(5));
                };
                released.push(json!({"chain_id": chain, "rpc": rpc, "proxy": proxy, "address": dep[*proxy], "implementation": now, "expected": dep[*fix],
                                     "ok": now.eq_ignore_ascii_case(dep[*fix].as_str().unwrap_or_default())}));
            }
            // B's exploit, the unguarded fee sweep, against the released code.
            let sweep = format!("0x{}{}", hex::encode(&keccak256(b"sweepFees(address)")[..4]), "00".repeat(12) + "000000000000000000000000000000000000beef");
            let reverts = evm.call("eth_call", json!([{"from": "0x000000000000000000000000000000000000beef", "to": dep["AcmeVault"], "data": sweep}, "latest"])).is_err();
            released.push(json!({"chain_id": chain, "b_exploit_reverts": reverts}));
        }
        let ok = released.iter().all(|r| r["ok"] != json!(false) && r["b_exploit_reverts"] != json!(false));
        let txs: Vec<Value> = permits(&relayer_file).values().filter(|v| ours(v)).cloned().collect();
        println!("released on both chains in {:.1} s: {}", t.elapsed().as_secs_f64(), if ok { "both proxies run the final fix; B's exploit reverts" } else { "MISMATCH" });
        self.s["release"] = json!({"checks": released, "ok": ok, "permits": txs, "seconds": t.elapsed().as_secs_f64()});
        Ok(())
    }

    /// The practice program (a tutorial): 24 vaults, one public bug; the vending contract; two judges with
    /// no account try it.
    fn practice(&mut self) -> Result<()> {
        let t = Instant::now();
        let root_ctx = self.root_ctx();
        let want = self.pid("practice");
        let pm_path = self.a.work.join("practice-manifest.json");
        let pm = antibody_verifier::manifest::Manifest::parse(&std::fs::read(&pm_path)?).map_err(|e| eyre!(e))?;
        // Each step once: a stage stopped part-way goes on from where it stopped.
        let ppid = match self.s["practice"]["program_id"].as_u64() {
            Some(p) => p as u32,
            None => {
                // Two escrows (practice, fork mode) and the vending contract's bond budget.
                if wnear_of(&root_ctx, HQ)? < 70 * MILLI {
                    wrap(&root_ctx, 70 * MILLI)?;
                }
                let practice_key = self.a.work.join("keys/practice.x25519.json");
                cmd::keygen_x25519(&practice_key)?;
                self.ensure_liquid(HQ, FREE_FOR_RUNS)?;
                let pc = cmd::program_create(&root_ctx, &pm_path, PRACTICE_ESCROW, &practice_key, None)?;
                let ppid = pc["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no practice program id: {pc}"))? as u32;
                if ppid != want {
                    return Err(eyre!("practice program {ppid} was created, but its base witness was built for {want}"));
                }
                self.s["practice"] = json!({"program_id": ppid, "create_tx": pc["tx"]["tx"], "manifest_sha256": hex::encode(sha256(&pm.canonical()))});
                self.save()?;
                ppid
            }
        };
        if root_ctx.program(ppid)?["sealing_pk"].is_null() {
            self.ensure_liquid(HQ, FREE_FOR_RUNS)?;
            let pact = cmd::program_activate(&root_ctx, ppid, &[demo::tutorial_cover(&self.a.work, "practice", &pm)?], true)?;
            demo::wait_t1_for(&root_ctx, pact["run_id"].as_u64().unwrap_or_default(), T1_WAIT)?;
        }
        if root_ctx.program(ppid)?["phase"] != json!("Open") {
            cmd::program_open(&root_ctx, ppid)?;
        }
        let vending = Self::acct("practice");
        let wasm = deployable(&self.a.root, "antibody_practice_vending", "contracts/practice-vending/target/wasm32-unknown-unknown/release/antibody_practice_vending.wasm")?;
        let account_wasm = deployable(&self.a.root, "antibody_practice_account", "contracts/practice-account/target/wasm32-unknown-unknown/release/antibody_practice_account.wasm")?;
        let code_bytes = std::fs::metadata(&wasm)?.len() as u128 + std::fs::metadata(&account_wasm)?.len() as u128;
        match self.near.account(&vending)? {
            None => {
                // Its code (and the practice-account code it keeps), its state, two tries' gas funds at once
                // (each comes back when its try closes the account), and a margin for the deploy.
                let amount = (code_bytes + 8_000) * 10u128.pow(19) + 2 * PRACTICE_GAS_FUNDS + 150 * MILLI;
                cmd::admin_create_account(&root_ctx, &vending, &cmd::fmt_tokens(amount, 24), None, Some(&self.a.keys.join(format!("{vending}.json"))))?;
            }
            Some(a) if a["code_hash"] != json!("11111111111111111111111111111111") && self.s["practice"]["vending"].is_null() => {
                // An earlier run's vending contract (locked, at launch): this build's code, pointed at the
                // new program by its owner.
                let built = bs58::encode(sha256(&std::fs::read(&wasm)?)).into_string();
                if a["code_hash"] != json!(built) {
                    return Err(eyre!("{vending} runs code {} and this build's vending contract is {built}: delete the account (its NEAR back to {HQ}) or deploy this build's code", a["code_hash"]));
                }
                let terms = practice::Terms { gas_funds: PRACTICE_GAS_FUNDS, allowance: PRACTICE_ALLOWANCE, bond_budget: 10 * MILLI, per_commit_cap: MILLI, per_hour: 6 };
                self.s["practice"]["vending"] = practice::retarget(&root_ctx, &vending, &account_wasm, ppid, 24, &terms)?;
                self.s["practice"]["vending_terms"] = json!({"program": ppid, "gas_funds": PRACTICE_GAS_FUNDS.to_string(), "allowance": PRACTICE_ALLOWANCE.to_string()});
                // Two tries' gas funds at once, and the margin a vend needs.
                let free = cmd::u128_of(&a["amount"]).saturating_sub(cmd::u128_of(&a["storage_usage"]) * 10u128.pow(19));
                let want = 2 * PRACTICE_GAS_FUNDS + 150 * MILLI;
                if free < want {
                    cmd::admin_send_near(&root_ctx, &vending, &cmd::fmt_tokens(want - free, 24))?;
                }
                self.save()?;
            }
            Some(_) => {}
        }
        let terms = practice::Terms { gas_funds: PRACTICE_GAS_FUNDS, allowance: PRACTICE_ALLOWANCE, bond_budget: 10 * MILLI, per_commit_cap: MILLI, per_hour: 6 };
        let want_terms = json!({"program": ppid, "gas_funds": PRACTICE_GAS_FUNDS.to_string(), "allowance": PRACTICE_ALLOWANCE.to_string()});
        if self.s["practice"]["vending"].is_null() {
            // The public vend key's allowance is a ceiling (each vend counts its 60 Tgas set aside): a locked
            // vending contract's key cannot be given more.
            self.s["practice"]["vending"] = practice::deploy(&self.ctx_of("practice"), &wasm, &account_wasm, HQ, ppid, 24, &terms, 10 * ONE_NEAR)?;
            self.s["practice"]["vending_terms"] = want_terms.clone();
            self.save()?;
        }
        if self.s["practice"]["vending_terms"] != want_terms {
            // Terms this build changed since the vending contract got them: its owner sets them again.
            self.s["practice"]["retargeted"] = practice::retarget(&root_ctx, &vending, &account_wasm, ppid, 24, &terms)?;
            self.s["practice"]["vending_terms"] = want_terms;
            self.save()?;
        }
        if self.s["practice"]["fund"].is_null() {
            self.ensure_liquid(HQ, FREE_FOR_RUNS)?;
            self.s["practice"]["fund"] = practice::fund(&root_ctx, &vending, ONE_NEAR * 15 / 100, 10 * MILLI)?;
            self.save()?;
        }
        println!("practice program {ppid} open; vending {vending} with the public vend key {}", self.s["practice"]["vending"]["public_vend_key"]);
        let mut served = self.dep.clone();
        served.practice = Some(PracticeDeployment { vending: vending.clone(), program: ppid });
        let judge = self.judge_ctx(&served);
        // What the vending contract holds: its NEAR, and its NEAR credit at near-antibody, which pays a try's
        // run and commit storage (OutLayer refunds a run's unused deposit to the vending contract's NEAR).
        let holds = |who: &str| -> Result<u128> {
            Ok(self.near.account(who)?.map(|a| cmd::u128_of(&a["amount"])).unwrap_or(0) + cmd::u128_of(&cmd::account(&root_ctx, who)?["near"]))
        };
        let mut tries = self.s["practice"]["tries"].as_array().cloned().unwrap_or_default();
        for t in TRIERS {
            if tries.iter().any(|r| r["trier"] == json!(t)) {
                continue;
            }
            // A vending contract keeps the names it vended: each program's tries have their own, and a try
            // that stopped part-way leaves its name taken (its account closed), so the next one is free.
            let base = format!("{t}-p{ppid}");
            let mut name = base.clone();
            for k in 2.. {
                if practice::vault_of(&root_ctx, &vending, &format!("{name}.{vending}"))?.is_none() {
                    break;
                }
                name = format!("{base}-{k}");
            }
            let before = holds(&vending)?;
            let mut r = match practice::try_it(&judge, &vending, ppid, &name) {
                Ok(r) => r,
                Err(e) => {
                    let mut failed = self.s["practice"]["failed_tries"].as_array().cloned().unwrap_or_default();
                    failed.push(json!({"trier": t, "account": format!("{name}.{vending}"), "error": format!("{e:#}")}));
                    self.s["practice"]["failed_tries"] = Value::Array(failed);
                    self.save()?;
                    return Err(e);
                }
            };
            r["trier"] = json!(t);
            // What the try cost the vending contract, once its account closed (its NEAR came back): gas, the
            // run's charge and the commit's storage. Our relayer's T1 relay is not in it.
            std::thread::sleep(Duration::from_secs(5));
            let after = holds(&vending)?;
            r["vending_spent_yocto"] = json!(before.saturating_sub(after).to_string());
            println!("{}: vault {} ({}), witness from the {} ({} bytes), {} {}, {} in {:.1} s; closed: {}, cost the vending {} NEAR",
                     r["account"], r["vault_index"], r["vault"], r["witness_source"].as_str().unwrap_or("-"), r["witness_bytes"],
                     r["verdict"]["kind"].as_str().unwrap_or("-"), r["t1"]["tier"].as_str().unwrap_or("-"), r["state"].as_str().unwrap_or("-"),
                     r["seconds"].as_f64().unwrap_or_default(), r["closed"], cmd::fmt_tokens(before.saturating_sub(after), 24));
            tries.push(r);
            self.s["practice"]["tries"] = Value::Array(tries.clone());
            self.save()?;
        }
        // A try pays its own commit when the queue lets it. An earlier commit still waiting holds the queue
        // in commit order (one never revealed waits out its reveal window), and the keeper moves it on:
        // each try's final state, once the keeper has run on the program.
        if tries.iter().any(|r| r["final_state"].as_str().is_none_or(|f| !f.starts_with("Done:"))) {
            // The keeper's calls set aside up to 300 Tgas (allocate 150, a pause ticket 300).
            self.ensure_liquid(&Self::acct("keeper"), FREE_FOR_RUNS)?;
            let keeper = self.ctx_of("keeper");
            let window = |k: &str| pm.settlement.as_ref().and_then(|s| s["windows"][k].as_u64()).unwrap_or(600);
            let until = Instant::now() + Duration::from_secs(window("reveal") + window("challenge") + 600);
            loop {
                let mut done = true;
                for r in tries.iter_mut() {
                    let c = root_ctx.commit(r["commit_id"].as_u64().unwrap_or_default())?;
                    let state = cmd::state_label(&c["state"]);
                    done &= state.starts_with("Done:");
                    if r["state"] != json!(state) {
                        r["settled_by"] = json!("keeper");
                    }
                    r["final_state"] = json!(state);
                    r["final_paid_raw"] = json!(cmd::paid_of(&c).to_string());
                }
                if done || Instant::now() > until {
                    break;
                }
                match cmd::keeper(&keeper, ppid) {
                    // What it did, and what stopped it (a call refused, or one it could not pay for).
                    Ok(k) => {
                        for st in k["steps"].as_array().into_iter().flatten() {
                            println!("  keeper on program {ppid}: {}", st.to_string().chars().take(300).collect::<String>());
                        }
                    }
                    Err(e) => println!("  keeper on program {ppid}: {e}"),
                }
                std::thread::sleep(Duration::from_secs(30));
            }
            self.s["practice"]["tries"] = Value::Array(tries.clone());
            self.save()?;
        }
        let ok = tries.iter().all(|r| r["final_state"] == json!("Done:Paid") && r["final_paid_raw"] == json!(MILLI.to_string()) && r["closed"] == json!(true))
            && tries[0]["vault"] != tries[1]["vault"]
            && tries.iter().all(|r| r["vended_by"] == json!(vending) && r["witness_source"] == json!("base"));
        self.s["practice"]["ok"] = json!(ok);
        self.s["practice"]["terms"] = json!(PRACTICE_TERMS);
        self.s["practice"]["seconds"] = json!(t.elapsed().as_secs_f64());
        std::fs::write(self.a.work.join("deployments.json"), serde_json::to_vec_pretty(&served)?)?;
        Ok(())
    }

    /// Fork mode: the practice program again, deployed only inside the verifier (nothing of it is on chain).
    fn fork(&mut self) -> Result<()> {
        let t = Instant::now();
        let root_ctx = self.root_ctx();
        let want = self.pid("fork");
        let fm_path = self.a.work.join("practice-fork-manifest.json");
        let fm = antibody_verifier::manifest::Manifest::parse(&std::fs::read(&fm_path)?).map_err(|e| eyre!(e))?;
        // Each step once: a stage stopped part-way goes on from where it stopped.
        let (fpid, create_tx) = match self.s["fork_mode"]["program_id"].as_u64() {
            Some(p) => (p as u32, self.s["fork_mode"]["create_tx"].clone()),
            None => {
                let fork_key = self.a.work.join("keys/practice-fork.x25519.json");
                cmd::keygen_x25519(&fork_key)?;
                self.ensure_liquid(HQ, FREE_FOR_RUNS)?;
                let fc = cmd::program_create(&root_ctx, &fm_path, PRACTICE_ESCROW, &fork_key, None)?;
                let fpid = fc["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no fork-mode program id: {fc}"))? as u32;
                if fpid != want {
                    return Err(eyre!("fork-mode program {fpid} was created, but its base witness was built for {want}"));
                }
                self.s["fork_mode"] = json!({"program_id": fpid, "create_tx": fc["tx"]["tx"]});
                self.save()?;
                (fpid, fc["tx"]["tx"].clone())
            }
        };
        if root_ctx.program(fpid)?["sealing_pk"].is_null() {
            self.ensure_liquid(HQ, FREE_FOR_RUNS)?;
            let fact = cmd::program_activate(&root_ctx, fpid, &[demo::tutorial_cover(&self.a.work, "practice-fork", &fm)?], true)?;
            demo::wait_t1_for(&root_ctx, fact["run_id"].as_u64().unwrap_or_default(), T1_WAIT)?;
        }
        if root_ctx.program(fpid)?["phase"] != json!("Open") {
            cmd::program_open(&root_ctx, fpid)?;
        }
        let vault3 = fm.targets.iter().find(|x| x.name == "vault-3").map(|x| format!("{:#x}", x.address)).ok_or_else(|| eyre!("no vault-3"))?;
        let on_chain = Evm::new(&self.sepolia()).read("eth_getCode", json!([vault3, "latest"]))?;
        let bundle = self.a.work.join("fork-tutorial.bundle.json");
        std::fs::write(&bundle, serde_json::to_vec_pretty(&practice::tutorial_bundle(&vault3))?)?;
        // Agent C (its own proof forfeited its bond, not its NEAR) submits the tutorial proof.
        let c = self.ctx_of("agent-c");
        self.ensure_liquid(&Self::acct("agent-c"), FREE_FOR_RUNS)?;
        let fr = cmd::submit(&c, fpid, &bundle, None, None, true)?;
        let fcid = fr["commit"]["commit_id"].as_u64().ok_or_else(|| eyre!("no fork-mode commit id: {fr}"))?;
        let frun = demo::wait_t1_for(&root_ctx, root_ctx.commit(fcid)?["run"].as_u64().unwrap_or_default(), T1_WAIT)?;
        let fkeeper = cmd::keeper(&self.ctx_of("keeper"), fpid)?;
        let fcommit = root_ctx.commit(fcid)?;
        let ok = cmd::state_label(&fcommit["state"]) == "Done:Paid"
            && cmd::paid_of(&fcommit) == MILLI
            && frun["kind"] == json!("Violation")
            && frun["tier"] == json!("T1")
            && on_chain == json!("0x")
            && fr["commit"]["witness_source"] == json!("base");
        println!("fork-mode program {fpid}: {} deploy transactions, {} targets, vault-3 on chain: {on_chain}; the tutorial proof: {} {} ({} witness), {} in {:.1} s",
                 fm.deploy.as_ref().map(|d| d.txs.len()).unwrap_or(0), fm.targets.len(), frun["kind"].as_str().unwrap_or("-"), frun["tier"].as_str().unwrap_or("-"),
                 fr["commit"]["witness_source"].as_str().unwrap_or("-"), cmd::state_label(&fcommit["state"]), t.elapsed().as_secs_f64());
        self.s["fork_mode"] = json!({"program_id": fpid, "manifest_sha256": hex::encode(sha256(&fm.canonical())), "create_tx": create_tx,
                                     "deploy_txs": fm.deploy.as_ref().map(|d| d.txs.len()), "targets": fm.targets.len(), "vault_3": vault3,
                                     "vault_3_code_on_chain": on_chain, "commit": fcid, "hunter": Self::acct("agent-c"), "verdict": fr["reveal"]["result"], "tier": frun["tier"],
                                     "witness_source": fr["commit"]["witness_source"], "state": cmd::state_label(&fcommit["state"]), "paid_raw": cmd::paid_of(&fcommit).to_string(),
                                     "keeper": fkeeper, "txs": {"commit": fr["commit"]["tx"]["tx"], "reveal": fr["reveal"]["tx"]["tx"]}, "ok": ok,
                                     "seconds": t.elapsed().as_secs_f64()});
        Ok(())
    }

    /// Every expected outcome, the audits, the judge's view, the deployment record and the API, into
    /// `<work>/results.json` (the sandbox's shape).
    fn results(&mut self) -> Result<()> {
        let root_ctx = self.root_ctx();
        let pid = self.pid("acme");
        let expected: BTreeMap<&str, (&str, &str, u128)> = BTreeMap::from([
            ("A (MCP)", ("Violation", "Done:Paid", 100 * MILLI)),
            ("B", ("Violation", "Done:Paid", 30 * MILLI)),
            ("C", ("NoViolation", "Done:Forfeited", 0)),
            ("D", ("Violation", "Done:DuplicateConfirmed", 0)),
            ("E", ("Violation", "Done:Known", 0)),
            ("F", ("Violation", "Done:Paid", 100 * MILLI)),
            ("G", ("NoViolation", "Done:Paid", 10 * MILLI)),
        ]);
        let mut ok = self.s["known"]["count"] == json!(1) && self.s["withdrawal"]["ok"] == json!(true) && self.s["evm"]["same_addresses"] == json!(true);
        let one = &self.s["one_moment"];
        ok &= one["paused_both"] == json!(true) && one["instant"] == json!(true);
        let gate = |r: &Value| r["opened"]["release_gate"].as_bool();
        ok &= gate(&self.s["rounds"][0]) == Some(false) && self.s["rounds"][0]["opened"]["carried"] == json!(2);
        ok &= gate(&self.s["rounds"][1]) == Some(true) && self.s["rounds"][1]["opened"]["released"] == json!(2);
        for k in ["release", "practice", "fork_mode", "project_read"] {
            ok &= self.s[k]["ok"] == json!(true);
        }
        let mut pays = BTreeMap::new();
        demo::pay_txs(&self.s, &mut pays);
        let mut table = Vec::new();
        println!("{:<8} {:>6} {:<12} {:<5} {:<15} {:>10} {:>8}", "agent", "commit", "verdict", "tier", "state", "paid wNEAR", "seconds");
        for row in self.s["rows"].as_array().cloned().unwrap_or_default() {
            let agent = row["agent"].as_str().unwrap_or_default().to_string();
            let id = row["commit"].as_u64().unwrap_or_default();
            let c = root_ctx.commit(id)?;
            let v = cmd::run_view(&cmd::run(&root_ctx, c["run"].as_u64().unwrap_or_default())?);
            let kind = v["kind"].as_str().unwrap_or("-").to_string();
            let state = cmd::state_label(&c["state"]);
            let paid = cmd::paid_of(&c);
            let (ek, es, ep) = expected[agent.as_str()];
            let pass = kind == ek && state == es && paid == ep && v["tier"] == json!("T1");
            ok &= pass;
            println!("{:<8} {:>6} {:<12} {:<5} {:<15} {:>10} {:>8.1} {}", agent, id, kind, v["tier"].as_str().unwrap_or("-"), state, cmd::fmt_tokens(paid, 24),
                     row["seconds"].as_f64().unwrap_or_default(), if pass { "OK" } else { "MISMATCH" });
            let mut txs = row["txs"].clone();
            txs["pay"] = pays.get(&id).cloned().unwrap_or(Value::Null);
            table.push(json!({"agent": agent, "commit": id, "run": c["run"], "verdict": kind, "tier": v["tier"], "keys": v["keys"], "state": c["state"],
                              "paid_raw": paid.to_string(), "seconds": row["seconds"], "txs": txs, "ok": pass}));
        }
        let ppid = self.pid("practice");
        let fpid = self.pid("fork");
        let audit = root_ctx.view("audit", json!({"program_id": pid}))?;
        let contract_audit = root_ctx.view("audit_contract", json!({}))?;
        let practice_audit = root_ctx.view("audit", json!({"program_id": ppid}))?;
        let fork_audit = root_ctx.view("audit", json!({"program_id": fpid}))?;
        ok &= [&audit, &contract_audit, &practice_audit, &fork_audit].iter().all(|a| a["ok"] == json!(true));
        let mut served = self.dep.clone();
        served.practice = Some(PracticeDeployment { vending: Self::acct("practice"), program: ppid });
        let judge_ctx = self.judge_ctx(&served);
        let judged = antibody_cli::judge::judge(&judge_ctx, None)?;
        // By id: a practice stage run again (`--redo-practice`) leaves the earlier programs listed.
        let program = |id: u32| judged["programs"].as_array().and_then(|ps| ps.iter().find(|p| p["program_id"] == json!(id))).cloned().unwrap_or(Value::Null);
        let (acme, practice, fork) = (program(pid), program(ppid), program(fpid));
        let judge_ok = acme["verdicts"]["t1_confirmed"].as_u64() >= Some(7)
            && acme["pauses"].as_u64() == Some(2)
            && acme["rule_strength"]["statement"].is_string()
            && practice["payouts"].as_array().map(|p| p.len()) == Some(2)
            && fork["payouts"].as_array().map(|p| p.len()) == Some(1);
        ok &= judge_ok;
        let record = antibody_cli::judge::deployment_record(&judge_ctx, None)?;
        let record_ok = record["contracts"].as_array().map(|c| c.len()) == Some(4) && record["verifier"]["wasm_hash"].is_string();
        ok &= record_ok;
        println!("audits ok: {}; judge ok: {judge_ok}; deployment record ({} contracts) ok: {record_ok}",
                 [&audit, &contract_audit, &practice_audit, &fork_audit].iter().all(|a| a["ok"] == json!(true)), record["contracts"].as_array().map(|c| c.len()).unwrap_or(0));

        // The API on the indexer's events, each endpoint once.
        let dir = self.services_dir();
        let indexed = crate::indexer::read_events(&dir.join("events.jsonl"), 0, usize::MAX).len();
        let api = crate::api::Api::new(self.near.clone(), served.clone(), dir.clone(), None, Default::default(), std::path::PathBuf::from("docs/results"));
        let rt = tokio::runtime::Runtime::new()?;
        let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))?;
        let addr = listener.local_addr()?;
        rt.spawn(async move { axum::serve(listener, crate::api::router(api)).await });
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(180)).build()?;
        let a_commit = self.s["one_moment"]["commit"].as_u64().unwrap_or_default();
        let paths = vec![
            "/health".to_string(), "/v1/status".into(), "/v1/stats".into(), "/v1/programs".into(), format!("/v1/programs/{pid}"), format!("/v1/programs/{ppid}/feed"),
            format!("/v1/commits/{a_commit}"), format!("/v1/runs/{}", self.s["activation"]["run_id"]), format!("/v1/accounts/{}", Self::acct("agent-a")),
            "/v1/leaderboard".into(), "/v1/events?limit=5".into(), "/v1/guardian".into(), "/v1/judge".into(), format!("/v1/programs/{pid}/reflex"),
            "/v1/trust".into(), "/v1/practice".into(), "/v1/mainnet".into(),
        ];
        let mut examples = serde_json::Map::new();
        for path in &paths {
            let r = client.get(format!("http://{addr}{path}")).send()?;
            let code = r.status().as_u16();
            examples.insert(path.clone(), json!({"status": code, "body": r.json::<Value>().unwrap_or(Value::Null)}));
        }
        let api_ok = examples.values().all(|e| e["status"] == json!(200)) && indexed > 0;
        ok &= api_ok;
        println!("API: {indexed} events indexed; {} endpoints; ok: {api_ok}", examples.len());
        std::fs::write(self.a.work.join("api-examples.json"), serde_json::to_vec_pretty(&examples)?)?;

        let s = &mut self.s;
        s["network"] = json!("testnet");
        s["submissions"] = Value::Array(table);
        s["audit"] = json!({"program": audit, "practice": practice_audit, "fork_mode": fork_audit, "contract": contract_audit});
        s["record"] = root_ctx.view("program_record", json!({"program_id": pid}))?;
        s["judge"] = judged;
        s["deployment_record"] = record;
        s["deployment"] = json!({"network": "testnet", "near_antibody": self.dep.antibody, "guardian": self.dep.guardian, "token": self.dep.token,
                                 "verifier_wasm_sha256": serde_json::to_value(&self.dep)?["release"]["verifier"]["wasm_hash"],
                                 "images": self.near.view(&self.dep.antibody, "get_images", json!({}))?});
        s["verdict_consumer"] = json!({"skipped": "not deployed on testnet (its code stake did not fit the lean budget); the sandbox run shows it"});
        s["api"] = json!({"events_indexed": indexed, "endpoints": paths, "ok": api_ok, "examples": self.a.work.join("api-examples.json"),
                          "failures": examples.iter().filter(|(_, e)| e["status"] != json!(200)).map(|(k, e)| json!({"path": k, "status": e["status"]})).collect::<Vec<_>>()});
        s["all_ok"] = json!(ok);
        std::fs::write(self.a.work.join("results.json"), serde_json::to_vec_pretty(&self.s)?)?;
        if !ok {
            return Err(eyre!("the live story finished with mismatches (see {})", self.a.work.join("results.json").display()));
        }
        println!("\nALL OK — results: {}", self.a.work.join("results.json").display());
        Ok(())
    }
}

pub fn run(a: LiveArgs) -> Result<Value> {
    // In-process CLI calls and agent A's MCP server read it (`cmd::min_near_credit`).
    std::env::set_var("ANTIBODY_MIN_NEAR_CREDIT", MIN_NEAR_CREDIT);
    let dep: Deployments = serde_json::from_str(&std::fs::read_to_string(a.root.join("deployments/near-testnet.json"))?)?;
    let near = Near::new("testnet", dep.rpc.clone());
    std::fs::create_dir_all(a.work.join("keys"))?;
    let s: Value = std::fs::read(a.work.join("live.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({"done": [], "started_unix": unix()}));
    let mut l = Live { a, near, dep, s, stop: Arc::new(AtomicBool::new(false)), threads: Vec::new() };
    if l.a.redo_practice {
        if !l.done("release") {
            return Err(eyre!("--redo-practice runs once the story has run up to the practice stage"));
        }
        l.redo_practice()?;
    }
    if l.a.t2 {
        if !l.done("results") {
            return Err(eyre!("--t2 runs once the story has run"));
        }
        step("live: T2 (near-t2 on testnet, the four disclosed runs' proofs)");
        let ok = l.t2()?;
        let results = l.a.work.join("results.json");
        std::fs::write(&results, serde_json::to_vec_pretty(&l.s)?)?;
        if !ok {
            return Err(eyre!("near-t2 did not act as expected (see t2 in {})", results.display()));
        }
        println!("T2: ok, results: {}", results.display());
        return Ok(l.s.clone());
    }
    if l.a.verdict_consumer {
        if !l.done("results") {
            return Err(eyre!("--verdict-consumer runs once the story has run: it reports the run's commits"));
        }
        step("live: verdict consumer (the verdict guard on testnet)");
        let ok = l.verdict_consumer()?;
        let results = l.a.work.join("results.json");
        std::fs::write(&results, serde_json::to_vec_pretty(&l.s)?)?;
        if !ok {
            return Err(eyre!("the verdict guard did not act as expected (see verdict_consumer in {})", results.display()));
        }
        println!("verdict consumer: ok — results: {}", results.display());
        return Ok(l.s.clone());
    }
    let services_from = STAGES.iter().position(|s| *s == "program").unwrap_or(0);
    for (i, stage) in STAGES.iter().enumerate() {
        if l.done(stage) {
            continue;
        }
        if i >= services_from {
            l.start_services()?;
            l.ensure_liquid(&Live::acct("relayer"), 600 * MILLI)?;
        }
        step(&format!("live: {stage}"));
        let t = Instant::now();
        let r = match *stage {
            "evm" => l.evm(),
            "anchor" => l.anchor(),
            "prebuild" => l.prebuild(),
            "light-client" => l.light_client(),
            "program" => l.program(),
            "guardian" => l.guardian(),
            "activate" => l.activate(),
            "agents" => l.agents(),
            "one-moment" => l.one_moment(),
            "submissions" => l.submissions(),
            "settle" => l.settle(),
            "project-read" => l.project_read(),
            "round-1" => l.round(1),
            "round-2" => l.round(2),
            "release" => l.release(),
            "practice" => l.practice(),
            "fork" => l.fork(),
            _ => l.results(),
        };
        l.s["seconds"][*stage] = json!(t.elapsed().as_secs_f64());
        if let Err(e) = r {
            // A failed prebuild needs a newer anchor (the RPC's proof window moves on).
            if *stage == "prebuild" {
                l.set_done("anchor", false);
            }
            l.save()?;
            l.stop_services();
            return Err(e.wrap_err(format!("live stage {stage}")));
        }
        l.set_done(stage, true);
        l.save()?;
        if l.a.until.as_deref() == Some(*stage) {
            break;
        }
    }
    l.stop_services();
    Ok(l.s)
}

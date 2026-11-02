//! `antibody-services demo`: the offline, keyless end-to-end run (and integration test).
//!
//! near-sandbox and an anvil chain (31337) with the Acme Lend fixture; the real near-antibody and
//! tUSD contracts; MOCK OutLayer and MOCK DCAP contracts. The mock worker runs the real verifier (TEE
//! mode, TEST root key) and records a MOCK attestation per run; the T1 relayer relays it, and
//! near-antibody runs its real T1 checks on it. Flow: program (manifest v2) → activation (T1) →
//! known issue (T1) → open → agents B, A (over MCP, with a function-call key), C–F → second runs for
//! verdicts above the instant cap → the keeper allocates in commit order and pays → a withdrawal →
//! audit(). Every expected outcome is asserted.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use alloy_primitives::{keccak256, B256};
use antibody_cli::{
    cmd,
    ctx::{Ctx, Deployments},
    near::{Near, Signer},
};
use antibody_rules::settlement::{Fees, Instant as InstantRule, Known, Pause, Payouts, SWindows, Settlement};
use antibody_verifier::{
    facts::sha256,
    manifest::{Anchor, Capabilities, ImpactConfig, Invariants, Manifest, PricedToken, ProgramInfo, Severity, Slot, Target, MANIFEST_SCHEMA_V2},
};
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{
    evm::Evm,
    mocks::{demo_mpc_key, demo_seed, eth_address, mock_image, MockWorker},
    t1::{Source, T1Relayer},
};

pub struct DemoArgs {
    pub root: PathBuf,
    pub work: PathBuf,
    pub sandbox_bin: PathBuf,
    pub anvil_bin: PathBuf,
    pub forge_bin: PathBuf,
    pub antibody_bin: PathBuf,
    pub keep: bool,
}

/// Kills child processes when the demo ends, whatever the outcome.
struct Procs(Vec<Child>);
impl Drop for Procs {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

pub(crate) fn step(name: &str) {
    println!("\n== {name}");
}

pub(crate) fn wait_for<F: FnMut() -> bool>(what: &str, timeout: Duration, mut f: F) -> Result<()> {
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if f() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(eyre!("timed out waiting for {what}"))
}

const ANVIL_SENDER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
pub(crate) const ONE_TUSD: u128 = 1_000_000;
const ESCROW_TUSD: &str = "40000";
/// The sandbox's terms text; its sha256 is committed in the manifest and named by every commit.
const TERMS: &str = "Acme Lend (sandbox demo) terms: a DELIBERATELY VULNERABLE TEST FIXTURE. Payouts in tUSD, a testnet token with no value.";

fn start_anvil(bin: &Path, chain_id: u64, port: u16, log: &Path) -> Result<Child> {
    let child = Command::new(bin)
        .args(["--hardfork", "osaka", "--chain-id", &chain_id.to_string(), "--port", &port.to_string(), "--silent"])
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(log)?)
        .spawn()?;
    let evm = Evm::new(&format!("http://127.0.0.1:{port}"));
    wait_for(&format!("anvil {chain_id}"), Duration::from_secs(30), || evm.chain_id().is_ok())?;
    Ok(child)
}

fn deploy_acme(a: &DemoArgs, rpc: &str, guardian: &str, out: &Path) -> Result<Value> {
    let st = Command::new(&a.forge_bin)
        .current_dir(a.root.join("contracts/evm"))
        .args(["script", "script/DeployAcme.s.sol", "--rpc-url", rpc, "--broadcast", "--unlocked", "--sender", ANVIL_SENDER, "--slow"])
        .env("GUARDIAN_ADDRESS", guardian)
        .env("SEED", "true")
        // A real transfer before the anchor (anvil_setBalance would rewrite the anchor block's state).
        .env("GUARDIAN_FUND_WEI", "100000000000000000")
        .env("DEPLOYMENTS_OUT", out)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if !st.status.success() {
        return Err(eyre!("forge script failed: {}", String::from_utf8_lossy(&st.stderr)));
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(out)?)?)
}

fn start_sandbox(a: &DemoArgs, home: &Path, log: &Path) -> Result<(Child, Signer)> {
    if home.exists() {
        std::fs::remove_dir_all(home)?;
    }
    let init = Command::new(&a.sandbox_bin).arg("--home").arg(home).arg("init").output()?;
    if !init.status.success() {
        return Err(eyre!("near-sandbox init failed: {}", String::from_utf8_lossy(&init.stderr)));
    }
    let child = Command::new(&a.sandbox_bin)
        .arg("--home")
        .arg(home)
        .arg("run")
        .stdout(std::fs::File::create(log)?)
        .stderr(std::fs::File::create(log.with_extension("err.log"))?)
        .spawn()?;
    let near = Near::new("sandbox", Some("http://127.0.0.1:3030".into()));
    wait_for("near-sandbox RPC", Duration::from_secs(60), || near.call("block", json!({"finality": "final"})).is_ok())?;
    let root = Signer::from_file(&home.join("validator_key.json"))?;
    Ok((child, root))
}

pub(crate) fn ctx_for(near: &Near, dep: &Deployments, account: &str, key: &Path, eth_rpc: &str, state: &Path) -> Ctx {
    Ctx {
        near: near.clone(),
        dep: dep.clone(),
        account: Some(account.to_string()),
        key_file: Some(key.to_path_buf()),
        eth_rpc: Some(eth_rpc.to_string()),
        state_dir: state.to_path_buf(),
    }
}

fn code_hash(evm: &Evm, addr: &str, block: u64) -> Result<B256> {
    let p = evm.call("eth_getProof", json!([addr, [], format!("{block:#x}")]))?;
    Ok(p["codeHash"].as_str().ok_or_else(|| eyre!("codeHash"))?.parse()?)
}

/// The demo's settlement terms: the Acme payouts, short windows so the flow fits in minutes.
fn demo_settlement(deadline: u64) -> Settlement {
    let t = |n: u128| (n * ONE_TUSD).to_string();
    Settlement {
        payouts: Payouts { critical: t(10_000), high: t(3_000), medium: t(1_000), low: "0".into(), impact_bps: 0, impact_cap: "0".into(), decimals: 6 },
        instant: InstantRule { cap: t(5_000), budget: t(10_000), window_secs: 3_600 },
        fees: Fees { protocol_bps: 0, release: "0".into() },
        bond: t(25),
        challenge_bond: t(100),
        windows: SWindows {
            reveal: 300,
            challenge: 30,
            resolve: 120,
            infra_grace: 300,
            run_timeout: 120,
            round_min: 60,
            rerun_window: 120,
            disclosure_max_delay: 600,
            deadline,
            exit_grace: 60,
        },
        slot_budgets: vec![],
        unevaluable: json!("refund"),
        gap: None,
        known: Known { max_count: 4, max_share_bps: 5_000 },
        pause: Pause { slots: vec![0], min_impact_usd_micro: "0".into() },
        terms_sha256: hex::encode(sha256(TERMS.as_bytes())),
    }
}

/// The demo program's manifest (v2): the Acme fixture and the demo's settlement terms.
fn demo_manifest(dep: &Value, evm: &Evm, anchor_n: u64, anchor_h: B256, project: &str, deadline: u64) -> Result<Manifest> {
    let a = |k: &str| -> Result<alloy_primitives::Address> { Ok(dep[k].as_str().ok_or_else(|| eyre!("deployment {k}"))?.parse()?) };
    let code: alloy_primitives::Bytes = dep["invariantsRuntimeCode"].as_str().unwrap_or_default().parse()?;
    let mut targets = Vec::new();
    for n in ["AcmeGuard", "acUSD", "acETH", "AcmePool", "AcmeLend", "AcmeVault", "AcmeTreasury"] {
        targets.push(Target { name: n.into(), address: a(n)?, code_hash: code_hash(evm, dep[n].as_str().unwrap(), anchor_n)? });
    }
    let inv_addr = alloy_primitives::Address::from_slice(&keccak256(b"antibody/invariants/v1")[12..]);
    Ok(Manifest {
        schema: MANIFEST_SCHEMA_V2.into(),
        program: ProgramInfo {
            name: "Acme Lend v1 — pre-launch (sandbox demo)".into(),
            project: project.into(),
            description: "Fictional lending protocol with planted bugs (DELIBERATELY VULNERABLE TEST FIXTURE).".into(),
        },
        anchor: Anchor { chain_id: 31337, block_number: anchor_n, block_hash: anchor_h, evm_spec: "osaka".into(), trust: "project-attested".into() },
        targets,
        invariants: Invariants {
            address: inv_addr,
            runtime_code_hash: keccak256(&code),
            runtime_code: code,
            gas_limit: 5_000_000,
            slots: vec![
                Slot { bit: 0, name: "solvency".into(), severity: Severity::Critical },
                Slot { bit: 1, name: "fees_to_treasury".into(), severity: Severity::High },
                Slot { bit: 2, name: "share_price".into(), severity: Severity::Medium },
            ],
        },
        impact: ImpactConfig {
            protocol_accounts: vec![a("AcmeLend")?, a("AcmeVault")?, a("AcmePool")?, a("AcmeTreasury")?],
            native_usd: "2000".into(),
            tokens: vec![
                PricedToken { symbol: "acUSD".into(), address: a("acUSD")?, decimals: 18, usd: "1".into() },
                PricedToken { symbol: "acETH".into(), address: a("acETH")?, decimals: 18, usd: "2000".into() },
            ],
        },
        capabilities: Capabilities {
            max_senders: 2,
            max_endowment_wei: (100u128 * 10u128.pow(18)).to_string(),
            max_txs: 4,
            max_gas_per_tx: 16_777_216,
            max_total_gas: 30_000_000,
            max_blocks: 1,
            max_timestamp_delta: 0,
            max_eval_gas: None,
            max_entry_calls: 256,
        },
        payout: None,
        rules: None,
        round: 0,
        overrides: Vec::new(),
        groups: vec![],
        protected_accounts: vec![],
        canaries: vec![],
        settlement: Some(serde_json::to_value(demo_settlement(deadline))?),
    })
}

/// Agent A submits through the MCP server, the way an AI agent would. `net` selects the NEAR
/// network; without `eth_rpc` the CLI reads `SEPOLIA_RPC` from `.env`, so the URL never appears in a
/// command line.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mcp_submit(
    antibody_bin: &Path,
    net: &[&str],
    deployments: &Path,
    account: &str,
    key: &Path,
    eth_rpc: Option<&str>,
    state: &Path,
    program: u32,
    bundle: &Path,
) -> Result<Value> {
    let mut command = Command::new(antibody_bin);
    command.args(net).arg("--deployments").arg(deployments).args(["--account", account, "--key-file"]).arg(key);
    if let Some(rpc) = eth_rpc {
        command.args(["--eth-rpc", rpc]);
    }
    let mut child = command
        .arg("mcp")
        .current_dir(state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut rpc = |msg: Value| -> Result<Value> {
        writeln!(stdin, "{msg}")?;
        stdin.flush()?;
        let mut line = String::new();
        out.read_line(&mut line)?;
        Ok(serde_json::from_str(&line)?)
    };
    rpc(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "agent-a", "version": "1"}}}))?;
    let tools = rpc(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))?;
    let n = tools["result"]["tools"].as_array().map(|t| t.len()).unwrap_or(0);
    let r = rpc(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "submit",
        "arguments": {"program_id": program, "bundle_path": bundle, "wait": true}}}))?;
    drop(stdin);
    let _ = child.wait();
    if r["result"]["isError"] == json!(true) {
        return Err(eyre!("MCP submit failed: {}", r["result"]["content"][0]["text"]));
    }
    let text = r["result"]["content"][0]["text"].as_str().unwrap_or("{}");
    Ok(json!({"mcp_tools": n, "result": serde_json::from_str::<Value>(text)?}))
}

/// Whether a verdict is above the program's instant cap: it is final only after an agreeing second
/// run and the challenge window (docs/spec.md §6.2).
fn above_cap(p: &Value, v: &Value) -> bool {
    let params = &p["params"];
    let cap = cmd::u128_of(&params["instant_cap"]);
    v["kind"] == json!("Violation")
        && v["keys"].as_array().into_iter().flatten().any(|k| {
            let sev = params["slots"].as_array().into_iter().flatten().find(|s| s["slot"] == k["slot"]).and_then(|s| s["severity"].as_str()).unwrap_or("low");
            cmd::u128_of(&params[sev]) > cap
        })
}

/// Waits until run `run_id` is T1 (the relayer thread relays its MOCK attestation).
fn wait_t1(ctx: &Ctx, run_id: u64) -> Result<Value> {
    let mut last = Value::Null;
    wait_for(&format!("T1 for run {run_id}"), Duration::from_secs(120), || {
        last = cmd::run(ctx, run_id).map(|r| cmd::run_view(&r)).unwrap_or(Value::Null);
        // An infrastructure error is never confirmed: stop at once with its reason.
        last["tier"] == json!("T1") || last.get("failed").is_some() || last["kind"] == json!("InfraError")
    })?;
    if last["tier"] != json!("T1") {
        return Err(eyre!("run {run_id} failed: {last}"));
    }
    Ok(last)
}

pub fn run(a: DemoArgs) -> Result<Value> {
    let t_all = Instant::now();
    std::fs::create_dir_all(&a.work)?;
    let mut procs = Procs(Vec::new());
    let mut results = json!({"started_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs()});

    step("EVM: anvil (chain 31337) and the Acme Lend fixture");
    procs.0.push(start_anvil(&a.anvil_bin, 31337, 8545, &a.work.join("anvil-31337.log"))?);
    let guardian_evm = eth_address(&demo_mpc_key());
    let dep_a = deploy_acme(&a, "http://127.0.0.1:8545", &guardian_evm, &a.work.join("acme-31337.json"))?;
    let evm_a = Evm::new("http://127.0.0.1:8545");
    let anchor = evm_a.call("eth_getBlockByNumber", json!(["latest", false]))?;
    let anchor_n = crate::evm::hex_u64(&anchor["number"])?;
    let anchor_h: B256 = anchor["hash"].as_str().unwrap_or_default().parse()?;
    // The anchor must never be anvil's mutable head: later blocks build on it, its state is fixed.
    evm_a.call("evm_mine", json!([]))?;

    step("NEAR: sandbox, accounts and contracts (tUSD, near-antibody, MOCK OutLayer, MOCK DCAP)");
    let (sandbox, root) = start_sandbox(&a, &a.work.join("near"), &a.work.join("sandbox.log"))?;
    procs.0.push(sandbox);
    let near = Near::new("sandbox", Some("http://127.0.0.1:3030".into()));
    let keys = a.work.join("keys");
    std::fs::create_dir_all(&keys)?;
    std::fs::write(keys.join("test.near.json"), serde_json::to_vec(&root.to_file_json())?)?;
    let names = ["tusd", "bounty", "outlayer", "dcap", "acme", "worker", "keeper", "relayer", "agent-a", "agent-b", "agent-c", "agent-d", "agent-e", "agent-f"];
    let acct = |n: &str| format!("{n}.test.near");
    let dep = Deployments {
        network: "sandbox".into(),
        antibody: acct("bounty"),
        tusd: acct("tusd"),
        guardian: None,
        outlayer: Some(acct("outlayer")),
        light_client: None,
        rpc: Some("http://127.0.0.1:3030".into()),
    };
    let dep_file = a.work.join("deployments.json");
    std::fs::write(&dep_file, serde_json::to_vec_pretty(&dep)?)?;
    let root_ctx = ctx_for(&near, &dep, "test.near", &keys.join("test.near.json"), "http://127.0.0.1:8545", &a.work.join("state/root"));
    for n in names {
        let amount = if n.starts_with("agent") || ["worker", "keeper", "relayer"].contains(&n) { "30" } else { "50" };
        cmd::admin_create_account(&root_ctx, &acct(n), amount, None, Some(&keys.join(format!("{}.json", acct(n)))))?;
    }
    let ctx_of = |n: &str| ctx_for(&near, &dep, &acct(n), &keys.join(format!("{}.json", acct(n))), "http://127.0.0.1:8545", &a.work.join(format!("state/{n}")));
    let rel = a.root.join("contracts/near-antibody/target/wasm32-unknown-unknown/release");
    let verifier_wasm = std::fs::read(a.root.join("target/wasm32-wasip2/release/antibody-verify.wasm")).unwrap_or_default();
    let verifier_hash = hex::encode(sha256(&verifier_wasm));
    cmd::admin_deploy(&ctx_of("tusd"), &rel.join("antibody_tusd.wasm"), Some("new".into()), Some(json!({"owner": "test.near"}).to_string()))?;
    cmd::admin_deploy(&ctx_of("outlayer"), &rel.join("antibody_mock_outlayer.wasm"), Some("new".into()),
        Some(json!({"worker": acct("worker"), "images": [mock_image()]}).to_string()))?;
    cmd::admin_deploy(&ctx_of("dcap"), &rel.join("antibody_mock_dcap.wasm"), Some("new".into()), Some("{}".into()))?;
    let defaults = json!({
        "verifier": {"wasm_url": "https://github.com/Himess/antibody/releases/download/sandbox/antibody-verify.wasm", "wasm_hash": verifier_hash,
                     "max_instructions": 5_000_000_000u64, "max_memory_mb": 512, "max_execution_seconds": 60, "deposit": "10000000000000000000000"},
        "outlayer": acct("outlayer"), "dcap_verifier": acct("dcap"), "accepted_tcb": ["UpToDate"], "image_delay_secs": 3_600,
        "light_client": null, "light_client_chain_id": 11155111, "guardian": null, "treasury": "test.near", "image_registry": acct("outlayer"),
    });
    cmd::admin_deploy(&ctx_of("bounty"), &rel.join("near_antibody.wasm"), Some("new".into()),
        Some(json!({"owner": "test.near", "token": acct("tusd"), "defaults": defaults}).to_string()))?;
    // Permissionless: near-antibody copies the (MOCK) approved worker images before any program exists.
    cmd::call(&ctx_of("keeper"), "sync_images", json!({}), 60)?;
    cmd::admin_call(&root_ctx, &acct("tusd"), "mint", &json!({"account_id": acct("acme"), "amount": (50_000 * ONE_TUSD).to_string()}).to_string(), 0, 30)?;
    for x in ["a", "b", "c", "d", "e", "f"] {
        cmd::admin_call(&root_ctx, &acct("tusd"), "mint", &json!({"account_id": acct(&format!("agent-{x}")), "amount": (200 * ONE_TUSD).to_string()}).to_string(), 0, 30)?;
    }
    results["deployment"] = json!({"near_antibody": acct("bounty"), "verifier_wasm_sha256": verifier_hash, "images": near.view(&acct("bounty"), "get_images", json!({}))?});

    step("Background: MOCK worker (the real verifier in TEE mode, TEST root key) and the T1 relayer");
    let stop = Arc::new(AtomicBool::new(false));
    let worker_key = Signer::from_file(&keys.join(format!("{}.json", acct("worker"))))?;
    let mut handles = Vec::new();
    {
        let (near, stop, signer, outlayer, seed) = (near.clone(), stop.clone(), worker_key.clone(), acct("outlayer"), demo_seed());
        handles.push(std::thread::spawn(move || {
            let mut w = MockWorker { near, outlayer, worker: signer, seed, done: Default::default() };
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = w.tick() {
                    eprintln!("mock worker: {e:#}");
                }
                std::thread::sleep(Duration::from_millis(700));
            }
        }));
    }
    {
        // Its own account: the keeper signs concurrently, and one access key cannot send two nonces at once.
        let (stop, ctx, state_dir) = (stop.clone(), ctx_of("relayer"), a.work.join("services"));
        let outlayer = acct("outlayer");
        handles.push(std::thread::spawn(move || {
            let r = T1Relayer { ctx, source: Source::Mock { outlayer }, state_dir, max_attempts: 5 };
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = r.tick() {
                    eprintln!("t1 relayer: {e:#}");
                }
                std::thread::sleep(Duration::from_millis(1_000));
            }
        }));
    }

    step(&format!("Program: manifest v2 committed on NEAR, escrow {ESCROW_TUSD} tUSD"));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let manifest = demo_manifest(&dep_a, &evm_a, anchor_n, anchor_h, &acct("acme"), now + 3_600)?;
    let manifest_path = a.work.join("manifest.json");
    std::fs::write(&manifest_path, manifest.canonical())?;
    let project_key = a.work.join("keys/acme.x25519.json");
    cmd::keygen_x25519(&project_key)?;
    let acme = ctx_of("acme");
    let created = cmd::program_create(&acme, &manifest_path, ESCROW_TUSD, &project_key, None)?;
    let pid = created["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no program id: {created}"))? as u32;
    println!("program {pid} created: {} (anchor {})", created["tx"]["tx"], created["anchor"]["anchor"]);
    results["program"] = json!({"program_id": pid, "manifest_sha256": hex::encode(sha256(&manifest.canonical())), "anchor_block": anchor_n,
                                "anchor_hash": format!("{anchor_h:#x}"), "create_tx": created["tx"]["tx"], "anchor": created["anchor"]});

    step("Activation: the base witness is pinned; the program's sealing key counts once T1-confirmed");
    let t = Instant::now();
    let act = cmd::program_activate(&acme, pid, true)?;
    let act_run = act["run_id"].as_u64().unwrap_or_default();
    wait_t1(&acme, act_run)?;
    let p = acme.program(pid)?;
    if p["sealing_pk"].is_null() {
        return Err(eyre!("activation run {act_run} is T1 but the program has no sealing key: {}", act["result"]));
    }
    println!("activated in {:.1}s: sealing key {}, base witness {} bytes", t.elapsed().as_secs_f64(), p["sealing_pk"], act["base"]["bytes"]);
    results["activation"] = json!({"run_id": act_run, "sealing_pk": p["sealing_pk"], "key_epoch": p["key_epoch"], "base": act["base"],
                                   "seconds": t.elapsed().as_secs_f64()});

    let fx = a.root.join("fixtures/acme-local/submissions");
    step("Known issue: declared before open, verified by the same TEE path, counts once T1-confirmed");
    let known = cmd::submit_known(&acme, pid, &fx.join("KNOWN.bundle.json"), None, None, true)?;
    let known_run = known["run_id"].as_u64().unwrap_or_default();
    wait_t1(&acme, known_run)?;
    let known_n = acme.program(pid)?["known"].as_array().map(|k| k.len()).unwrap_or(0);
    println!("known issues: {known_n}");
    cmd::program_open(&acme, pid)?;

    step("Agents: NEAR credit and tUSD credit; agent A gets a function-call key and submits through MCP");
    for x in ["a", "b", "c", "d", "e", "f"] {
        let c = ctx_of(&format!("agent-{x}"));
        cmd::storage_deposit(&c, "1")?;
        cmd::deposit(&c, "100")?;
    }
    let mut rows: Vec<(String, u64, f64)> = Vec::new();
    // B first: a High below the instant cap, final as soon as it is T1.
    let t = Instant::now();
    let b = cmd::submit(&ctx_of("agent-b"), pid, &fx.join("B.bundle.json"), None, None, true)?;
    let b_id = b["commit"]["commit_id"].as_u64().unwrap_or_default();
    wait_t1(&root_ctx, b["reveal"]["run_id"].as_u64().unwrap_or_default())?;
    let paid_b = cmd::keeper(&ctx_of("keeper"), pid)?;
    let b_secs = t.elapsed().as_secs_f64();
    println!("B: verdict T1 and paid {:.1}s after its commit: {}", b_secs, cmd::state_label(&root_ctx.commit(b_id)?["state"]));
    rows.push(("B".into(), b_id, b_secs));
    results["instant"] = json!({"commit": b_id, "seconds_commit_to_paid": b_secs, "keeper": paid_b});

    let agent_key = a.work.join("keys/agent-a.fc-key.json");
    cmd::agent_key_add(&ctx_of("agent-a"), 2.0, &agent_key)?;
    let state_a = a.work.join("state/agent-a-mcp");
    std::fs::create_dir_all(&state_a)?;
    let t = Instant::now();
    let sandbox_net = ["--network", "sandbox", "--rpc", "http://127.0.0.1:3030"];
    let mcp = mcp_submit(&a.antibody_bin, &sandbox_net, &dep_file, &acct("agent-a"), &agent_key, Some("http://127.0.0.1:8545"), &state_a, pid, &fx.join("A.bundle.json"))?;
    let a_commit = mcp["result"]["commit"]["commit_id"].as_u64().ok_or_else(|| eyre!("MCP submit: no commit id: {mcp}"))?;
    rows.push(("A (MCP)".into(), a_commit, t.elapsed().as_secs_f64()));
    results["mcp"] = json!({"tools": mcp["mcp_tools"], "commit_id": a_commit});

    step("Agents C–F (library/CLI path); D commits after A");
    for x in ["c", "d", "e", "f"] {
        let t = Instant::now();
        let report = if x == "c" { Some(fx.join("C.report.md")) } else { None };
        let r = cmd::submit(&ctx_of(&format!("agent-{x}")), pid, &fx.join(format!("{}.bundle.json", x.to_uppercase())), None, report.as_deref(), true)?;
        rows.push((x.to_uppercase(), r["commit"]["commit_id"].as_u64().unwrap_or(0), t.elapsed().as_secs_f64()));
    }
    for (_, id, _) in &rows {
        if let Some(r) = root_ctx.commit(*id)?["run"].as_u64() {
            wait_t1(&root_ctx, r)?;
        }
    }

    step("Above the instant cap: an independent second run (anyone may ask; the keeper does), then the challenge window");
    let program = root_ctx.program(pid)?;
    let mut seconds = Vec::new();
    for (name, id, _) in &rows {
        let c = root_ctx.commit(*id)?;
        let v = cmd::run_view(&cmd::run(&root_ctx, c["run"].as_u64().unwrap_or_default())?);
        if c["state"] == json!("Verifying") && above_cap(&program, &v) {
            let s = cmd::second(&ctx_of("keeper"), pid, *id, true)?;
            println!("second run for {name} (commit {id}): {}", s["result"]["kind"]);
            seconds.push(json!({"finding": name, "commit": id, "run": s["run_id"], "kind": s["result"]["kind"]}));
        }
    }
    results["second_runs"] = Value::Array(seconds);
    // The challenge window is measured in block time.
    near.call("sandbox_fast_forward", json!({"delta_height": 40}))?;

    step("Settlement: the keeper allocates in commit order and pays; B withdraws its credit");
    let mut last_keeper = Value::Null;
    wait_for("every claim settled", Duration::from_secs(180), || {
        last_keeper = cmd::keeper(&ctx_of("keeper"), pid).unwrap_or(Value::Null);
        rows.iter().all(|(_, id, _)| root_ctx.commit(*id).map(|c| cmd::state_label(&c["state"]) != "Verifying" && cmd::state_label(&c["state"]) != "Holder").unwrap_or(false))
    })?;
    let before = cmd::u128_of(&root_ctx.near.view(&acct("tusd"), "ft_balance_of", json!({"account_id": acct("agent-b")}))?);
    cmd::withdraw(&ctx_of("agent-b"), "3000")?;
    let after = cmd::u128_of(&root_ctx.near.view(&acct("tusd"), "ft_balance_of", json!({"account_id": acct("agent-b")}))?);
    results["keeper"] = last_keeper;
    results["withdrawal"] = json!({"account": acct("agent-b"), "wallet_before": cmd::fmt_tusd(before), "wallet_after": cmd::fmt_tusd(after)});
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.join();
    }

    step("Results");
    let expected: BTreeMap<&str, (&str, &str, u128)> = BTreeMap::from([
        ("A (MCP)", ("Violation", "Done:Paid", 10_000 * ONE_TUSD)),
        ("B", ("Violation", "Done:Paid", 3_000 * ONE_TUSD)),
        ("C", ("NoViolation", "Done:Forfeited", 0)),
        ("D", ("Violation", "Duplicate", 0)),
        ("E", ("Violation", "Done:Known", 0)),
        ("F", ("Violation", "Done:Paid", 10_000 * ONE_TUSD)),
    ]);
    let mut table = Vec::new();
    let mut ok = known_n == 1 && after == before + 3_000 * ONE_TUSD;
    println!("{:<8} {:>6} {:<12} {:<5} {:<34} {:<15} {:>10} {:>8}", "agent", "commit", "verdict", "tier", "keys (slot: tag)", "state", "paid tUSD", "seconds");
    for (agent, id, secs) in &rows {
        let c = root_ctx.commit(*id)?;
        let v = cmd::run_view(&cmd::run(&root_ctx, c["run"].as_u64().unwrap_or_default())?);
        let kind = v["kind"].as_str().unwrap_or("-").to_string();
        let state = cmd::state_label(&c["state"]);
        let paid = cmd::paid_of(&c);
        let keys: Vec<String> =
            v["keys"].as_array().into_iter().flatten().map(|k| format!("{}: {}", k["slot"], &k["tag"].as_str().unwrap_or_default()[..10])).collect();
        let (ek, es, ep) = expected[agent.as_str()];
        let pass = kind == ek && state == es && paid == ep && v["tier"] == json!("T1");
        ok &= pass;
        println!("{:<8} {:>6} {:<12} {:<5} {:<34} {:<15} {:>10} {:>8.1} {}", agent, id, kind, v["tier"].as_str().unwrap_or("-"), keys.join(", "),
                 state, paid / ONE_TUSD, secs, if pass { "OK" } else { "MISMATCH" });
        table.push(json!({"agent": agent, "commit": id, "verdict": kind, "tier": v["tier"], "keys": v["keys"], "state": c["state"],
                          "paid_raw": paid.to_string(), "seconds": secs, "ok": pass}));
    }
    let audit = root_ctx.view("audit", json!({"program_id": pid}))?;
    let contract_audit = root_ctx.view("audit_contract", json!({}))?;
    ok &= audit["ok"] == json!(true) && contract_audit["ok"] == json!(true);
    println!("known issues: {known_n}; audit(program): {}; audit(contract): {}; B withdrew 3000 tUSD: {}",
             audit["ok"], contract_audit["ok"], after == before + 3_000 * ONE_TUSD);
    results["submissions"] = Value::Array(table);
    results["audit"] = json!({"program": audit, "contract": contract_audit});
    results["record"] = root_ctx.view("program_record", json!({"program_id": pid}))?;
    results["all_ok"] = ok.into();
    results["seconds"] = t_all.elapsed().as_secs_f64().into();
    std::fs::write(a.work.join("results.json"), serde_json::to_vec_pretty(&results)?)?;
    if a.keep {
        println!("--keep: sandbox and anvil stay up; press Ctrl-C to stop");
        std::thread::park();
    }
    drop(procs);
    if !ok {
        return Err(eyre!("demo finished with mismatches (see {})", a.work.join("results.json").display()));
    }
    println!("\nALL OK in {:.0}s — results: {}", t_all.elapsed().as_secs_f64(), a.work.join("results.json").display());
    Ok(results)
}

//! `antibody-services demo`: the offline, keyless end-to-end run (and integration test).
//!
//! near-sandbox + two anvils ("verification chain" 31337 and "pause-only chain" 31338), the real
//! near-antibody / near-guardian / tUSD contracts, MOCK OutLayer and MOCK MPC contracts resolved by
//! the real verifier (TEE mode, TEST seed) and a TEST secp256k1 key. Flow:
//! deploy → program (manifest committed) → known issue (t0) → open → agents A–F (A through
//! `antibody mcp` with a function-call agent key) → guardian pause on both chains → finalize /
//! release → disclosure → `verify` of D's duplicate proof. Every expected outcome is asserted.

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

use alloy_primitives::{keccak256, Address, Bytes, B256};
use antibody_cli::{
    cmd,
    ctx::{Ctx, Deployments},
    near::{Near, Signer},
};
use antibody_verifier::{
    facts::sha256,
    manifest::{
        Anchor, Capabilities, ImpactConfig, Invariants, Manifest, Payout, PricedToken, ProgramInfo, Rules, Severity, Slot, Target,
        MANIFEST_SCHEMA,
    },
    sealed,
};
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{
    evm::Evm,
    mocks::{demo_mpc_key, demo_seed, eth_address, MockSigner, MockWorker},
    relayer::Relayer,
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

fn step(name: &str) {
    println!("\n== {name}");
}

fn wait_for<F: FnMut() -> bool>(what: &str, timeout: Duration, mut f: F) -> Result<()> {
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
const ONE_TUSD: u128 = 1_000_000;

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
        // 0.1 ETH so the guardian address pays for its own pause(); a real transfer before the
        // anchor (anvil_setBalance would rewrite the anchor block's state in place).
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

fn ctx_for(near: &Near, dep: &Deployments, account: &str, key: &Path, eth_rpc: &str, state: &Path) -> Ctx {
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

/// The demo program's manifest: the Acme fixture with short windows so the whole flow fits minutes.
fn demo_manifest(dep: &Value, evm: &Evm, anchor_n: u64, anchor_h: B256, project: &str, token: &str, deadline: u64) -> Result<Manifest> {
    let a = |k: &str| -> Result<alloy_primitives::Address> { Ok(dep[k].as_str().ok_or_else(|| eyre!("deployment {k}"))?.parse()?) };
    let code: alloy_primitives::Bytes = dep["invariantsRuntimeCode"].as_str().unwrap_or_default().parse()?;
    let mut targets = Vec::new();
    for n in ["AcmeGuard", "acUSD", "acETH", "AcmePool", "AcmeLend", "AcmeVault", "AcmeTreasury"] {
        targets.push(Target { name: n.into(), address: a(n)?, code_hash: code_hash(evm, dep[n].as_str().unwrap(), anchor_n)? });
    }
    let inv_addr = alloy_primitives::Address::from_slice(&keccak256(b"antibody/invariants/v1")[12..]);
    Ok(Manifest {
        schema: MANIFEST_SCHEMA.into(),
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
            max_entry_calls: 256,
        },
        payout: Payout {
            token: token.into(),
            decimals: 6,
            critical: (10_000 * ONE_TUSD).to_string(),
            high: (3_000 * ONE_TUSD).to_string(),
            medium: (1_000 * ONE_TUSD).to_string(),
            low: "0".into(),
            impact_bps: 0,
            impact_cap: "0".into(),
        },
        rules: Rules {
            bond: (25 * ONE_TUSD).to_string(),
            reveal_window_blocks: 15,
            disclosure_after_secs: 5,
            deadline_unix: deadline,
            instant_payout_cap: (5_000 * ONE_TUSD).to_string(),
            challenge_delay_secs: 8,
        },
        round: 0,
        overrides: Vec::new(),
    })
}

/// Agent A submits through the MCP server, the way an AI agent would.
#[allow(clippy::too_many_arguments)]
fn mcp_submit(antibody_bin: &Path, deployments: &Path, account: &str, key: &Path, eth_rpc: &str, state: &Path, program: u32, bundle: &Path) -> Result<Value> {
    let mut child = Command::new(antibody_bin)
        .args(["--network", "sandbox", "--rpc", "http://127.0.0.1:3030", "--deployments"])
        .arg(deployments)
        .args(["--account", account, "--key-file"])
        .arg(key)
        .args(["--eth-rpc", eth_rpc, "mcp"])
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

fn outcome_label(o: &Value) -> String {
    match o {
        Value::String(s) => s.clone(),
        Value::Object(m) => m.keys().next().cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn run(a: DemoArgs) -> Result<Value> {
    let t_all = Instant::now();
    std::fs::create_dir_all(&a.work)?;
    let mut procs = Procs(Vec::new());
    let mut results = json!({"started_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs()});

    step("EVM: two anvils (verification chain 31337, pause-only chain 31338) and the Acme Lend fixture");
    procs.0.push(start_anvil(&a.anvil_bin, 31337, 8545, &a.work.join("anvil-31337.log"))?);
    procs.0.push(start_anvil(&a.anvil_bin, 31338, 8546, &a.work.join("anvil-31338.log"))?);
    let mpc_key = demo_mpc_key();
    let guardian_evm = eth_address(&mpc_key);
    let dep_a = deploy_acme(&a, "http://127.0.0.1:8545", &guardian_evm, &a.work.join("acme-31337.json"))?;
    let dep_b = deploy_acme(&a, "http://127.0.0.1:8546", &guardian_evm, &a.work.join("acme-31338.json"))?;
    let same = ["AcmeGuard", "AcmeLend", "AcmeVault", "codehashAcmeLend", "codehashAcmeVault", "codehashAcmeGuard"].iter().all(|k| dep_a[k] == dep_b[k]);
    println!("same addresses and code hashes on both chains: {same}");
    results["evm"] = json!({"guardian_evm_address": guardian_evm, "acme_guard": dep_a["AcmeGuard"], "same_bytecode_both_chains": same});
    let evm_a = Evm::new("http://127.0.0.1:8545");
    let anchor = evm_a.call("eth_getBlockByNumber", json!(["latest", false]))?;
    let anchor_n = crate::evm::hex_u64(&anchor["number"])?;
    let anchor_h: B256 = anchor["hash"].as_str().unwrap_or_default().parse()?;
    // The anchor must never be anvil's mutable head: later blocks build on it, its state is fixed.
    evm_a.call("evm_mine", json!([]))?;

    step("NEAR: sandbox, accounts and contracts (tUSD, near-antibody, near-guardian, MOCK OutLayer, MOCK MPC)");
    let (sandbox, root) = start_sandbox(&a, &a.work.join("near"), &a.work.join("sandbox.log"))?;
    procs.0.push(sandbox);
    let near = Near::new("sandbox", Some("http://127.0.0.1:3030".into()));
    let keys = a.work.join("keys");
    std::fs::create_dir_all(&keys)?;
    std::fs::write(keys.join("test.near.json"), serde_json::to_vec(&root.to_file_json())?)?;
    let names = ["tusd", "bounty", "guardian", "outlayer", "mpc", "acme", "worker", "agent-a", "agent-b", "agent-c", "agent-d", "agent-e", "agent-f"];
    let acct = |n: &str| format!("{n}.test.near");
    let dep = Deployments {
        network: "sandbox".into(),
        antibody: acct("bounty"),
        tusd: acct("tusd"),
        guardian: Some(acct("guardian")),
        outlayer: Some(acct("outlayer")),
        light_client: None,
        rpc: Some("http://127.0.0.1:3030".into()),
    };
    let dep_file = a.work.join("deployments.json");
    std::fs::write(&dep_file, serde_json::to_vec_pretty(&dep)?)?;
    let root_ctx = ctx_for(&near, &dep, "test.near", &keys.join("test.near.json"), "http://127.0.0.1:8545", &a.work.join("state/root"));
    for n in names {
        let amount = if n.starts_with("agent") || n == "worker" { "30" } else { "50" };
        cmd::admin_create_account(&root_ctx, &acct(n), amount, None, Some(&keys.join(format!("{}.json", acct(n)))))?;
    }
    let ctx_of = |n: &str| ctx_for(&near, &dep, &acct(n), &keys.join(format!("{}.json", acct(n))), "http://127.0.0.1:8545", &a.work.join(format!("state/{n}")));
    let wasm = |p: &str| a.root.join(p);
    let rel = "contracts/near-antibody/target/wasm32-unknown-unknown/release";
    let verifier_wasm = std::fs::read(a.root.join("target/wasm32-wasip2/release/antibody-verify.wasm")).unwrap_or_default();
    let verifier_hash = hex::encode(sha256(&verifier_wasm));
    let seed = demo_seed();
    let pubkey = hex::encode(sealed::verifier_public(&seed));
    cmd::admin_deploy(&ctx_of("tusd"), &wasm(&format!("{rel}/antibody_tusd.wasm")), Some("new".into()), Some(json!({"owner": "test.near"}).to_string()))?;
    cmd::admin_deploy(&ctx_of("outlayer"), &wasm(&format!("{rel}/antibody_mock_outlayer.wasm")), Some("new".into()), Some(json!({"worker": acct("worker")}).to_string()))?;
    cmd::admin_deploy(&ctx_of("mpc"), &wasm(&format!("{rel}/antibody_mock_mpc.wasm")), Some("new".into()), Some(json!({"signer": acct("worker")}).to_string()))?;
    let verifier = json!({"wasm_url": "https://github.com/Himess/antibody/releases/download/sandbox/antibody-verify.wasm",
        "wasm_hash": verifier_hash, "pubkey": pubkey, "secrets_owner": null, "secrets_profile": null,
        "max_instructions": 5_000_000_000u64, "max_memory_mb": 512, "max_execution_seconds": 60,
        "deposit": "10000000000000000000000", "tier": "MOCK (sandbox: mock OutLayer, TEST seed)"});
    cmd::admin_deploy(&ctx_of("bounty"), &wasm(&format!("{rel}/near_antibody.wasm")), Some("new".into()),
        Some(json!({"owner": "test.near", "token": acct("tusd"), "outlayer": acct("outlayer"), "light_client": null, "verifier": verifier}).to_string()))?;
    cmd::admin_deploy(&ctx_of("guardian"), &wasm("contracts/near-guardian/target/wasm32-unknown-unknown/release/antibody_guardian.wasm"), Some("new".into()),
        Some(json!({"owner": "test.near", "mpc": acct("mpc"), "path": "guardian/acme", "guardian_address": guardian_evm}).to_string()))?;
    for (chain, name) in [(31337u64, "verification-chain"), (31338u64, "pause-only-chain")] {
        let target = json!({"target": {"chain_id": chain, "name": name, "to": dep_a["AcmeGuard"], "calldata": "0x8456cb59", "gas_limit": 80_000,
            "max_fee_per_gas": "30000000000", "max_priority_fee_per_gas": "2000000000", "next_nonce": 0, "explorer": format!("local-{chain}:")}});
        cmd::admin_call(&root_ctx, &acct("guardian"), "set_chain", &target.to_string(), 0, 30)?;
    }
    cmd::admin_call(&root_ctx, &acct("guardian"), "set_authorized", &json!({"account_id": acct("bounty"), "enabled": true}).to_string(), 0, 30)?;
    cmd::admin_call(&root_ctx, &acct("tusd"), "mint", &json!({"account_id": acct("acme"), "amount": (30_000 * ONE_TUSD).to_string()}).to_string(), 0, 30)?;
    for x in ["a", "b", "c", "d", "e", "f"] {
        cmd::admin_call(&root_ctx, &acct("tusd"), "mint", &json!({"account_id": acct(&format!("agent-{x}")), "amount": (200 * ONE_TUSD).to_string()}).to_string(), 0, 30)?;
    }

    step("Background: MOCK worker (verifier in TEE mode), MOCK MPC signer, relayer");
    let stop = Arc::new(AtomicBool::new(false));
    let worker_key = Signer::from_file(&keys.join(format!("{}.json", acct("worker"))))?;
    let mut handles = Vec::new();
    {
        let (near, stop, signer, outlayer) = (near.clone(), stop.clone(), worker_key.clone(), acct("outlayer"));
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
        let (near, stop, signer, mpc) = (near.clone(), stop.clone(), worker_key.clone(), acct("mpc"));
        handles.push(std::thread::spawn(move || {
            let mut s = MockSigner { near, mpc, signer, key: demo_mpc_key(), done: Default::default() };
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = s.tick() {
                    eprintln!("mock signer: {e:#}");
                }
                std::thread::sleep(Duration::from_millis(700));
            }
        }));
    }
    let relayer = Relayer {
        near: near.clone(),
        guardian: acct("guardian"),
        chains: BTreeMap::from([
            (31337, ("http://127.0.0.1:8545".to_string(), "local-31337:".to_string())),
            (31338, ("http://127.0.0.1:8546".to_string(), "local-31338:".to_string())),
        ]),
        state_file: a.work.join("services/relayer.json"),
    };

    step("Program: manifest committed on NEAR, escrow 25,000 tUSD");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let deadline = now + 360;
    let manifest = demo_manifest(&dep_a, &evm_a, anchor_n, anchor_h, &acct("acme"), &acct("tusd"), deadline)?;
    let manifest_path = a.work.join("manifest.json");
    std::fs::write(&manifest_path, manifest.canonical())?;
    let project_key = a.work.join("keys/acme.x25519.json");
    cmd::keygen_x25519(&project_key)?;
    let acme = ctx_of("acme");
    let created = cmd::program_create(&acme, &manifest_path, "25000", &project_key, Some(acct("guardian")), None)?;
    let pid = created["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no program_created event: {created}"))? as u32;
    println!("program {pid} created: {}", created["tx"]["tx"]);
    results["program"] = json!({"manifest_sha256": hex::encode(sha256(&manifest.canonical())), "anchor_block": anchor_n,
                                "anchor_hash": format!("{anchor_h:#x}"), "create_tx": created["tx"]["tx"], "program_id": pid});

    let fx = a.root.join("fixtures/acme-local/submissions");
    step("t0: the project declares its known issue (verified by the same TEE path)");
    let t = Instant::now();
    let known = cmd::submit_known(&acme, pid, &fx.join("KNOWN.bundle.json"), None, None, true)?;
    let mut rows = vec![("KNOWN".to_string(), known["commit_id"].as_u64().unwrap_or(0), t.elapsed().as_secs_f64())];
    cmd::program_open(&acme, pid)?;

    step("Agents: deposit bonds; agent A gets a function-call key and submits through MCP");
    for x in ["a", "b", "c", "d", "e", "f"] {
        cmd::deposit(&ctx_of(&format!("agent-{x}")), "100")?;
    }
    let agent_key = a.work.join("keys/agent-a.fc-key.json");
    cmd::agent_key_add(&ctx_of("agent-a"), 2.0, &agent_key)?;
    let state_a = a.work.join("state/agent-a-mcp");
    std::fs::create_dir_all(&state_a)?;
    let t = Instant::now();
    let mcp = mcp_submit(&a.antibody_bin, &dep_file, &acct("agent-a"), &agent_key, "http://127.0.0.1:8545", &state_a, pid, &fx.join("A.bundle.json"))?;
    let a_commit = mcp["result"]["commit"]["commit_id"].as_u64().ok_or_else(|| eyre!("MCP submit: no commit id: {mcp}"))?;
    rows.push(("A (MCP)".into(), a_commit, t.elapsed().as_secs_f64()));
    results["mcp"] = json!({"tools": mcp["mcp_tools"], "commit_id": a_commit});

    step("Guardian: the verified Critical makes near-guardian sign pause() for both chains; the relayer broadcasts");
    let t_pause = Instant::now();
    wait_for("pause() on both chains", Duration::from_secs(120), || {
        relayer.tick().map(|st| st.values().filter(|v| v["paused"] == json!(true) && v["status"] == "confirmed").count() == 2).unwrap_or(false)
    })?;
    let relay_state = relayer.load_state();
    println!("paused on both chains {:.1}s after A's verdict", t_pause.elapsed().as_secs_f64());
    results["guardian"] = json!({"pause_seconds_after_verdict": t_pause.elapsed().as_secs_f64(), "chains": relay_state});

    step("Agents B–F (library/CLI path); D commits after A");
    for x in ["b", "c", "d", "e", "f"] {
        let t = Instant::now();
        let report = if x == "c" { Some(fx.join("C.report.md")) } else { None };
        let r = cmd::submit(&ctx_of(&format!("agent-{x}")), pid, &fx.join(format!("{}.bundle.json", x.to_uppercase())), None, report.as_deref(), true)?;
        rows.push((x.to_uppercase(), r["commit"]["commit_id"].as_u64().unwrap_or(0), t.elapsed().as_secs_f64()));
    }

    step("Settlement: pass the reveal windows, finalize, release T0-delayed payouts");
    near.call("sandbox_fast_forward", json!({"delta_height": 40}))?;
    std::thread::sleep(Duration::from_secs(2));
    for (_, id, _) in &rows[1..] {
        let c = root_ctx.commit(*id)?;
        if c["outcome"] == json!("Pending") {
            cmd::program_simple(&root_ctx, "finalize", json!({"commit_id": id}), 60)?;
        }
    }
    for (_, id, _) in &rows[1..] {
        if root_ctx.commit(*id)?["outcome"].get("Delayed").is_some() {
            // The challenge window is measured in block time; retry until it has passed.
            wait_for("the challenge window", Duration::from_secs(90), || {
                cmd::program_simple(&root_ctx, "release", json!({"commit_id": id}), 60).is_ok()
            })?;
        }
    }

    step("Round 1: the project ships the fix for A's bug as a code override at the same anchor; the TEE re-runs the claimed findings");
    let fixed = dep_a["fixedLendRuntimeCode"].as_str().ok_or_else(|| eyre!("deployment has no fixedLendRuntimeCode"))?;
    let lend: Address = dep_a["AcmeLend"].as_str().unwrap_or_default().parse()?;
    let r1 = manifest
        .with_overrides(1, &[(lend, Bytes::from(hex::decode(fixed.trim_start_matches("0x"))?))])
        .map_err(|e| eyre!(e))?;
    let r1_path = a.work.join("round1.json");
    std::fs::write(&r1_path, r1.canonical())?;
    let proposed = cmd::round_propose(&acme, pid, &r1_path)?;
    let mut reruns = Vec::new();
    let mut rerun_ok = true;
    for (name, want) in [("KNOWN", "Violation"), ("A (MCP)", "NoViolation"), ("B", "Violation"), ("F", "Violation")] {
        let id = rows.iter().find(|r| r.0 == name).map(|r| r.1).unwrap();
        let t = Instant::now();
        let r = cmd::round_rerun(&acme, id, Some(&project_key), true)?;
        let kind = r["result"]["kind"].as_str().unwrap_or("-").to_string();
        let pass = r["result"]["status"] == json!("Done") && kind == want;
        rerun_ok &= pass;
        println!("re-run {name:<8} commit {id}: {kind:<12} at the fix ({:.1}s) {}", t.elapsed().as_secs_f64(), if pass { "OK" } else { "MISMATCH" });
        reruns.push(json!({"finding": name, "commit": id, "kind": kind, "still_broken": r["result"]["still_broken"],
                           "receipt_hash": r["result"]["receipt_hash"], "seconds": t.elapsed().as_secs_f64(), "ok": pass}));
    }
    let opened = cmd::round_open(&acme, pid)?;
    let (carried, released) = (opened["round"]["carried"].as_u64(), opened["round"]["released"].as_u64());
    rerun_ok &= carried == Some(3) && released == Some(1);
    println!("round 1 open: {} keys carried (known issue, B, F), {} released (A's: AcmeLend.borrow can be found again)",
             carried.unwrap_or(0), released.unwrap_or(0));
    results["round1"] = json!({"manifest_sha256": proposed["manifest_sha256"], "propose_tx": proposed["tx"]["tx"], "reruns": reruns,
                               "carried": carried, "released": released, "keys": opened["keys"], "ok": rerun_ok});

    step("Disclosure: after deadline + delay, one TEE run publishes every content key and k_program");
    // Disclosure opens at deadline + disclosure_after in block time; retry until the contract agrees.
    // 300 Tgas: OutLayer (100) and the callback (20) are forwarded; the rest reads every commit.
    let last_err = std::cell::RefCell::new(String::new());
    wait_for("the disclosure time", Duration::from_secs(420), || match cmd::program_simple(&root_ctx, "disclose", json!({"program_id": pid}), 300) {
        Ok(_) => true,
        Err(e) => {
            *last_err.borrow_mut() = e.to_string();
            false
        }
    })
    .map_err(|e| eyre!("{e} (last error: {})", last_err.borrow()))?;
    wait_for("the disclosure result", Duration::from_secs(60), || {
        root_ctx.program(pid).map(|p| !p["disclosure"].is_null()).unwrap_or(false)
    })?;
    let d_commit = rows.iter().find(|r| r.0 == "D").map(|r| r.1).unwrap();
    let verified = cmd::verify(&root_ctx, d_commit)?;
    println!("verify D: same rule, same function = {}, committed earlier = {}", verified["same_rule_same_function"], verified["committed_earlier"]);
    results["duplicate_verification"] = verified.clone();
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.join();
    }

    step("Results");
    let expected: BTreeMap<&str, (&str, &str, u128)> = BTreeMap::from([
        ("KNOWN", ("Violation", "KnownIssue", 0)),
        ("A (MCP)", ("Violation", "Paid", 10_000 * ONE_TUSD)),
        ("B", ("Violation", "Paid", 3_000 * ONE_TUSD)),
        ("C", ("NoViolation", "Rejected", 0)),
        ("D", ("Violation", "Duplicate", 0)),
        ("E", ("Violation", "Known", 0)),
        ("F", ("Violation", "Paid", 10_000 * ONE_TUSD)),
    ]);
    let mut table = Vec::new();
    let mut ok = true;
    println!("{:<8} {:>6} {:<12} {:<44} {:<11} {:>12} {:>8}", "agent", "commit", "verdict", "key (invariant @ function)", "outcome", "paid tUSD", "seconds");
    for (agent, id, secs) in &rows {
        let c = root_ctx.commit(*id)?;
        let kind = c["verdict"]["kind"].as_str().unwrap_or("-").to_string();
        let out = outcome_label(&c["outcome"]);
        let paid = c["outcome"]["Paid"]["amount"].as_str().and_then(|s| s.parse::<u128>().ok()).unwrap_or(0);
        let keys: Vec<String> = c["verdict"]["keys"].as_array().into_iter().flatten()
            .map(|k| format!("slot {} {} {}", k["slot"], k["severity"].as_str().unwrap_or_default(), &k["tag"].as_str().unwrap_or_default()[..10])).collect();
        let (ek, eo, ep) = expected[agent.as_str()];
        let pass = kind == ek && out == eo && paid == ep;
        ok &= pass;
        println!("{:<8} {:>6} {:<12} {:<44} {:<11} {:>12} {:>8.1} {}", agent, id, kind, keys.join(", "), out, paid / ONE_TUSD, secs, if pass { "OK" } else { "MISMATCH" });
        table.push(json!({"agent": agent, "commit": id, "verdict": kind, "keys": c["verdict"]["keys"], "outcome": c["outcome"], "paid_raw": paid.to_string(), "seconds": secs, "ok": pass}));
    }
    let paused_both = relay_state.values().filter(|v| v["paused"] == json!(true)).count() == 2;
    let dup_ok = verified["same_rule_same_function"] == json!(true) && verified["committed_earlier"] == json!(true)
        && verified["commit"]["receipt_hash_matches_chain"] == json!(true) && verified["earlier"]["receipt_hash_matches_chain"] == json!(true);
    ok &= paused_both && dup_ok && same && rerun_ok;
    println!("guardian paused both chains: {paused_both}; duplicate proof re-verified after disclosure: {dup_ok}; round 1 as expected: {rerun_ok}");
    results["submissions"] = Value::Array(table);
    results["all_ok"] = ok.into();
    results["seconds"] = t_all.elapsed().as_secs_f64().into();
    std::fs::write(a.work.join("results.json"), serde_json::to_vec_pretty(&results)?)?;
    if a.keep {
        println!("--keep: sandbox and anvils stay up; press Ctrl-C to stop");
        std::thread::park();
    }
    drop(procs);
    if !ok {
        return Err(eyre!("demo finished with mismatches (see {})", a.work.join("results.json").display()));
    }
    println!("\nALL OK in {:.0}s — results: {}", t_all.elapsed().as_secs_f64(), a.work.join("results.json").display());
    Ok(results)
}

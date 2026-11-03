//! `antibody-services demo`: the offline, keyless end-to-end run (and integration test).
//!
//! near-sandbox and two anvil chains (31337 verifies and pauses, 31338 only pauses) with the Acme Lend
//! fixture; the real near-antibody, near-guardian and tUSD contracts and AntibodyAdapter on both chains;
//! MOCK OutLayer, DCAP, MPC and light-client contracts. The mock worker runs the real verifier (TEE
//! mode, TEST root key) and records a MOCK attestation per run; the T1 relayer relays it and
//! near-antibody runs its real T1 checks on it; the mock signer signs the guardian's permits with one
//! TEST key; the relayer submits them to the adapters. Flow: program (manifest v2, anchor confirmed)
//! → adapters and guardian registration → activation (T1) → known issue (T1) → open → agent A over MCP:
//! T1, paid in seconds, pause permits on both chains ("the one moment") → agents B–F → the keeper
//! allocates in commit order and pays → a withdrawal → rounds and releases → the practice program (a
//! tutorial): the vending contract, and two judges with no account who try it → audit(). Every
//! expected outcome is asserted.

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
    near::{Near, Signer, ONE_NEAR},
    practice,
};
use antibody_rules::settlement::{Fees, Gap, Instant as InstantRule, Known, Pause, Payouts, SWindows, Settlement};
use antibody_verifier::{
    facts::sha256,
    manifest::{
        Anchor, Canary, Capabilities, ImpactConfig, Invariants, Manifest, PricedToken, ProgramInfo, ProtectedAccount, Severity, Slot, Target,
        MANIFEST_SCHEMA_V2,
    },
};
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{
    evm::Evm,
    evm::EvmSigner,
    mocks::{demo_mpc_key, demo_seed, eth_address, mock_image, MockSigner, MockWorker},
    relayer::Relayer,
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
    pub serve: bool,
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
/// anvil's well-known TEST account 2: deploys the practice vaults, so the Acme deployer's nonces stay
/// the same on both chains.
const ANVIL_SENDER_2: &str = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";
/// anvil's well-known TEST account 1: the relayer's EVM key in the sandbox.
const ANVIL_KEY_1: &str = "59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
pub(crate) const ONE_TUSD: u128 = 1_000_000;
const ESCROW_TUSD: &str = "40000";
/// The sandbox's terms text; its sha256 is committed in the manifest and named by every commit.
const TERMS: &str = "Acme Lend (sandbox demo) terms: a DELIBERATELY VULNERABLE TEST FIXTURE. Payouts in tUSD, a testnet token with no value.";
const PRACTICE_TERMS: &str =
    "Practice program (sandbox demo) terms: a TUTORIAL TARGET whose bug and proof are public. Payouts in tUSD, a testnet token with no value.";
const PRACTICE_ESCROW_TUSD: &str = "30";

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

/// The practice program's tutorial vaults (docs/practice.md): 24, a thousandth of an ether each.
fn deploy_practice(a: &DemoArgs, rpc: &str, out: &Path) -> Result<Value> {
    let st = Command::new(&a.forge_bin)
        .current_dir(a.root.join("contracts/evm"))
        .args(["script", "script/DeployPractice.s.sol", "--rpc-url", rpc, "--broadcast", "--unlocked", "--sender", ANVIL_SENDER_2, "--slow"])
        .env_remove("DEPLOYER_KEY")
        .env("COUNT", "24")
        .env("VALUE_WEI", "24000000000000000")
        .env("PRACTICE_OUT", out)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if !st.status.success() {
        return Err(eyre!("forge script failed: {}", String::from_utf8_lossy(&st.stderr)));
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(out)?)?)
}

/// The program's adapter on one chain (CREATE2: the same address everywhere), made Acme's pauser and
/// the AcmeLend proxy's admin.
fn deploy_adapter(a: &DemoArgs, rpc: &str, guardian: &str, program_key: &str, acme: &Path, out: &Path) -> Result<Value> {
    let st = Command::new(&a.forge_bin)
        .current_dir(a.root.join("contracts/evm"))
        .args(["script", "script/DeployAdapter.s.sol", "--rpc-url", rpc, "--broadcast", "--unlocked", "--sender", ANVIL_SENDER, "--slow"])
        .env("GUARDIAN_ADDRESS", guardian)
        .env("PROGRAM_KEY", program_key)
        .env("ACME", acme)
        .env("ADAPTER_OUT", out)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if !st.status.success() {
        return Err(eyre!("forge script DeployAdapter failed: {}", String::from_utf8_lossy(&st.stderr)));
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(out)?)?)
}

/// The TEST key's public key as the MPC contract formats derived keys: `secp256k1:<base58 of x ‖ y>`.
fn mpc_public_key() -> String {
    let pt = demo_mpc_key().verifying_key().to_encoded_point(false);
    format!("secp256k1:{}", bs58::encode(&pt.as_bytes()[1..]).into_string())
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
        // A Critical is paid in seconds when T1; the budget holds A and B, so F waits its window.
        instant: InstantRule { cap: t(10_000), budget: t(15_000), window_secs: 3_600 },
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
        gap: Some(Gap { threshold_usd_micro: "100000000".into(), claim_amount: t(1_000) }),
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
    for n in ["AcmeGuard", "acUSD", "acETH", "AcmePool", "AcmeLend", "AcmeLendImpl", "AcmeVault", "AcmeVaultImpl", "AcmeTreasury"] {
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
        protected_accounts: vec![ProtectedAccount { address: a("saver")?, kind: "user".into() }],
        // Liveness: a "fix" that reverts everything breaks no invariant, so these calls must still succeed.
        canaries: vec![
            Canary { to: a("AcmeLend")?, data: keccak256(b"collateralPrice()")[..4].to_vec().into() },
            Canary { to: a("AcmeVault")?, data: keccak256(b"totalAssets()")[..4].to_vec().into() },
        ],
        settlement: Some(serde_json::to_value(demo_settlement(deadline))?),
        deploy: None,
        rule_strength: None,
    })
}

/// The practice program's terms: one Low slot paid 1 tUSD under the instant cap, a 0.1 tUSD bond, no
/// pause, no gap lane, no known issues.
fn practice_settlement(deadline: u64) -> Settlement {
    let t = |n: u128| (n * ONE_TUSD).to_string();
    Settlement {
        payouts: Payouts { critical: "0".into(), high: "0".into(), medium: "0".into(), low: t(1), impact_bps: 0, impact_cap: "0".into(), decimals: 6 },
        instant: InstantRule { cap: t(1), budget: t(24), window_secs: 3_600 },
        fees: Fees { protocol_bps: 0, release: "0".into() },
        bond: (ONE_TUSD / 10).to_string(),
        challenge_bond: t(1),
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
        known: Known { max_count: 0, max_share_bps: 0 },
        pause: Pause { slots: vec![], min_impact_usd_micro: "0".into() },
        terms_sha256: hex::encode(sha256(PRACTICE_TERMS.as_bytes())),
    }
}

/// The practice program's manifest (v2): the factory and each vault as its own target (`vault-<i>`),
/// and the one rule.
fn practice_manifest(dep: &Value, evm: &Evm, anchor_n: u64, anchor_h: B256, project: &str, deadline: u64) -> Result<Manifest> {
    let code: alloy_primitives::Bytes = dep["invariantsRuntimeCode"].as_str().unwrap_or_default().parse()?;
    let factory = dep["factory"].as_str().ok_or_else(|| eyre!("practice deployment: factory"))?;
    let mut targets = vec![Target { name: "factory".into(), address: factory.parse()?, code_hash: code_hash(evm, factory, anchor_n)? }];
    let mut vaults = Vec::new();
    for (i, v) in dep["vaults"].as_array().into_iter().flatten().enumerate() {
        let v = v.as_str().unwrap_or_default();
        targets.push(Target { name: format!("vault-{i}"), address: v.parse()?, code_hash: code_hash(evm, v, anchor_n)? });
        vaults.push(v.parse()?);
    }
    let mut m = practice_manifest_parts(anchor_n, anchor_h, project, deadline)?;
    m.targets = targets;
    m.impact.protocol_accounts = vaults;
    m.invariants.runtime_code_hash = keccak256(&code);
    m.invariants.runtime_code = code;
    Ok(m)
}

/// The practice program's manifest, but for its targets and its rule's runtime code.
fn practice_manifest_parts(anchor_n: u64, anchor_h: B256, project: &str, deadline: u64) -> Result<Manifest> {
    Ok(Manifest {
        schema: MANIFEST_SCHEMA_V2.into(),
        program: ProgramInfo {
            name: "Practice program — a tutorial (sandbox demo)".into(),
            project: project.into(),
            description: "TUTORIAL TARGET: 24 vaults with one textbook bug (sweep has no access check), published with its proof (docs/practice.md).".into(),
        },
        anchor: Anchor { chain_id: 31337, block_number: anchor_n, block_hash: anchor_h, evm_spec: "osaka".into(), trust: "project-attested".into() },
        targets: vec![],
        invariants: Invariants {
            address: alloy_primitives::Address::from_slice(&keccak256(b"antibody/invariants/v1")[12..]),
            runtime_code_hash: keccak256([]),
            runtime_code: Default::default(),
            gas_limit: 5_000_000,
            slots: vec![Slot { bit: 0, name: "vault_keeps_its_ether".into(), severity: Severity::Low }],
        },
        impact: ImpactConfig { protocol_accounts: vec![], native_usd: "2000".into(), tokens: vec![] },
        capabilities: Capabilities {
            max_senders: 1,
            max_endowment_wei: 10u128.pow(16).to_string(),
            max_txs: 1,
            max_gas_per_tx: 1_000_000,
            max_total_gas: 1_000_000,
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
        settlement: Some(serde_json::to_value(practice_settlement(deadline))?),
        deploy: None,
        rule_strength: None,
    })
}

/// A contract's creation code from the forge build (contracts/evm/out).
fn creation_code(a: &DemoArgs, file: &str, contract: &str) -> Result<alloy_primitives::Bytes> {
    let art: Value = serde_json::from_str(&std::fs::read_to_string(a.root.join(format!("contracts/evm/out/{file}/{contract}.json")))?)?;
    Ok(art["bytecode"]["object"].as_str().ok_or_else(|| eyre!("{contract}: no bytecode"))?.parse()?)
}

fn word(x: impl Into<alloy_primitives::U256>) -> [u8; 32] {
    x.into().to_be_bytes::<32>()
}

/// The practice program in fork mode (docs/fork-mode.md): the factory and its 24 vaults exist only inside
/// the verifier, deployed by the manifest's two transactions on top of the anchor (the second
/// instantiates the rule, for its runtime code). Nothing of it is on chain.
fn fork_practice_manifest(a: &DemoArgs, ctx: &Ctx, anchor_n: u64, anchor_h: B256, project: &str, deadline: u64) -> Result<Manifest> {
    use antibody_verifier::manifest::{deployer, Deploy, DeployTx};
    let d = deployer();
    let factory = d.create(0);
    let rule = d.create(1);
    let mut factory_init = creation_code(a, "Practice.sol", "PracticeFactory")?.to_vec();
    factory_init.extend(word(alloy_primitives::U256::from(24u64)));
    let mut rule_init = creation_code(a, "Practice.sol", "PracticeInvariants")?.to_vec();
    rule_init.extend(factory.into_word().0);
    let mut vault_init = creation_code(a, "Practice.sol", "PracticeVault")?.to_vec();
    vault_init.extend(d.into_word().0);
    let vault_hash = keccak256(&vault_init);
    let vaults: Vec<alloy_primitives::Address> = (0..24u64).map(|i| factory.create2(B256::from(word(alloy_primitives::U256::from(i))), vault_hash)).collect();
    let mut m = practice_manifest_parts(anchor_n, anchor_h, project, deadline)?;
    m.program.name = "Practice program in fork mode — a tutorial (sandbox demo)".into();
    m.program.description = "TUTORIAL TARGET deployed only inside the verifier (fork mode): 24 vaults with one textbook bug (docs/fork-mode.md).".into();
    m.deploy = Some(Deploy {
        txs: vec![
            DeployTx { to: None, value: alloy_primitives::U256::from(24_000_000_000_000_000u64), data: factory_init.into() },
            DeployTx { to: None, value: alloy_primitives::U256::ZERO, data: rule_init.into() },
        ],
    });
    let mut addrs = vec![factory, rule];
    addrs.extend(&vaults);
    let codes = cmd::deployed_code(ctx, &m, &addrs)?;
    m.targets = std::iter::once(("factory".to_string(), factory, codes[0].0))
        .chain(vaults.iter().zip(&codes[2..]).enumerate().map(|(i, (v, c))| (format!("vault-{i}"), *v, c.0)))
        .map(|(name, address, code_hash)| Target { name, address, code_hash })
        .collect();
    m.impact.protocol_accounts = vaults;
    m.invariants.runtime_code = codes[1].1.clone();
    m.invariants.runtime_code_hash = keccak256(&codes[1].1);
    Ok(m)
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
    // The server only reads files inside its workspace (the state directory): the agent's own copy.
    let local = bundle.file_name().and_then(|n| n.to_str()).ok_or_else(|| eyre!("bundle file name"))?.to_string();
    std::fs::copy(bundle, state.join(&local))?;
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
    let names: Vec<Value> = tools["result"]["tools"].as_array().into_iter().flatten().map(|t| t["name"].clone()).collect();
    let r = rpc(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "submit",
        "arguments": {"program_id": program, "bundle_path": local, "wait": true}}}))?;
    drop(stdin);
    let _ = child.wait();
    if r["result"]["isError"] == json!(true) {
        return Err(eyre!("MCP submit failed: {}", r["result"]["content"][0]["text"]));
    }
    let data = antibody_cli::mcp::result_data(&r["result"]).ok_or_else(|| eyre!("MCP submit: no data in {}", r["result"]))?;
    Ok(json!({"mcp_tools": names.len(), "tool_names": names, "result": data}))
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

/// G's bundle (the gap lane): PocG_ArbitraryFrom deposits the saver's approved acUSD for itself.
fn gap_bundle(a: &DemoArgs, dep: &Value) -> Result<PathBuf> {
    let art: Value = serde_json::from_slice(&std::fs::read(a.root.join("contracts/evm/out/AcmePocs.sol/PocG_ArbitraryFrom.json"))?)?;
    let word = |k: &str| format!("{:0>64}", dep[k].as_str().unwrap_or_default().trim_start_matches("0x").to_lowercase());
    let data = format!("{}{}{}{}", art["bytecode"]["object"].as_str().unwrap_or_default(), word("AcmeVault"), word("acUSD"), word("saver"));
    let bundle = json!({"senders": 1, "endowment_wei": (10u128.pow(18)).to_string(), "protected": [],
                        "calls": [{"sender": 0, "to": null, "value": "0", "data": data, "gas_limit": 3_000_000}]});
    let path = a.work.join("G.bundle.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&bundle)?)?;
    Ok(path)
}

/// One round of a fix (docs/rounds.md). The round manifest is the program's own with code overrides read
/// from the deployed fixes. The project proposes it with its releases; the TEE checks both (check_round).
/// The keeper re-runs every eligible commit at the fix, then opens the round once its windows pass.
#[allow(clippy::too_many_arguments)]
fn run_round(a: &DemoArgs, acme: &Ctx, keeper: &Ctx, near: &Near, pid: u32, round: u32, base: &Path, dep: &Value, evm: &Evm,
             overrides: &[(&str, &str)], invariants: Option<(&str, &[&str])>, releases: &[(&str, &str)]) -> Result<Value> {
    let t = Instant::now();
    let dir = a.work.join(format!("round{round}"));
    std::fs::create_dir_all(&dir)?;
    let mut flags = Vec::new();
    for (target, fix) in overrides {
        let file = dir.join(format!("{fix}.runtime.hex"));
        std::fs::write(&file, evm.call("eth_getCode", json!([dep[*fix], "latest"]))?.as_str().unwrap_or_default())?;
        flags.push(format!("{}={}", dep[*target].as_str().unwrap_or_default(), file.display()));
    }
    let manifest = dir.join("manifest.json");
    let added: Vec<String> = invariants.map(|(_, s)| s.iter().map(|x| x.to_string()).collect()).unwrap_or_default();
    let inv_file = dir.join("invariants.hex");
    if let Some((code, _)) = invariants {
        std::fs::write(&inv_file, code)?;
    }
    // The final fix carries its rule strength, measured at that code (docs/rule-strength.md).
    let strength = a.root.join("docs/results/rule-strength-acme.json");
    let strength = (round == 2 && strength.exists()).then_some(strength);
    cmd::manifest_round(base, round, &flags, invariants.map(|_| (inv_file.as_path(), added.as_slice())), strength.as_deref(), &manifest)?;
    let rel: Vec<Value> = releases
        .iter()
        .map(|(proxy, fix)| json!({"proxy": dep[*proxy], "implementation": dep[*fix], "code_hash": dep[format!("codehash{fix}")]}))
        .collect();
    let proposed = cmd::round_propose(acme, pid, &manifest, &rel, None, true)?;
    let check = proposed["run_id"].as_u64().unwrap_or_default();
    wait_t1(acme, check)?;
    let rc = cmd::run(acme, check)?["outcome"]["Answered"]["facts"]["extra"]["RoundCheck"].clone();
    if rc["ok"] != json!(true) {
        return Err(eyre!("round {round}: the TEE did not accept the proposal (run {check}): {rc}"));
    }
    let k = cmd::keeper(keeper, pid)?;
    let runs: Vec<u64> = k["steps"].as_array().into_iter().flatten().filter_map(|s| s["run_id"].as_u64()).collect();
    for r in &runs {
        wait_t1(acme, *r)?;
    }
    let mut reruns = Vec::new();
    for r in &runs {
        let v = cmd::run_view(&cmd::run(acme, *r)?);
        reruns.push(json!({"commit": v["commit_id"], "run": r, "kind": v["kind"], "tier": v["tier"]}));
    }
    // The round's minimum duration and re-run window are block time.
    near.call("sandbox_fast_forward", json!({"delta_height": 200}))?;
    let opened = cmd::round_open(keeper, pid)?["round_opened"].clone();
    // Release permits beyond one call's gas are queued at near-guardian; the keeper asks for them.
    let k2 = cmd::keeper(keeper, pid)?;
    let summary = format!("{} re-runs ({} fixed), release gate {}, {} keys released, {} carried",
        reruns.len(), reruns.iter().filter(|r| r["kind"] == "NoViolation").count(), opened["release_gate"], opened["released"], opened["carried"]);
    Ok(json!({"round": round, "manifest_sha256": proposed["manifest_sha256"], "check_round_run": check, "canaries_ok": rc["canaries_ok"],
              "releases": rel, "reruns": reruns, "opened": opened, "summary": summary, "keeper": [k, k2], "seconds": t.elapsed().as_secs_f64()}))
}

/// Every payout transaction in a result (the keeper's `{"paid": commit, "tx": hash}` steps), by commit.
fn pay_txs(v: &Value, out: &mut BTreeMap<u64, Value>) {
    match v {
        Value::Object(m) => {
            if let (Some(c), Some(tx)) = (m.get("paid").and_then(|c| c.as_u64()), m.get("tx").filter(|t| t.is_string())) {
                out.entry(c).or_insert_with(|| tx.clone());
            }
            m.values().for_each(|x| pay_txs(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| pay_txs(x, out)),
        _ => {}
    }
}

pub fn run(a: DemoArgs) -> Result<Value> {
    let t_all = Instant::now();
    std::fs::create_dir_all(&a.work)?;
    // A fresh chain: no service state or local record may survive from an earlier run.
    for d in ["services", "state", "keys"] {
        let _ = std::fs::remove_dir_all(a.work.join(d));
    }
    let mut procs = Procs(Vec::new());
    let mut results = json!({"started_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs()});

    step("EVM: two anvils (31337 verifies and pauses, 31338 only pauses) and the Acme Lend fixture on both");
    procs.0.push(start_anvil(&a.anvil_bin, 31337, 8545, &a.work.join("anvil-31337.log"))?);
    procs.0.push(start_anvil(&a.anvil_bin, 31338, 8546, &a.work.join("anvil-31338.log"))?);
    // No pauser yet: the program's adapter becomes the pauser once it exists.
    let dep_a = deploy_acme(&a, "http://127.0.0.1:8545", "0x0000000000000000000000000000000000000000", &a.work.join("acme-31337.json"))?;
    let dep_b = deploy_acme(&a, "http://127.0.0.1:8546", "0x0000000000000000000000000000000000000000", &a.work.join("acme-31338.json"))?;
    let same = ["AcmeGuard", "AcmeLend", "AcmeLendImpl", "AcmeVault", "AcmeVaultImpl", "AcmeLendFixed", "AcmeLendFinal", "AcmeVaultFixed"]
        .iter()
        .all(|k| dep_a[k] == dep_b[k]);
    println!("same addresses on both chains: {same}");
    // The practice program's vaults exist at the same anchor (another deployer: Acme's nonces are untouched).
    let dep_p = deploy_practice(&a, "http://127.0.0.1:8545", &a.work.join("practice-31337.json"))?;
    let evm_a = Evm::new("http://127.0.0.1:8545");
    let anchor = evm_a.call("eth_getBlockByNumber", json!(["latest", false]))?;
    let anchor_n = crate::evm::hex_u64(&anchor["number"])?;
    let anchor_h: B256 = anchor["hash"].as_str().unwrap_or_default().parse()?;
    // The anchor must never be anvil's mutable head: later blocks build on it, its state is fixed.
    evm_a.call("evm_mine", json!([]))?;

    step("NEAR: sandbox, accounts and contracts (tUSD, near-antibody, near-guardian, MOCK OutLayer, DCAP, MPC, light client)");
    let (sandbox, root) = start_sandbox(&a, &a.work.join("near"), &a.work.join("sandbox.log"))?;
    procs.0.push(sandbox);
    let near = Near::new("sandbox", Some("http://127.0.0.1:3030".into()));
    let keys = a.work.join("keys");
    std::fs::create_dir_all(&keys)?;
    std::fs::write(keys.join("test.near.json"), serde_json::to_vec(&root.to_file_json())?)?;
    let names = ["tusd", "bounty", "guardian", "outlayer", "dcap", "mpc", "lc", "acme", "worker", "keeper", "relayer", "agent-a", "agent-b", "agent-c", "agent-d", "agent-e", "agent-f", "agent-g"];
    let acct = |n: &str| format!("{n}.test.near");
    let dep = Deployments {
        network: "sandbox".into(),
        antibody: acct("bounty"),
        token: acct("tusd"),
        token_decimals: None,
        token_symbol: None,
        guardian: Some(acct("guardian")),
        outlayer: Some(acct("outlayer")),
        light_client: Some(acct("lc")),
        rpc: Some("http://127.0.0.1:3030".into()),
        eth_rpc: None,
        practice: None,
        release: None,
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
    cmd::admin_deploy(&ctx_of("mpc"), &rel.join("antibody_mock_mpc.wasm"), Some("new".into()),
        Some(json!({"signer": acct("worker"), "public_key": mpc_public_key()}).to_string()))?;
    cmd::admin_deploy(&ctx_of("lc"), &rel.join("antibody_mock_lc.wasm"), Some("new".into()), Some(json!({"owner": "test.near"}).to_string()))?;
    // The local chain's anchor, as the real light client would have it for Sepolia.
    cmd::admin_call(&root_ctx, &acct("lc"), "set_block_hash", &json!({"number": anchor_n, "hash": format!("{anchor_h:#x}")}).to_string(), 0, 30)?;
    cmd::admin_deploy(&ctx_of("guardian"), &a.root.join("contracts/near-guardian/target/wasm32-unknown-unknown/release/antibody_guardian.wasm"),
        Some("new".into()), Some(json!({"owner": "test.near", "mpc": acct("mpc"), "antibody": acct("bounty")}).to_string()))?;
    let defaults = json!({
        "verifier": {"wasm_url": "https://github.com/Himess/antibody/releases/download/sandbox/antibody-verify.wasm", "wasm_hash": verifier_hash,
                     "max_instructions": 5_000_000_000u64, "max_memory_mb": 512, "max_execution_seconds": 60, "deposit": "10000000000000000000000"},
        "outlayer": acct("outlayer"), "dcap_verifier": acct("dcap"), "accepted_tcb": ["UpToDate"], "image_delay_secs": 3_600,
        "light_client": acct("lc"), "light_client_chain_id": 31337, "guardian": acct("guardian"), "treasury": "test.near", "image_registry": acct("outlayer"),
    });
    cmd::admin_deploy(&ctx_of("bounty"), &rel.join("near_antibody.wasm"), Some("new".into()),
        Some(json!({"owner": "test.near", "token": acct("tusd"), "defaults": defaults}).to_string()))?;
    // near-antibody copies the (MOCK) approved worker images before any program exists. The owner's sync
    // is paid from the contract's balance; anyone may sync later, paying the new entries' storage.
    cmd::admin_call(&root_ctx, &acct("bounty"), "sync_images", "{}", 0, 60)?;
    cmd::admin_call(&root_ctx, &acct("tusd"), "mint", &json!({"account_id": acct("acme"), "amount": (50_000 * ONE_TUSD).to_string()}).to_string(), 0, 30)?;
    for x in ["a", "b", "c", "d", "e", "f", "g"] {
        cmd::admin_call(&root_ctx, &acct("tusd"), "mint", &json!({"account_id": acct(&format!("agent-{x}")), "amount": (200 * ONE_TUSD).to_string()}).to_string(), 0, 30)?;
    }
    results["deployment"] = json!({"network": "sandbox", "near_antibody": acct("bounty"), "guardian": acct("guardian"), "token": acct("tusd"), "verifier_wasm_sha256": verifier_hash, "images": near.view(&acct("bounty"), "get_images", json!({}))?});

    step("Background: MOCK worker (the real verifier in TEE mode, TEST root key), MOCK MPC signer, T1 relayer, permit relayer");
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

    {
        let (near, stop, signer, mpc) = (near.clone(), stop.clone(), worker_key.clone(), acct("mpc"));
        handles.push(std::thread::spawn(move || {
            let mut s = MockSigner { near, mpc, signer, key: demo_mpc_key(), done: Default::default() };
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = s.tick() {
                    eprintln!("mock signer: {e:#}");
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }));
    }
    // The indexer runs from the start, as it does next to a deployment: a node garbage-collects old
    // blocks, so an indexer started late would miss their events. It stops after the API step (or with
    // the chains, under --serve).
    let services_dir = a.work.join("services/api");
    let ix_stop = Arc::new(AtomicBool::new(false));
    let ix_handle = {
        let ix = crate::indexer::Indexer {
            near: near.clone(),
            contracts: vec![acct("bounty"), acct("tusd"), acct("guardian")],
            antibody: acct("bounty"),
            guardian: Some(acct("guardian")),
            source: crate::indexer::Source::Blocks { from: 0 },
            state_dir: services_dir.clone(),
        };
        let stop = ix_stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = ix.tick() {
                    eprintln!("indexer: {e:#}");
                }
                std::thread::sleep(Duration::from_millis(1_000));
            }
        })
    };
    let relayer_file = a.work.join("services/relayer.json");
    {
        let r = Relayer {
            near: near.clone(),
            guardian: acct("guardian"),
            antibody: acct("bounty"),
            chains: BTreeMap::from([
                (31337, ("http://127.0.0.1:8545".to_string(), "local-31337:".to_string())),
                (31338, ("http://127.0.0.1:8546".to_string(), "local-31338:".to_string())),
            ]),
            signer: Some(EvmSigner::from_hex(ANVIL_KEY_1)?),
            state_file: relayer_file.clone(),
        };
        let stop = stop.clone();
        handles.push(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = r.tick() {
                    eprintln!("permit relayer: {e:#}");
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }));
    }

    step(&format!("Program: manifest v2 committed on NEAR, escrow {ESCROW_TUSD} tUSD"));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let manifest = demo_manifest(&dep_a, &evm_a, anchor_n, anchor_h, &acct("acme"), now + 3_600)?;
    let manifest_path = a.work.join("manifest.json");
    std::fs::write(&manifest_path, manifest.canonical())?;
    // The project key comes from acme's NEP-413 signature: the same NEAR key re-derives it.
    let project_key = a.work.join("keys/acme.x25519.json");
    let acme = ctx_of("acme");
    let derived = cmd::keygen_x25519_nep413(&acme, &project_key)?;
    let created = cmd::program_create(&acme, &manifest_path, ESCROW_TUSD, &project_key, None)?;
    let pid = created["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no program id: {created}"))? as u32;
    println!("program {pid} created: {} (anchor {})", created["tx"]["tx"], created["anchor"]["anchor"]);
    results["program"] = json!({"program_id": pid, "manifest_sha256": hex::encode(sha256(&manifest.canonical())), "anchor_block": anchor_n,
                                "anchor_hash": format!("{anchor_h:#x}"), "create_tx": created["tx"]["tx"], "anchor": created["anchor"],
                                "project_key": {"public_key": derived["public_key"], "nep413": derived["nep413"]}});
    let program_key = acme.program(pid)?["key"].as_str().unwrap_or_default().to_string();

    step("Guardian: the program's adapter on both chains (CREATE2), registered with near-guardian");
    let guardian_evm = eth_address(&demo_mpc_key());
    let ad_a = deploy_adapter(&a, "http://127.0.0.1:8545", &guardian_evm, &program_key, &a.work.join("acme-31337.json"), &a.work.join("adapter-31337.json"))?;
    let ad_b = deploy_adapter(&a, "http://127.0.0.1:8546", &guardian_evm, &program_key, &a.work.join("acme-31338.json"), &a.work.join("adapter-31338.json"))?;
    let adapter = ad_a["adapter"].as_str().unwrap_or_default().to_lowercase();
    if ad_b["adapter"].as_str().unwrap_or_default().to_lowercase() != adapter {
        return Err(eyre!("the adapter's address differs between chains"));
    }
    let proxies: Vec<String> = ["AcmeLend", "AcmeVault"].iter().map(|k| dep_a[*k].as_str().unwrap_or_default().to_lowercase()).collect();
    let chains = json!([{"chain_id": 31337, "adapter": adapter, "proxies": proxies}, {"chain_id": 31338, "adapter": adapter, "proxies": proxies}]);
    let reg = acme.near.function_call(&acme.signer()?, &acct("guardian"), "register", json!({"program_key": program_key, "chains": chains}), 60,
        100_000_000_000_000_000_000_000)?;
    let pinned = near.view(&acct("guardian"), "get_program", json!({"program_key": program_key, "project": acct("acme")}))?;
    println!("adapter {adapter} on 31337 and 31338; guardian address {} (expected {guardian_evm})", pinned["address"]);
    if pinned["address"].as_str().map(|x| x.eq_ignore_ascii_case(&guardian_evm)) != Some(true) {
        return Err(eyre!("near-guardian pinned {} for the program, not the adapter's {guardian_evm}", pinned["address"]));
    }
    results["guardian"] = json!({"adapter": adapter, "proxies": proxies, "guardian_address": pinned["address"], "register_tx": cmd::event(&json!({"events": antibody_cli::near::events(&reg)}), "program_registered")});

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
    for x in ["a", "b", "c", "d", "e", "f", "g"] {
        let c = ctx_of(&format!("agent-{x}"));
        cmd::storage_deposit(&c, "1")?;
        cmd::deposit(&c, "100")?;
    }
    // (finding, commit, seconds to the verdict, its commit and reveal transactions)
    let mut rows: Vec<(String, u64, f64, Value)> = Vec::new();
    let agent_key = a.work.join("keys/agent-a.fc-key.json");
    cmd::agent_key_add(&ctx_of("agent-a"), 2.0, &agent_key)?;
    let state_a = a.work.join("state/agent-a-mcp");
    std::fs::create_dir_all(&state_a)?;
    let t = Instant::now();
    let sandbox_net = ["--network", "sandbox", "--rpc", "http://127.0.0.1:3030"];
    let mcp = mcp_submit(&a.antibody_bin, &sandbox_net, &dep_file, &acct("agent-a"), &agent_key, Some("http://127.0.0.1:8545"), &state_a, pid, &fx.join("A.bundle.json"))?;
    let a_commit = mcp["result"]["commit"]["commit_id"].as_u64().ok_or_else(|| eyre!("MCP submit: no commit id: {mcp}"))?;
    let t_verdict = t.elapsed().as_secs_f64();
    results["mcp"] = json!({"tools": mcp["mcp_tools"], "tool_names": mcp["tool_names"], "commit_id": a_commit});

    step("The one moment: A's verdict is T1, A is paid in seconds, and both chains pause");
    wait_t1(&root_ctx, root_ctx.commit(a_commit)?["run"].as_u64().unwrap_or_default())?;
    let t_t1 = t.elapsed().as_secs_f64();
    let keeper_a = cmd::keeper(&ctx_of("keeper"), pid)?;
    let t_paid = t.elapsed().as_secs_f64();
    let paid_a = cmd::paid_of(&root_ctx.commit(a_commit)?);
    let mut paused_at: BTreeMap<String, f64> = BTreeMap::new();
    wait_for("pause() on both chains", Duration::from_secs(120), || {
        let st: BTreeMap<String, Value> = std::fs::read(&relayer_file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        for v in st.values() {
            if v["kind"] == "pause" && v["status"] == "confirmed" && v["targets_paused"].as_array().is_some_and(|p| !p.is_empty() && p.iter().all(|x| x == true)) {
                paused_at.entry(v["chain_id"].to_string()).or_insert_with(|| t.elapsed().as_secs_f64());
            }
        }
        paused_at.len() == 2
    })?;
    let relayed: BTreeMap<String, Value> = std::fs::read(&relayer_file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let paused_both = evm_a.paused(dep_a["AcmeGuard"].as_str().unwrap_or_default())? && Evm::new("http://127.0.0.1:8546").paused(dep_b["AcmeGuard"].as_str().unwrap_or_default())?;
    println!("A: commit→verdict {t_verdict:.1}s, T1 at {t_t1:.1}s, paid {} tUSD at {t_paid:.1}s; paused on 31337 at {:.1}s and 31338 at {:.1}s",
             paid_a / ONE_TUSD, paused_at.get("31337").copied().unwrap_or(-1.0), paused_at.get("31338").copied().unwrap_or(-1.0));
    rows.push(("A (MCP)".into(), a_commit, t_verdict, json!({"commit": mcp["result"]["commit"]["tx"]["tx"], "reveal": mcp["result"]["reveal"]["tx"]["tx"]})));
    results["one_moment"] = json!({"commit": a_commit, "seconds_from_commit": {"verdict": t_verdict, "t1": t_t1, "paid": t_paid,
                                   "paused_31337": paused_at.get("31337"), "paused_31338": paused_at.get("31338")},
                                   "paid_raw": paid_a.to_string(), "instant": root_ctx.commit(a_commit)?["state"]["Done"]["Paid"]["instant"],
                                   "paused_both": paused_both, "keeper": keeper_a, "permits": relayed});

    step("A NEAR app consuming the verdict: a DAO treasury freezes on A's T1-confirmed Critical (anyone reports it)");
    cmd::admin_create_account(&root_ctx, &acct("dao-treasury"), "10", None, Some(&keys.join(format!("{}.json", acct("dao-treasury")))))?;
    let guard_wasm = a.root.join("contracts/verdict-guard/target/wasm32-unknown-unknown/release/antibody_verdict_guard.wasm");
    cmd::admin_deploy(&ctx_of("dao-treasury"), &guard_wasm, Some("new".into()),
        Some(json!({"owner": acct("acme"), "antibody": acct("bounty"), "program_id": pid}).to_string()))?;
    let pay_out = |ctx: &Ctx| cmd::admin_call(ctx, &acct("dao-treasury"), "transfer", &json!({"to": "test.near", "amount": "1000000000000000000000000"}).to_string(), 1, 30);
    let before_freeze = pay_out(&acme).is_ok();
    let reported = cmd::admin_call(&ctx_of("keeper"), &acct("dao-treasury"), "report", &json!({"commit_id": a_commit}).to_string(), 0, 150)?;
    let state = near.view(&acct("dao-treasury"), "get_state", json!({}))?;
    let refused = pay_out(&acme).err().map(|e| e.to_string()).unwrap_or_default();
    let guard_ok = before_freeze && state["frozen"]["commit_id"] == json!(a_commit) && refused.contains("frozen");
    println!("DAO treasury: transfer before {}, frozen by commit {} (slot {}), transfer after: {}", if before_freeze { "ok" } else { "FAILED" },
             state["frozen"]["commit_id"], state["frozen"]["slot"], if refused.contains("frozen") { "refused" } else { "NOT refused" });
    results["verdict_consumer"] = json!({"contract": acct("dao-treasury"), "watched_program": pid, "report_tx": reported["tx"]["tx"], "reported_by": acct("keeper"),
                                         "frozen": state["frozen"], "transfer_before": before_freeze, "transfer_after_refused": refused.contains("frozen"), "ok": guard_ok});

    step("Agents B–G (library/CLI path); D commits after A; G harms the protected saver and breaks no invariant (the gap lane)");
    let g_bundle = gap_bundle(&a, &dep_a)?;
    // What one submission costs its hunter (B): near-antibody's storage, B's NEAR credit and wallet,
    // from before the commit to the verdict.
    let costs_of = |who: &str| -> Result<(u64, u128, u128)> {
        let c = near.account(&acct("bounty"))?.ok_or_else(|| eyre!("no contract account"))?;
        let w = near.account(&acct(who))?.ok_or_else(|| eyre!("no account {who}"))?;
        Ok((c["storage_usage"].as_u64().unwrap_or(0), cmd::u128_of(&cmd::account(&root_ctx, &acct(who))?["near"]), cmd::u128_of(&w["amount"])))
    };
    for x in ["b", "c", "d", "e", "f", "g"] {
        let t = Instant::now();
        let report = if x == "c" { Some(fx.join("C.report.md")) } else { None };
        let bundle = if x == "g" { g_bundle.clone() } else { fx.join(format!("{}.bundle.json", x.to_uppercase())) };
        let costs_before = if x == "b" { Some(costs_of("agent-b")?) } else { None };
        let r = cmd::submit(&ctx_of(&format!("agent-{x}")), pid, &bundle, None, report.as_deref(), true)?;
        if let Some((s0, c0, w0)) = costs_before {
            let (s1, c1, w1) = costs_of("agent-b")?;
            results["costs"] = json!({"submission": "B", "contract_storage_bytes": s1.saturating_sub(s0), "near_credit_spent_yocto": c0.saturating_sub(c1).to_string(),
                                      "wallet_delta_yocto": (w1 as i128 - w0 as i128).to_string(), "inline_blob_bytes": r["reveal"]["upload"]["inline_bytes"],
                                      "note": "sandbox: the MOCK OutLayer refunds the run deposit to the wallet, and gas costs ten times testnet's"});
        }
        rows.push((x.to_uppercase(), r["commit"]["commit_id"].as_u64().unwrap_or(0), t.elapsed().as_secs_f64(),
                   json!({"commit": r["commit"]["tx"]["tx"], "reveal": r["reveal"]["tx"]["tx"]})));
    }
    for (_, id, ..) in &rows {
        if let Some(r) = root_ctx.commit(*id)?["run"].as_u64() {
            wait_t1(&root_ctx, r)?;
        }
    }

    step("Above the instant cap: an independent second run (anyone may ask; the keeper does), then the challenge window");
    let program = root_ctx.program(pid)?;
    let mut seconds = Vec::new();
    for (name, id, ..) in &rows {
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
    let mut keeper_runs = Vec::new();
    wait_for("every claim settled", Duration::from_secs(180), || {
        keeper_runs.push(cmd::keeper(&ctx_of("keeper"), pid).unwrap_or(Value::Null));
        rows.iter().all(|(_, id, ..)| root_ctx.commit(*id).map(|c| cmd::state_label(&c["state"]) != "Verifying" && cmd::state_label(&c["state"]) != "Holder").unwrap_or(false))
    })?;
    let before = cmd::u128_of(&root_ctx.near.view(&acct("tusd"), "ft_balance_of", json!({"account_id": acct("agent-b")}))?);
    cmd::withdraw(&ctx_of("agent-b"), "3000")?;
    let after = cmd::u128_of(&root_ctx.near.view(&acct("tusd"), "ft_balance_of", json!({"account_id": acct("agent-b")}))?);
    results["keeper"] = keeper_runs.last().cloned().unwrap_or(Value::Null);
    results["keeper_runs"] = Value::Array(keeper_runs);
    results["withdrawal"] = json!({"account": acct("agent-b"), "wallet_before": cmd::fmt_tusd(before), "wallet_after": cmd::fmt_tusd(after)});

    step("The project reads what it paid for: A's proof, wrapped to its NEP-413 key after payment; D's duplicate is never wrapped");
    let t = Instant::now();
    let reveal_of = |name: &str| rows.iter().find(|r| r.0 == name).map(|r| (r.1, r.3["reveal"].as_str().unwrap_or_default().to_string()));
    let (a_id, a_reveal) = reveal_of("A (MCP)").ok_or_else(|| eyre!("no row for A"))?;
    let read = cmd::program_read(&acme, a_id, &project_key, &a.work.join("read/A"), &cmd::BlobFrom::Tx { hash: a_reveal, signer: None })?;
    let again = a.work.join("keys/acme.x25519.again.json");
    let rederived = cmd::keygen_x25519_nep413(&acme, &again)?["public_key"] == derived["public_key"];
    // near-antibody itself refuses to wrap a duplicate, even with its blob.
    let (d_id, d_reveal) = reveal_of("D").ok_or_else(|| eyre!("no row for D"))?;
    let d_blob = cmd::blob_arg_in_tx(&acme, &d_reveal, &acct("agent-d"), d_id)?;
    let d_refused = cmd::call(&acme, "release_wrap", json!({"commit_id": d_id, "blob": d_blob}), cmd::RUN_TGAS).err().map(|e| e.to_string()).unwrap_or_default();
    let read_ok = read["local_verdict"]["kind"] == json!("Violation") && read["receipt_matches_the_tee"] == json!(true) && rederived
        && d_refused.contains("only a paid finding");
    println!("project read A's proof (commit {a_id}): {} calls, local verdict {}, receipt {} the TEE's; key re-derived: {rederived}; D's release wrap: {}",
             read["calls"], read["local_verdict"]["kind"], if read["receipt_matches_the_tee"] == json!(true) { "equals" } else { "DIFFERS FROM" },
             if d_refused.contains("only a paid finding") { "refused" } else { "NOT refused" });
    results["project_read"] = json!({"commit": a_id, "read": read, "key_rederived": rederived, "duplicate": d_id, "duplicate_refused": d_refused,
                                     "ok": read_ok, "seconds": t.elapsed().as_secs_f64()});

    step("Round 1, a partial fix (AcmeLendFixed, bug #1) and a new invariant (protected_balances): the keeper re-runs every paid PoC; B and F still break, G's claim is promoted");
    let keeper = ctx_of("keeper");
    let v2 = dep_a["invariantsV2RuntimeCode"].as_str().unwrap_or_default().to_string();
    let r1 = run_round(&a, &acme, &keeper, &near, pid, 1, &manifest_path, &dep_a, &evm_a, &[("AcmeLendImpl", "AcmeLendFixed")],
                       Some((v2.as_str(), &["3:protected_balances:medium"])), &[("AcmeLend", "AcmeLendFixed")])?;
    println!("round 1: {}", r1["summary"]);
    step("Round 2, the final fix (AcmeLendFinal, AcmeVaultFixed): every paid PoC is fixed, so the release gate holds");
    let r2 = run_round(&a, &acme, &keeper, &near, pid, 2, &manifest_path, &dep_a, &evm_a,
                       &[("AcmeLendImpl", "AcmeLendFinal"), ("AcmeVaultImpl", "AcmeVaultFixed")],
                       Some((v2.as_str(), &["3:protected_balances:medium"])),
                       &[("AcmeLend", "AcmeLendFinal"), ("AcmeVault", "AcmeVaultFixed")])?;
    println!("round 2: {}", r2["summary"]);
    step("Release: the guardian signs a permit per proxy and chain; the relayer upgrades both proxies on both chains");
    let t = Instant::now();
    wait_for("4 releases confirmed", Duration::from_secs(180), || {
        let st: BTreeMap<String, Value> = std::fs::read(&relayer_file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        st.values().filter(|v| v["kind"] == "release" && v["status"] == "confirmed").count() == 4
    })?;
    let mut released = Vec::new();
    for (chain, rpc, dep) in [(31337, "http://127.0.0.1:8545", &dep_a), (31338, "http://127.0.0.1:8546", &dep_b)] {
        let evm = Evm::new(rpc);
        for (proxy, fix) in [("AcmeLend", "AcmeLendFinal"), ("AcmeVault", "AcmeVaultFixed")] {
            let word = evm.call("eth_getStorageAt", json!([dep[proxy], format!("{:#x}", antibody_verifier::run::EIP1967_IMPLEMENTATION_SLOT), "latest"]))?;
            let now = format!("0x{}", &word.as_str().unwrap_or_default().trim_start_matches("0x")[24..]);
            released.push(json!({"chain_id": chain, "proxy": proxy, "implementation": now, "expected": dep[fix],
                                 "ok": now.eq_ignore_ascii_case(dep[fix].as_str().unwrap_or_default())}));
        }
        // B's exploit, the unguarded fee sweep, against the released code.
        let sweep = format!("0x{}{}", hex::encode(&keccak256(b"sweepFees(address)")[..4]), "00".repeat(12) + "000000000000000000000000000000000000beef");
        let reverts = evm.call("eth_call", json!([{"from": "0x000000000000000000000000000000000000beef", "to": dep["AcmeVault"], "data": sweep}, "latest"])).is_err();
        released.push(json!({"chain_id": chain, "b_exploit_reverts": reverts}));
    }
    let releases_ok = released.iter().all(|r| r["ok"] != json!(false) && r["b_exploit_reverts"] != json!(false));
    println!("released on both chains in {:.1}s: {}", t.elapsed().as_secs_f64(), if releases_ok { "both proxies run the final fix; B's exploit reverts" } else { "MISMATCH" });
    results["rounds"] = json!([r1, r2]);
    results["release"] = json!({"checks": released, "ok": releases_ok, "seconds": t.elapsed().as_secs_f64()});

    step("Practice program (a tutorial): 24 vaults, one public bug; the vending contract; two judges with no account try it");
    let t = Instant::now();
    cmd::admin_call(&root_ctx, &acct("tusd"), "mint", &json!({"account_id": "test.near", "amount": (100 * ONE_TUSD).to_string()}).to_string(), 0, 30)?;
    cmd::storage_deposit(&root_ctx, "1")?;
    let pm = practice_manifest(&dep_p, &evm_a, anchor_n, anchor_h, "test.near", now + 3_600)?;
    let pm_path = a.work.join("practice-manifest.json");
    std::fs::write(&pm_path, pm.canonical())?;
    let practice_key = a.work.join("keys/practice.x25519.json");
    cmd::keygen_x25519(&practice_key)?;
    let pc = cmd::program_create(&root_ctx, &pm_path, PRACTICE_ESCROW_TUSD, &practice_key, None)?;
    let ppid = pc["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no practice program id: {pc}"))? as u32;
    let pact = cmd::program_activate(&root_ctx, ppid, true)?;
    wait_t1(&root_ctx, pact["run_id"].as_u64().unwrap_or_default())?;
    cmd::program_open(&root_ctx, ppid)?;
    cmd::admin_create_account(&root_ctx, &acct("practice"), "40", None, Some(&keys.join(format!("{}.json", acct("practice")))))?;
    // The sandbox's gas price is ten times testnet's (1e9 yoctoNEAR per gas): a practice account needs
    // 0.5 NEAR to prepay its 300 Tgas reveal here, where 0.05 does on testnet. Enough vends for the
    // measured agent runs (`--serve`).
    let terms = practice::Terms { gas_funds: ONE_NEAR / 2, allowance: ONE_NEAR / 2, bond_budget: ONE_TUSD / 10, per_commit_cap: ONE_TUSD / 10, per_hour: 30 };
    let vending_wasm = a.root.join("contracts/practice-vending/target/wasm32-unknown-unknown/release/antibody_practice_vending.wasm");
    let vending = practice::deploy(&ctx_of("practice"), &vending_wasm, "test.near", ppid, 24, &terms, 5 * ONE_NEAR)?;
    let funded = practice::fund(&root_ctx, &acct("practice"), 2 * ONE_NEAR, 2 * ONE_TUSD)?;
    println!("practice program {ppid} open; vending {} with the public vend key {}", acct("practice"), vending["public_vend_key"]);
    // Judges: no account, no key and no Ethereum RPC. The public vend key calls `vend`, and the proof is
    // sealed against the pinned base witness alone.
    let judge = Ctx { near: near.clone(), dep: dep.clone(), account: None, key_file: None, eth_rpc: None, state_dir: a.work.join("state/judges") };
    let mut tries = Vec::new();
    for name in ["ann", "bob"] {
        let r = practice::try_it(&judge, &acct("practice"), ppid, name)?;
        println!("{}: vault {} ({}), witness from the {} ({} bytes), {} {}, {} in {:.1}s", r["account"], r["vault_index"], r["vault"],
                 r["witness_source"].as_str().unwrap_or("-"), r["witness_bytes"], r["verdict"]["kind"].as_str().unwrap_or("-"),
                 r["t1"]["tier"].as_str().unwrap_or("-"), r["state"].as_str().unwrap_or("-"), r["seconds"].as_f64().unwrap_or_default());
        tries.push(r);
    }
    let practice_ok = tries.iter().all(|r| r["state"] == json!("Done:Paid") && r["paid_raw"] == json!(ONE_TUSD.to_string()))
        && tries[0]["vault"] != tries[1]["vault"]
        && tries.iter().all(|r| r["vended_by"] == json!(acct("practice")) && r["witness_source"] == json!("base"));
    results["practice"] = json!({"program_id": ppid, "create_tx": pc["tx"]["tx"], "manifest_sha256": hex::encode(sha256(&pm.canonical())),
                                 "vending": vending, "fund": funded, "tries": tries, "ok": practice_ok, "seconds": t.elapsed().as_secs_f64()});
    step("Fork mode: the practice program again, deployed only inside the verifier (nothing of it is on chain)");
    let t = Instant::now();
    let fm = fork_practice_manifest(&a, &root_ctx, anchor_n, anchor_h, "test.near", now + 3_600)?;
    let fm_path = a.work.join("practice-fork-manifest.json");
    std::fs::write(&fm_path, fm.canonical())?;
    let fork_key = a.work.join("keys/practice-fork.x25519.json");
    cmd::keygen_x25519(&fork_key)?;
    let fc = cmd::program_create(&root_ctx, &fm_path, PRACTICE_ESCROW_TUSD, &fork_key, None)?;
    let fpid = fc["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no fork-mode program id: {fc}"))? as u32;
    let fact = cmd::program_activate(&root_ctx, fpid, true)?;
    wait_t1(&root_ctx, fact["run_id"].as_u64().unwrap_or_default())?;
    cmd::program_open(&root_ctx, fpid)?;
    let vault3 = fm.targets.iter().find(|x| x.name == "vault-3").map(|x| format!("{:#x}", x.address)).ok_or_else(|| eyre!("no vault-3"))?;
    let on_chain = evm_a.call("eth_getCode", json!([vault3, "latest"]))?;
    let fork_bundle = a.work.join("fork-tutorial.bundle.json");
    std::fs::write(&fork_bundle, serde_json::to_vec_pretty(&practice::tutorial_bundle(&vault3))?)?;
    let fr = cmd::submit(&ctx_of("agent-a"), fpid, &fork_bundle, None, None, true)?;
    let fcid = fr["commit"]["commit_id"].as_u64().ok_or_else(|| eyre!("no fork-mode commit id: {fr}"))?;
    // The reveal returns the run at T0; the T1 relayer confirms it after.
    let frun = wait_t1(&root_ctx, root_ctx.commit(fcid)?["run"].as_u64().unwrap_or_default())?;
    let fkeeper = cmd::keeper(&ctx_of("keeper"), fpid)?;
    let fcommit = root_ctx.commit(fcid)?;
    let fork_ok = cmd::state_label(&fcommit["state"]) == "Done:Paid"
        && cmd::paid_of(&fcommit) == ONE_TUSD
        && frun["kind"] == json!("Violation")
        && frun["tier"] == json!("T1")
        && on_chain == json!("0x")
        && fr["commit"]["witness_source"] == json!("base");
    println!("fork-mode program {fpid}: {} deploy transactions, {} targets, vault-3 on chain: {on_chain}; the tutorial proof: {} {} ({} witness), {} in {:.1}s",
             fm.deploy.as_ref().map(|d| d.txs.len()).unwrap_or(0), fm.targets.len(), frun["kind"].as_str().unwrap_or("-"),
             frun["tier"].as_str().unwrap_or("-"), fr["commit"]["witness_source"].as_str().unwrap_or("-"),
             cmd::state_label(&fcommit["state"]), t.elapsed().as_secs_f64());
    results["fork_mode"] = json!({"program_id": fpid, "manifest_sha256": hex::encode(sha256(&fm.canonical())), "create_tx": fc["tx"]["tx"],
                                  "deploy_txs": fm.deploy.as_ref().map(|d| d.txs.len()), "targets": fm.targets.len(), "vault_3": vault3,
                                  "vault_3_code_on_chain": on_chain, "commit": fcid, "verdict": fr["reveal"]["result"], "tier": frun["tier"], "witness_source": fr["commit"]["witness_source"],
                                  "state": cmd::state_label(&fcommit["state"]), "paid_raw": cmd::paid_of(&fcommit).to_string(), "keeper": fkeeper,
                                  "txs": {"commit": fr["commit"]["tx"]["tx"], "reveal": fr["reveal"]["tx"]["tx"]}, "ok": fork_ok, "seconds": t.elapsed().as_secs_f64()});

    // The deployments file names the practice program, so `antibody try --name <you>` needs nothing else.
    let mut served = dep.clone();
    served.practice = Some(antibody_cli::ctx::PracticeDeployment { vending: acct("practice"), program: ppid });
    std::fs::write(&dep_file, serde_json::to_vec_pretty(&served)?)?;
    if !a.serve {
        stop.store(true, Ordering::Relaxed);
        for h in std::mem::take(&mut handles) {
            let _ = h.join();
        }
    }

    step("Results");
    let expected: BTreeMap<&str, (&str, &str, u128)> = BTreeMap::from([
        ("A (MCP)", ("Violation", "Done:Paid", 10_000 * ONE_TUSD)),
        ("B", ("Violation", "Done:Paid", 3_000 * ONE_TUSD)),
        ("C", ("NoViolation", "Done:Forfeited", 0)),
        ("D", ("Violation", "Done:DuplicateConfirmed", 0)),
        ("E", ("Violation", "Done:Known", 0)),
        ("F", ("Violation", "Done:Paid", 10_000 * ONE_TUSD)),
        ("G", ("NoViolation", "Done:Paid", 1_000 * ONE_TUSD)),
    ]);
    let mut table = Vec::new();
    let one = &results["one_moment"];
    let mut ok = known_n == 1 && after == before + 3_000 * ONE_TUSD && same && one["paused_both"] == json!(true) && one["instant"] == json!(true);
    // Round 1 is shut (B and F carried), round 2 releases them both.
    let gate = |r: &Value| r["opened"]["release_gate"].as_bool();
    ok &= gate(&results["rounds"][0]) == Some(false) && results["rounds"][0]["opened"]["carried"] == json!(2);
    ok &= gate(&results["rounds"][1]) == Some(true) && results["rounds"][1]["opened"]["released"] == json!(2);
    ok &= results["release"]["ok"] == json!(true);
    ok &= results["practice"]["ok"] == json!(true);
    ok &= results["fork_mode"]["ok"] == json!(true);
    ok &= results["verdict_consumer"]["ok"] == json!(true);
    ok &= results["project_read"]["ok"] == json!(true);
    println!("{:<8} {:>6} {:<12} {:<5} {:<34} {:<15} {:>10} {:>8}", "agent", "commit", "verdict", "tier", "keys (slot: tag)", "state", "paid tUSD", "seconds");
    let mut pays = BTreeMap::new();
    pay_txs(&results, &mut pays);
    for (agent, id, secs, txs) in &rows {
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
        let mut txs = txs.clone();
        txs["pay"] = pays.get(id).cloned().unwrap_or(Value::Null);
        table.push(json!({"agent": agent, "commit": id, "run": c["run"], "verdict": kind, "tier": v["tier"], "keys": v["keys"], "state": c["state"],
                          "paid_raw": paid.to_string(), "seconds": secs, "txs": txs, "ok": pass}));
    }
    let audit = root_ctx.view("audit", json!({"program_id": pid}))?;
    let contract_audit = root_ctx.view("audit_contract", json!({}))?;
    let practice_audit = root_ctx.view("audit", json!({"program_id": results["practice"]["program_id"]}))?;
    let fork_audit = root_ctx.view("audit", json!({"program_id": results["fork_mode"]["program_id"]}))?;
    ok &= audit["ok"] == json!(true) && contract_audit["ok"] == json!(true) && practice_audit["ok"] == json!(true) && fork_audit["ok"] == json!(true);
    println!("known issues: {known_n}; audit(program): {}; audit(practice): {}; audit(contract): {}; B withdrew 3000 tUSD: {}; A paid in seconds (T1, under the cap): {}; paused on both chains: {}; practice: {}",
             audit["ok"], practice_audit["ok"], contract_audit["ok"], after == before + 3_000 * ONE_TUSD, one["instant"], one["paused_both"], results["practice"]["ok"]);
    results["submissions"] = Value::Array(table);
    results["audit"] = json!({"program": audit, "practice": practice_audit, "fork_mode": fork_audit, "contract": contract_audit});
    results["record"] = root_ctx.view("program_record", json!({"program_id": pid}))?;
    let judged = antibody_cli::judge::judge(&judge, None)?;
    let judge_ok = judged["programs"].as_array().map(|p| p.len()) == Some(3)
        && judged["programs"][0]["verdicts"]["t1_confirmed"].as_u64() >= Some(7)
        && judged["programs"][0]["pauses"].as_u64() == Some(2)
        && judged["programs"][0]["rule_strength"]["statement"].is_string()
        && judged["programs"][1]["payouts"].as_array().map(|p| p.len()) == Some(2)
        && judged["programs"][2]["payouts"].as_array().map(|p| p.len()) == Some(1);
    println!("judge (view calls only): {} programs, {} pause permits, {} release permits; ok: {judge_ok}", judged["programs"].as_array().map(|p| p.len()).unwrap_or(0),
             judged["programs"][0]["pauses"], judged["programs"][0]["releases"]);
    ok &= judge_ok;
    results["judge"] = judged;
    // A versioned deployment record, as `antibody admin record-deployment` writes it.
    let record = antibody_cli::judge::deployment_record(&judge, None)?;
    ok &= record["contracts"].as_array().map(|c| c.len()) == Some(4) && record["verifier"]["wasm_hash"].is_string();
    results["deployment_record"] = record;

    step("API: the indexer, running since the start, has every block's events; each endpoint answers once (docs/openapi.yaml's examples)");
    let head = near.call("block", json!({"finality": "final"}))?["header"]["height"].as_u64().unwrap_or(0);
    wait_for("the indexer at the chain head", Duration::from_secs(300), || {
        serde_json::from_str::<Value>(&std::fs::read_to_string(services_dir.join("cursor.json")).unwrap_or_default()).unwrap_or(Value::Null)["last_block"]
            .as_u64()
            .unwrap_or(0)
            >= head
    })?;
    let indexed = crate::indexer::read_events(&services_dir.join("events.jsonl"), 0, usize::MAX).len();
    let api = crate::api::Api { near: near.clone(), dep: served.clone(), state_dir: services_dir.clone() };
    let rt = tokio::runtime::Runtime::new()?;
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))?;
    let addr = listener.local_addr()?;
    rt.spawn(async move { axum::serve(listener, crate::api::router(api)).await });
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(120)).build()?;
    let mut examples = serde_json::Map::new();
    let paths = vec![
        "/health".to_string(), "/v1/status".into(), "/v1/stats".into(), "/v1/programs".into(), format!("/v1/programs/{pid}"),
        format!("/v1/programs/{ppid}/feed"), format!("/v1/commits/{a_commit}"), "/v1/runs/0".into(), format!("/v1/accounts/{}", acct("agent-a")),
        "/v1/leaderboard".into(), "/v1/events?limit=5".into(), "/v1/guardian".into(), "/v1/judge".into(),
        format!("/v1/programs/{pid}/reflex"), "/v1/trust".into(), "/v1/practice".into(), "/v1/mainnet".into(),
    ];
    for path in &paths {
        let r = client.get(format!("http://{addr}{path}")).send()?;
        let code = r.status().as_u16();
        examples.insert(path.clone(), json!({"status": code, "body": r.json::<Value>().unwrap_or(Value::Null)}));
    }
    // A project finds a commit's blob through the API alone (the wrap exists: no new run).
    let via_api = cmd::program_read(&acme, a_commit, &project_key, &a.work.join("read/A-via-api"), &cmd::BlobFrom::Api(format!("http://{addr}")))?;
    let api_ok = examples.values().all(|e| e["status"] == json!(200)) && indexed > 0 && via_api["receipt_matches_the_tee"] == json!(true);
    println!("API: {} events indexed; {} endpoints, all 200; the project's read through the API: {}; ok: {api_ok}", indexed, examples.len(),
             via_api["local_verdict"]["kind"]);
    std::fs::write(a.work.join("api-examples.json"), serde_json::to_vec_pretty(&examples)?)?;
    results["api"] = json!({"events_indexed": indexed, "endpoints": paths, "ok": api_ok, "examples": a.work.join("api-examples.json"), "url": format!("http://{addr}"),
                            "project_read": {"receipt_matches_the_tee": via_api["receipt_matches_the_tee"], "requested": via_api["requested"]},
                            "failures": examples.iter().filter(|(_, e)| e["status"] != json!(200)).map(|(k, e)| json!({"path": k, "status": e["status"], "body": e["body"]})).collect::<Vec<_>>()});
    ok &= api_ok;
    if !a.serve {
        ix_stop.store(true, Ordering::Relaxed);
        let _ = ix_handle.join();
    }
    results["all_ok"] = ok.into();
    results["seconds"] = t_all.elapsed().as_secs_f64().into();
    std::fs::write(a.work.join("results.json"), serde_json::to_vec_pretty(&results)?)?;
    if a.serve {
        println!("--serve: near-sandbox http://127.0.0.1:3030, anvil :8545 and :8546, the API http://{addr}, the MOCK services and relayers stay up;");
        println!("  deployments {} (practice program {ppid}, vending {}); Ctrl-C stops", dep_file.display(), acct("practice"));
        std::thread::park();
    } else if a.keep {
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

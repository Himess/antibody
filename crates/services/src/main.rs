//! `antibody-services`: the backend around the contracts.
//!
//! - `indexer`  — NEP-297 event history with tx links + state snapshot + latency metrics
//! - `api`      — read-only JSON API (programs, feed, commits with duplicate proofs, leaderboard,
//!                guardian/relayer status, events, MOCK mainnet data)
//! - `relayer`  — submits the guardian's signed permits to each chain's adapter (its own EVM key)
//! - `notifier` — posts new events and monitoring alerts to webhooks
//! - `keeper`   — the permissionless settlement steps on every open program, in a loop
//! - `all`      — indexer + relayer + notifier + api (+ keeper and T1 relayer with their accounts)
//! - `t1`       — relays each answered run's quote to near-antibody (T1)
//! - `worker-mock`, `signer-mock` — sandbox stand-ins for OutLayer and the MPC network
//! - `demo`     — the offline end-to-end run on near-sandbox and anvil

mod api;
mod demo;
mod evm;
mod indexer;
mod keeper;
mod live;
mod mocks;
mod notifier;
mod relayer;
mod status;
mod t1;

use std::{collections::BTreeMap, path::PathBuf};

use antibody_cli::{
    ctx::{repo_root, Deployments},
    near::{Near, Signer},
};
use clap::{Parser, Subcommand};
use eyre::{eyre, Result};

#[derive(Parser)]
#[command(name = "antibody-services", version, about = "Antibody backend: indexer, API, relayer, notifier, sandbox demo")]
struct Cli {
    #[arg(long, global = true, default_value = "testnet")]
    network: String,
    #[arg(long, global = true)]
    deployments: Option<PathBuf>,
    /// NEAR RPC (default: $NEAR_RPC or the deployments file)
    #[arg(long, global = true)]
    rpc: Option<String>,
    /// Where the services keep their state (events.jsonl, snapshot.json, relayer.json, …)
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Indexer {
        /// fastnear (testnet/mainnet tx API) or blocks (any RPC)
        #[arg(long, default_value = "fastnear")]
        source: String,
        #[arg(long, default_value_t = 0)]
        from_block: u64,
        #[arg(long, default_value_t = 5000)]
        interval_ms: u64,
        #[arg(long)]
        once: bool,
    },
    Api {
        #[arg(long, default_value = "127.0.0.1:8787")]
        listen: String,
    },
    /// chain id = RPC URL (e.g. --chain 11155111=$SEPOLIA_RPC); defaults from $SEPOLIA_RPC/$BASE_SEPOLIA_RPC
    Relayer {
        #[arg(long = "chain")]
        chains: Vec<String>,
        #[arg(long, default_value_t = 3000)]
        interval_ms: u64,
        #[arg(long)]
        once: bool,
    },
    /// Webhook URLs come from $ANTIBODY_WEBHOOKS (comma-separated; never printed)
    Notifier {
        #[arg(long)]
        program: Option<u64>,
        #[arg(long, default_value_t = 5000)]
        interval_ms: u64,
    },
    All {
        #[arg(long, default_value = "127.0.0.1:8787")]
        listen: String,
        #[arg(long = "chain")]
        chains: Vec<String>,
        #[arg(long, default_value = "fastnear")]
        source: String,
        /// Run the keeper bot as this account (its key: --keeper-key-file, or .secrets/near/<account>.json)
        #[arg(long)]
        keeper_account: Option<String>,
        #[arg(long)]
        keeper_key_file: Option<PathBuf>,
        /// Run the T1 relayer as this account
        #[arg(long)]
        t1_account: Option<String>,
        #[arg(long)]
        t1_key_file: Option<PathBuf>,
        /// The T1 relayer's DCAP collateral: OutLayer's on-chain copies, fresh Intel PCS collateral, or
        /// both (auto: the copies while they verify, Intel PCS once they expire)
        #[arg(long, default_value = "auto", value_parser = ["auto", "outlayer", "pcs"])]
        t1_collateral: String,
        /// The T1 relayer gives up on a run after this many failed relays, 10 s apart: enough to ride out
        /// a rate-limited RPC or an attestation OutLayer publishes late (a practice try waits 300 s)
        #[arg(long, default_value_t = 60)]
        t1_max_attempts: u32,
    },
    /// The keeper bot: allocate, pay, pause tickets, debts, re-runs and pending permits on every open
    /// program (or --program), every --interval-ms. Pays gas only
    Keeper {
        #[arg(long)]
        account: String,
        #[arg(long)]
        key_file: Option<PathBuf>,
        #[arg(long = "program")]
        programs: Vec<u32>,
        #[arg(long, default_value_t = 30_000)]
        interval_ms: u64,
        #[arg(long)]
        once: bool,
    },
    /// T1 relayer: relays the quote of every answered run to near-antibody (`confirm_tee`). On
    /// testnet it needs the indexer's events; with --mock-outlayer (sandbox) it relays MOCK
    /// attestations. Pays gas only
    T1 {
        #[arg(long)]
        account: String,
        #[arg(long)]
        key_file: Option<PathBuf>,
        #[arg(long, default_value = antibody_cli::tee::DEFAULT_API)]
        api: String,
        /// Sandbox only: the mock-outlayer account whose MOCK attestations to relay
        #[arg(long)]
        mock_outlayer: Option<String>,
        #[arg(long, default_value_t = 10000)]
        interval_ms: u64,
        #[arg(long, default_value_t = 5)]
        max_attempts: u32,
        /// DCAP collateral: OutLayer's on-chain copies, fresh Intel PCS collateral, or both (auto)
        #[arg(long, default_value = "auto", value_parser = ["auto", "outlayer", "pcs"])]
        collateral: String,
        #[arg(long)]
        once: bool,
    },
    /// Sandbox only: resolve MOCK OutLayer requests with the verifier in TEE mode (TEST seed)
    WorkerMock {
        #[arg(long)]
        outlayer: String,
        #[arg(long)]
        key_file: PathBuf,
    },
    /// Sandbox only: answer MOCK MPC sign requests with the TEST key
    SignerMock {
        #[arg(long)]
        mpc: String,
        #[arg(long)]
        key_file: PathBuf,
    },
    /// Offline end-to-end run: near-sandbox and anvil, every expected outcome asserted
    Demo {
        #[arg(long)]
        sandbox_bin: Option<PathBuf>,
        #[arg(long)]
        work_dir: Option<PathBuf>,
        /// Keep the chains running at the end (for exploring with the CLI/API)
        #[arg(long)]
        keep: bool,
        /// Keep the chains and the MOCK services running at the end, so the practice program can be
        /// tried (by people or agents) against the sandbox; Ctrl-C stops
        #[arg(long)]
        serve: bool,
    },
    /// The story on NEAR testnet with Sepolia and Base Sepolia: our own deployment
    /// (deployments/near-testnet.json) and fixtures, every transaction on a testnet. Keys from
    /// `.secrets/`, RPCs from `.env` (SEPOLIA_RPC: endpoints that prove recent state, comma-separated;
    /// SEPOLIA_TX_RPC, optional: transactions and current state; BASE_SEPOLIA_RPC). Resumable: `<work
    /// dir>/live.json` records each stage
    Live {
        #[arg(long)]
        work_dir: Option<PathBuf>,
        /// Stop after this stage
        #[arg(long)]
        until: Option<String>,
        /// Run the practice and fork-mode stages again, on new programs at a new anchor
        #[arg(long)]
        redo_practice: bool,
        /// The EVM deployer's key file (default .secrets/sepolia_deployer.json): a new deployer deploys
        /// the fixtures at new addresses
        #[arg(long)]
        evm_key: Option<PathBuf>,
    },
}

fn load_dep(cli: &Cli) -> Result<Deployments> {
    let path = cli
        .deployments
        .clone()
        .or_else(|| std::env::var("ANTIBODY_DEPLOYMENTS").ok().map(PathBuf::from))
        .or_else(|| repo_root().map(|r| r.join(format!("deployments/near-{}.json", cli.network))))
        .ok_or_else(|| eyre!("no deployments file"))?;
    Ok(serde_json::from_str(&std::fs::read_to_string(&path).map_err(|e| eyre!("{}: {e}", path.display()))?)?)
}

fn near_for(cli: &Cli, dep: &Deployments) -> Near {
    Near::new(&cli.network, cli.rpc.clone().or_else(|| std::env::var("NEAR_RPC").ok()).or_else(|| dep.rpc.clone()))
}

fn state_dir(cli: &Cli) -> PathBuf {
    cli.state_dir.clone().unwrap_or_else(|| PathBuf::from(format!(".antibody-services/{}", cli.network)))
}

fn chains(args: &[String]) -> Result<BTreeMap<u64, (String, String)>> {
    let mut m = BTreeMap::new();
    for a in args {
        let (id, url) = a.split_once('=').ok_or_else(|| eyre!("--chain expects <chain id>=<rpc url>"))?;
        m.insert(id.parse()?, (url.to_string(), explorer(id.parse()?)));
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if m.is_empty() {
        if let Some(u) = env("SEPOLIA_RPC") {
            m.insert(11155111, (u, explorer(11155111)));
        }
        if let Some(u) = env("BASE_SEPOLIA_RPC") {
            m.insert(84532, (u, explorer(84532)));
        }
    }
    Ok(m)
}

/// A run's work directory, absolute: forge runs in contracts/evm and may write only under target/demo and
/// target/live (contracts/evm/foundry.toml), so a relative path would point elsewhere.
fn work_dir_or(given: &Option<PathBuf>, default: PathBuf) -> Result<PathBuf> {
    Ok(match given {
        Some(w) if w.is_relative() => std::env::current_dir()?.join(w),
        Some(w) => w.clone(),
        None => default,
    })
}

/// The relayer's EVM key, from $RELAYER_EVM_KEY or the repository's .env (never printed). Without it
/// the relayer only watches.
fn relayer_signer() -> Result<Option<evm::EvmSigner>> {
    let from_env = std::env::var("RELAYER_EVM_KEY").ok().filter(|v| !v.is_empty());
    let from_file = || {
        repo_root().and_then(|r| antibody_cli::ctx::load_dotenv(&r.join(".env")).into_iter().find(|(k, _)| k == "RELAYER_EVM_KEY").map(|(_, v)| v))
    };
    from_env.or_else(from_file).map(|k| evm::EvmSigner::from_hex(&k)).transpose()
}

fn explorer(chain_id: u64) -> String {
    match chain_id {
        11155111 => "https://sepolia.etherscan.io/tx/".into(),
        84532 => "https://sepolia.basescan.org/tx/".into(),
        other => format!("chain-{other}:"),
    }
}

fn indexer(cli: &Cli, dep: &Deployments, source: &str, from_block: u64) -> Result<indexer::Indexer> {
    let near = near_for(cli, dep);
    let mut contracts = vec![dep.antibody.clone(), dep.token.clone()];
    contracts.extend(dep.guardian.clone());
    let source = match source {
        "fastnear" => indexer::Source::FastNear {
            base: if cli.network == "mainnet" { "https://tx.main.fastnear.com".into() } else { "https://tx.test.fastnear.com".into() },
        },
        "blocks" => indexer::Source::Blocks { from: from_block },
        other => return Err(eyre!("unknown source {other}")),
    };
    Ok(indexer::Indexer { near, contracts, antibody: dep.antibody.clone(), guardian: dep.guardian.clone(), source, state_dir: state_dir(cli) })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.cmd {
        Cmd::Demo { sandbox_bin, work_dir, keep, serve } => {
            let root = repo_root().ok_or_else(|| eyre!("run the demo from inside the repository"))?;
            let exe = if cfg!(windows) { ".exe" } else { "" };
            let tool = |name: &str| -> PathBuf {
                let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
                let foundry = home.join(".foundry/bin").join(format!("{name}{exe}"));
                if foundry.exists() { foundry } else { PathBuf::from(format!("{name}{exe}")) }
            };
            let sandbox = sandbox_bin
                .clone()
                .or_else(|| std::env::var_os("NEAR_SANDBOX_BIN").map(PathBuf::from))
                .unwrap_or_else(|| root.join(".tools/near-sandbox"));
            demo::run(demo::DemoArgs {
                work: work_dir_or(work_dir, root.join("target/demo"))?,
                sandbox_bin: sandbox,
                anvil_bin: tool("anvil"),
                forge_bin: tool("forge"),
                antibody_bin: root.join(format!("target/release/antibody{exe}")),
                root,
                keep: *keep,
                serve: *serve,
            })?;
            Ok(())
        }
        Cmd::Live { work_dir, until, redo_practice, evm_key } => {
            let root = repo_root().ok_or_else(|| eyre!("run it from inside the repository"))?;
            let dotenv = antibody_cli::ctx::load_dotenv(&root.join(".env"));
            let var = |k: &str| -> Result<String> {
                std::env::var(k)
                    .ok()
                    .filter(|v| !v.is_empty())
                    .or_else(|| dotenv.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()))
                    .ok_or_else(|| eyre!("set {k} in .env"))
            };
            let exe = if cfg!(windows) { ".exe" } else { "" };
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
            let forge = home.join(".foundry/bin").join(format!("forge{exe}"));
            live::run(live::LiveArgs {
                work: work_dir_or(work_dir, root.join("target/live"))?,
                forge_bin: if forge.exists() { forge } else { PathBuf::from(format!("forge{exe}")) },
                antibody_bin: root.join(format!("target/release/antibody{exe}")),
                keys: root.join(".secrets/near"),
                evm_key: evm_key.clone().unwrap_or_else(|| root.join(".secrets/sepolia_deployer.json")),
                sepolia_tx_rpc: var("SEPOLIA_TX_RPC").or_else(|_| var("SEPOLIA_RPC").map(|v| v.split(',').next().unwrap_or_default().to_string()))?,
                sepolia_rpc: var("SEPOLIA_RPC")?,
                base_rpc: var("BASE_SEPOLIA_RPC")?,
                until: until.clone(),
                redo_practice: *redo_practice,
                root,
            })?;
            Ok(())
        }
        Cmd::WorkerMock { outlayer, key_file } => {
            let dep = load_dep(&cli)?;
            let mut w = mocks::MockWorker {
                near: near_for(&cli, &dep),
                outlayer: outlayer.clone(),
                worker: Signer::from_file(key_file)?,
                seed: mocks::demo_seed(),
                done: Default::default(),
            };
            loop {
                if let Err(e) = w.tick() {
                    eprintln!("worker-mock: {e:#}");
                }
                std::thread::sleep(std::time::Duration::from_millis(700));
            }
        }
        Cmd::SignerMock { mpc, key_file } => {
            let dep = load_dep(&cli)?;
            let mut s = mocks::MockSigner {
                near: near_for(&cli, &dep),
                mpc: mpc.clone(),
                signer: Signer::from_file(key_file)?,
                key: mocks::demo_mpc_key(),
                done: Default::default(),
            };
            loop {
                if let Err(e) = s.tick() {
                    eprintln!("signer-mock: {e:#}");
                }
                std::thread::sleep(std::time::Duration::from_millis(700));
            }
        }
        Cmd::Indexer { source, from_block, interval_ms, once } => {
            let dep = load_dep(&cli)?;
            let ix = indexer(&cli, &dep, source, *from_block)?;
            if *once {
                let ev = ix.tick()?;
                println!("{}", serde_json::json!({"new_events": ev.len(), "state_dir": ix.state_dir}));
                return Ok(());
            }
            ix.run(*interval_ms)
        }
        Cmd::Relayer { chains: c, interval_ms, once } => {
            let dep = load_dep(&cli)?;
            let r = relayer::Relayer {
                near: near_for(&cli, &dep),
                guardian: dep.guardian.clone().ok_or_else(|| eyre!("no guardian in deployments"))?,
                antibody: dep.antibody.clone(),
                chains: chains(c)?,
                signer: relayer_signer()?,
                state_file: state_dir(&cli).join("relayer.json"),
            };
            if *once {
                println!("{}", serde_json::to_string_pretty(&r.tick()?)?);
                return Ok(());
            }
            r.run(*interval_ms)
        }
        Cmd::T1 { account, key_file, api, mock_outlayer, interval_ms, max_attempts, collateral, once } => {
            let ctx = antibody_cli::ctx::Ctx::new(&cli.network, cli.deployments.clone(), cli.rpc.clone(), Some(account.clone()), key_file.clone(), None)?;
            let source = match mock_outlayer {
                Some(o) => t1::Source::Mock { outlayer: o.clone() },
                None => t1::Source::OutLayer { api: api.clone() },
            };
            let r = t1::T1Relayer { ctx, source, state_dir: state_dir(&cli), max_attempts: *max_attempts, collateral: collateral.clone() };
            if *once {
                println!("{}", serde_json::to_string_pretty(&r.tick()?)?);
                Ok(())
            } else {
                r.run(*interval_ms)
            }
        }
        Cmd::Notifier { program, interval_ms } => {
            let webhooks: Vec<String> = std::env::var("ANTIBODY_WEBHOOKS").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(String::from).collect();
            let status = load_dep(&cli).ok().map(|d| (near_for(&cli, &d), d));
            notifier::Notifier { state_dir: state_dir(&cli), webhooks, program: *program, status }.run(*interval_ms)
        }
        Cmd::Api { listen } => {
            let dep = load_dep(&cli)?;
            let api = api::Api {
                near: near_for(&cli, &dep),
                dep,
                state_dir: state_dir(&cli),
            };
            tokio::runtime::Runtime::new()?.block_on(api::serve(api, listen))
        }
        Cmd::Keeper { account, key_file, programs, interval_ms, once } => {
            let ctx = antibody_cli::ctx::Ctx::new(&cli.network, cli.deployments.clone(), cli.rpc.clone(), Some(account.clone()), key_file.clone(), None)?;
            let k = keeper::Keeper { ctx, programs: programs.clone(), state_file: state_dir(&cli).join("keeper.json") };
            if *once {
                println!("{}", serde_json::to_string_pretty(&k.tick()?)?);
                Ok(())
            } else {
                k.run(*interval_ms)
            }
        }
        Cmd::All { listen, chains: c, source, keeper_account, keeper_key_file, t1_account, t1_key_file, t1_collateral, t1_max_attempts } => {
            let dep = load_dep(&cli)?;
            let ix = indexer(&cli, &dep, source, 0)?;
            std::thread::spawn(move || ix.run(5000));
            if let Some(g) = dep.guardian.clone() {
                let r = relayer::Relayer {
                    near: near_for(&cli, &dep),
                    guardian: g,
                    antibody: dep.antibody.clone(),
                    chains: chains(c)?,
                    signer: relayer_signer()?,
                    state_file: state_dir(&cli).join("relayer.json"),
                };
                std::thread::spawn(move || r.run(3000));
            }
            let webhooks: Vec<String> = std::env::var("ANTIBODY_WEBHOOKS").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(String::from).collect();
            let n = notifier::Notifier { state_dir: state_dir(&cli), webhooks, program: None, status: Some((near_for(&cli, &dep), dep.clone())) };
            std::thread::spawn(move || n.run(5000));
            if let Some(acct) = keeper_account {
                let ctx = antibody_cli::ctx::Ctx::new(&cli.network, cli.deployments.clone(), cli.rpc.clone(), Some(acct.clone()), keeper_key_file.clone(), None)?;
                let k = keeper::Keeper { ctx, programs: vec![], state_file: state_dir(&cli).join("keeper.json") };
                std::thread::spawn(move || k.run(30_000));
            }
            if let Some(acct) = t1_account {
                let ctx = antibody_cli::ctx::Ctx::new(&cli.network, cli.deployments.clone(), cli.rpc.clone(), Some(acct.clone()), t1_key_file.clone(), None)?;
                let r = t1::T1Relayer {
                    ctx,
                    source: t1::Source::OutLayer { api: antibody_cli::tee::DEFAULT_API.into() },
                    state_dir: state_dir(&cli),
                    max_attempts: *t1_max_attempts,
                    collateral: t1_collateral.clone(),
                };
                std::thread::spawn(move || loop {
                    if let Err(e) = r.tick() {
                        eprintln!("t1 relayer: {e:#}");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10_000));
                });
            }
            let api = api::Api {
                near: near_for(&cli, &dep),
                dep,
                state_dir: state_dir(&cli),
            };
            tokio::runtime::Runtime::new()?.block_on(api::serve(api, listen))
        }
    }
}

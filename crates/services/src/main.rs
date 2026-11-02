//! `antibody-services`: the backend around the contracts.
//!
//! - `indexer`  — NEP-297 event history with tx links + state snapshot + latency metrics
//! - `api`      — read-only JSON API (programs, feed, commits with duplicate proofs, leaderboard,
//!                guardian/relayer status, events, MOCK mainnet data)
//! - `relayer`  — broadcasts the guardian's signed pause() transactions to every chain
//! - `notifier` — posts new events to webhooks
//! - `all`      — indexer + relayer + notifier + api in one process
//! - `t1`       — relays each answered run's quote to near-antibody (T1)
//! - `worker-mock`, `signer-mock` — sandbox stand-ins for OutLayer and the MPC network
//! - `demo`     — the offline end-to-end run on near-sandbox and anvil

mod api;
mod demo;
mod evm;
mod indexer;
mod mocks;
mod notifier;
mod relayer;
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
        #[arg(long)]
        mock_dir: Option<PathBuf>,
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

fn explorer(chain_id: u64) -> String {
    match chain_id {
        11155111 => "https://sepolia.etherscan.io/tx/".into(),
        84532 => "https://sepolia.basescan.org/tx/".into(),
        other => format!("chain-{other}:"),
    }
}

fn indexer(cli: &Cli, dep: &Deployments, source: &str, from_block: u64) -> Result<indexer::Indexer> {
    let near = near_for(cli, dep);
    let mut contracts = vec![dep.antibody.clone(), dep.tusd.clone()];
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
        Cmd::Demo { sandbox_bin, work_dir, keep } => {
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
                work: work_dir.clone().unwrap_or_else(|| root.join("target/demo")),
                sandbox_bin: sandbox,
                anvil_bin: tool("anvil"),
                forge_bin: tool("forge"),
                antibody_bin: root.join(format!("target/release/antibody{exe}")),
                root,
                keep: *keep,
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
                chains: chains(c)?,
                state_file: state_dir(&cli).join("relayer.json"),
            };
            if *once {
                println!("{}", serde_json::to_string_pretty(&r.tick()?)?);
                return Ok(());
            }
            r.run(*interval_ms)
        }
        Cmd::T1 { account, key_file, api, mock_outlayer, interval_ms, max_attempts, once } => {
            let ctx = antibody_cli::ctx::Ctx::new(&cli.network, cli.deployments.clone(), cli.rpc.clone(), Some(account.clone()), key_file.clone(), None)?;
            let source = match mock_outlayer {
                Some(o) => t1::Source::Mock { outlayer: o.clone() },
                None => t1::Source::OutLayer { api: api.clone() },
            };
            let r = t1::T1Relayer { ctx, source, state_dir: state_dir(&cli), max_attempts: *max_attempts };
            if *once {
                println!("{}", serde_json::to_string_pretty(&r.tick()?)?);
                Ok(())
            } else {
                r.run(*interval_ms)
            }
        }
        Cmd::Notifier { program, interval_ms } => {
            let webhooks: Vec<String> = std::env::var("ANTIBODY_WEBHOOKS").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(String::from).collect();
            notifier::Notifier { state_dir: state_dir(&cli), webhooks, program: *program }.run(*interval_ms)
        }
        Cmd::Api { listen, mock_dir } => {
            let dep = load_dep(&cli)?;
            let api = api::Api {
                near: near_for(&cli, &dep),
                dep,
                state_dir: state_dir(&cli),
                mock_dir: mock_dir.clone().unwrap_or_else(|| repo_root().unwrap_or_default().join("fixtures/mock-mainnet")),
            };
            tokio::runtime::Runtime::new()?.block_on(api::serve(api, listen))
        }
        Cmd::All { listen, chains: c, source } => {
            let dep = load_dep(&cli)?;
            let ix = indexer(&cli, &dep, source, 0)?;
            std::thread::spawn(move || ix.run(5000));
            if let Some(g) = dep.guardian.clone() {
                let r = relayer::Relayer { near: near_for(&cli, &dep), guardian: g, chains: chains(c)?, state_file: state_dir(&cli).join("relayer.json") };
                std::thread::spawn(move || r.run(3000));
            }
            let webhooks: Vec<String> = std::env::var("ANTIBODY_WEBHOOKS").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(String::from).collect();
            let n = notifier::Notifier { state_dir: state_dir(&cli), webhooks, program: None };
            std::thread::spawn(move || n.run(5000));
            let api = api::Api {
                near: near_for(&cli, &dep),
                dep,
                state_dir: state_dir(&cli),
                mock_dir: repo_root().unwrap_or_default().join("fixtures/mock-mainnet"),
            };
            tokio::runtime::Runtime::new()?.block_on(api::serve(api, listen))
        }
    }
}

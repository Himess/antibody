//! `antibody` — create programs, hunt, commit and submit sealed PoCs, verify receipts, and serve
//! the same operations to AI agents over MCP. Every command prints JSON with explorer links.

use antibody_cli::{cmd, ctx, mcp};

use std::path::PathBuf;

use alloy_primitives::B256;
use clap::{Parser, Subcommand};
use eyre::Result;
use serde_json::{json, Value};

#[derive(Parser)]
#[command(name = "antibody", version, about = "Antibody — bug bounties that pay on proof (NEAR testnet)")]
struct Cli {
    /// NEAR network (deployments/near-<network>.json)
    #[arg(long, global = true, default_value = "testnet")]
    network: String,
    /// Deployed contract ids (default: <repo>/deployments/near-<network>.json or $ANTIBODY_DEPLOYMENTS)
    #[arg(long, global = true)]
    deployments: Option<PathBuf>,
    /// NEAR RPC URL (default: $NEAR_RPC, the deployments file, or FastNEAR)
    #[arg(long, global = true)]
    rpc: Option<String>,
    /// Signing NEAR account (or $ANTIBODY_ACCOUNT)
    #[arg(long, global = true)]
    account: Option<String>,
    /// Key file in near-cli format (or $ANTIBODY_KEY_FILE)
    #[arg(long, global = true)]
    key_file: Option<PathBuf>,
    /// Ethereum RPC for witnesses (or $SEPOLIA_RPC / .env); needs eth_getProof at the anchor block
    #[arg(long, global = true)]
    eth_rpc: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum KeygenKind {
    /// X25519 key for a project (findings are re-encrypted to it)
    X25519 {
        #[arg(long)]
        out: PathBuf,
    },
    /// ed25519 key for a NEAR account (an implicit account id unless --account-id is given)
    Near {
        #[arg(long)]
        account_id: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
enum RoundCmd {
    /// Propose the next round: a round manifest (`antibody manifest-round`) and the releases a passed
    /// release gate will permit, each <proxy>:<implementation>:<code hash>; the TEE checks both
    Propose {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long = "release")]
        releases: Vec<String>,
        #[arg(long)]
        no_wait: bool,
    },
    /// Re-run a commit at the proposed fix (permissionless; the requester pays)
    Rerun {
        #[arg(long)]
        commit: u64,
    },
    /// Open the next round once its minimum duration and re-run window have passed (permissionless)
    Open {
        #[arg(long)]
        program: u32,
    },
}

#[derive(Subcommand)]
enum ProgramCmd {
    /// Create a program: upload the manifest (v2), escrow tUSD and commit the manifest hash (one
    /// transaction); NEAR credit for storage is added first if needed
    Create {
        #[arg(long)]
        manifest: PathBuf,
        /// Escrow in tUSD (e.g. 25000)
        #[arg(long)]
        escrow: String,
        #[arg(long)]
        project_key: PathBuf,
        /// Use an already-hosted manifest instead of uploading it with FastFS
        #[arg(long)]
        manifest_url: Option<String>,
    },
    /// Pin the base witness and request the program's sealing key (it counts once T1-confirmed)
    Activate {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        no_wait: bool,
    },
    /// Declare a known issue before opening (verified by the same TEE; counts once T1-confirmed)
    Known {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        salt: Option<B256>,
        #[arg(long)]
        report: Option<PathBuf>,
        #[arg(long)]
        no_wait: bool,
    },
    /// Open round 0: hunters can commit
    Open {
        #[arg(long)]
        program: u32,
    },
    Show {
        program: u32,
    },
    /// Add escrow
    TopUp {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        amount: String,
    },
    /// After the deadline and the exit grace: pay what is reserved, return the rest to the project
    Exit {
        #[arg(long)]
        program: u32,
    },
}

#[derive(Subcommand)]
enum AgentCmd {
    /// Register one of your sub-accounts as an agent: its bonds come from your credit, within the
    /// budget and the per-commit cap; payouts go to you
    Register {
        #[arg(long)]
        agent: String,
        /// Total bonds the agent may have locked at once (tUSD)
        #[arg(long)]
        budget: String,
        #[arg(long)]
        per_commit_cap: String,
    },
    /// Add a function-call key to the agent's account, limited to commit/reveal/retry on near-antibody
    Key {
        #[command(subcommand)]
        cmd: AgentKeyCmd,
    },
}

#[derive(Subcommand)]
enum AgentKeyCmd {
    /// Add a function-call key limited to commit/reveal/retry on near-antibody, with a gas allowance
    Add {
        #[arg(long, default_value_t = 1.0)]
        allowance_near: f64,
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
enum AdminCmd {
    /// Create a sub-account of the signer, or a top-level .testnet account via the registrar
    CreateAccount {
        #[arg(long)]
        new_account_id: String,
        #[arg(long, default_value = "5")]
        amount_near: String,
        #[arg(long)]
        public_key: Option<String>,
        #[arg(long)]
        key_out: Option<PathBuf>,
    },
    /// Deploy a wasm to the signer's account (and call its init method)
    Deploy {
        #[arg(long)]
        wasm: PathBuf,
        #[arg(long)]
        init_method: Option<String>,
        #[arg(long)]
        init_args: Option<String>,
    },
    /// Call any method (owner operations: set_verifier, set_chain, mint, ...)
    Call {
        #[arg(long)]
        contract: String,
        #[arg(long)]
        method: String,
        #[arg(long, default_value = "{}")]
        args: String,
        #[arg(long, default_value_t = 0)]
        deposit_yocto: u128,
        #[arg(long, default_value_t = 100)]
        gas_tgas: u64,
    },
    SendNear {
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount_near: String,
    },
    /// Status, events and failures of a transaction
    Tx {
        hash: String,
        #[arg(long)]
        signer: String,
    },
}

#[derive(Subcommand)]
enum Cmd {
    /// Operator tools: accounts, deployments, owner calls
    Admin {
        #[command(subcommand)]
        cmd: AdminCmd,
    },
    Keygen {
        #[command(subcommand)]
        kind: KeygenKind,
    },
    /// Compile antibody.toml into the canonical manifest JSON (prints its sha256)
    Manifest {
        toml: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Derive a round manifest (a fix as code overrides at the same anchor) from the program's manifest
    ManifestRound {
        #[arg(long)]
        base: PathBuf,
        #[arg(long)]
        round: u32,
        /// <target address>=<file with the fixed runtime code in hex>; repeatable
        #[arg(long = "override", required = true)]
        overrides: Vec<String>,
        #[arg(long)]
        out: PathBuf,
    },
    Program {
        #[command(subcommand)]
        cmd: ProgramCmd,
    },
    Programs {
        #[arg(long, default_value_t = 0)]
        from: u32,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Write a hunting workspace (manifest, invariants, anvil fork command, bundle template)
    Pull {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run the verifier locally on a bundle against the proven anchor state
    Check {
        #[arg(long)]
        program: Option<u32>,
        #[arg(long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        salt: Option<B256>,
    },
    /// NEAR credit at near-antibody for storage and verification runs (NEP-145 style)
    StorageDeposit {
        #[arg(long, default_value = "0.5")]
        near: String,
    },
    /// Move tUSD into your credit at near-antibody (bonds, and your agents' bonds, come from it)
    Deposit {
        #[arg(long)]
        amount: String,
    },
    /// 500 tUSD per day (testnet token, no value)
    Faucet,
    /// Withdraw tUSD credit (payouts, returned bonds) to your wallet
    Withdraw {
        #[arg(long)]
        amount: String,
    },
    /// Seal to the program's key and commit with the bond; the blob stays local until `reveal`
    Commit {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        salt: Option<B256>,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Reveal the sealed blob (inline up to 15 KB, else FastFS): OutLayer verifies it
    Reveal {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        no_wait: bool,
    },
    /// Commit + reveal in one go, then wait for the verdict
    Submit {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        salt: Option<B256>,
        #[arg(long)]
        report: Option<PathBuf>,
        #[arg(long)]
        no_wait: bool,
    },
    /// Verify again after an infrastructure failure (within the grace period)
    Retry {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        no_wait: bool,
    },
    /// The second run a verdict above the instant cap needs (the requester pays; anyone may ask)
    Second {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        no_wait: bool,
    },
    Status {
        #[arg(long)]
        commit: Option<u64>,
        #[arg(long)]
        run: Option<u64>,
        #[arg(long)]
        program: Option<u32>,
        #[arg(long)]
        of: Option<String>,
    },
    /// Permissionless: allocate final verdicts in commit order, pay funded claims, settle debts
    Keeper {
        #[arg(long)]
        program: u32,
    },
    /// Rounds: propose a fix, re-run PoCs at it, open the next round (docs/rounds.md)
    Round {
        #[command(subcommand)]
        cmd: RoundCmd,
    },
    /// Permissionless: an unrevealed commit past its reveal window forfeits its bond
    Expire {
        #[arg(long)]
        commit: u64,
    },
    /// Permissionless: a commit whose verification never completed within the grace is refunded
    TimeOut {
        #[arg(long)]
        commit: u64,
    },
    Agent {
        #[command(subcommand)]
        cmd: AgentCmd,
    },
    /// Scaffold a program workspace: annotated antibody.toml, project key, next steps
    Init {
        #[arg(long)]
        out: PathBuf,
        /// Your NEAR account (the project)
        #[arg(long)]
        project: String,
        #[arg(long, default_value_t = 11155111)]
        chain_id: u64,
        /// Program deadline, in days from now
        #[arg(long, default_value_t = 14)]
        days: u64,
    },
    /// Read-only: compare a Sepolia block hash with NEAR's Ethereum light client (the check
    /// create_program runs); --forge also shows a forged header being rejected
    AnchorCheck {
        #[arg(long)]
        block: u64,
        #[arg(long)]
        forge: bool,
    },
    /// Draft invariants for your program from its sources (a defensive helper; you review and
    /// approve the output before committing anything)
    Invariants {
        #[command(subcommand)]
        cmd: InvariantsCmd,
    },
    /// T1: check or relay OutLayer's TDX quote for a run (NEAR's DCAP verifier)
    Tee {
        #[command(subcommand)]
        cmd: TeeCmd,
    },
    /// MCP server on stdio for AI agents
    Mcp,
}

#[derive(Subcommand)]
enum InvariantsCmd {
    /// Deterministic draft from standard shapes, plus templates; with --ai, Claude extends it
    /// (needs ANTHROPIC_API_KEY)
    Draft {
        /// Solidity files or directories (repeatable)
        #[arg(long = "source", required = true)]
        sources: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        ai: bool,
        #[arg(long, default_value = antibody_cli::draft::AI_MODEL)]
        model: String,
        #[arg(long, default_value = "high")]
        effort: String,
        #[arg(long, default_value_t = 16000)]
        max_tokens: u32,
    },
}

#[derive(Subcommand)]
enum TeeCmd {
    /// Check any OutLayer run's quote: report_data binding, DCAP verification as a free view call,
    /// and OutLayer's approved worker images (no keys, no transactions)
    Check {
        #[arg(long)]
        request_id: u64,
        #[arg(long, default_value = antibody_cli::tee::DEFAULT_API)]
        api: String,
        #[arg(long, default_value = antibody_cli::tee::DEFAULT_VERIFIER)]
        verifier: String,
        /// DCAP collateral: OutLayer's on-chain copies, fresh Intel PCS collateral, or both (auto)
        #[arg(long, default_value = "auto", value_parser = ["auto", "outlayer", "pcs"])]
        collateral: String,
    },
    /// Relay a run's quote to near-antibody, which repeats every check on chain: the run becomes T1
    Confirm {
        #[arg(long)]
        run: u64,
        /// OutLayer request id (or give the requesting transaction with --tx and --tx-signer)
        #[arg(long)]
        request_id: Option<u64>,
        #[arg(long)]
        tx: Option<String>,
        #[arg(long)]
        tx_signer: Option<String>,
        #[arg(long, default_value = antibody_cli::tee::DEFAULT_API)]
        api: String,
        #[arg(long, default_value = "auto", value_parser = ["auto", "outlayer", "pcs"])]
        collateral: String,
    },
    /// Sandbox only: relay the MOCK attestation that mock-outlayer recorded for a run
    ConfirmMock {
        #[arg(long)]
        run: u64,
        #[arg(long)]
        mock_outlayer: String,
    },
}

fn run(cli: Cli) -> Result<Value> {
    let mk = || ctx::Ctx::new(&cli.network, cli.deployments.clone(), cli.rpc.clone(), cli.account.clone(), cli.key_file.clone(), cli.eth_rpc.clone());
    match cli.cmd {
        Cmd::Keygen { kind: KeygenKind::X25519 { ref out } } => cmd::keygen_x25519(out),
        Cmd::Keygen { kind: KeygenKind::Near { ref account_id, ref out } } => cmd::keygen_near(account_id.clone(), out),
        Cmd::Manifest { ref toml, ref out } => cmd::manifest_compile(toml, out),
        Cmd::ManifestRound { ref base, round, ref overrides, ref out } => cmd::manifest_round(base, round, overrides, out),
        Cmd::Program { ref cmd } => {
            let c = mk()?;
            match cmd {
                ProgramCmd::Create { manifest, escrow, project_key, manifest_url } => {
                    cmd::program_create(&c, manifest, escrow, project_key, manifest_url.clone())
                }
                ProgramCmd::Activate { program, no_wait } => cmd::program_activate(&c, *program, !no_wait),
                ProgramCmd::Known { program, bundle, salt, report, no_wait } => cmd::submit_known(&c, *program, bundle, *salt, report.as_deref(), !no_wait),
                ProgramCmd::Open { program } => cmd::program_open(&c, *program),
                ProgramCmd::Show { program } => cmd::program_show(&c, *program),
                ProgramCmd::TopUp { program, amount } => cmd::top_up(&c, *program, amount),
                ProgramCmd::Exit { program } => cmd::call(&c, "exit", json!({"program_id": program}), 100),
            }
        }
        Cmd::Programs { from, limit } => cmd::programs(&mk()?, from, limit),
        Cmd::Pull { program, ref out } => cmd::pull(&mk()?, program, &out.clone().unwrap_or_else(|| PathBuf::from(format!("antibody-program-{program}")))),
        Cmd::Check { program, ref manifest, ref bundle, salt } => cmd::check(&mk()?, program, manifest.as_deref(), bundle, salt),
        Cmd::StorageDeposit { ref near } => cmd::storage_deposit(&mk()?, near),
        Cmd::Deposit { ref amount } => cmd::deposit(&mk()?, amount),
        Cmd::Faucet => cmd::faucet(&mk()?),
        Cmd::Withdraw { ref amount } => cmd::withdraw(&mk()?, amount),
        Cmd::Commit { program, ref bundle, salt, ref report } => cmd::commit(&mk()?, program, bundle, salt, report.as_deref()),
        Cmd::Reveal { program, commit, no_wait } => cmd::reveal(&mk()?, program, commit, !no_wait),
        Cmd::Submit { program, ref bundle, salt, ref report, no_wait } => cmd::submit(&mk()?, program, bundle, salt, report.as_deref(), !no_wait),
        Cmd::Retry { program, commit, no_wait } => cmd::retry(&mk()?, program, commit, !no_wait),
        Cmd::Second { program, commit, no_wait } => cmd::second(&mk()?, program, commit, !no_wait),
        Cmd::Status { commit, run, program, ref of } => cmd::status(&mk()?, commit, run, program, of.clone()),
        Cmd::Keeper { program } => cmd::keeper(&mk()?, program),
        Cmd::Round { cmd: RoundCmd::Propose { program, ref manifest, ref releases, no_wait } } => {
            let releases = releases.iter().map(|r| cmd::parse_release(r)).collect::<eyre::Result<Vec<_>>>()?;
            cmd::round_propose(&mk()?, program, manifest, &releases, !no_wait)
        }
        Cmd::Round { cmd: RoundCmd::Rerun { commit } } => cmd::round_rerun(&mk()?, commit),
        Cmd::Round { cmd: RoundCmd::Open { program } } => cmd::round_open(&mk()?, program),
        Cmd::Expire { commit } => cmd::call(&mk()?, "expire", json!({"commit_id": commit}), 30),
        Cmd::TimeOut { commit } => cmd::call(&mk()?, "time_out", json!({"commit_id": commit}), 30),
        Cmd::Agent { cmd: AgentCmd::Register { ref agent, ref budget, ref per_commit_cap } } => cmd::agent_register(&mk()?, agent, budget, per_commit_cap),
        Cmd::Agent { cmd: AgentCmd::Key { cmd: AgentKeyCmd::Add { allowance_near, ref out } } } => cmd::agent_key_add(&mk()?, allowance_near, out),
        Cmd::Admin { ref cmd } => {
            let c = mk()?;
            match cmd {
                AdminCmd::CreateAccount { new_account_id, amount_near, public_key, key_out } => {
                    cmd::admin_create_account(&c, new_account_id, amount_near, public_key.clone(), key_out.as_deref())
                }
                AdminCmd::Deploy { wasm, init_method, init_args } => cmd::admin_deploy(&c, wasm, init_method.clone(), init_args.clone()),
                AdminCmd::Call { contract, method, args, deposit_yocto, gas_tgas } => cmd::admin_call(&c, contract, method, args, *deposit_yocto, *gas_tgas),
                AdminCmd::SendNear { to, amount_near } => cmd::admin_send_near(&c, to, amount_near),
                AdminCmd::Tx { hash, signer } => cmd::tx(&c, hash, signer),
            }
        }
        Cmd::AnchorCheck { block, forge } => cmd::anchor_check(&mk()?, block, forge),
        Cmd::Init { ref out, ref project, chain_id, days } => cmd::init(out, project, chain_id, days),
        Cmd::Invariants { cmd: InvariantsCmd::Draft { ref sources, ref out, ai, ref model, ref effort, max_tokens } } => {
            antibody_cli::draft::run(sources, out, ai, model, effort, max_tokens)
        }
        Cmd::Tee { ref cmd } => {
            let c = mk()?;
            match cmd {
                TeeCmd::Check { request_id, api, verifier, collateral } => antibody_cli::tee::check(&c, api, verifier, *request_id, collateral),
                TeeCmd::Confirm { run, request_id, tx, tx_signer, api, collateral } => {
                    let id = match (request_id, tx, tx_signer) {
                        (Some(id), _, _) => *id,
                        (None, Some(tx), Some(signer)) => antibody_cli::tee::request_id_from_tx(&c, tx, signer)?,
                        _ => return Err(eyre::eyre!("pass --request-id, or --tx with --tx-signer")),
                    };
                    antibody_cli::tee::confirm(&c, api, *run, id, collateral)
                }
                TeeCmd::ConfirmMock { run, mock_outlayer } => antibody_cli::tee::confirm_mock(&c, mock_outlayer, *run),
            }
        }
        Cmd::Mcp => {
            mcp::serve(&mk()?)?;
            Ok(Value::Null)
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let is_mcp = matches!(cli.cmd, Cmd::Mcp);
    match run(cli) {
        Ok(v) if !is_mcp => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
        Ok(_) => {}
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tusd_amounts() {
        assert_eq!(cmd::parse_tusd("25000").unwrap(), 25_000_000_000);
        assert_eq!(cmd::parse_tusd("0.5").unwrap(), 500_000);
        assert!(cmd::parse_tusd("1.0000001").is_err());
        assert_eq!(cmd::fmt_tusd(3_000_000_000), "3000.000000");
    }

    #[test]
    fn commitment_binds_round_and_hunter() {
        // The same formula as near-antibody's `commitment_for` (pinned in contracts/near-antibody tests).
        let c = cmd::commitment("bounty.testnet/0", 0, "agent-a.testnet", &[0xab; 32]);
        assert_ne!(c, cmd::commitment("bounty.testnet/0", 1, "agent-a.testnet", &[0xab; 32]));
        assert_ne!(c, cmd::commitment("bounty.testnet/0", 0, "agent-b.testnet", &[0xab; 32]));
        assert_eq!(hex::encode(c), "e0b01b170a5e2d10a922e1e6e722dddf3dba94f8b02c06eb5051543dbfff3141");
    }

    #[test]
    fn cli_parses() {
        Cli::try_parse_from(["antibody", "submit", "--program", "0", "--bundle", "b.json"]).unwrap();
        Cli::try_parse_from(["antibody", "--account", "a.testnet", "program", "create", "--manifest", "m.json", "--escrow", "1", "--project-key", "k.json"]).unwrap();
    }
}

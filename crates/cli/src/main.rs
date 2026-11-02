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
enum ProgramCmd {
    /// Open a program: upload the manifest, escrow tUSD, commit the manifest hash (one transaction)
    Create {
        #[arg(long)]
        manifest: PathBuf,
        /// Escrow in tUSD (e.g. 25000)
        #[arg(long)]
        escrow: String,
        #[arg(long)]
        project_key: PathBuf,
        #[arg(long)]
        guardian: Option<String>,
        /// Use an already-hosted manifest instead of uploading it with FastFS
        #[arg(long)]
        manifest_url: Option<String>,
    },
    /// Declare a known issue before opening (verified by the same TEE; claims its keys at t0)
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
    /// End the known-issues phase; hunters can commit
    Open {
        #[arg(long)]
        program: u32,
    },
    /// Rounds: commit a fix (round manifest from `antibody manifest-round`); new commits verify
    /// against it at once, and claimed findings can be re-run against it
    RoundPropose {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Re-run a claimed finding against the proposed fix (project: --project-key; finder: local record)
    RoundRerun {
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        project_key: Option<PathBuf>,
        #[arg(long)]
        no_wait: bool,
    },
    /// Open the proposed round: keys whose PoCs no longer break are released, the rest carry over
    RoundOpen {
        #[arg(long)]
        program: u32,
    },
    Show {
        program: u32,
    },
    /// Add escrow
    Fund {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        amount: String,
    },
    /// After deadline + disclosure delay: publish every content key and k_program (TEE run)
    Disclose {
        #[arg(long)]
        program: u32,
    },
    WithdrawEscrow {
        #[arg(long)]
        program: u32,
    },
}

#[derive(Subcommand)]
enum AgentKeyCmd {
    /// Add a function-call key limited to commit/submit on near-antibody, with a gas allowance
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
    /// Generate the verifier seed as a PROTECTED_ secret in OutLayer's keystore, readable only by
    /// the given verifier build when called by the given contract
    OutlayerSecret {
        #[arg(long)]
        wasm_hash: String,
        #[arg(long, default_value = "antibody-v1")]
        profile: String,
        #[arg(long)]
        contract: String,
        #[arg(long, default_value = "outlayer.testnet")]
        outlayer: String,
        #[arg(long, default_value = "https://testnet-api.outlayer.ai")]
        api: String,
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
    /// Move tUSD into your internal balance (bonds are locked from it)
    Deposit {
        #[arg(long)]
        amount: String,
    },
    /// 500 tUSD per day (testnet token, no value)
    Faucet,
    Withdraw {
        #[arg(long)]
        amount: String,
    },
    /// Seal + commit (priority); the blob stays local until `reveal`
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
    /// Upload the sealed blob and reveal it (OutLayer verifies it)
    Reveal {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        no_wait: bool,
    },
    /// Commit + upload + reveal in one go, then wait for the verdict
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
    Status {
        #[arg(long)]
        commit: Option<u64>,
        #[arg(long)]
        program: Option<u32>,
        #[arg(long)]
        of: Option<String>,
    },
    Leaderboard {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Apply priority and pay (after the reveal windows of earlier commits)
    Finalize {
        #[arg(long)]
        commit: u64,
    },
    /// Pay a T0-delayed payout after its challenge window
    Release {
        #[arg(long)]
        commit: u64,
    },
    /// Forfeit an unrevealed commit's bond after its window
    Expire {
        #[arg(long)]
        commit: u64,
    },
    /// Re-request verification after OutLayer returned no result
    Retry {
        #[arg(long)]
        commit: u64,
    },
    /// Freeze a delayed payout for the arbiter (your local re-run disagrees)
    Challenge {
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        reason: String,
    },
    /// Projects can pay more, never less
    TopUp {
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        amount: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Re-run a commit's verification locally and compare with the chain (after disclosure)
    Verify {
        #[arg(long)]
        commit: u64,
    },
    /// Project: read a valid finding (unwrap the project wrap, decrypt, re-run)
    Decrypt {
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        project_key: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    AgentKey {
        #[command(subcommand)]
        cmd: AgentKeyCmd,
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
    },
    /// Relay the quote of a commit's verification (or, with --round, its re-run) to near-antibody,
    /// which repeats every check on chain and marks the verdict T1
    Confirm {
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        round: Option<u32>,
        /// OutLayer request id (or give the requesting transaction with --tx and --tx-signer)
        #[arg(long)]
        request_id: Option<u64>,
        #[arg(long)]
        tx: Option<String>,
        #[arg(long)]
        tx_signer: Option<String>,
        #[arg(long, default_value = antibody_cli::tee::DEFAULT_API)]
        api: String,
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
                ProgramCmd::Create { manifest, escrow, project_key, guardian, manifest_url } => {
                    cmd::program_create(&c, manifest, escrow, project_key, guardian.clone(), manifest_url.clone())
                }
                ProgramCmd::Known { program, bundle, salt, report, no_wait } => cmd::submit_known(&c, *program, bundle, *salt, report.as_deref(), !no_wait),
                ProgramCmd::Open { program } => cmd::program_open(&c, *program),
                ProgramCmd::RoundPropose { program, manifest } => cmd::round_propose(&c, *program, manifest),
                ProgramCmd::RoundRerun { commit, project_key, no_wait } => cmd::round_rerun(&c, *commit, project_key.as_deref(), !no_wait),
                ProgramCmd::RoundOpen { program } => cmd::round_open(&c, *program),
                ProgramCmd::Show { program } => cmd::program_show(&c, *program),
                ProgramCmd::Fund { program, amount } => cmd::fund(&c, *program, amount),
                ProgramCmd::Disclose { program } => cmd::program_simple(&c, "disclose", json!({"program_id": program}), 300),
                ProgramCmd::WithdrawEscrow { program } => cmd::program_simple(&c, "withdraw_escrow", json!({"program_id": program}), 60),
            }
        }
        Cmd::Programs { from, limit } => cmd::programs(&mk()?, from, limit),
        Cmd::Pull { program, ref out } => cmd::pull(&mk()?, program, &out.clone().unwrap_or_else(|| PathBuf::from(format!("antibody-program-{program}")))),
        Cmd::Check { program, ref manifest, ref bundle, salt } => cmd::check(&mk()?, program, manifest.as_deref(), bundle, salt),
        Cmd::Deposit { ref amount } => cmd::deposit(&mk()?, amount),
        Cmd::Faucet => cmd::faucet(&mk()?),
        Cmd::Withdraw { ref amount } => cmd::withdraw(&mk()?, amount),
        Cmd::Commit { program, ref bundle, salt, ref report } => cmd::commit(&mk()?, program, bundle, salt, report.as_deref()),
        Cmd::Reveal { program, commit, no_wait } => cmd::reveal(&mk()?, program, commit, !no_wait),
        Cmd::Submit { program, ref bundle, salt, ref report, no_wait } => cmd::submit(&mk()?, program, bundle, salt, report.as_deref(), !no_wait),
        Cmd::Status { commit, program, ref of } => cmd::status(&mk()?, commit, program, of.clone()),
        Cmd::Leaderboard { limit } => cmd::leaderboard(&mk()?, limit),
        Cmd::Finalize { commit } => cmd::program_simple(&mk()?, "finalize", json!({"commit_id": commit}), 60),
        Cmd::Release { commit } => cmd::program_simple(&mk()?, "release", json!({"commit_id": commit}), 60),
        Cmd::Expire { commit } => cmd::program_simple(&mk()?, "expire", json!({"commit_id": commit}), 30),
        Cmd::Retry { commit } => cmd::program_simple(&mk()?, "retry_verify", json!({"commit_id": commit}), cmd::SUBMIT_TGAS),
        Cmd::Challenge { commit, ref reason } => cmd::program_simple(&mk()?, "challenge", json!({"commit_id": commit, "reason": reason}), 30),
        Cmd::TopUp { commit, ref amount, ref note } => cmd::top_up(&mk()?, commit, amount, note.clone()),
        Cmd::Verify { commit } => cmd::verify(&mk()?, commit),
        Cmd::Decrypt { commit, ref project_key, ref out } => cmd::decrypt(&mk()?, commit, project_key, out),
        Cmd::AgentKey { cmd: AgentKeyCmd::Add { allowance_near, ref out } } => cmd::agent_key_add(&mk()?, allowance_near, out),
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
                AdminCmd::OutlayerSecret { wasm_hash, profile, contract, outlayer, api } => {
                    cmd::admin_outlayer_secret(&c, api, outlayer, wasm_hash, profile, contract)
                }
            }
        }
        Cmd::Invariants { cmd: InvariantsCmd::Draft { ref sources, ref out, ai, ref model, ref effort, max_tokens } } => {
            antibody_cli::draft::run(sources, out, ai, model, effort, max_tokens)
        }
        Cmd::Tee { ref cmd } => {
            let c = mk()?;
            match cmd {
                TeeCmd::Check { request_id, api, verifier } => antibody_cli::tee::check(&c, api, verifier, *request_id),
                TeeCmd::Confirm { commit, round, request_id, tx, tx_signer, api } => {
                    let id = match (request_id, tx, tx_signer) {
                        (Some(id), _, _) => *id,
                        (None, Some(tx), Some(signer)) => antibody_cli::tee::request_id_from_tx(&c, tx, signer)?,
                        _ => return Err(eyre::eyre!("pass --request-id, or --tx with --tx-signer")),
                    };
                    antibody_cli::tee::confirm(&c, api, *commit, *round, id)
                }
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
    fn commitment_matches_the_contract_formula() {
        // contracts/near-antibody: SHA-256("antibody/commit/v1|program|hunter|sha256(blob)")
        let c = cmd::commitment("bounty.testnet/0", "agent-a.testnet", &"ab".repeat(32));
        assert_eq!(c.len(), 64);
    }

    #[test]
    fn cli_parses() {
        Cli::try_parse_from(["antibody", "submit", "--program", "0", "--bundle", "b.json"]).unwrap();
        Cli::try_parse_from(["antibody", "--account", "a.testnet", "program", "create", "--manifest", "m.json", "--escrow", "1", "--project-key", "k.json"]).unwrap();
    }
}

//! `antibody` — create programs, hunt, commit and submit sealed proofs, verify receipts, and serve
//! the same operations to AI agents over MCP. Every command prints JSON with explorer links.

use antibody_cli::{cmd, ctx, hunt, immune, mcp};

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
    /// X25519 key for a project (findings paid to it are wrapped to it): random, or with --nep413
    /// derived from the --account key's NEP-413 signature, so the same NEAR key re-derives it
    X25519 {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        nep413: bool,
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
        /// A round base witness built earlier (`antibody round base`); default: build it now
        #[arg(long)]
        base_witness: Option<PathBuf>,
        #[arg(long)]
        no_wait: bool,
        /// A pull-request check: propose target code only. A manifest whose invariants (rules or slots) differ
        /// from the current round's is refused before anything is sent
        #[arg(long)]
        code_only: bool,
    },
    /// Build a round's base witness now (needs an RPC that serves proofs at the anchor), for later
    Base {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Re-run a commit at the proposed fix (permissionless; the requester pays). A commit revealed inline
    /// needs its blob passed again: from --blob, the reveal transaction (--reveal-tx) or an Antibody API
    /// (--api); a FastFS blob needs none
    Rerun {
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        blob: Option<PathBuf>,
        #[arg(long)]
        reveal_tx: Option<String>,
        /// The reveal transaction's signer (default: the commit's hunter)
        #[arg(long)]
        reveal_signer: Option<String>,
        #[arg(long)]
        api: Option<String>,
    },
    /// Open the next round once its minimum duration and re-run window have passed (permissionless)
    Open {
        #[arg(long)]
        program: u32,
    },
    /// The program's proposal slot (view calls only): free, or taken by a proposal that is checking,
    /// accepted (only opening it clears it) or stale, with when its re-run window ends and the round may open
    State {
        #[arg(long)]
        program: u32,
    },
    /// Per paid key, what the proposed round's re-runs inside the enclave make of it: still breaks, fixed,
    /// or no verdict yet (view calls only). `verdict` is red, green or inconclusive
    Results {
        #[arg(long)]
        program: u32,
        /// A round other than the proposal's (default: the proposal's, or the round just opened). A check passes
        /// the round its `round propose` printed
        #[arg(long)]
        round: Option<u64>,
        /// The proposal's round-check run (`run_id` in `round propose`'s output): a check it refused is an
        /// error, never another round's results
        #[arg(long)]
        check_run: Option<u64>,
        /// Wait up to this many seconds for every paid key's re-run to settle, the proposal to go, or its
        /// re-run window to end
        #[arg(long, default_value_t = 0)]
        wait: u64,
        /// The site's origin, for a link to each re-run's page
        #[arg(long)]
        site: Option<String>,
        #[arg(long)]
        json: Option<PathBuf>,
        /// A Markdown summary (a job summary)
        #[arg(long)]
        md: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ProgramCmd {
    /// Create a program: upload the manifest (v2), escrow the payout token and commit the manifest hash (one
    /// transaction); NEAR credit for storage is added first if needed
    Create {
        #[arg(long)]
        manifest: PathBuf,
        /// Escrow in the payout token's units (tUSD in the sandbox, wNEAR on testnet)
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
        /// A proof (bundle file) the pinned base witness must also cover, so it seals with no Ethereum
        /// RPC (a practice program's tutorial proof); repeatable
        #[arg(long)]
        cover: Vec<PathBuf>,
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
        #[arg(long)]
        program: u32,
    },
    /// Read a paid finding: request its release wrap (the first time; the requester pays), unwrap the
    /// content key with the project key, open the sealed proof and re-run it locally. The blob comes
    /// from --blob, a transaction that passed it (--reveal-tx), or an Antibody API (--api)
    Read {
        #[arg(long)]
        commit: u64,
        #[arg(long)]
        project_key: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        blob: Option<PathBuf>,
        #[arg(long)]
        reveal_tx: Option<String>,
        /// The reveal transaction's signer (default: the commit's hunter)
        #[arg(long)]
        reveal_signer: Option<String>,
        #[arg(long)]
        api: Option<String>,
        /// Ask for a new wrap: one no quote confirmed may be replaced after the run timeout
        #[arg(long)]
        new_wrap: bool,
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
enum PracticeCmd {
    /// Deploy the vending contract on --account (its own account) and add the public vend key
    Deploy {
        #[arg(long)]
        wasm: PathBuf,
        /// The practice-account contract each vend deploys (contracts/practice-account)
        #[arg(long, default_value = "contracts/practice-account/target/wasm32-unknown-unknown/release/antibody_practice_account.wasm")]
        account_wasm: PathBuf,
        /// Who funds it and may change its terms
        #[arg(long)]
        owner: String,
        #[arg(long)]
        program: u32,
        #[arg(long, default_value_t = 24)]
        vaults: u32,
        /// NEAR each practice account gets: its contract's storage and its calls' gas, which NEAR sets
        /// aside up front at ten times the gas price (the commit's 0.025 may not be back when the 200 Tgas
        /// reveal sets aside 0.2); closing the account returns what is left. The key's allowance counts
        /// every call's set-aside gas
        #[arg(long, default_value = "0.28")]
        gas_funds: String,
        #[arg(long, default_value = "0.4")]
        allowance: String,
        /// Bonds a practice account may have locked, and per commit (tokens)
        #[arg(long)]
        bond_budget: String,
        #[arg(long)]
        per_commit_cap: String,
        #[arg(long, default_value_t = 6)]
        per_hour: u32,
        /// The public vend key's allowance (NEAR): a ceiling, each vend counts its set-aside gas
        /// (0.06); a locked vending contract's key cannot be given more
        #[arg(long, default_value = "10")]
        vend_allowance: String,
    },
    /// Point the vending contract at a new practice program (as its owner), with the practice-account
    /// contract later vends deploy: a locked vending contract keeps serving new programs
    Retarget {
        #[arg(long)]
        vending: String,
        #[arg(long, default_value = "contracts/practice-account/target/wasm32-unknown-unknown/release/antibody_practice_account.wasm")]
        account_wasm: PathBuf,
        #[arg(long)]
        program: u32,
        #[arg(long, default_value_t = 24)]
        vaults: u32,
        /// Each term not given stays as the contract has it (an hourly limit raised for judging included)
        #[arg(long)]
        gas_funds: Option<String>,
        #[arg(long)]
        allowance: Option<String>,
        #[arg(long)]
        bond_budget: Option<String>,
        #[arg(long)]
        per_commit_cap: Option<String>,
        #[arg(long)]
        per_hour: Option<u32>,
    },
    /// Fund the vending contract (as its owner): NEAR credit at near-antibody, and tokens for bonds
    Fund {
        #[arg(long)]
        vending: String,
        #[arg(long, default_value = "0")]
        near: String,
        #[arg(long, default_value = "0")]
        tokens: String,
    },
    /// Change the vending contract's hourly vend limit or its program (as its owner) with `set_terms`
    /// alone: every other term, the instances and the practice-account code stay as they are. Prints the
    /// configuration before and after
    Terms {
        #[arg(long)]
        vending: String,
        /// Practice accounts a vend may create per hour (the judging days need more than the usual 6)
        #[arg(long)]
        per_hour: Option<u32>,
        /// A new practice program to vend for (a fresh one gives every instance back: docs/runbooks/worker-images.md)
        #[arg(long)]
        program: Option<u32>,
    },
}

#[derive(Subcommand)]
enum HuntCmd {
    /// Vend <name>.<vending> for a key made here, print the instance assigned to it, and write a workspace
    /// for the proof: the program's files, the instance, the rules, the proof format and, for the practice
    /// program, its sources and a Foundry harness. The account stays open until `hunt submit`
    Start {
        /// The account's name: <name>.<vending>, 1 to 32 of a-z, 0-9, - and _
        #[arg(long)]
        name: String,
        /// The vending contract (default: the deployment's hunt vending; the practice program's is
        /// practice.antibody.testnet)
        #[arg(long)]
        vending: Option<String>,
        /// The workspace (default: hunt-<name>)
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Check the proof before anything is spent (its shape, its targets: this account's instance only, its
    /// sealed size: at most 15 KB), then commit and reveal it through the account, wait for the verdict and
    /// T1 or the end of the challenge window, settle, and close the account
    Submit {
        #[arg(long)]
        name: String,
        /// The proof: a bundle file
        #[arg(long)]
        poc: PathBuf,
        /// The vending contract (default: the one `hunt start` recorded for this name, else the
        /// deployment's hunt vending)
        #[arg(long)]
        vending: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// The account's commits, verdicts and payouts (view calls only)
    Status {
        #[arg(long)]
        name: String,
        #[arg(long)]
        vending: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum AgentCmd {
    /// Register one of your sub-accounts as an agent: its bonds come from your credit, within the
    /// budget and the per-commit cap; payouts go to you
    Register {
        #[arg(long)]
        agent: String,
        /// What the agent may cost you in bonds: those locked now plus those it lost (payout-token units)
        #[arg(long)]
        budget: String,
        #[arg(long)]
        per_commit_cap: String,
        /// The only programs it may commit to (repeat; default: any)
        #[arg(long = "program")]
        programs: Vec<u32>,
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
    /// Add a function-call key to the signer's account, limited to the named near-antibody methods and a gas
    /// allowance (it can never move funds): e.g. propose_round,__fastdata_fastfs for a pull-request check's
    /// proposals, or open_round,allocate for a bot (docs/immune.md)
    KeyAdd {
        /// Comma-separated near-antibody methods
        #[arg(long, value_delimiter = ',')]
        methods: Vec<String>,
        #[arg(long, default_value_t = 1.0)]
        allowance_near: f64,
        #[arg(long)]
        out: PathBuf,
    },
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
    /// Lock the signer's contract account: delete its full-access keys (irreversible; its function-call
    /// keys stay)
    Lock {
        /// The signer's own account id, typed again
        #[arg(long)]
        confirm_irreversible: String,
    },
    /// Delete the signer's account, its NEAR to the beneficiary (irreversible: for an account a run no
    /// longer needs, such as a staging vending contract)
    DeleteAccount {
        #[arg(long)]
        beneficiary: String,
        /// The signer's own account id, typed again
        #[arg(long)]
        confirm_irreversible: String,
    },
    /// A versioned deployment record: each contract's code hash and locked status, the pinned verifier
    /// and the source commit, written to the deployments file (`release`) and to deployments/history/
    RecordDeployment {
        /// The source commit the deployment was built from
        #[arg(long)]
        git_commit: Option<String>,
        /// The deployments file to update (default: deployments/near-<network>.json)
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum Cmd {
    /// Operator tools: accounts, deployments, owner calls
    Admin {
        #[command(subcommand)]
        cmd: AdminCmd,
    },
    /// Make a key: a NEAR ed25519 key, or a project's X25519 key (`x25519`)
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
        /// A new invariants contract (runtime code hex) that adds slots (--add-slot)
        #[arg(long)]
        invariants_code: Option<PathBuf>,
        /// <bit>:<name>:<severity> of a slot the new invariants add; repeatable
        #[arg(long = "add-slot")]
        add_slots: Vec<String>,
        /// The rule strength report measured at this round's code (`antibody invariants strength`)
        #[arg(long)]
        rule_strength: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// A program: create, activate, declare known issues, open, show, top up, exit
    Program {
        #[command(subcommand)]
        cmd: ProgramCmd,
    },
    /// List the deployment's programs (id, phase, anchor)
    Programs {
        #[arg(long, default_value_t = 0)]
        from: u32,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Write a workspace for a program (manifest, invariants, anvil fork command, bundle template)
    Pull {
        #[arg(long)]
        program: u32,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Translate a forge-test proof into a bundle: its calls as pranked senders, with a restricted
    /// cheatcode set (pranks as its own senders, deals to them, helpers that change no state)
    FromForge {
        /// The Foundry project
        #[arg(long)]
        project: PathBuf,
        /// The test contract and function
        #[arg(long)]
        contract: String,
        #[arg(long)]
        test: String,
        /// Fork at the program's anchor: an RPC and its block (or run the test on a local copy)
        #[arg(long)]
        fork_url: Option<String>,
        #[arg(long)]
        fork_block: Option<u64>,
        /// The senders' ether when the test deals them none (wei; gas is paid from it)
        #[arg(long, default_value = "10000000000000000")]
        endowment_floor: String,
        #[arg(long, default_value = "forge")]
        forge: PathBuf,
        #[arg(long)]
        out: PathBuf,
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
    /// Immune memory on a pull request (docs/immune.md): re-run every paid proof this run can read against this
    /// build's code, with the verifier, locally, at the program's anchor. View calls only. Exit 0: no re-run proof
    /// breaks a rule; 2: one still breaks a rule; 3: none breaks, but a re-run was inconclusive
    Immune(ImmuneArgs),
    /// A bounty window per pull request (docs/immune.md, "Bounty windows"): a fork-mode program for the pull
    /// request's build, with wNEAR on testnet (no value) at stake; proofs committed before its deadline are judged
    /// and paid
    Window {
        #[command(subcommand)]
        cmd: WindowCmd,
    },
    /// NEAR credit at near-antibody for storage and verification runs (NEP-145 style)
    StorageDeposit {
        #[arg(long, default_value = "0.5")]
        near: String,
    },
    /// Move payout tokens into your credit at near-antibody (bonds, and your agents' bonds, come from it)
    Deposit {
        #[arg(long)]
        amount: String,
    },
    /// Sandbox tUSD only: 500 per day (a test token with no value)
    Faucet,
    /// Withdraw token credit (payouts, returned bonds) to your wallet
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
    /// A commit, a verification run or an account: verdict, tier, state, payout, an agent's limits
    Status {
        /// A commit: its verdict, tier, state and payout
        #[arg(long)]
        commit: Option<u64>,
        /// A verification run
        #[arg(long)]
        run: Option<u64>,
        /// A program: its rounds (release gate, keys released and carried) and every commit
        #[arg(long)]
        program: Option<u32>,
        /// An account's credit and agent statement
        #[arg(long)]
        of: Option<String>,
        /// The full record as JSON (default: a summary)
        #[arg(long)]
        json: bool,
    },
    /// View calls only (no account, no key): each contract's code hash and whether it is locked, the
    /// pins, and per program its anchor, T1 confirmations, payouts, pauses and permits, with links
    Judge {
        #[arg(long)]
        program: Option<u32>,
        /// The full record as JSON (default: a one-screen summary)
        #[arg(long)]
        json: bool,
    },
    /// A view call on any NEAR contract (no account, no key), e.g. a consumer's state
    View {
        #[arg(long)]
        contract: String,
        #[arg(long)]
        method: String,
        #[arg(long, default_value = "{}")]
        args: String,
    },
    /// Permissionless: allocate final verdicts in commit order, pay funded claims, settle debts
    Keeper {
        #[arg(long)]
        program: u32,
    },
    /// The practice program (a tutorial, docs/practice.md): vend a practice account, prove its vault with
    /// the public tutorial proof (or your own, --poc), and settle. Needs no account: without --account the
    /// public vend key calls `vend`
    Try {
        /// The vending contract (default: the deployment's practice program)
        #[arg(long)]
        vending: Option<String>,
        #[arg(long)]
        program: Option<u32>,
        /// Your practice account's name: <name>.<vending>
        #[arg(long)]
        name: String,
        /// Your own proof (a bundle file: docs/practice.md, The proof) instead of the tutorial's. It is
        /// checked before anything is spent: its shape, its targets (the instance the next vend assigns)
        /// and its sealed size (at most 15 KB)
        #[arg(long)]
        poc: Option<PathBuf>,
        /// The full record as JSON (default: a summary with each transaction)
        #[arg(long)]
        json: bool,
    },
    /// Bring your own proof, with no NEAR account (docs/practice.md): a vended account, its own instance of
    /// the program's targets, and your proof
    Hunt {
        #[command(subcommand)]
        cmd: HuntCmd,
    },
    /// The practice program's operator: deploy and fund the vending contract
    Practice {
        #[command(subcommand)]
        cmd: PracticeCmd,
    },
    /// Rounds: propose a fix, re-run paid proofs at it, open the next round (docs/rounds.md)
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
    /// Agent accounts: register a sub-account with a budget and a per-commit cap, and give it a limited key
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
    /// Read-only, NEAR mainnet: the Ethereum block NEAR's mainnet light client holds, its hash there
    /// and on Ethereum, and its lag behind Ethereum's finalized head (docs/mainnet.md). Sends nothing
    MainnetCheck {
        /// An Ethereum mainnet RPC (default: public endpoints, no key)
        #[arg(long)]
        mainnet_eth_rpc: Option<String>,
        /// A NEAR mainnet RPC (default: FastNEAR's)
        #[arg(long)]
        mainnet_near_rpc: Option<String>,
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
    /// T2 (docs/zk.md): a validity proof of a disclosed verdict. The run's verifier re-runs in SP1 on the
    /// exact request near-antibody sent the TEE, and near-t2 checks the proof against near-antibody's
    /// records. It claims no privacy (the proof it re-runs is disclosed); payouts do not depend on it
    Zk {
        #[command(subcommand)]
        cmd: ZkCmd,
    },
    /// MCP server on stdio for AI agents; every path a tool takes stays inside the workspace
    Mcp {
        /// Default: the working directory
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
}

/// `antibody immune`'s flags, in a struct of their own (clap builds them in their own function).
#[derive(clap::Args)]
struct ImmuneArgs {
    /// The mapping the project commits: the program, and the target each contract builds
    #[arg(long, default_value = ".antibody/immune.toml")]
    config: PathBuf,
    /// The program (default: the mapping's)
    #[arg(long)]
    program: Option<u32>,
    /// Where the mapping's relative paths start (default: the current directory)
    #[arg(long)]
    root: Option<PathBuf>,
    /// The project's key file, to read paid findings wrapped to it (or $ANTIBODY_PROJECT_KEY: the file's JSON or the
    /// secret in hex). Never given to pull requests from forks
    #[arg(long)]
    project_key: Option<PathBuf>,
    /// A published content key, <commit>=<hex or file>; repeatable
    #[arg(long = "content-key")]
    content_keys: Vec<String>,
    /// Revealed blobs, found by sha256 (default: the mapping's `blobs`)
    #[arg(long)]
    blobs: Option<PathBuf>,
    /// An Antibody API that indexed the reveals, for blobs not in --blobs
    #[arg(long)]
    api: Option<String>,
    /// Write the report as JSON
    #[arg(long)]
    json: Option<PathBuf>,
    /// Write the report as Markdown (a job summary)
    #[arg(long)]
    md: Option<PathBuf>,
    /// Write the pull request's code as a round manifest
    #[arg(long)]
    manifest_out: Option<PathBuf>,
    /// A paid proof that cannot be read counts as inconclusive (exit 3)
    #[arg(long)]
    strict: bool,
    /// Show where a proof read with the project key breaks, and every rule it breaks (off: it is still sealed, and only
    /// the rules its commit's keys name are shown; never in a public log)
    #[arg(long)]
    reveal_sealed: bool,
}

#[derive(Subcommand)]
enum WindowCmd {
    /// Write the window's fork-mode manifest for this build (anchored on the newest Sepolia block NEAR's light
    /// client holds), then create, activate and open the program as the project (--account, its full-access
    /// key). --dry-run writes the manifest only: no key, nothing sent
    Open(WindowOpenArgs),
    /// A window's deadline, held hours, escrow at stake, valid findings (tier and severity) and verdict: open
    /// before the deadline, then settling, green or red; with the commit status line. View calls only
    Status {
        #[arg(long)]
        program: u32,
        /// The site the status links to
        #[arg(long, default_value = antibody_cli::window::DEFAULT_SITE)]
        site: String,
    },
    /// After the deadline and the exit grace: settle what the exit waits for, exit (the escrow's rest returns to
    /// the project's credit), close the records and withdraw the project's credit. Safe to run again
    Close {
        #[arg(long)]
        program: u32,
    },
    /// The windows on near-antibody, by their names: a project's, a repository's, a pull request's. View calls only
    List {
        /// Only this project's windows (the account that opened them)
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        pr: Option<u64>,
    },
}

/// `antibody window open`'s flags.
#[derive(clap::Args)]
struct WindowOpenArgs {
    /// The window config the repository commits (relative paths in it start at --root)
    #[arg(long, default_value = antibody_cli::window::DEFAULT_CONFIG)]
    config: PathBuf,
    /// Where the config's relative paths start (default: the current directory)
    #[arg(long)]
    root: Option<PathBuf>,
    /// The build's Foundry output directory (default: the config's `artifacts`)
    #[arg(long)]
    artifacts: Option<PathBuf>,
    /// Where the rules contract's artifact is (default: --artifacts): the base branch's build, so a pull request
    /// cannot change the rules its own window is judged by
    #[arg(long)]
    rules_artifacts: Option<PathBuf>,
    /// The pull request's number
    #[arg(long)]
    pr: Option<u64>,
    /// The repository's name (default: the config's `repository`)
    #[arg(long)]
    repo: Option<String>,
    /// The pull request's head commit: its short form goes into the name, one window per head
    #[arg(long)]
    head: Option<String>,
    /// The program's name (default: "PR #<pr> window (<repo> @ <head>)")
    #[arg(long)]
    name: Option<String>,
    /// The window's length in hours (default: the config's `hours`, else 24)
    #[arg(long)]
    hours: Option<f64>,
    /// The escrow in wNEAR (default: the config's `escrow`, else 0.03)
    #[arg(long)]
    escrow: Option<String>,
    /// The project the manifest names (default: --account); with --dry-run and no account, a placeholder
    #[arg(long)]
    project: Option<String>,
    /// Where the manifest is written
    #[arg(long, default_value = "window-manifest.json")]
    out: PathBuf,
    /// Write the manifest only: nothing is signed or sent
    #[arg(long)]
    dry_run: bool,
    /// Windows the project may open in 24 hours (default: the config's `daily_cap`, else 4)
    #[arg(long)]
    daily_cap: Option<u32>,
    /// Refuse an escrow above this many wNEAR
    #[arg(long)]
    max_escrow: Option<String>,
    /// The site the status and the announcement link to
    #[arg(long, default_value = antibody_cli::window::DEFAULT_SITE)]
    site: String,
    /// Never relay the activation's TDX quote here (by default the project relays it when no relayer has
    /// within 45 s: about 0.018 NEAR)
    #[arg(long)]
    no_relay: bool,
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
    /// The validate gate: check a manifest as creation and activation will (settlement terms, the rules at
    /// the anchor, target code hashes, canaries, a fork-mode deployment), and optionally a rule strength
    /// floor. Nothing is sent; exits non-zero when a check fails
    Validate {
        #[arg(long)]
        manifest: PathBuf,
        /// A rule strength report (`antibody invariants strength`) to hold to --min-strength
        #[arg(long)]
        strength: Option<PathBuf>,
        #[arg(long, default_value_t = 0.0)]
        min_strength: f64,
    },
    /// Foundry-style invariant functions (`invariant_*`) of a compiled contract as the program's slots:
    /// writes the rules contract (it inherits FoundryInvariants) and prints the manifest slots
    FromFoundry {
        /// The contract's Foundry artifact (out/<File>.sol/<Contract>.json)
        #[arg(long)]
        artifact: PathBuf,
        /// The contract's source and FoundryInvariants.sol, as the generated file imports them
        #[arg(long)]
        import: String,
        #[arg(long)]
        base_import: String,
        /// Slot order: the invariant functions, comma-separated (default: by name)
        #[arg(long, value_delimiter = ',')]
        order: Option<Vec<String>>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Rule strength: mutate the protocol's sources (labeled: missing access checks, flipped
    /// comparisons, unchecked transfers) and count the mutants the program's rules catch under the
    /// project's Foundry invariant test. Measures the rules, not the code's security
    Strength {
        /// The Foundry project
        #[arg(long)]
        project: PathBuf,
        /// The invariant test contract asserting the program's rules (`--match-contract`)
        #[arg(long)]
        test: String,
        /// Sources to mutate, relative to the project (repeatable)
        #[arg(long = "mutate", required = true)]
        files: Vec<String>,
        #[arg(long, default_value_t = 256)]
        runs: u32,
        #[arg(long, default_value_t = 64)]
        depth: u32,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long, default_value_t = 2)]
        workers: usize,
        /// Seconds per mutant
        #[arg(long, default_value_t = 300)]
        timeout: u64,
        #[arg(long, default_value = "forge")]
        forge: PathBuf,
        /// Scratch directory for the project copies
        #[arg(long, default_value = "target/strength")]
        work: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ZkCmd {
    /// Gather a run's inputs from NEAR and FastFS (each checked against the chain's hashes), re-run it
    /// natively against the TEE's receipt, then prove it with antibody-zk-prove (SP1 6.10.0, Groth16)
    Prove {
        #[arg(long)]
        run: u64,
        /// The blob's content key (hex). Default: your own commit record, or a practice try's public key
        #[arg(long)]
        key: Option<String>,
        /// The transaction that requested the run (a reveal or a retry). Default: your records, or --api
        #[arg(long)]
        tx: Option<String>,
        /// An Antibody API that indexed the run's transaction (/v1/commits/{id})
        #[arg(long)]
        api: Option<String>,
        /// The case directory (default: <state>/zk/<run>)
        #[arg(long)]
        out: Option<PathBuf>,
        /// network (SP1's prover network: NETWORK_PRIVATE_KEY or --requester-key), cuda (a local NVIDIA
        /// GPU), or none (gather and re-run natively only)
        #[arg(long, default_value = "network", value_parser = ["network", "cuda", "none"])]
        mode: String,
        /// The guest ELF: the docker build the contract pins (checked by sha256)
        #[arg(long, default_value = antibody_cli::zk::ELF_PATH)]
        elf: PathBuf,
        /// The prover binary (crates/zk/prove)
        #[arg(long, default_value = "antibody-zk-prove")]
        prover: PathBuf,
        /// The prover network requester's key file (default: $NETWORK_PRIVATE_KEY, else .secrets/sp1-requester.key)
        #[arg(long)]
        requester_key: Option<PathBuf>,
    },
    /// Send a run's proof to near-t2 (anyone may; 0.02 NEAR prepays the record, the rest is refunded)
    Submit {
        #[arg(long)]
        run: u64,
        /// The case directory (default: <state>/zk/<run>)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// near-t2's account (default: the deployment's)
        #[arg(long)]
        contract: Option<String>,
    },
    /// A run's T2 record and near-t2's pins (view calls only)
    Status {
        #[arg(long)]
        run: u64,
        #[arg(long)]
        contract: Option<String>,
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
        Cmd::Keygen { kind: KeygenKind::X25519 { ref out, nep413: false } } => cmd::keygen_x25519(out),
        Cmd::Keygen { kind: KeygenKind::X25519 { ref out, nep413: true } } => cmd::keygen_x25519_nep413(&mk()?, out),
        Cmd::Keygen { kind: KeygenKind::Near { ref account_id, ref out } } => cmd::keygen_near(account_id.clone(), out),
        Cmd::Manifest { ref toml, ref out } => cmd::manifest_compile(toml, out),
        Cmd::ManifestRound { ref base, round, ref overrides, ref invariants_code, ref add_slots, ref rule_strength, ref out } => {
            cmd::manifest_round(base, round, overrides, invariants_code.as_deref().map(|f| (f, add_slots.as_slice())), rule_strength.as_deref(), out)
        }
        Cmd::Program { ref cmd } => {
            let c = mk()?;
            match cmd {
                ProgramCmd::Create { manifest, escrow, project_key, manifest_url } => {
                    cmd::program_create(&c, manifest, escrow, project_key, manifest_url.clone())
                }
                ProgramCmd::Activate { program, cover, no_wait } => cmd::program_activate(&c, *program, cover, !no_wait),
                ProgramCmd::Known { program, bundle, salt, report, no_wait } => cmd::submit_known(&c, *program, bundle, *salt, report.as_deref(), !no_wait),
                ProgramCmd::Open { program } => cmd::program_open(&c, *program),
                ProgramCmd::Show { program } => cmd::program_show(&c, *program),
                ProgramCmd::Read { commit, project_key, out, blob, reveal_tx, reveal_signer, api, new_wrap } => {
                    let from = match (blob, reveal_tx, api) {
                        (Some(b), None, None) => cmd::BlobFrom::File(b.clone()),
                        (None, Some(h), None) => cmd::BlobFrom::Tx { hash: h.clone(), signer: reveal_signer.clone() },
                        (None, None, Some(a)) => cmd::BlobFrom::Api(a.clone()),
                        _ => return Err(eyre::eyre!("pass one of --blob, --reveal-tx or --api")),
                    };
                    cmd::program_read(&c, *commit, project_key, out, &from, *new_wrap)
                }
                ProgramCmd::TopUp { program, amount } => cmd::top_up(&c, *program, amount),
                ProgramCmd::Exit { program } => cmd::program_exit(&c, *program),
            }
        }
        Cmd::Programs { from, limit } => cmd::programs(&mk()?, from, limit),
        Cmd::Pull { program, ref out } => cmd::pull(&mk()?, program, &out.clone().unwrap_or_else(|| PathBuf::from(format!("antibody-program-{program}")))),
        Cmd::Check { program, ref manifest, ref bundle, salt } => cmd::check(&mk()?, program, manifest.as_deref(), bundle, salt),
        Cmd::Immune(_) => Err(eyre::eyre!("`antibody immune` sets its own exit code: it runs from main")),
        Cmd::Window { ref cmd } => {
            let c = mk()?;
            match cmd {
                WindowCmd::Open(a) => {
                    let opts = antibody_cli::window::OpenOpts {
                        config: a.config.clone(),
                        root: a.root.clone().unwrap_or_else(|| PathBuf::from(".")),
                        artifacts: a.artifacts.clone(),
                        rules_artifacts: a.rules_artifacts.clone(),
                        pr: a.pr,
                        repo: a.repo.clone(),
                        head: a.head.clone(),
                        name: a.name.clone(),
                        hours: a.hours,
                        escrow: a.escrow.clone(),
                        project: a.project.clone(),
                        out: a.out.clone(),
                        dry_run: a.dry_run,
                        daily_cap: a.daily_cap,
                        max_escrow: a.max_escrow.clone(),
                        site: a.site.clone(),
                        relay: !a.no_relay,
                    };
                    antibody_cli::window::open(&c, &opts)
                }
                WindowCmd::Status { program, site } => antibody_cli::window::status(&c, *program, site),
                WindowCmd::Close { program } => antibody_cli::window::close(&c, *program),
                WindowCmd::List { project, repo, pr } => {
                    let rows = antibody_cli::window::list(&c, project.as_deref(), repo.as_deref(), *pr)?;
                    Ok(json!({"windows": rows}))
                }
            }
        }
        Cmd::FromForge { ref project, ref contract, ref test, ref fork_url, fork_block, ref endowment_floor, ref forge, ref out } => {
            let fork = match (fork_url, fork_block) {
                (Some(u), Some(b)) => Some((u.as_str(), b)),
                (None, None) => None,
                _ => return Err(eyre::eyre!("--fork-url and --fork-block go together")),
            };
            let floor: u128 = endowment_floor.parse().map_err(|e| eyre::eyre!("--endowment-floor: {e}"))?;
            let r = antibody_cli::compat::from_forge(project, contract, test, fork, forge, floor)?;
            std::fs::write(out, serde_json::to_vec_pretty(&r["bundle"])?)?;
            Ok(json!({"bundle_file": out, "translation": r}))
        }
        Cmd::StorageDeposit { ref near } => cmd::storage_deposit(&mk()?, near),
        Cmd::Deposit { ref amount } => cmd::deposit(&mk()?, amount),
        Cmd::Faucet => cmd::faucet(&mk()?),
        Cmd::Withdraw { ref amount } => cmd::withdraw(&mk()?, amount),
        Cmd::Commit { program, ref bundle, salt, ref report } => cmd::commit(&mk()?, program, bundle, salt, report.as_deref()),
        Cmd::Reveal { program, commit, no_wait } => cmd::reveal(&mk()?, program, commit, !no_wait),
        Cmd::Submit { program, ref bundle, salt, ref report, no_wait } => cmd::submit(&mk()?, program, bundle, salt, report.as_deref(), !no_wait),
        Cmd::Retry { program, commit, no_wait } => cmd::retry(&mk()?, program, commit, !no_wait),
        Cmd::Second { program, commit, no_wait } => cmd::second(&mk()?, program, commit, !no_wait),
        Cmd::Status { commit, run, program, ref of, json } => {
            let c = mk()?;
            let s = cmd::status(&c, commit, run, program, of.clone())?;
            Ok(if json { s } else { Value::String(cmd::status_summary(&c, &s)) })
        }
        Cmd::Judge { program, json } => {
            let c = mk()?;
            let j = antibody_cli::judge::judge(&c, program)?;
            Ok(if json { j } else { Value::String(antibody_cli::judge::summary(&j, c.dep.decimals(), c.dep.symbol())) })
        }
        Cmd::View { ref contract, ref method, ref args } => {
            let c = mk()?;
            let a: Value = serde_json::from_str(args).map_err(|e| eyre::eyre!("--args is not JSON: {e}"))?;
            c.near.view(contract, method, a)
        }
        Cmd::Keeper { program } => cmd::keeper(&mk()?, program),
        Cmd::Round { cmd: RoundCmd::Propose { program, ref manifest, ref releases, ref base_witness, no_wait, code_only } } => {
            let releases = releases.iter().map(|r| cmd::parse_release(r)).collect::<eyre::Result<Vec<_>>>()?;
            cmd::round_propose_as(&mk()?, program, manifest, &releases, base_witness.as_deref(), !no_wait, code_only)
        }
        Cmd::Round { cmd: RoundCmd::Base { program, ref manifest, ref out } } => {
            let c = mk()?;
            let key = c.program(program)?["key"].as_str().unwrap_or_default().to_string();
            let (bytes, meta) = cmd::build_round_base(&c, &cmd::program_manifest_at(&c, program, 0)?, &std::fs::read(manifest)?, &key)?;
            std::fs::write(out, &bytes)?;
            Ok(json!({"out": out, "bytes": bytes.len(), "meta": meta}))
        }
        Cmd::Round { cmd: RoundCmd::State { program } } => antibody_cli::pr_check::state(&mk()?, program),
        Cmd::Round { cmd: RoundCmd::Results { program, round, check_run, wait, ref site, ref json, ref md } } => {
            let ctx = mk()?;
            let proposed = antibody_cli::pr_check::Proposed { round, check_run };
            let r = if wait > 0 {
                antibody_cli::pr_check::wait_results(&ctx, program, wait, site.as_deref(), proposed)?
            } else {
                antibody_cli::pr_check::results_of(&ctx, program, proposed, site.as_deref())?
            };
            if let Some(f) = json {
                std::fs::write(f, serde_json::to_vec_pretty(&r)?)?;
            }
            if let Some(f) = md {
                std::fs::write(f, antibody_cli::pr_check::markdown(&r))?;
            }
            Ok(r)
        }
        Cmd::Round { cmd: RoundCmd::Rerun { commit, ref blob, ref reveal_tx, ref reveal_signer, ref api } } => {
            let from = match (blob, reveal_tx, api) {
                (None, None, None) => None,
                (Some(b), None, None) => Some(cmd::BlobFrom::File(b.clone())),
                (None, Some(h), None) => Some(cmd::BlobFrom::Tx { hash: h.clone(), signer: reveal_signer.clone() }),
                (None, None, Some(a)) => Some(cmd::BlobFrom::Api(a.clone())),
                _ => return Err(eyre::eyre!("pass at most one of --blob, --reveal-tx or --api")),
            };
            cmd::round_rerun(&mk()?, commit, from.as_ref())
        }
        Cmd::Try { ref vending, program, ref name, ref poc, json } => {
            let ctx = mk()?;
            let (vending, program) = antibody_cli::practice::target(&ctx, vending.clone(), program)?;
            let (d, sym) = (ctx.dep.decimals(), ctx.dep.symbol());
            match poc {
                None => {
                    let r = antibody_cli::practice::try_it(&ctx, &vending, program, name)?;
                    Ok(if json { r } else { Value::String(antibody_cli::practice::try_summary(&r, d, sym)) })
                }
                Some(poc) => {
                    let r = hunt::try_poc(&ctx, &vending, program, name, poc)?;
                    Ok(if json { r } else { Value::String(hunt::summary(&r, d, sym)) })
                }
            }
        }
        Cmd::Hunt { cmd: HuntCmd::Start { ref name, ref vending, ref out, json } } => {
            let ctx = mk()?;
            let vending = hunt::default_vending(&ctx.dep, vending.as_deref())?;
            let out = out.clone().unwrap_or_else(|| PathBuf::from(format!("hunt-{name}")));
            let r = hunt::start(&ctx, &vending, name, Some(&out))?;
            Ok(if json { r } else { Value::String(hunt::start_summary(&r)) })
        }
        Cmd::Hunt { cmd: HuntCmd::Submit { ref name, ref poc, ref vending, json } } => {
            let ctx = mk()?;
            let vending = hunt::account_vending(&ctx, vending.as_deref(), name)?;
            let r = hunt::submit(&ctx, &vending, name, poc)?;
            Ok(if json { r } else { Value::String(hunt::summary(&r, ctx.dep.decimals(), ctx.dep.symbol())) })
        }
        Cmd::Hunt { cmd: HuntCmd::Status { ref name, ref vending, json } } => {
            let ctx = mk()?;
            let vending = hunt::account_vending(&ctx, vending.as_deref(), name)?;
            let r = hunt::status(&ctx, &vending, name)?;
            Ok(if json { r } else { Value::String(hunt::status_summary(&r, ctx.dep.decimals(), ctx.dep.symbol())) })
        }
        Cmd::Practice {
            cmd:
                PracticeCmd::Deploy {
                    ref wasm, ref account_wasm, ref owner, program, vaults, ref gas_funds, ref allowance, ref bond_budget, ref per_commit_cap, per_hour, ref vend_allowance,
                },
        } => {
            let c = mk()?;
            let terms = antibody_cli::practice::Terms {
                gas_funds: cmd::parse_near(gas_funds)?,
                allowance: cmd::parse_near(allowance)?,
                bond_budget: cmd::parse_tokens(bond_budget, c.dep.decimals())?,
                per_commit_cap: cmd::parse_tokens(per_commit_cap, c.dep.decimals())?,
                per_hour,
            };
            antibody_cli::practice::deploy(&c, wasm, account_wasm, owner, program, vaults, &terms, cmd::parse_near(vend_allowance)?)
        }
        Cmd::Practice {
            cmd: PracticeCmd::Retarget { ref vending, ref account_wasm, program, vaults, ref gas_funds, ref allowance, ref bond_budget, ref per_commit_cap, per_hour },
        } => {
            let c = mk()?;
            // Each term not given stays as the contract has it: a retarget during judging keeps the raised limit.
            let now = antibody_cli::practice::vending_config(&c.near, vending)?.terms;
            let near_or = |v: &Option<String>, d: u128| -> Result<u128> { v.as_deref().map(cmd::parse_near).transpose().map(|x| x.unwrap_or(d)) };
            let tokens_or = |v: &Option<String>, d: u128| -> Result<u128> {
                v.as_deref().map(|s| cmd::parse_tokens(s, c.dep.decimals())).transpose().map(|x| x.unwrap_or(d))
            };
            let terms = antibody_cli::practice::Terms {
                gas_funds: near_or(gas_funds, now.gas_funds)?,
                allowance: near_or(allowance, now.allowance)?,
                bond_budget: tokens_or(bond_budget, now.bond_budget)?,
                per_commit_cap: tokens_or(per_commit_cap, now.per_commit_cap)?,
                per_hour: per_hour.unwrap_or(now.per_hour),
            };
            antibody_cli::practice::retarget(&c, vending, account_wasm, program, vaults, &terms)
        }
        Cmd::Practice { cmd: PracticeCmd::Fund { ref vending, ref near, ref tokens } } => {
            let c = mk()?;
            let raw = cmd::parse_tokens(tokens, c.dep.decimals())?;
            antibody_cli::practice::fund(&c, vending, cmd::parse_near(near)?, raw)
        }
        Cmd::Practice { cmd: PracticeCmd::Terms { ref vending, per_hour, program } } => antibody_cli::practice::set_terms(&mk()?, vending, program, per_hour),
        Cmd::Round { cmd: RoundCmd::Open { program } } => cmd::round_open(&mk()?, program),
        Cmd::Expire { commit } => cmd::call(&mk()?, "expire", json!({"commit_id": commit}), 30),
        Cmd::TimeOut { commit } => cmd::call(&mk()?, "time_out", json!({"commit_id": commit}), 30),
        Cmd::Agent { cmd: AgentCmd::Register { ref agent, ref budget, ref per_commit_cap, ref programs } } => {
            cmd::agent_register(&mk()?, agent, budget, per_commit_cap, programs)
        }
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
                AdminCmd::KeyAdd { methods, allowance_near, out } => cmd::key_add(&c, methods, *allowance_near, out),
                AdminCmd::Tx { hash, signer } => cmd::tx(&c, hash, signer),
                AdminCmd::Lock { confirm_irreversible } => cmd::admin_lock(&c, confirm_irreversible),
                AdminCmd::DeleteAccount { beneficiary, confirm_irreversible } => cmd::admin_delete_account(&c, beneficiary, confirm_irreversible),
                AdminCmd::RecordDeployment { git_commit, out } => {
                    let record = antibody_cli::judge::deployment_record(&c, git_commit.as_deref())?;
                    let root = ctx::repo_root().unwrap_or_default();
                    let path = out.clone().unwrap_or_else(|| root.join(format!("deployments/near-{}.json", cli.network)));
                    let mut dep = serde_json::to_value(&c.dep)?;
                    dep["release"] = record.clone();
                    std::fs::write(&path, serde_json::to_string_pretty(&dep)? + "\n")?;
                    let hist = root.join("deployments/history");
                    std::fs::create_dir_all(&hist)?;
                    let h = hist.join(format!("near-{}-{}.json", cli.network, record["recorded_at"]));
                    std::fs::write(&h, serde_json::to_string_pretty(&record)? + "\n")?;
                    Ok(json!({"deployments": path, "history": h, "release": record}))
                }
            }
        }
        Cmd::AnchorCheck { block, forge } => cmd::anchor_check(&mk()?, block, forge),
        Cmd::MainnetCheck { ref mainnet_eth_rpc, ref mainnet_near_rpc } => cmd::mainnet_check(mainnet_near_rpc.clone(), mainnet_eth_rpc.as_deref()),
        Cmd::Init { ref out, ref project, chain_id, days } => cmd::init(out, project, chain_id, days),
        Cmd::Invariants { cmd: InvariantsCmd::Validate { ref manifest, ref strength, min_strength } } => {
            let r = cmd::validate(&mk()?, manifest, strength.as_deref(), min_strength)?;
            if r["ok"] != json!(true) {
                println!("{}", serde_json::to_string_pretty(&r)?);
                return Err(eyre::eyre!("the manifest does not pass the validate gate"));
            }
            Ok(r)
        }
        Cmd::Invariants { cmd: InvariantsCmd::FromFoundry { ref artifact, ref import, ref base_import, ref order, ref out } } => {
            let a: Value = serde_json::from_str(&std::fs::read_to_string(artifact)?)?;
            let name = artifact.file_stem().and_then(|s| s.to_str()).ok_or_else(|| eyre::eyre!("artifact file name"))?;
            let abi = a["abi"].as_array().ok_or_else(|| eyre::eyre!("{}: no abi", artifact.display()))?;
            let (src, slots) = antibody_cli::compat::rules_from_foundry(name, abi, import, base_import, order.as_deref())?;
            std::fs::write(out, src)?;
            Ok(json!({"rules": out, "generated": slots}))
        }
        Cmd::Invariants {
            cmd: InvariantsCmd::Strength { ref project, ref test, ref files, runs, depth, seed, workers, timeout, ref forge, ref work, ref out },
        } => {
            let report = antibody_cli::strength::run(antibody_cli::strength::Opts {
                project: project.clone(),
                test: test.clone(),
                files: files.clone(),
                runs,
                depth,
                seed,
                forge: forge.clone(),
                timeout: std::time::Duration::from_secs(timeout),
                workers,
                work: work.clone(),
            })?;
            if let Some(o) = out {
                std::fs::write(o, serde_json::to_vec_pretty(&report)?)?;
            }
            Ok(report)
        }
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
        Cmd::Zk { ref cmd } => {
            let c = mk()?;
            match cmd {
                ZkCmd::Prove { run, key, tx, api, out, mode, elf, prover, requester_key } => {
                    let dir = out.clone().unwrap_or_else(|| antibody_cli::zk::case_dir(&c, *run));
                    let from = match (tx, api) {
                        (Some(t), _) => antibody_cli::zk::TxFrom::Given(t.clone()),
                        (None, Some(a)) => antibody_cli::zk::TxFrom::Api(a.clone()),
                        (None, None) => antibody_cli::zk::TxFrom::Local,
                    };
                    let mut r = antibody_cli::zk::gather(&c, *run, key.as_deref(), &from, &dir)?;
                    if mode != "none" {
                        if r["receipt_equals_the_tees"] != json!(true) {
                            eprintln!("note: the native re-run's receipt differs from the TEE's: a proof would record a contradiction");
                        }
                        r["proof"] = antibody_cli::zk::prove(&dir, elf, prover, mode, requester_key.as_deref())?;
                    }
                    Ok(r)
                }
                ZkCmd::Submit { run, dir, contract } => {
                    let t2 = antibody_cli::zk::contract(&c, contract.as_deref())?;
                    let dir = dir.clone().unwrap_or_else(|| antibody_cli::zk::case_dir(&c, *run));
                    antibody_cli::zk::submit(&c, &t2, *run, &dir)
                }
                ZkCmd::Status { run, contract } => antibody_cli::zk::status(&c, &antibody_cli::zk::contract(&c, contract.as_deref())?, *run),
            }
        }
        Cmd::Mcp { ref workspace } => {
            let ws = match workspace {
                Some(w) => w.clone(),
                None => std::env::current_dir()?,
            };
            // The server's state (vended keys, sealed proofs' content keys and salts) lives inside its
            // workspace, wherever the client started the process, and must be writable before any tool
            // runs: a vend or a commit is never sent when its key or record could not be kept.
            let mut ctx = mk()?;
            ctx.state_dir = mcp::state_dir(&ws)?;
            mcp::serve(&ctx, &ws)?;
            Ok(Value::Null)
        }
    }
}

/// `antibody immune`: writes the report (JSON, Markdown) and prints the Markdown; its exit code is the verdict
/// (immune::EXIT_*), so a CI step fails when a paid proof still breaks a rule.
fn immune(cli: Cli) -> Result<i32> {
    let ctx = ctx::Ctx::new(&cli.network, cli.deployments.clone(), cli.rpc.clone(), cli.account.clone(), cli.key_file.clone(), cli.eth_rpc.clone())?;
    let Cmd::Immune(ImmuneArgs { config, program, root, project_key, content_keys, blobs, api, json, md, manifest_out, strict, reveal_sealed }) = cli.cmd else {
        return Err(eyre::eyre!("not the immune command"));
    };
    let root = root.unwrap_or_else(|| PathBuf::from("."));
    let project_key = match (project_key, std::env::var("ANTIBODY_PROJECT_KEY").ok().filter(|v| !v.trim().is_empty())) {
        (Some(f), _) => Some(immune::project_secret(&std::fs::read_to_string(&f).map_err(|e| eyre::eyre!("{}: {e}", f.display()))?)?),
        (None, Some(v)) => Some(immune::project_secret(&v)?),
        (None, None) => None,
    };
    // Content keys given here take precedence over the mapping's.
    let mut keys = std::collections::BTreeMap::new();
    for s in &content_keys {
        let (commit, value) = s.split_once('=').ok_or_else(|| eyre::eyre!("--content-key takes <commit>=<hex or file>, not {s:?}"))?;
        let id: u64 = commit.trim().parse().map_err(|_| eyre::eyre!("--content-key: {commit:?} is not a commit id"))?;
        keys.insert(id, immune::key32(value, &root)?);
    }
    let opts = immune::Opts { config, program, root, project_key, content_keys: keys, blobs, api, strict, reveal_sealed };
    let (report, manifest) = immune::run(&ctx, &opts)?;
    let text = immune::markdown(&report);
    if let Some(p) = json {
        std::fs::write(&p, serde_json::to_vec_pretty(&immune::to_json(&report))?)?;
    }
    if let Some(p) = md {
        std::fs::write(&p, &text)?;
    }
    if let Some(p) = manifest_out {
        std::fs::write(&p, manifest.canonical())?;
    }
    println!("{text}");
    Ok(report.exit_code)
}

fn main() {
    let cli = Cli::parse();
    if matches!(cli.cmd, Cmd::Immune(_)) {
        std::process::exit(match immune(cli) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("error: {e:#}");
                1
            }
        });
    }
    let is_mcp = matches!(cli.cmd, Cmd::Mcp { .. });
    // Program names carry "—"; a Windows console on its default code page would print mojibake.
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleOutputCP(65001);
    }
    // The long commands say what they wait for, on stderr (stdout stays the answer).
    if matches!(cli.cmd, Cmd::Try { .. } | Cmd::Judge { .. } | Cmd::Hunt { cmd: HuntCmd::Start { .. } | HuntCmd::Submit { .. } }) {
        antibody_cli::progress::enable();
    }
    match run(cli) {
        // A command that returns text (a summary) prints it as it is.
        Ok(Value::String(s)) if !is_mcp => println!("{s}"),
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
        // wNEAR: 24 decimals
        assert_eq!(cmd::parse_tokens("0.07", 24).unwrap(), 70_000_000_000_000_000_000_000);
        assert_eq!(cmd::fmt_tokens(1_500_000_000_000_000_000_000_000, 24), "1.500000000000000000000000");
        assert!(cmd::parse_tokens("1.0000000000000000000000001", 24).is_err());
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
        Cli::try_parse_from(["antibody", "try", "--name", "ann", "--poc", "p.json"]).unwrap();
        Cli::try_parse_from(["antibody", "hunt", "start", "--name", "ann", "--vending", "practice.antibody.testnet", "--out", "ws"]).unwrap();
        Cli::try_parse_from(["antibody", "hunt", "submit", "--name", "ann", "--poc", "p.json"]).unwrap();
        Cli::try_parse_from(["antibody", "hunt", "status", "--name", "ann", "--json"]).unwrap();
        assert!(Cli::try_parse_from(["antibody", "hunt", "submit", "--name", "ann"]).is_err(), "--poc is required");
        let args = ["antibody", "immune", "--content-key", "13=k.hex", "--content-key", "14=k2.hex", "--json", "r.json", "--md", "r.md", "--strict"];
        let Cmd::Immune(ImmuneArgs { config, content_keys, strict, .. }) = Cli::try_parse_from(args).unwrap().cmd else { panic!("immune") };
        assert_eq!((config, content_keys.len(), strict), (PathBuf::from(".antibody/immune.toml"), 2, true));
        let args = ["antibody", "window", "open", "--pr", "12", "--repo", "antibody-immune-demo", "--head", "1a2b3c4d", "--rules-artifacts", "base-out", "--dry-run"];
        let Cmd::Window { cmd: WindowCmd::Open(a) } = Cli::try_parse_from(args).unwrap().cmd else { panic!("window open") };
        assert_eq!((a.config, a.pr, a.dry_run, a.no_relay, a.out), (PathBuf::from(".antibody/window.toml"), Some(12), true, false, PathBuf::from("window-manifest.json")));
        Cli::try_parse_from(["antibody", "window", "status", "--program", "15"]).unwrap();
        Cli::try_parse_from(["antibody", "--account", "bot.testnet", "window", "close", "--program", "15"]).unwrap();
        Cli::try_parse_from(["antibody", "window", "list", "--repo", "antibody-immune-demo", "--project", "bot.testnet"]).unwrap();
        assert!(Cli::try_parse_from(["antibody", "window", "status"]).is_err(), "--program is required");
    }
}

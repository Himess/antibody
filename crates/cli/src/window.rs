//! `antibody window` (docs/immune.md, "Bounty windows"): a bounty window per pull request.
//!
//! A window is a fork-mode program (docs/fork-mode.md) for a pull request's build. The repository commits a
//! window config (`.antibody/window.toml`): the contracts to deploy, in order, with their constructor arguments,
//! the calls that seed them, the rules contract, and what each rule pays. `window open` turns the build's Foundry
//! artifacts into the manifest's deployment (transactions from the fixed deployer, nobody's key), anchors it on
//! the newest Sepolia block NEAR's Ethereum light client already holds, so nothing waits, and creates, activates
//! and opens the program as the project (a bot account). Low, Medium and High only: a fork-mode program has
//! nothing to pause. `window status` reads a window's verdict once its deadline has passed; `window close`
//! exits it when the windows allow and withdraws what its escrow has left.
//!
//! The same words wherever a window is shown: a window is a fork-mode program for the pull request's build; its
//! escrow is wNEAR on testnet, which has no value; proofs committed before its deadline are judged and paid.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use antibody_verifier::{
    facts::sha256,
    manifest::{
        deployer, Anchor, Capabilities, Deploy, DeployTx, ImpactConfig, Invariants, Manifest, PricedToken, ProgramInfo, Severity, Slot, Target,
        MANIFEST_SCHEMA_V2,
    },
};
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{cmd, ctx::Ctx, immune, tee};

pub const DEFAULT_CONFIG: &str = ".antibody/window.toml";
/// The site whose program pages the statuses link to.
pub const DEFAULT_SITE: &str = "https://antibody-near.vercel.app";
/// The commit status every window sets on its pull request's head.
pub const STATUS_CONTEXT: &str = "antibody/bounty-window (testnet)";
/// GitHub keeps a commit status's description to 140 characters.
pub const MAX_STATUS: usize = 140;
/// What a window's terms say; its sha256 is the manifest's `terms_sha256`, which every commit names.
pub const WINDOW_TERMS: &str = "Bounty window terms: a window is a fork-mode program for a pull request's build, whose contracts exist only \
inside the verifier. Payouts in wNEAR on NEAR testnet, a testnet token with no value. Proofs committed before the deadline are judged \
and paid by the program's rules; nothing else is paid.";

const DEFAULT_HOURS: f64 = 24.0;
const DEFAULT_ESCROW: &str = "0.03";
const DEFAULT_DAILY_CAP: u32 = 4;
/// NEAR the project keeps liquid for a window: the creation's and the activation's gas set aside (300 and 200
/// Tgas), the NEAR credit for storage and the run, the uploads and a T1 relay.
const DEFAULT_MIN_NEAR: &str = "1";
const DEFAULT_BOND: &str = "0.001";
const DEFAULT_CHALLENGE_BOND: &str = "0.005";
/// The shortest window `open` accepts: activation takes a few minutes.
const MIN_HOURS: f64 = 0.1;
/// The activation run's own relayer gets this long to make it T1 before the project relays the quote itself.
const RELAY_AFTER: Duration = Duration::from_secs(45);
const T1_WAIT: Duration = Duration::from_secs(300);

// ------------------------------------------------------------------------------ names

/// A window's program name, parsed: `PR #12 window (antibody-immune-demo @ 1a2b3c4)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowName {
    pub pr: u64,
    pub repo: String,
    /// The pull request head's short commit hash: one window per head.
    pub head: Option<String>,
}

/// The program name a window gets: the pull request, the repository and the head's short hash.
pub fn window_name(pr: u64, repo: &str, head: Option<&str>) -> String {
    match head {
        Some(h) => format!("PR #{pr} window ({repo} @ {h})"),
        None => format!("PR #{pr} window ({repo})"),
    }
}

fn repo_ok(r: &str) -> bool {
    !r.is_empty() && r.len() <= 100 && r.chars().all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
}

fn head_ok(h: &str) -> bool {
    (7..=40).contains(&h.len()) && h.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// A window's name, read back; `None` for any other program.
pub fn parse_window_name(name: &str) -> Option<WindowName> {
    let rest = name.strip_prefix("PR #")?;
    let (num, rest) = rest.split_once(" window (")?;
    if num.is_empty() || num.len() > 9 || !num.bytes().all(|b| b.is_ascii_digit()) || num.starts_with('0') {
        return None;
    }
    let inner = rest.strip_suffix(')')?;
    let (repo, head) = match inner.split_once(" @ ") {
        Some((r, h)) => (r, Some(h)),
        None => (inner, None),
    };
    if !repo_ok(repo) || head.is_some_and(|h| !head_ok(h)) {
        return None;
    }
    Some(WindowName { pr: num.parse().ok()?, repo: repo.to_string(), head: head.map(String::from) })
}

/// Whether a program is a pull request's bounty window, by the name a window gets. A label only, like
/// `judge::is_drill`: a program's name is its project's to choose, and nothing else depends on it.
pub fn is_window(name: &str) -> bool {
    parse_window_name(name).is_some()
}

/// A commit hash's short form, as a window's name carries it (7 lowercase hex digits).
pub fn short_head(sha: &str) -> Result<String> {
    let s = sha.trim().to_ascii_lowercase();
    if !s.chars().all(|c| c.is_ascii_hexdigit()) || s.len() < 7 || s.len() > 64 {
        return Err(eyre!("{sha:?} is not a commit hash"));
    }
    Ok(s[..7].to_string())
}

// ------------------------------------------------------------------------------ the config

/// `.antibody/window.toml`: what a pull request's window deploys, in order, and what it pays.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Foundry's output directory.
    #[serde(default = "default_artifacts")]
    pub artifacts: PathBuf,
    /// The repository's name in each window's program name (default: `--repo`).
    pub repository: Option<String>,
    /// What the deployment is, first in the program's description.
    pub description: Option<String>,
    /// The window's length: its deadline is this many hours after its manifest is written, just before creation.
    pub hours: Option<f64>,
    /// The escrow, in whole units of the payout token (wNEAR on testnet).
    pub escrow: Option<String>,
    /// Windows the project may open in 24 hours.
    pub daily_cap: Option<u32>,
    /// NEAR the project must hold, liquid, before a window opens.
    pub min_near: Option<String>,
    /// The bond each commit locks, in whole units of the payout token.
    pub bond: Option<String>,
    pub challenge_bond: Option<String>,
    /// The deployment: contracts to create and calls to make, in order.
    #[serde(default, rename = "tx")]
    pub txs: Vec<TxSpec>,
    /// The rules contract, created last: its runtime code (with its immutables) is what the program commits.
    pub rules: RulesSpec,
    /// The rules' slots: one bit each, Low, Medium or High.
    #[serde(default, rename = "slot")]
    pub slots: Vec<SlotSpec>,
    pub payouts: PayoutSpec,
    #[serde(default)]
    pub impact: ImpactSpec,
    #[serde(default)]
    pub capabilities: CapSpec,
    #[serde(default)]
    pub windows: WindowsSpec,
}

fn default_artifacts() -> PathBuf {
    PathBuf::from("out")
}

/// One transaction of the deployment: a creation (`name` with `contract` or `artifact`) or a call (`call` with
/// `function` or `data`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TxSpec {
    /// A creation: the contract's name in the manifest (a target unless `target = false`), and how later
    /// arguments name its address.
    pub name: Option<String>,
    /// A creation: the contract's Foundry artifact, `<artifacts>/<File>.sol/<contract>.json`.
    pub contract: Option<String>,
    /// A creation: the artifact's path, when the name is ambiguous.
    pub artifact: Option<PathBuf>,
    /// Constructor or call arguments: an address (`0x…`, `deployer`, or an earlier contract's name), a number
    /// (an integer, or a decimal or `0x` string), a bool, `bytesN` as hex, `bytes` as hex, or a `string`.
    #[serde(default)]
    pub args: Vec<toml::Value>,
    /// The ABI-encoded constructor arguments instead of `args`.
    pub args_hex: Option<String>,
    /// A creation: whether the contract is one of the program's targets (default true).
    pub target: Option<bool>,
    /// A call: the contract it calls (an earlier contract's name, or `0x…`).
    pub call: Option<String>,
    /// A call: the function's signature, `mint(address,uint256)`; its arguments are `args`.
    pub function: Option<String>,
    /// A call: the calldata itself (hex) instead of `function` and `args`.
    pub data: Option<String>,
    /// Wei sent with the transaction.
    pub value: Option<toml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesSpec {
    pub contract: Option<String>,
    pub artifact: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<toml::Value>,
    pub args_hex: Option<String>,
    /// Gas for each `snapshot()` or `check()` evaluation.
    pub gas_limit: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotSpec {
    pub bit: u8,
    pub name: String,
    /// `low`, `medium` or `high`.
    pub severity: String,
}

/// What each severity pays, in whole units of the payout token (Critical: never, in a window).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayoutSpec {
    pub low: Option<String>,
    pub medium: Option<String>,
    pub high: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactSpec {
    /// Contracts whose losses count as the protocol's (default: every target).
    pub protocol_accounts: Option<Vec<String>>,
    pub native_usd: Option<String>,
    #[serde(default, rename = "token")]
    pub tokens: Vec<TokenSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenSpec {
    pub symbol: String,
    /// The token: a deployed contract's name, or `0x…`.
    pub contract: String,
    pub decimals: u8,
    /// Its committed price in USD per whole token.
    pub usd: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapSpec {
    pub max_senders: Option<u8>,
    pub max_endowment_wei: Option<String>,
    pub max_txs: Option<u32>,
    pub max_gas_per_tx: Option<u64>,
    pub max_total_gas: Option<u64>,
    pub max_entry_calls: Option<u32>,
}

/// Settlement windows in seconds (spec §3.2); the deadline comes from `hours`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsSpec {
    pub reveal: Option<u64>,
    pub challenge: Option<u64>,
    pub resolve: Option<u64>,
    pub infra_grace: Option<u64>,
    pub run_timeout: Option<u64>,
    pub exit_grace: Option<u64>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path).map_err(|e| eyre!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| eyre!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Config> {
        toml::from_str(text).map_err(|e| eyre!("{e}"))
    }
}

// ------------------------------------------------------------------------------ ABI encoding

/// A function signature's canonical form and its parameter types: `mint(address, uint256)` gives
/// `mint(address,uint256)`. Tuples and arrays are not taken: give such a call's `data`.
pub fn parse_signature(sig: &str) -> Result<(String, Vec<String>)> {
    let s: String = sig.chars().filter(|c| !c.is_whitespace()).collect();
    let (name, rest) = s.split_once('(').ok_or_else(|| eyre!("{sig:?} is not a function signature: name(type,…)"))?;
    let inner = rest.strip_suffix(')').ok_or_else(|| eyre!("{sig:?} is not a function signature: name(type,…)"))?;
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') || name.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(eyre!("{sig:?}: {name:?} is not a function name"));
    }
    if inner.contains(['(', ')', '[', ']']) {
        return Err(eyre!("{sig:?}: tuple and array parameters are not encoded here: give the call's `data`"));
    }
    let types: Vec<String> = if inner.is_empty() { vec![] } else { inner.split(',').map(String::from).collect() };
    Ok((format!("{name}({})", types.join(",")), types))
}

fn dynamic(t: &str) -> bool {
    t == "string" || t == "bytes"
}

/// ABI-encodes arguments (`abi.encode`): static types as `antibody immune` encodes them, and `string` and `bytes`
/// (hex) in the tail. `address` turns an address given by name into its address.
pub fn encode_abi(types: &[String], values: &[toml::Value], address: &dyn Fn(&str) -> Result<Address, String>) -> Result<Vec<u8>> {
    if types.len() != values.len() {
        return Err(eyre!("{} arguments for {} parameters ({})", values.len(), types.len(), types.join(", ")));
    }
    let mut head: Vec<[u8; 32]> = Vec::with_capacity(types.len());
    let mut tails: Vec<(usize, Vec<u8>)> = Vec::new();
    for (i, (t, v)) in types.iter().zip(values).enumerate() {
        if t.contains('[') || t.starts_with('(') {
            return Err(eyre!("argument {i} ({t}): arrays and tuples are not encoded here: give `args_hex`, or a call's `data`"));
        }
        if dynamic(t) {
            let bytes = match (t.as_str(), v.as_str()) {
                ("string", Some(s)) => s.as_bytes().to_vec(),
                ("bytes", Some(s)) => hex::decode(s.trim_start_matches("0x")).map_err(|_| eyre!("argument {i} (bytes): not hex"))?,
                _ => return Err(eyre!("argument {i} ({t}): a string")),
            };
            head.push([0u8; 32]);
            tails.push((i, bytes));
        } else {
            head.push(immune::encode_word_with(t, v, address).map_err(|e| eyre!("argument {i} ({t}): {e}"))?);
        }
    }
    let mut tail = Vec::new();
    for (i, bytes) in tails {
        head[i] = U256::from(32 * types.len() + tail.len()).to_be_bytes::<32>();
        tail.extend(U256::from(bytes.len()).to_be_bytes::<32>());
        tail.extend(&bytes);
        tail.resize(tail.len() + (32 - bytes.len() % 32) % 32, 0);
    }
    let mut out: Vec<u8> = head.concat();
    out.extend(tail);
    Ok(out)
}

fn wei(v: &Option<toml::Value>) -> Result<U256> {
    match v {
        None => Ok(U256::ZERO),
        Some(toml::Value::Integer(i)) if *i >= 0 => Ok(U256::from(*i as u64)),
        Some(toml::Value::String(s)) => s.parse::<U256>().map_err(|_| eyre!("value {s:?} is not a number of wei")),
        Some(other) => Err(eyre!("value {other} is not a number of wei")),
    }
}

fn unhex(s: &str, what: &str) -> Result<Vec<u8>> {
    hex::decode(s.trim().trim_start_matches("0x")).map_err(|e| eyre!("{what}: not hex ({e})"))
}

// ------------------------------------------------------------------------------ the manifest

/// A contract the deployment creates.
#[derive(Debug, Clone, Serialize)]
pub struct Deployed {
    pub name: String,
    pub address: Address,
    /// Its transaction in the deployment (and the deployer's nonce): it creates at `deployer.create(tx)`.
    pub tx: usize,
    pub from: String,
    pub target: bool,
    pub code_hash: B256,
}

/// What `plan` needs besides the config: the program's identity, its deadline and escrow.
#[derive(Debug, Clone)]
pub struct PlanArgs {
    pub name: String,
    pub project: String,
    pub description: String,
    pub deadline: u64,
    pub escrow: u128,
    /// The payout token's decimals (24: wNEAR).
    pub decimals: u32,
    /// Relative paths start here.
    pub root: PathBuf,
    pub artifacts: PathBuf,
    /// Where the rules contract's artifact is found (the base branch's build, in the workflow).
    pub rules_artifacts: PathBuf,
    /// NEAR's clock, for the settlement's bounds.
    pub now: u64,
}

#[derive(Debug, Clone)]
pub struct Planned {
    pub manifest: Manifest,
    pub deployed: Vec<Deployed>,
    pub rules: Deployed,
    /// One line per deployment transaction, in order.
    pub txs: Vec<String>,
}

fn severity(s: &str) -> Result<Severity> {
    match s.to_ascii_lowercase().as_str() {
        "low" => Ok(Severity::Low),
        "medium" => Ok(Severity::Medium),
        "high" => Ok(Severity::High),
        "critical" => Err(eyre!("a window pays Low, Medium and High only: a fork-mode program has nothing to pause")),
        other => Err(eyre!("severity {other:?}: low, medium or high")),
    }
}

fn label(path: &Path, root: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string().replace('\\', "/")
}

/// An artifact by contract name under `dir`, or by path.
fn artifact_of(contract: &Option<String>, artifact: &Option<PathBuf>, root: &Path, dir: &Path, what: &str) -> Result<immune::Artifact> {
    let path = match (contract, artifact) {
        (_, Some(p)) => root.join(p),
        (Some(c), None) => immune::find_artifact(&root.join(dir), c)?,
        (None, None) => return Err(eyre!("{what}: give `contract` (or `artifact`)")),
    };
    immune::read_artifact(&path)
}

/// The window's settlement terms (spec §3.2): Low, Medium and High as the config prices them, every T1-confirmed
/// finding up to the largest payout paid in seconds, short windows, no pause, no gap lane, no known issues.
pub fn settlement(cfg: &Config, deadline: u64, escrow: u128, decimals: u32) -> Result<antibody_rules::settlement::Settlement> {
    use antibody_rules::settlement::{Fees, Instant, Known, Pause, Payouts, SWindows, Settlement};
    let tokens = |v: &Option<String>, d: &str, what: &str| cmd::parse_tokens(v.as_deref().unwrap_or(d), decimals).map_err(|e| eyre!("{what}: {e}"));
    let low = tokens(&cfg.payouts.low, "0", "payouts.low")?;
    let medium = tokens(&cfg.payouts.medium, "0", "payouts.medium")?;
    let high = tokens(&cfg.payouts.high, "0", "payouts.high")?;
    let largest = low.max(medium).max(high);
    if largest == 0 {
        return Err(eyre!("the window pays nothing: set payouts.low, payouts.medium or payouts.high"));
    }
    if escrow < largest {
        return Err(eyre!(
            "the escrow ({}) is below the largest payout ({}): one finding could not be paid",
            cmd::human_tokens(escrow, decimals),
            cmd::human_tokens(largest, decimals)
        ));
    }
    for s in &cfg.slots {
        let pays = match severity(&s.severity)? {
            Severity::Low => low,
            Severity::Medium => medium,
            _ => high,
        };
        if pays == 0 {
            return Err(eyre!("slot {} ({}) is {}, which this window pays nothing for", s.bit, s.name, s.severity));
        }
    }
    let w = &cfg.windows;
    Ok(Settlement {
        payouts: Payouts {
            critical: "0".into(),
            high: high.to_string(),
            medium: medium.to_string(),
            low: low.to_string(),
            impact_bps: 0,
            impact_cap: "0".into(),
            decimals: decimals as u8,
        },
        instant: Instant { cap: largest.to_string(), budget: escrow.to_string(), window_secs: 3_600 },
        fees: Fees { protocol_bps: 0, release: "0".into() },
        bond: tokens(&cfg.bond, DEFAULT_BOND, "bond")?.to_string(),
        challenge_bond: tokens(&cfg.challenge_bond, DEFAULT_CHALLENGE_BOND, "challenge_bond")?.to_string(),
        windows: SWindows {
            reveal: w.reveal.unwrap_or(600),
            challenge: w.challenge.unwrap_or(300),
            resolve: w.resolve.unwrap_or(300),
            infra_grace: w.infra_grace.unwrap_or(600),
            run_timeout: w.run_timeout.unwrap_or(240),
            round_min: 120,
            rerun_window: 600,
            disclosure_max_delay: 600,
            deadline,
            exit_grace: w.exit_grace.unwrap_or(600),
        },
        slot_budgets: vec![],
        unevaluable: json!("refund"),
        gap: None,
        known: Known { max_count: 0, max_share_bps: 0 },
        pause: Pause { slots: vec![], min_impact_usd_micro: "0".into() },
        terms_sha256: hex::encode(sha256(WINDOW_TERMS.as_bytes())),
    })
}

/// The window's manifest for this build: the config's transactions from the fixed deployer, the rules created
/// last, then `code_at` (the deployment run at the anchor: `cmd::deployed_code`) for the targets' code hashes and
/// the rules' runtime code. Checked as near-antibody checks a manifest before anything is sent.
pub fn plan(
    cfg: &Config,
    a: &PlanArgs,
    anchor: Anchor,
    code_at: impl FnOnce(&Manifest, &[Address]) -> Result<Vec<(B256, Bytes)>>,
) -> Result<Planned> {
    let d = deployer();
    let mut names: Vec<(String, Address)> = Vec::new();
    let mut txs: Vec<DeployTx> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut deployed: Vec<Deployed> = Vec::new();
    fn resolve(names: &[(String, Address)], d: Address, s: &str) -> Result<Address, String> {
        if s == "deployer" {
            return Ok(d);
        }
        if s.starts_with("0x") {
            return s.parse().map_err(|_| format!("{s} is not an address"));
        }
        names.iter().find(|(n, _)| n == s).map(|(_, a)| *a).ok_or_else(|| {
            let known: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
            format!("{s} is not `deployer`, an address, or a contract created before it ({})", if known.is_empty() { "none yet".into() } else { known.join(", ") })
        })
    }
    for (i, t) in cfg.txs.iter().enumerate() {
        let value = wei(&t.value).map_err(|e| eyre!("tx {i}: {e}"))?;
        match &t.call {
            None => {
                let name = t.name.clone().ok_or_else(|| eyre!("tx {i}: a creation names its contract (`name`), or a call names `call`"))?;
                if name.is_empty() || name.len() > 64 || name == "deployer" || name.starts_with("0x") || names.iter().any(|(n, _)| *n == name) {
                    return Err(eyre!("tx {i}: {name:?} cannot name a contract (empty, too long, `deployer`, an address, or taken)"));
                }
                if t.function.is_some() || t.data.is_some() {
                    return Err(eyre!("tx {i} ({name}): `function` and `data` belong to a call (`call = \"<contract>\"`)"));
                }
                let art = artifact_of(&t.contract, &t.artifact, &a.root, &a.artifacts, &format!("tx {i} ({name})"))?;
                let args = match (&t.args_hex, t.args.is_empty()) {
                    (Some(_), false) => return Err(eyre!("tx {i} ({name}): give `args` or `args_hex`, not both")),
                    (Some(h), true) => unhex(h, &format!("tx {i} ({name}) args_hex"))?,
                    (None, _) => encode_abi(&art.constructor, &t.args, &|s| resolve(&names, d, s)).map_err(|e| eyre!("tx {i} ({name}): {e}"))?,
                };
                let address = d.create(i as u64);
                let from = format!("{} ({})", t.contract.clone().unwrap_or_else(|| "artifact".into()), label(&art.path, &a.root));
                lines.push(format!("{i}: create {name} = {from} at {address}"));
                deployed.push(Deployed { name: name.clone(), address, tx: i, from, target: t.target.unwrap_or(true), code_hash: B256::ZERO });
                names.push((name, address));
                txs.push(DeployTx { to: None, value, data: [art.creation.as_slice(), args.as_slice()].concat().into() });
            }
            Some(to) => {
                if t.name.is_some() || t.contract.is_some() || t.artifact.is_some() || t.target.is_some() || t.args_hex.is_some() {
                    return Err(eyre!("tx {i}: a call takes `call`, `function` and `args` (or `data`), and `value`"));
                }
                let addr = resolve(&names, d, to).map_err(|e| eyre!("tx {i}: {e}"))?;
                let (data, what) = match (&t.function, &t.data) {
                    (Some(f), None) => {
                        let (sig, types) = parse_signature(f).map_err(|e| eyre!("tx {i}: {e}"))?;
                        let mut data = keccak256(sig.as_bytes())[..4].to_vec();
                        data.extend(encode_abi(&types, &t.args, &|s| resolve(&names, d, s)).map_err(|e| eyre!("tx {i} ({to}.{sig}): {e}"))?);
                        (data, sig)
                    }
                    (None, Some(h)) if t.args.is_empty() => (unhex(h, &format!("tx {i} data"))?, "calldata".to_string()),
                    _ => return Err(eyre!("tx {i}: a call gives `function` (with `args`) or `data`")),
                };
                lines.push(format!("{i}: call {to}.{what}"));
                txs.push(DeployTx { to: Some(addr), value, data: data.into() });
            }
        }
    }
    // The rules, last: their constructor sees the whole deployment, and their runtime code is taken from here.
    let n = cfg.txs.len();
    let r = &cfg.rules;
    let rart = artifact_of(&r.contract, &r.artifact, &a.root, &a.rules_artifacts, "rules")?;
    let rargs = match (&r.args_hex, r.args.is_empty()) {
        (Some(_), false) => return Err(eyre!("rules: give `args` or `args_hex`, not both")),
        (Some(h), true) => unhex(h, "rules args_hex")?,
        (None, _) => encode_abi(&rart.constructor, &r.args, &|s| resolve(&names, d, s)).map_err(|e| eyre!("rules: {e}"))?,
    };
    let rules_at = d.create(n as u64);
    let rfrom = format!("{} ({})", r.contract.clone().unwrap_or_else(|| "artifact".into()), label(&rart.path, &a.root));
    lines.push(format!("{n}: create the rules = {rfrom} at {rules_at}"));
    txs.push(DeployTx { to: None, value: U256::ZERO, data: [rart.creation.as_slice(), rargs.as_slice()].concat().into() });

    let targets: Vec<Target> =
        deployed.iter().filter(|x| x.target).map(|x| Target { name: x.name.clone(), address: x.address, code_hash: B256::ZERO }).collect();
    if targets.is_empty() || targets.len() > 32 {
        return Err(eyre!("a window has 1 to 32 targets ({} given)", targets.len()));
    }
    if cfg.slots.is_empty() || cfg.slots.len() > 64 {
        return Err(eyre!("the rules have 1 to 64 slots ([[slot]]), {} given", cfg.slots.len()));
    }
    let mut slots = Vec::new();
    for s in &cfg.slots {
        if s.bit == 255 || slots.iter().any(|x: &Slot| x.bit == s.bit) || s.name.is_empty() || s.name.len() > 64 {
            return Err(eyre!("slot {} ({:?}): bits 0 to 254, each once, with a name", s.bit, s.name));
        }
        slots.push(Slot { bit: s.bit, name: s.name.clone(), severity: severity(&s.severity).map_err(|e| eyre!("slot {} ({}): {e}", s.bit, s.name))? });
    }
    let addr_of = |s: &str| resolve(&names, d, s).map_err(|e| eyre!(e));
    let protocol_accounts = match &cfg.impact.protocol_accounts {
        Some(list) => list.iter().map(|s| addr_of(s)).collect::<Result<Vec<_>>>()?,
        None => targets.iter().map(|t| t.address).collect(),
    };
    let mut tokens = Vec::new();
    for t in &cfg.impact.tokens {
        antibody_verifier::manifest::usd_to_micro(&t.usd).map_err(|e| eyre!("token {}: {e}", t.symbol))?;
        tokens.push(PricedToken { symbol: t.symbol.clone(), address: addr_of(&t.contract)?, decimals: t.decimals, usd: t.usd.clone() });
    }
    let native_usd = cfg.impact.native_usd.clone().unwrap_or_else(|| "2000".into());
    antibody_verifier::manifest::usd_to_micro(&native_usd).map_err(|e| eyre!("impact.native_usd: {e}"))?;
    let c = &cfg.capabilities;
    let terms = settlement(cfg, a.deadline, a.escrow, a.decimals)?;
    let mut m = Manifest {
        schema: MANIFEST_SCHEMA_V2.into(),
        program: ProgramInfo { name: a.name.clone(), project: a.project.clone(), description: a.description.clone() },
        anchor,
        targets,
        invariants: Invariants {
            // Ignored by the verifier, which serves the rules at an address it derives per program (spec §5.7).
            address: Address::from_slice(&keccak256(b"antibody/invariants/v1")[12..]),
            runtime_code: Bytes::new(),
            runtime_code_hash: keccak256([]),
            gas_limit: r.gas_limit.unwrap_or(5_000_000),
            slots,
        },
        impact: ImpactConfig { protocol_accounts, native_usd, tokens },
        capabilities: Capabilities {
            max_senders: c.max_senders.unwrap_or(2),
            max_endowment_wei: c.max_endowment_wei.clone().unwrap_or_else(|| (100u128 * 10u128.pow(18)).to_string()),
            max_txs: c.max_txs.unwrap_or(4),
            max_gas_per_tx: c.max_gas_per_tx.unwrap_or(16_777_216),
            max_total_gas: c.max_total_gas.unwrap_or(30_000_000),
            max_blocks: 1,
            max_timestamp_delta: 0,
            max_eval_gas: None,
            max_entry_calls: c.max_entry_calls.unwrap_or(256),
        },
        payout: None,
        rules: None,
        round: 0,
        overrides: Vec::new(),
        groups: vec![],
        protected_accounts: vec![],
        canaries: vec![],
        settlement: Some(serde_json::to_value(&terms)?),
        deploy: Some(Deploy { txs }),
        rule_strength: None,
    };
    m.check_deploy().map_err(|e| eyre!(e))?;
    // Every contract the deployment creates must be there afterwards, targets or not.
    let mut addrs: Vec<Address> = deployed.iter().map(|x| x.address).collect();
    addrs.push(rules_at);
    let codes = code_at(&m, &addrs).map_err(|e| eyre!("{e:#}\n  the deployment, in order:\n    {}", lines.join("\n    ")))?;
    if codes.len() != addrs.len() {
        return Err(eyre!("the deployment's code was read at {} of {} addresses", codes.len(), addrs.len()));
    }
    let empty = keccak256([]);
    for (x, (h, code)) in deployed.iter_mut().zip(&codes) {
        if *h == empty || code.is_empty() {
            return Err(eyre!("the deployment leaves no code at {} ({})", x.name, x.address));
        }
        x.code_hash = *h;
    }
    for t in m.targets.iter_mut() {
        t.code_hash = deployed.iter().find(|x| x.address == t.address).map(|x| x.code_hash).unwrap_or_default();
    }
    let (rh, rcode) = codes.last().cloned().unwrap_or_default();
    if rh == empty || rcode.is_empty() {
        return Err(eyre!("the deployment leaves no code at the rules' address ({rules_at})"));
    }
    m.invariants.runtime_code_hash = keccak256(&rcode);
    m.invariants.runtime_code = rcode;
    check_manifest(&m, a.escrow, a.now)?;
    let rules = Deployed { name: "rules".into(), address: rules_at, tx: n, from: rfrom, target: false, code_hash: rh };
    Ok(Planned { manifest: m, deployed, rules, txs: lines })
}

/// The manifest as near-antibody and the verifier read it: it parses (round rules, groups, the evaluation
/// budget, the deployment's bounds), its strings fit, and its settlement makes parameters within every bound
/// (`Params::check`) for this escrow at `now`.
pub fn check_manifest(m: &Manifest, escrow: u128, now: u64) -> Result<()> {
    let bytes = m.canonical();
    let again = Manifest::parse(&bytes).map_err(|e| eyre!(e))?;
    again.check_deploy().map_err(|e| eyre!(e))?;
    if bytes.len() > 200_000 {
        return Err(eyre!("the manifest is {} bytes: near-antibody takes at most 200,000", bytes.len()));
    }
    if m.program.name.len() > 128 || m.program.description.len() > 2048 {
        return Err(eyre!("the program's name is at most 128 bytes and its description 2048"));
    }
    let s: antibody_rules::settlement::Settlement = serde_json::from_value(m.settlement.clone().ok_or_else(|| eyre!("no settlement"))?)?;
    let slots: Vec<(u8, antibody_rules::Severity)> = m
        .invariants
        .slots
        .iter()
        .map(|x| Ok((x.bit, serde_json::from_value(json!(x.severity))?)))
        .collect::<Result<Vec<_>>>()?;
    let params = s.params(&slots, escrow, 0).map_err(|e| eyre!(e))?;
    antibody_rules::types::bounds::check(&params, escrow, now).map_err(|e| eyre!("settlement: {e}"))?;
    if params.critical != 0 || !params.pause_slots.is_empty() {
        return Err(eyre!("a window pays no Critical and pauses nothing"));
    }
    Ok(())
}

// ------------------------------------------------------------------------------ words

/// Unix seconds as `2026-11-09 14:00 UTC`.
pub fn utc(t: u64) -> String {
    let (days, secs) = ((t / 86_400) as i64, t % 86_400);
    // Days since 1970-01-01 to a civil date (proleptic Gregorian).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe as i64 + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02} UTC", secs / 3_600, (secs % 3_600) / 60)
}

/// Hours in whole units where they are whole: 24 → "24", 1.5 → "1.5".
pub fn fmt_hours(h: f64) -> String {
    if (h - h.round()).abs() < 0.05 {
        format!("{}", h.round() as u64)
    } else {
        format!("{h:.1}")
    }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// The pending status, from the window's opening to its deadline.
pub fn pending_line(deadline: u64, escrow: &str, symbol: &str) -> String {
    fit(format!("Bounty window open until {}, {escrow} {symbol} at stake (testnet)", utc(deadline)))
}

/// The final status of a window with no valid finding.
pub fn green_line(held_hours: f64, escrow: &str, symbol: &str) -> String {
    fit(format!("Held {} h, {escrow} {symbol} at stake, 0 valid findings", fmt_hours(held_hours)))
}

/// The final status of a window with valid findings: their severities and tiers.
pub fn red_line(findings: &[Finding]) -> String {
    let k = findings.len();
    let s = if k == 1 { "" } else { "s" };
    let all: Vec<String> = findings.iter().map(|f| format!("{} {}", capitalized(&f.severity), f.tier)).collect();
    for shown in (0..=all.len()).rev() {
        let mut tiers = all[..shown].join(", ");
        if shown < all.len() {
            tiers += &format!("{}… {} more", if shown > 0 { ", " } else { "" }, all.len() - shown);
        }
        let line = format!("{k} valid finding{s} ({tiers}), merge blocked");
        if line.chars().count() <= MAX_STATUS {
            return line;
        }
    }
    fit(format!("{k} valid finding{s}, merge blocked"))
}

/// After the deadline, while proofs committed before it are still being judged.
pub fn settling_line(deadline: u64, in_flight: usize) -> String {
    let (s, are) = if in_flight == 1 { ("", "is") } else { ("s", "are") };
    fit(format!("Window ended {}; {in_flight} proof{s} committed before it {are} still being judged", utc(deadline)))
}

fn fit(s: String) -> String {
    if s.chars().count() <= MAX_STATUS {
        return s;
    }
    s.chars().take(MAX_STATUS - 1).collect::<String>() + "…"
}

/// The message a window's opening posts to webhooks.
pub fn announce(name: &str, program: u32, deadline: u64, escrow: &str, symbol: &str, link: &str) -> String {
    format!(
        "{name}: program {program} on near-antibody (NEAR testnet) is open until {}, {escrow} {symbol} at stake. A window is a fork-mode \
         program for the pull request's build; {escrow} {symbol} on testnet, which has no value; proofs committed before its deadline are \
         judged and paid. {link}",
        utc(deadline)
    )
}

/// The program's description: what the deployment is, which build, and the window's terms in the same words as
/// everywhere else.
pub fn describe(what: Option<&str>, w: Option<&WindowName>, head: Option<&str>, escrow: &str, symbol: &str, deadline: u64) -> String {
    let mut s = String::new();
    if let Some(x) = what.map(str::trim).filter(|x| !x.is_empty()) {
        s += x;
        s += " ";
    }
    if let Some(w) = w {
        s += &format!("Pull request #{} of {}", w.pr, w.repo);
        if let Some(h) = head.or(w.head.as_deref()) {
            s += &format!(" at {h}");
        }
        s += ". ";
    }
    s += &format!(
        "A window is a fork-mode program for the pull request's build: its contracts exist only inside the verifier. {escrow} {symbol} on \
         testnet, which has no value; proofs committed before its deadline ({}) are judged and paid.",
        utc(deadline)
    );
    if s.len() > 2048 {
        let mut cut = 2047;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

// ------------------------------------------------------------------------------ the verdict

/// A valid finding: a commit whose verdict is a Violation that is paid or owed.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub commit: u64,
    pub state: String,
    /// The highest severity among its keys' slots.
    pub severity: String,
    pub tier: String,
    pub slots: Vec<u64>,
    /// Paid so far, or owed (raw units).
    pub amount_raw: String,
}

/// The states of a valid finding: holding its keys (paid, owed, or with its rule's budget spent), or promoted.
pub fn is_valid_state(state: &str) -> bool {
    matches!(state, "Holder" | "Promoted" | "Done:Paid" | "Done:Exhausted" | "Done:Defaulted")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Created, not open yet (activation).
    Opening,
    /// Open, before its deadline.
    Open,
    /// Past its deadline with no valid finding yet, while proofs committed before it are still being judged.
    Settling,
    /// Past its deadline: no valid finding, nothing in flight.
    Green,
    /// Past its deadline, with at least one valid finding.
    Red,
    /// It never ran: its anchor was rejected, it was found misconfigured, or it was not open by its deadline.
    Void,
}

impl Verdict {
    /// The GitHub commit status state.
    pub fn state(self) -> &'static str {
        match self {
            Verdict::Opening | Verdict::Open | Verdict::Settling => "pending",
            Verdict::Green => "success",
            Verdict::Red => "failure",
            Verdict::Void => "error",
        }
    }
}

/// A window's verdict from its phase, whether it ever opened, NEAR's clock, its valid findings and the commits
/// still in flight.
pub fn classify(phase: &str, opened: bool, now: u64, deadline: u64, valid: usize, in_flight: usize) -> Verdict {
    match phase {
        "Setup" | "Pending" => return if now < deadline { Verdict::Opening } else { Verdict::Void },
        "Open" => {}
        // An exit before the deadline is a program closed early (misconfigured).
        "Exited" if opened && now >= deadline => {}
        _ => return Verdict::Void,
    }
    if now < deadline {
        Verdict::Open
    } else if valid > 0 {
        Verdict::Red
    } else if in_flight > 0 {
        Verdict::Settling
    } else {
        Verdict::Green
    }
}

fn void_line(phase: &str, opened: bool) -> String {
    match (phase, opened) {
        ("Rejected", _) => "The window never opened: its anchor was rejected, and its escrow credited back".into(),
        ("Closed" | "Exited", true) => "The window closed early: its program failed its checks at the anchor; its escrow returns at exit".into(),
        _ => "The window never opened (its activation was not confirmed by its deadline); its escrow returns at exit".into(),
    }
}

/// Hours a window was open: from its opening to its deadline (or to now, before it).
pub fn held_hours(opened: Option<u64>, deadline: u64, now: u64) -> f64 {
    match opened {
        Some(o) => now.min(deadline).saturating_sub(o) as f64 / 3_600.0,
        None => 0.0,
    }
}

fn near_now(ctx: &Ctx) -> u64 {
    ctx.near.final_block_secs().unwrap_or_else(|_| unix_now())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `antibody window status`: the window's deadline, the hours it was held, the escrow at stake, its valid findings
/// with their tier and severity, its verdict, and the commit status line for its pull request's head.
pub fn status(ctx: &Ctx, program: u32, site: &str) -> Result<Value> {
    let p = ctx.program(program)?;
    let meta = cmd::meta(ctx, program)?;
    let name = meta["name"].as_str().unwrap_or_default().to_string();
    let parsed = parse_window_name(&name);
    let (d, sym) = (ctx.dep.decimals(), ctx.dep.symbol());
    let now = near_now(ctx);
    let w = &p["params"]["windows"];
    let deadline = w["deadline"].as_u64().unwrap_or(0);
    let exit_grace = w["exit_grace"].as_u64().unwrap_or(0);
    let phase = p["phase"].as_str().unwrap_or_default().to_string();
    let opened = p["opened_at"].as_u64();
    let deposits = cmd::u128_of(&p["ledger"]["deposits"]);
    let severities: Vec<(u64, String)> = p["params"]["slots"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| (s["slot"].as_u64().unwrap_or(u64::MAX), s["severity"].as_str().unwrap_or("low").to_ascii_lowercase()))
        .collect();
    let rank = |s: &str| ["low", "medium", "high", "critical"].iter().position(|x| *x == s).unwrap_or(0);
    let site = site.trim_end_matches('/');
    let mut findings = Vec::new();
    let mut in_flight = 0usize;
    let mut commits = Vec::new();
    for c in cmd::list_commits(ctx, program)? {
        let st = cmd::state_label(&c["state"]);
        let v = match c["run"].as_u64() {
            Some(r) => cmd::run_view(&cmd::run(ctx, r)?),
            None => Value::Null,
        };
        if st == "Committed" || st == "Verifying" {
            in_flight += 1;
        }
        let id = c["id"].as_u64().unwrap_or_default();
        commits.push(json!({"commit": id, "state": st, "verdict": v["kind"], "tier": v["tier"], "link": format!("{site}/commits/{id}")}));
        if is_valid_state(&st) && v["kind"] == "Violation" {
            let slots: Vec<u64> = v["keys"].as_array().into_iter().flatten().filter_map(|k| k["slot"].as_u64()).collect();
            let sev = slots
                .iter()
                .filter_map(|s| severities.iter().find(|(b, _)| b == s).map(|(_, x)| x.clone()))
                .max_by_key(|x| rank(x))
                .unwrap_or_else(|| "low".into());
            let state = &c["state"][st.as_str()];
            let amount = match cmd::paid_of(&c) {
                0 => cmd::u128_of(&state["amount"]),
                x => x,
            };
            findings.push(Finding {
                commit: id,
                state: st.clone(),
                severity: sev,
                tier: v["tier"].as_str().unwrap_or("T0").to_string(),
                slots,
                amount_raw: amount.to_string(),
            });
        }
    }
    let verdict = classify(&phase, opened.is_some(), now, deadline, findings.len(), in_flight);
    let escrow = cmd::human_tokens(deposits, d);
    let held = held_hours(opened, deadline, now);
    let line = match verdict {
        Verdict::Opening => fit(format!("Bounty window opening (activation), until {}, {escrow} {sym} at stake (testnet)", utc(deadline))),
        Verdict::Open => pending_line(deadline, &escrow, sym),
        Verdict::Settling => settling_line(deadline, in_flight),
        Verdict::Green => green_line(held, &escrow, sym),
        Verdict::Red => red_line(&findings),
        Verdict::Void => void_line(&phase, opened.is_some()),
    };
    let exit_at = if phase == "Closed" { now } else { deadline.saturating_add(exit_grace) };
    let exitable = matches!(phase.as_str(), "Open" | "Setup" | "Closed") && now >= exit_at;
    let link = format!("{site}/programs/{program}");
    Ok(json!({
        "program": program, "name": name, "window": parsed.is_some(), "pull_request": parsed.as_ref().map(|w| w.pr),
        "repository": parsed.as_ref().map(|w| w.repo.clone()), "head": parsed.as_ref().and_then(|w| w.head.clone()),
        "project": p["project"], "phase": phase, "now": now,
        "opened_at": opened, "deadline": deadline, "deadline_utc": utc(deadline), "held_hours": (held * 10.0).round() / 10.0,
        "escrow": escrow, "escrow_raw": deposits.to_string(), "token": sym,
        "paid": cmd::human_tokens(cmd::u128_of(&p["ledger"]["paid"]), d),
        "commits": commits, "in_flight": in_flight,
        "valid_findings": findings, "verdict": verdict,
        "status": {"context": STATUS_CONTEXT, "state": verdict.state(), "description": line, "target_url": link},
        "exit": {"allowed": exitable, "opens_at": exit_at, "opens_at_utc": utc(exit_at), "done": phase == "Exited"},
        "links": {"program": link, "project": ctx.near.explorer_account(p["project"].as_str().unwrap_or_default())},
        "note": "A window is a fork-mode program for the pull request's build; its escrow is wNEAR on testnet, which has no value; proofs committed before its deadline are judged and paid.",
    }))
}

// ------------------------------------------------------------------------------ finding windows

/// Every window on near-antibody whose name parses, optionally only a project's, a repository's or a pull
/// request's: id, name, its parts, phase, deadline and when it was created.
pub fn list(ctx: &Ctx, project: Option<&str>, repo: Option<&str>, pr: Option<u64>) -> Result<Vec<Value>> {
    let n = ctx.view("get_stats", json!({}))?["next_program"].as_u64().unwrap_or(0) as u32;
    let mut out = Vec::new();
    let mut from = 0u32;
    while from < n {
        let page = ctx.view("list_programs", json!({"from": from, "limit": 50}))?;
        for p in page.as_array().into_iter().flatten() {
            if project.is_some_and(|x| p["project"].as_str() != Some(x)) {
                continue;
            }
            let id = p["id"].as_u64().unwrap_or_default() as u32;
            let meta = cmd::meta(ctx, id)?;
            let name = meta["name"].as_str().unwrap_or_default();
            let Some(w) = parse_window_name(name) else { continue };
            if repo.is_some_and(|r| w.repo != r) || pr.is_some_and(|x| w.pr != x) {
                continue;
            }
            let deadline = p["params"]["windows"]["deadline"].as_u64().unwrap_or(0);
            out.push(json!({"program": id, "name": name, "pull_request": w.pr, "repository": w.repo, "head": w.head, "project": p["project"],
                            "phase": p["phase"], "created_at": p["created_at"], "deadline": deadline, "deadline_utc": utc(deadline)}));
        }
        from += 50;
    }
    Ok(out)
}

/// The budget guard: the project opened fewer than `daily_cap` windows in the last 24 hours, its wallet and its
/// credit at near-antibody hold the escrow, and it keeps `min_near` NEAR liquid for gas and storage.
pub fn guard(ctx: &Ctx, project: &str, escrow: u128, daily_cap: u32, min_near: u128, now: u64) -> Result<Value> {
    let d = ctx.dep.decimals();
    let recent = list(ctx, Some(project), None, None)?
        .iter()
        .filter(|w| w["created_at"].as_u64().is_some_and(|t| t.saturating_add(86_400) > now))
        .count() as u32;
    let wallet = cmd::u128_of(&ctx.near.view(&ctx.dep.token, "ft_balance_of", json!({"account_id": project}))?);
    let credit = cmd::u128_of(&cmd::account(ctx, project)?["tokens"]);
    let near = match ctx.near.account(project)? {
        Some(a) => cmd::u128_of(&a["amount"]).saturating_sub(a["storage_usage"].as_u64().unwrap_or(0) as u128 * 10u128.pow(19)),
        None => 0,
    };
    let mut refused = Vec::new();
    if recent >= daily_cap {
        refused.push(format!("{recent} windows opened in the last 24 hours (the cap is {daily_cap})"));
    }
    if wallet + credit < escrow {
        refused.push(format!(
            "the escrow is {} {}, and the project holds {} (wallet {}, credit {})",
            cmd::human_tokens(escrow, d),
            ctx.dep.symbol(),
            cmd::human_tokens(wallet + credit, d),
            cmd::human_tokens(wallet, d),
            cmd::human_tokens(credit, d)
        ));
    }
    if near < min_near {
        refused.push(format!("{} NEAR liquid, under the {} NEAR a window needs for gas and storage", cmd::human_tokens(near, 24), cmd::human_tokens(min_near, 24)));
    }
    Ok(json!({"ok": refused.is_empty(), "refused": refused, "windows_last_24h": recent, "daily_cap": daily_cap,
              "wallet": cmd::human_tokens(wallet, d), "credit": cmd::human_tokens(credit, d), "near_liquid": cmd::human_tokens(near, 24),
              "escrow": cmd::human_tokens(escrow, d), "min_near": cmd::human_tokens(min_near, 24)}))
}

// ------------------------------------------------------------------------------ the anchor

/// The newest Sepolia block NEAR's Ethereum light client holds, and its hash there: the light client the
/// deployment pins for new programs (`get_defaults`), as `create_program` asks it.
pub fn light_client_head(ctx: &Ctx) -> Result<(String, u64, u64, B256)> {
    let defaults = ctx.view("get_defaults", json!({}))?;
    let lc = defaults["light_client"]
        .as_str()
        .map(String::from)
        .or_else(|| ctx.dep.light_client.clone())
        .ok_or_else(|| eyre!("near-antibody pins no light client for new programs"))?;
    let chain_id = defaults["light_client_chain_id"].as_u64().ok_or_else(|| eyre!("get_defaults: no light_client_chain_id"))?;
    let raw = ctx.near.view_raw(&lc, "last_block_number", &[])?;
    let n = u64::from_le_bytes(raw.get(..8).and_then(|b| b.try_into().ok()).ok_or_else(|| eyre!("{lc}.last_block_number: not a u64"))?);
    let h = ctx.near.view_raw(&lc, "block_hash_safe", &n.to_le_bytes())?;
    let hash = match h.first() {
        Some(1) if h.len() >= 33 => B256::from_slice(&h[1..33]),
        _ => return Err(eyre!("{lc} holds no hash for its own newest block {n}")),
    };
    Ok((lc, chain_id, n, hash))
}

// ------------------------------------------------------------------------------ open

pub struct OpenOpts {
    pub config: PathBuf,
    pub root: PathBuf,
    pub artifacts: Option<PathBuf>,
    pub rules_artifacts: Option<PathBuf>,
    pub pr: Option<u64>,
    pub repo: Option<String>,
    /// The pull request's head commit (its short form goes into the name).
    pub head: Option<String>,
    pub name: Option<String>,
    pub hours: Option<f64>,
    pub escrow: Option<String>,
    pub project: Option<String>,
    pub out: PathBuf,
    pub dry_run: bool,
    pub daily_cap: Option<u32>,
    pub max_escrow: Option<String>,
    pub site: String,
    /// Relay the activation's TDX quote if no relayer has within [`RELAY_AFTER`].
    pub relay: bool,
}

/// `antibody window open`: the window's manifest for this build, written to `out`; then (not with `dry_run`) the
/// budget guard, the program created as the project, activated on the light client's anchor, and opened. A window
/// already created for the same name (one per pull request head) is resumed if it is still in setup, reported if it
/// is open, and refused otherwise.
pub fn open(ctx: &Ctx, o: &OpenOpts) -> Result<Value> {
    let cfg = Config::load(&o.config)?;
    let (d, sym) = (ctx.dep.decimals(), ctx.dep.symbol().to_string());
    // The signing account: --account, else the key file's (a dry run may have neither).
    let account = ctx.account.clone().or_else(|| ctx.signer().ok().map(|s| s.account_id));
    let placeholder = o.project.is_none() && account.is_none();
    if placeholder && !o.dry_run {
        return Err(eyre!("no project: pass --account (the bot account that opens the window, with its key)"));
    }
    if let (Some(p), Some(a), false) = (&o.project, &account, o.dry_run) {
        if p != a {
            return Err(eyre!("--project {p} is not the signing account {a}: the project creates its program itself"));
        }
    }
    let project = o.project.clone().or_else(|| account.clone()).unwrap_or_else(|| "project.testnet".into());
    let repo = o.repo.clone().or_else(|| cfg.repository.clone());
    let head = o.head.as_deref().map(short_head).transpose()?;
    let name = match (&o.name, o.pr, &repo) {
        (Some(n), _, _) => n.clone(),
        (None, Some(pr), Some(r)) => {
            if !repo_ok(r) {
                return Err(eyre!("{r:?} is not a repository name"));
            }
            window_name(pr, r, head.as_deref())
        }
        _ => return Err(eyre!("give --pr and --repo (or the config's `repository`), or --name")),
    };
    if name.len() > 128 {
        return Err(eyre!("the program's name is at most 128 bytes: {name:?}"));
    }
    let parsed = parse_window_name(&name);
    let hours = o.hours.or(cfg.hours).unwrap_or(DEFAULT_HOURS);
    if !(MIN_HOURS..=365.0 * 24.0 - 1.0).contains(&hours) {
        return Err(eyre!("a window lasts {MIN_HOURS} hours to a year, not {hours}"));
    }
    let escrow_s = o.escrow.clone().or_else(|| cfg.escrow.clone()).unwrap_or_else(|| DEFAULT_ESCROW.into());
    let escrow = cmd::parse_tokens(&escrow_s, d)?;
    if let Some(max) = &o.max_escrow {
        let max = cmd::parse_tokens(max, d)?;
        if escrow > max {
            return Err(eyre!("the escrow {escrow_s} {sym} is above --max-escrow {}", cmd::human_tokens(max, d)));
        }
    }
    let site = o.site.trim_end_matches('/').to_string();
    let mut out = json!({"dry_run": o.dry_run, "name": name, "window": parsed.is_some(), "project": project,
                         "note": "A window is a fork-mode program for the pull request's build; its escrow is wNEAR on testnet, which has no value; proofs committed before its deadline are judged and paid."});
    if placeholder {
        out["project_note"] = json!("no --account: the manifest names project.testnet, so its hash differs from a real window's");
    }

    // One window per pull request head: the same name again resumes it (setup), reports it (open), or is refused.
    if !o.dry_run {
        let mine = list(ctx, Some(&project), None, None)?;
        if let Some(w) = mine.iter().rev().find(|w| w["name"] == json!(name)) {
            let pid = w["program"].as_u64().unwrap_or_default() as u32;
            out["program"] = json!(pid);
            match w["phase"].as_str().unwrap_or_default() {
                "Open" => {
                    out["existing"] = json!(true);
                    return finish(ctx, pid, &site, out);
                }
                "Setup" | "Pending" => {
                    out["resumed"] = json!(true);
                    out["activation"] = activate_and_open(ctx, pid, o.relay)?;
                    return finish(ctx, pid, &site, out);
                }
                phase => return Err(eyre!("a window for this head already ran: program {pid} ({name}), phase {phase}. One window per pull request head")),
            }
        }
    }

    let now = near_now(ctx);
    if let Some(account) = account.clone().filter(|_| !placeholder) {
        let cap = o.daily_cap.or(cfg.daily_cap).unwrap_or(DEFAULT_DAILY_CAP);
        let min_near = cmd::parse_near(cfg.min_near.as_deref().unwrap_or(DEFAULT_MIN_NEAR))?;
        let g = guard(ctx, &account, escrow, cap, min_near, now)?;
        if !o.dry_run && g["ok"] != json!(true) {
            return Err(eyre!("the budget guard refuses a window: {}", g["refused"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")));
        }
        out["guard"] = g;
    }

    // The anchor: the newest block the light client holds, so create_program's check confirms it at once.
    let (lc, chain_id, block, hash) = light_client_head(ctx)?;
    let pool = ctx.eth_pool()?;
    let header = antibody_witness::build::fetch_header(&pool, block)?;
    if header.hash_slow() != hash {
        return Err(eyre!("the Ethereum RPC's block {block} hashes to {:#x}; {lc} holds {hash:#x}", header.hash_slow()));
    }
    let anchor = Anchor { chain_id, block_number: block, block_hash: hash, evm_spec: "osaka".into(), trust: "light-client".into() };
    let deadline = now + (hours * 3_600.0).round() as u64;
    let escrow_h = cmd::human_tokens(escrow, d);
    let artifacts = o.artifacts.clone().unwrap_or_else(|| cfg.artifacts.clone());
    let args = PlanArgs {
        name: name.clone(),
        project: project.clone(),
        description: describe(cfg.description.as_deref(), parsed.as_ref(), o.head.as_deref(), &escrow_h, &sym, deadline),
        deadline,
        escrow,
        decimals: d,
        root: o.root.clone(),
        rules_artifacts: o.rules_artifacts.clone().unwrap_or_else(|| artifacts.clone()),
        artifacts,
        now,
    };
    let planned = plan(&cfg, &args, anchor, |m, addrs| cmd::deployed_code(ctx, m, addrs))?;
    let bytes = planned.manifest.canonical();
    if let Some(dir) = o.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&o.out, &bytes)?;
    out["manifest"] = json!({"path": o.out, "bytes": bytes.len(), "sha256": hex::encode(sha256(&bytes)), "deploy_txs": planned.txs});
    out["anchor"] = json!({"chain_id": chain_id, "block": block, "hash": format!("{hash:#x}"), "light_client": lc, "trust": "light-client"});
    out["deadline"] = json!({"unix": deadline, "utc": utc(deadline), "hours": hours});
    out["escrow"] = json!({"amount": escrow_h, "raw": escrow.to_string(), "token": sym});
    out["targets"] = json!(planned.deployed.iter().filter(|x| x.target).collect::<Vec<_>>());
    out["rules"] = json!({"address": planned.rules.address, "runtime_code_hash": planned.manifest.invariants.runtime_code_hash, "from": planned.rules.from,
                          "slots": planned.manifest.invariants.slots});
    out["status"] = json!({"context": STATUS_CONTEXT, "state": "pending", "description": pending_line(deadline, &escrow_h, &sym)});
    if o.dry_run {
        out["next"] = json!("not sent (--dry-run): without it, the program is created as the project, activated and opened");
        return Ok(out);
    }

    // The escrow leaves the project's wallet (ft_transfer_call): what earlier windows returned to its token credit
    // at near-antibody is withdrawn first.
    let wallet = cmd::u128_of(&ctx.near.view(&ctx.dep.token, "ft_balance_of", json!({"account_id": project}))?);
    if wallet < escrow {
        let credit = cmd::u128_of(&cmd::account(ctx, &project)?["tokens"]);
        if credit > 0 {
            let s = cmd::withdraw(ctx, &cmd::fmt_tokens(credit, d))?;
            out["withdrew_credit"] = json!({"amount": cmd::human_tokens(credit, d), "tx": s["explorer"]});
        }
    }
    // Create (the escrow with ft_transfer_call and one yoctoNEAR: the project's full-access key), activate, open.
    let key = ctx.state_dir.join("window").join(format!("{project}.x25519.json"));
    cmd::keygen_x25519_nep413(ctx, &key)?;
    let created = cmd::program_create(ctx, &o.out, &cmd::fmt_tokens(escrow, d), &key, None)?;
    let pid = created["program"]["program_id"].as_u64().ok_or_else(|| eyre!("no program id: {created}"))? as u32;
    out["program"] = json!(pid);
    out["create"] = json!({"tx": created["tx"]["explorer"], "anchor": created["anchor"], "manifest_upload": created["manifest_upload"]["location"]});
    if cmd::event(&created["tx"], "program_rejected").is_some() {
        return Err(eyre!("program {pid}: near-antibody rejected the anchor (the light client contradicts block {block}); the escrow is credited back"));
    }
    if created["anchor"]["anchor"] != json!("LightClient") {
        out["anchor_note"] = json!(format!("the light client did not confirm block {block} at creation: the anchor is {}", created["anchor"]["anchor"]));
    }
    out["activation"] = activate_and_open(ctx, pid, o.relay)?;
    finish(ctx, pid, &site, out)
}

/// The opened window's facts from the chain: its deadline, escrow, status line, announcement and links.
fn finish(ctx: &Ctx, pid: u32, site: &str, mut out: Value) -> Result<Value> {
    let p = ctx.program(pid)?;
    let (d, sym) = (ctx.dep.decimals(), ctx.dep.symbol());
    let deadline = p["params"]["windows"]["deadline"].as_u64().unwrap_or(0);
    let escrow = cmd::human_tokens(cmd::u128_of(&p["ledger"]["deposits"]), d);
    let link = format!("{site}/programs/{pid}");
    let name = out["name"].as_str().unwrap_or_default().to_string();
    out["phase"] = p["phase"].clone();
    out["deadline"] = json!({"unix": deadline, "utc": utc(deadline)});
    out["escrow"] = json!({"amount": escrow, "raw": cmd::u128_of(&p["ledger"]["deposits"]).to_string(), "token": sym});
    out["status"] = json!({"context": STATUS_CONTEXT, "state": "pending", "description": pending_line(deadline, &escrow, sym), "target_url": link});
    out["announce"] = json!(announce(&name, pid, deadline, &escrow, sym, &link));
    out["links"] = json!({"program": link, "project": ctx.near.explorer_account(p["project"].as_str().unwrap_or_default()),
                          "near_antibody": ctx.near.explorer_account(&ctx.dep.antibody)});
    Ok(out)
}

/// Activation, its T1 confirmation and the opening, resumable: an activation run in flight is waited for, one
/// that ended without T1 is requested again, and the project relays the run's quote itself when no relayer has.
pub fn activate_and_open(ctx: &Ctx, pid: u32, relay: bool) -> Result<Value> {
    let p = ctx.program(pid)?;
    let mut out = json!({});
    if p["phase"] == json!("Open") {
        return Ok(json!({"already_open": true}));
    }
    if p["sealing_pk"].is_null() {
        let now = near_now(ctx);
        let timeout = p["params"]["windows"]["run_timeout"].as_u64().unwrap_or(240);
        let in_flight = p["activation_run"].as_u64().and_then(|r| cmd::run(ctx, r).ok()).filter(|r| {
            r["outcome"] == json!("Pending") && r["started_at"].as_u64().is_some_and(|s| now < s.saturating_add(timeout))
        });
        let (run_id, tx) = match in_flight {
            Some(r) => (r["id"].as_u64().unwrap_or_default(), None),
            None => {
                let act = cmd::program_activate(ctx, pid, &[], true)?;
                out["base"] = act["base"].clone();
                out["tx"] = act["tx"]["explorer"].clone();
                (act["run_id"].as_u64().unwrap_or_default(), act["tx"]["tx"].as_str().map(String::from))
            }
        };
        out["run_id"] = json!(run_id);
        out["t1"] = wait_t1(ctx, pid, run_id, tx.as_deref(), relay)?;
    }
    let opened = cmd::program_open(ctx, pid)?;
    out["open_tx"] = opened["explorer"].clone();
    Ok(out)
}

/// Waits until the activation is T1-confirmed (the program has its sealing key). After [`RELAY_AFTER`] without a
/// relayer, the project relays the run's quote once (`confirm_tee`, about 0.018 NEAR of DCAP verification).
fn wait_t1(ctx: &Ctx, pid: u32, run_id: u64, tx: Option<&str>, relay: bool) -> Result<Value> {
    let t0 = Instant::now();
    let mut answered: Option<Instant> = None;
    let mut relayed = Value::Null;
    loop {
        let p = ctx.program(pid)?;
        if !p["sealing_pk"].is_null() {
            return Ok(json!({"confirmed": true, "seconds": t0.elapsed().as_secs(), "relayed_here": relayed}));
        }
        if p["phase"] == json!("Closed") {
            return Err(eyre!("program {pid} failed its checks at the anchor (ProgramMisconfigured): it is closed, and `window close` returns its escrow"));
        }
        let r = cmd::run_view(&cmd::run(ctx, run_id)?);
        if r.get("failed").is_some() || r["kind"] == json!("InfraError") {
            return Err(eyre!("activation run {run_id} failed ({}): `window open` again requests a new one", r));
        }
        if r["kind"] == json!("ProgramMisconfigured") {
            return Err(eyre!("activation run {run_id}: the program fails its checks at the anchor ({}): fix the window config", r["reason"]));
        }
        if r.get("pending").is_none() && answered.is_none() {
            answered = Some(Instant::now());
        }
        if let (Some(at), Some(tx), true, true) = (answered, tx, relay, relayed.is_null()) {
            if at.elapsed() >= RELAY_AFTER {
                let signer = ctx.signer()?.account_id;
                relayed = match tee::request_id_from_tx(ctx, tx, &signer).and_then(|id| tee::confirm_or_wait(ctx, tee::DEFAULT_API, run_id, id, "auto")) {
                    Ok(tee::Relay::Sent(v)) => v,
                    Ok(tee::Relay::Wait { why, .. }) => {
                        return Err(eyre!("activation run {run_id} cannot be T1-confirmed now: {why}; `window open` again requests a new run"));
                    }
                    Err(e) => json!({"error": format!("{e:#}")}),
                };
            }
        }
        if t0.elapsed() > T1_WAIT {
            return Err(eyre!(
                "activation run {run_id} is not T1-confirmed after {} s ({}); `window open` again resumes the window",
                T1_WAIT.as_secs(),
                if relayed.is_null() { "no relay was sent".to_string() } else { format!("relayed: {relayed}") }
            ));
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

// ------------------------------------------------------------------------------ close

/// The permissionless steps a window's exit waits for, for this program only: unrevealed commits past their
/// reveal window expire, stuck runs time out, final verdicts are allocated in commit order, funded claims are paid.
fn settle(ctx: &Ctx, program: u32) -> Result<Vec<Value>> {
    let mut steps = Vec::new();
    let p = ctx.program(program)?;
    let w = &p["params"]["windows"];
    let now = near_now(ctx);
    let secs = |v: &Value| v.as_u64().unwrap_or(u64::MAX);
    for c in cmd::list_commits(ctx, program)? {
        let due = match cmd::state_label(&c["state"]).as_str() {
            "Committed" if now > secs(&c["committed_at"]).saturating_add(secs(&w["reveal"])) => Some("expire"),
            "Verifying" => {
                let r = c["run"].as_u64().and_then(|r| cmd::run(ctx, r).ok()).unwrap_or(Value::Null);
                let stuck = match &r["outcome"] {
                    Value::String(s) if s == "Pending" => now > secs(&r["started_at"]).saturating_add(secs(&w["run_timeout"])),
                    o if o.get("Failed").is_some() => now > secs(&c["retry_until"]),
                    o => o["Answered"]["facts"]["kind"] == "InfraError" && now > secs(&c["retry_until"]),
                };
                stuck.then_some("time_out")
            }
            _ => None,
        };
        if let Some(method) = due {
            match cmd::call(ctx, method, json!({"commit_id": c["id"]}), 60) {
                Ok(s) => steps.push(json!({method: c["id"], "tx": s["explorer"]})),
                Err(e) => steps.push(json!({"not_closed": c["id"], "why": format!("{e:#}")})),
            }
        }
    }
    let p = ctx.program(program)?;
    if p["cursor"].as_u64().unwrap_or(0) < p["commits"].as_u64().unwrap_or(0) {
        loop {
            match cmd::call(ctx, "allocate", json!({"program_id": program, "max": 20}), 150) {
                Ok(s) => steps.push(json!({"allocate": s["explorer"]})),
                Err(e) => {
                    let e = format!("{e:#}");
                    if !e.contains("nothing to allocate") {
                        steps.push(json!({"allocate_stopped": e}));
                    }
                    break;
                }
            }
        }
    }
    for c in cmd::list_commits(ctx, program)? {
        let st = cmd::state_label(&c["state"]);
        let claim = &c["state"][st.as_str()];
        let funded = cmd::u128_of(&claim["debt"]) == 0 && cmd::u128_of(&claim["reserved"]) >= cmd::u128_of(&claim["amount"]);
        if (st == "Holder" || st == "Promoted") && funded {
            match cmd::call(ctx, "pay", json!({"commit_id": c["id"]}), 60) {
                Ok(s) => steps.push(json!({"paid": c["id"], "tx": s["explorer"]})),
                Err(e) => steps.push(json!({"not_paid": c["id"], "why": format!("{e:#}")})),
            }
        }
    }
    if ctx.program(program)?["debtors"].as_array().is_some_and(|d| !d.is_empty()) {
        match cmd::call(ctx, "settle_debts", json!({"program_id": program}), 100) {
            Ok(s) => steps.push(json!({"settle_debts": s["explorer"]})),
            Err(e) => steps.push(json!({"settle_debts_stopped": format!("{e:#}")})),
        }
    }
    Ok(steps)
}

/// `antibody window close`: once the deadline and the exit grace have passed (at once for a program closed as
/// misconfigured), the steps the exit waits for, the exit (permissionless: the escrow's rest goes to the project's
/// token credit), the commit records closed a page at a time, and the project's token credit withdrawn to its
/// wallet. Each step is skipped when done, so it can run again.
pub fn close(ctx: &Ctx, program: u32) -> Result<Value> {
    let signer = ctx.signer()?;
    let p = ctx.program(program)?;
    let project = p["project"].as_str().unwrap_or_default().to_string();
    let phase = p["phase"].as_str().unwrap_or_default().to_string();
    let d = ctx.dep.decimals();
    let now = near_now(ctx);
    let w = &p["params"]["windows"];
    let due = w["deadline"].as_u64().unwrap_or(u64::MAX).saturating_add(w["exit_grace"].as_u64().unwrap_or(0));
    let mut out = json!({"program": program, "project": project, "phase_before": phase});
    match phase.as_str() {
        "Exited" => {
            out["exit"] = json!("done earlier");
            if p["close_cursor"].as_u64() < p["commits"].as_u64() {
                out["close_pages"] = json!(close_pages(ctx, program)?);
            }
        }
        "Rejected" => out["exit"] = json!("none: the anchor was rejected at creation and the escrow refunded then"),
        "Pending" => {
            out["exit"] = json!("not yet: the anchor check is pending");
            return Ok(out);
        }
        _ if phase != "Closed" && now < due => {
            out["exit"] = json!({"not_yet": true, "opens_at": due, "opens_at_utc": utc(due)});
            return Ok(out);
        }
        _ => {
            out["settle"] = json!(settle(ctx, program)?);
            let q = ctx.program(program)?;
            let (cursor, commits) = (q["cursor"].as_u64().unwrap_or(0), q["commits"].as_u64().unwrap_or(0));
            if phase != "Closed" && cursor < commits {
                out["exit"] = json!({"waiting": format!("{} of {commits} commits are not allocated or closed yet; `window close` again exits it", commits - cursor)});
                return Ok(out);
            }
            let exit = cmd::call(ctx, "exit", json!({"program_id": program}), 150)?;
            out["exit"] = json!({"tx": exit["explorer"], "event": cmd::event(&exit, "program_exited")});
            out["close_pages"] = json!(close_pages(ctx, program)?);
        }
    }
    if signer.account_id == project {
        let credit = cmd::u128_of(&cmd::account(ctx, &project)?["tokens"]);
        if credit > 0 {
            let s = cmd::withdraw(ctx, &cmd::fmt_tokens(credit, d))?;
            out["withdraw"] = json!({"amount": cmd::human_tokens(credit, d), "token": ctx.dep.symbol(), "tx": s["explorer"]});
        } else {
            out["withdraw"] = json!({"amount": "0", "note": "no token credit to withdraw"});
        }
    } else {
        out["withdraw"] = json!(format!("skipped: only the project ({project}) withdraws its token credit"));
    }
    out["phase"] = ctx.program(program)?["phase"].clone();
    Ok(out)
}

fn close_pages(ctx: &Ctx, program: u32) -> Result<Vec<Value>> {
    let mut pages = Vec::new();
    loop {
        match cmd::call(ctx, "close_commits", json!({"program_id": program, "max": 100}), 300) {
            Ok(s) => pages.push(s["explorer"].clone()),
            Err(e) if e.to_string().contains("nothing to close") => return Ok(pages),
            Err(e) => return Err(e),
        }
    }
}

/// Unit tests with no EVM: the deployment's code is a stub here. `tests/window_fixture.rs` runs the fixture's
/// deployment, the verifier's activation checks and a proof with the verifier's own code.
#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/window")
    }

    fn s(v: &str) -> toml::Value {
        toml::Value::String(v.into())
    }

    const NOW: u64 = 1_800_000_000;

    fn anchor() -> Anchor {
        Anchor { chain_id: 31337, block_number: 1, block_hash: B256::repeat_byte(1), evm_spec: "osaka".into(), trust: "project-attested".into() }
    }

    /// Instead of the deployment: one byte of code at every address, distinct per address.
    fn stub_code(_: &Manifest, addrs: &[Address]) -> Result<Vec<(B256, Bytes)>> {
        Ok(addrs.iter().map(|a| Bytes::from(vec![0x60, a[19], 0x00])).map(|c| (keccak256(&c), c)).collect())
    }

    fn fixture_config() -> Config {
        Config::load(&fixture().join("window.toml")).unwrap()
    }

    fn args(cfg: &Config) -> PlanArgs {
        PlanArgs {
            name: window_name(7, "window-fixture", Some("1a2b3c4")),
            project: "window-bot.testnet".into(),
            description: describe(cfg.description.as_deref(), parse_window_name("PR #7 window (window-fixture @ 1a2b3c4)").as_ref(), None, "0.01", "wNEAR", NOW + 7_200),
            deadline: NOW + 7_200,
            escrow: cmd::parse_tokens(cfg.escrow.as_deref().unwrap_or("0.01"), 24).unwrap(),
            decimals: 24,
            root: fixture(),
            artifacts: cfg.artifacts.clone(),
            rules_artifacts: cfg.artifacts.clone(),
            now: NOW,
        }
    }

    #[test]
    fn window_names_are_written_and_read_back() {
        let n = window_name(12, "antibody-immune-demo", Some("1a2b3c4"));
        assert_eq!(n, "PR #12 window (antibody-immune-demo @ 1a2b3c4)");
        assert_eq!(parse_window_name(&n), Some(WindowName { pr: 12, repo: "antibody-immune-demo".into(), head: Some("1a2b3c4".into()) }));
        assert!(is_window("PR #12 window (antibody-immune-demo)"));
        assert!(is_window("PR #3 window (Himess/antibody-immune-demo @ 0123456789abcdef0123456789abcdef01234567)"));
        for other in [
            "Acme Lend drill 2026-11-10",
            "Practice program in fork mode — a tutorial (testnet)",
            "PR #12 window",
            "PR #0 window (demo)",
            "PR #012 window (demo)",
            "PR #12 window (demo @ 1A2B3C4)",
            "PR #12 window (demo @ 1a2b3c)",
            "PR #12 window (demo @ 1a2b3c4) extra",
            "PR #12 window (de mo)",
            "PR #12 window ()",
            "pr #12 window (demo)",
        ] {
            assert!(!is_window(other), "{other}");
        }
        assert_eq!(short_head("1A2B3C4D5E6F7a8b9c0d1e2f3a4b5c6d7e8f9a0b").unwrap(), "1a2b3c4");
        assert!(short_head("1a2b3c").is_err());
        assert!(short_head("not-hex-at-all").is_err());
    }

    #[test]
    fn arguments_encode_as_solidity_encodes_them() {
        let named = |s: &str| if s == "coin" { Ok(Address::repeat_byte(0xcc)) } else { Err(format!("{s}?")) };
        // abi.encode("Window Coin", coin, 7): the string's offset, the address, the number, then the string's tail.
        let out = encode_abi(&["string".into(), "address".into(), "uint8".into()], &[s("Window Coin"), s("coin"), toml::Value::Integer(7)], &named).unwrap();
        assert_eq!(out.len(), 32 * 5);
        assert_eq!(U256::from_be_slice(&out[..32]), U256::from(96u64), "the tail starts after three head words");
        assert_eq!(&out[44..64], Address::repeat_byte(0xcc).as_slice());
        assert_eq!(out[95], 7);
        assert_eq!(U256::from_be_slice(&out[96..128]), U256::from(11u64));
        assert_eq!(&out[128..139], b"Window Coin");
        assert!(out[139..].iter().all(|b| *b == 0), "padded to a word");
        // Two dynamic arguments: each offset counts the tails before it.
        let two = encode_abi(&["bytes".into(), "bytes".into()], &[s("0x1234"), s("0x")], &named).unwrap();
        assert_eq!(U256::from_be_slice(&two[..32]), U256::from(64u64));
        assert_eq!(U256::from_be_slice(&two[32..64]), U256::from(128u64));
        assert_eq!(&two[96..98], &[0x12, 0x34]);
        assert_eq!(two.len(), 32 * 5);
        assert!(encode_abi(&["address".into()], &[s("vault")], &named).unwrap_err().to_string().contains("vault?"));
        assert!(encode_abi(&["uint256[]".into()], &[s("1")], &named).unwrap_err().to_string().contains("arrays and tuples are not encoded here"));
        assert!(encode_abi(&["function".into()], &[s("1")], &named).unwrap_err().to_string().contains("not a static type"));
        assert!(encode_abi(&["uint256".into()], &[], &named).is_err());
        assert_eq!(parse_signature("mint(address, uint256)").unwrap(), ("mint(address,uint256)".to_string(), vec!["address".to_string(), "uint256".to_string()]));
        assert_eq!(parse_signature("approveLend()").unwrap().1.len(), 0);
        assert!(parse_signature("f((uint8,address))").is_err());
        assert!(parse_signature("f(uint256[])").is_err());
        assert!(parse_signature("mint").is_err());
    }

    #[test]
    fn the_fixture_config_becomes_its_manifest() {
        let cfg = fixture_config();
        let a = args(&cfg);
        let p = plan(&cfg, &a, anchor(), stub_code).unwrap();
        let m = &p.manifest;
        let d = deployer();
        // The deployment, in order: coin, vault, note (not a target), the mint, then the rules.
        let deploy = m.deploy.as_ref().unwrap();
        assert_eq!(deploy.txs.len(), 5);
        let (coin, vault, note) = (d.create(0), d.create(1), d.create(2));
        assert_eq!(p.rules.address, d.create(4));
        assert_eq!(p.deployed.iter().map(|x| (x.name.as_str(), x.address, x.target)).collect::<Vec<_>>(), vec![("coin", coin, true), ("vault", vault, true), ("note", note, false)]);
        assert_eq!(m.targets.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["coin", "vault"]);
        let coin_art = immune::read_artifact(&fixture().join("artifacts/Window.sol/Coin.json")).unwrap();
        let mut want = coin_art.creation.clone();
        want.extend(encode_abi(&coin_art.constructor, &[s("Window Coin"), s("deployer")], &|_| Ok(d)).unwrap());
        assert_eq!(deploy.txs[0].data.as_ref(), want.as_slice(), "the creation code with its constructor arguments");
        assert_eq!(deploy.txs[3].to, Some(coin));
        assert_eq!(&deploy.txs[3].data[..4], &keccak256(b"mint(address,uint256)")[..4]);
        assert_eq!(&deploy.txs[3].data[16..36], vault.as_slice(), "a contract's name stands for its address");
        // The rules are created last; their runtime code and the targets' code hashes are what the deployment leaves.
        let rules_art = immune::read_artifact(&fixture().join("artifacts/Window.sol/VaultRules.json")).unwrap();
        assert_eq!(&deploy.txs[4].data[..rules_art.creation.len()], rules_art.creation.as_slice());
        assert_eq!(&deploy.txs[4].data[rules_art.creation.len() + 12..rules_art.creation.len() + 32], vault.as_slice());
        let stub = stub_code(m, &[p.rules.address, coin]).unwrap();
        assert_eq!(m.invariants.runtime_code, stub[0].1);
        assert_eq!(m.invariants.runtime_code_hash, keccak256(&stub[0].1));
        assert_eq!(m.targets[0].code_hash, stub[1].0);
        assert_eq!(m.invariants.slots, vec![Slot { bit: 0, name: "vault_keeps_its_coins".into(), severity: Severity::High }]);
        assert_eq!(m.impact.protocol_accounts, vec![vault]);
        assert_eq!(m.impact.tokens[0].address, coin);
        // Its terms: no Critical, nothing to pause, the deadline, the short challenge window from the config.
        let terms: antibody_rules::settlement::Settlement = serde_json::from_value(m.settlement.clone().unwrap()).unwrap();
        assert_eq!((terms.payouts.critical.as_str(), terms.payouts.high.as_str()), ("0", "5000000000000000000000"));
        assert!(terms.pause.slots.is_empty() && terms.gap.is_none() && terms.known.max_count == 0);
        assert_eq!((terms.windows.deadline, terms.windows.challenge), (NOW + 7_200, 120));
        assert_eq!(terms.instant.cap, "5000000000000000000000", "a T1 finding up to the largest payout is paid in seconds");
        assert_eq!(terms.terms_sha256, hex::encode(sha256(WINDOW_TERMS.as_bytes())));
        assert_eq!(m.program.name, "PR #7 window (window-fixture @ 1a2b3c4)");
        assert!(m.program.description.contains("A window is a fork-mode program for the pull request's build"));
        assert!(m.program.description.contains("0.01 wNEAR on testnet, which has no value; proofs committed before its deadline"));
        assert_eq!(Manifest::parse(&m.canonical()).unwrap(), *m);
        check_manifest(m, a.escrow, NOW).unwrap();
    }

    #[test]
    fn the_rules_come_from_their_own_artifacts() {
        // The workflow builds the rules from the base branch, so a pull request cannot weaken the rules its window
        // is judged by. A directory with no rules artifact is refused, not silently replaced.
        let cfg = fixture_config();
        let mut a = args(&cfg);
        a.rules_artifacts = PathBuf::from("src");
        assert!(plan(&cfg, &a, anchor(), stub_code).unwrap_err().to_string().contains("no artifact for VaultRules"));
    }

    #[test]
    fn a_window_refuses_what_it_cannot_be() {
        let base = std::fs::read_to_string(fixture().join("window.toml")).unwrap();
        let err = |text: &str| -> String {
            match Config::parse(text) {
                Err(e) => e.to_string(),
                Ok(cfg) => plan(&cfg, &args(&cfg), anchor(), stub_code).err().map(|e| format!("{e:#}")).unwrap_or_default(),
            }
        };
        assert!(err(&base.replace("severity = \"high\"", "severity = \"critical\"")).contains("nothing to pause"));
        assert!(err(&base.replace("escrow = \"0.01\"", "escrow = \"0.001\"")).contains("below the largest payout"));
        assert!(err(&base.replace("high = \"0.005\"", "high = \"0\"")).contains("pays nothing for"));
        assert!(err(&base.replace("args = [\"coin\", \"deployer\"]", "args = [\"nobody\", \"deployer\"]")).contains("nobody is not `deployer`"));
        assert!(err(&base.replace("args = [\"vault\", \"1000000000000000000000\"]", "args = [\"rules\", \"1\"]")).contains("rules is not `deployer`"));
        assert!(err(&base.replace("name = \"vault\"\n", "name = \"coin\"\n")).contains("taken"));
        assert!(err(&base.replace("function = \"mint(address, uint256)\"", "function = \"mint(address)\"")).contains("2 arguments for 1 parameters"));
        assert!(err(&base.replace("contract = \"Vault\"", "contract = \"Vault\"\nfunction = \"x()\"")).contains("belong to a call"));
        assert!(err(&base.replace("[windows]", "[windows]\nsurprise = 1")).contains("unknown field"));
        assert!(err(&base.replace("[[tx]]\ncall = \"coin\"", "[[tx]]\ncall = \"coin\"\nname = \"x\"")).contains("a call takes"));
        // A deployment that leaves no code where a contract was created is refused.
        let cfg = fixture_config();
        let none = |_: &Manifest, addrs: &[Address]| Ok(addrs.iter().map(|_| (keccak256([]), Bytes::new())).collect());
        assert!(plan(&cfg, &args(&cfg), anchor(), none).unwrap_err().to_string().contains("leaves no code at coin"));
        // The settlement stays within near-antibody's bounds: a deadline within a year.
        let mut a = args(&cfg);
        a.deadline = NOW + 366 * 86_400;
        assert!(plan(&cfg, &a, anchor(), stub_code).unwrap_err().to_string().contains("within a year"));
    }

    #[test]
    fn verdicts_follow_the_deadline_and_the_findings() {
        let (dl, before, after) = (NOW, NOW - 1, NOW + 1);
        assert_eq!(classify("Setup", false, before, dl, 0, 0), Verdict::Opening);
        assert_eq!(classify("Setup", false, after, dl, 0, 0), Verdict::Void);
        assert_eq!(classify("Open", true, before, dl, 2, 1), Verdict::Open, "open until the deadline, whatever was found");
        assert_eq!(classify("Open", true, after, dl, 0, 0), Verdict::Green);
        assert_eq!(classify("Open", true, after, dl, 0, 2), Verdict::Settling, "proofs committed before the deadline are still judged");
        assert_eq!(classify("Open", true, after, dl, 1, 2), Verdict::Red);
        assert_eq!(classify("Exited", true, after, dl, 0, 0), Verdict::Green);
        assert_eq!(classify("Exited", true, before, dl, 0, 0), Verdict::Void, "an exit before the deadline: closed early");
        assert_eq!(classify("Exited", false, after, dl, 0, 0), Verdict::Void, "never opened");
        assert_eq!(classify("Rejected", false, after, dl, 0, 0), Verdict::Void);
        assert_eq!(classify("Closed", true, before, dl, 0, 0), Verdict::Void);
        assert_eq!([Verdict::Open, Verdict::Settling, Verdict::Green, Verdict::Red, Verdict::Void].map(Verdict::state), ["pending", "pending", "success", "failure", "error"]);
        for st in ["Holder", "Promoted", "Done:Paid", "Done:Exhausted", "Done:Defaulted"] {
            assert!(is_valid_state(st), "{st}");
        }
        for st in ["Committed", "Verifying", "Duplicate", "GapClaim", "Done:Forfeited", "Done:Refunded", "Done:DuplicateConfirmed", "Done:Known"] {
            assert!(!is_valid_state(st), "{st}");
        }
        assert_eq!(held_hours(Some(NOW - 86_400), NOW, NOW + 999), 24.0);
        assert_eq!(held_hours(Some(NOW - 1_800), NOW + 7_200, NOW), 0.5, "before the deadline: held so far");
        assert_eq!(held_hours(None, NOW, NOW + 1), 0.0);
    }

    #[test]
    fn the_status_lines_say_what_the_brief_says() {
        assert_eq!(utc(1_793_806_931), "2026-11-04 15:42 UTC");
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(pending_line(1_793_806_931, "0.03", "wNEAR"), "Bounty window open until 2026-11-04 15:42 UTC, 0.03 wNEAR at stake (testnet)");
        assert_eq!(green_line(24.0, "0.03", "wNEAR"), "Held 24 h, 0.03 wNEAR at stake, 0 valid findings");
        assert_eq!(green_line(1.5, "0.03", "wNEAR"), "Held 1.5 h, 0.03 wNEAR at stake, 0 valid findings");
        let f = |sev: &str, tier: &str| Finding { commit: 1, state: "Done:Paid".into(), severity: sev.into(), tier: tier.into(), slots: vec![0], amount_raw: "1".into() };
        assert_eq!(red_line(&[f("high", "T1")]), "1 valid finding (High T1), merge blocked");
        assert_eq!(red_line(&[f("high", "T1"), f("medium", "T0")]), "2 valid findings (High T1, Medium T0), merge blocked");
        let many: Vec<Finding> = (0..30).map(|_| f("medium", "T1")).collect();
        let line = red_line(&many);
        assert!(line.chars().count() <= MAX_STATUS && line.starts_with("30 valid findings (Medium T1,") && line.contains("more), merge blocked"), "{line}");
        assert_eq!(settling_line(1_793_806_931, 1), "Window ended 2026-11-04 15:42 UTC; 1 proof committed before it is still being judged");
        assert!(void_line("Setup", false).contains("never opened"));
        let a = announce("PR #12 window (demo @ 1a2b3c4)", 15, 1_793_806_931, "0.03", "wNEAR", "https://site/programs/15");
        assert!(a.contains("program 15") && a.contains("wNEAR on testnet, which has no value; proofs committed before its deadline are judged and paid"));
        assert_eq!(fit("x".repeat(200)).chars().count(), MAX_STATUS);
    }
}

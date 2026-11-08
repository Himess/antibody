//! `antibody immune` (docs/immune.md): immune memory on pull requests.
//!
//! A program's paid proofs are its immune memory. On a pull request, every paid proof this run can read (a
//! disclosed one, one wrapped to the project's key, or one whose content key was published) is re-run with
//! the verifier, locally, at the program's anchor, against the pull request's compiled code: each target the
//! repository builds becomes a code override at the same anchor, as a round's fix is committed
//! (docs/rounds.md). A proof that still breaks a committed rule means the pull request re-opens a paid hole.
//! Nothing is sent: view calls and local verifier runs only.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use alloy_primitives::{keccak256, Address, Bytes, B256, I256, U256};
use antibody_verifier::{
    db::MissingWitness,
    facts::{sha256, Kind},
    manifest::{Manifest, Severity},
    program::ProgramError,
    run::{execute_program_witness, reserved_address, run_program, ProgramContext, VerifyError},
    sealed, trie, BaseWitness, Envelope, Payload, ProgramWitness,
};
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{cmd, ctx::Ctx};

pub const REPORT_SCHEMA: &str = "antibody.immune.v1";
/// Exit codes: no re-run proof breaks a rule (sealed ones are listed, and counted only with `--strict`).
pub const EXIT_OK: i32 = 0;
/// At least one paid proof still breaks a committed rule: the pull request re-opens a paid hole.
pub const EXIT_STILL_BREAKS: i32 = 2;
/// None breaks, but at least one re-run could not be judged (or, with `--strict`, could not be read).
pub const EXIT_INCONCLUSIVE: i32 = 3;
/// Gas for running a contract's creation code.
const CREATION_GAS: u64 = 1 << 24;

// ------------------------------------------------------------------------- the mapping

/// `.antibody/immune.toml`: the program this repository's contracts belong to, and the target each one builds.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The program's id on near-antibody (`--program` overrides it).
    pub program: Option<u32>,
    /// Foundry's output directory.
    #[serde(default = "default_artifacts")]
    pub artifacts: PathBuf,
    /// Revealed sealed blobs, found by their sha256 (a blob is public once revealed; its key is not).
    pub blobs: Option<PathBuf>,
    /// Content keys published for some proofs: commit id = a hex key or a file holding one.
    #[serde(default)]
    pub content_keys: BTreeMap<String, String>,
    /// One entry per contract the repository builds.
    #[serde(default, rename = "contract")]
    pub contracts: Vec<Mapping>,
}

fn default_artifacts() -> PathBuf {
    PathBuf::from("out")
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    /// The program's target this contract builds: its name in the manifest, or its address.
    pub target: String,
    /// The contract's name: its Foundry artifact `<artifacts>/<File>.sol/<contract>.json`.
    pub contract: Option<String>,
    /// The artifact's path, when the name is ambiguous.
    pub artifact: Option<PathBuf>,
    /// Constructor arguments, one per parameter: an address (`0x…`, or a target's name), a number (an integer, or
    /// a decimal or `0x` string), a bool, or `bytesN` as hex.
    #[serde(default)]
    pub args: Vec<toml::Value>,
    /// The ABI-encoded constructor arguments, for parameters of dynamic types.
    pub args_hex: Option<String>,
    /// A file holding the runtime code (hex), instead of an artifact.
    pub runtime: Option<PathBuf>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path).map_err(|e| eyre!("{}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| eyre!("{}: {e}", path.display()))
    }
}

/// A 32-byte key: 64 hex digits, or a file holding them.
pub fn key32(value: &str, root: &Path) -> Result<[u8; 32]> {
    let v = value.trim().trim_start_matches("0x");
    let text = if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) {
        v.to_string()
    } else {
        let p = root.join(value);
        std::fs::read_to_string(&p).map_err(|e| eyre!("{}: {e}", p.display()))?.trim().trim_start_matches("0x").to_string()
    };
    hex::decode(&text)?.try_into().map_err(|_| eyre!("a content key is 32 bytes"))
}

/// The project's X25519 secret: a key file (`antibody keygen x25519`: JSON with `secret_key`), or its hex.
pub fn project_secret(text: &str) -> Result<[u8; 32]> {
    let t = text.trim();
    let hexs = if t.starts_with('{') {
        let v: Value = serde_json::from_str(t).map_err(|_| eyre!("the project key is neither a key file nor hex"))?;
        v["secret_key"].as_str().ok_or_else(|| eyre!("the project key file has no secret_key"))?.to_string()
    } else {
        t.trim_start_matches("0x").to_string()
    };
    hex::decode(hexs).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| eyre!("the project key must be 32 bytes of hex"))
}

// ---------------------------------------------------------------- artifacts and runtime code

/// A compiled contract: its creation code, its constructor's parameter types and its functions by selector.
#[derive(Debug, Clone)]
pub struct Artifact {
    pub path: PathBuf,
    pub creation: Vec<u8>,
    pub constructor: Vec<String>,
    /// `0x` + 4 bytes -> `name(types)`.
    pub selectors: BTreeMap<String, String>,
}

/// The artifact of `contract` under Foundry's output directory: `<File>.sol/<contract>.json` (or
/// `<contract>.<solc version>.json` when several compilers ran). Build info is skipped.
pub fn find_artifact(dir: &Path, contract: &str) -> Result<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| eyre!("{}: {e} (run `forge build` first)", d.display()))?;
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().and_then(|n| n.to_str()) != Some("build-info") {
                    stack.push(p);
                }
                continue;
            }
            let Some(stem) = p.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".json")) else { continue };
            let name = match stem.split_once('.') {
                Some((n, version)) if version.chars().all(|c| c.is_ascii_digit() || c == '.') => n,
                _ => stem,
            };
            if name == contract {
                found.push(p);
            }
        }
    }
    found.sort();
    match found.len() {
        0 => Err(eyre!("no artifact for {contract} under {} (run `forge build`, or name it with `artifact`)", dir.display())),
        1 => Ok(found.remove(0)),
        _ => Err(eyre!(
            "several artifacts for {contract}: {}; name one with `artifact`",
            found.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// An ABI parameter's canonical type (tuples as `(…)`, array suffixes kept).
fn canonical_type(p: &Value) -> String {
    let t = p["type"].as_str().unwrap_or_default();
    match t.strip_prefix("tuple") {
        Some(suffix) => {
            let inner: Vec<String> = p["components"].as_array().into_iter().flatten().map(canonical_type).collect();
            format!("({}){suffix}", inner.join(","))
        }
        None => t.to_string(),
    }
}

/// Reads a Foundry artifact (or one in the same layout, with `bytecode` as a string). Contracts that link libraries
/// are refused: their creation code holds placeholders, not code.
pub fn read_artifact(path: &Path) -> Result<Artifact> {
    let v: Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| eyre!("{}: {e}", path.display()))?)
        .map_err(|e| eyre!("{}: not an artifact: {e}", path.display()))?;
    let bc = &v["bytecode"];
    if bc["linkReferences"].as_object().is_some_and(|m| !m.is_empty()) || v["linkReferences"].as_object().is_some_and(|m| !m.is_empty()) {
        return Err(eyre!("{}: the contract links libraries, which immune memory does not resolve: give its runtime code (`runtime`)", path.display()));
    }
    let object = bc["object"].as_str().or(bc.as_str()).ok_or_else(|| eyre!("{}: no bytecode", path.display()))?;
    let creation = hex::decode(object.trim_start_matches("0x")).map_err(|e| eyre!("{}: the bytecode is not hex ({e})", path.display()))?;
    if creation.is_empty() {
        return Err(eyre!("{}: no creation code (an abstract contract or an interface)", path.display()));
    }
    let abi = v["abi"].as_array().cloned().unwrap_or_default();
    let constructor =
        abi.iter().find(|f| f["type"] == "constructor").map(|f| f["inputs"].as_array().into_iter().flatten().map(canonical_type).collect()).unwrap_or_default();
    let mut selectors = BTreeMap::new();
    for f in abi.iter().filter(|f| f["type"] == "function") {
        let types: Vec<String> = f["inputs"].as_array().into_iter().flatten().map(canonical_type).collect();
        let sig = format!("{}({})", f["name"].as_str().unwrap_or_default(), types.join(","));
        selectors.insert(format!("0x{}", hex::encode(&keccak256(sig.as_bytes())[..4])), sig);
    }
    Ok(Artifact { path: path.to_path_buf(), creation, constructor, selectors })
}

/// An address argument: `0x…`, or the name of one of the program's targets.
fn address_of(s: &str, m: &Manifest) -> Result<Address, String> {
    if s.starts_with("0x") {
        return s.parse().map_err(|_| format!("{s} is not an address"));
    }
    m.targets
        .iter()
        .find(|t| t.name == s)
        .map(|t| t.address)
        .ok_or_else(|| format!("{s} is neither an address nor a target ({})", m.targets.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")))
}

fn bits_of(t: &str, prefix: &str) -> Result<usize, String> {
    let n = &t[prefix.len()..];
    let bits = if n.is_empty() { 256 } else { n.parse().map_err(|_| format!("unknown type {t}"))? };
    if bits == 0 || bits > 256 || bits % 8 != 0 {
        return Err(format!("unknown type {t}"));
    }
    Ok(bits)
}

/// One static ABI parameter as a 32-byte word.
fn encode_word(t: &str, v: &toml::Value, m: &Manifest) -> Result<[u8; 32], String> {
    let s = v.as_str();
    match t {
        "address" => {
            let a = address_of(s.ok_or("an address is a string: 0x… or a target's name")?, m)?;
            Ok(a.into_word().0)
        }
        "bool" => match (v.as_bool(), s) {
            (Some(b), _) => Ok(U256::from(b as u8).to_be_bytes()),
            (None, Some("true")) => Ok(U256::from(1u8).to_be_bytes()),
            (None, Some("false")) => Ok([0u8; 32]),
            _ => Err("a bool is true or false".into()),
        },
        t if t.starts_with("uint") => {
            let bits = bits_of(t, "uint")?;
            let x = match (v.as_integer(), s) {
                (Some(i), _) if i >= 0 => U256::from(i as u64),
                (None, Some(s)) => s.parse::<U256>().map_err(|_| format!("{s} is not a number"))?,
                _ => return Err("an unsigned number: a non-negative integer, or a decimal or 0x string".into()),
            };
            if bits < 256 && x >> bits != U256::ZERO {
                return Err(format!("{x} does not fit in {t}"));
            }
            Ok(x.to_be_bytes())
        }
        t if t.starts_with("int") => {
            let bits = bits_of(t, "int")?;
            let x = match (v.as_integer(), s) {
                (Some(i), _) => I256::try_from(i).map_err(|_| "out of range".to_string())?,
                (None, Some(s)) => I256::from_dec_str(s).map_err(|_| format!("{s} is not a decimal number"))?,
                _ => return Err("a signed number: an integer, or a decimal string".into()),
            };
            if bits < 256 {
                let max = I256::from_raw(U256::from(1u8) << (bits - 1));
                if x >= max || x < -max {
                    return Err(format!("{x} does not fit in {t}"));
                }
            }
            Ok(x.into_raw().to_be_bytes())
        }
        t if t.starts_with("bytes") && t.len() > 5 => {
            let n: usize = t[5..].parse().map_err(|_| format!("unknown type {t}"))?;
            let b = hex::decode(s.ok_or("bytesN is a hex string")?.trim_start_matches("0x")).map_err(|_| "not hex".to_string())?;
            if n == 0 || n > 32 || b.len() != n {
                return Err(format!("{t} takes exactly {n} bytes"));
            }
            let mut w = [0u8; 32];
            w[..n].copy_from_slice(&b);
            Ok(w)
        }
        other => Err(format!("{other} is not a static type: give the encoded arguments with `args_hex`")),
    }
}

/// ABI-encodes constructor arguments of static types.
pub fn encode_args(types: &[String], values: &[toml::Value], m: &Manifest) -> Result<Vec<u8>> {
    if types.len() != values.len() {
        return Err(eyre!("the constructor takes {} arguments ({}), and {} are given", types.len(), types.join(", "), values.len()));
    }
    let mut out = Vec::with_capacity(32 * types.len());
    for (i, (t, v)) in types.iter().zip(values).enumerate() {
        out.extend(encode_word(t, v, m).map_err(|e| eyre!("argument {i} ({t}): {e}"))?);
    }
    Ok(out)
}

/// The runtime code a creation code leaves. It runs as a call at `at`, the target's own address, so
/// `address(this)` is the target's (as an immutable recording an implementation's own address needs), on an empty
/// state, under the anchor's EVM rules. A constructor may set immutables and storage (storage is ignored: an
/// override keeps the target's storage at the anchor), but not read other contracts.
pub fn runtime_of(creation: &[u8], at: Address, evm_spec: &str, chain_id: u64, block: u64) -> Result<Bytes> {
    use revm::{
        context::{result::EVMError, BlockEnv, CfgEnv, ContextSetters, TxEnv},
        database::{CacheDB, EmptyDB},
        handler::{EthFrame, Handler, MainnetHandler},
        interpreter::interpreter::EthInterpreter,
        primitives::TxKind,
        state::{AccountInfo, Bytecode},
        Context, MainBuilder, MainContext,
    };
    let spec = antibody_verifier::program::spec_id(evm_spec).ok_or_else(|| eyre!("unsupported evm_spec {evm_spec}"))?;
    let code = Bytecode::new_raw(Bytes::copy_from_slice(creation));
    let mut db = CacheDB::new(EmptyDB::default());
    db.insert_account_info(at, AccountInfo { balance: U256::ZERO, nonce: 1, code_hash: code.hash_slow(), code: Some(code), account_id: None });
    let block = BlockEnv { number: U256::from(block), ..Default::default() };
    let mut evm = Context::mainnet().with_db(db).with_block(block).with_cfg(CfgEnv::new_with_spec(spec).with_chain_id(chain_id)).build_mainnet();
    let tx = TxEnv::builder()
        .caller(antibody_verifier::manifest::deployer())
        .kind(TxKind::Call(at))
        .gas_limit(CREATION_GAS)
        .build()
        .map_err(|e| eyre!("creation call: {e:?}"))?;
    evm.set_tx(tx);
    let r = MainnetHandler::<_, EVMError<std::convert::Infallible>, EthFrame<EthInterpreter>>::default()
        .run_system_call(&mut evm)
        .map_err(|e| eyre!("creation code: {e:?}"))?;
    let out = r.output().cloned().unwrap_or_default();
    if !r.is_success() {
        return Err(eyre!("the constructor reverted on an empty state (0x{}): a constructor that reads other contracts needs `runtime`", hex::encode(&out)));
    }
    if out.is_empty() {
        return Err(eyre!("the creation code returned no runtime code"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------- the build

/// A target the repository builds: the code this build gives it.
#[derive(Debug, Clone, Serialize)]
pub struct Built {
    pub target: String,
    pub address: Address,
    /// The contract and its artifact, or the runtime file.
    pub from: String,
    pub code_hash: B256,
    #[serde(skip)]
    pub runtime: Bytes,
    #[serde(skip)]
    pub selectors: BTreeMap<String, String>,
}

/// The target a mapping names: by name or address.
fn target_of<'a>(m: &'a Manifest, t: &str) -> Result<&'a antibody_verifier::manifest::Target> {
    let by_address = t.parse::<Address>().ok();
    m.targets
        .iter()
        .find(|x| x.name == t || Some(x.address) == by_address)
        .ok_or_else(|| eyre!("{t} is not one of the program's targets ({})", m.targets.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(", ")))
}

/// Each mapped contract's runtime code: from its artifact (creation code and constructor arguments, run at the
/// target's address), or from a runtime file. Relative paths start at `root`.
pub fn build(cfg: &Config, root: &Path, m: &Manifest) -> Result<Vec<Built>> {
    let mut out: Vec<Built> = Vec::new();
    for (i, c) in cfg.contracts.iter().enumerate() {
        let t = target_of(m, &c.target).map_err(|e| eyre!("contract {i}: {e}"))?;
        if out.iter().any(|b| b.address == t.address) {
            return Err(eyre!("contract {i}: target {} is mapped twice", t.name));
        }
        let (runtime, from, selectors) = match (&c.runtime, &c.contract, &c.artifact) {
            (Some(file), None, None) => {
                if !c.args.is_empty() || c.args_hex.is_some() {
                    return Err(eyre!("contract {i}: a runtime file takes no constructor arguments"));
                }
                let p = root.join(file);
                let text = std::fs::read_to_string(&p).map_err(|e| eyre!("{}: {e}", p.display()))?;
                let code = hex::decode(text.trim().trim_start_matches("0x")).map_err(|e| eyre!("{}: not hex ({e})", p.display()))?;
                (Bytes::from(code), file.display().to_string().replace('\\', "/"), BTreeMap::new())
            }
            (None, name, artifact) if name.is_some() || artifact.is_some() => {
                let path = match artifact {
                    Some(a) => root.join(a),
                    None => find_artifact(&root.join(&cfg.artifacts), name.as_deref().unwrap_or_default())?,
                };
                let a = read_artifact(&path)?;
                let args = match (&c.args_hex, c.args.is_empty()) {
                    (Some(_), false) => return Err(eyre!("contract {i}: give `args` or `args_hex`, not both")),
                    (Some(h), true) => hex::decode(h.trim().trim_start_matches("0x")).map_err(|e| eyre!("contract {i}: args_hex is not hex ({e})"))?,
                    (None, _) => encode_args(&a.constructor, &c.args, m).map_err(|e| eyre!("contract {i} ({}): {e}", t.name))?,
                };
                let creation = [a.creation.as_slice(), args.as_slice()].concat();
                let runtime = runtime_of(&creation, t.address, &m.anchor.evm_spec, m.anchor.chain_id, m.anchor.block_number)
                    .map_err(|e| eyre!("contract {i} ({}): {e}", t.name))?;
                let shown = path.strip_prefix(root).unwrap_or(&path).display().to_string().replace('\\', "/");
                let label = format!("{} ({shown})", name.clone().unwrap_or_else(|| "artifact".into()));
                (runtime, label, a.selectors)
            }
            _ => return Err(eyre!("contract {i}: give `contract` (or `artifact`), or `runtime`")),
        };
        out.push(Built { target: t.name.clone(), address: t.address, from, code_hash: keccak256(&runtime), runtime, selectors });
    }
    Ok(out)
}

/// The code a target runs in the re-runs.
#[derive(Debug, Clone, Serialize)]
pub struct TargetCode {
    pub target: String,
    pub address: Address,
    /// `pull request`, `pull request (identical to the anchor's)`, `round N` or `anchor`.
    pub code: String,
    pub from: Option<String>,
    pub code_hash: B256,
}

/// The rules and code the re-runs judge: the program's current round's rules, every target this build gives code
/// to as an override at the same anchor, every other target with the current round's code (its committed fix),
/// or the anchor's. It is the round manifest the pull request would propose as the next round.
pub fn pr_manifest(m0: &Manifest, current: &Manifest, built: &[Built]) -> Result<(Manifest, Vec<TargetCode>)> {
    let mut code = Vec::new();
    let mut rows = Vec::new();
    for t in &m0.targets {
        let row = |c: &str, from: Option<String>, h: B256| TargetCode { target: t.name.clone(), address: t.address, code: c.into(), from, code_hash: h };
        if let Some(b) = built.iter().find(|b| b.address == t.address) {
            if b.code_hash == t.code_hash {
                rows.push(row("pull request (identical to the anchor's)", Some(b.from.clone()), b.code_hash));
            } else {
                code.push((t.address, b.runtime.clone()));
                rows.push(row("pull request", Some(b.from.clone()), b.code_hash));
            }
        } else if let Some(o) = current.overrides.iter().find(|o| o.address == t.address) {
            code.push((t.address, o.runtime_code.clone()));
            rows.push(row(&format!("round {}", current.round), None, o.code_hash));
        } else {
            rows.push(row("anchor", None, t.code_hash));
        }
    }
    let mut m = if code.is_empty() { m0.clone() } else { m0.with_overrides(current.round + 1, &code).map_err(|e| eyre!(e))? };
    if !code.is_empty() {
        m.rule_strength = None;
    }
    m.invariants = current.invariants.clone();
    Ok((m, rows))
}

// ------------------------------------------------------------------------- re-runs

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// A committed rule still breaks: the pull request re-opens a paid hole.
    StillBreaks,
    /// The proof ran and no committed rule broke.
    Fixed,
    /// The re-run could not judge the proof (the reason says exactly why). Never counted as fixed.
    Inconclusive,
    /// The proof could not be read here: it was not re-run.
    Sealed,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Outcome::StillBreaks => "still breaks",
            Outcome::Fixed => "fixed",
            Outcome::Inconclusive => "inconclusive",
            Outcome::Sealed => "not re-run (sealed)",
        }
    }
}

/// A rule a re-run broke, and where.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Broken {
    /// None when withheld: a rule a sealed proof breaks that its commit's keys do not name (`withhold`).
    pub slot: Option<u8>,
    pub rule: String,
    pub severity: String,
    /// `Target.function(types)`, or the selector when no artifact names it.
    pub entry_point: String,
    pub target: Address,
    pub selector: String,
}

pub fn severity(s: Severity) -> &'static str {
    match s {
        Severity::Low => "low",
        Severity::Medium => "medium",
        Severity::High => "high",
        Severity::Critical => "critical",
    }
}

/// A proof's witness, assembled as the TEE assembles it (`antibody_verifier::tee`): a submission merged with a
/// base witness, judged under `manifest` and that manifest's own rules. With `widen`, a full witness also takes
/// the base's proven state (more of the same anchor state, for code that reads more than the proof did).
pub fn assemble(inner: &Envelope, base: Option<&BaseWitness>, manifest: &Bytes, widen: bool) -> Result<(Envelope, ProgramWitness), String> {
    let invariants = Manifest::parse(manifest)?.invariants.runtime_code;
    let mut w = match inner.payload.clone() {
        Payload::Program(mut w) => {
            if let (true, Some(b)) = (widen, base) {
                w.state = trie::merge(&w.state, &b.state)?;
                for c in &b.bytecodes {
                    if !w.bytecodes.iter().any(|x| x.original_byte_slice() == c.original_byte_slice()) {
                        w.bytecodes.push(c.clone());
                    }
                }
            }
            w.manifest_json = manifest.clone();
            w
        }
        Payload::Submission(sub) => ProgramWitness::assemble(base.ok_or("a submission needs the program's base witness")?, &sub, manifest.clone())?,
        _ => return Err("the sealed witness is not a program witness".into()),
    };
    w.invariants_code = invariants;
    let mut env = inner.clone();
    env.payload = Payload::Program(w.clone());
    Ok((env, w))
}

/// Several base witnesses of the same anchor as one: every proven node of each.
pub fn merge_bases(bases: &[BaseWitness]) -> Result<Option<BaseWitness>, String> {
    let Some(first) = bases.first() else { return Ok(None) };
    let mut out = first.clone();
    for b in &bases[1..] {
        out.state = trie::merge(&out.state, &b.state)?;
        for c in &b.bytecodes {
            if !out.bytecodes.iter().any(|x| x.original_byte_slice() == c.original_byte_slice()) {
                out.bytecodes.push(c.clone());
            }
        }
    }
    Ok(Some(out))
}

fn named(a: Address, m: &Manifest) -> String {
    match m.target_name(a) {
        Some(n) => format!("{n} ({a})"),
        None => a.to_string(),
    }
}

/// What a re-run needed that the proof's witness does not prove.
pub fn missing(m: &MissingWitness, manifest: &Manifest) -> String {
    match m {
        MissingWitness::Account(a) => format!("the account {}", named(*a, manifest)),
        MissingWitness::StorageTrie(a) => format!("the storage of {}", named(*a, manifest)),
        MissingWitness::Slot(a, s) => format!("storage slot {s:#x} of {}", named(*a, manifest)),
        MissingWitness::Code(h) => format!("the code with hash {h}"),
        MissingWitness::BlockHash(n) => format!("the hash of block {n}"),
    }
}

/// The verdict of one re-run.
#[derive(Debug, Clone)]
pub struct Rerun {
    pub outcome: Outcome,
    pub broken: Vec<Broken>,
    pub reason: Option<String>,
}

/// Re-runs a proof's assembled witness with the verifier, as the TEE does (the program's reserved invariants
/// address, the manifest's rules), and classifies the result. A witness that lacks state the code reads, rules
/// that cannot be evaluated, or any other verdict than Violation or NoViolation is inconclusive, never fixed.
pub fn rerun(env: &Envelope, w: &ProgramWitness, program_key: &str, selectors: &BTreeMap<String, String>) -> Rerun {
    let m = match Manifest::parse(&w.manifest_json) {
        Ok(m) => m,
        Err(e) => return Rerun { outcome: Outcome::Inconclusive, broken: vec![], reason: Some(e) },
    };
    let inconclusive = |r: String| Rerun { outcome: Outcome::Inconclusive, broken: vec![], reason: Some(r) };
    match execute_program_witness(env, w, None, Some(reserved_address(program_key, env.claimed_block_hash))) {
        Ok(x) if x.run.broken_mask.is_zero() => Rerun { outcome: Outcome::Fixed, broken: vec![], reason: None },
        Ok(x) => {
            let broken = x
                .run
                .broken
                .iter()
                .map(|f| {
                    let function = selectors.get(&f.selector).cloned().unwrap_or_else(|| f.selector.clone());
                    let entry_point = if f.target.is_zero() {
                        "unattributed (outside any protocol call)".to_string()
                    } else {
                        format!("{}.{function}", f.target_name.clone().unwrap_or_else(|| f.target.to_string()))
                    };
                    Broken {
                        slot: Some(f.slot),
                        rule: f.name.clone(),
                        severity: severity(f.severity).into(),
                        entry_point,
                        target: f.target,
                        selector: f.selector.clone(),
                    }
                })
                .collect();
            Rerun { outcome: Outcome::StillBreaks, broken, reason: None }
        }
        Err(ProgramError::Verify(VerifyError::Incomplete(miss))) => inconclusive(format!(
            "the code under test reads {}, which this proof's witness does not prove (a witness covering it needs the anchor's state)",
            missing(&miss, &m)
        )),
        Err(ProgramError::Verdict { kind: Kind::Unevaluable, reason }) => inconclusive(format!("the rules could not be evaluated: {reason}")),
        Err(ProgramError::Verdict { kind: Kind::ProgramMisconfigured, reason }) => {
            inconclusive(format!("the program's checks at the anchor fail with this code: {reason}"))
        }
        Err(e) => {
            let (kind, reason) = antibody_verifier::program::classify(&e);
            inconclusive(format!("{kind:?}: {reason}"))
        }
    }
}

// --------------------------------------------------------------------------- the report

/// One paid proof and its re-run.
#[derive(Debug, Clone, Serialize)]
pub struct ProofRow {
    pub commit: u64,
    pub round: u32,
    pub hunter: String,
    pub state: String,
    /// What it was paid, or is due or owed, in whole tokens.
    pub paid: String,
    /// The rules its keys name, as paid: `name (severity)`.
    pub paid_for: Vec<String>,
    /// `disclosure`, `published content key` or `project key`; none when sealed.
    pub read_through: Option<String>,
    pub outcome: Outcome,
    pub broken: Vec<Broken>,
    pub reason: Option<String>,
    /// Whether the local run of the proof, as the TEE ran it, gave the TEE's receipt hash.
    pub receipt_matches_the_tee: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub paid: usize,
    pub rerun: usize,
    pub still_breaks: usize,
    pub fixed: usize,
    pub inconclusive: usize,
    pub sealed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub network: String,
    pub contract: String,
    pub program: u32,
    pub program_key: String,
    pub name: String,
    pub chain_id: u64,
    pub anchor_block: u64,
    /// The round whose rules the re-runs apply (the program's current round).
    pub rules_round: u32,
    pub build: Vec<TargetCode>,
    /// sha256 of the pull request's code as a round manifest.
    pub manifest_sha256: String,
    pub proofs: Vec<ProofRow>,
    pub summary: Summary,
    pub strict: bool,
    pub exit_code: i32,
    pub verifier: String,
    pub transactions_sent: u32,
}

impl Report {
    /// Fills the summary and the exit code from the rows.
    pub fn finish(mut self) -> Report {
        let n = |o: Outcome| self.proofs.iter().filter(|p| p.outcome == o).count();
        self.summary = Summary {
            paid: self.proofs.len(),
            rerun: self.proofs.len() - n(Outcome::Sealed),
            still_breaks: n(Outcome::StillBreaks),
            fixed: n(Outcome::Fixed),
            inconclusive: n(Outcome::Inconclusive),
            sealed: n(Outcome::Sealed),
        };
        self.exit_code = exit_code(&self.summary, self.strict);
        self
    }
}

pub fn exit_code(s: &Summary, strict: bool) -> i32 {
    if s.still_breaks > 0 {
        EXIT_STILL_BREAKS
    } else if s.inconclusive > 0 || (strict && s.sealed > 0) {
        EXIT_INCONCLUSIVE
    } else {
        EXIT_OK
    }
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The report for a job summary.
pub fn markdown(r: &Report) -> String {
    let s = &r.summary;
    let head = match r.exit_code {
        EXIT_STILL_BREAKS => {
            format!("{} still {} a rule", plural(s.still_breaks, "paid proof", "paid proofs"), if s.still_breaks == 1 { "breaks" } else { "break" })
        }
        EXIT_INCONCLUSIVE if s.inconclusive > 0 => {
            format!("inconclusive: no paid proof breaks a rule, but {} could not be judged", plural(s.inconclusive, "re-run", "re-runs"))
        }
        EXIT_INCONCLUSIVE => format!("inconclusive: {} could not be read (--strict)", plural(s.sealed, "paid proof", "paid proofs")),
        _ if s.paid == 0 => "the program has no paid proof yet".to_string(),
        _ if s.rerun == 0 => "nothing re-run: no paid proof is readable here".to_string(),
        _ if s.sealed > 0 => format!("no paid proof re-run here breaks a rule ({} of {} re-run, {} sealed)", s.rerun, s.paid, s.sealed),
        _ => format!("no paid proof breaks a rule ({} re-run)", s.rerun),
    };
    let mut o = format!("### Antibody immune memory: {head}\n\n");
    o += &format!(
        "Program {} on {} (*{}*, `{}`), anchored at chain {} block {}. Each paid proof this run could read was re-run with the \
         verifier, locally, at the program's anchor, against this build's code, under round {}'s rules. Nothing was sent.\n\n",
        r.program,
        r.network,
        cell(&r.name),
        r.program_key,
        r.chain_id,
        r.anchor_block,
        r.rules_round
    );
    if r.proofs.is_empty() {
        o += "The program has no paid proof yet.\n\n";
    } else {
        o += "| Result | Commit | Paid for | Read through | Rule broken | Entry point |\n|---|---|---|---|---|---|\n";
        for p in &r.proofs {
            let result = match p.outcome {
                Outcome::StillBreaks => "**still breaks**".to_string(),
                o => o.label().to_string(),
            };
            let paid_for = if p.paid_for.is_empty() { "no key (gap claim)".to_string() } else { p.paid_for.join(", ") };
            let rules = p.broken.iter().map(|b| format!("{} ({})", b.rule, b.severity)).collect::<Vec<_>>().join(", ");
            let entries = p.broken.iter().map(|b| format!("`{}`", b.entry_point)).collect::<Vec<_>>().join(", ");
            o += &format!(
                "| {result} | {} by {} (round {}, {}) | {}, {} | {} | {} | {} |\n",
                p.commit,
                cell(&p.hunter),
                p.round,
                cell(&p.state),
                cell(&paid_for),
                cell(&p.paid),
                p.read_through.as_deref().unwrap_or("-"),
                if rules.is_empty() { "-".into() } else { cell(&rules) },
                if entries.is_empty() { "-".into() } else { cell(&entries) },
            );
        }
        o += "\n";
        for p in r.proofs.iter().filter(|p| p.reason.is_some()) {
            o += &format!("- Commit {}, {}: {}\n", p.commit, p.outcome.label(), p.reason.as_deref().unwrap_or_default());
        }
        let checked: Vec<String> =
            r.proofs.iter().filter_map(|p| p.receipt_matches_the_tee.map(|m| format!("{} {}", p.commit, if m { "yes" } else { "NO" }))).collect();
        if !checked.is_empty() {
            o += &format!("- The TEE's receipt reproduced locally before the re-run (commit: match): {}\n", checked.join(", "));
        }
        o += "\n";
    }
    let from_build = r.build.iter().filter(|t| t.code.starts_with("pull request")).count();
    o += &format!(
        "<details><summary>The code under test: {}, {} from this build (as a round manifest, sha256 <code>{}</code>)</summary>\n\n",
        plural(r.build.len(), "target", "targets"),
        from_build,
        &r.manifest_sha256
    );
    o += "| Target | Address | Code | From | Code hash |\n|---|---|---|---|---|\n";
    for t in &r.build {
        o += &format!(
            "| {} | `{}` | {} | {} | `{}` |\n",
            cell(&t.target),
            t.address,
            t.code,
            t.from.as_deref().map(cell).unwrap_or_else(|| "-".into()),
            t.code_hash
        );
    }
    o += "\n</details>\n\n";
    o += &format!(
        "Exit code {}. 0: no re-run paid proof breaks a rule; 2: a paid proof still breaks one; 3: none breaks, but a re-run was \
         inconclusive{}. {}. Sealed proofs re-run inside the TEE are next, not built.\n",
        r.exit_code,
        if r.strict { " or a paid proof could not be read" } else { "" },
        r.verifier
    );
    o
}

// ------------------------------------------------------------------------ reading proofs

/// How a run reads proofs.
#[derive(Default)]
pub struct Opts {
    pub config: PathBuf,
    pub program: Option<u32>,
    pub root: PathBuf,
    pub project_key: Option<[u8; 32]>,
    pub content_keys: BTreeMap<u64, [u8; 32]>,
    pub blobs: Option<PathBuf>,
    pub api: Option<String>,
    pub strict: bool,
    /// Show where a proof read with the project key breaks a rule, and every rule it breaks. Off by default: such a
    /// proof is still sealed, its entry point is what the on-chain tag hides, and only the rules its commit's keys
    /// name are public (a job summary of a public repository is public).
    pub reveal_sealed: bool,
}

/// What a report may say about a proof read with the project key. Such a proof is still sealed: the report names a
/// rule it breaks only when the commit's keys on chain already name that rule's slot (`key_slots`, public). Any
/// other rule it breaks now (the code changed, a later round added the rule, or the verdict's keys were capped) is
/// `another rule (withheld)`, with no slot. Never where it breaks, nor what its run read.
pub fn withhold(row: &mut ProofRow, key_slots: &[u8]) {
    for b in &mut row.broken {
        if !b.slot.is_some_and(|s| key_slots.contains(&s)) {
            b.slot = None;
            b.rule = "another rule".into();
            b.severity = "withheld".into();
        }
        b.entry_point = "withheld (a sealed proof)".into();
        b.target = Address::ZERO;
        b.selector = String::new();
    }
    // The verifier lists broken rules by slot: the named ones go first, so a withheld rule's place does not tell its
    // slot from theirs.
    row.broken.sort_by_key(|b| b.slot.is_none());
    if row.outcome == Outcome::Inconclusive {
        row.reason = Some("details withheld (a sealed proof): re-run it locally with the project key and --reveal-sealed".into());
    }
}

/// States of a commit whose proof was paid or held a key: the program's immune memory.
pub fn is_paid(state: &str) -> bool {
    matches!(state, "Done:Paid" | "Holder" | "Promoted" | "Done:Exhausted" | "Done:Defaulted")
}

fn h32(s: &str) -> Option<[u8; 32]> {
    hex::decode(s.trim_start_matches("0x")).ok()?.try_into().ok()
}

/// The disclosed (round, key) pairs of a program (`get_program`'s `disclosed`: `[round, tag, key]`).
pub fn disclosed_keys(p: &Value) -> Vec<(u32, [u8; 32])> {
    p["disclosed"].as_array().into_iter().flatten().filter_map(|d| Some((d[0].as_u64()? as u32, h32(d[2].as_str()?)?))).collect()
}

/// The content key a disclosure key opens from a verify run's `wrap_d` (nonce ‖ AEAD under the program key).
pub fn open_wrap_d(wrap_d: &str, disclosure_key: &[u8; 32], program_key: &str) -> Option<[u8; 32]> {
    let b = hex::decode(wrap_d).ok()?;
    if b.len() < 12 {
        return None;
    }
    let nonce: [u8; 12] = b[..12].try_into().ok()?;
    sealed::aead_open_pub(disclosure_key, &nonce, &b[12..], program_key.as_bytes()).ok()?.try_into().ok()
}

/// The blobs of a directory by sha256.
fn blob_index(dir: &Path) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_file() && e.metadata().map(|m| m.len() <= 4 * 1024 * 1024).unwrap_or(false) {
            if let Ok(b) = std::fs::read(&p) {
                out.insert(hex::encode(sha256(&b)), p);
            }
        }
    }
    out
}

fn answered(run: &Value) -> &Value {
    &run["outcome"]["Answered"]["facts"]
}

/// Where a run looks for keys: the program's disclosure keys, published content keys and the project key.
struct Keys<'a> {
    disclosed: Vec<(u32, [u8; 32])>,
    published: BTreeMap<u64, [u8; 32]>,
    project: Option<&'a [u8; 32]>,
}

/// The content key of a paid commit, and through what it was read; or why the proof stays sealed here.
fn content_key(ctx: &Ctx, keys: &Keys, c: &Value, verify: &Value, key: &str) -> Result<([u8; 32], &'static str), String> {
    let id = c["id"].as_u64().unwrap_or_default();
    let round = c["round"].as_u64().unwrap_or_default() as u32;
    if let Some(w) = answered(verify)["extra"]["WrapD"]["wrap_d"].as_str() {
        if let Some(k) = keys.disclosed.iter().filter(|(r, _)| *r == round).find_map(|(_, dk)| open_wrap_d(w, dk, key)) {
            return Ok((k, "disclosure"));
        }
    }
    if let Some(k) = keys.published.get(&id) {
        return Ok((*k, "published content key"));
    }
    let project = match (keys.project, c["wrapped"].as_u64()) {
        (None, _) => "no project key here (a repository secret, never given to pull requests from forks)".to_string(),
        (Some(_), None) => format!("no release wrap for the project key yet (`antibody program read --commit {id}` asks for one, once)"),
        (Some(sk), Some(run_id)) => match unwrap_release(&cmd::run(ctx, run_id).map_err(|e| e.to_string())?, sk, key) {
            Ok(k) => return Ok((k, "project key")),
            Err(e) => format!("the release wrap (run {run_id}) {e}"),
        },
    };
    Err(format!("not disclosed, no published content key, and {project}"))
}

/// The content key a `release_wrap` run wrapped to the project, opened with the project's secret.
pub fn unwrap_release(wrap_run: &Value, project_secret: &[u8; 32], program_key: &str) -> Result<[u8; 32], String> {
    let wrap = answered(wrap_run)["extra"]["Wrap"]["wrap"].as_str().ok_or("has no wrap")?;
    let wrap = hex::decode(wrap).map_err(|_| "is not hex")?;
    sealed::project_unwrap(&wrap, &x25519_dalek::StaticSecret::from(*project_secret), program_key)
        .map_err(|e| format!("does not open with this project key ({e})"))
}

/// A readable paid proof: its sealed payload's witness and the TEE's receipt hash of its verify run.
struct Opened {
    inner: Envelope,
    witness_sha256: [u8; 32],
    read_through: &'static str,
}

fn open_proof(ctx: &Ctx, o: &Opts, keys: &Keys, c: &Value, verify: &Value, key: &str, blobs: &BTreeMap<String, PathBuf>) -> Result<Opened, String> {
    let (k, read_through) = content_key(ctx, keys, c, verify, key)?;
    let want = c["blob_sha256"].as_str().ok_or("the commit was never revealed")?.to_string();
    let blob = match blobs.get(&want) {
        Some(p) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?,
        None => match &o.api {
            Some(api) => cmd::commit_blob(ctx, c, &cmd::BlobFrom::Api(api.clone())).map_err(|e| e.to_string())?.0,
            None => return Err(format!("readable through {read_through}, but its blob ({want}) is not among the revealed blobs given (--blobs, or --api)")),
        },
    };
    if hex::encode(sha256(&blob)) != want {
        return Err(format!("the blob found does not hash to the commit's {want}"));
    }
    let payload = sealed::open_with_key(&blob, &k, key).map_err(|e| format!("the key does not open the blob: {e}"))?;
    let hunter = c["hunter"].as_str().unwrap_or_default();
    if payload.program_key != key || payload.hunter != hunter {
        return Err("the sealed payload is bound to another program or hunter".into());
    }
    let inner = Envelope::decode(&payload.envelope)?;
    Ok(Opened { witness_sha256: sha256(&payload.envelope), inner, read_through })
}

fn decode_base(bytes: &[u8]) -> Result<BaseWitness> {
    match Envelope::decode(bytes).map_err(|e| eyre!("base witness: {e}"))?.payload {
        Payload::Base(b) => Ok(b),
        _ => Err(eyre!("the pinned base is not a base witness")),
    }
}

/// Every base witness the program pinned (the activation's and each round's), checked against its hash.
fn pinned_bases(ctx: &Ctx, meta: &Value) -> Result<Vec<BaseWitness>> {
    let mut locs = vec![meta["base"].clone()];
    locs.extend(meta["round_bases"].as_array().cloned().unwrap_or_default());
    let mut out = Vec::new();
    for loc in locs.iter().filter(|l| !l.is_null()) {
        let bytes = crate::blob::fetch(&ctx.near, loc["location"].as_str().unwrap_or_default())?;
        let want = loc["sha256"].as_str().unwrap_or_default().trim_start_matches("0x").to_lowercase();
        if hex::encode(sha256(&bytes)) != want {
            return Err(eyre!("the base witness at {} does not hash to its pinned {want}", loc["location"]));
        }
        out.push(decode_base(&bytes)?);
    }
    Ok(out)
}

/// `antibody immune`: reads the program (view calls), builds the pull request's code from the mapping, opens
/// every readable paid proof and re-runs it locally. Nothing is sent.
pub fn run(ctx: &Ctx, o: &Opts) -> Result<(Report, Manifest)> {
    let cfg = Config::load(&o.config)?;
    let program = o.program.or(cfg.program).ok_or_else(|| eyre!("no program: pass --program or set `program` in {}", o.config.display()))?;
    let p = ctx.program(program)?;
    let meta = cmd::meta(ctx, program)?;
    let key = p["key"].as_str().unwrap_or_default().to_string();
    let round = p["round"].as_u64().unwrap_or(0) as u32;
    let m0_bytes = cmd::program_manifest_at(ctx, program, 0)?;
    let m0 = Manifest::parse(&m0_bytes).map_err(|e| eyre!(e))?;
    let current = if round == 0 { m0.clone() } else { Manifest::parse(&cmd::program_manifest_at(ctx, program, round)?).map_err(|e| eyre!(e))? };
    let built = build(&cfg, &o.root, &m0)?;
    let (pr, rows) = pr_manifest(&m0, &current, &built)?;
    let pr_bytes: Bytes = pr.canonical().into();
    let selectors: BTreeMap<String, String> = built.iter().flat_map(|b| b.selectors.clone()).collect();
    let bases = pinned_bases(ctx, &meta)?;
    let widened = merge_bases(&bases).map_err(|e| eyre!("the program's base witnesses do not merge: {e}"))?;
    let mut published = o.content_keys.clone();
    for (commit, value) in &cfg.content_keys {
        let id: u64 = commit.trim().parse().map_err(|_| eyre!("{}: content key {commit:?} is not a commit id", o.config.display()))?;
        if let std::collections::btree_map::Entry::Vacant(e) = published.entry(id) {
            e.insert(key32(value, &o.root)?);
        }
    }
    let keys = Keys { disclosed: disclosed_keys(&p), published, project: o.project_key.as_ref() };
    let blobs = o.blobs.clone().or(cfg.blobs.clone()).map(|d| blob_index(&o.root.join(d))).unwrap_or_default();
    let (decimals, symbol) = (ctx.dep.decimals(), ctx.dep.symbol().to_string());
    let mut proofs = Vec::new();
    // The manifest and base each proof was judged under, by round, to reproduce the TEE's receipt.
    let mut judged: BTreeMap<u32, (Bytes, BaseWitness)> = BTreeMap::new();
    let mut verifier_version: Option<&'static str> = None;
    for c in cmd::list_commits(ctx, program)? {
        let state = cmd::state_label(&c["state"]);
        if !is_paid(&state) {
            continue;
        }
        let commit = c["id"].as_u64().unwrap_or_default();
        let cround = c["round"].as_u64().unwrap_or(0) as u32;
        let verify = match c["run"].as_u64() {
            Some(r) => cmd::run(ctx, r)?,
            None => Value::Null,
        };
        let tokens = |raw: u128| format!("{} {symbol}", cmd::human_tokens(raw, decimals));
        let paid = match state.as_str() {
            "Done:Paid" => tokens(cmd::paid_of(&c)),
            "Done:Exhausted" => "nothing (the rule's budget ran out)".to_string(),
            "Done:Defaulted" => format!("{} owed", tokens(cmd::u128_of(&c["state"]["Done"]["Defaulted"]["owed"]))),
            s => format!("{} due", tokens(cmd::u128_of(&c["state"][s]["amount"]))),
        };
        // The slots the verdict's keys name: public on chain, so these rules may be named even for a sealed proof.
        let key_slots: Vec<u8> = answered(&verify)["keys"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| k["slot"].as_u64().and_then(|s| u8::try_from(s).ok()))
            .collect();
        let paid_for = key_slots
            .iter()
            .map(|&slot| match current.slot(slot) {
                Some(s) => format!("{} ({})", s.name, severity(s.severity)),
                None => format!("slot {slot}"),
            })
            .collect();
        let mut row = ProofRow {
            commit,
            round: cround,
            hunter: c["hunter"].as_str().unwrap_or_default().to_string(),
            state: state.clone(),
            paid,
            paid_for,
            read_through: None,
            outcome: Outcome::Sealed,
            broken: vec![],
            reason: None,
            receipt_matches_the_tee: None,
        };
        let opened = match open_proof(ctx, o, &keys, &c, &verify, &key, &blobs) {
            Ok(x) => x,
            Err(why) => {
                row.reason = Some(why);
                proofs.push(row);
                continue;
            }
        };
        row.read_through = Some(opened.read_through.to_string());
        // The proof as the TEE ran it: the receipt hash must be the TEE's before a "fixed" counts.
        if let std::collections::btree_map::Entry::Vacant(e) = judged.entry(cround) {
            let (mb, base): (Bytes, BaseWitness) = match (cround, bases.first()) {
                (0, Some(b)) if !meta["base"].is_null() => (m0_bytes.clone().into(), b.clone()),
                _ => (cmd::program_manifest_at(ctx, program, cround)?.into(), decode_base(&cmd::pinned_base_at(ctx, program, cround)?)?),
            };
            e.insert((mb, base));
        }
        let (mb, base) = &judged[&cround];
        let tee = answered(&verify)["receipt_sha256"].as_str().map(str::to_string);
        let reproduced = assemble(&opened.inner, Some(base), mb, false).map(|(env, w)| {
            let pctx = ProgramContext {
                program_id: key.clone(),
                commit_id: commit,
                expected_manifest_hash: Some(B256::from(sha256(mb))),
                reserved: Some(reserved_address(&key, env.claimed_block_hash)),
            };
            let receipt = run_program(&env, &w, opened.witness_sha256, &pctx);
            verifier_version = Some(receipt.verifier.version);
            hex::encode(receipt.hash())
        });
        row.receipt_matches_the_tee = match (&reproduced, &tee) {
            (Ok(local), Some(t)) => Some(local == t),
            _ => None,
        };
        let r = match assemble(&opened.inner, widened.as_ref(), &pr_bytes, true) {
            Ok((env, w)) => rerun(&env, &w, &key, &selectors),
            Err(e) => Rerun { outcome: Outcome::Inconclusive, broken: vec![], reason: Some(format!("the witness does not assemble: {e}")) },
        };
        row.outcome = r.outcome;
        row.broken = r.broken;
        row.reason = r.reason;
        // "No rule broke" counts only from a run that reproduces the TEE's own receipt for this proof.
        let unreproduced = match (&reproduced, &tee, row.receipt_matches_the_tee) {
            (_, _, Some(true)) => None,
            (Err(e), _, _) => Some(format!("the proof could not be run as the TEE ran it ({e})")),
            (Ok(_), None, _) => Some("the TEE's receipt for this proof is not on chain to compare with".to_string()),
            (Ok(_), Some(_), _) => Some("the local run of the proof as the TEE ran it does not give the TEE's receipt".to_string()),
        };
        if let Some(why) = unreproduced {
            if row.outcome == Outcome::Fixed {
                row.outcome = Outcome::Inconclusive;
                row.reason = Some(format!("no rule broke, but {why}, so that proves nothing"));
            } else if row.reason.is_none() {
                row.reason = Some(format!("note: {why}"));
            }
        }
        if opened.read_through == "project key" && !o.reveal_sealed {
            withhold(&mut row, &key_slots);
        }
        proofs.push(row);
    }
    let release = ctx.dep.release.clone().unwrap_or(Value::Null);
    let report = Report {
        schema: REPORT_SCHEMA,
        network: ctx.near.network.clone(),
        contract: ctx.dep.antibody.clone(),
        program,
        program_key: key,
        name: meta["name"].as_str().unwrap_or_default().to_string(),
        chain_id: m0.anchor.chain_id,
        anchor_block: m0.anchor.block_number,
        rules_round: round,
        build: rows,
        manifest_sha256: hex::encode(sha256(&pr_bytes)),
        proofs,
        summary: Summary::default(),
        strict: o.strict,
        exit_code: EXIT_OK,
        verifier: verifier_line(verifier_version, release["verifier"]["wasm_hash"].as_str()),
        transactions_sent: 0,
    }
    .finish();
    Ok((report, pr))
}

/// The verifier this binary links (its version as its receipts give it), and the build the deployment pins for the
/// TEE.
pub fn verifier_line(version: Option<&str>, pinned: Option<&str>) -> String {
    let base = match version {
        Some(v) => format!("The local verifier is antibody-verifier {v}"),
        None => "The local verifier is the antibody-verifier crate this binary links".to_string(),
    };
    match pinned {
        Some(h) => format!("{base}, the crate the TEE's pinned build (wasm sha256 {}…) is made from", &h[..h.len().min(12)]),
        None => base,
    }
}

/// The report as JSON, for `--json`.
pub fn to_json(r: &Report) -> Value {
    serde_json::to_value(r).unwrap_or_else(|e| json!({"error": e.to_string()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/acme-local")
    }

    fn manifest0() -> Manifest {
        Manifest::parse(&std::fs::read(fixture().join("manifest.json")).unwrap()).unwrap()
    }

    fn tmp(test: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("antibody-immune-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Creation code that returns whatever follows it: an artifact for given runtime code, with no constructor.
    const RETURN_REST: &str = "600d380380600d6000396000f3";

    fn artifact(dir: &Path, file: &str, contract: &str, creation_hex: &str, abi: Value) -> PathBuf {
        let d = dir.join(file);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("{contract}.json"));
        let a = json!({"abi": abi, "bytecode": {"object": format!("0x{creation_hex}"), "linkReferences": {}}, "deployedBytecode": {"object": "0x"}});
        std::fs::write(&p, serde_json::to_vec(&a).unwrap()).unwrap();
        p
    }

    fn s(v: &str) -> toml::Value {
        toml::Value::String(v.into())
    }

    #[test]
    fn constructor_arguments_encode_as_the_abi_does() {
        let m = manifest0();
        let types: Vec<String> = ["address", "address", "uint256", "int8", "bool", "bytes4", "uint8"].iter().map(|t| t.to_string()).collect();
        let vals = vec![
            s("AcmeGuard"),
            s("0x00000000000000000000000000000000000000aa"),
            s("2000000000000000000000"),
            toml::Value::Integer(-2),
            toml::Value::Boolean(true),
            s("0x8d4b5cd2"),
            s("0xff"),
        ];
        let out = encode_args(&types, &vals, &m).unwrap();
        assert_eq!(out.len(), 32 * 7);
        let guard = m.targets.iter().find(|t| t.name == "AcmeGuard").unwrap().address;
        assert_eq!(&out[12..32], guard.as_slice(), "a target's name is its address");
        assert_eq!(out[63], 0xaa);
        assert_eq!(U256::from_be_slice(&out[64..96]), U256::from(2000u64) * U256::from(10u64).pow(U256::from(18u64)));
        assert_eq!(&out[96..128], &[0xffu8; 31].iter().chain([0xfeu8].iter()).copied().collect::<Vec<_>>()[..], "two's complement");
        assert_eq!(out[159], 1);
        assert_eq!(&out[160..164], &[0x8d, 0x4b, 0x5c, 0xd2], "bytesN is left-aligned");
        assert_eq!(out[223], 0xff);
        assert!(encode_args(&types[..1], &[], &m).unwrap_err().to_string().contains("takes 1 arguments"));
        assert!(encode_args(&["address".into()], &[s("Nobody")], &m).unwrap_err().to_string().contains("neither an address nor a target"));
        assert!(encode_args(&["uint8".into()], &[s("256")], &m).unwrap_err().to_string().contains("does not fit"));
        assert!(encode_args(&["int8".into()], &[toml::Value::Integer(-129)], &m).unwrap_err().to_string().contains("does not fit"));
        assert!(encode_args(&["string".into()], &[s("x")], &m).unwrap_err().to_string().contains("args_hex"));
    }

    #[test]
    fn creation_code_runs_at_the_targets_address_and_its_arguments_reach_it() {
        // Returns its last 32 bytes, the constructor's argument (an immutable, in effect).
        let echo = "60206020380360003960206000f3";
        let at: Address = "0x00000000000000000000000000000000000000c0".parse().unwrap();
        let mut creation = hex::decode(echo).unwrap();
        creation.extend(U256::from(5u8).to_be_bytes::<32>());
        assert_eq!(runtime_of(&creation, at, "osaka", 31337, 29).unwrap().as_ref(), U256::from(5u8).to_be_bytes::<32>());
        // Returns ADDRESS: `address(this)` is the target's, as an immutable recording an implementation's address needs.
        let this = hex::decode("3060005260206000f3").unwrap();
        assert_eq!(&runtime_of(&this, at, "osaka", 31337, 29).unwrap()[12..], at.as_slice());
        // A constructor that reverts is reported, not taken as empty code.
        assert!(runtime_of(&hex::decode("60006000fd").unwrap(), at, "osaka", 1, 1).unwrap_err().to_string().contains("reverted"));
    }

    #[test]
    fn the_mapping_finds_artifacts_and_derives_each_targets_code() {
        let dir = tmp("mapping");
        let m = manifest0();
        let lend_impl = m.targets.iter().find(|t| t.name == "AcmeLendImpl").unwrap().address;
        // Echoes its argument: the runtime code is the 32-byte word given for `x`.
        let pair = json!({"type": "tuple", "components": [{"type": "uint8"}, {"type": "address"}]});
        let abi = json!([{"type": "constructor", "inputs": [{"name": "x", "type": "uint256"}]},
                         {"type": "function", "name": "sweepFees", "inputs": [{"name": "to", "type": "address"}]},
                         {"type": "function", "name": "f", "inputs": [pair, {"type": "bytes32[]"}]}]);
        artifact(&dir.join("out"), "Echo.sol", "Echo", "60206020380360003960206000f3", abi);
        artifact(&dir.join("out"), "Twin.sol", "Twin", RETURN_REST, json!([]));
        artifact(&dir.join("out"), "Other.sol", "Twin", RETURN_REST, json!([]));
        artifact(&dir.join("out"), "Multi.sol", "Multi.0.8.28", RETURN_REST, json!([]));
        std::fs::create_dir_all(dir.join("out/build-info")).unwrap();
        std::fs::write(dir.join("out/build-info/Echo.json"), b"{}").unwrap();
        assert!(find_artifact(&dir.join("out"), "Echo").unwrap().ends_with("Echo.sol/Echo.json"), "build info is skipped");
        assert!(find_artifact(&dir.join("out"), "Multi").unwrap().ends_with("Multi.0.8.28.json"), "a compiler version suffix");
        assert!(find_artifact(&dir.join("out"), "Twin").unwrap_err().to_string().contains("several artifacts"));
        assert!(find_artifact(&dir.join("out"), "Nope").unwrap_err().to_string().contains("no artifact"));
        let a = read_artifact(&find_artifact(&dir.join("out"), "Echo").unwrap()).unwrap();
        assert_eq!(a.constructor, vec!["uint256".to_string()]);
        assert_eq!(a.selectors.get("0x8d4b5cd2").map(String::as_str), Some("sweepFees(address)"));
        assert!(a.selectors.values().any(|s| s == "f((uint8,address),bytes32[])"), "tuples in canonical form");

        std::fs::write(dir.join("fixed.hex"), "0x6001600055\n").unwrap();
        let cfg = Config {
            contracts: vec![
                Mapping { target: "AcmeLendImpl".into(), contract: Some("Echo".into()), args: vec![s("7")], ..Default::default() },
                Mapping { target: "0x585c29EF94fD2447eE9c3Bf8b629Ed953f4D956F".into(), runtime: Some("fixed.hex".into()), ..Default::default() },
            ],
            ..Default::default()
        };
        let built = build(&cfg, &dir, &m).unwrap();
        assert_eq!((built[0].target.as_str(), built[0].address), ("AcmeLendImpl", lend_impl));
        assert_eq!(built[0].runtime.as_ref(), U256::from(7u8).to_be_bytes::<32>(), "the creation code ran with the mapping's argument");
        assert!(built[0].from.contains("Echo.sol"));
        assert_eq!((built[1].target.as_str(), built[1].runtime.as_ref()), ("AcmeVaultImpl", &[0x60, 0x01, 0x60, 0x00, 0x55][..]), "a target by address");

        // Mapped twice, unknown targets, both argument forms, and linked libraries are refused.
        let twice = Config { contracts: vec![cfg.contracts[1].clone(), cfg.contracts[1].clone()], ..Default::default() };
        assert!(build(&twice, &dir, &m).unwrap_err().to_string().contains("mapped twice"));
        let unknown =
            Config { contracts: vec![Mapping { target: "Nope".into(), runtime: Some("fixed.hex".into()), ..Default::default() }], ..Default::default() };
        assert!(build(&unknown, &dir, &m).unwrap_err().to_string().contains("not one of the program's targets"));
        let both = Config { contracts: vec![Mapping { args_hex: Some("00".into()), ..cfg.contracts[0].clone() }], ..Default::default() };
        assert!(build(&both, &dir, &m).unwrap_err().to_string().contains("not both"));
        let linked = dir.join("linked.json");
        std::fs::write(&linked, serde_json::to_vec(&json!({"abi": [], "bytecode": {"object": "0x00", "linkReferences": {"L.sol": {}}}})).unwrap()).unwrap();
        assert!(read_artifact(&linked).unwrap_err().to_string().contains("links libraries"));

        // The config file's own syntax.
        let toml = r#"
program = 5
blobs = "proofs"
[content_keys]
13 = "keys/13.hex"
[[contract]]
contract = "AcmeLend"
target = "AcmeLendImpl"
args = ["AcmeGuard", 2000]
"#;
        std::fs::write(dir.join("immune.toml"), toml).unwrap();
        let c = Config::load(&dir.join("immune.toml")).unwrap();
        assert_eq!((c.program, c.artifacts.clone(), c.contracts.len(), c.content_keys["13"].as_str()), (Some(5), PathBuf::from("out"), 1, "keys/13.hex"));
        std::fs::write(dir.join("bad.toml"), "program = 5\nprogramme = 6\n").unwrap();
        assert!(Config::load(&dir.join("bad.toml")).is_err(), "unknown fields are refused");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pull_requests_code_replaces_only_what_it_builds_under_the_current_rules() {
        let m0 = manifest0();
        let r2 = Manifest::parse(&std::fs::read(fixture().join("round2/manifest.json")).unwrap()).unwrap();
        let t = |n: &str| m0.targets.iter().find(|t| t.name == n).unwrap().clone();
        let (lend, vault) = (t("AcmeLendImpl"), t("AcmeVaultImpl"));
        let new_vault = Bytes::from(vec![0x60, 0x00]);
        let built = vec![Built {
            target: vault.name.clone(),
            address: vault.address,
            from: "AcmeVault".into(),
            code_hash: keccak256(&new_vault),
            runtime: new_vault.clone(),
            selectors: BTreeMap::new(),
        }];
        let (m, rows) = pr_manifest(&m0, &r2, &built).unwrap();
        assert_eq!(m.round, 3, "the next round after the current one");
        let code_of = |a: Address| m.overrides.iter().find(|o| o.address == a).map(|o| o.runtime_code.clone());
        assert_eq!(code_of(vault.address), Some(new_vault), "the pull request's code where it builds a target");
        assert_eq!(code_of(lend.address), r2.overrides.iter().find(|o| o.address == lend.address).map(|o| o.runtime_code.clone()), "round 2's fix elsewhere");
        assert_eq!(m.invariants, r2.invariants, "the current round's rules");
        let row = |n: &str| rows.iter().find(|r| r.target == n).unwrap().code.clone();
        assert_eq!((row("AcmeVaultImpl"), row("AcmeLendImpl"), row("AcmeGuard")), ("pull request".into(), "round 2".into(), "anchor".into()));
        // A build identical to the anchor's code is no override; with nothing to override the manifest is round 0's.
        let same = vec![Built { code_hash: vault.code_hash, ..built[0].clone() }];
        let (m, rows) = pr_manifest(&m0, &m0, &same).unwrap();
        assert!(m.overrides.is_empty() && m.round == 0);
        assert!(rows.iter().any(|r| r.code.contains("identical")));
        Manifest::parse(&m.canonical()).unwrap();
    }

    #[test]
    fn disclosure_keys_published_keys_and_the_project_key_open_content_keys() {
        let (dk, k, key) = ([3u8; 32], [9u8; 32], "bounty.antibody.testnet/5");
        let nonce = [1u8; 12];
        let wrap_d = hex::encode([nonce.as_slice(), sealed::aead_seal_pub(&dk, &nonce, &k, key.as_bytes()).as_slice()].concat());
        assert_eq!(open_wrap_d(&wrap_d, &dk, key), Some(k));
        assert_eq!(open_wrap_d(&wrap_d, &[4u8; 32], key), None, "another tag's key");
        assert_eq!(open_wrap_d(&wrap_d, &dk, "bounty.antibody.testnet/6"), None, "another program");
        let p = json!({"disclosed": [[0, hex::encode([7u8; 32]), hex::encode(dk)], [1, "zz", "zz"]]});
        assert_eq!(disclosed_keys(&p), vec![(0, dk)]);
        // A release wrap opens with the project's secret only.
        let sk = [5u8; 32];
        let wrap = sealed::project_wrap(&k, &sealed::x25519_public(sk), key);
        let run = json!({"outcome": {"Answered": {"facts": {"extra": {"Wrap": {"wrap": hex::encode(wrap)}}}}}});
        assert_eq!(unwrap_release(&run, &sk, key), Ok(k));
        assert!(unwrap_release(&run, &[6u8; 32], key).unwrap_err().contains("does not open"));
        assert!(unwrap_release(&json!({}), &sk, key).unwrap_err().contains("no wrap"));
        let dir = tmp("keys");
        std::fs::write(dir.join("13.hex"), format!("{}\n", hex::encode(k))).unwrap();
        assert_eq!(key32("13.hex", &dir).unwrap(), k);
        assert_eq!(key32(&format!("0x{}", hex::encode(k)), &dir).unwrap(), k);
        assert_eq!(project_secret(&json!({"type": "x25519", "secret_key": hex::encode(k)}).to_string()).unwrap(), k);
        assert_eq!(project_secret(&hex::encode(k)).unwrap(), k);
        assert!(project_secret("nope").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn row(commit: u64, outcome: Outcome) -> ProofRow {
        ProofRow {
            commit,
            round: 0,
            hunter: format!("agent-{commit}.testnet"),
            state: "Done:Paid".into(),
            paid: "0.03 wNEAR".into(),
            paid_for: vec!["fees_to_treasury (high)".into()],
            read_through: (outcome != Outcome::Sealed).then(|| "disclosure".into()),
            outcome,
            broken: if outcome == Outcome::StillBreaks {
                vec![Broken {
                    slot: Some(1),
                    rule: "fees_to_treasury".into(),
                    severity: "high".into(),
                    entry_point: "AcmeVault.sweepFees(address)".into(),
                    target: Address::ZERO,
                    selector: "0x8d4b5cd2".into(),
                }]
            } else {
                vec![]
            },
            reason: match outcome {
                Outcome::Inconclusive => Some("the code under test reads storage slot 0x7 of AcmeVault, which this proof's witness does not prove".into()),
                Outcome::Sealed => Some("sealed: not disclosed".into()),
                _ => None,
            },
            receipt_matches_the_tee: (outcome != Outcome::Sealed).then_some(true),
        }
    }

    fn report(rows: Vec<ProofRow>, strict: bool) -> Report {
        Report {
            schema: REPORT_SCHEMA,
            network: "testnet".into(),
            contract: "bounty.antibody.testnet".into(),
            program: 5,
            program_key: "bounty.antibody.testnet/5".into(),
            name: "Acme Lend | test".into(),
            chain_id: 11155111,
            anchor_block: 12063269,
            rules_round: 2,
            build: vec![TargetCode {
                target: "AcmeVaultImpl".into(),
                address: Address::ZERO,
                code: "pull request".into(),
                from: Some("AcmeVault (out/AcmeVault.sol/AcmeVault.json)".into()),
                code_hash: B256::ZERO,
            }],
            manifest_sha256: "ab".repeat(32),
            proofs: rows,
            summary: Summary::default(),
            strict,
            exit_code: -1,
            verifier: verifier_line(Some("0.1.0"), Some("ccc27f0ca60c503c")),
            transactions_sent: 0,
        }
        .finish()
    }

    #[test]
    fn the_exit_code_fails_on_a_break_and_never_counts_an_inconclusive_run_as_fixed() {
        use Outcome::*;
        assert_eq!(report(vec![row(13, Fixed)], false).exit_code, EXIT_OK);
        assert_eq!(report(vec![row(13, Fixed), row(14, StillBreaks), row(18, Inconclusive)], false).exit_code, EXIT_STILL_BREAKS);
        assert_eq!(report(vec![row(13, Fixed), row(18, Inconclusive)], false).exit_code, EXIT_INCONCLUSIVE);
        assert_eq!(report(vec![row(13, Fixed), row(14, Sealed)], false).exit_code, EXIT_OK, "a sealed proof is listed, not counted");
        assert_eq!(report(vec![row(13, Fixed), row(14, Sealed)], true).exit_code, EXIT_INCONCLUSIVE, "unless --strict");
        assert_eq!(report(vec![], true).exit_code, EXIT_OK);
        let r = report(vec![row(13, Fixed), row(14, StillBreaks), row(18, Inconclusive), row(19, Sealed)], false);
        assert_eq!((r.summary.paid, r.summary.rerun, r.summary.fixed, r.summary.still_breaks, r.summary.inconclusive, r.summary.sealed), (4, 3, 1, 1, 1, 1));
        let j = to_json(&r);
        assert_eq!(j["proofs"][1]["outcome"], "still_breaks");
        assert_eq!(j["proofs"][1]["broken"][0]["entry_point"], "AcmeVault.sweepFees(address)");
        assert_eq!(j["exit_code"], 2);
    }

    #[test]
    fn the_markdown_names_the_proof_that_breaks_again_and_why_others_are_not_fixed() {
        use Outcome::*;
        let md = markdown(&report(vec![row(14, StillBreaks), row(13, Fixed), row(18, Inconclusive), row(19, Sealed)], false));
        assert!(md.starts_with("### Antibody immune memory: 1 paid proof still breaks a rule\n"), "{md}");
        let breaks = concat!(
            "| **still breaks** | 14 by agent-14.testnet (round 0, Done:Paid) | fees_to_treasury (high), 0.03 wNEAR | disclosure ",
            "| fees_to_treasury (high) | `AcmeVault.sweepFees(address)` |"
        );
        assert!(md.contains(breaks), "{md}");
        assert!(md.contains("| fixed | 13 by agent-13.testnet"));
        assert!(md.contains("- Commit 18, inconclusive: the code under test reads storage slot 0x7 of AcmeVault"));
        assert!(md.contains("| not re-run (sealed) | 19 by"));
        assert!(md.contains("*Acme Lend \\| test*"), "cells are escaped");
        assert!(md.contains("| AcmeVaultImpl |") && md.contains("Exit code 2."));
        let ok = markdown(&report(vec![row(13, Fixed)], false));
        assert!(ok.starts_with("### Antibody immune memory: no paid proof breaks a rule (1 re-run)"));
        let none = markdown(&report(vec![row(14, Sealed)], false));
        assert!(none.starts_with("### Antibody immune memory: nothing re-run: no paid proof is readable here"));
        let inc = markdown(&report(vec![row(18, Inconclusive)], false));
        assert!(inc.starts_with("### Antibody immune memory: inconclusive: no paid proof breaks a rule, but 1 re-run could not be judged"));
        // A proof read with the project key is still sealed: its rule shows (its commit's key names its slot, public),
        // not where it broke.
        let mut sealed = vec![row(14, StillBreaks), row(18, Inconclusive)];
        sealed.iter_mut().for_each(|r| withhold(r, &[1]));
        let md = markdown(&report(sealed, false));
        assert!(md.contains("| fees_to_treasury (high) | `withheld (a sealed proof)` |") && !md.contains("sweepFees") && !md.contains("slot 0x7"), "{md}");
    }

    /// A sealed proof read with the project key names only the rules its commit's keys on chain name. A rule it breaks
    /// now that no key named (the code changed, or a later round added the rule) is another rule, with no slot, after
    /// the named ones: the verifier lists rules by slot, so its place would tell its slot.
    #[test]
    fn a_sealed_proof_names_only_the_rules_its_commits_keys_name() {
        let broken = |slot: u8, rule: &str, severity: &str| Broken {
            slot: Some(slot),
            rule: rule.into(),
            severity: severity.into(),
            entry_point: format!("Hidden.entry{slot}()"),
            target: "0x00000000000000000000000000000000000000aa".parse().unwrap(),
            selector: format!("0xdeadbe0{slot}"),
        };
        let mut r = row(14, Outcome::StillBreaks);
        r.read_through = Some("project key".into());
        // The commit's keys name slot 1 (fees_to_treasury); the re-run also breaks slots 0 and 2, in slot order.
        r.broken = vec![broken(0, "solvency", "critical"), broken(1, "fees_to_treasury", "high"), broken(2, "oracle_bounds", "medium")];
        withhold(&mut r, &[1]);
        let named = &r.broken[0];
        assert_eq!((named.slot, named.rule.as_str(), named.severity.as_str()), (Some(1), "fees_to_treasury", "high"));
        for b in &r.broken[1..] {
            assert_eq!((b.slot, b.rule.as_str(), b.severity.as_str()), (None, "another rule", "withheld"));
        }
        assert!(r.broken.iter().all(|b| b.entry_point == "withheld (a sealed proof)" && b.target == Address::ZERO && b.selector.is_empty()));

        let rep = report(vec![r], false);
        let md = markdown(&rep);
        assert!(md.contains("| project key | fees_to_treasury (high), another rule (withheld), another rule (withheld) |"), "{md}");
        let j = to_json(&rep);
        assert!(j["proofs"][0]["broken"][1]["slot"].is_null() && j["proofs"][0]["broken"][2]["slot"].is_null(), "{j}");
        assert_eq!(j["proofs"][0]["broken"][0]["slot"], 1);
        let text = j.to_string();
        for hidden in ["solvency", "critical", "oracle_bounds", "medium", "Hidden.entry", "deadbe"] {
            assert!(!md.contains(hidden) && !text.contains(hidden), "{hidden} is shown:\n{md}\n{text}");
        }

        // A gap claim's commit has no key: no rule it breaks is named.
        let mut gap = row(19, Outcome::StillBreaks);
        gap.paid_for = vec![];
        withhold(&mut gap, &[]);
        assert_eq!((gap.broken[0].slot, gap.broken[0].rule.as_str(), gap.broken[0].severity.as_str()), (None, "another rule", "withheld"));
        assert!(!markdown(&report(vec![gap], false)).contains("fees_to_treasury"));
    }

    /// The fixture's anchor code of a target, from a witness's bytecodes.
    fn anchor_code(w: &ProgramWitness, code_hash: B256) -> Bytes {
        w.bytecodes.iter().find(|c| c.hash_slow() == code_hash).map(|c| c.original_bytes()).expect("the witness holds the target's code")
    }

    fn program_witness(agent: &str) -> (Envelope, ProgramWitness) {
        let env = Envelope::decode(&std::fs::read(fixture().join(format!("submissions/{agent}.witness.bin"))).unwrap()).unwrap();
        let Payload::Program(w) = env.payload.clone() else { panic!("a full program witness") };
        (env, w)
    }

    /// End to end on Acme Lend's recorded fixture (fixtures/acme-local, bug B: `sweepFees` without access
    /// control, paid in round 0): the same proof still breaks `fees_to_treasury` on the vault as deployed, and
    /// breaks no rule on `AcmeVaultFixed`, each build given as a Foundry artifact through the mapping. A proof
    /// whose witness lacks what the code reads is inconclusive, naming the missing slot.
    #[test]
    fn a_paid_proof_still_breaks_on_the_buggy_code_and_is_fixed_on_its_fix() {
        let key = "antibody.testnet/acme-local";
        let m0 = manifest0();
        let vault_impl = m0.targets.iter().find(|t| t.name == "AcmeVaultImpl").unwrap().clone();
        let (inner, w) = program_witness("B");
        let dir = tmp("e2e");
        let abi = json!([{"type": "function", "name": "sweepFees", "inputs": [{"name": "to", "type": "address"}]}]);
        let fixed = std::fs::read_to_string(fixture().join("round2/AcmeVaultFixed.runtime.hex")).unwrap();
        artifact(&dir.join("out"), "AcmeVault.sol", "AcmeVault", &format!("{RETURN_REST}{}", hex::encode(anchor_code(&w, vault_impl.code_hash))), abi.clone());
        artifact(&dir.join("fixed"), "AcmeFinalFix.sol", "AcmeVaultFixed", &format!("{RETURN_REST}{}", fixed.trim().trim_start_matches("0x")), abi);
        let run = |artifacts: &str, contract: &str| {
            let cfg = Config {
                artifacts: artifacts.into(),
                contracts: vec![Mapping { target: "AcmeVaultImpl".into(), contract: Some(contract.into()), ..Default::default() }],
                ..Default::default()
            };
            let built = build(&cfg, &dir, &m0).unwrap();
            let (m, rows) = pr_manifest(&m0, &m0, &built).unwrap();
            let selectors: BTreeMap<String, String> = built.iter().flat_map(|b| b.selectors.clone()).collect();
            let (env, w) = assemble(&inner, None, &m.canonical().into(), true).unwrap();
            (rerun(&env, &w, key, &selectors), rows)
        };

        let (buggy, rows) = run("out", "AcmeVault");
        assert!(rows.iter().any(|r| r.target == "AcmeVaultImpl" && r.code.contains("identical")), "the buggy build is the anchor's code");
        assert_eq!(buggy.outcome, Outcome::StillBreaks, "{:?}", buggy.reason);
        assert_eq!(buggy.broken.len(), 1);
        let b = &buggy.broken[0];
        assert_eq!((b.slot, b.rule.as_str(), b.severity.as_str(), b.entry_point.as_str()), (Some(1), "fees_to_treasury", "high", "AcmeVault.sweepFees(address)"));

        let (fixed, rows) = run("fixed", "AcmeVaultFixed");
        assert!(rows.iter().any(|r| r.target == "AcmeVaultImpl" && r.code == "pull request"));
        assert_eq!(fixed.outcome, Outcome::Fixed, "{:?}", fixed.reason);

        // The same proof assembled as the TEE ran it in round 0 gives the fixture's recorded receipt hash, the check
        // a live run makes against the TEE's before it counts a proof as fixed.
        let expected: Value = serde_json::from_slice(&std::fs::read(fixture().join("expected.json")).unwrap()).unwrap();
        let (env, w0) = assemble(&inner, None, &w.manifest_json, false).unwrap();
        let pctx = ProgramContext {
            program_id: key.into(),
            commit_id: 2,
            expected_manifest_hash: Some(Manifest::hash(&w.manifest_json)),
            reserved: Some(reserved_address(key, env.claimed_block_hash)),
        };
        let receipt = run_program(&env, &w0, sha256(&std::fs::read(fixture().join("submissions/B.witness.bin")).unwrap()), &pctx);
        assert_eq!(json!(hex::encode(receipt.hash())), expected["submissions"]["B"]["receipt_hash"]);

        // A witness that does not prove what the code reads (here acUSD's storage is taken out, as if the code under
        // test read storage the proof never touched): the re-run is inconclusive, never fixed, and names it.
        let acusd = m0.targets.iter().find(|t| t.name == "acUSD").unwrap().address;
        let mut thin = inner.clone();
        let Payload::Program(ref mut tw) = thin.payload else { unreachable!() };
        assert!(tw.state.storage_tries.remove(&keccak256(acusd)).is_some());
        let (env, wt) = assemble(&thin, None, &m0.canonical().into(), true).unwrap();
        let r = rerun(&env, &wt, key, &BTreeMap::new());
        assert_eq!(r.outcome, Outcome::Inconclusive, "{r:?}");
        let reason = r.reason.unwrap_or_default();
        assert!(reason.contains(&format!("the storage of acUSD ({acusd})")) && reason.contains("does not prove"), "{reason}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

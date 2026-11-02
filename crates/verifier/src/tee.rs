//! OutLayer entry point (docs/spec.md §5). near-antibody sends a request v2 (JSON) as `input_data`; the
//! verifier answers with facts v2 on stdout. Every input gives typed facts:
//! - a failure the hunter can compute locally is `InvalidPoC`;
//! - a failure that is nobody's fault (a blob no RPC served, bytes that do not match their hash, a
//!   caller that is not the program's contract) is `InfraError`, which is retried and never forfeited.
//!
//! Modes: `activate`, `verify`, `second`, `rerun`, `check_round`, `distinct`, `release_wrap`,
//! `disclose` (spec §5.8). Keys come from [`crate::keys`]; nothing secret is ever printed except, at
//! disclosure, the disclosure keys of tags whose condition near-antibody has checked.

use alloy_primitives::{Address, Bytes, B256};
use serde::{Deserialize, Serialize};

use crate::{
    facts::{fp_tag_v2_group, sha256, Kind, ProgramReceipt},
    keys::{Mac, ProgramKeys},
    manifest::Manifest,
    run::{reserved_address, run_program, ProgramContext},
    sealed, BaseWitness, Envelope, Payload, ProgramWitness,
};

pub const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
/// The commit id a known issue's receipt names: known issues are declared by the project before open
/// and have no commit.
pub const KNOWN_ISSUE_COMMIT: u64 = u64::MAX;
/// Inline blobs travel in near-antibody's request (spec §4.2).
pub const MAX_INLINE_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Activate,
    Verify,
    Second,
    Rerun,
    CheckRound,
    Distinct,
    ReleaseWrap,
    Disclose,
}

/// Where a blob is: inline (hex), or a FastFS upload (`near-tx:<hash>:<signer>`) with its sha256.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BlobRef {
    Inline { inline: String },
    NearTx { near_tx: String, sha256: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Earlier {
    pub commit_id: u64,
    pub hunter: String,
    pub blob: BlobRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub v: u8,
    pub mode: Mode,
    pub program_key: String,
    pub manifest_sha256: String,
    pub round: u32,
    #[serde(default)]
    pub key_epoch: String,
    #[serde(default)]
    pub commit_id: Option<u64>,
    pub run_id: u64,
    #[serde(default)]
    pub hunter: Option<String>,
    #[serde(default)]
    pub blob: Option<BlobRef>,
    pub base: BlobRef,
    #[serde(default)]
    pub round_manifest: Option<BlobRef>,
    #[serde(default)]
    pub project_pk: Option<String>,
    #[serde(default)]
    pub earlier: Option<Earlier>,
    #[serde(default)]
    pub patch: Option<BlobRef>,
    #[serde(default)]
    pub slot: Option<u8>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// `check_round`: the releases the round proposes (spec §9).
    #[serde(default)]
    pub releases: Vec<ReleaseIn>,
    /// `check_round`: the slots the round adds, as near-antibody will price them (spec §8.1).
    #[serde(default)]
    pub new_slots: Vec<NewSlotIn>,
    /// `check_round` after round 1: the open round's manifest, which the proposal may only extend.
    #[serde(default)]
    pub prev_round_manifest: Option<BlobRef>,
}

/// A slot a round adds: its bit and severity must be the round manifest's.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewSlotIn {
    pub slot: u8,
    pub severity: String,
}

/// A proposed release: the proxy to upgrade and the code hash its new implementation must have.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseIn {
    pub proxy: String,
    pub code_hash: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KeyOut {
    pub slot: u8,
    pub tag: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RoundCheck {
    pub ok: bool,
    pub canaries_ok: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Disclosed {
    pub tag: String,
    pub key: String,
}

/// Facts v2 (spec §5.2): what `crates/rules` parses. Fields a mode does not use are omitted.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FactsOut {
    pub v: u8,
    pub mode: Mode,
    pub request_sha256: String,
    pub manifest_sha256: String,
    pub program_key: String,
    pub round: u32,
    pub key_epoch: String,
    pub commit_id: Option<u64>,
    pub run_id: u64,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<KeyOut>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub impact_usd_micro: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub protected_harm_usd_micro: String,
    pub receipt_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sealing_pk: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round_check: Option<RoundCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrap_d: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub disclosed: Vec<Disclosed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distinct: Option<bool>,
}

/// What the TEE needs from its host: the key, the blob fetcher, and the caller.
pub struct Env<'a> {
    pub mac: &'a dyn Mac,
    /// Fetches a `near-tx:` location from at least two RPCs (main.rs). Errors are infrastructure errors.
    pub fetch: &'a dyn Fn(&str) -> Result<Vec<u8>, String>,
    /// `NEAR_PREDECESSOR_ID`: the account that called OutLayer. It must be the program's contract.
    pub predecessor: Option<String>,
}

/// The full output of a run: the facts, and the receipt they summarise (kept off chain).
pub struct Output {
    pub facts: FactsOut,
    pub receipt: Option<ProgramReceipt>,
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    let b = hex::decode(s.trim_start_matches("0x")).ok()?;
    b.try_into().ok()
}

/// A round's proposed releases as (proxy, code hash); `None` if one is malformed.
fn releases_of(req: &Request) -> Option<Vec<(Address, B256)>> {
    req.releases.iter().map(|r| Some((r.proxy.parse::<Address>().ok()?, B256::from(hex32(&r.code_hash)?)))).collect()
}

/// Why a run produced no verdict: the facts kind to report and a short reason.
struct Stop(Kind, String);

fn infra(r: impl Into<String>) -> Stop {
    Stop(Kind::InfraError, r.into())
}
fn invalid(r: impl Into<String>) -> Stop {
    Stop(Kind::InvalidPoC, r.into())
}

fn short(r: &str) -> String {
    r.chars().take(64).collect()
}

/// Reads a blob. Inline bytes are bound by near-antibody's own hash; a fetched blob must match its
/// committed hash, and a mismatch is the RPC's fault (spec §5.3).
fn read_blob(env: &Env, b: &BlobRef) -> Result<Vec<u8>, Stop> {
    match b {
        BlobRef::Inline { inline } => {
            let bytes = hex::decode(inline).map_err(|_| infra("inline blob is not hex"))?;
            if bytes.len() > MAX_INLINE_BYTES {
                return Err(infra("inline blob above 32 KB"));
            }
            Ok(bytes)
        }
        BlobRef::NearTx { near_tx, sha256: want } => {
            let want = hex32(want).ok_or_else(|| infra("bad blob hash"))?;
            let bytes = (env.fetch)(near_tx).map_err(|e| infra(format!("fetch: {}", short(&e))))?;
            if bytes.len() > MAX_BLOB_BYTES {
                return Err(infra("blob too large"));
            }
            if sha256(&bytes) != want {
                return Err(infra("fetched bytes do not match their committed hash"));
            }
            Ok(bytes)
        }
    }
}

fn base_witness(env: &Env, req: &Request) -> Result<(Envelope, BaseWitness), Stop> {
    let bytes = read_blob(env, &req.base)?;
    let e = Envelope::decode(&bytes).map_err(|e| Stop(Kind::ProgramMisconfigured, format!("base witness: {}", short(&e))))?;
    match e.payload.clone() {
        Payload::Base(b) => Ok((e, b)),
        _ => Err(Stop(Kind::ProgramMisconfigured, "the base witness is not a base witness".into())),
    }
}

/// The manifest the run is judged under, checked against the request's hash (never the witness's).
fn manifest_for(env: &Env, req: &Request, base: &BaseWitness) -> Result<Bytes, Stop> {
    let want = hex32(&req.manifest_sha256).ok_or_else(|| infra("bad manifest hash"))?;
    let bytes: Bytes = match (&req.round_manifest, req.round) {
        (None, 0) => base.manifest_json.clone(),
        (Some(r), _) => read_blob(env, r)?.into(),
        (None, _) => return Err(infra("a later round needs its manifest")),
    };
    if sha256(&bytes) != want {
        return Err(infra("manifest bytes do not match the committed hash"));
    }
    Ok(bytes)
}

/// Opens a sealed blob for this program and hunter; returns the content key and the inner envelope.
fn open(keys: &ProgramKeys, program_key: &str, hunter: &str, blob: &[u8]) -> Result<([u8; 32], Envelope, [u8; 32]), Stop> {
    let sk = keys.seal_secret().map_err(infra)?;
    let (k, payload) = sealed::open(blob, &sk, program_key).map_err(|e| invalid(format!("cannot open the sealed blob: {e}")))?;
    if payload.program_key != program_key || payload.hunter != hunter {
        return Err(invalid("the sealed payload is bound to another program or hunter"));
    }
    // The report text is never parsed: only the code in the envelope is executed.
    let witness_sha = sha256(&payload.envelope);
    let envelope = Envelope::decode(&payload.envelope).map_err(|e| invalid(format!("witness: {}", short(&e))))?;
    Ok((k, envelope, witness_sha))
}

/// The witness a PoC runs on: a full program witness, or a submission merged with the base.
fn program_witness(env_in: Envelope, base: &BaseWitness, manifest: Bytes) -> Result<(Envelope, ProgramWitness), Stop> {
    match env_in.payload.clone() {
        Payload::Program(mut w) => {
            w.manifest_json = manifest;
            Ok((env_in, w))
        }
        Payload::Submission(sub) => {
            let w = ProgramWitness::assemble(base, &sub, manifest).map_err(|e| invalid(format!("witness: {}", short(&e))))?;
            let mut e = env_in;
            e.payload = Payload::Program(w.clone());
            Ok((e, w))
        }
        _ => Err(invalid("the sealed witness is not a program witness")),
    }
}

struct Verdict {
    receipt: ProgramReceipt,
    keys: Vec<KeyOut>,
    primary: Option<[u8; 32]>,
}

/// The keys of a receipt: tags under the program's key, by committed group (spec §7.2), and the primary
/// (highest-severity) tag.
pub fn receipt_keys(keys: &ProgramKeys, receipt: &ProgramReceipt, manifest_json: &[u8]) -> Result<(Vec<KeyOut>, Option<[u8; 32]>), String> {
    let manifest = Manifest::parse(manifest_json).ok();
    let mut out = vec![];
    let mut primary: Option<([u8; 32], u8)> = None;
    if receipt.kind == Kind::Violation {
        for f in receipt.broken.iter().take(crate::facts::MAX_KEYS) {
            let sel = crate::facts::parse_selector(&f.selector);
            let group = manifest.as_ref().and_then(|m| m.group_of(f.target, sel));
            let t = fp_tag_v2_group(keys, f.slot, group, f.target, sel)?;
            let sev = f.severity as u8;
            if primary.map(|(_, s)| sev > s).unwrap_or(true) {
                primary = Some((t, sev));
            }
            out.push(KeyOut { slot: f.slot, tag: hex::encode(t) });
        }
    }
    Ok((out, primary.map(|(t, _)| t)))
}

/// Facts for a local run (no request): the verdict, keys and impact, under the TEST key.
pub fn local_facts(keys: &ProgramKeys, receipt: &ProgramReceipt, manifest_json: &[u8], program_key: &str, commit_id: u64) -> FactsOut {
    let (k, _) = receipt_keys(keys, receipt, manifest_json).unwrap_or_default();
    FactsOut {
        v: 2,
        mode: Mode::Verify,
        request_sha256: String::new(),
        manifest_sha256: hex::encode(receipt.manifest_hash),
        program_key: program_key.to_string(),
        round: 0,
        key_epoch: keys.key_epoch().unwrap_or_default(),
        commit_id: Some(commit_id),
        run_id: 0,
        kind: receipt.kind,
        keys: k,
        impact_usd_micro: receipt.impact.as_ref().map(|i| i.impact_usd_micro.clone()).unwrap_or_default(),
        protected_harm_usd_micro: receipt.impact.as_ref().map(|i| i.protected_harm_usd_micro.clone()).filter(|h| h != "0").unwrap_or_default(),
        receipt_sha256: hex::encode(receipt.hash()),
        reason: receipt.reason.clone(),
        sealing_pk: None,
        round_check: None,
        wrap: None,
        wrap_d: None,
        disclosed: vec![],
        distinct: None,
    }
}

/// Runs a PoC and computes its keys.
fn judge(keys: &ProgramKeys, ctx: &ProgramContext, envelope: &Envelope, w: &ProgramWitness, witness_sha: [u8; 32]) -> Result<Verdict, Stop> {
    let receipt = run_program(envelope, w, witness_sha, ctx);
    let (out, primary) = receipt_keys(keys, &receipt, &w.manifest_json).map_err(infra)?;
    Ok(Verdict { receipt, keys: out, primary })
}

/// The content key under the disclosure key of the primary tag (spec §11.6), with a nonce derived
/// from the commit and the tag, so the output is deterministic.
fn wrap_d(keys: &ProgramKeys, program_key: &str, commit_id: u64, tag: &[u8; 32], k: &[u8; 32]) -> Result<String, Stop> {
    let dk = keys.disclose(tag).map_err(infra)?;
    let mut n = b"antibody/v2/wrap-d".to_vec();
    n.extend_from_slice(&commit_id.to_be_bytes());
    n.extend_from_slice(tag);
    let nonce: [u8; 12] = sha256(&n)[..12].try_into().unwrap();
    let ct = sealed::aead_seal_pub(&dk, &nonce, k, program_key.as_bytes());
    Ok(hex::encode([nonce.as_slice(), ct.as_slice()].concat()))
}

pub fn handle(input: &[u8], env: &Env) -> Output {
    let request_sha256 = hex::encode(sha256(input));
    let req: Request = match serde_json::from_slice(input) {
        Ok(r) => r,
        Err(e) => {
            return Output {
                facts: FactsOut {
                    v: 2,
                    mode: Mode::Verify,
                    request_sha256,
                    manifest_sha256: String::new(),
                    program_key: String::new(),
                    round: 0,
                    key_epoch: String::new(),
                    commit_id: None,
                    run_id: 0,
                    kind: Kind::InfraError,
                    keys: vec![],
                    impact_usd_micro: String::new(),
                    protected_harm_usd_micro: String::new(),
                    receipt_sha256: hex::encode([0u8; 32]),
                    reason: Some(short(&format!("bad request: {e}"))),
                    sealing_pk: None,
                    round_check: None,
                    wrap: None,
                    wrap_d: None,
                    disclosed: vec![],
                    distinct: None,
                },
                receipt: None,
            }
        }
    };
    let mut facts = FactsOut {
        v: 2,
        mode: req.mode,
        request_sha256,
        manifest_sha256: req.manifest_sha256.clone(),
        program_key: req.program_key.clone(),
        round: req.round,
        key_epoch: req.key_epoch.clone(),
        commit_id: req.commit_id,
        run_id: req.run_id,
        kind: Kind::InfraError,
        keys: vec![],
        impact_usd_micro: String::new(),
        protected_harm_usd_micro: String::new(),
        receipt_sha256: hex::encode([0u8; 32]),
        reason: None,
        sealing_pk: None,
        round_check: None,
        wrap: None,
        wrap_d: None,
        disclosed: vec![],
        distinct: None,
    };
    let mut receipt = None;
    match run_mode(&req, env, &mut facts, &mut receipt) {
        Ok(()) => {}
        Err(Stop(kind, reason)) => {
            facts.kind = kind;
            facts.reason = Some(short(&reason));
            facts.keys.clear();
        }
    }
    Output { facts, receipt }
}

fn run_mode(req: &Request, env: &Env, facts: &mut FactsOut, receipt_out: &mut Option<ProgramReceipt>) -> Result<(), Stop> {
    if req.v != 2 {
        return Err(infra("request version is not 2"));
    }
    // Fail closed: only the program's own contract may run it (its key is bound to that caller too).
    match &env.predecessor {
        Some(p) if !p.is_empty() && req.program_key.starts_with(&format!("{p}/")) => {}
        _ => return Err(infra("the caller is not the program's contract")),
    }
    let keys = ProgramKeys::new(env.mac, &req.program_key);
    let epoch = keys.key_epoch().map_err(infra)?;
    if req.mode != Mode::Activate && req.key_epoch != epoch {
        return Err(infra("the program's key epoch differs from this build's"));
    }
    match req.mode {
        Mode::Activate => {
            let (benv, base) = base_witness(env, req)?;
            let manifest = manifest_for(env, req, &base)?;
            check_base(&req.program_key, &benv, &base, &manifest)?;
            let pk = keys.seal_public().map_err(infra)?;
            facts.kind = Kind::NoViolation;
            facts.key_epoch = epoch;
            facts.sealing_pk = Some(hex::encode(pk));
            let mut r = b"antibody/v2/activation".to_vec();
            r.extend_from_slice(req.program_key.as_bytes());
            r.extend_from_slice(&sha256(&manifest));
            r.extend_from_slice(&pk);
            facts.receipt_sha256 = hex::encode(sha256(&r));
            Ok(())
        }
        Mode::Verify | Mode::Second | Mode::Rerun => {
            // A known issue (spec §7.6) is verified like a submission but has no commit: its receipt
            // names KNOWN_ISSUE_COMMIT, and it carries no disclosure wrap.
            let commit_id = match (req.mode, req.commit_id) {
                (_, Some(c)) => c,
                (Mode::Verify, None) => KNOWN_ISSUE_COMMIT,
                _ => return Err(infra("no commit")),
            };
            let hunter = req.hunter.as_deref().ok_or_else(|| infra("no hunter"))?;
            let (_, base) = base_witness(env, req)?;
            let manifest = manifest_for(env, req, &base)?;
            let blob = read_blob(env, req.blob.as_ref().ok_or_else(|| infra("no blob"))?)?;
            let (k, inner, witness_sha) = open(&keys, &req.program_key, hunter, &blob)?;
            let (envelope, w) = program_witness(inner, &base, manifest)?;
            let anchor: B256 = envelope.claimed_block_hash;
            let ctx = ProgramContext {
                program_id: req.program_key.clone(),
                commit_id,
                expected_manifest_hash: hex32(&req.manifest_sha256).map(B256::from),
                reserved: Some(reserved_address(&req.program_key, anchor)),
            };
            let v = judge(&keys, &ctx, &envelope, &w, witness_sha)?;
            facts.kind = v.receipt.kind;
            facts.reason = v.receipt.reason.as_ref().map(|r| short(r));
            facts.keys = v.keys;
            if let Some(i) = &v.receipt.impact {
                facts.impact_usd_micro = i.impact_usd_micro.clone();
                if i.protected_harm_usd_micro != "0" {
                    facts.protected_harm_usd_micro = i.protected_harm_usd_micro.clone();
                }
            }
            facts.receipt_sha256 = hex::encode(v.receipt.hash());
            if req.mode != Mode::Rerun && req.commit_id.is_some() {
                if let Some(t) = v.primary {
                    facts.wrap_d = Some(wrap_d(&keys, &req.program_key, commit_id, &t, &k)?);
                }
            }
            *receipt_out = Some(v.receipt);
            Ok(())
        }
        Mode::CheckRound => {
            let (benv, base) = base_witness(env, req)?;
            let round_bytes = manifest_for(env, req, &base)?;
            let base_m = Manifest::parse(&base.manifest_json).map_err(|e| Stop(Kind::ProgramMisconfigured, e))?;
            // The open round's manifest (round 0's for a first round): a round only ever adds invariants.
            let prev = match &req.prev_round_manifest {
                Some(r) => Manifest::parse(&read_blob(env, r)?).map_err(|e| infra(format!("previous round manifest: {}", short(&e))))?,
                None => base_m.clone(),
            };
            let rc = match Manifest::parse(&round_bytes) {
                Err(_) => RoundCheck { ok: false, canaries_ok: false },
                Ok(m) => match round_rules(&base_m, &m, req.round)
                    .and_then(|()| extends(&prev, &m))
                    .and_then(|()| new_slots_match(&prev, &m, &req.new_slots))
                {
                    Err(e) => {
                        facts.reason = Some(short(&e));
                        RoundCheck { ok: false, canaries_ok: false }
                    }
                    Ok(()) => {
                        let full = ProgramWitness {
                            manifest_json: round_bytes.clone(),
                            invariants_code: m.invariants.runtime_code.clone(),
                            header_rlp: base.header_rlp.clone(),
                            ancestor_headers_rlp: base.ancestor_headers_rlp.clone(),
                            state: base.state.clone(),
                            bytecodes: base.bytecodes.clone(),
                            bundle: crate::bundle::Bundle::default(),
                        };
                        let reserved = Some(reserved_address(&req.program_key, benv.claimed_block_hash));
                        match releases_of(req).map(|r| crate::run::check_round_at_anchor(&benv, &full, reserved, &r)) {
                            Some(Ok((canaries_ok, true))) => RoundCheck { ok: true, canaries_ok },
                            Some(Ok((_, false))) | None => {
                                facts.reason = Some("a release is not a checked override of a target proxy".into());
                                RoundCheck { ok: false, canaries_ok: false }
                            }
                            Some(Err(_)) => RoundCheck { ok: false, canaries_ok: false },
                        }
                    }
                },
            };
            let mut r = b"antibody/v2/round-check".to_vec();
            r.extend_from_slice(&sha256(&round_bytes));
            r.push(rc.ok as u8);
            r.push(rc.canaries_ok as u8);
            facts.kind = Kind::NoViolation;
            facts.receipt_sha256 = hex::encode(sha256(&r));
            facts.round_check = Some(rc);
            Ok(())
        }
        Mode::Distinct => {
            let commit_id = req.commit_id.ok_or_else(|| infra("no commit"))?;
            let hunter = req.hunter.as_deref().ok_or_else(|| infra("no hunter"))?;
            let slot = req.slot.ok_or_else(|| infra("no slot"))?;
            let earlier = req.earlier.as_ref().ok_or_else(|| infra("no earlier finding"))?;
            let (_, base) = base_witness(env, req)?;
            let manifest = manifest_for(env, req, &base)?;
            let base_m = Manifest::parse(&manifest).map_err(|e| Stop(Kind::ProgramMisconfigured, e))?;
            let patch_bytes = read_blob(env, req.patch.as_ref().ok_or_else(|| infra("no patch"))?)?;
            let patch: Vec<(Address, Bytes)> =
                serde_json::from_slice(&patch_bytes).map_err(|_| invalid("the patch is not a list of (target, code)"))?;
            let patched = base_m.with_overrides(1, &patch).map_err(|e| invalid(format!("patch: {}", short(&e))))?;
            let patched_bytes: Bytes = patched.canonical().into();
            let breaks = |hunter: &str, cid: u64, blob: &BlobRef| -> Result<bool, Stop> {
                let bytes = read_blob(env, blob)?;
                let (_, inner, wsha) = open(&keys, &req.program_key, hunter, &bytes)?;
                let (envelope, w) = program_witness(inner, &base, patched_bytes.clone())?;
                let ctx = ProgramContext {
                    program_id: req.program_key.clone(),
                    commit_id: cid,
                    expected_manifest_hash: None,
                    reserved: Some(reserved_address(&req.program_key, envelope.claimed_block_hash)),
                };
                let r = run_program(&envelope, &w, wsha, &ctx);
                Ok(r.kind == Kind::Violation && r.broken.iter().any(|f| f.slot == slot))
            };
            let earlier_breaks = breaks(&earlier.hunter, earlier.commit_id, &earlier.blob)?;
            let mine_breaks = breaks(hunter, commit_id, req.blob.as_ref().ok_or_else(|| infra("no blob"))?)?;
            let distinct = !earlier_breaks && mine_breaks;
            let mut r = b"antibody/v2/distinct".to_vec();
            r.extend_from_slice(&sha256(&patch_bytes));
            r.extend_from_slice(&commit_id.to_be_bytes());
            r.extend_from_slice(&earlier.commit_id.to_be_bytes());
            r.push(distinct as u8);
            facts.kind = Kind::NoViolation;
            facts.distinct = Some(distinct);
            facts.receipt_sha256 = hex::encode(sha256(&r));
            Ok(())
        }
        Mode::ReleaseWrap => {
            let hunter = req.hunter.as_deref().ok_or_else(|| infra("no hunter"))?;
            let pk = req.project_pk.as_deref().and_then(hex32).ok_or_else(|| infra("no project key"))?;
            if !sealed::is_contributory(&pk) {
                return Err(infra("the project key is a low-order point"));
            }
            let blob = read_blob(env, req.blob.as_ref().ok_or_else(|| infra("no blob"))?)?;
            let (k, _, _) = open(&keys, &req.program_key, hunter, &blob)?;
            let wrap = sealed::project_wrap(&k, &pk, &req.program_key);
            facts.kind = Kind::NoViolation;
            facts.wrap = Some(hex::encode(wrap));
            facts.receipt_sha256 = hex::encode(sha256(&wrap));
            Ok(())
        }
        Mode::Disclose => {
            if req.tags.is_empty() || req.tags.len() > 8 {
                return Err(infra("1 to 8 tags per disclosure"));
            }
            let mut out = vec![];
            for t in &req.tags {
                let tag = hex32(t).ok_or_else(|| infra("bad tag"))?;
                out.push(Disclosed { tag: hex::encode(tag), key: hex::encode(keys.disclose(&tag).map_err(infra)?) });
            }
            let mut r = b"antibody/v2/disclose".to_vec();
            for d in &out {
                r.extend_from_slice(d.tag.as_bytes());
            }
            facts.kind = Kind::NoViolation;
            facts.receipt_sha256 = hex::encode(sha256(&r));
            facts.disclosed = out;
            Ok(())
        }
    }
}

/// Activation: the base witness is canonical, anchored at the manifest's block, and the program passes
/// its checks at the anchor (spec §3.1).
fn check_base(program_key: &str, benv: &Envelope, base: &BaseWitness, manifest: &Bytes) -> Result<(), Stop> {
    crate::trie::validate(&base.state).map_err(|e| Stop(Kind::ProgramMisconfigured, format!("base witness: {e}")))?;
    let m = Manifest::parse(manifest).map_err(|e| Stop(Kind::ProgramMisconfigured, e))?;
    if m.invariants.runtime_code != base.invariants_code {
        return Err(Stop(Kind::ProgramMisconfigured, "the base witness's invariants differ from the manifest".into()));
    }
    let full = ProgramWitness {
        manifest_json: manifest.clone(),
        invariants_code: base.invariants_code.clone(),
        header_rlp: base.header_rlp.clone(),
        ancestor_headers_rlp: base.ancestor_headers_rlp.clone(),
        state: base.state.clone(),
        bytecodes: base.bytecodes.clone(),
        bundle: crate::bundle::Bundle::default(),
    };
    crate::run::check_program_at_anchor(benv, &full, Some(reserved_address(program_key, benv.claimed_block_hash)))
        .map(|_| ())
        .map_err(|(k, r)| Stop(k, r))
}

/// A round may replace target code and add slots; nothing else changes (spec §8.1).
/// A round keeps every invariant of the round it follows: its slots extend the previous ones, and the
/// invariants code changes only when slots are added.
fn extends(prev: &Manifest, round: &Manifest) -> Result<(), String> {
    let (p, r) = (&prev.invariants, &round.invariants);
    if r.slots.len() < p.slots.len() || r.slots[..p.slots.len()] != p.slots[..] {
        return Err("a round keeps every invariant of the round before it".into());
    }
    if r.slots.len() == p.slots.len() && r.runtime_code_hash != p.runtime_code_hash {
        return Err("the invariants code changes only with new slots".into());
    }
    Ok(())
}

/// The slots a proposal names are exactly the ones its round manifest adds, with their severities.
fn new_slots_match(base: &Manifest, round: &Manifest, named: &[NewSlotIn]) -> Result<(), String> {
    let added: Vec<(u8, String)> = round.invariants.slots[base.invariants.slots.len()..]
        .iter()
        .map(|s| (s.bit, serde_json::to_value(s.severity).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()))
        .collect();
    let named: Vec<(u8, String)> = named.iter().map(|s| (s.slot, s.severity.to_ascii_lowercase())).collect();
    if added != named {
        return Err("the proposal's new slots are not the ones the round manifest adds".into());
    }
    Ok(())
}

fn round_rules(base: &Manifest, round: &Manifest, n: u32) -> Result<(), String> {
    if round.round != n || n == 0 {
        return Err("wrong round number".into());
    }
    let mut b = base.clone();
    let mut r = round.clone();
    // Overridden targets carry their override's hash; slots may only be appended.
    if r.invariants.slots.len() < b.invariants.slots.len() || r.invariants.slots[..b.invariants.slots.len()] != b.invariants.slots[..] {
        return Err("a round may only add slots".into());
    }
    for t in r.targets.iter_mut() {
        if let Some(o) = round.overrides.iter().find(|o| o.address == t.address) {
            if let Some(bt) = b.targets.iter().find(|x| x.address == t.address) {
                if t.code_hash == o.code_hash {
                    t.code_hash = bt.code_hash;
                }
            }
        }
    }
    r.round = 0;
    r.overrides.clear();
    // New invariants (a new contract with more slots) are allowed; the rest must be byte-identical.
    let added_slots = r.invariants.slots.len() > b.invariants.slots.len();
    if added_slots {
        r.invariants.slots.truncate(b.invariants.slots.len());
        b.invariants.runtime_code = r.invariants.runtime_code.clone();
        b.invariants.runtime_code_hash = r.invariants.runtime_code_hash;
    }
    if r != b {
        return Err("a round changes more than target code and new slots".into());
    }
    Ok(())
}


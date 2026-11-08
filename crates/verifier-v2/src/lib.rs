//! Verifier v2: v1 plus the NEAR engine (docs/near-targets.md). This crate is glue only:
//!
//! - **Dispatch.** An EVM program (a manifest with no `vm`, or `vm: "evm"`) takes v1's code path through
//!   v1's own library functions, unchanged, so its facts and receipts are v1's byte for byte. A NEAR
//!   program (`vm: "near"`, carried in a NEAR base witness) runs on `antibody-verifier-near`.
//! - **Keys.** A NEAR root cause's tag is v1's tag (`ProgramKeys::tag`, docs/spec.md §7.2, §11.2) over
//!   NEAR group bytes (`antibody_verifier_near::receipt::group_bytes`).
//! - **Facts.** v1's facts structure (`tee::FactsOut`), filled from the NEAR receipt as v1 fills it from
//!   its own.

use antibody_verifier::{
    facts::{Kind as V1Kind, MAX_KEYS},
    keys::{Mac, ProgramKeys},
    sealed,
    tee::{self, BlobRef, Env, FactsOut, KeyOut, Mode, Output, Request},
};
use antibody_verifier_near::{
    receipt::{group_bytes, Kind, NearReceipt},
    run::{self, Input},
    util::{sha256, unb64},
    witness::{NearBase, NearSubmission, NearWitness, NEAR_BASE_SCHEMA, NEAR_SUBMISSION_SCHEMA, NEAR_WITNESS_SCHEMA},
};

/// The facts kind v1 names for a NEAR verdict.
pub fn v1_kind(k: Kind) -> V1Kind {
    match k {
        Kind::Violation => V1Kind::Violation,
        Kind::NoViolation => V1Kind::NoViolation,
        Kind::InvalidPoC => V1Kind::InvalidPoC,
        Kind::ProgramMisconfigured => V1Kind::ProgramMisconfigured,
        Kind::Unevaluable => V1Kind::Unevaluable,
        Kind::InfraError => V1Kind::InfraError,
    }
}

/// The keys of a NEAR receipt: tags under the program's key (at most v1's four), and the primary
/// (highest-severity, first on ties) tag.
pub fn near_keys(keys: &ProgramKeys, receipt: &NearReceipt) -> Result<(Vec<KeyOut>, Option<[u8; 32]>), String> {
    let mut out = vec![];
    let mut primary: Option<([u8; 32], u8)> = None;
    if receipt.kind == Kind::Violation {
        for f in receipt.broken.iter().take(MAX_KEYS) {
            let t = keys.tag(f.slot, &group_bytes(f.group, &f.receiver, &f.method))?;
            let sev = f.severity as u8;
            if primary.map(|(_, s)| sev > s).unwrap_or(true) {
                primary = Some((t, sev));
            }
            out.push(KeyOut { slot: f.slot, tag: hex::encode(t) });
        }
    }
    Ok((out, primary.map(|(t, _)| t)))
}

fn short(r: &str) -> String {
    r.chars().take(64).collect()
}

/// Facts for a local run (no request), as v1's `tee::local_facts` writes them.
pub fn local_facts(keys: &ProgramKeys, receipt: &NearReceipt, program_key: &str, commit_id: u64) -> FactsOut {
    let (k, _) = near_keys(keys, receipt).unwrap_or_default();
    FactsOut {
        v: 2,
        mode: Mode::Verify,
        request_sha256: String::new(),
        manifest_sha256: receipt.manifest_hash.trim_start_matches("0x").to_string(),
        program_key: program_key.to_string(),
        round: 0,
        key_epoch: keys.key_epoch().unwrap_or_default(),
        commit_id: Some(commit_id),
        run_id: 0,
        kind: v1_kind(receipt.kind),
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

/// Whether `input` is a NEAR program witness for a local run.
pub fn is_near_witness(input: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(input).ok().and_then(|v| v.get("schema").and_then(|s| s.as_str()).map(|s| s == NEAR_WITNESS_SCHEMA)).unwrap_or(false)
}

/// A local NEAR run (the analogue of v1's program mode): the receipt and the facts under `mac`.
pub fn local_run(input: &[u8], program_key: &str, commit_id: u64, expected_manifest: Option<[u8; 32]>, mac: &dyn Mac) -> (NearReceipt, FactsOut) {
    let witness_sha256 = sha256(input);
    let receipt = match serde_json::from_slice::<NearWitness>(input) {
        Ok(w) if w.schema == NEAR_WITNESS_SCHEMA => match w.codes.iter().map(|c| unb64(c)).collect::<Result<Vec<_>, _>>() {
            Ok(codes) => run::run(&Input {
                manifest_json: w.manifest.as_bytes(),
                codes: &codes,
                bundle: w.bundle.as_ref(),
                program: program_key,
                commit_id,
                witness_sha256,
                expected_manifest_hash: expected_manifest,
            }),
            Err(e) => failure(program_key, commit_id, witness_sha256, Kind::InvalidPoC, format!("witness: code {e}")),
        },
        _ => failure(program_key, commit_id, witness_sha256, Kind::InvalidPoC, "witness: not a NEAR program witness".into()),
    };
    let keys = ProgramKeys::new(mac, program_key);
    let facts = local_facts(&keys, &receipt, program_key, commit_id);
    (receipt, facts)
}

fn failure(program: &str, commit_id: u64, witness_sha256: [u8; 32], kind: Kind, reason: String) -> NearReceipt {
    run::failure_receipt(program, commit_id, witness_sha256, kind, reason)
}

// ---- TEE mode (docs/spec.md §5) ----

const MAX_INLINE_BYTES: usize = 32 * 1024;
const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;

struct Stop(V1Kind, String);

fn infra(r: impl Into<String>) -> Stop {
    Stop(V1Kind::InfraError, r.into())
}
fn invalid(r: impl Into<String>) -> Stop {
    Stop(V1Kind::InvalidPoC, r.into())
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    hex::decode(s.trim_start_matches("0x")).ok()?.try_into().ok()
}

/// Reads a blob as v1 does: inline bytes are bound by near-antibody's own hash; a fetched blob must match
/// its committed hash, and a mismatch is the RPC's fault.
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
            let bytes = (env.fetch)(near_tx, &want).map_err(|e| infra(format!("blob: unavailable ({})", short(&e))))?;
            if bytes.len() > MAX_BLOB_BYTES {
                return Err(infra("blob: too large"));
            }
            if sha256(&bytes) != want {
                return Err(infra("blob: does not match its committed hash"));
            }
            Ok(bytes)
        }
    }
}

/// A NEAR base witness, if `bytes` is one.
fn near_base(bytes: &[u8]) -> Option<Result<NearBase, String>> {
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if v.get("schema").and_then(|s| s.as_str()) != Some(NEAR_BASE_SCHEMA) {
        return None;
    }
    Some(serde_json::from_value::<NearBase>(v).map_err(|e| e.to_string()))
}

/// The TEE entry point. An EVM program's request goes to v1's `tee::handle` unchanged (the base witness is
/// fetched once here, to see which kind of program it is, and served to v1 from memory); a NEAR program's
/// request runs on the NEAR engine.
pub fn handle(input: &[u8], env: &Env) -> Output {
    let Ok(req) = serde_json::from_slice::<Request>(input) else {
        return tee::handle(input, env);
    };
    // Modes that read no program state are the same for every program: v1's.
    if matches!(req.mode, Mode::ReleaseWrap | Mode::Disclose) {
        return tee::handle(input, env);
    }
    let base = match read_blob(env, &req.base) {
        Ok(b) => b,
        Err(_) => return tee::handle(input, env),
    };
    match near_base(&base) {
        None => {
            let location = match &req.base {
                BlobRef::NearTx { near_tx, .. } => Some(near_tx.clone()),
                BlobRef::Inline { .. } => None,
            };
            let cached = |url: &str, want: &[u8; 32]| -> Result<Vec<u8>, String> {
                if location.as_deref() == Some(url) {
                    Ok(base.clone())
                } else {
                    (env.fetch)(url, want)
                }
            };
            let v1_env = Env { mac: env.mac, fetch: &cached, predecessor: env.predecessor.clone() };
            tee::handle(input, &v1_env)
        }
        Some(parsed) => near_handle(input, &req, parsed, env),
    }
}

fn near_handle(input: &[u8], req: &Request, base: Result<NearBase, String>, env: &Env) -> Output {
    let mut facts = FactsOut {
        v: 2,
        mode: req.mode,
        request_sha256: hex::encode(sha256(input)),
        manifest_sha256: req.manifest_sha256.clone(),
        program_key: req.program_key.clone(),
        round: req.round,
        key_epoch: req.key_epoch.clone(),
        commit_id: req.commit_id,
        run_id: req.run_id,
        kind: V1Kind::InfraError,
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
    if let Err(Stop(kind, reason)) = near_mode(req, base, env, &mut facts) {
        facts.kind = kind;
        facts.reason = Some(short(&reason));
        facts.keys.clear();
    }
    Output { facts, receipt: None }
}

/// The content key under the disclosure key of the primary tag, as v1 wraps it (docs/spec.md §11.6).
fn wrap_d(keys: &ProgramKeys, program_key: &str, commit_id: u64, round: u32, tag: &[u8; 32], k: &[u8; 32]) -> Result<String, Stop> {
    let dk = keys.disclose(tag, round).map_err(infra)?;
    let mut n = b"antibody/v2/wrap-d".to_vec();
    n.extend_from_slice(&commit_id.to_be_bytes());
    n.extend_from_slice(tag);
    let nonce: [u8; 12] = sha256(&n)[..12].try_into().expect("12 bytes");
    let ct = sealed::aead_seal_pub(&dk, &nonce, k, program_key.as_bytes());
    Ok(hex::encode([nonce.as_slice(), ct.as_slice()].concat()))
}

fn near_mode(req: &Request, base: Result<NearBase, String>, env: &Env, facts: &mut FactsOut) -> Result<(), Stop> {
    if req.v != 2 {
        return Err(infra("request version is not 2"));
    }
    match &env.predecessor {
        Some(p) if !p.is_empty() && req.program_key.starts_with(&format!("{p}/")) => {}
        _ => return Err(infra("the caller is not the program's contract")),
    }
    let keys = ProgramKeys::new(env.mac, &req.program_key);
    let epoch = keys.key_epoch().map_err(infra)?;
    if req.mode != Mode::Activate && req.key_epoch != epoch {
        return Err(infra("the program's key epoch differs from this build's"));
    }
    let base = base.map_err(|e| Stop(V1Kind::ProgramMisconfigured, format!("base witness: {}", short(&e))))?;
    if let Some(want) = req.program_manifest_sha256.as_deref() {
        if Some(sha256(base.manifest.as_bytes())) != hex32(want) {
            return Err(infra("the base witness does not carry this program's manifest"));
        }
    }
    if req.round != 0 || req.round_manifest.is_some() {
        return Err(infra("rounds after 0 are not supported for NEAR programs yet"));
    }
    let manifest = base.manifest.as_bytes();
    let want = hex32(&req.manifest_sha256).ok_or_else(|| infra("bad manifest hash"))?;
    if sha256(manifest) != want {
        return Err(infra("manifest bytes do not match the committed hash"));
    }
    let codes = base
        .codes
        .iter()
        .map(|c| unb64(c))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Stop(V1Kind::ProgramMisconfigured, format!("base witness: code {e}")))?;
    match req.mode {
        Mode::Activate => {
            let r = run::run(&Input {
                manifest_json: manifest,
                codes: &codes,
                bundle: None,
                program: &req.program_key,
                commit_id: 0,
                witness_sha256: [0; 32],
                expected_manifest_hash: Some(want),
            });
            if r.kind != Kind::NoViolation {
                return Err(Stop(v1_kind(r.kind), r.reason.unwrap_or_default()));
            }
            let pk = keys.seal_public().map_err(infra)?;
            facts.kind = V1Kind::NoViolation;
            facts.key_epoch = epoch;
            facts.sealing_pk = Some(hex::encode(pk));
            let mut m = b"antibody/v2/activation".to_vec();
            m.extend_from_slice(req.program_key.as_bytes());
            m.extend_from_slice(&sha256(manifest));
            m.extend_from_slice(&pk);
            facts.receipt_sha256 = hex::encode(sha256(&m));
            Ok(())
        }
        Mode::Verify | Mode::Second | Mode::Rerun => {
            let commit_id = match (req.mode, req.commit_id) {
                (_, Some(c)) => c,
                (Mode::Verify, None) => tee::KNOWN_ISSUE_COMMIT,
                _ => return Err(infra("no commit")),
            };
            let hunter = req.hunter.as_deref().ok_or_else(|| infra("no hunter"))?;
            let blob = read_blob(env, req.blob.as_ref().ok_or_else(|| infra("no blob"))?)?;
            let sk = keys.seal_secret().map_err(infra)?;
            let (k, payload) = sealed::open(&blob, &sk, &req.program_key).map_err(|e| invalid(format!("cannot open the sealed blob: {e}")))?;
            if payload.program_key != req.program_key || payload.hunter != hunter {
                return Err(invalid("the sealed payload is bound to another program or hunter"));
            }
            let witness_sha256 = sha256(&payload.envelope);
            let sub: NearSubmission = serde_json::from_slice(&payload.envelope).map_err(|e| invalid(format!("witness: {}", short(&e.to_string()))))?;
            if sub.schema != NEAR_SUBMISSION_SCHEMA {
                return Err(invalid("witness: not a NEAR submission"));
            }
            let r = run::run(&Input {
                manifest_json: manifest,
                codes: &codes,
                bundle: Some(&sub.bundle),
                program: &req.program_key,
                commit_id,
                witness_sha256,
                expected_manifest_hash: Some(want),
            });
            let (out, primary) = near_keys(&keys, &r).map_err(infra)?;
            facts.kind = v1_kind(r.kind);
            facts.reason = r.reason.as_ref().map(|x| short(x));
            facts.keys = out;
            if let Some(i) = &r.impact {
                facts.impact_usd_micro = i.impact_usd_micro.clone();
                if i.protected_harm_usd_micro != "0" {
                    facts.protected_harm_usd_micro = i.protected_harm_usd_micro.clone();
                }
            }
            facts.receipt_sha256 = hex::encode(r.hash());
            if req.mode == Mode::Rerun {
                facts.impact_usd_micro.clear();
                facts.protected_harm_usd_micro.clear();
            }
            if req.mode != Mode::Rerun && req.commit_id.is_some() {
                if let Some(t) = primary {
                    facts.wrap_d = Some(wrap_d(&keys, &req.program_key, commit_id, req.round, &t, &k)?);
                }
            }
            Ok(())
        }
        Mode::CheckRound | Mode::Distinct => Err(infra("this mode is not supported for NEAR programs yet")),
        Mode::ReleaseWrap | Mode::Disclose => unreachable!("handled by v1"),
    }
}

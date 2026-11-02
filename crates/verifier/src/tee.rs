//! OutLayer entry point (TEE mode). The near-antibody contract sends a small JSON request as
//! `input_data` (stdin); the verifier answers on stdout:
//!
//! - `{"mode":"pubkey"}` → `{"pubkey":hex}` — the sealed-submission public key of this build.
//! - `{"mode":"verify",…}` → facts (≤ 1 KB): fetch the sealed blob, check its hash, unwrap the content
//!   key, decrypt, check the payload is bound to this program and hunter, run program mode, then
//!   compute fp_tags with `k_program` and, for a Violation, the project wrap.
//! - `{"mode":"rerun",…}` → facts for a claimed finding re-run at a later round's manifest (the
//!   project's fix as code overrides, docs/rounds.md). The blob is a fresh witness for the same PoC:
//!   its `poc_hash` must equal the original verdict's, so only the code under test changes.
//! - `{"mode":"disclose",…}` → `{"program","k_program","keys":[{commit_id,round?,content_key}]}`.
//!
//! Deterministic problems with a submission become typed facts (`InvalidPoC`). Problems that a retry
//! can fix (fetching the blob) are `Transient`: the process exits non-zero, OutLayer delivers
//! `null`, and the contract allows a retry.

use alloy_primitives::B256;
use serde::Deserialize;
use serde_json::json;

use crate::{
    facts::{derive_k_program, sha256, Kind},
    run::{failure_receipt, run_program, ProgramContext},
    sealed, Envelope, Payload,
};

pub const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Request {
    Pubkey {},
    Verify {
        program_id: String,
        commit_id: u64,
        hunter: String,
        manifest_hash: String,
        sealed: SealedRefIn,
        project_pubkey: String,
    },
    Rerun {
        program_id: String,
        commit_id: u64,
        round: u32,
        hunter: String,
        /// The round's manifest hash committed on NEAR.
        manifest_hash: String,
        /// `poc_hash` of the original verdict: the re-run must execute the same PoC.
        poc_hash: String,
        sealed: SealedRefIn,
    },
    Disclose {
        program_id: String,
        entries: Vec<DiscloseEntry>,
    },
}

#[derive(Debug, Deserialize)]
pub struct SealedRefIn {
    pub url: String,
    pub sha256: String,
    pub wrap_v: String,
}

#[derive(Debug, Deserialize)]
pub struct DiscloseEntry {
    pub commit_id: u64,
    /// Set for a re-run blob (its own content key).
    #[serde(default)]
    pub round: Option<u32>,
    pub wrap_v: String,
}

#[derive(Debug)]
pub enum TeeError {
    /// Retry can help (fetch failed): exit non-zero so the contract may retry.
    Transient(String),
    /// The request itself is unusable (bad JSON, no secret, wrong caller): exit non-zero.
    Fatal(String),
}

pub struct TeeEnv<'a> {
    /// 32-byte seed from the protected secret `PROTECTED_ANTIBODY_SEED`.
    pub seed: Option<[u8; 32]>,
    /// `NEAR_PREDECESSOR_ID`: the account that called OutLayer (must own the program).
    pub predecessor: Option<String>,
    pub fetch: &'a dyn Fn(&str) -> Result<Vec<u8>, String>,
    pub wasm_hash: Option<String>,
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    let b = hex::decode(s.trim_start_matches("0x")).ok()?;
    b.try_into().ok()
}

enum Opened {
    Ok { k: [u8; 32], envelope: Envelope, witness_sha: [u8; 32] },
    /// Deterministically unusable: becomes an `InvalidPoC` receipt over this hash.
    Reject(String, [u8; 32]),
}

/// Fetches a sealed blob, checks it against the committed hash and key wrap, decrypts it and checks
/// that its payload is bound to this program and hunter.
fn open_sealed(env: &TeeEnv, seed: &[u8; 32], program_id: &str, hunter: &str, s: &SealedRefIn) -> Result<Opened, TeeError> {
    let blob = (env.fetch)(&s.url).map_err(|e| TeeError::Transient(format!("fetch sealed blob: {e}")))?;
    if blob.len() > MAX_BLOB_BYTES {
        return Ok(Opened::Reject("sealed blob too large".into(), sha256(&blob[..64.min(blob.len())])));
    }
    let blob_sha = sha256(&blob);
    if Some(blob_sha) != hex32(&s.sha256) {
        return Ok(Opened::Reject("sealed blob hash differs from the committed hash".into(), blob_sha));
    }
    let committed_wrap = hex::decode(&s.wrap_v).unwrap_or_default();
    match sealed::split(&blob) {
        Ok((w, _, _)) if w == committed_wrap.as_slice() => {}
        _ => return Ok(Opened::Reject("key wrap differs from the committed wrap_v".into(), blob_sha)),
    }
    let (k, payload) = match sealed::open(&blob, &sealed::verifier_secret(seed), program_id) {
        Ok(x) => x,
        Err(e) => return Ok(Opened::Reject(format!("cannot open the sealed blob: {e}"), blob_sha)),
    };
    if payload.program_key != program_id || payload.hunter != hunter {
        return Ok(Opened::Reject("sealed payload is bound to another program or hunter".into(), blob_sha));
    }
    // The report text is never parsed: only the code in the envelope is executed.
    let witness_sha = sha256(&payload.envelope);
    match Envelope::decode(&payload.envelope) {
        Ok(envelope) => Ok(Opened::Ok { k, envelope, witness_sha }),
        Err(e) => Ok(Opened::Reject(format!("witness: {e}"), witness_sha)),
    }
}

pub fn handle(input: &[u8], env: &TeeEnv) -> Result<Vec<u8>, TeeError> {
    let req: Request = serde_json::from_slice(input).map_err(|e| TeeError::Fatal(format!("bad request: {e}")))?;
    let seed = env.seed.ok_or_else(|| TeeError::Fatal("PROTECTED_ANTIBODY_SEED is not available".into()))?;
    let caller_owns = |program_id: &str| match &env.predecessor {
        Some(p) if !p.is_empty() => program_id.starts_with(&format!("{p}/")),
        _ => true,
    };
    match req {
        Request::Pubkey {} => Ok(json!({"pubkey": hex::encode(sealed::verifier_public(&seed))}).to_string().into_bytes()),
        Request::Disclose { program_id, entries } => {
            if !caller_owns(&program_id) {
                return Err(TeeError::Fatal("disclosure must be requested by the program's contract".into()));
            }
            let sk = sealed::verifier_secret(&seed);
            let keys: Vec<serde_json::Value> = entries
                .iter()
                .filter_map(|e| {
                    let w = hex::decode(&e.wrap_v).ok()?;
                    let k = sealed::unwrap_v(&w, &sk, &program_id).ok()?;
                    let mut v = json!({"commit_id": e.commit_id, "content_key": hex::encode(k)});
                    if let Some(r) = e.round {
                        v["round"] = json!(r);
                    }
                    Some(v)
                })
                .collect();
            let k_program = derive_k_program(&seed, &program_id);
            Ok(json!({"program": program_id, "k_program": hex::encode(k_program), "keys": keys}).to_string().into_bytes())
        }
        Request::Verify { program_id, commit_id, hunter, manifest_hash, sealed: s, project_pubkey } => {
            if !caller_owns(&program_id) {
                return Err(TeeError::Fatal("verification must be requested by the program's contract".into()));
            }
            let mh = hex32(&manifest_hash).ok_or_else(|| TeeError::Fatal("bad manifest_hash".into()))?;
            let ctx = ProgramContext { program_id: program_id.clone(), commit_id, expected_manifest_hash: Some(B256::from(mh)) };
            let k_program = derive_k_program(&seed, &program_id);
            let reject = |reason: &str, sha: [u8; 32]| {
                let r = failure_receipt(&ctx, Kind::InvalidPoC, reason, B256::from(mh), sha);
                serde_json::to_vec(&r.facts(Some(&k_program), None, env.wasm_hash.clone())).expect("facts serialize")
            };
            let (k, envelope, witness_sha) = match open_sealed(env, &seed, &program_id, &hunter, &s)? {
                Opened::Ok { k, envelope, witness_sha } => (k, envelope, witness_sha),
                Opened::Reject(reason, sha) => return Ok(reject(&reason, sha)),
            };
            let w = match &envelope.payload {
                Payload::Program(w) => w,
                _ => return Ok(reject("witness is not a program witness", witness_sha)),
            };
            let receipt = run_program(&envelope, w, witness_sha, &ctx);
            let wrap = match (receipt.kind, hex32(&project_pubkey)) {
                (Kind::Violation, Some(pk_p)) => Some(hex::encode(sealed::project_wrap(&k, &pk_p, &program_id))),
                _ => None,
            };
            Ok(serde_json::to_vec(&receipt.facts(Some(&k_program), wrap, env.wasm_hash.clone())).expect("facts serialize"))
        }
        Request::Rerun { program_id, commit_id, round, hunter, manifest_hash, poc_hash, sealed: s } => {
            if !caller_owns(&program_id) {
                return Err(TeeError::Fatal("re-runs must be requested by the program's contract".into()));
            }
            if round == 0 {
                return Err(TeeError::Fatal("re-runs are for rounds > 0".into()));
            }
            let mh = hex32(&manifest_hash).ok_or_else(|| TeeError::Fatal("bad manifest_hash".into()))?;
            let claimed = hex32(&poc_hash).ok_or_else(|| TeeError::Fatal("bad poc_hash".into()))?;
            let ctx = ProgramContext { program_id: program_id.clone(), commit_id, expected_manifest_hash: Some(B256::from(mh)) };
            let k_program = derive_k_program(&seed, &program_id);
            let reject = |reason: &str, sha: [u8; 32]| {
                let r = failure_receipt(&ctx, Kind::InvalidPoC, reason, B256::from(mh), sha);
                serde_json::to_vec(&r.facts(Some(&k_program), None, env.wasm_hash.clone())).expect("facts serialize")
            };
            let (envelope, witness_sha) = match open_sealed(env, &seed, &program_id, &hunter, &s)? {
                Opened::Ok { envelope, witness_sha, .. } => (envelope, witness_sha),
                Opened::Reject(reason, sha) => return Ok(reject(&reason, sha)),
            };
            let w = match &envelope.payload {
                Payload::Program(w) => w,
                _ => return Ok(reject("witness is not a program witness", witness_sha)),
            };
            if crate::run::poc_hash(&w.bundle) != claimed {
                return Ok(reject("the re-run PoC differs from the claimed finding's PoC", witness_sha));
            }
            let receipt = run_program(&envelope, w, witness_sha, &ctx);
            Ok(serde_json::to_vec(&receipt.facts(Some(&k_program), None, env.wasm_hash.clone())).expect("facts serialize"))
        }
    }
}

//! A disclosed verdict, re-run (docs/zk-feasibility.md).
//!
//! Given the exact request near-antibody sent to the TEE for a verification run, the program's pinned
//! base witness, the sealed blob and its disclosed content key, [`rerun`] runs the verifier core the TEE
//! ran (`antibody_verifier::run_program`), without the program's secret key, and returns its receipt.
//! [`Rerun::public_values`] is what a validity proof of that run commits: the hashes a NEAR contract
//! compares with near-antibody's records (the request, the blob, the base, the manifest, the receipt),
//! the run's place (program, round, commit, run), the anchor, the verifier's version and the verdict.
//!
//! The same code runs natively (`tests/`) and in SP1's zkVM (`crates/zk/guest`). It claims no privacy:
//! its input is a proof that was disclosed.
//!
//! What it cannot recompute: the keys' tags. They are MACs under the program's key, which only the TEE
//! holds. The public values carry each broken slot and its severity instead, which near-antibody's
//! recorded keys name too.

use alloy_primitives::{Bytes, B256};
use antibody_verifier::{
    facts::{sha256, Kind, ProgramReceipt},
    manifest::{Manifest, Severity},
    run::{reserved_address, run_program, ProgramContext},
    sealed,
    tee::{BlobRef, Mode, Request, KNOWN_ISSUE_COMMIT},
    Envelope, Payload, ProgramWitness,
};

/// The public values' layout version (the first byte).
pub const PUBLIC_VALUES_V1: u8 = 1;

/// What a disclosed verification run needs.
pub struct Disclosed<'a> {
    /// The exact bytes near-antibody sent to OutLayer (`input_data`): their sha256 is the run's
    /// `request_sha256` on chain.
    pub request: &'a [u8],
    /// The program's pinned base witness (the request names its sha256).
    pub base: &'a [u8],
    /// A later round's manifest; round 0 reads the base's.
    pub round_manifest: Option<&'a [u8]>,
    /// The sealed blob, when the request names it by hash (an inline blob travels in the request).
    pub blob: Option<&'a [u8]>,
    /// The blob's content key. Publishing it discloses the proof.
    pub content_key: [u8; 32],
}

/// The re-run: the receipt, and the facts that bind it to near-antibody's records.
pub struct Rerun {
    pub request_sha256: [u8; 32],
    pub blob_sha256: [u8; 32],
    pub base_sha256: [u8; 32],
    pub manifest_sha256: [u8; 32],
    pub program_key: String,
    pub round: u32,
    pub commit_id: u64,
    pub run_id: u64,
    pub receipt: ProgramReceipt,
    /// sha256 of the receipt's canonical bytes: the TEE's run records the same hash on chain.
    pub receipt_sha256: [u8; 32],
}

fn hex32(s: &str) -> Result<[u8; 32], String> {
    let b = hex::decode(s.trim_start_matches("0x")).map_err(|_| format!("not hex: {s}"))?;
    b.try_into().map_err(|_| format!("not 32 bytes: {s}"))
}

/// Whether `bytes` are the blob a request names: an inline blob's bytes, or a hash.
fn names(r: &BlobRef, bytes: &[u8]) -> Result<bool, String> {
    Ok(match r {
        BlobRef::Inline { inline } => hex::decode(inline).map_err(|_| "an inline blob that is not hex")? == bytes,
        BlobRef::NearTx { sha256: want, .. } => sha256(bytes) == hex32(want)?,
    })
}

/// Runs the verifier core on a disclosed verification run, as the TEE's `verify` mode does
/// (`antibody_verifier::tee`), with the content key in place of the program's sealing key. Every check
/// the TEE makes on its input is made here; a run that would have stopped before a verdict is an error.
pub fn rerun(d: &Disclosed) -> Result<Rerun, String> {
    let request_sha256 = sha256(d.request);
    let req: Request = serde_json::from_slice(d.request).map_err(|e| format!("request: {e}"))?;
    if req.v != 2 {
        return Err("request version is not 2".into());
    }
    let commit_id = match (req.mode, req.commit_id) {
        (Mode::Verify | Mode::Second | Mode::Rerun, Some(c)) => c,
        // A known issue is verified like a submission and names no commit.
        (Mode::Verify, None) => KNOWN_ISSUE_COMMIT,
        (m, _) => return Err(format!("a {m:?} run has no verdict to re-run")),
    };
    let hunter = req.hunter.as_deref().ok_or("no hunter")?;

    // The base witness the request pins.
    if !names(&req.base, d.base)? {
        return Err("the base witness is not the one the request pins".into());
    }
    let base = match Envelope::decode(d.base)?.payload {
        Payload::Base(b) => b,
        _ => return Err("the base witness is not a base witness".into()),
    };
    if let Some(want) = req.program_manifest_sha256.as_deref() {
        if sha256(&base.manifest_json) != hex32(want)? {
            return Err("the base witness does not carry this program's manifest".into());
        }
    }

    // The manifest the run is judged under, checked against the request's hash.
    let manifest: Bytes = match (&req.round_manifest, d.round_manifest, req.round) {
        (None, _, 0) => base.manifest_json.clone(),
        (Some(r), Some(m), _) if names(r, m)? => Bytes::copy_from_slice(m),
        _ => return Err("the round's manifest is missing, or not the one the request names".into()),
    };
    let manifest_sha256 = hex32(&req.manifest_sha256)?;
    if sha256(&manifest) != manifest_sha256 {
        return Err("manifest bytes do not match the committed hash".into());
    }

    // The sealed blob, opened with the disclosed key.
    let blob: Vec<u8> = match req.blob.as_ref().ok_or("no blob")? {
        BlobRef::Inline { inline } => hex::decode(inline).map_err(|_| "an inline blob that is not hex")?,
        r @ BlobRef::NearTx { .. } => {
            let b = d.blob.ok_or("the request names its blob by hash: supply the blob")?;
            if !names(r, b)? {
                return Err("the blob is not the one the request names".into());
            }
            b.to_vec()
        }
    };
    let blob_sha256 = sha256(&blob);
    let payload = sealed::open_with_key(&blob, &d.content_key, &req.program_key).map_err(|e| format!("the key does not open the blob: {e}"))?;
    if payload.program_key != req.program_key || payload.hunter != hunter {
        return Err("the sealed payload is bound to another program or hunter".into());
    }
    let witness_sha256 = sha256(&payload.envelope);
    let inner = Envelope::decode(&payload.envelope)?;

    // The witness the PoC runs on, assembled as the TEE assembles it: the manifest's own invariants.
    let invariants = Manifest::parse(&manifest)?.invariants.runtime_code;
    let w = match inner.payload.clone() {
        Payload::Program(mut w) => {
            w.manifest_json = manifest.clone();
            w.invariants_code = invariants;
            w
        }
        Payload::Submission(sub) => {
            let mut w = ProgramWitness::assemble(&base, &sub, manifest.clone())?;
            w.invariants_code = invariants;
            w
        }
        _ => return Err("the sealed witness is not a program witness".into()),
    };
    let envelope = Envelope { payload: Payload::Program(w.clone()), ..inner };
    let ctx = ProgramContext {
        program_id: req.program_key.clone(),
        commit_id,
        expected_manifest_hash: Some(B256::from(manifest_sha256)),
        reserved: Some(reserved_address(&req.program_key, envelope.claimed_block_hash)),
    };
    let receipt = run_program(&envelope, &w, witness_sha256, &ctx);
    let receipt_sha256 = receipt.hash();
    Ok(Rerun {
        request_sha256,
        blob_sha256,
        base_sha256: sha256(d.base),
        manifest_sha256,
        program_key: req.program_key,
        round: req.round,
        commit_id,
        run_id: req.run_id,
        receipt,
        receipt_sha256,
    })
}

/// A verdict's kind as one byte.
pub fn kind_byte(k: Kind) -> u8 {
    match k {
        Kind::Violation => 0,
        Kind::NoViolation => 1,
        Kind::InvalidPoC => 2,
        Kind::ProgramMisconfigured => 3,
        Kind::Unevaluable => 4,
        Kind::InfraError => 5,
    }
}

/// A broken invariant's severity as one byte, low to critical.
pub fn severity_byte(s: Severity) -> u8 {
    match s {
        Severity::Low => 0,
        Severity::Medium => 1,
        Severity::High => 2,
        Severity::Critical => 3,
    }
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    out.extend((s.len() as u16).to_be_bytes());
    out.extend(s.as_bytes());
}

impl Rerun {
    /// The verifier that ran, as the receipt names it.
    pub fn verifier(&self) -> String {
        let v = &self.receipt.verifier;
        format!("{} {} revm {} reth {} rsp {}", v.name, v.version, v.revm, v.reth, v.rsp)
    }

    /// The public values a validity proof commits (layout v1, integers big-endian):
    /// `u8 version | request_sha256 | receipt_sha256 | blob_sha256 | base_sha256 | manifest_sha256 |
    /// anchor block hash (32 bytes each) | u32 round | u64 commit | u64 run | u8 kind |
    /// u16 len, program key | u16 len, verifier | u8 n, n × (u8 slot, u8 severity)`.
    pub fn public_values(&self) -> Vec<u8> {
        let mut out = vec![PUBLIC_VALUES_V1];
        let anchor = self.receipt.anchor.as_ref().map(|a| a.block_hash).unwrap_or_default();
        for h in [self.request_sha256, self.receipt_sha256, self.blob_sha256, self.base_sha256, self.manifest_sha256, anchor.0] {
            out.extend(h);
        }
        out.extend(self.round.to_be_bytes());
        out.extend(self.commit_id.to_be_bytes());
        out.extend(self.run_id.to_be_bytes());
        out.push(kind_byte(self.receipt.kind));
        push_str(&mut out, &self.program_key);
        push_str(&mut out, &self.verifier());
        let broken = &self.receipt.broken[..self.receipt.broken.len().min(255)];
        out.push(broken.len() as u8);
        for f in broken {
            out.push(f.slot);
            out.push(severity_byte(f.severity));
        }
        out
    }
}

/// The public values, read back (the host prints them; a NEAR contract would read the same layout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicValues {
    pub request_sha256: [u8; 32],
    pub receipt_sha256: [u8; 32],
    pub blob_sha256: [u8; 32],
    pub base_sha256: [u8; 32],
    pub manifest_sha256: [u8; 32],
    pub anchor: [u8; 32],
    pub round: u32,
    pub commit_id: u64,
    pub run_id: u64,
    pub kind: u8,
    pub program_key: String,
    pub verifier: String,
    pub broken: Vec<(u8, u8)>,
}

pub fn decode_public_values(b: &[u8]) -> Result<PublicValues, String> {
    let mut i = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let s = b.get(i..i + n).ok_or("public values: truncated")?;
        i += n;
        Ok(s)
    };
    if take(1)?[0] != PUBLIC_VALUES_V1 {
        return Err("public values: not layout v1".into());
    }
    let mut h = || -> Result<[u8; 32], String> { Ok(take(32)?.try_into().unwrap()) };
    let (request_sha256, receipt_sha256, blob_sha256, base_sha256, manifest_sha256, anchor) = (h()?, h()?, h()?, h()?, h()?, h()?);
    let round = u32::from_be_bytes(take(4)?.try_into().unwrap());
    let commit_id = u64::from_be_bytes(take(8)?.try_into().unwrap());
    let run_id = u64::from_be_bytes(take(8)?.try_into().unwrap());
    let kind = take(1)?[0];
    let mut s = || -> Result<String, String> {
        let n = u16::from_be_bytes(take(2)?.try_into().unwrap()) as usize;
        String::from_utf8(take(n)?.to_vec()).map_err(|_| "public values: not utf-8".to_string())
    };
    let (program_key, verifier) = (s()?, s()?);
    let n = take(1)?[0] as usize;
    let mut broken = Vec::with_capacity(n);
    for _ in 0..n {
        let p = take(2)?;
        broken.push((p[0], p[1]));
    }
    if i != b.len() {
        return Err("public values: trailing bytes".into());
    }
    Ok(PublicValues { request_sha256, receipt_sha256, blob_sha256, base_sha256, manifest_sha256, anchor, round, commit_id, run_id, kind, program_key, verifier, broken })
}

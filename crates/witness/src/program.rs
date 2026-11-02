//! Program-mode witness builder (v0.4): trace the PoC through the verifier's own program path on a
//! fetching database at the anchor block, fetch proofs for everything it touched (including the
//! invariant evaluations), then iterate strict verification until the witness is complete.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

use alloy_primitives::{Address, Bytes, B256, U256};
use antibody_verifier::{
    bundle::{sender_address, Bundle},
    db::{MissingWitness, OverrideDb},
    execute_program_witness,
    facts::ProgramReceipt,
    manifest::Manifest,
    program::{anchor_checks, canaries_pass, classify, execute_program, ProgramError},
    run::{anchor_checks_on_witness, program_chain_spec, reserved_address},
    run_program, BaseWitness, Envelope, Payload, ProgramContext, ProgramWitness, QuorumVote, VerifyError, WITNESS_VERSION,
};
use eyre::{eyre, Result};
use revm::{primitives::KECCAK_EMPTY, state::Bytecode};
use rsp_mpt::EthereumState;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    build::{fetch_header, fetch_proofs_quorum, witness_stats, ProofQuorumReport, WitnessStats},
    fetchdb::{FetchingDb, Recorded, Seed},
    rpc::Pool,
};

#[derive(Debug, Clone, Serialize)]
pub struct ProgramBuildMeta {
    pub anchor_block: u64,
    pub anchor_hash: B256,
    pub senders: Vec<Address>,
    pub trace_verdict: Option<String>,
    pub trace_ms: u64,
    pub trace_rpc_reads: u64,
    pub fixpoint_passes: usize,
    pub fixpoint_added: Vec<String>,
    pub proof_quorum: Vec<ProofQuorumReport>,
    pub stats: WitnessStats,
    pub total_ms: u64,
}

/// `program_key` is the program's on-chain key: the verifier serves the invariants at the address it
/// derives from it, so the traced access set must use the same one.
pub fn build_program(
    pool: &Pool,
    manifest_json: &[u8],
    invariants_code: &Bytes,
    bundle: Bundle,
    min_agree: usize,
    program_key: &str,
) -> Result<(Vec<u8>, ProgramBuildMeta, ProgramReceipt)> {
    let t_all = Instant::now();
    let manifest = Manifest::parse(manifest_json).map_err(|e| eyre!(e))?;
    let m = manifest.anchor.block_number;
    let header = fetch_header(pool, m)?;
    let anchor_hash = header.hash_slow();
    if anchor_hash != manifest.anchor.block_hash {
        return Err(eyre!("provider header {anchor_hash} at {m} != manifest anchor {}", manifest.anchor.block_hash));
    }
    let spec = program_chain_spec(manifest.anchor.chain_id).map_err(|e| eyre!("{e}"))?;
    let inv = Bytecode::new_raw(invariants_code.clone());

    // Trace: the verifier's own program path on a fetching DB records the exact access set.
    let t_trace = Instant::now();
    let fdb = FetchingDb::new(pool.clone(), m, Seed::default());
    let reserved = Some(antibody_verifier::run::reserved_address(program_key, anchor_hash));
    let trace_verdict = match execute_program(&header, &manifest, &inv, &bundle, spec, &fdb, reserved) {
        Ok(_) => None,
        Err(ProgramError::Verdict { kind, reason }) => Some(format!("{kind:?}: {reason}")),
        Err(ProgramError::Verify(e)) => return Err(eyre!("trace failed: {e}")),
    };
    let trace_ms = t_trace.elapsed().as_millis() as u64;
    let rec: Recorded = fdb.rec.borrow().clone();
    let mut access: BTreeMap<Address, BTreeSet<U256>> = BTreeMap::new();
    for a in &rec.accounts {
        access.entry(*a).or_default();
    }
    for (a, s) in &rec.slots {
        access.entry(*a).or_default().extend(s.iter().copied());
    }
    let mut codes = rec.codes.clone();
    let mut block_numbers = rec.block_numbers.clone();
    let mut reports = Vec::new();
    let mut added = Vec::new();
    for pass in 1..=8usize {
        let (proofs, qr) = fetch_proofs_quorum(pool, m, &access, min_agree.min(pool.len()))?;
        reports.push(qr);
        let state = EthereumState::from_proofs(header.state_root, &proofs).map_err(|e| eyre!("from_proofs: {e}"))?;
        for p in proofs.values() {
            if let Some(info) = &p.info {
                let h = info.bytecode_hash.unwrap_or(KECCAK_EMPTY);
                if h != KECCAK_EMPTY && !codes.contains_key(&h) {
                    let v = pool[0].call("eth_getCode", json!([p.address, format!("{m:#x}")]))?;
                    let b: Bytes = serde_json::from_value(v)?;
                    let bc = Bytecode::new_raw(b);
                    codes.insert(bc.hash_slow(), bc);
                }
            }
        }
        let oldest = block_numbers.iter().next().copied().unwrap_or(m).min(m);
        let mut ancestors = Vec::new();
        for k in (oldest..m).rev() {
            ancestors.push(alloy_rlp::encode(fetch_header(pool, k)?).into());
        }
        let envelope = Envelope {
            version: WITNESS_VERSION,
            chain_id: manifest.anchor.chain_id,
            claimed_block_hash: anchor_hash,
            quorum: vec![QuorumVote { source: pool[0].label.clone(), block_hash: anchor_hash }],
            payload: Payload::Program(ProgramWitness {
                manifest_json: manifest_json.to_vec().into(),
                invariants_code: invariants_code.clone(),
                header_rlp: alloy_rlp::encode(&header).into(),
                ancestor_headers_rlp: ancestors,
                state,
                bytecodes: codes.values().cloned().collect(),
                bundle: bundle.clone(),
            }),
        };
        let bytes = envelope.encode();
        let Payload::Program(w) = &envelope.payload else { unreachable!() };
        match execute_program_witness(&envelope, w, None, reserved) {
            Ok(_) | Err(ProgramError::Verdict { .. }) => {
                let digest: [u8; 32] = Sha256::digest(&bytes).into();
                let receipt = run_program(&envelope, w, digest, &ProgramContext::default());
                let meta = ProgramBuildMeta {
                    anchor_block: m,
                    anchor_hash,
                    senders: (0..bundle.senders).map(|i| sender_address(&bundle.sender_salt, i)).collect(),
                    trace_verdict,
                    trace_ms,
                    trace_rpc_reads: rec.rpc_reads,
                    fixpoint_passes: pass,
                    fixpoint_added: added,
                    proof_quorum: reports,
                    stats: witness_stats(&envelope, &bytes, &proofs),
                    total_ms: t_all.elapsed().as_millis() as u64,
                };
                return Ok((bytes, meta, receipt));
            }
            Err(ProgramError::Verify(VerifyError::Incomplete(mw))) => {
                added.push(mw.to_string());
                match mw {
                    MissingWitness::Account(a) | MissingWitness::StorageTrie(a) => {
                        access.entry(a).or_default();
                    }
                    MissingWitness::Slot(a, s) => {
                        access.entry(a).or_default().insert(s);
                    }
                    MissingWitness::Code(h) => return Err(eyre!("code {h} missing after trace (pass {pass})")),
                    MissingWitness::BlockHash(b) => {
                        block_numbers.insert(b);
                    }
                }
            }
            Err(ProgramError::Verify(e)) => return Err(eyre!("strict verification failed (pass {pass}): {e}")),
        }
    }
    Err(eyre!("fixpoint not reached after 8 passes"))
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseBuildMeta {
    pub anchor_block: u64,
    pub anchor_hash: B256,
    pub trace_ms: u64,
    pub trace_rpc_reads: u64,
    pub fixpoint_passes: usize,
    pub fixpoint_added: Vec<String>,
    pub proof_quorum: Vec<ProofQuorumReport>,
    pub stats: WitnessStats,
    pub total_ms: u64,
}

/// The base witness a program pins at activation (docs/spec.md §4.4): the round-0 manifest, the
/// invariants, the anchor header, and every account, slot and code the activation checks read (the
/// reserved address, the targets' code hashes, `snapshot()`, `check(snapshot())`, the canaries),
/// proven against the anchor's state root. The verifier's own activation check passes on it.
pub fn build_base(pool: &Pool, manifest_json: &[u8], min_agree: usize, program_key: &str) -> Result<(Vec<u8>, BaseBuildMeta)> {
    build_base_for(pool, manifest_json, None, min_agree, program_key)
}

/// A base witness for a round: what the round-0 checks read and what the round's own checks read (its
/// code overrides, the invariants it adds), proven at the same anchor. It keeps the program's round-0
/// manifest, so the TEE still judges the round against it; near-antibody uses it for the round's runs.
pub fn build_round_base(
    pool: &Pool,
    manifest_json: &[u8],
    round_json: &[u8],
    min_agree: usize,
    program_key: &str,
) -> Result<(Vec<u8>, BaseBuildMeta)> {
    build_base_for(pool, manifest_json, Some(round_json), min_agree, program_key)
}

fn build_base_for(pool: &Pool, manifest_json: &[u8], round_json: Option<&[u8]>, min_agree: usize, program_key: &str) -> Result<(Vec<u8>, BaseBuildMeta)> {
    let t_all = Instant::now();
    let manifest = Manifest::parse(manifest_json).map_err(|e| eyre!(e))?;
    if manifest.round != 0 {
        return Err(eyre!("a base witness is built from the round-0 manifest"));
    }
    let round = match round_json {
        Some(r) => {
            let rm = Manifest::parse(r).map_err(|e| eyre!(e))?;
            if rm.round == 0 || rm.anchor != manifest.anchor {
                return Err(eyre!("a round manifest is round 1 or later, at the program's anchor"));
            }
            Some(rm)
        }
        None => None,
    };
    let m = manifest.anchor.block_number;
    let header = fetch_header(pool, m)?;
    let anchor_hash = header.hash_slow();
    if anchor_hash != manifest.anchor.block_hash {
        return Err(eyre!("provider header {anchor_hash} at {m} != manifest anchor {}", manifest.anchor.block_hash));
    }
    let spec = program_chain_spec(manifest.anchor.chain_id).map_err(|e| eyre!("{e}"))?;
    let inv = Bytecode::new_raw(manifest.invariants.runtime_code.clone());
    let reserved = Some(reserved_address(program_key, anchor_hash));

    // Trace: the verifier's activation checks on a fetching DB record the exact access set.
    let t_trace = Instant::now();
    let fdb = FetchingDb::new(pool.clone(), m, Seed::default());
    match anchor_checks(&header, &manifest, &inv, spec.clone(), &fdb, reserved).and_then(|a| canaries_pass(&manifest, &a, &fdb)) {
        Ok(true) => {}
        Ok(false) => return Err(eyre!("a canary fails at the anchor")),
        Err(e) => {
            let (kind, reason) = classify(&e);
            return Err(eyre!("the program fails its checks at the anchor: {kind:?}: {reason}"));
        }
    }
    // The round's checks, with its overrides and invariants, on the same fetching DB.
    if let Some(rm) = &round {
        let odb = OverrideDb::new(&fdb, rm.overrides.iter().map(|o| (o.address, Bytecode::new_raw(o.runtime_code.clone()))));
        let rinv = Bytecode::new_raw(rm.invariants.runtime_code.clone());
        match anchor_checks(&header, rm, &rinv, spec.clone(), &odb, reserved).and_then(|a| canaries_pass(rm, &a, &odb)) {
            Ok(true) => {}
            Ok(false) => return Err(eyre!("a canary fails at the anchor under the round's code")),
            Err(e) => {
                let (kind, reason) = classify(&e);
                return Err(eyre!("the round fails its checks at the anchor: {kind:?}: {reason}"));
            }
        }
    }
    let trace_ms = t_trace.elapsed().as_millis() as u64;
    let rec: Recorded = fdb.rec.borrow().clone();
    let mut access: BTreeMap<Address, BTreeSet<U256>> = BTreeMap::new();
    for a in &rec.accounts {
        access.entry(*a).or_default();
    }
    for (a, s) in &rec.slots {
        access.entry(*a).or_default().extend(s.iter().copied());
    }
    // Every target's ERC-1967 implementation slot, set or not, so a round can check its releases
    // against the anchor (spec §9).
    let implementation_slot = U256::from_be_bytes(antibody_verifier::run::EIP1967_IMPLEMENTATION_SLOT.0);
    for t in &manifest.targets {
        access.entry(t.address).or_default().insert(implementation_slot);
    }
    let mut codes = rec.codes.clone();
    let mut block_numbers = rec.block_numbers.clone();
    let mut reports = Vec::new();
    let mut added = Vec::new();
    for pass in 1..=8usize {
        let (proofs, qr) = fetch_proofs_quorum(pool, m, &access, min_agree.min(pool.len()))?;
        reports.push(qr);
        let state = EthereumState::from_proofs(header.state_root, &proofs).map_err(|e| eyre!("from_proofs: {e}"))?;
        for p in proofs.values() {
            if let Some(info) = &p.info {
                let h = info.bytecode_hash.unwrap_or(KECCAK_EMPTY);
                if h != KECCAK_EMPTY && !codes.contains_key(&h) {
                    let v = pool[0].call("eth_getCode", json!([p.address, format!("{m:#x}")]))?;
                    let b: Bytes = serde_json::from_value(v)?;
                    let bc = Bytecode::new_raw(b);
                    codes.insert(bc.hash_slow(), bc);
                }
            }
        }
        let oldest = block_numbers.iter().next().copied().unwrap_or(m).min(m);
        let mut ancestors = Vec::new();
        for k in (oldest..m).rev() {
            ancestors.push(alloy_rlp::encode(fetch_header(pool, k)?).into());
        }
        let base = BaseWitness {
            manifest_json: manifest_json.to_vec().into(),
            invariants_code: manifest.invariants.runtime_code.clone(),
            header_rlp: alloy_rlp::encode(&header).into(),
            ancestor_headers_rlp: ancestors,
            state,
            bytecodes: codes.values().cloned().collect(),
        };
        // What the verifier checks at activation: the base as a program witness with no PoC.
        let full = ProgramWitness {
            manifest_json: base.manifest_json.clone(),
            invariants_code: base.invariants_code.clone(),
            header_rlp: base.header_rlp.clone(),
            ancestor_headers_rlp: base.ancestor_headers_rlp.clone(),
            state: base.state.clone(),
            bytecodes: base.bytecodes.clone(),
            bundle: Bundle::default(),
        };
        let envelope = Envelope {
            version: WITNESS_VERSION,
            chain_id: manifest.anchor.chain_id,
            claimed_block_hash: anchor_hash,
            quorum: vec![QuorumVote { source: pool[0].label.clone(), block_hash: anchor_hash }],
            payload: Payload::Base(base),
        };
        // The round's checks on the same proven state: its manifest, the base's state.
        let checked = anchor_checks_on_witness(&envelope, &full, reserved).and_then(|ok| match (&round, round_json) {
            (Some(rm), Some(rj)) if ok => anchor_checks_on_witness(
                &envelope,
                &ProgramWitness { manifest_json: rj.to_vec().into(), invariants_code: rm.invariants.runtime_code.clone(), ..full.clone() },
                reserved,
            ),
            _ => Ok(ok),
        });
        match checked {
            Ok(true) => {
                let bytes = envelope.encode();
                let meta = BaseBuildMeta {
                    anchor_block: m,
                    anchor_hash,
                    trace_ms,
                    trace_rpc_reads: rec.rpc_reads,
                    fixpoint_passes: pass,
                    fixpoint_added: added,
                    proof_quorum: reports,
                    stats: witness_stats(&envelope, &bytes, &proofs),
                    total_ms: t_all.elapsed().as_millis() as u64,
                };
                return Ok((bytes, meta));
            }
            Ok(false) => return Err(eyre!("a canary fails on the proven anchor state")),
            Err(ProgramError::Verify(VerifyError::Incomplete(mw))) => {
                added.push(mw.to_string());
                match mw {
                    MissingWitness::Account(a) | MissingWitness::StorageTrie(a) => {
                        access.entry(a).or_default();
                    }
                    MissingWitness::Slot(a, s) => {
                        access.entry(a).or_default().insert(s);
                    }
                    MissingWitness::Code(h) => return Err(eyre!("code {h} missing after trace (pass {pass})")),
                    MissingWitness::BlockHash(b) => {
                        block_numbers.insert(b);
                    }
                }
            }
            Err(e) => {
                let (kind, reason) = classify(&e);
                return Err(eyre!("strict activation checks failed (pass {pass}): {kind:?}: {reason}"));
            }
        }
    }
    Err(eyre!("fixpoint not reached after 8 passes"))
}

/// `antibody.toml` -> canonical manifest JSON (the bytes whose sha256 the project commits).
pub fn compile_manifest(toml_src: &str) -> Result<(Vec<u8>, B256)> {
    let m: Manifest = toml::from_str(toml_src).map_err(|e| eyre!("antibody.toml: {e}"))?;
    let bytes = m.canonical();
    Manifest::parse(&bytes).map_err(|e| eyre!(e))?;
    Ok((bytes.clone(), Manifest::hash(&bytes)))
}

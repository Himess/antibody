//! The part of a v2 manifest near-antibody reads (docs/spec.md §3.2): the program's identity, its
//! anchor, its invariant slots and the `settlement` section (typed in `antibody_rules::settlement`).
//! The manifest arrives as its canonical JSON text; its SHA-256 is the program's manifest hash. The
//! verifier enforces everything else.

use antibody_rules::{settlement::Settlement, Params, Severity};
use near_sdk::{near, serde_json};

#[near(serializers = [json])]
pub struct ManifestIn {
    pub schema: String,
    pub program: MProgram,
    pub anchor: MAnchor,
    pub targets: Vec<MTarget>,
    pub invariants: MInvariants,
    pub settlement: Option<Settlement>,
}

#[near(serializers = [json])]
pub struct MProgram {
    pub name: String,
    pub project: String,
    pub description: String,
}

#[near(serializers = [json])]
pub struct MAnchor {
    pub chain_id: u64,
    pub block_number: u64,
    pub block_hash: String,
    pub evm_spec: String,
}

#[near(serializers = [json])]
pub struct MTarget {
    pub name: String,
    pub address: String,
    pub code_hash: String,
}

#[near(serializers = [json])]
pub struct MInvariants {
    pub slots: Vec<MSlot>,
    pub gas_limit: u64,
}

#[near(serializers = [json])]
pub struct MSlot {
    pub bit: u8,
    pub name: String,
    pub severity: Severity,
}

/// What `create_program` keeps from a manifest.
pub struct Parsed {
    pub manifest: ManifestIn,
    pub params: Params,
    pub terms_sha256: String,
}

/// Parses a v2 manifest. `escrow` sizes slots without an explicit budget; `run_deposit` comes from the
/// program's pinned verifier configuration.
pub fn parse(text: &str, escrow: u128, run_deposit: u128) -> Result<Parsed, String> {
    if text.len() > 200_000 {
        return Err("manifest too large".into());
    }
    let m: ManifestIn = serde_json::from_str(text).map_err(|e| format!("manifest: {e}"))?;
    if m.schema != "antibody.manifest.v2" {
        return Err("manifest: schema must be antibody.manifest.v2".into());
    }
    if m.program.name.len() > 128 || m.program.description.len() > 2048 || m.targets.len() > 32 {
        return Err("manifest: a field is too long".into());
    }
    if !antibody_rules::settlement::is_hex(&m.anchor.block_hash, 32) {
        return Err("manifest: anchor block hash".into());
    }
    let s = m.settlement.as_ref().ok_or("manifest: no settlement section")?;
    // The verifier declares bits 0..=254 only (its slot mask): bit 255 can never be broken.
    let slots: Vec<(u8, Severity)> = m.invariants.slots.iter().filter(|s| s.bit != 255).map(|s| (s.bit, s.severity)).collect();
    let params = s.params(&slots, escrow, run_deposit)?;
    let terms_sha256 = s.terms_sha256.trim_start_matches("0x").to_string();
    Ok(Parsed { manifest: m, params, terms_sha256 })
}

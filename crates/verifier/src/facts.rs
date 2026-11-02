//! Program-mode outputs. The full receipt (v2) stays sealed with the PoC until disclosure; the TEE
//! publishes only the facts (≤ 1 KB): verdict kind, broken-invariant mask, HMAC'd root-cause
//! fingerprint tags, impact and hashes. Rules (keys, priority, payouts) are applied on NEAR.

use alloy_primitives::{Address, B256};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{bundle::CallOutcome, manifest::Severity, receipt::VerifierInfo};

pub const RECEIPT_V2_SCHEMA: &str = "antibody.receipt.v2";
pub const FACTS_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    /// At least one committed invariant broke.
    Violation,
    /// The PoC ran and every invariant still holds (profit alone is not a bug).
    NoViolation,
    /// The submission is malformed, exceeds the committed capabilities, or its witness does not
    /// prove the anchor state (the hunter's fault; bond forfeited).
    InvalidPoC,
    /// The program itself is broken at the anchor (invariants fail before any PoC, reserved
    /// address in use, target code mismatch).
    ProgramMisconfigured,
    /// Neither party's fault (verifier/EVM failure, invariant evaluation failed after the PoC).
    ExecutionError,
}

/// Root cause of one broken invariant: the protocol-entry call (target + selector) at whose end
/// the invariant last flipped from holding to broken. `target == 0` means unattributed (the flip
/// happened outside any protocol-entry call).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fingerprint {
    pub slot: u8,
    pub name: String,
    pub severity: Severity,
    pub target: Address,
    pub target_name: Option<String>,
    pub selector: String,
    /// Index of the evaluation point (protocol-entry call end) where the last flip happened.
    pub flip_point: Option<u32>,
    pub tx_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountValue {
    pub account: Address,
    pub role: String,
    pub eth_delta_wei: String,
    pub tokens: Vec<(String, String)>,
    pub usd_micro: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpactV2 {
    /// L_P: loss of the manifest's protocol accounts at committed reference prices.
    pub protocol_loss_usd_micro: String,
    /// G_A: gain of the attacker closure (senders + created contracts), excluding the endowment
    /// and gas fees, at committed reference prices.
    pub attacker_gain_usd_micro: String,
    /// I = min(L_P, G_A). Sizes the payout only; validity is the invariant break.
    pub impact_usd_micro: String,
    pub impact_usd: String,
    pub accounts: Vec<AccountValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgramAnchor {
    pub chain_id: u64,
    pub block_number: u64,
    pub block_hash: B256,
    pub state_root: B256,
    pub timestamp: u64,
    pub evm_spec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgramChecks {
    pub header_hash_bound: bool,
    pub state_root_matches: bool,
    pub storage_tries_verified: usize,
    pub bytecodes_supplied: usize,
    pub ancestor_headers_linked: usize,
    pub reserved_address_empty: bool,
    pub target_code_hashes_match: bool,
    pub anchor_invariants_hold: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgramReceipt {
    pub schema: &'static str,
    pub verifier: VerifierInfo,
    pub program: String,
    pub commit_id: u64,
    pub manifest_hash: B256,
    pub invariants_code_hash: B256,
    pub anchor: Option<ProgramAnchor>,
    pub witness_sha256: String,
    pub poc_hash: String,
    pub kind: Kind,
    pub reason: Option<String>,
    pub checks: Option<ProgramChecks>,
    pub senders: Vec<Address>,
    pub endowment_wei_per_sender: String,
    pub calls: Vec<CallOutcome>,
    pub created_contracts: Vec<Address>,
    pub total_gas_used: u64,
    /// Number of invariant evaluations at protocol-entry call ends.
    pub evaluations: u32,
    pub broken_mask: String,
    pub broken: Vec<Fingerprint>,
    pub impact: Option<ImpactV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FpTag {
    pub slot: u8,
    pub tag: String,
}

/// Public output of a TEE run (≤ 1 KB). Facts only; the NEAR contract applies the rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Facts {
    pub v: u8,
    pub program: String,
    pub commit_id: u64,
    pub poc_hash: String,
    pub kind: Kind,
    pub broken_mask: String,
    pub fp_tags: Vec<FpTag>,
    pub impact_usd_micro: String,
    pub project_wrap: Option<String>,
    pub receipt_hash: String,
    pub manifest_hash: String,
    pub wasm_hash: Option<String>,
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// HMAC-SHA256 (RFC 2104).
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new().chain_update(ipad).chain_update(msg).finalize();
    Sha256::new().chain_update(opad).chain_update(inner).finalize().into()
}

/// Per-program fingerprint key, derived inside the TEE from its protected secret.
pub fn derive_k_program(secret: &[u8], program_id: &str) -> [u8; 32] {
    let mut m = b"antibody/k_program/v1".to_vec();
    m.extend_from_slice(program_id.as_bytes());
    hmac_sha256(secret, &m)
}

/// `fp_tag = HMAC(k_program, "antibody/fp/v1" ‖ slot ‖ target ‖ selector)`: equal tags prove
/// "same rule, same function" without revealing which function until `k_program` is disclosed.
pub fn fp_tag(k_program: &[u8; 32], slot: u8, target: Address, selector: [u8; 4]) -> [u8; 32] {
    let mut m = b"antibody/fp/v1".to_vec();
    m.push(slot);
    m.extend_from_slice(target.as_slice());
    m.extend_from_slice(&selector);
    hmac_sha256(k_program, &m)
}

pub fn parse_selector(s: &str) -> [u8; 4] {
    let mut out = [0u8; 4];
    if let Ok(b) = hex::decode(s.trim_start_matches("0x")) {
        if b.len() == 4 {
            out.copy_from_slice(&b);
        }
    }
    out
}

impl ProgramReceipt {
    /// Canonical bytes: compact JSON in struct order. The receipt hash is sha256 of these bytes.
    pub fn canonical(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("receipt serializes")
    }

    pub fn hash(&self) -> [u8; 32] {
        sha256(&self.canonical())
    }

    pub fn facts(&self, k_program: Option<&[u8; 32]>, project_wrap: Option<String>, wasm_hash: Option<String>) -> Facts {
        let fp_tags = match (self.kind, k_program) {
            (Kind::Violation, Some(k)) => self
                .broken
                .iter()
                .map(|f| FpTag { slot: f.slot, tag: hex::encode(fp_tag(k, f.slot, f.target, parse_selector(&f.selector))) })
                .collect(),
            _ => Vec::new(),
        };
        Facts {
            v: FACTS_VERSION,
            program: self.program.clone(),
            commit_id: self.commit_id,
            poc_hash: self.poc_hash.clone(),
            kind: self.kind,
            broken_mask: self.broken_mask.clone(),
            fp_tags,
            impact_usd_micro: self.impact.as_ref().map(|i| i.impact_usd_micro.clone()).unwrap_or_else(|| "0".into()),
            project_wrap,
            receipt_hash: hex::encode(self.hash()),
            manifest_hash: hex::encode(self.manifest_hash),
            wasm_hash,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_rfc4231_case2() {
        let tag = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex::encode(tag), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }
}

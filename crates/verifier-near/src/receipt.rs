//! The receipt of a NEAR run: v1's program receipt (the `antibody.receipt.v2` schema string v1's program
//! mode writes, the same field names in the same order where they apply) with NEAR's types, `chain_id`
//! 398, and an added `near` section. Canonical bytes are compact JSON in struct order; the receipt hash is
//! their sha256, byte-identical on every platform.

use serde::Serialize;

use crate::{manifest::Severity, runtime::ReceiptRecord};

/// The schema string v1's program receipts carry.
pub const RECEIPT_SCHEMA: &str = "antibody.receipt.v2";

/// The facts' verdict kinds, named as v1 names them.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum Kind {
    Violation,
    NoViolation,
    InvalidPoC,
    ProgramMisconfigured,
    Unevaluable,
    InfraError,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VerifierInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub engine: &'static str,
    pub near_protocol: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Anchor {
    pub chain_id: u64,
    pub block_number: u64,
    pub block_hash: String,
    pub timestamp_ns: u64,
    pub evm_spec: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Checks {
    /// Every target's committed sha256 matched its wasm, and every module was accepted.
    pub target_code_hashes_match: bool,
    /// The rules held after deployment, init and setup (the anchor of a fork-mode program).
    pub anchor_rules_hold: bool,
    /// The doubled-cost run agreed on every observable (`false` when it was not run).
    pub doubled_gas_run_identical: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CallOutcome {
    pub index: usize,
    pub sender: String,
    pub receiver: String,
    pub method: String,
    /// The call's own receipt succeeded (its receipt tree may still hold failures).
    pub success: bool,
    /// Receipts its tree executed.
    pub receipts: usize,
    pub gas_burnt: u64,
}

/// The root cause of a broken slot: the top-level call after which it last flipped from holding to broken.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Fingerprint {
    pub slot: u8,
    pub name: String,
    pub severity: Severity,
    /// The entry point: the call's receiver and method.
    pub receiver: String,
    pub method: String,
    /// The committed custom group of that entry point, if any.
    pub group: Option<u16>,
    pub flip_point: Option<u32>,
    pub call_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AccountValue {
    pub account: String,
    pub role: String,
    pub near_delta_yocto: String,
    pub tokens: Vec<(String, String)>,
    pub usd_micro: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Impact {
    pub protocol_loss_usd_micro: String,
    pub attacker_gain_usd_micro: String,
    pub impact_usd_micro: String,
    pub impact_usd: String,
    pub accounts: Vec<AccountValue>,
    pub protected_harm_usd_micro: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NearBlockInfo {
    pub height: u64,
    pub timestamp_ns: u64,
    pub epoch_height: u64,
    pub random_seed: String,
    pub gas_price: String,
}

/// What only a NEAR receipt has: the fixed block context and every receipt's outcome.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NearSection {
    pub protocol: u32,
    pub block: Option<NearBlockInfo>,
    /// Wasm instructions the bundle's calls ran (wasmi fuel).
    pub instructions: u64,
    pub receipts: Vec<ReceiptRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NearReceipt {
    pub schema: &'static str,
    pub verifier: VerifierInfo,
    pub program: String,
    pub commit_id: u64,
    pub manifest_hash: String,
    pub rules_code_hash: String,
    pub anchor: Option<Anchor>,
    pub witness_sha256: String,
    pub poc_hash: String,
    pub kind: Kind,
    pub reason: Option<String>,
    pub checks: Option<Checks>,
    pub senders: Vec<String>,
    pub endowment_yocto_per_sender: String,
    pub calls: Vec<CallOutcome>,
    pub created_accounts: Vec<String>,
    pub total_gas_used: u64,
    pub evaluations: u32,
    pub broken_mask: String,
    pub broken: Vec<Fingerprint>,
    pub impact: Option<Impact>,
    pub near: NearSection,
}

impl NearReceipt {
    pub fn canonical(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("receipt serializes")
    }

    pub fn hash(&self) -> [u8; 32] {
        crate::util::sha256(&self.canonical())
    }
}

/// The group bytes of a NEAR root cause (docs/spec.md §7.2, §11.2): a committed group's id as v1 encodes
/// it (`'g' ‖ u16_be(id)`), or the entry point `'n' ‖ u32_be(len) ‖ receiver ‖ u32_be(len) ‖ method`.
/// The `'n'` prefix keeps NEAR entry points apart from v1's `'f' ‖ address ‖ selector`.
pub fn group_bytes(custom: Option<u16>, receiver: &str, method: &str) -> Vec<u8> {
    match custom {
        Some(id) => {
            let mut v = vec![b'g'];
            v.extend_from_slice(&id.to_be_bytes());
            v
        }
        None => {
            let mut v = vec![b'n'];
            v.extend_from_slice(&(receiver.len() as u32).to_be_bytes());
            v.extend_from_slice(receiver.as_bytes());
            v.extend_from_slice(&(method.len() as u32).to_be_bytes());
            v.extend_from_slice(method.as_bytes());
            v
        }
    }
}

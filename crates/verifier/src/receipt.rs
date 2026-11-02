//! The receipt: deterministic, re-checkable output of a verification run.

use alloy_primitives::{Address, B256};
use serde::Serialize;

use crate::witness::QuorumVote;

pub const RECEIPT_SCHEMA: &str = "antibody.receipt.v1";

#[derive(Debug, Clone, Serialize)]
pub struct Receipt {
    pub schema: &'static str,
    pub verifier: VerifierInfo,
    pub mode: &'static str,
    pub chain_id: u64,
    /// Environment of execution, taken only from the verified header.
    pub env: EnvInfo,
    /// The header whose state root anchors the proven pre-state.
    pub state_anchor: AnchorInfo,
    /// Host-reported quorum observations of `env.block_hash` (not checkable inside the verifier).
    pub quorum: Vec<QuorumVote>,
    /// sha256 of the exact witness bytes the verifier consumed.
    pub witness_sha256: String,
    /// keccak256 over the concatenated hashes of the executed transactions.
    pub bundle_commitment: B256,
    /// keccak256 of the valuation manifest the verifier used (price feeds, fallback oracle, watched
    /// / protected accounts). The NEAR contract must compare it with the round's committed manifest.
    pub manifest_hash: B256,
    pub checks: Checks,
    pub target: Option<TargetOutcome>,
    pub impact: Option<Impact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle: Option<crate::bundle::BundleOutcome>,
    pub access: AccessStats,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifierInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub revm: &'static str,
    pub reth: &'static str,
    pub rsp: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvInfo {
    pub block_number: u64,
    pub block_hash: B256,
    pub timestamp: u64,
    pub base_fee: Option<u64>,
    pub prevrandao: B256,
    pub coinbase: Address,
    pub gas_limit: u64,
    pub excess_blob_gas: Option<u64>,
    pub spec: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnchorInfo {
    pub block_number: u64,
    pub block_hash: B256,
    pub state_root: B256,
}

#[derive(Debug, Clone, Serialize)]
pub struct Checks {
    pub header_hash_bound: bool,
    pub transactions_root_bound: bool,
    pub ancestor_headers_linked: usize,
    pub state_root_matches: bool,
    pub storage_tries_verified: usize,
    pub bytecodes_supplied: usize,
    /// Block mode only: post-state root recomputed and equal to the header's.
    pub post_state_root_matches: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetOutcome {
    pub index: u32,
    pub tx_hash: B256,
    pub from: Address,
    pub to: Option<Address>,
    pub success: bool,
    pub gas_used: u64,
    pub cumulative_gas_used: u64,
    pub logs: usize,
    /// keccak256(RLP(logs)) — compare with the on-chain receipt's logs.
    pub logs_keccak: B256,
    pub accounts_changed: usize,
    pub slots_changed: usize,
    /// keccak256 over the sorted state diff (see `diff` for the full list when requested).
    pub diff_keccak: B256,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<StateDiff>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct StateDiff {
    pub accounts: Vec<AccountChange>,
    pub slots: Vec<SlotChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountChange {
    pub address: Address,
    pub balance_before: String,
    pub balance_after: String,
    pub nonce_before: u64,
    pub nonce_after: u64,
    pub code_changed: bool,
    pub destroyed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlotChange {
    pub address: Address,
    pub slot: B256,
    pub before: B256,
    pub after: B256,
}

#[derive(Debug, Clone, Serialize)]
pub struct Impact {
    pub accounts: Vec<AccountImpact>,
    pub prices: Vec<PriceObservation>,
    /// Gas fee paid by the target transaction's sender (wei), for the "excluding fees" view.
    pub sender_fee_wei: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountImpact {
    pub account: Address,
    pub eth_delta_wei: String,
    pub eth_delta_excluding_fee_wei: String,
    pub tokens: Vec<TokenDelta>,
    /// Sum over priced assets, in micro-USD (integer), plus a display string.
    pub usd_micro_total: String,
    pub usd_total: String,
    pub unpriced_assets: Vec<Address>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenDelta {
    pub token: Address,
    pub decimals: Option<u8>,
    /// balanceOf(account) after − before, executed on the verified state.
    pub delta_by_balance_of: String,
    /// Sum of Transfer/Deposit/Withdrawal logs (cross-check).
    pub delta_by_logs: String,
    pub agree: bool,
    pub usd_micro: Option<String>,
    pub usd: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PriceObservation {
    pub token: Address,
    pub feed: Address,
    pub answer: String,
    pub feed_decimals: u8,
    pub updated_at: u64,
    pub age_seconds: i64,
    /// Older than MAX_PRICE_AGE_SECONDS (or non-positive answer): not used for valuation.
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccessStats {
    pub accounts_read: usize,
    pub slots_read: usize,
    pub codes_read: usize,
    pub block_hashes_read: usize,
}

/// Failure classes. Distinguishing them matters for economics: a tampered witness is the
/// submitter's/host's fault, an incomplete witness is an infra failure (refund), a revert is a
/// valid "no impact" outcome.
#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub schema: &'static str,
    pub ok: bool,
    pub class: &'static str,
    pub error: String,
}

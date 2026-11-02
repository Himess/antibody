//! Data types stored by near-antibody and exchanged with the verifier (facts) and clients.

use near_sdk::{json_types::U128, near, AccountId};

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Verdict kind produced by the verifier (see docs/crypto-spec.md §9).
#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Violation,
    NoViolation,
    InvalidPoC,
    ProgramMisconfigured,
    ExecutionError,
}

/// The pinned verifier build and how near-antibody invokes it on OutLayer.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct VerifierConfig {
    /// https URL of the wasm32-wasip2 component (OutLayer `WasmUrl`).
    pub wasm_url: String,
    /// Lowercase hex SHA-256 of the exact wasm bytes.
    pub wasm_hash: String,
    /// X25519 public key of the sealed-submission key (hex), from a `pubkey` run of this build.
    pub pubkey: Option<String>,
    /// OutLayer `secrets_ref` (owner account + profile) holding `PROTECTED_ANTIBODY_SEED`.
    pub secrets_owner: Option<AccountId>,
    pub secrets_profile: Option<String>,
    pub max_instructions: u64,
    pub max_memory_mb: u32,
    pub max_execution_seconds: u32,
    /// yoctoNEAR attached to each `request_execution` (unused part is refunded by OutLayer).
    pub deposit: U128,
    /// Attestation tier of verdicts produced through this configuration ("T0", "T1").
    pub tier: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub chain_id: u64,
    pub block_number: u64,
    /// 0x-prefixed, lowercase.
    pub block_hash: String,
    pub evm_spec: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum AnchorTrust {
    /// Light-client check in flight.
    Pending,
    /// `block_hash_safe(M) == H` on the NEAR Ethereum light client at creation.
    LightClient { client: AccountId, checked_at_height: u64 },
    /// Not checkable on NEAR (outside the light-client window, unsupported chain, local chain).
    ProjectAttested { reason: String },
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct SlotRule {
    pub bit: u8,
    pub name: String,
    pub severity: Severity,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct TargetInfo {
    pub name: String,
    pub address: String,
    pub code_hash: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct PayoutTable {
    pub critical: U128,
    pub high: U128,
    pub medium: U128,
    pub low: U128,
    /// Optional impact bonus in basis points of I (micro-USD = raw units at 6 decimals), capped.
    pub impact_bps: u32,
    pub impact_cap: U128,
    pub decimals: u8,
}

impl PayoutTable {
    pub fn base(&self, s: Severity) -> u128 {
        match s {
            Severity::Critical => self.critical.0,
            Severity::High => self.high.0,
            Severity::Medium => self.medium.0,
            Severity::Low => self.low.0,
        }
    }
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum ProgramStatus {
    /// Created; anchor check pending.
    Created,
    /// Known-issues phase (t0): the project may submit known issues; hunters cannot commit yet.
    KnownIssues,
    Open,
    /// The anchor contradicted the light client; escrow refunded.
    Rejected { reason: String },
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct DisclosedKey {
    pub commit_id: u64,
    pub content_key: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Disclosure {
    pub k_program: String,
    pub keys: Vec<DisclosedKey>,
    pub height: u64,
    pub ns: u64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub id: u32,
    /// Globally unique key "<contract>/<id>" used by the verifier (k_program, facts).
    pub key: String,
    pub name: String,
    pub project: AccountId,
    pub manifest_hash: String,
    pub manifest_url: String,
    pub anchor: Anchor,
    pub anchor_trust: AnchorTrust,
    pub targets: Vec<TargetInfo>,
    pub invariants_code_hash: String,
    pub slots: Vec<SlotRule>,
    pub payout: PayoutTable,
    pub bond: U128,
    pub reveal_window_blocks: u64,
    pub deadline_ns: u64,
    pub disclosure_after_ns: u64,
    pub instant_cap: U128,
    pub challenge_delay_ns: u64,
    pub verifier: VerifierConfig,
    pub project_pubkey: String,
    pub guardian: Option<AccountId>,
    pub status: ProgramStatus,
    pub created_height: u64,
    pub created_ns: u64,
    pub opened_ns: Option<u64>,
    pub escrow: U128,
    pub paid: U128,
    pub reserved: U128,
    pub escrow_withdrawn: bool,
    pub commits: Vec<u64>,
    pub pause_requested: bool,
    pub misconfigured: bool,
    pub disclosure_pending: bool,
    pub disclosure: Option<Disclosure>,
}

impl Program {
    pub fn severity(&self, bit: u8) -> Option<Severity> {
        self.slots.iter().find(|s| s.bit == bit).map(|s| s.severity)
    }
    pub fn available(&self) -> u128 {
        self.escrow.0.saturating_sub(self.paid.0).saturating_sub(self.reserved.0)
    }
}

/// Pointer to a sealed submission (docs/crypto-spec.md §4).
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct SealedRef {
    pub url: String,
    /// SHA-256 of the blob, lowercase hex.
    pub sha256: String,
    /// The 92-byte key wrap to the verifier, lowercase hex (stored for the disclosure run).
    pub wrap_v: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum CommitStatus {
    Committed,
    Verifying,
    VerifyFailed,
    Verified,
    Expired,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct FindingKey {
    pub slot: u8,
    pub tag: String,
    pub severity: Severity,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub kind: Kind,
    pub broken_mask: String,
    pub keys: Vec<FindingKey>,
    pub impact_usd_micro: U128,
    pub receipt_hash: String,
    pub poc_hash: String,
    pub project_wrap: Option<String>,
    pub wasm_hash: Option<String>,
    pub tier: String,
    pub verified_height: u64,
    pub verified_ns: u64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Not yet verified.
    None,
    /// Verified Violation, waiting for `finalize` (reveal windows of earlier commits).
    Pending,
    Paid { amount: U128, severity: Severity, height: u64, ns: u64 },
    /// T0 optimistic delay: above the instant cap, payable after `until_ns` unless challenged.
    Delayed { amount: U128, severity: Severity, until_ns: u64 },
    Challenged { amount: U128, severity: Severity, by: AccountId, reason: String },
    PayoutFailed { amount: U128, severity: Severity },
    /// Every key was already claimed by an earlier finder: no payout, bond refunded.
    Duplicate { of: u64 },
    /// Every key was claimed by the project's declared known issues at t0: no payout, bond refunded.
    Known { of: u64 },
    /// The project's own known-issue PoC (t0 claim).
    KnownIssue,
    /// NoViolation or InvalidPoC: bond forfeited.
    Rejected { kind: Kind },
    /// Never revealed within the window: bond forfeited.
    Forfeited,
    /// Not the hunter's fault (ProgramMisconfigured, ExecutionError, malformed facts): bond refunded.
    Error { reason: String },
    /// A challenge was upheld by the arbiter: payout cancelled, logged.
    Cancelled { reason: String },
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    pub id: u64,
    pub program: u32,
    pub hunter: AccountId,
    pub commit_hash: String,
    pub height: u64,
    pub ns: u64,
    pub bond: U128,
    pub known_issue: bool,
    pub status: CommitStatus,
    pub sealed: Option<SealedRef>,
    pub submitted_height: Option<u64>,
    pub submitted_ns: Option<u64>,
    pub verify_attempts: u8,
    pub verdict: Option<Verdict>,
    pub outcome: Outcome,
    pub bonus: U128,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Claim {
    pub commit: u64,
    pub known: bool,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reputation {
    pub commits: u32,
    pub valid_critical: u32,
    pub valid_high: u32,
    pub valid_medium: u32,
    pub valid_low: u32,
    /// Valid but late (duplicates).
    pub late: u32,
    /// Valid but already declared by the project.
    pub known: u32,
    /// NoViolation / InvalidPoC.
    pub slop: u32,
    pub unrevealed: u32,
    pub earned: U128,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub programs: u32,
    pub commits: u64,
    pub verdicts: u64,
    pub violations: u64,
    pub paid_findings: u64,
    pub duplicates: u64,
    pub known: u64,
    pub slop_rejected: u64,
    pub escrow_total: U128,
    pub paid_total: U128,
    /// Sum and count of (verdict block height - submit block height), for the median/mean view.
    pub verdict_latency_blocks_sum: u64,
    pub verdict_latency_samples: u64,
}

/// Facts as emitted by the verifier inside the TEE (crates/verifier/src/facts.rs).
#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct FactsIn {
    pub v: u8,
    pub program: String,
    pub commit_id: u64,
    pub poc_hash: String,
    pub kind: Kind,
    pub broken_mask: String,
    pub fp_tags: Vec<FpTagIn>,
    pub impact_usd_micro: String,
    pub project_wrap: Option<String>,
    pub receipt_hash: String,
    pub manifest_hash: String,
    pub wasm_hash: Option<String>,
}

#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct FpTagIn {
    pub slot: u8,
    pub tag: String,
}

/// Output of a `pubkey` run.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct PubkeyIn {
    pub pubkey: String,
}

/// Output of a `disclose` run.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct DisclosureIn {
    pub program: String,
    pub k_program: String,
    pub keys: Vec<DisclosedKey>,
}

/// `msg` of `ft_transfer_call` into near-antibody.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TokenMsg {
    /// Credit the sender's internal balance (bonds come from it).
    Deposit {},
    /// Open a program: `manifest` is the canonical manifest JSON (its sha256 is committed).
    CreateProgram { manifest: String, manifest_url: String, project_pubkey: String, guardian: Option<AccountId> },
    FundProgram { program_id: u32 },
    /// Voluntary bonus to the finder of `commit_id` (projects can pay more, never less).
    TopUp { commit_id: u64, note: Option<String> },
}

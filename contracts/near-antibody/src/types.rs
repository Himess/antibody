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
    /// Set for a re-run blob (rounds); `None` for the commit's own submission.
    #[serde(default)]
    pub round: Option<u32>,
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
    /// Rounds after the first (docs/rounds.md); `rounds[i]` is round `i + 1`.
    pub rounds: Vec<Round>,
    /// Latest proposed round: new commits verify against its manifest.
    pub round: u32,
    /// Latest open round: its claims (carried and new) are settled, so its commits can finalize.
    pub open_round: u32,
}

impl Program {
    pub fn severity(&self, bit: u8) -> Option<Severity> {
        self.slots.iter().find(|s| s.bit == bit).map(|s| s.severity)
    }
    pub fn available(&self) -> u128 {
        self.escrow.0.saturating_sub(self.paid.0).saturating_sub(self.reserved.0)
    }
    /// The manifest hash a commit of `round` is verified against.
    pub fn manifest_hash_of(&self, round: u32) -> &str {
        match round {
            0 => &self.manifest_hash,
            r => &self.rounds[r as usize - 1].manifest_hash,
        }
    }
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum RoundStatus {
    /// The fix is committed; claimed findings are being re-run against it. New commits already
    /// verify against this round's manifest but settle only once it opens.
    Proposed,
    /// Carried and released keys are settled.
    Open,
}

/// A fix committed as code overrides at the program's anchor (round manifest).
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    pub index: u32,
    pub manifest_hash: String,
    pub manifest_url: String,
    /// Overridden targets and the code hash of the fix.
    pub overrides: Vec<TargetInfo>,
    pub status: RoundStatus,
    pub proposed_height: u64,
    pub proposed_ns: u64,
    pub opened_ns: Option<u64>,
    /// Keys of the previous round carried into this one (still broken at the fix, or not re-run).
    pub carried: u32,
    /// Keys whose PoCs no longer break at the fix: open again for new findings in this round.
    pub released: u32,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum RerunStatus {
    Requested,
    /// No usable result (no output, or a re-run witness the TEE rejected): retry with a new blob.
    Failed { reason: String },
    Done,
}

/// A claimed finding re-run by the TEE at a later round's state.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Rerun {
    pub round: u32,
    pub sealed: SealedRef,
    pub requested_by: AccountId,
    pub attempts: u8,
    pub status: RerunStatus,
    pub kind: Option<Kind>,
    /// Keys the PoC still breaks at the round's state; every other key it held is released.
    pub still_broken: Vec<FindingKey>,
    pub receipt_hash: Option<String>,
    pub height: u64,
    pub input_sha256: Option<String>,
    pub output_sha256: Option<String>,
    pub tee: Option<TeeConfirmation>,
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
    /// The program round this commit belongs to (its manifest decides the verdict).
    pub round: u32,
    pub reruns: Vec<Rerun>,
    /// SHA-256 of the exact `input_data` sent to OutLayer and of the exact bytes it returned: the
    /// values a TDX quote's report_data must bind for a T1 confirmation.
    pub run_input_sha256: Option<String>,
    pub run_output_sha256: Option<String>,
    pub tee: Option<TeeConfirmation>,
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

// ------------------------------------------------------------------------ T1 (TDX quotes)

/// OutLayer's per-execution attestation (`testnet-api.outlayer.ai/attestations/by-request/{id}`).
/// SHA-256 over these fields, in this order, is the first half of the TDX quote's report_data
/// (OutLayer `worker/src/tdx_attestation.rs`, report data layout V1).
#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct TeeAttestationIn {
    pub task_type: String,
    pub task_id: i64,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    pub build_target: Option<String>,
    pub wasm_hash: Option<String>,
    pub input_hash: Option<String>,
    pub output_hash: String,
    pub block_height: Option<u64>,
    pub caller_account_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    pub secrets_ref: Option<String>,
    pub timestamp: i64,
    pub attached_usd: Option<String>,
}

impl TeeAttestationIn {
    pub fn task_hash(&self) -> Vec<u8> {
        let mut m: Vec<u8> = Vec::new();
        m.extend(self.task_type.as_bytes());
        m.extend(self.task_id.to_le_bytes());
        for s in [&self.repo, &self.commit, &self.build_target, &self.wasm_hash, &self.input_hash].into_iter().flatten() {
            m.extend(s.as_bytes());
        }
        m.extend(self.output_hash.as_bytes());
        if let Some(h) = self.block_height {
            m.extend(h.to_le_bytes());
        }
        for s in [&self.caller_account_id, &self.project_id, &self.secrets_ref].into_iter().flatten() {
            m.extend(s.as_bytes());
        }
        m.extend(self.timestamp.to_le_bytes());
        if let Some(u) = &self.attached_usd {
            m.extend(u.as_bytes());
        }
        near_sdk::env::sha256(&m)
    }
}

/// One approved worker image (OutLayer's `get_approved_measurements` entry), lowercase hex.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct TeeMeasurements {
    pub mrtd: String,
    pub rtmr0: String,
    pub rtmr1: String,
    pub rtmr2: String,
    pub rtmr3: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct TeeConfig {
    /// DCAP verifier contract (`verify_quote`), e.g. `tee-verifier-2026-07-22.testnet`.
    pub verifier: Option<AccountId>,
    /// Worker images whose quotes count (pinned from OutLayer's allowlist by the owner).
    pub measurements: Vec<TeeMeasurements>,
    /// Accepted TCB statuses (dcap-qvl), e.g. "UpToDate".
    pub accepted_tcb: Vec<String>,
}

impl Default for TeeConfig {
    fn default() -> Self {
        Self { verifier: None, measurements: Vec::new(), accepted_tcb: vec!["UpToDate".into()] }
    }
}

/// A verdict whose OutLayer run was proven by a TDX quote verified on NEAR (tier T1).
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct TeeConfirmation {
    pub quote_sha256: String,
    pub report_data: String,
    pub tcb_status: String,
    pub mrtd: String,
    pub rtmr3: String,
    pub outlayer_task_id: i64,
    pub verifier: AccountId,
    pub height: u64,
    pub ns: u64,
}

// Borsh mirrors of near/mpc `tee-verifier-interface` (the DCAP verifier's return value).

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum VerificationResultIn {
    Verified(VerifiedReportIn),
    Rejected(VerifierErrorIn),
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub enum VerifierErrorIn {
    DcapVerification(String),
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct VerifiedReportIn {
    pub status: String,
    pub advisory_ids: Vec<String>,
    pub report: ReportIn,
    pub ppid: Vec<u8>,
    pub qe_status: TcbStatusWithAdvisoryIn,
    pub platform_status: TcbStatusWithAdvisoryIn,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ReportIn {
    SgxEnclave(EnclaveReportIn),
    TD10(TDReport10In),
    TD15(TDReport15In),
    TD15Ex(TDReport15ExIn),
}

impl ReportIn {
    pub fn as_td10(&self) -> Option<&TDReport10In> {
        match self {
            ReportIn::TD10(r) => Some(r),
            ReportIn::TD15(r) => Some(&r.base),
            ReportIn::TD15Ex(r) => Some(&r.base.base),
            ReportIn::SgxEnclave(_) => None,
        }
    }
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct TDReport10In {
    pub tee_tcb_svn: [u8; 16],
    pub mr_seam: [u8; 48],
    pub mr_signer_seam: [u8; 48],
    pub seam_attributes: [u8; 8],
    pub td_attributes: [u8; 8],
    pub xfam: [u8; 8],
    pub mr_td: [u8; 48],
    pub mr_config_id: [u8; 48],
    pub mr_owner: [u8; 48],
    pub mr_owner_config: [u8; 48],
    pub rt_mr0: [u8; 48],
    pub rt_mr1: [u8; 48],
    pub rt_mr2: [u8; 48],
    pub rt_mr3: [u8; 48],
    pub report_data: [u8; 64],
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct TDReport15In {
    pub base: TDReport10In,
    pub tee_tcb_svn2: [u8; 16],
    pub mr_service_td: [u8; 48],
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct TDReport15ExIn {
    pub base: TDReport15In,
    pub vmid: u8,
    pub td_id: [u8; 32],
    pub dev_info: [u8; 48],
    pub init_service_td_hash: [u8; 48],
    pub init_service_td_attributes: [u8; 8],
    pub init_cpu_svn: [u8; 16],
    pub init_tee_tcb_svn: [u8; 16],
    pub init_tee_fmspc: [u8; 12],
    pub cur_service_td_hash: [u8; 48],
    pub cur_service_td_attributes: [u8; 8],
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct EnclaveReportIn {
    pub cpu_svn: [u8; 16],
    pub misc_select: u32,
    pub reserved1: [u8; 28],
    pub attributes: [u8; 16],
    pub mr_enclave: [u8; 32],
    pub reserved2: [u8; 32],
    pub mr_signer: [u8; 32],
    pub reserved3: [u8; 96],
    pub isv_prod_id: u16,
    pub isv_svn: u16,
    pub reserved4: [u8; 60],
    pub report_data: [u8; 64],
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub enum TcbStatusIn {
    UpToDate,
    OutOfDateConfigurationNeeded,
    OutOfDate,
    ConfigurationAndSWHardeningNeeded,
    ConfigurationNeeded,
    SWHardeningNeeded,
    Revoked,
    TDRelaunchAdvised,
    TDRelaunchAdvisedConfigurationNeeded,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct TcbStatusWithAdvisoryIn {
    pub status: TcbStatusIn,
    pub advisory_ids: Vec<String>,
}

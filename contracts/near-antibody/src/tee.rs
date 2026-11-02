//! T1: a run's TDX quote, verified on NEAR (docs/spec.md §6.1).
//!
//! OutLayer publishes one attestation per execution (`testnet-api.outlayer.ai/attestations/by-request/{id}`).
//! SHA-256 over its fields, in OutLayer's order (`worker/src/tdx_attestation.rs`), is the first half of
//! the quote's report_data. near-antibody recomputes it from its own record of the run, has NEAR's DCAP
//! verifier check the quote, and accepts only the program's pinned worker images.

use near_sdk::{env, near};

/// OutLayer's per-execution attestation.
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
        env::sha256(&m)
    }
}

/// One approved worker image (OutLayer's `get_approved_measurements` entry), lowercase hex.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measurements {
    pub mrtd: String,
    pub rtmr0: String,
    pub rtmr1: String,
    pub rtmr2: String,
    pub rtmr3: String,
}

/// A worker image the deployment knows, and since when (spec §15: additions reach live programs only
/// after `image_delay`).
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownImage {
    pub m: Measurements,
    pub added_at: u64,
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

/// What near-antibody recorded about a run: the attestation must describe exactly this.
pub struct RunRecord<'a> {
    pub wasm_hash: &'a str,
    pub request_sha256: String,
    pub output_sha256: String,
    pub signer: &'a str,
}

/// Checks that an attestation describes this run (spec §6.1). The caller is the transaction signer that
/// requested the run, or near-antibody itself for a relayed meta-transaction.
pub fn check_binding(a: &TeeAttestationIn, r: &RunRecord) -> Result<(), &'static str> {
    if a.task_type != "execute" {
        return Err("attestation: not an execution");
    }
    if a.build_target.as_deref() != Some("wasm32-wasip2") {
        return Err("attestation: build target");
    }
    if a.wasm_hash.as_deref() != Some(r.wasm_hash) {
        return Err("attestation: not the program's pinned verifier");
    }
    if a.input_hash.as_deref() != Some(r.request_sha256.as_str()) {
        return Err("attestation: input differs from what near-antibody sent");
    }
    if a.output_hash != r.output_sha256 {
        return Err("attestation: output differs from what near-antibody received");
    }
    let caller = a.caller_account_id.as_deref();
    let contract = env::current_account_id();
    if !(caller == Some(r.signer) || caller == Some(contract.as_str())) {
        return Err("attestation: another caller");
    }
    // v2 runs use no secret row: keys come from the build's own encryption key (spec §11.1).
    if a.secrets_ref.is_some() {
        return Err("attestation: a run with secrets is not a v2 run");
    }
    if a.repo.is_some() || a.commit.is_some() || a.project_id.is_some() {
        return Err("attestation: not a WasmUrl run");
    }
    Ok(())
}

pub fn measurements_of(td: &TDReport10In) -> Measurements {
    Measurements {
        mrtd: hex::encode(td.mr_td),
        rtmr0: hex::encode(td.rt_mr0),
        rtmr1: hex::encode(td.rt_mr1),
        rtmr2: hex::encode(td.rt_mr2),
        rtmr3: hex::encode(td.rt_mr3),
    }
}

pub fn dcap_verifier_args(quote: &[u8], collateral: &[u8]) -> Vec<u8> {
    let mut args = Vec::with_capacity(quote.len() + collateral.len() + 4);
    args.extend((quote.len() as u32).to_le_bytes());
    args.extend(quote);
    args.extend(collateral);
    args
}


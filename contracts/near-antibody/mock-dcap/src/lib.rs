//! MOCK of NEAR's DCAP quote verifier (`verify_quote`), for the offline sandbox demo only (never
//! deployed to testnet).
//!
//! The sandbox has no TDX hardware. A MOCK quote is JSON naming the worker image's measurements and
//! the report_data; this contract reports it as verified, in the real verifier's borsh return type.
//! near-antibody then runs its own T1 checks on the answer (the run binding, the TCB status, the
//! program's pinned images), so the sandbox exercises the same code path as testnet. What it does not
//! exercise is the quote's signature chain up to Intel's roots.

use near_sdk::{
    borsh::{BorshDeserialize, BorshSerialize},
    env, near,
    serde_json::{self, Value},
    PanicOnDefault,
};

// Borsh mirrors of near/mpc `tee-verifier-interface` (as in near-antibody's `tee.rs`).

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
#[borsh(crate = "near_sdk::borsh")]
#[allow(clippy::large_enum_variant)]
pub enum VerificationResult {
    Verified(VerifiedReport),
    Rejected(VerifierError),
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
#[borsh(crate = "near_sdk::borsh")]
pub enum VerifierError {
    DcapVerification(String),
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
#[borsh(crate = "near_sdk::borsh")]
pub struct VerifiedReport {
    pub status: String,
    pub advisory_ids: Vec<String>,
    pub report: Report,
    pub ppid: Vec<u8>,
    pub qe_status: TcbStatusWithAdvisory,
    pub platform_status: TcbStatusWithAdvisory,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
#[borsh(crate = "near_sdk::borsh")]
#[allow(clippy::large_enum_variant)]
pub enum Report {
    SgxEnclave(EnclaveReport),
    TD10(TDReport10),
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
#[borsh(crate = "near_sdk::borsh")]
pub struct TDReport10 {
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

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
#[borsh(crate = "near_sdk::borsh")]
pub struct EnclaveReport {
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

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
#[borsh(crate = "near_sdk::borsh")]
pub enum TcbStatus {
    UpToDate,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
#[borsh(crate = "near_sdk::borsh")]
pub struct TcbStatusWithAdvisory {
    pub status: TcbStatus,
    pub advisory_ids: Vec<String>,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockDcap {
    verified: u64,
}

fn bytes<const N: usize>(v: &Value, k: &str) -> Result<[u8; N], String> {
    let h = v[k].as_str().ok_or_else(|| format!("MOCK quote: no {k}"))?;
    let b = hex_decode(h).ok_or_else(|| format!("MOCK quote: {k} is not hex"))?;
    b.try_into().map_err(|_| format!("MOCK quote: {k} must be {N} bytes"))
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// A MOCK quote: `{"mock": "...", "mrtd", "rtmr0".."rtmr3", "report_data"}` (hex).
fn parse(quote: &[u8]) -> Result<TDReport10, String> {
    let v: Value = serde_json::from_slice(quote).map_err(|_| "not a MOCK quote".to_string())?;
    if !v["mock"].is_string() {
        return Err("not a MOCK quote".into());
    }
    Ok(TDReport10 {
        tee_tcb_svn: [0; 16],
        mr_seam: [0; 48],
        mr_signer_seam: [0; 48],
        seam_attributes: [0; 8],
        td_attributes: [0; 8],
        xfam: [0; 8],
        mr_td: bytes(&v, "mrtd")?,
        mr_config_id: [0; 48],
        mr_owner: [0; 48],
        mr_owner_config: [0; 48],
        rt_mr0: bytes(&v, "rtmr0")?,
        rt_mr1: bytes(&v, "rtmr1")?,
        rt_mr2: bytes(&v, "rtmr2")?,
        rt_mr3: bytes(&v, "rtmr3")?,
        report_data: bytes(&v, "report_data")?,
    })
}

#[near]
impl MockDcap {
    #[init]
    pub fn new() -> Self {
        Self { verified: 0 }
    }

    /// Same input as the real verifier: `u32_le(len(quote)) ‖ quote ‖ borsh(collateral)`.
    #[result_serializer(borsh)]
    pub fn verify_quote(&mut self) -> VerificationResult {
        let input = env::input().unwrap_or_default();
        let n = input.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).unwrap_or(usize::MAX);
        let parsed = input.get(4..4usize.saturating_add(n)).ok_or_else(|| "truncated input".to_string()).and_then(parse);
        match parsed {
            Ok(td) => {
                self.verified += 1;
                let ok = || TcbStatusWithAdvisory { status: TcbStatus::UpToDate, advisory_ids: vec![] };
                VerificationResult::Verified(VerifiedReport {
                    status: "UpToDate".into(),
                    advisory_ids: vec![],
                    report: Report::TD10(td),
                    ppid: vec![],
                    qe_status: ok(),
                    platform_status: ok(),
                })
            }
            Err(e) => VerificationResult::Rejected(VerifierError::DcapVerification(e)),
        }
    }

    pub fn get_verified(&self) -> u64 {
        self.verified
    }
}

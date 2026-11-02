//! The part of the program manifest (docs: crates/verifier/src/manifest.rs) the contract applies.
//! The manifest is passed as its canonical JSON text; its sha256 is the committed manifest hash.
//! Unknown fields (impact, capabilities) are ignored here: the verifier enforces them.

use near_sdk::{near, require};

use crate::types::Severity;

pub const MANIFEST_SCHEMA: &str = "antibody.manifest.v1";

#[near(serializers = [json])]
pub struct ManifestIn {
    pub schema: String,
    pub program: MProgram,
    pub anchor: MAnchor,
    pub targets: Vec<MTarget>,
    pub invariants: MInvariants,
    pub payout: MPayout,
    pub rules: MRules,
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
    pub trust: String,
}

#[near(serializers = [json])]
pub struct MTarget {
    pub name: String,
    pub address: String,
    pub code_hash: String,
}

#[near(serializers = [json])]
pub struct MInvariants {
    pub address: String,
    pub runtime_code_hash: String,
    pub gas_limit: u64,
    pub slots: Vec<MSlot>,
}

#[near(serializers = [json])]
pub struct MSlot {
    pub bit: u8,
    pub name: String,
    pub severity: Severity,
}

#[near(serializers = [json])]
pub struct MPayout {
    pub token: String,
    pub decimals: u8,
    pub critical: String,
    pub high: String,
    pub medium: String,
    pub low: String,
    pub impact_bps: u32,
    pub impact_cap: String,
}

#[near(serializers = [json])]
pub struct MRules {
    pub bond: String,
    pub reveal_window_blocks: u64,
    pub disclosure_after_secs: u64,
    pub deadline_unix: u64,
    pub instant_payout_cap: String,
    pub challenge_delay_secs: u64,
}

pub fn parse_amount(s: &str, what: &str) -> u128 {
    s.parse::<u128>().unwrap_or_else(|_| near_sdk::env::panic_str(&format!("manifest: bad amount for {what}")))
}

pub fn is_hex(s: &str, bytes: usize) -> bool {
    s.len() == bytes * 2 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn is_0x_hex(s: &str, bytes: usize) -> bool {
    s.len() == 2 + bytes * 2 && s.starts_with("0x") && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

impl ManifestIn {
    pub fn validate(&self) {
        require!(self.schema == MANIFEST_SCHEMA, "manifest: unsupported schema");
        require!(
            self.anchor.chain_id == 11155111 || self.anchor.chain_id == 31337,
            "manifest: anchor chain must be Sepolia (11155111) or a local chain (31337)"
        );
        require!(is_0x_hex(&self.anchor.block_hash, 32), "manifest: anchor.block_hash");
        require!(!self.targets.is_empty() && self.targets.len() <= 32, "manifest: 1..=32 targets");
        for t in &self.targets {
            require!(is_0x_hex(&t.address, 20) && is_0x_hex(&t.code_hash, 32), "manifest: target address/code_hash");
        }
        require!(is_0x_hex(&self.invariants.runtime_code_hash, 32), "manifest: invariants.runtime_code_hash");
        let s = &self.invariants.slots;
        require!(!s.is_empty() && s.len() <= 64, "manifest: 1..=64 invariant slots");
        for (i, a) in s.iter().enumerate() {
            require!(a.bit < 255, "manifest: slot bit < 255");
            require!(s[i + 1..].iter().all(|b| b.bit != a.bit), "manifest: duplicate slot bit");
        }
        require!(self.rules.reveal_window_blocks >= 1, "manifest: reveal window");
        for (v, w) in [
            (&self.payout.critical, "critical"),
            (&self.payout.high, "high"),
            (&self.payout.medium, "medium"),
            (&self.payout.low, "low"),
            (&self.payout.impact_cap, "impact_cap"),
            (&self.rules.bond, "bond"),
            (&self.rules.instant_payout_cap, "instant_payout_cap"),
        ] {
            parse_amount(v, w);
        }
    }
}

//! The part of the program manifest (docs: crates/verifier/src/manifest.rs) the contract applies.
//! The manifest is passed as its canonical JSON text; its sha256 is the committed manifest hash.
//! Unknown fields (impact, capabilities) are ignored here: the verifier enforces them.

use near_sdk::{
    env, near, require,
    serde_json::{self, Value},
};

use crate::types::{Severity, TargetInfo};

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

fn decode_0x(s: &str) -> Option<Vec<u8>> {
    let h = s.strip_prefix("0x")?;
    if h.len() % 2 != 0 {
        return None;
    }
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok()).collect()
}

/// Largest override accepted (matches the verifier).
pub const MAX_OVERRIDE_CODE: usize = 49_152;

/// Checks a round manifest against the program's own (base) manifest: it must be the base manifest
/// plus `round` and `overrides`, with each overridden target carrying the fix's code hash, and
/// nothing else changed (rules, payouts, invariants, anchor and capabilities stay committed).
/// Returns the overridden targets with their new code hashes.
pub fn check_round(base: &str, manifest: &str, round: u32) -> Vec<TargetInfo> {
    let b: Value = serde_json::from_str(base).unwrap_or_else(|e| env::panic_str(&format!("base manifest: {e}")));
    let mut r: Value = serde_json::from_str(manifest).unwrap_or_else(|e| env::panic_str(&format!("round manifest: {e}")));
    require!(r["round"].as_u64() == Some(round as u64), "round manifest: wrong round number");
    let overrides = r["overrides"].as_array().cloned().unwrap_or_default();
    require!(!overrides.is_empty(), "round manifest: no overrides");
    let obj = r.as_object_mut().unwrap_or_else(|| env::panic_str("round manifest: not an object"));
    obj.remove("round");
    obj.remove("overrides");
    let targets = r["targets"].as_array().cloned().unwrap_or_default();
    let base_targets = b["targets"].as_array().cloned().unwrap_or_default();
    require!(targets.len() == base_targets.len(), "round manifest: targets differ");
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for o in &overrides {
        let addr = o["address"].as_str().unwrap_or_default().to_lowercase();
        let hash = o["code_hash"].as_str().unwrap_or_default().to_lowercase();
        let code = decode_0x(o["runtime_code"].as_str().unwrap_or_default())
            .unwrap_or_else(|| env::panic_str("round manifest: override runtime_code"));
        require!(is_0x_hex(&addr, 20) && is_0x_hex(&hash, 32), "round manifest: override address/code_hash");
        require!(!code.is_empty() && code.len() <= MAX_OVERRIDE_CODE, "round manifest: override code size");
        require!(format!("0x{}", crate::hex(&env::keccak256(&code))) == hash, "round manifest: override code does not match its hash");
        require!(!seen.contains(&addr), "round manifest: duplicate override");
        seen.push(addr.clone());
        let i = targets
            .iter()
            .position(|t| t["address"].as_str().unwrap_or_default().to_lowercase() == addr)
            .unwrap_or_else(|| env::panic_str("round manifest: override is not a target"));
        require!(targets[i]["code_hash"].as_str().unwrap_or_default().to_lowercase() == hash, "round manifest: target must carry the override hash");
        out.push(TargetInfo { name: targets[i]["name"].as_str().unwrap_or_default().to_string(), address: addr, code_hash: hash });
        // Normalize back to the base code hash for the equality check below.
        r["targets"][i]["code_hash"] = base_targets[i]["code_hash"].clone();
    }
    require!(r == b, "round manifest: only target code may change in a round");
    out
}

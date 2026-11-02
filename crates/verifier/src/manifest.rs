//! Program manifest: a program's rules as code. `antibody.toml` (human source) compiles to this
//! canonical JSON; sha256 of the exact JSON bytes is the manifest hash the project commits on NEAR.
//! The verifier uses the execution-relevant parts (anchor, targets, invariants, impact config,
//! attacker capabilities); the NEAR contract applies the rest (severities, payouts, bond, windows).

use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MANIFEST_SCHEMA: &str = "antibody.manifest.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema: String,
    pub program: ProgramInfo,
    pub anchor: Anchor,
    pub targets: Vec<Target>,
    pub invariants: Invariants,
    pub impact: ImpactConfig,
    pub capabilities: Capabilities,
    pub payout: Payout,
    pub rules: Rules,
    /// Rounds (docs/rounds.md): 0 is the program as opened. A round-N manifest is the program's
    /// manifest with the project's fix committed as code overrides at the same anchor. Both fields
    /// are omitted in round 0, so round-0 manifests keep their exact bytes and hash.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub round: u32,
    /// Runtime code that replaces the anchor's code at these target addresses in this round. Each
    /// overridden target's `code_hash` is the override's hash; storage stays the anchor's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<CodeOverride>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeOverride {
    pub address: Address,
    pub code_hash: B256,
    pub runtime_code: Bytes,
}

/// Largest override accepted (twice EIP-170's limit; the verifier itself has no hard limit).
pub const MAX_OVERRIDE_CODE: usize = 49_152;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgramInfo {
    pub name: String,
    /// NEAR account of the project that opens the program.
    pub project: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Anchor {
    pub chain_id: u64,
    pub block_number: u64,
    pub block_hash: B256,
    /// EVM rules the PoC runs under ("osaka"). Committed, so a later chain fork cannot change a
    /// program's semantics; the state itself is proven against `block_hash`.
    pub evm_spec: String,
    /// "light-client" (checked on NEAR at creation) or "project-attested".
    pub trust: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    pub address: Address,
    /// keccak256 of the runtime code at the anchor (checked against the proven state).
    pub code_hash: B256,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Invariants {
    /// Reserved address where the runtime code is injected (must be empty at the anchor).
    pub address: Address,
    /// The runtime bytecode itself: the committed manifest carries the rules as code.
    pub runtime_code: alloy_primitives::Bytes,
    pub runtime_code_hash: B256,
    /// Gas cap for each `snapshot()` / `check()` evaluation.
    pub gas_limit: u64,
    pub slots: Vec<Slot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Slot {
    pub bit: u8,
    pub name: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpactConfig {
    /// Accounts whose losses count as protocol loss L_P.
    pub protocol_accounts: Vec<Address>,
    /// Committed reference price of native ETH, decimal USD string.
    pub native_usd: String,
    pub tokens: Vec<PricedToken>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PricedToken {
    pub symbol: String,
    pub address: Address,
    pub decimals: u8,
    /// Committed reference price per whole token, decimal USD string.
    pub usd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Capabilities {
    pub max_senders: u8,
    pub max_endowment_wei: String,
    pub max_txs: u32,
    pub max_gas_per_tx: u64,
    pub max_total_gas: u64,
    /// All transactions run in the anchor block's environment (v1 supports 1).
    pub max_blocks: u32,
    pub max_timestamp_delta: u64,
    /// Bound on protocol-entry calls (each one triggers an invariant evaluation).
    pub max_entry_calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Payout {
    /// NEP-141 token account on NEAR.
    pub token: String,
    pub decimals: u8,
    /// Raw token units per severity.
    pub critical: String,
    pub high: String,
    pub medium: String,
    pub low: String,
    /// Optional impact-scaled bonus in basis points of I (USD at reference prices), capped.
    pub impact_bps: u32,
    pub impact_cap: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rules {
    /// Raw payout-token units locked per commit.
    pub bond: String,
    /// NEAR blocks a commit has to reveal (submit) its PoC.
    pub reveal_window_blocks: u64,
    /// Seconds after the deadline when sealed PoCs become public (disclosure run).
    pub disclosure_after_secs: u64,
    /// Unix seconds: no commits after this.
    pub deadline_unix: u64,
    /// T0: payouts up to this many raw units are instant; above it an optimistic delay applies.
    pub instant_payout_cap: String,
    pub challenge_delay_secs: u64,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let m: Manifest = serde_json::from_slice(bytes).map_err(|e| format!("manifest: {e}"))?;
        if m.schema != MANIFEST_SCHEMA {
            return Err(format!("manifest: unsupported schema {}", m.schema));
        }
        m.check_overrides()?;
        Ok(m)
    }

    /// Round rules: round 0 has no overrides; a later round overrides at least one target, each
    /// override's hash matches its code, and the overridden target carries that hash.
    pub fn check_overrides(&self) -> Result<(), String> {
        if (self.round == 0) != self.overrides.is_empty() {
            return Err("manifest: overrides are required in rounds > 0 and not allowed in round 0".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for o in &self.overrides {
            if !seen.insert(o.address) {
                return Err(format!("manifest: duplicate override for {}", o.address));
            }
            if o.runtime_code.is_empty() || o.runtime_code.len() > MAX_OVERRIDE_CODE {
                return Err(format!("manifest: override code for {} is empty or too large", o.address));
            }
            if keccak256(&o.runtime_code) != o.code_hash {
                return Err(format!("manifest: override code for {} does not match its hash", o.address));
            }
            match self.targets.iter().find(|t| t.address == o.address) {
                Some(t) if t.code_hash == o.code_hash => {}
                Some(_) => return Err(format!("manifest: target {} must carry its override's code hash", o.address)),
                None => return Err(format!("manifest: override {} is not a target", o.address)),
            }
        }
        Ok(())
    }

    /// The round-`round` manifest for a fix: the base manifest with `code` replacing each listed
    /// target's runtime code at the same anchor. Nothing else changes (the contract enforces it).
    pub fn with_overrides(&self, round: u32, code: &[(Address, Bytes)]) -> Result<Manifest, String> {
        if self.round != 0 || !self.overrides.is_empty() {
            return Err("manifest: derive rounds from the round-0 manifest".into());
        }
        let mut m = self.clone();
        m.round = round;
        for (a, c) in code {
            let h = keccak256(c);
            let t = m.targets.iter_mut().find(|t| t.address == *a).ok_or_else(|| format!("manifest: {a} is not a target"))?;
            t.code_hash = h;
            m.overrides.push(CodeOverride { address: *a, code_hash: h, runtime_code: c.clone() });
        }
        m.check_overrides()?;
        Ok(m)
    }

    /// sha256 of the exact canonical bytes (what the project commits on NEAR).
    pub fn hash(bytes: &[u8]) -> B256 {
        B256::from_slice(&Sha256::digest(bytes))
    }

    /// Canonical encoding: compact serde JSON in struct field order.
    pub fn canonical(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("manifest serializes")
    }

    pub fn slot(&self, bit: u8) -> Option<&Slot> {
        self.invariants.slots.iter().find(|s| s.bit == bit)
    }

    pub fn target_name(&self, a: Address) -> Option<&str> {
        self.targets.iter().find(|t| t.address == a).map(|t| t.name.as_str())
    }
}

/// Parses a non-negative decimal USD string ("2000", "1.5") into micro-USD.
pub fn usd_to_micro(s: &str) -> Result<U256, String> {
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if int.is_empty() || frac.len() > 6 || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return Err(format!("bad USD amount {s:?}"));
    }
    let int: U256 = int.parse().map_err(|_| format!("bad USD amount {s:?}"))?;
    let frac_padded = format!("{frac:0<6}");
    let frac: U256 = frac_padded.parse().map_err(|_| format!("bad USD amount {s:?}"))?;
    Ok(int * U256::from(1_000_000u64) + frac)
}

pub fn parse_u128(s: &str, what: &str) -> Result<u128, String> {
    s.parse::<u128>().map_err(|_| format!("bad {what} {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Manifest {
        Manifest::parse(include_bytes!("../../../fixtures/acme-local/manifest.json")).unwrap()
    }

    #[test]
    fn round_zero_bytes_are_unchanged() {
        let raw = include_bytes!("../../../fixtures/acme-local/manifest.json");
        assert_eq!(base().canonical(), raw.to_vec(), "new optional fields must not change round-0 manifests");
    }

    #[test]
    fn round_manifests() {
        let b = base();
        let lend = b.targets.iter().find(|t| t.name == "AcmeLend").unwrap().address;
        let code = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xf3]);
        let r1 = b.with_overrides(1, &[(lend, code.clone())]).unwrap();
        assert_eq!(r1.round, 1);
        assert_eq!(r1.targets.iter().find(|t| t.address == lend).unwrap().code_hash, keccak256(&code));
        let again = Manifest::parse(&r1.canonical()).unwrap();
        assert_eq!(again, r1);
        assert!(r1.with_overrides(2, &[]).is_err(), "rounds derive from round 0");
        assert!(b.with_overrides(1, &[(Address::ZERO, code.clone())]).is_err(), "only targets");
        assert!(b.with_overrides(1, &[]).is_err(), "a round overrides something");
        let mut bad = r1.clone();
        bad.overrides[0].runtime_code = Bytes::from(vec![0x00]);
        assert!(bad.check_overrides().is_err(), "hash must match the code");
    }

    #[test]
    fn usd_parsing() {
        assert_eq!(usd_to_micro("2000").unwrap(), U256::from(2_000_000_000u64));
        assert_eq!(usd_to_micro("1.5").unwrap(), U256::from(1_500_000u64));
        assert!(usd_to_micro("-1").is_err());
        assert!(usd_to_micro("1.1234567").is_err());
    }
}

//! Program manifest: a program's rules as code. `antibody.toml` (human source) compiles to this
//! canonical JSON; sha256 of the exact JSON bytes is the manifest hash the project commits on NEAR.
//! The verifier uses the execution-relevant parts (anchor, targets, invariants, impact config,
//! attacker capabilities); the NEAR contract applies the rest (severities, payouts, bond, windows).

use alloy_primitives::{Address, B256, U256};
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
}

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

    #[test]
    fn usd_parsing() {
        assert_eq!(usd_to_micro("2000").unwrap(), U256::from(2_000_000_000u64));
        assert_eq!(usd_to_micro("1.5").unwrap(), U256::from(1_500_000u64));
        assert!(usd_to_micro("-1").is_err());
        assert!(usd_to_micro("1.1234567").is_err());
    }
}

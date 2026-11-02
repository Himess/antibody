//! Program manifest: a program's rules as code. `antibody.toml` (human source) compiles to this
//! canonical JSON; sha256 of the exact JSON bytes is the manifest hash the project commits on NEAR.
//! The verifier uses the execution-relevant parts (anchor, targets, invariants, impact config,
//! attacker capabilities); the NEAR contract applies the rest (severities, payouts, bond, windows).

use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MANIFEST_SCHEMA: &str = "antibody.manifest.v1";
pub const MANIFEST_SCHEMA_V2: &str = "antibody.manifest.v2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema: String,
    pub program: ProgramInfo,
    pub anchor: Anchor,
    pub targets: Vec<Target>,
    pub invariants: Invariants,
    pub impact: ImpactConfig,
    pub capabilities: Capabilities,
    /// v1 only: v2 carries every settlement term in `settlement`, so the terms have one source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payout: Option<Payout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Rules>,
    /// Rounds (docs/rounds.md): 0 is the program as opened. A round-N manifest is the program's
    /// manifest with the project's fix committed as code overrides at the same anchor. Both fields
    /// are omitted in round 0, so round-0 manifests keep their exact bytes and hash.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub round: u32,
    /// Runtime code that replaces the anchor's code at these target addresses in this round. Each
    /// overridden target's `code_hash` is the override's hash; storage stays the anchor's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<CodeOverride>,
    /// v2 (docs/spec.md §3.2). Absent fields keep their v1 meaning, so v1 manifests keep their bytes.
    /// Committed groups of entry points (§7.2); absent: one group per (target, selector).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    /// Accounts whose loss counts as harm for the gap lane (§7.7): user or protocol funds only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected_accounts: Vec<ProtectedAccount>,
    /// Calls that must keep succeeding after a fix (§8.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub canaries: Vec<Canary>,
    /// v2: the settlement rules near-antibody enforces (§3.2: payouts, windows, budgets, policies).
    /// The verifier carries it unchanged; the manifest hash covers it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Group {
    pub id: u16,
    pub name: String,
    pub members: Vec<GroupMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupMember {
    pub target: Address,
    /// `0x` + 4 bytes.
    pub selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtectedAccount {
    pub address: Address,
    /// `user` or `protocol`.
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Canary {
    pub to: Address,
    pub data: Bytes,
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
    /// Total gas of the evaluations at entry calls (§5.5). Absent: `(max_entry_calls + 2) × gas_limit`.
    /// The final evaluation runs with its own reserved `gas_limit`, outside this total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_eval_gas: Option<u64>,
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
        match m.schema.as_str() {
            MANIFEST_SCHEMA if m.payout.is_none() || m.rules.is_none() || m.settlement.is_some() => {
                return Err("manifest: a v1 manifest has payout and rules, and no settlement".into());
            }
            MANIFEST_SCHEMA_V2 if m.settlement.is_none() || m.payout.is_some() || m.rules.is_some() => {
                return Err("manifest: a v2 manifest has a settlement section, and no payout or rules".into());
            }
            MANIFEST_SCHEMA | MANIFEST_SCHEMA_V2 => {}
            other => return Err(format!("manifest: unsupported schema {other}")),
        }
        m.check_overrides()?;
        m.check_v2()?;
        Ok(m)
    }

    /// v2 fields: groups name targets and unique ids; protected accounts are user or protocol funds;
    /// the evaluation budget covers every entry call (docs/spec.md §3.3).
    pub fn check_v2(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for g in &self.groups {
            if !ids.insert(g.id) {
                return Err(format!("manifest: duplicate group id {}", g.id));
            }
            for m in &g.members {
                if !self.targets.iter().any(|t| t.address == m.target) {
                    return Err(format!("manifest: group {} names a non-target {}", g.id, m.target));
                }
                let ok = m.selector.len() == 10 && m.selector.starts_with("0x") && hex::decode(&m.selector[2..]).is_ok();
                if !ok {
                    return Err(format!("manifest: group {} has a malformed selector {}", g.id, m.selector));
                }
            }
        }
        for a in &self.protected_accounts {
            if a.kind != "user" && a.kind != "protocol" {
                return Err(format!("manifest: protected account {} must be user or protocol funds", a.address));
            }
        }
        let need = (self.capabilities.max_entry_calls as u64 + 2).saturating_mul(self.invariants.gas_limit);
        if let Some(g) = self.capabilities.max_eval_gas {
            if g < need {
                return Err(format!("manifest: max_eval_gas {g} must be at least (max_entry_calls + 2) x gas_limit = {need}"));
            }
        }
        if self.groups.len() > 64 || self.protected_accounts.len() > 64 || self.canaries.len() > 8 {
            return Err("manifest: too many groups, protected accounts or canaries".into());
        }
        Ok(())
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

    /// The committed group of an entry point, if the manifest commits groups.
    pub fn group_of(&self, target: Address, selector: [u8; 4]) -> Option<u16> {
        let sel = format!("0x{}", hex::encode(selector));
        self.groups.iter().find(|g| g.members.iter().any(|m| m.target == target && m.selector.eq_ignore_ascii_case(&sel))).map(|g| g.id)
    }

    pub fn max_eval_gas(&self) -> u64 {
        self.capabilities
            .max_eval_gas
            .unwrap_or_else(|| (self.capabilities.max_entry_calls as u64 + 2).saturating_mul(self.invariants.gas_limit))
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

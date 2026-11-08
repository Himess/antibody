//! The manifest of a NEAR program: `antibody.manifest.v2` with `vm: "near"` and a `near` section
//! (docs/near-targets.md §2). The fields near-antibody reads keep their types and meaning; the rest only
//! v2 reads. Parsing never re-encodes: the manifest hash is the sha256 of the exact bytes received.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::{
    config::{NEAR_TESTNET_CHAIN_ID, PROTOCOL_VERSION},
    util::{hex32, is_hex32_lower, is_valid_account_id, parse_u128, unb64, usd_to_micro},
};

pub const MANIFEST_SCHEMA_V2: &str = "antibody.manifest.v2";
/// v2's hard cap on receipts per run (docs/near-targets.md §10).
pub const MAX_RECEIPTS_PER_RUN: u32 = 256;
/// v2's promise depth bound (docs/near-targets.md §10).
pub const MAX_PROMISE_DEPTH: u32 = 16;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub program: ProgramInfo,
    pub anchor: Anchor,
    pub targets: Vec<Target>,
    pub invariants: Invariants,
    #[serde(default)]
    pub vm: Option<String>,
    #[serde(default)]
    pub near: Option<Near>,
    #[serde(default)]
    pub settlement: Option<serde_json::Value>,
    #[serde(default)]
    pub round: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProgramInfo {
    pub name: String,
    pub project: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Anchor {
    pub chain_id: u64,
    pub block_number: u64,
    pub block_hash: String,
    pub evm_spec: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Target {
    pub name: String,
    /// The NEAR account.
    pub address: String,
    /// Lowercase hex sha256 of its wasm.
    pub code_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Invariants {
    pub slots: Vec<Slot>,
    /// The rules module's instruction budget per evaluation.
    pub gas_limit: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Slot {
    pub bit: u8,
    pub name: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Near {
    pub protocol: u32,
    pub block: NearBlock,
    pub contracts: Vec<Contract>,
    pub accounts: Vec<AccountSpec>,
    #[serde(default)]
    pub setup: Vec<SetupCall>,
    pub rules: Rules,
    pub impact: Impact,
    pub capabilities: Capabilities,
    /// Custom groups of entry points (docs/spec.md §7.2); absent, each `(receiver, method)` is its own.
    #[serde(default)]
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NearBlock {
    pub height: u64,
    pub timestamp_ns: u64,
    pub epoch_height: u64,
    /// 32 bytes, hex.
    pub random_seed: String,
    /// yoctoNEAR per gas, decimal (recorded; gas is not charged to balances).
    pub gas_price: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WasmRef {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Contract {
    pub account: String,
    pub wasm: WasmRef,
    #[serde(default)]
    pub init: Option<InitCall>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InitCall {
    pub method: String,
    /// Base64.
    #[serde(default)]
    pub args: String,
    #[serde(default = "zero")]
    pub deposit: String,
    pub gas: u64,
}

fn zero() -> String {
    "0".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountSpec {
    pub account: String,
    pub amount: String,
    #[serde(default = "zero")]
    pub locked: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupCall {
    pub signer: String,
    pub receiver: String,
    pub method: String,
    #[serde(default)]
    pub args: String,
    #[serde(default = "zero")]
    pub deposit: String,
    pub gas: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rules {
    pub wasm: WasmRef,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Impact {
    /// NEAR's committed price, decimal USD.
    pub native_usd: String,
    #[serde(default)]
    pub tokens: Vec<Token>,
    #[serde(default)]
    pub protocol_accounts: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Token {
    /// The NEP-141 contract.
    pub contract: String,
    pub symbol: String,
    pub decimals: u8,
    pub usd: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Capabilities {
    pub max_senders: u8,
    pub max_endowment_yocto: String,
    pub max_calls: u32,
    pub max_receipts: u32,
    pub max_instructions_per_call: u64,
    pub max_total_instructions: u64,
    pub max_eval_instructions: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Group {
    pub id: u16,
    pub name: String,
    pub members: Vec<GroupMember>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupMember {
    pub account: String,
    pub method: String,
}

/// The manifest's `vm`: absent or `evm` is an EVM program (v1's path), `near` this engine's.
pub fn vm_of(manifest_json: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(manifest_json).ok()?;
    v.get("vm").and_then(|v| v.as_str()).map(str::to_string)
}

fn check_call(gas: u64, deposit: &str, args: &str, what: &str) -> Result<(), String> {
    if gas == 0 || gas > crate::config::Limits::protocol_74().max_total_prepaid_gas {
        return Err(format!("manifest: {what}: gas must be between 1 and 300 Tgas"));
    }
    parse_u128(deposit, "deposit").map_err(|e| format!("manifest: {what}: {e}"))?;
    unb64(args).map_err(|e| format!("manifest: {what}: args {e}"))?;
    Ok(())
}

impl Manifest {
    /// Parses and checks a NEAR program's manifest. Any error is the program's (`ProgramMisconfigured`).
    pub fn parse(bytes: &[u8]) -> Result<Manifest, String> {
        let m: Manifest = serde_json::from_slice(bytes).map_err(|e| format!("manifest: {e}"))?;
        m.check()?;
        Ok(m)
    }

    pub fn near(&self) -> &Near {
        self.near.as_ref().expect("checked by parse")
    }

    fn check(&self) -> Result<(), String> {
        if self.schema != MANIFEST_SCHEMA_V2 || self.settlement.is_none() {
            return Err("manifest: a NEAR program's manifest is antibody.manifest.v2 with a settlement section".into());
        }
        if self.vm.as_deref() != Some("near") {
            return Err("manifest: vm must be \"near\"".into());
        }
        if self.anchor.evm_spec != "near" {
            return Err("manifest: anchor.evm_spec must be \"near\" when vm is \"near\"".into());
        }
        if self.anchor.chain_id != NEAR_TESTNET_CHAIN_ID {
            return Err(format!("manifest: anchor.chain_id must be {NEAR_TESTNET_CHAIN_ID} (NEAR testnet)"));
        }
        if hex32(&self.anchor.block_hash).is_none() {
            return Err("manifest: anchor.block_hash must be 32 bytes of hex".into());
        }
        if self.round != 0 {
            return Err("manifest: rounds after 0 are not supported for NEAR programs yet".into());
        }
        let near = self.near.as_ref().ok_or("manifest: no near section")?;
        if near.protocol != PROTOCOL_VERSION {
            return Err(format!("manifest: near.protocol {} is not the version this verifier reproduces ({PROTOCOL_VERSION})", near.protocol));
        }
        if hex32(&near.block.random_seed).is_none() {
            return Err("manifest: near.block.random_seed must be 32 bytes of hex".into());
        }
        parse_u128(&near.block.gas_price, "gas price").map_err(|e| format!("manifest: {e}"))?;

        let mut accounts = BTreeSet::new();
        for a in &near.accounts {
            if !is_valid_account_id(&a.account) || !accounts.insert(a.account.as_str()) {
                return Err(format!("manifest: account {:?} is invalid or listed twice", a.account));
            }
            parse_u128(&a.amount, "amount").map_err(|e| format!("manifest: {e}"))?;
            parse_u128(&a.locked, "locked").map_err(|e| format!("manifest: {e}"))?;
        }
        let mut contracts = BTreeSet::new();
        for c in &near.contracts {
            if !accounts.contains(c.account.as_str()) {
                return Err(format!("manifest: contract {} is not in near.accounts", c.account));
            }
            if !contracts.insert(c.account.as_str()) {
                return Err(format!("manifest: contract {} is listed twice", c.account));
            }
            if !is_hex32_lower(&c.wasm.sha256) {
                return Err(format!("manifest: contract {}'s wasm.sha256 must be lowercase hex", c.account));
            }
            if let Some(i) = &c.init {
                check_call(i.gas, &i.deposit, &i.args, &format!("init of {}", c.account))?;
            }
        }
        if self.targets.is_empty() || self.targets.len() > 32 {
            return Err("manifest: 1 to 32 targets".into());
        }
        for t in &self.targets {
            let Some(c) = near.contracts.iter().find(|c| c.account == t.address) else {
                return Err(format!("manifest: target {} is not in near.contracts", t.address));
            };
            if t.code_hash != c.wasm.sha256 {
                return Err(format!("manifest: target {}'s code_hash differs from its wasm.sha256", t.address));
            }
        }
        for (i, s) in near.setup.iter().enumerate() {
            if !accounts.contains(s.signer.as_str()) {
                return Err(format!("manifest: setup call {i}'s signer is not in near.accounts"));
            }
            check_call(s.gas, &s.deposit, &s.args, &format!("setup call {i}"))?;
        }
        if !is_hex32_lower(&near.rules.wasm.sha256) {
            return Err("manifest: near.rules.wasm.sha256 must be lowercase hex".into());
        }
        let mut bits = BTreeSet::new();
        if self.invariants.slots.len() > 64 {
            return Err("manifest: at most 64 slots".into());
        }
        for s in &self.invariants.slots {
            if s.bit >= 64 || !bits.insert(s.bit) {
                return Err(format!("manifest: slot bit {} must be unique and below 64 (the rules return a u64 mask)", s.bit));
            }
        }
        if self.invariants.gas_limit == 0 {
            return Err("manifest: invariants.gas_limit (instructions per evaluation) must be positive".into());
        }
        usd_to_micro(&near.impact.native_usd).map_err(|e| format!("manifest: {e}"))?;
        for t in &near.impact.tokens {
            if !is_valid_account_id(&t.contract) || t.decimals > 38 {
                return Err(format!("manifest: token {} is invalid", t.contract));
            }
            usd_to_micro(&t.usd).map_err(|e| format!("manifest: {e}"))?;
        }
        if near.impact.protocol_accounts.iter().any(|a| !is_valid_account_id(a)) {
            return Err("manifest: a protocol account is not a valid account id".into());
        }
        let c = &near.capabilities;
        parse_u128(&c.max_endowment_yocto, "max_endowment_yocto").map_err(|e| format!("manifest: {e}"))?;
        if c.max_senders == 0 || c.max_calls == 0 || c.max_receipts == 0 || c.max_receipts > MAX_RECEIPTS_PER_RUN {
            return Err(format!("manifest: capabilities need 1+ senders and calls, and 1 to {MAX_RECEIPTS_PER_RUN} receipts"));
        }
        if c.max_instructions_per_call == 0 || c.max_total_instructions < c.max_instructions_per_call {
            return Err("manifest: max_total_instructions must cover max_instructions_per_call".into());
        }
        let need = (c.max_calls as u64).saturating_mul(self.invariants.gas_limit);
        if c.max_eval_instructions < need {
            return Err(format!("manifest: max_eval_instructions must be at least max_calls x gas_limit = {need}"));
        }
        let mut ids = BTreeSet::new();
        if near.groups.len() > 64 {
            return Err("manifest: at most 64 groups".into());
        }
        for g in &near.groups {
            if !ids.insert(g.id) {
                return Err(format!("manifest: duplicate group id {}", g.id));
            }
            for m in &g.members {
                if !self.targets.iter().any(|t| t.address == m.account) || m.method.is_empty() {
                    return Err(format!("manifest: group {} names a non-target or an empty method", g.id));
                }
            }
        }
        Ok(())
    }

    /// The committed group of an entry point, if any.
    pub fn group_of(&self, receiver: &str, method: &str) -> Option<u16> {
        self.near().groups.iter().find(|g| g.members.iter().any(|m| m.account == receiver && m.method == method)).map(|g| g.id)
    }

    pub fn slot(&self, bit: u8) -> Option<&Slot> {
        self.invariants.slots.iter().find(|s| s.bit == bit)
    }

    /// The mask of declared slots.
    pub fn declared(&self) -> u64 {
        self.invariants.slots.iter().fold(0u64, |m, s| m | (1u64 << s.bit))
    }
}

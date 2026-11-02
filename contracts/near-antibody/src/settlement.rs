//! The part of a v2 manifest near-antibody enforces (docs/spec.md §3.2): the program's identity, its
//! anchor, its invariant slots and the `settlement` section. The manifest arrives as its canonical JSON
//! text; its SHA-256 is the program's manifest hash. The verifier enforces everything else.

use antibody_rules::{GapLane, Params, Severity, SlotRule, UnevaluablePolicy, Windows};
use near_sdk::{
    near,
    serde_json::{self, Value},
};

#[near(serializers = [json])]
pub struct ManifestIn {
    pub schema: String,
    pub program: MProgram,
    pub anchor: MAnchor,
    pub targets: Vec<MTarget>,
    pub invariants: MInvariants,
    pub settlement: Option<Value>,
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
}

#[near(serializers = [json])]
pub struct MTarget {
    pub name: String,
    pub address: String,
    pub code_hash: String,
}

#[near(serializers = [json])]
pub struct MInvariants {
    pub slots: Vec<MSlot>,
    pub gas_limit: u64,
}

#[near(serializers = [json])]
pub struct MSlot {
    pub bit: u8,
    pub name: String,
    pub severity: Severity,
}

#[near(serializers = [json])]
pub struct Settlement {
    pub payouts: SPayouts,
    pub instant: SInstant,
    pub fees: SFees,
    pub bond: String,
    pub challenge_bond: String,
    pub windows: SWindows,
    /// Budget per invariant slot per round (raw token units); a slot not listed has the escrow as budget.
    #[serde(default)]
    pub slot_budgets: Vec<SBudget>,
    pub unevaluable: Value,
    #[serde(default)]
    pub gap: Option<SGap>,
    pub known: SKnown,
    pub pause: SPause,
    pub terms_sha256: String,
}

#[near(serializers = [json])]
pub struct SPayouts {
    pub critical: String,
    pub high: String,
    pub medium: String,
    pub low: String,
    pub impact_bps: u32,
    pub impact_cap: String,
    pub decimals: u8,
}

#[near(serializers = [json])]
pub struct SInstant {
    pub cap: String,
    pub budget: String,
    pub window_secs: u64,
}

#[near(serializers = [json])]
pub struct SFees {
    pub protocol_bps: u32,
    pub release: String,
}

#[near(serializers = [json])]
pub struct SWindows {
    pub reveal: u64,
    pub challenge: u64,
    pub resolve: u64,
    pub infra_grace: u64,
    pub run_timeout: u64,
    pub round_min: u64,
    pub rerun_window: u64,
    pub disclosure_max_delay: u64,
    /// Unix seconds.
    pub deadline: u64,
    pub exit_grace: u64,
}

#[near(serializers = [json])]
pub struct SBudget {
    pub slot: u8,
    pub budget: String,
}

#[near(serializers = [json])]
pub struct SGap {
    pub threshold_usd_micro: String,
    pub claim_amount: String,
}

#[near(serializers = [json])]
pub struct SKnown {
    pub max_count: u32,
    pub max_share_bps: u32,
}

#[near(serializers = [json])]
pub struct SPause {
    pub slots: Vec<u8>,
    pub min_impact_usd_micro: String,
}

fn amount(s: &str, what: &str) -> Result<u128, String> {
    if s.is_empty() || s.len() > 39 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{what}: not a decimal amount"));
    }
    s.parse::<u128>().map_err(|_| format!("{what}: out of range"))
}

fn is_hex(s: &str, bytes: usize) -> bool {
    s.len() == 2 * bytes && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// What `create_program` keeps from a manifest.
pub struct Parsed {
    pub manifest: ManifestIn,
    pub params: Params,
    pub terms_sha256: String,
}

/// Parses a v2 manifest. `escrow` sizes slots without an explicit budget; `run_deposit` comes from the
/// program's pinned verifier configuration.
pub fn parse(text: &str, escrow: u128, run_deposit: u128) -> Result<Parsed, String> {
    if text.len() > 200_000 {
        return Err("manifest too large".into());
    }
    let m: ManifestIn = serde_json::from_str(text).map_err(|e| format!("manifest: {e}"))?;
    if m.schema != "antibody.manifest.v2" {
        return Err("manifest: schema must be antibody.manifest.v2".into());
    }
    if m.program.name.len() > 128 || m.program.description.len() > 2048 || m.targets.len() > 32 {
        return Err("manifest: a field is too long".into());
    }
    let h = m.anchor.block_hash.trim_start_matches("0x");
    if !is_hex(h, 32) {
        return Err("manifest: anchor block hash".into());
    }
    let s: Settlement = serde_json::from_value(m.settlement.clone().ok_or("manifest: no settlement section")?)
        .map_err(|e| format!("settlement: {e}"))?;
    if !is_hex(s.terms_sha256.trim_start_matches("0x"), 32) {
        return Err("settlement: terms_sha256".into());
    }
    let mut slots = Vec::new();
    for sl in &m.invariants.slots {
        if sl.bit == 255 {
            continue;
        }
        let budget = match s.slot_budgets.iter().find(|b| b.slot == sl.bit) {
            Some(b) => amount(&b.budget, "slot budget")?,
            None => escrow,
        };
        slots.push(SlotRule { slot: sl.bit, severity: sl.severity, budget });
    }
    let unevaluable = match &s.unevaluable {
        Value::String(x) if x == "refund" => UnevaluablePolicy::Refund,
        Value::Object(o) if o.contains_key("violation") => {
            let sev: Severity = serde_json::from_value(o["violation"].clone()).map_err(|_| "settlement: unevaluable severity")?;
            UnevaluablePolicy::Violation { severity: sev }
        }
        _ => return Err("settlement: unevaluable must be \"refund\" or {\"violation\": severity}".into()),
    };
    let gap = match &s.gap {
        Some(g) => Some(GapLane {
            threshold_usd_micro: amount(&g.threshold_usd_micro, "gap threshold")?,
            claim_amount: amount(&g.claim_amount, "gap claim amount")?,
        }),
        None => None,
    };
    let w = &s.windows;
    let params = Params {
        critical: amount(&s.payouts.critical, "critical")?,
        high: amount(&s.payouts.high, "high")?,
        medium: amount(&s.payouts.medium, "medium")?,
        low: amount(&s.payouts.low, "low")?,
        impact_bps: s.payouts.impact_bps,
        impact_cap: amount(&s.payouts.impact_cap, "impact_cap")?,
        decimals: s.payouts.decimals,
        instant_cap: amount(&s.instant.cap, "instant cap")?,
        instant_budget: amount(&s.instant.budget, "instant budget")?,
        instant_window: s.instant.window_secs,
        protocol_fee_bps: s.fees.protocol_bps,
        release_fee: amount(&s.fees.release, "release fee")?,
        bond: amount(&s.bond, "bond")?,
        challenge_bond: amount(&s.challenge_bond, "challenge bond")?,
        windows: Windows {
            reveal: w.reveal,
            challenge: w.challenge,
            resolve: w.resolve,
            infra_grace: w.infra_grace,
            run_timeout: w.run_timeout,
            round_min: w.round_min,
            rerun_window: w.rerun_window,
            disclosure_max_delay: w.disclosure_max_delay,
            deadline: w.deadline,
            exit_grace: w.exit_grace,
        },
        slots,
        unevaluable,
        gap,
        known_max_count: s.known.max_count,
        known_max_share_bps: s.known.max_share_bps,
        pause_slots: s.pause.slots.clone(),
        pause_min_impact_usd_micro: amount(&s.pause.min_impact_usd_micro, "pause minimum impact")?,
        run_deposit,
    };
    Ok(Parsed { manifest: m, params, terms_sha256: s.terms_sha256.trim_start_matches("0x").to_string() })
}

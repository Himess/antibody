//! The manifest's `settlement` section (docs/spec.md §3.2): every term near-antibody enforces. The
//! contract parses it into [`Params`]; the CLI and the services build it. Amounts are decimal strings
//! in raw token units.

use serde::{Deserialize, Serialize};

use crate::types::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settlement {
    pub payouts: Payouts,
    pub instant: Instant,
    pub fees: Fees,
    pub bond: String,
    pub challenge_bond: String,
    pub windows: SWindows,
    /// Budget per invariant slot per round; a slot not listed has the escrow as its budget.
    #[serde(default)]
    pub slot_budgets: Vec<SlotBudget>,
    /// `"refund"`, or `{"violation": "<severity>"}` (§5.4).
    pub unevaluable: serde_json::Value,
    #[serde(default)]
    pub gap: Option<Gap>,
    pub known: Known,
    pub pause: Pause,
    /// SHA-256 of the program's terms text (§16); every commit names it.
    pub terms_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payouts {
    pub critical: String,
    pub high: String,
    pub medium: String,
    pub low: String,
    pub impact_bps: u32,
    pub impact_cap: String,
    pub decimals: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instant {
    pub cap: String,
    pub budget: String,
    pub window_secs: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fees {
    pub protocol_bps: u32,
    pub release: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotBudget {
    pub slot: u8,
    pub budget: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    pub threshold_usd_micro: String,
    pub claim_amount: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Known {
    pub max_count: u32,
    pub max_share_bps: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pause {
    pub slots: Vec<u8>,
    pub min_impact_usd_micro: String,
}

fn amount(s: &str, what: &str) -> Result<Amount, String> {
    if s.is_empty() || s.len() > 39 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{what}: not a decimal amount"));
    }
    s.parse::<Amount>().map_err(|_| format!("{what}: out of range"))
}

/// Lowercase hex of `bytes` bytes, with or without `0x`.
pub fn is_hex(s: &str, bytes: usize) -> bool {
    let s = s.strip_prefix("0x").unwrap_or(s);
    s.len() == 2 * bytes && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl Settlement {
    /// The rule core's parameters. `slots` are the manifest's invariant slots (bit, severity); `escrow`
    /// is the budget of a slot without one; `run_deposit` comes from the program's pinned verifier.
    pub fn params(&self, slots: &[(u8, Severity)], escrow: Amount, run_deposit: Amount) -> Result<Params, String> {
        if !is_hex(&self.terms_sha256, 32) {
            return Err("settlement: terms_sha256".into());
        }
        let mut rules = Vec::new();
        for (bit, severity) in slots {
            let budget = match self.slot_budgets.iter().find(|b| b.slot == *bit) {
                Some(b) => amount(&b.budget, "slot budget")?,
                None => escrow,
            };
            rules.push(SlotRule { slot: *bit, severity: *severity, budget });
        }
        if self.slot_budgets.iter().any(|b| !slots.iter().any(|(bit, _)| *bit == b.slot)) {
            return Err("settlement: a slot budget names an unknown slot".into());
        }
        let unevaluable = match &self.unevaluable {
            serde_json::Value::String(x) if x == "refund" => UnevaluablePolicy::Refund,
            serde_json::Value::Object(o) if o.len() == 1 && o.contains_key("violation") => {
                let severity: Severity = serde_json::from_value(o["violation"].clone()).map_err(|_| "settlement: unevaluable severity")?;
                UnevaluablePolicy::Violation { severity }
            }
            _ => return Err("settlement: unevaluable must be \"refund\" or {\"violation\": severity}".into()),
        };
        let gap = match &self.gap {
            Some(g) => Some(GapLane {
                threshold_usd_micro: amount(&g.threshold_usd_micro, "gap threshold")?,
                claim_amount: amount(&g.claim_amount, "gap claim amount")?,
            }),
            None => None,
        };
        let w = &self.windows;
        Ok(Params {
            critical: amount(&self.payouts.critical, "critical")?,
            high: amount(&self.payouts.high, "high")?,
            medium: amount(&self.payouts.medium, "medium")?,
            low: amount(&self.payouts.low, "low")?,
            impact_bps: self.payouts.impact_bps,
            impact_cap: amount(&self.payouts.impact_cap, "impact_cap")?,
            decimals: self.payouts.decimals,
            instant_cap: amount(&self.instant.cap, "instant cap")?,
            instant_budget: amount(&self.instant.budget, "instant budget")?,
            instant_window: self.instant.window_secs,
            protocol_fee_bps: self.fees.protocol_bps,
            release_fee: amount(&self.fees.release, "release fee")?,
            bond: amount(&self.bond, "bond")?,
            challenge_bond: amount(&self.challenge_bond, "challenge bond")?,
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
            slots: rules,
            unevaluable,
            gap,
            known_max_count: self.known.max_count,
            known_max_share_bps: self.known.max_share_bps,
            pause_slots: self.pause.slots.clone(),
            pause_min_impact_usd_micro: amount(&self.pause.min_impact_usd_micro, "pause minimum impact")?,
            run_deposit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Settlement {
        serde_json::from_value(serde_json::json!({
            "payouts": {"critical": "100", "high": "50", "medium": "10", "low": "1", "impact_bps": 0, "impact_cap": "0", "decimals": 6},
            "instant": {"cap": "100", "budget": "200", "window_secs": 3600},
            "fees": {"protocol_bps": 0, "release": "0"},
            "bond": "5", "challenge_bond": "5",
            "windows": {"reveal": 60, "challenge": 60, "resolve": 60, "infra_grace": 60, "run_timeout": 60, "round_min": 60,
                        "rerun_window": 60, "disclosure_max_delay": 60, "deadline": 10_000, "exit_grace": 60},
            "slot_budgets": [{"slot": 1, "budget": "70"}],
            "unevaluable": {"violation": "high"},
            "known": {"max_count": 2, "max_share_bps": 1000},
            "pause": {"slots": [0], "min_impact_usd_micro": "0"},
            "terms_sha256": "ab".repeat(32),
        }))
        .unwrap()
    }

    #[test]
    fn params_follow_the_section() {
        let p = s().params(&[(0, Severity::Critical), (1, Severity::High)], 1_000, 7).unwrap();
        assert_eq!((p.critical, p.bond, p.run_deposit), (100, 5, 7));
        assert_eq!(p.slots, vec![SlotRule { slot: 0, severity: Severity::Critical, budget: 1_000 }, SlotRule { slot: 1, severity: Severity::High, budget: 70 }]);
        assert_eq!(p.unevaluable, UnevaluablePolicy::Violation { severity: Severity::High });
        assert!(bounds::check(&p, 1_000, 1).is_ok());
    }

    #[test]
    fn malformed_sections_are_refused() {
        let mut x = s();
        x.bond = "5.0".into();
        assert!(x.params(&[(0, Severity::Critical)], 1, 1).is_err());
        let mut x = s();
        x.slot_budgets[0].slot = 9;
        assert!(x.params(&[(0, Severity::Critical)], 1, 1).is_err());
        let mut x = s();
        x.unevaluable = serde_json::json!("forfeit");
        assert!(x.params(&[(0, Severity::Critical)], 1, 1).is_err());
        assert!(serde_json::from_value::<Settlement>(serde_json::json!({"payouts": {}, "extra": 1})).is_err());
    }
}

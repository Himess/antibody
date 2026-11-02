//! Identifiers, enums and the program parameters the rules enforce (spec §3.2).

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

pub type AccountId = String;
pub type ProgramId = u32;
pub type CommitId = u64;
pub type RunId = u64;
/// Token amount (tUSD raw units) or yoctoNEAR, depending on the field.
pub type Amount = u128;
/// Seconds (NEAR block timestamp / 1e9).
pub type Time = u64;

/// A 32-byte value (tag, hash, key) serialised as lowercase hex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default, BorshSerialize, BorshDeserialize)]
pub struct H32(pub [u8; 32]);

impl H32 {
    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }
    pub fn from_hex(s: &str) -> Option<H32> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        hex::decode_to_slice(s, &mut out).ok()?;
        Some(H32(out))
    }
}

impl Serialize for H32 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for H32 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <String as Deserialize>::deserialize(d)?;
        H32::from_hex(&s).ok_or_else(|| serde::de::Error::custom("expected 32 bytes of hex"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Who vouches for a run's facts (spec §6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum Tier {
    /// OutLayer returned the facts.
    T0,
    /// The run's TDX quote was verified on NEAR.
    T1,
}

/// Verdict kinds (spec §5.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum Kind {
    Violation,
    NoViolation,
    InvalidPoC,
    Unevaluable,
    ProgramMisconfigured,
    InfraError,
}

/// Verifier run modes (spec §5.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Activate,
    Verify,
    Second,
    Rerun,
    CheckRound,
    Distinct,
    ReleaseWrap,
    Disclose,
}

/// One invariant slot: its severity and its payout budget per round (spec §7.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct SlotRule {
    pub slot: u8,
    pub severity: Severity,
    /// Total payable per round for keys of this slot.
    pub budget: Amount,
}

/// What `check()` reverting or running out of gas after the PoC means (spec §5.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnevaluablePolicy {
    Refund,
    Violation { severity: Severity },
}

/// The gap lane (spec §7.7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct GapLane {
    pub threshold_usd_micro: Amount,
    pub claim_amount: Amount,
}

/// Every window, in seconds (spec §3.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Windows {
    pub reveal: Time,
    pub challenge: Time,
    pub resolve: Time,
    pub infra_grace: Time,
    pub run_timeout: Time,
    pub round_min: Time,
    pub rerun_window: Time,
    pub disclosure_max_delay: Time,
    /// Absolute: no commit after this time.
    pub deadline: Time,
    pub exit_grace: Time,
}

/// The parameters of a program the rules enforce. The verifier enforces the rest of the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Params {
    pub critical: Amount,
    pub high: Amount,
    pub medium: Amount,
    pub low: Amount,
    pub impact_bps: u32,
    pub impact_cap: Amount,
    /// Decimals of the payout token; impacts are micro-USD.
    pub decimals: u8,
    pub instant_cap: Amount,
    pub instant_budget: Amount,
    pub instant_window: Time,
    pub protocol_fee_bps: u32,
    pub release_fee: Amount,
    pub bond: Amount,
    pub challenge_bond: Amount,
    pub windows: Windows,
    pub slots: Vec<SlotRule>,
    pub unevaluable: UnevaluablePolicy,
    pub gap: Option<GapLane>,
    pub known_max_count: u32,
    pub known_max_share_bps: u32,
    pub pause_slots: Vec<u8>,
    pub pause_min_impact_usd_micro: Amount,
    /// Verification deposit attached to each run, in yoctoNEAR (OutLayer refunds the unused part).
    pub run_deposit: Amount,
}

impl Params {
    pub fn base(&self, s: Severity) -> Amount {
        match s {
            Severity::Critical => self.critical,
            Severity::High => self.high,
            Severity::Medium => self.medium,
            Severity::Low => self.low,
        }
    }

    pub fn slot(&self, slot: u8) -> Option<&SlotRule> {
        self.slots.iter().find(|s| s.slot == slot)
    }

    /// Impact bonus in token units: `impact × impact_bps / 10⁴`, capped. Impacts are micro-USD.
    pub fn impact_bonus(&self, impact_usd_micro: Amount) -> Amount {
        if self.impact_bps == 0 {
            return 0;
        }
        let units = if self.decimals >= 6 {
            impact_usd_micro.saturating_mul(10u128.saturating_pow((self.decimals - 6) as u32))
        } else {
            impact_usd_micro / 10u128.pow((6 - self.decimals) as u32)
        };
        (units.saturating_mul(self.impact_bps as u128) / 10_000).min(self.impact_cap)
    }
}

/// Bounds every program must satisfy (spec §3.3). Amount bounds keep all payout arithmetic far from
/// `u128::MAX`.
pub mod bounds {
    use super::*;

    pub const MAX_AMOUNT: Amount = 1u128 << 100;
    pub const MAX_SLOTS: usize = 64;
    pub const MAX_KNOWN: u32 = 32;
    pub const MAX_BPS: u32 = 10_000;
    pub const MIN_WINDOW: Time = 30;
    pub const MAX_WINDOW: Time = 365 * 24 * 3600;

    pub fn check(p: &Params, escrow: Amount, now: Time) -> Result<(), &'static str> {
        let amounts = [
            p.critical, p.high, p.medium, p.low, p.impact_cap, p.instant_cap, p.instant_budget, p.release_fee, p.bond,
            p.challenge_bond, p.pause_min_impact_usd_micro, escrow,
        ];
        if amounts.iter().any(|a| *a > MAX_AMOUNT) {
            return Err("an amount is out of bounds");
        }
        if p.impact_bps > MAX_BPS || p.protocol_fee_bps > 2_000 || p.known_max_share_bps > MAX_BPS {
            return Err("a basis-point rate is out of bounds");
        }
        if p.decimals > 24 {
            return Err("decimals out of bounds");
        }
        let w = &p.windows;
        for t in [w.reveal, w.challenge, w.resolve, w.infra_grace, w.run_timeout, w.round_min, w.rerun_window, w.exit_grace] {
            if !(MIN_WINDOW..=MAX_WINDOW).contains(&t) {
                return Err("a window is out of bounds");
            }
        }
        if p.instant_window < MIN_WINDOW || p.instant_window > MAX_WINDOW {
            return Err("instant_window out of bounds");
        }
        if w.disclosure_max_delay > MAX_WINDOW || w.disclosure_max_delay < MIN_WINDOW {
            return Err("disclosure_max_delay out of bounds");
        }
        if w.deadline <= now || w.deadline > now + MAX_WINDOW {
            return Err("deadline must be in the future, within a year");
        }
        if w.infra_grace < w.run_timeout {
            return Err("infra_grace must cover at least one run timeout");
        }
        if p.slots.is_empty() || p.slots.len() > MAX_SLOTS {
            return Err("1 to 64 invariant slots");
        }
        let mut seen = [false; 256];
        for s in &p.slots {
            if seen[s.slot as usize] {
                return Err("duplicate slot");
            }
            seen[s.slot as usize] = true;
            if s.budget > MAX_AMOUNT {
                return Err("a slot budget is out of bounds");
            }
        }
        if p.known_max_count > MAX_KNOWN {
            return Err("too many known issues allowed");
        }
        if p.bond == 0 {
            return Err("the bond must be positive");
        }
        if p.challenge_bond == 0 {
            return Err("the challenge bond must be positive");
        }
        if p.instant_cap > p.instant_budget {
            return Err("instant_cap above instant_budget");
        }
        if let Some(g) = &p.gap {
            if g.claim_amount > MAX_AMOUNT || g.threshold_usd_micro > MAX_AMOUNT {
                return Err("gap lane amounts out of bounds");
            }
        }
        for s in &p.pause_slots {
            if p.slot(*s).is_none() {
                return Err("the pause policy names an unknown slot");
            }
        }
        Ok(())
    }
}

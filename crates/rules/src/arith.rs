//! The rule core's arithmetic as pure functions: the engine calls these, and Kani checks them
//! (`proofs.rs`, `cargo kani -p antibody-rules`). Escrow movements are the [`Ledger`] methods.
//!
//! Program bounds (spec §3.3, [`crate::types::bounds`]): every amount a program sets is at most 2^100,
//! every rate at most 10⁴ basis points (the protocol fee at most 2,000) and every window at least 30 s,
//! so nothing here saturates, overflows or divides by zero.
//!
//! [`Ledger`]: crate::state::Ledger

use crate::state::InstantWindow;
use crate::types::{Amount, Time};

/// Basis points in one.
pub const BPS: u128 = 10_000;

/// `amount × bps / 10⁴`, rounded down.
pub fn bps_of(amount: Amount, bps: u32) -> Amount {
    amount.saturating_mul(bps as u128) / BPS
}

/// Micro-USD in units of a token with `decimals` decimals, rounded down.
pub fn usd_micro_to_units(usd_micro: Amount, decimals: u8) -> Amount {
    if decimals >= 6 {
        usd_micro.saturating_mul(10u128.saturating_pow((decimals - 6) as u32))
    } else {
        usd_micro / 10u128.pow((6 - decimals) as u32)
    }
}

/// The impact bonus: `bps` of the impact in token units, capped at `cap`.
pub fn impact_bonus(impact_usd_micro: Amount, bps: u32, cap: Amount, decimals: u8) -> Amount {
    if bps == 0 {
        return 0;
    }
    bps_of(usd_micro_to_units(impact_usd_micro, decimals), bps).min(cap)
}

/// What a slot's budget still allows of a nominal payout: `min(nominal, budget − spent)`.
pub fn within_budget(nominal: Amount, budget: Amount, spent: Amount) -> Amount {
    nominal.min(budget.saturating_sub(spent))
}

/// Splits a claim into the part the escrow covers now and the shortfall. Nothing is covered while
/// earlier debts wait (`blocked`), so the shortfall queue is funded in commit order (spec §7.8).
pub fn cover(want: Amount, available: Amount, blocked: bool) -> (Amount, Amount) {
    let reserved = if blocked { 0 } else { want.min(available) };
    (reserved, want - reserved)
}

/// Fits a reservation to a new amount owed: `(kept, released, debt)`, where `kept + released` is the
/// old reservation and `kept + debt` the new amount.
pub fn refit(reserved: Amount, amount: Amount) -> (Amount, Amount, Amount) {
    let kept = reserved.min(amount);
    (kept, reserved - kept, amount - kept)
}

/// The instant budget's window at `now`.
pub fn window_index(now: Time, window: Time) -> u64 {
    now / window
}

/// The instant budget (spec §6.3): the window state after paying `amount` instantly in window
/// `index`, or `None` if the payout is above the cap or the window's budget cannot cover it.
pub fn instant(index: u64, current: &InstantWindow, amount: Amount, cap: Amount, budget: Amount) -> Option<InstantWindow> {
    let spent = if index == current.index { current.spent } else { 0 };
    let after = spent.checked_add(amount)?;
    (amount <= cap && after <= budget).then_some(InstantWindow { index, spent: after })
}

/// The protocol fee on a payout, taken from what is still available in the escrow.
pub fn protocol_fee(amount: Amount, bps: u32, available: Amount) -> Amount {
    bps_of(amount, bps).min(available)
}

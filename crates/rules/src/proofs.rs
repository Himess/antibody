//! Kani proofs of the rule core's arithmetic: the formulas in `arith.rs` and every `Ledger` movement.
//!
//!   cargo kani -p antibody-rules
//!
//! Inputs range over everything the program bounds allow (`types::bounds`, spec §3.3): amounts up to
//! 2^100, rates up to 10⁴ basis points (the protocol fee up to 2,000), windows of 30 s to a year. A
//! program's ledger is bounded by the token's supply, taken as 2^126. Kani also checks every `+`, `-`
//! and `*` on the way for overflow, so each proof shows the code cannot panic on these inputs.
//!
//! Three facts need 128-bit multiplication and division that the solver does not finish in 20 minutes:
//! a rate never exceeds its base, the protocol fee is at most a fifth of the payout, and a larger
//! impact never earns a smaller bonus. `tests/arith.rs` tests them instead (random inputs over the whole
//! range, plus the boundary values).

use crate::arith::*;
use crate::state::{InstantWindow, Ledger};
use crate::types::bounds::{MAX_AMOUNT, MAX_BPS, MAX_WINDOW, MIN_WINDOW};
use crate::types::{Amount, Time};

/// Above any token supply a program can hold.
const SUPPLY: Amount = 1 << 126;

/// The largest nominal payout: a base amount plus a capped impact bonus.
const MAX_NOMINAL: Amount = 2 * MAX_AMOUNT;

fn amount() -> Amount {
    kani::any_where(|a: &Amount| *a <= MAX_AMOUNT)
}

fn bps() -> u32 {
    kani::any_where(|b: &u32| *b <= MAX_BPS)
}

fn funded(l: &Ledger) -> Option<Amount> {
    l.deposits.checked_add(l.forfeits)?.checked_add(l.challenge_bonds)
}

fn accounted(l: &Ledger) -> Option<Amount> {
    l.available.checked_add(l.reserved)?.checked_add(l.paid)?.checked_add(l.fees)?.checked_add(l.returned)
}

/// Both sides of the ledger equation agree (spec §7.10).
fn balanced(l: &Ledger) -> bool {
    funded(l).is_some() && funded(l) == accounted(l)
}

/// Any ledger an engine step can start from.
fn ledger() -> Ledger {
    let l = Ledger {
        deposits: kani::any(),
        forfeits: kani::any(),
        challenge_bonds: kani::any(),
        available: kani::any(),
        reserved: kani::any(),
        paid: kani::any(),
        fees: kani::any(),
        returned: kani::any(),
    };
    kani::assume(balanced(&l) && funded(&l).unwrap() <= SUPPLY);
    l
}

// ---------------------------------------------------------------------------------- rates

#[kani::proof]
fn a_rate_of_a_bounded_amount_is_exact() {
    let (a, b) = (amount(), bps());
    assert!(a.checked_mul(b as u128).is_some());
}

#[kani::proof]
fn the_protocol_fee_is_never_more_than_available() {
    let (a, b, available) = (kani::any::<Amount>(), kani::any_where(|b: &u32| *b <= 2_000), kani::any::<Amount>());
    assert!(protocol_fee(a, b, available) <= available);
}


// The impact bonus, for every token precision the bounds allow (0 to 24 decimals): any impact, capped,
// and the nominal payout `base + bonus` cannot overflow.
macro_rules! impact_bonus_proofs {
    ($($name:ident: $d:expr,)*) => {$(
        #[kani::proof]
        fn $name() {
            let (impact, b, cap, base) = (kani::any::<Amount>(), bps(), amount(), amount());
            let bonus = impact_bonus(impact, b, cap, $d);
            assert!(bonus <= cap);
            assert!(base + bonus <= MAX_NOMINAL);
        }
    )*};
}

impact_bonus_proofs! {
    impact_bonus_d0: 0, impact_bonus_d1: 1, impact_bonus_d2: 2, impact_bonus_d3: 3, impact_bonus_d4: 4,
    impact_bonus_d5: 5, impact_bonus_d6: 6, impact_bonus_d7: 7, impact_bonus_d8: 8, impact_bonus_d9: 9,
    impact_bonus_d10: 10, impact_bonus_d11: 11, impact_bonus_d12: 12, impact_bonus_d13: 13, impact_bonus_d14: 14,
    impact_bonus_d15: 15, impact_bonus_d16: 16, impact_bonus_d17: 17, impact_bonus_d18: 18, impact_bonus_d19: 19,
    impact_bonus_d20: 20, impact_bonus_d21: 21, impact_bonus_d22: 22, impact_bonus_d23: 23, impact_bonus_d24: 24,
}

// ---------------------------------------------------------------------------------- budgets

#[kani::proof]
fn a_slot_never_pays_past_its_budget() {
    let (nominal, budget, spent) = (kani::any::<Amount>(), amount(), kani::any::<Amount>());
    let paid = within_budget(nominal, budget, spent);
    assert!(paid <= nominal);
    if spent <= budget {
        assert!(spent + paid <= budget);
    } else {
        assert!(paid == 0);
    }
}

#[kani::proof]
fn every_instant_window_has_an_index() {
    let window = kani::any_where(|w: &Time| (MIN_WINDOW..=MAX_WINDOW).contains(w));
    window_index(kani::any(), window);
}

#[kani::proof]
fn the_instant_budget_holds_in_every_window() {
    let index: u64 = kani::any();
    let budget = amount();
    let cap = kani::any_where(|c: &Amount| *c <= budget);
    let current = InstantWindow { index: kani::any(), spent: kani::any_where(|s: &Amount| *s <= budget) };
    let a: Amount = kani::any();
    if let Some(next) = instant(index, &current, a, cap, budget) {
        assert!(a <= cap);
        assert!(next.spent <= budget);
        assert!(next.index == index);
        // A new window starts empty; the same window adds up.
        let before = if index == current.index { current.spent } else { 0 };
        assert!(next.spent == before + a);
    }
}

// ---------------------------------------------------------------------------------- claims

#[kani::proof]
fn a_claim_is_covered_or_owed_never_both_lost() {
    let (want, available, blocked) = (kani::any::<Amount>(), kani::any::<Amount>(), kani::any::<bool>());
    let (reserved, debt) = cover(want, available, blocked);
    assert!(reserved + debt == want);
    assert!(reserved <= available);
    assert!(!blocked || reserved == 0);
}

#[kani::proof]
fn a_refitted_reservation_conserves_both_amounts() {
    let (reserved, amount) = (kani::any::<Amount>(), kani::any::<Amount>());
    let (kept, released, debt) = refit(reserved, amount);
    assert!(kept + released == reserved);
    assert!(kept + debt == amount);
}

// ---------------------------------------------------------------------------------- the ledger
// New escrow changes both sides of the equation at once; Kissat proves those in minutes where the
// default solver takes a quarter of an hour.

#[kani::proof]
#[kani::solver(kissat)]
fn a_deposit_keeps_the_ledger_balanced() {
    let mut l = ledger();
    l.deposit(kani::any_where(|a: &Amount| *a <= SUPPLY));
    assert!(balanced(&l));
}

#[kani::proof]
#[kani::solver(kissat)]
fn a_forfeit_keeps_the_ledger_balanced() {
    let mut l = ledger();
    l.forfeit(kani::any_where(|a: &Amount| *a <= SUPPLY));
    assert!(balanced(&l));
}

#[kani::proof]
#[kani::solver(kissat)]
fn an_award_keeps_the_ledger_balanced() {
    let mut l = ledger();
    l.award(kani::any_where(|a: &Amount| *a <= SUPPLY));
    assert!(balanced(&l));
}

#[kani::proof]
fn a_reservation_keeps_the_ledger_balanced() {
    let mut l = ledger();
    let (reserved, _) = cover(kani::any(), l.available, kani::any());
    l.reserve(reserved);
    assert!(balanced(&l));
}

#[kani::proof]
fn a_release_keeps_the_ledger_balanced() {
    let mut l = ledger();
    let r = l.reserved;
    l.unreserve(kani::any_where(|a: &Amount| *a <= r));
    assert!(balanced(&l));
}

/// A claim paid out of its reservation (`pay`, `exit`).
#[kani::proof]
fn a_payment_keeps_the_ledger_balanced() {
    let mut l = ledger();
    let r = l.reserved;
    l.pay(kani::any_where(|a: &Amount| *a <= r));
    assert!(balanced(&l));
}

/// A fee from what is available: the protocol fee (`the_protocol_fee_is_never_more_than_available`)
/// after a payment, which leaves `available` as it was, or the release fee.
#[kani::proof]
fn a_fee_keeps_the_ledger_balanced() {
    let mut l = ledger();
    let a = l.available;
    l.charge(kani::any_where(|f: &Amount| *f <= a));
    assert!(balanced(&l));
}

#[kani::proof]
fn returning_the_rest_keeps_the_ledger_balanced() {
    let mut l = ledger();
    let before = l.available;
    assert!(l.return_available() == before);
    assert!(l.available == 0);
    assert!(balanced(&l));
}

/// One step of the shortfall queue (`fund_debts`): the claim's reservation grows by what its debt
/// shrinks, and a debt is left only when nothing is available.
#[kani::proof]
fn funding_a_debt_moves_it_into_the_reservation() {
    let mut l = ledger();
    let claim_reserved = kani::any_where(|r: &Amount| *r <= l.reserved);
    let claim_debt = kani::any_where(|d: &Amount| *d <= MAX_NOMINAL);
    let (m, rest) = cover(claim_debt, l.available, false);
    l.reserve(m);
    assert!(claim_reserved + m + rest == claim_reserved + claim_debt);
    assert!(rest == 0 || l.available == 0);
    assert!(balanced(&l));
}

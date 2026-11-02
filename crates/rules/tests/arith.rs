//! Property tests for the three arithmetic facts whose 128-bit multiplications and divisions are
//! beyond Kani's solver in reasonable time (`src/proofs.rs` proves the rest): random inputs over the
//! whole range the bounds allow, plus the boundary values.

use antibody_rules::arith::*;
use antibody_rules::bounds::{MAX_AMOUNT, MAX_BPS};
use antibody_rules::Amount;
use proptest::prelude::*;

const MAX_NOMINAL: Amount = 2 * MAX_AMOUNT;

fn edges() -> Vec<Amount> {
    let mut v = vec![0, 1, 2, 9_999, 10_000, 10_001, MAX_AMOUNT - 1, MAX_AMOUNT, MAX_AMOUNT + 1, MAX_NOMINAL];
    v.extend([u128::MAX / 10_000 - 1, u128::MAX / 10_000, u128::MAX / 10_000 + 1, u128::MAX - 1, u128::MAX]);
    v
}

const EDGE_BPS: [u32; 6] = [0, 1, 1_999, 2_000, 9_999, 10_000];

#[test]
fn boundary_values() {
    for &a in &edges() {
        for b in EDGE_BPS {
            assert!(bps_of(a, b) <= a, "bps_of({a}, {b})");
            if a <= MAX_NOMINAL && b <= 2_000 {
                assert!(protocol_fee(a, b, Amount::MAX) * 5 <= a, "protocol_fee({a}, {b})");
            }
            for d in 0..=24u8 {
                for &j in &edges() {
                    if a <= j {
                        assert!(impact_bonus(a, b, MAX_AMOUNT, d) <= impact_bonus(j, b, MAX_AMOUNT, d), "bonus {a} <= {j}, {b}, {d}");
                    }
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn a_rate_never_exceeds_its_base(a in any::<u128>(), b in 0..=MAX_BPS) {
        prop_assert!(bps_of(a, b) <= a);
    }

    #[test]
    fn the_protocol_fee_is_at_most_a_fifth_of_the_payout(a in 0..=MAX_NOMINAL, b in 0..=2_000u32) {
        prop_assert!(protocol_fee(a, b, Amount::MAX) * 5 <= a);
    }

    #[test]
    fn a_larger_impact_never_earns_a_smaller_bonus(i in any::<u128>(), j in any::<u128>(), b in 0..=MAX_BPS, cap in 0..=MAX_AMOUNT, d in 0..=24u8) {
        let (lo, hi) = if i <= j { (i, j) } else { (j, i) };
        prop_assert!(impact_bonus(lo, b, cap, d) <= impact_bonus(hi, b, cap, d));
    }
}

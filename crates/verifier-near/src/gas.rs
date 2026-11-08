//! nearcore's gas counter: gas burnt by the execution, gas set aside for the receipts it creates, and the
//! limits (`max_gas_burnt` and the call's prepaid gas). Any limit hit is reported as [`GasStop`]: in v2 it
//! makes the run `Unevaluable` (docs/near-targets.md §3), so v2 never models what NEAR does after it.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GasStop {
    /// The prepaid gas or `max_gas_burnt` is exhausted.
    Exceeded,
    /// An amount overflowed u64 (nearcore's `IntegerOverflow` host error).
    Overflow,
}

#[derive(Debug, Clone)]
pub struct GasCounter {
    burnt: u64,
    promises: u64,
    max_burnt: u64,
    prepaid: u64,
    /// The burnt-gas limit: `min(max_burnt, prepaid - promises)`.
    limit: u64,
    is_view: bool,
}

impl GasCounter {
    pub fn new(max_burnt: u64, prepaid: u64, is_view: bool) -> Self {
        let prepaid = if is_view { u64::MAX } else { prepaid };
        GasCounter { burnt: 0, promises: 0, max_burnt, prepaid, limit: max_burnt.min(prepaid), is_view }
    }

    pub fn burnt(&self) -> u64 {
        self.burnt
    }

    pub fn used(&self) -> u64 {
        self.promises + self.burnt
    }

    /// Gas explicitly set aside for outgoing receipts (fees and attached gas).
    pub fn promises(&self) -> u64 {
        self.promises
    }

    pub fn prepaid(&self) -> u64 {
        self.prepaid
    }

    /// Gas still available to burn.
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.burnt)
    }

    pub fn burn(&mut self, gas: u64) -> Result<(), GasStop> {
        let new = self.burnt.checked_add(gas).ok_or(GasStop::Overflow)?;
        if new <= self.limit {
            self.burnt = new;
            Ok(())
        } else {
            self.burnt = self.limit;
            Err(GasStop::Exceeded)
        }
    }

    pub fn pay_base(&mut self, cost: u64) -> Result<(), GasStop> {
        self.burn(cost)
    }

    pub fn pay_per(&mut self, cost: u64, num: u64) -> Result<(), GasStop> {
        self.burn(num.checked_mul(cost).ok_or(GasStop::Overflow)?)
    }

    /// Burns `burn` and sets aside `use_ - burn` for receipts (nearcore's `deduct_gas`).
    pub fn pay_action(&mut self, burn: u64, use_: u64) -> Result<(), GasStop> {
        debug_assert!(burn <= use_);
        let promises = use_ - burn;
        let new_promises = self.promises.checked_add(promises).ok_or(GasStop::Overflow)?;
        let new_burnt = self.burnt.checked_add(burn).ok_or(GasStop::Overflow)?;
        let new_used = new_burnt.checked_add(new_promises).ok_or(GasStop::Overflow)?;
        if new_burnt <= self.max_burnt && new_used <= self.prepaid {
            if promises != 0 && !self.is_view {
                self.limit = self.max_burnt.min(self.prepaid - new_promises);
            }
            self.burnt = new_burnt;
            self.promises = new_promises;
            Ok(())
        } else {
            Err(GasStop::Exceeded)
        }
    }

    /// Gas attached to an outgoing function call.
    pub fn prepay(&mut self, gas: u64) -> Result<(), GasStop> {
        self.pay_action(0, gas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_like_nearcore() {
        let mut g = GasCounter::new(10, 10, false);
        g.pay_action(5, 10).unwrap();
        assert_eq!((g.burnt(), g.used()), (5, 10));
        assert_eq!(g.burn(1), Err(GasStop::Exceeded), "everything else was set aside");
        let mut v = GasCounter::new(100, 1, true);
        v.burn(50).unwrap();
        assert_eq!(v.used(), 50, "a view ignores the prepaid gas");
    }
}

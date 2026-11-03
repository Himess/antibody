//! A NEAR app that consumes Antibody's verdicts (docs/verdict-guard.md): a treasury that freezes when a
//! commit to the program it watches is verified, T1-confirmed, as a Violation of a Critical slot.
//!
//! Anyone may `report(commit_id)`. The guard then reads near-antibody itself (the commit, its
//! verification run and the program's slots, by cross-contract view calls) and decides on that state
//! alone: the reporter is trusted for nothing. While frozen, the owner's transfers are refused; only the
//! owner unfreezes (after a fix, or if a challenge overturns the verdict).

use near_sdk::{
    env, ext_contract, json_types::U128, near, require,
    serde_json::{json, Value},
    AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseError,
};

pub const VIEW_GAS: Gas = Gas::from_tgas(10);
pub const CALLBACK_GAS: Gas = Gas::from_tgas(30);

#[ext_contract(ext_antibody)]
#[allow(dead_code)]
trait Antibody {
    fn get_commit(&self, commit_id: u64) -> Option<Value>;
    fn get_run(&self, run_id: u64) -> Option<Value>;
    fn get_program_meta(&self, program_id: u32) -> Option<Value>;
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Freeze {
    pub commit_id: u64,
    pub run_id: u64,
    pub slot: u8,
    pub at_ns: u64,
    pub reported_by: AccountId,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Guard {
    owner: AccountId,
    antibody: AccountId,
    program_id: u32,
    frozen: Option<Freeze>,
}

/// The Critical slot a verification run breaks, if it is the commit's own run, T1-confirmed, and a
/// Violation of the watched program: the guard's whole decision, on near-antibody's records only.
pub fn critical_slot(program_id: u32, commit: &Value, run: &Value, meta: &Value) -> Result<Option<u8>, &'static str> {
    if commit["program"].as_u64() != Some(program_id as u64) {
        return Err("the commit is not to the watched program");
    }
    if run["id"] != commit["run"] || run["commit"] != commit["id"] {
        return Err("the run is not the commit's verification run");
    }
    let answered = &run["outcome"]["Answered"];
    if answered["tier"] != json!("T1") {
        return Err("the verdict is not T1-confirmed");
    }
    if answered["facts"]["kind"] != json!("Violation") {
        return Ok(None);
    }
    let critical: Vec<u64> = meta["slots"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s[2] == json!("critical"))
        .filter_map(|s| s[0].as_u64())
        .collect();
    Ok(answered["facts"]["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|k| k["slot"].as_u64())
        .find(|s| critical.contains(s))
        .map(|s| s as u8))
}

#[near]
impl Guard {
    #[init]
    pub fn new(owner: AccountId, antibody: AccountId, program_id: u32) -> Self {
        Self { owner, antibody, program_id, frozen: None }
    }

    /// Anyone: reads the commit, its run and the program's slots from near-antibody, and freezes the
    /// guard if the run is a T1-confirmed Violation of a Critical slot.
    pub fn report(&mut self, commit_id: u64) -> Promise {
        require!(self.frozen.is_none(), "already frozen");
        ext_antibody::ext(self.antibody.clone())
            .with_static_gas(VIEW_GAS)
            .get_commit(commit_id)
            .then(Self::ext(env::current_account_id()).with_static_gas(CALLBACK_GAS.saturating_add(VIEW_GAS).saturating_add(VIEW_GAS)).on_commit(env::predecessor_account_id()))
    }

    #[private]
    pub fn on_commit(&mut self, reporter: AccountId, #[callback_result] commit: Result<Option<Value>, PromiseError>) -> Promise {
        let c = commit.ok().flatten().unwrap_or_else(|| env::panic_str("no such commit"));
        require!(c["program"].as_u64() == Some(self.program_id as u64), "the commit is not to the watched program");
        let run_id = c["run"].as_u64().unwrap_or_else(|| env::panic_str("the commit has no verification run yet"));
        let antibody = || ext_antibody::ext(self.antibody.clone()).with_static_gas(VIEW_GAS);
        antibody()
            .get_run(run_id)
            .and(antibody().get_program_meta(self.program_id))
            .then(Self::ext(env::current_account_id()).with_static_gas(CALLBACK_GAS).on_run(reporter, c))
    }

    #[private]
    pub fn on_run(
        &mut self,
        reporter: AccountId,
        commit: Value,
        #[callback_result] run: Result<Option<Value>, PromiseError>,
        #[callback_result] meta: Result<Option<Value>, PromiseError>,
    ) -> bool {
        let run = run.ok().flatten().unwrap_or_else(|| env::panic_str("no such run"));
        let meta = meta.ok().flatten().unwrap_or_else(|| env::panic_str("no such program"));
        match critical_slot(self.program_id, &commit, &run, &meta).unwrap_or_else(|e| env::panic_str(e)) {
            Some(slot) if self.frozen.is_none() => {
                let f = Freeze {
                    commit_id: commit["id"].as_u64().unwrap_or_default(),
                    run_id: run["id"].as_u64().unwrap_or_default(),
                    slot,
                    at_ns: env::block_timestamp(),
                    reported_by: reporter,
                };
                env::log_str(&format!(
                    "EVENT_JSON:{}",
                    json!({"standard": "antibody-verdict-guard", "version": "0.1.0", "event": "frozen", "data": [f]})
                ));
                self.frozen = Some(f);
                true
            }
            _ => false,
        }
    }

    /// The owner's transfers, refused while frozen.
    #[payable]
    pub fn transfer(&mut self, to: AccountId, amount: U128) -> Promise {
        near_sdk::assert_one_yocto();
        require!(env::predecessor_account_id() == self.owner, "owner only");
        require!(self.frozen.is_none(), "frozen: a Critical was verified for the watched program");
        Promise::new(to).transfer(NearToken::from_yoctonear(amount.0))
    }

    #[payable]
    pub fn unfreeze(&mut self) {
        near_sdk::assert_one_yocto();
        require!(env::predecessor_account_id() == self.owner, "owner only");
        self.frozen = None;
    }

    pub fn get_state(&self) -> Value {
        json!({"owner": self.owner, "antibody": self.antibody, "program_id": self.program_id, "frozen": self.frozen})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit() -> Value {
        json!({"id": 7, "program": 0, "run": 12, "state": "Verifying"})
    }

    fn run(tier: &str, kind: &str, slots: &[u64]) -> Value {
        let keys: Vec<Value> = slots.iter().map(|s| json!({"slot": s, "tag": "00"})).collect();
        json!({"id": 12, "program": 0, "commit": 7, "outcome": {"Answered": {"tier": tier, "facts": {"kind": kind, "keys": keys}}}})
    }

    fn meta() -> Value {
        json!({"slots": [[0, "solvency", "critical"], [1, "fees_to_treasury", "high"], [2, "share_price", "medium"]]})
    }

    #[test]
    fn a_t1_violation_of_a_critical_slot_freezes() {
        assert_eq!(critical_slot(0, &commit(), &run("T1", "Violation", &[0]), &meta()), Ok(Some(0)));
    }

    #[test]
    fn only_critical_t1_violations_of_the_watched_program_count() {
        assert_eq!(critical_slot(0, &commit(), &run("T1", "Violation", &[1, 2]), &meta()), Ok(None), "High and Medium do not freeze");
        assert_eq!(critical_slot(0, &commit(), &run("T1", "NoViolation", &[]), &meta()), Ok(None));
        assert!(critical_slot(0, &commit(), &run("T0", "Violation", &[0]), &meta()).is_err(), "an unconfirmed verdict does not count");
        assert!(critical_slot(1, &commit(), &run("T1", "Violation", &[0]), &meta()).is_err(), "another program");
        let mut other = run("T1", "Violation", &[0]);
        other["id"] = json!(13);
        assert!(critical_slot(0, &commit(), &other, &meta()).is_err(), "a run that is not the commit's");
    }

    #[test]
    fn the_owner_alone_transfers_and_never_while_frozen() {
        use near_sdk::{test_utils::VMContextBuilder, testing_env};
        let ctx = |who: &str| {
            let mut b = VMContextBuilder::new();
            b.current_account_id("guard.testnet".parse().unwrap())
                .predecessor_account_id(who.parse().unwrap())
                .attached_deposit(NearToken::from_yoctonear(1))
                .account_balance(NearToken::from_near(10));
            testing_env!(b.build());
        };
        ctx("dao.testnet");
        let mut g = Guard::new("dao.testnet".parse().unwrap(), "bounty.testnet".parse().unwrap(), 0);
        let _ = g.transfer("bob.testnet".parse().unwrap(), U128(1));
        g.frozen = Some(Freeze { commit_id: 7, run_id: 12, slot: 0, at_ns: 0, reported_by: "anyone.testnet".parse().unwrap() });
        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = g.transfer("bob.testnet".parse().unwrap(), U128(1));
        }));
        assert!(refused.is_err(), "frozen");
        g.unfreeze();
        assert!(g.frozen.is_none());
    }

    // near-sdk declares the yield/resume host functions; Windows links the test binary against these
    // symbols anyway (never called).
    #[cfg(windows)]
    #[no_mangle]
    pub extern "C" fn promise_yield_create(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> u64 {
        unreachable!()
    }
    #[cfg(windows)]
    #[no_mangle]
    pub extern "C" fn promise_yield_resume(_: u64, _: u64, _: u64, _: u64) -> u32 {
        unreachable!()
    }
}

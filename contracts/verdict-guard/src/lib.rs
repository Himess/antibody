//! A NEAR app that consumes Antibody's verdicts (docs/verdict-guard.md): a treasury that freezes when a
//! commit to the program it watches is the first finding of a Critical slot's key, verified and
//! T1-confirmed against a light-client-confirmed anchor.
//!
//! Anyone may `report(commit_id)`. The guard then reads near-antibody itself (the commit, its
//! verification run, the program and, before allocation, the key's holder, by cross-contract view calls)
//! and decides on that state alone: the reporter is trusted for nothing. While frozen, the owner's
//! transfers are refused; only the owner unfreezes (after a fix, or if a challenge overturns the verdict).

use near_sdk::{
    env, ext_contract, json_types::U128, near, require,
    serde_json::{json, Value},
    AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseError, PromiseOrValue,
};

pub const VIEW_GAS: Gas = Gas::from_tgas(10);
pub const CALLBACK_GAS: Gas = Gas::from_tgas(30);
/// `on_run` may read the key's holder and decide in `on_holder`.
pub const ON_RUN_GAS: Gas = Gas::from_tgas(30 + 10 + 30);
/// `on_commit` reads the run and the program, then `on_run`.
pub const ON_COMMIT_GAS: Gas = Gas::from_tgas(30 + 10 + 10 + 70);

#[ext_contract(ext_antibody)]
#[allow(dead_code)]
trait Antibody {
    fn get_commit(&self, commit_id: u64) -> Option<Value>;
    fn get_run(&self, run_id: u64) -> Option<Value>;
    fn get_program(&self, program_id: u32) -> Option<Value>;
    fn get_holder(&self, program_id: u32, round: u32, tag: String) -> Option<u64>;
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

/// What a report shows about a Critical slot, on near-antibody's records alone.
#[derive(Debug, PartialEq)]
pub enum Finding {
    /// Not the first finding of a Critical: a NoViolation, other severities, a duplicate, a known issue.
    None,
    /// The first finding of a Critical slot's key: allocated to this commit (held or paid).
    First { slot: u8 },
    /// A Critical slot's key in a commit not allocated yet: the first finding unless another commit
    /// holds the key.
    Unallocated { slot: u8, round: u32, tag: String },
}

/// The guard's decision: the commit's own verification run, T1-confirmed, a Violation of a slot the
/// program prices Critical now (rounds add slots), in a program whose anchor the light client confirmed.
pub fn critical_finding(program_id: u32, commit: &Value, run: &Value, program: &Value) -> Result<Finding, &'static str> {
    if commit["program"].as_u64() != Some(program_id as u64) || program["id"].as_u64() != Some(program_id as u64) {
        return Err("the commit is not to the watched program");
    }
    if run["id"] != commit["run"] || run["commit"] != commit["id"] {
        return Err("the run is not the commit's verification run");
    }
    if program["anchor"] != json!("LightClient") {
        return Err("the light client did not confirm the program's anchor: its verdicts are about state the project attested");
    }
    let answered = &run["outcome"]["Answered"];
    if answered["tier"] != json!("T1") {
        return Err("the verdict is not T1-confirmed");
    }
    if answered["facts"]["kind"] != json!("Violation") {
        return Ok(Finding::None);
    }
    let critical: Vec<u64> = program["params"]["slots"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["severity"] == json!("critical"))
        .filter_map(|s| s["slot"].as_u64())
        .collect();
    let Some(key) = answered["facts"]["keys"].as_array().into_iter().flatten().find(|k| k["slot"].as_u64().is_some_and(|s| critical.contains(&s)))
    else {
        return Ok(Finding::None);
    };
    let slot = key["slot"].as_u64().unwrap_or_default() as u8;
    // Only the first finding of a key freezes: its duplicates and a known issue's matches were acted on.
    let state = &commit["state"];
    if state.get("Holder").is_some() || state["Done"].get("Paid").is_some() {
        Ok(Finding::First { slot })
    } else if state == "Verifying" {
        let tag = key["tag"].as_str().ok_or("a key without a tag")?.to_string();
        Ok(Finding::Unallocated { slot, round: commit["round"].as_u64().unwrap_or_default() as u32, tag })
    } else {
        Ok(Finding::None)
    }
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
            .then(Self::ext(env::current_account_id()).with_static_gas(ON_COMMIT_GAS).on_commit(env::predecessor_account_id()))
    }

    #[private]
    pub fn on_commit(&mut self, reporter: AccountId, #[callback_result] commit: Result<Option<Value>, PromiseError>) -> Promise {
        let c = commit.ok().flatten().unwrap_or_else(|| env::panic_str("no such commit"));
        require!(c["program"].as_u64() == Some(self.program_id as u64), "the commit is not to the watched program");
        let run_id = c["run"].as_u64().unwrap_or_else(|| env::panic_str("the commit has no verification run yet"));
        let antibody = || ext_antibody::ext(self.antibody.clone()).with_static_gas(VIEW_GAS);
        antibody()
            .get_run(run_id)
            .and(antibody().get_program(self.program_id))
            .then(Self::ext(env::current_account_id()).with_static_gas(ON_RUN_GAS).on_run(reporter, c))
    }

    #[private]
    pub fn on_run(
        &mut self,
        reporter: AccountId,
        commit: Value,
        #[callback_result] run: Result<Option<Value>, PromiseError>,
        #[callback_result] program: Result<Option<Value>, PromiseError>,
    ) -> PromiseOrValue<bool> {
        let run = run.ok().flatten().unwrap_or_else(|| env::panic_str("no such run"));
        let program = program.ok().flatten().unwrap_or_else(|| env::panic_str("no such program"));
        let commit_id = commit["id"].as_u64().unwrap_or_default();
        let run_id = run["id"].as_u64().unwrap_or_default();
        match critical_finding(self.program_id, &commit, &run, &program).unwrap_or_else(|e| env::panic_str(e)) {
            Finding::None => PromiseOrValue::Value(false),
            Finding::First { slot } => PromiseOrValue::Value(self.freeze(commit_id, run_id, slot, reporter)),
            // Not allocated yet: a duplicate of a held key does not freeze.
            Finding::Unallocated { slot, round, tag } => PromiseOrValue::Promise(
                ext_antibody::ext(self.antibody.clone())
                    .with_static_gas(VIEW_GAS)
                    .get_holder(self.program_id, round, tag)
                    .then(Self::ext(env::current_account_id()).with_static_gas(CALLBACK_GAS).on_holder(reporter, commit_id, run_id, slot)),
            ),
        }
    }

    #[private]
    pub fn on_holder(
        &mut self,
        reporter: AccountId,
        commit_id: u64,
        run_id: u64,
        slot: u8,
        #[callback_result] holder: Result<Option<u64>, PromiseError>,
    ) -> bool {
        let holder = holder.unwrap_or_else(|_| env::panic_str("the holder view failed"));
        if holder.is_some_and(|h| h != commit_id) {
            return false;
        }
        self.freeze(commit_id, run_id, slot, reporter)
    }

    fn freeze(&mut self, commit_id: u64, run_id: u64, slot: u8, reporter: AccountId) -> bool {
        if self.frozen.is_some() {
            return false;
        }
        let f = Freeze { commit_id, run_id, slot, at_ns: env::block_timestamp(), reported_by: reporter };
        env::log_str(&format!(
            "EVENT_JSON:{}",
            json!({"standard": "antibody-verdict-guard", "version": "0.1.0", "event": "frozen", "data": [f]})
        ));
        self.frozen = Some(f);
        true
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

    fn commit(state: Value) -> Value {
        json!({"id": 7, "program": 0, "round": 1, "run": 12, "state": state})
    }

    fn paid() -> Value {
        commit(json!({"Done": {"Paid": {"amount": "5000", "instant": true}}}))
    }

    fn run(tier: &str, kind: &str, slots: &[u64]) -> Value {
        let keys: Vec<Value> = slots.iter().map(|s| json!({"slot": s, "tag": format!("{s:02}").repeat(32)})).collect();
        json!({"id": 12, "program": 0, "commit": 7, "outcome": {"Answered": {"tier": tier, "facts": {"kind": kind, "keys": keys}}}})
    }

    /// Slot 3 came with round 1.
    fn program(anchor: &str) -> Value {
        json!({"id": 0, "anchor": anchor, "params": {"slots": [
            {"slot": 0, "severity": "critical", "budget": "1"}, {"slot": 1, "severity": "high", "budget": "1"},
            {"slot": 2, "severity": "medium", "budget": "1"}, {"slot": 3, "severity": "critical", "budget": "1"}]}})
    }

    #[test]
    fn the_first_finding_of_a_critical_freezes() {
        let lc = program("LightClient");
        assert_eq!(critical_finding(0, &paid(), &run("T1", "Violation", &[0]), &lc), Ok(Finding::First { slot: 0 }));
        assert_eq!(critical_finding(0, &paid(), &run("T1", "Violation", &[3]), &lc), Ok(Finding::First { slot: 3 }), "a later round's slot");
        let held = commit(json!({"Holder": {"amount": "5000", "reserved": "5000", "debt": "0", "new_keys": []}}));
        assert_eq!(critical_finding(0, &held, &run("T1", "Violation", &[0]), &lc), Ok(Finding::First { slot: 0 }));
        // Before allocation the guard asks near-antibody who holds the key.
        assert_eq!(
            critical_finding(0, &commit(json!("Verifying")), &run("T1", "Violation", &[0]), &lc),
            Ok(Finding::Unallocated { slot: 0, round: 1, tag: "00".repeat(32) })
        );
    }

    #[test]
    fn only_first_critical_t1_findings_against_a_confirmed_anchor_count() {
        let lc = program("LightClient");
        assert_eq!(critical_finding(0, &paid(), &run("T1", "Violation", &[1, 2]), &lc), Ok(Finding::None), "High and Medium do not freeze");
        assert_eq!(critical_finding(0, &paid(), &run("T1", "NoViolation", &[]), &lc), Ok(Finding::None));
        let dup = commit(json!({"Duplicate": {"of": 3, "amount": "0", "reserved": "0"}}));
        assert_eq!(critical_finding(0, &dup, &run("T1", "Violation", &[0]), &lc), Ok(Finding::None), "a duplicate");
        assert_eq!(critical_finding(0, &commit(json!({"Done": "Known"})), &run("T1", "Violation", &[0]), &lc), Ok(Finding::None), "a known issue");
        assert!(critical_finding(0, &paid(), &run("T0", "Violation", &[0]), &lc).is_err(), "an unconfirmed verdict does not count");
        assert!(critical_finding(0, &paid(), &run("T1", "Violation", &[0]), &program("Unconfirmed")).is_err(), "a project-attested anchor");
        assert!(critical_finding(1, &paid(), &run("T1", "Violation", &[0]), &lc).is_err(), "another program");
        let mut other = run("T1", "Violation", &[0]);
        other["id"] = json!(13);
        assert!(critical_finding(0, &paid(), &other, &lc).is_err(), "a run that is not the commit's");
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

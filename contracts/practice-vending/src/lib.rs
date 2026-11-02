//! Practice accounts for Antibody's practice program (docs/practice.md).
//!
//! `vend(name, public_key)` creates `<name>.<this account>` with a little NEAR for gas and one key:
//! a function-call key for near-antibody's `commit`, `reveal` and `retry` (and FastFS uploads) only. The
//! account is registered as an agent of this contract, so its bonds come from this contract's credit
//! within a small cap and its payouts come back here; it cannot withdraw. Each account is assigned its
//! own tutorial vault. Vending is rate-limited per hour and the amounts are small; the owner refills.
//!
//! Arguments and views are Borsh (the CLI encodes them): with no JSON the contract is half the size,
//! and its storage stake half as large.

use near_sdk::{
    env, near, require,
    json_types::U128,
    store::LookupMap,
    AccountId, Gas, NearToken, PanicOnDefault, Promise, PublicKey,
};

/// near-antibody methods a practice key may call (as an agent key, `cmd::AGENT_METHODS`).
pub const AGENT_METHODS: &str = "commit,reveal,retry,__fastdata_fastfs";
pub const CALL_GAS: Gas = Gas::from_tgas(20);

#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Terms {
    /// NEAR sent to the new account for its own gas (its key's allowance draws on it).
    pub gas_funds: U128,
    /// The key's allowance.
    pub allowance: U128,
    /// NEAR credit deposited at near-antibody for the account's verification runs.
    pub near_credit: U128,
    /// Token units of this contract's credit the account may bond in all, and per commit.
    pub bond_budget: U128,
    pub per_commit_cap: U128,
    /// Accounts vended per hour, at most.
    pub per_hour: u32,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Vending {
    owner: AccountId,
    antibody: AccountId,
    program_id: u32,
    vaults: u32,
    terms: Terms,
    vended: u32,
    hour: u64,
    in_hour: u32,
    accounts: LookupMap<AccountId, u32>,
}

#[near]
impl Vending {
    #[init]
    pub fn new(#[serializer(borsh)] owner: AccountId, #[serializer(borsh)] antibody: AccountId, #[serializer(borsh)] program_id: u32, #[serializer(borsh)] vaults: u32, #[serializer(borsh)] terms: Terms) -> Self {
        require!(vaults > 0, "at least one vault");
        Self { owner, antibody, program_id, vaults, terms, vended: 0, hour: 0, in_hour: 0, accounts: LookupMap::new(b"a") }
    }

    /// Creates the practice account `<name>.<this account>` for `public_key` and assigns it a vault.
    pub fn vend(&mut self, #[serializer(borsh)] name: String, #[serializer(borsh)] public_key: PublicKey) -> Promise {
        require!(
            !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'),
            "a name is 1 to 32 of a-z, 0-9, - and _"
        );
        let hour = env::block_timestamp() / 3_600_000_000_000;
        if hour != self.hour {
            self.hour = hour;
            self.in_hour = 0;
        }
        require!(self.in_hour < self.terms.per_hour, "the hour's practice accounts are vended; try again later");
        self.in_hour += 1;
        let account: AccountId = format!("{name}.{}", env::current_account_id()).parse().unwrap_or_else(|_| env::panic_str("invalid name"));
        require!(!self.accounts.contains_key(&account), "that practice account exists");
        let vault = self.vended % self.vaults;
        self.vended += 1;
        self.accounts.insert(account.clone(), vault);
        let t = self.terms.clone();
        env::log_str(&format!(
            "EVENT_JSON:{{\"standard\":\"antibody-practice\",\"version\":\"0.1.0\",\"event\":\"vended\",\"data\":[{{\"account\":\"{account}\",\"vault\":{vault},\"program_id\":{}}}]}}",
            self.program_id
        ));
        let create = Promise::new(account.clone())
            .create_account()
            .transfer(NearToken::from_yoctonear(t.gas_funds.0))
            .add_access_key_allowance(public_key, near_sdk::Allowance::limited(NearToken::from_yoctonear(t.allowance.0)).unwrap(), self.antibody.clone(), AGENT_METHODS.into());
        let credit = Promise::new(self.antibody.clone()).function_call(
            "storage_deposit".into(),
            format!("{{\"account_id\":\"{account}\"}}").into_bytes(),
            NearToken::from_yoctonear(t.near_credit.0),
            CALL_GAS,
        );
        let agent = Promise::new(self.antibody.clone()).function_call(
            "register_agent".into(),
            format!("{{\"agent\":\"{account}\",\"bond_budget\":\"{}\",\"per_commit_cap\":\"{}\"}}", t.bond_budget.0, t.per_commit_cap.0).into_bytes(),
            NearToken::from_yoctonear(1),
            CALL_GAS,
        );
        create.then(credit).then(agent)
    }

    /// The tutorial vault assigned to a practice account.
    #[result_serializer(borsh)]
    pub fn vault_of(&self, #[serializer(borsh)] account: AccountId) -> Option<u32> {
        self.accounts.get(&account).copied()
    }

    #[result_serializer(borsh)]
    pub fn get_config(&self) -> (AccountId, AccountId, u32, u32, Terms, u32) {
        (self.owner.clone(), self.antibody.clone(), self.program_id, self.vaults, self.terms.clone(), self.vended)
    }

    /// The owner: new amounts or a new program (refills arrive as plain transfers and token deposits).
    pub fn set_terms(&mut self, #[serializer(borsh)] program_id: u32, #[serializer(borsh)] vaults: u32, #[serializer(borsh)] terms: Terms) {
        require!(env::predecessor_account_id() == self.owner, "owner only");
        require!(vaults > 0, "at least one vault");
        self.program_id = program_id;
        self.vaults = vaults;
        self.terms = terms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::{test_utils::VMContextBuilder, testing_env};

    fn terms(per_hour: u32) -> Terms {
        let n = |x: u128| U128(x);
        Terms { gas_funds: n(10), allowance: n(5), near_credit: n(3), bond_budget: n(100), per_commit_cap: n(10), per_hour }
    }

    fn ctx(now_ns: u64) {
        let mut b = VMContextBuilder::new();
        b.current_account_id("practice.antibody.testnet".parse().unwrap())
            .predecessor_account_id("judge.testnet".parse().unwrap())
            .account_balance(NearToken::from_near(10))
            .block_timestamp(now_ns);
        testing_env!(b.build());
    }

    fn pk() -> PublicKey {
        "ed25519:6E8sCci9badyRkXb3JoRpBj5p8C6Tw41ELDZoiihKEtp".parse().unwrap()
    }

    fn vending(per_hour: u32) -> Vending {
        Vending::new("antibody.testnet".parse().unwrap(), "bounty.antibody.testnet".parse().unwrap(), 1, 3, terms(per_hour))
    }

    #[test]
    fn each_account_gets_the_next_vault() {
        ctx(0);
        let mut v = vending(10);
        for name in ["ann", "bob", "cat", "dan"] {
            ctx(0); // one transaction each
            let _ = v.vend(name.into(), pk());
        }
        let of = |n: &str| v.vault_of(format!("{n}.practice.antibody.testnet").parse().unwrap());
        assert_eq!((of("ann"), of("bob"), of("cat"), of("dan")), (Some(0), Some(1), Some(2), Some(0)));
    }

    #[test]
    #[should_panic(expected = "that practice account exists")]
    fn a_name_is_vended_once() {
        ctx(0);
        let mut v = vending(10);
        let _ = v.vend("ann".into(), pk());
        let _ = v.vend("ann".into(), pk());
    }

    #[test]
    fn vending_is_rate_limited_per_hour() {
        ctx(0);
        let mut v = vending(2);
        let _ = v.vend("ann".into(), pk());
        ctx(0);
        let _ = v.vend("bob".into(), pk());
        ctx(0);
        let third = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = v.vend("cat".into(), pk());
        }));
        assert!(third.is_err(), "the third in the hour is refused");
        ctx(3_600_000_000_000);
        let _ = v.vend("cat".into(), pk());
        assert_eq!(v.vault_of("cat.practice.antibody.testnet".parse().unwrap()), Some(2));
    }

    #[test]
    #[should_panic(expected = "a name is 1 to 32")]
    fn names_are_plain() {
        ctx(0);
        let _ = vending(10).vend("Ann.evil".into(), pk());
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

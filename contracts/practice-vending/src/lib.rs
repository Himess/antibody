//! Practice accounts for Antibody's practice program (docs/practice.md).
//!
//! `vend(name, public_key)` creates `<name>.<this account>` with the NEAR its gas sets aside, the
//! practice-account contract (contracts/practice-account) and one key: a function-call key for that
//! account's own `commit`, `reveal`, `retry`, `allocate`, `pay` and `close`. The first five go on to
//! near-antibody unchanged; `close` marks the try done here, and `reclaim` then deletes the account,
//! its NEAR back here, so a try costs this contract only the gas it burnt. The account is registered
//! as an agent of this contract, so
//! near-antibody takes its bonds, its verification runs and its storage from this contract's credit
//! there (the bonds within a small cap), and its payouts come back to that credit; it cannot withdraw.
//! Each account is assigned its own tutorial vault. Vending is rate-limited per hour; an account left
//! open may be reclaimed by anyone an hour after its vend.
//!
//! Anyone may call `vend`. The operator also adds a deliberately public function-call key for `vend`
//! and `reclaim` on this account, within an allowance, so a first try needs no NEAR account
//! (`antibody try`).
//!
//! Arguments and views are Borsh (the CLI encodes them): with no JSON the contract is half the size,
//! and its storage stake half as large.

use near_sdk::{
    env, near, require,
    json_types::U128,
    store::LookupMap,
    AccountId, Gas, NearToken, PanicOnDefault, Promise, PublicKey,
};

/// The practice account's own methods its key may call (its contract passes the first five on to
/// near-antibody: an agent key's and the permissionless `allocate` and `pay`, so the account settles its
/// own payout).
pub const PRACTICE_METHODS: &str = "commit,reveal,retry,allocate,pay,close";
pub const CALL_GAS: Gas = Gas::from_tgas(20);
pub const FUND_GAS: Gas = Gas::from_tgas(150);
/// The practice-account contract's `init` (it stores the near-antibody account) and `close`.
pub const INIT_GAS: Gas = Gas::from_tgas(5);
pub const CLOSE_GAS: Gas = Gas::from_tgas(10);
/// How long a practice account stays before anyone may close it (a try takes a minute or two).
pub const RECLAIM_AFTER_SECS: u64 = 3_600;

/// A vended practice account: its tutorial vault, when it was vended (seconds), and whether its try is
/// done (its key called `close`).
#[near(serializers = [borsh])]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vended {
    pub vault: u32,
    pub at: u64,
    pub done: bool,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Terms {
    /// NEAR sent to the new account: its contract's storage and its calls' gas, which NEAR sets aside
    /// up front (`close` returns what is left).
    pub gas_funds: U128,
    /// The key's allowance: each call's set-aside gas counts against it.
    pub allowance: U128,
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
    token: AccountId,
    program_id: u32,
    vaults: u32,
    terms: Terms,
    vended: u32,
    hour: u64,
    in_hour: u32,
    accounts: LookupMap<AccountId, Vended>,
    /// The practice-account contract each vend deploys.
    account_code: Vec<u8>,
}

#[near]
impl Vending {
    #[init]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        #[serializer(borsh)] owner: AccountId,
        #[serializer(borsh)] antibody: AccountId,
        #[serializer(borsh)] token: AccountId,
        #[serializer(borsh)] program_id: u32,
        #[serializer(borsh)] vaults: u32,
        #[serializer(borsh)] terms: Terms,
        #[serializer(borsh)] account_code: Vec<u8>,
    ) -> Self {
        require!(vaults > 0, "at least one vault");
        require!(account_code.starts_with(b"\0asm"), "the practice-account contract (wasm)");
        Self { owner, antibody, token, program_id, vaults, terms, vended: 0, hour: 0, in_hour: 0, accounts: LookupMap::new(b"a"), account_code }
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
        self.accounts.insert(account.clone(), Vended { vault, at: env::block_timestamp() / 1_000_000_000, done: false });
        let t = self.terms.clone();
        env::log_str(&format!(
            "EVENT_JSON:{{\"standard\":\"antibody-practice\",\"version\":\"0.1.0\",\"event\":\"vended\",\"data\":[{{\"account\":\"{account}\",\"vault\":{vault},\"program_id\":{}}}]}}",
            self.program_id
        ));
        let allowance = near_sdk::Allowance::limited(NearToken::from_yoctonear(t.allowance.0)).unwrap_or_else(|| env::panic_str("the allowance must be positive"));
        // The key calls only the account's own contract, which passes the near-antibody calls on.
        let create = Promise::new(account.clone())
            .create_account()
            .transfer(NearToken::from_yoctonear(t.gas_funds.0))
            .deploy_contract(self.account_code.clone())
            .function_call("init".into(), self.antibody.as_bytes().to_vec(), NearToken::from_yoctonear(0), INIT_GAS)
            .add_access_key_allowance(public_key, allowance, account.clone(), PRACTICE_METHODS.into());
        let agent = Promise::new(self.antibody.clone()).function_call(
            "register_agent".into(),
            // The practice account may commit to the practice program only.
            format!(
                "{{\"agent\":\"{account}\",\"bond_budget\":\"{}\",\"per_commit_cap\":\"{}\",\"programs\":[{}]}}",
                t.bond_budget.0, t.per_commit_cap.0, self.program_id
            )
            .into_bytes(),
            NearToken::from_yoctonear(1),
            CALL_GAS,
        );
        create.then(agent)
    }

    /// The owner moves `amount` of this contract's tokens into its credit at near-antibody, where the
    /// practice accounts' bonds come from (their payouts return there).
    #[payable]
    pub fn fund(&mut self, #[serializer(borsh)] amount: U128) -> Promise {
        near_sdk::assert_one_yocto();
        require!(env::predecessor_account_id() == self.owner, "owner only");
        // `msg` is near-antibody's token message `"deposit"`, itself a JSON string.
        Promise::new(self.token.clone()).function_call(
            "ft_transfer_call".into(),
            format!("{{\"receiver_id\":\"{}\",\"amount\":\"{}\",\"msg\":\"\\\"deposit\\\"\"}}", self.antibody, amount.0).into_bytes(),
            NearToken::from_yoctonear(1),
            FUND_GAS,
        )
    }

    /// A practice account's try is done: its contract calls this when its key calls `close`, so that
    /// `reclaim` may close the account at once.
    pub fn done(&mut self) {
        let account = env::predecessor_account_id();
        let mut v = self.accounts.get(&account).copied().unwrap_or_else(|| env::panic_str("not a practice account vended here"));
        v.done = true;
        self.accounts.insert(account, v);
    }

    /// Anyone: closes a practice account whose try is done, or one vended at least
    /// [`RECLAIM_AFTER_SECS`] ago, its NEAR back here. `antibody try` calls it with the public vend key:
    /// an account deleted in a transaction it signed itself would lose that transaction's gas refunds.
    pub fn reclaim(&mut self, #[serializer(borsh)] account: AccountId) -> Promise {
        let v = self.accounts.get(&account).copied().unwrap_or_else(|| env::panic_str("not a practice account vended here"));
        require!(
            v.done || env::block_timestamp() / 1_000_000_000 >= v.at + RECLAIM_AFTER_SECS,
            "a practice account may be reclaimed once its try is done, or an hour after its vend"
        );
        Promise::new(account).function_call("close".into(), vec![], NearToken::from_yoctonear(0), CLOSE_GAS)
    }

    /// The tutorial vault assigned to a practice account.
    #[result_serializer(borsh)]
    pub fn vault_of(&self, #[serializer(borsh)] account: AccountId) -> Option<u32> {
        self.accounts.get(&account).map(|v| v.vault)
    }

    /// The owner: a new practice-account contract for later vends.
    #[payable]
    pub fn set_account_code(&mut self, #[serializer(borsh)] account_code: Vec<u8>) {
        near_sdk::assert_one_yocto();
        require!(env::predecessor_account_id() == self.owner, "owner only");
        require!(account_code.starts_with(b"\0asm"), "the practice-account contract (wasm)");
        self.account_code = account_code;
    }

    #[result_serializer(borsh)]
    pub fn get_config(&self) -> (AccountId, AccountId, AccountId, u32, u32, Terms, u32) {
        (self.owner.clone(), self.antibody.clone(), self.token.clone(), self.program_id, self.vaults, self.terms.clone(), self.vended)
    }

    /// The owner: new amounts or a new program (NEAR refills arrive as plain transfers, tokens with `fund`).
    #[payable]
    pub fn set_terms(&mut self, #[serializer(borsh)] program_id: u32, #[serializer(borsh)] vaults: u32, #[serializer(borsh)] terms: Terms) {
        near_sdk::assert_one_yocto();
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
        Terms { gas_funds: n(10), allowance: n(5), bond_budget: n(100), per_commit_cap: n(10), per_hour }
    }

    fn ctx_as(now_ns: u64, caller: &str, deposit: u128) {
        let mut b = VMContextBuilder::new();
        b.current_account_id("practice.antibody.testnet".parse().unwrap())
            .predecessor_account_id(caller.parse().unwrap())
            .account_balance(NearToken::from_near(10))
            .attached_deposit(NearToken::from_yoctonear(deposit))
            .block_timestamp(now_ns);
        testing_env!(b.build());
    }

    fn ctx(now_ns: u64) {
        ctx_as(now_ns, "judge.testnet", 0);
    }

    fn pk() -> PublicKey {
        "ed25519:6E8sCci9badyRkXb3JoRpBj5p8C6Tw41ELDZoiihKEtp".parse().unwrap()
    }

    fn code() -> Vec<u8> {
        b"\0asm\x01\0\0\0".to_vec()
    }

    fn vending(per_hour: u32) -> Vending {
        Vending::new("antibody.testnet".parse().unwrap(), "bounty.antibody.testnet".parse().unwrap(), "wrap.testnet".parse().unwrap(), 1, 3, terms(per_hour), code())
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

    /// The Borsh layout `antibody try` writes for `vend` (crates/cli/src/practice.rs `vend_args`).
    #[test]
    fn vend_args_layout() {
        let key = pk();
        let bytes = near_sdk::borsh::to_vec(&("ann".to_string(), key.clone())).unwrap();
        let mut want = vec![3, 0, 0, 0, b'a', b'n', b'n', 33, 0, 0, 0, 0];
        want.extend(&key.as_bytes()[1..]);
        assert_eq!(bytes, want);
    }

    /// golden/vectors.json (written by crates/cli/tests/golden.rs).
    #[test]
    fn golden_vend_args_match_the_cli() {
        let g: near_sdk::serde_json::Value = near_sdk::serde_json::from_str(include_str!("../../../golden/vectors.json")).unwrap();
        let v = &g["vend_args"];
        let key_hex = v["public_key"].as_str().unwrap();
        let key_bytes: Vec<u8> = (0..key_hex.len()).step_by(2).map(|i| u8::from_str_radix(&key_hex[i..i + 2], 16).unwrap()).collect();
        let mut data = vec![0u8];
        data.extend(&key_bytes);
        let pk = PublicKey::try_from(data).unwrap();
        let bytes = near_sdk::borsh::to_vec(&(v["name"].as_str().unwrap().to_string(), pk)).unwrap();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, v["borsh"].as_str().unwrap());
    }

    #[test]
    #[should_panic(expected = "owner only")]
    fn only_the_owner_funds() {
        ctx_as(0, "judge.testnet", 1);
        let _ = vending(10).fund(U128(5));
    }

    #[test]
    fn the_owner_funds_and_sets_terms() {
        ctx_as(0, "antibody.testnet", 1);
        let mut v = vending(10);
        let _ = v.fund(U128(5));
        v.set_terms(2, 24, terms(3));
        let (.., program, vaults, t, _) = v.get_config();
        assert_eq!((program, vaults, t.per_hour), (2, 24, 3));
    }

    #[test]
    fn an_account_left_open_is_reclaimed_after_an_hour_by_anyone() {
        let at = 1_800_000_000 * 1_000_000_000u64;
        ctx(at);
        let mut v = vending(10);
        let _ = v.vend("ann".into(), pk());
        let ann: AccountId = "ann.practice.antibody.testnet".parse().unwrap();
        ctx(at + 3_599 * 1_000_000_000);
        let early = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = v.reclaim(ann.clone());
        }));
        assert!(early.is_err(), "within the hour a try may still be running");
        ctx(at + 3_600 * 1_000_000_000);
        let _ = v.reclaim(ann);
    }

    #[test]
    fn an_account_whose_try_is_done_is_reclaimed_at_once() {
        ctx(0);
        let mut v = vending(10);
        let _ = v.vend("ann".into(), pk());
        let _ = v.vend("bob".into(), pk());
        ctx_as(1_000_000_000, "ann.practice.antibody.testnet", 0);
        v.done();
        ctx(1_000_000_000);
        let _ = v.reclaim("ann.practice.antibody.testnet".parse().unwrap());
        let bob = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = v.reclaim("bob.practice.antibody.testnet".parse().unwrap());
        }));
        assert!(bob.is_err(), "bob's try is not done");
    }

    #[test]
    #[should_panic(expected = "not a practice account vended here")]
    fn only_vended_accounts_mark_a_try_done() {
        ctx(0);
        vending(10).done();
    }

    #[test]
    #[should_panic(expected = "not a practice account vended here")]
    fn only_vended_accounts_are_reclaimed() {
        ctx(0);
        let _ = vending(10).reclaim("judge.testnet".parse().unwrap());
    }

    #[test]
    fn the_owner_alone_replaces_the_account_code() {
        ctx_as(0, "antibody.testnet", 1);
        let mut v = vending(10);
        v.set_account_code(code());
        ctx_as(0, "judge.testnet", 1);
        let other = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.set_account_code(code())));
        assert!(other.is_err());
    }

    #[test]
    #[should_panic(expected = "the practice-account contract (wasm)")]
    fn the_account_code_is_wasm() {
        ctx(0);
        let _ = Vending::new("antibody.testnet".parse().unwrap(), "bounty.antibody.testnet".parse().unwrap(), "wrap.testnet".parse().unwrap(), 1, 3, terms(1), vec![1, 2]);
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

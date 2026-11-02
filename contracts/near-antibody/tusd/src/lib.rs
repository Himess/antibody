//! tUSD — a NEP-141 testnet token with **no value**, used for Antibody escrow, bonds and payouts.
//!
//! - NEP-141 core (`ft_transfer`, `ft_transfer_call` + `ft_resolve_transfer`, `ft_total_supply`,
//!   `ft_balance_of`), NEP-148 metadata, NEP-297 events (`nep141` `ft_mint` / `ft_transfer`).
//! - Accounts are registered automatically (the token pays its own storage); NEP-145 calls are
//!   accepted for wallet compatibility and refund any deposit.
//! - The owner mints; anyone may take a rate-limited faucet drip (testnet only).

use near_sdk::{
    env,
    json_types::U128,
    log, near, require,
    serde_json::json,
    store::LookupMap,
    AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseResult,
};

pub const GAS_FOR_RESOLVE: Gas = Gas::from_tgas(5);
pub const GAS_FOR_FT_TRANSFER_CALL: Gas = Gas::from_tgas(30);
pub const FAUCET_AMOUNT: u128 = 500_000_000; // 500 tUSD (6 decimals)
pub const FAUCET_PERIOD_NS: u64 = 24 * 3600 * 1_000_000_000;

#[near(serializers = [json])]
pub struct FungibleTokenMetadata {
    pub spec: String,
    pub name: String,
    pub symbol: String,
    pub icon: Option<String>,
    pub reference: Option<String>,
    pub reference_hash: Option<String>,
    pub decimals: u8,
}

#[near(serializers = [json])]
pub struct StorageBalance {
    pub total: U128,
    pub available: U128,
}

#[near(serializers = [json])]
pub struct StorageBalanceBounds {
    pub min: U128,
    pub max: Option<U128>,
}

fn emit(event: &str, data: near_sdk::serde_json::Value) {
    let ev = json!({"standard": "nep141", "version": "1.0.0", "event": event, "data": [data]});
    env::log_str(&format!("EVENT_JSON:{ev}"));
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Token {
    owner: AccountId,
    balances: LookupMap<AccountId, u128>,
    total_supply: u128,
    faucet_last: LookupMap<AccountId, u64>,
}

#[near]
impl Token {
    #[init]
    pub fn new(owner: AccountId) -> Self {
        Self { owner, balances: LookupMap::new(b"b"), total_supply: 0, faucet_last: LookupMap::new(b"f") }
    }

    fn balance(&self, a: &AccountId) -> u128 {
        self.balances.get(a).copied().unwrap_or(0)
    }

    fn deposit(&mut self, a: &AccountId, amount: u128) {
        let b = self.balance(a);
        self.balances.insert(a.clone(), b.checked_add(amount).unwrap_or_else(|| env::panic_str("balance overflow")));
    }

    fn withdraw(&mut self, a: &AccountId, amount: u128) {
        let b = self.balance(a);
        require!(b >= amount, "insufficient tUSD balance");
        self.balances.insert(a.clone(), b - amount);
    }

    fn internal_transfer(&mut self, from: &AccountId, to: &AccountId, amount: u128, memo: Option<String>) {
        require!(from != to, "sender and receiver must differ");
        require!(amount > 0, "amount must be positive");
        self.withdraw(from, amount);
        self.deposit(to, amount);
        emit("ft_transfer", json!({"old_owner_id": from, "new_owner_id": to, "amount": U128(amount), "memo": memo}));
    }

    // ------------------------------------------------------------------ mint

    pub fn mint(&mut self, account_id: AccountId, amount: U128) {
        require!(env::predecessor_account_id() == self.owner, "only the owner mints");
        self.deposit(&account_id, amount.0);
        self.total_supply += amount.0;
        emit("ft_mint", json!({"owner_id": account_id, "amount": amount, "memo": "testnet token, no value"}));
    }

    /// 500 tUSD once per 24 h per account (testnet only; the token has no value).
    pub fn faucet(&mut self) -> U128 {
        let who = env::predecessor_account_id();
        let now = env::block_timestamp();
        if let Some(&last) = self.faucet_last.get(&who) {
            require!(now >= last + FAUCET_PERIOD_NS, "faucet: once per 24 hours");
        }
        self.faucet_last.insert(who.clone(), now);
        self.deposit(&who, FAUCET_AMOUNT);
        self.total_supply += FAUCET_AMOUNT;
        emit("ft_mint", json!({"owner_id": who, "amount": U128(FAUCET_AMOUNT), "memo": "faucet (testnet, no value)"}));
        U128(FAUCET_AMOUNT)
    }

    // ---------------------------------------------------------------- NEP-141

    #[payable]
    pub fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        near_sdk::assert_one_yocto();
        let sender = env::predecessor_account_id();
        self.internal_transfer(&sender, &receiver_id, amount.0, memo);
    }

    #[payable]
    pub fn ft_transfer_call(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>, msg: String) -> Promise {
        near_sdk::assert_one_yocto();
        require!(env::prepaid_gas() > GAS_FOR_FT_TRANSFER_CALL, "more gas is required");
        let sender = env::predecessor_account_id();
        self.internal_transfer(&sender, &receiver_id, amount.0, memo);
        let receiver_gas = env::prepaid_gas()
            .saturating_sub(env::used_gas())
            .saturating_sub(GAS_FOR_FT_TRANSFER_CALL);
        Promise::new(receiver_id.clone())
            .function_call(
                "ft_on_transfer".into(),
                json!({"sender_id": sender, "amount": amount, "msg": msg}).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                receiver_gas,
            )
            .then(Self::ext(env::current_account_id()).with_static_gas(GAS_FOR_RESOLVE).ft_resolve_transfer(sender, receiver_id, amount))
    }

    /// Refunds the part of `amount` the receiver did not use (standard NEP-141 resolution).
    #[private]
    pub fn ft_resolve_transfer(&mut self, sender_id: AccountId, receiver_id: AccountId, amount: U128) -> U128 {
        let unused = match env::promise_result(0) {
            PromiseResult::Successful(v) => match near_sdk::serde_json::from_slice::<U128>(&v) {
                Ok(u) => u.0.min(amount.0),
                Err(_) => amount.0,
            },
            PromiseResult::Failed => amount.0,
        };
        if unused > 0 {
            let refund = unused.min(self.balance(&receiver_id));
            if refund > 0 {
                self.withdraw(&receiver_id, refund);
                self.deposit(&sender_id, refund);
                emit(
                    "ft_transfer",
                    json!({"old_owner_id": receiver_id, "new_owner_id": sender_id, "amount": U128(refund), "memo": "refund"}),
                );
                log!("refund {} from {} to {}", refund, receiver_id, sender_id);
            }
            return U128(amount.0 - refund);
        }
        amount
    }

    pub fn ft_total_supply(&self) -> U128 {
        U128(self.total_supply)
    }

    pub fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        U128(self.balance(&account_id))
    }

    pub fn ft_metadata(&self) -> FungibleTokenMetadata {
        FungibleTokenMetadata {
            spec: "ft-1.0.0".into(),
            name: "Testnet USD (no value)".into(),
            symbol: "tUSD".into(),
            icon: None,
            reference: None,
            reference_hash: None,
            decimals: 6,
        }
    }

    // ------------------------------------------- NEP-145 (compatibility only)

    /// Accounts are registered automatically; any attached deposit is refunded.
    #[payable]
    pub fn storage_deposit(&mut self, account_id: Option<AccountId>, registration_only: Option<bool>) -> StorageBalance {
        let _ = (account_id, registration_only);
        let d = env::attached_deposit();
        if !d.is_zero() {
            Promise::new(env::predecessor_account_id()).transfer(d);
        }
        StorageBalance { total: U128(0), available: U128(0) }
    }

    pub fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        let _ = account_id;
        Some(StorageBalance { total: U128(0), available: U128(0) })
    }

    pub fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        StorageBalanceBounds { min: U128(0), max: Some(U128(0)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::{test_utils::VMContextBuilder, testing_env, PromiseResult};

    fn acc(s: &str) -> AccountId {
        s.parse().unwrap()
    }

    fn ctx(pred: &str, deposit: u128, ts: u64) {
        let mut b = VMContextBuilder::new();
        b.current_account_id(acc("tusd.testnet"))
            .predecessor_account_id(acc(pred))
            .attached_deposit(NearToken::from_yoctonear(deposit))
            .block_timestamp(ts)
            .prepaid_gas(Gas::from_tgas(300));
        testing_env!(b.build());
    }

    #[test]
    fn mint_transfer_faucet_and_metadata() {
        ctx("owner.testnet", 0, 1);
        let mut t = Token::new(acc("owner.testnet"));
        t.mint(acc("alice.testnet"), U128(1_000));
        ctx("alice.testnet", 1, 1);
        t.ft_transfer(acc("bob.testnet"), U128(400), None);
        assert_eq!(t.ft_balance_of(acc("alice.testnet")).0, 600);
        assert_eq!(t.ft_balance_of(acc("bob.testnet")).0, 400);
        ctx("carol.testnet", 0, 10);
        assert_eq!(t.faucet().0, FAUCET_AMOUNT);
        ctx("carol.testnet", 0, 20);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.faucet())).is_err());
        ctx("carol.testnet", 0, 20 + FAUCET_PERIOD_NS);
        t.faucet();
        assert_eq!(t.ft_total_supply().0, 1_000 + 2 * FAUCET_AMOUNT);
        assert_eq!(t.ft_metadata().symbol, "tUSD");
    }

    #[test]
    fn only_owner_mints_and_one_yocto_is_required() {
        ctx("owner.testnet", 0, 1);
        let mut t = Token::new(acc("owner.testnet"));
        ctx("mallory.testnet", 0, 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.mint(acc("mallory.testnet"), U128(5)))).is_err());
        ctx("owner.testnet", 0, 1);
        t.mint(acc("alice.testnet"), U128(10));
        ctx("alice.testnet", 0, 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.ft_transfer(acc("bob.testnet"), U128(1), None))).is_err());
    }

    #[test]
    fn resolve_refunds_unused_amount() {
        ctx("owner.testnet", 0, 1);
        let mut t = Token::new(acc("owner.testnet"));
        t.mint(acc("alice.testnet"), U128(100));
        ctx("alice.testnet", 1, 1);
        let _ = t.ft_transfer_call(acc("bounty.testnet"), U128(100), None, "{}".into());
        assert_eq!(t.ft_balance_of(acc("bounty.testnet")).0, 100);
        // receiver says 30 unused
        let mut b = VMContextBuilder::new();
        b.current_account_id(acc("tusd.testnet")).predecessor_account_id(acc("tusd.testnet")).prepaid_gas(Gas::from_tgas(300));
        testing_env!(
            b.build(),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            Default::default(),
            vec![PromiseResult::Successful(near_sdk::serde_json::to_vec(&U128(30)).unwrap())],
        );
        let used = t.ft_resolve_transfer(acc("alice.testnet"), acc("bounty.testnet"), U128(100));
        assert_eq!(used.0, 70);
        assert_eq!(t.ft_balance_of(acc("alice.testnet")).0, 30);
        assert_eq!(t.ft_balance_of(acc("bounty.testnet")).0, 70);
    }
}

#[cfg(all(test, windows))]
#[no_mangle]
pub extern "C" fn promise_yield_create(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> u64 {
    unreachable!()
}
#[cfg(all(test, windows))]
#[no_mangle]
pub extern "C" fn promise_yield_resume(_: u64, _: u64, _: u64, _: u64) -> u32 {
    unreachable!()
}

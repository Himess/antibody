//! Verifier v2's NEAR fixture: a near-sdk-rs contract that keeps one status message per account, counts
//! the updates, and records deposits. It exercises what near-sdk-rs contracts do on every call: JSON
//! arguments and results, borsh state, storage reads and writes, logs, the predecessor and the attached
//! deposit, a panic, and a cross-contract transfer.
//!
//! Build (toolchain 1.85.0, from this directory):
//!   cargo build --release --target wasm32-unknown-unknown
//! The wasm and its sha256 are committed next to this crate (status_message.wasm, status_message.wasm.sha256).

use near_sdk::{env, log, near, require, store::LookupMap, AccountId, NearToken, Promise};

#[near(contract_state)]
pub struct StatusMessage {
    records: LookupMap<AccountId, String>,
    updates: u64,
    deposits: u128,
}

impl Default for StatusMessage {
    fn default() -> Self {
        env::panic_str("the contract is not initialized")
    }
}

#[near]
impl StatusMessage {
    #[init]
    pub fn new() -> Self {
        Self { records: LookupMap::new(b"r"), updates: 0, deposits: 0 }
    }

    pub fn set_status(&mut self, message: String) {
        require!(message.len() <= 256, "the message is too long");
        let account_id = env::predecessor_account_id();
        log!("{} set the status to {}", account_id, message);
        self.records.insert(account_id, message);
        self.updates += 1;
    }

    pub fn get_status(&self, account_id: AccountId) -> Option<String> {
        self.records.get(&account_id).cloned()
    }

    pub fn updates(&self) -> u64 {
        self.updates
    }

    /// Records the attached deposit.
    #[payable]
    pub fn donate(&mut self) -> String {
        let amount = env::attached_deposit().as_yoctonear();
        require!(amount > 0, "attach a deposit");
        self.deposits += amount;
        self.deposits.to_string()
    }

    pub fn deposits(&self) -> String {
        self.deposits.to_string()
    }

    /// Sends `amount` yoctoNEAR of the recorded deposits to the caller (and forgets them).
    pub fn refund(&mut self, amount: String) -> Promise {
        let amount: u128 = amount.parse().unwrap_or_else(|_| env::panic_str("amount is not a number"));
        require!(amount <= self.deposits, "more than was deposited");
        self.deposits -= amount;
        Promise::new(env::predecessor_account_id()).transfer(NearToken::from_yoctonear(amount))
    }
}

//! MOCK of NEAR's Ethereum light client (`block_hash_safe`), for the offline sandbox demo only (never
//! deployed to testnet).
//!
//! On testnet near-antibody asks the real client (`client-eth2.sepolia.testnet`), whose hashes come
//! from Ethereum's sync committee. The sandbox's chain is a local anvil, so its owner records the
//! anchor's hash here. Same call shape: borsh `u64` block number in, borsh `Option<[u8; 32]>` out.

use near_sdk::{
    borsh::{self, BorshDeserialize},
    env, near, require,
    store::LookupMap,
    AccountId, PanicOnDefault,
};

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockLightClient {
    owner: AccountId,
    hashes: LookupMap<u64, [u8; 32]>,
}

#[near]
impl MockLightClient {
    #[init]
    pub fn new(owner: AccountId) -> Self {
        Self { owner, hashes: LookupMap::new(b"h") }
    }

    /// Owner only: the hash of a block of the local chain (hex, with or without 0x).
    pub fn set_block_hash(&mut self, number: u64, hash: String) {
        require!(env::predecessor_account_id() == self.owner, "only the owner records hashes");
        let h = hash.strip_prefix("0x").unwrap_or(&hash);
        require!(h.len() == 64, "a block hash is 32 bytes of hex");
        let mut b = [0u8; 32];
        for (i, x) in b.iter_mut().enumerate() {
            *x = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap_or_else(|_| env::panic_str("bad hex"));
        }
        self.hashes.insert(number, b);
    }

    /// As the real client: borsh `u64` in, borsh `Option<[u8; 32]>` out.
    pub fn block_hash_safe(&self) {
        let input = env::input().unwrap_or_default();
        let n = u64::try_from_slice(&input).unwrap_or_else(|_| env::panic_str("borsh u64 expected"));
        let out: Option<[u8; 32]> = self.hashes.get(&n).copied();
        env::value_return(&borsh::to_vec(&out).unwrap());
    }
}

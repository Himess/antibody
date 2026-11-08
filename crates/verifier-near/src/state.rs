//! The world a run executes in: accounts, their keys, code and data, with storage usage counted as
//! nearcore counts it (the account record, each data record's key and value plus 40 bytes, the code, and
//! each access key's borsh encoding plus 40 bytes).

use std::{collections::BTreeMap, sync::Arc};

use crate::config::{STORAGE_NUM_BYTES_ACCOUNT, STORAGE_NUM_EXTRA_BYTES_RECORD};

/// An access key's permission. Keys are recorded state: v2 checks no signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyPermission {
    FullAccess,
    FunctionCall { allowance: Option<u128>, receiver: String, methods: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessKey {
    pub nonce: u64,
    pub permission: KeyPermission,
}

impl AccessKey {
    /// The length of the key's borsh encoding (what nearcore counts for storage).
    pub fn borsh_len(&self) -> u64 {
        let permission = match &self.permission {
            KeyPermission::FullAccess => 1,
            KeyPermission::FunctionCall { allowance, receiver, methods } => {
                1 + if allowance.is_some() { 17 } else { 1 }
                    + 4
                    + receiver.len() as u64
                    + 4
                    + methods.iter().map(|m| 4 + m.len() as u64).sum::<u64>()
            }
        };
        8 + permission
    }
}

/// The storage an access key counts: its public key's encoding, the access key's, and 40 bytes.
pub fn key_storage(public_key: &[u8], key: &AccessKey) -> u64 {
    STORAGE_NUM_EXTRA_BYTES_RECORD + public_key.len() as u64 + key.borsh_len()
}

/// The storage a data record counts.
pub fn record_storage(key: &[u8], value: &[u8]) -> u64 {
    key.len() as u64 + value.len() as u64 + STORAGE_NUM_EXTRA_BYTES_RECORD
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub amount: u128,
    pub locked: u128,
    pub storage_usage: u64,
    /// sha256 of the deployed code.
    pub code: Option<[u8; 32]>,
    /// Borsh-encoded public key → access key.
    pub keys: BTreeMap<Vec<u8>, AccessKey>,
    pub data: Arc<BTreeMap<Vec<u8>, Vec<u8>>>,
}

impl Account {
    pub fn new(amount: u128, locked: u128) -> Self {
        Account { amount, locked, storage_usage: STORAGE_NUM_BYTES_ACCOUNT, code: None, keys: BTreeMap::new(), data: Arc::default() }
    }
}

/// The NEAR-implicit account a 64-hex id names: its full-access key is the ed25519 key the id encodes.
pub fn implicit_account(id: &str, amount: u128) -> Account {
    let mut a = Account::new(amount, 0);
    if let Ok(pk) = hex::decode(id) {
        let mut enc = vec![0u8];
        enc.extend_from_slice(&pk);
        let key = AccessKey { nonce: 0, permission: KeyPermission::FullAccess };
        a.storage_usage += key_storage(&enc, &key);
        a.keys.insert(enc, key);
    }
    a
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct World {
    pub accounts: BTreeMap<String, Account>,
    /// Code by sha256.
    pub codes: BTreeMap<[u8; 32], Arc<Vec<u8>>>,
}

impl World {
    pub fn code_of(&self, account: &str) -> Option<(&[u8; 32], &Arc<Vec<u8>>)> {
        let h = self.accounts.get(account)?.code.as_ref()?;
        self.codes.get(h).map(|c| (h, c))
    }
}

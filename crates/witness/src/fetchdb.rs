//! Host-side "trace" database: answers revm reads lazily (from an optional execution-witness seed,
//! else from RPC at the parent block) and records every key that execution touched. The verifier's
//! own `execute_replay` runs on top of it, so the recorded access set is exactly what the strict
//! in-TEE run will need (modulo divergence, which the fixpoint loop catches).

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use antibody_verifier::db::{MissingTracker, MissingWitness};
use reth_trie::{TrieAccount, EMPTY_ROOT_HASH};
use revm::{
    database_interface::DBErrorMarker,
    primitives::KECCAK_EMPTY,
    state::{AccountInfo, Bytecode},
    DatabaseRef,
};
use rsp_mpt::EthereumState;
use serde_json::{json, Value};

use crate::rpc::{hex_u64, Provider};

#[derive(Debug, Clone)]
pub struct FetchError(pub String);
impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fetch: {}", self.0)
    }
}
impl std::error::Error for FetchError {}
impl DBErrorMarker for FetchError {}

/// Optional pre-built source of truth for most reads (e.g. from `debug_executionWitness`).
#[derive(Debug, Default)]
pub struct Seed {
    pub state: Option<EthereumState>,
    pub codes: BTreeMap<B256, Bytecode>,
    pub block_hashes: BTreeMap<u64, B256>,
}

#[derive(Debug, Default, Clone)]
pub struct Recorded {
    pub accounts: BTreeSet<Address>,
    pub slots: BTreeMap<Address, BTreeSet<U256>>,
    pub codes: BTreeMap<B256, Bytecode>,
    pub block_numbers: BTreeSet<u64>,
    pub seed_hits: u64,
    pub rpc_reads: u64,
}

#[derive(Debug)]
pub struct FetchingDb {
    pub providers: Vec<Arc<Provider>>,
    /// Block whose post-state is read (N-1 for a replay of block N).
    pub block: u64,
    pub seed: Seed,
    accounts: RefCell<BTreeMap<Address, Option<AccountInfo>>>,
    storage: RefCell<BTreeMap<(Address, U256), U256>>,
    pub rec: RefCell<Recorded>,
    rr: Cell<usize>,
}

impl FetchingDb {
    pub fn new(providers: Vec<Arc<Provider>>, block: u64, seed: Seed) -> Self {
        Self {
            providers,
            block,
            seed,
            accounts: Default::default(),
            storage: Default::default(),
            rec: Default::default(),
            rr: Cell::new(0),
        }
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, FetchError> {
        // Round-robin over providers; on failure try the next ones.
        let n = self.providers.len();
        let start = self.rr.get();
        self.rr.set((start + 1) % n);
        let mut errs = Vec::new();
        for k in 0..n {
            let p = &self.providers[(start + k) % n];
            match p.call(method, params.clone()) {
                Ok(v) => return Ok(v),
                Err(e) => errs.push(e.to_string()),
            }
        }
        Err(FetchError(errs.join(" | ")))
    }

    fn seed_account(&self, address: Address) -> Option<Option<TrieAccount>> {
        let st = self.seed.state.as_ref()?;
        st.state_trie.get_rlp::<TrieAccount>(keccak256(address).as_slice()).ok()
    }

    fn seed_slot(&self, address: Address, index: U256) -> Option<U256> {
        let st = self.seed.state.as_ref()?;
        let ha = keccak256(address);
        match st.storage_tries.get(&ha) {
            Some(t) => t
                .get_rlp::<U256>(keccak256(index.to_be_bytes::<32>()).as_slice())
                .ok()
                .map(|v| v.unwrap_or_default()),
            None => match self.seed_account(address)? {
                None => Some(U256::ZERO),
                Some(a) if a.storage_root == EMPTY_ROOT_HASH => Some(U256::ZERO),
                Some(_) => None,
            },
        }
    }

    fn code_for(&self, address: Address, code_hash: B256) -> Result<(), FetchError> {
        if code_hash == KECCAK_EMPTY || self.rec.borrow().codes.contains_key(&code_hash) {
            return Ok(());
        }
        let code = match self.seed.codes.get(&code_hash) {
            Some(c) => c.clone(),
            None => {
                let v = self.call("eth_getCode", json!([address, format!("{:#x}", self.block)]))?;
                let bytes: Bytes = serde_json::from_value(v).map_err(|e| FetchError(e.to_string()))?;
                Bytecode::new_raw(bytes)
            }
        };
        if code.hash_slow() != code_hash {
            return Err(FetchError(format!("code hash mismatch for {address}")));
        }
        self.rec.borrow_mut().codes.insert(code_hash, code);
        Ok(())
    }
}

impl MissingTracker for FetchingDb {
    fn first_missing(&self) -> Option<MissingWitness> {
        None
    }
}

impl DatabaseRef for FetchingDb {
    type Error = FetchError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.rec.borrow_mut().accounts.insert(address);
        if let Some(a) = self.accounts.borrow().get(&address) {
            return Ok(a.clone());
        }
        let info = if let Some(acc) = self.seed_account(address) {
            self.rec.borrow_mut().seed_hits += 1;
            acc.map(|a| AccountInfo { balance: a.balance, nonce: a.nonce, code_hash: a.code_hash, account_id: None, code: None })
        } else {
            self.rec.borrow_mut().rpc_reads += 1;
            let v = self.call("eth_getProof", json!([address, [], format!("{:#x}", self.block)]))?;
            let balance: U256 = serde_json::from_value(v["balance"].clone()).map_err(|e| FetchError(e.to_string()))?;
            let nonce = hex_u64(&v["nonce"]).map_err(|e| FetchError(e.to_string()))?;
            let mut code_hash: B256 = serde_json::from_value(v["codeHash"].clone()).map_err(|e| FetchError(e.to_string()))?;
            let storage_hash: B256 =
                serde_json::from_value(v["storageHash"].clone()).map_err(|e| FetchError(e.to_string()))?;
            if code_hash == B256::ZERO {
                code_hash = KECCAK_EMPTY;
            }
            let empty = nonce == 0
                && balance.is_zero()
                && code_hash == KECCAK_EMPTY
                && (storage_hash == B256::ZERO || storage_hash == EMPTY_ROOT_HASH);
            if empty {
                None
            } else {
                Some(AccountInfo { balance, nonce, code_hash, account_id: None, code: None })
            }
        };
        if let Some(i) = &info {
            self.code_for(address, i.code_hash)?;
        }
        self.accounts.borrow_mut().insert(address, info.clone());
        Ok(info)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if let Some(c) = self.rec.borrow().codes.get(&code_hash) {
            return Ok(c.clone());
        }
        if let Some(c) = self.seed.codes.get(&code_hash) {
            self.rec.borrow_mut().codes.insert(code_hash, c.clone());
            return Ok(c.clone());
        }
        Err(FetchError(format!("code {code_hash} requested before its account")))
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.rec.borrow_mut().slots.entry(address).or_default().insert(index);
        if let Some(v) = self.storage.borrow().get(&(address, index)) {
            return Ok(*v);
        }
        let v = if let Some(v) = self.seed_slot(address, index) {
            self.rec.borrow_mut().seed_hits += 1;
            v
        } else {
            self.rec.borrow_mut().rpc_reads += 1;
            let r = self.call(
                "eth_getStorageAt",
                json!([address, format!("{:#066x}", index), format!("{:#x}", self.block)]),
            )?;
            let b: B256 = serde_json::from_value(r).map_err(|e| FetchError(e.to_string()))?;
            U256::from_be_bytes(b.0)
        };
        self.storage.borrow_mut().insert((address, index), v);
        Ok(v)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.rec.borrow_mut().block_numbers.insert(number);
        if let Some(h) = self.seed.block_hashes.get(&number) {
            return Ok(*h);
        }
        let v = self.call("eth_getBlockByNumber", json!([format!("{number:#x}"), false]))?;
        serde_json::from_value(v["hash"].clone()).map_err(|e| FetchError(e.to_string()))
    }
}

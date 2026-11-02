//! A strict, non-panicking revm database over rsp's verified partial tries.
//!
//! rsp's `TrieDB` aborts on missing witness data with `unwrap`/`expect`. In wasm that is a trap,
//! which the caller cannot distinguish from a crash. `StrictDb` reads the same verified tries but
//! returns a structured error instead, so the verifier can report "incomplete witness" (infra
//! problem: refund) separately from "tampered witness" (proof failure) and "PoC reverted".

use std::cell::RefCell;
use std::collections::BTreeSet;

use alloy_primitives::{keccak256, map::HashMap, Address, B256, U256};
use reth_trie::{TrieAccount, EMPTY_ROOT_HASH};
use revm::{
    database_interface::DBErrorMarker,
    state::{AccountInfo, Bytecode},
    DatabaseRef,
};
use rsp_mpt::EthereumState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingWitness {
    Account(Address),
    StorageTrie(Address),
    Slot(Address, U256),
    Code(B256),
    BlockHash(u64),
}

impl std::fmt::Display for MissingWitness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Account(a) => write!(f, "missing account {a}"),
            Self::StorageTrie(a) => write!(f, "missing storage trie for {a}"),
            Self::Slot(a, s) => write!(f, "missing storage slot {a}[{s:#x}]"),
            Self::Code(h) => write!(f, "missing bytecode {h}"),
            Self::BlockHash(n) => write!(f, "missing block hash {n}"),
        }
    }
}

impl std::error::Error for MissingWitness {}
impl DBErrorMarker for MissingWitness {}

/// What execution actually read (to report witness utilisation and to drive the host fixpoint).
#[derive(Debug, Default, Clone)]
pub struct AccessLog {
    pub accounts: BTreeSet<Address>,
    pub slots: BTreeSet<(Address, U256)>,
    pub codes: BTreeSet<B256>,
    pub block_hashes: BTreeSet<u64>,
    pub missing: Vec<MissingWitness>,
}

/// Lets the executor classify a database error as "incomplete witness" (vs. an EVM failure).
pub trait MissingTracker {
    fn first_missing(&self) -> Option<MissingWitness>;
}

impl MissingTracker for StrictDb<'_> {
    fn first_missing(&self) -> Option<MissingWitness> {
        self.log.borrow().missing.first().cloned()
    }
}

#[derive(Debug)]
pub struct StrictDb<'a> {
    state: &'a EthereumState,
    block_hashes: HashMap<u64, B256>,
    bytecode_by_hash: HashMap<B256, &'a Bytecode>,
    pub log: RefCell<AccessLog>,
}

impl<'a> StrictDb<'a> {
    pub fn new(
        state: &'a EthereumState,
        block_hashes: HashMap<u64, B256>,
        bytecode_by_hash: HashMap<B256, &'a Bytecode>,
    ) -> Self {
        Self { state, block_hashes, bytecode_by_hash, log: RefCell::new(AccessLog::default()) }
    }

    fn fail<T>(&self, m: MissingWitness) -> Result<T, MissingWitness> {
        self.log.borrow_mut().missing.push(m.clone());
        Err(m)
    }

    fn trie_account(&self, address: Address) -> Result<Option<TrieAccount>, MissingWitness> {
        match self.state.state_trie.get_rlp::<TrieAccount>(keccak256(address).as_slice()) {
            Ok(a) => Ok(a),
            Err(_) => self.fail(MissingWitness::Account(address)),
        }
    }
}

impl DatabaseRef for StrictDb<'_> {
    type Error = MissingWitness;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.log.borrow_mut().accounts.insert(address);
        Ok(self.trie_account(address)?.map(|a| AccountInfo {
            balance: a.balance,
            nonce: a.nonce,
            code_hash: a.code_hash,
            account_id: None,
            code: None,
        }))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.log.borrow_mut().codes.insert(code_hash);
        match self.bytecode_by_hash.get(&code_hash) {
            Some(code) => Ok((*code).clone()),
            None => self.fail(MissingWitness::Code(code_hash)),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.log.borrow_mut().slots.insert((address, index));
        let hashed_address = keccak256(address);
        match self.state.storage_tries.get(&hashed_address) {
            Some(trie) => match trie.get_rlp::<U256>(keccak256(index.to_be_bytes::<32>()).as_slice()) {
                Ok(v) => Ok(v.unwrap_or_default()),
                Err(_) => self.fail(MissingWitness::Slot(address, index)),
            },
            // No storage trie supplied: only acceptable when the account provably has no storage.
            None => match self.trie_account(address)? {
                None => Ok(U256::ZERO),
                Some(a) if a.storage_root == EMPTY_ROOT_HASH => Ok(U256::ZERO),
                Some(_) => self.fail(MissingWitness::StorageTrie(address)),
            },
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.log.borrow_mut().block_hashes.insert(number);
        match self.block_hashes.get(&number) {
            Some(h) => Ok(*h),
            None => self.fail(MissingWitness::BlockHash(number)),
        }
    }
}

/// Rounds (docs/rounds.md): serves a round manifest's override code at the overridden target
/// addresses. Balance, nonce and storage still come from the proven anchor state underneath, so a
/// fix committed as a code override runs on exactly the state the original PoC ran on.
#[derive(Debug)]
pub struct OverrideDb<'a, D> {
    inner: &'a D,
    overrides: HashMap<Address, (B256, Bytecode)>,
}

impl<'a, D> OverrideDb<'a, D> {
    pub fn new(inner: &'a D, overrides: impl IntoIterator<Item = (Address, Bytecode)>) -> Self {
        Self { inner, overrides: overrides.into_iter().map(|(a, c)| (a, (c.hash_slow(), c))).collect() }
    }
}

impl<D: DatabaseRef> DatabaseRef for OverrideDb<'_, D> {
    type Error = D::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let info = self.inner.basic_ref(address)?;
        Ok(match self.overrides.get(&address) {
            None => info,
            Some((hash, code)) => {
                let (balance, nonce) = info.map(|i| (i.balance, i.nonce)).unwrap_or_default();
                Some(AccountInfo { balance, nonce, code_hash: *hash, account_id: None, code: Some(code.clone()) })
            }
        })
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self.overrides.values().find(|(h, _)| *h == code_hash) {
            Some((_, code)) => Ok(code.clone()),
            None => self.inner.code_by_hash_ref(code_hash),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.inner.storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.inner.block_hash_ref(number)
    }
}

impl<D: MissingTracker> MissingTracker for OverrideDb<'_, D> {
    fn first_missing(&self) -> Option<MissingWitness> {
        self.inner.first_missing()
    }
}

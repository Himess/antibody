//! Antibody Guardian: a NEAR contract that controls a Chain Signatures-derived EVM key and can only
//! ever sign one kind of transaction, `pause()` on a registered target, on every registered chain.
//!
//! - The contract builds each EIP-1559 transaction itself from the registered target (chain id,
//!   `to`, calldata, gas limit, fee caps) and its own per-chain nonce; callers choose nothing.
//! - For each chain it asks the MPC network (`sign`, domain 0) to sign keccak256 of the unsigned
//!   transaction, then re-verifies the signature with `env::ecrecover` against the pinned guardian
//!   address before publishing the signed raw transaction.
//! - `pause_all` is callable by the owner and by authorized callers (the near-antibody contract,
//!   after a verified Critical). Broadcasting is permissionless: the signed transactions are public
//!   and any relayer can submit them; `pause()` on the target is idempotent.

pub mod eip1559;

use eip1559::{hex_to_bytes, to_hex, CallTx};
use near_sdk::{
    env, json_types::U128, log, near, require, serde_json::json, store::LookupMap, AccountId, Gas, NearToken, PanicOnDefault,
    Promise, PromiseError,
};

pub const SIGN_GAS: Gas = Gas::from_tgas(40);
pub const CALLBACK_GAS: Gas = Gas::from_tgas(20);
/// `pause()` selector.
pub const PAUSE_CALLDATA: &str = "0x8456cb59";

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug)]
pub struct ChainTarget {
    pub chain_id: u64,
    pub name: String,
    /// 0x-prefixed 20-byte address of the protected contract (e.g. AcmeGuard).
    pub to: String,
    /// 0x-prefixed calldata; always `pause()` unless the owner registers another pause entrypoint.
    pub calldata: String,
    pub gas_limit: u64,
    pub max_fee_per_gas: U128,
    pub max_priority_fee_per_gas: U128,
    pub next_nonce: u64,
    /// Explorer base URL for tx links (informational).
    pub explorer: String,
}

#[near(serializers = [json])]
pub struct BigR {
    pub affine_point: String,
}

#[near(serializers = [json])]
pub struct Scalar {
    pub scalar: String,
}

/// Secp256k1 response of the MPC `sign` call.
#[near(serializers = [json])]
pub struct SignatureResponse {
    pub big_r: BigR,
    pub s: Scalar,
    pub recovery_id: u8,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug)]
pub struct SignedPause {
    pub request_id: u64,
    pub chain_id: u64,
    pub nonce: u64,
    pub max_fee_per_gas: U128,
    pub max_priority_fee_per_gas: U128,
    pub payload_hash: String,
    pub signed_tx: String,
    /// keccak256 of the signed raw transaction = the EVM transaction hash.
    pub tx_hash: String,
    pub reason: String,
    pub requested_at_block: u64,
    pub signed_at_block: u64,
    pub recovered_address: String,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Guardian {
    owner: AccountId,
    mpc: AccountId,
    path: String,
    /// Expected guardian EVM address (derived from the MPC root key, this account and `path`).
    guardian_address: String,
    authorized: Vec<AccountId>,
    chains: Vec<ChainTarget>,
    next_request: u64,
    failures: u64,
    /// Every signed pause, by request id.
    signed: LookupMap<u64, SignedPause>,
    /// Latest signed pause per chain id.
    latest: LookupMap<u64, u64>,
}

fn parse_addr(s: &str) -> [u8; 20] {
    let b = hex_to_bytes(s).unwrap_or_else(|| env::panic_str("bad address hex"));
    require!(b.len() == 20, "address must be 20 bytes");
    let mut a = [0u8; 20];
    a.copy_from_slice(&b);
    a
}

fn tx_for(t: &ChainTarget, nonce: u64) -> CallTx {
    CallTx {
        chain_id: t.chain_id,
        nonce,
        max_priority_fee_per_gas: t.max_priority_fee_per_gas.0,
        max_fee_per_gas: t.max_fee_per_gas.0,
        gas_limit: t.gas_limit,
        to: parse_addr(&t.to),
        data: hex_to_bytes(&t.calldata).unwrap_or_else(|| env::panic_str("bad calldata")),
    }
}

#[near]
impl Guardian {
    #[init]
    pub fn new(owner: AccountId, mpc: AccountId, path: String, guardian_address: String) -> Self {
        parse_addr(&guardian_address);
        Self {
            owner,
            mpc,
            path,
            guardian_address: guardian_address.to_lowercase(),
            authorized: Vec::new(),
            chains: Vec::new(),
            next_request: 0,
            failures: 0,
            signed: LookupMap::new(b"s"),
            latest: LookupMap::new(b"l"),
        }
    }

    fn assert_owner(&self) {
        require!(env::predecessor_account_id() == self.owner, "only the owner");
    }

    /// Registers or updates the pause target on one chain.
    pub fn set_chain(&mut self, target: ChainTarget) {
        self.assert_owner();
        parse_addr(&target.to);
        hex_to_bytes(&target.calldata).unwrap_or_else(|| env::panic_str("bad calldata"));
        require!(target.max_priority_fee_per_gas.0 <= target.max_fee_per_gas.0, "priority fee above max fee");
        match self.chains.iter_mut().find(|c| c.chain_id == target.chain_id) {
            Some(c) => *c = target,
            None => self.chains.push(target),
        }
    }

    pub fn set_authorized(&mut self, account_id: AccountId, enabled: bool) {
        self.assert_owner();
        self.authorized.retain(|a| a != &account_id);
        if enabled {
            self.authorized.push(account_id);
        }
    }

    /// Asks the MPC network to sign `pause()` for every registered chain (parallel requests).
    pub fn pause_all(&mut self, program: String, commit_id: u64) -> Promise {
        let caller = env::predecessor_account_id();
        require!(caller == self.owner || self.authorized.contains(&caller), "not authorized to request a pause");
        require!(!self.chains.is_empty(), "no chains registered");
        let reason = format!("{program}#{commit_id}");
        let mut joint: Option<Promise> = None;
        for i in 0..self.chains.len() {
            let p = self.request_sign(i, reason.clone());
            joint = Some(match joint {
                None => p,
                Some(j) => j.and(p),
            });
        }
        joint.unwrap()
    }

    /// Owner-only re-request for one chain (e.g. a fee bump at the same nonce after a stuck tx).
    pub fn resign(&mut self, chain_id: u64, nonce: u64, max_fee_per_gas: U128, max_priority_fee_per_gas: U128) -> Promise {
        self.assert_owner();
        let i = self.chains.iter().position(|c| c.chain_id == chain_id).unwrap_or_else(|| env::panic_str("unknown chain"));
        let c = &mut self.chains[i];
        require!(max_priority_fee_per_gas.0 <= max_fee_per_gas.0, "priority fee above max fee");
        c.max_fee_per_gas = max_fee_per_gas;
        c.max_priority_fee_per_gas = max_priority_fee_per_gas;
        c.next_nonce = nonce;
        self.request_sign(i, "fee bump".into())
    }

    pub fn set_nonce(&mut self, chain_id: u64, nonce: u64) {
        self.assert_owner();
        let c = self.chains.iter_mut().find(|c| c.chain_id == chain_id).unwrap_or_else(|| env::panic_str("unknown chain"));
        c.next_nonce = nonce;
    }

    fn request_sign(&mut self, i: usize, reason: String) -> Promise {
        let t = self.chains[i].clone();
        let nonce = t.next_nonce;
        self.chains[i].next_nonce += 1;
        let tx = tx_for(&t, nonce);
        let hash = env::keccak256(&tx.unsigned());
        let request_id = self.next_request;
        self.next_request += 1;
        let args = json!({"request": {"path": self.path, "payload_v2": {"Ecdsa": to_hex(&hash)}, "domain_id": 0}});
        log!("pause_sign_requested id={} chain={} nonce={} payload={}", request_id, t.chain_id, nonce, to_hex(&hash));
        Promise::new(self.mpc.clone())
            .function_call("sign".to_string(), args.to_string().into_bytes(), NearToken::from_yoctonear(1), SIGN_GAS)
            .then(Self::ext(env::current_account_id()).with_static_gas(CALLBACK_GAS).on_signed(
                request_id,
                t.chain_id,
                nonce,
                to_hex(&hash),
                reason,
                env::block_height(),
            ))
    }

    #[private]
    pub fn on_signed(
        &mut self,
        #[callback_result] res: Result<SignatureResponse, PromiseError>,
        request_id: u64,
        chain_id: u64,
        nonce: u64,
        payload_hash: String,
        reason: String,
        requested_at_block: u64,
    ) -> Option<SignedPause> {
        let sig = match res {
            Ok(s) => s,
            Err(_) => {
                self.failures += 1;
                log!("pause_sign_failed id={} chain={} (MPC timeout or error)", request_id, chain_id);
                return None;
            }
        };
        let big_r = hex_to_bytes(&sig.big_r.affine_point).unwrap_or_default();
        let s = hex_to_bytes(&sig.s.scalar).unwrap_or_default();
        if big_r.len() != 33 || s.len() != 32 || sig.recovery_id > 1 {
            self.failures += 1;
            log!("pause_sign_bad_encoding id={}", request_id);
            return None;
        }
        let mut r32 = [0u8; 32];
        r32.copy_from_slice(&big_r[1..33]);
        let mut s32 = [0u8; 32];
        s32.copy_from_slice(&s);
        let mut rs = [0u8; 64];
        rs[..32].copy_from_slice(&r32);
        rs[32..].copy_from_slice(&s32);
        let hash = hex_to_bytes(&payload_hash).unwrap();
        // Independent check: the signature must recover to the pinned guardian address.
        let recovered = match env::ecrecover(&hash, &rs, sig.recovery_id, true) {
            Some(pk) => format!("0x{}", to_hex(&env::keccak256(&pk)[12..])),
            None => String::new(),
        };
        if recovered != self.guardian_address {
            self.failures += 1;
            log!("pause_sign_wrong_key id={} recovered={} expected={}", request_id, recovered, self.guardian_address);
            return None;
        }
        let t = self.chains.iter().find(|c| c.chain_id == chain_id).cloned().unwrap();
        let signed = tx_for(&t, nonce).signed(sig.recovery_id, &r32, &s32);
        let out = SignedPause {
            request_id,
            chain_id,
            nonce,
            max_fee_per_gas: t.max_fee_per_gas,
            max_priority_fee_per_gas: t.max_priority_fee_per_gas,
            payload_hash,
            signed_tx: format!("0x{}", to_hex(&signed)),
            tx_hash: format!("0x{}", to_hex(&env::keccak256(&signed))),
            reason,
            requested_at_block,
            signed_at_block: env::block_height(),
            recovered_address: recovered,
        };
        let ev = json!({"standard": "antibody-guardian", "version": "1.0.0", "event": "pause_signed",
                        "data": [{"chain_id": chain_id, "nonce": nonce, "tx_hash": out.tx_hash, "signed_tx": out.signed_tx,
                                  "request_id": request_id, "reason": out.reason}]});
        env::log_str(&format!("EVENT_JSON:{ev}"));
        self.signed.insert(request_id, out.clone());
        self.latest.insert(chain_id, request_id);
        Some(out)
    }

    // ------------------------------------------------------------------ views

    /// The payload the contract would ask the MPC network to sign for `chain_id` at `nonce`.
    pub fn preview(&self, chain_id: u64, nonce: u64) -> String {
        let t = self.chains.iter().find(|c| c.chain_id == chain_id).unwrap_or_else(|| env::panic_str("unknown chain"));
        format!("0x{}", to_hex(&env::keccak256(&tx_for(t, nonce).unsigned())))
    }

    pub fn get_request(&self, request_id: u64) -> Option<SignedPause> {
        self.signed.get(&request_id).cloned()
    }

    pub fn get_latest(&self, chain_id: u64) -> Option<SignedPause> {
        self.latest.get(&chain_id).and_then(|id| self.signed.get(id)).cloned()
    }

    pub fn get_chains(&self) -> Vec<ChainTarget> {
        self.chains.clone()
    }

    pub fn get_config(&self) -> near_sdk::serde_json::Value {
        json!({"owner": self.owner, "mpc": self.mpc, "path": self.path, "guardian_address": self.guardian_address,
               "authorized": self.authorized, "chains": self.chains, "next_request": self.next_request, "failures": self.failures})
    }
}

#[cfg(test)]
mod tests;

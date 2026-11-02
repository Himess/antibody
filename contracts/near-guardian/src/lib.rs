//! Antibody Guardian (spike): a NEAR contract that controls a Chain Signatures-derived EVM key and
//! can only ever sign one kind of transaction: `pause()` on a registered target.
//!
//! - The contract builds the EIP-1559 transaction itself from the registered target; callers can
//!   only choose the nonce and fees within caps.
//! - It requests a secp256k1 signature from the MPC contract (`sign`, domain 0) over keccak256 of
//!   the unsigned transaction, then re-verifies the returned signature with `env::ecrecover`
//!   against the pinned guardian address before publishing the signed raw transaction.
//! - Broadcasting is permissionless: anyone can submit the stored raw transaction.
//!
//! In the full design `request_pause` is reachable only from the verifier callback after a
//! verified Critical; in this spike it is owner-only.

pub mod eip1559;

use eip1559::{hex_to_bytes, to_hex, CallTx};
use near_sdk::{
    env, json_types::U128, log, near, require, serde_json::json, store::LookupMap, AccountId, Gas, NearToken, Promise,
    PromiseError,
};

const SIGN_GAS: Gas = Gas::from_tgas(50);
const CALLBACK_GAS: Gas = Gas::from_tgas(20);

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug)]
pub struct PauseTarget {
    pub chain_id: u64,
    /// 0x-prefixed 20-byte address of the protected contract.
    pub to: String,
    /// 0x-prefixed calldata, e.g. "0x8456cb59" for pause().
    pub calldata: String,
    pub gas_limit: u64,
    pub max_fee_per_gas_cap: U128,
    pub max_priority_fee_per_gas_cap: U128,
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
    pub nonce: u64,
    pub max_fee_per_gas: U128,
    pub max_priority_fee_per_gas: U128,
    pub payload_hash: String,
    pub signed_tx: String,
    pub requested_at_block: u64,
    pub signed_at_block: u64,
    pub recovered_address: String,
}

#[near(contract_state)]
pub struct Guardian {
    owner: AccountId,
    mpc: AccountId,
    path: String,
    target: PauseTarget,
    /// Expected guardian EVM address (derived from the MPC root key, predecessor and path).
    guardian_address: String,
    next_request: u64,
    last: Option<SignedPause>,
    failures: u64,
    /// Every signed pause, by request id (fee-bump re-requests at the same nonce are allowed).
    history: LookupMap<u64, SignedPause>,
}

impl Default for Guardian {
    fn default() -> Self {
        env::panic_str("use new()")
    }
}

fn parse_addr(s: &str) -> [u8; 20] {
    let b = hex_to_bytes(s).unwrap_or_else(|| env::panic_str("bad address hex"));
    require!(b.len() == 20, "address must be 20 bytes");
    let mut a = [0u8; 20];
    a.copy_from_slice(&b);
    a
}

#[near]
impl Guardian {
    #[init]
    pub fn new(owner: AccountId, mpc: AccountId, path: String, target: PauseTarget, guardian_address: String) -> Self {
        parse_addr(&target.to);
        parse_addr(&guardian_address);
        Self {
            owner,
            mpc,
            path,
            target,
            guardian_address: guardian_address.to_lowercase(),
            next_request: 0,
            last: None,
            failures: 0,
            history: LookupMap::new(b"h"),
        }
    }

    fn tx(&self, nonce: u64, max_fee: u128, max_prio: u128) -> CallTx {
        CallTx {
            chain_id: self.target.chain_id,
            nonce,
            max_priority_fee_per_gas: max_prio,
            max_fee_per_gas: max_fee,
            gas_limit: self.target.gas_limit,
            to: parse_addr(&self.target.to),
            data: hex_to_bytes(&self.target.calldata).unwrap_or_else(|| env::panic_str("bad calldata")),
        }
    }

    /// Builds the registered pause transaction and asks the MPC network to sign it.
    #[payable]
    pub fn request_pause(&mut self, nonce: u64, max_fee_per_gas: U128, max_priority_fee_per_gas: U128) -> Promise {
        require!(env::predecessor_account_id() == self.owner, "only owner (spike); production: verifier callback");
        require!(max_fee_per_gas.0 <= self.target.max_fee_per_gas_cap.0, "maxFeePerGas above cap");
        require!(max_priority_fee_per_gas.0 <= self.target.max_priority_fee_per_gas_cap.0, "priority fee above cap");
        require!(max_priority_fee_per_gas.0 <= max_fee_per_gas.0, "priority > max fee");
        let tx = self.tx(nonce, max_fee_per_gas.0, max_priority_fee_per_gas.0);
        let hash = env::keccak256(&tx.unsigned());
        let request_id = self.next_request;
        self.next_request += 1;
        let args = json!({"request": {"path": self.path, "payload_v2": {"Ecdsa": to_hex(&hash)}, "domain_id": 0}});
        log!("pause_requested id={} nonce={} payload={}", request_id, nonce, to_hex(&hash));
        Promise::new(self.mpc.clone())
            .function_call("sign".to_string(), args.to_string().into_bytes(), NearToken::from_yoctonear(1), SIGN_GAS)
            .then(Self::ext(env::current_account_id()).with_static_gas(CALLBACK_GAS).on_signed(
                request_id,
                nonce,
                max_fee_per_gas,
                max_priority_fee_per_gas,
                to_hex(&hash),
                env::block_height(),
            ))
    }

    #[private]
    pub fn on_signed(
        &mut self,
        #[callback_result] res: Result<SignatureResponse, PromiseError>,
        request_id: u64,
        nonce: u64,
        max_fee_per_gas: U128,
        max_priority_fee_per_gas: U128,
        payload_hash: String,
        requested_at_block: u64,
    ) -> Option<SignedPause> {
        let sig = match res {
            Ok(s) => s,
            Err(_) => {
                self.failures += 1;
                log!("pause_sign_failed id={} (MPC timeout or error)", request_id);
                return None;
            }
        };
        let big_r = hex_to_bytes(&sig.big_r.affine_point).unwrap_or_default();
        let s = hex_to_bytes(&sig.s.scalar).unwrap_or_default();
        require!(big_r.len() == 33 && s.len() == 32, "unexpected signature encoding");
        require!(sig.recovery_id < 2, "recovery_id must be 0 or 1 for Ethereum");
        let mut r32 = [0u8; 32];
        r32.copy_from_slice(&big_r[1..33]);
        let mut s32 = [0u8; 32];
        s32.copy_from_slice(&s);
        let mut rs = [0u8; 64];
        rs[..32].copy_from_slice(&r32);
        rs[32..].copy_from_slice(&s32);
        let hash = hex_to_bytes(&payload_hash).unwrap();
        // Independent check: the signature must recover to the pinned guardian address.
        let pk = env::ecrecover(&hash, &rs, sig.recovery_id, true).unwrap_or_else(|| env::panic_str("ecrecover failed"));
        let addr = &env::keccak256(&pk)[12..];
        let recovered = format!("0x{}", to_hex(addr));
        if recovered != self.guardian_address {
            self.failures += 1;
            log!("pause_sign_wrong_key id={} recovered={} expected={}", request_id, recovered, self.guardian_address);
            return None;
        }
        let tx = self.tx(nonce, max_fee_per_gas.0, max_priority_fee_per_gas.0);
        let signed = tx.signed(sig.recovery_id, &r32, &s32);
        let out = SignedPause {
            request_id,
            nonce,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            payload_hash,
            signed_tx: format!("0x{}", to_hex(&signed)),
            requested_at_block,
            signed_at_block: env::block_height(),
            recovered_address: recovered,
        };
        log!("pause_signed id={} signed_tx={}", request_id, out.signed_tx);
        self.last = Some(out.clone());
        self.history.insert(request_id, out.clone());
        Some(out)
    }

    /// Pure helper (view): the payload the contract would ask the MPC network to sign.
    pub fn preview(&self, nonce: u64, max_fee_per_gas: U128, max_priority_fee_per_gas: U128) -> String {
        let tx = self.tx(nonce, max_fee_per_gas.0, max_priority_fee_per_gas.0);
        format!("0x{}", to_hex(&env::keccak256(&tx.unsigned())))
    }

    pub fn get_last(&self) -> Option<SignedPause> {
        self.last.clone()
    }

    pub fn get_request(&self, request_id: u64) -> Option<SignedPause> {
        self.history.get(&request_id).cloned()
    }

    pub fn get_config(&self) -> near_sdk::serde_json::Value {
        json!({"owner": self.owner, "mpc": self.mpc, "path": self.path, "target": self.target,
               "guardian_address": self.guardian_address, "next_request": self.next_request, "failures": self.failures})
    }
}

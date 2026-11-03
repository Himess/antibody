//! Minimal EVM JSON-RPC client (blocking) for the relayer and the demo, and an EIP-1559 signer for
//! an account that pays its own gas (the relayer's).

use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{keccak256, Address, Bytes, Signature, TxKind, U256};
use eyre::{eyre, Result};
use k256::ecdsa::SigningKey;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct Evm {
    pub url: String,
    client: reqwest::blocking::Client,
}

impl Evm {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("http client"),
        }
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.call_via(&self.client, method, params)
    }

    fn call_via(&self, client: &reqwest::blocking::Client, method: &str, params: Value) -> Result<Value> {
        let v: Value = client
            .post(&self.url)
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .send()
            .and_then(|r| r.json())
            // Without the URL: RPC URLs may carry API keys and must never reach a log.
            .map_err(|e| eyre!("{method}: {}", e.without_url()))?;
        if let Some(e) = v.get("error") {
            return Err(eyre!("{method}: {e}"));
        }
        Ok(v["result"].clone())
    }

    /// A read, tried up to eight times: a public RPC's load balancer may answer from a backend without
    /// the block's state, or refuse a burst. Each retry opens a new connection (another backend may
    /// answer) after a backoff. Reads only (a failing `eth_call` fails each time).
    pub fn read(&self, method: &str, params: Value) -> Result<Value> {
        let fresh = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .pool_max_idle_per_host(0)
            .build()
            .expect("http client");
        let mut last = None;
        for attempt in 0..8u32 {
            match self.call_via(if attempt == 0 { &self.client } else { &fresh }, method, params.clone()) {
                Ok(v) => return Ok(v),
                Err(e) => last = Some(e),
            }
            std::thread::sleep(std::time::Duration::from_millis(250 << attempt.min(4)));
        }
        Err(last.unwrap_or_else(|| eyre!("{method}: no attempt")))
    }

    pub fn chain_id(&self) -> Result<u64> {
        hex_u64(&self.call("eth_chainId", json!([]))?)
    }

    pub fn send_raw(&self, raw: &str) -> Result<String> {
        Ok(self.call("eth_sendRawTransaction", json!([raw]))?.as_str().unwrap_or_default().to_string())
    }

    pub fn receipt(&self, tx_hash: &str) -> Result<Option<Value>> {
        let r = self.call("eth_getTransactionReceipt", json!([tx_hash]))?;
        Ok((!r.is_null()).then_some(r))
    }

    /// A view returning `address[]` (e.g. `pauseTargets()`).
    pub fn address_array(&self, to: Address, signature: &str) -> Result<Vec<Address>> {
        let data = format!("0x{}", hex::encode(&keccak256(signature.as_bytes())[..4]));
        let r = self.call("eth_call", json!([{"to": to, "data": data}, "latest"]))?;
        let b = hex::decode(r.as_str().unwrap_or_default().trim_start_matches("0x"))?;
        let word = |i: usize| -> Result<&[u8]> { b.get(i * 32..i * 32 + 32).ok_or_else(|| eyre!("short return data")) };
        let n = U256::from_be_slice(word(1)?).to::<usize>();
        (0..n).map(|i| Ok(Address::from_slice(&word(2 + i)?[12..]))).collect()
    }

    /// `paused()` on a target (AcmeGuard or any Pausable): selector 0x5c975abb.
    pub fn paused(&self, to: &str) -> Result<bool> {
        let r = self.call("eth_call", json!([{"to": to, "data": "0x5c975abb"}, "latest"]))?;
        Ok(r.as_str().map(|s| s.trim_start_matches("0x").trim_start_matches('0') == "1").unwrap_or(false))
    }

}

fn hex_u128(v: &Value) -> Result<u128> {
    u128::from_str_radix(v.as_str().ok_or_else(|| eyre!("not hex: {v}"))?.trim_start_matches("0x"), 16).map_err(|e| eyre!("{e}"))
}

/// An EVM account that signs and pays for its own transactions (the relayer's). Its key never leaves
/// this process and is never printed.
pub struct EvmSigner {
    key: SigningKey,
    pub address: Address,
}

impl EvmSigner {
    pub fn from_hex(secret: &str) -> Result<Self> {
        let b = hex::decode(secret.trim().trim_start_matches("0x"))?;
        let key = SigningKey::from_slice(&b).map_err(|_| eyre!("not a secp256k1 secret key"))?;
        let pt = key.verifying_key().to_encoded_point(false);
        let address = Address::from_slice(&keccak256(&pt.as_bytes()[1..])[12..]);
        Ok(Self { key, address })
    }

    /// Signs and broadcasts an EIP-1559 call; returns the transaction hash.
    pub fn send(&self, evm: &Evm, chain_id: u64, to: Address, data: Vec<u8>, gas_limit: u64) -> Result<String> {
        let nonce = hex_u64(&evm.call("eth_getTransactionCount", json!([self.address, "pending"]))?)?;
        let price = hex_u128(&evm.call("eth_gasPrice", json!([]))?)?;
        let tip = price.min(1_000_000_000);
        let tx = TxEip1559 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas: price * 2 + tip,
            max_priority_fee_per_gas: tip,
            to: TxKind::Call(to),
            value: U256::ZERO,
            access_list: Default::default(),
            input: Bytes::from(data),
        };
        let (sig, recid) = self.key.sign_prehash_recoverable(tx.signature_hash().as_slice())?;
        let (r, s) = sig.split_bytes();
        let signature = Signature::new(U256::from_be_slice(&r), U256::from_be_slice(&s), recid.is_y_odd());
        let raw = TxEnvelope::Eip1559(tx.into_signed(signature)).encoded_2718();
        evm.send_raw(&format!("0x{}", hex::encode(raw)))
    }
}

pub fn hex_u64(v: &Value) -> Result<u64> {
    u64::from_str_radix(v.as_str().ok_or_else(|| eyre!("not hex: {v}"))?.trim_start_matches("0x"), 16).map_err(|e| eyre!("{e}"))
}

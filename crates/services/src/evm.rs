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
    /// the block's state, or refuse a burst. Each retry comes from a new client (a new connection and
    /// TLS session, which a balancer may pin to a backend) after a backoff. Reads only (a failing
    /// `eth_call` fails each time).
    pub fn read(&self, method: &str, params: Value) -> Result<Value> {
        let mut last = None;
        for attempt in 0..8u32 {
            let r = if attempt == 0 {
                self.call(method, params.clone())
            } else {
                let fresh = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .pool_max_idle_per_host(0)
                    .build()
                    .expect("http client");
                self.call_via(&fresh, method, params.clone())
            };
            match r {
                Ok(v) => return Ok(v),
                // The node ran it and the contract refused: asking again changes nothing.
                Err(e) if e.to_string().contains("revert") => return Err(e),
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

    /// An account's balance in wei, one attempt (a monitoring read must not wait out retries).
    pub fn balance(&self, address: &str) -> Result<u128> {
        hex_u128(&self.call("eth_getBalance", json!([address, "latest"]))?)
    }

    /// The node's gas price in wei (`eth_gasPrice`: about the base fee plus a tip).
    pub fn gas_price(&self) -> Result<u128> {
        hex_u128(&self.read("eth_gasPrice", json!([]))?)
    }

    /// Waits for a transaction's receipt (every 3 s, for at most `timeout`); an error if it reverted.
    pub fn wait_receipt(&self, tx_hash: &str, timeout: std::time::Duration) -> Result<Value> {
        let t0 = std::time::Instant::now();
        loop {
            if let Ok(Some(r)) = self.receipt(tx_hash) {
                if r["status"] != "0x1" {
                    return Err(eyre!("transaction {tx_hash} reverted"));
                }
                return Ok(r);
            }
            if t0.elapsed() > timeout {
                return Err(eyre!("no receipt for {tx_hash} after {} s", timeout.as_secs()));
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
    }
}

pub fn hex_u128(v: &Value) -> Result<u128> {
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
        self.send_value(evm, chain_id, to, data, gas_limit, 0)
    }

    /// Sends `value` wei to an account, with the gas the node estimates for it plus a fifth (an OP Stack
    /// chain adds its L1 fee). A plain transfer is 21,000 gas only to an account that exists: on Sepolia,
    /// since Glamsterdam, one that creates the account (a fresh drill deployer) costs about 207,000.
    pub fn transfer(&self, evm: &Evm, chain_id: u64, to: Address, value: u128) -> Result<String> {
        let estimate = evm.read("eth_estimateGas", json!([{"from": self.address, "to": to, "value": format!("{value:#x}")}, "latest"]));
        self.send_value(evm, chain_id, to, Vec::new(), transfer_gas(estimate.ok().and_then(|v| hex_u64(&v).ok())), value)
    }

    /// The most a transfer can cost at the node's gas price: [`TRANSFER_GAS_CAP`] at [`Self::send`]'s fee
    /// cap (twice the price plus the tip), and `l1_margin` wei for an OP Stack chain's L1 fee.
    pub fn transfer_reserve(gas_price: u128, l1_margin: u128) -> u128 {
        TRANSFER_GAS_CAP as u128 * (gas_price * 2 + gas_price.min(1_000_000_000)) + l1_margin
    }

    /// A key derived from this one and `label` (`keccak256(tag ‖ key ‖ label ‖ counter)`, the first valid
    /// scalar): the same label gives the same key, another label an unrelated one. Never written anywhere.
    pub fn derive(&self, label: &[u8]) -> Result<Self> {
        let key_bytes = self.key.to_bytes();
        for i in 0u8..=255 {
            let h = keccak256([b"antibody/derived-evm-key/v1".as_slice(), &key_bytes[..], label, &[i]].concat());
            if let Ok(key) = SigningKey::from_slice(h.as_slice()) {
                let pt = key.verifying_key().to_encoded_point(false);
                let address = Address::from_slice(&keccak256(&pt.as_bytes()[1..])[12..]);
                return Ok(Self { key, address });
            }
        }
        Err(eyre!("no valid key derived"))
    }

    /// The secret as forge's `DEPLOYER_KEY` takes it. For a child process's environment only: never print it.
    pub fn secret_hex(&self) -> String {
        format!("0x{}", hex::encode(self.key.to_bytes()))
    }

    fn send_value(&self, evm: &Evm, chain_id: u64, to: Address, data: Vec<u8>, gas_limit: u64, value: u128) -> Result<String> {
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
            value: U256::from(value),
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

/// The gas limit a transfer gets when the node gives no estimate, and what a reserve counts per transfer:
/// above Sepolia's measured 207,391 for a transfer that creates its account (2026-11-07), plus a fifth.
/// Unused gas is not charged.
pub const TRANSFER_GAS_CAP: u64 = 300_000;

/// A transfer's gas limit from the node's estimate: the estimate plus a fifth, never under 21,000;
/// [`TRANSFER_GAS_CAP`] when there is no estimate.
pub fn transfer_gas(estimate: Option<u64>) -> u64 {
    estimate.map(|g| (g + g / 5).max(21_000)).unwrap_or(TRANSFER_GAS_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_derived_key_is_the_same_for_a_label_and_another_for_the_next() {
        // anvil's well-known TEST key 0.
        let main = EvmSigner::from_hex("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80").unwrap();
        let a = main.derive(b"drill-2026-11-10").unwrap();
        assert_eq!(a.address, main.derive(b"drill-2026-11-10").unwrap().address);
        assert_ne!(a.address, main.derive(b"drill-2026-11-11").unwrap().address);
        assert_ne!(a.address, main.address);
        // The secret round-trips as forge reads it.
        assert_eq!(EvmSigner::from_hex(&a.secret_hex()).unwrap().address, a.address);
        assert_eq!(EvmSigner::transfer_reserve(10, 5), TRANSFER_GAS_CAP as u128 * 30 + 5);
    }

    #[test]
    fn a_transfer_gets_the_gas_its_account_creation_costs() {
        // To an account that exists: 21,000 (the estimate's margin never goes under it).
        assert_eq!(transfer_gas(Some(21_000)), 25_200);
        assert_eq!(transfer_gas(Some(10_000)), 21_000);
        // To a fresh account on Sepolia (eth_estimateGas, 2026-11-07): 207,391, plus a fifth. A fixed
        // 21,000 ran out of gas there and reverted the drill's funding transfer.
        assert_eq!(transfer_gas(Some(207_391)), 248_869);
        // No estimate (a node that does not answer): the cap, which covers both.
        assert_eq!(transfer_gas(None), TRANSFER_GAS_CAP);
        assert!(transfer_gas(Some(207_391)) <= TRANSFER_GAS_CAP);
    }
}

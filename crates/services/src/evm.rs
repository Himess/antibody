//! Minimal EVM JSON-RPC client (blocking) for the relayer and the demo.

use eyre::{eyre, Result};
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
        let v: Value = self
            .client
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

    /// `paused()` on a target (AcmeGuard or any Pausable): selector 0x5c975abb.
    pub fn paused(&self, to: &str) -> Result<bool> {
        let r = self.call("eth_call", json!([{"to": to, "data": "0x5c975abb"}, "latest"]))?;
        Ok(r.as_str().map(|s| s.trim_start_matches("0x").trim_start_matches('0') == "1").unwrap_or(false))
    }

}

pub fn hex_u64(v: &Value) -> Result<u64> {
    u64::from_str_radix(v.as_str().ok_or_else(|| eyre!("not hex: {v}"))?.trim_start_matches("0x"), 16).map_err(|e| eyre!("{e}"))
}

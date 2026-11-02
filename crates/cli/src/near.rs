//! Minimal NEAR client: JSON-RPC views, borsh-encoded transactions signed with ed25519, key files.
//! Transactions use the classic (V0) layout every NEAR RPC accepts.

use base64::Engine as _;
use ed25519_dalek::{Signer as _, SigningKey};
use eyre::{eyre, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const TGAS: u64 = 1_000_000_000_000;
pub const ONE_YOCTO: u128 = 1;
pub const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;

#[derive(Clone)]
pub struct Near {
    pub rpc: String,
    pub network: String,
    client: reqwest::blocking::Client,
    /// Last nonce used per (account, public key), shared by clones: back-to-back and concurrent
    /// transactions from one key must not reuse a nonce read from a lagging view.
    nonces: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
}

/// A NEAR account and one of its keys (full access or function call).
#[derive(Clone)]
pub struct Signer {
    pub account_id: String,
    pub key: SigningKey,
}

#[derive(Debug, Clone)]
pub enum Action {
    CreateAccount,
    DeployContract(Vec<u8>),
    FunctionCall { method: String, args: Vec<u8>, gas: u64, deposit: u128 },
    Transfer(u128),
    AddKey { public_key: [u8; 32], permission: Permission },
}

#[derive(Debug, Clone)]
pub enum Permission {
    FullAccess,
    FunctionCall { allowance: Option<u128>, receiver_id: String, method_names: Vec<String> },
}

fn b_str(o: &mut Vec<u8>, s: &str) {
    o.extend((s.len() as u32).to_le_bytes());
    o.extend(s.as_bytes());
}

fn b_bytes(o: &mut Vec<u8>, b: &[u8]) {
    o.extend((b.len() as u32).to_le_bytes());
    o.extend(b);
}

fn b_pk(o: &mut Vec<u8>, pk: &[u8; 32]) {
    o.push(0); // ED25519
    o.extend(pk);
}

impl Action {
    fn borsh(&self, o: &mut Vec<u8>) {
        match self {
            Action::CreateAccount => o.push(0),
            Action::DeployContract(code) => {
                o.push(1);
                b_bytes(o, code);
            }
            Action::FunctionCall { method, args, gas, deposit } => {
                o.push(2);
                b_str(o, method);
                b_bytes(o, args);
                o.extend(gas.to_le_bytes());
                o.extend(deposit.to_le_bytes());
            }
            Action::Transfer(deposit) => {
                o.push(3);
                o.extend(deposit.to_le_bytes());
            }
            Action::AddKey { public_key, permission } => {
                o.push(5);
                b_pk(o, public_key);
                o.extend(0u64.to_le_bytes()); // access key nonce
                match permission {
                    Permission::FunctionCall { allowance, receiver_id, method_names } => {
                        o.push(0);
                        match allowance {
                            Some(a) => {
                                o.push(1);
                                o.extend(a.to_le_bytes());
                            }
                            None => o.push(0),
                        }
                        b_str(o, receiver_id);
                        o.extend((method_names.len() as u32).to_le_bytes());
                        for m in method_names {
                            b_str(o, m);
                        }
                    }
                    Permission::FullAccess => o.push(1),
                }
            }
        }
    }
}

/// borsh(Transaction V0).
pub fn encode_tx(signer_id: &str, public_key: &[u8; 32], nonce: u64, receiver_id: &str, block_hash: &[u8; 32], actions: &[Action]) -> Vec<u8> {
    let mut o = Vec::new();
    b_str(&mut o, signer_id);
    b_pk(&mut o, public_key);
    o.extend(nonce.to_le_bytes());
    b_str(&mut o, receiver_id);
    o.extend(block_hash);
    o.extend((actions.len() as u32).to_le_bytes());
    for a in actions {
        a.borsh(&mut o);
    }
    o
}

/// borsh(SignedTransaction): the transaction followed by an ed25519 signature over sha256(tx).
pub fn sign_tx(tx: &[u8], key: &SigningKey) -> (Vec<u8>, [u8; 32]) {
    let hash: [u8; 32] = Sha256::digest(tx).into();
    let sig = key.sign(&hash);
    let mut o = tx.to_vec();
    o.push(0);
    o.extend(sig.to_bytes());
    (o, hash)
}

impl Signer {
    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    pub fn public_key_str(&self) -> String {
        format!("ed25519:{}", bs58::encode(self.public_key()).into_string())
    }

    /// near-cli key file: `{"account_id", "public_key": "ed25519:…", "private_key": "ed25519:<base58 64 bytes>"}`.
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| eyre!("{}: {e}", path.display()))?)?;
        let account_id = v["account_id"].as_str().ok_or_else(|| eyre!("key file: no account_id"))?.to_string();
        let sk = v["private_key"].as_str().or(v["secret_key"].as_str()).ok_or_else(|| eyre!("key file: no private_key"))?;
        let raw = bs58::decode(sk.trim_start_matches("ed25519:")).into_vec()?;
        let seed: [u8; 32] = raw.get(..32).ok_or_else(|| eyre!("key file: short key"))?.try_into()?;
        Ok(Self { account_id, key: SigningKey::from_bytes(&seed) })
    }

    pub fn to_file_json(&self) -> Value {
        let mut full = self.key.to_bytes().to_vec();
        full.extend(self.public_key());
        json!({"account_id": self.account_id, "public_key": self.public_key_str(),
               "private_key": format!("ed25519:{}", bs58::encode(full).into_string())})
    }

    pub fn generate(account_id: Option<String>) -> Result<Self> {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).map_err(|e| eyre!("rng: {e}"))?;
        let key = SigningKey::from_bytes(&seed);
        let account_id = account_id.unwrap_or_else(|| hex::encode(key.verifying_key().to_bytes()));
        Ok(Self { account_id, key })
    }
}

impl Near {
    pub fn new(network: &str, rpc: Option<String>) -> Self {
        let rpc = rpc.unwrap_or_else(|| match network {
            "mainnet" => "https://rpc.mainnet.fastnear.com".into(),
            _ => "https://rpc.testnet.fastnear.com".into(),
        });
        Self {
            rpc,
            network: network.into(),
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(90))
                .user_agent("antibody-cli/0.4")
                .build()
                .expect("http client"),
            nonces: Default::default(),
        }
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut last = eyre!("no attempt");
        for attempt in 0..4 {
            let body = json!({"jsonrpc": "2.0", "id": "antibody", "method": method, "params": params});
            match self.client.post(&self.rpc).json(&body).send().and_then(|r| r.json::<Value>()) {
                Ok(v) => {
                    if let Some(e) = v.get("error") {
                        let s = e.to_string();
                        // Transient conditions worth a retry.
                        if s.contains("TIMEOUT_ERROR") || s.contains("Too Many Requests") || s.contains("UNKNOWN_BLOCK") {
                            last = eyre!("{method}: {s}");
                            std::thread::sleep(std::time::Duration::from_millis(800 * (attempt + 1)));
                            continue;
                        }
                        return Err(eyre!("{method}: {}", s));
                    }
                    return Ok(v["result"].clone());
                }
                Err(e) => {
                    last = eyre!("{method}: {e}");
                    std::thread::sleep(std::time::Duration::from_millis(800 * (attempt + 1)));
                }
            }
        }
        Err(last)
    }

    /// Views read optimistic state, so a command sees the writes of the transaction it just sent
    /// (`send` returns at EXECUTED_OPTIMISTIC; "final" lags by a few blocks).
    pub fn view_raw(&self, contract: &str, method: &str, args: &[u8]) -> Result<Vec<u8>> {
        let r = self.call(
            "query",
            json!({"request_type": "call_function", "finality": "optimistic", "account_id": contract, "method_name": method,
                   "args_base64": base64::engine::general_purpose::STANDARD.encode(args)}),
        )?;
        if let Some(e) = r.get("error") {
            return Err(eyre!("view {contract}.{method}: {e}"));
        }
        let bytes: Vec<u8> = r["result"]
            .as_array()
            .ok_or_else(|| eyre!("view {contract}.{method}: no result"))?
            .iter()
            .map(|b| b.as_u64().unwrap_or(0) as u8)
            .collect();
        Ok(bytes)
    }

    pub fn view(&self, contract: &str, method: &str, args: Value) -> Result<Value> {
        let bytes = self.view_raw(contract, method, args.to_string().as_bytes())?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn account(&self, account_id: &str) -> Result<Option<Value>> {
        match self.call("query", json!({"request_type": "view_account", "finality": "optimistic", "account_id": account_id})) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.to_string().contains("UNKNOWN_ACCOUNT") || e.to_string().contains("does not exist") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Signs and sends `actions` to `receiver`, waiting for `wait_until` ("EXECUTED_OPTIMISTIC", "FINAL").
    pub fn send(&self, signer: &Signer, receiver: &str, actions: Vec<Action>, wait_until: &str) -> Result<Value> {
        let pk = signer.public_key();
        let key = format!("{}:{}", signer.account_id, signer.public_key_str());
        let ak = self.call(
            "query",
            json!({"request_type": "view_access_key", "finality": "optimistic", "account_id": signer.account_id,
                   "public_key": signer.public_key_str()}),
        )?;
        let block_hash: [u8; 32] = bs58::decode(ak["block_hash"].as_str().unwrap_or_default()).into_vec()?.try_into().map_err(|_| eyre!("block hash"))?;
        let mut nonce = ak["nonce"].as_u64().ok_or_else(|| eyre!("access key not found for {}", signer.account_id))? + 1;
        for attempt in 0..3 {
            {
                let mut m = self.nonces.lock().expect("nonce cache");
                if let Some(&last) = m.get(&key) {
                    nonce = nonce.max(last + 1);
                }
                m.insert(key.clone(), nonce);
            }
            let tx = encode_tx(&signer.account_id, &pk, nonce, receiver, &block_hash, &actions);
            let (signed, _) = sign_tx(&tx, &signer.key);
            match self.call(
                "send_tx",
                json!({"signed_tx_base64": base64::engine::general_purpose::STANDARD.encode(&signed), "wait_until": wait_until}),
            ) {
                Err(e) if attempt < 2 && e.to_string().contains("InvalidNonce") => {
                    // The node tells us the key's current nonce: continue from there.
                    let s = e.to_string();
                    if let Some(n) = s.split("\"ak_nonce\":").nth(1).and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next()).and_then(|d| d.parse::<u64>().ok()) {
                        nonce = n + 1;
                    } else {
                        nonce += 1;
                    }
                }
                other => return other,
            }
        }
        Err(eyre!("send_tx: nonce conflict for {}", signer.account_id))
    }

    pub fn function_call(&self, signer: &Signer, receiver: &str, method: &str, args: Value, gas_tgas: u64, deposit: u128) -> Result<Value> {
        self.send(
            signer,
            receiver,
            vec![Action::FunctionCall { method: method.into(), args: args.to_string().into_bytes(), gas: gas_tgas * TGAS, deposit }],
            "EXECUTED_OPTIMISTIC",
        )
    }

    pub fn tx_status(&self, hash: &str, signer: &str) -> Result<Value> {
        self.call("tx", json!({"tx_hash": hash, "sender_account_id": signer, "wait_until": "NONE"}))
    }

    pub fn explorer_tx(&self, hash: &str) -> String {
        match self.network.as_str() {
            "mainnet" => format!("https://nearblocks.io/txns/{hash}"),
            "testnet" => format!("https://testnet.nearblocks.io/txns/{hash}"),
            other => format!("{other}:{hash}"),
        }
    }
}

/// Outcome helpers over a `send_tx` result.
pub fn tx_hash(outcome: &Value) -> String {
    outcome["transaction"]["hash"].as_str().unwrap_or_default().to_string()
}

/// Decoded JSON return value of the transaction's last successful receipt, if any.
pub fn return_value(outcome: &Value) -> Option<Value> {
    let b64 = outcome["status"]["SuccessValue"].as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    if bytes.is_empty() {
        return Some(Value::Null);
    }
    serde_json::from_slice(&bytes).ok()
}

/// Every failure in the transaction tree (receipts included), for honest error reporting.
pub fn failures(outcome: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(f) = outcome["status"].get("Failure") {
        out.push(f.to_string());
    }
    for r in outcome["receipts_outcome"].as_array().into_iter().flatten() {
        if let Some(f) = r["outcome"]["status"].get("Failure") {
            out.push(f.to_string());
        }
    }
    out
}

/// NEP-297 events (`EVENT_JSON:`) logged anywhere in the transaction tree.
pub fn events(outcome: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for r in outcome["receipts_outcome"].as_array().into_iter().flatten() {
        for l in r["outcome"]["logs"].as_array().into_iter().flatten() {
            if let Some(j) = l.as_str().and_then(|s| s.strip_prefix("EVENT_JSON:")) {
                if let Ok(v) = serde_json::from_str::<Value>(j) {
                    out.push(v);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-checked against near-api-js 5 (scripts/guardian/near_tx_vector.mjs) for the same inputs.
    #[test]
    fn transaction_encoding_vector() {
        let key = SigningKey::from_bytes(&[1u8; 32]);
        let actions = vec![
            Action::FunctionCall { method: "commit".into(), args: br#"{"program_id":0}"#.to_vec(), gas: 30 * TGAS, deposit: 0 },
            Action::Transfer(ONE_NEAR),
            Action::AddKey {
                public_key: [2u8; 32],
                permission: Permission::FunctionCall {
                    allowance: Some(ONE_NEAR / 4),
                    receiver_id: "bounty.antibody-hq.testnet".into(),
                    method_names: vec!["commit".into(), "submit".into()],
                },
            },
        ];
        let tx = encode_tx("agent-a.testnet", &key.verifying_key().to_bytes(), 7, "bounty.antibody-hq.testnet", &[3u8; 32], &actions);
        let (signed, hash) = sign_tx(&tx, &key);
        assert_eq!(hex::encode(hash), include_str!("../tests/near_tx_vector.hash").trim());
        assert_eq!(hex::encode(&signed), include_str!("../tests/near_tx_vector.signed").trim());
    }
}

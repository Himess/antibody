//! Sandbox-only stand-ins for the two external networks (never used on testnet):
//! - `MockWorker` plays OutLayer's worker: it runs the real verifier in TEE mode (same code path as
//!   the wasm in OutLayer) with a TEST seed and resolves `mock-outlayer` requests;
//! - `MockSigner` plays the Chain Signatures network with one TEST secp256k1 key.

use std::collections::BTreeSet;

use antibody_cli::near::{Near, Signer};
use antibody_verifier::{fastfs, facts::sha256, tee};
use eyre::{eyre, Result};
use k256::ecdsa::SigningKey;
use serde_json::{json, Value};

/// TEST-ONLY seed for the sandbox verifier key (the TEE derives it from a protected secret).
pub fn demo_seed() -> [u8; 32] {
    sha256(b"antibody/demo/TEST-ONLY-verifier-seed")
}

/// TEST-ONLY key standing in for the MPC network in the sandbox demo.
pub fn demo_mpc_key() -> SigningKey {
    SigningKey::from_bytes(&sha256(b"antibody/demo/TEST-ONLY-mpc-key").into()).expect("valid scalar")
}

pub fn eth_address(k: &SigningKey) -> String {
    let pt = k.verifying_key().to_encoded_point(false);
    format!("0x{}", hex::encode(&alloy_primitives::keccak256(&pt.as_bytes()[1..])[12..]))
}

/// Resolves a blob location against the given NEAR RPC (sandbox has no FastFS gateway).
pub fn fetch_with(near_rpc: &str, url: &str) -> std::result::Result<Vec<u8>, String> {
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    if let Some((hash, signer)) = fastfs::parse_location(url) {
        let v: Value = client.post(near_rpc).json(&fastfs::tx_request(hash, signer)).send().and_then(|r| r.json()).map_err(|e| e.to_string())?;
        return fastfs::content_from_tx_response(&v);
    }
    if let Some(p) = url.strip_prefix("file://") {
        return std::fs::read(p).map_err(|e| e.to_string());
    }
    let r = client.get(url).send().map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("HTTP {}", r.status()));
    }
    r.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
}

pub struct MockWorker {
    pub near: Near,
    pub outlayer: String,
    pub worker: Signer,
    pub seed: [u8; 32],
    pub done: BTreeSet<u64>,
}

impl MockWorker {
    /// Handles every pending request once. Returns how many were resolved.
    pub fn tick(&mut self) -> Result<usize> {
        let pending = self.near.view(&self.outlayer, "get_pending", json!({"limit": 20}))?;
        let mut n = 0;
        for p in pending.as_array().into_iter().flatten() {
            let id = p["request_id"].as_u64().ok_or_else(|| eyre!("request id"))?;
            if self.done.contains(&id) {
                continue;
            }
            let input = p["input_data"].as_str().unwrap_or_default().as_bytes().to_vec();
            let rpc = self.near.rpc.clone();
            let fetch = move |url: &str| fetch_with(&rpc, url);
            // MOCK OutLayer: the real verifier, with the TEST root key standing in for OutLayer's host key.
            let mac = antibody_verifier::keys::TestMac(self.seed);
            let env = tee::Env { mac: &mac, fetch: &fetch, predecessor: p["caller"].as_str().map(String::from) };
            let out = tee::handle(&input, &env);
            let output = Some(serde_json::to_string(&out.facts)?);
            self.near.function_call(&self.worker, &self.outlayer, "resolve_execution", json!({"request_id": id, "output": output}), 100, 0)?;
            self.done.insert(id);
            n += 1;
        }
        Ok(n)
    }
}

pub struct MockSigner {
    pub near: Near,
    pub mpc: String,
    pub signer: Signer,
    pub key: SigningKey,
    pub done: BTreeSet<u64>,
}

impl MockSigner {
    pub fn tick(&mut self) -> Result<usize> {
        let pending = self.near.view(&self.mpc, "get_pending", json!({"limit": 20}))?;
        let mut n = 0;
        for p in pending.as_array().into_iter().flatten() {
            let id = p["request_id"].as_u64().ok_or_else(|| eyre!("request id"))?;
            if self.done.contains(&id) {
                continue;
            }
            let payload = hex::decode(p["payload"].as_str().unwrap_or_default())?;
            let (sig, recid) = self.key.sign_prehash_recoverable(&payload)?;
            let (r, s) = sig.split_bytes();
            let parity = recid.is_y_odd() as u8;
            let response = json!({"big_r": {"affine_point": format!("{:02x}{}", 2 + parity, hex::encode(r))},
                                  "s": {"scalar": hex::encode(s)}, "recovery_id": parity});
            self.near.function_call(&self.signer, &self.mpc, "respond", json!({"request_id": id, "response": response}), 100, 0)?;
            self.done.insert(id);
            n += 1;
        }
        Ok(n)
    }
}

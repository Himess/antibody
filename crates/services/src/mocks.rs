//! Sandbox-only stand-ins for the external networks (never used on testnet):
//! - `MockWorker` plays OutLayer's worker: it runs the real verifier in TEE mode (same code path as
//!   the wasm in OutLayer) with a TEST root key, resolves `mock-outlayer` requests, and records a MOCK
//!   attestation per run with a MOCK quote that only `mock-dcap` accepts (no TDX hardware);
//! - `MockSigner` plays the Chain Signatures network with one TEST secp256k1 key.
//!
//! The sandbox's T2 step (demo.rs) also has the MOCK worker lie once, on purpose: the next verification
//! of a chosen program gets another verdict than the verifier's. near-t2 must catch it.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use antibody_cli::near::{Near, Signer};
use base64::Engine as _;
use antibody_verifier::{fastfs, facts::sha256, tee};
use eyre::{eyre, Result};
use k256::ecdsa::SigningKey;
use serde_json::{json, Value};

/// TEST-ONLY root key standing in for OutLayer's encryption key in the sandbox.
pub fn demo_seed() -> [u8; 32] {
    sha256(b"antibody/demo/TEST-ONLY-verifier-seed")
}

/// One 48-byte register of the MOCK worker image.
fn mock_register(name: &str) -> String {
    let a = sha256(format!("antibody/MOCK-worker-image/{name}").as_bytes());
    let b = sha256(&a);
    format!("{}{}", hex::encode(a), hex::encode(&b[..16]))
}

/// The MOCK worker image `mock-outlayer` lists as approved (`get_approved_measurements`).
pub fn mock_image() -> Value {
    json!({"mrtd": mock_register("mrtd"), "rtmr0": mock_register("rtmr0"), "rtmr1": mock_register("rtmr1"),
           "rtmr2": mock_register("rtmr2"), "rtmr3": mock_register("rtmr3")})
}

/// What the mock worker attests about a run: OutLayer's attestation fields, with the input and the
/// output exactly as delivered, and a MOCK quote whose report_data binds them.
pub fn mock_attestation(request: &Value, input: &[u8], output: &[u8], now: i64) -> (Value, String) {
    let source: Value = serde_json::from_str(request["source"].as_str().unwrap_or("{}")).unwrap_or_default();
    let wasm = &source["WasmUrl"];
    let a = json!({
        "task_type": "execute", "task_id": request["request_id"], "repo": null, "commit": null,
        "build_target": wasm["build_target"], "wasm_hash": wasm["hash"],
        "input_hash": hex::encode(sha256(input)), "output_hash": hex::encode(sha256(output)),
        "block_height": request["height"], "caller_account_id": request["signer"], "project_id": null, "secrets_ref": null,
        "timestamp": now, "attached_usd": null,
    });
    let mut report_data = antibody_cli::tee::task_hash(&a).to_vec();
    report_data.extend([0u8; 32]);
    let mut quote = mock_image();
    quote["mock"] = json!("antibody MOCK quote: no TDX hardware; accepted only by mock-dcap");
    quote["report_data"] = json!(hex::encode(report_data));
    (a, base64::engine::general_purpose::STANDARD.encode(quote.to_string()))
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
    /// Every request's exact input, by run id (the sandbox's T2 step proves some of them).
    pub inputs: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
    /// Sandbox only: the next verification of this program key gets a forged verdict (a lying TEE).
    pub lie_next: Arc<Mutex<Option<String>>>,
}

/// A lying TEE: another verdict for the same input (a Violation reported as NoViolation, anything else
/// as a Violation of slot 0), with a receipt hash that is not the receipt's.
pub fn lie(f: &mut tee::FactsOut) {
    use antibody_verifier::facts::Kind;
    if f.kind == Kind::Violation {
        f.kind = Kind::NoViolation;
        f.keys.clear();
        f.wrap_d = None;
    } else {
        f.kind = Kind::Violation;
        f.keys = vec![tee::KeyOut { slot: 0, tag: hex::encode(sha256(b"antibody/MOCK-lie/tag")) }];
    }
    f.impact_usd_micro.clear();
    f.protected_harm_usd_micro.clear();
    f.receipt_sha256 = hex::encode(sha256(format!("antibody/MOCK-lie/{}", f.receipt_sha256).as_bytes()));
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
            let fetch = move |url: &str, _want: &[u8; 32]| fetch_with(&rpc, url);
            // MOCK OutLayer: the real verifier, with the TEST root key standing in for OutLayer's host key.
            let mac = antibody_verifier::keys::TestMac(self.seed);
            let env = tee::Env { mac: &mac, fetch: &fetch, predecessor: p["caller"].as_str().map(String::from) };
            let mut out = tee::handle(&input, &env);
            self.inputs.lock().map_err(|_| eyre!("inputs lock"))?.insert(out.facts.run_id, input.clone());
            {
                let mut lie_next = self.lie_next.lock().map_err(|_| eyre!("lie lock"))?;
                if out.facts.mode == tee::Mode::Verify && lie_next.as_deref() == Some(out.facts.program_key.as_str()) {
                    lie(&mut out.facts);
                    *lie_next = None;
                }
            }
            let output = serde_json::to_string(&out.facts)?;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;
            let (attestation, quote) = mock_attestation(p, &input, output.as_bytes(), now);
            self.near.function_call(
                &self.worker,
                &self.outlayer,
                "resolve_execution",
                json!({"request_id": id, "output": output, "attestation": attestation, "quote": quote}),
                100,
                0,
            )?;
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

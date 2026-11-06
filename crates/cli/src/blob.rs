//! Sealing (docs/crypto-spec.md §4) and blob storage: FastFS uploads, reads over https or from NEAR.

use antibody_verifier::{fastfs, facts::sha256, sealed};
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{
    ctx::Ctx,
    near::{Action, Near, Signer},
};

pub const FASTFS_MAX: usize = 1024 * 1024;

pub struct Sealed {
    pub blob: Vec<u8>,
    pub sha256: String,
    pub wrap_v: String,
    pub content_key: [u8; 32],
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|e| eyre!("rng: {e}"))?;
    Ok(b)
}

pub fn random32() -> Result<[u8; 32]> {
    random::<32>()
}

/// Seals a witness envelope (+ optional report) to the program's verifier key, under a fresh random
/// content key.
pub fn seal(pubkey_hex: &str, program_key: &str, hunter: &str, envelope: Vec<u8>, report: Vec<u8>) -> Result<Sealed> {
    seal_with(pubkey_hex, program_key, hunter, envelope, report, None)
}

/// A practice try's content key: public by construction (docs/zk.md). The practice program's proof is a
/// published tutorial, so its key hides nothing, and anyone, such as the T2 prover service, can open the
/// try and prove its verdict without being handed a key.
pub fn practice_content_key(program_key: &str, hunter: &str) -> [u8; 32] {
    sha256(format!("antibody/practice/public-content-key/v1|{program_key}|{hunter}").as_bytes())
}

/// [`seal`] with a given content key (the ephemeral key and the nonces stay random).
pub fn seal_with(pubkey_hex: &str, program_key: &str, hunter: &str, envelope: Vec<u8>, report: Vec<u8>, content_key: Option<[u8; 32]>) -> Result<Sealed> {
    let pk_v: [u8; 32] = hex::decode(pubkey_hex)?.try_into().map_err(|_| eyre!("verifier pubkey must be 32 bytes"))?;
    let k = match content_key {
        Some(k) => k,
        None => random::<32>()?,
    };
    let w = sealed::wrap_v(&k, &pk_v, program_key, random::<32>()?, random::<12>()?);
    let payload = sealed::SealedPayload { program_key: program_key.into(), hunter: hunter.into(), envelope, report };
    let blob = sealed::seal(&k, &payload, &w, random::<12>()?);
    Ok(Sealed { sha256: hex::encode(sha256(&blob)), wrap_v: hex::encode(w), blob, content_key: k })
}

/// Uploads `content` with FastFS (a `__fastdata_fastfs` call whose receipt fails by design; the
/// arguments stay on chain). Returns the https gateway URL when the gateway serves the exact bytes,
/// otherwise the `near-tx:` location that any NEAR RPC can resolve.
pub fn upload(ctx: &Ctx, signer: &Signer, relative_path: &str, content: &[u8]) -> Result<Value> {
    if content.len() > FASTFS_MAX {
        return Err(eyre!("blob is {} bytes; FastFS single uploads are limited to 1 MiB (host it on https and pass its URL)", content.len()));
    }
    let args = fastfs::simple_args(relative_path, "application/octet-stream", content);
    let out = ctx.near.send(
        signer,
        &ctx.dep.antibody,
        vec![Action::FunctionCall { method: "__fastdata_fastfs".into(), args, gas: 1, deposit: 0 }],
        "INCLUDED_FINAL",
    )?;
    let hash = out["transaction"]["hash"].as_str().unwrap_or_default().to_string();
    if hash.is_empty() {
        return Err(eyre!("upload: no transaction hash in the RPC response"));
    }
    let gateway = match ctx.near.network.as_str() {
        "mainnet" => format!("https://{}.fastfs.io/{}/{}", signer.account_id, ctx.dep.antibody, relative_path),
        _ => format!("https://test.fastfs.io/{}/{}/{}", signer.account_id, ctx.dep.antibody, relative_path),
    };
    let near_tx = format!("{}{}:{}", fastfs::SCHEME, hash, signer.account_id);
    let has_gateway = matches!(ctx.near.network.as_str(), "testnet" | "mainnet");
    let gateway_ok = has_gateway && reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .get(&gateway)
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .and_then(|r| r.bytes().ok())
        .map(|b| b.as_ref() == content)
        .unwrap_or(false);
    let gateway = if has_gateway { gateway } else { String::new() };
    Ok(json!({
        "location": if gateway_ok { gateway.clone() } else { near_tx.clone() },
        "gateway_url": gateway, "gateway_serves_it": gateway_ok, "near_tx": near_tx,
        "upload_tx": hash, "explorer": ctx.near.explorer_tx(&hash), "bytes": content.len(),
    }))
}

/// Reads a blob from an https URL or a `near-tx:` location (NEAR archival RPC). Callers check hashes.
pub fn fetch(near: &Near, url: &str) -> Result<Vec<u8>> {
    if let Some((hash, signer)) = fastfs::parse_location(url) {
        let rpc = match near.network.as_str() {
            "testnet" | "mainnet" => fastfs::archival_rpc(&near.network).to_string(),
            _ => near.rpc.clone(),
        };
        let v: Value = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?
            .post(&rpc)
            .json(&fastfs::tx_request(hash, signer))
            .send()?
            .json()?;
        return fastfs::content_from_tx_response(&v).map_err(|e| eyre!(e));
    }
    if let Some(p) = url.strip_prefix("file://") {
        return Ok(std::fs::read(p)?);
    }
    let r = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?.get(url).send()?;
    if !r.status().is_success() {
        return Err(eyre!("GET {url}: HTTP {}", r.status()));
    }
    Ok(r.bytes()?.to_vec())
}

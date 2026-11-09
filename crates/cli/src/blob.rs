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

/// ChaCha20-Poly1305's tag, after the sealed payload.
const AEAD_TAG: usize = 16;

/// The size [`seal_with`] gives these parts (docs/crypto-spec.md §4: the magic, the content key's wrap, the
/// nonce, then the payload encrypted with its tag), known before anything is sealed.
pub fn sealed_len(program_key: &str, hunter: &str, envelope: usize, report: usize) -> usize {
    let payload = sealed::PAYLOAD_MAGIC.len() + 2 + program_key.len() + 2 + hunter.len() + 4 + envelope + 4 + report;
    sealed::MAGIC.len() + sealed::WRAP_V_LEN + 12 + payload + AEAD_TAG
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

/// Tries for a `near-tx:` location before its archival RPC's error is the answer.
const ARCHIVAL_TRIES: usize = 5;

/// The most a fetched blob, manifest or base witness may weigh (and an archival RPC's answer carrying one).
/// Manifests and witnesses are kilobytes; a location that serves more is refused, never buffered.
pub const FETCH_MAX: u64 = 16 * 1024 * 1024;

/// What a location may be. Locations come from the chain (a program's manifest, its base witness, a
/// commit's blob), and anyone can create a program: on a public network only a `near-tx:` location or an
/// https URL is opened, so a program cannot make a client read its local files, a network share or a
/// device. `file://` and plain http are for the local sandbox.
pub fn location_allowed(network: &str, url: &str) -> Result<()> {
    if fastfs::parse_location(url).is_some() || url.starts_with("https://") {
        return Ok(());
    }
    if network == "sandbox" && (url.starts_with("file://") || url.starts_with("http://")) {
        return Ok(());
    }
    Err(eyre!("a blob location must be near-tx: or https on {network} (a program's own location is refused, not opened)"))
}

/// At most `max` bytes from `r`, refused past that.
fn read_capped(r: impl std::io::Read, max: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    r.take(max + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max {
        return Err(eyre!("the location serves more than {max} bytes: refused"));
    }
    Ok(buf)
}

/// Reads a blob from an https URL or a `near-tx:` location (NEAR archival RPC); `file://` and http only
/// on the sandbox ([`location_allowed`]); at most [`FETCH_MAX`] bytes. Callers check hashes.
pub fn fetch(near: &Near, url: &str) -> Result<Vec<u8>> {
    fetch_tries(near, url, ARCHIVAL_TRIES)
}

/// [`fetch`] with at most `tries` reads of a `near-tx:` location: one, for a value a summary can do without
/// (`judge`'s rule strength), so that a busy archival RPC never holds the command up.
pub fn fetch_tries(near: &Near, url: &str, tries: usize) -> Result<Vec<u8>> {
    location_allowed(&near.network, url)?;
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;
    if let Some((hash, signer)) = fastfs::parse_location(url) {
        let rpc = match near.network.as_str() {
            "testnet" | "mainnet" => fastfs::archival_rpc(&near.network).to_string(),
            _ => near.rpc.clone(),
        };
        // A public archival RPC rate-limits and times out now and then: its answer is an error, not the
        // transaction. A few tries, further apart each time, before saying so.
        let mut last = String::new();
        let tries = tries.max(1);
        for attempt in 0..tries {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_secs(3 * attempt as u64));
            }
            let answer = client
                .post(&rpc)
                .json(&fastfs::tx_request(hash, signer))
                .send()
                .map_err(|e| eyre!("{}", e.without_url()))
                .and_then(|r| read_capped(r, FETCH_MAX))
                .and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| eyre!("not JSON: {e}")));
            match answer {
                Ok(v) if v["result"]["transaction"].is_object() => return fastfs::content_from_tx_response(&v).map_err(|e| eyre!(e)),
                Ok(v) => last = v.get("error").map(|e| e.to_string()).unwrap_or_else(|| "no transaction in its answer".into()),
                Err(e) => last = format!("{e:#}"),
            }
            // Once, after the archival RPC's first failure: the regular RPCs (`--rpc` and the fallbacks) keep the last
            // days' transactions, enough for a program created this week (a practice program's base witness).
            if attempt == 0 && tries > 1 {
                let regular = near.call("tx", serde_json::json!({"tx_hash": hash, "sender_account_id": signer, "wait_until": "INCLUDED_FINAL"}));
                if let Ok(r) = regular {
                    if let Ok(b) = fastfs::content_from_tx_response(&serde_json::json!({"result": r})) {
                        return Ok(b);
                    }
                }
            }
        }
        return Err(eyre!(
            "the archival RPC did not return transaction {hash} in {tries} tries (its last answer: {last}). A public RPC fails like this for a minute at a time: run the same command again"
        ));
    }
    if let Some(p) = url.strip_prefix("file://") {
        return read_capped(std::fs::File::open(p)?, FETCH_MAX);
    }
    let r = client.get(url).send().map_err(|e| eyre!("GET: {}", e.without_url()))?;
    if !r.status().is_success() {
        return Err(eyre!("GET: HTTP {}", r.status()));
    }
    read_capped(r, FETCH_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A location from the chain is opened only as near-tx: or https on a public network: never a local
    /// file, a share or plain http (SA-26).
    #[test]
    fn a_chain_location_cannot_open_local_files_on_a_public_network() {
        for net in ["testnet", "mainnet"] {
            assert!(location_allowed(net, "https://fastfs.example/antibody/manifests/x.json").is_ok());
            assert!(location_allowed(net, "near-tx:4W4CYBQucfJL8bawGXJZL4ufAKR2A2Se45hBvT7YmtfZ:acme.antibody.testnet").is_ok());
            for bad in ["file:///etc/passwd", "file:////host/share/m.json", "file://C:/Users/x/.ssh/id", "http://203.0.113.7/m.json", "/etc/passwd", "ftp://x/y", ""] {
                assert!(location_allowed(net, bad).is_err(), "{net}: {bad}");
            }
        }
        // The local sandbox keeps its file and http locations.
        assert!(location_allowed("sandbox", "file:///tmp/m.json").is_ok());
        assert!(location_allowed("sandbox", "http://127.0.0.1:8080/m.json").is_ok());
    }

    /// A location that serves more than the cap is refused, not buffered.
    #[test]
    fn a_fetched_body_is_capped() {
        use std::io::Read;
        assert_eq!(read_capped(&[1u8; 10][..], 10).unwrap().len(), 10);
        assert!(read_capped(&[1u8; 11][..], 10).is_err());
        assert!(read_capped(std::io::repeat(0).take(1 << 20), 1000).is_err());
    }

    /// The size a vended account's proof is refused at is the size the reveal would send.
    #[test]
    fn the_sealed_size_is_known_before_sealing() {
        let pk = hex::encode(sealed::x25519_public([7; 32]));
        for (envelope, report) in [(0, 0), (363, 0), (15_000, 64), (40_000, 1)] {
            let s = seal_with(&pk, "bounty.antibody.testnet/6", "ann.practice.antibody.testnet", vec![5; envelope], vec![6; report], None).unwrap();
            assert_eq!(s.blob.len(), sealed_len("bounty.antibody.testnet/6", "ann.practice.antibody.testnet", envelope, report), "{envelope} + {report}");
        }
    }
}

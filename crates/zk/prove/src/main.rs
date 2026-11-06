//! `antibody-zk-prove`: Groth16 proofs of disclosed verification runs (docs/zk.md), with SP1 6.10.0.
//!
//! The program is `crates/zk/guest`, built in SP1's docker image (its vkey is pinned by the T2 contract).
//! Each case directory holds one disclosed run, as `antibody zk prove` or `fixtures/zk/` writes it:
//! `request.json` (the exact bytes near-antibody sent), `content_key.hex`, `blob.bin` when the request
//! names its blob by hash, `round_manifest.bin` for a later round, and the base witness as `base.bin` or
//! `../base/<sha256>.bin` (the sha256 from `expected.json`).
//!
//! ```text
//! antibody-zk-prove [--mode network|cuda|execute] [--vkey 0x…] [--key-file <file>] <elf> <case dir>…
//! antibody-zk-prove balance [--key-file <file>]
//! ```
//!
//! - `network` (default): SP1's prover network. The requester key comes from `NETWORK_PRIVATE_KEY` or
//!   `--key-file` (default `.secrets/sp1-requester.key`); it is never printed. Writes `proof.bin` (the
//!   356 bytes a verifier reads), `public_values.bin` and `proof.json` (request id, times, cost) into the
//!   case directory.
//! - `cuda`: a local NVIDIA GPU (built with `--features cuda`).
//! - `execute`: SP1's executor only, no proof: the cycles and the public values.
//!
//! The proof is checked locally (`sp1-sdk`'s verifier, against SP1 6.10.0's Groth16 key) before it is
//! written. It claims no privacy: its input is a disclosed proof.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sp1_sdk::{Elf, HashableKey, ProveRequest, Prover, ProverClient, ProvingKey, SP1Stdin};

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn now_s() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

/// One disclosed run: the guest's five inputs.
struct Case {
    dir: PathBuf,
    request: Vec<u8>,
    base: Vec<u8>,
    round_manifest: Vec<u8>,
    blob: Vec<u8>,
    key: [u8; 32],
}

fn load(dir: &Path) -> Result<Case> {
    let read = |name: &str| std::fs::read(dir.join(name));
    let request = read("request.json").with_context(|| format!("{}: request.json", dir.display()))?;
    let key_hex = std::fs::read_to_string(dir.join("content_key.hex")).with_context(|| format!("{}: content_key.hex", dir.display()))?;
    let key: [u8; 32] = hex::decode(key_hex.trim().trim_start_matches("0x"))?.try_into().map_err(|_| anyhow!("the content key is 32 bytes"))?;
    let base = match read("base.bin") {
        Ok(b) => b,
        Err(_) => {
            let expected: Value = serde_json::from_slice(&read("expected.json").context("no base.bin and no expected.json")?)?;
            let sha = expected["base_sha256"].as_str().ok_or_else(|| anyhow!("expected.json has no base_sha256"))?;
            std::fs::read(dir.join("../base").join(format!("{sha}.bin"))).with_context(|| format!("the base witness {sha}"))?
        }
    };
    Ok(Case {
        dir: dir.to_path_buf(),
        request,
        base,
        round_manifest: read("round_manifest.bin").unwrap_or_default(),
        blob: read("blob.bin").unwrap_or_default(),
        key,
    })
}

fn stdin_of(c: &Case) -> SP1Stdin {
    let mut s = SP1Stdin::new();
    s.write_vec(c.request.clone());
    s.write_vec(c.base.clone());
    s.write_vec(c.round_manifest.clone());
    s.write_vec(c.blob.clone());
    s.write_vec(c.key.to_vec());
    s
}

struct Args {
    mode: String,
    vkey: Option<String>,
    key_file: PathBuf,
    timeout_s: u64,
    elf: PathBuf,
    cases: Vec<PathBuf>,
}

fn args() -> Result<(Option<String>, Args)> {
    let mut it = std::env::args().skip(1).peekable();
    let mut cmd = None;
    if it.peek().map(String::as_str) == Some("balance") {
        cmd = it.next();
    }
    let mut a = Args { mode: "network".into(), vkey: None, key_file: ".secrets/sp1-requester.key".into(), timeout_s: 3600, elf: PathBuf::new(), cases: vec![] };
    let mut rest = vec![];
    while let Some(x) = it.next() {
        match x.as_str() {
            "--mode" => a.mode = it.next().ok_or_else(|| anyhow!("--mode needs a value"))?,
            "--vkey" => a.vkey = it.next(),
            "--key-file" => a.key_file = it.next().ok_or_else(|| anyhow!("--key-file needs a path"))?.into(),
            "--timeout" => a.timeout_s = it.next().ok_or_else(|| anyhow!("--timeout needs seconds"))?.parse()?,
            _ => rest.push(PathBuf::from(x)),
        }
    }
    if cmd.is_none() {
        if rest.len() < 2 {
            bail!("usage: antibody-zk-prove [--mode network|cuda|execute] [--vkey 0x…] [--key-file <file>] <elf> <case dir>…");
        }
        a.elf = rest.remove(0);
        a.cases = rest;
    }
    Ok((cmd, a))
}

/// The requester key: `NETWORK_PRIVATE_KEY`, else the key file. Never printed.
fn requester_key(a: &Args) -> Result<String> {
    if let Ok(k) = std::env::var("NETWORK_PRIVATE_KEY") {
        if !k.trim().is_empty() {
            return Ok(k.trim().to_string());
        }
    }
    let k = std::fs::read_to_string(&a.key_file).with_context(|| format!("the requester key: set NETWORK_PRIVATE_KEY or --key-file ({})", a.key_file.display()))?;
    Ok(k.trim().to_string())
}

/// PROVE has 18 decimals.
fn prove_units(wei: &str) -> Value {
    match wei.parse::<u128>() {
        Ok(w) => json!(w as f64 / 1e18),
        Err(_) => Value::Null,
    }
}

fn write_outputs(c: &Case, proof_bytes: &[u8], public_values: &[u8], meta: &Value) -> Result<()> {
    std::fs::write(c.dir.join("proof.bin"), proof_bytes)?;
    std::fs::write(c.dir.join("public_values.bin"), public_values)?;
    std::fs::write(c.dir.join("proof.json"), serde_json::to_string_pretty(meta)? + "\n")?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let (cmd, a) = args()?;

    if cmd.as_deref() == Some("balance") {
        let client = ProverClient::builder().network().private_key(&requester_key(&a)?).build().await;
        let b = client.get_balance().await?;
        println!("{}", json!({"balance_prove": prove_units(&b.to_string()), "balance_wei": b.to_string()}));
        return Ok(());
    }

    let elf_bytes = std::fs::read(&a.elf).with_context(|| format!("the ELF {}", a.elf.display()))?;
    let elf_sha256 = sha256_hex(&elf_bytes);
    let cases = a.cases.iter().map(|d| load(d)).collect::<Result<Vec<_>>>()?;
    eprintln!("ELF {} sha256 {elf_sha256}, {} case(s), mode {}", a.elf.display(), cases.len(), a.mode);

    match a.mode.as_str() {
        "execute" => {
            let client = ProverClient::builder().cpu().build().await;
            for c in &cases {
                let t = Instant::now();
                let (pv, report) = client.execute(Elf::from(elf_bytes.clone()), stdin_of(c)).await.map_err(|e| anyhow!("{}: {e}", c.dir.display()))?;
                let pv = pv.to_vec();
                println!(
                    "{}",
                    json!({"case": c.dir.display().to_string(), "cycles": report.total_instruction_count(), "gas": report.gas(),
                           "public_values_sha256": sha256_hex(&pv), "public_values_bytes": pv.len(), "seconds": t.elapsed().as_secs_f64()})
                );
                std::fs::write(c.dir.join("public_values.bin"), &pv)?;
            }
            Ok(())
        }
        "network" => {
            let client = ProverClient::builder().network().private_key(&requester_key(&a)?).build().await;
            let t = Instant::now();
            let pk = client.setup(Elf::from(elf_bytes.clone())).await.map_err(|e| anyhow!("setup: {e}"))?;
            let vkey = pk.verifying_key().bytes32();
            let setup_s = t.elapsed().as_secs_f64();
            eprintln!("program vkey {vkey} (setup {setup_s:.1} s)");
            if let Some(want) = &a.vkey {
                if want.to_lowercase() != vkey.to_lowercase() {
                    bail!("the ELF's vkey {vkey} is not the pinned {want}: prove the docker build (docs/zk.md)");
                }
            }
            for c in &cases {
                let balance_before = client.get_balance().await.ok();
                let t0 = Instant::now();
                let requested_at = now_s();
                let id = client
                    .prove(&pk, stdin_of(c))
                    .groth16()
                    .timeout(Duration::from_secs(a.timeout_s))
                    .request()
                    .await
                    .map_err(|e| anyhow!("{}: request: {e}", c.dir.display()))?;
                let request_s = t0.elapsed().as_secs_f64();
                eprintln!("{}: request 0x{} ({request_s:.1} s)", c.dir.display(), hex::encode(id));
                let proof = client
                    .wait_proof(id, Some(Duration::from_secs(a.timeout_s)), None)
                    .await
                    .map_err(|e| anyhow!("{}: request 0x{}: {e}", c.dir.display(), hex::encode(id)))?;
                let total_s = t0.elapsed().as_secs_f64();
                client.verify(&proof, pk.verifying_key(), None).map_err(|e| anyhow!("{}: the proof does not verify locally: {e}", c.dir.display()))?;
                let details = client.get_proof_request(id).await.ok().flatten();
                let balance_after = client.get_balance().await.ok();
                let proof_bytes = proof.bytes();
                let pv = proof.public_values.to_vec();
                let spent = match (balance_before, balance_after) {
                    (Some(b), Some(a)) if b >= a => json!((b - a).to_string()),
                    _ => Value::Null,
                };
                let d = details.as_ref();
                let meta = json!({
                    "sp1": "6.10.0",
                    "mode": "groth16",
                    "prover": "SP1 prover network (mainnet, auction)",
                    "request_id": format!("0x{}", hex::encode(id)),
                    "explorer": format!("https://explorer.succinct.xyz/request/0x{}", hex::encode(id)),
                    "elf_sha256": elf_sha256,
                    "program_vkey": vkey,
                    "public_values_sha256": sha256_hex(&pv),
                    "proof_sha256": sha256_hex(&proof_bytes),
                    "proof_bytes": proof_bytes.len(),
                    "requested_at": requested_at,
                    "seconds": {"setup_once": setup_s, "request": request_s, "request_to_proof": total_s},
                    "network": d.map(|r| json!({
                        "cycles": r.cycles, "gas_used": r.gas_used, "gas_limit": r.gas_limit,
                        "created_at": r.created_at, "fulfilled_at": r.fulfilled_at,
                        "fulfiller": r.fulfiller.as_ref().map(|f| format!("0x{}", hex::encode(f))),
                        "base_fee_prove": r.base_fee.as_deref().map(prove_units),
                        "max_price_per_pgu": r.max_price_per_pgu, "gas_price": r.gas_price,
                        "deduction_prove": r.deduction_amount.as_deref().map(prove_units),
                        "refund_prove": r.refund_amount.as_deref().map(prove_units),
                        "settlement_tx": format!("0x{}", hex::encode(&r.tx_hash)),
                    })),
                    "balance_spent_prove": spent.as_str().map(prove_units),
                });
                write_outputs(c, &proof_bytes, &pv, &meta)?;
                println!("{}", json!({"case": c.dir.display().to_string(), "proof": meta}));
            }
            Ok(())
        }
        #[cfg(feature = "cuda")]
        "cuda" => {
            let client = ProverClient::builder().cuda().build().await;
            let pk = client.setup(Elf::from(elf_bytes.clone())).await.map_err(|e| anyhow!("setup: {e}"))?;
            let vkey = pk.verifying_key().bytes32();
            if let Some(want) = &a.vkey {
                if want.to_lowercase() != vkey.to_lowercase() {
                    bail!("the ELF's vkey {vkey} is not the pinned {want}");
                }
            }
            for c in &cases {
                let t0 = Instant::now();
                let proof = client.prove(&pk, stdin_of(c)).groth16().await.map_err(|e| anyhow!("{}: {e}", c.dir.display()))?;
                let total_s = t0.elapsed().as_secs_f64();
                client.verify(&proof, pk.verifying_key(), None).map_err(|e| anyhow!("does not verify: {e}"))?;
                let proof_bytes = proof.bytes();
                let pv = proof.public_values.to_vec();
                let meta = json!({"sp1": "6.10.0", "mode": "groth16", "prover": "local CUDA", "elf_sha256": elf_sha256, "program_vkey": vkey,
                                  "public_values_sha256": sha256_hex(&pv), "proof_sha256": sha256_hex(&proof_bytes), "proof_bytes": proof_bytes.len(),
                                  "seconds": {"prove": total_s}});
                write_outputs(c, &proof_bytes, &pv, &meta)?;
                println!("{}", json!({"case": c.dir.display().to_string(), "proof": meta}));
            }
            Ok(())
        }
        m => bail!("unknown mode {m} (network, cuda with --features cuda, execute)"),
    }
}

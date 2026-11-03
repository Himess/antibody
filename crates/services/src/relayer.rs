//! Relayer: submits the guardian's signed permits to each chain's `AntibodyAdapter` and records what
//! happened (the EVM transaction, `paused()` on every target, the proxy's upgrade). Relaying is
//! permissionless: the relayer pays its own EVM gas and is trusted for nothing, since the adapter checks
//! every signature, expiry and replay key. Without a key it only watches.

use std::{collections::BTreeMap, path::PathBuf};

use alloy_primitives::{keccak256, Address};
use antibody_cli::near::Near;
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::evm::{Evm, EvmSigner};

pub struct Relayer {
    pub near: Near,
    pub guardian: String,
    /// near-antibody: a release's implementation address is in the program's round releases.
    pub antibody: String,
    /// chain id → (RPC URL, explorer tx base URL)
    pub chains: BTreeMap<u64, (String, String)>,
    pub signer: Option<EvmSigner>,
    pub state_file: PathBuf,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn word(bytes: &[u8]) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[32 - bytes.len()..].copy_from_slice(bytes);
    w
}

fn hex32(s: &str) -> Result<[u8; 32]> {
    let b = hex::decode(s.trim_start_matches("0x"))?;
    b.try_into().map_err(|_| eyre!("expected 32 bytes"))
}

fn address(s: &str) -> Result<Address> {
    s.parse().map_err(|e| eyre!("address {s}: {e}"))
}

/// Calldata of `AntibodyAdapter.pause(PausePermit, v, r, s)` or `release(ReleasePermit, implementation,
/// v, r, s)` for a guardian permit. Every argument is static, so the tuple is encoded in place.
pub fn adapter_calldata(p: &Value, implementation: Option<Address>) -> Result<Vec<u8>> {
    let program = keccak256(p["program_key"].as_str().unwrap_or_default().as_bytes());
    let round = p["round"].as_u64().ok_or_else(|| eyre!("permit round"))?;
    let deadline = p["deadline"].as_u64().ok_or_else(|| eyre!("permit deadline"))?;
    let v = p["v"].as_u64().ok_or_else(|| eyre!("permit v"))? as u8;
    let (r, s) = (hex32(p["r"].as_str().unwrap_or_default())?, hex32(p["s"].as_str().unwrap_or_default())?);
    let mut out = Vec::new();
    if let Some(t) = p["body"]["Pause"].get("ticket") {
        out.extend(&keccak256(b"pause((bytes32,uint32,uint64,uint256),uint8,bytes32,bytes32)")[..4]);
        out.extend(program.0);
        out.extend(word(&round.to_be_bytes()));
        out.extend(word(&t.as_u64().unwrap_or_default().to_be_bytes()));
    } else if let Some(rel) = p["body"].get("Release") {
        let implementation = implementation.ok_or_else(|| eyre!("a release needs the implementation's address"))?;
        out.extend(&keccak256(b"release((bytes32,uint32,address,bytes32,uint256),address,uint8,bytes32,bytes32)")[..4]);
        out.extend(program.0);
        out.extend(word(&round.to_be_bytes()));
        out.extend(word(address(rel["proxy"].as_str().unwrap_or_default())?.as_slice()));
        out.extend(hex32(rel["code_hash"].as_str().unwrap_or_default())?);
        out.extend(word(&deadline.to_be_bytes()));
        out.extend(word(implementation.as_slice()));
        out.extend(word(&[v]));
        out.extend(r);
        out.extend(s);
        return Ok(out);
    } else {
        return Err(eyre!("unknown permit body {}", p["body"]));
    }
    out.extend(word(&deadline.to_be_bytes()));
    out.extend(word(&[v]));
    out.extend(r);
    out.extend(s);
    Ok(out)
}

impl Relayer {
    pub fn load_state(&self) -> BTreeMap<String, Value> {
        std::fs::read_to_string(&self.state_file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save_state(&self, st: &BTreeMap<String, Value>) -> Result<()> {
        if let Some(d) = self.state_file.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&self.state_file, serde_json::to_vec_pretty(st)?)?;
        Ok(())
    }

    /// The implementation a release names: the program's round releases on near-antibody.
    fn implementation(&self, p: &Value) -> Result<Address> {
        let key = p["program_key"].as_str().unwrap_or_default();
        let id: u32 = key.rsplit('/').next().unwrap_or_default().parse()?;
        let meta = self.near.view(&self.antibody, "get_program_meta", json!({"program_id": id}))?;
        let round = p["round"].as_u64().unwrap_or(0) as usize;
        let rel = &p["body"]["Release"];
        let found = meta["round_releases"][round.saturating_sub(1)]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["proxy"].as_str().map(|x| x.eq_ignore_ascii_case(rel["proxy"].as_str().unwrap_or_default())) == Some(true));
        address(found.and_then(|r| r["implementation"].as_str()).ok_or_else(|| eyre!("no implementation for this release"))?)
    }

    /// One pass: submit new permits, track receipts, and read the targets' state. Returns the state.
    pub fn tick(&self) -> Result<BTreeMap<String, Value>> {
        let mut st = self.load_state();
        let permits = self.near.view(&self.guardian, "list_permits", json!({"from": 0, "limit": 100}))?;
        for p in permits.as_array().into_iter().flatten() {
            let id = p["id"].as_u64().unwrap_or_default().to_string();
            let chain_id = p["chain_id"].as_u64().unwrap_or_default();
            let Some((rpc, explorer)) = self.chains.get(&chain_id) else { continue };
            let evm = Evm::new(rpc);
            let adapter = address(p["adapter"].as_str().unwrap_or_default())?;
            let kind = if p["body"].get("Pause").is_some() { "pause" } else { "release" };
            let mut e = st.get(&id).cloned().unwrap_or_else(|| {
                json!({"permit_id": p["id"], "chain_id": chain_id, "kind": kind, "program_key": p["program_key"], "round": p["round"],
                       "adapter": p["adapter"], "signed_ns": p["signed_ns"], "first_seen_ms": now_ms(), "status": "signed"})
            });
            if e["status"] == "signed" {
                if let Some(signer) = &self.signer {
                    let implementation = if kind == "release" { Some(self.implementation(p)?) } else { None };
                    let data = adapter_calldata(p, implementation)?;
                    // The node's estimate with a margin: gas schedules differ by chain and fork (Sepolia's
                    // Glamsterdam prices new state several times higher); never below 400k.
                    let estimate = evm.read("eth_estimateGas", json!([{"from": signer.address, "to": adapter, "data": format!("0x{}", hex::encode(&data))}, "latest"]));
                    let gas = estimate.ok().and_then(|v| crate::evm::hex_u64(&v).ok()).map(|g| g * 3 / 2).unwrap_or(0).max(400_000);
                    match signer.send(&evm, chain_id, adapter, data, gas) {
                        Ok(h) => {
                            e["status"] = "sent".into();
                            e["tx_hash"] = h.clone().into();
                            e["explorer"] = format!("{explorer}{h}").into();
                            e["sent_ms"] = now_ms().into();
                        }
                        Err(err) => e["last_error"] = err.to_string().into(),
                    }
                }
            }
            if e["status"] == "sent" {
                if let Some(r) = evm.receipt(e["tx_hash"].as_str().unwrap_or_default())? {
                    e["status"] = if r["status"] == "0x1" { "confirmed" } else { "reverted" }.into();
                    e["block"] = r["blockNumber"].clone();
                    e["gas_used"] = r["gasUsed"].clone();
                    e["confirmed_ms"] = now_ms().into();
                }
            }
            if kind == "pause" {
                let targets = evm.address_array(adapter, "pauseTargets()")?;
                let paused: Vec<bool> = targets.iter().filter_map(|t| evm.paused(&format!("{t:#x}")).ok()).collect();
                e["targets_paused"] = json!(paused);
            }
            st.insert(id, e);
        }
        self.save_state(&st)?;
        Ok(st)
    }

    pub fn run(&self, interval_ms: u64) -> Result<()> {
        loop {
            match self.tick() {
                Ok(st) => {
                    for (k, v) in &st {
                        if v["status"] == "confirmed" && v["logged"].is_null() {
                            println!("{}", json!({"relayer": "permit confirmed", "permit": k, "chain_id": v["chain_id"], "kind": v["kind"], "tx": v["tx_hash"]}));
                        }
                    }
                }
                Err(e) => eprintln!("relayer: {e:#}"),
            }
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_calldata_is_selector_and_seven_words() {
        let p = json!({"program_key": "antibody.testnet/0", "round": 1, "deadline": 1_800_000_600u64, "v": 27,
                       "r": "11".repeat(32), "s": "22".repeat(32), "body": {"Pause": {"ticket": 7}}});
        let d = adapter_calldata(&p, None).unwrap();
        assert_eq!(d.len(), 4 + 7 * 32);
        assert_eq!(&d[..4], &keccak256(b"pause((bytes32,uint32,uint64,uint256),uint8,bytes32,bytes32)")[..4]);
        assert_eq!(&d[4..36], keccak256(b"antibody.testnet/0").as_slice());
        assert_eq!(d[4 + 2 * 32 + 31], 7, "the ticket");
        assert_eq!(d[4 + 4 * 32 + 31], 27, "v");
        // `cast calldata "pause((bytes32,uint32,uint64,uint256),uint8,bytes32,bytes32)" …` for the same permit.
        assert_eq!(hex::encode(&d), "d32bc670bd3ba0664ef9d370ea11a50a1e951d457ef8e2e258a05210c9e1adbe5621fbac00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000007000000000000000000000000000000000000000000000000000000006b49d458000000000000000000000000000000000000000000000000000000000000001b11111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222222222222222222222222222222");
    }

    #[test]
    fn a_release_needs_its_implementation() {
        let p = json!({"program_key": "antibody.testnet/0", "round": 1, "deadline": 1u64, "v": 28, "r": "11".repeat(32), "s": "22".repeat(32),
                       "body": {"Release": {"proxy": "0x00000000000000000000000000000000000000aa", "code_hash": "cd".repeat(32)}}});
        assert!(adapter_calldata(&p, None).is_err());
        let d = adapter_calldata(&p, Some(Address::repeat_byte(0xbb))).unwrap();
        assert_eq!(d.len(), 4 + 9 * 32);
        // Words: the permit (5), the implementation, v, r, s.
        assert_eq!(&d[4 + 5 * 32 + 12..4 + 6 * 32], &[0xbb; 20]);
        assert_eq!(d[4 + 6 * 32 + 31], 28, "v");
        // `cast calldata "release((bytes32,uint32,address,bytes32,uint256),address,uint8,bytes32,bytes32)" …`.
        assert_eq!(hex::encode(&d), "48a55b2abd3ba0664ef9d370ea11a50a1e951d457ef8e2e258a05210c9e1adbe5621fbac000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000aacdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd0000000000000000000000000000000000000000000000000000000000000001000000000000000000000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb000000000000000000000000000000000000000000000000000000000000001c11111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222222222222222222222222222222");
    }
}

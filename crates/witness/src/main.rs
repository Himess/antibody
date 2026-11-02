//! antibody-witness: host-side witness builder, on-chain cross-checker and tamper generator.
//!
//!   antibody-witness replay --tx <hash> [--watch a,b] [--out witnesses/x] [--min-agree 3] [--recent]
//!   antibody-witness check  --witness x.bin --receipt x.receipt.json [--recent]
//!   antibody-witness tamper --witness x.bin --kind <kind> --out y.bin
//!
//! RPC URLs come from antibody/.env (ETH_RPC_*) plus public archive endpoints; never printed.

mod build;
mod fetchdb;
mod program;
mod rpc;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use alloy_primitives::{keccak256, Address, Bytes, Log, LogData, B256, U256};
use antibody_verifier::{Envelope, Payload};
use eyre::{eyre, Result};
use reth_trie::TrieAccount;
use serde_json::{json, Value};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn root_dir() -> PathBuf {
    // crates/witness -> antibody
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).cloned().unwrap_or_default();
    let env_file = root_dir().join(".env");
    let archive = rpc::providers(&env_file, false);
    let recent = args.iter().any(|a| a == "--recent");
    let quorum_pool = rpc::providers(&env_file, recent);
    match cmd.as_str() {
        "replay" => {
            let tx: B256 = arg(&args, "--tx").ok_or_else(|| eyre!("--tx required"))?.parse()?;
            let watch: Vec<Address> = arg(&args, "--watch")
                .map(|s| s.split(',').filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect())
                .unwrap_or_default();
            let min_agree: usize = arg(&args, "--min-agree").map(|s| s.parse().unwrap()).unwrap_or(3);
            let out = arg(&args, "--out").unwrap_or_else(|| format!("witnesses/{}", &format!("{tx:#x}")[..12]));
            let forge: Option<u64> = arg(&args, "--forge-timestamp-delta").map(|s| s.parse().unwrap());
            let built = build::build_replay(&archive, &quorum_pool, tx, &watch, min_agree, forge)?;
            std::fs::write(format!("{out}.bin"), &built.bytes)?;
            std::fs::write(format!("{out}.meta.json"), serde_json::to_string_pretty(&built.meta)?)?;
            std::fs::write(format!("{out}.native-receipt.json"), &built.receipt_json)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "witness": format!("{out}.bin"),
                    "bytes": built.meta.stats.witness_bytes,
                    "sha256": built.meta.stats.witness_sha256,
                    "block": built.meta.block_number,
                    "index": built.meta.tx_index,
                    "header_agreeing": built.meta.header_agreeing,
                    "fixpoint_passes": built.meta.fixpoint_passes,
                    "trace_and_strict_agree": built.meta.trace_and_strict_agree,
                    "total_ms": built.meta.total_ms,
                }))?
            );
        }
        "check" => check(&archive, &args)?,
        "calls" => calls(&args)?,
        "inspect" => {
            let wpath = arg(&args, "--witness").ok_or_else(|| eyre!("--witness"))?;
            let bytes = std::fs::read(&wpath)?;
            let env = Envelope::decode(&bytes).map_err(|e| eyre!(e))?;
            let (kind, codes, code_bytes, tries, anc) = match &env.payload {
                Payload::Replay(w) => ("replay", w.bytecodes.len(), w.bytecodes.iter().map(|b| b.original_bytes().len()).sum::<usize>(), w.parent_state.storage_tries.len(), w.ancestor_headers.len()),
                Payload::Bundle(w) => ("bundle", w.bytecodes.len(), w.bytecodes.iter().map(|b| b.original_bytes().len()).sum::<usize>(), w.state.storage_tries.len(), w.ancestor_headers.len()),
                Payload::Block(i) => ("block", i.bytecodes.len(), i.bytecodes.iter().map(|b| b.original_bytes().len()).sum::<usize>(), i.parent_state.storage_tries.len(), i.ancestor_headers.len()),
                Payload::Program(w) => ("program", w.bytecodes.len(), w.bytecodes.iter().map(|b| b.original_bytes().len()).sum::<usize>(), w.state.storage_tries.len(), w.ancestor_headers_rlp.len()),
            };
            println!("{}", json!({"witness": wpath, "bytes": bytes.len(), "payload": kind, "bytecodes": codes, "bytecode_bytes": code_bytes, "storage_tries": tries, "ancestor_headers": anc, "claimed_block_hash": env.claimed_block_hash}));
        }
        "block" => block_cmd(&archive, &quorum_pool, &args)?,
        "bundle" => {
            let file = arg(&args, "--bundle").ok_or_else(|| eyre!("--bundle file.json"))?;
            let bf: build::BundleFile = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
            let anchor = arg(&args, "--anchor").unwrap_or_else(|| "finalized".into());
            let m: u64 = if anchor == "finalized" {
                let v = quorum_pool[0].call("eth_getBlockByNumber", json!(["finalized", false]))?;
                rpc::hex_u64(&v["number"])?
            } else {
                anchor.parse()?
            };
            let salt: B256 = match arg(&args, "--salt") {
                Some(s) => s.parse()?,
                None => keccak256(format!("antibody-demo-salt/{}/{}", std::process::id(), m).as_bytes()),
            };
            let bundle = build::parse_bundle(&bf, salt)?;
            let out = arg(&args, "--out").unwrap_or_else(|| format!("witnesses/bundle-{m}"));
            let (_env, bytes, meta, receipt) = build::build_bundle(&archive, &quorum_pool, m, bundle, bf.protected.clone(), 3)?;
            std::fs::write(format!("{out}.bin"), &bytes)?;
            std::fs::write(format!("{out}.meta.json"), serde_json::to_string_pretty(&meta)?)?;
            std::fs::write(format!("{out}.native-receipt.json"), &receipt)?;
            println!("{}", serde_json::to_string_pretty(&json!({"witness": format!("{out}.bin"), "bytes": bytes.len(),
                "anchor": m, "senders": meta.senders, "fixpoint_passes": meta.fixpoint_passes, "total_ms": meta.total_ms}))?);
        }

        "header" => {
            // Recomputes keccak(RLP(header)) locally for a block (e.g. a Glamsterdam-era Sepolia
            // header) and prints it next to the provider-reported hash.
            let url = arg(&args, "--rpc").ok_or_else(|| eyre!("--rpc url"))?;
            let p = rpc::Provider::new("rpc", &url);
            let tag = arg(&args, "--number").unwrap_or_else(|| "finalized".into());
            let tag = if tag.chars().all(|c| c.is_ascii_digit()) { format!("{:#x}", tag.parse::<u64>()?) } else { tag };
            let v = p.call("eth_getBlockByNumber", json!([tag, false]))?;
            let h: alloy_rpc_types::Header = serde_json::from_value(v)?;
            let reported = h.hash;
            let header: alloy_consensus::Header = h.inner.clone();
            println!("{}", json!({"number": header.number, "reported": reported, "recomputed": header.hash_slow(),
                "match": header.hash_slow() == reported, "block_access_list_hash": header.block_access_list_hash,
                "slot_number": header.slot_number, "rlp_bytes": alloy_rlp::encode(&header).len()}));
        }
        "manifest" => {
            // antibody.toml -> canonical JSON manifest; prints its sha256 (the committed hash).
            let src = arg(&args, "--toml").ok_or_else(|| eyre!("--toml antibody.toml"))?;
            let out = arg(&args, "--out").ok_or_else(|| eyre!("--out manifest.json"))?;
            let (bytes, hash) = program::compile_manifest(&std::fs::read_to_string(&src)?)?;
            std::fs::write(&out, &bytes)?;
            println!("{}", json!({"manifest": out, "bytes": bytes.len(), "sha256": format!("{hash:#x}")}));
        }
        "program" => {
            // Program-mode witness: --rpc URL[,URL] --manifest m.json --invariants-code hexfile
            //   --bundle b.json --salt 0x.. --out prefix [--min-agree N]
            let urls = arg(&args, "--rpc").ok_or_else(|| eyre!("--rpc url[,url]"))?;
            let pool: rpc::Pool = urls
                .split(',')
                .enumerate()
                .map(|(i, u)| std::sync::Arc::new(rpc::Provider::new(&format!("rpc{i}"), u)))
                .collect();
            let manifest_json = std::fs::read(arg(&args, "--manifest").ok_or_else(|| eyre!("--manifest"))?)?;
            let code_hex = std::fs::read_to_string(arg(&args, "--invariants-code").ok_or_else(|| eyre!("--invariants-code"))?)?;
            let code: alloy_primitives::Bytes = hex::decode(code_hex.trim().trim_start_matches("0x"))?.into();
            let file = arg(&args, "--bundle").ok_or_else(|| eyre!("--bundle file.json"))?;
            let bf: build::BundleFile = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
            let salt: B256 = arg(&args, "--salt").ok_or_else(|| eyre!("--salt 0x.."))?.parse()?;
            let bundle = build::parse_bundle(&bf, salt)?;
            let min_agree: usize = arg(&args, "--min-agree").map(|s| s.parse().unwrap()).unwrap_or(1);
            let out = arg(&args, "--out").ok_or_else(|| eyre!("--out prefix"))?;
            let (bytes, meta, receipt) = program::build_program(&pool, &manifest_json, &code, bundle, min_agree)?;
            std::fs::write(format!("{out}.witness.bin"), &bytes)?;
            std::fs::write(format!("{out}.meta.json"), serde_json::to_string_pretty(&meta)?)?;
            println!("{}", serde_json::to_string_pretty(&json!({
                "witness": format!("{out}.witness.bin"), "bytes": bytes.len(), "sha256": meta.stats.witness_sha256,
                "anchor": meta.anchor_block, "fixpoint_passes": meta.fixpoint_passes, "trace_verdict": meta.trace_verdict,
                "kind": receipt.kind, "broken_mask": receipt.broken_mask,
                "broken": receipt.broken.iter().map(|f| format!("{} @ {}.{}", f.name, f.target_name.clone().unwrap_or_else(|| f.target.to_string()), f.selector)).collect::<Vec<_>>(),
                "impact_usd": receipt.impact.as_ref().map(|i| i.impact_usd.clone()), "evaluations": receipt.evaluations,
                "reason": receipt.reason, "total_ms": meta.total_ms
            }))?);
        }
        "tamper" => tamper(&args)?,
        _ => {
            eprintln!("usage: antibody-witness <replay|check|tamper> ...");
            std::process::exit(2);
        }
    }
    Ok(())
}

/// Compares a verifier receipt (with full diff) against the chain: receipt status/gas/logs from
/// several providers, and every written slot/balance against block-N proofs. Mismatches are
/// explained only if a later transaction of the same block wrote the same key.
fn check(archive: &rpc::Pool, args: &[String]) -> Result<()> {
    let wpath = arg(args, "--witness").ok_or_else(|| eyre!("--witness"))?;
    let rpath = arg(args, "--receipt").ok_or_else(|| eyre!("--receipt"))?;
    let env = Envelope::decode(&std::fs::read(&wpath)?).map_err(|e| eyre!(e))?;
    let receipt: Value = serde_json::from_str(&std::fs::read_to_string(&rpath)?)?;
    let Payload::Replay(w) = &env.payload else { return Err(eyre!("replay witness expected")) };
    let block = <reth_ethereum_primitives::Block as alloy_rlp::Decodable>::decode(&mut w.block_rlp.as_ref())?;
    let n = block.header.number;
    let t = &receipt["target"];
    let tx_hash: B256 = serde_json::from_value(t["tx_hash"].clone())?;

    // 1. On-chain receipt from every provider.
    let mut receipt_checks = Vec::new();
    for p in archive.iter() {
        let r = match p.call("eth_getTransactionReceipt", json!([tx_hash])) {
            Ok(r) => r,
            Err(e) => {
                receipt_checks.push(json!({"source": p.label, "error": e.to_string()}));
                continue;
            }
        };
        let logs: Vec<Log> = r["logs"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|l| {
                let address: Address = serde_json::from_value(l["address"].clone()).unwrap();
                let topics: Vec<B256> = serde_json::from_value(l["topics"].clone()).unwrap();
                let data: Bytes = serde_json::from_value(l["data"].clone()).unwrap();
                Log { address, data: LogData::new_unchecked(topics, data) }
            })
            .collect();
        let mut enc = Vec::new();
        alloy_rlp::Encodable::encode(&logs, &mut enc);
        let onchain_logs = keccak256(&enc);
        let status_ok = r["status"].as_str() == Some("0x1");
        let gas_used = rpc::hex_u64(&r["gasUsed"]).unwrap_or_default();
        let cumulative = rpc::hex_u64(&r["cumulativeGasUsed"]).unwrap_or_default();
        receipt_checks.push(json!({
            "source": p.label,
            "status_match": status_ok == t["success"].as_bool().unwrap_or(false),
            "gas_used_match": gas_used == t["gas_used"].as_u64().unwrap_or(0),
            "cumulative_gas_match": cumulative == t["cumulative_gas_used"].as_u64().unwrap_or(0),
            "logs_match": onchain_logs == serde_json::from_value::<B256>(t["logs_keccak"].clone())?,
            "logs": logs.len(),
            "onchain_gas_used": gas_used,
        }));
    }

    // 2. Post-state of every key the target wrote, from block-N proofs.
    let mut slots_by_addr: BTreeMap<Address, Vec<(B256, B256)>> = BTreeMap::new();
    for s in t["diff"]["slots"].as_array().cloned().unwrap_or_default() {
        let a: Address = serde_json::from_value(s["address"].clone())?;
        slots_by_addr.entry(a).or_default().push((serde_json::from_value(s["slot"].clone())?, serde_json::from_value(s["after"].clone())?));
    }
    let mut accounts_after: BTreeMap<Address, (U256, u64)> = BTreeMap::new();
    for a in t["diff"]["accounts"].as_array().cloned().unwrap_or_default() {
        let addr: Address = serde_json::from_value(a["address"].clone())?;
        let bal: U256 = a["balance_after"].as_str().unwrap().parse()?;
        accounts_after.insert(addr, (bal, a["nonce_after"].as_u64().unwrap()));
        slots_by_addr.entry(addr).or_default();
    }
    let mut access: BTreeMap<Address, BTreeSet<U256>> = BTreeMap::new();
    for (a, v) in &slots_by_addr {
        access.insert(*a, v.iter().map(|(s, _)| U256::from_be_bytes(s.0)).collect());
    }
    let (post, post_q) = build::fetch_proofs_quorum(archive, n, &access, 2)?;
    let mut mismatched_slots: Vec<(Address, B256)> = Vec::new();
    let mut mismatched_accounts: Vec<Address> = Vec::new();
    let (mut slot_ok, mut acct_ok) = (0usize, 0usize);
    for (a, list) in &slots_by_addr {
        let p = &post[a];
        for (slot, after) in list {
            let v = p.storage_proofs.iter().find(|sp| sp.key == *slot).map(|sp| sp.value).unwrap_or_default();
            if B256::from(v) == *after {
                slot_ok += 1;
            } else {
                mismatched_slots.push((*a, *slot));
            }
        }
        if let Some((bal, nonce)) = accounts_after.get(a) {
            let (pb, pn) = p.info.map(|i| (i.balance, i.nonce)).unwrap_or_default();
            if pb == *bal && pn == *nonce {
                acct_ok += 1;
            } else {
                mismatched_accounts.push(*a);
            }
        }
    }

    // 3. Explain mismatches: which later transactions of block N wrote the same keys?
    let mut explained = 0usize;
    let mut unexplained = Vec::new();
    if !mismatched_slots.is_empty() || !mismatched_accounts.is_empty() {
        let writers = later_writers(archive, &block, w.target_index)?;
        for (a, s) in &mismatched_slots {
            match writers.slots.get(&(*a, *s)) {
                Some(j) => explained += 1 + 0 * j,
                None => unexplained.push(format!("slot {a}[{s}]")),
            }
        }
        for a in &mismatched_accounts {
            match writers.accounts.get(a) {
                Some(_) => explained += 1,
                None => unexplained.push(format!("account {a}")),
            }
        }
    }
    let total_slots: usize = slots_by_addr.values().map(|v| v.len()).sum();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "tx": tx_hash, "block": n, "index": w.target_index,
            "receipt_vs_chain": receipt_checks,
            "post_state": {
                "slots_written": total_slots, "slots_equal_block_n_proofs": slot_ok,
                "accounts_changed": accounts_after.len(), "accounts_equal_block_n_proofs": acct_ok,
                "mismatches_explained_by_later_txs_in_block": explained,
                "unexplained": unexplained,
                "post_proof_quorum": { "unanimous": post_q.unanimous, "accounts": post_q.accounts, "disagreements": post_q.disagreements },
            }
        }))?
    );
    Ok(())
}

#[derive(Default)]
struct Writers {
    slots: BTreeMap<(Address, B256), usize>,
    accounts: BTreeMap<Address, usize>,
}

/// Executes the whole block on the host (seeded from an execution witness when available) and
/// records, per transaction after `target`, which accounts/slots it changed.
fn later_writers(archive: &rpc::Pool, block: &reth_ethereum_primitives::Block, target: u32) -> Result<Writers> {
    use alloy_consensus::transaction::{Recovered, SignerRecoverable};
    use alloy_evm::block::{BlockExecutor, StateChangeSource};
    use reth_evm::ConfigureEvm;
    use revm::database::{State, WrapDatabaseRef};

    let n = block.header.number;
    let parent = build::fetch_header(archive, n - 1)?;
    let seed = match build::fetch_execution_witness(archive, n) {
        Some((_, w, _)) => build::seed_from_execution_witness(&w, parent.state_root),
        None => fetchdb::Seed::default(),
    };
    let db = fetchdb::FetchingDb::new(archive.clone(), n - 1, seed);
    let spec = Arc::new(rsp_primitives::chain_spec::mainnet().map_err(|e| eyre!("{e}"))?);
    revm::install_crypto(rsp_client_executor::custom::CustomCrypto::default());
    let cfg = reth_evm_ethereum::EthEvmConfig::new_with_evm_factory(spec, rsp_client_executor::custom::CustomEvmFactory::new(None));
    let sealed = reth_primitives_traits::SealedBlock::seal_slow(block.clone());
    let mut state = State::builder().with_database(WrapDatabaseRef(&db)).with_bundle_update().build();
    let writes: Arc<Mutex<Writers>> = Arc::new(Mutex::new(Writers::default()));
    let wr = writes.clone();
    let hook = move |src: StateChangeSource, st: &revm::state::EvmState| {
        if let StateChangeSource::Transaction(j) = src {
            if j as u32 <= target {
                return;
            }
            let mut w = wr.lock().unwrap();
            for (a, acc) in st.iter() {
                if !acc.is_touched() {
                    continue;
                }
                if acc.original_info.balance != acc.info.balance || acc.original_info.nonce != acc.info.nonce {
                    w.accounts.entry(*a).or_insert(j);
                }
                for (s, v) in acc.storage.iter() {
                    if v.is_changed() {
                        w.slots.entry((*a, B256::from(*s))).or_insert(j);
                    }
                }
            }
        }
    };
    let mut ex = cfg.executor_for_block(&mut state, &sealed).map_err(|e| eyre!("{e}"))?;
    ex.set_state_hook(Some(Box::new(hook)));
    ex.apply_pre_execution_changes().map_err(|e| eyre!("{e}"))?;
    for tx in block.body.transactions.iter() {
        let s = tx.recover_signer().map_err(|e| eyre!("{e}"))?;
        ex.execute_transaction(Recovered::new_unchecked(tx, s)).map_err(|e| eyre!("{e}"))?;
    }
    drop(ex);
    let w = std::mem::take(&mut *writes.lock().unwrap());
    Ok(w)
}

/// Counts EVM call frames of the target transaction, executing from the witness alone (no RPC).
#[derive(Debug, Default, Clone, Copy)]
struct CallCounter {
    calls: u64,
    creates: u64,
    depth: u64,
    max_depth: u64,
}

impl<CTX> revm::Inspector<CTX> for CallCounter {
    fn call(&mut self, _c: &mut CTX, _i: &mut revm::interpreter::CallInputs) -> Option<revm::interpreter::CallOutcome> {
        self.calls += 1;
        self.depth += 1;
        self.max_depth = self.max_depth.max(self.depth);
        None
    }
    fn call_end(&mut self, _c: &mut CTX, _i: &revm::interpreter::CallInputs, _o: &mut revm::interpreter::CallOutcome) {
        self.depth -= 1;
    }
    fn create(&mut self, _c: &mut CTX, _i: &mut revm::interpreter::CreateInputs) -> Option<revm::interpreter::CreateOutcome> {
        self.creates += 1;
        self.depth += 1;
        self.max_depth = self.max_depth.max(self.depth);
        None
    }
    fn create_end(&mut self, _c: &mut CTX, _i: &revm::interpreter::CreateInputs, _o: &mut revm::interpreter::CreateOutcome) {
        self.depth -= 1;
    }
}

fn calls(args: &[String]) -> Result<()> {
    use alloy_consensus::transaction::{Recovered, SignerRecoverable};
    use alloy_evm::{block::BlockExecutor, Evm};
    use antibody_verifier::db::StrictDb;
    use itertools::Itertools;
    use reth_evm::ConfigureEvm;
    use reth_primitives_traits::SealedHeader;
    use revm::database::{State, WrapDatabaseRef};

    let wpath = arg(args, "--witness").ok_or_else(|| eyre!("--witness"))?;
    let env = Envelope::decode(&std::fs::read(&wpath)?).map_err(|e| eyre!(e))?;
    let Payload::Replay(w) = &env.payload else { return Err(eyre!("replay witness expected")) };
    let block = <reth_ethereum_primitives::Block as alloy_rlp::Decodable>::decode(&mut w.block_rlp.as_ref())?;
    let sealed: Vec<SealedHeader> = std::iter::once(SealedHeader::seal_slow(block.header.clone()))
        .chain(w.ancestor_headers.iter().map(|h| SealedHeader::seal_slow(h.clone())))
        .collect();
    let mut hashes = alloy_primitives::map::HashMap::default();
    for (c, p) in sealed.iter().tuple_windows() {
        hashes.insert(p.number, c.parent_hash);
    }
    let codes = w.bytecodes.iter().map(|c| (c.hash_slow(), c)).collect();
    let db = StrictDb::new(&w.parent_state, hashes, codes);
    let spec = Arc::new(rsp_primitives::chain_spec::mainnet().map_err(|e| eyre!("{e}"))?);
    revm::install_crypto(rsp_client_executor::custom::CustomCrypto::default());
    let cfg = reth_evm_ethereum::EthEvmConfig::new_with_evm_factory(spec, rsp_client_executor::custom::CustomEvmFactory::new(None));
    let sealed_block = reth_primitives_traits::SealedBlock::seal_slow(block.clone());
    let mut state = State::builder().with_database(WrapDatabaseRef(&db)).with_bundle_update().build();
    let evm_env = cfg.evm_env(&block.header).map_err(|e| eyre!("{e}"))?;
    let evm = cfg.evm_with_env_and_inspector(&mut state, evm_env, CallCounter::default());
    let ctx = cfg.context_for_block(&sealed_block).map_err(|e| eyre!("{e}"))?;
    let mut ex = cfg.create_executor(evm, ctx);
    ex.apply_pre_execution_changes().map_err(|e| eyre!("{e}"))?;
    let target = w.target_index as usize;
    let mut before = CallCounter::default();
    for (i, tx) in block.body.transactions.iter().enumerate().take(target + 1) {
        if i == target {
            before = *ex.evm().inspector();
        }
        let s = tx.recover_signer().map_err(|e| eyre!("{e}"))?;
        ex.execute_transaction(Recovered::new_unchecked(tx, s)).map_err(|e| eyre!("{e}"))?;
    }
    let after = *ex.evm().inspector();
    println!(
        "{}",
        json!({
            "witness": wpath,
            "target_index": target,
            "call_frames_including_top_level": after.calls - before.calls,
            "internal_calls": (after.calls - before.calls).saturating_sub(1),
            "creates": after.creates - before.creates,
            "max_depth": after.max_depth,
        })
    );
    Ok(())
}

/// Full-block witness for rsp's client executor (stateless validation of block N), built from a
/// `debug_executionWitness` (state nodes + codes) with the header hash agreed by the quorum.
fn block_cmd(archive: &rpc::Pool, quorum_pool: &rpc::Pool, args: &[String]) -> Result<()> {
    use antibody_verifier::{QuorumVote, WITNESS_VERSION};
    use rsp_client_executor::io::ClientExecutorInput;
    let n: u64 = arg(args, "--number").ok_or_else(|| eyre!("--number"))?.parse()?;
    let out = arg(args, "--out").unwrap_or_else(|| format!("witnesses/block-{n}"));
    let t0 = std::time::Instant::now();
    let q = build::fetch_block_quorum(quorum_pool, n, 3)?;
    let parent = build::fetch_header(archive, n - 1)?;
    // Source 1: debug_executionWitness (single call). Source 2 (fallback, or --proofs): rsp-style
    // transition proofs from eth_getProof at N-1 (used + modified keys) and N (modified keys).
    let force_proofs = args.iter().any(|a| a == "--proofs");
    let ew = if force_proofs { None } else { build::fetch_execution_witness(archive, n) };
    let (state, bytecodes, oldest, src, ew_ms, extra) = match ew {
        Some((src, w, ms)) => {
            let state = rsp_mpt::EthereumState::from_execution_witness(&w, parent.state_root);
            let mut oldest = n - 1;
            for h in &w.headers {
                if let Ok(hdr) = <alloy_consensus::Header as alloy_rlp::Decodable>::decode(&mut h.as_ref()) {
                    oldest = oldest.min(hdr.number);
                }
            }
            let codes: Vec<revm::state::Bytecode> = w.codes.iter().map(|c| revm::state::Bytecode::new_raw(c.clone())).collect();
            (state, codes, oldest, src, ms, json!({"state_nodes": w.state.len(), "codes": w.codes.len()}))
        }
        None => {
            let (state, codes, oldest, meta) = transition_witness(archive, &q.block, &parent)?;
            (state, codes, oldest, "eth_getProof transition proofs".to_string(), 0, meta)
        }
    };
    let mut ancestors = vec![parent];
    for k in (oldest..n - 1).rev() {
        ancestors.push(build::fetch_header(archive, k)?);
    }
    let input = ClientExecutorInput {
        current_block: q.block.clone(),
        ancestor_headers: ancestors,
        parent_state: state,
        bytecodes,
        genesis: rsp_primitives::genesis::Genesis::Mainnet,
        custom_beneficiary: None,
        opcode_tracking: false,
    };
    let env = Envelope {
        version: WITNESS_VERSION,
        chain_id: 1,
        claimed_block_hash: q.hash,
        quorum: q.votes.iter().filter_map(|v| v.recomputed_hash.map(|h| QuorumVote { source: v.source.clone(), block_hash: h })).collect(),
        payload: Payload::Block(input),
    };
    let bytes = env.encode();
    std::fs::write(format!("{out}.bin"), &bytes)?;
    println!(
        "{}",
        json!({"witness": format!("{out}.bin"), "bytes": bytes.len(), "txs": q.block.body.transactions.len(),
               "source": src, "execution_witness_ms": ew_ms, "header_agreeing": q.agreeing, "details": extra,
               "total_ms": t0.elapsed().as_millis() as u64})
    );
    Ok(())
}

/// rsp-style full-block witness from eth_getProof only: trace the block on a fetching DB, then fetch
/// proofs at N-1 for every used or modified key and at N for modified keys (needed to recompute the
/// post-state root when trie nodes are deleted), and build `EthereumState::from_transition_proofs`.
fn transition_witness(
    archive: &rpc::Pool,
    block: &reth_ethereum_primitives::Block,
    parent: &alloy_consensus::Header,
) -> Result<(rsp_mpt::EthereumState, Vec<revm::state::Bytecode>, u64, Value)> {
    use reth_evm::execute::{BasicBlockExecutor, Executor};
    use reth_primitives_traits::Block as _;
    use revm::database::CacheDB;
    let n = block.header.number;
    let t0 = std::time::Instant::now();
    let fdb = fetchdb::FetchingDb::new(archive.clone(), n - 1, fetchdb::Seed::default());
    let spec = Arc::new(rsp_primitives::chain_spec::mainnet().map_err(|e| eyre!("{e}"))?);
    revm::install_crypto(rsp_client_executor::custom::CustomCrypto::default());
    let cfg = reth_evm_ethereum::EthEvmConfig::new_with_evm_factory(spec, rsp_client_executor::custom::CustomEvmFactory::new(None));
    let recovered = block.clone().try_into_recovered().map_err(|_| eyre!("signer recovery failed"))?;
    let output = BasicBlockExecutor::new(cfg, CacheDB::new(&fdb)).execute(&recovered).map_err(|e| eyre!("{e}"))?;
    let trace_ms = t0.elapsed().as_millis() as u64;
    let rec = fdb.rec.borrow().clone();
    let mut before: BTreeMap<Address, BTreeSet<U256>> = BTreeMap::new();
    for a in &rec.accounts {
        before.entry(*a).or_default();
    }
    for (a, s) in &rec.slots {
        before.entry(*a).or_default().extend(s.iter().copied());
    }
    let mut after: BTreeMap<Address, BTreeSet<U256>> = BTreeMap::new();
    let mut modified_slots = 0usize;
    for (a, acc) in output.state.state.iter() {
        let keys: Vec<U256> = acc.storage.keys().copied().collect();
        modified_slots += keys.len();
        before.entry(*a).or_default().extend(keys.iter().copied());
        after.entry(*a).or_default().extend(keys);
    }
    for a in before.keys() {
        after.entry(*a).or_default();
    }
    let (pre, q1) = build::fetch_proofs_quorum(archive, n - 1, &before, 2)?;
    let (post, q2) = build::fetch_proofs_quorum(archive, n, &after, 2)?;
    let state = rsp_mpt::EthereumState::from_transition_proofs(parent.state_root, &pre, &post).map_err(|e| eyre!("{e}"))?;
    let codes: Vec<revm::state::Bytecode> = rec.codes.values().cloned().collect();
    let oldest = rec.block_numbers.iter().next().copied().unwrap_or(n - 1).min(n - 1);
    let meta = json!({
        "trace_ms": trace_ms, "trace_rpc_reads": rec.rpc_reads, "accounts": before.len(),
        "slots_used_or_modified": before.values().map(|s| s.len()).sum::<usize>(),
        "accounts_modified": output.state.state.len(), "slots_modified": modified_slots,
        "pre_proofs_unanimous": q1.unanimous, "post_proofs_unanimous": q2.unanimous,
        "proof_disagreements": q1.disagreements.len() + q2.disagreements.len(),
        "gas_used": output.result.gas_used,
    });
    Ok((state, codes, oldest, meta))
}

/// Negative-test generator.
fn tamper(args: &[String]) -> Result<()> {
    let wpath = arg(args, "--witness").ok_or_else(|| eyre!("--witness"))?;
    let kind = arg(args, "--kind").ok_or_else(|| eyre!("--kind"))?;
    let out = arg(args, "--out").ok_or_else(|| eyre!("--out"))?;
    let mut env = Envelope::decode(&std::fs::read(&wpath)?).map_err(|e| eyre!(e))?;
    let Payload::Replay(w) = &mut env.payload else { return Err(eyre!("replay witness expected")) };
    let note: String;
    match kind.as_str() {
        "slot-value" | "slot-value-propagated" => {
            // First storage trie that reveals a leaf: bump the value by one.
            let mut target: Option<(B256, Vec<u8>, U256)> = None;
            for (ha, trie) in w.parent_state.storage_tries.iter() {
                let mut found: Option<(Vec<u8>, U256)> = None;
                trie.for_each_leaves(|k, mut v| {
                    if found.is_none() {
                        let val: U256 = alloy_rlp::Decodable::decode(&mut v).unwrap_or_default();
                        found = Some((k.to_vec(), val));
                    }
                });
                if let Some((k, v)) = found {
                    target = Some((*ha, k, v));
                    break;
                }
            }
            let (ha, key, val) = target.ok_or_else(|| eyre!("no revealed storage leaf"))?;
            let trie = w.parent_state.storage_tries.get_mut(&ha).unwrap();
            trie.insert_rlp(&key, val + U256::from(1)).map_err(|e| eyre!("{e:?}"))?;
            let new_root = trie.hash();
            note = format!("storage leaf {} of account-hash {ha}: {val} -> {}", hex::encode(&key), val + U256::from(1));
            if kind == "slot-value-propagated" {
                let mut acc: TrieAccount = w
                    .parent_state
                    .state_trie
                    .get_rlp(ha.as_slice())
                    .map_err(|e| eyre!("{e:?}"))?
                    .ok_or_else(|| eyre!("account leaf missing"))?;
                acc.storage_root = new_root;
                w.parent_state.state_trie.insert_rlp(ha.as_slice(), acc).map_err(|e| eyre!("{e:?}"))?;
            }
        }
        "balance" => {
            let mut found: Option<(Vec<u8>, TrieAccount)> = None;
            w.parent_state.state_trie.for_each_leaves(|k, mut v| {
                if found.is_none() {
                    if let Ok(a) = <TrieAccount as alloy_rlp::Decodable>::decode(&mut v) {
                        found = Some((k.to_vec(), a));
                    }
                }
            });
            let (k, mut a) = found.ok_or_else(|| eyre!("no account leaf"))?;
            note = format!("account-hash 0x{} balance {} -> {}", hex::encode(&k), a.balance, a.balance + U256::from(10u64.pow(18)));
            a.balance += U256::from(10u64.pow(18));
            w.parent_state.state_trie.insert_rlp(&k, a).map_err(|e| eyre!("{e:?}"))?;
        }
        "header-timestamp" | "header-timestamp-rehashed" => {
            let mut b = <reth_ethereum_primitives::Block as alloy_rlp::Decodable>::decode(&mut w.block_rlp.as_ref())?;
            b.header.timestamp += 1;
            w.block_rlp = alloy_rlp::encode(&b).into();
            note = format!("timestamp {} -> {}", b.header.timestamp - 1, b.header.timestamp);
            if kind == "header-timestamp-rehashed" {
                env.claimed_block_hash = b.header.hash_slow();
            }
        }
        "ancestor" => {
            let k = if w.ancestor_headers.len() > 1 { 1 } else { 0 };
            let h = w.ancestor_headers.get_mut(k).ok_or_else(|| eyre!("no ancestors"))?;
            h.extra_data = Bytes::from_static(b"tampered");
            note = format!("ancestor header #{k} (N-{}) extra_data changed", k + 1);
        }
        "drop-tx" => {
            let mut b = <reth_ethereum_primitives::Block as alloy_rlp::Decodable>::decode(&mut w.block_rlp.as_ref())?;
            b.body.transactions.pop();
            w.block_rlp = alloy_rlp::encode(&b).into();
            note = "last transaction of the block removed".into();
        }
        "code" => {
            let c = w.bytecodes.iter_mut().max_by_key(|c| c.original_bytes().len()).ok_or_else(|| eyre!("no code"))?;
            let mut bytes = c.original_bytes().to_vec();
            let last = bytes.len() - 1;
            bytes[last] ^= 0x01;
            *c = revm::state::Bytecode::new_raw(bytes.into());
            note = "one byte flipped in the largest bytecode".into();
        }
        "drop-storage-trie" => {
            let victim = w
                .parent_state
                .storage_tries
                .iter()
                .find(|(_, t)| t.hash() != reth_trie::EMPTY_ROOT_HASH)
                .map(|(k, _)| *k)
                .ok_or_else(|| eyre!("no non-empty storage trie"))?;
            w.parent_state.storage_tries.remove(&victim);
            note = format!("storage trie of account-hash {victim} removed");
        }
        other => return Err(eyre!("unknown tamper kind {other}")),
    }
    std::fs::write(&out, env.encode())?;
    println!("{}", json!({"kind": kind, "out": out, "note": note}));
    Ok(())
}

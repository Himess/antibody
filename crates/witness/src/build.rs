//! Witness construction: header quorum, trace via the verifier's own execution path, quorum
//! proofs for the recorded access set, and the strict-verification fixpoint loop.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Instant,
};

use alloy_consensus::Header;
use alloy_primitives::{keccak256, map::HashMap, Address, Bytes, B256, U256};
use alloy_rpc_types_debug::ExecutionWitness;
use antibody_verifier::{
    db::MissingWitness, execute_replay, Envelope, Payload, PriceFeed, QuorumVote, ReplayWitness, RunOptions,
    VerifyError, WITNESS_VERSION,
};
use eyre::{eyre, Result};
use reth_ethereum_primitives::{Block, EthPrimitives};
use reth_trie::AccountProof;
use revm::{primitives::KECCAK_EMPTY, state::Bytecode};
use rsp_client_executor::IntoPrimitives;
use rsp_mpt::EthereumState;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    fetchdb::{FetchingDb, Recorded, Seed},
    rpc::{hex_u64, Pool, Provider},
};

#[derive(Debug, Clone, Serialize)]
pub struct HeaderVote {
    pub source: String,
    pub reported_hash: Option<B256>,
    pub recomputed_hash: Option<B256>,
    pub error: Option<String>,
}

pub struct BlockQuorum {
    pub block: Block,
    pub hash: B256,
    pub votes: Vec<HeaderVote>,
    pub agreeing: usize,
}

fn parallel<T: Send>(pool: &Pool, f: impl Fn(&Arc<Provider>) -> T + Sync) -> Vec<(String, T)> {
    std::thread::scope(|s| {
        let handles: Vec<_> = pool.iter().map(|p| (p.label.clone(), s.spawn(|| f(p)))).collect();
        handles.into_iter().map(|(l, h)| (l, h.join().expect("thread"))).collect()
    })
}

/// Fetches block `n` (with transactions) from every provider, recomputes keccak(RLP(header))
/// locally and requires at least `min_agree` providers to agree on the same, locally reproduced hash.
pub fn fetch_block_quorum(pool: &Pool, n: u64, min_agree: usize) -> Result<BlockQuorum> {
    let res = parallel(pool, |p| p.call("eth_getBlockByNumber", json!([format!("{n:#x}"), true])));
    let mut votes = Vec::new();
    let mut by_hash: BTreeMap<B256, (usize, Option<Block>)> = BTreeMap::new();
    for (label, r) in res {
        match r.and_then(|v| serde_json::from_value::<alloy_rpc_types::Block>(v).map_err(|e| eyre!("{e}"))) {
            Ok(rpc_block) => {
                let reported = rpc_block.header.hash;
                let block: Block = EthPrimitives::into_primitive_block(rpc_block);
                let recomputed = block.header.hash_slow();
                votes.push(HeaderVote {
                    source: label,
                    reported_hash: Some(reported),
                    recomputed_hash: Some(recomputed),
                    error: None,
                });
                if reported == recomputed {
                    let e = by_hash.entry(recomputed).or_insert((0, None));
                    e.0 += 1;
                    if e.1.is_none() {
                        e.1 = Some(block);
                    }
                }
            }
            Err(e) => votes.push(HeaderVote {
                source: label,
                reported_hash: None,
                recomputed_hash: None,
                error: Some(e.to_string().chars().take(160).collect()),
            }),
        }
    }
    let (hash, (agreeing, block)) = by_hash
        .into_iter()
        .max_by_key(|(_, (c, _))| *c)
        .ok_or_else(|| eyre!("no provider returned a block whose recomputed hash matches its reported hash"))?;
    if agreeing < min_agree {
        return Err(eyre!("header quorum not reached for block {n}: {agreeing} < {min_agree}"));
    }
    Ok(BlockQuorum { block: block.unwrap(), hash, votes, agreeing })
}

pub fn fetch_header(pool: &Pool, n: u64) -> Result<Header> {
    let mut last = eyre!("no providers");
    for p in pool {
        match p
            .call("eth_getBlockByNumber", json!([format!("{n:#x}"), false]))
            .and_then(|v| serde_json::from_value::<alloy_rpc_types::Header>(v).map_err(|e| eyre!("{e}")))
        {
            Ok(h) => {
                let reported = h.hash;
                let header: Header = EthPrimitives::into_consensus_header(h);
                if header.hash_slow() == reported {
                    return Ok(header);
                }
                last = eyre!("{}: header hash not reproducible for {n}", p.label);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Tries `debug_executionWitness(n)` on each provider; returns the first success.
pub fn fetch_execution_witness(pool: &Pool, n: u64) -> Option<(String, ExecutionWitness, u64)> {
    for p in pool {
        let t0 = Instant::now();
        if let Ok(mut v) = p.call("debug_executionWitness", json!([format!("{n:#x}")])) {
            // Some providers return `null` for empty lists (e.g. `keys`).
            for k in ["state", "codes", "keys", "headers"] {
                if v.get(k).map(|x| x.is_null()).unwrap_or(true) {
                    v[k] = json!([]);
                }
            }
            if let Ok(w) = serde_json::from_value::<ExecutionWitness>(v) {
                return Some((p.label.clone(), w, t0.elapsed().as_millis() as u64));
            }
        }
    }
    None
}

pub fn seed_from_execution_witness(w: &ExecutionWitness, parent_state_root: B256) -> Seed {
    let state = std::panic::catch_unwind(|| EthereumState::from_execution_witness(w, parent_state_root)).ok();
    let codes = w
        .codes
        .iter()
        .map(|c| {
            let b = Bytecode::new_raw(c.clone());
            (b.hash_slow(), b)
        })
        .collect();
    let mut block_hashes = BTreeMap::new();
    for h in &w.headers {
        if let Ok(hdr) = <Header as alloy_rlp::Decodable>::decode(&mut h.as_ref()) {
            block_hashes.insert(hdr.number, hdr.hash_slow());
        }
    }
    Seed { state, codes, block_hashes }
}

/// Normalised eth_getProof result used for the byte-level quorum comparison. Clients differ in
/// cosmetic encodings (e.g. storage key echoed as `0x0` vs 32 bytes, codeHash `0x0` vs
/// keccak(""), storageHash `0x0` vs the empty root for absent accounts).
#[derive(Debug, Clone, PartialEq, Eq)]
struct NormProof {
    balance: U256,
    nonce: u64,
    code_hash: B256,
    storage_hash: B256,
    account_proof: Vec<Bytes>,
    storage: Vec<(B256, U256, Vec<Bytes>)>,
}

fn normalize(v: &Value) -> Result<NormProof> {
    let parse_b256 = |x: &Value| -> Result<B256> {
        let s = x.as_str().ok_or_else(|| eyre!("not a string"))?;
        let u = U256::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| eyre!("{e}"))?;
        Ok(B256::from(u))
    };
    let mut code_hash: B256 = serde_json::from_value(v["codeHash"].clone())?;
    if code_hash == B256::ZERO {
        code_hash = KECCAK_EMPTY;
    }
    let mut storage_hash: B256 = serde_json::from_value(v["storageHash"].clone())?;
    if storage_hash == B256::ZERO {
        storage_hash = reth_trie::EMPTY_ROOT_HASH;
    }
    let mut storage = Vec::new();
    for sp in v["storageProof"].as_array().cloned().unwrap_or_default() {
        storage.push((parse_b256(&sp["key"])?, serde_json::from_value(sp["value"].clone())?, serde_json::from_value(sp["proof"].clone())?));
    }
    storage.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(NormProof {
        balance: serde_json::from_value(v["balance"].clone())?,
        nonce: hex_u64(&v["nonce"])?,
        code_hash,
        storage_hash,
        account_proof: serde_json::from_value(v["accountProof"].clone())?,
        storage,
    })
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProofQuorumReport {
    pub accounts: usize,
    pub slots: usize,
    /// Accounts for which every responding provider returned identical normalised proofs.
    pub unanimous: usize,
    /// Accounts with at least `min_agree` identical answers.
    pub quorum_reached: usize,
    pub responses_per_provider: BTreeMap<String, usize>,
    pub failures_per_provider: BTreeMap<String, usize>,
    pub raw_json_differs_but_normalised_equal: usize,
    pub disagreements: Vec<String>,
    pub fetch_ms: u64,
}

/// Fetches eth_getProof(address, slots, block) for every touched account from every provider and
/// compares the normalised answers. Proofs are self-verifying against the state root, so the
/// quorum here is a provider-health and liveness check, not the soundness anchor.
pub fn fetch_proofs_quorum(
    pool: &Pool,
    block: u64,
    access: &BTreeMap<Address, BTreeSet<U256>>,
    min_agree: usize,
) -> Result<(HashMap<Address, AccountProof>, ProofQuorumReport)> {
    let t0 = Instant::now();
    let addrs: Vec<Address> = access.keys().copied().collect();
    let reqs: Vec<(String, Value)> = addrs
        .iter()
        .map(|a| {
            let slots: Vec<String> = access[a].iter().map(|s| format!("{s:#066x}")).collect();
            ("eth_getProof".to_string(), json!([a, slots, format!("{block:#x}")]))
        })
        .collect();
    let results = parallel(pool, |p| p.batch(&reqs));
    let mut report = ProofQuorumReport { accounts: addrs.len(), slots: access.values().map(|s| s.len()).sum(), ..Default::default() };
    let mut out: HashMap<Address, AccountProof> = HashMap::default();
    for (i, a) in addrs.iter().enumerate() {
        let mut answers: Vec<(String, NormProof, Value)> = Vec::new();
        for (label, rs) in &results {
            match &rs[i] {
                Ok(v) => match normalize(v) {
                    Ok(n) => {
                        *report.responses_per_provider.entry(label.clone()).or_default() += 1;
                        answers.push((label.clone(), n, v.clone()));
                    }
                    Err(_) => *report.failures_per_provider.entry(label.clone()).or_default() += 1,
                },
                Err(_) => *report.failures_per_provider.entry(label.clone()).or_default() += 1,
            }
        }
        if answers.is_empty() {
            return Err(eyre!("no provider returned a proof for {a} at block {block}"));
        }
        // Group identical normalised answers.
        let mut groups: Vec<(NormProof, Vec<String>, Value)> = Vec::new();
        for (l, n, v) in answers.iter() {
            match groups.iter_mut().find(|g| &g.0 == n) {
                Some(g) => g.1.push(l.clone()),
                None => groups.push((n.clone(), vec![l.clone()], v.clone())),
            }
        }
        groups.sort_by_key(|g| std::cmp::Reverse(g.1.len()));
        let raw_all_equal = answers.windows(2).all(|w| w[0].2 == w[1].2);
        if groups.len() == 1 {
            report.unanimous += 1;
            if !raw_all_equal {
                report.raw_json_differs_but_normalised_equal += 1;
            }
        } else {
            report.disagreements.push(format!(
                "{a}: {} groups: {}",
                groups.len(),
                groups.iter().map(|g| g.1.join("+")).collect::<Vec<_>>().join(" vs ")
            ));
        }
        if groups[0].1.len() >= min_agree {
            report.quorum_reached += 1;
        }
        let resp: alloy_rpc_types::EIP1186AccountProofResponse = serde_json::from_value(groups[0].2.clone())?;
        out.insert(*a, rsp_primitives::account_proof::eip1186_proof_to_account_proof(resp));
    }
    report.fetch_ms = t0.elapsed().as_millis() as u64;
    Ok((out, report))
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct WitnessStats {
    pub witness_bytes: usize,
    pub witness_sha256: String,
    pub accounts: usize,
    pub storage_slots: usize,
    pub storage_tries: usize,
    pub proof_nodes_total: usize,
    pub proof_nodes_unique: usize,
    pub proof_bytes_unique: usize,
    pub bytecodes: usize,
    pub bytecode_bytes: usize,
    pub ancestor_headers: usize,
    pub block_rlp_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildMeta {
    pub tx_hash: B256,
    pub block_number: u64,
    pub block_hash: B256,
    pub tx_index: u32,
    pub txs_in_block: usize,
    pub header_votes: Vec<HeaderVote>,
    pub header_agreeing: usize,
    pub execution_witness_source: Option<String>,
    pub execution_witness_ms: Option<u64>,
    pub trace_ms: u64,
    pub trace_seed_hits: u64,
    pub trace_rpc_reads: u64,
    pub fixpoint_passes: usize,
    pub fixpoint_added: Vec<String>,
    pub proof_quorum: Vec<ProofQuorumReport>,
    pub stats: WitnessStats,
    pub rpc: BTreeMap<String, crate::rpc::Stats>,
    pub total_ms: u64,
    pub verify_native_ms: u64,
    pub trace_and_strict_agree: bool,
}

pub fn witness_stats(env: &Envelope, bytes: &[u8], proofs: &HashMap<Address, AccountProof>) -> WitnessStats {
    let mut st = WitnessStats { witness_bytes: bytes.len(), witness_sha256: hex::encode(Sha256::digest(bytes)), ..Default::default() };
    let mut uniq: BTreeSet<B256> = BTreeSet::new();
    for p in proofs.values() {
        st.accounts += 1;
        for n in &p.proof {
            st.proof_nodes_total += 1;
            if uniq.insert(keccak256(n)) {
                st.proof_bytes_unique += n.len();
            }
        }
        for sp in &p.storage_proofs {
            st.storage_slots += 1;
            for n in &sp.proof {
                st.proof_nodes_total += 1;
                if uniq.insert(keccak256(n)) {
                    st.proof_bytes_unique += n.len();
                }
            }
        }
    }
    st.proof_nodes_unique = uniq.len();
    match &env.payload {
        Payload::Replay(w) => {
            st.storage_tries = w.parent_state.storage_tries.len();
            st.bytecodes = w.bytecodes.len();
            st.bytecode_bytes = w.bytecodes.iter().map(|b| b.original_bytes().len()).sum();
            st.ancestor_headers = w.ancestor_headers.len();
            st.block_rlp_bytes = w.block_rlp.len();
        }
        Payload::Bundle(w) => {
            st.storage_tries = w.state.storage_tries.len();
            st.bytecodes = w.bytecodes.len();
            st.bytecode_bytes = w.bytecodes.iter().map(|b| b.original_bytes().len()).sum();
            st.ancestor_headers = w.ancestor_headers.len();
            st.block_rlp_bytes = w.header_rlp.len();
        }
        Payload::Program(w) => {
            st.storage_tries = w.state.storage_tries.len();
            st.bytecodes = w.bytecodes.len();
            st.bytecode_bytes = w.bytecodes.iter().map(|b| b.original_bytes().len()).sum();
            st.ancestor_headers = w.ancestor_headers_rlp.len();
            st.block_rlp_bytes = w.header_rlp.len();
        }
        Payload::Base(w) => {
            st.storage_tries = w.state.storage_tries.len();
            st.bytecodes = w.bytecodes.len();
            st.bytecode_bytes = w.bytecodes.iter().map(|b| b.original_bytes().len()).sum();
            st.ancestor_headers = w.ancestor_headers_rlp.len();
            st.block_rlp_bytes = w.header_rlp.len();
        }
        Payload::Submission(w) => {
            st.storage_tries = w.state.storage_tries.len();
            st.bytecodes = w.bytecodes.len();
            st.bytecode_bytes = w.bytecodes.iter().map(|b| b.original_bytes().len()).sum();
        }
        Payload::Block(_) => {}
    }
    st
}

/// Mainnet Chainlink USD aggregator proxies (price manifest). Each is sanity-checked on chain via
/// `description()` by the witness builder before use.
pub const CHAINLINK_USD: &[(&str, &str, &str)] = &[
    ("0x0000000000000000000000000000000000000000", "0x5f4eC3Df9cbd43714FE2740f5E3616155c5b8419", "ETH / USD"),
    ("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2", "0x5f4eC3Df9cbd43714FE2740f5E3616155c5b8419", "ETH / USD"),
    ("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", "0x8fFfFfd4AfB6115b954Bd326cbe7B4BA576818f6", "USDC / USD"),
    ("0xdAC17F958D2ee523a2206206994597C13D831ec7", "0x3E7d1eAB13ad0104d2750B8863b489D65364e32D", "USDT / USD"),
    ("0x6B175474E89094C44Da98b954EedeAC495271d0F", "0xAed0c38402a5d19df6E4c03F4E2DceD6e29c1ee9", "DAI / USD"),
    ("0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599", "0xF4030086522a5bEEa4988F8cA5B36dbC97BeE88c", "BTC / USD"),
    ("0x514910771AF9Ca656af840dff83E8264EcF986CA", "0x2c1d072e956AFFC0D435Cb7AC38EF18d24d9127c", "LINK / USD"),
    ("0x7Fc66500c84A76Ad7e9c93437bFc5Ac33E2DDaE9", "0x547a514d5e3769680Ce22B2361c10Ea13619e8a9", "AAVE / USD"),
    ("0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984", "0x553303d460EE0afB37EdFf9bE42922D8FF63220e", "UNI / USD"),
    ("0xae7ab96520DE3A18E5e111B5EaAb095312D7fE84", "0xCfE54B5cD566aB89272946F602D76Ea879CAb4a8", "STETH / USD"),
    ("0x40D16FC0246aD3160Ccc09B8D0D3A2cD28aE6C2f", "0x3f12643D3f6f874d39C2a4c9f2Cd6f2DbAC877FC", "GHO / USD"),
];

/// Aave v3 Ethereum `AaveOracle` (fallback USD prices, 8 decimals, for Aave-listed assets).
pub const AAVE_ORACLE: &str = "0x54586bE62E3c3580375aE3723C145253060Ca0C2";

fn eth_call_any(pool: &Pool, to: &str, data: &str, block: u64) -> Option<String> {
    for p in pool {
        if let Ok(v) = p.call("eth_call", json!([{"to": to, "data": data}, format!("{block:#x}")])) {
            if let Some(s) = v.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Returns the price manifest after checking each feed's `description()` at `block`.
pub fn price_manifest(pool: &Pool, block: u64) -> (Vec<PriceFeed>, Vec<String>) {
    let mut feeds = Vec::new();
    let mut notes = Vec::new();
    let mut checked: BTreeMap<String, bool> = BTreeMap::new();
    for (token, feed, desc) in CHAINLINK_USD {
        let ok = *checked.entry(feed.to_string()).or_insert_with(|| {
            match eth_call_any(pool, feed, "0x7284e416", block) {
                Some(hexs) => {
                    let b = hex::decode(hexs.trim_start_matches("0x")).unwrap_or_default();
                    let s = String::from_utf8_lossy(&b).to_string();
                    s.contains(desc)
                }
                None => false,
            }
        });
        if ok {
            feeds.push(PriceFeed { token: token.parse().unwrap(), feed: feed.parse().unwrap() });
        } else {
            notes.push(format!("feed {feed} ({desc}) failed description() check; skipped"));
        }
    }
    (feeds, notes)
}

/// Returns the AaveOracle address if it answers getAssetPrice(WETH) with a non-zero price.
pub fn aave_oracle(pool: &Pool, block: u64) -> Option<Address> {
    let data = "0xb3596f07000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2";
    let r = eth_call_any(pool, AAVE_ORACLE, data, block)?;
    let v = U256::from_str_radix(r.trim_start_matches("0x"), 16).ok()?;
    (!v.is_zero()).then(|| AAVE_ORACLE.parse().unwrap())
}

pub struct Built {
    pub envelope: Envelope,
    pub bytes: Vec<u8>,
    pub meta: BuildMeta,
    pub receipt_json: String,
}

/// Builds a replay witness for `tx_hash` and runs the strict verifier natively to a fixpoint.
pub fn build_replay(
    archive: &Pool,
    quorum_pool: &Pool,
    tx_hash: B256,
    extra_watch: &[Address],
    min_agree: usize,
    forge_timestamp_delta: Option<u64>,
) -> Result<Built> {
    let t_all = Instant::now();
    let tx = archive[0].call("eth_getTransactionByHash", json!([tx_hash]))?;
    let n = hex_u64(&tx["blockNumber"])?;
    let idx = hex_u64(&tx["transactionIndex"])? as u32;
    let to: Option<Address> = serde_json::from_value(tx["to"].clone()).ok().flatten();

    // 1. Header quorum on block N (recomputed hash) and the parent header.
    let mut q = fetch_block_quorum(quorum_pool, n, min_agree)?;
    // Negative test (V1): a self-consistent forged header. Everything the verifier can check
    // internally still holds (parent link, tx root, real state proofs); only an external anchor
    // of the block hash can reject it.
    if let Some(d) = forge_timestamp_delta {
        q.block.header.timestamp += d;
        q.hash = q.block.header.hash_slow();
    }
    let parent = fetch_header(archive, n - 1)?;
    if parent.hash_slow() != q.block.header.parent_hash {
        return Err(eyre!("parent header does not hash to block.parent_hash"));
    }

    // 2. Optional seed from an execution witness (fast trace), validated against the parent root.
    let ew = fetch_execution_witness(archive, n);
    let (seed, ew_src, ew_ms) = match &ew {
        Some((src, w, ms)) => (seed_from_execution_witness(w, parent.state_root), Some(src.clone()), Some(*ms)),
        None => (Seed::default(), None, None),
    };

    // 3. Trace with the verifier's own execution path on a recording, fetching database.
    let (feeds, _notes) = price_manifest(quorum_pool, n - 1);
    let oracle = aave_oracle(quorum_pool, n - 1);
    let mut watch: Vec<Address> = extra_watch.to_vec();
    if let Some(t) = to {
        if !watch.contains(&t) {
            watch.push(t);
        }
    }
    let t_trace = Instant::now();
    let fdb = FetchingDb::new(archive.clone(), n - 1, seed);
    let trace_exec = execute_replay(&q.block, idx, &watch, &feeds, oracle, 1, true, &fdb)
        .map_err(|e| eyre!("trace execution failed: {e}"))?;
    let trace_ms = t_trace.elapsed().as_millis() as u64;
    let rec: Recorded = fdb.rec.borrow().clone();

    // 4. Access set -> quorum proofs -> witness -> strict verify; repeat to a fixpoint.
    let mut access: BTreeMap<Address, BTreeSet<U256>> = BTreeMap::new();
    for a in &rec.accounts {
        access.entry(*a).or_default();
    }
    for (a, s) in &rec.slots {
        access.entry(*a).or_default().extend(s.iter().copied());
    }
    let mut codes = rec.codes.clone();
    let mut block_numbers = rec.block_numbers.clone();
    let mut quorum_reports = Vec::new();
    let mut added = Vec::new();
    let block_rlp: Bytes = alloy_rlp::encode(&q.block).into();
    let mut ancestors_cache: BTreeMap<u64, Header> = BTreeMap::new();
    ancestors_cache.insert(n - 1, parent.clone());

    for pass in 1..=8usize {
        let (proofs, qr) = fetch_proofs_quorum(archive, n - 1, &access, min_agree.min(archive.len()))?;
        quorum_reports.push(qr);
        let state = EthereumState::from_proofs(parent.state_root, &proofs).map_err(|e| eyre!("from_proofs: {e}"))?;
        // Codes for every touched contract (verified by hash inside the verifier).
        for p in proofs.values() {
            if let Some(info) = &p.info {
                let h = info.bytecode_hash.unwrap_or(KECCAK_EMPTY);
                if h != KECCAK_EMPTY && !codes.contains_key(&h) {
                    let v = archive[0].call("eth_getCode", json!([p.address, format!("{:#x}", n - 1)]))?;
                    let b: Bytes = serde_json::from_value(v)?;
                    let bc = Bytecode::new_raw(b);
                    codes.insert(bc.hash_slow(), bc);
                }
            }
        }
        // Ancestor headers: N-1 down to the oldest BLOCKHASH requested.
        let oldest = block_numbers.iter().next().copied().unwrap_or(n - 1).min(n - 1);
        let mut ancestors = Vec::new();
        for k in (oldest..=n - 1).rev() {
            if !ancestors_cache.contains_key(&k) {
                ancestors_cache.insert(k, fetch_header(archive, k)?);
            }
            ancestors.push(ancestors_cache[&k].clone());
        }
        let envelope = Envelope {
            version: WITNESS_VERSION,
            chain_id: 1,
            claimed_block_hash: q.hash,
            quorum: q
                .votes
                .iter()
                .filter_map(|v| v.recomputed_hash.map(|h| QuorumVote { source: v.source.clone(), block_hash: h }))
                .collect(),
            payload: Payload::Replay(ReplayWitness {
                block_rlp: block_rlp.clone(),
                target_index: idx,
                ancestor_headers: ancestors,
                parent_state: state,
                bytecodes: codes.values().cloned().collect(),
                watch: watch.clone(),
                price_feeds: feeds.clone(),
                aave_oracle: oracle,
            }),
        };
        let bytes = envelope.encode();
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let t_v = Instant::now();
        match antibody_verifier::run(&envelope, digest, &RunOptions { full_diff: true }) {
            Ok(receipt) => {
                let verify_ms = t_v.elapsed().as_millis() as u64;
                let rt = receipt.target.as_ref().unwrap();
                let agree = rt.logs_keccak == trace_exec.target.logs_keccak
                    && rt.diff_keccak == trace_exec.target.diff_keccak
                    && rt.gas_used == trace_exec.target.gas_used;
                let stats = witness_stats(&envelope, &bytes, &proofs);
                let mut rpc: BTreeMap<String, crate::rpc::Stats> = BTreeMap::new();
                for p in archive.iter().chain(quorum_pool.iter()) {
                    let s = p.stats();
                    let e = rpc.entry(p.label.clone()).or_default();
                    e.http_requests += s.http_requests;
                    e.rpc_calls += s.rpc_calls;
                    e.errors += s.errors;
                    e.retries += s.retries;
                    e.bytes_in += s.bytes_in;
                    e.ms += s.ms;
                }
                let meta = BuildMeta {
                    tx_hash,
                    block_number: n,
                    block_hash: q.hash,
                    tx_index: idx,
                    txs_in_block: q.block.body.transactions.len(),
                    header_votes: q.votes.clone(),
                    header_agreeing: q.agreeing,
                    execution_witness_source: ew_src.clone(),
                    execution_witness_ms: ew_ms,
                    trace_ms,
                    trace_seed_hits: rec.seed_hits,
                    trace_rpc_reads: rec.rpc_reads,
                    fixpoint_passes: pass,
                    fixpoint_added: added.clone(),
                    proof_quorum: quorum_reports.clone(),
                    stats,
                    rpc,
                    total_ms: t_all.elapsed().as_millis() as u64,
                    verify_native_ms: verify_ms,
                    trace_and_strict_agree: agree,
                };
                let receipt_json = serde_json::to_string_pretty(&receipt)?;
                return Ok(Built { envelope, bytes, meta, receipt_json });
            }
            Err(VerifyError::Incomplete(m)) => {
                added.push(m.to_string());
                match m {
                    MissingWitness::Account(a) | MissingWitness::StorageTrie(a) => {
                        access.entry(a).or_default();
                    }
                    MissingWitness::Slot(a, s) => {
                        access.entry(a).or_default().insert(s);
                    }
                    MissingWitness::Code(h) => return Err(eyre!("code {h} missing after trace (pass {pass})")),
                    MissingWitness::BlockHash(b) => {
                        block_numbers.insert(b);
                    }
                }
            }
            Err(e) => return Err(eyre!("strict verification failed (pass {pass}): {e} [{}]", e.class())),
        }
    }
    Err(eyre!("fixpoint not reached after 8 passes"))
}


/// JSON form of a bundle. Calldata may contain `5e…5e` (20 bytes) = sender 0, `5f…5f` = sender 1, and
/// `5c…5c` followed by one byte k = the contract call k creates; `to` may be that placeholder too.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BundleFile {
    pub senders: u8,
    pub endowment_wei: String,
    #[serde(default)]
    pub protected: Vec<Address>,
    pub calls: Vec<BundleFileCall>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct BundleFileCall {
    pub sender: u8,
    pub to: Option<Address>,
    pub value: String,
    pub data: String,
    pub gas_limit: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BundleBuildMeta {
    pub anchor_block: u64,
    pub anchor_hash: B256,
    pub header_agreeing: usize,
    pub senders: Vec<Address>,
    pub trace_ms: u64,
    pub trace_rpc_reads: u64,
    pub fixpoint_passes: usize,
    pub fixpoint_added: Vec<String>,
    pub proof_quorum: Vec<ProofQuorumReport>,
    pub stats: WitnessStats,
    pub total_ms: u64,
    pub verify_native_ms: u64,
}

pub fn parse_bundle(f: &BundleFile, salt: B256) -> Result<antibody_verifier::bundle::Bundle> {
    use antibody_verifier::bundle::{sender_address, Bundle, BundleCall};
    let placeholders = [("5e".repeat(20), 0u8), ("5f".repeat(20), 1u8)];
    // Every call is a transaction from a fresh sender, so call k creates at its sender's count of earlier calls.
    let mut nonces = vec![0u64; f.senders as usize];
    let mut created = Vec::new();
    for c in &f.calls {
        let n = nonces.get_mut(c.sender as usize).ok_or_else(|| eyre!("call from sender {} of {}", c.sender, f.senders))?;
        created.push(c.to.is_none().then(|| sender_address(&salt, c.sender).create(*n)));
        *n += 1;
    }
    let creation = |k: usize, at: usize| -> Result<Address> {
        created.get(k).copied().flatten().ok_or_else(|| eyre!("call {at} names the contract call {k} creates, but call {k} creates none"))
    };
    let mut calls = Vec::new();
    for (at, c) in f.calls.iter().enumerate() {
        let mut data = c.data.trim_start_matches("0x").to_lowercase();
        for (ph, i) in &placeholders {
            if *i < f.senders {
                let addr = hex::encode(sender_address(&salt, *i).as_slice());
                data = data.replace(ph.as_str(), &addr);
            }
        }
        let prefix = "5c".repeat(19);
        let mut p = 0;
        while p + 40 <= data.len() {
            if data[p..p + 38] == prefix {
                let k = usize::from_str_radix(&data[p + 38..p + 40], 16)?;
                data.replace_range(p..p + 40, &hex::encode(creation(k, at)?));
                p += 40;
            } else {
                p += 2;
            }
        }
        let to = match c.to {
            Some(t) if t.as_slice()[..19] == [0x5c; 19] => Some(creation(t.as_slice()[19] as usize, at)?),
            other => other,
        };
        calls.push(BundleCall {
            sender: c.sender,
            to,
            value: c.value.parse()?,
            data: hex::decode(&data)?.into(),
            gas_limit: c.gas_limit,
        });
    }
    Ok(Bundle { sender_salt: salt, senders: f.senders, endowment_wei: f.endowment_wei.parse()?, calls })
}

/// Builds a bundle witness at anchor block `m` (state = post-state of m, env = header m).
pub fn build_bundle(
    archive: &Pool,
    quorum_pool: &Pool,
    m: u64,
    bundle: antibody_verifier::bundle::Bundle,
    protected: Vec<Address>,
    min_agree: usize,
) -> Result<(Envelope, Vec<u8>, BundleBuildMeta, String)> {
    use antibody_verifier::{bundle::sender_address, BundleWitness};
    let t_all = Instant::now();
    let q = fetch_block_quorum(quorum_pool, m, min_agree)?;
    let header = q.block.header.clone();
    let spec = Arc::new(rsp_primitives::chain_spec::mainnet().map_err(|e| eyre!("{e}"))?);
    let (feeds, _notes) = price_manifest(quorum_pool, m);
    let oracle = aave_oracle(quorum_pool, m);
    let t_trace = Instant::now();
    let fdb = FetchingDb::new(archive.clone(), m, Seed::default());
    antibody_verifier::bundle::execute_bundle(&header, &bundle, &protected, &feeds, oracle, spec, &fdb)
        .map_err(|e| eyre!("trace execution failed: {e}"))?;
    let trace_ms = t_trace.elapsed().as_millis() as u64;
    let rec: Recorded = fdb.rec.borrow().clone();
    let mut access: BTreeMap<Address, BTreeSet<U256>> = BTreeMap::new();
    for a in &rec.accounts {
        access.entry(*a).or_default();
    }
    for (a, s) in &rec.slots {
        access.entry(*a).or_default().extend(s.iter().copied());
    }
    let mut codes = rec.codes.clone();
    let mut block_numbers = rec.block_numbers.clone();
    let mut reports = Vec::new();
    let mut added = Vec::new();
    let mut anc_cache: BTreeMap<u64, Header> = BTreeMap::new();
    for pass in 1..=8usize {
        let (proofs, qr) = fetch_proofs_quorum(archive, m, &access, min_agree.min(archive.len()))?;
        reports.push(qr);
        let state = EthereumState::from_proofs(header.state_root, &proofs).map_err(|e| eyre!("from_proofs: {e}"))?;
        for p in proofs.values() {
            if let Some(info) = &p.info {
                let h = info.bytecode_hash.unwrap_or(KECCAK_EMPTY);
                if h != KECCAK_EMPTY && !codes.contains_key(&h) {
                    let v = archive[0].call("eth_getCode", json!([p.address, format!("{m:#x}")]))?;
                    let b: Bytes = serde_json::from_value(v)?;
                    let bc = Bytecode::new_raw(b);
                    codes.insert(bc.hash_slow(), bc);
                }
            }
        }
        let oldest = block_numbers.iter().next().copied().unwrap_or(m).min(m);
        let mut ancestors = Vec::new();
        for k in (oldest..m).rev() {
            if !anc_cache.contains_key(&k) {
                anc_cache.insert(k, fetch_header(archive, k)?);
            }
            ancestors.push(anc_cache[&k].clone());
        }
        let envelope = Envelope {
            version: WITNESS_VERSION,
            chain_id: 1,
            claimed_block_hash: q.hash,
            quorum: q
                .votes
                .iter()
                .filter_map(|v| v.recomputed_hash.map(|h| QuorumVote { source: v.source.clone(), block_hash: h }))
                .collect(),
            payload: Payload::Bundle(BundleWitness {
                header_rlp: alloy_rlp::encode(&header).into(),
                ancestor_headers: ancestors,
                state,
                bytecodes: codes.values().cloned().collect(),
                bundle: bundle.clone(),
                protected: protected.clone(),
                price_feeds: feeds.clone(),
                aave_oracle: oracle,
            }),
        };
        let bytes = envelope.encode();
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let t_v = Instant::now();
        match antibody_verifier::run(&envelope, digest, &RunOptions { full_diff: false }) {
            Ok(receipt) => {
                let meta = BundleBuildMeta {
                    anchor_block: m,
                    anchor_hash: q.hash,
                    header_agreeing: q.agreeing,
                    senders: (0..bundle.senders).map(|i| sender_address(&bundle.sender_salt, i)).collect(),
                    trace_ms,
                    trace_rpc_reads: rec.rpc_reads,
                    fixpoint_passes: pass,
                    fixpoint_added: added.clone(),
                    proof_quorum: reports.clone(),
                    stats: witness_stats(&envelope, &bytes, &proofs),
                    total_ms: t_all.elapsed().as_millis() as u64,
                    verify_native_ms: t_v.elapsed().as_millis() as u64,
                };
                let rj = serde_json::to_string_pretty(&receipt)?;
                return Ok((envelope, bytes, meta, rj));
            }
            Err(VerifyError::Incomplete(mw)) => {
                added.push(mw.to_string());
                match mw {
                    MissingWitness::Account(a) | MissingWitness::StorageTrie(a) => {
                        access.entry(a).or_default();
                    }
                    MissingWitness::Slot(a, s) => {
                        access.entry(a).or_default().insert(s);
                    }
                    MissingWitness::Code(h) => return Err(eyre!("code {h} missing after trace (pass {pass})")),
                    MissingWitness::BlockHash(b) => {
                        block_numbers.insert(b);
                    }
                }
            }
            Err(e) => return Err(eyre!("strict verification failed (pass {pass}): {e} [{}]", e.class())),
        }
    }
    Err(eyre!("fixpoint not reached after 8 passes"))
}

#[cfg(test)]
mod parse_bundle_tests {
    use super::*;
    use antibody_verifier::bundle::sender_address;

    #[test]
    fn a_created_contract_placeholder_becomes_the_senders_create_address() {
        let salt = B256::repeat_byte(7);
        let call = |sender: u8, to: Option<&str>, data: String| BundleFileCall { sender, to: to.map(|t| t.parse().unwrap()), value: "0".into(), data, gas_limit: 1_000_000 };
        let created = format!("0x{}00", "5c".repeat(19));
        let f = BundleFile {
            senders: 1,
            endowment_wei: "1".into(),
            protected: vec![],
            calls: vec![
                call(0, None, "0x6080".into()),
                call(0, Some(&created), format!("0xdeadbeef{}{}", "0".repeat(24), &created[2..])),
            ],
        };
        let b = parse_bundle(&f, salt).unwrap();
        let want = sender_address(&salt, 0).create(0);
        assert_eq!(b.calls[1].to, Some(want));
        assert_eq!(&b.calls[1].data[16..], want.as_slice());
        let bad = BundleFile { calls: vec![call(0, Some(&created), "0x".into())], ..f };
        assert!(parse_bundle(&bad, salt).unwrap_err().to_string().contains("creates none"));
    }
}

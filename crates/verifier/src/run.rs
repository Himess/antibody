//! Verification + execution entry points.

use std::{collections::BTreeMap, iter::once, sync::Arc};

use alloy_consensus::{
    transaction::{Recovered, SignerRecoverable},
    BlockHeader, Header, Transaction as _, TxReceipt,
};
use alloy_eips::eip2718::Encodable2718;
use alloy_evm::{
    block::{BlockExecutor, TxResult},
    Evm,
};
use alloy_primitives::{keccak256, map::HashMap, Address, Bytes, B256, I256, U256};
use alloy_rlp::{Decodable, Encodable};
use itertools::Itertools;
use reth_ethereum_primitives::{Block, TransactionSigned};
use reth_evm::ConfigureEvm;
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives_traits::{SealedBlock, SealedHeader};
use revm::{
    database::{State, WrapDatabaseRef},
    database_interface::DBErrorMarker,
    state::{Bytecode, EvmState},
    Database, DatabaseRef,
};
use rsp_client_executor::{
    custom::{CustomCrypto, CustomEvmFactory},
    executor::EthClientExecutor,
    io::{build_trie_db, EthClientExecutorInput},
};

use crate::{
    db::{MissingTracker, MissingWitness, StrictDb},
    impact::{self, PROBE_CALLER},
    receipt::*,
    witness::{BundleWitness, Envelope, Payload, PriceFeed, ProgramWitness, ReplayWitness, WITNESS_VERSION},
};

#[derive(Debug)]
pub enum VerifyError {
    Decode(String),
    UnsupportedChain(u64),
    HeaderHashMismatch { claimed: B256, computed: B256 },
    TransactionsRootMismatch { header: B256, computed: B256 },
    Witness(String),
    TargetOutOfRange(u32, usize),
    SignatureRecovery(u32),
    Incomplete(MissingWitness),
    Execution(String),
    UnsupportedFork(String),
}

impl VerifyError {
    /// tampered: a commitment check failed (proof, header hash, tx root) -> reject.
    /// incomplete: the witness lacks data execution needed -> infra failure, retry/refund.
    /// invalid_input: malformed request.  execution: EVM-level failure of the run itself.
    pub fn class(&self) -> &'static str {
        match self {
            Self::HeaderHashMismatch { .. } | Self::TransactionsRootMismatch { .. } | Self::Witness(_) => {
                "tampered"
            }
            Self::Incomplete(_) => "incomplete",
            Self::Decode(_) | Self::UnsupportedChain(_) | Self::TargetOutOfRange(..) | Self::SignatureRecovery(_) => {
                "invalid_input"
            }
            Self::Execution(_) => "execution",
            Self::UnsupportedFork(_) => "unsupported_fork",
        }
    }
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "decode: {e}"),
            Self::UnsupportedChain(c) => write!(f, "unsupported chain id {c}"),
            Self::HeaderHashMismatch { claimed, computed } => {
                write!(f, "header hash mismatch: claimed {claimed}, keccak(RLP(header)) = {computed}")
            }
            Self::TransactionsRootMismatch { header, computed } => {
                write!(f, "transactions root mismatch: header {header}, computed {computed}")
            }
            Self::Witness(e) => write!(f, "witness verification failed: {e}"),
            Self::TargetOutOfRange(i, n) => write!(f, "target index {i} out of range ({n} txs)"),
            Self::SignatureRecovery(i) => write!(f, "cannot recover signer of tx {i}"),
            Self::Incomplete(m) => write!(f, "incomplete witness: {m}"),
            Self::Execution(e) => write!(f, "execution: {e}"),
            Self::UnsupportedFork(e) => write!(f, "unsupported fork: {e}"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Include the full per-slot state diff of the target tx in the receipt.
    pub full_diff: bool,
}

fn verifier_info() -> VerifierInfo {
    VerifierInfo {
        name: "antibody-verifier",
        version: env!("CARGO_PKG_VERSION"),
        revm: "38.0.0",
        reth: "v2.2.0",
        rsp: "5afe72eb12a2d6accd9d7fdff66785d91cbc8e72",
    }
}

/// Mainnet headers at or after this timestamp are refused until the verifier is upgraded and
/// re-audited for the next fork (Glamsterdam mainnet date is TBD; set conservatively).
pub const MAX_SUPPORTED_TIMESTAMP_MAINNET: u64 = 1_796_083_200; // 2026-12-01T00:00:00Z
/// Glamsterdam activated on Sepolia at this timestamp; the compiled spec (reth v2.2.0) predates it.
pub const SEPOLIA_GLAMSTERDAM_TIMESTAMP: u64 = 1_791_294_816; // 2026-10-06T13:53:36Z

/// Fail-closed fork guard (review finding V6): refuse headers carrying fields of forks the compiled
/// chain spec does not implement, or timestamps beyond the supported range.
pub fn fork_guard(h: &Header, chain_id: u64) -> Result<(), VerifyError> {
    if h.block_access_list_hash.is_some() || h.slot_number.is_some() {
        return Err(VerifyError::UnsupportedFork("Amsterdam header fields present; compiled spec is Osaka".into()));
    }
    let limit = match chain_id {
        1 => MAX_SUPPORTED_TIMESTAMP_MAINNET,
        11155111 => SEPOLIA_GLAMSTERDAM_TIMESTAMP,
        other => return Err(VerifyError::UnsupportedChain(other)),
    };
    if h.timestamp >= limit {
        return Err(VerifyError::UnsupportedFork(format!("timestamp {} >= supported limit {limit}", h.timestamp)));
    }
    Ok(())
}

pub fn chain_spec(chain_id: u64) -> Result<Arc<reth_chainspec::ChainSpec>, VerifyError> {
    match chain_id {
        1 => Ok(Arc::new(
            rsp_primitives::chain_spec::mainnet().map_err(|e| VerifyError::Decode(e.to_string()))?,
        )),
        11155111 => Ok(Arc::new(
            rsp_primitives::chain_spec::sepolia().map_err(|e| VerifyError::Decode(e.to_string()))?,
        )),
        other => Err(VerifyError::UnsupportedChain(other)),
    }
}

pub fn run(env: &Envelope, witness_sha256: [u8; 32], opts: &RunOptions) -> Result<Receipt, VerifyError> {
    if env.version != WITNESS_VERSION {
        return Err(VerifyError::Decode(format!("unsupported witness version {}", env.version)));
    }
    match &env.payload {
        Payload::Replay(w) => run_replay(env, w, witness_sha256, opts),
        Payload::Block(input) => run_block(env, input.clone(), witness_sha256),
        Payload::Bundle(w) => run_bundle(env, w, witness_sha256),
        Payload::Program(_) => Err(VerifyError::Decode("program witnesses go through run_program".into())),
    }
}

/// Runs only the binding/proof checks (no execution). Used to attribute cost per phase.
pub fn verify_only(env: &Envelope) -> Result<Checks, VerifyError> {
    match &env.payload {
        Payload::Replay(w) => verify_replay(env, w).map(|v| v.checks),
        Payload::Block(_) | Payload::Bundle(_) | Payload::Program(_) => {
            Err(VerifyError::Decode("verify_only: replay witnesses only".into()))
        }
    }
}

/// Verifies everything that binds the witness to `claimed_block_hash` and returns the strict DB.
struct Verified<'a> {
    block: Block,
    db: StrictDb<'a>,
    checks: Checks,
}

fn verify_replay<'a>(env: &Envelope, w: &'a ReplayWitness) -> Result<Verified<'a>, VerifyError> {
    // 1. Header binding: the environment comes only from a header whose hash we recompute.
    let block = Block::decode(&mut w.block_rlp.as_ref()).map_err(|e| VerifyError::Decode(e.to_string()))?;
    let computed = block.header.hash_slow();
    if computed != env.claimed_block_hash {
        return Err(VerifyError::HeaderHashMismatch { claimed: env.claimed_block_hash, computed });
    }
    fork_guard(&block.header, env.chain_id)?;
    // 2. The replayed transactions are exactly the block's (transactions root).
    let tx_root = alloy_consensus::proofs::calculate_transaction_root(&block.body.transactions);
    if tx_root != block.header.transactions_root {
        return Err(VerifyError::TransactionsRootMismatch { header: block.header.transactions_root, computed: tx_root });
    }
    let anchor = w
        .ancestor_headers
        .first()
        .ok_or_else(|| VerifyError::Witness("no parent header".into()))?;
    // 3. rsp's in-guest verification: state root, every storage root, parent-hash chain.
    let sealed: Vec<SealedHeader> = once(SealedHeader::seal_slow(block.header.clone()))
        .chain(w.ancestor_headers.iter().map(|h| SealedHeader::seal_slow(h.clone())))
        .collect();
    build_trie_db(&w.parent_state, anchor.state_root, w.bytecodes.iter(), &sealed)
        .map_err(|e| VerifyError::Witness(e.to_string()))?;
    let mut block_hashes: HashMap<u64, B256> = HashMap::default();
    for (child, parent) in sealed.iter().tuple_windows() {
        block_hashes.insert(parent.number(), child.parent_hash());
    }
    let bytecode_by_hash: HashMap<B256, &Bytecode> =
        w.bytecodes.iter().map(|c| (c.hash_slow(), c)).collect();
    let checks = Checks {
        header_hash_bound: true,
        transactions_root_bound: true,
        ancestor_headers_linked: w.ancestor_headers.len(),
        state_root_matches: true,
        storage_tries_verified: w.parent_state.storage_tries.len(),
        bytecodes_supplied: w.bytecodes.len(),
        post_state_root_matches: None,
    };
    Ok(Verified { block, db: StrictDb::new(&w.parent_state, block_hashes, bytecode_by_hash), checks })
}

fn to_exec_err<T: MissingTracker + ?Sized, E: std::fmt::Display>(db: &T, e: E) -> VerifyError {
    match db.first_missing() {
        Some(m) => VerifyError::Incomplete(m),
        None => VerifyError::Execution(e.to_string()),
    }
}

fn run_replay(
    env: &Envelope,
    w: &ReplayWitness,
    witness_sha256: [u8; 32],
    opts: &RunOptions,
) -> Result<Receipt, VerifyError> {
    let Verified { block, db, checks } = verify_replay(env, w)?;
    let exec = execute_replay(&block, w.target_index, &w.watch, &w.price_feeds, w.aave_oracle, env.chain_id, opts.full_diff, &db)?;
    let header = &block.header;
    let log = db.log.borrow();
    let anchor = &w.ancestor_headers[0];
    Ok(Receipt {
        schema: RECEIPT_SCHEMA,
        verifier: verifier_info(),
        mode: "replay",
        chain_id: env.chain_id,
        env: env_info(header, env.claimed_block_hash, exec.spec_name),
        state_anchor: AnchorInfo {
            block_number: anchor.number,
            block_hash: header.parent_hash,
            state_root: anchor.state_root,
        },
        quorum: env.quorum.clone(),
        witness_sha256: hex::encode(witness_sha256),
        bundle_commitment: exec.commitment,
        manifest_hash: impact::manifest_hash(&w.price_feeds, w.aave_oracle, &w.watch),
        checks,
        target: Some(exec.target),
        impact: Some(exec.impact),
        bundle: None,
        access: AccessStats {
            accounts_read: log.accounts.len(),
            slots_read: log.slots.len(),
            codes_read: log.codes.len(),
            block_hashes_read: log.block_hashes.len(),
        },
    })
}

/// Result of executing a replay against any database (strict witness DB in the verifier, a
/// lazily-fetching recorder on the host: same code path, so the trace and the check agree).
#[derive(Debug, Clone)]
pub struct ReplayExec {
    pub spec_name: String,
    pub target: TargetOutcome,
    pub impact: Impact,
    pub commitment: B256,
}

pub fn execute_replay<D>(
    block: &Block,
    target_index: u32,
    watch_in: &[Address],
    price_feeds: &[PriceFeed],
    aave_oracle: Option<Address>,
    chain_id: u64,
    full_diff: bool,
    db: &D,
) -> Result<ReplayExec, VerifyError>
where
    D: DatabaseRef + MissingTracker + std::fmt::Debug,
    D::Error: DBErrorMarker + std::error::Error + Send + Sync + 'static,
{
    let header = block.header.clone();
    let n_txs = block.body.transactions.len();
    let target = target_index as usize;
    if target >= n_txs {
        return Err(VerifyError::TargetOutOfRange(target_index, n_txs));
    }

    let spec = chain_spec(chain_id)?;
    let blob_params = reth_chainspec::EthChainSpec::blob_params_at_timestamp(&*spec, header.timestamp);
    revm::install_crypto(CustomCrypto::default());
    let evm_config = EthEvmConfig::new_with_evm_factory(spec, CustomEvmFactory::new(None));
    let evm_env = evm_config.evm_env(&header).map_err(|e| VerifyError::Execution(e.to_string()))?;
    let spec_name = format!("{:?}", evm_env.cfg_env.spec);

    let sealed_block = SealedBlock::seal_slow(block.clone());
    let mut state = State::builder().with_database(WrapDatabaseRef(db)).with_bundle_update().build();

    let mut target_outcome: Option<TargetOutcome> = None;
    let mut impact_out: Option<Impact> = None;
    let mut commitment_input = Vec::with_capacity(32 * (target + 1));
    {
        let mut executor = evm_config
            .executor_for_block(&mut state, &sealed_block)
            .map_err(|e| VerifyError::Execution(e.to_string()))?;
        executor.apply_pre_execution_changes().map_err(|e| to_exec_err(db, e))?;

        for (i, tx) in block.body.transactions.iter().enumerate().take(target + 1) {
            let signer = tx.recover_signer().map_err(|_| VerifyError::SignatureRecovery(i as u32))?;
            let recovered: Recovered<&TransactionSigned> = Recovered::new_unchecked(tx, signer);
            commitment_input.extend_from_slice(tx.trie_hash().as_slice());
            if i < target {
                executor.execute_transaction(recovered).map_err(|e| to_exec_err(db, e))?;
                continue;
            }

            // ---- target transaction ----
            let mut watch: Vec<Address> = watch_in.to_vec();
            if !watch.contains(&signer) {
                watch.insert(0, signer);
            }
            let eth_before = balances(executor.evm_mut().db_mut(), &watch).map_err(|e| to_exec_err(db, e))?;

            let out = executor.execute_transaction_without_commit(recovered).map_err(|e| to_exec_err(db, e))?;
            let (logs, success, diff) = {
                let r = out.result();
                (r.result.logs().to_vec(), r.result.is_success(), state_diff(&r.state))
            };

            // Pre-state probes (state not yet committed): token balances, decimals, prices.
            let tokens = impact::discover_tokens(&logs, &watch);
            let probe = |evm: &mut _, to: Address, data: Bytes| probe_call(evm, to, data);
            let mut pre_bal: BTreeMap<(Address, Address), U256> = BTreeMap::new();
            let mut decimals: BTreeMap<Address, Option<u8>> = BTreeMap::new();
            for &t in &tokens {
                let d = probe(executor.evm_mut(), t, impact::calldata_decimals())
                    .map_err(|e| to_exec_err(db, e))?
                    .and_then(|o| impact::word(&o, 0))
                    .and_then(|v| u8::try_from(v).ok());
                decimals.insert(t, d);
                for &a in &watch {
                    let b = probe(executor.evm_mut(), t, impact::calldata_balance_of(a))
                        .map_err(|e| to_exec_err(db, e))?
                        .and_then(|o| impact::word(&o, 0))
                        .unwrap_or_default();
                    pre_bal.insert((a, t), b);
                }
            }
            let mut prices = Vec::new();
            let mut price_of: BTreeMap<Address, (I256, u8)> = BTreeMap::new();
            for pf in price_feeds {
                let relevant = pf.token == Address::ZERO || tokens.contains(&pf.token);
                if !relevant {
                    continue;
                }
                let fdec = probe(executor.evm_mut(), pf.feed, impact::calldata_decimals())
                    .map_err(|e| to_exec_err(db, e))?
                    .and_then(|o| impact::word(&o, 0))
                    .and_then(|v| u8::try_from(v).ok());
                let round = probe(executor.evm_mut(), pf.feed, impact::calldata_latest_round_data())
                    .map_err(|e| to_exec_err(db, e))?;
                if let (Some(fdec), Some(o)) = (fdec, round) {
                    let answer = impact::word(&o, 1).map(I256::from_raw).unwrap_or_default();
                    let updated_at = impact::word(&o, 3).map(|v| v.to::<u64>()).unwrap_or_default();
                    let age = header.timestamp as i64 - updated_at as i64;
                    let stale = age > impact::MAX_PRICE_AGE_SECONDS || answer <= I256::ZERO;
                    if !stale {
                        price_of.insert(pf.token, (answer, fdec));
                    }
                    prices.push(PriceObservation {
                        token: pf.token,
                        feed: pf.feed,
                        answer: answer.to_string(),
                        feed_decimals: fdec,
                        updated_at,
                        age_seconds: age,
                        stale,
                    });
                }
            }

            // Fallback oracle for tokens without a direct feed (same pre-target proven state).
            if let Some(oracle) = aave_oracle {
                for &t in &tokens {
                    if price_of.contains_key(&t) {
                        continue;
                    }
                    let out = probe(executor.evm_mut(), oracle, impact::calldata_get_asset_price(t))
                        .map_err(|e| to_exec_err(db, e))?;
                    if let Some(p) = out.and_then(|o| impact::word(&o, 0)).filter(|p| !p.is_zero()) {
                        let answer = I256::from_raw(p);
                        price_of.insert(t, (answer, 8));
                        prices.push(PriceObservation {
                            token: t,
                            feed: oracle,
                            answer: answer.to_string(),
                            feed_decimals: 8,
                            updated_at: 0,
                            age_seconds: 0,
                            stale: false,
                        });
                    }
                }
            }

            let _gas = executor.commit_transaction(out);

            // Post-state probes.
            let eth_after = balances(executor.evm_mut().db_mut(), &watch).map_err(|e| to_exec_err(db, e))?;
            let mut post_bal: BTreeMap<(Address, Address), U256> = BTreeMap::new();
            for &t in &tokens {
                for &a in &watch {
                    let b = probe_call(executor.evm_mut(), t, impact::calldata_balance_of(a))
                        .map_err(|e| to_exec_err(db, e))?
                        .and_then(|o| impact::word(&o, 0))
                        .unwrap_or_default();
                    post_bal.insert((a, t), b);
                }
            }

            let receipts = executor.receipts();
            let rc = receipts.last().expect("receipt of target");
            let cumulative = rc.cumulative_gas_used();
            let prev = if target == 0 { 0 } else { receipts[target - 1].cumulative_gas_used() };
            let gas_used = cumulative - prev;
            let eff_price = tx.effective_gas_price(header.base_fee_per_gas);
            let mut fee = U256::from(gas_used) * U256::from(eff_price);
            if let (Some(blob_gas), Some(blob_fee)) =
                (tx.blob_gas_used(), blob_params.and_then(|bp| header.blob_fee(bp)))
            {
                fee += U256::from(blob_gas) * U256::from(blob_fee);
            }

            let mut logs_rlp = Vec::new();
            logs.encode(&mut logs_rlp);
            let diff_keccak = diff_commitment(&diff);
            target_outcome = Some(TargetOutcome {
                index: target_index,
                tx_hash: tx.trie_hash(),
                from: signer,
                to: tx.to(),
                success,
                gas_used,
                cumulative_gas_used: cumulative,
                logs: logs.len(),
                logs_keccak: keccak256(&logs_rlp),
                accounts_changed: diff.accounts.len(),
                slots_changed: diff.slots.len(),
                diff_keccak,
                diff: if full_diff { Some(diff) } else { None },
            });

            // Assemble impact per watched account.
            let log_deltas = impact::deltas_from_logs(&logs, &watch);
            let mut accounts = Vec::new();
            for (k, &a) in watch.iter().enumerate() {
                let eth_delta = I256::from_raw(eth_after[k]) - I256::from_raw(eth_before[k]);
                let eth_ex_fee = if a == signer { eth_delta + I256::from_raw(fee) } else { eth_delta };
                let mut total = I256::ZERO;
                let mut unpriced = Vec::new();
                if eth_ex_fee != I256::ZERO {
                    match price_of.get(&Address::ZERO) {
                        Some((ans, fd)) => total += impact::usd_micro(eth_ex_fee, 18, *ans, *fd).unwrap_or_default(),
                        None => unpriced.push(Address::ZERO),
                    }
                }
                let mut token_deltas = Vec::new();
                for &t in &tokens {
                    let by_bal = I256::from_raw(post_bal[&(a, t)]) - I256::from_raw(pre_bal[&(a, t)]);
                    let by_logs = log_deltas.get(&(a, t)).copied().unwrap_or_default();
                    if by_bal == I256::ZERO && by_logs == I256::ZERO {
                        continue;
                    }
                    let dec = decimals.get(&t).copied().flatten();
                    let usd = match (price_of.get(&t), dec) {
                        (Some((ans, fd)), Some(d)) => impact::usd_micro(by_bal, d, *ans, *fd),
                        _ => None,
                    };
                    match usd {
                        Some(u) => total += u,
                        None => unpriced.push(t),
                    }
                    token_deltas.push(TokenDelta {
                        token: t,
                        decimals: dec,
                        delta_by_balance_of: by_bal.to_string(),
                        delta_by_logs: by_logs.to_string(),
                        agree: by_bal == by_logs,
                        usd_micro: usd.map(|u| u.to_string()),
                        usd: usd.map(impact::fmt_usd),
                    });
                }
                accounts.push(AccountImpact {
                    account: a,
                    eth_delta_wei: eth_delta.to_string(),
                    eth_delta_excluding_fee_wei: eth_ex_fee.to_string(),
                    tokens: token_deltas,
                    usd_micro_total: total.to_string(),
                    usd_total: impact::fmt_usd(total),
                    unpriced_assets: unpriced,
                });
            }
            impact_out = Some(Impact { accounts, prices, sender_fee_wei: fee.to_string() });
        }
    }

    match (target_outcome, impact_out) {
        (Some(target), Some(impact)) => Ok(ReplayExec { spec_name, target, impact, commitment: keccak256(&commitment_input) }),
        _ => Err(VerifyError::Execution("target transaction not executed".into())),
    }
}

fn env_info(header: &Header, hash: B256, spec: String) -> EnvInfo {
    EnvInfo {
        block_number: header.number,
        block_hash: hash,
        timestamp: header.timestamp,
        base_fee: header.base_fee_per_gas,
        prevrandao: header.mix_hash,
        coinbase: header.beneficiary,
        gas_limit: header.gas_limit,
        excess_blob_gas: header.excess_blob_gas,
        spec,
    }
}

fn balances<DB: Database>(db: &mut DB, who: &[Address]) -> Result<Vec<U256>, DB::Error> {
    who.iter().map(|a| db.basic(*a).map(|i| i.map(|i| i.balance).unwrap_or_default())).collect()
}

/// Executes a read-only call against the current (uncommitted) state. Returns the output bytes on
/// success, `None` if the callee reverted/halted (e.g. a token without `decimals()`).
fn probe_call<E: Evm>(evm: &mut E, to: Address, data: Bytes) -> Result<Option<Bytes>, String>
where
    E::Error: std::fmt::Display,
{
    match evm.transact_system_call(PROBE_CALLER, to, data) {
        Ok(rs) => Ok(if rs.result.is_success() { rs.result.output().cloned() } else { None }),
        Err(e) => Err(e.to_string()),
    }
}

fn state_diff(state: &EvmState) -> StateDiff {
    let mut accounts = Vec::new();
    let mut slots = Vec::new();
    for (addr, acc) in state.iter().sorted_by_key(|(a, _)| **a) {
        if !acc.is_touched() {
            continue;
        }
        let before = &acc.original_info;
        let changed = before.balance != acc.info.balance
            || before.nonce != acc.info.nonce
            || before.code_hash != acc.info.code_hash
            || acc.is_selfdestructed();
        if changed {
            accounts.push(AccountChange {
                address: *addr,
                balance_before: before.balance.to_string(),
                balance_after: acc.info.balance.to_string(),
                nonce_before: before.nonce,
                nonce_after: acc.info.nonce,
                code_changed: before.code_hash != acc.info.code_hash,
                destroyed: acc.is_selfdestructed(),
            });
        }
        for (slot, v) in acc.storage.iter().sorted_by_key(|(s, _)| **s) {
            if v.is_changed() {
                slots.push(SlotChange {
                    address: *addr,
                    slot: B256::from(*slot),
                    before: B256::from(v.original_value()),
                    after: B256::from(v.present_value()),
                });
            }
        }
    }
    StateDiff { accounts, slots }
}

fn diff_commitment(d: &StateDiff) -> B256 {
    let mut buf = Vec::new();
    for a in &d.accounts {
        buf.extend_from_slice(a.address.as_slice());
        buf.extend_from_slice(a.balance_after.as_bytes());
        buf.extend_from_slice(&a.nonce_after.to_be_bytes());
    }
    for s in &d.slots {
        buf.extend_from_slice(s.address.as_slice());
        buf.extend_from_slice(s.slot.as_slice());
        buf.extend_from_slice(s.after.as_slice());
    }
    keccak256(&buf)
}

/// Full-block stateless validation with rsp's client executor (post-state root must match).
fn run_block(env: &Envelope, input: EthClientExecutorInput, witness_sha256: [u8; 32]) -> Result<Receipt, VerifyError> {
    let header = input.current_block.header.clone();
    let computed = header.hash_slow();
    if computed != env.claimed_block_hash {
        return Err(VerifyError::HeaderHashMismatch { claimed: env.claimed_block_hash, computed });
    }
    fork_guard(&header, env.chain_id)?;
    let spec = chain_spec(env.chain_id)?;
    let evm_config = EthEvmConfig::new_with_evm_factory(spec.clone(), CustomEvmFactory::new(None));
    let spec_name = evm_config
        .evm_env(&header)
        .map(|e| format!("{:?}", e.cfg_env.spec))
        .unwrap_or_default();
    let parent = input.ancestor_headers[0].clone();
    let n_storage = input.parent_state.storage_tries.len();
    let n_codes = input.bytecodes.len();
    let n_anc = input.ancestor_headers.len();
    let mut commitment_input = Vec::new();
    for tx in &input.current_block.body.transactions {
        commitment_input.extend_from_slice(tx.trie_hash().as_slice());
    }
    let executor = EthClientExecutor::eth(spec, input.custom_beneficiary);
    let derived = executor.execute(input).map_err(|e| VerifyError::Witness(e.to_string()))?;
    let post_ok = derived.state_root == header.state_root;
    Ok(Receipt {
        schema: RECEIPT_SCHEMA,
        verifier: verifier_info(),
        mode: "block",
        chain_id: env.chain_id,
        env: env_info(&header, computed, spec_name),
        state_anchor: AnchorInfo { block_number: parent.number, block_hash: header.parent_hash, state_root: parent.state_root },
        quorum: env.quorum.clone(),
        witness_sha256: hex::encode(witness_sha256),
        bundle_commitment: keccak256(&commitment_input),
        manifest_hash: B256::ZERO,
        checks: Checks {
            header_hash_bound: true,
            transactions_root_bound: true,
            ancestor_headers_linked: n_anc,
            state_root_matches: true,
            storage_tries_verified: n_storage,
            bytecodes_supplied: n_codes,
            post_state_root_matches: Some(post_ok),
        },
        target: None,
        impact: None,
        bundle: None,
        access: AccessStats { accounts_read: 0, slots_read: 0, codes_read: 0, block_hashes_read: 0 },
    })
}

/// Bundle mode: anchor header M (hash bound), state = post-state of M, env = header M.
fn run_bundle(env: &Envelope, w: &BundleWitness, witness_sha256: [u8; 32]) -> Result<Receipt, VerifyError> {
    let header = Header::decode(&mut w.header_rlp.as_ref()).map_err(|e| VerifyError::Decode(e.to_string()))?;
    let computed = header.hash_slow();
    if computed != env.claimed_block_hash {
        return Err(VerifyError::HeaderHashMismatch { claimed: env.claimed_block_hash, computed });
    }
    fork_guard(&header, env.chain_id)?;
    let sealed: Vec<SealedHeader> = once(SealedHeader::seal_slow(header.clone()))
        .chain(w.ancestor_headers.iter().map(|h| SealedHeader::seal_slow(h.clone())))
        .collect();
    build_trie_db(&w.state, header.state_root, w.bytecodes.iter(), &sealed)
        .map_err(|e| VerifyError::Witness(e.to_string()))?;
    let mut block_hashes: HashMap<u64, B256> = HashMap::default();
    for (child, parent) in sealed.iter().tuple_windows() {
        block_hashes.insert(parent.number(), child.parent_hash());
    }
    let bytecode_by_hash: HashMap<B256, &Bytecode> = w.bytecodes.iter().map(|c| (c.hash_slow(), c)).collect();
    let db = StrictDb::new(&w.state, block_hashes, bytecode_by_hash);
    let spec = chain_spec(env.chain_id)?;
    let spec_name = EthEvmConfig::new_with_evm_factory(spec.clone(), CustomEvmFactory::new(None))
        .evm_env(&header)
        .map(|e| format!("{:?}", e.cfg_env.spec))
        .unwrap_or_default();
    let out = crate::bundle::execute_bundle(&header, &w.bundle, &w.protected, &w.price_feeds, w.aave_oracle, spec, &db)?;
    let mut commit = b"antibody/bundle/v1".to_vec();
    commit.extend(bincode::serialize(&w.bundle).unwrap_or_default());
    let log = db.log.borrow();
    Ok(Receipt {
        schema: RECEIPT_SCHEMA,
        verifier: verifier_info(),
        mode: "bundle",
        chain_id: env.chain_id,
        env: env_info(&header, computed, spec_name),
        state_anchor: AnchorInfo { block_number: header.number, block_hash: computed, state_root: header.state_root },
        quorum: env.quorum.clone(),
        witness_sha256: hex::encode(witness_sha256),
        bundle_commitment: keccak256(&commit),
        manifest_hash: impact::manifest_hash(&w.price_feeds, w.aave_oracle, &w.protected),
        checks: Checks {
            header_hash_bound: true,
            transactions_root_bound: false,
            ancestor_headers_linked: w.ancestor_headers.len(),
            state_root_matches: true,
            storage_tries_verified: w.state.storage_tries.len(),
            bytecodes_supplied: w.bytecodes.len(),
            post_state_root_matches: None,
        },
        target: None,
        impact: None,
        bundle: Some(out),
        access: AccessStats {
            accounts_read: log.accounts.len(),
            slots_read: log.slots.len(),
            codes_read: log.codes.len(),
            block_hashes_read: log.block_hashes.len(),
        },
    })
}

// ------------------------------------------------------------------------------------------------
// v0.4 program mode
// ------------------------------------------------------------------------------------------------

/// Per-run context supplied by the caller (in the TEE: by the near-antibody contract's request).
#[derive(Debug, Clone, Default)]
pub struct ProgramContext {
    pub program_id: String,
    pub commit_id: u64,
    /// Manifest hash committed on NEAR; the witness's manifest bytes must hash to it.
    pub expected_manifest_hash: Option<B256>,
}

/// Chain specs for program mode: Sepolia, or a local Osaka dev chain (anvil, chain id 31337).
pub fn program_chain_spec(chain_id: u64) -> Result<Arc<reth_chainspec::ChainSpec>, VerifyError> {
    match chain_id {
        11155111 => chain_spec(chain_id),
        31337 => Ok(Arc::new(
            // Empty genesis: building from mainnet's genesis alloc would hash ~9k accounts per run.
            reth_chainspec::ChainSpecBuilder::mainnet()
                .chain(reth_chainspec::Chain::from_id(31337))
                .genesis(alloy_genesis::Genesis::default())
                .osaka_activated()
                .build(),
        )),
        other => Err(VerifyError::UnsupportedChain(other)),
    }
}

pub fn poc_hash(bundle: &crate::bundle::Bundle) -> [u8; 32] {
    let mut m = b"antibody/poc/v1".to_vec();
    m.extend(bincode::serialize(bundle).unwrap_or_default());
    crate::facts::sha256(&m)
}

pub struct ProgramExec {
    pub manifest: crate::manifest::Manifest,
    pub manifest_hash: B256,
    pub anchor: crate::facts::ProgramAnchor,
    pub checks: crate::facts::ProgramChecks,
    pub run: crate::program::ProgramRun,
}

/// Binds the witness to the manifest and the anchor header, verifies every proof, then runs the
/// PoC. Errors propagate (the witness builder resolves `Incomplete` in its fixpoint loop).
pub fn execute_program_witness(
    env: &Envelope,
    w: &ProgramWitness,
    expected_manifest_hash: Option<B256>,
) -> Result<ProgramExec, crate::program::ProgramError> {
    use crate::{
        facts::{Kind, ProgramAnchor, ProgramChecks},
        manifest::Manifest,
        program::{execute_program, ProgramError},
    };
    let bad = |k: Kind, r: String| ProgramError::Verdict { kind: k, reason: r };
    let manifest_hash = Manifest::hash(&w.manifest_json);
    if let Some(expected) = expected_manifest_hash {
        if expected != manifest_hash {
            return Err(bad(Kind::InvalidPoC, format!("manifest hash {manifest_hash} != committed {expected}")));
        }
    }
    let manifest = Manifest::parse(&w.manifest_json).map_err(|e| bad(Kind::ProgramMisconfigured, e))?;
    if env.chain_id != manifest.anchor.chain_id || env.claimed_block_hash != manifest.anchor.block_hash {
        return Err(bad(Kind::InvalidPoC, "witness is not anchored at the program's committed block".into()));
    }
    if keccak256(&manifest.invariants.runtime_code) != manifest.invariants.runtime_code_hash {
        return Err(bad(Kind::ProgramMisconfigured, "manifest invariants code does not match its hash".into()));
    }
    if w.invariants_code != manifest.invariants.runtime_code {
        return Err(bad(Kind::InvalidPoC, "invariants code does not match the manifest".into()));
    }
    let header = Header::decode(&mut w.header_rlp.as_ref()).map_err(|e| VerifyError::Decode(e.to_string()))?;
    let computed = header.hash_slow();
    if computed != env.claimed_block_hash {
        return Err(VerifyError::HeaderHashMismatch { claimed: env.claimed_block_hash, computed }.into());
    }
    if header.number != manifest.anchor.block_number {
        return Err(bad(Kind::InvalidPoC, "anchor header number differs from the manifest".into()));
    }
    let mut ancestors = Vec::with_capacity(w.ancestor_headers_rlp.len());
    for rlp in &w.ancestor_headers_rlp {
        ancestors.push(Header::decode(&mut rlp.as_ref()).map_err(|e| VerifyError::Decode(e.to_string()))?);
    }
    let sealed: Vec<SealedHeader> = once(SealedHeader::seal_slow(header.clone()))
        .chain(ancestors.into_iter().map(SealedHeader::seal_slow))
        .collect();
    build_trie_db(&w.state, header.state_root, w.bytecodes.iter(), &sealed)
        .map_err(|e| VerifyError::Witness(e.to_string()))?;
    let mut block_hashes: HashMap<u64, B256> = HashMap::default();
    for (child, parent) in sealed.iter().tuple_windows() {
        block_hashes.insert(parent.number(), child.parent_hash());
    }
    let bytecode_by_hash: HashMap<B256, &Bytecode> = w.bytecodes.iter().map(|c| (c.hash_slow(), c)).collect();
    let db = StrictDb::new(&w.state, block_hashes, bytecode_by_hash);
    let spec = program_chain_spec(manifest.anchor.chain_id)?;
    let inv_code = Bytecode::new_raw(w.invariants_code.clone());
    let run = execute_program(&header, &manifest, &inv_code, &w.bundle, spec, &db)?;
    Ok(ProgramExec {
        anchor: ProgramAnchor {
            chain_id: manifest.anchor.chain_id,
            block_number: header.number,
            block_hash: computed,
            state_root: header.state_root,
            timestamp: header.timestamp,
            evm_spec: manifest.anchor.evm_spec.clone(),
        },
        checks: ProgramChecks {
            header_hash_bound: true,
            state_root_matches: true,
            storage_tries_verified: w.state.storage_tries.len(),
            bytecodes_supplied: w.bytecodes.len(),
            ancestor_headers_linked: w.ancestor_headers_rlp.len(),
            reserved_address_empty: run.reserved_address_empty,
            target_code_hashes_match: run.target_code_hashes_match,
            anchor_invariants_hold: true,
        },
        manifest,
        manifest_hash,
        run,
    })
}

/// Program-mode entry point: always returns a receipt with a typed verdict.
pub fn run_program(
    env: &Envelope,
    w: &ProgramWitness,
    witness_sha256: [u8; 32],
    ctx: &ProgramContext,
) -> crate::facts::ProgramReceipt {
    use crate::facts::{Kind, ProgramReceipt, RECEIPT_V2_SCHEMA};
    let base = |kind: Kind, reason: Option<String>| ProgramReceipt {
        schema: RECEIPT_V2_SCHEMA,
        verifier: verifier_info(),
        program: ctx.program_id.clone(),
        commit_id: ctx.commit_id,
        manifest_hash: crate::manifest::Manifest::hash(&w.manifest_json),
        invariants_code_hash: keccak256(&w.invariants_code),
        anchor: None,
        witness_sha256: hex::encode(witness_sha256),
        poc_hash: hex::encode(poc_hash(&w.bundle)),
        kind,
        reason,
        checks: None,
        senders: Vec::new(),
        endowment_wei_per_sender: w.bundle.endowment_wei.to_string(),
        calls: Vec::new(),
        created_contracts: Vec::new(),
        total_gas_used: 0,
        evaluations: 0,
        broken_mask: "0x0".into(),
        broken: Vec::new(),
        impact: None,
    };
    if env.version != WITNESS_VERSION {
        return base(Kind::InvalidPoC, Some(format!("unsupported witness version {}", env.version)));
    }
    match execute_program_witness(env, w, ctx.expected_manifest_hash) {
        Ok(x) => {
            let kind = if x.run.broken_mask.is_zero() { Kind::NoViolation } else { Kind::Violation };
            ProgramReceipt {
                anchor: Some(x.anchor),
                checks: Some(x.checks),
                senders: x.run.senders,
                calls: x.run.calls,
                created_contracts: x.run.created,
                total_gas_used: x.run.total_gas,
                evaluations: x.run.evaluations,
                broken_mask: format!("{:#x}", x.run.broken_mask),
                broken: x.run.broken,
                impact: Some(x.run.impact),
                ..base(kind, None)
            }
        }
        Err(e) => {
            let (kind, reason) = crate::program::classify(&e);
            base(kind, Some(reason))
        }
    }
}

/// A receipt for submissions rejected before any witness could be read (TEE mode: blob hash or
/// key-wrap mismatch, undecryptable blob, payload bound to another hunter).
pub fn failure_receipt(
    ctx: &ProgramContext,
    kind: crate::facts::Kind,
    reason: &str,
    manifest_hash: B256,
    witness_sha256: [u8; 32],
) -> crate::facts::ProgramReceipt {
    crate::facts::ProgramReceipt {
        schema: crate::facts::RECEIPT_V2_SCHEMA,
        verifier: verifier_info(),
        program: ctx.program_id.clone(),
        commit_id: ctx.commit_id,
        manifest_hash,
        invariants_code_hash: B256::ZERO,
        anchor: None,
        witness_sha256: hex::encode(witness_sha256),
        poc_hash: String::new(),
        kind,
        reason: Some(reason.to_string()),
        checks: None,
        senders: Vec::new(),
        endowment_wei_per_sender: "0".into(),
        calls: Vec::new(),
        created_contracts: Vec::new(),
        total_gas_used: 0,
        evaluations: 0,
        broken_mask: "0x0".into(),
        broken: Vec::new(),
        impact: None,
    }
}

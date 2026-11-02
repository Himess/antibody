//! Bundle mode: the actual Antibody workload. A researcher's bundle of calls is executed with
//! `eth_call`-at-M semantics: state = post-state of anchored block M, environment = header M.
//!
//! Sender model (fixes "impersonate an admin"): calls can only come from fresh addresses derived
//! as `keccak256("antibody/sender/v1" ‖ salt ‖ i)[12..]` from a researcher-held secret salt. Each
//! sender must be provably empty at M (absent, or nonce 0, balance 0, no code). The only state
//! override is a bounded ETH endowment credited to those senders; it is excluded from impact.

use std::collections::{BTreeMap, BTreeSet};

use alloy_consensus::Header;
use alloy_evm::Evm;
use alloy_primitives::{keccak256, Address, Bytes, TxKind, B256, I256, U256};
use reth_evm::ConfigureEvm;
use reth_evm_ethereum::EthEvmConfig;
use revm::{
    context::TxEnv,
    database::{State, WrapDatabaseRef},
    database_interface::DBErrorMarker,
    primitives::KECCAK_EMPTY,
    Database, DatabaseRef,
};
use rsp_client_executor::custom::{CustomCrypto, CustomEvmFactory};
use serde::{Deserialize, Serialize};

use crate::{
    db::MissingTracker,
    impact::{self, PROBE_CALLER},
    receipt::{AccountImpact, PriceObservation, TokenDelta},
    run::VerifyError,
    witness::PriceFeed,
};

/// Maximum endowment per sender (policy): 1000 ETH.
pub const MAX_ENDOWMENT_WEI: u128 = 1_000_000_000_000_000_000_000;
/// EIP-7825 per-transaction gas cap (Osaka).
pub const MAX_CALL_GAS: u64 = 16_777_216;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleCall {
    pub sender: u8,
    /// `None` = contract creation.
    pub to: Option<Address>,
    pub value: U256,
    pub data: Bytes,
    pub gas_limit: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bundle {
    /// Researcher-held secret; only its hash-derived senders may act.
    pub sender_salt: B256,
    pub senders: u8,
    pub endowment_wei: u128,
    pub calls: Vec<BundleCall>,
}

pub fn sender_address(salt: &B256, i: u8) -> Address {
    let mut buf = b"antibody/sender/v1".to_vec();
    buf.extend_from_slice(salt.as_slice());
    buf.push(i);
    Address::from_slice(&keccak256(&buf)[12..])
}

#[derive(Debug, Clone, Serialize)]
pub struct CallOutcome {
    pub index: usize,
    pub sender: Address,
    pub to: Option<Address>,
    pub success: bool,
    pub gas_used: u64,
    pub logs: usize,
    pub created: Option<Address>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BundleOutcome {
    pub senders: Vec<Address>,
    pub endowment_wei_per_sender: String,
    pub calls: Vec<CallOutcome>,
    pub created_contracts: Vec<Address>,
    /// Attacker closure = senders ∪ contracts created by the bundle.
    pub closure: Vec<Address>,
    pub total_gas_used: u64,
    pub fees_paid_wei: String,
    pub accounts: Vec<AccountImpact>,
    pub prices: Vec<PriceObservation>,
    /// Sum over the closure, excluding the endowment and fees (micro-USD, priced assets only).
    pub closure_gain_usd_micro: String,
    pub closure_gain_usd: String,
    /// Sum over watched non-closure accounts (e.g. manifest-protected contracts).
    pub protected_delta_usd_micro: String,
    pub protected_delta_usd: String,
}

fn probe<E: Evm>(evm: &mut E, to: Address, data: Bytes) -> Result<Option<Bytes>, String>
where
    E::Error: std::fmt::Display,
{
    match evm.transact_system_call(PROBE_CALLER, to, data) {
        Ok(rs) => Ok(if rs.result.is_success() { rs.result.output().cloned() } else { None }),
        Err(e) => Err(e.to_string()),
    }
}

fn to_err<T: MissingTracker + ?Sized, E: std::fmt::Display>(db: &T, e: E) -> VerifyError {
    match db.first_missing() {
        Some(m) => VerifyError::Incomplete(m),
        None => VerifyError::Execution(e.to_string()),
    }
}

/// Executes the bundle at anchored header `header` over `db` (state = post-state of `header`).
#[allow(clippy::too_many_arguments)]
pub fn execute_bundle<D>(
    header: &Header,
    bundle: &Bundle,
    protected: &[Address],
    price_feeds: &[PriceFeed],
    aave_oracle: Option<Address>,
    chain_spec: std::sync::Arc<reth_chainspec::ChainSpec>,
    db: &D,
) -> Result<BundleOutcome, VerifyError>
where
    D: DatabaseRef + MissingTracker + std::fmt::Debug,
    D::Error: DBErrorMarker + std::error::Error + Send + Sync + 'static,
{
    if bundle.endowment_wei > MAX_ENDOWMENT_WEI {
        return Err(VerifyError::Decode("endowment above policy cap".into()));
    }
    if bundle.senders == 0 || bundle.calls.is_empty() {
        return Err(VerifyError::Decode("empty bundle".into()));
    }
    revm::install_crypto(CustomCrypto::default());
    let evm_config = EthEvmConfig::new_with_evm_factory(chain_spec, CustomEvmFactory::new(None));
    let senders: Vec<Address> = (0..bundle.senders).map(|i| sender_address(&bundle.sender_salt, i)).collect();
    let base_fee = header.base_fee_per_gas.unwrap_or_default() as u128;

    // --- execution state (post-state of M + endowment) ---
    let mut state = State::builder().with_database(WrapDatabaseRef(db)).with_bundle_update().build();
    for s in &senders {
        let acc = state.basic(*s).map_err(|e| to_err(db, e))?;
        if let Some(a) = acc {
            if a.nonce != 0 || !a.balance.is_zero() || a.code_hash != KECCAK_EMPTY {
                return Err(VerifyError::Decode(format!("sender {s} is not a fresh empty account at M")));
            }
        }
    }
    // The only state override: credit the bounded endowment to the (proven-empty) senders.
    for s in &senders {
        state.insert_account(
            *s,
            revm::state::AccountInfo {
                balance: U256::from(bundle.endowment_wei),
                nonce: 0,
                code_hash: KECCAK_EMPTY,
                code: None,
                account_id: None,
            },
        );
    }
    let eth_before: Vec<U256> = senders.iter().map(|_| U256::from(bundle.endowment_wei)).collect();

    let mut evm = evm_config.evm_for_block(&mut state, header).map_err(|e| VerifyError::Execution(e.to_string()))?;
    let mut logs = Vec::new();
    let mut calls = Vec::new();
    let mut created = Vec::new();
    let mut nonces: BTreeMap<Address, u64> = BTreeMap::new();
    let mut total_gas = 0u64;
    for (i, c) in bundle.calls.iter().enumerate() {
        let sender = *senders.get(c.sender as usize).ok_or_else(|| VerifyError::Decode("bad sender index".into()))?;
        let nonce = *nonces.get(&sender).unwrap_or(&0);
        let tx = TxEnv {
            caller: sender,
            gas_limit: c.gas_limit.min(MAX_CALL_GAS),
            gas_price: base_fee,
            kind: match c.to {
                Some(a) => TxKind::Call(a),
                None => TxKind::Create,
            },
            value: c.value,
            data: c.data.clone(),
            nonce,
            chain_id: Some(evm.chain_id()),
            ..Default::default()
        };
        let res = evm.transact_commit(tx).map_err(|e| to_err(db, e))?;
        nonces.insert(sender, nonce + 1);
        let gas = res.tx_gas_used();
        total_gas += gas;
        let new_addr = if c.to.is_none() { res.created_address() } else { None };
        if let Some(a) = new_addr {
            created.push(a);
        }
        calls.push(CallOutcome {
            index: i,
            sender,
            to: c.to,
            success: res.is_success(),
            gas_used: gas,
            logs: res.logs().len(),
            created: new_addr,
        });
        logs.extend(res.into_logs());
    }
    let fees = U256::from(total_gas) * U256::from(base_fee);

    // Closure and watch list: senders ∪ created contracts ∪ protected contracts.
    let closure: Vec<Address> = senders.iter().chain(created.iter()).copied().collect();
    let mut watch: Vec<Address> = closure.clone();
    for p in protected {
        if !watch.contains(p) {
            watch.push(*p);
        }
    }
    let tokens = impact::discover_tokens(&logs, &watch);

    // Post-state probes on the execution state.
    let mut post_bal: BTreeMap<(Address, Address), U256> = BTreeMap::new();
    let mut eth_after: BTreeMap<Address, U256> = BTreeMap::new();
    for &a in &watch {
        let b = evm.db_mut().basic(a).map_err(|e| to_err(db, e))?.map(|i| i.balance).unwrap_or_default();
        eth_after.insert(a, b);
        for &t in &tokens {
            let v = probe(&mut evm, t, impact::calldata_balance_of(a))
                .map_err(|e| to_err(db, e))?
                .and_then(|o| impact::word(&o, 0))
                .unwrap_or_default();
            post_bal.insert((a, t), v);
        }
    }
    drop(evm);

    // Pre-state probes on a FRESH state over the same proven DB: exactly the anchored pre-state.
    let mut pre_state = State::builder().with_database(WrapDatabaseRef(db)).build();
    let mut pre = evm_config.evm_for_block(&mut pre_state, header).map_err(|e| VerifyError::Execution(e.to_string()))?;
    let mut pre_bal: BTreeMap<(Address, Address), U256> = BTreeMap::new();
    let mut eth_pre: BTreeMap<Address, U256> = BTreeMap::new();
    let mut decimals: BTreeMap<Address, Option<u8>> = BTreeMap::new();
    for &a in &watch {
        let b = pre.db_mut().basic(a).map_err(|e| to_err(db, e))?.map(|i| i.balance).unwrap_or_default();
        eth_pre.insert(a, b);
    }
    for (k, s) in senders.iter().enumerate() {
        eth_pre.insert(*s, eth_before[k]); // endowment is the sender's baseline
    }
    for &t in &tokens {
        let d = probe(&mut pre, t, impact::calldata_decimals())
            .map_err(|e| to_err(db, e))?
            .and_then(|o| impact::word(&o, 0))
            .and_then(|v| u8::try_from(v).ok());
        decimals.insert(t, d);
        for &a in &watch {
            let v = probe(&mut pre, t, impact::calldata_balance_of(a))
                .map_err(|e| to_err(db, e))?
                .and_then(|o| impact::word(&o, 0))
                .unwrap_or_default();
            pre_bal.insert((a, t), v);
        }
    }
    let mut prices = Vec::new();
    let mut price_of: BTreeMap<Address, (I256, u8)> = BTreeMap::new();
    for pf in price_feeds {
        if !(pf.token == Address::ZERO || tokens.contains(&pf.token)) || price_of.contains_key(&pf.token) {
            continue;
        }
        let fdec = probe(&mut pre, pf.feed, impact::calldata_decimals())
            .map_err(|e| to_err(db, e))?
            .and_then(|o| impact::word(&o, 0))
            .and_then(|v| u8::try_from(v).ok());
        let round = probe(&mut pre, pf.feed, impact::calldata_latest_round_data()).map_err(|e| to_err(db, e))?;
        if let (Some(fd), Some(o)) = (fdec, round) {
            let answer = impact::word(&o, 1).map(I256::from_raw).unwrap_or_default();
            let updated_at = impact::word(&o, 3).map(|v| v.to::<u64>()).unwrap_or_default();
            let age = header.timestamp as i64 - updated_at as i64;
            let stale = age > impact::MAX_PRICE_AGE_SECONDS || answer <= I256::ZERO;
            if !stale {
                price_of.insert(pf.token, (answer, fd));
            }
            prices.push(PriceObservation {
                token: pf.token,
                feed: pf.feed,
                answer: answer.to_string(),
                feed_decimals: fd,
                updated_at,
                age_seconds: age,
                stale,
            });
        }
    }
    if let Some(oracle) = aave_oracle {
        for &t in &tokens {
            if price_of.contains_key(&t) {
                continue;
            }
            let out = probe(&mut pre, oracle, impact::calldata_get_asset_price(t)).map_err(|e| to_err(db, e))?;
            if let Some(p) = out.and_then(|o| impact::word(&o, 0)).filter(|p| !p.is_zero()) {
                price_of.insert(t, (I256::from_raw(p), 8));
                prices.push(PriceObservation {
                    token: t,
                    feed: oracle,
                    answer: p.to_string(),
                    feed_decimals: 8,
                    updated_at: 0,
                    age_seconds: 0,
                    stale: false,
                });
            }
        }
    }
    drop(pre);

    let log_deltas = impact::deltas_from_logs(&logs, &watch);
    let closure_set: BTreeSet<Address> = closure.iter().copied().collect();
    let (mut closure_total, mut protected_total) = (I256::ZERO, I256::ZERO);
    let mut accounts = Vec::new();
    for &a in &watch {
        let eth_delta = I256::from_raw(eth_after[&a]) - I256::from_raw(eth_pre[&a]);
        // Fees are paid by senders out of the endowment; exclude them from "gain".
        let eth_ex_fee = if senders.contains(&a) && a == senders[0] { eth_delta + I256::from_raw(fees) } else { eth_delta };
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
        if closure_set.contains(&a) {
            closure_total += total;
        } else {
            protected_total += total;
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
    Ok(BundleOutcome {
        senders,
        endowment_wei_per_sender: bundle.endowment_wei.to_string(),
        calls,
        created_contracts: created,
        closure,
        total_gas_used: total_gas,
        fees_paid_wei: fees.to_string(),
        accounts,
        prices,
        closure_gain_usd_micro: closure_total.to_string(),
        closure_gain_usd: impact::fmt_usd(closure_total),
        protected_delta_usd_micro: protected_total.to_string(),
        protected_delta_usd: impact::fmt_usd(protected_total),
    })
}

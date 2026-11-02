//! Impact measurement helpers: token discovery from logs, ABI probes, signed deltas, USD value.

use alloy_primitives::{address, b256, Address, Bytes, Log, B256, I256, U256};
use std::collections::{BTreeMap, BTreeSet};

/// keccak("Transfer(address,address,uint256)")
pub const TRANSFER: B256 = b256!("ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");
/// keccak("Deposit(address,uint256)") (WETH9)
pub const WETH_DEPOSIT: B256 = b256!("e1fffcc4923d04b559f4d29a8bfc6cda04eb5b0d3c460751c2402c5c5cc9109c");
/// keccak("Withdrawal(address,uint256)") (WETH9)
pub const WETH_WITHDRAWAL: B256 = b256!("7fcf532c15f0a6db0bd6d0e038bea71d30d808c7d98cb3bf7268a95bf5081b65");

pub const WETH: Address = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

/// Price observations older than this (relative to the header timestamp) are not used.
/// Chainlink stablecoin feeds heartbeat at 24 h; ETH/USD at 1 h with 0.5 % deviation.
pub const MAX_PRICE_AGE_SECONDS: i64 = 90_000;

pub fn manifest_hash(feeds: &[crate::witness::PriceFeed], oracle: Option<Address>, accounts: &[Address]) -> B256 {
    let mut buf = b"antibody/manifest/v1".to_vec();
    for f in feeds {
        buf.extend_from_slice(f.token.as_slice());
        buf.extend_from_slice(f.feed.as_slice());
    }
    buf.push(0xff);
    if let Some(o) = oracle {
        buf.extend_from_slice(o.as_slice());
    }
    buf.push(0xfe);
    for a in accounts {
        buf.extend_from_slice(a.as_slice());
    }
    alloy_primitives::keccak256(&buf)
}

/// Caller used for read-only probes (balanceOf, decimals, latestRoundData). Probes are executed
/// as system calls and never committed.
pub const PROBE_CALLER: Address = address!("00000000000000000000000000000000000a11b0");

pub fn topic_address(t: &B256) -> Address {
    Address::from_slice(&t[12..])
}

pub fn word(data: &[u8], i: usize) -> Option<U256> {
    data.get(i * 32..(i + 1) * 32).map(U256::from_be_slice)
}

/// Signed token-unit deltas implied by ERC-20 Transfer (and WETH9 Deposit/Withdrawal) logs for
/// the watched accounts. Keyed by (account, token).
pub fn deltas_from_logs(logs: &[Log], watch: &[Address]) -> BTreeMap<(Address, Address), I256> {
    let watched: BTreeSet<Address> = watch.iter().copied().collect();
    let mut out: BTreeMap<(Address, Address), I256> = BTreeMap::new();
    for log in logs {
        let topics = log.data.topics();
        if topics.is_empty() {
            continue;
        }
        let token = log.address;
        let amount = |idx: usize| -> Option<I256> {
            word(&log.data.data, idx).map(|v| I256::from_raw(v))
        };
        if topics[0] == TRANSFER && topics.len() == 3 {
            // ERC-20 (ERC-721 Transfer has 4 topics and is ignored here)
            let (from, to) = (topic_address(&topics[1]), topic_address(&topics[2]));
            if let Some(v) = amount(0) {
                if watched.contains(&from) {
                    *out.entry((from, token)).or_default() -= v;
                }
                if watched.contains(&to) {
                    *out.entry((to, token)).or_default() += v;
                }
            }
        } else if token == WETH && topics.len() == 2 && (topics[0] == WETH_DEPOSIT || topics[0] == WETH_WITHDRAWAL) {
            let who = topic_address(&topics[1]);
            if watched.contains(&who) {
                if let Some(v) = amount(0) {
                    if topics[0] == WETH_DEPOSIT {
                        *out.entry((who, token)).or_default() += v;
                    } else {
                        *out.entry((who, token)).or_default() -= v;
                    }
                }
            }
        }
    }
    out
}

/// Tokens that moved to/from a watched account in these logs.
pub fn discover_tokens(logs: &[Log], watch: &[Address]) -> BTreeSet<Address> {
    deltas_from_logs(logs, watch).keys().map(|(_, t)| *t).collect()
}

pub fn calldata_balance_of(who: Address) -> Bytes {
    let mut v = Vec::with_capacity(36);
    v.extend_from_slice(&[0x70, 0xa0, 0x82, 0x31]);
    v.extend_from_slice(&[0u8; 12]);
    v.extend_from_slice(who.as_slice());
    v.into()
}

pub fn calldata_decimals() -> Bytes {
    Bytes::from_static(&[0x31, 0x3c, 0xe5, 0x67])
}

/// AaveOracle.getAssetPrice(address) -> uint256 (USD, 8 decimals)
pub fn calldata_get_asset_price(asset: Address) -> Bytes {
    let mut v = Vec::with_capacity(36);
    v.extend_from_slice(&[0xb3, 0x59, 0x6f, 0x07]);
    v.extend_from_slice(&[0u8; 12]);
    v.extend_from_slice(asset.as_slice());
    v.into()
}

pub fn calldata_latest_round_data() -> Bytes {
    Bytes::from_static(&[0xfe, 0xaf, 0x96, 0x8c])
}

/// USD value in micro-dollars (integer, deterministic) of a signed token delta at a feed price:
/// delta * answer * 1e6 / (10^token_decimals * 10^feed_decimals). No floating point in receipts.
pub fn usd_micro(delta: I256, token_decimals: u8, answer: I256, feed_decimals: u8) -> Option<I256> {
    let scale = |d: u8| I256::try_from(U256::from(10u8).pow(U256::from(d))).ok();
    let num = delta.checked_mul(answer)?.checked_mul(I256::try_from(1_000_000u64).ok()?)?;
    num.checked_div(scale(token_decimals)?)?.checked_div(scale(feed_decimals)?)
}

/// Formats micro-dollars as a decimal string with 2 decimals (display only).
pub fn fmt_usd(micro: I256) -> String {
    let neg = micro.is_negative();
    let abs = micro.unsigned_abs();
    let cents = abs / U256::from(10_000u64);
    let s = format!("{}.{:02}", cents / U256::from(100u64), (cents % U256::from(100u64)).to::<u64>());
    if neg {
        format!("-{s}")
    } else {
        s
    }
}

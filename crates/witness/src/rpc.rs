//! Minimal multi-provider JSON-RPC client (blocking) with retries and per-provider statistics.
//! URLs from `.env` are never printed; providers are referred to by label.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use eyre::{eyre, Result};
use serde_json::{json, Value};

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Stats {
    pub http_requests: u64,
    pub rpc_calls: u64,
    pub errors: u64,
    pub retries: u64,
    pub bytes_in: u64,
    pub ms: u64,
}

#[derive(Debug)]
pub struct Provider {
    pub label: String,
    url: String,
    client: reqwest::blocking::Client,
    pub stats: Mutex<Stats>,
    /// Max JSON-RPC batch size this provider accepted (1 = no batching).
    pub batch_size: Mutex<usize>,
}

impl Provider {
    pub fn new(label: &str, url: &str) -> Self {
        Self {
            label: label.to_string(),
            url: url.to_string(),
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(60))
                .user_agent("antibody-witness/0.1")
                .build()
                .expect("http client"),
            stats: Mutex::new(Stats::default()),
            batch_size: Mutex::new(20),
        }
    }

    fn post(&self, body: &Value) -> Result<Value> {
        self.post_via(&self.client, body)
    }

    /// A client that opens a new connection per request: a load balancer then picks a backend afresh.
    fn unpooled() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("antibody-witness/0.1")
            .pool_max_idle_per_host(0)
            .build()
            .expect("http client")
    }

    fn post_via(&self, client: &reqwest::blocking::Client, body: &Value) -> Result<Value> {
        let mut last = eyre!("no attempt");
        for attempt in 0..5 {
            let t0 = Instant::now();
            let res = client.post(&self.url).json(body).send();
            let mut st = self.stats.lock().unwrap();
            st.http_requests += 1;
            match res {
                Ok(r) => {
                    let status = r.status();
                    let bytes = r.bytes().unwrap_or_default();
                    st.ms += t0.elapsed().as_millis() as u64;
                    st.bytes_in += bytes.len() as u64;
                    if status.as_u16() == 429 || status.is_server_error() {
                        st.retries += 1;
                        drop(st);
                        last = eyre!("{} HTTP {}", self.label, status);
                        std::thread::sleep(Duration::from_millis(400 * (1 << attempt)));
                        continue;
                    }
                    if !status.is_success() {
                        st.errors += 1;
                        return Err(eyre!("{} HTTP {}", self.label, status));
                    }
                    return serde_json::from_slice(&bytes).map_err(|e| eyre!("{} bad json: {e}", self.label));
                }
                Err(e) => {
                    st.retries += 1;
                    drop(st);
                    last = eyre!("{} transport: {e}", self.label);
                    std::thread::sleep(Duration::from_millis(400 * (1 << attempt)));
                }
            }
        }
        self.stats.lock().unwrap().errors += 1;
        Err(last)
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
        let (mut limited, mut elsewhere) = (0u32, 0u32);
        let mut fresh = None;
        loop {
            self.stats.lock().unwrap().rpc_calls += 1;
            let v = match &fresh {
                Some(c) => self.post_via(c, &body)?,
                None => self.post(&body)?,
            };
            if let Some(e) = v.get("error") {
                let msg = e.to_string().to_lowercase();
                // JSON-level rate limiting (HTTP 200): back off and retry.
                if limited < 6 && rate_limited(&msg) {
                    self.stats.lock().unwrap().retries += 1;
                    std::thread::sleep(Duration::from_millis(700 * (1 << limited.min(3))));
                    limited += 1;
                    continue;
                }
                // A load-balanced endpoint whose backends differ (some keep no recent state for proofs):
                // ask again from a new client (a new connection and TLS session, which a balancer may
                // pin to a backend), which may reach another backend.
                if elsewhere < 10 && msg.contains("proof window") {
                    self.stats.lock().unwrap().retries += 1;
                    fresh = Some(Self::unpooled());
                    std::thread::sleep(Duration::from_millis(150));
                    elsewhere += 1;
                    continue;
                }
                return Err(eyre!("{} {method}: {e}", self.label));
            }
            return v.get("result").cloned().ok_or_else(|| eyre!("{} {method}: no result", self.label));
        }
    }

    /// Batch call; falls back to sequential calls if the provider rejects batches.
    pub fn batch(&self, reqs: &[(String, Value)]) -> Vec<Result<Value>> {
        let mut out: Vec<Result<Value>> = Vec::with_capacity(reqs.len());
        let mut i = 0;
        while i < reqs.len() {
            let bs = (*self.batch_size.lock().unwrap()).max(1);
            let chunk = &reqs[i..(i + bs).min(reqs.len())];
            if bs == 1 {
                out.push(self.call(&chunk[0].0, chunk[0].1.clone()));
                i += 1;
                continue;
            }
            let body: Vec<Value> = chunk
                .iter()
                .enumerate()
                .map(|(k, (m, p))| json!({"jsonrpc":"2.0","id":k,"method":m,"params":p}))
                .collect();
            self.stats.lock().unwrap().rpc_calls += chunk.len() as u64;
            match self.post(&Value::Array(body)) {
                Ok(Value::Array(items)) if items.len() == chunk.len() => {
                    let mut by_id: Vec<Option<Value>> = vec![None; chunk.len()];
                    for it in items {
                        if let Some(id) = it.get("id").and_then(|x| x.as_u64()) {
                            if (id as usize) < chunk.len() {
                                by_id[id as usize] = Some(it);
                            }
                        }
                    }
                    for (k, it) in by_id.into_iter().enumerate() {
                        out.push(match it {
                            // A backend without the block's state, or a rate limit on part of the batch:
                            // this one again, alone (`call` retries both).
                            Some(v) if {
                                let msg = v["error"].to_string().to_lowercase();
                                msg.contains("proof window") || rate_limited(&msg)
                            } =>
                            {
                                self.call(&chunk[k].0, chunk[k].1.clone())
                            }
                            Some(v) if v.get("error").is_some() => {
                                Err(eyre!("{} {}: {}", self.label, chunk[k].0, v["error"]))
                            }
                            Some(v) => v.get("result").cloned().ok_or_else(|| eyre!("no result")),
                            None => Err(eyre!("{} missing batch item", self.label)),
                        });
                    }
                    i += chunk.len();
                }
                _ => {
                    // Shrink the batch and retry this chunk, after a pause (the refusal may be a rate limit).
                    let mut b = self.batch_size.lock().unwrap();
                    *b = if *b > 5 { 5 } else { 1 };
                    drop(b);
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        out
    }

    pub fn stats(&self) -> Stats {
        self.stats.lock().unwrap().clone()
    }
}

/// A JSON-level rate limit (public endpoints answer some with HTTP 200).
fn rate_limited(msg: &str) -> bool {
    msg.contains("too fast") || msg.contains("rate") || msg.contains("cu limit") || msg.contains("too many")
}

pub type Pool = Vec<Arc<Provider>>;

pub fn load_env(path: &std::path::Path) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Ok(s) = std::fs::read_to_string(path) {
        for line in s.lines() {
            let line = line.trim();
            if line.starts_with('#') || !line.contains('=') {
                continue;
            }
            let (k, val) = line.split_once('=').unwrap();
            let val = val.trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                v.push((k.trim().to_string(), val.to_string()));
            }
        }
    }
    v
}

/// Archive-capable public endpoints found by `scripts/probe_rpcs.py` (historical eth_getProof).
pub const PUBLIC_ARCHIVE: &[(&str, &str)] = &[
    ("zan", "https://api.zan.top/eth-mainnet"),
    ("blast", "https://eth-mainnet.public.blastapi.io"),
    ("mevblocker", "https://rpc.mevblocker.io"),
];

/// Recent-state-only public endpoints (proof window of a few blocks): header quorum and recent
/// proofs only.
pub const PUBLIC_RECENT: &[(&str, &str)] = &[
    ("publicnode", "https://ethereum-rpc.publicnode.com"),
    ("drpc", "https://eth.drpc.org"),
    ("tenderly", "https://mainnet.gateway.tenderly.co"),
    ("blxr", "https://eth.rpc.blxrbdn.com"),
    ("stakely", "https://ethereum-json-rpc.stakely.io"),
    ("blockrazor", "https://eth.blockrazor.xyz"),
];

/// Providers: private ones from `.env` (ETH_RPC_*) first, then public archive endpoints, then
/// (optionally) recent-only ones.
pub fn providers(env_file: &std::path::Path, include_recent: bool) -> Pool {
    let mut pool: Pool = Vec::new();
    for (k, v) in load_env(env_file) {
        if k.starts_with("ETH_RPC_") {
            pool.push(Arc::new(Provider::new(&format!("private:{}", &k[8..]), &v)));
        }
    }
    for (l, u) in PUBLIC_ARCHIVE {
        pool.push(Arc::new(Provider::new(l, u)));
    }
    if include_recent {
        for (l, u) in PUBLIC_RECENT {
            pool.push(Arc::new(Provider::new(l, u)));
        }
    }
    pool
}

pub fn hex_u64(v: &Value) -> Result<u64> {
    let s = v.as_str().ok_or_else(|| eyre!("not a hex string: {v}"))?;
    u64::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| eyre!("{e}"))
}

//! Minimal NEAR client: JSON-RPC views, borsh-encoded transactions signed with ed25519, key files.
//! Transactions use the classic (V0) layout every NEAR RPC accepts.

use base64::Engine as _;
use ed25519_dalek::{Signer as _, SigningKey};
use eyre::{eyre, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const TGAS: u64 = 1_000_000_000_000;
pub const ONE_YOCTO: u128 = 1;
pub const ONE_NEAR: u128 = 1_000_000_000_000_000_000_000_000;
/// Attempts for a transaction the primary refuses on its rate limit (other calls stop after seven).
const SEND_RATE_LIMIT_ATTEMPTS: u64 = 16;
/// How long reads leave the primary alone after it answered a rate limit.
const PRIMARY_COOL_SECS: u64 = 30;
/// Where testnet reads go when the primary rate-limits and `$NEAR_RPC_EXTRA` is unset (set it, even empty, to
/// choose): public RPCs that answered views without a key on 2026-11-09. Transactions never go to them.
pub const DEFAULT_TESTNET_READ_FALLBACKS: [&str; 2] = ["https://near-testnet.drpc.org", "https://testnet-rpc.intea.rs"];

#[derive(Clone)]
pub struct Near {
    /// The primary RPC (the one shown; it never carries a key).
    pub rpc: String,
    pub network: String,
    /// Further RPCs, tried in turn when one fails or rate-limits (`$NEAR_RPC_EXTRA`, comma-separated:
    /// for instance a keyed FastNear URL or another provider). Never printed: they may carry keys.
    extra: Vec<String>,
    /// `$NEAR_RPC_API_KEY`: sent as a bearer token to FastNear hosts. Never printed.
    bearer: Option<String>,
    client: reqwest::blocking::Client,
    /// Last nonce used per (account, public key), shared by clones: back-to-back and concurrent
    /// transactions from one key must not reuse a nonce read from a lagging view.
    nonces: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
    /// Until when the primary is left to transactions, shared by clones: it answered a rate limit, so reads
    /// start at the extra RPCs meanwhile (only with `$NEAR_RPC_EXTRA` set).
    cool_until: std::sync::Arc<std::sync::Mutex<Option<std::time::Instant>>>,
}

/// A NEAR account and one of its keys (full access or function call).
#[derive(Clone)]
pub struct Signer {
    pub account_id: String,
    pub key: SigningKey,
}

#[derive(Debug, Clone)]
pub enum Action {
    CreateAccount,
    DeployContract(Vec<u8>),
    FunctionCall { method: String, args: Vec<u8>, gas: u64, deposit: u128 },
    Transfer(u128),
    AddKey { public_key: [u8; 32], permission: Permission },
    DeleteKey { public_key: [u8; 32] },
    DeleteAccount { beneficiary: String },
}

#[derive(Debug, Clone)]
pub enum Permission {
    FullAccess,
    FunctionCall { allowance: Option<u128>, receiver_id: String, method_names: Vec<String> },
}

fn b_str(o: &mut Vec<u8>, s: &str) {
    o.extend((s.len() as u32).to_le_bytes());
    o.extend(s.as_bytes());
}

fn b_bytes(o: &mut Vec<u8>, b: &[u8]) {
    o.extend((b.len() as u32).to_le_bytes());
    o.extend(b);
}

fn b_pk(o: &mut Vec<u8>, pk: &[u8; 32]) {
    o.push(0); // ED25519
    o.extend(pk);
}

impl Action {
    fn borsh(&self, o: &mut Vec<u8>) {
        match self {
            Action::CreateAccount => o.push(0),
            Action::DeployContract(code) => {
                o.push(1);
                b_bytes(o, code);
            }
            Action::FunctionCall { method, args, gas, deposit } => {
                o.push(2);
                b_str(o, method);
                b_bytes(o, args);
                o.extend(gas.to_le_bytes());
                o.extend(deposit.to_le_bytes());
            }
            Action::Transfer(deposit) => {
                o.push(3);
                o.extend(deposit.to_le_bytes());
            }
            Action::AddKey { public_key, permission } => {
                o.push(5);
                b_pk(o, public_key);
                o.extend(0u64.to_le_bytes()); // access key nonce
                match permission {
                    Permission::FunctionCall { allowance, receiver_id, method_names } => {
                        o.push(0);
                        match allowance {
                            Some(a) => {
                                o.push(1);
                                o.extend(a.to_le_bytes());
                            }
                            None => o.push(0),
                        }
                        b_str(o, receiver_id);
                        o.extend((method_names.len() as u32).to_le_bytes());
                        for m in method_names {
                            b_str(o, m);
                        }
                    }
                    Permission::FullAccess => o.push(1),
                }
            }
            Action::DeleteKey { public_key } => {
                o.push(6);
                b_pk(o, public_key);
            }
            Action::DeleteAccount { beneficiary } => {
                o.push(7);
                b_str(o, beneficiary);
            }
        }
    }
}

/// borsh(Transaction V0).
pub fn encode_tx(signer_id: &str, public_key: &[u8; 32], nonce: u64, receiver_id: &str, block_hash: &[u8; 32], actions: &[Action]) -> Vec<u8> {
    let mut o = Vec::new();
    b_str(&mut o, signer_id);
    b_pk(&mut o, public_key);
    o.extend(nonce.to_le_bytes());
    b_str(&mut o, receiver_id);
    o.extend(block_hash);
    o.extend((actions.len() as u32).to_le_bytes());
    for a in actions {
        a.borsh(&mut o);
    }
    o
}

/// borsh(SignedTransaction): the transaction followed by an ed25519 signature over sha256(tx).
pub fn sign_tx(tx: &[u8], key: &SigningKey) -> (Vec<u8>, [u8; 32]) {
    let hash: [u8; 32] = Sha256::digest(tx).into();
    let sig = key.sign(&hash);
    let mut o = tx.to_vec();
    o.push(0);
    o.extend(sig.to_bytes());
    (o, hash)
}

impl Signer {
    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    pub fn public_key_str(&self) -> String {
        format!("ed25519:{}", bs58::encode(self.public_key()).into_string())
    }

    /// near-cli key file: `{"account_id", "public_key": "ed25519:…", "private_key": "ed25519:<base58 64 bytes>"}`.
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| eyre!("{}: {e}", path.display()))?)?;
        let account_id = v["account_id"].as_str().ok_or_else(|| eyre!("key file: no account_id"))?.to_string();
        let sk = v["private_key"].as_str().or(v["secret_key"].as_str()).ok_or_else(|| eyre!("key file: no private_key"))?;
        let raw = bs58::decode(sk.trim_start_matches("ed25519:")).into_vec()?;
        let seed: [u8; 32] = raw.get(..32).ok_or_else(|| eyre!("key file: short key"))?.try_into()?;
        Ok(Self { account_id, key: SigningKey::from_bytes(&seed) })
    }

    pub fn to_file_json(&self) -> Value {
        let mut full = self.key.to_bytes().to_vec();
        full.extend(self.public_key());
        json!({"account_id": self.account_id, "public_key": self.public_key_str(),
               "private_key": format!("ed25519:{}", bs58::encode(full).into_string())})
    }

    /// A key from a known seed (the practice program's deliberately public vend key).
    pub fn from_seed(account_id: String, seed: [u8; 32]) -> Self {
        Self { account_id, key: SigningKey::from_bytes(&seed) }
    }

    pub fn generate(account_id: Option<String>) -> Result<Self> {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).map_err(|e| eyre!("rng: {e}"))?;
        let key = SigningKey::from_bytes(&seed);
        let account_id = account_id.unwrap_or_else(|| hex::encode(key.verifying_key().to_bytes()));
        Ok(Self { account_id, key })
    }
}

impl Near {
    pub fn new(network: &str, rpc: Option<String>) -> Self {
        let rpc = rpc.unwrap_or_else(|| match network {
            "mainnet" => "https://rpc.mainnet.fastnear.com".into(),
            _ => "https://rpc.testnet.fastnear.com".into(),
        });
        // A second RPC (and a key) for the regular endpoint; archival lookups stay archival. Unset on testnet,
        // reads fall back to public RPCs that answered when FastNEAR's free tier rate-limited (2026-11-09):
        // transactions still go to the primary only.
        let extra = if rpc.contains("archival") {
            vec![]
        } else {
            let list = match std::env::var("NEAR_RPC_EXTRA") {
                Ok(v) => v,
                Err(_) if network == "testnet" => DEFAULT_TESTNET_READ_FALLBACKS.join(","),
                Err(_) => String::new(),
            };
            list.split(',').map(str::trim).filter(|u| !u.is_empty() && *u != rpc).map(String::from).collect()
        };
        let bearer = std::env::var("NEAR_RPC_API_KEY").ok().filter(|k| !k.trim().is_empty()).map(|k| k.trim().to_string());
        Self {
            rpc,
            network: network.into(),
            extra,
            bearer,
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(90))
                .user_agent("antibody-cli/0.4")
                .build()
                .expect("http client"),
            nonces: Default::default(),
            cool_until: Default::default(),
        }
    }

    /// The first RPC a read tries: an extra one while the primary cools down after a rate limit, so the
    /// reads of a busy run (polls, balances, nonces) leave the primary's budget to the transactions,
    /// which go to it only. Otherwise the primary, whose answers include the writes it just took.
    fn read_start(&self) -> usize {
        let cooling = self.cool_until.lock().map(|c| c.is_some_and(|t| std::time::Instant::now() < t)).unwrap_or(false);
        usize::from(cooling && !self.extra.is_empty())
    }

    /// The primary answered a rate limit: reads start elsewhere for the next half minute.
    fn cool_primary(&self) {
        if let Ok(mut c) = self.cool_until.lock() {
            *c = Some(std::time::Instant::now() + std::time::Duration::from_secs(PRIMARY_COOL_SECS));
        }
    }

    /// The RPC for an attempt: the primary, then each extra one in turn.
    fn endpoint(&self, attempt: usize) -> &str {
        match attempt % (1 + self.extra.len()) {
            0 => &self.rpc,
            i => &self.extra[i - 1],
        }
    }

    fn post(&self, url: &str, body: &Value) -> reqwest::Result<Value> {
        let mut req = self.client.post(url).json(body);
        if let (Some(k), true) = (&self.bearer, url.contains("fastnear")) {
            req = req.bearer_auth(k);
        }
        req.send().and_then(|r| r.json::<Value>())
    }

    /// An attempt that got no answer at all (no connection, no DNS, a timeout): which RPC, by its label, what
    /// failed, and how to use another. The HTTP client's error names the URL, which may carry a key: never
    /// in a message (SA-31).
    fn no_answer(&self, method: &str, url: &str, e: reqwest::Error) -> eyre::Report {
        let role = match self.extra.iter().position(|x| x == url) {
            Some(i) => format!("extra RPC {} of $NEAR_RPC_EXTRA", i + 1),
            None => "the NEAR RPC".to_string(),
        };
        let e = e.without_url();
        let mut why = e.to_string();
        let mut source = std::error::Error::source(&e);
        while let Some(s) = source {
            why.push_str(": ");
            why.push_str(&s.to_string());
            source = s.source();
        }
        eyre!("{method}: no answer from {role} {} ({why}). Use another NEAR RPC with --rpc <url> or $NEAR_RPC", rpc_label(url))
    }

    /// How many RPCs this client tries (the primary and `$NEAR_RPC_EXTRA`'s).
    pub fn endpoints(&self) -> usize {
        1 + self.extra.len()
    }

    /// One attempt, no retry: for queries whose error is an answer (a height with no block).
    pub fn call_once(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({"jsonrpc": "2.0", "id": "antibody", "method": method, "params": params});
        let v = self.post(&self.rpc, &body).map_err(|e| self.no_answer(method, &self.rpc, e))?;
        match v.get("error") {
            Some(e) => Err(eyre!("{method}: {e}")),
            None => Ok(v["result"].clone()),
        }
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut last = eyre!("no attempt");
        // A transaction goes to the primary RPC only. Another RPC may take it and time out without saying
        // whether it was sent (a free plan's "Request timeout", 2026-11-08), and an answer that the nonce is
        // used then has `send` sign it again with the next one: a second execution. The primary's rate limit
        // refuses a request outright (nothing sent), so the same transaction waits and goes again.
        let sends = method == "send_tx";
        let endpoints = if sends { 1 } else { self.endpoints() };
        let start = if sends { 0 } else { self.read_start() };
        let body = json!({"jsonrpc": "2.0", "id": "antibody", "method": method, "params": params});
        for attempt in 0..SEND_RATE_LIMIT_ATTEMPTS {
            // A failed or rate-limited attempt goes to the next RPC (one RPC: the same, after a pause).
            let url = if sends { self.rpc.as_str() } else { self.endpoint(start + attempt as usize) };
            let (limited, pause) = match self.post(url, &body) {
                Ok(v) => {
                    let limited = if let Some(e) = v.get("error") {
                        let s = e.to_string();
                        // A retired endpoint answers -429 to everything: no retry makes it answer. A read goes on to
                        // the next RPC; a transaction, or the only RPC there is, says so at once.
                        if deprecated(&s) {
                            let e = eyre!(
                                "{method}: {} is a retired NEAR RPC ({s}). Leave out --rpc for the default, or pass another provider's (docs.near.org/api/rpc/providers)",
                                rpc_label(url)
                            );
                            if sends || endpoints == 1 {
                                return Err(e);
                            }
                            last = e;
                            continue;
                        }
                        // Transient conditions worth a retry: a rate limit (FastNEAR answers -429, a request
                        // it refused, so a transaction was not sent) backs off longer.
                        if !transient(&s) {
                            return Err(eyre!("{method}: {s}"));
                        }
                        last = eyre!("{method}: {s}");
                        rate_limited(&s)
                    } else if let Some(r) = v.get("result") {
                        return Ok(r.clone());
                    } else {
                        // A body with neither `result` nor `error` is not a JSON-RPC answer (a gateway's
                        // rate-limit page, say): nothing was answered, and for a transaction nothing was sent.
                        let s = v.to_string();
                        last = eyre!("{method}: not a JSON-RPC answer: {}", s.chars().take(200).collect::<String>());
                        rate_limited(&s)
                    };
                    if limited && url == self.rpc {
                        self.cool_primary();
                    }
                    (limited, retry_pause_ms(attempt, endpoints, limited))
                }
                Err(e) => {
                    last = self.no_answer(method, url, e);
                    (false, 800 * (attempt + 1))
                }
            };
            // Seven attempts. A transaction the primary keeps refusing on its rate limit waits longer (about
            // four minutes in all): nothing was sent, and a stage that stopped there would start again.
            let next = attempt + 1;
            if next >= SEND_RATE_LIMIT_ATTEMPTS || (next >= 7 && !(sends && limited)) {
                break;
            }
            // A wait long enough to look like a hang says what it waits for (the long commands' progress lines).
            if limited && pause >= 2_000 {
                crate::progress::step(format!(
                    "{method}: the public RPC is rate-limiting (429); trying again in {} s{}",
                    pause.div_ceil(1000),
                    if sends { ", the same transaction (nothing was sent)" } else { "" }
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(pause));
        }
        Err(last)
    }

    /// Views read optimistic state, so a command sees the writes of the transaction it just sent
    /// (`send` returns at EXECUTED_OPTIMISTIC; "final" lags by a few blocks).
    pub fn view_raw(&self, contract: &str, method: &str, args: &[u8]) -> Result<Vec<u8>> {
        let r = self.call(
            "query",
            json!({"request_type": "call_function", "finality": "optimistic", "account_id": contract, "method_name": method,
                   "args_base64": base64::engine::general_purpose::STANDARD.encode(args)}),
        )?;
        if let Some(e) = r.get("error") {
            return Err(eyre!("view {contract}.{method}: {e}"));
        }
        let bytes: Vec<u8> = r["result"]
            .as_array()
            .ok_or_else(|| eyre!("view {contract}.{method}: no result"))?
            .iter()
            .map(|b| b.as_u64().unwrap_or(0) as u8)
            .collect();
        Ok(bytes)
    }

    pub fn view(&self, contract: &str, method: &str, args: Value) -> Result<Value> {
        let bytes = self.view_raw(contract, method, args.to_string().as_bytes())?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn account(&self, account_id: &str) -> Result<Option<Value>> {
        match self.call("query", json!({"request_type": "view_account", "finality": "optimistic", "account_id": account_id})) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.to_string().contains("UNKNOWN_ACCOUNT") || e.to_string().contains("does not exist") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Signs and sends `actions` to `receiver`, waiting for `wait_until` ("EXECUTED_OPTIMISTIC", "FINAL").
    pub fn send(&self, signer: &Signer, receiver: &str, actions: Vec<Action>, wait_until: &str) -> Result<Value> {
        let pk = signer.public_key();
        let key = format!("{}:{}", signer.account_id, signer.public_key_str());
        // The nonce from the latest state (a key added a moment ago is there), the block hash from a final
        // block: every node of a load-balanced RPC knows it (an optimistic one may reach a node that has not
        // seen it, which refuses the transaction as expired).
        let ak = self.call(
            "query",
            json!({"request_type": "view_access_key", "finality": "optimistic", "account_id": signer.account_id,
                   "public_key": signer.public_key_str()}),
        )?;
        let hash_of = |s: &Value| -> Result<[u8; 32]> {
            bs58::decode(s.as_str().unwrap_or_default()).into_vec()?.try_into().map_err(|_| eyre!("block hash"))
        };
        let final_hash = || -> Result<[u8; 32]> { hash_of(&self.call("block", json!({"finality": "final"}))?["header"]["hash"]) };
        let mut block_hash = final_hash()?;
        let mut nonce = ak["nonce"].as_u64().ok_or_else(|| eyre!("access key not found for {}", signer.account_id))? + 1;
        for attempt in 0..4 {
            {
                let mut m = self.nonces.lock().expect("nonce cache");
                if let Some(&last) = m.get(&key) {
                    nonce = nonce.max(last + 1);
                }
                m.insert(key.clone(), nonce);
            }
            let tx = encode_tx(&signer.account_id, &pk, nonce, receiver, &block_hash, &actions);
            let (signed, hash) = sign_tx(&tx, &signer.key);
            match self.call(
                "send_tx",
                json!({"signed_tx_base64": base64::engine::general_purpose::STANDARD.encode(&signed), "wait_until": wait_until}),
            ) {
                Err(e) if attempt < 3 && e.to_string().contains("InvalidNonce") => {
                    // `call` sends the same signed bytes again after an attempt with no answer: if one of them
                    // ran, its nonce is the one now used. That transaction's outcome is the answer, never a
                    // second one signed with the next nonce.
                    let h = bs58::encode(hash).into_string();
                    if let Ok(done) = self.call("tx", json!({"tx_hash": h, "sender_account_id": signer.account_id, "wait_until": wait_until})) {
                        return Ok(done);
                    }
                    // The node tells us the key's current nonce: continue from there.
                    let s = e.to_string();
                    if let Some(n) = s.split("\"ak_nonce\":").nth(1).and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next()).and_then(|d| d.parse::<u64>().ok()) {
                        nonce = n + 1;
                    } else {
                        nonce += 1;
                    }
                }
                // Refused before it ran: again with the latest block's hash first (right after a
                // fast-forward the final block can be older than the validity window: the sandbox's
                // rounds), then a fresh final one.
                Err(e) if attempt < 3 && e.to_string().contains("\"Expired\"") => {
                    block_hash = if attempt == 0 { hash_of(&ak["block_hash"])? } else { final_hash()? };
                }
                other => return other,
            }
        }
        Err(eyre!("send_tx: nonce conflict for {}", signer.account_id))
    }

    pub fn function_call(&self, signer: &Signer, receiver: &str, method: &str, args: Value, gas_tgas: u64, deposit: u128) -> Result<Value> {
        self.send(
            signer,
            receiver,
            vec![Action::FunctionCall { method: method.into(), args: args.to_string().into_bytes(), gas: gas_tgas * TGAS, deposit }],
            "EXECUTED_OPTIMISTIC",
        )
    }

    pub fn tx_status(&self, hash: &str, signer: &str) -> Result<Value> {
        self.call("tx", json!({"tx_hash": hash, "sender_account_id": signer, "wait_until": "NONE"}))
    }

    /// NEAR's clock at the latest final block, as near-antibody reads it. A transaction sent after
    /// this read executes in a later block, so near-antibody's `now` for it is at least this.
    pub fn final_block_secs(&self) -> Result<u64> {
        block_secs(&self.call("block", json!({"finality": "final"}))?["header"])
    }

    pub fn explorer_account(&self, account: &str) -> String {
        match self.network.as_str() {
            "mainnet" => format!("https://nearblocks.io/address/{account}"),
            "testnet" => format!("https://testnet.nearblocks.io/address/{account}"),
            other => format!("{other}:{account}"),
        }
    }

    /// An account's access keys (`view_access_key_list`).
    pub fn access_keys(&self, account: &str) -> Result<Vec<Value>> {
        let r = self.call("query", json!({"request_type": "view_access_key_list", "finality": "optimistic", "account_id": account}))?;
        Ok(r["keys"].as_array().cloned().unwrap_or_default())
    }

    pub fn explorer_tx(&self, hash: &str) -> String {
        match self.network.as_str() {
            "mainnet" => format!("https://nearblocks.io/txns/{hash}"),
            "testnet" => format!("https://testnet.nearblocks.io/txns/{hash}"),
            other => format!("{other}:{hash}"),
        }
    }
}

/// A block header's time in whole seconds, rounded down as near-antibody's `now` is
/// (`block_timestamp() / 1_000_000_000`).
pub fn block_secs(header: &Value) -> Result<u64> {
    // The string first: a number this large loses precision in any JavaScript proxy on the way.
    let ns = header["timestamp_nanosec"].as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| header["timestamp"].as_u64());
    ns.map(|n| n / 1_000_000_000).ok_or_else(|| eyre!("a block header with no timestamp: {header}"))
}

/// What a message may say about an RPC (SA-31): its host and port, never its URL, whose path, query or user
/// part may carry a key. A host with a long label, as a key in a subdomain would be, is not named either.
pub fn rpc_label(url: &str) -> String {
    let Ok(u) = reqwest::Url::parse(url) else { return "(its URL does not parse)".into() };
    match u.host_str() {
        Some(h) if !h.split('.').any(|l| l.len() >= 32) => match u.port() {
            Some(p) => format!("{h}:{p}"),
            None => h.to_string(),
        },
        Some(_) => "(its host is not shown: it may carry a key)".into(),
        None => "(no host)".into(),
    }
}

/// An RPC's refusal to answer now, in any case: FastNEAR's -429, "Too Many Requests", "rate limit".
pub fn rate_limited(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    s.contains("-429") || l.contains("too many requests") || l.contains("rate limit") || l.contains("rate-limit")
}

/// A retired endpoint's answer (rpc.testnet.near.org since 2026: "THIS ENDPOINT IS DEPRECATED", as a -429).
pub fn deprecated(s: &str) -> bool {
    s.contains("ENDPOINT IS DEPRECATED")
}

/// An RPC error worth another attempt: a rate limit, a timeout, a block the node has not seen yet.
pub fn transient(s: &str) -> bool {
    rate_limited(s) || s.contains("TIMEOUT_ERROR") || s.contains("UNKNOWN_BLOCK")
}

/// How long attempt `attempt` (from 0) waits after a transient error before the next. With other RPCs to
/// turn to, a short pause, and a longer one each time every RPC has been tried: on 2026-11-08 a drill and
/// the services shared one host, and a rate limit outlasted three RPCs tried 250 ms apart. With one RPC,
/// a back-off (longer for a rate limit).
pub fn retry_pause_ms(attempt: u64, endpoints: usize, limited: bool) -> u64 {
    let n = endpoints.max(1) as u64;
    if n > 1 {
        if (attempt + 1) % n == 0 {
            1_500 * ((attempt + 1) / n)
        } else {
            250
        }
    } else if limited {
        (2_000 * (attempt + 1)).min(20_000)
    } else {
        800 * (attempt + 1)
    }
}

/// Outcome helpers over a `send_tx` result.
pub fn tx_hash(outcome: &Value) -> String {
    outcome["transaction"]["hash"].as_str().unwrap_or_default().to_string()
}

/// Decoded JSON return value of the transaction's last successful receipt, if any.
pub fn return_value(outcome: &Value) -> Option<Value> {
    let b64 = outcome["status"]["SuccessValue"].as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    if bytes.is_empty() {
        return Some(Value::Null);
    }
    serde_json::from_slice(&bytes).ok()
}

/// Every failure in the transaction tree (receipts included), for honest error reporting.
pub fn failures(outcome: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(f) = outcome["status"].get("Failure") {
        out.push(f.to_string());
    }
    for r in outcome["receipts_outcome"].as_array().into_iter().flatten() {
        if let Some(f) = r["outcome"]["status"].get("Failure") {
            out.push(f.to_string());
        }
    }
    out
}

/// NEP-297 events (`EVENT_JSON:`) logged anywhere in the transaction tree.
pub fn events(outcome: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for r in outcome["receipts_outcome"].as_array().into_iter().flatten() {
        for l in r["outcome"]["logs"].as_array().into_iter().flatten() {
            if let Some(j) = l.as_str().and_then(|s| s.strip_prefix("EVENT_JSON:")) {
                if let Ok(v) = serde_json::from_str::<Value>(j) {
                    out.push(v);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_limit_waits_longer_once_every_rpc_was_tried() {
        // Three RPCs: two short pauses, then a longer one after the round, and longer after the next.
        let p: Vec<u64> = (0..7).map(|a| retry_pause_ms(a, 3, true)).collect();
        assert_eq!(p, vec![250, 250, 1_500, 250, 250, 3_000, 250]);
        // One RPC: the back-off as before, at most 20 s a pause: a transaction refused on the rate limit
        // every time waits 210 s over its sixteen attempts.
        assert_eq!((retry_pause_ms(0, 1, true), retry_pause_ms(2, 1, false)), (2_000, 2_400));
        assert_eq!(retry_pause_ms(15, 1, true), 20_000);
        assert_eq!((0..SEND_RATE_LIMIT_ATTEMPTS - 1).map(|a| retry_pause_ms(a, 1, true)).sum::<u64>(), 210_000);
    }

    /// A rate limit on the primary sends reads to the extra RPCs for a while (clones share it), leaving the
    /// primary to transactions; with no extra RPC, reads stay on the primary.
    #[test]
    fn a_rate_limited_primary_leaves_reads_to_the_extra_rpcs() {
        let mut n = Near::new("testnet", Some("https://primary.example".into()));
        n.extra = vec!["https://extra.example".into()];
        assert_eq!(n.endpoint(n.read_start()), "https://primary.example");
        let clone = n.clone();
        n.cool_primary();
        assert_eq!(n.endpoint(n.read_start()), "https://extra.example");
        assert_eq!(clone.endpoint(clone.read_start()), "https://extra.example");
        // The next attempt after the extra one is the primary again.
        assert_eq!(n.endpoint(n.read_start() + 1), "https://primary.example");
        let mut lone = Near::new("testnet", Some("https://primary.example".into()));
        lone.extra.clear();
        lone.cool_primary();
        assert_eq!(lone.read_start(), 0);
    }

    /// An RPC is named by its host and port, never by a path, query or user part, and never by a host that
    /// may hold a key (SA-31).
    #[test]
    fn an_rpc_label_carries_no_key() {
        assert_eq!(rpc_label("https://rpc.testnet.fastnear.com/?apiKey=SECRET"), "rpc.testnet.fastnear.com");
        assert_eq!(rpc_label("https://user:SECRET@near.example.org:8443/v1/SECRET"), "near.example.org:8443");
        let keyed = rpc_label("https://0123456789abcdef0123456789abcdef.rpc.example.com/");
        assert!(!keyed.contains("0123") && keyed.contains("not shown"), "{keyed}");
        assert!(!rpc_label("not a url SECRET").contains("SECRET"));
    }

    /// An RPC that cannot be reached is named, with how to use another, and its URL's key stays out.
    /// A retired endpoint's -429 is not a rate limit to wait out.
    #[test]
    fn a_retired_rpc_is_said_at_once() {
        let s = r#"{"code":-429,"message":"WARNING! THIS ENDPOINT IS DEPRECATED! STOP USING IT NOW! Switch to https://fastnear.com"}"#;
        assert!(deprecated(s));
        assert!(!deprecated(r#"{"code":-429,"message":"Rate limits exceeded"}"#));
    }

    #[test]
    fn an_unreachable_rpc_says_which_and_how_to_change_it() {
        // Nothing listens on port 9 here: the connection is refused.
        let n = Near::new("testnet", Some("http://127.0.0.1:9/v1/SECRET?apiKey=SECRET".into()));
        let e = n.call_once("status", json!([])).unwrap_err().to_string();
        assert!(e.starts_with("status: no answer from the NEAR RPC 127.0.0.1:9 ("), "{e}");
        assert!(e.contains("--rpc <url>") && e.contains("$NEAR_RPC"), "{e}");
        assert!(!e.contains("SECRET"), "{e}");
    }

    /// A rate limit is one in any case, and a timeout or an unknown block is worth another attempt; a
    /// contract's refusal is not.
    #[test]
    fn transient_errors_are_told_apart() {
        for s in [r#"{"code":-429,"message":"Rate limits exceeded"}"#, "Too Many Requests", "rate limit exceeded", "RATE LIMIT", "rate-limit exceeded",
                  "TIMEOUT_ERROR", "UNKNOWN_BLOCK"] {
            assert!(transient(s), "{s}");
        }
        assert!(!transient(r#"{"name":"HANDLER_ERROR","cause":{"name":"UNKNOWN_ACCOUNT"}}"#));
        assert!(!transient("Smart contract panicked: not the owner"));
    }

    /// Cross-checked against near-api-js 5 (scripts/guardian/near_tx_vector.mjs) for the same inputs.
    #[test]
    fn transaction_encoding_vector() {
        let key = SigningKey::from_bytes(&[1u8; 32]);
        let actions = vec![
            Action::FunctionCall { method: "commit".into(), args: br#"{"program_id":0}"#.to_vec(), gas: 30 * TGAS, deposit: 0 },
            Action::Transfer(ONE_NEAR),
            Action::AddKey {
                public_key: [2u8; 32],
                permission: Permission::FunctionCall {
                    allowance: Some(ONE_NEAR / 4),
                    receiver_id: "bounty.antibody-hq.testnet".into(),
                    method_names: vec!["commit".into(), "submit".into()],
                },
            },
        ];
        let tx = encode_tx("agent-a.testnet", &key.verifying_key().to_bytes(), 7, "bounty.antibody-hq.testnet", &[3u8; 32], &actions);
        let (signed, hash) = sign_tx(&tx, &key);
        assert_eq!(hex::encode(hash), include_str!("../tests/near_tx_vector.hash").trim());
        assert_eq!(hex::encode(&signed), include_str!("../tests/near_tx_vector.signed").trim());
    }

    /// nearcore's `Action::DeleteKey` is variant 6, then the key (curve byte 0 for ED25519, 32 bytes).
    #[test]
    fn delete_key_encoding() {
        let mut o = vec![];
        Action::DeleteKey { public_key: [7u8; 32] }.borsh(&mut o);
        assert_eq!(o.len(), 34);
        assert_eq!(&o[..2], &[6, 0]);
        assert!(o[2..].iter().all(|b| *b == 7));
    }

    /// A block header as NEAR RPCs return it: whole seconds, rounded down like near-antibody's `now`.
    #[test]
    fn block_time_in_seconds() {
        let h = json!({"height": 276_600_000, "timestamp": 1_793_809_254_999_999_999u64, "timestamp_nanosec": "1793809254999999999"});
        assert_eq!(block_secs(&h).unwrap(), 1_793_809_254);
        assert_eq!(block_secs(&json!({"timestamp": 1_793_809_255_000_000_000u64})).unwrap(), 1_793_809_255);
        assert!(block_secs(&json!({"height": 1})).is_err());
    }

    /// nearcore's `Action::DeleteAccount` is variant 7, then the beneficiary (a borsh string).
    #[test]
    fn delete_account_encoding() {
        let mut o = vec![];
        Action::DeleteAccount { beneficiary: "antibody.testnet".into() }.borsh(&mut o);
        assert_eq!(o[0], 7);
        assert_eq!(&o[1..5], &16u32.to_le_bytes());
        assert_eq!(&o[5..], b"antibody.testnet");
    }
}

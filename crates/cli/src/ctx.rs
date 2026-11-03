//! Shared context: network, deployed contract ids, signer, Ethereum RPC, local state.

use std::path::{Path, PathBuf};

use eyre::{eyre, Result};
use serde_json::Value;

use crate::near::{Near, Signer};

/// Contract ids for one NEAR network (`deployments/near-<network>.json`).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Deployments {
    pub network: String,
    pub antibody: String,
    pub tusd: String,
    #[serde(default)]
    pub guardian: Option<String>,
    #[serde(default)]
    pub outlayer: Option<String>,
    #[serde(default)]
    pub light_client: Option<String>,
    #[serde(default)]
    pub rpc: Option<String>,
    /// A public Ethereum RPC for the anchors' chain, used when none is given (no key needed). Public
    /// nodes keep recent state only: proofs that read beyond the pinned base need it near the anchor.
    #[serde(default)]
    pub eth_rpc: Option<String>,
    /// The practice program (docs/practice.md): `antibody try` needs only a name.
    #[serde(default)]
    pub practice: Option<PracticeDeployment>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct PracticeDeployment {
    pub vending: String,
    pub program: u32,
}

/// The testnet deployment, built in: a downloaded binary needs no repository.
pub const TESTNET_DEPLOYMENTS: &str = include_str!("../../../deployments/near-testnet.json");

pub struct Ctx {
    pub near: Near,
    pub dep: Deployments,
    pub account: Option<String>,
    pub key_file: Option<PathBuf>,
    pub eth_rpc: Option<String>,
    pub state_dir: PathBuf,
}

/// Reads `KEY=value` lines of a dotenv file without printing anything.
pub fn load_dotenv(path: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.starts_with('#') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            let v = v.trim().trim_matches('"').trim_matches('\'');
            (!v.is_empty()).then(|| (k.trim().to_string(), v.to_string()))
        })
        .collect()
}

pub fn repo_root() -> Option<PathBuf> {
    let mut d = std::env::current_dir().ok()?;
    loop {
        if d.join("deployments").is_dir() && d.join("crates").is_dir() {
            return Some(d);
        }
        if !d.pop() {
            return None;
        }
    }
}

fn env_or_dotenv(key: &str) -> Option<String> {
    if let Ok(v) = std::env::var(key) {
        if !v.is_empty() {
            return Some(v);
        }
    }
    let mut files = vec![PathBuf::from(".env")];
    if let Some(r) = repo_root() {
        files.push(r.join(".env"));
    }
    files.iter().flat_map(|f| load_dotenv(f)).find(|(k, _)| k == key).map(|(_, v)| v)
}

impl Ctx {
    pub fn new(
        network: &str,
        deployments: Option<PathBuf>,
        rpc: Option<String>,
        account: Option<String>,
        key_file: Option<PathBuf>,
        eth_rpc: Option<String>,
    ) -> Result<Self> {
        let path = deployments
            .or_else(|| std::env::var("ANTIBODY_DEPLOYMENTS").ok().map(PathBuf::from))
            .or_else(|| repo_root().map(|r| r.join(format!("deployments/near-{network}.json"))));
        let text = match path {
            Some(p) => std::fs::read_to_string(&p).map_err(|e| eyre!("deployments {}: {e}", p.display()))?,
            None if network == "testnet" => TESTNET_DEPLOYMENTS.to_string(),
            None => return Err(eyre!("no deployments file: pass --deployments or set ANTIBODY_DEPLOYMENTS")),
        };
        let dep: Deployments = serde_json::from_str(&text)?;
        let rpc = rpc.or_else(|| env_or_dotenv("NEAR_RPC")).or_else(|| dep.rpc.clone());
        let near = Near::new(network, rpc);
        let eth_rpc = eth_rpc.or_else(|| env_or_dotenv("SEPOLIA_RPC")).or_else(|| dep.eth_rpc.clone());
        let state_dir = PathBuf::from(".antibody");
        Ok(Self { near, dep, account: account.or_else(|| std::env::var("ANTIBODY_ACCOUNT").ok()), key_file, eth_rpc, state_dir })
    }

    /// The signing account: `--key-file`, else `.secrets/near/<account>.json`, else near-cli's
    /// `~/.near-credentials/<network>/<account>.json`.
    pub fn signer(&self) -> Result<Signer> {
        if let Some(f) = self.key_file.clone().or_else(|| std::env::var("ANTIBODY_KEY_FILE").ok().map(PathBuf::from)) {
            let s = Signer::from_file(&f)?;
            if let Some(a) = &self.account {
                if a != &s.account_id {
                    return Ok(Signer { account_id: a.clone(), key: s.key });
                }
            }
            return Ok(s);
        }
        let account = self.account.clone().ok_or_else(|| eyre!("no account: pass --account (and --key-file)"))?;
        let mut candidates = Vec::new();
        if let Some(r) = repo_root() {
            candidates.push(r.join(".secrets/near").join(format!("{account}.json")));
        }
        candidates.push(PathBuf::from(".secrets/near").join(format!("{account}.json")));
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            candidates.push(PathBuf::from(home).join(".near-credentials").join(&self.near.network).join(format!("{account}.json")));
        }
        for c in candidates {
            if c.exists() {
                let s = Signer::from_file(&c)?;
                return Ok(Signer { account_id: account, key: s.key });
            }
        }
        Err(eyre!("no key file for {account} (pass --key-file)"))
    }

    pub fn eth_pool(&self) -> Result<antibody_witness::rpc::Pool> {
        let urls = self.eth_rpc.clone().ok_or_else(|| eyre!("no Ethereum RPC: pass --eth-rpc or set SEPOLIA_RPC (.env)"))?;
        Ok(urls
            .split(',')
            .enumerate()
            .map(|(i, u)| std::sync::Arc::new(antibody_witness::rpc::Provider::new(&format!("eth{i}"), u)))
            .collect())
    }

    pub fn view(&self, method: &str, args: Value) -> Result<Value> {
        self.near.view(&self.dep.antibody, method, args)
    }

    pub fn program(&self, id: u32) -> Result<Value> {
        let p = self.view("get_program", serde_json::json!({"program_id": id}))?;
        if p.is_null() {
            return Err(eyre!("program {id} does not exist on {}", self.dep.antibody));
        }
        Ok(p)
    }

    pub fn commit(&self, id: u64) -> Result<Value> {
        let c = self.view("get_commit", serde_json::json!({"commit_id": id}))?;
        if c.is_null() {
            return Err(eyre!("commit {id} does not exist on {}", self.dep.antibody));
        }
        Ok(c)
    }

    pub fn record_path(&self, program: u32, commit: u64) -> PathBuf {
        self.state_dir.join("submissions").join(format!("{program}-{commit}.json"))
    }
}

//! near-guardian v2: EIP-712 pause and release permits, signed with Chain Signatures under each
//! program's own derivation path (docs/spec.md §9, §10).
//!
//! - A project registers its program. The guardian asks the MPC network for the program's derived key
//!   (path `antibody/<project>/<program key>`) and pins its EVM address. The project deploys
//!   `AntibodyAdapter` with that address (CREATE2: one address on every chain) and registers each
//!   chain's adapter and proxies.
//! - near-antibody, the only account that may ask, requests a permit when the program's committed
//!   policy says so: a T1-confirmed pause ticket, or a passed release gate. For each registered chain
//!   the guardian computes the EIP-712 digest the adapter checks, asks the MPC network to sign it, and
//!   keeps the signature only if it recovers to the pinned address.
//! - Relaying is permissionless (`AntibodyAdapter.pause` and `release`). The guardian never builds or
//!   pays for an EVM transaction; the MPC fees come from the project's deposit here.

use near_sdk::{
    env, json_types::U128, log, near, require, serde_json::json, store::LookupMap, AccountId, Gas, NearToken, PanicOnDefault, Promise,
    PromiseError,
};

#[cfg(test)]
mod tests;

pub const SIGN_GAS: Gas = Gas::from_tgas(40);
pub const SIGN_CB_GAS: Gas = Gas::from_tgas(20);
pub const KEY_GAS: Gas = Gas::from_tgas(10);
pub const KEY_CB_GAS: Gas = Gas::from_tgas(10);
/// A permit expires a week after it is signed; a relayer has that long to submit it.
pub const PERMIT_TTL_SECS: u64 = 7 * 86_400;
/// yoctoNEAR attached to each MPC `sign` call, paid from the project's deposit.
pub const SIGN_FEE: u128 = 1;
pub const MAX_CHAINS: usize = 8;
/// secp256k1 group order n, big-endian: signatures are kept in low-s form (s ≤ n/2).
const N: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b,
    0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
];
const HALF_N: [u8; 32] = [
    0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d,
    0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b, 0x20, 0xa0,
];

/// One chain where a program has an adapter.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct ChainAdapter {
    pub chain_id: u64,
    /// 0x-prefixed address of the program's `AntibodyAdapter`.
    pub adapter: String,
    /// Proxies the adapter may upgrade with a release permit.
    pub proxies: Vec<String>,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug)]
pub struct ProgramGuard {
    pub path: String,
    /// The derived EVM address (0x…), pinned when the MPC network answers `derived_public_key`.
    pub address: Option<String>,
    pub chains: Vec<ChainAdapter>,
    /// yoctoNEAR for MPC fees.
    pub credit: U128,
    pub permits: u64,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq)]
pub enum PermitBody {
    Pause { ticket: u64 },
    Release { proxy: String, code_hash: String },
}

/// A signed permit, ready for anyone to relay.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug)]
pub struct SignedPermit {
    pub id: u64,
    pub program_key: String,
    pub project: AccountId,
    pub chain_id: u64,
    pub adapter: String,
    pub round: u32,
    pub body: PermitBody,
    pub deadline: u64,
    pub digest: String,
    pub v: u8,
    pub r: String,
    pub s: String,
    pub requested_ns: u64,
    pub signed_ns: u64,
}

#[near(serializers = [json])]
pub struct BigR {
    pub affine_point: String,
}

#[near(serializers = [json])]
pub struct Scalar {
    pub scalar: String,
}

/// The MPC network's secp256k1 answer to `sign`.
#[near(serializers = [json])]
pub struct SignatureResponse {
    pub big_r: BigR,
    pub s: Scalar,
    pub recovery_id: u8,
}

/// A permit that did not fit in the gas of the call that asked for it, waiting for `issue_pending`.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug)]
pub struct Pending {
    pub program_key: String,
    pub project: AccountId,
    pub chain_id: u64,
    pub round: u32,
    pub body: PermitBody,
}

/// A release named by near-antibody: upgrade `proxy` to an implementation with this code hash.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
pub struct ReleaseIn {
    pub proxy: String,
    pub code_hash: String,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Guardian {
    owner: AccountId,
    mpc: AccountId,
    /// near-antibody: the only account that may ask for permits.
    antibody: AccountId,
    programs: LookupMap<(String, AccountId), ProgramGuard>,
    permits: LookupMap<u64, SignedPermit>,
    next_permit: u64,
    failures: u64,
    /// Permits queued for `issue_pending`, oldest first. Each signature needs its own gas, so one call
    /// signs at most three; a round that releases two proxies on two chains needs four.
    pending: Vec<Pending>,
}

pub fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    hex::decode(s.strip_prefix("0x").unwrap_or(s)).ok()
}

fn word_u64(x: u64) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[24..].copy_from_slice(&x.to_be_bytes());
    w
}

fn word_addr(a: &str) -> [u8; 32] {
    let b = hex_bytes(a).filter(|b| b.len() == 20).unwrap_or_else(|| env::panic_str("an address is 20 bytes of hex"));
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(&b);
    w
}

fn word_b32(h: &str) -> [u8; 32] {
    hex_bytes(h).and_then(|b| <[u8; 32]>::try_from(b).ok()).unwrap_or_else(|| env::panic_str("a hash is 32 bytes of hex"))
}

fn keccak(parts: &[&[u8]]) -> [u8; 32] {
    env::keccak256_array(&parts.concat())
}

/// The adapter's EIP-712 domain separator (`AntibodyAdapter.domainSeparator`).
pub fn domain_separator(chain_id: u64, adapter: &str) -> [u8; 32] {
    let t = keccak(&[b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"]);
    keccak(&[&t, &keccak(&[b"Antibody"]), &keccak(&[b"1"]), &word_u64(chain_id), &word_addr(adapter)])
}

/// The digest `AntibodyAdapter.pauseDigest` / `releaseDigest` computes for this permit.
pub fn permit_digest(chain_id: u64, adapter: &str, program_key: &str, round: u32, body: &PermitBody, deadline: u64) -> [u8; 32] {
    let program = keccak(&[program_key.as_bytes()]);
    let h = match body {
        PermitBody::Pause { ticket } => {
            let t = keccak(&[b"PausePermit(bytes32 program,uint32 round,uint64 ticket,uint256 deadline)"]);
            keccak(&[&t, &program, &word_u64(round as u64), &word_u64(*ticket), &word_u64(deadline)])
        }
        PermitBody::Release { proxy, code_hash } => {
            let t = keccak(&[b"ReleasePermit(bytes32 program,uint32 round,address proxy,bytes32 codeHash,uint256 deadline)"]);
            keccak(&[&t, &program, &word_u64(round as u64), &word_addr(proxy), &word_b32(code_hash), &word_u64(deadline)])
        }
    };
    keccak(&[b"\x19\x01", &domain_separator(chain_id, adapter), &h])
}

/// `n - s` (big-endian), for `s < n`.
fn neg_mod_n(s: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut borrow = 0i16;
    for i in (0..32).rev() {
        let mut d = N[i] as i16 - s[i] as i16 - borrow;
        borrow = if d < 0 { 1 } else { 0 };
        if d < 0 {
            d += 256;
        }
        out[i] = d as u8;
    }
    out
}

/// An EVM address from an uncompressed secp256k1 key (`x ‖ y`, 64 bytes).
pub fn address_of(xy: &[u8]) -> String {
    format!("0x{}", hex::encode(&env::keccak256_array(xy)[12..]))
}

#[near]
impl Guardian {
    #[init]
    pub fn new(owner: AccountId, mpc: AccountId, antibody: AccountId) -> Self {
        Self { owner, mpc, antibody, programs: LookupMap::new(b"p"), permits: LookupMap::new(b"s"), next_permit: 0, failures: 0, pending: Vec::new() }
    }

    /// The project registers its program; the deposit pays MPC fees. The guardian asks the MPC network
    /// for the derived key of `antibody/<project>/<program key>` and pins its address.
    #[payable]
    pub fn register(&mut self, program_key: String, chains: Vec<ChainAdapter>) -> Promise {
        require!(program_key.len() <= 128, "program key too long");
        let project = env::predecessor_account_id();
        let key = (program_key.clone(), project.clone());
        require!(!self.programs.contains_key(&key), "already registered (set_chains changes chains)");
        Self::check_chains(&chains);
        let path = format!("antibody/{project}/{program_key}");
        let g = ProgramGuard { path: path.clone(), address: None, chains, credit: U128(env::attached_deposit().as_yoctonear()), permits: 0 };
        self.programs.insert(key, g);
        Promise::new(self.mpc.clone())
            .function_call(
                "derived_public_key".into(),
                json!({"path": path, "predecessor": env::current_account_id(), "domain_id": 0}).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                KEY_GAS,
            )
            .then(Self::ext(env::current_account_id()).with_static_gas(KEY_CB_GAS).on_key(program_key, project))
    }

    #[private]
    pub fn on_key(&mut self, program_key: String, project: AccountId, #[callback_result] r: Result<String, PromiseError>) -> Option<String> {
        let key = (program_key.clone(), project.clone());
        let mut g = self.programs.get(&key)?.clone();
        let pk = r.ok()?;
        let b58 = pk.strip_prefix("secp256k1:")?;
        let xy = bs58::decode(b58).into_vec().ok().filter(|b| b.len() == 64)?;
        let address = address_of(&xy);
        g.address = Some(address.clone());
        self.programs.insert(key, g);
        self.emit("program_registered", json!({"program_key": program_key, "project": project, "guardian_address": address}));
        Some(address)
    }

    /// The project replaces its chains (e.g. after deploying an adapter on one more chain).
    pub fn set_chains(&mut self, program_key: String, chains: Vec<ChainAdapter>) {
        Self::check_chains(&chains);
        let key = (program_key.clone(), env::predecessor_account_id());
        let mut g = self.programs.get(&key).cloned().unwrap_or_else(|| env::panic_str("not registered"));
        g.chains = chains;
        self.programs.insert(key, g);
        self.emit("chains_set", json!({"program_key": program_key, "project": env::predecessor_account_id()}));
    }

    #[payable]
    pub fn add_credit(&mut self, program_key: String, project: AccountId) {
        let key = (program_key, project);
        let mut g = self.programs.get(&key).cloned().unwrap_or_else(|| env::panic_str("not registered"));
        g.credit = U128(g.credit.0 + env::attached_deposit().as_yoctonear());
        self.programs.insert(key, g);
    }

    fn check_chains(chains: &[ChainAdapter]) {
        require!(chains.len() <= MAX_CHAINS, "too many chains");
        for c in chains {
            word_addr(&c.adapter);
            require!(c.proxies.len() <= 8, "too many proxies");
            for p in &c.proxies {
                word_addr(p);
            }
        }
    }

    /// near-antibody: a pause permit for every chain of the program (a T1-confirmed ticket).
    pub fn issue_pause(&mut self, program_key: String, project: AccountId, round: u32, ticket: u64) -> u32 {
        require!(env::predecessor_account_id() == self.antibody, "only near-antibody asks for permits");
        let g = self.guard(&program_key, &project);
        let mut n = 0;
        for c in g.chains.clone() {
            n += self.request(&program_key, &project, &c, round, PermitBody::Pause { ticket }) as u32;
        }
        n
    }

    /// near-antibody: a release permit for each named proxy on every chain where it is registered.
    pub fn issue_release(&mut self, program_key: String, project: AccountId, round: u32, releases: Vec<ReleaseIn>) -> u32 {
        require!(env::predecessor_account_id() == self.antibody, "only near-antibody asks for permits");
        let g = self.guard(&program_key, &project);
        let mut n = 0;
        for c in g.chains.clone() {
            for r in &releases {
                word_b32(&r.code_hash);
                if c.proxies.iter().any(|p| p.eq_ignore_ascii_case(&r.proxy)) {
                    let body = PermitBody::Release { proxy: r.proxy.to_lowercase(), code_hash: r.code_hash.to_lowercase() };
                    n += self.request(&program_key, &project, &c, round, body) as u32;
                }
            }
        }
        n
    }

    fn guard(&self, program_key: &str, project: &AccountId) -> ProgramGuard {
        self.programs.get(&(program_key.to_string(), project.clone())).cloned().unwrap_or_else(|| env::panic_str("the program has no guardian registration"))
    }

    /// Asks the MPC network to sign one permit; false (logged) if the program cannot pay, has no key, or
    /// this call has too little gas left for another signature.
    fn request(&mut self, program_key: &str, project: &AccountId, c: &ChainAdapter, round: u32, body: PermitBody) -> bool {
        if !Self::gas_for_one() {
            self.emit("permit_queued", json!({"program_key": program_key, "chain_id": c.chain_id, "round": round}));
            self.pending.push(Pending { program_key: program_key.to_string(), project: project.clone(), chain_id: c.chain_id, round, body });
            return false;
        }
        let key = (program_key.to_string(), project.clone());
        let mut g = self.programs.get(&key).cloned().unwrap();
        if g.address.is_none() || g.credit.0 < SIGN_FEE {
            self.failures += 1;
            self.emit("permit_skipped", json!({"program_key": program_key, "chain_id": c.chain_id, "reason": if g.address.is_none() { "no derived key yet" } else { "no MPC credit" }}));
            return false;
        }
        g.credit = U128(g.credit.0 - SIGN_FEE);
        g.permits += 1;
        let path = g.path.clone();
        self.programs.insert(key, g);
        let deadline = env::block_timestamp() / 1_000_000_000 + PERMIT_TTL_SECS;
        let digest = permit_digest(c.chain_id, &c.adapter, program_key, round, &body, deadline);
        let id = self.next_permit;
        self.next_permit += 1;
        let stub = SignedPermit {
            id,
            program_key: program_key.to_string(),
            project: project.clone(),
            chain_id: c.chain_id,
            adapter: c.adapter.to_lowercase(),
            round,
            body,
            deadline,
            digest: hex::encode(digest),
            v: 0,
            r: String::new(),
            s: String::new(),
            requested_ns: env::block_timestamp(),
            signed_ns: 0,
        };
        let args = json!({"request": {"path": path, "payload_v2": {"Ecdsa": hex::encode(digest)}, "domain_id": 0}});
        // Dropping the promise schedules it: one chain's failure must not hold up the others.
        drop(
            Promise::new(self.mpc.clone())
                .function_call("sign".into(), args.to_string().into_bytes(), NearToken::from_yoctonear(SIGN_FEE), SIGN_GAS)
                .then(Self::ext(env::current_account_id()).with_static_gas(SIGN_CB_GAS).on_signed(stub)),
        );
        true
    }

    fn gas_for_one() -> bool {
        let left = env::prepaid_gas().as_gas().saturating_sub(env::used_gas().as_gas());
        left >= SIGN_GAS.as_gas() + SIGN_CB_GAS.as_gas() + Gas::from_tgas(5).as_gas()
    }

    /// Anyone: asks the MPC network for queued permits, oldest first, as many as this call's gas allows.
    /// The program's own MPC credit pays, as for the call that queued them.
    pub fn issue_pending(&mut self) -> u32 {
        let mut n = 0;
        while !self.pending.is_empty() && Self::gas_for_one() {
            let q = self.pending.remove(0);
            let Some(c) = self.guard(&q.program_key, &q.project).chains.into_iter().find(|c| c.chain_id == q.chain_id) else { continue };
            n += self.request(&q.program_key, &q.project, &c, q.round, q.body) as u32;
        }
        n
    }

    #[private]
    pub fn on_signed(&mut self, stub: SignedPermit, #[callback_result] res: Result<SignatureResponse, PromiseError>) -> Option<u64> {
        let fail = |this: &mut Self, why: &str| {
            this.failures += 1;
            this.emit("permit_failed", json!({"permit_id": stub.id, "program_key": stub.program_key, "chain_id": stub.chain_id, "reason": why}));
            None
        };
        let Ok(sig) = res else { return fail(self, "the MPC network did not sign (timeout or error)") };
        let (Some(big_r), Some(s)) = (hex_bytes(&sig.big_r.affine_point), hex_bytes(&sig.s.scalar)) else { return fail(self, "bad signature encoding") };
        if big_r.len() != 33 || s.len() != 32 || sig.recovery_id > 1 {
            return fail(self, "bad signature encoding");
        }
        let mut r32 = [0u8; 32];
        r32.copy_from_slice(&big_r[1..]);
        let mut s32 = [0u8; 32];
        s32.copy_from_slice(&s);
        let mut rec = sig.recovery_id;
        // Low-s form, as the adapter requires; the twin signature recovers with the other parity.
        if s32 > HALF_N {
            s32 = neg_mod_n(&s32);
            rec ^= 1;
        }
        let mut rs = [0u8; 64];
        rs[..32].copy_from_slice(&r32);
        rs[32..].copy_from_slice(&s32);
        let digest = hex::decode(&stub.digest).unwrap();
        let recovered = env::ecrecover(&digest, &rs, rec, true).map(|pk| address_of(&pk)).unwrap_or_default();
        let expected = self.programs.get(&(stub.program_key.clone(), stub.project.clone())).and_then(|g| g.address.clone()).unwrap_or_default();
        if recovered != expected {
            log!("recovered {recovered}, expected {expected}");
            return fail(self, "the signature does not recover to the program's guardian address");
        }
        let p = SignedPermit { v: 27 + rec, r: hex::encode(r32), s: hex::encode(s32), signed_ns: env::block_timestamp(), ..stub };
        self.emit("permit_signed", json!(p));
        self.permits.insert(p.id, p.clone());
        Some(p.id)
    }

    fn emit(&self, event: &str, data: near_sdk::serde_json::Value) {
        let ev = json!({"standard": "antibody-guardian", "version": "2.0.0", "event": event, "data": [data]});
        env::log_str(&format!("EVENT_JSON:{ev}"));
    }

    // ------------------------------------------------------------------ views

    pub fn get_program(&self, program_key: String, project: AccountId) -> Option<ProgramGuard> {
        self.programs.get(&(program_key, project)).cloned()
    }

    pub fn get_permit(&self, id: u64) -> Option<SignedPermit> {
        self.permits.get(&id).cloned()
    }

    /// Permits from `from` (by id), at most `limit`.
    pub fn list_permits(&self, from: Option<u64>, limit: Option<u64>) -> Vec<SignedPermit> {
        let from = from.unwrap_or(0);
        (from..self.next_permit.min(from + limit.unwrap_or(50).min(100))).filter_map(|i| self.permits.get(&i).cloned()).collect()
    }

    pub fn pending_permits(&self) -> Vec<Pending> {
        self.pending.clone()
    }

    pub fn get_config(&self) -> near_sdk::serde_json::Value {
        json!({"owner": self.owner, "mpc": self.mpc, "antibody": self.antibody, "permits": self.next_permit, "failures": self.failures,
               "permit_ttl_secs": PERMIT_TTL_SECS, "sign_fee": U128(SIGN_FEE)})
    }
}

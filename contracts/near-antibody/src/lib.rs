//! near-antibody — bug bounties that pay on proof.
//!
//! Facts come from the TEE; the rules live here and are immutable per program:
//! - **Program.** A project opens a program with `ft_transfer_call` on the payout token. It commits
//!   the canonical manifest (anchor, targets, invariants code hash, severity per invariant, payout
//!   table, bond, reveal window, deadline, disclosure delay) and escrows the pool. For Sepolia
//!   anchors, the anchor hash is checked against the NEAR Ethereum light client at creation.
//! - **Known issues (t0).** Before opening, the project may submit its known issues through the
//!   same verifier. They claim their keys at t0, and nothing can be added afterwards.
//! - **Commit → reveal.** A hunter commits `SHA-256("antibody/commit/v1|program|hunter|sha256(blob)")`,
//!   locking a bond from their internal balance, then reveals a pointer to the sealed blob. The
//!   contract asks OutLayer to run the pinned verifier; the callback receives the facts.
//! - **Keys and priority.** A finding's key is `(slot, fp_tag)`. The earliest commit whose reveal
//!   is a Violation claims every key it breaks. It is paid once, at the highest severity among the
//!   keys it newly claims, after the reveal windows of all earlier commits have closed.
//! - **Duplicates and known issues** get no payout of any kind; their bond is refunded and they can
//!   view the earlier claim's proof (`get_duplicate_proof`).
//! - **Bonds** are refunded on any Violation and forfeited (to the project, covering verification)
//!   on NoViolation, InvalidPoC or a missed reveal.
//! - **Critical** findings make the program's guardian sign `pause()` on every registered chain.
//! - **T0.** Payouts above `instant_cap` wait `challenge_delay`; `challenge()` freezes them for the
//!   arbiter.
//! - **Disclosure.** After `deadline + disclosure_after`, one TEE run publishes every content key
//!   and `k_program`, so anyone can re-verify every receipt and tag.

pub mod events;
pub mod manifest;
pub mod types;

#[cfg(test)]
mod tests;

use events::emit;
use manifest::{is_hex, parse_amount, ManifestIn};
use near_sdk::{
    env,
    json_types::U128,
    log, near, require,
    serde_json::{self, json, Value},
    store::{LookupMap, Vector},
    AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseError, PromiseOrValue,
};
use types::*;

pub const OUTLAYER_GAS: Gas = Gas::from_tgas(100);
pub const PAUSE_GAS: Gas = Gas::from_tgas(160);
pub const VERIFY_CB_GAS: Gas = Gas::from_tgas(200);
pub const SMALL_CB_GAS: Gas = Gas::from_tgas(20);
pub const FT_GAS: Gas = Gas::from_tgas(15);
pub const LC_GAS: Gas = Gas::from_tgas(15);
pub const LC_CB_GAS: Gas = Gas::from_tgas(20);
pub const MAX_VERIFY_ATTEMPTS: u8 = 3;
pub const MAX_COMMITS_PER_PROGRAM: usize = 1000;
pub const MAX_DISCLOSURE_ENTRIES: usize = 150;
pub const SEPOLIA: u64 = 11155111;

pub fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}

/// `SHA-256("antibody/commit/v1|" ‖ program_key ‖ "|" ‖ hunter ‖ "|" ‖ hex(sha256(blob)))`.
pub fn commit_hash_for(program_key: &str, hunter: &str, blob_sha256: &str) -> String {
    hex(&env::sha256(format!("antibody/commit/v1|{program_key}|{hunter}|{blob_sha256}").as_bytes()))
}

/// Lowercase `0x` hex of the 256-bit mask with the given bits set (matches the verifier's `{:#x}`).
pub fn mask_hex(bits: &[u8]) -> String {
    let mut be = [0u8; 32];
    for &b in bits {
        be[31 - (b as usize / 8)] |= 1 << (b % 8);
    }
    let h = hex(&be);
    let t = h.trim_start_matches('0');
    format!("0x{}", if t.is_empty() { "0" } else { t })
}

fn claim_key(program: u32, slot: u8, tag: &str) -> String {
    format!("{program}:{slot}:{tag}")
}

fn valid_sealed(s: &SealedRef) {
    require!(
        (s.url.starts_with("https://") || s.url.starts_with("near-tx:")) && s.url.len() <= 512,
        "sealed.url must be https://… or near-tx:<hash>:<signer> (<= 512 chars)"
    );
    require!(is_hex(&s.sha256, 32), "sealed.sha256: 32-byte lowercase hex");
    require!(is_hex(&s.wrap_v, 92), "sealed.wrap_v: 92-byte lowercase hex");
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Antibody {
    owner: AccountId,
    token: AccountId,
    outlayer: AccountId,
    light_client: Option<AccountId>,
    verifier: VerifierConfig,
    programs: Vector<Program>,
    commits: Vector<Commit>,
    claims: LookupMap<String, Claim>,
    program_claims: LookupMap<u32, Vec<String>>,
    balances: LookupMap<AccountId, u128>,
    reputation: LookupMap<AccountId, Reputation>,
    hunters: Vector<AccountId>,
    stats: Stats,
}

#[near]
impl Antibody {
    #[init]
    pub fn new(
        owner: AccountId,
        token: AccountId,
        outlayer: AccountId,
        light_client: Option<AccountId>,
        verifier: VerifierConfig,
    ) -> Self {
        require!(is_hex(&verifier.wasm_hash, 32), "verifier.wasm_hash: 32-byte lowercase hex");
        Self {
            owner,
            token,
            outlayer,
            light_client,
            verifier,
            programs: Vector::new(b"p"),
            commits: Vector::new(b"c"),
            claims: LookupMap::new(b"k"),
            program_claims: LookupMap::new(b"q"),
            balances: LookupMap::new(b"b"),
            reputation: LookupMap::new(b"r"),
            hunters: Vector::new(b"h"),
            stats: Stats::default(),
        }
    }

    // ------------------------------------------------------------------ admin

    fn assert_owner(&self) {
        require!(env::predecessor_account_id() == self.owner, "only the owner");
    }

    /// Sets the verifier for programs created from now on (existing programs keep their pin).
    pub fn set_verifier(&mut self, verifier: VerifierConfig) {
        self.assert_owner();
        require!(is_hex(&verifier.wasm_hash, 32), "verifier.wasm_hash: 32-byte lowercase hex");
        if let Some(pk) = &verifier.pubkey {
            require!(is_hex(pk, 32), "verifier.pubkey: 32-byte lowercase hex");
        }
        emit("verifier_set", json!({"wasm_hash": verifier.wasm_hash, "wasm_url": verifier.wasm_url, "pubkey": verifier.pubkey, "tier": verifier.tier}));
        self.verifier = verifier;
    }

    pub fn set_light_client(&mut self, light_client: Option<AccountId>) {
        self.assert_owner();
        self.light_client = light_client;
    }

    pub fn set_outlayer(&mut self, outlayer: AccountId) {
        self.assert_owner();
        self.outlayer = outlayer;
    }

    /// Runs the pinned verifier in `pubkey` mode and stores the sealed-submission public key it prints.
    pub fn refresh_verifier_pubkey(&mut self) -> Promise {
        self.assert_owner();
        let input = json!({"mode": "pubkey"}).to_string();
        self.outlayer_request(&self.verifier.clone(), input).then(
            Self::ext(env::current_account_id())
                .with_static_gas(SMALL_CB_GAS)
                .on_pubkey(self.verifier.wasm_hash.clone()),
        )
    }

    #[private]
    pub fn on_pubkey(&mut self, wasm_hash: String, #[callback_result] r: Result<Option<PubkeyIn>, PromiseError>) -> Option<String> {
        match r {
            Ok(Some(p)) if is_hex(&p.pubkey, 32) && wasm_hash == self.verifier.wasm_hash => {
                self.verifier.pubkey = Some(p.pubkey.clone());
                emit("verifier_pubkey", json!({"wasm_hash": wasm_hash, "pubkey": p.pubkey}));
                Some(p.pubkey)
            }
            _ => {
                log!("verifier pubkey run failed or returned an invalid key");
                None
            }
        }
    }

    // ------------------------------------------------------------ token entry

    /// NEP-141 receiver: deposits, program creation, escrow top-ups and finder bonuses.
    pub fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> PromiseOrValue<U128> {
        require!(env::predecessor_account_id() == self.token, "only the payout token");
        let m: TokenMsg = serde_json::from_str(&msg).unwrap_or_else(|e| env::panic_str(&format!("bad msg: {e}")));
        match m {
            TokenMsg::Deposit {} => {
                self.credit(&sender_id, amount.0);
                emit("deposit", json!({"account": sender_id, "amount": amount}));
                PromiseOrValue::Value(U128(0))
            }
            TokenMsg::CreateProgram { manifest, manifest_url, project_pubkey, guardian } => {
                self.create_program(sender_id, amount.0, manifest, manifest_url, project_pubkey, guardian)
            }
            TokenMsg::FundProgram { program_id } => {
                let p = self.programs.get_mut(program_id).unwrap_or_else(|| env::panic_str("no such program"));
                require!(p.project == sender_id, "only the project funds its escrow");
                require!(!matches!(p.status, ProgramStatus::Rejected { .. }), "program rejected");
                require!(!p.escrow_withdrawn, "escrow already withdrawn");
                p.escrow = U128(p.escrow.0 + amount.0);
                self.stats.escrow_total = U128(self.stats.escrow_total.0 + amount.0);
                emit("program_funded", json!({"program_id": program_id, "amount": amount}));
                PromiseOrValue::Value(U128(0))
            }
            TokenMsg::TopUp { commit_id, note } => {
                let c = self.load_commit(commit_id);
                let p = self.load_program(c.program);
                require!(p.project == sender_id, "only the project tops up its findings");
                require!(
                    matches!(c.verdict.as_ref().map(|v| v.kind), Some(Kind::Violation)) && !c.known_issue,
                    "top-ups go to verified violations"
                );
                self.credit(&c.hunter, amount.0);
                let cm = self.commit_mut(commit_id);
                cm.bonus = U128(cm.bonus.0 + amount.0);
                self.rep_mut(&c.hunter).earned.0 += amount.0;
                self.stats.paid_total = U128(self.stats.paid_total.0 + amount.0);
                emit("top_up", json!({"commit_id": commit_id, "hunter": c.hunter, "amount": amount, "note": note}));
                PromiseOrValue::Value(U128(0))
            }
        }
    }

    fn create_program(
        &mut self,
        project: AccountId,
        amount: u128,
        manifest: String,
        manifest_url: String,
        project_pubkey: String,
        guardian: Option<AccountId>,
    ) -> PromiseOrValue<U128> {
        let m: ManifestIn = serde_json::from_str(&manifest).unwrap_or_else(|e| env::panic_str(&format!("manifest: {e}")));
        m.validate();
        require!(m.program.project == project.as_str(), "manifest.program.project must be the sender");
        require!(m.payout.token == self.token.as_str(), "manifest.payout.token must be the payout token");
        require!(is_hex(&project_pubkey, 32), "project_pubkey: 32-byte lowercase hex X25519 key");
        require!(self.verifier.pubkey.is_some(), "verifier pubkey not set (refresh_verifier_pubkey)");
        let now = env::block_timestamp();
        let deadline_ns = m.rules.deadline_unix.saturating_mul(1_000_000_000);
        require!(deadline_ns > now, "deadline is in the past");
        let id = self.programs.len();
        let key = format!("{}/{}", env::current_account_id(), id);
        let chain_id = m.anchor.chain_id;
        let block_number = m.anchor.block_number;
        let program = Program {
            id,
            key: key.clone(),
            name: m.program.name.clone(),
            project: project.clone(),
            manifest_hash: hex(&env::sha256(manifest.as_bytes())),
            manifest_url,
            anchor: Anchor {
                chain_id,
                block_number,
                block_hash: m.anchor.block_hash.to_lowercase(),
                evm_spec: m.anchor.evm_spec.clone(),
            },
            anchor_trust: AnchorTrust::Pending,
            targets: m
                .targets
                .iter()
                .map(|t| TargetInfo { name: t.name.clone(), address: t.address.to_lowercase(), code_hash: t.code_hash.to_lowercase() })
                .collect(),
            invariants_code_hash: m.invariants.runtime_code_hash.to_lowercase(),
            slots: m.invariants.slots.iter().map(|s| SlotRule { bit: s.bit, name: s.name.clone(), severity: s.severity }).collect(),
            payout: PayoutTable {
                critical: U128(parse_amount(&m.payout.critical, "critical")),
                high: U128(parse_amount(&m.payout.high, "high")),
                medium: U128(parse_amount(&m.payout.medium, "medium")),
                low: U128(parse_amount(&m.payout.low, "low")),
                impact_bps: m.payout.impact_bps,
                impact_cap: U128(parse_amount(&m.payout.impact_cap, "impact_cap")),
                decimals: m.payout.decimals,
            },
            bond: U128(parse_amount(&m.rules.bond, "bond")),
            reveal_window_blocks: m.rules.reveal_window_blocks,
            deadline_ns,
            disclosure_after_ns: m.rules.disclosure_after_secs.saturating_mul(1_000_000_000),
            instant_cap: U128(parse_amount(&m.rules.instant_payout_cap, "instant_payout_cap")),
            challenge_delay_ns: m.rules.challenge_delay_secs.saturating_mul(1_000_000_000),
            verifier: self.verifier.clone(),
            project_pubkey,
            guardian,
            status: ProgramStatus::Created,
            created_height: env::block_height(),
            created_ns: now,
            opened_ns: None,
            escrow: U128(amount),
            paid: U128(0),
            reserved: U128(0),
            escrow_withdrawn: false,
            commits: Vec::new(),
            pause_requested: false,
            misconfigured: false,
            disclosure_pending: false,
            disclosure: None,
        };
        emit(
            "program_created",
            json!({"program_id": id, "key": key, "project": project, "manifest_hash": program.manifest_hash,
                   "chain_id": chain_id, "anchor_block": block_number, "anchor_hash": program.anchor.block_hash,
                   "escrow": U128(amount), "verifier_wasm_hash": program.verifier.wasm_hash}),
        );
        self.programs.push(program);
        self.stats.programs += 1;
        self.stats.escrow_total = U128(self.stats.escrow_total.0 + amount);
        match (&self.light_client, chain_id) {
            (Some(lc), SEPOLIA) => PromiseOrValue::Promise(
                Promise::new(lc.clone())
                    .function_call("block_hash_safe".into(), block_number.to_le_bytes().to_vec(), NearToken::from_yoctonear(0), LC_GAS)
                    .then(Self::ext(env::current_account_id()).with_static_gas(LC_CB_GAS).on_anchor_checked(id, lc.clone())),
            ),
            _ => {
                let reason = if chain_id == SEPOLIA { "no light client configured" } else { "local chain (no light client)" };
                let p = self.programs.get_mut(id).unwrap();
                p.anchor_trust = AnchorTrust::ProjectAttested { reason: reason.into() };
                p.status = ProgramStatus::KnownIssues;
                emit("anchor_checked", json!({"program_id": id, "trust": "project-attested", "reason": reason}));
                PromiseOrValue::Value(U128(0))
            }
        }
    }

    /// Light-client callback: verified, project-attested (outside the window) or rejected + refunded.
    #[private]
    pub fn on_anchor_checked(
        &mut self,
        program_id: u32,
        client: AccountId,
        #[callback_result]
        #[serializer(borsh)]
        r: Result<Option<[u8; 32]>, PromiseError>,
    ) -> U128 {
        let height = env::block_height();
        let p = self.programs.get_mut(program_id).unwrap();
        match r {
            Ok(Some(h)) => {
                let got = format!("0x{}", hex(&h));
                if got == p.anchor.block_hash {
                    p.anchor_trust = AnchorTrust::LightClient { client: client.clone(), checked_at_height: height };
                    p.status = ProgramStatus::KnownIssues;
                    emit("anchor_checked", json!({"program_id": program_id, "trust": "light-client", "client": client, "block_hash": got}));
                    U128(0)
                } else {
                    let refund = p.escrow.0;
                    p.status = ProgramStatus::Rejected { reason: format!("light client has {got} at block {}", p.anchor.block_number) };
                    p.escrow = U128(0);
                    self.stats.escrow_total = U128(self.stats.escrow_total.0.saturating_sub(refund));
                    emit("program_rejected", json!({"program_id": program_id, "light_client_hash": got}));
                    U128(refund)
                }
            }
            Ok(None) => {
                p.anchor_trust = AnchorTrust::ProjectAttested { reason: "anchor outside the light-client window".into() };
                p.status = ProgramStatus::KnownIssues;
                emit("anchor_checked", json!({"program_id": program_id, "trust": "project-attested", "reason": "outside window"}));
                U128(0)
            }
            Err(_) => {
                p.anchor_trust = AnchorTrust::ProjectAttested { reason: "light client call failed".into() };
                p.status = ProgramStatus::KnownIssues;
                emit("anchor_checked", json!({"program_id": program_id, "trust": "project-attested", "reason": "call failed"}));
                U128(0)
            }
        }
    }

    // ------------------------------------------------------- program lifecycle

    /// The project's declared known issues, verified by the same TEE before the program opens (t0).
    pub fn submit_known(&mut self, program_id: u32, sealed: SealedRef) -> Promise {
        let project = env::predecessor_account_id();
        let p = self.load_program(program_id);
        require!(p.project == project, "only the project");
        require!(p.status == ProgramStatus::KnownIssues, "known issues only before the program opens");
        valid_sealed(&sealed);
        let id = self.commits.len() as u64;
        let commit_hash = commit_hash_for(&p.key, project.as_str(), &sealed.sha256);
        self.commits.push(Commit {
            id,
            program: program_id,
            hunter: project.clone(),
            commit_hash: commit_hash.clone(),
            height: env::block_height(),
            ns: env::block_timestamp(),
            bond: U128(0),
            known_issue: true,
            status: CommitStatus::Verifying,
            sealed: Some(sealed.clone()),
            submitted_height: Some(env::block_height()),
            submitted_ns: Some(env::block_timestamp()),
            verify_attempts: 0,
            verdict: None,
            outcome: Outcome::None,
            bonus: U128(0),
        });
        self.programs.get_mut(program_id).unwrap().commits.push(id);
        emit("known_issue_submitted", json!({"program_id": program_id, "commit_id": id, "commit_hash": commit_hash, "sealed_sha256": sealed.sha256}));
        self.request_verify(id)
    }

    /// Ends the known-issues phase; hunters can commit from now on.
    pub fn open_program(&mut self, program_id: u32) {
        let p = self.load_program(program_id);
        require!(env::predecessor_account_id() == p.project, "only the project");
        require!(p.status == ProgramStatus::KnownIssues, "program is not in the known-issues phase");
        for &c in &p.commits {
            require!(self.load_commit(c).status != CommitStatus::Verifying, "a known issue is still being verified");
        }
        let p = self.programs.get_mut(program_id).unwrap();
        p.status = ProgramStatus::Open;
        p.opened_ns = Some(env::block_timestamp());
        emit("program_opened", json!({"program_id": program_id, "height": env::block_height()}));
    }

    // ---------------------------------------------------------- commit/reveal

    /// Commits to a sealed submission; locks the program's bond from the caller's internal balance.
    /// Callable with a function-call access key (no deposit).
    pub fn commit(&mut self, program_id: u32, commit_hash: String) -> u64 {
        let hunter = env::predecessor_account_id();
        let p = self.load_program(program_id);
        require!(p.status == ProgramStatus::Open, "program is not open");
        require!(!p.misconfigured, "program is misconfigured");
        require!(env::block_timestamp() < p.deadline_ns, "program deadline has passed");
        require!(is_hex(&commit_hash, 32), "commit_hash: 32-byte lowercase hex");
        require!(p.commits.len() < MAX_COMMITS_PER_PROGRAM, "program commit limit reached");
        let bal = self.balance(&hunter);
        require!(bal >= p.bond.0, "insufficient internal balance for the bond (deposit with ft_transfer_call msg {\"deposit\":{}})");
        self.balances.insert(hunter.clone(), bal - p.bond.0);
        let id = self.commits.len() as u64;
        self.commits.push(Commit {
            id,
            program: program_id,
            hunter: hunter.clone(),
            commit_hash: commit_hash.clone(),
            height: env::block_height(),
            ns: env::block_timestamp(),
            bond: p.bond,
            known_issue: false,
            status: CommitStatus::Committed,
            sealed: None,
            submitted_height: None,
            submitted_ns: None,
            verify_attempts: 0,
            verdict: None,
            outcome: Outcome::None,
            bonus: U128(0),
        });
        self.programs.get_mut(program_id).unwrap().commits.push(id);
        self.rep_mut(&hunter).commits += 1;
        self.stats.commits += 1;
        emit(
            "committed",
            json!({"program_id": program_id, "commit_id": id, "hunter": hunter, "commit_hash": commit_hash,
                   "height": env::block_height(), "bond": p.bond}),
        );
        id
    }

    /// Reveals the sealed submission of `commit_id` and requests verification.
    /// Callable with a function-call access key; attach ~320 Tgas.
    pub fn submit(&mut self, commit_id: u64, sealed: SealedRef) -> Promise {
        let c = self.load_commit(commit_id);
        require!(env::predecessor_account_id() == c.hunter, "only the committing account");
        require!(c.status == CommitStatus::Committed, "commit already revealed or expired");
        let p = self.load_program(c.program);
        require!(env::block_height() <= c.height + p.reveal_window_blocks, "reveal window closed");
        valid_sealed(&sealed);
        require!(
            commit_hash_for(&p.key, c.hunter.as_str(), &sealed.sha256) == c.commit_hash,
            "sealed blob does not match the commitment"
        );
        let cm = self.commit_mut(commit_id);
        cm.sealed = Some(sealed.clone());
        cm.status = CommitStatus::Verifying;
        cm.submitted_height = Some(env::block_height());
        cm.submitted_ns = Some(env::block_timestamp());
        emit("submitted", json!({"program_id": c.program, "commit_id": commit_id, "sealed_sha256": sealed.sha256, "url": sealed.url}));
        self.request_verify(commit_id)
    }

    /// Re-requests verification after OutLayer returned no result (anyone may call).
    pub fn retry_verify(&mut self, commit_id: u64) -> Promise {
        let c = self.load_commit(commit_id);
        require!(c.status == CommitStatus::VerifyFailed, "not awaiting a retry");
        require!(c.verify_attempts < MAX_VERIFY_ATTEMPTS, "no retries left");
        self.commit_mut(commit_id).status = CommitStatus::Verifying;
        self.request_verify(commit_id)
    }

    fn outlayer_request(&self, v: &VerifierConfig, input: String) -> Promise {
        let mut args = json!({
            "source": {"WasmUrl": {"url": v.wasm_url, "hash": v.wasm_hash, "build_target": "wasm32-wasip2"}},
            "resource_limits": {"max_instructions": v.max_instructions, "max_memory_mb": v.max_memory_mb,
                                "max_execution_seconds": v.max_execution_seconds},
            "input_data": input,
            "response_format": "Json",
        });
        if let (Some(owner), Some(profile)) = (&v.secrets_owner, &v.secrets_profile) {
            args["secrets_ref"] = json!({"profile": profile, "account_id": owner});
        }
        Promise::new(self.outlayer.clone()).function_call(
            "request_execution".into(),
            args.to_string().into_bytes(),
            NearToken::from_yoctonear(v.deposit.0),
            OUTLAYER_GAS,
        )
    }

    fn request_verify(&mut self, commit_id: u64) -> Promise {
        let c = self.load_commit(commit_id);
        let p = self.load_program(c.program);
        self.commit_mut(commit_id).verify_attempts += 1;
        let input = json!({
            "mode": "verify",
            "program_id": p.key,
            "commit_id": commit_id,
            "hunter": c.hunter,
            "manifest_hash": p.manifest_hash,
            "sealed": c.sealed,
            "project_pubkey": p.project_pubkey,
        })
        .to_string();
        self.outlayer_request(&p.verifier, input).then(
            Self::ext(env::current_account_id()).with_static_gas(VERIFY_CB_GAS).on_verified(commit_id),
        )
    }

    /// OutLayer callback: applies the committed rules to the TEE's facts.
    #[private]
    pub fn on_verified(&mut self, commit_id: u64, #[callback_result] r: Result<Option<FactsIn>, PromiseError>) {
        let c = self.load_commit(commit_id);
        if c.status != CommitStatus::Verifying {
            log!("on_verified: commit {} is not verifying", commit_id);
            return;
        }
        let p = self.load_program(c.program);
        let facts = match r {
            Ok(Some(f)) => f,
            _ => {
                if c.verify_attempts >= MAX_VERIFY_ATTEMPTS {
                    self.settle_error(commit_id, "verification produced no result after the maximum attempts");
                } else {
                    self.commit_mut(commit_id).status = CommitStatus::VerifyFailed;
                    emit("verify_failed", json!({"commit_id": commit_id, "attempts": c.verify_attempts}));
                }
                return;
            }
        };
        if facts.commit_id != commit_id || facts.program != p.key || facts.manifest_hash != p.manifest_hash || facts.v != 1 {
            self.settle_error(commit_id, "facts do not match the request");
            return;
        }
        let impact = facts.impact_usd_micro.parse::<u128>().unwrap_or(0);
        let mut keys: Vec<FindingKey> = Vec::new();
        if facts.kind == Kind::Violation {
            for t in &facts.fp_tags {
                let sev = match p.severity(t.slot) {
                    Some(s) => s,
                    None => return self.settle_error(commit_id, "facts name an undeclared invariant"),
                };
                if !is_hex(&t.tag, 32) || keys.iter().any(|k| k.slot == t.slot) {
                    return self.settle_error(commit_id, "malformed fingerprint tags");
                }
                keys.push(FindingKey { slot: t.slot, tag: t.tag.clone(), severity: sev });
            }
            let bits: Vec<u8> = keys.iter().map(|k| k.slot).collect();
            if keys.is_empty() || mask_hex(&bits) != facts.broken_mask.to_lowercase() {
                return self.settle_error(commit_id, "broken mask and tags disagree");
            }
        }
        let height = env::block_height();
        let verdict = Verdict {
            kind: facts.kind,
            broken_mask: facts.broken_mask.to_lowercase(),
            keys: keys.clone(),
            impact_usd_micro: U128(impact),
            receipt_hash: facts.receipt_hash.clone(),
            poc_hash: facts.poc_hash.clone(),
            project_wrap: if facts.kind == Kind::Violation { facts.project_wrap.clone() } else { None },
            wasm_hash: facts.wasm_hash.clone(),
            tier: p.verifier.tier.clone(),
            verified_height: height,
            verified_ns: env::block_timestamp(),
        };
        self.stats.verdicts += 1;
        if let Some(sh) = c.submitted_height {
            self.stats.verdict_latency_blocks_sum += height.saturating_sub(sh);
            self.stats.verdict_latency_samples += 1;
        }
        {
            let cm = self.commit_mut(commit_id);
            cm.verdict = Some(verdict);
            cm.status = CommitStatus::Verified;
        }
        emit(
            "verdict",
            json!({"program_id": c.program, "commit_id": commit_id, "kind": facts.kind, "broken_mask": facts.broken_mask,
                   "keys": keys, "impact_usd_micro": U128(impact), "receipt_hash": facts.receipt_hash,
                   "tier": p.verifier.tier, "known_issue": c.known_issue}),
        );
        match facts.kind {
            Kind::Violation if c.known_issue => {
                for k in &keys {
                    let ck = claim_key(c.program, k.slot, &k.tag);
                    if !self.claims.contains_key(&ck) {
                        self.claims.insert(ck.clone(), Claim { commit: commit_id, known: true });
                        self.push_program_claim(c.program, ck);
                    }
                }
                self.commit_mut(commit_id).outcome = Outcome::KnownIssue;
                emit("known_issue_claimed", json!({"program_id": c.program, "commit_id": commit_id, "keys": keys}));
            }
            Kind::Violation => {
                for k in &keys {
                    let ck = claim_key(c.program, k.slot, &k.tag);
                    match self.claims.get(&ck).copied() {
                        None => {
                            self.claims.insert(ck.clone(), Claim { commit: commit_id, known: false });
                            self.push_program_claim(c.program, ck);
                        }
                        Some(cl) if !cl.known && cl.commit > commit_id && !self.is_settled(cl.commit) => {
                            // An earlier commit revealed later: priority goes to the earlier commit.
                            self.claims.insert(ck, Claim { commit: commit_id, known: false });
                        }
                        _ => {}
                    }
                }
                self.credit(&c.hunter, c.bond.0);
                self.commit_mut(commit_id).outcome = Outcome::Pending;
                self.stats.violations += 1;
                let critical = keys.iter().any(|k| k.severity == Severity::Critical);
                if critical && !p.pause_requested {
                    if let Some(g) = p.guardian.clone() {
                        self.programs.get_mut(c.program).unwrap().pause_requested = true;
                        emit("pause_requested", json!({"program_id": c.program, "commit_id": commit_id, "guardian": g}));
                        Promise::new(g).function_call(
                            "pause_all".into(),
                            json!({"program": p.key, "commit_id": commit_id}).to_string().into_bytes(),
                            NearToken::from_yoctonear(0),
                            PAUSE_GAS,
                        );
                    }
                }
            }
            Kind::NoViolation | Kind::InvalidPoC => {
                if !c.known_issue {
                    self.credit(&p.project, c.bond.0);
                    self.rep_mut(&c.hunter).slop += 1;
                    self.stats.slop_rejected += 1;
                }
                self.commit_mut(commit_id).outcome = Outcome::Rejected { kind: facts.kind };
            }
            Kind::ProgramMisconfigured => {
                self.credit(&c.hunter, c.bond.0);
                self.programs.get_mut(c.program).unwrap().misconfigured = true;
                self.commit_mut(commit_id).outcome = Outcome::Error { reason: "program misconfigured at the anchor".into() };
                emit("program_misconfigured", json!({"program_id": c.program, "commit_id": commit_id}));
            }
            Kind::ExecutionError => {
                self.credit(&c.hunter, c.bond.0);
                self.commit_mut(commit_id).outcome = Outcome::Error { reason: "verifier execution error".into() };
            }
        }
    }

    fn settle_error(&mut self, commit_id: u64, reason: &str) {
        let c = self.load_commit(commit_id);
        self.credit(&c.hunter, c.bond.0);
        let cm = self.commit_mut(commit_id);
        cm.status = CommitStatus::Verified;
        cm.outcome = Outcome::Error { reason: reason.into() };
        emit("verdict_error", json!({"commit_id": commit_id, "reason": reason}));
    }

    // -------------------------------------------------------------- settlement

    /// Applies priority once every earlier commit's reveal window has closed and every earlier
    /// reveal is verified. Anyone may call.
    pub fn finalize(&mut self, commit_id: u64) -> PromiseOrValue<()> {
        let c = self.load_commit(commit_id);
        let p = self.load_program(c.program);
        if let Outcome::PayoutFailed { amount, severity } = c.outcome.clone() {
            return self.pay(commit_id, amount.0, severity);
        }
        require!(c.status == CommitStatus::Verified && c.outcome == Outcome::Pending, "nothing to finalize");
        require!(
            env::block_height() > c.height + p.reveal_window_blocks,
            "reveal windows of earlier commits are still open"
        );
        for &e in &p.commits {
            if e >= commit_id {
                break;
            }
            let ec = self.load_commit(e);
            let in_flight = ec.status == CommitStatus::Verifying
                || (ec.status == CommitStatus::VerifyFailed && ec.verify_attempts < MAX_VERIFY_ATTEMPTS);
            require!(!in_flight, "an earlier commit is still being verified");
        }
        let keys = c.verdict.as_ref().map(|v| v.keys.clone()).unwrap_or_default();
        let mut newly: Vec<&FindingKey> = Vec::new();
        for k in &keys {
            let cl = self.claims.get(&claim_key(c.program, k.slot, &k.tag)).copied();
            if cl == Some(Claim { commit: commit_id, known: false }) {
                newly.push(k);
            }
        }
        if newly.is_empty() {
            let mut sorted = keys.clone();
            sorted.sort_by(|a, b| b.severity.cmp(&a.severity));
            let holders: Vec<Claim> =
                sorted.iter().map(|k| *self.claims.get(&claim_key(c.program, k.slot, &k.tag)).unwrap()).collect();
            if holders.iter().all(|h| h.known) {
                self.commit_mut(commit_id).outcome = Outcome::Known { of: holders[0].commit };
                self.rep_mut(&c.hunter).known += 1;
                self.stats.known += 1;
                emit("known", json!({"program_id": c.program, "commit_id": commit_id, "of": holders[0].commit}));
            } else {
                let of = holders.iter().find(|h| !h.known).unwrap().commit;
                self.commit_mut(commit_id).outcome = Outcome::Duplicate { of };
                self.rep_mut(&c.hunter).late += 1;
                self.stats.duplicates += 1;
                emit("duplicate", json!({"program_id": c.program, "commit_id": commit_id, "of": of}));
            }
            return PromiseOrValue::Value(());
        }
        let severity = newly.iter().map(|k| k.severity).max().unwrap();
        let mut amount = p.payout.base(severity);
        if p.payout.impact_bps > 0 {
            let impact = c.verdict.as_ref().map(|v| v.impact_usd_micro.0).unwrap_or(0);
            let units = if p.payout.decimals >= 6 {
                impact.saturating_mul(10u128.pow((p.payout.decimals - 6) as u32))
            } else {
                impact / 10u128.pow((6 - p.payout.decimals) as u32)
            };
            amount += (units.saturating_mul(p.payout.impact_bps as u128) / 10_000).min(p.payout.impact_cap.0);
        }
        amount = amount.min(p.available());
        {
            let r = self.rep_mut(&c.hunter);
            match severity {
                Severity::Critical => r.valid_critical += 1,
                Severity::High => r.valid_high += 1,
                Severity::Medium => r.valid_medium += 1,
                Severity::Low => r.valid_low += 1,
            }
        }
        self.stats.paid_findings += 1;
        if amount > p.instant_cap.0 {
            let until_ns = env::block_timestamp() + p.challenge_delay_ns;
            let pm = self.programs.get_mut(c.program).unwrap();
            pm.reserved = U128(pm.reserved.0 + amount);
            self.commit_mut(commit_id).outcome = Outcome::Delayed { amount: U128(amount), severity, until_ns };
            emit("payout_delayed", json!({"program_id": c.program, "commit_id": commit_id, "amount": U128(amount), "until_ns": until_ns}));
            return PromiseOrValue::Value(());
        }
        self.pay(commit_id, amount, severity)
    }

    fn pay(&mut self, commit_id: u64, amount: u128, severity: Severity) -> PromiseOrValue<()> {
        let c = self.load_commit(commit_id);
        let height = env::block_height();
        {
            let pm = self.programs.get_mut(c.program).unwrap();
            pm.paid = U128(pm.paid.0 + amount);
        }
        self.stats.paid_total = U128(self.stats.paid_total.0 + amount);
        self.rep_mut(&c.hunter).earned.0 += amount;
        self.commit_mut(commit_id).outcome =
            Outcome::Paid { amount: U128(amount), severity, height, ns: env::block_timestamp() };
        emit("paid", json!({"program_id": c.program, "commit_id": commit_id, "hunter": c.hunter, "amount": U128(amount), "severity": severity}));
        if amount == 0 {
            return PromiseOrValue::Value(());
        }
        PromiseOrValue::Promise(
            Promise::new(self.token.clone())
                .function_call(
                    "ft_transfer".into(),
                    json!({"receiver_id": c.hunter, "amount": U128(amount),
                           "memo": format!("antibody payout program {} commit {}", c.program, commit_id)})
                    .to_string()
                    .into_bytes(),
                    NearToken::from_yoctonear(1),
                    FT_GAS,
                )
                .then(Self::ext(env::current_account_id()).with_static_gas(SMALL_CB_GAS).on_payout(commit_id, U128(amount), severity)),
        )
    }

    #[private]
    pub fn on_payout(&mut self, commit_id: u64, amount: U128, severity: Severity, #[callback_result] r: Result<(), PromiseError>) {
        if r.is_ok() {
            return;
        }
        let c = self.load_commit(commit_id);
        {
            let pm = self.programs.get_mut(c.program).unwrap();
            pm.paid = U128(pm.paid.0.saturating_sub(amount.0));
        }
        self.stats.paid_total = U128(self.stats.paid_total.0.saturating_sub(amount.0));
        let r = self.rep_mut(&c.hunter);
        r.earned = U128(r.earned.0.saturating_sub(amount.0));
        self.commit_mut(commit_id).outcome = Outcome::PayoutFailed { amount, severity };
        emit("payout_failed", json!({"commit_id": commit_id, "amount": amount}));
    }

    /// T0: pays a delayed payout once its challenge window passed without a challenge.
    pub fn release(&mut self, commit_id: u64) -> PromiseOrValue<()> {
        let c = self.load_commit(commit_id);
        let (amount, severity, until_ns) = match c.outcome {
            Outcome::Delayed { amount, severity, until_ns } => (amount, severity, until_ns),
            _ => env::panic_str("not a delayed payout"),
        };
        require!(env::block_timestamp() >= until_ns, "challenge window still open");
        let pm = self.programs.get_mut(c.program).unwrap();
        pm.reserved = U128(pm.reserved.0.saturating_sub(amount.0));
        self.pay(commit_id, amount.0, severity)
    }

    /// T0: anyone whose local re-run disagrees can freeze a delayed payout for the arbiter.
    pub fn challenge(&mut self, commit_id: u64, reason: String) {
        let c = self.load_commit(commit_id);
        let (amount, severity, until_ns) = match c.outcome {
            Outcome::Delayed { amount, severity, until_ns } => (amount, severity, until_ns),
            _ => env::panic_str("only delayed payouts can be challenged"),
        };
        require!(env::block_timestamp() < until_ns, "challenge window closed");
        require!(reason.len() <= 512, "reason too long");
        let by = env::predecessor_account_id();
        self.commit_mut(commit_id).outcome = Outcome::Challenged { amount, severity, by: by.clone(), reason: reason.clone() };
        emit("challenged", json!({"commit_id": commit_id, "by": by, "reason": reason}));
    }

    /// Arbiter (the owner in T0) resolves a challenge; the resolution is logged.
    pub fn resolve_challenge(&mut self, commit_id: u64, uphold_payout: bool, note: String) -> PromiseOrValue<()> {
        self.assert_owner();
        let c = self.load_commit(commit_id);
        let (amount, severity) = match c.outcome {
            Outcome::Challenged { amount, severity, .. } => (amount, severity),
            _ => env::panic_str("not challenged"),
        };
        {
            let pm = self.programs.get_mut(c.program).unwrap();
            pm.reserved = U128(pm.reserved.0.saturating_sub(amount.0));
        }
        emit("challenge_resolved", json!({"commit_id": commit_id, "payout_upheld": uphold_payout, "note": note}));
        if uphold_payout {
            return self.pay(commit_id, amount.0, severity);
        }
        self.commit_mut(commit_id).outcome = Outcome::Cancelled { reason: note };
        PromiseOrValue::Value(())
    }

    /// Unrevealed commits forfeit their bond after the reveal window (anyone may call).
    pub fn expire(&mut self, commit_id: u64) {
        let c = self.load_commit(commit_id);
        require!(c.status == CommitStatus::Committed, "not an unrevealed commit");
        let p = self.load_program(c.program);
        require!(env::block_height() > c.height + p.reveal_window_blocks, "reveal window still open");
        self.credit(&p.project, c.bond.0);
        self.rep_mut(&c.hunter).unrevealed += 1;
        let cm = self.commit_mut(commit_id);
        cm.status = CommitStatus::Expired;
        cm.outcome = Outcome::Forfeited;
        emit("expired", json!({"commit_id": commit_id, "hunter": c.hunter}));
    }

    // ------------------------------------------------------------- withdrawals

    /// Withdraws the caller's internal balance (not callable by an agent's function-call key
    /// restricted to commit/submit).
    pub fn withdraw(&mut self, amount: U128) -> Promise {
        let who = env::predecessor_account_id();
        let bal = self.balance(&who);
        require!(amount.0 > 0 && bal >= amount.0, "insufficient balance");
        self.balances.insert(who.clone(), bal - amount.0);
        emit("withdraw", json!({"account": who, "amount": amount}));
        Promise::new(self.token.clone())
            .function_call(
                "ft_transfer".into(),
                json!({"receiver_id": who, "amount": amount, "memo": "antibody withdraw"}).to_string().into_bytes(),
                NearToken::from_yoctonear(1),
                FT_GAS,
            )
            .then(Self::ext(env::current_account_id()).with_static_gas(SMALL_CB_GAS).on_refund_if_failed(who, amount))
    }

    #[private]
    pub fn on_refund_if_failed(&mut self, who: AccountId, amount: U128, #[callback_result] r: Result<(), PromiseError>) {
        if r.is_err() {
            self.credit(&who, amount.0);
            emit("withdraw_failed", json!({"account": who, "amount": amount}));
        }
    }

    /// Returns unused escrow to the project after the deadline, once every commit is settled.
    pub fn withdraw_escrow(&mut self, program_id: u32) -> Promise {
        let p = self.load_program(program_id);
        require!(env::predecessor_account_id() == p.project, "only the project");
        require!(!p.escrow_withdrawn, "already withdrawn");
        require!(env::block_timestamp() > p.deadline_ns, "deadline not reached");
        for &id in &p.commits {
            let c = self.load_commit(id);
            let settled = match c.status {
                CommitStatus::Expired => true,
                CommitStatus::Verified => !matches!(
                    c.outcome,
                    Outcome::Pending | Outcome::Delayed { .. } | Outcome::Challenged { .. } | Outcome::PayoutFailed { .. }
                ),
                _ => false,
            };
            require!(settled, "escrow stays locked while any commit is pending");
        }
        let remaining = p.available();
        let pm = self.programs.get_mut(program_id).unwrap();
        pm.escrow_withdrawn = true;
        pm.escrow = U128(pm.escrow.0 - remaining);
        emit("escrow_withdrawn", json!({"program_id": program_id, "amount": U128(remaining)}));
        Promise::new(self.token.clone()).function_call(
            "ft_transfer".into(),
            json!({"receiver_id": p.project, "amount": U128(remaining), "memo": "antibody escrow refund"}).to_string().into_bytes(),
            NearToken::from_yoctonear(1),
            FT_GAS,
        )
    }

    // -------------------------------------------------------------- disclosure

    /// After `deadline + disclosure_after`, one TEE run publishes every content key and `k_program`.
    pub fn disclose(&mut self, program_id: u32) -> Promise {
        let p = self.load_program(program_id);
        require!(p.disclosure.is_none() && !p.disclosure_pending, "already disclosed or in flight");
        require!(env::block_timestamp() >= p.deadline_ns + p.disclosure_after_ns, "disclosure time not reached");
        let mut entries = Vec::new();
        for &id in &p.commits {
            if let Some(s) = &self.load_commit(id).sealed {
                entries.push(json!({"commit_id": id, "wrap_v": s.wrap_v}));
            }
        }
        require!(entries.len() <= MAX_DISCLOSURE_ENTRIES, "too many entries for one disclosure run");
        self.programs.get_mut(program_id).unwrap().disclosure_pending = true;
        let input = json!({"mode": "disclose", "program_id": p.key, "entries": entries}).to_string();
        self.outlayer_request(&p.verifier, input).then(
            Self::ext(env::current_account_id()).with_static_gas(SMALL_CB_GAS).on_disclosed(program_id),
        )
    }

    #[private]
    pub fn on_disclosed(&mut self, program_id: u32, #[callback_result] r: Result<Option<DisclosureIn>, PromiseError>) {
        let p = self.programs.get_mut(program_id).unwrap();
        p.disclosure_pending = false;
        match r {
            Ok(Some(d)) if d.program == p.key && is_hex(&d.k_program, 32) && d.keys.iter().all(|k| is_hex(&k.content_key, 32)) => {
                emit("disclosed", json!({"program_id": program_id, "k_program": d.k_program, "keys": d.keys.len()}));
                p.disclosure = Some(Disclosure { k_program: d.k_program, keys: d.keys, height: env::block_height(), ns: env::block_timestamp() });
            }
            _ => emit("disclosure_failed", json!({"program_id": program_id})),
        }
    }

    // ---------------------------------------------------------------- helpers

    fn load_program(&self, id: u32) -> Program {
        self.programs.get(id).cloned().unwrap_or_else(|| env::panic_str("no such program"))
    }

    fn load_commit(&self, id: u64) -> Commit {
        self.commits.get(id as u32).cloned().unwrap_or_else(|| env::panic_str("no such commit"))
    }

    fn commit_mut(&mut self, id: u64) -> &mut Commit {
        self.commits.get_mut(id as u32).unwrap_or_else(|| env::panic_str("no such commit"))
    }

    fn balance(&self, who: &AccountId) -> u128 {
        self.balances.get(who).copied().unwrap_or(0)
    }

    fn credit(&mut self, who: &AccountId, amount: u128) {
        if amount == 0 {
            return;
        }
        let b = self.balance(who);
        self.balances.insert(who.clone(), b + amount);
    }

    fn rep_mut(&mut self, who: &AccountId) -> &mut Reputation {
        if !self.reputation.contains_key(who) {
            self.reputation.insert(who.clone(), Reputation::default());
            self.hunters.push(who.clone());
        }
        self.reputation.get_mut(who).unwrap()
    }

    fn push_program_claim(&mut self, program: u32, key: String) {
        let mut v = self.program_claims.get(&program).cloned().unwrap_or_default();
        v.push(key);
        self.program_claims.insert(program, v);
    }

    fn is_settled(&self, commit_id: u64) -> bool {
        matches!(
            self.load_commit(commit_id).outcome,
            Outcome::Paid { .. } | Outcome::Delayed { .. } | Outcome::Challenged { .. } | Outcome::PayoutFailed { .. }
        )
    }

    // ------------------------------------------------------------------ views

    pub fn get_config(&self) -> Value {
        json!({"owner": self.owner, "token": self.token, "outlayer": self.outlayer, "light_client": self.light_client,
               "verifier": self.verifier, "programs": self.programs.len(), "commits": self.commits.len()})
    }

    pub fn get_program(&self, program_id: u32) -> Option<Program> {
        self.programs.get(program_id).cloned()
    }

    pub fn list_programs(&self, from: Option<u32>, limit: Option<u32>) -> Vec<Program> {
        let from = from.unwrap_or(0);
        let limit = limit.unwrap_or(20).min(50);
        (from..self.programs.len()).take(limit as usize).filter_map(|i| self.programs.get(i).cloned()).collect()
    }

    pub fn get_commit(&self, commit_id: u64) -> Option<Commit> {
        self.commits.get(commit_id as u32).cloned()
    }

    pub fn list_commits(&self, program_id: u32, from: Option<u32>, limit: Option<u32>) -> Vec<Commit> {
        let p = match self.programs.get(program_id) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let from = from.unwrap_or(0) as usize;
        let limit = limit.unwrap_or(50).min(100) as usize;
        p.commits.iter().skip(from).take(limit).filter_map(|&i| self.commits.get(i as u32).cloned()).collect()
    }

    /// Claimed keys of a program: (slot, tag) → claimant, and whether it is a known issue.
    pub fn get_keys(&self, program_id: u32) -> Vec<Value> {
        let p = match self.programs.get(program_id) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let keys = self.program_claims.get(&program_id).cloned().unwrap_or_default();
        keys.iter()
            .filter_map(|k| {
                let cl = self.claims.get(k)?;
                let mut parts = k.splitn(3, ':');
                let _ = parts.next();
                let slot: u8 = parts.next()?.parse().ok()?;
                let tag = parts.next()?.to_string();
                let rule = p.slots.iter().find(|s| s.bit == slot);
                Some(json!({"slot": slot, "name": rule.map(|r| r.name.clone()), "severity": rule.map(|r| r.severity),
                            "tag": tag, "commit": cl.commit, "known": cl.known}))
            })
            .collect()
    }

    /// The proof a duplicate (or known-issue) submission is shown: the earlier claim's commitment,
    /// account, NEAR block and time, equal tags, both receipt hashes, the attestation tier, the earlier
    /// payout, and — after disclosure — both content keys and `k_program` to re-verify everything.
    pub fn get_duplicate_proof(&self, commit_id: u64) -> Option<Value> {
        let c = self.commits.get(commit_id as u32)?;
        let of = match c.outcome {
            Outcome::Duplicate { of } | Outcome::Known { of } => of,
            _ => return None,
        };
        let e = self.commits.get(of as u32)?;
        let p = self.programs.get(c.program)?;
        let mine = c.verdict.as_ref().map(|v| v.keys.clone()).unwrap_or_default();
        let theirs = e.verdict.as_ref().map(|v| v.keys.clone()).unwrap_or_default();
        let equal: Vec<&FindingKey> = mine.iter().filter(|k| theirs.iter().any(|t| t.slot == k.slot && t.tag == k.tag)).collect();
        let content_key = |id: u64| p.disclosure.as_ref().and_then(|d| d.keys.iter().find(|k| k.commit_id == id)).map(|k| k.content_key.clone());
        Some(json!({
            "commit": {"id": c.id, "hunter": c.hunter, "height": c.height, "ns": c.ns, "commit_hash": c.commit_hash,
                       "receipt_hash": c.verdict.as_ref().map(|v| v.receipt_hash.clone()), "keys": mine,
                       "sealed_sha256": c.sealed.as_ref().map(|s| s.sha256.clone())},
            "earlier": {"id": e.id, "hunter": e.hunter, "height": e.height, "ns": e.ns, "commit_hash": e.commit_hash,
                        "known_issue": e.known_issue, "receipt_hash": e.verdict.as_ref().map(|v| v.receipt_hash.clone()),
                        "keys": theirs, "outcome": e.outcome, "sealed_sha256": e.sealed.as_ref().map(|s| s.sha256.clone())},
            "equal_keys": equal,
            "tier": c.verdict.as_ref().map(|v| v.tier.clone()),
            "sealed_until_ns": p.deadline_ns + p.disclosure_after_ns,
            "disclosed": p.disclosure.is_some(),
            "k_program": p.disclosure.as_ref().map(|d| d.k_program.clone()),
            "content_key": content_key(c.id),
            "earlier_content_key": content_key(e.id),
        }))
    }

    pub fn get_balance(&self, account_id: AccountId) -> U128 {
        U128(self.balance(&account_id))
    }

    pub fn get_reputation(&self, account_id: AccountId) -> Reputation {
        self.reputation.get(&account_id).cloned().unwrap_or_default()
    }

    /// Hunters by earnings, then by valid findings.
    pub fn leaderboard(&self, limit: Option<u32>) -> Vec<(AccountId, Reputation)> {
        let mut all: Vec<(AccountId, Reputation)> = self
            .hunters
            .iter()
            .filter_map(|a| self.reputation.get(a).map(|r| (a.clone(), r.clone())))
            .collect();
        all.sort_by(|a, b| {
            let va = a.1.valid_critical + a.1.valid_high + a.1.valid_medium + a.1.valid_low;
            let vb = b.1.valid_critical + b.1.valid_high + b.1.valid_medium + b.1.valid_low;
            b.1.earned.0.cmp(&a.1.earned.0).then(vb.cmp(&va)).then(a.0.cmp(&b.0))
        });
        all.truncate(limit.unwrap_or(20).min(100) as usize);
        all
    }

    pub fn get_stats(&self) -> Stats {
        self.stats.clone()
    }

    /// Helper for clients: the commitment for a sealed blob (docs/crypto-spec.md §5).
    pub fn commitment_for(&self, program_id: u32, hunter: AccountId, sealed_sha256: String) -> String {
        commit_hash_for(&format!("{}/{}", env::current_account_id(), program_id), hunter.as_str(), &sealed_sha256)
    }
}

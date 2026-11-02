//! near-antibody v2: bug bounties that pay on proof (docs/spec.md).
//!
//! Every rule lives in `antibody-rules`, a pure state machine replayed against the Quint model of
//! settlement (formal/antibody.qnt). This contract is its NEAR adapter:
//! - it keeps the state in NEAR storage;
//! - it builds each verifier request from its own records, and calls OutLayer with the program's
//!   pinned build, the requester as payer;
//! - it turns the rule core's effects into promises: token and NEAR transfers, light-client and DCAP
//!   calls, guardian permits;
//! - it emits NEP-297 events.
//!
//! Callbacks never panic: every promise failure becomes a typed event in the rule core.

pub mod events;
pub mod settlement;
pub mod store;
pub mod tee;

#[cfg(test)]
mod tests;

use antibody_rules::{
    audit,
    engine::{NewProgram, RunSpec},
    Account, AnchorStatus, Commit, Ctx, Effect, End, Engine, IdKind, Mode, Program, Run, RunOutcome, Severity, SlotRule, Store, H32,
};
use near_sdk::{
    assert_one_yocto, env,
    json_types::{Base64VecU8, U128},
    log, near, require,
    serde_json::{self, json, Value},
    store::LookupMap,
    AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseError, PromiseOrValue, PromiseResult,
};
use store::NearStore;
use tee::{KnownImage, Measurements};

pub const OUTLAYER_GAS: Gas = Gas::from_tgas(100);
pub const RUN_CB_GAS: Gas = Gas::from_tgas(60);
pub const SMALL_CB_GAS: Gas = Gas::from_tgas(20);
pub const FT_GAS: Gas = Gas::from_tgas(15);
pub const LC_GAS: Gas = Gas::from_tgas(15);
/// The guardian signs one permit per chain (an MPC call and its callback, about 60 Tgas each).
pub const GUARDIAN_GAS: Gas = Gas::from_tgas(200);
/// DCAP `verify_quote` burns about 176 Tgas on testnet (docs/measurements.md).
pub const TEE_VERIFY_GAS: Gas = Gas::from_tgas(200);
pub const TEE_CB_GAS: Gas = Gas::from_tgas(30);
/// Kept from each run's charge to pay for the storage its answer adds (about 1 KB).
pub const RUN_STORAGE_ALLOWANCE: u128 = 10_000_000_000_000_000_000_000; // 0.01 NEAR
/// OutLayer holds extra deposit for `input_data` beyond 32 KB: requests stay within that.
pub const OUTLAYER_FREE_INPUT: usize = 32 * 1024;
/// An inline blob travels hex-encoded in the request; larger ones go through FastFS (`near-tx`).
pub const MAX_INLINE_BLOB: usize = 15 * 1024;

/// The pinned verifier build and how near-antibody runs it on OutLayer.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct VerifierPin {
    pub wasm_url: String,
    /// Lowercase hex SHA-256 of the wasm.
    pub wasm_hash: String,
    pub max_instructions: u64,
    pub max_memory_mb: u32,
    pub max_execution_seconds: u32,
    /// yoctoNEAR attached to each run (OutLayer refunds the unused part to the requester).
    pub deposit: U128,
}

/// What a program pins at creation (docs/spec.md §15). The owner's defaults apply to new programs only.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Pins {
    pub verifier: VerifierPin,
    pub outlayer: AccountId,
    pub dcap_verifier: AccountId,
    pub accepted_tcb: Vec<String>,
    /// OutLayer's approved worker images at creation.
    pub images: Vec<Measurements>,
    /// Images approved later count only after this delay (longer than any challenge window).
    pub image_delay_secs: u64,
    pub light_client: Option<AccountId>,
    /// The chain the light client follows: only anchors on it can be checked.
    pub light_client_chain_id: u64,
    pub guardian: Option<AccountId>,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Defaults {
    pub verifier: VerifierPin,
    pub outlayer: AccountId,
    pub dcap_verifier: AccountId,
    pub accepted_tcb: Vec<String>,
    pub image_delay_secs: u64,
    pub light_client: Option<AccountId>,
    pub light_client_chain_id: u64,
    pub guardian: Option<AccountId>,
    /// Receives the protocol fees (`withdraw_fees`).
    pub treasury: AccountId,
    /// The account whose `get_approved_measurements` lists OutLayer's worker images.
    pub image_registry: AccountId,
}

/// A FastFS upload: `near-tx:<hash>:<signer>` and the SHA-256 of its content.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Loc {
    pub location: String,
    pub sha256: String,
}

/// A sealed blob: inline (hex, at most 15 KB) or a FastFS upload.
#[near(serializers = [json])]
#[derive(Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BlobIn {
    Inline(String),
    NearTx(Loc),
}

/// What a round's release permits allow: upgrading `proxy` to an implementation with this code hash.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// 0x-prefixed EVM address of the proxy.
    pub proxy: String,
    /// keccak256 of the new implementation's runtime code (hex): what the permit commits to.
    pub code_hash: String,
    /// Where that implementation is deployed (the same address on every chain), for relayers. The
    /// adapter checks the code at it against `code_hash`.
    pub implementation: String,
}

fn is_address(a: &str) -> bool {
    let a = a.strip_prefix("0x").unwrap_or("");
    a.len() == 40 && a.bytes().all(|b| b.is_ascii_hexdigit())
}

fn valid_release(r: &Release) {
    require!(is_address(&r.proxy) && is_address(&r.implementation), "a release names 0x-prefixed proxy and implementation addresses");
    require!(hex32(&r.code_hash).is_some(), "a release names a 32-byte code hash");
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramMeta {
    pub name: String,
    pub description: String,
    pub manifest_url: String,
    pub terms_sha256: String,
    /// The project's X25519 key for release wraps (hex).
    pub project_pk: String,
    pub chain_id: u64,
    pub anchor_block: u64,
    pub anchor_hash: String,
    pub targets: Vec<(String, String)>,
    pub slots: Vec<(u8, String, Severity)>,
    /// The base witness, pinned at activation (docs/spec.md §4.4).
    pub base: Option<Loc>,
    /// The manifest of each round after the first.
    pub round_manifests: Vec<Loc>,
    pub proposal_manifest: Option<Loc>,
    /// The releases of each round after the first, and of the proposal.
    pub round_releases: Vec<Vec<Release>>,
    pub proposal_releases: Vec<Release>,
}

#[near(serializers = [json])]
pub struct CreateProgramMsg {
    /// The canonical manifest JSON; its SHA-256 is the program's manifest hash.
    pub manifest: String,
    pub manifest_url: String,
    pub project_pk: String,
}

/// `msg` of `ft_transfer_call` into near-antibody.
#[near(serializers = [json])]
#[serde(rename_all = "snake_case")]
pub enum TokenMsg {
    CreateProgram(CreateProgramMsg),
    TopUp { program_id: u32 },
    Commit { program_id: u32, round: u32, commitment: String, terms_sha256: String },
    Challenge { commit_id: u64, blob: Option<BlobIn> },
    /// Credit for later bonds (an agent's bonds come from its owner's credit).
    Deposit,
}

#[near(serializers = [json])]
pub struct StorageBalance {
    pub total: U128,
    pub available: U128,
}

#[near(serializers = [json])]
pub struct SlotIn {
    pub slot: u8,
    pub severity: Severity,
    pub budget: U128,
}

#[near(
    contract_state,
    contract_metadata(
        version = "2.0.0",
        link = "https://github.com/Himess/antibody",
        standard(standard = "nep145", version = "1.0.0"),
        standard(standard = "nep297", version = "1.0.0"),
        standard(standard = "nep330", version = "1.2.0")
    )
)]
#[derive(PanicOnDefault)]
pub struct Contract {
    owner: AccountId,
    /// The payout token, fixed for the deployment's life: every credit is in this token.
    token: AccountId,
    defaults: Defaults,
    /// OutLayer's worker images known to this deployment, and when each was first seen.
    images: Vec<KnownImage>,
    vetoed: Vec<Measurements>,
    store: NearStore,
    pins: LookupMap<u32, Pins>,
    meta: LookupMap<u32, ProgramMeta>,
    /// FastFS locations of large sealed blobs, by commit (inline blobs are not stored).
    blob_locs: LookupMap<u64, Loc>,
}

fn hex32(s: &str) -> Option<H32> {
    H32::from_hex(s.trim_start_matches("0x"))
}

fn fail(e: antibody_rules::Error) -> ! {
    env::panic_str(&e.to_string())
}

fn now() -> u64 {
    env::block_timestamp() / 1_000_000_000
}

fn valid_loc(l: &Loc) {
    require!(l.location.starts_with("near-tx:") && l.location.len() <= 256, "a location is near-tx:<hash>:<signer>");
    require!(hex32(&l.sha256).is_some(), "sha256: 32 bytes of hex");
}

#[near]
impl Contract {
    #[init]
    pub fn new(owner: AccountId, token: AccountId, defaults: Defaults) -> Self {
        Self {
            owner,
            token,
            defaults,
            images: Vec::new(),
            vetoed: Vec::new(),
            store: NearStore::new(),
            pins: LookupMap::new(b"P".to_vec()),
            meta: LookupMap::new(b"M".to_vec()),
            blob_locs: LookupMap::new(b"B".to_vec()),
        }
    }

    fn ctx(&self) -> Ctx {
        Ctx { now: now(), caller: env::predecessor_account_id().to_string(), signer: env::signer_account_id().to_string() }
    }

    fn engine(&mut self) -> Engine<&mut NearStore> {
        Engine::new(&mut self.store, env::current_account_id().as_str())
    }

    // ---------------------------------------------------------------- owner: defaults for new programs

    /// Defaults for programs created from now on. Live programs keep their pins (docs/spec.md §15).
    #[payable]
    pub fn set_defaults(&mut self, defaults: Defaults) {
        assert_one_yocto();
        require!(env::predecessor_account_id() == self.owner, "owner only");
        events::emit("defaults_changed", json!({"verifier_wasm_hash": defaults.verifier.wasm_hash}));
        self.defaults = defaults;
    }

    /// A veto can only remove an image: verdicts it would have confirmed wait their window instead.
    #[payable]
    pub fn veto_image(&mut self, m: Measurements) {
        assert_one_yocto();
        require!(env::predecessor_account_id() == self.owner, "owner only");
        if !self.vetoed.contains(&m) {
            self.vetoed.push(m.clone());
        }
        events::emit("image_vetoed", json!({"mrtd": m.mrtd}));
    }

    /// Permissionless: copies OutLayer's approved worker images, stamping the new ones with now.
    pub fn sync_images(&mut self) -> Promise {
        Promise::new(self.defaults.image_registry.clone())
            .function_call("get_approved_measurements".into(), b"{}".to_vec(), NearToken::from_yoctonear(0), LC_GAS)
            .then(Self::ext(env::current_account_id()).with_static_gas(SMALL_CB_GAS).on_images())
    }

    #[private]
    pub fn on_images(&mut self, #[callback_result] r: Result<Vec<Measurements>, PromiseError>) -> u32 {
        let Ok(list) = r else { return 0 };
        let mut added = 0;
        for m in list.into_iter().take(256) {
            if !self.images.iter().any(|k| k.m == m) {
                self.images.push(KnownImage { m, added_at: now() });
                added += 1;
            }
        }
        if added > 0 {
            events::emit("images_synced", json!({"added": added, "known": self.images.len()}));
        }
        added
    }

    // ---------------------------------------------------------------- storage (NEP-145 style, spec §12.1)

    #[payable]
    pub fn storage_deposit(&mut self, account_id: Option<AccountId>, registration_only: Option<bool>) -> StorageBalance {
        let _ = registration_only;
        let who = account_id.unwrap_or_else(env::predecessor_account_id);
        let amount = env::attached_deposit().as_yoctonear();
        require!(amount > 0, "attach NEAR");
        let before = env::storage_usage();
        self.engine().near_deposit(who.as_str(), amount).unwrap_or_else(|e| fail(e));
        // A new account's entry is paid from its own deposit.
        self.charge_storage(before, who.as_str());
        self.storage_balance_of(who).unwrap()
    }

    /// One yoctoNEAR attached: a full-access key signs every withdrawal.
    #[payable]
    pub fn storage_withdraw(&mut self, amount: Option<U128>) -> StorageBalance {
        assert_one_yocto();
        let ctx = self.ctx();
        let have = self.store.account(&ctx.caller).near;
        let amount = amount.map(|a| a.0).unwrap_or(have);
        let fx = self.engine().near_withdraw(&ctx, amount).unwrap_or_else(|e| fail(e));
        self.apply(fx);
        self.storage_balance_of(env::predecessor_account_id()).unwrap()
    }

    pub fn storage_balance_bounds(&self) -> Value {
        json!({"min": U128(self.defaults.verifier.deposit.0 + RUN_STORAGE_ALLOWANCE), "max": null})
    }

    pub fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        let a = self.store.account(account_id.as_str());
        Some(StorageBalance { total: U128(a.near), available: U128(a.near) })
    }

    fn flush(&mut self) {
        self.store.flush();
        self.pins.flush();
        self.meta.flush();
        self.blob_locs.flush();
    }

    /// Charges the storage a call added to `payer`'s NEAR credit.
    fn charge_storage(&mut self, before: u64, payer: &str) {
        // Collections cache their writes until the call ends: write them now to measure them.
        self.flush();
        let after = env::storage_usage();
        if after > before {
            let cost = (after - before) as u128 * env::storage_byte_cost().as_yoctonear();
            self.engine().charge_storage(payer, cost).unwrap_or_else(|e| fail(e));
        }
    }

    // ---------------------------------------------------------------- tokens in (NEP-141 receiver)

    pub fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> PromiseOrValue<U128> {
        require!(env::predecessor_account_id() == self.token, "only the payout token");
        let m: TokenMsg = serde_json::from_str(&msg).unwrap_or_else(|_| env::panic_str("unknown message"));
        let before = env::storage_usage();
        let ctx = Ctx { now: now(), caller: sender_id.to_string(), signer: env::signer_account_id().to_string() };
        match m {
            TokenMsg::CreateProgram(c) => {
                let id = self.create_program(&ctx, c, amount.0);
                self.charge_storage(before, sender_id.as_str());
                events::emit("program_escrowed", json!({"program_id": id, "amount": amount}));
            }
            TokenMsg::TopUp { program_id } => {
                let fx = self.engine().top_up(program_id, amount.0).unwrap_or_else(|e| fail(e));
                self.apply(fx);
            }
            TokenMsg::Commit { program_id, round, commitment, terms_sha256 } => {
                let terms_ok = self.terms_ok(program_id, &terms_sha256);
                let cm = hex32(&commitment).unwrap_or_else(|| env::panic_str("commitment: 32 bytes of hex"));
                let (_, fx) = self.engine().commit(&ctx, program_id, round, cm, terms_ok, amount.0, false).unwrap_or_else(|e| fail(e));
                self.apply(fx);
                self.charge_storage(before, sender_id.as_str());
            }
            TokenMsg::Challenge { commit_id, blob } => {
                let c = self.load_commit(commit_id);
                let run_id = self.store.peek_id(IdKind::Run);
                let req = self.request_for(Mode::Second, &c, run_id, blob.as_ref(), json!({}));
                let fx = self.engine().challenge(&ctx, commit_id, amount.0, spec(&req)).unwrap_or_else(|e| fail(e));
                self.apply_run(fx, req, run_id);
            }
            TokenMsg::Deposit => {
                self.engine().token_deposit(sender_id.as_str(), amount.0).unwrap_or_else(|e| fail(e));
                self.charge_storage(before, sender_id.as_str());
            }
        }
        PromiseOrValue::Value(U128(0))
    }

    fn terms_ok(&self, program_id: u32, terms: &str) -> bool {
        self.meta.get(&program_id).map(|m| m.terms_sha256 == terms.trim_start_matches("0x")).unwrap_or(false)
    }

    fn create_program(&mut self, ctx: &Ctx, c: CreateProgramMsg, escrow: u128) -> u32 {
        require!(c.manifest_url.len() <= 512, "manifest_url too long");
        let pk = hex32(&c.project_pk).unwrap_or_else(|| env::panic_str("project_pk: 32 bytes of hex"));
        require!(contributory(&pk.0), "the project key is a low-order point");
        let d = self.defaults.clone();
        let deposit = d.verifier.deposit.0 + RUN_STORAGE_ALLOWANCE;
        let parsed = settlement::parse(&c.manifest, escrow, deposit).unwrap_or_else(|e| env::panic_str(&e));
        let m = &parsed.manifest;
        require!(m.program.project == ctx.caller, "the manifest's project must create the program");
        let manifest_sha256 = H32(env::sha256_array(c.manifest.as_bytes()));
        let checkable = m.anchor.chain_id == d.light_client_chain_id && d.light_client.is_some();
        let images = self.images.iter().map(|k| k.m.clone()).filter(|m| !self.vetoed.contains(m)).collect();
        let pins = Pins {
            verifier: d.verifier.clone(),
            outlayer: d.outlayer.clone(),
            dcap_verifier: d.dcap_verifier.clone(),
            accepted_tcb: d.accepted_tcb.clone(),
            images,
            image_delay_secs: d.image_delay_secs,
            light_client: d.light_client.clone(),
            light_client_chain_id: d.light_client_chain_id,
            guardian: d.guardian.clone(),
        };
        let meta = ProgramMeta {
            name: m.program.name.clone(),
            description: m.program.description.clone(),
            manifest_url: c.manifest_url.clone(),
            terms_sha256: parsed.terms_sha256.clone(),
            project_pk: pk.hex(),
            chain_id: m.anchor.chain_id,
            anchor_block: m.anchor.block_number,
            anchor_hash: m.anchor.block_hash.trim_start_matches("0x").to_string(),
            targets: m.targets.iter().map(|t| (t.name.clone(), t.address.clone())).collect(),
            slots: m.invariants.slots.iter().map(|s| (s.bit, s.name.clone(), s.severity)).collect(),
            base: None,
            round_manifests: vec![],
            proposal_manifest: None,
            round_releases: vec![],
            proposal_releases: vec![],
        };
        let np = NewProgram {
            project: ctx.caller.clone(),
            manifest_sha256,
            params: parsed.params,
            escrow,
            anchor_uncheckable: !checkable,
        };
        let (id, fx) = self.engine().create_program(ctx, np).unwrap_or_else(|e| fail(e));
        self.pins.insert(id, pins);
        self.meta.insert(id, meta);
        self.apply(fx);
        id
    }

    #[private]
    pub fn on_anchor(
        &mut self,
        program_id: u32,
        #[callback_result]
        #[serializer(borsh)]
        r: Result<Option<[u8; 32]>, PromiseError>,
    ) {
        let want = self.meta.get(&program_id).map(|m| m.anchor_hash.clone()).unwrap_or_default();
        let verdict = match r {
            Ok(Some(h)) => Some(hex::encode(h) == want),
            _ => None,
        };
        let r = self.engine().anchor_checked(program_id, verdict);
        match r {
            Ok(fx) => self.apply(fx),
            Err(e) => log!("on_anchor {program_id}: {e}"),
        }
    }

    /// Permissionless while the block is inside the light client's window.
    pub fn recheck_anchor(&mut self, program_id: u32) {
        let fx = self.engine().recheck_anchor(program_id).unwrap_or_else(|e| fail(e));
        self.apply(fx);
    }

    // ---------------------------------------------------------------- program lifecycle

    /// Pins the base witness and asks the verifier for the program's sealing key (T1 binds it).
    pub fn activate(&mut self, program_id: u32, base: Loc) {
        valid_loc(&base);
        let p = self.load_program(program_id);
        require!(env::predecessor_account_id().as_str() == p.project, "only the project activates");
        let mut meta = self.meta.get(&program_id).unwrap().clone();
        meta.base = Some(base);
        self.meta.insert(program_id, meta);
        let run_id = self.store.peek_id(IdKind::Run);
        let req = self.request(Mode::Activate, &p, None, run_id, 0, None, json!({}));
        let ctx = self.ctx();
        let fx = self.engine().activate(&ctx, program_id, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
    }

    pub fn submit_known(&mut self, program_id: u32, blob: BlobIn) {
        let p = self.load_program(program_id);
        let run_id = self.store.peek_id(IdKind::Run);
        let b = blob_json(&blob, None);
        let req = self.request(Mode::Verify, &p, None, run_id, 0, None, json!({"hunter": p.project, "blob": b}));
        let ctx = self.ctx();
        let fx = self.engine().submit_known(&ctx, program_id, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
    }

    pub fn open_program(&mut self, program_id: u32) {
        let ctx = self.ctx();
        let fx = self.engine().open_program(&ctx, program_id).unwrap_or_else(|e| fail(e));
        self.apply(fx);
    }

    /// The project's X25519 key for future release wraps (spec §11.5).
    #[payable]
    pub fn rotate_project_key(&mut self, program_id: u32, project_pk: String) {
        assert_one_yocto();
        let p = self.load_program(program_id);
        require!(env::predecessor_account_id().as_str() == p.project, "only the project");
        let pk = hex32(&project_pk).unwrap_or_else(|| env::panic_str("project_pk: 32 bytes of hex"));
        require!(contributory(&pk.0), "the project key is a low-order point");
        let mut meta = self.meta.get(&program_id).unwrap().clone();
        meta.project_pk = pk.hex();
        self.meta.insert(program_id, meta);
        events::emit("project_key_rotated", json!({"program_id": program_id}));
    }

    /// A fix as a round (spec §8.1). `releases` are the upgrades a passed release gate will permit.
    pub fn propose_round(&mut self, program_id: u32, manifest: Loc, overrides_sha256: String, new_slots: Vec<SlotIn>, releases: Vec<Release>) {
        valid_loc(&manifest);
        require!(releases.len() <= 4, "at most 4 releases per round");
        releases.iter().for_each(valid_release);
        // The TEE checks each release against the code the round's re-runs ran (spec §9).
        let checked: Vec<Value> =
            releases.iter().map(|r| json!({"proxy": r.proxy.to_ascii_lowercase(), "code_hash": r.code_hash.to_ascii_lowercase()})).collect();
        // ... and that the slots it will price are the ones the round manifest adds.
        let named: Vec<Value> = new_slots.iter().map(|s| json!({"slot": s.slot, "severity": s.severity})).collect();
        // ... and that it keeps every invariant of the open round.
        let prev = self.meta.get(&program_id).and_then(|m| m.round_manifests.last().cloned()).map(|l| json!({"near_tx": l.location, "sha256": l.sha256}));
        let p = self.load_program(program_id);
        let mut meta = self.meta.get(&program_id).unwrap().clone();
        meta.proposal_manifest = Some(manifest.clone());
        meta.proposal_releases = releases;
        self.meta.insert(program_id, meta);
        let manifest_sha = hex32(&manifest.sha256).unwrap();
        let overrides = hex32(&overrides_sha256).unwrap_or_else(|| env::panic_str("overrides_sha256: 32 bytes of hex"));
        let slots: Vec<SlotRule> = new_slots.iter().map(|s| SlotRule { slot: s.slot, severity: s.severity, budget: s.budget.0 }).collect();
        let run_id = self.store.peek_id(IdKind::Run);
        let round = p.round + 1;
        let req = self.request_raw(Mode::CheckRound, &p, None, run_id, round, manifest_sha, Some(&manifest), json!({"releases": checked, "new_slots": named, "prev_round_manifest": prev}));
        let ctx = self.ctx();
        let before = env::storage_usage();
        let fx = self.engine().propose_round(&ctx, program_id, manifest_sha, overrides, slots, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
        self.charge_storage(before, &ctx.caller);
    }

    pub fn open_round(&mut self, program_id: u32) {
        let ctx = self.ctx();
        let opening = self.load_program(program_id).proposal.clone();
        let fx = self.engine().open_round(&ctx, program_id).unwrap_or_else(|e| fail(e));
        if opening.is_some() {
            let mut meta = self.meta.get(&program_id).unwrap().clone();
            if let Some(m) = meta.proposal_manifest.take() {
                meta.round_manifests.push(m);
                meta.round_releases.push(std::mem::take(&mut meta.proposal_releases));
            }
            self.meta.insert(program_id, meta);
        }
        self.apply(fx);
    }

    // ---------------------------------------------------------------- hunters

    /// A commit whose bond comes from the beneficiary's token credit (agents, spec §13). An agent's
    /// commit over its cap or budget is refused and recorded as a containment event: `None`.
    pub fn commit(&mut self, program_id: u32, round: u32, commitment: String, terms_sha256: String) -> Option<u64> {
        let ctx = self.ctx();
        let before = env::storage_usage();
        let terms_ok = self.terms_ok(program_id, &terms_sha256);
        let cm = hex32(&commitment).unwrap_or_else(|| env::panic_str("commitment: 32 bytes of hex"));
        let r = self.engine().commit(&ctx, program_id, round, cm, terms_ok, 0, true);
        let (id, fx) = match r {
            Ok(x) => x,
            Err(antibody_rules::Error::Forbidden(why)) if why.starts_with("containment") => {
                self.engine().containment_event(&ctx.caller);
                events::emit("containment", json!({"agent": ctx.caller, "program_id": program_id, "reason": why}));
                return None;
            }
            Err(e) => fail(e),
        };
        self.apply(fx);
        let payer = self.load_commit(id).beneficiary.clone();
        self.charge_storage(before, &payer);
        Some(id)
    }

    /// The reveal: `blob` must hash to the commitment's blob.
    pub fn reveal(&mut self, commit_id: u64, blob: BlobIn) {
        let c = self.load_commit(commit_id);
        let p = self.load_program(c.program);
        let blob_sha = match &blob {
            BlobIn::Inline(h) => {
                let bytes = hex::decode(h).unwrap_or_else(|_| env::panic_str("inline blob: hex"));
                require!(bytes.len() <= MAX_INLINE_BLOB, "inline blobs are at most 15 KB; upload larger ones with FastFS");
                H32(env::sha256_array(&bytes))
            }
            BlobIn::NearTx(l) => {
                valid_loc(l);
                hex32(&l.sha256).unwrap()
            }
        };
        let commitment = commitment_for(&p.key, c.round, &c.hunter, &blob_sha);
        if let BlobIn::NearTx(l) = &blob {
            self.blob_locs.insert(commit_id, l.clone());
        }
        let run_id = self.store.peek_id(IdKind::Run);
        // The commit has no blob hash before this reveal: the request carries the blob just checked.
        let req = self.request_with_blob(Mode::Verify, &c, run_id, blob_json(&blob, Some(blob_sha)), json!({}));
        let ctx = self.ctx();
        let before = env::storage_usage();
        let fx = self.engine().reveal(&ctx, commit_id, commitment, blob_sha, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
        self.charge_storage(before, &c.beneficiary);
    }

    pub fn retry(&mut self, commit_id: u64, blob: Option<BlobIn>) {
        let c = self.load_commit(commit_id);
        let run_id = self.store.peek_id(IdKind::Run);
        let req = self.request_for(Mode::Verify, &c, run_id, blob.as_ref(), json!({}));
        let ctx = self.ctx();
        let fx = self.engine().retry(&ctx, commit_id, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
    }

    pub fn request_second(&mut self, commit_id: u64, blob: Option<BlobIn>) {
        let c = self.load_commit(commit_id);
        let run_id = self.store.peek_id(IdKind::Run);
        let req = self.request_for(Mode::Second, &c, run_id, blob.as_ref(), json!({}));
        let ctx = self.ctx();
        let fx = self.engine().request_second(&ctx, commit_id, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
    }

    pub fn retry_challenge(&mut self, commit_id: u64, blob: Option<BlobIn>) {
        let c = self.load_commit(commit_id);
        let run_id = self.store.peek_id(IdKind::Run);
        let req = self.request_for(Mode::Second, &c, run_id, blob.as_ref(), json!({}));
        let ctx = self.ctx();
        let fx = self.engine().retry_challenge(&ctx, commit_id, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
    }

    /// A distinctness top-up (spec §7.5): `patch` is a FastFS upload of `[(target, code)]`.
    pub fn request_distinct(&mut self, commit_id: u64, earlier_commit: u64, patch: Loc, blob: Option<BlobIn>, earlier_blob: Option<BlobIn>) {
        valid_loc(&patch);
        let c = self.load_commit(commit_id);
        let e = self.load_commit(earlier_commit);
        require!(e.program == c.program && e.index < c.index, "the earlier finding must be an earlier commit of the program");
        let run_id = self.store.peek_id(IdKind::Run);
        let slot = self.exhausted_slot(&c);
        let extra = json!({
            "earlier": {"commit_id": e.id, "hunter": e.hunter, "blob": self.blob_ref(&e, earlier_blob.as_ref())},
            "patch": {"near_tx": patch.location, "sha256": patch.sha256},
            "slot": slot,
        });
        let req = self.request_for(Mode::Distinct, &c, run_id, blob.as_ref(), extra);
        let ctx = self.ctx();
        let fx = self.engine().request_distinct(&ctx, commit_id, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
    }

    fn exhausted_slot(&self, c: &Commit) -> u8 {
        c.run
            .and_then(|r| self.store.run(r))
            .and_then(|r| r.facts().and_then(|f| f.keys.first().map(|k| k.slot)))
            .unwrap_or_else(|| env::panic_str("no verdict"))
    }

    pub fn time_out(&mut self, commit_id: u64) {
        let ctx = self.ctx();
        let fx = self.engine().time_out(&ctx, commit_id).unwrap_or_else(|e| fail(e));
        self.apply(fx);
    }

    pub fn expire(&mut self, commit_id: u64) {
        let ctx = self.ctx();
        let fx = self.engine().expire(&ctx, commit_id).unwrap_or_else(|e| fail(e));
        self.apply(fx);
    }

    // ---------------------------------------------------------------- keeper (permissionless)

    pub fn allocate(&mut self, program_id: u32, max: Option<u32>) {
        let ctx = self.ctx();
        let fx = self.engine().allocate(&ctx, program_id, max.unwrap_or(20).min(50)).unwrap_or_else(|e| fail(e));
        self.apply(fx);
    }

    pub fn pay(&mut self, commit_id: u64) {
        let ctx = self.ctx();
        let fx = self.engine().pay(&ctx, commit_id).unwrap_or_else(|e| fail(e));
        self.apply(fx);
    }

    pub fn settle_debts(&mut self, program_id: u32, max: Option<u32>) {
        let fx = self.engine().settle_debts(program_id, max.unwrap_or(20).min(50)).unwrap_or_else(|e| fail(e));
        self.apply(fx);
    }

    pub fn pause_ticket(&mut self, commit_id: u64) {
        let ctx = self.ctx();
        let fx = self.engine().pause_ticket(&ctx, commit_id).unwrap_or_else(|e| fail(e));
        self.apply(fx);
    }

    pub fn request_rerun(&mut self, commit_id: u64, blob: Option<BlobIn>) {
        let c = self.load_commit(commit_id);
        let p = self.load_program(c.program);
        let pm = self.meta.get(&c.program).and_then(|m| m.proposal_manifest.clone()).unwrap_or_else(|| env::panic_str("no proposal"));
        let pr = p.proposal.clone().unwrap_or_else(|| env::panic_str("no proposal"));
        let run_id = self.store.peek_id(IdKind::Run);
        let extra = json!({"hunter": c.hunter, "blob": self.blob_ref(&c, blob.as_ref())});
        let req = self.request_raw(Mode::Rerun, &p, Some(c.id), run_id, pr.round, pr.manifest_sha256, Some(&pm), extra);
        let ctx = self.ctx();
        let before = env::storage_usage();
        let fx = self.engine().request_rerun(&ctx, commit_id, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
        self.charge_storage(before, &ctx.caller);
    }

    pub fn release_wrap(&mut self, commit_id: u64, blob: Option<BlobIn>) {
        let c = self.load_commit(commit_id);
        let pk = self.meta.get(&c.program).unwrap().project_pk.clone();
        let run_id = self.store.peek_id(IdKind::Run);
        let req = self.request_for(Mode::ReleaseWrap, &c, run_id, blob.as_ref(), json!({"project_pk": pk}));
        let ctx = self.ctx();
        let fx = self.engine().release_wrap(&ctx, commit_id, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
    }

    pub fn request_disclosure(&mut self, program_id: u32, tags: Vec<String>) {
        let p = self.load_program(program_id);
        let parsed: Vec<H32> = tags.iter().map(|t| hex32(t).unwrap_or_else(|| env::panic_str("tag: 32 bytes of hex"))).collect();
        let run_id = self.store.peek_id(IdKind::Run);
        let req = self.request(Mode::Disclose, &p, None, run_id, p.round, None, json!({"tags": tags}));
        let ctx = self.ctx();
        let before = env::storage_usage();
        let fx = self.engine().request_disclosure(&ctx, program_id, &parsed, spec(&req)).unwrap_or_else(|e| fail(e));
        self.apply_run(fx, req, run_id);
        self.charge_storage(before, &ctx.caller);
    }

    pub fn exit(&mut self, program_id: u32) {
        let ctx = self.ctx();
        let fx = self.engine().exit(&ctx, program_id).unwrap_or_else(|e| fail(e));
        self.apply(fx);
    }

    // ---------------------------------------------------------------- T1 (spec §6.1)

    /// Anyone relays a run's quote. near-antibody checks that the attestation describes the run it
    /// requested, has NEAR's DCAP verifier check the quote, and accepts only the program's images.
    pub fn confirm_tee(&mut self, run_id: u64, attestation: tee::TeeAttestationIn, quote: Base64VecU8, collateral: Base64VecU8) -> Promise {
        let r = self.store.run(run_id).unwrap_or_else(|| env::panic_str("no such run"));
        let pins = self.pins.get(&r.program).unwrap().clone();
        let output_sha256 = match &r.outcome {
            RunOutcome::Answered { output_sha256, tier: antibody_rules::Tier::T0, .. } => output_sha256.hex(),
            _ => env::panic_str("the run has no T0 answer to confirm"),
        };
        let rec = tee::RunRecord { wasm_hash: &pins.verifier.wasm_hash, request_sha256: r.request_sha256.hex(), output_sha256, signer: &r.signer };
        tee::check_binding(&attestation, &rec).unwrap_or_else(|e| env::panic_str(e));
        let report_data = hex::encode(attestation.task_hash());
        events::emit("tee_confirm_requested", json!({"run_id": run_id, "outlayer_task_id": attestation.task_id}));
        Promise::new(pins.dcap_verifier.clone())
            .function_call("verify_quote".into(), tee::dcap_verifier_args(&quote.0, &collateral.0), NearToken::from_yoctonear(0), TEE_VERIFY_GAS)
            .then(Self::ext(env::current_account_id()).with_static_gas(TEE_CB_GAS).on_tee(run_id, report_data))
    }

    #[private]
    pub fn on_tee(
        &mut self,
        run_id: u64,
        report_data: String,
        #[callback_result]
        #[serializer(borsh)]
        r: Result<tee::VerificationResultIn, PromiseError>,
    ) -> bool {
        let reject = |reason: String| {
            events::emit("tee_rejected", json!({"run_id": run_id, "reason": reason}));
            false
        };
        let v = match r {
            Ok(tee::VerificationResultIn::Verified(v)) => v,
            Ok(tee::VerificationResultIn::Rejected(tee::VerifierErrorIn::DcapVerification(e))) => return reject(format!("quote rejected: {e}")),
            Err(_) => return reject("DCAP verifier call failed".into()),
        };
        let Some(td) = v.report.as_td10() else { return reject("not a TDX report".into()) };
        if hex::encode(&td.report_data[..32]) != report_data || td.report_data[32..] != [0u8; 32] {
            return reject("the quote does not bind this run".into());
        }
        let Some(run) = self.store.run(run_id) else { return reject("no such run".into()) };
        let pins = self.pins.get(&run.program).unwrap().clone();
        if !pins.accepted_tcb.contains(&v.status) {
            return reject(format!("TCB status {} not accepted", v.status));
        }
        let m = tee::measurements_of(td);
        if !self.image_allowed(&pins, &m) {
            return reject("worker image not pinned for this program".into());
        }
        let ctx = self.ctx();
        let r = self.engine().confirm_t1(&ctx, run_id);
        match r {
            Ok(fx) => {
                self.apply(fx);
                true
            }
            Err(e) => reject(e.to_string()),
        }
    }

    /// The program's images at creation, plus later approvals older than the image delay, minus vetoes.
    fn image_allowed(&self, pins: &Pins, m: &Measurements) -> bool {
        if self.vetoed.contains(m) {
            return false;
        }
        pins.images.contains(m) || self.images.iter().any(|k| &k.m == m && k.added_at + pins.image_delay_secs <= now())
    }

    // ---------------------------------------------------------------- OutLayer runs

    #[private]
    pub fn on_run(&mut self, run_id: u64) {
        let ctx = self.ctx();
        let result = if env::promise_results_count() == 1 { env::promise_result(0) } else { PromiseResult::Failed };
        let out = match result {
            PromiseResult::Successful(bytes) if !bytes.is_empty() && bytes.as_slice() != b"null" => {
                let sha = H32(env::sha256_array(&bytes));
                self.engine().run_answered(&ctx, run_id, &bytes, sha)
            }
            _ => self.engine().run_failed(&ctx, run_id, "OutLayer returned no output"),
        };
        match out {
            Ok(fx) => self.apply(fx),
            Err(e) => log!("on_run {run_id}: {e}"),
        }
    }

    /// Builds the request for a commit's run (verify, second, distinct, release wrap).
    fn request_for(&self, mode: Mode, c: &Commit, run_id: u64, blob: Option<&BlobIn>, extra: Value) -> String {
        let b = self.blob_ref(c, blob);
        self.request_with_blob(mode, c, run_id, b, extra)
    }

    fn request_with_blob(&self, mode: Mode, c: &Commit, run_id: u64, blob: Value, extra: Value) -> String {
        let p = self.load_program(c.program);
        let mut x = json!({"hunter": c.hunter, "blob": blob});
        if let Value::Object(m) = extra {
            for (k, v) in m {
                x[k] = v;
            }
        }
        let rm = if c.round > 0 { self.meta.get(&c.program).unwrap().round_manifests.get(c.round as usize - 1).cloned() } else { None };
        self.request_raw(mode, &p, Some(c.id), run_id, c.round, p.manifest_of(c.round), rm.as_ref(), x)
    }

    /// A commit's blob for a request: its FastFS location, or the inline bytes the caller supplies again
    /// (checked against the committed hash; inline blobs are not stored).
    fn blob_ref(&self, c: &Commit, given: Option<&BlobIn>) -> Value {
        let want = c.blob_sha256.unwrap_or_else(|| env::panic_str("the commit has no blob yet"));
        if let Some(l) = self.blob_locs.get(&c.id) {
            return json!({"near_tx": l.location, "sha256": l.sha256});
        }
        match given {
            Some(b) => blob_json(b, Some(want)),
            None => env::panic_str("pass the commit's inline blob (from its reveal transaction)"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn request(&self, mode: Mode, p: &Program, commit: Option<u64>, run_id: u64, round: u32, rm: Option<&Loc>, extra: Value) -> String {
        self.request_raw(mode, p, commit, run_id, round, p.manifest_of(round), rm, extra)
    }

    /// The verifier's request v2 (crates/verifier/src/tee.rs): canonical JSON, hashed into the run.
    #[allow(clippy::too_many_arguments)]
    fn request_raw(&self, mode: Mode, p: &Program, commit: Option<u64>, run_id: u64, round: u32, manifest: H32, rm: Option<&Loc>, extra: Value) -> String {
        let meta = self.meta.get(&p.id).unwrap();
        let base = meta.base.clone().unwrap_or_else(|| env::panic_str("no base witness pinned (activate)"));
        let mut r = json!({
            "v": 2,
            "mode": mode,
            "program_key": p.key,
            "manifest_sha256": manifest.hex(),
            "round": round,
            "key_epoch": p.key_epoch,
            "commit_id": commit,
            "run_id": run_id,
            "base": {"near_tx": base.location, "sha256": base.sha256},
        });
        if let Some(l) = rm {
            r["round_manifest"] = json!({"near_tx": l.location, "sha256": l.sha256});
        }
        if let Value::Object(m) = extra {
            for (k, v) in m {
                r[k] = v;
            }
        }
        r.to_string()
    }

    // ---------------------------------------------------------------- tokens out (credit, then withdraw)

    /// One yoctoNEAR attached: a full-access key signs every withdrawal (an agent's key cannot).
    #[payable]
    pub fn withdraw(&mut self, amount: U128) {
        assert_one_yocto();
        let ctx = self.ctx();
        let fx = self.engine().withdraw(&ctx, amount.0).unwrap_or_else(|e| fail(e));
        self.apply(fx);
    }

    /// Protocol fees go to the treasury of the deployment's defaults. Permissionless.
    pub fn withdraw_fees(&mut self) {
        let have = self.store.account(antibody_rules::engine::TREASURY).tokens;
        require!(have > 0, "no fees");
        let ctx = Ctx { now: now(), caller: antibody_rules::engine::TREASURY.into(), signer: antibody_rules::engine::TREASURY.into() };
        let fx = self.engine().withdraw(&ctx, have).unwrap_or_else(|e| fail(e));
        let to = self.defaults.treasury.clone();
        let fx = fx
            .into_iter()
            .map(|f| match f {
                Effect::TransferTokens { amount, .. } => Effect::TransferTokens { to: to.to_string(), amount },
                other => other,
            })
            .collect();
        self.apply(fx);
    }

    #[private]
    pub fn on_withdraw(&mut self, account: String, amount: U128, #[callback_result] r: Result<(), PromiseError>) {
        if r.is_err() {
            let r = self.engine().withdraw_failed(&account, amount.0);
            match r {
                Ok(fx) => self.apply(fx),
                Err(e) => log!("on_withdraw: {e}"),
            }
        }
    }

    /// One yoctoNEAR attached: an owner's full-access key grants an agent its budget.
    #[payable]
    pub fn register_agent(&mut self, agent: AccountId, bond_budget: U128, per_commit_cap: U128) {
        assert_one_yocto();
        let ctx = self.ctx();
        let before = env::storage_usage();
        let fx = self.engine().register_agent(&ctx, agent.as_str(), bond_budget.0, per_commit_cap.0).unwrap_or_else(|e| fail(e));
        self.apply(fx);
        self.charge_storage(before, &ctx.caller);
    }

    // ---------------------------------------------------------------- effects

    /// Applies effects that start no run.
    fn apply(&mut self, fx: Vec<Effect>) {
        for f in fx {
            self.effect(f, None);
        }
    }

    /// Applies effects of a call that started run `run_id` with request bytes `req`.
    fn apply_run(&mut self, fx: Vec<Effect>, req: String, run_id: u64) {
        let mut req = Some(req);
        for f in fx {
            let r = match &f {
                Effect::RequestRun { run, .. } if *run == run_id => req.take(),
                _ => None,
            };
            self.effect(f, r);
        }
    }

    fn effect(&mut self, f: Effect, req: Option<String>) {
        match f {
            Effect::Event { name, data } => events::emit_raw(name, &data),
            Effect::RequestRun { run, deposit, payer, .. } => {
                let req = req.unwrap_or_else(|| env::panic_str("a run without its request"));
                require!(req.len() <= OUTLAYER_FREE_INPUT, "the request is over OutLayer's 32 KB of free input: upload the blob with FastFS");
                let r = self.store.run(run).unwrap();
                // Relayers find a run's OutLayer request from the transaction that carries this event.
                events::emit(
                    "run_requested",
                    json!({"run_id": run, "program_id": r.program, "commit_id": r.commit, "mode": r.mode,
                           "request_sha256": r.request_sha256, "payer": payer}),
                );
                let pins = self.pins.get(&r.program).unwrap().clone();
                let attached = deposit.saturating_sub(RUN_STORAGE_ALLOWANCE).min(pins.verifier.deposit.0);
                let args = json!({
                    "source": {"WasmUrl": {"url": pins.verifier.wasm_url, "hash": pins.verifier.wasm_hash, "build_target": "wasm32-wasip2"}},
                    "resource_limits": {"max_instructions": pins.verifier.max_instructions, "max_memory_mb": pins.verifier.max_memory_mb,
                                        "max_execution_seconds": pins.verifier.max_execution_seconds},
                    "input_data": req,
                    "response_format": "Json",
                    "payer_account_id": payer,
                });
                Promise::new(pins.outlayer.clone())
                    .function_call("request_execution".into(), args.to_string().into_bytes(), NearToken::from_yoctonear(attached), OUTLAYER_GAS)
                    .then(Self::ext(env::current_account_id()).with_static_gas(RUN_CB_GAS).on_run(run));
            }
            Effect::CheckAnchor { program } => {
                let pins = self.pins.get(&program).unwrap().clone();
                let meta = self.meta.get(&program).unwrap().clone();
                if let Some(lc) = pins.light_client {
                    Promise::new(lc)
                        .function_call("block_hash_safe".into(), meta.anchor_block.to_le_bytes().to_vec(), NearToken::from_yoctonear(0), LC_GAS)
                        .then(Self::ext(env::current_account_id()).with_static_gas(SMALL_CB_GAS).on_anchor(program));
                }
            }
            Effect::TransferTokens { to, amount } => {
                let token = self.token.clone();
                Promise::new(token)
                    .function_call(
                        "ft_transfer".into(),
                        json!({"receiver_id": to, "amount": U128(amount), "memo": "antibody credit"}).to_string().into_bytes(),
                        NearToken::from_yoctonear(1),
                        FT_GAS,
                    )
                    .then(Self::ext(env::current_account_id()).with_static_gas(SMALL_CB_GAS).on_withdraw(to, U128(amount)));
            }
            Effect::TransferNear { to, amount } => {
                let to: AccountId = to.parse().unwrap_or_else(|_| env::panic_str("account"));
                Promise::new(to).transfer(NearToken::from_yoctonear(amount));
            }
            Effect::PausePermit { program, round, ticket } => self.permit("issue_pause", program, round, Some(ticket)),
            Effect::ReleasePermit { program, round } => self.permit("issue_release", program, round, None),
        }
    }

    /// Asks the pinned guardian to sign a permit (docs/spec.md §9–§10). EVM fees never live on NEAR.
    fn permit(&mut self, method: &str, program: u32, round: u32, ticket: Option<u64>) {
        let pins = self.pins.get(&program).unwrap().clone();
        let p = self.load_program(program);
        let Some(g) = pins.guardian else {
            log!("{method}: program {program} has no guardian");
            return;
        };
        let args = match ticket {
            Some(t) => json!({"program_key": p.key, "project": p.project, "round": round, "ticket": t}),
            None => {
                let releases = self.meta.get(&program).and_then(|m| m.round_releases.get(round as usize - 1).cloned()).unwrap_or_default();
                json!({"program_key": p.key, "project": p.project, "round": round, "releases": releases})
            }
        };
        events::emit("permit_requested", json!({"program_id": program, "guardian": g, "method": method, "round": round, "ticket": ticket}));
        Promise::new(g).function_call(method.into(), args.to_string().into_bytes(), NearToken::from_yoctonear(0), GUARDIAN_GAS);
    }

    // ---------------------------------------------------------------- views

    fn load_program(&self, id: u32) -> Program {
        self.store.program(id).unwrap_or_else(|| env::panic_str("no such program"))
    }

    fn load_commit(&self, id: u64) -> Commit {
        self.store.commit(id).unwrap_or_else(|| env::panic_str("no such commit"))
    }

    pub fn get_owner(&self) -> AccountId {
        self.owner.clone()
    }

    pub fn get_token(&self) -> AccountId {
        self.token.clone()
    }

    /// Contract-wide totals and the next ids (runs are numbered from 0).
    pub fn get_stats(&self) -> Value {
        json!({"totals": self.store.totals(), "next_program": self.store.peek_id(IdKind::Program),
               "next_commit": self.store.peek_id(IdKind::Commit), "next_run": self.store.peek_id(IdKind::Run)})
    }

    pub fn get_defaults(&self) -> Defaults {
        self.defaults.clone()
    }

    pub fn get_images(&self) -> (Vec<KnownImage>, Vec<Measurements>) {
        (self.images.clone(), self.vetoed.clone())
    }

    pub fn get_program(&self, program_id: u32) -> Option<Program> {
        self.store.program(program_id)
    }

    pub fn get_program_meta(&self, program_id: u32) -> Option<ProgramMeta> {
        self.meta.get(&program_id).cloned()
    }

    pub fn get_pins(&self, program_id: u32) -> Option<Pins> {
        self.pins.get(&program_id).cloned()
    }

    pub fn list_programs(&self, from: Option<u32>, limit: Option<u32>) -> Vec<Program> {
        let from = from.unwrap_or(0);
        let n = self.store.peek_id(IdKind::Program) as u32;
        (from..n.min(from + limit.unwrap_or(20).min(50))).filter_map(|i| self.store.program(i)).collect()
    }

    pub fn get_commit(&self, commit_id: u64) -> Option<Commit> {
        self.store.commit(commit_id)
    }

    pub fn list_commits(&self, program_id: u32, from: Option<u32>, limit: Option<u32>) -> Vec<Commit> {
        let p = match self.store.program(program_id) {
            Some(p) => p,
            None => return vec![],
        };
        let from = from.unwrap_or(0);
        (from..p.commits.min(from + limit.unwrap_or(20).min(50)))
            .filter_map(|i| self.store.commit_at(program_id, i).and_then(|c| self.store.commit(c)))
            .collect()
    }

    pub fn get_run(&self, run_id: u64) -> Option<Run> {
        self.store.run(run_id)
    }

    pub fn get_holder(&self, program_id: u32, round: u32, tag: String) -> Option<u64> {
        hex32(&tag).and_then(|t| self.store.holder(program_id, round, &t))
    }

    pub fn get_account(&self, account_id: AccountId) -> Account {
        self.store.account(account_id.as_str())
    }

    /// Both sides of every conservation equation (docs/spec.md §7.10).
    pub fn audit(&self, program_id: u32) -> Option<audit::ProgramAudit> {
        audit::program(&self.store, program_id)
    }

    pub fn audit_contract(&self) -> audit::ContractAudit {
        audit::contract(&self.store)
    }

    /// An agent's statement (spec §13): bonds, verification spend, payouts, net, containment events.
    pub fn agent_statement(&self, agent: AccountId) -> Value {
        let a = self.store.account(agent.as_str());
        match a.agent {
            None => json!({"agent": agent, "registered": false}),
            Some(g) => {
                let net = g.paid as i128 - g.bonds_lost as i128;
                json!({
                    "agent": agent, "registered": true, "owner": g.owner,
                    "bond_budget": U128(g.bond_budget), "per_commit_cap": U128(g.per_commit_cap),
                    "bonds_locked": U128(g.bonds_locked), "bonds_lost": U128(g.bonds_lost), "bonds_returned": U128(g.bonds_returned),
                    "paid": U128(g.paid), "verification_spent_yocto": U128(g.verification_spent),
                    "net_tokens": net.to_string(), "commits": g.commits, "containment_events": g.containment_events,
                    "key_permissions": "commit, reveal, upload and the permissionless keeper methods; no withdrawal, transfer or deposit",
                })
            }
        }
    }

    /// The program's record (spec §9): survived, at stake, verified attempts; and bait metrics.
    pub fn program_record(&self, program_id: u32) -> Option<Value> {
        let p = self.store.program(program_id)?;
        let mut verdicts = 0u32;
        let mut paid = 0u32;
        for i in 0..p.commits {
            if let Some(c) = self.store.commit_at(program_id, i).and_then(|c| self.store.commit(c)) {
                if c.run.and_then(|r| self.store.run(r)).map(|r| r.facts().is_some()).unwrap_or(false) {
                    verdicts += 1;
                }
                if matches!(c.state, antibody_rules::CommitState::Done(End::Paid { .. })) {
                    paid += 1;
                }
            }
        }
        let since = p.rounds.last().map(|r| r.opened_at).or(p.opened_at);
        Some(json!({
            "program_id": program_id,
            "phase": p.phase,
            "anchor": match p.anchor { AnchorStatus::LightClient => "light-client", AnchorStatus::Unconfirmed => "unconfirmed", AnchorStatus::Pending => "pending" },
            "survived_secs": since.map(|s| now().saturating_sub(s)),
            "at_stake": U128(p.ledger.available + p.ledger.reserved),
            "verified_attempts": verdicts,
            "paid_findings": paid,
            "rounds": p.rounds.len(),
            "release_permits": p.release_permits,
            "time_to_first_claim_secs": match (p.opened_at, p.first_claim_at) { (Some(o), Some(f)) => Some(f.saturating_sub(o)), _ => None },
            "share_claimed_first_hour_bps": (p.paid_first_hour * 10_000).checked_div(p.ledger.deposits).unwrap_or(0) as u64,
            "defaulted": U128(p.defaulted),
        }))
    }
}

/// The commitment a reveal must match (spec §4.1).
pub fn commitment_for(program_key: &str, round: u32, hunter: &str, blob_sha256: &H32) -> H32 {
    let mut m = b"antibody/commit/v2".to_vec();
    m.extend_from_slice(&(program_key.len() as u32).to_be_bytes());
    m.extend_from_slice(program_key.as_bytes());
    m.extend_from_slice(&round.to_be_bytes());
    m.extend_from_slice(&(hunter.len() as u32).to_be_bytes());
    m.extend_from_slice(hunter.as_bytes());
    m.extend_from_slice(&blob_sha256.0);
    H32(env::sha256_array(&m))
}

fn spec(req: &str) -> RunSpec {
    RunSpec { request_sha256: H32(env::sha256_array(req.as_bytes())) }
}

/// A blob for a request; an inline blob must hash to `want` when given.
fn blob_json(b: &BlobIn, want: Option<H32>) -> Value {
    match b {
        BlobIn::Inline(h) => {
            let bytes = hex::decode(h).unwrap_or_else(|_| env::panic_str("inline blob: hex"));
            require!(bytes.len() <= MAX_INLINE_BLOB, "inline blobs are at most 15 KB; upload larger ones with FastFS");
            if let Some(w) = want {
                require!(env::sha256_array(&bytes) == w.0, "the inline blob does not match the committed hash");
            }
            json!({"inline": h})
        }
        BlobIn::NearTx(l) => {
            valid_loc(l);
            if let Some(w) = want {
                require!(hex32(&l.sha256) == Some(w), "the blob location's hash does not match the committed hash");
            }
            json!({"near_tx": l.location, "sha256": l.sha256})
        }
    }
}

/// Low-order X25519 points are refused (spec §11.5): the eight small-order encodings and their
/// non-canonical twins.
pub fn contributory(pk: &[u8; 32]) -> bool {
    const LOW: [[u8; 32]; 2] = [[0u8; 32], {
        let mut x = [0u8; 32];
        x[0] = 1;
        x
    }];
    if LOW.contains(pk) {
        return false;
    }
    let bad: [&str; 5] = [
        "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800",
        "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
    ];
    let h = hex::encode(pk);
    !bad.contains(&h.as_str())
}

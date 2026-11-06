//! near-t2: validity proofs of disclosed verdicts (T2, docs/zk.md).
//!
//! Once a verdict's proof is disclosed, anyone can re-run the verifier on it. T2 makes that re-run a
//! Groth16 proof (SP1 6.10.0, the program `crates/zk/guest`) that this contract checks on NEAR, and then
//! compares with near-antibody's own records of the run. It claims no privacy: the proof is about a
//! proof that is already public. Payouts in the locked deployment do not depend on it.
//!
//! It pins, at creation: SP1 6.10.0's Groth16 key (built in, `vk.rs`), the guest's program vkey (the
//! docker build's), near-antibody's account, and the verifier the guest re-runs (its version string and
//! the TEE wasm's sha256 that programs pin). It has no owner and no method that changes them.
//!
//! `submit(run_id, public_values, proof)`, by anyone: the proof must verify (≈ 34 Tgas). The contract
//! then reads the run, its commit, the program, its metadata and its pins from near-antibody and compares
//! every bound field ([`compare`]). Equal: the run is T2, `t2_confirmed`. The same input with another
//! receipt, kind or slots: a contradiction is recorded, `t2_contradicts`. It reverses nothing in the
//! locked deployment; consumers may act on it. Anything else is rejected (`t2_rejected`, nothing stored).
//! The submitter prepays the record's storage; the unused part is refunded.

use near_sdk::{
    env, ext_contract,
    json_types::{Base64VecU8, U128, U64},
    near, require,
    serde_json::{json, Value},
    AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseError,
};

pub mod compare;
pub mod groth16;
pub mod pv;
pub mod vk;

pub use groth16::{Groth16KeyJson, Sp1Groth16Vk};
use groth16::{hex, unhex};

pub const VIEW_GAS: Gas = Gas::from_tgas(10);
pub const CALLBACK_GAS: Gas = Gas::from_tgas(50);
/// Prepaid for a record's storage (about 0.004 NEAR is used); the rest is refunded.
pub const RECORD_DEPOSIT: NearToken = NearToken::from_millinear(20);
/// Records keep at most this many broken slots (a receipt lists every broken invariant).
pub const MAX_SLOTS: usize = 16;

pub const STANDARD: &str = "antibody-t2";
pub const VERSION: &str = "1.0.0";

fn emit(event: &str, data: Value) {
    env::log_str(&format!("EVENT_JSON:{}", json!({"standard": STANDARD, "version": VERSION, "event": event, "data": [data]})));
}

#[ext_contract(ext_antibody)]
#[allow(dead_code)]
trait Antibody {
    fn get_run(&self, run_id: u64) -> Option<Value>;
    fn get_commit(&self, commit_id: u64) -> Option<Value>;
    fn get_program(&self, program_id: u32) -> Option<Value>;
    fn get_program_meta(&self, program_id: u32) -> Option<Value>;
    fn get_pins(&self, program_id: u32) -> Option<Value>;
}

/// A run's T2 record.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub run_id: u64,
    pub program_id: u32,
    pub commit_id: u64,
    pub round: u32,
    /// Empty: confirmed. Otherwise the outputs that differ ("receipt", "kind", "slots").
    pub differ: Vec<String>,
    pub request_sha256: [u8; 32],
    /// The receipt the proof re-ran to, and the one near-antibody recorded from the TEE.
    pub receipt_sha256: [u8; 32],
    pub tee_receipt_sha256: [u8; 32],
    pub kind: u8,
    pub tee_kind: String,
    /// (slot, severity) proven; the TEE's keys' slots.
    pub slots: Vec<(u8, u8)>,
    pub tee_slots: Vec<u8>,
    pub tee_tier: String,
    pub public_values_sha256: [u8; 32],
    pub proof_sha256: [u8; 32],
    pub submitter: AccountId,
    pub at_ns: u64,
    pub block: u64,
}

impl Record {
    pub fn to_json(&self) -> Value {
        json!({
            "run_id": self.run_id,
            "program_id": self.program_id,
            "commit_id": (self.commit_id != pv::KNOWN_ISSUE_COMMIT).then_some(self.commit_id),
            "round": self.round,
            "outcome": if self.differ.is_empty() { "confirmed" } else { "contradicts" },
            "differ": self.differ,
            "request_sha256": hex(&self.request_sha256),
            "receipt_sha256": hex(&self.receipt_sha256),
            "tee_receipt_sha256": hex(&self.tee_receipt_sha256),
            "kind": pv::kind_name(self.kind),
            "tee_kind": self.tee_kind,
            "slots": self.slots.iter().map(|(s, v)| json!({"slot": s, "severity": pv::severity_name(*v)})).collect::<Vec<_>>(),
            "tee_slots": self.tee_slots,
            "tee_tier": self.tee_tier,
            "public_values_sha256": hex(&self.public_values_sha256),
            "proof_sha256": hex(&self.proof_sha256),
            "submitter": self.submitter,
            "at": self.at_ns / 1_000_000_000,
            "block": self.block,
        })
    }
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct T2 {
    antibody: AccountId,
    program_vkey: [u8; 32],
    groth16: Sp1Groth16Vk,
    verifier: String,
    verifier_wasm: String,
    /// Records so far: `o<i>` holds the i-th record's run id, `r<run>` the record.
    records: u32,
    confirmed: u32,
    contradicted: u32,
}

fn rkey(run: u64) -> Vec<u8> {
    [b"r".as_slice(), &run.to_le_bytes()].concat()
}

fn okey(i: u32) -> Vec<u8> {
    [b"o".as_slice(), &i.to_le_bytes()].concat()
}

fn load(run: u64) -> Option<Record> {
    env::storage_read(&rkey(run)).and_then(|b| near_sdk::borsh::from_slice(&b).ok())
}

fn h32(v: &Value) -> [u8; 32] {
    v.as_str().and_then(|s| unhex::<32>(s).ok()).unwrap_or_default()
}

#[near]
impl T2 {
    /// `groth16`: omit it for SP1 6.10.0's key (built in); the sandbox passes a TEST key.
    #[init]
    pub fn new(antibody: AccountId, program_vkey: String, verifier: String, verifier_wasm: String, groth16: Option<Groth16KeyJson>) -> Self {
        let program_vkey = unhex::<32>(&program_vkey).unwrap_or_else(|e| env::panic_str(e));
        let groth16 = match groth16 {
            None => vk::SP1_V6_10_0,
            Some(j) => Sp1Groth16Vk::try_from(&j).unwrap_or_else(|e| env::panic_str(e)),
        };
        require!(unhex::<32>(&verifier_wasm).is_ok(), "verifier_wasm is a sha256 (hex)");
        Self {
            antibody,
            program_vkey,
            groth16,
            verifier,
            verifier_wasm: verifier_wasm.to_lowercase(),
            records: 0,
            confirmed: 0,
            contradicted: 0,
        }
    }

    /// Anyone. Verifies the proof, then reads near-antibody's records of the run and compares them
    /// (`on_records`). Attach 0.02 NEAR for the record's storage (the unused part is refunded) and about
    /// 200 Tgas.
    #[payable]
    pub fn submit(&mut self, run_id: U64, public_values: Base64VecU8, proof: Base64VecU8) -> Promise {
        let deposit = env::attached_deposit();
        require!(deposit >= RECORD_DEPOSIT, "attach 0.02 NEAR for the record's storage (the unused part is refunded)");
        require!(!env::storage_has_key(&rkey(run_id.0)), "this run already has a T2 record");
        let pv = pv::decode(&public_values.0).unwrap_or_else(|e| env::panic_str(e));
        require!(pv.run_id == run_id.0, "the proof is about another run");
        let program_id = compare::program_id(self.antibody.as_str(), &pv.program_key).unwrap_or_else(|e| env::panic_str(e));
        match groth16::verify(&self.groth16, &self.program_vkey, &public_values.0, &proof.0) {
            Ok(true) => {}
            Ok(false) => env::panic_str("the proof does not verify"),
            Err(e) => env::panic_str(e),
        }
        let proof_sha256 = hex(&env::sha256_array(&proof.0));
        let ab = || ext_antibody::ext(self.antibody.clone()).with_static_gas(VIEW_GAS);
        ab().get_run(run_id.0)
            .and(ab().get_commit(pv.commit_id))
            .and(ab().get_program(program_id))
            .and(ab().get_program_meta(program_id))
            .and(ab().get_pins(program_id))
            .then(Self::ext(env::current_account_id()).with_static_gas(CALLBACK_GAS).on_records(
                public_values,
                proof_sha256,
                env::predecessor_account_id(),
                U128(deposit.as_yoctonear()),
            ))
    }

    /// Compares and records. It never panics: a rejection refunds the deposit.
    #[private]
    #[allow(clippy::too_many_arguments)]
    pub fn on_records(
        &mut self,
        public_values: Base64VecU8,
        proof_sha256: String,
        submitter: AccountId,
        deposit: U128,
        #[callback_result] run: Result<Option<Value>, PromiseError>,
        #[callback_result] commit: Result<Option<Value>, PromiseError>,
        #[callback_result] program: Result<Option<Value>, PromiseError>,
        #[callback_result] meta: Result<Option<Value>, PromiseError>,
        #[callback_result] pins: Result<Option<Value>, PromiseError>,
    ) -> bool {
        let reject = |run_id: u64, why: &str| {
            emit("t2_rejected", json!({"run_id": run_id, "reason": why, "submitter": submitter}));
            let _ = Promise::new(submitter.clone()).transfer(NearToken::from_yoctonear(deposit.0));
            false
        };
        let Ok(pv) = pv::decode(&public_values.0) else { return reject(0, "public values") };
        if env::storage_has_key(&rkey(pv.run_id)) {
            return reject(pv.run_id, "this run already has a T2 record");
        }
        let (Ok(Some(run)), Ok(commit), Ok(Some(program)), Ok(Some(meta)), Ok(Some(pins))) = (run, commit, program, meta, pins) else {
            return reject(pv.run_id, "near-antibody's records could not be read");
        };
        let pinned = compare::Pinned { antibody: self.antibody.as_str(), verifier: &self.verifier, verifier_wasm: &self.verifier_wasm };
        let chain = compare::Chain { run: &run, commit: commit.as_ref(), program: &program, meta: &meta, pins: &pins };
        let differ = match compare::compare(&pinned, &pv, &chain) {
            Err(why) => return reject(pv.run_id, why),
            Ok(compare::Outcome::Confirmed) => vec![],
            Ok(compare::Outcome::Contradicts(d)) => d.into_iter().map(String::from).collect::<Vec<_>>(),
        };
        let facts = &run["outcome"]["Answered"]["facts"];
        let record = Record {
            run_id: pv.run_id,
            program_id: program["id"].as_u64().unwrap_or_default() as u32,
            commit_id: pv.commit_id,
            round: pv.round,
            differ,
            request_sha256: pv.request_sha256,
            receipt_sha256: pv.receipt_sha256,
            tee_receipt_sha256: h32(&facts["receipt_sha256"]),
            kind: pv.kind,
            tee_kind: facts["kind"].as_str().unwrap_or_default().chars().take(32).collect(),
            slots: pv.broken.iter().take(MAX_SLOTS).copied().collect(),
            tee_slots: facts["keys"].as_array().into_iter().flatten().filter_map(|k| k["slot"].as_u64()).take(MAX_SLOTS).map(|s| s as u8).collect(),
            tee_tier: run["outcome"]["Answered"]["tier"].as_str().unwrap_or_default().chars().take(8).collect(),
            public_values_sha256: env::sha256_array(&public_values.0),
            proof_sha256: unhex::<32>(&proof_sha256).unwrap_or_default(),
            submitter: submitter.clone(),
            at_ns: env::block_timestamp(),
            block: env::block_height(),
        };
        let before = env::storage_usage();
        env::storage_write(&rkey(pv.run_id), &near_sdk::borsh::to_vec(&record).unwrap_or_default());
        env::storage_write(&okey(self.records), &pv.run_id.to_le_bytes());
        let cost = env::storage_byte_cost().saturating_mul((env::storage_usage().saturating_sub(before)) as u128);
        if cost.as_yoctonear() > deposit.0 {
            // Cannot happen with the bounds above; never let a submitter's record cost the contract.
            env::storage_remove(&rkey(pv.run_id));
            env::storage_remove(&okey(self.records));
            return reject(pv.run_id, "the record needs more storage than was attached");
        }
        self.records += 1;
        if record.differ.is_empty() {
            self.confirmed += 1;
            emit("t2_confirmed", record.to_json());
        } else {
            self.contradicted += 1;
            emit("t2_contradicts", record.to_json());
        }
        let refund = deposit.0 - cost.as_yoctonear();
        if refund > 0 {
            let _ = Promise::new(submitter).transfer(NearToken::from_yoctonear(refund));
        }
        true
    }

    /// Checks a proof against the pinned keys only (no records): for tools and judges.
    pub fn verify_proof(&self, public_values: Base64VecU8, proof: Base64VecU8) -> bool {
        groth16::verify(&self.groth16, &self.program_vkey, &public_values.0, &proof.0).unwrap_or(false)
    }

    pub fn get_pins(&self) -> Value {
        json!({
            "antibody": self.antibody,
            "program_vkey": format!("0x{}", hex(&self.program_vkey)),
            "groth16": {
                "selector": format!("0x{}", hex(&self.groth16.selector)),
                "vk_root": format!("0x{}", hex(&self.groth16.vk_root)),
                "is_sp1_v6_10_0": self.groth16 == vk::SP1_V6_10_0,
            },
            "verifier": self.verifier,
            "verifier_wasm": self.verifier_wasm,
            "record_deposit": U128(RECORD_DEPOSIT.as_yoctonear()),
        })
    }

    pub fn get_record(&self, run_id: U64) -> Option<Value> {
        load(run_id.0).map(|r| r.to_json())
    }

    /// Records in the order they were made.
    pub fn list_records(&self, from: Option<u32>, limit: Option<u32>) -> Vec<Value> {
        let from = from.unwrap_or(0);
        let limit = limit.unwrap_or(50).min(100);
        (from..self.records.min(from.saturating_add(limit)))
            .filter_map(|i| env::storage_read(&okey(i)))
            .filter_map(|b| b.try_into().ok().map(u64::from_le_bytes))
            .filter_map(|id| load(id).map(|r| r.to_json()))
            .collect()
    }

    pub fn get_stats(&self) -> Value {
        json!({"records": self.records, "confirmed": self.confirmed, "contradicted": self.contradicted})
    }
}

#[cfg(test)]
mod tests;

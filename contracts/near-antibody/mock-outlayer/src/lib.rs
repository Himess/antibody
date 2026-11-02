//! MOCK of OutLayer's `request_execution`, for the offline sandbox demo only (never on testnet).
//!
//! It keeps OutLayer's call shape and delivery semantics: the request yields (`promise_yield_create`),
//! a worker resolves it (`promise_yield_resume`), and the caller's `.then` receives the guest's raw
//! JSON stdout, or `null` on failure or timeout. The mock worker (`antibody-services worker-mock`)
//! runs the real verifier in TEE mode with a TEST root key.
//!
//! OutLayer publishes an attestation per execution and lists its approved worker images; the mock
//! does both with MOCK values. A MOCK attestation is what the mock worker reports about the run, and
//! its MOCK quote is plain JSON that only `mock-dcap` accepts: no TDX hardware is involved.

use near_sdk::{
    env, json_types::U128, near, require, serde_json::{json, Value}, store::{IterableMap, LookupMap}, AccountId, Gas,
    GasWeight, PanicOnDefault, Promise, PromiseResult,
};

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Pending {
    pub request_id: u64,
    pub data_id: String,
    pub caller: AccountId,
    /// The transaction's signer: OutLayer attests it as the run's caller.
    pub signer: AccountId,
    pub source: String,
    pub resource_limits: String,
    pub input_data: String,
    pub secrets_ref: Option<String>,
    pub height: u64,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockOutLayer {
    worker: AccountId,
    next: u64,
    pending: IterableMap<u64, Pending>,
    resolved: u64,
    /// Outputs waiting for their callback. As OutLayer does with `submit_execution_output`, the output
    /// is kept in state and the yield is resumed with a small marker: `promise_yield_resume` carries at
    /// most 1,024 bytes, and a disclosure run's output is larger than that.
    outputs: LookupMap<u64, String>,
    /// MOCK attestations by request id, and the request id of each input hash.
    attestations: LookupMap<u64, Attested>,
    by_input: LookupMap<String, u64>,
    /// MOCK worker images (`get_approved_measurements`), as JSON text.
    images: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Attested {
    pub request_id: u64,
    /// OutLayer's attestation fields, as JSON text.
    pub attestation: String,
    /// The MOCK quote, base64.
    pub quote: String,
}

fn emit(event: &str, data: Value) {
    let ev = json!({"standard": "mock-outlayer", "version": "1.0.0", "event": event, "data": [data]});
    env::log_str(&format!("EVENT_JSON:{ev}"));
}

#[near]
impl MockOutLayer {
    #[init]
    pub fn new(worker: AccountId, images: Vec<Value>) -> Self {
        Self {
            worker,
            next: 0,
            pending: IterableMap::new(b"p"),
            resolved: 0,
            outputs: LookupMap::new(b"o"),
            attestations: LookupMap::new(b"a"),
            by_input: LookupMap::new(b"i"),
            images: Value::Array(images).to_string(),
        }
    }

    #[payable]
    #[allow(clippy::too_many_arguments)]
    pub fn request_execution(
        &mut self,
        source: Value,
        resource_limits: Option<Value>,
        input_data: Option<String>,
        secrets_ref: Option<Value>,
        response_format: Option<String>,
        payer_account_id: Option<AccountId>,
        params: Option<Value>,
    ) {
        let _ = (response_format, params);
        require!(resource_limits.is_some(), "resource_limits is required (compile-only is not supported)");
        let id = self.next;
        self.next += 1;
        let idx = env::promise_yield_create(
            "on_execution_response",
            json!({"request_id": id}).to_string().as_bytes(),
            Gas::from_tgas(10),
            GasWeight(0),
            0,
        );
        let data_id: [u8; 32] = env::read_register(0).expect("yield data id").try_into().expect("32-byte data id");
        let caller = env::predecessor_account_id();
        self.pending.insert(
            id,
            Pending {
                request_id: id,
                data_id: data_id.iter().map(|b| format!("{b:02x}")).collect(),
                caller: caller.clone(),
                signer: env::signer_account_id(),
                source: source.to_string(),
                resource_limits: resource_limits.map(|r| r.to_string()).unwrap_or_default(),
                input_data: input_data.clone().unwrap_or_default(),
                secrets_ref: secrets_ref.map(|s| s.to_string()),
                height: env::block_height(),
            },
        );
        emit("execution_requested", json!({"request_id": id, "caller": caller, "input_data": input_data}));
        // The mock is free: return any attached deposit.
        let deposit = env::attached_deposit();
        if !deposit.is_zero() {
            Promise::new(payer_account_id.unwrap_or(caller)).transfer(deposit);
        }
        env::promise_return(idx);
    }

    /// Worker-only: delivers the guest's stdout (JSON) or `None` (failure) to the waiting caller, with
    /// the run's MOCK attestation and quote.
    pub fn resolve_execution(&mut self, request_id: u64, output: Option<String>, attestation: Option<Value>, quote: Option<String>) {
        require!(env::predecessor_account_id() == self.worker, "only the worker resolves");
        let p = self.pending.remove(&request_id).unwrap_or_else(|| env::panic_str("unknown or already resolved request"));
        let payload = output.unwrap_or_else(|| "null".into());
        require!(near_sdk::serde_json::from_str::<Value>(&payload).is_ok(), "output must be JSON");
        let mut data_id = [0u8; 32];
        for (i, b) in data_id.iter_mut().enumerate() {
            *b = u8::from_str_radix(&p.data_id[2 * i..2 * i + 2], 16).unwrap();
        }
        let bytes = payload.len();
        if let (Some(a), Some(q)) = (attestation, quote) {
            let input_hash = a["input_hash"].as_str().unwrap_or_default().to_string();
            self.by_input.insert(input_hash, request_id);
            self.attestations.insert(request_id, Attested { request_id, attestation: a.to_string(), quote: q });
        }
        self.outputs.insert(request_id, payload);
        require!(env::promise_yield_resume(&data_id, b"{}"), "the request already timed out");
        self.resolved += 1;
        emit("execution_resolved", json!({"request_id": request_id, "bytes": bytes}));
    }

    #[private]
    pub fn on_execution_response(&mut self, request_id: u64) {
        let stored = self.outputs.remove(&request_id);
        let out = match env::promise_result(0) {
            PromiseResult::Successful(_) => stored.map(String::into_bytes).unwrap_or_else(|| b"null".to_vec()),
            PromiseResult::Failed => {
                self.pending.remove(&request_id);
                b"null".to_vec()
            }
        };
        env::value_return(&out);
    }

    pub fn get_pending(&self, limit: Option<u32>) -> Vec<Pending> {
        self.pending.values().take(limit.unwrap_or(50) as usize).cloned().collect()
    }

    pub fn get_attestation(&self, request_id: u64) -> Option<Attested> {
        self.attestations.get(&request_id).cloned()
    }

    /// The attestation of the run whose input hashes to `input_hash` (lowercase hex).
    pub fn find_attestation(&self, input_hash: String) -> Option<Attested> {
        self.by_input.get(&input_hash).and_then(|id| self.attestations.get(id)).cloned()
    }

    pub fn get_approved_measurements(&self) -> Value {
        near_sdk::serde_json::from_str(&self.images).unwrap_or(Value::Null)
    }

    pub fn get_stats(&self) -> Value {
        json!({"requests": self.next, "resolved": self.resolved, "pending": self.pending.len(), "worker": self.worker,
               "estimate_execution_cost": U128(0)})
    }
}

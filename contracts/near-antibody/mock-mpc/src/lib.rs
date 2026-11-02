//! MOCK of the Chain Signatures MPC `sign` (domain 0), for the offline sandbox demo only.
//!
//! Same request and response shape as `v1.signer-prod.testnet`: the request yields, a signer resolves
//! it with `{big_r: {affine_point}, s: {scalar}, recovery_id}`, and a timeout fails the promise. The
//! mock signer (`antibody-services signer-mock`) signs with a single TEST secp256k1 key instead of a
//! threshold network, so there is no key derivation per path.

use near_sdk::{
    env, near, require,
    serde_json::{json, Value},
    store::IterableMap,
    AccountId, Gas, GasWeight, PanicOnDefault, PromiseResult,
};

#[near(serializers = [json])]
pub struct SignRequest {
    pub path: String,
    pub payload_v2: Value,
    pub domain_id: u32,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct PendingSign {
    pub request_id: u64,
    pub data_id: String,
    pub caller: AccountId,
    pub path: String,
    pub payload: String,
    pub height: u64,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockMpc {
    signer: AccountId,
    next: u64,
    pending: IterableMap<u64, PendingSign>,
}

#[near]
impl MockMpc {
    #[init]
    pub fn new(signer: AccountId) -> Self {
        Self { signer, next: 0, pending: IterableMap::new(b"p") }
    }

    #[payable]
    pub fn sign(&mut self, request: SignRequest) {
        require!(request.domain_id == 0, "mock supports domain 0 (secp256k1) only");
        let payload = request.payload_v2["Ecdsa"].as_str().unwrap_or_else(|| env::panic_str("payload_v2.Ecdsa required")).to_string();
        require!(payload.len() == 64, "payload must be a 32-byte hex hash");
        let id = self.next;
        self.next += 1;
        let idx = env::promise_yield_create("on_sign_response", json!({"request_id": id}).to_string().as_bytes(), Gas::from_tgas(10), GasWeight(0), 0);
        let data_id: [u8; 32] = env::read_register(0).expect("yield data id").try_into().expect("32-byte data id");
        self.pending.insert(
            id,
            PendingSign {
                request_id: id,
                data_id: data_id.iter().map(|b| format!("{b:02x}")).collect(),
                caller: env::predecessor_account_id(),
                path: request.path,
                payload,
                height: env::block_height(),
            },
        );
        env::promise_return(idx);
    }

    /// Signer-only: delivers `{big_r, s, recovery_id}` to the waiting caller.
    pub fn respond(&mut self, request_id: u64, response: Value) {
        require!(env::predecessor_account_id() == self.signer, "only the mock signer responds");
        let p = self.pending.remove(&request_id).unwrap_or_else(|| env::panic_str("unknown or already answered request"));
        let mut data_id = [0u8; 32];
        for (i, b) in data_id.iter_mut().enumerate() {
            *b = u8::from_str_radix(&p.data_id[2 * i..2 * i + 2], 16).unwrap();
        }
        require!(env::promise_yield_resume(&data_id, response.to_string().as_bytes()), "the request already timed out");
    }

    #[private]
    pub fn on_sign_response(&mut self, request_id: u64) {
        match env::promise_result(0) {
            PromiseResult::Successful(b) => env::value_return(&b),
            PromiseResult::Failed => {
                self.pending.remove(&request_id);
                env::panic_str("signature request timed out");
            }
        }
    }

    pub fn get_pending(&self, limit: Option<u32>) -> Vec<PendingSign> {
        self.pending.values().take(limit.unwrap_or(50) as usize).cloned().collect()
    }
}

//! What a NEAR run reads: the manifest's exact bytes, the wasm it commits to (inline, checked against
//! their sha256), and the hunter's bundle of top-level calls (docs/near-targets.md §2).
//!
//! - `antibody.near-base.v1`: the base witness a project pins at activation (manifest and code).
//! - `antibody.near-submission.v1`: what a sealed blob carries for a NEAR program (the bundle).
//! - `antibody.near-witness.v1`: both together, for a local run (the CLI and the tests).
//!
//! All three are JSON. The manifest is a JSON string holding the manifest's exact bytes; code is base64.

use serde::{Deserialize, Serialize};

use crate::util::{hex32, sha256};

pub const NEAR_BASE_SCHEMA: &str = "antibody.near-base.v1";
pub const NEAR_SUBMISSION_SCHEMA: &str = "antibody.near-submission.v1";
pub const NEAR_WITNESS_SCHEMA: &str = "antibody.near-witness.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NearBase {
    pub schema: String,
    pub manifest: String,
    pub codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NearSubmission {
    pub schema: String,
    pub bundle: Bundle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NearWitness {
    pub schema: String,
    pub manifest: String,
    pub codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<Bundle>,
}

/// The hunter's proof: top-level calls from fresh senders the run creates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    /// 32 bytes, hex: the senders' account ids derive from it.
    pub sender_salt: String,
    pub senders: u8,
    /// Each sender's starting balance, yoctoNEAR (at most `max_endowment_yocto`).
    pub endowment_yocto: String,
    pub calls: Vec<Call>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Call {
    /// The sender's index.
    pub signer: u8,
    pub receiver: String,
    pub method: String,
    /// Base64.
    #[serde(default)]
    pub args: String,
    #[serde(default = "zero")]
    pub deposit: String,
    pub gas: u64,
}

fn zero() -> String {
    "0".into()
}

/// Sender `i` of a bundle: the NEAR-implicit account `hex(sha256("antibody/near/sender/v1" ‖ salt ‖ i))`.
/// Nobody holds its key (that would take an ed25519 key whose public half is a hash output), and no
/// salt reaches an existing account except by a sha256 collision.
pub fn sender_account(salt: &[u8; 32], i: u8) -> String {
    let mut m = b"antibody/near/sender/v1".to_vec();
    m.extend_from_slice(salt);
    m.push(i);
    hex::encode(sha256(&m))
}

impl Bundle {
    pub fn salt(&self) -> Option<[u8; 32]> {
        hex32(&self.sender_salt)
    }

    /// `sha256("antibody/near/poc/v1" ‖ canonical JSON of the bundle)`: the bundle commitment the receipt
    /// carries.
    pub fn poc_hash(&self) -> [u8; 32] {
        let mut m = b"antibody/near/poc/v1".to_vec();
        m.extend(serde_json::to_vec(self).expect("bundle serializes"));
        sha256(&m)
    }
}

/// The borsh-encoded ed25519 key a named account signs with in the run. Keys are recorded state and v2
/// checks no signatures, so a signer named in the manifest gets this deterministic placeholder:
/// `0 ‖ sha256("antibody/near/signer-pk/v1" ‖ account)`. A NEAR-implicit account's key is its id.
pub fn signer_pk(account: &str) -> Vec<u8> {
    let mut pk = vec![0u8];
    match hex::decode(account) {
        Ok(b) if b.len() == 32 && crate::util::account_type(account) == crate::util::AccountType::NearImplicit => pk.extend_from_slice(&b),
        _ => {
            let mut m = b"antibody/near/signer-pk/v1".to_vec();
            m.extend_from_slice(account.as_bytes());
            pk.extend_from_slice(&sha256(&m));
        }
    }
    pk
}

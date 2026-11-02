//! Witness envelope: everything the verifier needs, with no network access.
//!
//! Trust model: the only value the verifier cannot check by itself is `claimed_block_hash`
//! (it comes from the host quorum, or in production from an on-chain light client). Everything
//! else (header fields, ancestors, transactions, state, storage, bytecode) is bound to that hash
//! by keccak / Merkle-Patricia proofs inside the verifier.

use alloy_consensus::Header;
use alloy_primitives::{Address, Bytes, B256};
use revm::state::Bytecode;
use rsp_client_executor::io::EthClientExecutorInput;
use rsp_mpt::EthereumState;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

pub const WITNESS_VERSION: u32 = 1;

/// A host-side observation of the block hash from one RPC source. Informational inside the
/// verifier (it cannot re-check it without network); copied into the receipt for auditors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumVote {
    pub source: String,
    pub block_hash: B256,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u32,
    pub chain_id: u64,
    /// Hash of the header that supplies the execution environment (replay: block N; block mode:
    /// the executed block). The verifier recomputes keccak(RLP(header)) and requires equality.
    pub claimed_block_hash: B256,
    pub quorum: Vec<QuorumVote>,
    pub payload: Payload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum Payload {
    /// Transaction-level replay: execute transactions `0..=target_index` of block N on the
    /// post-state of block N-1 (exactly what happened on chain), with pre-block system calls.
    Replay(ReplayWitness),
    /// Full-block stateless validation with rsp's client executor (state root must match).
    Block(EthClientExecutorInput),
    /// A researcher bundle executed with `eth_call`-at-M semantics on the anchored state of M.
    Bundle(BundleWitness),
    /// v0.4 program mode: a hunter's PoC against a program's committed manifest and invariants.
    Program(ProgramWitness),
    /// v2: the base witness a project pins at open (docs/spec.md §4.4): the round-0 manifest, the
    /// invariants, the anchor header and the targets' state.
    Base(BaseWitness),
    /// v2: what a sealed submission carries: the PoC and the extra state it touches, merged with the
    /// pinned base witness inside the verifier.
    Submission(SubmissionWitness),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseWitness {
    pub manifest_json: Bytes,
    pub invariants_code: Bytes,
    pub header_rlp: Bytes,
    pub ancestor_headers_rlp: Vec<Bytes>,
    pub state: EthereumState,
    pub bytecodes: Vec<Bytecode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionWitness {
    pub bundle: crate::bundle::Bundle,
    /// Proofs of every account and slot the PoC touches beyond the base witness (a partial trie of the
    /// same state root).
    pub state: EthereumState,
    pub bytecodes: Vec<Bytecode>,
}

impl ProgramWitness {
    /// A full program witness from the pinned base and a submission (docs/spec.md §4.4). `manifest` is
    /// the manifest the run is judged under: the base's in round 0, the round's manifest after.
    pub fn assemble(base: &BaseWitness, sub: &SubmissionWitness, manifest_json: Bytes) -> Result<ProgramWitness, String> {
        let state = crate::trie::merge(&base.state, &sub.state)?;
        let mut bytecodes = base.bytecodes.clone();
        for c in &sub.bytecodes {
            if !bytecodes.iter().any(|b| b.original_byte_slice() == c.original_byte_slice()) {
                bytecodes.push(c.clone());
            }
        }
        let manifest = crate::manifest::Manifest::parse(&manifest_json)?;
        Ok(ProgramWitness {
            manifest_json,
            invariants_code: manifest.invariants.runtime_code.clone(),
            header_rlp: base.header_rlp.clone(),
            ancestor_headers_rlp: base.ancestor_headers_rlp.clone(),
            state,
            bytecodes,
            bundle: sub.bundle.clone(),
        })
    }
}

/// Everything a program-mode run needs. `claimed_block_hash` must equal the manifest's anchor
/// hash; the manifest bytes must hash to the program's committed manifest hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramWitness {
    /// Canonical manifest JSON (sha256 = the manifest hash committed on NEAR).
    pub manifest_json: Bytes,
    /// Runtime bytecode of the invariants contract (keccak256 must match the manifest).
    pub invariants_code: Bytes,
    /// RLP of the anchor header M (state = post-state of M, environment = header M).
    pub header_rlp: Bytes,
    /// RLP of ancestors M-1, M-2, ... (newest first) for BLOCKHASH.
    pub ancestor_headers_rlp: Vec<Bytes>,
    pub state: EthereumState,
    pub bytecodes: Vec<Bytecode>,
    pub bundle: crate::bundle::Bundle,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleWitness {
    /// RLP of the anchor header M: its hash must equal `claimed_block_hash`; state = post-state of
    /// M (its own `state_root`); environment = header M.
    pub header_rlp: Bytes,
    /// Ancestors of M, newest first (M-1, ...), for BLOCKHASH.
    #[serde_as(as = "Vec<alloy_consensus::serde_bincode_compat::Header>")]
    pub ancestor_headers: Vec<Header>,
    pub state: EthereumState,
    pub bytecodes: Vec<Bytecode>,
    pub bundle: crate::bundle::Bundle,
    /// Manifest-protected contracts measured on the other side of the ledger.
    pub protected: Vec<Address>,
    pub price_feeds: Vec<PriceFeed>,
    pub aave_oracle: Option<Address>,
}

/// A USD price feed (Chainlink AggregatorV3 proxy) for a token. `token == Address::ZERO` means
/// native ETH.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PriceFeed {
    pub token: Address,
    pub feed: Address,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayWitness {
    /// RLP of the full block N (header + body). The header supplies the environment and its hash is
    /// bound to `claimed_block_hash`; the body is bound to the header's `transactions_root`.
    pub block_rlp: Bytes,
    /// Transactions `0..=target_index` are executed; the impact is measured on `target_index`.
    pub target_index: u32,
    /// Ancestor headers, newest first: N-1 (the state anchor), N-2, ... (for BLOCKHASH).
    #[serde_as(as = "Vec<alloy_consensus::serde_bincode_compat::Header>")]
    pub ancestor_headers: Vec<Header>,
    /// Partial state tries anchored at `ancestor_headers[0].state_root`.
    pub parent_state: EthereumState,
    /// Contract bytecodes (keyed by keccak inside the verifier).
    pub bytecodes: Vec<Bytecode>,
    /// Accounts whose token-unit balance deltas are measured.
    pub watch: Vec<Address>,
    /// Price manifest: token -> USD feed. Prices are read from the pre-target proven state.
    pub price_feeds: Vec<PriceFeed>,
    /// Optional fallback oracle (Aave v3 `AaveOracle.getAssetPrice(asset)`, USD with 8 decimals)
    /// for tokens without a direct feed. Also read from the pre-target proven state.
    pub aave_oracle: Option<Address>,
}

impl Envelope {
    pub fn encode(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialization of the envelope")
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        bincode::deserialize(bytes).map_err(|e| format!("witness decode: {e}"))
    }
}

/// Block hash claimed for the replay environment header, without decoding the whole witness.
pub fn header_hash(h: &Header) -> B256 {
    h.hash_slow()
}

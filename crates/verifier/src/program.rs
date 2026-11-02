//! Program mode (v0.4): run a hunter's PoC bundle on the proven anchor state of a program and
//! report which committed invariants broke, where (root-cause fingerprint), and the impact at the
//! committed reference prices. Validity is an invariant break, never profit.
//!
//! Invariant evaluation. The program's invariants runtime code is served at a reserved address by
//! an overlay database used only for evaluation calls, so the PoC never sees it and it is never
//! deployed. `pre = snapshot()` runs at the anchor and `check(pre)` must return 0 there. During the
//! PoC, `check(pre)` runs on the live journaled state at the end of every non-static
//! protocol-entry call (a call into a manifest target from a frame that is not itself executing in
//! a target, so calls made inside flash-loan callbacks count), and once after the PoC. For every
//! invariant broken at the end, the fingerprint is the entry call at whose end it last flipped from
//! holding to broken: temporary mid-transaction breaks that the PoC later restores are ignored.
//! Evaluating on the journal at `call_end` is exact: revm commits or reverts the frame's
//! checkpoint before `call_end` runs, and the nested evaluation EVM reads the in-flight journal
//! first and the committed state below it.

use std::collections::BTreeSet;

use alloy_consensus::Header;
use alloy_evm::{eth::EthEvmContext, Evm, EvmError};
use alloy_primitives::{keccak256, Address, Bytes, TxKind, I256, U256};
use reth_evm::ConfigureEvm;
use reth_evm_ethereum::EthEvmConfig;
use revm::{
    context::{result::EVMError, BlockEnv, CfgEnv, TxEnv},
    database::{State, WrapDatabaseRef},
    database_interface::DBErrorMarker,
    interpreter::{CallInputs, CallOutcome as FrameCallOutcome, CreateInputs, CreateOutcome},
    primitives::{hardfork::SpecId, KECCAK_EMPTY, B256},
    state::{AccountInfo, Bytecode, EvmState},
    Context, Database, DatabaseRef, Inspector, MainBuilder, MainContext, SystemCallEvm,
};
use rsp_client_executor::custom::{CustomCrypto, CustomEvmFactory};

use crate::{
    bundle::{sender_address, Bundle, CallOutcome, MAX_CALL_GAS},
    db::{MissingTracker, OverrideDb},
    facts::{AccountValue, Fingerprint, ImpactV2, Kind},
    impact::{self, PROBE_CALLER},
    manifest::{parse_u128, usd_to_micro, Manifest, Severity},
    run::VerifyError,
};

/// Why a program run produced no verdict on the PoC itself.
#[derive(Debug)]
pub enum ProgramError {
    /// Witness/verification errors (the witness builder's fixpoint loop resolves `Incomplete`).
    Verify(VerifyError),
    /// A typed verdict other than Violation/NoViolation.
    Verdict { kind: Kind, reason: String },
}

impl From<VerifyError> for ProgramError {
    fn from(e: VerifyError) -> Self {
        Self::Verify(e)
    }
}

fn verdict(kind: Kind, reason: impl Into<String>) -> ProgramError {
    ProgramError::Verdict { kind, reason: reason.into() }
}

#[derive(Debug, Clone)]
pub struct ProgramRun {
    pub senders: Vec<Address>,
    pub calls: Vec<CallOutcome>,
    pub created: Vec<Address>,
    pub total_gas: u64,
    pub evaluations: u32,
    pub broken_mask: U256,
    pub broken: Vec<Fingerprint>,
    pub impact: ImpactV2,
    pub target_code_hashes_match: bool,
    pub reserved_address_empty: bool,
}

pub fn spec_id(name: &str) -> Option<SpecId> {
    match name.to_ascii_lowercase().as_str() {
        "cancun" => Some(SpecId::CANCUN),
        "prague" => Some(SpecId::PRAGUE),
        "osaka" => Some(SpecId::OSAKA),
        _ => None,
    }
}

pub fn selector_of(sig: &str) -> [u8; 4] {
    let h = keccak256(sig.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

/// The program's invariants contract as the evaluation overlay serves it.
#[derive(Debug, Clone)]
pub struct InvariantCode {
    pub address: Address,
    pub code: Bytecode,
    pub code_hash: B256,
    pub gas_limit: u64,
}

/// Read view used only by evaluation calls: in-flight journal first (if any), then the committed
/// state below it, plus the injected invariants code at its reserved address.
struct Overlay<'a, D> {
    journal: Option<&'a EvmState>,
    inner: D,
    inv: &'a InvariantCode,
}

impl<D: Database> Database for Overlay<'_, D> {
    type Error = D::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        if address == self.inv.address {
            return Ok(Some(AccountInfo {
                balance: U256::ZERO,
                nonce: 1,
                code_hash: self.inv.code_hash,
                code: Some(self.inv.code.clone()),
                account_id: None,
            }));
        }
        if let Some(acc) = self.journal.and_then(|s| s.get(&address)) {
            if acc.is_selfdestructed() || acc.is_loaded_as_not_existing_not_touched() {
                return Ok(None);
            }
            return Ok(Some(acc.info.clone()));
        }
        self.inner.basic(address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if code_hash == self.inv.code_hash {
            return Ok(self.inv.code.clone());
        }
        if let Some(st) = self.journal {
            if let Some(code) =
                st.values().find_map(|a| if a.info.code_hash == code_hash { a.info.code.clone() } else { None })
            {
                return Ok(code);
            }
        }
        self.inner.code_by_hash(code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        if address == self.inv.address {
            return Ok(U256::ZERO);
        }
        if let Some(acc) = self.journal.and_then(|s| s.get(&address)) {
            if let Some(slot) = acc.storage.get(&index) {
                return Ok(slot.present_value);
            }
            if acc.is_created() || acc.is_selfdestructed() {
                return Ok(U256::ZERO);
            }
        }
        self.inner.storage(address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.inner.block_hash(number)
    }
}

struct EvalOut {
    success: bool,
    output: Bytes,
    gas_used: u64,
}

/// A never-committed call from `PROBE_CALLER` (system-call semantics: no fee, no nonce).
fn eval_call<DB: Database>(db: DB, block: &BlockEnv, cfg: &CfgEnv, to: Address, data: Bytes) -> Result<EvalOut, DB::Error> {
    let mut evm = Context::mainnet().with_db(db).with_block(block.clone()).with_cfg(cfg.clone()).build_mainnet();
    match evm.system_call_one_with_caller(PROBE_CALLER, to, data) {
        Ok(r) => Ok(EvalOut {
            success: r.is_success(),
            output: r.output().cloned().unwrap_or_default(),
            gas_used: r.tx_gas_used(),
        }),
        Err(EVMError::Database(e)) => Err(e),
        Err(_) => Ok(EvalOut { success: false, output: Bytes::new(), gas_used: 0 }),
    }
}

fn word0(out: &[u8]) -> Option<U256> {
    (out.len() >= 32).then(|| U256::from_be_slice(&out[..32]))
}

fn mask_of(o: &EvalOut, gas_limit: u64) -> Option<U256> {
    if o.success && o.gas_used <= gas_limit {
        word0(&o.output)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
struct Frame {
    in_target: bool,
    entry: bool,
    is_static: bool,
    target: Address,
    selector: [u8; 4],
    /// `points.len()` when the frame began. A frame that fails is truncated back to it, so the
    /// timeline holds only evaluations of state that persisted.
    mark: usize,
}

/// One invariant evaluation at the end of a protocol-entry call.
#[derive(Debug, Clone)]
pub struct EvalPoint {
    pub tx: u32,
    pub target: Address,
    pub selector: [u8; 4],
    /// `None` when the evaluation reverted or exceeded the gas cap (skipped).
    pub mask: Option<U256>,
}

/// Records protocol-entry calls and evaluates the invariants at the end of each one.
struct FingerprintInspector {
    targets: BTreeSet<Address>,
    inv: InvariantCode,
    check_data: Bytes,
    declared: U256,
    max_points: u32,
    /// Evaluations performed (the `max_entry_calls` cap), including ones later truncated.
    evals: u32,
    stack: Vec<Frame>,
    tx: u32,
    points: Vec<EvalPoint>,
    db_error: Option<String>,
    too_many: bool,
}

impl FingerprintInspector {
    fn begin_tx(&mut self, tx: u32) {
        self.tx = tx;
        self.stack.clear();
    }
}

impl<DB: Database> Inspector<EthEvmContext<DB>> for FingerprintInspector {
    fn call(&mut self, context: &mut EthEvmContext<DB>, inputs: &mut CallInputs) -> Option<FrameCallOutcome> {
        let parent_in_target = self.stack.last().map(|f| f.in_target).unwrap_or(false);
        let in_target = self.targets.contains(&inputs.target_address);
        let entry = in_target && !parent_in_target;
        let mut selector = [0u8; 4];
        if entry {
            let input = inputs.input.bytes(context);
            if input.len() >= 4 {
                selector.copy_from_slice(&input[..4]);
            }
        }
        let mark = self.points.len();
        self.stack.push(Frame { in_target, entry, is_static: inputs.is_static, target: inputs.target_address, selector, mark });
        None
    }

    fn call_end(&mut self, context: &mut EthEvmContext<DB>, _inputs: &CallInputs, outcome: &mut FrameCallOutcome) {
        let Some(frame) = self.stack.pop() else { return };
        if !outcome.result.is_ok() {
            // Everything the frame did was reverted, and so are the evaluations recorded inside it.
            self.points.truncate(frame.mark);
            return;
        }
        if !frame.entry || frame.is_static || self.db_error.is_some() {
            return;
        }
        if self.evals >= self.max_points {
            self.too_many = true;
            return;
        }
        self.evals += 1;
        let block = context.block.clone();
        let cfg = context.cfg.clone();
        let journal = &mut context.journaled_state;
        let overlay = Overlay { journal: Some(&journal.inner.state), inner: &mut journal.database, inv: &self.inv };
        match eval_call(overlay, &block, &cfg, self.inv.address, self.check_data.clone()) {
            Ok(o) => {
                let mask = mask_of(&o, self.inv.gas_limit).map(|m| m & self.declared);
                self.points.push(EvalPoint { tx: self.tx, target: frame.target, selector: frame.selector, mask });
            }
            Err(e) => self.db_error = Some(e.to_string()),
        }
    }

    fn create(&mut self, _context: &mut EthEvmContext<DB>, _inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        let mark = self.points.len();
        self.stack.push(Frame { in_target: false, entry: false, is_static: false, target: Address::ZERO, selector: [0; 4], mark });
        None
    }

    fn create_end(&mut self, _context: &mut EthEvmContext<DB>, _inputs: &CreateInputs, outcome: &mut CreateOutcome) {
        if let Some(frame) = self.stack.pop() {
            if !outcome.result.is_ok() {
                self.points.truncate(frame.mark);
            }
        }
    }
}

fn missing_or<T: MissingTracker + ?Sized>(db: &T, e: impl std::fmt::Display) -> ProgramError {
    match db.first_missing() {
        Some(m) => ProgramError::Verify(VerifyError::Incomplete(m)),
        None => verdict(Kind::ExecutionError, e.to_string()),
    }
}

/// Runs the PoC of `bundle` against the program `manifest` on the anchored state `db` (the
/// post-state of `header`, already proven by the caller). In a round > 0 the manifest's code
/// overrides (the project's fix) replace the anchor's code at their target addresses.
pub fn execute_program<D>(
    header: &Header,
    manifest: &Manifest,
    inv_code: &Bytecode,
    bundle: &Bundle,
    chain_spec: std::sync::Arc<reth_chainspec::ChainSpec>,
    db: &D,
) -> Result<ProgramRun, ProgramError>
where
    D: DatabaseRef + MissingTracker + std::fmt::Debug,
    D::Error: DBErrorMarker + std::error::Error + Send + Sync + 'static,
{
    manifest.check_overrides().map_err(|e| verdict(Kind::ProgramMisconfigured, e))?;
    if manifest.overrides.iter().any(|o| o.address == manifest.invariants.address) {
        return Err(verdict(Kind::ProgramMisconfigured, "an override targets the reserved invariants address"));
    }
    let odb = OverrideDb::new(db, manifest.overrides.iter().map(|o| (o.address, Bytecode::new_raw(o.runtime_code.clone()))));
    execute_on(header, manifest, inv_code, bundle, chain_spec, &odb)
}

fn execute_on<D>(
    header: &Header,
    manifest: &Manifest,
    inv_code: &Bytecode,
    bundle: &Bundle,
    chain_spec: std::sync::Arc<reth_chainspec::ChainSpec>,
    db: &D,
) -> Result<ProgramRun, ProgramError>
where
    D: DatabaseRef + MissingTracker + std::fmt::Debug,
    D::Error: DBErrorMarker + std::error::Error + Send + Sync + 'static,
{
    // ---- committed attacker capabilities (InvalidPoC) ----
    let caps = &manifest.capabilities;
    let max_endowment =
        parse_u128(&caps.max_endowment_wei, "max_endowment_wei").map_err(|e| verdict(Kind::ProgramMisconfigured, e))?;
    if bundle.senders == 0 || bundle.calls.is_empty() {
        return Err(verdict(Kind::InvalidPoC, "empty bundle"));
    }
    if bundle.senders > caps.max_senders {
        return Err(verdict(Kind::InvalidPoC, format!("{} senders > cap {}", bundle.senders, caps.max_senders)));
    }
    if bundle.endowment_wei > max_endowment {
        return Err(verdict(Kind::InvalidPoC, "endowment above the committed cap"));
    }
    if bundle.calls.len() as u64 > caps.max_txs as u64 {
        return Err(verdict(Kind::InvalidPoC, format!("{} txs > cap {}", bundle.calls.len(), caps.max_txs)));
    }
    let per_tx_cap = caps.max_gas_per_tx.min(MAX_CALL_GAS);
    let mut gas_sum = 0u64;
    for (i, c) in bundle.calls.iter().enumerate() {
        if c.gas_limit > per_tx_cap {
            return Err(verdict(Kind::InvalidPoC, format!("tx {i}: gas limit {} > cap {per_tx_cap}", c.gas_limit)));
        }
        if c.sender >= bundle.senders {
            return Err(verdict(Kind::InvalidPoC, format!("tx {i}: bad sender index")));
        }
        gas_sum = gas_sum.saturating_add(c.gas_limit);
    }
    if gas_sum > caps.max_total_gas {
        return Err(verdict(Kind::InvalidPoC, format!("total gas {gas_sum} > cap {}", caps.max_total_gas)));
    }

    // ---- environment: header M, committed EVM rules ----
    let spec = spec_id(&manifest.anchor.evm_spec)
        .ok_or_else(|| verdict(Kind::ProgramMisconfigured, format!("unsupported evm_spec {}", manifest.anchor.evm_spec)))?;
    revm::install_crypto(CustomCrypto::default());
    let evm_config = EthEvmConfig::new_with_evm_factory(chain_spec, CustomEvmFactory::new(None));
    let mut evm_env = evm_config.evm_env(header).map_err(|e| verdict(Kind::ExecutionError, e.to_string()))?;
    evm_env.cfg_env.spec = spec;
    evm_env.cfg_env.chain_id = manifest.anchor.chain_id;
    let block = evm_env.block_env.clone();
    let cfg = evm_env.cfg_env.clone();
    let inv = InvariantCode {
        address: manifest.invariants.address,
        code_hash: inv_code.hash_slow(),
        code: inv_code.clone(),
        gas_limit: manifest.invariants.gas_limit,
    };
    let declared = manifest
        .invariants
        .slots
        .iter()
        .filter(|s| s.bit < 255)
        .fold(U256::ZERO, |m, s| m | (U256::from(1u8) << s.bit as usize));

    // ---- anchor checks (ProgramMisconfigured) ----
    let mut pre_state = State::builder().with_database(WrapDatabaseRef(db)).build();
    match pre_state.basic(inv.address).map_err(|e| missing_or(db, e))? {
        None => {}
        Some(a) if a.nonce == 0 && a.balance.is_zero() && a.code_hash == KECCAK_EMPTY => {}
        Some(_) => return Err(verdict(Kind::ProgramMisconfigured, "reserved invariants address is in use at the anchor")),
    }
    for t in &manifest.targets {
        let a = pre_state.basic(t.address).map_err(|e| missing_or(db, e))?;
        let h = a.map(|a| a.code_hash).unwrap_or(KECCAK_EMPTY);
        if h != t.code_hash {
            return Err(verdict(Kind::ProgramMisconfigured, format!("target {} code hash {h} != manifest", t.name)));
        }
    }
    let snap = eval_call(
        Overlay { journal: None, inner: &mut pre_state, inv: &inv },
        &block,
        &cfg,
        inv.address,
        selector_of("snapshot()").to_vec().into(),
    )
    .map_err(|e| missing_or(db, e))?;
    if !snap.success || snap.gas_used > inv.gas_limit {
        return Err(verdict(Kind::ProgramMisconfigured, "snapshot() failed at the anchor"));
    }
    // `bytes` return data is exactly the ABI encoding of a single `bytes` argument.
    let check_data: Bytes = [selector_of("check(bytes)").as_slice(), snap.output.as_ref()].concat().into();
    let m0 = eval_call(Overlay { journal: None, inner: &mut pre_state, inv: &inv }, &block, &cfg, inv.address, check_data.clone())
        .map_err(|e| missing_or(db, e))?;
    match mask_of(&m0, inv.gas_limit) {
        Some(m) if (m & declared).is_zero() => {}
        Some(m) => return Err(verdict(Kind::ProgramMisconfigured, format!("check(snapshot()) = {m:#x} at the anchor"))),
        None => return Err(verdict(Kind::ProgramMisconfigured, "check() failed at the anchor")),
    }
    drop(pre_state);

    // ---- the PoC: fresh salted senders, bounded endowment, anchor environment ----
    let senders: Vec<Address> = (0..bundle.senders).map(|i| sender_address(&bundle.sender_salt, i)).collect();
    let mut state = State::builder().with_database(WrapDatabaseRef(db)).with_bundle_update().build();
    for s in &senders {
        if let Some(a) = state.basic(*s).map_err(|e| missing_or(db, e))? {
            if a.nonce != 0 || !a.balance.is_zero() || a.code_hash != KECCAK_EMPTY {
                return Err(verdict(Kind::InvalidPoC, format!("sender {s} is not a fresh empty account at the anchor")));
            }
        }
    }
    for s in &senders {
        state.insert_account(
            *s,
            AccountInfo {
                balance: U256::from(bundle.endowment_wei),
                nonce: 0,
                code_hash: KECCAK_EMPTY,
                code: None,
                account_id: None,
            },
        );
    }
    let base_fee = header.base_fee_per_gas.unwrap_or_default() as u128;
    let inspector = FingerprintInspector {
        targets: manifest.targets.iter().map(|t| t.address).collect(),
        inv: inv.clone(),
        check_data: check_data.clone(),
        declared,
        max_points: caps.max_entry_calls,
        evals: 0,
        stack: Vec::new(),
        tx: 0,
        points: Vec::new(),
        db_error: None,
        too_many: false,
    };
    let mut evm = evm_config.evm_with_env_and_inspector(&mut state, evm_env, inspector);
    let mut calls = Vec::new();
    let mut created = Vec::new();
    let mut nonces = vec![0u64; senders.len()];
    let mut fees = vec![U256::ZERO; senders.len()];
    let mut total_gas = 0u64;
    for (i, c) in bundle.calls.iter().enumerate() {
        let si = c.sender as usize;
        let tx = TxEnv {
            caller: senders[si],
            gas_limit: c.gas_limit,
            gas_price: base_fee,
            kind: match c.to {
                Some(a) => TxKind::Call(a),
                None => TxKind::Create,
            },
            value: c.value,
            data: c.data.clone(),
            nonce: nonces[si],
            chain_id: Some(manifest.anchor.chain_id),
            ..Default::default()
        };
        evm.inspector_mut().begin_tx(i as u32);
        let res = evm.transact_commit(tx);
        if let Some(e) = evm.inspector_mut().db_error.take() {
            return Err(missing_or(db, e));
        }
        if evm.inspector().too_many {
            return Err(verdict(Kind::InvalidPoC, format!("more than {} protocol-entry calls", caps.max_entry_calls)));
        }
        let res = match res {
            Ok(r) => r,
            Err(e) if e.is_invalid_tx_err() => return Err(verdict(Kind::InvalidPoC, format!("tx {i}: {e}"))),
            Err(e) => return Err(missing_or(db, e)),
        };
        nonces[si] += 1;
        let gas = res.tx_gas_used();
        total_gas += gas;
        fees[si] += U256::from(gas) * U256::from(base_fee);
        let new_addr = if c.to.is_none() { res.created_address() } else { None };
        if let Some(a) = new_addr {
            created.push(a);
        }
        calls.push(CallOutcome {
            index: i,
            sender: senders[si],
            to: c.to,
            success: res.is_success(),
            gas_used: gas,
            logs: res.logs().len(),
            created: new_addr,
        });
    }
    let points = evm.inspector().points.clone();
    drop(evm);

    // ---- final evaluation ----
    let fin = eval_call(Overlay { journal: None, inner: &mut state, inv: &inv }, &block, &cfg, inv.address, check_data)
        .map_err(|e| missing_or(db, e))?;
    let final_mask = match mask_of(&fin, inv.gas_limit) {
        Some(m) => m & declared,
        None => return Err(verdict(Kind::ExecutionError, "invariant check reverted or ran out of gas after the PoC")),
    };

    // ---- root-cause fingerprints: last 0 -> 1 flip per broken invariant ----
    let mut seq: Vec<(U256, Option<usize>)> = vec![(U256::ZERO, None)];
    for (k, p) in points.iter().enumerate() {
        if let Some(m) = p.mask {
            seq.push((m, Some(k)));
        }
    }
    seq.push((final_mask, None));
    let mut broken = Vec::new();
    for bit in 0..256usize {
        if !final_mask.bit(bit) {
            continue;
        }
        let flip = (1..seq.len()).rev().find(|&j| seq[j].0.bit(bit) && !seq[j - 1].0.bit(bit));
        let point = flip.and_then(|j| seq[j].1);
        let slot = manifest.slot(bit as u8);
        let (target, selector, tx) = match point {
            Some(k) => (points[k].target, points[k].selector, Some(points[k].tx)),
            None => (Address::ZERO, [0u8; 4], None),
        };
        broken.push(Fingerprint {
            slot: bit as u8,
            name: slot.map(|s| s.name.clone()).unwrap_or_else(|| format!("bit{bit}")),
            severity: slot.map(|s| s.severity).unwrap_or(Severity::Low),
            target,
            target_name: manifest.target_name(target).map(str::to_owned),
            selector: format!("0x{}", hex::encode(selector)),
            flip_point: point.map(|k| k as u32),
            tx_index: tx,
        });
    }

    // ---- impact at the committed reference prices ----
    let closure: Vec<Address> = senders.iter().chain(created.iter()).copied().collect();
    let impact = measure_impact(manifest, db, &mut state, &inv, &block, &cfg, &senders, &closure, bundle.endowment_wei, &fees)?;

    Ok(ProgramRun {
        senders,
        calls,
        created,
        total_gas,
        evaluations: points.len() as u32,
        broken_mask: final_mask,
        broken,
        impact,
        target_code_hashes_match: true,
        reserved_address_empty: true,
    })
}

#[allow(clippy::too_many_arguments)]
fn measure_impact<D>(
    manifest: &Manifest,
    db: &D,
    post: &mut State<WrapDatabaseRef<&D>>,
    inv: &InvariantCode,
    block: &BlockEnv,
    cfg: &CfgEnv,
    senders: &[Address],
    closure: &[Address],
    endowment: u128,
    fees: &[U256],
) -> Result<ImpactV2, ProgramError>
where
    D: DatabaseRef + MissingTracker + std::fmt::Debug,
    D::Error: DBErrorMarker + std::error::Error + Send + Sync + 'static,
{
    let bad = |e: String| verdict(Kind::ProgramMisconfigured, e);
    let native = usd_to_micro(&manifest.impact.native_usd).map_err(bad)?;
    let mut tokens = Vec::new();
    for t in &manifest.impact.tokens {
        tokens.push((t.address, t.symbol.clone(), t.decimals, usd_to_micro(&t.usd).map_err(bad)?));
    }
    let mut accounts: Vec<(Address, &'static str)> =
        manifest.impact.protocol_accounts.iter().map(|a| (*a, "protocol")).collect();
    accounts.extend(closure.iter().map(|a| (*a, "attacker")));

    let mut pre = State::builder().with_database(WrapDatabaseRef(db)).build();
    let balances = |st: &mut State<WrapDatabaseRef<&D>>| -> Result<Vec<(U256, Vec<U256>)>, ProgramError> {
        let mut out = Vec::new();
        for (a, _) in &accounts {
            let eth = st.basic(*a).map_err(|e| missing_or(db, e))?.map(|i| i.balance).unwrap_or_default();
            let mut tb = Vec::new();
            for (t, _, _, _) in &tokens {
                let o = eval_call(
                    Overlay { journal: None, inner: &mut *st, inv },
                    block,
                    cfg,
                    *t,
                    impact::calldata_balance_of(*a),
                )
                .map_err(|e| missing_or(db, e))?;
                tb.push(if o.success { word0(&o.output).unwrap_or_default() } else { U256::ZERO });
            }
            out.push((eth, tb));
        }
        Ok(out)
    };
    let before = balances(&mut pre)?;
    let after = balances(post)?;

    let to_i = |u: U256| I256::from_raw(u);
    let (mut protocol, mut attacker) = (I256::ZERO, I256::ZERO);
    let mut rows = Vec::new();
    for (k, (a, role)) in accounts.iter().enumerate() {
        let sender_idx = senders.iter().position(|s| s == a);
        let eth_pre = match sender_idx {
            Some(_) => U256::from(endowment),
            None => before[k].0,
        };
        let mut eth_delta = to_i(after[k].0) - to_i(eth_pre);
        if let Some(si) = sender_idx {
            eth_delta += to_i(fees[si]); // gas is paid from the endowment, not a loss or a gain
        }
        let mut usd = eth_delta * to_i(native) / to_i(U256::from(10u64).pow(U256::from(18u64)));
        let mut token_rows = Vec::new();
        for (j, (_, sym, dec, price)) in tokens.iter().enumerate() {
            let d = to_i(after[k].1[j]) - to_i(before[k].1[j]);
            if d != I256::ZERO {
                usd += d * to_i(*price) / to_i(U256::from(10u64).pow(U256::from(*dec as u64)));
                token_rows.push((sym.clone(), d.to_string()));
            }
        }
        if *role == "protocol" {
            protocol += usd;
        } else {
            attacker += usd;
        }
        if eth_delta != I256::ZERO || !token_rows.is_empty() {
            rows.push(AccountValue {
                account: *a,
                role: role.to_string(),
                eth_delta_wei: eth_delta.to_string(),
                tokens: token_rows,
                usd_micro: usd.to_string(),
            });
        }
    }
    let l_p = if protocol < I256::ZERO { -protocol } else { I256::ZERO };
    let g_a = if attacker > I256::ZERO { attacker } else { I256::ZERO };
    let i = if l_p < g_a { l_p } else { g_a };
    Ok(ImpactV2 {
        protocol_loss_usd_micro: l_p.to_string(),
        attacker_gain_usd_micro: g_a.to_string(),
        impact_usd_micro: i.to_string(),
        impact_usd: impact::fmt_usd(i),
        accounts: rows,
    })
}

/// Maps a program-run error to a typed verdict for the receipt (the TEE never fails silently).
pub fn classify(e: &ProgramError) -> (Kind, String) {
    match e {
        ProgramError::Verdict { kind, reason } => (*kind, reason.clone()),
        ProgramError::Verify(v) => match v {
            VerifyError::Incomplete(m) => (Kind::InvalidPoC, format!("witness incomplete: {m}")),
            VerifyError::HeaderHashMismatch { .. }
            | VerifyError::TransactionsRootMismatch { .. }
            | VerifyError::Witness(_)
            | VerifyError::Decode(_)
            | VerifyError::TargetOutOfRange(..)
            | VerifyError::SignatureRecovery(_) => (Kind::InvalidPoC, v.to_string()),
            VerifyError::UnsupportedChain(_) | VerifyError::UnsupportedFork(_) | VerifyError::Execution(_) => {
                (Kind::ExecutionError, v.to_string())
            }
        },
    }
}

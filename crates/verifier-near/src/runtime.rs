//! Receipts, promises and accounts (docs/near-targets.md §5), with nearcore's semantics:
//!
//! - **Order.** Receipts run in one FIFO queue, as one shard would. Each pass over the queue is one block:
//!   the height goes up by one and the timestamp by one second per pass. Receipts created during a pass
//!   join the queue for the next one, in creation order. A callback joins the queue when its last input
//!   arrives. A top-level call's whole receipt tree finishes before the next call starts.
//! - **Actions** run in order within a receipt; a failed action fails the receipt, which rolls back its
//!   own changes only (storage, balance, keys, code, the account it created) and creates no receipts.
//! - **Callbacks** get their promises' results: the returned value, or failure.
//! - **Refunds:** a failed receipt's attached deposits return to its predecessor in a refund receipt.
//! - **Storage staking** is checked on the receiver after each receipt (10^19 yoctoNEAR per byte, and
//!   nearcore's zero-balance allowance of 770 bytes); a receipt that leaves it short fails.
//! - **Bounds:** receipts per run, promise depth, instructions per call and per run.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use serde::Serialize;

use crate::{
    config::{
        Config, MAX_ACCOUNT_DELETION_STORAGE_USAGE, MIN_ALLOWED_TOP_LEVEL_ACCOUNT_LENGTH, REGISTRAR_ACCOUNT_ID, STORAGE_AMOUNT_PER_BYTE,
        SYSTEM_ACCOUNT_ID, ZERO_BALANCE_ACCOUNT_STORAGE_LIMIT,
    },
    exec::{self, Outcome, Vm},
    logic::{Action, Ctx, NewReceipt, PromiseResult, ReturnData},
    state::{implicit_account, key_storage, Account, World},
    util::{account_type, is_sub_account_of, is_top_level, is_valid_account_id, sha256, AccountType},
};

/// The fixed block context (`near.block`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub height: u64,
    pub timestamp_ns: u64,
    pub epoch_height: u64,
    pub random_seed: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct Receipt {
    pub id: u64,
    pub predecessor: String,
    pub receiver: String,
    pub signer: String,
    pub signer_pk: Vec<u8>,
    pub depth: u32,
    pub input_data_ids: Vec<u64>,
    pub output_data_receivers: Vec<(u64, String)>,
    pub actions: Vec<Action>,
}

/// One executed receipt, as the NEAR receipt section records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceiptRecord {
    pub id: u64,
    /// `init`, `setup` or `call`, and the index of the top-level call in that phase.
    pub phase: String,
    pub call: u32,
    pub block_height: u64,
    pub predecessor: String,
    pub receiver: String,
    pub actions: Vec<String>,
    pub success: bool,
    /// Base64 of the returned bytes, or `receipt:<id>` when it returned a promise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub logs: Vec<String>,
    /// Gas burnt by the receipt's function calls (never judged; excluded from the doubled-run comparison).
    pub gas_burnt: u64,
}

/// What stops a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    /// v2 cannot judge the run (gas, floats, budgets, refused modules, what it does not model).
    Unevaluable(String),
    /// A committed bound was exceeded (receipts, depth, total instructions): the hunter's fault for a
    /// bundle, the project's for activation.
    Bound(String),
}

enum Value {
    None,
    Bytes(Vec<u8>),
    Receipt(u64),
}

pub struct Runtime {
    pub vm: Vm,
    pub cfg: Arc<Config>,
    pub world: World,
    pub block: Block,
    /// Passes (blocks) run so far.
    pub pass: u64,
    next_receipt: u64,
    next_data: u64,
    pub max_receipts: u64,
    pub max_depth: u32,
    pub fuel_per_call: u64,
    /// The bundle's total instruction budget (`max_total_instructions`), when set.
    pub fuel_total_limit: Option<u64>,
    pub fuel_total: u64,
    pub receipts_executed: u64,
    pub records: Vec<ReceiptRecord>,
    /// Accounts created by `CreateAccount` or implicit account creation, in order.
    pub created_accounts: Vec<String>,
    /// Whether the 10% gas margin is checked (the base run; the doubled run compares instead).
    pub margin_check: bool,
    queue: VecDeque<Receipt>,
    postponed: BTreeMap<u64, (Receipt, usize)>,
    waiting: BTreeMap<u64, u64>,
    data: BTreeMap<u64, Option<Vec<u8>>>,
    phase: String,
    call: u32,
}

impl Runtime {
    pub fn new(vm: Vm, cfg: Arc<Config>, world: World, block: Block) -> Self {
        Runtime {
            vm,
            cfg,
            world,
            block,
            pass: 0,
            next_receipt: 0,
            next_data: 0,
            max_receipts: 256,
            max_depth: 16,
            fuel_per_call: 1_000_000_000,
            fuel_total_limit: None,
            fuel_total: 0,
            receipts_executed: 0,
            records: Vec::new(),
            created_accounts: Vec::new(),
            margin_check: true,
            queue: VecDeque::new(),
            postponed: BTreeMap::new(),
            waiting: BTreeMap::new(),
            data: BTreeMap::new(),
            phase: String::new(),
            call: 0,
        }
    }

    /// A copy that continues from the same point (the doubled run starts from the anchor this way).
    pub fn fork(&self, cfg: Arc<Config>) -> Runtime {
        Runtime {
            vm: self.vm.clone(),
            cfg,
            world: self.world.clone(),
            block: self.block.clone(),
            pass: self.pass,
            next_receipt: self.next_receipt,
            next_data: self.next_data,
            max_receipts: self.max_receipts,
            max_depth: self.max_depth,
            fuel_per_call: self.fuel_per_call,
            fuel_total_limit: self.fuel_total_limit,
            fuel_total: self.fuel_total,
            receipts_executed: self.receipts_executed,
            records: self.records.clone(),
            created_accounts: self.created_accounts.clone(),
            margin_check: self.margin_check,
            queue: VecDeque::new(),
            postponed: BTreeMap::new(),
            waiting: BTreeMap::new(),
            data: BTreeMap::new(),
            phase: self.phase.clone(),
            call: self.call,
        }
    }

    /// The current block's height and timestamp.
    pub fn now(&self) -> (u64, u64) {
        (self.block.height + self.pass, self.block.timestamp_ns + self.pass * 1_000_000_000)
    }

    /// A transaction from `signer`: its receipt, then everything it causes. Returns the first receipt's
    /// record index.
    pub fn transaction(&mut self, signer: &str, signer_pk: Vec<u8>, receiver: &str, actions: Vec<Action>, phase: &str, call: u32) -> Result<usize, Halt> {
        self.phase = phase.to_string();
        self.call = call;
        let deposit: u128 = actions.iter().map(Action::deposit).sum();
        let Some(acc) = self.world.accounts.get_mut(signer) else {
            return Err(Halt::Bound(format!("the signer {signer} does not exist")));
        };
        acc.amount = acc.amount.checked_sub(deposit).ok_or_else(|| Halt::Bound(format!("the signer {signer} cannot pay the deposit")))?;
        let id = self.receipt_id();
        self.queue.push_back(Receipt {
            id,
            predecessor: signer.to_string(),
            receiver: receiver.to_string(),
            signer: signer.to_string(),
            signer_pk,
            depth: 0,
            input_data_ids: vec![],
            output_data_receivers: vec![],
            actions,
        });
        let first = self.records.len();
        while !self.queue.is_empty() {
            for _ in 0..self.queue.len() {
                let r = self.queue.pop_front().expect("queue");
                self.apply(r)?;
            }
            self.pass += 1;
        }
        if !self.postponed.is_empty() {
            return Err(Halt::Unevaluable("a callback never received its data".into()));
        }
        Ok(first)
    }

    fn receipt_id(&mut self) -> u64 {
        self.next_receipt += 1;
        self.next_receipt - 1
    }

    fn deliver(&mut self, data_id: u64, value: Option<Vec<u8>>) {
        self.data.insert(data_id, value);
        if let Some(rid) = self.waiting.remove(&data_id) {
            if let Some((_, left)) = self.postponed.get_mut(&rid) {
                *left -= 1;
                if *left == 0 {
                    let (r, _) = self.postponed.remove(&rid).expect("postponed");
                    self.queue.push_back(r);
                }
            }
        }
    }

    fn enqueue(&mut self, r: Receipt) {
        let missing: Vec<u64> = r.input_data_ids.iter().copied().filter(|d| !self.data.contains_key(d)).collect();
        if missing.is_empty() {
            self.queue.push_back(r);
        } else {
            for d in &missing {
                self.waiting.insert(*d, r.id);
            }
            self.postponed.insert(r.id, (r, missing.len()));
        }
    }

    fn system_transfer(&mut self, receiver: &str, amount: u128, depth: u32) -> Receipt {
        Receipt {
            id: self.receipt_id(),
            predecessor: SYSTEM_ACCOUNT_ID.into(),
            receiver: receiver.to_string(),
            signer: SYSTEM_ACCOUNT_ID.into(),
            signer_pk: vec![0; 33],
            depth,
            input_data_ids: vec![],
            output_data_receivers: vec![],
            actions: vec![Action::Transfer { deposit: amount }],
        }
    }

    fn apply(&mut self, r: Receipt) -> Result<(), Halt> {
        self.receipts_executed += 1;
        if self.receipts_executed > self.max_receipts {
            return Err(Halt::Bound(format!("more than {} receipts", self.max_receipts)));
        }
        if r.depth > self.max_depth {
            return Err(Halt::Bound(format!("a promise chain deeper than {}", self.max_depth)));
        }
        let (height, timestamp) = self.now();
        let results: Vec<PromiseResult> = r
            .input_data_ids
            .iter()
            .map(|d| match self.data.remove(d).flatten() {
                Some(v) => PromiseResult::Successful(v),
                None => PromiseResult::Failed,
            })
            .collect();
        let is_refund = r.predecessor == SYSTEM_ACCOUNT_ID;
        let mut account = self.world.accounts.get(&r.receiver).cloned();
        let mut actor = r.predecessor.clone();
        let mut pending: Vec<Receipt> = Vec::new();
        let mut value = Value::None;
        let mut logs = Vec::new();
        let mut failure: Option<String> = None;
        let mut codes: Vec<([u8; 32], Arc<Vec<u8>>)> = Vec::new();
        let mut created: Vec<String> = Vec::new();
        let mut gas_burnt = 0u64;

        for (index, action) in r.actions.iter().enumerate() {
            let fail = |kind: String| Some(format!("ActionError {{ index: {index}, kind: {kind} }}"));
            // nearcore's account existence check.
            match action {
                Action::CreateAccount => {
                    if account.is_some() {
                        failure = fail(format!("AccountAlreadyExists {{ account_id: {:?} }}", r.receiver));
                        break;
                    }
                }
                Action::Transfer { deposit } if account.is_none() => {
                    let only = r.actions.len() == 1;
                    match account_type(&r.receiver) {
                        AccountType::NearImplicit if only && !is_refund => {
                            account = Some(implicit_account(&r.receiver, 0));
                            created.push(r.receiver.clone());
                            let a = account.as_mut().expect("account");
                            a.amount = *deposit;
                            continue;
                        }
                        AccountType::EthImplicit if only && !is_refund => {
                            return Err(Halt::Unevaluable("an ETH-implicit account creation, which v2 does not model".into()));
                        }
                        _ => {
                            failure = fail(format!("AccountDoesNotExist {{ account_id: {:?} }}", r.receiver));
                            break;
                        }
                    }
                }
                _ if account.is_none() => {
                    failure = fail(format!("AccountDoesNotExist {{ account_id: {:?} }}", r.receiver));
                    break;
                }
                _ => {}
            }
            // nearcore's actor permission check.
            if matches!(action, Action::DeployContract { .. } | Action::AddKey { .. } | Action::DeleteKey { .. } | Action::DeleteAccount { .. }) && actor != r.receiver {
                failure = fail(format!("ActorNoPermission {{ account_id: {:?}, actor_id: {:?} }}", r.receiver, actor));
                break;
            }
            if matches!(action, Action::DeleteAccount { .. }) && account.as_ref().is_some_and(|a| a.locked != 0) {
                failure = fail(format!("DeleteAccountStaking {{ account_id: {:?} }}", r.receiver));
                break;
            }
            match action {
                Action::CreateAccount => {
                    let id = &r.receiver;
                    let kind = if !is_valid_account_id(id) || account_type(id) != AccountType::Named {
                        Some(format!("OnlyImplicitAccountCreationAllowed {{ account_id: {id:?} }}"))
                    } else if is_top_level(id) {
                        (id.len() < MIN_ALLOWED_TOP_LEVEL_ACCOUNT_LENGTH && r.predecessor != REGISTRAR_ACCOUNT_ID).then(|| {
                            format!("CreateAccountOnlyByRegistrar {{ account_id: {id:?}, registrar_account_id: \"registrar\", predecessor_id: {:?} }}", r.predecessor)
                        })
                    } else if !is_sub_account_of(id, &r.predecessor) {
                        Some(format!("CreateAccountNotAllowed {{ account_id: {id:?}, predecessor_id: {:?} }}", r.predecessor))
                    } else {
                        None
                    };
                    if let Some(k) = kind {
                        failure = fail(k);
                        break;
                    }
                    account = Some(Account::new(0, 0));
                    actor = r.receiver.clone();
                    created.push(r.receiver.clone());
                }
                Action::DeployContract { code } => {
                    let a = account.as_mut().expect("account");
                    let old = a.code.and_then(|h| self.world.codes.get(&h).map(|c| c.len()).or_else(|| codes.iter().find(|(x, _)| *x == h).map(|(_, c)| c.len())));
                    let h = sha256(code);
                    a.storage_usage = a.storage_usage.saturating_sub(old.unwrap_or(0) as u64) + code.len() as u64;
                    a.code = Some(h);
                    codes.push((h, code.clone()));
                }
                Action::FunctionCall { method, args, deposit, gas, .. } => {
                    let a = account.as_mut().expect("account");
                    let Some(h) = a.code else {
                        failure = fail(format!("FunctionCallError(CompilationError(CodeDoesNotExist {{ account_id: {:?} }}))", r.receiver));
                        break;
                    };
                    let code = codes.iter().rev().find(|(x, _)| *x == h).map(|(_, c)| c.clone()).or_else(|| self.world.codes.get(&h).cloned());
                    let Some(code) = code else {
                        return Err(Halt::Unevaluable("a contract's code is missing".into()));
                    };
                    let compiled = self.vm.contract(h, &code, &self.cfg).map_err(|e| Halt::Unevaluable(format!("module refused at call time: {e}")))?;
                    let ctx = Ctx {
                        current_account_id: r.receiver.clone(),
                        signer_account_id: r.signer.clone(),
                        signer_account_pk: r.signer_pk.clone(),
                        predecessor_account_id: r.predecessor.clone(),
                        input: args.clone(),
                        block_height: height,
                        block_timestamp: timestamp,
                        epoch_height: self.block.epoch_height,
                        random_seed: self.block.random_seed.to_vec(),
                        account_balance: a.amount,
                        account_locked_balance: a.locked,
                        storage_usage: a.storage_usage,
                        attached_deposit: *deposit,
                        prepaid_gas: *gas,
                        promise_results: results.clone(),
                        output_data_receivers: r.output_data_receivers.iter().map(|(_, rcv)| rcv.clone()).collect(),
                        is_view: false,
                    };
                    let ex = exec::execute(&self.vm.engine, &compiled, self.cfg.clone(), ctx, a.data.clone(), method, self.fuel_per_call);
                    self.fuel_total = self.fuel_total.saturating_add(ex.instructions);
                    if let Some(limit) = self.fuel_total_limit {
                        if self.fuel_total > limit {
                            return Err(Halt::Bound(format!("the bundle ran more than {limit} instructions")));
                        }
                    }
                    gas_burnt = gas_burnt.saturating_add(ex.burnt_gas);
                    logs.extend(ex.logs.iter().cloned());
                    if let Outcome::Unevaluable(why) = &ex.outcome {
                        return Err(Halt::Unevaluable(why.clone()));
                    }
                    if self.margin_check {
                        // The call's own gas: what it did not attach to the receipts it created.
                        let own = gas.saturating_sub(ex.promises_gas) as u128;
                        if ex.burnt_gas as u128 * self.cfg.margin_den as u128 > own * self.cfg.margin_num as u128 {
                            return Err(Halt::Unevaluable("gas: a call ended within 10% of its gas".into()));
                        }
                    }
                    match ex.outcome {
                        Outcome::Failed(e) => {
                            failure = fail(format!("FunctionCallError({e})"));
                            break;
                        }
                        Outcome::Ok(rd) => {
                            a.amount = ex.balance;
                            a.storage_usage = ex.storage_usage;
                            if !ex.overlay.is_empty() {
                                let data = Arc::make_mut(&mut a.data);
                                for (k, v) in ex.overlay {
                                    match v {
                                        Some(v) => data.insert(k, v),
                                        None => data.remove(&k),
                                    };
                                }
                            }
                            let mut new = ex.receipts;
                            distribute_unused_gas(&mut new, &ex.gas_weights, gas.saturating_sub(ex.used_gas));
                            if let Some(e) = validate_receipts(&new, &self.cfg) {
                                failure = fail(format!("NewReceiptValidationError({e})"));
                                break;
                            }
                            let ids = self.link(&r, new, &mut pending);
                            value = match rd {
                                ReturnData::None => Value::None,
                                ReturnData::Value(v) => Value::Bytes(v),
                                ReturnData::ReceiptIndex(i) => Value::Receipt(ids[i]),
                            };
                        }
                        Outcome::Unevaluable(_) => unreachable!("handled above"),
                    }
                }
                Action::Transfer { deposit } => {
                    let a = account.as_mut().expect("account");
                    a.amount = a.amount.checked_add(*deposit).ok_or_else(|| Halt::Unevaluable("a balance overflowed u128".into()))?;
                }
                Action::AddKey { public_key, key } => {
                    let a = account.as_mut().expect("account");
                    if a.keys.contains_key(public_key) {
                        failure = fail(format!("AddKeyAlreadyExists {{ account_id: {:?}, public_key: {:?} }}", r.receiver, hex::encode(public_key)));
                        break;
                    }
                    a.storage_usage += key_storage(public_key, key);
                    a.keys.insert(public_key.clone(), key.clone());
                }
                Action::DeleteKey { public_key } => {
                    let a = account.as_mut().expect("account");
                    let Some(key) = a.keys.remove(public_key) else {
                        failure = fail(format!("DeleteKeyDoesNotExist {{ account_id: {:?}, public_key: {:?} }}", r.receiver, hex::encode(public_key)));
                        break;
                    };
                    a.storage_usage = a.storage_usage.saturating_sub(key_storage(public_key, &key));
                }
                Action::DeleteAccount { beneficiary } => {
                    let a = account.as_ref().expect("account");
                    let code_len = a.code.and_then(|h| self.world.codes.get(&h).map(|c| c.len() as u64)).unwrap_or(0);
                    if a.storage_usage.saturating_sub(code_len) > MAX_ACCOUNT_DELETION_STORAGE_USAGE {
                        failure = fail(format!("DeleteAccountWithLargeState {{ account_id: {:?} }}", r.receiver));
                        break;
                    }
                    if a.amount > 0 {
                        let refund = self.system_transfer(beneficiary, a.amount, r.depth + 1);
                        pending.push(refund);
                    }
                    account = None;
                    actor = r.predecessor.clone();
                }
            }
        }

        // nearcore's storage staking check on the receiver.
        if failure.is_none() {
            if let Some(a) = &account {
                let required = (a.storage_usage as u128).saturating_mul(STORAGE_AMOUNT_PER_BYTE);
                let available = a.amount.saturating_add(a.locked);
                if available < required && a.storage_usage > ZERO_BALANCE_ACCOUNT_STORAGE_LIMIT {
                    failure = Some(format!("LackBalanceForState {{ account_id: {:?}, amount: {} }}", r.receiver, required - available));
                }
            }
        }

        let success = failure.is_none();
        let record_value = match (&value, success) {
            (Value::Bytes(v), true) => Some(crate::util::b64(v)),
            (Value::Receipt(id), true) => Some(format!("receipt:{id}")),
            _ => None,
        };
        if success {
            match account {
                Some(a) => {
                    self.world.accounts.insert(r.receiver.clone(), a);
                }
                None => {
                    self.world.accounts.remove(&r.receiver);
                }
            }
            for (h, c) in codes {
                self.world.codes.insert(h, c);
            }
            self.created_accounts.extend(created);
            // The call's receipts first, then its data (nearcore appends data receipts after them).
            let data = match value {
                Value::Receipt(id) => {
                    if let Some(p) = pending.iter_mut().find(|p| p.id == id) {
                        p.output_data_receivers.extend(r.output_data_receivers.iter().cloned());
                    }
                    None
                }
                Value::Bytes(v) => Some(v),
                Value::None => Some(vec![]),
            };
            for p in pending {
                self.enqueue(p);
            }
            if let Some(v) = data {
                for (d, _) in &r.output_data_receivers {
                    self.deliver(*d, Some(v.clone()));
                }
            }
        } else {
            for (d, _) in &r.output_data_receivers {
                self.deliver(*d, None);
            }
            let refund: u128 = r.actions.iter().map(Action::deposit).sum();
            if refund > 0 && !is_refund {
                let rr = self.system_transfer(&r.predecessor, refund, r.depth + 1);
                self.enqueue(rr);
            }
        }
        self.records.push(ReceiptRecord {
            id: r.id,
            phase: self.phase.clone(),
            call: self.call,
            block_height: height,
            predecessor: r.predecessor.clone(),
            receiver: r.receiver.clone(),
            actions: r.actions.iter().map(Action::describe).collect(),
            success,
            value: record_value,
            error: failure,
            logs,
            gas_burnt,
        });
        Ok(())
    }

    /// Turns a call's new receipts into receipts of the run: ids, depth, and the data links of
    /// `promise_then` (each dependency sends its result to the receipt that waits on it). Returns their ids.
    fn link(&mut self, parent: &Receipt, new: Vec<NewReceipt>, pending: &mut Vec<Receipt>) -> Vec<u64> {
        let start = pending.len();
        let mut ids = Vec::with_capacity(new.len());
        for n in new {
            let id = self.receipt_id();
            ids.push(id);
            let mut input = Vec::new();
            for d in &n.deps {
                let data_id = self.next_data;
                self.next_data += 1;
                pending[start + d].output_data_receivers.push((data_id, n.receiver.clone()));
                input.push(data_id);
            }
            pending.push(Receipt {
                id,
                predecessor: parent.receiver.clone(),
                receiver: n.receiver,
                signer: parent.signer.clone(),
                signer_pk: parent.signer_pk.clone(),
                depth: parent.depth + 1,
                input_data_ids: input,
                output_data_receivers: vec![],
                actions: n.actions,
            });
        }
        ids
    }
}

/// nearcore's weight distribution: the call's unused gas goes to its weighted function calls, in the order
/// they were appended, the last one taking the remainder.
fn distribute_unused_gas(receipts: &mut [NewReceipt], weights: &[(usize, usize)], unused: u64) {
    let weight_of = |r: &[NewReceipt], (i, j): (usize, usize)| match &r[i].actions[j] {
        Action::FunctionCall { weight, .. } => *weight as u128,
        _ => 0,
    };
    let total: u128 = weights.iter().map(|&w| weight_of(receipts, w)).sum();
    if total == 0 {
        return;
    }
    let mut distributed = 0u64;
    for (k, &(i, j)) in weights.iter().enumerate() {
        let w = weight_of(receipts, (i, j));
        let assigned = if k + 1 == weights.len() { unused - distributed } else { (unused as u128 * w / total) as u64 };
        distributed += assigned;
        if let Action::FunctionCall { gas, .. } = &mut receipts[i].actions[j] {
            *gas = gas.saturating_add(assigned);
        }
    }
}

/// nearcore's checks on the receipts a call creates (`validate_receipt`): receiver ids, at most 100
/// actions, deletion last, method name and argument lengths, total prepaid gas, access key method names.
fn validate_receipts(receipts: &[NewReceipt], cfg: &Config) -> Option<String> {
    let l = &cfg.limits;
    for r in receipts {
        if !is_valid_account_id(&r.receiver) {
            return Some(format!("InvalidReceiverId {{ account_id: {:?} }}", r.receiver));
        }
        if r.actions.len() as u64 > l.max_actions_per_receipt {
            return Some(format!(
                "ActionsValidation(TotalNumberOfActionsExceeded {{ total_number_of_actions: {}, limit: {} }})",
                r.actions.len(),
                l.max_actions_per_receipt
            ));
        }
        let mut gas = 0u64;
        for (i, a) in r.actions.iter().enumerate() {
            match a {
                Action::DeleteAccount { beneficiary } => {
                    if i + 1 != r.actions.len() {
                        return Some("ActionsValidation(DeleteActionMustBeFinal)".into());
                    }
                    if !is_valid_account_id(beneficiary) {
                        return Some(format!("ActionsValidation(InvalidAccountId {{ account_id: {beneficiary:?} }})"));
                    }
                }
                Action::FunctionCall { method, args, gas: g, .. } => {
                    if method.len() as u64 > l.max_length_method_name {
                        return Some("ActionsValidation(FunctionCallMethodNameLengthExceeded)".into());
                    }
                    if args.len() as u64 > l.max_arguments_length {
                        return Some("ActionsValidation(FunctionCallArgumentsLengthExceeded)".into());
                    }
                    gas = match gas.checked_add(*g) {
                        Some(v) => v,
                        None => return Some("ActionsValidation(IntegerOverflow)".into()),
                    };
                }
                Action::AddKey { key, .. } => {
                    if let crate::state::KeyPermission::FunctionCall { receiver, methods, .. } = &key.permission {
                        if !is_valid_account_id(receiver) {
                            return Some(format!("ActionsValidation(InvalidAccountId {{ account_id: {receiver:?} }})"));
                        }
                        let bytes: u64 = methods.iter().map(|m| m.len() as u64 + 1).sum();
                        if bytes > l.max_number_bytes_method_names || methods.iter().any(|m| m.len() as u64 > l.max_length_method_name) {
                            return Some("ActionsValidation(AddKeyMethodNamesNumberOfBytesExceeded)".into());
                        }
                    }
                }
                _ => {}
            }
        }
        if gas > l.max_total_prepaid_gas {
            return Some(format!(
                "ActionsValidation(TotalPrepaidGasExceeded {{ total_prepaid_gas: {gas}, limit: {} }})",
                l.max_total_prepaid_gas
            ));
        }
    }
    None
}

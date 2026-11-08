//! NEAR's host functions with nearcore's semantics: the same checks in the same order, the same limits,
//! the same gas charges (src/config.rs), and the same errors, as nearcore's VM logic for protocol 74.
//!
//! Every function listed in `prepare::NEAR_HOST_FUNCTIONS` is implemented here; the refused ones never
//! link (a module importing one is refused before it runs). Where v2 departs from nearcore it says so:
//! gas exhaustion stops the run as `Unevaluable` instead of failing the receipt, and storage operations
//! charge no trie-node fees (nearcore's depend on its trie's shape).
#![allow(unused_variables)]

use std::{collections::BTreeMap, sync::Arc};

use wasmi::{Caller, FuncType, Linker, Memory, TrapCode, Val};

use crate::{
    config::Config,
    crypto,
    gas::{GasCounter, GasStop},
    state::{AccessKey, KeyPermission},
    util::{account_type, AccountType},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromiseResult {
    Successful(Vec<u8>),
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnData {
    None,
    Value(Vec<u8>),
    /// The call returned the promise of one of the receipts it created (by its index in the call).
    ReceiptIndex(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    CreateAccount,
    DeployContract { code: Arc<Vec<u8>> },
    FunctionCall { method: String, args: Vec<u8>, deposit: u128, gas: u64, weight: u64 },
    Transfer { deposit: u128 },
    AddKey { public_key: Vec<u8>, key: AccessKey },
    DeleteKey { public_key: Vec<u8> },
    DeleteAccount { beneficiary: String },
}

impl Action {
    pub fn deposit(&self) -> u128 {
        match self {
            Action::FunctionCall { deposit, .. } | Action::Transfer { deposit } => *deposit,
            _ => 0,
        }
    }

    /// A short, gas-free description for the receipt.
    pub fn describe(&self) -> String {
        match self {
            Action::CreateAccount => "CreateAccount".into(),
            Action::DeployContract { code } => format!("DeployContract({})", hex::encode(crate::util::sha256(code))),
            Action::FunctionCall { method, deposit, .. } => format!("FunctionCall({method}, {deposit})"),
            Action::Transfer { deposit } => format!("Transfer({deposit})"),
            Action::AddKey { public_key, .. } => format!("AddKey({})", hex::encode(public_key)),
            Action::DeleteKey { public_key } => format!("DeleteKey({})", hex::encode(public_key)),
            Action::DeleteAccount { beneficiary } => format!("DeleteAccount({beneficiary})"),
        }
    }
}

/// A receipt a call creates: its receiver, the receipts (of the same call) it waits on, and its actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReceipt {
    pub receiver: String,
    pub deps: Vec<usize>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone)]
enum Promise {
    Receipt(usize),
    NotReceipt(Vec<usize>),
}

/// The context a call runs in (nearcore's `VMContext`).
#[derive(Debug, Clone)]
pub struct Ctx {
    pub current_account_id: String,
    pub signer_account_id: String,
    /// Borsh-encoded public key.
    pub signer_account_pk: Vec<u8>,
    pub predecessor_account_id: String,
    pub input: Vec<u8>,
    pub block_height: u64,
    pub block_timestamp: u64,
    pub epoch_height: u64,
    pub random_seed: Vec<u8>,
    pub account_balance: u128,
    pub account_locked_balance: u128,
    pub storage_usage: u64,
    pub attached_deposit: u128,
    pub prepaid_gas: u64,
    pub promise_results: Vec<PromiseResult>,
    /// The receivers of the data this call's result is sent to.
    pub output_data_receivers: Vec<String>,
    pub is_view: bool,
}

/// Why a call stopped before returning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// A NEAR host error: the call fails, as it would on NEAR (nearcore's `HostError`, formatted as it does).
    Host(String),
    /// v2 cannot judge the run: gas exhausted, a float executed, an instruction budget reached, ...
    Unevaluable(String),
}

impl From<GasStop> for Stop {
    fn from(g: GasStop) -> Self {
        match g {
            GasStop::Exceeded => Stop::Unevaluable("gas: a call exhausted its gas".into()),
            GasStop::Overflow => Stop::Host("IntegerOverflow".into()),
        }
    }
}

fn host(msg: impl Into<String>) -> Stop {
    Stop::Host(msg.into())
}

fn violation() -> Stop {
    host("MemoryAccessViolation")
}

fn prohibited(name: &str) -> Stop {
    host(format!("ProhibitedInView {{ method_name: \"{name}\" }}"))
}

/// The state of one call: nearcore's `VMLogic` and its result state.
pub struct Host {
    pub cfg: Arc<Config>,
    pub ctx: Ctx,
    pub memory: Option<Memory>,
    registers: BTreeMap<u64, Vec<u8>>,
    registers_usage: u64,
    pub gas: GasCounter,
    /// The account's balance as the call sees it: balance plus attached deposit, minus what it attaches.
    pub balance: u128,
    pub storage_usage: u64,
    /// The account's data before the call, and the call's writes over it (`None`: removed).
    pub storage: Arc<BTreeMap<Vec<u8>, Vec<u8>>>,
    pub overlay: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    pub logs: Vec<String>,
    total_log_length: u64,
    pub return_data: ReturnData,
    promises: Vec<Promise>,
    pub receipts: Vec<NewReceipt>,
    /// Weighted function calls `(receipt, action)`, in the order they were appended (nearcore's order for
    /// distributing unused gas).
    pub gas_weights: Vec<(usize, usize)>,
    pub stop: Option<Stop>,
    /// The call's instruction budget left, the fuel given to wasmi at the last sync, and instructions run.
    pub fuel_left: u64,
    pub fuel_window: u64,
    pub instructions: u64,
    /// Memory the host cannot back traps (src/exec.rs).
    pub limiter: crate::exec::NoQuietGrowth,
}

impl Host {
    pub fn new(cfg: Arc<Config>, ctx: Ctx, storage: Arc<BTreeMap<Vec<u8>, Vec<u8>>>, fuel_budget: u64) -> Result<Self, Stop> {
        let balance = ctx
            .account_balance
            .checked_add(ctx.attached_deposit)
            .ok_or_else(|| Stop::Unevaluable("balance plus deposit overflows".into()))?;
        Ok(Host {
            gas: GasCounter::new(cfg.limits.max_gas_burnt, ctx.prepaid_gas, ctx.is_view),
            cfg,
            balance,
            storage_usage: ctx.storage_usage,
            ctx,
            memory: None,
            registers: BTreeMap::new(),
            registers_usage: 0,
            storage,
            overlay: BTreeMap::new(),
            logs: Vec::new(),
            total_log_length: 0,
            return_data: ReturnData::None,
            promises: Vec::new(),
            receipts: Vec::new(),
            gas_weights: Vec::new(),
            stop: None,
            fuel_left: fuel_budget,
            fuel_window: 0,
            instructions: 0,
            limiter: crate::exec::NoQuietGrowth,
        })
    }

    // ---- memory and registers (nearcore's vmstate) ----

    fn mem_view<'m>(&mut self, mem: &'m [u8], ptr: u64, len: u64) -> Result<&'m [u8], Stop> {
        self.gas.pay_base(self.cfg.ext.read_memory_base)?;
        self.gas.pay_per(self.cfg.ext.read_memory_byte, len)?;
        slice(mem, ptr, len).ok_or_else(violation)
    }

    fn mem_write(&mut self, mem: &mut [u8], ptr: u64, data: &[u8]) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.write_memory_base)?;
        self.gas.pay_per(self.cfg.ext.write_memory_byte, data.len() as u64)?;
        let end = ptr.checked_add(data.len() as u64).ok_or_else(violation)?;
        if end > mem.len() as u64 {
            return Err(violation());
        }
        mem[ptr as usize..end as usize].copy_from_slice(data);
        Ok(())
    }

    fn mem_get<const N: usize>(&mut self, mem: &[u8], ptr: u64) -> Result<[u8; N], Stop> {
        let s = self.mem_view(mem, ptr, N as u64)?;
        Ok(s.try_into().unwrap())
    }

    fn mem_get_u128(&mut self, mem: &[u8], ptr: u64) -> Result<u128, Stop> {
        Ok(u128::from_le_bytes(self.mem_get::<16>(mem, ptr)?))
    }

    fn reg_get(&mut self, id: u64) -> Result<Vec<u8>, Stop> {
        let Some(data) = self.registers.get(&id) else {
            return Err(host(format!("InvalidRegisterId {{ register_id: {id} }}")));
        };
        let data = data.clone();
        self.gas.pay_base(self.cfg.ext.read_register_base)?;
        self.gas.pay_per(self.cfg.ext.read_register_byte, data.len() as u64)?;
        Ok(data)
    }

    fn reg_set(&mut self, id: u64, data: &[u8]) -> Result<(), Stop> {
        let len = data.len() as u64;
        self.gas.pay_base(self.cfg.ext.write_register_base)?;
        self.gas.pay_per(self.cfg.ext.write_register_byte, len)?;
        let l = &self.cfg.limits;
        if len > l.max_register_size {
            return Err(violation());
        }
        // nearcore's rule, kept: at the register count limit even replacing a register fails.
        if self.registers.len() as u64 >= l.max_number_registers {
            return Err(violation());
        }
        let old = self.registers.get(&id).map(|d| d.len() as u64 + 8).unwrap_or(0);
        let usage = (self.registers_usage - old).checked_add(len + 8).ok_or_else(violation)?;
        if usage > l.registers_memory_limit {
            return Err(violation());
        }
        self.registers_usage = usage;
        self.registers.insert(id, data.to_vec());
        Ok(())
    }

    /// nearcore's `get_memory_or_register`: `len == u64::MAX` reads register `ptr`.
    fn mem_or_reg(&mut self, mem: &[u8], ptr: u64, len: u64) -> Result<Vec<u8>, Stop> {
        if len == u64::MAX {
            self.reg_get(ptr)
        } else {
            Ok(self.mem_view(mem, ptr, len)?.to_vec())
        }
    }

    fn view_check(&self, name: &str) -> Result<(), Stop> {
        if self.ctx.is_view {
            Err(prohibited(name))
        } else {
            Ok(())
        }
    }

    // ---- strings and logs ----

    fn log_room(&self) -> u64 {
        self.cfg.limits.max_total_log_length.saturating_sub(self.total_log_length)
    }

    fn log_exceeded(&self, add: u64) -> Stop {
        host(format!(
            "TotalLogLengthExceeded {{ length: {}, limit: {} }}",
            self.total_log_length.saturating_add(add),
            self.cfg.limits.max_total_log_length
        ))
    }

    fn get_utf8_string(&mut self, mem: &[u8], len: u64, ptr: u64) -> Result<String, Stop> {
        self.gas.pay_base(self.cfg.ext.utf8_decoding_base)?;
        let max_len = self.log_room();
        let buf = if len != u64::MAX {
            if len > max_len {
                return Err(self.log_exceeded(len));
            }
            self.mem_view(mem, ptr, len)?.to_vec()
        } else {
            let mut buf = vec![];
            let mut i = 0u64;
            loop {
                let at = ptr.checked_add(i).ok_or_else(violation)?;
                let el = self.mem_get::<1>(mem, at)?[0];
                if el == 0 {
                    break;
                }
                if i == max_len {
                    return Err(self.log_exceeded(max_len.saturating_add(1)));
                }
                buf.push(el);
                i += 1;
            }
            buf
        };
        self.gas.pay_per(self.cfg.ext.utf8_decoding_byte, buf.len() as u64)?;
        String::from_utf8(buf).map_err(|_| host("BadUTF8"))
    }

    fn get_utf16_string(&mut self, mem: &[u8], mut len: u64, ptr: u64) -> Result<String, Stop> {
        self.gas.pay_base(self.cfg.ext.utf16_decoding_base)?;
        let max_len = self.log_room();
        let bytes = if len == u64::MAX {
            len = self.nul_terminated_utf16_len(mem, ptr, max_len)?;
            slice(mem, ptr, len).ok_or_else(violation)?.to_vec()
        } else {
            self.mem_view(mem, ptr, len)?.to_vec()
        };
        if bytes.len() % 2 != 0 {
            return Err(host("BadUTF16"));
        }
        if len > max_len {
            return Err(self.log_exceeded(len));
        }
        self.gas.pay_per(self.cfg.ext.utf16_decoding_byte, len)?;
        char::decode_utf16(bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])))
            .collect::<Result<String, _>>()
            .map_err(|_| host("BadUTF16"))
    }

    fn nul_terminated_utf16_len(&mut self, mem: &[u8], ptr: u64, max_len: u64) -> Result<u64, Stop> {
        let mut len = 0u64;
        loop {
            if u16::from_le_bytes(self.mem_get::<2>(mem, ptr.saturating_add(len))?) == 0 {
                return Ok(len);
            }
            len = match len.checked_add(2) {
                Some(l) if l <= max_len => l,
                Some(l) => return Err(self.log_exceeded(l)),
                None => return Err(self.log_exceeded(u64::MAX)),
            };
        }
    }

    fn check_can_add_log(&self) -> Result<(), Stop> {
        if self.logs.len() as u64 >= self.cfg.limits.max_number_logs {
            Err(host(format!("NumberOfLogsExceeded {{ limit: {} }}", self.cfg.limits.max_number_logs)))
        } else {
            Ok(())
        }
    }

    fn checked_push_log(&mut self, message: String) -> Result<(), Stop> {
        let len = message.len() as u64;
        let Some(total) = self.total_log_length.checked_add(len) else {
            return Err(self.log_exceeded(len));
        };
        self.total_log_length = total;
        if total > self.cfg.limits.max_total_log_length {
            return Err(self.log_exceeded(len));
        }
        self.logs.push(message);
        Ok(())
    }

    fn read_account_id(&mut self, mem: &[u8], ptr: u64, len: u64) -> Result<String, Stop> {
        let buf = self.mem_or_reg(mem, ptr, len)?;
        self.gas.pay_base(self.cfg.ext.utf8_decoding_base)?;
        self.gas.pay_per(self.cfg.ext.utf8_decoding_byte, buf.len() as u64)?;
        String::from_utf8(buf).map_err(|_| host("BadUTF8"))
    }

    // ---- context ----

    fn current_account_id(&mut self, reg: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        let v = self.ctx.current_account_id.clone().into_bytes();
        self.reg_set(reg, &v)
    }

    fn signer_account_id(&mut self, reg: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("signer_account_id")?;
        let v = self.ctx.signer_account_id.clone().into_bytes();
        self.reg_set(reg, &v)
    }

    fn signer_account_pk(&mut self, reg: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("signer_account_pk")?;
        let v = self.ctx.signer_account_pk.clone();
        self.reg_set(reg, &v)
    }

    fn predecessor_account_id(&mut self, reg: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("predecessor_account_id")?;
        let v = self.ctx.predecessor_account_id.clone().into_bytes();
        self.reg_set(reg, &v)
    }

    fn input(&mut self, reg: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        let v = self.ctx.input.clone();
        self.reg_set(reg, &v)
    }

    fn base_value(&mut self, v: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        Ok(v)
    }

    // ---- economics ----

    fn write_u128(&mut self, mem: &mut [u8], ptr: u64, v: u128) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.mem_write(mem, ptr, &v.to_le_bytes())
    }

    fn prepaid_gas(&mut self) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("prepaid_gas")?;
        Ok(self.ctx.prepaid_gas)
    }

    fn used_gas(&mut self) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("used_gas")?;
        Ok(self.gas.used())
    }

    // ---- math ----

    fn random_seed(&mut self, reg: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        let v = self.ctx.random_seed.clone();
        self.reg_set(reg, &v)
    }

    fn hash(&mut self, mem: &[u8], which: Hash, len: u64, ptr: u64, reg: u64) -> Result<(), Stop> {
        let e = &self.cfg.ext;
        let (base, byte) = match which {
            Hash::Sha256 => (e.sha256_base, e.sha256_byte),
            Hash::Keccak256 => (e.keccak256_base, e.keccak256_byte),
            Hash::Keccak512 => (e.keccak512_base, e.keccak512_byte),
            Hash::Ripemd160 => (e.ripemd160_base, e.ripemd160_block),
        };
        self.gas.pay_base(base)?;
        let value = self.mem_or_reg(mem, ptr, len)?;
        let out = match which {
            Hash::Ripemd160 => {
                let blocks = (value.len() as u64).checked_add(8).ok_or_else(|| host("IntegerOverflow"))? / 64 + 1;
                self.gas.pay_per(byte, blocks)?;
                crypto::ripemd160(&value)
            }
            _ => {
                self.gas.pay_per(byte, value.len() as u64)?;
                match which {
                    Hash::Sha256 => crypto::sha256(&value).to_vec(),
                    Hash::Keccak256 => crypto::keccak256(&value),
                    _ => crypto::keccak512(&value),
                }
            }
        };
        self.reg_set(reg, &out)
    }

    #[allow(clippy::too_many_arguments)]
    fn ecrecover(&mut self, mem: &[u8], hash_len: u64, hash_ptr: u64, sig_len: u64, sig_ptr: u64, v: u64, malleability: u64, reg: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.ecrecover_base)?;
        let sig = self.mem_or_reg(mem, sig_ptr, sig_len)?;
        let sig: [u8; 64] = sig.as_slice().try_into().map_err(|_| {
            host(format!("ECRecoverError {{ msg: \"The length of the signature: {}, exceeds the limit of 64 bytes\" }}", sig.len()))
        })?;
        if v >= 4 {
            return Err(host(format!("ECRecoverError {{ msg: \"V recovery byte 0 through 3 are valid but was provided {v}\" }}")));
        }
        let hash = self.mem_or_reg(mem, hash_ptr, hash_len)?;
        let hash: [u8; 32] = hash.as_slice().try_into().map_err(|_| {
            host(format!("ECRecoverError {{ msg: \"The length of the hash: {}, exceeds the limit of 32 bytes\" }}", hash.len()))
        })?;
        if malleability > 1 {
            return Err(host(format!(
                "ECRecoverError {{ msg: \"Malleability flag needs to be 0 or 1, but is instead {malleability}\" }}"
            )));
        }
        if !crypto::check_signature_values(&sig, malleability != 0) {
            return Ok(0);
        }
        match crypto::ecrecover(&hash, &sig, v as u8) {
            Some(pk) => {
                self.reg_set(reg, &pk)?;
                Ok(1)
            }
            None => Ok(0),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ed25519_verify(&mut self, mem: &[u8], sig_len: u64, sig_ptr: u64, msg_len: u64, msg_ptr: u64, pk_len: u64, pk_ptr: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.ed25519_verify_base)?;
        let sig = self.mem_or_reg(mem, sig_ptr, sig_len)?;
        let sig: [u8; 64] =
            sig.as_slice().try_into().map_err(|_| host("Ed25519VerifyInvalidInput { msg: \"invalid signature length\" }"))?;
        if sig[63] & 0b1110_0000 != 0 {
            return Ok(0);
        }
        let message = self.mem_or_reg(mem, msg_ptr, msg_len)?;
        self.gas.pay_per(self.cfg.ext.ed25519_verify_byte, message.len() as u64)?;
        let pk = self.mem_or_reg(mem, pk_ptr, pk_len)?;
        let pk: [u8; 32] =
            pk.as_slice().try_into().map_err(|_| host("Ed25519VerifyInvalidInput { msg: \"invalid public key length\" }"))?;
        Ok(crypto::ed25519_verify(&sig, &message, &pk) as u64)
    }

    fn gas_opcodes(&mut self, opcodes: u32) -> Result<(), Stop> {
        Ok(self.gas.burn((opcodes as u64).saturating_mul(self.cfg.regular_op_cost))?)
    }

    // ---- promises ----

    fn push_promise(&mut self, p: Promise) -> Result<u64, Stop> {
        let idx = self.promises.len() as u64;
        self.promises.push(p);
        let limit = self.cfg.limits.max_promises_per_function_call_action;
        if self.promises.len() as u64 > limit {
            return Err(host(format!("NumberPromisesExceeded {{ number_of_promises: {}, limit: {limit} }}", self.promises.len())));
        }
        Ok(idx)
    }

    fn pay_new_receipt(&mut self, sir: bool, deps: &[bool]) -> Result<(), Stop> {
        let a = &self.cfg.actions;
        let mut burn = a.action_receipt_creation.send(sir);
        let mut use_ = a.action_receipt_creation.execution;
        for &dep in deps {
            burn = burn
                .checked_add(a.data_receipt_creation_base.send(dep))
                .and_then(|b| b.checked_add(a.data_receipt_creation_base.execution))
                .ok_or_else(|| host("IntegerOverflow"))?;
        }
        use_ = use_.checked_add(burn).ok_or_else(|| host("IntegerOverflow"))?;
        Ok(self.gas.pay_action(burn, use_)?)
    }

    fn pay_action_base(&mut self, fee: crate::config::Fee, sir: bool) -> Result<(), Stop> {
        let burn = fee.send(sir);
        let use_ = burn.checked_add(fee.execution).ok_or_else(|| host("IntegerOverflow"))?;
        Ok(self.gas.pay_action(burn, use_)?)
    }

    fn pay_action_per_byte(&mut self, fee: crate::config::Fee, n: u64, sir: bool) -> Result<(), Stop> {
        let over = || host("IntegerOverflow");
        let burn = n.checked_mul(fee.send(sir)).ok_or_else(over)?;
        let use_ = burn.checked_add(n.checked_mul(fee.execution).ok_or_else(over)?).ok_or_else(over)?;
        Ok(self.gas.pay_action(burn, use_)?)
    }

    fn deduct_balance(&mut self, amount: u128) -> Result<(), Stop> {
        self.balance = self.balance.checked_sub(amount).ok_or_else(|| host("BalanceExceeded"))?;
        Ok(())
    }

    fn promise_batch_create(&mut self, mem: &[u8], acc_len: u64, acc_ptr: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_batch_create")?;
        let account = self.read_account_id(mem, acc_ptr, acc_len)?;
        let sir = account == self.ctx.current_account_id;
        self.pay_new_receipt(sir, &[])?;
        self.receipts.push(NewReceipt { receiver: account, deps: vec![], actions: vec![] });
        self.push_promise(Promise::Receipt(self.receipts.len() - 1))
    }

    fn promise_batch_then(&mut self, mem: &[u8], promise_idx: u64, acc_len: u64, acc_ptr: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_batch_then")?;
        let account = self.read_account_id(mem, acc_ptr, acc_len)?;
        let deps = match self.promises.get(usize::try_from(promise_idx).unwrap_or(usize::MAX)) {
            Some(Promise::Receipt(r)) => vec![*r],
            Some(Promise::NotReceipt(rs)) => rs.clone(),
            None => return Err(host(format!("InvalidPromiseIndex {{ promise_idx: {promise_idx} }}"))),
        };
        let sir = account == self.ctx.current_account_id;
        let deps_sir: Vec<bool> = deps.iter().map(|&d| self.receipts[d].receiver == account).collect();
        self.pay_new_receipt(sir, &deps_sir)?;
        self.receipts.push(NewReceipt { receiver: account, deps, actions: vec![] });
        self.push_promise(Promise::Receipt(self.receipts.len() - 1))
    }

    fn promise_and(&mut self, mem: &[u8], ptr: u64, count: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_and")?;
        self.gas.pay_base(self.cfg.ext.promise_and_base)?;
        let len = count.checked_mul(8).ok_or_else(|| host("IntegerOverflow"))?;
        self.gas.pay_per(self.cfg.ext.promise_and_per_promise, len)?;
        let raw = self.mem_view(mem, ptr, len)?.to_vec();
        let mut deps = vec![];
        for c in raw.chunks_exact(8) {
            let idx = u64::from_le_bytes(c.try_into().unwrap());
            match self.promises.get(usize::try_from(idx).unwrap_or(usize::MAX)) {
                Some(Promise::Receipt(r)) => deps.push(*r),
                Some(Promise::NotReceipt(rs)) => deps.extend(rs.iter().copied()),
                None => return Err(host(format!("InvalidPromiseIndex {{ promise_idx: {idx} }}"))),
            }
            let limit = self.cfg.limits.max_number_input_data_dependencies;
            if deps.len() as u64 > limit {
                return Err(host(format!(
                    "NumberInputDataDependenciesExceeded {{ number_of_input_data_dependencies: {}, limit: {limit} }}",
                    deps.len()
                )));
            }
        }
        self.push_promise(Promise::NotReceipt(deps))
    }

    fn receipt_of(&self, promise_idx: u64) -> Result<(usize, bool), Stop> {
        let r = match self.promises.get(usize::try_from(promise_idx).unwrap_or(usize::MAX)) {
            Some(Promise::Receipt(r)) => *r,
            Some(Promise::NotReceipt(_)) => return Err(host("CannotAppendActionToJointPromise")),
            None => return Err(host(format!("InvalidPromiseIndex {{ promise_idx: {promise_idx} }}"))),
        };
        Ok((r, self.receipts[r].receiver == self.ctx.current_account_id))
    }

    fn action_create_account(&mut self, promise_idx: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_batch_action_create_account")?;
        let (r, sir) = self.receipt_of(promise_idx)?;
        self.pay_action_base(self.cfg.actions.create_account, sir)?;
        self.receipts[r].actions.push(Action::CreateAccount);
        Ok(())
    }

    fn action_deploy_contract(&mut self, mem: &[u8], promise_idx: u64, code_len: u64, code_ptr: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_batch_action_deploy_contract")?;
        let code = self.mem_or_reg(mem, code_ptr, code_len)?;
        let limit = self.cfg.limits.max_contract_size;
        if code.len() as u64 > limit {
            return Err(host(format!("ContractSizeExceeded {{ size: {}, limit: {limit} }}", code.len())));
        }
        let (r, sir) = self.receipt_of(promise_idx)?;
        self.pay_action_base(self.cfg.actions.deploy_contract, sir)?;
        self.pay_action_per_byte(self.cfg.actions.deploy_contract_per_byte, code.len() as u64, sir)?;
        self.receipts[r].actions.push(Action::DeployContract { code: Arc::new(code) });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn action_function_call(
        &mut self,
        mem: &[u8],
        promise_idx: u64,
        m_len: u64,
        m_ptr: u64,
        a_len: u64,
        a_ptr: u64,
        amount_ptr: u64,
        gas: u64,
        weight: u64,
    ) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_batch_action_function_call")?;
        let amount = self.mem_get_u128(mem, amount_ptr)?;
        let method = self.mem_or_reg(mem, m_ptr, m_len)?;
        if method.is_empty() {
            return Err(host("EmptyMethodName"));
        }
        let args = self.mem_or_reg(mem, a_ptr, a_len)?;
        let (r, sir) = self.receipt_of(promise_idx)?;
        let n = method.len() as u64 + args.len() as u64;
        self.pay_action_base(self.cfg.actions.function_call, sir)?;
        self.pay_action_per_byte(self.cfg.actions.function_call_per_byte, n, sir)?;
        self.gas.prepay(gas)?;
        self.deduct_balance(amount)?;
        let method = String::from_utf8(method).map_err(|_| host("InvalidMethodName"))?;
        self.receipts[r].actions.push(Action::FunctionCall { method, args, deposit: amount, gas, weight });
        if weight > 0 {
            self.gas_weights.push((r, self.receipts[r].actions.len() - 1));
        }
        Ok(())
    }

    fn action_transfer(&mut self, mem: &[u8], promise_idx: u64, amount_ptr: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_batch_action_transfer")?;
        let amount = self.mem_get_u128(mem, amount_ptr)?;
        let (r, sir) = self.receipt_of(promise_idx)?;
        let a = &self.cfg.actions;
        // nearcore's transfer fees with implicit account creation enabled (protocol 74).
        let (send, exec) = match account_type(&self.receipts[r].receiver) {
            AccountType::Named => (a.transfer.send(sir), a.transfer.execution),
            AccountType::NearImplicit => (
                a.transfer.send(sir) + a.create_account.send(sir) + a.add_full_access_key.send(sir),
                a.transfer.execution + a.create_account.execution + a.add_full_access_key.execution,
            ),
            AccountType::EthImplicit => (a.transfer.send(sir) + a.create_account.send(sir), a.transfer.execution + a.create_account.execution),
        };
        let use_ = send.checked_add(exec).ok_or_else(|| host("IntegerOverflow"))?;
        self.gas.pay_action(send, use_)?;
        self.deduct_balance(amount)?;
        self.receipts[r].actions.push(Action::Transfer { deposit: amount });
        Ok(())
    }

    fn action_add_full_access_key(&mut self, mem: &[u8], promise_idx: u64, pk_len: u64, pk_ptr: u64, nonce: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_batch_action_add_key_with_full_access")?;
        let pk = self.mem_or_reg(mem, pk_ptr, pk_len)?;
        let (r, sir) = self.receipt_of(promise_idx)?;
        self.pay_action_base(self.cfg.actions.add_full_access_key, sir)?;
        let public_key = decode_public_key(&pk)?;
        self.receipts[r].actions.push(Action::AddKey { public_key, key: AccessKey { nonce, permission: KeyPermission::FullAccess } });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn action_add_function_call_key(
        &mut self,
        mem: &[u8],
        promise_idx: u64,
        pk_len: u64,
        pk_ptr: u64,
        nonce: u64,
        allowance_ptr: u64,
        recv_len: u64,
        recv_ptr: u64,
        names_len: u64,
        names_ptr: u64,
    ) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_batch_action_add_key_with_function_call")?;
        let pk = self.mem_or_reg(mem, pk_ptr, pk_len)?;
        let allowance = self.mem_get_u128(mem, allowance_ptr)?;
        let allowance = if allowance > 0 { Some(allowance) } else { None };
        let receiver = self.read_account_id(mem, recv_ptr, recv_len)?;
        let raw = self.mem_or_reg(mem, names_ptr, names_len)?;
        let names: Vec<Vec<u8>> = if raw.is_empty() {
            vec![]
        } else {
            raw.split(|c| *c == b',').map(|v| if v.is_empty() { Err(host("EmptyMethodName")) } else { Ok(v.to_vec()) }).collect::<Result<_, _>>()?
        };
        let (r, sir) = self.receipt_of(promise_idx)?;
        let n: u64 = names.iter().map(|v| v.len() as u64 + 1).sum();
        self.pay_action_base(self.cfg.actions.add_function_call_key, sir)?;
        self.pay_action_per_byte(self.cfg.actions.add_function_call_key_per_byte, n, sir)?;
        let public_key = decode_public_key(&pk)?;
        let methods = names.into_iter().map(|m| String::from_utf8(m).map_err(|_| host("InvalidMethodName"))).collect::<Result<_, _>>()?;
        self.receipts[r].actions.push(Action::AddKey {
            public_key,
            key: AccessKey { nonce, permission: KeyPermission::FunctionCall { allowance, receiver, methods } },
        });
        Ok(())
    }

    fn action_delete_key(&mut self, mem: &[u8], promise_idx: u64, pk_len: u64, pk_ptr: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_batch_action_delete_key")?;
        let pk = self.mem_or_reg(mem, pk_ptr, pk_len)?;
        let (r, sir) = self.receipt_of(promise_idx)?;
        self.pay_action_base(self.cfg.actions.delete_key, sir)?;
        let public_key = decode_public_key(&pk)?;
        self.receipts[r].actions.push(Action::DeleteKey { public_key });
        Ok(())
    }

    fn action_delete_account(&mut self, mem: &[u8], promise_idx: u64, ben_len: u64, ben_ptr: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_batch_action_delete_account")?;
        let beneficiary = self.read_account_id(mem, ben_ptr, ben_len)?;
        let (r, sir) = self.receipt_of(promise_idx)?;
        self.pay_action_base(self.cfg.actions.delete_account, sir)?;
        self.receipts[r].actions.push(Action::DeleteAccount { beneficiary });
        Ok(())
    }

    fn promise_results_count(&mut self) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_results_count")?;
        Ok(self.ctx.promise_results.len() as u64)
    }

    fn promise_result(&mut self, idx: u64, reg: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("promise_result")?;
        match self.ctx.promise_results.get(usize::try_from(idx).unwrap_or(usize::MAX)).cloned() {
            None => Err(host(format!("InvalidPromiseResultIndex {{ result_idx: {idx} }}"))),
            Some(PromiseResult::Successful(data)) => {
                self.reg_set(reg, &data)?;
                Ok(1)
            }
            Some(PromiseResult::Failed) => Ok(2),
        }
    }

    fn promise_return(&mut self, promise_idx: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.gas.pay_base(self.cfg.ext.promise_return)?;
        self.view_check("promise_return")?;
        match self.promises.get(usize::try_from(promise_idx).unwrap_or(usize::MAX)) {
            Some(Promise::Receipt(r)) => {
                self.return_data = ReturnData::ReceiptIndex(*r);
                Ok(())
            }
            Some(Promise::NotReceipt(_)) => Err(host("CannotReturnJointPromise")),
            None => Err(host(format!("InvalidPromiseIndex {{ promise_idx: {promise_idx} }}"))),
        }
    }

    // ---- results and logs ----

    fn value_return(&mut self, mem: &[u8], len: u64, ptr: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        let value = self.mem_or_reg(mem, ptr, len)?;
        let n = value.len() as u64;
        let limit = self.cfg.limits.max_length_returned_data;
        if n > limit {
            return Err(host(format!("ReturnedValueLengthExceeded {{ length: {n}, limit: {limit} }}")));
        }
        let fee = self.cfg.actions.data_receipt_creation_per_byte;
        let mut burn = 0u64;
        for receiver in &self.ctx.output_data_receivers {
            let sir = *receiver == self.ctx.current_account_id;
            let per = fee.send(sir).checked_add(fee.execution).and_then(|f| f.checked_mul(n)).ok_or_else(|| host("IntegerOverflow"))?;
            burn = burn.checked_add(per).ok_or_else(|| host("IntegerOverflow"))?;
        }
        self.gas.pay_action(burn, burn)?;
        self.return_data = ReturnData::Value(value);
        Ok(())
    }

    fn panic(&mut self) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        Err(host("GuestPanic { panic_msg: \"explicit guest panic\" }"))
    }

    fn panic_utf8(&mut self, mem: &[u8], len: u64, ptr: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        let msg = self.get_utf8_string(mem, len, ptr)?;
        Err(host(format!("GuestPanic {{ panic_msg: {msg:?} }}")))
    }

    fn log_utf8(&mut self, mem: &[u8], len: u64, ptr: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.check_can_add_log()?;
        let message = self.get_utf8_string(mem, len, ptr)?;
        self.gas.pay_base(self.cfg.ext.log_base)?;
        self.gas.pay_per(self.cfg.ext.log_byte, message.len() as u64)?;
        self.checked_push_log(message)
    }

    fn log_utf16(&mut self, mem: &[u8], len: u64, ptr: u64) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.check_can_add_log()?;
        let message = self.get_utf16_string(mem, len, ptr)?;
        self.gas.pay_base(self.cfg.ext.log_base)?;
        self.gas.pay_per(self.cfg.ext.log_byte, message.len() as u64)?;
        self.checked_push_log(message)
    }

    fn abort(&mut self, mem: &[u8], msg_ptr: u32, filename_ptr: u32, line: u32, col: u32) -> Result<(), Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        if msg_ptr < 4 || filename_ptr < 4 {
            return Err(host("BadUTF16"));
        }
        self.check_can_add_log()?;
        let msg_len = u32::from_le_bytes(self.mem_get::<4>(mem, (msg_ptr - 4) as u64)?);
        let filename_len = u32::from_le_bytes(self.mem_get::<4>(mem, (filename_ptr - 4) as u64)?);
        let msg = self.get_utf16_string(mem, msg_len as u64, msg_ptr as u64)?;
        let filename = self.get_utf16_string(mem, filename_len as u64, filename_ptr as u64)?;
        let message = format!("{msg}, filename: \"{filename}\" line: {line} col: {col}");
        self.gas.pay_base(self.cfg.ext.log_base)?;
        self.gas.pay_per(self.cfg.ext.log_byte, message.len() as u64)?;
        self.checked_push_log(format!("ABORT: {message}"))?;
        Err(host(format!("GuestPanic {{ panic_msg: {message:?} }}")))
    }

    // ---- storage ----

    pub fn storage_get(&self, key: &[u8]) -> Option<&Vec<u8>> {
        match self.overlay.get(key) {
            Some(v) => v.as_ref(),
            None => self.storage.get(key),
        }
    }

    fn key_check(&self, key: &[u8]) -> Result<(), Stop> {
        let limit = self.cfg.limits.max_length_storage_key;
        if key.len() as u64 > limit {
            return Err(host(format!("KeyLengthExceeded {{ length: {}, limit: {limit} }}", key.len())));
        }
        Ok(())
    }

    fn storage_write(&mut self, mem: &[u8], k_len: u64, k_ptr: u64, v_len: u64, v_ptr: u64, reg: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("storage_write")?;
        self.gas.pay_base(self.cfg.ext.storage_write_base)?;
        let key = self.mem_or_reg(mem, k_ptr, k_len)?;
        self.key_check(&key)?;
        let value = self.mem_or_reg(mem, v_ptr, v_len)?;
        let limit = self.cfg.limits.max_length_storage_value;
        if value.len() as u64 > limit {
            return Err(host(format!("ValueLengthExceeded {{ length: {}, limit: {limit} }}", value.len())));
        }
        self.gas.pay_per(self.cfg.ext.storage_write_key_byte, key.len() as u64)?;
        self.gas.pay_per(self.cfg.ext.storage_write_value_byte, value.len() as u64)?;
        let evicted = self.storage_get(&key).cloned();
        if let Some(old) = &evicted {
            self.gas.pay_per(self.cfg.ext.storage_write_evicted_byte, old.len() as u64)?;
        }
        let (klen, vlen) = (key.len() as u64, value.len() as u64);
        self.overlay.insert(key, Some(value));
        match evicted {
            Some(old) => {
                self.storage_usage = self.storage_usage.saturating_sub(old.len() as u64).saturating_add(vlen);
                self.reg_set(reg, &old)?;
                Ok(1)
            }
            None => {
                self.storage_usage += vlen + klen + crate::config::STORAGE_NUM_EXTRA_BYTES_RECORD;
                Ok(0)
            }
        }
    }

    fn storage_read(&mut self, mem: &[u8], k_len: u64, k_ptr: u64, reg: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.gas.pay_base(self.cfg.ext.storage_read_base)?;
        let key = self.mem_or_reg(mem, k_ptr, k_len)?;
        self.key_check(&key)?;
        self.gas.pay_per(self.cfg.ext.storage_read_key_byte, key.len() as u64)?;
        match self.storage_get(&key).cloned() {
            Some(value) => {
                self.gas.pay_per(self.cfg.ext.storage_read_value_byte, value.len() as u64)?;
                self.reg_set(reg, &value)?;
                Ok(1)
            }
            None => Ok(0),
        }
    }

    fn storage_remove(&mut self, mem: &[u8], k_len: u64, k_ptr: u64, reg: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.view_check("storage_remove")?;
        self.gas.pay_base(self.cfg.ext.storage_remove_base)?;
        let key = self.mem_or_reg(mem, k_ptr, k_len)?;
        self.key_check(&key)?;
        self.gas.pay_per(self.cfg.ext.storage_remove_key_byte, key.len() as u64)?;
        let removed = self.storage_get(&key).cloned();
        if let Some(v) = &removed {
            self.gas.pay_per(self.cfg.ext.storage_remove_ret_value_byte, v.len() as u64)?;
        }
        let klen = key.len() as u64;
        self.overlay.insert(key, None);
        match removed {
            Some(value) => {
                self.storage_usage = self.storage_usage.saturating_sub(value.len() as u64 + klen + crate::config::STORAGE_NUM_EXTRA_BYTES_RECORD);
                self.reg_set(reg, &value)?;
                Ok(1)
            }
            None => Ok(0),
        }
    }

    fn storage_has_key(&mut self, mem: &[u8], k_len: u64, k_ptr: u64) -> Result<u64, Stop> {
        self.gas.pay_base(self.cfg.ext.base)?;
        self.gas.pay_base(self.cfg.ext.storage_has_key_base)?;
        let key = self.mem_or_reg(mem, k_ptr, k_len)?;
        self.key_check(&key)?;
        self.gas.pay_per(self.cfg.ext.storage_has_key_byte, key.len() as u64)?;
        Ok(self.storage_get(&key).is_some() as u64)
    }
}

#[derive(Clone, Copy)]
enum Hash {
    Sha256,
    Keccak256,
    Keccak512,
    Ripemd160,
}

fn slice(mem: &[u8], ptr: u64, len: u64) -> Option<&[u8]> {
    let end = ptr.checked_add(len)?;
    if end > mem.len() as u64 {
        return None;
    }
    Some(&mem[ptr as usize..end as usize])
}

/// A borsh-encoded NEAR public key: `0 ‖ 32 bytes` (ed25519) or `1 ‖ 64 bytes` (secp256k1).
pub fn decode_public_key(b: &[u8]) -> Result<Vec<u8>, Stop> {
    match (b.first(), b.len()) {
        (Some(0), 33) | (Some(1), 65) => Ok(b.to_vec()),
        _ => Err(host("InvalidPublicKey")),
    }
}

// ---- metering: wasmi fuel → gas ----

/// Charges the gas of the instructions run since the last sync.
pub fn sync_fuel(caller: &mut Caller<'_, Host>) -> Result<(), Stop> {
    let now = caller.get_fuel().map_err(|_| Stop::Unevaluable("fuel metering is off".into()))?;
    let h = caller.data_mut();
    let used = h.fuel_window.saturating_sub(now);
    h.fuel_window = now;
    h.instructions += used;
    h.fuel_left = h.fuel_left.saturating_sub(used);
    let op = h.cfg.regular_op_cost;
    Ok(h.gas.burn(used.saturating_mul(op))?)
}

/// The fuel wasmi gets until the next host call: the call's instruction budget, capped by the gas left.
pub fn window(h: &Host) -> u64 {
    let ops = if h.ctx.is_view { u64::MAX } else { h.gas.remaining() / h.cfg.regular_op_cost.max(1) + 1 };
    h.fuel_left.min(ops)
}

fn refuel(caller: &mut Caller<'_, Host>) -> Result<(), wasmi::Error> {
    let w = window(caller.data());
    caller.data_mut().fuel_window = w;
    caller.set_fuel(w)
}

fn fail(caller: &mut Caller<'_, Host>, stop: Stop) -> wasmi::Error {
    caller.data_mut().stop = Some(stop);
    wasmi::Error::new("v2: the call stopped")
}

/// Runs one host function: charges the wasm instructions since the last call, runs `f` over the host
/// state and the guest's memory, and gives wasmi its next fuel window.
fn hostcall<R>(caller: &mut Caller<'_, Host>, f: impl FnOnce(&mut Host, &mut [u8]) -> Result<R, Stop>) -> Result<R, wasmi::Error> {
    if let Err(s) = sync_fuel(caller) {
        return Err(fail(caller, s));
    }
    let Some(mem) = caller.data().memory else {
        return Err(fail(caller, Stop::Unevaluable("no memory".into())));
    };
    let (data, h) = mem.data_and_store_mut(&mut *caller);
    match f(h, data) {
        Ok(r) => {
            refuel(caller)?;
            Ok(r)
        }
        Err(s) => Err(fail(caller, s)),
    }
}

macro_rules! def {
    ($l:ident, $name:literal, |$h:ident, $m:ident $(, $a:ident : $t:ty)*| -> $r:ty $body:block) => {
        $l.func_wrap("env", $name, move |mut c: Caller<'_, Host>, $($a: $t),*| -> Result<$r, wasmi::Error> {
            hostcall(&mut c, |$h: &mut Host, $m: &mut [u8]| -> Result<$r, Stop> { $body })
        })
        .map_err(|e| e.to_string())?;
    };
}

/// Defines every host function v2 implements in `linker` (module `env`).
pub fn link(l: &mut Linker<Host>) -> Result<(), String> {
    def!(l, "read_register", |h, m, id: u64, ptr: u64| -> () {
        h.gas.pay_base(h.cfg.ext.base)?;
        let data = h.reg_get(id)?;
        h.mem_write(m, ptr, &data)
    });
    def!(l, "register_len", |h, m, id: u64| -> u64 {
        h.gas.pay_base(h.cfg.ext.base)?;
        Ok(h.registers.get(&id).map(|d| d.len() as u64).unwrap_or(u64::MAX))
    });
    def!(l, "write_register", |h, m, id: u64, len: u64, ptr: u64| -> () {
        h.gas.pay_base(h.cfg.ext.base)?;
        let data = h.mem_view(m, ptr, len)?.to_vec();
        h.reg_set(id, &data)
    });
    def!(l, "current_account_id", |h, m, r: u64| -> () { h.current_account_id(r) });
    def!(l, "signer_account_id", |h, m, r: u64| -> () { h.signer_account_id(r) });
    def!(l, "signer_account_pk", |h, m, r: u64| -> () { h.signer_account_pk(r) });
    def!(l, "predecessor_account_id", |h, m, r: u64| -> () { h.predecessor_account_id(r) });
    def!(l, "input", |h, m, r: u64| -> () { h.input(r) });
    def!(l, "block_index", |h, m| -> u64 { let v = h.ctx.block_height; h.base_value(v) });
    def!(l, "block_timestamp", |h, m| -> u64 { let v = h.ctx.block_timestamp; h.base_value(v) });
    def!(l, "epoch_height", |h, m| -> u64 { let v = h.ctx.epoch_height; h.base_value(v) });
    def!(l, "storage_usage", |h, m| -> u64 { let v = h.storage_usage; h.base_value(v) });
    def!(l, "account_balance", |h, m, p: u64| -> () { let v = h.balance; h.write_u128(m, p, v) });
    def!(l, "account_locked_balance", |h, m, p: u64| -> () { let v = h.ctx.account_locked_balance; h.write_u128(m, p, v) });
    def!(l, "attached_deposit", |h, m, p: u64| -> () { let v = h.ctx.attached_deposit; h.write_u128(m, p, v) });
    def!(l, "prepaid_gas", |h, m| -> u64 { h.prepaid_gas() });
    def!(l, "used_gas", |h, m| -> u64 { h.used_gas() });
    def!(l, "random_seed", |h, m, r: u64| -> () { h.random_seed(r) });
    def!(l, "sha256", |h, m, len: u64, ptr: u64, r: u64| -> () { h.hash(m, Hash::Sha256, len, ptr, r) });
    def!(l, "keccak256", |h, m, len: u64, ptr: u64, r: u64| -> () { h.hash(m, Hash::Keccak256, len, ptr, r) });
    def!(l, "keccak512", |h, m, len: u64, ptr: u64, r: u64| -> () { h.hash(m, Hash::Keccak512, len, ptr, r) });
    def!(l, "ripemd160", |h, m, len: u64, ptr: u64, r: u64| -> () { h.hash(m, Hash::Ripemd160, len, ptr, r) });
    def!(l, "ecrecover", |h, m, hl: u64, hp: u64, sl: u64, sp: u64, v: u64, mf: u64, r: u64| -> u64 { h.ecrecover(m, hl, hp, sl, sp, v, mf, r) });
    def!(l, "ed25519_verify", |h, m, sl: u64, sp: u64, ml: u64, mp: u64, pl: u64, pp: u64| -> u64 { h.ed25519_verify(m, sl, sp, ml, mp, pl, pp) });
    def!(l, "value_return", |h, m, len: u64, ptr: u64| -> () { h.value_return(m, len, ptr) });
    def!(l, "panic", |h, m| -> () { h.panic() });
    def!(l, "panic_utf8", |h, m, len: u64, ptr: u64| -> () { h.panic_utf8(m, len, ptr) });
    def!(l, "log_utf8", |h, m, len: u64, ptr: u64| -> () { h.log_utf8(m, len, ptr) });
    def!(l, "log_utf16", |h, m, len: u64, ptr: u64| -> () { h.log_utf16(m, len, ptr) });
    def!(l, "abort", |h, m, mp: u32, fp: u32, line: u32, col: u32| -> () { h.abort(m, mp, fp, line, col) });
    def!(l, "promise_create", |h, m, al: u64, ap: u64, ml: u64, mp: u64, gl: u64, gp: u64, amt: u64, gas: u64| -> u64 {
        let p = h.promise_batch_create(m, al, ap)?;
        h.action_function_call(m, p, ml, mp, gl, gp, amt, gas, 0)?;
        Ok(p)
    });
    def!(l, "promise_then", |h, m, pi: u64, al: u64, ap: u64, ml: u64, mp: u64, gl: u64, gp: u64, amt: u64, gas: u64| -> u64 {
        let p = h.promise_batch_then(m, pi, al, ap)?;
        h.action_function_call(m, p, ml, mp, gl, gp, amt, gas, 0)?;
        Ok(p)
    });
    def!(l, "promise_and", |h, m, ptr: u64, count: u64| -> u64 { h.promise_and(m, ptr, count) });
    def!(l, "promise_batch_create", |h, m, al: u64, ap: u64| -> u64 { h.promise_batch_create(m, al, ap) });
    def!(l, "promise_batch_then", |h, m, pi: u64, al: u64, ap: u64| -> u64 { h.promise_batch_then(m, pi, al, ap) });
    def!(l, "promise_batch_action_create_account", |h, m, pi: u64| -> () { h.action_create_account(pi) });
    def!(l, "promise_batch_action_deploy_contract", |h, m, pi: u64, cl: u64, cp: u64| -> () { h.action_deploy_contract(m, pi, cl, cp) });
    def!(l, "promise_batch_action_function_call", |h, m, pi: u64, ml: u64, mp: u64, gl: u64, gp: u64, amt: u64, gas: u64| -> () {
        h.action_function_call(m, pi, ml, mp, gl, gp, amt, gas, 0)
    });
    def!(l, "promise_batch_action_function_call_weight", |h, m, pi: u64, ml: u64, mp: u64, gl: u64, gp: u64, amt: u64, gas: u64, w: u64| -> () {
        h.action_function_call(m, pi, ml, mp, gl, gp, amt, gas, w)
    });
    def!(l, "promise_batch_action_transfer", |h, m, pi: u64, amt: u64| -> () { h.action_transfer(m, pi, amt) });
    def!(l, "promise_batch_action_add_key_with_full_access", |h, m, pi: u64, pl: u64, pp: u64, nonce: u64| -> () {
        h.action_add_full_access_key(m, pi, pl, pp, nonce)
    });
    def!(l, "promise_batch_action_add_key_with_function_call", |h, m, pi: u64, pl: u64, pp: u64, nonce: u64, ap: u64, rl: u64, rp: u64, nl: u64, np: u64| -> () {
        h.action_add_function_call_key(m, pi, pl, pp, nonce, ap, rl, rp, nl, np)
    });
    def!(l, "promise_batch_action_delete_key", |h, m, pi: u64, pl: u64, pp: u64| -> () { h.action_delete_key(m, pi, pl, pp) });
    def!(l, "promise_batch_action_delete_account", |h, m, pi: u64, bl: u64, bp: u64| -> () { h.action_delete_account(m, pi, bl, bp) });
    def!(l, "promise_results_count", |h, m| -> u64 { h.promise_results_count() });
    def!(l, "promise_result", |h, m, i: u64, r: u64| -> u64 { h.promise_result(i, r) });
    def!(l, "promise_return", |h, m, p: u64| -> () { h.promise_return(p) });
    def!(l, "storage_write", |h, m, kl: u64, kp: u64, vl: u64, vp: u64, r: u64| -> u64 { h.storage_write(m, kl, kp, vl, vp, r) });
    def!(l, "storage_read", |h, m, kl: u64, kp: u64, r: u64| -> u64 { h.storage_read(m, kl, kp, r) });
    def!(l, "storage_remove", |h, m, kl: u64, kp: u64, r: u64| -> u64 { h.storage_remove(m, kl, kp, r) });
    def!(l, "storage_has_key", |h, m, kl: u64, kp: u64| -> u64 { h.storage_has_key(m, kl, kp) });
    def!(l, "storage_iter_prefix", |h, m, a: u64, b: u64| -> u64 { Err(host("Deprecated { method_name: \"storage_iter_prefix\" }")) });
    def!(l, "storage_iter_range", |h, m, a: u64, b: u64, c: u64, d: u64| -> u64 { Err(host("Deprecated { method_name: \"storage_iter_range\" }")) });
    def!(l, "storage_iter_next", |h, m, a: u64, b: u64, c: u64| -> u64 { Err(host("Deprecated { method_name: \"storage_iter_next\" }")) });
    def!(l, "gas", |h, m, opcodes: u32| -> () { h.gas_opcodes(opcodes) });
    // The refused host functions: linked (near-sdk imports them), but calling one stops the run by name
    // (src/prepare.rs REFUSED_HOST_FUNCTIONS). It never returns, so the result slots stay untouched.
    let to_wasmi = |v: &wasmparser::ValType| match v {
        wasmparser::ValType::I32 => wasmi::ValType::I32,
        _ => wasmi::ValType::I64,
    };
    for r in crate::prepare::REFUSED_HOST_FUNCTIONS {
        let ty = FuncType::new(r.params.iter().map(to_wasmi), r.results.iter().map(to_wasmi));
        let reason = format!("a run called {}, which v2 refuses: {}", r.name, r.reason);
        l.func_new("env", r.name, ty, move |mut c: Caller<'_, Host>, _: &[Val], _: &mut [Val]| -> Result<(), wasmi::Error> {
            Err(fail(&mut c, Stop::Unevaluable(reason.clone())))
        })
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// nearcore's `WasmTrap` names for wasmi's traps (every one of wasmi 2.0.0's, named). `None`: v2 cannot judge it
/// (fuel, stack, the host's memory).
pub fn wasm_trap(t: TrapCode) -> Option<&'static str> {
    Some(match t {
        TrapCode::UnreachableCodeReached => "Unreachable",
        TrapCode::MemoryOutOfBounds => "MemoryOutOfBounds",
        TrapCode::TableOutOfBounds => "CallIndirectOOB",
        TrapCode::IndirectCallToNull => "IndirectCallToNull",
        TrapCode::IntegerDivisionByZero | TrapCode::IntegerOverflow | TrapCode::BadConversionToInteger => "IllegalArithmetic",
        TrapCode::BadSignature => "IncorrectCallIndirectSignature",
        // The host's limits, not the contract's behaviour: never a NEAR failure (src/exec.rs, NoQuietGrowth).
        TrapCode::StackOverflow | TrapCode::OutOfFuel | TrapCode::OutOfSystemMemory | TrapCode::GrowthOperationLimited => return None,
    })
}

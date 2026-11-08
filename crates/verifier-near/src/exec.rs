//! One function call under wasmi: the engine configuration, the compiled-module cache, and the mapping
//! of an execution's end to NEAR's outcomes (success, a NEAR failure) or to `Unevaluable`.

use std::{collections::BTreeMap, sync::Arc};

use wasmi::{CompilationMode, Engine, Linker, Memory, MemoryType, Module, Store, Val};

use crate::{
    config::Config,
    logic::{self, Ctx, Host, NewReceipt, ReturnData, Stop},
    prepare::{self, Kind, FLOAT_FLAG_EXPORT},
};

/// A module ready to instantiate.
#[derive(Clone)]
pub struct Compiled {
    pub module: Module,
    pub float_flag: bool,
    /// The original code's length (contract loading is charged on it).
    pub code_len: u64,
    pub imports: Vec<String>,
    pub has_start: bool,
}

/// wasmi configured for NEAR: fuel on, eager compilation (no lazily charged translation), and the same
/// feature set src/prepare.rs validates against. Reference-types *parsing* is on for the one reason given
/// there (near-sdk's overlong `call_indirect` encoding); src/prepare.rs has already refused every
/// reference-types instruction and construct, so nothing reference-types-specific reaches the engine.
pub fn new_engine() -> Engine {
    let mut c = wasmi::Config::default();
    c.consume_fuel(true)
        .compilation_mode(CompilationMode::Eager)
        .wasm_mutable_global(true)
        .wasm_sign_extension(true)
        .wasm_reference_types(true)
        .wasm_saturating_float_to_int(false)
        .wasm_multi_value(false)
        .wasm_multi_memory(false)
        .wasm_bulk_memory(false)
        .wasm_tail_call(false)
        .wasm_extended_const(false)
        .wasm_custom_page_sizes(false)
        .wasm_wide_arithmetic(false)
        .floats(true)
        .ignore_custom_sections(true)
        .set_max_recursion_depth(2048);
    Engine::new(&c)
}

/// Memory the host cannot back traps instead of failing quietly. On a failed allocation wasmi's `memory.grow`
/// returns -1 to the contract, which then runs on along a path NEAR never takes (NEAR always backs its pages):
/// natively the allocation succeeds, in the enclave under its memory cap it may not, and the two builds would
/// judge the same proof differently. Trapping makes the call `Unevaluable`, and no verdict rests on it. Growth
/// past a memory's maximum still returns -1, as on NEAR: wasmi refuses it before asking the host.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoQuietGrowth;

impl wasmi::ResourceLimiter for NoQuietGrowth {
    fn memory_growing(&mut self, _current: usize, _desired: usize, _maximum: Option<usize>) -> Result<bool, wasmi_core::LimiterError> {
        Ok(true)
    }
    fn table_growing(&mut self, _current: usize, _desired: usize, _maximum: Option<usize>) -> Result<bool, wasmi_core::LimiterError> {
        Ok(true)
    }
    fn memory_grow_failed(&mut self, error: &wasmi_core::MemoryError) -> Result<(), wasmi_core::LimiterError> {
        match error {
            wasmi_core::MemoryError::OutOfSystemMemory => Err(wasmi_core::LimiterError::ResourceLimiterDeniedAllocation),
            _ => Ok(()),
        }
    }
    fn instances(&self) -> usize {
        10_000
    }
    fn tables(&self) -> usize {
        10_000
    }
    fn memories(&self) -> usize {
        10_000
    }
}

/// The engine and every contract compiled so far, by the sha256 of its original code.
#[derive(Clone)]
pub struct Vm {
    pub engine: Engine,
    contracts: BTreeMap<[u8; 32], Result<Arc<Compiled>, String>>,
}

impl Default for Vm {
    fn default() -> Self {
        Vm { engine: new_engine(), contracts: BTreeMap::new() }
    }
}

impl Vm {
    /// Prepares and compiles a contract (cached). An error is the reason the module is refused.
    pub fn contract(&mut self, hash: [u8; 32], code: &[u8], cfg: &Config) -> Result<Arc<Compiled>, String> {
        if let Some(c) = self.contracts.get(&hash) {
            return c.clone();
        }
        let r = compile(&self.engine, code, Kind::Contract, cfg).map(Arc::new);
        self.contracts.insert(hash, r.clone());
        r
    }

    /// Every contract that compiled.
    pub fn compiled(&self) -> BTreeMap<[u8; 32], Arc<Compiled>> {
        self.contracts.iter().filter_map(|(h, c)| c.as_ref().ok().map(|c| (*h, c.clone()))).collect()
    }
}

pub fn compile(engine: &Engine, code: &[u8], kind: Kind, cfg: &Config) -> Result<Compiled, String> {
    let p = prepare::prepare(code, kind, &cfg.limits)?;
    let module = Module::new(engine, &p.wasm).map_err(|e| format!("the module does not compile: {e}"))?;
    Ok(Compiled { module, float_flag: p.float_flag, code_len: code.len() as u64, imports: p.imports, has_start: p.has_start })
}

/// How a call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok(ReturnData),
    /// A NEAR failure, as nearcore names it (`FunctionCallError`).
    Failed(String),
    /// v2 cannot judge the run.
    Unevaluable(String),
}

/// Everything one call produced.
pub struct Exec {
    pub outcome: Outcome,
    pub logs: Vec<String>,
    pub receipts: Vec<NewReceipt>,
    pub gas_weights: Vec<(usize, usize)>,
    pub balance: u128,
    pub storage_usage: u64,
    pub overlay: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    pub burnt_gas: u64,
    pub used_gas: u64,
    /// Gas set aside for outgoing receipts (fees and attached gas), before any weight distribution.
    pub promises_gas: u64,
    pub instructions: u64,
}

/// Runs `method` of `compiled` in `ctx` over the account's `storage`, with at most `fuel_budget` wasm
/// instructions.
pub fn execute(
    engine: &Engine,
    compiled: &Compiled,
    cfg: Arc<Config>,
    ctx: Ctx,
    storage: Arc<BTreeMap<Vec<u8>, Vec<u8>>>,
    method: &str,
    fuel_budget: u64,
) -> Exec {
    let mut host = match Host::new(cfg.clone(), ctx, storage, fuel_budget) {
        Ok(h) => h,
        Err(Stop::Unevaluable(r)) | Err(Stop::Host(r)) => return empty(Outcome::Unevaluable(r)),
    };
    if method.is_empty() {
        return finish(host, Outcome::Failed("MethodResolveError(MethodEmptyName)".into()));
    }
    // Contract loading, charged after loading as protocol 74 does (`fix_contract_loading_cost` is off).
    if !host.ctx.is_view {
        let loading = compiled
            .code_len
            .checked_mul(cfg.ext.contract_loading_bytes)
            .and_then(|g| g.checked_add(cfg.ext.contract_loading_base))
            .unwrap_or(u64::MAX);
        if host.gas.burn(loading).is_err() {
            return finish(host, Outcome::Unevaluable("gas: contract loading exhausted the call's gas".into()));
        }
    }
    let first = logic::window(&host);
    host.fuel_window = first;
    let mut store = Store::new(engine, host);
    store.limiter(|h| &mut h.limiter);
    if store.set_fuel(first).is_err() {
        return finish(store.into_data(), Outcome::Unevaluable("fuel metering is off".into()));
    }
    let limits = &cfg.limits;
    let memory = match Memory::new(&mut store, MemoryType::new(limits.initial_memory_pages, Some(limits.max_memory_pages))) {
        Ok(m) => m,
        Err(e) => return finish(store.into_data(), Outcome::Unevaluable(format!("memory: {e}"))),
    };
    store.data_mut().memory = Some(memory);
    let mut linker = <Linker<Host>>::new(engine);
    let linked = logic::link(&mut linker).and_then(|_| linker.define("env", "memory", memory).map(|_| ()).map_err(|e| e.to_string()));
    if let Err(e) = linked {
        return finish(store.into_data(), Outcome::Unevaluable(format!("link: {e}")));
    }
    let instance = match linker.instantiate_and_start(&mut store, &compiled.module) {
        Ok(i) => i,
        // A trap in the start function. With float code in the module the flag cannot be read (the
        // instance never reached us), so v2 does not judge it.
        Err(e) if compiled.float_flag && compiled.has_start => {
            return end(store, &cfg, Err(e), false).unjudged("float: a start function trapped in a module with float code");
        }
        Err(e) => return end(store, &cfg, Err(e), false),
    };
    let Some(func) = instance.get_func(&store, method) else {
        return finish(store.into_data(), Outcome::Failed("MethodResolveError(MethodNotFound)".into()));
    };
    let ty = func.ty(&store);
    if !ty.params().is_empty() || !ty.results().is_empty() {
        return finish(store.into_data(), Outcome::Failed("MethodResolveError(MethodInvalidSignature)".into()));
    }
    let result = func.call(&mut store, &[], &mut []);
    let float =
        compiled.float_flag && instance.get_global(&store, FLOAT_FLAG_EXPORT).is_some_and(|g| matches!(g.get(&store), Val::I32(1)));
    end(store, &cfg, result, float)
}

/// Charges the instructions run since the last host call and maps how the call ended.
fn end(store: Store<Host>, cfg: &Config, result: Result<(), wasmi::Error>, float: bool) -> Exec {
    let now = store.get_fuel().unwrap_or(0);
    let mut host = store.into_data();
    // Whether the last fuel window was the call's whole remaining instruction budget (rather than the gas
    // left): wasmi traps on a block it cannot pay for without consuming the rest, so "fuel left = 0" is
    // not the test.
    let budget_bound = host.fuel_window >= host.fuel_left;
    let used = host.fuel_window.saturating_sub(now);
    host.instructions += used;
    host.fuel_left = host.fuel_left.saturating_sub(used);
    let gas_ok = host.ctx.is_view || host.gas.burn(used.saturating_mul(cfg.regular_op_cost)).is_ok();
    let outcome = match result {
        Ok(()) if !gas_ok => Outcome::Unevaluable("gas: a call exhausted its gas".into()),
        Ok(()) => Outcome::Ok(host.return_data.clone()),
        Err(e) => match host.stop.clone() {
            Some(Stop::Host(msg)) => Outcome::Failed(format!("HostError({msg})")),
            Some(Stop::Unevaluable(r)) => Outcome::Unevaluable(r),
            None if float => Outcome::Unevaluable("float: the run executed a float instruction".into()),
            None => match e.as_trap_code() {
                Some(wasmi::TrapCode::OutOfFuel) if budget_bound => {
                    Outcome::Unevaluable("instructions: a call reached its instruction budget".into())
                }
                Some(wasmi::TrapCode::OutOfFuel) => Outcome::Unevaluable("gas: a call exhausted its gas".into()),
                Some(t @ (wasmi::TrapCode::OutOfSystemMemory | wasmi::TrapCode::GrowthOperationLimited)) => {
                    Outcome::Unevaluable(format!("memory: the host could not back the call's memory ({t})"))
                }
                Some(t) => match logic::wasm_trap(t) {
                    Some(name) => Outcome::Failed(format!("WasmTrap({name})")),
                    None => Outcome::Unevaluable(format!("stack: wasmi's bound was reached ({t})")),
                },
                None => Outcome::Unevaluable(format!("engine: {e}")),
            },
        },
    };
    finish(host, outcome)
}

impl Exec {
    /// A NEAR failure becomes `Unevaluable` with this reason.
    fn unjudged(mut self, reason: &str) -> Exec {
        if matches!(self.outcome, Outcome::Failed(_)) {
            self.outcome = Outcome::Unevaluable(reason.into());
        }
        self
    }
}

fn empty(outcome: Outcome) -> Exec {
    Exec {
        outcome,
        logs: vec![],
        receipts: vec![],
        gas_weights: vec![],
        balance: 0,
        storage_usage: 0,
        overlay: BTreeMap::new(),
        burnt_gas: 0,
        used_gas: 0,
        promises_gas: 0,
        instructions: 0,
    }
}

fn finish(host: Host, outcome: Outcome) -> Exec {
    Exec {
        outcome,
        burnt_gas: host.gas.burnt(),
        used_gas: host.gas.used(),
        promises_gas: host.gas.promises(),
        instructions: host.instructions,
        logs: host.logs,
        receipts: host.receipts,
        gas_weights: host.gas_weights,
        balance: host.balance,
        storage_usage: host.storage_usage,
        overlay: host.overlay,
    }
}

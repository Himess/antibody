//! A NEAR program's rules as code (docs/near-targets.md §6): a wasm module that exports
//! `check() -> i64`, the mask of broken slots (bit `b` set: slot `b` is broken), and its memory as
//! `memory`. It imports only these functions from module `antibody` (all integers are i64; strings and
//! outputs are `(len, ptr)` / `ptr` into the module's memory):
//!
//! | Import | Does |
//! |---|---|
//! | `view(account_len, account_ptr, method_len, method_ptr, args_len, args_ptr) -> status` | A read-only call of a target's method. 0: it returned (the value is the result), 1: it failed on NEAR (panic, trap, missing account or code) |
//! | `result_len() -> len`, `result_read(ptr)` | The last view's returned bytes |
//! | `balance(account_len, account_ptr, out_ptr) -> exists`, `locked_balance(...)` | An account's balance, u128 little-endian at `out_ptr` (0 if it does not exist) |
//! | `ft_balance_of(token_len, token_ptr, account_len, account_ptr, out_ptr) -> ok` | A NEP-141 balance through the token's `ft_balance_of` view, u128 at `out_ptr` |
//! | `block_height() -> u64`, `block_timestamp() -> u64` | The current block |
//!
//! An evaluation cannot change any state: views run in NEAR's view mode (writes and promises are
//! refused) on a snapshot. Its instruction budget (`invariants.gas_limit`) covers the module and every
//! view it makes. A view may only run code that was deployed at the anchor: a call into an account that
//! had no code then, or whose code changed since, fails the evaluation (docs/spec.md §5.4: code deployed
//! by the proof never runs during an evaluation).

use std::{collections::BTreeMap, sync::Arc};

use wasmi::{Caller, Engine, Linker, Memory, Store, Val};

use crate::{
    config::Config,
    exec::{self, Compiled, Outcome},
    logic::{Ctx, ReturnData},
    prepare::FLOAT_FLAG_EXPORT,
    state::World,
};

/// What an evaluation reads besides the world.
#[derive(Clone)]
pub struct EvalEnv {
    pub engine: Engine,
    pub cfg: Arc<Config>,
    /// Compiled contracts by code hash.
    pub modules: Arc<BTreeMap<[u8; 32], Arc<Compiled>>>,
    /// The accounts that had code at the anchor, and that code's hash.
    pub anchor_code: Arc<BTreeMap<String, [u8; 32]>>,
    pub height: u64,
    pub timestamp_ns: u64,
    pub epoch_height: u64,
    pub random_seed: [u8; 32],
}

pub struct Eval {
    /// The mask, or why the evaluation failed.
    pub mask: Result<u64, String>,
    pub instructions: u64,
}

struct RulesHost {
    env: EvalEnv,
    world: World,
    memory: Option<Memory>,
    result: Vec<u8>,
    stop: Option<String>,
    limiter: crate::exec::NoQuietGrowth,
}

/// The outcome of a view call: its value, a NEAR failure, or a reason the evaluation cannot go on.
pub enum ViewResult {
    Value(Vec<u8>),
    Failed,
    Stop(String),
}

/// A read-only call of `method` on `account` with at most `fuel` instructions. Returns the result and the
/// instructions it ran.
pub fn view(env: &EvalEnv, world: &World, account: &str, method: &str, args: &[u8], fuel: u64) -> (ViewResult, u64) {
    let Some(anchor) = env.anchor_code.get(account) else {
        return (ViewResult::Stop(format!("an evaluation called {account}, which had no code at the anchor")), 0);
    };
    let Some(acc) = world.accounts.get(account) else {
        return (ViewResult::Failed, 0);
    };
    let Some(code) = acc.code else {
        return (ViewResult::Failed, 0);
    };
    if code != *anchor {
        return (ViewResult::Stop(format!("an evaluation called {account}, whose code changed after the anchor")), 0);
    }
    let Some(compiled) = env.modules.get(&code) else {
        return (ViewResult::Stop(format!("the code of {account} was not compiled")), 0);
    };
    let ctx = Ctx {
        current_account_id: account.to_string(),
        signer_account_id: String::new(),
        signer_account_pk: Vec::new(),
        predecessor_account_id: String::new(),
        input: args.to_vec(),
        block_height: env.height,
        block_timestamp: env.timestamp_ns,
        epoch_height: env.epoch_height,
        random_seed: env.random_seed.to_vec(),
        account_balance: acc.amount,
        account_locked_balance: acc.locked,
        storage_usage: acc.storage_usage,
        attached_deposit: 0,
        prepaid_gas: 0,
        promise_results: vec![],
        output_data_receivers: vec![],
        is_view: true,
    };
    let ex = exec::execute(&env.engine, compiled, env.cfg.clone(), ctx, acc.data.clone(), method, fuel);
    let r = match ex.outcome {
        Outcome::Ok(ReturnData::Value(v)) => ViewResult::Value(v),
        Outcome::Ok(_) => ViewResult::Value(vec![]),
        Outcome::Failed(_) => ViewResult::Failed,
        Outcome::Unevaluable(why) => ViewResult::Stop(why),
    };
    (r, ex.instructions)
}

/// `ft_balance_of(account)` on a NEP-141 contract, parsed from its JSON string result.
pub fn ft_balance_of(env: &EvalEnv, world: &World, token: &str, account: &str, fuel: u64) -> (Result<Option<u128>, String>, u64) {
    let args = serde_json::to_vec(&serde_json::json!({ "account_id": account })).expect("json");
    let (r, used) = view(env, world, token, "ft_balance_of", &args, fuel);
    let v = match r {
        ViewResult::Value(v) => Ok(serde_json::from_slice::<String>(&v).ok().and_then(|s| crate::util::parse_u128(&s, "balance").ok())),
        ViewResult::Failed => Ok(None),
        ViewResult::Stop(why) => Err(why),
    };
    (v, used)
}

fn read(caller: &Caller<'_, RulesHost>, ptr: u64, len: u64) -> Option<Vec<u8>> {
    let mem = caller.data().memory?;
    let data = mem.data(caller);
    let end = ptr.checked_add(len)?;
    data.get(ptr as usize..usize::try_from(end).ok()?).map(|s| s.to_vec())
}

fn write(caller: &mut Caller<'_, RulesHost>, ptr: u64, bytes: &[u8]) -> Option<()> {
    let mem = caller.data().memory?;
    let data = mem.data_mut(&mut *caller);
    let end = ptr.checked_add(bytes.len() as u64)?;
    data.get_mut(ptr as usize..usize::try_from(end).ok()?)?.copy_from_slice(bytes);
    Some(())
}

fn halt(caller: &mut Caller<'_, RulesHost>, why: impl Into<String>) -> wasmi::Error {
    caller.data_mut().stop = Some(why.into());
    wasmi::Error::new("v2: the evaluation stopped")
}

fn read_str(caller: &mut Caller<'_, RulesHost>, ptr: u64, len: u64) -> Result<String, wasmi::Error> {
    match read(caller, ptr, len).and_then(|b| String::from_utf8(b).ok()) {
        Some(s) => Ok(s),
        None => Err(halt(caller, "the rules passed a bad string")),
    }
}

/// Runs a nested view from a rules host function, drawing on the evaluation's fuel.
fn nested_view(caller: &mut Caller<'_, RulesHost>, account: &str, method: &str, args: &[u8]) -> Result<ViewResult, wasmi::Error> {
    let fuel = caller.get_fuel()?;
    let (r, used) = {
        let h = caller.data();
        view(&h.env, &h.world, account, method, args, fuel)
    };
    caller.set_fuel(fuel.saturating_sub(used))?;
    Ok(r)
}

fn link(l: &mut Linker<RulesHost>) -> Result<(), String> {
    let e = |e: wasmi::errors::LinkerError| e.to_string();
    l.func_wrap("antibody", "view", |mut c: Caller<'_, RulesHost>, al: u64, ap: u64, ml: u64, mp: u64, gl: u64, gp: u64| -> Result<u64, wasmi::Error> {
        let account = read_str(&mut c, ap, al)?;
        let method = read_str(&mut c, mp, ml)?;
        let Some(args) = read(&c, gp, gl) else { return Err(halt(&mut c, "the rules passed bad view arguments")) };
        match nested_view(&mut c, &account, &method, &args)? {
            ViewResult::Value(v) => {
                c.data_mut().result = v;
                Ok(0)
            }
            ViewResult::Failed => {
                c.data_mut().result.clear();
                Ok(1)
            }
            ViewResult::Stop(why) => Err(halt(&mut c, why)),
        }
    })
    .map_err(e)?;
    l.func_wrap("antibody", "result_len", |c: Caller<'_, RulesHost>| -> u64 { c.data().result.len() as u64 }).map_err(e)?;
    l.func_wrap("antibody", "result_read", |mut c: Caller<'_, RulesHost>, ptr: u64| -> Result<(), wasmi::Error> {
        let r = c.data().result.clone();
        write(&mut c, ptr, &r).ok_or_else(|| halt(&mut c, "the rules read the result out of bounds"))
    })
    .map_err(e)?;
    for (name, locked) in [("balance", false), ("locked_balance", true)] {
        l.func_wrap("antibody", name, move |mut c: Caller<'_, RulesHost>, al: u64, ap: u64, out: u64| -> Result<u64, wasmi::Error> {
            let account = read_str(&mut c, ap, al)?;
            let (v, exists) = match c.data().world.accounts.get(&account) {
                Some(a) => (if locked { a.locked } else { a.amount }, 1),
                None => (0, 0),
            };
            write(&mut c, out, &v.to_le_bytes()).ok_or_else(|| halt(&mut c, "the rules wrote a balance out of bounds"))?;
            Ok(exists)
        })
        .map_err(e)?;
    }
    l.func_wrap("antibody", "ft_balance_of", |mut c: Caller<'_, RulesHost>, tl: u64, tp: u64, al: u64, ap: u64, out: u64| -> Result<u64, wasmi::Error> {
        let token = read_str(&mut c, tp, tl)?;
        let account = read_str(&mut c, ap, al)?;
        let fuel = c.get_fuel()?;
        let (r, used) = {
            let h = c.data();
            ft_balance_of(&h.env, &h.world, &token, &account, fuel)
        };
        c.set_fuel(fuel.saturating_sub(used))?;
        let (v, ok) = match r {
            Ok(Some(v)) => (v, 1),
            Ok(None) => (0, 0),
            Err(why) => return Err(halt(&mut c, why)),
        };
        write(&mut c, out, &v.to_le_bytes()).ok_or_else(|| halt(&mut c, "the rules wrote a balance out of bounds"))?;
        Ok(ok)
    })
    .map_err(e)?;
    l.func_wrap("antibody", "block_height", |c: Caller<'_, RulesHost>| -> u64 { c.data().env.height }).map_err(e)?;
    l.func_wrap("antibody", "block_timestamp", |c: Caller<'_, RulesHost>| -> u64 { c.data().env.timestamp_ns }).map_err(e)?;
    Ok(())
}

/// One evaluation of the rules over `world`, with at most `budget` instructions in all.
pub fn evaluate(env: &EvalEnv, rules: &Compiled, world: &World, budget: u64) -> Eval {
    let host = RulesHost { env: env.clone(), world: world.clone(), memory: None, result: vec![], stop: None, limiter: crate::exec::NoQuietGrowth };
    let mut store = Store::new(&env.engine, host);
    store.limiter(|h| &mut h.limiter);
    let fail = |why: String| Eval { mask: Err(why), instructions: 0 };
    if store.set_fuel(budget).is_err() {
        return fail("fuel metering is off".into());
    }
    let mut linker = <Linker<RulesHost>>::new(&env.engine);
    if let Err(e) = link(&mut linker) {
        return fail(format!("link: {e}"));
    }
    let instance = match linker.instantiate_and_start(&mut store, &rules.module) {
        Ok(i) => i,
        Err(e) => return fail(format!("the rules module does not instantiate: {e}")),
    };
    store.data_mut().memory = instance.get_memory(&store, "memory");
    if store.data().memory.is_none() {
        return fail("the rules module exports no memory".into());
    }
    let check = match instance.get_typed_func::<(), i64>(&store, "check") {
        Ok(f) => f,
        Err(_) => return fail("the rules module exports no check() -> i64".into()),
    };
    let r = check.call(&mut store, ());
    let left = store.get_fuel().unwrap_or(0);
    let instructions = budget.saturating_sub(left);
    let float = rules.float_flag && instance.get_global(&store, FLOAT_FLAG_EXPORT).is_some_and(|g| matches!(g.get(&store), Val::I32(1)));
    let mask = match r {
        Ok(m) => Ok(m as u64),
        Err(e) => Err(match store.data().stop.clone() {
            Some(why) => why,
            None if float => "float: the rules executed a float instruction".into(),
            None => match e.as_trap_code() {
                Some(wasmi::TrapCode::OutOfFuel) => "the evaluation ran out of its instruction budget".into(),
                Some(t) => format!("the rules trapped ({t})"),
                None => format!("the rules failed: {e}"),
            },
        }),
    };
    Eval { mask, instructions }
}

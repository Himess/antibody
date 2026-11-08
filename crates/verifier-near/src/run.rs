//! A NEAR program run, end to end (docs/near-targets.md §3, §5, §6, §7):
//!
//! 1. the manifest and every module it commits to are checked (`ProgramMisconfigured` otherwise);
//! 2. fork-mode activation builds the anchor state: accounts, contracts, `init` calls, `setup` calls;
//! 3. the rules must hold there (`ProgramMisconfigured` otherwise);
//! 4. the bundle's calls run one after the other, each call's whole receipt tree first, and the rules are
//!    evaluated after each one;
//! 5. the bundle runs a second time from the same anchor with every wasm and host cost doubled; any
//!    difference in an observable makes the run `Unevaluable`;
//! 6. broken slots get their root cause (the call after which they last flipped), and the impact is
//!    `min(protocol loss, attacker gain)` at the committed prices.

use std::{collections::BTreeMap, sync::Arc};

use crate::{
    config::{Config, PROTOCOL_VERSION},
    exec::{self, Compiled, Vm},
    logic::Action,
    manifest::{Manifest, MAX_PROMISE_DEPTH, MAX_RECEIPTS_PER_RUN},
    prepare::Kind as ModuleKind,
    receipt::*,
    rules::{self, EvalEnv},
    runtime::{Block, Halt, ReceiptRecord, Runtime},
    state::{implicit_account, Account, World},
    util::{fmt_usd, hex32, is_valid_account_id, mul_div, parse_u128, sha256, signed, unb64, usd_to_micro},
    witness::{sender_account, signer_pk, Bundle},
};

/// One run's inputs.
pub struct Input<'a> {
    pub manifest_json: &'a [u8],
    /// The wasm modules the manifest commits to (by sha256), in any order.
    pub codes: &'a [Vec<u8>],
    /// `None`: activation only (the program is checked at the anchor).
    pub bundle: Option<&'a Bundle>,
    pub program: &'a str,
    pub commit_id: u64,
    pub witness_sha256: [u8; 32],
    /// The manifest hash committed on NEAR, when known.
    pub expected_manifest_hash: Option<[u8; 32]>,
}

fn verifier_info() -> VerifierInfo {
    VerifierInfo { name: "antibody-verifier-v2", version: env!("CARGO_PKG_VERSION"), engine: "wasmi 2.0.0", near_protocol: PROTOCOL_VERSION }
}

struct Stop(Kind, String);

fn mis(r: impl Into<String>) -> Stop {
    Stop(Kind::ProgramMisconfigured, r.into())
}
fn invalid(r: impl Into<String>) -> Stop {
    Stop(Kind::InvalidPoC, r.into())
}

/// A receipt with nothing run yet.
fn skeleton(program: &str, commit_id: u64, manifest_json: &[u8], witness_sha256: [u8; 32], bundle: Option<&Bundle>) -> NearReceipt {
    NearReceipt {
        schema: RECEIPT_SCHEMA,
        verifier: verifier_info(),
        program: program.to_string(),
        commit_id,
        manifest_hash: format!("0x{}", hex::encode(sha256(manifest_json))),
        rules_code_hash: String::new(),
        anchor: None,
        witness_sha256: hex::encode(witness_sha256),
        poc_hash: bundle.map(|b| hex::encode(b.poc_hash())).unwrap_or_default(),
        kind: Kind::NoViolation,
        reason: None,
        checks: None,
        senders: vec![],
        endowment_yocto_per_sender: bundle.map(|b| b.endowment_yocto.clone()).unwrap_or_else(|| "0".into()),
        calls: vec![],
        created_accounts: vec![],
        total_gas_used: 0,
        evaluations: 0,
        broken_mask: "0x0".into(),
        broken: vec![],
        impact: None,
        near: NearSection { protocol: PROTOCOL_VERSION, block: None, instructions: 0, receipts: vec![] },
    }
}

/// A receipt for inputs rejected before any run (a witness that does not decode, say).
pub fn failure_receipt(program: &str, commit_id: u64, witness_sha256: [u8; 32], kind: Kind, reason: String) -> NearReceipt {
    let mut r = skeleton(program, commit_id, b"", witness_sha256, None);
    r.manifest_hash = String::new();
    r.kind = kind;
    r.reason = Some(reason);
    r
}

/// Runs a program and always returns a receipt with a typed verdict.
pub fn run(input: &Input) -> NearReceipt {
    let mut r = skeleton(input.program, input.commit_id, input.manifest_json, input.witness_sha256, input.bundle);
    if let Err(Stop(kind, reason)) = run_into(input, &mut r) {
        r.kind = kind;
        r.reason = Some(reason);
        r.broken.clear();
        r.broken_mask = "0x0".into();
        r.impact = None;
    }
    r
}

/// The program as deployed: the runtime at the anchor, the compiled rules, the evaluation environment.
pub struct Activated {
    pub manifest: Manifest,
    pub runtime: Runtime,
    pub rules: Compiled,
    pub env: EvalEnv,
    pub anchor_records: usize,
}

/// Steps 1 to 3: checks the program and builds its anchor state. Errors are the program's.
fn activate(input: &Input, r: &mut NearReceipt) -> Result<Activated, Stop> {
    let manifest_hash = sha256(input.manifest_json);
    if let Some(expected) = input.expected_manifest_hash {
        if expected != manifest_hash {
            return Err(invalid("the manifest bytes do not match the committed manifest hash"));
        }
    }
    let m = Manifest::parse(input.manifest_json).map_err(mis)?;
    let near = m.near().clone();
    r.anchor = Some(Anchor {
        chain_id: m.anchor.chain_id,
        block_number: m.anchor.block_number,
        block_hash: m.anchor.block_hash.clone(),
        timestamp_ns: near.block.timestamp_ns,
        evm_spec: m.anchor.evm_spec.clone(),
    });
    r.near.block = Some(NearBlockInfo {
        height: near.block.height,
        timestamp_ns: near.block.timestamp_ns,
        epoch_height: near.block.epoch_height,
        random_seed: near.block.random_seed.clone(),
        gas_price: near.block.gas_price.clone(),
    });
    let cfg = Arc::new(Config::protocol_74());
    let codes: BTreeMap<String, Arc<Vec<u8>>> = input.codes.iter().map(|c| (hex::encode(sha256(c)), Arc::new(c.clone()))).collect();

    let mut vm = Vm::default();
    let mut world = World::default();
    for a in &near.accounts {
        let amount = parse_u128(&a.amount, "amount").map_err(mis)?;
        let locked = parse_u128(&a.locked, "locked").map_err(mis)?;
        world.accounts.insert(a.account.clone(), Account::new(amount, locked));
    }
    for c in &near.contracts {
        let code = codes.get(&c.wasm.sha256).ok_or_else(|| mis(format!("the wasm of {} is not in the witness", c.account)))?;
        let h = sha256(code);
        vm.contract(h, code, &cfg).map_err(|e| mis(format!("contract {} is refused: {e}", c.account)))?;
        let acc = world.accounts.get_mut(&c.account).expect("checked by the manifest");
        acc.storage_usage += code.len() as u64;
        acc.code = Some(h);
        world.codes.insert(h, code.clone());
    }
    let rules_code = codes.get(&near.rules.wasm.sha256).ok_or_else(|| mis("the rules module is not in the witness"))?;
    r.rules_code_hash = near.rules.wasm.sha256.clone();
    let rules = exec::compile(&vm.engine, rules_code, ModuleKind::Rules, &cfg).map_err(|e| mis(format!("the rules module is refused: {e}")))?;

    let seed = hex32(&near.block.random_seed).expect("checked by the manifest");
    let block = Block { height: near.block.height, timestamp_ns: near.block.timestamp_ns, epoch_height: near.block.epoch_height, random_seed: seed };
    let mut rt = Runtime::new(vm, cfg.clone(), world, block);
    rt.max_receipts = MAX_RECEIPTS_PER_RUN as u64;
    rt.max_depth = MAX_PROMISE_DEPTH;
    rt.fuel_per_call = near.capabilities.max_instructions_per_call;
    let activation = |rt: &mut Runtime, signer: &str, receiver: &str, method: &str, args: &str, deposit: &str, gas: u64, phase: &str, i: u32| -> Result<(), Stop> {
        let args = unb64(args).map_err(mis)?;
        let deposit = parse_u128(deposit, "deposit").map_err(mis)?;
        let action = Action::FunctionCall { method: method.into(), args, deposit, gas, weight: 0 };
        let first = rt.transaction(signer, signer_pk(signer), receiver, vec![action], phase, i).map_err(|h| match h {
            Halt::Unevaluable(why) | Halt::Bound(why) => mis(format!("{phase} call {i}: {why}")),
        })?;
        match &rt.records[first] {
            rec if rec.success => Ok(()),
            rec => Err(mis(format!("{phase} call {i} failed: {}", rec.error.clone().unwrap_or_default()))),
        }
    };
    let mut i = 0u32;
    for c in &near.contracts {
        if let Some(init) = &c.init {
            activation(&mut rt, &c.account, &c.account, &init.method, &init.args, &init.deposit, init.gas, "init", i)?;
            i += 1;
        }
    }
    for (i, s) in near.setup.iter().enumerate() {
        activation(&mut rt, &s.signer, &s.receiver, &s.method, &s.args, &s.deposit, s.gas, "setup", i as u32)?;
    }
    let anchor_code: BTreeMap<String, [u8; 32]> = rt.world.accounts.iter().filter_map(|(a, acc)| acc.code.map(|h| (a.clone(), h))).collect();
    let (height, timestamp_ns) = rt.now();
    let env = EvalEnv {
        engine: rt.vm.engine.clone(),
        cfg,
        modules: Arc::new(rt.vm.compiled()),
        anchor_code: Arc::new(anchor_code),
        height,
        timestamp_ns,
        epoch_height: near.block.epoch_height,
        random_seed: seed,
    };
    let anchor_records = rt.records.len();
    let ev = rules::evaluate(&env, &rules, &rt.world, m.invariants.gas_limit);
    match ev.mask {
        Err(why) => return Err(mis(format!("the rules do not evaluate at the anchor: {why}"))),
        Ok(mask) if mask & m.declared() != 0 => return Err(mis(format!("the rules do not hold at the anchor (mask {:#x})", mask & m.declared()))),
        Ok(_) => {}
    }
    Ok(Activated { manifest: m, runtime: rt, rules, env, anchor_records })
}

/// What one pass of the bundle produced.
#[derive(PartialEq, Eq)]
struct BundleRun {
    /// The mask after each call (`None`: that evaluation failed and is skipped, as v1 skips it).
    points: Vec<Option<u64>>,
    calls: Vec<CallOutcome>,
}

/// Step 4 (and 5, with the doubled schedule): the bundle's calls and the evaluations after each.
fn run_bundle(rt: &mut Runtime, a: &Activated, bundle: &Bundle, senders: &[String]) -> Result<BundleRun, Stop> {
    let caps = &a.manifest.near().capabilities;
    let declared = a.manifest.declared();
    let mut points = Vec::new();
    let mut calls = Vec::new();
    let mut eval_total = 0u64;
    for (i, c) in bundle.calls.iter().enumerate() {
        let sender = &senders[c.signer as usize];
        let args = unb64(&c.args).map_err(|_| invalid(format!("call {i}: args are not base64")))?;
        let deposit = parse_u128(&c.deposit, "deposit").map_err(|e| invalid(format!("call {i}: {e}")))?;
        let action = Action::FunctionCall { method: c.method.clone(), args, deposit, gas: c.gas, weight: 0 };
        let first = rt.transaction(sender, signer_pk(sender), &c.receiver, vec![action], "call", i as u32).map_err(|h| match h {
            Halt::Unevaluable(why) => Stop(Kind::Unevaluable, format!("call {i}: {why}")),
            Halt::Bound(why) => invalid(format!("call {i}: {why}")),
        })?;
        let tree = &rt.records[first..];
        calls.push(CallOutcome {
            index: i,
            sender: sender.clone(),
            receiver: c.receiver.clone(),
            method: c.method.clone(),
            success: tree[0].success,
            receipts: tree.len(),
            gas_burnt: tree.iter().map(|t| t.gas_burnt).sum(),
        });
        let last = i + 1 == bundle.calls.len();
        let (height, timestamp_ns) = rt.now();
        let env = EvalEnv { height, timestamp_ns, modules: Arc::new(rt.vm.compiled()), ..a.env.clone() };
        let ev = rules::evaluate(&env, &a.rules, &rt.world, a.manifest.invariants.gas_limit);
        if !last {
            // The final evaluation has its own budget, outside the total (docs/spec.md §3.3, §5.5).
            eval_total = eval_total.saturating_add(ev.instructions);
            if eval_total > caps.max_eval_instructions {
                return Err(invalid(format!("the evaluations ran more than max_eval_instructions ({})", caps.max_eval_instructions)));
            }
        }
        match ev.mask {
            Ok(m) => points.push(Some(m & declared)),
            Err(why) if last => return Err(Stop(Kind::Unevaluable, format!("the rules failed after the proof: {why}"))),
            Err(_) => points.push(None),
        }
    }
    Ok(BundleRun { points, calls })
}

/// The bundle's checks against the committed capabilities: the hunter can compute each one locally.
fn check_bundle(m: &Manifest, b: &Bundle) -> Result<(u128, Vec<String>), Stop> {
    let caps = &m.near().capabilities;
    let max_endowment = parse_u128(&caps.max_endowment_yocto, "max_endowment_yocto").map_err(mis)?;
    let salt = b.salt().ok_or_else(|| invalid("the sender salt is not 32 bytes of hex"))?;
    if b.senders == 0 || b.calls.is_empty() {
        return Err(invalid("empty bundle"));
    }
    if b.senders > caps.max_senders {
        return Err(invalid(format!("{} senders > cap {}", b.senders, caps.max_senders)));
    }
    let endowment = parse_u128(&b.endowment_yocto, "endowment").map_err(invalid)?;
    if endowment > max_endowment {
        return Err(invalid("endowment above the committed cap"));
    }
    if b.calls.len() as u64 > caps.max_calls as u64 {
        return Err(invalid(format!("{} calls > cap {}", b.calls.len(), caps.max_calls)));
    }
    for (i, c) in b.calls.iter().enumerate() {
        if c.signer >= b.senders {
            return Err(invalid(format!("call {i}: bad sender index")));
        }
        if !is_valid_account_id(&c.receiver) {
            return Err(invalid(format!("call {i}: the receiver is not a valid account id")));
        }
        if c.method.is_empty() || c.method.len() > 256 {
            return Err(invalid(format!("call {i}: bad method name")));
        }
        if c.gas == 0 || c.gas > crate::config::Limits::protocol_74().max_total_prepaid_gas {
            return Err(invalid(format!("call {i}: gas must be between 1 and 300 Tgas")));
        }
        parse_u128(&c.deposit, "deposit").map_err(|e| invalid(format!("call {i}: {e}")))?;
        unb64(&c.args).map_err(|_| invalid(format!("call {i}: args are not base64")))?;
    }
    Ok((endowment, (0..b.senders).map(|i| sender_account(&salt, i)).collect()))
}

/// The bundle part of the records with gas left out: what the doubled run must reproduce.
fn observable(records: &[ReceiptRecord]) -> Vec<ReceiptRecord> {
    records.iter().cloned().map(|mut r| {
        r.gas_burnt = 0;
        r
    }).collect()
}

fn run_into(input: &Input, r: &mut NearReceipt) -> Result<(), Stop> {
    let a = activate(input, r)?;
    let mut rt = a.runtime.fork(a.runtime.cfg.clone());
    r.checks = Some(Checks { target_code_hashes_match: true, anchor_rules_hold: true, doubled_gas_run_identical: false });
    let Some(bundle) = input.bundle else {
        r.near.receipts = rt.records.clone();
        return Ok(());
    };
    let (endowment, senders) = check_bundle(&a.manifest, bundle)?;
    for s in &senders {
        if rt.world.accounts.contains_key(s) {
            return Err(invalid("a sender collides with an existing account"));
        }
        rt.world.accounts.insert(s.clone(), implicit_account(s, endowment));
    }
    r.senders = senders.clone();
    let caps = a.manifest.near().capabilities.clone();
    rt.max_receipts = caps.max_receipts as u64;
    rt.receipts_executed = 0;
    rt.fuel_total = 0;
    rt.fuel_total_limit = Some(caps.max_total_instructions);
    let before = rt.world.clone();
    let created_before = rt.created_accounts.len();
    let mut doubled = rt.fork(Arc::new(rt.cfg.doubled()));
    doubled.margin_check = false;

    let base = run_bundle(&mut rt, &a, bundle, &senders);
    r.near.receipts = rt.records.clone();
    r.near.instructions = rt.fuel_total;
    let base = base?;
    r.calls = base.calls.clone();
    r.total_gas_used = base.calls.iter().map(|c| c.gas_burnt).sum();
    r.created_accounts = rt.created_accounts[created_before..].to_vec();
    r.evaluations = base.points.iter().filter(|p| p.is_some()).count() as u32;

    // Step 5: the same bundle with every wasm and host cost doubled.
    let second = run_bundle(&mut doubled, &a, bundle, &senders);
    let same = match &second {
        Ok(d) => {
            d.points == base.points
                && observable(&doubled.records[a.anchor_records..]) == observable(&rt.records[a.anchor_records..])
                && doubled.world == rt.world
        }
        Err(_) => false,
    };
    if !same {
        let why = match second {
            Err(Stop(_, why)) => format!("gas: the doubled-cost run stopped where the first did not ({why})"),
            Ok(_) => "gas: the doubled-cost run differs on an observable (outcomes, values, logs, storage, balances or rules)".into(),
        };
        return Err(Stop(Kind::Unevaluable, why));
    }
    if let Some(c) = r.checks.as_mut() {
        c.doubled_gas_run_identical = true;
    }

    // Step 6: root causes and impact.
    let final_mask = base.points.last().copied().flatten().unwrap_or(0);
    let mut seq: Vec<(u64, Option<usize>)> = vec![(0, None)];
    seq.extend(base.points.iter().enumerate().filter_map(|(i, p)| p.map(|m| (m, Some(i)))));
    for bit in 0..64u8 {
        if final_mask & (1u64 << bit) == 0 {
            continue;
        }
        let flip = (1..seq.len()).rev().find(|&j| seq[j].0 & (1u64 << bit) != 0 && seq[j - 1].0 & (1u64 << bit) == 0);
        let call = flip.and_then(|j| seq[j].1);
        let (receiver, method) = call.map(|i| (bundle.calls[i].receiver.clone(), bundle.calls[i].method.clone())).unwrap_or_default();
        let slot = a.manifest.slot(bit);
        r.broken.push(Fingerprint {
            slot: bit,
            name: slot.map(|s| s.name.clone()).unwrap_or_else(|| format!("bit{bit}")),
            severity: slot.map(|s| s.severity).unwrap_or(crate::manifest::Severity::Low),
            group: a.manifest.group_of(&receiver, &method),
            receiver,
            method,
            flip_point: flip.map(|j| (j - 1) as u32),
            call_index: call.map(|i| i as u32),
        });
    }
    r.broken_mask = format!("{final_mask:#x}");
    r.kind = if final_mask == 0 { Kind::NoViolation } else { Kind::Violation };
    let (height, timestamp_ns) = rt.now();
    let env = EvalEnv { height, timestamp_ns, modules: Arc::new(rt.vm.compiled()), ..a.env.clone() };
    r.impact = Some(impact(&a.manifest, &env, &before, &rt.world, &senders, &rt.created_accounts[created_before..])?);
    Ok(())
}

/// A signed amount: (negative, magnitude).
type Signed = (bool, u128);

fn add(a: Signed, b: Signed) -> Signed {
    match (a.0, b.0) {
        (x, y) if x == y => (x, a.1.saturating_add(b.1)),
        _ if a.1 >= b.1 => (a.0, a.1 - b.1),
        _ => (b.0, b.1 - a.1),
    }
}

fn delta(before: u128, after: u128) -> Signed {
    if after >= before {
        (false, after - before)
    } else {
        (true, before - after)
    }
}

/// `min(protocol loss, attacker gain)` over NEAR balances (gas is never charged to balances in v2) and the
/// listed NEP-141 balances, at the committed prices (docs/near-targets.md §6).
fn impact(m: &Manifest, env: &EvalEnv, before: &World, after: &World, senders: &[String], created: &[String]) -> Result<Impact, Stop> {
    let near = m.near();
    let native = usd_to_micro(&near.impact.native_usd).map_err(mis)?;
    let mut accounts: Vec<(String, &'static str)> = near.impact.protocol_accounts.iter().map(|a| (a.clone(), "protocol")).collect();
    for s in senders.iter().chain(created.iter().filter(|c| senders.iter().any(|s| c.ends_with(&format!(".{s}"))))) {
        accounts.push((s.clone(), "attacker"));
    }
    let total = |w: &World, a: &str| w.accounts.get(a).map(|x| x.amount.saturating_add(x.locked)).unwrap_or(0);
    let fuel = near.capabilities.max_instructions_per_call;
    let token = |w: &World, t: &str, a: &str| -> Result<u128, Stop> {
        match rules::ft_balance_of(env, w, t, a, fuel).0 {
            Ok(v) => Ok(v.unwrap_or(0)),
            Err(why) => Err(Stop(Kind::Unevaluable, format!("impact: {why}"))),
        }
    };
    let (mut protocol, mut attacker): (Signed, Signed) = ((false, 0), (false, 0));
    let mut rows = Vec::new();
    for (account, role) in &accounts {
        let d = delta(total(before, account), total(after, account));
        let mut usd: Signed = (d.0, mul_div(d.1, native, 10u128.pow(24)));
        let mut token_rows = Vec::new();
        for t in &near.impact.tokens {
            let price = usd_to_micro(&t.usd).map_err(mis)?;
            let td = delta(token(before, &t.contract, account)?, token(after, &t.contract, account)?);
            if td.1 != 0 {
                usd = add(usd, (td.0, mul_div(td.1, price, 10u128.pow(t.decimals as u32))));
                token_rows.push((t.symbol.clone(), signed(td.0, td.1)));
            }
        }
        if *role == "protocol" {
            protocol = add(protocol, usd);
        } else {
            attacker = add(attacker, usd);
        }
        if d.1 != 0 || !token_rows.is_empty() {
            rows.push(AccountValue {
                account: account.clone(),
                role: role.to_string(),
                near_delta_yocto: signed(d.0, d.1),
                tokens: token_rows,
                usd_micro: signed(usd.0, usd.1),
            });
        }
    }
    let loss = if protocol.0 { protocol.1 } else { 0 };
    let gain = if attacker.0 { 0 } else { attacker.1 };
    let i = loss.min(gain);
    Ok(Impact {
        protocol_loss_usd_micro: loss.to_string(),
        attacker_gain_usd_micro: gain.to_string(),
        impact_usd_micro: i.to_string(),
        impact_usd: fmt_usd(false, i),
        accounts: rows,
        protected_harm_usd_micro: "0".into(),
    })
}

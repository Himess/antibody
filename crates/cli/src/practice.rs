//! `antibody try` and the practice operator's commands (docs/practice.md). The practice target is a
//! tutorial: its bug and its proof are public; the point is the loop of commit, sealed reveal, verdict
//! and payment.

use std::{path::Path, time::Instant};

use antibody_verifier::{facts::sha256, manifest::Manifest};
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{
    cmd,
    ctx::Ctx,
    near::{Action, Permission, Signer, ONE_YOCTO, TGAS},
};

/// `sweep(0x…dEaD)`: the tutorial proof's one call (`cast calldata "sweep(address)" 0x…dEaD`).
pub const TUTORIAL_CALL: &str = "0x01681a62000000000000000000000000000000000000000000000000000000000000dead";

/// The public vend key's seed. Deliberately public: anyone derives this key, so a first try needs no
/// NEAR account. On a vending contract it may call only `vend`, within the allowance the operator set,
/// and vending is limited per hour.
pub const PUBLIC_VEND_SEED: &str = "antibody/practice/public-vend-key/v1";

/// The public vend key, signing as the vending account.
pub fn public_vend_signer(vending: &str) -> Signer {
    Signer::from_seed(vending.to_string(), sha256(PUBLIC_VEND_SEED.as_bytes()))
}

/// The vending contract's terms, in its Borsh layout (amounts in yoctoNEAR and raw token units).
#[derive(Clone, Debug)]
pub struct Terms {
    pub gas_funds: u128,
    pub allowance: u128,
    pub bond_budget: u128,
    pub per_commit_cap: u128,
    pub per_hour: u32,
}

impl Terms {
    fn borsh(&self, o: &mut Vec<u8>) {
        for x in [self.gas_funds, self.allowance, self.bond_budget, self.per_commit_cap] {
            o.extend(x.to_le_bytes());
        }
        o.extend(self.per_hour.to_le_bytes());
    }
}

fn borsh_str(s: &str) -> Vec<u8> {
    let mut v = (s.len() as u32).to_le_bytes().to_vec();
    v.extend(s.as_bytes());
    v
}

/// The vending contract's `new` arguments, in Borsh.
pub fn new_args(owner: &str, antibody: &str, token: &str, program: u32, vaults: u32, terms: &Terms) -> Vec<u8> {
    let mut v = borsh_str(owner);
    v.extend(borsh_str(antibody));
    v.extend(borsh_str(token));
    v.extend(program.to_le_bytes());
    v.extend(vaults.to_le_bytes());
    terms.borsh(&mut v);
    v
}

/// The vending contract's `vend(name, public_key)` arguments, in Borsh (an ed25519 key is tag 0).
pub fn vend_args(name: &str, public_key: &[u8; 32]) -> Vec<u8> {
    let mut v = borsh_str(name);
    v.push(0);
    v.extend(public_key);
    v
}

/// The vault the vending contract assigned to `account`.
pub fn vault_of(ctx: &Ctx, vending: &str, account: &str) -> Result<Option<u32>> {
    let b = ctx.near.view_raw(vending, "vault_of", &borsh_str(account))?;
    Ok(match b.as_slice() {
        [1, a, b2, c, d, ..] => Some(u32::from_le_bytes([*a, *b2, *c, *d])),
        _ => None,
    })
}

/// The tutorial proof against one vault.
pub fn tutorial_bundle(vault: &str) -> Value {
    json!({"senders": 1, "endowment_wei": "1000000000000000", "protected": [],
           "calls": [{"sender": 0, "to": vault, "value": "0", "data": TUTORIAL_CALL, "gas_limit": 100_000}]})
}

/// The address of vault `i` in the practice program's manifest (targets `vault-<i>`).
pub fn vault_address(ctx: &Ctx, program: u32, i: u32) -> Result<String> {
    let m = Manifest::parse(&cmd::program_manifest_at(ctx, program, 0)?).map_err(|e| eyre!(e))?;
    m.targets.iter().find(|t| t.name == format!("vault-{i}")).map(|t| format!("{:#x}", t.address)).ok_or_else(|| eyre!("no vault-{i} in program {program}"))
}

/// Who calls `vend`: your account if you name one, otherwise the public vend key.
fn vend_caller(ctx: &Ctx, vending: &str) -> Result<Signer> {
    if ctx.account.is_some() || ctx.key_file.is_some() || std::env::var("ANTIBODY_KEY_FILE").is_ok() {
        ctx.signer()
    } else {
        Ok(public_vend_signer(vending))
    }
}

/// Vends `<name>.<vending>` for a new key (written to `key_out`).
pub fn vend(ctx: &Ctx, vending: &str, name: &str, key_out: &Path) -> Result<Value> {
    let caller = vend_caller(ctx, vending)?;
    let account = format!("{name}.{vending}");
    let key = Signer::generate(Some(account.clone()))?;
    let out = ctx.near.send(
        &caller,
        vending,
        vec![Action::FunctionCall { method: "vend".into(), args: vend_args(name, &key.public_key()), gas: 100 * TGAS, deposit: 0 }],
        "EXECUTED_OPTIMISTIC",
    )?;
    let s = cmd::ensure_ok(ctx, &out, "vend")?;
    if let Some(dir) = key_out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(key_out, serde_json::to_vec_pretty(&key.to_file_json())?)?;
    let vault = vault_of(ctx, vending, &account)?.ok_or_else(|| eyre!("{vending} assigned no vault to {account}"))?;
    Ok(json!({"account": account, "key_file": key_out, "vault": vault, "vended_by": caller.account_id, "tx": s}))
}

/// Waits until a run is T1-confirmed (or failed).
fn wait_t1(ctx: &Ctx, run_id: u64, timeout_s: u64) -> Result<Value> {
    let t0 = Instant::now();
    loop {
        let v = cmd::run_view(&cmd::run(ctx, run_id)?);
        if v["tier"] == json!("T1") {
            return Ok(v);
        }
        if v.get("failed").is_some() || v["kind"] == json!("InfraError") {
            return Err(eyre!("run {run_id} failed: {v}"));
        }
        if t0.elapsed().as_secs() > timeout_s {
            return Err(eyre!("run {run_id} is not T1-confirmed after {timeout_s} s: {v}"));
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// Settles one commit with the permissionless calls a practice key may make: `allocate` the program's
/// claims, then `pay` this one.
pub fn settle(ctx: &Ctx, program: u32, commit: u64) -> Result<Value> {
    let mut steps = Vec::new();
    match cmd::call(ctx, "allocate", json!({"program_id": program, "max": 20}), 150) {
        Ok(s) => steps.push(json!({"allocate": s["tx"]})),
        Err(e) if e.to_string().contains("nothing to allocate") => {}
        Err(e) => return Err(e),
    }
    if cmd::state_label(&ctx.commit(commit)?["state"]) == "Holder" {
        let s = cmd::call(ctx, "pay", json!({"commit_id": commit}), 60)?;
        steps.push(json!({"pay": s["tx"], "paid": cmd::event(&s, "paid")}));
    }
    Ok(Value::Array(steps))
}

/// The whole try-it: vend a practice account, prove its vault with the tutorial proof (commit, sealed
/// reveal, verdict, T1), and settle as the practice account. With no `--account`, the public vend key
/// calls `vend`, so nothing but this command is needed.
pub fn try_it(ctx: &Ctx, vending: &str, program: u32, name: &str) -> Result<Value> {
    let t0 = Instant::now();
    let dir = ctx.state_dir.join("practice").join(name);
    let key_file = dir.join("key.json");
    let vended = vend(ctx, vending, name, &key_file)?;
    let account = vended["account"].as_str().unwrap_or_default().to_string();
    let vault = vault_address(ctx, program, vended["vault"].as_u64().unwrap_or_default() as u32)?;
    let bundle = dir.join("tutorial.bundle.json");
    std::fs::write(&bundle, serde_json::to_vec_pretty(&tutorial_bundle(&vault))?)?;
    let practice = Ctx {
        near: ctx.near.clone(),
        dep: ctx.dep.clone(),
        account: Some(account.clone()),
        key_file: Some(key_file),
        eth_rpc: ctx.eth_rpc.clone(),
        state_dir: dir.join("state"),
    };
    let submitted = cmd::submit(&practice, program, &bundle, None, None, true)?;
    let commit = submitted["commit"]["commit_id"].as_u64().ok_or_else(|| eyre!("no commit id: {submitted}"))?;
    let run = submitted["reveal"]["run_id"].as_u64().ok_or_else(|| eyre!("no run id: {submitted}"))?;
    let t1 = wait_t1(&practice, run, 300)?;
    let settled = settle(&practice, program, commit)?;
    let c = ctx.commit(commit)?;
    let mut links = vec![json!({"vend": vended["tx"]["explorer"]}), json!({"commit": submitted["commit"]["tx"]["explorer"]}),
                         json!({"reveal": submitted["reveal"]["tx"]["explorer"]})];
    for s in settled.as_array().into_iter().flatten() {
        for (k, v) in s.as_object().into_iter().flatten() {
            if v.get("explorer").is_some() {
                links.push(json!({k: v["explorer"]}));
            }
        }
    }
    Ok(json!({"account": account, "vault": vault, "vault_index": vended["vault"], "vended_by": vended["vended_by"], "commit_id": commit,
              "run_id": run, "verdict": submitted["reveal"]["result"], "t1": t1, "settle": settled,
              "state": cmd::state_label(&c["state"]), "paid_raw": cmd::paid_of(&c).to_string(), "beneficiary": c["beneficiary"],
              "links": links, "seconds": t0.elapsed().as_secs_f64()}))
}

// ------------------------------------------------------------------------- the operator

/// Deploys the vending contract on the signer's own account and adds the public vend key, for `vend`
/// only, within `vend_allowance` yoctoNEAR. The payout token is the deployment's.
pub fn deploy(ctx: &Ctx, wasm: &Path, owner: &str, program: u32, vaults: u32, terms: &Terms, vend_allowance: u128) -> Result<Value> {
    let me = ctx.signer()?;
    let public = public_vend_signer(&me.account_id);
    let out = ctx.near.send(
        &me,
        &me.account_id,
        vec![
            Action::DeployContract(std::fs::read(wasm)?),
            Action::FunctionCall {
                method: "new".into(),
                args: new_args(owner, &ctx.dep.antibody, &ctx.dep.tusd, program, vaults, terms),
                gas: 30 * TGAS,
                deposit: 0,
            },
            Action::AddKey {
                public_key: public.public_key(),
                permission: Permission::FunctionCall { allowance: Some(vend_allowance), receiver_id: me.account_id.clone(), method_names: vec!["vend".into()] },
            },
        ],
        "EXECUTED_OPTIMISTIC",
    )?;
    let s = cmd::ensure_ok(ctx, &out, "deploy")?;
    Ok(json!({"vending": me.account_id, "owner": owner, "program": program, "vaults": vaults,
              "public_vend_key": public.public_key_str(), "vend_allowance": vend_allowance.to_string(), "tx": s}))
}

/// The owner funds the vending contract: NEAR credit at near-antibody for the practice accounts' runs
/// and storage, and `tokens` raw units of the payout token moved into its credit there for their bonds.
pub fn fund(ctx: &Ctx, vending: &str, near_credit: u128, tokens: u128) -> Result<Value> {
    let owner = ctx.signer()?;
    let token = ctx.dep.tusd.clone();
    let mut res = json!({"vending": vending});
    if near_credit > 0 {
        let out = ctx.near.function_call(&owner, &ctx.dep.antibody, "storage_deposit", json!({"account_id": vending}), 30, near_credit)?;
        res["near_credit"] = cmd::ensure_ok(ctx, &out, "storage_deposit")?;
    }
    if tokens > 0 {
        // The token's own storage registration for the vending contract, where it needs one.
        let min = cmd::u128_of(&ctx.near.view(&token, "storage_balance_bounds", json!({}))?["min"]);
        if min > 0 && ctx.near.view(&token, "storage_balance_of", json!({"account_id": vending}))?.is_null() {
            let out = ctx.near.function_call(&owner, &token, "storage_deposit", json!({"account_id": vending, "registration_only": true}), 30, min)?;
            res["token_registration"] = cmd::ensure_ok(ctx, &out, "storage_deposit")?;
        }
        let out = ctx.near.function_call(&owner, &token, "ft_transfer", json!({"receiver_id": vending, "amount": tokens.to_string()}), 30, ONE_YOCTO)?;
        res["tokens_sent"] = cmd::ensure_ok(ctx, &out, "ft_transfer")?;
        let out = ctx.near.send(
            &owner,
            vending,
            vec![Action::FunctionCall { method: "fund".into(), args: tokens.to_le_bytes().to_vec(), gas: 250 * TGAS, deposit: ONE_YOCTO }],
            "EXECUTED_OPTIMISTIC",
        )?;
        res["fund"] = cmd::ensure_ok(ctx, &out, "fund")?;
    }
    res["credit"] = ctx.view("get_account", json!({"account_id": vending}))?;
    Ok(res)
}

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

/// The vending contract's `vend(name, public_key)` arguments, in Borsh. near-sdk's `PublicKey` is a
/// `Vec<u8>`: its length (33), the curve (0, ed25519) and the key.
pub fn vend_args(name: &str, public_key: &[u8; 32]) -> Vec<u8> {
    let mut v = borsh_str(name);
    v.extend(33u32.to_le_bytes());
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

/// A practice account's name, as the vending contract takes it (it also names a local directory).
pub fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 32 || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
        return Err(eyre!("a practice name is 1 to 32 of a-z, 0-9, - and _"));
    }
    Ok(())
}

/// Vends `<name>.<vending>` for a new key (written to `key_out`). The public vend key calls `vend`;
/// once its allowance is spent, your own account does, if you named one.
pub fn vend(ctx: &Ctx, vending: &str, name: &str, key_out: &Path) -> Result<Value> {
    check_name(name)?;
    let account = format!("{name}.{vending}");
    let key = Signer::generate(Some(account.clone()))?;
    let call = vec![Action::FunctionCall { method: "vend".into(), args: vend_args(name, &key.public_key()), gas: 100 * TGAS, deposit: 0 }];
    let public = public_vend_signer(vending);
    let (out, by) = match ctx.near.send(&public, vending, call.clone(), "EXECUTED_OPTIMISTIC") {
        Ok(out) => (out, public.account_id),
        Err(e) if ctx.account.is_some() || ctx.key_file.is_some() => {
            let me = ctx.signer()?;
            eprintln!("the public vend key could not call vend ({e}); calling it as {}", me.account_id);
            (ctx.near.send(&me, vending, call, "EXECUTED_OPTIMISTIC")?, me.account_id)
        }
        Err(e) => return Err(e),
    };
    let s = cmd::ensure_ok(ctx, &out, "vend")?;
    if let Some(dir) = key_out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(key_out, serde_json::to_vec_pretty(&key.to_file_json())?)?;
    let vault = vault_of(ctx, vending, &account)?.ok_or_else(|| eyre!("{vending} assigned no vault to {account}"))?;
    Ok(json!({"account": account, "key_file": key_out, "vault": vault, "vended_by": by, "tx": s}))
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

/// The vending contract and practice program to try: as given, else the deployment's.
pub fn target(ctx: &Ctx, vending: Option<String>, program: Option<u32>) -> Result<(String, u32)> {
    let dep = ctx.dep.practice.as_ref();
    let none = || {
        eyre!(
            "the {} deployment has no practice program yet (the README's Status says when it is); the sandbox has one: \
             `make serve` in the repository, then antibody --network sandbox --rpc http://127.0.0.1:3030 \
             --deployments target/demo/deployments.json try --name <you>",
            ctx.near.network
        )
    };
    let vending = vending.or_else(|| dep.map(|p| p.vending.clone())).ok_or_else(none)?;
    let program = program.or_else(|| dep.map(|p| p.program)).ok_or_else(none)?;
    Ok((vending, program))
}

/// The whole try-it: vend a practice account, prove its vault with the tutorial proof (commit, sealed
/// reveal, verdict, T1), and settle as the practice account. With no `--account`, the public vend key
/// calls `vend`, so nothing but this command is needed.
pub fn try_it(ctx: &Ctx, vending: &str, program: u32, name: &str) -> Result<Value> {
    check_name(name)?;
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
        for k in ["allocate", "pay"] {
            if let Some(h) = s[k].as_str() {
                links.push(json!({k: ctx.near.explorer_tx(h)}));
            }
        }
    }
    Ok(json!({"account": account, "vault": vault, "vault_index": vended["vault"], "vended_by": vended["vended_by"], "commit_id": commit,
              "run_id": run, "verdict": submitted["reveal"]["result"], "witness_source": submitted["commit"]["witness_source"],
              "witness_bytes": submitted["commit"]["witness_bytes"], "t1": t1, "settle": settled,
              "state": cmd::state_label(&c["state"]), "paid_raw": cmd::paid_of(&c).to_string(), "beneficiary": c["beneficiary"],
              "links": links, "seconds": t0.elapsed().as_secs_f64()}))
}

/// `try`'s summary: what happened, in words, with each transaction (`--json` for the record).
pub fn try_summary(r: &Value, decimals: u32, symbol: &str) -> String {
    let paid = r["paid_raw"].as_str().and_then(|p| p.parse::<u128>().ok()).unwrap_or(0);
    let amount = cmd::fmt_tokens(paid, decimals);
    let amount = amount.trim_end_matches('0').trim_end_matches('.');
    let to = r["beneficiary"].as_str().unwrap_or("?");
    let mut out = vec![
        format!("Practice try: {}, vault {} ({})", r["account"].as_str().unwrap_or("?"), r["vault_index"], r["vault"].as_str().unwrap_or("?")),
        format!("  proof: the public tutorial proof, sealed against the program's pinned base witness ({} bytes; no Ethereum RPC)", r["witness_bytes"]),
        format!(
            "  verdict: {}, first answered at T0 (OutLayer's result), then T1 (its TDX quote verified on NEAR)",
            r["t1"]["kind"].as_str().or(r["verdict"]["kind"].as_str()).unwrap_or("?")
        ),
        format!("  state: {}, {amount} {symbol} paid to {to}: the vending contract paid your bond and your run, so the payout returns to it",
                r["state"].as_str().unwrap_or("?")),
        format!("  {:.1} s from vend to payout; transactions:", r["seconds"].as_f64().unwrap_or(0.0)),
    ];
    for l in r["links"].as_array().into_iter().flatten() {
        for (k, v) in l.as_object().into_iter().flatten() {
            out.push(format!("    {k:<9} {}", v.as_str().unwrap_or("")));
        }
    }
    out.push("The full record: add --json".into());
    out.join("\n")
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
                args: new_args(owner, &ctx.dep.antibody, &ctx.dep.token, program, vaults, terms),
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
    let token = ctx.dep.token.clone();
    let mut res = json!({"vending": vending});
    if near_credit > 0 {
        let out = ctx.near.function_call(&owner, &ctx.dep.antibody, "storage_deposit", json!({"account_id": vending}), 30, near_credit)?;
        res["near_credit"] = cmd::ensure_ok(ctx, &out, "storage_deposit")?;
    }
    if tokens > 0 {
        // The token's own storage registration for the vending contract, where it needs one.
        if let Some(r) = cmd::ensure_token_registered(ctx, &owner, vending)? {
            res["token_registration"] = r;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borsh_args_match_the_contract() {
        let t = Terms { gas_funds: 1, allowance: 2, bond_budget: 3, per_commit_cap: 4, per_hour: 5 };
        let a = new_args("o", "b", "t", 7, 24, &t);
        // three strings (4 + 1 bytes each), two u32, four u128, one u32
        assert_eq!(a.len(), 3 * 5 + 8 + 64 + 4);
        assert_eq!(&a[15..19], &7u32.to_le_bytes());
        let v = vend_args("ann", &[9; 32]);
        assert_eq!(&v[..11], &[3, 0, 0, 0, b'a', b'n', b'n', 33, 0, 0, 0]);
        assert_eq!((v[11], v.len()), (0, 11 + 1 + 32));
    }

    #[test]
    fn practice_names_are_plain() {
        assert!(check_name("ann-1_x").is_ok());
        for bad in ["", "Ann", "../x", "a.b", &"a".repeat(33)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_public_vend_key_is_derived_from_the_published_seed() {
        let k = public_vend_signer("practice.antibody.testnet");
        assert_eq!(k.account_id, "practice.antibody.testnet");
        assert_eq!(k.public_key(), public_vend_signer("other.testnet").public_key());
    }
}

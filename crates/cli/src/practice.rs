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
    near::{Action, Near, Permission, Signer, ONE_YOCTO, TGAS},
    payable,
};

/// `sweep(0x…dEaD)`: the tutorial proof's one call (`cast calldata "sweep(address)" 0x…dEaD`).
pub const TUTORIAL_CALL: &str = "0x01681a62000000000000000000000000000000000000000000000000000000000000dead";

/// The public vend key's seed. Deliberately public: anyone derives this key, so a first try needs no
/// NEAR account. On a vending contract it may call only `vend` and `reclaim`, within the allowance the
/// operator set, and vending is limited per hour.
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

/// The vending contract's `new` arguments, in Borsh: the practice-account contract last.
pub fn new_args(owner: &str, antibody: &str, token: &str, program: u32, vaults: u32, terms: &Terms, account_code: &[u8]) -> Vec<u8> {
    let mut v = borsh_str(owner);
    v.extend(borsh_str(antibody));
    v.extend(borsh_str(token));
    v.extend(program.to_le_bytes());
    v.extend(vaults.to_le_bytes());
    terms.borsh(&mut v);
    v.extend((account_code.len() as u32).to_le_bytes());
    v.extend(account_code);
    v
}

/// What a practice account's calls attach (near-antibody burns about 11 Tgas on a commit, 15 on an
/// allocation, 11 on a payment, measured on testnet; the account's contract keeps 3 to pass each on).
/// NEAR sets each call's prepaid gas aside up front: the account's gas funds cover the largest, the
/// reveal ([`cmd::RUN_TGAS`]).
pub const ALLOCATE_TGAS: u64 = 40;
pub const PAY_TGAS: u64 = 30;
/// `close` (the account tells the vending contract its try is done) and `reclaim` (the vending contract
/// deletes it).
pub const CLOSE_TGAS: u64 = 15;
pub const RECLAIM_TGAS: u64 = 20;
/// `vend`: the account, its contract and key, and its agent registration.
pub const VEND_TGAS: u64 = 60;

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
    vault_of_near(&ctx.near, vending, account)
}

/// [`vault_of`] with a bare RPC client (the status page has no signer). The vending contract keeps every
/// account it vended, so this answers for a practice account long after it closed.
pub fn vault_of_near(near: &Near, vending: &str, account: &str) -> Result<Option<u32>> {
    let b = near.view_raw(vending, "vault_of", &borsh_str(account))?;
    Ok(match b.as_slice() {
        [1, a, b2, c, d, ..] => Some(u32::from_le_bytes([*a, *b2, *c, *d])),
        _ => None,
    })
}

/// Whether a commit holds (or may still take) its rule key: paid, exhausted, or not settled yet. A
/// forfeited or invalid one leaves the key to the next proof.
pub fn holds_key(state: &Value) -> bool {
    match state.get("Done") {
        None => !state.is_null(),
        Some(done) => done.get("Paid").is_some() || done.get("Exhausted").is_some() || done == "Paid" || done == "Exhausted",
    }
}

/// What a vending contract's program still pays for. Each instance's rule key is paid once per program
/// and round, so an instance a commit already holds (`taken`) gives the next try on it a duplicate, with
/// no payout. `{instances, proved, fresh, next_instance, next_is_fresh}`; a fresh program gives every
/// instance back.
pub fn instances_left(config: &VendingConfig, taken: &std::collections::BTreeSet<u32>) -> Value {
    let proved = taken.iter().filter(|v| **v < config.instances).count() as u32;
    let next = config.next_instance();
    json!({"program": config.program, "instances": config.instances, "vended": config.vended, "per_hour": config.terms.per_hour,
           "proved": proved, "fresh": config.instances.saturating_sub(proved), "next_instance": next, "next_is_fresh": !taken.contains(&next)})
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

/// Whether a send failed because the signing access key itself was refused (its allowance spent, or a key
/// that may not make this call), so that nothing was executed.
pub fn vend_key_refused(err: &str) -> bool {
    ["InvalidAccessKeyError", "NotEnoughAllowance", "AccessKeyNotFound", "MethodNameMismatch", "ReceiverMismatch"].iter().any(|m| err.contains(m))
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
    // A closed practice account is gone, but the vending contract keeps its name.
    if ctx.near.account(&account)?.is_some() || vault_of(ctx, vending, &account)?.is_some() {
        if let Some(resumed) = resume_vend(ctx, vending, &account, key_out)? {
            return Ok(resumed);
        }
        return Err(eyre!("{account} is taken: pick another --name (a new one, lowercase)"));
    }
    crate::progress::step(format!("vending {account} (the practice vending contract pays its bond, run and storage)"));
    let key = Signer::generate(Some(account.clone()))?;
    // The key is kept before the vend is sent: a vend that executes while its answer is lost (a timeout)
    // leaves an account whose key is still here.
    crate::cmd::write_secret_json(key_out, &key.to_file_json())?;
    let call = vec![Action::FunctionCall { method: "vend".into(), args: vend_args(name, &key.public_key()), gas: VEND_TGAS * TGAS, deposit: 0 }];
    let public = public_vend_signer(vending);
    let (out, by) = match ctx.near.send(&public, vending, call.clone(), "EXECUTED_OPTIMISTIC") {
        Ok(out) => (out, public.account_id),
        // Your own account calls `vend` only when the public key itself was refused (its allowance spent):
        // after any other error, a timeout above all, the first vend may have gone out.
        Err(e) if (ctx.account.is_some() || ctx.key_file.is_some()) && vend_key_refused(&format!("{e:#}")) => {
            let me = ctx.signer()?;
            let first = format!("{e:#}");
            let out = ctx
                .near
                .send(&me, vending, call, "EXECUTED_OPTIMISTIC")
                .map_err(|e2| eyre!("the public vend key could not call vend ({first}); calling it as {} failed too: {e2:#}", me.account_id))?;
            (out, me.account_id)
        }
        // The public key refused (its allowance spent): nothing was sent, and the key is of no use.
        Err(e) if vend_key_refused(&format!("{e:#}")) => {
            let _ = std::fs::remove_file(key_out);
            return Err(e);
        }
        Err(e) => return Err(e),
    };
    // A vend that executed and failed (the hour's cap, say) made no account: its key goes too.
    let s = cmd::ensure_ok(ctx, &out, "vend").map_err(|e| {
        let _ = std::fs::remove_file(key_out);
        hourly_cap(&ctx.near, vending, name, e)
    })?;
    let vault = vault_of(ctx, vending, &account)?.ok_or_else(|| eyre!("{vending} assigned no vault to {account}"))?;
    crate::progress::step(format!("vended {account}: instance {vault}"));
    Ok(json!({"account": account, "key_file": key_out, "vault": vault, "vended_by": by, "tx": s}))
}

/// An account an earlier attempt with this name vended, whose key is still here (a vend that executed while
/// its answer was lost, or a try stopped before it committed): the attempt goes on from the vend. Anything
/// else (another key, no key, a closed account) is None, and the name stays taken.
fn resume_vend(ctx: &Ctx, vending: &str, account: &str, key_out: &Path) -> Result<Option<Value>> {
    let Ok(key) = crate::near::Signer::from_file(key_out) else { return Ok(None) };
    if key.account_id != account || ctx.near.account(account)?.is_none() {
        return Ok(None);
    }
    let holds = ctx
        .near
        .call("query", json!({"request_type": "view_access_key", "finality": "optimistic", "account_id": account, "public_key": key.public_key_str()}))
        .is_ok_and(|r| r.get("nonce").is_some());
    if !holds {
        return Ok(None);
    }
    let Some(vault) = vault_of(ctx, vending, account)? else { return Ok(None) };
    crate::progress::step(format!("{account} was vended by an earlier attempt, and its key is here: going on from the vend"));
    Ok(Some(json!({"account": account, "key_file": key_out, "vault": vault, "vended_by": "an earlier attempt with this name", "resumed": true,
                   "tx": {"explorer": null}})))
}

/// The vending contract's hourly cap, said plainly: how many accounts an hour, that nothing was spent, and the
/// two ways on (wait, or vend with your own testnet account). Other errors pass unchanged.
fn hourly_cap(near: &crate::near::Near, vending: &str, name: &str, e: eyre::Report) -> eyre::Report {
    if !format!("{e:#}").contains("the hour's practice accounts are vended") {
        return e;
    }
    let per_hour = vending_config(near, vending).map(|c| c.terms.per_hour.to_string()).unwrap_or_else(|_| "a few".into());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (at, mins) = next_hour(now);
    eyre!(
        "{vending} vends at most {per_hour} practice accounts an hour, and this hour's are taken. Nothing was spent and the name is still free. The count starts again at {at} UTC (in {mins} min): run the same command then, or now with your own testnet account: antibody --account <you>.testnet try --name {name}. The recorded tries are on the site's /moment."
    )
}

/// The next full UTC hour after `now` (the vending contract counts its hours from block time / 3600 s), as
/// "HH:00", and the whole minutes until it (at least 1).
pub fn next_hour(now: u64) -> (String, u64) {
    let next = (now / 3600 + 1) * 3600;
    (format!("{:02}:00", (next / 3600) % 24), (next - now).div_ceil(60).max(1))
}

/// Closes a practice account with the vending contract's `reclaim`, signed by the public vend key (by
/// your own account once its allowance is spent, if you named one): the refunds of that transaction go
/// to the vending contract, which stays. Allowed once the account's try is done, or an hour after its
/// vend. The summary names its signer (`by`).
pub fn reclaim(ctx: &Ctx, vending: &str, account: &str) -> Result<Value> {
    let call = vec![Action::FunctionCall { method: "reclaim".into(), args: borsh_str(account), gas: RECLAIM_TGAS * TGAS, deposit: 0 }];
    let (out, by) = match ctx.near.send(&public_vend_signer(vending), vending, call.clone(), "EXECUTED_OPTIMISTIC") {
        Ok(out) => (out, vending.to_string()),
        Err(e) if ctx.account.is_some() || ctx.key_file.is_some() => {
            let me = ctx.signer()?;
            eprintln!("the public vend key could not call reclaim ({e}); calling it as {}", me.account_id);
            (ctx.near.send(&me, vending, call, "EXECUTED_OPTIMISTIC")?, me.account_id)
        }
        Err(e) => return Err(e),
    };
    let mut s = cmd::ensure_ok(ctx, &out, "reclaim")?;
    s["by"] = json!(by);
    Ok(s)
}

/// Settles one commit with the permissionless calls a practice key may make: `allocate` the program's
/// claims, then `pay` this one.
pub fn settle(ctx: &Ctx, program: u32, commit: u64) -> Result<Value> {
    let mut steps = Vec::new();
    match cmd::call(ctx, "allocate", json!({"program_id": program, "max": 20}), ALLOCATE_TGAS) {
        Ok(s) => {
            steps.push(json!({"allocate": s["tx"]}));
            // Views read a final block, a moment behind the allocation just executed.
            let t0 = Instant::now();
            while cmd::state_label(&ctx.commit(commit)?["state"]) == "Verifying" && t0.elapsed().as_secs() < 20 {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
        Err(e) if e.to_string().contains("nothing to allocate") => {}
        Err(e) => return Err(e),
    }
    if cmd::state_label(&ctx.commit(commit)?["state"]) == "Holder" {
        match cmd::call(ctx, "pay", json!({"commit_id": commit}), PAY_TGAS) {
            Ok(s) => steps.push(json!({"pay": s["tx"], "paid": cmd::event(&s, "paid")})),
            // `pay` is permissionless: the services' keeper may pay the commit between the read above (a final
            // block, a moment behind) and this call, and near-antibody then refuses a second payment. The
            // commit's own state says whether it was paid.
            Err(e) => match settled_elsewhere(ctx, commit)? {
                Some(state) => steps.push(json!({"pay": null, "paid_by": "another caller (the services' keeper)", "state": state})),
                None => return Err(e),
            },
        }
    }
    Ok(Value::Array(steps))
}

/// A commit someone else settled while this try was settling it: its final state once the views catch up
/// (a few seconds), or None when it is still not done.
fn settled_elsewhere(ctx: &Ctx, commit: u64) -> Result<Option<String>> {
    let t0 = Instant::now();
    loop {
        let state = cmd::state_label(&ctx.commit(commit)?["state"]);
        if state.starts_with("Done:") {
            return Ok(Some(state));
        }
        if t0.elapsed().as_secs() >= 15 {
            return Ok(None);
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// The vending contract and practice program to try: as given, else the deployment's.
pub fn target(ctx: &Ctx, vending: Option<String>, program: Option<u32>) -> Result<(String, u32)> {
    let dep = ctx.dep.practice.as_ref();
    let none = || {
        eyre!(
            "the {} deployment has no practice program yet (docs/deployment.md lists where it runs); the sandbox has one: \
             `make serve` in the repository, then antibody --network sandbox --rpc http://127.0.0.1:3030 \
             --deployments target/demo/deployments.json try --name <you>",
            ctx.near.network
        )
    };
    let vending = vending.or_else(|| dep.map(|p| p.vending.clone())).ok_or_else(none)?;
    // The vending contract's own program first: its owner can point it at a new practice program (for
    // instance after a worker image rotation, docs/runbooks/worker-images.md), and a vended account may
    // commit to that program only.
    // Only a vending contract without `get_config` (an older build) leaves the deployments file's program:
    // an RPC error is an error, never "use the deployments file", which may name a program it left.
    let program = match program {
        Some(p) => p,
        None => match vending_program(&ctx.near, &vending) {
            Ok(p) => p,
            Err(e) if format!("{e:#}").contains("MethodNotFound") => dep.map(|p| p.program).ok_or_else(none)?,
            Err(e) => return Err(e.wrap_err(format!("reading the program {vending} vends for"))),
        },
    };
    Ok((vending, program))
}

/// The practice program a vending contract vends for (`get_config`, Borsh: owner, antibody and token
/// account ids, then the program id).
pub fn vending_program(near: &Near, vending: &str) -> Result<u32> {
    let b = near.view_raw(vending, "get_config", b"")?;
    let mut r = Borsh { b: &b, i: 0 };
    for _ in 0..3 {
        r.string()?;
    }
    r.u32()
}

/// A vending contract's configuration (`get_config`): its owner, the contracts it works with, the program
/// it vends for, its instances (the tutorial vaults: an account is assigned one) and terms, and how many
/// accounts it has vended. The next vend assigns instance `vended % instances`.
#[derive(Clone, Debug)]
pub struct VendingConfig {
    pub owner: String,
    pub antibody: String,
    pub token: String,
    pub program: u32,
    pub instances: u32,
    pub terms: Terms,
    pub vended: u32,
}

impl VendingConfig {
    /// `get_config`'s Borsh: three account ids, the program id and the instances, the terms, then the count
    /// of vends.
    pub fn parse(b: &[u8]) -> Result<Self> {
        let mut r = Borsh { b, i: 0 };
        let (owner, antibody, token) = (r.string()?, r.string()?, r.string()?);
        let (program, instances) = (r.u32()?, r.u32()?);
        let terms = Terms { gas_funds: r.u128()?, allowance: r.u128()?, bond_budget: r.u128()?, per_commit_cap: r.u128()?, per_hour: r.u32()? };
        let vended = r.u32()?;
        if r.i != b.len() {
            return Err(eyre!("get_config: {} bytes after the configuration", b.len() - r.i));
        }
        Ok(Self { owner, antibody, token, program, instances, terms, vended })
    }

    /// The instance the next vend assigns.
    pub fn next_instance(&self) -> u32 {
        self.vended.checked_rem(self.instances).unwrap_or(0)
    }
}

/// A vending contract's configuration, read from it.
pub fn vending_config(near: &Near, vending: &str) -> Result<VendingConfig> {
    VendingConfig::parse(&near.view_raw(vending, "get_config", b"")?).map_err(|e| e.wrap_err(format!("{vending} is not a vending contract")))
}

/// Reads Borsh: little-endian integers and length-prefixed strings.
struct Borsh<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Borsh<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.b.get(self.i..self.i.saturating_add(n)).ok_or_else(|| eyre!("get_config: truncated"))?;
        self.i += n;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    fn u128(&mut self) -> Result<u128> {
        Ok(u128::from_le_bytes(self.take(16)?.try_into()?))
    }

    fn string(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        Ok(String::from_utf8(self.take(n)?.to_vec())?)
    }
}

/// The whole try-it: vend a practice account, prove its vault with the tutorial proof (commit, sealed
/// reveal, verdict, then T1 or the end of the challenge window on NEAR's clock), settle as the practice
/// account, and close it (its NEAR back to the vending contract). With no `--account`, the public vend
/// key calls `vend`, so nothing but this command is needed.
pub fn try_it(ctx: &Ctx, vending: &str, program: u32, name: &str) -> Result<Value> {
    check_name(name)?;
    let t0 = Instant::now();
    // `practice/<name>`, whatever the deployments file names: the T2 step and `zk gather` read a try's record
    // there. (Over MCP a try uses the deployment's practice vending contract only, where a hunt account of the
    // same name has this same directory.)
    let dir = ctx.state_dir.join("practice").join(name);
    let key_file = dir.join("key.json");
    let vended = vend(ctx, vending, name, &key_file)?;
    let account = vended["account"].as_str().unwrap_or_default().to_string();
    let vault = vault_address(ctx, program, vended["vault"].as_u64().unwrap_or_default() as u32)?;
    let bundle = dir.join("tutorial.bundle.json");
    std::fs::write(&bundle, serde_json::to_vec_pretty(&tutorial_bundle(&vault))?)?;
    // Sealed in the account's own context, so that a failure there closes it too.
    let p = prove(ctx, vending, program, &account, &key_file, &dir, |practice| {
        cmd::seal_proof(practice, program, &bundle, None, None, &account, Some(cmd::MAX_INLINE_BLOB))
    })?;
    let c = ctx.commit(p.commit_id)?;
    let mut links = vec![json!({"vend": vended["tx"]["explorer"]})];
    links.extend(proof_links(ctx, &p));
    let state = cmd::state_label(&c["state"]);
    // The run as the wait last read it: T1-confirmed, or T0 with why.
    let mut t1 = p.ready.run.clone();
    if let Some(note) = &p.ready.note {
        t1["note"] = json!(note);
    }
    let path = p.ready.path.as_str();
    let payout = payout_words(path, Some(p.ready.challenge_s), state == "Done:Paid");
    Ok(json!({"account": account, "vault": vault, "vault_index": vended["vault"], "vended_by": vended["vended_by"], "commit_id": p.commit_id,
              "run_id": p.run_id, "verdict": p.reveal["result"], "witness_source": p.commit["witness_source"],
              "witness_bytes": p.commit["witness_bytes"], "t1": t1, "payout_path": path, "payout": payout,
              "challenge_window_s": p.ready.challenge_s, "settle": p.settled, "state": state, "paid_raw": cmd::paid_of(&c).to_string(),
              "beneficiary": c["beneficiary"], "closed": ctx.near.account(&account)?.is_none(), "reclaimed_by": p.reclaim["by"], "links": links,
              "seconds": t0.elapsed().as_secs_f64()}))
}

/// What a vended account's proof came to, from its commit to the account's close.
pub struct Proved {
    /// `cmd::commit_sealed`'s summary, and `cmd::reveal`'s (its `result`: the verdict when answered).
    pub commit: Value,
    pub reveal: Value,
    pub commit_id: u64,
    pub run_id: u64,
    /// When the verdict became payable: T1, or a final NEAR block past the challenge window.
    pub ready: payable::Payable,
    pub settled: Value,
    pub close: Value,
    pub reclaim: Value,
}

/// The context a vended account acts in: its own key signs, its contract passes the near-antibody calls
/// on, and its records go to `dir/state`.
pub fn account_ctx(ctx: &Ctx, account: &str, key_file: &Path, dir: &Path) -> Ctx {
    Ctx {
        near: ctx.near.clone(),
        dep: ctx.dep.clone(),
        account: Some(account.to_string()),
        key_file: Some(key_file.to_path_buf()),
        eth_rpc: ctx.eth_rpc.clone(),
        state_dir: dir.join("state"),
        // Its key calls only its own contract, which passes the near-antibody calls on.
        via: Some(account.to_string()),
    }
}

/// Proves through the vended `account` and closes it: `seal` gives the proof sealed for it (it runs in the
/// account's context), then the commit and the sealed reveal, the wait for T1 or the end of the challenge
/// window on NEAR's clock, `allocate` and `pay`, and the close (`close`, then the vending contract's
/// `reclaim`: its NEAR back there). The try is over either way: on an error the account is closed too, and
/// the error says whether it is.
pub fn prove(
    ctx: &Ctx,
    vending: &str,
    program: u32,
    account: &str,
    key_file: &Path,
    dir: &Path,
    seal: impl FnOnce(&Ctx) -> Result<cmd::SealedProof>,
) -> Result<Proved> {
    let practice = account_ctx(ctx, account, key_file, dir);
    // The account says its try is done, and the public vend key has the vending contract delete it.
    let close = || -> Result<(Value, Value)> {
        let out = practice.near.function_call(&practice.signer()?, account, "close", json!({}), CLOSE_TGAS, 0)?;
        let done = cmd::ensure_ok(ctx, &out, "close")?;
        Ok((done, reclaim(ctx, vending, account)?))
    };
    let tried = (|| -> Result<_> {
        crate::progress::step("sealing the proof to the program's key, then committing its hash with the bond");
        let commit = cmd::commit_sealed(&practice, seal(&practice)?)?;
        let commit_id = commit["commit_id"].as_u64().ok_or_else(|| eyre!("no commit id: {commit}"))?;
        crate::progress::step(format!("committed: commit {commit_id}; revealing the sealed proof (the enclave verifies it)"));
        let reveal = cmd::reveal(&practice, program, commit_id, true)?;
        let run_id = reveal["run_id"].as_u64().ok_or_else(|| eyre!("no run id: {reveal}"))?;
        let kind = reveal["result"]["kind"].as_str().or(reveal["result"]["facts"]["kind"].as_str()).unwrap_or("answered").to_string();
        crate::progress::step(format!("verdict: {kind} (run {run_id}); waiting for its attestation on NEAR, or the end of the challenge window"));
        // T1 normally arrives in seconds. When it cannot (a worker image the program does not count yet,
        // docs/runbooks/worker-images.md), the verdict stays T0 and is final once NEAR's clock passes the
        // challenge window: the try settles at whichever comes first.
        let ready = payable::wait_on_near(&practice, program, run_id)?;
        // Only a Violation pays; any other verdict is final too, and settling returns or forfeits the bond.
        crate::progress::step(if kind == "Violation" {
            format!("payable ({}); settling: allocate, then pay", ready.path.as_str())
        } else {
            format!("final ({}): a {kind} pays nothing; settling the bond", ready.path.as_str())
        });
        let settled = settle(&practice, program, commit_id)?;
        Ok((commit, reveal, commit_id, run_id, ready, settled))
    })();
    // The try is over, done or not: the account goes, its NEAR back to the vending contract (one left
    // open, the vending contract's `reclaim` closes an hour after its vend).
    let (commit, reveal, commit_id, run_id, ready, settled) = match tried {
        Ok(t) => t,
        Err(e) => {
            let closed = close().map(|_| format!("{account} is closed, its NEAR back to {vending}")).unwrap_or_else(|c| format!("{account} stays open ({c})"));
            return Err(e.wrap_err(closed));
        }
    };
    crate::progress::step("settled; closing the practice account (its NEAR back to the vending contract)");
    let (close, reclaim) = close()?;
    Ok(Proved { commit, reveal, commit_id, run_id, ready, settled, close, reclaim })
}

/// A link for each transaction of a proof, in order: the commit, the reveal, `allocate` and `pay`, the
/// close and the reclaim.
pub fn proof_links(ctx: &Ctx, p: &Proved) -> Vec<Value> {
    let mut links = vec![json!({"commit": p.commit["tx"]["explorer"]}), json!({"reveal": p.reveal["tx"]["explorer"]})];
    for s in p.settled.as_array().into_iter().flatten() {
        for k in ["allocate", "pay"] {
            if let Some(h) = s[k].as_str() {
                links.push(json!({k: ctx.near.explorer_tx(h)}));
            }
        }
    }
    links.push(json!({"close": p.close["explorer"]}));
    links.push(json!({"reclaim": p.reclaim["explorer"]}));
    links
}

/// One transaction of a summary: its explorer link and its signer (`antibody admin tx <hash> --signer <account>`
/// shows it in full). A vend an earlier attempt made (the try or hunt went on from it) has no link: the line says so.
pub fn tx_line(k: &str, link: &Value, signer: &str) -> String {
    match link.as_str().unwrap_or("") {
        "" => format!("    {k:<9} (made by an earlier attempt with this name)"),
        url => format!("    {k:<9} {url}  (signed by {signer})"),
    }
}

/// The payout's path in words: T1 pays at once, under the instant cap; a T0 verdict waits out the
/// challenge window. `final` while the payout itself waits for an earlier commit's claim.
pub fn payout_words(path: &str, window: Option<u64>, paid: bool) -> String {
    let window = window.map(|w| format!("the {w} s challenge window")).unwrap_or_else(|| "the challenge window".into());
    match (path == "T1", paid) {
        (true, true) => "T1-confirmed: paid at once (an instant payout, under the cap)".into(),
        (true, false) => "T1-confirmed: final at once (under the instant cap)".into(),
        (false, true) => format!("T0, paid after {window}"),
        (false, false) => format!("T0, final after {window}"),
    }
}

/// `try`'s summary: what happened, in words, with each transaction (`--json` for the record).
pub fn try_summary(r: &Value, decimals: u32, symbol: &str) -> String {
    let paid = r["paid_raw"].as_str().and_then(|p| p.parse::<u128>().ok()).unwrap_or(0);
    let amount = cmd::fmt_tokens(paid, decimals);
    let amount = amount.trim_end_matches('0').trim_end_matches('.');
    let to = r["beneficiary"].as_str().unwrap_or("?");
    let kind = r["t1"]["kind"].as_str().or(r["verdict"]["kind"].as_str()).unwrap_or("?");
    // A record from before `payout_path`: its run's tier says the same.
    let path = r["payout_path"].as_str().or(r["t1"]["tier"].as_str()).unwrap_or("T0");
    let mut out = vec![
        format!("Practice try: {}, vault {} ({})", r["account"].as_str().unwrap_or("?"), r["vault_index"], r["vault"].as_str().unwrap_or("?")),
        format!("  proof: the public tutorial proof, sealed against the program's pinned base witness ({} bytes; no Ethereum RPC)", r["witness_bytes"]),
        if path == "T1" {
            format!("  verdict: {kind}, first answered at T0 (OutLayer's result), then T1 (its TDX quote verified on NEAR)")
        } else {
            format!("  verdict: {kind}, answered at T0 (OutLayer's result); no T1 (its TDX quote verified on NEAR) before the challenge window ended")
        },
        format!("  payout: {}", payout_words(path, r["challenge_window_s"].as_u64(), r["state"] == json!("Done:Paid"))),
        if r["state"] == json!("Verifying") {
            // Final, but claims are allocated in commit order: an earlier commit not yet final holds it.
            format!("  state: Verifying: verified, and its payout ({symbol} to {to}) follows once the program's earlier commits are settled \
                     (claims go in commit order; the keeper runs allocate and pay)")
        } else {
            format!("  state: {}, {amount} {symbol} paid to {to}: the vending contract paid your bond and your run, so the payout returns to it",
                    r["state"].as_str().unwrap_or("?"))
        },
        format!("  {:.1} s from vend to {}; transactions:", r["seconds"].as_f64().unwrap_or(0.0), if r["state"] == json!("Done:Paid") { "payout" } else { "the end of the try" }),
    ];
    for l in r["links"].as_array().into_iter().flatten() {
        for (k, v) in l.as_object().into_iter().flatten() {
            // The vend and the reclaim are signed by whoever called them; the rest by the practice account.
            let signer = match k.as_str() {
                "vend" => r["vended_by"].as_str(),
                "reclaim" => r["reclaimed_by"].as_str().or(r["vended_by"].as_str()),
                _ => r["account"].as_str(),
            };
            out.push(tx_line(k, v, signer.unwrap_or("?")));
        }
    }
    out.push("Any transaction in full: antibody admin tx <hash> --signer <account>. The full record: add --json".into());
    out.join("\n")
}

// ------------------------------------------------------------------------- the operator

/// Deploys the vending contract on the signer's own account, with the practice-account contract it
/// deploys on each vend (`account_wasm`), and adds the public vend key, for `vend` and `reclaim` only,
/// within `vend_allowance` yoctoNEAR. The payout token is the deployment's.
#[allow(clippy::too_many_arguments)]
pub fn deploy(ctx: &Ctx, wasm: &Path, account_wasm: &Path, owner: &str, program: u32, vaults: u32, terms: &Terms, vend_allowance: u128) -> Result<Value> {
    let me = ctx.signer()?;
    let public = public_vend_signer(&me.account_id);
    let out = ctx.near.send(
        &me,
        &me.account_id,
        vec![
            Action::DeployContract(std::fs::read(wasm)?),
            Action::FunctionCall {
                method: "new".into(),
                args: new_args(owner, &ctx.dep.antibody, &ctx.dep.token, program, vaults, terms, &std::fs::read(account_wasm)?),
                gas: 30 * TGAS,
                deposit: 0,
            },
            Action::AddKey {
                public_key: public.public_key(),
                permission: Permission::FunctionCall {
                    allowance: Some(vend_allowance),
                    receiver_id: me.account_id.clone(),
                    method_names: vec!["vend".into(), "reclaim".into()],
                },
            },
        ],
        "EXECUTED_OPTIMISTIC",
    )?;
    let s = cmd::ensure_ok(ctx, &out, "deploy")?;
    Ok(json!({"vending": me.account_id, "owner": owner, "program": program, "vaults": vaults,
              "public_vend_key": public.public_key_str(), "vend_allowance": vend_allowance.to_string(), "tx": s}))
}

/// The owner points the vending contract at a new practice program (`set_terms`) and gives it the
/// practice-account contract later vends deploy (`set_account_code`): a locked vending contract keeps
/// serving the deployment's new practice programs.
pub fn retarget(ctx: &Ctx, vending: &str, account_wasm: &Path, program: u32, vaults: u32, terms: &Terms) -> Result<Value> {
    let owner = ctx.signer()?;
    // As `set_terms` checks: only the owner, and never an hourly limit of 0 (it would stop every vend).
    let config = vending_config(&ctx.near, vending)?;
    if config.owner != owner.account_id.as_str() {
        return Err(eyre!("{vending}'s owner is {}, not {}", config.owner, owner.account_id));
    }
    if terms.per_hour == 0 {
        return Err(eyre!("an hourly limit of 0 would stop every vend"));
    }
    let mut set_terms = program.to_le_bytes().to_vec();
    set_terms.extend(vaults.to_le_bytes());
    terms.borsh(&mut set_terms);
    let code = std::fs::read(account_wasm)?;
    let mut set_code = (code.len() as u32).to_le_bytes().to_vec();
    set_code.extend(&code);
    let out = ctx.near.send(
        &owner,
        vending,
        vec![
            Action::FunctionCall { method: "set_terms".into(), args: set_terms, gas: 10 * TGAS, deposit: ONE_YOCTO },
            Action::FunctionCall { method: "set_account_code".into(), args: set_code, gas: 20 * TGAS, deposit: ONE_YOCTO },
        ],
        "EXECUTED_OPTIMISTIC",
    )?;
    let s = cmd::ensure_ok(ctx, &out, "set_terms")?;
    Ok(json!({"vending": vending, "owner": owner.account_id, "program": program, "vaults": vaults, "retargeted": true,
              "account_code_sha256": hex::encode(sha256(&code)), "public_vend_key": public_vend_signer(vending).public_key_str(), "tx": s}))
}

/// `set_terms`' arguments (Borsh: the program, the instances, the terms) for `config` with a new program
/// or hourly limit: everything else as the contract has it.
pub fn set_terms_args(config: &VendingConfig, program: Option<u32>, per_hour: Option<u32>) -> Vec<u8> {
    let mut terms = config.terms.clone();
    if let Some(n) = per_hour {
        terms.per_hour = n;
    }
    let mut args = program.unwrap_or(config.program).to_le_bytes().to_vec();
    args.extend(config.instances.to_le_bytes());
    terms.borsh(&mut args);
    args
}

/// The owner changes the hourly vend limit or the program with `set_terms` alone: the other terms, the
/// instances and the practice-account code (`set_account_code`, which [`retarget`] also sends) stay as
/// the contract has them. The configuration is read before and after.
pub fn set_terms(ctx: &Ctx, vending: &str, program: Option<u32>, per_hour: Option<u32>) -> Result<Value> {
    let owner = ctx.signer()?;
    let before = vending_config(&ctx.near, vending)?;
    if before.owner != owner.account_id.as_str() {
        return Err(eyre!("{vending}'s owner is {}, not {}", before.owner, owner.account_id));
    }
    if program.is_none() && per_hour.is_none() {
        return Err(eyre!("nothing to change: give --per-hour or --program"));
    }
    if per_hour == Some(0) {
        return Err(eyre!("--per-hour 0 would stop every vend"));
    }
    let args = set_terms_args(&before, program, per_hour);
    let out = ctx.near.send(&owner, vending, vec![Action::FunctionCall { method: "set_terms".into(), args, gas: 10 * TGAS, deposit: ONE_YOCTO }], "EXECUTED_OPTIMISTIC")?;
    let s = cmd::ensure_ok(ctx, &out, "set_terms")?;
    let after = vending_config(&ctx.near, vending)?;
    Ok(json!({"vending": vending, "owner": owner.account_id,
              "before": {"program": before.program, "instances": before.instances, "per_hour": before.terms.per_hour, "vended": before.vended},
              "after": {"program": after.program, "instances": after.instances, "per_hour": after.terms.per_hour, "vended": after.vended},
              "public_vend_key": public_vend_signer(vending).public_key_str(), "tx": s}))
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

    /// The cap's message names the next full UTC hour and the minutes until it.
    #[test]
    fn the_hourly_cap_says_when_it_starts_again() {
        // 2026-11-16 09:37:10 UTC
        let now = 1_794_821_830;
        assert_eq!(now % 3600, 37 * 60 + 10);
        assert_eq!(next_hour(now), ("10:00".to_string(), 23));
        assert_eq!(next_hour(now - now % 3600), ("10:00".to_string(), 60));
        assert_eq!(next_hour(now - now % 3600 + 3599), ("10:00".to_string(), 1));
        assert_eq!(next_hour(1_794_873_599).0, "00:00");
    }

    /// Your own account calls `vend` only when the public key itself was refused: never after a timeout,
    /// when the first vend may have gone out.
    #[test]
    fn only_a_refused_vend_key_falls_back_to_your_account() {
        for refused in [r#"{"InvalidTxError":{"InvalidAccessKeyError":{"NotEnoughAllowance":{}}}}"#, "AccessKeyNotFound", "MethodNameMismatch"] {
            assert!(vend_key_refused(refused), "{refused}");
        }
        for other in ["send_tx: TIMEOUT_ERROR", "send_tx: Rate limits exceeded", "Smart contract panicked: the hourly limit is reached"] {
            assert!(!vend_key_refused(other), "{other}");
        }
    }

    #[test]
    fn borsh_args_match_the_contract() {
        let t = Terms { gas_funds: 1, allowance: 2, bond_budget: 3, per_commit_cap: 4, per_hour: 5 };
        let a = new_args("o", "b", "t", 7, 24, &t, b"\0asm");
        // three strings (4 + 1 bytes each), two u32, four u128, one u32, then the code (4 + 4 bytes)
        assert_eq!(a.len(), 3 * 5 + 8 + 64 + 4 + 8);
        assert_eq!(&a[15..19], &7u32.to_le_bytes());
        assert_eq!(&a[a.len() - 8..], &[4, 0, 0, 0, 0, b'a', b's', b'm']);
        let v = vend_args("ann", &[9; 32]);
        assert_eq!(&v[..11], &[3, 0, 0, 0, b'a', b'n', b'n', 33, 0, 0, 0]);
        assert_eq!((v[11], v.len()), (0, 11 + 1 + 32));
    }

    /// `get_config` of the deployed practice vending (practice.antibody.testnet, read on 2026-11-07): its
    /// program, its 24 instances, its terms and its 10 vends so far.
    #[test]
    fn a_vending_contracts_config_is_read_whole() {
        let b = hex::decode(
            "10000000616e7469626f64792e746573746e657417000000626f756e74792e616e7469626f64792e746573746e65740c000000777261702e746573746e65740600\
             0000180000000000007f6b1096d44a3b000000000000000000da2b851f0bb454000000000000000040b2bac9e0191e020000000000000000a0dec5adc93536000000\
             00000000060000000a000000",
        )
        .unwrap();
        let c = VendingConfig::parse(&b).unwrap();
        assert_eq!((c.owner.as_str(), c.antibody.as_str(), c.token.as_str()), ("antibody.testnet", "bounty.antibody.testnet", "wrap.testnet"));
        assert_eq!((c.program, c.instances, c.terms.per_hour, c.vended, c.next_instance()), (6, 24, 6, 10, 10));
        assert_eq!(c.terms.per_commit_cap, 1_000_000_000_000_000_000_000);
        assert!(VendingConfig::parse(&b[..b.len() - 1]).unwrap_err().to_string().contains("truncated"));
        assert!(VendingConfig::parse(&[b.as_slice(), &[0]].concat()).is_err(), "nothing after the configuration");
        // The program alone, as `practice::target` reads it.
        let mut r = Borsh { b: &b, i: 0 };
        for _ in 0..3 {
            r.string().unwrap();
        }
        assert_eq!(r.u32().unwrap(), 6);
    }

    /// Which commits hold an instance's key, and what is left: the practice program on 2026-11-07 had 16
    /// vends, 10 paid tries and 2 forfeited ones.
    #[test]
    fn instances_left_counts_the_keys_commits_hold() {
        assert!(holds_key(&json!({"Done": {"Paid": {"amount": "1000000000000000000000", "instant": false, "at": 1}}})));
        assert!(holds_key(&json!({"Done": "Exhausted"})));
        assert!(holds_key(&json!("Revealed")), "a commit still settling may take the key");
        assert!(!holds_key(&json!({"Done": "Forfeited"})));
        assert!(!holds_key(&json!({"Done": "Invalid"})));
        let terms = Terms { gas_funds: 1, allowance: 1, bond_budget: 1, per_commit_cap: 1, per_hour: 6 };
        let mut c = VendingConfig { owner: "o".into(), antibody: "a".into(), token: "t".into(), program: 6, instances: 24, terms, vended: 16 };
        let taken: std::collections::BTreeSet<u32> = [0, 1, 2, 3, 5, 6, 8, 9, 12, 13].into_iter().collect();
        let v = instances_left(&c, &taken);
        assert_eq!((v["proved"].as_u64(), v["fresh"].as_u64(), v["next_instance"].as_u64(), v["next_is_fresh"].as_bool()), (Some(10), Some(14), Some(16), Some(true)));
        // Eight vends later the count wraps to instance 0, which a paid try holds: that try is a duplicate.
        c.vended = 24;
        assert_eq!(instances_left(&c, &taken)["next_is_fresh"], json!(false));
    }

    /// `practice terms` changes only what it is given: the hourly limit, or the program, with the
    /// instances and every other term as the contract has them (the same config as above).
    #[test]
    fn set_terms_changes_only_the_hour_or_the_program() {
        let b = hex::decode(
            "10000000616e7469626f64792e746573746e657417000000626f756e74792e616e7469626f64792e746573746e65740c000000777261702e746573746e65740600\
             0000180000000000007f6b1096d44a3b000000000000000000da2b851f0bb454000000000000000040b2bac9e0191e020000000000000000a0dec5adc93536000000\
             00000000060000000a000000",
        )
        .unwrap();
        let c = VendingConfig::parse(&b).unwrap();
        // The contract's `set_terms(program_id, vaults, terms)`: two u32, four u128, one u32.
        let a = set_terms_args(&c, None, Some(30));
        assert_eq!(a.len(), 4 + 4 + 64 + 4);
        assert_eq!(&a[..8], &[6, 0, 0, 0, 24, 0, 0, 0]);
        assert_eq!(&a[a.len() - 4..], &30u32.to_le_bytes());
        // The four amounts are get_config's, byte for byte: the hex above from the program id on, less its
        // hourly limit and count.
        let terms_at = b.len() - 4 - 4 - 64;
        assert_eq!(&a[8..72], &b[terms_at..terms_at + 64]);
        let p = set_terms_args(&c, Some(8), None);
        assert_eq!(&p[..4], &8u32.to_le_bytes());
        assert_eq!(&p[p.len() - 4..], &6u32.to_le_bytes(), "the hourly limit stays");
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

    /// A try's record, as `try_it` returns it, down to what the summary reads.
    fn record(path: &str, window: u64, state: &str) -> Value {
        json!({"account": "ann.practice.antibody.testnet", "vault_index": 4, "vault": "0x185d4f6a747091760f39adbe66a3a414ce8fe56a",
               "vended_by": "practice.antibody.testnet", "witness_bytes": 363, "t1": {"kind": "Violation", "tier": path},
               "payout_path": path, "challenge_window_s": window, "state": state, "paid_raw": "1000000000000000000000",
               "beneficiary": "practice.antibody.testnet", "seconds": 44.0, "links": [{"pay": "testnet:Hash1"}]})
    }

    #[test]
    fn the_summary_says_which_path_paid() {
        let t1 = try_summary(&record("T1", 60, "Done:Paid"), 24, "wNEAR");
        assert!(t1.contains("\n  payout: T1-confirmed: paid at once (an instant payout, under the cap)\n"), "{t1}");
        assert!(t1.contains("then T1 (its TDX quote verified on NEAR)") && t1.contains("0.001 wNEAR paid to practice.antibody.testnet"), "{t1}");
        // The program's own window, not a constant.
        let t0 = try_summary(&record("T0", 90, "Done:Paid"), 24, "wNEAR");
        assert!(t0.contains("\n  payout: T0, paid after the 90 s challenge window\n"), "{t0}");
        assert!(!t0.contains("then T1") && t0.contains("no T1 (its TDX quote verified on NEAR) before the challenge window ended"), "{t0}");
        // Final, its payout behind an earlier commit's claim.
        let held = try_summary(&record("T0", 60, "Verifying"), 24, "wNEAR");
        assert!(held.contains("payout: T0, final after the 60 s challenge window") && held.contains("state: Verifying"), "{held}");
        // A record from before `payout_path`: the run's tier.
        let mut old = record("T1", 60, "Done:Paid");
        old.as_object_mut().unwrap().retain(|k, _| k != "payout_path" && k != "challenge_window_s");
        assert!(try_summary(&old, 24, "wNEAR").contains("payout: T1-confirmed: paid at once"));
    }
}

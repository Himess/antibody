//! Replays traces of `formal/antibody.qnt` against the rule core.
//!
//! Each step's action and nondeterministic picks (`quint run --mbt --out-itf`) are applied to the
//! engine, which must accept every action the model took. After every step the model's state is compared
//! with the engine's: money, bonds, every commit's state, key holders, tickets, permits and disclosures.
//!
//! Traces come from `QUINT_TRACES` (a directory of `*.itf.json`). Without it the test only checks that
//! the harness is wired; CI generates traces first (`scripts/formal/replay.sh`).

use std::collections::BTreeMap;

use antibody_rules::engine::{NewProgram, RunSpec};
use antibody_rules::*;
use serde_json::Value;

const T0: Time = 1_800_000_000; // divisible by the instant window (400 s), like the model's tick 0
const TICK: Time = 100;
const PROJECT: &str = "project.testnet";
const KEEPER: &str = "keeper.testnet";
const THIRD: &str = "third.testnet";

fn params() -> Params {
    Params {
        critical: 200,
        high: 400,
        medium: 100,
        low: 0,
        impact_bps: 0,
        impact_cap: 0,
        decimals: 6,
        instant_cap: 300,
        instant_budget: 300,
        instant_window: 4 * TICK,
        protocol_fee_bps: 1000,
        release_fee: 0,
        bond: 100,
        challenge_bond: 100,
        windows: Windows {
            reveal: 2 * TICK,
            challenge: 2 * TICK,
            resolve: 2 * TICK,
            infra_grace: 3 * TICK,
            run_timeout: 30,
            round_min: 3 * TICK,
            rerun_window: TICK,
            disclosure_max_delay: 6 * TICK,
            deadline: T0 + 10 * TICK,
            exit_grace: 2 * TICK,
        },
        slots: vec![
            SlotRule { slot: 0, severity: Severity::Critical, budget: 500 },
            SlotRule { slot: 1, severity: Severity::High, budget: 500 },
        ],
        unevaluable: UnevaluablePolicy::Refund,
        gap: Some(GapLane { threshold_usd_micro: 1_000, claim_amount: 100 }),
        known_max_count: 1,
        known_max_share_bps: 5000,
        pause_slots: vec![0],
        pause_min_impact_usd_micro: 0,
        run_deposit: 1,
    }
}

fn tag(key: &str) -> H32 {
    let b = match key {
        "A" => 0xA1,
        "B" => 0xB1,
        "C" => 0xC1,
        _ => 0x61, // the gap invariant's key
    };
    H32([b; 32])
}

// ---------------------------------------------------------------- ITF decoding

fn int(v: &Value) -> i64 {
    match v {
        Value::Object(m) if m.contains_key("#bigint") => m["#bigint"].as_str().unwrap().parse().unwrap(),
        Value::Number(n) => n.as_i64().unwrap(),
        _ => panic!("not an int: {v}"),
    }
}
fn string(v: &Value) -> String {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}")).to_string()
}
fn set(v: &Value) -> Vec<Value> {
    v["#set"].as_array().unwrap().clone()
}
fn map(v: &Value) -> Vec<(Value, Value)> {
    v["#map"].as_array().unwrap().iter().map(|e| (e[0].clone(), e[1].clone())).collect()
}
fn tup(v: &Value) -> Vec<Value> {
    v["#tup"].as_array().unwrap().clone()
}
fn pick(state: &Value, name: &str) -> Option<Value> {
    let p = &state["mbt::nondetPicks"][name];
    if p["tag"] == "Some" {
        Some(p["value"].clone())
    } else {
        None
    }
}

// ---------------------------------------------------------------- the model's vocabulary

fn key_of(facts: &str) -> &'static str {
    match facts {
        "VA" => "A",
        "VB" => "B",
        _ => "C",
    }
}
fn facts_of(truth: &str) -> String {
    match truth {
        "VA1" | "VA2" => "VA".into(),
        "VB1" | "VB2" => "VB".into(),
        "VC1" | "VC2" => "VC".into(),
        t => t.into(),
    }
}
fn slot_of(key: &str) -> u8 {
    if key == "A" {
        0
    } else {
        1
    }
}

/// Facts in the engine's terms: (kind, keys, protected harm).
fn engine_facts(f: &str) -> (Kind, Vec<(u8, H32)>, Amount) {
    match f {
        "VA" | "VB" | "VC" => (Kind::Violation, vec![(slot_of(key_of(f)), tag(key_of(f)))], 0),
        "NoV" => (Kind::NoViolation, vec![], 0),
        "Gap" => (Kind::NoViolation, vec![], 5_000),
        "Inv" => (Kind::InvalidPoC, vec![], 0),
        "Unev" => (Kind::Unevaluable, vec![], 0),
        "Mis" => (Kind::ProgramMisconfigured, vec![], 0),
        "Infra" => (Kind::InfraError, vec![], 0),
        x => panic!("unknown facts {x}"),
    }
}

// ---------------------------------------------------------------- the harness

struct Replay {
    e: Engine<MemStore>,
    pid: ProgramId,
    tick: i64,
    req: u64,
    truth: BTreeMap<u64, String>,
    fix: Vec<String>,
    adds_inv: bool,
}

fn last_run(fx: &[Effect]) -> RunId {
    fx.iter().rev().find_map(|f| if let Effect::RequestRun { run, .. } = f { Some(*run) } else { None }).expect("a run")
}

impl Replay {
    fn new() -> Replay {
        let mut e = Engine::new(MemStore::default(), "antibody.testnet");
        for a in [PROJECT, "h1.testnet", "h2.testnet", KEEPER, THIRD] {
            e.near_deposit(a, 1 << 60).unwrap();
        }
        let mut r = Replay { e, pid: 0, tick: 0, req: 0, truth: BTreeMap::new(), fix: vec![], adds_inv: false };
        let ctx = r.ctx(PROJECT);
        let (pid, _) = r
            .e
            .create_program(&ctx, NewProgram { project: PROJECT.into(), manifest_sha256: H32([7; 32]), params: params(), escrow: 500, anchor_uncheckable: false })
            .unwrap();
        r.pid = pid;
        r.e.anchor_checked(pid, Some(true)).unwrap();
        let spec = r.spec();
        let run = last_run(&r.e.activate(&ctx, pid, spec).unwrap());
        r.answer_raw(run, Kind::NoViolation, &[], 0, serde_json::json!({"sealing_pk": H32([9; 32])}));
        r.t1(run);
        r
    }

    fn now(&self) -> Time {
        T0 + self.tick as Time * TICK
    }
    fn ctx(&self, who: &str) -> Ctx {
        Ctx { now: self.now(), caller: who.into(), signer: who.into() }
    }
    fn spec(&mut self) -> RunSpec {
        self.req += 1;
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&self.req.to_be_bytes());
        RunSpec { request_sha256: H32(b) }
    }
    fn hunter_of(&self, c: CommitId) -> String {
        self.e.store.commit(c).unwrap().hunter
    }

    fn answer_raw(&mut self, run: RunId, kind: Kind, keys: &[(u8, H32)], harm: Amount, extra: Value) {
        let r = self.e.store.run(run).unwrap();
        let p = self.e.store.program(r.program).unwrap();
        let manifest = match (&p.proposal, r.mode) {
            (Some(pr), Mode::Rerun | Mode::CheckRound) => pr.manifest_sha256,
            _ => p.manifest_of(r.round),
        };
        let mut f = serde_json::json!({
            "v": 2, "mode": r.mode, "request_sha256": r.request_sha256, "manifest_sha256": manifest,
            "program_key": p.key, "round": r.round,
            "key_epoch": if r.mode == Mode::Activate { "00112233aabbccdd".to_string() } else { p.key_epoch.clone() },
            "commit_id": r.commit, "run_id": run, "kind": kind,
            "keys": keys.iter().map(|(s, t)| serde_json::json!({"slot": s, "group": 0, "tag": t})).collect::<Vec<_>>(),
            "protected_harm_usd_micro": harm.to_string(),
            "receipt_sha256": H32([format!("{kind:?}{keys:?}{harm}").bytes().fold(7u8, |a, b| a.wrapping_mul(31).wrapping_add(b)); 32]),
        });
        if let Value::Object(m) = extra {
            for (k, v) in m {
                f[k] = v;
            }
        }
        let bytes = serde_json::to_vec(&f).unwrap();
        let ctx = self.ctx("outlayer.testnet");
        self.e.run_answered(&ctx, run, &bytes, H32([0xEE; 32])).unwrap();
    }

    fn answer(&mut self, run: RunId, facts: &str) {
        let (kind, keys, harm) = engine_facts(facts);
        self.answer_raw(run, kind, &keys, harm, serde_json::json!({}));
    }

    fn t1(&mut self, run: RunId) {
        let ctx = self.ctx("relayer.testnet");
        self.e.confirm_t1(&ctx, run).unwrap();
    }

    /// The commit's pending verification run, after a retry if it has none.
    fn pending_run(&mut self, c: CommitId) -> Result<RunId, Error> {
        let cm = self.e.store.commit(c).unwrap();
        if let Some(rid) = cm.run {
            if self.e.store.run(rid).unwrap().outcome == RunOutcome::Pending {
                return Ok(rid);
            }
        }
        let h = self.hunter_of(c);
        let ctx = self.ctx(&h);
        let spec = self.spec();
        Ok(last_run(&self.e.retry(&ctx, c, spec)?))
    }

    fn rerun_facts(&self, c: CommitId) -> String {
        let t = self.truth.get(&c).cloned().unwrap_or_default();
        if t.starts_with('V') {
            if self.fix.contains(&t) {
                "NoV".into()
            } else {
                facts_of(&t)
            }
        } else if t == "Gap" && self.adds_inv {
            "GapBreaks".into()
        } else {
            "NoV".into()
        }
    }

    /// Applies one model step. Errors mean the engine refused an action the model took.
    fn apply(&mut self, action: &str, st: &Value) -> Result<(), Error> {
        let c = pick(st, "c").map(|v| int(&v) as CommitId);
        let ctx_keeper = self.ctx(KEEPER);
        match action {
            "Tick" => self.tick += 1,
            "OwnerSetPins" => {}
            "DeclareKnown" => {
                let b = string(&pick(st, "b").unwrap());
                let ctx = self.ctx(PROJECT);
                let spec = self.spec();
                let run = last_run(&self.e.submit_known(&ctx, self.pid, spec)?);
                self.answer(run, &facts_of(&b));
                self.t1(run);
                if self.e.store.program(self.pid).unwrap().known.is_empty() {
                    return Err(Error::State("the engine refused a known issue the model accepted"));
                }
            }
            "Open" => {
                let ctx = self.ctx(PROJECT);
                self.e.open_program(&ctx, self.pid)?;
            }
            "MakeCommit" => {
                let h = format!("{}.testnet", string(&pick(st, "h").unwrap()));
                let ctx = self.ctx(&h);
                let round = self.e.store.program(self.pid).unwrap().round;
                let n = self.e.store.program(self.pid).unwrap().commits;
                self.e.commit(&ctx, self.pid, round, H32([n as u8 + 1; 32]), true, 100, false)?;
            }
            "Reveal" => {
                let c = c.unwrap();
                let t = string(&pick(st, "t").unwrap());
                self.truth.insert(c, t);
                let h = self.hunter_of(c);
                let ctx = self.ctx(&h);
                let cm = self.e.store.commit(c).unwrap().commitment;
                let spec = self.spec();
                self.e.reveal(&ctx, c, cm, H32([1; 32]), spec)?;
            }
            "Expire" => {
                self.e.expire(&ctx_keeper, c.unwrap())?;
            }
            "FactsT0" => {
                let f = string(&pick(st, "f").unwrap());
                let run = self.pending_run(c.unwrap())?;
                self.answer(run, &f);
            }
            "FactsT1" => {
                let c = c.unwrap();
                let f = facts_of(&self.truth[&c]);
                let run = self.pending_run(c)?;
                self.answer(run, &f);
                self.t1(run);
            }
            "ConfirmT1" => {
                let run = self.e.store.commit(c.unwrap()).unwrap().run.unwrap();
                let ctx = self.ctx("relayer.testnet");
                self.e.confirm_t1(&ctx, run)?;
            }
            "Retry" => {
                let c = c.unwrap();
                let h = self.hunter_of(c);
                let ctx = self.ctx(&h);
                let spec = self.spec();
                self.e.retry(&ctx, c, spec)?;
            }
            "InfraTimeout" | "ChallengeTimeout" => {
                self.e.time_out(&ctx_keeper, c.unwrap())?;
            }
            "SecondRun" => {
                let c = c.unwrap();
                let f = string(&pick(st, "f").unwrap());
                let h = self.hunter_of(c);
                let ctx = self.ctx(&h);
                let spec = self.spec();
                let run = last_run(&self.e.request_second(&ctx, c, spec)?);
                self.answer(run, &f);
            }
            "Challenge" => {
                let c = c.unwrap();
                let who = match string(&pick(st, "w").unwrap()).as_str() {
                    "project" => PROJECT.to_string(),
                    "hunter" => self.hunter_of(c),
                    _ => THIRD.to_string(),
                };
                let ctx = self.ctx(&who);
                let spec = self.spec();
                self.e.challenge(&ctx, c, 100, spec)?;
            }
            "ResolveT1" => {
                let c = c.unwrap();
                let run = self.e.store.commit(c).unwrap().challenge.unwrap().run.unwrap();
                let f = facts_of(&self.truth[&c]);
                self.answer(run, &f);
                self.t1(run);
            }
            "Allocate" => {
                self.e.allocate(&ctx_keeper, self.pid, 1)?;
            }
            "Pay" | "PayPromoted" => {
                self.e.pay(&ctx_keeper, c.unwrap())?;
            }
            "SettleDebt" => {
                self.e.settle_debts(self.pid, 1)?;
            }
            "TopUp" => {
                self.e.top_up(self.pid, 200)?;
            }
            "Withdraw" => {
                let mut v = int(&pick(st, "v").unwrap()) as Amount;
                for a in [engine::TREASURY, PROJECT, "h1.testnet", "h2.testnet", THIRD] {
                    let have = self.e.store.account(a).tokens;
                    let take = have.min(v);
                    if take > 0 {
                        self.e.withdraw(&self.ctx(a), take)?;
                        v -= take;
                    }
                }
                assert_eq!(v, 0, "the engine's credits do not cover a withdrawal the model allows");
            }
            "PauseTicket" => {
                self.e.pause_ticket(&ctx_keeper, c.unwrap())?;
            }
            "ProposeRound" => {
                self.fix = set(&pick(st, "f").unwrap()).iter().map(string).collect();
                self.adds_inv = pick(st, "a").unwrap().as_bool().unwrap();
                let canaries = pick(st, "b").unwrap().as_bool().unwrap();
                let slots = if self.adds_inv { vec![SlotRule { slot: 2, severity: Severity::Medium, budget: 500 }] } else { vec![] };
                let ctx = self.ctx(PROJECT);
                let spec = self.spec();
                let run = last_run(&self.e.propose_round(&ctx, self.pid, H32([8; 32]), H32([6; 32]), slots, spec)?);
                self.answer_raw(run, Kind::NoViolation, &[], 0, serde_json::json!({"round_check": {"ok": true, "canaries_ok": canaries}}));
                self.t1(run);
            }
            "Rerun" => {
                let c = c.unwrap();
                let f = self.rerun_facts(c);
                let spec = self.spec();
                let run = last_run(&self.e.request_rerun(&ctx_keeper, c, spec)?);
                if f == "GapBreaks" {
                    self.answer_raw(run, Kind::Violation, &[(2, tag("G"))], 0, serde_json::json!({}));
                } else {
                    self.answer(run, &f);
                }
                self.t1(run);
            }
            "OpenRound" => {
                self.e.open_round(&ctx_keeper, self.pid)?;
            }
            "Disclose" => {
                let k = string(&pick(st, "k").unwrap());
                let r = int(&pick(st, "dr").unwrap()) as u32;
                let spec = self.spec();
                let run = last_run(&self.e.request_disclosure(&ctx_keeper, self.pid, &[(r, tag(&k))], spec)?);
                self.answer_raw(run, Kind::NoViolation, &[], 0, serde_json::json!({"disclosed": [{"round": r, "tag": tag(&k), "key": H32([4; 32])}]}));
                self.t1(run);
            }
            "Exit" => {
                self.e.exit(&ctx_keeper, self.pid)?;
            }
            other => panic!("unmapped action {other}"),
        }
        Ok(())
    }

    /// Compares the model's state with the engine's; returns the first difference.
    fn compare(&self, s: &Value) -> Result<(), String> {
        let p = self.e.store.program(self.pid).unwrap();
        let t = self.e.store.totals();
        let l = &p.ledger;
        let eq = |name: &str, model: i64, engine: Amount| -> Result<(), String> {
            if model as Amount != engine {
                Err(format!("{name}: model {model}, engine {engine}"))
            } else {
                Ok(())
            }
        };
        eq("available", int(&s["available"]), l.available)?;
        eq("reserved", int(&s["reserved"]), l.reserved)?;
        eq("paid", int(&s["paid"]), l.paid)?;
        eq("fees", int(&s["fees"]), l.fees)?;
        eq("returned", int(&s["returned"]), l.returned)?;
        eq("funded", int(&s["funded"]), l.funded())?;
        eq("bond forfeits", int(&s["bondForfeited"]), l.forfeits)?;
        eq("bonds locked", int(&s["bondLocked"]), t.bonds_locked)?;
        eq("challenge bonds locked", int(&s["cbondLocked"]), t.challenge_bonds_locked)?;
        eq("credits", int(&s["credits"]), t.credits)?;
        let phase = match (string(&s["phase"]).as_str(), s["misconfigured"].as_bool().unwrap()) {
            ("closed", _) => Phase::Exited,
            ("setup", _) => Phase::Setup,
            (_, true) => Phase::Closed,
            _ => Phase::Open,
        };
        if p.phase != phase {
            return Err(format!("phase: model {phase:?}, engine {:?}", p.phase));
        }
        eq("round", int(&s["round"]), p.round as Amount)?;
        eq("release permits", int(&s["permits"]), p.release_permits as Amount)?;
        let tickets: i64 = map(&s["tickets"]).iter().map(|(_, v)| int(v)).sum();
        eq("pause tickets", tickets, p.ticket_rounds.len() as Amount)?;
        let mut disclosed: Vec<(u32, H32)> = set(&s["disclosed"])
            .iter()
            .map(|rk| {
                let rk = tup(rk);
                (int(&rk[0]) as u32, tag(&string(&rk[1])))
            })
            .collect();
        disclosed.sort();
        let mut ed: Vec<(u32, H32)> = p.disclosed.iter().map(|(r, t, _)| (*r, *t)).collect();
        ed.sort();
        if disclosed != ed {
            return Err("disclosed tags differ".into());
        }
        for (rk, h) in map(&s["holder"]) {
            let rk = tup(&rk);
            let (round, key) = (int(&rk[0]) as u32, string(&rk[1]));
            let model = int(&h);
            let engine = self.e.store.holder(self.pid, round, &tag(&key)).map(|x| x as i64).unwrap_or(-1);
            if model != engine {
                return Err(format!("holder of {key} in round {round}: model {model}, engine {engine}"));
            }
        }
        let n = int(&s["n"]);
        for (cid, x) in map(&s["cs"]) {
            let cid = int(&cid);
            if cid >= n {
                continue;
            }
            let c = self.e.store.commit(cid as CommitId).unwrap();
            let st = string(&x["st"]);
            let alloc = string(&x["alloc"]);
            let end = string(&x["end"]);
            let (res, debt, amount) = (int(&x["res"]) as Amount, int(&x["debt"]) as Amount, int(&x["amount"]) as Amount);
            let ok = match (st.as_str(), alloc.as_str(), end.as_str(), &c.state) {
                ("committed", _, _, CommitState::Committed) => true,
                ("verifying" | "verdict", "none", "", CommitState::Verifying) => true,
                (_, "holder", "", CommitState::Holder { amount: a, reserved: r, debt: d, .. }) => *a == amount && *r == res && *d == debt,
                (_, "dup", "", CommitState::Duplicate { reserved: r, .. }) => *r == res,
                (_, "gap", "", CommitState::GapClaim { reserved: r, .. }) => *r == res,
                (_, "promoted", "", CommitState::Promoted { amount: a, reserved: r, debt: d, .. }) => *a == amount && *r == res && *d == debt,
                ("done", _, "paid", CommitState::Done(End::Paid { .. })) => true,
                ("done", _, "expired", CommitState::Done(End::Expired)) => true,
                ("done", _, "forfeit", CommitState::Done(End::Forfeited)) => true,
                ("done", _, "refund" | "void", CommitState::Done(End::Refunded { .. })) => true,
                ("done", _, "known", CommitState::Done(End::Known)) => true,
                ("done", _, "dupconf", CommitState::Done(End::DuplicateConfirmed)) => true,
                ("done", _, "exhausted", CommitState::Done(End::Exhausted)) => true,
                ("done", _, "gapopen", CommitState::Done(End::GapOpen)) => true,
                ("done", _, "defaulted", CommitState::Done(End::Defaulted { .. })) => true,
                _ => false,
            };
            if !ok {
                return Err(format!("commit {cid}: model ({st}, {alloc}, {end}, res {res}, debt {debt}), engine {:?}", c.state));
            }
        }
        Ok(())
    }
}

fn replay_file(path: &std::path::Path) -> Result<usize, String> {
    let d: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let states = d["states"].as_array().unwrap();
    let mut r = Replay::new();
    r.compare(&states[0]["s"]).map_err(|e| format!("initial state: {e}"))?;
    for (i, st) in states.iter().enumerate().skip(1) {
        let action = string(&st["mbt::actionTaken"]);
        r.apply(&action, st).map_err(|e| format!("step {i} ({action}): the engine refused: {e}"))?;
        r.compare(&st["s"]).map_err(|e| format!("step {i} ({action}): {e}"))?;
        let a = audit::program(&r.e.store, r.pid).unwrap();
        if !a.ok || !audit::contract(&r.e.store).ok {
            return Err(format!("step {i} ({action}): audit failed {a:?}"));
        }
    }
    Ok(states.len())
}

#[test]
fn replays_quint_traces() {
    let Ok(dir) = std::env::var("QUINT_TRACES") else {
        eprintln!("QUINT_TRACES not set: no traces replayed (scripts/formal/replay.sh generates them)");
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with(".itf.json")).collect();
    files.sort();
    assert!(!files.is_empty(), "no traces in {dir}");
    let mut steps = 0;
    let mut failures = vec![];
    for f in &files {
        match replay_file(f) {
            Ok(n) => steps += n,
            Err(e) => failures.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!("replayed {} traces, {} steps", files.len(), steps);
    if !failures.is_empty() {
        panic!("{} of {} traces diverge:\n{}", failures.len(), files.len(), failures.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
    }
}

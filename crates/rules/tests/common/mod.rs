//! A test world around the engine: it plays the contract adapter (request bytes, OutLayer answers,
//! relayed quotes) so tests read like the Quint scenarios in `formal/antibody.qnt`.
#![allow(dead_code)]

use antibody_rules::engine::{NewProgram, RunSpec};
use antibody_rules::*;

pub const CONTRACT: &str = "antibody.testnet";
pub const PROJECT: &str = "acme.testnet";
pub const H1: &str = "alice.testnet";
pub const H2: &str = "bob.testnet";
pub const T0: Time = 1_800_000_000;

pub fn tag(n: u8) -> H32 {
    H32([n; 32])
}

pub fn params() -> Params {
    Params {
        critical: 200,
        high: 100,
        medium: 50,
        low: 10,
        impact_bps: 0,
        impact_cap: 0,
        decimals: 6,
        instant_cap: 300,
        instant_budget: 300,
        instant_window: 3600,
        protocol_fee_bps: 0,
        release_fee: 0,
        bond: 10,
        challenge_bond: 10,
        windows: Windows {
            reveal: 60,
            challenge: 120,
            resolve: 120,
            infra_grace: 300,
            run_timeout: 60,
            round_min: 60,
            rerun_window: 60,
            disclosure_max_delay: 600,
            deadline: T0 + 10_000,
            exit_grace: 600,
        },
        slots: vec![
            SlotRule { slot: 0, severity: Severity::Critical, budget: 1_000 },
            SlotRule { slot: 1, severity: Severity::High, budget: 1_000 },
            SlotRule { slot: 2, severity: Severity::Medium, budget: 1_000 },
        ],
        unevaluable: UnevaluablePolicy::Refund,
        gap: Some(GapLane { threshold_usd_micro: 1_000, claim_amount: 20 }),
        known_max_count: 2,
        known_max_share_bps: 2_000,
        pause_slots: vec![0],
        pause_min_impact_usd_micro: 0,
        run_deposit: 7,
    }
}

pub struct W {
    pub e: Engine<MemStore>,
    pub now: Time,
    req: u64,
    pub events: Vec<(String, serde_json::Value)>,
}

pub fn last_run(fx: &[Effect]) -> RunId {
    fx.iter()
        .rev()
        .find_map(|f| match f {
            Effect::RequestRun { run, .. } => Some(*run),
            _ => None,
        })
        .expect("a run was requested")
}

impl W {
    pub fn new() -> W {
        let mut w = W { e: Engine::new(MemStore::default(), CONTRACT), now: T0, req: 0, events: vec![] };
        for a in [PROJECT, H1, H2, "carol.testnet", "keeper.testnet", "anyone.testnet"] {
            w.e.near_deposit(a, 1_000_000).unwrap();
            w.e.token_deposit(a, 1_000).unwrap();
        }
        w
    }

    pub fn ctx(&self, who: &str) -> Ctx {
        Ctx { now: self.now, caller: who.to_string(), signer: who.to_string() }
    }

    pub fn spec(&mut self) -> RunSpec {
        self.req += 1;
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&self.req.to_be_bytes());
        b[31] = 0xAA;
        RunSpec { request_sha256: H32(b) }
    }

    pub fn tick(&mut self, secs: Time) {
        self.now += secs;
    }

    /// Closes every record of an exited program (`close_commits`, a page at a time).
    pub fn close_all(&mut self, program: ProgramId) {
        let ctx = self.ctx("anyone.testnet");
        while let Ok(fx) = self.e.close_commits(&ctx, program, 3) {
            self.log(&fx);
        }
    }

    pub fn log(&mut self, fx: &[Effect]) {
        for f in fx {
            if let Effect::Event { name, data } = f {
                self.events.push((name.to_string(), serde_json::from_str(data).unwrap()));
            }
        }
    }

    pub fn ok(&mut self, r: Result<Vec<Effect>, Error>) -> Vec<Effect> {
        let fx = r.unwrap_or_else(|e| panic!("unexpected error: {e}"));
        self.log(&fx);
        fx
    }

    pub fn program(&self, id: ProgramId) -> Program {
        self.e.store.program(id).unwrap()
    }
    pub fn commit(&self, id: CommitId) -> Commit {
        self.e.store.commit(id).unwrap()
    }
    pub fn run(&self, id: RunId) -> Run {
        self.e.store.run(id).unwrap()
    }

    /// The facts a verifier run would return for `run`.
    pub fn facts(&self, run: RunId, kind: Kind, keys: &[(u8, H32)], extra: serde_json::Value) -> Vec<u8> {
        let r = self.run(run);
        let p = self.program(r.program);
        let manifest = match (&p.proposal, r.mode) {
            (Some(pr), Mode::Rerun | Mode::CheckRound) => pr.manifest_sha256,
            _ => p.manifest_of(r.round),
        };
        let mut f = serde_json::json!({
            "v": 2,
            "mode": r.mode,
            "request_sha256": r.request_sha256,
            "manifest_sha256": manifest,
            "program_key": p.key,
            "round": r.round,
            "key_epoch": if r.mode == Mode::Activate { "00112233aabbccdd".to_string() } else { p.key_epoch.clone() },
            "commit_id": r.commit,
            "run_id": run,
            "kind": kind,
            "keys": keys.iter().map(|(s, t)| serde_json::json!({"slot": s, "group": 0, "tag": t})).collect::<Vec<_>>(),
            // Deterministic: two runs of the same PoC give the same receipt.
            "receipt_sha256": H32([format!("{kind:?}{keys:?}").bytes().fold(7u8, |a, b| a.wrapping_mul(31).wrapping_add(b)); 32]),
        });
        if let serde_json::Value::Object(m) = extra {
            for (k, v) in m {
                f[k] = v;
            }
        }
        serde_json::to_vec(&f).unwrap()
    }

    pub fn answer(&mut self, run: RunId, kind: Kind, keys: &[(u8, H32)]) -> Vec<Effect> {
        self.answer_x(run, kind, keys, serde_json::json!({}))
    }

    pub fn answer_x(&mut self, run: RunId, kind: Kind, keys: &[(u8, H32)], extra: serde_json::Value) -> Vec<Effect> {
        let bytes = self.facts(run, kind, keys, extra);
        let ctx = self.ctx("outlayer.testnet");
        let r = self.e.run_answered(&ctx, run, &bytes, H32([0xEE; 32]));
        self.ok(r)
    }

    pub fn t1(&mut self, run: RunId) -> Vec<Effect> {
        let ctx = self.ctx("relayer.testnet");
        let r = self.e.confirm_t1(&ctx, run);
        self.ok(r)
    }

    /// A program through creation, a light-client anchor, a T1 activation and open.
    pub fn program_open(&mut self, escrow: Amount, p: Params) -> ProgramId {
        let ctx = self.ctx(PROJECT);
        let (id, fx) = self
            .e
            .create_program(&ctx, NewProgram { project: PROJECT.into(), manifest_sha256: H32([7; 32]), params: p, escrow, anchor_uncheckable: false })
            .unwrap();
        self.log(&fx);
        let r = self.e.anchor_checked(id, Some(true));
        self.ok(r);
        let spec = self.spec();
        let r = self.e.activate(&ctx, id, spec);
        let fx = self.ok(r);
        let run = last_run(&fx);
        self.answer_x(run, Kind::NoViolation, &[], serde_json::json!({"sealing_pk": H32([9; 32])}));
        self.t1(run);
        let r = self.e.open_program(&ctx, id);
        self.ok(r);
        id
    }

    /// A commit and its reveal; returns (commit, run).
    pub fn commit_reveal(&mut self, program: ProgramId, who: &str) -> (CommitId, RunId) {
        let ctx = self.ctx(who);
        let round = self.program(program).round;
        let cm = H32([self.req as u8 + 100; 32]);
        let (cid, fx) = self.e.commit(&ctx, program, round, cm, true, 10, false).unwrap();
        self.log(&fx);
        let spec = self.spec();
        let r = self.e.reveal(&ctx, cid, cm, H32([1; 32]), spec);
        let fx = self.ok(r);
        (cid, last_run(&fx))
    }

    /// A commit, its reveal, and an answer (optionally T1).
    pub fn finding(&mut self, program: ProgramId, who: &str, kind: Kind, keys: &[(u8, H32)], t1: bool) -> (CommitId, RunId) {
        let (cid, run) = self.commit_reveal(program, who);
        self.answer(run, kind, keys);
        if t1 {
            self.t1(run);
        }
        (cid, run)
    }

    pub fn allocate(&mut self, program: ProgramId) -> Result<Vec<Effect>, Error> {
        let ctx = self.ctx("keeper.testnet");
        let r = self.e.allocate(&ctx, program, 50);
        if let Ok(fx) = &r {
            self.log(fx);
        }
        r
    }

    pub fn pay(&mut self, commit: CommitId) -> Result<Vec<Effect>, Error> {
        let ctx = self.ctx("keeper.testnet");
        let r = self.e.pay(&ctx, commit);
        if let Ok(fx) = &r {
            self.log(fx);
        }
        r
    }

    pub fn end(&self, c: CommitId) -> Option<End> {
        match self.commit(c).state {
            CommitState::Done(e) => Some(e),
            _ => None,
        }
    }

    /// Proposes a round with a T1-confirmed round check.
    pub fn propose(&mut self, program: ProgramId, new_slots: Vec<SlotRule>, canaries_ok: bool) -> RunId {
        let ctx = self.ctx(PROJECT);
        let spec = self.spec();
        let r = self.e.propose_round(&ctx, program, H32([8; 32]), new_slots, spec);
        let fx = self.ok(r);
        let run = last_run(&fx);
        self.answer_x(run, Kind::NoViolation, &[], serde_json::json!({"round_check": {"ok": true, "canaries_ok": canaries_ok}}));
        self.t1(run);
        run
    }

    /// A T1 re-run of `commit` at the proposal answering `kind`/`keys`.
    pub fn rerun(&mut self, commit: CommitId, kind: Kind, keys: &[(u8, H32)]) -> RunId {
        let ctx = self.ctx("keeper.testnet");
        let spec = self.spec();
        let r = self.e.request_rerun(&ctx, commit, spec);
        let fx = self.ok(r);
        let run = last_run(&fx);
        self.answer(run, kind, keys);
        self.t1(run);
        run
    }

    /// Opens the next round, a page of 2 commits per call (as many calls as it takes).
    pub fn open_round(&mut self, program: ProgramId) -> Result<Vec<Effect>, Error> {
        let ctx = self.ctx("keeper.testnet");
        let mut all = vec![];
        loop {
            let fx = self.e.open_round(&ctx, program, 2)?;
            self.log(&fx);
            all.extend(fx);
            if self.e.store.program(program).unwrap().opening.is_none() {
                return Ok(all);
            }
        }
    }

    /// Every conservation equation holds.
    pub fn audit_ok(&self, programs: &[ProgramId]) {
        for id in programs {
            let a = audit::program(&self.e.store, *id).unwrap();
            assert!(a.ok, "program audit failed: {a:?}");
        }
        let c = audit::contract(&self.e.store);
        assert!(c.ok, "contract audit failed: {c:?}");
        assert!(audit::escrow_matches(&self.e.store, programs), "escrow total mismatch");
    }
}

//! The transitions (spec §3–§14). One method per contract method or callback. Every method either
//! returns the effects to carry out, or an error and changes nothing that it has not already
//! validated: checks come before writes.

use crate::arith;
use crate::facts::{self, Expect, Extra, FactsSummary};
use crate::state::*;
use crate::store::{IdKind, Store};
use crate::types::*;

/// The caller and the time of a call.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub now: Time,
    /// The account calling near-antibody (the predecessor).
    pub caller: AccountId,
    /// The transaction's signer (OutLayer attests it as a run's caller).
    pub signer: AccountId,
}

/// What the contract must do after a transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Ask the light client for the anchor's block hash (`anchor_checked` answers).
    CheckAnchor { program: ProgramId },
    /// Call OutLayer for this run; `run_answered` or `run_failed` answers.
    RequestRun { run: RunId, mode: Mode, deposit: Amount, payer: AccountId },
    /// Transfer tokens out (`withdraw`); `withdraw_failed` credits them back.
    TransferTokens { to: AccountId, amount: Amount },
    /// Transfer NEAR out (`withdraw_near`).
    TransferNear { to: AccountId, amount: Amount },
    /// Ask the guardian for a pause permit (spec §10).
    PausePermit { program: ProgramId, round: u32, ticket: CommitId },
    /// Ask the guardian for a release permit (spec §9).
    ReleasePermit { program: ProgramId, round: u32 },
    /// An NEP-297 event.
    Event { name: &'static str, data: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    NotFound(&'static str),
    Forbidden(&'static str),
    State(&'static str),
    Invalid(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::NotFound(w) => write!(f, "not found: {w}"),
            Error::Forbidden(w) => write!(f, "forbidden: {w}"),
            Error::State(w) => write!(f, "not now: {w}"),
            Error::Invalid(w) => write!(f, "invalid: {w}"),
        }
    }
}

type R<T> = Result<T, Error>;

fn ev(name: &'static str, data: serde_json::Value) -> Effect {
    Effect::Event { name, data: data.to_string() }
}

/// Parameters of a new program, from the manifest (spec §3.2).
#[derive(Clone, Debug)]
pub struct NewProgram {
    pub project: AccountId,
    pub manifest_sha256: H32,
    pub params: Params,
    pub escrow: Amount,
    /// The light client cannot check this chain or block: the anchor is `Unconfirmed` at once.
    pub anchor_uncheckable: bool,
}

/// A run about to be requested: the adapter built the request bytes and hashed them.
#[derive(Clone, Debug)]
pub struct RunSpec {
    pub request_sha256: H32,
}

pub struct Engine<S: Store> {
    pub store: S,
    /// near-antibody's account: program keys are `"<account>/<id>"`.
    pub contract: AccountId,
}

impl<S: Store> Engine<S> {
    pub fn new(store: S, contract: &str) -> Self {
        Engine { store, contract: contract.to_string() }
    }

    // ------------------------------------------------------------------ loading

    fn get_program(&self, id: ProgramId) -> R<Program> {
        self.store.program(id).ok_or(Error::NotFound("program"))
    }
    fn get_commit(&self, id: CommitId) -> R<Commit> {
        self.store.commit(id).ok_or(Error::NotFound("commit"))
    }
    fn get_run(&self, id: RunId) -> R<Run> {
        self.store.run(id).ok_or(Error::NotFound("run"))
    }

    // ------------------------------------------------------------------ money helpers

    fn credit_tokens(&mut self, who: &str, amount: Amount) {
        if amount == 0 {
            return;
        }
        let mut a = self.store.account(who);
        a.tokens += amount;
        self.store.put_account(who, &a);
        let mut t = self.store.totals();
        t.credits += amount;
        self.store.put_totals(&t);
    }

    fn debit_tokens(&mut self, who: &str, amount: Amount) -> R<()> {
        let mut a = self.store.account(who);
        if a.tokens < amount {
            return Err(Error::State("not enough token credit"));
        }
        a.tokens -= amount;
        self.store.put_account(who, &a);
        let mut t = self.store.totals();
        t.credits -= amount;
        self.store.put_totals(&t);
        Ok(())
    }

    fn debit_near(&mut self, who: &str, amount: Amount) -> R<()> {
        let mut a = self.store.account(who);
        if a.near < amount {
            return Err(Error::State("not enough NEAR credit for verification (storage_deposit)"));
        }
        a.near -= amount;
        if let Some(ag) = a.agent.as_mut() {
            ag.verification_spent += amount;
        }
        self.store.put_account(who, &a);
        let mut t = self.store.totals();
        t.near_credits -= amount;
        self.store.put_totals(&t);
        Ok(())
    }

    /// The contract-wide escrow total, beside a program's ledger (`Ledger::deposit`, `forfeit`, `award`).
    fn escrow_in(&mut self, amount: Amount) {
        let mut t = self.store.totals();
        t.escrow += amount;
        self.store.put_totals(&t);
    }

    fn escrow_out(&mut self, amount: Amount) {
        let mut t = self.store.totals();
        t.escrow -= amount;
        self.store.put_totals(&t);
    }

    fn bonds_locked(&mut self, delta_in: Amount, delta_out: Amount) {
        let mut t = self.store.totals();
        t.bonds_locked = t.bonds_locked + delta_in - delta_out;
        self.store.put_totals(&t);
    }

    /// A bond goes back to the commit's beneficiary.
    fn refund_bond(&mut self, c: &Commit) {
        self.bonds_locked(0, c.bond);
        self.credit_tokens(&c.beneficiary.clone(), c.bond);
        self.agent_bond_settled(c, false);
    }

    /// A bond goes to the program's escrow (spec §12.2).
    fn forfeit_bond(&mut self, p: &mut Program, c: &Commit) {
        self.bonds_locked(0, c.bond);
        p.ledger.forfeit(c.bond);
        self.escrow_in(c.bond);
        self.agent_bond_settled(c, true);
    }

    fn agent_bond_settled(&mut self, c: &Commit, lost: bool) {
        if c.hunter == c.beneficiary {
            return;
        }
        let mut a = self.store.account(&c.hunter);
        if let Some(ag) = a.agent.as_mut() {
            ag.bonds_locked = ag.bonds_locked.saturating_sub(c.bond);
            if lost {
                ag.bonds_lost += c.bond;
            } else {
                ag.bonds_returned += c.bond;
            }
            self.store.put_account(&c.hunter, &a);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn new_run(&mut self, program: &Program, commit: Option<CommitId>, mode: Mode, round: u32, spec: &RunSpec, ctx: &Ctx, payer: &str) -> Run {
        let id = self.store.next_id(IdKind::Run);
        let r = Run {
            id,
            program: program.id,
            commit,
            mode,
            round,
            request_sha256: spec.request_sha256,
            signer: ctx.signer.clone(),
            payer: payer.to_string(),
            started_at: ctx.now,
            outcome: RunOutcome::Pending,
        };
        self.store.put_run(&r);
        let mut t = self.store.totals();
        t.runs += 1;
        self.store.put_totals(&t);
        r
    }

    /// Charges the run deposit to the payer's NEAR credit and emits the request.
    fn request_run(&mut self, p: &Program, run: &Run, payer: &str) -> R<Effect> {
        self.debit_near(payer, p.params.run_deposit)?;
        Ok(Effect::RequestRun { run: run.id, mode: run.mode, deposit: p.params.run_deposit, payer: payer.to_string() })
    }

    // ------------------------------------------------------------------ accounts

    /// NEAR credit for verification and storage (NEP-145 style, spec §12.1).
    pub fn near_deposit(&mut self, account: &str, amount: Amount) -> R<Vec<Effect>> {
        let mut a = self.store.account(account);
        a.near += amount;
        self.store.put_account(account, &a);
        let mut t = self.store.totals();
        t.near_credits += amount;
        self.store.put_totals(&t);
        Ok(vec![])
    }

    pub fn near_withdraw(&mut self, ctx: &Ctx, amount: Amount) -> R<Vec<Effect>> {
        if self.store.account(&ctx.caller).agent.is_some() {
            return Err(Error::Forbidden("an agent cannot withdraw"));
        }
        self.debit_near(&ctx.caller, amount)?;
        Ok(vec![Effect::TransferNear { to: ctx.caller.clone(), amount }])
    }

    /// Storage a call added, paid from the account's NEAR credit (spec §12.1). Unlike verification, it
    /// is not an agent's verification spend.
    pub fn charge_storage(&mut self, account: &str, amount: Amount) -> R<()> {
        let mut a = self.store.account(account);
        if a.near < amount {
            return Err(Error::State("not enough NEAR credit for storage (storage_deposit)"));
        }
        a.near -= amount;
        self.store.put_account(account, &a);
        let mut t = self.store.totals();
        t.near_credits -= amount;
        self.store.put_totals(&t);
        Ok(())
    }

    /// Tokens received for an account's credit (bonds for agents, challenge bonds).
    pub fn token_deposit(&mut self, account: &str, amount: Amount) -> R<Vec<Effect>> {
        let mut t = self.store.totals();
        t.tokens_in += amount;
        self.store.put_totals(&t);
        self.credit_tokens(account, amount);
        Ok(vec![])
    }

    /// Credit, then withdraw: the only outbound token transfer (spec §7.9).
    pub fn withdraw(&mut self, ctx: &Ctx, amount: Amount) -> R<Vec<Effect>> {
        if self.store.account(&ctx.caller).agent.is_some() {
            return Err(Error::Forbidden("an agent cannot withdraw"));
        }
        if amount == 0 {
            return Err(Error::Invalid("amount must be positive".into()));
        }
        self.debit_tokens(&ctx.caller, amount)?;
        let mut t = self.store.totals();
        t.tokens_out += amount;
        self.store.put_totals(&t);
        Ok(vec![
            Effect::TransferTokens { to: ctx.caller.clone(), amount },
            ev("withdraw", serde_json::json!({"account": ctx.caller, "amount": amount.to_string()})),
        ])
    }

    /// A failed transfer is credited back.
    pub fn withdraw_failed(&mut self, account: &str, amount: Amount) -> R<Vec<Effect>> {
        let mut t = self.store.totals();
        t.tokens_out -= amount;
        self.store.put_totals(&t);
        self.credit_tokens(account, amount);
        Ok(vec![ev("withdraw_refunded", serde_json::json!({"account": account, "amount": amount.to_string()}))])
    }

    /// An owner registers one of its sub-accounts as an agent (spec §13).
    pub fn register_agent(&mut self, ctx: &Ctx, agent: &str, bond_budget: Amount, per_commit_cap: Amount) -> R<Vec<Effect>> {
        let suffix = format!(".{}", ctx.caller);
        if !agent.ends_with(&suffix) || agent.len() <= suffix.len() || agent[..agent.len() - suffix.len()].contains('.') {
            return Err(Error::Forbidden("an agent must be a direct sub-account of its owner"));
        }
        if per_commit_cap > bond_budget || bond_budget > bounds::MAX_AMOUNT {
            return Err(Error::Invalid("per_commit_cap must not exceed bond_budget".into()));
        }
        let mut a = self.store.account(agent);
        let prev = a.agent.take();
        if let Some(p) = &prev {
            if p.owner != ctx.caller {
                return Err(Error::Forbidden("another owner"));
            }
        }
        let mut ag = prev.unwrap_or(Agent {
            owner: ctx.caller.clone(),
            bond_budget,
            per_commit_cap,
            bonds_locked: 0,
            bonds_lost: 0,
            bonds_returned: 0,
            paid: 0,
            verification_spent: 0,
            commits: 0,
            containment_events: 0,
        });
        ag.bond_budget = bond_budget;
        ag.per_commit_cap = per_commit_cap;
        a.agent = Some(ag);
        self.store.put_account(agent, &a);
        Ok(vec![ev(
            "agent_registered",
            serde_json::json!({"agent": agent, "owner": ctx.caller, "bond_budget": bond_budget.to_string(), "per_commit_cap": per_commit_cap.to_string()}),
        )])
    }

    // ------------------------------------------------------------------ programs

    /// `ft_transfer_call` with a create message (spec §3.1). The escrow is credited when the anchor
    /// check completes.
    pub fn create_program(&mut self, ctx: &Ctx, np: NewProgram) -> R<(ProgramId, Vec<Effect>)> {
        bounds::check(&np.params, np.escrow, ctx.now).map_err(|e| Error::Invalid(e.into()))?;
        if np.project != ctx.signer && np.project != ctx.caller {
            return Err(Error::Forbidden("the project account must create its program"));
        }
        let id = self.store.next_id(IdKind::Program) as ProgramId;
        let p = Program {
            id,
            key: format!("{}/{}", self.contract, id),
            project: np.project,
            manifest_sha256: np.manifest_sha256,
            params: np.params,
            phase: Phase::Pending,
            anchor: AnchorStatus::Pending,
            created_at: ctx.now,
            opened_at: None,
            activation_run: None,
            sealing_pk: None,
            key_epoch: String::new(),
            known: vec![],
            known_runs: vec![],
            // Held at once; usable only after the anchor check (phase gating).
            ledger: {
                let mut l = Ledger::default();
                l.deposit(np.escrow);
                l
            },
            instant: InstantWindow::default(),
            commits: 0,
            cursor: 0,
            debtors: vec![],
            round: 0,
            round_started_at: ctx.now,
            round_manifests: vec![],
            proposal: None,
            rounds: vec![],
            ticket_rounds: vec![],
            release_permits: 0,
            claimed: vec![],
            fixed_tags: vec![],
            disclosed: vec![],
            first_claim_at: None,
            paid_first_hour: 0,
            defaulted: 0,
        };
        let mut t = self.store.totals();
        t.tokens_in += np.escrow;
        t.programs += 1;
        t.escrow += np.escrow;
        self.store.put_totals(&t);
        let mut fx = vec![ev("program_created", serde_json::json!({"program_id": id, "project": p.project, "escrow": np.escrow.to_string()}))];
        self.store.put_program(&p);
        if np.anchor_uncheckable {
            fx.extend(self.anchor_checked(id, None)?);
        } else {
            fx.push(Effect::CheckAnchor { program: id });
        }
        Ok((id, fx))
    }

    /// The light client's answer: `Some(true)` matches, `Some(false)` contradicts, `None` the call
    /// failed or the anchor is not checkable.
    pub fn anchor_checked(&mut self, program: ProgramId, ok: Option<bool>) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        if p.phase != Phase::Pending && !(p.anchor == AnchorStatus::Unconfirmed && ok == Some(true)) {
            return Err(Error::State("the anchor was already checked"));
        }
        let mut fx = vec![];
        match ok {
            Some(false) => {
                // Contradicted: reject and return the escrow to the project.
                if p.phase == Phase::Pending {
                    let e = p.ledger.return_available();
                    p.phase = Phase::Rejected;
                    p.anchor = AnchorStatus::Unconfirmed;
                    self.escrow_out(e);
                    let proj = p.project.clone();
                    self.store.put_program(&p);
                    self.credit_tokens(&proj, e);
                    return Ok(vec![ev("program_rejected", serde_json::json!({"program_id": program, "reason": "the light client contradicts the anchor"}))]);
                }
                return Err(Error::State("the anchor was already accepted"));
            }
            Some(true) => p.anchor = AnchorStatus::LightClient,
            None => p.anchor = AnchorStatus::Unconfirmed,
        }
        if p.phase == Phase::Pending {
            // The program becomes usable only now (spec §3.1).
            p.phase = Phase::Setup;
        }
        fx.push(ev("anchor_checked", serde_json::json!({"program_id": program, "anchor": format!("{:?}", p.anchor)})));
        self.store.put_program(&p);
        Ok(fx)
    }

    /// Permissionless while the block is inside the light client's window (the adapter checks that).
    pub fn recheck_anchor(&mut self, program: ProgramId) -> R<Vec<Effect>> {
        let p = self.get_program(program)?;
        if p.anchor != AnchorStatus::Unconfirmed || p.phase == Phase::Rejected {
            return Err(Error::State("only an unconfirmed anchor can be rechecked"));
        }
        Ok(vec![Effect::CheckAnchor { program }])
    }

    /// Requests the activation run (spec §11.2). The project pays.
    pub fn activate(&mut self, ctx: &Ctx, program: ProgramId, spec: RunSpec) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        if p.phase != Phase::Setup {
            return Err(Error::State("activation happens in setup"));
        }
        if p.sealing_pk.is_some() {
            return Err(Error::State("already activated"));
        }
        if let Some(r) = p.activation_run {
            let r = self.get_run(r)?;
            if r.outcome == RunOutcome::Pending && ctx.now < r.started_at + p.params.windows.run_timeout {
                return Err(Error::State("an activation run is in flight"));
            }
        }
        let run = self.new_run(&p, None, Mode::Activate, 0, &spec, ctx, &ctx.caller);
        let fx = self.request_run(&p, &run, &ctx.caller)?;
        p.activation_run = Some(run.id);
        self.store.put_program(&p);
        Ok(vec![fx])
    }

    /// Before open only: a known issue's sealed PoC is verified like a submission (spec §7.6).
    pub fn submit_known(&mut self, ctx: &Ctx, program: ProgramId, spec: RunSpec) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        if ctx.caller != p.project {
            return Err(Error::Forbidden("only the project declares known issues"));
        }
        if p.phase != Phase::Setup || p.sealing_pk.is_none() {
            return Err(Error::State("known issues are declared after activation and before open"));
        }
        if p.known.len() as u32 >= p.params.known_max_count {
            return Err(Error::State("the program's known-issue count is reached"));
        }
        if p.known_runs.len() as u32 >= 2 * bounds::MAX_KNOWN {
            return Err(Error::State("too many known-issue runs"));
        }
        let run = self.new_run(&p, None, Mode::Verify, 0, &spec, ctx, &ctx.caller);
        let fx = self.request_run(&p, &run, &ctx.caller)?;
        p.known_runs.push(run.id);
        self.store.put_program(&p);
        Ok(vec![fx])
    }

    /// Opens round 0 (spec §3.1): activation is T1 and every known issue is final.
    pub fn open_program(&mut self, ctx: &Ctx, program: ProgramId) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        if ctx.caller != p.project {
            return Err(Error::Forbidden("only the project opens its program"));
        }
        if p.phase != Phase::Setup || p.sealing_pk.is_none() {
            return Err(Error::State("open needs an activated program in setup"));
        }
        // A known-issue run still pending would add a known issue after open: wait for it, or for its
        // timeout. Only T1-confirmed known issues count (spec §7.6).
        for rid in p.known_runs.clone() {
            let r = self.get_run(rid)?;
            if r.outcome == RunOutcome::Pending && ctx.now < r.started_at + p.params.windows.run_timeout {
                return Err(Error::State("a known-issue run is still in flight"));
            }
        }
        p.phase = Phase::Open;
        p.opened_at = Some(ctx.now);
        p.round_started_at = ctx.now;
        self.store.put_program(&p);
        Ok(vec![ev("program_opened", serde_json::json!({"program_id": program, "known_issues": p.known.len()}))])
    }

    pub fn top_up(&mut self, program: ProgramId, amount: Amount) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        if !matches!(p.phase, Phase::Setup | Phase::Open) {
            return Err(Error::State("top-ups go to a program in setup or open"));
        }
        let mut t = self.store.totals();
        t.tokens_in += amount;
        self.store.put_totals(&t);
        p.ledger.deposit(amount);
        self.escrow_in(amount);
        self.store.put_program(&p);
        Ok(vec![ev("top_up", serde_json::json!({"program_id": program, "amount": amount.to_string()}))])
    }

    // ------------------------------------------------------------------ hunters

    /// A commit with its bond (spec §4.1). `bond_from_credit`: an agent's (or hunter's) bond comes from
    /// the beneficiary's token credit; otherwise it arrived with the call.
    #[allow(clippy::too_many_arguments)]
    pub fn commit(&mut self, ctx: &Ctx, program: ProgramId, round: u32, commitment: H32, terms_ok: bool, bond_paid: Amount, bond_from_credit: bool) -> R<(CommitId, Vec<Effect>)> {
        let mut p = self.get_program(program)?;
        if p.phase != Phase::Open {
            return Err(Error::State("the program is not open"));
        }
        if ctx.now >= p.params.windows.deadline {
            return Err(Error::State("the deadline has passed"));
        }
        if round != p.round {
            return Err(Error::Invalid("commits are bound to the current round".into()));
        }
        if !terms_ok {
            return Err(Error::Invalid("the terms hash does not match the program's".into()));
        }
        let bond = p.params.bond;
        let hunter = ctx.caller.clone();
        let acc = self.store.account(&hunter);
        let (beneficiary, agent) = match &acc.agent {
            Some(ag) => (ag.owner.clone(), Some(ag.clone())),
            None => (hunter.clone(), None),
        };
        if let Some(ag) = &agent {
            if !bond_from_credit {
                return Err(Error::Forbidden("an agent's bond comes from its owner's credit"));
            }
            if bond > ag.per_commit_cap || ag.bonds_locked + bond > ag.bond_budget {
                return Err(Error::Forbidden("containment: the bond exceeds the agent's cap or budget"));
            }
        }
        if bond_from_credit {
            self.debit_tokens(&beneficiary, bond)?;
        } else {
            if bond_paid != bond {
                return Err(Error::Invalid(format!("the bond is {bond}")));
            }
            let mut t = self.store.totals();
            t.tokens_in += bond;
            self.store.put_totals(&t);
        }
        self.bonds_locked(bond, 0);
        let id = self.store.next_id(IdKind::Commit);
        let c = Commit {
            id,
            program,
            index: p.commits,
            round,
            hunter: hunter.clone(),
            beneficiary,
            commitment,
            blob_sha256: None,
            bond,
            committed_at: ctx.now,
            state: CommitState::Committed,
            run: None,
            retry_until: 0,
            attempts: 0,
            second: None,
            second_agrees: false,
            challenge: None,
            rerun: None,
            ticket: false,
            wrapped: None,
        };
        self.store.set_commit_at(program, p.commits, id);
        p.commits += 1;
        self.store.put_commit(&c);
        self.store.put_program(&p);
        if agent.is_some() {
            let mut a = self.store.account(&hunter);
            if let Some(ag) = a.agent.as_mut() {
                ag.bonds_locked += bond;
                ag.commits += 1;
            }
            self.store.put_account(&hunter, &a);
        }
        Ok((id, vec![ev("commit", serde_json::json!({"program_id": program, "commit_id": id, "index": c.index, "round": round, "hunter": hunter}))]))
    }

    /// Records a refused agent commit (containment event, spec §13). The call that triggered it fails.
    pub fn containment_event(&mut self, agent: &str) {
        let mut a = self.store.account(agent);
        if let Some(ag) = a.agent.as_mut() {
            ag.containment_events += 1;
            self.store.put_account(agent, &a);
        }
    }

    /// The reveal (spec §4.2): starts the verification run, paid by the beneficiary.
    pub fn reveal(&mut self, ctx: &Ctx, commit: CommitId, commitment_check: H32, blob_sha256: H32, spec: RunSpec) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let p = self.get_program(c.program)?;
        if ctx.caller != c.hunter {
            return Err(Error::Forbidden("only the committer reveals"));
        }
        if c.state != CommitState::Committed {
            return Err(Error::State("already revealed"));
        }
        if ctx.now > c.committed_at + p.params.windows.reveal {
            return Err(Error::State("the reveal window has passed"));
        }
        if commitment_check != c.commitment {
            return Err(Error::Invalid("the blob does not match the commitment".into()));
        }
        let payer = c.beneficiary.clone();
        let run = self.new_run(&p, Some(commit), Mode::Verify, c.round, &spec, ctx, &payer);
        let fx = self.request_run(&p, &run, &payer)?;
        c.blob_sha256 = Some(blob_sha256);
        c.state = CommitState::Verifying;
        c.run = Some(run.id);
        c.retry_until = ctx.now + p.params.windows.infra_grace;
        c.attempts = 1;
        self.store.put_commit(&c);
        Ok(vec![fx, ev("reveal", serde_json::json!({"commit_id": commit, "run_id": run.id}))])
    }

    /// A new attempt after an infrastructure failure, a void verdict or a timeout (spec §6.5). Until
    /// `retry_until` only the hunter may retry; the requester pays.
    pub fn retry(&mut self, ctx: &Ctx, commit: CommitId, spec: RunSpec) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let p = self.get_program(c.program)?;
        if c.state != CommitState::Verifying {
            return Err(Error::State("only a commit in verification retries"));
        }
        if ctx.now > c.retry_until {
            return Err(Error::State("the infrastructure grace has passed: time_out closes it"));
        }
        if ctx.caller != c.hunter && ctx.caller != c.beneficiary {
            return Err(Error::Forbidden("only the hunter retries"));
        }
        if let Some(rid) = c.run {
            let r = self.get_run(rid)?;
            match &r.outcome {
                RunOutcome::Pending if ctx.now < r.started_at + p.params.windows.run_timeout => {
                    return Err(Error::State("the current run is still in flight"));
                }
                RunOutcome::Answered { facts, .. } if facts.kind != Kind::InfraError && c.challenge.is_none() => {
                    return Err(Error::State("the run answered: nothing to retry"));
                }
                _ => {}
            }
        }
        if c.challenge.is_some() {
            return Err(Error::State("a challenge is open"));
        }
        let payer = c.beneficiary.clone();
        let run = self.new_run(&p, Some(commit), Mode::Verify, c.round, &spec, ctx, &payer);
        let fx = self.request_run(&p, &run, &payer)?;
        c.run = Some(run.id);
        c.attempts += 1;
        c.second = None;
        c.second_agrees = false;
        self.store.put_commit(&c);
        Ok(vec![fx])
    }

    // ------------------------------------------------------------------ runs

    /// OutLayer answered. `output` is what near-antibody received; parse failures are the run's
    /// fault (InfraError), never the hunter's.
    pub fn run_answered(&mut self, ctx: &Ctx, run: RunId, output: &[u8], output_sha256: H32) -> R<Vec<Effect>> {
        let mut r = self.get_run(run)?;
        if r.outcome != RunOutcome::Pending {
            return Err(Error::State("stale callback"));
        }
        let mut p = self.get_program(r.program)?;
        let round_slots = self.slots_for(&p, r.round);
        let commit_id = r.commit;
        let ex = Expect {
            mode: r.mode,
            request_sha256: r.request_sha256,
            manifest_sha256: self.manifest_for_run(&p, &r),
            program_key: &p.key,
            round: r.round,
            key_epoch: &p.key_epoch,
            commit_id,
            run_id: run,
            slots: &round_slots,
        };
        let facts = match facts::parse(output, &ex) {
            Ok(f) => f,
            Err(e) => FactsSummary {
                kind: Kind::InfraError,
                keys: vec![],
                impact_usd_micro: 0,
                protected_harm_usd_micro: 0,
                receipt_sha256: H32::default(),
                reason: Some(format!("facts rejected: {}", e.chars().take(56).collect::<String>())),
                extra: Extra::None,
            },
        };
        r.outcome = RunOutcome::Answered { output_sha256, facts: facts.clone(), at: ctx.now, tier: Tier::T0 };
        self.store.put_run(&r);
        let mut fx = vec![ev(
            "run_answered",
            serde_json::json!({"run_id": run, "mode": r.mode, "kind": facts.kind, "commit_id": commit_id, "tier": "T0",
                               "keys": facts.keys, "receipt_sha256": facts.receipt_sha256}),
        )];
        fx.extend(self.after_answer(ctx, &mut p, &r)?);
        self.store.put_program(&p);
        Ok(fx)
    }

    /// The run's callback failed or carried nothing (spec §6.5).
    pub fn run_failed(&mut self, ctx: &Ctx, run: RunId, reason: &str) -> R<Vec<Effect>> {
        let mut r = self.get_run(run)?;
        if r.outcome != RunOutcome::Pending {
            return Err(Error::State("stale callback"));
        }
        r.outcome = RunOutcome::Failed { at: ctx.now, reason: reason.chars().take(64).collect() };
        self.store.put_run(&r);
        let mut fx = vec![ev("run_failed", serde_json::json!({"run_id": run, "reason": reason}))];
        if r.mode == Mode::CheckRound {
            fx.extend(self.drop_unchecked_proposal(r.program, run)?);
        }
        // A challenge's re-run that failed leaves the challenge open until `resolve`.
        if r.mode == Mode::Second {
            if let Some(cid) = r.commit {
                let mut c = self.get_commit(cid)?;
                if c.second == Some(run) {
                    c.second = None;
                    self.store.put_commit(&c);
                }
            }
        }
        fx.push(ev("retry_available", serde_json::json!({"run_id": run, "commit_id": r.commit})));
        Ok(fx)
    }

    /// A relayed quote verified on NEAR for this run (the adapter checked the binding and DCAP).
    pub fn confirm_t1(&mut self, ctx: &Ctx, run: RunId) -> R<Vec<Effect>> {
        let mut r = self.get_run(run)?;
        let (output_sha256, facts, at) = match &r.outcome {
            RunOutcome::Answered { output_sha256, facts, at, tier } => {
                if *tier == Tier::T1 {
                    return Err(Error::State("already T1"));
                }
                (*output_sha256, facts.clone(), *at)
            }
            _ => return Err(Error::State("the run has no answer to confirm")),
        };
        if facts.kind == Kind::InfraError {
            return Err(Error::State("an infrastructure error is not confirmed"));
        }
        r.outcome = RunOutcome::Answered { output_sha256, facts: facts.clone(), at, tier: Tier::T1 };
        self.store.put_run(&r);
        let mut p = self.get_program(r.program)?;
        let mut fx = vec![ev("t1_confirmed", serde_json::json!({"run_id": run, "mode": r.mode, "commit_id": r.commit}))];
        fx.extend(self.after_t1(ctx, &mut p, &r)?);
        self.store.put_program(&p);
        Ok(fx)
    }

    fn slots_for(&self, p: &Program, round: u32) -> Vec<SlotRule> {
        let mut s = p.params.slots.clone();
        if let Some(pr) = &p.proposal {
            if round >= pr.round {
                s.extend(pr.new_slots.iter().cloned());
            }
        }
        s
    }

    fn manifest_for_run(&self, p: &Program, r: &Run) -> H32 {
        match (&p.proposal, r.mode) {
            (Some(pr), Mode::Rerun | Mode::CheckRound) => pr.manifest_sha256,
            _ => p.manifest_of(r.round),
        }
    }

    /// What an answer changes at once. Verdicts wait for allocation; mode-specific runs act here.
    fn after_answer(&mut self, _ctx: &Ctx, p: &mut Program, r: &Run) -> R<Vec<Effect>> {
        let facts = r.facts().cloned().unwrap();
        let mut fx = vec![];
        match r.mode {
            Mode::Verify | Mode::Second => {
                if let Some(cid) = r.commit {
                    let mut c = self.get_commit(cid)?;
                    if r.mode == Mode::Second && c.second == Some(r.id) {
                        // The primary verdict may have been voided while this run was in flight.
                        let primary_facts = c.run.and_then(|rid| self.store.run(rid)).and_then(|pr| pr.facts().cloned());
                        let agrees = primary_facts.map(|pf| facts::agree(&pf, &facts)).unwrap_or(false);
                        if agrees {
                            c.second_agrees = true;
                        } else if facts.kind != Kind::InfraError {
                            // A disagreement voids the verdict: back to verification, priority kept.
                            c.second = None;
                            c.second_agrees = false;
                            fx.push(ev("verdict_void", serde_json::json!({"commit_id": cid, "why": "the second run disagrees"})));
                            c.run = None;
                            c.retry_until = _ctx.now + p.params.windows.infra_grace;
                        } else {
                            c.second = None;
                        }
                        self.store.put_commit(&c);
                    }
                    if let Some(ch) = &c.challenge {
                        if ch.run == Some(r.id) && facts.kind == Kind::InfraError {
                            let mut c2 = c.clone();
                            if let Some(ch2) = c2.challenge.as_mut() {
                                ch2.run = None;
                            }
                            self.store.put_commit(&c2);
                        }
                    }
                }
            }
            Mode::CheckRound => {
                if let Some(pr) = p.proposal.as_mut() {
                    if pr.check_run == r.id {
                        match facts.extra {
                            Extra::RoundCheck { ok: true, canaries_ok } => pr.canaries_ok = canaries_ok,
                            // Rejected, or no verdict at all (an infrastructure error): the project
                            // may propose again.
                            _ => {
                                fx.push(ev("round_rejected", serde_json::json!({"program_id": p.id, "round": pr.round})));
                                p.proposal = None;
                            }
                        }
                    }
                }
            }
            Mode::Disclose => {}
            Mode::ReleaseWrap => {
                if let Some(cid) = r.commit {
                    let mut c = self.get_commit(cid)?;
                    if let Extra::Wrap { wrap } = &facts.extra {
                        c.wrapped = Some(r.id);
                        fx.push(ev("release_wrap", serde_json::json!({"program_id": p.id, "commit_id": cid, "wrap": wrap})));
                        self.store.put_commit(&c);
                    }
                }
            }
            Mode::Activate | Mode::Rerun | Mode::Distinct => {}
        }
        Ok(fx)
    }

    fn after_t1(&mut self, ctx: &Ctx, p: &mut Program, r: &Run) -> R<Vec<Effect>> {
        let facts = r.facts().cloned().unwrap();
        let mut fx = vec![];
        match r.mode {
            Mode::Activate if p.activation_run == Some(r.id) && p.sealing_pk.is_none() => {
                if let Extra::SealingKey { pk, epoch } = facts.extra {
                    p.sealing_pk = Some(pk);
                    p.key_epoch = epoch.clone();
                    fx.push(ev("program_activated", serde_json::json!({"program_id": p.id, "sealing_pk": pk, "key_epoch": epoch})));
                }
            }
            Mode::Verify | Mode::Second => {
                if let Some(cid) = r.commit {
                    let c = self.get_commit(cid)?;
                    // A challenge is settled by any T1 result for its request (spec §6.4).
                    if c.challenge.is_some() {
                        fx.extend(self.settle_challenge(ctx, p, cid, r)?);
                    }
                } else if p.known_runs.contains(&r.id) && p.phase == Phase::Setup {
                    fx.extend(self.known_answered(p, r)?);
                }
            }
            Mode::Disclose => {
                // Disclosure keys count only with T1: a wrong key from a lying operator must not mark a
                // tag disclosed (spec §11.6).
                if let Extra::Disclosed { keys } = &facts.extra {
                    for (tag, key) in keys {
                        if self.disclosure_condition(p, tag, ctx.now) && !p.disclosed.iter().any(|(t, _)| t == tag) {
                            p.disclosed.push((*tag, *key));
                            fx.push(ev("disclosed", serde_json::json!({"program_id": p.id, "tag": tag, "key": key})));
                        }
                    }
                }
            }
            Mode::Distinct => {
                if let (Some(cid), Extra::Distinct { distinct: true }) = (r.commit, &facts.extra) {
                    fx.extend(self.distinct_top_up(ctx, p, cid)?);
                }
            }
            _ => {}
        }
        Ok(fx)
    }

    fn known_answered(&mut self, p: &mut Program, r: &Run) -> R<Vec<Effect>> {
        let f = r.facts().unwrap();
        if f.kind != Kind::Violation {
            return Ok(vec![ev("known_issue_rejected", serde_json::json!({"program_id": p.id, "run_id": r.id, "kind": f.kind}))]);
        }
        let severity = f.keys.iter().filter_map(|k| p.params.slot(k.slot)).map(|s| s.severity).max().unwrap_or(Severity::Low);
        let share: Amount = p.known.iter().map(|k| p.params.base(k.severity)).sum::<Amount>() + p.params.base(severity);
        let cap = arith::bps_of(p.ledger.deposits, p.params.known_max_share_bps);
        if share > cap {
            return Ok(vec![ev("known_issue_rejected", serde_json::json!({"program_id": p.id, "run_id": r.id, "reason": "over the known-issue cap"}))]);
        }
        p.known.push(KnownIssue {
            run: r.id,
            tags: f.keys.iter().map(|k| k.tag).collect(),
            slots: f.keys.iter().map(|k| k.slot).collect(),
            severity,
            impact_usd_micro: f.impact_usd_micro,
            fixed: false,
        });
        Ok(vec![ev(
            "known_issue",
            serde_json::json!({"program_id": p.id, "run_id": r.id, "slots": f.keys.iter().map(|k| k.slot).collect::<Vec<_>>(),
                               "severity": severity, "impact_usd_micro": f.impact_usd_micro.to_string()}),
        )])
    }

    // ------------------------------------------------------------------ finality (spec §6.2)

    fn nominal(&self, p: &Program, f: &FactsSummary, only: Option<&[H32]>) -> (Amount, Option<u8>) {
        let mut best: Option<(Severity, u8)> = None;
        for k in &f.keys {
            if let Some(o) = only {
                if !o.contains(&k.tag) {
                    continue;
                }
            }
            if let Some(s) = p.params.slot(k.slot).or_else(|| p.proposal.as_ref().and_then(|pr| pr.new_slots.iter().find(|x| x.slot == k.slot))) {
                if best.map(|(b, _)| s.severity > b).unwrap_or(true) {
                    best = Some((s.severity, k.slot));
                }
            }
        }
        match best {
            Some((sev, slot)) => (p.params.base(sev) + p.params.impact_bonus(f.impact_usd_micro), Some(slot)),
            None => (0, None),
        }
    }

    /// Whether a commit's current verdict is final.
    pub fn is_final(&self, now: Time, p: &Program, c: &Commit) -> bool {
        let Some(rid) = c.run else { return false };
        let Ok(r) = self.get_run(rid) else { return false };
        let RunOutcome::Answered { facts, at, tier, .. } = &r.outcome else { return false };
        if facts.kind == Kind::InfraError || c.challenge.is_some() {
            return false;
        }
        let above_cap = facts.kind == Kind::Violation && self.nominal(p, facts, None).0 > p.params.instant_cap;
        (*tier == Tier::T1 && !above_cap) || (now > at + p.params.windows.challenge && (!above_cap || c.second_agrees))
    }

    /// Requests the independent second run of a verdict above the cap (spec §6.2). The requester pays.
    pub fn request_second(&mut self, ctx: &Ctx, commit: CommitId, spec: RunSpec) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let p = self.get_program(c.program)?;
        if c.state != CommitState::Verifying || c.second.is_some() || c.second_agrees {
            return Err(Error::State("no second run is needed now"));
        }
        let primary = self.get_run(c.run.ok_or(Error::State("no verdict"))?)?;
        let f = primary.facts().ok_or(Error::State("no verdict yet"))?;
        if f.kind != Kind::Violation || self.nominal(&p, f, None).0 <= p.params.instant_cap {
            return Err(Error::State("only a violation above the instant cap needs a second run"));
        }
        let run = self.new_run(&p, Some(commit), Mode::Second, c.round, &spec, ctx, &ctx.caller);
        let fx = self.request_run(&p, &run, &ctx.caller)?;
        c.second = Some(run.id);
        self.store.put_commit(&c);
        Ok(vec![fx])
    }

    // ------------------------------------------------------------------ challenges (spec §6.4)

    pub fn challenge(&mut self, ctx: &Ctx, commit: CommitId, bond_paid: Amount, spec: RunSpec) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let p = self.get_program(c.program)?;
        if c.state != CommitState::Verifying || c.challenge.is_some() {
            return Err(Error::State("nothing to challenge"));
        }
        let r = self.get_run(c.run.ok_or(Error::State("no verdict"))?)?;
        let RunOutcome::Answered { facts, at, .. } = &r.outcome else { return Err(Error::State("no verdict yet")) };
        if facts.kind == Kind::InfraError {
            return Err(Error::State("an infrastructure error is retried, not challenged"));
        }
        if self.is_final(ctx.now, &p, &c) || ctx.now > at + p.params.windows.challenge {
            return Err(Error::State("the verdict is final"));
        }
        if bond_paid != p.params.challenge_bond {
            return Err(Error::Invalid(format!("the challenge bond is {}", p.params.challenge_bond)));
        }
        let mut t = self.store.totals();
        t.tokens_in += bond_paid;
        t.challenge_bonds_locked += bond_paid;
        self.store.put_totals(&t);
        let run = self.new_run(&p, Some(commit), Mode::Second, c.round, &spec, ctx, &ctx.caller);
        let fx = self.request_run(&p, &run, &ctx.caller)?;
        c.challenge = Some(Challenge { by: ctx.caller.clone(), bond: bond_paid, at: ctx.now, run: Some(run.id) });
        self.store.put_commit(&c);
        Ok(vec![fx, ev("challenge", serde_json::json!({"commit_id": commit, "by": ctx.caller, "run_id": run.id}))])
    }

    /// A T1 result for the challenged request: equal facts uphold, different facts replace.
    fn settle_challenge(&mut self, ctx: &Ctx, p: &mut Program, commit: CommitId, t1run: &Run) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let ch = c.challenge.clone().unwrap();
        let primary = self.get_run(c.run.unwrap())?;
        let t1facts = t1run.facts().unwrap();
        let upheld = primary.facts().map(|pf| facts::agree(pf, t1facts)).unwrap_or(false);
        let mut t = self.store.totals();
        t.challenge_bonds_locked -= ch.bond;
        self.store.put_totals(&t);
        let mut fx = vec![];
        if upheld {
            // The T1 run confirms the verdict.
            if t1run.id != primary.id {
                let mut pr = primary.clone();
                if let RunOutcome::Answered { output_sha256, facts, at, .. } = pr.outcome.clone() {
                    pr.outcome = RunOutcome::Answered { output_sha256, facts, at, tier: Tier::T1 };
                }
                self.store.put_run(&pr);
            }
            c.second_agrees = true;
            if ch.by == c.hunter || ch.by == c.beneficiary {
                // The hunter challenged their own adverse verdict and lost: the bond goes to escrow.
                p.ledger.award(ch.bond);
                self.escrow_in(ch.bond);
            } else {
                self.credit_tokens(&c.beneficiary.clone(), ch.bond);
            }
            fx.push(ev("challenge_upheld", serde_json::json!({"commit_id": commit, "run_id": t1run.id})));
        } else {
            // Different facts: the T1 run's facts replace the verdict.
            c.run = Some(t1run.id);
            c.second_agrees = false;
            c.second = None;
            self.credit_tokens(&ch.by, ch.bond);
            fx.push(ev("challenge_overturned", serde_json::json!({"commit_id": commit, "run_id": t1run.id})));
        }
        c.challenge = None;
        let _ = ctx;
        self.store.put_commit(&c);
        Ok(fx)
    }

    /// Permissionless timeouts (spec §14.1): a challenge without T1 voids the verdict; a commit past
    /// its infrastructure grace closes with a refund.
    pub fn time_out(&mut self, ctx: &Ctx, commit: CommitId) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let p = self.get_program(c.program)?;
        let mut fx = vec![];
        if let Some(ch) = c.challenge.clone() {
            if ctx.now <= ch.at + p.params.windows.resolve {
                return Err(Error::State("the challenge can still be resolved"));
            }
            let mut t = self.store.totals();
            t.challenge_bonds_locked -= ch.bond;
            self.store.put_totals(&t);
            self.credit_tokens(&ch.by, ch.bond);
            c.challenge = None;
            c.run = None;
            c.second = None;
            c.second_agrees = false;
            c.retry_until = ctx.now + p.params.windows.infra_grace;
            self.store.put_commit(&c);
            return Ok(vec![ev("verdict_void", serde_json::json!({"commit_id": commit, "why": "no T1 result for the challenge"}))]);
        }
        if c.state != CommitState::Verifying {
            return Err(Error::State("nothing to time out"));
        }
        let infra = match c.run {
            None => true,
            Some(rid) => {
                let r = self.get_run(rid)?;
                match &r.outcome {
                    RunOutcome::Pending => {
                        if ctx.now >= r.started_at + p.params.windows.run_timeout {
                            let mut r2 = r.clone();
                            r2.outcome = RunOutcome::Failed { at: ctx.now, reason: "run timeout".into() };
                            self.store.put_run(&r2);
                            fx.push(ev("run_failed", serde_json::json!({"run_id": rid, "reason": "run timeout"})));
                        }
                        true
                    }
                    RunOutcome::Failed { .. } => true,
                    RunOutcome::Answered { facts, .. } => facts.kind == Kind::InfraError,
                }
            }
        };
        if !infra {
            if fx.is_empty() {
                return Err(Error::State("the commit has a verdict"));
            }
            return Ok(fx);
        }
        if ctx.now <= c.retry_until {
            if fx.is_empty() {
                return Err(Error::State("the infrastructure grace has not passed"));
            }
            return Ok(fx);
        }
        // Void after the grace: bond refunded, nothing paid.
        self.refund_bond(&c);
        c.state = CommitState::Done(End::Refunded { reason: "no verdict within the infrastructure grace".into() });
        self.store.put_commit(&c);
        fx.push(ev("commit_void", serde_json::json!({"commit_id": commit})));
        self.store.put_program(&p);
        Ok(fx)
    }

    // ------------------------------------------------------------------ allocation (spec §7)

    /// Advances the cursor over up to `max` commits. Permissionless.
    pub fn allocate(&mut self, ctx: &Ctx, program: ProgramId, max: u32) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        let mut fx = vec![];
        let mut n = 0;
        while n < max && p.cursor < p.commits {
            let cid = self.store.commit_at(program, p.cursor).ok_or(Error::NotFound("commit order"))?;
            let mut c = self.get_commit(cid)?;
            if c.round > p.round {
                break;
            }
            match &c.state {
                CommitState::Committed => break,
                CommitState::Verifying => {
                    if !self.is_final(ctx.now, &p, &c) {
                        break;
                    }
                    fx.extend(self.allocate_one(ctx, &mut p, &mut c)?);
                    self.store.put_commit(&c);
                }
                _ => {}
            }
            p.cursor += 1;
            n += 1;
        }
        self.store.put_program(&p);
        if n == 0 && fx.is_empty() {
            return Err(Error::State("nothing to allocate"));
        }
        Ok(fx)
    }

    /// An unrevealed commit past its reveal window forfeits its bond to the escrow (spec §4.2).
    /// Permissionless.
    pub fn expire(&mut self, ctx: &Ctx, commit: CommitId) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let mut p = self.get_program(c.program)?;
        if c.state != CommitState::Committed || ctx.now <= c.committed_at + p.params.windows.reveal {
            return Err(Error::State("only an unrevealed commit past its reveal window expires"));
        }
        self.forfeit_bond(&mut p, &c);
        c.state = CommitState::Done(End::Expired);
        self.store.put_commit(&c);
        self.store.put_program(&p);
        Ok(vec![ev("expired", serde_json::json!({"commit_id": commit}))])
    }

    /// Funds the shortfall queue in commit order from what is available, for up to `max` debtors
    /// (spec §7.8). Permissionless.
    pub fn settle_debts(&mut self, program: ProgramId, max: u32) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        let fx = self.fund_debts(&mut p, max)?;
        if fx.is_empty() {
            return Err(Error::State("no debt can be funded now"));
        }
        self.store.put_program(&p);
        Ok(fx)
    }

    fn allocate_one(&mut self, ctx: &Ctx, p: &mut Program, c: &mut Commit) -> R<Vec<Effect>> {
        let r = self.get_run(c.run.unwrap())?;
        let f = r.facts().unwrap().clone();
        let blocked = !p.debtors.is_empty();
        let cid = c.id;
        match f.kind {
            Kind::Violation => {
                let tags: Vec<H32> = f.keys.iter().map(|k| k.tag).collect();
                if tags.iter().all(|t| p.known_tag(t)) {
                    self.refund_bond(c);
                    c.state = CommitState::Done(End::Known);
                    return Ok(vec![ev("known_match", serde_json::json!({"commit_id": cid}))]);
                }
                let new_keys: Vec<H32> =
                    tags.iter().copied().filter(|t| !p.known_tag(t) && self.store.holder(p.id, c.round, t).is_none()).collect();
                self.refund_bond(c);
                if new_keys.is_empty() {
                    // Duplicate of the earliest holder of its highest-severity key.
                    let (want, _) = self.nominal(p, &f, None);
                    let mut of = cid;
                    let mut best: Option<Severity> = None;
                    for k in &f.keys {
                        if let Some(h) = self.store.holder(p.id, c.round, &k.tag) {
                            let sev = p.params.slot(k.slot).map(|s| s.severity).unwrap_or(Severity::Low);
                            if best.map(|b| sev > b || (sev == b && h < of)).unwrap_or(true) {
                                best = Some(sev);
                                of = h;
                            }
                        }
                    }
                    let (reserved, _) = arith::cover(want, p.ledger.available, blocked);
                    p.ledger.reserve(reserved);
                    c.state = CommitState::Duplicate { of, amount: want, reserved };
                    return Ok(vec![ev("duplicate", serde_json::json!({"commit_id": cid, "of": of, "reserved": reserved.to_string()}))]);
                }
                let (nominal, slot) = self.nominal(p, &f, Some(&new_keys));
                let slot = slot.unwrap();
                let budget = p.params.slot(slot).map(|s| s.budget).unwrap_or(0);
                let spent = self.store.spent(p.id, c.round, slot);
                let amount = arith::within_budget(nominal, budget, spent);
                self.store.set_spent(p.id, c.round, slot, spent + amount);
                for t in &new_keys {
                    self.store.set_holder(p.id, c.round, t, Some(cid));
                    if !p.claimed.iter().any(|(x, _, _)| x == t) {
                        p.claimed.push((*t, ctx.now, false));
                    }
                }
                if p.first_claim_at.is_none() {
                    p.first_claim_at = Some(ctx.now);
                }
                if amount == 0 {
                    c.state = CommitState::Done(End::Exhausted);
                    return Ok(vec![ev("holder", serde_json::json!({"commit_id": cid, "amount": "0", "exhausted": true}))]);
                }
                let (reserved, debt) = arith::cover(amount, p.ledger.available, blocked);
                p.ledger.reserve(reserved);
                if debt > 0 {
                    p.debtors.push(cid);
                }
                c.state = CommitState::Holder { amount, reserved, debt, new_keys: new_keys.clone() };
                Ok(vec![ev(
                    "holder",
                    serde_json::json!({"commit_id": cid, "amount": amount.to_string(), "reserved": reserved.to_string(), "debt": debt.to_string(), "keys": new_keys}),
                )])
            }
            Kind::NoViolation => {
                if let Some(g) = p.params.gap.clone() {
                    if f.protected_harm_usd_micro > g.threshold_usd_micro {
                        self.refund_bond(c);
                        let (reserved, _) = arith::cover(g.claim_amount, p.ledger.available, blocked);
                        p.ledger.reserve(reserved);
                        c.state = CommitState::GapClaim { amount: g.claim_amount, reserved };
                        return Ok(vec![ev("gap_claim", serde_json::json!({"commit_id": cid, "harm_usd_micro": f.protected_harm_usd_micro.to_string()}))]);
                    }
                }
                self.forfeit_bond(p, c);
                c.state = CommitState::Done(End::Forfeited);
                Ok(vec![ev("forfeited", serde_json::json!({"commit_id": cid, "kind": "NoViolation"}))])
            }
            Kind::InvalidPoC => {
                self.forfeit_bond(p, c);
                c.state = CommitState::Done(End::Forfeited);
                Ok(vec![ev("forfeited", serde_json::json!({"commit_id": cid, "kind": "InvalidPoC"}))])
            }
            Kind::Unevaluable => match p.params.unevaluable.clone() {
                UnevaluablePolicy::Refund => {
                    self.refund_bond(c);
                    c.state = CommitState::Done(End::Refunded { reason: "unevaluable".into() });
                    Ok(vec![ev("unevaluable", serde_json::json!({"commit_id": cid}))])
                }
                UnevaluablePolicy::Violation { severity } => {
                    self.refund_bond(c);
                    let amount = p.params.base(severity);
                    let (reserved, debt) = arith::cover(amount, p.ledger.available, blocked);
                    p.ledger.reserve(reserved);
                    if debt > 0 {
                        p.debtors.push(cid);
                    }
                    c.state = CommitState::Holder { amount, reserved, debt, new_keys: vec![] };
                    Ok(vec![ev("holder", serde_json::json!({"commit_id": cid, "amount": amount.to_string(), "unevaluable": true}))])
                }
            },
            Kind::ProgramMisconfigured => {
                self.refund_bond(c);
                c.state = CommitState::Done(End::Refunded { reason: "program misconfigured".into() });
                p.phase = Phase::Closed;
                Ok(vec![ev("program_closed", serde_json::json!({"program_id": p.id, "commit_id": cid}))])
            }
            Kind::InfraError => Err(Error::State("an infrastructure error is not allocated")),
        }
    }

    /// Funds debts in commit order from what is available (spec §7.8). Never a pro-rata cut: the
    /// first debtor takes everything it can before the next one gets anything.
    fn fund_debts(&mut self, p: &mut Program, max: u32) -> R<Vec<Effect>> {
        let mut fx = vec![];
        let mut n = 0;
        while let Some(&cid) = p.debtors.first() {
            if p.ledger.available == 0 || n >= max {
                break;
            }
            n += 1;
            let mut c = self.get_commit(cid)?;
            let (m, _) = arith::cover(c.state.debt(), p.ledger.available, false);
            p.ledger.reserve(m);
            match &mut c.state {
                CommitState::Holder { reserved, debt, .. } | CommitState::Promoted { reserved, debt, .. } => {
                    *reserved += m;
                    *debt -= m;
                }
                _ => {}
            }
            let done = c.state.debt() == 0;
            self.store.put_commit(&c);
            fx.push(ev("debt_funded", serde_json::json!({"commit_id": cid, "amount": m.to_string()})));
            if done {
                p.debtors.remove(0);
            } else {
                break;
            }
        }
        Ok(fx)
    }

    // ------------------------------------------------------------------ payment (spec §6.3, §7.9)

    pub fn pay(&mut self, ctx: &Ctx, commit: CommitId) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let mut p = self.get_program(c.program)?;
        let (amount, reserved, debt, promoted) = match &c.state {
            CommitState::Holder { amount, reserved, debt, .. } => (*amount, *reserved, *debt, false),
            CommitState::Promoted { amount, reserved, debt, .. } => (*amount, *reserved, *debt, true),
            _ => return Err(Error::State("only a holder or a promoted claim is paid")),
        };
        if debt > 0 || reserved < amount {
            return Err(Error::State("the claim is not funded yet (top_up)"));
        }
        let mut instant = false;
        if !promoted {
            let r = self.get_run(c.run.unwrap())?;
            let (at, tier) = (r.answered_at().unwrap(), r.tier().unwrap());
            let window_ok = ctx.now > at + p.params.windows.challenge;
            let pp = &p.params;
            let next = if tier == Tier::T1 {
                let index = arith::window_index(ctx.now, pp.instant_window);
                arith::instant(index, &p.instant, amount, pp.instant_cap, pp.instant_budget)
            } else {
                None
            };
            if next.is_none() && !window_ok {
                return Err(Error::State("waits its window: no T1, above the cap, or the instant budget is spent"));
            }
            if let (Some(w), false) = (next, window_ok) {
                instant = true;
                p.instant = w;
            }
        }
        let fee = arith::protocol_fee(amount, p.params.protocol_fee_bps, p.ledger.available);
        p.ledger.pay(amount);
        p.ledger.charge(fee);
        self.escrow_out(amount + fee);
        if let Some(opened) = p.opened_at {
            if ctx.now <= opened + 3600 {
                p.paid_first_hour += amount;
            }
        }
        let tags: Vec<H32> = match &c.state {
            CommitState::Holder { new_keys, .. } => new_keys.clone(),
            _ => c.run.and_then(|r| self.store.run(r)).and_then(|r| r.facts().map(|f| f.keys.iter().map(|k| k.tag).collect())).unwrap_or_default(),
        };
        for t in &tags {
            if let Some(e) = p.claimed.iter_mut().find(|(x, _, _)| x == t) {
                e.2 = true;
            }
        }
        let beneficiary = c.beneficiary.clone();
        self.credit_tokens(&beneficiary, amount);
        self.credit_tokens(TREASURY, fee);
        if c.hunter != c.beneficiary {
            let mut a = self.store.account(&c.hunter);
            if let Some(ag) = a.agent.as_mut() {
                ag.paid += amount;
                self.store.put_account(&c.hunter, &a);
            }
        }
        c.state = CommitState::Done(End::Paid { amount, instant, at: ctx.now });
        self.store.put_commit(&c);
        self.store.put_program(&p);
        Ok(vec![ev(
            "paid",
            serde_json::json!({"program_id": p.id, "commit_id": commit, "beneficiary": beneficiary, "amount": amount.to_string(),
                               "fee": fee.to_string(), "instant": instant}),
        )])
    }

    // ------------------------------------------------------------------ pause tickets (spec §10)

    pub fn pause_ticket(&mut self, ctx: &Ctx, commit: CommitId) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let mut p = self.get_program(c.program)?;
        if p.phase != Phase::Open || p.anchor != AnchorStatus::LightClient {
            return Err(Error::State("pauses need an open program with a light-client anchor"));
        }
        let new_keys = match &c.state {
            CommitState::Holder { new_keys, .. } => new_keys.clone(),
            CommitState::Done(End::Paid { .. }) | CommitState::Done(End::Exhausted) => vec![],
            _ => return Err(Error::State("only a first finding pauses")),
        };
        if c.round != p.round || c.ticket || p.ticket_rounds.contains(&p.round) {
            return Err(Error::State("one pause ticket per round"));
        }
        let r = self.get_run(c.run.unwrap())?;
        if r.tier() != Some(Tier::T1) {
            return Err(Error::State("a pause needs a T1-confirmed verdict"));
        }
        let f = r.facts().unwrap();
        let held: Vec<&crate::facts::FactKey> = f
            .keys
            .iter()
            .filter(|k| new_keys.is_empty() || new_keys.contains(&k.tag))
            .filter(|k| self.store.holder(p.id, c.round, &k.tag) == Some(commit))
            .collect();
        if !held.iter().any(|k| p.params.pause_slots.contains(&k.slot)) {
            return Err(Error::State("no key of this finding is in the pause policy"));
        }
        if f.impact_usd_micro < p.params.pause_min_impact_usd_micro {
            return Err(Error::State("the proven impact is below the pause policy's minimum"));
        }
        c.ticket = true;
        p.ticket_rounds.push(p.round);
        self.store.put_commit(&c);
        self.store.put_program(&p);
        let _ = ctx;
        Ok(vec![
            Effect::PausePermit { program: p.id, round: p.round, ticket: commit },
            ev("pause_ticket", serde_json::json!({"program_id": p.id, "round": p.round, "ticket": commit})),
        ])
    }

    // ------------------------------------------------------------------ release wrap (spec §11.4)

    pub fn release_wrap(&mut self, ctx: &Ctx, commit: CommitId, spec: RunSpec) -> R<Vec<Effect>> {
        let c = self.get_commit(commit)?;
        let p = self.get_program(c.program)?;
        if !matches!(c.state, CommitState::Done(End::Paid { .. })) {
            return Err(Error::State("only a paid finding is wrapped for the project"));
        }
        if c.wrapped.is_some() {
            return Err(Error::State("already wrapped"));
        }
        let run = self.new_run(&p, Some(commit), Mode::ReleaseWrap, c.round, &spec, ctx, &ctx.caller);
        let fx = self.request_run(&p, &run, &ctx.caller)?;
        Ok(vec![fx])
    }

    // ------------------------------------------------------------------ rounds (spec §8)

    pub fn propose_round(&mut self, ctx: &Ctx, program: ProgramId, manifest_sha256: H32, overrides_sha256: H32, new_slots: Vec<SlotRule>, spec: RunSpec) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        if ctx.caller != p.project {
            return Err(Error::Forbidden("only the project proposes a round"));
        }
        if p.phase != Phase::Open || p.proposal.is_some() {
            return Err(Error::State("one proposal at a time, in an open program"));
        }
        if ctx.now >= p.params.windows.deadline {
            return Err(Error::State("the deadline has passed"));
        }
        let all_slots = self.slots_for(&p, p.round);
        for s in &new_slots {
            if all_slots.iter().any(|x| x.slot == s.slot) {
                return Err(Error::Invalid("a round may only add new slots".into()));
            }
            if s.budget > bounds::MAX_AMOUNT {
                return Err(Error::Invalid("slot budget out of bounds".into()));
            }
        }
        if all_slots.len() + new_slots.len() > bounds::MAX_SLOTS {
            return Err(Error::Invalid("too many slots".into()));
        }
        let round = p.round + 1;
        p.proposal = Some(Proposal {
            round,
            manifest_sha256,
            overrides_sha256,
            new_slots,
            check_run: 0,
            proposed_at: ctx.now,
            canaries_ok: false,
        });
        let run = self.new_run(&p, None, Mode::CheckRound, round, &spec, ctx, &ctx.caller);
        let fx = self.request_run(&p, &run, &ctx.caller)?;
        p.proposal.as_mut().unwrap().check_run = run.id;
        self.store.put_program(&p);
        Ok(vec![fx, ev("round_proposed", serde_json::json!({"program_id": program, "round": round, "run_id": run.id}))])
    }

    /// A proposal whose round check failed without a verdict is dropped, so the project can propose
    /// again (one proposal at a time would otherwise block the program's rounds for good).
    fn drop_unchecked_proposal(&mut self, program: ProgramId, run: RunId) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        let Some(pr) = p.proposal.as_ref().filter(|pr| pr.check_run == run) else { return Ok(vec![]) };
        let fx = vec![ev("round_rejected", serde_json::json!({"program_id": program, "round": pr.round, "run_id": run}))];
        p.proposal = None;
        self.store.put_program(&p);
        Ok(fx)
    }

    /// When the proposal's round check was answered, if it is T1-confirmed and valid. Structural facts
    /// need T1 (spec §6.2).
    fn proposal_accepted_at(&self, p: &Program) -> Option<Time> {
        let pr = p.proposal.as_ref()?;
        let r = self.get_run(pr.check_run).ok()?;
        match &r.outcome {
            RunOutcome::Answered { facts, at, tier: Tier::T1, .. } if matches!(facts.extra, Extra::RoundCheck { ok: true, .. }) => Some(*at),
            _ => None,
        }
    }

    /// Immune memory and appeals (spec §8.2): permissionless, requester pays.
    pub fn request_rerun(&mut self, ctx: &Ctx, commit: CommitId, spec: RunSpec) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let p = self.get_program(c.program)?;
        if p.phase != Phase::Open {
            return Err(Error::State("the program is not open"));
        }
        let Some(accepted_at) = self.proposal_accepted_at(&p) else {
            return Err(Error::State("no T1-confirmed round proposal"));
        };
        if ctx.now > accepted_at + p.params.windows.rerun_window {
            return Err(Error::State("the re-run window has passed"));
        }
        // The round's first findings, duplicates and gap claims; every paid PoC; every commit that ever
        // held a key (immune memory, spec §8.2).
        let of_round = c.round == p.round
            && matches!(
                c.state,
                CommitState::Holder { .. }
                    | CommitState::Duplicate { .. }
                    | CommitState::GapClaim { .. }
                    | CommitState::Done(End::Paid { .. })
                    | CommitState::Done(End::Exhausted)
            );
        let paid = matches!(c.state, CommitState::Done(End::Paid { .. }));
        if !(of_round || paid || self.ever_held(&p, &c, p.round)) {
            return Err(Error::State("re-runs are for the round's findings, paid PoCs and key holders"));
        }
        let pr = p.proposal.clone().unwrap();
        if let Some(r) = self.rerun_at(&c, pr.round) {
            let stuck = matches!(r.outcome, RunOutcome::Failed { .. })
                || matches!(&r.outcome, RunOutcome::Answered { facts, .. } if facts.kind == Kind::InfraError)
                || (r.outcome == RunOutcome::Pending && ctx.now >= r.started_at + p.params.windows.run_timeout);
            if !stuck {
                return Err(Error::State("already re-run at this fix"));
            }
        }
        let run = self.new_run(&p, Some(commit), Mode::Rerun, pr.round, &spec, ctx, &ctx.caller);
        let fx = self.request_run(&p, &run, &ctx.caller)?;
        c.rerun = Some(run.id);
        self.store.put_commit(&c);
        Ok(vec![fx, ev("rerun_requested", serde_json::json!({"commit_id": commit, "run_id": run.id, "round": pr.round}))])
    }

    /// Whether a commit holds a key in round `r` (it was carried or promoted into it).
    fn holds_in(&self, p: &Program, c: &Commit, r: u32) -> bool {
        c.run
            .and_then(|rid| self.store.run(rid))
            .and_then(|run| run.facts().map(|f| f.keys.iter().any(|k| self.store.holder(p.id, r, &k.tag) == Some(c.id))))
            .unwrap_or(false)
    }

    /// The T1-confirmed re-run result of a commit at the proposal: `Some(true)` still breaks `tags`,
    /// `Some(false)` fixed, `None` no T1 result (not requested, in flight, failed, or T0 only).
    /// Whether a commit held a key in any round up to `r`.
    fn ever_held(&self, p: &Program, c: &Commit, r: u32) -> bool {
        (0..=r).any(|x| self.holds_in(p, c, x))
    }

    /// The commit's re-run at the fix proposed for `round`: a re-run counts for its own proposal only.
    fn rerun_at(&self, c: &Commit, round: u32) -> Option<Run> {
        self.get_run(c.rerun?).ok().filter(|r| r.round == round)
    }

    /// Whether the commit's T1 re-run at the fix for `round` still breaks one of `tags`.
    fn rerun_breaks(&self, c: &Commit, tags: &[H32], round: u32) -> Option<bool> {
        let r = self.rerun_at(c, round)?;
        match &r.outcome {
            RunOutcome::Answered { facts, tier: Tier::T1, .. } if facts.kind != Kind::InfraError => {
                Some(facts.kind == Kind::Violation && facts.keys.iter().any(|k| tags.contains(&k.tag)))
            }
            _ => None,
        }
    }

    /// Whether the commit's T1 re-run at the fix for `round` shows no violation at all.
    fn rerun_clean(&self, c: &Commit, round: u32) -> Option<bool> {
        let r = self.rerun_at(c, round)?;
        match &r.outcome {
            RunOutcome::Answered { facts, tier: Tier::T1, .. } if facts.kind != Kind::InfraError => Some(facts.kind != Kind::Violation),
            _ => None,
        }
    }

    /// The gap claim's T1 re-run breaks an invariant this round adds.
    fn rerun_breaks_new_slot(&self, p: &Program, c: &Commit) -> Option<bool> {
        let pr = p.proposal.as_ref()?;
        let r = self.rerun_at(c, pr.round)?;
        match &r.outcome {
            RunOutcome::Answered { facts, tier: Tier::T1, .. } if facts.kind != Kind::InfraError => {
                Some(facts.kind == Kind::Violation && facts.keys.iter().any(|k| pr.new_slots.iter().any(|s| s.slot == k.slot)))
            }
            _ => None,
        }
    }

    /// Opens the next round (spec §8.3–§9). Permissionless.
    pub fn open_round(&mut self, ctx: &Ctx, program: ProgramId) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        if p.phase != Phase::Open {
            return Err(Error::State("the program is not open"));
        }
        let Some(accepted_at) = self.proposal_accepted_at(&p) else {
            return Err(Error::State("no T1-confirmed round proposal"));
        };
        let pr = p.proposal.clone().unwrap();
        if ctx.now < p.round_started_at + p.params.windows.round_min || ctx.now < accepted_at + p.params.windows.rerun_window {
            return Err(Error::State("the round's minimum duration or re-run window has not passed"));
        }
        let r = p.round;
        // Nothing of the closing round may still decide a key.
        for i in 0..p.commits {
            let cid = self.store.commit_at(program, i).unwrap();
            let c = self.get_commit(cid)?;
            if c.round == r && matches!(c.state, CommitState::Committed | CommitState::Verifying) {
                return Err(Error::State("every commit of the round must be allocated or closed first"));
            }
        }
        let r1 = pr.round;
        // Gather the round's commits and their re-run results.
        let mut ids = vec![];
        for i in 0..p.commits {
            let cid = self.store.commit_at(program, i).unwrap();
            let c = self.get_commit(cid)?;
            if c.round == r || self.holds_in(&p, &c, r) {
                ids.push(cid);
            }
        }
        // Re-runs still in flight hold the opening; a re-run without a T1 result after its timeout counts
        // as an infrastructure failure, which carries its key (spec §8.3).
        for i in 0..p.commits {
            let c = self.get_commit(self.store.commit_at(program, i).unwrap())?;
            if let Some(rr) = self.rerun_at(&c, r1) {
                if rr.outcome == RunOutcome::Pending && ctx.now < rr.started_at + p.params.windows.run_timeout {
                    return Err(Error::State("a re-run is still in flight"));
                }
                if let RunOutcome::Answered { at, tier: Tier::T0, facts, .. } = &rr.outcome {
                    if facts.kind != Kind::InfraError && ctx.now <= at + p.params.windows.run_timeout {
                        return Err(Error::State("a re-run is waiting for its quote"));
                    }
                }
            }
        }
        let crit_high = |p: &Program, slot: u8| -> bool {
            p.params.slot(slot).map(|s| s.severity >= Severity::High).unwrap_or(false)
        };
        let mut released: Vec<H32> = vec![];
        let mut carried: Vec<(H32, CommitId)> = vec![];
        let mut gate = pr.canaries_ok;
        let mut fx = vec![];
        // 1. Keys of round r: released or carried.
        for &cid in &ids {
            let c = self.get_commit(cid)?;
            // Expired and void commits have no verdict and hold nothing.
            let Some(f) = c.run.and_then(|rid| self.store.run(rid)).and_then(|r0| r0.facts().cloned()) else { continue };
            let held: Vec<(H32, u8)> =
                f.keys.iter().filter(|k| self.store.holder(program, r, &k.tag) == Some(cid)).map(|k| (k.tag, k.slot)).collect();
            if held.is_empty() {
                continue;
            }
            let tags: Vec<H32> = held.iter().map(|(t, _)| *t).collect();
            match self.rerun_breaks(&c, &tags, r1) {
                Some(false) => {
                    released.extend(tags.iter().copied());
                    for t in &tags {
                        if !p.fixed_tags.contains(t) {
                            p.fixed_tags.push(*t);
                        }
                    }
                }
                Some(true) => {
                    for (t, s) in &held {
                        carried.push((*t, cid));
                        if crit_high(&p, *s) {
                            gate = false;
                        }
                    }
                }
                None => {
                    if self.rerun_at(&c, r1).is_none() {
                        // Nobody re-ran it: released by default, but it was never shown fixed.
                        released.extend(tags.iter().copied());
                        if held.iter().any(|(_, s)| crit_high(&p, *s)) {
                            gate = false;
                        }
                    } else {
                        // Requested and failed for infrastructure reasons: carried.
                        for (t, s) in &held {
                            carried.push((*t, cid));
                            if crit_high(&p, *s) {
                                gate = false;
                            }
                        }
                    }
                }
            }
        }
        // Immune memory (spec §9): every paid PoC, and every holder of a Critical or High key, from any
        // round, was re-run at this fix and shows no violation.
        for i in 0..p.commits {
            let c = self.get_commit(self.store.commit_at(program, i).unwrap())?;
            let paid = matches!(c.state, CommitState::Done(End::Paid { .. }));
            let held_crit_high = c.run.and_then(|rid| self.store.run(rid)).and_then(|run| run.facts().cloned()).is_some_and(|f| {
                f.keys.iter().any(|k| crit_high(&p, k.slot) && (0..=r).any(|x| self.store.holder(program, x, &k.tag) == Some(c.id)))
            });
            if (paid || held_crit_high) && self.rerun_clean(&c, r1) != Some(true) {
                gate = false;
            }
        }
        // 2. Duplicates: promotion (the one-way appeal) or confirmation.
        let mut promoted_tags: Vec<H32> = vec![];
        let mut promoted: Vec<CommitId> = vec![];
        for &cid in &ids {
            let mut c = self.get_commit(cid)?;
            let CommitState::Duplicate { amount, reserved, .. } = c.state.clone() else { continue };
            let Some(f) = c.run.and_then(|rid| self.store.run(rid)).and_then(|r0| r0.facts().cloned()) else { continue };
            let tags: Vec<H32> = f.keys.iter().map(|k| k.tag).collect();
            let crit = f.keys.iter().any(|k| crit_high(&p, k.slot));
            let breaks = self.rerun_breaks(&c, &tags, r1);
            if crit && breaks != Some(false) {
                // A Critical or High duplicate that is not shown fixed blocks the release.
                gate = false;
            }
            let promotable: Vec<H32> =
                tags.iter().copied().filter(|t| released.contains(t) && !promoted_tags.contains(t)).collect();
            if !promotable.is_empty() && breaks == Some(true) {
                let (nominal, slot) = self.nominal(&p, &f, Some(&promotable));
                let slot = slot.unwrap();
                let budget = self.slots_for(&p, r1).iter().find(|s| s.slot == slot).map(|s| s.budget).unwrap_or(0);
                let spent = self.store.spent(program, r1, slot);
                // Never above the duplicate's own amount, within the new round's budget.
                let amt = arith::within_budget(nominal.min(amount), budget, spent);
                self.store.set_spent(program, r1, slot, spent + amt);
                let (use_, released, debt) = arith::refit(reserved, amt);
                p.ledger.unreserve(released);
                if debt > 0 {
                    p.debtors.push(cid);
                }
                for t in &promotable {
                    promoted_tags.push(*t);
                }
                c.state = CommitState::Promoted { round: r1, amount: amt, reserved: use_, debt };
                promoted.push(cid);
                fx.push(ev("promoted", serde_json::json!({"commit_id": cid, "round": r1, "amount": amt.to_string()})));
            } else {
                p.ledger.unreserve(reserved);
                c.state = CommitState::Done(End::DuplicateConfirmed);
                fx.push(ev("duplicate_confirmed", serde_json::json!({"commit_id": cid})));
            }
            self.store.put_commit(&c);
        }
        // 3. Gap claims.
        for &cid in &ids {
            let mut c = self.get_commit(cid)?;
            let CommitState::GapClaim { amount, reserved } = c.state.clone() else { continue };
            match self.rerun_breaks_new_slot(&p, &c) {
                Some(true) => {
                    let (use_, released, debt) = arith::refit(reserved, amount);
                    p.ledger.unreserve(released);
                    if debt > 0 {
                        p.debtors.push(cid);
                    }
                    gate = false;
                    c.state = CommitState::Promoted { round: r1, amount, reserved: use_, debt };
                    promoted.push(cid);
                    fx.push(ev("gap_promoted", serde_json::json!({"commit_id": cid, "round": r1})));
                }
                _ => {
                    p.ledger.unreserve(reserved);
                    c.state = CommitState::Done(End::GapOpen);
                }
            }
            self.store.put_commit(&c);
        }
        // 4. Holders of round r+1: carried keys and promotions.
        for (t, h) in &carried {
            self.store.set_holder(program, r1, t, Some(*h));
        }
        for &cid in &promoted {
            let c = self.get_commit(cid)?;
            if let Some(rid) = c.run {
                if let Some(f) = self.get_run(rid)?.facts() {
                    for k in &f.keys {
                        if promoted_tags.contains(&k.tag) {
                            self.store.set_holder(program, r1, &k.tag, Some(cid));
                        }
                    }
                }
            }
        }
        p.params.slots.extend(pr.new_slots.iter().cloned());
        p.round_manifests.push(pr.manifest_sha256);
        p.rounds.push(RoundRecord {
            round: r1,
            manifest_sha256: pr.manifest_sha256,
            opened_at: ctx.now,
            released: released.clone(),
            carried: carried.iter().map(|(t, _)| *t).collect(),
            promoted: promoted.clone(),
            release_gate: gate,
        });
        p.round = r1;
        p.round_started_at = ctx.now;
        p.proposal = None;
        fx.push(ev("round_opened", serde_json::json!({"program_id": program, "round": r1, "release_gate": gate,
                                                     "released": released.len(), "carried": carried.len(), "promoted": promoted.len()})));
        if gate {
            // The release fee comes from the escrow, if available, to the treasury.
            let fee = p.params.release_fee.min(p.ledger.available);
            p.ledger.charge(fee);
            self.escrow_out(fee);
            self.credit_tokens(TREASURY, fee);
            p.release_permits += 1;
            fx.push(Effect::ReleasePermit { program, round: r1 });
        }
        self.store.put_program(&p);
        Ok(fx)
    }

    // ------------------------------------------------------------------ disclosure (spec §11.6)

    pub fn disclosure_condition(&self, p: &Program, tag: &H32, now: Time) -> bool {
        if p.tag_fixed(tag) {
            return true;
        }
        if let Some((_, at, paid)) = p.claimed.iter().find(|(t, _, _)| t == tag) {
            if *paid && now >= at + p.params.windows.disclosure_max_delay {
                return true;
            }
            if p.phase == Phase::Exited {
                return true;
            }
        }
        false
    }

    /// Paged disclosure: permissionless; anyone funds it.
    pub fn request_disclosure(&mut self, ctx: &Ctx, program: ProgramId, tags: &[H32], spec: RunSpec) -> R<Vec<Effect>> {
        let p = self.get_program(program)?;
        if tags.is_empty() || tags.len() > facts::MAX_DISCLOSE {
            return Err(Error::Invalid("1 to 8 tags per disclosure".into()));
        }
        for t in tags {
            if !self.disclosure_condition(&p, t, ctx.now) {
                return Err(Error::State("a tag's disclosure condition does not hold"));
            }
            if p.disclosed.iter().any(|(x, _)| x == t) {
                return Err(Error::State("a tag is already disclosed"));
            }
        }
        let run = self.new_run(&p, None, Mode::Disclose, p.round, &spec, ctx, &ctx.caller);
        let fx = self.request_run(&p, &run, &ctx.caller)?;
        Ok(vec![fx])
    }

    // ------------------------------------------------------------------ exit (spec §14.3)

    pub fn exit(&mut self, ctx: &Ctx, program: ProgramId) -> R<Vec<Effect>> {
        let mut p = self.get_program(program)?;
        let due = ctx.now >= p.params.windows.deadline + p.params.windows.exit_grace;
        let early = p.phase == Phase::Closed;
        if !(due || early) || matches!(p.phase, Phase::Exited | Phase::Rejected | Phase::Pending) {
            return Err(Error::State("exit opens after the deadline and its grace"));
        }
        let mut fx = vec![];
        // Debts first, in commit order.
        fx.extend(self.fund_debts(&mut p, u32::MAX)?);
        for i in 0..p.commits {
            let cid = self.store.commit_at(program, i).unwrap();
            let mut c = self.get_commit(cid)?;
            if let Some(ch) = c.challenge.take() {
                let mut t = self.store.totals();
                t.challenge_bonds_locked -= ch.bond;
                self.store.put_totals(&t);
                self.credit_tokens(&ch.by, ch.bond);
            }
            match c.state.clone() {
                CommitState::Committed => {
                    self.forfeit_bond(&mut p, &c);
                    c.state = CommitState::Done(End::Expired);
                }
                CommitState::Verifying => {
                    self.refund_bond(&c);
                    c.state = CommitState::Done(End::Refunded { reason: "no final verdict at exit".into() });
                }
                CommitState::Holder { amount, reserved, debt, .. } | CommitState::Promoted { amount, reserved, debt, .. } => {
                    if debt == 0 && reserved == amount {
                        // Owed and funded: paid.
                        p.ledger.pay(amount);
                        self.escrow_out(amount);
                        let b = c.beneficiary.clone();
                        self.credit_tokens(&b, amount);
                        c.state = CommitState::Done(End::Paid { amount, instant: false, at: ctx.now });
                    } else {
                        // Pay what is reserved; the rest is recorded as defaulted. Never a pro-rata
                        // cut among claims: earlier debtors were funded first.
                        p.ledger.pay(reserved);
                        self.escrow_out(reserved);
                        let b = c.beneficiary.clone();
                        self.credit_tokens(&b, reserved);
                        p.defaulted += debt;
                        c.state = CommitState::Done(End::Defaulted { owed: debt });
                    }
                }
                CommitState::Duplicate { reserved, .. } => {
                    p.ledger.unreserve(reserved);
                    c.state = CommitState::Done(End::DuplicateConfirmed);
                }
                CommitState::GapClaim { reserved, .. } => {
                    p.ledger.unreserve(reserved);
                    c.state = CommitState::Done(End::GapOpen);
                }
                CommitState::Done(_) => {}
            }
            self.store.put_commit(&c);
        }
        p.debtors.clear();
        p.cursor = p.commits;
        let ret = p.ledger.return_available();
        self.escrow_out(ret);
        let proj = p.project.clone();
        self.credit_tokens(&proj, ret);
        p.phase = Phase::Exited;
        self.store.put_program(&p);
        fx.push(ev("program_exited", serde_json::json!({"program_id": program, "returned": ret.to_string(), "defaulted": p.defaulted.to_string()})));
        Ok(fx)
    }
}

impl<S: Store> Engine<S> {
    /// A new run for an open challenge whose run failed or came back without a quote (spec §6.4).
    pub fn retry_challenge(&mut self, ctx: &Ctx, commit: CommitId, spec: RunSpec) -> R<Vec<Effect>> {
        let mut c = self.get_commit(commit)?;
        let p = self.get_program(c.program)?;
        let ch = c.challenge.clone().ok_or(Error::State("no open challenge"))?;
        if ctx.now > ch.at + p.params.windows.resolve {
            return Err(Error::State("the challenge has expired: time_out voids it"));
        }
        if let Some(rid) = ch.run {
            let r = self.get_run(rid)?;
            let retryable = match &r.outcome {
                RunOutcome::Pending => ctx.now >= r.started_at + p.params.windows.run_timeout,
                RunOutcome::Failed { .. } => true,
                RunOutcome::Answered { facts, tier, at, .. } => {
                    facts.kind == Kind::InfraError || (*tier == Tier::T0 && ctx.now >= at + p.params.windows.run_timeout)
                }
            };
            if !retryable {
                return Err(Error::State("the challenge's run is still in flight"));
            }
        }
        let run = self.new_run(&p, Some(commit), Mode::Second, c.round, &spec, ctx, &ctx.caller);
        let fx = self.request_run(&p, &run, &ctx.caller)?;
        if let Some(ch) = c.challenge.as_mut() {
            ch.run = Some(run.id);
        }
        self.store.put_commit(&c);
        Ok(vec![fx])
    }

    /// A finding whose invariant budget was exhausted proves, with a patch, that its bug is distinct
    /// (spec §7.5). The hunter pays the run.
    pub fn request_distinct(&mut self, ctx: &Ctx, commit: CommitId, spec: RunSpec) -> R<Vec<Effect>> {
        let c = self.get_commit(commit)?;
        let p = self.get_program(c.program)?;
        if c.state != CommitState::Done(End::Exhausted) {
            return Err(Error::State("only a finding whose invariant budget is exhausted proves distinctness"));
        }
        if ctx.caller != c.hunter && ctx.caller != c.beneficiary {
            return Err(Error::Forbidden("only the finder proves distinctness"));
        }
        if p.phase != Phase::Open || c.round != p.round {
            return Err(Error::State("distinctness is proven in the finding's round"));
        }
        let payer = c.beneficiary.clone();
        let run = self.new_run(&p, Some(commit), Mode::Distinct, c.round, &spec, ctx, &payer);
        let fx = self.request_run(&p, &run, &payer)?;
        Ok(vec![fx])
    }

    /// A T1 distinctness result: the finding becomes a claim beyond the invariant's budget, funded in
    /// commit order like any other.
    fn distinct_top_up(&mut self, ctx: &Ctx, p: &mut Program, cid: CommitId) -> R<Vec<Effect>> {
        let mut c = self.get_commit(cid)?;
        if c.state != CommitState::Done(End::Exhausted) {
            return Ok(vec![]);
        }
        let f = self.get_run(c.run.ok_or(Error::State("no verdict"))?)?.facts().cloned().ok_or(Error::State("no verdict"))?;
        let (amount, _) = self.nominal(p, &f, None);
        let (reserved, debt) = arith::cover(amount, p.ledger.available, !p.debtors.is_empty());
        p.ledger.reserve(reserved);
        if debt > 0 {
            p.debtors.push(cid);
        }
        c.state = CommitState::Promoted { round: c.round, amount, reserved, debt };
        self.store.put_commit(&c);
        let _ = ctx;
        Ok(vec![ev("distinct_top_up", serde_json::json!({"commit_id": cid, "amount": amount.to_string()}))])
    }
}

/// The protocol's fee account inside the credits ledger.
pub const TREASURY: &str = "__treasury__";

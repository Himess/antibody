//! Storage behind the rules. The contract implements it with NEAR collections; `MemStore` serves the
//! tests, the services and the trace replay.

use std::collections::BTreeMap;

use crate::state::*;
use crate::types::*;

pub trait Store {
    fn program(&self, id: ProgramId) -> Option<Program>;
    fn put_program(&mut self, p: &Program);
    fn commit(&self, id: CommitId) -> Option<Commit>;
    fn put_commit(&mut self, c: &Commit);
    /// The commit at `index` in `program`'s commit order.
    fn commit_at(&self, program: ProgramId, index: u32) -> Option<CommitId>;
    fn set_commit_at(&mut self, program: ProgramId, index: u32, id: CommitId);
    fn run(&self, id: RunId) -> Option<Run>;
    fn put_run(&mut self, r: &Run);
    /// The holder of a key (tag) in a round.
    fn holder(&self, program: ProgramId, round: u32, tag: &H32) -> Option<CommitId>;
    fn set_holder(&mut self, program: ProgramId, round: u32, tag: &H32, holder: Option<CommitId>);
    /// Amount charged to an invariant's budget in a round.
    fn spent(&self, program: ProgramId, round: u32, slot: u8) -> Amount;
    fn set_spent(&mut self, program: ProgramId, round: u32, slot: u8, v: Amount);
    fn account(&self, id: &str) -> Account;
    fn put_account(&mut self, id: &str, a: &Account);
    fn totals(&self) -> Totals;
    fn put_totals(&mut self, t: &Totals);
    fn next_id(&mut self, kind: IdKind) -> u64;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IdKind {
    Program,
    Commit,
    Run,
}

#[derive(Clone, Debug, Default)]
pub struct MemStore {
    pub programs: BTreeMap<ProgramId, Program>,
    pub commits: BTreeMap<CommitId, Commit>,
    pub order: BTreeMap<(ProgramId, u32), CommitId>,
    pub runs: BTreeMap<RunId, Run>,
    pub holders: BTreeMap<(ProgramId, u32, H32), CommitId>,
    pub spent: BTreeMap<(ProgramId, u32, u8), Amount>,
    pub accounts: BTreeMap<String, Account>,
    pub totals: Totals,
    pub ids: BTreeMap<IdKind, u64>,
}

impl Store for MemStore {
    fn program(&self, id: ProgramId) -> Option<Program> {
        self.programs.get(&id).cloned()
    }
    fn put_program(&mut self, p: &Program) {
        self.programs.insert(p.id, p.clone());
    }
    fn commit(&self, id: CommitId) -> Option<Commit> {
        self.commits.get(&id).cloned()
    }
    fn put_commit(&mut self, c: &Commit) {
        self.commits.insert(c.id, c.clone());
    }
    fn commit_at(&self, program: ProgramId, index: u32) -> Option<CommitId> {
        self.order.get(&(program, index)).copied()
    }
    fn set_commit_at(&mut self, program: ProgramId, index: u32, id: CommitId) {
        self.order.insert((program, index), id);
    }
    fn run(&self, id: RunId) -> Option<Run> {
        self.runs.get(&id).cloned()
    }
    fn put_run(&mut self, r: &Run) {
        self.runs.insert(r.id, r.clone());
    }
    fn holder(&self, program: ProgramId, round: u32, tag: &H32) -> Option<CommitId> {
        self.holders.get(&(program, round, *tag)).copied()
    }
    fn set_holder(&mut self, program: ProgramId, round: u32, tag: &H32, holder: Option<CommitId>) {
        match holder {
            Some(h) => {
                self.holders.insert((program, round, *tag), h);
            }
            None => {
                self.holders.remove(&(program, round, *tag));
            }
        }
    }
    fn spent(&self, program: ProgramId, round: u32, slot: u8) -> Amount {
        self.spent.get(&(program, round, slot)).copied().unwrap_or(0)
    }
    fn set_spent(&mut self, program: ProgramId, round: u32, slot: u8, v: Amount) {
        self.spent.insert((program, round, slot), v);
    }
    fn account(&self, id: &str) -> Account {
        self.accounts.get(id).cloned().unwrap_or_default()
    }
    fn put_account(&mut self, id: &str, a: &Account) {
        self.accounts.insert(id.to_string(), a.clone());
    }
    fn totals(&self) -> Totals {
        self.totals.clone()
    }
    fn put_totals(&mut self, t: &Totals) {
        self.totals = t.clone();
    }
    fn next_id(&mut self, kind: IdKind) -> u64 {
        let e = self.ids.entry(kind).or_insert(0);
        let id = *e;
        *e += 1;
        id
    }
}

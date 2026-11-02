//! The rule core's state in NEAR storage (`antibody_rules::Store`).

use antibody_rules::{Account, Amount, Commit, CommitId, IdKind, Program, ProgramId, Run, RunId, Store, Totals, H32};
use near_sdk::{near, store::LookupMap};

#[near(serializers = [borsh])]
pub struct NearStore {
    programs: LookupMap<ProgramId, Program>,
    commits: LookupMap<CommitId, Commit>,
    order: LookupMap<(ProgramId, u32), CommitId>,
    runs: LookupMap<RunId, Run>,
    holders: LookupMap<(ProgramId, u32, H32), CommitId>,
    spent: LookupMap<(ProgramId, u32, u8), Amount>,
    accounts: LookupMap<String, Account>,
    totals: Totals,
    next_program: u64,
    next_commit: u64,
    next_run: u64,
}

impl NearStore {
    pub fn new() -> Self {
        NearStore {
            programs: LookupMap::new(b"p".to_vec()),
            commits: LookupMap::new(b"c".to_vec()),
            order: LookupMap::new(b"o".to_vec()),
            runs: LookupMap::new(b"r".to_vec()),
            holders: LookupMap::new(b"h".to_vec()),
            spent: LookupMap::new(b"s".to_vec()),
            accounts: LookupMap::new(b"a".to_vec()),
            totals: Totals::default(),
            next_program: 0,
            next_commit: 0,
            next_run: 0,
        }
    }
}

impl NearStore {
    /// Writes the cached entries to storage (they are written anyway when the call ends).
    pub fn flush(&mut self) {
        self.programs.flush();
        self.commits.flush();
        self.order.flush();
        self.runs.flush();
        self.holders.flush();
        self.spent.flush();
        self.accounts.flush();
    }
}

impl Default for NearStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Store for NearStore {
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
        let c = match kind {
            IdKind::Program => &mut self.next_program,
            IdKind::Commit => &mut self.next_commit,
            IdKind::Run => &mut self.next_run,
        };
        let id = *c;
        *c += 1;
        id
    }
    fn peek_id(&self, kind: IdKind) -> u64 {
        match kind {
            IdKind::Program => self.next_program,
            IdKind::Commit => self.next_commit,
            IdKind::Run => self.next_run,
        }
    }
}

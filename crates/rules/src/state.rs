//! The state the rules keep (spec §3–§14).

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::facts::FactsSummary;
use crate::types::*;

/// How the anchor block hash was established (spec §3.1, §17).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum AnchorStatus {
    Pending,
    LightClient,
    /// The light client could not confirm it (call failed, outside its window, or another chain).
    Unconfirmed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum Phase {
    /// Created; the anchor check is in flight.
    Pending,
    /// Anchor checked; activation and known issues.
    Setup,
    Open,
    /// A verified `ProgramMisconfigured`: no commits; `exit` is available.
    Closed,
    /// After `exit`.
    Exited,
    /// The light client contradicted the anchor; the escrow was refunded.
    Rejected,
}

/// Program escrow (spec §7.10): `funded = available + reserved + paid + fees + returned`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Ledger {
    /// Initial escrow and top-ups.
    #[serde(with = "crate::types::amount")]
    pub deposits: Amount,
    /// Bonds forfeited to the escrow.
    #[serde(with = "crate::types::amount")]
    pub forfeits: Amount,
    /// Challenge bonds awarded to the escrow.
    #[serde(with = "crate::types::amount")]
    pub challenge_bonds: Amount,
    #[serde(with = "crate::types::amount")]
    pub available: Amount,
    #[serde(with = "crate::types::amount")]
    pub reserved: Amount,
    #[serde(with = "crate::types::amount")]
    pub paid: Amount,
    #[serde(with = "crate::types::amount")]
    pub fees: Amount,
    #[serde(with = "crate::types::amount")]
    pub returned: Amount,
}

/// Every movement of a program's escrow is one of these methods, and each keeps
/// `funded() == accounted()` (Kani: `proofs.rs`). Callers keep the preconditions: `reserve` and
/// `charge` take at most `available`, `unreserve` and `pay` at most `reserved`.
impl Ledger {
    /// What came in: `deposits + forfeits + challenge_bonds`.
    pub fn funded(&self) -> Amount {
        self.deposits + self.forfeits + self.challenge_bonds
    }

    /// Where it is: `available + reserved + paid + fees + returned`.
    pub fn accounted(&self) -> Amount {
        self.available + self.reserved + self.paid + self.fees + self.returned
    }

    /// The initial escrow or a top-up.
    pub fn deposit(&mut self, amount: Amount) {
        self.deposits += amount;
        self.available += amount;
    }

    /// A bond forfeited to the escrow (spec §12.2).
    pub fn forfeit(&mut self, amount: Amount) {
        self.forfeits += amount;
        self.available += amount;
    }

    /// A challenge bond awarded to the escrow.
    pub fn award(&mut self, amount: Amount) {
        self.challenge_bonds += amount;
        self.available += amount;
    }

    /// available → reserved, for a claim.
    pub fn reserve(&mut self, amount: Amount) {
        self.available -= amount;
        self.reserved += amount;
    }

    /// reserved → available.
    pub fn unreserve(&mut self, amount: Amount) {
        self.reserved -= amount;
        self.available += amount;
    }

    /// reserved → paid out to a claimant.
    pub fn pay(&mut self, amount: Amount) {
        self.reserved -= amount;
        self.paid += amount;
    }

    /// available → fees (the protocol fee, the release fee).
    pub fn charge(&mut self, fee: Amount) {
        self.available -= fee;
        self.fees += fee;
    }

    /// Returns everything still available to the project; the amount returned.
    pub fn return_available(&mut self) -> Amount {
        let amount = self.available;
        self.available = 0;
        self.returned += amount;
        amount
    }
}

/// The instant budget's current window (spec §6.3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct InstantWindow {
    pub index: u64,
    #[serde(with = "crate::types::amount")]
    pub spent: Amount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct KnownIssue {
    pub run: RunId,
    pub tags: Vec<H32>,
    pub slots: Vec<u8>,
    pub severity: Severity,
    #[serde(with = "crate::types::amount")]
    pub impact_usd_micro: Amount,
    /// Set when a round's re-run showed it fixed: its tags no longer block.
    pub fixed: bool,
}

/// A fix round under way (spec §8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Proposal {
    /// The round it would open.
    pub round: u32,
    pub manifest_sha256: H32,
    pub overrides_sha256: H32,
    pub new_slots: Vec<SlotRule>,
    pub check_run: RunId,
    /// The proposal counts once its `check_round` run is T1-confirmed and valid; re-runs may then start.
    pub proposed_at: Time,
    pub canaries_ok: bool,
}

/// One closed round's record (spec §9).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct RoundRecord {
    pub round: u32,
    pub manifest_sha256: H32,
    pub opened_at: Time,
    pub released: Vec<H32>,
    pub carried: Vec<H32>,
    pub promoted: Vec<CommitId>,
    pub release_gate: bool,
}

/// A program (spec §3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Program {
    pub id: ProgramId,
    /// `"<near-antibody account>/<id>"`.
    pub key: String,
    pub project: AccountId,
    pub manifest_sha256: H32,
    pub params: Params,
    pub phase: Phase,
    pub anchor: AnchorStatus,
    pub created_at: Time,
    pub opened_at: Option<Time>,
    /// Activation (spec §11.2).
    pub activation_run: Option<RunId>,
    pub sealing_pk: Option<H32>,
    pub key_epoch: String,
    pub known: Vec<KnownIssue>,
    pub known_runs: Vec<RunId>,
    pub ledger: Ledger,
    pub instant: InstantWindow,
    /// Commits so far; their indices are 0..commits.
    pub commits: u32,
    /// Allocation cursor (spec §7.1).
    pub cursor: u32,
    /// Commits with debt, in commit order (spec §7.8).
    pub debtors: Vec<CommitId>,
    pub round: u32,
    pub round_started_at: Time,
    /// Manifest of each round after the first (index = round - 1).
    pub round_manifests: Vec<H32>,
    pub proposal: Option<Proposal>,
    pub rounds: Vec<RoundRecord>,
    /// Pause tickets issued, one entry per round that issued one (spec §10.1).
    pub ticket_rounds: Vec<u32>,
    pub release_permits: u32,
    /// Tags whose holder was paid (project has seen it), with the time of the first claim.
    pub claimed: Vec<(u32, H32, Time, bool)>,
    /// Tags fixed by a round.
    pub fixed_tags: Vec<(u32, H32)>,
    /// (round, tag, key): the proofs of a tag committed in a round open with that round's key.
    pub disclosed: Vec<(u32, H32, H32)>,
    /// Bait metrics: time of the first claim, and paid within the first hour.
    pub first_claim_at: Option<Time>,
    #[serde(with = "crate::types::amount")]
    pub paid_first_hour: Amount,
    #[serde(with = "crate::types::amount")]
    pub defaulted: Amount,
    /// (round, key, commit): the duplicate holding a key's one promotion reservation, since one
    /// duplicate per key can be promoted (spec §7.8).
    pub dup_reserved: Vec<(u32, H32, CommitId)>,
    /// (round, commit): the gap claim holding a round's one gap reservation (spec §7.7).
    pub gap_reserved: Vec<(u32, CommitId)>,
    /// Holders and promoted claims not yet paid: what the exit settles, bounded by the program's keys
    /// rather than its commits.
    pub owed: Vec<CommitId>,
    /// After the exit: the next commit whose record `close_commits` closes.
    pub close_cursor: u32,
}

impl Program {
    pub fn manifest_of(&self, round: u32) -> H32 {
        if round == 0 {
            self.manifest_sha256
        } else {
            self.round_manifests.get(round as usize - 1).copied().unwrap_or(self.manifest_sha256)
        }
    }

    /// A tag held in `round` whose fix was released (the release gate held).
    pub fn tag_fixed(&self, round: u32, t: &H32) -> bool {
        self.fixed_tags.contains(&(round, *t))
    }

    pub fn known_tag(&self, t: &H32) -> bool {
        self.known.iter().any(|k| !k.fixed && k.tags.contains(t))
    }
}

/// Where a commit is (spec §4–§8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum CommitState {
    /// Waiting for the reveal.
    Committed,
    /// A verification run is in flight, or answered and not yet allocated (see the run).
    Verifying,
    /// Allocated: the earliest final proof of at least one key (spec §7.3).
    Holder {
        #[serde(with = "crate::types::amount")]
        amount: Amount,
        #[serde(with = "crate::types::amount")]
        reserved: Amount,
        #[serde(with = "crate::types::amount")]
        debt: Amount,
        new_keys: Vec<H32>,
    },
    /// Every key was held: nothing paid; a promotion reservation is kept (spec §8.4).
    Duplicate {
        of: CommitId,
        #[serde(with = "crate::types::amount")]
        amount: Amount,
        #[serde(with = "crate::types::amount")]
        reserved: Amount,
    },
    /// Gap lane (spec §7.7).
    GapClaim {
        #[serde(with = "crate::types::amount")]
        amount: Amount,
        #[serde(with = "crate::types::amount")]
        reserved: Amount,
    },
    /// A duplicate or gap claim promoted at a round; paid once funded.
    Promoted {
        round: u32,
        #[serde(with = "crate::types::amount")]
        amount: Amount,
        #[serde(with = "crate::types::amount")]
        reserved: Amount,
        #[serde(with = "crate::types::amount")]
        debt: Amount,
    },
    Done(End),
}

impl CommitState {
    pub fn reserved(&self) -> Amount {
        match self {
            CommitState::Holder { reserved, .. }
            | CommitState::Duplicate { reserved, .. }
            | CommitState::GapClaim { reserved, .. }
            | CommitState::Promoted { reserved, .. } => *reserved,
            _ => 0,
        }
    }
    pub fn debt(&self) -> Amount {
        match self {
            CommitState::Holder { debt, .. } | CommitState::Promoted { debt, .. } => *debt,
            _ => 0,
        }
    }
    pub fn is_done(&self) -> bool {
        matches!(self, CommitState::Done(_))
    }
}

/// How a commit ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum End {
    Paid {
        #[serde(with = "crate::types::amount")]
        amount: Amount,
        instant: bool,
        at: Time,
    },
    /// A final `NoViolation` or `InvalidPoC`: bond to the escrow.
    Forfeited,
    /// Not revealed in time: bond to the escrow.
    Expired,
    /// `Unevaluable` (policy refund), `ProgramMisconfigured`, or void after the infrastructure grace.
    Refunded { reason: String },
    /// A proof of a known issue.
    Known,
    /// A duplicate the fix stopped, or that was never promoted.
    DuplicateConfirmed,
    /// Held keys, but the invariant's budget was exhausted.
    Exhausted,
    /// A gap claim no round promoted.
    GapOpen,
    /// A valid claim the escrow never covered (at exit).
    Defaulted {
        #[serde(with = "crate::types::amount")]
        owed: Amount,
    },
}

/// A challenge (spec §6.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Challenge {
    pub by: AccountId,
    #[serde(with = "crate::types::amount")]
    pub bond: Amount,
    pub at: Time,
    pub run: Option<RunId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Commit {
    pub id: CommitId,
    pub program: ProgramId,
    pub index: u32,
    pub round: u32,
    pub hunter: AccountId,
    /// Who receives payouts and refunds: the hunter, or an agent's owner (spec §13).
    pub beneficiary: AccountId,
    pub commitment: H32,
    pub blob_sha256: Option<H32>,
    #[serde(with = "crate::types::amount")]
    pub bond: Amount,
    pub committed_at: Time,
    pub state: CommitState,
    /// The current primary verification run.
    pub run: Option<RunId>,
    /// Infrastructure failures keep the commit's priority until then (spec §6.5).
    pub retry_until: Time,
    pub attempts: u32,
    pub second: Option<RunId>,
    pub second_agrees: bool,
    pub challenge: Option<Challenge>,
    /// Immune memory / appeal re-run at the current proposal (spec §8.2).
    pub rerun: Option<RunId>,
    pub ticket: bool,
    pub wrapped: Option<RunId>,
}

/// A verifier run (spec §5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Run {
    pub id: RunId,
    pub program: ProgramId,
    pub commit: Option<CommitId>,
    pub mode: Mode,
    pub round: u32,
    pub request_sha256: H32,
    /// The transaction signer that requested it: OutLayer attests it as the caller (T1).
    pub signer: AccountId,
    pub payer: AccountId,
    pub started_at: Time,
    pub outcome: RunOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum RunOutcome {
    Pending,
    Answered { output_sha256: H32, facts: FactsSummary, at: Time, tier: Tier },
    /// No answer, or an answer that does not match the request.
    Failed { at: Time, reason: String },
}

impl Run {
    pub fn facts(&self) -> Option<&FactsSummary> {
        match &self.outcome {
            RunOutcome::Answered { facts, .. } => Some(facts),
            _ => None,
        }
    }
    pub fn tier(&self) -> Option<Tier> {
        match &self.outcome {
            RunOutcome::Answered { tier, .. } => Some(*tier),
            _ => None,
        }
    }
    pub fn answered_at(&self) -> Option<Time> {
        match &self.outcome {
            RunOutcome::Answered { at, .. } => Some(*at),
            _ => None,
        }
    }
}

/// An agent (spec §13).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Agent {
    pub owner: AccountId,
    #[serde(with = "crate::types::amount")]
    pub bond_budget: Amount,
    #[serde(with = "crate::types::amount")]
    pub per_commit_cap: Amount,
    #[serde(with = "crate::types::amount")]
    pub bonds_locked: Amount,
    #[serde(with = "crate::types::amount")]
    pub bonds_lost: Amount,
    #[serde(with = "crate::types::amount")]
    pub bonds_returned: Amount,
    #[serde(with = "crate::types::amount")]
    pub paid: Amount,
    #[serde(with = "crate::types::amount")]
    pub verification_spent: Amount,
    pub commits: u32,
    pub containment_events: u32,
    /// The programs it may commit to (`None`: any). A practice account's is the practice program.
    pub programs: Option<Vec<ProgramId>>,
}

/// Balances held for an account (spec §7.9, §12.1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Account {
    /// Tokens credited and withdrawable (payouts, refunds, returned escrow, fees).
    #[serde(with = "crate::types::amount")]
    pub tokens: Amount,
    /// yoctoNEAR credit for verification and storage.
    #[serde(with = "crate::types::amount")]
    pub near: Amount,
    pub agent: Option<Agent>,
}

/// Contract-wide totals for `audit()` (spec §7.10).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Totals {
    /// Tokens received (escrow, top-ups, bonds, challenge bonds, deposits).
    #[serde(with = "crate::types::amount")]
    pub tokens_in: Amount,
    /// Tokens sent out by withdrawals.
    #[serde(with = "crate::types::amount")]
    pub tokens_out: Amount,
    /// Σ (available + reserved) over programs.
    #[serde(with = "crate::types::amount")]
    pub escrow: Amount,
    /// Σ token credits.
    #[serde(with = "crate::types::amount")]
    pub credits: Amount,
    #[serde(with = "crate::types::amount")]
    pub bonds_locked: Amount,
    #[serde(with = "crate::types::amount")]
    pub challenge_bonds_locked: Amount,
    /// NEAR credits held for accounts.
    #[serde(with = "crate::types::amount")]
    pub near_credits: Amount,
    pub programs: u32,
    pub runs: u64,
}

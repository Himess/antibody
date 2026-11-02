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
    pub deposits: Amount,
    /// Bonds forfeited to the escrow.
    pub forfeits: Amount,
    /// Challenge bonds awarded to the escrow.
    pub challenge_bonds: Amount,
    pub available: Amount,
    pub reserved: Amount,
    pub paid: Amount,
    pub fees: Amount,
    pub returned: Amount,
}

impl Ledger {
    pub fn funded(&self) -> Amount {
        self.deposits + self.forfeits + self.challenge_bonds
    }
}

/// The instant budget's current window (spec §6.3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct InstantWindow {
    pub index: u64,
    pub spent: Amount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct KnownIssue {
    pub run: RunId,
    pub tags: Vec<H32>,
    pub slots: Vec<u8>,
    pub severity: Severity,
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
    pub proposed_at: Time,
    /// Set when the `check_round` run is final and valid: re-runs may start.
    pub accepted_at: Option<Time>,
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
    pub claimed: Vec<(H32, Time, bool)>,
    /// Tags fixed by a round.
    pub fixed_tags: Vec<H32>,
    /// Tags disclosed, with their disclosure keys.
    pub disclosed: Vec<(H32, H32)>,
    /// Bait metrics: time of the first claim, and paid within the first hour.
    pub first_claim_at: Option<Time>,
    pub paid_first_hour: Amount,
    pub defaulted: Amount,
}

impl Program {
    pub fn manifest_of(&self, round: u32) -> H32 {
        if round == 0 {
            self.manifest_sha256
        } else {
            self.round_manifests.get(round as usize - 1).copied().unwrap_or(self.manifest_sha256)
        }
    }

    pub fn tag_fixed(&self, t: &H32) -> bool {
        self.fixed_tags.contains(t)
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
    Holder { amount: Amount, reserved: Amount, debt: Amount, new_keys: Vec<H32> },
    /// Every key was held: nothing paid; a promotion reservation is kept (spec §8.4).
    Duplicate { of: CommitId, amount: Amount, reserved: Amount },
    /// Gap lane (spec §7.7).
    GapClaim { amount: Amount, reserved: Amount },
    /// A duplicate or gap claim promoted at a round; paid once funded.
    Promoted { round: u32, amount: Amount, reserved: Amount, debt: Amount },
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
    Paid { amount: Amount, instant: bool, at: Time },
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
    Defaulted { owed: Amount },
}

/// A challenge (spec §6.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Challenge {
    pub by: AccountId,
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
    pub bond_budget: Amount,
    pub per_commit_cap: Amount,
    pub bonds_locked: Amount,
    pub bonds_lost: Amount,
    pub bonds_returned: Amount,
    pub paid: Amount,
    pub verification_spent: Amount,
    pub commits: u32,
    pub containment_events: u32,
}

/// Balances held for an account (spec §7.9, §12.1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Account {
    /// Tokens credited and withdrawable (payouts, refunds, returned escrow, fees).
    pub tokens: Amount,
    /// yoctoNEAR credit for verification and storage.
    pub near: Amount,
    pub agent: Option<Agent>,
}

/// Contract-wide totals for `audit()` (spec §7.10).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Totals {
    /// Tokens received (escrow, top-ups, bonds, challenge bonds, deposits).
    pub tokens_in: Amount,
    /// Tokens sent out by withdrawals.
    pub tokens_out: Amount,
    /// Σ (available + reserved) over programs.
    pub escrow: Amount,
    /// Σ token credits.
    pub credits: Amount,
    pub bonds_locked: Amount,
    pub challenge_bonds_locked: Amount,
    /// NEAR credits held for accounts.
    pub near_credits: Amount,
    pub programs: u32,
    pub runs: u64,
}

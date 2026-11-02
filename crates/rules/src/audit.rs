//! `audit()`: both sides of every conservation equation (spec §7.10).

use serde::Serialize;

use crate::store::Store;
use crate::types::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProgramAudit {
    pub program_id: ProgramId,
    /// deposits + forfeits + challenge bonds awarded to the escrow
    pub funded: String,
    /// available + reserved + paid + fees + returned
    pub accounted: String,
    pub available: String,
    pub reserved: String,
    /// Σ reservations held by the program's commits
    pub reservations: String,
    pub paid: String,
    pub fees: String,
    pub returned: String,
    /// Σ debts of the shortfall queue (owed, not funded)
    pub debt: String,
    pub defaulted: String,
    pub ok: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContractAudit {
    /// Tokens received minus tokens sent: what near-antibody must hold.
    pub tokens_held: String,
    /// escrow + credits + locked bonds + locked challenge bonds
    pub liabilities: String,
    pub escrow: String,
    pub credits: String,
    pub bonds_locked: String,
    pub challenge_bonds_locked: String,
    pub near_credits: String,
    pub ok: bool,
}

pub fn program<S: Store>(s: &S, id: ProgramId) -> Option<ProgramAudit> {
    let p = s.program(id)?;
    let l = &p.ledger;
    let mut reservations: Amount = 0;
    let mut debt: Amount = 0;
    for i in 0..p.commits {
        if let Some(c) = s.commit_at(id, i).and_then(|cid| s.commit(cid)) {
            reservations += c.state.reserved();
            debt += c.state.debt();
        }
    }
    let accounted = l.available + l.reserved + l.paid + l.fees + l.returned;
    Some(ProgramAudit {
        program_id: id,
        funded: l.funded().to_string(),
        accounted: accounted.to_string(),
        available: l.available.to_string(),
        reserved: l.reserved.to_string(),
        reservations: reservations.to_string(),
        paid: l.paid.to_string(),
        fees: l.fees.to_string(),
        returned: l.returned.to_string(),
        debt: debt.to_string(),
        defaulted: p.defaulted.to_string(),
        ok: l.funded() == accounted && l.reserved == reservations,
    })
}

pub fn contract<S: Store>(s: &S) -> ContractAudit {
    let t = s.totals();
    let held = t.tokens_in - t.tokens_out;
    let liabilities = t.escrow + t.credits + t.bonds_locked + t.challenge_bonds_locked;
    ContractAudit {
        tokens_held: held.to_string(),
        liabilities: liabilities.to_string(),
        escrow: t.escrow.to_string(),
        credits: t.credits.to_string(),
        bonds_locked: t.bonds_locked.to_string(),
        challenge_bonds_locked: t.challenge_bonds_locked.to_string(),
        near_credits: t.near_credits.to_string(),
        ok: held == liabilities,
    }
}

/// Σ (available + reserved) over every program must equal the contract's escrow total.
pub fn escrow_matches<S: Store>(s: &S, programs: &[ProgramId]) -> bool {
    let sum: Amount = programs.iter().filter_map(|id| s.program(*id)).map(|p| p.ledger.available + p.ledger.reserved).sum();
    sum == s.totals().escrow
}

//! The scenarios of `formal/antibody.qnt`, against the real rule core. Each test is named after the
//! Quint run it mirrors.

mod common;

use antibody_rules::*;
use common::*;

const A: u8 = 0; // Critical slot
const B: u8 = 1; // High slot

#[test]
fn instant_payout() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c).unwrap();
    assert!(matches!(w.end(c), Some(End::Paid { amount: 200, instant: true, .. })));
    // One pause ticket per round, for a T1 first finding in the pause policy.
    let ctx = w.ctx("keeper.testnet");
    let fx = w.e.pause_ticket(&ctx, c).unwrap();
    assert!(fx.contains(&Effect::PausePermit { program: p, round: 0, ticket: c }));
    assert!(w.e.pause_ticket(&ctx, c).is_err());
    w.audit_ok(&[p]);
}

#[test]
fn t0_waits_window() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], false);
    assert!(w.allocate(p).is_err(), "a T0 verdict is provisional");
    w.tick(121);
    w.allocate(p).unwrap();
    w.pay(c).unwrap();
    assert!(matches!(w.end(c), Some(End::Paid { instant: false, .. })));
    // A T0 verdict cannot mint a pause ticket.
    let ctx = w.ctx("keeper.testnet");
    assert!(w.e.pause_ticket(&ctx, c).is_err());
    w.audit_ok(&[p]);
}

#[test]
fn instant_budget() {
    let mut w = W::new();
    let mut prm = params();
    prm.instant_budget = 300;
    let p = w.program_open(1_000, prm);
    let (c1, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    let (c2, _) = w.finding(p, H2, Kind::Violation, &[(B, tag(2))], true);
    w.allocate(p).unwrap();
    w.pay(c1).unwrap(); // 200 instant
    w.pay(c2).unwrap(); // 100 more: 300, exactly the budget
    assert!(matches!(w.end(c2), Some(End::Paid { instant: true, .. })));
    assert_eq!(w.program(p).instant.spent, 300);
    w.audit_ok(&[p]);
}

#[test]
fn instant_budget_exhausted_waits_window() {
    let mut w = W::new();
    let mut prm = params();
    prm.instant_budget = 300;
    prm.instant_cap = 300;
    let p = w.program_open(1_000, prm);
    let (c1, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    let (c2, _) = w.finding(p, H2, Kind::Violation, &[(A, tag(2))], true);
    w.allocate(p).unwrap();
    w.pay(c1).unwrap(); // 200 instant
    assert!(w.pay(c2).is_err(), "200 + 200 exceeds the instant budget of 300");
    w.tick(121);
    w.pay(c2).unwrap();
    assert!(matches!(w.end(c2), Some(End::Paid { instant: false, .. })));
    w.audit_ok(&[p]);
}

#[test]
fn above_cap_needs_second_run() {
    let mut w = W::new();
    let mut prm = params();
    prm.instant_cap = 150; // a Critical (200) is above it
    prm.instant_budget = 150;
    let p = w.program_open(1_000, prm);
    let (c, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    assert!(w.allocate(p).is_err(), "T1 above the cap is not final at once");
    w.tick(121);
    assert!(w.allocate(p).is_err(), "the window passed, but no second run agreed");
    let ctx = w.ctx(H1);
    let spec = w.spec();
    let r = w.e.request_second(&ctx, c, spec);
    let fx = w.ok(r);
    let run2 = last_run(&fx);
    w.answer(run2, Kind::Violation, &[(A, tag(1))]);
    w.allocate(p).unwrap();
    w.pay(c).unwrap();
    assert!(matches!(w.end(c), Some(End::Paid { amount: 200, instant: false, .. })));
    w.audit_ok(&[p]);
}

#[test]
fn disagreeing_second_run_voids() {
    let mut w = W::new();
    let mut prm = params();
    prm.instant_cap = 150;
    prm.instant_budget = 150;
    let p = w.program_open(1_000, prm);
    let (c, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], false); // a lying T0 answer
    let ctx = w.ctx(H1);
    let spec = w.spec();
    let r = w.e.request_second(&ctx, c, spec);
    let fx = w.ok(r);
    w.answer(last_run(&fx), Kind::NoViolation, &[]);
    assert_eq!(w.commit(c).run, None, "void: the hunter retries with priority kept");
    let spec = w.spec();
    let r = w.e.retry(&ctx, c, spec);
    let fx = w.ok(r);
    let run = last_run(&fx);
    w.answer(run, Kind::NoViolation, &[]);
    w.t1(run);
    w.allocate(p).unwrap();
    assert_eq!(w.end(c), Some(End::Forfeited));
    w.audit_ok(&[p]);
}

#[test]
fn shortfall_queue() {
    let mut w = W::new();
    let p = w.program_open(300, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true); // holder 200
    let (c1, _) = w.finding(p, H2, Kind::Violation, &[(A, tag(1))], true); // duplicate, reserves 100 of 200
    let (c2, _) = w.finding(p, H1, Kind::Violation, &[(B, tag(2))], true); // holder 100, nothing left
    w.allocate(p).unwrap();
    match w.commit(c2).state {
        CommitState::Holder { amount: 100, reserved: 0, debt: 100, .. } => {}
        s => panic!("expected a holder in debt, got {s:?}"),
    }
    assert!(matches!(w.commit(c1).state, CommitState::Duplicate { reserved: 100, .. }));
    assert!(w.pay(c2).is_err(), "never a pro-rata cut");
    w.pay(c0).unwrap();
    let r = w.e.top_up(p, 60);
    w.ok(r);
    let r = w.e.settle_debts(p, 10);
    w.ok(r);
    assert!(matches!(w.commit(c2).state, CommitState::Holder { reserved: 60, debt: 40, .. }));
    let r = w.e.top_up(p, 40);
    w.ok(r);
    let r = w.e.settle_debts(p, 10);
    w.ok(r);
    w.pay(c2).unwrap();
    assert!(matches!(w.end(c2), Some(End::Paid { amount: 100, .. })));
    w.audit_ok(&[p]);
}

#[test]
fn challenge_overturns_lying_t0() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c, _) = w.finding(p, H1, Kind::NoViolation, &[], false); // a lying operator rejects a real finding
    let ctx = w.ctx(H1);
    let spec = w.spec();
    let r = w.e.challenge(&ctx, c, 10, spec);
    let fx = w.ok(r);
    let run = last_run(&fx);
    w.answer(run, Kind::Violation, &[(A, tag(1))]);
    w.t1(run);
    assert!(w.commit(c).challenge.is_none());
    assert!(w.events.iter().any(|(n, _)| n == "challenge_overturned"));
    w.allocate(p).unwrap();
    w.pay(c).unwrap();
    assert!(matches!(w.end(c), Some(End::Paid { amount: 200, .. })));
    w.audit_ok(&[p]);
}

#[test]
fn challenge_upheld_pays_hunter() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c, run) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], false);
    let before = w.e.store.account(H1).tokens;
    let ctx = w.ctx(PROJECT);
    let spec = w.spec();
    let r = w.e.challenge(&ctx, c, 10, spec);
    w.ok(r);
    // The original run's quote is relayed: a T1 result for the same request settles the challenge.
    w.t1(run);
    assert!(w.events.iter().any(|(n, _)| n == "challenge_upheld"));
    assert_eq!(w.e.store.account(H1).tokens, before + 10, "the challenger's bond goes to the hunter");
    w.allocate(p).unwrap();
    w.pay(c).unwrap();
    w.audit_ok(&[p]);
}

#[test]
fn challenge_timeout_keeps_priority() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], false);
    let (c1, _) = w.finding(p, H2, Kind::Violation, &[(A, tag(1))], true);
    let ctx = w.ctx(PROJECT);
    let spec = w.spec();
    let r = w.e.challenge(&ctx, c0, 10, spec);
    w.ok(r);
    assert!(w.allocate(p).is_err(), "commit 1 waits behind commit 0");
    w.tick(121);
    let ctx = w.ctx(PROJECT);
    let r = w.e.time_out(&ctx, c0);
    w.ok(r);
    assert_eq!(w.commit(c0).run, None, "void, priority kept");
    let ctx1 = w.ctx(H1);
    let spec = w.spec();
    let r = w.e.retry(&ctx1, c0, spec);
    let fx = w.ok(r);
    let run = last_run(&fx);
    w.answer(run, Kind::Violation, &[(A, tag(1))]);
    w.t1(run);
    w.allocate(p).unwrap();
    assert!(matches!(w.commit(c0).state, CommitState::Holder { .. }));
    assert!(matches!(w.commit(c1).state, CommitState::Duplicate { of, .. } if of == c0));
    w.audit_ok(&[p]);
}

#[test]
fn appeal_promotes_distinct_bug_and_blocks_release() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true); // bug 1
    let (c1, _) = w.finding(p, H2, Kind::Violation, &[(A, tag(1))], true); // bug 2, same key
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    assert!(matches!(w.commit(c1).state, CommitState::Duplicate { .. }));
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::NoViolation, &[]); // the fix stops bug 1
    w.rerun(c1, Kind::Violation, &[(A, tag(1))]); // not bug 2
    w.tick(61);
    let fx = w.open_round(p).unwrap();
    assert!(matches!(w.commit(c1).state, CommitState::Promoted { round: 1, .. }));
    assert!(!fx.iter().any(|f| matches!(f, Effect::ReleasePermit { .. })), "a Critical still breaks the fix");
    w.pay(c1).unwrap();
    assert!(matches!(w.end(c1), Some(End::Paid { amount: 200, .. })));
    assert_eq!(w.e.store.holder(p, 1, &tag(1)), Some(c1));
    w.audit_ok(&[p]);
}

#[test]
fn confirmed_duplicate_releases_reservation_and_release_ships() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    let (c1, _) = w.finding(p, H2, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::NoViolation, &[]);
    w.rerun(c1, Kind::NoViolation, &[]);
    w.tick(61);
    let fx = w.open_round(p).unwrap();
    assert_eq!(w.end(c1), Some(End::DuplicateConfirmed));
    assert!(fx.contains(&Effect::ReleasePermit { program: p, round: 1 }), "only code that survived is released");
    assert_eq!(w.program(p).ledger.reserved, 0);
    w.audit_ok(&[p]);
}

#[test]
fn unexamined_duplicate_blocks_release() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    let (c1, _) = w.finding(p, H2, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::NoViolation, &[]); // nobody re-runs the duplicate
    w.tick(61);
    let fx = w.open_round(p).unwrap();
    assert!(!fx.iter().any(|f| matches!(f, Effect::ReleasePermit { .. })));
    assert_eq!(w.end(c1), Some(End::DuplicateConfirmed));
    w.audit_ok(&[p]);
}

#[test]
fn gap_lane_promoted() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c, run) = w.commit_reveal(p, H1);
    w.answer_x(run, Kind::NoViolation, &[], serde_json::json!({"protected_harm_usd_micro": "5000"}));
    w.t1(run);
    w.allocate(p).unwrap();
    assert!(matches!(w.commit(c).state, CommitState::GapClaim { reserved: 20, .. }));
    w.propose(p, vec![SlotRule { slot: 7, severity: Severity::High, budget: 500 }], true);
    w.rerun(c, Kind::Violation, &[(7, tag(9))]);
    w.tick(61);
    w.open_round(p).unwrap();
    w.pay(c).unwrap();
    assert!(matches!(w.end(c), Some(End::Paid { amount: 20, .. })));
    assert!(w.program(p).params.slot(7).is_some(), "the round added the invariant");
    w.audit_ok(&[p]);
}

#[test]
fn release_gate_blocks_unfixed() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::Violation, &[(A, tag(1))]); // a "fix" that fixes nothing
    w.tick(61);
    let fx = w.open_round(p).unwrap();
    assert!(!fx.iter().any(|f| matches!(f, Effect::ReleasePermit { .. })));
    assert_eq!(w.e.store.holder(p, 1, &tag(1)), Some(c0), "the key is carried");
    w.audit_ok(&[p]);
}

/// A partial fix carries the key; the next round's fix is re-run (a re-run counts for its own
/// proposal only) and releases it.
#[test]
fn later_fix_releases_carried_key() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::Violation, &[(A, tag(1))]);
    w.tick(61);
    let fx = w.open_round(p).unwrap();
    assert!(!fx.iter().any(|f| matches!(f, Effect::ReleasePermit { .. })));
    assert_eq!(w.e.store.holder(p, 1, &tag(1)), Some(c0), "carried by the partial fix");
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::NoViolation, &[]);
    w.tick(61);
    let fx = w.open_round(p).unwrap();
    assert!(fx.iter().any(|f| matches!(f, Effect::ReleasePermit { round: 2, .. })));
    assert_eq!(w.e.store.holder(p, 2, &tag(1)), None, "released at round 2");
    w.audit_ok(&[p]);
}

/// Immune memory: every paid PoC is re-run at every fix, so a fix that brings a paid bug back is
/// not released.
#[test]
fn regression_blocks_release() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::NoViolation, &[]);
    w.tick(61);
    assert!(w.open_round(p).unwrap().iter().any(|f| matches!(f, Effect::ReleasePermit { round: 1, .. })));
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::Violation, &[(A, tag(1))]); // the new code reintroduces the bug
    w.tick(61);
    assert!(!w.open_round(p).unwrap().iter().any(|f| matches!(f, Effect::ReleasePermit { .. })));
    w.audit_ok(&[p]);
}

/// A round check that fails without a verdict (OutLayer down, an infrastructure error) drops the
/// proposal: one proposal at a time must not block the program's rounds for good.
#[test]
fn a_failed_round_check_lets_the_project_propose_again() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let ctx = w.ctx(PROJECT);
    let spec = w.spec();
    let r = w.e.propose_round(&ctx, p, H32([8; 32]), H32([6; 32]), vec![], spec);
    let run = last_run(&w.ok(r));
    let now = w.ctx(PROJECT);
    let r = w.e.run_failed(&now, run, "outlayer unavailable");
    w.ok(r);
    assert!(w.e.store.program(p).unwrap().proposal.is_none(), "the failed check drops the proposal");
    w.propose(p, vec![], true);
    assert!(w.e.store.program(p).unwrap().proposal.is_some(), "and the project proposes again");
}

/// A re-run the verifier could not judge (InvalidPoC) shows nothing fixed: the key is carried, the gate
/// stays shut, and the re-run may be asked again.
#[test]
fn an_unjudged_rerun_is_no_evidence() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::InvalidPoC, &[]);
    w.rerun(c0, Kind::InvalidPoC, &[]); // no evidence: it may be asked again
    w.tick(61);
    let fx = w.open_round(p).unwrap();
    assert!(!fx.iter().any(|f| matches!(f, Effect::ReleasePermit { .. })), "nothing was shown fixed");
    assert_eq!(w.e.store.holder(p, 1, &tag(1)), Some(c0), "the key is carried");
    w.audit_ok(&[p]);
}

#[test]
fn canaries_block_release() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    w.propose(p, vec![], false);
    w.rerun(c0, Kind::NoViolation, &[]);
    w.tick(61);
    let fx = w.open_round(p).unwrap();
    assert!(!fx.iter().any(|f| matches!(f, Effect::ReleasePermit { .. })));
}

#[test]
fn known_issue_needs_t1_cap_and_setup() {
    let mut w = W::new();
    let ctx = w.ctx(PROJECT);
    let (p, fx) = w
        .e
        .create_program(&ctx, engine::NewProgram { project: PROJECT.into(), manifest_sha256: H32([7; 32]), params: params(), escrow: 1_000, anchor_uncheckable: false })
        .unwrap();
    w.log(&fx);
    let r = w.e.anchor_checked(p, Some(true));
    w.ok(r);
    let spec = w.spec();
    let r = w.e.activate(&ctx, p, spec);
    let run = last_run(&w.ok(r));
    w.answer_x(run, Kind::NoViolation, &[], serde_json::json!({"sealing_pk": H32([9; 32])}));
    w.t1(run);
    // A known High issue (100 ≤ 20% of 1000).
    let spec = w.spec();
    let r = w.e.submit_known(&ctx, p, spec);
    let k1 = last_run(&w.ok(r));
    w.answer(k1, Kind::Violation, &[(B, tag(2))]);
    assert!(w.program(p).known.is_empty(), "a T0 known issue does not count");
    w.t1(k1);
    assert_eq!(w.program(p).known.len(), 1);
    // A known Critical (200) would exceed the 20% share.
    let spec = w.spec();
    let r = w.e.submit_known(&ctx, p, spec);
    let k2 = last_run(&w.ok(r));
    w.answer(k2, Kind::Violation, &[(A, tag(3))]);
    w.t1(k2);
    assert_eq!(w.program(p).known.len(), 1, "over the known-issue cap");
    let r = w.e.open_program(&ctx, p);
    w.ok(r);
    let spec = w.spec();
    assert!(w.e.submit_known(&ctx, p, spec).is_err(), "never after open");
    let (c, _) = w.finding(p, H1, Kind::Violation, &[(B, tag(2))], true);
    w.allocate(p).unwrap();
    assert_eq!(w.end(c), Some(End::Known));
    w.audit_ok(&[p]);
}

#[test]
fn exit_settles_everything() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let ctx = w.ctx(H1);
    let (unrevealed, fx) = w.e.commit(&ctx, p, 0, H32([3; 32]), true, 10, false).unwrap();
    w.log(&fx);
    let (in_flight, _) = w.commit_reveal(p, H2);
    let (paid_later, _) = w.finding(p, H1, Kind::Violation, &[(B, tag(5))], true);
    let _ = paid_later;
    assert!(w.e.exit(&w.ctx("anyone.testnet"), p).is_err(), "not before the deadline and its grace");
    w.tick(10_000 + 600);
    let r = w.e.exit(&w.ctx("anyone.testnet"), p);
    w.ok(r);
    assert_eq!(w.end(unrevealed), Some(End::Expired));
    assert!(matches!(w.end(in_flight), Some(End::Refunded { .. })));
    let prog = w.program(p);
    assert_eq!(prog.phase, Phase::Exited);
    assert_eq!(prog.ledger.available + prog.ledger.reserved, 0);
    assert_eq!(w.e.store.totals().bonds_locked, 0);
    w.audit_ok(&[p]);
}

#[test]
fn misconfigured_program_closes() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c, _) = w.finding(p, H1, Kind::ProgramMisconfigured, &[], true);
    w.allocate(p).unwrap();
    assert!(matches!(w.end(c), Some(End::Refunded { .. })));
    let ctx = w.ctx(H2);
    assert!(w.e.commit(&ctx, p, 0, H32([4; 32]), true, 10, false).is_err());
    let r = w.e.exit(&w.ctx("anyone.testnet"), p);
    w.ok(r);
    w.audit_ok(&[p]);
}

#[test]
fn infra_error_keeps_priority_then_refunds() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, run) = w.commit_reveal(p, H1);
    w.answer(run, Kind::InfraError, &[]);
    let (_c1, _) = w.finding(p, H2, Kind::Violation, &[(A, tag(1))], true);
    assert!(w.allocate(p).is_err(), "the earlier commit keeps its priority while it retries");
    let ctx = w.ctx(H1);
    let spec = w.spec();
    let r = w.e.retry(&ctx, c0, spec);
    let run2 = last_run(&w.ok(r));
    w.answer(run2, Kind::InfraError, &[]);
    w.tick(301);
    let ctx = w.ctx(H1);
    let r = w.e.time_out(&ctx, c0);
    w.ok(r);
    assert!(matches!(w.end(c0), Some(End::Refunded { .. })), "never forfeited");
    w.allocate(p).unwrap();
    w.audit_ok(&[p]);
}

#[test]
fn facts_for_another_request_are_an_infra_error() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c, run) = w.commit_reveal(p, H1);
    let mut bytes = w.facts(run, Kind::NoViolation, &[], serde_json::json!({}));
    let s = String::from_utf8(bytes.clone()).unwrap().replace("\"run_id\":", "\"run_id\":1");
    bytes = s.into_bytes();
    let ctx = w.ctx("outlayer.testnet");
    let r = w.e.run_answered(&ctx, run, &bytes, H32([0; 32]));
    w.ok(r);
    assert_eq!(w.run(run).facts().unwrap().kind, Kind::InfraError);
    assert!(w.e.run_answered(&ctx, run, &bytes, H32([0; 32])).is_err(), "a stale callback is ignored");
    let _ = c;
}

#[test]
fn agent_containment() {
    let mut w = W::new();
    let owner = "ops.testnet";
    let agent = "scout.ops.testnet";
    w.e.token_deposit(owner, 100).unwrap();
    w.e.near_deposit(owner, 1_000).unwrap();
    let octx = w.ctx(owner);
    assert!(w.e.register_agent(&octx, "scout.other.testnet", 20, 10, None).is_err(), "only a direct sub-account");
    let r = w.e.register_agent(&octx, agent, 20, 10, None);
    w.ok(r);
    let p = w.program_open(1_000, params());
    let actx = w.ctx(agent);
    // Bond from the owner's credit, within the cap.
    let (c1, _) = w.e.commit(&actx, p, 0, H32([5; 32]), true, 0, true).unwrap();
    let (c2, _) = w.e.commit(&actx, p, 0, H32([6; 32]), true, 0, true).unwrap();
    // A third bond exceeds the budget of 20.
    assert!(matches!(w.e.commit(&actx, p, 0, H32([7; 32]), true, 0, true), Err(Error::Forbidden(_))));
    // An agent never withdraws.
    assert!(w.e.withdraw(&actx, 1).is_err());
    assert!(w.e.near_withdraw(&actx, 1).is_err());
    // Its payouts go to its owner.
    let spec = w.spec();
    let r = w.e.reveal(&actx, c1, H32([5; 32]), H32([1; 32]), spec);
    let run = last_run(&w.ok(r));
    w.answer(run, Kind::Violation, &[(A, tag(1))]);
    w.t1(run);
    w.tick(61); // the second commit expires
    let kctx = w.ctx("keeper.testnet");
    let r = w.e.expire(&kctx, c2);
    w.ok(r);
    w.allocate(p).unwrap();
    let before = w.e.store.account(owner).tokens;
    w.pay(c1).unwrap();
    assert_eq!(w.e.store.account(owner).tokens, before + 200);
    let ag = w.e.store.account(agent).agent.unwrap();
    assert_eq!(ag.paid, 200);
    assert_eq!(ag.bonds_lost, 10, "the expired commit's bond");
    // Losses count against the budget: one more bond fits (10 lost + 10 locked = 20), the next does not,
    // so a program built to forfeit bonds cannot drain the owner past the budget.
    let (c3, fx) = w.e.commit(&actx, p, 0, H32([8; 32]), true, 0, true).unwrap();
    w.log(&fx);
    let _ = c3;
    assert!(matches!(w.e.commit(&actx, p, 0, H32([9; 32]), true, 0, true), Err(Error::Forbidden(_))));
    w.audit_ok(&[p]);
}

#[test]
fn an_agent_commits_only_to_its_programs() {
    let mut w = W::new();
    let owner = "ops.testnet";
    let agent = "scout.ops.testnet";
    w.e.token_deposit(owner, 100).unwrap();
    w.e.near_deposit(owner, 1_000).unwrap();
    let p = w.program_open(1_000, params());
    let q = w.program_open(1_000, params());
    let octx = w.ctx(owner);
    assert!(w.e.register_agent(&octx, agent, 50, 10, Some(vec![])).is_err(), "an empty list is no list");
    let r = w.e.register_agent(&octx, agent, 50, 10, Some(vec![p]));
    w.ok(r);
    let actx = w.ctx(agent);
    let (_, fx) = w.e.commit(&actx, p, 0, H32([5; 32]), true, 0, true).unwrap();
    w.log(&fx);
    assert!(matches!(w.e.commit(&actx, q, 0, H32([6; 32]), true, 0, true), Err(Error::Forbidden(_))), "not one of its programs");
}

#[test]
fn per_invariant_budget_and_no_distinct_top_up() {
    let mut w = W::new();
    let mut prm = params();
    prm.slots[0].budget = 200; // one Critical payout per round for slot 0
    let p = w.program_open(1_000, prm);
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    let (c1, _) = w.finding(p, H2, Kind::Violation, &[(A, tag(2))], true); // another entry point
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    assert_eq!(w.end(c1), Some(End::Exhausted));
    // The distinctness top-up is off: a patch the requester chooses could read the earlier sealed proof.
    let ctx = w.ctx(H2);
    let spec = w.spec();
    assert!(w.e.request_distinct(&ctx, c1, spec).is_err());
    assert_eq!(w.end(c1), Some(End::Exhausted));
    w.audit_ok(&[p]);
}

#[test]
fn disclosure_needs_condition_and_t1() {
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    let ctx = w.ctx("anyone.testnet");
    let spec = w.spec();
    assert!(w.e.request_disclosure(&ctx, p, &[(0, tag(1))], spec).is_err(), "not before the delay or a fix");
    w.tick(601);
    let ctx = w.ctx("anyone.testnet");
    let spec = w.spec();
    assert!(w.e.request_disclosure(&ctx, p, &[(1, tag(1))], spec).is_err(), "round 1 holds no claim of the tag");
    let spec = w.spec();
    let r = w.e.request_disclosure(&ctx, p, &[(0, tag(1))], spec);
    let run = last_run(&w.ok(r));
    w.answer_x(run, Kind::NoViolation, &[], serde_json::json!({"disclosed": [{"round": 0, "tag": tag(1), "key": H32([4; 32])}]}));
    assert!(w.program(p).disclosed.is_empty(), "a T0 key is not trusted");
    w.t1(run);
    assert_eq!(w.program(p).disclosed, vec![(0, tag(1), H32([4; 32]))]);
}

#[test]
fn a_fix_that_does_not_ship_discloses_nothing() {
    // Round 1's code stops A's bug, but a Critical duplicate still breaks it, so the release gate fails:
    // the tag is not "fixed" for disclosure, and A's working proof stays sealed.
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    let (c1, _) = w.finding(p, H2, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::NoViolation, &[]);
    w.rerun(c1, Kind::Violation, &[(A, tag(1))]);
    w.tick(61);
    let fx = w.open_round(p).unwrap();
    assert!(!fx.iter().any(|f| matches!(f, Effect::ReleasePermit { .. })));
    assert!(!w.program(p).tag_fixed(0, &tag(1)), "fixed only when the fix ships");
    let ctx = w.ctx("anyone.testnet");
    let spec = w.spec();
    assert!(w.e.request_disclosure(&ctx, p, &[(0, tag(1))], spec).is_err());
}

#[test]
fn a_released_fix_discloses_its_round_only() {
    // The fix ships, so round 0's proofs of the tag may be disclosed; a round-1 proof of the same tag
    // (a new bug behind the same entry point) needs its own round's condition.
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let (c0, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true);
    w.allocate(p).unwrap();
    w.pay(c0).unwrap();
    w.propose(p, vec![], true);
    w.rerun(c0, Kind::NoViolation, &[]);
    w.tick(61);
    let fx = w.open_round(p).unwrap();
    assert!(fx.contains(&Effect::ReleasePermit { program: p, round: 1 }));
    assert!(w.program(p).tag_fixed(0, &tag(1)));
    assert!(!w.program(p).tag_fixed(1, &tag(1)));
    let ctx = w.ctx("anyone.testnet");
    let spec = w.spec();
    assert!(w.e.request_disclosure(&ctx, p, &[(1, tag(1))], spec).is_err(), "round 1's proofs stay sealed");
    let spec = w.spec();
    let r = w.e.request_disclosure(&ctx, p, &[(0, tag(1))], spec);
    w.ok(r);
}

#[test]
fn anchor_contradicted_refunds_escrow() {
    let mut w = W::new();
    let ctx = w.ctx(PROJECT);
    let before = w.e.store.account(PROJECT).tokens;
    let (p, _) = w
        .e
        .create_program(&ctx, engine::NewProgram { project: PROJECT.into(), manifest_sha256: H32([7; 32]), params: params(), escrow: 500, anchor_uncheckable: false })
        .unwrap();
    let r = w.e.anchor_checked(p, Some(false));
    w.ok(r);
    assert_eq!(w.program(p).phase, Phase::Rejected);
    assert_eq!(w.e.store.account(PROJECT).tokens, before + 500);
    w.audit_ok(&[p]);
}

#[test]
fn copies_of_a_held_proof_take_one_reservation() {
    // The project reads A's paid proof and commits copies from other accounts: each is a duplicate
    // with its bond refunded, but only the first reserves a promotion, so later holders are still paid.
    let mut w = W::new();
    let p = w.program_open(600, params());
    let (a, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true); // holder 200
    w.allocate(p).unwrap();
    w.pay(a).unwrap();
    let copies: Vec<_> = (0..3).map(|_| w.finding(p, H2, Kind::Violation, &[(A, tag(1))], true).0).collect();
    let (b, _) = w.finding(p, H1, Kind::Violation, &[(B, tag(2))], true); // holder 100
    w.allocate(p).unwrap();
    let reserved: Vec<_> = copies.iter().map(|c| match w.commit(*c).state {
        CommitState::Duplicate { reserved, .. } => reserved,
        s => panic!("expected a duplicate, got {s:?}"),
    }).collect();
    assert_eq!(reserved, vec![200, 0, 0], "one promotion reservation per key");
    assert!(matches!(w.commit(b).state, CommitState::Holder { reserved: 100, debt: 0, .. }));
    w.pay(b).unwrap();
    w.audit_ok(&[p]);
}

#[test]
fn exit_funds_owed_claims_before_releasing_duplicates() {
    // An open duplicate's reservation cannot leave a holder in debt at exit: it goes back first.
    let mut w = W::new();
    let p = w.program_open(300, params());
    let (a, _) = w.finding(p, H1, Kind::Violation, &[(A, tag(1))], true); // holder 200
    let (d, _) = w.finding(p, H2, Kind::Violation, &[(A, tag(1))], true); // duplicate, reserves the last 100
    let (b, _) = w.finding(p, H1, Kind::Violation, &[(B, tag(2))], true); // holder 100, all in debt
    w.allocate(p).unwrap();
    w.pay(a).unwrap();
    assert!(matches!(w.commit(d).state, CommitState::Duplicate { reserved: 100, .. }));
    assert!(matches!(w.commit(b).state, CommitState::Holder { reserved: 0, debt: 100, .. }));
    w.tick(10_000 + 600);
    let r = w.e.exit(&w.ctx("anyone.testnet"), p);
    w.ok(r);
    assert!(matches!(w.end(b), Some(End::Paid { amount: 100, .. })), "the duplicate's reservation funded the debt");
    assert_eq!(w.end(d), Some(End::DuplicateConfirmed));
    assert_eq!(w.program(p).defaulted, 0);
    w.audit_ok(&[p]);
}

#[test]
fn one_gap_bug_is_promoted_once() {
    // Copies of one gap proof: the first takes the round's gap reservation; when a round adds the rule,
    // the earliest is promoted and paid, the copies are its duplicates.
    let mut w = W::new();
    let p = w.program_open(1_000, params());
    let mut claims = vec![];
    for h in [H1, H2, H1] {
        let (c, run) = w.commit_reveal(p, h);
        w.answer_x(run, Kind::NoViolation, &[], serde_json::json!({"protected_harm_usd_micro": "5000"}));
        w.t1(run);
        claims.push(c);
    }
    w.allocate(p).unwrap();
    let reserved: Vec<_> = claims.iter().map(|c| match w.commit(*c).state {
        CommitState::GapClaim { reserved, .. } => reserved,
        s => panic!("expected a gap claim, got {s:?}"),
    }).collect();
    assert_eq!(reserved, vec![20, 0, 0], "one gap reservation per round");
    w.propose(p, vec![SlotRule { slot: 7, severity: Severity::High, budget: 500 }], true);
    for c in &claims {
        w.rerun(*c, Kind::Violation, &[(7, tag(9))]);
    }
    w.tick(61);
    w.open_round(p).unwrap();
    w.pay(claims[0]).unwrap();
    assert!(matches!(w.end(claims[0]), Some(End::Paid { amount: 20, .. })));
    assert_eq!(w.end(claims[1]), Some(End::DuplicateConfirmed));
    assert_eq!(w.end(claims[2]), Some(End::DuplicateConfirmed));
    w.audit_ok(&[p]);
}

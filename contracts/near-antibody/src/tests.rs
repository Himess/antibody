//! Rule tests driven by the recorded fixture: the exact canonical manifest bytes and the fp_tags the
//! verifier produced for agents A–F and the project's known issue (fixtures/acme-local).

use near_sdk::{
    json_types::U128,
    serde_json::{self, json, Value},
    test_utils::VMContextBuilder,
    testing_env, AccountId, PromiseError,
};

use super::*;

const MANIFEST: &str = include_str!("../../../fixtures/acme-local/manifest.json");
const EXPECTED: &str = include_str!("../../../fixtures/acme-local/expected.json");
const ONE_TUSD: u128 = 1_000_000;

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn contract_id() -> AccountId {
    acc("antibody.testnet")
}
fn token() -> AccountId {
    acc("tusd.antibody-hq.testnet")
}
fn project() -> AccountId {
    acc("acme.antibody-hq.testnet")
}
fn owner() -> AccountId {
    acc("owner.testnet")
}
fn hunter(a: &str) -> AccountId {
    acc(&format!("agent-{}.testnet", a.to_lowercase()))
}

struct World {
    c: Antibody,
    height: u64,
    ts: u64,
}

fn manifest_json() -> Value {
    serde_json::from_str(MANIFEST).unwrap()
}

fn expected() -> Value {
    serde_json::from_str(EXPECTED).unwrap()
}

/// fp_tags recorded for one fixture submission: [(slot, tag)].
fn tags_of(agent: &str) -> Vec<(u8, String)> {
    expected()["submissions"][agent]["fp_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["slot"].as_u64().unwrap() as u8, t["tag"].as_str().unwrap().to_string()))
        .collect()
}

fn verifier() -> VerifierConfig {
    VerifierConfig {
        wasm_url: "https://github.com/Himess/antibody/releases/download/v0.4.0/antibody-verify.wasm".into(),
        wasm_hash: "23136cf2367e6b60a59c50cdb9b1af5d9a467acd11fbe4c75c40ad9b0aeac08b".into(),
        pubkey: Some("11".repeat(32)),
        secrets_owner: Some(acc("antibody-hq.testnet")),
        secrets_profile: Some("antibody-v1".into()),
        max_instructions: 5_000_000_000,
        max_memory_mb: 512,
        max_execution_seconds: 60,
        deposit: U128(10_000_000_000_000_000_000_000),
        tier: "T0".into(),
    }
}

impl World {
    fn new() -> Self {
        let deadline = manifest_json()["rules"]["deadline_unix"].as_u64().unwrap();
        let mut w = World { c: unsafe_default(), height: 1_000, ts: (deadline - 12 * 86_400) * 1_000_000_000 };
        w.ctx(owner());
        w.c = Antibody::new(owner(), token(), acc("outlayer.testnet"), None, verifier());
        w
    }

    fn ctx(&self, predecessor: AccountId) {
        let mut b = VMContextBuilder::new();
        b.current_account_id(contract_id())
            .predecessor_account_id(predecessor.clone())
            .signer_account_id(predecessor)
            .block_height(self.height)
            .block_timestamp(self.ts)
            .account_balance(near_sdk::NearToken::from_near(100))
            .prepaid_gas(near_sdk::Gas::from_tgas(1000));
        testing_env!(b.build());
    }

    fn advance(&mut self, blocks: u64) {
        self.height += blocks;
        self.ts += blocks * 600_000_000;
    }

    fn create_program(&mut self, guardian: Option<AccountId>) -> u32 {
        self.ctx(token());
        let msg = json!({"create_program": {"manifest": MANIFEST, "manifest_url": "https://example.org/manifest.json",
                                            "project_pubkey": "22".repeat(32), "guardian": guardian}})
        .to_string();
        let _ = self.c.ft_on_transfer(project(), U128(25_000 * ONE_TUSD), msg);
        self.c.get_program(0).map(|p| p.id).unwrap()
    }

    fn deposit(&mut self, who: &AccountId, amount: u128) {
        self.ctx(token());
        let _ = self.c.ft_on_transfer(who.clone(), U128(amount), json!({"deposit": {}}).to_string());
    }

    fn sealed(&self, who: &AccountId, salt: u8) -> SealedRef {
        let sha = hex(&near_sdk::env::sha256(format!("blob/{who}/{salt}").as_bytes()));
        SealedRef { url: format!("https://blobs.example.org/{sha}"), sha256: sha, wrap_v: "ab".repeat(92) }
    }

    /// commit + submit for `who`; returns the commit id.
    fn commit_and_submit(&mut self, who: &AccountId, salt: u8) -> u64 {
        let s = self.sealed(who, salt);
        let h = commit_hash_for(&format!("{}/0", contract_id()), who.as_str(), &s.sha256);
        self.ctx(who.clone());
        let id = self.c.commit(0, h);
        self.ctx(who.clone());
        let _ = self.c.submit(id, s);
        id
    }

    fn facts(&self, commit_id: u64, kind: Kind, tags: &[(u8, String)], impact: u128) -> FactsIn {
        let bits: Vec<u8> = tags.iter().map(|t| t.0).collect();
        FactsIn {
            v: 1,
            program: format!("{}/0", contract_id()),
            commit_id,
            poc_hash: "33".repeat(32),
            kind,
            broken_mask: mask_hex(&bits),
            fp_tags: tags.iter().map(|(s, t)| FpTagIn { slot: *s, tag: t.clone() }).collect(),
            impact_usd_micro: impact.to_string(),
            project_wrap: if kind == Kind::Violation { Some("44".repeat(80)) } else { None },
            receipt_hash: hex(&near_sdk::env::sha256(format!("receipt/{commit_id}").as_bytes())),
            manifest_hash: hex(&near_sdk::env::sha256(MANIFEST.as_bytes())),
            wasm_hash: None,
        }
    }

    /// Facts for a round-`r` commit or re-run (`manifest_hash` of that round's manifest).
    fn facts_at(&self, commit_id: u64, kind: Kind, tags: &[(u8, String)], manifest_hash: &str, poc_hash: &str) -> FactsIn {
        let mut f = self.facts(commit_id, kind, tags, 0);
        f.manifest_hash = manifest_hash.to_string();
        f.poc_hash = poc_hash.to_string();
        f.project_wrap = None;
        f
    }

    fn verdict(&mut self, commit_id: u64, f: FactsIn) {
        self.ctx(contract_id());
        self.c.on_verified(commit_id, Ok(Some(f)));
    }

    fn outcome(&self, id: u64) -> Outcome {
        self.c.get_commit(id).unwrap().outcome
    }

    fn finalize(&mut self, id: u64) {
        self.ctx(acc("anyone.testnet"));
        let _ = self.c.finalize(id);
    }
}

#[allow(clippy::uninit_assumed_init, invalid_value)]
fn unsafe_default() -> Antibody {
    // Replaced immediately by `Antibody::new` in `World::new`; never used before that.
    Antibody {
        owner: owner(),
        token: token(),
        outlayer: acc("outlayer.testnet"),
        light_client: None,
        verifier: verifier(),
        programs: Vector::new(b"P"),
        commits: Vector::new(b"C"),
        claims: LookupMap::new(b"K"),
        program_claims: LookupMap::new(b"Q"),
        balances: LookupMap::new(b"B"),
        reputation: LookupMap::new(b"R"),
        hunters: Vector::new(b"H"),
        stats: Stats::default(),
    }
}

fn setup_open_program_with_known_issue(guardian: Option<AccountId>) -> (World, u128) {
    let mut w = World::new();
    w.create_program(guardian);
    let p = w.c.get_program(0).unwrap();
    assert_eq!(p.status, ProgramStatus::KnownIssues);
    assert!(matches!(p.anchor_trust, AnchorTrust::ProjectAttested { .. }));
    assert_eq!(p.manifest_hash, hex(&near_sdk::env::sha256(MANIFEST.as_bytes())));
    // t0: the project's known issue (dust withdrawals) claims (share_price, AcmeVault.withdraw).
    let s = w.sealed(&project(), 0);
    w.ctx(project());
    let _ = w.c.submit_known(0, s);
    let known = w.facts(0, Kind::Violation, &tags_of("KNOWN"), 0);
    w.verdict(0, known);
    assert_eq!(w.outcome(0), Outcome::KnownIssue);
    w.ctx(project());
    w.c.open_program(0);
    assert_eq!(w.c.get_program(0).unwrap().status, ProgramStatus::Open);
    let bond = w.c.get_program(0).unwrap().bond.0;
    for a in ["A", "B", "C", "D", "E", "F"] {
        w.deposit(&hunter(a), 100 * ONE_TUSD);
    }
    (w, bond)
}

#[test]
fn six_scripted_submissions_follow_the_committed_rules() {
    let (mut w, bond) = setup_open_program_with_known_issue(Some(acc("guardian.antibody-hq.testnet")));
    let a = w.commit_and_submit(&hunter("A"), 1);
    let b = w.commit_and_submit(&hunter("B"), 1);
    let c = w.commit_and_submit(&hunter("C"), 1);
    w.advance(5);
    let d = w.commit_and_submit(&hunter("D"), 1);
    let e = w.commit_and_submit(&hunter("E"), 1);
    let f = w.commit_and_submit(&hunter("F"), 1);
    assert_eq!((a, b, c, d, e, f), (1, 2, 3, 4, 5, 6));

    // Facts exactly as the verifier produced them for the fixture.
    w.verdict(a, w.facts(a, Kind::Violation, &tags_of("A"), 462_931_853_867));
    assert!(w.c.get_program(0).unwrap().pause_requested, "a verified Critical triggers the guardian");
    w.verdict(b, w.facts(b, Kind::Violation, &tags_of("B"), 1_000_000_000));
    w.verdict(c, w.facts(c, Kind::NoViolation, &[], 33_480_361));
    w.verdict(d, w.facts(d, Kind::Violation, &tags_of("D"), 6_375_600_000));
    w.verdict(e, w.facts(e, Kind::Violation, &tags_of("E"), 0));
    w.verdict(f, w.facts(f, Kind::Violation, &tags_of("F"), 10_400_000_000));

    // Bonds: refunded on any Violation, forfeited to the project on NoViolation.
    for x in ["A", "B", "D", "E", "F"] {
        assert_eq!(w.c.get_balance(hunter(x)).0, 100 * ONE_TUSD, "{x} bond refunded");
    }
    assert_eq!(w.c.get_balance(hunter("C")).0, 100 * ONE_TUSD - bond, "C forfeits the bond");
    assert_eq!(w.c.get_balance(project()).0, bond, "the forfeited bond goes to the project");
    assert_eq!(w.outcome(c), Outcome::Rejected { kind: Kind::NoViolation });

    // Finalization waits for every earlier reveal window.
    w.ctx(acc("anyone.testnet"));
    let early = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.c.finalize(a);
    }));
    assert!(early.is_err(), "finalize must wait for the reveal window");
    let window = w.c.get_program(0).unwrap().reveal_window_blocks;
    w.advance(window + 10);
    for id in [a, b, d, e, f] {
        w.finalize(id);
    }

    // A: Critical 10,000 tUSD > instant cap 5,000 => T0 optimistic delay, then release.
    let delayed = w.outcome(a);
    assert!(matches!(delayed, Outcome::Delayed { amount, severity: Severity::Critical, .. } if amount.0 == 10_000 * ONE_TUSD));
    // B: High 3,000 tUSD <= cap => paid at once.
    assert!(matches!(w.outcome(b), Outcome::Paid { amount, severity: Severity::High, .. } if amount.0 == 3_000 * ONE_TUSD));
    // D: same (slot, tag) as A, committed later => Duplicate of A, no payout.
    assert_eq!(w.outcome(d), Outcome::Duplicate { of: a });
    // E: key claimed by the project's known issue at t0 => Known.
    assert_eq!(w.outcome(e), Outcome::Known { of: 0 });
    // F: solvency again but a different function (repay) => a new key, paid.
    assert!(matches!(w.outcome(f), Outcome::Delayed { severity: Severity::Critical, .. }));

    let proof = w.c.get_duplicate_proof(d).unwrap();
    assert_eq!(proof["earlier"]["id"], json!(a));
    assert_eq!(proof["equal_keys"].as_array().unwrap().len(), 1);
    assert_eq!(proof["disclosed"], json!(false));
    assert!(w.c.get_duplicate_proof(e).is_some(), "known issues show the t0 claim too");

    // Release after the challenge delay.
    w.ts += 301 * 1_000_000_000;
    w.ctx(acc("anyone.testnet"));
    let _ = w.c.release(a);
    w.ctx(acc("anyone.testnet"));
    let _ = w.c.release(f);
    assert!(matches!(w.outcome(a), Outcome::Paid { amount, .. } if amount.0 == 10_000 * ONE_TUSD));
    assert!(matches!(w.outcome(f), Outcome::Paid { .. }));

    let p = w.c.get_program(0).unwrap();
    assert_eq!(p.paid.0, 23_000 * ONE_TUSD);
    assert_eq!(p.reserved.0, 0);
    let s = w.c.get_stats();
    assert_eq!((s.paid_findings, s.duplicates, s.known, s.slop_rejected), (3, 1, 1, 1));
    let keys = w.c.get_keys(0, None);
    assert_eq!(keys.len(), 4, "known(share_price) + A + B + F");
    let rep_d = w.c.get_reputation(hunter("D"));
    assert_eq!((rep_d.late, rep_d.earned.0), (1, 0));
    let board = w.c.leaderboard(None);
    assert_eq!(board[0].0, hunter("A"));
}

#[test]
fn an_earlier_commit_revealed_later_still_wins() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    // A commits first but reveals after D (same key).
    let sa = w.sealed(&hunter("A"), 7);
    w.ctx(hunter("A"));
    let a = w.c.commit(0, commit_hash_for("antibody.testnet/0", hunter("A").as_str(), &sa.sha256));
    let d = w.commit_and_submit(&hunter("D"), 7);
    w.verdict(d, w.facts(d, Kind::Violation, &tags_of("D"), 1));
    w.ctx(hunter("A"));
    let _ = w.c.submit(a, sa);
    w.verdict(a, w.facts(a, Kind::Violation, &tags_of("A"), 1));
    w.advance(1_000);
    w.finalize(d);
    assert_eq!(w.outcome(d), Outcome::Duplicate { of: a });
    w.finalize(a);
    assert!(matches!(w.outcome(a), Outcome::Delayed { .. } | Outcome::Paid { .. }));
}

#[test]
fn finalize_waits_for_earlier_verifications() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    let a = w.commit_and_submit(&hunter("A"), 2); // stays Verifying
    let b = w.commit_and_submit(&hunter("B"), 2);
    w.verdict(b, w.facts(b, Kind::Violation, &tags_of("B"), 1));
    w.advance(1_000);
    w.ctx(acc("anyone.testnet"));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.c.finalize(b);
    }));
    assert!(r.is_err(), "A (earlier) is still being verified");
    w.verdict(a, w.facts(a, Kind::NoViolation, &[], 0));
    w.finalize(b);
    assert!(matches!(w.outcome(b), Outcome::Paid { .. }));
}

#[test]
fn unrevealed_commits_forfeit_and_late_reveals_fail() {
    let (mut w, bond) = setup_open_program_with_known_issue(None);
    let s = w.sealed(&hunter("A"), 3);
    w.ctx(hunter("A"));
    let a = w.c.commit(0, commit_hash_for("antibody.testnet/0", hunter("A").as_str(), &s.sha256));
    w.advance(1_000);
    w.ctx(hunter("A"));
    let late = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.c.submit(a, s.clone());
    }));
    assert!(late.is_err(), "reveal after the window must fail");
    w.ctx(acc("anyone.testnet"));
    w.c.expire(a);
    assert_eq!(w.outcome(a), Outcome::Forfeited);
    assert_eq!(w.c.get_balance(project()).0, bond);
    assert_eq!(w.c.get_reputation(hunter("A")).unrevealed, 1);
}

#[test]
fn reveal_must_match_the_commitment_and_the_committer() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    let s = w.sealed(&hunter("A"), 4);
    w.ctx(hunter("A"));
    let a = w.c.commit(0, commit_hash_for("antibody.testnet/0", hunter("A").as_str(), &s.sha256));
    // Another account cannot reveal A's commit.
    w.ctx(hunter("B"));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.c.submit(a, s.clone());
    }))
    .is_err());
    // A different blob does not match the commitment.
    let other = w.sealed(&hunter("A"), 5);
    w.ctx(hunter("A"));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.c.submit(a, other);
    }))
    .is_err());
}

#[test]
fn malformed_or_mismatched_facts_are_errors_not_payouts() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    let a = w.commit_and_submit(&hunter("A"), 6);
    let mut f = w.facts(a, Kind::Violation, &tags_of("A"), 1);
    f.manifest_hash = "00".repeat(32);
    w.verdict(a, f);
    assert!(matches!(w.outcome(a), Outcome::Error { .. }));
    let b = w.commit_and_submit(&hunter("B"), 6);
    let mut f = w.facts(b, Kind::Violation, &tags_of("B"), 1);
    f.broken_mask = "0x3".into();
    w.verdict(b, f);
    assert!(matches!(w.outcome(b), Outcome::Error { .. }));
    let c = w.commit_and_submit(&hunter("C"), 6);
    w.verdict(c, w.facts(c, Kind::Violation, &[(9, "aa".repeat(32))], 1));
    assert!(matches!(w.outcome(c), Outcome::Error { .. }), "undeclared invariant");
    for x in ["A", "B", "C"] {
        assert_eq!(w.c.get_balance(hunter(x)).0, 100 * ONE_TUSD, "{x}: bond refunded on error");
    }
}

#[test]
fn null_results_are_retried_then_given_up() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    let a = w.commit_and_submit(&hunter("A"), 8);
    w.ctx(contract_id());
    w.c.on_verified(a, Ok(None));
    assert_eq!(w.c.get_commit(a).unwrap().status, CommitStatus::VerifyFailed);
    for _ in 0..2 {
        w.ctx(acc("anyone.testnet"));
        let _ = w.c.retry_verify(a);
        w.ctx(contract_id());
        w.c.on_verified(a, Err(PromiseError::Failed));
    }
    assert!(matches!(w.outcome(a), Outcome::Error { .. }));
    assert_eq!(w.c.get_balance(hunter("A")).0, 100 * ONE_TUSD);
}

#[test]
fn challenge_freezes_a_delayed_payout_for_the_arbiter() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    let a = w.commit_and_submit(&hunter("A"), 9);
    w.verdict(a, w.facts(a, Kind::Violation, &tags_of("A"), 1));
    w.advance(1_000);
    w.finalize(a);
    w.ctx(acc("watcher.testnet"));
    w.c.challenge(a, "local re-run disagrees".into());
    assert!(matches!(w.outcome(a), Outcome::Challenged { .. }));
    w.ts += 10_000 * 1_000_000_000;
    w.ctx(acc("anyone.testnet"));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.c.release(a);
    }))
    .is_err());
    w.ctx(owner());
    let _ = w.c.resolve_challenge(a, false, "receipt mismatch confirmed".into());
    assert!(matches!(w.outcome(a), Outcome::Cancelled { .. }));
    assert_eq!(w.c.get_program(0).unwrap().reserved.0, 0);
}

#[test]
fn light_client_anchor_check() {
    let mut w = World::new();
    w.ctx(owner());
    w.c.set_light_client(Some(acc("client-eth2.sepolia.testnet")));
    // A Sepolia variant of the fixture manifest.
    let mut m = manifest_json();
    m["anchor"]["chain_id"] = json!(SEPOLIA);
    let manifest = serde_json::to_string(&m).unwrap();
    let anchor_hash = m["anchor"]["block_hash"].as_str().unwrap().to_string();
    let create = |w: &mut World| {
        w.ctx(token());
        let msg = json!({"create_program": {"manifest": manifest, "manifest_url": "https://example.org/m.json",
                                            "project_pubkey": "22".repeat(32), "guardian": null}})
        .to_string();
        let _ = w.c.ft_on_transfer(project(), U128(1_000 * ONE_TUSD), msg);
    };
    let mut h = [0u8; 32];
    h.copy_from_slice(&(0..32).map(|i| u8::from_str_radix(&anchor_hash[2 + 2 * i..4 + 2 * i], 16).unwrap()).collect::<Vec<_>>());
    create(&mut w);
    w.ctx(contract_id());
    assert_eq!(w.c.on_anchor_checked(0, acc("client-eth2.sepolia.testnet"), Ok(Some(h))).0, 0);
    assert!(matches!(w.c.get_program(0).unwrap().anchor_trust, AnchorTrust::LightClient { .. }));
    create(&mut w);
    w.ctx(contract_id());
    let refund = w.c.on_anchor_checked(1, acc("client-eth2.sepolia.testnet"), Ok(Some([7u8; 32])));
    assert_eq!(refund.0, 1_000 * ONE_TUSD, "a forged anchor refunds the escrow");
    assert!(matches!(w.c.get_program(1).unwrap().status, ProgramStatus::Rejected { .. }));
    create(&mut w);
    w.ctx(contract_id());
    assert_eq!(w.c.on_anchor_checked(2, acc("client-eth2.sepolia.testnet"), Ok(None)).0, 0);
    assert!(matches!(w.c.get_program(2).unwrap().anchor_trust, AnchorTrust::ProjectAttested { .. }));
}

#[test]
fn disclosure_publishes_content_keys_for_duplicate_proofs() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    let a = w.commit_and_submit(&hunter("A"), 11);
    let d = w.commit_and_submit(&hunter("D"), 11);
    w.verdict(a, w.facts(a, Kind::Violation, &tags_of("A"), 1));
    w.verdict(d, w.facts(d, Kind::Violation, &tags_of("D"), 1));
    w.advance(1_000);
    w.finalize(d);
    let p = w.c.get_program(0).unwrap();
    w.ts = p.deadline_ns + p.disclosure_after_ns + 1;
    w.ctx(acc("anyone.testnet"));
    let _ = w.c.disclose(0);
    w.ctx(contract_id());
    let keys = vec![
        DisclosedKey { commit_id: a, round: None, content_key: "aa".repeat(32) },
        DisclosedKey { commit_id: d, round: None, content_key: "dd".repeat(32) },
    ];
    w.c.on_disclosed(0, Ok(Some(DisclosureIn { program: "antibody.testnet/0".into(), k_program: "cc".repeat(32), keys })));
    let proof = w.c.get_duplicate_proof(d).unwrap();
    assert_eq!(proof["disclosed"], json!(true));
    assert_eq!(proof["earlier_content_key"], json!("aa".repeat(32)));
    assert_eq!(proof["k_program"], json!("cc".repeat(32)));
}

#[test]
fn escrow_stays_locked_while_commits_are_pending() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    let a = w.commit_and_submit(&hunter("A"), 12);
    w.verdict(a, w.facts(a, Kind::Violation, &tags_of("A"), 1));
    let p = w.c.get_program(0).unwrap();
    w.ts = p.deadline_ns + 1;
    w.ctx(project());
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.c.withdraw_escrow(0);
    }))
    .is_err());
    w.advance(1_000);
    w.finalize(a);
    w.ts += 10_000 * 1_000_000_000;
    w.ctx(acc("anyone.testnet"));
    let _ = w.c.release(a);
    w.ctx(project());
    let _ = w.c.withdraw_escrow(0);
    let p = w.c.get_program(0).unwrap();
    assert!(p.escrow_withdrawn);
    assert_eq!(p.escrow.0, p.paid.0);
}

#[test]
fn commit_needs_an_open_program_and_a_bond() {
    let mut w = World::new();
    w.create_program(None);
    let s = w.sealed(&hunter("A"), 1);
    w.deposit(&hunter("A"), 100 * ONE_TUSD);
    w.ctx(hunter("A"));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        w.c.commit(0, commit_hash_for("antibody.testnet/0", hunter("A").as_str(), &s.sha256));
    }))
    .is_err(), "not open yet (known-issues phase)");
    w.ctx(project());
    w.c.open_program(0);
    w.ctx(hunter("Z"));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        w.c.commit(0, "ab".repeat(32));
    }))
    .is_err(), "no internal balance for the bond");
}

#[test]
fn mask_hex_matches_the_verifier_format() {
    assert_eq!(mask_hex(&[]), "0x0");
    assert_eq!(mask_hex(&[0]), "0x1");
    assert_eq!(mask_hex(&[2]), "0x4");
    assert_eq!(mask_hex(&[0, 1, 2]), "0x7");
    assert_eq!(mask_hex(&[8]), "0x100");
    assert_eq!(mask_hex(&[254]), format!("0x4{}", "0".repeat(63)));
}

/// The round-1 manifest of the fixture: AcmeLend's runtime code replaced (the fix), nothing else.
fn round1_manifest() -> (String, String) {
    let mut m = manifest_json();
    let i = m["targets"].as_array().unwrap().iter().position(|t| t["name"] == "AcmeLend").unwrap();
    let code = [0x60u8, 0x00, 0x60, 0x00, 0xf3];
    let h = format!("0x{}", hex(&near_sdk::env::keccak256(&code)));
    m["targets"][i]["code_hash"] = json!(h);
    m["round"] = json!(1);
    m["overrides"] = json!([{"address": m["targets"][i]["address"], "code_hash": h, "runtime_code": format!("0x{}", hex(&code))}]);
    let s = serde_json::to_string(&m).unwrap();
    let mh = hex(&near_sdk::env::sha256(s.as_bytes()));
    (s, mh)
}

fn panics(f: impl FnOnce()) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err()
}

#[test]
fn rounds_release_fixed_keys_and_carry_the_rest() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    for x in ["G", "H", "K"] {
        w.deposit(&hunter(x), 100 * ONE_TUSD);
    }
    let a = w.commit_and_submit(&hunter("A"), 1);
    let b = w.commit_and_submit(&hunter("B"), 1);
    let f = w.commit_and_submit(&hunter("F"), 1);
    w.verdict(a, w.facts(a, Kind::Violation, &tags_of("A"), 0));
    w.verdict(b, w.facts(b, Kind::Violation, &tags_of("B"), 0));
    w.verdict(f, w.facts(f, Kind::Violation, &tags_of("F"), 0));
    let window = w.c.get_program(0).unwrap().reveal_window_blocks;
    w.advance(window + 1);
    for id in [a, b, f] {
        w.finalize(id);
    }

    // The project ships the fix for A's bug as a code override at the same anchor.
    let (r1, r1_hash) = round1_manifest();
    w.ctx(acc("mallory.testnet"));
    assert!(panics(|| { let _ = w.c.request_rerun(a, w.sealed(&project(), 9)); }), "no proposed round yet");
    w.ctx(project());
    w.c.propose_round(0, MANIFEST.into(), r1.clone(), "https://example.org/round1.json".into());
    let p = w.c.get_program(0).unwrap();
    assert_eq!((p.round, p.open_round, p.rounds[0].status.clone()), (1, 0, RoundStatus::Proposed));
    assert_eq!(p.rounds[0].overrides[0].name, "AcmeLend");

    // A new hunter finds a second, different bug in AcmeLend.borrow at the fixed state (same key as A).
    let g = w.commit_and_submit(&hunter("G"), 1);
    assert_eq!(w.c.get_commit(g).unwrap().round, 1);
    w.verdict(g, w.facts_at(g, Kind::Violation, &tags_of("A"), &r1_hash, &"77".repeat(32)));
    w.advance(window + 1);
    w.ctx(acc("anyone.testnet"));
    assert!(panics(|| {
        let _ = w.c.finalize(g);
    }), "round 1 is not open yet");

    // Re-runs: only the project or the finder, only claim holders.
    w.ctx(acc("mallory.testnet"));
    assert!(panics(|| { let _ = w.c.request_rerun(a, w.sealed(&project(), 9)); }), "strangers cannot re-run");
    w.ctx(project());
    let _ = w.c.request_rerun(a, w.sealed(&project(), 9));
    w.ctx(project());
    assert!(panics(|| { let _ = w.c.request_rerun(a, w.sealed(&project(), 10)); }), "one re-run in flight");
    let poc = "33".repeat(32); // the original verdicts' poc_hash in these tests
    w.ctx(contract_id());
    w.c.on_rerun(a, 1, Ok(Some(w.facts_at(a, Kind::NoViolation, &[], &r1_hash, &poc))));
    w.ctx(hunter("F"));
    let _ = w.c.request_rerun(f, w.sealed(&hunter("F"), 9));
    w.ctx(contract_id());
    w.c.on_rerun(f, 1, Ok(Some(w.facts_at(f, Kind::Violation, &tags_of("F"), &r1_hash, &poc))));
    // A re-run that executed another PoC is no evidence: B's key will carry over.
    w.ctx(project());
    let _ = w.c.request_rerun(b, w.sealed(&project(), 11));
    w.ctx(contract_id());
    w.c.on_rerun(b, 1, Ok(Some(w.facts_at(b, Kind::NoViolation, &[], &r1_hash, &"55".repeat(32)))));
    let rb = w.c.get_commit(b).unwrap().reruns[0].clone();
    assert!(matches!(rb.status, RerunStatus::Failed { ref reason } if reason.contains("another PoC")));
    let ra = w.c.get_commit(a).unwrap().reruns[0].clone();
    assert_eq!((ra.status, ra.kind, ra.still_broken.len()), (RerunStatus::Done, Some(Kind::NoViolation), 0));

    w.ctx(acc("mallory.testnet"));
    assert!(panics(|| w.c.open_round(0)), "only the project opens a round");
    w.ctx(project());
    w.c.open_round(0);
    let p = w.c.get_program(0).unwrap();
    assert_eq!((p.open_round, p.rounds[0].carried, p.rounds[0].released), (1, 3, 1));
    let keys = w.c.get_keys(0, Some(1));
    assert_eq!(keys.len(), 4, "known + B + F carried, A's key now held by G");
    let a_key = &tags_of("A")[0];
    let holder = keys.iter().find(|k| k["slot"] == json!(a_key.0) && k["tag"] == json!(a_key.1)).unwrap();
    assert_eq!(holder["commit"], json!(g));
    assert_eq!(w.c.get_keys(0, Some(0)).len(), 4, "round-0 claims are kept as history");

    // G is paid: the key was released by A's re-run. A still breaking key (F) and the known issue
    // stay claimed in round 1.
    w.finalize(g);
    // Critical, capped by the escrow still uncommitted (25,000 - B 3,000 - A and F 10,000 each).
    assert!(matches!(w.outcome(g), Outcome::Paid { amount, severity: Severity::Critical, .. } if amount.0 == 2_000 * ONE_TUSD));
    let h = w.commit_and_submit(&hunter("H"), 1);
    w.verdict(h, w.facts_at(h, Kind::Violation, &tags_of("F"), &r1_hash, &"88".repeat(32)));
    let k = w.commit_and_submit(&hunter("K"), 1);
    w.verdict(k, w.facts_at(k, Kind::Violation, &tags_of("KNOWN"), &r1_hash, &"99".repeat(32)));
    w.advance(window + 1);
    w.finalize(h);
    w.finalize(k);
    assert_eq!(w.outcome(h), Outcome::Duplicate { of: f });
    assert_eq!(w.outcome(k), Outcome::Known { of: 0 });
    // Disclosure covers re-run blobs too.
    let p = w.c.get_program(0).unwrap();
    w.ts = p.deadline_ns + p.disclosure_after_ns + 1;
    w.ctx(acc("anyone.testnet"));
    let _ = w.c.disclose(0);
}

#[test]
fn round_manifests_may_only_change_target_code() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    let (r1, _) = round1_manifest();
    let tweak = |f: &dyn Fn(&mut Value)| {
        let mut m: Value = serde_json::from_str(&r1).unwrap();
        f(&mut m);
        serde_json::to_string(&m).unwrap()
    };
    let lower_bond = tweak(&|m| m["rules"]["bond"] = json!("1"));
    let cheaper = tweak(&|m| m["payout"]["critical"] = json!("1"));
    let new_anchor = tweak(&|m| m["anchor"]["block_number"] = json!(23));
    let wrong_round = tweak(&|m| m["round"] = json!(2));
    let bad_hash = tweak(&|m| m["overrides"][0]["code_hash"] = json!(format!("0x{}", "00".repeat(32))));
    let no_override = tweak(&|m| m["overrides"] = json!([]));
    for (bad, why) in [
        (&lower_bond, "rules"),
        (&cheaper, "payouts"),
        (&new_anchor, "anchor"),
        (&wrong_round, "round number"),
        (&bad_hash, "override hash"),
        (&no_override, "no override"),
    ] {
        w.ctx(project());
        assert!(panics(|| w.c.propose_round(0, MANIFEST.into(), bad.clone(), "https://e.org/r.json".into())), "{why} must not change");
    }
    w.ctx(project());
    assert!(panics(|| w.c.propose_round(0, r1.clone(), r1.clone(), "https://e.org/r.json".into())), "base must be the program's manifest");
    w.ctx(hunter("A"));
    assert!(panics(|| w.c.propose_round(0, MANIFEST.into(), r1.clone(), "https://e.org/r.json".into())), "only the project");
    w.ctx(project());
    w.c.propose_round(0, MANIFEST.into(), r1.clone(), "https://e.org/r.json".into());
    w.ctx(project());
    assert!(panics(|| w.c.propose_round(0, MANIFEST.into(), r1.clone(), "https://e.org/r.json".into())), "one proposed round at a time");
}

#[test]
fn rounds_wait_for_earlier_commits_and_need_a_claim_to_rerun() {
    let (mut w, _) = setup_open_program_with_known_issue(None);
    let a = w.commit_and_submit(&hunter("A"), 1);
    let d = w.commit_and_submit(&hunter("D"), 1);
    w.verdict(a, w.facts(a, Kind::Violation, &tags_of("A"), 0));
    w.verdict(d, w.facts(d, Kind::Violation, &tags_of("D"), 0));
    let (r1, _) = round1_manifest();
    w.ctx(project());
    w.c.propose_round(0, MANIFEST.into(), r1, "https://example.org/round1.json".into());
    w.ctx(project());
    assert!(panics(|| w.c.open_round(0)), "A and D are not finalized yet");
    let window = w.c.get_program(0).unwrap().reveal_window_blocks;
    w.advance(window + 1);
    w.finalize(a);
    w.finalize(d);
    assert_eq!(w.outcome(d), Outcome::Duplicate { of: a });
    w.ctx(project());
    assert!(panics(|| { let _ = w.c.request_rerun(d, w.sealed(&project(), 3)); }), "a duplicate holds no key");
    // Never re-run: everything carries over.
    w.ctx(project());
    w.c.open_round(0);
    let p = w.c.get_program(0).unwrap();
    assert_eq!((p.rounds[0].carried, p.rounds[0].released), (2, 0));
}

// near-sdk 5.9's mocked blockchain does not implement yield/resume, yet `env` references the host
// functions. ELF links tolerate the unused references; the MSVC linker does not, so give the
// Windows unit-test binary inert definitions (never called by these tests).
#[cfg(windows)]
#[no_mangle]
pub extern "C" fn promise_yield_create(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> u64 {
    unreachable!("yield/resume is not used by near-antibody")
}
#[cfg(windows)]
#[no_mangle]
pub extern "C" fn promise_yield_resume(_: u64, _: u64, _: u64, _: u64) -> u32 {
    unreachable!("yield/resume is not used by near-antibody")
}

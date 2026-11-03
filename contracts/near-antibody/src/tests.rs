//! Adapter tests. The rules themselves are tested in `crates/rules` (scenarios, and replay against the
//! Quint model); these cover what near-antibody adds: requests built from its own records, the
//! OutLayer call, callbacks that never panic, the T1 binding and image pins, storage paid from NEAR
//! credit, and credit-then-withdraw.

use near_sdk::{
    mock::MockAction,
    test_utils::{get_created_receipts, get_logs, VMContextBuilder},
    testing_env, RuntimeFeesConfig,
};

use super::*;
use antibody_rules::{CommitState, Kind, Phase, Tier};

const ONE: u128 = 1_000_000;
const NEAR: u128 = 1_000_000_000_000_000_000_000_000;
const RUN_DEPOSIT: u128 = 20_000_000_000_000_000_000_000;
const START: u64 = 1_800_000_000;
const ESCROW: u128 = 50_000 * ONE;
const BOND: u128 = 10 * ONE;
const CRITICAL: u128 = 5_000 * ONE;
const EPOCH: &str = "00112233aabbccdd";

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}
fn me() -> AccountId {
    acc("antibody.testnet")
}
fn token() -> AccountId {
    acc("tusd.antibody.testnet")
}
fn project() -> AccountId {
    acc("acme.testnet")
}
fn alice() -> AccountId {
    acc("alice.testnet")
}
fn wasm_hash() -> String {
    "ab".repeat(32)
}
fn terms() -> String {
    "cc".repeat(32)
}

/// A worker image whose five registers are the bytes n, n+1, …, n+4.
fn image(n: u8) -> Measurements {
    let h = |k: u8| format!("{:02x}", n + k).repeat(48);
    Measurements { mrtd: h(0), rtmr0: h(1), rtmr1: h(2), rtmr2: h(3), rtmr3: h(4) }
}

fn defaults(hash: &str) -> Defaults {
    Defaults {
        verifier: VerifierPin {
            wasm_url: "https://example.org/antibody-verify.wasm".into(),
            wasm_hash: hash.into(),
            max_instructions: 10_000_000_000,
            max_memory_mb: 512,
            max_execution_seconds: 60,
            deposit: U128(RUN_DEPOSIT),
        },
        outlayer: acc("outlayer.testnet"),
        dcap_verifier: acc("tee-verifier.testnet"),
        accepted_tcb: vec!["UpToDate".into()],
        image_delay_secs: 7 * 86_400,
        light_client: None,
        light_client_chain_id: 11155111,
        guardian: None,
        treasury: acc("treasury.testnet"),
        image_registry: acc("worker.outlayer.testnet"),
    }
}

fn manifest(schema: &str, project: &AccountId) -> String {
    json!({
        "schema": schema,
        "program": {"name": "Acme Lend (test)", "project": project, "description": "A test fixture with a planted bug."},
        "anchor": {"chain_id": 31337, "block_number": 22, "evm_spec": "osaka", "trust": "project-attested",
                   "block_hash": "0x44fa77658758a4b2b25756b8c833664247c4e2bada183160084e83a59fdc54e0"},
        "targets": [{"name": "Pool", "address": "0x1fca3c5c59c22727fa8052158f737f4fc4c3062e", "code_hash": format!("0x{}", "11".repeat(32))}],
        "invariants": {"gas_limit": 1_000_000, "slots": [
            {"bit": 0, "name": "solvency", "severity": "critical"},
            {"bit": 1, "name": "accounting", "severity": "high"}]},
        "settlement": {
            "payouts": {"critical": CRITICAL.to_string(), "high": (1_000 * ONE).to_string(), "medium": (100 * ONE).to_string(),
                        "low": (10 * ONE).to_string(), "impact_bps": 0, "impact_cap": "0", "decimals": 6},
            "instant": {"cap": CRITICAL.to_string(), "budget": (10_000 * ONE).to_string(), "window_secs": 86_400},
            "fees": {"protocol_bps": 100, "release": "0"},
            "bond": BOND.to_string(),
            "challenge_bond": (50 * ONE).to_string(),
            "windows": {"reveal": 3_600, "challenge": 1_800, "resolve": 3_600, "infra_grace": 3_600, "run_timeout": 600,
                        "round_min": 3_600, "rerun_window": 3_600, "disclosure_max_delay": 86_400,
                        "deadline": START + 10 * 86_400, "exit_grace": 86_400},
            "slot_budgets": [{"slot": 0, "budget": (20_000 * ONE).to_string()}],
            "unevaluable": "refund",
            "known": {"max_count": 4, "max_share_bps": 5_000},
            "pause": {"slots": [0], "min_impact_usd_micro": "0"},
            "terms_sha256": terms(),
        }
    })
    .to_string()
}

fn base() -> Loc {
    Loc { location: "near-tx:9Xq1base:acme.testnet".into(), sha256: "33".repeat(32) }
}

struct W {
    c: Contract,
    now: u64,
}

fn set_env(predecessor: &AccountId, signer: &AccountId, now: u64, deposit: u128, results: Vec<PromiseResult>) {
    let mut b = VMContextBuilder::new();
    b.current_account_id(me())
        .predecessor_account_id(predecessor.clone())
        .signer_account_id(signer.clone())
        .block_timestamp(now * 1_000_000_000)
        .attached_deposit(NearToken::from_yoctonear(deposit))
        .account_balance(NearToken::from_near(1_000))
        .prepaid_gas(Gas::from_tgas(300));
    testing_env!(b.build(), near_sdk::test_vm_config(), RuntimeFeesConfig::test(), Default::default(), results);
}

impl W {
    fn new() -> W {
        set_env(&acc("owner.testnet"), &acc("owner.testnet"), START, 0, vec![]);
        let c = Contract::new(acc("owner.testnet"), token(), defaults(&wasm_hash()));
        let mut w = W { c, now: START };
        // OutLayer's registry lists image 1 before any program exists.
        w.callback(vec![]);
        assert_eq!(w.c.on_images(Ok(vec![image(1)])), 1);
        w
    }

    fn call(&self, who: &AccountId) {
        set_env(who, who, self.now, 0, vec![]);
    }
    fn call_with(&self, who: &AccountId, deposit: u128) {
        set_env(who, who, self.now, deposit, vec![]);
    }
    fn token_call(&self, sender: &AccountId) {
        set_env(&token(), sender, self.now, 0, vec![]);
    }
    fn callback(&self, results: Vec<PromiseResult>) {
        set_env(&me(), &acc("relayer.testnet"), self.now, 0, results);
    }
    fn tick(&mut self, secs: u64) {
        self.now += secs;
    }

    fn register(&mut self, who: &AccountId) {
        self.call_with(who, NEAR);
        self.c.storage_deposit(None, None);
    }

    fn create(&mut self) -> u32 {
        self.register(&project());
        self.token_call(&project());
        let msg = json!({"create_program": {"manifest": manifest("antibody.manifest.v2", &project()),
                                           "manifest_url": "https://example.org/acme.json", "project_pk": "22".repeat(32)}});
        let _ = self.c.ft_on_transfer(project(), U128(ESCROW), msg.to_string());
        self.c.list_programs(None, None).last().unwrap().id
    }

    fn facts(&self, run_id: u64, kind: &str, keys: Value, extra: Value) -> Vec<u8> {
        let r = self.c.get_run(run_id).unwrap();
        let p = self.c.get_program(r.program).unwrap();
        let mut f = json!({
            "v": 2, "mode": r.mode, "request_sha256": r.request_sha256, "manifest_sha256": p.manifest_of(r.round),
            "program_key": p.key, "round": r.round,
            "key_epoch": if r.mode == Mode::Activate { EPOCH.to_string() } else { p.key_epoch.clone() },
            "commit_id": r.commit, "run_id": run_id, "kind": kind, "keys": keys, "receipt_sha256": "77".repeat(32),
        });
        if let Value::Object(m) = extra {
            for (k, v) in m {
                f[k] = v;
            }
        }
        serde_json::to_vec(&f).unwrap()
    }

    fn answer(&mut self, run_id: u64, out: &[u8]) {
        self.callback(vec![PromiseResult::Successful(out.to_vec())]);
        self.c.on_run(run_id);
    }

    fn attestation(&self, run_id: u64, out: &[u8]) -> tee::TeeAttestationIn {
        let r = self.c.get_run(run_id).unwrap();
        tee::TeeAttestationIn {
            task_type: "execute".into(),
            task_id: 42,
            repo: None,
            commit: None,
            build_target: Some("wasm32-wasip2".into()),
            wasm_hash: Some(wasm_hash()),
            input_hash: Some(r.request_sha256.hex()),
            output_hash: hex::encode(env::sha256(out)),
            block_height: Some(100),
            caller_account_id: Some(r.signer.clone()),
            project_id: None,
            secrets_ref: None,
            timestamp: 1_700_000_000,
            attached_usd: None,
        }
    }

    /// The DCAP verifier's answer for a quote from `img` whose report_data is `rd`.
    fn verified(&self, img: &Measurements, rd: &[u8], status: &str) -> tee::VerificationResultIn {
        let b48 = |h: &str| <[u8; 48]>::try_from(hex::decode(h).unwrap()).unwrap();
        let mut report_data = [0u8; 64];
        report_data[..32].copy_from_slice(rd);
        let td = tee::TDReport10In {
            tee_tcb_svn: [0; 16],
            mr_seam: [0; 48],
            mr_signer_seam: [0; 48],
            seam_attributes: [0; 8],
            td_attributes: [0; 8],
            xfam: [0; 8],
            mr_td: b48(&img.mrtd),
            mr_config_id: [0; 48],
            mr_owner: [0; 48],
            mr_owner_config: [0; 48],
            rt_mr0: b48(&img.rtmr0),
            rt_mr1: b48(&img.rtmr1),
            rt_mr2: b48(&img.rtmr2),
            rt_mr3: b48(&img.rtmr3),
            report_data,
        };
        let ok = || tee::TcbStatusWithAdvisoryIn { status: tee::TcbStatusIn::UpToDate, advisory_ids: vec![] };
        tee::VerificationResultIn::Verified(tee::VerifiedReportIn {
            status: status.into(),
            advisory_ids: vec![],
            report: tee::ReportIn::TD10(td),
            ppid: vec![],
            qe_status: ok(),
            platform_status: ok(),
        })
    }

    /// Relays run `run_id`'s quote from `img`; returns whether near-antibody accepted it.
    fn t1(&mut self, run_id: u64, out: &[u8], img: &Measurements) -> bool {
        let a = self.attestation(run_id, out);
        self.call(&acc("relayer.testnet"));
        let _ = self.c.confirm_tee(run_id, a.clone(), Base64VecU8(vec![1, 2, 3]), Base64VecU8(vec![4, 5]));
        let rd = a.task_hash();
        self.callback(vec![]);
        let v = self.verified(img, &rd, "UpToDate");
        self.c.on_tee(run_id, hex::encode(&rd), Ok(v))
    }

    /// A program through creation, a T1 activation and open.
    fn open(&mut self) -> u32 {
        let id = self.create();
        self.call(&project());
        self.c.activate(id, base());
        let run = self.c.get_program(id).unwrap().activation_run.unwrap();
        let out = self.facts(run, "NoViolation", json!([]), json!({"sealing_pk": "55".repeat(32)}));
        self.answer(run, &out);
        assert!(self.t1(run, &out, &image(1)));
        self.call(&project());
        self.c.open_program(id);
        id
    }

    fn commitment(&self, program: u32, who: &AccountId, blob: &[u8]) -> String {
        let p = self.c.get_program(program).unwrap();
        commitment_for(&p.key, p.round, who.as_str(), &H32(env::sha256_array(blob))).hex()
    }

    /// A commit with its bond by `ft_transfer_call`; returns the commit id.
    fn commit(&mut self, program: u32, who: &AccountId, blob: &[u8]) -> u64 {
        let cm = self.commitment(program, who, blob);
        let round = self.c.get_program(program).unwrap().round;
        self.token_call(who);
        let msg = json!({"commit": {"program_id": program, "round": round, "commitment": cm, "terms_sha256": terms()}});
        let _ = self.c.ft_on_transfer(who.clone(), U128(BOND), msg.to_string());
        let p = self.c.get_program(program).unwrap();
        self.c.list_commits(program, Some(p.commits - 1), Some(1))[0].id
    }

    fn outlayer_call(&self) -> (Value, u128) {
        let r = get_created_receipts().into_iter().find(|r| r.receiver_id == acc("outlayer.testnet")).expect("an OutLayer call");
        match &r.actions[0] {
            MockAction::FunctionCallWeight { method_name, args, attached_deposit, .. } => {
                assert_eq!(method_name, b"request_execution");
                (serde_json::from_slice(args).unwrap(), attached_deposit.as_yoctonear())
            }
            a => panic!("unexpected action {a:?}"),
        }
    }

    fn events(&self, name: &str) -> Vec<Value> {
        get_logs()
            .iter()
            .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
            .map(|j| serde_json::from_str::<Value>(j).unwrap())
            .filter(|e| e["event"] == name)
            .map(|e| e["data"][0].clone())
            .collect()
    }
}

// ------------------------------------------------------------------------------------------ creation

#[test]
fn create_pins_the_defaults_and_charges_storage_to_the_project() {
    let mut w = W::new();
    let id = w.create();
    let p = w.c.get_program(id).unwrap();
    assert_eq!(p.key, "antibody.testnet/0");
    assert_eq!(p.phase, Phase::Setup, "a chain the light client cannot check: unconfirmed, then setup");
    assert_eq!(p.manifest_sha256, H32(env::sha256_array(manifest("antibody.manifest.v2", &project()).as_bytes())));
    assert_eq!(p.params.critical, CRITICAL);
    assert_eq!(p.params.run_deposit, RUN_DEPOSIT + RUN_STORAGE_ALLOWANCE);
    let pins = w.c.get_pins(id).unwrap();
    assert_eq!(pins.verifier.wasm_hash, wasm_hash());
    assert_eq!(pins.images, vec![image(1)]);
    let meta = w.c.get_program_meta(id).unwrap();
    assert_eq!(meta.terms_sha256, terms());
    assert_eq!(meta.slots.len(), 2);
    let credit = w.c.get_account(project()).near;
    assert!(credit < NEAR && credit > NEAR - NEAR / 10, "storage came from the project's NEAR credit: {credit}");
    assert!(w.c.audit_contract().ok);
}

#[test]
#[should_panic(expected = "not enough NEAR credit for storage")]
fn create_needs_a_storage_deposit() {
    let mut w = W::new();
    w.token_call(&project());
    let msg = json!({"create_program": {"manifest": manifest("antibody.manifest.v2", &project()), "manifest_url": "u", "project_pk": "22".repeat(32)}});
    let _ = w.c.ft_on_transfer(project(), U128(ESCROW), msg.to_string());
}

#[test]
#[should_panic(expected = "the manifest's project must create the program")]
fn only_the_manifests_project_creates_it() {
    let mut w = W::new();
    w.register(&alice());
    w.token_call(&alice());
    let msg = json!({"create_program": {"manifest": manifest("antibody.manifest.v2", &project()), "manifest_url": "u", "project_pk": "22".repeat(32)}});
    let _ = w.c.ft_on_transfer(alice(), U128(ESCROW), msg.to_string());
}

#[test]
#[should_panic(expected = "schema must be antibody.manifest.v2")]
fn a_v1_manifest_is_refused() {
    let mut w = W::new();
    w.register(&project());
    w.token_call(&project());
    let msg = json!({"create_program": {"manifest": manifest("antibody.manifest.v1", &project()), "manifest_url": "u", "project_pk": "22".repeat(32)}});
    let _ = w.c.ft_on_transfer(project(), U128(ESCROW), msg.to_string());
}

#[test]
#[should_panic(expected = "low-order point")]
fn a_low_order_project_key_is_refused() {
    let mut w = W::new();
    w.register(&project());
    w.token_call(&project());
    let msg = json!({"create_program": {"manifest": manifest("antibody.manifest.v2", &project()), "manifest_url": "u", "project_pk": "00".repeat(32)}});
    let _ = w.c.ft_on_transfer(project(), U128(ESCROW), msg.to_string());
}

#[test]
#[should_panic(expected = "only the payout token")]
fn only_the_payout_token_calls_ft_on_transfer() {
    let mut w = W::new();
    w.call(&alice());
    let _ = w.c.ft_on_transfer(alice(), U128(1), json!("deposit").to_string());
}

#[test]
#[should_panic(expected = "not enough NEAR credit for storage")]
fn a_registration_pays_for_its_own_entry() {
    let w = W::new();
    let mut c = w.c;
    set_env(&alice(), &alice(), START, 1, vec![]);
    c.storage_deposit(Some(acc("spam-1.testnet")), None);
}

#[test]
fn defaults_reach_new_programs_only() {
    let mut w = W::new();
    let first = w.create();
    w.call_with(&acc("owner.testnet"), 1);
    w.c.set_defaults(defaults(&"cd".repeat(32)));
    let second = w.create();
    assert_eq!(w.c.get_pins(first).unwrap().verifier.wasm_hash, wasm_hash());
    assert_eq!(w.c.get_pins(second).unwrap().verifier.wasm_hash, "cd".repeat(32));
}

// ------------------------------------------------------------------------------------------ runs

#[test]
fn a_run_request_names_its_run_and_is_hashed_into_it() {
    let mut w = W::new();
    let id = w.create();
    w.call(&project());
    w.c.activate(id, base());
    let run = w.c.get_program(id).unwrap().activation_run.unwrap();
    let (args, attached) = w.outlayer_call();
    assert_eq!(attached, RUN_DEPOSIT, "the pinned deposit; the storage allowance stays with near-antibody");
    assert_eq!(args["payer_account_id"], json!(project()), "OutLayer refunds the unused deposit to the requester");
    assert_eq!(args["source"]["WasmUrl"]["hash"], json!(wasm_hash()));
    assert_eq!(args["response_format"], json!("Json"));
    let input = args["input_data"].as_str().unwrap();
    assert_eq!(w.c.get_run(run).unwrap().request_sha256, H32(env::sha256_array(input.as_bytes())));
    let req: Value = serde_json::from_str(input).unwrap();
    assert_eq!(req["v"], json!(2));
    assert_eq!(req["mode"], json!("activate"));
    assert_eq!(req["program_key"], json!("antibody.testnet/0"));
    assert_eq!(req["run_id"], json!(run));
    assert_eq!(req["commit_id"], Value::Null);
    assert_eq!(req["base"], json!({"near_tx": base().location, "sha256": base().sha256}));
    let ev = &w.events("run_requested")[0];
    assert_eq!((ev["run_id"].clone(), ev["mode"].clone(), ev["payer"].clone()), (json!(run), json!("activate"), json!(project())));
    assert_eq!(w.c.get_stats()["next_run"], json!(run + 1));
    let left = w.c.get_account(project()).near;
    w.call(&project());
    assert!(left < NEAR - RUN_DEPOSIT - RUN_STORAGE_ALLOWANCE, "the requester paid the run");
}

#[test]
fn outlayer_failures_and_bad_output_never_panic() {
    let mut w = W::new();
    let id = w.create();
    w.call(&project());
    w.c.activate(id, base());
    let run = w.c.get_program(id).unwrap().activation_run.unwrap();
    // The call failed: a typed failure, and the run can be retried.
    w.callback(vec![PromiseResult::Failed]);
    w.c.on_run(run);
    assert!(matches!(w.c.get_run(run).unwrap().outcome, RunOutcome::Failed { .. }));
    assert_eq!(w.events("retry_available").len(), 1);
    // A second callback for the same run is stale: logged, not a panic.
    w.callback(vec![PromiseResult::Successful(b"{}".to_vec())]);
    w.c.on_run(run);
    // `null` (OutLayer's failure output) is a failure too.
    w.tick(601);
    w.call(&project());
    w.c.activate(id, base());
    let run2 = w.c.get_program(id).unwrap().activation_run.unwrap();
    assert_ne!(run, run2);
    w.answer(run2, b"null");
    assert!(matches!(w.c.get_run(run2).unwrap().outcome, RunOutcome::Failed { .. }));
    // Output that is not facts is the run's fault: InfraError, never a verdict.
    w.tick(601);
    w.call(&project());
    w.c.activate(id, base());
    let run3 = w.c.get_program(id).unwrap().activation_run.unwrap();
    w.answer(run3, b"{not json");
    assert_eq!(w.c.get_run(run3).unwrap().facts().unwrap().kind, Kind::InfraError);
}

// ------------------------------------------------------------------------------------------ T1

#[test]
fn activation_counts_only_with_t1() {
    let mut w = W::new();
    let id = w.create();
    w.call(&project());
    w.c.activate(id, base());
    let run = w.c.get_program(id).unwrap().activation_run.unwrap();
    let out = w.facts(run, "NoViolation", json!([]), json!({"sealing_pk": "55".repeat(32)}));
    w.answer(run, &out);
    assert_eq!(w.c.get_run(run).unwrap().tier(), Some(Tier::T0));
    assert!(w.c.get_program(id).unwrap().sealing_pk.is_none(), "T0 does not activate");
    assert!(w.t1(run, &out, &image(1)));
    let p = w.c.get_program(id).unwrap();
    assert_eq!(p.sealing_pk, Some(H32([0x55; 32])));
    assert_eq!(p.key_epoch, EPOCH);
    assert_eq!(w.c.get_run(run).unwrap().tier(), Some(Tier::T1));
}

#[test]
#[should_panic(expected = "not the program's pinned verifier")]
fn t1_binds_the_pinned_build() {
    let mut w = W::new();
    let id = w.create();
    w.call(&project());
    w.c.activate(id, base());
    let run = w.c.get_program(id).unwrap().activation_run.unwrap();
    let out = w.facts(run, "NoViolation", json!([]), json!({"sealing_pk": "55".repeat(32)}));
    w.answer(run, &out);
    let mut a = w.attestation(run, &out);
    a.wasm_hash = Some("ee".repeat(32));
    w.call(&alice());
    let _ = w.c.confirm_tee(run, a, Base64VecU8(vec![1]), Base64VecU8(vec![2]));
}

#[test]
#[should_panic(expected = "output differs")]
fn t1_binds_the_output_near_antibody_received() {
    let mut w = W::new();
    let id = w.create();
    w.call(&project());
    w.c.activate(id, base());
    let run = w.c.get_program(id).unwrap().activation_run.unwrap();
    let out = w.facts(run, "NoViolation", json!([]), json!({"sealing_pk": "55".repeat(32)}));
    w.answer(run, &out);
    let a = w.attestation(run, b"{\"another\":\"output\"}");
    w.call(&alice());
    let _ = w.c.confirm_tee(run, a, Base64VecU8(vec![1]), Base64VecU8(vec![2]));
}

#[test]
#[should_panic(expected = "a run with secrets is not a v2 run")]
fn t1_refuses_runs_with_secrets() {
    let mut w = W::new();
    let id = w.create();
    w.call(&project());
    w.c.activate(id, base());
    let run = w.c.get_program(id).unwrap().activation_run.unwrap();
    let out = w.facts(run, "NoViolation", json!([]), json!({"sealing_pk": "55".repeat(32)}));
    w.answer(run, &out);
    let mut a = w.attestation(run, &out);
    a.secrets_ref = Some("antibody-hq.testnet/antibody-v1".into());
    w.call(&alice());
    let _ = w.c.confirm_tee(run, a, Base64VecU8(vec![1]), Base64VecU8(vec![2]));
}

#[test]
fn quotes_that_do_not_bind_or_come_from_other_images_are_refused() {
    let mut w = W::new();
    let id = w.create();
    w.call(&project());
    w.c.activate(id, base());
    let run = w.c.get_program(id).unwrap().activation_run.unwrap();
    let out = w.facts(run, "NoViolation", json!([]), json!({"sealing_pk": "55".repeat(32)}));
    w.answer(run, &out);
    let a = w.attestation(run, &out);
    let rd = a.task_hash();
    // Another task's report_data.
    w.callback(vec![]);
    let v = w.verified(&image(1), &[9u8; 32], "UpToDate");
    assert!(!w.c.on_tee(run, hex::encode(&rd), Ok(v)));
    // A TCB status the program does not accept.
    w.callback(vec![]);
    let v = w.verified(&image(1), &rd, "OutOfDate");
    assert!(!w.c.on_tee(run, hex::encode(&rd), Ok(v)));
    // The DCAP verifier rejected the quote, or its call failed.
    w.callback(vec![]);
    let rej = tee::VerificationResultIn::Rejected(tee::VerifierErrorIn::DcapVerification("bad quote".into()));
    assert!(!w.c.on_tee(run, hex::encode(&rd), Ok(rej)));
    w.callback(vec![]);
    assert!(!w.c.on_tee(run, hex::encode(&rd), Err(PromiseError::Failed)));
    // An image approved after the program was created counts only after the image delay.
    w.callback(vec![]);
    assert_eq!(w.c.on_images(Ok(vec![image(1), image(20)])), 1);
    w.callback(vec![]);
    let v = w.verified(&image(20), &rd, "UpToDate");
    assert!(!w.c.on_tee(run, hex::encode(&rd), Ok(v)));
    assert_eq!(w.events("tee_rejected")[0]["reason"], json!("worker image not pinned for this program"));
    assert!(w.c.get_program(id).unwrap().sealing_pk.is_none());
    w.tick(7 * 86_400 + 1);
    w.callback(vec![]);
    let v = w.verified(&image(20), &rd, "UpToDate");
    assert!(w.c.on_tee(run, hex::encode(&rd), Ok(v)));
    assert!(w.c.get_program(id).unwrap().sealing_pk.is_some());
}

#[test]
fn a_vetoed_image_is_refused_even_when_pinned() {
    let mut w = W::new();
    let id = w.create();
    w.call(&project());
    w.c.activate(id, base());
    let run = w.c.get_program(id).unwrap().activation_run.unwrap();
    let out = w.facts(run, "NoViolation", json!([]), json!({"sealing_pk": "55".repeat(32)}));
    w.answer(run, &out);
    w.call_with(&acc("owner.testnet"), 1);
    w.c.veto_image(image(1));
    assert!(!w.t1(run, &out, &image(1)));
}

// ------------------------------------------------------------------------------------------ hunters

#[test]
fn proof_to_payment_to_withdrawal() {
    let mut w = W::new();
    let id = w.open();
    w.register(&alice());
    let blob = b"sealed proof of concept".to_vec();
    let cid = w.commit(id, &alice(), &blob);
    assert_eq!(w.c.get_commit(cid).unwrap().state, CommitState::Committed);
    w.call(&alice());
    w.c.reveal(cid, BlobIn::Inline(hex::encode(&blob)));
    let (args, _) = w.outlayer_call();
    let req: Value = serde_json::from_str(args["input_data"].as_str().unwrap()).unwrap();
    assert_eq!(req["mode"], json!("verify"));
    assert_eq!(req["hunter"], json!(alice()));
    assert_eq!(req["blob"], json!({"inline": hex::encode(&blob)}));
    assert_eq!(args["payer_account_id"], json!(alice()));
    let run = w.c.get_commit(cid).unwrap().run.unwrap();
    let out = w.facts(run, "Violation", json!([{"slot": 0, "tag": "66".repeat(32)}]), json!({"impact_usd_micro": "0"}));
    w.answer(run, &out);
    // T0: final only after the challenge window.
    w.call(&acc("keeper.testnet"));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| w.c.allocate(id, None))).is_err());
    w.tick(1_801);
    w.call(&acc("keeper.testnet"));
    w.c.allocate(id, None);
    assert!(matches!(w.c.get_commit(cid).unwrap().state, CommitState::Holder { .. }));
    w.call(&acc("keeper.testnet"));
    w.c.pay(cid);
    assert_eq!(w.c.get_account(alice()).tokens, CRITICAL + BOND, "payout plus the returned bond");
    let a = w.c.audit(id).unwrap();
    assert!(a.ok, "{a:?}");
    // Credit, then withdraw.
    w.call_with(&alice(), 1);
    w.c.withdraw(U128(CRITICAL));
    let r = get_created_receipts().into_iter().find(|r| r.receiver_id == token()).expect("an ft_transfer");
    match &r.actions[0] {
        MockAction::FunctionCallWeight { method_name, args, attached_deposit, .. } => {
            assert_eq!(method_name, b"ft_transfer");
            assert_eq!(attached_deposit.as_yoctonear(), 1);
            let a: Value = serde_json::from_slice(args).unwrap();
            assert_eq!(a["receiver_id"], json!(alice()));
            assert_eq!(a["amount"], json!(CRITICAL.to_string()));
        }
        a => panic!("unexpected action {a:?}"),
    }
    assert_eq!(w.c.get_account(alice()).tokens, BOND);
    // A failed transfer is credited back.
    w.callback(vec![]);
    w.c.on_withdraw(alice().to_string(), U128(CRITICAL), Err(PromiseError::Failed));
    assert_eq!(w.c.get_account(alice()).tokens, CRITICAL + BOND);
    assert!(w.c.audit_contract().ok);
}

#[test]
#[should_panic(expected = "does not match the commitment")]
fn a_reveal_must_match_its_commitment() {
    let mut w = W::new();
    let id = w.open();
    w.register(&alice());
    let cid = w.commit(id, &alice(), b"the committed blob");
    w.call(&alice());
    w.c.reveal(cid, BlobIn::Inline(hex::encode(b"another blob")));
}

#[test]
#[should_panic(expected = "inline blobs are at most 15 KB")]
fn large_blobs_go_through_fastfs() {
    let mut w = W::new();
    let id = w.open();
    w.register(&alice());
    let blob = vec![7u8; MAX_INLINE_BLOB + 1];
    let cid = w.commit(id, &alice(), &blob);
    w.call(&alice());
    w.c.reveal(cid, BlobIn::Inline(hex::encode(&blob)));
}

#[test]
fn a_fastfs_blob_is_recorded_and_reused_by_later_runs() {
    let mut w = W::new();
    let id = w.open();
    w.register(&alice());
    let blob = vec![7u8; 40 * 1024];
    let cid = w.commit(id, &alice(), &blob);
    let loc = Loc { location: "near-tx:4Zk9blob:alice.testnet".into(), sha256: hex::encode(env::sha256(&blob)) };
    w.call(&alice());
    w.c.reveal(cid, BlobIn::NearTx(loc.clone()));
    let run = w.c.get_commit(cid).unwrap().run.unwrap();
    // OutLayer failed; the retry finds the blob without the hunter passing it again.
    w.callback(vec![PromiseResult::Failed]);
    w.c.on_run(run);
    w.call(&alice());
    w.c.retry(cid, None);
    let (args, _) = w.outlayer_call();
    let req: Value = serde_json::from_str(args["input_data"].as_str().unwrap()).unwrap();
    assert_eq!(req["blob"], json!({"near_tx": loc.location, "sha256": loc.sha256}));
}

#[test]
#[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
fn withdrawals_need_a_full_access_key() {
    let mut w = W::new();
    w.register(&alice());
    w.call(&alice());
    w.c.withdraw(U128(1));
}

#[test]
fn an_agent_over_its_cap_is_contained() {
    let mut w = W::new();
    let id = w.open();
    w.register(&alice());
    // Alice funds her credit and lets her agent bond at most half a bond per commit.
    w.token_call(&alice());
    let _ = w.c.ft_on_transfer(alice(), U128(100 * ONE), json!("deposit").to_string());
    let bot = acc("bot.alice.testnet");
    w.call_with(&alice(), 1);
    w.c.register_agent(bot.clone(), U128(100 * ONE), U128(BOND / 2));
    let cm = w.commitment(id, &bot, b"agent blob");
    w.call(&bot);
    assert_eq!(w.c.commit(id, 0, cm.clone(), terms()), None);
    let s = w.c.agent_statement(bot.clone());
    assert_eq!(s["containment_events"], json!(1));
    assert_eq!(w.events("containment").len(), 1);
    assert_eq!(w.c.get_account(alice()).tokens, 100 * ONE, "nothing was bonded");
    // Within its cap, the bond comes from Alice's credit and payouts go to her.
    w.call_with(&alice(), 1);
    w.c.register_agent(bot.clone(), U128(100 * ONE), U128(BOND));
    w.call(&bot);
    let cid = w.c.commit(id, 0, cm, terms()).unwrap();
    let c = w.c.get_commit(cid).unwrap();
    assert_eq!((c.hunter.as_str(), c.beneficiary.as_str()), (bot.as_str(), "alice.testnet"));
    assert_eq!(w.c.get_account(alice()).tokens, 100 * ONE - BOND);
}

#[test]
#[should_panic(expected = "an agent cannot withdraw")]
fn an_agent_cannot_withdraw() {
    let mut w = W::new();
    w.register(&alice());
    let bot = acc("bot.alice.testnet");
    w.call_with(&alice(), 1);
    w.c.register_agent(bot.clone(), U128(100 * ONE), U128(BOND));
    w.call_with(&bot, 1);
    w.c.withdraw(U128(1));
}

#[test]
fn the_commitment_binds_the_hunter() {
    // The same blob committed by another account gives another commitment.
    let w = W::new();
    let h = H32([1; 32]);
    assert_ne!(commitment_for("antibody.testnet/0", 0, "alice.testnet", &h), commitment_for("antibody.testnet/0", 0, "bob.testnet", &h));
    assert_ne!(commitment_for("antibody.testnet/0", 0, "alice.testnet", &h), commitment_for("antibody.testnet/0", 1, "alice.testnet", &h));
    // The CLI pins the same vector (crates/cli/src/main.rs).
    assert_eq!(commitment_for("bounty.testnet/0", 0, "agent-a.testnet", &H32([0xab; 32])).hex(), "e0b01b170a5e2d10a922e1e6e722dddf3dba94f8b02c06eb5051543dbfff3141");
    drop(w);
}

// near-sdk 5.9's mocked blockchain does not implement yield/resume, yet `env` references the host
// functions. ELF links tolerate the unused references; the MSVC linker does not, so give the
// Windows unit-test binary inert definitions (never called by these tests).
#[cfg(windows)]
#[no_mangle]
pub extern "C" fn promise_yield_create(_: u64, _: u64, _: u64, _: u64, _: u64, _: u64, _: u64) -> u64 {
    unreachable!("yield/resume is not used by near-antibody")
}
/// golden/vectors.json (written by crates/cli/tests/golden.rs): the CLI's commitments are this contract's.
#[test]
fn golden_commitments_match_the_cli() {
    let g: near_sdk::serde_json::Value = near_sdk::serde_json::from_str(include_str!("../../../golden/vectors.json")).unwrap();
    for v in g["commitment"].as_array().unwrap() {
        let mut blob = [0u8; 32];
        blob.copy_from_slice(&hex::decode(v["blob_sha256"].as_str().unwrap()).unwrap());
        let c = commitment_for(v["program_key"].as_str().unwrap(), v["round"].as_u64().unwrap() as u32, v["hunter"].as_str().unwrap(), &H32(blob));
        assert_eq!(c.hex(), v["commitment"].as_str().unwrap());
    }
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn promise_yield_resume(_: u64, _: u64, _: u64, _: u64) -> u32 {
    unreachable!("yield/resume is not used by near-antibody")
}

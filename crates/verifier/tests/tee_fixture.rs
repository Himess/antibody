//! TEE mode v2 end to end on the recorded fixture (docs/spec.md §5): the program is activated from its
//! base witness, each submission is sealed to the program's own key exactly as a hunter's CLI does, and
//! `tee::handle` runs it the way OutLayer runs the verifier. Every output is parsed with the settlement
//! rules' own facts parser (`antibody-rules`), so the verifier and the contract agree on the format.

use std::path::PathBuf;

use antibody_rules::facts::{self as rf, Expect};
use antibody_verifier::{
    facts::{sha256, Kind},
    keys::{ProgramKeys, TestMac},
    sealed::{self, SealedPayload},
    tee::{handle, BlobRef, Env},
    BaseWitness, Envelope, Payload,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const CONTRACT: &str = "antibody.testnet";
const PROJECT_SK: [u8; 32] = [77u8; 32];

fn mac() -> TestMac {
    TestMac(sha256(b"antibody/TEST-ONLY-root/never-use-in-production"))
}

fn expected() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("fixtures/acme-local/expected.json")).unwrap()).unwrap()
}

/// A base witness: a fixture witness without its PoC (the project's pinned state at the anchor).
fn base_from(agent: &str) -> Vec<u8> {
    let bytes = std::fs::read(root().join(format!("fixtures/acme-local/submissions/{agent}.witness.bin"))).unwrap();
    let mut env = Envelope::decode(&bytes).unwrap();
    let Payload::Program(w) = env.payload.clone() else { panic!("program witness") };
    env.payload = Payload::Base(BaseWitness {
        manifest_json: w.manifest_json,
        invariants_code: w.invariants_code,
        header_rlp: w.header_rlp,
        ancestor_headers_rlp: w.ancestor_headers_rlp,
        state: w.state,
        bytecodes: w.bytecodes,
    });
    env.encode()
}

fn seal(program_key: &str, hunter: &str, agent: &str) -> Vec<u8> {
    let envelope = std::fs::read(root().join(format!("fixtures/acme-local/submissions/{agent}.witness.bin"))).unwrap();
    let m = mac();
    let pk = ProgramKeys::new(&m, program_key).seal_public().unwrap();
    let k = sha256(format!("content-key/{agent}").as_bytes());
    let wrap = sealed::wrap_v(&k, &pk, program_key, sha256(format!("eph/{agent}").as_bytes()), [1u8; 12]);
    let payload = SealedPayload { program_key: program_key.into(), hunter: hunter.into(), envelope, report: b"ignored".to_vec() };
    sealed::seal(&k, &payload, &wrap, [2u8; 12])
}

struct World {
    program_key: String,
    base: Vec<u8>,
    manifest: String,
    epoch: String,
    /// Sealed blobs served as FastFS uploads (a full fixture witness is above the 32 KB inline cap).
    blobs: std::cell::RefCell<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl World {
    fn new() -> World {
        let e = expected();
        // The fixtures' program key, "<near-antibody account>/<id>" (scripts/demo/v2keys.py).
        let program_key = e["program_id"].as_str().unwrap().to_string();
        assert!(program_key.starts_with(&format!("{CONTRACT}/")));
        let m = mac();
        let epoch = ProgramKeys::new(&m, &program_key).key_epoch().unwrap();
        World {
            program_key,
            base: base_from("KNOWN"),
            manifest: e["manifest_sha256"].as_str().unwrap().trim_start_matches("0x").into(),
            epoch,
            blobs: Default::default(),
        }
    }

    fn fetch(&self) -> impl Fn(&str) -> Result<Vec<u8>, String> + '_ {
        move |loc: &str| {
            if loc == "near-tx:base:project.testnet" {
                return Ok(self.base.clone());
            }
            self.blobs.borrow().get(loc).cloned().ok_or_else(|| format!("unknown {loc}"))
        }
    }

    /// Uploads a blob (FastFS) and returns its reference for a request.
    fn upload(&self, blob: &[u8]) -> serde_json::Value {
        let loc = format!("near-tx:{}:agent.testnet", hex::encode(&sha256(blob)[..8]));
        self.blobs.borrow_mut().insert(loc.clone(), blob.to_vec());
        serde_json::json!({"near_tx": loc, "sha256": hex::encode(sha256(blob))})
    }

    fn request(&self, mode: &str, commit: Option<u64>, extra: serde_json::Value) -> Vec<u8> {
        let mut r = serde_json::json!({
            "v": 2, "mode": mode, "program_key": self.program_key, "manifest_sha256": self.manifest, "round": 0,
            "key_epoch": if mode == "activate" { String::new() } else { self.epoch.clone() },
            "commit_id": commit, "run_id": 41,
            "base": {"near_tx": "near-tx:base:project.testnet", "sha256": hex::encode(sha256(&self.base))},
        });
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        serde_json::to_vec(&r).unwrap()
    }

    fn run(&self, req: &[u8], caller: &str) -> (antibody_verifier::tee::Output, Vec<u8>) {
        let m = mac();
        let fetch = self.fetch();
        let env = Env { mac: &m, fetch: &fetch, predecessor: Some(caller.into()) };
        let out = handle(req, &env);
        let bytes = serde_json::to_vec(&out.facts).unwrap();
        (out, bytes)
    }

    /// The contract's own parser accepts the facts for this request.
    fn parse(&self, req: &[u8], facts: &[u8], mode: antibody_rules::Mode, commit: Option<u64>) -> rf::FactsSummary {
        let slots = vec![
            antibody_rules::SlotRule { slot: 0, severity: antibody_rules::Severity::Critical, budget: 1 },
            antibody_rules::SlotRule { slot: 1, severity: antibody_rules::Severity::High, budget: 1 },
            antibody_rules::SlotRule { slot: 2, severity: antibody_rules::Severity::Medium, budget: 1 },
        ];
        let ex = Expect {
            mode,
            request_sha256: antibody_rules::H32(sha256(req)),
            manifest_sha256: antibody_rules::H32::from_hex(&self.manifest).unwrap(),
            program_key: &self.program_key,
            round: 0,
            key_epoch: &self.epoch,
            commit_id: commit,
            run_id: 41,
            slots: &slots,
        };
        rf::parse(facts, &ex).unwrap_or_else(|e| panic!("the contract rejects the facts: {e}: {}", String::from_utf8_lossy(facts)))
    }
}

#[test]
fn activation_outputs_the_program_key() {
    let w = World::new();
    let req = w.request("activate", None, serde_json::json!({}));
    let (out, bytes) = w.run(&req, CONTRACT);
    assert_eq!(out.facts.kind, Kind::NoViolation, "{:?}", out.facts.reason);
    let s = w.parse(&req, &bytes, antibody_rules::Mode::Activate, None);
    let m = mac();
    let want = ProgramKeys::new(&m, &w.program_key).seal_public().unwrap();
    assert!(matches!(s.extra, rf::Extra::SealingKey { pk, .. } if pk.0 == want));
    assert!(bytes.len() <= rf::MAX_FACTS_BYTES);
}

#[test]
fn verify_gives_the_recorded_verdicts_in_v2_facts() {
    let w = World::new();
    let e = expected();
    let hunter = "agent.testnet";
    for (agent, x) in e["submissions"].as_object().unwrap() {
        let commit = x["commit_id"].as_u64().unwrap();
        let blob = seal(&w.program_key, hunter, agent);
        let req = w.request("verify", Some(commit), serde_json::json!({"hunter": hunter, "blob": w.upload(&blob)}));
        let (out, bytes) = w.run(&req, CONTRACT);
        assert_eq!(format!("{:?}", out.facts.kind), x["kind"].as_str().unwrap(), "{agent}: {:?}", out.facts.reason);
        let s = w.parse(&req, &bytes, antibody_rules::Mode::Verify, Some(commit));
        let slots: Vec<u8> = s.keys.iter().map(|k| k.slot).collect();
        let want: Vec<u8> = x["broken"].as_array().unwrap().iter().map(|b| b["slot"].as_u64().unwrap() as u8).collect();
        assert_eq!(slots, want, "{agent}: broken slots");
        assert!(bytes.len() <= rf::MAX_FACTS_BYTES, "{agent}: facts are {} bytes", bytes.len());
        if out.facts.kind == Kind::Violation {
            assert!(out.facts.wrap_d.is_some(), "{agent}: a violation carries its disclosure wrap");
        }
        // A second run of the same request gives the same receipt: agreement is by content.
        let (out2, _) = w.run(&req, CONTRACT);
        assert_eq!(out.facts.receipt_sha256, out2.facts.receipt_sha256);
    }
}

#[test]
fn a_known_issue_is_verified_without_a_commit() {
    // submit_known: the project's sealed PoC, no commit. Same verdict and keys; no disclosure wrap.
    let w = World::new();
    let e = expected();
    let (agent, x) = e["submissions"].as_object().unwrap().iter().find(|(_, x)| x["kind"] == "Violation").unwrap();
    let project = "acme.testnet";
    let blob = seal(&w.program_key, project, agent);
    let req = w.request("verify", None, serde_json::json!({"hunter": project, "blob": w.upload(&blob)}));
    let (out, bytes) = w.run(&req, CONTRACT);
    assert_eq!(format!("{:?}", out.facts.kind), x["kind"].as_str().unwrap(), "{:?}", out.facts.reason);
    assert!(out.facts.wrap_d.is_none(), "no commit, no disclosure wrap");
    let s = w.parse(&req, &bytes, antibody_rules::Mode::Verify, None);
    assert!(!s.keys.is_empty(), "the rule core accepts the facts and their keys");
    // A second run without a commit is refused as an infrastructure error, never a verdict.
    let req2 = w.request("second", None, serde_json::json!({"hunter": project, "blob": w.upload(&blob)}));
    let (out2, _) = w.run(&req2, CONTRACT);
    assert_eq!(out2.facts.kind, Kind::InfraError);
}

#[test]
fn equal_root_causes_give_equal_tags() {
    let w = World::new();
    let e = expected();
    let mut tags: std::collections::BTreeMap<(u64, String, String), std::collections::BTreeSet<String>> = Default::default();
    for (agent, x) in e["submissions"].as_object().unwrap() {
        let commit = x["commit_id"].as_u64().unwrap();
        let blob = seal(&w.program_key, "agent.testnet", agent);
        let req = w.request("verify", Some(commit), serde_json::json!({"hunter": "agent.testnet", "blob": w.upload(&blob)}));
        let (out, _) = w.run(&req, CONTRACT);
        for (k, b) in out.facts.keys.iter().zip(x["broken"].as_array().unwrap()) {
            let key = (b["slot"].as_u64().unwrap(), b["target"].as_str().unwrap().to_string(), b["selector"].as_str().unwrap().to_string());
            tags.entry(key).or_default().insert(k.tag.clone());
        }
    }
    for (key, t) in tags {
        assert_eq!(t.len(), 1, "{key:?} produced different tags");
    }
}

#[test]
fn failures_are_typed_and_never_forfeit_for_infrastructure() {
    let w = World::new();
    let blob = seal(&w.program_key, "agent.testnet", "A");
    // Another hunter's account: the blob is bound to its sealer (InvalidPoC, the replayer's bond).
    let req = w.request("verify", Some(1), serde_json::json!({"hunter": "mallory.testnet", "blob": w.upload(&blob)}));
    assert_eq!(w.run(&req, CONTRACT).0.facts.kind, Kind::InvalidPoC);
    // A blob no RPC served: InfraError, never a forfeit.
    let req = w.request("verify", Some(1), serde_json::json!({"hunter": "agent.testnet",
        "blob": {"near_tx": "near-tx:missing:agent.testnet", "sha256": hex::encode(sha256(&blob))}}));
    assert_eq!(w.run(&req, CONTRACT).0.facts.kind, Kind::InfraError);
    // Another caller than the program's contract: refused, and it would not hold the key anyway.
    let req = w.request("verify", Some(1), serde_json::json!({"hunter": "agent.testnet", "blob": w.upload(&blob)}));
    assert_eq!(w.run(&req, "someone.testnet").0.facts.kind, Kind::InfraError);
    // Garbage in: typed facts out.
    assert_eq!(w.run(b"{not json", CONTRACT).0.facts.kind, Kind::InfraError);
}

#[test]
fn release_wrap_opens_with_the_project_key_only() {
    let w = World::new();
    let blob = seal(&w.program_key, "agent.testnet", "A");
    let pk = sealed::x25519_public(PROJECT_SK);
    let req = w.request("release_wrap", Some(1), serde_json::json!({"hunter": "agent.testnet", "blob": w.upload(&blob), "project_pk": hex::encode(pk)}));
    let (out, bytes) = w.run(&req, CONTRACT);
    w.parse(&req, &bytes, antibody_rules::Mode::ReleaseWrap, Some(1));
    let wrap = hex::decode(out.facts.wrap.unwrap()).unwrap();
    let k = sealed::project_unwrap(&wrap, &x25519_dalek::StaticSecret::from(PROJECT_SK), &w.program_key).unwrap();
    assert_eq!(k, sha256(b"content-key/A"));
    assert!(sealed::project_unwrap(&wrap, &x25519_dalek::StaticSecret::from([5u8; 32]), &w.program_key).is_err());
    // A low-order project key is refused, never a panic.
    let req = w.request("release_wrap", Some(1), serde_json::json!({"hunter": "agent.testnet", "blob": w.upload(&blob), "project_pk": hex::encode([0u8; 32])}));
    assert_eq!(w.run(&req, CONTRACT).0.facts.kind, Kind::InfraError);
}

#[test]
fn disclosure_keys_open_the_disclosure_wraps() {
    let w = World::new();
    let blob = seal(&w.program_key, "agent.testnet", "A");
    let req = w.request("verify", Some(1), serde_json::json!({"hunter": "agent.testnet", "blob": w.upload(&blob)}));
    let (out, _) = w.run(&req, CONTRACT);
    let tag = out.facts.keys[0].tag.clone();
    let req = w.request("disclose", None, serde_json::json!({"tags": [{"round": 0, "tag": tag}]}));
    let (dout, bytes) = w.run(&req, CONTRACT);
    w.parse(&req, &bytes, antibody_rules::Mode::Disclose, None);
    let dk: [u8; 32] = hex::decode(&dout.facts.disclosed[0].key).unwrap().try_into().unwrap();
    let wd = hex::decode(out.facts.wrap_d.unwrap()).unwrap();
    let nonce: [u8; 12] = wd[..12].try_into().unwrap();
    let k = sealed::aead_open_pub(&dk, &nonce, &wd[12..], w.program_key.as_bytes()).unwrap();
    assert_eq!(k, sha256(b"content-key/A").to_vec(), "the disclosure key opens the content key");
}

#[test]
fn base_and_submission_merge_into_the_same_verdict() {
    // A submission carrying only the PoC and the full state still merges with the base (idempotent).
    let w = World::new();
    let bytes = std::fs::read(root().join("fixtures/acme-local/submissions/A.witness.bin")).unwrap();
    let mut env = Envelope::decode(&bytes).unwrap();
    let Payload::Program(p) = env.payload.clone() else { panic!() };
    env.payload = Payload::Submission(antibody_verifier::SubmissionWitness { bundle: p.bundle, state: p.state, bytecodes: vec![] });
    let m = mac();
    let pk = ProgramKeys::new(&m, &w.program_key).seal_public().unwrap();
    let k = [9u8; 32];
    let wrap = sealed::wrap_v(&k, &pk, &w.program_key, [3u8; 32], [1u8; 12]);
    let payload = SealedPayload { program_key: w.program_key.clone(), hunter: "agent.testnet".into(), envelope: env.encode(), report: vec![] };
    let blob = sealed::seal(&k, &payload, &wrap, [2u8; 12]);
    let req = w.request("verify", Some(1), serde_json::json!({"hunter": "agent.testnet", "blob": w.upload(&blob)}));
    let (out, _) = w.run(&req, CONTRACT);
    assert_eq!(out.facts.kind, Kind::Violation, "{:?}", out.facts.reason);
    let full = seal(&w.program_key, "agent.testnet", "A");
    let req2 = w.request("verify", Some(1), serde_json::json!({"hunter": "agent.testnet", "blob": w.upload(&full)}));
    let (out2, _) = w.run(&req2, CONTRACT);
    assert_eq!(out.facts.keys, out2.facts.keys, "same keys from the merged witness");
    let _ = BlobRef::Inline { inline: String::new() };
}

/// A round check binds the round's releases (spec §9): each must name a target that is an ERC-1967
/// proxy at the anchor and the code hash of the override that replaces its implementation.
#[test]
fn a_round_check_binds_its_releases() {
    let w = World::new();
    let round_bytes = std::fs::read(root().join("fixtures/acme-local/round1/manifest.json")).unwrap();
    let m: serde_json::Value = serde_json::from_slice(&round_bytes).unwrap();
    let target = |name: &str| m["targets"].as_array().unwrap().iter().find(|t| t["name"] == name).unwrap()["address"].as_str().unwrap().to_string();
    let (lend, vault) = (target("AcmeLend"), target("AcmeVault"));
    let o = &m["overrides"][0];
    assert_eq!(o["address"].as_str().unwrap(), target("AcmeLendImpl"), "round 1 overrides AcmeLend's implementation");
    let fixed = o["code_hash"].as_str().unwrap().to_string();
    let check = |releases: serde_json::Value| {
        let req = w.request(
            "check_round",
            None,
            serde_json::json!({"round": 1, "manifest_sha256": hex::encode(sha256(&round_bytes)), "round_manifest": w.upload(&round_bytes),
                               "releases": releases}),
        );
        let facts = serde_json::to_value(w.run(&req, CONTRACT).0.facts).unwrap();
        (facts["round_check"]["ok"].as_bool().unwrap(), facts["reason"].clone())
    };
    assert_eq!(check(serde_json::json!([])).0, true, "a round without releases");
    assert_eq!(check(serde_json::json!([{"proxy": lend, "code_hash": fixed}])).0, true, "the proxy, to the code the re-runs ran");
    let (ok, reason) = check(serde_json::json!([{"proxy": lend, "code_hash": format!("0x{}", "11".repeat(32))}]));
    assert!(!ok, "other code than the override's");
    assert!(reason.as_str().unwrap().contains("release"));
    assert!(!check(serde_json::json!([{"proxy": vault, "code_hash": fixed}])).0, "a target that is not a proxy of the override");
    assert!(!check(serde_json::json!([{"proxy": "0x0000000000000000000000000000000000000001", "code_hash": fixed}])).0, "not a target");
    assert!(!check(serde_json::json!([{"proxy": "nope", "code_hash": fixed}])).0, "malformed");
}

/// Immune memory in the TEE (spec §8.2): the same sealed PoC re-run under a round's manifest. Round 1
/// (bug #1 fixed) stops A but not B, whose keys stay exactly as in round 0; round 2 (the final fix)
/// stops B and F too.
#[test]
fn a_rerun_judges_the_same_poc_at_the_fix() {
    let w = World::new();
    let e = expected();
    let rerun = |agent: &str, round: u32| {
        let round_bytes = std::fs::read(root().join(format!("fixtures/acme-local/round{round}/manifest.json"))).unwrap();
        let commit = e["submissions"][agent]["commit_id"].as_u64().unwrap();
        let blob = seal(&w.program_key, "agent.testnet", agent);
        let req = w.request(
            "rerun",
            Some(commit),
            serde_json::json!({"hunter": "agent.testnet", "blob": w.upload(&blob), "round": round,
                               "manifest_sha256": hex::encode(sha256(&round_bytes)), "round_manifest": w.upload(&round_bytes)}),
        );
        let out = w.run(&req, CONTRACT).0;
        (out.facts.kind, serde_json::to_value(&out.facts.keys).unwrap())
    };
    assert_eq!(rerun("A", 1).0, Kind::NoViolation, "A is fixed in round 1");
    let (kind, keys) = rerun("B", 1);
    assert_eq!(kind, Kind::Violation, "B still breaks in round 1");
    assert_eq!(keys, e["submissions"]["B"]["keys"], "with its round-0 keys");
    assert_eq!(rerun("B", 2).0, Kind::NoViolation, "B is fixed in round 2");
    assert_eq!(rerun("F", 2).0, Kind::NoViolation, "F is fixed in round 2");
}

/// A round that adds an invariant (spec §8.1): the slots the proposal will price must be exactly the
/// ones its manifest adds, at their severities.
#[test]
fn a_round_check_binds_its_new_slots() {
    use antibody_verifier::manifest::{Manifest, Severity, Slot};
    let w = World::new();
    let base = Manifest::parse(&std::fs::read(root().join("fixtures/acme-local/manifest.json")).unwrap()).unwrap();
    let r1 = Manifest::parse(&std::fs::read(root().join("fixtures/acme-local/round1/manifest.json")).unwrap()).unwrap();
    let fix: Vec<_> = r1.overrides.iter().map(|o| (o.address, o.runtime_code.clone())).collect();
    let slot = Slot { bit: 3, name: "protected_balances".into(), severity: Severity::Medium };
    let round = base.with_round(1, &fix, Some((base.invariants.runtime_code.clone(), vec![slot]))).unwrap().canonical();
    let check = |new_slots: serde_json::Value| {
        let req = w.request(
            "check_round",
            None,
            serde_json::json!({"round": 1, "manifest_sha256": hex::encode(sha256(&round)), "round_manifest": w.upload(&round),
                               "new_slots": new_slots}),
        );
        serde_json::to_value(w.run(&req, CONTRACT).0.facts).unwrap()["round_check"]["ok"].as_bool().unwrap()
    };
    assert!(check(serde_json::json!([{"slot": 3, "severity": "medium"}])), "the slot the manifest adds");
    assert!(!check(serde_json::json!([])), "a new slot the proposal does not price");
    assert!(!check(serde_json::json!([{"slot": 3, "severity": "low"}])), "at another severity");
    assert!(!check(serde_json::json!([{"slot": 3, "severity": "medium"}, {"slot": 4, "severity": "medium"}])), "a slot the manifest does not add");
}

/// A round keeps every invariant of the round it follows (spec §8.1): after round 1 adds slot 3, a
/// round 2 that drops it, or changes the invariants code without adding a slot, is refused.
#[test]
fn a_round_keeps_the_invariants_of_the_round_before() {
    use antibody_verifier::manifest::{Manifest, Severity, Slot};
    let w = World::new();
    let base = Manifest::parse(&std::fs::read(root().join("fixtures/acme-local/manifest.json")).unwrap()).unwrap();
    let fix = |n: u32| -> Vec<_> {
        let m = Manifest::parse(&std::fs::read(root().join(format!("fixtures/acme-local/round{n}/manifest.json"))).unwrap()).unwrap();
        m.overrides.iter().map(|o| (o.address, o.runtime_code.clone())).collect()
    };
    let code = base.invariants.runtime_code.clone();
    let slot3 = || vec![Slot { bit: 3, name: "protected_balances".into(), severity: Severity::Medium }];
    let r1 = base.with_round(1, &fix(1), Some((code.clone(), slot3()))).unwrap().canonical();
    let check = |round: &[u8]| {
        let req = w.request(
            "check_round",
            None,
            serde_json::json!({"round": 2, "manifest_sha256": hex::encode(sha256(round)), "round_manifest": w.upload(round),
                               "new_slots": [], "prev_round_manifest": w.upload(&r1)}),
        );
        serde_json::to_value(w.run(&req, CONTRACT).0.facts).unwrap()["round_check"]["ok"].as_bool().unwrap()
    };
    let keep = base.with_round(2, &fix(2), Some((code.clone(), slot3()))).unwrap().canonical();
    assert!(check(&keep), "round 2 keeps slot 3");
    let drop = base.with_round(2, &fix(2), None).unwrap().canonical();
    assert!(!check(&drop), "round 2 drops slot 3");
    let mut other = code.to_vec();
    other.push(0x00);
    let changed = base.with_round(2, &fix(2), Some((other.into(), slot3()))).unwrap().canonical();
    assert!(!check(&changed), "the invariants code changes with no new slot");
}

/// A base witness must carry the program's own round-0 manifest: a round's base comes from the project.
#[test]
fn a_base_witness_of_another_manifest_is_refused() {
    let w = World::new();
    let blob = seal(&w.program_key, "agent.testnet", "A");
    let commit = expected()["submissions"]["A"]["commit_id"].as_u64().unwrap();
    let verify = |program_manifest: &str| {
        let req = w.request("verify", Some(commit), serde_json::json!({"hunter": "agent.testnet", "blob": w.upload(&blob),
                                                                         "program_manifest_sha256": program_manifest}));
        w.run(&req, CONTRACT).0.facts.kind
    };
    assert_eq!(verify(&w.manifest), Kind::Violation, "the program's own manifest");
    assert_eq!(verify(&"11".repeat(32)), Kind::InfraError, "a base built for another manifest");
}

/// A run is judged by the manifest's invariants: a round that brings new invariants code re-runs a
/// round-0 witness under it (the witness's own copy is the old code).
#[test]
fn a_rerun_uses_the_rounds_invariants_code() {
    use antibody_verifier::manifest::{Manifest, Severity, Slot};
    let w = World::new();
    let base = Manifest::parse(&std::fs::read(root().join("fixtures/acme-local/manifest.json")).unwrap()).unwrap();
    let r1 = Manifest::parse(&std::fs::read(root().join("fixtures/acme-local/round1/manifest.json")).unwrap()).unwrap();
    let fix: Vec<_> = r1.overrides.iter().map(|o| (o.address, o.runtime_code.clone())).collect();
    let mut code = base.invariants.runtime_code.to_vec();
    code.push(0x00); // other bytes, the same behaviour
    let slot = Slot { bit: 3, name: "protected_balances".into(), severity: Severity::Medium };
    let round = base.with_round(1, &fix, Some((code.into(), vec![slot]))).unwrap().canonical();
    let e = expected();
    let blob = seal(&w.program_key, "agent.testnet", "A");
    let req = w.request(
        "rerun",
        Some(e["submissions"]["A"]["commit_id"].as_u64().unwrap()),
        serde_json::json!({"hunter": "agent.testnet", "blob": w.upload(&blob), "round": 1,
                           "manifest_sha256": hex::encode(sha256(&round)), "round_manifest": w.upload(&round)}),
    );
    assert_eq!(w.run(&req, CONTRACT).0.facts.kind, Kind::NoViolation, "A is fixed, judged under the round's invariants");
}

//! What a verified proof says about a run, against near-antibody's own records (its views, read by
//! cross-contract calls). Two kinds of difference:
//!
//! - **The input** (the request's hash, the run's place, the commit's blob, the base, the manifest, the
//!   anchor, the verifier): the proof is about something other than this run as near-antibody recorded
//!   it. The submission is rejected and nothing is recorded.
//! - **The output** (the receipt's hash, the verdict's kind, the keys' slots): the same input, re-run by
//!   the verifier in SP1, gave another verdict than the one near-antibody recorded from the TEE. That is a
//!   contradiction, and it is recorded.

use near_sdk::serde_json::Value;

use crate::{
    groth16::hex,
    pv::{kind_name, severity_name, PublicValues, KNOWN_ISSUE_COMMIT},
};

/// What the contract pins besides the keys.
pub struct Pinned<'a> {
    /// near-antibody's account: a program's key is `<account>/<id>`.
    pub antibody: &'a str,
    /// The receipt's verifier string the guest runs.
    pub verifier: &'a str,
    /// The sha256 of the TEE wasm whose verifier core the guest re-runs (a program's `wasm_hash` pin).
    pub verifier_wasm: &'a str,
}

/// The chain's records of one run (near-antibody's views).
pub struct Chain<'a> {
    pub run: &'a Value,
    /// `None` for a known issue's run, which names no commit.
    pub commit: Option<&'a Value>,
    pub program: &'a Value,
    pub meta: &'a Value,
    pub pins: &'a Value,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Every compared field is equal.
    Confirmed,
    /// The same input; the listed outputs differ.
    Contradicts(Vec<&'static str>),
}

/// A program's id from its key `<antibody>/<id>`.
pub fn program_id(antibody: &str, program_key: &str) -> Result<u32, &'static str> {
    let id = program_key
        .strip_prefix(antibody)
        .and_then(|r| r.strip_prefix('/'))
        .ok_or("the proof is about another near-antibody's program")?;
    if id.is_empty() || id.len() > 10 || !id.bytes().all(|b| b.is_ascii_digit()) || (id.len() > 1 && id.starts_with('0')) {
        return Err("the program key's id is not a number");
    }
    id.parse().map_err(|_| "the program key's id is not a number")
}

fn eq_hex(v: &Value, want: &[u8; 32]) -> bool {
    v.as_str().is_some_and(|s| s.eq_ignore_ascii_case(&hex(want)))
}

fn check(ok: bool, why: &'static str) -> Result<(), &'static str> {
    if ok {
        Ok(())
    } else {
        Err(why)
    }
}

/// near-antibody's `base_for`: the base witness a run at `round` reads.
fn base_for<'a>(meta: &'a Value, round: u32) -> Option<&'a Value> {
    let manifests = meta["round_manifests"].as_array().map(|a| a.len()).unwrap_or(0);
    if round as usize == manifests + 1 && !meta["proposal_base"].is_null() {
        return Some(&meta["proposal_base"]);
    }
    meta["round_bases"]
        .as_array()
        .and_then(|bases| bases.iter().take(round as usize).rev().find(|b| !b.is_null()))
        .or_else(|| (!meta["base"].is_null()).then_some(&meta["base"]))
}

pub fn compare(p: &Pinned, pv: &PublicValues, c: &Chain) -> Result<Outcome, &'static str> {
    let pid = program_id(p.antibody, &pv.program_key)?;
    // The run, as near-antibody recorded it, is the one the proof re-ran.
    let run = c.run;
    check(run["id"].as_u64() == Some(pv.run_id), "the proof is about another run")?;
    check(run["program"].as_u64() == Some(pid as u64) && c.program["id"].as_u64() == Some(pid as u64), "the run is in another program")?;
    check(c.program["key"].as_str() == Some(pv.program_key.as_str()), "the program's key differs")?;
    check(matches!(run["mode"].as_str(), Some("verify" | "second" | "rerun")), "the run is not a verification run")?;
    check(eq_hex(&run["request_sha256"], &pv.request_sha256), "the proof ran on another input than near-antibody sent the TEE (request sha256)")?;
    check(run["round"].as_u64() == Some(pv.round as u64), "the run is in another round")?;
    // The commit and its blob.
    if pv.commit_id == KNOWN_ISSUE_COMMIT {
        check(run["commit"].is_null(), "the run names a commit; the proof names a known issue")?;
    } else {
        check(run["commit"].as_u64() == Some(pv.commit_id), "the run is another commit's")?;
        let commit = c.commit.ok_or("near-antibody has no such commit")?;
        check(commit["id"].as_u64() == Some(pv.commit_id), "another commit")?;
        check(commit["program"].as_u64() == Some(pid as u64), "the commit is to another program")?;
        check(commit["round"].as_u64() == Some(pv.round as u64), "the commit is in another round")?;
        check(eq_hex(&commit["blob_sha256"], &pv.blob_sha256), "the proof opened another blob than the commit's")?;
    }
    // The manifest, the base and the anchor the run is judged under.
    let manifest = if pv.round == 0 { &c.program["manifest_sha256"] } else { &c.program["round_manifests"][pv.round as usize - 1] };
    check(eq_hex(manifest, &pv.manifest_sha256), "the round's manifest differs")?;
    let base = base_for(c.meta, pv.round).ok_or("the program has no base witness")?;
    check(eq_hex(&base["sha256"], &pv.base_sha256), "the base witness differs")?;
    if base == &c.meta["base"] {
        // A later round's base carries its own anchor state: it is bound by the base's hash.
        check(eq_hex(&c.meta["anchor_hash"], &pv.anchor), "the anchor block differs")?;
    }
    // The verifier: the program pins the TEE wasm whose core the guest re-runs.
    check(c.pins["verifier"]["wasm_hash"].as_str().is_some_and(|h| h.eq_ignore_ascii_case(p.verifier_wasm)), "the program pins another verifier wasm than the one this guest re-runs")?;
    check(pv.verifier == p.verifier, "the proof ran another verifier version")?;
    // Severities come from the manifest the run read; they must be the ones the program prices.
    let slots = c.program["params"]["slots"].as_array().ok_or("the program has no slots")?;
    for (slot, sev) in &pv.broken {
        let priced = slots.iter().find(|s| s["slot"].as_u64() == Some(*slot as u64)).ok_or("a broken slot the program does not price")?;
        check(priced["severity"].as_str() == Some(severity_name(*sev)), "a broken slot's severity differs from the program's")?;
    }

    // The TEE's answer.
    let answered = &run["outcome"]["Answered"];
    check(!answered.is_null(), "the run has no answer to compare")?;
    let facts = &answered["facts"];
    let tee_receipt = facts["receipt_sha256"].as_str().unwrap_or_default();
    check(!tee_receipt.is_empty(), "the TEE stopped before a verdict (no receipt): nothing to compare")?;

    let mut differ = vec![];
    if !tee_receipt.eq_ignore_ascii_case(&hex(&pv.receipt_sha256)) {
        differ.push("receipt");
    }
    if facts["kind"].as_str() != Some(kind_name(pv.kind)) {
        differ.push("kind");
    }
    // A Violation's keys are the first four broken slots, in order (antibody_verifier::tee::receipt_keys).
    let proven: Vec<u64> = if kind_name(pv.kind) == "Violation" { pv.broken.iter().take(4).map(|(s, _)| *s as u64).collect() } else { vec![] };
    let recorded: Vec<u64> = facts["keys"].as_array().into_iter().flatten().filter_map(|k| k["slot"].as_u64()).collect();
    if proven != recorded {
        differ.push("slots");
    }
    Ok(if differ.is_empty() { Outcome::Confirmed } else { Outcome::Contradicts(differ) })
}

#[cfg(test)]
mod tests {
    use near_sdk::serde_json::{self, json};

    use super::*;
    use crate::pv::decode;

    pub const VERIFIER: &str = "antibody-verifier 0.1.0 revm 38.0.0 reth v2.2.0 rsp 5afe72eb12a2d6accd9d7fdff66785d91cbc8e72";
    pub const WASM: &str = "ccc27f0ca60c503c46f6f1b2024b54481709fdd0c8849fe8d9a8912c637b28c1";

    fn pinned() -> Pinned<'static> {
        Pinned { antibody: "bounty.antibody.testnet", verifier: VERIFIER, verifier_wasm: WASM }
    }

    fn chain(name: &str) -> Value {
        let path = format!("{}/tests/data/chain/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|_| panic!("{path}"))).unwrap()
    }

    struct Case {
        pv: PublicValues,
        run: Value,
        commit: Value,
        program: Value,
        meta: Value,
        pins: Value,
    }

    fn case(dir: &str, run: u64, commit: u64, program: u32) -> Case {
        let pv = std::fs::read(format!("{}/../../fixtures/zk/{dir}/public_values.bin", env!("CARGO_MANIFEST_DIR"))).unwrap();
        Case {
            pv: decode(&pv).unwrap(),
            run: chain(&format!("run-{run}")),
            commit: chain(&format!("commit-{commit}")),
            program: chain(&format!("program-{program}")),
            meta: chain(&format!("meta-{program}")),
            pins: chain(&format!("pins-{program}")),
        }
    }

    fn run(c: &Case) -> Result<Outcome, &'static str> {
        compare(&pinned(), &c.pv, &Chain { run: &c.run, commit: Some(&c.commit), program: &c.program, meta: &c.meta, pins: &c.pins })
    }

    pub fn all() -> Vec<(&'static str, u64, u64, u32)> {
        vec![("a", 33, 13, 5), ("d", 36, 16, 5), ("c", 35, 15, 5), ("practice", 53, 20, 6)]
    }

    #[test]
    fn each_disclosed_runs_public_values_match_near_antibodys_records() {
        for (dir, r, c, p) in all() {
            assert_eq!(run(&case(dir, r, c, p)), Ok(Outcome::Confirmed), "{dir}");
        }
    }

    #[test]
    fn a_tee_that_reported_another_verdict_is_contradicted() {
        // The TEE said NoViolation for A's input: another receipt, another kind, no keys.
        let mut c = case("a", 33, 13, 5);
        let facts = &mut c.run["outcome"]["Answered"]["facts"];
        facts["receipt_sha256"] = json!("11".repeat(32));
        facts["kind"] = json!("NoViolation");
        facts["keys"] = json!([]);
        assert_eq!(run(&c), Ok(Outcome::Contradicts(vec!["receipt", "kind", "slots"])));
        // The true receipt's hash, but facts that disagree with it.
        let mut c = case("c", 35, 15, 5);
        c.run["outcome"]["Answered"]["facts"]["kind"] = json!("Violation");
        c.run["outcome"]["Answered"]["facts"]["keys"] = json!([{"slot": 0, "tag": "00".repeat(32)}]);
        assert_eq!(run(&c), Ok(Outcome::Contradicts(vec!["kind", "slots"])));
    }

    #[test]
    fn a_proof_about_another_input_is_rejected() {
        let base = || case("a", 33, 13, 5);
        let mut c = base();
        c.run["request_sha256"] = json!("00".repeat(32));
        assert!(run(&c).unwrap_err().contains("request sha256"));
        // Another run's proof against this run's records.
        let mut c = base();
        c.pv = case("d", 36, 16, 5).pv;
        assert!(run(&c).is_err());
        let mut c = base();
        c.commit["blob_sha256"] = json!("00".repeat(32));
        assert!(run(&c).unwrap_err().contains("blob"));
        let mut c = base();
        c.program["manifest_sha256"] = json!("00".repeat(32));
        assert!(run(&c).unwrap_err().contains("manifest"));
        let mut c = base();
        c.meta["base"]["sha256"] = json!("00".repeat(32));
        assert!(run(&c).unwrap_err().contains("base"));
        let mut c = base();
        c.meta["anchor_hash"] = json!("00".repeat(32));
        assert!(run(&c).unwrap_err().contains("anchor"));
        let mut c = base();
        c.pins["verifier"]["wasm_hash"] = json!("00".repeat(32));
        assert!(run(&c).unwrap_err().contains("verifier wasm"));
        let mut c = base();
        c.pv.verifier.push('!');
        assert!(run(&c).unwrap_err().contains("verifier version"));
        let mut c = base();
        c.program["params"]["slots"][0]["severity"] = json!("low");
        assert!(run(&c).unwrap_err().contains("severity"));
        let mut c = base();
        c.run["outcome"] = json!("Pending");
        assert!(run(&c).unwrap_err().contains("no answer"));
        let mut c = base();
        c.run["outcome"]["Answered"]["facts"]["receipt_sha256"] = json!("");
        assert!(run(&c).unwrap_err().contains("before a verdict"));
        let mut c = base();
        c.pv.program_key = "bounty.antibody.testnet/6".into();
        assert!(run(&c).is_err(), "another program");
        let other = Pinned { antibody: "other.testnet", ..pinned() };
        assert!(compare(&other, &base().pv, &Chain { run: &base().run, commit: Some(&base().commit), program: &base().program, meta: &base().meta, pins: &base().pins }).is_err());
    }

    #[test]
    fn program_keys_parse_strictly() {
        assert_eq!(program_id("bounty.antibody.testnet", "bounty.antibody.testnet/5"), Ok(5));
        for bad in ["bounty.antibody.testnet/", "bounty.antibody.testnet/05", "bounty.antibody.testnet/5x", "bounty.antibody.testnetx/5", "x.testnet/5"] {
            assert!(program_id("bounty.antibody.testnet", bad).is_err(), "{bad}");
        }
    }
}

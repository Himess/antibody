//! Immune memory v2 (docs/immune.md): a pull request's check through near-antibody's rounds. The pull
//! request's build is proposed as the next round of a program kept for these checks, the keeper requests a
//! re-run of every paid proof at it, and each re-run runs inside the enclave. Only which keys still break is
//! published (SA-19). This module reads that state: whether the program's one proposal slot is free, and,
//! per paid key, whether the proposed code still breaks it.

use std::time::Duration;

use eyre::Result;
use serde_json::{json, Value};

use crate::{cmd, ctx::Ctx};

/// What near-antibody makes of a proposal from its round check, as `proposal_stale` and
/// `proposal_accepted_at` do (crates/rules/src/engine.rs):
/// - `accepted`: the check answered `ok` and is T1-confirmed. Only opening the round clears it.
/// - `checking`: the check is pending, or answered at T0, within `run_timeout`.
/// - `stale`: the project may replace it with another proposal (a late T1 can still accept it until then).
pub fn proposal_state(check_run: &Value, now: u64, run_timeout: u64) -> &'static str {
    let o = &check_run["outcome"];
    if let Some(a) = o.get("Answered") {
        let ok = a["facts"]["extra"]["RoundCheck"]["ok"] == json!(true);
        return match (a["tier"].as_str(), ok) {
            (Some("T1"), true) => "accepted",
            (Some("T0"), _) if now <= a["at"].as_u64().unwrap_or(0) + run_timeout => "checking",
            _ => "stale",
        };
    }
    if o.get("Failed").is_some() || check_run.is_null() {
        return "stale";
    }
    if now >= check_run["started_at"].as_u64().unwrap_or(0) + run_timeout {
        "stale"
    } else {
        "checking"
    }
}

/// One paid key at the proposed code: `still breaks` when the commit's T1 re-run at `round` names the key's
/// tag again, `fixed` when that re-run was judged (Violation or NoViolation) without it, and `no verdict
/// yet` otherwise (no re-run at this round, pending, failed, unjudged, or still T0).
pub fn key_verdict(rerun: Option<&Value>, round: u64, tag: &str) -> &'static str {
    let Some(r) = rerun.filter(|r| r["round"].as_u64() == Some(round)) else { return "no verdict yet" };
    let a = &r["outcome"]["Answered"];
    let judged = matches!(a["facts"]["kind"].as_str(), Some("Violation" | "NoViolation"));
    if a["tier"] != "T1" || !judged {
        return "no verdict yet";
    }
    let breaks = a["facts"]["keys"].as_array().into_iter().flatten().any(|k| k["tag"].as_str() == Some(tag));
    if breaks {
        "still breaks"
    } else {
        "fixed"
    }
}

fn chain_now(ctx: &Ctx) -> Result<u64> {
    crate::near::block_secs(&ctx.near.call("block", json!({"finality": "final"}))?["header"])
}

/// The program's proposal slot: free (no proposal, or a stale one the project may replace), or taken, with
/// when its re-run window ends and the round may open.
pub fn state(ctx: &Ctx, program: u32) -> Result<Value> {
    let p = ctx.program(program)?;
    let now = chain_now(ctx)?;
    let w = &p["params"]["windows"];
    let (run_timeout, rerun_window, round_min) =
        (w["run_timeout"].as_u64().unwrap_or(0), w["rerun_window"].as_u64().unwrap_or(0), w["round_min"].as_u64().unwrap_or(0));
    let open = p["phase"] == "Open" && p["opening"].is_null();
    let mut out = json!({"program": program, "phase": p["phase"], "round": p["round"], "opening": !p["opening"].is_null(), "now": now});
    let Some(check) = p["proposal"]["check_run"].as_u64() else {
        out["proposal"] = Value::Null;
        out["free"] = json!(open);
        return Ok(out);
    };
    let run = cmd::run(ctx, check)?;
    let st = proposal_state(&run, now, run_timeout);
    let mut prop = json!({"round": p["proposal"]["round"], "check_run": check, "state": st});
    if st == "accepted" {
        let at = run["outcome"]["Answered"]["at"].as_u64().unwrap_or(0);
        let window_ends = at + rerun_window;
        prop["rerun_window_ends_at"] = json!(window_ends);
        // Also needed to open: every commit allocated, and no re-run in flight (near-antibody checks both).
        prop["opens_from"] = json!(window_ends.max(p["round_started_at"].as_u64().unwrap_or(0) + round_min));
    }
    out["proposal"] = prop;
    out["free"] = json!(open && st == "stale");
    Ok(out)
}

/// Per paid key of the program's eligible commits, what the round at `round` (default: the proposal's, or
/// the round just opened) makes of it. A commit's keys are its own verdict's; a gap claim has none.
pub fn results(ctx: &Ctx, program: u32, round: Option<u64>, site: Option<&str>) -> Result<Value> {
    let p = ctx.program(program)?;
    let meta = cmd::meta(ctx, program)?;
    let round = round.or_else(|| p["proposal"]["round"].as_u64()).or_else(|| p["round"].as_u64()).unwrap_or(0);
    let name_of = |slot: u64| -> (String, String) {
        meta["slots"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s[0].as_u64() == Some(slot))
            .map(|s| (s[1].as_str().unwrap_or("").to_string(), s[2].as_str().unwrap_or("").to_lowercase()))
            .unwrap_or_default()
    };
    let (mut rows, mut counts) = (Vec::new(), json!({"still breaks": 0, "fixed": 0, "no verdict yet": 0}));
    for c in cmd::list_commits(ctx, program)? {
        let label = cmd::state_label(&c["state"]);
        if !matches!(label.as_str(), "Holder" | "Duplicate" | "GapClaim" | "Promoted" | "Done:Paid" | "Done:Exhausted") {
            continue;
        }
        let verify = match c["run"].as_u64() {
            Some(r) => cmd::run(ctx, r)?,
            None => continue,
        };
        let rerun = match c["rerun"].as_u64() {
            Some(r) => Some(cmd::run(ctx, r)?),
            None => None,
        };
        let at_round = rerun.as_ref().filter(|r| r["round"].as_u64() == Some(round));
        let tier = at_round.and_then(|r| r["outcome"]["Answered"]["tier"].as_str()).unwrap_or("-");
        let rerun_id = at_round.and_then(|r| r["id"].as_u64());
        for k in verify["outcome"]["Answered"]["facts"]["keys"].as_array().into_iter().flatten() {
            let (slot, tag) = (k["slot"].as_u64().unwrap_or(0), k["tag"].as_str().unwrap_or(""));
            let verdict = key_verdict(rerun.as_ref(), round, tag);
            counts[verdict] = json!(counts[verdict].as_u64().unwrap_or(0) + 1);
            let (rule, severity) = name_of(slot);
            rows.push(json!({
                "commit": c["id"], "state": label, "slot": slot, "tag": tag,
                // Only a key that still breaks names its rule and severity here: near-antibody publishes the slot of
                // every key, so the rule is public, while where it breaks stays sealed (SA-19).
                "rule": rule, "severity": severity, "verdict": verdict, "rerun_run": rerun_id, "tier": tier,
                "run_page": match (site, rerun_id) { (Some(s), Some(id)) => json!(format!("{}/runs/{id}", s.trim_end_matches('/'))), _ => Value::Null },
            }));
        }
    }
    let n = |v: &str| counts[v].as_u64().unwrap_or(0);
    // No paid key: nothing was re-run, which is never a pass.
    if rows.is_empty() {
        return Ok(not_checked(program, Some(round), None, "the program has no paid key at this round: nothing was re-run", &Value::Null));
    }
    let verdict = if n("still breaks") > 0 { "red" } else if n("no verdict yet") > 0 { "inconclusive" } else { "green" };
    Ok(json!({"program": program, "round": round, "keys": rows, "counts": counts, "verdict": verdict,
              "note": "re-run inside the enclave through near-antibody's rounds; only which keys still break is published"}))
}

/// Why near-antibody did not accept a proposal, from its round-check run: the check failed, or answered
/// anything but `ok` (a wrong round number, canaries that broke, an InfraError). `None` while it is pending
/// or once it answered `ok`.
pub fn check_refusal(check_run: &Value) -> Option<String> {
    let o = &check_run["outcome"];
    if let Some(f) = o.get("Failed") {
        return Some(format!("the round check failed: {}", f["reason"].as_str().unwrap_or("no reason given")));
    }
    let a = o.get("Answered")?;
    if a["facts"]["extra"]["RoundCheck"]["ok"] == json!(true) {
        return None;
    }
    Some(format!("near-antibody's round check refused the proposal: {}", a["facts"]["reason"].as_str().unwrap_or("its check was not ok")))
}

/// A check that did not run: no paid proof was re-run against the proposed code, so it is never a pass.
fn not_checked(program: u32, round: Option<u64>, check_run: Option<u64>, why: &str, proposal: &Value) -> Value {
    json!({"program": program, "round": round, "check_run": check_run, "keys": [], "verdict": "error", "reason": why,
           "counts": {"still breaks": 0, "fixed": 0, "no verdict yet": 0}, "proposal": proposal,
           "note": "the proposal was not checked: no paid proof was re-run against this code"})
}

/// The proposal a check sent: the round it proposed (after renumbering) and its round-check run.
#[derive(Clone, Copy, Debug, Default)]
pub struct Proposed {
    pub round: Option<u64>,
    pub check_run: Option<u64>,
}

/// `results` for one proposal: an error, never another round's results, when its round check refused it.
pub fn results_of(ctx: &Ctx, program: u32, proposed: Proposed, site: Option<&str>) -> Result<Value> {
    if let Some(id) = proposed.check_run {
        if let Some(why) = check_refusal(&cmd::run(ctx, id)?) {
            return Ok(not_checked(program, proposed.round, Some(id), &why, &Value::Null));
        }
    }
    results(ctx, program, proposed.round, site)
}

/// `results`, waiting up to `wait` seconds for the check and every paid key's re-run to settle. It follows
/// one proposal: the one given (`round propose` prints its round and check run), else the one there when it
/// starts. It stops once no key of that round is left without a verdict, once the proposal is stale or its
/// re-run window has ended, and with an error once its check refused it or it is gone without its round
/// opening: a refused proposal used to read as the open round's results, which could be green.
pub fn wait_results(ctx: &Ctx, program: u32, wait: u64, site: Option<&str>, proposed: Proposed) -> Result<Value> {
    let start = std::time::Instant::now();
    let first = state(ctx, program)?;
    let round = proposed.round.or_else(|| first["proposal"]["round"].as_u64());
    let check_run = proposed.check_run.or_else(|| first["proposal"]["check_run"].as_u64().filter(|_| first["proposal"]["round"].as_u64() == round));
    loop {
        let st = state(ctx, program)?;
        if let Some(id) = check_run {
            if let Some(why) = check_refusal(&cmd::run(ctx, id)?) {
                return Ok(not_checked(program, round, Some(id), &why, &st["proposal"]));
            }
        }
        let r = results(ctx, program, round, site)?;
        let prop = &st["proposal"];
        // The proposal followed: its round-check run when known (another proposal may take the same round once
        // this one went stale), else its round.
        let ours = match check_run {
            Some(id) => prop["check_run"].as_u64() == Some(id),
            None => round.is_some() && prop["round"].as_u64() == round,
        };
        let opened = round.is_some_and(|n| st["round"].as_u64().unwrap_or(0) >= n);
        if round.is_some() && !ours && !opened {
            return Ok(not_checked(program, round, check_run, "the proposal is gone and its round never opened (refused, or replaced while stale)", prop));
        }
        let window_over = ours && prop["rerun_window_ends_at"].as_u64().is_some_and(|e| st["now"].as_u64().unwrap_or(0) > e);
        let settled = r["verdict"] != "inconclusive" && (opened || (ours && prop["state"] == "accepted"));
        let stale = ours && prop["state"] == "stale";
        if settled || stale || window_over || round.is_none() || start.elapsed() >= Duration::from_secs(wait) {
            let mut r = r;
            r["proposal"] = st["proposal"].clone();
            return Ok(r);
        }
        std::thread::sleep(Duration::from_secs(15));
    }
}

/// The check's job summary.
pub fn markdown(r: &Value) -> String {
    if r["verdict"] == "error" {
        return [
            "### Antibody immune memory: the check did not run".to_string(),
            String::new(),
            format!("Program {}, round {}: {}.", r["program"], r["round"], r["reason"].as_str().unwrap_or("no reason given")),
            String::new(),
            "No paid proof was re-run against this pull request's code, so this is not a pass.".into(),
        ]
        .join("\n");
    }
    let title = match r["verdict"].as_str() {
        Some("red") => "a paid proof still breaks a rule against this pull request's code",
        Some("green") => "no paid proof breaks a rule against this pull request's code",
        _ => "not every paid proof has a verdict yet",
    };
    let mut out = vec![
        format!("### Antibody immune memory: {title}"),
        String::new(),
        format!("Program {}, round {}: {}.", r["program"], r["round"], r["note"].as_str().unwrap_or("")),
        String::new(),
        "| Commit | Key | Rule | Verdict | Re-run | Tier |".into(),
        "|---|---|---|---|---|---|".into(),
    ];
    for k in r["keys"].as_array().into_iter().flatten() {
        let rule = if k["verdict"] == "still breaks" { format!("{} ({})", k["rule"].as_str().unwrap_or(""), k["severity"].as_str().unwrap_or("")) } else { "-".into() };
        let rerun = match (k["rerun_run"].as_u64(), k["run_page"].as_str()) {
            (Some(id), Some(url)) => format!("[run {id}]({url})"),
            (Some(id), None) => format!("run {id}"),
            _ => "-".into(),
        };
        out.push(format!("| {} | slot {} | {} | {} | {} | {} |", k["commit"], k["slot"], rule, k["verdict"].as_str().unwrap_or(""), rerun, k["tier"].as_str().unwrap_or("-")));
    }
    if r["keys"].as_array().map_or(true, |k| k.is_empty()) {
        out.push("| - | - | - | the program has no paid key | - | - |".into());
    }
    out.push(String::new());
    out.push("Nothing sealed is shown: a key's rule appears only when the key still breaks.".into());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answered(tier: &str, kind: &str, ok: Option<bool>, keys: Value) -> Value {
        let extra = match ok {
            Some(ok) => json!({"RoundCheck": {"ok": ok, "canaries_ok": true}}),
            None => json!("None"),
        };
        json!({"id": 7, "round": 3, "started_at": 900, "outcome": {"Answered": {"tier": tier, "at": 1000,
            "facts": {"kind": kind, "keys": keys, "extra": extra}}}})
    }

    /// The same reading of a proposal as near-antibody's: accepted only at T1 with `ok`, checking within the run
    /// timeout, stale past it or after any other answer.
    #[test]
    fn a_proposal_is_read_as_near_antibody_reads_it() {
        assert_eq!(proposal_state(&answered("T1", "NoViolation", Some(true), json!([])), 99_999, 240), "accepted");
        assert_eq!(proposal_state(&answered("T0", "NoViolation", Some(true), json!([])), 1240, 240), "checking");
        assert_eq!(proposal_state(&answered("T0", "NoViolation", Some(true), json!([])), 1241, 240), "stale");
        assert_eq!(proposal_state(&answered("T1", "NoViolation", Some(false), json!([])), 1001, 240), "stale");
        assert_eq!(proposal_state(&json!({"started_at": 900, "outcome": "Pending"}), 1139, 240), "checking");
        assert_eq!(proposal_state(&json!({"started_at": 900, "outcome": "Pending"}), 1140, 240), "stale");
        assert_eq!(proposal_state(&json!({"started_at": 900, "outcome": {"Failed": {"reason": "x"}}}), 901, 240), "stale");
    }

    /// A key still breaks only when its own tag is in the T1 re-run at this round; a judged T1 re-run without
    /// it is a fix; anything else is not a verdict yet.
    #[test]
    fn a_paid_key_is_judged_by_its_own_tag_at_this_round() {
        let tag = "aa".repeat(32);
        let breaks = answered("T1", "Violation", None, json!([{"slot": 1, "tag": tag}]));
        let other = answered("T1", "Violation", None, json!([{"slot": 1, "tag": "bb".repeat(32)}]));
        let clean = answered("T1", "NoViolation", None, json!([]));
        assert_eq!(key_verdict(Some(&breaks), 3, &tag), "still breaks");
        assert_eq!(key_verdict(Some(&other), 3, &tag), "fixed", "another key breaks; this one does not");
        assert_eq!(key_verdict(Some(&clean), 3, &tag), "fixed");
        assert_eq!(key_verdict(Some(&breaks), 4, &tag), "no verdict yet", "a re-run at another round");
        assert_eq!(key_verdict(Some(&answered("T0", "Violation", None, json!([{"slot": 1, "tag": tag}]))), 3, &tag), "no verdict yet");
        assert_eq!(key_verdict(Some(&answered("T1", "InfraError", None, json!([]))), 3, &tag), "no verdict yet");
        assert_eq!(key_verdict(None, 3, &tag), "no verdict yet");
    }

    /// A check that failed or answered anything but `ok` refuses the proposal, with near-antibody's reason; a
    /// pending check or an `ok` one does not. The summary of a refused check never reads as a pass.
    #[test]
    fn a_refused_round_check_is_an_error_never_a_pass() {
        let mut wrong = answered("T1", "NoViolation", Some(false), json!([]));
        wrong["outcome"]["Answered"]["facts"]["reason"] = json!("wrong round number");
        assert_eq!(check_refusal(&wrong).as_deref(), Some("near-antibody's round check refused the proposal: wrong round number"));
        assert!(check_refusal(&json!({"outcome": {"Failed": {"reason": "OutLayer returned no output"}}})).unwrap().contains("no output"));
        assert!(check_refusal(&answered("T1", "InfraError", None, json!([]))).is_some(), "an InfraError check is not ok");
        assert_eq!(check_refusal(&answered("T0", "NoViolation", Some(true), json!([]))), None);
        assert_eq!(check_refusal(&json!({"started_at": 900, "outcome": "Pending"})), None);
        let r = not_checked(12, Some(4), Some(120), "near-antibody's round check refused the proposal: wrong round number", &Value::Null);
        assert_eq!(r["verdict"], "error");
        let md = markdown(&r);
        assert!(md.contains("did not run") && md.contains("wrong round number") && md.contains("not a pass"), "{md}");
    }

    /// The summary names a rule only for a key that still breaks, and says when there is no paid key at all.
    #[test]
    fn the_summary_shows_nothing_sealed() {
        let r = json!({"program": 12, "round": 3, "verdict": "red", "note": "n",
            "keys": [{"commit": 1, "slot": 1, "rule": "fees_to_treasury", "severity": "high", "verdict": "still breaks", "rerun_run": 9, "run_page": "https://s/runs/9", "tier": "T1"},
                     {"commit": 2, "slot": 0, "rule": "solvency", "severity": "critical", "verdict": "fixed", "rerun_run": 10, "run_page": null, "tier": "T1"}]});
        let md = markdown(&r);
        assert!(md.contains("fees_to_treasury (high)") && md.contains("[run 9](https://s/runs/9)"));
        assert!(!md.contains("solvency"), "a fixed key's rule is not repeated");
        assert!(markdown(&json!({"program": 1, "round": 1, "verdict": "green", "keys": []})).contains("no paid key"));
    }
}

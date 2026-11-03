//! Foundry compatibility: a forge-test PoC becomes an Antibody bundle (`antibody poc from-forge`).
//!
//! The test runs once under `forge test -vvvv --json` (forked at the program's anchor, or on a local copy of
//! it), and its top-level calls become the bundle's calls: every call and contract creation the test makes
//! as a pranked sender, in order. The cheatcodes are restricted to what a bundle can express:
//!
//! - `prank`/`startPrank`/`stopPrank`, only as the test's own senders (at most two, never the protocol's
//!   accounts): they become the bundle's fresh senders;
//! - `deal`, only to those senders: the largest becomes the bundle's endowment;
//! - assertion and labeling helpers that change no state (`expectRevert`, `label`, `addr`, `assume`, reads).
//!
//! Anything else is refused, naming the cheatcode: storage writes (`store`), code changes (`etch`), mocked
//! calls, balances for other accounts, and time travel (`warp`, `roll`: the verifier runs every call in
//! the anchor block). A call the test makes as itself, rather than as a sender, is refused too.
//!
//! In the bundle, a sender's address becomes its placeholder (`5e…5e`, `5f…5f`), and a contract a call
//! created becomes `5c…5c<k>` (call k's creation), so the bundle holds for any salt.
//!
//! Rules, the other way round: a contract's Foundry-style invariant functions (`invariant_*()`, views)
//! become the program's slots (`antibody invariants from-foundry`): a generated contract inherits it and
//! `FoundryInvariants`, which turns each function into one bit.

use std::{collections::BTreeMap, path::Path, process::Command};

use alloy_primitives::{keccak256, Address};
use eyre::{eyre, Result};
use serde_json::{json, Value};

/// The cheatcode contract's address.
pub const CHEATCODES: &str = "0x7109709ecfa91a80626ff3989d68f67f5b1dd12d";

/// Cheatcodes that change no chain state a bundle would need.
const HARMLESS: &[&str] = &[
    "label(address,string)",
    "expectRevert()",
    "expectRevert(bytes4)",
    "expectRevert(bytes)",
    "addr(uint256)",
    "assume(bool)",
    "load(address,bytes32)",
    "getNonce(address)",
    "recordLogs()",
    "getRecordedLogs()",
    "toString(address)",
    "toString(uint256)",
    "toString(bytes)",
    "toString(bytes32)",
    "envOr(string,string)",
    "envString(string)",
];
const PRANKS: &[&str] = &["prank(address)", "prank(address,address)", "startPrank(address)", "startPrank(address,address)"];
const KNOWN_REFUSED: &[&str] = &[
    "store(address,bytes32,bytes32)",
    "etch(address,bytes)",
    "warp(uint256)",
    "roll(uint256)",
    "mockCall(address,bytes,bytes)",
    "mockCall(address,uint256,bytes,bytes)",
    "setNonce(address,uint64)",
    "deal(address,address,uint256)",
    "createSelectFork(string,uint256)",
    "selectFork(uint256)",
    "allowCheatcodes(address)",
    "ffi(string[])",
];

fn selector(sig: &str) -> String {
    hex::encode(&keccak256(sig.as_bytes())[..4])
}

fn word_address(data: &str, i: usize) -> Option<String> {
    let w = data.get(8 + 64 * i..8 + 64 * (i + 1))?;
    Some(format!("0x{}", &w[24..]))
}

fn word_u128(data: &str, i: usize) -> Option<u128> {
    u128::from_str_radix(data.get(8 + 64 * i..8 + 64 * (i + 1))?, 16).ok()
}

/// The bundle a test's execution trace (forge `--json`, `traces`) makes, and what the translation saw.
pub fn bundle_from_trace(arena: &[Value], endowment_floor: u128) -> Result<(Value, Value)> {
    let names: BTreeMap<String, &str> = HARMLESS.iter().chain(PRANKS).chain(KNOWN_REFUSED).chain(["stopPrank()", "deal(address,uint256)"].iter()).map(|s| (selector(s), *s)).collect();
    let harmless: Vec<String> = HARMLESS.iter().map(|s| selector(s)).collect();
    let pranks: Vec<String> = PRANKS.iter().map(|s| selector(s)).collect();
    let (stop, deal) = (selector("stopPrank()"), selector("deal(address,uint256)"));
    let test = arena.iter().find(|n| n["trace"]["depth"] == json!(0)).and_then(|n| n["trace"]["address"].as_str()).unwrap_or_default().to_lowercase();
    let mut senders: Vec<String> = Vec::new();
    let mut deals: BTreeMap<String, u128> = BTreeMap::new();
    let mut cheats = Vec::new();
    let mut calls: Vec<(String, Option<String>, u128, String, u64)> = Vec::new();
    let mut created: Vec<(String, usize)> = Vec::new();
    for n in arena {
        let t = &n["trace"];
        if t["depth"] != json!(1) {
            continue;
        }
        let kind = t["kind"].as_str().unwrap_or_default();
        let caller = t["caller"].as_str().unwrap_or_default().to_lowercase();
        let to = t["address"].as_str().unwrap_or_default().to_lowercase();
        let data = t["data"].as_str().unwrap_or_default().trim_start_matches("0x").to_lowercase();
        if to == CHEATCODES {
            let sel = data.get(..8).unwrap_or_default().to_string();
            let name = names.get(&sel).copied().unwrap_or("an unknown cheatcode");
            cheats.push(json!({"selector": format!("0x{sel}"), "name": name}));
            if pranks.contains(&sel) {
                let who = word_address(&data, 0).ok_or_else(|| eyre!("{name}: no address"))?;
                if (sel == pranks[1] || sel == pranks[3]) && word_address(&data, 1).as_deref() != Some(who.as_str()) {
                    return Err(eyre!("{name}: tx.origin must be the sender itself"));
                }
                if !senders.contains(&who) {
                    if senders.len() == 2 {
                        return Err(eyre!("a bundle has at most two senders; the test pranks a third ({who})"));
                    }
                    senders.push(who);
                }
            } else if sel == deal {
                let who = word_address(&data, 0).ok_or_else(|| eyre!("deal: no address"))?;
                let amount = word_u128(&data, 1).ok_or_else(|| eyre!("deal: amount above 2^128"))?;
                let e = deals.entry(who).or_default();
                *e = (*e).max(amount);
            } else if sel != stop && !harmless.contains(&sel) {
                return Err(eyre!("{name} (0x{sel}) is not allowed: a bundle can express only pranks as its own senders, deals to them and helpers that change no state"));
            }
            continue;
        }
        if kind == "STATICCALL" {
            continue;
        }
        if !senders.contains(&caller) {
            if caller == test {
                return Err(eyre!("the test calls {to} as itself; make every call as a pranked sender"));
            }
            continue;
        }
        let gas = t["gas_used"].as_u64().unwrap_or(0).saturating_mul(3).saturating_add(100_000).clamp(200_000, 15_000_000);
        let value = u128::from_str_radix(t["value"].as_str().unwrap_or("0x0").trim_start_matches("0x"), 16).unwrap_or(0);
        if kind.starts_with("CREATE") {
            created.push((to.clone(), calls.len()));
            calls.push((caller, None, value, data, gas));
        } else {
            calls.push((caller, Some(to), value, data, gas));
        }
    }
    for who in deals.keys() {
        if !senders.contains(who) {
            return Err(eyre!("deal to {who}, which is not one of the test's senders: a bundle funds only its senders"));
        }
    }
    if calls.is_empty() {
        return Err(eyre!("the test made no call as a pranked sender"));
    }
    let placeholder = |i: usize| if i == 0 { "5e".repeat(20) } else { "5f".repeat(20) };
    let created_ph = |k: usize| format!("{}{k:02x}", "5c".repeat(19));
    let swap = |mut s: String| {
        for (i, a) in senders.iter().enumerate() {
            s = s.replace(a.trim_start_matches("0x"), &placeholder(i));
        }
        for (a, k) in &created {
            s = s.replace(a.trim_start_matches("0x"), &created_ph(*k));
        }
        s
    };
    let out: Vec<Value> = calls
        .iter()
        .map(|(caller, to, value, data, gas)| {
            let sender = senders.iter().position(|s| s == caller).unwrap_or(0);
            let to = to.as_ref().map(|t| format!("0x{}", swap(t.trim_start_matches("0x").to_string())));
            json!({"sender": sender, "to": to, "value": value.to_string(), "data": format!("0x{}", swap(data.clone())), "gas_limit": gas})
        })
        .collect();
    let endowment = deals.values().copied().max().unwrap_or(0).max(endowment_floor);
    let bundle = json!({"senders": senders.len(), "endowment_wei": endowment.to_string(), "protected": [], "calls": out});
    let report = json!({"senders": senders, "deals": deals.iter().map(|(k, v)| (k.clone(), v.to_string())).collect::<BTreeMap<_, _>>(),
                        "cheatcodes": cheats, "created": created.iter().map(|(a, k)| json!({"address": a, "by_call": k})).collect::<Vec<_>>()});
    Ok((bundle, report))
}

/// Runs one forge test and translates it (`fork` = (rpc, block) at the program's anchor, if any).
pub fn from_forge(project: &Path, contract: &str, test: &str, fork: Option<(&str, u64)>, forge: &Path, endowment_floor: u128) -> Result<Value> {
    let mut cmd = Command::new(forge);
    cmd.current_dir(project).args(["test", "--match-contract", contract, "--match-test", test, "-vvvv", "--json"]);
    if let Some((rpc, block)) = fork {
        cmd.args(["--fork-url", rpc, "--fork-block-number", &block.to_string()]);
    }
    let out = cmd.output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json_line = stdout.lines().rev().find(|l| l.trim_start().starts_with('{')).ok_or_else(|| {
        eyre!("forge printed no JSON: {}", String::from_utf8_lossy(&out.stderr).lines().rev().take(5).collect::<Vec<_>>().join(" | "))
    })?;
    let v: Value = serde_json::from_str(json_line)?;
    let (path, result) = v
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(p, c)| c["test_results"].as_object().into_iter().flatten().map(move |(t, r)| (p.clone(), t.clone(), r.clone())))
        .find(|(p, t, _)| p.ends_with(&format!(":{contract}")) && t.starts_with(&format!("{test}(")))
        .map(|(p, _, r)| (p, r))
        .ok_or_else(|| eyre!("no result for {contract}.{test} in forge's output"))?;
    let arena = result["traces"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t[0] == json!("Execution"))
        .and_then(|t| t[1]["arena"].as_array().cloned())
        .ok_or_else(|| eyre!("no execution trace for {contract}.{test} (forge -vvvv --json)"))?;
    let (bundle, report) = bundle_from_trace(&arena, endowment_floor)?;
    Ok(json!({"bundle": bundle, "test": format!("{path}.{test}"), "forge_status": result["status"], "translation": report,
              "note": "check it before you commit: `antibody check --program <id> --bundle <file>`"}))
}

fn snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The rules contract for a compiled contract's Foundry-style invariant functions (`invariant_*`, views
/// with no arguments), and the manifest slots they become (bit i = the i-th function, in `order` if given,
/// else by name; severities are the project's to set). `import` is the contract's source, `base_import`
/// FoundryInvariants.sol, both as the generated file imports them.
pub fn rules_from_foundry(name: &str, abi: &[Value], import: &str, base_import: &str, order: Option<&[String]>) -> Result<(String, Value)> {
    let mut fns: Vec<String> = abi
        .iter()
        .filter(|e| e["type"] == "function" && e["inputs"].as_array().is_some_and(|i| i.is_empty()))
        .filter(|e| matches!(e["stateMutability"].as_str(), Some("view") | Some("pure")))
        .filter_map(|e| e["name"].as_str().filter(|n| n.starts_with("invariant_")).map(String::from))
        .collect();
    if let Some(o) = order {
        for f in o {
            if !fns.contains(f) {
                return Err(eyre!("{f} is not one of {name}'s invariant functions"));
            }
        }
        fns.sort_by_key(|f| o.iter().position(|x| x == f).unwrap_or(usize::MAX));
    }
    if fns.is_empty() {
        return Err(eyre!("{name} has no invariant functions (`invariant_*`, views with no arguments)"));
    }
    if fns.len() > 256 {
        return Err(eyre!("{name} has {} invariant functions; a program has at most 256 slots", fns.len()));
    }
    let mut params = Vec::new();
    let mut args = Vec::new();
    if let Some(c) = abi.iter().find(|e| e["type"] == "constructor") {
        for (i, p) in c["inputs"].as_array().into_iter().flatten().enumerate() {
            let t = p["internalType"].as_str().or(p["type"].as_str()).unwrap_or_default();
            if t.starts_with("struct ") || t.contains('[') {
                return Err(eyre!("{name}'s constructor takes {t}: write the rules contract by hand (docs/foundry.md)"));
            }
            params.push(format!("{} a{i}", t.trim_start_matches("contract ").trim_start_matches("enum ")));
            args.push(format!("a{i}"));
        }
    }
    let mut src = format!(
        "// SPDX-License-Identifier: MIT\npragma solidity 0.8.28;\n\n// Generated by `antibody invariants from-foundry` from {name}: its invariant functions are the\n\
         // program's slots, in this order. Instantiate it with {name}'s constructor arguments and commit its\n\
         // runtime code in the manifest (docs/foundry.md).\n\nimport \"{import}\";\nimport {{FoundryInvariants}} from \"{base_import}\";\n\n\
         contract {name}Rules is {name}, FoundryInvariants {{\n"
    );
    if !params.is_empty() {
        src += &format!("    constructor({}) {name}({}) {{}}\n\n", params.join(", "), args.join(", "));
    }
    src += &format!("    function _invariants() internal pure override returns (bytes4[] memory s) {{\n        s = new bytes4[]({});\n", fns.len());
    for (i, f) in fns.iter().enumerate() {
        src += &format!("        s[{i}] = {name}.{f}.selector;\n");
    }
    src += "    }\n}\n";
    let slots: Vec<Value> = fns.iter().enumerate().map(|(i, f)| json!({"bit": i, "name": snake(f.trim_start_matches("invariant_")), "severity": "medium"})).collect();
    Ok((src, json!({"contract": format!("{name}Rules"), "functions": fns, "slots": slots})))
}

/// The address call `k` of a bundle creates (for `5c…5c<k>`): its sender's CREATE at that sender's nonce.
pub fn created_address(sender: Address, nonce: u64) -> Address {
    sender.create(nonce)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(depth: u64, kind: &str, caller: &str, to: &str, data: &str) -> Value {
        json!({"trace": {"depth": depth, "kind": kind, "caller": caller, "address": to, "data": data, "value": "0x0", "gas_used": 50_000}})
    }

    const TEST: &str = "0x7fa9385be102ac3eac297483dd6233d62b3e1496";
    const ATTACKER: &str = "0x000000000000000000000000000000000000a0a0";
    const VAULT: &str = "0x1299edc79ebd316cbb8b493219f50a274f87e4d3";

    fn prank() -> Value {
        frame(1, "CALL", TEST, CHEATCODES, &format!("0x{}{:0>64}{:0>64}", selector("prank(address,address)"), "a0a0", "a0a0"))
    }

    #[test]
    fn foundry_invariant_functions_become_slots_in_the_committed_example() {
        let abi: Value = serde_json::from_str(r#"[
            {"type": "constructor", "inputs": [{"name": "factory_", "type": "address", "internalType": "contract PracticeFactory"},
                                               {"name": "each_", "type": "uint256", "internalType": "uint256"}], "stateMutability": "nonpayable"},
            {"type": "function", "name": "each", "inputs": [], "outputs": [], "stateMutability": "view"},
            {"type": "function", "name": "invariant_factoryHoldsNothing", "inputs": [], "outputs": [], "stateMutability": "view"},
            {"type": "function", "name": "invariant_vaultsKeepTheirEther", "inputs": [], "outputs": [], "stateMutability": "view"}]"#).unwrap();
        let order = vec!["invariant_vaultsKeepTheirEther".to_string(), "invariant_factoryHoldsNothing".to_string()];
        let (src, slots) = rules_from_foundry("PracticeFoundryInvariants", abi.as_array().unwrap(), "./PracticeFoundryInvariants.sol",
                                              "../../src/antibody/FoundryInvariants.sol", Some(&order)).unwrap();
        // The committed example is exactly what the generator writes.
        let committed = include_str!("../../../contracts/evm/test/compat/PracticeFoundryInvariantsRules.sol");
        assert_eq!(src, committed.replace("\r\n", "\n"));
        assert_eq!(slots["slots"][0]["name"], "vaults_keep_their_ether");
        assert_eq!(slots["slots"][1]["bit"], 1);
    }

    #[test]
    fn a_pranked_call_becomes_a_bundle_call_with_its_sender_placeholder() {
        let arena = vec![
            frame(0, "CALL", "0x1804c8ab1f12e6bbf3894d4083f33e07309d1f38", TEST, "0x7f5416dc"),
            frame(1, "STATICCALL", TEST, VAULT, "0x8da5cb5b"),
            frame(1, "CALL", TEST, CHEATCODES, &format!("0x{}{:0>64}{:0>64}", selector("deal(address,uint256)"), "a0a0", "de0b6b3a7640000")),
            prank(),
            frame(1, "CALL", ATTACKER, VAULT, &format!("0x01681a62{:0>64}", "a0a0")),
        ];
        let (b, r) = bundle_from_trace(&arena, 0).unwrap();
        assert_eq!(b["senders"], 1);
        assert_eq!(b["endowment_wei"], "1000000000000000000");
        assert_eq!(b["calls"][0]["to"], VAULT);
        assert_eq!(b["calls"][0]["data"], format!("0x01681a62{}{}", "0".repeat(24), "5e".repeat(20)));
        assert_eq!(r["senders"][0], ATTACKER);
    }

    #[test]
    fn a_created_contract_is_named_by_its_creating_call() {
        let helper = "0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f";
        let arena = vec![
            frame(0, "CALL", "0x1804c8ab1f12e6bbf3894d4083f33e07309d1f38", TEST, "0x7f5416dc"),
            prank(),
            frame(1, "CREATE", ATTACKER, helper, "0x6080"),
            prank(),
            frame(1, "CALL", ATTACKER, helper, &format!("0xdeadbeef{:0>64}", &VAULT[2..])),
        ];
        let (b, _) = bundle_from_trace(&arena, 10).unwrap();
        assert_eq!(b["calls"][0]["to"], Value::Null);
        assert_eq!(b["calls"][1]["to"], format!("0x{}00", "5c".repeat(19)));
        assert_eq!(b["endowment_wei"], "10");
    }

    #[test]
    fn storage_writes_time_travel_and_calls_as_the_test_are_refused() {
        let store = frame(1, "CALL", TEST, CHEATCODES, &format!("0x{}{:0>64}{:0>64}{:0>64}", selector("store(address,bytes32,bytes32)"), "1", "0", "1"));
        let e = bundle_from_trace(&[frame(0, "CALL", "0x1", TEST, "0x"), store], 0).unwrap_err().to_string();
        assert!(e.contains("store(address,bytes32,bytes32)"), "{e}");
        let warp = frame(1, "CALL", TEST, CHEATCODES, &format!("0x{}{:0>64}", selector("warp(uint256)"), "64"));
        assert!(bundle_from_trace(&[frame(0, "CALL", "0x1", TEST, "0x"), warp], 0).unwrap_err().to_string().contains("warp"));
        let as_itself = frame(1, "CALL", TEST, VAULT, "0x01681a62");
        assert!(bundle_from_trace(&[frame(0, "CALL", "0x1", TEST, "0x"), as_itself], 0).unwrap_err().to_string().contains("as itself"));
        let deal_other = frame(1, "CALL", TEST, CHEATCODES, &format!("0x{}{:0>64}{:0>64}", selector("deal(address,uint256)"), "beef", "1"));
        let e = bundle_from_trace(&[frame(0, "CALL", "0x1", TEST, "0x"), prank(), frame(1, "CALL", ATTACKER, VAULT, "0x01"), deal_other], 0).unwrap_err().to_string();
        assert!(e.contains("not one of the test's senders"), "{e}");
    }
}

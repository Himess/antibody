//! Invariant drafting for projects: a defensive helper whose output the project reviews and
//! approves (`antibody invariants draft`). Nothing it writes is binding until the project commits it
//! in a manifest.
//!
//! - **Scan (always, offline).** A deterministic read of the project's Solidity sources. It
//!   recognizes standard shapes and writes:
//!   - a compiling `IAntibodyInvariants` draft for the shapes it can express completely: an
//!     ERC-4626-style share price that never falls, and a mutable owner or admin that never changes;
//!   - commented templates for shapes that need the project's own numbers, such as lending solvency
//!     at a reference price.
//! - **AI (optional, `--ai`).** Asks Claude to extend that draft from the sources, as structured
//!   output. It asks for properties that must always hold, never for ways to break them.
//!
//! Either way the project then compiles the draft, commits its runtime code in `antibody.toml`, and
//! runs `antibody check` with the no-op bundle. The verifier itself enforces
//! `check(snapshot()) == 0` at the anchor (`ProgramMisconfigured` otherwise).

use std::collections::{BTreeMap, BTreeSet};

use eyre::{eyre, Result};
use serde_json::{json, Value};

#[derive(Debug, Clone, Default)]
pub struct Contract {
    pub name: String,
    pub file: String,
    pub bases: Vec<String>,
    /// Functions and public state variables (both are callable getters).
    pub members: BTreeSet<String>,
    /// Public state variables declared `immutable` or `constant` (they cannot change).
    pub fixed_vars: BTreeSet<String>,
}

/// Removes `//` and `/* */` comments (string literals are kept intact).
pub fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                let q = b[i];
                let start = i;
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(b.len());
                out.push_str(&src[start..i]);
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                out.push(' ');
            }
            _ => {
                let ch = src[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

fn word_at(s: &str, at: usize, word: &str) -> bool {
    s[at..].starts_with(word)
        && s[..at].chars().next_back().map(|c| !is_ident(c)).unwrap_or(true)
        && s[at + word.len()..].chars().next().map(|c| !is_ident(c)).unwrap_or(true)
}

fn ident_after(s: &str) -> String {
    s.trim_start().chars().take_while(|&c| is_ident(c)).collect()
}

/// Deployable contracts in one source file (interfaces, libraries and abstract contracts are skipped).
pub fn scan(src: &str, file: &str) -> Vec<Contract> {
    let s = strip_comments(src);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = s[i..].find("contract") {
        let at = i + off;
        i = at + "contract".len();
        if !word_at(&s, at, "contract") {
            continue;
        }
        let before = s[..at].trim_end();
        let is_abstract = before.ends_with("abstract") && word_at(before, before.len() - "abstract".len(), "abstract");
        let name = ident_after(&s[i..]);
        if name.is_empty() || !s[i..].starts_with(char::is_whitespace) {
            continue; // e.g. the word inside a string literal
        }
        let Some(open_rel) = s[i..].find('{') else { break };
        let header = &s[i..i + open_rel];
        let open = i + open_rel;
        let mut depth = 0usize;
        let mut close = open;
        for (k, c) in s[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + k;
                        break;
                    }
                }
                _ => {}
            }
        }
        i = close.max(open + 1);
        if is_abstract {
            continue;
        }
        let bases = match header.find(" is ") {
            Some(p) => header[p + 4..].split(',').map(ident_after).filter(|b| !b.is_empty()).collect(),
            None => Vec::new(),
        };
        let body = &s[open + 1..close];
        let mut c = Contract { name, file: file.into(), bases, ..Default::default() };
        // Top-level statements of the contract body.
        let mut depth = 0i32;
        let mut stmt = String::new();
        for ch in body.chars() {
            match ch {
                '{' => {
                    if depth == 0 {
                        record(&mut c, &stmt);
                        stmt.clear();
                    }
                    depth += 1;
                }
                '}' => depth -= 1,
                ';' if depth == 0 => {
                    record(&mut c, &stmt);
                    stmt.clear();
                }
                _ if depth == 0 => stmt.push(ch),
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

fn record(c: &mut Contract, stmt: &str) {
    let t = stmt.trim();
    if let Some(rest) = t.strip_prefix("function") {
        let n = ident_after(rest);
        if !n.is_empty() {
            c.members.insert(n);
        }
        return;
    }
    if ["event", "error", "modifier", "constructor", "using", "struct", "enum", "receive", "fallback"].iter().any(|k| t.starts_with(k)) {
        return;
    }
    let words: Vec<&str> = t.split(|ch: char| ch.is_whitespace()).filter(|w| !w.is_empty()).collect();
    if let Some(p) = words.iter().position(|w| *w == "public") {
        let mut fixed = false;
        let mut name = None;
        for w in &words[p + 1..] {
            match *w {
                "immutable" | "constant" => fixed = true,
                "override" | "virtual" => {}
                other => {
                    name = Some(other.trim_end_matches(|ch: char| !is_ident(ch)).to_string());
                    break;
                }
            }
        }
        if let Some(n) = name.filter(|n| !n.is_empty() && n.chars().all(is_ident)) {
            if fixed {
                c.fixed_vars.insert(n.clone());
            }
            c.members.insert(n);
        }
    }
}

/// Adds the members of base contracts found in the same scan (transitively).
pub fn resolve_bases(contracts: &mut [Contract], abstracts: &BTreeMap<String, Contract>) {
    let all: BTreeMap<String, Contract> =
        abstracts.iter().map(|(k, v)| (k.clone(), v.clone())).chain(contracts.iter().map(|c| (c.name.clone(), c.clone()))).collect();
    for c in contracts.iter_mut() {
        let mut todo: Vec<String> = c.bases.clone();
        let mut seen = BTreeSet::new();
        while let Some(b) = todo.pop() {
            if !seen.insert(b.clone()) {
                continue;
            }
            if let Some(base) = all.get(&b) {
                c.members.extend(base.members.iter().cloned());
                c.fixed_vars.extend(base.fixed_vars.iter().cloned());
                todo.extend(base.bases.iter().cloned());
            }
        }
    }
}

/// Abstract contracts too, for inheritance only.
pub fn scan_abstracts(src: &str, file: &str) -> Vec<Contract> {
    let s = strip_comments(src).replace("abstract contract", "contract");
    scan(&s, file)
}

#[derive(Debug, Clone, PartialEq)]
pub struct DraftInvariant {
    pub bit: u8,
    pub constant: String,
    pub slot_name: String,
    pub severity: &'static str,
    pub contract: String,
    pub property: String,
    pub holds_if: String,
}

#[derive(Debug, Clone)]
pub struct Draft {
    pub solidity: String,
    pub slots_toml: String,
    pub review: String,
    pub invariants: Vec<DraftInvariant>,
    pub templates: Vec<(String, String)>,
}

fn lower_camel(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_lowercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

enum Rule {
    SharePrice { contract: String },
    RoleUnchanged { contract: String, getter: &'static str },
}

pub fn draft(contracts: &[Contract], sources: &[String]) -> Draft {
    let mut rules = Vec::new();
    let mut templates = Vec::new();
    let mut pausable = Vec::new();
    for c in contracts {
        let has = |m: &str| c.members.contains(m);
        if has("totalAssets") && has("totalSupply") {
            rules.push(Rule::SharePrice { contract: c.name.clone() });
        }
        for getter in ["owner", "admin"] {
            if has(getter) && !c.fixed_vars.contains(getter) {
                rules.push(Rule::RoleUnchanged { contract: c.name.clone(), getter });
                break;
            }
        }
        let lending: Vec<&str> =
            ["totalSupplied", "totalDebt", "debtOf", "collateralOf", "borrow", "repay"].into_iter().filter(|m| has(m)).collect();
        if lending.len() >= 2 {
            templates.push((
                c.name.clone(),
                format!(
                    "**Solvency at a committed reference price (suggested: critical).** `{n}` looks like a lending book \
                     ({members}). Lender claims must stay covered:\n\n\
                     ```solidity\n\
                     // cash + Σ_i min(debt_i, collateral_i × P_REF / 1e18 × LT_BPS / 10_000) ≥ totalSupplied\n\
                     // P_REF: a reference price you commit as an immutable (never read a pool's spot price);\n\
                     // LT_BPS: your liquidation threshold; iterate borrowers with a bounded loop.\n\
                     ```\n\
                     `contracts/evm/src/acme/AcmeInvariants.sol` (`solvent()`) is a complete example.",
                    n = c.name,
                    members = lending.join(", ")
                ),
            ));
        }
        if has("paused") {
            pausable.push(format!("`{}`", c.name));
        }
    }
    if !pausable.is_empty() {
        templates.push((
            "Pausable contracts".into(),
            format!(
                "**Pausing is not a break.** These contracts can be paused: {}. Keep the guardian as the pauser, and \
                 do not write an invariant that a pause would trip.",
                pausable.join(", ")
            ),
        ));
    }
    let mut invariants = Vec::new();
    for (bit, r) in rules.iter().enumerate() {
        let (constant, slot, sev, contract, property, holds_if) = match r {
            Rule::SharePrice { contract } => (
                format!("SHARE_PRICE_{}", contract.to_uppercase()),
                format!("share_price_{}", contract.to_lowercase()),
                "medium",
                contract.clone(),
                format!("{contract}'s share price (totalAssets / totalSupply) never falls"),
                "the vault can never realize a loss or charge an exit fee; otherwise bound the allowed drop or remove it".to_string(),
            ),
            Rule::RoleUnchanged { contract, getter } => (
                format!("{}_UNCHANGED_{}", getter.to_uppercase(), contract.to_uppercase()),
                format!("{getter}_unchanged_{}", contract.to_lowercase()),
                "critical",
                contract.clone(),
                format!("{contract}.{getter}() never changes"),
                "nobody but the current holder can transfer the role, and a PoC never runs as the holder (fresh senders)".to_string(),
            ),
        };
        invariants.push(DraftInvariant { bit: bit as u8, constant, slot_name: slot, severity: sev, contract, property, holds_if });
    }
    let solidity = render_solidity(&rules, &invariants, sources);
    let slots_toml = invariants
        .iter()
        .map(|i| format!("[[invariants.slots]]\nbit = {}\nname = \"{}\"\nseverity = \"{}\"\n", i.bit, i.slot_name, i.severity))
        .collect::<Vec<_>>()
        .join("\n");
    let review = render_review(&invariants, &templates, sources);
    Draft { solidity, slots_toml, review, invariants, templates }
}

fn render_solidity(rules: &[Rule], inv: &[DraftInvariant], sources: &[String]) -> String {
    let targets: BTreeSet<String> = inv.iter().map(|i| i.contract.clone()).collect();
    let mut s = String::new();
    s.push_str("// SPDX-License-Identifier: MIT\npragma solidity ^0.8.20;\n\n");
    s.push_str("// DRAFT generated by `antibody invariants draft` (deterministic scan, not reviewed) from:\n");
    for f in sources {
        s.push_str(&format!("//   {f}\n"));
    }
    s.push_str("// Review every invariant in REVIEW.md, add your protocol's economics, then commit the runtime code\n");
    s.push_str("// in antibody.toml. `antibody check` with noop.bundle.json must report NoViolation at the anchor.\n\n");
    s.push_str("interface IDraftAntibodyInvariants {\n    function snapshot() external view returns (bytes memory);\n    function check(bytes calldata pre) external view returns (uint256 broken);\n}\n\n");
    if rules.iter().any(|r| matches!(r, Rule::SharePrice { .. })) {
        s.push_str("interface IDraftVault {\n    function totalAssets() external view returns (uint256);\n    function totalSupply() external view returns (uint256);\n}\n\n");
    }
    for g in ["owner", "admin"] {
        if rules.iter().any(|r| matches!(r, Rule::RoleUnchanged { getter, .. } if *getter == g)) {
            let cap = g[..1].to_uppercase() + &g[1..];
            s.push_str(&format!("interface IDraft{cap} {{\n    function {g}() external view returns (address);\n}}\n\n"));
        }
    }
    s.push_str("contract DraftInvariants is IDraftAntibodyInvariants {\n");
    for i in inv {
        s.push_str(&format!("    /// bit {} ({}): {}.\n    /// Holds only if {}.\n", i.bit, i.severity, i.property, i.holds_if));
        s.push_str(&format!("    uint256 public constant {} = 1 << {};\n", i.constant, i.bit));
    }
    s.push('\n');
    for t in &targets {
        s.push_str(&format!("    address public immutable {};\n", lower_camel(t)));
    }
    let args: Vec<String> = targets.iter().map(|t| format!("address {}_", lower_camel(t))).collect();
    s.push_str(&format!("\n    constructor({}) {{\n", args.join(", ")));
    for t in &targets {
        s.push_str(&format!("        {0} = {0}_;\n", lower_camel(t)));
    }
    s.push_str("    }\n\n");
    // snapshot: uint256 values first, then addresses, each rule in order.
    let (mut nu, mut na) = (0usize, 0usize);
    let mut snap = Vec::new();
    let mut check = Vec::new();
    for (r, i) in rules.iter().zip(inv) {
        match r {
            Rule::SharePrice { contract } => {
                let v = lower_camel(contract);
                snap.push(format!("        u[{nu}] = IDraftVault({v}).totalAssets();\n        u[{}] = IDraftVault({v}).totalSupply();\n", nu + 1));
                check.push(format!(
                    "        {{\n            uint256 a1 = IDraftVault({v}).totalAssets();\n            uint256 s1 = IDraftVault({v}).totalSupply();\n            // a1/s1 >= a0/s0, compared exactly\n            if (u[{s0}] != 0 && s1 != 0 && a1 * u[{s0}] < u[{a0}] * s1) broken |= {c};\n        }}\n",
                    s0 = nu + 1,
                    a0 = nu,
                    c = i.constant
                ));
                nu += 2;
            }
            Rule::RoleUnchanged { contract, getter } => {
                let v = lower_camel(contract);
                let cap = getter[..1].to_uppercase() + &getter[1..];
                snap.push(format!("        a[{na}] = IDraft{cap}({v}).{getter}();\n"));
                check.push(format!("        if (IDraft{cap}({v}).{getter}() != a[{na}]) broken |= {};\n", i.constant));
                na += 1;
            }
        }
    }
    s.push_str("    function snapshot() external view returns (bytes memory) {\n");
    s.push_str(&format!("        uint256[] memory u = new uint256[]({nu});\n        address[] memory a = new address[]({na});\n"));
    for l in &snap {
        s.push_str(l);
    }
    s.push_str("        return abi.encode(u, a);\n    }\n\n");
    s.push_str("    function check(bytes calldata pre) external view returns (uint256 broken) {\n");
    s.push_str("        (uint256[] memory u, address[] memory a) = abi.decode(pre, (uint256[], address[]));\n");
    if nu == 0 {
        s.push_str("        u;\n");
    }
    if na == 0 {
        s.push_str("        a;\n");
    }
    for l in &check {
        s.push_str(l);
    }
    s.push_str("    }\n}\n");
    s
}

fn render_review(inv: &[DraftInvariant], templates: &[(String, String)], sources: &[String]) -> String {
    let mut r = String::from("# Invariants draft: review before committing anything\n\n");
    r.push_str(
        "This draft comes from a deterministic scan of your sources. It is not a judgement of your code. An \
         invariant is a property that must hold for every honest use of your protocol, at the anchor and after any \
         transactions. Once committed, it decides payouts automatically, so each one needs your approval.\n\n",
    );
    r.push_str("## Drafted (in `DraftInvariants.sol`)\n\n| Bit | Severity | Property | Holds only if |\n|---|---|---|---|\n");
    for i in inv {
        r.push_str(&format!("| {} | {} | {} | {} |\n", i.bit, i.severity, i.property, i.holds_if));
    }
    if inv.is_empty() {
        r.push_str("| - | - | No standard shape found | Write your invariants from the templates below |\n");
    }
    r.push_str("\n## Templates that need your numbers\n\n");
    for (c, t) in templates {
        r.push_str(&format!("### {c}\n\n{t}\n\n"));
    }
    r.push_str(
        "**Every protocol: custody.** For each token you hold for users: \
         `token.balanceOf(protocol) >= recorded liabilities` (suggested: critical).\n\n",
    );
    r.push_str("## Validate\n\n");
    r.push_str("1. Compile `DraftInvariants.sol` with your target addresses as constructor arguments. Simulate the deployment; do not broadcast it. Take the runtime code (`contracts/evm/script/DeployAcme.s.sol` shows how).\n");
    r.push_str("2. Put the runtime code, its keccak256 and the slots from `slots.toml` into `antibody.toml`, then run `antibody manifest antibody.toml --out manifest.json`.\n");
    r.push_str("3. Run `antibody check --manifest manifest.json --bundle noop.bundle.json`. It must report `NoViolation`; `ProgramMisconfigured` means an invariant does not hold at the anchor.\n");
    r.push_str("4. Run your own known-issue PoCs. Each should break exactly the invariant you expect, and you can declare them at t0.\n\n");
    r.push_str("Sources scanned:\n");
    for f in sources {
        r.push_str(&format!("- `{f}`\n"));
    }
    r
}

/// A bundle that changes nothing: one call to the identity precompile.
pub fn noop_bundle() -> Value {
    json!({"senders": 1, "endowment_wei": "1000000000000000000", "protected": [],
           "calls": [{"sender": 0, "to": "0x0000000000000000000000000000000000000004", "value": "0", "data": "0x", "gas_limit": 100000}]})
}

// --------------------------------------------------------------------------------- AI (optional)

pub const AI_MODEL: &str = "claude-opus-5-5";
pub const API_VERSION: &str = "2026-10-21";
/// Server-side refusal fallback ("default" routes by refusal category).
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

const SYSTEM: &str = "You draft invariants for a bug-bounty program. The program pays only when a submitted sequence of \
transactions breaks one of its invariants. An invariant is a property of the protocol's state that must hold before and \
after any sequence of transactions. The project reviews and approves your draft before it becomes a binding rule, so \
precision matters more than coverage.\n\n\
Write a complete Solidity ^0.8.20 contract named DraftInvariants. It implements \
`snapshot() external view returns (bytes memory)` and `check(bytes calldata pre) external view returns (uint256 broken)`, \
where bit i of `broken` means invariant i is broken. Requirements:\n\
- View functions only. Write no storage: the code is injected at a fixed address without running a constructor at \
that address, so addresses and committed constants must be immutables set in the constructor.\n\
- `check(snapshot())` must return 0 on the protocol's current honest state.\n\
- Never revert: guard arithmetic, bound every loop, and wrap calls that could revert in try/catch.\n\
- Value assets at constants the project commits, never at a price that a trade can move.\n\
- Document every bit with its severity (critical, high, medium or low) and the assumptions it relies on.\n\
- Keep the invariants of the deterministic draft you are given, unless one is wrong for this protocol; then say why in \
review_notes.\n\n\
Describe properties only. Do not describe how they could be violated, and do not write attack or exploit code. If you \
are unsure whether a property holds for honest use, put it in review_notes as a question for the project.";

pub fn ai_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "solidity": {"type": "string"},
            "invariants": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "bit": {"type": "integer"},
                    "name": {"type": "string"},
                    "severity": {"type": "string", "enum": ["critical", "high", "medium", "low"]},
                    "property": {"type": "string"},
                    "holds_because": {"type": "string"},
                    "assumptions": {"type": "string"}
                },
                "required": ["bit", "name", "severity", "property", "holds_because", "assumptions"],
                "additionalProperties": false
            }},
            "review_notes": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["solidity", "invariants", "review_notes"],
        "additionalProperties": false
    })
}

pub fn ai_request_body(sources: &[(String, String)], d: &Draft, model: &str, effort: &str, max_tokens: u32) -> Value {
    let mut user = format!("<deterministic_draft>\n{}\n</deterministic_draft>\n\n", d.solidity);
    for (file, src) in sources {
        user.push_str(&format!("<source file=\"{file}\">\n{src}\n</source>\n\n"));
    }
    user.push_str("Extend the deterministic draft into DraftInvariants for this protocol.");
    json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": SYSTEM,
        "output_config": {"effort": effort, "format": {"type": "json_schema", "schema": ai_schema()}},
        "fallbacks": "default",
        "messages": [{"role": "user", "content": user}]
    })
}

/// Returns the structured JSON, or why there is none (refusal, truncation, no text).
pub fn parse_ai_response(v: &Value) -> Result<Value> {
    if let Some(e) = v.get("error") {
        return Err(eyre!("Claude API error: {}", e["message"].as_str().unwrap_or("unknown")));
    }
    match v["stop_reason"].as_str() {
        Some("refusal") => {
            return Err(eyre!(
                "the request was declined (stop_details category: {}); nothing was drafted",
                v["stop_details"]["category"].as_str().unwrap_or("unspecified")
            ))
        }
        Some("max_tokens") => return Err(eyre!("the draft was cut off at max_tokens; rerun with a larger --max-tokens")),
        _ => {}
    }
    let text = v["content"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|b| b["type"] == "text")
        .and_then(|b| b["text"].as_str())
        .ok_or_else(|| eyre!("no text block in the response"))?;
    let out: Value = serde_json::from_str(text).map_err(|e| eyre!("structured output did not parse: {e}"))?;
    for k in ["solidity", "invariants", "review_notes"] {
        if out.get(k).is_none() {
            return Err(eyre!("structured output lacks `{k}`"));
        }
    }
    Ok(out)
}

/// Calls the Messages API. Credentials come from `ANTHROPIC_API_KEY` (or an OAuth access token in
/// `ANTHROPIC_AUTH_TOKEN`) and are never printed.
pub fn ai_extend(sources: &[(String, String)], d: &Draft, model: &str, effort: &str, max_tokens: u32) -> Result<Value> {
    let base = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".into());
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(600)).build()?;
    let mut req = client
        .post(format!("{}/v1/messages", base.trim_end_matches('/')))
        .header("content-type", "application/json")
        .header("anthropic-version", API_VERSION);
    let mut betas = vec![FALLBACK_BETA];
    if let Ok(k) = std::env::var("ANTHROPIC_API_KEY").map(|k| k.trim().to_string()).and_then(|k| if k.is_empty() { Err(std::env::VarError::NotPresent) } else { Ok(k) }) {
        req = req.header("x-api-key", k);
    } else if let Ok(t) = std::env::var("ANTHROPIC_AUTH_TOKEN") {
        req = req.header("authorization", format!("Bearer {}", t.trim()));
        betas.push("oauth-2025-04-20");
    } else {
        return Err(eyre!("--ai needs Claude API credentials: set ANTHROPIC_API_KEY (or ANTHROPIC_AUTH_TOKEN)"));
    }
    let body = ai_request_body(sources, d, model, effort, max_tokens);
    let v: Value = req.header("anthropic-beta", betas.join(",")).json(&body).send()?.json()?;
    parse_ai_response(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acme() -> (Vec<Contract>, Vec<String>) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/evm/src/acme");
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
        files.sort();
        let mut contracts = Vec::new();
        let mut abstracts = BTreeMap::new();
        let mut names = Vec::new();
        for f in files.iter().filter(|f| f.extension().map(|e| e == "sol").unwrap_or(false)) {
            let src = std::fs::read_to_string(f).unwrap();
            let rel = format!("contracts/evm/src/acme/{}", f.file_name().unwrap().to_string_lossy());
            for a in scan_abstracts(&src, &rel) {
                abstracts.insert(a.name.clone(), a);
            }
            contracts.extend(scan(&src, &rel));
            names.push(rel);
        }
        resolve_bases(&mut contracts, &abstracts);
        (contracts, names)
    }

    #[test]
    fn comments_and_strings() {
        let s = strip_comments("a // x { \n b /* contract X { } */ c \"// not a comment\"");
        assert!(s.contains("a ") && s.contains(" b ") && s.contains(" c ") && s.contains("\"// not a comment\""));
        assert!(!s.contains("contract X"));
    }

    #[test]
    fn scan_finds_members_bases_and_fixed_vars() {
        let src = "interface I { function f() external; }\nabstract contract B { function paused() public view returns (bool) {} }\n\
                   contract V is B, I { uint256 public totalSupply; address public immutable admin; address public owner;\n\
                   function totalAssets() public view returns (uint256) { if (true) { return 1; } } }";
        let mut cs = scan(src, "v.sol");
        assert_eq!(cs.len(), 1, "interfaces and abstract contracts are not deployable");
        let abs: BTreeMap<String, Contract> = scan_abstracts(src, "v.sol").into_iter().map(|c| (c.name.clone(), c)).collect();
        resolve_bases(&mut cs, &abs);
        let v = &cs[0];
        assert_eq!(v.bases, vec!["B", "I"]);
        for m in ["totalSupply", "admin", "owner", "totalAssets", "paused"] {
            assert!(v.members.contains(m), "{m}");
        }
        assert!(v.fixed_vars.contains("admin") && !v.fixed_vars.contains("owner"));
    }

    #[test]
    fn acme_draft_rediscovers_the_share_price_rule() {
        let (cs, files) = acme();
        let d = draft(&cs, &files);
        let names: Vec<&str> = d.invariants.iter().map(|i| i.constant.as_str()).collect();
        assert_eq!(names, vec!["SHARE_PRICE_ACMEVAULT"], "Acme's admins are immutable: no role invariant");
        assert!(d.templates.iter().any(|(c, t)| c == "AcmeLend" && t.contains("Solvency")), "lending template for AcmeLend");
        assert!(d.solidity.contains("contract DraftInvariants") && d.slots_toml.contains("share_price_acmevault"));
        let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/evm/test/drafts/DraftInvariants.sol");
        let want = std::fs::read_to_string(&golden).unwrap_or_default().replace("\r\n", "\n");
        assert_eq!(d.solidity, want, "regenerate with `antibody invariants draft --source contracts/evm/src/acme --out target/acme-draft` and copy DraftInvariants.sol to contracts/evm/test/drafts/");
    }

    #[test]
    fn ai_request_and_response_shapes() {
        let (cs, files) = acme();
        let d = draft(&cs, &files);
        let body = ai_request_body(&[("a.sol".into(), "contract A {}".into())], &d, AI_MODEL, "high", 16000);
        assert_eq!(body["model"], AI_MODEL);
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["output_config"]["format"]["schema"]["additionalProperties"], false);
        assert!(body.get("thinking").is_none(), "Opus 5.5 always thinks; budgets are rejected");
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("<source file=\"a.sol\">"));
        let ok = json!({"stop_reason": "end_turn", "content": [{"type": "thinking", "thinking": ""},
            {"type": "text", "text": "{\"solidity\":\"contract DraftInvariants {}\",\"invariants\":[],\"review_notes\":[]}"}]});
        assert_eq!(parse_ai_response(&ok).unwrap()["solidity"], "contract DraftInvariants {}");
        let refused = json!({"stop_reason": "refusal", "stop_details": {"category": "cyber"}, "content": []});
        assert!(parse_ai_response(&refused).unwrap_err().to_string().contains("declined"));
        let cut = json!({"stop_reason": "max_tokens", "content": [{"type": "text", "text": "{\"solid"}]});
        assert!(parse_ai_response(&cut).is_err());
    }
}

/// `antibody invariants draft`: scans the sources (files or directories), writes the draft, and
/// with `ai` also Claude's extension of it. Returns a summary.
pub fn run(sources: &[std::path::PathBuf], out: &std::path::Path, ai: bool, model: &str, effort: &str, max_tokens: u32) -> Result<Value> {
    let mut files = Vec::new();
    for s in sources {
        if s.is_dir() {
            let mut stack = vec![s.clone()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d)? {
                    let p = e?.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().map(|e| e == "sol").unwrap_or(false) {
                        files.push(p);
                    }
                }
            }
        } else {
            files.push(s.clone());
        }
    }
    files.sort();
    if files.is_empty() {
        return Err(eyre!("no .sol files under the given --source paths"));
    }
    let mut contracts = Vec::new();
    let mut abstracts = BTreeMap::new();
    let mut names = Vec::new();
    let mut texts = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f)?;
        let rel = f.to_string_lossy().replace('\\', "/");
        for a in scan_abstracts(&src, &rel) {
            abstracts.insert(a.name.clone(), a);
        }
        contracts.extend(scan(&src, &rel));
        names.push(rel.clone());
        texts.push((rel, src));
    }
    resolve_bases(&mut contracts, &abstracts);
    let d = draft(&contracts, &names);
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("DraftInvariants.sol"), &d.solidity)?;
    std::fs::write(out.join("slots.toml"), &d.slots_toml)?;
    std::fs::write(out.join("noop.bundle.json"), serde_json::to_vec_pretty(&noop_bundle())?)?;
    let mut review = d.review.clone();
    let mut summary = json!({
        "out": out, "sources": names, "contracts": contracts.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        "drafted": d.invariants.iter().map(|i| json!({"bit": i.bit, "name": i.slot_name, "severity": i.severity, "property": i.property})).collect::<Vec<_>>(),
        "templates": d.templates.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>(),
        "files": ["DraftInvariants.sol", "slots.toml", "REVIEW.md", "noop.bundle.json"],
    });
    if ai {
        let v = ai_extend(&texts, &d, model, effort, max_tokens)?;
        std::fs::write(out.join("DraftInvariants.ai.sol"), v["solidity"].as_str().unwrap_or_default())?;
        std::fs::write(out.join("invariants.ai.json"), serde_json::to_vec_pretty(&v)?)?;
        review.push_str(&format!(
            "\n## AI draft ({model}): `DraftInvariants.ai.sol`, not reviewed\n\n\
             | Bit | Severity | Property | Holds because | Assumptions |\n|---|---|---|---|---|\n"
        ));
        for i in v["invariants"].as_array().into_iter().flatten() {
            let s = |k: &str| i[k].as_str().unwrap_or_default().to_string();
            review.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                i["bit"],
                s("severity"),
                s("property"),
                s("holds_because"),
                s("assumptions")
            ));
        }
        review.push_str("\nQuestions for the project:\n");
        for n in v["review_notes"].as_array().into_iter().flatten() {
            review.push_str(&format!("- {}\n", n.as_str().unwrap_or_default()));
        }
        summary["ai"] = json!({"model": model, "invariants": v["invariants"].as_array().map(|a| a.len()), "files": ["DraftInvariants.ai.sol", "invariants.ai.json"]});
    }
    std::fs::write(out.join("REVIEW.md"), review)?;
    Ok(summary)
}

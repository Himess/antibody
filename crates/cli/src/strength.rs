//! Rule strength (docs/rule-strength.md): how many deliberately broken versions of a program's code its
//! rules catch. Each mutant changes one line of the protocol's sources in one labeled way:
//!
//! - `missing-access-check`: a caller check is deleted (an `only…` modifier, or a `require`/`if … revert`
//!   on `msg.sender`);
//! - `flipped-comparison`: a comparison in a condition is negated (`<` becomes `>=`, `==` becomes `!=`);
//! - `unchecked-transfer`: a token transfer's (or a low-level call's) success check is dropped.
//!
//! The project's own Foundry invariant test, which asserts the program's rules (`check(snapshot()) == 0`),
//! runs on every mutant with a fixed seed. A mutant is **caught** when the fuzzer breaks a rule within the
//! budget. The score, "caught X of N mutants", measures the rules and the test's reach, not the code's
//! security; a mutant that changes no behaviour cannot be caught, and the survivors are listed.

use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use eyre::{eyre, Result};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize)]
pub struct Mutant {
    pub id: usize,
    pub class: &'static str,
    pub file: String,
    pub line: usize,
    pub before: String,
    pub after: String,
}

/// A line's code with string literals and `//` comments blanked (one space per byte, so byte offsets
/// stay those of the line), so operators inside them are never mutated.
fn code_of(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_str: Option<char> = None;
    let b: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let blank = |out: &mut String, c: char| out.extend(std::iter::repeat(' ').take(c.len_utf8()));
        match in_str {
            Some(q) => {
                blank(&mut out, c);
                if c == '\\' && i + 1 < b.len() {
                    blank(&mut out, b[i + 1]);
                    i += 1;
                } else if c == q {
                    in_str = None;
                }
            }
            None if c == '"' || c == '\'' => {
                in_str = Some(c);
                blank(&mut out, c);
            }
            None if c == '/' && b.get(i + 1) == Some(&'/') => {
                for &r in &b[i..] {
                    blank(&mut out, r);
                }
                break;
            }
            None => out.push(c),
        }
        i += 1;
    }
    out
}

const NEGATED: [(&str, &str); 6] = [("<=", ">"), (">=", "<"), ("==", "!="), ("!=", "=="), ("<", ">="), (">", "<=")];

/// Every comparison operator in a condition, with its negation: (byte offset, operator, negation).
fn comparisons(code: &str) -> Vec<(usize, &'static str, &'static str)> {
    let trimmed = code.trim_start();
    let condition = trimmed.starts_with("if (")
        || trimmed.starts_with("} else if (")
        || trimmed.starts_with("require(")
        || trimmed.starts_with("while (")
        || trimmed.starts_with("return ")
        || code.contains(" ? ");
    if !condition {
        return vec![];
    }
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let two = code.get(i..i + 2).unwrap_or("");
        // Shifts, arrows and mappings are not comparisons.
        if two == "<<" || two == ">>" || two == "=>" || two == "->" {
            i += 2;
            continue;
        }
        if let Some((op, neg)) = NEGATED.iter().find(|(op, _)| code[i..].starts_with(op)) {
            // A lone `<`/`>` must not be part of a longer operator (`<=`, `>=`, `<<`, `>>`).
            let single = op.len() == 1;
            let next = b.get(i + 1).copied();
            let prev = if i > 0 { b.get(i - 1).copied() } else { None };
            if !(single && (next == Some(b'=') || next == Some(b'<') || next == Some(b'>') || prev == Some(b'<') || prev == Some(b'>') || prev == Some(b'='))) {
                out.push((i, *op, *neg));
            }
            i += op.len();
            continue;
        }
        i += 1;
    }
    out
}

fn indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// The call inside `if (!X.transfer(...)) revert …;` or `require(X.transfer(...), …);`, if the line is one.
fn checked_transfer(code: &str, line: &str) -> Option<String> {
    let start = code.len() - code.trim_start().len();
    let (t, l) = (&code[start..], &line[start..]);
    let is_transfer = |s: &str| s.contains(".transfer(") || s.contains(".transferFrom(") || s.contains(".safeTransfer");
    if let Some(rest) = t.strip_prefix("if (!") {
        let end = rest.find(")) revert")?;
        let call = &l["if (!".len().."if (!".len() + end + 1];
        return is_transfer(&rest[..end + 1]).then(|| format!("{}{call};", indent(line)));
    }
    if let Some(rest) = t.strip_prefix("require(") {
        // `require(X.transfer(a, b), "msg");`: the call ends at the comma that closes its arguments.
        let mut depth = 0i32;
        for (k, c) in rest.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    let call = &l["require(".len().."require(".len() + k];
                    return is_transfer(&rest[..k]).then(|| format!("{}{call};", indent(line)));
                }
                _ => {}
            }
        }
    }
    None
}

/// Whether a statement checks the caller: a `require`/`if … revert` that reads `msg.sender`.
fn caller_check(code: &str) -> bool {
    let t = code.trim();
    t.contains("msg.sender") && ((t.starts_with("if (") && t.contains(") revert") && t.ends_with(';')) || (t.starts_with("require(") && t.ends_with(");")))
}

/// Whether a statement checks a low-level call's success (`require(ok…)`, `if (!ok) revert …`).
fn call_success_check(code: &str, prev: &[String]) -> bool {
    let t = code.trim();
    let checks_ok = t.starts_with("require(ok") || t.starts_with("if (!ok)") || t.starts_with("require(success") || t.starts_with("if (!success)");
    checks_ok && prev.iter().rev().take(3).any(|p| p.contains(".call(") || p.contains(".call{"))
}

/// The mutants of one source file: one changed line each, in a fixed order.
pub fn mutate(file: &str, src: &str, first_id: usize) -> Vec<(Mutant, String)> {
    let lines: Vec<&str> = src.split('\n').collect();
    let codes: Vec<String> = lines.iter().map(|l| code_of(l)).collect();
    let mut out = Vec::new();
    let mut in_block_comment = false;
    let mut in_header = false;
    let mut push = |class: &'static str, i: usize, after: String| {
        let mut m = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        let before = m[i].clone();
        m[i] = after.clone();
        let id = first_id + out.len();
        out.push((Mutant { id, class, file: file.to_string(), line: i + 1, before: before.trim().to_string(), after: after.trim().to_string() }, m.join("\n")));
    };
    for (i, line) in lines.iter().enumerate() {
        let code = &codes[i];
        let t = code.trim();
        if in_block_comment {
            if t.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if t.starts_with("/*") || t.starts_with('*') {
            in_block_comment = !t.contains("*/");
            continue;
        }
        if t.starts_with("function ") || t.starts_with("modifier ") {
            in_header = true;
        }
        if in_header {
            // `only…` modifiers in a function header.
            let mut k = 0;
            while let Some(p) = code[k..].find("only") {
                let at = k + p;
                let starts_word = at == 0 || !code.as_bytes()[at - 1].is_ascii_alphanumeric() && code.as_bytes()[at - 1] != b'_';
                let upper = code.as_bytes().get(at + 4).is_some_and(|c| c.is_ascii_uppercase());
                if starts_word && upper && !t.starts_with("modifier ") {
                    let mut end = at + 4;
                    while end < code.len() && (code.as_bytes()[end].is_ascii_alphanumeric() || code.as_bytes()[end] == b'_') {
                        end += 1;
                    }
                    if code.as_bytes().get(end) == Some(&b'(') {
                        end += code[end..].find(')').map(|x| x + 1).unwrap_or(0);
                    }
                    let from = if at > 0 && code.as_bytes()[at - 1] == b' ' { at - 1 } else { at };
                    let mut after = line.to_string();
                    after.replace_range(from..end, "");
                    push("missing-access-check", i, after);
                }
                k = at + 4;
            }
            if t.contains('{') || t.ends_with(';') {
                in_header = false;
            }
            continue;
        }
        if caller_check(code) {
            push("missing-access-check", i, String::new());
            continue;
        }
        if let Some(after) = checked_transfer(code, line) {
            push("unchecked-transfer", i, after);
        } else if call_success_check(code, &codes[..i]) {
            push("unchecked-transfer", i, String::new());
        }
        if t.starts_with("for (") {
            continue;
        }
        for (at, op, neg) in comparisons(code) {
            let mut after = line.to_string();
            after.replace_range(at..at + op.len(), neg);
            push("flipped-comparison", i, after);
        }
    }
    out
}

pub struct Opts {
    pub project: PathBuf,
    pub test: String,
    pub files: Vec<String>,
    pub runs: u32,
    pub depth: u32,
    pub seed: u64,
    pub forge: PathBuf,
    pub timeout: Duration,
    pub workers: usize,
    pub work: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct Outcome {
    #[serde(flatten)]
    mutant: Mutant,
    result: &'static str,
    detail: String,
    seconds: f64,
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        let n = name.to_string_lossy();
        // Build output, caches and broadcasts are the copy's own.
        if ["out", "cache", "broadcast", "deployments", "node_modules"].contains(&n.as_ref()) {
            continue;
        }
        let (src, dst) = (e.path(), to.join(&name));
        if e.file_type()?.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// One `forge test` of the strength test in `dir`: (result, detail).
fn forge_test(o: &Opts, dir: &Path) -> Result<(&'static str, String)> {
    let mut child = Command::new(&o.forge)
        .current_dir(dir)
        .args(["test", "--match-contract", &o.test, "--fuzz-seed", &o.seed.to_string()])
        .env("FOUNDRY_INVARIANT_RUNS", o.runs.to_string())
        .env("FOUNDRY_INVARIANT_DEPTH", o.depth.to_string())
        .env("FOUNDRY_INVARIANT_FAIL_ON_REVERT", "false")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let t0 = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if t0.elapsed() > o.timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(("timeout", format!("over {} s", o.timeout.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let out = child.wait_with_output()?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if text.contains("Compiler run failed") || text.contains("Error (") && !text.contains("[FAIL") {
        let first = text.lines().find(|l| l.contains("Error")).unwrap_or("compile error").trim().to_string();
        return Ok(("invalid", first));
    }
    if let Some(l) = text.lines().find(|l| l.contains("[FAIL")) {
        let detail: String = l.trim().chars().take(240).collect();
        if l.contains("a program rule breaks") {
            return Ok(("caught", detail));
        }
        return Ok(("errored", detail));
    }
    if text.lines().any(|l| l.contains("[PASS]")) {
        return Ok(("survived", String::new()));
    }
    Ok(("invalid", text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no result").trim().chars().take(240).collect()))
}

/// Runs the strength test on every mutant of `files` (and once on the unmutated code, which must pass).
pub fn run(o: Opts) -> Result<Value> {
    let t0 = Instant::now();
    let mut mutants = Vec::new();
    let mut sources = Vec::new();
    for f in &o.files {
        let src = std::fs::read_to_string(o.project.join(f)).map_err(|e| eyre!("{f}: {e}"))?;
        sources.push(json!({"file": f, "sha256": hex::encode(antibody_verifier::facts::sha256(src.as_bytes()))}));
        let next = mutants.len();
        mutants.extend(mutate(f, &src, next));
    }
    let _ = std::fs::remove_dir_all(&o.work);
    let workers: Vec<PathBuf> = (0..o.workers.max(1)).map(|k| o.work.join(format!("w{k}"))).collect();
    for w in &workers {
        copy_dir(&o.project, w)?;
    }
    let (base, detail) = forge_test(&o, &workers[0])?;
    if base != "survived" {
        return Err(eyre!("the strength test must pass on the unmutated code: {base} {detail}"));
    }
    let queue = Arc::new(Mutex::new(mutants.into_iter().collect::<VecDeque<_>>()));
    let results = Arc::new(Mutex::new(Vec::new()));
    let o = Arc::new(o);
    let handles: Vec<_> = workers
        .into_iter()
        .map(|dir| {
            let (queue, results, o) = (queue.clone(), results.clone(), o.clone());
            std::thread::spawn(move || -> Result<()> {
                loop {
                    let Some((m, src)) = queue.lock().unwrap().pop_front() else { return Ok(()) };
                    let path = dir.join(&m.file);
                    let original = std::fs::read_to_string(&path)?;
                    std::fs::write(&path, &src)?;
                    let t = Instant::now();
                    let r = forge_test(&o, &dir);
                    std::fs::write(&path, &original)?;
                    let (result, detail) = r?;
                    results.lock().unwrap().push(Outcome { mutant: m, result, detail, seconds: t.elapsed().as_secs_f64() });
                }
            })
        })
        .collect();
    for h in handles {
        h.join().map_err(|_| eyre!("a worker panicked"))??;
    }
    let mut results = Arc::try_unwrap(results).map_err(|_| eyre!("results still shared"))?.into_inner().unwrap();
    results.sort_by_key(|r| r.mutant.id);
    let mut by_class: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    for r in &results {
        if r.result == "caught" || r.result == "survived" {
            let e = by_class.entry(r.mutant.class).or_default();
            e.1 += 1;
            if r.result == "caught" {
                e.0 += 1;
            }
        }
    }
    let (caught, total) = by_class.values().fold((0, 0), |(c, t), (a, b)| (c + a, t + b));
    let count = |k: &str| results.iter().filter(|r| r.result == k).count();
    Ok(json!({
        "statement": format!("caught {caught} of {total} mutants"),
        "label": "rule strength: how many deliberately broken versions of the code the program's rules catch under the fuzzer; it measures the rules, not the code's security",
        "caught": caught, "total": total, "invalid": count("invalid"), "errored": count("errored"), "timeout": count("timeout"),
        "by_class": by_class.iter().map(|(k, (c, t))| (k.to_string(), json!({"caught": c, "total": t}))).collect::<BTreeMap<_, _>>(),
        "test": o.test, "sources": sources, "fuzz": {"runs": o.runs, "depth": o.depth, "seed": o.seed},
        "survivors": results.iter().filter(|r| r.result == "survived").map(|r| json!({"id": r.mutant.id, "class": r.mutant.class, "file": r.mutant.file, "line": r.mutant.line, "before": r.mutant.before, "after": r.mutant.after})).collect::<Vec<_>>(),
        "mutants": results, "seconds": t0.elapsed().as_secs_f64(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(src: &str) -> Vec<(&'static str, String)> {
        mutate("x.sol", src, 0).into_iter().map(|(m, _)| (m.class, m.after)).collect()
    }

    #[test]
    fn a_caller_check_is_deleted() {
        let m = classes("    function f() external {\n        if (msg.sender != admin) revert NotAdmin();\n        x = 1;\n    }\n");
        assert_eq!(m, vec![("missing-access-check", String::new())]);
        let r = classes("        require(msg.sender == admin, \"not admin\");\n");
        assert_eq!(r[0], ("missing-access-check", String::new()));
    }

    #[test]
    fn an_only_modifier_is_removed() {
        let m = classes("    function set(uint256 v) external onlyOwner {\n        x = v;\n    }\n");
        assert_eq!(m, vec![("missing-access-check", "function set(uint256 v) external {".to_string())]);
    }

    #[test]
    fn a_transfer_check_is_dropped() {
        let m = classes("        if (!acUSD.transfer(to, amount)) revert TransferFailed();\n");
        assert_eq!(m, vec![("unchecked-transfer", "acUSD.transfer(to, amount);".to_string())]);
        let r = classes("        require(token.transferFrom(a, b, c), \"tf\");\n");
        assert_eq!(r, vec![("unchecked-transfer", "token.transferFrom(a, b, c);".to_string())]);
        let c = classes("        (bool ok, ) = to.call(data);\n        require(ok, \"call failed\");\n");
        assert_eq!(c, vec![("unchecked-transfer", String::new())]);
    }

    #[test]
    fn comparisons_in_conditions_are_negated_once_each() {
        let m = classes("        if (amountOut == 0 || amountOut < minOut) revert Slippage();\n");
        assert_eq!(m, vec![
            ("flipped-comparison", "if (amountOut != 0 || amountOut < minOut) revert Slippage();".to_string()),
            ("flipped-comparison", "if (amountOut == 0 || amountOut >= minOut) revert Slippage();".to_string()),
        ]);
    }

    #[test]
    fn strings_comments_loops_shifts_and_mappings_are_left_alone() {
        assert!(classes("        require(x, \"a < b\"); // c == d\n").is_empty());
        assert!(classes("        for (uint256 i; i < n; ++i) {\n").is_empty());
        assert!(classes("        if (x << 2 > y) revert E();\n").len() == 1);
        assert!(classes("    mapping(address => uint256) public balanceOf;\n").is_empty());
    }
}

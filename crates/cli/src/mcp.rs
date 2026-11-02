//! `antibody mcp`: a Model Context Protocol server (JSON-RPC 2.0 over stdio, one message per line)
//! exposing the CLI library to AI agents. Agents hunt with their own tools; this server only lets
//! them read programs, check a PoC locally, commit/submit sealed PoCs and verify receipts. Run it
//! with a function-call access key limited to commit/submit (`antibody agent-key add`).

use std::{
    io::{BufRead, Write},
    path::PathBuf,
};

use alloy_primitives::B256;
use eyre::{eyre, Result};
use serde_json::{json, Value};

use crate::{cmd, ctx::Ctx};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

fn tools() -> Value {
    let int = |d: &str| json!({"type": "integer", "minimum": 0, "description": d});
    let s = |d: &str| json!({"type": "string", "description": d});
    json!([
        {"name": "list_programs", "description": "List bounty programs on near-antibody (escrow, anchor, status) and global stats.",
         "inputSchema": {"type": "object", "properties": {"from": int("first program id"), "limit": int("max programs")}}},
        {"name": "get_program", "description": "One program: committed rules (invariants, severities, payouts, bond, windows), anchor and its light-client status, claimed keys and the sealed submissions feed.",
         "inputSchema": {"type": "object", "properties": {"program_id": int("program id")}, "required": ["program_id"]}},
        {"name": "fetch_env", "description": "Write a hunting workspace for a program: the hash-checked manifest, invariants code, targets, a bundle template and the anvil fork command for the anchor block.",
         "inputSchema": {"type": "object", "properties": {"program_id": int("program id"), "out_dir": s("directory to write")}, "required": ["program_id"]}},
        {"name": "check_poc", "description": "Run the verifier locally on a PoC bundle against the proven anchor state (no keys, nothing sent). Returns the verdict: which invariants break, where, and the impact.",
         "inputSchema": {"type": "object", "properties": {"program_id": int("program id"), "bundle_path": s("bundle JSON file"), "salt": s("optional 0x 32-byte sender salt")}, "required": ["program_id", "bundle_path"]}},
        {"name": "commit", "description": "Seal the PoC to the TEE's key and commit its hash on NEAR (sets priority, locks the bond). The blob stays private until `submit`.",
         "inputSchema": {"type": "object", "properties": {"program_id": int("program id"), "bundle_path": s("bundle JSON file"), "report_path": s("optional report (never read by the verifier)"), "salt": s("optional 0x salt")}, "required": ["program_id", "bundle_path"]}},
        {"name": "submit", "description": "Reveal a committed PoC (commit_id), or commit+reveal a bundle in one step; the contract has OutLayer verify it and applies the committed rules. Waits for the verdict.",
         "inputSchema": {"type": "object", "properties": {"program_id": int("program id"), "commit_id": int("an existing commit to reveal"), "bundle_path": s("bundle JSON (commit + reveal)"), "report_path": s("optional report"), "wait": {"type": "boolean"}}, "required": ["program_id"]}},
        {"name": "status", "description": "A commit (verdict, outcome, duplicate proof), a program summary, and/or an account's balances and reputation.",
         "inputSchema": {"type": "object", "properties": {"commit_id": int("commit id"), "program_id": int("program id"), "account": s("NEAR account")}}},
        {"name": "verify_receipt", "description": "Re-run a commit's verification locally from its sealed blob (after disclosure, or with your own local record) and compare receipt hash and fingerprint tags with the chain; for a duplicate also re-verify the earlier claim.",
         "inputSchema": {"type": "object", "properties": {"commit_id": int("commit id")}, "required": ["commit_id"]}}
    ])
}

fn arg_u64(a: &Value, k: &str) -> Result<u64> {
    a[k].as_u64().ok_or_else(|| eyre!("missing integer argument {k}"))
}

fn salt(a: &Value) -> Result<Option<B256>> {
    a["salt"].as_str().map(|s| s.parse::<B256>().map_err(|e| eyre!("salt: {e}"))).transpose()
}

fn call(ctx: &Ctx, name: &str, a: &Value) -> Result<Value> {
    match name {
        "list_programs" => cmd::programs(ctx, a["from"].as_u64().unwrap_or(0) as u32, a["limit"].as_u64().unwrap_or(20) as u32),
        "get_program" => cmd::program_show(ctx, arg_u64(a, "program_id")? as u32),
        "fetch_env" => {
            let p = arg_u64(a, "program_id")? as u32;
            let out = a["out_dir"].as_str().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("antibody-program-{p}")));
            cmd::pull(ctx, p, &out)
        }
        "check_poc" => {
            let bundle = PathBuf::from(a["bundle_path"].as_str().ok_or_else(|| eyre!("bundle_path"))?);
            cmd::check(ctx, Some(arg_u64(a, "program_id")? as u32), None, &bundle, salt(a)?)
        }
        "commit" => {
            let bundle = PathBuf::from(a["bundle_path"].as_str().ok_or_else(|| eyre!("bundle_path"))?);
            let report = a["report_path"].as_str().map(PathBuf::from);
            cmd::commit(ctx, arg_u64(a, "program_id")? as u32, &bundle, salt(a)?, report.as_deref())
        }
        "submit" => {
            let p = arg_u64(a, "program_id")? as u32;
            let wait = a["wait"].as_bool().unwrap_or(true);
            match a["commit_id"].as_u64() {
                Some(c) => cmd::reveal(ctx, p, c, wait),
                None => {
                    let bundle = PathBuf::from(a["bundle_path"].as_str().ok_or_else(|| eyre!("bundle_path or commit_id"))?);
                    let report = a["report_path"].as_str().map(PathBuf::from);
                    cmd::submit(ctx, p, &bundle, salt(a)?, report.as_deref(), wait)
                }
            }
        }
        "status" => cmd::status(ctx, a["commit_id"].as_u64(), a["program_id"].as_u64().map(|x| x as u32), a["account"].as_str().map(String::from)),
        "verify_receipt" => cmd::verify(ctx, arg_u64(a, "commit_id")?),
        other => Err(eyre!("unknown tool {other}")),
    }
}

fn respond(out: &mut impl Write, id: &Value, result: Value) -> Result<()> {
    writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result}))?;
    out.flush()?;
    Ok(())
}

fn respond_error(out: &mut impl Write, id: &Value, code: i64, msg: &str) -> Result<()> {
    writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}}))?;
    out.flush()?;
    Ok(())
}

/// Serves until stdin closes.
pub fn serve(ctx: &Ctx) -> Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                respond_error(&mut out, &Value::Null, -32700, &format!("parse error: {e}"))?;
                continue;
            }
        };
        let id = msg["id"].clone();
        let method = msg["method"].as_str().unwrap_or_default();
        if id.is_null() {
            continue; // notifications (e.g. notifications/initialized) need no answer
        }
        match method {
            "initialize" => {
                let pv = msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION).to_string();
                respond(&mut out, &id, json!({
                    "protocolVersion": pv,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "antibody", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "Antibody verifies and pays; it does not hunt. Use your own tools to find a bug in a program's targets, express it as a PoC bundle, check it locally with check_poc, then commit and submit. Validity is a broken committed invariant; text is never read."
                }))?;
            }
            "ping" => respond(&mut out, &id, json!({}))?,
            "tools/list" => respond(&mut out, &id, json!({"tools": tools()}))?,
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or_default();
                let args = msg["params"]["arguments"].clone();
                let result = match call(ctx, name, &args) {
                    Ok(v) => json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&v)?}], "isError": false}),
                    Err(e) => json!({"content": [{"type": "text", "text": format!("{e:#}")}], "isError": true}),
                };
                respond(&mut out, &id, result)?;
            }
            other => respond_error(&mut out, &id, -32601, &format!("method not found: {other}"))?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schemas_are_valid_objects() {
        for t in tools().as_array().unwrap() {
            assert!(t["name"].is_string() && t["description"].is_string());
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }
}

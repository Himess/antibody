//! Progress lines on stderr for the long commands (`try`, `hunt start|submit`, `judge`): what is happening
//! and since when, so a wait for an RPC, the enclave or NEAR's clock never looks like a hang. Off unless the
//! CLI turns it on: the MCP server keeps stderr quiet, and stdout stays the command's answer alone.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();
/// The MCP tool call in progress, when its request carried a progressToken: the token, the call's start
/// and the steps sent so far.
static MCP: Mutex<Option<(serde_json::Value, Instant, u64)>> = Mutex::new(None);

/// While `token` is set, steps go to the MCP client as `notifications/progress` (stdout, the protocol's own
/// channel), and stderr stays quiet; `None` ends it.
pub fn mcp_sink(token: Option<serde_json::Value>) {
    if let Ok(mut m) = MCP.lock() {
        *m = token.map(|t| (t, Instant::now(), 0));
    }
}

/// Turns the progress lines on, counting from now.
pub fn enable() {
    let _ = START.set(Instant::now());
    ON.store(true, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// One line: the seconds since the command started, then `msg`. stderr is unbuffered, so the line shows at
/// once, piped or not.
pub fn step(msg: impl AsRef<str>) {
    if let Ok(mut m) = MCP.lock() {
        if let Some((token, start, n)) = m.as_mut() {
            *n += 1;
            let note = serde_json::json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {
                "progressToken": token, "progress": *n,
                "message": format!("[{:.1} s] {}", start.elapsed().as_secs_f64(), msg.as_ref())}});
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{}", crate::mcp::ascii_json(&note));
            let _ = out.flush();
            return;
        }
    }
    if enabled() {
        let t = START.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        eprintln!("  [{t:6.1} s] {}", msg.as_ref());
    }
}

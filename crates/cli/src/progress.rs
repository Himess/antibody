//! Progress lines on stderr for the long commands (`try`, `hunt start|submit`, `judge`): what is happening
//! and since when, so a wait for an RPC, the enclave or NEAR's clock never looks like a hang. Off unless the
//! CLI turns it on: the MCP server keeps stderr quiet, and stdout stays the command's answer alone.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();

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
    if enabled() {
        let t = START.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        eprintln!("  [{t:6.1} s] {}", msg.as_ref());
    }
}

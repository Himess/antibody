//! NEP-297 events: `EVENT_JSON:{"standard":"antibody","version":"1.0.0","event":…,"data":[…]}`.
//! The indexer (services/) reconstructs the full history from these.

use near_sdk::{env, serde_json::Value};

pub const STANDARD: &str = "antibody";
pub const VERSION: &str = "1.0.0";

pub fn emit(event: &str, data: Value) {
    let ev = near_sdk::serde_json::json!({
        "standard": STANDARD,
        "version": VERSION,
        "event": event,
        "data": [data],
    });
    env::log_str(&format!("EVENT_JSON:{ev}"));
}

//! NEP-297 events: `EVENT_JSON:{"standard":"antibody","version":"2.0.0","event":…,"data":[…]}`.
//! The indexer (services/) reconstructs the full history from these.

use near_sdk::{env, serde_json::Value};

pub const STANDARD: &str = "antibody";
pub const VERSION: &str = "2.0.0";

pub fn emit(event: &str, data: Value) {
    emit_raw(event, &data.to_string());
}

/// An event whose data is already JSON (the rule core's effects carry it as text).
pub fn emit_raw(event: &str, data: &str) {
    env::log_str(&format!(r#"EVENT_JSON:{{"standard":"{STANDARD}","version":"{VERSION}","event":"{event}","data":[{data}]}}"#));
}

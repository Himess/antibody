//! The `antibody` client library: NEAR client, sealing and blob storage, command implementations
//! and the MCP server. The CLI binary and the backend services are thin layers over it.

pub mod blob;
pub mod cmd;
pub mod compat;
pub mod ctx;
pub mod draft;
pub mod hunt;
pub mod immune;
pub mod judge;
pub mod mcp;
pub mod near;
pub mod payable;
pub mod pr_check;
pub mod practice;
pub mod progress;
pub mod strength;
pub mod tee;
pub mod window;
pub mod zk;

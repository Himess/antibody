//! The `antibody` client library: NEAR client, sealing and blob storage, command implementations
//! and the MCP server. The CLI binary and the backend services are thin layers over it.

pub mod blob;
pub mod cmd;
pub mod ctx;
pub mod draft;
pub mod mcp;
pub mod near;
pub mod tee;

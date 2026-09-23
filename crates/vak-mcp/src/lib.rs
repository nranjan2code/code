//! vak-mcp: Model Context Protocol client over stdio (newline-delimited
//! JSON-RPC 2.0). Servers spawn lazily on first use; tool descriptions are
//! fetched on demand so context stays lean.

pub mod client;
pub mod manager;
pub mod tool;

pub use client::{
    McpClient, McpError, McpNotification, McpToolInfo, NotificationSink, ServerConfig,
};
pub use manager::{IDLE_TTL, LIST_TIMEOUT, ListOutcome, McpManager, ServerObservation};
pub use tool::McpTool;

mod validate;

//! starlist: a log collector that distills an application's logs for AI agents over MCP.

pub mod alerts;
pub mod analysis;
pub mod api;
pub mod collector;
pub mod http;
pub mod location;
pub mod merge;
pub mod pattern;
pub mod redact;
pub mod request;
pub mod server;
pub mod store;

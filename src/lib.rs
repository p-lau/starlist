//! starlist: a log collector that distills an application's logs for AI agents over MCP.

pub mod alerts;
pub mod analysis;
pub mod api;
pub mod collector;
pub mod config;
#[cfg(feature = "dashboard")]
pub mod dashboard;
pub mod diagnose;
pub mod export;
pub mod http;
#[cfg(feature = "hub")]
pub mod hub;
pub mod init;
pub mod issues;
pub mod metrics;
#[cfg(feature = "prometheus")]
pub mod prometheus;
pub mod push;
pub mod server;
pub mod sources;
pub mod store;
pub mod timing;

// The log engine lives in starlist-core; re-exported so `starlist::pattern`
// and friends keep working.
pub use starlist_core::{location, merge, pattern, redact, request};

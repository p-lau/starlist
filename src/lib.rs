//! starlist: a log collector that distills an application's logs for AI agents over MCP.

pub mod alerts;
pub mod analysis;
pub mod api;
pub mod collector;
pub mod dashboard;
pub mod diagnose;
pub mod export;
pub mod http;
pub mod hub;
pub mod issues;
pub mod location;
pub mod merge;
pub mod metrics;
pub mod pattern;
pub mod prometheus;
pub mod push;
pub mod redact;
pub mod request;
pub mod server;
pub mod store;

//! starlist-core: the log engine shared by the starlist binaries.
//!
//! Turns raw log text into redacted events with a stable pattern ID, merges
//! patterns that differ in one word, and finds request IDs and the app's own
//! code location. No storage, async runtime or network: the collector adds
//! those on top.

pub mod location;
pub mod merge;
pub mod pattern;
pub mod redact;
pub mod request;

//! starlist-ci: what a failing CI run logged that the last passing run didn't.
//!
//! Both logs go through the starlist engine (`starlist-core`): lines are
//! grouped into events, redacted and clustered into patterns, so the two runs
//! can be compared pattern by pattern. Everything stays in memory; nothing is
//! written to disk.

pub mod clean;
pub mod ingest;
pub mod report;

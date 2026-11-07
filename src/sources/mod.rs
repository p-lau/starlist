//! Log sources beyond files and stdin: Docker, journald, Kubernetes, OTLP and
//! Loki. Each one is a Cargo feature, and every source feeds the same
//! `Pipeline`, so redaction, clustering, markers and request IDs work the same
//! as for a tailed file.
//!
//! Docker, journald and Kubernetes run the platform's own command-line tool
//! (`docker`, `journalctl`, `kubectl`) and read its output, which keeps the
//! binary small and uses whatever access and context the user already has.

#[cfg(feature = "docker")]
pub mod docker;
#[cfg(feature = "journald")]
pub mod journald;
#[cfg(feature = "kubernetes")]
pub mod kubernetes;
#[cfg(feature = "loki")]
pub mod loki;
#[cfg(feature = "otel")]
pub mod otel;

#[cfg(any(feature = "docker", feature = "journald", feature = "kubernetes"))]
pub mod command;

/// Where a follower starts reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Start {
    /// Only lines written from now on.
    New,
    /// Everything the source still has.
    All,
    /// From this time (Unix nanoseconds), skipping lines already read.
    Since(i64),
}

/// RFC 3339 with nanoseconds, as Docker and kubectl accept it.
pub fn rfc3339(ns: i64) -> String {
    chrono::DateTime::from_timestamp_nanos(ns).to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

/// Split a `--timestamps` prefix (`2026-11-07T01:02:03.123456789Z line`) off a
/// line from `docker logs` or `kubectl logs`, as Unix nanoseconds.
pub fn split_timestamp(line: &str) -> Option<(i64, &str)> {
    let (ts, rest) = line.split_once(' ').unwrap_or((line, ""));
    let t = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
    Some((t.timestamp_nanos_opt()?, rest))
}

/// Skips lines already read when a follower resumes from a time: everything
/// before the newest time seen, and lines seen at exactly that time.
#[derive(Default)]
pub struct Resume {
    /// Unix nanoseconds.
    pub last: Option<i64>,
    at_last: std::collections::HashSet<u64>,
}

impl Resume {
    /// True when this line is new.
    pub fn admit(&mut self, ts: i64, line: &str) -> bool {
        let h = hash(line);
        match self.last {
            Some(l) if ts < l => false,
            Some(l) if ts == l => self.at_last.insert(h),
            _ => {
                self.last = Some(ts);
                self.at_last.clear();
                self.at_last.insert(h);
                true
            }
        }
    }
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

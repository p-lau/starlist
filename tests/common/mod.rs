//! Helpers shared by the integration tests. Each test crate uses a subset.
#![allow(dead_code)]

use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// A fresh database path, unique per process and call.
pub fn temp_db(prefix: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&p);
    p
}

/// An empty store in a temporary file.
pub fn store() -> Store {
    Store::open(temp_db("starlist-test")).unwrap()
}

/// An event parsed from `text` with the default redactor, at time `ts`.
pub fn ev(text: &str, ts: f64) -> Event {
    let mut e = parse_event(text, &Redactor::default(), None, None);
    e.ts = Some(ts);
    e
}

/// The pattern ID `text` gets.
pub fn id_of(text: &str) -> String {
    ev(text, 0.0).pattern_id
}

/// `count` events spread evenly over [from, to).
pub fn spread(store: &Store, text: &str, from: f64, to: f64, count: usize) {
    let step = (to - from) / count as f64;
    let evs: Vec<Event> = (0..count)
        .map(|i| ev(text, from + step * i as f64))
        .collect();
    store.record(&evs).unwrap();
}

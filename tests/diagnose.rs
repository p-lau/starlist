//! The diagnose answer and the slow-rise test, with controlled time.

use starlist::analysis::{DEFAULT_MAX_CHARS, Detail, Page, whats_new};
use starlist::diagnose::{diagnose, rises};
use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;

const NOW: f64 = 1_800_000_000.0;
const H: f64 = 3600.0;
const M: f64 = 60.0;

fn store() -> Store {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "starlist-diagnose-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    Store::open(p).unwrap()
}

fn ev(text: &str, ts: f64) -> Event {
    let mut e = parse_event(text, &Redactor::default(), None, None);
    e.ts = Some(ts);
    e
}

/// `count` events spread evenly over [from, to).
fn spread(store: &Store, text: &str, from: f64, to: f64, count: usize) {
    let step = (to - from) / count as f64;
    let evs: Vec<Event> = (0..count)
        .map(|i| ev(text, from + step * i as f64))
        .collect();
    store.record(&evs).unwrap();
}

fn id_of(text: &str) -> String {
    ev(text, 0.0).pattern_id
}

/// A day of steady traffic.
fn background(s: &Store) {
    spread(
        s,
        "INFO GET /api/items 200 in 12ms",
        NOW - 26.0 * H,
        NOW,
        26_000,
    );
}

const REJECT: &str = "WARN checkout rejected: out of stock";

#[test]
fn a_slow_rise_is_found_without_a_deploy() {
    let s = store();
    background(&s);
    // 40/h for most of the day, then 1.6x for the last 3 hours: no single hour is a spike.
    spread(&s, REJECT, NOW - 26.0 * H, NOW - 3.0 * H, 23 * 40);
    spread(&s, REJECT, NOW - 3.0 * H, NOW, 3 * 64);

    let r = rises(&s, H, NOW).unwrap();
    let rise = r.get(&id_of(REJECT)).expect("rise found");
    assert!(rise.ratio > 1.4 && rise.ratio < 1.8, "{rise:?}");
    assert!(rise.past_max.is_some(), "{rise:?}");
    assert_eq!(r.len(), 1, "only the rising pattern: {r:?}");

    let v = whats_new(
        &s,
        None,
        None,
        None,
        &Page::first(10, Detail::Brief),
        Detail::Brief,
        NOW,
    )
    .unwrap();
    let f = &v["findings"][0];
    assert_eq!(f["kind"], "rising", "{v}");
    assert!(v["summary"].as_str().unwrap().contains("1 rising"), "{v}");
}

#[test]
fn a_pattern_that_always_swings_is_not_a_rise() {
    let s = store();
    background(&s);
    // A batch job: every 3 hours, one busy hour at twice the rate. The last hour is busy.
    let mut t = NOW - 26.0 * H;
    let mut i = 0;
    while t < NOW {
        let n = if i % 3 == 2 { 80 } else { 40 };
        spread(&s, REJECT, t, t + H, n);
        t += H;
        i += 1;
    }
    let r = rises(&s, H, NOW).unwrap();
    assert!(r.is_empty(), "{r:?}");
}

#[test]
fn diagnose_names_the_new_error_and_its_deploy() {
    let s = store();
    background(&s);
    spread(&s, REJECT, NOW - 26.0 * H, NOW, 26 * 40);
    s.add_deploy("v2.4.0", NOW - 45.0 * M, None).unwrap();
    let err = "ERROR request_id=abc123 failed to send receipt: TypeError: unsupported operand";
    spread(&s, err, NOW - 44.0 * M, NOW, 30);

    let v = diagnose(&s, None, None, 3, DEFAULT_MAX_CHARS, Detail::Brief, NOW).unwrap();
    let c = &v["causes"][0];
    assert_eq!(c["kind"], "new", "{v}");
    assert_eq!(c["started_with"], "v2.4.0", "{v}");
    assert!(
        c["samples"][0].as_str().unwrap().contains("TypeError"),
        "{v}"
    );
    assert!(c["why"].as_str().unwrap().starts_with("New: 30"), "{v}");
    let verdict = v["verdict"].as_str().unwrap();
    assert!(verdict.starts_with("Most likely a new ERROR"), "{verdict}");
    assert!(verdict.contains("since deploy v2.4.0"), "{verdict}");
    assert!(v["next"].is_string(), "{v}");

    // A small budget still keeps the top cause.
    let small = diagnose(&s, None, None, 3, 1000, Detail::Brief, NOW).unwrap();
    assert!(
        serde_json::to_string(&small).unwrap().len() <= 1100,
        "{small}"
    );
    assert_eq!(small["causes"][0]["kind"], "new", "{small}");
}

#[test]
fn diagnose_with_nothing_new_lists_what_is_ongoing() {
    let s = store();
    background(&s);
    spread(&s, REJECT, NOW - 26.0 * H, NOW, 26 * 40);
    let v = diagnose(&s, None, None, 3, DEFAULT_MAX_CHARS, Detail::Brief, NOW).unwrap();
    assert!(
        v["verdict"]
            .as_str()
            .unwrap()
            .starts_with("Nothing stands out"),
        "{v}"
    );
    assert!(v["causes"].as_array().is_none_or(|c| c.is_empty()), "{v}");
    assert_eq!(v["ongoing"][0]["level"], "WARN", "{v}");
}

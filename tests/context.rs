//! Raw, redacted lines around an event (issue #10).

use starlist::analysis::{Detail, compact, get_context};
use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;

const NOW: f64 = 1_800_000_000.0;
const BUG: &str = "ERROR checkout failed for cart 7\nTraceback (most recent call last):\n  File \"/app/pricing.py\", line 41, in price_cart\nKeyError: 'currency'";

fn store() -> Store {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "starlist-context-{}-{}.db",
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

/// 60 lines a second apart ending 10 minutes ago, with the error at line 40.
fn timeline(s: &Store) -> Vec<Event> {
    let start = NOW - 600.0 - 60.0;
    let evs: Vec<Event> = (0..60)
        .map(|i| {
            let ts = start + i as f64;
            match i {
                38 => ev("WARN cart lookup slow for alice@example.com", ts),
                40 => ev(BUG, ts),
                _ => ev(&format!("INFO request {i} ok"), ts),
            }
        })
        .collect();
    s.record(&evs).unwrap();
    evs
}

fn len(v: &serde_json::Value) -> usize {
    serde_json::to_string(&compact(v.clone()))
        .unwrap()
        .chars()
        .count()
}

#[test]
fn shows_the_lines_around_the_latest_occurrence() {
    let s = store();
    let evs = timeline(&s);
    let v = get_context(
        &s,
        Some(&evs[40].pattern_id),
        None,
        5,
        3,
        8000,
        Detail::Brief,
        NOW,
    )
    .unwrap();
    let lines: Vec<&str> = v["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    assert_eq!(lines.len(), 9, "{v}");
    assert!(lines[5].starts_with(">> 0s ERROR checkout failed"), "{v}");
    assert!(lines[5].contains("pricing.py"), "{v}");
    assert!(lines[0].starts_with("-5s INFO request 35"), "{v}");
    assert!(lines[8].starts_with("+3s INFO request 43"), "{v}");
    // Lines are redacted before they are stored.
    assert!(
        lines[3].contains("WARN cart lookup slow") && !lines[3].contains("alice@"),
        "{v}"
    );
}

#[test]
fn centers_on_a_time() {
    let s = store();
    timeline(&s);
    // 10m30s ago is line 30.
    let v = get_context(&s, None, Some("630s"), 1, 1, 8000, Detail::Full, NOW).unwrap();
    let lines = v["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1]["anchor"], true);
    assert!(
        lines[1]["text"].as_str().unwrap().contains("request 30"),
        "{v}"
    );
}

#[test]
fn stays_within_the_budget_and_keeps_the_anchor() {
    let s = store();
    let long = format!("INFO {}", "padding words ".repeat(40));
    let mut evs: Vec<Event> = (0..200)
        .map(|i| ev(&long, NOW - 300.0 + i as f64))
        .collect();
    evs.push(ev(BUG, NOW - 99.0));
    evs.extend((0..200).map(|i| ev(&long, NOW - 98.0 + i as f64 * 0.1)));
    s.record(&evs).unwrap();
    let v = get_context(
        &s,
        Some(&evs[200].pattern_id),
        None,
        200,
        200,
        2000,
        Detail::Brief,
        NOW,
    )
    .unwrap();
    assert!(len(&v) <= 2000, "{} chars", len(&v));
    let lines = v["lines"].as_array().unwrap();
    assert!(lines.iter().any(|l| l.as_str().unwrap().starts_with(">> ")));
    assert!(v["trimmed"].as_str().unwrap().contains("left out"), "{v}");
}

#[test]
fn the_buffer_stays_within_its_size() {
    let s = store();
    s.set_raw_cap(Some(20_000)).unwrap();
    for b in 0..20 {
        let evs: Vec<Event> = (0..100)
            .map(|i| {
                ev(
                    &format!("INFO line {b}-{i} {}", "x".repeat(80)),
                    NOW - 5000.0 + (b * 100 + i) as f64,
                )
            })
            .collect();
        s.record(&evs).unwrap();
    }
    let (n, oldest) = s.raw_span().unwrap();
    // About 100 bytes per line: never more than the cap, and the newest lines are kept.
    assert!(n > 100 && n * 90 <= 20_000, "{n} lines");
    assert!(oldest.unwrap() > NOW - 5000.0 + 1500.0);
}

#[test]
fn no_raw_deletes_lines_and_explains() {
    let s = store();
    let evs = timeline(&s);
    s.set_raw_cap(None).unwrap();
    s.record(&[ev("INFO later", NOW)]).unwrap();
    assert_eq!(s.raw_span().unwrap().0, 0);
    let err = get_context(
        &s,
        Some(&evs[40].pattern_id),
        None,
        5,
        5,
        8000,
        Detail::Brief,
        NOW,
    )
    .unwrap_err();
    assert!(err.contains("--no-raw"), "{err}");
}

#[test]
fn bad_inputs_are_explained() {
    let s = store();
    timeline(&s);
    assert!(
        get_context(&s, None, None, 5, 5, 8000, Detail::Brief, NOW)
            .unwrap_err()
            .contains("pattern_id, at")
    );
    assert!(
        get_context(&s, Some("pnope"), None, 5, 5, 8000, Detail::Brief, NOW)
            .unwrap_err()
            .contains("list_patterns")
    );
    // Before the buffer starts.
    let err = get_context(&s, None, Some("2h"), 5, 5, 8000, Detail::Brief, NOW).unwrap_err();
    assert!(err.contains("back to"), "{err}");
}

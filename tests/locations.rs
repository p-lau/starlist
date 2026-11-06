//! Code locations from stack traces on findings and patterns (issue #9).

use starlist::analysis::{DEFAULT_MAX_CHARS, Detail, Page, get_pattern, whats_new};
use starlist::collector::Pipeline;
use starlist::redact::Redactor;
use starlist::store::Store;
use std::sync::Arc;

fn store() -> Arc<Store> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let db = std::env::temp_dir().join(format!(
        "starlist-loc-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&db);
    Arc::new(Store::open(&db).unwrap())
}

fn trace(ts: &str, func: &str, line: u32) -> String {
    format!(
        "{ts} ERROR checkout failed\nTraceback (most recent call last):\n  File \"/app/shop/views.py\", line 12, in checkout\n  File \"/app/shop/pricing.py\", line {line}, in {func}\n  File \"/usr/lib/python3.12/site-packages/lib/x.py\", line 3, in get\nKeyError: 'currency'"
    )
}

#[test]
fn findings_point_at_the_failing_line_and_patterns_group_by_it() {
    let s = store();
    let p = Pipeline::with_merging(s.clone(), Redactor::default(), None, None)
        .with_source_roots(vec!["/app".into()]);
    let mut events = vec![];
    for m in 0..6 {
        events.push(trace(
            &format!("2027-01-15T07:{:02}:00Z", 30 + m),
            "price_cart",
            41,
        ));
    }
    for m in 0..2 {
        events.push(trace(
            &format!("2027-01-15T07:{:02}:30Z", 40 + m),
            "refund_cart",
            88,
        ));
    }
    p.ingest(&events);
    let now = chrono::DateTime::parse_from_rfc3339("2027-01-15T08:00:00Z")
        .unwrap()
        .timestamp() as f64;

    let v = whats_new(
        &s,
        Some("1h"),
        Some("24h"),
        None,
        &Page::first(10, Detail::Brief),
        Detail::Brief,
        now,
    )
    .unwrap();
    let f = &v["findings"][0];
    assert_eq!(f["kind"], "new", "{v}");
    // The most frequent location, relative to the repository.
    assert_eq!(f["location"], "shop/pricing.py:41 in price_cart", "{v}");

    let id = f["pattern_id"].as_str().unwrap();
    let g = get_pattern(&s, id, None, DEFAULT_MAX_CHARS, Detail::Brief, now).unwrap();
    let locs = g["locations"].as_array().unwrap();
    assert_eq!(locs.len(), 2, "{g}");
    assert_eq!(locs[0]["location"], "shop/pricing.py:41 in price_cart");
    assert_eq!(locs[0]["count"], 6);
    assert_eq!(locs[1]["location"], "shop/pricing.py:88 in refund_cart");
    assert_eq!(locs[1]["count"], 2);
}

#[test]
fn lines_without_a_trace_have_no_location() {
    let s = store();
    let p = Pipeline::with_merging(s.clone(), Redactor::default(), None, None);
    p.ingest(&["2027-01-15T07:30:00Z ERROR db timeout after 3000ms".to_string()]);
    let id = p.parse("ERROR db timeout after 3000ms", 0.0).pattern_id;
    let g = get_pattern(
        &s,
        &id,
        None,
        DEFAULT_MAX_CHARS,
        Detail::Brief,
        1_800_000_000.0,
    )
    .unwrap();
    assert!(g.get("location").is_none_or(|l| l.is_null()), "{g}");
    assert!(g.get("locations").is_none_or(|l| l.is_null()), "{g}");
}

#[test]
fn merged_patterns_keep_their_locations() {
    let s = store();
    let p = Pipeline::with_merging(s.clone(), Redactor::default(), None, Some(3))
        .with_source_roots(vec!["/app".into()]);
    let events: Vec<String> = ["alice", "bob", "carol", "dave"]
        .iter()
        .enumerate()
        .map(|(i, u)| {
            format!(
                "2027-01-15T07:3{i}:00Z ERROR login failed for {u}\nTraceback (most recent call last):\n  File \"/app/auth.py\", line 7, in login\nValueError: bad password"
            )
        })
        .collect();
    p.ingest(&events);
    let id = p.parse("ERROR login failed for alice\nTraceback (most recent call last):\n  File \"/app/auth.py\", line 7, in login\nValueError: bad password", 0.0).pattern_id;
    let g = get_pattern(
        &s,
        &id,
        None,
        DEFAULT_MAX_CHARS,
        Detail::Brief,
        1_800_000_000.0,
    )
    .unwrap();
    assert!(g["template"].as_str().unwrap().contains("<*>"), "{g}");
    assert_eq!(g["location"], "auth.py:7 in login", "{g}");
    assert_eq!(g["total_seen"], 4);
}

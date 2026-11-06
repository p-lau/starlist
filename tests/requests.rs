//! Following one request through the log (issue #11).

use starlist::analysis::{DEFAULT_MAX_CHARS, Detail, Page, compact, get_request, whats_new};
use starlist::collector::Pipeline;
use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;
use std::sync::Arc;

const NOW: f64 = 1_800_000_000.0;

fn store() -> Arc<Store> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "starlist-requests-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    Arc::new(Store::open(p).unwrap())
}

fn ev(text: &str, ts: f64) -> Event {
    let mut e = parse_event(text, &Redactor::default(), None, None);
    e.ts = Some(ts);
    e
}

/// Two requests interleaved; the second one fails.
fn two_requests(s: &Store) {
    let t = NOW - 600.0;
    s.record(&[
        ev("INFO GET /cart request_id=okreq-aaaa", t),
        ev("INFO GET /checkout request_id=badreq-bbbb", t + 0.1),
        ev("INFO cart loaded request_id=okreq-aaaa", t + 0.2),
        ev("INFO pricing cart in EUR request_id=badreq-bbbb", t + 0.3),
        ev(
            "ERROR checkout failed request_id=badreq-bbbb\nKeyError: 'currency'",
            t + 0.5,
        ),
        ev("INFO 200 OK request_id=okreq-aaaa", t + 0.6),
        ev(
            "INFO 500 Internal Server Error request_id=badreq-bbbb",
            t + 0.7,
        ),
    ])
    .unwrap();
}

#[test]
fn a_finding_links_to_a_request_that_can_be_followed() {
    let s = store();
    two_requests(&s);
    let v = whats_new(
        &s,
        "1h",
        "24h",
        Some("ERROR"),
        &Page::first(5, Detail::Brief),
        Detail::Brief,
        NOW,
    )
    .unwrap();
    let f = &v["findings"][0];
    assert_eq!(f["example_request"], "badreq-bbbb", "{v}");

    let r = get_request(&s, "badreq-bbbb", DEFAULT_MAX_CHARS, Detail::Brief, NOW).unwrap();
    let lines: Vec<&str> = r["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    assert_eq!(r["events"], 4, "{r}");
    assert!(lines[0].starts_with("0s INFO GET /checkout"), "{r}");
    assert!(lines[2].contains("ERROR checkout failed"), "{r}");
    assert!(lines[3].contains("500"), "{r}");
    assert!(!lines.iter().any(|l| l.contains("okreq")), "{r}");
}

#[test]
fn request_ids_stay_out_of_patterns() {
    let a = ev("INFO handled request_id=abcdefgh", 0.0);
    let b = ev("INFO handled request_id=qrstuvwx", 0.0);
    assert_eq!(a.pattern_id, b.pattern_id);
    assert!(a.template.contains("<REQ>"), "{}", a.template);
    assert_eq!(a.request_id.as_deref(), Some("abcdefgh"));
}

#[test]
fn long_requests_keep_their_start_and_end() {
    let s = store();
    let mut evs = vec![ev("INFO start request_id=longreq-1", NOW - 100.0)];
    evs.extend((0..300).map(|i| {
        ev(
            &format!(
                "INFO step {} {} request_id=longreq-1",
                i,
                "detail ".repeat(20)
            ),
            NOW - 99.0 + i as f64 * 0.1,
        )
    }));
    evs.push(ev("ERROR gave up request_id=longreq-1", NOW - 50.0));
    s.record(&evs).unwrap();
    let r = get_request(&s, "longreq-1", 2000, Detail::Brief, NOW).unwrap();
    let n = serde_json::to_string(&compact(r.clone()))
        .unwrap()
        .chars()
        .count();
    assert!(n <= 2000, "{n} chars");
    let lines = r["lines"].as_array().unwrap();
    assert!(
        lines
            .first()
            .unwrap()
            .as_str()
            .unwrap()
            .contains("INFO start"),
        "{r}"
    );
    assert!(
        lines
            .last()
            .unwrap()
            .as_str()
            .unwrap()
            .contains("ERROR gave up"),
        "{r}"
    );
    assert!(
        r["trimmed"].as_str().unwrap().contains("middle events"),
        "{r}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.as_str().unwrap().contains("left out")),
        "{r}"
    );
}

#[test]
fn traceparent_and_extra_keys_work() {
    let s = store();
    let p = Pipeline::with_merging(s.clone(), Redactor::default(), None, None)
        .with_request_keys(&["order_ref".to_string()]);
    p.ingest(&[
        "2027-01-15T07:30:00Z INFO paid orderRef=ORD-9911".to_string(),
        "2027-01-15T07:30:01Z INFO shipped orderRef=ORD-9911".to_string(),
        "2027-01-15T07:30:02Z INFO call traceparent=00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string(),
    ]);
    let r = get_request(&s, "ORD-9911", DEFAULT_MAX_CHARS, Detail::Brief, NOW).unwrap();
    assert_eq!(r["events"], 2, "{r}");
    let t = get_request(
        &s,
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        DEFAULT_MAX_CHARS,
        Detail::Brief,
        NOW,
    )
    .unwrap();
    assert_eq!(t["request_id"], "4bf92f3577b34da6a3ce929d0e0e4736");
    assert_eq!(t["events"], 1);
}

#[test]
fn unknown_requests_and_no_raw_are_explained() {
    let s = store();
    two_requests(&s);
    let err = get_request(&s, "nope-1234", DEFAULT_MAX_CHARS, Detail::Brief, NOW).unwrap_err();
    assert!(err.contains("--request-key"), "{err}");
    s.set_raw_cap(None).unwrap();
    let err = get_request(&s, "badreq-bbbb", DEFAULT_MAX_CHARS, Detail::Brief, NOW).unwrap_err();
    assert!(err.contains("--no-raw"), "{err}");
}

#[test]
fn databases_from_before_request_ids_are_upgraded() {
    let p = std::env::temp_dir().join(format!("starlist-requests-old-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&p);
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch(
            "CREATE TABLE raw(id INTEGER PRIMARY KEY, ts REAL NOT NULL, pattern_id TEXT NOT NULL, text TEXT NOT NULL);
             INSERT INTO raw(ts, pattern_id, text) VALUES (1.0, 'pold', 'INFO old line');",
        )
        .unwrap();
    }
    let s = Store::open(&p).unwrap();
    s.record(&[ev("INFO new request_id=fresh-123", NOW)])
        .unwrap();
    assert_eq!(s.raw_span().unwrap().0, 2);
    assert_eq!(s.request_lines("fresh-123", 10).unwrap().len(), 1);
}

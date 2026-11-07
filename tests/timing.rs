//! Errors that come at certain times of day: usual spikes are labelled, not alerted, and
//! verify_fix waits for a pattern's usual hours.

use starlist::alerts::{AlertConfig, Detector};
use starlist::analysis::{verify_fix_data, whats_new_data};
use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;

/// 08:00 UTC.
const NOW: f64 = 1_800_000_000.0;
const DAY: f64 = 86400.0;
const HOUR: f64 = 3600.0;
const NIGHTLY: &str = "ERROR nightly export failed\nTimeoutError: upstream";

fn store() -> Store {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "starlist-timing-{}-{}.db",
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

fn record(s: &Store, text: &str, ts: f64, n: usize) {
    let evs: Vec<Event> = (0..n).map(|i| ev(text, ts + i as f64 * 0.5)).collect();
    s.record(&evs).unwrap();
}

/// Steady healthy traffic, one line a minute, from `lo` to `hi`.
fn traffic(s: &Store, lo: f64, hi: f64) {
    let evs: Vec<Event> = (0..((hi - lo) / 60.0) as usize)
        .map(|i| ev("INFO GET /items 200", lo + i as f64 * 60.0))
        .collect();
    s.record(&evs).unwrap();
}

/// A week of traffic, with `n` nightly errors at `offset` before `NOW` on each earlier day.
fn week(s: &Store, offset: f64, n: usize, days: usize) {
    traffic(s, NOW - 8.0 * DAY, NOW);
    for k in 1..=days {
        record(s, NIGHTLY, NOW - k as f64 * DAY - offset, n);
    }
}

fn nightly_id(s: &Store) -> String {
    s.patterns_seen_since(0.0)
        .unwrap()
        .into_iter()
        .find(|p| p.template.contains("nightly"))
        .unwrap()
        .id
}

fn spike(s: &Store, now: f64) -> starlist::analysis::Finding {
    let data = whats_new_data(s, HOUR, DAY, None, now).unwrap();
    data.findings
        .into_iter()
        .find(|f| f.row.template.contains("nightly"))
        .expect("the nightly error is a finding")
}

#[test]
fn a_spike_at_its_usual_time_is_labelled_and_not_alerted() {
    let s = store();
    week(&s, 10.0 * 60.0, 30, 7);
    record(&s, NIGHTLY, NOW - 8.0 * 60.0, 30);

    let f = spike(&s, NOW);
    assert_eq!(f.kind, "spike", "still reported");
    assert!(f.usual, "{:?}", f.note);
    assert!(
        f.note
            .as_deref()
            .unwrap()
            .contains("Usual for this time of day")
    );

    let mut d = Detector::new(AlertConfig::default());
    let mut alerts = vec![];
    for t in 0..30 {
        alerts.extend(d.tick(&s, NOW - 5.0 * 60.0 + t as f64 * 10.0));
    }
    assert!(alerts.is_empty(), "usual spikes aren't alerted: {alerts:?}");
}

#[test]
fn a_spike_worse_than_usual_still_stands_out() {
    let s = store();
    week(&s, 10.0 * 60.0, 30, 7);
    record(&s, NIGHTLY, NOW - 8.0 * 60.0, 200);

    let f = spike(&s, NOW);
    assert!(!f.usual);
    assert!(
        f.note.as_deref().unwrap().contains("Worse than usual"),
        "{:?}",
        f.note
    );

    let mut d = Detector::new(AlertConfig::default());
    let alert = (0..30).find_map(|t| d.tick(&s, NOW - 5.0 * 60.0 + t as f64 * 10.0));
    assert!(alert.is_some(), "worse than usual is alerted");
}

#[test]
fn too_little_history_is_not_usual() {
    let s = store();
    week(&s, 10.0 * 60.0, 30, 2);
    record(&s, NIGHTLY, NOW - 8.0 * 60.0, 30);
    let f = spike(&s, NOW);
    assert!(!f.usual, "two earlier nights aren't a habit yet");
}

#[test]
fn overrides_win() {
    let s = store();
    week(&s, 10.0 * 60.0, 30, 7);
    record(&s, NIGHTLY, NOW - 8.0 * 60.0, 30);
    let id = nightly_id(&s);

    starlist::issues::set_state_and_timing(
        &s,
        &id,
        None,
        Some("always_alert"),
        None,
        None,
        None,
        NOW,
    )
    .unwrap();
    assert!(!spike(&s, NOW).usual, "always_alert is never usual");

    // A new pattern with no history, marked as expected.
    let t = store();
    traffic(&t, NOW - 2.0 * DAY, NOW);
    record(&t, NIGHTLY, NOW - DAY - 600.0, 30);
    record(&t, NIGHTLY, NOW - 8.0 * 60.0, 30);
    assert!(!spike(&t, NOW).usual);
    let id = nightly_id(&t);
    starlist::issues::set_state_and_timing(&t, &id, None, Some("usual"), None, None, None, NOW)
        .unwrap();
    let f = spike(&t, NOW);
    assert!(f.usual, "{:?}", f.note);
    assert!(f.note.unwrap().contains("Marked as expected"));

    let err = starlist::issues::set_state_and_timing(&t, &id, None, None, None, None, None, NOW);
    assert!(err.is_err());
    let err = starlist::issues::set_state_and_timing(
        &t,
        &id,
        None,
        Some("sometimes"),
        None,
        None,
        None,
        NOW,
    );
    assert!(err.unwrap_err().contains("Unknown timing"));
}

#[test]
fn verify_fix_waits_for_the_usual_hours() {
    let s = store();
    // Fails around 02:00 every night, today too; it's 08:00 and the fix went out at 06:00.
    week(&s, 6.0 * HOUR, 20, 7);
    record(&s, NIGHTLY, NOW - 6.0 * HOUR, 20);
    let id = nightly_id(&s);

    let c = verify_fix_data(&s, &id, "2h", None, NOW).unwrap();
    assert_eq!(c.verdict, "too_early", "{}", c.explanation);
    assert!(
        c.usual_hours
            .as_deref()
            .unwrap()
            .contains("02:00-03:00 UTC"),
        "{:?}",
        c.usual_hours
    );
    let wait = c.wait_minutes.unwrap();
    assert!(
        (18.0 * 60.0..=20.0 * 60.0).contains(&wait),
        "until 03:00 tomorrow, got {wait}"
    );

    // 04:00 the next day: the usual hour came round without it.
    let later = NOW + 20.0 * HOUR;
    traffic(&s, NOW, later);
    let c = verify_fix_data(&s, &id, "22h", None, later).unwrap();
    assert_eq!(c.verdict, "fixed", "{}", c.explanation);
    assert!(
        c.explanation.contains("come round 1 time"),
        "{}",
        c.explanation
    );
}

#[test]
fn errors_spread_through_the_day_have_no_usual_hours() {
    let s = store();
    traffic(&s, NOW - 8.0 * DAY, NOW);
    for h in 0..(7 * 24) {
        if h % 5 == 0 {
            record(&s, NIGHTLY, NOW - 7.0 * DAY + h as f64 * HOUR, 2);
        }
    }
    let id = nightly_id(&s);
    let c = verify_fix_data(&s, &id, "3h", None, NOW).unwrap();
    assert!(c.usual_hours.is_none(), "{:?}", c.usual_hours);
}

//! Scenario tests for "what's new" and "verify my fix" with controlled time.

use starlist::analysis::{Detail, compact, compare_deploy, get_pattern, trim_trace, verify_fix, whats_new};
use starlist::pattern::{Event, parse_event, pattern_id, mask};
use starlist::redact::Redactor;
use starlist::store::Store;

const NOW: f64 = 1_800_000_000.0;
const H: f64 = 3600.0;
const M: f64 = 60.0;

fn store() -> Store {
    let p = std::env::temp_dir().join(format!("starlist-test-{}-{}.db", std::process::id(), rand_suffix()));
    Store::open(p).unwrap()
}

fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

fn ev(text: &str, ts: f64) -> Event {
    let mut e = parse_event(text, &Redactor::default(), None, None);
    e.ts = Some(ts);
    e
}

/// `count` events spread evenly over [from, to).
fn spread(store: &Store, text: &str, from: f64, to: f64, count: usize) {
    let step = (to - from) / count as f64;
    let evs: Vec<Event> = (0..count).map(|i| ev(text, from + step * i as f64)).collect();
    store.record(&evs).unwrap();
}

fn id_of(text: &str) -> String {
    ev(text, 0.0).pattern_id
}

#[test]
fn whats_new_finds_new_spiking_and_quiet_patterns() {
    let s = store();
    // Steady background for a day.
    spread(&s, "INFO GET /api/items 200 in 12ms", NOW - 25.0 * H, NOW, 3000);
    spread(&s, "WARN slow query took 900ms", NOW - 25.0 * H, NOW, 50);
    // Spike: timeouts were rare, now frequent.
    spread(&s, "ERROR upstream timeout after 3000ms", NOW - 25.0 * H, NOW - H, 12);
    spread(&s, "ERROR upstream timeout after 3000ms", NOW - 50.0 * M, NOW, 40);
    // Brand new error in the last hour.
    spread(&s, "ERROR checkout failed for cart 7\nTraceback (most recent call last):\n  File \"a.py\"\nKeyError: 'currency'", NOW - 30.0 * M, NOW, 9);
    // Heartbeat that stopped an hour ago.
    spread(&s, "INFO heartbeat ok", NOW - 25.0 * H, NOW - 70.0 * M, 1440);

    let v = whats_new(&s, "1h", "24h", None, 10, Detail::Brief, NOW).unwrap();
    let findings = v["findings"].as_array().unwrap();
    let kinds: Vec<(&str, &str)> = findings
        .iter()
        .map(|f| (f["kind"].as_str().unwrap(), f["template"].as_str().unwrap()))
        .collect();
    println!("{}", serde_json::to_string_pretty(&v).unwrap());

    // New error ranks first, with its exception type and a redacted sample.
    assert_eq!(kinds[0].0, "new");
    assert!(kinds[0].1.contains("KeyError"));
    assert!(findings[0]["sample"].as_str().unwrap().contains("Traceback"));
    assert!(kinds.iter().any(|(k, t)| *k == "spike" && t.contains("upstream timeout")));
    // Good news comes last.
    assert_eq!(kinds.last().unwrap().0, "gone_quiet");
    assert!(kinds.last().unwrap().1.contains("heartbeat"));
    // Steady patterns are not reported.
    assert!(!kinds.iter().any(|(_, t)| t.contains("GET /api/items") || t.contains("slow query")));
}

#[test]
fn whats_new_is_quiet_when_nothing_changed() {
    let s = store();
    spread(&s, "INFO GET /api/items 200 in 12ms", NOW - 25.0 * H, NOW, 3000);
    let v = whats_new(&s, "1h", "24h", None, 10, Detail::Brief, NOW).unwrap();
    assert!(v["findings"].as_array().unwrap().is_empty());
    assert!(v["summary"].as_str().unwrap().starts_with("Nothing new"));
}

const BUG: &str = "ERROR checkout failed for cart 7\nTraceback (most recent call last):\nKeyError: 'currency'";

#[test]
fn verify_fix_says_fixed_when_error_stops() {
    let s = store();
    spread(&s, "INFO request ok 200", NOW - 3.0 * H, NOW, 2000);
    spread(&s, BUG, NOW - 2.0 * H, NOW - 30.0 * M, 90); // ~1/min until the fix
    s.add_marker("fix-currency", NOW - 30.0 * M, "manual", None).unwrap();
    let v = verify_fix(&s, &id_of(BUG), "fix-currency", None, Detail::Brief, NOW).unwrap();
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
    assert_eq!(v["verdict"], "fixed");
    assert_eq!(v["confidence"], "high");
    assert_eq!(v["split"]["marker"], "fix-currency");
}

#[test]
fn verify_fix_says_too_early_for_rare_errors() {
    let s = store();
    spread(&s, BUG, NOW - 10.0 * H, NOW - 10.0 * M, 5); // ~1 every 2 hours
    let v = verify_fix(&s, &id_of(BUG), "10m", None, Detail::Brief, NOW).unwrap();
    assert_eq!(v["verdict"], "too_early");
    assert!(v["wait_minutes"].as_f64().unwrap() > 60.0, "{v}");
}

#[test]
fn verify_fix_detects_still_occurring_and_worse() {
    let s = store();
    spread(&s, BUG, NOW - 2.0 * H, NOW - H, 60);
    spread(&s, BUG, NOW - H, NOW, 55);
    assert_eq!(verify_fix(&s, &id_of(BUG), "1h", None, Detail::Brief, NOW).unwrap()["verdict"], "still_occurring");

    let s = store();
    spread(&s, BUG, NOW - 2.0 * H, NOW - H, 10);
    spread(&s, BUG, NOW - H, NOW, 80);
    assert_eq!(verify_fix(&s, &id_of(BUG), "1h", None, Detail::Brief, NOW).unwrap()["verdict"], "worse");
}

#[test]
fn verify_fix_flags_errors_introduced_by_the_change() {
    let s = store();
    s.add_marker("v2.0.0", NOW - H, "deploy", None).unwrap();
    spread(&s, BUG, NOW - 50.0 * M, NOW, 20);
    let v = verify_fix(&s, &id_of(BUG), "v2.0.0", None, Detail::Brief, NOW).unwrap();
    assert_eq!(v["verdict"], "appeared_after");
}

#[test]
fn verify_fix_warns_when_traffic_dropped() {
    let s = store();
    spread(&s, "INFO request ok 200", NOW - 2.0 * H, NOW - H, 3000);
    spread(&s, "INFO request ok 200", NOW - H, NOW, 100);
    spread(&s, BUG, NOW - 2.0 * H, NOW - H, 30);
    let v = verify_fix(&s, &id_of(BUG), "1h", None, Detail::Brief, NOW).unwrap();
    assert_eq!(v["verdict"], "fixed");
    assert!(v["caveat"].as_str().unwrap().contains("log volume"), "{v}");
}

#[test]
fn compare_deploy_lists_regressions_first() {
    let s = store();
    spread(&s, "INFO request ok 200", NOW - 3.0 * H, NOW, 3000);
    spread(&s, "WARN cache miss 5", NOW - 3.0 * H, NOW - H, 100);
    spread(&s, "ERROR db timeout 3000ms", NOW - 3.0 * H, NOW, 20);
    s.add_marker("v1.5.0", NOW - H, "deploy", None).unwrap();
    spread(&s, BUG, NOW - 55.0 * M, NOW, 30);
    spread(&s, "ERROR db timeout 3000ms", NOW - 55.0 * M, NOW, 60);
    let v = compare_deploy(&s, "v1.5.0", Some("2h"), 10, Detail::Brief, NOW).unwrap();
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
    let f = v["findings"].as_array().unwrap();
    assert_eq!(f[0]["kind"], "new_after");
    assert_eq!(f[1]["kind"], "increased");
    assert!(f.iter().any(|x| x["kind"] == "stopped" && x["template"].as_str().unwrap().contains("cache miss")));
}

#[test]
fn unknown_inputs_give_helpful_errors() {
    let s = store();
    assert!(verify_fix(&s, "pnope", "1h", None, Detail::Brief, NOW).unwrap_err().contains("list_patterns"));
    spread(&s, BUG, NOW - H, NOW, 3);
    assert!(verify_fix(&s, &id_of(BUG), "no-such-marker", None, Detail::Brief, NOW).unwrap_err().contains("list_markers"));
    assert!(whats_new(&s, "soon", "24h", None, 5, Detail::Brief, NOW).is_err());
}

#[test]
fn pattern_ids_match_template_hash() {
    assert_eq!(id_of("WARN cache miss 5"), pattern_id(&mask("WARN cache miss 5")));
}

#[test]
fn busier_traffic_alone_is_not_a_spike() {
    let s = store();
    spread(&s, "INFO GET /api/items 200 in 12ms", NOW - 25.0 * H, NOW - H, 2400); // 100/h
    spread(&s, "INFO GET /api/items 200 in 12ms", NOW - H, NOW, 500); // 5x busier
    spread(&s, "WARN slow query took 900ms", NOW - 25.0 * H, NOW - H, 240); // 10/h
    spread(&s, "WARN slow query took 900ms", NOW - H, NOW, 50); // grew with traffic
    spread(&s, "ERROR db pool exhausted", NOW - 25.0 * H, NOW - H, 24); // 1/h
    spread(&s, "ERROR db pool exhausted", NOW - H, NOW, 60); // grew much faster than traffic
    let v = whats_new(&s, "1h", "24h", None, 10, Detail::Brief, NOW).unwrap();
    let f = v["findings"].as_array().unwrap();
    assert_eq!(f.len(), 1, "{v}");
    assert!(f[0]["template"].as_str().unwrap().contains("db pool"));
    assert!(v["traffic"]["note"].as_str().unwrap().contains("log volume"));
}

#[test]
fn trims_library_frames_but_keeps_app_frames() {
    let trace = "ERROR request failed
Traceback (most recent call last):
  File \"/app/views.py\", line 10, in handle
    return checkout(cart)
  File \"/usr/lib/python3.12/site-packages/django/core/handlers.py\", line 55, in inner
    response = get_response(request)
  File \"/app/pricing.py\", line 41, in price_cart
    currency = cart.meta['currency']
KeyError: 'currency'";
    let t = trim_trace(trace, 20);
    assert!(t.contains("/app/views.py") && t.contains("/app/pricing.py") && t.contains("KeyError"), "{t}");
    assert!(!t.contains("site-packages") && !t.contains("get_response"), "{t}");
    assert!(t.contains("1 library frames hidden"), "{t}");

    let java = "ERROR boom\njava.lang.IllegalStateException: x\n\tat com.shop.Cart.total(Cart.java:12)\n\tat java.base/java.util.ArrayList.forEach(ArrayList.java:1)\n\tat org.springframework.web.Servlet.service(S.java:3)";
    let t = trim_trace(java, 20);
    assert!(t.contains("com.shop.Cart") && !t.contains("ArrayList") && !t.contains("springframework"), "{t}");
}

#[test]
fn compact_drops_nulls_and_empties() {
    let v = compact(serde_json::json!({"a": null, "b": [], "c": {}, "d": {"e": null, "f": 1}, "g": [{"h": null, "i": 2}]}));
    assert_eq!(v, serde_json::json!({"d": {"f": 1}, "g": [{"i": 2}]}));
}

#[test]
fn brief_output_is_much_smaller_than_full() {
    let s = store();
    spread(&s, "INFO GET /api/items 200 in 12ms", NOW - 25.0 * H, NOW, 3000);
    for i in 0..6 {
        let text = format!("ERROR job {i} failed\nTraceback (most recent call last):\n  File \"/usr/lib/python3.12/site-packages/celery/app.py\", line 1, in run\n    x()\n  File \"/app/jobs.py\", line 9, in job{i}\n    do()\nValueError: bad {i}");
        spread(&s, &text, NOW - 30.0 * M, NOW, 4);
    }
    let size = |d| serde_json::to_string(&compact(whats_new(&s, "1h", "24h", None, 10, d, NOW).unwrap())).unwrap().len();
    let (brief, full) = (size(Detail::Brief), size(Detail::Full));
    println!("whats_new brief={brief} full={full}");
    assert!(brief * 10 < full * 7, "brief {brief} should be well under full {full}");

    let id = ev("ERROR job 0 failed\nTraceback (most recent call last):\nValueError: bad 0", 0.0).pattern_id;
    let g = get_pattern(&s, &id, None, Detail::Brief, NOW).unwrap();
    assert_eq!(g["samples"].as_array().unwrap().len(), 1);
}

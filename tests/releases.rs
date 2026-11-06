//! Which release introduced each pattern (issue #7).

use starlist::analysis::{Detail, add_marker, compare_deploy, get_pattern, whats_new};
use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;

const NOW: f64 = 1_800_000_000.0;
const H: f64 = 3600.0;
const M: f64 = 60.0;
const BG: &str = "INFO GET /api/items 200 in 12ms";
const BUG: &str = "ERROR checkout failed for cart 7\nKeyError: 'currency'";

fn store() -> Store {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "starlist-releases-{}-{}.db",
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

#[test]
fn whats_new_names_the_release_that_introduced_a_pattern() {
    let s = store();
    s.add_deploy("v1.4.2", NOW - 26.0 * H, None).unwrap();
    spread(&s, BG, NOW - 25.0 * H, NOW, 2500);
    s.add_deploy("v1.4.3", NOW - 40.0 * M, None).unwrap();
    spread(&s, BUG, NOW - 30.0 * M, NOW, 9);

    let v = whats_new(&s, "1h", "24h", None, 10, Detail::Brief, NOW).unwrap();
    assert_eq!(v["current_release"], "v1.4.3", "{v}");
    let f = &v["findings"][0];
    assert_eq!(f["kind"], "new");
    assert_eq!(f["first_release"], "v1.4.3", "{v}");

    let p = get_pattern(&s, &id_of(BG), None, Detail::Brief, NOW).unwrap();
    assert_eq!(p["first_release"], "v1.4.2", "{p}");
}

#[test]
fn patterns_before_the_first_deploy_have_no_release() {
    let s = store();
    spread(&s, BG, NOW - 3.0 * H, NOW, 300);
    s.add_deploy("v2.0.0", NOW - H, None).unwrap();
    let p = get_pattern(&s, &id_of(BG), None, Detail::Brief, NOW).unwrap();
    assert!(p.get("first_release").is_none_or(|r| r.is_null()), "{p}");
}

#[test]
fn deploys_skip_restarts_but_keep_rollbacks() {
    let s = store();
    assert!(s.add_deploy("v1", NOW - 3.0 * H, None).unwrap());
    // A replica restart, and re-importing the same log line.
    assert!(!s.add_deploy("v1", NOW - 2.5 * H, None).unwrap());
    assert!(!s.add_deploy("v1", NOW - 3.0 * H, None).unwrap());
    assert!(s.add_deploy("v2", NOW - 2.0 * H, None).unwrap());
    // Rolling back to v1 is a new deploy.
    assert!(s.add_deploy("v1", NOW - H, None).unwrap());
    let names: Vec<String> = s.deploys().unwrap().into_iter().map(|d| d.1).collect();
    assert_eq!(names, ["v1", "v2", "v1"]);
}

#[test]
fn add_marker_can_record_a_deploy() {
    let s = store();
    let v = add_marker(&s, "v3.1.0", Some("release"), Some("2h"), true, NOW).unwrap();
    assert_eq!(v["kind"], "deploy");
    let v = add_marker(&s, "fix-currency", None, None, false, NOW).unwrap();
    assert_eq!(v["kind"], "manual");
    assert_eq!(s.deploys().unwrap().len(), 1);
}

#[test]
fn compare_deploy_defaults_to_the_latest_deploy() {
    let s = store();
    let err = compare_deploy(&s, None, None, 10, Detail::Brief, NOW).unwrap_err();
    assert!(err.contains("deploy"), "{err}");

    spread(&s, BG, NOW - 3.0 * H, NOW, 1000);
    s.add_deploy("v1.5.0", NOW - 2.0 * H, None).unwrap();
    s.add_marker("fix-cache", NOW - 90.0 * M, "manual", None)
        .unwrap();
    s.add_deploy("v1.5.1", NOW - H, None).unwrap();
    spread(&s, BUG, NOW - 50.0 * M, NOW, 20);
    let v = compare_deploy(&s, None, None, 10, Detail::Brief, NOW).unwrap();
    assert_eq!(v["split"]["marker"], "v1.5.1", "{v}");
    assert_eq!(v["findings"][0]["kind"], "new_after");
    assert_eq!(v["findings"][0]["first_release"], "v1.5.1");
}

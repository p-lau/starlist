//! Issue states, regressions, releases, manual merges and grouping rules (issue #23).

use regex::Regex;
use starlist::analysis::{Detail, Page, get_release, whats_new};
use starlist::collector::Pipeline;
use starlist::issues::{Effective, States, merge, release, set_state};
use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;
use std::sync::Arc;

const NOW: f64 = 1_800_000_000.0;
const H: f64 = 3600.0;

fn store() -> Arc<Store> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "starlist-issues-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&p);
    Arc::new(Store::open(p).unwrap())
}

fn ev(text: &str, ts: f64) -> Event {
    let mut e = parse_event(text, &Redactor::default(), None, None);
    e.ts = Some(ts);
    e
}

/// `n` errors spread evenly over [from, to).
fn errors(s: &Store, text: &str, from: f64, to: f64, n: usize) -> String {
    let evs: Vec<Event> = (0..n)
        .map(|i| ev(text, from + (to - from) * i as f64 / n as f64))
        .collect();
    s.record(&evs).unwrap();
    evs[0].pattern_id.clone()
}

fn kinds(s: &Store, now: f64) -> Vec<(String, String)> {
    let v = whats_new(
        s,
        Some("1h"),
        Some("24h"),
        None,
        &Page::first(20, Detail::Brief),
        Detail::Brief,
        now,
    )
    .unwrap();
    // Empty lists are left out of answers.
    v["findings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|f| {
            (
                f["pattern_id"].as_str().unwrap().to_string(),
                f["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn a_resolved_error_that_comes_back_is_reported_as_regressed() {
    let s = store();
    s.add_deploy("v1", NOW - 10.0 * H, None).unwrap();
    let id = errors(
        &s,
        "ERROR charge failed: card declined",
        NOW - 6.0 * H,
        NOW - 4.0 * H,
        30,
    );
    set_state(
        &s,
        &id,
        "resolved",
        None,
        None,
        Some("fixed in v2"),
        NOW - 3.0 * H,
    )
    .unwrap();

    let row = s.pattern(&id).unwrap().unwrap();
    let st = States::load(&s).unwrap();
    assert!(matches!(
        st.effective(&s, &row, NOW - 2.0 * H),
        Effective::Resolved { .. }
    ));

    s.add_deploy("v2", NOW - 50.0 * 60.0, None).unwrap();
    errors(
        &s,
        "ERROR charge failed: card declined",
        NOW - 30.0 * 60.0,
        NOW - 60.0,
        6,
    );
    let row = s.pattern(&id).unwrap().unwrap();
    let st = States::load(&s).unwrap();
    match st.effective(&s, &row, NOW) {
        Effective::Regressed { resolved_at, at } => {
            assert_eq!(resolved_at, NOW - 3.0 * H);
            assert!(
                (NOW - 31.0 * 60.0..=NOW - 29.0 * 60.0).contains(&at),
                "{at}"
            );
        }
        e => panic!("expected regressed, got {e:?}"),
    }
    assert_eq!(kinds(&s, NOW), vec![(id.clone(), "regressed".to_string())]);

    let v = whats_new(
        &s,
        Some("1h"),
        Some("24h"),
        None,
        &Page::first(5, Detail::Brief),
        Detail::Brief,
        NOW,
    )
    .unwrap();
    assert!(v["summary"].as_str().unwrap().contains("1 regressed"));
    assert!(v["findings"][0]["note"].as_str().unwrap().contains("in v2"));

    // The release view shows it regressed in v2.
    let r = release(&s, None, NOW).unwrap();
    assert_eq!(r.name, "v2");
    assert_eq!(r.regressed.len(), 1);
    assert_eq!(r.previous.as_deref(), Some("v1"));
}

#[test]
fn resolved_in_next_release_waits_for_the_deploy() {
    let s = store();
    s.add_deploy("v1", NOW - 10.0 * H, None).unwrap();
    let text = "ERROR timeout calling inventory";
    let id = errors(&s, text, NOW - 5.0 * H, NOW - 3.0 * H, 20);
    set_state(&s, &id, "resolved_next", None, None, None, NOW - 3.0 * H).unwrap();
    // Still happening before the fix ships: not a regression.
    errors(&s, text, NOW - 2.5 * H, NOW - 2.0 * H, 5);
    let row = s.pattern(&id).unwrap().unwrap();
    assert_eq!(
        States::load(&s).unwrap().effective(&s, &row, NOW - 2.0 * H),
        Effective::ResolvedNext
    );
    s.add_deploy("v2", NOW - 1.5 * H, None).unwrap();
    let row = s.pattern(&id).unwrap().unwrap();
    assert!(matches!(
        States::load(&s).unwrap().effective(&s, &row, NOW),
        Effective::Resolved { at } if at == NOW - 1.5 * H
    ));
    let r = release(&s, Some("v2"), NOW).unwrap();
    assert_eq!(r.resolved.len(), 1);
    // After the deploy, it comes back.
    errors(&s, text, NOW - 20.0 * 60.0, NOW - 60.0, 4);
    let row = s.pattern(&id).unwrap().unwrap();
    assert_eq!(
        States::load(&s).unwrap().effective(&s, &row, NOW).name(),
        "regressed"
    );
}

#[test]
fn ignored_patterns_stay_out_of_whats_new_until_the_ignore_ends() {
    let s = store();
    let a = errors(
        &s,
        "ERROR noisy health check failed",
        NOW - 40.0 * 60.0,
        NOW,
        20,
    );
    let b = errors(
        &s,
        "ERROR flaky cache miss storm",
        NOW - 40.0 * 60.0,
        NOW,
        20,
    );
    assert_eq!(kinds(&s, NOW).len(), 2);

    set_state(&s, &a, "ignored", None, None, None, NOW).unwrap();
    set_state(&s, &b, "ignored", Some("1h"), None, None, NOW).unwrap();
    assert!(kinds(&s, NOW).is_empty());
    // b's hour is up.
    let later = kinds(&s, NOW + 1.5 * H);
    assert!(later.iter().all(|(id, _)| *id != a));

    // An ignore bounded by a count ends after that many more events.
    let c = errors(
        &s,
        "ERROR disk almost full",
        NOW - 40.0 * 60.0,
        NOW - 60.0,
        10,
    );
    set_state(&s, &c, "ignored", None, Some(5), None, NOW).unwrap();
    errors(&s, "ERROR disk almost full", NOW - 30.0, NOW - 20.0, 3);
    let row = s.pattern(&c).unwrap().unwrap();
    assert!(
        States::load(&s)
            .unwrap()
            .effective(&s, &row, NOW)
            .is_ignored()
    );
    errors(&s, "ERROR disk almost full", NOW - 10.0, NOW - 5.0, 2);
    let row = s.pattern(&c).unwrap().unwrap();
    assert_eq!(
        States::load(&s).unwrap().effective(&s, &row, NOW).name(),
        "open"
    );

    set_state(&s, &a, "open", None, None, None, NOW).unwrap();
    assert!(kinds(&s, NOW).iter().any(|(id, _)| *id == a));
    assert!(set_state(&s, &a, "snoozed", None, None, None, NOW).is_err());
    assert!(set_state(&s, "nope", "resolved", None, None, None, NOW).is_err());
}

#[test]
fn release_view_lists_new_patterns_and_compares_error_rates() {
    let s = store();
    s.add_deploy("v1", NOW - 4.0 * H, None).unwrap();
    errors(
        &s,
        "ERROR old problem",
        NOW - 4.0 * H + 60.0,
        NOW - 2.0 * H,
        20,
    );
    s.add_deploy("v2", NOW - 2.0 * H, None).unwrap();
    errors(&s, "ERROR old problem", NOW - 2.0 * H + 60.0, NOW, 60);
    let new_id = errors(&s, "ERROR brand new failure", NOW - H, NOW, 10);
    s.set_meta("env", Some("production")).unwrap();

    let r = release(&s, None, NOW).unwrap();
    assert_eq!(r.name, "v2");
    assert_eq!(r.env.as_deref(), Some("production"));
    assert_eq!(r.new.len(), 1);
    assert_eq!(r.new[0].id, new_id);
    assert!(r.errors_per_hour > r.previous_errors_per_hour.unwrap());

    let v1 = release(&s, Some("v1"), NOW).unwrap();
    assert_eq!(v1.until, Some(NOW - 2.0 * H));
    assert!(release(&s, Some("v9"), NOW).is_err());

    let v = get_release(&s, None, 8000, Detail::Brief, NOW).unwrap();
    assert_eq!(v["release"], "v2");
    assert_eq!(v["new"]["total"], 1);
    assert!(v["vs_previous"].as_str().unwrap().contains("in v1"));
}

#[test]
fn manual_merge_keeps_old_ids_working_and_routes_new_events() {
    let s = store();
    let a = errors(
        &s,
        "ERROR payment provider unreachable",
        NOW - H,
        NOW - 0.5 * H,
        5,
    );
    let b = errors(&s, "ERROR payments gateway down", NOW - H, NOW - 0.5 * H, 7);
    let kept = merge(&s, &[a.clone(), b.clone()]).unwrap();
    assert_eq!(kept.id, a);
    assert_eq!(kept.total, 12);
    assert_eq!(s.pattern(&b).unwrap().unwrap().id, a);
    // New events of the merged pattern count toward the kept one.
    errors(&s, "ERROR payments gateway down", NOW - 60.0, NOW - 30.0, 3);
    assert_eq!(s.pattern(&a).unwrap().unwrap().total, 15);
    assert!(merge(&s, std::slice::from_ref(&a)).is_err());
    assert!(merge(&s, &[a.clone(), b]).is_err());
}

#[test]
fn group_rules_put_matching_lines_in_one_pattern() {
    let s = store();
    let p = Pipeline::new(s.clone(), Redactor::default(), None).with_groups(vec![
        Regex::new(r"connection (?:reset|refused|closed) by (?P<name>\w+)").unwrap(),
        Regex::new(r"^WARN slow").unwrap(),
    ]);
    let a = p.parse("ERROR connection reset by postgres while saving order", NOW);
    let b = p.parse("ERROR connection refused by postgres", NOW);
    let c = p.parse("ERROR connection closed by redis during flush", NOW);
    let d = p.parse("WARN slow request /checkout took 3s", NOW);
    let e = p.parse("WARN slow query on carts", NOW);
    assert_eq!(a.pattern_id, b.pattern_id);
    assert_ne!(a.pattern_id, c.pattern_id);
    assert_eq!(a.template, "ERROR group: postgres");
    assert_eq!(d.pattern_id, e.pattern_id);
    assert_eq!(d.level, Some("WARN"));
}

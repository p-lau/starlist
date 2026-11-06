//! Output stays within its size budget on a noisy log, and paging covers
//! every item exactly once (issue #8).

use serde_json::Value;
use starlist::analysis::{
    Detail, MIN_MAX_CHARS, Page, compact, compare_deploy, get_context, get_pattern, list_markers,
    list_patterns, verify_fix, whats_new,
};
use starlist::pattern::{Event, parse_event};
use starlist::redact::Redactor;
use starlist::store::Store;
use std::collections::HashSet;

const NOW: f64 = 1_800_000_000.0;
const H: f64 = 3600.0;
const M: f64 = 60.0;
const PATTERNS: usize = 300;

fn store() -> Store {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "starlist-budget-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    Store::open(p).unwrap()
}

/// A digit-free word per index, so each line is its own pattern.
fn word(mut i: usize) -> String {
    let mut w = String::new();
    loop {
        w.push((b'a' + (i % 26) as u8) as char);
        i /= 26;
        if i == 0 {
            break w;
        }
    }
}

/// A long error with a stack trace, distinct per index.
fn noisy_line(i: usize) -> String {
    let w = word(i);
    let frames: String = (0..40)
        .map(|f| format!("\n  File \"/srv/app/{w}/module{f}.py\", line {f}, in handler_{w}"))
        .collect();
    format!(
        "ERROR request failed in {w} {} because the upstream {w} service rejected it{frames}\nValueError: bad {w}",
        "while processing a very long description of the work ".repeat(4)
    )
}

fn ev(text: &str, ts: f64) -> Event {
    let mut e = parse_event(text, &Redactor::default(), None, None);
    e.ts = Some(ts);
    e
}

fn noisy_store() -> Store {
    let s = store();
    s.add_deploy("v1.0.0", NOW - 30.0 * H, None).unwrap();
    s.add_deploy("v1.1.0", NOW - 40.0 * M, None).unwrap();
    for i in 0..PATTERNS {
        let line = noisy_line(i);
        // Half the patterns are old and steady, half are new since the deploy.
        let evs: Vec<Event> = if i % 2 == 0 {
            (0..30)
                .map(|k| ev(&line, NOW - 25.0 * H + k as f64 * 2.0 * H / 3.0))
                .collect()
        } else {
            (0..6)
                .map(|k| ev(&line, NOW - 30.0 * M + k as f64 * 60.0))
                .collect()
        };
        s.record(&evs).unwrap();
    }
    for i in 0..120 {
        s.add_marker(
            &format!("fix-{}", word(i)),
            NOW - i as f64 * M,
            "manual",
            Some(&"a long note about what changed ".repeat(20)),
        )
        .unwrap();
    }
    s
}

fn len(v: &Value) -> usize {
    serde_json::to_string(&compact(v.clone()))
        .unwrap()
        .chars()
        .count()
}

fn id_of(text: &str) -> String {
    ev(text, 0.0).pattern_id
}

#[test]
fn every_tool_stays_within_its_default_budget() {
    let s = noisy_store();
    for d in [Detail::Brief, Detail::Full] {
        let max = Page::default_budget(d);
        // Largest limits the tools accept.
        let checks = [
            (
                "whats_new",
                whats_new(&s, "1h", "24h", None, &Page::first(50, d), d, NOW),
            ),
            (
                "compare_deploy",
                compare_deploy(&s, None, None, &Page::first(50, d), d, NOW),
            ),
            (
                "list_patterns",
                list_patterns(&s, "48h", None, None, &Page::first(100, d), d, NOW),
            ),
            (
                "get_pattern",
                get_pattern(&s, &id_of(&noisy_line(1)), Some(5), max, d, NOW),
            ),
            (
                "verify_fix",
                verify_fix(&s, &id_of(&noisy_line(0)), "v1.1.0", None, d, NOW),
            ),
            ("list_markers", list_markers(&s, 100, max, NOW)),
            (
                "get_context",
                get_context(
                    &s,
                    Some(&id_of(&noisy_line(1))),
                    None,
                    200,
                    200,
                    max,
                    d,
                    NOW,
                ),
            ),
        ];
        for (name, r) in checks {
            let v = r.unwrap();
            let n = len(&v);
            println!("{name} {d:?}: {n} chars");
            assert!(n <= max, "{name} ({d:?}) is {n} chars, over {max}: {v}");
        }
    }
}

#[test]
fn a_smaller_budget_is_respected_and_says_what_was_cut() {
    let s = noisy_store();
    let d = Detail::Brief;
    let (page, now) = Page::from_args(50, None, Some(2500), d, NOW).unwrap();
    let v = whats_new(&s, "1h", "24h", None, &page, d, now).unwrap();
    assert!(len(&v) <= 2500, "{} chars: {v}", len(&v));
    let trimmed = v["trimmed"].as_str().unwrap();
    assert!(trimmed.contains("next page"), "{trimmed}");
    assert!(v["next_cursor"].is_string());

    // Below the minimum, the minimum applies.
    let (page, _) = Page::from_args(10, None, Some(10), d, NOW).unwrap();
    assert_eq!(page.max_chars, MIN_MAX_CHARS);
}

#[test]
fn paging_returns_every_pattern_once() {
    let s = noisy_store();
    let d = Detail::Brief;
    let mut seen = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        // Time moves on between calls; the cursor keeps the first call's view.
        let (page, now) =
            Page::from_args(40, cursor.as_deref(), None, d, NOW + pages as f64 * 5.0).unwrap();
        let v = list_patterns(&s, "48h", None, None, &page, d, now).unwrap();
        for p in v["patterns"].as_array().unwrap() {
            assert!(seen.insert(p["pattern_id"].as_str().unwrap().to_string()));
        }
        pages += 1;
        match v["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => {
                assert!(v.get("remaining").is_none());
                break;
            }
        }
    }
    assert_eq!(seen.len(), PATTERNS);
    assert!(pages >= PATTERNS / 40, "{pages} pages");
}

#[test]
fn bad_cursor_is_explained() {
    let err = Page::from_args(10, Some("abc"), None, Detail::Brief, NOW).unwrap_err();
    assert!(err.contains("next_cursor"), "{err}");
}

//! Pattern expiry and the pattern cap.

mod common;

use clap::{Arg, Command};
use common::{id_of, spread, store};
use starlist::collector::Pipeline;
use starlist::config::to_args;
use starlist::redact::Redactor;
use starlist::store::{StateRow, Store};
use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::sync::Arc;

const D: f64 = 86400.0;

fn now() -> f64 {
    starlist::analysis::now()
}

fn resolved(id: &str, at: f64) -> StateRow {
    StateRow {
        pattern_id: id.into(),
        state: "resolved".into(),
        since: at,
        until_ts: None,
        until_count: None,
        base_total: 0,
        note: None,
    }
}

fn exists(s: &Store, id: &str) -> bool {
    s.pattern(id).unwrap().is_some()
}

/// Record `n` events of `text` at `ts`.
fn put(s: &Store, text: &str, ts: f64, n: usize) -> String {
    spread(s, text, ts, ts + 1.0, n);
    id_of(text)
}

const OLD_A: &str = "WARN alpha cache miss for tenant";
const OLD_B: &str = "WARN bravo queue stalled";
const FRESH: &str = "WARN charlie heartbeat late";

#[test]
fn expiry_removes_aged_out_patterns_and_their_rows() {
    let s = store();
    let t = now();
    let old = t - 40.0 * D;
    let gone = put(&s, OLD_A, old, 3);
    let kept_state = put(&s, OLD_B, old, 3);
    let fresh = put(&s, FRESH, t - 60.0, 3);
    s.set_state(&kept_state, Some(&resolved(&kept_state, old)))
        .unwrap();
    assert!(!s.samples(&gone, 5).unwrap().is_empty());
    assert!(s.raw_anchor(Some(&gone), None).unwrap().is_some());

    let gen_before = s.split_generation();
    s.rollup(t - 7.0 * D, Some(t - 14.0 * D)).unwrap();

    assert!(!exists(&s, &gone));
    assert!(s.samples(&gone, 5).unwrap().is_empty());
    assert!(s.locations(&gone, 5).unwrap().is_empty());
    assert!(s.raw_anchor(Some(&gone), None).unwrap().is_none());
    assert_eq!(s.count(&gone, 0.0, t + 1.0).unwrap(), 0);
    assert!(exists(&s, &kept_state), "stated pattern must stay");
    assert!(s.states().unwrap().contains_key(&kept_state));
    assert!(exists(&s, &fresh), "pattern with recent counts must stay");
    assert_eq!(s.count(&fresh, t - D, t + 1.0).unwrap(), 3);
    assert!(s.split_generation() > gen_before);
}

#[test]
fn rollup_without_history_keeps_patterns_and_expire_is_idempotent() {
    let s = store();
    let t = now();
    let id = put(&s, OLD_A, t - 40.0 * D, 2);
    s.rollup(t - 7.0 * D, None).unwrap();
    assert!(exists(&s, &id));
    assert_eq!(s.expire_patterns().unwrap(), 0);
    s.rollup(t - 7.0 * D, Some(t - 14.0 * D)).unwrap();
    assert!(!exists(&s, &id));
    assert_eq!(s.expire_patterns().unwrap(), 0);
}

/// `n` distinct patterns "WARN p<letters> ..." seen `count` times, the i-th
/// at `t - (n - i) * 60` so that larger i is more recent.
fn fill(s: &Store, n: usize, count: usize, t: f64) -> Vec<String> {
    (0..n)
        .map(|i| {
            let word: String = (0..4)
                .map(|k| (b'a' + ((i / 26usize.pow(k)) % 26) as u8) as char)
                .collect();
            put(
                s,
                &format!("WARN w{word} unusual thing"),
                t - (n - i) as f64 * 60.0,
                count,
            )
        })
        .collect()
}

#[test]
fn cap_evicts_least_recent_low_count_first_down_to_ninety_percent() {
    let s = store();
    let t = now();
    let ids = fill(&s, 20, 1, t);
    assert_eq!(s.pattern_count().unwrap(), 20);
    let n = s.cap_patterns(10, t).unwrap();
    assert_eq!(n, 11);
    assert_eq!(s.pattern_count().unwrap(), 9);
    // The nine most recent survive.
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(exists(&s, id), i >= 11, "pattern {i}");
    }
}

#[test]
fn cap_prefers_dropping_rare_patterns_over_busy_older_ones() {
    let s = store();
    let t = now();
    let busy = put(&s, OLD_A, t - 5.0 * D, 50);
    let rare_recent = put(&s, OLD_B, t - 60.0, 1);
    let other = put(&s, FRESH, t - 30.0, 1);
    let n = s.cap_patterns(2, t).unwrap();
    assert_eq!(n, 2);
    assert!(exists(&s, &busy), "high-count pattern outlives rare ones");
    assert!(!exists(&s, &rare_recent));
    assert!(!exists(&s, &other));
    assert!(s.split_generation() > 0);
}

#[test]
fn cap_spares_stated_and_recent_error_patterns() {
    let s = store();
    let t = now();
    let stated = put(&s, OLD_A, t - 20.0 * D, 1);
    s.set_state(&stated, Some(&resolved(&stated, t))).unwrap();
    let err_recent = put(&s, "ERROR delta database down", t - 2.0 * D, 1);
    let err_old = put(&s, "ERROR echo disk failure", t - 30.0 * D, 1);
    let plain = fill(&s, 7, 1, t);
    // 10 patterns, max 5 -> evict down to 4, but only 8 are eligible.
    let n = s.cap_patterns(5, t).unwrap();
    assert_eq!(n, 6);
    assert!(exists(&s, &stated));
    assert!(exists(&s, &err_recent));
    assert!(!exists(&s, &err_old), "old ERROR is not protected");
    assert_eq!(s.pattern_count().unwrap(), 10 - n as i64);
    // Eviction is oldest first: any survivor of the plain ones is newer
    // than every evicted one.
    let alive: Vec<bool> = plain.iter().map(|id| exists(&s, id)).collect();
    assert!(alive.windows(2).all(|w| !w[0] || w[1]), "{alive:?}");
}

#[test]
fn cap_zero_or_under_limit_evicts_nothing() {
    let s = store();
    let t = now();
    fill(&s, 6, 1, t);
    assert_eq!(s.cap_patterns(0, t).unwrap(), 0);
    assert_eq!(s.cap_patterns(6, t).unwrap(), 0);
    assert_eq!(s.cap_patterns(100, t).unwrap(), 0);
    assert_eq!(s.pattern_count().unwrap(), 6);
    assert_eq!(s.split_generation(), 0);
}

#[test]
fn deletion_bumps_split_generation() {
    let s = store();
    let t = now();
    fill(&s, 5, 1, t);
    let g = s.split_generation();
    assert!(s.cap_patterns(2, t).unwrap() > 0);
    assert!(s.split_generation() > g);
}

fn help(sub: &str) -> String {
    let out = Proc::new(env!("CARGO_BIN_EXE_starlist"))
        .args([sub, "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn max_patterns_flag_is_documented_for_serve_collect_and_hub() {
    for sub in ["serve", "collect"] {
        let h = help(sub);
        assert!(h.contains("--max-patterns"), "{sub}: {h}");
        assert!(h.contains("10000"), "{sub} default: {h}");
    }
    #[cfg(feature = "hub")]
    assert!(help("hub").contains("--max-patterns"));
}

#[test]
fn config_key_max_patterns_becomes_the_flag() {
    let cli = Command::new("starlist")
        .subcommand(Command::new("serve").arg(Arg::new("max-patterns").long("max-patterns")));
    let serve = cli.find_subcommand("serve").unwrap();
    let out = to_args(&cli, serve, "max-patterns = 5", Path::new("/base")).unwrap();
    let out: Vec<String> = out
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(out, ["--max-patterns", "5"]);
}

fn set_timing(s: &Store, id: &str) {
    s.set_timing(id, Some("recurring")).unwrap();
}

#[test]
fn timing_override_exempts_from_cap_and_expiry() {
    let s = store();
    let t = now();
    let old = t - 40.0 * D;
    let timed = put(&s, OLD_A, old, 1);
    set_timing(&s, &timed);
    let doomed = put(&s, OLD_B, old, 1);
    s.rollup(t - 7.0 * D, Some(t - 14.0 * D)).unwrap();
    assert!(exists(&s, &timed), "timing survives expiry");
    assert!(!exists(&s, &doomed));

    fill(&s, 8, 1, t);
    assert_eq!(s.pattern_count().unwrap(), 9);
    // Max 4 -> down to 3; the timed pattern is the least recent but exempt.
    assert_eq!(s.cap_patterns(4, t).unwrap(), 6);
    assert!(exists(&s, &timed), "timing survives the cap");
    assert!(s.timings().unwrap().contains_key(&timed));
    assert_eq!(s.pattern_count().unwrap(), 3);
}

fn open_arc() -> (Arc<Store>, PathBuf) {
    let p = common::temp_db("starlist-cap");
    (Arc::new(Store::open(&p).unwrap()), p)
}

const USERS: [&str; 4] = ["alice", "bob", "carol", "dave"];

fn user_line(u: &str, ts: Option<&str>) -> String {
    match ts {
        Some(ts) => format!("{ts} WARN login failed for {u}"),
        None => format!("WARN login failed for {u}"),
    }
}

/// Four old lines that merge into one canonical pattern. Returns it.
fn ingest_merged(p: &Pipeline, s: &Store) -> String {
    let lines: Vec<String> = USERS
        .iter()
        .enumerate()
        .map(|(i, u)| user_line(u, Some(&format!("2020-01-01T00:0{i}:00Z"))))
        .collect();
    p.ingest(&lines);
    let aliases = s.aliases().unwrap();
    assert!(!aliases.is_empty(), "lines should have merged");
    let c = aliases.values().next().unwrap().clone();
    assert!(aliases.values().all(|v| *v == c));
    assert!(exists(s, &c));
    c
}

fn template_rows(path: &Path) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM merged_templates", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn cap_and_expiry_of_a_canonical_clear_aliases_and_templates_and_merger_reloads() {
    for by_cap in [false, true] {
        let (s, path) = open_arc();
        let p = Pipeline::with_merging(s.clone(), Redactor::default(), None, Some(3));
        let c = ingest_merged(&p, &s);
        assert!(template_rows(&path) > 0);
        let t = now();
        let gen_before = s.split_generation();
        if by_cap {
            fill(&s, 3, 1, t);
            assert!(s.cap_patterns(2, t).unwrap() > 0);
        } else {
            s.rollup(t - 7.0 * D, Some(t - 14.0 * D)).unwrap();
        }
        assert!(!exists(&s, &c), "by_cap={by_cap}");
        assert!(s.aliases().unwrap().is_empty(), "by_cap={by_cap}");
        assert!(s.merged_ids(&c).unwrap().is_empty());
        assert_eq!(template_rows(&path), 0, "by_cap={by_cap}");
        assert!(s.split_generation() > gen_before);

        // The merger drops what it knew about the deleted pattern.
        p.ingest(&[user_line("alice", None)]);
        let id = id_of(&user_line("alice", None));
        let row = s
            .pattern(&id)
            .unwrap()
            .expect("pattern recreated under its own ID");
        assert_eq!(row.total, 1, "totals start fresh, by_cap={by_cap}");
        assert!(!row.template.is_empty());
        assert!(s.aliases().unwrap().is_empty());
        assert!(
            !exists(&s, &c) || c == id,
            "no stale canonical, by_cap={by_cap}"
        );
    }
}

#[test]
fn manual_split_exempts_from_cap_and_expiry() {
    for by_cap in [false, true] {
        let (s, _) = open_arc();
        let p = Pipeline::with_merging(s.clone(), Redactor::default(), None, Some(3));
        let c = ingest_merged(&p, &s);
        let a = s.aliases().unwrap().keys().next().unwrap().clone();
        let split = s.split_patterns(&c, std::slice::from_ref(&a)).unwrap();
        assert_eq!(split, std::slice::from_ref(&a));
        assert!(s.kept_apart().unwrap().contains(&a));
        let t = now();
        if by_cap {
            fill(&s, 5, 1, t);
            // The split pattern is empty and old, so it would go first.
            assert!(s.cap_patterns(3, t).unwrap() > 0);
        } else {
            s.rollup(t - 7.0 * D, Some(t - 14.0 * D)).unwrap();
        }
        assert!(exists(&s, &a), "split pattern stays, by_cap={by_cap}");
        assert!(s.kept_apart().unwrap().contains(&a));
    }
}

#[test]
fn cap_drops_rare_first_then_least_recent_busy_when_rare_run_out() {
    let s = store();
    let t = now();
    // 3 rare (total <= 10) and 12 busy patterns, busy ones more recent.
    let rare: Vec<String> = (0..3)
        .map(|i| {
            put(
                &s,
                &format!("WARN rare{} thing", "xyz".chars().nth(i).unwrap()),
                t - 100.0 * D,
                1,
            )
        })
        .collect();
    let busy: Vec<String> = fill(&s, 12, 50, t - 60.0);
    assert_eq!(s.pattern_count().unwrap(), 15);
    // Max 10 -> 9 remain: 6 evictions = 3 rare, then the 3 least recent busy.
    assert_eq!(s.cap_patterns(10, t).unwrap(), 6);
    assert_eq!(s.pattern_count().unwrap(), 9);
    for id in &rare {
        assert!(!exists(&s, id), "rare goes first");
    }
    for (i, id) in busy.iter().enumerate() {
        assert_eq!(exists(&s, id), i >= 3, "busy {i}");
    }
}

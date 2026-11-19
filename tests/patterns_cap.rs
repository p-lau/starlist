//! Pattern expiry and the pattern cap.

mod common;

use clap::{Arg, Command};
use common::{id_of, spread, store};
use starlist::config::to_args;
use starlist::store::{StateRow, Store};
use std::path::Path;
use std::process::Command as Proc;

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
    assert!(n >= 1);
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

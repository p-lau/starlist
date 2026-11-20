//! Comparing the failing run with the passing one, and writing the answer.

use crate::ingest::{RunLog, Seen};
use regex::Regex;
use serde_json::{Value, json};
use starlist_core::pattern::{level_rank, truncate};
use std::sync::LazyLock;

/// A line with no level that still reads as a failure, such as cargo's
/// `error[E0308]:`, `test x ... FAILED` or a panic. Ranked like an error.
static FAILURE_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i:^(?:error|fatal)(?:\[[^\]]*\])?:)",
        r"|\bFAILED\b|\bFAIL\b|\bpanicked at\b",
        r"|(?i:\b(?:tests?|build|command|job|step|assertion|compilation|check|process) failed\b)",
        r"|(?i:\bfailed (?:to|with)\b)",
        r"|(?i:\bexit (?:code|status) [1-9])",
    ))
    .unwrap()
});

const TEMPLATE_CHARS: usize = 240;
const BRIEF_TEMPLATE_CHARS: usize = 160;
const SAMPLE_CHARS: usize = 1500;
const BRIEF_SAMPLE_CHARS: usize = 400;
const TRIMMED_SAMPLE_CHARS: usize = 200;
/// In brief output only the top findings carry a sample.
const BRIEF_SAMPLES: usize = 3;
/// Same answer budgets as the MCP tools: roughly 2k tokens brief, 4k full.
pub const DEFAULT_MAX_CHARS: usize = 8000;
pub const DEFAULT_MAX_CHARS_FULL: usize = 16000;
pub const MIN_MAX_CHARS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// In the failing run only.
    New,
    /// At least twice as often as in the passing run.
    Grew,
    /// In the passing run only.
    Stopped,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::New => "new",
            Kind::Grew => "grew",
            Kind::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub kind: Kind,
    pub pattern_id: String,
    pub seen: Seen,
    /// Events in the failing run.
    pub count: usize,
    /// Events in the passing run.
    pub before: usize,
    /// Level rank used for ordering: the level, or error for a line that reads as a failure.
    pub severity: u8,
}

fn severity(s: &Seen) -> u8 {
    let rank = level_rank(s.level);
    if s.level.is_none() && FAILURE_WORDS.is_match(s.template.lines().next().unwrap_or("")) {
        3
    } else {
        rank
    }
}

/// What changed between the runs, most telling first: by severity, then new
/// before grew before stopped, then by where it first appears in the log.
/// Without a passing run every pattern counts as new.
pub fn compare(failing: &RunLog, passing: Option<&RunLog>, min_severity: u8) -> Vec<Finding> {
    let mut out = vec![];
    for (id, s) in &failing.patterns {
        let before = passing
            .and_then(|p| p.patterns.get(id))
            .map_or(0, |b| b.count);
        let kind = if before == 0 {
            Kind::New
        } else if s.count >= 2 * before && s.count - before >= 2 {
            Kind::Grew
        } else {
            continue;
        };
        out.push(Finding {
            kind,
            pattern_id: id.clone(),
            severity: severity(s),
            count: s.count,
            before,
            seen: s.clone(),
        });
    }
    if let Some(p) = passing {
        for (id, s) in &p.patterns {
            if !failing.patterns.contains_key(id) {
                out.push(Finding {
                    kind: Kind::Stopped,
                    pattern_id: id.clone(),
                    severity: severity(s),
                    count: 0,
                    before: s.count,
                    seen: s.clone(),
                });
            }
        }
    }
    out.retain(|f| f.severity >= min_severity);
    out.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(a.kind.cmp(&b.kind))
            .then(a.seen.line.cmp(&b.seen.line))
            .then(a.pattern_id.cmp(&b.pattern_id))
    });
    out
}

/// How much to say.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    pub full: bool,
    pub limit: usize,
    pub max_chars: usize,
}

fn finding_json(f: &Finding, i: usize, shape: Shape, has_baseline: bool) -> Value {
    let s = &f.seen;
    let with_sample = shape.full || i < BRIEF_SAMPLES;
    let mut v = json!({
        "kind": f.kind.as_str(),
        "pattern_id": f.pattern_id,
        "level": s.level,
        "template": truncate(&s.template, if shape.full { TEMPLATE_CHARS } else { BRIEF_TEMPLATE_CHARS }),
        "count": f.count,
        "before": f.before,
        "location": s.location,
        "step": s.section,
    });
    // The line is in the failing log, except for a pattern that stopped.
    let key = if f.kind == Kind::Stopped {
        "passing_line"
    } else {
        "line"
    };
    v[key] = json!(s.line);
    if !has_baseline {
        // Nothing to compare with, so "new" and "before" would say nothing.
        let o = v.as_object_mut().unwrap();
        o.remove("kind");
        o.remove("before");
    }
    if s.level.is_none() && f.severity == 3 {
        v["level"] = json!("ERROR?");
    }
    // A sample that only repeats the template adds nothing.
    if with_sample && s.sample != s.template {
        v["sample"] = json!(truncate(
            &s.sample,
            if shape.full {
                SAMPLE_CHARS
            } else {
                BRIEF_SAMPLE_CHARS
            }
        ));
    }
    compact(v)
}

/// Drop nulls, so absent fields cost nothing.
fn compact(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, compact(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(compact).collect()),
        other => other,
    }
}

fn json_len(v: &Value) -> usize {
    serde_json::to_string(v).map_or(0, |s| s.chars().count())
}

fn summary(failing: &RunLog, passing: Option<&RunLog>, findings: &[Finding]) -> String {
    let n = |k: Kind| findings.iter().filter(|f| f.kind == k).count();
    let first = findings.iter().find(|f| f.kind != Kind::Stopped);
    let lead = match first {
        Some(f) => format!(
            " Start with line {}: {}",
            f.seen.line,
            truncate(f.seen.template.lines().next().unwrap_or(""), 120)
        ),
        None => String::new(),
    };
    let text = match passing {
        Some(_) if findings.is_empty() => {
            "Nothing in the failing run stands out from the passing run.".to_string()
        }
        Some(_) => format!(
            "{} new, {} grew, {} stopped compared with the passing run.{lead}",
            n(Kind::New),
            n(Kind::Grew),
            n(Kind::Stopped),
        ),
        None if findings.is_empty() => {
            "No passing run to compare with, and no errors or warnings in this log.".to_string()
        }
        None => format!(
            "No passing run to compare with: {} error and warning patterns in this log.{lead}",
            findings.len()
        ),
    };
    if failing.events == 0 {
        text + " The failing log had no events."
    } else {
        text
    }
}

/// The answer as JSON, kept within `shape.max_chars`: samples are shortened,
/// then dropped, then the last findings are left out, and it says so.
pub fn to_json(
    failing: &RunLog,
    passing: Option<&RunLog>,
    findings: &[Finding],
    shape: Shape,
) -> Value {
    let head = json!({
        "summary": summary(failing, passing, findings),
        "events": {
            "failing": failing.events,
            "passing": passing.map(|p| p.events),
        },
        "patterns": {
            "failing": failing.patterns.len(),
            "passing": passing.map(|p| p.patterns.len()),
        },
    });
    let total = findings.len();
    let mut items: Vec<Value> = findings
        .iter()
        .take(shape.limit)
        .enumerate()
        .map(|(i, f)| finding_json(f, i, shape, passing.is_some()))
        .collect();
    let page_len = items.len();
    let build = |items: &[Value], notes: &[String]| {
        let mut v = head.clone();
        v["findings"] = json!(items);
        if items.len() < total {
            v["remaining"] = json!(total - items.len());
        }
        if !notes.is_empty() {
            v["trimmed"] = json!(notes.join(" "));
        }
        compact(v)
    };
    let fits = |v: &Value| json_len(v) <= shape.max_chars;

    let mut notes: Vec<String> = vec![];
    let v = build(&items, &notes);
    if fits(&v) {
        return v;
    }
    for step in [Some(TRIMMED_SAMPLE_CHARS), None] {
        for it in items.iter_mut() {
            let Some(o) = it.as_object_mut() else {
                continue;
            };
            match step {
                Some(n) => {
                    if let Some(Value::String(s)) = o.get("sample") {
                        let t = truncate(s, n);
                        o.insert("sample".into(), json!(t));
                    }
                }
                None => {
                    o.remove("sample");
                }
            }
        }
        notes = vec![match step {
            Some(_) => "Samples shortened to fit max_chars.".to_string(),
            None => {
                "Samples removed to fit max_chars; the line numbers point into the log.".to_string()
            }
        }];
        let v = build(&items, &notes);
        if fits(&v) {
            return v;
        }
    }
    while items.len() > 1 {
        items.pop();
        let mut n = notes.clone();
        n.push(format!(
            "{} more left out to fit max_chars.",
            page_len - items.len()
        ));
        let v = build(&items, &n);
        if fits(&v) {
            return v;
        }
    }
    build(&items, &notes)
}

/// The same answer as Markdown, for a job summary or a pull request comment.
pub fn to_markdown(v: &Value) -> String {
    let mut out = String::from("### starlist-ci\n\n");
    out += v["summary"].as_str().unwrap_or("");
    out += "\n\n";
    let findings = v["findings"].as_array().cloned().unwrap_or_default();
    let compared = v["events"]["passing"].is_u64();
    if !findings.is_empty() {
        out += if compared {
            "| | Level | Pattern | Failing | Passing | Where |\n|---|---|---|---|---|---|\n"
        } else {
            "| Level | Pattern | Count | Where |\n|---|---|---|---|\n"
        };
    }
    for f in &findings {
        let kind = f["kind"].as_str().unwrap_or("");
        let level = f["level"].as_str().unwrap_or("");
        let template = cell(f["template"].as_str().unwrap_or(""));
        let mut place = vec![];
        if let Some(l) = f["line"].as_u64() {
            place.push(format!("line {l}"));
        }
        if let Some(l) = f["passing_line"].as_u64() {
            place.push(format!("passing log line {l}"));
        }
        if let Some(s) = f["step"].as_str() {
            place.push(format!("step {}", cell(s)));
        }
        if let Some(l) = f["location"].as_str() {
            place.push(format!("`{}`", cell(l)));
        }
        let count = f["count"].as_u64().unwrap_or(0);
        let place = place.join(", ");
        out += &if compared {
            let before = f["before"].as_u64().unwrap_or(0);
            format!("| {kind} | {level} | `{template}` | {count} | {before} | {place} |\n")
        } else {
            format!("| {level} | `{template}` | {count} | {place} |\n")
        };
    }
    let samples: Vec<&Value> = findings
        .iter()
        .filter(|f| f["sample"].is_string())
        .collect();
    if !samples.is_empty() {
        out += "\n";
    }
    for f in samples {
        let where_ = f["line"]
            .as_u64()
            .map(|l| format!("line {l}"))
            .or_else(|| {
                f["passing_line"]
                    .as_u64()
                    .map(|l| format!("passing log line {l}"))
            })
            .unwrap_or_default();
        out += &format!(
            "<details><summary>{}, {where_}</summary>\n\n```text\n{}\n```\n\n</details>\n",
            html(&truncate(f["template"].as_str().unwrap_or(""), 80)),
            f["sample"].as_str().unwrap_or("").replace("```", "'''"),
        );
    }
    let mut notes = vec![];
    if let Some(n) = v["remaining"].as_u64() {
        notes.push(format!("{n} more not shown."));
    }
    if let Some(t) = v["trimmed"].as_str() {
        notes.push(t.to_string());
    }
    if !notes.is_empty() {
        out += &format!("\n_{}_\n", notes.join(" "));
    }
    out
}

/// Text that is safe inside a table cell and inline code.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('`', "'").replace('\n', " ")
}

/// Text that is safe inside an HTML element, such as `<NUM>` in a summary line.
fn html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn seen(level: Option<&'static str>, template: &str, count: usize, line: usize) -> Seen {
        Seen {
            template: template.into(),
            level,
            count,
            line,
            section: None,
            sample: template.into(),
            location: None,
        }
    }

    fn run(items: Vec<(&str, Seen)>) -> RunLog {
        let events = items.iter().map(|(_, s)| s.count).sum();
        RunLog {
            patterns: items
                .into_iter()
                .map(|(id, s)| (id.to_string(), s))
                .collect::<HashMap<_, _>>(),
            events,
            ..Default::default()
        }
    }

    fn shape(full: bool, limit: usize, max_chars: usize) -> Shape {
        Shape {
            full,
            limit,
            max_chars,
        }
    }

    fn ids(f: &[Finding]) -> Vec<&str> {
        f.iter().map(|f| f.pattern_id.as_str()).collect()
    }

    #[test]
    fn orders_by_severity_then_kind_then_line() {
        let failing = run(vec![
            ("new-err-late", seen(Some("ERROR"), "late", 1, 10)),
            ("new-err-early", seen(Some("ERROR"), "early", 1, 5)),
            ("new-warn", seen(Some("WARN"), "warn", 1, 1)),
            ("grew-err", seen(Some("ERROR"), "grew", 6, 2)),
            ("same", seen(Some("ERROR"), "same", 3, 4)),
        ]);
        let passing = run(vec![
            ("grew-err", seen(Some("ERROR"), "grew", 2, 2)),
            ("same", seen(Some("ERROR"), "same", 3, 4)),
            ("gone-err", seen(Some("ERROR"), "gone", 1, 3)),
        ]);
        let f = compare(&failing, Some(&passing), 0);
        assert_eq!(
            ids(&f),
            [
                "new-err-early",
                "new-err-late",
                "grew-err",
                "gone-err",
                "new-warn"
            ]
        );
        let kinds: Vec<_> = f.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            [Kind::New, Kind::New, Kind::Grew, Kind::Stopped, Kind::New]
        );
        assert_eq!((f[2].count, f[2].before), (6, 2));
        assert_eq!((f[3].count, f[3].before), (0, 1));
    }

    #[test]
    fn grew_needs_double_and_two_more() {
        let passing = run(vec![
            ("a", seen(Some("ERROR"), "a", 1, 1)),
            ("b", seen(Some("ERROR"), "b", 2, 2)),
            ("c", seen(Some("ERROR"), "c", 2, 3)),
        ]);
        let failing = run(vec![
            ("a", seen(Some("ERROR"), "a", 2, 1)),
            ("b", seen(Some("ERROR"), "b", 3, 2)),
            ("c", seen(Some("ERROR"), "c", 4, 3)),
        ]);
        assert_eq!(ids(&compare(&failing, Some(&passing), 0)), ["c"]);
    }

    #[test]
    fn min_severity_filters_findings() {
        let failing = run(vec![
            ("w", seen(Some("WARN"), "w", 1, 1)),
            ("e", seen(Some("ERROR"), "e", 1, 2)),
        ]);
        assert_eq!(ids(&compare(&failing, None, 3)), ["e"]);
    }

    #[test]
    fn levelless_failure_lines_rank_as_errors() {
        let failing = run(vec![
            ("p", seen(None, "error[E0308]: mismatched types", 1, 9)),
            ("q", seen(None, "compiling foo", 1, 1)),
        ]);
        let f = compare(&failing, None, 0);
        assert_eq!(ids(&f), ["p", "q"]);
        assert_eq!((f[0].severity, f[1].severity), (3, 1));
    }

    #[test]
    fn identical_runs_have_no_findings() {
        let mk = || run(vec![("a", seen(Some("ERROR"), "boom", 4, 3))]);
        let (failing, passing) = (mk(), mk());
        let f = compare(&failing, Some(&passing), 0);
        assert!(f.is_empty());
        let v = to_json(&failing, Some(&passing), &f, shape(false, 10, 8000));
        assert_eq!(
            v["summary"],
            "Nothing in the failing run stands out from the passing run."
        );
        assert_eq!(v["findings"], json!([]));
        assert!(v.get("remaining").is_none() && v.get("trimmed").is_none());
    }

    #[test]
    fn without_a_passing_run_every_pattern_is_new() {
        let failing = run(vec![("a", seen(Some("ERROR"), "boom", 4, 3))]);
        let f = compare(&failing, None, 0);
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].kind, f[0].count, f[0].before), (Kind::New, 4, 0));
        let empty = run(vec![]);
        assert!(compare(&empty, None, 0).is_empty());
    }

    #[test]
    fn json_has_the_documented_fields() {
        let mut s = seen(Some("ERROR"), "db down <NUM>", 3, 12);
        s.section = Some("Run tests".into());
        s.location = Some("src/db.rs:10".into());
        s.sample = "ERROR db down 5\n  at db.rs:10".into();
        let failing = run(vec![("a", s)]);
        let passing = run(vec![("z", seen(Some("ERROR"), "old", 2, 7))]);
        let f = compare(&failing, Some(&passing), 0);
        let v = to_json(&failing, Some(&passing), &f, shape(true, 10, 16000));
        assert_eq!(v["events"], json!({"failing": 3, "passing": 2}));
        assert_eq!(v["patterns"], json!({"failing": 1, "passing": 1}));
        assert!(
            v["summary"]
                .as_str()
                .unwrap()
                .starts_with("1 new, 0 grew, 1 stopped compared with the passing run.")
        );
        let items = v["findings"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        let a = &items[0];
        assert_eq!(a["kind"], "new");
        assert_eq!(a["pattern_id"], "a");
        assert_eq!(a["level"], "ERROR");
        assert_eq!(a["template"], "db down <NUM>");
        assert_eq!(
            (a["count"].as_u64(), a["before"].as_u64()),
            (Some(3), Some(0))
        );
        assert_eq!(a["line"], 12);
        assert_eq!(a["step"], "Run tests");
        assert_eq!(a["location"], "src/db.rs:10");
        assert_eq!(a["sample"], "ERROR db down 5\n  at db.rs:10");
        // A stopped pattern points into the passing log and has no null fields.
        let z = &items[1];
        assert_eq!(z["kind"], "stopped");
        assert_eq!(z["passing_line"], 7);
        assert!(z.get("line").is_none() && z.get("step").is_none());
        assert!(z.get("location").is_none() && z.get("sample").is_none());
        assert_eq!(
            (z["count"].as_u64(), z["before"].as_u64()),
            (Some(0), Some(2))
        );
    }

    #[test]
    fn json_without_a_baseline_drops_kind_and_before() {
        let failing = run(vec![
            ("a", seen(None, "error: boom", 1, 1)),
            ("b", seen(Some("WARN"), "slow", 1, 2)),
        ]);
        let f = compare(&failing, None, 0);
        let v = to_json(&failing, None, &f, shape(false, 10, 8000));
        assert!(v["events"].get("passing").is_none());
        let a = &v["findings"][0];
        assert!(a.get("kind").is_none() && a.get("before").is_none());
        assert_eq!(a["level"], "ERROR?");
        assert_eq!(v["findings"][1]["level"], "WARN");
        assert!(
            v["summary"]
                .as_str()
                .unwrap()
                .starts_with("No passing run to compare with: 2 error and warning patterns")
        );
    }

    #[test]
    fn json_limit_reports_remaining() {
        let items = ["a", "b", "c", "d", "e"]
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id, seen(Some("ERROR"), "t", 1, i + 1)))
            .collect();
        let failing = run(items);
        let f = compare(&failing, None, 0);
        let v = to_json(&failing, None, &f, shape(false, 2, 8000));
        assert_eq!(v["findings"].as_array().unwrap().len(), 2);
        assert_eq!(v["remaining"], 3);
        assert!(v.get("trimmed").is_none());
    }

    fn big(n: usize, sample: usize) -> (RunLog, Vec<Finding>) {
        let names: Vec<String> = (0..n).map(|i| format!("p{i:03}")).collect();
        let items = names
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let mut s = seen(Some("ERROR"), &format!("failure number {i}"), 1, i + 1);
                s.sample = format!("{}\n{}", s.template, "x".repeat(sample));
                (id.as_str(), s)
            })
            .collect();
        let failing = run(items);
        let f = compare(&failing, None, 0);
        (failing, f)
    }

    #[test]
    fn trims_samples_first() {
        let (failing, f) = big(3, 1000);
        let v = to_json(&failing, None, &f, shape(true, 10, 2000));
        assert!(json_len(&v) <= 2000);
        assert_eq!(v["findings"].as_array().unwrap().len(), 3);
        assert!(v.get("remaining").is_none());
        assert_eq!(v["trimmed"], "Samples shortened to fit max_chars.");
        for it in v["findings"].as_array().unwrap() {
            assert!(it["sample"].as_str().unwrap().chars().count() <= 210);
        }
    }

    #[test]
    fn drops_samples_then_findings_and_says_so() {
        let (failing, f) = big(40, 1000);
        let max = 3000;
        let v = to_json(&failing, None, &f, shape(true, 100, max));
        assert!(json_len(&v) <= max, "{}", json_len(&v));
        let shown = v["findings"].as_array().unwrap();
        assert!(shown.len() < 40 && !shown.is_empty());
        assert!(shown.iter().all(|it| it.get("sample").is_none()));
        assert_eq!(v["remaining"], 40 - shown.len());
        let note = v["trimmed"].as_str().unwrap();
        assert!(note.contains("Samples removed to fit max_chars"), "{note}");
        let left_out = format!("{} more left out to fit max_chars.", 40 - shown.len());
        assert!(note.contains(&left_out), "{note}");
        // The most telling findings are the ones kept.
        assert_eq!(shown[0]["pattern_id"], "p000");
    }

    #[test]
    fn brief_output_samples_only_the_top_findings() {
        let (failing, f) = big(5, 50);
        let v = to_json(&failing, None, &f, shape(false, 10, 16000));
        let with: Vec<bool> = v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|it| it.get("sample").is_some())
            .collect();
        assert_eq!(with, [true, true, true, false, false]);
    }

    #[test]
    fn markdown_table_for_a_comparison() {
        let mut s = seen(Some("ERROR"), "bad | thing", 3, 12);
        s.section = Some("Run tests".into());
        s.location = Some("src/db.rs:10".into());
        s.sample = "bad | thing\n```boom```".into();
        let failing = run(vec![("a", s)]);
        let passing = run(vec![("z", seen(Some("WARN"), "old", 2, 7))]);
        let f = compare(&failing, Some(&passing), 0);
        let v = to_json(&failing, Some(&passing), &f, shape(true, 10, 16000));
        let md = to_markdown(&v);
        assert!(md.starts_with("### starlist-ci\n\n1 new, 0 grew, 1 stopped"));
        assert!(md.contains("| | Level | Pattern | Failing | Passing | Where |"));
        assert!(md.contains(
            "| new | ERROR | `bad \\| thing` | 3 | 0 | line 12, step Run tests, `src/db.rs:10` |"
        ));
        assert!(md.contains("| stopped | WARN | `old` | 0 | 2 | passing log line 7 |"));
        assert!(md.contains("<details><summary>bad | thing, line 12</summary>"));
        assert!(md.contains("'''boom'''") && !md.contains("```boom"));
    }

    #[test]
    fn markdown_without_a_baseline_and_with_notes() {
        let (failing, f) = big(40, 1000);
        let v = to_json(&failing, None, &f, shape(true, 100, 3000));
        let md = to_markdown(&v);
        assert!(md.contains("| Level | Pattern | Count | Where |"));
        assert!(!md.contains("Passing"));
        assert!(md.contains("| ERROR | `failure number 0` | 1 | line 1 |"));
        let tail = md.trim_end().lines().last().unwrap();
        assert!(tail.starts_with('_') && tail.ends_with('_'), "{tail}");
        assert!(tail.contains(&format!("{} more not shown.", v["remaining"])));
        assert!(tail.contains("Samples removed to fit max_chars"));
    }

    #[test]
    fn markdown_for_an_empty_answer_is_just_the_summary() {
        let empty = run(vec![]);
        let v = to_json(&empty, None, &[], shape(false, 10, 8000));
        assert_eq!(
            to_markdown(&v),
            "### starlist-ci\n\nNo passing run to compare with, and no errors or warnings in this log. The failing log had no events.\n\n"
        );
    }
}

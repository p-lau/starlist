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

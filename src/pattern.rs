//! Turning raw log lines into events with a stable pattern ID.
//!
//! A pattern is the log message with its variable parts (numbers, IDs, UUIDs,
//! quoted values, timestamps) masked out. The ID is a hash of that template, so
//! the same kind of log line gets the same ID across restarts and across hosts.

use crate::redact::Redactor;
use chrono::{DateTime, NaiveDateTime};
use regex::{Captures, Regex};
use std::sync::LazyLock;

pub const LEVELS: [&str; 5] = ["DEBUG", "INFO", "WARN", "ERROR", "FATAL"];

pub fn normalize_level(raw: &str) -> Option<&'static str> {
    Some(match raw.to_ascii_uppercase().as_str() {
        "TRACE" | "DEBUG" | "DBG" => "DEBUG",
        "INFO" | "NOTICE" | "INF" => "INFO",
        "WARN" | "WARNING" | "WRN" => "WARN",
        "ERROR" | "ERR" | "SEVERE" => "ERROR",
        "CRITICAL" | "CRIT" | "FATAL" | "PANIC" | "EMERG" | "ALERT" => "FATAL",
        _ => return None,
    })
}

pub fn level_rank(level: Option<&str>) -> u8 {
    match level {
        Some("DEBUG") => 0,
        Some("INFO") | None => 1,
        Some("WARN") => 2,
        Some("ERROR") => 3,
        Some("FATAL") => 4,
        _ => 1,
    }
}

static TS_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[?(\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?)\]?[\s,|]*")
        .unwrap()
});
static LEVEL_KV: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\b(?:level|lvl|severity)[=:]\s*"?([A-Za-z]+)"#).unwrap());
static LEVEL_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(TRACE|DEBUG|INFO|NOTICE|WARN|WARNING|ERROR|ERR|SEVERE|CRITICAL|FATAL|PANIC)\b")
        .unwrap()
});
static CONTINUATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:\s+\S|Traceback \(most recent call last\)|Caused by:|During handling of the above exception|The above exception was the direct cause|\.\.\. \d+ (?:more|common frames omitted)|[A-Za-z_][\w.$]*(?:Error|Exception|Exit|Interrupt|Warning)(?::|\s*$))",
    )
    .unwrap()
});
static EXC_TYPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z_][\w.$]*(?:Error|Exception|Exit|Interrupt|Fault))(?::|\s*$)").unwrap()
});

static MASK_UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
        .unwrap()
});
static MASK_TS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?")
        .unwrap()
});
static MASK_DQ: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""(?:[^"\\]|\\.)*""#).unwrap());
static MASK_HEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:0x[0-9a-fA-F]+|[0-9a-fA-F]{12,})\b").unwrap());
static MASK_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9_-]*\d[A-Za-z0-9_-]*(?:\.[A-Za-z0-9_-]+)*").unwrap());
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[-+]?\d+(?:\.\d+)?(?:[A-Za-z%]{1,3})?$").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

const MAX_TEMPLATE_CHARS: usize = 400;
const MAX_EVENT_LINES: usize = 200;

static HAS_ZONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:[Zz]|[+-]\d{2}:?\d{2})$").unwrap());

/// Whether a timestamp string carries its own zone ("Z" or an offset).
pub fn has_zone(raw: &str) -> bool {
    HAS_ZONE.is_match(raw.trim())
}

/// Parse "UTC", "Z", "+02:00", "-0800" or "+5" into seconds east of UTC.
pub fn parse_utc_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("utc") || s.eq_ignore_ascii_case("z") {
        return Some(0);
    }
    let (sign, rest) = match s.chars().next()? {
        '+' => (1, &s[1..]),
        '-' => (-1, &s[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|c| *c != ':').collect();
    if digits.is_empty() || digits.len() > 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m) = if digits.len() <= 2 {
        (digits.parse::<i32>().ok()?, 0)
    } else {
        let split = digits.len() - 2;
        (
            digits[..split].parse::<i32>().ok()?,
            digits[split..].parse::<i32>().ok()?,
        )
    };
    (h <= 14 && m < 60).then_some(sign * (h * 3600 + m * 60))
}

/// Parse a timestamp string into epoch seconds. Timestamps without a zone are
/// read as UTC (the collector can shift them with --log-tz).
pub fn parse_timestamp(raw: &str) -> Option<f64> {
    let mut s = raw.trim().replace(',', ".");
    if s.len() > 10 && s.as_bytes()[10] == b' ' {
        s.replace_range(10..11, "T");
    }
    if s.ends_with('Z') || s.ends_with('z') {
        s.truncate(s.len() - 1);
        s.push_str("+00:00");
    }
    // +0000 -> +00:00
    let bytes = s.as_bytes();
    if bytes.len() > 5 {
        let tail = &s[s.len() - 5..];
        if (tail.starts_with('+') || tail.starts_with('-'))
            && tail[1..].chars().all(|c| c.is_ascii_digit())
        {
            s.insert(s.len() - 2, ':');
        }
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
        return Some(dt.timestamp_micros() as f64 / 1e6);
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(dt.and_utc().timestamp_micros() as f64 / 1e6);
    }
    None
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    truncate_chars(s, max)
}

/// Mask the variable parts of a message so similar lines share a template.
pub fn mask(message: &str) -> String {
    let s = MASK_UUID.replace_all(message, "<UUID>");
    let s = MASK_TS.replace_all(&s, "<TS>");
    let s = MASK_DQ.replace_all(&s, "\"<STR>\"");
    let s = MASK_HEX.replace_all(&s, "<HEX>");
    let s = MASK_TOKEN.replace_all(&s, |c: &Captures| {
        if NUMBER.is_match(&c[0]) {
            "<NUM>"
        } else {
            "<ID>"
        }
        .to_string()
    });
    let s = SPACES.replace_all(s.trim(), " ");
    truncate_chars(&s, MAX_TEMPLATE_CHARS)
}

pub fn pattern_id(template: &str) -> String {
    let hex = sha1_smol::Sha1::from(template.as_bytes())
        .digest()
        .to_string();
    format!("p{}", &hex[..10])
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Epoch seconds, from the log line when it has a timestamp.
    pub ts: Option<f64>,
    /// The timestamp came from the log without a zone, so it was read as UTC.
    pub ts_naive: bool,
    pub level: Option<&'static str>,
    /// Redacted first line, without the timestamp prefix.
    pub head: String,
    /// Redacted full event, including stack trace lines.
    pub body: String,
    pub template: String,
    pub pattern_id: String,
    pub exc_type: Option<String>,
    /// Top app frame of the stack trace, `file:line in function`.
    pub location: Option<String>,
    /// Request or trace ID, found before masking.
    pub request_id: Option<String>,
    /// Numeric fields read from the line (`--metric`).
    pub metrics: Vec<(String, f64)>,
    /// Keep this event out of the raw buffer (sent by a collector with `--no-raw`).
    pub no_raw: bool,
}

fn detect_level(message: &str) -> Option<&'static str> {
    let prefix: String = message.chars().take(100).collect();
    if let Some(c) = LEVEL_KV.captures(&prefix)
        && let Some(l) = normalize_level(&c[1])
    {
        return Some(l);
    }
    LEVEL_WORD
        .captures(&prefix)
        .and_then(|c| normalize_level(&c[1]))
}

fn exception_type<'a>(lines: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut found = None;
    for line in lines {
        let t = line.trim();
        if t.starts_with("File ") || t.starts_with("at ") {
            continue;
        }
        if let Some(c) = EXC_TYPE.captures(t) {
            found = Some(c[1].to_string());
        }
    }
    found
}

fn json_str(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(x) = v.get(*k) {
            match x {
                serde_json::Value::String(s) => return Some(s.clone()),
                serde_json::Value::Null => {}
                other => return Some(other.to_string()),
            }
        }
    }
    None
}

/// The event time and whether it lacked a zone.
fn json_ts(v: &serde_json::Value) -> Option<(f64, bool)> {
    for k in ["timestamp", "time", "ts", "@timestamp", "t", "datetime"] {
        match v.get(k) {
            Some(serde_json::Value::String(s)) => {
                if let Some(t) = parse_timestamp(s) {
                    return Some((t, !has_zone(s)));
                }
            }
            Some(serde_json::Value::Number(n)) => {
                let f = n.as_f64()?;
                // Milliseconds vs seconds.
                return Some((if f > 1e11 { f / 1000.0 } else { f }, false));
            }
            _ => {}
        }
    }
    None
}

/// Build an event from one complete log event (one or more lines).
/// `ts_hint` and `level_hint` come from callers that know them already, such as
/// an in-process logging hook.
pub fn parse_event(
    text: &str,
    redactor: &Redactor,
    ts_hint: Option<f64>,
    level_hint: Option<&str>,
) -> Event {
    parse_event_with(
        text,
        redactor,
        ts_hint,
        level_hint,
        crate::request::default_keys(),
    )
}

/// `parse_event` with a custom set of request ID keys.
pub fn parse_event_with(
    text: &str,
    redactor: &Redactor,
    ts_hint: Option<f64>,
    level_hint: Option<&str>,
    request_keys: &crate::request::RequestKeys,
) -> Event {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("").trim_end();
    let rest: Vec<&str> = lines.collect();

    let (ts, level, message, extra_body) = match first.trim_start().starts_with('{') {
        true => match serde_json::from_str::<serde_json::Value>(first) {
            Ok(v) if v.is_object() => {
                let level = json_str(&v, &["level", "severity", "levelname", "lvl", "log.level"])
                    .and_then(|l| normalize_level(&l));
                let msg = json_str(&v, &["message", "msg", "event", "text"])
                    .unwrap_or_else(|| first.to_string());
                let exc = json_str(
                    &v,
                    &[
                        "exception",
                        "exc_info",
                        "stack",
                        "stack_trace",
                        "stacktrace",
                        "error",
                        "err",
                    ],
                );
                let msg = format!("{} {}", level.unwrap_or("-"), msg);
                (json_ts(&v), level, msg, exc)
            }
            _ => (None, None, first.to_string(), None),
        },
        false => {
            let (ts, msg) = match TS_PREFIX.captures(first) {
                Some(c) => (
                    parse_timestamp(&c[1]).map(|t| (t, !has_zone(&c[1]))),
                    first[c.get(0).unwrap().end()..].to_string(),
                ),
                None => (None, first.to_string()),
            };
            let level = detect_level(&msg);
            (ts, level, msg, None)
        }
    };

    let level = level_hint.and_then(normalize_level).or(level);
    let ts_naive = ts_hint.is_none() && ts.is_some_and(|(_, naive)| naive);
    let ts = ts_hint.or(ts.map(|(t, _)| t));

    let mut body_lines: Vec<String> = vec![message.clone()];
    if let Some(e) = &extra_body {
        body_lines.extend(e.lines().map(str::to_string));
    }
    body_lines.extend(rest.iter().map(|l| l.to_string()));
    let exc_type = exception_type(body_lines.iter().skip(1).map(String::as_str));

    let head = redactor.redact(&message);
    let body = redactor.redact(&body_lines.join("\n"));
    // An ID that redaction would change is treated as a secret, not a request ID.
    let request_id = request_keys
        .find(text)
        .filter(|id| redactor.redact(id) == *id);
    // Keep request IDs out of the template, whatever they look like.
    let mut template = match &request_id {
        Some(id) => mask(&head.replace(id.as_str(), "<REQ>")),
        None => mask(&head),
    };
    if let Some(e) = &exc_type {
        template = format!("{template} | {e}");
    }
    let pattern_id = pattern_id(&template);
    let location = crate::location::app_frame(&body).map(|f| f.display(&[]));
    Event {
        ts,
        ts_naive,
        level,
        head,
        body,
        template,
        pattern_id,
        exc_type,
        location,
        request_id,
        metrics: vec![],
        no_raw: false,
    }
}

/// Groups physical lines into log events, so a stack trace stays attached to
/// the log line that produced it.
#[derive(Default)]
pub struct Assembler {
    pending: Vec<String>,
}

impl Assembler {
    pub fn is_continuation(line: &str) -> bool {
        CONTINUATION.is_match(line)
    }

    /// Feed one line; returns a finished event's text when this line starts a new one.
    pub fn feed(&mut self, line: &str) -> Option<String> {
        let line = line.trim_end_matches(['\n', '\r']);
        if line.trim().is_empty() {
            return None;
        }
        if !self.pending.is_empty() && Self::is_continuation(line) {
            if self.pending.len() < MAX_EVENT_LINES {
                self.pending.push(line.to_string());
            }
            return None;
        }
        let done = self.flush();
        self.pending.push(line.to_string());
        done
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn flush(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.pending).join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> Event {
        parse_event(s, &Redactor::default(), None, None)
    }

    #[test]
    fn same_shape_same_pattern() {
        let a = ev("2026-11-06T10:00:00Z ERROR payment 8812 failed for order ord_42a after 1200ms");
        let b = ev("2026-11-06T10:05:01.123Z ERROR payment 9 failed for order ord_7f after 35ms");
        assert_eq!(a.template, b.template);
        assert_eq!(a.pattern_id, b.pattern_id);
        assert_eq!(a.level, Some("ERROR"));
        assert!(a.ts.is_some());
    }

    #[test]
    fn different_messages_differ() {
        let a = ev("ERROR payment failed");
        let b = ev("ERROR refund failed");
        assert_ne!(a.pattern_id, b.pattern_id);
    }

    #[test]
    fn ids_are_stable() {
        // Pinned so an accidental change to masking or hashing is caught:
        // agents keep pattern IDs across restarts.
        let e = ev("WARN cache miss for key 123");
        assert_eq!(e.template, "WARN cache miss for key <NUM>");
        assert_eq!(e.pattern_id, pattern_id("WARN cache miss for key <NUM>"));
    }

    #[test]
    fn masks_uuids_quotes_and_hosts() {
        let t = mask(
            r#"user "alice smith" req 3f2b1c9e-1d2a-4c3b-9a8f-0b1c2d3e4f5a on db-3.internal took 12.5ms"#,
        );
        assert_eq!(t, r#"user "<STR>" req <UUID> on <ID> took <NUM>"#);
    }

    #[test]
    fn groups_python_traceback_and_names_exception() {
        let mut asm = Assembler::default();
        let lines = [
            "2026-11-06 10:00:00,123 ERROR checkout failed for cart 99",
            "Traceback (most recent call last):",
            "  File \"app.py\", line 10, in checkout",
            "    total = cart['currency']",
            "KeyError: 'currency'",
            "2026-11-06 10:00:01,000 INFO next request",
        ];
        let mut events = vec![];
        for l in lines {
            if let Some(e) = asm.feed(l) {
                events.push(e);
            }
        }
        events.extend(asm.flush());
        assert_eq!(events.len(), 2);
        let e = ev(&events[0]);
        assert_eq!(e.exc_type.as_deref(), Some("KeyError"));
        assert!(e.template.ends_with("| KeyError"), "{}", e.template);
        assert!(e.body.contains("Traceback"));
    }

    #[test]
    fn groups_java_stack_trace() {
        let mut asm = Assembler::default();
        let lines = [
            "ERROR Request failed",
            "java.lang.IllegalStateException: boom",
            "\tat com.x.Foo.bar(Foo.java:12)",
            "Caused by: java.io.IOException: disk",
            "\t... 3 more",
            "INFO ok",
        ];
        let mut events = vec![];
        for l in lines {
            events.extend(asm.feed(l));
        }
        assert_eq!(events.len(), 1);
        assert_eq!(
            ev(&events[0]).exc_type.as_deref(),
            Some("java.io.IOException")
        );
    }

    #[test]
    fn parses_json_lines() {
        let e = ev(
            r#"{"ts":"2026-11-06T10:00:00Z","level":"error","msg":"db timeout after 3000ms","user":"x@example.com"}"#,
        );
        assert_eq!(e.level, Some("ERROR"));
        assert_eq!(e.template, "ERROR db timeout after <NUM>");
        assert!((e.ts.unwrap() - 1793959200.0).abs() < 1.0);
    }

    #[test]
    fn utc_offsets_and_zone_detection() {
        assert_eq!(parse_utc_offset("UTC"), Some(0));
        assert_eq!(parse_utc_offset("+02:00"), Some(7200));
        assert_eq!(parse_utc_offset("-0800"), Some(-28800));
        assert_eq!(parse_utc_offset("+5"), Some(18000));
        assert_eq!(parse_utc_offset("+05:30"), Some(19800));
        assert_eq!(parse_utc_offset("Europe/Paris"), None);
        assert!(ev("2026-11-06 10:00:00 INFO x").ts_naive);
        assert!(!ev("2026-11-06T10:00:00Z INFO x").ts_naive);
        assert!(!ev("2026-11-06T10:00:00+0200 INFO x").ts_naive);
    }

    #[test]
    fn timestamp_formats() {
        let a = parse_timestamp("2026-11-06T10:00:00Z").unwrap();
        assert_eq!(parse_timestamp("2026-11-06 10:00:00").unwrap(), a);
        assert_eq!(parse_timestamp("2026-11-06T12:00:00+0200").unwrap(), a);
        assert_eq!(parse_timestamp("2026-11-06T10:00:00,500").unwrap(), a + 0.5);
    }
}

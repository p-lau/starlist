//! The questions an agent asks: "what's new?" and "did my fix work?".
//! Every function takes `now` so tests can control time.

use crate::pattern::{level_rank, normalize_level, truncate};
use crate::store::{BUCKET_SECS, PatternRow, SplitStat, Store};
use chrono::{DateTime, SecondsFormat};
use serde_json::{Value, json};

pub type AResult = Result<Value, String>;

const TEMPLATE_CHARS: usize = 240;
const SAMPLE_CHARS: usize = 600;
const BRIEF_SAMPLE_CHARS: usize = 400;
/// In brief mode only the top findings carry a sample.
const BRIEF_SAMPLES: usize = 3;

/// How much to say. `Brief` is the default and is tuned to keep agent context small;
/// `Full` adds absolute times, raw rates and untrimmed samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Detail {
    #[default]
    Brief,
    Full,
}

impl Detail {
    pub fn parse(s: Option<&str>) -> Result<Self, String> {
        match s.map(|x| x.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("brief") => Ok(Detail::Brief),
            Some("full") => Ok(Detail::Full),
            Some(o) => Err(format!("detail must be 'brief' or 'full', got '{o}'")),
        }
    }
    fn full(self) -> bool {
        self == Detail::Full
    }
}

/// Remove nulls, empty arrays and empty objects so they cost no tokens.
pub fn compact(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| (k, compact(v)))
                .filter(|(_, v)| !is_empty(v))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(compact).collect()),
        other => other,
    }
}

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// Shorten a stack trace for an agent: keep the log line, the app's own frames and
/// the exception lines; drop frames from the standard library and third-party packages.
pub fn trim_trace(text: &str, max_lines: usize) -> String {
    let lib_frame = |l: &str| {
        let t = l.trim_start();
        (t.starts_with("File \"")
            && [
                "site-packages",
                "dist-packages",
                "/lib/python",
                "\\lib\\python",
                "<frozen",
            ]
            .iter()
            .any(|m| t.contains(m)))
            || (t.starts_with("at ")
                && [
                    "at java.",
                    "at javax.",
                    "at jdk.",
                    "at sun.",
                    "at kotlin.",
                    "at scala.",
                    "at org.springframework.",
                    "at org.apache.",
                    "at io.netty.",
                    "node:internal",
                    "node_modules",
                    "(node:",
                ]
                .iter()
                .any(|m| t.contains(m)))
    };
    let mut kept: Vec<&str> = vec![];
    let mut hidden = 0usize;
    let mut skip_code_line = false;
    for (i, line) in text.lines().enumerate() {
        if i == 0 {
            kept.push(line);
            continue;
        }
        if skip_code_line {
            skip_code_line = false;
            // Python prints the source line under each frame, indented further.
            if line.starts_with("    ") && !line.trim_start().starts_with("File ") {
                continue;
            }
        }
        if lib_frame(line) {
            hidden += 1;
            skip_code_line = line.trim_start().starts_with("File ");
            continue;
        }
        kept.push(line);
    }
    let mut out: Vec<String> = if kept.len() > max_lines {
        let tail = 3.min(max_lines.saturating_sub(1));
        let head = max_lines - tail;
        let mut v: Vec<String> = kept[..head].iter().map(|s| s.to_string()).collect();
        v.push(format!("  … {} lines …", kept.len() - head - tail));
        v.extend(kept[kept.len() - tail..].iter().map(|s| s.to_string()));
        v
    } else {
        kept.iter().map(|s| s.to_string()).collect()
    };
    if hidden > 0 {
        out.push(format!("  ({hidden} library frames hidden)"));
    }
    out.join("\n")
}
/// Default upper bound on one tool answer, in characters of compact JSON
/// (roughly 2k tokens brief, 4k full).
pub const DEFAULT_MAX_CHARS: usize = 8000;
pub const DEFAULT_MAX_CHARS_FULL: usize = 16000;
/// Smallest budget an agent may ask for; below this an answer can't say much.
pub const MIN_MAX_CHARS: usize = 1000;
/// Sample length kept when trimming to fit the budget.
const TRIMMED_SAMPLE_CHARS: usize = 200;

/// Which slice of a list to return and how large the answer may get.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Page {
    pub limit: usize,
    pub offset: usize,
    pub max_chars: usize,
}

impl Page {
    /// The first page with the default budget for `d`.
    pub fn first(limit: usize, d: Detail) -> Self {
        Self {
            limit: limit.max(1),
            offset: 0,
            max_chars: Self::default_budget(d),
        }
    }

    pub fn default_budget(d: Detail) -> usize {
        if d.full() {
            DEFAULT_MAX_CHARS_FULL
        } else {
            DEFAULT_MAX_CHARS
        }
    }

    /// Build a page from tool arguments. A cursor carries the offset and the
    /// `now` of the first call, so later pages rank the same snapshot of time;
    /// the returned time replaces `now` for this call.
    pub fn from_args(
        limit: usize,
        cursor: Option<&str>,
        max_chars: Option<usize>,
        d: Detail,
        now: f64,
    ) -> Result<(Self, f64), String> {
        let max_chars = max_chars
            .unwrap_or_else(|| Self::default_budget(d))
            .max(MIN_MAX_CHARS);
        let (offset, now) = match cursor.map(str::trim).filter(|c| !c.is_empty()) {
            None => (0, now),
            Some(c) => parse_cursor(c)?,
        };
        Ok((
            Self {
                limit: limit.max(1),
                offset,
                max_chars,
            },
            now,
        ))
    }
}

fn make_cursor(offset: usize, now: f64) -> String {
    format!("{offset}@{}", (now * 1000.0).round() as i64)
}

fn parse_cursor(c: &str) -> Result<(usize, f64), String> {
    let bad = || format!("Invalid cursor '{c}'. Pass next_cursor exactly as returned.");
    let (o, t) = c.split_once('@').ok_or_else(bad)?;
    let offset = o.parse::<usize>().map_err(|_| bad())?;
    let ms = t.parse::<i64>().map_err(|_| bad())?;
    Ok((offset, ms as f64 / 1000.0))
}

fn json_len(v: &Value) -> usize {
    serde_json::to_string(v)
        .map(|s| s.chars().count())
        .unwrap_or(0)
}

/// Shorten (`Some(n)`) or drop (`None`) the sample text in a finding or pattern.
fn shrink_samples(v: &mut Value, keep: Option<usize>) {
    let Some(o) = v.as_object_mut() else { return };
    match keep {
        Some(n) => {
            if let Some(Value::String(t)) = o.get_mut("sample") {
                *t = truncate(t, n);
            }
            if let Some(Value::Array(a)) = o.get_mut("samples") {
                for x in a.iter_mut() {
                    if let Some(Value::String(t)) = x.get_mut("text") {
                        *t = truncate(t, n);
                    }
                }
            }
        }
        None => {
            o.remove("sample");
            o.remove("samples");
        }
    }
}

/// Assemble an answer from `head` plus one page of `items` under `key`, and keep
/// it within the page budget: shorten samples, then drop them, then move the
/// last items to the next page. Says what was cut and how to get the rest.
/// With `pageable` false there is no cursor, only a count of what was left out.
fn paged(
    head: Value,
    key: &str,
    mut items: Vec<Value>,
    total: usize,
    page: &Page,
    pageable: bool,
    now: f64,
) -> Value {
    let page_len = items.len();
    let build = |items: &[Value], notes: &[String]| {
        let mut v = head.clone();
        v[key] = json!(items);
        let shown = page.offset + items.len();
        if shown < total {
            if pageable {
                v["next_cursor"] = json!(make_cursor(shown, now));
            }
            v["remaining"] = json!(total - shown);
        }
        if !notes.is_empty() {
            v["trimmed"] = json!(notes.join(" "));
        }
        compact(v)
    };
    let fits = |v: &Value| json_len(v) <= page.max_chars;

    let mut notes: Vec<String> = vec![];
    let v = build(&items, &notes);
    if fits(&v) {
        return v;
    }
    for step in [Some(TRIMMED_SAMPLE_CHARS), None] {
        for it in items.iter_mut() {
            shrink_samples(it, step);
        }
        notes = vec![match step {
            Some(_) => "Samples shortened to fit max_chars.".to_string(),
            None => "Samples removed to fit max_chars; get_pattern shows them.".to_string(),
        }];
        let v = build(&items, &notes);
        if fits(&v) {
            return v;
        }
    }
    while items.len() > 1 {
        items.pop();
        let moved = page_len - items.len();
        let mut n = notes.clone();
        n.push(if pageable {
            format!("{moved} more moved to the next page to fit max_chars.")
        } else {
            format!("{moved} more left out to fit max_chars.")
        });
        let v = build(&items, &n);
        if fits(&v) {
            return v;
        }
    }
    let moved = page_len - items.len();
    if moved > 0 {
        notes.push(format!(
            "{moved} more moved to the next page to fit max_chars."
        ));
    }
    build(&items, &notes)
}

/// Keep a single-object answer within budget by shortening, then dropping, its samples.
fn fit_samples(mut v: Value, max_chars: usize) -> Value {
    v = compact(v);
    for step in [Some(TRIMMED_SAMPLE_CHARS), None] {
        if json_len(&v) <= max_chars {
            break;
        }
        shrink_samples(&mut v, step);
        v["trimmed"] = json!(match step {
            Some(_) => "Samples shortened to fit max_chars.",
            None => "Samples removed to fit max_chars.",
        });
    }
    v
}

/// How many occurrences we need to have expected before calling an absence meaningful.
const MIN_EXPECTED: f64 = 3.0;

pub(crate) fn db<E: std::fmt::Display>(e: E) -> String {
    format!("log store error: {e}")
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// "90s", "30m", "1h", "2d", "1w" → seconds.
pub fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim().trim_start_matches('-');
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mult = match unit.trim() {
        "s" | "sec" | "secs" => 1.0,
        "m" | "min" | "mins" => 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
        "d" | "day" | "days" => 86400.0,
        "w" | "week" | "weeks" => 604800.0,
        _ => return None,
    };
    Some(n * mult)
}

pub fn fmt_duration(secs: f64) -> String {
    let s = secs.abs();
    if s < 90.0 {
        format!("{}s", s.round() as i64)
    } else if s < 90.0 * 60.0 {
        format!("{}m", (s / 60.0).round() as i64)
    } else if s < 48.0 * 3600.0 {
        let h = s / 3600.0;
        if h < 10.0 {
            format!("{:.1}h", h)
        } else {
            format!("{}h", h.round() as i64)
        }
    } else {
        format!("{}d", (s / 86400.0).round() as i64)
    }
}

pub fn fmt_time(ts: f64, now: f64) -> String {
    let iso = DateTime::from_timestamp(ts.floor() as i64, 0)
        .map(|d| d.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_else(|| ts.to_string());
    let delta = now - ts;
    if delta >= 0.0 {
        format!("{iso} ({} ago)", fmt_duration(delta))
    } else {
        format!("{iso} (in {})", fmt_duration(delta))
    }
}

/// Relative time only in brief mode ("52m ago"); ISO plus relative in full.
fn fmt_t(ts: f64, now: f64, d: Detail) -> String {
    if d.full() {
        return fmt_time(ts, now);
    }
    let delta = now - ts;
    if delta >= 0.0 {
        format!("{} ago", fmt_duration(delta))
    } else {
        format!("in {}", fmt_duration(delta))
    }
}

/// "27 in 35m (46.2/h)"
fn count_str(count: i64, secs: f64) -> String {
    format!(
        "{count} in {} ({}/h)",
        fmt_duration(secs),
        per_hour(count, secs)
    )
}

fn per_hour(count: i64, secs: f64) -> f64 {
    if secs <= 0.0 {
        0.0
    } else {
        (count as f64 * 3600.0 / secs * 10.0).round() / 10.0
    }
}

/// A point in time: "now", a duration back from now ("30m"), an ISO time,
/// epoch seconds, or a marker name (optionally written "marker:NAME").
/// Returns the time and, if it came from a marker, the marker's name.
pub fn resolve_time(store: &Store, spec: &str, now: f64) -> Result<(f64, Option<String>), String> {
    let s = spec.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("now") {
        return Ok((now, None));
    }
    if let Some(name) = s.strip_prefix("marker:") {
        return match store.marker(name).map_err(db)? {
            Some(m) => Ok((m.ts, Some(m.name))),
            None => Err(format!(
                "No marker named '{name}'. Call list_markers to see what exists."
            )),
        };
    }
    if let Some(d) = parse_duration(s) {
        return Ok((now - d, None));
    }
    if let Some(t) = crate::pattern::parse_timestamp(s) {
        return Ok((t, None));
    }
    if let Ok(t) = s.parse::<f64>() {
        return Ok((t, None));
    }
    match store.marker(s).map_err(db)? {
        Some(m) => Ok((m.ts, Some(m.name))),
        None => Err(format!(
            "Could not read '{s}' as a time. Use a duration back from now like '30m', an ISO time, or a marker name from list_markers."
        )),
    }
}

pub(crate) fn duration_arg(s: &str, what: &str) -> Result<f64, String> {
    parse_duration(s)
        .filter(|d| *d > 0.0)
        .ok_or_else(|| format!("{what} must be a duration like '1h' or '30m', got '{s}'"))
}

fn sample_text(text: &str, d: Detail) -> String {
    if d.full() {
        truncate(text, SAMPLE_CHARS)
    } else {
        truncate(&trim_trace(text, 10), BRIEF_SAMPLE_CHARS)
    }
}

fn latest_sample(store: &Store, id: &str, d: Detail) -> Option<String> {
    store
        .samples(id, 1)
        .ok()?
        .into_iter()
        .next()
        .map(|(_, t)| sample_text(&t, d))
}

/// Deploy markers in time order, to tell which release was live at a given time.
pub struct Releases(Vec<(f64, String)>);

impl Releases {
    pub fn load(store: &Store) -> Result<Self, String> {
        Ok(Self(store.deploys().map_err(db)?))
    }

    /// The release live at `ts`: the latest deploy at or before it.
    /// None when `ts` is before the first recorded deploy.
    pub fn at(&self, ts: f64) -> Option<&str> {
        let i = self.0.partition_point(|(t, _)| *t <= ts);
        i.checked_sub(1).map(|i| self.0[i].1.as_str())
    }

    pub fn latest(&self) -> Option<(f64, &str)> {
        self.0.last().map(|(t, n)| (*t, n.as_str()))
    }
}

fn pattern_brief(row: &PatternRow, rel: &Releases, now: f64, d: Detail) -> Value {
    json!({
        "pattern_id": row.id,
        "level": row.level,
        "template": truncate(&row.template, TEMPLATE_CHARS),
        "first_seen": if d.full() { Some(fmt_t(row.first_seen, now, d)) } else { None },
        "first_release": rel.at(row.first_seen),
        "location": row.location,
        "example_request": row.example_request,
        "last_seen": fmt_t(row.last_seen, now, d),
    })
}

pub(crate) fn min_level_filter(min_level: Option<&str>) -> Result<u8, String> {
    match min_level {
        None => Ok(0),
        Some(l) => normalize_level(l)
            .map(|l| level_rank(Some(l)))
            .ok_or_else(|| format!("Unknown level '{l}'. Use DEBUG, INFO, WARN, ERROR or FATAL.")),
    }
}

fn markers_between(
    store: &Store,
    lo: f64,
    hi: f64,
    now: f64,
    d: Detail,
) -> Result<Vec<Value>, String> {
    Ok(store
        .markers(lo, hi, 10)
        .map_err(db)?
        .into_iter()
        .map(|m| {
            let note = m.note.map(|n| truncate(&n, if d.full() { 300 } else { 100 }));
            json!({"name": m.name, "kind": if d.full() { Some(m.kind) } else { None }, "at": fmt_t(m.ts, now, d), "note": note})
        })
        .collect())
}

/// One `whats_new` finding, before formatting.
#[derive(Debug, Clone)]
pub struct Finding {
    pub row: PatternRow,
    /// "new", "returning", "spike" or "gone_quiet".
    pub kind: &'static str,
    pub count_window: i64,
    pub count_baseline: i64,
    pub per_hour_window: f64,
    pub per_hour_baseline: f64,
    /// Why it was flagged, when the kind alone doesn't say.
    pub note: Option<String>,
}

/// The result of `whats_new`, before formatting. Findings are sorted: errors
/// and news first, patterns that went quiet last.
#[derive(Debug, Clone)]
pub struct WhatsNewData {
    /// Start of the window; the baseline is [baseline_from, window_from).
    pub window_from: f64,
    pub baseline_from: f64,
    pub now: f64,
    pub events_in_window: i64,
    pub events_in_baseline: i64,
    /// Overall log volume in the window relative to the baseline.
    pub traffic: f64,
    /// Patterns with events in the window.
    pub active: usize,
    /// Findings per kind: new, returning, spike, gone_quiet.
    pub tally: [usize; 4],
    /// Resolved patterns that came back.
    pub regressed: usize,
    pub findings: Vec<Finding>,
}

impl WhatsNewData {
    pub fn summary(&self) -> String {
        if self.findings.is_empty() {
            format!(
                "Nothing new: {} events across {} known patterns, all within their usual rates.",
                self.events_in_window, self.active
            )
        } else {
            let t = self.tally;
            let regressed = if self.regressed > 0 {
                format!("{} regressed, ", self.regressed)
            } else {
                String::new()
            };
            format!(
                "{} new, {regressed}{} returning, {} spiking, {} gone quiet. {} events across {} patterns in the window.",
                t[0], t[1], t[2], t[3], self.events_in_window, self.active
            )
        }
    }
}

/// Classify patterns in the last `window` seconds against the `baseline` before it.
pub fn whats_new_data(
    store: &Store,
    window: f64,
    baseline: f64,
    min_level: Option<&str>,
    now: f64,
) -> Result<WhatsNewData, String> {
    let (w, b) = (window, baseline);
    let min_rank = min_level_filter(min_level)?;
    let split = now - w;
    let lo = split - b;
    let stats = store.split_counts(lo, split, now).map_err(db)?;
    let events_in_window: i64 = stats.iter().map(|s| s.after).sum();
    let events_in_baseline: i64 = stats.iter().map(|s| s.before).sum();
    // When the whole app gets busier, every pattern rises with it. Judge spikes
    // against overall volume so only patterns that grew faster than traffic show up.
    let traffic = if events_in_baseline > 0 {
        per_hour(events_in_window, w) / per_hour(events_in_baseline, b).max(0.001)
    } else {
        1.0
    };
    let active = stats.iter().filter(|s| s.after > 0).count();

    let states = crate::issues::States::load(store)?;
    let rel = Releases::load(store)?;
    // (group, level, kind order, count) for sorting; group 1 = good news, listed last.
    let mut found: Vec<((u8, u8, u8, i64), Finding)> = vec![];
    let mut tally = [0usize; 4];
    let mut regressed = 0;
    for SplitStat { row, before, after } in stats {
        let lvl = level_rank(row.level.as_deref());
        if lvl < min_rank {
            continue;
        }
        let state = states.effective(store, &row, now);
        if state.is_ignored() {
            continue;
        }
        let (a_rate, b_rate) = (per_hour(after, w), per_hour(before, b));
        if after > 0 && matches!(state, crate::issues::Effective::Regressed { .. }) {
            regressed += 1;
            found.push((
                (0, 4 - lvl, 0, -after),
                Finding {
                    note: crate::issues::describe(&state, &rel, now),
                    row,
                    kind: "regressed",
                    count_window: after,
                    count_baseline: before,
                    per_hour_window: a_rate,
                    per_hour_baseline: b_rate,
                },
            ));
            continue;
        }
        let (kind, order, note) = if after > 0 && row.first_seen >= split {
            ("new", 0u8, None)
        } else if after > 0 && before == 0 {
            (
                "returning",
                1,
                Some(format!(
                    "Back after a quiet baseline; first seen {}.",
                    fmt_t(row.first_seen, now, Detail::Brief)
                )),
            )
        } else if after >= 5 && before > 0 && a_rate >= 3.0 * b_rate * traffic.max(1.0) {
            let x = a_rate / b_rate.max(0.001);
            let note = if traffic > 1.2 {
                format!("{x:.1}x its baseline rate, while overall log volume is {traffic:.1}x.")
            } else {
                format!("{x:.1}x its baseline rate.")
            };
            ("spike", 2, Some(note))
        } else if after == 0 && (before as f64) * w / b >= 5.0 {
            (
                "gone_quiet",
                3,
                Some(format!(
                    "Expected ~{:.0} in the window from its baseline rate, saw none.",
                    before as f64 * w / b
                )),
            )
        } else {
            continue;
        };
        tally[order as usize] += 1;
        let group = u8::from(kind == "gone_quiet");
        found.push((
            (group, 4 - lvl, order, -after),
            Finding {
                row,
                kind,
                count_window: after,
                count_baseline: before,
                per_hour_window: a_rate,
                per_hour_baseline: b_rate,
                note,
            },
        ));
    }
    found.sort_by_key(|a| a.0);
    Ok(WhatsNewData {
        window_from: split,
        baseline_from: lo,
        now,
        events_in_window,
        events_in_baseline,
        traffic,
        active,
        tally,
        regressed,
        findings: found.into_iter().map(|(_, f)| f).collect(),
    })
}

/// What changed in the last `window` compared with the `baseline` before it.
pub fn whats_new(
    store: &Store,
    window: &str,
    baseline: &str,
    min_level: Option<&str>,
    page: &Page,
    d: Detail,
    now: f64,
) -> AResult {
    let w = duration_arg(window, "window")?;
    let b = duration_arg(baseline, "baseline")?;
    let data = whats_new_data(store, w, b, min_level, now)?;
    let rel = Releases::load(store)?;
    let split = data.window_from;
    let total = data.findings.len();
    let findings: Vec<Value> = data
        .findings
        .iter()
        .skip(page.offset)
        .take(page.limit)
        .enumerate()
        .map(|(i, f)| {
            let mut v = pattern_brief(&f.row, &rel, now, d);
            v["kind"] = json!(f.kind);
            if let Some(n) = &f.note {
                // The note was written for brief output; redo the time in full mode.
                v["note"] = json!(if d.full() && f.kind == "returning" {
                    format!(
                        "Back after a quiet baseline; first seen {}.",
                        fmt_t(f.row.first_seen, now, d)
                    )
                } else {
                    n.clone()
                });
            }
            v["count_in_window"] = json!(f.count_window);
            v["per_hour"] = if d.full() {
                json!({"window": f.per_hour_window, "baseline": f.per_hour_baseline})
            } else {
                json!(format!(
                    "{} vs {} baseline",
                    f.per_hour_window, f.per_hour_baseline
                ))
            };
            if f.kind != "gone_quiet" && (d.full() || i < BRIEF_SAMPLES) {
                v["sample"] = json!(latest_sample(store, &f.row.id, d));
            }
            v
        })
        .collect();

    let traffic = data.traffic;
    let traffic_note = (traffic >= 2.0 || traffic <= 0.5)
        .then(|| format!("Overall log volume is {traffic:.1}x its baseline."));
    let head = json!({
        "window": if d.full() {
            format!("last {} (since {}) vs the {} before", fmt_duration(w), fmt_time(split, now), fmt_duration(b))
        } else {
            format!("last {} vs prior {}", fmt_duration(w), fmt_duration(b))
        },
        "summary": data.summary(),
        "traffic": if d.full() {
            json!({
                "per_hour_window": per_hour(data.events_in_window, w),
                "per_hour_baseline": per_hour(data.events_in_baseline, b),
                "note": traffic_note,
            })
        } else {
            json!({"note": traffic_note})
        },
        "current_release": rel.latest().map(|(_, n)| n),
        "markers_in_window": markers_between(store, split, now, now, d)?,
    });
    Ok(paged(head, "findings", findings, total, page, true, now))
}

/// The result of `verify_fix`, before formatting.
#[derive(Debug, Clone)]
pub struct FixCheck {
    pub row: PatternRow,
    pub split: f64,
    /// The marker `since` named, if it was one.
    pub marker: Option<String>,
    /// fixed, reduced, still_occurring, worse, too_early, appeared_after or not_active_before.
    pub verdict: &'static str,
    pub explanation: String,
    /// Start of the period measured before the split (absent for appeared_after).
    pub before_from: Option<f64>,
    pub before_count: i64,
    pub after_count: i64,
    pub now: f64,
    /// Occurrences expected after the split at the old rate.
    pub expected_after: Option<f64>,
    /// For "fixed": "high" or "medium".
    pub confidence: Option<&'static str>,
    /// For "too_early": minutes until a verdict is meaningful.
    pub wait_minutes: Option<f64>,
    /// Set when traffic dropped too much to trust a fixed or reduced verdict.
    pub caveat: Option<String>,
}

/// Did this pattern stop (or drop) after a point in time? Typed result.
pub fn verify_fix_data(
    store: &Store,
    pattern_id: &str,
    since: &str,
    before: Option<&str>,
    now: f64,
) -> Result<FixCheck, String> {
    let row = store.pattern(pattern_id).map_err(db)?.ok_or_else(|| {
        format!("Unknown pattern_id '{pattern_id}'. Use whats_new or list_patterns to find IDs.")
    })?;
    let (split, marker) = resolve_time(store, since, now)?;
    if split >= now {
        return Err("'since' is in the future.".into());
    }
    let after_secs = now - split;
    let span = match before {
        Some(b) => duration_arg(b, "before")?,
        None => after_secs.max(3600.0),
    };
    let mut out = FixCheck {
        row: row.clone(),
        split,
        marker,
        verdict: "appeared_after",
        explanation: String::new(),
        before_from: None,
        before_count: 0,
        after_count: 0,
        now,
        expected_after: None,
        confidence: None,
        wait_minutes: None,
        caveat: None,
    };

    if row.first_seen >= split {
        out.explanation = format!(
            "This pattern did not exist before the split; it first appeared {}. The change likely introduced it.",
            fmt_t(row.first_seen, now, Detail::Brief)
        );
        out.after_count = store.count(&row.id, split, now).map_err(db)?;
        return Ok(out);
    }

    // Measure the "before" rate only while the pattern existed, so a bug that
    // started 20 minutes before the fix isn't diluted by an hour of quiet.
    // Without an explicit `before`, widen the window for rare errors until it
    // holds a few occurrences (up to a day).
    let spans: Vec<f64> = match before {
        Some(_) => vec![span],
        None => vec![span, span.max(6.0 * 3600.0), span.max(86400.0)],
    };
    let (mut before_lo, mut b, mut a) = (split, 0, 0);
    for s in spans {
        before_lo = (split - s).max(row.first_seen - BUCKET_SECS);
        (b, a) = store
            .split_counts(before_lo, split, now)
            .map_err(db)?
            .into_iter()
            .find(|st| st.row.id == row.id)
            .map(|st| (st.before, st.after))
            .unwrap_or((0, 0));
        if b as f64 >= MIN_EXPECTED {
            break;
        }
    }
    let before_secs = (split - before_lo).max(60.0);
    let (tb, ta) = store.total_split(before_lo, split, now).map_err(db)?;

    let b_rate = b as f64 / before_secs; // per second
    let a_rate = a as f64 / after_secs;
    let expected = b_rate * after_secs;
    out.before_from = Some(split - before_secs);
    out.before_count = b;
    out.after_count = a;
    out.expected_after = Some((expected * 10.0).round() / 10.0);

    let (verdict, explanation) = if b == 0 {
        (
            "not_active_before",
            format!(
                "No occurrences in the {} before the split, so there is nothing to compare. Try a longer 'before' window.",
                fmt_duration(before_secs)
            ),
        )
    } else if a == 0 {
        if expected >= MIN_EXPECTED {
            out.confidence = Some(if expected >= 2.0 * MIN_EXPECTED {
                "high"
            } else {
                "medium"
            });
            (
                "fixed",
                format!(
                    "At the old rate about {expected:.0} would have occurred since the split; none did."
                ),
            )
        } else {
            let wait = ((MIN_EXPECTED / b_rate - after_secs) / 60.0)
                .ceil()
                .max(1.0);
            out.wait_minutes = Some(wait);
            (
                "too_early",
                format!(
                    "None since the split, but at the old rate only ~{expected:.1} were expected yet. Check again in about {wait:.0} minutes."
                ),
            )
        }
    } else {
        let ratio = a_rate / b_rate;
        if ratio >= 2.0 {
            (
                "worse",
                format!("Happening {ratio:.1}x as often as before the split."),
            )
        } else if ratio <= 0.5 && expected >= MIN_EXPECTED {
            (
                "reduced",
                format!(
                    "Down to {:.0}% of its earlier rate but still occurring.",
                    ratio * 100.0
                ),
            )
        } else {
            (
                "still_occurring",
                format!(
                    "Still happening at {:.0}% of its earlier rate.",
                    ratio * 100.0
                ),
            )
        }
    };
    if (verdict == "fixed" || verdict == "reduced") && tb > 0 {
        let traffic = (ta as f64 / after_secs) / (tb as f64 / before_secs);
        if traffic < 0.3 {
            out.caveat = Some(format!(
                "Overall log volume dropped to {:.0}% of its earlier rate after the split; low traffic can hide errors.",
                traffic * 100.0
            ));
        }
    }
    out.verdict = verdict;
    out.explanation = explanation;
    Ok(out)
}

/// Did this pattern stop (or drop) after a point in time?
pub fn verify_fix(
    store: &Store,
    pattern_id: &str,
    since: &str,
    before: Option<&str>,
    d: Detail,
    now: f64,
) -> AResult {
    let c = verify_fix_data(store, pattern_id, since, before, now)?;
    let row = &c.row;
    let mut base = json!({
        "pattern_id": row.id,
        "level": if d.full() { row.level.clone() } else { None },
        "template": truncate(&row.template, if d.full() { TEMPLATE_CHARS } else { 120 }),
        "split": {"at": fmt_t(c.split, now, d), "marker": c.marker},
        "last_seen": fmt_t(row.last_seen, now, d),
    });
    match c.before_from {
        None => {
            base["verdict"] = json!(c.verdict);
            base["explanation"] = json!(format!(
                "This pattern did not exist before the split; it first appeared {}. The change likely introduced it.",
                fmt_t(row.first_seen, now, d)
            ));
            base["after"] = json!({"count": c.after_count});
        }
        Some(before_lo) => {
            let (before_secs, after_secs) = (c.split - before_lo, now - c.split);
            let (b, a) = (c.before_count, c.after_count);
            if d.full() {
                base["before"] = json!({"window": format!("{} before the split", fmt_duration(before_secs)), "count": b, "per_hour": per_hour(b, before_secs)});
                base["after"] = json!({"window": format!("{} since the split", fmt_duration(after_secs)), "count": a, "per_hour": per_hour(a, after_secs)});
                base["expected_after_if_unchanged"] = json!(c.expected_after);
            } else {
                base["before"] = json!(count_str(b, before_secs));
                base["after"] = json!(count_str(a, after_secs));
            }
            base["confidence"] = json!(c.confidence);
            base["wait_minutes"] = json!(c.wait_minutes);
            base["caveat"] = json!(c.caveat);
            base["verdict"] = json!(c.verdict);
            base["explanation"] = json!(c.explanation);
        }
    }
    Ok(compact(base))
}

/// One `compare_deploy` finding, before formatting.
#[derive(Debug, Clone)]
pub struct Change {
    pub row: PatternRow,
    /// "new_after", "increased", "stopped" or "decreased".
    pub kind: &'static str,
    pub count_before: i64,
    pub count_after: i64,
    pub per_hour_before: f64,
    pub per_hour_after: f64,
}

/// The result of `compare_deploy`, before formatting.
#[derive(Debug, Clone)]
pub struct DeployDiff {
    pub split: f64,
    pub marker: Option<String>,
    pub before_from: f64,
    pub now: f64,
    pub events_before: i64,
    pub events_after: i64,
    /// Changes per kind: new_after, increased, stopped, decreased.
    pub tally: [usize; 4],
    pub changes: Vec<Change>,
}

impl DeployDiff {
    pub fn summary(&self) -> String {
        let t = self.tally;
        format!(
            "{} new since the split, {} increased, {} stopped, {} decreased.",
            t[0], t[1], t[2], t[3]
        )
    }
}

/// Fleet-wide before/after diff around a deploy. Typed result.
pub fn compare_deploy_data(
    store: &Store,
    at: Option<&str>,
    before: Option<&str>,
    now: f64,
) -> Result<DeployDiff, String> {
    let rel = Releases::load(store)?;
    let (split, marker) = match at.map(str::trim).filter(|a| !a.is_empty()) {
        Some(at) => resolve_time(store, at, now)?,
        None => match rel.latest() {
            Some((t, n)) => (t, Some(n.to_string())),
            None => {
                return Err("No deploy markers yet. Pass 'at' (a marker name, ISO time or duration), or record deploys with `starlist mark --deploy NAME` or STARLIST_VERSION.".into());
            }
        },
    };
    if split >= now {
        return Err("'at' is in the future.".into());
    }
    let after_secs = now - split;
    let span = match before {
        Some(b) => duration_arg(b, "before")?,
        None => after_secs.max(3600.0),
    };
    let lo = split - span;
    let stats = store.split_counts(lo, split, now).map_err(db)?;
    let (tb, ta) = store.total_split(lo, split, now).map_err(db)?;
    let traffic = if tb > 0 {
        (ta as f64 / after_secs) / (tb as f64 / span)
    } else {
        1.0
    };

    let mut found: Vec<((u8, u8, u8, i64), Change)> = vec![];
    let mut tally = [0usize; 4];
    for SplitStat { row, before, after } in stats {
        let (b, a) = (before, after);
        let b_rate = b as f64 / span;
        let a_rate = a as f64 / after_secs;
        let expected = b_rate * after_secs;
        let (kind, order) = if a > 0 && row.first_seen >= split {
            ("new_after", 0u8)
        } else if a >= 5 && b > 0 && a_rate >= 2.0 * b_rate * traffic.max(1.0) {
            ("increased", 1)
        } else if a == 0 && expected >= MIN_EXPECTED {
            ("stopped", 2)
        } else if a > 0 && expected >= 5.0 && a_rate <= 0.5 * b_rate * traffic.min(1.0) {
            ("decreased", 3)
        } else {
            continue;
        };
        tally[order as usize] += 1;
        let group = u8::from(order >= 2);
        let key = (
            group,
            4 - level_rank(row.level.as_deref()),
            order,
            -(a.max(b)),
        );
        found.push((
            key,
            Change {
                row,
                kind,
                count_before: b,
                count_after: a,
                per_hour_before: per_hour(b, span),
                per_hour_after: per_hour(a, after_secs),
            },
        ));
    }
    found.sort_by_key(|x| x.0);
    Ok(DeployDiff {
        split,
        marker,
        before_from: lo,
        now,
        events_before: tb,
        events_after: ta,
        tally,
        changes: found.into_iter().map(|(_, c)| c).collect(),
    })
}

/// Fleet-wide before/after diff around a deploy or other point in time.
pub fn compare_deploy(
    store: &Store,
    at: Option<&str>,
    before: Option<&str>,
    page: &Page,
    d: Detail,
    now: f64,
) -> AResult {
    let diff = compare_deploy_data(store, at, before, now)?;
    let rel = Releases::load(store)?;
    let (span, after_secs) = (diff.split - diff.before_from, now - diff.split);
    let total = diff.changes.len();
    let findings: Vec<Value> = diff
        .changes
        .iter()
        .skip(page.offset)
        .take(page.limit)
        .map(|c| {
            let mut v = pattern_brief(&c.row, &rel, now, d);
            v["kind"] = json!(c.kind);
            if d.full() {
                v["before"] = json!({"count": c.count_before, "per_hour": c.per_hour_before});
                v["after"] = json!({"count": c.count_after, "per_hour": c.per_hour_after});
            } else {
                v["before"] = json!(count_str(c.count_before, span));
                v["after"] = json!(count_str(c.count_after, after_secs));
            }
            v
        })
        .collect();
    let head = json!({
        "split": {"at": fmt_t(diff.split, now, d), "marker": diff.marker},
        "compared": format!("{} before vs {} after", fmt_duration(span), fmt_duration(after_secs)),
        "summary": diff.summary(),
        "traffic_per_hour": format!("{} before, {} after", per_hour(diff.events_before, span), per_hour(diff.events_after, after_secs)),
    });
    Ok(paged(head, "findings", findings, total, page, true, now))
}

pub fn list_patterns(
    store: &Store,
    since: &str,
    min_level: Option<&str>,
    contains: Option<&str>,
    page: &Page,
    d: Detail,
    now: f64,
) -> AResult {
    let (lo, _) = resolve_time(store, since, now)?;
    let min_rank = min_level_filter(min_level)?;
    let needle = contains.map(str::to_lowercase);
    // Put the split past `now` so everything counts as "before".
    let mut stats: Vec<SplitStat> = store
        .split_counts(lo, now + 2.0 * BUCKET_SECS, now)
        .map_err(db)?
        .into_iter()
        .filter(|s| s.before > 0 && level_rank(s.row.level.as_deref()) >= min_rank)
        .filter(|s| {
            needle
                .as_ref()
                .is_none_or(|n| s.row.template.to_lowercase().contains(n))
        })
        .collect();
    stats.sort_by_key(|s| {
        (
            std::cmp::Reverse(level_rank(s.row.level.as_deref())),
            std::cmp::Reverse(s.before),
        )
    });
    let total = stats.len();
    let rel = Releases::load(store)?;
    let items: Vec<Value> = stats
        .iter()
        .skip(page.offset)
        .take(page.limit)
        .map(|s| {
            let mut v = pattern_brief(&s.row, &rel, now, d);
            v["count"] = json!(s.before);
            v
        })
        .collect();
    let head = json!({"since": fmt_t(lo, now, d)});
    Ok(paged(head, "patterns", items, total, page, true, now))
}

pub fn get_pattern(
    store: &Store,
    pattern_id: &str,
    samples: Option<i64>,
    max_chars: usize,
    d: Detail,
    now: f64,
) -> AResult {
    let row = store
        .pattern(pattern_id)
        .map_err(db)?
        .ok_or_else(|| format!("Unknown pattern_id '{pattern_id}'."))?;
    let day = now - 86400.0;
    let mut v = pattern_brief(&row, &Releases::load(store)?, now, Detail::Full);
    v["template"] = json!(row.template);
    v["exception"] = json!(row.exc_type);
    let locs = store.locations(&row.id, 5).map_err(db)?;
    if locs.len() > 1 {
        v["locations"] = json!(
            locs.into_iter()
                .map(|(l, n)| json!({"location": l, "count": n}))
                .collect::<Vec<_>>()
        );
    }
    let states = crate::issues::States::load(store)?;
    let state = states.effective(store, &row, now);
    v["state"] = json!(state.name());
    v["state_note"] = json!(crate::issues::describe(
        &state,
        &Releases::load(store)?,
        now
    ));
    v["total_seen"] = json!(row.total);
    v["count_last_hour"] = json!(store.count(&row.id, now - 3600.0, now).map_err(db)?);
    v["count_last_24h"] = json!(store.count(&row.id, day, now).map_err(db)?);
    v["hourly_last_24h"] = json!(store.histogram(&row.id, day, now, 3600.0).map_err(db)?);
    let fields: Vec<String> = crate::metrics::summaries(store, Some(&row.id), day, now)
        .map_err(db)?
        .iter()
        .map(|f| {
            format!(
                "{}: p50 {}, p95 {}, p99 {}, max {} over {} events",
                f.field, f.p50, f.p95, f.p99, f.max, f.count
            )
        })
        .collect();
    if !fields.is_empty() {
        v["fields_last_24h"] = json!(fields);
    }
    v["markers_last_24h"] = json!(markers_between(store, day, now, now, d)?);
    let n = samples.unwrap_or(if d.full() { 3 } else { 1 }).clamp(1, 5);
    v["samples"] = json!(
        store
            .samples(&row.id, n)
            .map_err(db)?
            .into_iter()
            .map(|(ts, t)| {
                let text = if d.full() {
                    truncate(&t, 1500)
                } else {
                    truncate(&trim_trace(&t, 16), 800)
                };
                json!({"at": fmt_t(ts, now, d), "text": text})
            })
            .collect::<Vec<_>>()
    );
    Ok(fit_samples(v, max_chars))
}

pub fn add_marker(
    store: &Store,
    name: &str,
    note: Option<&str>,
    at: Option<&str>,
    deploy: bool,
    now: f64,
) -> AResult {
    let name = name.trim();
    if name.is_empty() {
        return Err("Marker name can't be empty.".into());
    }
    let ts = match at {
        Some(a) => resolve_time(store, a, now)?.0,
        None => now,
    };
    let kind = if deploy { "deploy" } else { "manual" };
    if deploy {
        store.add_deploy(name, ts, note).map_err(db)?;
    } else {
        store.add_marker(name, ts, kind, note).map_err(db)?;
    }
    Ok(json!({"marker": name, "kind": kind, "at": fmt_time(ts, now)}))
}

pub fn list_markers(store: &Store, limit: i64, max_chars: usize, now: f64) -> AResult {
    let ms = store
        .markers(f64::MIN, f64::MAX, limit.clamp(1, 100))
        .map_err(db)?;
    let items: Vec<Value> = ms
        .into_iter()
        .map(|m| json!({"name": m.name, "kind": m.kind, "at": fmt_time(m.ts, now), "note": m.note.map(|n| truncate(&n, 300))}))
        .collect();
    let total = items.len();
    let page = Page {
        limit: total,
        offset: 0,
        max_chars,
    };
    Ok(paged(json!({}), "markers", items, total, &page, false, now))
}

/// Signed offset from the anchor, like "-12s" or "+2m".
fn offset(dt: f64) -> String {
    if dt.abs() < 0.5 {
        return "0s".into();
    }
    format!(
        "{}{}",
        if dt < 0.0 { "-" } else { "+" },
        fmt_duration(dt.abs())
    )
}

/// The raw, redacted lines around an occurrence of a pattern or a point in time.
/// Lines come from the raw buffer, so only recent history is available.
#[allow(clippy::too_many_arguments)]
pub fn get_context(
    store: &Store,
    pattern_id: Option<&str>,
    at: Option<&str>,
    before: i64,
    after: i64,
    max_chars: usize,
    d: Detail,
    now: f64,
) -> AResult {
    let pid = match pattern_id.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Some(
            store
                .pattern(p)
                .map_err(db)?
                .ok_or_else(|| {
                    format!("Unknown pattern_id '{p}'. Call list_patterns or whats_new for IDs.")
                })?
                .id,
        ),
        None => None,
    };
    let at = match at.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => Some(resolve_time(store, a, now)?.0),
        None if pid.is_none() => return Err("Pass pattern_id, at, or both.".into()),
        None => None,
    };
    let Some(anchor) = store.raw_anchor(pid.as_deref(), at).map_err(db)? else {
        let (n, oldest) = store.raw_span().map_err(db)?;
        return Err(match oldest {
            None if n == 0 => {
                "No raw lines are stored. starlist keeps recent lines unless it runs with --no-raw."
                    .into()
            }
            _ => format!(
                "No stored line matches. The raw buffer holds the last {n} events, back to {}.",
                fmt_time(oldest.unwrap_or(now), now)
            ),
        });
    };
    let lines = store
        .raw_around(&anchor, before.clamp(0, 200), after.clamp(0, 200))
        .map_err(db)?;

    let render = |l: &crate::store::RawLine, short: Option<usize>| -> Value {
        let is_anchor = l.id == anchor.id;
        let limit = short.unwrap_or(match (d.full(), is_anchor) {
            (true, _) => 2000,
            (false, true) => 800,
            (false, false) => 300,
        });
        let text = if d.full() {
            truncate(&l.text, limit)
        } else {
            truncate(&trim_trace(&l.text, if is_anchor { 16 } else { 6 }), limit)
        };
        let off = offset(l.ts - anchor.ts);
        if d.full() {
            json!({
                "at": fmt_time(l.ts, now),
                "offset": off,
                "pattern_id": l.pattern_id,
                "anchor": if is_anchor { Some(true) } else { None },
                "text": text,
            })
        } else {
            json!(format!(
                "{}{off} {text}",
                if is_anchor { ">> " } else { "" }
            ))
        }
    };
    let head = json!({
        "anchor": {"pattern_id": anchor.pattern_id, "at": fmt_t(anchor.ts, now, d)},
        "note": if d.full() { None } else { Some("Offsets are from the anchor line, marked >>.") },
    });
    let build = |ls: &[crate::store::RawLine], short: Option<usize>, notes: &[String]| {
        let mut v = head.clone();
        v["lines"] = json!(
            ls.iter()
                .map(|l| {
                    // The anchor line is never shortened further.
                    render(l, if l.id == anchor.id { None } else { short })
                })
                .collect::<Vec<_>>()
        );
        if !notes.is_empty() {
            v["trimmed"] = json!(notes.join(" "));
        }
        compact(v)
    };

    let mut ls = lines;
    let mut notes: Vec<String> = vec![];
    let mut short = None;
    let mut v = build(&ls, short, &notes);
    if json_len(&v) > max_chars {
        short = Some(120);
        notes.push("Lines shortened to fit max_chars.".into());
        v = build(&ls, short, &notes);
    }
    let (mut cut_before, mut cut_after) = (0, 0);
    while json_len(&v) > max_chars && ls.len() > 1 {
        let pos = ls.iter().position(|l| l.id == anchor.id).unwrap_or(0);
        // Drop the line farthest from the anchor, from the longer side.
        if pos >= ls.len() - 1 - pos {
            ls.remove(0);
            cut_before += 1;
        } else {
            ls.pop();
            cut_after += 1;
        }
        let mut n = notes.clone();
        n.push(format!(
            "{cut_before} earlier and {cut_after} later lines left out to fit max_chars."
        ));
        v = build(&ls, short, &n);
    }
    Ok(v)
}

/// Most events returned for one request before the budget applies.
const REQUEST_LINES: i64 = 500;

/// The events of one request in order, from the raw buffer.
pub fn get_request(
    store: &Store,
    request_id: &str,
    max_chars: usize,
    d: Detail,
    now: f64,
) -> AResult {
    let id = crate::request::normalize_query(request_id);
    if id.is_empty() {
        return Err("Pass a request_id, e.g. the example_request of a finding.".into());
    }
    let lines = store.request_lines(&id, REQUEST_LINES + 1).map_err(db)?;
    if lines.is_empty() {
        let (n, oldest) = store.raw_span().map_err(db)?;
        return Err(if n == 0 {
            "No raw lines are stored, so requests can't be followed. starlist keeps recent lines unless it runs with --no-raw.".into()
        } else {
            format!(
                "No events for request '{id}'. The raw buffer goes back to {}; IDs come from trace_id, request_id, traceparent and similar fields (add more with --request-key).",
                fmt_time(oldest.unwrap_or(now), now)
            )
        });
    }
    let capped = lines.len() as i64 > REQUEST_LINES;
    let lines = &lines[..lines.len().min(REQUEST_LINES as usize)];
    let start = lines[0].ts;
    let end = lines[lines.len() - 1].ts;
    let patterns: std::collections::HashSet<&str> =
        lines.iter().map(|l| l.pattern_id.as_str()).collect();

    let render = |l: &crate::store::RawLine, short: Option<usize>| -> Value {
        let limit = short.unwrap_or(if d.full() { 2000 } else { 400 });
        let off = offset(l.ts - start);
        if d.full() {
            json!({"at": fmt_time(l.ts, now), "offset": off, "pattern_id": l.pattern_id, "text": truncate(&l.text, limit)})
        } else {
            json!(format!(
                "{off} {}",
                truncate(&trim_trace(&l.text, 10), limit)
            ))
        }
    };
    let head = json!({
        "request_id": id,
        "started": fmt_t(start, now, d),
        "duration": fmt_duration(end - start),
        "events": lines.len(),
        "patterns": patterns.len(),
        "note": if capped { Some(format!("Only the first {REQUEST_LINES} events are shown.")) } else { None },
    });
    let build = |keep: &[usize], short: Option<usize>, notes: &[String]| {
        let mut v = head.clone();
        let mut out = vec![];
        for (n, &i) in keep.iter().enumerate() {
            if n > 0 && i != keep[n - 1] + 1 {
                out.push(json!(format!(
                    "… {} events left out …",
                    i - keep[n - 1] - 1
                )));
            }
            out.push(render(&lines[i], short));
        }
        v["lines"] = json!(out);
        if !notes.is_empty() {
            v["trimmed"] = json!(notes.join(" "));
        }
        compact(v)
    };
    let mut keep: Vec<usize> = (0..lines.len()).collect();
    let mut notes: Vec<String> = vec![];
    let mut short = None;
    let mut v = build(&keep, short, &notes);
    if json_len(&v) > max_chars {
        short = Some(160);
        notes.push("Lines shortened to fit max_chars.".into());
        v = build(&keep, short, &notes);
    }
    // Keep how the request started and how it ended; drop from the middle.
    while json_len(&v) > max_chars && keep.len() > 2 {
        keep.remove(keep.len() / 2);
        let mut n = notes.clone();
        n.push(format!(
            "{} middle events left out to fit max_chars.",
            lines.len() - keep.len()
        ));
        v = build(&keep, short, &n);
    }
    Ok(v)
}

/// MCP: change a pattern's issue state.
pub fn set_pattern_state(
    store: &Store,
    pattern_id: &str,
    state: &str,
    ignore_for: Option<&str>,
    ignore_count: Option<i64>,
    note: Option<&str>,
    now: f64,
) -> AResult {
    let (row, eff) = crate::issues::set_state(
        store,
        pattern_id,
        state,
        ignore_for,
        ignore_count,
        note,
        now,
    )?;
    let rel = Releases::load(store)?;
    Ok(compact(json!({
        "pattern_id": row.id,
        "template": truncate(&row.template, 120),
        "state": eff.name(),
        "note": crate::issues::describe(&eff, &rel, now),
    })))
}

/// MCP: what a release changed: new, regressed and resolved patterns, and its error rate.
pub fn get_release(
    store: &Store,
    name: Option<&str>,
    max_chars: usize,
    d: Detail,
    now: f64,
) -> AResult {
    let r = crate::issues::release(store, name, now)?;
    let rel = Releases::load(store)?;
    let limit = if d.full() { 25 } else { 8 };
    let list = |rows: Vec<Value>, total: usize| {
        let mut v = json!({"total": total, "patterns": rows});
        if total == 0 {
            v = json!({"total": 0});
        }
        v
    };
    let new: Vec<Value> = r
        .new
        .iter()
        .take(limit)
        .map(|p| pattern_brief(p, &rel, now, d))
        .collect();
    let regressed: Vec<Value> = r
        .regressed
        .iter()
        .take(limit)
        .map(|(p, at)| {
            let mut v = pattern_brief(p, &rel, now, d);
            v["back"] = json!(fmt_t(*at, now, d));
            v
        })
        .collect();
    let resolved: Vec<Value> = r
        .resolved
        .iter()
        .take(limit)
        .map(|(p, at)| {
            let mut v = pattern_brief(p, &rel, now, d);
            v["resolved"] = json!(fmt_t(*at, now, d));
            v
        })
        .collect();
    let trend = match r.previous_errors_per_hour {
        Some(prev) if prev > 0.0 => Some(format!(
            "{}/h errors vs {prev}/h in {}",
            r.errors_per_hour,
            r.previous.as_deref().unwrap_or("the previous release")
        )),
        Some(_) => Some(format!(
            "{}/h errors; none in {}",
            r.errors_per_hour,
            r.previous.as_deref().unwrap_or("the previous release")
        )),
        None => None,
    };
    let v = json!({
        "release": r.name,
        "env": r.env,
        "live": match r.until {
            Some(u) => format!("{} to {}", fmt_t(r.at, now, d), fmt_t(u, now, d)),
            None => format!("since {} (current)", fmt_t(r.at, now, d)),
        },
        "events_per_hour": r.events_per_hour,
        "errors_per_hour": r.errors_per_hour,
        "vs_previous": trend,
        "new": list(new, r.new.len()),
        "regressed": list(regressed, r.regressed.len()),
        "resolved": list(resolved, r.resolved.len()),
    });
    Ok(fit_samples(v, max_chars))
}

/// MCP: merge patterns into the first by hand.
pub fn merge_patterns(store: &Store, ids: &[String]) -> AResult {
    let row = crate::issues::merge(store, ids)?;
    Ok(json!({
        "pattern_id": row.id,
        "template": truncate(&row.template, TEMPLATE_CHARS),
        "merged": ids.len() - 1,
        "total_seen": row.total,
        "note": "The other IDs now resolve to this one.",
    }))
}

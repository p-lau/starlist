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
/// How many occurrences we need to have expected before calling an absence meaningful.
const MIN_EXPECTED: f64 = 3.0;

fn db<E: std::fmt::Display>(e: E) -> String {
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

fn duration_arg(s: &str, what: &str) -> Result<f64, String> {
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
        "last_seen": fmt_t(row.last_seen, now, d),
    })
}

fn min_level_filter(min_level: Option<&str>) -> Result<u8, String> {
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
        .map(|m| json!({"name": m.name, "kind": if d.full() { Some(m.kind) } else { None }, "at": fmt_t(m.ts, now, d), "note": m.note}))
        .collect())
}

/// What changed in the last `window` compared with the `baseline` before it.
pub fn whats_new(
    store: &Store,
    window: &str,
    baseline: &str,
    min_level: Option<&str>,
    limit: usize,
    d: Detail,
    now: f64,
) -> AResult {
    let w = duration_arg(window, "window")?;
    let b = duration_arg(baseline, "baseline")?;
    let min_rank = min_level_filter(min_level)?;
    let split = now - w;
    let lo = split - b;
    let stats = store.split_counts(lo, split, now).map_err(db)?;
    let rel = Releases::load(store)?;
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

    // (group, kind order, level, count) for sorting; group 1 = good news, listed last.
    let mut found: Vec<((u8, u8, u8, i64), Value)> = vec![];
    let mut tally = [0usize; 4];
    for SplitStat { row, before, after } in &stats {
        let lvl = level_rank(row.level.as_deref());
        if lvl < min_rank {
            continue;
        }
        let (a_rate, b_rate) = (per_hour(*after, w), per_hour(*before, b));
        let (kind, order, note) = if *after > 0 && row.first_seen >= split {
            ("new", 0u8, String::new())
        } else if *after > 0 && *before == 0 {
            (
                "returning",
                1,
                format!(
                    "Back after a quiet baseline; first seen {}.",
                    fmt_t(row.first_seen, now, d)
                ),
            )
        } else if *after >= 5 && *before > 0 && a_rate >= 3.0 * b_rate * traffic.max(1.0) {
            let x = a_rate / b_rate.max(0.001);
            let note = if traffic > 1.2 {
                format!("{x:.1}x its baseline rate, while overall log volume is {traffic:.1}x.")
            } else {
                format!("{x:.1}x its baseline rate.")
            };
            ("spike", 2, note)
        } else if *after == 0 && (*before as f64) * w / b >= 5.0 {
            (
                "gone_quiet",
                3,
                format!(
                    "Expected ~{:.0} in the window from its baseline rate, saw none.",
                    *before as f64 * w / b
                ),
            )
        } else {
            continue;
        };
        tally[order as usize] += 1;
        let mut v = pattern_brief(row, &rel, now, d);
        v["kind"] = json!(kind);
        if !note.is_empty() {
            v["note"] = json!(note);
        }
        v["count_in_window"] = json!(after);
        v["per_hour"] = if d.full() {
            json!({"window": a_rate, "baseline": b_rate})
        } else {
            json!(format!("{a_rate} vs {b_rate} baseline"))
        };
        let group = u8::from(kind == "gone_quiet");
        found.push(((group, 4 - lvl, order, -after), v));
    }
    found.sort_by_key(|a| a.0);
    let more = found.len().saturating_sub(limit);
    let findings: Vec<Value> = found
        .into_iter()
        .take(limit)
        .enumerate()
        .map(|(i, (_, mut v))| {
            if v["kind"] != "gone_quiet"
                && (d.full() || i < BRIEF_SAMPLES)
                && let Some(id) = v["pattern_id"].as_str()
            {
                v["sample"] = json!(latest_sample(store, id, d));
            }
            v
        })
        .collect();

    let summary = if findings.is_empty() {
        format!(
            "Nothing new: {events_in_window} events across {active} known patterns, all within their usual rates."
        )
    } else {
        format!(
            "{} new, {} returning, {} spiking, {} gone quiet. {events_in_window} events across {active} patterns in the window.",
            tally[0], tally[1], tally[2], tally[3]
        )
    };
    let traffic_note = (traffic >= 2.0 || traffic <= 0.5)
        .then(|| format!("Overall log volume is {traffic:.1}x its baseline."));
    Ok(json!({
        "window": if d.full() {
            format!("last {} (since {}) vs the {} before", fmt_duration(w), fmt_time(split, now), fmt_duration(b))
        } else {
            format!("last {} vs prior {}", fmt_duration(w), fmt_duration(b))
        },
        "summary": summary,
        "traffic": if d.full() {
            json!({
                "per_hour_window": per_hour(events_in_window, w),
                "per_hour_baseline": per_hour(events_in_baseline, b),
                "note": traffic_note,
            })
        } else {
            json!({"note": traffic_note})
        },
        "current_release": rel.latest().map(|(_, n)| n),
        "markers_in_window": markers_between(store, split, now, now, d)?,
        "findings": findings,
        "more_not_shown": if more > 0 { Some(more) } else { None },
    }))
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
    let mut base = json!({
        "pattern_id": row.id,
        "level": if d.full() { row.level.clone() } else { None },
        "template": truncate(&row.template, if d.full() { TEMPLATE_CHARS } else { 120 }),
        "split": {"at": fmt_t(split, now, d), "marker": marker},
        "last_seen": fmt_t(row.last_seen, now, d),
    });

    if row.first_seen >= split {
        base["verdict"] = json!("appeared_after");
        base["explanation"] = json!(format!(
            "This pattern did not exist before the split; it first appeared {}. The change likely introduced it.",
            fmt_t(row.first_seen, now, d)
        ));
        base["after"] = json!({"count": store.count(&row.id, split, now).map_err(db)?});
        return Ok(base);
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
    if d.full() {
        base["before"] = json!({"window": format!("{} before the split", fmt_duration(before_secs)), "count": b, "per_hour": per_hour(b, before_secs)});
        base["after"] = json!({"window": format!("{} since the split", fmt_duration(after_secs)), "count": a, "per_hour": per_hour(a, after_secs)});
        base["expected_after_if_unchanged"] = json!((expected * 10.0).round() / 10.0);
    } else {
        base["before"] = json!(count_str(b, before_secs));
        base["after"] = json!(count_str(a, after_secs));
    }

    let (verdict, explanation) = if b == 0 {
        (
            "not_active_before".to_string(),
            format!(
                "No occurrences in the {} before the split, so there is nothing to compare. Try a longer 'before' window.",
                fmt_duration(before_secs)
            ),
        )
    } else if a == 0 {
        if expected >= MIN_EXPECTED {
            let conf = if expected >= 2.0 * MIN_EXPECTED {
                "high"
            } else {
                "medium"
            };
            base["confidence"] = json!(conf);
            (
                "fixed".to_string(),
                format!(
                    "At the old rate about {expected:.0} would have occurred since the split; none did."
                ),
            )
        } else {
            let wait = ((MIN_EXPECTED / b_rate - after_secs) / 60.0)
                .ceil()
                .max(1.0);
            base["wait_minutes"] = json!(wait);
            (
                "too_early".to_string(),
                format!(
                    "None since the split, but at the old rate only ~{expected:.1} were expected yet. Check again in about {wait:.0} minutes."
                ),
            )
        }
    } else {
        let ratio = a_rate / b_rate;
        if ratio >= 2.0 {
            (
                "worse".to_string(),
                format!("Happening {ratio:.1}x as often as before the split."),
            )
        } else if ratio <= 0.5 && expected >= MIN_EXPECTED {
            (
                "reduced".to_string(),
                format!(
                    "Down to {:.0}% of its earlier rate but still occurring.",
                    ratio * 100.0
                ),
            )
        } else {
            (
                "still_occurring".to_string(),
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
            base["caveat"] = json!(format!(
                "Overall log volume dropped to {:.0}% of its earlier rate after the split; low traffic can hide errors.",
                traffic * 100.0
            ));
        }
    }
    base["verdict"] = json!(verdict);
    base["explanation"] = json!(explanation);
    Ok(base)
}

/// Fleet-wide before/after diff around a deploy or other point in time.
pub fn compare_deploy(
    store: &Store,
    at: Option<&str>,
    before: Option<&str>,
    limit: usize,
    d: Detail,
    now: f64,
) -> AResult {
    let rel = Releases::load(store)?;
    let (split, marker) = match at {
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

    let mut found: Vec<((u8, u8, u8, i64), Value)> = vec![];
    let mut tally = [0usize; 4];
    for SplitStat { row, before, after } in &stats {
        let (b, a) = (*before, *after);
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
        let mut v = pattern_brief(row, &rel, now, d);
        v["kind"] = json!(kind);
        if d.full() {
            v["before"] = json!({"count": b, "per_hour": per_hour(b, span)});
            v["after"] = json!({"count": a, "per_hour": per_hour(a, after_secs)});
        } else {
            v["before"] = json!(count_str(b, span));
            v["after"] = json!(count_str(a, after_secs));
        }
        let group = u8::from(order >= 2);
        found.push((
            (
                group,
                4 - level_rank(row.level.as_deref()),
                order,
                -(a.max(b)),
            ),
            v,
        ));
    }
    found.sort_by_key(|x| x.0);
    let more = found.len().saturating_sub(limit);
    let findings: Vec<Value> = found.into_iter().take(limit).map(|(_, v)| v).collect();
    Ok(json!({
        "split": {"at": fmt_t(split, now, d), "marker": marker},
        "compared": format!("{} before vs {} after", fmt_duration(span), fmt_duration(after_secs)),
        "summary": format!(
            "{} new since the split, {} increased, {} stopped, {} decreased.",
            tally[0], tally[1], tally[2], tally[3]
        ),
        "traffic_per_hour": format!("{} before, {} after", per_hour(tb, span), per_hour(ta, after_secs)),
        "findings": findings,
        "more_not_shown": if more > 0 { Some(more) } else { None },
    }))
}

pub fn list_patterns(
    store: &Store,
    since: &str,
    min_level: Option<&str>,
    contains: Option<&str>,
    limit: usize,
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
        .take(limit)
        .map(|s| {
            let mut v = pattern_brief(&s.row, &rel, now, d);
            v["count"] = json!(s.before);
            v
        })
        .collect();
    let more = total.saturating_sub(limit);
    Ok(json!({
        "since": fmt_t(lo, now, d),
        "patterns": items,
        "more_not_shown": if more > 0 { Some(more) } else { None },
    }))
}

pub fn get_pattern(
    store: &Store,
    pattern_id: &str,
    samples: Option<i64>,
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
    v["total_seen"] = json!(row.total);
    v["count_last_hour"] = json!(store.count(&row.id, now - 3600.0, now).map_err(db)?);
    v["count_last_24h"] = json!(store.count(&row.id, day, now).map_err(db)?);
    v["hourly_last_24h"] = json!(store.histogram(&row.id, day, now, 3600.0).map_err(db)?);
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
    Ok(v)
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

pub fn list_markers(store: &Store, limit: i64, now: f64) -> AResult {
    let ms = store
        .markers(f64::MIN, f64::MAX, limit.clamp(1, 100))
        .map_err(db)?;
    Ok(json!({
        "markers": ms.into_iter().map(|m| json!({"name": m.name, "kind": m.kind, "at": fmt_time(m.ts, now), "note": m.note})).collect::<Vec<_>>()
    }))
}

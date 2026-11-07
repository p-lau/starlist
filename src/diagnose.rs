//! One answer to "what is wrong right now?": the likely causes, ranked, each with why it
//! stands out, the deploy it started with, its code location and a few lines. Also the
//! slow-rise test that `whats_new` uses for increases too small to count as a spike.

use crate::analysis::{
    self, AResult, Detail, Releases, db, fmt_duration, fmt_t, min_level_filter, new_window,
    per_hour, sample_text, trim_trace, whats_new_data,
};
use crate::pattern::{level_rank, truncate};
use crate::store::Store;
use serde_json::{Value, json};
use std::collections::HashMap;

/// A rise needs at least this many events in its window...
const RISE_MIN_COUNT: i64 = 20;
/// ...this many expected from its usual share, so a ratio means something...
const RISE_MIN_EXPECTED: f64 = 3.0;
/// ...to be at least this much above its usual share of the logs...
const RISE_MIN_RATIO: f64 = 1.3;
/// ...by this many standard deviations (Poisson), so noise doesn't count.
const RISE_MIN_Z: f64 = 3.0;
/// Gradual rises are also looked for over this long a period, against the day before it.
const SLOW_WINDOW: f64 = 3.0 * 3600.0;
const DAY: f64 = 24.0 * 3600.0;
/// Resolution of the history a rise is compared with.
const HIST_STEP: f64 = 300.0;
/// With fewer earlier periods than this to compare with, a rise needs a stronger z.
const HIST_MIN: usize = 6;
const RISE_MIN_Z_YOUNG: f64 = 5.0;
/// A rise must beat the day's highest earlier result by this factor.
const PAST_MARGIN: f64 = 1.15;

/// A pattern that takes a bigger share of the logs than it used to, by less than a spike.
#[derive(Debug, Clone, PartialEq)]
pub struct Rise {
    /// Events in the recent period.
    pub after: i64,
    /// Events expected there had the pattern kept its share of all events.
    pub expected: f64,
    pub ratio: f64,
    /// Standard deviations above the expected count.
    pub z: f64,
    /// The recent period and the period before it that it was compared with, in seconds.
    pub window: f64,
    pub baseline: f64,
    /// The highest ratio the same test gave over the day before, when there was enough history.
    pub past_max: Option<f64>,
}

impl Rise {
    pub fn note(&self) -> String {
        format!(
            "Rising: {:.1}x its usual share of the logs over the last {} ({} seen, ~{:.0} expected from the {} before).",
            self.ratio,
            fmt_duration(self.window),
            self.after,
            self.expected,
            fmt_duration(self.baseline)
        ) + &self
            .past_max
            .map(|m| format!(" Highest earlier in the day: {m:.1}x."))
            .unwrap_or_default()
    }
}

/// Patterns whose share of all events grew in the last `window` seconds (against the period
/// just before it, at least 2h) or over the last 3h (against the day before). Comparing
/// shares rather than rates keeps traffic swings out, and the Poisson test keeps small
/// counts from looking like trends. Many patterns also swing on their own (batch jobs,
/// stock running low by evening), so a rise must also beat every result the same test gave
/// over the day before; on a young store, without that history, it needs a stronger z.
/// The strongest rise per pattern is kept.
pub fn rises(store: &Store, window: f64, now: f64) -> Result<HashMap<String, Rise>, String> {
    let start = store.data_start().map_err(db)?.unwrap_or(now - DAY);
    let mut horizons = vec![(window, (3.0 * window).max(2.0 * 3600.0))];
    if window < SLOW_WINDOW {
        horizons.push((SLOW_WINDOW, DAY));
    }
    let mut out: HashMap<String, Rise> = HashMap::new();
    for (w, wanted_b) in horizons {
        let split = now - w;
        let lo = (split - wanted_b).max(start);
        let b = split - lo;
        if b < w.max(1800.0) {
            continue;
        }
        let stats = store.split_counts(lo, split, now).map_err(db)?;
        let total_before: i64 = stats.iter().map(|s| s.before).sum();
        let total_after: i64 = stats.iter().map(|s| s.after).sum();
        if total_before == 0 || total_after == 0 {
            continue;
        }
        let scale = total_after as f64 / total_before as f64;
        let mut totals: Option<Vec<i64>> = None;
        let hist_lo = (split - DAY - wanted_b).max(start);
        for s in stats {
            if s.after < RISE_MIN_COUNT || s.before == 0 {
                continue;
            }
            let expected = s.before as f64 * scale;
            if expected < RISE_MIN_EXPECTED {
                continue;
            }
            let ratio = s.after as f64 / expected;
            let z = (s.after as f64 - expected) / expected.sqrt();
            if ratio < RISE_MIN_RATIO || z < RISE_MIN_Z {
                continue;
            }
            if totals.is_none() {
                totals = Some(total_series(store, hist_lo, split)?);
            }
            let p = store
                .histogram(&s.row.id, hist_lo, split, HIST_STEP)
                .map_err(db)?;
            let past = past_ratios(&p, totals.as_deref().unwrap_or(&[]), w, wanted_b);
            let past_max = if past.len() >= HIST_MIN {
                let m = past.iter().copied().fold(0.0, f64::max);
                if ratio <= m * PAST_MARGIN {
                    continue;
                }
                Some(m)
            } else if z < RISE_MIN_Z_YOUNG {
                continue;
            } else {
                None
            };
            let r = Rise {
                after: s.after,
                expected,
                ratio,
                z,
                window: w,
                baseline: b,
                past_max,
            };
            match out.get(&s.row.id) {
                Some(old) if old.z >= r.z => {}
                _ => {
                    out.insert(s.row.id.clone(), r);
                }
            }
        }
    }
    Ok(out)
}

/// All events per `HIST_STEP` from `lo` to `hi`.
fn total_series(store: &Store, lo: f64, hi: f64) -> Result<Vec<i64>, String> {
    let series = store.level_series(lo, hi, HIST_STEP).map_err(db)?;
    let n = series.first().map(|(_, c)| c.len()).unwrap_or(0);
    let mut out = vec![0i64; n];
    for (_, c) in series {
        for (o, x) in out.iter_mut().zip(c) {
            *o += x;
        }
    }
    Ok(out)
}

/// The rise test (share in a `w` window against up to `b` before it) at earlier positions,
/// stepping back half a window at a time from the end of `p` and `t` (a pattern's and all
/// events' counts per `HIST_STEP`). Positions with too few expected events are skipped.
fn past_ratios(p: &[i64], t: &[i64], w: f64, b: f64) -> Vec<f64> {
    let n = p.len().min(t.len());
    let wn = ((w / HIST_STEP).round() as usize).max(1);
    let bn_max = ((b / HIST_STEP).round() as usize).max(wn);
    let step = (wn / 2).max(1);
    let sum = |v: &[i64], a: usize, z: usize| v[a..z].iter().sum::<i64>() as f64;
    let mut out = vec![];
    let mut end = n;
    while end >= 2 * wn {
        let mid = end - wn;
        let bn = bn_max.min(mid);
        let (pb, tb, ta) = (
            sum(p, mid - bn, mid),
            sum(t, mid - bn, mid),
            sum(t, mid, end),
        );
        if pb > 0.0 && tb > 0.0 {
            let expected = pb * ta / tb;
            if expected >= RISE_MIN_EXPECTED {
                out.push(sum(p, mid, end) / expected);
            }
        }
        end -= step;
    }
    out
}

/// Lines of one request, first line of each event only, for showing a cause in context.
fn request_lines(store: &Store, id: &str, max: usize) -> Result<Vec<String>, String> {
    let lines = store.request_lines(id, max as i64 + 1).map_err(db)?;
    let Some(start) = lines.first().map(|l| l.ts) else {
        return Ok(vec![]);
    };
    let mut out: Vec<String> = lines
        .iter()
        .take(max)
        .map(|l| {
            let first = l.text.lines().next().unwrap_or("");
            format!("+{:.2}s {}", l.ts - start, truncate(first, 200))
        })
        .collect();
    if lines.len() > max {
        out.push("… more in get_request".into());
    }
    Ok(out)
}

fn json_len(v: &Value) -> usize {
    serde_json::to_string(v).map(|s| s.len()).unwrap_or(0)
}

/// The likely causes of trouble in the window (since the latest deploy when it is recent,
/// else the last hour), most likely first. Each says why it stands out, the deploy it
/// started with, where in the code it comes from, a few sample lines, and for the top one
/// the lines of a request that hit it. Built for agents that don't know the symptom yet,
/// so one call is usually enough to go and read the code.
pub fn diagnose(
    store: &Store,
    window: Option<&str>,
    min_level: Option<&str>,
    limit: usize,
    max_chars: usize,
    d: Detail,
    now: f64,
) -> AResult {
    let min_rank = min_level_filter(min_level)?;
    let nw = new_window(store, window, None, now)?;
    let data = whats_new_data(store, nw.window, nw.baseline, min_level, now)?;
    let rel = Releases::load(store)?;
    let w = data.now - data.window_from;
    let period = match &nw.deploy {
        Some(name) => format!("since deploy {name} ({} ago)", fmt_duration(w)),
        None => format!("last {}", fmt_duration(w)),
    };

    let mut causes: Vec<Value> = vec![];
    let mut also: Vec<Value> = vec![];
    for (i, f) in data.findings.iter().enumerate() {
        let row = &f.row;
        let started_with = if f.kind == "new" {
            rel.at(row.first_seen).map(str::to_string)
        } else {
            nw.deploy.clone()
        };
        if i >= limit {
            if also.len() < 5 {
                also.push(json!({
                    "pattern_id": row.id,
                    "kind": f.kind,
                    "template": truncate(&row.template, 120),
                }));
            }
            continue;
        }
        let why = match f.kind {
            "new" => format!(
                "New: {} in {} ({}/h), first seen {}.",
                f.count_window,
                fmt_duration(w),
                f.per_hour_window,
                fmt_t(row.first_seen, now, Detail::Brief)
            ),
            _ => {
                let rate = format!(
                    " {}/h now vs {}/h before.",
                    f.per_hour_window, f.per_hour_baseline
                );
                format!("{}{}", f.note.clone().unwrap_or_default(), rate)
            }
        };
        let n_samples = match (f.kind, i, d.full()) {
            ("gone_quiet", _, _) => 0,
            (_, 0, _) | (_, _, true) => 2,
            _ => 1,
        };
        let samples: Vec<String> = store
            .samples(&row.id, n_samples)
            .map_err(db)?
            .into_iter()
            .map(|(_, t)| sample_text(&t, d))
            .collect();
        let mut c = json!({
            "pattern_id": row.id,
            "kind": f.kind,
            "level": row.level,
            "template": truncate(&row.template, analysis::TEMPLATE_CHARS),
            "why": why,
            "started_with": started_with,
            "location": row.location,
            "samples": samples,
        });
        if let Some(req) = &row.example_request {
            c["request_id"] = json!(req);
            if causes.is_empty() && f.kind != "gone_quiet" {
                let lines = request_lines(store, req, if d.full() { 20 } else { 8 })?;
                if lines.len() > 1 {
                    c["request"] = json!(lines);
                }
            }
        }
        causes.push(c);
    }

    let verdict = match data.findings.first() {
        None => format!(
            "Nothing stands out {period}: {} events across {} patterns, all at their usual rates and shares.",
            data.events_in_window, data.active
        ),
        Some(f) => {
            let what = match f.kind {
                "new" => "a new",
                "regressed" => "a regressed",
                "returning" => "a returning",
                "spike" => "a spiking",
                "rising" => "a rising",
                _ => "a now-silent",
            };
            let level = f.row.level.as_deref().unwrap_or("INFO");
            let at = f
                .row
                .location
                .as_deref()
                .map(|l| format!(" at {l}"))
                .unwrap_or_default();
            format!(
                "Most likely {what} {level} pattern{at} ({period}). {}",
                data.summary()
            )
        }
    };

    let mut v = json!({
        "window": period,
        "verdict": verdict,
        "note": nw.note,
        "causes": causes,
    });
    if !also.is_empty() {
        v["also_changed"] = json!(also);
    }
    if data.findings.is_empty() {
        // Nothing changed, so the answer is in what was already wrong: the busiest
        // warnings and errors, for an agent that has a symptom to match them against.
        let ongoing = ongoing(store, data.window_from, now, min_rank.max(2))?;
        if !ongoing.is_empty() {
            v["ongoing"] = json!(ongoing);
        }
    } else {
        v["next"] = json!(
            "Read the code at the location. get_pattern or get_request for more; after a fix, verify_fix."
        );
    }
    if d.full() {
        let traffic = data.traffic;
        v["traffic"] = json!(format!(
            "{}/h now vs {}/h before ({traffic:.2}x)",
            per_hour(data.events_in_window, w),
            per_hour(data.events_in_baseline, nw.baseline)
        ));
    }
    Ok(fit(analysis::compact(v), max_chars))
}

/// The busiest patterns at `min_rank` or above since `lo`.
fn ongoing(store: &Store, lo: f64, now: f64, min_rank: u8) -> Result<Vec<Value>, String> {
    let mut stats: Vec<_> = store
        .split_counts(lo, lo, now)
        .map_err(db)?
        .into_iter()
        .filter(|s| s.after > 0 && level_rank(s.row.level.as_deref()) >= min_rank)
        .collect();
    stats.sort_by_key(|s| {
        (
            std::cmp::Reverse(level_rank(s.row.level.as_deref())),
            -s.after,
        )
    });
    Ok(stats
        .into_iter()
        .take(5)
        .map(|s| {
            json!({
                "pattern_id": s.row.id,
                "level": s.row.level,
                "template": truncate(&s.row.template, 160),
                "count": s.after,
                "location": s.row.location,
            })
        })
        .collect())
}

/// Shorten an answer to `max_chars`: the request lines, then samples, then whole causes go.
fn fit(mut v: Value, max_chars: usize) -> Value {
    if json_len(&v) <= max_chars {
        return v;
    }
    let mut note = "Shortened to fit max_chars; get_pattern and get_request have the rest.";
    if let Some(c) = v["causes"].get_mut(0).and_then(Value::as_object_mut) {
        c.remove("request");
    }
    for keep in [Some(1usize), None] {
        if json_len(&v) <= max_chars {
            break;
        }
        if let Some(cs) = v["causes"].as_array_mut() {
            for c in cs.iter_mut() {
                let Some(o) = c.as_object_mut() else { continue };
                match keep {
                    Some(n) => {
                        if let Some(Value::Array(s)) = o.get_mut("samples") {
                            s.truncate(n);
                            for t in s.iter_mut() {
                                if let Value::String(x) = t {
                                    *x = truncate(&trim_trace(x, 4), 200);
                                }
                            }
                        }
                    }
                    None => {
                        o.remove("samples");
                    }
                }
            }
        }
    }
    v.as_object_mut().map(|o| o.remove("also_changed"));
    while json_len(&v) > max_chars && v["causes"].as_array().is_some_and(|c| c.len() > 1) {
        v["causes"].as_array_mut().map(|c| c.pop());
        note = "Fewer causes shown to fit max_chars; whats_new lists every change.";
    }
    v["trimmed"] = json!(note);
    v
}

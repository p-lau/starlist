//! Errors that come at certain times of day. A job that fails around 02:00 every night
//! isn't a new spike each night, and a quiet afternoon doesn't prove it fixed. Spikes are
//! compared with the same time on earlier days and labelled when they are usual, and
//! `verify_fix` waits until a pattern's usual hours have come round. Times are UTC.

use crate::analysis::db;
use crate::store::Store;
use std::collections::HashMap;

const DAY: f64 = 24.0 * 3600.0;
const HOUR: f64 = 3600.0;
/// How many earlier days are looked at.
const LOOKBACK_DAYS: usize = 7;
/// A pattern needs to have been seen on this many of them to have usual times.
const MIN_DAYS: usize = 3;
/// A spike is usual unless it beats the highest count at the same time on earlier days by this.
const USUAL_MARGIN: f64 = 1.5;
/// Earlier days are searched this far either side of the same clock time, for jobs that drift.
const SLACK: f64 = 1800.0;
/// The usual hours must hold this share of a pattern's events, and be at most half the day.
const MIN_SHARE: f64 = 0.8;
const MAX_HOURS: usize = 12;

/// A per-pattern override of the time-of-day rules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Judged from history.
    Auto,
    /// Expected at set times: spikes are usual unless clearly worse than earlier days.
    Usual,
    /// Never labelled usual.
    AlwaysAlert,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Usual => "usual",
            Mode::AlwaysAlert => "always_alert",
        }
    }

    /// Parse a user's value; `None` for an unknown one.
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Some(Mode::Auto),
            "usual" | "expected" | "scheduled" => Some(Mode::Usual),
            "always_alert" | "always" | "never_usual" => Some(Mode::AlwaysAlert),
            _ => None,
        }
    }
}

/// Stored overrides, loaded once per answer.
pub struct Modes(HashMap<String, String>);

impl Modes {
    pub fn load(store: &Store) -> Result<Self, String> {
        Ok(Self(store.timings().map_err(db)?))
    }

    pub fn get(&self, pattern_id: &str) -> Mode {
        self.0
            .get(pattern_id)
            .and_then(|m| Mode::parse(m))
            .unwrap_or(Mode::Auto)
    }
}

/// Set or clear (`auto`) a pattern's override.
pub fn set_mode(store: &Store, pattern_id: &str, mode: Mode) -> Result<(), String> {
    let m = (mode != Mode::Auto).then(|| mode.name());
    store.set_timing(pattern_id, m).map_err(db)
}

/// How a spike compares with the same time of day on earlier days.
#[derive(Debug, Clone, PartialEq)]
pub enum SameTime {
    /// Within what earlier days saw; the note says so.
    Usual(String),
    /// Seen at this time before, but not this much; the note says so.
    Worse(String),
    /// Nothing to say.
    Unknown,
}

/// Compare `after` events in `[split, now]` with the same clock time on up to 7 earlier
/// days. For each day the highest count in any window as long as this one, within half an
/// hour either side, is taken, so a job that runs a little earlier or later still matches.
pub fn same_time(
    store: &Store,
    pattern_id: &str,
    mode: Mode,
    after: i64,
    split: f64,
    now: f64,
    data_start: f64,
) -> Result<SameTime, String> {
    if mode == Mode::AlwaysAlert {
        return Ok(SameTime::Unknown);
    }
    let w = (now - split).max(10.0);
    let step = (w / 6.0).max(10.0);
    let n = ((w / step).round() as usize).max(1);
    let (mut days, mut seen, mut max) = (0usize, 0usize, 0i64);
    for k in 1..=LOOKBACK_DAYS {
        let lo = split - k as f64 * DAY - SLACK;
        if lo < data_start {
            break;
        }
        let hi = now - k as f64 * DAY + SLACK;
        let h = store.histogram(pattern_id, lo, hi, step).map_err(db)?;
        let best = h
            .windows(n.min(h.len()))
            .map(|x| x.iter().sum::<i64>())
            .max()
            .unwrap_or(0);
        days += 1;
        if best > 0 {
            seen += 1;
        }
        max = max.max(best);
    }
    let within = after as f64 <= USUAL_MARGIN * max as f64;
    let history = format!(
        "seen around this time on {seen} of the last {days} days, at most {max} in a window like this"
    );
    Ok(match mode {
        Mode::Usual if seen == 0 => SameTime::Usual("Marked as expected at set times.".into()),
        Mode::Usual if within => {
            SameTime::Usual(format!("Marked as expected at set times; {history}."))
        }
        _ if seen >= MIN_DAYS && within => {
            SameTime::Usual(format!("Usual for this time of day: {history}."))
        }
        _ if seen >= MIN_DAYS || mode == Mode::Usual => {
            SameTime::Worse(format!("Worse than usual for this time of day: {history}."))
        }
        _ => SameTime::Unknown,
    })
}

/// The hours of the day (UTC) a pattern usually occurs in, from the last 7 days.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub hours: [bool; 24],
    /// Full days looked at, and days the pattern was seen on.
    pub days: usize,
    pub active_days: usize,
    /// Events in the usual hours per day looked at, on average.
    pub per_day: f64,
}

fn hour_of_day(t: f64) -> usize {
    ((t.rem_euclid(DAY)) / HOUR) as usize % 24
}

/// A pattern's usual hours, when it has them: seen on at least 3 of the last 7 days, with
/// most of its events in the same hours (at most half the day) each time.
pub fn profile(
    store: &Store,
    pattern_id: &str,
    now: f64,
    data_start: f64,
) -> Result<Option<Profile>, String> {
    let end = (now / HOUR).floor() * HOUR;
    let days = (((end - data_start) / DAY).floor().max(0.0) as usize).min(LOOKBACK_DAYS);
    if days < MIN_DAYS {
        return Ok(None);
    }
    let lo = end - days as f64 * DAY;
    let h = store.histogram(pattern_id, lo, end, HOUR).map_err(db)?;
    let mut hit = [0usize; 24];
    let mut total = [0i64; 24];
    let mut active_days = 0;
    for (d, day) in h.chunks(24).enumerate() {
        let mut any = false;
        for (i, &c) in day.iter().enumerate() {
            if c > 0 {
                let hod = hour_of_day(lo + ((d * 24 + i) as f64) * HOUR);
                hit[hod] += 1;
                total[hod] += c;
                any = true;
            }
        }
        active_days += usize::from(any);
    }
    if active_days < MIN_DAYS {
        return Ok(None);
    }
    let need = 2.max(active_days.div_ceil(2));
    let mut hours = [false; 24];
    for i in 0..24 {
        hours[i] = hit[i] >= need;
    }
    let n_hours = hours.iter().filter(|x| **x).count();
    let all: i64 = total.iter().sum();
    let usual: i64 = (0..24).filter(|i| hours[*i]).map(|i| total[i]).sum();
    if n_hours == 0 || n_hours > MAX_HOURS || (usual as f64) < MIN_SHARE * all as f64 {
        return Ok(None);
    }
    Ok(Some(Profile {
        hours,
        days,
        active_days,
        per_day: usual as f64 / days as f64,
    }))
}

impl Profile {
    /// The usual hours as ranges, like "01:00-03:00 UTC".
    pub fn describe(&self) -> String {
        let mut runs: Vec<(usize, usize)> = vec![];
        // Start from an hour that isn't usual so a run across midnight stays whole.
        let first = (0..24).find(|i| !self.hours[*i]).unwrap_or(0);
        let mut cur: Option<usize> = None;
        for k in 1..=24 {
            let i = (first + k) % 24;
            match (self.hours[i], cur) {
                (true, None) => cur = Some(i),
                (false, Some(s)) => {
                    runs.push((s, i));
                    cur = None;
                }
                _ => {}
            }
        }
        if let Some(s) = cur {
            runs.push((s, first));
        }
        let r: Vec<String> = runs
            .iter()
            .map(|(a, b)| format!("{a:02}:00-{b:02}:00"))
            .collect();
        format!("{} UTC", r.join(", "))
    }

    /// When every usual hour has come round in full since `split`: `Ok(n)` with the fewest
    /// times any of them has by `now`, or `Err(t)` with the time the last one will have.
    pub fn passed_since(&self, split: f64, now: f64) -> Result<usize, f64> {
        let first = (split / HOUR).ceil() * HOUR;
        let mut counts = [0usize; 24];
        let mut t = first;
        while t + HOUR <= now {
            counts[hour_of_day(t)] += 1;
            t += HOUR;
        }
        let pending: Vec<usize> = (0..24)
            .filter(|i| self.hours[*i] && counts[*i] == 0)
            .collect();
        if pending.is_empty() {
            return Ok((0..24)
                .filter(|i| self.hours[*i])
                .map(|i| counts[i])
                .min()
                .unwrap_or(0));
        }
        // The next whole occurrence of each pending hour ends after `now`; wait for the last.
        let at = pending
            .iter()
            .map(|&h| {
                let mut s = first;
                while hour_of_day(s) != h {
                    s += HOUR;
                }
                s + HOUR
            })
            .fold(now, f64::max);
        Err(at)
    }

    /// A short note for verify_fix and get_pattern.
    pub fn note(&self) -> String {
        format!(
            "Usually occurs around {} (seen on {} of the last {} days).",
            self.describe(),
            self.active_days,
            self.days
        )
    }
}

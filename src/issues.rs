//! Patterns as issues: resolve, ignore, notice regressions, and see what each
//! release changed. States are stored per pattern; what a state means *now*
//! (still resolved, regressed, ignore expired) is worked out when asked, so it
//! is right whichever process collects the logs.

use crate::analysis::{Releases, db, fmt_duration, parse_duration};
use crate::merge::MergeOp;
use crate::pattern::level_rank;
use crate::store::{PatternRow, StateRow, Store};
use std::collections::HashMap;

/// What a pattern's state means at a given time.
#[derive(Debug, Clone, PartialEq)]
pub enum Effective {
    Open,
    /// Resolved; occurrences after `at` reopen it.
    Resolved {
        at: f64,
    },
    /// Resolved in the next release, which hasn't been deployed yet.
    ResolvedNext,
    /// Left out of whats_new and alerts until a time or a count.
    Ignored {
        until: Option<f64>,
        remaining: Option<i64>,
    },
    /// Resolved, then seen again at `at`.
    Regressed {
        resolved_at: f64,
        at: f64,
    },
}

impl Effective {
    pub fn name(&self) -> &'static str {
        match self {
            Effective::Open => "open",
            Effective::Resolved { .. } => "resolved",
            Effective::ResolvedNext => "resolved_next",
            Effective::Ignored { .. } => "ignored",
            Effective::Regressed { .. } => "regressed",
        }
    }
    pub fn is_ignored(&self) -> bool {
        matches!(self, Effective::Ignored { .. })
    }
}

/// All stored states, resolved against the store at `now`.
pub struct States {
    rows: HashMap<String, StateRow>,
    deploys: Vec<(f64, String)>,
}

impl States {
    pub fn load(store: &Store) -> Result<Self, String> {
        Ok(Self {
            rows: store.states().map_err(db)?,
            deploys: store.deploys().map_err(db)?,
        })
    }

    pub fn row(&self, pattern_id: &str) -> Option<&StateRow> {
        self.rows.get(pattern_id)
    }

    /// When a resolve takes effect: at once, or at the first deploy after it
    /// for "resolved in the next release".
    fn resolved_at(&self, s: &StateRow) -> Option<f64> {
        match s.state.as_str() {
            "resolved" => Some(s.since),
            "resolved_next" => self
                .deploys
                .iter()
                .find(|(t, _)| *t > s.since)
                .map(|(t, _)| *t),
            _ => None,
        }
    }

    pub fn effective(&self, store: &Store, row: &PatternRow, now: f64) -> Effective {
        let Some(s) = self.rows.get(&row.id) else {
            return Effective::Open;
        };
        match s.state.as_str() {
            "ignored" => {
                let remaining = s.until_count.map(|c| c - (row.total - s.base_total));
                let expired =
                    s.until_ts.is_some_and(|t| now >= t) || remaining.is_some_and(|r| r <= 0);
                if expired {
                    Effective::Open
                } else {
                    Effective::Ignored {
                        until: s.until_ts,
                        remaining,
                    }
                }
            }
            "resolved" | "resolved_next" => match self.resolved_at(s) {
                None => Effective::ResolvedNext,
                Some(at) if row.last_seen <= at => Effective::Resolved { at },
                Some(at) => match store.first_after(&row.id, at).ok().flatten() {
                    Some(back) => Effective::Regressed {
                        resolved_at: at,
                        at: back,
                    },
                    None => Effective::Resolved { at },
                },
            },
            _ => Effective::Open,
        }
    }
}

/// Set a pattern's state: "open", "resolved", "resolved_next" or "ignored".
/// `ignore_for` (a duration) and `ignore_count` bound an ignore.
pub fn set_state(
    store: &Store,
    pattern_id: &str,
    state: &str,
    ignore_for: Option<&str>,
    ignore_count: Option<i64>,
    note: Option<&str>,
    now: f64,
) -> Result<(PatternRow, Effective), String> {
    let row = store
        .pattern(pattern_id)
        .map_err(db)?
        .ok_or_else(|| format!("Unknown pattern_id '{pattern_id}'."))?;
    let state = state.trim().to_ascii_lowercase();
    let stored = match state.as_str() {
        "open" | "reopen" | "unresolve" => None,
        "resolved"
        | "resolve"
        | "resolved_next"
        | "resolved_in_next_release"
        | "ignored"
        | "ignore" => {
            let name = match state.as_str() {
                "resolved" | "resolve" => "resolved",
                "ignored" | "ignore" => "ignored",
                _ => "resolved_next",
            };
            let until_ts = match ignore_for.map(str::trim).filter(|s| !s.is_empty()) {
                Some(d) if name == "ignored" => Some(
                    now + parse_duration(d).filter(|x| *x > 0.0).ok_or_else(|| {
                        format!("ignore_for must be a duration like '24h', got '{d}'")
                    })?,
                ),
                _ => None,
            };
            let until_count = match ignore_count {
                Some(c) if c < 1 => return Err("ignore_count must be at least 1.".into()),
                Some(c) if name == "ignored" => Some(c),
                _ => None,
            };
            Some(StateRow {
                pattern_id: row.id.clone(),
                state: name.to_string(),
                since: now,
                until_ts,
                until_count,
                base_total: row.total,
                note: note
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(str::to_string),
            })
        }
        o => {
            return Err(format!(
                "Unknown state '{o}'. Use open, resolved, resolved_next or ignored."
            ));
        }
    };
    store.set_state(&row.id, stored.as_ref()).map_err(db)?;
    let eff = States::load(store)?.effective(store, &row, now);
    Ok((row, eff))
}

/// Merge patterns into the first one by hand. The other IDs keep resolving to it.
pub fn merge(store: &Store, ids: &[String]) -> Result<PatternRow, String> {
    if ids.len() < 2 {
        return Err("Pass at least two pattern IDs; the first one is kept.".into());
    }
    let mut rows: Vec<PatternRow> = vec![];
    for id in ids {
        let r = store
            .pattern(id)
            .map_err(db)?
            .ok_or_else(|| format!("Unknown pattern_id '{id}'."))?;
        if !rows.iter().any(|x| x.id == r.id) {
            rows.push(r);
        }
    }
    if rows.len() < 2 {
        return Err("Those IDs are already the same pattern.".into());
    }
    let keep = &rows[0];
    store
        .merge_patterns(&MergeOp {
            canonical: keep.id.clone(),
            template: keep.template.clone(),
            aliases: rows[1..].iter().map(|r| r.id.clone()).collect(),
        })
        .map_err(db)?;
    store
        .pattern(&keep.id)
        .map_err(db)?
        .ok_or_else(|| "The merged pattern disappeared.".into())
}

/// One release's report card.
#[derive(Debug, Clone)]
pub struct Release {
    pub name: String,
    pub at: f64,
    /// When the next release replaced it, if one did.
    pub until: Option<f64>,
    pub previous: Option<String>,
    pub env: Option<String>,
    pub events_per_hour: f64,
    pub errors_per_hour: f64,
    /// The previous release's error rate, over at most as long as this one has run.
    pub previous_errors_per_hour: Option<f64>,
    /// Patterns first seen while this release was live.
    pub new: Vec<PatternRow>,
    /// Resolved patterns that came back while it was live.
    pub regressed: Vec<(PatternRow, f64)>,
    /// Patterns resolved while it was live (or resolved for this release).
    pub resolved: Vec<(PatternRow, f64)>,
}

fn rates(store: &Store, lo: f64, hi: f64) -> Result<(f64, f64), String> {
    let span = (hi - lo).max(1.0);
    let series = store.level_series(lo, hi, span).map_err(db)?;
    let (mut all, mut err) = (0i64, 0i64);
    for (level, c) in series {
        let n: i64 = c.iter().sum();
        all += n;
        if level_rank(Some(&level)) >= 3 {
            err += n;
        }
    }
    let ph = |n: i64| (n as f64 * 3600.0 / span * 10.0).round() / 10.0;
    Ok((ph(all), ph(err)))
}

/// What a release changed. `name` defaults to the live release.
pub fn release(store: &Store, name: Option<&str>, now: f64) -> Result<Release, String> {
    let deploys = store.deploys().map_err(db)?;
    if deploys.is_empty() {
        return Err("No deploys recorded yet. starlist records one from lines like 'Starting api version 1.4.2', from STARLIST_VERSION, or with `starlist mark --deploy NAME`.".into());
    }
    let i = match name.map(str::trim).filter(|n| !n.is_empty()) {
        None => deploys.len() - 1,
        Some(n) => deploys.iter().rposition(|(_, d)| d == n).ok_or_else(|| {
            format!("No marker named '{n}' among the deploys. Call list_markers to see releases.")
        })?,
    };
    let (at, rel_name) = deploys[i].clone();
    let until = deploys.get(i + 1).map(|(t, _)| *t);
    let hi = until.unwrap_or(now).min(now);
    let (events_per_hour, errors_per_hour) = rates(store, at, hi)?;
    let previous = i.checked_sub(1).map(|j| deploys[j].clone());
    let previous_errors_per_hour = match &previous {
        Some((pt, _)) => {
            let lo = pt.max(at - (hi - at));
            Some(rates(store, lo, at)?.1)
        }
        None => None,
    };
    let in_period = |t: f64| t >= at && t < until.unwrap_or(f64::INFINITY);
    let states = States::load(store)?;
    let mut regressed = vec![];
    let mut resolved = vec![];
    for id in states.rows.keys() {
        let Some(row) = store.pattern(id).map_err(db)? else {
            continue;
        };
        match states.effective(store, &row, now) {
            Effective::Regressed { at: back, .. } if in_period(back) => regressed.push((row, back)),
            Effective::Resolved { at: r } if in_period(r) => resolved.push((row, r)),
            _ => {}
        }
    }
    let rank = |r: &PatternRow| std::cmp::Reverse(level_rank(r.level.as_deref()));
    regressed.sort_by_key(|(r, _)| rank(r));
    resolved.sort_by_key(|(r, _)| rank(r));
    let mut new = store
        .patterns_first_seen(at, until.unwrap_or(f64::INFINITY))
        .map_err(db)?;
    new.sort_by_key(|r| (rank(r), std::cmp::Reverse(r.total)));
    Ok(Release {
        name: rel_name,
        at,
        until,
        previous: previous.map(|(_, n)| n),
        env: store.meta("env").map_err(db)?,
        events_per_hour,
        errors_per_hour,
        previous_errors_per_hour,
        new,
        regressed,
        resolved,
    })
}

/// A sentence about an effective state, for agents.
pub fn describe(e: &Effective, rel: &Releases, now: f64) -> Option<String> {
    let ago = |t: f64| fmt_duration(now - t);
    match e {
        Effective::Open => None,
        Effective::Resolved { at } => Some(format!("Resolved {} ago.", ago(*at))),
        Effective::ResolvedNext => Some("Resolved in the next release (not deployed yet).".into()),
        Effective::Ignored { until, remaining } => Some(match (until, remaining) {
            (Some(t), _) => format!("Ignored for {} more.", fmt_duration(t - now)),
            (None, Some(r)) => format!("Ignored for {r} more events."),
            (None, None) => "Ignored.".into(),
        }),
        Effective::Regressed { resolved_at, at } => Some(format!(
            "Regressed: resolved {} ago, back {} ago{}.",
            ago(*resolved_at),
            ago(*at),
            rel.at(*at).map(|r| format!(" in {r}")).unwrap_or_default()
        )),
    }
}

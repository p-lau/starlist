//! A Prometheus text endpoint (`--prometheus`): event counters per level,
//! patterns per state, and the percentiles of fields read from logs.

use crate::analysis::db;
use crate::issues::States;
use crate::store::Store;
use std::collections::BTreeMap;
use std::fmt::Write;

fn label(v: &str) -> String {
    v.replace('\\', r"\\")
        .replace('"', "\\\"")
        .replace('\n', r"\n")
}

fn header(out: &mut String, name: &str, kind: &str, help: &str) {
    let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} {kind}");
}

/// The exposition text at `now`.
pub fn render(store: &Store, now: f64) -> Result<String, String> {
    let mut out = String::new();
    let totals = store.level_totals().map_err(db)?;

    header(
        &mut out,
        "starlist_events_total",
        "counter",
        "Log events seen, by level.",
    );
    for (level, n, _) in &totals {
        let _ = writeln!(
            out,
            "starlist_events_total{{level=\"{}\"}} {n}",
            label(level)
        );
    }

    header(
        &mut out,
        "starlist_patterns",
        "gauge",
        "Distinct patterns, by level.",
    );
    for (level, _, p) in &totals {
        let _ = writeln!(out, "starlist_patterns{{level=\"{}\"}} {p}", label(level));
    }

    let states = States::load(store)?;
    let mut by_state: BTreeMap<&str, i64> = [
        ("open", 0),
        ("resolved", 0),
        ("resolved_next", 0),
        ("ignored", 0),
        ("regressed", 0),
    ]
    .into();
    let mut stated = 0;
    for id in states.ids() {
        if let Some(row) = store.pattern(id).map_err(db)? {
            stated += 1;
            *by_state
                .entry(states.effective(store, &row, now).name())
                .or_default() += 1;
        }
    }
    let all: i64 = totals.iter().map(|(_, _, p)| p).sum();
    *by_state.entry("open").or_default() += all - stated;
    header(
        &mut out,
        "starlist_patterns_by_state",
        "gauge",
        "Patterns by issue state.",
    );
    for (st, n) in &by_state {
        let _ = writeln!(out, "starlist_patterns_by_state{{state=\"{st}\"}} {n}");
    }

    let fields = store.metric_fields(None).map_err(db)?;
    if !fields.is_empty() {
        header(
            &mut out,
            "starlist_field",
            "gauge",
            "Percentiles of numbers read from log lines (--metric), over the current and previous hour.",
        );
        let mut counts = String::new();
        for f in &fields {
            let h = crate::metrics::window(store, f, None, now - 3600.0, now).map_err(db)?;
            for (q, name) in [(0.5, "0.5"), (0.95, "0.95"), (0.99, "0.99"), (1.0, "1")] {
                if let Some(v) = h.quantile(q) {
                    let _ = writeln!(
                        out,
                        "starlist_field{{field=\"{}\",quantile=\"{name}\"}} {v}",
                        label(f)
                    );
                }
            }
            let _ = writeln!(
                counts,
                "starlist_field_count{{field=\"{}\"}} {}",
                label(f),
                h.n
            );
        }
        header(
            &mut out,
            "starlist_field_count",
            "gauge",
            "Numbers read from log lines over the current and previous hour.",
        );
        out.push_str(&counts);
    }

    header(
        &mut out,
        "starlist_db_bytes",
        "gauge",
        "Size of the starlist database file.",
    );
    let _ = writeln!(out, "starlist_db_bytes {}", store.size_bytes().map_err(db)?);
    Ok(out)
}

//! A Prometheus text endpoint (`--prometheus`): event counters per level,
//! patterns per state, and the percentiles of fields read from logs.

use crate::analysis::db;
use crate::issues::States;
use crate::store::Store;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Arc;

fn label(v: &str) -> String {
    v.replace('\\', r"\\")
        .replace('"', "\\\"")
        .replace('\n', r"\n")
}

fn header(out: &mut String, name: &str, kind: &str, help: &str) {
    let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} {kind}");
}

/// `name{service="x",labels} value`; the service label only on a hub.
fn series(
    out: &mut String,
    name: &str,
    svc: Option<&str>,
    labels: &str,
    value: impl std::fmt::Display,
) {
    let mut l = String::new();
    if let Some(s) = svc {
        let _ = write!(l, "service=\"{}\"", label(s));
    }
    if !labels.is_empty() {
        if !l.is_empty() {
            l.push(',');
        }
        l.push_str(labels);
    }
    if l.is_empty() {
        let _ = writeln!(out, "{name} {value}");
    } else {
        let _ = writeln!(out, "{name}{{{l}}} {value}");
    }
}

/// The exposition text at `now`.
pub fn render(store: &Store, now: f64) -> Result<String, String> {
    render_all(&[(None, store)], now)
}

/// The exposition text of a hub: the same metrics, each header once, with a
/// `service` label on every series.
pub fn render_services(stores: &[(String, Arc<Store>)], now: f64) -> Result<String, String> {
    // One unreadable service must not take the whole hub endpoint down.
    let list: Vec<(Option<&str>, &Store)> = stores
        .iter()
        .filter(|(n, s)| match s.level_totals() {
            Ok(_) => true,
            Err(e) => {
                eprintln!("starlist: metrics: skipping service {n}: {e}");
                false
            }
        })
        .map(|(n, s)| (Some(n.as_str()), s.as_ref()))
        .collect();
    render_all(&list, now)
}

fn render_all(stores: &[(Option<&str>, &Store)], now: f64) -> Result<String, String> {
    let mut out = String::new();
    let totals: Vec<_> = stores
        .iter()
        .map(|(_, st)| st.level_totals().map_err(db))
        .collect::<Result<_, _>>()?;

    header(
        &mut out,
        "starlist_events_total",
        "counter",
        "Log events seen, by level.",
    );
    for ((svc, _), t) in stores.iter().zip(&totals) {
        for (level, n, _) in t {
            series(
                &mut out,
                "starlist_events_total",
                *svc,
                &format!("level=\"{}\"", label(level)),
                n,
            );
        }
    }

    header(
        &mut out,
        "starlist_patterns",
        "gauge",
        "Distinct patterns, by level.",
    );
    for ((svc, _), t) in stores.iter().zip(&totals) {
        for (level, _, p) in t {
            series(
                &mut out,
                "starlist_patterns",
                *svc,
                &format!("level=\"{}\"", label(level)),
                p,
            );
        }
    }

    header(
        &mut out,
        "starlist_patterns_by_state",
        "gauge",
        "Patterns by issue state.",
    );
    for ((svc, store), t) in stores.iter().zip(&totals) {
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
        let all: i64 = t.iter().map(|(_, _, p)| p).sum();
        *by_state.entry("open").or_default() += all - stated;
        for (st, n) in &by_state {
            series(
                &mut out,
                "starlist_patterns_by_state",
                *svc,
                &format!("state=\"{st}\""),
                n,
            );
        }
    }

    let mut fields = vec![];
    for (svc, store) in stores {
        for f in store.metric_fields(None).map_err(db)? {
            let h = crate::metrics::window(store, &f, None, now - 3600.0, now).map_err(db)?;
            fields.push((*svc, f, h));
        }
    }
    if !fields.is_empty() {
        header(
            &mut out,
            "starlist_field",
            "gauge",
            "Percentiles of numbers read from log lines (--metric), over the current and previous hour.",
        );
        for (svc, f, h) in &fields {
            for (q, name) in [(0.5, "0.5"), (0.95, "0.95"), (0.99, "0.99"), (1.0, "1")] {
                if let Some(v) = h.quantile(q) {
                    let l = format!("field=\"{}\",quantile=\"{name}\"", label(f));
                    series(&mut out, "starlist_field", *svc, &l, v);
                }
            }
        }
        header(
            &mut out,
            "starlist_field_count",
            "gauge",
            "Numbers read from log lines over the current and previous hour.",
        );
        for (svc, f, h) in &fields {
            series(
                &mut out,
                "starlist_field_count",
                *svc,
                &format!("field=\"{}\"", label(f)),
                h.n,
            );
        }
    }

    header(
        &mut out,
        "starlist_db_bytes",
        "gauge",
        "Size of the starlist database file.",
    );
    for (svc, store) in stores {
        series(
            &mut out,
            "starlist_db_bytes",
            *svc,
            "",
            store.size_bytes().map_err(db)?,
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_series_carry_service_and_headers_once() {
        let dir = std::env::temp_dir().join(format!("starlist-prom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = Arc::new(Store::open(dir.join("a.db")).unwrap());
        let b = Arc::new(Store::open(dir.join("b.db")).unwrap());
        let text =
            render_services(&[("api".into(), a.clone()), ("web".into(), b)], 1000.0).unwrap();
        assert_eq!(text.matches("# TYPE starlist_db_bytes").count(), 1);
        assert!(text.contains("starlist_db_bytes{service=\"api\"} "));
        assert!(text.contains("starlist_db_bytes{service=\"web\"} "));
        assert!(text.contains("starlist_patterns_by_state{service=\"api\",state=\"open\"} 0"));
        let plain = render(&a, 1000.0).unwrap();
        assert!(plain.contains("starlist_db_bytes "));
        assert!(!plain.contains("service="));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

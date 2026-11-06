//! `starlist export`: patterns, hourly counts, field histograms and markers as
//! newline-delimited JSON, for a warehouse, a spreadsheet or a backup.

use crate::analysis::{db, now};
use crate::api::iso;
use crate::issues::States;
use crate::store::Store;
use serde_json::{Value, json};
use std::io::Write;

/// Write everything from `since` on, one JSON object per line, each with a
/// `type` of "pattern", "count", "field" or "marker". Returns the line count.
pub fn write(store: &Store, since: f64, out: &mut impl Write) -> Result<u64, String> {
    let io = |e: std::io::Error| format!("writing the export: {e}");
    let mut n = 0u64;
    let mut line = |out: &mut dyn Write, v: Value| -> Result<(), String> {
        serde_json::to_writer(&mut *out, &v).map_err(|e| e.to_string())?;
        out.write_all(b"\n").map_err(io)?;
        n += 1;
        Ok(())
    };
    let t = now();
    let states = States::load(store)?;
    for p in store.patterns_seen_since(since).map_err(db)? {
        let state = states.effective(store, &p, t).name();
        line(
            out,
            json!({
                "type": "pattern",
                "id": p.id,
                "level": p.level.as_deref().unwrap_or("INFO"),
                "template": p.template,
                "exception": p.exc_type,
                "location": p.location,
                "first_seen": iso(p.first_seen),
                "last_seen": iso(p.last_seen),
                "total": p.total,
                "state": state,
            }),
        )?;
    }
    let mut err = None;
    store
        .hourly_counts(since, |pid, hour, c| {
            if err.is_none() {
                err = line(
                    out,
                    json!({"type": "count", "pattern_id": pid, "hour": iso(hour as f64 * 3600.0), "n": c}),
                )
                .err();
            }
        })
        .map_err(db)?;
    store
        .metric_rows(since, |pid, field, hour, h| {
            if err.is_none() {
                err = line(
                    out,
                    json!({
                        "type": "field",
                        "pattern_id": pid,
                        "field": field,
                        "hour": iso(hour as f64 * 3600.0),
                        "count": h.n,
                        "sum": h.sum,
                        "min": h.min,
                        "max": h.max,
                        "p50": h.quantile(0.5),
                        "p95": h.quantile(0.95),
                        "p99": h.quantile(0.99),
                    }),
                )
                .err();
            }
        })
        .map_err(db)?;
    if let Some(e) = err {
        return Err(e);
    }
    let mut markers = store.markers(since, f64::INFINITY, i64::MAX).map_err(db)?;
    markers.reverse();
    for m in markers {
        line(
            out,
            json!({"type": "marker", "name": m.name, "kind": m.kind, "at": iso(m.ts), "note": m.note}),
        )?;
    }
    out.flush().map_err(io)?;
    Ok(n)
}

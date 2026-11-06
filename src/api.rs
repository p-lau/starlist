//! Headless JSON API under `/api/v1`, for dashboards and custom UIs.
//!
//! It answers from the same analysis as the MCP tools, but returns plain data
//! for programs: ISO 8601 UTC times, numbers rather than summaries, full sample
//! text, and `limit`/`offset` paging instead of a character budget.

use crate::alerts::Alerts;
use crate::analysis::{self, Releases};
use crate::pattern::{level_rank, truncate};
use crate::store::{Marker, PatternRow, RawLine, Store};
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, SecondsFormat};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

/// Longest sample or raw line returned, in characters.
const TEXT_CHARS: usize = 8000;
/// Most raw lines returned for one request.
const REQUEST_LINES: i64 = 2000;

#[derive(Clone)]
pub struct ApiState {
    pub store: Arc<Store>,
    pub alerts: Option<Arc<Alerts>>,
}

/// The `/api/v1` routes. Auth, Host checks and CORS are added by `http::router`.
pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/timeline", get(timeline))
        .route("/whats-new", get(whats_new))
        .route("/patterns", get(patterns))
        .route("/patterns/{id}", get(pattern))
        .route("/patterns/{id}/verify", get(verify))
        .route("/compare", get(compare))
        .route("/context", get(context))
        .route("/requests/{id}", get(request))
        .route("/markers", get(markers).post(add_marker))
        .route("/alerts", get(latest_alert))
        .route("/alerts/stream", get(alert_stream))
        .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "No such API route.") })
        .with_state(state)
}

/// An error as `{"error": "..."}` with a status code.
#[derive(Debug)]
pub struct ApiError(StatusCode, String);

impl ApiError {
    fn new(code: StatusCode, msg: impl Into<String>) -> Self {
        Self(code, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

/// Analysis errors are messages; tell "not found" and store failures apart.
impl From<String> for ApiError {
    fn from(m: String) -> Self {
        let code = if m.starts_with("log store error") {
            StatusCode::INTERNAL_SERVER_ERROR
        } else if m.starts_with("Unknown pattern_id")
            || m.starts_with("No events for request")
            || m.starts_with("No stored line")
            || m.starts_with("No marker named")
        {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        Self(code, m)
    }
}

impl From<QueryRejection> for ApiError {
    fn from(r: QueryRejection) -> Self {
        Self(StatusCode::BAD_REQUEST, r.body_text())
    }
}

impl From<JsonRejection> for ApiError {
    fn from(r: JsonRejection) -> Self {
        Self(r.status(), r.body_text())
    }
}

type ApiResult = Result<Json<Value>, ApiError>;

/// Run a store query off the async workers; SQLite calls block.
async fn blocking<F>(f: F) -> ApiResult
where
    F: FnOnce() -> Result<Value, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => Ok(Json(r?)),
        Err(e) => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("internal error: {e}"),
        )),
    }
}

/// ISO 8601 UTC with milliseconds.
pub fn iso(ts: f64) -> String {
    DateTime::from_timestamp_millis((ts * 1000.0).round() as i64)
        .map(|d| d.to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_default()
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn pattern_json(row: &PatternRow, rel: &Releases) -> Value {
    json!({
        "id": row.id,
        "level": row.level.as_deref().unwrap_or("INFO"),
        "template": row.template,
        "exception": row.exc_type,
        "location": row.location,
        "first_seen": iso(row.first_seen),
        "last_seen": iso(row.last_seen),
        "first_release": rel.at(row.first_seen),
        "total": row.total,
        "example_request": row.example_request,
    })
}

fn marker_json(m: &Marker) -> Value {
    json!({"name": m.name, "kind": m.kind, "at": iso(m.ts), "note": m.note})
}

fn line_json(l: &RawLine) -> Value {
    json!({"id": l.id, "at": iso(l.ts), "pattern_id": l.pattern_id, "text": truncate(&l.text, TEXT_CHARS)})
}

fn time_or(store: &Store, spec: Option<&str>, default: &str, now: f64) -> Result<f64, String> {
    let s = spec
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default);
    Ok(analysis::resolve_time(store, s, now)?.0)
}

fn markers_in(store: &Store, lo: f64, hi: f64) -> Result<Vec<Value>, String> {
    Ok(store
        .markers(lo, hi, 100)
        .map_err(analysis::db)?
        .iter()
        .map(marker_json)
        .collect())
}

/// A step that gives about `target` points over `span`, rounded to a length
/// people read easily. Never below the 10-second storage bucket.
fn nice_step(span: f64, target: f64) -> f64 {
    const STEPS: [f64; 14] = [
        10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0, 10800.0, 21600.0,
        43200.0, 86400.0,
    ];
    let want = span / target;
    STEPS
        .iter()
        .copied()
        .find(|s| *s >= want)
        .unwrap_or_else(|| (want / 86400.0).ceil() * 86400.0)
}

// ---- routes ----

async fn status(State(s): State<ApiState>) -> ApiResult {
    let alerts_on = s.alerts.is_some();
    blocking(move || {
        let store = &s.store;
        let now = analysis::now();
        let rel = Releases::load(store)?;
        let hour = store
            .level_series(now - 3600.0, now, 3600.0)
            .map_err(analysis::db)?;
        let events: i64 = hour.iter().map(|(_, c)| c.iter().sum::<i64>()).sum();
        let errors: i64 = hour
            .iter()
            .filter(|(l, _)| level_rank(Some(l)) >= 3)
            .map(|(_, c)| c.iter().sum::<i64>())
            .sum();
        let (raw_n, raw_oldest) = store.raw_span().map_err(analysis::db)?;
        Ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "now": iso(now),
            "service": Value::Null,
            "current_release": rel.latest().map(|(t, n)| json!({"name": n, "at": iso(t)})),
            "patterns": store.pattern_count().map_err(analysis::db)?,
            "last_hour": {"events": events, "errors": errors},
            "raw": {"events": raw_n, "oldest": raw_oldest.map(iso)},
            "alerts": alerts_on,
        }))
    })
    .await
}

#[derive(Deserialize)]
struct TimelineQ {
    since: Option<String>,
    until: Option<String>,
    /// Seconds per point, or a duration like "5m". Default: about 100 points.
    step: Option<String>,
    pattern_id: Option<String>,
}

async fn timeline(
    State(s): State<ApiState>,
    q: Result<Query<TimelineQ>, QueryRejection>,
) -> ApiResult {
    let Query(q) = q?;
    blocking(move || {
        let store = &s.store;
        let now = analysis::now();
        let hi = time_or(store, q.until.as_deref(), "now", now)?;
        let lo = time_or(store, q.since.as_deref(), "24h", now)?;
        if lo >= hi {
            return Err("'since' must be before 'until'.".into());
        }
        let step = match q.step.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => nice_step(hi - lo, 100.0),
            Some(st) => st
                .parse::<f64>()
                .ok()
                .or_else(|| analysis::parse_duration(st))
                .filter(|x| *x > 0.0)
                .ok_or_else(|| format!("step must be seconds or a duration like '5m', got '{st}'"))?
                .max(crate::store::BUCKET_SECS),
        };
        // Align to the step so points land on round times.
        let lo = (lo / step).floor() * step;
        let points = ((hi - lo) / step).ceil();
        if points > 5000.0 {
            return Err(format!(
                "That would be {points} points; use a larger step (at most 5000 points)."
            ));
        }
        let series: Vec<Value> = match q.pattern_id.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => {
                let row = store
                    .pattern(p)
                    .map_err(analysis::db)?
                    .ok_or_else(|| format!("Unknown pattern_id '{p}'."))?;
                let counts = store.histogram(&row.id, lo, hi, step).map_err(analysis::db)?;
                vec![json!({"pattern_id": row.id, "level": row.level.as_deref().unwrap_or("INFO"), "counts": counts})]
            }
            None => {
                let mut by_level = store.level_series(lo, hi, step).map_err(analysis::db)?;
                by_level.sort_by_key(|(l, _)| std::cmp::Reverse(level_rank(Some(l))));
                by_level
                    .into_iter()
                    .map(|(level, counts)| json!({"level": level, "counts": counts}))
                    .collect()
            }
        };
        Ok(json!({
            "from": iso(lo),
            "to": iso(hi),
            "step_seconds": step,
            "series": series,
            "markers": markers_in(store, lo, hi)?,
        }))
    })
    .await
}

#[derive(Deserialize)]
struct WhatsNewQ {
    window: Option<String>,
    baseline: Option<String>,
    min_level: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn whats_new(
    State(s): State<ApiState>,
    q: Result<Query<WhatsNewQ>, QueryRejection>,
) -> ApiResult {
    let Query(q) = q?;
    blocking(move || {
        let store = &s.store;
        let now = analysis::now();
        let w = analysis::duration_arg(q.window.as_deref().unwrap_or("1h"), "window")?;
        let b = analysis::duration_arg(q.baseline.as_deref().unwrap_or("24h"), "baseline")?;
        let data = analysis::whats_new_data(store, w, b, q.min_level.as_deref(), now)?;
        let rel = Releases::load(store)?;
        let (limit, offset) = (q.limit.unwrap_or(20).clamp(1, 200), q.offset.unwrap_or(0));
        let findings: Vec<Value> = data
            .findings
            .iter()
            .skip(offset)
            .take(limit)
            .map(|f| {
                let sample = store
                    .samples(&f.row.id, 1)
                    .ok()
                    .and_then(|v| v.into_iter().next())
                    .map(|(ts, t)| json!({"at": iso(ts), "text": truncate(&t, TEXT_CHARS)}));
                json!({
                    "kind": f.kind,
                    "note": f.note,
                    "count_window": f.count_window,
                    "count_baseline": f.count_baseline,
                    "per_hour_window": f.per_hour_window,
                    "per_hour_baseline": f.per_hour_baseline,
                    "pattern": pattern_json(&f.row, &rel),
                    "sample": sample,
                })
            })
            .collect();
        let t = data.tally;
        Ok(json!({
            "window": {"from": iso(data.window_from), "to": iso(now)},
            "baseline": {"from": iso(data.baseline_from), "to": iso(data.window_from)},
            "summary": data.summary(),
            "counts": {"new": t[0], "returning": t[1], "spike": t[2], "gone_quiet": t[3]},
            "traffic": {
                "events_window": data.events_in_window,
                "events_baseline": data.events_in_baseline,
                "ratio": round1(data.traffic),
            },
            "current_release": rel.latest().map(|(_, n)| n),
            "markers": markers_in(store, data.window_from, now)?,
            "total": data.findings.len(),
            "offset": offset,
            "findings": findings,
        }))
    })
    .await
}

#[derive(Deserialize)]
struct PatternsQ {
    since: Option<String>,
    min_level: Option<String>,
    contains: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    /// Points in each pattern's `trend` over the period, 0 to 96. Default 24.
    trend: Option<usize>,
}

async fn patterns(
    State(s): State<ApiState>,
    q: Result<Query<PatternsQ>, QueryRejection>,
) -> ApiResult {
    let Query(q) = q?;
    blocking(move || {
        let store = &s.store;
        let now = analysis::now();
        let lo = time_or(store, q.since.as_deref(), "24h", now)?;
        let min_rank = analysis::min_level_filter(q.min_level.as_deref())?;
        let needle = q
            .contains
            .as_deref()
            .map(str::to_lowercase)
            .filter(|n| !n.is_empty());
        let mut stats: Vec<_> = store
            .split_counts(lo, now + 2.0 * crate::store::BUCKET_SECS, now)
            .map_err(analysis::db)?
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
        let rel = Releases::load(store)?;
        let (limit, offset) = (q.limit.unwrap_or(50).clamp(1, 500), q.offset.unwrap_or(0));
        let points = q.trend.unwrap_or(24).min(96);
        let step = (now - lo) / points.max(1) as f64;
        let items = stats
            .iter()
            .skip(offset)
            .take(limit)
            .map(|s| {
                let mut v = pattern_json(&s.row, &rel);
                v["count"] = json!(s.before);
                if points > 0 {
                    v["trend"] = json!(
                        store
                            .histogram(&s.row.id, lo, now, step)
                            .map_err(analysis::db)?
                    );
                }
                Ok(v)
            })
            .collect::<Result<Vec<Value>, String>>()?;
        Ok(json!({
            "since": iso(lo),
            "trend_step_seconds": if points > 0 { Some(step) } else { None },
            "total": stats.len(),
            "offset": offset,
            "patterns": items,
        }))
    })
    .await
}

async fn pattern(State(s): State<ApiState>, Path(id): Path<String>) -> ApiResult {
    blocking(move || {
        let store = &s.store;
        let now = analysis::now();
        let row = store
            .pattern(&id)
            .map_err(analysis::db)?
            .ok_or_else(|| format!("Unknown pattern_id '{id}'."))?;
        let mut v = pattern_json(&row, &Releases::load(store)?);
        v["locations"] = json!(
            store
                .locations(&row.id, 20)
                .map_err(analysis::db)?
                .into_iter()
                .map(|(l, n)| json!({"location": l, "count": n}))
                .collect::<Vec<_>>()
        );
        v["count_last_hour"] = json!(
            store
                .count(&row.id, now - 3600.0, now)
                .map_err(analysis::db)?
        );
        v["count_last_24h"] = json!(
            store
                .count(&row.id, now - 86400.0, now)
                .map_err(analysis::db)?
        );
        v["samples"] = json!(
            store
                .samples(&row.id, 5)
                .map_err(analysis::db)?
                .into_iter()
                .map(|(ts, t)| json!({"at": iso(ts), "text": truncate(&t, TEXT_CHARS)}))
                .collect::<Vec<_>>()
        );
        // Asked for by its old ID after a merge: say so.
        if row.id != id {
            v["merged_from"] = json!(id);
        }
        Ok(v)
    })
    .await
}

#[derive(Deserialize)]
struct VerifyQ {
    since: Option<String>,
    before: Option<String>,
}

async fn verify(
    State(s): State<ApiState>,
    Path(id): Path<String>,
    q: Result<Query<VerifyQ>, QueryRejection>,
) -> ApiResult {
    let Query(q) = q?;
    blocking(move || {
        let store = &s.store;
        let now = analysis::now();
        let since = q
            .since
            .as_deref()
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .ok_or("Pass 'since': a marker name, ISO time or duration like '20m'.")?;
        let c = analysis::verify_fix_data(store, &id, since, q.before.as_deref(), now)?;
        let rel = Releases::load(store)?;
        let per_hour = |n: i64, secs: f64| {
            if secs > 0.0 {
                round1(n as f64 * 3600.0 / secs)
            } else {
                0.0
            }
        };
        Ok(json!({
            "pattern": pattern_json(&c.row, &rel),
            "verdict": c.verdict,
            "explanation": c.explanation,
            "split": {"at": iso(c.split), "marker": c.marker},
            "before": c.before_from.map(|lo| json!({
                "from": iso(lo), "to": iso(c.split),
                "count": c.before_count, "per_hour": per_hour(c.before_count, c.split - lo),
            })),
            "after": {
                "from": iso(c.split), "to": iso(now),
                "count": c.after_count, "per_hour": per_hour(c.after_count, now - c.split),
            },
            "expected_after": c.expected_after,
            "confidence": c.confidence,
            "wait_minutes": c.wait_minutes,
            "caveat": c.caveat,
        }))
    })
    .await
}

#[derive(Deserialize)]
struct CompareQ {
    at: Option<String>,
    before: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn compare(
    State(s): State<ApiState>,
    q: Result<Query<CompareQ>, QueryRejection>,
) -> ApiResult {
    let Query(q) = q?;
    blocking(move || {
        let store = &s.store;
        let now = analysis::now();
        let d = analysis::compare_deploy_data(store, q.at.as_deref(), q.before.as_deref(), now)?;
        let rel = Releases::load(store)?;
        let (limit, offset) = (q.limit.unwrap_or(50).clamp(1, 500), q.offset.unwrap_or(0));
        let changes: Vec<Value> = d
            .changes
            .iter()
            .skip(offset)
            .take(limit)
            .map(|c| {
                json!({
                    "kind": c.kind,
                    "before": {"count": c.count_before, "per_hour": c.per_hour_before},
                    "after": {"count": c.count_after, "per_hour": c.per_hour_after},
                    "pattern": pattern_json(&c.row, &rel),
                })
            })
            .collect();
        let t = d.tally;
        Ok(json!({
            "split": {"at": iso(d.split), "marker": d.marker},
            "before": {"from": iso(d.before_from), "to": iso(d.split), "events": d.events_before},
            "after": {"from": iso(d.split), "to": iso(now), "events": d.events_after},
            "summary": d.summary(),
            "counts": {"new_after": t[0], "increased": t[1], "stopped": t[2], "decreased": t[3]},
            "total": d.changes.len(),
            "offset": offset,
            "changes": changes,
        }))
    })
    .await
}

#[derive(Deserialize)]
struct ContextQ {
    pattern_id: Option<String>,
    at: Option<String>,
    before: Option<i64>,
    after: Option<i64>,
}

async fn context(
    State(s): State<ApiState>,
    q: Result<Query<ContextQ>, QueryRejection>,
) -> ApiResult {
    let Query(q) = q?;
    blocking(move || {
        let store = &s.store;
        let now = analysis::now();
        let pid = match q.pattern_id.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => Some(
                store
                    .pattern(p)
                    .map_err(analysis::db)?
                    .ok_or_else(|| format!("Unknown pattern_id '{p}'."))?
                    .id,
            ),
            None => None,
        };
        let at = match q.at.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            Some(a) => Some(analysis::resolve_time(store, a, now)?.0),
            None if pid.is_none() => return Err("Pass pattern_id, at, or both.".into()),
            None => None,
        };
        let anchor = store
            .raw_anchor(pid.as_deref(), at)
            .map_err(analysis::db)?
            .ok_or_else(|| "No stored line matches. Raw lines are kept for recent events only, and not at all with --no-raw.".to_string())?;
        let lines = store
            .raw_around(
                &anchor,
                q.before.unwrap_or(20).clamp(0, 500),
                q.after.unwrap_or(20).clamp(0, 500),
            )
            .map_err(analysis::db)?;
        Ok(json!({
            "anchor": line_json(&anchor),
            "lines": lines.iter().map(line_json).collect::<Vec<_>>(),
        }))
    })
    .await
}

async fn request(State(s): State<ApiState>, Path(id): Path<String>) -> ApiResult {
    blocking(move || {
        let store = &s.store;
        let id = crate::request::normalize_query(&id);
        if id.is_empty() {
            return Err("Pass a request ID.".into());
        }
        let lines = store
            .request_lines(&id, REQUEST_LINES + 1)
            .map_err(analysis::db)?;
        if lines.is_empty() {
            return Err(format!("No events for request '{id}' in the raw buffer."));
        }
        let truncated = lines.len() as i64 > REQUEST_LINES;
        let lines = &lines[..lines.len().min(REQUEST_LINES as usize)];
        let start = lines[0].ts;
        let end = lines[lines.len() - 1].ts;
        Ok(json!({
            "request_id": id,
            "service": Value::Null,
            "started": iso(start),
            "duration_ms": ((end - start) * 1000.0).round(),
            "events": lines.len(),
            "truncated": truncated,
            "lines": lines.iter().map(|l| {
                let mut v = line_json(l);
                v["offset_ms"] = json!(((l.ts - start) * 1000.0).round());
                v
            }).collect::<Vec<_>>(),
        }))
    })
    .await
}

#[derive(Deserialize)]
struct MarkersQ {
    limit: Option<i64>,
}

async fn markers(
    State(s): State<ApiState>,
    q: Result<Query<MarkersQ>, QueryRejection>,
) -> ApiResult {
    let Query(q) = q?;
    blocking(move || {
        let ms = s
            .store
            .markers(f64::MIN, f64::MAX, q.limit.unwrap_or(100).clamp(1, 1000))
            .map_err(analysis::db)?;
        Ok(json!({"markers": ms.iter().map(marker_json).collect::<Vec<_>>()}))
    })
    .await
}

#[derive(Deserialize)]
struct NewMarker {
    name: String,
    note: Option<String>,
    at: Option<String>,
    #[serde(default)]
    deploy: bool,
}

async fn add_marker(
    State(s): State<ApiState>,
    body: Result<Json<NewMarker>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let Json(m) = body?;
    let Json(v) = blocking(move || {
        let store = &s.store;
        let now = analysis::now();
        analysis::add_marker(
            store,
            &m.name,
            m.note.as_deref(),
            m.at.as_deref(),
            m.deploy,
            now,
        )?;
        let saved = store
            .marker(m.name.trim())
            .map_err(analysis::db)?
            .ok_or("The marker was not saved.")?;
        Ok(marker_json(&saved))
    })
    .await?;
    Ok((StatusCode::CREATED, Json(v)))
}

async fn latest_alert(State(s): State<ApiState>) -> ApiResult {
    let now = analysis::now();
    let alert = s
        .alerts
        .as_ref()
        .and_then(|a| a.latest())
        .map(|a| a.to_json(now));
    Ok(Json(json!({"enabled": s.alerts.is_some(), "alert": alert})))
}

async fn alert_stream(State(s): State<ApiState>) -> Result<Response, ApiError> {
    let Some(alerts) = s.alerts else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "Alerts are off (--no-alerts).",
        ));
    };
    let rx = alerts.subscribe();
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(a) => {
                    let ev = Event::default()
                        .event("alert")
                        .data(a.to_json(analysis::now()).to_string());
                    return Some((Ok::<_, std::convert::Infallible>(ev), rx));
                }
                // Missed some while the client was slow; carry on with the next.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

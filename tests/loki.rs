//! Reading a LogQL stream from Loki (issue #27).

use axum::extract::Query;
use starlist::collector::Pipeline;
use starlist::redact::Redactor;
use starlist::sources::loki::{LokiConfig, follow};
use starlist::store::Store;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

static CALLS: AtomicUsize = AtomicUsize::new(0);

/// A fake Loki that always answers with the same lines (so every poll overlaps
/// the last) and checks the query and auth it is sent.
async fn query_range(
    headers: axum::http::HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    CALLS.fetch_add(1, Ordering::Relaxed);
    assert_eq!(q["query"], r#"{app="shop"}"#);
    assert_eq!(q["direction"], "forward");
    assert_eq!(headers["authorization"], "Bearer loki-token");
    assert_eq!(headers["x-scope-orgid"], "tenant1");
    let start: u64 = q["start"].parse().unwrap();
    // Five seconds before the first poll, the same on every poll.
    static T: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let t = *T.get_or_init(|| (starlist::analysis::now() * 1e9) as u64 - 5_000_000_000);
    let values: Vec<(String, &str)> = [
        (t, "2026-01-01T00:00:00Z ERROR payment declined order=77"),
        (t + 1, "Traceback (most recent call last):"),
        (t + 2, "  File \"shop/pay.py\", line 9, in charge"),
        (t + 3, "ValueError: declined"),
        (t + 4, "2026-01-01T00:00:01Z INFO GET /health 200"),
    ]
    .into_iter()
    .filter(|(ts, _)| *ts >= start)
    .map(|(ts, l)| (ts.to_string(), l))
    .collect();
    axum::Json(
        serde_json::json!({"status": "success", "data": {"resultType": "streams",
        "result": [{"stream": {"app": "shop"}, "values": values}]}}),
    )
    .into_response()
}

#[tokio::test(flavor = "multi_thread")]
async fn follows_a_stream_without_reading_lines_twice() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app =
        axum::Router::new().route("/loki/api/v1/query_range", axum::routing::get(query_range));
    tokio::spawn(axum::serve(listener, app).into_future());

    let p = std::env::temp_dir().join(format!("starlist-loki-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let s = Arc::new(Store::open(p).unwrap());
    let pipeline = Pipeline::new(s.clone(), Redactor::default(), None);
    static STOP: AtomicBool = AtomicBool::new(false);
    let cfg = LokiConfig {
        url: format!("http://{addr}/"),
        query: r#"{app="shop"}"#.into(),
        token: Some("loki-token".into()),
        user: None,
        org: Some("tenant1".into()),
    };
    let h = std::thread::spawn(move || follow(&pipeline, cfg, false, &STOP));
    // Three polls or more, two seconds apart.
    tokio::time::sleep(Duration::from_millis(4500)).await;
    STOP.store(true, Ordering::Relaxed);
    tokio::task::spawn_blocking(move || h.join().unwrap())
        .await
        .unwrap();
    assert!(CALLS.load(Ordering::Relaxed) >= 2);

    let patterns = s.patterns_seen_since(0.0).unwrap();
    assert_eq!(patterns.len(), 2, "{patterns:?}");
    for p in &patterns {
        assert_eq!(p.total, 1, "read more than once: {p:?}");
    }
    let err = patterns
        .iter()
        .find(|p| p.level.as_deref() == Some("ERROR"))
        .unwrap();
    assert_eq!(err.exc_type.as_deref(), Some("ValueError"));
}

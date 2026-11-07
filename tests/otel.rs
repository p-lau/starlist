//! OTLP logs over HTTP (issue #27): protobuf, JSON and gzip, with trace IDs
//! kept for get_request.

use starlist::analysis::{Detail, get_request};
use starlist::collector::Pipeline;
use starlist::redact::Redactor;
use starlist::sources::otel::{self, pb::encode};
use starlist::store::Store;
use std::sync::Arc;

const TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

fn store(name: &str) -> Arc<Store> {
    let p = std::env::temp_dir().join(format!("starlist-otel-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let s = Arc::new(Store::open(p).unwrap());
    s.set_raw_cap(Some(1 << 20)).unwrap();
    s
}

fn now_ns() -> u64 {
    (starlist::analysis::now() * 1e9) as u64
}

/// An ExportLogsServiceRequest with one ERROR record carrying a trace ID and an exception.
fn protobuf_request() -> Vec<u8> {
    let trace: Vec<u8> = (0..16)
        .map(|i| u8::from_str_radix(&TRACE[i * 2..i * 2 + 2], 16).unwrap())
        .collect();
    let mut body = vec![];
    encode::bytes(1, b"checkout failed for order 1234", &mut body);
    let mut rec = vec![];
    encode::fixed64(1, now_ns(), &mut rec);
    encode::uint(2, 17, &mut rec);
    encode::bytes(5, &body, &mut rec);
    encode::bytes(
        6,
        &encode::string_kv("exception.type", "KeyError"),
        &mut rec,
    );
    encode::bytes(
        6,
        &encode::string_kv("exception.message", "'currency'"),
        &mut rec,
    );
    encode::bytes(9, &trace, &mut rec);
    encode::bytes(10, &[1, 2, 3, 4, 5, 6, 7, 8], &mut rec);
    let mut scope = vec![];
    encode::bytes(2, &rec, &mut scope);
    let mut resource = vec![];
    encode::bytes(1, &encode::string_kv("service.name", "shop"), &mut resource);
    let mut rl = vec![];
    encode::bytes(1, &resource, &mut rl);
    encode::bytes(2, &scope, &mut rl);
    let mut req = vec![];
    encode::bytes(1, &rl, &mut req);
    req
}

fn json_request() -> String {
    serde_json::json!({"resourceLogs": [{
        "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "shop"}}]},
        "scopeLogs": [{"logRecords": [
            {"timeUnixNano": now_ns().to_string(), "severityNumber": 9,
             "body": {"stringValue": "GET /cart 200"}, "traceId": TRACE,
             "spanId": "00f067aa0ba902b7",
             "attributes": [{"key": "duration_ms", "value": {"intValue": "12"}}]},
            {"timeUnixNano": now_ns().to_string(), "severityText": "WARN",
             "body": {"stringValue": "slow pricing call"}, "traceId": "00000000000000000000000000000000"}
        ]}]
    }]})
    .to_string()
}

#[test]
fn protobuf_records_become_json_lines() {
    let lines = otel::decode(&protobuf_request(), true).unwrap();
    assert_eq!(lines.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(v["level"], "ERROR");
    assert_eq!(v["message"], "checkout failed for order 1234");
    assert_eq!(v["trace_id"], TRACE);
    assert_eq!(v["span_id"], "0102030405060708");
    assert_eq!(v["service.name"], "shop");
    assert_eq!(v["stack"], "KeyError: 'currency'");
}

#[test]
fn json_records_and_zero_trace_ids() {
    let lines = otel::decode(json_request().as_bytes(), false).unwrap();
    assert_eq!(lines.len(), 2);
    let a: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(a["level"], "INFO");
    assert_eq!(a["duration_ms"], 12);
    let b: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(b["level"], "WARN");
    assert!(b.get("trace_id").is_none());
}

#[test]
fn bad_protobuf_is_an_error() {
    assert!(otel::decode(&[0x0a, 0x05, 0x01], true).is_err());
}

#[tokio::test]
async fn receiver_records_events_with_their_trace() {
    let s = store("receiver");
    let p = Arc::new(Pipeline::new(s.clone(), Redactor::default(), None));
    let token = "otlp-test-token-1234567890".to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, otel::router(p, Some(token.clone()))).into_future());
    let url = format!("http://{addr}/v1/logs");
    let c = reqwest::Client::new();

    // No token: refused.
    let r = c
        .post(&url)
        .header("content-type", "application/json")
        .body(json_request())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // A form post (what another site could make a browser send): refused.
    let r = c
        .post(&url)
        .bearer_auth(&token)
        .header("content-type", "text/plain")
        .body(json_request())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 415);

    let r = c
        .post(&url)
        .bearer_auth(&token)
        .header("content-type", "application/json")
        .body(json_request())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // Protobuf, gzipped, as the OpenTelemetry Collector sends it.
    use std::io::Write;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&protobuf_request()).unwrap();
    let r = c
        .post(&url)
        .bearer_auth(&token)
        .header("content-type", "application/x-protobuf")
        .header("content-encoding", "gzip")
        .body(gz.finish().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers()["content-type"].to_str().unwrap(),
        "application/x-protobuf"
    );

    let patterns = s.patterns_seen_since(0.0).unwrap();
    assert_eq!(patterns.len(), 3, "{patterns:?}");
    let err = patterns
        .iter()
        .find(|p| p.level.as_deref() == Some("ERROR"))
        .unwrap();
    assert_eq!(err.exc_type.as_deref(), Some("KeyError"));

    // The INFO and ERROR records share a trace: get_request follows it.
    let v = get_request(&s, TRACE, 8000, Detail::Full, starlist::analysis::now()).unwrap();
    let text = v.to_string();
    assert!(text.contains("GET /cart 200"), "{text}");
    assert!(text.contains("checkout failed"), "{text}");
}

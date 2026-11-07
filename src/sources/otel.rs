//! OpenTelemetry: accept OTLP logs over HTTP (`POST /v1/logs`, protobuf or
//! JSON, optionally gzipped), the endpoint OTLP exporters and the OpenTelemetry
//! Collector send to. Each log record becomes one event, with its trace and
//! span IDs kept so `get_request` can follow a trace.
//!
//! Protobuf is decoded by hand (the few messages logs need), so no protobuf
//! library is linked in.

use crate::collector::Pipeline;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};
use std::sync::Arc;

/// The most a request body may hold, after decompression.
const MAX_BODY: usize = 16 << 20;

/// One log record, from either encoding.
#[derive(Debug, Default, PartialEq)]
pub struct Record {
    pub time_ns: u64,
    pub severity_number: i64,
    pub severity_text: String,
    pub body: Value,
    pub attributes: Map<String, Value>,
    pub trace_id: String,
    pub span_id: String,
}

/// The OTLP receiver: `POST /v1/logs`, guarded by a bearer token when one is set.
pub fn router(pipeline: Arc<Pipeline>, token: Option<String>) -> axum::Router {
    let r = axum::Router::new()
        .route("/v1/logs", axum::routing::post(receive))
        .with_state(pipeline)
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY));
    match token {
        Some(t) => r.layer(axum::middleware::from_fn_with_state(
            Arc::new(t),
            crate::http::require_bearer,
        )),
        None => r,
    }
}

async fn receive(State(p): State<Arc<Pipeline>>, headers: HeaderMap, body: Bytes) -> Response {
    let ctype = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let protobuf = match ctype.as_str() {
        "application/x-protobuf" | "application/protobuf" => true,
        "application/json" => false,
        // Only these two: a browser can't send either across origins without
        // a CORS preflight, which this endpoint never grants.
        _ => {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "send application/x-protobuf or application/json\n",
            )
                .into_response();
        }
    };
    let gzip = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("gzip"));
    let body = if gzip {
        match gunzip(&body) {
            Ok(b) => Bytes::from(b),
            Err(e) => return (StatusCode::BAD_REQUEST, format!("{e}\n")).into_response(),
        }
    } else {
        body
    };
    let lines = match decode(&body, protobuf) {
        Ok(l) => l,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{e}\n")).into_response(),
    };
    if !lines.is_empty() {
        let _ = tokio::task::spawn_blocking(move || p.ingest(&lines)).await;
    }
    // An empty ExportLogsServiceResponse: no fields means full success.
    if protobuf {
        (
            [(header::CONTENT_TYPE, "application/x-protobuf")],
            Vec::new(),
        )
            .into_response()
    } else {
        ([(header::CONTENT_TYPE, "application/json")], "{}").into_response()
    }
}

fn gunzip(b: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(b)
        .take(MAX_BODY as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("bad gzip body: {e}"))?;
    if out.len() > MAX_BODY {
        return Err("body too large".into());
    }
    Ok(out)
}

/// An export request as log lines for the pipeline.
pub fn decode(body: &[u8], protobuf: bool) -> Result<Vec<String>, String> {
    let records = if protobuf {
        pb::export_request(body)?
    } else {
        from_json(body)?
    };
    Ok(records.iter().map(to_line).collect())
}

/// A record as one JSON log line, in the keys the parser already reads:
/// `time`, `level`, `message`, `trace_id`, `span_id`, `stack`, plus the
/// record's and its resource's attributes.
pub fn to_line(r: &Record) -> String {
    let mut o = Map::new();
    let mut stack = None;
    let (mut exc_type, mut exc_msg) = (None, None);
    for (k, v) in &r.attributes {
        match k.as_str() {
            "exception.stacktrace" => stack = v.as_str().map(str::to_string),
            "exception.type" => exc_type = v.as_str().map(str::to_string),
            "exception.message" => exc_msg = v.as_str().map(str::to_string),
            _ => {
                o.insert(k.clone(), v.clone());
            }
        }
    }
    if r.time_ns > 0 {
        o.insert(
            "time".into(),
            Value::String(super::rfc3339(r.time_ns as i64)),
        );
    }
    let level = if r.severity_text.is_empty() {
        severity(r.severity_number)
    } else {
        Some(r.severity_text.as_str())
    };
    if let Some(l) = level {
        o.insert("level".into(), Value::String(l.to_string()));
    }
    let message = match &r.body {
        Value::String(s) => s.clone(),
        Value::Null => exc_msg.clone().unwrap_or_default(),
        Value::Object(m) => match m.get("message").or_else(|| m.get("msg")) {
            Some(Value::String(s)) => s.clone(),
            _ => r.body.to_string(),
        },
        other => other.to_string(),
    };
    o.insert("message".into(), Value::String(message));
    if !r.trace_id.is_empty() {
        o.insert("trace_id".into(), Value::String(r.trace_id.clone()));
    }
    if !r.span_id.is_empty() {
        o.insert("span_id".into(), Value::String(r.span_id.clone()));
    }
    let stack = stack.or_else(|| match (&exc_type, &exc_msg) {
        (Some(t), Some(m)) => Some(format!("{t}: {m}")),
        (Some(t), None) => Some(t.clone()),
        _ => None,
    });
    if let Some(s) = stack {
        o.insert("stack".into(), Value::String(s));
    }
    Value::Object(o).to_string()
}

/// The level for an OTLP severity number (1-24).
fn severity(n: i64) -> Option<&'static str> {
    Some(match n {
        1..=8 => "DEBUG",
        9..=12 => "INFO",
        13..=16 => "WARN",
        17..=20 => "ERROR",
        21..=24 => "FATAL",
        _ => return None,
    })
}

/// Trace and span IDs that are all zeros mean "none".
fn id_or_empty(hex: String) -> String {
    if hex.bytes().all(|b| b == b'0') {
        String::new()
    } else {
        hex.to_ascii_lowercase()
    }
}

fn from_json(body: &[u8]) -> Result<Vec<Record>, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let mut out = vec![];
    for rl in arr(&v, "resourceLogs") {
        let res = kvs(rl
            .get("resource")
            .map(|r| arr(r, "attributes"))
            .unwrap_or_default());
        for sl in arr(rl, "scopeLogs") {
            for lr in arr(sl, "logRecords") {
                let mut attributes = res.clone();
                attributes.extend(kvs(arr(lr, "attributes")));
                let num = |k: &str| match lr.get(k) {
                    Some(Value::String(s)) => s.parse::<u64>().unwrap_or(0),
                    Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
                    _ => 0,
                };
                let time_ns = match num("timeUnixNano") {
                    0 => num("observedTimeUnixNano"),
                    t => t,
                };
                let severity_number = match lr.get("severityNumber") {
                    Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
                    Some(Value::String(s)) => severity_from_name(s),
                    _ => 0,
                };
                let s = |k: &str| lr.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                out.push(Record {
                    time_ns,
                    severity_number,
                    severity_text: s("severityText"),
                    body: lr.get("body").map(any_json).unwrap_or(Value::Null),
                    attributes,
                    trace_id: id_or_empty(s("traceId")),
                    span_id: id_or_empty(s("spanId")),
                });
            }
        }
    }
    Ok(out)
}

/// `SEVERITY_NUMBER_ERROR2` and friends, as some JSON encoders write them.
fn severity_from_name(s: &str) -> i64 {
    let base = |name: &str| -> i64 {
        match name {
            "TRACE" => 1,
            "DEBUG" => 5,
            "INFO" => 9,
            "WARN" => 13,
            "ERROR" => 17,
            "FATAL" => 21,
            _ => 0,
        }
    };
    let rest = s.trim_start_matches("SEVERITY_NUMBER_");
    let (name, n) = match rest.char_indices().find(|(_, c)| c.is_ascii_digit()) {
        Some((i, _)) => (&rest[..i], rest[i..].parse::<i64>().unwrap_or(1)),
        None => (rest, 1),
    };
    match base(name) {
        0 => 0,
        b => b + n - 1,
    }
}

fn arr<'a>(v: &'a Value, k: &str) -> Vec<&'a Value> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn kvs(list: Vec<&Value>) -> Map<String, Value> {
    list.into_iter()
        .filter_map(|kv| {
            let k = kv.get("key")?.as_str()?;
            Some((
                k.to_string(),
                kv.get("value").map(any_json).unwrap_or(Value::Null),
            ))
        })
        .collect()
}

/// An OTLP JSON `AnyValue` as plain JSON.
fn any_json(v: &Value) -> Value {
    if let Some(s) = v.get("stringValue") {
        return s.clone();
    }
    if let Some(b) = v.get("boolValue") {
        return b.clone();
    }
    if let Some(i) = v.get("intValue") {
        // int64 is a string in OTLP JSON.
        return match i {
            Value::String(s) => s.parse::<i64>().map(Value::from).unwrap_or(i.clone()),
            other => other.clone(),
        };
    }
    if let Some(d) = v.get("doubleValue") {
        return d.clone();
    }
    if let Some(a) = v.get("arrayValue") {
        return Value::Array(arr(a, "values").into_iter().map(any_json).collect());
    }
    if let Some(kv) = v.get("kvlistValue") {
        return Value::Object(kvs(arr(kv, "values")));
    }
    if let Some(b) = v.get("bytesValue") {
        return b.clone();
    }
    Value::Null
}

/// Just enough protobuf for `ExportLogsServiceRequest`.
pub mod pb {
    use super::{Record, id_or_empty};
    use serde_json::{Map, Value};

    struct Reader<'a> {
        b: &'a [u8],
    }

    enum Field<'a> {
        Varint(u64),
        Fixed64(u64),
        Fixed32,
        Bytes(&'a [u8]),
    }

    impl<'a> Reader<'a> {
        fn varint(&mut self) -> Result<u64, String> {
            let mut v = 0u64;
            for i in 0..10 {
                let (&byte, rest) = self.b.split_first().ok_or("truncated varint")?;
                self.b = rest;
                v |= ((byte & 0x7f) as u64) << (7 * i);
                if byte & 0x80 == 0 {
                    return Ok(v);
                }
            }
            Err("varint too long".into())
        }

        fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
            if self.b.len() < n {
                return Err("truncated message".into());
            }
            let (x, rest) = self.b.split_at(n);
            self.b = rest;
            Ok(x)
        }

        /// The next field number and value, or None at the end.
        fn next(&mut self) -> Result<Option<(u64, Field<'a>)>, String> {
            if self.b.is_empty() {
                return Ok(None);
            }
            let key = self.varint()?;
            let f = match key & 7 {
                0 => Field::Varint(self.varint()?),
                1 => Field::Fixed64(u64::from_le_bytes(self.take(8)?.try_into().unwrap())),
                2 => {
                    let n = self.varint()? as usize;
                    Field::Bytes(self.take(n)?)
                }
                5 => {
                    self.take(4)?;
                    Field::Fixed32
                }
                w => return Err(format!("unsupported wire type {w}")),
            };
            Ok(Some((key >> 3, f)))
        }
    }

    fn fields<'a>(
        b: &'a [u8],
        mut each: impl FnMut(u64, Field<'a>) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut r = Reader { b };
        while let Some((n, f)) = r.next()? {
            each(n, f)?;
        }
        Ok(())
    }

    fn text(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    pub fn export_request(b: &[u8]) -> Result<Vec<Record>, String> {
        let mut out = vec![];
        fields(b, |n, f| {
            if let (1, Field::Bytes(rl)) = (n, f) {
                resource_logs(rl, &mut out)?;
            }
            Ok(())
        })?;
        Ok(out)
    }

    fn resource_logs(b: &[u8], out: &mut Vec<Record>) -> Result<(), String> {
        let mut res = Map::new();
        let mut scopes = vec![];
        fields(b, |n, f| {
            match (n, f) {
                (1, Field::Bytes(r)) => fields(r, |n, f| {
                    if let (1, Field::Bytes(kv)) = (n, f) {
                        let (k, v) = key_value(kv)?;
                        res.insert(k, v);
                    }
                    Ok(())
                })?,
                (2, Field::Bytes(s)) => scopes.push(s),
                _ => {}
            }
            Ok(())
        })?;
        for s in scopes {
            fields(s, |n, f| {
                if let (2, Field::Bytes(lr)) = (n, f) {
                    out.push(log_record(lr, &res)?);
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn log_record(b: &[u8], res: &Map<String, Value>) -> Result<Record, String> {
        let mut r = Record {
            attributes: res.clone(),
            body: Value::Null,
            ..Default::default()
        };
        let mut observed = 0;
        fields(b, |n, f| {
            match (n, f) {
                (1, Field::Fixed64(t)) => r.time_ns = t,
                (11, Field::Fixed64(t)) => observed = t,
                (2, Field::Varint(s)) => r.severity_number = s as i64,
                (3, Field::Bytes(s)) => r.severity_text = text(s),
                (5, Field::Bytes(v)) => r.body = any_value(v)?,
                (6, Field::Bytes(kv)) => {
                    let (k, v) = key_value(kv)?;
                    r.attributes.insert(k, v);
                }
                (9, Field::Bytes(id)) => r.trace_id = id_or_empty(hex(id)),
                (10, Field::Bytes(id)) => r.span_id = id_or_empty(hex(id)),
                _ => {}
            }
            Ok(())
        })?;
        if r.time_ns == 0 {
            r.time_ns = observed;
        }
        Ok(r)
    }

    fn key_value(b: &[u8]) -> Result<(String, Value), String> {
        let (mut k, mut v) = (String::new(), Value::Null);
        fields(b, |n, f| {
            match (n, f) {
                (1, Field::Bytes(s)) => k = text(s),
                (2, Field::Bytes(a)) => v = any_value(a)?,
                _ => {}
            }
            Ok(())
        })?;
        Ok((k, v))
    }

    fn any_value(b: &[u8]) -> Result<Value, String> {
        let mut v = Value::Null;
        fields(b, |n, f| {
            v = match (n, f) {
                (1, Field::Bytes(s)) => Value::String(text(s)),
                (2, Field::Varint(x)) => Value::Bool(x != 0),
                (3, Field::Varint(x)) => Value::from(x as i64),
                (4, Field::Fixed64(x)) => serde_json::Number::from_f64(f64::from_bits(x))
                    .map(Value::Number)
                    .unwrap_or(Value::Null),
                (5, Field::Bytes(a)) => {
                    let mut items = vec![];
                    fields(a, |n, f| {
                        if let (1, Field::Bytes(x)) = (n, f) {
                            items.push(any_value(x)?);
                        }
                        Ok(())
                    })?;
                    Value::Array(items)
                }
                (6, Field::Bytes(kvl)) => {
                    let mut m = Map::new();
                    fields(kvl, |n, f| {
                        if let (1, Field::Bytes(kv)) = (n, f) {
                            let (k, v) = key_value(kv)?;
                            m.insert(k, v);
                        }
                        Ok(())
                    })?;
                    Value::Object(m)
                }
                (7, Field::Bytes(x)) => Value::String(hex(x)),
                _ => return Ok(()),
            };
            Ok(())
        })?;
        Ok(v)
    }

    /// Encoding helpers for tests and examples.
    pub mod encode {
        pub fn varint(mut v: u64, out: &mut Vec<u8>) {
            loop {
                let b = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    out.push(b);
                    return;
                }
                out.push(b | 0x80);
            }
        }
        pub fn bytes(field: u64, b: &[u8], out: &mut Vec<u8>) {
            varint(field << 3 | 2, out);
            varint(b.len() as u64, out);
            out.extend_from_slice(b);
        }
        pub fn fixed64(field: u64, v: u64, out: &mut Vec<u8>) {
            varint(field << 3 | 1, out);
            out.extend_from_slice(&v.to_le_bytes());
        }
        pub fn uint(field: u64, v: u64, out: &mut Vec<u8>) {
            varint(field << 3, out);
            varint(v, out);
        }
        /// `KeyValue { key, value: AnyValue { string_value } }`.
        pub fn string_kv(key: &str, value: &str) -> Vec<u8> {
            let mut any = vec![];
            bytes(1, value.as_bytes(), &mut any);
            let mut kv = vec![];
            bytes(1, key.as_bytes(), &mut kv);
            bytes(2, &any, &mut kv);
            kv
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn severity_names() {
        assert_eq!(severity_from_name("SEVERITY_NUMBER_ERROR"), 17);
        assert_eq!(severity_from_name("SEVERITY_NUMBER_WARN3"), 15);
        assert_eq!(severity_from_name("nonsense"), 0);
    }

    #[test]
    fn exception_attributes_become_the_stack() {
        let mut attributes = Map::new();
        attributes.insert("exception.type".into(), json!("KeyError"));
        attributes.insert("exception.message".into(), json!("'currency'"));
        let line = to_line(&Record {
            severity_number: 17,
            body: json!("checkout failed"),
            attributes,
            ..Default::default()
        });
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["level"], "ERROR");
        assert_eq!(v["stack"], "KeyError: 'currency'");
        assert!(v.get("exception.type").is_none());
    }
}

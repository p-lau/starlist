//! Loki: read a LogQL stream by polling `query_range`. Each poll looks back a
//! little over the last one, so lines that reach Loki late are still read, and
//! lines already read are skipped.

use crate::collector::{Feeder, Pipeline};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Lines asked for per query; a full page is followed by another at once.
const LIMIT: usize = 2000;
/// How far each poll looks back past the newest line read, for late lines.
const OVERLAP_NS: u64 = 15_000_000_000;
const EVERY: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct LokiConfig {
    /// Base URL, like `http://loki:3100`.
    pub url: String,
    /// A LogQL log query, like `{app="api"}`.
    pub query: String,
    /// Sent as `Authorization: Bearer`, or as the password with `user`.
    pub token: Option<String>,
    /// For basic auth, as Grafana Cloud uses.
    pub user: Option<String>,
    /// `X-Scope-OrgID` for a multi-tenant Loki.
    pub org: Option<String>,
}

/// Lines from one `query_range` answer: (time in ns, stream labels, line), oldest first.
pub fn parse_answer(body: &str) -> Result<Vec<(u64, String, String)>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("unexpected answer: {e}"))?;
    if v.get("status").and_then(Value::as_str) != Some("success") {
        return Err(format!(
            "query failed: {}",
            v.get("error")
                .and_then(Value::as_str)
                .unwrap_or(body.trim())
        ));
    }
    let data = &v["data"];
    if data.get("resultType").and_then(Value::as_str) != Some("streams") {
        return Err(
            "the query returns metrics, not log lines; use a log query like {app=\"api\"}".into(),
        );
    }
    let mut out = vec![];
    for s in data["result"].as_array().into_iter().flatten() {
        let labels = s.get("stream").map(Value::to_string).unwrap_or_default();
        for e in s["values"].as_array().into_iter().flatten() {
            let (Some(ts), Some(line)) = (
                e.get(0)
                    .and_then(Value::as_str)
                    .and_then(|t| t.parse::<u64>().ok()),
                e.get(1).and_then(Value::as_str),
            ) else {
                continue;
            };
            out.push((ts, labels.clone(), line.to_string()));
        }
    }
    out.sort_by_key(|e| e.0);
    Ok(out)
}

fn basic(user: &str, pass: &str) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = format!("{user}:{pass}").into_bytes();
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    format!("Basic {s}")
}

fn query(
    agent: &ureq::Agent,
    cfg: &LokiConfig,
    start_ns: u64,
    end_ns: u64,
) -> Result<Vec<(u64, String, String)>, String> {
    let url = format!("{}/loki/api/v1/query_range", cfg.url.trim_end_matches('/'));
    let mut req = agent
        .get(&url)
        .query("query", &cfg.query)
        .query("start", start_ns.to_string())
        .query("end", end_ns.to_string())
        .query("direction", "forward")
        .query("limit", LIMIT.to_string())
        // Keep a stream's own labels apart from per-line ones (Loki 3's
        // `detected_level`, labels a `| json` stage parses), so the lines of one
        // stream, like a stack trace's, are read as one stream. Older Lokis
        // ignore it.
        .header("X-Loki-Response-Encoding-Flags", "categorize-labels");
    match (&cfg.user, &cfg.token) {
        (Some(u), Some(t)) => req = req.header("Authorization", &basic(u, t)),
        (None, Some(t)) => req = req.header("Authorization", &format!("Bearer {t}")),
        _ => {}
    }
    if let Some(o) = &cfg.org {
        req = req.header("X-Scope-OrgID", o);
    }
    let mut res = req.call().map_err(|e| format!("{url}: {e}"))?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(format!("{url} answered {status}: {}", text.trim()));
    }
    parse_answer(&text)
}

fn now_ns() -> u64 {
    (crate::analysis::now() * 1e9) as u64
}

/// Follow the query until `stop` is set. With `from_start`, begins an hour back.
pub fn follow(pipeline: &Pipeline, cfg: LokiConfig, from_start: bool, stop: &AtomicBool) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into();
    eprintln!("starlist: following Loki {} {}", cfg.url, cfg.query);
    let mut newest = now_ns() - if from_start { 3600 * 1_000_000_000 } else { 0 };
    // Lines read in the look-back window, by time and a hash of stream + line.
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut feeders: HashMap<String, Feeder> = HashMap::new();
    let mut last_err = String::new();
    while !stop.load(Ordering::Relaxed) {
        let start = newest.saturating_sub(OVERLAP_NS);
        let mut full = false;
        match query(&agent, &cfg, start, now_ns()) {
            Ok(lines) => {
                last_err.clear();
                full = lines.len() >= LIMIT;
                for (ts, labels, line) in &lines {
                    if !seen.insert((*ts, hash(labels, line))) {
                        continue;
                    }
                    newest = newest.max(*ts);
                    feeders
                        .entry(labels.clone())
                        .or_insert_with(|| Feeder::new(pipeline))
                        .line(line);
                }
                if full && let Some(last) = lines.last() {
                    // A full page: move on past it even if the window holds more.
                    newest = newest.max(last.0 + OVERLAP_NS);
                }
                let floor = newest.saturating_sub(OVERLAP_NS);
                seen.retain(|(t, _)| *t >= floor);
            }
            Err(e) => {
                if e != last_err {
                    eprintln!("starlist: Loki: {e}");
                    last_err = e;
                }
            }
        }
        for f in feeders.values_mut() {
            f.idle(false);
        }
        if !full {
            let mut left = EVERY;
            while !left.is_zero() && !stop.load(Ordering::Relaxed) {
                let step = left.min(crate::collector::POLL);
                std::thread::sleep(step);
                left -= step;
                for f in feeders.values_mut() {
                    f.idle(false);
                }
            }
        }
    }
    for f in feeders.values_mut() {
        f.idle(true);
    }
}

fn hash(labels: &str, line: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    labels.hash(&mut h);
    line.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_auth() {
        assert_eq!(
            basic("Aladdin", "open sesame"),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
        assert_eq!(basic("a", "b"), "Basic YTpi");
    }

    #[test]
    fn answers() {
        let body = r#"{"status":"success","data":{"resultType":"streams","result":[
            {"stream":{"app":"api"},"values":[["2000","second"],["1000","first"]]},
            {"stream":{"app":"web"},"values":[["1500","middle"]]}]}}"#;
        let lines = parse_answer(body).unwrap();
        let text: Vec<&str> = lines.iter().map(|l| l.2.as_str()).collect();
        assert_eq!(text, ["first", "middle", "second"]);
        // With categorize-labels, per-line labels come third and the stream
        // keeps only its own.
        let body = r#"{"status":"success","data":{"resultType":"streams","encodingFlags":["categorize-labels"],"result":[
            {"stream":{"app":"api"},"values":[["1000","ERROR failed",{"structuredMetadata":{"detected_level":"error"}}],
                ["1001","java.lang.IllegalStateException: no",{"structuredMetadata":{"detected_level":"unknown"}}]]}]}}"#;
        let lines = parse_answer(body).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].1, lines[1].1);
        let metric = r#"{"status":"success","data":{"resultType":"matrix","result":[]}}"#;
        assert!(parse_answer(metric).is_err());
    }
}

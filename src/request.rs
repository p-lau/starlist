//! Request and trace IDs, found in a log event before it is masked, so the
//! events of one request can be followed in order.
//!
//! Recognized: a W3C `traceparent` value anywhere in the event, and
//! `trace_id`, `request_id`, `req_id`, `x-request-id`, `correlation_id` (in
//! any common spelling) as JSON fields or `key=value` / `key: value` text,
//! plus any extra keys the user configures.

use regex::Regex;
use std::sync::LazyLock;

static TRACEPARENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[0-9a-f]{2}-([0-9a-f]{32})-[0-9a-f]{16}-[0-9a-f]{2}\b").unwrap()
});

const DEFAULT_KEYS: &[&str] = &[
    "trace_id",
    "request_id",
    "req_id",
    "x_request_id",
    "correlation_id",
];

static DEFAULT: LazyLock<RequestKeys> = LazyLock::new(|| RequestKeys::new(&[]));

/// Which keys hold a request ID.
pub struct RequestKeys {
    /// Normalized key names (lowercase, without `-` and `_`).
    names: Vec<String>,
    text: Regex,
}

/// `Request-ID`, `request_id` and `requestId` all become `requestid`.
fn norm(key: &str) -> String {
    key.chars()
        .filter(|c| *c != '_' && *c != '-' && *c != '.')
        .flat_map(char::to_lowercase)
        .collect()
}

/// A key pattern that matches any spelling: `request_id`, `request-id`, `requestId`.
fn key_regex(key: &str) -> String {
    let parts: Vec<String> = key
        .split(['_', '-', '.'])
        .filter(|p| !p.is_empty())
        .map(regex::escape)
        .collect();
    parts.join("[_.-]?")
}

impl RequestKeys {
    /// The default keys plus `extra` (e.g. "order_ref").
    pub fn new(extra: &[String]) -> Self {
        let keys: Vec<&str> = DEFAULT_KEYS
            .iter()
            .copied()
            .chain(extra.iter().map(String::as_str))
            .collect();
        let alt: Vec<String> = keys.iter().map(|k| key_regex(k)).collect();
        let text = Regex::new(&format!(
            r#"(?i)(?:^|[^A-Za-z0-9_])(?:{})["']?\s*[=:]\s*["']?([A-Za-z0-9][A-Za-z0-9._:/-]{{3,127}})"#,
            alt.join("|")
        ))
        .expect("request key regex");
        Self {
            names: keys.iter().map(|k| norm(k)).collect(),
            text,
        }
    }

    /// The request ID in an event, if any.
    pub fn find(&self, text: &str) -> Option<String> {
        if let Some(c) = TRACEPARENT.captures(text) {
            return Some(c[1].to_string());
        }
        let first = text.lines().next().unwrap_or("").trim_start();
        if first.starts_with('{')
            && let Ok(serde_json::Value::Object(m)) = serde_json::from_str(first)
        {
            for (k, v) in &m {
                if self.names.contains(&norm(k)) {
                    let id = match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        _ => continue,
                    };
                    if let Some(id) = clean(&id) {
                        return Some(id);
                    }
                }
            }
        }
        self.text.captures_iter(text).find_map(|c| clean(&c[1]))
    }
}

fn clean(id: &str) -> Option<String> {
    let id = id.trim().trim_end_matches(['.', ':', ',', ';', '/']);
    let lower = id.to_ascii_lowercase();
    (id.len() >= 4 && !["null", "none", "undefined", "unknown"].contains(&lower.as_str()))
        .then(|| id.to_string())
}

/// The request ID in an event, using the default keys.
pub fn find(text: &str) -> Option<String> {
    DEFAULT.find(text)
}

/// The default keys.
pub fn default_keys() -> &'static RequestKeys {
    &DEFAULT
}

/// Accept a full `traceparent` value where a trace ID is expected.
pub fn normalize_query(id: &str) -> String {
    let id = id.trim();
    TRACEPARENT
        .captures(id)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ids_in_text_and_json() {
        assert_eq!(
            find("INFO GET /cart request_id=3f2b9c1d status=200").unwrap(),
            "3f2b9c1d"
        );
        assert_eq!(
            find("INFO done (X-Request-ID: ab12-cd34)").unwrap(),
            "ab12-cd34"
        );
        assert_eq!(
            find("WARN slow traceId=\"9f1e2d3c4b5a\"").unwrap(),
            "9f1e2d3c4b5a"
        );
        assert_eq!(
            find(r#"{"level":"error","msg":"boom","requestId":"req-77aa"}"#).unwrap(),
            "req-77aa"
        );
        assert_eq!(
            find("INFO call traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
                .unwrap(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
    }

    #[test]
    fn ignores_lookalikes_and_placeholders() {
        assert!(find("INFO request ok 200").is_none());
        assert!(find("INFO request_id=null").is_none());
        assert!(find("INFO subrequest_identifier=abcdef").is_none());
        assert!(find("INFO my_request_id_count=12345").is_none());
    }

    #[test]
    fn extra_keys_are_matched_in_any_spelling() {
        let k = RequestKeys::new(&["order_ref".to_string()]);
        assert_eq!(k.find("INFO paid orderRef=ORD-9911").unwrap(), "ORD-9911");
        assert_eq!(
            k.find(r#"{"msg":"x","order-ref":"ORD-1"}"#).unwrap(),
            "ORD-1"
        );
        assert!(find("INFO paid orderRef=ORD-9911").is_none());
    }

    #[test]
    fn queries_accept_a_traceparent() {
        assert_eq!(
            normalize_query("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert_eq!(normalize_query(" 3f2b9c1d "), "3f2b9c1d");
    }
}

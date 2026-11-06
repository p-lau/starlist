//! Pushing to a hub (`--hub URL --service NAME`).
//!
//! A collector writes what it records to an outbox table in its own database,
//! in the same transaction, and a background thread sends the outbox to the
//! hub in batches. The hub acknowledges the last sequence number it applied,
//! and only then are rows deleted, so a hub restart or a network outage loses
//! nothing as long as the outbox fits in `--hub-buffer`. Nothing is ever pulled
//! from a collector.

use crate::pattern::{Event, normalize_level};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Version of the push format, sent with every batch.
pub const PROTOCOL: u32 = 1;
/// Largest batch sent at once, in bytes of outbox JSON.
const BATCH_BYTES: usize = 4 << 20;

/// One event as sent to the hub: already redacted and fingerprinted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WireEvent {
    pub ts: f64,
    pub level: Option<String>,
    pub template: String,
    pub pattern_id: String,
    /// Redacted text, for samples (and the hub's raw buffer unless `no_raw`).
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exc_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<(String, f64)>,
    /// The collector keeps no raw lines, so the hub shouldn't either.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_raw: bool,
}

impl WireEvent {
    pub fn from_event(e: &Event, no_raw: bool) -> Self {
        Self {
            ts: e.ts.unwrap_or_default(),
            level: e.level.map(str::to_string),
            template: e.template.clone(),
            pattern_id: e.pattern_id.clone(),
            body: e.body.clone(),
            exc_type: e.exc_type.clone(),
            location: e.location.clone(),
            request_id: e.request_id.clone(),
            metrics: e.metrics.clone(),
            no_raw: no_raw || e.no_raw,
        }
    }

    pub fn into_event(self) -> Event {
        let head = self.body.lines().next().unwrap_or_default().to_string();
        Event {
            ts: Some(self.ts),
            ts_naive: false,
            level: self.level.as_deref().and_then(normalize_level),
            head,
            body: self.body,
            template: self.template,
            pattern_id: self.pattern_id,
            exc_type: self.exc_type,
            location: self.location,
            request_id: self.request_id,
            metrics: self.metrics,
            no_raw: self.no_raw,
        }
    }
}

/// One outbox entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Item {
    Events {
        events: Vec<WireEvent>,
    },
    Marker {
        name: String,
        at: f64,
        kind: String,
        #[serde(default)]
        note: Option<String>,
    },
    Merge {
        canonical: String,
        template: String,
        aliases: Vec<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub seq: i64,
    pub item: Item,
}

/// A batch from one collector.
#[derive(Debug, Serialize, Deserialize)]
pub struct Batch {
    pub protocol: u32,
    pub service: String,
    /// Stable per collector database, so the hub can tell replicas apart.
    pub instance: String,
    #[serde(default)]
    pub env: Option<String>,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Ack {
    /// The highest sequence number applied; the collector may drop up to it.
    pub acked: i64,
}

/// A service name: letters, digits, `-`, `_` and `.`, up to 64 characters.
pub fn valid_service(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !s.starts_with('.')
}

/// Where and how to push.
#[derive(Clone)]
pub struct HubTarget {
    /// Base URL of the hub, like "https://hub.internal:7341".
    pub url: String,
    pub service: String,
    pub token: String,
}

/// This database's instance ID, created on first use.
pub fn instance_id(store: &Store) -> Result<String, String> {
    if let Some(id) = store.meta("instance").map_err(crate::analysis::db)? {
        return Ok(id);
    }
    let id = crate::http::generate_token()
        .map_err(|e| e.to_string())?
        .chars()
        .take(16)
        .collect::<String>();
    store
        .set_meta("instance", Some(&id))
        .map_err(crate::analysis::db)?;
    Ok(id)
}

/// Send one batch from the outbox. Returns how many entries the hub applied,
/// or `Ok(0)` when the outbox is empty.
pub fn push_once(
    store: &Store,
    target: &HubTarget,
    instance: &str,
    agent: &ureq::Agent,
) -> Result<usize, String> {
    let rows = store
        .outbox_batch(BATCH_BYTES)
        .map_err(crate::analysis::db)?;
    if rows.is_empty() {
        return Ok(0);
    }
    // The rows are JSON already; splice them in rather than parse them again.
    let mut body = serde_json::to_string(&serde_json::json!({
        "protocol": PROTOCOL,
        "service": target.service,
        "instance": instance,
        "env": store.meta("env").map_err(crate::analysis::db)?,
    }))
    .map_err(|e| e.to_string())?;
    body.pop();
    body.push_str(",\"entries\":[");
    for (i, (seq, item)) in rows.iter().enumerate() {
        if i > 0 {
            body.push(',');
        }
        body.push_str(&format!("{{\"seq\":{seq},\"item\":{item}}}"));
    }
    body.push_str("]}");
    let url = format!("{}/hub/v1/push", target.url.trim_end_matches('/'));
    let mut res = agent
        .post(&url)
        .header("Authorization", &format!("Bearer {}", target.token))
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|e| format!("pushing to {url}: {e}"))?;
    let status = res.status().as_u16();
    let text = res.body_mut().read_to_string().unwrap_or_default();
    if status != 200 {
        return Err(format!("the hub answered {status}: {}", text.trim()));
    }
    let ack: Ack =
        serde_json::from_str(&text).map_err(|e| format!("unexpected answer from the hub: {e}"))?;
    let n = rows.iter().filter(|(s, _)| *s <= ack.acked).count();
    store.outbox_ack(ack.acked).map_err(crate::analysis::db)?;
    Ok(n)
}

/// Background thread that keeps sending the outbox, backing off while the hub
/// is unreachable.
pub fn spawn_pusher(store: Arc<Store>, target: HubTarget) -> Result<(), String> {
    let instance = instance_id(&store)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into();
    std::thread::spawn(move || {
        let mut wait = 1u64;
        let mut failing = false;
        loop {
            match push_once(&store, &target, &instance, &agent) {
                Ok(n) => {
                    if failing {
                        eprintln!("starlist: reached the hub again; catching up");
                        failing = false;
                    }
                    wait = 1;
                    // More waiting: send it right away.
                    if n > 0 {
                        continue;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
                Err(e) => {
                    if !failing {
                        eprintln!("starlist: {e}; will keep retrying and catch up later");
                        failing = true;
                    }
                    std::thread::sleep(Duration::from_secs(wait));
                    wait = (wait * 2).min(60);
                }
            }
        }
    });
    Ok(())
}

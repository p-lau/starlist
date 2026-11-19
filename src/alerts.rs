//! Push alerts: notice new or spiking ERROR/FATAL patterns within seconds and
//! tell subscribed agents (MCP `resources/updated`) and an optional webhook.
//!
//! A watcher polls the store every few seconds, so it works whether the
//! collector runs in the same process or writes the database from another one.
//! One incident is one notification: patterns found close together are
//! gathered for a short debounce, alerts are spaced by a minimum gap, and a
//! pattern is not alerted again until its cooldown has passed.

use crate::analysis::{self, Detail, Page};
use crate::store::Store;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

/// The subscribable resource: `whats_new` plus the latest alert.
pub const RESOURCE_URI: &str = "starlist://whats-new";

#[derive(Debug, Clone)]
pub struct AlertConfig {
    /// How often to check for new errors.
    pub interval: Duration,
    /// Recent window compared against the 24h baseline.
    pub window: String,
    /// Wait this long after the first finding to gather the rest of an incident.
    pub debounce_secs: f64,
    /// Minimum time between two notifications.
    pub min_gap_secs: f64,
    /// Don't alert about the same pattern again within this time.
    pub cooldown_secs: f64,
    /// POST each alert as JSON to this URL.
    pub notify_url: Option<String>,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(5),
            window: "5m".into(),
            debounce_secs: 10.0,
            min_gap_secs: 60.0,
            cooldown_secs: 1800.0,
            notify_url: None,
        }
    }
}

/// One notification: the patterns of one incident.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub at: f64,
    pub summary: String,
    pub findings: Vec<Value>,
    /// The service it is about; set on a hub, `None` on a collector.
    pub service: Option<String>,
}

impl Alert {
    pub fn to_json(&self, now: f64) -> Value {
        let mut v = json!({
            "source": "starlist",
            "at": analysis::fmt_time(self.at, now),
            "summary": self.summary,
            "findings": self.findings,
            "next": "Call whats_new or get_pattern for details.",
        });
        if let (Some(s), Some(o)) = (&self.service, v.as_object_mut()) {
            o.insert("service".into(), json!(s));
        }
        v
    }
}

/// Decides when to alert. Pure apart from reading the store, so tests can
/// drive it with their own clock.
pub struct Detector {
    cfg: AlertConfig,
    alerted: HashMap<String, f64>,
    pending: Vec<Value>,
    pending_since: Option<f64>,
    last_sent: Option<f64>,
}

impl Detector {
    pub fn new(cfg: AlertConfig) -> Self {
        Self {
            cfg,
            alerted: HashMap::new(),
            pending: vec![],
            pending_since: None,
            last_sent: None,
        }
    }

    /// Check the store at `now`; returns an alert when one is due.
    pub fn tick(&mut self, store: &Store, now: f64) -> Option<Alert> {
        let page = Page::first(20, Detail::Brief);
        if let Ok(v) = analysis::whats_new(
            store,
            Some(&self.cfg.window),
            Some("24h"),
            Some("ERROR"),
            &page,
            Detail::Brief,
            now,
        ) {
            for f in v["findings"].as_array().into_iter().flatten() {
                let kind = f["kind"].as_str().unwrap_or("");
                // A spike that is usual for this time of day (a nightly job) isn't an incident.
                if !matches!(kind, "new" | "returning" | "spike" | "regressed")
                    || f["usual"] == true
                {
                    continue;
                }
                let Some(id) = f["pattern_id"].as_str() else {
                    continue;
                };
                let cooling = self
                    .alerted
                    .get(id)
                    .is_some_and(|t| now - t < self.cfg.cooldown_secs);
                let queued = self.pending.iter().any(|p| p["pattern_id"] == id);
                if cooling || queued {
                    continue;
                }
                let mut f = f.clone();
                if let Some(o) = f.as_object_mut() {
                    o.remove("sample");
                }
                self.pending.push(f);
                self.pending_since.get_or_insert(now);
            }
        }
        let since = self.pending_since?;
        let gap_ok = self
            .last_sent
            .is_none_or(|t| now - t >= self.cfg.min_gap_secs);
        if now - since < self.cfg.debounce_secs || !gap_ok {
            return None;
        }
        let findings = std::mem::take(&mut self.pending);
        self.pending_since = None;
        self.last_sent = Some(now);
        for f in &findings {
            if let Some(id) = f["pattern_id"].as_str() {
                self.alerted.insert(id.to_string(), now);
            }
        }
        self.alerted
            .retain(|_, t| now - *t < self.cfg.cooldown_secs);
        Some(Alert {
            at: now,
            summary: summarize(&findings),
            findings,
            service: None,
        })
    }
}

fn summarize(findings: &[Value]) -> String {
    let first = findings
        .first()
        .and_then(|f| f["template"].as_str())
        .map(|t| crate::pattern::truncate(t, 120))
        .unwrap_or_default();
    match findings.len() {
        1 => format!(
            "{} error pattern: {first}",
            match findings[0]["kind"].as_str() {
                Some("spike") => "Spiking",
                Some("returning") => "Returning",
                Some("regressed") => "Regressed",
                _ => "New",
            }
        ),
        n => format!("{n} new or spiking error patterns, starting with: {first}"),
    }
}

/// Where alerts go: subscribed MCP sessions listen on the channel.
pub struct Alerts {
    tx: broadcast::Sender<Alert>,
    latest: Mutex<Option<Alert>>,
}

impl Alerts {
    pub fn subscribe(&self) -> broadcast::Receiver<Alert> {
        self.tx.subscribe()
    }

    pub fn latest(&self) -> Option<Alert> {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn publish(&self, a: Alert) {
        *self.latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(a.clone());
        // No receivers is fine: nobody is subscribed right now.
        let _ = self.tx.send(a);
    }
}

/// Start the watcher thread for one store. Returns the channel subscribed
/// sessions listen on.
pub fn spawn_watcher(store: Arc<Store>, cfg: AlertConfig) -> Arc<Alerts> {
    spawn_watcher_for(move || vec![(None, store.clone())], cfg)
}

/// Start one watcher thread over several stores. `sources` is called on every
/// tick, so stores that appear later (a hub's new services) are picked up.
/// Each named store gets its own detector and so its own spacing and cooldowns.
pub fn spawn_watcher_for<F>(sources: F, cfg: AlertConfig) -> Arc<Alerts>
where
    F: Fn() -> Vec<(Option<String>, Arc<Store>)> + Send + 'static,
{
    let (tx, _) = broadcast::channel(16);
    let alerts = Arc::new(Alerts {
        tx,
        latest: Mutex::new(None),
    });
    let out = alerts.clone();
    std::thread::spawn(move || {
        let mut dets: HashMap<Option<String>, Detector> = HashMap::new();
        loop {
            std::thread::sleep(cfg.interval);
            // Nobody to tell: skip the work.
            if cfg.notify_url.is_none() && alerts.tx.receiver_count() == 0 {
                continue;
            }
            let now = analysis::now();
            for (name, store) in sources() {
                let det = dets
                    .entry(name.clone())
                    .or_insert_with(|| Detector::new(cfg.clone()));
                if let Some(mut a) = det.tick(&store, now) {
                    if let Some(n) = &name {
                        a.summary = format!("{n}: {}", a.summary);
                    }
                    a.service = name;
                    eprintln!("starlist: alert: {}", a.summary);
                    if let Some(url) = &cfg.notify_url {
                        post_webhook(url, &a.to_json(now));
                    }
                    alerts.publish(a);
                }
            }
        }
    });
    out
}

/// POST the alert as JSON. Failures are logged, never fatal.
pub fn post_webhook(url: &str, body: &Value) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    if let Err(e) = agent
        .post(url)
        .header("content-type", "application/json")
        .send(body.to_string())
    {
        eprintln!("starlist: webhook to {url} failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_is_in_json_only_when_set() {
        let mut a = Alert {
            at: 100.0,
            summary: "x".into(),
            findings: vec![],
            service: None,
        };
        assert!(a.to_json(100.0).get("service").is_none());
        a.service = Some("api".into());
        assert_eq!(a.to_json(100.0)["service"], "api");
    }
}
